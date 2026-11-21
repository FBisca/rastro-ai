#![allow(non_snake_case)]
//! A recorded scripted session validates against the committed plugin protocol schema.
mod support;

use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
use tempfile::TempDir;

/// The committed schema, as one validator for every recorded line.
fn validator() -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(include_str!(
        "../../../schema/plugin-protocol-v1.schema.json"
    ))
    .unwrap();
    jsonschema::validator_for(&schema).unwrap()
}

/// A home with the test connector installed behind a recorder, and one source bound to it.
struct Session {
    home: TempDir,
}
impl Session {
    fn new() -> Self {
        let home = TempDir::new().unwrap();
        let plugin = home.path().join("dev-plugins/test");
        fs::create_dir_all(&plugin).unwrap();
        let mut manifest: toml::Table =
            include_str!("../../rastro-test-connector/rastro-plugin.toml")
                .parse()
                .unwrap();
        // The host launches the manifest's command directly; a recorder wraps the connector and
        // tees both directions of the wire to files the test reads back.
        manifest.insert("command".into(), "record.sh".into());
        fs::write(plugin.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
        let recorder = format!(
            "#!/bin/sh\ntee '{}' | '{}' | tee '{}'\n",
            home.path().join("host-to-connector.jsonl").display(),
            support::test_connector().display(),
            home.path().join("connector-to-host.jsonl").display(),
        );
        let script = plugin.join("record.sh");
        fs::write(&script, recorder).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let session = Self { home };
        session.ok(&[
            "source",
            "add",
            "project",
            "--connector",
            "test",
            "--repo",
            "o/p",
            "--scope",
            "personal",
        ]);
        session
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(support::rastro());
        command
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.home.path().join("dev-plugins"))
            .env("RASTRO_TEST_SCRIPT", self.home.path().join("script.jsonl"))
            .args(args);
        command
    }
    fn ok(&self, args: &[&str]) -> String {
        let output = self.command(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn script(&self, steps: &[Value]) {
        let lines: Vec<_> = steps.iter().map(Value::to_string).collect();
        fs::write(self.home.path().join("script.jsonl"), lines.join("\n")).unwrap();
    }
    /// Every line the host and the connector wrote, in both directions.
    fn recorded(&self) -> Vec<Value> {
        let mut lines = Vec::new();
        for name in ["host-to-connector.jsonl", "connector-to-host.jsonl"] {
            let text = fs::read_to_string(self.home.path().join(name)).unwrap();
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                lines.push(serde_json::from_str(line).unwrap());
            }
        }
        lines
    }
}

fn send(message: Value) -> Value {
    json!({"do": "send", "message": message})
}

/// spec: The generated plugin protocol schema equals the committed file
/// Every message a scripted connector exchanges with the host in a sync validates against the
/// committed schema.
#[test]
fn a_recorded_scripted_session_validates_against_the_schema() {
    let session = Session::new();
    session.script(&[
        json!({"do": "initialize"}),
        send(json!({"jsonrpc": "2.0", "method": "evidence.submit", "params": {
            "kind": "issue", "location": "https://github.com/o/p/issues/1", "field": "/body",
            "start": 0, "end": 5, "quote": "hello", "trust_class": "untrusted content",
            "source_updated_at": "2026-11-01T00:00:00Z", "original": {"body": "hello"}}})),
        // A minimal submission: optional fields absent, and an explicit null original.
        send(json!({"jsonrpc": "2.0", "method": "evidence.submit", "params": {
            "kind": "comment", "location": "https://github.com/o/p/issues/1#issuecomment-2",
            "field": "/body", "start": 0, "end": 2, "quote": "hi",
            "trust_class": "untrusted content", "original": null}})),
        send(json!({"jsonrpc": "2.0", "method": "coverage.report", "params": {"status": "complete"}})),
        json!({"do": "finish", "submitted": 2}),
    ]);
    let output = session
        .command(&["sync", "project", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let run = &value["result"]["runs"][0];
    assert_eq!(run["status"], "ok", "{run}");
    assert_eq!(run["counts"]["accepted"], 2, "{run}");

    let validator = validator();
    let messages = session.recorded();
    // The handshake, both submissions, the coverage report and the finish exchange were recorded.
    assert!(messages.len() >= 7, "recorded too little: {messages:#?}");
    let failures: Vec<String> = messages
        .iter()
        .enumerate()
        .filter_map(|(i, message)| {
            validator
                .validate(message)
                .err()
                .map(|error| format!("line {i}: {error}: {message}"))
        })
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
}
