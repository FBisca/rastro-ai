//! The codex adapter over the plugin protocol, with a script standing in for `codex`, so no
//! model is called.
use rastro_plugin_sdk::{JUDGMENT_FLOOR, Manifest, PROTOCOL_VERSION};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

// spec: An adapter's manifest declares its kind and names no tool
#[test]
fn the_manifest_declares_the_kind_and_capabilities_and_names_no_tool() {
    let manifest = Manifest::parse(include_str!("../rastro-plugin.toml")).unwrap();
    manifest.check_agent_tool().unwrap();
    assert_eq!(manifest.kinds, ["agent-tool-adapter"]);
    assert_eq!(
        manifest.agent_tool.unwrap().capabilities,
        ["judge_text".to_string()]
    );
    assert!(manifest.permissions.actions.is_empty());
    assert_eq!(manifest.enforcement, JUDGMENT_FLOOR);
}

/// What one fake `codex` invocation received.
struct Turn {
    args: Vec<String>,
    prompt: String,
    codex_home: String,
    schema: Value,
    /// The working directory the codex process ran in.
    cwd: String,
}

/// What the fake `codex` does on a resume invocation.
enum Resume {
    /// Prints this JSONL and exits 0.
    Answer(String),
    /// Prints this to stderr and exits 1: the session is not usable.
    Fail(String),
}

/// One conversation with the adapter: the replies it sent and what the fake harness received.
/// The temporary directories stay alive for the assertions that read them.
struct Conversation {
    initialized: Value,
    replies: Vec<Value>,
    turns: Vec<Turn>,
    rastro_home: PathBuf,
    _dir: tempfile::TempDir,
}

/// Runs the adapter against a fake `codex` that records every invocation's arguments, prompt,
/// `CODEX_HOME` and schema file, and prints `fresh` on a fresh turn and `resume` on a resume.
/// The adapter's `RASTRO_HOME` is a directory under the fake's own, and its `HOME` holds no
/// codex login, so no credential is touched.
/// Writes the fake `codex` script: it records each invocation's arguments, prompt,
/// `CODEX_HOME`, working directory and schema file, then prints the canned JSONL.
fn write_fake_codex(path: &Path, fresh: &str, resume: Resume) -> PathBuf {
    let fake = path.join("codex");
    let resume_branch = match resume {
        Resume::Answer(output) => format!("cat <<'EOF'\n{output}\nEOF\n"),
        Resume::Fail(message) => format!("echo '{message}' >&2\nexit 1\n"),
    };
    fs::write(
        &fake,
        format!(
            r#"#!/bin/sh
n=$(cat '{0}/n' 2>/dev/null || echo 0)
n=$((n+1))
printf '%s' "$n" > '{0}/n'
printf '%s\n' "$@" > "{0}/args.$n"
cat > "{0}/prompt.$n"
printf '%s\n' "${{CODEX_HOME:-}}" > "{0}/home.$n"
pwd > "{0}/cwd.$n"
prev=""
for a in "$@"; do
  if [ "$prev" = "--output-schema" ]; then cat "$a" > "{0}/schema.$n"; fi
  prev="$a"
done
if [ "$1" = "exec" ] && [ "$2" = "resume" ]; then
{1}else
cat <<'EOF'
{2}
EOF
fi
"#,
            path.display(),
            resume_branch,
            fresh
        ),
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    fake
}

fn converse(fresh: &str, resume: Resume, requests: &[Value]) -> Conversation {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    let fake = write_fake_codex(path, fresh, resume);
    let home = path.join("fake-home");
    fs::create_dir_all(&home).unwrap();
    let rastro_home = path.join("rastro-home");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rastro-adapter-codex"))
        .env("RASTRO_CODEX_COMMAND", &fake)
        .env("RASTRO_HOME", &rastro_home)
        .env("HOME", &home)
        .env_remove("CODEX_HOME")
        .env_remove("CODEX_API_KEY")
        .env_remove("RASTRO_CODEX_MODEL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut ask = |message: Value| {
        writeln!(stdin, "{message}").unwrap();
        serde_json::from_str::<Value>(&lines.next().unwrap().unwrap()).unwrap()
    };
    let initialized = ask(
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocol": PROTOCOL_VERSION, "manifest_id": "rastro-adapter-codex",
        "role": "agent_tool_adapter"}}),
    );
    let mut replies = Vec::new();
    for (i, params) in requests.iter().enumerate() {
        replies.push(ask(json!({
            "jsonrpc": "2.0", "id": i + 2, "method": "judgment.request", "params": params
        })));
    }
    drop(stdin);
    child.wait().unwrap();
    let count: usize = fs::read_to_string(path.join("n"))
        .unwrap_or_else(|_| "0".into())
        .trim()
        .parse()
        .unwrap();
    let turns = (1..=count)
        .map(|n| Turn {
            args: fs::read_to_string(path.join(format!("args.{n}")))
                .unwrap()
                .lines()
                .map(String::from)
                .collect(),
            prompt: fs::read_to_string(path.join(format!("prompt.{n}"))).unwrap(),
            codex_home: fs::read_to_string(path.join(format!("home.{n}")))
                .unwrap()
                .trim()
                .into(),
            schema: serde_json::from_str(
                &fs::read_to_string(path.join(format!("schema.{n}"))).unwrap(),
            )
            .unwrap(),
            cwd: fs::read_to_string(path.join(format!("cwd.{n}")))
                .unwrap()
                .trim()
                .into(),
        })
        .collect();
    Conversation {
        initialized,
        replies,
        turns,
        rastro_home,
        _dir: dir,
    }
}

/// A `judgment.request` params object; `session` is the retry-turn object when present.
fn request(session: Option<Value>) -> Value {
    let mut params = json!({
        "instructions": "Find commitments.",
        "owner_speaker": "Alice",
        "turns": [{"turn": 1, "speaker": "Alice", "text": "I'll send it."}],
        "fields": ["turn", "kind", "quote"]
    });
    if let Some(session) = session {
        params["session"] = session;
    }
    params
}

/// The JSONL of a completed turn: a thread id and one structured agent message.
fn turn_output(thread_id: &str, candidates: &str) -> String {
    format!(
        concat!(
            "{{\"type\":\"thread.started\",\"thread_id\":\"{thread_id}\"}}\n",
            "{{\"type\":\"turn.started\"}}\n",
            "{{\"type\":\"item.completed\",\"item\":{{\"id\":\"item_1\",\"type\":\"agent_message\",\"text\":{candidates}}}}}\n",
            "{{\"type\":\"turn.completed\",\"usage\":{{\"input_tokens\":10,\"cached_input_tokens\":0,\"output_tokens\":5,\"reasoning_output_tokens\":0}}}}\n"
        ),
        thread_id = thread_id,
        candidates = serde_json::to_string(candidates).unwrap()
    )
}

/// The retry-turn session object: turn 2, one quote-gate failure, the prior answer.
fn retry_session() -> Value {
    json!({
        "turn": 2,
        "feedback": [{
            "gate": "quote",
            "field": "quote",
            "expected": "an exact, contiguous piece of turn 1",
            "actual": "I will send it tomorrow"
        }],
        "prior_answer": {"candidates": [{
            "turn": 1, "kind": "promise", "promise": "I will send it tomorrow",
            "quote": "I will send it tomorrow"
        }]}
    })
}

/// The lockdown kit every turn must carry: tools off, no MCP servers, no setting sources.
fn assert_lockdown(args: &[String]) {
    for flag in [
        "--json",
        "--skip-git-repo-check",
        "--ignore-user-config",
        "--ignore-rules",
    ] {
        assert!(args.iter().any(|a| a == flag), "{flag} missing: {args:?}");
    }
    for tool in [
        "shell_tool",
        "view_image",
        "apps",
        "browser_use",
        "multi_agent",
        "remote_plugin",
        "hooks",
        "sleep_tool",
        "tool_suggest",
    ] {
        assert!(
            args.windows(2).any(|w| w[0] == "--disable" && w[1] == tool),
            "--disable {tool} missing: {args:?}"
        );
    }
    for setting in [
        "sandbox_mode=\"read-only\"",
        "web_search=\"disabled\"",
        "mcp_servers={}",
        "plugins._default.enabled=false",
        "agents.enabled=false",
        "project_doc_max_bytes=0",
    ] {
        assert!(
            args.windows(2).any(|w| w[0] == "-c" && w[1] == setting),
            "-c {setting} missing: {args:?}"
        );
    }
    assert!(
        args.iter().any(|a| a == "--output-schema"),
        "--output-schema missing: {args:?}"
    );
}

// spec: The core calls the adapter and keeps only the candidates that pass its checks
#[test]
fn a_one_shot_request_drives_codex_under_lockdown_and_relays_the_answer() {
    let answer = r#"{"candidates":[{"turn":1,"kind":"promise","quote":"I'll send it."}]}"#;
    let conv = converse(
        &turn_output("11111111-1111-1111-1111-111111111111", answer),
        Resume::Answer(turn_output("unused", answer)),
        &[request(None)],
    );
    assert_eq!(conv.initialized["id"], 1);
    assert_eq!(
        conv.initialized["result"],
        json!({"protocol": PROTOCOL_VERSION, "manifest_id": "rastro-adapter-codex"})
    );
    assert_eq!(conv.replies.len(), 1);
    assert_eq!(conv.replies[0]["id"], 2);
    assert_eq!(
        conv.replies[0]["result"],
        json!({
            "candidates": [{"turn": 1, "kind": "promise", "quote": "I'll send it."}],
            "session_id": "11111111-1111-1111-1111-111111111111"
        })
    );
    assert_eq!(conv.turns.len(), 1);
    let turn = &conv.turns[0];
    assert_eq!(turn.args[0], "exec");
    assert!(!turn.args.iter().any(|a| a == "resume"), "{:?}", turn.args);
    assert_lockdown(&turn.args);
    // The prompt is the instructions, then the turns as data.
    assert!(turn.prompt.contains("Find commitments."), "{}", turn.prompt);
    let (_, data) = turn.prompt.split_once("\n\nData:\n").unwrap();
    let data: Value = serde_json::from_str(data.trim()).unwrap();
    assert_eq!(data["owner_speaker"], "Alice");
    assert_eq!(data["turns"][0]["text"], "I'll send it.");
    // The schema file asks for exactly the requested fields.
    let item = &turn.schema["properties"]["candidates"]["items"];
    assert_eq!(item["required"], json!(["turn", "kind", "quote"]));
    assert_eq!(item["additionalProperties"], false);
    // Codex runs in a clean working root, and its home is relocated under RASTRO_HOME.
    let work = turn
        .args
        .windows(2)
        .find(|w| w[0] == "-C")
        .map(|w| PathBuf::from(&w[1]))
        .expect("-C missing");
    assert!(work.is_dir(), "{}", work.display());
    assert_eq!(fs::read_dir(&work).unwrap().count(), 0);
    assert_eq!(work, conv.rastro_home.join("data/adapter-codex/work"));
    // The codex process itself runs in the clean working root (pwd resolves symlinks).
    assert_eq!(
        fs::canonicalize(&turn.cwd).unwrap(),
        fs::canonicalize(&work).unwrap()
    );
    assert_eq!(
        turn.codex_home,
        conv.rastro_home
            .join("data/adapter-codex/home")
            .display()
            .to_string()
    );
}

// spec: The core calls the adapter and keeps only the candidates that pass its checks
#[test]
fn a_retry_turn_resumes_the_harness_session_in_a_fresh_process() {
    let thread = "22222222-2222-2222-2222-222222222222";
    let first = r#"{"candidates":[{"turn":1,"kind":"promise","quote":"I will send it tomorrow"}]}"#;
    let corrected = r#"{"candidates":[{"turn":1,"kind":"promise","quote":"I'll send it."}]}"#;
    let conv = converse(
        &turn_output(thread, first),
        Resume::Answer(turn_output(thread, corrected)),
        &[request(None), request(Some(retry_session()))],
    );
    assert_eq!(conv.replies.len(), 2);
    assert_eq!(conv.replies[0]["result"]["session_id"], thread);
    // The retry answer is the corrected one, in the same harness thread, not degraded.
    assert_eq!(
        conv.replies[1]["result"],
        json!({
            "candidates": [{"turn": 1, "kind": "promise", "quote": "I'll send it."}],
            "session_id": thread
        })
    );
    // Two fresh processes: the second resumes the thread the first produced.
    assert_eq!(conv.turns.len(), 2);
    let retry = &conv.turns[1];
    assert_eq!(retry.args[0], "exec");
    assert_eq!(retry.args[1], "resume");
    assert_eq!(retry.args[2], thread);
    assert_lockdown(&retry.args);
    // `codex exec resume` rejects `-C`; the adapter runs it in the clean working root
    // through the process cwd, so no project document reaches the retry turn.
    assert!(
        !retry.args.windows(2).any(|w| w[0] == "-C"),
        "resume rejects -C: {:?}",
        retry.args
    );
    let work = conv.rastro_home.join("data/adapter-codex/work");
    assert_eq!(
        fs::canonicalize(&retry.cwd).unwrap(),
        fs::canonicalize(&work).unwrap()
    );
    // The retry prompt carries the instructions, the data, and the session context: the
    // core's gate feedback and the prior answer being corrected.
    assert!(
        retry.prompt.contains("Find commitments."),
        "{}",
        retry.prompt
    );
    let session = retry.prompt.lines().last().unwrap();
    let session: Value = serde_json::from_str(session).unwrap();
    assert_eq!(session["turn"], 2);
    assert_eq!(session["feedback"][0]["gate"], "quote");
    assert_eq!(
        session["feedback"][0]["expected"],
        "an exact, contiguous piece of turn 1"
    );
    assert_eq!(
        session["prior_answer"]["candidates"][0]["quote"],
        "I will send it tomorrow"
    );
}

// spec: The core calls the adapter and keeps only the candidates that pass its checks
#[test]
fn an_unusable_resume_degrades_to_a_fresh_turn_recorded() {
    let thread = "33333333-3333-3333-3333-333333333333";
    let answer = r#"{"candidates":[{"turn":1,"kind":"promise","quote":"I'll send it."}]}"#;
    let conv = converse(
        &turn_output(thread, answer),
        Resume::Fail("session not found".into()),
        &[request(None), request(Some(retry_session()))],
    );
    assert_eq!(conv.replies.len(), 2);
    // The retry answer comes from a fresh turn, and the degradation is recorded.
    assert_eq!(
        conv.replies[1]["result"],
        json!({
            "candidates": [{"turn": 1, "kind": "promise", "quote": "I'll send it."}],
            "session_id": thread,
            "degraded": true
        })
    );
    // Three processes: the first turn, the failed resume, the degraded fresh turn.
    assert_eq!(conv.turns.len(), 3);
    assert_eq!(conv.turns[1].args[1], "resume");
    let degraded = &conv.turns[2];
    assert_eq!(degraded.args[0], "exec");
    assert!(
        !degraded.args.iter().any(|a| a == "resume"),
        "{:?}",
        degraded.args
    );
    assert_lockdown(&degraded.args);
    // The fresh turn carries the prior answer and the failure reason.
    let session = degraded.prompt.lines().last().unwrap();
    let session: Value = serde_json::from_str(session).unwrap();
    assert_eq!(session["turn"], 2);
    assert_eq!(session["feedback"][0]["gate"], "quote");
    assert_eq!(
        session["prior_answer"]["candidates"][0]["quote"],
        "I will send it tomorrow"
    );
}

// spec: The core calls the adapter and keeps only the candidates that pass its checks
#[test]
fn a_retry_without_a_harness_session_id_degrades_recorded() {
    // A harness turn that completes without producing a thread id: nothing to resume.
    let answer = r#"{"candidates":[{"turn":1,"kind":"promise","quote":"I'll send it."}]}"#;
    let message = serde_json::to_string(answer).unwrap();
    let fresh = format!(
        concat!(
            "{{\"type\":\"turn.started\"}}\n",
            "{{\"type\":\"item.completed\",\"item\":{{\"id\":\"item_1\",\"type\":\"agent_message\",\"text\":{message}}}}}\n",
            "{{\"type\":\"turn.completed\",\"usage\":{{\"input_tokens\":10,\"cached_input_tokens\":0,\"output_tokens\":5,\"reasoning_output_tokens\":0}}}}\n"
        ),
        message = message
    );
    let conv = converse(
        &fresh,
        Resume::Fail("resume must not be attempted".into()),
        &[request(None), request(Some(retry_session()))],
    );
    assert_eq!(conv.replies.len(), 2);
    assert!(conv.replies[0]["result"].get("session_id").is_none());
    assert_eq!(
        conv.replies[1]["result"],
        json!({
            "candidates": [{"turn": 1, "kind": "promise", "quote": "I'll send it."}],
            "degraded": true
        })
    );
    // Two fresh processes; no resume was attempted.
    assert_eq!(conv.turns.len(), 2);
    assert!(
        !conv.turns[1].args.iter().any(|a| a == "resume"),
        "{:?}",
        conv.turns[1].args
    );
    assert_lockdown(&conv.turns[1].args);
    let session = conv.turns[1].prompt.lines().last().unwrap();
    let session: Value = serde_json::from_str(session).unwrap();
    assert_eq!(session["feedback"][0]["gate"], "quote");
    assert_eq!(
        session["prior_answer"]["candidates"][0]["quote"],
        "I will send it tomorrow"
    );
}

// spec: The core calls the adapter and keeps only the candidates that pass its checks
#[test]
fn a_codex_failure_is_an_error_response_not_a_result() {
    let failed = concat!(
        "{\"type\":\"thread.started\",\"thread_id\":\"t-1\"}\n",
        "{\"type\":\"turn.started\"}\n",
        "{\"type\":\"turn.failed\",\"error\":{\"message\":\"overloaded\"}}\n"
    );
    let conv = converse(failed, Resume::Fail("unused".into()), &[request(None)]);
    assert_eq!(conv.replies.len(), 1);
    assert!(
        conv.replies[0].get("result").is_none(),
        "{}",
        conv.replies[0]
    );
    assert!(
        conv.replies[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("overloaded"),
        "{}",
        conv.replies[0]
    );
}
