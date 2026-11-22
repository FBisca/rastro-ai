//! The turn history a session keeps (ADR 0016 §3): every turn's request, answer and
//! gate verdict, failed turns included; an export leaves it out and forgetting removes it.

use chrono::{DateTime, Utc};
use rastro_core::Storage;
use rastro_core::meeting::{Cause, MeetingRef, RunRecord, RunState, TranscriptKeep};
use rastro_storage_sqlite::{SessionTurn, SqliteStorage};

fn time(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

fn open(home: &tempfile::TempDir) -> SqliteStorage {
    SqliteStorage::open(&home.path().join("rastro.sqlite3")).unwrap()
}

/// A finished failed run, seeded through the store so the turn history under test is written
/// against a real `conversation_run` row.
fn seed_run(storage: &mut SqliteStorage, meeting: &str, transcript: TranscriptKeep) -> i64 {
    storage
        .record_meeting_run(&RunRecord {
            meeting: MeetingRef {
                id: meeting.into(),
                scope: "work".into(),
                captured_at: time("2026-11-10T10:00:00Z"),
            },
            started_at: time("2026-11-10T10:00:00Z"),
            finished_at: time("2026-11-10T10:01:00Z"),
            state: RunState::Failed,
            cause: Some(Cause::InvalidQuote),
            reason: Some("the quote is not a contiguous piece of its turn".into()),
            extractor: Some("test-adapter".into()),
            owner_speaker: Some("Alice".into()),
            turns_total: 1,
            turns_read: 1,
            excluded: 0,
            hypotheses: Vec::new(),
            replay_of: None,
            recorded: None,
            transcript,
            turns: Vec::new(),
        })
        .unwrap()
}

/// S4: a run that failed after retry turns keeps every turn's request, answer and gate verdict,
/// in turn order, across a reopen.
// spec: Every turn of a judgment session is recorded
#[test]
fn a_runs_history_holds_every_turns_request_answer_and_verdict() {
    let home = tempfile::tempdir().unwrap();
    let mut storage = open(&home);
    let run = seed_run(&mut storage, "weekly-1", TranscriptKeep::Drop);
    let turns = vec![
        SessionTurn {
            turn: 1,
            request: r#"{"method":"judgment.request","params":{"turn":1}}"#.into(),
            answer: Some(r#"{"candidates":[{"quote":"I'll send the budget sometime"}]}"#.into()),
            verdict: r#"{"gate":"quote","field":"candidates[0].quote","passed":false}"#.into(),
        },
        SessionTurn {
            turn: 2,
            request: r#"{"method":"judgment.request","params":{"turn":2,"feedback":{"gate":"quote","expected":"an exact contiguous quote","actual":"I'll send the budget sometime"}}}"#.into(),
            answer: Some(r#"{"candidates":[{"quote":"I'll send Bob the budget by Friday."}]}"#.into()),
            verdict: r#"{"gate":"quote","field":"candidates[0].quote","passed":false}"#.into(),
        },
        SessionTurn {
            turn: 3,
            request: r#"{"method":"judgment.request","params":{"turn":3,"feedback":{"gate":"quote"}}}"#.into(),
            answer: Some(r#"{"candidates":[{"quote":"I'll send Bob the budget by Friday"}]}"#.into()),
            verdict: r#"{"gate":"quote","field":"candidates[0].quote","passed":false}"#.into(),
        },
    ];
    for turn in &turns {
        storage.record_session_turn(run, turn).unwrap();
    }
    assert_eq!(storage.session_turns(run).unwrap(), turns);
    // The history survives the connection that wrote it.
    drop(storage);
    assert_eq!(open(&home).session_turns(run).unwrap(), turns);
}

/// A turn the adapter never answered (a timeout) still keeps its request and the core's verdict,
/// with no answer.
// spec: Every turn of a judgment session is recorded
#[test]
fn a_turn_the_adapter_never_answered_keeps_its_request_and_verdict() {
    let home = tempfile::tempdir().unwrap();
    let mut storage = open(&home);
    let run = seed_run(&mut storage, "weekly-1", TranscriptKeep::Drop);
    let turn = SessionTurn {
        turn: 1,
        request: r#"{"method":"judgment.request","params":{"turn":1}}"#.into(),
        answer: None,
        verdict: r#"{"gate":"system","passed":false,"reason":"the adapter timed out"}"#.into(),
    };
    storage.record_session_turn(run, &turn).unwrap();
    assert_eq!(storage.session_turns(run).unwrap(), vec![turn]);
}

/// S6: an export leaves the turn history out, even when the transcript is retained.
// spec: Every turn of a judgment session is recorded
#[test]
fn an_export_leaves_turn_history_out() {
    let home = tempfile::tempdir().unwrap();
    let mut storage = open(&home);
    let transcript = "Alice: I'll send Bob the budget by Friday.\n";
    let run = seed_run(
        &mut storage,
        "weekly-1",
        TranscriptKeep::Store(transcript.into()),
    );
    let turn = SessionTurn {
        turn: 1,
        request:
            r#"{"method":"judgment.request","params":{"instructions":"judge the transcript"}}"#
                .into(),
        answer: Some(r#"{"candidates":[],"session_id":"sess-42"}"#.into()),
        verdict: r#"{"gate":"quote","passed":false,"feedback":"the quote is not contiguous"}"#
            .into(),
    };
    storage.record_session_turn(run, &turn).unwrap();
    assert_eq!(
        storage.session_turns(run).unwrap(),
        vec![turn.clone()],
        "the history must be stored for the export to leave it out"
    );
    let exported = storage
        .snapshot(|view| view.meeting_export("weekly-1"))
        .unwrap()
        .expect("the meeting exists");
    let exported = serde_json::to_value(&exported).unwrap();
    // Only what the owner supplied: no request, no answer, no gate verdict.
    let mut keys: Vec<_> = exported.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "format",
            "meeting",
            "notice",
            "owner_speaker",
            "speakers",
            "transcript"
        ],
        "{exported}"
    );
    assert_eq!(exported["transcript"], transcript);
    let text = exported.to_string();
    for marker in [
        "judge the transcript",
        "sess-42",
        "contiguous",
        "feedback",
        "judgment.request",
    ] {
        assert!(!text.contains(marker), "{marker} leaked: {text}");
    }
}

/// Turn history follows recorded-output retention (plan decision D16): forgetting a meeting
/// removes its turns, and another meeting's history stays.
#[test]
fn forgetting_a_meeting_removes_its_turn_history() {
    let home = tempfile::tempdir().unwrap();
    let mut storage = open(&home);
    let run = seed_run(&mut storage, "weekly-1", TranscriptKeep::Drop);
    let other = seed_run(&mut storage, "weekly-2", TranscriptKeep::Drop);
    let turn = SessionTurn {
        turn: 1,
        request: r#"{"method":"judgment.request","params":{"turn":1}}"#.into(),
        answer: Some(r#"{"candidates":[]}"#.into()),
        verdict: r#"{"gate":"quote","passed":false}"#.into(),
    };
    storage.record_session_turn(run, &turn).unwrap();
    storage.record_session_turn(other, &turn).unwrap();
    let removed = storage
        .forget_meeting("weekly-1")
        .unwrap()
        .expect("the meeting exists");
    assert_eq!(removed.runs, 1);
    assert!(
        storage.session_turns(run).unwrap().is_empty(),
        "the forgotten meeting's turn history stays behind"
    );
    assert_eq!(storage.session_turns(other).unwrap(), vec![turn]);
}
