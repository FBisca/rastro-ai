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
        self.rastro_at(args, None)
    }
    /// `now` ages the clock coverage is judged by (`RASTRO_NOW`); the core alone reads it.
    fn rastro_at(&self, args: &[&str], now: Option<&str>) -> Output {
        self.rastro_os(args.iter().map(Into::into).collect(), now)
    }
    fn rastro_os(&self, args: Vec<std::ffi::OsString>, now: Option<&str>) -> Output {
        let mut command = Command::new(support::rastro());
        if let Some(now) = now {
            command.env("RASTRO_NOW", now);
        }
        command
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
    /// Runs with `--json`, whatever the exit status, and checks the output against the schema.
    fn json_any(&self, args: &[&str], now: Option<&str>) -> (bool, Value) {
        let output = self.rastro_at(&[args, &["--json"]].concat(), now);
        let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
            panic!("{e}: {}", String::from_utf8_lossy(&output.stderr));
        });
        validate(&value);
        (output.status.success(), value)
    }
    fn json(&self, args: &[&str]) -> Value {
        let (ok, value) = self.json_any(args, None);
        assert!(ok, "{value}");
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

fn validate(value: &Value) {
    let schema: Value =
        serde_json::from_str(include_str!("../../../schema/operations-v1.schema.json")).unwrap();
    jsonschema::validator_for(&schema)
        .unwrap()
        .validate(value)
        .unwrap_or_else(|e| panic!("{e}: {value}"));
}

fn synced_once(env: &Env) {
    env.add_source("project");
    sync_with(
        env,
        &[
            ("https://x/1", "first body"),
            ("https://x/2", "second body"),
        ],
    );
}
fn coverage_of(env: &Env, now: Option<&str>) -> Value {
    let (ok, value) = env.json_any(&["coverage", "project"], now);
    assert!(ok, "{value}");
    value["result"]["coverage"][0].clone()
}

/// evidence/R51: past freshness or after a failed attempt coverage is unknown with its evidence
#[test]
fn past_freshness_or_after_a_failed_attempt_coverage_is_unknown_with_its_evidence() {
    let env = Env::new();
    synced_once(&env);
    let fresh = coverage_of(&env, None);
    assert_eq!(fresh["state"], "found");
    let checked = fresh["last_checked"].as_str().unwrap().to_string();
    // Two hours on, the default one-hour rule has lapsed.
    let later = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
    let stale = coverage_of(&env, Some(&later));
    assert_eq!(stale["state"], "unknown");
    assert_eq!(stale["last_checked"], checked);
    assert!(stale.get("reason").is_none(), "{stale}");
    let human =
        String::from_utf8(env.rastro_at(&["coverage", "project"], Some(&later)).stdout).unwrap();
    assert!(
        human.contains("unknown") && human.contains(&checked),
        "{human}"
    );
    // The connector goes away; the latest attempt fails inside the freshness rule.
    fs::remove_dir_all(env.home.path().join("dev-plugins/test")).unwrap();
    assert!(!env.rastro(&["sync", "project"]).status.success());
    let failed = coverage_of(&env, None);
    assert_eq!(failed["state"], "unknown");
    assert_eq!(failed["reason"], "not_installed");
    assert_eq!(failed["last_checked"], checked);
    let human = env.ok(&["coverage", "project"]);
    assert!(
        human.contains("unknown") && human.contains("not_installed"),
        "{human}"
    );
    // An empty answer from search says what it covered, not that the source is empty.
    let found = env.json(&["search", "nothing-matches"]);
    assert_eq!(found["result"]["coverage"][0]["state"], "unknown");
}

/// evidence/R52: rebuilding the dropped index returns the same results with citations
#[test]
fn rebuilding_the_dropped_index_returns_the_same_results_with_citations() {
    let env = Env::new();
    synced_once(&env);
    let before = env.json(&["search", "body", "--history"]);
    let results = before["result"]["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert!(
        results
            .iter()
            .all(|r| r["location"].as_str().unwrap().starts_with("https://x/"))
    );
    Connection::open(env.home.path().join("data/rastro.sqlite3"))
        .unwrap()
        .execute_batch("DROP TABLE evidence_fts")
        .unwrap();
    assert!(!env.rastro(&["search", "body"]).status.success());
    let rebuilt = env.json(&["index", "rebuild"]);
    assert_eq!(rebuilt["operation"], "index.rebuild");
    assert_eq!(rebuilt["result"]["indexed"], 2);
    let after = env.json(&["search", "body", "--history"]);
    assert_eq!(after["result"], before["result"]);
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 2);
    assert!(env.ok(&["index", "rebuild"]).contains("2"));
}

/// evidence/R54: every command validates against the operation schema on success and failure
#[test]
fn every_command_validates_against_the_operation_schema_on_success_and_failure() {
    let env = Env::new();
    // Successes.
    let added = env.json(&[
        "source",
        "add",
        "project",
        "--connector",
        "test",
        "--repo",
        "o/p",
        "--scope",
        "personal",
    ]);
    assert_eq!(added["operation"], "source.add");
    assert_eq!(env.json(&["source", "list"])["operation"], "source.list");
    assert_eq!(env.json(&["coverage"])["operation"], "coverage");
    assert_eq!(env.json(&["coverage", "project"])["operation"], "coverage");
    sync_with(&env, &[("https://x/1", "first body")]);
    assert_eq!(env.json(&["sync"])["operation"], "sync");
    assert_eq!(env.json(&["search", "body"])["operation"], "search");
    assert_eq!(
        env.json(&["search", "body", "--history"])["operation"],
        "search"
    );
    assert_eq!(env.json(&["search", "zzz"])["operation"], "search");
    assert_eq!(
        env.json(&["index", "rebuild"])["operation"],
        "index.rebuild"
    );
    // Failures that are errors.
    let errors: [(&[&str], &str); 4] = [
        (
            &[
                "source",
                "add",
                "project",
                "--connector",
                "test",
                "--repo",
                "o/p",
                "--scope",
                "personal",
            ],
            "source.add",
        ),
        (&["sync", "nobody"], "sync"),
        (&["coverage", "nobody"], "coverage"),
        (
            &[
                "source",
                "add",
                "bad",
                "--connector",
                "github",
                "--repo",
                "nope",
                "--scope",
                "x",
            ],
            "source.add",
        ),
    ];
    for (args, operation) in errors {
        let (ok, value) = env.json_any(args, None);
        assert!(!ok);
        assert_eq!(value["operation"], operation);
        assert!(value["error"].is_string(), "{value}");
    }
    let (ok, value) = env.json_any(&["coverage", "project"], Some("not a time"));
    assert!(!ok && value["error"].is_string(), "{value}");
    // A failed run is a failed result, still schema-valid.
    fs::remove_dir_all(env.home.path().join("dev-plugins/test")).unwrap();
    let (ok, value) = env.json_any(&["sync", "project"], None);
    assert!(!ok);
    assert_eq!(value["result"]["runs"][0]["status"], "failed");
    // With the index gone, search and rebuild fail or recover, each in the schema's shape.
    Connection::open(env.home.path().join("data/rastro.sqlite3"))
        .unwrap()
        .execute_batch("DROP TABLE evidence_fts")
        .unwrap();
    let (ok, value) = env.json_any(&["search", "body"], None);
    assert!(
        !ok && value["operation"] == "search" && value["error"].is_string(),
        "{value}"
    );
    // A config the core cannot read fails every command the same way.
    fs::write(env.home.path().join("config/config.toml"), "sources = 3").unwrap();
    for (args, operation) in [
        (&["source", "list"][..], "source.list"),
        (&["coverage"], "coverage"),
        (&["sync"], "sync"),
        (&["search", "x"], "search"),
        (&["search", "x", "--history"], "search"),
        (&["index", "rebuild"], "index.rebuild"),
    ] {
        let (ok, value) = env.json_any(args, None);
        assert!(!ok, "{args:?}");
        assert_eq!(value["operation"], operation);
        assert!(value["error"].is_string(), "{value}");
    }
}

/// evidence/R55: arguments that do not parse give a schema valid error with json
#[test]
fn arguments_that_do_not_parse_give_a_schema_valid_error_with_json() {
    let env = Env::new();
    let cases: [(&[&str], &str); 5] = [
        (&["search"], "search"),
        (
            &[
                "source",
                "add",
                "project",
                "--connector",
                "test",
                "--repo",
                "o/p",
            ],
            "source.add",
        ),
        (&["index", "rebuild", "unexpected"], "index.rebuild"),
        (&["sync", "a", "b"], "sync"),
        (&["coverage", "a", "b"], "coverage"),
    ];
    for (args, operation) in cases {
        for json_first in [false, true] {
            let mut full: Vec<&str> = args.to_vec();
            if json_first {
                full.insert(0, "--json");
            } else {
                full.push("--json");
            }
            let output = env.rastro(&full);
            assert_eq!(output.status.code(), Some(2), "{full:?}");
            let value: Value =
                serde_json::from_slice(&output.stdout).unwrap_or_else(|e| panic!("{full:?}: {e}"));
            validate(&value);
            assert_eq!(value["operation"], operation, "{full:?}");
            assert!(
                value["error"].as_str().unwrap().starts_with("usage:"),
                "{value}"
            );
        }
        // Without --json the plain parser output stays as it was.
        let plain = env.rastro(args);
        assert_eq!(plain.status.code(), Some(2));
        assert!(plain.stdout.is_empty());
    }
    for args in [
        &["--help", "--json"][..],
        &["search", "--help", "--json"],
        &["--version", "--json"],
    ] {
        let output = env.rastro(args);
        assert!(output.status.success(), "{args:?}");
        assert!(!output.stdout.is_empty());
        assert!(serde_json::from_slice::<Value>(&output.stdout).is_err());
    }
    assert!(env.rastro(&["--version"]).status.success());
}

/// evidence/R53: a rebuilt index behaves like a freshly migrated one
#[test]
fn a_rebuilt_index_behaves_like_a_freshly_migrated_one() {
    let bodies: [&[(&str, &str)]; 3] = [
        &[
            ("https://x/1", "running café"),
            ("https://x/2", "other thing"),
        ],
        &[("https://x/1", "runs naïve cafés")],
        &[("https://x/1", "running café")],
    ];
    let build = || {
        let env = Env::new();
        env.add_source("project");
        for round in bodies {
            sync_with(&env, round);
        }
        env
    };
    let fresh = build();
    let rebuilt = build();
    Connection::open(rebuilt.home.path().join("data/rastro.sqlite3"))
        .unwrap()
        .execute_batch("DROP TABLE evidence_fts")
        .unwrap();
    rebuilt.json(&["index", "rebuild"]);
    let definition = "SELECT sql FROM sqlite_master WHERE name = 'evidence_fts'";
    assert_eq!(
        fresh.sql::<String>(definition),
        rebuilt.sql::<String>(definition)
    );
    let cited = |env: &Env, args: &[&str]| -> Vec<(i64, String, String, String)> {
        env.json(args)["result"]["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["id"].as_i64().unwrap(),
                    r["location"].as_str().unwrap().into(),
                    r["field"].as_str().unwrap().into(),
                    r["quote"].as_str().unwrap().into(),
                )
            })
            .collect()
    };
    for query in ["run", "cafe", "naive", "café", "thing", "absent"] {
        for history in [false, true] {
            let mut args = vec!["search", query];
            if history {
                args.push("--history");
            }
            assert_eq!(cited(&fresh, &args), cited(&rebuilt, &args), "{args:?}");
        }
    }
    // The queries are not vacuous: stemming, diacritics and the latest-only rule all bite.
    assert_eq!(cited(&rebuilt, &["search", "run"]).len(), 1);
    assert_eq!(cited(&rebuilt, &["search", "cafe", "--history"]).len(), 3);
    assert_eq!(cited(&rebuilt, &["search", "naive"]).len(), 0);
}

/// evidence/R56: an argument that is not utf8 never panics
#[cfg(unix)]
#[test]
fn an_argument_that_is_not_utf8_never_panics() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let env = Env::new();
    let bad = || OsString::from_vec(vec![0xff, 0xfe]);
    for (words, operation) in [
        (vec!["search"], "search"),
        (vec!["sync"], "sync"),
        (vec!["coverage"], "coverage"),
        (vec!["index", "rebuild"], "index.rebuild"),
        (vec!["source", "add"], "source.add"),
    ] {
        let mut args: Vec<OsString> = words.iter().map(Into::into).collect();
        args.push(bad());
        let mut json = args.clone();
        json.push("--json".into());
        let output = env.rastro_os(json, None);
        assert_eq!(output.status.code(), Some(2), "{words:?}");
        let value: Value =
            serde_json::from_slice(&output.stdout).unwrap_or_else(|e| panic!("{words:?}: {e}"));
        validate(&value);
        assert_eq!(value["operation"], operation);
        let plain = env.rastro_os(args, None);
        assert_eq!(plain.status.code(), Some(2), "{words:?}");
        assert!(plain.stdout.is_empty());
    }
}
