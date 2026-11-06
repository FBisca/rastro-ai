//! A fixture GitHub server and a home with the real connector and extractor installed, driving
//! the real `rastro` binary.
#![allow(dead_code)]
#[path = "../../../rastro-cli/tests/support/binaries.rs"]
mod binaries;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
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

/// What the fixture server answers: every search returns `items`; an item's comments are
/// `comments[number]`.
#[derive(Clone, Default)]
pub struct World {
    pub items: Vec<Value>,
    pub comments: BTreeMap<u64, Vec<Value>>,
}

pub struct Fixture {
    pub base: String,
    pub world: Arc<Mutex<World>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Fixture {
    pub fn start(world: World) -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.server_addr().to_ip().unwrap());
        let world = Arc::new(Mutex::new(world));
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_world, thread_stop) = (world.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(50)) else {
                    continue;
                };
                let url = request.url().to_string();
                let path = url.split_once('?').map_or(&*url, |(p, _)| p);
                let world = thread_world.lock().unwrap().clone();
                let comments = path
                    .strip_prefix("/repos/o/p/issues/")
                    .and_then(|p| p.strip_suffix("/comments"))
                    .and_then(|n| n.parse::<u64>().ok());
                let (status, body) = match (path, comments) {
                    ("/user", _) => (200, json!({"login": "me"})),
                    ("/search/issues", _) => (
                        200,
                        json!({"total_count": world.items.len(), "incomplete_results": false,
                            "items": world.items}),
                    ),
                    (_, Some(number)) => (
                        200,
                        json!(world.comments.get(&number).cloned().unwrap_or_default()),
                    ),
                    (path, _) if path.starts_with("/users/") => {
                        let login = path
                            .trim_start_matches("/users/")
                            .replace("%5B", "[")
                            .replace("%5D", "]");
                        let user = world
                            .items
                            .iter()
                            .chain(world.comments.values().flatten())
                            .map(|item| &item["user"])
                            .find(|user| user["login"] == login);
                        match user {
                            Some(user) => (200, json!({"id": user["id"], "name": null})),
                            None => (404, json!({"message": "Not Found"})),
                        }
                    }
                    _ => (404, json!({"message": "Not Found"})),
                };
                let content =
                    tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                        .unwrap();
                let response = tiny_http::Response::from_string(body.to_string())
                    .with_status_code(status)
                    .with_header(content);
                let _ = request.respond(response);
            }
        });
        Self {
            base,
            world,
            stop,
            thread: Some(thread),
        }
    }
    /// Changes what the server answers from now on.
    pub fn change(&self, edit: impl FnOnce(&mut World)) {
        edit(&mut self.world.lock().unwrap());
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
pub fn user(id: u64, login: &str, kind: &str) -> Value {
    json!({"id": id, "login": login, "type": kind})
}
/// An issue or pull request in `o/p`.
pub fn item(number: u64, pull: bool, state: &str, title: &str, body: &str, user: Value) -> Value {
    let path = if pull { "pull" } else { "issues" };
    let mut item = json!({
        "number": number, "title": title, "body": body, "state": state,
        "html_url": format!("https://github.com/o/p/{path}/{number}"),
        "updated_at": days_ago(1), "user": user, "labels": [], "assignees": [],
    });
    if pull {
        item["pull_request"] = json!({});
    }
    item
}
pub fn comment(url: &str, body: &str, user: Value) -> Value {
    json!({"html_url": url, "body": body, "updated_at": days_ago(1), "user": user})
}

pub struct Env {
    pub home: TempDir,
    pub api: String,
}
impl Env {
    /// A home with the GitHub connector and the GitHub extractor installed, and source `x`
    /// reading `o/p` in the personal scope.
    pub fn new(api: &str) -> Self {
        let env = Self {
            home: TempDir::new().unwrap(),
            api: api.into(),
        };
        env.install(
            "github",
            include_str!("../../../rastro-connector-github/rastro-plugin.toml"),
            connector_binary(),
        );
        let extractor = env.install(
            "rastro-extractor-github",
            include_str!("../../rastro-plugin.toml"),
            extractor_binary(),
        );
        fs::write(
            extractor.join("vocabulary.toml"),
            include_str!("../../vocabulary.toml"),
        )
        .unwrap();
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
    fn install(&self, id: &str, manifest: &str, command: &Path) -> PathBuf {
        let dir = self.home.path().join("plugins").join(id);
        fs::create_dir_all(&dir).unwrap();
        let mut manifest: toml::Table = manifest.parse().unwrap();
        manifest.insert("command".into(), command.to_str().unwrap().into());
        fs::write(dir.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
        dir
    }
    pub fn run(&self, args: &[&str]) -> Output {
        Command::new(rastro_binary())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.home.path())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.home.path().join("plugins"))
            .env("RASTRO_GITHUB_API_URL", &self.api)
            .env("GH_CONFIG_DIR", self.home.path())
            .env("GH_TOKEN", "ghp_fixture_token")
            .args(args)
            .output()
            .unwrap()
    }
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
    /// The `result` of a command run with `--json`.
    pub fn json(&self, args: &[&str]) -> Value {
        let text = self.ok(&[args, &["--json"]].concat());
        serde_json::from_str::<Value>(&text).unwrap()["result"].clone()
    }
    /// Syncs `x` (with any extra arguments) and returns its extraction report.
    pub fn sync(&self, extra: &[&str]) -> Value {
        let synced = self.json(&[&["sync", "x"][..], extra].concat());
        synced["runs"][0]["extraction"].clone()
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
    /// The id of the claim with this subject identifier, predicate and object (an entity
    /// identifier or a literal value).
    pub fn claim(&self, subject: &str, predicate: &str, object: &str) -> String {
        let ids = self.column(&format!(
            "SELECT c.id FROM claim c JOIN entity s ON s.id = c.subject
             LEFT JOIN entity o ON o.id = c.object_entity
             WHERE s.identifier = '{subject}' AND c.predicate = '{predicate}'
             AND coalesce(o.identifier, c.object_value) = '{object}'"
        ));
        assert_eq!(ids.len(), 1, "{subject} {predicate} {object}: {ids:?}");
        ids[0].clone()
    }
    /// `rastro why` on that claim.
    pub fn why(&self, subject: &str, predicate: &str, object: &str) -> Value {
        self.json(&["why", &self.claim(subject, predicate, object), "--full"])
    }
}

/// The extractor under test, as a private copy no concurrent build can rewrite.
pub fn extractor_binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        binaries::private_copy(Path::new(env!("CARGO_BIN_EXE_rastro-extractor-github")))
    })
}
/// `rastro` and the GitHub connector live in other packages: build them next to the extractor
/// once and copy them.
fn built(name: &str) -> PathBuf {
    static BUILT: OnceLock<()> = OnceLock::new();
    let extractor = Path::new(env!("CARGO_BIN_EXE_rastro-extractor-github"));
    BUILT.get_or_init(|| {
        let mut build = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        build.args([
            "build",
            "--quiet",
            "-p",
            "rastro-cli",
            "-p",
            "rastro-connector-github",
        ]);
        if extractor.parent().is_some_and(|d| d.ends_with("release")) {
            build.arg("--release");
        }
        assert!(build.status().unwrap().success(), "building rastro failed");
    });
    binaries::private_copy(&extractor.with_file_name(name))
}
fn rastro_binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| built("rastro"))
}
fn connector_binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| built("rastro-connector-github"))
}
