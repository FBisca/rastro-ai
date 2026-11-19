//! `rastro actions undo` reads the whole comment and change history of the run's issue before it
//! deletes it, and leaves the issue alone, with nothing recorded, when someone other than the owner
//! commented on or edited it or when that history cannot be read whole. The provider is the real
//! `rastro-action-linear` plugin, and Linear is a fake GraphQL server on loopback that pages an
//! issue's history and records every request.
mod support;
use rusqlite::Connection;
use serde_json::Value;
use std::{
    fs,
    process::{Command, Output},
};
use support::linear_api::{self, COLLEAGUE, Fixture, OWNER};
use tempfile::TempDir;

struct Env {
    home: TempDir,
    linear: Fixture,
}

impl Env {
    /// A home with the Linear action provider installed and a fake Linear to write to.
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

    /// Proposes an issue and accepts it, so the run's issue exists and the run reads `done`.
    /// Returns the run's id.
    fn done_run(&self) -> String {
        let args = [
            "actions",
            "propose",
            "linear.create_issue",
            "--title",
            "Flaky sync test",
            "--why",
            "owner asked in session",
        ];
        let item = self.ok_json(&args)["id"].as_str().unwrap().to_string();
        self.ok_json(&["queue", "accept", &item]);
        let run = self.run();
        assert_eq!(run["status"], "done", "{run}");
        run["id"].as_str().unwrap().to_string()
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

    /// Undoes `run`, which must be refused with the issue left alone and nothing recorded.
    /// Returns what the command said.
    fn refused_undo(&self, run: &str) -> String {
        let out = self.rastro(&["actions", "undo", run]);
        assert!(!out.status.success(), "the undo was not refused");
        assert!(self.linear.deletes().is_empty(), "a delete was sent");
        assert_eq!(self.linear.issues()[0]["trashed"], serde_json::Value::Null);
        assert_eq!(self.undo_intents(), 0);
        assert_eq!(self.run()["status"], "done");
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    /// The owner commented on and edited the issue, more times than a page holds.
    fn owner_touches_issue(&self) {
        for n in 0..3 {
            self.linear.comment(OWNER, &format!("note {n}"));
            self.linear.edit_title(OWNER, &format!("Title {n}"));
        }
    }
}

// spec: The owner undoes an action
#[test]
fn undoing_a_run_whose_issue_a_colleague_commented_on_is_refused_naming_them_and_the_comment() {
    let env = Env::new();
    let run = env.done_run();
    env.owner_touches_issue();
    env.linear.comment(COLLEAGUE, "I need this one open");

    let said = env.refused_undo(&run);

    assert!(said.contains(COLLEAGUE.name), "{said}");
    assert!(said.contains("I need this one open"), "{said}");
}

// spec: The owner undoes an action
#[test]
fn undoing_a_run_whose_issue_a_colleague_edited_is_refused_naming_them_and_the_edit() {
    let env = Env::new();
    let run = env.done_run();
    env.owner_touches_issue();
    env.linear
        .edit_title(COLLEAGUE, "Flaky sync test (blocking release)");

    let said = env.refused_undo(&run);

    assert!(said.contains(COLLEAGUE.name), "{said}");
    assert!(
        said.contains("Flaky sync test (blocking release)"),
        "{said}"
    );
}

// spec: Undo deletes only what it can verify
#[test]
fn undoing_a_run_whose_history_cannot_be_read_on_any_page_is_refused_saying_so() {
    for (read, page) in [
        ("IssueComments", 0),
        ("IssueComments", 1),
        ("IssueHistory", 1),
    ] {
        let env = Env::new();
        let run = env.done_run();
        env.owner_touches_issue();
        env.linear.fail_page(read, page);

        let said = env.refused_undo(&run);

        assert!(
            said.contains("history could not be checked"),
            "{read} {page}: {said}"
        );
    }
}

// spec: Undo deletes only what it can verify
#[test]
fn undoing_a_run_whose_issue_only_the_owner_changed_reads_all_its_history_then_deletes_it() {
    let env = Env::new();
    let run = env.done_run();
    env.owner_touches_issue();

    let undone = env.ok_json(&["actions", "undo", &run]);

    assert_eq!(env.linear.deletes().len(), 1);
    assert_eq!(undone["status"], "undone", "{undone}");
    assert_eq!(env.run()["status"], "undone");
    let operations: Vec<String> = (env.linear.requests().into_iter())
        .filter(|req| !matches!(req.operation.as_str(), "IssueCreate" | "IssueLookup"))
        .map(|req| {
            req.body["variables"]["after"]
                .as_str()
                .map_or(req.operation.clone(), |after| {
                    format!("{} after {after}", req.operation)
                })
        })
        .collect();
    for read in [
        "IssueComments",
        "IssueComments after 2",
        "IssueHistory",
        "IssueHistory after 2",
    ] {
        let at = operations.iter().position(|op| op == read);
        let delete = operations.iter().position(|op| op == "IssueDelete");
        assert!(
            at < delete && at.is_some(),
            "{read} before the delete: {operations:?}"
        );
    }
}
