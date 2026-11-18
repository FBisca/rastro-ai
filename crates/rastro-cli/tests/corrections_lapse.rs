#![allow(non_snake_case)]
//! A correction follows its claim's lifecycle fingerprint (ADR 0014), end to end: a retraction
//! holds while the primary justification it was bound to holds and lapses when that one ends,
//! leaving the claim back with its evidence-derived status marked earlier retracted; a second
//! source corroborating the claim changes nothing the retraction is bound to; a confirmation
//! stays in force when its only evidence ends, marked stale, with the ended justification in
//! `why`'s history.
mod support;
use chrono::{DateTime, Utc};
use rastro_core::claims::{self, Entity, Object, ObservedClaim, Support};
use rastro_core::knowledge::AboutOptions;
use rastro_core::{Evidence, Scope, Source, Storage, TrustClass};
use rastro_storage_sqlite::SqliteStorage;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
};
use tempfile::TempDir;

/// The extractor a fixture source is bound to; it is never installed.
const EXTRACTOR: &str = "missing-extractor";
const ACCOUNT: &str = "github:account/7";

fn time(stamp: &str) -> DateTime<Utc> {
    stamp.parse().unwrap()
}

struct Env {
    home: TempDir,
}
impl Env {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().unwrap(),
        }
    }
    fn ok(&self, args: &[&str]) -> Value {
        let out = Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.home.path().join("no-plugins"))
            .args(args)
            .arg("--json")
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
            panic!(
                "{args:?}: {}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        });
        let schema: Value =
            serde_json::from_str(include_str!("../../../schema/operations-v2.schema.json"))
                .unwrap();
        jsonschema::validator_for(&schema)
            .unwrap()
            .validate(&value)
            .unwrap_or_else(|error| panic!("{error}: {value}"));
        assert!(out.status.success(), "{args:?}: {value}");
        value["result"].clone()
    }
    /// The login `value` of account 7, read at `location` by `source` on `day` (2026-11-<day>).
    /// A later capture at the same location replaces the earlier one.
    fn login(&self, source: &str, location: &str, value: &str, day: u32) {
        self.login_in("personal", source, location, value, day);
    }
    /// [`Env::login`] read in `scope`.
    fn login_in(&self, scope: &str, source: &str, location: &str, value: &str, day: u32) {
        let captured = time(&format!("2026-11-{day:02}T00:00:00Z"));
        let source = Source::github(source.into(), "o/p".into(), Scope(scope.into()));
        let subject = Entity::new(ACCOUNT.into(), "code:Account".into());
        let object = Object::Literal {
            value: value.into(),
            datatype: "login".into(),
        };
        let mut db = SqliteStorage::open(&self.home.path().join("data/rastro.sqlite3")).unwrap();
        db.project_sources(std::slice::from_ref(&source)).unwrap();
        db.bind_extractor(&source.name, Some(EXTRACTOR)).unwrap();
        let run = db
            .start_run(&source, (EXTRACTOR, "1"), captured, (captured, captured))
            .unwrap();
        let evidence = Evidence {
            source: source.name.clone(),
            scope: source.scope.clone(),
            location: location.into(),
            field: "/login".into(),
            start: 0,
            end: value.len() as u64,
            quote: value.into(),
            trust_class: TrustClass::Structured,
            captured_at: captured,
            source_updated_at: Some(captured),
            original: None,
        };
        db.append_evidence(run, &evidence).unwrap();
        let (evidence_id, _) = db
            .pending_evidence((EXTRACTOR, "1"), &source)
            .unwrap()
            .into_iter()
            .rev()
            .find(|(_, item)| item.location == location && item.field == "/login")
            .unwrap();
        let observed = ObservedClaim {
            id: claims::claim_id(&subject.id, claims::ALIAS_PREDICATE, &object),
            subject,
            predicate: claims::ALIAS_PREDICATE.into(),
            object,
            scalar: true,
            type_ancestors: Vec::new(),
            supports: BTreeSet::from([Support {
                portable_key: claims::portable_evidence_key(&evidence),
                evidence_id,
                field: "/login".into(),
                start: 0,
                end: evidence.end,
                extraction_rule: "fixture-rule".into(),
            }]),
        };
        db.record_claims(run, (EXTRACTOR, "1"), &[observed], None)
            .unwrap();
    }
    /// The claim `about` lists for account 7's login `value`, if it lists one.
    fn listed(&self, value: &str) -> Option<Value> {
        let about = self.ok(&["about", ACCOUNT]);
        about["claims"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["predicate"] == claims::ALIAS_PREDICATE && c["object"]["value"] == value)
            .cloned()
    }
    fn seen(&self, value: &str) -> Value {
        self.listed(value)
            .unwrap_or_else(|| panic!("{value} is not listed"))
    }
    /// The claims a read scoped to `scope` lists for account 7, as (login, status, marks).
    fn scoped(&self, scope: &str) -> Vec<(String, String, Vec<&'static str>)> {
        let mut db = SqliteStorage::open(&self.home.path().join("data/rastro.sqlite3")).unwrap();
        db.about_scoped_within(
            ACCOUNT,
            &AboutOptions::default(),
            &[Scope(scope.into())],
            1_000,
        )
        .unwrap()
        .into_iter()
        .filter(|c| c.claim.predicate == claims::ALIAS_PREDICATE)
        .map(|c| {
            let Object::Literal { value, .. } = c.claim.object else {
                panic!("an alias is a literal")
            };
            let marks = [
                (c.claim.earlier_retracted, "earlier_retracted"),
                (c.claim.stale, "stale"),
            ];
            let marks = marks.into_iter().filter(|m| m.0).map(|m| m.1).collect();
            (value, c.claim.status, marks)
        })
        .collect()
    }
    /// Records one correction of `claim` as the owner read it; the intent it recorded.
    fn correct(&self, verb: &str, claim: &Value) -> String {
        let done = self.ok(&[
            "claim",
            verb,
            claim["id"].as_str().unwrap(),
            "--seen",
            claim["revision"].as_str().unwrap(),
        ]);
        done["intents"][0]["id"].as_str().unwrap().to_string()
    }
    /// The `queue` an MCP server launched with `scope` returns: the items it lists.
    fn mcp_queue(&self, scope: &str) -> Vec<Value> {
        let mut server = Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.home.path().join("no-plugins"))
            .args(["mcp", "--scope", scope])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = server.stdin.take().unwrap();
        let mut output = BufReader::new(server.stdout.take().unwrap());
        let mut send = |message: Value| writeln!(input, "{message}").unwrap();
        let mut receive = |id: u64| loop {
            let mut line = String::new();
            assert!(
                output.read_line(&mut line).unwrap() > 0,
                "the server closed"
            );
            let message: Value = serde_json::from_str(&line).unwrap();
            if message["id"] == id && message.get("method").is_none() {
                return message;
            }
        };
        send(
            serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-11-25", "capabilities": {},
                       "clientInfo": {"name": "scripted-client", "version": "1.0"}}}),
        );
        receive(1);
        send(serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        send(
            serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "queue", "arguments": {}}}),
        );
        let response = receive(2);
        server.kill().unwrap();
        server.wait().unwrap();
        let queue = &response["result"]["structuredContent"]["result"];
        queue["items"]
            .as_array()
            .unwrap_or_else(|| panic!("{response}"))
            .clone()
    }
    /// The human queue read the owner prints: what it prints is what it showed, and Rastro records
    /// that it showed it.
    fn print_queue(&self) {
        let out = Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.home.path().join("no-plugins"))
            .arg("queue")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("retraction lapsed"), "{text}");
    }
    /// The questions the queue holds about a correction, one per lapsed retraction or stale
    /// confirmation.
    fn asks(&self) -> Vec<Value> {
        let queue = self.ok(&["queue"]);
        (queue["items"].as_array().unwrap().iter())
            .filter(|item| item.get("correction").is_some())
            .cloned()
            .collect()
    }
}

// spec: A correction follows its claim's lifecycle state
#[test]
fn a_retracted_claim_returns_marked_when_its_bound_occurrence_closes() {
    let env = Env::new();
    // Two sources say the login; the older reading is the one the retraction binds to.
    env.login("project", "https://api.github.com/users/7", "octocat", 1);
    env.login("mirror", "https://example.test/mirror/7", "octocat", 2);
    let seen = env.seen("octocat");
    env.correct("reject", &seen);
    assert!(env.listed("octocat").is_none(), "a retraction hides it");

    // The older reading is replaced: its occurrence closes, the mirror's still holds the claim.
    env.login("project", "https://api.github.com/users/7", "octocat2", 3);
    let returned = env
        .listed("octocat")
        .expect("the retraction lapsed with its occurrence");
    assert_eq!(returned["status"], "observed", "{returned}");
    assert_eq!(returned["earlier_retracted"], true, "{returned}");
    assert!(returned.get("stale").is_none(), "{returned}");
    // The claim the owner never rejected carries no mark.
    assert!(env.seen("octocat2").get("earlier_retracted").is_none());
}

// spec: A correction follows its claim's lifecycle state
#[test]
fn corroboration_from_another_source_keeps_a_retracted_claim_out() {
    let env = Env::new();
    env.login("project", "https://api.github.com/users/7", "octocat", 1);
    env.correct("reject", &env.seen("octocat"));
    assert!(env.listed("octocat").is_none());

    env.login("mirror", "https://example.test/mirror/7", "octocat", 2);
    assert!(
        env.listed("octocat").is_none(),
        "a second source changes the revision, not the lifecycle the retraction is bound to"
    );
}

// spec: A correction follows its claim's lifecycle state
#[test]
fn a_confirmed_claim_whose_only_evidence_ends_is_listed_confirmed_and_stale() {
    let env = Env::new();
    env.login("project", "https://api.github.com/users/7", "octocat", 1);
    let seen = env.seen("octocat");
    env.correct("confirm", &seen);
    assert!(env.seen("octocat").get("stale").is_none(), "fresh");

    env.login("project", "https://api.github.com/users/7", "octocat2", 3);
    let kept = env
        .listed("octocat")
        .expect("a confirmation outlives its evidence");
    assert_eq!(kept["status"], "confirmed", "{kept}");
    assert_eq!(kept["stale"], true, "{kept}");
    assert!(kept.get("earlier_retracted").is_none(), "{kept}");

    let why = env.ok(&["why", seen["id"].as_str().unwrap()]);
    assert_eq!(why["claim"]["stale"], true, "{why}");
    let provenance: Vec<&Value> = why["supports"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["justification_kind"] == "provenance")
        .collect();
    assert_eq!(provenance.len(), 1, "{why}");
    assert_eq!(provenance[0]["current"], false, "{why}");
}

// spec: A correction follows its claim's lifecycle state
#[test]
fn a_scope_that_sees_only_the_corroboration_still_leaves_a_retracted_claim_out() {
    let env = Env::new();
    // The retraction binds to the personal reading, the claim's primary justification. The work
    // scope sees only the later one, whose own fingerprint is another.
    env.login_in(
        "personal",
        "project",
        "https://api.github.com/users/7",
        "octocat",
        1,
    );
    env.login_in(
        "work",
        "mirror",
        "https://example.test/mirror/7",
        "octocat",
        2,
    );
    env.correct("reject", &env.seen("octocat"));
    assert!(env.listed("octocat").is_none());

    let work = env.scoped("work");
    assert!(
        work.iter().all(|(login, _, _)| login != "octocat"),
        "a scope that sees only the corroboration lists the retracted claim: {work:?}"
    );
}

// spec: A correction follows its claim's lifecycle state
#[test]
fn a_scope_that_sees_only_the_corroboration_does_not_read_a_confirmation_stale() {
    let env = Env::new();
    env.login_in(
        "personal",
        "project",
        "https://api.github.com/users/7",
        "octocat",
        1,
    );
    env.login_in(
        "work",
        "mirror",
        "https://example.test/mirror/7",
        "octocat",
        2,
    );
    env.correct("confirm", &env.seen("octocat"));

    assert_eq!(
        env.scoped("work"),
        [("octocat".to_string(), "confirmed".to_string(), vec![])]
    );
}

/// Reads account 7's login `octocat` retracted by the owner, then lets the retraction's bound
/// occurrence close: the claim is back, observed, marked earlier retracted. Returns the claim as
/// the owner saw it and the reject intent.
fn a_lapsed_retraction(env: &Env) -> (Value, String) {
    env.login("project", "https://api.github.com/users/7", "octocat", 1);
    env.login("mirror", "https://example.test/mirror/7", "octocat", 2);
    let seen = env.seen("octocat");
    let intent = env.correct("reject", &seen);
    assert!(
        env.asks().is_empty(),
        "a retraction that holds asks nothing"
    );
    env.login("project", "https://api.github.com/users/7", "octocat2", 3);
    (seen, intent)
}

// spec: A lapsed or stale correction asks the owner
#[test]
fn the_queue_asks_about_a_lapsed_retraction() {
    let env = Env::new();
    let (seen, intent) = a_lapsed_retraction(&env);

    let asks = env.asks();
    assert_eq!(asks.len(), 1, "{asks:?}");
    let ask = &asks[0];
    assert_eq!(ask["kind"], "question", "{ask}");
    assert_eq!(ask["correction"]["kind"], "lapsed_retraction", "{ask}");
    assert_eq!(ask["correction"]["claim"]["id"], seen["id"], "{ask}");
    assert_eq!(ask["correction"]["claim"]["status"], "observed", "{ask}");
    assert_eq!(ask["correction"]["intent"], intent, "{ask}");
}

// spec: A lapsed or stale correction asks the owner
#[test]
fn answering_yes_to_a_lapsed_retraction_retracts_the_claim_again() {
    let env = Env::new();
    let (_, _) = a_lapsed_retraction(&env);
    assert!(env.listed("octocat").is_some(), "the claim is back");
    let item = env.asks()[0]["id"].as_str().unwrap().to_string();

    let answered = env.ok(&["queue", "answer", &item, "yes"]);
    assert_eq!(
        answered["intents"].as_array().unwrap().len(),
        1,
        "{answered}"
    );
    assert_eq!(answered["intents"][0]["kind"], "reject", "{answered}");
    assert!(env.listed("octocat").is_none(), "yes retracts it again");
    assert!(env.asks().is_empty(), "the question is gone");
    let mcp = env.mcp_queue("personal");
    assert!(
        mcp.iter().all(|i| i["id"] != item),
        "the question is gone from the MCP queue: {mcp:?}"
    );
}

// spec: A lapsed or stale correction asks the owner
#[test]
fn the_queue_asks_about_a_stale_confirmation_and_a_yes_confirms_it_again() {
    let env = Env::new();
    env.login("project", "https://api.github.com/users/7", "octocat", 1);
    let seen = env.seen("octocat");
    let intent = env.correct("confirm", &seen);
    assert!(env.asks().is_empty(), "a fresh confirmation asks nothing");
    env.login("project", "https://api.github.com/users/7", "octocat2", 3);

    let asks = env.asks();
    assert_eq!(asks.len(), 1, "{asks:?}");
    assert_eq!(
        asks[0]["correction"]["kind"], "stale_confirmation",
        "{asks:?}"
    );
    assert_eq!(
        asks[0]["correction"]["claim"]["status"], "confirmed",
        "{asks:?}"
    );
    assert_eq!(asks[0]["correction"]["intent"], intent, "{asks:?}");

    let item = asks[0]["id"].as_str().unwrap().to_string();
    let answered = env.ok(&["queue", "answer", &item, "yes"]);
    assert_eq!(answered["intents"][0]["kind"], "confirm", "{answered}");
    let kept = env.seen("octocat");
    assert_eq!(kept["status"], "confirmed", "{kept}");
    assert!(
        kept.get("stale").is_none(),
        "confirmed again, bound to what it is now: {kept}"
    );
    assert!(env.asks().is_empty(), "the question is gone");
}

// spec: A lapsed or stale correction asks the owner
#[test]
fn a_caller_that_cannot_see_the_claim_is_not_asked_about_it() {
    let env = Env::new();
    // The retraction binds to the personal reading; the claim lapses when that reading closes.
    env.login_in(
        "personal",
        "project",
        "https://api.github.com/users/7",
        "octocat",
        1,
    );
    env.login_in(
        "personal",
        "mirror",
        "https://example.test/mirror/7",
        "octocat",
        2,
    );
    env.correct("reject", &env.seen("octocat"));
    env.login_in(
        "personal",
        "project",
        "https://api.github.com/users/7",
        "octocat2",
        3,
    );
    assert_eq!(env.asks().len(), 1, "the owner is asked");

    let asked = |scope| {
        let items = env.mcp_queue(scope);
        items
            .iter()
            .filter(|item| item.get("correction").is_some())
            .count()
    };
    assert_eq!(asked("personal"), 1, "a scope that sees the claim is asked");
    assert_eq!(asked("work"), 0, "a scope that does not is not");
}

// spec: A lapsed or stale correction asks the owner
#[test]
fn a_no_to_a_correction_question_is_refused_and_records_nothing() {
    let env = Env::new();
    a_lapsed_retraction(&env);
    let item = env.asks()[0]["id"].as_str().unwrap().to_string();

    let out = Command::new(support::rastro())
        .env("RASTRO_HOME", env.home.path())
        .env("RASTRO_PLUGIN_PATH", env.home.path().join("no-plugins"))
        .args(["queue", "answer", &item, "no"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let message = String::from_utf8_lossy(&out.stderr);
    assert!(message.contains("rastro claim confirm"), "{message}");
    assert_eq!(env.asks().len(), 1, "the question still waits");
}

// spec: A lapsed or stale correction asks the owner
#[test]
fn an_answered_correction_question_the_owner_was_shown_is_an_answered_outcome() {
    let env = Env::new();
    a_lapsed_retraction(&env);
    env.print_queue();
    let item = env.asks()[0]["id"].as_str().unwrap().to_string();
    env.ok(&["queue", "answer", &item, "yes"]);

    let outcomes = env.ok(&["queue", "outcomes"]);
    let listed = outcomes["outcomes"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "{outcomes}");
    assert_eq!(listed[0]["item"], item, "{outcomes}");
    assert_eq!(listed[0]["kind"], "answered", "{outcomes}");
    assert_eq!(outcomes["totals"]["acted_on"], 1, "{outcomes}");
}
