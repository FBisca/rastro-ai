//! A pending Act shows where the owner looks: first in `rastro queue` with the time left and
//! its stop command, and in the `actions` read on the CLI and MCP with the time it runs. The
//! provider is the real `rastro-action-linear` plugin and Linear is a fake GraphQL server on
//! loopback that records every request; the clock is `RASTRO_NOW`, so the grace window passes
//! without a wait and a read at 10:04 sees six minutes left of a ten-minute grace.
mod support;
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
/// While the grace still runs: six minutes before the Act is due.
const READ_AT: &str = "2026-11-19T10:04:00Z";

struct Env {
    home: TempDir,
    linear: Fixture,
}

impl Env {
    /// A home with the real Linear provider, a fake Linear to write to, the Linear source
    /// `linear-work` in scope `work` and the rule `my-issues` on team `H`. The source was checked
    /// just now, so a read finds it fresh and names no gap.
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
    fn propose(&self, title: &str, team: &str) -> (Option<String>, String) {
        let answer = self.ok_json(
            &[
                "actions",
                "propose",
                "linear.create_issue",
                "--title",
                title,
                "--team",
                team,
                "--why",
                "owner asked",
            ],
            PROPOSED_AT,
        );
        let run = answer["run"]["id"].as_str().map(String::from);
        (run, answer["id"].as_str().unwrap().to_string())
    }

    /// A running `rastro mcp` over this home, with the clock at `now`.
    fn mcp(&self, now: &str) -> Mcp {
        Mcp::start(self, now)
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
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
    fn start(env: &Env, now: &str) -> Self {
        let mut child = Command::new(support::rastro())
            .env("RASTRO_HOME", env.home.path())
            .env("RASTRO_NOW", now)
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

/// The `result` member of a successful tool call: not `isError`, and a v2 success envelope.
fn ok_result(result: &Value) -> &Value {
    assert!(
        result.is_object(),
        "no result (a protocol error?): {result}"
    );
    assert_ne!(result["isError"], true, "{result}");
    let envelope = &result["structuredContent"];
    assert_eq!(envelope["schema_version"], 2, "{result}");
    assert!(envelope.get("error").is_none(), "{result}");
    envelope
}

// spec: A pending Act shows where the owner looks
#[test]
fn the_actions_read_lists_a_pending_act_with_the_time_it_runs_and_its_stop_command() {
    let env = Env::new();
    let (run, _proposal) = env.propose("Retry the stale sync", "H");
    let run = run.expect("a matching proposal starts a pending Act");
    let stop = format!("rastro actions stop {run}");

    // The owner's list says when it runs and how to stop it.
    let text = stdout(&env.rastro(&["actions"], READ_AT));
    let line = text
        .lines()
        .find(|line| line.contains(&run))
        .unwrap_or_else(|| panic!("no run line in: {text}"));
    assert!(line.contains("pending"), "{line}");
    assert!(line.contains("10:10"), "{line}");
    assert!(line.contains(&stop), "{line}");

    // The machine read carries the same: pending, due at 10:10, with the stop command.
    let listed = env.ok_json(&["actions"], READ_AT)["runs"].clone();
    assert_eq!(listed[0]["status"], "pending", "{listed}");
    assert_eq!(listed[0]["due_at"], "2026-11-19T10:10:00Z", "{listed}");
    assert_eq!(listed[0]["stop_command"], stop.as_str(), "{listed}");

    // An agent over MCP reads exactly the same list.
    let mut mcp = env.mcp(READ_AT);
    mcp.initialize();
    let response = mcp.call("actions", json!({}));
    let envelope = ok_result(&response["result"]);
    assert_eq!(envelope["result"]["runs"], listed, "{response}");
    mcp.finish();

    // Reading sent nothing: the Act still waits out its grace.
    assert!(
        env.linear.requests().is_empty(),
        "a read reached Linear: {:?}",
        env.linear.requests()
    );
}

// spec: A pending Act shows where the owner looks
#[test]
fn the_queue_shows_a_pending_act_first_with_the_time_left_and_its_stop_command() {
    let env = Env::new();
    let (run, act_proposal) = env.propose("Retry the stale sync", "H");
    let run = run.expect("a matching proposal starts a pending Act");
    let (no_run, other_proposal) = env.propose("Another team's issue", "ENG");
    assert!(
        no_run.is_none(),
        "a proposal for another team starts no Act"
    );
    let stop = format!("rastro actions stop {run}");

    // The first item line names the pending Act: its title, the time left and the stop command,
    // before the action proposal that waits for the owner.
    let text = stdout(&env.rastro(&["queue"], READ_AT));
    let first = text
        .lines()
        .nth(1)
        .unwrap_or_else(|| panic!("no item line in: {text}"));
    for part in ["Retry the stale sync", "runs in 6 min", stop.as_str()] {
        assert!(first.contains(part), "{part} missing from: {first}");
    }
    let act_at = text
        .find("Retry the stale sync")
        .unwrap_or_else(|| panic!("the pending Act is not listed: {text}"));
    let other_at = text
        .find(&other_proposal)
        .unwrap_or_else(|| panic!("the other proposal is not listed: {text}"));
    assert!(act_at < other_at, "the pending Act is not first: {text}");

    // The machine read agrees: the pending Act's proposal is the first item, carrying its run,
    // and the proposal waits once, not twice.
    let items = env.ok_json(&["queue"], READ_AT)["items"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(items.len(), 2, "{items:?}");
    assert_eq!(items[0]["id"], act_proposal.as_str(), "{items:?}");
    assert_eq!(items[0]["act"]["id"], run.as_str(), "{items:?}");
    assert_eq!(items[0]["act"]["status"], "pending", "{items:?}");
    assert_eq!(
        items[0]["act"]["due_at"], "2026-11-19T10:10:00Z",
        "{items:?}"
    );
    assert_eq!(items[0]["act"]["stop_command"], stop.as_str(), "{items:?}");
    assert_eq!(items[1]["id"], other_proposal.as_str(), "{items:?}");
    assert!(items[1].get("act").is_none(), "{items:?}");
}
