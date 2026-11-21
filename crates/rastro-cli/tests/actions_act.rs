//! A proposal an owner rule matches starts an Act: a pending run stored with the rule as it was
//! when the run started, due when its grace ends, with no accept intent and nothing sent. The
//! propose answer names the run, when it runs and its stop command; a proposal for another team
//! or flagged as a possible duplicate waits in the queue instead. `actions show` explains an Act
//! run from what was stored, so a later config change never revises it. The clock is `RASTRO_NOW`;
//! the matching proposal is driven against the real Linear provider and a fake Linear server, so
//! a test sees that nothing reached it. A done Act run is seeded through the store's own result
//! record: executing an Act is the tick's work, not this read's.
mod support;
use chrono::{DateTime, Utc};
use rastro_core::{Appended, Evidence, Scope, Source, Storage, TrustClass, actions::RunResult};
use rastro_storage_sqlite::SqliteStorage;
use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Output},
};
use support::linear_api::{self, Fixture};
use tempfile::TempDir;

/// The time every command runs at: the grace of a rule with no `grace` of its own ends at 10:10.
const NOW: &str = "2026-11-19T10:00:00Z";

struct Env {
    home: TempDir,
    linear: Option<Fixture>,
}

impl Env {
    /// A home with the scripted fake provider, the Linear source `linear-work` in scope `work`
    /// and the rule `my-issues` on `team`. Nothing here talks to Linear.
    fn fake(team: &str) -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
            linear: None,
        };
        support::provider::install(&env.config_dir(), |_| {});
        env.write_config(team);
        env
    }

    /// A home with the real Linear provider and a fake Linear to write to, the same source and
    /// rule, so a test sees every request the proposal would send.
    fn with_linear(team: &str) -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
            linear: Some(Fixture::start()),
        };
        support::provider::install_linear(&env.config_dir(), support::action_linear());
        env.write_config(team);
        env
    }

    fn config_dir(&self) -> std::path::PathBuf {
        self.home.path().join("config")
    }

    fn write_config(&self, team: &str) {
        fs::write(
            self.config_dir().join("config.toml"),
            format!(
                "[[sources]]\nname = \"linear-work\"\nconnector = \"linear\"\nselector = {{}}\n\
                 scope = \"work\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
                 keep_original = false\n\n\
                 [[rules]]\nname = \"my-issues\"\naction = \"linear.create_issue\"\nteam = \"{team}\"\n\
                 assignee = \"me\"\n"
            ),
        )
        .unwrap();
    }

    fn rastro(&self, args: &[&str]) -> Output {
        let mut command = Command::new(support::rastro());
        command
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_NOW", NOW);
        if let Some(linear) = &self.linear {
            command
                .env("LINEAR_API_KEY", linear_api::KEY)
                .env("RASTRO_LINEAR_API_URL", &linear.base);
        }
        command.args(args).output().unwrap()
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

    /// The runs `rastro actions` lists.
    fn runs(&self) -> Value {
        self.ok_json(&["actions"])["runs"].clone()
    }

    /// The queue's items, as a machine reads them.
    fn queue_items(&self) -> Vec<Value> {
        self.ok_json(&["queue"])["items"]
            .as_array()
            .unwrap()
            .clone()
    }

    /// An issue Linear says exists in `linear-work`, seeded as a connector run's evidence.
    fn observe_issue(&self, id: &str, identifier: &str, title: &str, state: &str) {
        let source = Source::new("linear-work".into(), "linear".into(), Scope("work".into()));
        let mut db = self.db();
        db.project_sources(std::slice::from_ref(&source)).unwrap();
        let at: DateTime<Utc> = "2026-11-18T00:00:00Z".parse().unwrap();
        let run = db
            .start_run(&source, ("fixture", "1"), at, (at, at))
            .unwrap();
        for (field, quote) in [
            ("/identifier", identifier),
            ("/title", title),
            ("/state/type", state),
        ] {
            let evidence = Evidence {
                source: source.name.clone(),
                scope: source.scope.clone(),
                location: format!("linear:issue/{id}"),
                field: field.into(),
                start: 0,
                end: quote.len() as u64,
                quote: quote.into(),
                trust_class: TrustClass::Structured,
                captured_at: at,
                source_updated_at: Some(at),
                original: None,
            };
            match db.append_evidence(run, &evidence).unwrap() {
                Appended::New(_) | Appended::Unchanged(_) => {}
            }
        }
    }

    fn db(&self) -> SqliteStorage {
        SqliteStorage::open(&self.home.path().join("data/rastro.sqlite3")).unwrap()
    }

    /// Records the issue a finished execution of the run created, as the executor itself does.
    fn finish_run(&self, run: &str, identifier: &str) {
        self.db()
            .record_action_result(
                run,
                &RunResult {
                    identifier: identifier.into(),
                    url: format!("https://linear.example/acme/issue/{identifier}"),
                    finished_at: "2026-11-19T10:10:00Z".parse().unwrap(),
                },
            )
            .unwrap();
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The arguments of a `linear.create_issue` proposal titled `title`, with `extra` after it.
fn propose<'a>(title: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "actions",
        "propose",
        "linear.create_issue",
        "--title",
        title,
    ];
    args.extend_from_slice(extra);
    args
}

// spec: A matching rule starts an Act
// spec: Human records render as blocks
#[test]
fn a_matching_proposal_starts_a_pending_act_and_sends_nothing() {
    let env = Env::with_linear("H");
    let out = env.rastro(&propose("T", &["--team", "H", "--why", "W"]));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let answer = stdout(&out);

    // The run waits: pending, at level act, due when the grace ends.
    let runs = env.runs();
    assert_eq!(runs.as_array().unwrap().len(), 1, "{runs}");
    let run = &runs[0];
    let id = run["id"].as_str().unwrap();
    assert!(id.starts_with("r_"), "{id}");
    assert_eq!(run["status"], "pending");
    assert_eq!(run["level"], "act");
    assert_eq!(run["due_at"], "2026-11-19T10:10:00Z");
    assert_eq!(run["fields"]["title"], "T");
    assert_eq!(run["fields"]["team"], "H");

    // The answer names the run, says it runs at 10:10 unless stopped, and gives the stop command.
    for part in [
        id,
        "10:10",
        "unless stopped",
        &format!("rastro actions stop {id}"),
    ] {
        assert!(answer.contains(part), "{part} missing from: {answer}");
    }

    // The answer reads as a block: the proposal's id on the header line, then one labelled
    // field per line, and no pipe-packed line.
    let proposal = run["proposal"].as_str().unwrap();
    assert!(
        answer.lines().any(|l| l == format!("Proposed {proposal}")),
        "{answer}"
    );
    for field in [
        "  Source: linear linear-work",
        "  Scope: work",
        "  Action: linear.create_issue",
        "  Title: T",
        "  Team: H",
        "  Assignee: me",
        "  Why: W",
    ] {
        assert!(
            answer.lines().any(|l| l == field),
            "{field} missing: {answer}"
        );
    }
    assert!(!answer.contains(" | "), "pipe-packed line: {answer}");

    // The owner's list says the same in words: the run as a block, its status and level on
    // their own lines.
    let listed = stdout(&env.rastro(&["actions"]));
    let block = listed
        .split("\n\n")
        .find(|block| block.starts_with(&format!("Run {id}")))
        .unwrap_or_else(|| panic!("no run block in: {listed}"));
    assert!(block.lines().any(|l| l == "  Status: pending"), "{block}");
    assert!(block.lines().any(|l| l == "  Level: act"), "{block}");

    // Nothing reached Linear: the grace window is the owner's chance to stop it.
    assert_eq!(env.linear.as_ref().unwrap().requests().len(), 0);
}

// spec: A matching rule starts an Act
#[test]
fn a_proposal_for_another_team_waits_in_the_queue() {
    let env = Env::fake("H");
    let proposed = env.ok_json(&propose("Other team", &["--team", "ENG", "--why", "W"]));
    assert!(proposed.get("run").is_none(), "{proposed}");

    let items = env.queue_items();
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["id"], proposed["id"]);
    assert_eq!(items[0]["kind"], "action_proposal");
    assert_eq!(env.runs(), json!([]));
    assert_eq!(support::provider::calls(&env.config_dir()), "");

    // The rule is live: a proposal for its own team starts an Act, so the wait above is the
    // team's doing and not a rule nothing reads.
    let matching = env.ok_json(&propose("Its own team", &["--team", "H", "--why", "W"]));
    assert_eq!(matching["run"]["status"], "pending", "{matching}");
    assert_eq!(env.runs().as_array().unwrap().len(), 1);
}

// spec: A matching rule starts an Act
#[test]
fn a_possible_duplicate_waits_in_the_queue() {
    let env = Env::fake("H");
    env.observe_issue("a1", "H-12", "T", "started");
    let proposed = env.ok_json(&propose("T", &["--team", "H", "--why", "W"]));
    assert!(proposed.get("run").is_none(), "{proposed}");

    let items = env.queue_items();
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["id"], proposed["id"]);
    assert_eq!(items[0]["possible_duplicates"], json!(["H-12"]));
    assert_eq!(env.runs(), json!([]));
    let text = stdout(&env.rastro(&["queue"]));
    assert!(text.contains("possible duplicate of H-12"), "{text}");
    assert_eq!(support::provider::calls(&env.config_dir()), "");

    // The rule is live and the title is the only difference: another title starts an Act.
    let fresh = env.ok_json(&propose("Another title", &["--team", "H", "--why", "W"]));
    assert_eq!(fresh["run"]["status"], "pending", "{fresh}");
    assert_eq!(env.runs().as_array().unwrap().len(), 1);
}

// spec: An Act run explains itself
// spec: Human records render as blocks
#[test]
fn an_act_run_shows_the_rule_it_started_under_and_no_accept() {
    let env = Env::fake("H");
    env.ok_json(&propose("T", &["--team", "H", "--why", "owner asked"]));
    let id = env.runs()[0]["id"].as_str().unwrap().to_string();
    env.finish_run(&id, "H-1");

    let shown = env.ok_json(&["actions", "show", &id]);
    let run = &shown["runs"][0];
    assert_eq!(run["id"], id.as_str());
    assert_eq!(run["status"], "done");
    assert_eq!(run["level"], "act");
    assert!(run.get("accept_intent").is_none(), "{run}");
    assert_eq!(
        run["rule"],
        json!({"name": "my-issues", "action": "linear.create_issue", "team": "H",
               "assignee": "me", "grace_seconds": 600})
    );
    assert_eq!(run["due_at"], "2026-11-19T10:10:00Z");
    let why = &shown["why"];
    assert_eq!(why["proposal"]["why"], "owner asked");
    assert_eq!(why["proposal"]["fields"]["title"], "T");
    assert_eq!(why["level"], "act");
    assert!(why.get("accept").is_none(), "{why}");

    // The owner reads the same in words: the rule that authorized it, and no accept.
    let text = stdout(&env.rastro(&["actions", "show", &id]));
    for part in [
        "Level: act",
        "my-issues",
        "grace 10 min",
        "Why: owner asked",
    ] {
        assert!(text.contains(part), "{part} missing from: {text}");
    }
    assert!(text.lines().any(|l| l == "Why: owner asked"), "{text}");
    assert!(!text.contains(" | "), "pipe-packed line: {text}");
    assert!(!text.contains("Accepted:"), "{text}");
}

// spec: An Act run explains itself
#[test]
fn the_rule_copy_survives_a_config_change() {
    let env = Env::fake("H");
    env.ok_json(&propose("T", &["--team", "H", "--why", "W"]));
    let id = env.runs()[0]["id"].as_str().unwrap().to_string();

    // The owner changes the rule's team after the run started.
    let config = env.config_dir().join("config.toml");
    let text = fs::read_to_string(&config).unwrap();
    fs::write(&config, text.replace("team = \"H\"", "team = \"ENG\"")).unwrap();
    let check = env.ok_json(&["rules", "check"]);
    assert_eq!(check["rules"][0]["team"], "ENG", "{check}");

    // The run still explains itself under the rule as it was: team H, not the config's ENG.
    let shown = env.ok_json(&["actions", "show", &id]);
    assert_eq!(shown["runs"][0]["rule"]["team"], "H", "{shown}");
    assert_eq!(shown["runs"][0]["rule"]["name"], "my-issues");
    assert_eq!(shown["runs"][0]["fields"]["team"], "H");
}
