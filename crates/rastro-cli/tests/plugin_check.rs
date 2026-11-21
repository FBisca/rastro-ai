//! `rastro plugin check <id>`: one synthetic connector run through the host's validation,
//! storing nothing. The connector is the scripted test connector, installed as a plugin under
//! a fresh home; no source is registered, so nothing a sync would store exists to begin with.
#![allow(non_snake_case)]
mod support;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    process::{Command, Output, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use tempfile::TempDir;

struct Env {
    home: TempDir,
}
impl Env {
    fn new() -> Self {
        Self::with_manifest(|_| {})
    }
    /// Installs the scripted test connector as plugin `test` under a fresh home. Unlike a sync
    /// fixture it registers no source: the check binds its own synthetic run.
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
        command
    }
    fn script(&self, steps: &[Value]) {
        let lines: Vec<_> = steps.iter().map(Value::to_string).collect();
        fs::write(self.home.path().join("script.jsonl"), lines.join("\n")).unwrap();
    }
    fn check(&self, args: &[&str]) -> Output {
        let mut full = vec!["plugin", "check", "test"];
        full.extend_from_slice(args);
        self.command(&full).output().unwrap()
    }
}

/// Everything the check printed, on either stream: its result lines and the connector's relay.
fn both(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
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
/// The scripted run a conforming connector gives: one valid observation, complete coverage,
/// a finish.
fn conforming() -> Vec<Value> {
    vec![init(), send(evidence()), complete(), finish()]
}

/// spec: Plugin check runs one synthetic connector run and stores nothing
/// A conforming connector exits zero with the accepted counts, as text and as JSON.
#[test]
fn a_conforming_connector_exits_zero_with_the_accepted_counts() {
    let env = Env::new();
    env.script(&conforming());
    let output = env.check(&[]);
    let text = both(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("accepted 1"), "{text}");
    let output = env.check(&["--json"]);
    let text = both(&output);
    assert!(output.status.success(), "{text}");
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["result"]["counts"]["accepted"], json!(1), "{text}");
}

/// spec: Plugin check runs one synthetic connector run and stores nothing
/// A check against a fresh home stores no evidence, coverage or run: it never opens a store.
#[test]
fn a_check_against_a_fresh_home_stores_nothing() {
    let env = Env::new();
    env.script(&conforming());
    let output = env.check(&[]);
    assert!(output.status.success(), "{}", both(&output));
    assert!(
        !env.home.path().join("data/rastro.sqlite3").exists(),
        "the check created a database"
    );
    let data = env.home.path().join("data");
    assert!(
        !data.exists() || fs::read_dir(&data).unwrap().next().is_none(),
        "the check wrote under data/"
    );
}

/// spec: Plugin check names each protocol violation
/// A plain log line on stdout is named as an invalid protocol line and fails the check.
#[test]
fn a_log_line_on_stdout_is_named_and_fails_the_check() {
    let env = Env::new();
    env.script(&[
        init(),
        json!({"do": "send_raw", "line": "just a log line"}),
        complete(),
        finish(),
    ]);
    let output = env.check(&[]);
    let text = both(&output);
    assert!(!output.status.success(), "{text}");
    assert!(text.contains("invalid protocol line"), "{text}");
    assert!(text.contains("just a log line"), "{text}");
}

/// spec: Plugin check names each protocol violation
/// Evidence of a kind the manifest does not permit is named and fails the check.
#[test]
fn evidence_of_an_unpermitted_kind_is_named_and_fails_the_check() {
    let env = Env::new();
    let mut outside = evidence();
    outside["params"]["kind"] = json!("mail");
    env.script(&[init(), send(outside), complete(), finish()]);
    let output = env.check(&[]);
    let text = both(&output);
    assert!(!output.status.success(), "{text}");
    assert!(text.contains("kind not permitted"), "{text}");
    assert!(text.contains("mail"), "{text}");
}

/// spec: Plugin check names each protocol violation
/// A connector that exits without a finish is named as interrupted and fails the check.
#[test]
fn exiting_without_a_finish_is_named_as_interrupted() {
    let env = Env::new();
    env.script(&[init(), send(evidence()), json!({"do": "exit", "code": 0})]);
    let output = env.check(&[]);
    let text = both(&output);
    assert!(!output.status.success(), "{text}");
    assert!(text.contains("interrupted"), "{text}");
}

/// spec: Plugin check warns on an unknown manifest kind
/// An unknown manifest kind warns naming it, and a run with no violation still exits zero.
#[test]
fn an_unknown_manifest_kind_warns_and_a_clean_run_exits_zero() {
    let env = Env::with_manifest(|m| {
        m["kinds"] = toml::Value::Array(vec!["connector".into(), "mail".into()]);
    });
    env.script(&conforming());
    let output = env.check(&[]);
    let text = both(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("Warning"), "{text}");
    assert!(text.contains("mail"), "{text}");
}

/// spec: Plugin check stays on the CLI
/// The check is a CLI command, and an agent listing MCP tools sees no plugin check tool.
#[test]
fn the_check_is_a_cli_command_and_no_mcp_tool_lists_it() {
    let env = Env::new();
    let help = env
        .command(&["plugin", "check", "--help"])
        .output()
        .unwrap();
    assert!(
        help.status.success(),
        "{}",
        String::from_utf8_lossy(&help.stderr)
    );
    let mut session = McpSession::start(&env);
    session.initialize();
    let tools = session.list_tools();
    assert!(!tools.is_empty(), "the MCP server listed no tools");
    for tool in &tools {
        let name = tool["name"].as_str().unwrap();
        assert!(!name.contains("plugin"), "MCP lists a plugin tool: {name}");
    }
    session.finish();
}

/// A minimal MCP client over stdio: initialize, then the requests the test needs.
struct McpSession {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    lines: mpsc::Receiver<String>,
    next_id: u64,
}
impl McpSession {
    fn start(env: &Env) -> Self {
        let mut child = env
            .command(&["mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = sender.send(line);
            }
        });
        Self {
            child,
            stdin,
            lines,
            next_id: 1,
        }
    }
    fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(20))
                .expect("the MCP server printed nothing within its deadline");
            let message: Value = serde_json::from_str(&line).unwrap();
            if message["id"] == json!(id) && message.get("method").is_none() {
                return message;
            }
        }
    }
    fn initialize(&mut self) {
        self.request(
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {},
                   "clientInfo": {"name": "plugin-check-test", "version": "1.0"}}),
        );
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    }
    fn list_tools(&mut self) -> Vec<Value> {
        self.request("tools/list", json!({}))["result"]["tools"]
            .as_array()
            .unwrap()
            .clone()
    }
    /// Closes stdin, which is how the server is told to exit, and waits for it.
    fn finish(&mut self) {
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.child.try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "the MCP server did not exit after stdin closed"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for McpSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
