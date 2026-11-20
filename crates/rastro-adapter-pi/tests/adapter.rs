//! The pi adapter over the plugin protocol, with a script standing in for `pi`, so no model is
//! called. The live proof against the installed pi lives in `live.rs`.
use rastro_plugin_sdk::{Manifest, PROTOCOL_VERSION};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

// spec: A harness adapter is a thin CLI driver bindable for one-shot judgment
// spec: An adapter's manifest declares its kind and names no tool
// spec: The core matches the adapter's declared boundary enforcement before a run
#[test]
fn the_manifest_declares_the_kind_and_capabilities_and_names_no_tool() {
    let manifest = Manifest::parse(include_str!("../rastro-plugin.toml")).unwrap();
    manifest.check_agent_tool().unwrap();
    assert_eq!(manifest.kinds, ["agent-tool-adapter"]);
    assert_eq!(
        manifest.agent_tool.unwrap().capabilities,
        ["judge_text"],
        "the pi adapter judges text; it names no tool"
    );
    assert!(manifest.permissions.actions.is_empty());
    // The full judgment floor, declared only after the live proof (tests/live.rs).
    assert_eq!(manifest.enforcement, rastro_plugin_sdk::JUDGMENT_FLOOR);
}

/// One fake `pi` stdout: a session header, then the assistant's final message carrying `answer`
/// as its text, the way `pi -p --mode json` prints it.
fn pi_output(session_id: &str, answer: &Value) -> String {
    let lines = [
        json!({"type": "session", "version": 3, "id": session_id, "timestamp": "2026-11-20T00:00:00.000Z", "cwd": "/fake"}),
        json!({"type": "agent_start"}),
        json!({"type": "message_end", "message": {"role": "user", "content": "the prompt"}}),
        json!({"type": "message_end", "message": {"role": "assistant", "content": [{"type": "text", "text": answer.to_string()}], "stopReason": "stop"}}),
        json!({"type": "agent_settled", "aborted": false}),
    ];
    lines
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

/// A fake `pi` script: it saves its arguments, prompt and working directory per invocation,
/// keeps a session file when the real pi would, and prints the output prepared for that turn.
fn write_fake_pi(dir: &Path, outputs: &[&str], keeps_session: bool, exit_code: u8) -> PathBuf {
    for (n, output) in outputs.iter().enumerate() {
        fs::write(dir.join(format!("output-{}", n + 1)), output).unwrap();
    }
    let keeps = if keeps_session { "yes" } else { "no" };
    let fake = dir.join("pi");
    fs::write(
        &fake,
        format!(
            r#"#!/bin/sh
d='{0}'
n=$(cat "$d/count" 2>/dev/null || echo 0)
n=$((n+1))
echo "$n" > "$d/count"
printf '%s\n' "$@" > "$d/args-$n"
pwd > "$d/pwd-$n"
cat > "$d/prompt-$n"
sd=''
prev=''
for a in "$@"; do
  if [ "$prev" = '--session-dir' ]; then sd="$a"; fi
  prev="$a"
done
if [ -n "$sd" ] && [ '{1}' = 'yes' ]; then
  mkdir -p "$sd/--fake--"
  printf '%s\n' '{{"type":"session","version":3,"id":"fake-session-1","timestamp":"2026-11-20T00:00:00.000Z","cwd":"/fake"}}' > "$sd/--fake--/20261120_fake-session-1.jsonl"
fi
cat "$d/output-$n"
exit {2}
"#,
            dir.display(),
            keeps,
            exit_code
        ),
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    fake
}

/// The adapter process talking to a fake pi, with RASTRO_HOME inside a temporary directory.
struct Adapter {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    lines: std::io::Lines<BufReader<std::process::ChildStdout>>,
    dir: tempfile::TempDir,
    next_id: u64,
}
impl Adapter {
    fn start(outputs: &[&str], keeps_session: bool) -> Self {
        Self::start_exiting(outputs, keeps_session, 0)
    }
    fn start_exiting(outputs: &[&str], keeps_session: bool, exit_code: u8) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let fake = write_fake_pi(dir.path(), outputs, keeps_session, exit_code);
        let mut child = Command::new(env!("CARGO_BIN_EXE_rastro-adapter-pi"))
            .env("RASTRO_PI_COMMAND", &fake)
            .env("RASTRO_HOME", dir.path().join("home"))
            .env_remove("RASTRO_PI_MODEL")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        Self {
            stdin: child.stdin.take().unwrap(),
            lines: BufReader::new(child.stdout.take().unwrap()).lines(),
            child,
            dir,
            next_id: 1,
        }
    }
    fn ask(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        )
        .unwrap();
        serde_json::from_str(&self.lines.next().unwrap().unwrap()).unwrap()
    }
    fn initialize(&mut self) -> Value {
        self.ask(
            "initialize",
            json!({"protocol": PROTOCOL_VERSION, "manifest_id": "rastro-adapter-pi", "role": "agent_tool_adapter"}),
        )
    }
    fn judge(&mut self, params: Value) -> Value {
        self.ask("judgment.request", params)
    }
    fn args(&self, turn: usize) -> Vec<String> {
        fs::read_to_string(self.dir.path().join(format!("args-{turn}")))
            .unwrap()
            .lines()
            .map(String::from)
            .collect()
    }
    fn prompt(&self, turn: usize) -> String {
        fs::read_to_string(self.dir.path().join(format!("prompt-{turn}"))).unwrap()
    }
    fn pwd(&self, turn: usize) -> String {
        fs::read_to_string(self.dir.path().join(format!("pwd-{turn}")))
            .unwrap()
            .trim()
            .to_string()
    }
    fn session_dir(&self) -> PathBuf {
        self.dir.path().join("home/pi-sessions")
    }
}
impl Drop for Adapter {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn one_shot_params() -> Value {
    json!({"instructions": "Find commitments.", "owner_speaker": "Alice",
        "turns": [{"turn": 1, "speaker": "Bob", "text": "I'll send it."}],
        "fields": ["turn", "kind", "quote"]})
}

/// A retry turn of the same session: the core's gate feedback and the prior answer, whose
/// `session_id` is the harness session the first turn produced (null when it produced none).
fn retry_params(session_id: Option<&str>) -> Value {
    json!({"instructions": "Find commitments.", "owner_speaker": "Alice",
        "turns": [{"turn": 1, "speaker": "Bob", "text": "I'll send it."}],
        "fields": ["turn", "kind", "quote"],
        "session": {"turn": 2,
            "feedback": [{"gate": "quote", "field": "quote",
                "expected": "an exact contiguous piece of turn 1",
                "actual": "I'll send it tomorrow."}],
            "prior_answer": {"candidates": [{"turn": 1, "kind": "promise",
                "promise": "send it", "quote": "I'll send it tomorrow."}],
                "session_id": session_id}}})
}

// spec: A harness adapter is a thin CLI driver bindable for one-shot judgment
#[test]
fn a_one_shot_request_drives_pi_under_lockdown_and_relays_the_answer() {
    let answer = json!({"candidates": [{"turn": 1, "kind": "promise", "promise": "send it", "quote": "I'll send it."}]});
    let mut adapter = Adapter::start(&[&pi_output("fake-session-1", &answer)], true);
    let initialized = adapter.initialize();
    assert_eq!(
        initialized["result"],
        json!({"protocol": PROTOCOL_VERSION, "manifest_id": "rastro-adapter-pi"})
    );
    let judged = adapter.judge(one_shot_params());
    assert_eq!(judged["result"]["candidates"], answer["candidates"]);
    assert_eq!(judged["result"]["session_id"], "fake-session-1");
    assert!(judged["result"].get("degraded").is_none(), "{judged}");

    // pi runs print-only under the full lockdown kit, its sessions kept under RASTRO_HOME.
    let args = adapter.args(1);
    for flag in [
        "-p",
        "--no-tools",
        "--no-mcp",
        "--no-extensions",
        "--no-skills",
        "--no-prompt-templates",
        "--no-themes",
        "--no-context-files",
        "--no-approve",
        "--system-prompt",
        "--session-dir",
    ] {
        assert!(args.iter().any(|a| a == flag), "{flag} missing: {args:?}");
    }
    assert!(
        args.windows(2).any(|w| w[0] == "--mode" && w[1] == "json"),
        "{args:?}"
    );
    assert!(
        !args.iter().any(|a| a == "--session-id"),
        "a first turn opens no existing session: {args:?}"
    );
    let session_dir = adapter.session_dir();
    assert!(
        args.windows(2)
            .any(|w| w[0] == "--session-dir" && w[1] == session_dir.to_str().unwrap()),
        "{args:?}"
    );
    assert_eq!(
        adapter.pwd(1),
        fs::canonicalize(&session_dir).unwrap().to_str().unwrap()
    );

    let prompt = adapter.prompt(1);
    assert!(prompt.starts_with("Find commitments."));
    let (_, data) = prompt.split_once("\n\nData:\n").expect("a data section");
    let data: Value = serde_json::from_str(data.trim()).unwrap();
    assert_eq!(data["owner_speaker"], "Alice");
    assert_eq!(data["turns"][0]["text"], "I'll send it.");
}

// spec: A session turn resumes the harness or degrades recorded
#[test]
fn a_retry_turn_resumes_the_persisted_session_in_a_fresh_process() {
    let first = json!({"candidates": [{"turn": 1, "kind": "promise", "promise": "send it", "quote": "I'll send it tomorrow."}]});
    let second = json!({"candidates": [{"turn": 1, "kind": "promise", "promise": "send it", "quote": "I'll send it."}]});
    let mut adapter = Adapter::start(
        &[
            &pi_output("fake-session-1", &first),
            &pi_output("fake-session-1", &second),
        ],
        true,
    );
    adapter.initialize();
    let turn1 = adapter.judge(one_shot_params());
    let session_id = turn1["result"]["session_id"].as_str().unwrap().to_string();
    let turn2 = adapter.judge(retry_params(Some(&session_id)));
    assert_eq!(turn2["result"]["candidates"], second["candidates"]);
    assert_eq!(turn2["result"]["session_id"], json!(session_id));
    assert!(turn2["result"].get("degraded").is_none(), "{turn2}");

    // A second pi process resumes the persisted id from the same directory, lockdown re-applied.
    assert!(
        adapter.dir.path().join("args-2").exists(),
        "the retry turn must run a fresh pi process"
    );
    let args = adapter.args(2);
    assert!(
        args.windows(2)
            .any(|w| w[0] == "--session-id" && w[1] == session_id),
        "{args:?}"
    );
    let session_dir = adapter.session_dir();
    assert!(
        args.windows(2)
            .any(|w| w[0] == "--session-dir" && w[1] == session_dir.to_str().unwrap()),
        "{args:?}"
    );
    for flag in [
        "--no-tools",
        "--no-mcp",
        "--no-extensions",
        "--no-skills",
        "--no-prompt-templates",
        "--no-themes",
        "--no-context-files",
        "--no-approve",
    ] {
        assert!(args.iter().any(|a| a == flag), "{flag} missing: {args:?}");
    }
    assert_eq!(adapter.pwd(2), adapter.pwd(1));
    let prompt = adapter.prompt(2);
    assert!(prompt.contains("quote"), "{prompt}");
    assert!(
        prompt.contains("an exact contiguous piece of turn 1"),
        "{prompt}"
    );
    assert!(prompt.contains("I'll send it tomorrow."), "{prompt}");
}

// spec: A session turn resumes the harness or degrades recorded
#[test]
fn an_unusable_resume_degrades_to_a_fresh_turn_recorded() {
    let first = json!({"candidates": [{"turn": 1, "kind": "promise", "promise": "send it", "quote": "I'll send it tomorrow."}]});
    let second = json!({"candidates": [{"turn": 1, "kind": "promise", "promise": "send it", "quote": "I'll send it."}]});
    // This pi keeps no session file: the id the first turn reported cannot be resumed.
    let mut adapter = Adapter::start(
        &[
            &pi_output("fake-session-1", &first),
            &pi_output("fake-session-2", &second),
        ],
        false,
    );
    adapter.initialize();
    let turn1 = adapter.judge(one_shot_params());
    let session_id = turn1["result"]["session_id"].as_str().unwrap().to_string();
    let turn2 = adapter.judge(retry_params(Some(&session_id)));
    assert_eq!(turn2["result"]["degraded"], json!(true), "{turn2}");
    assert_eq!(turn2["result"]["candidates"], second["candidates"]);

    // A fresh turn, not a resume, carrying the prior answer and the failure reason as data.
    let args = adapter.args(2);
    assert!(
        !args.iter().any(|a| a == "--session-id"),
        "an unusable resume must not be attempted: {args:?}"
    );
    let prompt = adapter.prompt(2);
    assert!(prompt.contains("I'll send it tomorrow."), "{prompt}");
    assert!(
        prompt.contains("an exact contiguous piece of turn 1"),
        "{prompt}"
    );
    assert!(prompt.contains("\n\nData:\n"), "{prompt}");
    assert!(prompt.contains("I'll send it."), "{prompt}");
}

// spec: A harness adapter is a thin CLI driver bindable for one-shot judgment
#[test]
fn a_pi_failure_is_an_error_response_not_a_result() {
    let mut adapter = Adapter::start_exiting(&["partial output"], true, 1);
    adapter.initialize();
    let judged = adapter.judge(one_shot_params());
    assert!(judged.get("result").is_none(), "{judged}");
    assert!(
        judged["error"]["message"]
            .as_str()
            .unwrap()
            .contains("exited"),
        "{judged}"
    );
}
