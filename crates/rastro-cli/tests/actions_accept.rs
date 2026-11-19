//! Accepting an action proposal creates its issue in Linear, once, with exactly the content the
//! owner accepted. The provider is the real `rastro-action-linear` plugin, and Linear is a fake
//! GraphQL server on loopback that records every request, so a test sees what reached it.
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

// spec: Accepting a proposal runs the action once
#[test]
fn accepting_an_action_proposal_creates_the_issue_once_and_the_run_reads_done() {
    let env = Env::new();
    let item = env.propose("Flaky sync test", &["--why", "owner asked in session"]);
    assert!(
        env.linear.requests().is_empty(),
        "a proposal reached Linear"
    );

    env.ok_json(&["queue", "accept", &item]);

    let creates = env.linear.creates();
    assert_eq!(creates.len(), 1, "{creates:?}");
    let sent: Vec<_> = (env.linear.requests().into_iter())
        .map(|req| req.operation)
        .collect();
    assert_eq!(
        sent,
        ["IssueLookup", "IssueCreate"],
        "the id is looked up first"
    );
    let create = &creates[0];
    assert_eq!(create.method, "POST");
    assert_eq!(create.path, "/graphql");
    assert_eq!(create.auth.as_deref(), Some(linear_api::KEY));
    assert_eq!(create.input()["title"], "Flaky sync test");
    assert_eq!(create.input()["teamId"], "ENG");
    assert_eq!(create.input()["assigneeId"], "me");

    let runs = env.runs();
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0]["status"], "done");
    assert_eq!(runs[0]["identifier"], "ENG-1");
    assert_eq!(runs[0]["url"], "https://linear.example/acme/issue/ENG-1");
    assert!(
        env.ok_json(&["queue"])["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let printed = env.rastro(&["actions"]);
    let text = String::from_utf8_lossy(&printed.stdout).into_owned();
    let line = text
        .lines()
        .find(|line| line.contains("ENG-1"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(line.contains("done"), "{line}");
    assert!(
        line.contains("https://linear.example/acme/issue/ENG-1"),
        "{line}"
    );
}

// spec: Nothing about Rastro reaches a source
#[test]
fn the_issue_create_holds_only_the_accepted_content_and_the_runs_random_id() {
    let env = Env::new();
    let home = env.home.path();
    let cites = [
        support::seed::claim_in(home, "work", "o/p#1"),
        support::seed::claim_in(home, "work", "o/p#2"),
    ];
    let description = "Seen twice in CI.\n\nRepro: run `sync` twice — “flaky”, not “slow”.";
    let why = "owner asked in session after the red build";
    let item = env.propose(
        "Flaky sync test",
        &[
            "--description",
            description,
            "--why",
            why,
            "--cite",
            &cites[0],
            "--cite",
            &cites[1],
        ],
    );

    env.ok_json(&["queue", "accept", &item]);

    let runs = env.runs();
    let issue_id = runs[0]["issue_id"].as_str().unwrap();
    let creates = env.linear.creates();
    assert_eq!(creates.len(), 1, "{creates:?}");
    let body = &creates[0].body;

    // The request is one GraphQL document with one input.
    let mut parts: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    parts.sort_unstable();
    assert_eq!(parts, ["query", "variables"]);
    assert_eq!(body["variables"].as_object().unwrap().len(), 1, "{body}");

    // The input is the accepted content, each field unchanged, plus the run's random id: a v4 UUID.
    let input = creates[0].input().as_object().unwrap();
    let mut fields: Vec<&str> = input.keys().map(String::as_str).collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        ["assigneeId", "description", "id", "teamId", "title"]
    );
    assert_eq!(input["title"], "Flaky sync test");
    assert_eq!(input["description"], description);
    assert_eq!(input["teamId"], "ENG");
    assert_eq!(input["assigneeId"], "me");
    assert_eq!(input["id"], issue_id);
    let uuid = issue_id.as_bytes();
    assert_eq!(uuid.len(), 36, "{issue_id}");
    assert!(
        uuid[14] == b'4' && b"89ab".contains(&uuid[19]),
        "not a v4 UUID: {issue_id}"
    );

    // Nothing else of Rastro's is in it: not the why, a cited claim, the item, the run, or its name.
    let sent = body.to_string().to_lowercase();
    let run = runs[0]["id"].as_str().unwrap();
    for secret in [why, &cites[0], &cites[1], &item, run, "rastro"] {
        assert!(
            !sent.contains(&secret.to_lowercase()),
            "{secret} reached Linear: {sent}"
        );
    }
}
