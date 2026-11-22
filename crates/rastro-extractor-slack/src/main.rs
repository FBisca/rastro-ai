//! The Slack extractor: deterministic rules that turn the Slack connector's structured
//! evidence into `slack` claims (ADR 0008; decisions/vocabulary.md §24–§27, decisions/plugins.md
//! §55).
//!
//! It reads only `structured` observations of five fields: `/id`, `/is_bot`, `/name`,
//! `/real_name` and `/profile/display_name`, plus the location they were observed at. It never
//! reads a message's text or any other field, so no claim and no alias comes from text a person
//! wrote in a channel.
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
use rastro_plugin_sdk::{
    BatchItem, CLAIM_SUBMIT, ClaimObject, ClaimSubmit, EntityRef, Envelope, EvidenceBatch,
    ExtractorInitializeParams, InitializeResult, PROTOCOL_VERSION, RUN_FINISH, Role, RunFinish,
    SupportRef, Transport,
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
    match item.span.quote.as_str() {
        "true" => Some((item, true)),
        "false" => Some((item, false)),
        _ => None,
    }
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
        let batch: EvidenceBatch = transport.read()?.context("core closed")?.params()?;
        items.extend(batch.items);
        if batch.last {
            break;
        }
    }
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
}
