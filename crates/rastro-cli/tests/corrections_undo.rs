#![allow(non_snake_case)]
//! The latest correction of a claim decides what it reads, and `rastro claim undo` removes only
//! the correction it names: a confirm after a reject reads `confirmed`, and undoing a reject
//! issued after a confirm brings the confirmation back. The MCP server lists no claim write.
mod support;
use chrono::{DateTime, Utc};
use rastro_core::claims::{self, Entity, Object, ObservedClaim, Support};
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
    /// The tools the server lists.
    fn tools(&mut self) -> Vec<Value> {
        self.request("tools/list", serde_json::json!({}))["result"]["tools"]
            .as_array()
            .unwrap()
            .clone()
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// spec: The latest intent wins, and undo removes only its own
#[test]
fn a_confirmation_after_a_rejection_reads_confirmed_in_about() {
    let env = Env::new();
    env.account(7, "octocat");
    let seen = env.claim("github:account/7", "rastro:alias", "octocat");
    let id = seen["id"].as_str().unwrap();
    env.reject(&seen);
    // The retracted claim has left `about`; the owner reads it again to confirm it.
    let why = env.ok(&["why", id]);
    assert_eq!(why["claim"]["status"], "retracted", "{why}");
    env.confirm(&why["claim"]);

    let confirmed = env.claim("github:account/7", "rastro:alias", "octocat");
    assert_eq!(confirmed["id"], id);
    assert_eq!(confirmed["status"], "confirmed", "{confirmed}");
}

/// Confirms, then rejects, the alias claim of account 7; returns the claim's id and the ids of
/// the confirmation and the rejection.
fn confirmed_then_rejected(env: &Env) -> (String, String, String) {
    env.account(7, "octocat");
    let seen = env.claim("github:account/7", "rastro:alias", "octocat");
    let id = seen["id"].as_str().unwrap().to_string();
    let confirmed = env.confirm(&seen)["intents"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let rejected = env.reject(&env.claim("github:account/7", "rastro:alias", "octocat"))["intents"]
        [0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    (id, confirmed, rejected)
}

// spec: The latest intent wins, and undo removes only its own
#[test]
fn undoing_a_rejection_issued_after_a_confirmation_restores_the_confirmation_and_why_keeps_the_undone_rejection()
 {
    let env = Env::new();
    let (id, confirmed, rejected) = confirmed_then_rejected(&env);
    assert!(
        env.about("github:account/7")["claims"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["id"] != id.as_str()),
        "the rejection should hide the claim"
    );

    let written = env.ok(&["claim", "undo", &rejected]);
    assert_eq!(written["claim"], id.as_str(), "{written}");
    assert_eq!(written["intents"].as_array().unwrap().len(), 1, "{written}");
    assert_eq!(written["intents"][0]["kind"], "undo", "{written}");
    assert_eq!(written["intents"][0]["undo_of"], rejected.as_str());

    let shown = env.claim("github:account/7", "rastro:alias", "octocat");
    assert_eq!(shown["status"], "confirmed", "{shown}");
    let why = env.ok(&["why", &id]);
    assert_eq!(why["claim"]["status"], "confirmed", "{why}");
    let support = |intent: &str| {
        why["supports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["intent"]["id"] == intent)
            .unwrap_or_else(|| panic!("no support by {intent}: {why}"))
            .clone()
    };
    assert_eq!(support(&confirmed)["current"], true, "{why}");
    // The undone rejection stays in the history, no longer holding.
    assert_eq!(support(&rejected)["current"], false, "{why}");

    // A rebuild replays the undo with the rest: the claim still reads confirmed.
    env.ok(&["claims", "rebuild"]);
    assert_eq!(env.ok(&["why", &id])["claim"]["status"], "confirmed");
}

// spec: The latest intent wins, and undo removes only its own
#[test]
fn undoing_a_fix_brings_back_the_claim_it_replaced_and_drops_the_one_it_asserted() {
    let env = Env::with_vocabulary();
    let pr = env.pull_request();
    let seen = env.claim(pr, "code:state", "open");
    let edit = env.edit(&seen, "closed");
    let edit = edit["intents"][0]["id"].as_str().unwrap();

    env.ok(&["claim", "undo", edit]);
    let about = env.about(pr);
    let states: Vec<_> = about["claims"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["predicate"] == "code:state")
        .map(|c| (c["object"]["value"].clone(), c["status"].clone()))
        .collect();
    assert_eq!(
        states,
        [(serde_json::json!("open"), serde_json::json!("observed"))],
        "{about}"
    );
}

// spec: The latest intent wins, and undo removes only its own
#[test]
fn each_undo_path_refuses_the_other_paths_intents_naming_its_own() {
    let env = Env::new();
    let (_, confirmed, _) = confirmed_then_rejected(&env);
    // Account 7's login is retracted; another account is linked to a Person.
    let other = env.account(8, "hubot");
    let ana = env.person("Ana");
    let link = env.ok(&["identity", "link", &other, &ana])["intents"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let before = env.intents();

    let error = env.refused(&["identity", "undo", &confirmed]);
    assert!(error.contains("claim undo"), "{error}");
    let error = env.refused(&["claim", "undo", &link]);
    assert!(error.contains("identity undo"), "{error}");
    assert_eq!(env.intents(), before, "a refusal records nothing");
}

// spec: Corrections are CLI writes in the catalog
#[test]
fn the_mcp_server_lists_no_correction_write_and_about_and_why_take_as_of() {
    let env = Env::new();
    let mut mcp = Mcp::start(&env);
    let tools = mcp.tools();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(
        names.iter().all(|name| !name.starts_with("claim")),
        "{names:?}"
    );
    for name in ["about", "why"] {
        let tool = tools
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("no {name} tool in {names:?}"));
        assert!(
            tool["inputSchema"]["properties"]["as_of"].is_object(),
            "{name}: {tool}"
        );
    }
    // The four writes exist in the catalog, as CLI-only operations.
    for write in ["claim.confirm", "claim.reject", "claim.edit", "claim.undo"] {
        let spec = rastro_operations::catalog::find(write)
            .unwrap_or_else(|| panic!("{write} is not in the catalog"));
        assert_eq!(spec.surfaces, [rastro_operations::Surface::Cli], "{write}");
        assert!(!spec.read_only, "{write}");
    }
}
