#![allow(non_snake_case)]
mod support;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;

/// Two RFC 3339 timestamps name the same instant, however each is written
/// (`...01Z` and `...01.000Z` are equal).
fn same_instant(left: &Value, right: &Value, label: &str) {
    let instant = |v: &Value| -> chrono::DateTime<chrono::Utc> {
        v.as_str()
            .and_then(|t| t.parse().ok())
            .unwrap_or_else(|| panic!("{label}: not a timestamp: {v}"))
    };
    assert_eq!(instant(left), instant(right), "{label}: {left} vs {right}");
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
        serde_json::from_str(include_str!("../../../schema/operations-v2.schema.json")).unwrap();
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
    // The human line prints milliseconds, `--json` drops zero ones: same instant.
    let checked_at: chrono::DateTime<chrono::Utc> = checked.parse().unwrap();
    assert!(
        human.contains("unknown")
            && human.contains(&checked_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
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
            let mut args = vec!["search", query, "--full"];
            if history {
                args.push("--history");
            }
            assert_eq!(cited(&fresh, &args), cited(&rebuilt, &args), "{args:?}");
        }
    }
    // The queries are not vacuous: stemming, diacritics and the latest-only rule all bite.
    assert_eq!(cited(&rebuilt, &["search", "run", "--full"]).len(), 1);
    assert_eq!(
        cited(&rebuilt, &["search", "cafe", "--history", "--full"]).len(),
        3
    );
    assert_eq!(cited(&rebuilt, &["search", "naive", "--full"]).len(), 0);
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

fn raw_sync(env: &Env, messages: Vec<Value>) -> (bool, Value) {
    raw_sync_finishing(
        env,
        messages,
        json!({"do": "finish", "status": "ok", "submitted": 0}),
    )
}
fn raw_sync_finishing(env: &Env, messages: Vec<Value>, finish: Value) -> (bool, Value) {
    let mut steps = vec![json!({"do": "initialize"})];
    steps.extend(
        messages
            .into_iter()
            .map(|m| json!({"do": "send", "message": m})),
    );
    steps.push(finish);
    env.script(&steps);
    let (ok, value) = env.json_any(&["sync", "project"], None);
    (ok, value["result"]["runs"][0].clone())
}
fn report(params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": "coverage.report", "params": params})
}
fn windows(env: &Env) -> Vec<Value> {
    coverage_of(env, None)["windows"]
        .as_array()
        .unwrap()
        .clone()
}

/// evidence/R63: coverage lists each window with its state selection and retry time
#[test]
fn coverage_lists_each_window_with_its_state_selection_and_retry_time() {
    let env = Env::new();
    synced_once(&env);
    let retry = "2030-01-01T00:00:00Z";
    let (ok, run) = raw_sync(
        &env,
        vec![report(
            json!({"status": "couldnt_look", "reason": "rate_limited",
            "retry_at": retry, "selection": "authored"}),
        )],
    );
    assert!(!ok, "{run}");
    let listed = windows(&env);
    assert_eq!(listed.len(), 2, "{listed:?}");
    // Newest first.
    assert_eq!(listed[1]["state"], "found");
    assert_eq!(listed[1]["selection"], "all");
    assert_eq!(listed[0]["state"], "couldn't look");
    assert_eq!(listed[0]["selection"], "authored");
    assert_eq!(listed[0]["reason"], "rate_limited");
    same_instant(&listed[0]["retry_at"], &json!(retry), "retry_at");
    // Windows are half-open and consecutive: the failed window did not advance the start.
    same_instant(&listed[0]["from"], &listed[1]["to"], "");
    assert!(listed[1].get("retry_at").is_none());
    let human = env.ok(&["coverage", "project"]);
    for expected in [
        "rate_limited",
        "2030-01-01T00:00:00.000Z",
        "authored",
        "found",
    ] {
        assert!(human.contains(expected), "missing {expected}: {human}");
    }
    // The next sync starts where the last complete window ended, not after the failed one.
    let next = sync_with(&env, &[("https://x/1", "first body")]);
    same_instant(&next["window"]["from"], &listed[1]["to"], "");
}

/// evidence/R64: a quiet later window is empty in its row and the source stays found
#[test]
fn a_quiet_later_window_is_empty_in_its_row_and_the_source_stays_found() {
    let env = Env::new();
    synced_once(&env);
    let quiet = raw_sync(
        &env,
        vec![report(json!({"status": "complete", "selection": "all"}))],
    );
    assert_eq!(quiet.1["coverage"], "empty", "{quiet:?}");
    let source = coverage_of(&env, None);
    assert_eq!(source["state"], "found");
    let listed = windows(&env);
    assert_eq!(
        listed
            .iter()
            .map(|w| w["state"].clone())
            .collect::<Vec<_>>(),
        ["empty", "found"]
    );
    let hit = env.json(&["search", "body"]);
    assert_eq!(hit["result"]["results"][0]["coverage"]["state"], "found");
    assert_eq!(hit["result"]["coverage"][0]["state"], "found");
    // Only a source whose every complete window was empty is empty.
    let fresh = Env::new();
    fresh.add_source("project");
    raw_sync(
        &fresh,
        vec![report(json!({"status": "complete", "selection": "all"}))],
    );
    raw_sync(
        &fresh,
        vec![report(json!({"status": "complete", "selection": "all"}))],
    );
    assert_eq!(coverage_of(&fresh, None)["state"], "empty");
}

/// evidence/R65: a run whose submissions are all rejected is couldnt look protocol never empty
#[test]
fn a_run_whose_submissions_are_all_rejected_is_couldnt_look_protocol_never_empty() {
    let env = Env::new();
    env.add_source("project");
    let bad = json!({"jsonrpc": "2.0", "method": "evidence.submit", "params": {"kind": "issue"}});
    let (ok, run) = raw_sync(
        &env,
        vec![
            bad.clone(),
            bad,
            report(json!({"status": "complete", "selection": "all"})),
        ],
    );
    assert!(!ok);
    assert_eq!(run["counts"]["rejected"], 2);
    assert_eq!(run["reason"], "protocol");
    assert_eq!(run["coverage"], "couldn't look");
    let listed = windows(&env);
    assert_eq!(listed[0]["state"], "couldn't look");
    assert_eq!(listed[0]["reason"], "protocol");
    assert_eq!(coverage_of(&env, None)["state"], "unknown");
}

/// evidence/R66: some rejected submissions keep the accepted evidence but the window is couldnt look
#[test]
fn some_rejected_submissions_keep_the_accepted_evidence_but_the_window_is_couldnt_look() {
    let env = Env::new();
    env.add_source("project");
    let bad = json!({"jsonrpc": "2.0", "method": "evidence.submit", "params": {"kind": "issue"}});
    let (ok, run) = raw_sync(
        &env,
        vec![
            submit("https://x/1", "kept body"),
            bad,
            report(json!({"status": "complete", "selection": "all"})),
        ],
    );
    assert!(!ok);
    assert_eq!(run["counts"]["accepted"], 1);
    assert_eq!(run["counts"]["rejected"], 1);
    assert_eq!(run["coverage"], "couldn't look");
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 1);
    let listed = windows(&env);
    assert_eq!(listed[0]["state"], "couldn't look");
    assert_eq!(listed[0]["reason"], "protocol");
    // The window did not complete, so the next sync reads it again.
    let again = sync_with(&env, &[("https://x/1", "kept body")]);
    same_instant(&again["window"]["from"], &listed[0]["from"], "");
}

/// evidence/R68: a connector that isnt installed is couldnt look not installed a published reason
#[test]
fn a_connector_that_isnt_installed_is_couldnt_look_not_installed_a_published_reason() {
    let env = Env::new();
    env.add_source("project");
    fs::remove_dir_all(env.home.path().join("dev-plugins/test")).unwrap();
    let (ok, value) = env.json_any(&["sync", "project"], None);
    assert!(!ok);
    let run = &value["result"]["runs"][0];
    assert_eq!(
        (&run["status"], &run["reason"]),
        (&json!("failed"), &json!("not_installed"))
    );
    let listed = windows(&env);
    assert_eq!(listed[0]["state"], "couldn't look");
    assert_eq!(listed[0]["reason"], "not_installed");
    // It is a published typed reason: documented, and accepted when a connector reports it.
    let decisions = include_str!("../../../docs/decisions/knowledge.md");
    let published = decisions.lines().find(|l| l.starts_with("18.")).unwrap();
    assert!(published.contains("`not_installed`"), "{published}");
    let env = Env::new();
    env.add_source("project");
    let (_, run) = raw_sync(
        &env,
        vec![report(
            json!({"status": "couldnt_look", "reason": "not_installed"}),
        )],
    );
    assert_eq!(run["reason"], "not_installed", "{run}");
}

/// evidence/R69: a manifest that fails its check is a failed run and other sources still run
#[test]
fn a_manifest_that_fails_its_check_is_a_failed_run_and_other_sources_still_run() {
    type Edit = fn(&mut toml::Table);
    let edits: [(&str, Edit); 3] = [
        ("protocol major", |m| {
            m.insert("protocol".into(), 2.into());
        }),
        ("not a connector", |m| {
            m.insert("kinds".into(), vec![toml::Value::from("extractor")].into());
        }),
        ("holds actions", |m| {
            m["permissions"]
                .as_table_mut()
                .unwrap()
                .insert("actions".into(), vec![toml::Value::from("post")].into());
        }),
    ];
    for (label, edit) in edits {
        let env = Env::new();
        // A second connector whose manifest fails its check.
        let bad = env.home.path().join("dev-plugins/bad");
        fs::create_dir_all(&bad).unwrap();
        let mut manifest: toml::Table =
            include_str!("../../rastro-test-connector/rastro-plugin.toml")
                .parse()
                .unwrap();
        manifest.insert("id".into(), "bad".into());
        edit(&mut manifest);
        fs::write(bad.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
        env.ok(&[
            "source",
            "add",
            "broken",
            "--connector",
            "bad",
            "--repo",
            "o/p",
            "--scope",
            "personal",
        ]);
        env.add_source("project");
        env.script(&[
            json!({"do": "initialize"}),
            json!({"do": "send", "message": submit("https://x/1", "kept body")}),
            json!({"do": "send", "message": report(json!({"status": "complete", "selection": "all"}))}),
            json!({"do": "finish", "status": "ok", "submitted": 1}),
        ]);
        let (ok, value) = env.json_any(&["sync"], None);
        assert!(!ok, "{label}");
        let runs = value["result"]["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 2, "{label}: {value}");
        let broken = runs.iter().find(|r| r["source"] == "broken").unwrap();
        assert_eq!(
            (&broken["status"], &broken["reason"], &broken["coverage"]),
            (
                &json!("failed"),
                &json!("protocol"),
                &json!("couldn't look")
            ),
            "{label}"
        );
        let other = runs.iter().find(|r| r["source"] == "project").unwrap();
        assert_eq!(other["status"], "ok", "{label}: {other}");
        assert_eq!(
            env.sql::<i64>("SELECT count(*) FROM evidence"),
            1,
            "{label}"
        );
        assert_eq!(
            env.sql::<String>(
                "SELECT state || ':' || reason FROM coverage WHERE source = 'broken'"
            ),
            "couldn't look:protocol",
            "{label}"
        );
        assert_eq!(env.sql::<i64>("SELECT count(*) FROM runs"), 2, "{label}");
    }
}

/// evidence/R67: a rejected submission wins over a reported failure or a failed finish
#[test]
fn a_rejected_submission_wins_over_a_reported_failure_or_a_failed_finish() {
    let bad = json!({"jsonrpc": "2.0", "method": "evidence.submit", "params": {"kind": "issue"}});
    let reported = report(json!({"status": "couldnt_look", "reason": "rate_limited",
        "retry_at": "2030-01-01T00:00:00Z", "selection": "all"}));
    let complete = report(json!({"status": "complete", "selection": "all"}));
    let ok_finish = json!({"do": "finish", "status": "ok", "submitted": 2});
    let auth_finish = json!({"do": "finish", "status": "failed", "reason": "auth", "submitted": 2});
    for (coverage, finish, label) in [
        (reported.clone(), ok_finish.clone(), "reported rate limit"),
        (complete.clone(), auth_finish.clone(), "failed finish"),
        (reported, auth_finish, "both"),
    ] {
        let env = Env::new();
        env.add_source("project");
        let (ok, run) = raw_sync_finishing(
            &env,
            vec![submit("https://x/1", "kept body"), bad.clone(), coverage],
            finish,
        );
        assert!(!ok, "{label}");
        assert_eq!(run["reason"], "protocol", "{label}: {run}");
        assert_eq!(run["counts"]["accepted"], 1, "{label}");
        assert_eq!(run["counts"]["rejected"], 1, "{label}");
        assert_eq!(
            env.sql::<i64>("SELECT count(*) FROM evidence"),
            1,
            "{label}"
        );
        let listed = windows(&env);
        assert_eq!(listed[0]["state"], "couldn't look", "{label}");
        assert_eq!(listed[0]["reason"], "protocol", "{label}");
        // The retry time belonged to a reason that no longer explains the window.
        assert!(listed[0].get("retry_at").is_none(), "{label}: {listed:?}");
        // A couldn't-look window does not advance the next sync's start.
        let again = sync_with(&env, &[("https://x/1", "kept body")]);
        same_instant(&again["window"]["from"], &listed[0]["from"], label);
    }
}

/// evidence/R70: manifest parse diagnostics name the position and never echo values
#[test]
fn manifest_parse_diagnostics_name_the_position_and_never_echo_values() {
    let env = Env::new();
    env.add_source("project");
    let broken = env.home.path().join("dev-plugins/broken");
    fs::create_dir_all(&broken).unwrap();
    fs::write(
        broken.join("rastro-plugin.toml"),
        "id = \"test-secret-token-1234\"\nname = \"x\"\nversion = \n",
    )
    .unwrap();
    env.script(&[json!({"do": "initialize"})]);
    let output = env.rastro(&["sync", "project"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("rastro-plugin.toml"), "{stderr}");
    assert!(stderr.contains("line 3"), "{stderr}");
    assert!(!stderr.contains("test-secret-token-1234"), "{stderr}");
    assert!(!stderr.contains("version ="), "{stderr}");
}

/// S21a: a last-checked time on a whole second is the same instant in human output and --json
/// evidence/R58: a whole second last checked is the same instant in human and json
#[test]
fn a_whole_second_last_checked_is_the_same_instant_in_human_and_json() {
    let env = Env::new();
    synced_once(&env);
    Connection::open(env.home.path().join("data/rastro.sqlite3"))
        .unwrap()
        .execute(
            "UPDATE coverage SET checked_at = '2026-11-04T10:00:00.000Z'",
            [],
        )
        .unwrap();
    let json = coverage_of(&env, Some("2026-11-04T10:05:00Z"));
    let from_json: chrono::DateTime<chrono::Utc> =
        json["last_checked"].as_str().unwrap().parse().unwrap();
    let human = env.rastro_at(&["coverage", "project"], Some("2026-11-04T10:05:00Z"));
    let human = String::from_utf8(human.stdout).unwrap();
    let printed = human
        .split("Last checked: ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("no last checked in {human}"));
    let from_human: chrono::DateTime<chrono::Utc> = printed.parse().unwrap();
    assert_eq!(from_json, from_human, "{human} vs {json}");
    assert_eq!(from_json.timestamp(), 1_793_786_400);
}

/// A found window, then `quiet` empty ones: the only evidence is the oldest window.
fn found_then_quiet(env: &Env, quiet: usize) {
    synced_once(env);
    for _ in 0..quiet {
        let (ok, run) = raw_sync(
            env,
            vec![report(json!({"status": "complete", "selection": "all"}))],
        );
        assert!(ok, "{run}");
    }
}

/// evidence/R63: a source with more than ten windows lists its ten newest, newest first
#[test]
fn a_source_with_more_than_ten_windows_lists_the_ten_newest_newest_first() {
    let env = Env::new();
    found_then_quiet(&env, 11);
    let source = coverage_of(&env, None);
    assert_eq!(source["windows_total"], 12, "{source}");
    let listed = source["windows"].as_array().unwrap().clone();
    assert_eq!(listed.len(), 10, "{listed:?}");
    assert!(listed.iter().all(|w| w["state"] == "empty"), "{listed:?}");
    let starts: Vec<chrono::DateTime<chrono::Utc>> = listed
        .iter()
        .map(|w| w["from"].as_str().unwrap().parse().unwrap())
        .collect();
    assert!(starts.windows(2).all(|p| p[0] >= p[1]), "{starts:?}");
    let human = env.ok(&["coverage", "project"]);
    assert!(human.contains("showing 10 of 12"), "{human}");

    let (ok, all) = env.json_any(&["coverage", "project", "--all-windows"], None);
    assert!(ok, "{all}");
    let source = &all["result"]["coverage"][0];
    assert_eq!(source["windows_total"], 12);
    let every = source["windows"].as_array().unwrap();
    assert_eq!(every.len(), 12);
    assert_eq!(every[11]["state"], "found");
    assert_eq!(every[0], listed[0]);
    let human = env.ok(&["coverage", "project", "--all-windows"]);
    assert!(!human.contains("showing"), "{human}");
}

/// evidence/R64: the only window that found evidence being outside the newest ten keeps "found"
#[test]
fn the_only_found_window_older_than_the_newest_ten_still_answers_found() {
    let env = Env::new();
    found_then_quiet(&env, 11);
    let source = coverage_of(&env, None);
    assert!(
        source["windows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w["state"] == "empty")
    );
    assert_eq!(source["state"], "found");
    let hit = env.json(&["search", "body"]);
    let hits = hit["result"]["results"].as_array().unwrap();
    assert!(!hits.is_empty());
    for h in hits {
        assert_eq!(h["coverage"]["state"], "found", "{h}");
    }
    assert_eq!(hit["result"]["coverage"][0]["state"], "found");
}

fn edit_sources(env: &Env, edit: impl FnOnce(&mut Vec<toml::Value>)) {
    let path = env.home.path().join("config/config.toml");
    let mut config: toml::Table = fs::read_to_string(&path).unwrap().parse().unwrap();
    let sources = config
        .get_mut("sources")
        .and_then(toml::Value::as_array_mut)
        .unwrap();
    edit(sources);
    fs::write(path, config.to_string()).unwrap();
}
fn all_coverage(env: &Env, now: Option<&str>) -> Vec<Value> {
    let (ok, value) = env.json_any(&["coverage"], now);
    assert!(ok, "{value}");
    value["result"]["coverage"].as_array().unwrap().clone()
}

/// surfaces/R7: a removed source's hits and entry carry its stored coverage, registered false
#[test]
fn a_removed_source_keeps_its_stored_coverage_marked_unregistered_until_it_goes_unknown() {
    let env = Env::new();
    synced_once(&env);
    edit_sources(&env, |sources| sources.clear());
    let entries = all_coverage(&env, None);
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0]["source"], "project");
    assert_eq!(entries[0]["state"], "found");
    assert_eq!(entries[0]["registered"], false);
    assert_eq!(entries[0]["windows_total"], 1);
    let named = env.json(&["coverage", "project"]);
    assert_eq!(named["result"]["coverage"][0]["registered"], false);
    let hits = env.json(&["search", "body"]);
    let results = hits["result"]["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    for hit in results {
        assert_eq!(hit["coverage"]["state"], "found", "{hit}");
        assert_eq!(hit["coverage"]["registered"], false, "{hit}");
    }
    // Past its freshness rule the stored windows no longer vouch for it.
    let later = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
    let stale = all_coverage(&env, Some(&later));
    assert_eq!(stale[0]["state"], "unknown");
    assert_eq!(stale[0]["registered"], false);
    let (_, hits) = env.json_any(&["search", "body"], Some(&later));
    assert_eq!(hits["result"]["results"][0]["coverage"]["state"], "unknown");
    let human = env.ok(&["coverage"]);
    assert!(
        human.contains("project") && human.contains("Not in config"),
        "{human}"
    );
}

/// surfaces/R8: a removed source that never synced is listed as never looked, unregistered
#[test]
fn a_removed_source_that_never_synced_is_never_looked_and_unregistered() {
    let env = Env::new();
    env.add_source("project");
    env.add_source("other");
    edit_sources(&env, |sources| {
        sources.retain(|s| s["name"].as_str() == Some("other"))
    });
    let entries = all_coverage(&env, None);
    let by_name = |name: &str| {
        entries
            .iter()
            .find(|e| e["source"] == name)
            .unwrap()
            .clone()
    };
    assert_eq!(entries.len(), 2, "{entries:?}");
    assert_eq!(by_name("other")["registered"], true);
    assert_eq!(by_name("project")["state"], "never looked");
    assert_eq!(by_name("project")["registered"], false);
    assert_eq!(by_name("project")["windows_total"], 0);
}

/// surfaces/R9: a changed scope leaves the old pair's coverage on its hits, unregistered
#[test]
fn hits_under_the_old_scope_carry_the_old_pairs_coverage_and_new_ones_the_new() {
    let env = Env::with_manifest(|manifest| {
        manifest["permissions"]["scopes"] = json_scopes();
    });
    synced_once(&env);
    edit_sources(&env, |sources| {
        sources[0]["scope"] = "work".into();
    });
    // The writing path projects the new scope; sync once under it.
    let run = sync_with(&env, &[("https://x/3", "third body")]);
    assert_eq!(run["coverage"], "found", "{run}");
    let entries = all_coverage(&env, None);
    assert_eq!(entries.len(), 2, "{entries:?}");
    let pair = |scope: &str| {
        entries
            .iter()
            .find(|e| e["scope"] == scope)
            .unwrap_or_else(|| panic!("no {scope}: {entries:?}"))
            .clone()
    };
    assert_eq!(pair("personal")["registered"], false);
    assert_eq!(pair("personal")["state"], "found");
    assert_eq!(pair("work")["registered"], true);
    let hits = env.json(&["search", "body"]);
    let results = hits["result"]["results"].as_array().unwrap();
    assert_eq!(results.len(), 3, "{results:?}");
    for hit in results {
        let scope = hit["scope"].as_str().unwrap();
        assert_eq!(hit["coverage"]["scope"], scope, "{hit}");
        assert_eq!(hit["coverage"]["registered"], scope == "work", "{hit}");
        assert_eq!(hit["coverage"]["state"], "found", "{hit}");
    }
    // Hours on, the old pair goes stale under the source's rule; so does the new one.
    let later = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
    let (_, aged) = env.json_any(&["search", "body"], Some(&later));
    for hit in aged["result"]["results"].as_array().unwrap() {
        assert_eq!(hit["coverage"]["state"], "unknown", "{hit}");
    }
    // A name in neither config nor storage is unknown while storage holds other pairs.
    assert!(!env.rastro(&["coverage", "nosuch"]).status.success());
}
fn json_scopes() -> toml::Value {
    toml::Value::Array(vec!["personal".into(), "work".into()])
}
