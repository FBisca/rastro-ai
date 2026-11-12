#![allow(non_snake_case)]
mod support;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Output, Stdio},
    time::Instant,
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
        self.command(args).output().unwrap()
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(support::rastro());
        command
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.home.path().join("dev-plugins"))
            .env("RASTRO_SENTINEL_SECRET", "hunter2")
            .env("RASTRO_DECLARED_VAR", "declared-value")
            .env("RASTRO_TEST_SCRIPT", self.home.path().join("script.jsonl"))
            .env(
                "RASTRO_TEST_EXTRACTOR_SCRIPT",
                self.home.path().join("extractor.jsonl"),
            )
            .args(args);
        command
    }
    /// What `rastro` wrote to stderr for `args`, which must have succeeded.
    fn stderr(&self, args: &[&str]) -> String {
        let output = self.rastro(args);
        assert!(
            output.status.success(),
            "stdout: {} stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stderr).unwrap()
    }
    /// The test extractor beside the connector, which now binds it.
    fn with_extractor() -> Self {
        let env = Self::with_manifest(|m| {
            m.insert("default_extractor".into(), "test-extractor".into());
        });
        let dir = env.home.path().join("dev-plugins/test-extractor");
        fs::create_dir_all(&dir).unwrap();
        env.extractor_manifest("0.1.0");
        fs::write(
            dir.join("vocabulary.toml"),
            include_str!("../../rastro-test-extractor/vocabulary.toml"),
        )
        .unwrap();
        env
    }
    fn extractor_manifest(&self, version: &str) {
        let mut manifest: toml::Table =
            include_str!("../../rastro-test-extractor/rastro-plugin.toml")
                .parse()
                .unwrap();
        manifest.insert(
            "command".into(),
            support::test_extractor().to_str().unwrap().into(),
        );
        manifest.insert("version".into(), version.into());
        fs::write(
            self.home
                .path()
                .join("dev-plugins/test-extractor/rastro-plugin.toml"),
            manifest.to_string(),
        )
        .unwrap();
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

/// evidence/R73: reread starts at registration and appends only new fields
#[test]
fn reread_starts_at_registration_and_appends_only_new_fields() {
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

const MARKER: &str = "test-connector-started";
const WINDOW_START: &str = "2026-01-01T00:00:00.000Z";

fn complete() -> Value {
    json!({"do": "send", "message": {"jsonrpc": "2.0", "method": "coverage.report",
        "params": {"status": "complete", "selection": "all"}}})
}
/// Marker first: whatever stderr shows before it was printed before the read began.
fn script_bodies(env: &Env, bodies: &[(&str, &str)]) {
    let mut steps = vec![
        json!({"do": "stderr", "text": MARKER}),
        json!({"do": "initialize"}),
    ];
    for (location, quote) in bodies {
        steps.push(json!({"do": "send", "message": submit(location, quote)}));
    }
    steps.push(complete());
    steps.push(json!({"do": "finish", "status": "ok", "submitted": bodies.len()}));
    env.script(&steps);
}
fn add_windowed(env: &Env, name: &str) {
    env.ok(&[
        "source",
        "add",
        name,
        "--connector",
        "test",
        "--repo",
        "o/p",
        "--scope",
        "personal",
        "--window-start",
        WINDOW_START,
    ]);
}
fn at(stderr: &str, what: &str, test: impl Fn(&str) -> bool) -> usize {
    stderr
        .lines()
        .position(test)
        .unwrap_or_else(|| panic!("no stderr line {what}:\n{stderr}"))
}

/// sync-progress/S13: before the read, stderr names the source and why it reads what it reads
#[test]
fn S13_stderr_names_the_source_and_its_read_reason_before_the_read() {
    let env = Env::new();
    add_windowed(&env, "project");
    let bodies = [("https://x/1", "first body")];
    for (args, reason) in [
        (vec!["sync", "project"], "first read from"),
        (vec!["sync", "project"], "incremental since"),
        (vec!["sync", "project", "--reread"], "re-read from"),
    ] {
        script_bodies(&env, &bodies);
        let err = env.stderr(&args);
        let line = at(&err, reason, |l| {
            l.contains("project") && l.contains(reason)
        });
        let marker = at(&err, "the connector marker", |l| l.contains(MARKER));
        assert!(line < marker, "{reason} came after the read began:\n{err}");
        let text = err.lines().nth(line).unwrap();
        if reason != "incremental since" {
            assert!(text.contains("2026-01-01"), "not from window_start: {text}");
        } else {
            assert!(
                !text.contains("2026-01-01"),
                "not since the window end: {text}"
            );
        }
    }
}

/// sync-progress/S14: read, extract and project show in order, and a replay says so
#[test]
fn S14_stderr_shows_read_extract_and_project_in_order_and_a_replay_says_so() {
    let env = Env::with_extractor();
    add_windowed(&env, "project");
    script_bodies(
        &env,
        &[("https://x/1", "first body"), ("https://x/2", "second")],
    );
    let stage = |err: &str, name: &str| {
        at(err, name, |l| {
            l.contains("project") && l.contains(&format!("stage {name}"))
        })
    };
    let first = env.stderr(&["sync", "project"]);
    let (read, extract, project) = (
        stage(&first, "read"),
        stage(&first, "extract"),
        stage(&first, "project"),
    );
    assert!(read < extract && extract < project, "{first}");
    assert!(
        !first
            .lines()
            .nth(extract)
            .unwrap()
            .to_lowercase()
            .contains("replay"),
        "a first extraction is no replay:\n{first}"
    );
    // The extractor's version changes: the next sync reads nothing new and replays.
    env.extractor_manifest("0.2.0");
    script_bodies(
        &env,
        &[("https://x/1", "first body"), ("https://x/2", "second")],
    );
    let second = env.stderr(&["sync", "project"]);
    let (read, extract, project) = (
        stage(&second, "read"),
        stage(&second, "extract"),
        stage(&second, "project"),
    );
    assert!(read < extract && extract < project, "{second}");
    assert!(
        second
            .lines()
            .nth(extract)
            .unwrap()
            .to_lowercase()
            .contains("replay"),
        "the extract line doesn't say it replays:\n{second}"
    );
}

/// sync-progress/S15: a read over 2 s shows a running count, and no more than one per 2 s
#[test]
fn S15_a_slow_read_shows_running_counts_at_most_one_per_two_seconds() {
    let env = Env::new();
    add_windowed(&env, "project");
    let mut steps = vec![json!({"do": "initialize"})];
    for n in 0..13 {
        steps.push(json!({"do": "send", "message": submit(&format!("https://x/{n}"), "body")}));
        steps.push(json!({"do": "sleep", "ms": 500}));
    }
    steps.push(complete());
    steps.push(json!({"do": "finish", "status": "ok", "submitted": 13}));
    env.script(&steps);
    let started = Instant::now();
    let err = env.stderr(&["sync", "project"]);
    let seconds = started.elapsed().as_secs_f64();
    assert!(seconds > 2.0, "the read was not slow: {seconds}");
    let counts = err
        .lines()
        .filter(|l| l.contains("project") && l.contains("items"))
        .count();
    assert!(counts >= 1, "no running count in {seconds}s:\n{err}");
    assert!(
        counts as f64 <= (seconds / 2.0).ceil(),
        "{counts} count lines in {seconds}s:\n{err}"
    );
}

/// sync-progress/S16: the result line has counts, and a rate-limited one has its retry time
#[test]
fn S16_stderr_result_line_gives_counts_and_the_rate_limit_retry_time() {
    let env = Env::new();
    add_windowed(&env, "project");
    script_bodies(
        &env,
        &[("https://x/1", "first body"), ("https://x/2", "second")],
    );
    env.ok(&["sync", "project"]);
    script_bodies(
        &env,
        &[("https://x/1", "first body"), ("https://x/2", "changed")],
    );
    let err = env.stderr(&["sync", "project"]);
    let line = at(&err, "with the counts", |l| {
        l.contains("submitted 2") && l.contains("accepted 1") && l.contains("unchanged 1")
    });
    assert!(err.lines().nth(line).unwrap().contains("project"), "{err}");
    env.script(&[
        json!({"do": "initialize"}),
        json!({"do": "send", "message": {"jsonrpc": "2.0", "method": "coverage.report",
            "params": {"status": "couldnt_look", "reason": "rate_limited",
            "retry_at": "2030-01-01T00:00:00Z", "selection": "all"}}}),
        json!({"do": "finish", "status": "ok", "submitted": 0}),
    ]);
    let output = env.rastro(&["sync", "project"]);
    assert!(!output.status.success());
    let err = String::from_utf8(output.stderr).unwrap();
    let line = at(&err, "with the retry time", |l| l.contains("retry at"));
    let text = err.lines().nth(line).unwrap();
    assert!(
        text.contains("rate-limited") && text.contains("2030-01-01"),
        "{text}"
    );
}

/// `sync project --json` with stderr as it is, closed, or with its read end gone.
fn sync_json(env: &Env, how: &str) -> Output {
    let mut command = env.command(&["sync", "project", "--json"]);
    if how == "closed" {
        // File descriptor 2 closed: a write to it fails with EBADF.
        let mut shell = Command::new("sh");
        shell.args(["-c", "exec \"$0\" \"$@\" 2>&-"]);
        shell.arg(command.get_program());
        shell.args(command.get_args());
        for (key, value) in command.get_envs() {
            shell.env(key, value.unwrap());
        }
        command = shell;
        command.stdout(Stdio::piped());
    } else {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
    }
    let mut child = command.spawn().unwrap();
    // Broken: the read end of the pipe is gone before `rastro` writes, so writes get EPIPE.
    if how == "broken" {
        drop(child.stderr.take());
    }
    child.wait_with_output().unwrap()
}

/// sync-progress/S17: stdout holds one envelope; a broken or closed stderr changes nothing
#[test]
fn S17_a_broken_or_closed_stderr_changes_neither_the_result_nor_the_exit_code() {
    let bodies = [("https://x/1", "first body")];
    let reads = |how: &str| {
        let env = Env::new();
        add_windowed(&env, "project");
        script_bodies(&env, &bodies);
        sync_json(&env, how)
    };
    // A diagnostic the core prints itself, not the connector: its plugin fails the check.
    let refused = |how: &str| {
        let env = Env::with_manifest(|m| {
            m["permissions"]["actions"] = toml::Value::Array(vec!["act".into()]);
        });
        add_windowed(&env, "project");
        sync_json(&env, how)
    };
    for (name, run, succeeds) in [
        ("read", &reads as &dyn Fn(&str) -> Output, true),
        ("refused", &refused, false),
    ] {
        let normal = run("normal");
        assert_eq!(normal.status.success(), succeeds, "{name}: {normal:?}");
        let expected: Value = serde_json::from_slice(&normal.stdout).unwrap();
        for how in ["normal", "closed", "broken"] {
            let out = run(how);
            assert_eq!(
                out.status.code(),
                normal.status.code(),
                "{name} {how}: {out:?}"
            );
            let values: Vec<Value> = serde_json::Deserializer::from_slice(&out.stdout)
                .into_iter::<Value>()
                .collect::<Result<_, _>>()
                .unwrap_or_else(|e| panic!("{name} {how}: stdout is not JSON only ({e}): {out:?}"));
            assert_eq!(values.len(), 1, "{name} {how}: {values:?}");
            let (run, wanted) = (
                &values[0]["result"]["runs"][0],
                &expected["result"]["runs"][0],
            );
            for field in ["status", "counts", "coverage", "reason"] {
                assert_eq!(run[field], wanted[field], "{name} {how}: {field}");
            }
        }
    }
}
