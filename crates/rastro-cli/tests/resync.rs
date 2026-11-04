#![allow(non_snake_case)]
mod support;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;

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
            support::test_connector().to_str().unwrap().into(),
        );
        edit(&mut manifest);
        fs::write(plugin.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
        env
    }
    fn rastro(&self, args: &[&str]) -> Output {
        Command::new(support::rastro())
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
            "stdout: {} stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn json(&self, args: &[&str]) -> Value {
        let value: Value = serde_json::from_str(&self.ok(&[args, &["--json"]].concat())).unwrap();
        let schema: Value =
            serde_json::from_str(include_str!("../../../schema/operations-v2.schema.json"))
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

/// evidence/R47: syncing again with nothing changed writes no evidence and links the run
#[test]
fn syncing_again_with_nothing_changed_writes_no_evidence_and_links_the_run() {
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

/// evidence/R48: a body changing and changing back leaves three observations
#[test]
fn a_body_changing_and_changing_back_leaves_three_observations() {
    let env = Env::new();
    env.add_source("project");
    let at = "https://x/1";
    for body in ["alpha body", "beta body", "alpha body"] {
        let run = sync_with(&env, &[(at, body)]);
        assert_eq!(run["counts"]["accepted"], 1, "{run}");
    }
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 3);
    // Whatever the capture times say, the latest observation is the last one appended.
    let db = Connection::open(env.home.path().join("data/rastro.sqlite3")).unwrap();
    db.execute_batch(
        "DROP TRIGGER evidence_no_update;
         UPDATE evidence SET captured_at = '2026-11-05T00:00:00.000Z';",
    )
    .unwrap();
    let rows = |found: &Value| -> Vec<(i64, String)> {
        found["result"]["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["id"].as_i64().unwrap(),
                    r["quote"].as_str().unwrap().into(),
                )
            })
            .collect()
    };
    let latest = env.json(&["search", "body", "--full"]);
    assert_eq!(rows(&latest), [(3, "alpha body".to_string())]);
    let old = env.json(&["search", "beta", "--full"]);
    assert!(rows(&old).is_empty(), "{old}");
    let history = env.json(&["search", "body", "--history", "--full"]);
    assert_eq!(
        rows(&history),
        [
            (1, "alpha body".to_string()),
            (2, "beta body".to_string()),
            (3, "alpha body".to_string())
        ]
    );
    let human = env.ok(&["search", "beta", "--history", "--full"]);
    assert!(human.contains("beta body"), "{human}");
}

#[test]
fn S2_reread_starts_at_registration_and_appends_only_new_fields() {
    let env = Env::new();
    let start = "2026-01-01T00:00:00.000Z";
    env.ok(&[
        "source",
        "add",
        "project",
        "--connector",
        "test",
        "--repo",
        "o/p",
        "--scope",
        "personal",
        "--window-start",
        start,
    ]);
    let initial = sync_with(
        &env,
        &[
            ("https://x/1", "first body"),
            ("https://x/2", "absent later"),
        ],
    );
    assert_eq!(initial["window"]["from"], start);
    let resumed = sync_with(&env, &[("https://x/1", "first body")]);
    assert_eq!(resumed["window"]["from"], initial["window"]["to"]);
    let mut author = submit("https://x/1", "42");
    author["params"]["field"] = json!("/user/id");
    author["params"]["trust_class"] = json!("structured");
    env.script(&[
        json!({"do": "initialize"}),
        json!({"do": "send", "message": submit("https://x/1", "first body")}),
        json!({"do": "send", "message": author}),
        json!({"do": "send", "message": {"jsonrpc": "2.0", "method": "coverage.report",
            "params": {"status": "complete", "selection": "all"}}}),
        json!({"do": "finish", "status": "ok", "submitted": 2}),
    ]);
    let reread = env.json(&["sync", "project", "--reread"])["result"]["runs"][0].clone();
    assert_eq!(reread["window"]["from"], start);
    assert_eq!(
        reread["counts"],
        json!({"submitted": 2, "accepted": 1, "rejected": 0, "unchanged": 1})
    );
    assert_eq!(reread["coverage"], "found");
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 3);
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence WHERE location = 'https://x/2' AND quote = 'absent later'"), 1);
    let run = reread["run_id"].as_i64().unwrap();
    assert_eq!(
        env.sql::<i64>(&format!(
            "SELECT count(*) FROM run_evidence WHERE run_id = {run}"
        )),
        2
    );
    assert_eq!(
        env.sql::<i64>(&format!(
            "SELECT count(*) FROM run_evidence WHERE run_id = {run} AND evidence_id = 2"
        )),
        0
    );
    let coverage = env.json(&["coverage", "project", "--all-windows"]);
    let windows = coverage["result"]["coverage"][0]["windows"]
        .as_array()
        .unwrap();
    assert_eq!(windows.len(), 3);
    let instant = |value: &Value| -> chrono::DateTime<chrono::Utc> {
        value.as_str().unwrap().parse().unwrap()
    };
    assert!(windows.iter().any(|window| {
        instant(&window["from"]) == instant(&reread["window"]["from"])
            && instant(&window["to"]) == instant(&reread["window"]["to"])
            && window["state"] == "found"
    }));
}
