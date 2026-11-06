//! The GitHub extractor: deterministic rules that turn the GitHub connector's structured
//! evidence into `code` claims (ADR 0008; decisions/knowledge.md §23–§27, vocabulary.md
//! §10–§13).
//!
//! It reads structured state, account and profile fields, plus the token-owner and verified-email
//! snapshot observations. It never reads a title or body.
//!
//! # The URL rule
//! A location is a canonical `html_url` when it is exactly
//! `https://<host>/<owner>/<repo>/(pull|issues)/<number>`, optionally followed by
//! `#issuecomment-<id>`: lowercase host, no port, user, query or trailing slash, `<number>` and
//! `<id>` decimal without leading zeros. Then:
//! - `/pull/<n>` is a `code:PullRequest`, `/issues/<n>` a `code:Issue`;
//! - with `#issuecomment-<id>` it is a `code:Comment`, on the item the URL names without its
//!   fragment (that parent link is not claimed yet);
//! - its repository is `https://<host>/<owner>/<repo>`, a `code:Repository`.
//!
//! Each of these is claimed only beside a structured observation at that location, which
//! supports it. A location that isn't canonical yields no claim at all.
//!
//! Types and aliases are the core's `rastro:type` and `rastro:alias` claims (the core owns the
//! `rastro:` prefix). Every claim of a run is submitted with the `rastro:type` claims of its own
//! endpoints, because the core takes endpoint types from the same submission only. A claim that
//! needs several spans (an author needs `/user/id` and `/user/type`, an alias `/user/id` and
//! `/user/login`) lists them all in one submission: together they are one justification.
//!
//! # The rules (the manifest's `extraction_rules`, all deterministic)
//! - `state`: `/state` `open` or `closed` on a pull request or issue → `code:state`.
//! - `kind-from-url`: the URL rule's type for the location → a `rastro:type` claim, supported by the
//!   `/state` or `/user/id` observation there.
//! - `repository-from-url`: a pull request or issue's `/state` observation →
//!   `code:in_repository` and the repository's `rastro:type` claim.
//! - `author`: `/user/id` with `/user/type` `User` or `Bot` → `code:author`, the account
//!   `github:account/<id>` (a `code:Account` or `code:Bot`). A null author records no
//!   evidence, so no author claim.
//! - `account-type`: the same pair → the account's `rastro:type` claim. Nothing is typed `Person`.
//! - `login`: `/user/login` with `/user/id` → a `rastro:alias` of kind `login` on the account. A
//!   rename adds a login; the account, named by its numeric id, stays the same entity.
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
const FIELDS: [&str; 8] = [
    "/state",
    "/user/id",
    "/user/login",
    "/user/type",
    "/user/name",
    "/id",
    "/type",
    "/emails/verified",
];
/// Core meta-predicates (see the core's `claims` module).
const TYPE: &str = "rastro:type";
const ALIAS: &str = "rastro:alias";
const TYPE_DATATYPE: &str = "rastro:type";

/// What a canonical html_url names.
#[derive(Debug, Eq, PartialEq)]
struct Url {
    kind: &'static str,
    repository: String,
}

fn decimal(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) && !text.starts_with('0')
}
/// The URL rule (see the module docs).
fn parse_url(location: &str) -> Option<Url> {
    let rest = location.strip_prefix("https://")?;
    let (path, fragment) = match rest.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (rest, None),
    };
    let parts: Vec<&str> = path.split('/').collect();
    let [host, owner, repo, section, number] = parts[..] else {
        return None;
    };
    // Dot-separated labels of lowercase letters, digits and inner hyphens.
    let host_ok = host.split('.').all(|label| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    });
    let name_ok = |name: &str| {
        !name.is_empty()
            && name != "."
            && name != ".."
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    };
    if host.is_empty() || !host_ok || !name_ok(owner) || !name_ok(repo) || !decimal(number) {
        return None;
    }
    let item = match section {
        "pull" => "code:PullRequest",
        "issues" => "code:Issue",
        _ => return None,
    };
    let kind = match fragment {
        None => item,
        Some(fragment) if fragment.strip_prefix("issuecomment-").is_some_and(decimal) => {
            "code:Comment"
        }
        Some(_) => return None,
    };
    Some(Url {
        kind,
        repository: format!("https://{host}/{owner}/{repo}"),
    })
}

/// A GitHub login: ASCII letters, digits and hyphens, with GitHub's `[bot]` suffix allowed.
fn login(text: &str) -> bool {
    let name = text.strip_suffix("[bot]").unwrap_or(text);
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
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
    if location == "github:viewer" {
        return viewer_claims(fields);
    }
    let Some(url) = parse_url(location) else {
        return vec![];
    };
    let subject = || entity(location, url.kind);
    let kind = || literal(url.kind, TYPE_DATATYPE);
    let mut claims = Vec::new();
    let quote = |field: &str| fields.get(field).map(|i| (*i, i.span.quote.as_str()));
    if url.kind != "code:Comment"
        && let Some((state, value @ ("open" | "closed"))) = quote("/state")
    {
        let repository = || entity(&url.repository, "code:Repository");
        let rule = "repository-from-url";
        claims.extend([
            claim(
                subject(),
                "code:state",
                literal(value, "string"),
                "state",
                &[state],
            ),
            claim(subject(), TYPE, kind(), "kind-from-url", &[state]),
            claim(
                subject(),
                "code:in_repository",
                ClaimObject::Entity {
                    identifier: url.repository.clone(),
                    entity_type: "code:Repository".into(),
                },
                rule,
                &[state],
            ),
            claim(
                repository(),
                TYPE,
                literal("code:Repository", TYPE_DATATYPE),
                rule,
                &[state],
            ),
        ]);
    }
    let account_type = match quote("/user/type") {
        Some((item, "User")) => Some((item, "code:Account")),
        Some((item, "Bot")) => Some((item, "code:Bot")),
        _ => None,
    };
    if let (Some((id, number)), Some((type_item, account_type))) = (quote("/user/id"), account_type)
        && decimal(number)
    {
        let identifier = format!("github:account/{number}");
        let account = || entity(&identifier, account_type);
        claims.extend([
            claim(
                subject(),
                "code:author",
                ClaimObject::Entity {
                    identifier: identifier.clone(),
                    entity_type: account_type.into(),
                },
                "author",
                &[id, type_item],
            ),
            claim(subject(), TYPE, kind(), "kind-from-url", &[id]),
            claim(
                account(),
                TYPE,
                literal(account_type, TYPE_DATATYPE),
                "account-type",
                &[id, type_item],
            ),
        ]);
        if let Some((login_item, name)) = quote("/user/login")
            && login(name)
        {
            claims.push(claim(
                account(),
                ALIAS,
                literal(name, "login"),
                "login",
                &[id, login_item],
            ));
        }
        if let Some((name_item, name)) = quote("/user/name") {
            claims.push(claim(
                account(),
                ALIAS,
                literal(name, "name"),
                "name",
                &[id, name_item],
            ));
        }
    }
    claims
}

fn viewer_claims(fields: &BTreeMap<&str, &BatchItem>) -> Vec<ClaimSubmit> {
    let (Some(id), Some(kind)) = (fields.get("/id"), fields.get("/type")) else {
        return vec![];
    };
    if !decimal(&id.span.quote) || kind.span.quote != "User" {
        return vec![];
    }
    let identifier = format!("github:account/{}", id.span.quote);
    let account = || entity(&identifier, "code:Account");
    let mut out = vec![
        claim(
            account(),
            TYPE,
            literal("code:Account", TYPE_DATATYPE),
            "account-type",
            &[id, kind],
        ),
        claim(
            account(),
            "rastro:token_owner",
            literal("true", "boolean"),
            "token-owner",
            &[id, kind],
        ),
    ];
    if let Some(snapshot) = fields.get("/emails/verified")
        && let Ok(addresses) = serde_json::from_str::<Vec<String>>(
            snapshot
                .span
                .quote
                .split_once('\n')
                .map_or(snapshot.span.quote.as_str(), |(list, _)| list),
        )
    {
        out.push(claim(
            account(),
            TYPE,
            literal("code:Account", TYPE_DATATYPE),
            "account-type",
            &[id, kind, snapshot],
        ));
        let mut raw_offset = snapshot.span.quote.find('\n').map(|n| n + 1);
        for address in addresses {
            let encoded = serde_json::to_string(&address).unwrap_or_default();
            let offset = raw_offset
                .and_then(|offset| {
                    (snapshot.span.quote.get(offset..offset + address.len())
                        == Some(address.as_str()))
                    .then_some(offset)
                })
                .or_else(|| {
                    (encoded == format!("\"{address}\""))
                        .then(|| snapshot.span.quote.find(&encoded).map(|n| n + 1))
                        .flatten()
                });
            if let Some(offset) = offset {
                let mut alias = claim(
                    account(),
                    ALIAS,
                    literal(&address, "email"),
                    "email",
                    &[id, snapshot],
                );
                alias.supports[1].start = snapshot.span.start + offset as u64;
                alias.supports[1].end = alias.supports[1].start + address.len() as u64;
                out.push(alias);
            }
            raw_offset = raw_offset.map(|offset| offset + address.len());
            out.push(claim(
                account(),
                "rastro:verified_email",
                literal(&address, "email"),
                "verified-email",
                &[id, kind, snapshot],
            ));
        }
    }
    out
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
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use rastro_plugin_sdk::Span;

    fn observed(id: i64, location: &str, field: &str, quote: &str, trust: &str) -> BatchItem {
        BatchItem {
            evidence_id: id,
            source: "x".into(),
            scope: "personal".into(),
            location: location.into(),
            span: Span {
                field: field.into(),
                start: 0,
                end: quote.len() as u64,
                quote: quote.into(),
            },
            captured_at: "2026-11-04T00:00:00.000Z".into(),
            source_updated_at: None,
            trust_class: trust.into(),
        }
    }

    /// claims/R19: the url rule names kind and repository of canonical urls only
    #[test]
    fn the_url_rule_names_kind_and_repository_of_canonical_urls_only() {
        let repo = "https://github.com/o/p".to_string();
        for (url, kind) in [
            ("https://github.com/o/p/pull/4", "code:PullRequest"),
            ("https://github.com/o/p/issues/1", "code:Issue"),
            (
                "https://github.com/o/p/issues/1#issuecomment-10",
                "code:Comment",
            ),
            (
                "https://github.com/o/p/pull/4#issuecomment-11",
                "code:Comment",
            ),
        ] {
            let parsed = parse_url(url);
            assert_eq!(
                parsed,
                Some(Url {
                    kind,
                    repository: repo.clone()
                }),
                "{url}"
            );
        }
        for url in [
            "http://github.com/o/p/pull/4",
            "https://GitHub.com/o/p/pull/4",
            "https://github.com/o/p/pull/4/",
            "https://github.com/o/p/pull/04",
            "https://github.com/o/p/pull/4?x=1",
            "https://github.com/o/p/pull/4#discussion_r5",
            "https://github.com/o/p/commit/4",
            "https://github.com:443/o/p/pull/4",
            "https://u@github.com/o/p/pull/4",
            "https://github.com/o/p",
            "https://github.com/../p/pull/4",
            "https://github.com/o/../pull/4",
            "https://github.com/./p/pull/4",
            "https://.github.com/o/p/pull/4",
            "https://github..com/o/p/pull/4",
            "https://github.com./o/p/pull/4",
            "https://-github.com/o/p/pull/4",
            "https://github-.com/o/p/pull/4",
        ] {
            assert_eq!(parse_url(url), None, "{url}");
        }
    }

    /// claims/R18: titles bodies and untrusted fields are never read
    #[test]
    fn titles_bodies_and_untrusted_fields_are_never_read() {
        let pr = "https://github.com/o/p/pull/4";
        let items = [
            observed(1, pr, "/title", "open", "untrusted content"),
            observed(2, pr, "/body", "closed", "untrusted content"),
            observed(3, pr, "/state", "open", "untrusted content"),
            observed(4, pr, "/labels/0/name", "open", "structured"),
        ];
        assert!(extract(&items).is_empty());
    }

    /// claims/R17: an unknown user type or state yields no claim about it
    #[test]
    fn an_unknown_user_type_or_state_yields_no_claim_about_it() {
        let pr = "https://github.com/o/p/pull/4";
        let items = [
            observed(1, pr, "/state", "merged", "structured"),
            observed(2, pr, "/user/id", "7", "structured"),
            observed(3, pr, "/user/type", "Organization", "structured"),
            observed(4, pr, "/user/login", "o", "structured"),
        ];
        assert!(extract(&items).is_empty());
    }

    /// claims/R19: an author and an alias are each one bundle of all the spans they need
    #[test]
    fn an_author_and_an_alias_are_each_one_bundle_of_all_the_spans_they_need() {
        let pr = "https://github.com/o/p/pull/4";
        let items = [
            observed(1, pr, "/state", "open", "structured"),
            observed(2, pr, "/user/id", "7", "structured"),
            observed(3, pr, "/user/type", "User", "structured"),
            observed(4, pr, "/user/login", "octocat", "structured"),
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
        assert_eq!(spans("code:author"), [[(2, "/user/id"), (3, "/user/type")]]);
        assert_eq!(spans(ALIAS), [[(2, "/user/id"), (4, "/user/login")]]);
        // Every claim's endpoints are typed by `rastro:type` claims of the same run.
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
        // Nothing is typed Person and no claim uses an unprefixed meta-predicate.
        assert!(claims.iter().all(|c| c.subject.entity_type != "Person"));
        assert!(
            claims
                .iter()
                .all(|c| c.predicate != "type" && c.predicate != "alias")
        );
    }
}
