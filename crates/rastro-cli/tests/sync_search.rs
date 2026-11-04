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
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    /// A command that must fail (exit non-zero) yet still print a schema-valid result.
    fn json_failing(&self, args: &[&str]) -> Value {
        let output = self.rastro(&[args, &["--json"]].concat());
        assert!(!output.status.success());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        check_schema(&value);
        value
    }
    fn json(&self, args: &[&str]) -> Value {
        let value: Value = serde_json::from_str(&self.ok(&[args, &["--json"]].concat())).unwrap();
        check_schema(&value);
        value
    }
    fn add_source(&self, name: &str) {
        self.add_source_with(name, &[]);
    }
    fn add_source_with(&self, name: &str, extra: &[&str]) {
        self.ok(&[
            &[
                "source",
                "add",
                name,
                "--connector",
                "test",
                "--repo",
                "o/p",
                "--scope",
                "personal",
            ][..],
            extra,
        ]
        .concat());
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

fn check_schema(value: &Value) {
    let schema: Value =
        serde_json::from_str(include_str!("../../../schema/operations-v1.schema.json")).unwrap();
    jsonschema::validator_for(&schema)
        .unwrap()
        .validate(value)
        .unwrap_or_else(|e| panic!("{e}: {value}"));
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

/// evidence/R6: sync stores evidence and found coverage
#[test]
fn sync_stores_evidence_and_found_coverage() {
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

/// evidence/R7: complete sync without evidence records empty
#[test]
fn complete_sync_without_evidence_records_empty() {
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

/// evidence/R8: evidence without location span quote or trust class is rejected
#[test]
fn evidence_without_location_span_quote_or_trust_class_is_rejected() {
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
    // Rejected submissions mean the window was not read as promised, though the accepted
    // evidence stays.
    let synced = env.json_failing(&["sync", "project"]);
    assert_eq!(synced["result"]["runs"][0]["reason"], "protocol");
    assert_eq!(
        synced["result"]["runs"][0]["counts"],
        json!({"submitted": 9, "accepted": 1, "rejected": 8, "unchanged": 0})
    );
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 1);
    assert_eq!(env.sql::<i64>("SELECT rejected FROM runs"), 8);
    let human = String::from_utf8(env.rastro(&["sync", "project"]).stdout).unwrap();
    assert!(human.contains("rejected 8"), "{human}");
}

/// evidence/R9: forbidden kind or forged source or scope is rejected
#[test]
fn forbidden_kind_or_forged_source_or_scope_is_rejected() {
    let env = Env::new();
    env.add_source("project");
    let good = valid("https://github.com/o/p/issues/1", "hello world");
    env.script(&run_script(vec![
        with(good.clone(), "kind", json!("commit")),
        with(good.clone(), "source", json!("other")),
        with(good.clone(), "scope", json!("work")),
        complete(),
    ]));
    let synced = env.json_failing(&["sync", "project"]);
    assert_eq!(synced["result"]["runs"][0]["reason"], "protocol");
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

/// evidence/R49: search results carry the citation and coverage
#[test]
fn search_results_carry_the_citation_and_coverage() {
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

/// evidence/R50: search without a match states the coverage of each source
#[test]
fn search_without_a_match_states_the_coverage_of_each_source() {
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

/// evidence/R6: sync without manifest records couldnt look
#[test]
fn sync_without_manifest_records_couldnt_look() {
    let env = Env::new();
    env.ok(&[
        "source",
        "add",
        "ghost",
        "--connector",
        "missing",
        "--repo",
        "o/p",
        "--scope",
        "personal",
    ]);
    let output = env.rastro(&["sync", "ghost", "--json"]);
    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let run = &value["result"]["runs"][0];
    assert_eq!(run["status"], "failed");
    assert_eq!(run["reason"], "not_installed");
    assert_eq!(run["coverage"], "couldn't look");
    assert_eq!(
        env.sql::<String>("SELECT reason FROM runs"),
        "not_installed"
    );
    assert_eq!(
        env.sql::<String>("SELECT state || ':' || reason FROM coverage"),
        "couldn't look:not_installed"
    );
    let human = env.rastro(&["sync", "ghost"]);
    assert!(!human.status.success());
    assert!(String::from_utf8_lossy(&human.stdout).contains("not_installed"));
}

fn set_permission(manifest: &mut toml::Table, key: &str, values: &[&str]) {
    let values: Vec<toml::Value> = values.iter().map(|v| (*v).into()).collect();
    manifest["permissions"]
        .as_table_mut()
        .unwrap()
        .insert(key.into(), values.into());
}

/// evidence/R10: source scope outside the manifest scopes is refused
#[test]
fn source_scope_outside_the_manifest_scopes_is_refused() {
    let env = Env::with_manifest(|m| set_permission(m, "scopes", &["work"]));
    env.add_source("project");
    env.script(&run_script(vec![valid("https://x/1", "hello"), complete()]));
    let output = env.rastro(&["sync", "project", "--json"]);
    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let run = &value["result"]["runs"][0];
    assert_eq!(
        (&run["status"], &run["reason"]),
        (&json!("failed"), &json!("forbidden"))
    );
    assert_eq!(run["coverage"], "couldn't look");
    assert_eq!(
        env.sql::<String>("SELECT reason FROM coverage"),
        "forbidden"
    );
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 0);
    // An empty list declares no scopes at all.
    let env = Env::with_manifest(|m| set_permission(m, "scopes", &[]));
    env.add_source("project");
    assert!(!env.rastro(&["sync", "project"]).status.success());
}

/// evidence/R10: a window other than the bound one is rejected
#[test]
fn a_window_other_than_the_bound_one_is_rejected() {
    let env = Env::new();
    env.add_source("project");
    let forged = json!({"from": "1900-01-01T00:00:00Z", "to": "1900-02-01T00:00:00Z"});
    let mut report = complete();
    report["params"]["window"] = forged.clone();
    env.script(&run_script(vec![
        with(valid("https://x/1", "hello"), "window", forged),
        report,
    ]));
    let output = env.rastro(&["sync", "project", "--json"]);
    assert!(!output.status.success());
    let synced: Value = serde_json::from_slice(&output.stdout).unwrap();
    let run = &synced["result"]["runs"][0];
    assert_eq!(
        (&run["status"], &run["reason"]),
        (&json!("failed"), &json!("protocol"))
    );
    assert_eq!(
        run["counts"],
        json!({"submitted": 1, "accepted": 0, "rejected": 1, "unchanged": 0})
    );
    assert_eq!(run["coverage"], "couldn't look");
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 0);
}

/// evidence/R11: connector sees only allowed and declared environment
#[test]
fn connector_sees_only_allowed_and_declared_environment() {
    let env = Env::with_manifest(|m| {
        set_permission(m, "env", &["RASTRO_TEST_SCRIPT", "RASTRO_DECLARED_VAR"]);
    });
    env.add_source("project");
    let dump = env.home.path().join("env.txt");
    env.script(&[
        json!({"do": "initialize"}),
        json!({"do": "dump_env", "path": dump}),
        json!({"do": "send", "message": complete()}),
        json!({"do": "finish"}),
    ]);
    env.ok(&["sync", "project"]);
    let seen = fs::read_to_string(dump).unwrap();
    assert!(
        seen.contains("RASTRO_DECLARED_VAR=declared-value"),
        "{seen}"
    );
    assert!(seen.contains("PATH="), "{seen}");
    assert!(
        !seen.contains("RASTRO_SENTINEL_SECRET") && !seen.contains("hunter2"),
        "{seen}"
    );
    assert!(!seen.contains("RASTRO_HOME"), "{seen}");
}

/// evidence/R12: control characters from connectors are escaped in human output
#[test]
fn control_characters_from_connectors_are_escaped_in_human_output() {
    let env = Env::new();
    env.add_source("project");
    let esc = "\u{1b}[31mred";
    env.script(&run_script(vec![
        with(valid("https://x/1", "hello"), "kind", json!(esc)),
        valid(&format!("https://x/{esc}"), &format!("fox {esc}")),
        complete(),
    ]));
    let output = env.rastro(&["sync", "project"]);
    assert!(!output.stdout.contains(&0x1b) && !output.stderr.contains(&0x1b));
    assert!(String::from_utf8_lossy(&output.stderr).contains("\\u{1b}"));
    let human = env.ok(&["search", "fox"]);
    assert!(!human.contains('\u{1b}'), "{human:?}");
    assert!(human.contains("\\u{1b}[31mred"), "{human}");
    let exact = env.json(&["search", "fox"]);
    assert_eq!(exact["result"]["results"][0]["quote"], format!("fox {esc}"));
    env.script(&[
        json!({"do": "initialize"}),
        json!({"do": "send", "message": complete()}),
        json!({"do": "finish", "status": "failed", "reason": esc}),
    ]);
    let failed = env.rastro(&["sync", "project"]);
    assert!(!failed.stdout.contains(&0x1b) && !failed.stderr.contains(&0x1b));
}

/// S25: the core keeps an original only for a source with the flag, whatever the connector sends
#[test]
fn S25_core_drops_an_original_from_a_source_without_the_flag() {
    let original = json!({"id": 7, "body": "hello world", "extra": [1, 2]});
    let script = run_script(vec![
        with(
            valid("https://github.com/o/p/issues/1", "hello world"),
            "original",
            original.clone(),
        ),
        complete(),
    ]);
    let plain = Env::new();
    plain.add_source("project");
    plain.script(&script);
    plain.json(&["sync", "project"]);
    assert_eq!(
        plain.sql::<i64>("SELECT count(*) FROM evidence WHERE original IS NULL"),
        1
    );
    let found = plain.json(&["search", "hello"]);
    assert!(found["result"]["results"][0].get("original").is_none());

    let keeping = Env::new();
    keeping.add_source_with("project", &["--keep-original"]);
    keeping.script(&script);
    keeping.json(&["sync", "project"]);
    let found = keeping.json(&["search", "hello"]);
    assert_eq!(found["result"]["results"][0]["original"], original);
}

/// S25a: an original that isn't a JSON object is rejected, nothing is stored, search stays schema-valid
#[test]
fn S25a_a_non_object_original_is_rejected_for_a_source_that_keeps_originals() {
    for bad in [json!("not-an-object"), json!([1, 2]), json!(7), json!(true)] {
        let env = Env::new();
        env.add_source_with("project", &["--keep-original"]);
        env.script(&run_script(vec![
            with(
                valid("https://github.com/o/p/issues/1", "hello world"),
                "original",
                bad.clone(),
            ),
            complete(),
        ]));
        let synced = env.json_failing(&["sync", "project"]);
        let run = synced["result"]["runs"][0].clone();
        assert_eq!(run["status"], "failed", "{bad}: {run}");
        assert_eq!(run["reason"], "protocol", "{bad}: {run}");
        assert_eq!(run["counts"]["rejected"], 1, "{bad}: {run}");
        assert_eq!(run["counts"]["accepted"], 0, "{bad}: {run}");
        assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 0);
        let found = env.json(&["search", "hello"]);
        assert_eq!(found["result"]["results"], json!([]));
    }
}

/// S25b: an explicit null original is rejected for a source that keeps originals; omitted is fine
#[test]
fn S25b_a_null_original_is_rejected_but_an_omitted_one_is_accepted() {
    let message = || valid("https://github.com/o/p/issues/1", "hello world");
    let env = Env::new();
    env.add_source_with("project", &["--keep-original"]);
    env.script(&run_script(vec![
        with(message(), "original", Value::Null),
        complete(),
    ]));
    let run = env.json_failing(&["sync", "project"])["result"]["runs"][0].clone();
    assert_eq!(run["reason"], "protocol", "{run}");
    assert_eq!(run["counts"]["rejected"], 1, "{run}");
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 0);

    let omitted = Env::new();
    omitted.add_source_with("project", &["--keep-original"]);
    omitted.script(&run_script(vec![message(), complete()]));
    let run = omitted.json(&["sync", "project"])["result"]["runs"][0].clone();
    assert_eq!(run["counts"]["accepted"], 1, "{run}");

    let plain = Env::new();
    plain.add_source("project");
    plain.script(&run_script(vec![
        with(message(), "original", Value::Null),
        complete(),
    ]));
    let run = plain.json(&["sync", "project"])["result"]["runs"][0].clone();
    assert_eq!(run["counts"]["accepted"], 1, "{run}");
}
