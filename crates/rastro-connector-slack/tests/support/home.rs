//! A temporary Rastro home with the Slack connector installed, driving the real `rastro` binary
//! against the scripted Slack API. The keychain is a stand-in on `PATH`, so no test ever reads
//! the real one.
#![allow(dead_code)]
use rusqlite::Connection;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

pub struct Env {
    pub home: TempDir,
    pub tools: TempDir,
    pub api: String,
    /// `SLACK_USER_TOKEN`, when set.
    pub token: Option<String>,
    rastro: PathBuf,
}
impl Env {
    /// A home with the connector installed and source `s` (connector `slack`, channel
    /// [`super::api::CHANNEL`], scope `work`, window from 2026-08-01), and an empty keychain.
    pub fn new(rastro: &Path, api: &str) -> Self {
        let env = Self {
            home: TempDir::new().unwrap(),
            tools: TempDir::new().unwrap(),
            api: api.into(),
            token: Some(super::api::TOKEN.into()),
            rastro: rastro.into(),
        };
        let dir = env.home.path().join("plugins/slack");
        fs::create_dir_all(&dir).unwrap();
        let mut manifest: toml::Table = include_str!("../../rastro-plugin.toml").parse().unwrap();
        manifest.insert(
            "command".into(),
            super::connector_binary().to_str().unwrap().into(),
        );
        fs::write(dir.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
        fs::create_dir_all(env.home.path().join("config")).unwrap();
        fs::write(
            env.home.path().join("config/config.toml"),
            format!(
                "[[sources]]\nname = \"s\"\nconnector = \"slack\"\n\
                 selector = {{channel = \"{}\"}}\nscope = \"work\"\nfreshness_rule = \"1h\"\n\
                 window_start = \"2026-08-01T00:00:00Z\"\nkeep_original = false\n",
                super::api::CHANNEL
            ),
        )
        .unwrap();
        env.keychain(None);
        env
    }
    /// The `security` command the connector finds on `PATH`: it answers `token` for the item
    /// `rastro-slack`, or fails as a keychain without it does.
    pub fn keychain(&self, token: Option<&str>) {
        let script = match token {
            Some(token) => format!(
                "#!/bin/sh\nfor a in \"$@\"; do [ \"$a\" = rastro-slack ] && echo '{token}' && exit 0; done\nexit 44\n"
            ),
            None => "#!/bin/sh\necho 'security: The specified item could not be found in the keychain.' >&2\nexit 44\n".into(),
        };
        let security = self.tools.path().join("security");
        fs::write(&security, script).unwrap();
        Command::new("chmod")
            .arg("+x")
            .arg(&security)
            .status()
            .unwrap();
    }
    pub fn run(&self, args: &[&str]) -> Output {
        let path = format!("{}:/usr/bin:/bin", self.tools.path().display());
        let mut command = Command::new(&self.rastro);
        command
            .env_clear()
            .env("PATH", path)
            .env("HOME", self.home.path())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.home.path().join("plugins"))
            .env("RASTRO_SLACK_API_URL", &self.api)
            .args(args);
        if let Some(token) = &self.token {
            command.env("SLACK_USER_TOKEN", token);
        }
        command.output().unwrap()
    }
    /// A command that must succeed. A failed sync exits non-zero, so use `run` for those.
    pub fn ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{args:?}: {} stdout: {} stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    /// The `result` of a command run with `--json`, whether or not it succeeded.
    pub fn json(&self, args: &[&str]) -> Value {
        let output = self.run(&[args, &["--json"]].concat());
        serde_json::from_slice::<Value>(&output.stdout)
            .unwrap_or_else(|_| panic!("{args:?}: {}", String::from_utf8_lossy(&output.stderr)))
            ["result"]
            .clone()
    }
    /// Syncs `s` (with any extra arguments) and returns its run.
    pub fn sync(&self, extra: &[&str]) -> Value {
        self.json(&[&["sync", "s"][..], extra].concat())["runs"][0].clone()
    }
    pub fn rows(&self, query: &str) -> Vec<Vec<String>> {
        let db = Connection::open(self.home.path().join("data/rastro.sqlite3")).unwrap();
        let mut statement = db.prepare(query).unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |row| {
                (0..columns)
                    .map(|i| {
                        Ok(match row.get::<_, rusqlite::types::Value>(i)? {
                            rusqlite::types::Value::Integer(n) => n.to_string(),
                            rusqlite::types::Value::Text(t) => t,
                            _ => String::new(),
                        })
                    })
                    .collect::<rusqlite::Result<Vec<String>>>()
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
    /// The first column of every row.
    pub fn column(&self, query: &str) -> Vec<String> {
        self.rows(query).into_iter().map(|r| r[0].clone()).collect()
    }
    /// Whether `needle` is anywhere under the Rastro home (database, config, logs) or in what
    /// the commands printed.
    pub fn leaks(&self, needle: &str, outputs: &[Output]) -> bool {
        fn walk(dir: &Path, needle: &[u8]) -> bool {
            fs::read_dir(dir).unwrap().flatten().any(|entry| {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, needle)
                } else {
                    fs::read(&path)
                        .unwrap()
                        .windows(needle.len())
                        .any(|w| w == needle)
                }
            })
        }
        walk(self.home.path(), needle.as_bytes())
            || outputs.iter().any(|o| {
                [&o.stdout, &o.stderr]
                    .iter()
                    .any(|b| b.windows(needle.len()).any(|w| w == needle.as_bytes()))
            })
    }
}
