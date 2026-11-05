//! Bounded claim reads. Scoped reads recompute from visible justifications.
use crate::claims::ClaimView;
use crate::{Coverage, Rastro, ReadStorage, Reader, Storage, read_only};
use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Clone, Debug)]
pub struct AboutOptions {
    pub since: Option<DateTime<Utc>>,
    pub limit: usize,
    pub history: bool,
}
impl Default for AboutOptions {
    fn default() -> Self {
        Self {
            since: None,
            limit: 20,
            history: false,
        }
    }
}
#[derive(Debug, JsonSchema, Serialize)]
pub struct ClaimSummary {
    #[serde(flatten)]
    pub claim: ClaimView,
    pub subject_aliases: Vec<String>,
    pub object_aliases: Vec<String>,
    /// Number of live justifications, independent of the explanation page limit.
    pub support_count: usize,
}
#[derive(Debug, JsonSchema, Serialize)]
pub struct About {
    /// Whether the identifier named an entity this read can see. A well-formed identifier that
    /// names nothing visible (unknown, or hidden-only) is `false`, with the unknown-entity answer.
    pub resolved: bool,
    // Every claim shown, each with its own id; a group's members are all listed here.
    pub claims: Vec<AboutClaim>,
    pub truncated: bool,
    pub coverage: Vec<Coverage>,
    pub extraction: Vec<crate::runs::ExtractionPair>,
    /// Equivalent claims under a merge, grouped. A group counts once toward `limit`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<ClaimGroup>,
    /// When the identifier names a Person: the Person and each account it holds. The claims of
    /// those accounts are among `claims`, each naming its account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub person: Option<PersonView>,
}
// One claim of an `about` answer. No doc comment: the schema keeps the claim's description.
#[derive(Debug, JsonSchema, Serialize)]
pub struct AboutClaim {
    #[serde(flatten)]
    pub summary: ClaimSummary,
    /// In a Person's view, the entity id of the linked account this claim is about; absent for
    /// the Person's own claims and outside a Person's view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}
/// A Person, as `about` shows it: the accounts it holds (or a Person merged with it holds),
/// each with the reasons it is linked.
#[derive(Debug, JsonSchema, Serialize)]
pub struct PersonView {
    pub id: String,
    pub identifier: String,
    /// Ordered by identifier.
    pub accounts: Vec<LinkedAccount>,
}
#[derive(Debug, JsonSchema, Serialize)]
pub struct LinkedAccount {
    pub id: String,
    pub identifier: String,
    /// The account's displayed type.
    #[serde(rename = "type")]
    pub entity_type: String,
    /// Its current aliases.
    pub aliases: Vec<String>,
    /// Why it is linked: one entry per current justification of each of its `rastro:held_by`
    /// claims to the Person or a Person merged with it.
    pub reasons: Vec<LinkReason>,
}
/// One reason an account is linked to a Person.
#[derive(Debug, JsonSchema, Serialize)]
pub struct LinkReason {
    pub kind: LinkReasonKind,
    /// The `rastro:held_by` claim the reason justifies; `why` explains it.
    pub claim: String,
    /// The justification's id, as `why` lists it.
    pub justification: String,
    /// The owner intent, for a reason of kind `owner_intent`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
}
/// How an account came to be linked: the owner said so, an equal verified email, or the account
/// owns the source's token.
#[derive(Clone, Copy, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkReasonKind {
    OwnerIntent,
    VerifiedEmail,
    TokenOwner,
}
/// What storage reads for `about`, before grouping and `limit`.
#[derive(Debug, Default)]
pub struct AboutClaims {
    pub resolved: bool,
    /// Ordered newest first, then by id. For a Person, every claim of its view; otherwise up to
    /// one more than `limit`.
    pub claims: Vec<AboutClaim>,
    pub person: Option<PersonView>,
}
/// Claims that say the same thing once merged Persons are taken as one (knowledge §35). There is
/// no representative claim: each member keeps its own id, status and revision.
#[derive(Debug, JsonSchema, Serialize)]
pub struct ClaimGroup {
    pub members: Vec<GroupMember>,
    /// `observed` when any member is.
    #[schemars(extend("enum" = ["observed", "superseded"]))]
    pub status: String,
    /// The hash of the sorted (member id, revision) pairs. Never a claim id.
    pub revision: String,
}
#[derive(Debug, JsonSchema, Serialize)]
pub struct GroupMember {
    pub claim: String,
    #[schemars(extend("enum" = ["observed", "superseded"]))]
    pub status: String,
    pub revision: String,
}
fn validate_entity(entity: &str) -> Result<()> {
    crate::entity_form::check_reference(entity).map_err(anyhow::Error::msg)
}
/// Claims ordered newest first become the answer: claims equivalent under the observed merges
/// among them group, an item (a lone claim or a group, placed at its newest member) counts once
/// toward `limit`, and every member of a kept item is listed.
fn answer(
    read: AboutClaims,
    limit: usize,
    coverage: Vec<Coverage>,
    extraction: Vec<crate::runs::ExtractionPair>,
) -> About {
    let AboutClaims {
        resolved,
        claims,
        person,
    } = read;
    use crate::claims::Object;
    use std::collections::BTreeMap;
    let mut parent = BTreeMap::<String, String>::new();
    fn root(parent: &BTreeMap<String, String>, id: &str) -> String {
        let mut id = id.to_string();
        while let Some(next) = parent.get(&id).filter(|next| **next != id) {
            id = next.clone();
        }
        id
    }
    for c in claims.iter().map(|s| &s.summary.claim) {
        if let (crate::identity::SAME_AS, "observed", Object::Entity { id, .. }) =
            (c.predicate.as_str(), c.status.as_str(), &c.object)
        {
            let (a, b) = (root(&parent, &c.subject.id), root(&parent, id));
            if a != b {
                let (low, high) = if a < b { (a, b) } else { (b, a) };
                parent.insert(high, low);
            }
        }
    }
    let mut items: Vec<Vec<AboutClaim>> = vec![];
    let mut index = BTreeMap::<(String, String, String, String), usize>::new();
    for summary in claims {
        let c = &summary.summary.claim;
        let object = match &c.object {
            Object::Entity { id, .. } => (String::new(), root(&parent, id)),
            Object::Literal { value, datatype } => (datatype.clone(), value.clone()),
        };
        // A merge is never equivalent to another merge: each `rastro:same_as` stays its own item.
        let key = if c.predicate == crate::identity::SAME_AS {
            (
                c.id.clone(),
                c.predicate.clone(),
                String::new(),
                String::new(),
            )
        } else {
            (
                root(&parent, &c.subject.id),
                c.predicate.clone(),
                object.0,
                object.1,
            )
        };
        match index.get(&key) {
            Some(&item) if !parent.is_empty() => items[item].push(summary),
            _ => {
                index.insert(key, items.len());
                items.push(vec![summary]);
            }
        }
    }
    let truncated = items.len() > limit;
    items.truncate(limit);
    let groups = items
        .iter()
        .filter(|members| members.len() > 1)
        .map(|members| {
            let members: Vec<_> = members
                .iter()
                .map(|m| GroupMember {
                    claim: m.summary.claim.id.clone(),
                    status: m.summary.claim.status.clone(),
                    revision: m.summary.claim.revision.clone(),
                })
                .collect();
            let pairs: Vec<_> = members
                .iter()
                .map(|m| (m.claim.clone(), m.revision.clone()))
                .collect();
            ClaimGroup {
                status: if members.iter().any(|m| m.status == "observed") {
                    "observed".into()
                } else {
                    "superseded".into()
                },
                revision: crate::claims::group_revision(&pairs),
                members,
            }
        })
        .collect();
    About {
        resolved,
        claims: items.into_iter().flatten().collect(),
        truncated,
        coverage,
        extraction,
        groups,
        person,
    }
}
impl<S: ReadStorage> Reader<S> {
    pub fn about(&mut self, entity: &str, options: &AboutOptions) -> Result<About> {
        validate_entity(entity)?;
        ensure!(
            (1..=50).contains(&options.limit),
            "limit must be from 1 to 50"
        );
        read_only::snapshot_live(
            &self.locations,
            &mut self.storage,
            &self.scopes,
            |view, unfinished, live| {
                let claims = match view {
                    Some(view) => view.about_view(entity, options, &self.scopes)?,
                    None => AboutClaims::default(),
                };
                let coverage = read_only::coverage_of(
                    view,
                    unfinished,
                    &self.config.sources,
                    None,
                    false,
                    &self.scopes,
                )?;
                let extraction = extraction_of(
                    view,
                    &self.locations,
                    &self.config.sources,
                    &coverage,
                    live,
                    &self.scopes,
                )?;
                Ok(answer(claims, options.limit, coverage, extraction))
            },
        )
    }
}
impl<S: Storage> Rastro<S> {
    pub fn about(&self, entity: &str, options: &AboutOptions) -> Result<About> {
        validate_entity(entity)?;
        ensure!(
            (1..=50).contains(&options.limit),
            "limit must be from 1 to 50"
        );
        self.storage.snapshot(|view| {
            let coverage =
                read_only::coverage_of(Some(view), &[], &self.config.sources, None, false, &[])?;
            let extraction = extraction_of(
                Some(view),
                &self.locations,
                &self.config.sources,
                &coverage,
                true,
                &[],
            )?;
            Ok(answer(
                view.about_view(entity, options, &[])?,
                options.limit,
                coverage,
                extraction,
            ))
        })
    }
}

/// A complete derivation. The limit counts bundles, never individual required spans.
#[derive(Debug, JsonSchema, Serialize)]
pub struct JustificationView {
    pub justification_id: String,
    pub justification_kind: String,
    pub extractor: String,
    pub extractor_version: String,
    pub extraction_rule: String,
    pub vocabulary_version: String,
    /// False for a historical justification that no longer supports the current claim.
    pub current: bool,
    pub spans: Vec<Citation>,
    /// The owner intent behind a justification of kind `owner_intent`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<IntentCitation>,
}
/// The owner intent a justification rests on: its id, when it was issued and by whom.
#[derive(Debug, JsonSchema, Serialize)]
pub struct IntentCitation {
    pub id: String,
    pub issued_at: DateTime<Utc>,
    pub issuer: crate::identity::IssuerContext,
}
#[derive(Debug, JsonSchema, Serialize)]
pub struct Citation {
    pub evidence_id: i64,
    pub portable_key: String,
    pub source: String,
    pub scope: crate::Scope,
    pub location: String,
    pub field: String,
    pub start: u64,
    pub end: u64,
    pub captured_at: DateTime<Utc>,
    pub source_updated_at: Option<DateTime<Utc>>,
    pub trust_class: crate::TrustClass,
    pub excerpt: CitationExcerpt,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
}
/// A UTF-8-safe prefix of a cited span; no search query or matching word is involved.
#[derive(Debug, JsonSchema, Serialize)]
pub struct CitationExcerpt {
    /// Byte offset of the excerpt in the cited field.
    pub start: u64,
    /// Byte offset just past the excerpt in the cited field.
    pub end: u64,
    pub text: String,
    pub clipped_before: bool,
    pub clipped_after: bool,
}
impl CitationExcerpt {
    pub fn new(quote: &str, origin: u64) -> Self {
        let excerpt = crate::search::excerpt(quote, None, origin);
        Self {
            start: excerpt.start,
            end: excerpt.end,
            text: excerpt.text,
            clipped_before: excerpt.clipped_before,
            clipped_after: excerpt.clipped_after,
        }
    }
}
#[derive(Debug, JsonSchema, Serialize)]
pub struct Explanation {
    pub claim: Box<ClaimSummary>,
    pub supports: Vec<JustificationView>,
    pub truncated: bool,
    pub coverage: Vec<Coverage>,
    pub extraction: Vec<crate::runs::ExtractionPair>,
}
#[derive(Clone, Copy, Debug)]
pub struct WhyOptions {
    pub limit: usize,
    pub full: bool,
}
impl Default for WhyOptions {
    fn default() -> Self {
        Self {
            limit: 20,
            full: false,
        }
    }
}
impl<S: ReadStorage> Reader<S> {
    pub fn explain(&mut self, claim: &str, options: &WhyOptions) -> Result<Explanation> {
        ensure!(
            (1..=50).contains(&options.limit),
            "limit must be from 1 to 50"
        );
        read_only::snapshot_live(
            &self.locations,
            &mut self.storage,
            &self.scopes,
            |view, unfinished, live| {
                let mut why = view
                    .map(|view| view.explain_scoped(claim, options, &self.scopes))
                    .transpose()?
                    .flatten()
                    .ok_or_else(|| anyhow::anyhow!("unknown claim: {claim}"))?;
                why.coverage = read_only::coverage_of(
                    view,
                    unfinished,
                    &self.config.sources,
                    None,
                    false,
                    &self.scopes,
                )?;
                why.extraction = extraction_of(
                    view,
                    &self.locations,
                    &self.config.sources,
                    &why.coverage,
                    live,
                    &self.scopes,
                )?;
                Ok(why)
            },
        )
    }
}
impl<S: Storage> Rastro<S> {
    pub fn explain(&self, claim: &str, options: &WhyOptions) -> Result<Explanation> {
        ensure!(
            (1..=50).contains(&options.limit),
            "limit must be from 1 to 50"
        );
        self.storage.snapshot(|view| {
            let mut why = view
                .explain(claim, options)?
                .ok_or_else(|| anyhow::anyhow!("unknown claim: {claim}"))?;
            why.coverage =
                read_only::coverage_of(Some(view), &[], &self.config.sources, None, false, &[])?;
            why.extraction = extraction_of(
                Some(view),
                &self.locations,
                &self.config.sources,
                &why.coverage,
                true,
                &[],
            )?;
            Ok(why)
        })
    }
}

/// Pairs come from coverage, so never-looked and historical scopes also get an extraction state.
fn extraction_of(
    view: Option<&dyn crate::Snapshot>,
    locations: &crate::Locations,
    config: &[crate::Source],
    coverage: &[Coverage],
    live: bool,
    scopes: &[crate::Scope],
) -> Result<Vec<crate::runs::ExtractionPair>> {
    use crate::runs::{ExtractionFacts, ExtractionPair, ExtractionStatus, extraction_pair};
    let bindings = view
        .map(|view| view.extraction_sources(scopes))
        .transpose()?
        .unwrap_or_default();
    coverage
        .iter()
        .map(|pair| {
            let stored = bindings
                .iter()
                .find(|(source, _)| source.name == pair.source && source.scope == pair.scope)
                .or_else(|| {
                    bindings
                        .iter()
                        .find(|(source, _)| source.name == pair.source)
                });
            let configured = config.iter().find(|source| {
                source.name == pair.source && (scopes.is_empty() || scopes.contains(&source.scope))
            });
            let default = configured
                .and_then(|source| {
                    crate::host::discover(locations, &source.connector, "connector").ok()
                })
                .and_then(|plugin| plugin.manifest.default_extractor);
            let source = stored.map(|(source, _)| source).or(configured);
            let extractor = stored
                .map(|(_, extractor)| extractor.as_str())
                .or(default.as_deref());
            let Some((source, extractor)) = source.zip(extractor) else {
                return Ok(ExtractionPair {
                    source: pair.source.clone(),
                    scope: pair.scope.0.clone(),
                    extractor: String::new(),
                    extractor_version: None,
                    state: ExtractionStatus::NotInstalled,
                });
            };
            let mut source = source.clone();
            source.scope = pair.scope.clone();
            extraction_pair(
                locations,
                &source,
                extractor,
                live,
                |version, vocabulary_version| {
                    view.map(|view| {
                        view.extraction_facts(&source, extractor, version, vocabulary_version)
                    })
                    .unwrap_or_else(|| Ok(ExtractionFacts::default()))
                },
            )
        })
        .collect()
}
