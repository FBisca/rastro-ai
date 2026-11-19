//! `rastro tick`: the one entry point a schedule calls. It syncs each source past its freshness
//! rule, does nothing while another tick runs, and keeps going when one source fails.
//!
//! The connector is the scripted test connector; the clock is `RASTRO_NOW`, so a source ages
//! without a wait, and the lock another tick holds is the real lock file, held by the test.
mod support;
use chrono::{Duration, Utc};
use rusqlite::Connection;
use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;

struct Env {
    home: TempDir,
}

impl Env {
    /// A home with the test connector installed and no source yet.
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

    /// `rastro args`, with the clock at `now` when given.
    fn rastro(&self, args: &[&str], now: Option<&str>) -> Output {
        let mut command = Command::new(support::rastro());
        command
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.home.path().join("dev-plugins"))
            .args(args);
        if let Some(now) = now {
            command.env("RASTRO_NOW", now);
        }
        command.output().unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.rastro(args, None);
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn add_source(&self, name: &str, freshness_rule: &str) {
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
            "--freshness-rule",
            freshness_rule,
        ]);
    }

    /// The runs recorded for `source`.
    fn runs(&self, source: &str) -> i64 {
        Connection::open(self.home.path().join("data/rastro.sqlite3"))
            .unwrap()
            .query_row(
                "SELECT count(*) FROM runs WHERE source = ?1 AND role = 'connector'",
                [source],
                |row| row.get(0),
            )
            .unwrap()
    }
}

fn after(hours: i64) -> String {
    (Utc::now() + Duration::hours(hours)).to_rfc3339()
}

/// spec: One tick syncs every stale source
#[test]
fn tick_syncs_the_source_past_its_freshness_rule_and_skips_the_one_within_it() {
    let env = Env::new();
    env.add_source("stale", "1m");
    env.add_source("fresh", "1d");
    env.ok(&["sync"]);
    assert_eq!((env.runs("stale"), env.runs("fresh")), (1, 1));

    // An hour on: past one source's rule of a minute, within the other's of a day.
    let out = env.rastro(&["tick"], Some(&after(1)));

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!((env.runs("stale"), env.runs("fresh")), (2, 1));
}

/// spec: One tick syncs every stale source
#[test]
fn a_tick_that_finds_another_tick_running_exits_0_and_records_no_run() {
    let env = Env::new();
    env.add_source("project", "1h");
    // The first tick's lock, held for as long as `first` lives.
    let first = fs::File::create(env.home.path().join("data/tick.lock")).unwrap();
    first.lock().unwrap();

    let out = env.rastro(&["tick"], None);

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let said = String::from_utf8_lossy(&out.stdout).to_lowercase();
    assert!(said.contains("already running"), "{said}");
    assert_eq!(env.runs("project"), 0);
}

/// spec: One tick syncs every stale source
#[test]
fn a_source_that_fails_does_not_stop_the_next_and_tick_names_it() {
    let env = Env::new();
    env.add_source("broken", "1h");
    // The first source names a connector that is not installed, so its sync fails.
    let config = env.home.path().join("config/config.toml");
    let text = fs::read_to_string(&config).unwrap();
    let broken = text.replace("connector = \"test\"", "connector = \"absent\"");
    let both = format!(
        "{broken}\n{}",
        text.replace("name = \"broken\"", "name = \"second\"")
    );
    fs::write(&config, both).unwrap();

    let out = env.rastro(&["tick"], None);

    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("broken"), "{said}");
    assert!(!said.contains("second"), "{said}");
    assert_eq!(env.runs("second"), 1);
}
