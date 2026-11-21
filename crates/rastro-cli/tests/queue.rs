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
    collections::{BTreeMap, BTreeSet},
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
enforcement = ["tools_off", "no_mcp_servers", "no_setting_sources"]

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
        self.run_at(args, None)
    }

    /// Runs `args` with the core's clock at `at`, so a test can date what it shows.
    fn run_at(&self, args: &[&str], at: Option<&str>) -> (bool, Value) {
        let mut command = self.command(args);
        if let Some(at) = at {
            command.env("RASTRO_NOW", at);
        }
        let out = command.arg("--json").output().unwrap();
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
        self.ok_at(args, None)
    }

    fn ok_at(&self, args: &[&str], at: Option<&str>) -> Value {
        let (ok, value) = self.run_at(args, at);
        assert!(ok, "{args:?}: {value}");
        value["result"].clone()
    }

    /// The human queue read the owner prints: what it prints is what it showed.
    fn print_queue_at(&self, at: &str) -> String {
        let out = self
            .command(&["queue"])
            .env("RASTRO_NOW", at)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// Every outcome the owner reads, newest first.
    fn outcomes_at(&self, at: &str) -> Value {
        self.ok_at(&["queue", "outcomes"], Some(at))
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
        self.import_from_at(meeting, file, scope, captured_at, extra, None)
    }

    /// Imports `meeting`, reading the clock at `at` so its run is dated.
    fn import_from_at(
        &self,
        meeting: &str,
        file: &str,
        scope: &str,
        captured_at: &str,
        extra: &[&str],
        at: Option<&str>,
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
        let (ok, value) = self.run_at(&args, at);
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

    /// Imports `meeting`, reading the clock at `at` as its run's finish.
    fn import_ok_at(
        &self,
        meeting: &str,
        scope: &str,
        captured_at: &str,
        extra: &[&str],
        at: &str,
    ) -> Value {
        let (ok, result) = self.import_from_at(
            meeting,
            "transcript.txt",
            scope,
            captured_at,
            extra,
            Some(at),
        );
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

    fn sql<T: rusqlite::types::FromSql>(&self, query: &str) -> T {
        rusqlite::Connection::open(self.database())
            .unwrap()
            .query_row(query, [], |row| row.get(0))
            .unwrap()
    }

    /// How many owner intents are recorded.
    fn intents(&self) -> i64 {
        self.sql("SELECT count(*) FROM intent")
    }

    /// Every claim the owner intent backs in the projection, in id order: an accept mints one
    /// claim per standard term of the item it accepted.
    fn claims_of_intent(&self, intent: &str) -> Vec<String> {
        let connection = rusqlite::Connection::open(self.database()).unwrap();
        let mut statement = connection
            .prepare(
                "SELECT claim_id FROM intent_justification WHERE intent_id = ?1 ORDER BY claim_id",
            )
            .unwrap();
        let rows = statement.query_map([intent], |row| row.get(0)).unwrap();
        rows.collect::<Result<_, _>>().unwrap()
    }

    /// How many claims the projection holds.
    fn claims(&self) -> i64 {
        self.sql("SELECT count(*) FROM claim")
    }

    /// The newest owner intent, as the insert-only table stores it.
    fn last_intent(&self) -> Value {
        let recorded: String = self.sql(
            "SELECT json_object('id', id, 'kind', kind, 'domain', domain_key,\n\
             'targets', json(targets_json), 'payload', json(payload_json))\n\
             FROM intent ORDER BY append_seq DESC LIMIT 1",
        );
        serde_json::from_str(&recorded).unwrap()
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

/// Records one later `name` intent for `person` and returns its id. No CLI operation names an
/// existing Person yet, so the test seeds the intent a naming surface would record.
fn name_person(env: &Env, person: &Value, name: &str) -> String {
    use rastro_core::identity::{
        EntityTarget, FORMAT_VERSION, Intent, IntentKind, IssuerContext, OWNER, Payload,
        SHAPE_VERSION, Targets,
    };
    let mut intent = Intent {
        id: "i_test_name".into(),
        append_seq: 0,
        format_version: FORMAT_VERSION,
        kind: IntentKind::Name,
        owner_id: OWNER.into(),
        issuer_context: IssuerContext::owner_cli(&[]),
        issued_at: Utc::now(),
        domain_key: String::new(),
        targets: Targets::Entity {
            version: SHAPE_VERSION,
            entity: EntityTarget {
                id: person["id"].as_str().unwrap().into(),
                identifier: person["identifier"].as_str().unwrap().into(),
                role: "person".into(),
            },
        },
        payload: Payload {
            version: SHAPE_VERSION,
            name: Some(name.into()),
        },
        undo_of: None,
        supersedes: None,
    };
    intent.domain_key = intent.expected_domain().unwrap_or_default();
    let mut db = rastro_storage_sqlite::SqliteStorage::open(&env.database()).unwrap();
    db.issue_intents(&[], |_| Ok((vec![intent], ()))).unwrap();
    "i_test_name".into()
}

/// The question item whose accounts include `identifier`.
fn question<'a>(queue: &'a Value, identifier: &str) -> &'a Value {
    queue["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["kind"] == "question"
                && item["question"]["accounts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|account| account["identifier"] == identifier)
        })
        .unwrap_or_else(|| panic!("no question holding {identifier}: {queue}"))
}

/// The `std:` claims an `about` answer shows, as (subject id, predicate, literal value): the
/// standard statements one or more commitments already hold.
fn commitment_statements(about: &Value) -> BTreeSet<(String, String, String)> {
    (about["claims"].as_array().into_iter().flatten())
        .filter(|claim| {
            claim["predicate"]
                .as_str()
                .is_some_and(|predicate| predicate.starts_with("std:"))
        })
        .map(|claim| {
            (
                claim["subject"]["id"].as_str().unwrap().to_string(),
                claim["predicate"].as_str().unwrap().to_string(),
                claim["object"]["value"].as_str().unwrap().to_string(),
            )
        })
        .collect()
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

/// spec: One queue of proposals and questions
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

/// spec: One queue of proposals and questions
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

    // A question has no time, so it sorts before every proposal and its order is its accounts'
    // identifiers, not its hash.
    let pairs: Vec<(String, String)> = (items.iter())
        .map(|item| {
            let accounts = item["question"]["accounts"].as_array().unwrap();
            (
                accounts[0]["identifier"].as_str().unwrap().to_string(),
                accounts[1]["identifier"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        pairs,
        [
            (
                "github:account/1".to_string(),
                "linear:user/u-1".to_string()
            ),
            (
                "github:account/2".to_string(),
                "linear:user/u-1".to_string()
            ),
        ],
        "{queue}"
    );

    // The same content-addressed id on a second read.
    assert_eq!(ids(&queue), ids(&env.queue()));
}

/// spec: One queue of proposals and questions
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

/// A literal claim object, as an extractor submits the text a source states.
fn literal(value: &str) -> Object {
    Object::Literal {
        value: value.into(),
        datatype: "string".into(),
    }
}

/// spec: Everything is checked against what is known
/// S4: an item whose standard statement an observed claim already holds is absent from the queue
/// and the result counts it as already known; a claim matching only a name or the promise's text
/// drops nothing.
#[test]
fn S4_an_item_an_observed_claim_already_holds_is_omitted_and_counted() {
    let env = Env::new();
    env.candidates(&good());
    env.import_ok(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    let queue = env.queue();
    assert_eq!(queue["items"].as_array().unwrap().len(), 2, "{queue}");
    assert_eq!(queue["already_known"], 0, "{queue}");

    // Look-alikes that hold no more than a part of a statement: a title repeating the promise's
    // words, a tracker field repeating a party's name, and the promise's two party names and its
    // date each on a different commitment, so no one commitment holds the item's statement.
    let issue = Entity::new("tracker:issue/1".into(), "tracker:Issue".into());
    env.record(
        &issue,
        "tracker:title",
        literal("send Bob the budget"),
        ("https://tracker.test/issue/1", "/title"),
    );
    env.record(
        &issue,
        "tracker:assignee",
        literal("Alice"),
        ("https://tracker.test/issue/1", "/assignee"),
    );
    let owes = Entity::new("std:commitment/owes".into(), "std:Commitment".into());
    env.record(
        &owes,
        "std:owes",
        literal("Alice"),
        ("https://meetings.test/other", "/owes"),
    );
    let owed_to = Entity::new("std:commitment/owed-to".into(), "std:Commitment".into());
    env.record(
        &owed_to,
        "std:owed_to",
        literal("Bob"),
        ("https://meetings.test/other", "/owed_to"),
    );
    let due = Entity::new("std:commitment/due".into(), "std:Commitment".into());
    env.record(
        &due,
        "std:due",
        literal("by Friday"),
        ("https://meetings.test/other", "/due"),
    );
    let queue = env.queue();
    assert_eq!(queue["items"].as_array().unwrap().len(), 2, "{queue}");
    assert_eq!(queue["already_known"], 0, "{queue}");

    // Observed claims hold the first proposal's statements: the same standard predicates, party
    // names and stated date.
    let commitment = Entity::new("std:commitment/1".into(), "std:Commitment".into());
    env.record(
        &commitment,
        "std:owes",
        literal("Alice"),
        ("https://meetings.test/weekly-1", "/owes"),
    );
    env.record(
        &commitment,
        "std:owed_to",
        literal("Bob"),
        ("https://meetings.test/weekly-1", "/owed_to"),
    );
    env.record(
        &commitment,
        "std:due",
        literal("by Friday"),
        ("https://meetings.test/weekly-1", "/due"),
    );

    let queue = env.queue();
    let items = queue["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{queue}");
    assert_eq!(
        items[0]["proposal"]["kind"], "waiting_on",
        "the promise is known, the waiting-on is not: {queue}"
    );
    assert_eq!(queue["already_known"], 1, "{queue}");
}

/// spec: Everything is checked against what is known
/// S5: a proposal naming a party the owner mapped to an entity that has visible claims carries
/// those claims as context and stays in the queue.
#[test]
fn S5_a_mapped_partys_visible_claims_are_the_items_context() {
    let env = Env::new();
    let me = env.ok(&["identity", "me"])["person"]["identifier"]
        .as_str()
        .unwrap()
        .to_string();
    let person = Entity::new(me.clone(), "rastro:Person".into());
    let owns = literal("the budget");
    env.record(
        &person,
        "note:owns",
        owns.clone(),
        ("https://contacts.test/owner", "/owns"),
    );

    env.candidates(&good());
    env.import_ok(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );

    let queue = env.queue();
    assert_eq!(queue["items"].as_array().unwrap().len(), 2, "{queue}");
    let promise = item(&queue, "weekly-1");
    assert_eq!(
        promise["proposal"]["responsible"]["identity"]["identifier"], me,
        "{promise}"
    );
    let context = promise["context"]
        .as_array()
        .unwrap_or_else(|| panic!("the mapped party brings no context: {promise}"));
    let held = context
        .iter()
        .find(|claim| claim["id"] == claims::claim_id(&person.id, "note:owns", &owns))
        .unwrap_or_else(|| panic!("the party's visible claim is not context: {promise}"));
    assert_eq!(held["subject"]["id"], person.id, "{promise}");
    assert_eq!(held["object"]["value"], "the budget", "{promise}");
    assert_eq!(held["status"], "observed", "{promise}");
    // The item stays in the queue, and every claim appears once.
    let ids: BTreeSet<&str> = context
        .iter()
        .map(|claim| claim["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), context.len(), "{promise}");
}

/// spec: One intent per answer, recorded once
/// S6: a rejection with a reason and a yes or no answer each record one owner intent, the item
/// leaves the queue on the CLI and on an MCP read, a rejection mints no claim, and a yes still
/// creates and names the Person that holds a pair nobody held.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the rejection and the answer each travel the CLI and the MCP read on one seeded queue, so both outcomes stay beside their intents"
)]
fn S6_a_rejection_and_an_answer_record_one_intent_and_leave_every_surface() {
    let env = Env::new();
    env.candidates(&good());
    env.import_ok(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    env.typed_account(
        "github:account/1",
        "code:Account",
        ("name", "Ana Silva"),
        "https://api.github.com/users/1",
    );
    env.typed_account(
        "linear:user/u-1",
        "tracker:Account",
        ("name", "ana silva"),
        "https://linear.app/rastro/profiles/u-1",
    );
    env.typed_account(
        "github:account/2",
        "code:Account",
        ("name", "Bob Jones"),
        "https://api.github.com/users/2",
    );
    env.typed_account(
        "linear:user/u-2",
        "tracker:Account",
        ("name", "bob jones"),
        "https://linear.app/rastro/profiles/u-2",
    );

    let queue = env.queue();
    let proposal = item(&queue, "weekly-1")["id"].as_str().unwrap().to_string();
    let ana = question(&queue, "github:account/1")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let bob = question(&queue, "github:account/2")["id"]
        .as_str()
        .unwrap()
        .to_string();

    // A rejection: one intent, no claim, and the item leaves the queue on both surfaces.
    let claims = env.claims();
    let intents = env.intents();
    let rejected = env.ok(&[
        "queue",
        "reject",
        &proposal,
        "--reason",
        "Alice never promised that",
    ]);
    assert_eq!(rejected["item"], proposal, "{rejected}");
    let recorded = rejected["intents"].as_array().unwrap();
    assert_eq!(recorded.len(), 1, "{rejected}");
    assert_eq!(recorded[0]["kind"], "reject", "{rejected}");
    assert_eq!(env.intents(), intents + 1, "{rejected}");
    assert_eq!(env.claims(), claims, "a rejection mints no claim");
    assert!(!ids(&env.queue()).contains(&proposal), "still on the CLI");
    let read = env.mcp(&[], "queue", json!({}));
    assert!(
        !ids(&read["structuredContent"]["result"]).contains(&proposal),
        "still on MCP: {read}"
    );

    // A yes on a pair no Person holds yet: still one intent, and it is the answer itself.
    let claims = env.claims();
    let before = env.intents();
    let answered = env.ok(&["queue", "answer", &bob, "yes"]);
    let recorded = answered["intents"].as_array().unwrap();
    assert_eq!(recorded.len(), 1, "{answered}");
    assert_eq!(recorded[0]["kind"], "answer", "{answered}");
    assert_eq!(env.intents(), before + 1, "{answered}");
    assert!(!ids(&env.queue()).contains(&bob), "still queued");
    let read = env.mcp(&[], "queue", json!({}));
    assert!(
        !ids(&read["structuredContent"]["result"]).contains(&bob),
        "still on MCP: {read}"
    );
    // The one answer decided the whole question: its Person is named, typed and holds both
    // accounts, and those claims are the answer's.
    assert!(env.claims() > claims, "a yes mints the claims it decides");
    let person = env.ok(&["about", "rastro:name/Bob Jones"]);
    let accounts: Vec<&str> = (person["person"]["accounts"].as_array().unwrap().iter())
        .map(|account| account["identifier"].as_str().unwrap())
        .collect();
    assert!(
        accounts.contains(&"github:account/2") && accounts.contains(&"linear:user/u-2"),
        "{person}"
    );
    let held: Vec<&str> = (person["person"]["accounts"].as_array().unwrap().iter())
        .map(|account| account["reasons"][0]["intent"].as_str().unwrap())
        .collect();
    assert_eq!(held.len(), 2, "{person}");
    assert_eq!(
        held,
        [recorded[0]["id"].as_str().unwrap(); 2],
        "both held-by claims are the one answer's: {person}"
    );

    // A no: still one intent, and the question leaves too.
    let claims = env.claims();
    let before = env.intents();
    let answered = env.ok(&["queue", "answer", &ana, "no"]);
    let recorded = answered["intents"].as_array().unwrap();
    assert_eq!(recorded.len(), 1, "{answered}");
    assert_eq!(recorded[0]["kind"], "answer", "{answered}");
    assert_eq!(env.intents(), before + 1, "{answered}");
    assert!(!ids(&env.queue()).contains(&ana), "still queued");
    assert_eq!(env.claims(), claims, "a no mints no claim");
    let read = env.mcp(&[], "queue", json!({}));
    assert!(
        !ids(&read["structuredContent"]["result"]).contains(&ana),
        "still on MCP: {read}"
    );
}

/// spec: One intent per answer, recorded once
/// Undoing a yes that was answered in the queue ends what it decided — the accounts it held and
/// the name it gave — and the question waits again.
#[test]
fn undoing_a_queues_yes_ends_its_decision_and_returns_the_question() {
    let env = Env::new();
    env.typed_account(
        "github:account/7",
        "code:Account",
        ("name", "Carla Dias"),
        "https://api.github.com/users/7",
    );
    env.typed_account(
        "linear:user/u-7",
        "tracker:Account",
        ("name", "carla dias"),
        "https://linear.app/rastro/profiles/u-7",
    );
    let queue = env.queue();
    let item = question(&queue, "github:account/7")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let answered = env.ok(&["queue", "answer", &item, "yes"]);
    let answer = answered["intents"][0]["id"].as_str().unwrap().to_string();
    assert!(!ids(&env.queue()).contains(&item), "{answered}");

    // Undoing the answer is refused by nothing: it ends the answer's decision.
    let undone = env.ok(&["identity", "undo", &answer]);
    assert_eq!(undone["intents"][0]["kind"], "undo", "{undone}");
    assert_eq!(undone["intents"][0]["status"], "effective", "{undone}");
    let history = env.ok(&["identity", "history", "github:account/7"]);
    let answer_row = (history["intents"].as_array().unwrap().iter())
        .find(|intent| intent["id"] == answer.as_str())
        .unwrap_or_else(|| panic!("the answer is not in the account's history: {history}"));
    assert_eq!(answer_row["status"], "undone", "{history}");

    // The question waits again, and no account is held now.
    assert!(
        ids(&env.queue()).contains(&item),
        "the question returns: {}",
        env.queue()
    );
    let about = env.ok(&["about", "github:account/7"]);
    let held = (about["claims"].as_array().unwrap().iter())
        .filter(|claim| claim["predicate"] == "rastro:held_by")
        .count();
    assert_eq!(held, 0, "no current held-by claim: {about}");
    // It ends the claim; it does not erase that the account was held until the undo.
    let history = env.ok(&["about", "github:account/7", "--history"]);
    let ended = (history["claims"].as_array().unwrap().iter())
        .find(|claim| claim["predicate"] == "rastro:held_by")
        .unwrap_or_else(|| panic!("the held-by claim is gone, not ended: {history}"));
    assert_eq!(ended["status"], "superseded", "{history}");
    let known = (ended["known_intervals"].as_array().unwrap().iter())
        .any(|interval| interval["end"].is_string());
    assert!(known, "the claim ends at the undo: {history}");
}

/// The answer records the accounts it links when it is written: undoing an earlier link on one of
/// them never re-decides the answer on replay.
#[test]
fn an_answers_decision_survives_undoing_an_earlier_link() {
    let env = Env::new();
    env.typed_account(
        "github:account/1",
        "code:Account",
        ("name", "Ana Silva"),
        "https://api.github.com/users/1",
    );
    env.typed_account(
        "linear:user/u-1",
        "tracker:Account",
        ("name", "ana silva"),
        "https://linear.app/rastro/profiles/u-1",
    );
    // The first account is the owner's, so a yes links only the second.
    let linked = env.ok(&["identity", "link", "github:account/1", "me"]);
    let link = linked["intents"][0]["id"].as_str().unwrap().to_string();
    let item = question(&env.queue(), "github:account/1")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let answered = env.ok(&["queue", "answer", &item, "yes"]);
    assert_eq!(
        answered["intents"].as_array().unwrap().len(),
        1,
        "{answered}"
    );

    // Undoing the earlier link leaves the answer's decision as it was written: one account.
    env.ok(&["identity", "undo", &link]);
    let me = env.ok(&["about", "me"]);
    let accounts: Vec<&str> = (me["person"]["accounts"].as_array().unwrap().iter())
        .map(|account| account["identifier"].as_str().unwrap())
        .collect();
    assert_eq!(
        accounts,
        ["linear:user/u-1"],
        "the answer still links only the account it recorded: {me}"
    );
}

/// A yes that created a Person can't be undone while a later intent holds or merges that Person:
/// the undo would leave a held-by claim pointing at something that is no longer a Person.
#[test]
fn an_answer_that_created_a_person_is_kept_while_later_intents_depend_on_it() {
    let env = Env::new();
    env.typed_account(
        "github:account/9",
        "code:Account",
        ("name", "Dora Melo"),
        "https://api.github.com/users/9",
    );
    env.typed_account(
        "linear:user/u-9",
        "tracker:Account",
        ("name", "dora melo"),
        "https://linear.app/rastro/profiles/u-9",
    );
    env.typed_account(
        "github:account/10",
        "code:Account",
        ("name", "Dora Work"),
        "https://api.github.com/users/10",
    );
    let item = question(&env.queue(), "github:account/9")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let answered = env.ok(&["queue", "answer", &item, "yes"]);
    let answer = answered["intents"][0]["id"].as_str().unwrap().to_string();
    // A later link holds a third account by the Person the answer created.
    let person = env.ok(&["about", "rastro:name/Dora Melo"]);
    let person = person["person"]["identifier"].as_str().unwrap().to_string();
    env.ok(&["identity", "link", "github:account/10", &person]);

    let link = env.last_intent()["id"].as_str().unwrap().to_string();
    let intents = env.intents();
    let (ok, refused) = env.run(&["identity", "undo", &answer]);
    assert!(!ok, "{refused}");
    assert_eq!(
        refused["error"].as_str().unwrap(),
        format!("{person} is held, named or merged by intent {link}; undo that first"),
        "the refusal reads as one sentence: {refused}"
    );
    assert_eq!(env.intents(), intents, "nothing is recorded");
    let me = env.ok(&["about", &person]);
    let accounts: Vec<&str> = (me["person"]["accounts"].as_array().unwrap().iter())
        .map(|account| account["identifier"].as_str().unwrap())
        .collect();
    assert_eq!(
        accounts,
        ["github:account/10", "github:account/9", "linear:user/u-9"],
        "the Person still holds what it did: {me}"
    );
}

/// A yes that created a Person can't be undone while a later name intent targets that Person:
/// the undo would leave a name claim on an entity that is no longer a Person.
#[test]
fn an_answer_that_created_a_person_is_kept_while_a_later_name_targets_it() {
    let env = Env::new();
    env.typed_account(
        "github:account/11",
        "code:Account",
        ("name", "Dora Melo"),
        "https://api.github.com/users/11",
    );
    env.typed_account(
        "linear:user/u-11",
        "tracker:Account",
        ("name", "dora melo"),
        "https://linear.app/rastro/profiles/u-11",
    );
    let item = question(&env.queue(), "github:account/11")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let answered = env.ok(&["queue", "answer", &item, "yes"]);
    let answer = answered["intents"][0]["id"].as_str().unwrap().to_string();
    let person = env.ok(&["about", "rastro:name/Dora Melo"])["person"].clone();
    let name = name_person(&env, &person, "Dora M.");

    let intents = env.intents();
    let (ok, refused) = env.run(&["identity", "undo", &answer]);
    assert!(!ok, "{refused}");
    let identifier = person["identifier"].as_str().unwrap();
    assert_eq!(
        refused["error"].as_str().unwrap(),
        format!("{identifier} is held, named or merged by intent {name}; undo that first"),
        "the refusal names the later name intent: {refused}"
    );
    assert_eq!(env.intents(), intents, "nothing is recorded");

    // The Person keeps its new name and both accounts.
    let about = env.ok(&["about", "rastro:name/Dora M."]);
    let mut accounts: Vec<&str> = (about["person"]["accounts"].as_array().unwrap().iter())
        .map(|account| account["identifier"].as_str().unwrap())
        .collect();
    accounts.sort();
    assert_eq!(
        accounts,
        ["github:account/11", "linear:user/u-11"],
        "{about}"
    );
}

/// spec: One intent per answer, recorded once
/// S7: a missed report records its text and, when one was named, the item; the named item stays
/// pending and the report is recorded as missed.
#[test]
fn S7_a_missed_report_records_its_text_and_keeps_the_named_item_pending() {
    let env = Env::new();
    env.candidates(&good());
    env.import_ok(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    let queue = env.queue();
    let item = item(&queue, "weekly-1")["id"].as_str().unwrap().to_string();

    // With no item: something Rastro should have caught that may never have been queued.
    let before = env.intents();
    let text = "Alice asked me for the budget on Monday";
    let report = env.ok(&["queue", "missed", text]);
    assert!(report.get("item").is_none(), "{report}");
    assert_eq!(report["intents"].as_array().unwrap().len(), 1, "{report}");
    let recorded = env.last_intent();
    assert_eq!(recorded["kind"], "missed", "{recorded}");
    assert_eq!(recorded["targets"]["text"], text, "{recorded}");
    assert!(recorded["targets"]["item"].is_null(), "{recorded}");
    assert_eq!(env.intents(), before + 1, "{recorded}");
    assert_eq!(ids(&env.queue()), ids(&queue), "nothing left the queue");

    // With an item: it records the item and the item stays pending.
    let before = env.intents();
    let report = env.ok(&["queue", "missed", "I promised to send it", "--item", &item]);
    assert_eq!(report["item"], item, "{report}");
    let recorded = env.last_intent();
    assert_eq!(recorded["kind"], "missed", "{recorded}");
    assert_eq!(
        recorded["targets"]["text"], "I promised to send it",
        "{recorded}"
    );
    assert_eq!(recorded["targets"]["item"], item, "{recorded}");
    assert_eq!(env.intents(), before + 1, "{recorded}");
    assert!(ids(&env.queue()).contains(&item), "the item stays pending");
}

/// spec: One intent per answer, recorded once
/// S8: an answer naming an item the queue no longer holds, or the same item a second time, is
/// refused with a reason naming the item; nothing is recorded and the queue reads the same.
#[test]
fn S8_an_answer_for_an_item_the_queue_no_longer_holds_is_refused_and_records_nothing() {
    let env = Env::new();
    env.candidates(&good());
    env.import_ok(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    let queue = env.queue();
    let item = item(&queue, "weekly-1")["id"].as_str().unwrap().to_string();

    // An item the queue never held.
    let unknown = "q_00000000000000000000000000000000";
    let intents = env.intents();
    let before = env.queue();
    let (ok, refused) = env.run(&["queue", "reject", unknown, "--reason", "not mine"]);
    assert!(!ok, "{refused}");
    assert!(
        refused["error"].as_str().unwrap().contains(unknown),
        "the refusal names the item: {refused}"
    );
    assert_eq!(env.intents(), intents, "nothing is recorded");
    assert_eq!(env.queue(), before, "the queue reads the same");

    // The same item a second time: the first answer left the queue.
    env.ok(&["queue", "reject", &item, "--reason", "not a promise"]);
    let after = env.queue();
    assert!(!ids(&after).contains(&item));
    let intents = env.intents();
    let (ok, refused) = env.run(&["queue", "reject", &item, "--reason", "still not"]);
    assert!(!ok, "{refused}");
    assert!(
        refused["error"].as_str().unwrap().contains(&item),
        "the refusal names the item: {refused}"
    );
    assert_eq!(env.intents(), intents, "nothing is recorded");
    assert_eq!(env.queue(), after, "the queue reads the same");
}

/// spec: One intent per answer, recorded once
/// An accept and an edit each record one owner intent, snapshotting what the owner saw and,
/// when they edited it, what they made; the item leaves the queue.
#[test]
fn an_accept_and_an_edit_record_one_intent_each_and_leave_the_queue() {
    let env = Env::new();
    env.candidates(&good());
    env.import_ok(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    let queue = env.queue();
    let items = queue["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{queue}");
    let first = items[0]["id"].as_str().unwrap().to_string();
    let second = items[1]["id"].as_str().unwrap().to_string();
    let seen = json!({"promise": "send Bob the budget", "stated_date": "by Friday"});
    let second_seen = json!({"promise": "review the numbers", "stated_date": "next week"});

    let before = env.intents();
    let accepted = env.ok(&["queue", "accept", &first]);
    assert_eq!(accepted["item"], first, "{accepted}");
    let recorded = accepted["intents"].as_array().unwrap();
    assert_eq!(recorded.len(), 1, "{accepted}");
    assert_eq!(recorded[0]["kind"], "accept", "{accepted}");
    assert_eq!(env.intents(), before + 1, "{accepted}");
    let stored = env.last_intent();
    assert_eq!(stored["targets"]["seen"], seen, "{stored}");
    assert!(stored["targets"]["edited"].is_null(), "{stored}");
    assert!(!ids(&env.queue()).contains(&first), "still queued");

    let before = env.intents();
    let edited = env.ok(&[
        "queue",
        "edit",
        &second,
        "--promise",
        "review the numbers on Friday",
        "--date",
        "next Friday",
    ]);
    assert_eq!(edited["item"], second, "{edited}");
    let recorded = edited["intents"].as_array().unwrap();
    assert_eq!(recorded.len(), 1, "{edited}");
    assert_eq!(recorded[0]["kind"], "edit", "{edited}");
    assert_eq!(env.intents(), before + 1, "{edited}");
    let stored = env.last_intent();
    assert_eq!(stored["targets"]["seen"], second_seen, "{stored}");
    assert_eq!(
        stored["targets"]["edited"],
        json!({"promise": "review the numbers on Friday", "stated_date": "next Friday"}),
        "{stored}"
    );
    assert!(!ids(&env.queue()).contains(&second), "still queued");
}

/// spec: An accepted proposal becomes an observed claim
/// S9: accepting a proposal unchanged records one owner intent and mints one observed commitment
/// on the owner's Person, holding the item's statements — each standard predicate carrying the
/// party name or the stated date as a literal — and justified by that intent alone; `why` names
/// the intent, the item leaves the queue, and the intent reads as a clean accept.
#[test]
fn S9_accepting_a_proposal_mints_its_standard_terms_justified_by_the_intent() {
    let env = Env::new();
    env.candidates(&good());
    env.import_ok(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    let me = env.ok(&["identity", "me"])["person"].clone();
    let person = me["id"].as_str().unwrap().to_string();
    let proposal = item(&env.queue(), "weekly-1")["id"]
        .as_str()
        .unwrap()
        .to_string();

    let claims = env.claims();
    let accepted = env.ok(&["queue", "accept", &proposal]);
    let recorded = accepted["intents"].as_array().unwrap();
    assert_eq!(recorded.len(), 1, "{accepted}");
    assert_eq!(recorded[0]["kind"], "accept", "{accepted}");
    assert!(
        env.claims() > claims,
        "the accept mints the claim it decided"
    );
    assert!(!ids(&env.queue()).contains(&proposal), "still queued");

    // One commitment holds the item's statements: the item's own `std:Commitment` carries every
    // standard term, each predicate with the party's name or the stated date as a literal, and
    // none is split across subjects.
    let about = env.ok(&["about", "me"]);
    assert_eq!(about["person"]["id"], person, "{about}");
    let commitment = claims::entity_id(&format!("std:commitment/{proposal}"));
    assert_ne!(commitment, person, "the commitment is its own entity");
    let expected: BTreeSet<(String, String, String)> = [
        (
            commitment.clone(),
            "std:owes".to_string(),
            "Alice".to_string(),
        ),
        (
            commitment.clone(),
            "std:owed_to".to_string(),
            "Bob".to_string(),
        ),
        (
            commitment.clone(),
            "std:due".to_string(),
            "by Friday".to_string(),
        ),
    ]
    .into_iter()
    .collect();
    assert_eq!(commitment_statements(&about), expected, "{about}");

    // Each is an observed claim whose only justification is the accept intent, with no evidence
    // span: `why` explains it by that intent.
    let claim = (about["claims"].as_array().unwrap().iter())
        .find(|claim| claim["predicate"] == "std:owes")
        .unwrap_or_else(|| panic!("no owes claim: {about}"));
    assert_eq!(claim["status"], "observed", "{about}");
    let why = env.ok(&["why", claim["id"].as_str().unwrap()]);
    let supports = why["supports"].as_array().unwrap();
    assert_eq!(supports.len(), 1, "{why}");
    assert_eq!(supports[0]["justification_kind"], "owner_intent", "{why}");
    assert_eq!(supports[0]["intent"]["id"], recorded[0]["id"], "{why}");
    assert_eq!(supports[0]["spans"], json!([]), "{why}");

    // What the owner accepted is knowledge: the same statements proposed again by another
    // conversation are left out and counted, never shown twice.
    env.import_ok(
        "weekly-2",
        "work",
        "2026-11-11T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    let queue = env.queue();
    assert_eq!(queue["already_known"], 1, "{queue}");
    assert!(
        (queue["items"].as_array().unwrap().iter())
            .all(|item| item["proposal"]["kind"] == "waiting_on"),
        "only the promise is known: {queue}"
    );
}

/// spec: An accepted proposal becomes an observed claim
/// S10: editing a proposal's promise or stated date and accepting it mints the claim from the
/// owner's edited statement, and the one intent snapshots both what the owner saw and what they
/// made, so the record reads as an accept with edits rather than a clean accept.
#[test]
fn S10_accepting_with_edits_mints_the_edited_statement_and_snapshots_both() {
    let env = Env::new();
    env.candidates(&good());
    env.import_ok(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    let waiting_on = ids(&env.queue())[1].clone();

    let edited = env.ok(&[
        "queue",
        "edit",
        &waiting_on,
        "--promise",
        "review the numbers on Friday",
        "--date",
        "next Friday",
    ]);
    let recorded = edited["intents"].as_array().unwrap();
    assert_eq!(recorded.len(), 1, "{edited}");
    assert_eq!(recorded[0]["kind"], "edit", "accepted with edits: {edited}");
    assert!(!ids(&env.queue()).contains(&waiting_on), "still queued");

    // The one intent snapshots both statements: what the owner saw and what they made.
    let stored = env.last_intent();
    assert_eq!(
        stored["targets"]["seen"],
        json!({"promise": "review the numbers", "stated_date": "next week"}),
        "{stored}"
    );
    assert_eq!(
        stored["targets"]["edited"],
        json!({"promise": "review the numbers on Friday", "stated_date": "next Friday"}),
        "{stored}"
    );

    // The claim holds the owner's edited statement: the date as they made it, not the one they
    // replaced. The statements are the item's own commitment's, and the intent backs them.
    let about = env.ok(&["about", "me"]);
    let commitment = claims::entity_id(&format!("std:commitment/{waiting_on}"));
    let expected: BTreeSet<(String, String, String)> = [
        (
            commitment.clone(),
            "std:waiting_on".to_string(),
            "Bob".to_string(),
        ),
        (
            commitment.clone(),
            "std:owed_to".to_string(),
            "Alice".to_string(),
        ),
        (
            commitment.clone(),
            "std:due".to_string(),
            "next Friday".to_string(),
        ),
    ]
    .into_iter()
    .collect();
    assert_eq!(commitment_statements(&about), expected, "{about}");
    let due = (about["claims"].as_array().unwrap().iter())
        .find(|claim| claim["predicate"] == "std:due")
        .unwrap_or_else(|| panic!("no due claim: {about}"));
    assert_eq!(due["object"]["value"], "next Friday", "{about}");
    let why = env.ok(&["why", due["id"].as_str().unwrap()]);
    assert_eq!(
        why["supports"][0]["intent"]["id"], recorded[0]["id"],
        "the edit alone backs the edited claim: {why}"
    );
}

/// spec: An accepted proposal becomes an observed claim
/// S11: an accepted claim keeps its item's source's scope. A work-scoped read of the queue and of
/// `about me` succeeds with neither the item nor the claim, while a read without a scope shows
/// the claim in `about me` and `why` names the intent.
#[test]
fn S11_an_accepted_claim_is_visible_only_in_the_scopes_that_see_its_source() {
    let env = Env::new();
    env.candidates(&good());
    env.import_ok(
        "personal-meeting",
        "personal",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    let person = env.ok(&["identity", "me"])["person"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let proposal = item(&env.queue(), "personal-meeting")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let accepted = env.ok(&["queue", "accept", &proposal]);
    let intent = accepted["intents"][0]["id"].as_str().unwrap().to_string();

    // Read without a scope: every scope, so the item's commitment is part of `about me` and its
    // `why` names the accept intent.
    let about = env.ok(&["about", "me"]);
    assert_eq!(about["person"]["id"], person, "{about}");
    let commitment = claims::entity_id(&format!("std:commitment/{proposal}"));
    let statements = commitment_statements(&about);
    assert_eq!(statements.len(), 3, "{about}");
    assert!(
        statements
            .iter()
            .all(|(subject, _, _)| subject == &commitment),
        "the commitment's own subject: {about}"
    );
    let due = (about["claims"].as_array().unwrap().iter())
        .find(|claim| claim["predicate"] == "std:due")
        .unwrap_or_else(|| panic!("no due claim: {about}"));
    let why = env.ok(&["why", due["id"].as_str().unwrap()]);
    assert_eq!(why["supports"][0]["intent"]["id"], intent, "{why}");

    // Read with a work scope: the source is hidden, so the claim is too, and the read succeeds.
    let read = env.mcp(&["work"], "about", json!({"entity": "me"}));
    let scoped = &read["structuredContent"]["result"];
    assert_eq!(scoped["resolved"], true, "the read succeeds: {read}");
    assert!(
        commitment_statements(scoped).is_empty(),
        "the personal claim is absent in a work scope: {read}"
    );
    let queue = env.mcp(&["work"], "queue", json!({}));
    assert!(
        !ids(&queue["structuredContent"]["result"]).contains(&proposal),
        "the item is absent in a work scope: {queue}"
    );
}

/// spec: An accepted proposal becomes an observed claim
/// An accepted item keeps its own standard terms: each acceptance is one commitment of its own,
/// so two accepted proposals never merge into one statement set, and the same statements proposed
/// again are known one commitment at a time.
#[test]
fn an_accepted_commitment_keeps_its_own_standard_terms() {
    let env = Env::new();
    env.candidates(&good());
    env.import_ok(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    let items = ids(&env.queue());
    assert_eq!(items.len(), 2, "two proposals: {items:?}");
    for item in &items {
        env.ok(&["queue", "accept", item]);
    }

    // Two commitments, each carrying exactly its own three statements, not one merged set.
    let about = env.ok(&["about", "me"]);
    let mut per_subject: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
    for (subject, predicate, value) in commitment_statements(&about) {
        per_subject
            .entry(subject)
            .or_default()
            .insert((predicate, value));
    }
    assert_eq!(per_subject.len(), 2, "one commitment per item: {about}");
    for statements in per_subject.values() {
        assert_eq!(statements.len(), 3, "each holds its own three: {about}");
    }

    // The same statements proposed again are known, one commitment at a time: the waiting-on's
    // set must not have been merged with the promise's.
    env.import_ok(
        "weekly-2",
        "work",
        "2026-11-11T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    let queue = env.queue();
    assert_eq!(queue["already_known"], 2, "{queue}");
    assert!(queue["items"].as_array().unwrap().is_empty(), "{queue}");
}

/// spec: An accepted proposal becomes an observed claim
/// A scoped read never explains a hidden acceptance: a claim visible in work cites only the
/// intents that scope sees, never a personal item's accept.
#[test]
fn a_scoped_read_never_explains_a_hidden_accept() {
    let env = Env::new();
    env.candidates(&good());
    env.import_ok(
        "personal-meeting",
        "personal",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    // A work meeting proposes the same parties with another date: the two proposals share the
    // parties' terms but neither statement set.
    fs::write(
        env.path("work.txt"),
        "Alice: I'll send Bob the budget next week.\nBob: Thanks.\n",
    )
    .unwrap();
    env.candidates(&[candidate(
        1,
        "promise",
        Some("Alice"),
        Some("Bob"),
        "send Bob the budget",
        Some("next week"),
        "I'll send Bob the budget next week.",
    )]);
    env.import_from(
        "work-meeting",
        "work.txt",
        "work",
        "2026-11-11T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );

    let personal = item(&env.queue(), "personal-meeting")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let personal_accept = env.ok(&["queue", "accept", &personal])["intents"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let work = item(&env.queue(), "work-meeting")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let work_accept = env.ok(&["queue", "accept", &work])["intents"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // The work commitment's own statement, read without a scope, then explained in a work scope.
    let about = env.ok(&["about", "me"]);
    let work_subject = (about["claims"].as_array().unwrap().iter())
        .find(|claim| claim["predicate"] == "std:due" && claim["object"]["value"] == "next week")
        .map(|claim| claim["subject"]["id"].as_str().unwrap().to_string())
        .unwrap_or_else(|| panic!("no work statement: {about}"));
    let owed_to = (about["claims"].as_array().unwrap().iter())
        .find(|claim| claim["subject"]["id"] == work_subject && claim["predicate"] == "std:owed_to")
        .map(|claim| claim["id"].as_str().unwrap().to_string())
        .unwrap_or_else(|| panic!("no work owed_to claim: {about}"));

    // A work-scoped read shows the work commitment in `about me` and not the personal one.
    let read = env.mcp(&["work"], "about", json!({"entity": "me"}));
    let scoped = commitment_statements(&read["structuredContent"]["result"]);
    assert!(
        scoped
            .iter()
            .any(|(_, predicate, value)| predicate == "std:due" && value == "next week"),
        "the work commitment shows in its scope: {read}"
    );
    assert!(
        scoped.iter().all(|(_, _, value)| value != "by Friday"),
        "the personal item stays hidden: {read}"
    );

    let why = env.mcp(&["work"], "why", json!({"claim": owed_to, "limit": 20}));
    let why = &why["structuredContent"]["result"];
    let intents: Vec<&str> = (why["supports"].as_array().unwrap().iter())
        .filter_map(|support| support["intent"]["id"].as_str())
        .collect();
    assert_eq!(
        intents,
        [work_accept.as_str()],
        "only the work accept backs it: {why}"
    );
    assert!(
        !intents.contains(&personal_accept.as_str()),
        "the personal accept stays hidden: {why}"
    );
}

/// What a re-import of `TRANSCRIPT` proposes instead: the second turn's waiting-on, with a
/// different promise, so no earlier item's content survives it.
fn revised() -> Vec<Value> {
    vec![candidate(
        2,
        "waiting_on",
        Some("Bob"),
        Some("Alice"),
        "review the final numbers",
        Some("next week"),
        "I'll review it next week for you if the numbers are final.",
    )]
}

/// The id of `meeting`'s proposal promising to send Bob the budget.
fn promised(queue: &Value, meeting: &str) -> String {
    (queue["items"].as_array().unwrap().iter())
        .find(|item| {
            item["source"]["name"] == meeting
                && item["proposal"]["promise"] == "send Bob the budget"
        })
        .unwrap_or_else(|| panic!("no promise from {meeting}: {queue}"))["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// spec: The renderer reports what it showed
/// S12: an item the owner printed and a re-import later replaced, left unanswered, reads
/// ignored with the time it waited since its first shown report; an item still pending is never
/// ignored however often it was shown.
#[test]
fn S12_an_item_shown_then_replaced_unanswered_reads_ignored_with_its_wait() {
    let env = Env::new();
    env.candidates(&good());
    env.import_ok(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    let queue = env.queue();
    let old = ids(&queue);
    assert_eq!(old.len(), 2, "{queue}");

    // The owner prints the queue twice: the first print starts each item's wait.
    env.print_queue_at("2026-11-10T11:00:00Z");
    env.print_queue_at("2026-11-10T11:30:00Z");

    // A replay and a re-import that carry the same content leave both items waiting under the
    // same ids: neither replaced the source's content, so neither dates their departure.
    let replayed = env.ok_at(
        &["meeting", "replay", "weekly-1"],
        Some("2026-11-10T11:45:00Z"),
    );
    assert_eq!(
        replayed["hypotheses"].as_array().unwrap().len(),
        2,
        "{replayed}"
    );
    env.import_ok_at(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
        "2026-11-10T11:50:00Z",
    );
    assert_eq!(
        ids(&env.queue()),
        old,
        "the same content keeps the same ids"
    );

    // A re-import replaces the hypotheses: both earlier items leave unanswered.
    env.candidates(&revised());
    env.import_ok_at(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
        "2026-11-10T12:00:00Z",
    );
    let replaced = env.queue();
    assert_eq!(replaced["items"].as_array().unwrap().len(), 1, "{replaced}");

    // Each earlier item reads ignored, dated when its source replaced it, having waited since
    // the first time it was printed.
    let outcomes = env.outcomes_at("2026-11-10T13:00:00Z");
    let ignored = outcomes["outcomes"].as_array().unwrap();
    assert_eq!(ignored.len(), 2, "{outcomes}");
    let mut seen: Vec<&str> = (ignored.iter())
        .map(|outcome| outcome["item"].as_str().unwrap())
        .collect();
    seen.sort();
    let mut expected: Vec<&str> = (old.iter()).map(String::as_str).collect();
    expected.sort();
    assert_eq!(seen, expected, "{outcomes}");
    for outcome in ignored {
        assert_eq!(outcome["kind"], "ignored", "{outcomes}");
        assert_eq!(outcome["at"], "2026-11-10T12:00:00Z", "{outcomes}");
        assert_eq!(outcome["waited_seconds"], 3600, "{outcomes}");
    }

    // The item still pending is shown twice more and never reads ignored.
    env.print_queue_at("2026-11-10T13:00:00Z");
    env.print_queue_at("2026-11-10T14:00:00Z");
    let later = env.outcomes_at("2026-11-10T15:00:00Z");
    assert_eq!(later["outcomes"], outcomes["outcomes"], "{later}");
}

/// spec: The renderer reports what it showed
/// S13: the queue read as `--json` or over MCP records no shown report, so automation cannot
/// make an item look ignored.
#[test]
fn S13_a_machine_read_records_no_shown_report() {
    let env = Env::new();
    env.candidates(&good());
    env.import_ok(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    // Two machine reads: the JSON one the owner pipes elsewhere, and an MCP tool call.
    assert_eq!(ids(&env.queue()).len(), 2);
    let read = env.mcp(&[], "queue", json!({}));
    assert_eq!(
        read["structuredContent"]["result"]["items"]
            .as_array()
            .unwrap()
            .len(),
        2,
        "{read}"
    );

    // A re-import replaces both items. Nothing was shown on a terminal, so nothing is ignored.
    env.candidates(&revised());
    env.import_ok_at(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
        "2026-11-10T12:00:00Z",
    );
    let outcomes = env.outcomes_at("2026-11-10T13:00:00Z");
    assert_eq!(outcomes["outcomes"], json!([]), "{outcomes}");
}

/// spec: Outcomes are derived and rebuilt
/// S14: every outcome carries its item id, its kind, its time and, for an accepted item, the
/// claim it became; the totals count what was shown and what was acted on.
#[test]
fn S14_every_outcome_carries_its_item_kind_time_and_claim_and_the_totals_count_shown_and_acted_on()
{
    let env = Env::new();
    env.candidates(&good());
    env.import_ok(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    env.typed_account(
        "github:account/1",
        "code:Account",
        ("name", "Ana Silva"),
        "https://api.github.com/users/1",
    );
    env.typed_account(
        "linear:user/u-1",
        "tracker:Account",
        ("name", "ana silva"),
        "https://linear.app/rastro/profiles/u-1",
    );

    let queue = env.queue();
    assert_eq!(queue["items"].as_array().unwrap().len(), 3, "{queue}");
    let question = question(&queue, "github:account/1")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let proposals: Vec<String> = (ids(&queue).into_iter())
        .filter(|id| *id != question)
        .collect();
    let (first, second) = (&proposals[0], &proposals[1]);

    // What the owner saw: one terminal print of all three items.
    env.print_queue_at("2026-11-10T10:30:00Z");

    let accepted = env.ok_at(&["queue", "accept", first], Some("2026-11-10T11:00:00Z"));
    let accepted_intent = accepted["intents"][0]["id"].as_str().unwrap().to_string();
    env.ok_at(
        &[
            "queue",
            "reject",
            second,
            "--reason",
            "Alice never promised that",
        ],
        Some("2026-11-10T12:00:00Z"),
    );
    env.ok_at(
        &["queue", "answer", &question, "yes"],
        Some("2026-11-10T13:00:00Z"),
    );

    let outcomes = env.outcomes_at("2026-11-10T14:00:00Z");
    let listed = outcomes["outcomes"].as_array().unwrap();
    assert_eq!(listed.len(), 3, "{outcomes}");
    let kinds: Vec<&str> = (listed.iter())
        .map(|outcome| outcome["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["answered", "rejected", "accepted"], "{outcomes}");
    assert_eq!(listed[0]["item"], question.as_str(), "{outcomes}");
    assert_eq!(listed[0]["at"], "2026-11-10T13:00:00Z", "{outcomes}");
    assert_eq!(listed[1]["item"], second.as_str(), "{outcomes}");
    assert_eq!(listed[1]["at"], "2026-11-10T12:00:00Z", "{outcomes}");
    assert_eq!(listed[2]["item"], first.as_str(), "{outcomes}");
    assert_eq!(listed[2]["at"], "2026-11-10T11:00:00Z", "{outcomes}");
    // The accepted one names a claim it became: one the accept intent backs, about the
    // commitment the item minted.
    let claim = listed[2]
        .get("claim")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("an accept mints a claim: {outcomes}"))
        .to_string();
    let backed = env.claims_of_intent(&accepted_intent);
    assert!(
        backed.contains(&claim),
        "the outcome names a claim the accept backs: {claim} not in {backed:?}"
    );
    let commitment = claims::entity_id(&format!("std:commitment/{first}"));
    let about = env.ok(&["about", "me"]);
    let minted: BTreeSet<&str> = (about["claims"].as_array().into_iter().flatten())
        .filter(|claim| claim["subject"]["id"] == commitment)
        .filter_map(|claim| claim["id"].as_str())
        .collect();
    assert!(
        minted.contains(claim.as_str()),
        "the outcome names a claim about the commitment the item became: {claim} not in {minted:?}"
    );
    // The totals count what was shown and what was acted on, each item once.
    assert_eq!(
        outcomes["totals"],
        json!({"shown": 3, "acted_on": 3}),
        "{outcomes}"
    );
}

/// spec: Outcomes are derived and rebuilt
/// S14: an item the queue settled elsewhere — the owner accepted the identical proposal from
/// another meeting, so this one was already known and left unanswered — reads became moot, not
/// ignored, and no read and no rebuild moves its outcome.
#[test]
fn S14_an_item_settled_elsewhere_reads_became_moot_and_no_read_moves_it() {
    let env = Env::new();
    env.candidates(&good());
    for meeting in ["weekly-1", "weekly-2"] {
        env.import_ok(
            meeting,
            "work",
            "2026-11-10T10:00:00Z",
            &["--owner-speaker", "Alice"],
        );
    }
    let queue = env.queue();
    assert_eq!(queue["items"].as_array().unwrap().len(), 4, "{queue}");
    let accepted = promised(&queue, "weekly-1");
    let settled = promised(&queue, "weekly-2");

    // The owner prints the queue, then accepts the promise from one meeting: the identical
    // proposal from the other is already known and leaves the queue with no answer.
    env.print_queue_at("2026-11-10T11:00:00Z");
    env.ok_at(
        &["queue", "accept", &accepted],
        Some("2026-11-10T12:00:00Z"),
    );
    let after = env.queue();
    assert_eq!(after["already_known"], 1, "{after}");
    assert!(!ids(&after).contains(&settled), "{after}");

    // It reads became moot, dated at the last time a terminal read showed it waiting — not the
    // read's own time, with no wait it never measured — and it is not counted as acted on.
    let outcomes = env.outcomes_at("2026-11-10T13:00:00Z");
    let listed = outcomes["outcomes"].as_array().unwrap();
    assert_eq!(listed.len(), 2, "{outcomes}");
    let moot = (listed.iter())
        .find(|outcome| outcome["item"] == settled.as_str())
        .unwrap_or_else(|| panic!("no outcome for the settled item: {outcomes}"));
    assert_eq!(moot["kind"], "became_moot", "{outcomes}");
    assert_eq!(moot["at"], "2026-11-10T11:00:00Z", "{outcomes}");
    assert!(
        moot.get("waited_seconds").is_none(),
        "a moot item measures no wait: {outcomes}"
    );
    assert_eq!(
        outcomes["totals"],
        json!({"shown": 4, "acted_on": 1}),
        "{outcomes}"
    );

    // A later re-import replaces that meeting's content, so a run now lacks the settled item.
    // It still reads became moot at the same time: the queue settled it before any run could
    // have dropped it, and a run that merely lacks it never turns that into ignored.
    env.candidates(&revised());
    env.import_ok_at(
        "weekly-2",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
        "2026-11-10T14:00:00Z",
    );
    let replaced = env.outcomes_at("2026-11-10T15:00:00Z");
    let listed = replaced["outcomes"].as_array().unwrap();
    let still = (listed.iter())
        .find(|outcome| outcome["item"] == settled.as_str())
        .unwrap_or_else(|| panic!("no outcome for the settled item: {replaced}"));
    assert_eq!(still["kind"], "became_moot", "{replaced}");
    assert_eq!(still["at"], "2026-11-10T11:00:00Z", "{replaced}");
    assert!(
        still.get("waited_seconds").is_none(),
        "a moot item measures no wait: {replaced}"
    );
    // The item that re-import did replace reads ignored, dated at that run: the read still
    // tells a run's departure from a settled one.
    let dropped = (listed.iter())
        .find(|outcome| outcome["item"] != settled.as_str() && outcome["kind"] == "ignored")
        .unwrap_or_else(|| panic!("the re-import dropped an unanswered item: {replaced}"));
    assert_eq!(dropped["at"], "2026-11-10T14:00:00Z", "{replaced}");
    assert_eq!(
        replaced["totals"],
        json!({"shown": 4, "acted_on": 1}),
        "{replaced}"
    );

    // Nothing moves it: not a later read, not a rebuild.
    let later = env.outcomes_at("2026-11-10T20:00:00Z");
    assert_eq!(later["outcomes"], replaced["outcomes"], "{later}");
    env.ok_at(&["claims", "rebuild"], Some("2026-11-10T21:00:00Z"));
    let rebuilt = env.outcomes_at("2026-11-10T22:00:00Z");
    assert_eq!(rebuilt["outcomes"], replaced["outcomes"], "{rebuilt}");
    assert_eq!(rebuilt["totals"], replaced["totals"], "{rebuilt}");
}

/// spec: Outcomes are derived and rebuilt
/// S14: an item a run really dropped stays ignored, dated at that run, even when later knowledge
/// holds the statements it expressed: which came first — the run or the knowledge — fixes the
/// kind and the time, in either order.
#[test]
fn S14_an_item_a_run_dropped_stays_ignored_when_later_knowledge_holds_it() {
    let env = Env::new();
    env.candidates(&good());
    for meeting in ["weekly-1", "weekly-2"] {
        env.import_ok(
            meeting,
            "work",
            "2026-11-10T10:00:00Z",
            &["--owner-speaker", "Alice"],
        );
    }
    let queue = env.queue();
    let accepted = promised(&queue, "weekly-1");
    let dropped = promised(&queue, "weekly-2");

    // The owner prints the queue, then a re-import replaces weekly-2's content: its promise
    // leaves unanswered, and nothing had settled it.
    env.print_queue_at("2026-11-10T11:00:00Z");
    env.candidates(&revised());
    env.import_ok_at(
        "weekly-2",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
        "2026-11-10T12:00:00Z",
    );
    let before = env.outcomes_at("2026-11-10T12:30:00Z");
    let ignored = (before["outcomes"].as_array().unwrap().iter())
        .find(|outcome| outcome["item"] == dropped.as_str())
        .unwrap_or_else(|| panic!("no outcome for the dropped item: {before}"));
    assert_eq!(ignored["kind"], "ignored", "{before}");
    assert_eq!(ignored["at"], "2026-11-10T12:00:00Z", "{before}");
    assert_eq!(ignored["waited_seconds"], 3600, "{before}");

    // The owner then accepts the identical promise from the other meeting, so knowledge now holds
    // the statements the dropped item expressed. The run came first, and it stays ignored.
    env.ok_at(
        &["queue", "accept", &accepted],
        Some("2026-11-10T13:00:00Z"),
    );
    let after = env.outcomes_at("2026-11-10T13:30:00Z");
    let still = (after["outcomes"].as_array().unwrap().iter())
        .find(|outcome| outcome["item"] == dropped.as_str())
        .unwrap_or_else(|| panic!("no outcome for the dropped item: {after}"));
    assert_eq!(still["kind"], "ignored", "{after}");
    assert_eq!(still["at"], "2026-11-10T12:00:00Z", "{after}");
    assert_eq!(still["waited_seconds"], 3600, "{after}");
    assert_eq!(
        after["totals"],
        json!({"shown": 4, "acted_on": 1}),
        "{after}"
    );

    // No later read and no rebuild derives it again.
    let later = env.outcomes_at("2026-11-10T13:45:00Z");
    assert_eq!(later["outcomes"], after["outcomes"], "{later}");
    env.ok_at(&["claims", "rebuild"], Some("2026-11-10T14:00:00Z"));
    let rebuilt = env.outcomes_at("2026-11-10T14:30:00Z");
    assert_eq!(rebuilt["outcomes"], after["outcomes"], "{rebuilt}");
}

/// spec: Outcomes are derived and rebuilt
/// S14: a question that goes away when the owner links its two accounts to one Person reads
/// became moot at its last showing: a question carries no source run, so no run dates it.
#[test]
fn S14_a_question_settled_by_a_link_reads_became_moot_at_its_last_showing() {
    let env = Env::new();
    let person = env.ok(&["identity", "me"]);
    let person = person["person"]["identifier"].as_str().unwrap().to_string();
    env.typed_account(
        "github:account/1",
        "code:Account",
        ("name", "Ana Silva"),
        "https://api.github.com/users/1",
    );
    env.typed_account(
        "linear:user/u-1",
        "tracker:Account",
        ("name", "ana silva"),
        "https://linear.app/rastro/profiles/u-1",
    );
    let queue = env.queue();
    assert_eq!(queue["items"].as_array().unwrap().len(), 1, "{queue}");
    let question = ids(&queue)[0].clone();

    env.print_queue_at("2026-11-10T11:00:00Z");
    env.ok_at(
        &["identity", "link", "github:account/1", &person],
        Some("2026-11-10T12:00:00Z"),
    );
    env.ok_at(
        &["identity", "link", "linear:user/u-1", &person],
        Some("2026-11-10T12:30:00Z"),
    );
    assert_eq!(env.queue()["items"], json!([]), "the question is settled");

    let outcomes = env.outcomes_at("2026-11-10T13:00:00Z");
    let listed = outcomes["outcomes"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "{outcomes}");
    assert_eq!(listed[0]["item"], question.as_str(), "{outcomes}");
    assert_eq!(listed[0]["kind"], "became_moot", "{outcomes}");
    assert_eq!(listed[0]["at"], "2026-11-10T11:00:00Z", "{outcomes}");
    assert_eq!(
        outcomes["totals"],
        json!({"shown": 1, "acted_on": 0}),
        "{outcomes}"
    );
    let later = env.outcomes_at("2026-11-10T20:00:00Z");
    assert_eq!(later["outcomes"], outcomes["outcomes"], "{later}");
}

/// spec: Outcomes are derived and rebuilt
/// S15: a claims rebuild returns every accepted claim with the same id and reproduces every
/// outcome and shown report, calling no adapter.
#[test]
fn S15_a_rebuild_reproduces_the_accepted_claim_and_every_outcome_with_no_adapter_call() {
    let env = Env::new();
    env.candidates(&good());
    env.import_ok(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    let proposals = ids(&env.queue());
    assert_eq!(proposals.len(), 2, "{proposals:?}");
    let accepted = proposals[0].clone();

    env.print_queue_at("2026-11-10T10:30:00Z");
    let answer = env.ok_at(
        &["queue", "accept", &accepted],
        Some("2026-11-10T11:00:00Z"),
    );
    let accepted_intent = answer["intents"][0]["id"].as_str().unwrap().to_string();

    // The other item leaves unanswered when a re-import replaces it, so there is an ignored
    // outcome and a shown report to reproduce too.
    env.candidates(&revised());
    env.import_ok_at(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
        "2026-11-10T12:00:00Z",
    );

    let before = env.outcomes_at("2026-11-10T13:00:00Z");
    let claims = env.claims_of_intent(&accepted_intent);
    assert!(!claims.is_empty(), "an accept mints a claim: {before}");
    let accepted_outcome = (before["outcomes"].as_array().unwrap().iter())
        .find(|outcome| outcome["item"] == accepted.as_str())
        .unwrap();
    let named = accepted_outcome["claim"]
        .as_str()
        .unwrap_or_else(|| panic!("the accepted outcome names a claim: {before}"))
        .to_string();
    assert!(
        claims.contains(&named),
        "the accepted outcome names a claim its intent backs: {named} not in {claims:?}"
    );

    // An adapter that would be called now fails and leaves a mark.
    let marker = env.path("adapter-was-called");
    let adapter = env.path("config/plugins/test-adapter/adapter");
    fs::write(
        &adapter,
        format!("#!/bin/sh\ntouch {}\nexit 1\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&adapter, fs::Permissions::from_mode(0o755)).unwrap();
    env.ok_at(&["claims", "rebuild"], Some("2026-11-10T14:00:00Z"));
    assert!(!marker.exists(), "the rebuild called no adapter");

    // Every claim the accept minted returns with the same id, and every outcome and shown
    // report with it.
    assert_eq!(
        env.claims_of_intent(&accepted_intent),
        claims,
        "every accepted claim returns with the same id"
    );
    let after = env.outcomes_at("2026-11-10T15:00:00Z");
    assert_eq!(after["outcomes"], before["outcomes"], "{after}");
    assert_eq!(after["totals"], before["totals"], "{after}");
}

/// The morning the owner works: two meetings' proposals — the second proposing another promise,
/// so no item of the first already holds it — and one identity question.
fn the_morning() -> Env {
    let env = Env::new();
    env.candidates(&good());
    env.import_ok(
        "weekly-1",
        "work",
        "2026-11-10T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    env.candidates(&revised());
    env.import_ok(
        "later-1",
        "work",
        "2026-11-11T10:00:00Z",
        &["--owner-speaker", "Alice"],
    );
    env.typed_account(
        "github:account/1",
        "code:Account",
        ("name", "Ana Silva"),
        "https://api.github.com/users/1",
    );
    env.typed_account(
        "linear:user/u-1",
        "tracker:Account",
        ("name", "ana silva"),
        "https://linear.app/rastro/profiles/u-1",
    );
    env
}

/// spec: The queue serves a real morning
/// One seated morning, worked from the terminal: the owner prints the queue once, then accepts a
/// proposal, edits and accepts another, rejects a third, answers the question and files a missed
/// report — one sitting — and reads the outcomes afterwards.
#[test]
fn the_owner_works_one_queue_end_to_end_in_one_sitting() {
    let env = the_morning();

    // One print of the morning: every item the owner is about to work.
    let printed = env.print_queue_at("2026-11-10T10:30:00Z");
    for text in ["send Bob the budget", "review the numbers", "Ana Silva"] {
        assert!(printed.contains(text), "{printed}");
    }

    let queue = env.queue();
    let question = question(&queue, "github:account/1")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let budget = promised(&queue, "weekly-1");
    let proposal = |promise: &str| {
        (queue["items"].as_array().unwrap().iter())
            .find(|item| item["proposal"]["promise"] == promise)
            .unwrap_or_else(|| panic!("no {promise}: {queue}"))["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let review = proposal("review the numbers");
    let later = proposal("review the final numbers");

    env.ok_at(&["queue", "accept", &budget], Some("2026-11-10T11:00:00Z"));
    env.ok_at(
        &[
            "queue",
            "edit",
            &review,
            "--promise",
            "review the numbers on Friday",
            "--date",
            "next Friday",
        ],
        Some("2026-11-10T11:15:00Z"),
    );
    env.ok_at(
        &[
            "queue",
            "reject",
            &later,
            "--reason",
            "Bob and I did this already",
        ],
        Some("2026-11-10T11:30:00Z"),
    );
    env.ok_at(
        &["queue", "answer", &question, "yes"],
        Some("2026-11-10T11:45:00Z"),
    );
    let missed = env.ok_at(
        &["queue", "missed", "The owner will book the room himself"],
        Some("2026-11-10T12:00:00Z"),
    );
    assert_eq!(missed["intents"][0]["kind"], "missed", "{missed}");

    // One sitting worked every item: nothing waits afterwards, and the outcomes read afterwards
    // hold one per item, newest first, and none for the report that named no item.
    assert_eq!(env.queue()["items"], json!([]), "still waiting");
    let outcomes = env.outcomes_at("2026-11-10T12:30:00Z");
    let listed = outcomes["outcomes"].as_array().unwrap();
    let column = |field: &str| -> Vec<String> {
        (listed.iter())
            .map(|outcome| outcome[field].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        column("kind"),
        ["answered", "rejected", "accepted_with_edits", "accepted"].map(String::from),
        "{outcomes}"
    );
    assert_eq!(
        column("item"),
        [question, later, review, budget],
        "{outcomes}"
    );
    assert_eq!(
        outcomes["totals"],
        json!({"shown": 4, "acted_on": 4}),
        "{outcomes}"
    );
}

/// An Ask cell's match surfaces as a question, not an action proposal: the queue lists it naming
/// the action and title, the terminal read renders it, and no run starts for it. The cell is driven
/// to Ask through the real propose/undo boundary against the scripted fake provider, so no outside
/// system is called; the done runs are seeded with the store's own result record.
mod ask {
    use super::support;
    use rastro_core::{Storage, actions::RunResult};
    use rastro_storage_sqlite::SqliteStorage;
    use serde_json::Value;
    use std::{fs, process::Command};
    use tempfile::TempDir;

    const NOW: &str = "2026-11-03T10:00:00Z";

    struct Env {
        home: TempDir,
    }

    impl Env {
        fn new() -> Self {
            let env = Self {
                home: tempfile::tempdir().unwrap(),
            };
            let config = env.home.path().join("config");
            support::provider::install(&config, |_| {});
            fs::write(
                config.join("config.toml"),
                "[[sources]]\nname = \"linear-work\"\nconnector = \"linear\"\nselector = {}\n\
                 scope = \"work\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
                 keep_original = false\n",
            )
            .unwrap();
            env
        }

        fn add_rule(&self) {
            let config = self.home.path().join("config/config.toml");
            let text = fs::read_to_string(&config).unwrap();
            fs::write(
                &config,
                format!(
                    "{text}\n[[rules]]\nname = \"my-issues\"\naction = \"linear.create_issue\"\n\
                     team = \"H\"\nassignee = \"me\"\n"
                ),
            )
            .unwrap();
        }

        fn rastro_at(&self, now: &str, args: &[&str]) -> std::process::Output {
            Command::new(support::rastro())
                .env("RASTRO_HOME", self.home.path())
                .env("RASTRO_NOW", now)
                .args(args)
                .output()
                .unwrap()
        }

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
                    "H",
                    "--why",
                    "owner asked",
                ],
            )
        }

        fn act_run(&self, title: &str, now: &str) -> String {
            let proposed = self.propose(title, now);
            assert_eq!(proposed["run"]["status"], "pending", "{proposed}");
            let id = proposed["run"]["id"].as_str().unwrap().to_string();
            self.db()
                .record_action_result(
                    &id,
                    &RunResult {
                        identifier: format!("H-{title}"),
                        url: format!("https://fake.invalid/H-{title}"),
                        finished_at: now.parse().unwrap(),
                    },
                )
                .unwrap();
            id
        }

        fn undo(&self, run: &str, now: &str) {
            let undone = self.ok_json_at(now, &["actions", "undo", run]);
            assert_eq!(undone["status"], "undone", "{undone}");
        }

        fn seed_ask(&self) {
            self.add_rule();
            let first = self.act_run("Issue 1", "2026-11-01T10:00:00Z");
            let second = self.act_run("Issue 2", "2026-11-02T10:00:00Z");
            self.undo(&first, "2026-11-01T12:00:00Z");
            self.undo(&second, "2026-11-02T12:00:00Z");
            let cell = self.ok_json_at(NOW, &["trust"])["cells"][0].clone();
            assert_eq!(cell["level"], "ask", "{cell}");
        }
    }

    // spec: An Ask cell asks instead of proposing
    #[test]
    fn an_ask_cells_match_is_listed_as_a_question_naming_the_action_and_title() {
        let env = Env::new();
        env.seed_ask();
        let proposed = env.propose("Ask issue", NOW);
        assert!(
            proposed["run"].is_null(),
            "an Ask match starts no run: {proposed}"
        );
        let item = proposed["id"].as_str().unwrap().to_string();

        // The queue lists a question naming the action and title, not an action proposal.
        let items = env.ok_json_at(NOW, &["queue"])["items"]
            .as_array()
            .unwrap()
            .clone();
        let question = items
            .iter()
            .find(|queued| queued["id"] == item)
            .unwrap_or_else(|| panic!("the queue holds no item for {item}: {items:?}"));
        assert_eq!(question["kind"], "question", "{question}");
        assert_eq!(
            question["action"]["action"], "linear.create_issue",
            "{question}"
        );
        assert_eq!(
            question["action"]["fields"]["title"], "Ask issue",
            "{question}"
        );

        // The terminal read renders the question naming the action and title.
        let printed = env.rastro_at(NOW, &["queue"]);
        assert!(
            printed.status.success(),
            "queue: {}{}",
            String::from_utf8_lossy(&printed.stderr),
            String::from_utf8_lossy(&printed.stdout)
        );
        let text = String::from_utf8_lossy(&printed.stdout);
        let line = text
            .lines()
            .find(|line| line.contains(&item))
            .unwrap_or_else(|| panic!("{text}"));
        assert!(line.contains("Question"), "{line}");
        let body = text
            .lines()
            .skip_while(|line| !line.contains(&item))
            .take(3)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(body.contains("linear.create_issue"), "{body}");
        assert!(body.contains("Ask issue"), "{body}");

        // No run started for the Ask match.
        let runs = env.ok_json_at(NOW, &["actions"])["runs"]
            .as_array()
            .unwrap()
            .clone();
        assert!(
            runs.iter().all(|run| run["proposal"] != item),
            "an Ask match started a run: {runs:?}"
        );
    }
}
