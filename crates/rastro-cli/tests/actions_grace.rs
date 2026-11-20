//! `rastro tick` runs a pending Act once its grace ends: before that it sends nothing and the run
//! stays pending; at the due time it creates the issue exactly once, holding exactly the proposed
//! content and the run's issue id, through the same executor an accepted run uses; a later tick
//! creates nothing more. A done Act run undoes like any run. The provider is the real
//! `rastro-action-linear` plugin and Linear is a fake GraphQL server on loopback that records every
//! request; the clock is `RASTRO_NOW`, so the grace window passes without a wait.
mod support;
use serde_json::Value;
use std::{
    fs,
    process::{Command, Output},
};
use support::linear_api::{self, Fixture};
use tempfile::TempDir;

/// When the proposal is made: the rule's grace of 10 minutes ends at 10:10.
const PROPOSED_AT: &str = "2026-11-19T10:00:00Z";
/// A minute before the grace ends.
const BEFORE: &str = "2026-11-19T10:09:00Z";
/// The instant the grace ends.
const DUE: &str = "2026-11-19T10:10:00Z";
/// A minute after the grace ended.
const AFTER: &str = "2026-11-19T10:11:00Z";

const TITLE: &str = "Retry the stale sync";
const DESCRIPTION: &str = "Seen twice in CI.";

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
            "[[sources]]\nname = \"linear-work\"\nconnector = \"linear\"\nselector = {}\n\
             scope = \"work\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
             keep_original = false\n\n\
             [[rules]]\nname = \"my-issues\"\naction = \"linear.create_issue\"\nteam = \"H\"\n\
             assignee = \"me\"\n",
        )
        .unwrap();
        support::seed::checked_now(env.home.path(), "linear-work", "work");
        env
    }

    fn config_dir(&self) -> std::path::PathBuf {
        self.home.path().join("config")
    }

    /// `rastro args`, with the clock at `now`.
    fn rastro(&self, args: &[&str], now: &str) -> Output {
        Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_NOW", now)
            .env("LINEAR_API_KEY", linear_api::KEY)
            .env("RASTRO_LINEAR_API_URL", &self.linear.base)
            .args(args)
            .output()
            .unwrap()
    }

    /// A command that must succeed.
    fn ok(&self, args: &[&str], now: &str) {
        let out = self.rastro(args, now);
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
    }

    /// The `result` of a command that must succeed, read with `--json`.
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

    /// Proposes `TITLE` for team `H` at 10:00 and returns the pending Act run's id.
    fn propose(&self) -> String {
        let answer = self.ok_json(
            &[
                "actions",
                "propose",
                "linear.create_issue",
                "--title",
                TITLE,
                "--description",
                DESCRIPTION,
                "--team",
                "H",
                "--why",
                "owner asked",
            ],
            PROPOSED_AT,
        );
        let run = &answer["run"];
        assert_eq!(run["status"], "pending", "{answer}");
        run["id"].as_str().unwrap().to_string()
    }

    /// The one run `rastro actions` lists.
    fn run(&self) -> Value {
        let runs = self.ok_json(&["actions"], DUE)["runs"].clone();
        let runs = runs.as_array().unwrap();
        assert_eq!(runs.len(), 1, "{runs:?}");
        runs[0].clone()
    }
}

// spec: An Act runs when its grace ends
#[test]
fn tick_before_the_grace_ends_sends_nothing_and_the_run_stays_pending() {
    let env = Env::new();
    let id = env.propose();

    env.ok(&["tick"], BEFORE);

    assert!(
        env.linear.requests().is_empty(),
        "a tick before the grace ends reached Linear"
    );
    let run = env.run();
    assert_eq!(run["id"], id.as_str());
    assert_eq!(run["status"], "pending");
    assert_eq!(run["due_at"], "2026-11-19T10:10:00Z");

    // The grace window is the only thing holding it: a tick at 10:10 creates the issue.
    env.ok(&["tick"], DUE);
    assert_eq!(env.linear.creates().len(), 1);
    assert_eq!(env.run()["status"], "done");
}

// spec: An Act runs when its grace ends
#[test]
fn tick_at_the_grace_end_creates_the_issue_once_with_exactly_the_proposed_content() {
    let env = Env::new();
    let id = env.propose();
    let issue_id = env.run()["issue_id"].as_str().unwrap().to_string();

    env.ok(&["tick"], DUE);

    let creates = env.linear.creates();
    assert_eq!(creates.len(), 1, "{creates:?}");
    let sent: Vec<String> = (env.linear.requests().into_iter())
        .map(|req| req.operation)
        .collect();
    assert_eq!(
        sent,
        ["IssueLookup", "Teams", "Viewer", "IssueCreate"],
        "the run's id is looked up before any create, and the ids it names just before it"
    );
    // The create holds exactly the proposal's content and the run's issue id, and nothing else.
    let input = creates[0].input().as_object().unwrap();
    let mut fields: Vec<&str> = input.keys().map(String::as_str).collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        ["assigneeId", "description", "id", "teamId", "title"]
    );
    assert_eq!(input["title"], TITLE);
    assert_eq!(input["description"], DESCRIPTION);
    assert_eq!(input["teamId"], linear_api::TEAM_H.id);
    assert_eq!(input["assigneeId"], linear_api::OWNER.id);
    assert_eq!(input["id"], issue_id.as_str());

    let run = env.run();
    assert_eq!(run["id"], id.as_str());
    assert_eq!(run["status"], "done");
    assert_eq!(run["identifier"], "ENG-1");
    assert_eq!(run["url"], "https://linear.example/acme/issue/ENG-1");
}

// spec: An Act runs when its grace ends
#[test]
fn a_second_tick_after_the_act_is_done_creates_nothing_more() {
    let env = Env::new();
    env.propose();
    env.ok(&["tick"], DUE);
    assert_eq!(env.linear.creates().len(), 1);
    let received = env.linear.requests().len();

    env.ok(&["tick"], AFTER);

    assert_eq!(
        env.linear.creates().len(),
        1,
        "a done Act is never created again"
    );
    assert_eq!(
        env.linear.requests().len(),
        received,
        "nothing more reached Linear"
    );
    assert_eq!(env.run()["status"], "done");
}

// spec: An Act run can be undone
#[test]
fn undoing_a_done_act_deletes_its_issue_and_the_run_reads_undone() {
    let env = Env::new();
    let id = env.propose();
    env.ok(&["tick"], DUE);
    let run = env.run();
    assert_eq!(run["status"], "done");
    let issue_id = run["issue_id"].as_str().unwrap().to_string();

    // Only the owner touched the issue: no comment or edit of anyone else's is seeded.
    let undone = env.ok_json(&["actions", "undo", &id], AFTER);

    let deletes = env.linear.deletes();
    assert_eq!(deletes.len(), 1, "one delete for the issue");
    assert_eq!(deletes[0].body["variables"]["id"], issue_id.as_str());
    assert_eq!(env.linear.issues()[0]["trashed"], true);
    assert_eq!(undone["status"], "undone");
    assert!(
        undone["undo"]["intent"]
            .as_str()
            .is_some_and(|intent| !intent.is_empty())
    );
    assert_eq!(env.run()["status"], "undone");
}

// spec: A real issue goes through a rule
/// The switch day in one sitting: with the owner's rule on team `H`, an agent proposes the owner's
/// next issue, the grace window passes with no accept, a tick creates it, and Linear holds exactly
/// the proposed title and description; `actions show` names the rule that authorized it.
#[test]
fn an_agents_proposal_under_the_owners_rule_becomes_the_issue_once_the_grace_passes() {
    let env = Env::new();
    let id = env.propose();

    env.ok(&["tick"], BEFORE);
    assert!(
        env.linear.creates().is_empty(),
        "sent before the grace ended"
    );

    env.ok(&["tick"], DUE);

    let creates = env.linear.creates();
    assert_eq!(creates.len(), 1, "{creates:?}");
    let input = creates[0].input();
    assert_eq!(input["title"], TITLE);
    assert_eq!(input["description"], DESCRIPTION);
    assert_eq!(input["teamId"], linear_api::TEAM_H.id);
    assert_eq!(input["assigneeId"], linear_api::OWNER.id);
    let shown = env.ok_json(&["actions", "show", &id], AFTER);
    let run = &shown["runs"][0];
    assert_eq!(run["status"], "done", "{shown}");
    assert_eq!(run["level"], "act", "{shown}");
    assert!(run.get("accept_intent").is_none(), "{run}");
    assert_eq!(run["rule"]["name"], "my-issues", "{shown}");
}

/// The ids of the items `rastro queue` lists at `now`.
fn queued(env: &Env, now: &str) -> Vec<String> {
    (env.ok_json(&["queue"], now)["items"]
        .as_array()
        .unwrap()
        .iter())
    .map(|item| item["id"].as_str().unwrap().to_string())
    .collect()
}

/// The proposal a run started on.
fn proposal_of(env: &Env, run: &str) -> String {
    let shown = env.ok_json(&["actions", "show", run], AFTER);
    shown["runs"][0]["proposal"].as_str().unwrap().to_string()
}

// spec: A proposal a rule acted on leaves the queue
#[test]
fn a_done_acts_proposal_leaves_the_queue_and_cannot_be_accepted() {
    let env = Env::new();
    let id = env.propose();
    let proposal = proposal_of(&env, &id);
    env.ok(&["tick"], DUE);
    assert_eq!(env.run()["status"], "done");

    assert!(
        !queued(&env, AFTER).contains(&proposal),
        "{}",
        env.ok_json(&["queue"], AFTER)
    );
    let accept = env.rastro(&["queue", "accept", &proposal], AFTER);
    assert!(
        !accept.status.success(),
        "{}",
        String::from_utf8_lossy(&accept.stdout)
    );
    assert_eq!(env.linear.creates().len(), 1, "a second create was sent");
    let runs = env.ok_json(&["actions"], AFTER)["runs"].clone();
    assert_eq!(runs.as_array().unwrap().len(), 1, "{runs}");
}

// spec: A proposal a rule acted on leaves the queue
#[test]
fn an_undone_acts_proposal_does_not_return_to_the_queue() {
    let env = Env::new();
    let id = env.propose();
    let proposal = proposal_of(&env, &id);
    env.ok(&["tick"], DUE);
    env.ok(&["actions", "undo", &id], AFTER);
    assert_eq!(env.run()["status"], "undone");

    assert!(
        !queued(&env, AFTER).contains(&proposal),
        "{}",
        env.ok_json(&["queue"], AFTER)
    );
}

// spec: A proposal a rule acted on leaves the queue
#[test]
fn a_failed_acts_proposal_is_neither_queued_nor_matched_by_its_rule() {
    let env = Env::new();
    // A rule and a proposal on a team the fake Linear doesn't hold: the Act fails at its grace end.
    let config = env.config_dir().join("config.toml");
    let text = fs::read_to_string(&config).unwrap();
    fs::write(&config, text.replace("team = \"H\"", "team = \"ZZ\"")).unwrap();
    let answer = env.ok_json(
        &[
            "actions",
            "propose",
            "linear.create_issue",
            "--title",
            TITLE,
            "--team",
            "ZZ",
            "--why",
            "owner asked",
        ],
        PROPOSED_AT,
    );
    let id = answer["run"]["id"].as_str().unwrap().to_string();
    let proposal = proposal_of(&env, &id);
    // Tick reports the failed run and exits non-zero.
    assert!(!env.rastro(&["tick"], DUE).status.success());
    assert_eq!(env.run()["status"], "failed");

    assert!(
        !queued(&env, AFTER).contains(&proposal),
        "{}",
        env.ok_json(&["queue"], AFTER)
    );
    let check = env.ok_json(&["rules", "check"], AFTER);
    assert!(
        check["rules"][0]["matches"].as_array().unwrap().is_empty(),
        "{check}"
    );
}
