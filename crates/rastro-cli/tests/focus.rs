//! The focus report: ranking, citations, gaps, the queue count and the empty case, at the core's
//! public `focus::report` seam over scripted evidence (no database, no process).
#![allow(non_snake_case)]
use chrono::{DateTime, FixedOffset, Utc};
use std::collections::BTreeSet;
use rastro_core::{
    CoverageState,
    commitments::{
        self, Extraction, Hypothesis, Identity, Kind, Known, Party, PartyState, Quote, ResultsFrom,
        SourceCoverage, SourceKind as CommitmentSourceKind, StatedDate,
    },
    focus::{
        self, GapReason, Input, LinearRead, Read, Reason, SourceKind, Task, TaskState, Urgency,
    },
    meeting::{Cause, RunState},
    queue::{Origin, Queue},
};

fn at(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

/// 2026-11-19 10:00 in the owner's local offset (UTC+1).
fn now() -> DateTime<Utc> {
    at("2026-11-19T09:00:00Z")
}

fn task(identifier: &str, state: TaskState, due: Option<&str>, urgency: Urgency) -> Task {
    Task {
        location: format!("linear:issue/{identifier}"),
        identifier: identifier.into(),
        title: format!("Work on {identifier}"),
        url: Some(format!("https://linear.app/h/issue/{identifier}")),
        state,
        due: due.map(at),
        urgency,
    }
}

fn good(tasks: Vec<Task>) -> Option<LinearRead> {
    Some(LinearRead {
        read: Read::Good {
            at: at("2026-11-19T08:30:00Z"),
        },
        tasks,
    })
}

/// The owner's id in the identity graph, as a mapped party carries it.
const OWNER_ID: &str = "person:owner";

fn owner() -> Party {
    Party {
        state: PartyState::Mapped,
        label: Some("Me".into()),
        identity: Some(Identity {
            id: OWNER_ID.into(),
            identifier: "rastro:person/owner".into(),
        }),
    }
}

fn ana() -> Party {
    Party {
        state: PartyState::Unknown,
        label: Some("Ana".into()),
        identity: None,
    }
}

/// `responsible` owes `recipient` the `promise`.
fn hypothesis_between(
    kind: Kind,
    responsible: Party,
    recipient: Party,
    promise: &str,
) -> Hypothesis {
    Hypothesis::new(
        kind,
        responsible,
        recipient,
        promise.into(),
        StatedDate {
            state: Known::Unknown,
            text: None,
        },
        Quote {
            text: format!("I'll send {promise}."),
            turn: 3,
            speaker: "Me".into(),
            start: 0,
            end: 10,
        },
    )
}

/// A hypothesis the owner owes Ana.
fn hypothesis(kind: Kind, promise: &str) -> Hypothesis {
    hypothesis_between(kind, owner(), ana(), promise)
}

/// A meeting source whose latest extraction ended as `state`, holding `hypotheses`.
fn meeting(state: RunState, hypotheses: Vec<Hypothesis>) -> commitments::Source {
    let failed = state == RunState::Failed;
    commitments::Source {
        kind: CommitmentSourceKind::Meeting,
        name: "weekly-1".into(),
        scope: "work".into(),
        extraction: Extraction {
            state,
            cause: failed.then_some(Cause::ExtractorFailed),
            reason: None,
            run: "2".into(),
            finished_at: at("2026-11-19T07:40:00Z"),
            adapter: None,
            replay_of: None,
            excluded: 0,
        },
        coverage: SourceCoverage {
            state: if failed {
                CoverageState::CouldNotLook
            } else {
                CoverageState::Found
            },
            turns_read: 4,
            turns_total: 4,
            reason: None,
        },
        stale: failed,
        results_from: Some(ResultsFrom {
            run: "1".into(),
            imported_at: at("2026-11-18T17:00:00Z"),
            replayed_at: None,
        }),
        hypotheses,
    }
}

fn report(linear: Option<LinearRead>, meetings: &[commitments::Source]) -> focus::Report {
    report_with_queue(linear, meetings, &queue_of(meetings, &BTreeSet::new()))
}

/// The queue the meetings' proposals make, less those whose ids are in `answered`.
fn queue_of(meetings: &[commitments::Source], answered: &BTreeSet<String>) -> Queue {
    let origins = (meetings.iter())
        .map(|source| Origin {
            at: at("2026-11-18T10:00:00Z"),
            source: source.clone(),
            known: vec![],
        })
        .collect();
    Queue::new(origins, vec![], vec![], answered)
}

fn report_with_queue(
    linear: Option<LinearRead>,
    meetings: &[commitments::Source],
    queue: &Queue,
) -> focus::Report {
    focus::report(&Input {
        now: now(),
        offset: FixedOffset::east_opt(3600).unwrap(),
        linear,
        owner_id: OWNER_ID,
        meetings,
        queue,
    })
}

fn ids(report: &focus::Report) -> Vec<String> {
    report
        .focus
        .iter()
        .chain(&report.needs_you)
        .map(|item| item.title.clone())
        .collect()
}

// spec: A focus report ranks what needs the owner
#[test]
fn an_overdue_task_is_the_focus_above_one_in_progress() {
    let report = report(
        good(vec![
            task("H-8095", TaskState::InProgress, None, Urgency::Urgent),
            task(
                "H-8102",
                TaskState::InProgress,
                Some("2026-11-18T12:00:00Z"),
                Urgency::Low,
            ),
        ]),
        &[],
    );
    let focus = report.focus.as_ref().expect("a recommended focus");
    assert_eq!(focus.title, "Work on H-8102");
    assert_eq!(focus.reason, Reason::Overdue);
    assert_eq!(report.needs_you.len(), 1);
    assert_eq!(report.needs_you[0].title, "Work on H-8095");
    assert_eq!(report.needs_you[0].reason, Reason::InProgress);
}

// spec: A focus report ranks what needs the owner
#[test]
fn a_waiting_promise_is_the_focus_when_nothing_is_due() {
    let report = report(
        good(vec![task(
            "H-8095",
            TaskState::InProgress,
            None,
            Urgency::Urgent,
        )]),
        &[meeting(
            RunState::Succeeded,
            vec![hypothesis(Kind::Promise, "the doc for Ana")],
        )],
    );
    let focus = report.focus.as_ref().expect("a recommended focus");
    assert_eq!(focus.title, "the doc for Ana");
    assert_eq!(focus.reason, Reason::WaitingOnOwner);
    assert_eq!(focus.citation.source, SourceKind::Meeting);
    assert_eq!(report.needs_you[0].title, "Work on H-8095");
}

// spec: A focus report ranks what needs the owner
#[test]
fn what_others_owe_the_owner_is_not_the_owners_to_do() {
    let meetings = [meeting(
        RunState::Succeeded,
        vec![
            hypothesis_between(Kind::Promise, ana(), owner(), "the doc from Ana"),
            hypothesis_between(Kind::Ask, ana(), owner(), "the budget from Ana"),
            hypothesis_between(Kind::WaitingOn, ana(), owner(), "Ana's numbers"),
            hypothesis_between(Kind::Ask, owner(), ana(), "the review for Ana"),
        ],
    )];
    let report = report(
        good(vec![task(
            "H-8095",
            TaskState::InProgress,
            None,
            Urgency::Low,
        )]),
        &meetings,
    );
    assert_eq!(
        ids(&report),
        ["the review for Ana", "Work on H-8095"],
        "only what the owner owes is waiting on the owner"
    );
}

// spec: A focus report ranks what needs the owner
#[test]
fn no_candidate_is_an_empty_report_not_an_error() {
    let report = report(
        good(vec![task("H-1", TaskState::Closed, None, Urgency::None)]),
        &[meeting(RunState::Succeeded, vec![])],
    );
    assert!(report.focus.is_none());
    assert!(report.needs_you.is_empty());
    assert_eq!(report.notice.as_deref(), Some(focus::NOTHING_NEEDS_YOU));
}

// spec: Focus ranking is deterministic and ordered
#[test]
fn two_candidates_in_one_band_rank_by_due_then_urgency_then_id() {
    let report = report(
        good(vec![
            task(
                "D",
                TaskState::Open,
                Some("2026-11-19T15:00:00Z"),
                Urgency::Low,
            ),
            task(
                "C",
                TaskState::Open,
                Some("2026-11-19T15:00:00Z"),
                Urgency::High,
            ),
            task(
                "B",
                TaskState::Open,
                Some("2026-11-19T15:00:00Z"),
                Urgency::High,
            ),
            task(
                "A",
                TaskState::Open,
                Some("2026-11-19T18:00:00Z"),
                Urgency::Urgent,
            ),
            task(
                "Z",
                TaskState::Open,
                Some("2026-11-19T11:00:00Z"),
                Urgency::None,
            ),
        ]),
        &[],
    );
    // Z is due earliest; B and C tie on due and urgency, so id order; D is less urgent; A is
    // later though the most urgent, and the report holds a focus plus three, so A is cut.
    assert_eq!(
        ids(&report),
        ["Work on Z", "Work on B", "Work on C", "Work on D"]
    );
}

// spec: Focus ranking is deterministic and ordered
#[test]
fn the_same_evidence_gives_the_same_report_in_any_order() {
    let tasks = vec![
        task("H-1", TaskState::InProgress, None, Urgency::High),
        task(
            "H-2",
            TaskState::Open,
            Some("2026-11-17T09:00:00Z"),
            Urgency::Low,
        ),
        task("H-3", TaskState::InProgress, None, Urgency::High),
    ];
    let mut reversed = tasks.clone();
    reversed.reverse();
    let meetings = [meeting(
        RunState::Succeeded,
        vec![hypothesis(Kind::Ask, "the budget")],
    )];
    let first = report(good(tasks), &meetings);
    assert_eq!(first.focus.as_ref().unwrap().title, "Work on H-2");
    assert_eq!(first, report(good(reversed), &meetings));
}

// spec: Focus cites and tells the truth about gaps
#[test]
fn a_failed_source_shows_its_gap_and_its_items_keep_the_last_good_time() {
    let linear = Some(LinearRead {
        read: Read::Failed {
            reason: GapReason::SyncFailed,
            last_good: Some(at("2026-11-19T07:35:00Z")),
        },
        tasks: vec![task(
            "H-8102",
            TaskState::InProgress,
            Some("2026-11-18T12:00:00Z"),
            Urgency::High,
        )],
    });
    let report = report(linear, &[]);
    assert_eq!(report.gaps.len(), 1);
    assert_eq!(report.gaps[0].source, SourceKind::Linear);
    assert_eq!(report.gaps[0].reason, GapReason::SyncFailed);
    assert_eq!(report.gaps[0].as_of, Some(at("2026-11-19T07:35:00Z")));
    let focus = report.focus.as_ref().unwrap();
    assert_eq!(focus.citation.observed_at, at("2026-11-19T07:35:00Z"));
    assert_eq!(focus.citation.location, "linear:issue/H-8102");
    assert_eq!(report.as_of, at("2026-11-19T07:35:00Z"));
}

// spec: Focus cites and tells the truth about gaps
#[test]
fn a_failed_meeting_extraction_is_a_typed_gap_stamped_with_its_last_good_import() {
    let report = report(
        None,
        &[meeting(
            RunState::Failed,
            vec![hypothesis(Kind::Promise, "the doc for Ana")],
        )],
    );
    assert_eq!(
        report.gaps[0].reason,
        GapReason::ExtractionFailed {
            cause: Some(Cause::ExtractorFailed)
        }
    );
    assert_eq!(
        report.focus.as_ref().unwrap().citation.observed_at,
        at("2026-11-18T17:00:00Z")
    );
}

// spec: Focus cites and tells the truth about gaps
#[test]
fn a_source_that_returned_nothing_shows_no_gap() {
    let report = report(
        good(vec![]),
        &[meeting(
            RunState::Succeeded,
            vec![hypothesis(Kind::Ask, "the budget")],
        )],
    );
    assert!(report.gaps.is_empty());
    assert_eq!(report.focus.as_ref().unwrap().title, "the budget");
}

// spec: Focus shows the queue as a count
#[test]
fn the_queue_is_a_count_and_never_its_text() {
    let proposals = meeting(
        RunState::Succeeded,
        vec![
            hypothesis(Kind::Promise, "SECRET-PROPOSAL-ONE"),
            hypothesis(Kind::Promise, "SECRET-PROPOSAL-TWO"),
        ],
    );
    let queue = Queue::new(
        vec![Origin {
            at: at("2026-11-18T10:00:00Z"),
            source: proposals,
            known: vec![],
        }],
        vec![],
        vec![],
        &Default::default(),
    );
    assert_eq!(queue.items.len(), 2);
    let report = report_with_queue(good(vec![]), &[], &queue);
    assert_eq!(report.queue_count, 2);
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("SECRET-PROPOSAL"), "{json}");
}

// spec: A focus report ranks what needs the owner
#[test]
fn a_promise_the_owner_already_decided_leaves_the_focus() {
    let meetings = [meeting(
        RunState::Succeeded,
        vec![
            hypothesis(Kind::Promise, "the decided doc"),
            hypothesis(Kind::Promise, "the pending deck"),
        ],
    )];
    let pending = queue_of(&meetings, &BTreeSet::new());
    let decided = (pending.items.iter())
        .find(|item| item.proposal.as_ref().unwrap().promise == "the decided doc")
        .unwrap()
        .id
        .clone();
    let queue = queue_of(&meetings, &BTreeSet::from([decided]));
    let report = report_with_queue(good(vec![]), &meetings, &queue);
    assert_eq!(ids(&report), ["the pending deck"]);
}
