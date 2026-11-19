//! The owner edits or rejects an action proposal from the queue. The provider is the real
//! `rastro-action-linear` plugin, and Linear is a fake GraphQL server on loopback that records
//! every request, so a test sees what reached it.
mod support;
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

    /// Proposes `linear.create_issue` with `extra` after the title, and returns the item's id.
    fn propose(&self, title: &str, extra: &[&str]) -> String {
        let mut args = vec![
            "actions",
            "propose",
            "linear.create_issue",
            "--title",
            title,
        ];
        args.extend_from_slice(extra);
        self.ok_json(&args)["id"].as_str().unwrap().to_string()
    }

    /// The runs `rastro actions` lists.
    fn runs(&self) -> Vec<Value> {
        self.ok_json(&["actions"])["runs"]
            .as_array()
            .unwrap()
            .clone()
    }
}

// spec: The owner edits or rejects an action proposal
#[test]
fn an_edited_action_proposal_creates_the_issue_with_the_edited_title() {
    let env = Env::new();
    let item = env.propose("Flaky sync test", &["--why", "owner asked in session"]);

    env.ok_json(&["queue", "edit", &item, "--title", "Sync test flakes in CI"]);

    let creates = env.linear.creates();
    assert_eq!(creates.len(), 1, "{creates:?}");
    assert_eq!(env.linear.requests().len(), 1, "only the create was sent");
    let input = creates[0].input();
    assert_eq!(input["title"], "Sync test flakes in CI");
    assert_eq!(input["teamId"], "ENG");
    assert_eq!(input["assigneeId"], "me");
    let runs = env.runs();
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0]["status"], "done");
    assert_eq!(runs[0]["fields"]["title"], "Sync test flakes in CI");
    assert!(
        env.ok_json(&["queue"])["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

// spec: The owner edits or rejects an action proposal
#[test]
fn a_rejected_action_proposal_runs_nothing_and_leaves_the_queue() {
    let env = Env::new();
    let item = env.propose("Flaky sync test", &["--why", "owner asked in session"]);

    env.ok_json(&["queue", "reject", &item, "--reason", "not worth a ticket"]);

    assert!(env.linear.requests().is_empty(), "a reject reached Linear");
    assert!(env.runs().is_empty());
    assert!(
        env.ok_json(&["queue"])["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
