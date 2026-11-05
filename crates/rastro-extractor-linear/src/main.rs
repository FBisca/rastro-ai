//! The Linear extractor: deterministic rules that turn the Linear connector's structured
//! evidence into `tracker` claims (ADR 0008; decisions/vocabulary.md §24–§26).
//!
//! It reads only `structured` observations of nine fields: `/id`, `/app`, `/name`, `/email`,
//! `/active`, `/creator/id`, `/creator/app`, `/assignee/id` and `/assignee/app`, plus the
//! location they were observed at. It never reads a title, a description or any other field, so
//! no claim and no alias comes from text a person wrote in an issue.
//!
//! # The location rule
//! A location names a record when it is exactly `linear:user/<uuid>` or `linear:issue/<uuid>`,
//! with `<uuid>` a lowercase hyphenated UUID, and the latest structured `/id` observation there
//! quotes that same uuid. That location is the record's canonical identifier. Any other
//! location yields no claim at all (the API-key user's observation at `linear:viewer` included:
//! links are derived from it elsewhere).
//!
//! Types and aliases are the core's `rastro:type` and `rastro:alias` claims. Every claim of a
//! run is submitted with the `rastro:type` claims of its own endpoints, because the core takes
//! endpoint types from the same submission only. A claim that needs several spans lists them
//! all in one submission: together they are one justification.
//!
//! # The rules (the manifest's `extraction_rules`, all deterministic)
//! - `account`: a user's `/id` → its `rastro:type` claim: `tracker:Bot` when the latest `/app`
//!   delivered at that location quotes `true` (Linear's app, or integration, user), otherwise
//!   `tracker:Account`, with that `/app` as a support when there is one. A user's flag never
//!   changes, so the latest one types it from its first observation on, a user first read
//!   before the connector read the flag included: one run never types it both ways. Nothing is
//!   typed Person: a workspace user is an account, an app user a bot.
//! - `email`: `/email` with `/id` → a `rastro:alias` of kind `email`, when the address has one
//!   `@` with text on both sides and no whitespace. Accounts only.
//! - `name`: `/name` with `/id` → a `rastro:alias` of kind `name`, for an account or a bot. The
//!   display name is a handle and is not read.
//! - `active`: `/active` `true` or `false` with `/id` → `tracker:active`. Accounts only.
//! - `issue`: an issue's `/id` → its `rastro:type` claim, `tracker:Issue`.
//! - `creator` and `assignee`: `/creator/id` or `/assignee/id` naming a uuid, with the issue's
//!   `/id` → `tracker:creator` or `tracker:assignee` `linear:user/<uuid>`, and that user's
//!   `rastro:type` claim supported by the field: `tracker:Bot` when that id's `/creator/app` or
//!   `/assignee/app` flag quotes `true`, otherwise `tracker:Account`, with the flag as a support
//!   when there is one. An id's flag is worked out over the whole run (`actor_flags`): the
//!   connector reads it just before the id, but an unchanged observation is never stored again,
//!   so it may come before an earlier id or, for an issue read before flags were, after the id.
//!   A null creator or assignee records no evidence, so no claim.
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
const FIELDS: [&str; 9] = [
    "/id",
    "/app",
    "/name",
    "/email",
    "/active",
    "/creator/id",
    "/creator/app",
    "/assignee/id",
    "/assignee/app",
];
/// Core meta-predicates (see the core's `claims` module).
const TYPE: &str = "rastro:type";
const ALIAS: &str = "rastro:alias";
const TYPE_DATATYPE: &str = "rastro:type";
const ACCOUNT: &str = "tracker:Account";
const BOT: &str = "tracker:Bot";
const ISSUE: &str = "tracker:Issue";

/// A Linear id: a lowercase hyphenated UUID, `8-4-4-4-12` hex digits.
fn uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.iter().map(|g| g.len()).eq([8, 4, 4, 4, 12])
        && groups.iter().all(|g| {
            g.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}
/// The record a location names: its type and uuid (see the location rule).
fn record(location: &str) -> Option<(&'static str, &str)> {
    let (kind, id) = match location.split_once('/')? {
        ("linear:user", id) => (ACCOUNT, id),
        ("linear:issue", id) => (ISSUE, id),
        _ => return None,
    };
    uuid(id).then_some((kind, id))
}
/// An address with one `@`, text on both sides, and no whitespace or control character.
fn email(text: &str) -> bool {
    text.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty() && !domain.is_empty() && !domain.contains('@')
    }) && !text.chars().any(|c| c.is_whitespace() || c.is_control())
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

/// An `app` flag: the observation and whether it says the user is an app. Anything but `true`
/// or `false` is no flag.
fn flag(item: &BatchItem) -> Option<(&BatchItem, bool)> {
    match item.span.quote.as_str() {
        "true" => Some((item, true)),
        "false" => Some((item, false)),
        _ => None,
    }
}
/// A user's type given its flag, and the supports the flag adds.
fn user_type(app: Option<(&BatchItem, bool)>) -> (&'static str, Vec<&BatchItem>) {
    match app {
        Some((item, true)) => (BOT, vec![item]),
        Some((item, false)) => (ACCOUNT, vec![item]),
        None => (ACCOUNT, vec![]),
    }
}
/// The latest observation of each field at one location, with its place in replay order.
type Fields<'a> = BTreeMap<&'a str, (usize, &'a BatchItem)>;

/// Each issue actor id's `app` flag, keyed by the id's place in replay order and worked out over
/// the whole run, so the actor is typed one way however the run is read. An unchanged
/// observation is never stored again, so an id and its flag may lie apart: the flag of an id is
/// the latest flag of that actor before it with no other id between them; otherwise the first
/// flag after it with no id between (an id read before the connector read flags, backfilled
/// later); otherwise the latest earlier flag (unchanged since an earlier id).
fn actor_flags(items: &[BatchItem]) -> BTreeMap<usize, &BatchItem> {
    // Per issue and actor: its ids (`true`) and flags (`false`) in replay order.
    type Seq<'a> = Vec<(usize, bool, &'a BatchItem)>;
    let mut actors: BTreeMap<(&str, &str), Seq> = BTreeMap::new();
    for (seen, item) in items.iter().enumerate() {
        let (actor, is_id) = match item.span.field.as_str() {
            "/creator/id" => ("creator", true),
            "/creator/app" => ("creator", false),
            "/assignee/id" => ("assignee", true),
            "/assignee/app" => ("assignee", false),
            _ => continue,
        };
        if item.trust_class != "structured" || (!is_id && flag(item).is_none()) {
            continue;
        }
        let key = (item.location.as_str(), actor);
        actors.entry(key).or_default().push((seen, is_id, item));
    }
    let mut flags = BTreeMap::new();
    for seq in actors.values() {
        let is_flag = |entry: &&(usize, bool, &BatchItem)| !entry.1;
        for (k, &(seen, is_id, _)) in seq.iter().enumerate() {
            if !is_id {
                continue;
            }
            let before = seq[..k].iter().rev().take_while(is_flag).next();
            let after = seq[k + 1..].iter().take_while(is_flag).next();
            let earlier = seq[..k].iter().rev().find(is_flag);
            if let Some(&(_, _, item)) = before.or(after).or(earlier) {
                flags.insert(seen, item);
            }
        }
    }
    flags
}

/// The claims a location supports, given the latest observation of each field there; at a
/// user, the latest `/app` flag the run delivers there; at an issue, each actor id's flag.
fn claims_at(
    location: &str,
    fields: &Fields,
    app: Option<&BatchItem>,
    actor_flags: &BTreeMap<usize, &BatchItem>,
) -> Vec<ClaimSubmit> {
    let Some((record_kind, uuid_text)) = record(location) else {
        return vec![];
    };
    let quote = |field: &str| fields.get(field).map(|(_, i)| (*i, i.span.quote.as_str()));
    let Some((id, _)) = quote("/id").filter(|(_, q)| *q == uuid_text) else {
        return vec![];
    };
    let (kind, flagged) = if record_kind == ACCOUNT {
        user_type(app.and_then(flag))
    } else {
        (ISSUE, vec![])
    };
    let subject = || entity(location, kind);
    let rule = if record_kind == ACCOUNT {
        "account"
    } else {
        "issue"
    };
    let mut claims = vec![claim(
        subject(),
        TYPE,
        literal(kind, TYPE_DATATYPE),
        rule,
        &[&[id][..], &flagged].concat(),
    )];
    if kind == BOT {
        // An app keeps its name; an email or an active state is an account's.
        if let Some((item, full)) = quote("/name").filter(|(_, q)| name(q)) {
            claims.push(claim(
                subject(),
                ALIAS,
                literal(full, "name"),
                "name",
                &[id, item],
            ));
        }
        return claims;
    }
    if kind == ACCOUNT {
        if let Some((item, address)) = quote("/email").filter(|(_, q)| email(q)) {
            claims.push(claim(
                subject(),
                ALIAS,
                literal(address, "email"),
                "email",
                &[id, item],
            ));
        }
        if let Some((item, full)) = quote("/name").filter(|(_, q)| name(q)) {
            claims.push(claim(
                subject(),
                ALIAS,
                literal(full, "name"),
                "name",
                &[id, item],
            ));
        }
        if let Some((item, value @ ("true" | "false"))) = quote("/active") {
            claims.push(claim(
                subject(),
                "tracker:active",
                literal(value, "boolean"),
                "active",
                &[id, item],
            ));
        }
        return claims;
    }
    for (field, predicate, rule) in [
        ("/creator/id", "tracker:creator", "creator"),
        ("/assignee/id", "tracker:assignee", "assignee"),
    ] {
        let Some(&(seen, item)) = fields.get(field).filter(|(_, i)| uuid(&i.span.quote)) else {
            continue;
        };
        let (user_kind, flagged) = user_type(actor_flags.get(&seen).and_then(|i| flag(i)));
        let user = format!("linear:user/{}", item.span.quote);
        claims.extend([
            claim(
                subject(),
                predicate,
                ClaimObject::Entity {
                    identifier: user.clone(),
                    entity_type: user_kind.into(),
                },
                rule,
                &[&[id, item][..], &flagged].concat(),
            ),
            claim(
                entity(&user, user_kind),
                TYPE,
                literal(user_kind, TYPE_DATATYPE),
                rule,
                &[&[item][..], &flagged].concat(),
            ),
        ]);
    }
    claims
}

/// Every claim the observations support, in replay order, each sent once.
fn extract(items: &[BatchItem]) -> Vec<ClaimSubmit> {
    let read = |item: &&BatchItem| item.trust_class == "structured";
    // A user's app flag never changes: the latest one delivered types it all run long.
    let apps: BTreeMap<&str, &BatchItem> = items
        .iter()
        .filter(read)
        .filter(|i| i.span.field == "/app" && flag(i).is_some())
        .map(|i| (i.location.as_str(), i))
        .collect();
    let actor_flags = actor_flags(items);
    let mut latest: BTreeMap<&str, Fields> = BTreeMap::new();
    let mut sent = BTreeSet::new();
    let mut claims = Vec::new();
    for (seen, item) in items.iter().enumerate() {
        let field = item.span.field.as_str();
        if !read(&item) || !FIELDS.contains(&field) {
            continue;
        }
        let fields = latest.entry(&item.location).or_default();
        fields.insert(field, (seen, item));
        let app = apps.get(item.location.as_str()).copied();
        for claim in claims_at(&item.location, fields, app, &actor_flags) {
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
    use rastro_plugin_sdk::Span;

    const USER: &str = "0b5a8c1e-0000-4000-8000-000000000002";
    const OTHER: &str = "0b5a8c1e-0000-4000-8000-000000000003";
    const THIRD: &str = "0b5a8c1e-0000-4000-8000-000000000004";
    const ITEM: &str = "0b5a8c1e-0000-4000-8000-0000000000a1";

    fn observed(id: i64, location: &str, field: &str, quote: &str, trust: &str) -> BatchItem {
        BatchItem {
            evidence_id: id,
            source: "l".into(),
            scope: "work".into(),
            location: location.into(),
            span: Span {
                field: field.into(),
                start: 0,
                end: quote.len() as u64,
                quote: quote.into(),
            },
            captured_at: "2026-11-05T00:00:00.000Z".into(),
            source_updated_at: None,
            trust_class: trust.into(),
        }
    }
    fn user(location: &str, id: &str) -> Vec<BatchItem> {
        vec![
            observed(1, location, "/id", id, "structured"),
            observed(2, location, "/email", "ana@example.com", "structured"),
            observed(3, location, "/name", "Ana Silva", "structured"),
            observed(4, location, "/active", "true", "structured"),
        ]
    }

    /// Only canonical record locations whose `/id` agrees name a record
    #[test]
    fn S20_only_canonical_locations_whose_id_agrees_name_a_record() {
        let location = format!("linear:user/{USER}");
        assert_eq!(extract(&user(&location, USER)).len(), 4);
        for (location, id) in [
            (format!("linear:user/{USER}"), OTHER),
            (format!("linear:user/{}", USER.to_uppercase()), USER),
            (format!("linear:user/{USER}/x"), USER),
            (format!("linear:team/{USER}"), USER),
            (format!("github:user/{USER}"), USER),
            ("linear:user/not-a-uuid".to_string(), "not-a-uuid"),
            ("linear:viewer".to_string(), USER),
        ] {
            assert!(extract(&user(&location, id)).is_empty(), "{location} {id}");
        }
    }

    /// Titles, descriptions, display names and untrusted fields are never read
    #[test]
    fn S20_titles_descriptions_and_untrusted_fields_are_never_read() {
        let issue = format!("linear:issue/{ITEM}");
        let account = format!("linear:user/{USER}");
        let items = [
            observed(1, &issue, "/id", ITEM, "untrusted content"),
            observed(2, &issue, "/title", ITEM, "untrusted content"),
            observed(3, &issue, "/description", USER, "untrusted content"),
            observed(4, &account, "/displayName", USER, "structured"),
            observed(5, &account, "/id", USER, "untrusted content"),
        ];
        assert!(extract(&items).is_empty());
    }

    /// Malformed values yield no claim about them
    #[test]
    fn S20_malformed_values_yield_no_claim_about_them() {
        let location = format!("linear:user/{USER}");
        let items = [
            observed(1, &location, "/id", USER, "structured"),
            observed(2, &location, "/email", "not an address", "structured"),
            observed(3, &location, "/name", " Ana", "structured"),
            observed(4, &location, "/active", "yes", "structured"),
        ];
        let claims = extract(&items);
        assert_eq!(claims.len(), 1, "only the type");
        assert_eq!(claims[0].predicate, TYPE);
        let issue = format!("linear:issue/{ITEM}");
        let items = [
            observed(1, &issue, "/id", ITEM, "structured"),
            observed(2, &issue, "/creator/id", "someone", "structured"),
        ];
        assert_eq!(extract(&items).len(), 1);
    }

    /// Creator and assignee are each one bundle with the issue's `/id`, and their accounts are
    /// typed in the same run
    #[test]
    fn S20_creator_and_assignee_are_bundles_with_typed_accounts() {
        let issue = format!("linear:issue/{ITEM}");
        let items = [
            observed(1, &issue, "/id", ITEM, "structured"),
            observed(2, &issue, "/creator/id", USER, "structured"),
            observed(3, &issue, "/assignee/id", OTHER, "structured"),
        ];
        let claims = extract(&items);
        let spans = |predicate: &str| -> Vec<Vec<(i64, &str)>> {
            claims
                .iter()
                .filter(|c| c.predicate == predicate)
                .map(|c| {
                    c.supports
                        .iter()
                        .map(|s| (s.evidence_id, &*s.field))
                        .collect()
                })
                .collect()
        };
        assert_eq!(spans("tracker:creator"), [[(1, "/id"), (2, "/creator/id")]]);
        assert_eq!(
            spans("tracker:assignee"),
            [[(1, "/id"), (3, "/assignee/id")]]
        );
        let typed: BTreeSet<(&str, &str)> = claims
            .iter()
            .filter(|c| c.predicate == TYPE)
            .map(|c| (&*c.subject.identifier, &*c.subject.entity_type))
            .collect();
        for claim in &claims {
            assert!(typed.contains(&(&*claim.subject.identifier, &*claim.subject.entity_type)));
            if let ClaimObject::Entity {
                identifier,
                entity_type,
            } = &claim.object
            {
                assert!(typed.contains(&(&**identifier, &**entity_type)));
            }
        }
        assert!(
            claims
                .iter()
                .all(|c| !c.subject.entity_type.contains("Person"))
        );
    }

    /// The user types a run sends, by subject identifier.
    fn types(claims: &[ClaimSubmit]) -> BTreeSet<(String, String)> {
        claims
            .iter()
            .filter(|c| c.predicate == TYPE)
            .map(|c| (c.subject.identifier.clone(), c.subject.entity_type.clone()))
            .collect()
    }

    /// S38: an app user is typed `tracker:Bot`, never `tracker:Account`, whether its `/app`
    /// flag was read before or after its `/id` (a user first read before the connector read the
    /// flag included); it keeps its name, and nothing an account has
    #[test]
    fn S38_an_app_user_is_a_bot_never_an_account() {
        let location = format!("linear:user/{USER}");
        let app = |id: i64, value: &str| observed(id, &location, "/app", value, "structured");
        let read_later = [user(&location, USER), vec![app(5, "true")]].concat();
        let read_first = [vec![app(0, "true")], user(&location, USER)].concat();
        for items in [read_later, read_first] {
            let claims = extract(&items);
            assert_eq!(types(&claims), [(location.clone(), BOT.to_string())].into());
            assert!(claims.iter().all(|c| c.subject.entity_type == BOT));
            let said: BTreeSet<_> = claims
                .iter()
                .map(|c| {
                    (
                        c.predicate.as_str(),
                        serde_json::to_string(&c.object).unwrap(),
                    )
                })
                .map(|(p, o)| format!("{p} {o}"))
                .collect();
            assert_eq!(said.len(), 2, "{said:?}");
            assert!(said.iter().any(|s| s.contains("Ana Silva")), "{said:?}");
        }
        // A user Linear flags `false`, or one read before the flag, is an account.
        for items in [
            user(&location, USER),
            [user(&location, USER), vec![app(5, "false")]].concat(),
        ] {
            let claims = extract(&items);
            assert_eq!(claims.len(), 4);
            assert_eq!(
                types(&claims),
                [(location.clone(), ACCOUNT.to_string())].into()
            );
        }
    }

    /// S38: an issue's creator or assignee is typed `tracker:Bot` by the issue's own `/…/app`
    /// flag, and one actor's flag is never paired with another actor's id, even when the
    /// assignee changes from an app to a person
    #[test]
    fn S38_an_issue_actors_app_flag_types_it_and_never_another_actor() {
        let issue = format!("linear:issue/{ITEM}");
        let at =
            |id: i64, field: &str, quote: &str| observed(id, &issue, field, quote, "structured");
        let items = [
            at(1, "/id", ITEM),
            at(2, "/creator/app", "true"),
            at(3, "/creator/id", USER),
            at(4, "/assignee/app", "true"),
            at(5, "/assignee/id", OTHER),
            // Reassigned to a person: the flag is read just before the new id.
            at(6, "/assignee/app", "false"),
            at(7, "/assignee/id", THIRD),
        ];
        let claims = extract(&items);
        let users = |id: &str| format!("linear:user/{id}");
        assert_eq!(
            types(&claims),
            [
                (issue.clone(), ISSUE.to_string()),
                (users(USER), BOT.to_string()),
                (users(OTHER), BOT.to_string()),
                (users(THIRD), ACCOUNT.to_string()),
            ]
            .into()
        );
        let creator: Vec<_> = claims
            .iter()
            .filter(|c| c.predicate == "tracker:creator")
            .collect();
        assert_eq!(creator.len(), 1);
        assert!(
            matches!(&creator[0].object, ClaimObject::Entity { entity_type, .. } if entity_type == BOT)
        );
        let spans: Vec<_> = creator[0]
            .supports
            .iter()
            .map(|s| (s.evidence_id, s.field.as_str()))
            .collect();
        assert_eq!(spans, [(1, "/id"), (3, "/creator/id"), (2, "/creator/app")]);
    }

    /// S38: an issue read before the connector read actors' flags gets its flag later, after
    /// the unchanged id (which is never stored again): that flag is the id's, so an app creator
    /// is typed `tracker:Bot` and never `tracker:Account`. An unchanged flag before an earlier
    /// id still holds for a later id, and a flag after a later id is not an earlier id's
    #[test]
    fn S38_a_flag_read_after_an_unchanged_id_types_that_id() {
        let issue = format!("linear:issue/{ITEM}");
        let at =
            |id: i64, field: &str, quote: &str| observed(id, &issue, field, quote, "structured");
        let users = |id: &str| format!("linear:user/{id}");
        // Backfill: the id was read before the flag existed.
        let backfill = [
            at(1, "/id", ITEM),
            at(2, "/creator/id", USER),
            at(10, "/creator/app", "true"),
        ];
        assert_eq!(
            types(&extract(&backfill)),
            [
                (issue.clone(), ISSUE.to_string()),
                (users(USER), BOT.to_string())
            ]
            .into()
        );
        // The creator's flag, unchanged, is stored once before its first id and holds for the
        // next. The assignee changed twice before any flag was read: the flag read after the
        // second is the second's, not the first's.
        const FOURTH: &str = "0b5a8c1e-0000-4000-8000-000000000005";
        let history = [
            at(1, "/id", ITEM),
            at(2, "/creator/app", "true"),
            at(3, "/creator/id", USER),
            at(4, "/creator/id", OTHER),
            at(5, "/assignee/id", THIRD),
            at(6, "/assignee/id", FOURTH),
            at(7, "/assignee/app", "true"),
        ];
        assert_eq!(
            types(&extract(&history)),
            [
                (issue.clone(), ISSUE.to_string()),
                (users(USER), BOT.to_string()),
                (users(OTHER), BOT.to_string()),
                (users(THIRD), ACCOUNT.to_string()),
                (users(FOURTH), BOT.to_string()),
            ]
            .into()
        );
    }

    /// S38: the `tracker` vocabulary declares `tracker:Account` an account and `tracker:Bot`
    /// not, so a link to a bot is refused
    #[test]
    fn S38_the_tracker_vocabulary_declares_an_account_but_never_a_bot() {
        let vocabulary =
            rastro_plugin_sdk::Vocabulary::parse(include_str!("../vocabulary.toml")).unwrap();
        let account = |t: &str| {
            vocabulary
                .ancestors(t)
                .contains(&rastro_plugin_sdk::ACCOUNT_TYPE.to_string())
        };
        assert!(vocabulary.has_type(BOT));
        assert!(account(ACCOUNT) && !account(BOT) && !account(ISSUE));
    }
}
