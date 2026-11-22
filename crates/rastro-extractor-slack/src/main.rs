//! The Slack extractor: deterministic rules that turn the Slack connector's structured
//! evidence into `slack` claims (ADR 0008; decisions/vocabulary.md §24–§27, decisions/plugins.md
//! §55).
//!
//! For claims it reads only `structured` observations of five fields: `/id`, `/is_bot`,
//! `/name`, `/real_name` and `/profile/display_name`, plus the location they were observed at.
//! It never reads a message's text for a claim, so no claim and no alias comes from text a
//! person wrote in a channel.
//!
//! # Conversation turns
//! The extractor also answers `conversation.turns` (ADR 0011, amendment 2026-11-22): the core
//! sends a thread's identity and its evidence — the messages' observations and the structured
//! user records of their authors — and the extractor makes one turn per message in post order.
//! A speaker is the account the evidence resolves (`slack:Account`, or `slack:Bot` with kind
//! `bot`), never a name parsed from text; an author no record resolves is a label of its id.
//! The text keeps the message's own bytes with user mentions resolved to display labels (the
//! id appended when authors share one), and each turn cites its message.
//!
//! # The location rule
//! A location names a record when it is exactly `slack:user/<id>`, with `<id>` a Slack user id
//! (`U`, or `W` on Enterprise Grid, then uppercase alphanumerics), and the latest structured
//! `/id` observation there quotes that same id. That location is the record's canonical
//! identifier. Any other location yields no claim at all except `slack:viewer`, whose `/id`
//! identifies the token owner's account.
//!
//! Types and aliases are the core's `rastro:type` and `rastro:alias` claims. Every claim of a
//! run is submitted with the `rastro:type` claims of its own endpoints, because the core takes
//! endpoint types from the same submission only.
//!
//! # The rules (the manifest's `extraction_rules`, all deterministic)
//! - `account`: a user's `/id` → its `rastro:type` claim: `slack:Bot` when the latest `/is_bot`
//!   delivered at that location quotes `true`, otherwise `slack:Account`, with that `/is_bot`
//!   as a support when there is one. A user's flag never changes, so the latest one types it
//!   from its first observation on. Nothing is typed Person: a workspace user is an account, a
//!   bot user a bot.
//! - `token-owner`: the viewer's `/id` → the owner's account typed `slack:Account` and its
//!   `rastro:token_owner` claim, so the core holds it by `me`.
//! - `name`: `/name` or `/real_name` with `/id` → a `rastro:alias` of kind `name`, for an
//!   account or a bot.
//! - `display-name`: `/profile/display_name` with `/id` → a `rastro:alias` of kind
//!   `display_name`, for an account or a bot.
//!
//! Observations are read in replay order. At each one, the claims its location supports given
//! the latest observation of each field so far are sent, each once per run.
use anyhow::{Context, Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use rastro_plugin_sdk::{
    BatchItem, CLAIM_SUBMIT, CONVERSATION_TURNS, ClaimObject, ClaimSubmit, ConversationSpeaker,
    ConversationTurn, ConversationTurnKind, ConversationTurnsRequest, ConversationTurnsResponse,
    EVIDENCE_BATCH, EntityRef, Envelope, EvidenceBatch, ExtractorInitializeParams,
    InitializeResult, PROTOCOL_VERSION, RUN_FINISH, Role, RunFinish, SupportRef, Transport,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

/// The fields any rule reads. Everything else is skipped unread.
const FIELDS: [&str; 5] = [
    "/id",
    "/is_bot",
    "/name",
    "/real_name",
    "/profile/display_name",
];
/// Core meta-predicates (see the core's `claims` module).
const TYPE: &str = "rastro:type";
const ALIAS: &str = "rastro:alias";
const TYPE_DATATYPE: &str = "rastro:type";
const TOKEN_OWNER: &str = "rastro:token_owner";
const ACCOUNT: &str = "slack:Account";
const BOT: &str = "slack:Bot";

/// A Slack user id: `U` (or `W` on Enterprise Grid) then uppercase alphanumerics.
fn slack_id(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some('U' | 'W'))
        && chars.clone().next().is_some()
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}
/// The record a location names: its user id (see the location rule).
fn record(location: &str) -> Option<&str> {
    match location.split_once('/')? {
        ("slack:user", id) => slack_id(id).then_some(id),
        _ => None,
    }
}
/// A name: non-empty, no whitespace at either end, no control character.
fn name(text: &str) -> bool {
    !text.is_empty() && text.trim() == text && !text.chars().any(char::is_control)
}

fn entity(identifier: &str, entity_type: &str) -> EntityRef {
    EntityRef {
        identifier: identifier.into(),
        entity_type: entity_type.into(),
    }
}
fn literal(value: &str, datatype: &str) -> ClaimObject {
    ClaimObject::Literal {
        value: value.into(),
        datatype: datatype.into(),
    }
}
/// The whole span of an observation.
fn support(item: &BatchItem) -> SupportRef {
    SupportRef {
        evidence_id: item.evidence_id,
        field: item.span.field.clone(),
        start: item.span.start,
        end: item.span.end,
    }
}
fn claim(
    subject: EntityRef,
    predicate: &str,
    object: ClaimObject,
    rule: &str,
    supports: &[&BatchItem],
) -> ClaimSubmit {
    ClaimSubmit {
        subject,
        predicate: predicate.into(),
        object,
        extraction_rule: rule.into(),
        supports: supports.iter().map(|i| support(i)).collect(),
    }
}

/// The latest observation of each field at one location, with its place in replay order.
type Fields<'a> = BTreeMap<&'a str, (usize, &'a BatchItem)>;

/// An `is_bot` flag: the observation and whether it says the user is a bot. Anything but
/// `true` or `false` is no flag.
fn flag(item: &BatchItem) -> Option<(&BatchItem, bool)> {
    flag_value(&item.span.quote).map(|bot| (item, bot))
}
/// A user's type given its flag, and the supports the flag adds.
fn user_type(bot: Option<(&BatchItem, bool)>) -> (&'static str, Vec<&BatchItem>) {
    match bot {
        Some((item, true)) => (BOT, vec![item]),
        Some((item, false)) => (ACCOUNT, vec![item]),
        None => (ACCOUNT, vec![]),
    }
}

/// The claims a user location supports, given the latest observation of each field there and
/// the latest `/is_bot` flag the run delivers there.
fn claims_at(location: &str, fields: &Fields, bot: Option<&BatchItem>) -> Vec<ClaimSubmit> {
    let Some(id_text) = record(location) else {
        return vec![];
    };
    let quote = |field: &str| fields.get(field).map(|(_, i)| (*i, i.span.quote.as_str()));
    let Some((id, _)) = quote("/id").filter(|(_, q)| *q == id_text) else {
        return vec![];
    };
    let (kind, flagged) = user_type(bot.and_then(flag));
    let subject = || entity(location, kind);
    let mut claims = vec![claim(
        subject(),
        TYPE,
        literal(kind, TYPE_DATATYPE),
        "account",
        &[&[id][..], &flagged].concat(),
    )];
    // A bot keeps its names, like an account: they look it up, they never link it.
    for field in ["/name", "/real_name"] {
        if let Some((item, value)) = quote(field).filter(|(_, q)| name(q)) {
            claims.push(claim(
                subject(),
                ALIAS,
                literal(value, "name"),
                "name",
                &[id, item],
            ));
        }
    }
    if let Some((item, value)) = quote("/profile/display_name").filter(|(_, q)| name(q)) {
        claims.push(claim(
            subject(),
            ALIAS,
            literal(value, "display_name"),
            "display-name",
            &[id, item],
        ));
    }
    claims
}

/// Every claim the observations support, in replay order, each sent once.
fn extract(items: &[BatchItem]) -> Vec<ClaimSubmit> {
    let read = |item: &&BatchItem| item.trust_class == "structured";
    // A user's bot flag never changes: the latest one delivered types it all run long.
    let bots: BTreeMap<&str, &BatchItem> = items
        .iter()
        .filter(read)
        .filter(|i| i.span.field == "/is_bot" && flag(i).is_some())
        .map(|i| (i.location.as_str(), i))
        .collect();
    let mut latest: BTreeMap<&str, Fields> = BTreeMap::new();
    let mut sent = BTreeSet::new();
    let mut claims = Vec::new();
    for (seen, item) in items.iter().enumerate() {
        let field = item.span.field.as_str();
        if !read(&item) || !FIELDS.contains(&field) {
            continue;
        }
        // Whose token it is: the viewer's `/id` names the owner's account, held by `me`
        // through the core's token-owner link. A user token's owner is a human account.
        if item.location == "slack:viewer" && field == "/id" && slack_id(&item.span.quote) {
            let identifier = format!("slack:user/{}", item.span.quote);
            for c in [
                claim(
                    entity(&identifier, ACCOUNT),
                    TYPE,
                    literal(ACCOUNT, TYPE_DATATYPE),
                    "account",
                    &[item],
                ),
                claim(
                    entity(&identifier, ACCOUNT),
                    TOKEN_OWNER,
                    literal("true", "boolean"),
                    "token-owner",
                    &[item],
                ),
            ] {
                if sent.insert(serde_json::to_string(&c).unwrap_or_default()) {
                    claims.push(c);
                }
            }
            continue;
        }
        let fields = latest.entry(&item.location).or_default();
        fields.insert(field, (seen, item));
        let bot = bots.get(item.location.as_str()).copied();
        for claim in claims_at(&item.location, fields, bot) {
            if sent.insert(serde_json::to_string(&claim).unwrap_or_default()) {
                claims.push(claim);
            }
        }
    }
    claims
}

/// An author the request's evidence resolves: its type and the names a label can come from.
struct Author<'a> {
    kind: &'static str,
    display_name: Option<&'a str>,
    real_name: Option<&'a str>,
    name: Option<&'a str>,
}
impl Author<'_> {
    /// The display label: the display name, else the full name, else the username, else the id.
    fn label<'a>(&'a self, id: &'a str) -> &'a str {
        self.display_name
            .or(self.real_name)
            .or(self.name)
            .unwrap_or(id)
    }
}

/// The authors the request's evidence resolves: a user record's latest `/id` must quote its
/// location's id, and its latest `/is_bot` flag types it, exactly as the extraction rules type
/// it. An author no record resolves is absent.
fn authors(items: &[BatchItem]) -> BTreeMap<String, Author<'_>> {
    let mut fields: BTreeMap<&str, BTreeMap<&str, &BatchItem>> = BTreeMap::new();
    for item in items.iter().filter(|i| i.trust_class == "structured") {
        if record(&item.location).is_some() {
            fields
                .entry(&item.location)
                .or_default()
                .insert(&item.span.field, item);
        }
    }
    let mut resolved = BTreeMap::new();
    for (location, fields) in fields {
        let Some(id) = record(location) else { continue };
        let quote = |field: &str| fields.get(field).map(|i| i.span.quote.as_str());
        if quote("/id") != Some(id) {
            continue;
        }
        let kind = match quote("/is_bot").and_then(flag_value) {
            Some(true) => BOT,
            _ => ACCOUNT,
        };
        let named = |field: &str| quote(field).filter(|q| name(q));
        resolved.insert(
            id.to_string(),
            Author {
                kind,
                display_name: named("/profile/display_name"),
                real_name: named("/real_name"),
                name: named("/name"),
            },
        );
    }
    resolved
}
/// An `/is_bot` quote: anything but `true` or `false` is no flag.
fn flag_value(quote: &str) -> Option<bool> {
    match quote {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// The label each resolved id reads as: its display label, with the id appended when two or
/// more users in the request's evidence share that label, so a shared name never collapses
/// two authors into one.
fn display_labels<'a>(
    authors: &'a BTreeMap<String, Author<'a>>,
    owner: Option<&'a str>,
) -> BTreeMap<String, String> {
    fn base<'a>(id: &'a str, authors: &'a BTreeMap<String, Author<'a>>) -> &'a str {
        authors.get(id).map(|a| a.label(id)).unwrap_or(id)
    }
    let ids = || {
        authors
            .keys()
            .map(String::as_str)
            .chain(owner.filter(|id| !authors.contains_key(*id)))
    };
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for id in ids() {
        *counts.entry(base(id, authors)).or_default() += 1;
    }
    ids()
        .map(|id| {
            let label = base(id, authors);
            let label = if counts.get(label).is_some_and(|n| *n > 1) {
                format!("{label} ({id})")
            } else {
                label.to_string()
            };
            (id.to_string(), label)
        })
        .collect()
}

/// The label a mentioned id reads as, and the account it resolves to: a recorded author, the
/// token owner (an account even without a profile), or a bare id no evidence resolves.
fn mention(
    id: &str,
    authors: &BTreeMap<String, Author<'_>>,
    owner: Option<&str>,
    labels: &BTreeMap<String, String>,
) -> (String, Option<EntityRef>) {
    let identifier = format!("slack:user/{id}");
    let label = labels.get(id).map(String::as_str).unwrap_or(id).to_string();
    match authors.get(id) {
        Some(author) => (label, Some(entity(&identifier, author.kind))),
        None if Some(id) == owner => (label, Some(entity(&identifier, ACCOUNT))),
        None => (label, None),
    }
}

/// The message text with user mentions resolved to labels, and the accounts it mentions in
/// first-appearance order. Anything that is not a user mention stays as written.
fn resolve_mentions(
    text: &str,
    authors: &BTreeMap<String, Author<'_>>,
    owner: Option<&str>,
    labels: &BTreeMap<String, String>,
) -> (String, Vec<EntityRef>) {
    let mut out = String::new();
    let mut mentions = Vec::new();
    let mut seen = BTreeSet::new();
    let mut rest = text;
    while let Some(at) = rest.find("<@") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 2..];
        let Some(end) = after.find('>') else {
            out.push_str(&rest[at..]);
            rest = "";
            break;
        };
        let inner = &after[..end];
        let id = inner.split('|').next().unwrap_or(inner);
        if slack_id(id) {
            let (label, account) = mention(id, authors, owner, labels);
            out.push_str(&label);
            if let Some(account) = account
                && seen.insert(id.to_string())
            {
                mentions.push(account);
            }
        } else {
            out.push_str(&rest[at..at + 2 + end + 1]);
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    (out, mentions)
}

/// The channel and ts a message location names, with the ts as epoch seconds and
/// microseconds: `slack:message/<channel>/<ts>`. Anything else names no message.
fn message_ts(location: &str) -> Option<(&str, &str, i64, u32)> {
    let (channel, ts) = location.strip_prefix("slack:message/")?.rsplit_once('/')?;
    if channel.is_empty() {
        return None;
    }
    let (secs, frac) = ts.split_once('.')?;
    let secs: i64 = secs.parse().ok()?;
    let mut frac = frac.to_string();
    frac.truncate(6);
    while frac.len() < 6 {
        frac.push('0');
    }
    let micros: u32 = frac.parse().ok()?;
    Some((channel, ts, secs, micros))
}

/// The thread's turns, one per message in post order: the speaker the evidence resolves — an
/// account, `slack:Bot` with kind `bot`, or a label of the id when no record resolves it — the
/// message's own text with user mentions resolved to display labels, its time, its reply-to,
/// the accounts it mentions, and its own location as the citation. Only the named thread's
/// messages become turns.
fn make_turns(request: &ConversationTurnsRequest) -> Vec<ConversationTurn> {
    let authors = authors(&request.messages);
    // Whose token it is: the viewer's latest `/id` names the owner's account.
    let owner = request
        .messages
        .iter()
        .rev()
        .find(|i| {
            i.trust_class == "structured"
                && i.location == "slack:viewer"
                && i.span.field == "/id"
                && slack_id(&i.span.quote)
        })
        .map(|i| i.span.quote.as_str());
    let labels = display_labels(&authors, owner);
    // The latest observation of each field at each message location. The text is untrusted
    // content; who wrote it and what thread it answers are structured.
    let mut messages: BTreeMap<&str, BTreeMap<&str, &BatchItem>> = BTreeMap::new();
    for item in &request.messages {
        if !item.location.starts_with("slack:message/") {
            continue;
        }
        let field = item.span.field.as_str();
        let read = match field {
            "/text" => true,
            "/user" | "/thread_ts" | "/subtype" => item.trust_class == "structured",
            _ => false,
        };
        if read {
            messages
                .entry(&item.location)
                .or_default()
                .insert(field, item);
        }
    }
    let mut turns = Vec::new();
    let mut ordered = Vec::new();
    for (location, fields) in &messages {
        let Some((channel, ts, secs, micros)) = message_ts(location) else {
            continue;
        };
        // Only the named thread: its start message and the replies that name it.
        let thread_ts = fields.get("/thread_ts").map(|i| i.span.quote.as_str());
        if ts != request.thread && thread_ts != Some(request.thread.as_str()) {
            continue;
        }
        ordered.push((secs, micros, channel, ts, *location, fields));
    }
    ordered.sort_by_key(|(secs, micros, ..)| (*secs, *micros));
    for (secs, micros, channel, ts, location, fields) in ordered {
        let quote = |field: &str| fields.get(field).map(|i| i.span.quote.as_str());
        let speaker = match quote("/user").filter(|id| slack_id(id)) {
            Some(id) => match authors.get(id) {
                Some(author) => {
                    ConversationSpeaker::Account(entity(&format!("slack:user/{id}"), author.kind))
                }
                None if Some(id) == owner => {
                    ConversationSpeaker::Account(entity(&format!("slack:user/{id}"), ACCOUNT))
                }
                None => ConversationSpeaker::Label(id.into()),
            },
            None => ConversationSpeaker::Label("unknown".into()),
        };
        let kind = match &speaker {
            ConversationSpeaker::Account(account) if account.entity_type == BOT => {
                ConversationTurnKind::Bot
            }
            _ if quote("/subtype") == Some("bot_message") => ConversationTurnKind::Bot,
            _ => ConversationTurnKind::Person,
        };
        let (text, mentions) =
            resolve_mentions(quote("/text").unwrap_or_default(), &authors, owner, &labels);
        let time = DateTime::<Utc>::from_timestamp(secs, micros * 1000)
            .map(|at| at.to_rfc3339_opts(SecondsFormat::Millis, true));
        let reply_to = quote("/thread_ts")
            .filter(|thread| *thread != ts)
            .map(|thread| format!("slack:message/{channel}/{thread}"));
        turns.push(ConversationTurn {
            speaker,
            kind,
            text,
            time,
            reply_to,
            cites: Some(location.into()),
            mentions,
        });
    }
    turns
}

fn main() -> Result<()> {
    let mut transport = Transport::new(io::stdin().lock(), io::stdout().lock());
    let request = transport.read()?.context("core closed before initialize")?;
    let params: ExtractorInitializeParams = request.params()?;
    if params.role != Role::Extractor {
        bail!("not started as an extractor");
    }
    transport.send(&Envelope::response(
        request.id,
        InitializeResult {
            protocol: PROTOCOL_VERSION.into(),
            manifest_id: params.manifest_id,
        },
    )?)?;
    let mut items = Vec::new();
    loop {
        let Some(message) = transport.read()? else {
            break;
        };
        match message.method.as_deref() {
            Some(EVIDENCE_BATCH) => {
                let batch: EvidenceBatch = message.params()?;
                items.extend(batch.items);
                if batch.last {
                    let claims = extract(&items);
                    for claim in &claims {
                        transport.send(&Envelope::notification(CLAIM_SUBMIT, claim)?)?;
                    }
                    let finish = RunFinish {
                        status: "ok".into(),
                        submitted: claims.len() as u64,
                        reason: None,
                    };
                    transport.send(&Envelope::request(1, RUN_FINISH, finish)?)?;
                    transport
                        .read()?
                        .context("core closed before answering run.finish")?;
                    return Ok(());
                }
            }
            Some(CONVERSATION_TURNS) => {
                let request: ConversationTurnsRequest = message.params()?;
                let turns = make_turns(&request);
                transport.send(&Envelope::response(
                    message.id,
                    ConversationTurnsResponse { turns },
                )?)?;
            }
            // Responses and anything else carry no work for an extractor.
            _ => {}
        }
    }
    Ok(())
}
#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use rastro_plugin_sdk::{ClaimObject, Span};
    use std::collections::BTreeSet;

    const ANA: &str = "U0ANA";
    const BOT: &str = "U0BOT";

    fn observed(id: i64, location: &str, field: &str, quote: &str, trust: &str) -> BatchItem {
        BatchItem {
            evidence_id: id,
            source: "s".into(),
            scope: "work".into(),
            location: location.into(),
            span: Span {
                field: field.into(),
                start: 0,
                end: quote.len() as u64,
                quote: quote.into(),
            },
            captured_at: "2026-11-21T00:00:00.000Z".into(),
            source_updated_at: None,
            trust_class: trust.into(),
        }
    }
    fn user(location: &str, id: &str) -> Vec<BatchItem> {
        vec![
            observed(1, location, "/id", id, "structured"),
            observed(2, location, "/name", "ana.silva", "structured"),
            observed(3, location, "/real_name", "Ana Silva", "structured"),
            observed(4, location, "/profile/display_name", "Ana", "structured"),
        ]
    }
    /// The (predicate, object value) pairs a run says.
    fn said(claims: &[ClaimSubmit]) -> BTreeSet<(String, String)> {
        claims
            .iter()
            .map(|c| {
                let value = match &c.object {
                    ClaimObject::Literal { value, .. } => value.clone(),
                    ClaimObject::Entity { identifier, .. } => identifier.clone(),
                };
                (c.predicate.clone(), value)
            })
            .collect()
    }
    fn account_claims() -> BTreeSet<(String, String)> {
        [
            ("rastro:type", "slack:Account"),
            ("rastro:alias", "ana.silva"),
            ("rastro:alias", "Ana Silva"),
            ("rastro:alias", "Ana"),
        ]
        .map(|(p, o)| (p.to_string(), o.to_string()))
        .into()
    }

    /// spec: A Slack sync records accounts with their names
    /// Only canonical record locations whose `/id` agrees name a record
    #[test]
    fn only_canonical_locations_whose_id_agrees_name_a_record() {
        let location = format!("slack:user/{ANA}");
        assert_eq!(said(&extract(&user(&location, ANA))), account_claims());
        for (location, id) in [
            (format!("slack:user/{ANA}"), "U0OTHER"),
            (format!("slack:user/{}", ANA.to_lowercase()), ANA),
            (format!("slack:user/{ANA}/x"), ANA),
            (format!("slack:channel/{ANA}"), ANA),
            (format!("linear:user/{ANA}"), ANA),
            ("slack:user/not-an-id".to_string(), "not-an-id"),
            ("slack:user/U0ana".to_string(), "U0ana"),
        ] {
            assert!(extract(&user(&location, id)).is_empty(), "{location} {id}");
        }
    }

    /// spec: A Slack sync records accounts with their names
    /// Untrusted observations, unknown fields and malformed values yield no claim, while the
    /// well-formed structured fields of the same user still yield theirs
    #[test]
    fn untrusted_and_malformed_fields_yield_no_claim() {
        let location = format!("slack:user/{ANA}");
        let mut items = user(&location, ANA);
        items.extend([
            observed(5, &location, "/real_name", "Evil Name", "untrusted content"),
            observed(6, &location, "/name", " ana", "structured"),
            observed(7, &location, "/profile/display_name", "", "structured"),
            observed(8, &location, "/text", "Ana Silva", "untrusted content"),
        ]);
        assert_eq!(said(&extract(&items)), account_claims());
        // An untrusted `/id` names no record at all.
        let untrusted_id = [
            observed(1, &location, "/id", ANA, "untrusted content"),
            observed(2, &location, "/name", "ana.silva", "structured"),
        ];
        assert!(extract(&untrusted_id).is_empty());
    }

    /// spec: The Slack token owner holds me
    /// The viewer's `/id` types the owner's account and claims the token owner, citing the
    /// viewer observation; a malformed viewer id claims nothing
    #[test]
    fn the_viewer_id_claims_the_token_owner() {
        let owner = format!("slack:user/{ANA}");
        let claims = extract(&[observed(1, "slack:viewer", "/id", ANA, "structured")]);
        assert_eq!(
            said(&claims),
            [
                ("rastro:type", "slack:Account"),
                ("rastro:token_owner", "true"),
            ]
            .map(|(p, o)| (p.to_string(), o.to_string()))
            .into()
        );
        for claim in &claims {
            assert_eq!(claim.subject.identifier, owner);
            assert_eq!(claim.subject.entity_type, ACCOUNT);
            let spans: Vec<_> = claim
                .supports
                .iter()
                .map(|s| (s.evidence_id, s.field.as_str()))
                .collect();
            assert_eq!(spans, [(1, "/id")]);
        }
        assert!(
            extract(&[observed(
                1,
                "slack:viewer",
                "/id",
                "not-an-id",
                "structured"
            )])
            .is_empty()
        );
    }

    /// spec: A Slack bot never links
    /// A bot author is typed `slack:Bot`, never `slack:Account`, whether its `/is_bot` flag
    /// was read before or after its `/id`; it keeps its names. A user Slack flags `false`, or
    /// one read before the flag, is an account
    #[test]
    fn a_bot_author_is_a_slack_bot_never_an_account() {
        let location = format!("slack:user/{BOT}");
        let flag = |id: i64, value: &str| observed(id, &location, "/is_bot", value, "structured");
        let profile = || {
            vec![
                observed(1, &location, "/id", BOT, "structured"),
                observed(2, &location, "/name", "deploy.bot", "structured"),
                observed(3, &location, "/real_name", "Deploy Bot", "structured"),
            ]
        };
        let read_later = [profile(), vec![flag(5, "true")]].concat();
        let read_first = [vec![flag(0, "true")], profile()].concat();
        for items in [read_later, read_first] {
            let claims = extract(&items);
            assert!(
                claims.iter().all(|c| c.subject.entity_type == "slack:Bot"),
                "{claims:?}"
            );
            assert_eq!(
                said(&claims),
                [
                    ("rastro:type", "slack:Bot"),
                    ("rastro:alias", "deploy.bot"),
                    ("rastro:alias", "Deploy Bot"),
                ]
                .map(|(p, o)| (p.to_string(), o.to_string()))
                .into()
            );
        }
        for items in [profile(), [profile(), vec![flag(5, "false")]].concat()] {
            let claims = extract(&items);
            assert!(
                claims.iter().all(|c| c.subject.entity_type == ACCOUNT),
                "{claims:?}"
            );
        }
    }

    /// spec: A Slack bot never links
    /// The `slack` vocabulary declares `slack:Account` an account and `slack:Bot` not, so a
    /// link to a bot is refused and a bot is never a candidate
    #[test]
    fn the_slack_vocabulary_declares_an_account_but_never_a_bot() {
        let vocabulary =
            rastro_plugin_sdk::Vocabulary::parse(include_str!("../vocabulary.toml")).unwrap();
        let account = |t: &str| {
            vocabulary
                .ancestors(t)
                .contains(&rastro_plugin_sdk::ACCOUNT_TYPE.to_string())
        };
        assert!(vocabulary.has_type("slack:Bot"));
        assert!(account("slack:Account") && !account("slack:Bot"));
    }

    const CHANNEL: &str = "C0ENG";
    const START: &str = "1700000000.000100";

    fn message_location(ts: &str) -> String {
        format!("slack:message/{CHANNEL}/{ts}")
    }
    /// One message's stored observations: its text untrusted, its author and thread structured.
    fn message_items(
        id: i64,
        ts: &str,
        author: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Vec<BatchItem> {
        let location = message_location(ts);
        let mut items = vec![
            observed(id, &location, "/text", text, "untrusted content"),
            observed(id + 1, &location, "/user", author, "structured"),
        ];
        if let Some(thread) = thread_ts {
            items.push(observed(
                id + 2,
                &location,
                "/thread_ts",
                thread,
                "structured",
            ));
        }
        items
    }
    fn turns_request(thread: &str, messages: Vec<BatchItem>) -> ConversationTurnsRequest {
        ConversationTurnsRequest {
            source: "s".into(),
            scope: "work".into(),
            thread: thread.into(),
            messages,
        }
    }
    fn account_speaker(id: &str, entity_type: &str) -> ConversationSpeaker {
        ConversationSpeaker::Account(EntityRef {
            identifier: format!("slack:user/{id}"),
            entity_type: entity_type.into(),
        })
    }

    /// spec: A chat thread's turns come from its source's extractor
    /// A message whose second line starts `Note:` is one turn by its author, carrying the
    /// whole text: no speaker comes from the text
    #[test]
    fn a_multi_line_message_is_one_turn_with_no_speaker_from_its_text() {
        let text = "I'll send it Friday\nNote: bring the doc";
        let mut items = user(&format!("slack:user/{ANA}"), ANA);
        items.extend(message_items(10, START, ANA, text, None));
        let turns = make_turns(&turns_request(START, items));
        assert_eq!(turns.len(), 1, "{turns:?}");
        let turn = &turns[0];
        assert_eq!(turn.speaker, account_speaker(ANA, ACCOUNT));
        assert_eq!(turn.text, text);
        assert_eq!(
            turn.cites.as_deref(),
            Some(message_location(START).as_str())
        );
    }

    /// spec: A chat thread's turns come from its source's extractor
    /// An author Slack flags as a bot speaks as its `slack:Bot` account with kind `bot`, a
    /// human author with kind `person`; a bot message with no author id is still kind `bot`
    #[test]
    fn a_bot_authors_turn_is_kind_bot_with_its_bot_account() {
        let mut items = user(&format!("slack:user/{ANA}"), ANA);
        let bot_location = format!("slack:user/{BOT}");
        items.extend([
            observed(20, &bot_location, "/id", BOT, "structured"),
            observed(21, &bot_location, "/is_bot", "true", "structured"),
        ]);
        items.extend(message_items(10, START, ANA, "hi", None));
        items.extend(message_items(
            30,
            "1700000060.000200",
            BOT,
            "deployed",
            Some(START),
        ));
        let orphan = "1700000120.000300";
        let location = message_location(orphan);
        items.extend([
            observed(
                40,
                &location,
                "/text",
                "release shipped",
                "untrusted content",
            ),
            observed(41, &location, "/thread_ts", START, "structured"),
            observed(42, &location, "/subtype", "bot_message", "structured"),
        ]);
        let turns = make_turns(&turns_request(START, items));
        assert_eq!(turns.len(), 3, "{turns:?}");
        assert_eq!(turns[0].speaker, account_speaker(ANA, ACCOUNT));
        assert_eq!(turns[0].kind, ConversationTurnKind::Person);
        assert_eq!(turns[1].speaker, account_speaker(BOT, "slack:Bot"));
        assert_eq!(turns[1].kind, ConversationTurnKind::Bot);
        assert_eq!(
            turns[2].speaker,
            ConversationSpeaker::Label("unknown".into())
        );
        assert_eq!(turns[2].kind, ConversationTurnKind::Bot);
    }

    /// spec: A chat thread's turns come from its source's extractor
    /// A mention of the token owner by id reads as the owner's label in the turn text, and
    /// the owner's account is among the turn's mentions — with a profile the label is the
    /// display name, without one the id itself
    #[test]
    fn a_mention_of_the_owner_reads_as_the_owners_label() {
        let owner = "U0OWNER";
        let owner_location = format!("slack:user/{owner}");
        let with_profile = [
            user(&format!("slack:user/{ANA}"), ANA),
            vec![
                observed(20, &owner_location, "/id", owner, "structured"),
                observed(
                    21,
                    &owner_location,
                    "/profile/display_name",
                    "Owner",
                    "structured",
                ),
                observed(22, "slack:viewer", "/id", owner, "structured"),
            ],
            message_items(10, START, ANA, "<@U0OWNER> can you send the doc?", None),
        ]
        .concat();
        let viewer_only = [
            user(&format!("slack:user/{ANA}"), ANA),
            vec![observed(22, "slack:viewer", "/id", owner, "structured")],
            message_items(10, START, ANA, "<@U0OWNER> can you send the doc?", None),
        ]
        .concat();
        for (items, label) in [(with_profile, "Owner"), (viewer_only, owner)] {
            let turns = make_turns(&turns_request(START, items));
            assert_eq!(turns.len(), 1, "{turns:?}");
            assert_eq!(turns[0].text, format!("{label} can you send the doc?"));
            assert_eq!(
                turns[0].mentions,
                vec![EntityRef {
                    identifier: format!("slack:user/{owner}"),
                    entity_type: ACCOUNT.into(),
                }]
            );
        }
    }

    /// spec: A chat thread's turns come from its source's extractor
    /// Two authors with the same display name keep turn labels that differ by their ids, and
    /// a mention of either reads as a label carrying its id
    #[test]
    fn two_authors_sharing_a_display_name_turn_labels_differ_by_id() {
        let ana2 = "U0AN2";
        let mut items = user(&format!("slack:user/{ANA}"), ANA);
        let other = format!("slack:user/{ana2}");
        items.extend([
            observed(20, &other, "/id", ana2, "structured"),
            observed(21, &other, "/profile/display_name", "Ana", "structured"),
        ]);
        items.extend(message_items(10, START, ANA, "hi", None));
        items.extend(message_items(
            30,
            "1700000060.000200",
            ana2,
            "hello",
            Some(START),
        ));
        items.extend(message_items(
            40,
            "1700000120.000300",
            ANA,
            "<@U0ANA> and <@U0AN2> pair up",
            Some(START),
        ));
        let turns = make_turns(&turns_request(START, items));
        assert_eq!(turns.len(), 3, "{turns:?}");
        assert_eq!(turns[0].speaker, account_speaker(ANA, ACCOUNT));
        assert_eq!(turns[1].speaker, account_speaker(ana2, ACCOUNT));
        assert_ne!(turns[0].speaker, turns[1].speaker);
        assert_eq!(turns[2].text, "Ana (U0ANA) and Ana (U0AN2) pair up");
    }

    /// spec: A chat thread's turns come from its source's extractor
    /// Each turn cites its own message and carries its time; a reply names the thread start's
    /// location as its reply-to, the start names none; turns come in post order whatever
    /// order the evidence arrives in
    #[test]
    fn each_turn_cites_its_message_and_replies_to_the_thread_start() {
        let reply_ts = "1700000060.000200";
        let mut items = user(&format!("slack:user/{ANA}"), ANA);
        // Delivered reply-first: post order must not depend on delivery order.
        items.extend(message_items(30, reply_ts, ANA, "hello", Some(START)));
        items.extend(message_items(10, START, ANA, "hi", None));
        let turns = make_turns(&turns_request(START, items));
        assert_eq!(turns.len(), 2, "{turns:?}");
        assert_eq!(
            turns[0].cites.as_deref(),
            Some(message_location(START).as_str())
        );
        assert_eq!(turns[0].reply_to, None);
        assert_eq!(turns[0].time.as_deref(), Some("2023-11-14T22:13:20.000Z"));
        assert_eq!(turns[0].text, "hi");
        assert_eq!(
            turns[1].cites.as_deref(),
            Some(message_location(reply_ts).as_str())
        );
        assert_eq!(
            turns[1].reply_to.as_deref(),
            Some(message_location(START).as_str())
        );
        assert_eq!(turns[1].time.as_deref(), Some("2023-11-14T22:14:20.000Z"));
        assert_eq!(turns[1].text, "hello");
    }

    /// spec: A chat thread's turns come from its source's extractor
    /// Only the named thread's messages become turns: another thread's messages delivered
    /// with the evidence make none
    #[test]
    fn messages_outside_the_thread_make_no_turn() {
        let other_start = "1700000000.000900";
        let reply_ts = "1700000060.000200";
        let mut items = user(&format!("slack:user/{ANA}"), ANA);
        items.extend(message_items(10, START, ANA, "hi", None));
        items.extend(message_items(30, reply_ts, ANA, "reply", Some(START)));
        items.extend(message_items(40, other_start, ANA, "elsewhere", None));
        items.extend(message_items(
            50,
            "1700000060.000900",
            ANA,
            "other reply",
            Some(other_start),
        ));
        let turns = make_turns(&turns_request(START, items));
        let cites: Vec<_> = turns.iter().map(|t| t.cites.clone()).collect();
        assert_eq!(
            cites,
            [
                Some(message_location(START)),
                Some(message_location(reply_ts))
            ],
            "{turns:?}"
        );
    }

    /// spec: A chat thread's turns come from its source's extractor
    /// An author no user record resolves is a label of its id, never a fabricated account: a
    /// profile the sync could not read leaves the speaker unresolved
    #[test]
    fn an_author_without_a_profile_is_a_label_not_an_account() {
        let items = message_items(10, START, ANA, "hi", None);
        let turns = make_turns(&turns_request(START, items));
        assert_eq!(turns.len(), 1, "{turns:?}");
        assert_eq!(turns[0].speaker, ConversationSpeaker::Label(ANA.into()));
        assert_eq!(turns[0].kind, ConversationTurnKind::Person);
    }
}
