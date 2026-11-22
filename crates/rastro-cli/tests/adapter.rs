#![allow(non_snake_case)]
//! The agent tool adapter end to end: `rastro meeting import` calls the configured adapter, a
//! plugin of kind `agent-tool-adapter`, over the plugin protocol, runs its own gates on every
//! answer, and sends a gate failure back as the next turn of the same session while turns
//! remain under the cap. Only the final validated output becomes evidence. The adapter here is
//! a shell script that records every request it is sent and answers turn `n` from
//! `response-<n>.json` (falling back to `response.json`), so no model is ever called.
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
enforcement = ["tools_off", "no_mcp_servers", "no_setting_sources"]

[permissions]
submit = []
scopes = []
env = []
actions = []
"#;

/// Records every launch and every judgment request, and answers turn `n` from
/// `response-<n>.json`, else `response.json`. `error-<n>.json` answers turn `n` with a
/// JSON-RPC error instead; `silent-from` holds the first turn number that gets no answer.
const ADAPTER: &str = r#"#!/bin/sh
echo launch >> "$RASTRO_HOME/launches"
printf '0\n' > "$RASTRO_HOME/turns"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' "$line" > "$RASTRO_HOME/initialize.json"
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocol":"1.0","manifest_id":"test-adapter"}}\n' "$id"
      ;;
    *'"method":"judgment.request"'*)
      n=$(( $(cat "$RASTRO_HOME/turns" 2>/dev/null || echo 0) + 1 ))
      printf '%s\n' "$n" > "$RASTRO_HOME/turns"
      printf '%s\n' "$line" > "$RASTRO_HOME/judgment-$n.json"
      if [ -f "$RASTRO_HOME/silent-from" ] && [ "$n" -ge "$(cat "$RASTRO_HOME/silent-from")" ]; then
        exec sleep 30
      fi
      if [ -f "$RASTRO_HOME/error-$n.json" ]; then
        printf '{"jsonrpc":"2.0","id":%s,"error":' "$id"
        cat "$RASTRO_HOME/error-$n.json"
        printf '}\n'
      else
        printf '{"jsonrpc":"2.0","id":%s,"result":' "$id"
        if [ -f "$RASTRO_HOME/response-$n.json" ]; then
          cat "$RASTRO_HOME/response-$n.json"
        else
          cat "$RASTRO_HOME/response.json"
        fi
        printf '}\n'
      fi
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
        env.config("[commitment_adapter]\nplugin = \"test-adapter\"\n");
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
    fn config(&self, text: &str) {
        fs::write(self.path("config/config.toml"), text).unwrap();
    }
    fn manifest(&self, text: &str) {
        fs::write(
            self.path("config/plugins/test-adapter/rastro-plugin.toml"),
            text,
        )
        .unwrap();
    }
    /// What the adapter answers on every turn without a per-turn answer: the whole `result`.
    fn respond(&self, result: Value) {
        fs::write(self.path("response.json"), result.to_string()).unwrap();
    }
    /// What the adapter answers on turn `n`: the whole `result` of its response.
    fn respond_turn(&self, n: u64, result: Value) {
        fs::write(self.path(&format!("response-{n}.json")), result.to_string()).unwrap();
    }
    /// The JSON-RPC error the adapter answers turn `n` with.
    fn error_turn(&self, n: u64, error: Value) {
        fs::write(self.path(&format!("error-{n}.json")), error.to_string()).unwrap();
    }
    /// The adapter stops answering from turn `n` on.
    fn silent_from(&self, n: u64) {
        fs::write(self.path("silent-from"), n.to_string()).unwrap();
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
        (
            ok,
            value
                .get("result")
                .filter(|result| !result.is_null())
                .cloned()
                .unwrap_or(value),
        )
    }
    fn import_ok(&self) -> Value {
        let (ok, result) = self.import();
        assert!(ok, "{result}");
        result
    }
    /// The params of the `judgment.request` the adapter was sent on turn `n`.
    fn judgment(&self, n: u64) -> Value {
        let line = fs::read(self.path(&format!("judgment-{n}.json"))).unwrap();
        let message: Value = serde_json::from_slice(&line).unwrap();
        message["params"].clone()
    }
    /// How many judgment turns the adapter was sent.
    fn turns(&self) -> u64 {
        fs::read_to_string(self.path("turns"))
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0)
    }
    /// How many adapter processes were launched.
    fn launches(&self) -> usize {
        fs::read_to_string(self.path("launches"))
            .map(|text| text.lines().count())
            .unwrap_or(0)
    }
    /// Forgets the launches recorded so far, so a count names one run's processes.
    fn forget_launches(&self) {
        fs::write(self.path("launches"), "").unwrap();
    }
    fn launched(&self) -> bool {
        self.launches() > 0
    }
    /// The session history the run recorded: each turn's request, answer and gate verdict as
    /// JSON, in turn order. The answer is `null` for a turn the adapter never answered.
    fn history(&self, run: i64) -> Vec<Value> {
        let database = rusqlite::Connection::open(self.path("data/rastro.sqlite3")).unwrap();
        database
            .prepare(
                "SELECT turn, request, answer, verdict FROM session_turn
                 WHERE run_id = ?1 ORDER BY turn",
            )
            .unwrap()
            .query_map([run], |row| {
                let request: String = row.get(1)?;
                let answer: Option<String> = row.get(2)?;
                let verdict: String = row.get(3)?;
                Ok(json!({
                    "turn": row.get::<_, i64>(0)?,
                    "request": serde_json::from_str::<Value>(&request).unwrap(),
                    "answer": answer.map(|a| serde_json::from_str::<Value>(&a).unwrap()),
                    "verdict": serde_json::from_str::<Value>(&verdict).unwrap(),
                }))
            })
            .unwrap()
            .map(|turn| turn.unwrap())
            .collect()
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

/// The same candidates with the first one's quote no longer an exact piece of its turn.
fn broken_quote() -> Vec<Value> {
    let mut broken = good();
    broken[0]["quote"] = json!("I'll send Bob the budget by Friday!");
    broken
}

/// S1: the owner imports a conversation with a configured adapter and an answer that passes
/// the core's gates; only the candidates that pass the core's own checks are kept: an exact
/// contiguous quote, the owner's involvement and a stated date the quote states.
// spec: The core calls the adapter and keeps only the candidates that pass its checks
#[test]
fn S1_only_the_candidates_the_cores_checks_pass_are_kept() {
    let env = Env::new();
    env.config(
        "[commitment_adapter]\nplugin = \"test-adapter\"\nmax_turns = 3\ntimeout_seconds = 600\n",
    );
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

    // The answer passed every gate: one session, one turn, no retry.
    assert_eq!(env.launches(), 1);
    assert_eq!(env.turns(), 1);

    // The core asked in the plugin protocol: initialize as an adapter, then the judgment.
    let initialize: Value =
        serde_json::from_slice(&fs::read(env.path("initialize.json")).unwrap()).unwrap();
    assert_eq!(initialize["params"]["role"], "agent_tool_adapter");
    assert_eq!(initialize["params"]["manifest_id"], "test-adapter");
    let request = env.judgment(1);
    assert!(request.get("session").is_none(), "{request}");
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
        .query_row("SELECT candidates FROM conversation_output", [], |row| {
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

/// S2: an answer that fails a shape or span gate earns the failure reason as the next turn
/// of the same session while turns remain under the cap: gate, field, expected and actual,
/// the prior answer along, and no new session opened.
// spec: The core calls the adapter and keeps only the candidates that pass its checks
#[test]
fn S2_a_gate_failure_earns_a_retry_turn_in_the_same_session() {
    // The span gate: a quote that is not an exact, contiguous piece of its turn.
    let env = Env::new();
    let broken = broken_quote();
    env.respond_turn(1, json!({ "candidates": broken }));
    env.respond_turn(2, json!({ "candidates": good() }));
    let result = env.import_ok();
    assert_eq!(result["extraction"]["state"], "succeeded", "{result}");
    assert_eq!(
        result["hypotheses"].as_array().unwrap().len(),
        2,
        "{result}"
    );
    // One process, one initialize, two turns: the retry stayed in the same session.
    assert_eq!(env.launches(), 1);
    assert_eq!(env.turns(), 2);
    let first = env.judgment(1);
    assert!(first.get("session").is_none(), "{first}");
    let second = env.judgment(2);
    assert_eq!(second["instructions"], first["instructions"]);
    assert_eq!(second["turns"], first["turns"]);
    assert_eq!(second["fields"], first["fields"]);
    let session = &second["session"];
    assert_eq!(session["turn"], 2, "{second}");
    assert_eq!(session["prior_answer"], json!({ "candidates": broken }));
    assert_eq!(
        session["feedback"],
        json!([{
            "gate": "span",
            "field": "candidates[0].quote",
            "expected": "an exact, contiguous piece of turn 1's text",
            "actual": "I'll send Bob the budget by Friday!"
        }]),
        "{second}"
    );

    // The shape gate: a field the judgment shape does not have.
    let env = Env::new();
    let mut broken = good();
    broken[0]["confidence"] = json!(0.9);
    env.respond_turn(1, json!({ "candidates": broken, "session_id": "sess-1" }));
    env.respond_turn(2, json!({ "candidates": good() }));
    let result = env.import_ok();
    assert_eq!(
        result["hypotheses"].as_array().unwrap().len(),
        2,
        "{result}"
    );
    assert_eq!(env.launches(), 1);
    assert_eq!(env.turns(), 2);
    let session = &env.judgment(2)["session"];
    assert_eq!(session["turn"], 2, "{session}");
    // The broken answer is not the judgment shape; the retry carries the whole prior answer in
    // the task's own shape, its harness session id still riding along so the retry can resume it,
    // and the feedback names what was there.
    assert_eq!(
        session["prior_answer"],
        json!({ "candidates": broken, "session_id": "sess-1" })
    );
    assert_eq!(
        session["feedback"],
        json!([{
            "gate": "shape",
            "field": "candidates[0].confidence",
            "expected": "only the fields turn, kind, responsible, recipient, promise, stated_date, quote",
            "actual": "0.9"
        }]),
        "{session}"
    );
}

/// S3: gate failures that exhaust the turn cap fail the run, and the last good results stay
/// marked stale. The cap is three turns by default and the owner can set it.
// spec: The core calls the adapter and keeps only the candidates that pass its checks
#[test]
fn S3_gate_failures_that_exhaust_the_turn_cap_fail_the_run_stale() {
    // The span gate, every turn, under the default cap of three turns.
    let env = Env::new();
    let last_good = env.import_ok();
    env.forget_launches();
    env.candidates(&broken_quote());
    let (ok, failed) = env.import();
    assert!(!ok, "{failed}");
    assert_eq!(failed["extraction"]["state"], "failed", "{failed}");
    assert_eq!(failed["extraction"]["cause"], "invalid_quote", "{failed}");
    assert_eq!(failed["stale"], true, "{failed}");
    assert_eq!(failed["hypotheses"], last_good["hypotheses"], "{failed}");
    assert_eq!(env.turns(), 3, "the cap is three turns by default");
    assert_eq!(env.launches(), 1, "the retries stayed in one session");
    assert_eq!(env.judgment(3)["session"]["turn"], 3);

    // The shape gate, every turn: the same cap, the shape's own cause.
    let env = Env::new();
    let last_good = env.import_ok();
    env.forget_launches();
    let mut broken = good();
    broken[0]["confidence"] = json!(0.9);
    env.candidates(&broken);
    let (ok, failed) = env.import();
    assert!(!ok, "{failed}");
    assert_eq!(
        failed["extraction"]["cause"], "extractor_output_invalid",
        "{failed}"
    );
    assert_eq!(failed["stale"], true, "{failed}");
    assert_eq!(failed["hypotheses"], last_good["hypotheses"], "{failed}");
    assert_eq!(env.turns(), 3);

    // A cap the owner sets bounds the retries too.
    let env = Env::new();
    env.config("[commitment_adapter]\nplugin = \"test-adapter\"\nmax_turns = 2\n");
    let last_good = env.import_ok();
    env.forget_launches();
    env.candidates(&broken_quote());
    let (ok, failed) = env.import();
    assert!(!ok, "{failed}");
    assert_eq!(failed["extraction"]["cause"], "invalid_quote", "{failed}");
    assert_eq!(failed["stale"], true, "{failed}");
    assert_eq!(failed["hypotheses"], last_good["hypotheses"], "{failed}");
    assert_eq!(env.turns(), 2);
    assert_eq!(env.launches(), 1);
}

/// S3: an adapter that never answers or answers with an error fails the run, on the first
/// turn or a retry turn alike, and the last good results stay marked stale.
// spec: The core calls the adapter and keeps only the candidates that pass its checks
#[test]
fn S3_an_adapter_that_never_answers_or_answers_with_an_error_fails_the_run_stale() {
    // Never answers the first turn: stopped at the per-turn timeout.
    let env = Env::new();
    env.config("[commitment_adapter]\nplugin = \"test-adapter\"\ntimeout_seconds = 1\n");
    let last_good = env.import_ok();
    env.forget_launches();
    env.silent_from(1);
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
    assert_eq!(failed["stale"], true, "{failed}");
    assert_eq!(failed["hypotheses"], last_good["hypotheses"], "{failed}");

    // Answers the first turn with an error: its message is the reason.
    let env = Env::new();
    let last_good = env.import_ok();
    env.forget_launches();
    env.error_turn(1, json!({"code": -32000, "message": "overloaded"}));
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
    assert_eq!(failed["stale"], true, "{failed}");
    assert_eq!(failed["hypotheses"], last_good["hypotheses"], "{failed}");

    // A retry turn that never answers: the per-turn timeout bounds it too, and the failed
    // session leaves no partial evidence behind.
    let env = Env::new();
    env.config("[commitment_adapter]\nplugin = \"test-adapter\"\ntimeout_seconds = 1\n");
    let last_good = env.import_ok();
    env.forget_launches();
    env.candidates(&broken_quote());
    env.silent_from(2);
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
    assert_eq!(failed["stale"], true, "{failed}");
    assert_eq!(failed["hypotheses"], last_good["hypotheses"], "{failed}");
    assert_eq!(env.turns(), 2);
    assert_eq!(env.launches(), 1);

    // A retry turn answered with an error fails the run the same way.
    let env = Env::new();
    let last_good = env.import_ok();
    env.forget_launches();
    env.candidates(&broken_quote());
    env.error_turn(2, json!({"code": -32000, "message": "overloaded"}));
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
    assert_eq!(failed["stale"], true, "{failed}");
    assert_eq!(failed["hypotheses"], last_good["hypotheses"], "{failed}");
    assert_eq!(env.turns(), 2);
}

/// S5: a run that succeeds after retry turns makes only the final validated output evidence:
/// the recorded output and the hypotheses are the last turn's, not a merge of the session.
// spec: Every turn of a judgment session is recorded
#[test]
fn S5_only_the_final_validated_output_becomes_evidence() {
    let env = Env::new();
    // Turn 1: a broken quote beside a candidate that would pass on its own.
    let mut first = broken_quote();
    first.truncate(1);
    first.push(good().pop().unwrap());
    // Turn 2: only the corrected candidate; the turn-1 passing one is not proposed again.
    let final_answer = vec![good().remove(0)];
    env.respond_turn(1, json!({ "candidates": first }));
    env.respond_turn(2, json!({ "candidates": final_answer }));
    let result = env.import_ok();
    assert_eq!(env.turns(), 2);
    assert_eq!(result["extraction"]["state"], "succeeded", "{result}");

    let hypotheses = result["hypotheses"].as_array().unwrap();
    assert_eq!(hypotheses.len(), 1, "{result}");
    assert_eq!(hypotheses[0]["promise"], "send Bob the budget");
    assert_eq!(hypotheses[0]["quote"]["turn"], 1);

    // The recorded output is the final turn's answer alone: nothing the failed turn proposed
    // survives into the evidence, even the candidate that would have passed.
    let database = rusqlite::Connection::open(env.path("data/rastro.sqlite3")).unwrap();
    let recorded: String = database
        .query_row("SELECT candidates FROM conversation_output", [], |row| {
            row.get(0)
        })
        .unwrap();
    let recorded: Value = serde_json::from_str(&recorded).unwrap();
    assert_eq!(recorded, json!(final_answer));
    let spans: Vec<String> = database
        .prepare("SELECT text FROM conversation_span WHERE text != '' ORDER BY turn")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|text| text.unwrap())
        .collect();
    assert_eq!(
        spans,
        ["I'll send Bob the budget by Friday."],
        "only the final output's cited turn is evidence"
    );
}

/// S7: a manifest that names a tool, or that does not declare the kind, is refused before the
/// adapter runs.
// spec: An adapter's manifest declares its kind and names no tool
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

/// The judgment floor's three controls, as a manifest declares them.
const FLOOR_LINE: &str =
    "enforcement = [\"tools_off\", \"no_mcp_servers\", \"no_setting_sources\"]\n";

/// S7: a bound adapter whose manifest lacks a control the judgment floor requires is refused
/// before the run, naming the unenforceable control; the adapter is never started.
// spec: The core matches the adapter's declared boundary enforcement before a run
#[test]
fn S7_a_manifest_lacking_a_floor_control_refuses_the_run_naming_it() {
    // The manifest without any enforcement declaration, whatever the fixture's default.
    let base = MANIFEST.replace(FLOOR_LINE, "");
    let floor = ["tools_off", "no_mcp_servers", "no_setting_sources"];
    for missing in floor {
        let declared: Vec<String> = floor
            .iter()
            .filter(|control| **control != missing)
            .map(|control| format!("\"{control}\""))
            .collect();
        let manifest = base.replace(
            "agent_tool = { capabilities = [\"judge_text\"] }",
            &format!(
                "agent_tool = {{ capabilities = [\"judge_text\"] }}\nenforcement = [{}]",
                declared.join(", ")
            ),
        );
        let env = Env::new();
        env.manifest(&manifest);
        let (ok, failed) = env.import();
        assert!(!ok, "{missing}: {failed}");
        assert_eq!(
            failed["extraction"]["cause"], "extractor_failed",
            "{missing}: {failed}"
        );
        let reason = failed["extraction"]["reason"].as_str().unwrap();
        assert!(reason.contains(missing), "{missing} not named: {reason}");
        assert!(!env.launched(), "{missing}: the adapter ran");
    }

    // A manifest that declares no enforcement at all lacks every control the floor requires.
    let env = Env::new();
    env.manifest(&base);
    let (ok, failed) = env.import();
    assert!(!ok, "{failed}");
    assert_eq!(
        failed["extraction"]["cause"], "extractor_failed",
        "{failed}"
    );
    assert!(
        !env.launched(),
        "the adapter ran without declaring any control"
    );
}

/// S8: a bound adapter whose manifest declares every control the judgment floor requires
/// proceeds: the run imports and the adapter starts.
// spec: The core matches the adapter's declared boundary enforcement before a run
#[test]
fn S8_a_full_enforcement_declaration_lets_the_run_proceed() {
    let env = Env::new();
    let result = env.import_ok();
    assert_eq!(result["extraction"]["state"], "succeeded", "{result}");
    assert!(env.launched(), "the adapter never started");
    assert_eq!(env.turns(), 1);
}

/// S4: a run that includes failed turns keeps each turn's request, answer and gate verdict in
/// its history, in turn order — the failed turn and the passing one alike.
// spec: Every turn of a judgment session is recorded
#[test]
fn S4_a_runs_history_holds_each_turns_request_answer_and_gate_verdict() {
    // A retry run: turn 1 fails the span gate, turn 2 passes. Both turns are recorded.
    let env = Env::new();
    let broken = broken_quote();
    env.respond_turn(1, json!({ "candidates": broken, "session_id": "sess-1" }));
    env.respond_turn(2, json!({ "candidates": good() }));
    let result = env.import_ok();
    let run = result["extraction"]["run"].as_i64().unwrap();
    let history = env.history(run);
    assert_eq!(history.len(), 2, "{history:?}");
    // Turn 1: the request the core sent, the answer it got, and the span gate's verdict.
    assert_eq!(history[0]["turn"], 1);
    assert_eq!(history[0]["request"], env.judgment(1));
    assert_eq!(
        history[0]["answer"],
        json!({ "candidates": broken, "session_id": "sess-1" })
    );
    assert_eq!(
        history[0]["verdict"],
        json!({ "passed": false, "feedback": env.judgment(2)["session"]["feedback"] }),
        "the failed turn's verdict is the feedback the retry carried"
    );
    // Turn 2: the retry request with its session feedback, the passing answer, a pass verdict.
    assert_eq!(history[1]["turn"], 2);
    assert_eq!(history[1]["request"], env.judgment(2));
    assert_eq!(history[1]["answer"], json!({ "candidates": good() }));
    assert_eq!(history[1]["verdict"], json!({ "passed": true }));

    // A run whose every turn fails keeps all of them, up to the cap.
    let env = Env::new();
    env.candidates(&broken_quote());
    let (ok, failed) = env.import();
    assert!(!ok, "{failed}");
    let run = failed["extraction"]["run"].as_i64().unwrap();
    let history = env.history(run);
    assert_eq!(history.len(), 3, "{history:?}");
    for (n, turn) in history.iter().enumerate() {
        let n = n as u64 + 1;
        assert_eq!(turn["turn"], n as i64);
        assert_eq!(turn["request"], env.judgment(n));
        assert_eq!(turn["answer"], json!({ "candidates": broken_quote() }));
        assert_eq!(turn["verdict"]["passed"], false);
    }

    // A turn the adapter never answered keeps its request and verdict, with no answer.
    let env = Env::new();
    env.config("[commitment_adapter]\nplugin = \"test-adapter\"\ntimeout_seconds = 1\n");
    env.candidates(&broken_quote());
    env.silent_from(2);
    let (ok, failed) = env.import();
    assert!(!ok, "{failed}");
    let run = failed["extraction"]["run"].as_i64().unwrap();
    let history = env.history(run);
    assert_eq!(history.len(), 2, "{history:?}");
    assert_eq!(history[1]["turn"], 2);
    assert_eq!(history[1]["request"], env.judgment(2));
    assert_eq!(history[1]["answer"], json!(null));
    assert_eq!(history[1]["verdict"]["passed"], false);
    assert!(
        history[1]["verdict"]["reason"]
            .as_str()
            .unwrap()
            .contains("did not answer within 1 s"),
        "{:?}",
        history[1]["verdict"]
    );
}

/// S6: an export leaves the turn history out, even when the core recorded failed turns and the
/// transcript is retained: only what the owner supplied is exported.
// spec: Every turn of a judgment session is recorded
#[test]
fn S6_an_export_leaves_turn_history_out() {
    let env = Env::new();
    env.respond_turn(
        1,
        json!({ "candidates": broken_quote(), "session_id": "sess-42" }),
    );
    env.respond_turn(2, json!({ "candidates": good() }));
    let transcript = env.path("transcript.txt");
    let (ok, value) = env.run(&[
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
        "--retain-transcript",
    ]);
    assert!(ok, "{value}");
    let run = value["result"]["extraction"]["run"].as_i64().unwrap();
    // The history is really there, so the export omitting it is not vacuous.
    assert_eq!(
        env.history(run).len(),
        2,
        "the run recorded no history to leave out"
    );

    let export = env.ok(&["meeting", "export", "weekly-1"]);
    let mut keys: Vec<_> = export.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "format",
            "meeting",
            "notice",
            "owner_speaker",
            "speakers",
            "transcript"
        ],
        "{export}"
    );
    assert_eq!(export["transcript"], TRANSCRIPT);
    let text = export.to_string();
    for marker in [
        "sess-42",
        "by Friday!",
        "judgment.request",
        "candidates[0].quote",
    ] {
        assert!(!text.contains(marker), "{marker} leaked: {text}");
    }
}

/// A retry turn carries the harness session id the first turn produced, even when a later
/// answer omits it: the core persists the id across the session so the adapter can resume the
/// same harness session (ADR-0016 §4).
// spec: The core calls the adapter and keeps only the candidates that pass its checks
#[test]
fn a_retry_turn_carries_the_harness_session_id_the_first_turn_produced() {
    let env = Env::new();
    env.respond_turn(
        1,
        json!({ "candidates": broken_quote(), "session_id": "sess-1" }),
    );
    // Turn 2 fails the span gate again, and its answer names no session id.
    env.respond_turn(2, json!({ "candidates": broken_quote() }));
    env.respond_turn(3, json!({ "candidates": good() }));
    let result = env.import_ok();
    assert_eq!(result["extraction"]["state"], "succeeded", "{result}");
    assert_eq!(env.turns(), 3);
    assert_eq!(
        env.judgment(2)["session"]["prior_answer"]["session_id"],
        "sess-1"
    );
    // The id the first turn produced still rides turn 3, though turn 2's answer omitted it.
    assert_eq!(
        env.judgment(3)["session"]["prior_answer"]["session_id"],
        "sess-1"
    );
}
