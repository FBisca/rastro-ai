//! The learning journal: every level change the visible cells derived, newest first, each with
//! its cause and time, and a claims rebuild that reproduces the trust view and the journal
//! exactly. The clock is `RASTRO_NOW`; runs are driven through the real propose/accept/undo
//! boundary against the scripted fake provider, so no outside system is called.
mod support;
use rastro_core::{Storage, actions::RunResult};
use rastro_storage_sqlite::SqliteStorage;
use serde_json::Value;
use std::{fs, process::Command};
use tempfile::TempDir;

/// The time a command runs at when the test names none.
const NOW: &str = "2026-11-19T10:00:00Z";

struct Env {
    home: TempDir,
}

impl Env {
    /// A home with the scripted fake provider and the Linear source `linear-work` in scope
    /// `work`, and no rule until the test adds one.
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
        };
        support::provider::install(&env.config_dir(), |_| {});
        env.set_config(&env.source_config());
        env
    }

    fn config_dir(&self) -> std::path::PathBuf {
        self.home.path().join("config")
    }

    /// The config with one Linear source in scope `work` and nothing else.
    fn source_config(&self) -> String {
        "[[sources]]\nname = \"linear-work\"\nconnector = \"linear\"\nselector = {}\n\
         scope = \"work\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
         keep_original = false\n"
            .to_string()
    }

    fn set_config(&self, text: &str) {
        fs::write(self.config_dir().join("config.toml"), text).unwrap();
    }

    fn config(&self) -> String {
        fs::read_to_string(self.config_dir().join("config.toml")).unwrap()
    }

    /// The owner's rule on team `H`, appended to the config: the cell's baseline is Act.
    fn add_rule(&self) {
        self.set_config(&format!(
            "{}\n[[rules]]\nname = \"my-issues\"\naction = \"linear.create_issue\"\n\
             team = \"H\"\nassignee = \"me\"\n",
            self.config()
        ));
    }

    /// A command at `now`; the fake provider answers every action message.
    fn rastro_at(&self, now: &str, args: &[&str]) -> std::process::Output {
        Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_NOW", now)
            .args(args)
            .output()
            .unwrap()
    }

    /// The `result` of a command that must succeed, read with `--json` at `now`.
    fn ok_json_at(&self, now: &str, args: &[&str]) -> Value {
        let out = self.rastro_at(now, &[args, &["--json"]].concat());
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"].clone()
    }

    fn ok_json(&self, args: &[&str]) -> Value {
        self.ok_json_at(NOW, args)
    }

    /// The stdout of a command that must succeed, read as the terminal reads it.
    fn ok_text(&self, args: &[&str]) -> String {
        let out = self.rastro_at(NOW, args);
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn db(&self) -> SqliteStorage {
        SqliteStorage::open(&self.home.path().join("data/rastro.sqlite3")).unwrap()
    }

    /// A proposal at `now`; the whole answer, so a test can read whether a rule started an Act
    /// or the proposal waits in the queue.
    fn propose(&self, title: &str, now: &str) -> Value {
        self.ok_json_at(
            now,
            &[
                "actions",
                "propose",
                "linear.create_issue",
                "--title",
                title,
                "--team",
                "H",
                "--why",
                "owner asked",
            ],
        )
    }

    /// A rule-started Act at `now`, finished through the store's own result record: executing
    /// an Act is the tick's work, not this read's. Returns the run's id.
    fn act_run(&self, title: &str, now: &str) -> String {
        let proposed = self.propose(title, now);
        assert_eq!(proposed["run"]["status"], "pending", "{proposed}");
        let id = proposed["run"]["id"].as_str().unwrap().to_string();
        self.db()
            .record_action_result(
                &id,
                &RunResult {
                    identifier: format!("H-{title}"),
                    url: format!("https://fake.invalid/H-{title}"),
                    finished_at: now.parse().unwrap(),
                },
            )
            .unwrap();
        id
    }

    /// A proposal the owner accepts unchanged at `now`: its run reads `done` through the fake
    /// provider. Returns the run's id.
    fn clean_run(&self, title: &str, team: &str, now: &str) -> String {
        let item = self.ok_json_at(
            now,
            &[
                "actions",
                "propose",
                "linear.create_issue",
                "--title",
                title,
                "--team",
                team,
                "--why",
                "owner asked",
            ],
        )["id"]
            .as_str()
            .unwrap()
            .to_string();
        let answered = self.ok_json_at(now, &["queue", "accept", &item]);
        assert_eq!(answered["run"]["status"], "done", "{answered}");
        answered["run"]["id"].as_str().unwrap().to_string()
    }

    /// The owner undoes a done run at `now`; the fake provider answers the delete.
    fn undo(&self, run: &str, now: &str) {
        let undone = self.ok_json_at(now, &["actions", "undo", run]);
        assert_eq!(undone["status"], "undone", "{undone}");
    }
}

// spec: The learning journal records every level change
#[test]
fn the_journal_lists_changes_newest_first_with_their_causes_and_times() {
    let env = Env::new();
    env.add_rule();
    // The cell runs on October 1st; the owner undoes that run on October 2nd.
    let run = env.act_run("Old issue", "2026-10-01T10:00:00Z");
    env.undo(&run, "2026-10-02T10:00:00Z");
    // November 1st is 31 days later: a new proposal uses the cell and the decay applies.
    env.propose("New issue", "2026-11-01T10:00:00Z");

    let journal = env.ok_json(&["trust", "journal"]);
    let changes = journal["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 2, "{journal}");
    // Newest first: the decay, then the undo it followed. Each names its cell, its cause and
    // its time.
    assert_eq!(
        changes[0]["cell"]["action"], "linear.create_issue",
        "{journal}"
    );
    assert_eq!(changes[0]["cell"]["team"], "H", "{journal}");
    assert_eq!(changes[0]["from"], "propose", "{journal}");
    assert_eq!(changes[0]["to"], "ask", "{journal}");
    assert!(
        changes[0]["cause"]
            .as_str()
            .is_some_and(|cause| cause.contains("decay")),
        "{journal}"
    );
    assert!(
        changes[0]["at"]
            .as_str()
            .is_some_and(|at| at.contains("2026-11-01T10:00:00")),
        "{journal}"
    );
    assert_eq!(changes[1]["cell"]["team"], "H", "{journal}");
    assert_eq!(changes[1]["from"], "act", "{journal}");
    assert_eq!(changes[1]["to"], "propose", "{journal}");
    assert_eq!(changes[1]["cause"], "undo", "{journal}");
    assert!(
        changes[1]["at"]
            .as_str()
            .is_some_and(|at| at.contains("2026-10-02T10:00:00")),
        "{journal}"
    );

    // The terminal read lists the same changes, newest first.
    let text = env.ok_text(&["trust", "journal"]);
    let decay = text
        .find("decay")
        .unwrap_or_else(|| panic!("no decay: {text}"));
    let undo = text
        .find("undo")
        .unwrap_or_else(|| panic!("no undo: {text}"));
    assert!(decay < undo, "the journal reads newest first: {text}");
}

// spec: A cell's trust is derived from its outcomes
#[test]
fn a_rebuild_reproduces_the_trust_view_and_the_journal() {
    let env = Env::new();
    env.add_rule();
    // H: an Act run idle past the decay window, a new use that decays it, an undo that demotes
    // it further, and a reset that restores its baseline.
    let old = env.act_run("Old issue", "2026-10-01T10:00:00Z");
    env.propose("New issue", "2026-11-01T10:00:00Z");
    env.undo(&old, "2026-11-02T10:00:00Z");
    // G: a clean run pinned at Propose, then undone: the pin holds the level.
    let pinned = env.clean_run("Pinned issue", "G", "2026-11-03T10:00:00Z");
    env.ok_json_at(
        "2026-11-03T12:00:00Z",
        &["trust", "pin", "linear.create_issue", "G", "propose"],
    );
    env.undo(&pinned, "2026-11-04T10:00:00Z");
    env.ok_json_at(
        "2026-11-05T10:00:00Z",
        &["trust", "reset", "linear.create_issue", "H"],
    );
    // A terminal queue read records what it showed; the rebuild carries it too.
    env.ok_text(&["queue"]);

    let trust_before = env.ok_json(&["trust"]);
    let journal_before = env.ok_json(&["trust", "journal"]);
    assert!(
        !journal_before["changes"].as_array().unwrap().is_empty(),
        "{journal_before}"
    );

    // The store is rebuilt from the same evidence, intents and shown reports.
    env.ok_json(&["claims", "rebuild"]);

    assert_eq!(env.ok_json(&["trust"]), trust_before);
    assert_eq!(env.ok_json(&["trust", "journal"]), journal_before);
}
