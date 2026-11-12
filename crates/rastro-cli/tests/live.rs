//! Re-runnable owner check against a real repository; ignored unless asked for.
#![allow(non_snake_case)]
mod support;

use serde_json::Value;
use std::{fs, process::Command};

fn rastro(home: &std::path::Path, plugins: &std::path::Path, args: &[&str]) -> Value {
    let output = Command::new(support::rastro())
        .env("RASTRO_HOME", home)
        .env("RASTRO_PLUGIN_PATH", plugins)
        .args([args, &["--json"]].concat())
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!("{args:?}: {e}: {}", String::from_utf8_lossy(&output.stderr));
    })
}

/// What the live check was asked to do: `Ok(None)` skips it (neither variable set), `Ok(Some)`
/// runs it, and `Err` says which variable is missing, empty or not UTF-8.
fn live_config(
    repo: Option<std::ffi::OsString>,
    phrase: Option<std::ffi::OsString>,
) -> Result<Option<(String, String)>, String> {
    if repo.is_none() && phrase.is_none() {
        return Ok(None);
    }
    let read = |name: &str, value: Option<std::ffi::OsString>| match value {
        None => Err(format!("{name} is not set, but the other live variable is")),
        Some(value) => match value.into_string() {
            Ok(text) if text.is_empty() => Err(format!("{name} is empty")),
            Ok(text) => Ok(text),
            Err(_) => Err(format!("{name} is not valid UTF-8")),
        },
    };
    Ok(Some((
        read("RASTRO_LIVE_REPO", repo)?,
        read("RASTRO_LIVE_PHRASE", phrase)?,
    )))
}

/// evidence/R57: a real repository syncs and a remembered phrase is found with a working link
///
/// Needs `RASTRO_LIVE_REPO` (owner/name), `RASTRO_LIVE_PHRASE` and a GitHub token (`GH_TOKEN` or
/// `gh auth login`); run with `cargo test -p rastro-cli -- --ignored`.
#[test]
#[ignore = "talks to GitHub; set RASTRO_LIVE_REPO and RASTRO_LIVE_PHRASE"]
fn owner_syncs_a_real_repo_and_finds_a_remembered_phrase() {
    let config = live_config(
        std::env::var_os("RASTRO_LIVE_REPO"),
        std::env::var_os("RASTRO_LIVE_PHRASE"),
    )
    .unwrap_or_else(|why| panic!("the live check is half-configured: {why}"));
    let Some((repo, phrase)) = config else {
        return;
    };
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let built = Command::new(cargo)
        .args(["build", "--quiet", "-p", "rastro-connector-github"])
        .status()
        .unwrap();
    assert!(built.success(), "building the GitHub connector failed");
    let connector = support::binaries::private_copy(
        &std::path::Path::new(env!("CARGO_BIN_EXE_rastro"))
            .with_file_name("rastro-connector-github"),
    );
    let home = tempfile::tempdir().unwrap();
    let plugins = home.path().join("dev-plugins");
    fs::create_dir_all(plugins.join("github")).unwrap();
    let mut manifest: toml::Table =
        include_str!("../../rastro-connector-github/rastro-plugin.toml")
            .parse()
            .unwrap();
    manifest.insert("command".into(), connector.to_str().unwrap().into());
    fs::write(
        plugins.join("github/rastro-plugin.toml"),
        manifest.to_string(),
    )
    .unwrap();

    let added = rastro(
        home.path(),
        &plugins,
        &[
            "source",
            "add",
            "live",
            "--connector",
            "github",
            "--repo",
            &repo,
            "--scope",
            "personal",
        ],
    );
    assert!(added["error"].is_null(), "{added}");
    let synced = rastro(home.path(), &plugins, &["sync", "live"]);
    let run = &synced["result"]["runs"][0];
    assert_eq!(run["status"], "ok", "{synced}");
    assert_eq!(run["coverage"], "found", "{synced}");

    let found = rastro(home.path(), &plugins, &["search", &phrase]);
    let link = format!("https://github.com/{repo}/");
    let results = found["result"]["results"].as_array().unwrap();
    assert!(
        results.iter().any(|r| {
            r["quote"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains(&phrase.to_lowercase())
                && r["location"].as_str().unwrap().starts_with(&link)
        }),
        "no cited hit for {phrase:?} under {link}: {found}"
    );
    assert_eq!(found["result"]["coverage"][0]["state"], "found");
}

/// The GitHub API through `gh`, as the owner is logged in; the answer GitHub itself gives.
fn gh_api(path: &str, jq: &str) -> String {
    let output = Command::new("gh")
        .args(["api", path, "--jq", jq])
        .output()
        .expect("`gh` must be installed for the live check");
    assert!(
        output.status.success(),
        "gh api {path}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// Installs a built plugin under `plugins/<dir>`: its manifest pointing at a private copy of the
/// binary, plus any files the manifest names.
fn install_plugin(
    plugins: &std::path::Path,
    dir: &str,
    manifest: &str,
    binary: &std::path::Path,
    files: &[(&str, &str)],
) {
    let target = plugins.join(dir);
    fs::create_dir_all(&target).unwrap();
    let mut manifest: toml::Table = manifest.parse().unwrap();
    manifest.insert(
        "command".into(),
        support::binaries::private_copy(binary)
            .to_str()
            .unwrap()
            .into(),
    );
    fs::write(target.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
    for (name, text) in files {
        fs::write(target.join(name), text).unwrap();
    }
}

/// One `rastro mcp --scope <scope>` tool call over stdio; returns the tool's v2 envelope.
fn scoped_mcp_call(
    home: &std::path::Path,
    plugins: &std::path::Path,
    scope: &str,
    tool: &str,
    arguments: Value,
) -> Value {
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;
    let mut child = Command::new(support::rastro())
        .env("RASTRO_HOME", home)
        .env("RASTRO_PLUGIN_PATH", plugins)
        .args(["mcp", "--scope", scope])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut send = |message: Value| {
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    };
    send(
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-11-25", "capabilities": {},
        "clientInfo": {"name": "live-check", "version": "1.0"}}}),
    );
    let initialized = lines.next().unwrap().unwrap();
    assert!(initialized.contains("\"result\""), "{initialized}");
    send(serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    send(
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": tool, "arguments": arguments}}),
    );
    let answer: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    drop(stdin);
    let _ = child.wait();
    assert_ne!(answer["result"]["isError"], true, "{answer}");
    answer["result"]["structuredContent"].clone()
}

/// Number of the pull request or issue at a GitHub URL.
fn item_number(location: &str) -> &str {
    location.rsplit('/').next().unwrap()
}

/// claims/R37: after `sync --reread` on a real repository, the owner's account has
/// `code:author` claims on that repository's pull requests and issues, `why` cites the
/// `/user/id` and `/user/type` observations, and both match what GitHub says. The same claim
/// read through `rastro mcp --scope` shows only that scope's supports.
///
/// Needs `RASTRO_LIVE_REPO` (owner/name), `RASTRO_LIVE_GITHUB_LOGIN` (an account that authored a
/// pull request or issue in that repository, usually the owner's) and `gh auth login` or
/// `GH_TOKEN`; run with `cargo test -p rastro-cli --test live -- --ignored the_owners_account`.
#[test]
#[ignore = "talks to GitHub; set RASTRO_LIVE_REPO and RASTRO_LIVE_GITHUB_LOGIN"]
fn the_owners_account_claims_and_receipts_match_github() {
    let config = live_config(
        std::env::var_os("RASTRO_LIVE_REPO"),
        std::env::var_os("RASTRO_LIVE_GITHUB_LOGIN"),
    )
    .unwrap_or_else(|why| panic!("the live check is half-configured: {why}"));
    let Some((repo, login)) = config else {
        return;
    };
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let built = Command::new(cargo)
        .args([
            "build",
            "--quiet",
            "-p",
            "rastro-connector-github",
            "-p",
            "rastro-extractor-github",
        ])
        .status()
        .unwrap();
    assert!(built.success(), "building the GitHub plugins failed");
    let target = std::path::Path::new(env!("CARGO_BIN_EXE_rastro"));
    let home = tempfile::tempdir().unwrap();
    let plugins = home.path().join("dev-plugins");
    install_plugin(
        &plugins,
        "github",
        include_str!("../../rastro-connector-github/rastro-plugin.toml"),
        &target.with_file_name("rastro-connector-github"),
        &[],
    );
    install_plugin(
        &plugins,
        "github-extractor",
        include_str!("../../rastro-extractor-github/rastro-plugin.toml"),
        &target.with_file_name("rastro-extractor-github"),
        &[(
            "vocabulary.toml",
            include_str!("../../rastro-extractor-github/vocabulary.toml"),
        )],
    );

    // The same repository in two scopes, so a scoped read has something to hide.
    for (name, scope) in [("live-personal", "personal"), ("live-work", "work")] {
        let added = rastro(
            home.path(),
            &plugins,
            &[
                "source",
                "add",
                name,
                "--connector",
                "github",
                "--repo",
                &repo,
                "--scope",
                scope,
            ],
        );
        assert!(added["error"].is_null(), "{added}");
    }
    for args in [vec!["sync"], vec!["sync", "--reread"]] {
        let synced = rastro(home.path(), &plugins, &args);
        assert!(synced["error"].is_null(), "{synced}");
        for run in synced["result"]["runs"].as_array().unwrap() {
            assert_eq!(run["status"], "ok", "{synced}");
        }
    }

    // GitHub's own answer for the account id.
    let id = gh_api(&format!("users/{login}"), ".id");
    let account = format!("github:account/{id}");
    let about = rastro(home.path(), &plugins, &["about", &account, "--limit", "50"]);
    assert!(about["error"].is_null(), "{about}");
    let claims = about["result"]["claims"].as_array().unwrap();
    let authored: Vec<&Value> = claims
        .iter()
        .filter(|c| c["predicate"] == "code:author" && c["object"]["identifier"] == account)
        .collect();
    assert!(
        !authored.is_empty(),
        "no code:author claim for {account} in {repo}: {about}"
    );
    for claim in &authored {
        let subject = claim["subject"]["identifier"].as_str().unwrap();
        assert!(subject.contains(&repo), "{subject} is not in {repo}");
    }

    let claim = authored[0];
    let why = rastro(
        home.path(),
        &plugins,
        &["why", claim["id"].as_str().unwrap()],
    );
    assert!(why["error"].is_null(), "{why}");
    let supports = why["result"]["supports"].as_array().unwrap();
    let current: Vec<&Value> = supports.iter().filter(|s| s["current"] == true).collect();
    assert!(!current.is_empty(), "{why}");
    let link = format!("https://github.com/{repo}/");
    for support in &current {
        let spans = support["spans"].as_array().unwrap();
        let field = |name: &str| {
            spans
                .iter()
                .find(|s| s["field"] == name)
                .unwrap_or_else(|| panic!("no {name} span: {support}"))
        };
        assert_eq!(
            field("/user/id")["excerpt"]["text"],
            id.as_str(),
            "{support}"
        );
        assert_eq!(field("/user/type")["excerpt"]["text"], "User", "{support}");
        for span in spans {
            let location = span["location"].as_str().unwrap();
            assert!(location.starts_with(&link), "{location} is not in {repo}");
            // GitHub agrees that this item's author is this account.
            let path = format!("repos/{repo}/issues/{}", item_number(location));
            assert_eq!(gh_api(&path, ".user.id"), id, "{path}");
            assert_eq!(gh_api(&path, ".user.type"), "User", "{path}");
        }
    }

    // One scoped answer: only that scope's supports, and fewer than the unscoped read's.
    let unscoped = supports.len();
    for scope in ["personal", "work"] {
        let scoped = scoped_mcp_call(
            home.path(),
            &plugins,
            scope,
            "why",
            serde_json::json!({"claim": claim["id"]}),
        );
        assert!(scoped["error"].is_null(), "{scoped}");
        let rows = scoped["result"]["supports"].as_array().unwrap();
        assert!(!rows.is_empty() && rows.len() <= unscoped, "{scoped}");
        for row in rows {
            for span in row["spans"].as_array().unwrap() {
                assert_eq!(span["scope"], scope, "{scoped}");
            }
        }
    }
}

#[cfg(unix)]
fn not_utf8() -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(vec![0xff, 0xfe])
}

/// evidence/R59: half configured or malformed live variables fail naming them
#[test]
fn half_configured_or_malformed_live_variables_fail_naming_them() {
    let os = |s: &str| Some(std::ffi::OsString::from(s));
    assert_eq!(live_config(None, None), Ok(None));
    assert_eq!(
        live_config(os("o/p"), os("phrase")),
        Ok(Some(("o/p".into(), "phrase".into())))
    );
    for (repo, phrase, named) in [
        (os("o/p"), None, "RASTRO_LIVE_PHRASE"),
        (None, os("phrase"), "RASTRO_LIVE_REPO"),
        (os("o/p"), os(""), "RASTRO_LIVE_PHRASE"),
        (os(""), os("phrase"), "RASTRO_LIVE_REPO"),
        (os(""), os(""), "RASTRO_LIVE_REPO"),
    ] {
        let error = live_config(repo, phrase).unwrap_err();
        assert!(error.contains(named), "{error}");
    }
    // One variable set and the other empty is half-configured, not skipped.
    assert!(live_config(os(""), None).is_err());
    #[cfg(unix)]
    {
        let error = live_config(Some(not_utf8()), os("phrase")).unwrap_err();
        assert!(
            error.contains("RASTRO_LIVE_REPO") && error.contains("UTF-8"),
            "{error}"
        );
        let error = live_config(os("o/p"), Some(not_utf8())).unwrap_err();
        assert!(
            error.contains("RASTRO_LIVE_PHRASE") && error.contains("UTF-8"),
            "{error}"
        );
    }
}

/// identity/R31: on a copy of the owner's database, after the owner synced a GitHub repository and
/// Linear and ran `rastro identity me`, `about me` lists a GitHub and a Linear account, and
/// `rastro identity candidates` answers without error.
///
/// Needs `RASTRO_LIVE_DB`: a COPY of the owner's database (for example made with
/// `sqlite3 ~/.local/share/rastro/rastro.sqlite3 ".backup /tmp/rastro-copy.sqlite3"`). The test
/// copies it again into a temporary home, so the file it is given is never written. Run with
/// `cargo test -p rastro-cli --test live -- --ignored the_owners_copy_lists_github_and_linear_accounts_and_runs_candidates`.
#[test]
#[ignore = "reads a copy of the owner's database; set RASTRO_LIVE_DB"]
fn the_owners_copy_lists_github_and_linear_accounts_and_runs_candidates() {
    let Some(source) = std::env::var_os("RASTRO_LIVE_DB") else {
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join("data");
    fs::create_dir_all(&data).unwrap();
    fs::copy(&source, data.join("rastro.sqlite3")).unwrap();
    let plugins = home.path().join("plugins");
    fs::create_dir_all(&plugins).unwrap();

    let about = rastro(home.path(), &plugins, &["about", "me"]);
    let person = &about["result"]["person"];
    let accounts: Vec<&str> = person["accounts"]
        .as_array()
        .unwrap_or_else(|| panic!("about me has no person accounts: {about}"))
        .iter()
        .filter_map(|a| a["identifier"].as_str())
        .collect();
    assert!(
        accounts.iter().any(|a| a.starts_with("github:")),
        "no GitHub account held by me: {accounts:?}"
    );
    assert!(
        accounts.iter().any(|a| a.starts_with("linear:")),
        "no Linear account held by me: {accounts:?}"
    );

    let listed = rastro(home.path(), &plugins, &["identity", "candidates"]);
    assert!(
        listed["result"]["candidates"].is_array(),
        "candidates failed: {listed}"
    );
}

/// labels/S1, S3: on a copy of the owner's database, `about me` labels pull requests and Linear
/// issues with a cited title, and the read stays fast with the label lookups in it.
///
/// Needs `RASTRO_LIVE_DB`, as above; it prints how long `about <entity>` took (`RASTRO_LIVE_ENTITY`,
/// default `me`). Run with
/// `cargo test -p rastro-cli --test live -- --ignored --nocapture the_owners_copy_labels_what_about_me_returns`.
#[test]
#[ignore = "reads a copy of the owner's database; set RASTRO_LIVE_DB"]
fn the_owners_copy_labels_what_about_me_returns() {
    let Some(source) = std::env::var_os("RASTRO_LIVE_DB") else {
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join("data");
    fs::create_dir_all(&data).unwrap();
    fs::copy(&source, data.join("rastro.sqlite3")).unwrap();
    let plugins = home.path().join("plugins");
    fs::create_dir_all(&plugins).unwrap();
    // A writing open applies the pending migrations, as `rastro mcp` and `rastro sync` do.
    drop(rastro_storage_sqlite::SqliteStorage::open(&data.join("rastro.sqlite3")).unwrap());
    let entity = std::env::var("RASTRO_LIVE_ENTITY").unwrap_or_else(|_| "me".into());
    let started = std::time::Instant::now();
    let about = rastro(home.path(), &plugins, &["about", &entity, "--limit", "50"]);
    println!("about {entity}, every scope: {:?}", started.elapsed());
    let claims = about["result"]["claims"].as_array().unwrap();
    let labelled = claims
        .iter()
        .flat_map(|c| [&c["subject_label"], &c["object_label"]])
        .filter(|label| label.is_object())
        .count();
    println!("{} claims, {labelled} labelled sides", claims.len());
    assert!(labelled > 0, "no label on any side of `about me`: {about}");
    for label in claims
        .iter()
        .flat_map(|c| [&c["subject_label"], &c["object_label"]])
        .filter(|label| label.is_object())
    {
        assert!(label["text"].as_str().unwrap().chars().count() <= 200);
        assert!(label["trust_class"].is_string());
    }
}
