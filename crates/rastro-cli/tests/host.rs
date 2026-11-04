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

fn connector_binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let status = Command::new(cargo)
            .args(["build", "--quiet", "-p", "rastro-test-connector"])
            .status()
            .unwrap();
        assert!(status.success(), "building the test connector failed");
        Path::new(env!("CARGO_BIN_EXE_rastro")).with_file_name("rastro-test-connector")
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
        ]);
        env
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rastro"));
        command
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.home.path().join("dev-plugins"))
            .env("RASTRO_TEST_SCRIPT", self.home.path().join("script.jsonl"))
            .env("RASTRO_HOST_SILENCE_MS", "600")
            .env("RASTRO_HOST_MAX_LINE", "4096")
            .args(args);
        command
    }
    fn rastro(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
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
    fn script(&self, steps: &[Value]) {
        let lines: Vec<_> = steps.iter().map(Value::to_string).collect();
        fs::write(self.home.path().join("script.jsonl"), lines.join("\n")).unwrap();
    }
    fn db(&self) -> Connection {
        Connection::open(self.home.path().join("data/rastro.sqlite3")).unwrap()
    }
    fn sql<T: rusqlite::types::FromSql>(&self, query: &str) -> T {
        self.db().query_row(query, [], |row| row.get(0)).unwrap()
    }
    /// Syncs and returns the run as JSON; the exit status is checked by the caller.
    fn sync(&self) -> (Output, Value) {
        let output = self.rastro(&["sync", "project", "--json"]);
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let run = value["result"]["runs"][0].clone();
        (output, run)
    }
}

fn evidence() -> Value {
    json!({"jsonrpc": "2.0", "method": "evidence.submit", "params": {
        "kind": "issue", "location": "https://x/1", "field": "/body", "start": 0, "end": 5,
        "quote": "hello", "trust_class": "untrusted content"}})
}
fn send(message: Value) -> Value {
    json!({"do": "send", "message": message})
}
fn complete() -> Value {
    send(json!({"jsonrpc": "2.0", "method": "coverage.report", "params": {"status": "complete"}}))
}
fn init() -> Value {
    json!({"do": "initialize"})
}
fn finish() -> Value {
    json!({"do": "finish", "submitted": 1})
}

#[test]
fn S7_unsupported_major_or_different_manifest_id_fails_with_protocol() {
    for handshake in [
        json!({"do": "initialize", "protocol": "2.0"}),
        json!({"do": "initialize", "manifest_id": "someone-else"}),
    ] {
        let env = Env::new();
        env.script(&[handshake, send(evidence()), complete(), finish()]);
        let (output, run) = env.sync();
        assert!(!output.status.success());
        assert_eq!(
            (&run["status"], &run["reason"]),
            (&json!("failed"), &json!("protocol"))
        );
        assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 0);
        assert_eq!(
            env.sql::<String>("SELECT state FROM coverage"),
            "couldn't look"
        );
    }
}

#[test]
fn S7a_manifest_without_permissions_env_still_loads() {
    let env = Env::with_manifest(|m| {
        m["permissions"].as_table_mut().unwrap().remove("env");
    });
    let (output, run) = env.sync();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(run["status"], "ok");
}

#[test]
fn S8_connector_exit_before_finish_is_interrupted_and_keeps_evidence() {
    let env = Env::new();
    env.script(&[init(), send(evidence()), json!({"do": "exit", "code": 3})]);
    let (output, run) = env.sync();
    assert!(!output.status.success());
    assert_eq!(
        (&run["status"], &run["reason"]),
        (&json!("failed"), &json!("interrupted"))
    );
    assert_eq!(run["coverage"], "couldn't look");
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 1);
    assert_eq!(
        env.sql::<String>("SELECT reason FROM coverage"),
        "interrupted"
    );
}

#[test]
fn S9_malformed_oversized_or_silent_connector_is_killed() {
    let cases = [
        (
            vec![
                send(evidence()),
                json!({"do": "send_raw", "line": "{\"jsonrpc\": \"2.0\", \"meth"}),
            ],
            "protocol",
        ),
        (
            vec![json!({"do": "send_oversized", "bytes": 10000}), complete()],
            "protocol",
        ),
        (vec![json!({"do": "sleep", "ms": 20000})], "timeout"),
    ];
    for (steps, reason) in cases {
        let env = Env::new();
        let mut script = vec![init()];
        script.extend(steps);
        script.extend([complete(), finish()]);
        env.script(&script);
        let started = std::time::Instant::now();
        let (output, run) = env.sync();
        assert!(
            started.elapsed().as_secs() < 10,
            "the connector wasn't killed"
        );
        assert!(!output.status.success());
        assert_eq!(
            (&run["status"], &run["reason"]),
            (&json!("failed"), &json!(reason))
        );
        assert_eq!(run["coverage"], "couldn't look");
        let stored = env.sql::<i64>("SELECT count(*) FROM evidence");
        assert_eq!(
            stored,
            i64::from(reason == "protocol" && run["counts"]["accepted"] == 1)
        );
    }
}

#[test]
fn S9a_stderr_is_relayed_escaped_and_never_blocks_the_connector() {
    let env = Env::new();
    env.script(&[
        init(),
        json!({"do": "stderr", "text": "\u{1b}[31mred alert"}),
        json!({"do": "stderr", "text": "x".repeat(2000), "repeat": 2000}),
        complete(),
        finish(),
    ]);
    let output = env.rastro(&["sync", "project"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.stderr.contains(&0x1b));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("\\u{1b}[31mred alert"),
        "{}",
        &stderr[..200.min(stderr.len())]
    );
    assert!(stderr.lines().all(|line| line.chars().count() < 1000));
}

#[test]
fn S10_unfinished_run_is_marked_interrupted_when_the_core_starts() {
    let env = Env::new();
    env.db()
        .execute(
            "INSERT INTO runs (source, scope, plugin_id, plugin_version, started_at, status)
             VALUES ('project', 'personal', 'test', '0.1.0', '2026-11-04T00:00:00.000Z', 'running')",
            [],
        )
        .unwrap();
    env.ok(&["coverage"]);
    assert_eq!(
        env.sql::<String>("SELECT status || ':' || reason FROM runs WHERE id = 1"),
        "failed:interrupted"
    );
    assert!(
        env.sql::<Option<String>>("SELECT finished_at FROM runs WHERE id = 1")
            .is_some()
    );
    // A sync still running in another process is left alone.
    env.script(&[
        init(),
        json!({"do": "sleep", "ms": 1500}),
        complete(),
        finish(),
    ]);
    let mut running = env
        .command(&["sync", "project"])
        .env("RASTRO_HOST_SILENCE_MS", "5000")
        .spawn()
        .unwrap();
    for _ in 0..50 {
        if env.sql::<i64>("SELECT count(*) FROM runs WHERE finished_at IS NULL") == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    env.ok(&["coverage"]);
    assert_eq!(
        env.sql::<i64>("SELECT count(*) FROM runs WHERE finished_at IS NULL"),
        1
    );
    assert!(running.wait().unwrap().success());
}

#[test]
fn S11_typed_couldnt_look_reason_is_stored_and_never_empty() {
    let retry = "2026-11-05T10:00:00Z";
    let report = |reason: &str| {
        send(
            json!({"jsonrpc": "2.0", "method": "coverage.report", "params": {
            "status": "couldnt_look", "reason": reason, "retry_at": retry}}),
        )
    };
    let env = Env::new();
    env.script(&[init(), send(evidence()), report("rate_limited"), finish()]);
    let (output, run) = env.sync();
    assert!(!output.status.success());
    assert_eq!(run["coverage"], "couldn't look");
    assert_eq!(
        env.sql::<String>("SELECT reason FROM coverage"),
        "rate_limited"
    );
    assert!(
        env.sql::<String>("SELECT retry_at FROM coverage")
            .starts_with("2026-11-05T10:00:00")
    );
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 1);
    // An unknown reason is mapped to `protocol`: the connector broke the contract.
    let env = Env::new();
    env.script(&[init(), report("the-moon-is-in-the-wrong-phase"), finish()]);
    let (_, run) = env.sync();
    assert_eq!(run["coverage"], "couldn't look");
    assert_eq!(env.sql::<String>("SELECT reason FROM coverage"), "protocol");
}
