//! Where a run's proposal stands in the queue, as the run's own read says it: `actions show` on
//! the CLI and the MCP `actions` tool both state whether the proposal still waits in `rastro queue`
//! and, when it does not, what took it out, by the same rule the queue read uses. A returned Act's
//! proposal waits for the owner; a done Act's proposal left because that run settled it; an
//! accepted proposal left because of the owner's accept, naming the accept intent. The provider is
//! the real `rastro-action-linear` plugin against a fake Linear, and the clock is `RASTRO_NOW`, so
//! the grace window passes without a wait.
mod support;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio},
};
use support::linear_api::{self, Fixture};
use tempfile::TempDir;

/// When the proposal is made: the rule's grace of 10 minutes ends at 10:10.
const PROPOSED_AT: &str = "2026-11-19T10:00:00Z";
/// The instant the grace ends.
const DUE: &str = "2026-11-19T10:10:00Z";

const TITLE: &str = "Ship the doc";

/// The Linear source every home registers, in scope `work`.
const SOURCE: &str = "[[sources]]\nname = \"linear-work\"\nconnector = \"linear\"\nselector = {}\n\
     scope = \"work\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
     keep_original = false\n";
/// The rule `my-issues` on team `H`, under which a matching proposal starts an Act.
const RULE: &str = "\n[[rules]]\nname = \"my-issues\"\naction = \"linear.create_issue\"\nteam = \"H\"\n\
     assignee = \"me\"\n";

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
        env.set_rules(RULE);
        support::seed::checked_now(env.home.path(), "linear-work", "work");
        env
    }

    fn config_dir(&self) -> std::path::PathBuf {
        self.home.path().join("config")
    }

    /// Writes `config.toml` as the Linear source followed by `rules` (empty: no rule).
    fn set_rules(&self, rules: &str) {
        fs::write(
            self.config_dir().join("config.toml"),
            format!("{SOURCE}{rules}"),
        )
        .unwrap();
    }

    /// `rastro args`, with the clock at `now`.
    fn command(&self, args: &[&str], now: &str) -> Command {
        let mut command = Command::new(support::rastro());
        command
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_NOW", now)
            .env("LINEAR_API_KEY", linear_api::KEY)
            .env("RASTRO_LINEAR_API_URL", &self.linear.base)
            .args(args);
        command
    }

    fn rastro(&self, args: &[&str], now: &str) -> Output {
        self.command(args, now).output().unwrap()
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

    /// Proposes `TITLE` for team `H` at 10:00 and returns the whole answer: the proposal's queue
    /// id and, when a rule matched, the pending Act run it started.
    fn propose(&self) -> Value {
        self.ok_json(
            &[
                "actions",
                "propose",
                "linear.create_issue",
                "--title",
                TITLE,
                "--team",
                "H",
                "--why",
                "owner asked",
            ],
            PROPOSED_AT,
        )
    }

    /// The one run `rastro actions` lists.
    fn run(&self) -> Value {
        let runs = self.ok_json(&["actions"], DUE)["runs"].clone();
        let runs = runs.as_array().unwrap();
        assert_eq!(runs.len(), 1, "{runs:?}");
        runs[0].clone()
    }

    /// The queue's item ids, as the CLI reads them.
    fn queued_ids(&self) -> Vec<String> {
        self.ok_json(&["queue"], DUE)["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["id"].as_str().unwrap().to_string())
            .collect()
    }

    /// An MCP server launched with `scopes`.
    fn serve(&self, scopes: &[&str]) -> Mcp {
        let mut args = vec!["mcp"];
        for scope in scopes {
            args.extend(["--scope", scope]);
        }
        Mcp::start(self.command(&args, DUE))
    }
}

/// A scripted MCP client: raw JSON lines over the server's stdio.
struct Mcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Mcp {
    fn start(mut command: Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut mcp = Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        };
        mcp.request(
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {},
                   "clientInfo": {"name": "scripted-client", "version": "1.0"}}),
        );
        mcp.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        mcp
    }

    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            assert_ne!(
                self.stdout.read_line(&mut line).unwrap(),
                0,
                "server exited"
            );
            let message: Value = serde_json::from_str(&line).unwrap();
            if message["id"] == json!(id) && message.get("method").is_none() {
                return message;
            }
        }
    }

    /// The v2 envelope of a successful tool call.
    fn read(&mut self, tool: &str, arguments: Value) -> Value {
        let call = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        let result = &call["result"];
        assert!(result.is_object(), "{tool}: no result: {call}");
        assert_ne!(result["isError"], true, "{tool}: {call}");
        result["structuredContent"].clone()
    }

    /// The queue's item ids, as the MCP read gives them.
    fn queued_ids(&mut self) -> Vec<String> {
        self.read("queue", json!({}))["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["id"].as_str().unwrap().to_string())
            .collect()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// spec: A run says where its proposal stands in the queue
#[test]
fn a_returned_acts_proposal_waits_for_the_owner() {
    let env = Env::new();
    let proposed = env.propose();
    let id = proposed["run"]["id"].as_str().unwrap().to_string();
    let proposal = proposed["id"].as_str().unwrap().to_string();

    // The owner deletes the rule before the grace ends: the re-check fails and the run returns.
    env.set_rules("");
    env.ok(&["tick"], DUE);
    assert_eq!(env.run()["status"], "returned");

    // The CLI's read of the run says the proposal waits.
    let text =
        String::from_utf8_lossy(&env.rastro(&["actions", "show", &id], DUE).stdout).into_owned();
    assert!(text.contains("Queue: waiting for the owner"), "{text}");

    // The MCP read of the run says the same.
    let mut mcp = env.serve(&["work"]);
    let answer = mcp.read("actions", json!({"run": id}));
    assert_eq!(
        answer["result"]["why"]["queue"],
        json!({"state": "waiting"}),
        "{answer}"
    );

    // And the queue itself lists the proposal, on both surfaces.
    assert!(
        env.queued_ids().contains(&proposal),
        "{:?}",
        env.queued_ids()
    );
    assert!(mcp.queued_ids().contains(&proposal), "{proposal}");
}

// spec: A run says where its proposal stands in the queue
#[test]
fn a_done_acts_proposal_left_the_queue_because_the_run_is_done() {
    let env = Env::new();
    let proposed = env.propose();
    let id = proposed["run"]["id"].as_str().unwrap().to_string();
    let proposal = proposed["id"].as_str().unwrap().to_string();

    // The grace ends and the tick creates the issue: the run reads done.
    env.ok(&["tick"], DUE);
    assert_eq!(env.run()["status"], "done");

    // The CLI's read of the run says the proposal left because this run is done.
    let text =
        String::from_utf8_lossy(&env.rastro(&["actions", "show", &id], DUE).stdout).into_owned();
    assert!(
        text.contains(&format!("Queue: out, run {id} is done")),
        "{text}"
    );

    // The MCP read of the run says the same.
    let mut mcp = env.serve(&["work"]);
    let answer = mcp.read("actions", json!({"run": id}));
    assert_eq!(
        answer["result"]["why"]["queue"],
        json!({"state": "settled", "run": id, "status": "done"}),
        "{answer}"
    );

    // And the queue no longer lists the proposal, on either surface.
    assert!(!env.queued_ids().contains(&proposal), "{proposal}");
    assert!(!mcp.queued_ids().contains(&proposal), "{proposal}");
}

// spec: A run says where its proposal stands in the queue
#[test]
fn an_accepted_proposal_left_the_queue_because_of_the_owners_accept() {
    let env = Env::new();
    // No rule: the proposal waits for the owner, who accepts it.
    env.set_rules("");
    let proposed = env.propose();
    assert!(proposed.get("run").is_none(), "{proposed}");
    let proposal = proposed["id"].as_str().unwrap().to_string();
    env.ok(&["queue", "accept", &proposal], DUE);

    let run = env.run();
    assert_eq!(run["status"], "done");
    let id = run["id"].as_str().unwrap().to_string();
    let intent = run["accept_intent"].as_str().unwrap().to_string();

    // The CLI's read of the run says the proposal left because of the owner's accept, naming it.
    let text =
        String::from_utf8_lossy(&env.rastro(&["actions", "show", &id], DUE).stdout).into_owned();
    assert!(
        text.contains(&format!("Queue: out, the owner's accept (intent {intent})")),
        "{text}"
    );

    // The MCP read of the run says the same.
    let mut mcp = env.serve(&["work"]);
    let answer = mcp.read("actions", json!({"run": id}));
    assert_eq!(
        answer["result"]["why"]["queue"],
        json!({"state": "answered", "kind": "accept", "intent": intent}),
        "{answer}"
    );

    // And the queue no longer lists the proposal, on either surface.
    assert!(!env.queued_ids().contains(&proposal), "{proposal}");
    assert!(!mcp.queued_ids().contains(&proposal), "{proposal}");
}
