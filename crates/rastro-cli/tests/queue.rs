#![allow(non_snake_case)]
//! The queue end to end: one list of a conversation's proposals and identity's questions, each
//! with a content-addressed id, oldest first, every source's gap named, and launch scopes
//! filtering what a caller sees. The agent tool adapter is a shell script that speaks the plugin
//! protocol and answers with the candidates in `candidates.json`, so no model is ever called.
//! Identity's accounts are seeded straight into the projection.
mod support;
use chrono::{DateTime, Utc};
use rastro_core::claims::{self, Entity, Object, ObservedClaim, Support};
use rastro_core::{Evidence, Scope, Source, Storage, TrustClass};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
};
use tempfile::TempDir;

/// The extractor a seeded source is bound to; it is never installed, so nothing re-reads it.
const MISSING: &str = "missing-extractor";

const TRANSCRIPT: &str = "Alice: I'll send Bob the budget by Friday.\n\
Bob: Thanks. I'll review it next week for you if the numbers are final.\n";

const MANIFEST: &str = r#"id = "test-adapter"
name = "Rastro test agent tool adapter"
version = "0.1.0"
author = "Rastro"
kinds = ["agent-tool-adapter"]
protocol = 1
command = "adapter"
agent_tool = { capabilities = ["judge_text"] }

[permissions]
submit = []
scopes = []
env = []
actions = []
"#;

/// A fake agent tool adapter: it answers the plugin protocol and returns the candidates in
/// `$RASTRO_HOME/candidates.json`.
const ADAPTER: &str = r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocol":"1.0","manifest_id":"test-adapter"}}\n' "$id"
      ;;
    *'"method":"judgment.request"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"candidates":' "$id"
      cat "$RASTRO_HOME/candidates.json"
      printf '}}\n'
      ;;
    *'"method":"run.finish"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"status":"ok","submitted":1}}\n' "$id"
      ;;
  esac
done
"#;

/// A candidate the adapter proposes.
fn candidate(
    turn: u32,
    kind: &str,
    responsible: Option<&str>,
    recipient: Option<&str>,
    promise: &str,
    date: Option<&str>,
    quote: &str,
) -> Value {
    json!({"turn": turn, "kind": kind, "responsible": responsible, "recipient": recipient,
        "promise": promise, "stated_date": date, "quote": quote})
}

/// The two owner-involved commitments of `TRANSCRIPT`: a promise and a waiting-on.
fn good() -> Vec<Value> {
    vec![
        candidate(
            1,
            "promise",
            Some("Alice"),
            Some("Bob"),
            "send Bob the budget",
            Some("by Friday"),
            "I'll send Bob the budget by Friday",
        ),
        candidate(
            2,
            "waiting_on",
            Some("Bob"),
            Some("Alice"),
            "review the numbers",
            Some("next week"),
            "I'll review it next week for you if the numbers are final.",
        ),
    ]
}

fn time() -> DateTime<Utc> {
    "2026-11-01T00:00:00Z".parse().unwrap()
}

struct Env {
    home: TempDir,
}

impl Env {
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
        };
        fs::create_dir_all(env.path("config/plugins/test-adapter")).unwrap();
        fs::write(
            env.path("config/config.toml"),
            "[commitment_adapter]\nplugin = \"test-adapter\"\n",
        )
        .unwrap();
        fs::write(
            env.path("config/plugins/test-adapter/rastro-plugin.toml"),
            MANIFEST,
        )
        .unwrap();
        let adapter = env.path("config/plugins/test-adapter/adapter");
        fs::write(&adapter, ADAPTER).unwrap();
        fs::set_permissions(&adapter, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(env.path("transcript.txt"), TRANSCRIPT).unwrap();
        // Extraction resolves the owner speaker, so the owner's Person must exist.
        env.ok(&["identity", "me"]);
        env
    }

    fn path(&self, name: &str) -> PathBuf {
        self.home.path().join(name)
    }

    /// What the adapter answers with: the candidates the script wraps in its answer.
    fn candidates(&self, candidates: &[Value]) {
        fs::write(self.path("candidates.json"), json!(candidates).to_string()).unwrap();
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(support::rastro());
        command.env("RASTRO_HOME", self.home.path()).args(args);
        command
    }

    fn run(&self, args: &[&str]) -> (bool, Value) {
        let out = self.command(args).arg("--json").output().unwrap();
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

    /// Imports `meeting` from `file`, with the usual arguments plus `extra`.
    fn import_from(
        &self,
        meeting: &str,
        file: &str,
        scope: &str,
        captured_at: &str,
        extra: &[&str],
    ) -> (bool, Value) {
        let file = self.path(file);
        let mut args = vec![
            "meeting",
            "import",
            file.to_str().unwrap(),
            "--meeting",
            meeting,
            "--scope",
            scope,
            "--captured-at",
            captured_at,
        ];
        args.extend_from_slice(extra);
        let (ok, value) = self.run(&args);
        (ok, value["result"].clone())
    }

    fn import(
        &self,
        meeting: &str,
        scope: &str,
        captured_at: &str,
        extra: &[&str],
    ) -> (bool, Value) {
        self.import_from(meeting, "transcript.txt", scope, captured_at, extra)
    }

    fn import_ok(&self, meeting: &str, scope: &str, captured_at: &str, extra: &[&str]) -> Value {
        let (ok, result) = self.import(meeting, scope, captured_at, extra);
        assert!(ok, "{result}");
        result
    }

    /// Reads the queue on the CLI.
    fn queue(&self) -> Value {
        self.ok(&["queue"])
    }

    /// Calls one MCP tool on a server limited to `scopes`.
    fn mcp(&self, scopes: &[&str], tool: &str, arguments: Value) -> Value {
        let mut command = self.command(&["mcp"]);
        for scope in scopes {
            command.args(["--scope", scope]);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut send = |message: Value| writeln!(stdin, "{message}").unwrap();
        send(
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": {"name": "test", "version": "1"}}}),
        );
        lines.next().unwrap().unwrap();
        send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": tool, "arguments": arguments}}));
        let reply: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        drop(stdin);
        let _ = child.wait();
        reply["result"].clone()
    }

    fn database(&self) -> PathBuf {
        self.path("data/rastro.sqlite3")
    }

    /// Records one claim, read from `location` and `field` by source `project` (scope
    /// `personal`), bound to an extractor that isn't installed.
    fn record(&self, subject: &Entity, predicate: &str, object: Object, at: (&str, &str)) {
        self.record_at(subject, predicate, object, at, time());
    }

    fn record_at(
        &self,
        subject: &Entity,
        predicate: &str,
        object: Object,
        at: (&str, &str),
        captured: DateTime<Utc>,
    ) {
        let (location, field) = at;
        let source = Source::github("project".into(), "o/p".into(), Scope("personal".into()));
        let mut db = rastro_storage_sqlite::SqliteStorage::open(&self.database()).unwrap();
        db.project_sources(std::slice::from_ref(&source)).unwrap();
        db.bind_extractor(&source.name, Some(MISSING)).unwrap();
        let run = db
            .start_run(&source, (MISSING, "1"), time(), (time(), time()))
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
            captured_at: captured,
            source_updated_at: Some(captured),
            original: None,
        };
        db.append_evidence(run, &e).unwrap();
        let (evidence_id, _) = db
            .pending_evidence((MISSING, "1"), &source)
            .unwrap()
            .into_iter()
            .rev()
            .find(|(_, item)| item.location == location && item.field == field)
            .unwrap();
        // `code:Account` and `tracker:Account` are accounts: their type claims carry the core's
        // supertype, as their vocabularies declare.
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
        db.record_claims(run, (MISSING, "1"), &[observed], None)
            .unwrap();
    }

    /// An account `identifier` typed `kind`, with one alias, read at `location`.
    fn typed_account(&self, identifier: &str, kind: &str, alias: (&str, &str), location: &str) {
        let subject = Entity::new(identifier.into(), kind.into());
        let typed = Object::Literal {
            value: kind.into(),
            datatype: claims::TYPE_DATATYPE.into(),
        };
        self.record(&subject, claims::TYPE_PREDICATE, typed, (location, "/type"));
        let (alias_kind, value) = alias;
        let alias = Object::Literal {
            value: value.into(),
            datatype: alias_kind.into(),
        };
        let field = format!("/{alias_kind}");
        self.record(&subject, claims::ALIAS_PREDICATE, alias, (location, &field));
    }
}

/// The ids of the queue's items, in order.
fn ids(queue: &Value) -> Vec<String> {
    queue["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_string())
        .collect()
}

/// The item whose source is the proposal source `name`.
fn item<'a>(queue: &'a Value, name: &str) -> &'a Value {
    queue["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["source"]["name"] == name)
        .unwrap_or_else(|| panic!("no item from {name}: {queue}"))
}

/// The gap a source names.
fn gap<'a>(queue: &'a Value, name: &str) -> &'a Value {
    queue["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|gap| gap["source"]["name"] == name)
        .unwrap_or_else(|| panic!("no gap for {name}: {queue}"))
}

/// S1: a meeting's hypotheses are the queue's proposals, each with a content-addressed id, its
/// source, kind, promise, parties, stated date, quote and terms, oldest first, and reading twice
/// returns the same ids in the same order.
#[test]
fn S1_a_meetings_hypotheses_are_proposals_with_stable_ids_oldest_first() {
    let env = Env::new();
    let me = env.ok(&["identity", "me"])["person"]["identifier"]
        .as_str()
        .unwrap()
        .to_string();
    env.candidates(&good());
    // Imported newest first, so the order proves the read is by the meetings' time, not by import.
    env.import_ok(
        "later-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    env.import_ok(
        "earlier-1",
        "work",
        "2026-11-01T09:00:00Z",
        &["--owner-speaker", "Alice"],
    );

    let queue = env.queue();
    let items = queue["items"].as_array().unwrap();
    assert_eq!(items.len(), 4, "{queue}");
    let names: Vec<&str> = queue["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["source"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["earlier-1", "earlier-1", "later-1", "later-1"],
        "{queue}"
    );
    assert_eq!(queue["gaps"], json!([]), "{queue}");

    let proposal = item(&queue, "earlier-1");
    assert_eq!(proposal["kind"], "proposal", "{proposal}");
    assert_eq!(
        proposal["source"],
        json!({"kind": "meeting", "name": "earlier-1", "scope": "work"}),
        "{proposal}"
    );
    let id = proposal["id"].as_str().unwrap();
    assert!(id.starts_with("q_"), "{proposal}");
    let content = &proposal["proposal"];
    assert_eq!(content["kind"], "promise", "{content}");
    assert_eq!(content["promise"], "send Bob the budget", "{content}");
    assert_eq!(
        content["stated_date"],
        json!({"state": "stated", "text": "by Friday"}),
        "{content}"
    );
    assert_eq!(content["responsible"]["state"], "mapped", "{content}");
    assert_eq!(
        content["responsible"]["identity"]["identifier"], me,
        "{content}"
    );
    assert_eq!(content["recipient"]["state"], "unknown", "{content}");
    assert_eq!(content["recipient"]["label"], "Bob", "{content}");
    assert_eq!(content["quote"]["turn"], 1, "{content}");
    assert_eq!(content["quote"]["speaker"], "Alice", "{content}");
    assert_eq!(
        content["quote"]["text"], "I'll send Bob the budget by Friday",
        "{content}"
    );
    assert_eq!(
        content["terms"],
        json!(["std:owes", "std:owed_to", "std:due"]),
        "{content}"
    );

    // Reading twice returns the same ids in the same order.
    let again = env.queue();
    assert_eq!(ids(&queue), ids(&again));

    // Content-addressed: the same proposal re-imported keeps its id.
    env.import_ok(
        "earlier-1",
        "work",
        "2026-11-01T09:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    let reimported = env.queue();
    assert!(ids(&reimported).contains(&id.to_string()), "{reimported}");
    assert_eq!(
        reimported["items"].as_array().unwrap().len(),
        4,
        "{reimported}"
    );
}

/// S2: identity's same-person candidates are the queue's questions, each with the same kind of
/// content-addressed id, its two accounts with their labels and the signals that produced it,
/// and no candidate appears twice.
#[test]
fn S2_identity_candidates_are_questions_with_labels_and_no_duplicate() {
    let env = Env::new();
    env.typed_account(
        "github:account/1",
        "code:Account",
        ("name", "Ana Silva"),
        "https://api.github.com/users/1",
    );
    env.typed_account(
        "github:account/2",
        "code:Account",
        ("name", "Ana Silva"),
        "https://api.github.com/users/2",
    );
    env.typed_account(
        "linear:user/u-1",
        "tracker:Account",
        ("name", "ana silva"),
        "https://linear.app/rastro/profiles/u-1",
    );

    let queue = env.queue();
    let items = queue["items"].as_array().unwrap();
    // The two GitHub accounts are one namespace, so they never pair; each pairs with Linear.
    assert_eq!(items.len(), 2, "{queue}");
    let mut seen = BTreeSet::new();
    for item in items {
        assert_eq!(item["kind"], "question", "{item}");
        assert_eq!(item["source"]["kind"], "identity", "{item}");
        assert_eq!(item["source"]["name"], "identity", "{item}");
        assert!(item["id"].as_str().unwrap().starts_with("q_"), "{item}");
        // No candidate appears twice.
        assert!(
            seen.insert(item["id"].as_str().unwrap().to_string()),
            "{item}"
        );
        assert!(item.get("proposal").is_none(), "{item}");

        let question = &item["question"];
        let accounts = question["accounts"].as_array().unwrap();
        assert_eq!(accounts.len(), 2, "{question}");
        let linear = accounts
            .iter()
            .find(|account| account["identifier"] == "linear:user/u-1")
            .unwrap_or_else(|| panic!("no linear account: {question}"));
        assert_eq!(linear["names"], json!(["ana silva"]), "{question}");
        let github = accounts
            .iter()
            .find(|account| {
                account["identifier"]
                    .as_str()
                    .unwrap()
                    .starts_with("github:")
            })
            .unwrap_or_else(|| panic!("no github account: {question}"));
        assert_eq!(github["names"], json!(["Ana Silva"]), "{question}");
        assert_eq!(
            question["reasons"],
            json!([{"reason": "same name", "values": ["Ana Silva", "ana silva"]}]),
            "{question}"
        );
        assert_eq!(question["name"], "Ana Silva", "{question}");
    }

    // The same content-addressed id on a second read.
    assert_eq!(ids(&queue), ids(&env.queue()));
}

/// S3: a source that found nothing, a failed run and a source that couldn't look are named with
/// their gap, a source hidden by a launch scope leaves the list, and the read still succeeds.
#[test]
fn S3_every_gap_is_named_and_a_hidden_sources_items_are_absent() {
    let env = Env::new();
    // Found nothing.
    env.candidates(&[]);
    env.import_ok(
        "empty-1",
        "work",
        "2026-11-04T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    // A failed run: the adapter's answer is not the response shape.
    fs::write(env.path("candidates.json"), "not json").unwrap();
    let (ok, broken) = env.import(
        "broken-1",
        "work",
        "2026-11-05T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    assert!(!ok, "{broken}");
    // Couldn't look: the transcript can't be read.
    let (ok, unreadable) = env.import_from(
        "missing-1",
        "no-such-transcript.txt",
        "work",
        "2026-11-06T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    assert!(!ok, "{unreadable}");
    // Two good meetings in different scopes, so a launch scope can hide one.
    env.candidates(&good());
    env.import_ok(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    env.import_ok(
        "private-1",
        "personal",
        "2026-11-11T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );

    let queue = env.queue();
    assert_eq!(gap(&queue, "empty-1")["state"], "found_nothing", "{queue}");
    assert!(gap(&queue, "empty-1")["run"].is_string(), "{queue}");
    let failed = gap(&queue, "broken-1");
    assert_eq!(failed["state"], "failed", "{queue}");
    assert_eq!(failed["cause"], "extractor_output_invalid", "{queue}");
    assert!(failed["reason"].is_string(), "{queue}");
    let could_not_look = gap(&queue, "missing-1");
    assert_eq!(could_not_look["state"], "could_not_look", "{queue}");
    assert!(
        could_not_look["reason"]
            .as_str()
            .unwrap()
            .contains("can't read"),
        "{queue}"
    );
    // Every source's items are present in the unscoped read.
    let names: Vec<&str> = queue["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["source"]["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"weekly-1") && names.contains(&"private-1"),
        "{queue}"
    );

    // A launch scope hides the other scope's source and its items, and the read still succeeds.
    let work = env.mcp(&["work"], "queue", json!({}));
    let shown = &work["structuredContent"]["result"];
    let names: Vec<&str> = shown["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["source"]["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"weekly-1"), "{work}");
    assert!(!names.contains(&"private-1"), "{work}");
    let gapped: Vec<&str> = shown["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|gap| gap["source"]["name"].as_str().unwrap())
        .collect();
    assert!(!gapped.contains(&"private-1"), "{work}");

    let personal = env.mcp(&["personal"], "queue", json!({}));
    let shown = &personal["structuredContent"]["result"];
    let names: Vec<&str> = shown["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["source"]["name"].as_str().unwrap())
        .collect();
    assert!(!names.contains(&"weekly-1"), "{personal}");
    assert!(names.contains(&"private-1"), "{personal}");
}
