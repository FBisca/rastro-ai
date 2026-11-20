//! The full `rules check`: each rule's match and grace, its daily cap and today's count, its
//! pending Acts and the queued proposals it matches. The provider is the scripted fake, so a
//! test sees that a check sends nothing; the clock is `RASTRO_NOW`.
mod support;
use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;

/// When the proposals are made: the rule's grace of 10 minutes ends at 10:10.
const PROPOSED_AT: &str = "2026-11-19T10:00:00Z";
/// While the grace still runs: the check reads the pending Act before it is due.
const READ_AT: &str = "2026-11-19T10:04:00Z";
/// The instant the grace ends: a tick here executes a due Act or returns it.
const DUE: &str = "2026-11-19T10:10:00Z";
/// After the grace ended: the check reads runs a tick has finished.
const AFTER: &str = "2026-11-19T10:11:00Z";

/// The Linear source every proposal writes to, in scope `work`.
const SOURCE: &str = "[[sources]]\nname = \"linear-work\"\nconnector = \"linear\"\nselector = {}\n\
                      scope = \"work\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
                      keep_original = false\n";

struct Env {
    home: TempDir,
}

impl Env {
    /// A home with the scripted fake provider and the Linear source, and no rule yet.
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
        };
        support::provider::install(&env.config_dir(), |_| {});
        fs::write(env.config_dir().join("config.toml"), SOURCE).unwrap();
        env
    }

    fn config_dir(&self) -> std::path::PathBuf {
        self.home.path().join("config")
    }

    /// Writes the rule `my-issues` on team `H` beside the source, with the daily cap when given.
    fn set_rule(&self, cap: Option<u32>) {
        let cap = cap
            .map(|cap| format!("daily_cap = {cap}\n"))
            .unwrap_or_default();
        fs::write(
            self.config_dir().join("config.toml"),
            format!(
                "{SOURCE}\n[[rules]]\nname = \"my-issues\"\naction = \"linear.create_issue\"\n\
                 team = \"H\"\nassignee = \"me\"\n{cap}"
            ),
        )
        .unwrap();
    }

    /// Removes the rule, leaving only the source.
    fn remove_rule(&self) {
        fs::write(self.config_dir().join("config.toml"), SOURCE).unwrap();
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

    /// `rastro args`, with the clock at `now`.
    fn rastro(&self, args: &[&str], now: &str) -> Output {
        Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_NOW", now)
            .args(args)
            .output()
            .unwrap()
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

    /// Proposes an issue titled `title` for `team` and returns the answer.
    fn propose(&self, title: &str, team: &str, now: &str) -> Value {
        self.ok_json(
            &[
                "actions",
                "propose",
                "linear.create_issue",
                "--title",
                title,
                "--team",
                team,
                "--why",
                "owner asked",
            ],
            now,
        )
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// spec: The owner checks what each rule does now
#[test]
fn rules_check_shows_todays_count_against_the_cap_and_lists_the_pending_act() {
    let env = Env::new();
    env.set_rule(Some(5));

    // Two Act runs start today; the owner stops the first. A stopped run still used the budget.
    let first = env.propose("First", "H", PROPOSED_AT);
    let second = env.propose("Second", "H", PROPOSED_AT);
    assert_eq!(first["run"]["status"], "pending", "{first}");
    assert_eq!(second["run"]["status"], "pending", "{second}");
    let stopped = first["run"]["id"].as_str().unwrap();
    let pending = second["run"]["id"].as_str().unwrap().to_string();
    let out = env.rastro(&["actions", "stop", stopped], PROPOSED_AT);
    assert!(out.status.success(), "{}", stdout(&out));

    // The machine read: 2 of the cap of 5 used today, and the one pending run with its due time
    // and stop command. Both proposals already have a run, so none waits for the rule to match.
    let check = env.ok_json(&["rules", "check"], READ_AT);
    let rule = &check["rules"][0];
    assert_eq!(rule["name"], "my-issues", "{check}");
    assert_eq!(rule["daily_cap"], 5, "{check}");
    assert_eq!(rule["used_today"], 2, "{check}");
    assert_eq!(rule["pending"].as_array().unwrap().len(), 1, "{check}");
    assert_eq!(rule["pending"][0]["id"], pending.as_str(), "{check}");
    assert_eq!(rule["pending"][0]["status"], "pending", "{check}");
    assert_eq!(
        rule["pending"][0]["due_at"], "2026-11-19T10:10:00Z",
        "{check}"
    );
    assert_eq!(
        rule["pending"][0]["stop_command"],
        format!("rastro actions stop {pending}"),
        "{check}"
    );
    assert_eq!(rule["matches"], json!([]), "{check}");

    // The owner's read says the same: the count against the cap, and the pending run listed.
    let text = stdout(&env.rastro(&["rules", "check"], READ_AT));
    assert!(text.contains("2 of 5 used today"), "{text}");
    let line = text
        .lines()
        .find(|line| line.contains(&pending))
        .unwrap_or_else(|| panic!("the pending run is not listed: {text}"));
    assert!(line.contains("pending"), "{line}");
    assert!(line.contains("10:10"), "{line}");
    assert!(
        line.contains(&format!("rastro actions stop {pending}")),
        "{line}"
    );

    // A check sends nothing: the fake provider was never called.
    assert_eq!(support::provider::calls(&env.config_dir()), "");
}

// spec: The owner checks what each rule does now
#[test]
fn rules_check_lists_the_queued_proposals_a_rule_matches() {
    let env = Env::new();

    // A proposal for team H is queued before the rule exists, so it waits with no run; a
    // proposal for another team waits too, and the rule will not match it.
    let early = env.propose("Before the rule", "H", PROPOSED_AT);
    assert!(early.get("run").is_none(), "{early}");
    let other = env.propose("Another team's issue", "ENG", PROPOSED_AT);
    assert!(other.get("run").is_none(), "{other}");

    // The owner writes the rule. The check lists the queued proposal it matches under the rule.
    env.set_rule(Some(5));
    let check = env.ok_json(&["rules", "check"], READ_AT);
    let rule = &check["rules"][0];
    assert_eq!(rule["name"], "my-issues", "{check}");
    assert_eq!(rule["used_today"], 0, "{check}");
    assert_eq!(rule["pending"], json!([]), "{check}");
    assert_eq!(rule["matches"].as_array().unwrap().len(), 1, "{check}");
    assert_eq!(rule["matches"][0]["id"], early["id"], "{check}");
    assert_eq!(
        rule["matches"][0]["proposal"]["fields"]["title"], "Before the rule",
        "{check}"
    );

    // The owner's read names the proposal under the rule.
    let text = stdout(&env.rastro(&["rules", "check"], READ_AT));
    let line = text
        .lines()
        .find(|line| line.contains(early["id"].as_str().unwrap()))
        .unwrap_or_else(|| panic!("the matching proposal is not listed: {text}"));
    assert!(line.contains("Before the rule"), "{line}");
    assert!(!text.contains(other["id"].as_str().unwrap()), "{text}");

    // Listing the match starts nothing: the proposal still waits, and nothing was sent.
    let runs = env.ok_json(&["actions"], READ_AT)["runs"].clone();
    assert_eq!(runs, json!([]), "{runs}");
    assert_eq!(support::provider::calls(&env.config_dir()), "");
}

// spec: The owner checks what each rule does now
#[test]
fn rules_check_lists_queued_proposals_whose_acts_returned_or_were_undone() {
    let env = Env::new();
    env.set_rule(Some(5));
    // The source was checked just now, so a tick finds it fresh and only runs actions.
    support::seed::checked_now(env.home.path(), "linear-work", "work");

    // One Act is returned: the owner deletes the rule before its grace ends, so the tick's
    // re-check fails and sends nothing. Its proposal waits in the queue as a normal proposal.
    let returned = env.propose("Returned issue", "H", PROPOSED_AT);
    let returned_run = returned["run"]["id"].as_str().unwrap().to_string();
    env.remove_rule();
    env.ok(&["tick"], DUE);
    env.set_rule(Some(5));

    // Another Act runs to done and is undone: its proposal also waits in the queue, since an
    // undo records no accept and an Act never had one.
    let undone = env.propose("Undone issue", "H", PROPOSED_AT);
    let undone_run = undone["run"]["id"].as_str().unwrap().to_string();
    env.ok(&["tick"], DUE);
    env.ok(&["actions", "undo", &undone_run], DUE);

    // The setup holds: one run returned, one undone, neither pending.
    let runs = env.ok_json(&["actions"], AFTER)["runs"].clone();
    let status = |id: &str| {
        runs.as_array()
            .unwrap()
            .iter()
            .find(|run| run["id"] == id)
            .unwrap_or_else(|| panic!("no run {id}: {runs}"))["status"]
            .clone()
    };
    assert_eq!(status(&returned_run), "returned", "{runs}");
    assert_eq!(status(&undone_run), "undone", "{runs}");

    // The queue holds both proposals, and the rule matches both: the check lists them under the
    // rule, not nothing. Neither is pending, and both used today's budget.
    let items = env.ok_json(&["queue"], AFTER)["items"]
        .as_array()
        .unwrap()
        .clone();
    for proposal in [&returned["id"], &undone["id"]] {
        assert!(
            items.iter().any(|item| item["id"] == *proposal),
            "the queue dropped {proposal}: {items:?}"
        );
    }
    let check = env.ok_json(&["rules", "check"], AFTER);
    let rule = &check["rules"][0];
    assert_eq!(rule["used_today"], 2, "{check}");
    assert_eq!(rule["pending"], json!([]), "{check}");
    let matched: Vec<&str> = rule["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|filed| filed["id"].as_str().unwrap())
        .collect();
    assert_eq!(matched.len(), 2, "{check}");
    assert!(
        matched.contains(&returned["id"].as_str().unwrap()),
        "{check}"
    );
    assert!(matched.contains(&undone["id"].as_str().unwrap()), "{check}");

    // The owner's read names both proposals under the rule.
    let text = stdout(&env.rastro(&["rules", "check"], AFTER));
    for id in [
        returned["id"].as_str().unwrap(),
        undone["id"].as_str().unwrap(),
    ] {
        assert!(text.contains(id), "{id} missing from: {text}");
    }
}
