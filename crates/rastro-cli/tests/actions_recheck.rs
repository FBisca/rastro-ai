//! Before a pending Act runs at the end of its grace, it is checked again: the rule it started under
//! must still exist and match, no claim it cites may be retracted, and no open observed issue may hold
//! its title. A failed check sends nothing, marks the run `returned`, and leaves its proposal waiting
//! in the queue; a returned run never executes again. The provider is the real `rastro-action-linear`
//! plugin and Linear is a fake GraphQL server on loopback that records every request; the clock is
//! `RASTRO_NOW`, so the grace window passes without a wait.
mod support;
use chrono::{DateTime, Utc};
use rastro_core::{Appended, Evidence, Scope, Source, Storage, TrustClass};
use rastro_storage_sqlite::SqliteStorage;
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

/// The Linear source every home registers, in scope `work`.
const SOURCE: &str = "[[sources]]\nname = \"linear-work\"\nconnector = \"linear\"\nselector = {}\n\
     scope = \"work\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
     keep_original = false\n";
/// The rule `my-issues` on team `H`, under which a matching proposal starts an Act.
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
        env.set_rules(RULE);
        support::seed::checked_now(env.home.path(), "linear-work", "work");
        env
    }

    fn config_dir(&self) -> std::path::PathBuf {
        self.home.path().join("config")
    }

    /// Writes `config.toml` as the Linear source followed by `rules` (empty: no rule).
    fn set_rules(&self, rules: &str) {
        fs::write(
            self.config_dir().join("config.toml"),
            format!("{SOURCE}{rules}"),
        )
        .unwrap();
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

    /// Proposes `TITLE` for team `H` at 10:00, citing `cites`, and returns the pending Act run's id
    /// and its proposal's queue id.
    fn propose(&self, cites: &[&str]) -> (String, String) {
        let mut args = vec![
            "actions",
            "propose",
            "linear.create_issue",
            "--title",
            TITLE,
            "--team",
            "H",
            "--why",
            "owner asked",
        ];
        for cite in cites {
            args.push("--cite");
            args.push(cite);
        }
        let answer = self.ok_json(&args, PROPOSED_AT);
        let run = &answer["run"];
        assert_eq!(run["status"], "pending", "{answer}");
        (
            run["id"].as_str().unwrap().to_string(),
            answer["id"].as_str().unwrap().to_string(),
        )
    }

    /// The one run `rastro actions` lists.
    fn run(&self) -> Value {
        let runs = self.ok_json(&["actions"], DUE)["runs"].clone();
        let runs = runs.as_array().unwrap();
        assert_eq!(runs.len(), 1, "{runs:?}");
        runs[0].clone()
    }

    /// The queue's items, as a machine reads them.
    fn queue_items(&self) -> Vec<Value> {
        self.ok_json(&["queue"], DUE)["items"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn db(&self) -> SqliteStorage {
        SqliteStorage::open(&self.home.path().join("data/rastro.sqlite3")).unwrap()
    }

    /// An open issue a sync observed in `linear-work`, seeded as a connector run's evidence: what
    /// the re-check reads, without calling Linear.
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
}

// spec: An Act is checked again before it runs
#[test]
fn the_removed_rule_returns_the_act_and_its_proposal_waits() {
    let env = Env::new();
    let (id, proposal) = env.propose(&[]);

    // The owner deletes the rule before the grace ends.
    env.set_rules("");
    env.ok(&["tick"], DUE);

    assert!(
        env.linear.requests().is_empty(),
        "a returned Act reached Linear: {:?}",
        env.linear.requests()
    );
    let run = env.run();
    assert_eq!(run["id"], id.as_str());
    assert_eq!(run["status"], "returned", "{run}");

    // The proposal waits in the queue as an action proposal, for the owner to decide.
    let items = env.queue_items();
    let item = items
        .iter()
        .find(|item| item["id"] == proposal.as_str())
        .unwrap_or_else(|| panic!("no queue item for {proposal}: {items:?}"));
    assert_eq!(item["kind"], "action_proposal", "{item}");
    assert_eq!(item["action"]["fields"]["title"], TITLE, "{item}");

    // A returned run is final: a later tick sends nothing and changes nothing.
    env.ok(&["tick"], AFTER);
    assert!(env.linear.requests().is_empty(), "a returned Act ran again");
    assert_eq!(env.run()["status"], "returned");
}

// spec: An Act is checked again before it runs
#[test]
fn an_open_issue_with_the_same_title_returns_the_act() {
    let env = Env::new();
    let (id, _proposal) = env.propose(&[]);

    // A sync observes an open Linear issue whose title equals the pending Act's, during grace.
    env.observe_issue("a1", "H-12", TITLE, "started");

    env.ok(&["tick"], DUE);

    assert!(
        env.linear.requests().is_empty(),
        "an Act whose title an open issue holds reached Linear: {:?}",
        env.linear.requests()
    );
    let run = env.run();
    assert_eq!(run["id"], id.as_str());
    assert_eq!(run["status"], "returned", "{run}");

    // A returned run is final: a later tick sends nothing and changes nothing.
    env.ok(&["tick"], AFTER);
    assert!(env.linear.requests().is_empty(), "a returned Act ran again");
    assert_eq!(env.run()["status"], "returned");
}

// spec: An Act is checked again before it runs
#[test]
fn a_retracted_citation_returns_the_act() {
    let env = Env::new();
    // A claim observed in the work scope, which the proposal cites. Its source was checked just
    // now too, so a tick finds it fresh and only runs actions.
    let claim = support::seed::claim_in(env.home.path(), "work", "https://example.test/o/p/pull/1");
    support::seed::checked_now(env.home.path(), "notes-work", "work");
    let (id, _proposal) = env.propose(&[&claim]);

    // The owner retracts the cited claim during the grace window.
    let why = env.ok_json(&["why", &claim], PROPOSED_AT);
    let revision = why["claim"]["revision"].as_str().unwrap();
    env.ok(&["claim", "reject", &claim, "--seen", revision], BEFORE);

    env.ok(&["tick"], DUE);

    assert!(
        env.linear.requests().is_empty(),
        "an Act with a retracted citation reached Linear: {:?}",
        env.linear.requests()
    );
    let run = env.run();
    assert_eq!(run["id"], id.as_str());
    assert_eq!(run["status"], "returned", "{run}");

    // A returned run is final: a later tick sends nothing and changes nothing.
    env.ok(&["tick"], AFTER);
    assert!(env.linear.requests().is_empty(), "a returned Act ran again");
    assert_eq!(env.run()["status"], "returned");
}
