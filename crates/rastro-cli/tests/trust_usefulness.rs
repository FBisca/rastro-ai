//! Per-plugin usefulness and cost in the trust view: engagement measured from the terminal's
//! shown reports and the intents that answered them, cost from the wall time of a plugin's runs
//! and the adapter turns they used. Runs, wall time, turns and hypotheses are seeded through the
//! store's own record API — the import flow is the meeting suites' subject — while what the owner
//! saw and answered travels the real CLI boundary: a terminal `queue` read records the shown
//! reports, `queue accept` and `queue reject` record the intents. The trust read under test
//! answers on both surfaces, and the clock is `RASTRO_NOW`.
mod support;
use rastro_core::{
    Counts, RunEnd, Scope, Source, Storage,
    commitments::{Identity, Kind, Known, PartyState, Quote, StatedDate},
    meeting::{Hypothesis, MeetingRef, Party, RunRecord, RunState, SessionTurn, TranscriptKeep},
    queue::proposed_ids,
};
use rastro_storage_sqlite::SqliteStorage;
use serde_json::{Value, json};
use std::{fs, path::PathBuf, process::Command};
use tempfile::TempDir;

/// The time a command runs at when the test names none.
const NOW: &str = "2026-11-20T10:00:00Z";
/// The time the terminal read shows the items.
const SHOWN: &str = "2026-11-20T11:00:00Z";

struct Env {
    home: TempDir,
}

impl Env {
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
        };
        fs::create_dir_all(env.config_dir()).unwrap();
        fs::write(env.config_dir().join("config.toml"), "").unwrap();
        env
    }

    fn config_dir(&self) -> PathBuf {
        self.home.path().join("config")
    }

    fn db(&self) -> SqliteStorage {
        SqliteStorage::open(&self.home.path().join("data/rastro.sqlite3")).unwrap()
    }

    fn rastro_at(&self, now: &str, args: &[&str]) -> std::process::Output {
        Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_NOW", now)
            .args(args)
            .output()
            .unwrap()
    }

    /// The `result` of a command that must succeed, read with `--json` at `now`, validated
    /// against the committed operation schema.
    fn ok_json_at(&self, now: &str, args: &[&str]) -> Value {
        let out = self.rastro_at(now, &[args, &["--json"]].concat());
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        let value: Value = serde_json::from_slice(&out.stdout).unwrap();
        let schema: Value =
            serde_json::from_str(include_str!("../../../schema/operations-v2.schema.json"))
                .unwrap();
        jsonschema::validator_for(&schema)
            .unwrap()
            .validate(&value)
            .unwrap_or_else(|error| panic!("{error}: {value}"));
        value["result"].clone()
    }

    /// The stdout of a command that must succeed, read as the terminal reads it.
    fn ok_text_at(&self, now: &str, args: &[&str]) -> String {
        let out = self.rastro_at(now, args);
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// The owner's Person, created on first use.
    fn owner(&self) -> Identity {
        let me = self.ok_json_at(NOW, &["identity", "me"])["person"].clone();
        Identity {
            id: me["id"].as_str().unwrap().to_string(),
            identifier: me["identifier"].as_str().unwrap().to_string(),
        }
    }

    /// One succeeded meeting run of `extractor`, seeded with its wall time, its adapter turns
    /// and one promise per item, each the owner's to keep. Returns the items' queue ids.
    fn seed_meeting(
        &self,
        meeting: &str,
        extractor: &str,
        started_at: &str,
        finished_at: &str,
        turns: usize,
        promises: &[&str],
    ) -> Vec<String> {
        let owner = self.owner();
        let hypotheses: Vec<Hypothesis> = promises
            .iter()
            .enumerate()
            .map(|(i, promise)| {
                let quote = format!("I'll {promise}");
                Hypothesis {
                    status: "hypothesis".into(),
                    kind: Kind::Promise,
                    responsible: Party {
                        state: PartyState::Mapped,
                        label: Some("Me".into()),
                        identity: Some(owner.clone()),
                    },
                    recipient: Party {
                        state: PartyState::Unknown,
                        label: Some("Bob".into()),
                        identity: None,
                    },
                    promise: (*promise).to_string(),
                    // Each item states its own date, so accepting one never settles another
                    // as already known.
                    stated_date: StatedDate {
                        state: Known::Stated,
                        text: Some(format!("by day {}", i + 1)),
                    },
                    quote: Quote {
                        end: quote.len(),
                        text: quote,
                        turn: 1,
                        speaker: "Me".into(),
                        start: 0,
                    },
                }
            })
            .collect();
        let record = RunRecord {
            meeting: MeetingRef {
                id: meeting.into(),
                scope: "work".into(),
                captured_at: "2026-11-20T08:00:00Z".parse().unwrap(),
            },
            kind: rastro_core::commitments::SourceKind::Meeting,
            started_at: started_at.parse().unwrap(),
            finished_at: finished_at.parse().unwrap(),
            state: RunState::Succeeded,
            cause: None,
            reason: None,
            extractor: Some(extractor.into()),
            owner_speaker: Some("Me".into()),
            turns_total: turns,
            turns_read: turns,
            excluded: 0,
            hypotheses: hypotheses.clone(),
            replay_of: None,
            recorded: None,
            transcript: TranscriptKeep::Drop,
            turns: (1..=turns)
                .map(|turn| SessionTurn {
                    turn,
                    request: json!({"method": "judgment.request"}).to_string(),
                    answer: Some(json!({"candidates": []}).to_string()),
                    verdict: json!({"gate": "ok"}).to_string(),
                })
                .collect(),
        };
        self.db().record_meeting_run(&record).unwrap();
        proposed_ids(meeting, "work", &hypotheses)
    }

    /// A connector run of `plugin` over a fresh source, finished after `wall_ms`.
    fn seed_connector_run(&self, source_name: &str, plugin: &str, started_at: &str, wall_ms: i64) {
        let start: chrono::DateTime<chrono::Utc> = started_at.parse().unwrap();
        let source = Source::new(source_name.into(), plugin.into(), Scope("work".into()));
        let mut db = self.db();
        db.project_sources(std::slice::from_ref(&source)).unwrap();
        let run = db
            .start_run(&source, (plugin, "1"), start, (start, start))
            .unwrap();
        db.finish_run(
            run,
            &RunEnd {
                status: "ok".into(),
                reason: None,
                finished_at: start + chrono::Duration::milliseconds(wall_ms),
                counts: Counts::default(),
            },
        )
        .unwrap();
    }

    /// The terminal queue read: what it prints is what it showed.
    fn show(&self, at: &str) {
        self.ok_text_at(at, &["queue"]);
    }

    fn accept(&self, item: &str, at: &str) {
        self.ok_json_at(at, &["queue", "accept", item]);
    }

    fn reject(&self, item: &str, at: &str) {
        self.ok_json_at(at, &["queue", "reject", item, "--reason", "not mine"]);
    }

    /// The trust answer's row for `plugin`.
    fn plugin<'a>(&self, answer: &'a Value, plugin: &str) -> &'a Value {
        answer["plugins"]
            .as_array()
            .unwrap_or_else(|| panic!("no plugins in the trust answer: {answer}"))
            .iter()
            .find(|row| row["plugin"] == json!(plugin))
            .unwrap_or_else(|| panic!("no plugins row for {plugin}: {answer}"))
    }
}

// spec: Usefulness is visible per plugin
#[test]
fn engagement_is_measured_from_shown_reports_and_answers() {
    let env = Env::new();
    let promises: Vec<String> = (1..=10).map(|i| format!("send report {i}")).collect();
    let promises: Vec<&str> = promises.iter().map(String::as_str).collect();
    let items = env.seed_meeting(
        "weekly-1",
        "test-adapter",
        "2026-11-20T09:00:00Z",
        "2026-11-20T09:02:00Z",
        1,
        &promises,
    );
    assert_eq!(items.len(), 10);

    // One terminal read shows all ten items; the owner accepts six, each after its own wait:
    // 60, 120, 180, 240, 300 and 360 seconds, so the time to action reads their mean, 210.
    env.show(SHOWN);
    for (i, item) in items.iter().take(6).enumerate() {
        let at = format!("2026-11-20T11:{:02}:00Z", i + 1);
        env.accept(item, &at);
    }

    let answer = env.ok_json_at("2026-11-20T12:00:00Z", &["trust"]);
    let row = env.plugin(&answer, "test-adapter");
    assert_eq!(row["engagement"]["acted"], 6, "{row}");
    assert_eq!(row["engagement"]["seen"], 10, "{row}");
    assert_eq!(row["time_to_action_seconds"], 210.0, "{row}");

    // The terminal read tells the same story.
    let text = env.ok_text_at("2026-11-20T12:00:00Z", &["trust"]);
    assert!(text.contains("test-adapter"), "{text}");
    assert!(text.contains("6 of 10"), "{text}");
    assert!(text.contains("210"), "{text}");
}

// spec: Usefulness is visible per plugin
#[test]
fn a_plugin_with_runs_but_no_shown_reports_reads_unmeasured() {
    let env = Env::new();
    // A meeting run whose items were never printed, and a connector read: both plugins ran,
    // nothing was shown, so neither engagement reads 0.
    env.seed_meeting(
        "weekly-1",
        "quiet-adapter",
        "2026-11-20T09:00:00Z",
        "2026-11-20T09:00:30Z",
        2,
        &["send report 1"],
    );
    env.seed_connector_run("linear-work", "linear", "2026-11-20T09:10:00Z", 30_000);

    let answer = env.ok_json_at(NOW, &["trust"]);
    let quiet = env.plugin(&answer, "quiet-adapter");
    assert_eq!(quiet["engagement"], json!("unmeasured"), "{quiet}");
    let linear = env.plugin(&answer, "linear");
    assert_eq!(linear["engagement"], json!("unmeasured"), "{linear}");
    // The connector run's wall time is cost even though nothing it read was ever shown.
    assert_eq!(linear["cost"]["wall_ms"], 30_000, "{linear}");
    assert_eq!(linear["cost"]["turns"], 0, "{linear}");

    let text = env.ok_text_at(NOW, &["trust"]);
    assert!(text.contains("quiet-adapter"), "{text}");
    assert!(text.contains("unmeasured"), "{text}");
    assert!(!text.contains("0 of 0"), "{text}");
}

// spec: Cost is wall time plus adapter turns
#[test]
fn cost_and_cost_per_accepted_read_wall_time_and_turns() {
    let env = Env::new();
    let items = env.seed_meeting(
        "weekly-1",
        "costly-adapter",
        "2026-11-20T09:00:00Z",
        "2026-11-20T09:02:00Z",
        4,
        &["send report 1", "send report 2"],
    );
    env.show(SHOWN);
    env.accept(&items[0], "2026-11-20T11:10:00Z");
    env.accept(&items[1], "2026-11-20T11:20:00Z");

    let answer = env.ok_json_at("2026-11-20T12:00:00Z", &["trust"]);
    let row = env.plugin(&answer, "costly-adapter");
    assert_eq!(row["cost"]["wall_ms"], 120_000, "{row}");
    assert_eq!(row["cost"]["turns"], 4, "{row}");
    assert_eq!(row["cost_per_accepted"]["wall_ms"], 60_000, "{row}");
    assert_eq!(row["cost_per_accepted"]["turns"], 2, "{row}");

    let text = env.ok_text_at("2026-11-20T12:00:00Z", &["trust"]);
    assert!(text.contains("120s"), "{text}");
    assert!(text.contains("4 turns"), "{text}");
    assert!(text.contains("60s"), "{text}");
    assert!(text.contains("2 turns"), "{text}");
}

// spec: Cost is wall time plus adapter turns
#[test]
fn zero_accepts_reads_a_dash() {
    let env = Env::new();
    let items = env.seed_meeting(
        "weekly-1",
        "costly-adapter",
        "2026-11-20T09:00:00Z",
        "2026-11-20T09:02:00Z",
        4,
        &["send report 1", "send report 2"],
    );
    env.show(SHOWN);
    // Both items were acted on — rejected — so engagement measures 2 of 2, but nothing was
    // accepted: the cost per accepted reads a dash, not a division by zero.
    env.reject(&items[0], "2026-11-20T11:10:00Z");
    env.reject(&items[1], "2026-11-20T11:20:00Z");

    let answer = env.ok_json_at("2026-11-20T12:00:00Z", &["trust"]);
    let row = env.plugin(&answer, "costly-adapter");
    assert_eq!(row["engagement"]["acted"], 2, "{row}");
    assert_eq!(row["engagement"]["seen"], 2, "{row}");
    assert_eq!(row["cost"]["wall_ms"], 120_000, "{row}");
    assert_eq!(row["cost_per_accepted"], json!("—"), "{row}");

    let text = env.ok_text_at("2026-11-20T12:00:00Z", &["trust"]);
    assert!(text.contains("2 of 2"), "{text}");
    assert!(text.contains("—"), "{text}");
}
