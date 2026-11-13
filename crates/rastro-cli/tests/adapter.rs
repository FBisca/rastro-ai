#![allow(non_snake_case)]
//! The agent tool adapter end to end: `rastro meeting import` calls the configured adapter, a
//! plugin of kind `agent-tool-adapter`, over the plugin protocol, and keeps only what its own
//! checks accept. The adapter here is a shell script that saves the request it is sent and
//! answers with the candidates in `candidates.json`, so no model is ever called.
mod support;
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command, time::Duration};
use tempfile::TempDir;

const TRANSCRIPT: &str = "Alice: I'll send Bob the budget by Friday.\n\
Bob: Thanks. I'll review it with you next week if the numbers are final.\n\
Carol: I will email Dave the notes tomorrow.\n\
Alice: Bob, can you share the roadmap?\n";

const MANIFEST: &str = r#"id = "test-adapter"
name = "Rastro test agent tool adapter"
version = "0.1.0"
author = "Rastro"
kinds = ["agent-tool-adapter"]
protocol = 1
command = "adapter"
agent_tool = { capabilities = ["judge_text"] }

[permissions]
submit = []
scopes = []
env = []
actions = []
"#;

/// Saves the judgment request, marks that it was launched, and answers `candidates.json`.
const ADAPTER: &str = r#"#!/bin/sh
touch "$RASTRO_HOME/launched"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' "$line" > "$RASTRO_HOME/initialize.json"
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocol":"1.0","manifest_id":"test-adapter"}}\n' "$id"
      ;;
    *'"method":"judgment.request"'*)
      printf '%s\n' "$line" > "$RASTRO_HOME/judgment.json"
      printf '{"jsonrpc":"2.0","id":%s,"result":' "$id"
      cat "$RASTRO_HOME/response.json"
      printf '}\n'
      ;;
  esac
done
"#;

struct Env {
    home: TempDir,
}
impl Env {
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
        };
        fs::create_dir_all(env.path("config/plugins/test-adapter")).unwrap();
        fs::write(
            env.path("config/config.toml"),
            "[commitment_adapter]\nplugin = \"test-adapter\"\n",
        )
        .unwrap();
        env.manifest(MANIFEST);
        let adapter = env.path("config/plugins/test-adapter/adapter");
        fs::write(&adapter, ADAPTER).unwrap();
        fs::set_permissions(&adapter, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(env.path("transcript.txt"), TRANSCRIPT).unwrap();
        env.candidates(&good());
        env.ok(&["identity", "me"]);
        env
    }
    fn path(&self, name: &str) -> PathBuf {
        self.home.path().join(name)
    }
    fn manifest(&self, text: &str) {
        fs::write(
            self.path("config/plugins/test-adapter/rastro-plugin.toml"),
            text,
        )
        .unwrap();
    }
    /// What the adapter answers with: the whole `result` of its response.
    fn respond(&self, result: Value) {
        fs::write(self.path("response.json"), result.to_string()).unwrap();
    }
    fn candidates(&self, candidates: &[Value]) {
        self.respond(json!({ "candidates": candidates }));
    }
    fn run(&self, args: &[&str]) -> (bool, Value) {
        let out = Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.path("no-plugins"))
            .args(args)
            .arg("--json")
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
            panic!(
                "{args:?}: {}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        });
        (out.status.success(), value)
    }
    fn ok(&self, args: &[&str]) -> Value {
        let (ok, value) = self.run(args);
        assert!(ok, "{args:?}: {value}");
        value["result"].clone()
    }
    fn import(&self) -> (bool, Value) {
        let transcript = self.path("transcript.txt");
        let (ok, value) = self.run(&[
            "meeting",
            "import",
            transcript.to_str().unwrap(),
            "--meeting",
            "weekly-1",
            "--scope",
            "work",
            "--captured-at",
            "2026-11-10T10:00:00Z",
            "--owner-speaker",
            "Alice",
        ]);
        (ok, value["result"].clone())
    }
    fn import_ok(&self) -> Value {
        let (ok, result) = self.import();
        assert!(ok, "{result}");
        result
    }
    /// The params of the `judgment.request` the adapter was sent.
    fn judgment(&self) -> Value {
        let line = fs::read(self.path("judgment.json")).unwrap();
        let message: Value = serde_json::from_slice(&line).unwrap();
        message["params"].clone()
    }
    fn launched(&self) -> bool {
        self.path("launched").exists()
    }
}

fn candidate(
    turn: u32,
    kind: &str,
    responsible: &str,
    recipient: Option<&str>,
    promise: &str,
    date: Option<&str>,
    quote: &str,
) -> Value {
    json!({"turn": turn, "kind": kind, "responsible": responsible, "recipient": recipient,
        "promise": promise, "stated_date": date, "quote": quote})
}

fn good() -> Vec<Value> {
    vec![
        candidate(
            1,
            "promise",
            "Alice",
            Some("Bob"),
            "send Bob the budget",
            Some("by Friday"),
            "I'll send Bob the budget by Friday",
        ),
        candidate(
            2,
            "waiting_on",
            "Bob",
            Some("Alice"),
            "review the numbers",
            Some("next week"),
            "I'll review it with you next week if the numbers are final.",
        ),
    ]
}

/// S6: the core calls the adapter over the plugin protocol with the content, its own
/// instructions and the fields it expects, and keeps only the candidates that pass its checks.
#[test]
fn S6_the_core_calls_the_adapter_and_keeps_only_the_candidates_that_pass_its_checks() {
    let env = Env::new();
    let mut proposed = good();
    // Between other people: the owner is not involved, so the core leaves it out and counts it.
    proposed.push(candidate(
        3,
        "promise",
        "Carol",
        Some("Dave"),
        "email Dave the notes",
        None,
        "I will email Dave the notes tomorrow",
    ));
    // A date the quote does not state is not kept as stated.
    proposed.push(candidate(
        4,
        "ask",
        "Bob",
        Some("Alice"),
        "share the roadmap",
        Some("by Monday"),
        "Bob, can you share the roadmap?",
    ));
    env.candidates(&proposed);
    let result = env.import_ok();
    assert_eq!(result["extraction"]["state"], "succeeded", "{result}");
    assert_eq!(
        result["extraction"]["extractor"], "test-adapter",
        "{result}"
    );
    assert_eq!(result["extraction"]["excluded"], 1, "{result}");

    let hypotheses = result["hypotheses"].as_array().unwrap();
    assert_eq!(hypotheses.len(), 3, "{result}");
    assert_eq!(hypotheses[0]["stated_date"]["text"], "by Friday");
    assert_eq!(hypotheses[2]["stated_date"], json!({"state": "unknown"}));

    // The core asked in the plugin protocol: initialize as an adapter, then the judgment.
    let initialize: Value =
        serde_json::from_slice(&fs::read(env.path("initialize.json")).unwrap()).unwrap();
    assert_eq!(initialize["params"]["role"], "agent_tool_adapter");
    assert_eq!(initialize["params"]["manifest_id"], "test-adapter");
    let request = env.judgment();
    assert!(
        request["instructions"]
            .as_str()
            .unwrap()
            .contains("untrusted"),
        "{request}"
    );
    assert_eq!(request["owner_speaker"], "Alice");
    let turns = request["turns"].as_array().unwrap();
    assert_eq!(turns.len(), 4);
    assert_eq!(
        turns[0],
        json!({"turn": 1, "speaker": "Alice", "text": "I'll send Bob the budget by Friday."})
    );
    assert_eq!(
        request["fields"],
        json!([
            "turn",
            "kind",
            "responsible",
            "recipient",
            "promise",
            "stated_date",
            "quote"
        ])
    );

    // What the adapter proposed as a kind is kept with the recorded output.
    let database = rusqlite::Connection::open(env.path("data/rastro.sqlite3")).unwrap();
    let recorded: String = database
        .query_row("SELECT candidates FROM meeting_output", [], |row| {
            row.get(0)
        })
        .unwrap();
    let recorded: Value = serde_json::from_str(&recorded).unwrap();
    let kinds: Vec<&str> = recorded
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["promise", "waiting_on", "ask"]);
}

/// S6: a response that breaks the expected shape fails the run and keeps the last good results.
#[test]
fn S6_an_adapter_whose_response_breaks_the_shape_fails_the_run_and_keeps_the_last_good_results() {
    let env = Env::new();
    let last_good = env.import_ok();
    assert_eq!(last_good["hypotheses"].as_array().unwrap().len(), 2);

    let mut unknown_field = good();
    unknown_field[0]["confidence"] = json!(0.9);
    let mut no_kind = good();
    no_kind[0].as_object_mut().unwrap().remove("kind");
    let mut other_kind = good();
    other_kind[0]["kind"] = json!("done");
    let mut no_promise = good();
    no_promise[0].as_object_mut().unwrap().remove("promise");
    for (name, response) in [
        ("an unknown field", json!({"candidates": unknown_field})),
        ("a missing kind", json!({"candidates": no_kind})),
        ("a kind that is not one", json!({"candidates": other_kind})),
        ("a missing promise", json!({"candidates": no_promise})),
        ("candidates that are not a list", json!({"candidates": "x"})),
        ("no candidates", json!({})),
        (
            "an extra top-level field",
            json!({"candidates": [], "done": true}),
        ),
    ] {
        env.respond(response);
        let (ok, failed) = env.import();
        assert!(!ok, "{name}: {failed}");
        assert_eq!(failed["extraction"]["state"], "failed", "{name}");
        assert_eq!(
            failed["extraction"]["cause"], "extractor_output_invalid",
            "{name}: {failed}"
        );
        assert_eq!(failed["stale"], true, "{name}");
        assert_eq!(failed["hypotheses"], last_good["hypotheses"], "{name}");
    }

    // An answer that is not even a JSON-RPC line breaks the shape too.
    fs::write(env.path("response.json"), "not json").unwrap();
    let (ok, failed) = env.import();
    assert!(!ok);
    assert_eq!(failed["extraction"]["cause"], "extractor_output_invalid");
    assert_eq!(failed["hypotheses"], last_good["hypotheses"]);
}

/// S7: a manifest that names a tool, or that does not declare the kind, is refused before the
/// adapter runs.
#[test]
fn S7_a_manifest_that_names_a_tool_or_lacks_the_kind_is_refused_before_anything_runs() {
    let named_in_table = MANIFEST.replace(
        "agent_tool = { capabilities = [\"judge_text\"] }",
        "agent_tool = { capabilities = [\"judge_text\"], tool = \"claude-code\" }",
    );
    let named_as_capability = MANIFEST.replace("\"judge_text\"", "\"claude-code\"");
    let no_capabilities = MANIFEST.replace("[\"judge_text\"]", "[]");
    let no_agent_tool = MANIFEST.replace("agent_tool = { capabilities = [\"judge_text\"] }\n", "");
    let wrong_kind = MANIFEST.replace("[\"agent-tool-adapter\"]", "[\"extractor\"]");
    let with_actions = MANIFEST.replace("actions = []", "actions = [\"send\"]");
    for (name, manifest) in [
        ("a tool named in the declaration", named_in_table),
        ("a tool named as a capability", named_as_capability),
        ("no capabilities", no_capabilities),
        ("no agent tool declaration", no_agent_tool),
        ("a different kind", wrong_kind),
        ("an action permission", with_actions),
    ] {
        let env = Env::new();
        env.manifest(&manifest);
        let (ok, failed) = env.import();
        assert!(!ok, "{name}: {failed}");
        assert_eq!(failed["extraction"]["state"], "failed", "{name}");
        assert_eq!(failed["extraction"]["cause"], "extractor_failed", "{name}");
        assert!(!env.launched(), "{name}: the adapter ran");
    }

    // The same manifest naming the kind and its capabilities is accepted and run.
    let env = Env::new();
    env.import_ok();
    assert!(env.launched());
}

/// S6: an adapter that never answers is stopped at its timeout and fails the run, and one that
/// answers with a JSON-RPC error fails it too, with its message as the reason.
#[test]
fn S6_an_adapter_that_never_answers_or_answers_with_an_error_fails_the_run() {
    let env = Env::new();
    fs::write(
        env.path("config/config.toml"),
        "[commitment_adapter]\nplugin = \"test-adapter\"\ntimeout_seconds = 1\n",
    )
    .unwrap();
    let script = env.path("config/plugins/test-adapter/adapter");
    fs::write(&script, "#!/bin/sh\nexec sleep 30\n").unwrap();
    let started = std::time::Instant::now();
    let (ok, failed) = env.import();
    assert!(!ok, "{failed}");
    assert_eq!(
        failed["extraction"]["cause"], "extractor_failed",
        "{failed}"
    );
    assert!(
        failed["extraction"]["reason"]
            .as_str()
            .unwrap()
            .contains("did not answer within 1 s"),
        "{failed}"
    );
    assert!(started.elapsed() < Duration::from_secs(20));

    fs::write(
        &script,
        ADAPTER.replace(
            "printf '{\"jsonrpc\":\"2.0\",\"id\":%s,\"result\":' \"$id\"\n      cat \"$RASTRO_HOME/response.json\"\n      printf '}\\n'",
            "printf '{\"jsonrpc\":\"2.0\",\"id\":%s,\"error\":{\"code\":-32000,\"message\":\"overloaded\"}}\\n' \"$id\"",
        ),
    )
    .unwrap();
    let (ok, failed) = env.import();
    assert!(!ok, "{failed}");
    assert_eq!(
        failed["extraction"]["cause"], "extractor_failed",
        "{failed}"
    );
    assert!(
        failed["extraction"]["reason"]
            .as_str()
            .unwrap()
            .contains("overloaded"),
        "{failed}"
    );
}
