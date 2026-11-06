#![allow(non_snake_case)]
mod support;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::{path::Path, process::Command};
use tempfile::TempDir;

fn command(home: &Path) -> Command {
    let mut cmd = Command::new(support::rastro());
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

/// evidence/R1: source add then list shows the source
#[test]
fn source_add_then_list_shows_the_source() {
    let home = TempDir::new().unwrap();
    let before = Utc::now();
    let added = run(&mut command(home.path()), &[ADD, &["--json"]].concat());
    let added: Value = serde_json::from_str(&added).unwrap();
    assert_eq!(added["schema_version"], 2);
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
    assert_eq!(listed["schema_version"], 2);
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

/// evidence/R2: registered source has never looked coverage
#[test]
fn registered_source_has_never_looked_coverage() {
    let home = TempDir::new().unwrap();
    run(&mut command(home.path()), ADD);
    assert!(run(&mut command(home.path()), &["coverage", "project"]).contains("never looked"));
    let json: Value = serde_json::from_str(&run(
        &mut command(home.path()),
        &["coverage", "project", "--json"],
    ))
    .unwrap();
    assert_eq!(json["schema_version"], 2);
    assert_eq!(json["operation"], "coverage");
    assert_eq!(json["result"]["coverage"][0]["state"], "never looked");
    assert_eq!(json["result"]["coverage"][0]["source"], "project");
    let missing = command(home.path())
        .args(["coverage", "missing", "--json"])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    let error: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(error["schema_version"], 2);
    assert!(error["error"].as_str().unwrap().contains("unknown source"));
}

/// evidence/R3: locations follow rastro xdg and home
#[test]
fn locations_follow_rastro_xdg_and_home() {
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

/// evidence/R1: concurrent registrations persist and reject duplicates
#[test]
fn concurrent_registrations_persist_and_reject_duplicates() {
    let home = TempDir::new().unwrap();
    run(&mut command(home.path()), &["source", "list"]);
    let children: Vec<_> = (0..20)
        .map(|i| {
            let mut args = ADD.to_vec();
            let name = format!("project-{i}");
            args[2] = &name;
            command(home.path()).args(args).spawn().unwrap()
        })
        .collect();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let listed: Value = serde_json::from_str(&run(
        &mut command(home.path()),
        &["source", "list", "--json"],
    ))
    .unwrap();
    assert_eq!(listed["result"]["sources"].as_array().unwrap().len(), 20);
    let children: Vec<_> = (0..2)
        .map(|_| command(home.path()).args(ADD).spawn().unwrap())
        .collect();
    let successes = children
        .into_iter()
        .filter_map(|mut child| child.wait().unwrap().success().then_some(()))
        .count();
    assert_eq!(successes, 1, "exactly one duplicate registration succeeds");
}

/// evidence/R3: private files under permissive umask
#[cfg(unix)]
#[test]
fn private_files_under_permissive_umask() {
    use std::os::unix::fs::PermissionsExt;
    if std::env::var_os("RASTRO_TEST_PRIVATE_CHILD").is_none() {
        let output = Command::new("sh")
            .args(["-c", "umask 000; exec \"$@\"", "rastro-test"])
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "S3_private_files_under_permissive_umask",
                "--nocapture",
            ])
            .env("RASTRO_TEST_PRIVATE_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let home = TempDir::new().unwrap();
    let mut cmd = Command::new("sh");
    cmd.args([
        "-c",
        "umask 000; exec \"$@\"",
        "rastro-test",
        support::rastro().to_str().unwrap(),
    ])
    .env("RASTRO_HOME", home.path());
    run(&mut cmd, ADD);
    let mode = |path: &Path| {
        std::fs::metadata(path)
            .unwrap_or_else(|e| panic!("{path:?}: {e}"))
            .permissions()
            .mode()
            & 0o777
    };
    for directory in ["config", "data"] {
        assert_eq!(mode(&home.path().join(directory)), 0o700, "{directory}");
    }
    for file in [
        "data/rastro.sqlite3",
        "config/config.toml",
        "config/config.lock",
    ] {
        assert_eq!(mode(&home.path().join(file)), 0o600, "{file}");
    }
    let db = home.path().join("data/rastro.sqlite3");
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL; BEGIN IMMEDIATE; UPDATE sources SET connector='changed';",
        )
        .unwrap();
    for suffix in ["-wal", "-shm"] {
        assert_eq!(
            mode(&db.with_file_name(format!("rastro.sqlite3{suffix}"))),
            0o600,
            "{suffix}"
        );
    }
    connection.execute_batch("ROLLBACK; PRAGMA journal_mode=DELETE; BEGIN IMMEDIATE; UPDATE sources SET connector='changed';").unwrap();
    assert_eq!(mode(&db.with_file_name("rastro.sqlite3-journal")), 0o600);
    connection.execute_batch("ROLLBACK;").unwrap();
}

/// evidence/R1: freshness rejects signed duration and outputs match schema
#[test]
fn freshness_rejects_signed_duration_and_outputs_match_schema() {
    let home = TempDir::new().unwrap();
    let schema: Value =
        serde_json::from_str(include_str!("../../../schema/operations-v2.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let output = command(home.path())
        .args(ADD)
        .args(["--freshness-rule", "+1h", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "signed duration must be rejected");
    validator
        .validate(&serde_json::from_slice::<Value>(&output.stdout).unwrap())
        .unwrap();
    for args in [
        [ADD, &["--freshness-rule", "2h", "--json"]].concat(),
        vec!["source", "list", "--json"],
        vec!["coverage", "project", "--json"],
    ] {
        let value: Value = serde_json::from_str(&run(&mut command(home.path()), &args)).unwrap();
        validator.validate(&value).unwrap();
    }
}

/// evidence/R3: empty and relative location overrides
#[test]
fn empty_and_relative_location_overrides() {
    for (rastro, xdg, succeeds) in [
        (Some(""), Some(""), true),
        (None, Some("relative"), true),
        (Some("relative"), None, false),
    ] {
        let root = TempDir::new().unwrap();
        let home = root.path().join("home");
        let mut cmd = Command::new(support::rastro());
        cmd.current_dir(root.path())
            .env("HOME", &home)
            .env_remove("RASTRO_HOME")
            .env_remove("XDG_DATA_HOME")
            .env_remove("XDG_CONFIG_HOME");
        if let Some(value) = rastro {
            cmd.env("RASTRO_HOME", value);
        }
        if let Some(value) = xdg {
            cmd.env("XDG_DATA_HOME", value)
                .env("XDG_CONFIG_HOME", value);
        }
        let output = cmd.args(ADD).arg("--json").output().unwrap();
        assert_eq!(
            output.status.success(),
            succeeds,
            "rastro={rastro:?}, xdg={xdg:?}"
        );
        if succeeds {
            assert!(home.join(".local/share/rastro/rastro.sqlite3").is_file());
            assert!(home.join(".config/rastro/config.toml").is_file());
        } else {
            let json: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(
                json["error"]
                    .as_str()
                    .unwrap()
                    .contains("RASTRO_HOME must be absolute")
            );
        }
        assert!(!root.path().join("relative").exists());
    }
}

/// evidence/R75: declared options are required and unknown options are refused
#[test]
fn declared_options_are_required_and_unknown_options_are_refused() {
    let home = TempDir::new().unwrap();
    let plugin = home.path().join("config/plugins/test");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(plugin.join("rastro-plugin.toml"), format!("{}\n[[source_options]]\nname = \"workspace\"\nrequired = true\npattern = \".+\"\nhelp = \"Workspace identifier\"\n\n[[source_options]]\nname = \"region\"\nhelp = \"Region\"\n", include_str!("../../rastro-test-connector/rastro-plugin.toml"))).unwrap();
    for options in [vec![], vec!["--option", "other=value"]] {
        let output = command(home.path())
            .args([
                "source",
                "add",
                "test",
                "--connector",
                "test",
                "--scope",
                "personal",
                "--json",
            ])
            .args(options)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            error["error"].as_str().unwrap().contains("workspace"),
            "{error}"
        );
        let listed: Value = serde_json::from_str(&run(
            &mut command(home.path()),
            &["source", "list", "--json"],
        ))
        .unwrap();
        assert!(listed["result"]["sources"].as_array().unwrap().is_empty());
    }
}

/// evidence/R76: github repo shorthand uses the manifest pattern
#[test]
fn github_repo_shorthand_uses_the_manifest_pattern() {
    let home = TempDir::new().unwrap();
    let plugin = home.path().join("config/plugins/github");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(
        plugin.join("rastro-plugin.toml"),
        include_str!("../../rastro-connector-github/rastro-plugin.toml"),
    )
    .unwrap();
    run(&mut command(home.path()), ADD);
    let output = command(home.path())
        .args([
            "source",
            "add",
            "bad",
            "--connector",
            "github",
            "--repo",
            "invalid",
            "--scope",
            "personal",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(error["error"].as_str().unwrap().contains("repo"), "{error}");
    assert!(
        error["error"].as_str().unwrap().contains("pattern"),
        "{error}"
    );
}

/// evidence/R75: a declared option is saved listed and survives reload
#[test]
fn a_declared_option_is_saved_listed_and_survives_reload() {
    let home = TempDir::new().unwrap();
    let plugin = home.path().join("config/plugins/test");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(
        plugin.join("rastro-plugin.toml"),
        format!(
            "{}\n[[source_options]]\nname = \"workspace\"\nrequired = true\n",
            include_str!("../../rastro-test-connector/rastro-plugin.toml")
                .replace("[[source_options]]", "[[ignored]]")
                .split("[[ignored]]")
                .next()
                .unwrap()
        ),
    )
    .unwrap();
    run(
        &mut command(home.path()),
        &[
            "source",
            "add",
            "w",
            "--connector",
            "test",
            "--scope",
            "personal",
            "--option",
            "workspace=x",
        ],
    );
    for _ in 0..2 {
        let listed: Value = serde_json::from_str(&run(
            &mut command(home.path()),
            &["source", "list", "--json"],
        ))
        .unwrap();
        assert_eq!(listed["result"]["sources"][0]["selector"]["workspace"], "x");
    }
}

/// evidence/R75: a manifest declaring no options refuses any option and an empty name
#[test]
fn a_manifest_declaring_no_options_refuses_any_option_and_an_empty_name() {
    let home = TempDir::new().unwrap();
    let plugin = home.path().join("config/plugins/test");
    std::fs::create_dir_all(&plugin).unwrap();
    let manifest = include_str!("../../rastro-test-connector/rastro-plugin.toml")
        .split("[[source_options]]")
        .next()
        .unwrap();
    std::fs::write(plugin.join("rastro-plugin.toml"), manifest).unwrap();
    for option in ["anything=1", "=x"] {
        let output = command(home.path())
            .args([
                "source",
                "add",
                "w",
                "--connector",
                "test",
                "--scope",
                "personal",
                "--json",
                "--option",
                option,
            ])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{option}");
    }
    let listed: Value = serde_json::from_str(&run(
        &mut command(home.path()),
        &["source", "list", "--json"],
    ))
    .unwrap();
    assert!(listed["result"]["sources"].as_array().unwrap().is_empty());
}

/// evidence/R76: the bundled github declaration checks the repo when no plugin is installed
#[test]
fn the_bundled_github_declaration_checks_the_repo_when_no_plugin_is_installed() {
    let home = TempDir::new().unwrap();
    for repo in ["invalid", "../x", "o/..", "a b/c"] {
        let output = command(home.path())
            .args([
                "source",
                "add",
                "bad",
                "--connector",
                "github",
                "--repo",
                repo,
                "--scope",
                "personal",
                "--json",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{repo}");
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            error["error"].as_str().unwrap().contains("pattern"),
            "{error}"
        );
    }
    run(&mut command(home.path()), ADD);
}

/// evidence/R77: a connector whose manifest declares `single_source` refuses a second source with an
/// error naming the existing one, in any scope, and nothing is saved; a connector that doesn't
/// declare it takes several
#[test]
fn a_single_source_connector_refuses_a_second_source_naming_the_first() {
    let manifest = include_str!("../../rastro-test-connector/rastro-plugin.toml")
        .split("[[source_options]]")
        .next()
        .unwrap();
    for single in [false, true] {
        let home = TempDir::new().unwrap();
        let plugin = home.path().join("config/plugins/test");
        std::fs::create_dir_all(&plugin).unwrap();
        let declared = if single { "single_source = true\n" } else { "" };
        std::fs::write(
            plugin.join("rastro-plugin.toml"),
            format!("{declared}{manifest}"),
        )
        .unwrap();
        let add = |name: &str, scope: &str| {
            command(home.path())
                .args(["source", "add", name, "--connector", "test"])
                .args(["--scope", scope, "--json"])
                .output()
                .unwrap()
        };
        let config = || std::fs::read_to_string(home.path().join("config/config.toml")).unwrap();
        assert!(add("first", "personal").status.success());
        if !single {
            assert!(add("second", "personal").status.success());
            continue;
        }
        let before = config();
        for scope in ["personal", "work"] {
            let output = add("second", scope);
            assert!(!output.status.success(), "{scope}");
            let error: Value = serde_json::from_slice(&output.stdout).unwrap();
            let message = error["error"].as_str().unwrap();
            assert!(
                message.contains("connector test") && message.contains("source first"),
                "{error}"
            );
            assert_eq!(config(), before);
        }
    }
}
