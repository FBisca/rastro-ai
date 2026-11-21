//! Demotion and decay: an undo or a correction of a cited claim drops the cell one level, two
//! undos in its last twenty outcomes drop it to Ask, stopped and returned runs demote nothing,
//! and a cell idle past the decay window drops a level at its next use. Every level change is
//! derived with its cause and time, so the journal names what moved the cell. The clock is
//! `RASTRO_NOW`; runs are driven through the real propose/accept/undo/stop/tick boundary against
//! the scripted fake provider, so no outside system is called.
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

    /// A command that must succeed at `now`, read as the terminal reads it.
    fn ok_at(&self, now: &str, args: &[&str]) {
        let out = self.rastro_at(now, args);
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
    }

    fn db(&self) -> SqliteStorage {
        SqliteStorage::open(&self.home.path().join("data/rastro.sqlite3")).unwrap()
    }

    /// A proposal at `now`, citing `cites`; the whole answer, so a test can read whether a rule
    /// started an Act or the proposal waits in the queue.
    fn propose(&self, title: &str, now: &str, cites: &[&str]) -> Value {
        let mut args = vec![
            "actions",
            "propose",
            "linear.create_issue",
            "--title",
            title,
            "--team",
            "H",
            "--why",
            "owner asked",
        ];
        for cite in cites {
            args.push("--cite");
            args.push(cite);
        }
        self.ok_json_at(now, &args)
    }

    /// A rule-started Act at `now`, finished through the store's own result record: executing
    /// an Act is the tick's work, not this read's. Returns the run's id.
    fn act_run(&self, title: &str, now: &str, cites: &[&str]) -> String {
        let proposed = self.propose(title, now, cites);
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

    /// The owner undoes a done run at `now`; the fake provider answers the delete.
    fn undo(&self, run: &str, now: &str) {
        let undone = self.ok_json_at(now, &["actions", "undo", run]);
        assert_eq!(undone["status"], "undone", "{undone}");
    }

    /// A claim observed in the work scope, seeded straight into the store, and its id.
    fn claim(&self, subject: &str) -> String {
        support::seed::claim_in(self.home.path(), "work", subject)
    }

    /// The owner retracts a claim at `now`, naming the revision they read.
    fn retract(&self, claim: &str, now: &str) {
        let why = self.ok_json_at(now, &["why", claim]);
        let revision = why["claim"]["revision"].as_str().unwrap();
        let out = self.rastro_at(now, &["claim", "reject", claim, "--seen", revision]);
        assert!(
            out.status.success(),
            "claim reject: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
    }

    /// The one cell the trust view holds, as JSON.
    fn cell(&self, now: &str) -> Value {
        let answer = self.ok_json_at(now, &["trust"]);
        let cells = answer["cells"].as_array().unwrap();
        assert_eq!(cells.len(), 1, "{answer}");
        cells[0].clone()
    }
}

// spec: Demotion is automatic and fast
#[test]
fn one_undo_drops_the_cell_one_level_and_the_journal_names_the_undo() {
    let env = Env::new();
    env.add_rule();
    let run = env.act_run("Issue 1", "2026-11-19T10:00:00Z", &[]);

    env.undo(&run, "2026-11-19T12:00:00Z");

    let cell = env.cell(NOW);
    assert_eq!(cell["level"], "propose", "{cell}");
    assert_eq!(cell["undos"], 1, "{cell}");
    // The journal names the undo as the cause, with the level it moved and its time.
    let changes = cell["changes"].as_array().unwrap_or_else(|| {
        panic!("the cell carries no journal of its level changes: {cell}");
    });
    assert_eq!(changes.len(), 1, "{cell}");
    assert_eq!(changes[0]["from"], "act", "{cell}");
    assert_eq!(changes[0]["to"], "propose", "{cell}");
    assert_eq!(changes[0]["cause"], "undo", "{cell}");
    assert!(
        changes[0]["at"]
            .as_str()
            .is_some_and(|at| at.contains("2026-11-19T12:00:00")),
        "{cell}"
    );
    assert_eq!(cell["last_change"]["cause"], "undo", "{cell}");
}

// spec: Demotion is automatic and fast
#[test]
fn two_undos_in_the_last_twenty_outcomes_drop_the_cell_to_ask() {
    let env = Env::new();
    env.add_rule();
    // Five outcomes, the last two of them undos: both sit well inside the last twenty.
    for day in 1..=3 {
        env.act_run(
            &format!("Issue {day}"),
            &format!("2026-11-{day:02}T10:00:00Z"),
            &[],
        );
    }
    let fourth = env.act_run("Issue 4", "2026-11-04T10:00:00Z", &[]);
    let fifth = env.act_run("Issue 5", "2026-11-05T10:00:00Z", &[]);
    env.undo(&fourth, "2026-11-04T12:00:00Z");
    env.undo(&fifth, "2026-11-05T12:00:00Z");

    let cell = env.cell(NOW);
    assert_eq!(cell["level"], "ask", "{cell}");
    assert_eq!(cell["undos"], 2, "{cell}");
    let changes = cell["changes"].as_array().unwrap_or_else(|| {
        panic!("the cell carries no journal of its level changes: {cell}");
    });
    assert_eq!(changes.len(), 2, "{cell}");
    assert_eq!(changes[0]["to"], "propose", "{cell}");
    assert_eq!(changes[1]["from"], "propose", "{cell}");
    assert_eq!(changes[1]["to"], "ask", "{cell}");
}

// spec: Demotion is automatic and fast
#[test]
fn a_retracted_citation_drops_the_cell_exactly_as_an_undo() {
    let env = Env::new();
    env.add_rule();
    let claim = env.claim("https://example.test/o/p/pull/1");
    let run = env.act_run("Issue 1", "2026-11-19T10:00:00Z", &[&claim]);
    assert_eq!(env.cell(NOW)["level"], "act");

    // The owner retracts the claim the done run cited.
    env.retract(&claim, "2026-11-19T12:00:00Z");

    let cell = env.cell(NOW);
    assert_eq!(cell["level"], "propose", "{cell}");
    assert_eq!(cell["undos"], 1, "{cell}");
    let changes = cell["changes"].as_array().unwrap_or_else(|| {
        panic!("the cell carries no journal of its level changes: {cell}");
    });
    assert_eq!(changes.len(), 1, "{cell}");
    assert_eq!(changes[0]["from"], "act", "{cell}");
    assert_eq!(changes[0]["to"], "propose", "{cell}");
    let cause = changes[0]["cause"].as_str().unwrap();
    assert!(cause.contains("retraction"), "{cause}");
    assert!(cause.contains(&claim), "{cause}");
    assert!(
        changes[0]["at"]
            .as_str()
            .is_some_and(|at| at.contains("2026-11-19T12:00:00")),
        "{cell}"
    );
    // The run itself is untouched: the correction demotes the cell, it does not undo the run.
    let runs = env.ok_json(&["actions"])["runs"].clone();
    assert_eq!(runs[0]["id"], run.as_str());
    assert_eq!(runs[0]["status"], "done", "{runs}");
}

// spec: Stopped and returned runs demote nothing
#[test]
fn stopped_and_returned_runs_leave_the_level_unchanged_and_the_journal_empty() {
    let env = Env::new();
    env.add_rule();
    // The source was checked just now, so a tick finds it fresh and only runs actions.
    support::seed::checked_now(env.home.path(), "linear-work", "work");
    let claim = env.claim("https://example.test/o/p/pull/2");
    support::seed::checked_now(env.home.path(), "notes-work", "work");

    // Two pending Acts: the owner stops the first; the second cites a claim that is retracted
    // during grace, so the pre-run recheck returns it.
    let stopped = env.propose("Stopped issue", "2026-11-19T10:00:00Z", &[]);
    let stopped = stopped["run"]["id"].as_str().unwrap();
    let returned = env.propose("Returned issue", "2026-11-19T10:01:00Z", &[&claim]);
    let returned = returned["run"]["id"].as_str().unwrap();
    env.ok_json_at("2026-11-19T10:02:00Z", &["actions", "stop", stopped]);
    env.retract(&claim, "2026-11-19T10:03:00Z");

    env.ok_at("2026-11-19T10:11:00Z", &["tick"]);

    let runs = env.ok_json_at("2026-11-19T10:11:00Z", &["actions"])["runs"].clone();
    let by_id = |id: &str| {
        runs.as_array()
            .unwrap()
            .iter()
            .find(|run| run["id"] == id)
            .unwrap_or_else(|| panic!("no run {id}: {runs}"))
            .clone()
    };
    assert_eq!(by_id(stopped)["status"], "stopped", "{runs}");
    assert_eq!(by_id(returned)["status"], "returned", "{runs}");

    // Neither run demotes: the cell stays at Act and the journal records no change.
    let cell = env.cell("2026-11-19T10:11:00Z");
    assert_eq!(cell["level"], "act", "{cell}");
    assert_eq!(cell["undos"], 0, "{cell}");
    assert_eq!(cell["changes"], serde_json::json!([]), "{cell}");
    assert!(cell["last_change"].is_null(), "{cell}");
}

// spec: Unused trust decays on next use
#[test]
fn a_cell_idle_past_the_window_decays_at_its_next_use() {
    let env = Env::new();
    env.add_rule();
    // The cell last ran on October 1st; the default decay window is 30 days.
    env.act_run("Old issue", "2026-10-01T10:00:00Z", &[]);
    assert_eq!(env.cell("2026-10-01T10:00:00Z")["level"], "act");

    // November 1st is 31 days later: a new proposal uses the cell, and the decay applies.
    env.propose("New issue", "2026-11-01T10:00:00Z", &[]);

    let cell = env.cell("2026-11-01T10:00:00Z");
    assert_eq!(cell["level"], "propose", "{cell}");
    let changes = cell["changes"].as_array().unwrap_or_else(|| {
        panic!("the cell carries no journal of its level changes: {cell}");
    });
    assert_eq!(changes.len(), 1, "{cell}");
    assert_eq!(changes[0]["from"], "act", "{cell}");
    assert_eq!(changes[0]["to"], "propose", "{cell}");
    let cause = changes[0]["cause"].as_str().unwrap();
    assert!(cause.contains("decay"), "{cause}");
    assert!(cause.contains("30"), "the cause names the window: {cause}");
    assert!(
        changes[0]["at"]
            .as_str()
            .is_some_and(|at| at.contains("2026-11-01T10:00:00")),
        "{cell}"
    );
}
