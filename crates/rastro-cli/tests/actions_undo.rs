//! `rastro actions undo` deletes the issue a done run created and records an undo intent on the
//! run, refuses a run that is not done, and is safe to repeat when the delete's response was lost.
//! The provider is the real `rastro-action-linear` plugin, and Linear is a fake GraphQL server on
//! loopback that records every request.
mod support;
use rusqlite::Connection;
use serde_json::Value;
use std::{
    fs,
    process::{Command, Output},
};
use support::linear_api::{self, Fixture};
use tempfile::TempDir;

struct Env {
    home: TempDir,
    linear: Fixture,
}

impl Env {
    /// A home with the Linear action provider installed, the Linear source `linear-work` in scope
    /// `work` and `[actions.linear] default_team = "ENG"`, and a fake Linear to write to.
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
            linear: Fixture::start(),
        };
        let config = env.home.path().join("config");
        support::provider::install_linear(&config, support::action_linear());
        fs::write(
            config.join("config.toml"),
            "[[sources]]\nname = \"linear-work\"\nconnector = \"linear\"\nselector = {}\n\
             scope = \"work\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
             keep_original = false\n\n[actions.linear]\ndefault_team = \"ENG\"\n",
        )
        .unwrap();
        env
    }

    fn rastro(&self, args: &[&str]) -> Output {
        Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("LINEAR_API_KEY", linear_api::KEY)
            .env("RASTRO_LINEAR_API_URL", &self.linear.base)
            .args(args)
            .output()
            .unwrap()
    }

    /// The `result` of a command that must succeed, read with `--json`.
    fn ok_json(&self, args: &[&str]) -> Value {
        let out = self.rastro(&[args, &["--json"]].concat());
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"].clone()
    }

    /// `rastro args` with the clock at `now`, for the grace window a test drives.
    fn rastro_at(&self, args: &[&str], now: &str) -> Output {
        Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_NOW", now)
            .env("LINEAR_API_KEY", linear_api::KEY)
            .env("RASTRO_LINEAR_API_URL", &self.linear.base)
            .args(args)
            .output()
            .unwrap()
    }

    /// A command that must succeed at `now`, read as the terminal reads it.
    fn ok_at(&self, args: &[&str], now: &str) {
        let out = self.rastro_at(args, now);
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
    }

    /// The `result` of a command that must succeed, read with `--json` at `now`.
    fn ok_json_at(&self, args: &[&str], now: &str) -> Value {
        let out = self.rastro_at(&[args, &["--json"]].concat(), now);
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"].clone()
    }

    /// Proposes an issue and accepts it, so the run's issue exists and the run reads `done`.
    /// Returns the run's id.
    fn done_run(&self) -> String {
        let item = self.propose();
        self.ok_json(&["queue", "accept", &item]);
        let run = self.run();
        assert_eq!(run["status"], "done", "{run}");
        run["id"].as_str().unwrap().to_string()
    }

    /// Proposes an issue and returns the queue item's id.
    fn propose(&self) -> String {
        let args = [
            "actions",
            "propose",
            "linear.create_issue",
            "--title",
            "Flaky sync test",
            "--why",
            "owner asked in session",
        ];
        self.ok_json(&args)["id"].as_str().unwrap().to_string()
    }

    /// The one run `rastro actions` lists.
    fn run(&self) -> Value {
        let runs = self.ok_json(&["actions"])["runs"].clone();
        let runs = runs.as_array().unwrap();
        assert_eq!(runs.len(), 1, "{runs:?}");
        runs[0].clone()
    }

    /// How many undo intents the owner's store holds.
    fn undo_intents(&self) -> i64 {
        let database = Connection::open(self.home.path().join("data/rastro.sqlite3")).unwrap();
        database
            .query_row(
                "SELECT count(*) FROM intent WHERE kind = 'undo_action'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }
}

// spec: The owner undoes an action
// spec: Human records render as blocks
// spec: Human timestamps share one format
#[test]
fn undoing_a_done_run_deletes_its_issue_and_the_run_reads_undone_with_the_undo_intent() {
    let env = Env::new();
    let run = env.done_run();
    let issue_id = env.linear.issues()[0]["id"].clone();

    // The undo reads as a block: the run's id on the header line, its fields one per line, and
    // the undo's time at minute precision.
    let out = env.rastro_at(&["actions", "undo", &run], "2026-11-19T12:00:00Z");
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.lines().any(|l| l == format!("Undone: Run {run}")),
        "{text}"
    );
    assert!(text.lines().any(|l| l == "  Status: undone"), "{text}");
    assert!(
        text.lines()
            .any(|l| l.starts_with("  Undone: intent ") && l.ends_with(", 2026-11-19 12:00 UTC")),
        "{text}"
    );
    let undone = env.run();

    let deletes = env.linear.deletes();
    assert_eq!(deletes.len(), 1, "one delete for the issue");
    assert_eq!(deletes[0].body["variables"]["id"], issue_id);
    assert_eq!(
        env.linear.issues()[0]["trashed"],
        true,
        "the issue is in the trash"
    );
    for run in [env.run(), undone] {
        assert_eq!(run["status"], "undone", "{run}");
        assert!(
            run["undo"]["intent"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
        );
        assert!(
            run["undo"]["at"]
                .as_str()
                .is_some_and(|at| at.contains('T'))
        );
    }
    assert_eq!(env.undo_intents(), 1);
}

// spec: Undo deletes only what it can verify
#[test]
fn undoing_a_run_that_failed_is_refused_naming_its_status_and_records_nothing() {
    let env = Env::new();
    let item = env.propose();
    env.linear.refuse_creates();
    assert!(!env.rastro(&["queue", "accept", &item]).status.success());
    let run = env.run();
    assert_eq!(run["status"], "failed");
    let received = env.linear.requests().len();

    let out = env.rastro(&["actions", "undo", run["id"].as_str().unwrap()]);

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("failed"), "{stderr}");
    assert_eq!(env.linear.requests().len(), received, "nothing was sent");
    assert_eq!(env.undo_intents(), 0);
    assert_eq!(env.run()["status"], "failed");
}

// spec: Undo deletes only what it can verify
#[test]
fn undoing_a_run_already_undone_is_refused_naming_its_status_and_records_nothing_more() {
    let env = Env::new();
    let run = env.done_run();
    env.ok_json(&["actions", "undo", &run]);
    let received = env.linear.requests().len();

    let out = env.rastro(&["actions", "undo", &run]);

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("undone"), "{stderr}");
    assert_eq!(env.linear.requests().len(), received, "nothing was sent");
    assert_eq!(env.undo_intents(), 1);
}

// spec: Undo is safe to repeat
#[test]
fn undoing_again_after_a_lost_delete_response_sends_no_second_delete_and_records_one_undo() {
    let env = Env::new();
    let run = env.done_run();
    env.linear.lose_next_delete_response();
    let lost = env.rastro(&["actions", "undo", &run]);
    assert!(!lost.status.success());
    assert_eq!(
        env.linear.issues()[0]["trashed"],
        true,
        "Linear trashed the issue"
    );
    assert_eq!(env.run()["status"], "done", "the owner never saw it go");
    assert_eq!(env.undo_intents(), 0);

    let again = env.ok_json(&["actions", "undo", &run]);

    assert_eq!(env.linear.deletes().len(), 1, "no second delete");
    assert_eq!(again["status"], "undone");
    assert_eq!(env.run()["status"], "undone");
    assert_eq!(env.undo_intents(), 1);
}

// spec: A demotion during grace returns the Act
// spec: An Act is checked again before it runs
#[test]
fn a_demotion_during_grace_returns_the_pending_act_and_sends_nothing() {
    let env = Env::new();
    // A rule on team H, and a source checked just now so a tick only runs actions.
    let config = env.home.path().join("config/config.toml");
    let text = fs::read_to_string(&config).unwrap();
    fs::write(
        &config,
        format!(
            "{text}\n[[rules]]\nname = \"my-issues\"\naction = \"linear.create_issue\"\n\
             team = \"H\"\nassignee = \"me\"\n"
        ),
    )
    .unwrap();
    support::seed::checked_now(env.home.path(), "linear-work", "work");

    // One Act runs to done; a second is pending when the owner undoes the first.
    let first = env.ok_json_at(
        &[
            "actions",
            "propose",
            "linear.create_issue",
            "--title",
            "First issue",
            "--team",
            "H",
            "--why",
            "owner asked",
        ],
        "2026-11-19T10:00:00Z",
    );
    let first_run = first["run"]["id"].as_str().unwrap().to_string();
    env.ok_at(&["tick"], "2026-11-19T10:10:00Z");
    assert_eq!(env.linear.creates().len(), 1);
    let second = env.ok_json_at(
        &[
            "actions",
            "propose",
            "linear.create_issue",
            "--title",
            "Second issue",
            "--team",
            "H",
            "--why",
            "owner asked",
        ],
        "2026-11-19T10:11:00Z",
    );
    let second_run = second["run"]["id"].as_str().unwrap().to_string();
    let second_proposal = second["id"].as_str().unwrap().to_string();
    assert_eq!(second["run"]["status"], "pending", "{second}");

    // The undo drops the cell below Act while the second Act is still in its grace.
    let undone = env.ok_json_at(&["actions", "undo", &first_run], "2026-11-19T10:12:00Z");
    assert_eq!(undone["status"], "undone", "{undone}");

    // At its grace end the recheck returns the pending Act: nothing is sent.
    env.ok_at(&["tick"], "2026-11-19T10:21:00Z");
    assert_eq!(
        env.linear.creates().len(),
        1,
        "the demoted cell's pending Act created an issue: {:?}",
        env.linear.requests()
    );
    let runs = env.ok_json_at(&["actions"], "2026-11-19T10:21:00Z")["runs"].clone();
    let returned = runs
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["id"] == second_run.as_str())
        .unwrap_or_else(|| panic!("no second run: {runs}"));
    assert_eq!(returned["status"], "returned", "{returned}");

    // The proposal waits in the queue for the owner.
    let items = env.ok_json_at(&["queue"], "2026-11-19T10:21:00Z")["items"].clone();
    let item = items
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == second_proposal.as_str())
        .unwrap_or_else(|| panic!("no queue item for {second_proposal}: {items:?}"));
    assert_eq!(item["kind"], "action_proposal", "{item}");

    // A returned run is final: a later tick sends nothing more.
    let received = env.linear.requests().len();
    env.ok_at(&["tick"], "2026-11-19T10:22:00Z");
    assert_eq!(env.linear.requests().len(), received);
}

// spec: Undo is safe to repeat
#[test]
fn undoing_a_run_whose_issue_is_archived_but_not_trashed_still_sends_the_delete() {
    let env = Env::new();
    let run = env.done_run();
    env.linear.archive_issues();

    let undone = env.ok_json(&["actions", "undo", &run]);

    assert_eq!(
        env.linear.deletes().len(),
        1,
        "an archived issue is not deleted"
    );
    assert_eq!(
        env.linear.issues()[0]["trashed"],
        true,
        "the issue is in the trash"
    );
    assert_eq!(undone["status"], "undone");
    assert_eq!(env.undo_intents(), 1);
}
