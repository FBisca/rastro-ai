//! A rule's optional daily cap: at most `daily_cap` Act runs per local calendar day, counting
//! every run the rule started, stopped ones included. Past the cap a matching proposal waits in
//! the queue as a normal proposal, and the propose answer names the rule and its cap; the next
//! local day the cap resets and a matching proposal starts a pending Act again. The clock is
//! `RASTRO_NOW`; the provider is the scripted fake, so a test sees that nothing was executed.
mod support;
use chrono::{Local, TimeZone, Utc};
use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;

struct Env {
    home: TempDir,
}

impl Env {
    /// A home with the scripted fake provider, the Linear source `linear-work` in scope `work`
    /// and the rule `my-issues` on team `H` with the daily cap `cap`.
    fn new(cap: u32) -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
        };
        support::provider::install(&env.config_dir(), |_| {});
        fs::write(
            env.config_dir().join("config.toml"),
            format!(
                "[[sources]]\nname = \"linear-work\"\nconnector = \"linear\"\nselector = {{}}\n\
                 scope = \"work\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
                 keep_original = false\n\n\
                 [[rules]]\nname = \"my-issues\"\naction = \"linear.create_issue\"\nteam = \"H\"\n\
                 assignee = \"me\"\ndaily_cap = {cap}\n"
            ),
        )
        .unwrap();
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

    /// Proposes an issue titled `title` for team `H` and returns the answer.
    fn propose(&self, title: &str, now: &str) -> Value {
        self.ok_json(&propose(title), now)
    }

    /// The runs `rastro actions` lists.
    fn runs(&self, now: &str) -> Value {
        self.ok_json(&["actions"], now)["runs"].clone()
    }

    /// The queue's items, as a machine reads them.
    fn queue_items(&self, now: &str) -> Vec<Value> {
        self.ok_json(&["queue"], now)["items"]
            .as_array()
            .unwrap()
            .clone()
    }
}

/// The arguments of a `linear.create_issue` proposal titled `title` for team `H`.
fn propose(title: &str) -> Vec<&str> {
    vec![
        "actions",
        "propose",
        "linear.create_issue",
        "--title",
        title,
        "--team",
        "H",
        "--why",
        "W",
    ]
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Noon of a local calendar day, as the RFC 3339 instant `RASTRO_NOW` reads: noon keeps the day
/// the same in every time zone the test runs in, whatever its offset or daylight saving.
fn local_noon(year: i32, month: u32, day: u32) -> String {
    Local
        .with_ymd_and_hms(year, month, day, 12, 0, 0)
        .unwrap()
        .with_timezone(&Utc)
        .to_rfc3339()
}

// spec: A rule's daily cap
#[test]
fn past_the_cap_a_matching_proposal_waits_in_the_queue() {
    let env = Env::new(2);
    let now = "2026-11-19T10:00:00Z";

    // Two Act runs start today: the rule is live and its cap is 2.
    let first = env.propose("First", now);
    let second = env.propose("Second", now);
    assert_eq!(first["run"]["status"], "pending", "{first}");
    assert_eq!(second["run"]["status"], "pending", "{second}");
    let stopped = first["run"]["id"].as_str().unwrap().to_string();

    // The owner stops one. A stopped run still used the day's budget: it is not refunded.
    let out = env.rastro(&["actions", "stop", &stopped], now);
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );

    // The third matching proposal waits in the queue, and the answer names the rule and its cap.
    let out = env.rastro(&propose("Third"), now);
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    let answer = stdout(&out);
    for part in ["queue", "my-issues", "cap of 2"] {
        assert!(answer.contains(part), "{part} missing from: {answer}");
    }
    // The waiting answer is a block too: the proposal's id on the header line.
    assert!(
        answer.lines().any(|l| l.starts_with("Proposed q_")),
        "{answer}"
    );
    assert!(!answer.contains(" | "), "pipe-packed line: {answer}");
    let third = env.propose("Third", now);
    assert!(third.get("run").is_none(), "{third}");
    assert_eq!(
        third["cap"],
        json!({"rule": "my-issues", "cap": 2}),
        "{third}"
    );

    // The queue holds it as a normal action proposal, and no new run started.
    let items = env.queue_items(now);
    assert!(
        items
            .iter()
            .any(|item| item["id"] == third["id"] && item["kind"] == "action_proposal"),
        "{items:?}"
    );
    let runs = env.runs(now);
    assert_eq!(runs.as_array().unwrap().len(), 2, "{runs}");
    assert!(
        !runs
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["proposal"] == third["id"]),
        "{runs}"
    );
    assert_eq!(support::provider::calls(&env.config_dir()), "");
}

// spec: A rule's daily cap
#[test]
fn the_cap_resets_on_the_next_local_day() {
    let env = Env::new(2);
    let day_one = local_noon(2026, 11, 19);
    let day_two = local_noon(2026, 11, 20);

    // Two Act runs start on the first local day, and the cap is reached there.
    let first = env.propose("First", &day_one);
    let second = env.propose("Second", &day_one);
    assert_eq!(first["run"]["status"], "pending", "{first}");
    assert_eq!(second["run"]["status"], "pending", "{second}");
    let waited = env.propose("Third", &day_one);
    assert!(waited.get("run").is_none(), "{waited}");

    // The next local day the cap resets: a matching proposal starts a new pending Act.
    let fresh = env.propose("Fourth", &day_two);
    let run = &fresh["run"];
    assert_eq!(run["status"], "pending", "{fresh}");
    assert_eq!(run["level"], "act", "{fresh}");
    let id = run["id"].as_str().unwrap();
    assert_ne!(id, first["run"]["id"].as_str().unwrap());
    assert_ne!(id, second["run"]["id"].as_str().unwrap());

    // The answer names the new run, and the list holds the two old runs and no more.
    let answer = stdout(&env.rastro(&propose("Fourth"), &day_two));
    assert!(answer.contains(id), "{id} missing from: {answer}");
    assert_eq!(env.runs(&day_two).as_array().unwrap().len(), 3);
}
