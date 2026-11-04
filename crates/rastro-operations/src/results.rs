//! Result types: what each operation returns, serialised into the envelope's `result`.
use crate::{envelope, requests::*};
use rastro_core::{
    Coverage, Source,
    runs::{RunReport, SearchResult},
};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, JsonSchema, Serialize)]
pub struct SourceAddResult {
    pub source: Source,
}
#[derive(Debug, JsonSchema, Serialize)]
pub struct SourceListResult {
    pub sources: Vec<Source>,
}
#[derive(Debug, JsonSchema, Serialize)]
pub struct CoverageResult {
    pub coverage: Vec<Coverage>,
}
#[derive(Debug, JsonSchema, Serialize)]
pub struct SyncResult {
    pub runs: Vec<RunReport>,
}
/// Matches, plus the coverage of every source searched.
#[derive(Debug, JsonSchema, Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub coverage: Vec<Coverage>,
}
#[derive(Debug, JsonSchema, Serialize)]
pub struct IndexRebuildResult {
    pub indexed: usize,
}

/// What an operation returned.
#[derive(Debug)]
pub enum Response {
    SourceAdd(SourceAddResult),
    SourceList(SourceListResult),
    Coverage(CoverageResult),
    Sync(SyncResult),
    Search(SearchResponse),
    IndexRebuild(IndexRebuildResult),
}

impl Response {
    pub fn operation(&self) -> &'static str {
        match self {
            Self::SourceAdd(_) => SourceAddRequest::NAME,
            Self::SourceList(_) => SourceListRequest::NAME,
            Self::Coverage(_) => CoverageRequest::NAME,
            Self::Sync(_) => SyncRequest::NAME,
            Self::Search(_) => SearchRequest::NAME,
            Self::IndexRebuild(_) => IndexRebuildRequest::NAME,
        }
    }
    /// The `result` member of the envelope.
    pub fn result(&self) -> Value {
        match self {
            Self::SourceAdd(r) => serde_json::to_value(r),
            Self::SourceList(r) => serde_json::to_value(r),
            Self::Coverage(r) => serde_json::to_value(r),
            Self::Sync(r) => serde_json::to_value(r),
            Self::Search(r) => serde_json::to_value(r),
            Self::IndexRebuild(r) => serde_json::to_value(r),
        }
        .expect("results serialise")
    }
    pub fn envelope(&self) -> Value {
        envelope::success(self.operation(), &self.result())
    }
    /// False when a sync ran a source that failed.
    pub fn succeeded(&self) -> bool {
        match self {
            Self::Sync(r) => r.runs.iter().all(|run| run.status == "ok"),
            _ => true,
        }
    }
}
