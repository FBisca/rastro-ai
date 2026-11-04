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

#[cfg(unix)]
fn not_utf8() -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(vec![0xff, 0xfe])
}

#[test]
fn S24a_half_configured_or_malformed_live_variables_fail_naming_them() {
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
