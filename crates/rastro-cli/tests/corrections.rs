#![allow(non_snake_case)]
//! The owner confirms, retracts or fixes a claim end to end: `rastro claim confirm`, `reject` and
//! `edit` record one intent bound to the revision the owner saw. A confirmed claim reads
//! `confirmed` in `about`, `why`, a merged Person's group and a scoped replay; a retracted claim
//! leaves every current read, in every scope that sees it; a rebuild brings both back unchanged.
mod support;
use chrono::{DateTime, Utc};
use rastro_core::claims::{self, Entity, Object, ObservedClaim, Support};
use rastro_core::knowledge::AboutOptions;
use rastro_core::{Evidence, Scope, Source, Storage, TrustClass};
use rastro_storage_sqlite::SqliteStorage;
use rusqlite::Connection;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};
use tempfile::TempDir;

/// The extractor a fixture source is bound to; it is never installed, so a rebuild carries the
/// source's pair over.
const MISSING: &str = "missing-extractor";

fn time() -> DateTime<Utc> {
    "2026-11-01T00:00:00Z".parse().unwrap()
}
struct Env {
    home: TempDir,
    /// The extractor the fixture claims are bound to, and the directory its manifest is found in.
    extractor: &'static str,
    plugins: std::path::PathBuf,
}
impl Env {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let plugins = home.path().join("no-plugins");
        Self {
            home,
            extractor: MISSING,
            plugins,
        }
    }
    /// An environment whose claims come from the installed test extractor, so the vocabulary
    /// that defines their predicates is there to check an edit against.
    fn with_vocabulary() -> Self {
        Self {
            extractor: "test-extractor",
            plugins: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../rastro-test-extractor"),
            ..Self::new()
        }
    }
    fn run(&self, args: &[&str]) -> (bool, Value) {
        let out = Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", &self.plugins)
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
        (out.status.success(), value)
    }
    fn ok(&self, args: &[&str]) -> Value {
        let (ok, value) = self.run(args);
        assert!(ok, "{args:?}: {value}");
        value["result"].clone()
    }
    /// The operation error a refused command gives.
    fn refused(&self, args: &[&str]) -> String {
        let (ok, value) = self.run(args);
        assert!(!ok, "{args:?} should be refused: {value}");
        value["error"].as_str().unwrap().to_string()
    }
    fn path(&self) -> std::path::PathBuf {
        self.home.path().join("data/rastro.sqlite3")
    }
    fn db(&self) -> SqliteStorage {
        SqliteStorage::open(&self.path()).unwrap()
    }
    fn intents(&self) -> i64 {
        Connection::open(self.path())
            .unwrap()
            .query_row("SELECT count(*) FROM intent", [], |row| row.get(0))
            .unwrap()
    }
    /// Records one claim read from `location` and `field` by source `project` (scope `personal`).
    fn record(&self, subject: &Entity, predicate: &str, object: Object, at: (&str, &str)) {
        let (location, field) = at;
        let source = Source::github("project".into(), "o/p".into(), Scope("personal".into()));
        let mut db = self.db();
        db.project_sources(std::slice::from_ref(&source)).unwrap();
        db.bind_extractor(&source.name, Some(self.extractor))
            .unwrap();
        let run = db
            .start_run(&source, (self.extractor, "1"), time(), (time(), time()))
            .unwrap();
        let quote = match &object {
            Object::Literal { value, .. } => value.clone(),
            Object::Entity { identifier, .. } => identifier.clone(),
        };
        let e = Evidence {
            source: source.name.clone(),
            scope: source.scope.clone(),
            location: location.into(),
            field: field.into(),
            start: 0,
            end: quote.len() as u64,
            quote,
            trust_class: TrustClass::Structured,
            captured_at: time(),
            source_updated_at: Some(time()),
            original: None,
        };
        db.append_evidence(run, &e).unwrap();
        let (evidence_id, _) = db
            .pending_evidence((self.extractor, "1"), &source)
            .unwrap()
            .into_iter()
            .rev()
            .find(|(_, item)| item.location == location && item.field == field)
            .unwrap();
        let account = matches!(&object, Object::Literal { value, .. }
            if predicate == claims::TYPE_PREDICATE && value.ends_with(":Account"));
        let observed = ObservedClaim {
            id: claims::claim_id(&subject.id, predicate, &object),
            subject: subject.clone(),
            predicate: predicate.into(),
            object,
            scalar: true,
            type_ancestors: if account {
                vec![rastro_core::identity::ACCOUNT_TYPE.into()]
            } else {
                Vec::new()
            },
            supports: BTreeSet::from([Support {
                portable_key: claims::portable_evidence_key(&e),
                evidence_id,
                field: field.into(),
                start: 0,
                end: e.end,
                extraction_rule: "fixture-rule".into(),
            }]),
        };
        db.record_claims(run, (self.extractor, "1"), &[observed], None)
            .unwrap();
    }
    /// A GitHub-shaped account `github:account/<n>`, typed `code:Account`, with a login alias.
    fn account(&self, n: u32, login: &str) -> String {
        let location = format!("https://api.github.com/users/{n}");
        let subject = Entity::new(format!("github:account/{n}"), "code:Account".into());
        let typed = Object::Literal {
            value: "code:Account".into(),
            datatype: claims::TYPE_DATATYPE.into(),
        };
        self.record(
            &subject,
            claims::TYPE_PREDICATE,
            typed,
            (&location, "/type"),
        );
        self.login(n, login, &location);
        format!("github:login/{login}")
    }
    /// The login alias of account `n`, read at `location`.
    fn login(&self, n: u32, login: &str, location: &str) {
        let subject = Entity::new(format!("github:account/{n}"), "code:Account".into());
        let alias = Object::Literal {
            value: login.into(),
            datatype: "login".into(),
        };
        self.record(
            &subject,
            claims::ALIAS_PREDICATE,
            alias,
            (location, "/login"),
        );
    }
    /// A new named Person; returns its canonical identifier.
    fn person(&self, name: &str) -> String {
        let created = self.ok(&["identity", "person", "create", "--name", name]);
        created["person"]["identifier"]
            .as_str()
            .unwrap()
            .to_string()
    }
    fn about(&self, entity: &str) -> Value {
        self.ok(&["about", entity])
    }
    /// The claim of `predicate` that `entity`'s `about` shows, whose object is `object`.
    fn claim(&self, entity: &str, predicate: &str, object: &str) -> Value {
        let about = self.about(entity);
        about["claims"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| {
                c["predicate"] == predicate
                    && (c["object"]["identifier"] == object || c["object"]["value"] == object)
            })
            .unwrap_or_else(|| panic!("no {predicate} {object}: {about}"))
            .clone()
    }
    fn reject(&self, claim: &Value) -> Value {
        self.ok(&[
            "claim",
            "reject",
            claim["id"].as_str().unwrap(),
            "--seen",
            claim["revision"].as_str().unwrap(),
        ])
    }
    fn edit(&self, claim: &Value, object: &str) -> Value {
        self.ok(&[
            "claim",
            "edit",
            claim["id"].as_str().unwrap(),
            "--seen",
            claim["revision"].as_str().unwrap(),
            "--object",
            object,
        ])
    }
    /// Pull request `github:pr/o/p/1`, which the installed test extractor's vocabulary says is
    /// `open` and in repository `github:repo/o/p`.
    fn pull_request(&self) -> &'static str {
        let pr = Entity::new("github:pr/o/p/1".into(), "code:PullRequest".into());
        let location = "https://api.github.com/repos/o/p/pulls/1";
        let state = Object::Literal {
            value: "open".into(),
            datatype: "string".into(),
        };
        self.record(&pr, "code:state", state, (location, "/state"));
        let repository = Object::Entity {
            id: claims::entity_id("github:repo/o/p"),
            identifier: "github:repo/o/p".into(),
            entity_type: "code:Repository".into(),
        };
        self.record(&pr, "code:in_repository", repository, (location, "/repo"));
        "github:pr/o/p/1"
    }
    fn confirm(&self, claim: &Value) -> Value {
        self.ok(&[
            "claim",
            "confirm",
            claim["id"].as_str().unwrap(),
            "--seen",
            claim["revision"].as_str().unwrap(),
        ])
    }
}

// spec: The owner confirms a claim
#[test]
fn a_confirmed_claim_reads_confirmed_and_why_cites_the_confirmation() {
    let env = Env::new();
    env.account(7, "octocat");
    let seen = env.claim("github:account/7", "rastro:alias", "octocat");
    assert_eq!(seen["status"], "observed");
    let written = env.confirm(&seen);
    let intent = &written["intents"][0];
    assert_eq!(intent["kind"], "confirm", "{written}");
    assert_eq!(written["intents"].as_array().unwrap().len(), 1);

    let id = seen["id"].as_str().unwrap();
    let about = env.about("github:account/7");
    let shown = about["claims"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("{id} missing: {about}"));
    assert_eq!(shown["status"], "confirmed", "{about}");

    let why = env.ok(&["why", id]);
    assert_eq!(why["claim"]["status"], "confirmed", "{why}");
    let supports = why["supports"].as_array().unwrap();
    let confirmation = supports
        .iter()
        .find(|s| s["justification_kind"] == "confirmation")
        .unwrap_or_else(|| panic!("no confirmation justification: {why}"));
    assert_eq!(confirmation["intent"]["id"], intent["id"], "{why}");
    assert_eq!(
        confirmation["intent"]["issued_at"], intent["issued_at"],
        "{why}"
    );
    assert_eq!(confirmation["current"], true);
    // The evidence that supports the claim is still listed beside the owner's word.
    assert!(
        supports
            .iter()
            .any(|s| s["justification_kind"] == "provenance"),
        "{why}"
    );
}

// spec: The owner confirms a claim
#[test]
fn confirming_a_revision_that_is_no_longer_current_is_a_conflict_and_records_nothing() {
    let env = Env::new();
    env.account(7, "octocat");
    let seen = env.claim("github:account/7", "rastro:alias", "octocat");
    // A second source says the same: the claim has a new revision.
    env.login(7, "octocat", "https://example.test/mirror/7");
    let now = env.claim("github:account/7", "rastro:alias", "octocat");
    assert_ne!(seen["revision"], now["revision"]);

    let error = env.refused(&[
        "claim",
        "confirm",
        seen["id"].as_str().unwrap(),
        "--seen",
        seen["revision"].as_str().unwrap(),
    ]);
    assert!(error.contains(seen["id"].as_str().unwrap()), "{error}");
    assert!(error.contains(now["revision"].as_str().unwrap()), "{error}");
    assert_eq!(env.intents(), 0);
    assert_eq!(
        env.claim("github:account/7", "rastro:alias", "octocat")["status"],
        "observed"
    );
}

// spec: The owner confirms a claim
#[test]
fn a_confirmed_member_leads_its_group_before_an_observed_one() {
    let env = Env::new();
    let octocat = env.account(7, "octocat");
    let (ana, silva) = (env.person("Ana"), env.person("Ana Silva"));
    env.ok(&["identity", "link", &octocat, &ana]);
    env.ok(&["identity", "link", &octocat, &silva]);
    env.ok(&["identity", "merge", &ana, &silva]);
    // The link to Silva is the newer claim, so it leads its group until the owner confirms the
    // older one, the link to Ana.
    let held = |about: &Value| -> Vec<(String, String)> {
        let group = about["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| {
                let id = g["members"][0]["claim"].as_str().unwrap();
                about["claims"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c["id"] == id && c["predicate"] == "rastro:held_by")
            })
            .unwrap_or_else(|| panic!("no held_by group: {about}"));
        group["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                (
                    m["claim"].as_str().unwrap().to_string(),
                    m["status"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    };
    let before = held(&env.about(&ana));
    let older = env.claim(&ana, "rastro:held_by", &ana);
    assert_ne!(before[0].0, older["id"].as_str().unwrap(), "fixture order");

    env.confirm(&older);
    let about = env.about(&ana);
    let members = held(&about);
    assert_eq!(
        members,
        [
            (
                older["id"].as_str().unwrap().to_string(),
                "confirmed".into()
            ),
            (before[0].0.clone(), "observed".into()),
        ],
        "{about}"
    );
    // The two claims stay side by side in the listing, the confirmed one first, and the group
    // says the strongest of its members.
    let listed: Vec<&str> = about["claims"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    let at = |id: &str| listed.iter().position(|c| *c == id).unwrap();
    assert!(at(&members[0].0) < at(&members[1].0), "{about}");
    let group = about["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["members"][0]["claim"] == members[0].0.as_str())
        .unwrap();
    assert_eq!(group["status"], "confirmed", "{group}");
}

/// Every projection row a rebuild must reproduce, and the intent table, as stored.
fn rows(env: &Env) -> (Vec<String>, Vec<String>) {
    let db = Connection::open(env.path()).unwrap();
    let dump = |query: &str| -> Vec<String> {
        let mut statement = db.prepare(query).unwrap();
        let width = statement.column_count();
        statement
            .query_map([], |row| {
                (0..width)
                    .map(|i| {
                        row.get::<_, rusqlite::types::Value>(i)
                            .map(|v| format!("{v:?}"))
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(|cells| cells.join("|"))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let mut projection = Vec::new();
    for query in [
        "SELECT 'claim', * FROM claim ORDER BY id",
        "SELECT 'intent_justification', * FROM intent_justification ORDER BY id",
        "SELECT 'justification', id, claim_id, extractor, current FROM justification ORDER BY id",
        "SELECT 'occurrence', id, justification_id, evidence_id FROM occurrence ORDER BY 2",
        "SELECT 'link', * FROM claim_link ORDER BY 2, 3, 4",
        "SELECT 'entity', * FROM entity ORDER BY id",
    ] {
        projection.extend(dump(query));
    }
    (projection, dump("SELECT * FROM intent ORDER BY append_seq"))
}

// spec: Corrections replay at rebuild
#[test]
fn a_rebuild_brings_every_confirmation_retraction_and_fix_back_with_the_same_ids_and_statuses() {
    let env = Env::new();
    let octocat = env.account(7, "octocat");
    let ana = env.person("Ana");
    env.ok(&["identity", "link", &octocat, &ana]);
    // One confirmation on a claim evidence holds, one on a claim only an owner intent holds.
    let observed = env.claim("github:account/7", "rastro:alias", "octocat");
    let link = env.claim(&ana, "rastro:held_by", &ana);
    env.confirm(&observed);
    env.confirm(&link);
    // One retraction, on a claim evidence holds.
    env.login(7, "octo-alias", "https://example.test/alias/7");
    let wrong = env.claim("github:account/7", "rastro:alias", "octo-alias");
    env.reject(&wrong);
    // One fix, on a claim of the standard vocabulary the core bundles.
    let promise = Entity::new("std:commitment/c1".into(), "std:Commitment".into());
    let owes = Object::Literal {
        value: "Ana".into(),
        datatype: "string".into(),
    };
    env.record(&promise, "std:owes", owes, ("meeting:m/1", "/owes"));
    let promised = env.claim("std:commitment/c1", "std:owes", "Ana");
    env.edit(&promised, "Bea");
    let fixed = env.claim("std:commitment/c1", "std:owes", "Bea");

    let before = rows(&env);
    let read = |entity: &str, claim: &Value| {
        (
            env.about(entity),
            env.ok(&["why", claim["id"].as_str().unwrap()]),
        )
    };
    let (about_before, why_before) = read(&ana, &link);
    let (account_about, account_why) = read("github:account/7", &observed);
    let (promise_about, fixed_why) = read("std:commitment/c1", &fixed);
    assert_eq!(
        env.ok(&["why", wrong["id"].as_str().unwrap()])["claim"]["status"],
        "retracted"
    );
    assert_eq!(
        env.ok(&["why", promised["id"].as_str().unwrap()])["claim"]["status"],
        "retracted"
    );
    assert!(
        about_before.to_string().contains("\"confirmed\""),
        "{about_before}"
    );

    let rebuilt = env.ok(&["claims", "rebuild"]);
    assert_eq!(rebuilt["pairs"][0]["state"], "not_installed", "{rebuilt}");
    assert_eq!(rows(&env), before);
    assert_eq!(read(&ana, &link), (about_before, why_before));
    assert_eq!(
        read("github:account/7", &observed),
        (account_about, account_why)
    );
    assert_eq!(
        read("std:commitment/c1", &fixed),
        (promise_about, fixed_why)
    );
    assert_eq!(
        env.ok(&["why", promised["id"].as_str().unwrap()])["claim"]["status"],
        "retracted"
    );
    env.ok(&["claims", "rebuild"]);
    assert_eq!(rows(&env), before);
}

// spec: The owner confirms a claim
#[test]
fn a_scoped_read_keeps_the_confirmed_status_of_a_claim_it_can_see() {
    let env = Env::new();
    env.account(7, "octocat");
    let seen = env.claim("github:account/7", "rastro:alias", "octocat");
    env.confirm(&seen);

    let read = env
        .db()
        .about_scoped_within(
            "github:account/7",
            &AboutOptions::default(),
            &[Scope("personal".into())],
            1_000,
        )
        .unwrap();
    let shown = read
        .iter()
        .find(|c| c.claim.id == seen["id"].as_str().unwrap())
        .expect("the scope sees the claim");
    assert_eq!(shown.claim.status, "confirmed");
}

/// `rastro mcp --scope personal`, driven over stdio by raw JSON lines.
struct Mcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}
impl Mcp {
    fn start(env: &Env) -> Self {
        let mut child = Command::new(support::rastro())
            .env("RASTRO_HOME", env.home.path())
            .env("RASTRO_PLUGIN_PATH", &env.plugins)
            .args(["mcp", "--scope", "personal"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut mcp = Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        };
        mcp.request(
            "initialize",
            serde_json::json!({"protocolVersion": "2025-11-25", "capabilities": {},
                "clientInfo": {"name": "corrections", "version": "1"}}),
        );
        mcp.send(serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        mcp
    }
    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(
            serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        );
        loop {
            let mut line = String::new();
            assert_ne!(
                self.stdout.read_line(&mut line).unwrap(),
                0,
                "server closed"
            );
            let message: Value = serde_json::from_str(&line).unwrap();
            if message["id"] == serde_json::json!(id) && message.get("method").is_none() {
                return message;
            }
        }
    }
    /// The claim ids the scoped `about` tool lists for `entity`.
    fn about(&mut self, entity: &str) -> Vec<String> {
        let answer = self.request(
            "tools/call",
            serde_json::json!({"name": "about", "arguments": {"entity": entity}}),
        );
        let result = &answer["result"]["structuredContent"]["result"];
        result["claims"]
            .as_array()
            .unwrap_or_else(|| panic!("no claims: {answer}"))
            .iter()
            .map(|c| c["id"].as_str().unwrap().to_string())
            .collect()
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// spec: The owner retracts a claim
#[test]
fn a_rejected_claim_leaves_about_and_why_says_retracted_by_the_reject_intent() {
    let env = Env::new();
    env.account(7, "octocat");
    let seen = env.claim("github:account/7", "rastro:alias", "octocat");
    let id = seen["id"].as_str().unwrap();
    let written = env.reject(&seen);
    let intent = &written["intents"][0];
    assert_eq!(intent["kind"], "reject", "{written}");
    assert_eq!(written["intents"].as_array().unwrap().len(), 1);
    assert_eq!(env.intents(), 1);

    let about = env.about("github:account/7");
    assert!(
        about["claims"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["id"] != id),
        "the retracted claim is still listed: {about}"
    );
    let why = env.ok(&["why", id]);
    assert_eq!(why["claim"]["status"], "retracted", "{why}");
    let retraction = why["supports"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["extraction_rule"] == "reject")
        .unwrap_or_else(|| panic!("no retraction justification: {why}"));
    assert_eq!(retraction["intent"]["id"], intent["id"], "{why}");
    assert_eq!(retraction["current"], true);
    // The evidence that said it is still listed beside the owner's word.
    assert!(
        why["supports"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["justification_kind"] == "provenance"),
        "{why}"
    );
}

// spec: The owner retracts a claim
#[test]
fn a_scoped_mcp_read_leaves_out_a_rejected_claim() {
    let env = Env::new();
    env.account(7, "octocat");
    let seen = env.claim("github:account/7", "rastro:alias", "octocat");
    let id = seen["id"].as_str().unwrap();
    let mut mcp = Mcp::start(&env);
    assert!(
        mcp.about("github:account/7").iter().any(|c| c == id),
        "the scope should see the claim before it is rejected"
    );

    env.reject(&seen);
    let listed = mcp.about("github:account/7");
    assert!(!listed.is_empty(), "the account's other claims stay");
    assert!(
        !listed.iter().any(|c| c == id),
        "the rejected claim is still in the scoped read: {listed:?}"
    );
}

// spec: The owner retracts a claim
#[test]
fn rejecting_a_claim_an_owner_link_made_is_sent_to_identity_undo() {
    let env = Env::new();
    let octocat = env.account(7, "octocat");
    let ana = env.person("Ana");
    let linked = env.ok(&["identity", "link", &octocat, &ana]);
    let link = linked["intents"][0]["id"].as_str().unwrap().to_string();
    let held = env.claim(&ana, "rastro:held_by", &ana);
    let before = env.intents();

    let error = env.refused(&[
        "claim",
        "reject",
        held["id"].as_str().unwrap(),
        "--seen",
        held["revision"].as_str().unwrap(),
    ]);
    assert!(error.contains("identity undo"), "{error}");
    assert!(error.contains(&link), "{error}");
    assert_eq!(env.intents(), before);
    assert_eq!(
        env.claim(&ana, "rastro:held_by", &ana)["status"],
        "observed"
    );
}

// spec: The owner retracts a claim
#[test]
fn a_later_confirmation_ends_the_retraction_it_overrides() {
    let env = Env::new();
    env.account(7, "octocat");
    let seen = env.claim("github:account/7", "rastro:alias", "octocat");
    let id = seen["id"].as_str().unwrap();
    let rejected = env.reject(&seen)["intents"][0]["id"].clone();
    // The retracted claim is read again at its new revision, and confirmed.
    let why = env.ok(&["why", id]);
    let confirmed = env.confirm(&why["claim"])["intents"][0]["id"].clone();

    let why = env.ok(&["why", id]);
    assert_eq!(why["claim"]["status"], "confirmed", "{why}");
    let current = |intent: &Value| {
        why["supports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["intent"]["id"] == *intent)
            .unwrap_or_else(|| panic!("no support by {intent}: {why}"))["current"]
            .clone()
    };
    assert_eq!(current(&confirmed), true, "{why}");
    assert_eq!(
        current(&rejected),
        false,
        "the overridden retraction still holds: {why}"
    );
}

/// The claims `entity`'s `about` lists for `predicate`, as (object text, status).
fn listed(env: &Env, entity: &str, predicate: &str) -> Vec<(String, String)> {
    env.about(entity)["claims"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["predicate"] == predicate)
        .map(|c| {
            let object = &c["object"];
            let text = object["value"].as_str().or(object["identifier"].as_str());
            (
                text.unwrap().to_string(),
                c["status"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

// spec: The owner fixes a claim's object
#[test]
fn an_edited_claim_replaces_the_old_one_and_why_cites_the_edit_alone() {
    let env = Env::with_vocabulary();
    let pr = env.pull_request();
    let seen = env.claim(pr, "code:state", "open");
    let written = env.edit(&seen, "closed");
    let intent = &written["intents"][0];
    assert_eq!(intent["kind"], "edit", "{written}");
    assert_eq!(written["intents"].as_array().unwrap().len(), 1);
    assert_eq!(env.intents(), 1);

    assert_eq!(
        listed(&env, pr, "code:state"),
        [("closed".to_string(), "observed".to_string())],
        "{}",
        env.about(pr)
    );
    let fixed = env.claim(pr, "code:state", "closed");
    let why = env.ok(&["why", fixed["id"].as_str().unwrap()]);
    let supports = why["supports"].as_array().unwrap();
    assert_eq!(supports.len(), 1, "{why}");
    assert_eq!(supports[0]["intent"]["id"], intent["id"], "{why}");
    assert_eq!(supports[0]["justification_kind"], "owner_intent", "{why}");
    assert_eq!(supports[0]["current"], true);
    // The claim it replaced says it is retracted, by the same intent.
    let old = env.ok(&["why", seen["id"].as_str().unwrap()]);
    assert_eq!(old["claim"]["status"], "retracted", "{old}");
}

// spec: The owner fixes a claim's object
#[test]
fn an_object_outside_the_predicates_range_is_refused_naming_both_and_records_nothing() {
    let env = Env::with_vocabulary();
    let pr = env.pull_request();
    let seen = env.claim(pr, "code:in_repository", "github:repo/o/p");
    let before = env.about(pr)["claims"].clone();

    let error = env.refused(&[
        "claim",
        "edit",
        seen["id"].as_str().unwrap(),
        "--seen",
        seen["revision"].as_str().unwrap(),
        "--object",
        "next-ish",
    ]);
    assert!(error.contains("code:in_repository"), "{error}");
    assert!(error.contains("code:Repository"), "{error}");
    assert_eq!(env.intents(), 0);
    assert_eq!(env.about(pr)["claims"], before);
}

// spec: The owner fixes a claim's object
#[test]
fn an_entity_in_the_predicates_range_fixes_an_entity_object() {
    let env = Env::with_vocabulary();
    let pr = env.pull_request();
    // A second repository the evidence knows, typed as one.
    let other = Entity::new("github:repo/o/q".into(), "code:Repository".into());
    let typed = Object::Literal {
        value: "code:Repository".into(),
        datatype: claims::TYPE_DATATYPE.into(),
    };
    let at = "https://api.github.com/repos/o/q";
    env.record(&other, claims::TYPE_PREDICATE, typed, (at, "/type"));
    let seen = env.claim(pr, "code:in_repository", "github:repo/o/p");

    env.edit(&seen, "github:repo/o/q");
    assert_eq!(
        listed(&env, pr, "code:in_repository"),
        [("github:repo/o/q".to_string(), "observed".to_string())],
    );
}

// spec: The owner fixes a claim's object
#[test]
fn a_scoped_mcp_read_shows_the_fixed_claim_and_not_the_one_it_replaced() {
    let env = Env::with_vocabulary();
    let pr = env.pull_request();
    let seen = env.claim(pr, "code:state", "open");
    let mut mcp = Mcp::start(&env);
    assert!(
        mcp.about(pr)
            .iter()
            .any(|c| c == seen["id"].as_str().unwrap())
    );

    env.edit(&seen, "closed");
    let fixed = env.claim(pr, "code:state", "closed");
    let listed = mcp.about(pr);
    assert!(
        listed.iter().any(|c| c == fixed["id"].as_str().unwrap()),
        "{listed:?}"
    );
    assert!(
        !listed.iter().any(|c| c == seen["id"].as_str().unwrap()),
        "{listed:?}"
    );
}
