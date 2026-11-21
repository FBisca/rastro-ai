//! `rastro check <source>`: a no-write authentication probe through the real connectors, and the
//! report `rastro source add` makes of it. A fake API on loopback answers the one cheap call.
#![allow(non_snake_case)]
mod support;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex, OnceLock},
};
use tempfile::TempDir;

const LINEAR_KEY: &str = "lin_api_check_fixture_secret";
const GITHUB_TOKEN: &str = "ghp_check_fixture_secret";

/// A loopback API that accepts one Linear key and one GitHub token and records each request.
struct Api {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
}
impl Api {
    fn start() -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.server_addr().to_ip().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        std::thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let auth = request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Authorization"))
                    .map(|h| h.value.to_string())
                    .unwrap_or_default();
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);
                log.lock()
                    .unwrap()
                    .push(format!("{} {}", request.method(), request.url()));
                let (status, reply) = match request.url() {
                    "/graphql" if auth == LINEAR_KEY => {
                        (200, json!({"data": {"viewer": {"id": "u1", "app": false}}}))
                    }
                    "/graphql" => (
                        401,
                        json!({"errors": [{"extensions": {"code": "AUTHENTICATION_ERROR"}}]}),
                    ),
                    "/user" if auth == format!("Bearer {GITHUB_TOKEN}") => {
                        (200, json!({"login": "owner", "id": 1}))
                    }
                    _ => (401, json!({"message": "Bad credentials"})),
                };
                let mut response =
                    tiny_http::Response::from_string(reply.to_string()).with_status_code(status);
                response.add_header(
                    tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
                );
                let _ = request.respond(response);
            }
        });
        Self { base, requests }
    }
    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

/// A home with a real connector installed and a fake API to talk to.
struct Env {
    home: TempDir,
    api: Api,
}
impl Env {
    fn new() -> Self {
        static LINEAR: OnceLock<std::path::PathBuf> = OnceLock::new();
        static GITHUB: OnceLock<std::path::PathBuf> = OnceLock::new();
        let env = Self {
            home: TempDir::new().unwrap(),
            api: Api::start(),
        };
        let linear = LINEAR.get_or_init(|| support::connector("rastro-connector-linear"));
        let github = GITHUB.get_or_init(|| support::connector("rastro-connector-github"));
        env.install(
            "rastro-connector-linear",
            include_str!("../../rastro-connector-linear/rastro-plugin.toml"),
            linear,
        );
        env.install(
            "rastro-connector-github",
            include_str!("../../rastro-connector-github/rastro-plugin.toml"),
            github,
        );
        env
    }
    fn install(&self, name: &str, manifest: &str, binary: &Path) {
        let dir = self.home.path().join("config/plugins").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("rastro-plugin.toml"), manifest).unwrap();
        let command = dir.join(name);
        fs::copy(binary, &command).unwrap();
        fs::set_permissions(&command, fs::Permissions::from_mode(0o755)).unwrap();
    }
    /// `rastro` with no credential of its own: an empty `PATH` hides the keychain and `gh`.
    fn rastro(&self, credentials: &[(&str, &str)], args: &[&str]) -> Output {
        let nowhere = self.home.path().join("nowhere");
        let mut command = Command::new(support::rastro());
        command
            .env_clear()
            .env("RASTRO_HOME", self.home.path())
            .env("HOME", self.home.path())
            .env("PATH", nowhere)
            .env("RASTRO_LINEAR_API_URL", &self.api.base)
            .env("RASTRO_GITHUB_API_URL", &self.api.base)
            .envs(credentials.iter().copied())
            .args(args)
            .stdin(Stdio::null());
        command.output().unwrap()
    }
    fn configure(&self, sources: &str) {
        fs::write(self.home.path().join("config/config.toml"), sources).unwrap();
    }
    fn linear_source(&self) {
        self.configure(
            "[[sources]]\nname = \"work\"\nconnector = \"linear\"\nselector = {}\nscope = \"work\"\n\
             freshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\nkeep_original = false\n",
        );
    }
    fn github_source(&self) {
        self.configure(
            "[[sources]]\nname = \"project\"\nconnector = \"github\"\nselector = {repo = \"o/p\"}\n\
             scope = \"work\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
             keep_original = false\n",
        );
    }
    /// Every file under the home but the installed plugins, by path.
    fn state(&self) -> BTreeMap<String, Vec<u8>> {
        fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                let name = path.strip_prefix(root).unwrap().display().to_string();
                if name == "config/plugins" || name.starts_with("nowhere") {
                    continue;
                }
                if path.is_dir() {
                    walk(root, &path, out);
                } else {
                    out.insert(name, fs::read(&path).unwrap());
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(self.home.path(), self.home.path(), &mut out);
        out
    }
}
fn shown(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// A Linear source with a valid key reports the credential source and success, and writes nothing.
/// spec: Source credentials can be checked without writing
#[test]
fn a_linear_source_with_a_valid_key_reports_the_credential_and_authenticates() {
    let env = Env::new();
    env.linear_source();
    let before = env.state();
    let output = env.rastro(&[("LINEAR_API_KEY", LINEAR_KEY)], &["check", "work"]);
    let text = shown(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("LINEAR_API_KEY"), "{text}");
    assert!(text.contains("authenticated"), "{text}");
    assert!(!text.contains(LINEAR_KEY), "{text}");
    assert_eq!(env.api.requests(), ["POST /graphql"]);
    assert_eq!(env.state(), before, "a check wrote");
}

/// A GitHub source with a valid token reports the credential source and success.
/// spec: Source credentials can be checked without writing
#[test]
fn a_github_source_with_a_valid_token_reports_the_credential_and_authenticates() {
    let env = Env::new();
    env.github_source();
    let before = env.state();
    let output = env.rastro(&[("GH_TOKEN", GITHUB_TOKEN)], &["check", "project"]);
    let text = shown(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("GH_TOKEN"), "{text}");
    assert!(text.contains("authenticated"), "{text}");
    assert!(!text.contains(GITHUB_TOKEN), "{text}");
    assert_eq!(env.api.requests(), ["GET /user"]);
    assert_eq!(env.state(), before, "a check wrote");
}

/// A rejected key fails with a fix, exits 1, leaks no secret and writes nothing.
/// spec: Source credentials can be checked without writing
#[test]
fn a_rejected_key_fails_with_a_fix_and_exits_unsuccessfully() {
    let env = Env::new();
    env.linear_source();
    let before = env.state();
    let wrong = "lin_api_wrong_secret";
    let output = env.rastro(&[("LINEAR_API_KEY", wrong)], &["check", "work"]);
    let text = shown(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("rejected"), "{text}");
    assert!(
        text.contains("Fix:") && text.contains("LINEAR_API_KEY"),
        "{text}"
    );
    assert!(!text.contains(wrong), "{text}");
    assert_eq!(env.state(), before, "a check wrote");
}

/// With no credential at all the check names what to set, sends no request and exits 1.
/// spec: Source credentials can be checked without writing
#[test]
fn a_missing_credential_fails_with_a_fix_and_sends_no_request() {
    let env = Env::new();
    env.github_source();
    let before = env.state();
    let output = env.rastro(&[], &["check", "project"]);
    let text = shown(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("missing"), "{text}");
    assert!(text.contains("Fix:") && text.contains("GH_TOKEN"), "{text}");
    assert!(env.api.requests().is_empty());
    assert_eq!(env.state(), before, "a check wrote");
}

/// An agent listing MCP tools sees no check tool, though the CLI has the command.
/// spec: Source credentials can be checked without writing
#[test]
fn the_check_is_a_cli_command_and_no_mcp_tool() {
    let env = Env::new();
    env.linear_source();
    let help = env.rastro(&[], &["check", "--help"]);
    assert!(help.status.success(), "{}", shown(&help));
    let mut child = Command::new(support::rastro())
        .env("RASTRO_HOME", env.home.path())
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for message in [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": {"name": "t", "version": "1"}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    ] {
        writeln!(stdin, "{message}").unwrap();
    }
    drop(stdin);
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    child.wait().unwrap();
    let listed = out
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|m| m["id"] == 2)
        .expect("a tools/list answer");
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(!names.is_empty());
    assert!(
        names
            .iter()
            .all(|n| *n != "check" && !n.contains("source") && !n.contains("auth")),
        "{names:?}"
    );
}

/// Adding a source whose credentials fail keeps it configured and explains the failure and fix.
/// spec: Source setup reports authentication
#[test]
fn adding_a_source_with_bad_credentials_keeps_it_and_explains_the_failure() {
    let env = Env::new();
    let wrong = "lin_api_wrong_secret";
    let add = env.rastro(
        &[("LINEAR_API_KEY", wrong)],
        &[
            "source",
            "add",
            "work",
            "--connector",
            "linear",
            "--scope",
            "work",
        ],
    );
    let text = shown(&add);
    assert!(add.status.success(), "{text}");
    assert!(text.contains("Source registered: work"), "{text}");
    assert!(text.contains("rejected"), "{text}");
    assert!(
        text.contains("Fix:") && text.contains("LINEAR_API_KEY"),
        "{text}"
    );
    assert!(!text.contains(wrong), "{text}");
    let list = shown(&env.rastro(&[], &["source", "list"]));
    assert!(list.lines().any(|l| l == "work"), "{list}");
    assert!(list.contains("Connector: linear"), "{list}");
}
