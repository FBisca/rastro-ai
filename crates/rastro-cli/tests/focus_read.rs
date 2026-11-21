#![allow(non_snake_case)]
//! The focus read over stored evidence: what `rastro focus --json` assembles from a real store
//! (meeting commitments, the queue, the owner's Person, a Linear source's issues and its sync),
//! as the JSON a renderer reads. The agent tool adapter and the test connector are scripts, so no
//! model and no network is touched.
mod support;
use chrono::{DateTime, NaiveDateTime, Timelike as _, Utc};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};
use tempfile::TempDir;

const TRANSCRIPT: &str = "Alice: I'll send Bob the budget by Friday.\n";

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

/// A fake agent tool adapter returning the candidates in `$RASTRO_HOME/candidates.json`.
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

/// The key owner's Linear user id, and a colleague's.
const OWNER_LINEAR: &str = "11111111-1111-4111-8111-111111111111";
const COLLEAGUE: &str = "22222222-2222-4222-8222-222222222222";

const NOW: &str = "2026-11-19T09:00:00Z";

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
        let plugin = env.path("dev-plugins/test");
        fs::create_dir_all(&plugin).unwrap();
        let mut manifest: toml::Table =
            include_str!("../../rastro-test-connector/rastro-plugin.toml")
                .parse()
                .unwrap();
        manifest.insert(
            "command".into(),
            support::test_connector().to_str().unwrap().into(),
        );
        manifest["permissions"]["scopes"] = toml::Value::Array(vec!["work".into()]);
        fs::write(plugin.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
        // Extraction resolves the owner speaker, so the owner's Person must exist.
        env.ok(&["identity", "me"]);
        env
    }
    fn path(&self, name: &str) -> PathBuf {
        self.home.path().join(name)
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(support::rastro());
        command
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.path("dev-plugins"))
            .env("RASTRO_TEST_SCRIPT", self.path("script.jsonl"))
            .env("RASTRO_NOW", NOW)
            .args(args);
        command
    }
    fn run(&self, args: &[&str]) -> (bool, Value) {
        let out = self.command(args).arg("--json").output().unwrap();
        let value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{args:?}: {e}: {}", String::from_utf8_lossy(&out.stderr)));
        (out.status.success(), value)
    }
    fn ok(&self, args: &[&str]) -> Value {
        let (ok, value) = self.run(args);
        assert!(ok, "{args:?}: {value}");
        value["result"].clone()
    }
    fn import(&self, meeting: &str) -> bool {
        let file = self.path("transcript.txt");
        self.run(&[
            "meeting",
            "import",
            file.to_str().unwrap(),
            "--meeting",
            meeting,
            "--scope",
            "work",
            "--captured-at",
            "2026-11-18T10:00:00Z",
            "--owner-speaker",
            "Alice",
        ])
        .0
    }
    fn candidates(&self, text: &str) {
        fs::write(self.path("candidates.json"), text).unwrap();
    }
    /// A `linear` source whose connector run submits `issues` as (id, title, state type,
    /// assignee), after the key owner's `linear:viewer`, and finishes with `status`.
    fn sync_linear(&self, issues: &[(&str, &str, &str, &str)], status: &str) {
        let mut steps = vec![json!({"do": "initialize"})];
        let mut submit = |kind: &str, location: String, field: &str, quote: String| {
            steps.push(json!({"do": "send", "message": {"jsonrpc": "2.0",
                "method": "evidence.submit", "params": {"kind": kind,
                "location": location, "field": field, "start": 0,
                "end": quote.len(), "quote": quote, "trust_class": "structured",
                "source_updated_at": "2026-11-18T00:00:00Z"}}}));
        };
        if status != "ok-no-viewer" {
            submit("issue", "linear:viewer".into(), "/id", OWNER_LINEAR.into());
        }
        for (id, title, state, assignee) in issues {
            let location = format!("linear:issue/{id}");
            for (field, quote) in [
                ("/identifier", format!("H-{id}")),
                ("/title", (*title).to_string()),
                ("/state/type", (*state).to_string()),
                ("/assignee/id", (*assignee).to_string()),
            ] {
                submit("issue", location.clone(), field, quote);
            }
        }
        steps.push(json!({"do": "send", "message": {"jsonrpc": "2.0",
            "method": "coverage.report", "params": {"status": "complete", "selection": "all"}}}));
        steps.push(json!({"do": "finish", "status": status.trim_end_matches("-no-viewer"), "submitted": issues.len() * 4 + 1}));
        let lines: Vec<_> = steps.iter().map(Value::to_string).collect();
        fs::write(self.path("script.jsonl"), lines.join("\n")).unwrap();
        // The test connector stands in for Linear: the core knows the source by its connector, so
        // it is a `test` source while it syncs and a `linear` one when it is read.
        let config = self.path("config/config.toml");
        if !config.exists() || !fs::read_to_string(&config).unwrap().contains("tracker") {
            self.ok(&[
                "source",
                "add",
                "tracker",
                "--connector",
                "test",
                "--repo",
                "o/p",
                "--scope",
                "work",
            ]);
        }
        self.connector("linear", "test");
        let (_, out) = self.run(&["sync", "tracker"]);
        assert!(out.get("error").is_none(), "{out}");
        self.connector("test", "linear");
    }
    fn connector(&self, from: &str, to: &str) {
        let config = self.path("config/config.toml");
        let text = fs::read_to_string(&config).unwrap();
        let text = text.replace(
            &format!("connector = \"{from}\""),
            &format!("connector = \"{to}\""),
        );
        fs::write(&config, text).unwrap();
    }
}

/// A focus item as a renderer reads it: nothing but these.
const ITEM_KEYS: [&str; 6] = ["citation", "due", "id", "reason", "title", "urgency"];
const CITATION_KEYS: [&str; 5] = ["location", "name", "observed_at", "source", "url"];
const GAP_KEYS: [&str; 4] = ["as_of", "name", "reason", "source"];

fn keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<_> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

/// spec: Focus is a read in the catalog
/// from a real store the answer holds the owner's meeting promise as the recommended focus with
/// its citation, the queue count and a failed extraction's gap, and no terminal-only field
#[test]
fn the_answer_over_a_stored_meeting_holds_the_promise_its_citation_and_the_gap() {
    let env = Env::new();
    env.candidates(
        &json!([{"turn": 1, "kind": "promise", "responsible": "Alice", "recipient": "Bob",
        "promise": "send Bob the budget", "stated_date": "by Friday",
        "quote": "I'll send Bob the budget by Friday"}])
        .to_string(),
    );
    assert!(env.import("weekly-1"));
    env.candidates("not json");
    assert!(!env.import("broken-1"));

    let queue = env.ok(&["queue"]);
    let waiting = queue["items"].as_array().unwrap().len();
    assert!(waiting > 0, "{queue}");
    let focus = env.ok(&["focus"]);

    let item = &focus["focus"];
    assert_eq!(item["reason"], "waiting_on_owner", "{focus}");
    assert_eq!(item["title"], "send Bob the budget", "{focus}");
    assert_eq!(keys(item), ITEM_KEYS, "{focus}");
    assert_eq!(keys(&item["citation"]), CITATION_KEYS, "{focus}");
    assert_eq!(item["citation"]["source"], "meeting", "{focus}");
    assert_eq!(item["citation"]["name"], "weekly-1", "{focus}");
    assert_eq!(item["citation"]["location"], "weekly-1#turn-1", "{focus}");
    assert_eq!(focus["queue_count"], waiting, "{focus}");
    assert!(focus["notice"].is_null(), "{focus}");
    let gaps = focus["gaps"].as_array().unwrap();
    assert_eq!(gaps.len(), 1, "{focus}");
    assert_eq!(keys(&gaps[0]), GAP_KEYS, "{focus}");
    assert_eq!(gaps[0]["name"], "broken-1", "{focus}");
    assert_eq!(gaps[0]["reason"]["kind"], "extraction_failed", "{focus}");
    assert_eq!(
        keys(&focus),
        [
            "as_of",
            "focus",
            "gaps",
            "needs_you",
            "notice",
            "queue_count"
        ]
    );
}

/// spec: A focus report ranks what needs the owner
/// only the owner's own started issue is a candidate: one a colleague holds is absent, whoever
/// filed it; and a Linear source's started issue is cited in the answer, and a failed sync shows its gap with
/// the last good read's time
#[test]
fn the_answer_over_a_linear_source_holds_its_issues_and_a_failed_sync_as_a_gap() {
    let env = Env::new();
    env.sync_linear(
        &[
            ("a0", "Colleague's started issue", "started", COLLEAGUE),
            ("a1", "Ship the schema", "started", OWNER_LINEAR),
            ("b2", "Old", "completed", OWNER_LINEAR),
        ],
        "ok",
    );
    let focus = env.ok(&["focus"]);
    let item = &focus["focus"];
    assert_eq!(item["reason"], "in_progress", "{focus}");
    assert_eq!(item["title"], "Ship the schema", "{focus}");
    assert_eq!(item["citation"]["source"], "linear", "{focus}");
    assert_eq!(item["citation"]["location"], "linear:issue/a1", "{focus}");
    assert_eq!(
        focus["needs_you"],
        json!([]),
        "a closed issue is not a candidate"
    );
    assert_eq!(focus["gaps"], json!([]), "{focus}");

    env.sync_linear(&[], "failed");
    let failed = env.ok(&["focus"]);
    assert_eq!(failed["focus"]["title"], "Ship the schema", "{failed}");
    let gaps = failed["gaps"].as_array().unwrap();
    assert_eq!(gaps.len(), 1, "{failed}");
    assert_eq!(gaps[0]["name"], "linear", "{failed}");
    assert_eq!(gaps[0]["reason"]["kind"], "sync_failed", "{failed}");
    assert!(gaps[0]["as_of"].is_string(), "{failed}");
}

/// spec: A focus report ranks what needs the owner
/// without the owner's own Linear id in the store, no Linear issue is a candidate
#[test]
fn without_the_owners_linear_id_no_issue_needs_the_owner() {
    let env = Env::new();
    env.sync_linear(
        &[("a1", "Ship the schema", "started", OWNER_LINEAR)],
        "ok-no-viewer",
    );
    let focus = env.ok(&["focus"]);
    assert!(focus["focus"].is_null(), "{focus}");
    assert_eq!(focus["gaps"], json!([]), "{focus}");
}

/// spec: Human records render as blocks
/// spec: Human timestamps share one format
/// The human focus read renders the recommended focus and each gap as blocks — a header line then
/// labelled fields — with the observed and as-of times at minute precision, naming the same
/// instants the JSON read carries.
#[test]
fn the_human_focus_read_renders_blocks_with_minute_precision_times() {
    let env = Env::new();
    env.candidates(
        &json!([{"turn": 1, "kind": "promise", "responsible": "Alice", "recipient": "Bob",
        "promise": "send Bob the budget", "stated_date": "by Friday",
        "quote": "I'll send Bob the budget by Friday"}])
        .to_string(),
    );
    assert!(env.import("weekly-1"));
    env.candidates("not json");
    assert!(!env.import("broken-1"));

    let focus = env.ok(&["focus"]);
    let out = env.command(&["focus"]).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);

    // The recommended focus is a block: its title header, then labelled citation fields.
    assert!(text.contains("Focus: send Bob the budget"), "{text}");
    assert!(text.contains("  Source: meeting weekly-1"), "{text}");
    assert!(text.contains("  Location: weekly-1#turn-1"), "{text}");
    // The failed extraction is a gap block naming its reason.
    assert!(text.contains("Gap: broken-1"), "{text}");
    assert!(text.contains("  Reason: its extraction failed"), "{text}");
    assert!(!text.contains(" | "), "pipe-packed line: {text}");

    // The observed and as-of times read at minute precision and name the JSON instants.
    let minute_of = |rfc3339: &str| {
        let at: DateTime<Utc> = rfc3339.parse().unwrap();
        at.naive_utc()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap()
    };
    for (label, line, instant) in [
        (
            "Observed",
            text.lines()
                .find(|l| l.trim_start().starts_with("Observed: "))
                .unwrap_or_else(|| panic!("no Observed field in {text}")),
            focus["focus"]["citation"]["observed_at"].as_str().unwrap(),
        ),
        (
            "As of",
            // The report's as-of stands at the top level; a gap's is an indented field.
            text.lines()
                .find(|l| l.starts_with("As of: "))
                .unwrap_or_else(|| panic!("no report As of line in {text}")),
            focus["as_of"].as_str().unwrap(),
        ),
    ] {
        let value = line
            .trim_start()
            .strip_prefix(&format!("{label}: "))
            .unwrap();
        let shown = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M UTC")
            .unwrap_or_else(|e| panic!("{label}: {value:?} is not YYYY-MM-DD HH:MM UTC: {e}"));
        assert_eq!(
            shown,
            minute_of(instant),
            "{label} names a different instant"
        );
    }
}
