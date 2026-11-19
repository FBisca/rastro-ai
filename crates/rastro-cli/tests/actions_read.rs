//! The `actions` read over MCP: an agent asks why a run happened and gets the proposal, the why,
//! each cited claim with its evidence, the owner's accept and the cell's risk; a server launched
//! with another scope sees neither the proposal nor the run. The provider is the real
//! `rastro-action-linear` plugin against a fake Linear, so the run is made by a real accept.
mod support;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio},
};
use support::linear_api::{self, Fixture};
use tempfile::TempDir;

struct Env {
    home: TempDir,
    linear: Fixture,
}

impl Env {
    /// A home with the Linear action provider installed, the Linear source `linear-work` in scope
    /// `work` and `[actions.linear] default_team = "ENG"`, and a fake Linear to write to.
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
            linear: Fixture::start(),
        };
        let config = env.home.path().join("config");
        support::provider::install_linear(&config, support::action_linear());
        fs::write(
            config.join("config.toml"),
            "[[sources]]\nname = \"linear-work\"\nconnector = \"linear\"\nselector = {}\n\
             scope = \"work\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
             keep_original = false\n\n[actions.linear]\ndefault_team = \"ENG\"\n",
        )
        .unwrap();
        env
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(support::rastro());
        command
            .env("RASTRO_HOME", self.home.path())
            .env("LINEAR_API_KEY", linear_api::KEY)
            .env("RASTRO_LINEAR_API_URL", &self.linear.base)
            .args(args);
        command
    }

    fn rastro(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    /// The `result` of a command that must succeed, read with `--json`.
    fn ok_json(&self, args: &[&str]) -> Value {
        let out = self.rastro(&[args, &["--json"]].concat());
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"].clone()
    }

    /// Proposes `linear.create_issue` with `extra` after the title and returns the item's id.
    fn propose(&self, title: &str, extra: &[&str]) -> String {
        let mut args = vec![
            "actions",
            "propose",
            "linear.create_issue",
            "--title",
            title,
        ];
        args.extend_from_slice(extra);
        self.ok_json(&args)["id"].as_str().unwrap().to_string()
    }

    /// An MCP server launched with `scopes`.
    fn serve(&self, scopes: &[&str]) -> Mcp {
        let mut args = vec!["mcp"];
        for scope in scopes {
            args.extend(["--scope", scope]);
        }
        Mcp::start(self.command(&args))
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
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// spec: The owner reads why an action ran
#[test]
fn an_agent_asking_the_actions_tool_for_a_run_gets_why_it_ran() {
    let env = Env::new();
    let claims = [
        support::seed::claim_in(env.home.path(), "work", "o/p#1"),
        support::seed::claim_in(env.home.path(), "work", "o/p#2"),
    ];
    let item = env.propose(
        "Flaky sync test",
        &[
            "--description",
            "Seen twice in CI.",
            "--why",
            "owner asked in session after the red build",
            "--cite",
            &claims[0],
            "--cite",
            &claims[1],
        ],
    );
    env.ok_json(&["queue", "accept", &item]);
    let run = env.ok_json(&["actions"])["runs"][0].clone();
    let id = run["id"].as_str().unwrap();

    let mut mcp = env.serve(&["work"]);
    let answer = mcp.read("actions", json!({"run": id}));

    assert_eq!(answer["operation"], "actions");
    let result = &answer["result"];
    let shown = &result["runs"][0];
    assert_eq!(shown["id"], id);
    assert_eq!(shown["action"], "linear.create_issue");
    assert_eq!(shown["status"], "done");
    assert_eq!(shown["identifier"], "ENG-1");
    assert_eq!(
        shown["fields"],
        json!({"title": "Flaky sync test", "description": "Seen twice in CI.",
               "team": "ENG", "assignee": "me"})
    );
    let why = &result["why"];
    assert_eq!(
        why["proposal"]["why"],
        "owner asked in session after the red build"
    );
    let mut cited: Vec<&str> = why["cited"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["claim"]["id"].as_str().unwrap())
        .collect();
    cited.sort_unstable();
    let mut expected: Vec<&str> = claims.iter().map(String::as_str).collect();
    expected.sort_unstable();
    assert_eq!(cited, expected, "{why}");
    for claim in why["cited"].as_array().unwrap() {
        assert_eq!(
            claim["supports"][0]["spans"][0]["excerpt"]["text"], "open",
            "a cited claim carries its evidence: {claim}"
        );
    }
    assert_eq!(why["accept"]["intent"], run["accept_intent"]);
    assert_eq!(why["accept"]["at"], run["accepted_at"]);
    assert_eq!(why["level"], "propose");
    assert_eq!(
        why["cell"],
        json!({"action": "linear.create_issue", "team": "ENG"})
    );
    assert_eq!(why["reach"], "others see it");
    assert_eq!(why["reversibility"], "can be undone");

    // The CLI says the same under `actions show`: the same result, whatever the envelope names.
    let cli: Value =
        serde_json::from_slice(&env.rastro(&["actions", "show", id, "--json"]).stdout).unwrap();
    assert_eq!(cli["result"], answer["result"]);

    // The owner reads the same in words.
    let text = String::from_utf8_lossy(&env.rastro(&["actions", "show", id]).stdout).into_owned();
    for part in [
        "Why: owner asked in session after the red build",
        &format!("Cites: {}", claims[0]),
        &format!(
            "Accepted: intent {}",
            run["accept_intent"].as_str().unwrap()
        ),
        "Level: propose | Cell: linear.create_issue x ENG | Reach: others see it | Reversibility: can be undone",
    ] {
        assert!(text.contains(part), "{part} missing: {text}");
    }
}

// spec: The owner reads why an action ran
#[test]
fn a_server_launched_with_another_scope_sees_neither_the_proposal_nor_the_run() {
    let env = Env::new();
    let accepted = env.propose("Flaky sync test", &["--why", "owner asked"]);
    env.ok_json(&["queue", "accept", &accepted]);
    let waiting = env.propose("Slow search", &["--why", "owner asked again"]);
    let run = env.ok_json(&["actions"])["runs"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut elsewhere = env.serve(&["personal"]);
    let queue = elsewhere.read("queue", json!({}));
    let actions = elsewhere.read("actions", json!({}));
    assert_eq!(queue["result"]["items"], json!([]), "{queue}");
    assert_eq!(actions["result"]["runs"], json!([]), "{actions}");
    let one = elsewhere.request(
        "tools/call",
        json!({"name": "actions", "arguments": {"run": run}}),
    );
    assert_eq!(one["result"]["isError"], true, "{one}");

    // The same data is there for the scope the Linear source is in.
    let mut at_work = env.serve(&["work"]);
    let queue = at_work.read("queue", json!({}));
    let ids: Vec<&Value> = queue["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| &item["id"])
        .collect();
    assert_eq!(ids, [&json!(waiting)], "{queue}");
    let actions = at_work.read("actions", json!({}));
    assert_eq!(actions["result"]["runs"][0]["id"], json!(run), "{actions}");
}

// spec: The owner reads why an action ran
#[test]
fn a_run_is_explained_after_its_provider_is_uninstalled() {
    let env = Env::new();
    let item = env.propose("Flaky sync test", &["--why", "owner asked in session"]);
    env.ok_json(&["queue", "accept", &item]);
    let id = env.ok_json(&["actions"])["runs"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    fs::remove_dir_all(env.home.path().join("config/plugins/rastro-action-linear")).unwrap();

    let mut mcp = env.serve(&["work"]);
    let answer = mcp.read("actions", json!({"run": id}));

    let why = &answer["result"]["why"];
    assert_eq!(why["proposal"]["why"], "owner asked in session");
    assert_eq!(why["reach"], "others see it", "{why}");
    assert_eq!(why["reversibility"], "can be undone", "{why}");
}
