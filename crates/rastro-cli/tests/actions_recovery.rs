//! An action run creates its issue exactly once, however it is cut short or raced: the run's issue
//! id is looked up before it is created, one process executes a run at a time, and `rastro tick`
//! finishes an accepted run that never completed. The provider is the real `rastro-action-linear`
//! plugin, and Linear is a fake GraphQL server on loopback that records every request.
mod support;
use rusqlite::Connection;
use serde_json::Value;
use std::{
    fs, net,
    process::{Command, Output, Stdio},
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
        // Checked just now, so a tick leaves the source to sync later and only finishes runs.
        support::seed::checked_now(env.home.path(), "linear-work", "work");
        env
    }

    /// `rastro args`, with Linear at `api`.
    fn command(&self, args: &[&str], api: &str) -> Command {
        let mut command = Command::new(support::rastro());
        command
            .env("RASTRO_HOME", self.home.path())
            .env("LINEAR_API_KEY", linear_api::KEY)
            .env("RASTRO_LINEAR_API_URL", api)
            .args(args);
        command
    }

    fn rastro(&self, args: &[&str]) -> Output {
        self.command(args, &self.linear.base).output().unwrap()
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

    /// The runs `rastro actions` lists.
    fn runs(&self) -> Vec<Value> {
        self.ok_json(&["actions"])["runs"]
            .as_array()
            .unwrap()
            .clone()
    }

    /// A tick that must succeed.
    fn tick(&self) -> String {
        let out = self.rastro(&["tick"]);
        assert!(
            out.status.success(),
            "tick: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// The operations Linear has received, oldest first.
    fn operations(&self) -> Vec<String> {
        (self.linear.requests().into_iter())
            .map(|req| req.operation)
            .collect()
    }
}

/// An address nothing listens on, so a request to it is never answered.
fn nobody_home() -> String {
    let port = net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    format!("http://127.0.0.1:{port}")
}

// spec: Accepting a proposal runs the action once
#[test]
fn a_run_executed_again_after_its_issue_was_created_creates_nothing_and_reads_done_once() {
    let env = Env::new();
    let item = env.propose();
    env.ok_json(&["queue", "accept", &item]);
    assert_eq!(env.linear.creates().len(), 1);
    // The first execution made the issue but the run never recorded it: drop its result.
    let database = Connection::open(env.home.path().join("data/rastro.sqlite3")).unwrap();
    database
        .execute_batch("DROP TRIGGER action_result_no_delete; DELETE FROM action_result;")
        .unwrap();
    drop(database);
    assert_eq!(env.runs()[0]["status"], "accepted");

    env.tick();

    assert_eq!(env.linear.creates().len(), 1, "{:?}", env.operations());
    assert_eq!(env.linear.issues().len(), 1);
    let runs = env.runs();
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0]["status"], "done");
    assert_eq!(runs[0]["identifier"], "ENG-1");
    let database = Connection::open(env.home.path().join("data/rastro.sqlite3")).unwrap();
    let results: i64 = database
        .query_row("SELECT count(*) FROM action_result", [], |row| row.get(0))
        .unwrap();
    assert_eq!(results, 1);
}

// spec: An interrupted run is finished by tick
#[test]
fn tick_executes_an_accepted_run_that_was_cut_off_before_any_request_was_sent() {
    let env = Env::new();
    let item = env.propose();
    // The accept is recorded, then the process cannot reach Linear: no request is sent.
    let out = env
        .command(&["queue", "accept", &item], &nobody_home())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(env.linear.requests().is_empty());
    assert_eq!(env.runs()[0]["status"], "failed");

    env.tick();

    assert_eq!(env.linear.creates().len(), 1, "{:?}", env.operations());
    let runs = env.runs();
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0]["status"], "done");
    assert_eq!(runs[0]["identifier"], "ENG-1");
}

// spec: An interrupted run is finished by tick
#[test]
fn tick_finds_the_issue_by_the_runs_id_when_the_create_response_was_lost() {
    let env = Env::new();
    let item = env.propose();
    env.linear.lose_next_create_response();
    let out = env.rastro(&["queue", "accept", &item]);
    assert!(!out.status.success());
    assert_eq!(env.linear.issues().len(), 1, "Linear made the issue");
    assert_eq!(env.runs()[0]["status"], "failed");
    let issue_id = env.runs()[0]["issue_id"].as_str().unwrap().to_string();

    env.tick();

    assert_eq!(env.linear.creates().len(), 1, "{:?}", env.operations());
    let lookups: Vec<_> = (env.linear.requests().into_iter())
        .filter(|req| req.operation == "IssueLookup")
        .collect();
    assert!(
        lookups
            .iter()
            .any(|req| req.body["variables"]["id"] == issue_id.as_str()),
        "tick never looked the issue up by the run's id: {:?}",
        env.operations()
    );
    let runs = env.runs();
    assert_eq!(runs[0]["status"], "done");
    assert_eq!(runs[0]["identifier"], "ENG-1");
}

// spec: An interrupted run is finished by tick
#[test]
fn a_tick_leaves_a_run_another_process_is_executing_untouched() {
    let env = Env::new();
    let item = env.propose();
    // The accept creates the issue but waits for Linear's answer, holding the run's execution.
    env.linear.hold_create_responses();
    let accept = env
        .command(&["queue", "accept", &item], &env.linear.base)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    env.linear.wait_for_creates(1);
    // The accept looked the id up, found nothing, resolved the team and assignee, and created the issue.
    let before = env.operations();
    assert_eq!(before, ["IssueLookup", "Teams", "Viewer", "IssueCreate"]);

    let out = env.rastro(&["tick"]);

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(env.operations(), before, "the tick reached Linear");
    assert_eq!(env.runs()[0]["status"], "accepted");

    env.linear.release_creates();
    let accepted = accept.wait_with_output().unwrap();
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    assert_eq!(env.linear.creates().len(), 1);
    assert_eq!(env.runs()[0]["status"], "done");
}
