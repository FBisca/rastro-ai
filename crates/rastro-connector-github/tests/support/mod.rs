//! A fixture GitHub server and a harness that drives the real `rastro` binary.
#![allow(dead_code)]
#[path = "../../../rastro-cli/tests/support/binaries.rs"]
mod binaries;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};
use tempfile::TempDir;

pub const TOKEN: &str = "ghp_fixture_secret_token";

#[derive(Clone, Debug)]
pub struct Req {
    pub method: String,
    pub path: String,
    /// Percent-decoded query string.
    pub query: String,
    pub auth: Option<String>,
}
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}
impl Reply {
    pub fn ok(body: Value) -> Self {
        Self::status(200, body)
    }
    pub fn status(status: u16, body: Value) -> Self {
        Self {
            status,
            headers: vec![],
            body,
        }
    }
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

fn decode(text: &str) -> String {
    let bytes = text.replace('+', " ").into_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok());
        match (bytes[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Serves canned replies and records every request it receives.
pub struct Fixture {
    pub base: String,
    pub log: Arc<Mutex<Vec<Req>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Fixture {
    pub fn start(route: impl Fn(&Req, &str) -> Reply + Send + 'static) -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.server_addr().to_ip().unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_log, thread_stop, thread_base) = (log.clone(), stop.clone(), base.clone());
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(50)) else {
                    continue;
                };
                let url = request.url().to_string();
                let (path, query) = url.split_once('?').unwrap_or((&url, ""));
                let req = Req {
                    method: request.method().to_string(),
                    path: path.into(),
                    query: decode(query),
                    auth: request
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv("Authorization"))
                        .map(|h| h.value.to_string()),
                };
                thread_log.lock().unwrap().push(req.clone());
                let reply = route(&req, &thread_base);
                let mut response = tiny_http::Response::from_string(reply.body.to_string())
                    .with_status_code(reply.status);
                for (name, value) in reply
                    .headers
                    .iter()
                    .chain(&[("Content-Type".to_string(), "application/json".to_string())])
                {
                    response.add_header(
                        tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap(),
                    );
                }
                let _ = request.respond(response);
            }
        });
        Self {
            base,
            log,
            stop,
            thread: Some(thread),
        }
    }
    pub fn requests(&self) -> Vec<Req> {
        self.log.lock().unwrap().clone()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn days_ago(days: i64) -> String {
    (chrono::Utc::now() - chrono::Duration::days(days))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
pub fn issue(number: u64, title: &str, body: Option<&str>, pull: bool) -> Value {
    let mut item = json!({
        "number": number, "title": title, "body": body, "state": "open",
        "html_url": format!("https://github.com/o/p/{}/{number}", if pull { "pull" } else { "issues" }),
        "updated_at": days_ago(1),
        "labels": [{"name": "bug"}], "assignees": [{"login": "me"}],
    });
    if pull {
        item["pull_request"] = json!({});
    }
    item
}
pub fn search_page(items: Vec<Value>) -> Value {
    json!({"total_count": items.len(), "incomplete_results": false, "items": items})
}

/// The qualifier a search request asked for, e.g. `author`.
pub fn qualifier(req: &Req) -> &'static str {
    ["author", "assignee", "review-requested", "mentions"]
        .into_iter()
        .find(|q| req.query.contains(&format!("{q}:me")))
        .unwrap_or("")
}

pub struct Env {
    pub home: TempDir,
    pub tools: TempDir,
    pub api: String,
    pub token: Option<String>,
}
impl Env {
    pub fn new(api: &str) -> Self {
        let env = Self {
            home: TempDir::new().unwrap(),
            tools: TempDir::new().unwrap(),
            api: api.into(),
            token: Some(TOKEN.into()),
        };
        let plugin = env.home.path().join("plugins/github");
        fs::create_dir_all(&plugin).unwrap();
        let mut manifest: toml::Table = include_str!("../../rastro-plugin.toml").parse().unwrap();
        manifest.insert(
            "command".into(),
            connector_binary().to_str().unwrap().into(),
        );
        fs::write(plugin.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
        env.ok(&[
            "source",
            "add",
            "x",
            "--connector",
            "github",
            "--repo",
            "o/p",
            "--scope",
            "personal",
        ]);
        env
    }
    pub fn without_token(mut self) -> Self {
        self.token = None;
        let gh = self.tools.path().join("gh");
        fs::write(&gh, "#!/bin/sh\necho 'gh: not logged in' >&2\nexit 1\n").unwrap();
        Command::new("chmod").arg("+x").arg(&gh).status().unwrap();
        self
    }
    pub fn run(&self, args: &[&str]) -> Output {
        let path = format!("{}:/usr/bin:/bin", self.tools.path().display());
        let mut command = Command::new(rastro_binary());
        command
            .env_clear()
            .env("PATH", path)
            .env("HOME", self.home.path())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.home.path().join("plugins"))
            .env("RASTRO_GITHUB_API_URL", &self.api)
            .env("GH_CONFIG_DIR", self.tools.path())
            .args(args);
        if let Some(token) = &self.token {
            command.env("GH_TOKEN", token);
        }
        command.output().unwrap()
    }
    /// A command that must succeed. A failed sync exits non-zero, so use `run` for those.
    pub fn ok(&self, args: &[&str]) {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{args:?}: {} stdout: {} stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    pub fn json(&self, args: &[&str]) -> Value {
        let output = self.run(&[args, &["--json"]].concat());
        serde_json::from_slice(&output.stdout).unwrap()
    }
    pub fn sql<T: rusqlite::types::FromSql>(&self, query: &str) -> T {
        Connection::open(self.home.path().join("data/rastro.sqlite3"))
            .unwrap()
            .query_row(query, [], |row| row.get(0))
            .unwrap()
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
    /// Every byte under the Rastro home plus everything the commands printed.
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

/// The connector under test, as a private copy no concurrent build can rewrite.
fn connector_binary() -> PathBuf {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY
        .get_or_init(|| {
            binaries::private_copy(Path::new(env!("CARGO_BIN_EXE_rastro-connector-github")))
        })
        .clone()
}
/// `rastro` lives in another package, so build it next to the connector once and copy it.
fn rastro_binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let connector = Path::new(env!("CARGO_BIN_EXE_rastro-connector-github"));
        let mut build = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        build.args(["build", "--quiet", "-p", "rastro-cli"]);
        if connector.parent().is_some_and(|d| d.ends_with("release")) {
            build.arg("--release");
        }
        assert!(build.status().unwrap().success(), "building rastro failed");
        binaries::private_copy(&connector.with_file_name("rastro"))
    })
}
