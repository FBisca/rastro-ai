//! The focus report: where the owner should put their attention now, from what Rastro already
//! reads (Linear tasks, meeting commitments and the queue), ranked without a model.
//!
//! [`report`] is a pure function of its [`Input`]: the same evidence gives the same report, in
//! whatever order the sources listed it. Nothing here reads a clock, a database or a source.
//!
//! # Ranking
//! Candidates fall in bands, in this order (decisions D10):
//! 1. overdue or due today (the due date's day in the owner's local offset),
//! 2. a meeting promise or ask the owner is responsible for, so it waits on the owner,
//! 3. the next action of a Linear task in progress.
//!
//! Inside a band the earlier due time ranks first, then the higher urgency, then the item id.
//! The best candidate is the recommended focus; the next [`MAX_NEEDS_YOU`] are the needs-you items.
//!
//! # Truth about what was read
//! Every item cites its source, location and the time that source was last read well. A source
//! whose last read failed shows a [`Gap`] with a typed reason, and its items keep the last good
//! read's time. A source that read fine and returned nothing shows no gap.
//!
//! The queue appears as a count and nothing else: no proposal text reaches the report.
use crate::{
    commitments::{self, Hypothesis, Kind},
    meeting::{Cause, RunState},
    queue::Queue,
};
use chrono::{DateTime, FixedOffset, Offset, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::cmp::{Ordering, Reverse};

/// How many needs-you items follow the recommended focus.
pub const MAX_NEEDS_YOU: usize = 3;
/// What an empty report says.
pub const NOTHING_NEEDS_YOU: &str = "Nothing needs you.";

/// How pressing a task is, as its tracker says. Higher ranks first within a band.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "FocusUrgency")]
pub enum Urgency {
    None,
    Low,
    Medium,
    High,
    Urgent,
}

/// Where a Linear task stands.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "FocusTaskState")]
pub enum TaskState {
    /// Started: its next action is a candidate.
    InProgress,
    /// Not started: a candidate only once it is due.
    Open,
    /// Done or canceled: never a candidate.
    Closed,
}

/// A Linear task as the focus reads it.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "FocusTask")]
pub struct Task {
    /// Where the task lives, e.g. `linear:issue/<uuid>`; also its stable id.
    pub location: String,
    /// The tracker's short name for it, e.g. `H-8102`.
    pub identifier: String,
    pub title: String,
    pub url: Option<String>,
    pub state: TaskState,
    pub due: Option<DateTime<Utc>>,
    pub urgency: Urgency,
}

/// Why a source could not be looked at.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[schemars(rename = "FocusGapReason")]
pub enum GapReason {
    /// The source's last sync failed.
    SyncFailed,
    /// The source's latest extraction failed, for this cause when it is known.
    ExtractionFailed { cause: Option<Cause> },
}

/// How a source's latest read went.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[schemars(rename = "FocusRead")]
pub enum Read {
    /// The latest read worked, at this time.
    Good { at: DateTime<Utc> },
    /// The latest read failed; the last good one, if any, was at `last_good`.
    Failed {
        reason: GapReason,
        last_good: Option<DateTime<Utc>>,
    },
}

/// The Linear source's latest read and the tasks it holds (the last good read's, when it failed).
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "FocusLinearRead")]
pub struct LinearRead {
    pub read: Read,
    pub tasks: Vec<Task>,
}

/// Everything one focus report is computed from.
pub struct Input<'a> {
    pub now: DateTime<Utc>,
    /// The owner's local offset: what "today" means.
    pub offset: FixedOffset,
    /// `None` when no Linear source is configured: no items and no gap.
    pub linear: Option<LinearRead>,
    /// The owner's entity id: a meeting hypothesis waits on the owner only when this identity is
    /// its responsible party.
    pub owner_id: &'a str,
    pub meetings: &'a [commitments::Source],
    pub queue: &'a Queue,
}

/// The sources focus reads.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "FocusSourceKind")]
pub enum SourceKind {
    Linear,
    Meeting,
}

/// Why an item is on the report.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "FocusReason")]
pub enum Reason {
    Overdue,
    DueToday,
    WaitingOnOwner,
    InProgress,
}

/// Where an item comes from and when that source was last read well.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "FocusCitation")]
pub struct Citation {
    pub source: SourceKind,
    /// The source's name: `linear`, or the meeting's.
    pub name: String,
    pub location: String,
    pub url: Option<String>,
    pub observed_at: DateTime<Utc>,
}

/// One thing that needs the owner.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "FocusItem")]
pub struct Item {
    pub id: String,
    pub title: String,
    pub reason: Reason,
    pub due: Option<DateTime<Utc>>,
    pub urgency: Urgency,
    pub citation: Citation,
}

/// A source that could not look.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "FocusGap")]
pub struct Gap {
    pub source: SourceKind,
    pub name: String,
    pub reason: GapReason,
    /// The last good read, which stamps this source's items; null when there never was one.
    pub as_of: Option<DateTime<Utc>>,
}

/// The focus report.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "FocusReport")]
pub struct Report {
    /// The oldest read time any source's items rest on; `now` when no source was read.
    pub as_of: DateTime<Utc>,
    /// The one recommended focus; null when nothing needs the owner.
    pub focus: Option<Item>,
    /// Up to three more, in rank order.
    pub needs_you: Vec<Item>,
    /// How many proposals and questions wait in the queue; never their content.
    pub queue_count: usize,
    pub gaps: Vec<Gap>,
    /// Says nothing needs the owner, when that is so.
    pub notice: Option<String>,
}

/// Builds the report. Never fails: no candidate is an empty report.
pub fn report(input: &Input) -> Report {
    let mut items = Vec::new();
    let mut gaps = Vec::new();
    let mut read_times = Vec::new();
    if let Some(linear) = &input.linear {
        let observed = match linear.read {
            Read::Good { at } => Some(at),
            Read::Failed { reason, last_good } => {
                gaps.push(Gap {
                    source: SourceKind::Linear,
                    name: LINEAR.into(),
                    reason,
                    as_of: last_good,
                });
                last_good
            }
        };
        read_times.extend(observed);
        if let Some(observed) = observed {
            items.extend((linear.tasks.iter()).filter_map(|task| task_item(input, task, observed)));
        }
    }
    for source in input.meetings {
        let results = (source.results_from.as_ref()).map(|from| from.imported_at);
        let observed = match source.extraction.state {
            RunState::Succeeded => Some(source.extraction.finished_at),
            RunState::Failed => {
                gaps.push(Gap {
                    source: SourceKind::Meeting,
                    name: source.name.clone(),
                    reason: GapReason::ExtractionFailed {
                        cause: source.extraction.cause,
                    },
                    as_of: results,
                });
                results
            }
        };
        read_times.extend(observed);
        if let Some(observed) = observed {
            items.extend(
                (source.hypotheses.iter())
                    .filter(|hypothesis| owner_owes(hypothesis, input.owner_id))
                    .map(|hypothesis| promise_item(source, hypothesis, observed)),
            );
        }
    }
    items.sort_by(rank);
    let mut ranked = items.into_iter();
    let focus = ranked.next();
    gaps.sort_by(|a, b| (a.source, &a.name).cmp(&(b.source, &b.name)));
    Report {
        as_of: read_times.into_iter().min().unwrap_or(input.now),
        notice: focus.is_none().then(|| NOTHING_NEEDS_YOU.to_string()),
        focus,
        needs_you: ranked.take(MAX_NEEDS_YOU).collect(),
        queue_count: input.queue.items.len(),
        gaps,
    }
}

const LINEAR: &str = "linear";

/// A task as a candidate: overdue or due today in any open state, otherwise only when started.
fn task_item(input: &Input, task: &Task, observed_at: DateTime<Utc>) -> Option<Item> {
    let today = input.now.with_timezone(&input.offset).date_naive();
    let due_day = |due: DateTime<Utc>| due.with_timezone(&input.offset).date_naive();
    let reason = match (task.state, task.due) {
        (TaskState::Closed, _) => return None,
        (_, Some(due)) if due_day(due) < today => Reason::Overdue,
        (_, Some(due)) if due_day(due) == today => Reason::DueToday,
        (TaskState::InProgress, _) => Reason::InProgress,
        _ => return None,
    };
    Some(Item {
        id: task.location.clone(),
        title: task.title.clone(),
        reason,
        due: task.due,
        urgency: task.urgency,
        citation: Citation {
            source: SourceKind::Linear,
            name: LINEAR.into(),
            location: task.location.clone(),
            url: task.url.clone(),
            observed_at,
        },
    })
}

/// Whether a hypothesis is the owner's to do: an ask or a promise whose responsible party, the one
/// who acts, is the owner. What another owes the owner, or the owner is waiting on, is not: a
/// `waiting_on` is the owner waiting for someone else's action (`std:waiting_on`).
fn owner_owes(hypothesis: &Hypothesis, owner_id: &str) -> bool {
    hypothesis.kind != Kind::WaitingOn
        && (hypothesis.responsible.identity.as_ref()).is_some_and(|party| party.id == owner_id)
}

/// A meeting ask or promise as a candidate. Its stated date stays the speaker's words, so it has
/// no due time to rank by.
fn promise_item(
    source: &commitments::Source,
    hypothesis: &Hypothesis,
    observed_at: DateTime<Utc>,
) -> Item {
    let location = format!("{}#turn-{}", source.name, hypothesis.quote.turn);
    Item {
        id: format!(
            "{location}:{}-{}",
            hypothesis.quote.start, hypothesis.quote.end
        ),
        title: hypothesis.promise.clone(),
        reason: Reason::WaitingOnOwner,
        due: None,
        urgency: Urgency::None,
        citation: Citation {
            source: SourceKind::Meeting,
            name: source.name.clone(),
            location,
            url: None,
            observed_at,
        },
    }
}

fn band(reason: Reason) -> u8 {
    match reason {
        Reason::Overdue | Reason::DueToday => 0,
        Reason::WaitingOnOwner => 1,
        Reason::InProgress => 2,
    }
}

/// Band, then the earlier due time (none last), then the higher urgency, then id order; the title
/// settles two items that share an id.
fn rank(a: &Item, b: &Item) -> Ordering {
    let due = |item: &Item| (item.due.is_none(), item.due);
    (band(a.reason), due(a), Reverse(a.urgency), &a.id, &a.title).cmp(&(
        band(b.reason),
        due(b),
        Reverse(b.urgency),
        &b.id,
        &b.title,
    ))
}

/// The focus report for what a snapshot holds, judged at the coverage clock and the owner's local
/// offset. Linear issues carry no due date or urgency in the evidence, so they rank by state alone.
fn read_report(
    view: &dyn crate::Snapshot,
    sources: &[crate::Source],
    scopes: &[crate::Scope],
) -> anyhow::Result<Report> {
    let now = crate::coverage::now()?;
    let meetings = view.commitment_sources(scopes)?;
    let queue = crate::queue::read(view, scopes)?;
    // The first Linear source the owner may see: its read, or its gap when the last sync failed.
    let linear = sources
        .iter()
        .filter(|s| s.connector == LINEAR && (scopes.is_empty() || scopes.contains(&s.scope)))
        .find_map(|s| view.linear_read(&s.name, scopes).transpose())
        .transpose()?;
    // The owner's Person, when one exists; without it no meeting ask is the owner's.
    let owner_id = view
        .resolve_entity(crate::identity::ME)
        .ok()
        .flatten()
        .map(|(id, _)| id)
        .unwrap_or_default();
    let offset = chrono::Local::now().offset().fix();
    Ok(report(&Input {
        now,
        offset,
        linear,
        owner_id: &owner_id,
        meetings: &meetings,
        queue: &queue,
    }))
}

impl<S: crate::Storage> crate::Rastro<S> {
    /// The focus report over every scope, on a writing open.
    pub fn focus(&mut self) -> anyhow::Result<Report> {
        let sources = &self.config.sources;
        self.storage
            .snapshot(|view| read_report(view, sources, &[]))
    }
}

impl<S: crate::ReadStorage> crate::Reader<S> {
    /// The focus report over the reader's scopes; a store that doesn't exist yet is an empty report.
    pub fn focus(&mut self) -> anyhow::Result<Report> {
        let Self {
            locations,
            storage,
            scopes,
            config,
        } = self;
        crate::read_only::snapshot(locations, storage, scopes, |view, _| match view {
            Some(view) => read_report(view, &config.sources, scopes),
            None => {
                let queue = Queue::empty();
                let now = crate::coverage::now()?;
                Ok(report(&Input {
                    now,
                    offset: chrono::Local::now().offset().fix(),
                    linear: None,
                    owner_id: "",
                    meetings: &[],
                    queue: &queue,
                }))
            }
        })
    }
}
