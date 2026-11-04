//! Result types: what each operation returns, serialised into the envelope's `result`.
use crate::{envelope, requests::*};
use rastro_core::{Coverage, Source, runs::RunReport, search::SearchResult};
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
    /// More hits matched than `limit` allowed.
    pub truncated: bool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CATALOG;
    use rastro_core::{Scope, Source};

    /// S2: a request, its response and the catalog give an operation the same name
    #[test]
    #[allow(non_snake_case)]
    fn S2_request_response_and_catalog_agree_on_names() {
        let responses = [
            Response::SourceAdd(SourceAddResult {
                source: Source::github("a".into(), "o/r".into(), Scope("w".into())),
            }),
            Response::SourceList(SourceListResult { sources: vec![] }),
            Response::Coverage(CoverageResult { coverage: vec![] }),
            Response::Sync(SyncResult { runs: vec![] }),
            Response::Search(SearchResponse {
                query: String::new(),
                results: vec![],
                truncated: false,
                coverage: vec![],
            }),
            Response::IndexRebuild(IndexRebuildResult { indexed: 0 }),
        ];
        let arguments = |name: &str| match name {
            "source.add" => {
                serde_json::json!({"name": "a", "connector": "github", "repo": "o/r", "scope": "w"})
            }
            "search" => serde_json::json!({"text": "x"}),
            _ => serde_json::json!({}),
        };
        for (spec, response) in CATALOG.iter().zip(&responses) {
            assert_eq!(response.operation(), spec.name);
            let request = spec.decode(arguments(spec.name)).unwrap();
            assert_eq!(request.name(), spec.name);
            assert!(spec.decode(serde_json::json!({"scope": "x"})).is_err());
        }
    }
}
