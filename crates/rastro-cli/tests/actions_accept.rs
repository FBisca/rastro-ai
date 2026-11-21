//! Accepting an action proposal creates its issue in Linear, once, with exactly the content the
//! owner accepted. The provider is the real `rastro-action-linear` plugin, and Linear is a fake
//! GraphQL server on loopback that records every request, so a test sees what reached it.
mod support;
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::{Command, Output},
};
use support::linear_api::{self, Fixture};
use tempfile::TempDir;

struct Env {
    home: TempDir,
    linear: Fixture,
    /// Holds the stand-in `security` command that `rastro` finds first on `PATH`.
    tools: TempDir,
    /// The `LINEAR_API_KEY` the commands run with, when there is one.
    env_key: Option<&'static str>,
}

impl Env {
    /// A home with the Linear action provider installed, the Linear source `linear-work` in scope
    /// `work` and `[actions.linear] default_team = "ENG"`, and a fake Linear to write to.
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
            linear: Fixture::start(),
            tools: tempfile::tempdir().unwrap(),
            env_key: Some(linear_api::KEY),
        };
        env.keychain(None);
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

    /// The `security` command on `PATH`: it answers `key` for the item `rastro-linear`, or fails
    /// as a keychain without the item does. No test reads the real keychain.
    fn keychain(&self, key: Option<&str>) {
        let script = match key {
            Some(key) => format!(
                "#!/bin/sh\necho asked >> \"$(dirname \"$0\")/asked\"\nfor a in \"$@\"; do [ \"$a\" = rastro-linear ] && echo '{key}' && exit 0; done\nexit 44\n"
            ),
            None => "#!/bin/sh\necho asked >> \"$(dirname \"$0\")/asked\"\necho 'security: The specified item could not be found in the keychain.' >&2\nexit 44\n".into(),
        };
        let security = self.tools.path().join("security");
        fs::write(&security, script).unwrap();
        fs::set_permissions(&security, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn rastro(&self, args: &[&str]) -> Output {
        let path = format!(
            "{}:{}",
            self.tools.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut command = Command::new(support::rastro());
        command
            .env("RASTRO_HOME", self.home.path())
            .env("PATH", path)
            .env("RASTRO_LINEAR_API_URL", &self.linear.base)
            .env_remove("LINEAR_API_KEY")
            .args(args);
        if let Some(key) = self.env_key {
            command.env("LINEAR_API_KEY", key);
        }
        command.output().unwrap()
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
        ["IssueLookup", "Teams", "Viewer", "IssueCreate"],
        "the id is looked up first, the ids it names just before the create"
    );
    let create = &creates[0];
    assert_eq!(create.method, "POST");
    assert_eq!(create.path, "/graphql");
    assert_eq!(create.auth.as_deref(), Some(linear_api::KEY));
    assert_eq!(create.input()["title"], "Flaky sync test");
    assert_eq!(create.input()["teamId"], linear_api::TEAM_ENG.id);
    assert_eq!(create.input()["assigneeId"], linear_api::OWNER.id);

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
    let block = text
        .split("\n\n")
        .find(|block| block.starts_with("Run "))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(block.lines().any(|l| l == "  Status: done"), "{block}");
    assert!(
        block
            .lines()
            .any(|l| l == "  Issue: ENG-1 https://linear.example/acme/issue/ENG-1"),
        "{block}"
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
    assert_eq!(input["teamId"], linear_api::TEAM_ENG.id);
    assert_eq!(input["assigneeId"], linear_api::OWNER.id);
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

// spec: A real issue goes through Rastro
/// The switch day in one sitting: an agent proposes the owner's next issue, the owner accepts it,
/// and Linear holds exactly the accepted title and description, under the team's and the owner's
/// own ids.
#[test]
fn an_agents_proposal_the_owner_accepts_becomes_the_issue_in_linear_as_accepted() {
    let env = Env::new();
    let item = env.propose(
        "Retry the stale sync",
        &[
            "--description",
            "Seen twice in CI.",
            "--why",
            "owner asked in session",
        ],
    );

    env.ok_json(&["queue", "accept", &item]);

    let creates = env.linear.creates();
    assert_eq!(creates.len(), 1, "{creates:?}");
    let input = creates[0].input();
    assert_eq!(input["title"], "Retry the stale sync");
    assert_eq!(input["description"], "Seen twice in CI.");
    assert_eq!(input["teamId"], linear_api::TEAM_ENG.id);
    assert_eq!(input["assigneeId"], linear_api::OWNER.id);
    let runs = env.runs();
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0]["status"], "done");
    assert_eq!(runs[0]["identifier"], "ENG-1");
}

const KEYCHAIN_KEY: &str = "lin_api_from_the_keychain";

/// Everything a command printed, stdout and stderr.
fn printed(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

// spec: Linear writer shares the connector credential
#[test]
fn the_writer_authenticates_with_the_keychain_item_when_the_environment_has_no_key() {
    let mut env = Env::new();
    env.env_key = None;
    env.keychain(Some(KEYCHAIN_KEY));
    let item = env.propose("Keychain write", &["--why", "owner asked in session"]);

    let out = env.rastro(&["queue", "accept", &item, "--json"]);

    assert!(out.status.success(), "{}", printed(&out));
    let creates = env.linear.creates();
    assert_eq!(creates.len(), 1, "{creates:?}");
    assert_eq!(creates[0].auth.as_deref(), Some(KEYCHAIN_KEY));
    assert!(!printed(&out).contains(KEYCHAIN_KEY));
    let runs = env.runs();
    assert_eq!(runs[0]["status"], "done", "{runs:?}");
}

// spec: Linear writer shares the connector credential
#[test]
fn the_writer_prefers_the_environment_key_to_the_keychain_item() {
    let env = Env::new();
    env.keychain(Some(KEYCHAIN_KEY));
    let item = env.propose("Environment write", &["--why", "owner asked in session"]);

    env.ok_json(&["queue", "accept", &item]);

    let creates = env.linear.creates();
    assert_eq!(creates.len(), 1, "{creates:?}");
    assert_eq!(creates[0].auth.as_deref(), Some(linear_api::KEY));
    assert!(
        !env.tools.path().join("asked").exists(),
        "the keychain was read although the environment had a key"
    );
}

// spec: Linear writer shares the connector credential
#[test]
fn the_writer_reports_an_authentication_failure_without_a_credential_and_exposes_no_secret() {
    let mut env = Env::new();
    env.env_key = None;
    let item = env.propose("No credential", &["--why", "owner asked in session"]);

    let out = env.rastro(&["queue", "accept", &item, "--json"]);
    let runs = env.runs();

    assert!(env.linear.creates().is_empty());
    let shown = format!("{}{runs:?}", printed(&out)).to_lowercase();
    // The owner is told authentication failed, in the writer's words, not merely where it looked.
    assert!(
        shown.contains("authentication failed"),
        "no authentication failure is reported: {shown}"
    );
    assert!(
        shown.contains("linear_api_key") && shown.contains("keychain"),
        "{shown}"
    );
    assert!(!shown.contains(&linear_api::KEY.to_lowercase()), "{shown}");
    assert!(!shown.contains(KEYCHAIN_KEY), "{shown}");
    assert_ne!(runs[0]["status"], "done", "{runs:?}");

    // The failure reads where the owner reads their runs.
    let listing = env.rastro(&["actions"]);
    let text = String::from_utf8_lossy(&listing.stdout).into_owned();
    let line = text
        .lines()
        .find(|line| line.contains("Error:"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        line.to_lowercase().contains("authentication failed"),
        "{line}"
    );
}
