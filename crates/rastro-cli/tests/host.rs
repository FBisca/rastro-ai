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
    /// Extra environment for the `rastro` process.
    vars: Vec<(&'static str, String)>,
}
impl Env {
    fn new() -> Self {
        Self::with_manifest(|_| {})
    }
    fn with_manifest(edit: impl FnOnce(&mut toml::Table)) -> Self {
        let env = Self {
            home: TempDir::new().unwrap(),
            vars: Vec::new(),
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
    /// For scenarios whose point is a timeout: a short silence limit, which a slow machine can
    /// only make come sooner. Every other scenario keeps the generous default, so a connector
    /// that is merely slow to start is never mistaken for a silent one.
    fn tight(mut self) -> Self {
        self.vars.push(("RASTRO_HOST_SILENCE_MS", "1500".into()));
        self
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(support::rastro());
        command
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.home.path().join("dev-plugins"))
            .env("RASTRO_TEST_SCRIPT", self.home.path().join("script.jsonl"))
            .env("RASTRO_HOST_SILENCE_MS", "8000")
            .env("RASTRO_HOST_MAX_LINE", "4096")
            .env("RASTRO_HOST_SHUTDOWN_MS", "3000")
            .env("RASTRO_HOST_STDERR_LINES", "50")
            .args(args);
        for (key, value) in &self.vars {
            command.env(key, value);
        }
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
    /// Runs `rastro sync project --json` and fails the test, killing it, if it takes longer than
    /// `secs`. With `read_stderr` false its stderr is a pipe nobody reads.
    fn sync_within(&self, secs: u64, read_stderr: bool) -> (Output, Value) {
        let (output, value) = self.within(&["sync", "project", "--json"], secs, read_stderr);
        let run = value["result"]["runs"][0].clone();
        (output, run)
    }
    fn within(&self, args: &[&str], secs: u64, read_stderr: bool) -> (Output, Value) {
        use std::io::Read;
        let mut command = self.command(args);
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn().unwrap();
        let pid = child.id();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let mut err = Vec::new();
            let reader = std::thread::spawn(move || {
                stdout.read_to_end(&mut out).unwrap();
                out
            });
            if read_stderr {
                stderr.read_to_end(&mut err).unwrap();
            }
            let status = child.wait().unwrap();
            let _ = sender.send(Output {
                status,
                stdout: reader.join().unwrap(),
                stderr: err,
            });
            drop(stderr);
        });
        let output = match receiver.recv_timeout(std::time::Duration::from_secs(secs)) {
            Ok(output) => output,
            Err(_) => {
                let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
                panic!("sync did not finish within {secs}s");
            }
        };
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        (output, value)
    }
    fn rastro_within(&self, args: &[&str], secs: u64) -> Output {
        let child = self
            .command(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(child.wait_with_output());
        });
        match receiver.recv_timeout(std::time::Duration::from_secs(secs)) {
            Ok(output) => output.unwrap(),
            Err(_) => {
                let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
                panic!("rastro did not finish within {secs}s");
            }
        }
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

/// evidence/R13: unsupported major or different manifest id fails with protocol
#[test]
fn unsupported_major_or_different_manifest_id_fails_with_protocol() {
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

/// evidence/R14: manifest without permissions env still loads
#[test]
fn manifest_without_permissions_env_still_loads() {
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

/// evidence/R15: connector exit before finish is interrupted and keeps evidence
#[test]
fn connector_exit_before_finish_is_interrupted_and_keeps_evidence() {
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

/// evidence/R16: malformed oversized or silent connector is killed
#[test]
fn malformed_oversized_or_silent_connector_is_killed() {
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
        let env = if reason == "timeout" {
            Env::new().tight()
        } else {
            Env::new()
        };
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

/// evidence/R17: stderr is relayed escaped and never blocks the connector
#[test]
fn stderr_is_relayed_escaped_and_never_blocks_the_connector() {
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

/// evidence/R30: unfinished run is marked interrupted when the core starts
#[test]
fn unfinished_run_is_marked_interrupted_when_the_core_starts() {
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

/// evidence/R32: typed couldnt look reason is stored and never empty
#[test]
fn typed_couldnt_look_reason_is_stored_and_never_empty() {
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

fn failed_with(run: &Value, reason: &str) {
    assert_eq!(
        (&run["status"], &run["reason"]),
        (&json!("failed"), &json!(reason))
    );
}

/// evidence/R18: fast stdout is held in bounded memory and the run completes
#[test]
fn fast_stdout_is_held_in_bounded_memory_and_the_run_completes() {
    let env = Env::new();
    env.script(&[
        init(),
        json!({"do": "flood", "lines": 20000}),
        complete(),
        finish(),
    ]);
    let (output, run) = env.sync_within(20, true);
    // Every line was rejected, so the window was not read as it promises.
    assert!(!output.status.success());
    failed_with(&run, "protocol");
    assert_eq!(run["counts"]["rejected"], 20000);
}

/// evidence/R19: connector that stops reading stdin times out
#[test]
fn connector_that_stops_reading_stdin_times_out() {
    let env = Env::new().tight();
    // The binding in `initialize` is bigger than any pipe buffer.
    env.ok(&[
        "source",
        "add",
        "big",
        "--connector",
        "test",
        "--repo",
        &"r".repeat(120_000),
        "--scope",
        "personal",
    ]);
    env.script(&[json!({"do": "sleep", "ms": 20000})]);
    let output = env.rastro_within(&["sync", "big", "--json"], 15);
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    failed_with(&value["result"]["runs"][0], "timeout");
    assert!(!output.status.success());
}

/// evidence/R20: connector that lingers after finish is killed and the run kept
#[test]
fn connector_that_lingers_after_finish_is_killed_and_the_run_kept() {
    for linger in [
        json!({"do": "sleep", "ms": 20000}),
        json!({"do": "spawn_holder", "secs": 120}),
    ] {
        let env = Env::new();
        let mut script = vec![init(), complete(), finish(), linger];
        if script[3]["do"] == "spawn_holder" {
            script.push(json!({"do": "sleep", "ms": 20000}));
        }
        env.script(&script);
        let (output, run) = env.sync_within(10, true);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(run["status"], "ok");
        assert_eq!(
            env.sql::<i64>("SELECT count(*) FROM runs WHERE finished_at IS NULL"),
            0
        );
    }
}

/// evidence/R21: stderr relay drops the excess and never blocks
#[test]
fn stderr_relay_drops_the_excess_and_never_blocks() {
    let env = Env::new();
    env.script(&[
        init(),
        json!({"do": "stderr", "text": "noisy", "repeat": 5000}),
        complete(),
        finish(),
    ]);
    let output = env.rastro(&["sync", "project"]);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let relayed = stderr.lines().filter(|l| l.contains("noisy")).count();
    assert!((50..=60).contains(&relayed), "{relayed} lines relayed");
    assert!(
        stderr.contains("dropped 49"),
        "no dropped notice: {}",
        &stderr[stderr.len().saturating_sub(300)..]
    );
    // The core's own stderr is a pipe nobody reads: neither side may wait for it.
    env.script(&[
        init(),
        json!({"do": "stderr", "text": "x".repeat(900), "repeat": 20000}),
        complete(),
        finish(),
    ]);
    let (_, run) = env.sync_within(15, false);
    assert_eq!(run["status"], "ok");
}

/// evidence/R22: invalid utf8 or a bad envelope fails with protocol
#[test]
fn invalid_utf8_or_a_bad_envelope_fails_with_protocol() {
    let mut bad_version = evidence();
    bad_version["jsonrpc"] = json!("1.0");
    let mut no_method = evidence();
    no_method.as_object_mut().unwrap().remove("method");
    // The bad byte sits inside a quoted value, where lossy decoding would still parse.
    let mut invalid = serde_json::to_vec(&evidence()).unwrap();
    let at = invalid.windows(5).position(|w| w == b"hello").unwrap();
    invalid.splice(at + 2..at + 3, [0xff]);
    let finish_without_id = send(json!({"jsonrpc": "2.0", "method": "run.finish",
        "params": {"status": "ok", "submitted": 0}}));
    for line in [
        json!({"do": "send_bytes", "bytes": invalid}),
        send(bad_version),
        send(no_method),
        finish_without_id,
    ] {
        let env = Env::new();
        env.script(&[init(), line, complete(), finish()]);
        let (output, run) = env.sync_within(10, true);
        assert!(!output.status.success());
        failed_with(&run, "protocol");
        assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 0);
        assert_eq!(env.sql::<String>("SELECT reason FROM coverage"), "protocol");
    }
}

/// evidence/R31: recovered run records couldnt look for its window
#[test]
fn recovered_run_records_couldnt_look_for_its_window() {
    let env = Env::new();
    env.script(&[
        init(),
        json!({"do": "sleep", "ms": 3000}),
        complete(),
        finish(),
    ]);
    let mut crashing = env
        .command(&["sync", "project"])
        .env("RASTRO_HOST_SILENCE_MS", "10000")
        .spawn()
        .unwrap();
    for _ in 0..100 {
        if env.sql::<i64>("SELECT count(*) FROM runs") == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    crashing.kill().unwrap();
    crashing.wait().unwrap();
    env.ok(&["coverage"]);
    assert_eq!(
        env.sql::<String>("SELECT status || ':' || reason FROM runs"),
        "failed:interrupted"
    );
    assert_eq!(
        env.sql::<String>(
            "SELECT state || ':' || reason || ':' || source || ':' || scope FROM coverage"
        ),
        "couldn't look:interrupted:project:personal"
    );
    assert_eq!(
        env.sql::<String>("SELECT coverage.window_start || coverage.window_end FROM coverage"),
        env.sql::<String>("SELECT window_start || window_end FROM runs")
    );
    let covered: Value = serde_json::from_str(&env.ok(&["coverage", "project", "--json"])).unwrap();
    assert_eq!(covered["result"]["coverage"][0]["state"], "unknown");
    assert_eq!(covered["result"]["coverage"][0]["reason"], "interrupted");
}

/// evidence/R33: finish without coverage fails with protocol
#[test]
fn finish_without_coverage_fails_with_protocol() {
    let env = Env::new();
    env.script(&[init(), send(evidence()), finish()]);
    let (output, run) = env.sync_within(10, true);
    assert!(!output.status.success());
    failed_with(&run, "protocol");
    assert_eq!(run["coverage"], "couldn't look");
    assert_eq!(env.sql::<String>("SELECT reason FROM coverage"), "protocol");
}

/// evidence/R33: finish with only rejected coverage reports fails with protocol
#[test]
fn finish_with_only_rejected_coverage_reports_fails_with_protocol() {
    let env = Env::new();
    let forged = json!({"from": "1900-01-01T00:00:00Z", "to": "1900-02-01T00:00:00Z"});
    let report = send(json!({"jsonrpc": "2.0", "method": "coverage.report",
        "params": {"status": "complete", "window": forged}}));
    env.script(&[init(), report.clone(), report, finish()]);
    let (output, run) = env.sync_within(10, true);
    assert!(!output.status.success());
    failed_with(&run, "protocol");
    assert_eq!(
        env.sql::<String>("SELECT state || ':' || reason FROM coverage"),
        "couldn't look:protocol"
    );
}

/// A process that is running or sleeping; a zombie (exited, unreaped) is not alive.
fn alive(pid: &str) -> bool {
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", pid])
        .output()
        .unwrap();
    let state = String::from_utf8_lossy(&output.stdout);
    let state = state.trim();
    !state.is_empty() && !state.starts_with('Z')
}

/// evidence/R23: many core diagnostics never make the run wait on stderr
#[test]
fn many_core_diagnostics_never_make_the_run_wait_on_stderr() {
    let env = Env::new();
    env.script(&[
        init(),
        json!({"do": "flood", "lines": 20000}),
        complete(),
        finish(),
    ]);
    let (_, run) = env.sync_within(20, false);
    assert_eq!(run["counts"]["rejected"], 20000);
    let forged = json!({"jsonrpc": "2.0", "method": "coverage.report",
        "params": {"status": "complete", "scope": "work"}});
    let reports: Vec<Value> = (0..20000).map(|_| send(forged.clone())).collect();
    let mut steps = vec![init()];
    steps.extend(reports);
    steps.extend([complete(), finish()]);
    env.script(&steps);
    let (_, run) = env.sync_within(20, false);
    assert_eq!(run["status"], "ok");
}

/// evidence/R24: unwritable run finish reply fails with timeout
#[test]
fn unwritable_run_finish_reply_fails_with_timeout() {
    let mut env = Env::new().tight();
    env.vars.push(("RASTRO_HOST_MAX_LINE", "1000000".into()));
    env.script(&[
        init(),
        complete(),
        json!({"do": "finish_big_id", "bytes": 200_000}),
        json!({"do": "sleep", "ms": 20000}),
    ]);
    let (output, run) = env.sync_within(15, true);
    assert!(!output.status.success());
    failed_with(&run, "timeout");
}

/// evidence/R25: ids and shapes json rpc does not allow fail with protocol
#[test]
fn ids_and_shapes_json_rpc_does_not_allow_fail_with_protocol() {
    let mut shapes = Vec::new();
    for id in [json!({}), json!([]), json!(true)] {
        let mut message = evidence();
        message["id"] = id;
        shapes.push(message);
    }
    for key in ["result", "error"] {
        let mut message = evidence();
        message[key] = json!({});
        shapes.push(message);
    }
    for message in shapes {
        let env = Env::new();
        env.script(&[init(), send(message), complete(), finish()]);
        let (output, run) = env.sync_within(10, true);
        assert!(!output.status.success());
        failed_with(&run, "protocol");
        assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 0);
    }
}

/// evidence/R26: the connectors process group is killed when the run ends
#[test]
fn the_connectors_process_group_is_killed_when_the_run_ends() {
    for linger in [false, true] {
        let env = Env::new();
        let pid_file = env.home.path().join("holder.pid");
        let mut script = vec![
            init(),
            json!({"do": "spawn_holder", "secs": 120, "pid_file": pid_file}),
            complete(),
            finish(),
        ];
        if linger {
            script.push(json!({"do": "sleep", "ms": 20000}));
        }
        env.script(&script);
        let (output, run) = env.sync_within(10, true);
        assert!(output.status.success());
        assert_eq!(run["status"], "ok");
        let pid = fs::read_to_string(&pid_file).unwrap();
        let mut gone = false;
        for _ in 0..40 {
            if !alive(&pid) {
                gone = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if !gone {
            let _ = Command::new("kill").args(["-9", &pid]).status();
        }
        assert!(gone, "the connector's child outlived the run");
    }
}

/// evidence/R28: span offsets the store cannot hold are rejected without waiting on stderr
#[test]
fn span_offsets_the_store_cannot_hold_are_rejected_without_waiting_on_stderr() {
    let mut env = Env::new();
    env.vars.push(("RASTRO_HOST_STDERR_LINES", "100000".into()));
    let huge = json!({"jsonrpc": "2.0", "method": "evidence.submit", "params": {
        "kind": "issue", "location": "https://x/1", "field": "/body",
        "start": 9223372036854775808u64, "end": 9223372036854775813u64, "quote": "hello",
        "trust_class": "untrusted content"}});
    env.script(&[
        init(),
        json!({"do": "stderr", "text": "x".repeat(900), "repeat": 20000}),
        send(huge),
        complete(),
        finish(),
    ]);
    let (output, run) = env.sync_within(15, false);
    assert!(!output.status.success());
    failed_with(&run, "protocol");
    assert_eq!(
        run["counts"],
        json!({"submitted": 1, "accepted": 0, "rejected": 1, "unchanged": 0})
    );
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 0);
}

/// evidence/R29: many syncs with blocked stderr finish without piling up printers
#[test]
fn many_syncs_with_blocked_stderr_finish_without_piling_up_printers() {
    let mut env = Env::new();
    env.vars.push(("RASTRO_HOST_STDERR_LINES", "100000".into()));
    for name in ["two", "three", "four", "five"] {
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
        ]);
    }
    env.script(&[
        init(),
        json!({"do": "stderr", "text": "x".repeat(900), "repeat": 5000}),
        complete(),
        finish(),
    ]);
    let started = std::time::Instant::now();
    let (_, value) = env.within(&["sync", "--json"], 20, false);
    assert_eq!(value["result"]["runs"].as_array().unwrap().len(), 5);
    assert!(started.elapsed().as_secs() < 10);
}

/// evidence/R28: a storage failure under blocked stderr fails the run with storage without hanging
#[test]
fn a_storage_failure_under_blocked_stderr_fails_the_run_with_storage_without_hanging() {
    let mut env = Env::new();
    env.vars.push(("RASTRO_HOST_STDERR_LINES", "100000".into()));
    // The search index is gone, so appending an accepted observation fails inside the store.
    env.db().execute_batch("DROP TABLE evidence_fts").unwrap();
    env.script(&[
        init(),
        json!({"do": "stderr", "text": "x".repeat(900), "repeat": 20000}),
        send(evidence()),
        complete(),
        finish(),
    ]);
    let (output, run) = env.sync_within(15, false);
    assert!(!output.status.success());
    assert_eq!(run["status"], "failed");
    assert_eq!(run["reason"], "storage");
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 0);
}

/// evidence/R75: a stored option the manifest doesn't declare is refused at sync, before the connector runs
#[test]
fn a_hand_edited_option_is_refused_at_sync_before_the_connector_runs() {
    let env = Env::new();
    let config = env.home.path().join("config/config.toml");
    let text = fs::read_to_string(&config).unwrap();
    assert!(text.contains("repo = \"o/p\""));
    fs::write(&config, text.replace("repo = \"o/p\"", "bogus = \"1\"")).unwrap();
    env.script(&[evidence(), complete(), finish()]);
    let (output, run) = env.sync();
    assert!(!output.status.success());
    assert_eq!(
        (&run["status"], &run["reason"]),
        (&json!("failed"), &json!("protocol"))
    );
    assert_eq!(env.sql::<i64>("SELECT count(*) FROM evidence"), 0);
}

/// evidence/R76: a hand-edited GitHub repo that fails the declared pattern never reaches the connector
#[test]
fn a_hand_edited_github_repo_is_refused_at_sync() {
    let env = Env::with_manifest(|m| {
        m.insert("id".into(), "github".into());
        m.remove("source_options");
    });
    let config = env.home.path().join("config/config.toml");
    let text = fs::read_to_string(&config).unwrap();
    let edited = text
        .replace("repo = \"o/p\"", "repo = \"../x\"")
        .replace("connector = \"test\"", "connector = \"github\"");
    assert_ne!(text, edited);
    fs::write(&config, edited).unwrap();
    env.script(&[evidence(), complete(), finish()]);
    let (output, run) = env.sync();
    assert!(!output.status.success());
    assert_eq!(
        (&run["status"], &run["reason"]),
        (&json!("failed"), &json!("protocol"))
    );
}
