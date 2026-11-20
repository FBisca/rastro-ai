//! An issue create reaches Linear under Linear's own ids: the team key becomes its team's id and
//! the assignee `me` becomes the key owner's user id. The provider is the real
//! `rastro-action-linear` plugin, and Linear is a fake GraphQL server on loopback that holds teams
//! and refuses a team or assignee that is not one of its ids, as Linear does.
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

    /// Proposes an issue for `team` and returns the item's id.
    fn propose(&self, team: &str) -> String {
        let args = [
            "actions",
            "propose",
            "linear.create_issue",
            "--title",
            "Flaky sync test",
            "--team",
            team,
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
}

// spec: A created issue names Linear's own ids
#[test]
fn a_run_with_team_h_creates_the_issue_under_team_hs_linear_id() {
    let env = Env::new();
    let item = env.propose("H");

    env.ok_json(&["queue", "accept", &item]);

    let creates = env.linear.creates();
    assert_eq!(creates.len(), 1, "{creates:?}");
    assert_eq!(creates[0].input()["teamId"], linear_api::TEAM_H.id);
    // The run still names the team as the rule and the owner wrote it.
    let run = env.run();
    assert_eq!(run["fields"]["team"], "H");
    assert_eq!(run["status"], "done");
}

// spec: A created issue names Linear's own ids
#[test]
fn a_run_with_assignee_me_creates_the_issue_assigned_to_the_owners_linear_user_id() {
    let env = Env::new();
    let item = env.propose("H");

    env.ok_json(&["queue", "accept", &item]);

    let creates = env.linear.creates();
    assert_eq!(creates.len(), 1, "{creates:?}");
    assert_eq!(creates[0].input()["assigneeId"], linear_api::OWNER.id);
    // The run still names the assignee as written.
    assert_eq!(env.run()["fields"]["assignee"], "me");
}

// spec: A created issue names Linear's own ids
#[test]
fn a_team_key_linear_does_not_hold_creates_nothing_and_the_error_names_the_key() {
    let env = Env::new();
    let item = env.propose("ZZZ");

    let accept = env.rastro(&["queue", "accept", &item]);

    assert!(!accept.status.success());
    assert!(
        env.linear.creates().is_empty(),
        "{:?}",
        env.linear.requests()
    );
    let stderr = String::from_utf8_lossy(&accept.stderr);
    assert!(stderr.contains("ZZZ"), "{stderr}");
    let run = env.run();
    assert_eq!(run["status"], "failed");
    assert!(run["error"].as_str().unwrap().contains("ZZZ"), "{run}");
}
