#![allow(non_snake_case)]
//! `about` and `why` as of a past time, end to end: the answer is rebuilt from the evidence
//! captured by that time and the intents issued by it, under today's rules, and says so. It names
//! a source the entity has claims from that had not been read yet, and the MCP `about` tool
//! answers the same claims and statuses as the CLI.
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
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};
use tempfile::TempDir;

/// The extractor a fixture source is bound to; it is never installed.
const EXTRACTOR: &str = "missing-extractor";
const ACCOUNT: &str = "github:account/7";
const PROJECT: &str = "https://api.github.com/users/7";
const MIRROR: &str = "https://example.test/mirror/7";
/// Well before any owner intent the tests record, which is stamped with the clock's now.
const READ: &str = "2020-01-01T00:00:00Z";
const REREAD: &str = "2020-03-01T00:00:00Z";
const LATER: &str = "2020-04-01T00:00:00Z";
const BETWEEN: &str = "2020-02-01T00:00:00Z";
const BEFORE_ANY: &str = "2019-06-01T00:00:00Z";
const AFTER_ALL: &str = "2100-01-01T00:00:00Z";

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
    /// The login `value` of account 7, read at `location` by `source` at `captured`. A later
    /// capture at the same location replaces the earlier one.
    fn login(&self, source: &str, location: &str, value: &str, captured: &str) {
        self.login_in("personal", source, location, value, captured);
    }
    /// [`Env::login`] read in `scope`.
    fn login_in(&self, scope: &str, source: &str, location: &str, value: &str, captured: &str) {
        let subject = Entity::new(ACCOUNT.into(), "code:Account".into());
        let object = Object::Literal {
            value: value.into(),
            datatype: "login".into(),
        };
        let alias = (claims::ALIAS_PREDICATE, object, "/login", Vec::new());
        self.record((scope, source), &subject, alias, location, captured);
    }
    /// Account `n`, typed `code:Account` with a login alias, read in `scope` by `source`.
    fn account_in(&self, scope: &str, source: &str, n: u32, login: &str) {
        let location = format!("https://api.github.com/users/{n}");
        let subject = Entity::new(format!("github:account/{n}"), "code:Account".into());
        let typed = Object::Literal {
            value: "code:Account".into(),
            datatype: claims::TYPE_DATATYPE.into(),
        };
        let typed = (
            claims::TYPE_PREDICATE,
            typed,
            "/type",
            vec![rastro_core::identity::ACCOUNT_TYPE.to_string()],
        );
        self.record((scope, source), &subject, typed, &location, READ);
        let alias = Object::Literal {
            value: login.into(),
            datatype: "login".into(),
        };
        let alias = (claims::ALIAS_PREDICATE, alias, "/login", Vec::new());
        self.record((scope, source), &subject, alias, &location, READ);
    }
    /// Records one claim of `subject` read at `location` by `source` of `scope` at `captured`.
    fn record(
        &self,
        (scope, source): (&str, &str),
        subject: &Entity,
        (predicate, object, field, ancestors): (&str, Object, &str, Vec<String>),
        location: &str,
        captured: &str,
    ) {
        let captured: DateTime<Utc> = captured.parse().unwrap();
        let source = Source::github(source.into(), "o/p".into(), Scope(scope.into()));
        let quote = match &object {
            Object::Literal { value, .. } => value.clone(),
            Object::Entity { identifier, .. } => identifier.clone(),
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
            field: field.into(),
            start: 0,
            end: quote.len() as u64,
            quote,
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
            .find(|(_, item)| item.location == location && item.field == field)
            .unwrap();
        let observed = ObservedClaim {
            id: claims::claim_id(&subject.id, predicate, &object),
            subject: subject.clone(),
            predicate: predicate.into(),
            object,
            scalar: true,
            type_ancestors: ancestors,
            supports: BTreeSet::from([Support {
                portable_key: claims::portable_evidence_key(&evidence),
                evidence_id,
                field: field.into(),
                start: 0,
                end: evidence.end,
                extraction_rule: "fixture-rule".into(),
            }]),
        };
        db.record_claims(run, (EXTRACTOR, "1"), &[observed], None)
            .unwrap();
    }
    /// The operation error a refused command gives.
    fn refused(&self, args: &[&str]) -> String {
        let out = Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.home.path().join("no-plugins"))
            .args(args)
            .arg("--json")
            .output()
            .unwrap();
        assert!(!out.status.success(), "{args:?} should be refused");
        let value: Value = serde_json::from_slice(&out.stdout).unwrap();
        value["error"].as_str().unwrap().to_string()
    }
    /// The (id, status) of every claim a read scoped to `scope` lists for `entity`, as of `as_of`
    /// when given, sorted.
    fn scoped(&self, scope: &str, entity: &str, as_of: Option<&str>) -> Vec<(String, String)> {
        let mut db = SqliteStorage::open(&self.home.path().join("data/rastro.sqlite3")).unwrap();
        let options = AboutOptions {
            as_of: as_of.map(|at| at.parse().unwrap()),
            ..AboutOptions::default()
        };
        let mut read: Vec<_> = db
            .about_scoped_within(entity, &options, &[Scope(scope.into())], 1_000)
            .unwrap()
            .into_iter()
            .map(|c| (c.claim.id, c.claim.status))
            .collect();
        read.sort();
        read
    }
    /// What `about` lists for account 7, as of `as_of` when given: each alias claim's id and
    /// status, sorted, and the whole answer.
    fn about(&self, as_of: Option<&str>) -> (Vec<(String, String)>, Value) {
        let mut args = vec!["about", ACCOUNT];
        if let Some(at) = as_of {
            args.extend(["--as-of", at]);
        }
        let about = self.ok(&args);
        (listed(&about), about)
    }
    /// The claim `about` lists for account 7's login `value`.
    fn seen(&self, value: &str) -> Value {
        self.about(None).1["claims"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["object"]["value"] == value)
            .unwrap_or_else(|| panic!("{value} is not listed"))
            .clone()
    }
    fn reject(&self, claim: &Value) {
        self.ok(&[
            "claim",
            "reject",
            claim["id"].as_str().unwrap(),
            "--seen",
            claim["revision"].as_str().unwrap(),
        ]);
    }
}
/// The alias claims of an `about` answer as (id, status), sorted.
fn listed(about: &Value) -> Vec<(String, String)> {
    let mut claims: Vec<(String, String)> = about["claims"]
        .as_array()
        .unwrap_or_else(|| panic!("no claims: {about}"))
        .iter()
        .filter(|c| c["predicate"] == claims::ALIAS_PREDICATE)
        .map(|c| {
            (
                c["id"].as_str().unwrap().to_string(),
                c["status"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    claims.sort();
    claims
}

// spec: Reads as of a past time
#[test]
fn a_read_before_a_rejection_lists_the_claim_and_says_it_is_a_reconstruction() {
    let env = Env::new();
    env.login("project", PROJECT, "octocat", READ);
    let seen = env.seen("octocat");
    env.reject(&seen);
    assert!(env.about(None).0.is_empty(), "the rejection hides it today");

    let (claims, about) = env.about(Some(BETWEEN));
    assert_eq!(
        claims,
        [(seen["id"].as_str().unwrap().to_string(), "observed".into())],
        "{about}"
    );
    let reconstruction = &about["reconstruction"];
    assert_eq!(reconstruction["as_of"], "2020-02-01T00:00:00Z", "{about}");
    assert_eq!(
        reconstruction["extractors"],
        serde_json::json!([{
            "extractor": EXTRACTOR,
            "extractor_version": "1",
            "vocabulary_version": ""
        }]),
        "{about}"
    );
    assert!(
        reconstruction["statement"]
            .as_str()
            .unwrap()
            .starts_with("Reconstructed from the evidence captured and the intents issued by"),
        "{about}"
    );
    // A present read is no reconstruction.
    assert!(env.about(None).1.get("reconstruction").is_none());
}

// spec: Reads as of a past time
#[test]
fn a_read_after_everything_equals_todays_read() {
    let env = Env::new();
    env.login("project", PROJECT, "octocat", READ);
    env.login("mirror", MIRROR, "octocat", REREAD);
    env.reject(&env.seen("octocat"));
    // The reading the retraction is bound to is replaced, so the retraction lapses.
    env.login("project", PROJECT, "octocat2", LATER);

    let now = env.about(None).0;
    assert_eq!(
        now.len(),
        2,
        "the lapsed claim and its neighbour list: {now:?}"
    );
    assert_eq!(env.about(Some(AFTER_ALL)).0, now);
}

// spec: Reads as of a past time
#[test]
fn a_source_not_read_yet_is_named_instead_of_no_claims() {
    let env = Env::new();
    env.login("project", PROJECT, "octocat", READ);
    env.login("mirror", MIRROR, "octocat", REREAD);

    let (claims, about) = env.about(Some(BEFORE_ANY));
    assert!(claims.is_empty(), "{about}");
    let unread = |about: &Value| -> Vec<String> {
        about["reconstruction"]["not_read_yet"]
            .as_array()
            .unwrap_or_else(|| panic!("no not_read_yet: {about}"))
            .iter()
            .map(|s| s["source"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(unread(&about), ["mirror", "project"], "{about}");

    // Between the two reads the claim is listed, and only the later source is still unread.
    let (claims, about) = env.about(Some(BETWEEN));
    assert_eq!(claims.len(), 1, "{about}");
    assert_eq!(unread(&about), ["mirror"], "{about}");
    assert_eq!(
        about["reconstruction"]["not_read_yet"][0]["scope"], "personal",
        "{about}"
    );
}

// spec: Reads as of a past time
#[test]
fn why_as_of_explains_the_claim_by_the_evidence_captured_then() {
    let env = Env::new();
    env.login("project", PROJECT, "octocat", READ);
    env.login("mirror", MIRROR, "octocat", REREAD);
    let seen = env.seen("octocat");
    let id = seen["id"].as_str().unwrap();

    let why = env.ok(&["why", id, "--as-of", BETWEEN]);
    let supports = why["supports"].as_array().unwrap();
    // The mirror's reading did not exist yet: one justification cites only the project.
    let sources: BTreeSet<&str> = supports
        .iter()
        .flat_map(|s| s["spans"].as_array().unwrap())
        .map(|span| span["source"].as_str().unwrap())
        .collect();
    assert_eq!(sources, BTreeSet::from(["project"]), "{why}");
    assert_eq!(
        why["reconstruction"]["not_read_yet"][0]["source"], "mirror",
        "{why}"
    );
}

// spec: Reads as of a past time
#[test]
fn a_scoped_read_as_of_leaves_out_what_its_scope_cannot_see() {
    let env = Env::new();
    env.account_in("work", "hub", 8, "hubber");
    let ana = env.ok(&["identity", "person", "create", "--name", "Ana"])["person"]["identifier"]
        .as_str()
        .unwrap()
        .to_string();
    env.ok(&["identity", "link", "github:login/hubber", &ana]);

    let today = env.scoped("personal", &ana, None);
    let past = env.scoped("personal", &ana, Some(AFTER_ALL));
    assert_eq!(
        past, today,
        "a scope reads the same claims as of any time after them"
    );
    // Nothing of the work account's link shows to the personal scope: its claims are the Person's.
    let work = env.scoped("work", &ana, Some(AFTER_ALL));
    assert!(
        work.len() > past.len(),
        "the work scope sees the link: {work:?} {past:?}"
    );
}

// spec: Reads as of a past time
#[test]
fn a_scoped_read_as_of_keeps_a_retraction_where_the_store_keeps_it() {
    let env = Env::new();
    // The retraction binds to the work reading, the claim's primary justification, which the
    // personal scope cannot see; it sees only the later corroboration.
    env.login_in("work", "project", PROJECT, "octocat", READ);
    env.login_in("personal", "mirror", MIRROR, "octocat", REREAD);
    env.reject(&env.seen("octocat"));
    assert!(
        env.about(Some(AFTER_ALL)).0.is_empty(),
        "the store keeps it"
    );

    assert!(
        env.scoped("personal", ACCOUNT, None)
            .iter()
            .all(|(_, status)| status != "observed"),
        "a read today leaves it out"
    );
    let past = env.scoped("personal", ACCOUNT, Some(AFTER_ALL));
    assert!(
        past.iter().all(|(_, status)| status != "observed"),
        "as of after the rejection the scope reads it observed: {past:?}"
    );
}

// spec: Reads as of a past time
#[test]
fn a_claim_not_known_yet_is_told_apart_from_one_that_does_not_exist() {
    let env = Env::new();
    env.login("mirror", MIRROR, "octocat", REREAD);
    let id = env.seen("octocat")["id"].as_str().unwrap().to_string();

    let error = env.refused(&["why", &id, "--as-of", BETWEEN]);
    assert!(error.contains("not known"), "{error}");
    assert!(error.contains("Reconstructed from"), "{error}");
    assert!(error.contains("mirror: not read yet"), "{error}");
    let error = env.refused(&["why", "c_nothing", "--as-of", BETWEEN]);
    assert!(error.contains("unknown claim"), "{error}");
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
            .env("RASTRO_PLUGIN_PATH", env.home.path().join("no-plugins"))
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
                "clientInfo": {"name": "as-of", "version": "1"}}),
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
    /// The scoped `about` tool's answer for account 7 as of `as_of`.
    fn about(&mut self, as_of: &str) -> Value {
        let answer = self.request(
            "tools/call",
            serde_json::json!({"name": "about", "arguments": {"entity": ACCOUNT, "as_of": as_of}}),
        );
        assert_ne!(answer["result"]["isError"], true, "{answer}");
        answer["result"]["structuredContent"]["result"].clone()
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// spec: Reads as of a past time
#[test]
fn the_mcp_about_tool_answers_the_same_claims_and_statuses_as_the_cli() {
    let env = Env::new();
    env.login("project", PROJECT, "octocat", READ);
    env.login("mirror", MIRROR, "octocat", REREAD);
    env.reject(&env.seen("octocat"));
    let mut mcp = Mcp::start(&env);

    for as_of in [BEFORE_ANY, BETWEEN, AFTER_ALL] {
        let (cli, about) = env.about(Some(as_of));
        let answered = mcp.about(as_of);
        assert_eq!(listed(&answered), cli, "as of {as_of}");
        assert_eq!(
            answered["reconstruction"], about["reconstruction"],
            "as of {as_of}"
        );
    }
    assert!(!env.about(Some(BETWEEN)).0.is_empty());
}
