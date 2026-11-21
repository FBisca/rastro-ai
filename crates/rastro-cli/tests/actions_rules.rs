//! The level gate: a matching rule starts an Act only while the cell sits at Act. A cell demoted
//! by an undo keeps its matching proposals waiting in the queue — no run starts and nothing is
//! sent. The provider is the real `rastro-action-linear` plugin and Linear is a fake GraphQL
//! server on loopback that records every request; the clock is `RASTRO_NOW`, so the grace window
//! passes without a wait.
mod support;
use serde_json::Value;
use std::{fs, process::Command};
use support::linear_api::{self, Fixture};
use tempfile::TempDir;

/// When the first proposal is made: the rule's grace of 10 minutes ends at 10:10.
const PROPOSED_AT: &str = "2026-11-19T10:00:00Z";
/// The instant the first Act's grace ends.
const DUE: &str = "2026-11-19T10:10:00Z";

const SOURCE: &str = "[[sources]]\nname = \"linear-work\"\nconnector = \"linear\"\nselector = {}\n\
     scope = \"work\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
     keep_original = false\n";
const RULE: &str = "\n[[rules]]\nname = \"my-issues\"\naction = \"linear.create_issue\"\nteam = \"H\"\n\
     assignee = \"me\"\n";

struct Env {
    home: TempDir,
    linear: Fixture,
}

impl Env {
    /// A home with the real Linear provider, a fake Linear to write to, the Linear source
    /// `linear-work` in scope `work` and the rule `my-issues` on team `H`. The source was checked
    /// just now, so a tick finds it fresh and only runs actions.
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
            linear: Fixture::start(),
        };
        support::provider::install_linear(&env.config_dir(), support::action_linear());
        fs::write(
            env.config_dir().join("config.toml"),
            format!("{SOURCE}{RULE}"),
        )
        .unwrap();
        support::seed::checked_now(env.home.path(), "linear-work", "work");
        env
    }

    fn config_dir(&self) -> std::path::PathBuf {
        self.home.path().join("config")
    }

    fn rastro(&self, args: &[&str], now: &str) -> std::process::Output {
        Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_NOW", now)
            .env("LINEAR_API_KEY", linear_api::KEY)
            .env("RASTRO_LINEAR_API_URL", &self.linear.base)
            .args(args)
            .output()
            .unwrap()
    }

    /// The `result` of a command that must succeed, read with `--json` at `now`.
    fn ok_json(&self, args: &[&str], now: &str) -> Value {
        let out = self.rastro(&[args, &["--json"]].concat(), now);
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"].clone()
    }

    /// A command that must succeed at `now`, read as the terminal reads it.
    fn ok_at(&self, args: &[&str], now: &str) {
        let out = self.rastro(args, now);
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
    }

    /// Proposes `title` for team `H` at `now`; the whole answer.
    fn propose(&self, title: &str, now: &str) -> Value {
        self.ok_json(
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
            now,
        )
    }
}

// spec: A rule cannot raise a demoted cell
// spec: A matching rule starts an Act
#[test]
fn a_demoted_cells_matching_proposal_waits_in_the_queue() {
    let env = Env::new();
    // One Act runs to done under the rule, then the owner undoes it: the cell drops to Propose.
    let first = env.propose("First issue", PROPOSED_AT);
    let run = first["run"]["id"].as_str().unwrap().to_string();
    assert_eq!(first["run"]["status"], "pending", "{first}");
    env.ok_at(&["tick"], DUE);
    assert_eq!(
        env.linear.creates().len(),
        1,
        "the first Act created its issue"
    );
    let undone = env.ok_json(&["actions", "undo", &run], "2026-11-19T10:11:00Z");
    assert_eq!(undone["status"], "undone", "{undone}");
    assert_eq!(
        env.ok_json(&["trust"], "2026-11-19T10:11:00Z")["cells"][0]["level"],
        "propose"
    );

    // A matching proposal under the same rule: the demoted cell starts no Act.
    let received = env.linear.requests().len();
    let second = env.propose("Second issue", "2026-11-19T10:12:00Z");
    assert!(
        second["run"].is_null(),
        "a demoted cell's match started a run: {second}"
    );

    // The proposal waits in the queue as an action proposal.
    let items = env.ok_json(&["queue"], "2026-11-19T10:12:00Z")["items"].clone();
    let item = items
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == second["id"])
        .unwrap_or_else(|| panic!("no queue item for the proposal: {items:?}"));
    assert_eq!(item["kind"], "action_proposal", "{item}");
    assert_eq!(item["action"]["fields"]["title"], "Second issue", "{item}");

    // No run for the proposal: the only run is the undone one.
    let runs = env.ok_json(&["actions"], "2026-11-19T10:12:00Z")["runs"].clone();
    let runs = runs.as_array().unwrap();
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0]["id"], run.as_str());
    assert_eq!(runs[0]["status"], "undone");

    // Nothing reached Linear for the waiting proposal.
    assert_eq!(
        env.linear.requests().len(),
        received,
        "a demoted cell's match reached Linear: {:?}",
        env.linear.requests()
    );

    // A later tick still sends nothing: the proposal waits for the owner, not for a grace end.
    env.ok_at(&["tick"], "2026-11-19T10:30:00Z");
    assert_eq!(env.linear.requests().len(), received);
    assert_eq!(env.linear.creates().len(), 1);
}
