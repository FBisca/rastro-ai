//! The read-only open path the MCP server uses per tool call.
#![allow(non_snake_case)]
mod support;
use anyhow::Result;
use rastro_core::{
    Coverage, CoverageState, Locations, Rastro, ReadStorage, Reader, Refusal, SearchOptions,
    Snapshot, UnfinishedRun,
};
use rastro_storage_sqlite::SqliteStorage;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::mpsc,
    time::Duration,
};
use tempfile::TempDir;

/// Generous: a deadline here only fails a call that is stuck on a lock, never one that is slow.
const DEADLINE: Duration = Duration::from_secs(20);

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
    fn ok(&self, args: &[&str]) -> String {
        let output = self.rastro(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn add_source(&self, name: &str) {
        self.ok(&[
            "source",
            "add",
            name,
            "--connector",
            "test",
            "--repo",
            "o/p",
            "--scope",
            "personal",
        ]);
    }
    fn script(&self, steps: &[Value]) {
        let lines: Vec<_> = steps.iter().map(Value::to_string).collect();
        fs::write(self.home.path().join("script.jsonl"), lines.join("\n")).unwrap();
    }
    /// A sync of `source` that submits one body per `(location, quote)`, after `sleep_ms`.
    fn sync(&self, source: &str, bodies: &[(&str, &str)], sleep_ms: u64) -> Output {
        self.script(&sync_script(bodies, sleep_ms));
        self.command(&["sync", source])
            .env("RASTRO_HOST_SILENCE_MS", "15000")
            .output()
            .unwrap()
    }
    fn locations(&self) -> Locations {
        Locations {
            data: self.home.path().join("data"),
            config: self.home.path().join("config"),
        }
    }
    fn database(&self) -> PathBuf {
        self.home.path().join("data/rastro.sqlite3")
    }
    fn db(&self) -> Connection {
        Connection::open(self.database()).unwrap()
    }
    /// The first source's coverage as the writing path answers it, from `--json`.
    fn json_coverage(&self) -> Value {
        let output = self.rastro(&["coverage", "--json"]);
        assert!(output.status.success());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        value["result"]["coverage"][0].clone()
    }
    fn reader(&self) -> Reader<SqliteStorage> {
        Rastro::open_read_only(self.locations(), SqliteStorage::open_read_only).unwrap()
    }
}

fn sync_script(bodies: &[(&str, &str)], sleep_ms: u64) -> Vec<Value> {
    let mut steps = vec![json!({"do": "initialize"})];
    for (location, quote) in bodies {
        steps.push(
            json!({"do": "send", "message": {"jsonrpc": "2.0", "method": "evidence.submit",
            "params": {"kind": "issue", "location": location, "field": "/body", "start": 0,
            "end": quote.len(), "quote": quote, "trust_class": "untrusted content",
            "source_updated_at": "2026-11-01T00:00:00Z"}}}),
        );
    }
    if sleep_ms > 0 {
        steps.push(json!({"do": "sleep", "ms": sleep_ms}));
    }
    steps.push(
        json!({"do": "send", "message": {"jsonrpc": "2.0", "method": "coverage.report",
        "params": {"status": "complete", "selection": "all"}}}),
    );
    steps.push(json!({"do": "finish", "status": "ok", "submitted": bodies.len()}));
    steps
}

/// Every row of every table, in a stable order.
fn dump(connection: &Connection) -> Vec<(String, Vec<Vec<String>>)> {
    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    tables
        .into_iter()
        .map(|table| {
            let mut statement = connection
                .prepare(&format!("SELECT * FROM \"{table}\" ORDER BY 1"))
                .unwrap();
            let width = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    (0..width)
                        .map(|i| {
                            Ok(match row.get_ref(i)? {
                                rusqlite::types::ValueRef::Null => "NULL".to_string(),
                                rusqlite::types::ValueRef::Integer(n) => n.to_string(),
                                rusqlite::types::ValueRef::Real(n) => n.to_string(),
                                rusqlite::types::ValueRef::Text(t) => {
                                    String::from_utf8_lossy(t).into_owned()
                                }
                                rusqlite::types::ValueRef::Blob(b) => format!("{b:?}"),
                            })
                        })
                        .collect()
                })
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            (table, rows)
        })
        .collect()
}

/// Runs `work` and fails if it hasn't returned within `DEADLINE` (a lock it is stuck on).
fn within<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });
    receiver
        .recv_timeout(DEADLINE)
        .expect("a read-only call did not answer within its deadline")
}

fn synced() -> Env {
    let env = Env::new();
    env.add_source("project");
    let output = env.sync(
        "project",
        &[
            ("https://x/1", "first body"),
            ("https://x/2", "second body"),
        ],
        0,
    );
    assert!(output.status.success());
    env
}

fn only(coverage: Vec<Coverage>) -> Coverage {
    assert_eq!(coverage.len(), 1, "{coverage:?}");
    coverage.into_iter().next().unwrap()
}

/// surfaces/R14: no row changes and no exclusive config lock
#[test]
fn search_and_coverage_change_no_row_and_take_no_exclusive_config_lock() {
    let env = synced();
    let before = dump(&env.db());
    // A shared holder on the config lock: an exclusive lock request would wait forever.
    let holder = fs::File::open(env.home.path().join("config/config.lock")).unwrap();
    holder.lock_shared().unwrap();
    let locations = env.locations();
    let (hits, coverage) = within(move || {
        let mut reader = Rastro::open_read_only(locations, SqliteStorage::open_read_only).unwrap();
        (
            reader.search("body", &SearchOptions::default()).unwrap(),
            reader.coverage(None, false).unwrap(),
        )
    });
    drop(holder);
    assert_eq!(hits.results.len(), 2);
    assert_eq!(only(coverage).state, CoverageState::Found);
    assert_eq!(dump(&env.db()), before);
}

/// surfaces/R14: the writing open puts the database in WAL mode, an old database included
#[test]
fn the_writing_open_switches_the_database_to_wal_mode() {
    let env = synced();
    let mode = |env: &Env| {
        env.db()
            .query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
    };
    assert_eq!(mode(&env).unwrap(), "wal");
    env.db()
        .query_row("PRAGMA journal_mode = DELETE", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap();
    assert_eq!(mode(&env).unwrap(), "delete");
    env.ok(&["coverage"]);
    assert_eq!(mode(&env).unwrap(), "wal");
    // The read-only path leaves the mode alone.
    env.db()
        .query_row("PRAGMA journal_mode = DELETE", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap();
    env.reader().coverage(None, false).unwrap();
    assert_eq!(mode(&env).unwrap(), "delete");
}

/// surfaces/R15: calls during a sync in another process answer, with no lock error
#[test]
fn calls_during_a_sync_in_another_process_answer_without_a_lock_error() {
    let env = synced();
    env.script(&sync_script(&[("https://x/3", "third body")], 3000));
    let mut sync = env
        .command(&["sync", "project"])
        .env("RASTRO_HOST_SILENCE_MS", "15000")
        .spawn()
        .unwrap();
    // Wait until the sync's run is open: its row exists and it holds sync.lock.
    let mut started = false;
    for _ in 0..200 {
        started = env
            .db()
            .query_row(
                "SELECT count(*) FROM runs WHERE finished_at IS NULL",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            == 1;
        if started {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        started,
        "the sync never started, so nothing ran concurrently"
    );
    for _ in 0..5 {
        let locations = env.locations();
        let (hits, coverage) = within(move || {
            let mut reader =
                Rastro::open_read_only(locations, SqliteStorage::open_read_only).unwrap();
            (
                reader.search("body", &SearchOptions::default()).unwrap(),
                reader.coverage(None, false).unwrap(),
            )
        });
        // The sync may already have committed its third body; never fewer than before.
        assert!(hits.results.len() >= 2);
        // The live run's unfinished row doesn't change the answer.
        assert_eq!(only(coverage).state, CoverageState::Found);
    }
    assert!(sync.wait().unwrap().success());
    assert_eq!(
        env.reader()
            .search("body", &SearchOptions::default())
            .unwrap()
            .results
            .len(),
        3
    );
}

/// A storage whose snapshot lets another writer commit right after the snapshot is taken.
struct Interleaved {
    inner: SqliteStorage,
    between: Option<Box<dyn FnOnce()>>,
}
struct InterleavedView<'a> {
    inner: &'a dyn Snapshot,
    between: std::cell::RefCell<Option<Box<dyn FnOnce()>>>,
}
impl InterleavedView<'_> {
    fn interleave(&self) {
        if let Some(write) = self.between.borrow_mut().take() {
            write();
        }
    }
}
impl Snapshot for InterleavedView<'_> {
    fn coverage(&self, source: &rastro_core::Pair, all_windows: bool) -> Result<Coverage> {
        self.interleave();
        self.inner.coverage(source, all_windows)
    }
    fn stored_pairs(&self, scopes: &[rastro_core::Scope]) -> Result<Vec<rastro_core::Pair>> {
        self.interleave();
        self.inner.stored_pairs(scopes)
    }
    fn search(
        &self,
        query: &str,
        limit: usize,
        history: bool,
        scopes: &[rastro_core::Scope],
    ) -> Result<Vec<rastro_core::Hit>> {
        self.interleave();
        self.inner.search(query, limit, history, scopes)
    }
    fn unfinished_runs(&self, scopes: &[rastro_core::Scope]) -> Result<Vec<UnfinishedRun>> {
        self.interleave();
        self.inner.unfinished_runs(scopes)
    }
}
impl ReadStorage for Interleaved {
    fn read<T>(&mut self, work: impl FnOnce(&dyn Snapshot) -> Result<T>) -> Result<T> {
        let between = self.between.take();
        self.inner.read(|view| {
            work(&InterleavedView {
                inner: view,
                between: std::cell::RefCell::new(between),
            })
        })
    }
}

/// surfaces/R15: the hits and the coverage in one answer come from the same snapshot
#[test]
fn hits_and_coverage_in_one_answer_come_from_the_same_snapshot() {
    let env = synced();
    let old = only(env.reader().coverage(None, false).unwrap()).last_checked;
    let database = env.database();
    let write_another_sync = move || {
        // A whole run commits between the first read of the call and its last.
        let mut writer = SqliteStorage::open(&database).unwrap();
        let source = rastro_core::Source {
            name: "project".into(),
            connector: "test".into(),
            selector: Default::default(),
            scope: rastro_core::Scope("personal".into()),
            freshness_rule: "1h".into(),
            window_start: chrono::Utc::now(),
            keep_original: false,
        };
        use rastro_core::Storage;
        let now = chrono::Utc::now();
        let run = writer
            .start_run(&source, ("test", "0.1.0"), now, (now, now))
            .unwrap();
        writer
            .append_evidence(
                run,
                &rastro_core::Evidence {
                    source: "project".into(),
                    scope: rastro_core::Scope("personal".into()),
                    location: "https://x/3".into(),
                    field: "/body".into(),
                    start: 0,
                    end: 10,
                    quote: "third body".into(),
                    trust_class: rastro_core::TrustClass::parse("untrusted content").unwrap(),
                    captured_at: now,
                    source_updated_at: None,
                    original: None,
                },
            )
            .unwrap();
        writer
            .record_coverage(&rastro_core::CoverageRecord {
                run_id: run,
                source: "project".into(),
                scope: rastro_core::Scope("personal".into()),
                window: (now, now),
                selection: "all".into(),
                state: CoverageState::Found,
                reason: None,
                retry_at: None,
                checked_at: now,
            })
            .unwrap();
        writer
            .finish_run(
                run,
                &rastro_core::RunEnd {
                    status: "ok".into(),
                    reason: None,
                    finished_at: now,
                    counts: Default::default(),
                },
            )
            .unwrap();
    };
    let storage = Interleaved {
        inner: SqliteStorage::open_read_only(&env.database())
            .unwrap()
            .unwrap(),
        between: Some(Box::new(write_another_sync)),
    };
    let mut reader = Rastro::open_read_only(env.locations(), move |_| Ok(Some(storage))).unwrap();
    let answer = reader.search("body", &SearchOptions::default()).unwrap();
    assert_eq!(
        answer.results.len(),
        2,
        "the late commit leaked into the hits"
    );
    assert_eq!(only(answer.coverage).last_checked, old);
    // The next call sees what committed.
    let next = env
        .reader()
        .search("body", &SearchOptions::default())
        .unwrap();
    assert_eq!(next.results.len(), 3);
    assert_ne!(only(next.coverage).last_checked, old);
}

/// surfaces/R16: a source added after the reader's previous call shows up in the next one
#[test]
fn a_source_added_after_the_last_call_appears_in_the_next() {
    let env = synced();
    let names = |env: &Env| -> Vec<String> {
        env.reader()
            .coverage(None, false)
            .unwrap()
            .into_iter()
            .map(|c| c.source)
            .collect()
    };
    assert_eq!(names(&env), ["project"]);
    env.add_source("later");
    assert_eq!(names(&env), ["project", "later"]);
    let answer = env
        .reader()
        .search("nothing-matches", &SearchOptions::default())
        .unwrap();
    assert_eq!(answer.coverage.len(), 2);
}

fn unfinished_run(env: &Env) {
    env.db()
        .execute(
            "INSERT INTO runs (source, scope, plugin_id, plugin_version, started_at, status,
             window_start, window_end)
             VALUES ('project', 'personal', 'test', '0.1.0', '2026-11-04T00:00:00.000Z',
             'running', '2026-11-01T00:00:00.000Z', '2026-11-04T00:00:00.000Z')",
            [],
        )
        .unwrap();
}

/// surfaces/R17: an unfinished run no sync holds reads "unknown: interrupted", unwritten
#[test]
fn an_unfinished_run_with_no_live_sync_reads_unknown_interrupted_without_writing() {
    let env = Env::new();
    env.add_source("project");
    unfinished_run(&env);
    let before = dump(&env.db());
    // A live sync holds sync.lock: its unfinished run changes nothing.
    let live = fs::File::create(env.home.path().join("data/sync.lock")).unwrap();
    live.lock().unwrap();
    let during = only(env.reader().coverage(None, false).unwrap());
    assert_eq!(during.state, CoverageState::NeverLooked, "{during:?}");
    assert_eq!(during.reason, None);
    drop(live);
    // Nothing holds it: the run was left by a crash.
    let after = only(env.reader().coverage(None, false).unwrap());
    assert_eq!(after.state, CoverageState::Unknown, "{after:?}");
    assert_eq!(after.reason.as_deref(), Some("interrupted"));
    let searched = env
        .reader()
        .search("anything", &SearchOptions::default())
        .unwrap();
    assert_eq!(
        only(searched.coverage).reason.as_deref(),
        Some("interrupted")
    );
    assert_eq!(dump(&env.db()), before, "a read wrote");
    // The writing open records the same answer.
    env.ok(&["coverage"]);
    let recovered = only(env.reader().coverage(None, false).unwrap());
    assert_eq!(recovered.state, CoverageState::Unknown);
    assert_eq!(recovered.reason.as_deref(), Some("interrupted"));
}

/// surfaces/R18: no database file: every source never looked, no hits, no file created
#[test]
fn a_missing_database_answers_from_config_and_creates_no_file() {
    let env = Env::new();
    env.add_source("project");
    env.add_source("other");
    fs::remove_dir_all(env.home.path().join("data")).unwrap();
    let mut reader = env.reader();
    let coverage = reader.coverage(None, false).unwrap();
    assert_eq!(coverage.len(), 2);
    assert!(
        coverage
            .iter()
            .all(|c| c.state == CoverageState::NeverLooked && c.windows.is_empty())
    );
    let answer = reader.search("body", &SearchOptions::default()).unwrap();
    assert!(answer.results.is_empty());
    assert_eq!(answer.coverage.len(), 2);
    assert!(
        answer
            .coverage
            .iter()
            .all(|c| c.state == CoverageState::NeverLooked)
    );
    assert!(reader.coverage(Some("nobody"), false).is_err());
    assert!(
        !env.home.path().join("data").exists(),
        "a read created data"
    );
}

/// surfaces/R18: with nothing configured and nothing on disk, a read creates nothing
#[test]
fn a_read_with_no_config_and_no_database_creates_nothing() {
    let home = TempDir::new().unwrap();
    let locations = Locations {
        data: home.path().join("data"),
        config: home.path().join("config"),
    };
    let mut reader = Rastro::open_read_only(locations, SqliteStorage::open_read_only).unwrap();
    assert!(reader.coverage(None, false).unwrap().is_empty());
    assert!(
        reader
            .search("x", &SearchOptions::default())
            .unwrap()
            .results
            .is_empty()
    );
    assert_eq!(fs::read_dir(home.path()).unwrap().count(), 0);
}

/// surfaces/R19: a pending or unknown newer migration is a typed refusal, not a migration
#[test]
fn a_pending_or_unknown_migration_is_refused_with_a_typed_error() {
    let env = synced();
    let refusal = |env: &Env| -> Refusal {
        let mut reader = env.reader();
        let error = reader.coverage(None, false).unwrap_err();
        let search = reader
            .search("body", &SearchOptions::default())
            .unwrap_err();
        assert!(search.downcast_ref::<Refusal>().is_some());
        error.downcast::<Refusal>().unwrap()
    };
    let last: String = env
        .db()
        .query_row("SELECT max(name) FROM schema_migrations", [], |r| r.get(0))
        .unwrap();
    env.db()
        .execute("DELETE FROM schema_migrations WHERE name = ?1", [&last])
        .unwrap();
    let before = dump(&env.db());
    assert!(matches!(refusal(&env), Refusal::PendingMigrations(p) if p == [last.clone()]));
    assert_eq!(dump(&env.db()), before, "a refusal migrated");
    env.db()
        .execute(
            "INSERT INTO schema_migrations (name, applied_at) VALUES (?1, 'now')",
            [&last],
        )
        .unwrap();
    env.db()
        .execute(
            "INSERT INTO schema_migrations (name, applied_at) VALUES ('299912312359_future', 'now')",
            [],
        )
        .unwrap();
    assert!(matches!(refusal(&env), Refusal::UnknownMigration(n) if n == "299912312359_future"));
}

/// evidence/R30: a sync that takes sync.lock marks a never-finished run interrupted before its own,
/// even when a reader blocked the startup recovery
#[test]
fn a_sync_starting_beside_a_reader_still_recovers_the_crashed_run_before_its_own() {
    let env = synced();
    unfinished_run(&env);
    let holder = fs::File::open(env.home.path().join("data/sync.lock")).unwrap();
    holder.lock_shared().unwrap();
    // The sync's open holds config.lock until it has given up waiting on us, so held-then-free
    // means the open is over and the sync is on its way to taking sync.lock. Only then let go.
    env.script(&sync_script(&[("https://x/3", "third body")], 0));
    let child = env
        .command(&["sync", "project"])
        .env("RASTRO_HOST_SILENCE_MS", "15000")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let config_lock = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(env.home.path().join("config/config.lock"))
        .unwrap();
    let held = |file: &fs::File| match file.try_lock() {
        Ok(()) => {
            file.unlock().unwrap();
            false
        }
        Err(_) => true,
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !held(&config_lock) {
        assert!(std::time::Instant::now() < deadline, "open never started");
        std::thread::sleep(Duration::from_millis(1));
    }
    while held(&config_lock) {
        assert!(std::time::Instant::now() < deadline, "open never ended");
        std::thread::sleep(Duration::from_millis(1));
    }
    drop(holder);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let runs = |env: &Env| -> Vec<String> {
        env.db()
            .prepare("SELECT status || ':' || coalesce(reason, '') FROM runs ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(runs(&env), ["ok:", "failed:interrupted", "ok:"]);
    // The recovered window sorts before the later success, so the latest attempt decides.
    let states: Vec<String> = env
        .db()
        .prepare("SELECT state FROM coverage ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        states.last().map(String::as_str),
        Some("found"),
        "{states:?}"
    );
    assert_eq!(
        only(env.reader().coverage(None, false).unwrap()).state,
        CoverageState::Found
    );
    let human = env.ok(&["coverage"]);
    // The overall line says found; the interrupted window stays listed as history.
    assert!(human.contains("Coverage: found"), "{human}");
}

/// A found window, then `quiet` empty ones: the only evidence is the oldest window.
fn found_then_quiet(quiet: usize) -> Env {
    let env = synced();
    for _ in 0..quiet {
        assert!(env.sync("project", &[], 0).status.success());
    }
    env
}

/// evidence/R63: a source with more than ten windows lists the ten newest, newest first
#[test]
fn the_reader_lists_the_ten_newest_windows_newest_first_or_all_on_request() {
    let env = found_then_quiet(11);
    let capped = only(env.reader().coverage(None, false).unwrap());
    assert_eq!(capped.windows_total, 12);
    assert_eq!(capped.windows.len(), 10);
    assert!(
        capped
            .windows
            .iter()
            .all(|w| w.state == CoverageState::Empty)
    );
    assert!(capped.windows.windows(2).all(|p| p[0].from >= p[1].from));
    let all = only(env.reader().coverage(None, true).unwrap());
    assert_eq!(all.windows_total, 12);
    assert_eq!(all.windows.len(), 12);
    assert_eq!(all.windows[11].state, CoverageState::Found);
    assert_eq!(all.windows[0].from, capped.windows[0].from);
}

/// evidence/R64: the only found window being outside the newest ten keeps the source found
#[test]
fn the_reader_answers_found_when_the_only_found_window_is_outside_the_newest_ten() {
    let env = found_then_quiet(11);
    let mut reader = env.reader();
    assert_eq!(
        only(reader.coverage(None, false).unwrap()).state,
        CoverageState::Found
    );
    let hits = reader.search("body", &SearchOptions::default()).unwrap();
    assert_eq!(hits.results.len(), 2);
    assert!(
        hits.results
            .iter()
            .all(|h| h.coverage.as_ref().unwrap().state == CoverageState::Found)
    );
}

/// evidence/R63: an interrupted run beside more than ten windows is the newest, counted, capped
#[test]
fn an_interrupted_run_is_the_newest_window_counted_and_capped_like_its_recovery() {
    let env = found_then_quiet(11);
    unfinished_run(&env);
    let read = only(env.reader().coverage(None, false).unwrap());
    assert_eq!(read.windows_total, 13);
    assert_eq!(read.windows.len(), 10);
    assert_eq!(read.windows[0].state, CoverageState::CouldNotLook);
    assert_eq!(read.windows[0].reason.as_deref(), Some("interrupted"));
    assert_eq!(read.state, CoverageState::Unknown);
    // The writing open recovers the run, then answers the same total and order.
    let written = env.json_coverage();
    assert_eq!(written["windows_total"], 13);
    let shape = |states: Vec<(String, Option<String>)>| states;
    let from_read = shape(
        read.windows
            .iter()
            .map(|w| (w.state.as_str().to_string(), w.reason.clone()))
            .collect(),
    );
    let from_written = shape(
        written["windows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| {
                (
                    w["state"].as_str().unwrap().to_string(),
                    w.get("reason").map(|r| r.as_str().unwrap().to_string()),
                )
            })
            .collect(),
    );
    assert_eq!(from_read, from_written);
    assert_eq!(written["state"], "unknown");
}

fn edit_sources(env: &Env, edit: impl FnOnce(&mut Vec<toml::Value>)) {
    let path = env.home.path().join("config/config.toml");
    let mut config: toml::Table = fs::read_to_string(&path).unwrap().parse().unwrap();
    edit(config.get_mut("sources").unwrap().as_array_mut().unwrap());
    fs::write(path, config.to_string()).unwrap();
}

/// surfaces/R7: the reader keeps a removed source's coverage, unregistered
#[test]
fn the_reader_answers_a_removed_source_from_its_stored_windows_unregistered() {
    let env = synced();
    edit_sources(&env, |sources| sources.clear());
    let mut reader = env.reader();
    let entry = only(reader.coverage(None, false).unwrap());
    assert_eq!(
        (entry.state, entry.registered),
        (CoverageState::Found, false)
    );
    assert_eq!(entry.windows_total, 1);
    let hits = reader.search("body", &SearchOptions::default()).unwrap();
    assert_eq!(hits.results.len(), 2);
    for hit in &hits.results {
        let covering = hit.coverage.as_ref().unwrap();
        assert_eq!(
            (covering.state, covering.registered),
            (CoverageState::Found, false)
        );
    }
    // Long past its one-hour rule: unknown, with the last check still shown.
    env.db()
        .execute(
            "UPDATE coverage SET checked_at='2000-01-01T00:00:00.000Z'",
            [],
        )
        .unwrap();
    let mut reader = env.reader();
    let stale = only(reader.coverage(None, false).unwrap());
    assert_eq!(
        (stale.state, stale.registered),
        (CoverageState::Unknown, false)
    );
    assert_eq!(stale.last_checked.unwrap().to_rfc3339()[..4], *"2000");
    for hit in reader
        .search("body", &SearchOptions::default())
        .unwrap()
        .results
    {
        let covering = hit.coverage.unwrap();
        assert_eq!(
            (covering.state, covering.registered),
            (CoverageState::Unknown, false)
        );
    }
}

/// surfaces/R7: a removed source's unfinished run is its newest, counted window
#[test]
fn the_reader_counts_a_removed_sources_interrupted_run_as_a_window() {
    let env = synced();
    edit_sources(&env, |sources| sources.clear());
    unfinished_run(&env);
    let entry = only(env.reader().coverage(None, false).unwrap());
    assert!(!entry.registered);
    assert_eq!(entry.windows_total, 2);
    assert_eq!(entry.windows[0].state, CoverageState::CouldNotLook);
    assert_eq!(entry.windows[0].reason.as_deref(), Some("interrupted"));
    assert_eq!(entry.state, CoverageState::Unknown);
}

/// surfaces/R7: a name in neither config nor storage is unknown, however many pairs storage has
#[test]
fn a_name_in_neither_config_nor_storage_is_an_unknown_source() {
    let env = synced();
    edit_sources(&env, |sources| sources.clear());
    let error = env.reader().coverage(Some("nosuch"), false).unwrap_err();
    assert!(error.downcast_ref::<rastro_core::UnknownSource>().is_some());
    assert!(!env.rastro(&["coverage", "nosuch"]).status.success());
    // The stored name still answers.
    assert!(env.rastro(&["coverage", "project"]).status.success());
}

/// surfaces/R8: a removed source that never synced reads never looked, unregistered
#[test]
fn the_reader_lists_a_removed_source_that_never_synced_as_never_looked() {
    let env = Env::new();
    env.add_source("project");
    edit_sources(&env, |sources| sources.clear());
    let entry = only(env.reader().coverage(None, false).unwrap());
    assert_eq!(entry.source, "project");
    assert_eq!(
        (entry.state, entry.registered),
        (CoverageState::NeverLooked, false)
    );
}

/// surfaces/R9: after a scope change, old-scope hits carry the old pair, unregistered
#[test]
fn the_reader_gives_old_scope_hits_the_old_pairs_coverage() {
    let env = synced();
    edit_sources(&env, |sources| sources[0]["scope"] = "work".into());
    let mut reader = env.reader();
    let coverage = reader.coverage(None, false).unwrap();
    assert_eq!(coverage.len(), 2, "{coverage:?}");
    let old = coverage.iter().find(|c| c.scope.0 == "personal").unwrap();
    let new = coverage.iter().find(|c| c.scope.0 == "work").unwrap();
    assert_eq!((old.state, old.registered), (CoverageState::Found, false));
    assert_eq!(
        (new.state, new.registered),
        (CoverageState::NeverLooked, true)
    );
    let hits = reader.search("body", &SearchOptions::default()).unwrap();
    assert!(!hits.results.is_empty());
    for hit in &hits.results {
        let covering = hit.coverage.as_ref().unwrap();
        assert_eq!(covering.scope, hit.scope);
        assert!(!covering.registered);
        assert_eq!(covering.state, CoverageState::Found);
    }
    // The old pair goes stale under the rule stored for the source.
    env.db()
        .execute(
            "UPDATE coverage SET checked_at='2000-01-01T00:00:00.000Z'",
            [],
        )
        .unwrap();
    let hits = env
        .reader()
        .search("body", &SearchOptions::default())
        .unwrap();
    for hit in &hits.results {
        assert_eq!(hit.coverage.as_ref().unwrap().state, CoverageState::Unknown);
    }
}
