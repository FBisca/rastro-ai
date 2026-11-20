//! The reference adapter over the plugin protocol, with a script standing in for `claude`, so no
//! model is called. The fake records every call's arguments, prompt and config dir, numbered
//! from 1, so the turns of a session can be told apart.
use rastro_plugin_sdk::{Manifest, PROTOCOL_VERSION};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Lines, Write},
    os::unix::fs::PermissionsExt,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

// spec: An adapter's manifest declares its kind and names no tool
#[test]
fn the_manifest_declares_the_kind_and_capabilities_and_names_no_tool() {
    let manifest = Manifest::parse(include_str!("../rastro-plugin.toml")).unwrap();
    manifest.check_agent_tool().unwrap();
    assert_eq!(manifest.kinds, ["agent-tool-adapter"]);
    assert!(manifest.permissions.actions.is_empty());
    // The lockdown kit enforces the whole judgment floor, and the manifest says so.
    assert_eq!(
        manifest.enforcement,
        ["tools_off", "no_mcp_servers", "no_setting_sources"]
    );
}

/// An adapter process talking to a fake `claude` that records each call's arguments, prompt and
/// `CLAUDE_CONFIG_DIR` under numbered files and prints `output` every time. The adapter starts
/// with neither `RASTRO_HOME` nor `CLAUDE_CONFIG_DIR` unless `envs` sets them.
struct Session {
    dir: tempfile::TempDir,
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl Session {
    fn start(output: &str, envs: &[(&str, &str)]) -> Session {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("claude");
        fs::write(
            &fake,
            format!(
                "#!/bin/sh\n\
                 n=$(( $(cat '{0}/calls' 2>/dev/null || echo 0) + 1 ))\n\
                 printf '%s' \"$n\" > '{0}/calls'\n\
                 printf '%s\\n' \"$@\" > '{0}/args.'\"$n\"\n\
                 cat > '{0}/prompt.'\"$n\"\n\
                 printf '%s' \"${{CLAUDE_CONFIG_DIR:-}}\" > '{0}/config_dir.'\"$n\"\n\
                 cat <<'EOF'\n{output}\nEOF\n",
                dir.path().display()
            ),
        )
        .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let mut all = vec![("RASTRO_CLAUDE_COMMAND", fake.to_str().unwrap())];
        all.extend(envs.iter().copied());
        Self::spawn(dir, &all)
    }

    /// The real `claude` (or `RASTRO_CLAUDE_COMMAND`), no fake: the live contract run.
    fn start_live(envs: &[(&str, &str)]) -> Session {
        Self::spawn(tempfile::tempdir().unwrap(), envs)
    }

    fn spawn(dir: tempfile::TempDir, envs: &[(&str, &str)]) -> Session {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rastro-adapter-claude-code"))
            .env_remove("RASTRO_HOME")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("ANTHROPIC_API_KEY")
            .envs(envs.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let lines = BufReader::new(child.stdout.take().unwrap()).lines();
        Session {
            dir,
            child,
            stdin: Some(stdin),
            lines,
            next_id: 1,
        }
    }

    fn ask(&mut self, mut message: Value) -> Value {
        message["id"] = json!(self.next_id);
        self.next_id += 1;
        writeln!(self.stdin.as_mut().unwrap(), "{message}").unwrap();
        serde_json::from_str::<Value>(&self.lines.next().unwrap().unwrap()).unwrap()
    }

    fn initialize(&mut self) -> Value {
        self.ask(json!({"jsonrpc": "2.0", "method": "initialize", "params": {
            "protocol": PROTOCOL_VERSION, "manifest_id": "rastro-adapter-claude-code",
            "role": "agent_tool_adapter"}}))
    }

    fn judge(&mut self, params: Value) -> Value {
        self.ask(json!({"jsonrpc": "2.0", "method": "judgment.request", "params": params}))
    }

    fn finish(&mut self) {
        drop(self.stdin.take());
        self.child.wait().unwrap();
    }

    fn args(&self, call: usize) -> Vec<String> {
        fs::read_to_string(self.dir.path().join(format!("args.{call}")))
            .unwrap()
            .lines()
            .map(String::from)
            .collect()
    }

    fn prompt(&self, call: usize) -> String {
        fs::read_to_string(self.dir.path().join(format!("prompt.{call}"))).unwrap()
    }

    fn config_dir(&self, call: usize) -> String {
        fs::read_to_string(self.dir.path().join(format!("config_dir.{call}"))).unwrap()
    }
}

/// The one-shot request the core sends on a first turn: no session object.
fn one_shot_request() -> Value {
    json!({
        "instructions": "Find commitments.", "owner_speaker": "Alice",
        "turns": [{"turn": 1, "speaker": "Alice", "text": "I'll send it."}],
        "fields": ["turn", "kind", "quote"]})
}

/// Runs the adapter against a fake `claude` for one one-shot judgment; returns the replies to
/// `initialize` and `judgment.request` and what claude got.
fn converse(output: &str) -> (Value, Value, Vec<String>, String) {
    let mut session = Session::start(output, &[]);
    let initialized = session.initialize();
    let judged = session.judge(one_shot_request());
    session.finish();
    (initialized, judged, session.args(1), session.prompt(1))
}

// spec: The core calls the adapter and keeps only the candidates that pass its checks
#[test]
fn it_relays_what_claude_returns_with_every_tool_turned_off() {
    let (initialized, judged, args, prompt) = converse(
        r#"{"is_error":false,"session_id":"sess-1","structured_output":{"candidates":[{"turn":1,"kind":"promise","quote":"I'll send it."}]}}"#,
    );
    assert_eq!(initialized["id"], 1);
    assert_eq!(
        initialized["result"],
        json!({"protocol": PROTOCOL_VERSION, "manifest_id": "rastro-adapter-claude-code"})
    );
    assert_eq!(judged["id"], 2);
    assert_eq!(
        judged["result"],
        json!({"candidates": [{"turn": 1, "kind": "promise", "quote": "I'll send it."}], "session_id": "sess-1"})
    );
    // Claude Code runs print-only, with no tool, settings, MCP server or slash command.
    for flag in [
        "-p",
        "--tools",
        "--disable-slash-commands",
        "--strict-mcp-config",
    ] {
        assert!(args.iter().any(|a| a == flag), "{flag} missing: {args:?}");
    }
    let value_of = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .map(|i| args[i + 1].clone())
    };
    assert_eq!(value_of("--tools").as_deref(), Some(""), "{args:?}");
    assert_eq!(
        value_of("--setting-sources").as_deref(),
        Some(""),
        "{args:?}"
    );
    assert!(
        !args.iter().any(|a| a == "--mcp-config"),
        "an MCP config would let servers in: {args:?}"
    );
    assert!(prompt.starts_with("Find commitments."));
    assert!(prompt.contains("I'll send it."));
}

// spec: The core calls the adapter and keeps only the candidates that pass its checks
#[test]
fn a_retry_turn_resumes_the_harness_session_in_a_fresh_process_under_lockdown() {
    let mut session = Session::start(
        r#"{"is_error":false,"session_id":"sess-1","structured_output":{"candidates":[{"turn":1,"kind":"promise","quote":"I'll send it."}]}}"#,
        &[],
    );
    session.initialize();
    let first = session.judge(one_shot_request());
    assert_eq!(first["result"]["session_id"], "sess-1");
    let retry = session.judge(json!({
        "instructions": "Find commitments.", "owner_speaker": "Alice",
        "turns": [{"turn": 1, "speaker": "Alice", "text": "I'll send it."}],
        "fields": ["turn", "kind", "quote"],
        "session": {
            "turn": 2,
            "feedback": [{"gate": "quote", "field": "quote",
                "expected": "an exact, contiguous piece of turn 1",
                "actual": "I'll send it tomorrow"}],
            "prior_answer": {
                "candidates": [{"turn": 1, "kind": "promise", "promise": "send it", "quote": "I'll send it tomorrow"}],
                "session_id": "sess-1"
            }
        }
    }));
    session.finish();
    // A fresh claude process per turn: the fake ran twice, and the retry resumes the id the
    // first turn produced.
    let retry_args = session.args(2);
    let resume = retry_args
        .iter()
        .position(|a| a == "--resume")
        .unwrap_or_else(|| panic!("--resume missing: {retry_args:?}"));
    assert_eq!(retry_args[resume + 1], "sess-1");
    assert!(!session.args(1).iter().any(|a| a == "--resume"));
    // The lockdown kit is re-applied on the retry turn, and persistence stays on so the
    // session remains resumable.
    let value_of = |flag: &str| {
        retry_args
            .iter()
            .position(|a| a == flag)
            .map(|i| retry_args[i + 1].clone())
    };
    assert_eq!(value_of("--tools").as_deref(), Some(""), "{retry_args:?}");
    assert_eq!(
        value_of("--setting-sources").as_deref(),
        Some(""),
        "{retry_args:?}"
    );
    assert!(retry_args.iter().any(|a| a == "--strict-mcp-config"));
    assert!(retry_args.iter().any(|a| a == "--disable-slash-commands"));
    assert!(
        !retry_args.iter().any(|a| a == "--no-session-persistence"),
        "a persisted session must stay resumable: {retry_args:?}"
    );
    // The core's gate feedback and the prior answer ride in the retry prompt as data.
    let prompt = session.prompt(2);
    assert!(
        prompt.contains("an exact, contiguous piece of turn 1"),
        "{prompt}"
    );
    assert!(prompt.contains("I'll send it tomorrow"), "{prompt}");
    // The corrected answer is relayed.
    assert_eq!(
        retry["result"]["candidates"],
        json!([{"turn": 1, "kind": "promise", "quote": "I'll send it."}])
    );
}

// spec: The core calls the adapter and keeps only the candidates that pass its checks
#[test]
fn a_retry_without_a_usable_resume_runs_a_fresh_turn_and_records_the_degradation() {
    // The harness keeps no session id, so there is nothing to resume.
    let mut session = Session::start(
        r#"{"is_error":false,"structured_output":{"candidates":[{"turn":1,"kind":"promise","quote":"I'll send it."}]}}"#,
        &[],
    );
    session.initialize();
    let first = session.judge(one_shot_request());
    assert!(first["result"].get("session_id").is_none(), "{first}");
    let retry = session.judge(json!({
        "instructions": "Find commitments.", "owner_speaker": "Alice",
        "turns": [{"turn": 1, "speaker": "Alice", "text": "I'll send it."}],
        "fields": ["turn", "kind", "quote"],
        "session": {
            "turn": 2,
            "feedback": [{"gate": "quote", "field": "quote",
                "expected": "an exact, contiguous piece of turn 1",
                "actual": "I'll send it tomorrow"}],
            "prior_answer": {
                "candidates": [{"turn": 1, "kind": "promise", "promise": "send it", "quote": "I'll send it tomorrow"}]
            }
        }
    }));
    session.finish();
    // A fresh turn: no resume flag, the lockdown kit still on.
    let retry_args = session.args(2);
    assert!(
        !retry_args.iter().any(|a| a == "--resume"),
        "nothing to resume: {retry_args:?}"
    );
    assert!(retry_args.iter().any(|a| a == "--strict-mcp-config"));
    assert!(retry_args.iter().any(|a| a == "--disable-slash-commands"));
    // The fresh turn carries the prior answer and the failure reason, since the harness holds
    // no context of its own.
    let prompt = session.prompt(2);
    assert!(prompt.contains("I'll send it tomorrow"), "{prompt}");
    assert!(
        prompt.contains("an exact, contiguous piece of turn 1"),
        "{prompt}"
    );
    // The degradation is recorded, never silent, and the answer is relayed.
    assert_eq!(retry["result"]["degraded"], true, "{retry}");
    assert_eq!(
        retry["result"]["candidates"],
        json!([{"turn": 1, "kind": "promise", "quote": "I'll send it."}])
    );
}

// ADR-0016 §7: the harness session store relocates under RASTRO_HOME when the run can still
// authenticate there (an API key survives the move; claude's file-based sign-in does not), so
// its untrusted transcripts stay on Rastro's retention surface. Untagged until the session
// requirement lands in a spec at archive; the fake-harness boundary is where the child's env is
// observable.
#[test]
fn the_harness_config_dir_relocates_under_rastro_home_when_the_run_can_authenticate_there() {
    let output = r#"{"is_error":false,"structured_output":{"candidates":[]}}"#;
    let home = tempfile::tempdir().unwrap();
    let home_env = ("RASTRO_HOME", home.path().to_str().unwrap());
    // An API key authenticates whatever the config dir: the store moves under RASTRO_HOME,
    // ready for claude.
    let mut session = Session::start(output, &[home_env, ("ANTHROPIC_API_KEY", "sk-test")]);
    session.initialize();
    session.judge(one_shot_request());
    session.finish();
    let relocated = home.path().join("claude");
    assert_eq!(
        session.config_dir(1),
        relocated.display().to_string(),
        "the session store must land under RASTRO_HOME"
    );
    assert!(relocated.is_dir(), "{} must exist", relocated.display());
    // An explicit CLAUDE_CONFIG_DIR is the owner's choice and passes through untouched.
    let owner_dir = tempfile::tempdir().unwrap();
    let mut session = Session::start(
        output,
        &[
            home_env,
            ("ANTHROPIC_API_KEY", "sk-test"),
            ("CLAUDE_CONFIG_DIR", owner_dir.path().to_str().unwrap()),
        ],
    );
    session.initialize();
    session.judge(one_shot_request());
    session.finish();
    assert_eq!(
        session.config_dir(1),
        owner_dir.path().display().to_string()
    );
    // Without an API key, relocation would move claude's sign-in away and break the run: the
    // store stays in the owner's home, the disclosed residual of ADR-0016 §7.
    let unkeyed_home = tempfile::tempdir().unwrap();
    let mut session = Session::start(
        output,
        &[("RASTRO_HOME", unkeyed_home.path().to_str().unwrap())],
    );
    session.initialize();
    session.judge(one_shot_request());
    session.finish();
    assert_eq!(session.config_dir(1), "");
    assert!(
        !unkeyed_home.path().join("claude").exists(),
        "no relocation without an API key"
    );
    // Neither home nor key: claude keeps its default; the adapter forces nothing.
    let mut session = Session::start(output, &[]);
    session.initialize();
    session.judge(one_shot_request());
    session.finish();
    assert_eq!(session.config_dir(1), "");
}

// ADR-0016 §4 and §7, the live contract proofs (plan decision D15): run once inside the driver
// ticket, recorded in its report; CI keeps the fake-harness tests above. Untagged until the
// session requirement lands in a spec at archive. Both need a signed-in claude (the owner's
// config dir); the resume test spends two model calls, the relocation-fallback test one.
#[test]
#[ignore = "live: drives the real claude CLI, two model calls, needs a signed-in claude; reads RASTRO_CLAUDE_COMMAND, RASTRO_CLAUDE_MODEL and ANTHROPIC_API_KEY; clears RASTRO_HOME and CLAUDE_CONFIG_DIR"]
fn live_claude_resumes_a_session_under_lockdown() {
    let mut session = Session::start_live(&[]);
    session.initialize();
    let first = session.judge(json!({
        "instructions": "Memory test: remember the codeword the data states. Answer with an empty candidates list.",
        "owner_speaker": "Alice",
        "turns": [{"turn": 1, "speaker": "Alice", "text": "The codeword is lantern."}],
        "fields": ["turn", "kind", "quote"]
    }));
    let session_id = first["result"]["session_id"]
        .as_str()
        .unwrap_or_else(|| panic!("turn 1 produced no session id: {first}"));
    println!("turn 1 session id: {session_id}");
    let retry = session.judge(json!({
        "instructions": "Memory test: answer with one candidate whose quote is the codeword you remember, exactly as stated.",
        "owner_speaker": "Alice",
        "turns": [{"turn": 1, "speaker": "Alice", "text": "The codeword is lantern."}],
        "fields": ["turn", "kind", "quote"],
        "session": {"turn": 2, "feedback": [], "prior_answer": {"candidates": [], "session_id": session_id}}
    }));
    session.finish();
    assert!(
        retry["result"].get("degraded").is_none(),
        "the resume must be usable: {retry}"
    );
    let quotes: Vec<&str> = retry["result"]["candidates"]
        .as_array()
        .unwrap_or_else(|| panic!("turn 2 answered no candidates: {retry}"))
        .iter()
        .map(|c| c["quote"].as_str().unwrap_or(""))
        .collect();
    println!("turn 2 quotes: {quotes:?}");
    assert!(
        quotes.iter().any(|q| q.contains("lantern")),
        "the resumed session must remember the codeword: {retry}"
    );
}

// The live proof of the relocation fallback: claude's file-based sign-in lives in its config
// dir, so without an API key the adapter must leave the store in the owner's home (disclosed
// residual) and the run must still authenticate and answer.
#[test]
#[ignore = "live: drives the real claude CLI, one model call, needs a signed-in claude and no ANTHROPIC_API_KEY; reads RASTRO_CLAUDE_COMMAND and RASTRO_CLAUDE_MODEL; sets RASTRO_HOME to a temp dir and clears CLAUDE_CONFIG_DIR"]
fn live_without_an_api_key_the_session_store_stays_in_the_owners_home() {
    assert!(
        std::env::var_os("ANTHROPIC_API_KEY").is_none_or(|key| key.is_empty()),
        "this proof needs claude's own sign-in, not an API key"
    );
    let home = tempfile::tempdir().unwrap();
    let mut session = Session::start_live(&[("RASTRO_HOME", home.path().to_str().unwrap())]);
    session.initialize();
    let first = session.judge(json!({
        "instructions": "Answer with an empty candidates list.",
        "owner_speaker": "Alice",
        "turns": [{"turn": 1, "speaker": "Alice", "text": "Hello."}],
        "fields": ["turn", "kind", "quote"]
    }));
    session.finish();
    let session_id = first["result"]["session_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the run must still authenticate: {first}"));
    println!("session id: {session_id}");
    assert!(
        !home.path().join("claude").exists(),
        "without an API key the store must stay in the owner's home"
    );
    println!("disclosed residual: the session store stays in the owner's claude config dir");
}

#[test]
fn a_claude_failure_is_an_error_response_not_a_result() {
    let (_, judged, _, _) = converse(r#"{"is_error":true,"result":"overloaded"}"#);
    assert!(judged.get("result").is_none(), "{judged}");
    assert!(
        judged["error"]["message"]
            .as_str()
            .unwrap()
            .contains("overloaded"),
        "{judged}"
    );
}
