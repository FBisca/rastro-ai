//! Outcomes: what happened to each item the owner answered, and each item the owner was shown
//! and never answered.
//!
//! An outcome is derived, never recorded in a table of its own: the owner intent is the durable
//! record, and the terminal read records what it printed. A proposal or question the owner
//! accepted, accepted with edits, rejected or answered carries that intent's kind and time, and
//! the claim the intent backs when there is one. An item with no answering intent that the queue
//! no longer holds was ignored: it was shown, never answered, and left. Its outcome names the
//! time it waited, from its first shown report to the run that replaced its source's content.
use super::{ShownReport, read};
use crate::identity::{Intent, IntentKind};
use crate::read_only::Snapshot;
use crate::{Rastro, ReadStorage, Reader, Scope, Storage};
use anyhow::Result;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// What these outcomes are, and what they are not: they are what the owner did and saw, not
/// what Rastro decided about the person or the commitment.
pub const NOTICE: &str = "What happened to each proposal and question: what you answered, and what a terminal queue read showed you and you never answered. An outcome counts only where you saw the item: automation reading the queue records nothing.";

/// What happened to one item.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "QueueOutcomeKind")]
pub enum OutcomeKind {
    /// Accepted unchanged.
    Accepted,
    /// Accepted with the owner's edits, which is not a clean accept.
    AcceptedWithEdits,
    /// Rejected, with the owner's reason recorded by the intent.
    Rejected,
    /// A question answered yes or no.
    Answered,
    /// Shown, never answered, and gone from the queue.
    Ignored,
}

/// What happened to one item.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "QueueOutcome")]
pub struct Outcome {
    /// The item's id as the queue read gave it.
    pub item: String,
    pub kind: OutcomeKind,
    /// When it happened: the intent's time, or, for an ignored item, when it left the queue.
    pub at: DateTime<Utc>,
    /// The claim an accepted item became; absent when no claim holds it. One accept mints one
    /// claim per standard term of the item, so an outcome names the first of them in id order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim: Option<String>,
    /// How long an ignored item waited from its first shown report to its departure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waited_seconds: Option<i64>,
}

/// What was shown and what was acted on, each counted once per item.
#[derive(Clone, Copy, Debug, JsonSchema, Serialize)]
#[schemars(rename = "QueueOutcomeTotals")]
pub struct Totals {
    /// How many distinct items a terminal queue read showed.
    pub shown: usize,
    /// How many distinct items the owner answered (accepted, edited, rejected or answered).
    pub acted_on: usize,
}

/// What `queue outcomes` returns.
#[derive(Debug, JsonSchema, Serialize)]
pub struct Outcomes {
    pub notice: String,
    /// Every outcome, newest first.
    pub outcomes: Vec<Outcome>,
    pub totals: Totals,
}

impl Outcomes {
    pub fn empty() -> Self {
        Self {
            notice: NOTICE.into(),
            outcomes: Vec::new(),
            totals: Totals {
                shown: 0,
                acted_on: 0,
            },
        }
    }
}

/// The outcome an answer records, if the intent answers an item.
fn answered(intent: &Intent) -> Option<OutcomeKind> {
    match intent.kind {
        IntentKind::Accept => Some(OutcomeKind::Accepted),
        IntentKind::Edit => Some(OutcomeKind::AcceptedWithEdits),
        IntentKind::Reject => Some(OutcomeKind::Rejected),
        IntentKind::Answer => Some(OutcomeKind::Answered),
        _ => None,
    }
}

/// Derives every outcome from the intents, the shown reports and what still waits.
pub(crate) fn outcomes(view: &dyn Snapshot, scopes: &[Scope]) -> Result<Outcomes> {
    let intents = view.intents()?;
    let shown = view.shown_reports()?;
    let pending: BTreeSet<String> = (read(view, scopes)?.items)
        .into_iter()
        .map(|item| item.id)
        .collect();
    let claims = view.intent_claims()?;
    let now = crate::coverage::now()?;
    let mut outcomes = Vec::new();
    let mut answered_items = BTreeSet::new();
    for intent in &intents {
        let (Some(kind), Some(item)) = (answered(intent), intent.queue_item()) else {
            continue;
        };
        answered_items.insert(item.to_string());
        // Only an accept or an edit mints an accepted claim. One accept mints a claim per
        // standard term of the item, and the outcome is one row, so it names the first of them
        // in id order: the same one before and after a rebuild, since the set is the same.
        let claim = matches!(kind, OutcomeKind::Accepted | OutcomeKind::AcceptedWithEdits)
            .then(|| (claims.get(intent.id.as_str())).cloned())
            .flatten();
        outcomes.push(Outcome {
            item: item.into(),
            kind,
            at: intent.issued_at,
            claim,
            waited_seconds: None,
        });
    }
    for (item, reports) in by_item(&shown) {
        if answered_items.contains(&item) || pending.contains(&item) {
            continue;
        }
        let first = reports.first().expect("by_item groups at least one report");
        let last = reports.last().expect("by_item groups at least one report");
        let left = if last.run.is_empty() {
            now
        } else {
            (view.successor_run(&last.source, &last.run)?).unwrap_or(now)
        };
        outcomes.push(Outcome {
            item: item.clone(),
            kind: OutcomeKind::Ignored,
            at: left,
            claim: None,
            waited_seconds: Some((left - first.at).num_seconds()),
        });
    }
    // Newest first; two outcomes of one instant keep a stable order by item.
    outcomes.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.item.cmp(&b.item)));
    let shown = by_item(&shown).len();
    let acted_on = (outcomes.iter())
        .filter(|outcome| outcome.kind != OutcomeKind::Ignored)
        .map(|outcome| outcome.item.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    Ok(Outcomes {
        notice: NOTICE.into(),
        outcomes,
        totals: Totals { shown, acted_on },
    })
}

/// The shown reports grouped by item, each item's oldest report first.
fn by_item(shown: &[ShownReport]) -> BTreeMap<String, Vec<&ShownReport>> {
    let mut grouped: BTreeMap<String, Vec<&ShownReport>> = BTreeMap::new();
    for report in shown {
        grouped.entry(report.item.clone()).or_default().push(report);
    }
    for reports in grouped.values_mut() {
        reports.sort_by_key(|report| report.at);
    }
    grouped
}

impl<S: Storage> Rastro<S> {
    /// Every outcome over every scope, on a writing open.
    pub fn outcomes(&mut self) -> Result<Outcomes> {
        self.storage.snapshot(|view| outcomes(view, &[]))
    }
}

impl<S: ReadStorage> Reader<S> {
    /// Every outcome the reader can see, newest first.
    pub fn outcomes(&mut self) -> Result<Outcomes> {
        let Self {
            locations,
            storage,
            scopes,
            ..
        } = self;
        crate::read_only::snapshot(locations, storage, scopes, |view, _| {
            let Some(view) = view else {
                return Ok(Outcomes::empty());
            };
            outcomes(view, scopes)
        })
    }
}
