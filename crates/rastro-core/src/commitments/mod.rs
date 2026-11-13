//! The commitments list: every source's commitment hypotheses in one source-neutral shape.
//!
//! A hypothesis is what an agent tool adapter proposed from a conversation and the core checked
//! (ADR 0011, ADR 0012). It is not a claim and not a commitment Rastro knows: the list never says
//! one is open, done or tracked. A source is listed even when it has nothing to show, so that a
//! source that found nothing, failed, or could not be looked at is never left out silently.
//!
//! The list reads through [`Snapshot::commitment_sources`]: storage turns what it keeps for each
//! kind of source into [`Source`] entries, and the core filters them. A scope that hides a source
//! hides its entry, and the read still succeeds.
use crate::{
    CoverageState, Rastro, ReadStorage, Reader, Storage,
    meeting::{Cause, RunState},
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What every list says first: these are hypotheses, and nothing here is a verdict.
pub const NOTICE: &str = "Hypotheses an adapter proposed from a conversation. They are not commitments Rastro knows: it says nothing about whether they are open, done or tracked.";

/// The standard term of a hypothesis that has a responsible party and is not a waiting-on.
pub const OWES: &str = "std:owes";
/// The standard term of a hypothesis that has a recipient.
pub const OWED_TO: &str = "std:owed_to";
/// The standard term of a hypothesis whose quote states a date.
pub const DUE: &str = "std:due";
/// The standard term of a hypothesis that is a waiting-on.
pub const WAITING_ON: &str = "std:waiting_on";

/// What a hypothesis is.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Someone asks another to do something.
    Ask,
    /// Someone says they will do something.
    Promise,
    /// Someone is waiting on another's action.
    WaitingOn,
}
impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Promise => "promise",
            Self::WaitingOn => "waiting_on",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        [Self::Ask, Self::Promise, Self::WaitingOn]
            .into_iter()
            .find(|kind| kind.as_str() == text)
    }
}
impl From<rastro_plugin_sdk::CandidateKind> for Kind {
    fn from(kind: rastro_plugin_sdk::CandidateKind) -> Self {
        match kind {
            rastro_plugin_sdk::CandidateKind::Ask => Self::Ask,
            rastro_plugin_sdk::CandidateKind::Promise => Self::Promise,
            rastro_plugin_sdk::CandidateKind::WaitingOn => Self::WaitingOn,
        }
    }
}

/// An entity a speaker was explicitly mapped to.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Identity {
    pub id: String,
    pub identifier: String,
}
/// Whether a party is a known identity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PartyState {
    /// The owner explicitly mapped the label to an existing entity.
    Mapped,
    /// No explicit mapping: who this is stays unknown.
    Unknown,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Known {
    Stated,
    Unknown,
}
/// A date exactly as the speaker said it ("next Friday"), or unknown. Never resolved.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct StatedDate {
    pub state: Known,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}
/// The exact text a hypothesis rests on, and where it sits.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Quote {
    pub text: String,
    /// The turn it is in, counted from 1.
    pub turn: usize,
    /// The label of that turn's speaker.
    pub speaker: String,
    /// Byte range of the quote within the turn's text.
    pub start: usize,
    pub end: usize,
}

/// A person in a hypothesis. `identity` is there only when the owner mapped the label explicitly,
/// and is null otherwise: a party that is only named carries its label and no identity.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "CommitmentParty")]
pub struct Party {
    pub state: PartyState,
    /// The label or name the source uses, when there is one; never an identity.
    pub label: Option<String>,
    pub identity: Option<Identity>,
}
impl Party {
    /// Whether the hypothesis names this side at all.
    fn present(&self) -> bool {
        self.label.is_some() || self.identity.is_some()
    }
}
impl From<&crate::meeting::Party> for Party {
    fn from(party: &crate::meeting::Party) -> Self {
        Self {
            state: party.state,
            label: party.label.clone(),
            identity: party.identity.clone(),
        }
    }
}

/// One proposed commitment, whatever source it came from.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "CommitmentHypothesis")]
pub struct Hypothesis {
    /// Always `hypothesis`.
    pub status: String,
    pub kind: Kind,
    pub responsible: Party,
    pub recipient: Party,
    pub promise: String,
    pub stated_date: StatedDate,
    pub quote: Quote,
    /// The standard vocabulary terms it expresses: `std:owes` when it has a responsible party
    /// (and is not a waiting-on), `std:owed_to` when it has a recipient, `std:due` when it states
    /// a date, and `std:waiting_on` when it is a waiting-on.
    pub terms: Vec<String>,
}
impl Hypothesis {
    pub fn new(
        kind: Kind,
        responsible: Party,
        recipient: Party,
        promise: String,
        stated_date: StatedDate,
        quote: Quote,
    ) -> Self {
        let mut terms = Vec::new();
        if kind != Kind::WaitingOn && responsible.present() {
            terms.push(OWES.to_string());
        }
        if recipient.present() {
            terms.push(OWED_TO.to_string());
        }
        if stated_date.state == Known::Stated {
            terms.push(DUE.to_string());
        }
        if kind == Kind::WaitingOn {
            terms.push(WAITING_ON.to_string());
        }
        Self {
            status: "hypothesis".into(),
            kind,
            responsible,
            recipient,
            promise,
            stated_date,
            quote,
            terms,
        }
    }
}
impl From<&crate::meeting::Hypothesis> for Hypothesis {
    fn from(hypothesis: &crate::meeting::Hypothesis) -> Self {
        Self::new(
            hypothesis.kind,
            (&hypothesis.responsible).into(),
            (&hypothesis.recipient).into(),
            hypothesis.promise.clone(),
            hypothesis.stated_date.clone(),
            hypothesis.quote.clone(),
        )
    }
}

/// Whether a current type value is a bot: the `Bot` of a namespace, such as `code:Bot` or
/// `tracker:Bot`. Evidence that says so excludes a candidate whose party is mapped to that
/// account; a party that is only named is never excluded.
pub fn is_bot_type(value: &str) -> bool {
    value
        .split_once(':')
        .is_some_and(|(namespace, name)| !namespace.is_empty() && name == "Bot")
}

/// What kind of source a conversation comes from.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Meeting,
}

/// How the source's latest extraction went.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "SourceExtraction")]
pub struct Extraction {
    pub state: RunState,
    /// Why the run failed, when it did.
    pub cause: Option<Cause>,
    pub reason: Option<String>,
    /// The latest run.
    pub run: String,
    pub finished_at: DateTime<Utc>,
    /// The plugin id of the adapter that judged the content.
    pub adapter: Option<String>,
    /// The run whose recorded output this run checked again, with no model call.
    pub replay_of: Option<String>,
    /// Candidates the core left out because the content did not support the owner's part.
    pub excluded: usize,
}
/// What the latest run looked at: the turns read, never whether everything was found.
#[derive(Clone, Debug, JsonSchema, Serialize)]
pub struct SourceCoverage {
    /// `found` when the latest run looked (whether or not it found anything) and `couldn't look`
    /// when it failed.
    pub state: CoverageState,
    pub turns_read: usize,
    pub turns_total: usize,
    pub reason: Option<String>,
}
/// The run the listed hypotheses come from.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "SourceResultsFrom")]
pub struct ResultsFrom {
    /// The last good run: the one whose output the hypotheses are.
    pub run: String,
    /// When the import those results trace back to finished.
    pub imported_at: DateTime<Utc>,
    /// When a replay last checked them again, with no model call.
    pub replayed_at: Option<DateTime<Utc>>,
}

/// One source and what it proposed.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "CommitmentSource")]
pub struct Source {
    pub kind: SourceKind,
    pub name: String,
    pub scope: String,
    pub extraction: Extraction,
    pub coverage: SourceCoverage,
    /// True when the latest run failed, or an earlier import's record is all there is: the
    /// hypotheses say what an earlier look found, not what the content now says.
    pub stale: bool,
    /// Where the hypotheses come from; null when no run has succeeded.
    pub results_from: Option<ResultsFrom>,
    pub hypotheses: Vec<Hypothesis>,
}

/// What `commitments` returns.
#[derive(Clone, Debug, JsonSchema, Serialize)]
pub struct Commitments {
    pub notice: String,
    pub sources: Vec<Source>,
}

impl Commitments {
    /// Narrows the sources: `source` keeps the one source of that name; `term` keeps the
    /// hypotheses expressing that term. A source whose last run looked and has no hypothesis
    /// expressing the term is dropped; a source whose run failed stays, with the matching
    /// hypotheses of its last good run, because its gap is the answer. A filter that matches
    /// nothing leaves an empty list.
    pub fn of(mut sources: Vec<Source>, source: Option<&str>, term: Option<&str>) -> Self {
        if let Some(name) = source {
            sources.retain(|s| s.name == name);
        }
        if let Some(term) = term {
            for s in &mut sources {
                s.hypotheses
                    .retain(|h| h.terms.iter().any(|expressed| expressed == term));
            }
            sources.retain(|s| !s.hypotheses.is_empty() || s.extraction.state == RunState::Failed);
        }
        Self {
            notice: NOTICE.into(),
            sources,
        }
    }
}

impl crate::meeting::StoredMeeting {
    /// The meeting as a source of the commitments list.
    pub fn source(&self) -> Source {
        let latest = &self.latest;
        let coverage = match latest.state {
            RunState::Failed => SourceCoverage {
                state: CoverageState::CouldNotLook,
                turns_read: 0,
                turns_total: latest.turns_total,
                reason: latest.reason.clone(),
            },
            RunState::Succeeded => SourceCoverage {
                state: CoverageState::Found,
                turns_read: latest.turns_read,
                turns_total: latest.turns_total,
                reason: None,
            },
        };
        Source {
            kind: SourceKind::Meeting,
            name: self.meeting.id.clone(),
            scope: self.meeting.scope.clone(),
            extraction: Extraction {
                state: latest.state,
                cause: latest.cause,
                reason: latest.reason.clone(),
                run: latest.id.to_string(),
                finished_at: latest.finished_at,
                adapter: latest.extractor.clone(),
                replay_of: latest.replay_of.map(|run| run.to_string()),
                excluded: latest.excluded,
            },
            coverage,
            stale: self.stale,
            results_from: self.good.as_ref().map(|good| ResultsFrom {
                run: good.id.to_string(),
                imported_at: self.origin.as_ref().unwrap_or(good).finished_at,
                replayed_at: good.replay_of.map(|_| good.finished_at),
            }),
            hypotheses: self.hypotheses.iter().map(Hypothesis::from).collect(),
        }
    }
}

impl<S: Storage> Rastro<S> {
    /// The same read as [`Reader::commitments`], on a writing open, over every scope.
    pub fn commitments(&mut self, source: Option<&str>, term: Option<&str>) -> Result<Commitments> {
        let sources = self.storage.snapshot(|view| view.commitment_sources(&[]))?;
        Ok(Commitments::of(sources, source, term))
    }
}

impl<S: ReadStorage> Reader<S> {
    /// Every source in the reader's scopes with what it proposed, narrowed by the filters. A source
    /// in a scope the reader can't see is absent, and the read still succeeds.
    pub fn commitments(&mut self, source: Option<&str>, term: Option<&str>) -> Result<Commitments> {
        let Self {
            locations,
            storage,
            scopes,
            ..
        } = self;
        let sources =
            crate::read_only::snapshot(locations, storage, scopes, |view, _| match view {
                Some(view) => view.commitment_sources(scopes),
                None => Ok(Vec::new()),
            })?;
        Ok(Commitments::of(sources, source, term))
    }
}
