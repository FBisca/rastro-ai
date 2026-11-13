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
/// `warnings` is always written, but earlier envelopes without it stay valid.
fn warnings_optional(schema: &mut schemars::Schema) {
    if let Some(required) = schema
        .get_mut("required")
        .and_then(serde_json::Value::as_array_mut)
    {
        required.retain(|name| name != "warnings");
    }
}
#[derive(Debug, JsonSchema, Serialize)]
#[schemars(transform = warnings_optional)]
pub struct SyncResult {
    pub runs: Vec<RunReport>,
    /// What the sync warned of before it read, such as a renamed source reading its whole
    /// window again. Empty when there is nothing to warn of.
    pub warnings: Vec<String>,
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
/// Every source's commitment hypotheses, each source with its extraction and coverage state.
pub type CommitmentsResult = rastro_core::commitments::Commitments;
/// A meeting's commitment hypotheses with extraction and coverage state.
pub type MeetingResult = rastro_core::meeting::MeetingCommitments;
/// What forgetting a meeting removed.
pub type MeetingForgetResult = rastro_core::meeting::MeetingForgotten;
/// A meeting as the owner supplied it, without recorded model output.
pub type MeetingExportResult = rastro_core::meeting::MeetingExport;

/// A claim and bounded complete justifications, each citing every required span.
pub type WhyResult = rastro_core::knowledge::Explanation;

impl Response {
    pub fn envelope(&self) -> Value {
        envelope::success(self.operation(), &self.result())
    }
    /// False when a sync ran a source, or its extractor, that failed, or a meeting import's or replay's run failed.
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
            Self::MeetingImport(r) | Self::MeetingReplay(r) => {
                r.extraction.state == rastro_core::meeting::RunState::Succeeded
            }
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

/// The fuzzy candidates, computed on read, each with its two accounts and its reasons.
#[derive(Debug, JsonSchema, Serialize)]
pub struct IdentityCandidatesResult {
    pub candidates: Vec<rastro_core::identity::Candidate>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CATALOG;
    use rastro_core::{Scope, Source};

    fn meeting() -> MeetingResult {
        use rastro_core::meeting::*;
        let at = "2026-11-10T10:00:00Z".parse().unwrap();
        MeetingResult {
            meeting: MeetingRef {
                id: "m".into(),
                scope: "w".into(),
                captured_at: at,
            },
            notice: NOTICE.into(),
            extraction: Extraction {
                state: RunState::Succeeded,
                cause: None,
                reason: None,
                run: 1,
                finished_at: at,
                extractor: None,
                replay_of: None,
                excluded: 0,
            },
            coverage: MeetingCoverage {
                state: rastro_core::CoverageState::Empty,
                turns_read: 0,
                turns_total: 0,
                reason: None,
            },
            stale: false,
            results_from: None,
            transcript_retained: false,
            hypotheses: vec![],
        }
    }

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
            Response::Sync(SyncResult {
                runs: vec![],
                warnings: vec![],
            }),
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
                    subject_label: None,
                    object_label: None,
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
                resolved: false,
                claims: vec![],
                truncated: false,
                coverage: vec![],
                extraction: vec![],
                groups: vec![],
                person: None,
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
            Response::IdentityCandidates(IdentityCandidatesResult { candidates: vec![] }),
            Response::IdentityAnswer(IdentityWriteResult { intents: vec![] }),
            Response::MeetingImport(meeting()),
            Response::MeetingCommitments(meeting()),
            Response::MeetingReplay(meeting()),
            Response::MeetingForget(rastro_core::meeting::MeetingForgotten {
                meeting: "m".into(),
                removed: Default::default(),
            }),
            Response::MeetingExport(rastro_core::meeting::MeetingExport {
                format: rastro_core::meeting::EXPORT_FORMAT.into(),
                notice: rastro_core::meeting::EXPORT_NOTICE.into(),
                meeting: meeting().meeting,
                owner_speaker: None,
                speakers: vec![],
                transcript: None,
            }),
            Response::Commitments(rastro_core::commitments::Commitments {
                notice: rastro_core::commitments::NOTICE.into(),
                sources: vec![],
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
            "meeting.import" => serde_json::json!({"file": "t.txt", "meeting": "m", "scope": "w",
                "captured_at": "2026-11-10T10:00:00Z"}),
            "meeting.commitments" | "meeting.replay" | "meeting.forget" | "meeting.export" => {
                serde_json::json!({"meeting": "m"})
            }
            "identity.link" => serde_json::json!({"account": "github:login/a", "person": "me"}),
            "identity.merge" => serde_json::json!({"person": "me", "other": "rastro:name/Ana"}),
            "identity.undo" => serde_json::json!({"intent": "i_1"}),
            "identity.history" => serde_json::json!({"entity": "me"}),
            "identity.answer" => {
                serde_json::json!({"account": "github:login/a", "other": "linear:user/u", "verdict": "no"})
            }
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
