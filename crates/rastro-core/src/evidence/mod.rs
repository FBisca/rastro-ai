//! Evidence: validated observations the store appends and never changes (ADR 0005).
use crate::Scope;
use chrono::{DateTime, Utc};
use rastro_plugin_sdk::{EvidenceSubmit, Manifest, Window};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TrustClass {
    #[serde(rename = "structured")]
    Structured,
    #[serde(rename = "unstructured")]
    Unstructured,
    #[serde(rename = "untrusted content")]
    UntrustedContent,
}
impl TrustClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Structured => "structured",
            Self::Unstructured => "unstructured",
            Self::UntrustedContent => "untrusted content",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        [Self::Structured, Self::Unstructured, Self::UntrustedContent]
            .into_iter()
            .find(|class| class.as_str() == text)
    }
}

/// One observation of one field at one location: a half-open UTF-8 byte span and its quote.
#[derive(Clone, Debug, Serialize)]
pub struct Evidence {
    pub source: String,
    pub scope: Scope,
    pub location: String,
    pub field: String,
    pub start: u64,
    pub end: u64,
    pub quote: String,
    pub trust_class: TrustClass,
    pub captured_at: DateTime<Utc>,
    pub source_updated_at: Option<DateTime<Utc>>,
}

/// A stored observation that matched a search.
#[derive(Clone, Debug, Serialize)]
pub struct Hit {
    pub id: i64,
    #[serde(flatten)]
    pub evidence: Evidence,
}

/// What a source may submit: the manifest narrowed by the source, bound to its scope.
pub struct Grant {
    pub source: String,
    pub scope: Scope,
    pub kinds: BTreeSet<String>,
    pub window: Window,
}
impl Grant {
    /// `None` when the manifest doesn't declare the source's scope.
    pub fn new(manifest: &Manifest, source: &str, scope: &Scope, window: &Window) -> Option<Self> {
        manifest
            .permissions
            .scopes
            .contains(&scope.0)
            .then(|| Self {
                source: source.into(),
                scope: scope.clone(),
                kinds: manifest.permissions.submit.iter().cloned().collect(),
                window: window.clone(),
            })
    }
    /// A message may repeat the bound window, never name another.
    pub fn allows_window(&self, window: &Option<Window>) -> bool {
        let instants = |w: &Window| {
            Some((
                w.from.parse::<DateTime<Utc>>().ok()?,
                w.to.parse::<DateTime<Utc>>().ok()?,
            ))
        };
        window
            .as_ref()
            .is_none_or(|w| instants(w).is_some() && instants(w) == instants(&self.window))
    }
}

/// Checks a submission against the grant and the span rules. `Err` carries the reason.
pub fn validate(
    submit: EvidenceSubmit,
    grant: &Grant,
    captured_at: DateTime<Utc>,
) -> Result<Evidence, String> {
    let kind = submit.kind.ok_or("missing kind")?;
    if !grant.kinds.contains(&kind) {
        return Err(format!("kind not permitted: {kind}"));
    }
    if submit.source.is_some_and(|s| s != grant.source) {
        return Err("source differs from the bound source".into());
    }
    if submit.scope.is_some_and(|s| s != grant.scope.0) {
        return Err("scope differs from the bound scope".into());
    }
    if !grant.allows_window(&submit.window) {
        return Err("window differs from the bound window".into());
    }
    let location = submit
        .location
        .filter(|l| !l.is_empty())
        .ok_or("missing location")?;
    let field = submit
        .field
        .filter(|f| !f.is_empty())
        .ok_or("missing field")?;
    let (start, end) = submit.start.zip(submit.end).ok_or("missing span")?;
    let quote = submit
        .quote
        .filter(|q| !q.is_empty())
        .ok_or("missing quote")?;
    if end.checked_sub(start) != Some(quote.len() as u64) {
        return Err("quote length in UTF-8 bytes differs from end - start".into());
    }
    let trust_class = submit.trust_class.ok_or("missing trust class")?;
    let trust_class = TrustClass::parse(&trust_class).ok_or("unknown trust class")?;
    let source_updated_at = submit
        .source_updated_at
        .map(|t| t.parse().map_err(|_| "invalid source_updated_at"))
        .transpose()?;
    Ok(Evidence {
        source: grant.source.clone(),
        scope: grant.scope.clone(),
        location,
        field,
        start,
        end,
        quote,
        trust_class,
        captured_at,
        source_updated_at,
    })
}
