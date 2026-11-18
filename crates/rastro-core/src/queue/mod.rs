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
//!
//! Everything is checked against what is known before it is shown. A proposal whose standard
//! statements one observed commitment already holds is left out and counted ("already known"):
//! the same `std:` predicates with the same literals — a party's name as the source said it or
//! its stated date — carried by the claims of a single observed commitment, and no others. Only
//! an exact statement match drops an item, never a name, a text or a partial match, and claims
//! on several commitments never add up to one. A proposal's mapped parties bring their visible
//! claims as the item's context, and the item stays.

mod answers;
pub use answers::{Answered, answered_items};
mod outcomes;
pub use outcomes::{NOTICE as OUTCOMES_NOTICE, Outcome, OutcomeKind, Outcomes, Totals};

use crate::commitments::{self, Hypothesis};
use crate::identity::{
    AccountFacts, CandidateAccount, CandidateReason, Resolution, candidate_pairs,
};
use crate::knowledge::{AboutOptions, ClaimSummary};
use crate::meeting::{Cause, RunState};
use crate::{CoverageState, Rastro, ReadStorage, Reader, Scope, Snapshot, Storage};
use anyhow::Result;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

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
#[derive(Debug, JsonSchema, Serialize)]
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
    /// What Rastro already knows about the parties the owner mapped: their visible claims, each
    /// once. Context only — an item with claims beside it stays in the queue.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<ClaimSummary>,
    /// Set on a proposal whose statements are exactly those an observed commitment held until the
    /// owner retracted them: new evidence proposes the statement again, and the owner decides
    /// again. Never set on a question.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub earlier_retracted: bool,
    /// The source run the item's content came from, when it came from one: what a shown report
    /// records, so an outcome can date the item's departure. Never printed or published.
    #[serde(skip)]
    #[schemars(skip)]
    pub(crate) run: Option<String>,
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

/// A standard statement a hypothesis expresses: one `std:` predicate with the literal it
/// carries, as the source said it. This is what an observed claim must hold for the item to be
/// already known.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Statement {
    pub predicate: String,
    pub value: String,
}

/// One commitment an observed claim set already holds: the entity its `std:` claims are about,
/// with the statements those claims carry. Claims on several commitments never add up to a
/// match, and a commitment says nothing about a hypothesis unless its statements are the
/// hypothesis's own.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnownCommitment {
    pub subject: String,
    pub statements: BTreeSet<Statement>,
    /// The statements the owner retracted from this commitment, or that a retracted anchor took
    /// with it. They are not known: they only let the queue say a proposal returns what the owner
    /// once retracted.
    pub retracted: BTreeSet<Statement>,
}

/// One item a terminal read printed, when it printed it, and the source run that carried it.
/// Insert-only, outside the claims projection: it is what the owner saw, not what Rastro knows.
#[derive(Clone, Debug)]
pub struct ShownReport {
    pub item: String,
    /// The source's name, as the item's source carried it.
    pub source: String,
    /// The run the item's content came from; empty for a question, which is computed on read.
    pub run: String,
    /// The standard statements the item expressed, kept because an outcome has to tell an item
    /// the queue settled for being already known from one a run dropped, and the item itself is
    /// gone by then. Empty for a question.
    pub statements: Vec<Statement>,
    pub at: DateTime<Utc>,
}

/// One shown report per item a terminal read printed.
pub fn shown(queue: &Queue, at: DateTime<Utc>) -> Vec<ShownReport> {
    (queue.items.iter())
        .map(|item| ShownReport {
            item: item.id.clone(),
            source: item.source.name.clone(),
            run: item.run.clone().unwrap_or_default(),
            statements: (item.proposal.as_ref()).map(statements).unwrap_or_default(),
            at,
        })
        .collect()
}

/// A source of proposals and the time its content belongs to.
#[derive(Clone, Debug)]
pub struct Origin {
    /// A meeting's capture time: not when it was imported.
    pub at: DateTime<Utc>,
    pub source: commitments::Source,
    /// The commitments observed `std:` claims already hold, visible to the caller. Storage reads
    /// them beside the source, so the queue never shows an item it can tell is known; the queue
    /// unions them over every origin.
    pub known: Vec<KnownCommitment>,
}

/// What `queue` returns.
#[derive(Debug, JsonSchema, Serialize)]
pub struct Queue {
    pub notice: String,
    /// Every pending proposal and question, oldest first.
    pub items: Vec<Item>,
    /// What each source that showed no item, or fewer than it should, has to say.
    pub gaps: Vec<Gap>,
    /// How many proposals this read left out because one observed commitment already held
    /// exactly the standard statements they express.
    pub already_known: usize,
}

impl Queue {
    pub fn empty() -> Self {
        Self {
            notice: NOTICE.into(),
            items: Vec::new(),
            gaps: Vec::new(),
            already_known: 0,
        }
    }

    /// Assembles the queue from the sources storage keeps and the identity graph's candidates.
    /// An item whose source is hidden never reaches here: storage filters the sources before
    /// this reads them. Identical items collapse to one, so no candidate is shown twice. A
    /// proposal the owner has answered is left out: the answer is recorded, and the queue holds
    /// only what still waits.
    pub fn new(
        origins: Vec<Origin>,
        questions: Vec<Question>,
        answered: &BTreeSet<String>,
    ) -> Self {
        let known = known_by_subject(origins.iter().flat_map(|origin| origin.known.iter()));
        let retracted = retracted_by_subject(origins.iter().flat_map(|origin| origin.known.iter()));
        let mut aged = Vec::new();
        let mut gaps = Vec::new();
        let mut seen = BTreeSet::new();
        let mut already_known = 0;
        for origin in origins {
            let source = Source::of(&origin.source);
            for hypothesis in &origin.source.hypotheses {
                let mut item = Item {
                    id: proposal_id(&source, hypothesis),
                    kind: ItemKind::Proposal,
                    source: source.clone(),
                    proposal: Some(hypothesis.clone()),
                    question: None,
                    context: Vec::new(),
                    earlier_retracted: false,
                    run: (origin.source.results_from.as_ref()).map(|results| results.run.clone()),
                };
                if answered.contains(&item.id) {
                    continue;
                }
                if !seen.insert(item.id.clone()) {
                    continue;
                }
                if held(&known, &statements(hypothesis)) {
                    already_known += 1;
                    continue;
                }
                item.earlier_retracted = returns(&known, &retracted, &statements(hypothesis));
                aged.push(Aged {
                    at: Some(origin.at),
                    question: false,
                    turn: hypothesis.quote.turn,
                    accounts: String::new(),
                    item,
                });
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
                context: Vec::new(),
                earlier_retracted: false,
                run: None,
            };
            if seen.insert(item.id.clone()) {
                // Computed on read, so a question has no time: it is older than every proposal
                // and sorts before one on that tie, ordered by its accounts' identifiers.
                let accounts = (item.question.as_ref()).map(|question| {
                    format!(
                        "{}\u{1f}{}",
                        question.accounts[0].identifier, question.accounts[1].identifier
                    )
                });
                aged.push(Aged {
                    at: None,
                    question: true,
                    turn: 0,
                    accounts: accounts.unwrap_or_default(),
                    item,
                });
            }
        }
        aged.sort_by(|a, b| a.key().cmp(&b.key()));
        Self {
            notice: NOTICE.into(),
            items: aged.into_iter().map(|aged| aged.item).collect(),
            gaps,
            already_known,
        }
    }

    /// Reads each proposal's mapped parties' visible claims as its context, from the same
    /// snapshot and scopes as the queue itself. A claim both parties bring appears once. Context
    /// never drops an item.
    fn attach_context(&mut self, view: &dyn Snapshot, scopes: &[Scope]) -> Result<()> {
        for item in &mut self.items {
            let Some(proposal) = &item.proposal else {
                continue;
            };
            let mut context: Vec<ClaimSummary> = Vec::new();
            let mut parties = BTreeSet::new();
            for party in [&proposal.responsible, &proposal.recipient] {
                let Some(identity) = &party.identity else {
                    continue;
                };
                if !parties.insert(identity.id.clone()) {
                    continue;
                }
                let claims = view.about_scoped(&identity.id, &AboutOptions::default(), scopes)?;
                for claim in claims {
                    if (context.iter()).all(|seen| seen.claim.id != claim.claim.id) {
                        context.push(claim);
                    }
                }
            }
            item.context = context;
        }
        Ok(())
    }
}

/// The standard statements a hypothesis expresses, in term order: `std:owes` and
/// `std:waiting_on` name the responsible party, `std:owed_to` the recipient and `std:due` the
/// stated date, each as the source said it. A term whose value the source left out states
/// nothing and is skipped. An accepted proposal records these as the claims its intent mints.
pub(super) fn statements(hypothesis: &Hypothesis) -> Vec<Statement> {
    let mut statements = Vec::new();
    for term in &hypothesis.terms {
        let value = match term.as_str() {
            term if term == commitments::OWES || term == commitments::WAITING_ON => {
                hypothesis.responsible.label.as_ref()
            }
            term if term == commitments::OWED_TO => hypothesis.recipient.label.as_ref(),
            term if term == commitments::DUE => hypothesis.stated_date.text.as_ref(),
            _ => None,
        };
        if let Some(value) = value {
            statements.push(Statement {
                predicate: term.clone(),
                value: value.clone(),
            });
        }
    }
    statements
}

/// The statements observed claims hold, keyed by the commitment they are about. A hypothesis or
/// a shown item matches when one commitment holds exactly its statements.
pub(crate) fn known_by_subject<'a>(
    known: impl IntoIterator<Item = &'a KnownCommitment>,
) -> BTreeMap<String, BTreeSet<Statement>> {
    let mut by_subject: BTreeMap<String, BTreeSet<Statement>> = BTreeMap::new();
    for commitment in known {
        by_subject
            .entry(commitment.subject.clone())
            .or_default()
            .extend(commitment.statements.iter().cloned());
    }
    by_subject
}

/// The statements the owner retracted, keyed by the commitment they were about.
fn retracted_by_subject<'a>(
    known: impl IntoIterator<Item = &'a KnownCommitment>,
) -> BTreeMap<String, BTreeSet<Statement>> {
    let mut by_subject: BTreeMap<String, BTreeSet<Statement>> = BTreeMap::new();
    for commitment in known {
        if !commitment.retracted.is_empty() {
            by_subject
                .entry(commitment.subject.clone())
                .or_default()
                .extend(commitment.retracted.iter().cloned());
        }
    }
    by_subject
}

/// Whether one commitment held exactly the statements `expressed` before the owner retracted some
/// of them: what it still holds and what was retracted make up the item's statements. A retracted
/// statement is not known, so this never hides an item; it only marks one.
fn returns(
    known: &BTreeMap<String, BTreeSet<Statement>>,
    retracted: &BTreeMap<String, BTreeSet<Statement>>,
    expressed: &[Statement],
) -> bool {
    let expressed: BTreeSet<Statement> = expressed.iter().cloned().collect();
    !expressed.is_empty()
        && retracted.iter().any(|(subject, gone)| {
            let mut before = known.get(subject).cloned().unwrap_or_default();
            before.extend(gone.iter().cloned());
            before == expressed
        })
}

/// Whether an observed commitment already holds the statements `expressed`: the same set, on one
/// subject. A commitment carrying more statements than the item, or claims spread over several
/// commitments, never adds up to a match; the queue hides an item on an exact statement match
/// only, never on a name or a text match, and an item stating nothing is never already known.
fn held(known: &BTreeMap<String, BTreeSet<Statement>>, expressed: &[Statement]) -> bool {
    let expressed: BTreeSet<Statement> = expressed.iter().cloned().collect();
    !expressed.is_empty() && (known.values()).any(|held| *held == expressed)
}

/// The first moment one observed commitment held exactly the statements `expressed`, from when
/// each of its claims was known: the moment the last of them arrived, unless that commitment
/// already carried a statement the item does not express, in which case its set was never the
/// item's. `None` when no commitment ever held them, and for an item that expresses nothing.
pub(crate) fn held_since(
    history: &[(String, Statement, DateTime<Utc>)],
    expressed: &[Statement],
) -> Option<DateTime<Utc>> {
    let wanted: BTreeSet<&Statement> = expressed.iter().collect();
    if wanted.is_empty() {
        return None;
    }
    let mut by_subject: BTreeMap<&str, BTreeMap<&Statement, DateTime<Utc>>> = BTreeMap::new();
    for (subject, statement, known_at) in history {
        (by_subject.entry(subject).or_default().entry(statement))
            .and_modify(|known| *known = (*known).min(*known_at))
            .or_insert(*known_at);
    }
    let mut held: Option<DateTime<Utc>> = None;
    for known in by_subject.values() {
        let Some(complete) = wanted
            .iter()
            .map(|statement| known.get(statement).copied())
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let at = complete
            .into_iter()
            .max()
            .expect("one moment per statement");
        let extra = (known.iter())
            .any(|(statement, known_at)| *known_at <= at && !wanted.contains(statement));
        if !extra {
            held = Some(held.map_or(at, |held| held.min(at)));
        }
    }
    held
}

/// An item with the key the queue's order sorts by.
struct Aged {
    /// When the item's content belongs to; a question has none.
    at: Option<DateTime<Utc>>,
    /// Questions first, then proposals.
    question: bool,
    /// The quote's turn, which orders one source's proposals.
    turn: usize,
    /// A question's two account identifiers, in order: its content has no time, so this is what
    /// orders it. A proposal leaves it empty and is ordered by its time, then its turn.
    accounts: String,
    item: Item,
}
impl Aged {
    /// Oldest first, questions before proposals; questions by their accounts' identifiers and
    /// proposals by their turn, then by id, so the order is stable across reads.
    fn key(&self) -> (Option<DateTime<Utc>>, bool, usize, &str, &str) {
        (
            self.at,
            self.question,
            self.turn,
            &self.accounts,
            &self.item.id,
        )
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

/// The ids of the proposals a meeting's content carries, one per hypothesis as the queue would
/// show it: the content a successful run proposes. Storage records them beside the run, because
/// only the latest good run keeps its hypotheses, and an outcome still has to name the run that
/// dropped an item.
pub fn proposed_ids(
    meeting: &str,
    scope: &str,
    hypotheses: &[crate::meeting::Hypothesis],
) -> Vec<String> {
    let source = Source {
        kind: SourceKind::Meeting,
        name: meeting.into(),
        scope: scope.into(),
    };
    (hypotheses.iter())
        .map(|hypothesis| proposal_id(&source, &Hypothesis::from(hypothesis)))
        .collect()
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

/// The queue as `scopes` sees it (empty: every scope), inside a snapshot the caller holds.
pub(crate) fn read(
    view: &dyn crate::read_only::Snapshot,
    scopes: &[crate::Scope],
) -> Result<Queue> {
    let intents = view.intents()?;
    let answered = answered_items(&intents);
    let origins = view.queue_sources(scopes)?;
    // Same-person candidates are computed over every scope, so only an unscoped caller is
    // shown a question.
    let questions = if scopes.is_empty() {
        let resolution = Resolution::of(&intents);
        questions(&view.identity_accounts()?, &resolution)
    } else {
        Vec::new()
    };
    let mut queue = Queue::new(origins, questions, &answered);
    queue.attach_context(view, scopes)?;
    Ok(queue)
}

impl<S: Storage> Rastro<S> {
    /// The queue over every scope, on a writing open.
    pub fn queue(&mut self) -> Result<Queue> {
        self.storage.snapshot(|view| read(view, &[]))
    }

    /// Records one shown report per item a terminal read printed, at the time it printed them.
    /// Only the terminal read calls it: a machine read records nothing.
    pub fn queue_shown(&mut self, queue: &Queue) -> Result<usize> {
        let at = crate::coverage::now()?;
        self.storage.record_shown_reports(&shown(queue, at))
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
            read(view, scopes)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// S3: a source that couldn't look is named with its reason, and the last good run's
    /// proposals stay listed beside the gap, never silently dropped.
    #[test]
    fn a_source_that_could_not_look_names_its_reason_and_keeps_the_last_good_items() {
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
                known: Vec::new(),
            }],
            Vec::new(),
            &BTreeSet::new(),
        );
        assert_eq!(queue.items.len(), 1, "the earlier results stay pending");
        let gap = &queue.gaps[0];
        assert_eq!(gap.state, GapState::CouldNotLook);
        assert_eq!(gap.source.name, "weekly-1");
        assert_eq!(gap.run, "7");
        assert_eq!(gap.reason.as_deref(), Some("can't read the transcript"));
    }

    /// A meeting origin holding `hypotheses`, beside the commitments already known.
    fn origin(hypotheses: Vec<Hypothesis>, known: Vec<KnownCommitment>) -> Origin {
        use crate::commitments::{
            Extraction, Source as CommitmentSource, SourceCoverage,
            SourceKind as CommitmentSourceKind,
        };
        Origin {
            at: "2026-11-10T10:00:00Z".parse().unwrap(),
            source: CommitmentSource {
                kind: CommitmentSourceKind::Meeting,
                name: "weekly-1".into(),
                scope: "work".into(),
                extraction: Extraction {
                    state: RunState::Succeeded,
                    cause: None,
                    reason: None,
                    run: "1".into(),
                    finished_at: "2026-11-10T10:00:00Z".parse().unwrap(),
                    adapter: None,
                    replay_of: None,
                    excluded: 0,
                },
                coverage: SourceCoverage {
                    state: CoverageState::Found,
                    turns_read: 2,
                    turns_total: 2,
                    reason: None,
                },
                stale: false,
                results_from: None,
                hypotheses,
            },
            known,
        }
    }

    /// A hypothesis naming `responsible` and, as they are given, a recipient and a stated date.
    fn hypothesis(
        kind: commitments::Kind,
        responsible: &str,
        recipient: Option<&str>,
        date: Option<&str>,
        promise: &str,
    ) -> Hypothesis {
        use crate::commitments::{Known, Party, PartyState, Quote, StatedDate};
        let party = |label: Option<&str>| Party {
            state: PartyState::Unknown,
            label: label.map(String::from),
            identity: None,
        };
        Hypothesis::new(
            kind,
            party(Some(responsible)),
            party(recipient),
            promise.into(),
            StatedDate {
                state: if date.is_some() {
                    Known::Stated
                } else {
                    Known::Unknown
                },
                text: date.map(String::from),
            },
            Quote {
                text: format!("I'll {promise}."),
                turn: 1,
                speaker: responsible.into(),
                start: 0,
                end: promise.len() + 5,
            },
        )
    }

    /// A commitment an observed claim set already holds, from `(predicate, value)` statements.
    fn commitment(subject: &str, statements: &[(&str, &str)]) -> KnownCommitment {
        KnownCommitment {
            subject: subject.into(),
            statements: statement_set(statements),
            retracted: BTreeSet::new(),
        }
    }

    fn statement_set(statements: &[(&str, &str)]) -> BTreeSet<Statement> {
        (statements.iter())
            .map(|(predicate, value)| Statement {
                predicate: (*predicate).into(),
                value: (*value).into(),
            })
            .collect()
    }

    /// A commitment hides a proposal only when it holds that proposal's own statements, as one
    /// set: a commitment carrying more statements than the proposal says nothing about it.
    #[test]
    fn a_commitment_hides_only_the_proposal_with_its_own_statements() {
        let accepted = commitment(
            "std:commitment/1",
            &[
                (commitments::OWES, "Alice"),
                (commitments::OWED_TO, "Bob"),
                (commitments::DUE, "by Friday"),
            ],
        );
        // The same statements as the accepted commitment: known, and left out.
        let same = hypothesis(
            commitments::Kind::Promise,
            "Alice",
            Some("Bob"),
            Some("by Friday"),
            "send Bob the budget",
        );
        // The accepted commitment's parties and date but no recipient of its own: its statements
        // are only a part of that commitment's, so it stays, even though every one of them is
        // held by the accepted commitment and by nothing else.
        let narrower = hypothesis(
            commitments::Kind::Promise,
            "Alice",
            None,
            Some("by Friday"),
            "keep the budget",
        );
        let queue = Queue::new(
            vec![origin(vec![same, narrower], vec![accepted])],
            Vec::new(),
            &BTreeSet::new(),
        );
        let shown: Vec<&str> = (queue.items.iter())
            .filter_map(|item| item.proposal.as_ref().map(|p| p.promise.as_str()))
            .collect();
        assert_eq!(shown, ["keep the budget"]);
        assert_eq!(queue.already_known, 1);
    }
    /// A retracted statement is not known: the proposal that expresses what one commitment held
    /// before a retraction stays, marked, whether the whole commitment or only part of it was
    /// retracted; a proposal that expresses something else is not marked.
    #[test]
    fn a_retracted_commitment_no_longer_hides_a_proposal_but_marks_it() {
        let terms = [
            (commitments::OWES, "Alice"),
            (commitments::OWED_TO, "Bob"),
            (commitments::DUE, "by Friday"),
        ];
        let proposal = |date: &str| {
            hypothesis(
                commitments::Kind::Promise,
                "Alice",
                Some("Bob"),
                Some(date),
                "send Bob the budget",
            )
        };
        let whole = KnownCommitment {
            subject: "std:commitment/1".into(),
            statements: BTreeSet::new(),
            retracted: statement_set(&terms),
        };
        let partial = KnownCommitment {
            subject: "std:commitment/2".into(),
            statements: statement_set(&terms[..2]),
            retracted: statement_set(&terms[2..]),
        };
        for (known, date, marked) in [
            (whole.clone(), "by Friday", true),
            (partial, "by Friday", true),
            (whole, "by Monday", false),
        ] {
            let queue = Queue::new(
                vec![origin(vec![proposal(date)], vec![known])],
                Vec::new(),
                &BTreeSet::new(),
            );
            assert_eq!(queue.already_known, 0);
            assert_eq!(queue.items.len(), 1);
            assert_eq!(queue.items[0].earlier_retracted, marked, "{date}");
        }
    }
}
