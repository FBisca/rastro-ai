//! The decision adapter over the plugin protocol, with a fake scoring service standing in for
//! jev, so no live service is called. The core's schema gate and keep rule are stood in for here
//! (the core owns them, ADR-0016 §2); the adapter's contract is to relay the endpoint's answer
//! unchanged so that gate alone decides what is kept.
use rastro_plugin_sdk::{JUDGMENT_FLOOR, Manifest, PROTOCOL_VERSION};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};

// spec: A decision adapter scores text through the same session protocol
#[test]
fn the_manifest_declares_score_text_and_no_action_permission() {
    let manifest = Manifest::parse(include_str!("../rastro-plugin.toml")).unwrap();
    manifest.check_agent_tool().unwrap();
    assert_eq!(manifest.kinds, ["agent-tool-adapter"]);
    assert_eq!(manifest.agent_tool.unwrap().capabilities, ["score_text"]);
    assert!(manifest.permissions.actions.is_empty());
    assert!(manifest.permissions.submit.is_empty());
    assert_eq!(manifest.permissions.env, ["RASTRO_DECISION_ENDPOINT"]);
    assert_eq!(manifest.enforcement, JUDGMENT_FLOOR.map(String::from));
}

/// The caller's schema the tests score against: a score in [0, 1] and a label, nothing else.
fn caller_schema() -> Value {
    json!({
        "type": "object",
        "required": ["score", "label"],
        "additionalProperties": false,
        "properties": {
            "score": {"type": "number", "minimum": 0, "maximum": 1},
            "label": {"type": "string"}
        }
    })
}

/// The core's schema gate, stood in for the tests: a direct reading of [`caller_schema`],
/// independent of the adapter's code.
fn passes_caller_schema(output: &Value) -> bool {
    let Some(object) = output.as_object() else {
        return false;
    };
    object.contains_key("score")
        && object.contains_key("label")
        && object.keys().all(|key| key == "score" || key == "label")
        && object["score"]
            .as_f64()
            .is_some_and(|score| (0.0..=1.0).contains(&score))
        && object["label"].is_string()
}

/// The core's keep rule, stood in for the tests: only output that passes the caller's schema
/// replaces the last good results; anything else leaves them.
fn kept_after(last_good: &Value, reply: &Value) -> Value {
    match reply.get("result").and_then(|result| result.get("output")) {
        Some(output) if passes_caller_schema(output) => output.clone(),
        _ => last_good.clone(),
    }
}

/// A fake scoring service: answers one request with `status` and `body`, and reports the method,
/// path and body it received.
fn fake_scorer(status: u16, body: &str) -> (String, Receiver<(String, String, String)>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let base = format!("http://{}", server.server_addr().to_ip().unwrap());
    let (tx, rx) = mpsc::channel();
    let body = body.to_string();
    thread::spawn(move || {
        // No request within the wait: dropping the sender surfaces it at the receiver.
        let Ok(Some(mut request)) = server.recv_timeout(Duration::from_secs(10)) else {
            return;
        };
        let mut received = String::new();
        request.as_reader().read_to_string(&mut received).unwrap();
        let seen = (
            request.method().to_string(),
            request.url().to_string(),
            received,
        );
        let response = tiny_http::Response::from_string(body).with_status_code(status);
        request.respond(response).unwrap();
        // A test that never reads the record still gets its reply.
        let _ = tx.send(seen);
    });
    (base, rx)
}

fn decision_request() -> Value {
    json!({"jsonrpc": "2.0", "id": 2, "method": "decision.request", "params": {
        "instructions": "Score the text for commitment risk.",
        "input": {"text": "I'll send it by Friday."},
        "schema": caller_schema()}})
}

/// Runs the adapter against the endpoint (when set) and returns the replies to `initialize` and
/// `decision.request`.
fn converse(endpoint: Option<&str>, request: Value) -> (Value, Value) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rastro-adapter-decision"));
    command.env_remove("RASTRO_DECISION_ENDPOINT");
    if let Some(endpoint) = endpoint {
        command.env("RASTRO_DECISION_ENDPOINT", endpoint);
    }
    let mut child = command
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
        "protocol": PROTOCOL_VERSION, "manifest_id": "rastro-adapter-decision",
        "role": "agent_tool_adapter"}}),
    );
    let decided = ask(request);
    drop(stdin);
    child.wait().unwrap();
    (initialized, decided)
}

// spec: A decision adapter scores text through the same session protocol
#[test]
fn schema_valid_decision_output_is_relayed_and_kept() {
    let answer = json!({"score": 0.9, "label": "commitment"});
    let (base, seen) = fake_scorer(200, &answer.to_string());
    let (initialized, decided) = converse(Some(&format!("{base}/score")), decision_request());
    assert_eq!(
        initialized["result"],
        json!({"protocol": PROTOCOL_VERSION, "manifest_id": "rastro-adapter-decision"})
    );
    // The answer is relayed unchanged, passes the caller's schema, and is kept.
    assert_eq!(decided["id"], 2);
    assert_eq!(decided["result"], json!({"output": answer}));
    assert!(passes_caller_schema(&decided["result"]["output"]));
    let last_good = json!({"score": 0.2, "label": "old"});
    assert_eq!(kept_after(&last_good, &decided), answer);
    // The endpoint received the caller's instructions, input and schema as data.
    let (method, path, body) = seen
        .recv_timeout(Duration::from_secs(5))
        .expect("the adapter answered without calling the scoring endpoint");
    assert_eq!(method, "POST");
    assert_eq!(path, "/score");
    let sent: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(sent["instructions"], "Score the text for commitment risk.");
    assert_eq!(sent["input"], json!({"text": "I'll send it by Friday."}));
    assert_eq!(sent["schema"], caller_schema());
}

// spec: A decision adapter scores text through the same session protocol
#[test]
fn a_broken_decision_shape_fails_the_run_and_keeps_the_last_good() {
    // The service answers a bare score instead of the object the caller's schema asks for.
    let broken = json!(0.9);
    let (base, _seen) = fake_scorer(200, &broken.to_string());
    let (_, decided) = converse(Some(&format!("{base}/score")), decision_request());
    // Relayed unchanged: the adapter neither repairs it into a keepable shape nor hides it.
    assert_eq!(decided["result"], json!({"output": broken}));
    // The core's gate fails on it, so the run fails...
    assert!(!passes_caller_schema(&decided["result"]["output"]));
    // ...and the last good results stay.
    let last_good = json!({"score": 0.2, "label": "old"});
    assert_eq!(kept_after(&last_good, &decided), last_good);
}

// spec: A decision adapter scores text through the same session protocol
#[test]
fn a_failed_scoring_call_is_an_error_response_not_a_result() {
    for (status, body) in [(500, r#"{"error":"overloaded"}"#), (200, "score: 0.9")] {
        let (base, _seen) = fake_scorer(status, body);
        let (_, decided) = converse(Some(&format!("{base}/score")), decision_request());
        assert!(
            decided.get("result").is_none(),
            "{status} {body}: {decided}"
        );
        let message = decided["error"]["message"].as_str().unwrap();
        assert!(!message.is_empty(), "{decided}");
        if status == 500 {
            assert!(message.contains("500"), "{decided}");
        }
    }
}

// spec: A decision adapter scores text through the same session protocol
#[test]
fn without_an_endpoint_the_request_is_an_error_naming_the_variable() {
    let (initialized, decided) = converse(None, decision_request());
    assert_eq!(
        initialized["result"]["manifest_id"],
        "rastro-adapter-decision"
    );
    assert!(decided.get("result").is_none(), "{decided}");
    assert!(
        decided["error"]["message"]
            .as_str()
            .unwrap()
            .contains("RASTRO_DECISION_ENDPOINT"),
        "{decided}"
    );
}
