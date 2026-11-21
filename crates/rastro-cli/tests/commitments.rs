#![allow(non_snake_case)]
//! The commitments list end to end: a candidate whose party is explicitly mapped to an account the
//! evidence says is a bot is excluded and counted; a party that is only named is kept. The agent
//! tool adapter is a shell script that speaks the plugin protocol and prints a canned response.
mod support;
use chrono::{DateTime, Utc};
use rastro_core::claims::{self, Entity, Object, ObservedClaim, Support};
use rastro_core::{Evidence, Scope, Source, Storage, TrustClass};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};
use tempfile::TempDir;

/// The extractor a fixture source is bound to; it is never installed, so nothing re-reads it.
const MISSING: &str = "missing-extractor";

const TRANSCRIPT: &str = "Alice: I'll send Bob the budget by Friday.\n\
Bob: Thanks. I'll review it next week if the numbers are final.\n\
Carol: I will email Dave the notes tomorrow.\n\
Alice: Bob, can you share the roadmap?\n\
Bob: Sure, I'll share the roadmap with you after the board meeting.\n\
Alice: I'll update the plan.\n";

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

/// Answers the plugin protocol: answers `initialize`, and for a judgment request prints
/// `response.json` as the result.
const ADAPTER: &str = r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocol":"1.0","manifest_id":"test-adapter"}}\n' "$id"
      ;;
    *'"method":"judgment.request"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":' "$id"
      cat "$RASTRO_HOME/response.json"
      printf '}\n'
      ;;
  esac
done
"#;

fn candidate(
    turn: u32,
    responsible: Option<&str>,
    recipient: Option<&str>,
    promise: &str,
    date: Option<&str>,
    quote: &str,
) -> Value {
    json!({"turn": turn, "kind": "promise", "responsible": responsible, "recipient": recipient,
        "promise": promise, "stated_date": date, "quote": quote})
}

/// The three owner-involved promises of `TRANSCRIPT`: two name Bob as a party, one does not.
fn good() -> Vec<Value> {
    vec![
        candidate(
            1,
            Some("Alice"),
            Some("Bob"),
            "send Bob the budget",
            Some("by Friday"),
            "I'll send Bob the budget by Friday",
        ),
        candidate(
            5,
            Some("Bob"),
            Some("Alice"),
            "share the roadmap with Alice",
            Some("after the board meeting"),
            "I'll share the roadmap with you after the board meeting",
        ),
        candidate(
            6,
            Some("Alice"),
            None,
            "update the plan",
            None,
            "I'll update the plan.",
        ),
    ]
}

struct Env {
    home: TempDir,
}
impl Env {
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
        };
        env.configure();
        fs::write(env.path("transcript.txt"), TRANSCRIPT).unwrap();
        fs::write(
            env.path("response.json"),
            json!({"candidates": good()}).to_string(),
        )
        .unwrap();
        env.ok(&["identity", "me"]);
        env
    }
    fn path(&self, name: &str) -> PathBuf {
        self.home.path().join(name)
    }
    /// Installs the adapter plugin and binds it as the commitment adapter.
    fn configure(&self) {
        fs::create_dir_all(self.path("config/plugins/test-adapter")).unwrap();
        fs::write(
            self.path("config/config.toml"),
            "[commitment_adapter]\nplugin = \"test-adapter\"\n",
        )
        .unwrap();
        fs::write(
            self.path("config/plugins/test-adapter/rastro-plugin.toml"),
            MANIFEST,
        )
        .unwrap();
        let script = self.path("config/plugins/test-adapter/adapter");
        fs::write(&script, ADAPTER).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fn run(&self, args: &[&str]) -> (bool, Value) {
        let out = Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.path("no-plugins"))
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
    /// The human read of `args`: what the owner sees, not JSON.
    fn human(&self, args: &[&str]) -> String {
        let out = Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.path("no-plugins"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }
    fn import_ok(&self, extra: &[&str]) -> Value {
        let transcript = self.path("transcript.txt");
        let mut args = vec![
            "meeting",
            "import",
            transcript.to_str().unwrap(),
            "--meeting",
            "weekly-1",
            "--scope",
            "work",
            "--captured-at",
            "2026-11-10T10:00:00Z",
        ];
        args.extend_from_slice(extra);
        let (ok, value) = self.run(&args);
        assert!(ok, "{value}");
        value["result"].clone()
    }
    /// One `rastro mcp --scope work` tool call over stdio; returns the tool's v2 envelope.
    fn mcp(&self, tool: &str, arguments: Value) -> Value {
        use std::io::{BufRead, BufReader, Write};
        use std::process::Stdio;
        let mut child = Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.path("no-plugins"))
            .args(["mcp", "--scope", "work"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
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
        reply["result"]["structuredContent"].clone()
    }

    /// Records the evidence-backed claim that the account `identifier` is typed `code:Bot`.
    fn bot(&self, identifier: &str) {
        let location = format!("https://api.github.com/users/{identifier}");
        let subject = Entity::new(identifier.into(), "code:Bot".into());
        let object = Object::Literal {
            value: "code:Bot".into(),
            datatype: claims::TYPE_DATATYPE.into(),
        };
        let time: DateTime<Utc> = "2026-11-01T00:00:00Z".parse().unwrap();
        let source = Source::github("project".into(), "o/p".into(), Scope("work".into()));
        let mut db =
            rastro_storage_sqlite::SqliteStorage::open(&self.path("data/rastro.sqlite3")).unwrap();
        db.project_sources(std::slice::from_ref(&source)).unwrap();
        db.bind_extractor(&source.name, Some(MISSING)).unwrap();
        let run = db
            .start_run(&source, (MISSING, "1"), time, (time, time))
            .unwrap();
        let quote = "code:Bot".to_string();
        let evidence = Evidence {
            source: source.name.clone(),
            scope: source.scope.clone(),
            location: location.clone(),
            field: "/type".into(),
            start: 0,
            end: quote.len() as u64,
            quote,
            trust_class: TrustClass::Structured,
            captured_at: time,
            source_updated_at: Some(time),
            original: None,
        };
        db.append_evidence(run, &evidence).unwrap();
        let (evidence_id, _) = db
            .pending_evidence((MISSING, "1"), &source)
            .unwrap()
            .into_iter()
            .rev()
            .find(|(_, item)| item.location == location && item.field == "/type")
            .unwrap();
        let observed = ObservedClaim {
            id: claims::claim_id(&subject.id, claims::TYPE_PREDICATE, &object),
            subject,
            predicate: claims::TYPE_PREDICATE.into(),
            object,
            scalar: true,
            type_ancestors: Vec::new(),
            supports: std::collections::BTreeSet::from([Support {
                portable_key: claims::portable_evidence_key(&evidence),
                evidence_id,
                field: "/type".into(),
                start: 0,
                end: evidence.end,
                extraction_rule: "fixture-rule".into(),
            }]),
        };
        db.record_claims(run, (MISSING, "1"), &[observed], None)
            .unwrap();
    }
}

/// S9: a candidate whose party is mapped to a bot account is excluded and counted; a party that
/// is only named (no mapping) is kept.
// spec: A party mapped to a bot excludes the candidate
#[test]
fn S9_a_party_mapped_to_a_bot_is_excluded_and_counted_and_a_named_party_is_not() {
    let env = Env::new();
    env.bot("github:account/9");

    let bot = env.import_ok(&[
        "--owner-speaker",
        "Alice",
        "--speaker",
        "Bob=github:account/9",
    ]);
    // Both candidates that name Bob are excluded; the one that does not name him stays.
    assert_eq!(bot["extraction"]["excluded"], 2, "{bot}");
    let hypotheses = bot["hypotheses"].as_array().unwrap();
    assert_eq!(hypotheses.len(), 1, "{bot}");
    assert_eq!(hypotheses[0]["quote"]["text"], "I'll update the plan.");

    let listed = env.ok(&["commitments"]);
    let source = &listed["sources"][0];
    assert_eq!(source["extraction"]["excluded"], 2, "{listed}");
    assert_eq!(
        source["hypotheses"].as_array().unwrap().len(),
        1,
        "{listed}"
    );

    // Bob only named: nothing is excluded and all three stay.
    let named = env.import_ok(&["--owner-speaker", "Alice"]);
    assert_eq!(named["extraction"]["excluded"], 0, "{named}");
    assert_eq!(named["hypotheses"].as_array().unwrap().len(), 3, "{named}");
}

// spec: The owner reads a real conversation's commitments over MCP
/// commitments/R10: the owner imports a conversation, deletes the transcript, and reads the
/// conversation's hypotheses over MCP; every hypothesis carries the exact quote it rests on, its
/// turn and its speaker, so the transcript is never opened by hand.
#[test]
fn the_owner_reads_a_conversations_hypotheses_over_mcp_without_opening_the_transcript() {
    let env = Env::new();
    let imported = env.import_ok(&["--owner-speaker", "Alice"]);
    assert_eq!(
        imported["hypotheses"].as_array().unwrap().len(),
        3,
        "{imported}"
    );
    fs::remove_file(env.path("transcript.txt")).unwrap();

    // The one read the owner makes all morning: no transcript on disk, only the list.
    let list = env.mcp("commitments", json!({}));
    let hypotheses = list["result"]["sources"][0]["hypotheses"]
        .as_array()
        .unwrap();
    assert_eq!(hypotheses.len(), 3, "{list}");

    let quoted: Vec<(&str, &str, u64)> = hypotheses
        .iter()
        .map(|hypothesis| {
            assert_eq!(hypothesis["status"], "hypothesis", "{hypothesis}");
            let quote = &hypothesis["quote"];
            (
                quote["text"].as_str().unwrap(),
                quote["speaker"].as_str().unwrap(),
                quote["turn"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        quoted,
        [
            ("I'll send Bob the budget by Friday", "Alice", 1),
            (
                "I'll share the roadmap with you after the board meeting",
                "Bob",
                5
            ),
            ("I'll update the plan.", "Alice", 6),
        ],
        "the list alone says whose words each hypothesis rests on"
    );
}

// spec: Human records render as blocks
// spec: Human timestamps share one format
/// The commitments list renders each source and each hypothesis as a block, with the finished
/// time at minute precision naming the instant the JSON read carries.
#[test]
fn the_commitments_list_renders_sources_and_hypotheses_as_blocks() {
    let env = Env::new();
    env.import_ok(&["--owner-speaker", "Alice"]);
    let text = env.human(&["commitments"]);
    // The notice stays the preamble; the source is one block, its fields one per line.
    assert!(text.starts_with(rastro_core::commitments::NOTICE), "{text}");
    assert!(text.lines().any(|l| l == "weekly-1"), "{text}");
    assert!(text.contains("  Kind: meeting"), "{text}");
    assert!(text.contains("  Scope: work"), "{text}");
    assert!(text.contains("  Extraction: succeeded"), "{text}");
    assert!(text.contains("  Adapter: test-adapter"), "{text}");
    assert!(text.contains("  Excluded: 0"), "{text}");
    assert!(text.contains("  Coverage: found"), "{text}");
    assert!(text.contains("  Turns read: 6 of 6"), "{text}");
    let listed = env.ok(&["commitments"]);
    let finished = listed["sources"][0]["extraction"]["finished_at"]
        .as_str()
        .unwrap();
    let minute = format!("{} UTC", finished[..16].replace('T', " "));
    assert!(
        text.lines().any(|l| l == format!("  Finished: {minute}")),
        "{text}"
    );
    // Each hypothesis is a block: its kind, parties, promise, date and terms, then its quote.
    // The owner's label is mapped to the owner's Person; Bob was never mapped.
    let me = env.ok(&["identity", "me"])["person"]["identifier"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        text.lines().filter(|l| *l == "Hypothesis").count(),
        3,
        "{text}"
    );
    assert!(text.contains("  Kind: promise"), "{text}");
    assert!(
        text.lines().any(|l| l == format!("  From: {me} (Alice)")),
        "{text}"
    );
    assert!(text.lines().any(|l| l == "  To: unknown (Bob)"), "{text}");
    assert!(text.contains("  Promise: send Bob the budget"), "{text}");
    assert!(text.contains("  Date: by Friday"), "{text}");
    assert!(
        text.contains("  Terms: std:owes, std:owed_to, std:due"),
        "{text}"
    );
    assert!(
        text.contains("  Quote: \"I'll send Bob the budget by Friday\""),
        "{text}"
    );
    assert!(!text.contains(" | "), "pipe-packed line: {text}");
}

// spec: Empty and no-op results speak with one voice
/// An empty commitments list prints the notice, then one sentence starting `No ` naming what is
/// absent.
#[test]
fn an_empty_commitments_list_prints_one_no_sentence() {
    let env = Env::new();
    let text = env.human(&["commitments"]);
    assert_eq!(
        text,
        format!("{}\n\nNo sources\n", rastro_core::commitments::NOTICE)
    );
}

// spec: Human records render as blocks
/// A source with no hypotheses says so in its block: found nothing when its run looked,
/// couldn't look when it failed, with the cause and the stale results it keeps.
#[test]
fn a_source_with_no_hypotheses_says_found_nothing_or_couldnt_look_in_its_block() {
    let env = Env::new();
    // A run that looked and found nothing.
    fs::write(
        env.path("response.json"),
        json!({"candidates": []}).to_string(),
    )
    .unwrap();
    env.import_ok(&["--owner-speaker", "Alice"]);
    let text = env.human(&["commitments"]);
    assert!(
        text.lines().any(|l| l == "  Hypotheses: found nothing"),
        "{text}"
    );
    assert!(!text.contains(" | "), "pipe-packed line: {text}");
    // A failed run: the block names the cause, that it could not look, and the run the kept
    // results come from.
    fs::write(env.path("response.json"), "not json").unwrap();
    let transcript = env.path("transcript.txt");
    let (ok, failed) = env.run(&[
        "meeting",
        "import",
        transcript.to_str().unwrap(),
        "--meeting",
        "weekly-1",
        "--scope",
        "work",
        "--captured-at",
        "2026-11-10T10:00:00Z",
        "--owner-speaker",
        "Alice",
    ]);
    assert!(!ok, "{failed}");
    let text = env.human(&["commitments"]);
    assert!(text.contains("  Extraction: failed"), "{text}");
    let listed = env.ok(&["commitments"]);
    let source = &listed["sources"][0];
    let cause = source["extraction"]["cause"].as_str().unwrap();
    assert!(
        text.lines().any(|l| l == format!("  Cause: {cause}")),
        "{text}"
    );
    assert!(
        text.lines().any(|l| l == "  Hypotheses: couldn't look"),
        "{text}"
    );
    let from = &source["results_from"];
    let imported = from["imported_at"].as_str().unwrap();
    let minute = format!("{} UTC", imported[..16].replace('T', " "));
    assert!(
        text.lines().any(|l| l == format!(
            "  Stale: the hypotheses below come from run {} (imported {minute}), not the latest",
            from["run"].as_str().unwrap()
        )),
        "{text}"
    );
    assert!(!text.contains(" | "), "pipe-packed line: {text}");
}
