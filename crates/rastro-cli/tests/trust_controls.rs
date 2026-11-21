//! Pin and reset as owner intents: a pin holds the cell's level against learning in both
//! directions, a reset clears its learned history back to its baseline, and the journal names
//! both. The clock is `RASTRO_NOW`; runs are driven through the real propose/accept/undo
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

    /// The owner's rule on `team`, appended to the config: that cell's baseline is Act.
    fn add_rule(&self, team: &str) {
        self.set_config(&format!(
            "{}\n[[rules]]\nname = \"my-issues\"\naction = \"linear.create_issue\"\n\
             team = \"{team}\"\nassignee = \"me\"\n",
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

    fn db(&self) -> SqliteStorage {
        SqliteStorage::open(&self.home.path().join("data/rastro.sqlite3")).unwrap()
    }

    /// A rule-started Act on `team` at `now`, finished through the store's own result record:
    /// executing an Act is the tick's work, not this read's. Returns the run's id.
    fn act_run(&self, title: &str, team: &str, now: &str) -> String {
        let proposed = self.ok_json_at(
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
        );
        assert_eq!(proposed["run"]["status"], "pending", "{proposed}");
        let id = proposed["run"]["id"].as_str().unwrap().to_string();
        self.db()
            .record_action_result(
                &id,
                &RunResult {
                    identifier: format!("{team}-{title}"),
                    url: format!("https://fake.invalid/{team}-{title}"),
                    finished_at: now.parse().unwrap(),
                },
            )
            .unwrap();
        id
    }

    /// A proposal on `team` the owner accepts unchanged at `now`: its run reads `done` through
    /// the fake provider. Returns the run's id.
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

    /// The trust view's one cell on `team`, as JSON.
    fn cell(&self, team: &str) -> Value {
        let answer = self.ok_json(&["trust"]);
        answer["cells"]
            .as_array()
            .unwrap()
            .iter()
            .find(|cell| cell["team"] == team)
            .unwrap_or_else(|| panic!("no cell on team {team}: {answer}"))
            .clone()
    }
}

// spec: Pin and reset are owner intents
#[test]
fn a_pin_holds_the_level_against_an_undo_and_the_journal_names_the_pin() {
    let env = Env::new();
    let run = env.clean_run("Issue 1", "H", "2026-11-01T10:00:00Z");
    env.ok_json_at(
        "2026-11-02T10:00:00Z",
        &["trust", "pin", "linear.create_issue", "H", "propose"],
    );
    // Without the pin the undo would drop the cell to Ask.
    env.undo(&run, "2026-11-03T10:00:00Z");

    let cell = env.cell("H");
    assert_eq!(cell["level"], "propose", "{cell}");
    assert_eq!(cell["undos"], 1, "{cell}");
    // The journal names the pin holding the level, with the cause it held and its time.
    let changes = cell["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1, "{cell}");
    assert_eq!(changes[0]["from"], "propose", "{cell}");
    assert_eq!(changes[0]["to"], "propose", "{cell}");
    let cause = changes[0]["cause"].as_str().unwrap();
    assert!(cause.contains("pin"), "{cause}");
    assert!(cause.contains("undo"), "{cause}");
    assert!(
        changes[0]["at"]
            .as_str()
            .is_some_and(|at| at.contains("2026-11-03T10:00:00")),
        "{cell}"
    );
    let journal = env.ok_json(&["trust", "journal"]);
    let changes = journal["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1, "{journal}");
    assert_eq!(changes[0]["cell"]["team"], "H", "{journal}");
    assert!(
        changes[0]["cause"]
            .as_str()
            .is_some_and(|cause| cause.contains("pin")),
        "{journal}"
    );
}

// spec: Pin and reset are owner intents
#[test]
fn a_pin_holds_the_level_against_a_rule_too() {
    let env = Env::new();
    env.clean_run("Issue 1", "H", "2026-11-01T10:00:00Z");
    env.ok_json_at(
        "2026-11-02T10:00:00Z",
        &["trust", "pin", "linear.create_issue", "H", "propose"],
    );
    // The rule raises the baseline to Act; the pin holds the cell at Propose regardless.
    env.add_rule("H");

    let cell = env.cell("H");
    assert_eq!(cell["level"], "propose", "{cell}");
}

// spec: Pin and reset are owner intents
#[test]
fn a_reset_restores_each_cells_baseline_and_the_journal_names_the_reset() {
    let env = Env::new();
    env.add_rule("H");
    // H sits at Act by its rule; G has no rule and sits at Propose. One undo demotes each.
    let h = env.act_run("H issue", "H", "2026-11-01T10:00:00Z");
    let g = env.clean_run("G issue", "G", "2026-11-01T10:00:00Z");
    env.undo(&h, "2026-11-02T10:00:00Z");
    env.undo(&g, "2026-11-02T10:00:00Z");
    assert_eq!(env.cell("H")["level"], "propose");
    assert_eq!(env.cell("G")["level"], "ask");

    let reset_h = env.ok_json_at(
        "2026-11-03T10:00:00Z",
        &["trust", "reset", "linear.create_issue", "H"],
    );
    let reset_g = env.ok_json_at(
        "2026-11-03T10:00:00Z",
        &["trust", "reset", "linear.create_issue", "G"],
    );
    // The cell with a rule returns to Act; the one without returns to Propose.
    assert_eq!(reset_h["level"], "act", "{reset_h}");
    assert_eq!(reset_g["level"], "propose", "{reset_g}");

    // The learned history is cleared: the undo no longer counts against either cell.
    let h = env.cell("H");
    assert_eq!(h["level"], "act", "{h}");
    assert_eq!(h["clean"], 0, "{h}");
    assert_eq!(h["undos"], 0, "{h}");
    let g = env.cell("G");
    assert_eq!(g["level"], "propose", "{g}");
    assert_eq!(g["clean"], 0, "{g}");
    assert_eq!(g["undos"], 0, "{g}");

    // The journal names each reset with the levels it restored and its time, newest first;
    // the undos it cleared follow.
    let journal = env.ok_json(&["trust", "journal"]);
    let changes = journal["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 4, "{journal}");
    assert_eq!(changes[0]["cause"], "reset", "{journal}");
    assert_eq!(changes[1]["cause"], "reset", "{journal}");
    assert_eq!(changes[2]["cause"], "undo", "{journal}");
    assert_eq!(changes[3]["cause"], "undo", "{journal}");
    let reset_of = |team: &str| {
        changes
            .iter()
            .find(|change| change["cell"]["team"] == team && change["cause"] == "reset")
            .unwrap_or_else(|| panic!("no reset for {team}: {journal}"))
            .clone()
    };
    let h = reset_of("H");
    assert_eq!(h["from"], "propose", "{h}");
    assert_eq!(h["to"], "act", "{h}");
    assert!(
        h["at"]
            .as_str()
            .is_some_and(|at| at.contains("2026-11-03T10:00:00")),
        "{h}"
    );
    let g = reset_of("G");
    assert_eq!(g["from"], "ask", "{g}");
    assert_eq!(g["to"], "propose", "{g}");
}

// spec: Promotion is proposed, never self-applied
#[test]
fn a_demoted_eligible_cell_is_proposed_a_reset_and_stays_demoted_until_it() {
    let env = Env::new();
    env.set_config(&format!(
        "{}\n[learning]\nclean_min = 3\nclean_days = 1\nundo_window = 3\n",
        env.source_config()
    ));
    env.add_rule("H");
    let first = env.act_run("Issue 1", "H", "2026-11-01T10:00:00Z");
    env.undo(&first, "2026-11-02T10:00:00Z");
    // Three clean outcomes push the undo out of the last-three window: eligible again.
    for (day, title) in [(3, "Issue 2"), (4, "Issue 3"), (5, "Issue 4")] {
        env.clean_run(title, "H", &format!("2026-11-{day:02}T10:00:00Z"));
    }

    let cell = env.cell("H");
    assert_eq!(cell["level"], "propose", "{cell}");
    assert_eq!(cell["promotion"]["eligible"], true, "{cell}");
    assert_eq!(
        cell["promotion"]["reset_command"], "rastro trust reset linear.create_issue H",
        "{cell}"
    );
    assert!(cell["promotion"]["rule_text"].is_null(), "{cell}");

    // The level stays demoted until the owner runs the proposed reset; then it is Act.
    let reset = env.ok_json_at(
        "2026-11-06T10:00:00Z",
        &["trust", "reset", "linear.create_issue", "H"],
    );
    assert_eq!(reset["level"], "act", "{reset}");
    assert_eq!(env.cell("H")["level"], "act");
}
