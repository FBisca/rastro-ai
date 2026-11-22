#![allow(non_snake_case)]
//! Chat commitments end to end: a chat source added with `--extract-commitments` has its new
//! threads' turns asked of its bound extractor after a sync, sent through the bundled template
//! to the configured agent tool adapter, and listed beside meetings as sources of kind `chat`.
//! The connector is the scripted test connector speaking Slack-shaped evidence; the extractor
//! is the real Slack extractor; the adapter is a shell script that records its request and
//! answers a canned response.
mod support;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
};
use tempfile::TempDir;

/// The thread the `chat_thread` fixture syncs: its start message's ts.
const THREAD_TS: &str = "1700000000.000100";
/// The owner's account, as the viewer slot names it and the Person holds it.
const OWNER: &str = "slack:user/U0OWNER";
/// The other author's account.
const ANA: &str = "slack:user/U0ANA";
/// A bot author's account.
const BOT: &str = "slack:user/U0BOT";

const ADAPTER_MANIFEST: &str = r#"id = "test-adapter"
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

/// Answers the plugin protocol: records every launch and each judgment request, and answers
/// from `response.json`.
fn adapter_script() -> String {
    r#"#!/bin/sh
echo launch >> "$RASTRO_HOME/launches"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocol":"1.0","manifest_id":"test-adapter"}}\n' "$id"
      ;;
    *'"method":"judgment.request"'*)
      printf '%s\n' "$line" > "$RASTRO_HOME/request.json"
      printf '{"jsonrpc":"2.0","id":%s,"result":' "$id"
      cat "$RASTRO_HOME/response.json"
      printf '}\n'
      ;;
  esac
done
"#
    .into()
}

/// An extractor that answers `conversation.turns` with a turn citing a message outside the
/// thread, so the core's refusal can be observed. It makes no claims; a test syncs with the real
/// extractor first, then rebinds to this one for the tick.
fn liar_script() -> String {
    r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocol":"1.0","manifest_id":"lying-extractor"}}\n' "$id"
      ;;
    *'"method":"conversation.turns"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":' "$id"
      cat "$RASTRO_HOME/turns.json"
      printf '}\n'
      ;;
  esac
done
"#
    .into()
}

/// The turns the lying extractor answers: the owner's turn cites a message of another channel,
/// outside the thread; Ana's turn cites the thread's start and mentions the owner.
fn liar_turns() -> Value {
    json!({"turns": [
        {"speaker": {"identifier": OWNER, "type": "slack:Account"}, "kind": "person",
         "text": "I'll send it Friday.", "cites": "slack:message/C0OTHER/1700000009.000900"},
        {"speaker": {"identifier": ANA, "type": "slack:Account"}, "kind": "person",
         "text": "can you send me the RBAC doc?",
         "cites": "slack:message/C0ENGCHAN/1700000000.000100",
         "mentions": [{"identifier": OWNER, "type": "slack:Account"}]}
    ]})
}

const LIAR_MANIFEST: &str = r#"id = "lying-extractor"
name = "Rastro lying extractor"
version = "0.1.0"
author = "Rastro"
kinds = ["extractor"]
protocol = 1
command = "extractor"
capabilities = ["conversation.turns"]

[permissions]
submit = []
scopes = ["work"]
env = ["RASTRO_HOME"]
actions = []
"#;

/// The two candidates the fixture's thread supports: Ana's ask of the owner, and the owner's
/// promise. The speakers are the accounts the extractor resolves, as the judgment wire names
/// them; the quotes are exact pieces of the resolved turn text.
fn response() -> Value {
    json!({"candidates": [
        {"turn": 1, "kind": "ask", "responsible": ANA, "recipient": OWNER,
         "promise": "send Ana the RBAC doc", "stated_date": null,
         "quote": "can you send me the RBAC doc?"},
        {"turn": 2, "kind": "promise", "responsible": OWNER, "recipient": null,
         "promise": "send the RBAC doc", "stated_date": "Friday",
         "quote": "I'll send it Friday."}
    ]})
}

/// One `evidence.submit` as a connector script step: the Slack-shaped fields the extractor reads.
fn submit(kind: &str, location: &str, field: &str, quote: &str, trust: &str) -> Value {
    json!({"do": "send", "message": {"jsonrpc": "2.0", "method": "evidence.submit", "params": {
        "kind": kind, "location": location, "field": field, "start": 0,
        "end": quote.len(), "quote": quote, "trust_class": trust,
        "source_updated_at": "2023-11-14T22:13:20Z"}}})
}
/// The token owner's viewer slot: the extractor claims its account held by `me`.
fn viewer() -> Value {
    submit("viewer", "slack:viewer", "/id", "U0OWNER", "structured")
}
/// One author's records: its id, whether it is a bot, and its display name.
fn author(id: &str, display: &str, bot: bool) -> Vec<Value> {
    let location = format!("slack:user/{id}");
    vec![
        submit("user", &location, "/id", id, "structured"),
        submit(
            "user",
            &location,
            "/is_bot",
            if bot { "true" } else { "false" },
            "structured",
        ),
        submit(
            "user",
            &location,
            "/profile/display_name",
            display,
            "structured",
        ),
    ]
}
/// A thread's start message: its text and author.
fn start_message(channel: &str, ts: &str, user: &str, text: &str) -> Vec<Value> {
    let location = format!("slack:message/{channel}/{ts}");
    vec![
        submit("message", &location, "/text", text, "untrusted content"),
        submit("message", &location, "/user", user, "structured"),
    ]
}
/// A reply naming the thread it answers: its text, author and structured `/thread_ts`.
fn reply_message(channel: &str, ts: &str, thread_ts: &str, user: &str, text: &str) -> Vec<Value> {
    let location = format!("slack:message/{channel}/{ts}");
    vec![
        submit("message", &location, "/text", text, "untrusted content"),
        submit("message", &location, "/user", user, "structured"),
        submit("message", &location, "/thread_ts", thread_ts, "structured"),
    ]
}
/// A connector script that syncs the given evidence with complete coverage.
fn sync_script(evidence: Vec<Value>) -> Vec<Value> {
    let mut steps = vec![json!({"do": "initialize"})];
    steps.extend(evidence);
    steps.push(
        json!({"do": "send", "message": {"jsonrpc": "2.0", "method": "coverage.report",
        "params": {"status": "complete"}}}),
    );
    steps.push(json!({"do": "finish"}));
    steps
}

struct Env {
    home: TempDir,
}
impl Env {
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
        };
        let plugins = env.path("dev-plugins");
        // The scripted connector, bound to the real Slack extractor, allowed to submit the
        // chat evidence kinds in both scopes.
        let connector = plugins.join("test");
        fs::create_dir_all(&connector).unwrap();
        let mut manifest: toml::Table =
            include_str!("../../rastro-test-connector/rastro-plugin.toml")
                .parse()
                .unwrap();
        manifest.insert(
            "command".into(),
            support::test_connector().to_str().unwrap().into(),
        );
        manifest.insert("default_extractor".into(), "rastro-extractor-slack".into());
        let permissions = manifest["permissions"].as_table_mut().unwrap();
        permissions.insert(
            "submit".into(),
            toml::Value::Array(
                ["issue", "comment", "message", "user", "viewer"]
                    .iter()
                    .map(|k| toml::Value::String((*k).into()))
                    .collect(),
            ),
        );
        permissions.insert(
            "scopes".into(),
            toml::Value::Array(
                ["personal", "work"]
                    .iter()
                    .map(|s| toml::Value::String((*s).into()))
                    .collect(),
            ),
        );
        fs::write(connector.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
        // The real Slack extractor: it makes the thread's turns and claims the viewer's
        // account, which the core holds by the owner's Person.
        let extractor = plugins.join("rastro-extractor-slack");
        fs::create_dir_all(&extractor).unwrap();
        let mut manifest: toml::Table =
            include_str!("../../rastro-extractor-slack/rastro-plugin.toml")
                .parse()
                .unwrap();
        manifest.insert(
            "command".into(),
            support::connector("rastro-extractor-slack")
                .to_str()
                .unwrap()
                .into(),
        );
        fs::write(extractor.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
        fs::write(
            extractor.join("vocabulary.toml"),
            include_str!("../../rastro-extractor-slack/vocabulary.toml"),
        )
        .unwrap();
        // An extractor that declares no conversation.turns, for the refused opt-in.
        let plain = plugins.join("plain-extractor");
        fs::create_dir_all(&plain).unwrap();
        let mut manifest: toml::Table =
            include_str!("../../rastro-test-extractor/rastro-plugin.toml")
                .parse()
                .unwrap();
        manifest.insert("id".into(), "plain-extractor".into());
        manifest.insert(
            "command".into(),
            support::test_extractor().to_str().unwrap().into(),
        );
        fs::write(plain.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
        fs::write(
            plain.join("vocabulary.toml"),
            include_str!("../../rastro-test-extractor/vocabulary.toml"),
        )
        .unwrap();
        // An extractor that cites a message outside the thread, for the refused-turn skip.
        let liar = plugins.join("lying-extractor");
        fs::create_dir_all(&liar).unwrap();
        fs::write(liar.join("rastro-plugin.toml"), LIAR_MANIFEST).unwrap();
        let script = liar.join("extractor");
        fs::write(&script, liar_script()).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(env.path("turns.json"), liar_turns().to_string()).unwrap();
        // The judgment adapter.
        let adapter = plugins.join("test-adapter");
        fs::create_dir_all(&adapter).unwrap();
        fs::write(adapter.join("rastro-plugin.toml"), ADAPTER_MANIFEST).unwrap();
        let script = adapter.join("adapter");
        fs::write(&script, adapter_script()).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(env.path("response.json"), response().to_string()).unwrap();
        fs::create_dir_all(env.path("config")).unwrap();
        fs::write(
            env.path("config/config.toml"),
            "[commitment_adapter]\nplugin = \"test-adapter\"\n",
        )
        .unwrap();
        // One synced thread: the fixture's ask and promise, with complete coverage.
        env.script(&[
            json!({"do": "initialize"}),
            json!({"do": "fixture", "name": "chat_thread"}),
            json!({"do": "send", "message": {"jsonrpc": "2.0", "method": "coverage.report",
                "params": {"status": "complete"}}}),
            json!({"do": "finish"}),
        ]);
        env.ok(&["identity", "me"]);
        env
    }
    fn path(&self, name: &str) -> PathBuf {
        self.home.path().join(name)
    }
    fn script(&self, steps: &[Value]) {
        let lines: Vec<_> = steps.iter().map(Value::to_string).collect();
        fs::write(self.path("script.jsonl"), lines.join("\n")).unwrap();
    }
    /// Replaces the canned judgment answer the adapter gives.
    fn set_response(&self, response: Value) {
        fs::write(self.path("response.json"), response.to_string()).unwrap();
    }
    /// Rebinds the connector's sources to another installed extractor.
    fn bind_extractor(&self, extractor: &str) {
        let path = self.path("dev-plugins/test/rastro-plugin.toml");
        let mut manifest: toml::Table = fs::read_to_string(&path).unwrap().parse().unwrap();
        manifest.insert("default_extractor".into(), extractor.into());
        fs::write(path, manifest.to_string()).unwrap();
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(support::rastro());
        command
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.path("dev-plugins"))
            .env("RASTRO_TEST_SCRIPT", self.path("script.jsonl"))
            .args(args);
        command
    }
    fn run(&self, args: &[&str]) -> (bool, Value) {
        let out = self.command(args).arg("--json").output().unwrap();
        let value: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
            panic!(
                "{args:?}: {}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        });
        let schema: Value =
            serde_json::from_str(include_str!("../../../schema/operations-v2.schema.json"))
                .unwrap();
        jsonschema::validator_for(&schema)
            .unwrap()
            .validate(&value)
            .unwrap_or_else(|error| panic!("{error}: {value}"));
        (out.status.success(), value)
    }
    fn ok(&self, args: &[&str]) -> Value {
        let (ok, value) = self.run(args);
        assert!(ok, "{args:?}: {value}");
        value["result"].clone()
    }
    /// A tick: its human report, asserting it finished without a failed run or source.
    fn tick(&self) -> String {
        let out = self.command(&["tick"]).output().unwrap();
        assert!(
            out.status.success(),
            "tick: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }
    /// How many adapter processes were launched.
    fn launches(&self) -> usize {
        fs::read_to_string(self.path("launches"))
            .map(|text| text.lines().count())
            .unwrap_or(0)
    }
    /// The params of the judgment request the adapter was sent.
    fn request(&self) -> Value {
        let message: Value =
            serde_json::from_slice(&fs::read(self.path("request.json")).unwrap()).unwrap();
        message["params"].clone()
    }
    fn mcp(&self, scopes: &[&str], tool: &str, arguments: Value) -> Value {
        let mut command = self.command(&["mcp"]);
        for scope in scopes {
            command.args(["--scope", scope]);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut send = |message: Value| writeln!(stdin, "{message}").unwrap();
        send(
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": {"name": "test", "version": "1"}}}),
        );
        lines.next().unwrap().unwrap();
        send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": tool, "arguments": arguments}}));
        let reply: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        drop(stdin);
        let _ = child.wait();
        reply["result"]["structuredContent"].clone()
    }
}

/// S3 and S4: the opt-in is what sends a thread — a source added without it never reaches the
/// adapter, and the one added with it has its thread asked of the extractor, judged, and listed
/// as a `chat` source named by the channel source and the thread. S12 and S13: the parties are
/// identified by account — the owner's held account is `me`, and another author is that author's
/// account, never a display name.
// spec: A chat source's threads reach the adapter only after its opt-in
// spec: Chat parties are identified by account
#[test]
fn only_an_opted_in_chat_source_sends_its_threads_to_the_adapter() {
    let env = Env::new();
    let me = env.ok(&["identity", "me"])["person"]["identifier"]
        .as_str()
        .unwrap()
        .to_string();
    env.ok(&[
        "source",
        "add",
        "eng",
        "--connector",
        "test",
        "--scope",
        "work",
        "--extract-commitments",
    ]);
    env.ok(&[
        "source",
        "add",
        "lobby",
        "--connector",
        "test",
        "--scope",
        "work",
    ]);
    env.ok(&["sync"]);
    env.tick();
    // The adapter was launched once, for the opted-in source's thread: the extractor's turns,
    // one per message in post order, with the resolved mention and the owner's account.
    assert_eq!(
        env.launches(),
        1,
        "the non-opted-in source reached the adapter"
    );
    let request = env.request();
    assert_eq!(request["owner_speaker"], json!(OWNER));
    let turns = request["turns"].as_array().unwrap();
    assert_eq!(turns.len(), 2, "{request}");
    assert_eq!(turns[0]["speaker"], json!(ANA));
    assert_eq!(
        turns[0]["text"],
        json!("Felippe Bisca can you send me the RBAC doc?")
    );
    assert_eq!(turns[1]["speaker"], json!(OWNER));
    assert_eq!(turns[1]["text"], json!("I'll send it Friday."));
    // The list shows one chat source, named by the channel source and the thread, with the
    // hypotheses the run kept; the non-opted-in source has none.
    let list = env.ok(&["commitments"]);
    let sources = list["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 1, "{list}");
    let source = &sources[0];
    assert_eq!(source["kind"], json!("chat"));
    assert_eq!(source["name"], json!(format!("eng/{THREAD_TS}")));
    assert_eq!(source["scope"], json!("work"));
    assert_eq!(source["extraction"]["state"], json!("succeeded"));
    assert_eq!(source["coverage"]["state"], json!("found"));
    assert_eq!(source["coverage"]["turns_read"], json!(2));
    assert_eq!(source["coverage"]["turns_total"], json!(2));
    let hypotheses = source["hypotheses"].as_array().unwrap();
    assert_eq!(hypotheses.len(), 2, "{source}");
    assert_eq!(hypotheses[0]["kind"], json!("ask"));
    assert_eq!(hypotheses[1]["kind"], json!("promise"));
    assert_parties_identified_by_account(hypotheses, &me);
    assert_eq!(
        hypotheses[1]["quote"]["text"],
        json!("I'll send it Friday.")
    );
}

/// S12 and S13: the parties are identified by account — Ana's ask names her account as responsible
/// and `me` as recipient; the owner's promise names `me` as responsible, never a display name.
fn assert_parties_identified_by_account(hypotheses: &[Value], me: &str) {
    assert_eq!(hypotheses[0]["responsible"]["label"], json!(ANA));
    assert_eq!(hypotheses[0]["responsible"]["state"], json!("mapped"));
    assert_eq!(
        hypotheses[0]["responsible"]["identity"]["identifier"],
        json!(ANA)
    );
    assert_eq!(
        hypotheses[0]["recipient"]["identity"]["identifier"],
        json!(me)
    );
    assert_eq!(hypotheses[1]["responsible"]["state"], json!("mapped"));
    assert_eq!(
        hypotheses[1]["responsible"]["identity"]["identifier"],
        json!(me)
    );
}

/// S5: a reader limited to other scopes sees no chat source of the channel, and the read still
/// succeeds.
// spec: A chat source's threads reach the adapter only after its opt-in
#[test]
fn a_reader_limited_to_other_scopes_sees_no_chat_source() {
    let env = Env::new();
    env.ok(&[
        "source",
        "add",
        "eng",
        "--connector",
        "test",
        "--scope",
        "work",
        "--extract-commitments",
    ]);
    env.ok(&["sync", "eng"]);
    env.tick();
    // The owner sees the chat source.
    let list = env.ok(&["commitments"]);
    assert_eq!(list["sources"][0]["kind"], json!("chat"), "{list}");
    // A server scoped elsewhere sees none, and the read succeeds.
    let hidden = env.mcp(&["personal"], "commitments", json!({}));
    assert_eq!(hidden["result"]["sources"], json!([]), "{hidden}");
}

/// S18: an opt-in whose extractor does not declare `conversation.turns` fails naming the
/// extractor and the missing capability, and records nothing.
// spec: The core refuses turns it cannot trace to the thread
#[test]
fn an_opt_in_whose_extractor_lacks_turns_is_refused_and_records_nothing() {
    let env = Env::new();
    env.bind_extractor("plain-extractor");
    let (ok, value) = env.run(&[
        "source",
        "add",
        "eng",
        "--connector",
        "test",
        "--scope",
        "work",
        "--extract-commitments",
    ]);
    assert!(!ok, "{value}");
    let error = value["error"].as_str().unwrap();
    assert!(error.contains("plain-extractor"), "{error}");
    assert!(error.contains("conversation.turns"), "{error}");
    // Nothing is recorded: no source in the config, and no thread of it anywhere.
    let list = env.ok(&["source", "list"]);
    assert_eq!(list["sources"], json!([]), "{list}");
    let commitments = env.ok(&["commitments"]);
    assert_eq!(commitments["sources"], json!([]), "{commitments}");
}

/// A thread whose first run failed is extracted again on a later tick: the failed run never
/// buries it. The first tick has no adapter configured; once the owner configures one, the
/// next tick sends the thread and the list shows its hypotheses.
// spec: A chat source's threads reach the adapter only after its opt-in
#[test]
fn a_thread_whose_first_run_failed_is_extracted_again() {
    let env = Env::new();
    // No adapter configured yet.
    fs::write(env.path("config/config.toml"), "").unwrap();
    env.ok(&[
        "source",
        "add",
        "eng",
        "--connector",
        "test",
        "--scope",
        "work",
        "--extract-commitments",
    ]);
    env.ok(&["sync", "eng"]);
    env.tick();
    assert_eq!(env.launches(), 0);
    let list = env.ok(&["commitments"]);
    let sources = list["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 1, "{list}");
    assert_eq!(sources[0]["kind"], json!("chat"));
    assert_eq!(sources[0]["extraction"]["state"], json!("failed"));
    assert_eq!(
        sources[0]["extraction"]["cause"],
        json!("extractor_unconfigured")
    );
    // The owner configures the adapter; the next tick retries the failed thread. The source
    // stays registered: the adapter table joins the config the source add wrote.
    let mut config = fs::read_to_string(env.path("config/config.toml")).unwrap();
    config.push_str("\n[commitment_adapter]\nplugin = \"test-adapter\"\n");
    fs::write(env.path("config/config.toml"), config).unwrap();
    env.tick();
    assert_eq!(
        env.launches(),
        1,
        "the failed thread was never extracted again: {}",
        env.ok(&["commitments"])
    );
    let list = env.ok(&["commitments"]);
    let sources = list["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 1, "{list}");
    assert_eq!(sources[0]["name"], json!(format!("eng/{THREAD_TS}")));
    assert_eq!(sources[0]["extraction"]["state"], json!("succeeded"));
    assert_eq!(sources[0]["hypotheses"].as_array().unwrap().len(), 2);
}

/// S14: a candidate whose party is a bot turn is excluded and counted. The bot promises the owner
/// something, so the candidate names the owner on one side and is otherwise supported; the core
/// leaves it out because its responsible party is the bot's account, and counts the exclusion.
// spec: Chat parties are identified by account
#[test]
fn a_candidate_whose_party_is_a_bot_turn_is_excluded_and_counted() {
    let env = Env::new();
    let mut evidence = vec![viewer()];
    evidence.extend(author("U0OWNER", "Felippe Bisca", false));
    evidence.extend(author("U0BOT", "Deploy Bot", true));
    evidence.extend(start_message(
        "C0ENGCHAN",
        THREAD_TS,
        "U0BOT",
        "<@U0OWNER> I'll send you the report Friday.",
    ));
    evidence.extend(reply_message(
        "C0ENGCHAN",
        "1700000001.000200",
        THREAD_TS,
        "U0OWNER",
        "great",
    ));
    env.script(&sync_script(evidence));
    env.set_response(json!({"candidates": [
        {"turn": 1, "kind": "promise", "responsible": BOT, "recipient": OWNER,
         "promise": "send the owner the report", "stated_date": "Friday",
         "quote": "I'll send you the report Friday."}
    ]}));
    env.ok(&[
        "source",
        "add",
        "eng",
        "--connector",
        "test",
        "--scope",
        "work",
        "--extract-commitments",
    ]);
    env.ok(&["sync", "eng"]);
    env.tick();
    // The owner is in the thread, so it is extracted; the bot's candidate is excluded, not kept.
    assert_eq!(
        env.launches(),
        1,
        "the thread the owner is in reached the adapter"
    );
    let list = env.ok(&["commitments"]);
    let source = &list["sources"][0];
    assert_eq!(
        source["extraction"]["state"],
        json!("succeeded"),
        "{source}"
    );
    assert_eq!(source["extraction"]["excluded"], json!(1), "{source}");
    assert_eq!(source["hypotheses"], json!([]), "{source}");
}

/// S15: a thread the owner is not in is skipped without a failed run and counted in coverage. No
/// speaker or mentioned account is held by the owner's Person, so the adapter is never asked, and
/// the list names the gap with the reason instead of failing.
// spec: A thread that cannot be extracted is skipped and counted
#[test]
fn a_thread_the_owner_is_not_in_is_skipped_and_counted() {
    let env = Env::new();
    let mut evidence = vec![viewer()];
    evidence.extend(author("U0ANA", "Ana Duarte", false));
    evidence.extend(author("U0BOB", "Bob Lima", false));
    evidence.extend(start_message(
        "C0ENGCHAN",
        THREAD_TS,
        "U0ANA",
        "can you send the doc?",
    ));
    evidence.extend(reply_message(
        "C0ENGCHAN",
        "1700000001.000200",
        THREAD_TS,
        "U0BOB",
        "I'll send it Friday.",
    ));
    env.script(&sync_script(evidence));
    env.ok(&[
        "source",
        "add",
        "eng",
        "--connector",
        "test",
        "--scope",
        "work",
        "--extract-commitments",
    ]);
    env.ok(&["sync", "eng"]);
    env.tick();
    assert_eq!(
        env.launches(),
        0,
        "the owner-absent thread reached the adapter"
    );
    let list = env.ok(&["commitments"]);
    let sources = list["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 1, "{list}");
    let source = &sources[0];
    assert_eq!(source["name"], json!(format!("eng/{THREAD_TS}")));
    assert_eq!(
        source["extraction"]["state"],
        json!("succeeded"),
        "{source}"
    );
    assert_eq!(source["extraction"]["cause"], json!(null), "{source}");
    assert_eq!(source["coverage"]["turns_read"], json!(0), "{source}");
    assert_eq!(source["coverage"]["turns_total"], json!(2), "{source}");
    assert_eq!(
        source["coverage"]["reason"],
        json!("owner absent: no speaker or mentioned account is held by the owner's Person"),
        "{source}"
    );
    assert_eq!(source["hypotheses"], json!([]), "{source}");
}

/// S16: a thread whose turns exceed the transcript cap is skipped without a failed run and counted
/// in coverage. The owner is in it, but its transcript is larger than the core reads, so the core
/// skips it from the evidence it holds — before asking the extractor, whose one-line turn answer
/// the default host line cap could not carry — and the adapter is never asked. The tick runs under
/// the default line cap, so this proves the skip on a real install, not a raised debug cap.
// spec: A thread that cannot be extracted is skipped and counted
#[test]
fn a_thread_over_the_transcript_cap_is_skipped_and_counted() {
    let env = Env::new();
    let mut evidence = vec![viewer()];
    evidence.extend(author("U0OWNER", "Felippe Bisca", false));
    evidence.extend(author("U0ANA", "Ana Duarte", false));
    evidence.extend(start_message(
        "C0ENGCHAN",
        THREAD_TS,
        "U0ANA",
        "<@U0OWNER> can you send the doc?",
    ));
    // Five replies whose text alone is over the 4 MiB transcript cap.
    let big = "x".repeat(900_000);
    for n in 1..=5 {
        evidence.extend(reply_message(
            "C0ENGCHAN",
            &format!("170000000{n}.000200"),
            THREAD_TS,
            "U0OWNER",
            &big,
        ));
    }
    env.script(&sync_script(evidence));
    env.ok(&[
        "source",
        "add",
        "eng",
        "--connector",
        "test",
        "--scope",
        "work",
        "--extract-commitments",
    ]);
    env.ok(&["sync", "eng"]);
    env.tick();
    assert_eq!(env.launches(), 0, "the oversize thread reached the adapter");
    let list = env.ok(&["commitments"]);
    let source = &list["sources"][0];
    assert_eq!(
        source["extraction"]["state"],
        json!("succeeded"),
        "{source}"
    );
    assert_eq!(source["extraction"]["cause"], json!(null), "{source}");
    assert_eq!(source["coverage"]["turns_read"], json!(0), "{source}");
    assert_eq!(source["coverage"]["turns_total"], json!(6), "{source}");
    assert_eq!(
        source["coverage"]["reason"],
        json!("too large: the thread's turns exceed the transcript cap"),
        "{source}"
    );
    assert_eq!(source["hypotheses"], json!([]), "{source}");
}

/// S17: a turn that cites a message outside the thread is refused, and the thread is skipped
/// without a failed run and counted in coverage. The owner is in the thread and it is small, so
/// the refused citation is the only reason the adapter is never asked.
// spec: The core refuses turns it cannot trace to the thread
#[test]
fn a_turn_citing_outside_the_thread_is_refused_and_the_thread_skipped() {
    let env = Env::new();
    env.ok(&[
        "source",
        "add",
        "eng",
        "--connector",
        "test",
        "--scope",
        "work",
        "--extract-commitments",
    ]);
    // Sync with the real extractor so the owner's account is held, then rebind to the one that
    // answers with a turn citing another channel.
    env.ok(&["sync", "eng"]);
    env.bind_extractor("lying-extractor");
    env.tick();
    assert_eq!(
        env.launches(),
        0,
        "the thread with a refused turn reached the adapter"
    );
    let list = env.ok(&["commitments"]);
    let sources = list["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 1, "{list}");
    let source = &sources[0];
    assert_eq!(source["name"], json!(format!("eng/{THREAD_TS}")));
    assert_eq!(
        source["extraction"]["state"],
        json!("succeeded"),
        "{source}"
    );
    assert_eq!(source["extraction"]["cause"], json!(null), "{source}");
    assert_eq!(source["coverage"]["turns_read"], json!(0), "{source}");
    assert_eq!(source["coverage"]["turns_total"], json!(2), "{source}");
    assert_eq!(
        source["coverage"]["reason"],
        json!("a turn was refused: it does not cite a message of the thread"),
        "{source}"
    );
    assert_eq!(source["hypotheses"], json!([]), "{source}");
}

/// The owner is in a thread they are mentioned in even when they never speak (decisions D13): a
/// thread between two others that mentions the owner is extracted, not skipped as owner-absent.
// spec: A thread that cannot be extracted is skipped and counted
#[test]
fn a_thread_that_mentions_the_owner_is_not_skipped_as_owner_absent() {
    let env = Env::new();
    let mut evidence = vec![viewer()];
    evidence.extend(author("U0OWNER", "Felippe Bisca", false));
    evidence.extend(author("U0ANA", "Ana Duarte", false));
    evidence.extend(author("U0BOB", "Bob Lima", false));
    evidence.extend(start_message(
        "C0ENGCHAN",
        THREAD_TS,
        "U0ANA",
        "<@U0OWNER> can you send the doc?",
    ));
    evidence.extend(reply_message(
        "C0ENGCHAN",
        "1700000001.000200",
        THREAD_TS,
        "U0BOB",
        "I'll send it Friday.",
    ));
    env.script(&sync_script(evidence));
    env.set_response(json!({"candidates": [
        {"turn": 1, "kind": "ask", "responsible": ANA, "recipient": OWNER,
         "promise": "send the doc", "stated_date": null, "quote": "can you send the doc?"}
    ]}));
    env.ok(&[
        "source",
        "add",
        "eng",
        "--connector",
        "test",
        "--scope",
        "work",
        "--extract-commitments",
    ]);
    env.ok(&["sync", "eng"]);
    env.tick();
    // The mention holds the owner in the thread, so the adapter is asked; it is not skipped.
    assert_eq!(
        env.launches(),
        1,
        "the thread mentioning the owner was skipped"
    );
    let list = env.ok(&["commitments"]);
    let source = &list["sources"][0];
    assert_eq!(
        source["extraction"]["state"],
        json!("succeeded"),
        "{source}"
    );
    assert_eq!(source["coverage"]["reason"], json!(null), "{source}");
    assert_eq!(source["coverage"]["turns_read"], json!(2), "{source}");
}
