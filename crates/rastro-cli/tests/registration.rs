#![allow(non_snake_case)]
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::{path::Path, process::Command};
use tempfile::TempDir;

fn command(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rastro"));
    cmd.env("RASTRO_HOME", home);
    cmd
}
fn run(cmd: &mut Command, args: &[&str]) -> String {
    let output = cmd.args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
const ADD: &[&str] = &[
    "source",
    "add",
    "project",
    "--connector",
    "github",
    "--repo",
    "o/p",
    "--scope",
    "personal",
];

#[test]
fn S1_source_add_then_list_shows_the_source() {
    let home = TempDir::new().unwrap();
    let before = Utc::now();
    let added = run(&mut command(home.path()), &[ADD, &["--json"]].concat());
    let added: Value = serde_json::from_str(&added).unwrap();
    assert_eq!(added["schema_version"], 1);
    assert_eq!(added["operation"], "source.add");
    let source = &added["result"]["source"];
    assert_eq!(source["name"], "project");
    assert_eq!(source["connector"], "github");
    assert_eq!(source["selector"]["repo"], "o/p");
    assert_eq!(source["scope"], "personal");
    assert_eq!(source["freshness_rule"], "1h");
    assert_eq!(source["keep_original"], false);
    let start: DateTime<Utc> = source["window_start"].as_str().unwrap().parse().unwrap();
    let age = before - start;
    assert!((age.num_seconds() - 90 * 86400).abs() < 5);
    let human = run(&mut command(home.path()), &["source", "list"]);
    for field in [
        "project",
        "github",
        "o/p",
        "personal",
        "1h",
        source["window_start"].as_str().unwrap(),
    ] {
        assert!(human.contains(field), "missing {field}: {human}");
    }
    let listed: Value = serde_json::from_str(&run(
        &mut command(home.path()),
        &["source", "list", "--json"],
    ))
    .unwrap();
    assert_eq!(listed["schema_version"], 1);
    assert_eq!(listed["operation"], "source.list");
    assert_eq!(listed["result"]["sources"][0], *source);
    let config: toml::Value = std::fs::read_to_string(home.path().join("config/config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let saved = &config["sources"][0];
    for field in [
        "name",
        "connector",
        "scope",
        "freshness_rule",
        "window_start",
    ] {
        assert_eq!(saved[field].as_str(), source[field].as_str());
    }
    assert_eq!(saved["selector"]["repo"].as_str(), Some("o/p"));
    assert_eq!(saved["keep_original"].as_bool(), Some(false));
}

#[test]
fn S2_registered_source_has_never_looked_coverage() {
    let home = TempDir::new().unwrap();
    run(&mut command(home.path()), ADD);
    assert!(run(&mut command(home.path()), &["coverage", "project"]).contains("never looked"));
    let json: Value = serde_json::from_str(&run(
        &mut command(home.path()),
        &["coverage", "project", "--json"],
    ))
    .unwrap();
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["operation"], "coverage");
    assert_eq!(json["result"]["coverage"][0]["state"], "never looked");
    assert_eq!(json["result"]["coverage"][0]["source"], "project");
    let missing = command(home.path())
        .args(["coverage", "missing", "--json"])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    let error: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(error["schema_version"], 1);
    assert!(error["error"].as_str().unwrap().contains("unknown source"));
}

#[test]
fn S3_locations_follow_rastro_xdg_and_home() {
    for mode in 0..3 {
        let root = TempDir::new().unwrap();
        let home = root.path().join("home");
        let xdg_data = root.path().join("xdg-data");
        let xdg_config = root.path().join("xdg-config");
        let override_home = root.path().join("override");
        let mut cmd = command(&override_home);
        cmd.env("HOME", &home)
            .env("XDG_DATA_HOME", &xdg_data)
            .env("XDG_CONFIG_HOME", &xdg_config);
        if mode > 0 {
            cmd.env_remove("RASTRO_HOME");
        }
        if mode == 2 {
            cmd.env_remove("XDG_DATA_HOME")
                .env_remove("XDG_CONFIG_HOME");
        }
        let (data, config) = match mode {
            0 => (override_home.join("data"), override_home.join("config")),
            1 => (xdg_data.join("rastro"), xdg_config.join("rastro")),
            _ => (
                home.join(".local/share/rastro"),
                home.join(".config/rastro"),
            ),
        };
        run(&mut cmd, ADD);
        assert!(
            config.join("config.toml").is_file(),
            "missing config in {config:?}"
        );
        assert!(
            data.join("rastro.sqlite3").is_file(),
            "missing database in {data:?}"
        );
        // Read persisted configuration through a fresh process with the same environment.
        let mut list = command(&override_home);
        list.env("HOME", &home)
            .env("XDG_DATA_HOME", &xdg_data)
            .env("XDG_CONFIG_HOME", &xdg_config);
        if mode > 0 {
            list.env_remove("RASTRO_HOME");
        }
        if mode == 2 {
            list.env_remove("XDG_DATA_HOME")
                .env_remove("XDG_CONFIG_HOME");
        }
        assert!(run(&mut list, &["source", "list", "--json"]).contains("project"));
        if mode == 0 {
            assert!(!xdg_data.exists() && !xdg_config.exists() && !home.exists());
        } else {
            assert!(!override_home.exists());
        }
    }
}
