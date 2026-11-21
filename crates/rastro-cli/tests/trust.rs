//! The trust view: each cell's level, evidence counts and eligibility derived from the runs and
//! owner intents the store holds, what the next promotion needs, and the whole story on one row.
//! The clock is `RASTRO_NOW`; runs are driven through the real propose/accept boundary against
//! the scripted fake provider, so no outside system is called. A scoped read is driven through
//! the MCP catalog gateway, the only surface that reads under launch scopes.
mod support;
use rastro_core::{Storage, actions::RunResult};
use rastro_storage_sqlite::SqliteStorage;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::Duration,
};
use tempfile::TempDir;

/// A deadline only fails a server that is stuck, never one that is slow.
const DEADLINE: Duration = Duration::from_secs(30);

/// The time a command runs at when the test names none.
const NOW: &str = "2026-11-19T10:00:00Z";

struct Env {
    home: TempDir,
}

impl Env {
    /// A home with the scripted fake provider and the Linear source `linear-work` in scope
    /// `work`, and no rule: a proposal waits in the queue until the owner accepts it.
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
        };
        support::provider::install(&env.config_dir(), |_| {});
        env.set_config(&env.source_config("work"));
        env
    }

    fn config_dir(&self) -> std::path::PathBuf {
        self.home.path().join("config")
    }

    /// The config with one Linear source in `scope` and nothing else.
    fn source_config(&self, scope: &str) -> String {
        format!(
            "[[sources]]\nname = \"linear-work\"\nconnector = \"linear\"\nselector = {{}}\n\
             scope = \"{scope}\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
             keep_original = false\n"
        )
    }

    fn set_config(&self, text: &str) {
        fs::write(self.config_dir().join("config.toml"), text).unwrap();
    }

    fn config(&self) -> String {
        fs::read_to_string(self.config_dir().join("config.toml")).unwrap()
    }

    /// A command at `now`; the fake provider answers every action message.
    fn rastro_at(&self, now: &str, args: &[&str]) -> std::process::Output {
        Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_NOW", now)
            .args(args)
            .output()
            .unwrap()
    }

    /// The `result` of a command that must succeed, read with `--json` at `now`.
    fn ok_json_at(&self, now: &str, args: &[&str]) -> Value {
        let out = self.rastro_at(now, &[args, &["--json"]].concat());
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"].clone()
    }

    fn ok_json(&self, args: &[&str]) -> Value {
        self.ok_json_at(NOW, args)
    }

    /// The stdout of a command that must succeed, read as the terminal reads it.
    fn ok_text(&self, args: &[&str]) -> String {
        let out = self.rastro_at(NOW, args);
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// A proposal the owner accepts unchanged at `now`: its run reads `done` through the fake
    /// provider. Returns the run's id.
    fn clean_run(&self, title: &str, team: &str, now: &str) -> String {
        let item = self.ok_json_at(
            now,
            &[
                "actions",
                "propose",
                "linear.create_issue",
                "--title",
                title,
                "--team",
                team,
                "--why",
                "owner asked",
            ],
        )["id"]
            .as_str()
            .unwrap()
            .to_string();
        let answered = self.ok_json_at(now, &["queue", "accept", &item]);
        assert_eq!(answered["run"]["status"], "done", "{answered}");
        answered["run"]["id"].as_str().unwrap().to_string()
    }

    /// The owner's rule on `team`, appended to the config.
    fn add_rule(&self, team: &str) {
        self.set_config(&format!(
            "{}\n[[rules]]\nname = \"my-issues\"\naction = \"linear.create_issue\"\n\
             team = \"{team}\"\nassignee = \"me\"\n",
            self.config()
        ));
    }

    fn db(&self) -> SqliteStorage {
        SqliteStorage::open(&self.home.path().join("data/rastro.sqlite3")).unwrap()
    }

    /// A rule-started Act at `now`, finished through the store's own result record: executing
    /// an Act is the tick's work, not this read's. Returns the run's id.
    fn act_run(&self, title: &str, team: &str, now: &str) -> String {
        let proposed = self.ok_json_at(
            now,
            &[
                "actions",
                "propose",
                "linear.create_issue",
                "--title",
                title,
                "--team",
                team,
                "--why",
                "owner asked",
            ],
        );
        assert_eq!(proposed["run"]["status"], "pending", "{proposed}");
        let id = proposed["run"]["id"].as_str().unwrap().to_string();
        self.db()
            .record_action_result(
                &id,
                &RunResult {
                    identifier: format!("H-{title}"),
                    url: format!("https://fake.invalid/H-{title}"),
                    finished_at: now.parse().unwrap(),
                },
            )
            .unwrap();
        id
    }

    /// The owner undoes a done run at `now`; the fake provider answers the delete.
    fn undo(&self, run: &str, now: &str) {
        let undone = self.ok_json_at(now, &["actions", "undo", run]);
        assert_eq!(undone["status"], "undone", "{undone}");
    }

    /// A running `rastro mcp` under the given launch arguments.
    fn mcp(&self, args: &[&str]) -> Mcp {
        let mut child = Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .arg("mcp")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
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
        let mut stderr = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes);
        });
        Mcp {
            child,
            stdin,
            lines,
            next_id: 1,
        }
    }
}

/// A scripted MCP client: one request, one response, over stdio JSON lines.
struct Mcp {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next_id: u64,
}

impl Mcp {
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        )
        .unwrap();
        self.stdin.flush().unwrap();
        loop {
            let line = self
                .lines
                .recv_timeout(DEADLINE)
                .expect("the server printed nothing within its deadline");
            let message: Value =
                serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}"));
            if message["id"] == json!(id) && message.get("method").is_none() {
                return message;
            }
        }
    }

    fn initialize(&mut self) {
        self.request(
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {},
                   "clientInfo": {"name": "trust-test", "version": "1.0"}}),
        );
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        )
        .unwrap();
        self.stdin.flush().unwrap();
    }

    fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The v2 success envelope a tool call returned: present, not an error, with a result.
fn ok_envelope(result: &Value) -> &Value {
    assert!(
        result.is_object(),
        "no result (a protocol error?): {result}"
    );
    assert_ne!(result["isError"], true, "{result}");
    let envelope = &result["structuredContent"];
    assert_eq!(envelope["schema_version"], 2, "{result}");
    assert!(envelope.get("result").is_some(), "{result}");
    assert!(envelope.get("error").is_none(), "{result}");
    envelope
}

// spec: A cell's trust is derived from its outcomes
#[test]
fn clean_outcomes_make_a_cell_eligible() {
    let env = Env::new();
    // Ten clean accepted outcomes spanning eight days: one a day for nine days, one extra.
    for day in 1..=9 {
        env.clean_run(
            &format!("Issue {day}"),
            "H",
            &format!("2026-11-{day:02}T10:00:00Z"),
        );
    }
    env.clean_run("Issue 10", "H", "2026-11-05T15:00:00Z");

    let answer = env.ok_json(&["trust"]);
    let cells = answer["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 1, "{answer}");
    let cell = &cells[0];
    assert_eq!(cell["action"], "linear.create_issue", "{cell}");
    assert_eq!(cell["team"], "H", "{cell}");
    assert_eq!(cell["clean"], 10, "{cell}");
    assert_eq!(cell["undos"], 0, "{cell}");
    assert_eq!(cell["promotion"]["eligible"], true, "{cell}");

    // The terminal read shows the counts behind the eligibility.
    let text = env.ok_text(&["trust"]);
    assert!(text.contains("Clean: 10"), "{text}");
    assert!(text.contains("Undos: 0"), "{text}");
}

// spec: A cell's trust is derived from its outcomes
#[test]
fn configured_thresholds_name_a_cell_eligible() {
    let env = Env::new();
    env.set_config(&format!(
        "{}\n[learning]\nclean_min = 5\n",
        env.source_config("work")
    ));
    // Five clean outcomes over seven days: the default bar of ten would not name it eligible.
    for day in [1, 2, 3, 4, 8] {
        env.clean_run(
            &format!("Issue {day}"),
            "H",
            &format!("2026-11-{day:02}T10:00:00Z"),
        );
    }
    let answer = env.ok_json(&["trust"]);
    let cell = &answer["cells"][0];
    assert_eq!(cell["clean"], 5, "{cell}");
    assert_eq!(cell["promotion"]["eligible"], true, "{cell}");
}

// spec: Promotion is proposed, never self-applied
#[test]
fn an_eligible_cell_without_a_rule_is_proposed_one() {
    let env = Env::new();
    for day in 1..=9 {
        env.clean_run(
            &format!("Issue {day}"),
            "H",
            &format!("2026-11-{day:02}T10:00:00Z"),
        );
    }
    env.clean_run("Issue 10", "H", "2026-11-05T15:00:00Z");

    let answer = env.ok_json(&["trust"]);
    let cell = &answer["cells"][0];
    // Eligible, but the level stays Propose until the owner writes the rule.
    assert_eq!(cell["level"], "propose", "{cell}");
    assert_eq!(cell["promotion"]["eligible"], true, "{cell}");
    let rule_text = cell["promotion"]["rule_text"]
        .as_str()
        .unwrap_or_else(|| panic!("no proposed rule: {cell}"));
    assert!(rule_text.contains("[[rules]]"), "{rule_text}");
    assert!(rule_text.contains("linear.create_issue"), "{rule_text}");
    assert!(rule_text.contains("\"H\""), "{rule_text}");
    // The terminal read shows the entry too.
    let text = env.ok_text(&["trust"]);
    assert!(text.contains("[[rules]]"), "{text}");
}

// spec: Promotion is proposed, never self-applied
#[test]
fn the_owners_rule_raises_the_cell() {
    let env = Env::new();
    for day in 1..=9 {
        env.clean_run(
            &format!("Issue {day}"),
            "H",
            &format!("2026-11-{day:02}T10:00:00Z"),
        );
    }
    env.clean_run("Issue 10", "H", "2026-11-05T15:00:00Z");

    // The owner pastes the exact proposed entry into config.toml.
    let proposed = env.ok_json(&["trust"])["cells"][0]["promotion"]["rule_text"]
        .as_str()
        .unwrap()
        .to_string();
    env.set_config(&format!("{}\n{proposed}", env.config()));

    let answer = env.ok_json(&["trust"]);
    let cell = &answer["cells"][0];
    assert_eq!(cell["level"], "act", "{cell}");
}

// spec: The trust view tells the whole story
#[test]
fn a_row_tells_the_whole_story() {
    let env = Env::new();
    env.add_rule("H");
    // Three clean Act runs and one the owner undoes: the undo drops the cell one level.
    for day in 1..=3 {
        env.act_run(
            &format!("Issue {day}"),
            "H",
            &format!("2026-11-{day:02}T10:00:00Z"),
        );
    }
    let undone = env.act_run("Issue 4", "H", "2026-11-04T10:00:00Z");
    env.undo(&undone, "2026-11-04T12:00:00Z");

    let answer = env.ok_json(&["trust"]);
    let cell = &answer["cells"][0];
    assert_eq!(cell["level"], "propose", "{cell}");
    assert_eq!(cell["clean"], 3, "{cell}");
    assert_eq!(cell["undos"], 1, "{cell}");
    assert_eq!(cell["last_change"]["cause"], "undo", "{cell}");
    assert!(
        cell["last_change"]["at"]
            .as_str()
            .is_some_and(|at| at.contains("2026-11-04T12:00:00")),
        "{cell}"
    );
    assert_eq!(cell["promotion"]["eligible"], false, "{cell}");
    assert!(
        cell["promotion"]["needs"]
            .as_str()
            .is_some_and(|needs| !needs.is_empty()),
        "{cell}"
    );

    // The terminal row tells the same story: level, counts, last change with its cause, and
    // what the next promotion needs.
    let text = env.ok_text(&["trust"]);
    for part in [
        "Level: propose",
        "Clean: 3",
        "Undos: 1",
        "Last change: undo",
        "2026-11-04T12:00:00",
        "Next promotion:",
    ] {
        assert!(text.contains(part), "{part} missing from: {text}");
    }
}

// spec: The trust view tells the whole story
#[test]
fn hidden_scopes_count_nowhere() {
    let env = Env::new();
    // The visible cell: three clean outcomes in scope work.
    for day in 1..=3 {
        env.clean_run(
            &format!("Work {day}"),
            "H",
            &format!("2026-11-{day:02}T10:00:00Z"),
        );
    }
    // The hidden cell: ten clean outcomes in scope hidden, eligible were they counted. The
    // source's scope moves between accepts; each run keeps the scope it was recorded in.
    env.set_config(&env.source_config("hidden"));
    for day in 1..=9 {
        env.clean_run(
            &format!("Hidden {day}"),
            "SECRET",
            &format!("2026-11-{day:02}T10:00:00Z"),
        );
    }
    env.clean_run("Hidden 10", "SECRET", "2026-11-05T15:00:00Z");
    env.set_config(&env.source_config("work"));

    // The CLI sees every scope: both cells, each with its own counts.
    let all = env.ok_json(&["trust"]);
    let cells = all["cells"].as_array().unwrap();
    let teams: Vec<_> = cells
        .iter()
        .map(|cell| cell["team"].as_str().unwrap())
        .collect();
    assert_eq!(teams, ["H", "SECRET"], "{all}");

    // Under the visible scopes the hidden cell is absent and counts none of its outcomes.
    let mut session = env.mcp(&["--scope", "work"]);
    session.initialize();
    let call = session.call(
        "catalog",
        json!({"mode": "call", "operation": "trust", "arguments": {}}),
    );
    let envelope = ok_envelope(&call["result"]);
    assert_eq!(envelope["operation"], "trust", "{envelope}");
    let cells = envelope["result"]["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 1, "{envelope}");
    assert_eq!(cells[0]["team"], "H", "{cells:?}");
    assert_eq!(cells[0]["clean"], 3, "{cells:?}");
}

// spec: The trust view tells the whole story
#[test]
fn an_empty_store_lists_no_cells() {
    let env = Env::new();
    // No database at all: the read-only open holds no store.
    let answer = env.ok_json(&["trust"]);
    assert_eq!(answer["cells"], json!([]), "{answer}");
    let text = env.ok_text(&["trust"]);
    assert!(
        text.to_lowercase().contains("no trust history"),
        "the empty read says the store holds no trust history: {text}"
    );
    // A database with sources but no runs: still no cells.
    env.ok_json(&["coverage"]);
    let answer = env.ok_json(&["trust"]);
    assert_eq!(answer["cells"], json!([]), "{answer}");
}
