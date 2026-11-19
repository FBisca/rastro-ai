//! A failed Action run is recorded with its error and its attempts, and `rastro tick` retries it
//! until it has made 3 attempts, then leaves it failed. The provider is the real
//! `rastro-action-linear` plugin, and Linear is a fake GraphQL server on loopback that records
//! every request.
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
        // Checked just now, so a tick leaves the source to sync later and only runs actions.
        support::seed::checked_now(env.home.path(), "linear-work", "work");
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

    /// Proposes an issue and accepts it while Linear refuses every create: the accept fails, and
    /// the run it recorded is the first failed attempt. Returns the accept's output.
    fn accept_while_linear_refuses(&self) -> Output {
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
        self.linear.refuse_creates();
        self.rastro(&["queue", "accept", &item])
    }

    /// The one run `rastro actions` lists.
    fn run(&self) -> Value {
        let runs = self.ok_json(&["actions"])["runs"].clone();
        let runs = runs.as_array().unwrap();
        assert_eq!(runs.len(), 1, "{runs:?}");
        runs[0].clone()
    }
}

// spec: A failed run is retried by tick, then left failed
#[test]
fn a_failed_create_fails_the_accept_and_is_recorded_with_its_error_and_one_attempt() {
    let env = Env::new();

    let accept = env.accept_while_linear_refuses();

    assert!(!accept.status.success());
    let stderr = String::from_utf8_lossy(&accept.stderr);
    assert!(stderr.contains("INTERNAL_SERVER_ERROR"), "{stderr}");
    let run = env.run();
    assert_eq!(run["status"], "failed");
    assert_eq!(run["attempts"], 1);
    assert!(
        run["error"]
            .as_str()
            .unwrap()
            .contains("INTERNAL_SERVER_ERROR"),
        "{run}"
    );
    // The accept intent stays recorded: the run names it, and the proposal left the queue.
    assert!(
        run["accept_intent"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    let queue = env.ok_json(&["queue"]);
    assert!(!queue.to_string().contains("Flaky sync test"), "{queue}");
}

// spec: A failed run is retried by tick, then left failed
#[test]
fn tick_retries_a_failed_run_with_fewer_than_three_attempts_and_the_run_reads_done() {
    let env = Env::new();
    assert!(!env.accept_while_linear_refuses().status.success());
    assert_eq!(env.run()["status"], "failed");
    assert_eq!(env.run()["attempts"], 1);
    env.linear.accept_creates();

    let tick = env.rastro(&["tick"]);

    assert!(
        tick.status.success(),
        "{}",
        String::from_utf8_lossy(&tick.stderr)
    );
    assert_eq!(env.linear.creates().len(), 2, "one refused, one made");
    assert_eq!(env.linear.issues().len(), 1);
    let run = env.run();
    assert_eq!(run["status"], "done");
    assert_eq!(run["identifier"], "ENG-1");
}

// spec: A failed run is retried by tick, then left failed
#[test]
fn tick_leaves_a_run_that_has_made_three_attempts_failed_and_sends_nothing() {
    let env = Env::new();
    assert!(!env.accept_while_linear_refuses().status.success());
    // Two ticks are the second and third attempts, each failing again.
    for attempts in [2, 3] {
        let tick = env.rastro(&["tick"]);
        assert!(!tick.status.success());
        assert_eq!(env.run()["attempts"], attempts);
    }
    let received = env.linear.requests().len();
    // Linear is well again, but the run has used its attempts.
    env.linear.accept_creates();

    let tick = env.rastro(&["tick"]);

    assert!(
        tick.status.success(),
        "{}",
        String::from_utf8_lossy(&tick.stderr)
    );
    assert_eq!(env.linear.requests().len(), received);
    let run = env.run();
    assert_eq!(run["status"], "failed");
    assert_eq!(run["attempts"], 3);
}
