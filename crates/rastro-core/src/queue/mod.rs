//! The queue: every pending proposal and question the caller can see, in one list.
//!
//! A proposal is a commitment hypothesis an agent tool adapter proposed from a conversation
//! (slice 5's [`crate::commitments`]); a question is a pair of accounts identity holds as a
//! same-person candidate (slice 4's [`crate::identity`]). The queue invents no pending-item
//! object beyond those two, and it never says a commitment is open, done or tracked.
//!
//! Each item carries a **content-addressed id**: the hash of its canonical encoding, so a
//! re-import or a replay of the same content keeps its id, and an item whose content changed is
//! a different item. The list is oldest first, ordered by the time its content belongs to — a
//! meeting's capture time. Questions are computed on read and have no such time, so they come
//! first, ordered by their accounts' identifiers.
//!
//! A source that found nothing, failed or couldn't look is never left out silently: its gap is
//! named beside the items. A source in a scope the caller can't see is absent, items and gap
//! alike, and the read still succeeds. Same-person candidates are computed over every scope, so
//! a caller whose launch scopes are restricted is shown none of them.
use crate::commitments::{self, Hypothesis};
use crate::identity::{
    AccountFacts, CandidateAccount, CandidateReason, Resolution, candidate_pairs,
};
use crate::meeting::{Cause, RunState};
use crate::{CoverageState, Rastro, ReadStorage, Reader, Storage};
use anyhow::Result;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// What every read says first: these are not commitments Rastro knows, and nothing is linked.
pub const NOTICE: &str = "The proposals and questions Rastro is waiting on you for. A proposal is a hypothesis an adapter proposed from a conversation, not a commitment Rastro knows: it says nothing about whether one is open, done or tracked. A question is two accounts that may be one person; nothing is linked until you answer.";

/// The name the identity graph's source carries. Identity is not a connector and no scope bounds
/// it, so its items name a source of kind `identity`, this name and an empty scope.
pub const IDENTITY_SOURCE: &str = "identity";

/// A source of hypotheses, as the queue's items and gaps name it.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "QueueSource")]
pub struct Source {
    pub kind: SourceKind,
    pub name: String,
    pub scope: String,
}
impl Source {
    /// The source a commitments list entry comes from.
    fn of(source: &commitments::Source) -> Self {
        Self {
            kind: match source.kind {
                commitments::SourceKind::Meeting => SourceKind::Meeting,
            },
            name: source.name.clone(),
            scope: source.scope.clone(),
        }
    }
    fn identity() -> Self {
        Self {
            kind: SourceKind::Identity,
            name: IDENTITY_SOURCE.into(),
            scope: String::new(),
        }
    }
}

/// What kind of source an item or a gap comes from.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "QueueSourceKind")]
pub enum SourceKind {
    /// An imported conversation, listed by the `commitments` list.
    Meeting,
    /// The identity graph's same-person candidates, computed on read.
    Identity,
}

/// Whether an item waits for the owner as a proposal or as a question.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "QueueItemKind")]
pub enum ItemKind {
    Proposal,
    Question,
}

/// One account of a question: its entity id, canonical identifier and the current names it goes
/// by, sorted.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "QueueAccount")]
pub struct Account {
    pub id: String,
    pub identifier: String,
    pub names: Vec<String>,
}

/// Two accounts that may be one person, and the signals that say so.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "QueueQuestion")]
pub struct Question {
    /// The two accounts, in identifier order.
    pub accounts: [Account; 2],
    pub reasons: Vec<CandidateReason>,
    /// The name a yes gives the Person it creates when neither account is held.
    pub name: String,
}

/// One pending proposal or question.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "QueueItem")]
pub struct Item {
    /// The content address of the item's canonical encoding.
    pub id: String,
    pub kind: ItemKind,
    pub source: Source,
    /// The proposed commitment; present exactly when `kind` is `proposal`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal: Option<Hypothesis>,
    /// The same-person candidate; present exactly when `kind` is `question`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<Question>,
}

/// What a source's gap is.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "QueueGapState")]
pub enum GapState {
    /// The source's latest run looked and proposed nothing.
    FoundNothing,
    /// The source's latest run failed; `cause` says why.
    Failed,
    /// The source couldn't be looked at; `reason` says why.
    CouldNotLook,
}

/// Why a source shows no item, or fewer than its content now says.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "QueueGap")]
pub struct Gap {
    pub source: Source,
    pub state: GapState,
    /// The run that found nothing, failed or couldn't look.
    pub run: String,
    /// Why the run failed, when it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<Cause>,
    /// What went wrong, when the run recorded it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A source of proposals and the time its content belongs to.
#[derive(Clone, Debug)]
pub struct Origin {
    /// A meeting's capture time: not when it was imported.
    pub at: DateTime<Utc>,
    pub source: commitments::Source,
}

/// What `queue` returns.
#[derive(Clone, Debug, JsonSchema, Serialize)]
pub struct Queue {
    pub notice: String,
    /// Every pending proposal and question, oldest first.
    pub items: Vec<Item>,
    /// What each source that showed no item, or fewer than it should, has to say.
    pub gaps: Vec<Gap>,
}

impl Queue {
    pub fn empty() -> Self {
        Self {
            notice: NOTICE.into(),
            items: Vec::new(),
            gaps: Vec::new(),
        }
    }

    /// Assembles the queue from the sources storage keeps and the identity graph's candidates.
    /// An item whose source is hidden never reaches here: storage filters the sources before
    /// this reads them. Identical items collapse to one, so no candidate is shown twice.
    pub fn new(origins: Vec<Origin>, questions: Vec<Question>) -> Self {
        let mut aged = Vec::new();
        let mut gaps = Vec::new();
        let mut seen = BTreeSet::new();
        for origin in origins {
            let source = Source::of(&origin.source);
            for hypothesis in &origin.source.hypotheses {
                let item = Item {
                    id: proposal_id(&source, hypothesis),
                    kind: ItemKind::Proposal,
                    source: source.clone(),
                    proposal: Some(hypothesis.clone()),
                    question: None,
                };
                if seen.insert(item.id.clone()) {
                    aged.push(Aged {
                        at: Some(origin.at),
                        question: false,
                        turn: hypothesis.quote.turn,
                        item,
                    });
                }
            }
            if let Some(gap) = gap_of(&origin.source) {
                gaps.push(gap);
            }
        }
        for question in questions {
            let item = Item {
                id: question_id(&question),
                kind: ItemKind::Question,
                source: Source::identity(),
                proposal: None,
                question: Some(question),
            };
            if seen.insert(item.id.clone()) {
                // Computed on read, so a question has no time: it is older than every proposal
                // and sorts before one on that tie.
                aged.push(Aged {
                    at: None,
                    question: true,
                    turn: 0,
                    item,
                });
            }
        }
        aged.sort_by(|a, b| a.key().cmp(&b.key()));
        Self {
            notice: NOTICE.into(),
            items: aged.into_iter().map(|aged| aged.item).collect(),
            gaps,
        }
    }
}

/// An item with the key the queue's order sorts by.
struct Aged {
    /// When the item's content belongs to; a question has none.
    at: Option<DateTime<Utc>>,
    /// Questions first, then proposals.
    question: bool,
    /// The quote's turn, which orders one source's proposals.
    turn: usize,
    item: Item,
}
impl Aged {
    /// Oldest first, questions before proposals, then by turn and id, so the order is stable.
    fn key(&self) -> (Option<DateTime<Utc>>, bool, usize, &str) {
        (self.at, self.question, self.turn, &self.item.id)
    }
}

/// What a source has to say instead of an item.
fn gap_of(source: &commitments::Source) -> Option<Gap> {
    let extraction = &source.extraction;
    let named = Source::of(source);
    if extraction.state == RunState::Failed {
        // A content that could not be read at all is `couldn't look`; a run that read it and
        // still failed is a failed run, named with its cause.
        let unreadable = matches!(
            extraction.cause,
            Some(Cause::SourceUnreadable | Cause::TranscriptInvalid)
        );
        return Some(Gap {
            source: named,
            state: if unreadable {
                GapState::CouldNotLook
            } else {
                GapState::Failed
            },
            run: extraction.run.clone(),
            cause: extraction.cause,
            reason: extraction.reason.clone(),
        });
    }
    if source.coverage.state == CoverageState::CouldNotLook {
        return Some(Gap {
            source: named,
            state: GapState::CouldNotLook,
            run: extraction.run.clone(),
            cause: None,
            reason: source.coverage.reason.clone(),
        });
    }
    if source.hypotheses.is_empty() {
        return Some(Gap {
            source: named,
            state: GapState::FoundNothing,
            run: extraction.run.clone(),
            cause: None,
            reason: None,
        });
    }
    None
}

/// The identity graph's same-person candidates, each account with the names it goes by now.
pub fn questions(accounts: &[AccountFacts], resolution: &Resolution) -> Vec<Question> {
    let names: std::collections::BTreeMap<&str, &[String]> = (accounts.iter())
        .map(|account| (account.id.as_str(), account.names.as_slice()))
        .collect();
    let shown = |account: CandidateAccount| Account {
        names: (names.get(account.id.as_str()))
            .map(|names| names.to_vec())
            .unwrap_or_default(),
        id: account.id,
        identifier: account.identifier,
    };
    (candidate_pairs(accounts, resolution))
        .into_iter()
        .map(|candidate| {
            let [a, b] = candidate.accounts;
            Question {
                accounts: [shown(a), shown(b)],
                reasons: candidate.reasons,
                name: candidate.name,
            }
        })
        .collect()
}

/// Canonical encoding, version 1: a domain tag `D`, then each part as one tag byte, its length
/// in bytes as a big-endian u64, and its UTF-8 bytes. The id is `q_` plus the first 16 bytes of
/// the SHA-256 of that, in hex. Changing any of this is a new version (a new domain tag). This
/// is the encoding a claim's id uses, under the queue's own domain, so an item's id never
/// collides with a claim's.
fn item_id(parts: &[(u8, String)]) -> String {
    let mut hash = Sha256::new();
    for (tag, text) in
        std::iter::once((b'D', "rastro/queue-item/v1".to_string())).chain(parts.iter().cloned())
    {
        hash.update([tag]);
        hash.update((text.len() as u64).to_be_bytes());
        hash.update(text.as_bytes());
    }
    let bytes = hash.finalize();
    let hex: String = bytes[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!("q_{hex}")
}

/// A party as its id encodes it: the mapped entity's id, else the label the source used.
fn party_part(party: &commitments::Party) -> String {
    match (&party.identity, &party.label) {
        (Some(identity), _) => format!("mapped\u{1f}{}", identity.id),
        (None, Some(label)) => format!("unknown\u{1f}{label}"),
        (None, None) => "absent".into(),
    }
}

/// The content address of a proposal: its source and everything the hypothesis says.
fn proposal_id(source: &Source, hypothesis: &Hypothesis) -> String {
    item_id(&[
        (b'K', ItemKind::Proposal.as_str().into()),
        (b'S', source.kind.as_str().into()),
        (b'N', source.name.clone()),
        (b'O', source.scope.clone()),
        (b'H', hypothesis.kind.as_str().into()),
        (b'P', hypothesis.promise.clone()),
        (b'R', party_part(&hypothesis.responsible)),
        (b'C', party_part(&hypothesis.recipient)),
        (
            b'D',
            hypothesis.stated_date.text.clone().unwrap_or_default(),
        ),
        (b'Q', hypothesis.quote.text.clone()),
        (b'T', hypothesis.quote.turn.to_string()),
        (b'U', hypothesis.quote.speaker.clone()),
        (
            b'V',
            format!("{}:{}", hypothesis.quote.start, hypothesis.quote.end),
        ),
        (b'W', hypothesis.terms.join("\u{1f}")),
    ])
}

/// The content address of a question: its two accounts and the signals that produced it.
fn question_id(question: &Question) -> String {
    let reasons: Vec<String> = (question.reasons.iter())
        .map(|reason| {
            let signal = serde_json::to_value(reason.reason)
                .ok()
                .and_then(|value| value.as_str().map(String::from))
                .unwrap_or_default();
            format!(
                "{signal}\u{1f}{}\u{1f}{}",
                reason.values[0], reason.values[1]
            )
        })
        .collect();
    item_id(&[
        (b'K', ItemKind::Question.as_str().into()),
        (b'A', question.accounts[0].id.clone()),
        (b'B', question.accounts[1].id.clone()),
        (b'R', reasons.join("\u{1e}")),
    ])
}

impl SourceKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Meeting => "meeting",
            Self::Identity => "identity",
        }
    }
}
impl ItemKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Proposal => "proposal",
            Self::Question => "question",
        }
    }
}

impl<S: Storage> Rastro<S> {
    /// The queue over every scope, on a writing open.
    pub fn queue(&mut self) -> Result<Queue> {
        self.storage.snapshot(|view| {
            let origins = view.queue_sources(&[])?;
            let resolution = Resolution::of(&view.intents()?);
            let questions = questions(&view.identity_accounts()?, &resolution);
            Ok(Queue::new(origins, questions))
        })
    }
}

impl<S: ReadStorage> Reader<S> {
    /// Every pending proposal and question in the reader's scopes. A source in a scope the
    /// reader can't see is absent, with its gap, and the read still succeeds. Same-person
    /// candidates are computed over every scope, so a reader with launch scopes is shown no
    /// question.
    pub fn queue(&mut self) -> Result<Queue> {
        let Self {
            locations,
            storage,
            scopes,
            ..
        } = self;
        crate::read_only::snapshot(locations, storage, scopes, |view, _| {
            let Some(view) = view else {
                return Ok(Queue::empty());
            };
            let origins = view.queue_sources(scopes)?;
            let questions = if scopes.is_empty() {
                let resolution = Resolution::of(&view.intents()?);
                questions(&view.identity_accounts()?, &resolution)
            } else {
                Vec::new()
            };
            Ok(Queue::new(origins, questions))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// S3: a source that couldn't look is named with its reason, and none of its hypotheses is
    /// shown, even when an earlier run's results survive the failure.
    #[test]
    fn a_source_that_could_not_look_names_its_reason_and_shows_no_item() {
        use crate::commitments::{
            Extraction, Known, Party, PartyState, Quote, Source as CommitmentSource,
            SourceCoverage, SourceKind as CommitmentSourceKind, StatedDate,
        };
        let hypothesis = Hypothesis::new(
            commitments::Kind::Promise,
            Party {
                state: PartyState::Unknown,
                label: Some("Alice".into()),
                identity: None,
            },
            Party {
                state: PartyState::Unknown,
                label: None,
                identity: None,
            },
            "send the budget".into(),
            StatedDate {
                state: Known::Unknown,
                text: None,
            },
            Quote {
                text: "I'll send the budget.".into(),
                turn: 1,
                speaker: "Alice".into(),
                start: 0,
                end: 21,
            },
        );
        // The latest run failed to read the content, but an earlier run's results are kept and
        // still listed by the commitments list.
        let source = CommitmentSource {
            kind: CommitmentSourceKind::Meeting,
            name: "weekly-1".into(),
            scope: "work".into(),
            extraction: Extraction {
                state: RunState::Failed,
                cause: Some(Cause::SourceUnreadable),
                reason: Some("can't read the transcript".into()),
                run: "7".into(),
                finished_at: "2026-11-10T10:00:00Z".parse().unwrap(),
                adapter: None,
                replay_of: None,
                excluded: 0,
            },
            coverage: SourceCoverage {
                state: CoverageState::CouldNotLook,
                turns_read: 0,
                turns_total: 0,
                reason: Some("can't read the transcript".into()),
            },
            stale: true,
            results_from: None,
            hypotheses: vec![hypothesis],
        };
        let queue = Queue::new(
            vec![Origin {
                at: "2026-11-10T10:00:00Z".parse().unwrap(),
                source,
            }],
            Vec::new(),
        );
        assert_eq!(queue.items.len(), 1, "the earlier results stay pending");
        let gap = &queue.gaps[0];
        assert_eq!(gap.state, GapState::CouldNotLook);
        assert_eq!(gap.source.name, "weekly-1");
        assert_eq!(gap.run, "7");
        assert_eq!(gap.reason.as_deref(), Some("can't read the transcript"));
    }
}
