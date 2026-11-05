//! Bounded claim reads. Unscoped reads use the published state; surfaces supply no scopes yet.
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
    pub claims: Vec<ClaimSummary>,
    pub truncated: bool,
    pub coverage: Vec<Coverage>,
    pub extraction: Vec<crate::runs::ExtractionPair>,
}
fn validate_entity(entity: &str) -> Result<()> {
    let id = entity
        .strip_prefix("e_")
        .is_some_and(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()));
    let canonical = entity
        .split_once(':')
        .is_some_and(|(prefix, value)| !prefix.is_empty() && !value.is_empty())
        && !entity.chars().any(char::is_whitespace);
    ensure!(
        id || canonical,
        "entity must be an entity id, canonical alias or exact canonical URL"
    );
    Ok(())
}
fn answer(
    mut claims: Vec<ClaimSummary>,
    limit: usize,
    coverage: Vec<Coverage>,
    extraction: Vec<crate::runs::ExtractionPair>,
) -> About {
    let truncated = claims.len() > limit;
    claims.truncate(limit);
    About {
        claims,
        truncated,
        coverage,
        extraction,
    }
}
impl<S: ReadStorage> Reader<S> {
    pub fn about(&mut self, entity: &str, options: &AboutOptions) -> Result<About> {
        validate_entity(entity)?;
        ensure!(
            self.scopes.is_empty(),
            "scoped claim reads are not available yet"
        );
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
                    Some(view) => view.about(entity, options)?,
                    None => vec![],
                };
                let coverage = read_only::coverage_of(
                    view,
                    unfinished,
                    &self.config.sources,
                    None,
                    false,
                    &self.scopes,
                )?;
                let extraction =
                    extraction_of(view, &self.locations, &self.config.sources, &coverage, live)?;
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
            )?;
            Ok(answer(
                view.about(entity, options)?,
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
            self.scopes.is_empty(),
            "scoped claim reads are not available yet"
        );
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
                    .map(|view| view.explain(claim, options))
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
) -> Result<Vec<crate::runs::ExtractionPair>> {
    use crate::runs::{ExtractionFacts, ExtractionPair, ExtractionStatus, extraction_pair};
    let bindings = view
        .map(|view| view.extraction_sources(&[]))
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
            let configured = config.iter().find(|source| source.name == pair.source);
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
            extraction_pair(locations, &source, extractor, live, |version| {
                view.map(|view| view.extraction_facts(&source, extractor, version))
                    .unwrap_or_else(|| Ok(ExtractionFacts::default()))
            })
        })
        .collect()
}
