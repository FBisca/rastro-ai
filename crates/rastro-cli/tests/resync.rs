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
        Self::with_manifest(|_| {})
    }
    fn with_manifest(edit: impl FnOnce(&mut toml::Table)) -> Self {
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
        edit(&mut manifest);
        fs::write(plugin.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
        env
    }
    fn rastro(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rastro"))
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.home.path().join("dev-plugins"))
            .env("RASTRO_SENTINEL_SECRET", "hunter2")
            .env("RASTRO_DECLARED_VAR", "declared-value")
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

fn submit(location: &str, quote: &str) -> Value {
    json!({"jsonrpc": "2.0", "method": "evidence.submit", "params": {
        "kind": "issue", "location": location, "field": "/body", "start": 0,
        "end": quote.len(), "quote": quote, "trust_class": "untrusted content",
        "source_updated_at": "2026-11-01T00:00:00Z"}})
}
fn sync_with(env: &Env, bodies: &[(&str, &str)]) -> Value {
    let mut steps = vec![json!({"do": "initialize"})];
    for (location, quote) in bodies {
        steps.push(json!({"do": "send", "message": submit(location, quote)}));
    }
    steps.push(
        json!({"do": "send", "message": {"jsonrpc": "2.0", "method": "coverage.report",
        "params": {"status": "complete", "selection": "all"}}}),
    );
    steps.push(json!({"do": "finish", "status": "ok", "submitted": bodies.len()}));
    env.script(&steps);
    env.json(&["sync", "project"])["result"]["runs"][0].clone()
}

#[test]
fn S17_syncing_again_with_nothing_changed_writes_no_evidence_and_links_the_run() {
    let env = Env::new();
    env.add_source("project");
    let bodies = [
        ("https://x/1", "first body"),
        ("https://x/2", "second body"),
    ];
    let first = sync_with(&env, &bodies);
    assert_eq!(
        first["counts"],
        json!({"submitted": 2, "accepted": 2, "rejected": 0, "unchanged": 0})
    );
    let second = sync_with(&env, &bodies);
    assert_eq!(
        second["counts"],
        json!({"submitted": 2, "accepted": 0, "rejected": 0, "unchanged": 2})
    );
    assert_eq!(second["coverage"], "found");
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 2);
    let run = second["run_id"].as_i64().unwrap();
    assert_eq!(
        env.sql::<i64>(&format!(
            "SELECT count(*) FROM run_evidence WHERE run_id = {run}"
        )),
        2
    );
    let human = env.ok(&["sync", "project"]);
    assert!(human.contains("unchanged 2"), "{human}");
}

#[test]
fn S18_a_body_changing_and_changing_back_leaves_three_observations() {
    let env = Env::new();
    env.add_source("project");
    let at = "https://x/1";
    for body in ["alpha body", "beta body", "alpha body"] {
        let run = sync_with(&env, &[(at, body)]);
        assert_eq!(run["counts"]["accepted"], 1, "{run}");
    }
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 3);
    let quotes = |found: &Value| -> Vec<String> {
        found["result"]["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["quote"].as_str().unwrap().to_string())
            .collect()
    };
    let latest = env.json(&["search", "body"]);
    assert_eq!(quotes(&latest), ["alpha body"]);
    let old = env.json(&["search", "beta"]);
    assert!(quotes(&old).is_empty(), "{old}");
    let history = env.json(&["search", "body", "--history"]);
    let mut all = quotes(&history);
    all.sort();
    assert_eq!(all, ["alpha body", "alpha body", "beta body"]);
    let human = env.ok(&["search", "beta", "--history"]);
    assert!(human.contains("beta body"), "{human}");
}
