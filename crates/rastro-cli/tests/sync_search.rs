#![allow(non_snake_case)]
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::OnceLock,
};
use tempfile::TempDir;

/// The test connector lives next to `rastro` in the target directory.
fn connector_binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let status = Command::new(cargo)
            .args(["build", "--quiet", "-p", "rastro-test-connector"])
            .status()
            .unwrap();
        assert!(status.success(), "building the test connector failed");
        let rastro = Path::new(env!("CARGO_BIN_EXE_rastro"));
        rastro.with_file_name("rastro-test-connector")
    })
}

struct Env {
    home: TempDir,
}
impl Env {
    fn new() -> Self {
        let env = Self {
            home: TempDir::new().unwrap(),
        };
        let plugin = env.home.path().join("dev-plugins/test");
        fs::create_dir_all(&plugin).unwrap();
        let mut manifest: toml::Table =
            include_str!("../../rastro-test-connector/rastro-plugin.toml")
                .parse()
                .unwrap();
        manifest.insert(
            "command".into(),
            connector_binary().to_str().unwrap().into(),
        );
        fs::write(plugin.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
        env
    }
    fn rastro(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rastro"))
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.home.path().join("dev-plugins"))
            .env("RASTRO_TEST_SCRIPT", self.home.path().join("script.jsonl"))
            .args(args)
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let output = self.rastro(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn json(&self, args: &[&str]) -> Value {
        let value: Value = serde_json::from_str(&self.ok(&[args, &["--json"]].concat())).unwrap();
        let schema: Value =
            serde_json::from_str(include_str!("../../../schema/operations-v1.schema.json"))
                .unwrap();
        jsonschema::validator_for(&schema)
            .unwrap()
            .validate(&value)
            .unwrap_or_else(|e| panic!("{e}: {value}"));
        value
    }
    fn add_source(&self, name: &str) {
        self.ok(&[
            "source",
            "add",
            name,
            "--connector",
            "test",
            "--repo",
            "o/p",
            "--scope",
            "personal",
        ]);
    }
    fn script(&self, steps: &[Value]) {
        let lines: Vec<_> = steps.iter().map(Value::to_string).collect();
        fs::write(self.home.path().join("script.jsonl"), lines.join("\n")).unwrap();
    }
    fn sql<T: rusqlite::types::FromSql>(&self, query: &str) -> T {
        Connection::open(self.home.path().join("data/rastro.sqlite3"))
            .unwrap()
            .query_row(query, [], |row| row.get(0))
            .unwrap()
    }
}

fn evidence(params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": "evidence.submit", "params": params})
}
fn valid(location: &str, quote: &str) -> Value {
    evidence(json!({
        "kind": "issue", "location": location, "field": "/body", "start": 0,
        "end": quote.len(), "quote": quote, "trust_class": "untrusted content",
        "source_updated_at": "2026-11-01T00:00:00Z"
    }))
}
fn with(mut message: Value, key: &str, value: Value) -> Value {
    message["params"][key] = value;
    message
}
fn without(mut message: Value, key: &str) -> Value {
    message["params"].as_object_mut().unwrap().remove(key);
    message
}
fn complete() -> Value {
    json!({"jsonrpc": "2.0", "method": "coverage.report",
           "params": {"status": "complete", "selection": "all"}})
}
fn run_script(messages: Vec<Value>) -> Vec<Value> {
    let submitted = messages
        .iter()
        .filter(|m| m["method"] == "evidence.submit")
        .count();
    let mut steps = vec![json!({"do": "initialize"})];
    steps.extend(
        messages
            .into_iter()
            .map(|m| json!({"do": "send", "message": m})),
    );
    steps.push(json!({"do": "finish", "status": "ok", "submitted": submitted}));
    steps
}

#[test]
fn S4_sync_stores_evidence_and_found_coverage() {
    let env = Env::new();
    env.add_source("project");
    env.script(&run_script(vec![
        valid("https://github.com/o/p/issues/1", "hello world"),
        valid("https://github.com/o/p/issues/2", "second body"),
        complete(),
    ]));
    let synced = env.json(&["sync", "project"]);
    assert_eq!(synced["operation"], "sync");
    let run = &synced["result"]["runs"][0];
    assert_eq!(run["source"], "project");
    assert_eq!(run["status"], "ok");
    assert_eq!(run["plugin_id"], "test");
    assert_eq!(run["plugin_version"], "0.1.0");
    assert_eq!(
        run["counts"],
        json!({"submitted": 2, "accepted": 2, "rejected": 0, "unchanged": 0})
    );
    assert_eq!(run["coverage"], "found");
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 2);
    assert_eq!(env.sql::<String>("SELECT state FROM coverage"), "found");
    assert_eq!(
        env.sql::<String>("SELECT plugin_version || status FROM runs"),
        "0.1.0ok"
    );
    assert_eq!(
        env.sql::<String>("SELECT window_end FROM coverage"),
        run["window"]["to"].as_str().unwrap().to_string()
    );
    let covered = env.json(&["coverage", "project"]);
    assert_eq!(covered["result"]["coverage"][0]["state"], "found");
    env.script(&run_script(vec![complete()]));
    let human = env.ok(&["sync", "project"]);
    for expected in ["project", "ok", "0.1.0", "accepted 0", "empty"] {
        assert!(human.contains(expected), "missing {expected}: {human}");
    }
    let second_window_start = env.sql::<String>("SELECT window_start FROM coverage WHERE id = 2");
    assert_eq!(
        second_window_start,
        env.sql::<String>("SELECT window_end FROM coverage WHERE id = 1")
    );
}

#[test]
fn S4a_complete_sync_without_evidence_records_empty() {
    let env = Env::new();
    env.add_source("project");
    env.script(&run_script(vec![complete()]));
    let synced = env.json(&["sync"]);
    assert_eq!(synced["result"]["runs"][0]["status"], "ok");
    assert_eq!(synced["result"]["runs"][0]["coverage"], "empty");
    assert_eq!(env.sql::<String>("SELECT state FROM coverage"), "empty");
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 0);
    let covered = env.json(&["coverage", "project"]);
    assert_eq!(covered["result"]["coverage"][0]["state"], "empty");
}

#[test]
fn S5_evidence_without_location_span_quote_or_trust_class_is_rejected() {
    let env = Env::new();
    env.add_source("project");
    let good = valid("https://github.com/o/p/issues/1", "hello world");
    env.script(&run_script(vec![
        without(good.clone(), "location"),
        without(good.clone(), "start"),
        without(good.clone(), "end"),
        without(good.clone(), "field"),
        without(good.clone(), "quote"),
        without(good.clone(), "trust_class"),
        with(good.clone(), "trust_class", json!("trusted")),
        with(
            with(
                valid("https://github.com/o/p/issues/9", "é"),
                "end",
                json!(1),
            ),
            "start",
            json!(0),
        ),
        good,
        complete(),
    ]));
    let synced = env.json(&["sync", "project"]);
    assert_eq!(
        synced["result"]["runs"][0]["counts"],
        json!({"submitted": 9, "accepted": 1, "rejected": 8, "unchanged": 0})
    );
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 1);
    assert_eq!(env.sql::<i64>("SELECT rejected FROM runs"), 8);
    let human = env.ok(&["sync", "project"]);
    assert!(human.contains("rejected 8"), "{human}");
}

#[test]
fn S6_forbidden_kind_or_forged_source_or_scope_is_rejected() {
    let env = Env::new();
    env.add_source("project");
    let good = valid("https://github.com/o/p/issues/1", "hello world");
    env.script(&run_script(vec![
        with(good.clone(), "kind", json!("commit")),
        with(good.clone(), "source", json!("other")),
        with(good.clone(), "scope", json!("work")),
        complete(),
    ]));
    let synced = env.json(&["sync", "project"]);
    assert_eq!(
        synced["result"]["runs"][0]["counts"],
        json!({"submitted": 3, "accepted": 0, "rejected": 3, "unchanged": 0})
    );
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 0);
}

fn synced_env() -> Env {
    let env = Env::new();
    env.add_source("project");
    env.add_source("other");
    env.script(&run_script(vec![
        valid("https://github.com/o/p/issues/1", "the quick brown fox"),
        complete(),
    ]));
    env.ok(&["sync", "project"]);
    env
}

#[test]
fn S19_search_results_carry_the_citation_and_coverage() {
    let env = synced_env();
    let human = env.ok(&["search", "quick fox"]);
    for expected in [
        "the quick brown fox",
        "https://github.com/o/p/issues/1",
        "/body",
        "[0, 19)",
        "Captured: 20",
        "Trust class: untrusted content",
        "Scope: personal",
        "Coverage: found",
        "Fresh until: 20",
    ] {
        assert!(human.contains(expected), "missing {expected}: {human}");
    }
    let found = env.json(&["search", "quick"]);
    assert_eq!(found["operation"], "search");
    let hit = &found["result"]["results"][0];
    assert_eq!(hit["quote"], "the quick brown fox");
    assert_eq!(hit["location"], "https://github.com/o/p/issues/1");
    assert_eq!(
        (
            hit["field"].clone(),
            hit["start"].clone(),
            hit["end"].clone()
        ),
        (json!("/body"), json!(0), json!(19))
    );
    assert_eq!(hit["trust_class"], "untrusted content");
    assert_eq!(hit["scope"], "personal");
    assert_eq!(hit["source"], "project");
    assert!(hit["captured_at"].as_str().unwrap().starts_with("20"));
    assert_eq!(hit["coverage"]["state"], "found");
    assert!(hit["coverage"]["fresh_until"].is_string());
}

#[test]
fn S20_search_without_a_match_states_the_coverage_of_each_source() {
    let env = synced_env();
    let human = env.ok(&["search", "zebra"]);
    assert!(human.contains("no evidence found"), "{human}");
    assert!(
        human.contains("project | Scope: personal | Coverage: found"),
        "{human}"
    );
    assert!(
        human.contains("other | Scope: personal | Coverage: never looked"),
        "{human}"
    );
    let none = env.json(&["search", "zebra"]);
    assert_eq!(none["result"]["results"], json!([]));
    let coverage = none["result"]["coverage"].as_array().unwrap();
    assert_eq!(coverage.len(), 2);
    assert_eq!(coverage[0]["source"], "project");
    assert_eq!(coverage[0]["state"], "found");
    assert_eq!(coverage[1]["state"], "never looked");
}
