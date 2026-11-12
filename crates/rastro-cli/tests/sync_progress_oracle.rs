//! Oracle for plan `answers-i-can-read` (sync progress and the rename warning): black-box checks
//! of S13, S16, S17 and S18 through the `rastro` binary and the scripted test connector.
//!
//! The connector writes a marker line to its stderr as soon as it starts, which the host relays
//! to `rastro`'s stderr: anything printed before the marker was printed before the read.
#![allow(non_snake_case)]
mod support;
use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Output, Stdio},
};
use tempfile::TempDir;

const WINDOW_START: &str = "2026-01-01T00:00:00.000Z";
const MARKER: &str = "oracle-connector-started";

struct Env {
    home: TempDir,
}
impl Env {
    fn new() -> Self {
        let env = Self {
            home: TempDir::new().unwrap(),
        };
        let plugin = env.home.path().join("dev-plugins/test");
        fs::create_dir_all(&plugin).unwrap();
        let mut manifest: toml::Table =
            include_str!("../../rastro-test-connector/rastro-plugin.toml")
                .parse()
                .unwrap();
        manifest.insert(
            "command".into(),
            support::test_connector().to_str().unwrap().into(),
        );
        manifest["permissions"]["scopes"] =
            toml::Value::Array(["personal", "work"].into_iter().map(|s| s.into()).collect());
        fs::write(plugin.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
        env
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
    fn rastro(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }
    fn ok(&self, args: &[&str]) -> Output {
        let output = self.rastro(args);
        assert!(
            output.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
    /// The `--json` envelope of a command, whatever its exit code, checked against the schema.
    fn json(&self, args: &[&str]) -> (Output, Value) {
        let output = self.rastro(&[args, &["--json"]].concat());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
            panic!(
                "{args:?}: not one JSON value ({e}): {}",
                String::from_utf8_lossy(&output.stdout)
            )
        });
        let schema: Value =
            serde_json::from_str(include_str!("../../../schema/operations-v2.schema.json"))
                .unwrap();
        jsonschema::validator_for(&schema)
            .unwrap()
            .validate(&value)
            .unwrap_or_else(|e| panic!("{e}: {value}"));
        (output, value)
    }
    fn add_source(&self, name: &str, scope: &str) {
        self.ok(&[
            "source",
            "add",
            name,
            "--connector",
            "test",
            "--repo",
            "o/p",
            "--scope",
            scope,
            "--window-start",
            WINDOW_START,
        ]);
    }
    fn script(&self, steps: &[Value]) {
        let mut all = vec![
            json!({"do": "stderr", "text": MARKER}),
            json!({"do": "initialize"}),
        ];
        all.extend_from_slice(steps);
        let lines: Vec<_> = all.iter().map(Value::to_string).collect();
        fs::write(self.home.path().join("script.jsonl"), lines.join("\n")).unwrap();
    }
    /// The connector submits one body per `(location, quote)` and reports complete coverage.
    fn bodies(&self, bodies: &[(&str, &str)]) {
        let mut steps: Vec<Value> = bodies
            .iter()
            .map(|(location, quote)| json!({"do": "send", "message": submit(location, quote)}))
            .collect();
        steps.push(json!({"do": "send", "message": complete()}));
        steps.push(json!({"do": "finish", "status": "ok", "submitted": bodies.len()}));
        self.script(&steps);
    }
    fn config(&self) -> std::path::PathBuf {
        self.home.path().join("config/config.toml")
    }
}

fn submit(location: &str, quote: &str) -> Value {
    json!({"jsonrpc": "2.0", "method": "evidence.submit", "params": {
        "kind": "issue", "location": location, "field": "/body", "start": 0,
        "end": quote.len(), "quote": quote, "trust_class": "untrusted content",
        "source_updated_at": "2026-11-01T00:00:00Z"}})
}
fn complete() -> Value {
    json!({"jsonrpc": "2.0", "method": "coverage.report",
        "params": {"status": "complete", "selection": "all"}})
}
fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
/// Index of the first stderr line satisfying `test`, with the whole stderr on failure.
fn line_where(stderr: &str, what: &str, test: impl Fn(&str) -> bool) -> usize {
    stderr
        .lines()
        .position(test)
        .unwrap_or_else(|| panic!("no stderr line {what}:\n{stderr}"))
}
fn marker(stderr: &str) -> usize {
    line_where(stderr, "with the connector's start marker", |l| {
        l.contains(MARKER)
    })
}
/// The integer written right next to `word` (`submitted 2`, `2 submitted`, `submitted: 2`).
fn count_next_to(line: &str, word: &str) -> Option<u64> {
    let tokens: Vec<String> = line
        .split(|c: char| c.is_whitespace() || ",;:=|()".contains(c))
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect();
    let at = tokens.iter().position(|t| t == word)?;
    let number = |i: usize| tokens.get(i).and_then(|t| t.parse().ok());
    number(at + 1).or_else(|| at.checked_sub(1).and_then(number))
}

/// S13: before the read, stderr names the source and why it reads what it reads: first read,
/// incremental, re-read
#[test]
fn S13_stderr_names_the_source_and_its_read_reason_before_the_read() {
    let env = Env::new();
    env.add_source("project", "personal");
    let start_date = &WINDOW_START[..10];

    env.bodies(&[("https://x/1", "first body")]);
    let first = stderr(&env.ok(&["sync", "project"]));
    let line = line_where(&first, "naming `project` and `first read from`", |l| {
        l.contains("project") && l.contains("first read from")
    });
    let text = first.lines().nth(line).unwrap();
    assert!(text.contains(start_date), "not from window_start: {text}");
    assert!(
        line < marker(&first),
        "the reason came after the read began:\n{first}"
    );

    let (_, coverage) = env.json(&["coverage", "project"]);
    let window_end = coverage["result"]["coverage"][0]["windows"][0]["to"]
        .as_str()
        .unwrap()
        .to_string();
    env.bodies(&[("https://x/1", "first body")]);
    let second = stderr(&env.ok(&["sync", "project"]));
    let line = line_where(&second, "naming `project` and `incremental since`", |l| {
        l.contains("project") && l.contains("incremental since")
    });
    let text = second.lines().nth(line).unwrap();
    assert!(
        text.contains(&window_end[..10]),
        "not since the last window end {window_end}: {text}"
    );
    assert!(
        line < marker(&second),
        "the reason came after the read began:\n{second}"
    );

    env.bodies(&[("https://x/1", "first body")]);
    let reread = stderr(&env.ok(&["sync", "project", "--reread"]));
    let line = line_where(&reread, "naming `project` and `re-read from`", |l| {
        l.contains("project") && l.contains("re-read from")
    });
    let text = reread.lines().nth(line).unwrap();
    assert!(text.contains(start_date), "not from window_start: {text}");
    assert!(
        line < marker(&reread),
        "the reason came after the read began:\n{reread}"
    );
}

/// S16: stderr ends each source with its counts; a rate-limited run gives its retry time
#[test]
fn S16_stderr_result_line_gives_counts_and_the_rate_limit_retry_time() {
    let env = Env::new();
    env.add_source("project", "personal");
    env.bodies(&[
        ("https://x/1", "first body"),
        ("https://x/2", "second body"),
    ]);
    env.ok(&["sync", "project"]);
    env.bodies(&[
        ("https://x/1", "first body"),
        ("https://x/2", "changed body"),
    ]);
    let err = stderr(&env.ok(&["sync", "project"]));
    let line = line_where(&err, "with submitted, accepted and unchanged counts", |l| {
        let l = l.to_lowercase();
        l.contains("submitted") && l.contains("accepted") && l.contains("unchanged")
    });
    let text = err.lines().nth(line).unwrap();
    assert_eq!(count_next_to(text, "submitted"), Some(2), "{text}");
    assert_eq!(count_next_to(text, "accepted"), Some(1), "{text}");
    assert_eq!(count_next_to(text, "unchanged"), Some(1), "{text}");

    let retry = "2030-01-01T00:00:00Z";
    env.script(&[
        json!({"do": "send", "message": {"jsonrpc": "2.0", "method": "coverage.report",
            "params": {"status": "couldnt_look", "reason": "rate_limited",
            "retry_at": retry, "selection": "all"}}}),
        json!({"do": "finish", "status": "ok", "submitted": 0}),
    ]);
    let output = env.rastro(&["sync", "project"]);
    assert!(
        !output.status.success(),
        "a rate-limited run fails, as before"
    );
    let err = stderr(&output);
    let line = line_where(&err, "with `retry at`", |l| {
        l.to_lowercase().contains("retry at")
    });
    let text = err.lines().nth(line).unwrap().to_lowercase();
    assert!(
        text.contains("rate") && text.contains("limited") && text.contains("2030-01-01"),
        "not the rate-limit retry time: {text}"
    );
}

/// The rename: `old` is synced (personal), then renamed in config to `new` in another scope,
/// with the same connector and selector. Returns the env, ready to sync `new`.
fn renamed(old: &str, new: &str) -> Env {
    let env = Env::new();
    env.add_source(old, "personal");
    env.bodies(&[
        ("https://x/1", "first body"),
        ("https://x/2", "second body"),
    ]);
    env.ok(&["sync", old]);
    let config = fs::read_to_string(env.config()).unwrap();
    let edited = config
        .replace(&format!("name = \"{old}\""), &format!("name = \"{new}\""))
        .replace("scope = \"personal\"", "scope = \"work\"");
    assert_ne!(config, edited, "the config edit changed nothing: {config}");
    fs::write(env.config(), edited).unwrap();
    env.bodies(&[
        ("https://x/1", "first body"),
        ("https://x/2", "second body"),
    ]);
    env
}

/// S18: a configured source with no complete check, and a stored source no longer in config
/// with the same connector and selector, in another scope: stderr warns before the read naming
/// both, the result's `warnings` lists it, and the read still runs from `window_start`
#[test]
fn S18_a_renamed_source_is_warned_about_before_its_full_read() {
    let (old, new) = ("alpha-src", "beta-src");
    let env = renamed(old, new);
    let err = stderr(&env.ok(&["sync", new]));
    let line = line_where(&err, &format!("naming both {old} and {new}"), |l| {
        l.contains(old) && l.contains(new)
    });
    assert!(
        line < marker(&err),
        "the warning came after the read began:\n{err}"
    );

    let env = renamed(old, new);
    let (output, value) = env.json(&["sync", new]);
    assert!(output.status.success(), "{value}");
    let result = &value["result"];
    let warnings = result["warnings"]
        .as_array()
        .unwrap_or_else(|| panic!("no `warnings` array in the sync result: {value}"));
    assert!(
        warnings.iter().any(|w| {
            let w = w.to_string();
            w.contains(old) && w.contains(new)
        }),
        "no warning naming both sources: {value}"
    );
    let run = &result["runs"][0];
    assert_eq!(run["source"], new, "{value}");
    assert_eq!(run["status"], "ok", "the read still runs: {value}");
    assert_eq!(run["counts"]["submitted"], 2, "{value}");
    let from: chrono::DateTime<chrono::Utc> =
        run["window"]["from"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        from,
        WINDOW_START
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
    );
}

/// S17: `sync --json` prints exactly one envelope on stdout; a broken stderr changes neither
/// the result nor the exit code
#[test]
fn S17_sync_json_stdout_is_one_envelope_and_a_broken_stderr_changes_nothing() {
    let bodies = [
        ("https://x/1", "first body"),
        ("https://x/2", "second body"),
    ];
    let env = Env::new();
    env.add_source("project", "personal");
    env.bodies(&bodies);
    let output = env.rastro(&["sync", "project", "--json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let values: Vec<Value> = serde_json::Deserializer::from_slice(&output.stdout)
        .into_iter::<Value>()
        .collect::<Result<_, _>>()
        .unwrap_or_else(|e| panic!("stdout is not JSON only ({e}): {output:?}"));
    assert_eq!(values.len(), 1, "{values:?}");
    let envelope = &values[0];
    assert_eq!(envelope["operation"], "sync", "{envelope}");
    assert_eq!(envelope["schema_version"], 2, "{envelope}");

    // The same first sync with stderr's read end closed before `rastro` writes to it.
    let broken = Env::new();
    broken.add_source("project", "personal");
    broken.bodies(&bodies);
    let mut child = broken
        .command(&["sync", "project", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stderr.take());
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), output.status.code(), "{out:?}");
    let value: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("not one envelope with a broken stderr ({e}): {out:?}"));
    let (a, b) = (&envelope["result"]["runs"][0], &value["result"]["runs"][0]);
    for field in ["status", "counts", "coverage"] {
        assert_eq!(a[field], b[field], "{field}: {a} vs {b}");
    }
}
