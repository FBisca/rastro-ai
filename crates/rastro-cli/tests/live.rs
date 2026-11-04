//! Re-runnable owner check against a real repository; ignored unless asked for.
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

/// evidence/R57: a real repository syncs and a remembered phrase is found with a working link
///
/// Needs `RASTRO_LIVE_REPO` (owner/name), `RASTRO_LIVE_PHRASE` and a GitHub token (`GH_TOKEN` or
/// `gh auth login`); run with `cargo test -p rastro-cli -- --ignored`.
#[test]
#[ignore = "talks to GitHub; set RASTRO_LIVE_REPO and RASTRO_LIVE_PHRASE"]
fn owner_syncs_a_real_repo_and_finds_a_remembered_phrase() {
    let (Ok(repo), Ok(phrase)) = (
        std::env::var("RASTRO_LIVE_REPO"),
        std::env::var("RASTRO_LIVE_PHRASE"),
    ) else {
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
