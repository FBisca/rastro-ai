//! `rastro actions stop <run>` ends a pending Act for good: the owner's stop intent is recorded,
//! the run reads `stopped`, a tick after the grace sends nothing, and the proposal leaves the
//! queue. Stopping a run that is not pending is refused, and a pending Act's proposal can't be
//! accepted, edited or rejected. Stop is CLI-only: no MCP tool stops a run or writes a rule. The
//! provider is the real `rastro-action-linear` plugin and Linear is a fake GraphQL server on
//! loopback that records every request; the clock is `RASTRO_NOW`, so the grace window passes
//! without a wait.
mod support;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    process::{Command, Output, Stdio},
    sync::mpsc::{self, Receiver},
    time::Duration,
};
use support::linear_api::{self, Fixture};
use tempfile::TempDir;

/// When the proposal is made: the rule's grace of 10 minutes ends at 10:10.
const PROPOSED_AT: &str = "2026-11-19T10:00:00Z";
/// While the grace still runs.
const STOP_AT: &str = "2026-11-19T10:05:00Z";
/// A minute before the grace ends.
const BEFORE: &str = "2026-11-19T10:09:00Z";
/// The instant the grace ends.
const DUE: &str = "2026-11-19T10:10:00Z";
/// A minute after the grace ended.
const AFTER: &str = "2026-11-19T10:11:00Z";

struct Env {
    home: TempDir,
    linear: Fixture,
}

impl Env {
    /// A home with the real Linear provider, a fake Linear to write to, the Linear source
    /// `linear-work` in scope `work` and the rule `my-issues` on team `H`. The source was checked
    /// just now, so a tick finds it fresh and only runs actions.
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
            linear: Fixture::start(),
        };
        support::provider::install_linear(&env.config_dir(), support::action_linear());
        fs::write(
            env.config_dir().join("config.toml"),
            "[[sources]]\nname = \"linear-work\"\nconnector = \"linear\"\nselector = {}\n\
             scope = \"work\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
             keep_original = false\n\n\
             [[rules]]\nname = \"my-issues\"\naction = \"linear.create_issue\"\nteam = \"H\"\n\
             assignee = \"me\"\n",
        )
        .unwrap();
        support::seed::checked_now(env.home.path(), "linear-work", "work");
        env
    }

    fn config_dir(&self) -> std::path::PathBuf {
        self.home.path().join("config")
    }

    /// `rastro args`, with the clock at `now`.
    fn rastro(&self, args: &[&str], now: &str) -> Output {
        Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_NOW", now)
            .env("LINEAR_API_KEY", linear_api::KEY)
            .env("RASTRO_LINEAR_API_URL", &self.linear.base)
            .args(args)
            .output()
            .unwrap()
    }

    /// A command that must succeed.
    fn ok(&self, args: &[&str], now: &str) {
        let out = self.rastro(args, now);
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
    }

    /// The `result` of a command that must succeed, read with `--json`.
    fn ok_json(&self, args: &[&str], now: &str) -> Value {
        let out = self.rastro(&[args, &["--json"]].concat(), now);
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"].clone()
    }

    /// Proposes an issue for team `H` at 10:00 and returns the pending Act run's id and its
    /// proposal's queue id.
    fn propose(&self) -> (String, String) {
        let answer = self.ok_json(
            &[
                "actions",
                "propose",
                "linear.create_issue",
                "--title",
                "Retry the stale sync",
                "--team",
                "H",
                "--why",
                "owner asked",
            ],
            PROPOSED_AT,
        );
        let run = &answer["run"];
        assert_eq!(run["status"], "pending", "{answer}");
        (
            run["id"].as_str().unwrap().to_string(),
            answer["id"].as_str().unwrap().to_string(),
        )
    }

    /// The one run `rastro actions` lists.
    fn run(&self, now: &str) -> Value {
        let runs = self.runs(now);
        assert_eq!(runs.len(), 1, "{runs:?}");
        runs[0].clone()
    }

    /// The runs `rastro actions` lists.
    fn runs(&self, now: &str) -> Vec<Value> {
        self.ok_json(&["actions"], now)["runs"]
            .as_array()
            .unwrap()
            .clone()
    }

    /// The queue's items, as a machine reads them.
    fn queue_items(&self, now: &str) -> Vec<Value> {
        self.ok_json(&["queue"], now)["items"]
            .as_array()
            .unwrap()
            .clone()
    }

    /// How many owner intents the store holds.
    fn intents(&self) -> i64 {
        let database = Connection::open(self.home.path().join("data/rastro.sqlite3")).unwrap();
        database
            .query_row("SELECT count(*) FROM intent", [], |row| row.get(0))
            .unwrap()
    }

    /// A running `rastro mcp` over this home, as an agent's client sees it.
    fn mcp(&self) -> Mcp {
        Mcp::start(self)
    }
}

/// A running `rastro mcp` and the scripted client's requests to it, the pattern of the `mcp`
/// suite: one JSON-RPC message per line, a deadline that only fails a stuck server.
struct Mcp {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    lines: Receiver<String>,
    next_id: u64,
}

/// Generous: a deadline here only fails a server that is stuck, never one that is slow.
const DEADLINE: Duration = Duration::from_secs(30);

impl Mcp {
    fn start(env: &Env) -> Self {
        let mut child = Command::new(support::rastro())
            .env("RASTRO_HOME", env.home.path())
            .env("RASTRO_NOW", PROPOSED_AT)
            .env("LINEAR_API_KEY", linear_api::KEY)
            .env("RASTRO_LINEAR_API_URL", &env.linear.base)
            .arg("mcp")
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
        // Drain stderr so a chatty server never blocks on a full pipe.
        let mut stderr = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            let mut sink = Vec::new();
            let _ = std::io::Read::read_to_end(&mut stderr, &mut sink);
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

    /// Sends a request and returns the response to it.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let line = self
                .lines
                .recv_timeout(DEADLINE)
                .expect("the server printed nothing within its deadline");
            let message: Value =
                serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}"));
            if message["id"] == json!(id) && message.get("method").is_none() {
                return message;
            }
        }
    }

    fn initialize(&mut self) {
        self.request(
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {},
                   "clientInfo": {"name": "scripted-client", "version": "1.0"}}),
        );
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    }

    fn list_tools(&mut self) -> Vec<Value> {
        self.request("tools/list", json!({}))["result"]["tools"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
    }

    /// Closes stdin and waits for the exit the protocol promises.
    fn finish(&mut self) {
        drop(self.stdin.take());
        let deadline = std::time::Instant::now() + DEADLINE;
        loop {
            if self.child.try_wait().unwrap().is_some() {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the server did not exit after stdin closed"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// spec: The owner stops a pending Act
#[test]
fn a_stopped_act_never_runs_and_its_proposal_leaves_the_queue() {
    let env = Env::new();
    let (run, proposal) = env.propose();

    let stopped = env.ok_json(&["actions", "stop", &run], STOP_AT);
    assert_eq!(stopped["status"], "stopped", "{stopped}");

    // The grace ends and a tick runs: nothing reaches Linear, and the run reads stopped.
    env.ok(&["tick"], DUE);
    assert!(
        env.linear.requests().is_empty(),
        "a tick after the grace reached Linear: {:?}",
        env.linear.requests()
    );
    assert_eq!(env.run(DUE)["status"], "stopped");

    // A stopped run is final: its proposal waits nowhere.
    let items = env.queue_items(DUE);
    assert!(
        items.iter().all(|item| item["id"] != proposal.as_str()),
        "the stopped proposal is still queued: {items:?}"
    );
}

// spec: The owner stops a pending Act
#[test]
fn stopping_a_done_run_is_refused_naming_undo_and_records_nothing() {
    let env = Env::new();
    let (run, _proposal) = env.propose();
    env.ok(&["tick"], DUE);
    assert_eq!(env.run(DUE)["status"], "done");
    let intents_before = env.intents();
    let received = env.linear.requests().len();

    let out = env.rastro(&["actions", "stop", &run], AFTER);

    assert!(!out.status.success(), "stopping a done run succeeded");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&format!("rastro actions undo {run}")),
        "{stderr}"
    );
    assert_eq!(env.intents(), intents_before, "a refusal records nothing");
    assert_eq!(env.linear.requests().len(), received, "nothing was sent");
    assert_eq!(env.run(AFTER)["status"], "done");
}

// spec: A pending Act runs only once
#[test]
fn answering_a_pending_acts_proposal_is_refused_naming_the_run_and_its_stop_command() {
    let env = Env::new();
    let (run, proposal) = env.propose();

    // One contract, three answers: accept, edit and reject each refuse the same way.
    for (label, args) in [
        ("accept", vec!["queue", "accept", proposal.as_str()]),
        (
            "edit",
            vec!["queue", "edit", proposal.as_str(), "--title", "Edited"],
        ),
        (
            "reject",
            vec!["queue", "reject", proposal.as_str(), "--reason", "not now"],
        ),
    ] {
        let out = env.rastro(&args, BEFORE);
        assert!(!out.status.success(), "{label} succeeded: {out:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(&run), "{label}: {stderr}");
        assert!(
            stderr.contains(&format!("rastro actions stop {run}")),
            "{label}: {stderr}"
        );
        // One proposal never has two runs: the pending Act is the only one.
        let runs = env.runs(BEFORE);
        assert_eq!(runs.len(), 1, "{label}: {runs:?}");
        assert_eq!(runs[0]["id"], run.as_str(), "{label}: {runs:?}");
        assert_eq!(runs[0]["status"], "pending", "{label}: {runs:?}");
    }
    assert!(
        env.linear.requests().is_empty(),
        "a refused answer reached Linear"
    );
}

// spec: The owner stops a pending Act
#[test]
fn no_mcp_tool_stops_a_run_or_writes_a_rule() {
    let env = Env::new();
    let (run, _proposal) = env.propose();
    let mut mcp = env.mcp();
    mcp.initialize();

    // Every tool is a read: none stops a run, and a read-only surface writes no rule.
    let tools = mcp.list_tools();
    assert!(!tools.is_empty(), "the server lists no tools");
    for tool in &tools {
        let name = tool["name"].as_str().unwrap();
        assert!(!name.contains("stop"), "a tool stops a run: {tool}");
        assert_eq!(
            tool["annotations"]["readOnlyHint"], true,
            "a tool writes: {tool}"
        );
    }

    // Calling the stop operation anyway is a protocol error that records nothing.
    let response = mcp.call("actions_stop", json!({"run": run}));
    assert!(response.get("result").is_none(), "{response}");
    assert_eq!(response["error"]["code"], -32602, "{response}");
    assert_eq!(env.intents(), 0, "the refused call recorded an intent");
    mcp.finish();
}
