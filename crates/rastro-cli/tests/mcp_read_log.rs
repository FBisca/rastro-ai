//! The MCP read log: a machine read appends one audit row naming its tool, arguments and time
//! and never a shown report; the terminal read shows its items and appends no read log row.
//! Driven through the real boundaries — the `rastro mcp` server over stdio and the human
//! `rastro queue` read — on a store seeded with one action proposal.
#![allow(non_snake_case)]
mod support;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::Duration,
};
use tempfile::TempDir;

const PINNED: &str = "2025-11-25";
/// Generous: a deadline here only fails a server that is stuck, never one that is slow.
const DEADLINE: Duration = Duration::from_secs(30);

struct Env {
    home: TempDir,
}
impl Env {
    /// A home with the fake action provider and the Linear source `linear-work` in scope `work`,
    /// so an action proposal can be filed and waits in the queue.
    fn new() -> Self {
        let env = Self {
            home: TempDir::new().unwrap(),
        };
        support::provider::install(&env.home.path().join("config"), |_| {});
        fs::write(
            env.home.path().join("config/config.toml"),
            "[[sources]]\nname = \"linear-work\"\nconnector = \"linear\"\nselector = {}\n\
             scope = \"work\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
             keep_original = false\n",
        )
        .unwrap();
        env
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(support::rastro());
        command.env("RASTRO_HOME", self.home.path()).args(args);
        command
    }
    /// The `result` of a command that must succeed, read with `--json` at the injected clock.
    fn ok_json(&self, args: &[&str], at: &str) -> Value {
        let out = self
            .command(&[args, &["--json"]].concat())
            .env("RASTRO_NOW", at)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"].clone()
    }
    /// Files one action proposal, which waits in the queue; returns its result.
    fn propose(&self, title: &str, at: &str) -> Value {
        self.ok_json(
            &[
                "actions",
                "propose",
                "linear.create_issue",
                "--title",
                title,
                "--team",
                "H",
                "--why",
                "owner asked",
            ],
            at,
        )
    }
    /// The human queue read the owner prints: what it prints is what it showed.
    fn print_queue(&self, at: &str) -> String {
        let out = self
            .command(&["queue"])
            .env("RASTRO_NOW", at)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }
    fn database(&self) -> PathBuf {
        self.home.path().join("data/rastro.sqlite3")
    }
    fn db(&self) -> Connection {
        Connection::open(self.database()).unwrap()
    }
    fn start(&self, at: &str) -> Session {
        let mut command = self.command(&["mcp"]);
        command.env("RASTRO_NOW", at);
        Session::start(command)
    }
}

/// A running `rastro mcp` and the lines it has printed.
struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    stderr: Receiver<Vec<u8>>,
    next_id: u64,
}
impl Session {
    fn start(mut command: Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
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
        let mut stderr_pipe = child.stderr.take().unwrap();
        let (sender, stderr) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stderr_pipe.read_to_end(&mut bytes);
            let _ = sender.send(bytes);
        });
        Self {
            child,
            stdin,
            lines,
            stderr,
            next_id: 1,
        }
    }
    fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }
    fn receive(&mut self) -> Value {
        let line = self
            .lines
            .recv_timeout(DEADLINE)
            .expect("the server printed nothing within its deadline");
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}"))
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let message = self.receive();
            if message["id"] == json!(id) && message.get("method").is_none() {
                return message;
            }
        }
    }
    fn initialize(&mut self) {
        self.request(
            "initialize",
            json!({"protocolVersion": PINNED, "capabilities": {},
                   "clientInfo": {"name": "scripted-client", "version": "1.0"}}),
        );
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    }
    fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
    }
    /// Closes stdin and waits for the exit; returns the status and everything on stderr.
    fn finish(&mut self) -> (std::process::ExitStatus, Vec<u8>) {
        drop(self.stdin.take());
        let deadline = std::time::Instant::now() + DEADLINE;
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the server did not exit after stdin closed"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        let stderr = self.stderr.recv_timeout(DEADLINE).unwrap();
        (status, stderr)
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The `result` member of a successful tool call: a v2 success envelope with a result.
fn ok_result(result: &Value) -> &Value {
    assert!(
        result.is_object(),
        "no result (a protocol error?): {result}"
    );
    assert_ne!(result["isError"], true, "{result}");
    let envelope = &result["structuredContent"];
    assert_eq!(envelope["schema_version"], 2, "{result}");
    assert!(envelope.get("result").is_some(), "{result}");
    assert!(envelope.get("error").is_none(), "{result}");
    envelope
}

/// The instant an RFC 3339 stamp names, compared with the clock the test injected.
fn stamp_of(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|e| panic!("not a time ({e}): {text}"))
        .with_timezone(&Utc)
}

/// spec: MCP reads are logged, never seen
/// the terminal queue read writes one shown report per printed item and no read log row
#[test]
fn a_terminal_queue_read_writes_a_shown_report_and_no_read_log_row() {
    let env = Env::new();
    let proposal = env.propose("Send the budget", "2026-11-19T10:00:00Z");
    let id = proposal["id"].as_str().unwrap();
    let at = "2026-11-19T10:04:00Z";
    let printed = env.print_queue(at);
    assert!(
        printed.contains(id),
        "the terminal read printed no item: {printed}"
    );

    let db = env.db();
    let shown: Vec<(String, String)> = db
        .prepare("SELECT item, shown_at FROM shown_report ORDER BY append_seq")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        shown.len(),
        1,
        "one shown report per printed item: {shown:?}"
    );
    assert_eq!(shown[0].0, id);
    assert_eq!(stamp_of(&shown[0].1), at.parse::<DateTime<Utc>>().unwrap());
    let logged: i64 = db
        .query_row("SELECT count(*) FROM read_log", [], |row| row.get(0))
        .unwrap();
    assert_eq!(logged, 0, "a terminal read is not an MCP read");
}

/// spec: MCP reads are logged, never seen
/// an MCP queue read appends one read log row naming the tool, arguments and time, and writes no
/// shown report
#[test]
fn an_mcp_queue_read_is_logged_and_writes_no_shown_report() {
    let env = Env::new();
    env.propose("Send the budget", "2026-11-19T10:00:00Z");
    // A machine read on the CLI shows nothing and logs nothing: only the MCP server logs.
    let machine = env.ok_json(&["queue"], "2026-11-19T10:02:00Z");
    assert_eq!(machine["items"].as_array().unwrap().len(), 1);

    let at = "2026-11-19T10:04:00Z";
    let mut session = env.start(at);
    session.initialize();
    let call = session.call("queue", json!({}));
    let envelope = ok_result(&call["result"]);
    assert_eq!(envelope["operation"], "queue", "{call}");
    assert_eq!(
        envelope["result"]["items"].as_array().unwrap().len(),
        1,
        "{call}"
    );
    let (status, stderr) = session.finish();
    assert!(status.success(), "{}", String::from_utf8_lossy(&stderr));

    let db = env.db();
    let rows: Vec<(String, String, String)> = db
        .prepare("SELECT tool, arguments_json, at FROM read_log ORDER BY append_seq")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows.len(),
        1,
        "one read log row per MCP read call: {rows:?}"
    );
    assert_eq!(rows[0].0, "queue");
    assert_eq!(rows[0].1, "{}");
    assert_eq!(stamp_of(&rows[0].2), at.parse::<DateTime<Utc>>().unwrap());
    let shown: i64 = db
        .query_row("SELECT count(*) FROM shown_report", [], |row| row.get(0))
        .unwrap();
    assert_eq!(shown, 0, "an MCP read never marks an item seen");
}

/// spec: MCP reads are logged, never seen
/// a read that ran answers even when the read log can't be written: the append never masks the
/// read's own outcome
#[test]
fn a_read_answers_even_when_the_read_log_cannot_be_written() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    let mut session = env.start("2026-11-19T10:04:00Z");
    session.initialize();
    ok_result(&session.call("queue", json!({}))["result"]);
    // The database file becomes read-only while its directory stays writable: the read still
    // opens (the WAL side files live in the directory), but the append can't write the file.
    let database = env.database();
    let before = fs::metadata(&database).unwrap().permissions();
    fs::set_permissions(&database, fs::Permissions::from_mode(0o444)).unwrap();
    let writable = fs::OpenOptions::new().write(true).open(&database).is_ok();
    let call = session.call("queue", json!({}));
    fs::set_permissions(&database, before).unwrap();
    if writable {
        // As root nothing is refused; there is nothing to check.
        return;
    }
    ok_result(&call["result"]);
    let (status, stderr) = session.finish();
    assert!(status.success());
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(
        stderr.contains("read log append failed"),
        "no diagnostic: {stderr}"
    );
}
