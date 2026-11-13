//! The reference adapter over the plugin protocol, with a script standing in for `claude`, so no
//! model is called.
use rastro_plugin_sdk::{Manifest, PROTOCOL_VERSION};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
};

#[test]
fn the_manifest_declares_the_kind_and_capabilities_and_names_no_tool() {
    let manifest = Manifest::parse(include_str!("../rastro-plugin.toml")).unwrap();
    manifest.check_agent_tool().unwrap();
    assert_eq!(manifest.kinds, ["agent-tool-adapter"]);
    assert!(manifest.permissions.actions.is_empty());
}

/// Runs the adapter against a fake `claude` that saves its arguments and prompt and prints
/// `output`; returns the replies to `initialize` and `judgment.request` and what claude got.
fn converse(output: &str) -> (Value, Value, Vec<String>, String) {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("claude");
    fs::write(
        &fake,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{0}/args'\ncat > '{0}/prompt'\ncat <<'EOF'\n{output}\nEOF\n",
            dir.path().display()
        ),
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rastro-adapter-claude-code"))
        .env("RASTRO_CLAUDE_COMMAND", &fake)
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
        "protocol": PROTOCOL_VERSION, "manifest_id": "rastro-adapter-claude-code",
        "role": "agent_tool_adapter"}}),
    );
    let judged = ask(
        json!({"jsonrpc": "2.0", "id": 2, "method": "judgment.request", "params": {
        "instructions": "Find commitments.", "owner_speaker": "Alice",
        "turns": [{"turn": 1, "speaker": "Alice", "text": "I'll send it."}],
        "fields": ["turn", "kind", "quote"]}}),
    );
    drop(stdin);
    child.wait().unwrap();
    let args = fs::read_to_string(dir.path().join("args"))
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    let prompt = fs::read_to_string(dir.path().join("prompt")).unwrap();
    (initialized, judged, args, prompt)
}

#[test]
fn it_relays_what_claude_returns_with_every_tool_turned_off() {
    let (initialized, judged, args, prompt) = converse(
        r#"{"is_error":false,"structured_output":{"candidates":[{"turn":1,"kind":"promise","quote":"I'll send it."}]}}"#,
    );
    assert_eq!(initialized["id"], 1);
    assert_eq!(
        initialized["result"],
        json!({"protocol": PROTOCOL_VERSION, "manifest_id": "rastro-adapter-claude-code"})
    );
    assert_eq!(judged["id"], 2);
    assert_eq!(
        judged["result"],
        json!({"candidates": [{"turn": 1, "kind": "promise", "quote": "I'll send it."}]})
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
    assert!(prompt.starts_with("Find commitments."));
    assert!(prompt.contains("I'll send it."));
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
