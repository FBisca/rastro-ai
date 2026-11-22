//! The extractor's protocol boundary: the real binary answers `conversation.turns` after
//! `initialize`, with the contract's response, and exits when the core closes the wire.
#[path = "../../rastro-cli/tests/support/binaries.rs"]
mod binaries;
use rastro_plugin_sdk::{
    CONVERSATION_TURNS, ConversationTurnsResponse, Envelope, ExtractionBinding,
    ExtractorInitializeParams, InitializeResult, PROTOCOL_VERSION, Role, SourceScope,
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::OnceLock,
};

/// The extractor under test, as a private copy no concurrent build can rewrite.
fn extractor() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        binaries::private_copy(Path::new(env!("CARGO_BIN_EXE_rastro-extractor-slack")))
    })
}

const START: &str = "1700000000.000100";
const REPLY: &str = "1700000060.000200";
const ANA: &str = "U0ANA";
const BOT: &str = "U0BOT";
const OWNER: &str = "U0OWNER";

fn item(id: i64, location: &str, field: &str, quote: &str, trust: &str) -> Value {
    json!({
        "evidence_id": id,
        "source": "s",
        "scope": "work",
        "location": location,
        "span": {"field": field, "start": 0, "end": quote.len(), "quote": quote},
        "captured_at": "2026-11-21T10:00:00.000Z",
        "trust_class": trust,
    })
}
fn message(id: i64, ts: &str, author: &str, text: &str, thread_ts: Option<&str>) -> Vec<Value> {
    let location = format!("slack:message/C0ENG/{ts}");
    let mut items = vec![
        item(id, &location, "/text", text, "untrusted content"),
        item(id + 1, &location, "/user", author, "structured"),
    ];
    if let Some(thread) = thread_ts {
        items.push(item(id + 2, &location, "/thread_ts", thread, "structured"));
    }
    items
}
fn user(id: i64, user_id: &str, display: &str, is_bot: bool) -> Vec<Value> {
    let location = format!("slack:user/{user_id}");
    vec![
        item(id, &location, "/id", user_id, "structured"),
        item(
            id + 1,
            &location,
            "/is_bot",
            if is_bot { "true" } else { "false" },
            "structured",
        ),
        item(
            id + 2,
            &location,
            "/profile/display_name",
            display,
            "structured",
        ),
    ]
}

struct Wire {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}
impl Wire {
    fn start() -> Self {
        let mut child = Command::new(extractor())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("extractor binary starts");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }
    fn send(&mut self, message: &Envelope) {
        let line = serde_json::to_string(message).unwrap();
        self.stdin.write_all(line.as_bytes()).unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
    }
    fn read(&mut self) -> Envelope {
        let mut line = String::new();
        let n = self.stdout.read_line(&mut line).unwrap();
        assert!(n > 0, "extractor closed the wire before answering");
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("not an envelope: {line:?}: {e}"))
    }
    fn finish(mut self) {
        drop(self.stdin);
        let status = self.child.wait().unwrap();
        assert!(status.success(), "extractor exited with {status}");
    }
}

/// spec: A chat thread's turns come from its source's extractor
/// WHEN the core initializes the extractor and asks `conversation.turns` for a thread of two
/// messages, THEN the response carries one turn per message with its speaker account, its
/// kind and its citation, and the extractor exits cleanly when the wire closes
#[test]
fn the_extractor_answers_conversation_turns_over_the_wire() {
    let mut wire = Wire::start();
    wire.send(
        &Envelope::request(
            1,
            "initialize",
            ExtractorInitializeParams {
                protocol: PROTOCOL_VERSION.into(),
                manifest_id: "rastro-extractor-slack".into(),
                role: Role::Extractor,
                run: ExtractionBinding {
                    run_id: 1,
                    extractor_version: "0.1.0".into(),
                    pairs: vec![SourceScope {
                        source: "s".into(),
                        scope: "work".into(),
                    }],
                },
            },
        )
        .unwrap(),
    );
    let hello: InitializeResult = wire.read().result().unwrap();
    assert_eq!(hello.protocol, PROTOCOL_VERSION);

    let mut messages = user(10, ANA, "Ana", false);
    messages.extend(user(20, BOT, "Deploy Bot", true));
    messages.push(item(30, "slack:viewer", "/id", OWNER, "structured"));
    messages.extend(message(40, START, ANA, "hi", None));
    let mut reply = message(50, REPLY, BOT, "deployed", Some(START));
    reply.push(item(
        53,
        &format!("slack:message/C0ENG/{REPLY}"),
        "/subtype",
        "bot_message",
        "structured",
    ));
    messages.extend(reply);
    wire.send(
        &Envelope::request(
            7,
            CONVERSATION_TURNS,
            json!({"source": "s", "scope": "work", "thread": START, "messages": messages}),
        )
        .unwrap(),
    );
    let answer = wire.read();
    assert_eq!(answer.id, Some(json!(7)), "{answer:?}");
    let response: ConversationTurnsResponse = answer.result().unwrap();
    assert_eq!(response.turns.len(), 2, "{response:?}");
    let start = &response.turns[0];
    assert_eq!(
        start.speaker,
        rastro_plugin_sdk::ConversationSpeaker::Account(rastro_plugin_sdk::EntityRef {
            identifier: format!("slack:user/{ANA}"),
            entity_type: "slack:Account".into(),
        })
    );
    assert_eq!(start.kind, rastro_plugin_sdk::ConversationTurnKind::Person);
    assert_eq!(start.text, "hi");
    assert_eq!(
        start.cites.as_deref(),
        Some(format!("slack:message/C0ENG/{START}").as_str())
    );
    let bot = &response.turns[1];
    assert_eq!(
        bot.speaker,
        rastro_plugin_sdk::ConversationSpeaker::Account(rastro_plugin_sdk::EntityRef {
            identifier: format!("slack:user/{BOT}"),
            entity_type: "slack:Bot".into(),
        })
    );
    assert_eq!(bot.kind, rastro_plugin_sdk::ConversationTurnKind::Bot);
    assert_eq!(
        bot.cites.as_deref(),
        Some(format!("slack:message/C0ENG/{REPLY}").as_str())
    );
    wire.finish();
}
