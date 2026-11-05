//! The Linear extractor: deterministic rules that turn the Linear connector's structured
//! evidence into `tracker` claims (ADR 0008; decisions/vocabulary.md §24–§26).
//!
//! It reads only `structured` observations of six fields: `/id`, `/name`, `/email`, `/active`,
//! `/creator/id` and `/assignee/id`, plus the location they were observed at. It never reads a
//! title, a description or any other field, so no claim and no alias comes from text a person
//! wrote in an issue.
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
//! - `account`: a user's `/id` → its `rastro:type` claim, `tracker:Account`. Nothing is typed
//!   Person: a workspace user is an account.
//! - `email`: `/email` with `/id` → a `rastro:alias` of kind `email`, when the address has one
//!   `@` with text on both sides and no whitespace.
//! - `name`: `/name` with `/id` → a `rastro:alias` of kind `name`. The display name is a handle
//!   and is not read.
//! - `active`: `/active` `true` or `false` with `/id` → `tracker:active`.
//! - `issue`: an issue's `/id` → its `rastro:type` claim, `tracker:Issue`.
//! - `creator` and `assignee`: `/creator/id` or `/assignee/id` naming a uuid, with the issue's
//!   `/id` → `tracker:creator` or `tracker:assignee` `linear:user/<uuid>`, and that account's
//!   `rastro:type` claim supported by the field alone. A null creator or assignee records no
//!   evidence, so no claim.
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
const FIELDS: [&str; 6] = [
    "/id",
    "/name",
    "/email",
    "/active",
    "/creator/id",
    "/assignee/id",
];
/// Core meta-predicates (see the core's `claims` module).
const TYPE: &str = "rastro:type";
const ALIAS: &str = "rastro:alias";
const TYPE_DATATYPE: &str = "rastro:type";
const ACCOUNT: &str = "tracker:Account";
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

/// The claims a location supports, given the latest observation of each field there.
fn claims_at(location: &str, fields: &BTreeMap<&str, &BatchItem>) -> Vec<ClaimSubmit> {
    let Some((kind, uuid_text)) = record(location) else {
        return vec![];
    };
    let quote = |field: &str| fields.get(field).map(|i| (*i, i.span.quote.as_str()));
    let Some((id, _)) = quote("/id").filter(|(_, q)| *q == uuid_text) else {
        return vec![];
    };
    let subject = || entity(location, kind);
    let rule = if kind == ACCOUNT { "account" } else { "issue" };
    let mut claims = vec![claim(
        subject(),
        TYPE,
        literal(kind, TYPE_DATATYPE),
        rule,
        &[id],
    )];
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
        let Some((item, user)) = quote(field).filter(|(_, q)| uuid(q)) else {
            continue;
        };
        let account = format!("linear:user/{user}");
        claims.extend([
            claim(
                subject(),
                predicate,
                ClaimObject::Entity {
                    identifier: account.clone(),
                    entity_type: ACCOUNT.into(),
                },
                rule,
                &[id, item],
            ),
            claim(
                entity(&account, ACCOUNT),
                TYPE,
                literal(ACCOUNT, TYPE_DATATYPE),
                rule,
                &[item],
            ),
        ]);
    }
    claims
}

/// Every claim the observations support, in replay order, each sent once.
fn extract(items: &[BatchItem]) -> Vec<ClaimSubmit> {
    let mut latest: BTreeMap<&str, BTreeMap<&str, &BatchItem>> = BTreeMap::new();
    let mut sent = BTreeSet::new();
    let mut claims = Vec::new();
    for item in items {
        let field = item.span.field.as_str();
        if item.trust_class != "structured" || !FIELDS.contains(&field) {
            continue;
        }
        let fields = latest.entry(&item.location).or_default();
        fields.insert(field, item);
        for claim in claims_at(&item.location, fields) {
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
mod tests {
    use super::*;
    use rastro_plugin_sdk::Span;

    const USER: &str = "0b5a8c1e-0000-4000-8000-000000000002";
    const OTHER: &str = "0b5a8c1e-0000-4000-8000-000000000003";
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
    fn only_canonical_locations_whose_id_agrees_name_a_record() {
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
    fn titles_descriptions_and_untrusted_fields_are_never_read() {
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
    fn malformed_values_yield_no_claim_about_them() {
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
    fn creator_and_assignee_are_bundles_with_typed_accounts() {
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
}
