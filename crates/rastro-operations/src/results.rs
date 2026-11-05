//! Result types: what each operation returns, serialised into the envelope's `result`.
pub use crate::catalog::Response;
use crate::envelope;
use rastro_core::claims::{ClaimView, SupportView};
use rastro_core::runs::{ExtractionState, RunReport};
use rastro_core::{Coverage, Source, search::SearchResult};
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
#[derive(Debug, JsonSchema, Serialize)]
pub struct ClaimsRebuildResult {
    pub claims: usize,
}
/// A claim and every support it has, each citing its evidence.
#[derive(Debug, JsonSchema, Serialize)]
pub struct WhyResult {
    pub claim: Box<ClaimView>,
    pub supports: Vec<SupportView>,
}

impl Response {
    pub fn envelope(&self) -> Value {
        envelope::success(self.operation(), &self.result())
    }
    /// False when a sync ran a source, or its extractor, that failed.
    pub fn succeeded(&self) -> bool {
        match self {
            Self::Sync(r) => r.runs.iter().all(|run| {
                run.status == "ok"
                    && run
                        .extraction
                        .as_ref()
                        .is_none_or(|e| e.state != ExtractionState::Failed)
            }),
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CATALOG;
    use rastro_core::{Scope, Source};

    /// surfaces/R2: a request, its response and the catalog give an operation the same name
    #[test]
    #[allow(non_snake_case)]
    fn S26_request_response_and_catalog_agree_on_names() {
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
            Response::Why(WhyResult {
                claim: Box::new(ClaimView {
                    id: "c".into(),
                    subject: rastro_core::claims::Entity::new("s".into(), "t".into()),
                    predicate: "p".into(),
                    object: rastro_core::claims::Object::Literal {
                        value: "v".into(),
                        datatype: "string".into(),
                    },
                    status: "observed".into(),
                    revision: "r".into(),
                    known_intervals: vec![],
                    valid_time: rastro_core::claims::ValidTime::Unknown,
                    conflicts_with: vec![],
                    supersedes: vec![],
                    superseded_by: vec![],
                }),
                supports: vec![],
            }),
            Response::ClaimsRebuild(ClaimsRebuildResult { claims: 0 }),
        ];
        let arguments = |name: &str| match name {
            "source.add" => {
                serde_json::json!({"name": "a", "connector": "github", "repo": "o/r", "scope": "w"})
            }
            "search" => serde_json::json!({"text": "x"}),
            "why" => serde_json::json!({"claim": "c"}),
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
