//! An Ask cell's match becomes a queue question the owner answers in one step: yes runs the action
//! exactly once as an accepted proposal would, no rejects it and runs nothing. The cell is driven
//! to Ask through the real propose/undo boundary (two undos in its last twenty outcomes); the done
//! runs are seeded with the store's own result record so the fake Linear server receives no create
//! until the owner answers yes. The provider is the real `rastro-action-linear` plugin and Linear
//! is a fake GraphQL server on loopback that records every request, so a test sees exactly what
//! reached it. The clock is `RASTRO_NOW`.
mod support;
use rastro_core::{Storage, actions::RunResult};
use rastro_storage_sqlite::SqliteStorage;
use serde_json::Value;
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
use support::linear_api::{self, Fixture};
use tempfile::TempDir;

/// The time a command runs at when the test names none: the day after the two seeded runs.
const NOW: &str = "2026-11-03T10:00:00Z";

struct Env {
    home: TempDir,
    linear: Fixture,
    /// Holds the stand-in `security` command that `rastro` finds first on `PATH`.
    tools: TempDir,
}

impl Env {
    /// A home with the real Linear action provider, the Linear source `linear-work` in scope
    /// `work`, `[actions.linear] default_team = "ENG"`, and a fake Linear to write to.
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
            linear: Fixture::start(),
            tools: tempfile::tempdir().unwrap(),
        };
        env.keychain();
        let config = env.config_dir();
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

    fn config_dir(&self) -> std::path::PathBuf {
        self.home.path().join("config")
    }

    /// The `security` command on `PATH`: it has no keychain item, so the provider reads the key
    /// from `LINEAR_API_KEY`. No test reads the real keychain.
    fn keychain(&self) {
        let security = self.tools.path().join("security");
        fs::write(
            &security,
            "#!/bin/sh\necho 'security: The specified item could not be found in the keychain.' >&2\nexit 44\n",
        )
        .unwrap();
        fs::set_permissions(&security, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// The owner's rule on team `ENG`, appended to the config: the cell's baseline is Act.
    fn add_rule(&self) {
        let config = self.config_dir().join("config.toml");
        let text = fs::read_to_string(&config).unwrap();
        fs::write(
            &config,
            format!(
                "{text}\n[[rules]]\nname = \"my-issues\"\naction = \"linear.create_issue\"\n\
                 team = \"ENG\"\nassignee = \"me\"\n"
            ),
        )
        .unwrap();
    }

    fn rastro_at(&self, now: &str, args: &[&str]) -> std::process::Output {
        let path = format!(
            "{}:{}",
            self.tools.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_NOW", now)
            .env("PATH", path)
            .env("RASTRO_LINEAR_API_URL", &self.linear.base)
            .env("LINEAR_API_KEY", linear_api::KEY)
            .args(args)
            .output()
            .unwrap()
    }

    /// The `result` of a command that must succeed, read with `--json` at `now`.
    fn ok_json_at(&self, now: &str, args: &[&str]) -> Value {
        let out = self.rastro_at(now, &[args, &["--json"]].concat());
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"].clone()
    }

    fn db(&self) -> SqliteStorage {
        SqliteStorage::open(&self.home.path().join("data/rastro.sqlite3")).unwrap()
    }

    /// A proposal at `now` on team `ENG`; the whole answer, so a test can read whether a rule
    /// started an Act or the proposal waits in the queue.
    fn propose(&self, title: &str, now: &str) -> Value {
        self.ok_json_at(
            now,
            &[
                "actions",
                "propose",
                "linear.create_issue",
                "--title",
                title,
                "--team",
                "ENG",
                "--why",
                "owner asked",
            ],
        )
    }

    /// A rule-started Act at `now`, finished through the store's own result record so the fake
    /// Linear receives no create: executing it is not what this test exercises. Returns its id.
    fn act_run(&self, title: &str, now: &str) -> String {
        let proposed = self.propose(title, now);
        assert_eq!(proposed["run"]["status"], "pending", "{proposed}");
        let id = proposed["run"]["id"].as_str().unwrap().to_string();
        self.db()
            .record_action_result(
                &id,
                &RunResult {
                    identifier: format!("ENG-{title}"),
                    url: format!("https://fake.invalid/ENG-{title}"),
                    finished_at: now.parse().unwrap(),
                },
            )
            .unwrap();
        id
    }

    /// The owner undoes a done run at `now`; the provider finds the seeded issue absent and the
    /// undo intent is recorded, demoting the cell.
    fn undo(&self, run: &str, now: &str) {
        let undone = self.ok_json_at(now, &["actions", "undo", run]);
        assert_eq!(undone["status"], "undone", "{undone}");
    }

    /// Drives the one cell to Ask: a rule, two done runs, then both undone — two undos in its last
    /// twenty outcomes. No create reaches the fake Linear.
    fn seed_ask(&self) {
        self.add_rule();
        let first = self.act_run("Issue 1", "2026-11-01T10:00:00Z");
        let second = self.act_run("Issue 2", "2026-11-02T10:00:00Z");
        self.undo(&first, "2026-11-01T12:00:00Z");
        self.undo(&second, "2026-11-02T12:00:00Z");
        assert_eq!(self.cell(NOW)["level"], "ask", "the cell must sit at Ask");
        assert!(
            self.linear.creates().is_empty(),
            "seeding the Ask state reached Linear with a create"
        );
    }

    /// The one cell the trust view holds, as JSON.
    fn cell(&self, now: &str) -> Value {
        let answer = self.ok_json_at(now, &["trust"]);
        let cells = answer["cells"].as_array().unwrap();
        assert_eq!(cells.len(), 1, "{answer}");
        cells[0].clone()
    }

    /// The queue's items at `now`, as JSON.
    fn items(&self, now: &str) -> Vec<Value> {
        self.ok_json_at(now, &["queue"])["items"]
            .as_array()
            .unwrap()
            .clone()
    }

    /// The item the queue holds under `id`, if any.
    fn item(&self, now: &str, id: &str) -> Option<Value> {
        self.items(now).into_iter().find(|item| item["id"] == id)
    }

    /// The runs `rastro actions` lists at `now`.
    fn runs(&self, now: &str) -> Vec<Value> {
        self.ok_json_at(now, &["actions"])["runs"]
            .as_array()
            .unwrap()
            .clone()
    }
}

// spec: An Ask cell asks instead of proposing
#[test]
fn answering_an_ask_question_yes_runs_the_action_once() {
    let env = Env::new();
    env.seed_ask();
    let proposed = env.propose("Ask issue", NOW);
    assert!(
        proposed["run"].is_null(),
        "an Ask match starts no run: {proposed}"
    );
    let item = proposed["id"].as_str().unwrap().to_string();
    let question = env
        .item(NOW, &item)
        .unwrap_or_else(|| panic!("the queue holds no question for {item}"));
    assert_eq!(question["kind"], "question", "{question}");

    env.ok_json_at(NOW, &["queue", "answer", &item, "yes"]);

    // The action ran exactly once as an accepted proposal would: one create reached Linear.
    let creates = env.linear.creates();
    assert_eq!(creates.len(), 1, "{creates:?}");
    assert_eq!(creates[0].input()["title"], "Ask issue");
    assert_eq!(creates[0].input()["teamId"], linear_api::TEAM_ENG.id);
    assert_eq!(creates[0].input()["assigneeId"], linear_api::OWNER.id);
    // One run, done, started from the question's proposal at the propose level.
    let runs: Vec<_> = env
        .runs(NOW)
        .into_iter()
        .filter(|run| run["proposal"] == item)
        .collect();
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0]["status"], "done", "{runs:?}");
    assert_eq!(runs[0]["level"], "propose", "{runs:?}");
    assert_eq!(runs[0]["identifier"], "ENG-1", "{runs:?}");
    // The answered question leaves the queue.
    assert!(
        env.item(NOW, &item).is_none(),
        "the answered question waits on"
    );
}

// spec: An Ask cell asks instead of proposing
#[test]
fn answering_an_ask_question_no_rejects_it() {
    let env = Env::new();
    env.seed_ask();
    let proposed = env.propose("Ask issue", NOW);
    assert!(
        proposed["run"].is_null(),
        "an Ask match starts no run: {proposed}"
    );
    let item = proposed["id"].as_str().unwrap().to_string();
    let question = env
        .item(NOW, &item)
        .unwrap_or_else(|| panic!("the queue holds no question for {item}"));
    assert_eq!(question["kind"], "question", "{question}");

    env.ok_json_at(NOW, &["queue", "answer", &item, "no"]);

    // Nothing ran: no create reached Linear and no run started from the question.
    assert!(
        env.linear.creates().is_empty(),
        "a rejection reached Linear"
    );
    assert!(
        env.runs(NOW).iter().all(|run| run["proposal"] != item),
        "a rejection started a run"
    );
    // The question's outcome reads rejected.
    let outcomes = env.ok_json_at(NOW, &["queue", "outcomes"])["outcomes"]
        .as_array()
        .unwrap()
        .clone();
    let outcome = outcomes
        .iter()
        .find(|outcome| outcome["item"] == item)
        .unwrap_or_else(|| panic!("no outcome for {item}: {outcomes:?}"));
    assert_eq!(outcome["kind"], "rejected", "{outcome}");
    // The rejected question leaves the queue.
    assert!(
        env.item(NOW, &item).is_none(),
        "the rejected question waits on"
    );
}
