//! Result types: what each operation returns, serialised into the envelope's `result`.
pub use crate::catalog::Response;
use crate::envelope;
use rastro_core::identity::{IntentView, Issued};
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
    /// Every (source, scope) pair the rebuild visited.
    pub pairs: Vec<rastro_core::runs::RebuildPair>,
}
/// A claim and bounded complete justifications, each citing every required span.
pub type WhyResult = rastro_core::knowledge::Explanation;

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
            Self::ClaimsRebuild(r) => r.pairs.iter().all(|p| p.state != ExtractionState::Failed),
            _ => true,
        }
    }
}

pub type AboutResult = rastro_core::knowledge::About;

/// A Person, by its entity id and canonical identifier.
#[derive(Debug, JsonSchema, Serialize)]
pub struct PersonRef {
    pub id: String,
    pub identifier: String,
}
/// What `identity me` and `identity person create` did.
#[derive(Debug, JsonSchema, Serialize)]
pub struct IdentityPersonResult {
    pub person: PersonRef,
    /// False when the Person already existed and nothing was recorded.
    pub created: bool,
    /// The intents accepted, in acceptance order.
    pub intents: Vec<IntentView>,
}
impl IdentityPersonResult {
    pub(crate) fn of(issued: Issued) -> anyhow::Result<Self> {
        let person = issued
            .person
            .ok_or_else(|| anyhow::anyhow!("an identity write named no Person"))?;
        Ok(Self {
            person: PersonRef {
                id: person.id,
                identifier: person.identifier,
            },
            created: issued.created,
            intents: views(&issued.intents),
        })
    }
}
/// The intents an identity write accepted, in acceptance order; none when it was already in force.
#[derive(Debug, JsonSchema, Serialize)]
pub struct IdentityWriteResult {
    pub intents: Vec<IntentView>,
}
impl IdentityWriteResult {
    pub(crate) fn of(issued: Issued) -> Self {
        Self {
            intents: views(&issued.intents),
        }
    }
}
/// Freshly accepted intents stand as written.
fn views(intents: &[rastro_core::identity::Intent]) -> Vec<IntentView> {
    let resolution = rastro_core::identity::Resolution::of(intents);
    intents
        .iter()
        .map(|i| IntentView::of(i, &resolution))
        .collect()
}
pub type IdentityHistoryResult = rastro_core::identity::History;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CATALOG;
    use rastro_core::{Scope, Source};

    fn person() -> IdentityPersonResult {
        IdentityPersonResult {
            person: PersonRef {
                id: "e_1".into(),
                identifier: "rastro:person/1".into(),
            },
            created: true,
            intents: vec![],
        }
    }

    /// surfaces/R2: a request, its response and the catalog give an operation the same name
    #[test]
    #[allow(non_snake_case)]
    fn request_response_and_catalog_agree_on_names() {
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
                claim: Box::new(rastro_core::knowledge::ClaimSummary {
                    subject_aliases: vec![],
                    object_aliases: vec![],
                    support_count: 0,
                    claim: rastro_core::claims::ClaimView {
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
                    },
                }),
                supports: vec![],
                truncated: false,
                coverage: vec![],
                extraction: vec![],
            }),
            Response::ClaimsRebuild(ClaimsRebuildResult {
                claims: 0,
                pairs: vec![],
            }),
            Response::About(AboutResult {
                claims: vec![],
                truncated: false,
                coverage: vec![],
                extraction: vec![],
                groups: vec![],
            }),
            Response::IdentityMe(person()),
            Response::IdentityPersonCreate(person()),
            Response::IdentityLink(IdentityWriteResult { intents: vec![] }),
            Response::IdentityMerge(IdentityWriteResult { intents: vec![] }),
            Response::IdentityUndo(IdentityWriteResult { intents: vec![] }),
            Response::IdentityHistory(IdentityHistoryResult {
                entity: None,
                intents: vec![],
            }),
        ];
        let arguments = |name: &str| match name {
            "source.add" => {
                serde_json::json!({"name": "a", "connector": "github", "repo": "o/r", "scope": "w"})
            }
            "search" => serde_json::json!({"text": "x"}),
            "why" => serde_json::json!({"claim": "c"}),
            "about" => serde_json::json!({"entity": "github:account/1"}),
            "identity.person.create" => serde_json::json!({"name": "Ana"}),
            "identity.link" => serde_json::json!({"account": "github:login/a", "person": "me"}),
            "identity.merge" => serde_json::json!({"person": "me", "other": "rastro:name/Ana"}),
            "identity.undo" => serde_json::json!({"intent": "i_1"}),
            "identity.history" => serde_json::json!({"entity": "me"}),
            _ => serde_json::json!({}),
        };
        assert_eq!(CATALOG.len(), responses.len());
        for (spec, response) in CATALOG.iter().zip(&responses) {
            assert_eq!(response.operation(), spec.name);
            let request = spec.decode(arguments(spec.name)).unwrap();
            assert_eq!(request.name(), spec.name);
            assert!(spec.decode(serde_json::json!({"scope": "x"})).is_err());
        }
    }
}
