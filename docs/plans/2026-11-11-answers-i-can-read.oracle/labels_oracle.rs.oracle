//! Oracle for plan `answers-i-can-read` (labels): black-box checks of S1, S3, S4, S9, S10 and
//! S11 through the `rastro` binary, `--json` and `rastro mcp`. Labels are read as JSON values,
//! so the checks need no new Rust types.
#![allow(non_snake_case)]
mod support;
use chrono::{DateTime, Utc};
use rastro_core::claims::{self, Entity, Object, ObservedClaim, Support};
use rastro_core::{Appended, Evidence, Scope, Source, Storage, TrustClass};
use rastro_storage_sqlite::SqliteStorage;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{OnceLock, mpsc},
    time::Duration,
};
use tempfile::TempDir;

const PR: &str = "https://github.com/o/p/pull/1";
const ISSUE: &str = "https://github.com/o/p/issues/2";
const REPO: &str = "https://github.com/o/p";
const COMMENT: &str = "https://github.com/o/p/pull/1#issuecomment-5";
const ACCOUNT: &str = "github:account/42";
const LINEAR_ISSUE: &str = "0b5a8c1e-0000-4000-8000-0000000000a1";
const LINEAR_USER: &str = "0b5a8c1e-0000-4000-8000-000000000001";

fn time() -> DateTime<Utc> {
    "2026-11-01T00:00:00Z".parse().unwrap()
}
fn later() -> DateTime<Utc> {
    time() + chrono::Duration::days(1)
}

/// The real Linear extractor, built next to `rastro` and copied.
fn linear_extractor() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let status = Command::new(cargo)
            .args(["build", "--quiet", "-p", "rastro-extractor-linear"])
            .status()
            .unwrap();
        assert!(status.success(), "building the Linear extractor failed");
        let original =
            Path::new(env!("CARGO_BIN_EXE_rastro")).with_file_name("rastro-extractor-linear");
        support::binaries::private_copy(&original)
    })
}

struct Env {
    home: TempDir,
}
impl Env {
    /// A home with GitHub source `project` (scope `work`), seeded through storage.
    fn new() -> Self {
        let env = Self {
            home: TempDir::new().unwrap(),
        };
        fs::create_dir_all(env.home.path().join("dev-plugins")).unwrap();
        env.ok(&[
            "source",
            "add",
            "project",
            "--connector",
            "github",
            "--repo",
            "o/p",
            "--scope",
            "work",
        ]);
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
    fn output(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }
    /// The whole `--json` envelope of a command that must succeed, checked against the schema.
    fn envelope(&self, args: &[&str]) -> Value {
        let output = self.output(&[args, &["--json"]].concat());
        assert!(
            output.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let schema: Value =
            serde_json::from_str(include_str!("../../../schema/operations-v2.schema.json"))
                .unwrap();
        jsonschema::validator_for(&schema)
            .unwrap()
            .validate(&value)
            .unwrap_or_else(|e| panic!("{e}: {value}"));
        value
    }
    fn ok(&self, args: &[&str]) -> Value {
        self.envelope(args)["result"].clone()
    }
    fn text(&self, args: &[&str]) -> String {
        let output = self.output(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn db(&self) -> SqliteStorage {
        SqliteStorage::open(&self.home.path().join("data/rastro.sqlite3")).unwrap()
    }
    fn source() -> Source {
        Source::github("project".into(), "o/p".into(), Scope("work".into()))
    }
    /// Appends one observation of `field` at `location`; returns its evidence id.
    fn observe(
        &self,
        location: &str,
        field: &str,
        quote: &str,
        trust_class: TrustClass,
        captured: DateTime<Utc>,
    ) -> i64 {
        let source = Self::source();
        let mut db = self.db();
        let run = db
            .start_run(&source, ("fixture", "1"), captured, (captured, captured))
            .unwrap();
        let appended = db
            .append_evidence(
                run,
                &Evidence {
                    source: source.name.clone(),
                    scope: source.scope.clone(),
                    location: location.into(),
                    field: field.into(),
                    start: 0,
                    end: quote.len() as u64,
                    quote: quote.into(),
                    trust_class,
                    captured_at: captured,
                    source_updated_at: Some(captured),
                    original: None,
                },
            )
            .unwrap();
        match appended {
            Appended::New(id) | Appended::Unchanged(id) => id,
        }
    }
    /// Records one observed claim about `subject`, supported by its own structured evidence.
    fn claim(
        &self,
        subject: (&str, &str),
        predicate: &str,
        object: Object,
        captured: DateTime<Utc>,
    ) -> String {
        let quote = match &object {
            Object::Literal { value, .. } => value.clone(),
            Object::Entity { identifier, .. } => identifier.clone(),
        };
        let field = format!("/fixture/{predicate}");
        let evidence_id = self.observe(subject.0, &field, &quote, TrustClass::Structured, captured);
        let source = Self::source();
        let mut db = self.db();
        let run = db
            .start_run(&source, ("fixture", "1"), captured, (captured, captured))
            .unwrap();
        let e = Evidence {
            source: source.name.clone(),
            scope: source.scope.clone(),
            location: subject.0.into(),
            field: field.clone(),
            start: 0,
            end: quote.len() as u64,
            quote,
            trust_class: TrustClass::Structured,
            captured_at: captured,
            source_updated_at: Some(captured),
            original: None,
        };
        let entity = Entity::new(subject.0.into(), subject.1.into());
        let id = claims::claim_id(&entity.id, predicate, &object);
        db.record_claims(
            run,
            ("fixture", "1"),
            &[ObservedClaim {
                id: id.clone(),
                subject: entity,
                predicate: predicate.into(),
                object,
                scalar: predicate == "code:state" || predicate == claims::TYPE_PREDICATE,
                type_ancestors: Vec::new(),
                supports: BTreeSet::from([Support {
                    portable_key: claims::portable_evidence_key(&e),
                    evidence_id,
                    field,
                    start: 0,
                    end: e.end,
                    extraction_rule: "fixture-rule".into(),
                }]),
            }],
            None,
        )
        .unwrap();
        id
    }
    /// Types `identifier` as `entity_type` with a `rastro:type` claim, as an extractor does.
    fn typed(&self, identifier: &str, entity_type: &str) {
        let object = Object::Literal {
            value: entity_type.into(),
            datatype: claims::TYPE_DATATYPE.into(),
        };
        self.claim(
            (identifier, entity_type),
            claims::TYPE_PREDICATE,
            object,
            time(),
        );
    }
    /// The claim rows `about <entity>` returns.
    fn about(&self, entity: &str) -> Vec<Value> {
        self.ok(&["about", entity, "--limit", "50"])["claims"]
            .as_array()
            .unwrap_or_else(|| panic!("no claims about {entity}"))
            .clone()
    }
}

fn literal(value: &str) -> Object {
    Object::Literal {
        value: value.into(),
        datatype: "string".into(),
    }
}
fn entity(identifier: &str, entity_type: &str) -> Object {
    let e = Entity::new(identifier.into(), entity_type.into());
    Object::Entity {
        id: e.id,
        identifier: e.identifier,
        entity_type: e.entity_type,
    }
}
fn row<'a>(rows: &'a [Value], id: &str) -> &'a Value {
    rows.iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("claim {id} missing from {rows:?}"))
}
/// A side's label: the key must be present (null or an object), not merely absent.
fn label<'a>(row: &'a Value, side: &str) -> &'a Value {
    let key = format!("{side}_label");
    row.get(&key)
        .unwrap_or_else(|| panic!("claim row has no `{key}` field: {row}"))
}
fn same_instant(value: &Value, expected: DateTime<Utc>, context: &Value) {
    let found: DateTime<Utc> = value
        .as_str()
        .unwrap_or_else(|| panic!("captured_at is not a string: {context}"))
        .parse()
        .unwrap_or_else(|e| panic!("captured_at is not RFC 3339 ({e}): {context}"));
    assert_eq!(found, expected, "captured_at: {context}");
}

/// A PR with its title, state, repository, author and a comment, plus an issue with a title
/// and no state. Repository, comment and account carry title- or name-shaped evidence at their
/// own locations, which must still give no label (D2).
struct World {
    title_id: i64,
    issue_title_id: i64,
    state: String,
    in_repo: String,
    author: String,
    comment_author: String,
    issue_author: String,
}
fn world(env: &Env) -> World {
    let account = entity(ACCOUNT, "code:Account");
    for (identifier, entity_type) in [
        (PR, "code:PullRequest"),
        (ISSUE, "code:Issue"),
        (REPO, "code:Repository"),
        (COMMENT, "code:Comment"),
        (ACCOUNT, "code:Account"),
    ] {
        env.typed(identifier, entity_type);
    }
    let title_id = env.observe(
        PR,
        "/title",
        "Fix the login flow",
        TrustClass::UntrustedContent,
        time(),
    );
    let issue_title_id = env.observe(
        ISSUE,
        "/title",
        "Docs are stale",
        TrustClass::UntrustedContent,
        time(),
    );
    env.observe(
        REPO,
        "/title",
        "Repository title",
        TrustClass::UntrustedContent,
        time(),
    );
    env.observe(
        COMMENT,
        "/title",
        "Comment title",
        TrustClass::UntrustedContent,
        time(),
    );
    env.observe(
        ACCOUNT,
        "/name",
        "Ana Silva",
        TrustClass::Structured,
        time(),
    );
    let pr = (PR, "code:PullRequest");
    env.claim(pr, "code:state", literal("open"), time());
    // The current state is the later observation; the earlier one is superseded.
    let state = env.claim(pr, "code:state", literal("closed"), later());
    let in_repo = env.claim(
        pr,
        "code:in_repository",
        entity(REPO, "code:Repository"),
        time(),
    );
    let author = env.claim(pr, "code:author", account.clone(), time());
    let comment_author = env.claim(
        (COMMENT, "code:Comment"),
        "code:author",
        account.clone(),
        time(),
    );
    let issue_author = env.claim((ISSUE, "code:Issue"), "code:author", account, time());
    World {
        title_id,
        issue_title_id,
        state,
        in_repo,
        author,
        comment_author,
        issue_author,
    }
}

/// Control: `about` over seeded storage still answers, before and after the slice.
#[test]
fn control_about_still_returns_claims() {
    let env = Env::new();
    let w = world(&env);
    let rows = env.about(PR);
    let state = row(&rows, &w.state);
    assert_eq!(state["predicate"], "code:state");
    assert_eq!(state["subject"]["identifier"], PR);
    assert_eq!(state["object"]["value"], "closed");
    assert_eq!(state["status"], "observed");
    row(&rows, &w.in_repo);
    row(&rows, &w.author);
}

/// S1: a GitHub PR side carries its `/title` label with every cited field
#[test]
fn S1_a_github_pr_side_carries_its_cited_title_label() {
    let env = Env::new();
    let w = world(&env);
    let rows = env.about(PR);
    for id in [&w.state, &w.in_repo, &w.author] {
        let row = row(&rows, id);
        let label = label(row, "subject");
        assert!(label.is_object(), "subject_label is not an object: {row}");
        assert_eq!(label["text"], "Fix the login flow", "{row}");
        assert_eq!(label["url"], PR, "{row}");
        assert_eq!(label["evidence_id"], json!(w.title_id), "{row}");
        assert_eq!(label["field"], "/title", "{row}");
        same_instant(&label["captured_at"], time(), row);
        assert_eq!(label["trust_class"], "untrusted content", "{row}");
        assert_eq!(label["clipped"], false, "{row}");
    }
    // The object side of an incoming claim is labelled too.
    let incoming = env.about(ACCOUNT);
    let issue_side = label(row(&incoming, &w.issue_author), "subject");
    assert_eq!(issue_side["text"], "Docs are stale", "{issue_side}");
    assert_eq!(issue_side["evidence_id"], json!(w.issue_title_id));
    assert_eq!(issue_side["url"], ISSUE);
}

/// S4: repositories, comments and GitHub accounts get a null label, even with title- or
/// name-shaped evidence at their location
#[test]
fn S4_repository_comment_and_account_sides_have_a_null_label() {
    let env = Env::new();
    let w = world(&env);
    let rows = env.about(PR);
    let repo = row(&rows, &w.in_repo);
    assert_eq!(*label(repo, "object"), Value::Null, "repository: {repo}");
    let author = row(&rows, &w.author);
    assert_eq!(*label(author, "object"), Value::Null, "account: {author}");
    let incoming = env.about(ACCOUNT);
    let comment = row(&incoming, &w.comment_author);
    assert_eq!(
        *label(comment, "subject"),
        Value::Null,
        "comment: {comment}"
    );
    assert_eq!(*label(comment, "object"), Value::Null, "account: {comment}");
    // Not a blanket null: the PR side of the same answer is labelled.
    assert!(label(row(&incoming, &w.author), "subject").is_object());
}

/// Installs the Linear connector's manifest (run by the test connector) and the real Linear
/// extractor, adds source `lin`, and syncs one issue created by one user.
fn linear(env: &Env) {
    let plugins = env.home.path().join("dev-plugins");
    let connector = plugins.join("linear");
    fs::create_dir_all(&connector).unwrap();
    let mut manifest: toml::Table =
        include_str!("../../rastro-connector-linear/rastro-plugin.toml")
            .parse()
            .unwrap();
    manifest.insert(
        "command".into(),
        support::test_connector().to_str().unwrap().into(),
    );
    manifest["permissions"]["env"]
        .as_array_mut()
        .unwrap()
        .push("RASTRO_TEST_SCRIPT".into());
    fs::write(connector.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
    let extractor = plugins.join("rastro-extractor-linear");
    fs::create_dir_all(&extractor).unwrap();
    let mut manifest: toml::Table =
        include_str!("../../rastro-extractor-linear/rastro-plugin.toml")
            .parse()
            .unwrap();
    manifest.insert(
        "command".into(),
        linear_extractor().to_str().unwrap().into(),
    );
    fs::write(extractor.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
    fs::write(
        extractor.join("vocabulary.toml"),
        include_str!("../../rastro-extractor-linear/vocabulary.toml"),
    )
    .unwrap();
    env.ok(&[
        "source",
        "add",
        "lin",
        "--connector",
        "linear",
        "--scope",
        "work",
    ]);
    let user = format!("linear:user/{LINEAR_USER}");
    let issue = format!("linear:issue/{LINEAR_ISSUE}");
    let observations = [
        ("user", &user, "/id", LINEAR_USER, "structured"),
        ("user", &user, "/name", "Ana Silva", "structured"),
        ("user", &user, "/displayName", "ana", "structured"),
        ("user", &user, "/email", "ana@example.com", "structured"),
        ("user", &user, "/active", "true", "structured"),
        ("issue", &issue, "/id", LINEAR_ISSUE, "structured"),
        (
            "issue",
            &issue,
            "/title",
            "Ship the labels",
            "untrusted content",
        ),
        ("issue", &issue, "/state/name", "In Progress", "structured"),
        ("issue", &issue, "/state/type", "started", "structured"),
        ("issue", &issue, "/creator/id", LINEAR_USER, "structured"),
    ];
    let mut steps = vec![json!({"do": "initialize"})];
    for (kind, location, field, quote, trust_class) in observations {
        steps.push(json!({"do": "send", "message": {"jsonrpc": "2.0",
            "method": "evidence.submit", "params": {"kind": kind, "location": location,
            "field": field, "start": 0, "end": quote.len(), "quote": quote,
            "trust_class": trust_class, "source_updated_at": "2026-11-01T00:00:00Z"}}}));
    }
    steps.push(json!({"do": "send", "message": {"jsonrpc": "2.0",
        "method": "coverage.report", "params": {"status": "complete", "selection": "all"}}}));
    steps.push(json!({"do": "finish", "status": "ok", "submitted": observations.len()}));
    let lines: Vec<_> = steps.iter().map(Value::to_string).collect();
    fs::write(env.home.path().join("script.jsonl"), lines.join("\n")).unwrap();
    let synced = env.ok(&["sync", "lin"]);
    assert_eq!(synced["runs"][0]["status"], "ok", "{synced}");
    assert!(
        synced["runs"][0]["extraction"].is_object(),
        "the Linear extractor did not run: {synced}"
    );
}

/// S9 (and S3): `label.state` holds the current observed `code:state` of a PR and the
/// `tracker:state` the real Linear extractor reads from `/state/name`; an entity with no state
/// claim has a null state. A Linear issue's label comes from `/title` with a null url; a
/// Linear user's from `/name`, not `/displayName`.
#[test]
fn S9_label_state_holds_the_current_code_or_tracker_state() {
    let env = Env::new();
    let w = world(&env);
    linear(&env);
    let identifier = format!("linear:issue/{LINEAR_ISSUE}");
    let linear_rows = env.about(&identifier);
    let own: Vec<&Value> = linear_rows
        .iter()
        .filter(|r| r["subject"]["identifier"] == identifier)
        .collect();
    let creator = own
        .iter()
        .find(|r| r["predicate"] == "tracker:creator")
        .unwrap_or_else(|| panic!("no tracker:creator claim: {linear_rows:?}"));

    let rows = env.about(PR);
    let pr = label(row(&rows, &w.state), "subject");
    assert!(pr.is_object(), "no PR label: {rows:?}");
    assert_eq!(
        pr["state"], "closed",
        "the current state, not the superseded one: {pr}"
    );
    let incoming = env.about(ACCOUNT);
    let issue = label(row(&incoming, &w.issue_author), "subject");
    assert!(issue.is_object(), "no issue label: {incoming:?}");
    assert_eq!(
        issue.get("state"),
        Some(&Value::Null),
        "issue without state: {issue}"
    );

    for row in &own {
        let label = label(row, "subject");
        assert!(label.is_object(), "no Linear issue label: {row}");
        assert_eq!(label["text"], "Ship the labels", "{row}");
        assert_eq!(label["field"], "/title", "{row}");
        assert_eq!(label["url"], Value::Null, "{row}");
        assert_eq!(label["trust_class"], "untrusted content", "{row}");
        assert_eq!(label["state"], "In Progress", "tracker:state: {row}");
    }
    let user = label(creator, "object");
    assert!(user.is_object(), "no Linear user label: {creator}");
    assert_eq!(user["text"], "Ana Silva", "{creator}");
    assert_eq!(user["field"], "/name", "{creator}");
    assert_eq!(user.get("state"), Some(&Value::Null), "{creator}");
}

/// S10: CLI text shows the label after the id, control chars escaped; `--json` carries the
/// labels and equals the MCP result
#[test]
fn S10_cli_text_shows_the_escaped_label_after_the_id_and_json_equals_mcp() {
    let env = Env::new();
    env.observe(
        PR,
        "/title",
        "Login \u{1b}[31mfails\nbadly",
        TrustClass::UntrustedContent,
        time(),
    );
    env.typed(PR, "code:PullRequest");
    let state = env.claim(
        (PR, "code:PullRequest"),
        "code:state",
        literal("open"),
        time(),
    );
    let text = env.text(&["about", PR]);
    assert!(
        !text.contains('\u{1b}'),
        "a raw escape reached the terminal: {text:?}"
    );
    let line = text
        .lines()
        .find(|l| l.contains(&state))
        .unwrap_or_else(|| panic!("no line for claim {state}: {text}"));
    let id_at = line.find(PR).unwrap_or_else(|| panic!("no id: {line}"));
    let label_at = line
        .find("Login ")
        .unwrap_or_else(|| panic!("no label text on the claim line: {line}"));
    assert!(label_at > id_at, "the label is not after the id: {line}");
    for part in ["fails", "badly", "\\u{1b}"] {
        assert!(
            line.contains(part),
            "missing {part:?} (escaped) on the line: {line}"
        );
    }

    let cli = env.envelope(&["about", PR]);
    let row = row(cli["result"]["claims"].as_array().unwrap(), &state);
    let label = label(row, "subject");
    assert_eq!(label["text"], "Login \u{1b}[31mfails\nbadly", "{row}");
    let mut session = Mcp::start(env.command(&["mcp"]));
    let call = session.request(
        "tools/call",
        json!({"name": "about", "arguments": {"entity": PR}}),
    );
    // The claims, labels included, are equal. (Coverage of the fixture's unfinished runs is
    // stamped at read time, so the whole envelope differs by `checked_at`.)
    assert_eq!(
        call["result"]["structuredContent"]["result"]["claims"], cli["result"]["claims"],
        "{call}"
    );
}

/// S11: the MCP `about` and `why` descriptions say label text is untrusted data
#[test]
fn S11_mcp_about_and_why_descriptions_mark_label_text_untrusted() {
    let env = Env::new();
    let mut session = Mcp::start(env.command(&["mcp"]));
    let listed = session.request("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    for name in ["about", "why"] {
        let tool = tools
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("no {name} tool: {listed}"));
        let description = tool["description"].as_str().unwrap().to_lowercase();
        assert!(
            description.contains("label") && description.contains("untrusted"),
            "{name}: {description}"
        );
    }
}

/// A `rastro mcp` over stdio, initialized.
struct Mcp {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    lines: mpsc::Receiver<String>,
    next: u64,
}
impl Mcp {
    fn start(mut command: Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = sender.send(line);
            }
        });
        let mut session = Self {
            child,
            stdin,
            lines,
            next: 1,
        };
        session.request(
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {},
                   "clientInfo": {"name": "oracle", "version": "1.0"}}),
        );
        session.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        session
    }
    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(30))
                .expect("the MCP server printed nothing within 30 s");
            let message: Value = serde_json::from_str(&line).unwrap();
            if message["id"] == json!(id) && message.get("method").is_none() {
                return message;
            }
        }
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
