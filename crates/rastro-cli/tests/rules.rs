//! Owner rules in `config.toml`: read on load, checked, and listed by `rastro rules check`. The
//! provider is the scripted fake action provider that declares `linear.create_issue`, so no
//! outside system is called.
mod support;
use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;

/// A home with the fake action provider installed and `config` as its `config.toml`.
fn home(config: &str) -> TempDir {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("config");
    support::provider::install(&dir, |_| {});
    fs::write(dir.join("config.toml"), config).unwrap();
    home
}

fn rastro(home: &TempDir, args: &[&str]) -> Output {
    Command::new(support::rastro())
        .env("RASTRO_HOME", home.path())
        .args(args)
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn rule(name: &str, extra: &str) -> String {
    format!(
        "[[rules]]\nname = \"{name}\"\naction = \"linear.create_issue\"\nteam = \"H\"\n\
         assignee = \"me\"\n{extra}\n"
    )
}

// spec: Owner rules live in config
#[test]
fn a_valid_rule_is_listed_with_its_action_team_and_assignee() {
    let home = home(&rule("my-issues", ""));
    let out = rastro(&home, &["rules", "check"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    let line = text
        .lines()
        .find(|line| line.contains("my-issues"))
        .unwrap_or_else(|| panic!("no line for my-issues in:\n{text}"));
    for part in ["linear.create_issue", "team H", "assignee me"] {
        assert!(line.contains(part), "{part} missing from: {line}");
    }
}

// spec: Owner rules live in config
#[test]
fn a_rule_naming_an_action_no_provider_declares_fails_the_queue_command() {
    let home = home(
        "[[rules]]\nname = \"bad\"\naction = \"linear.make_coffee\"\nteam = \"H\"\nassignee = \"me\"\n",
    );
    let out = rastro(&home, &["queue"]);
    assert!(!out.status.success());
    let error = stderr(&out);
    assert!(
        error.contains("bad") && error.contains("action"),
        "the error names neither the rule nor the field: {error}"
    );
    assert_eq!(stdout(&out), "", "a failed command prints no answer");
}

// spec: Owner rules live in config
#[test]
fn a_rule_whose_assignee_is_not_the_owner_fails_the_check() {
    let home = home(
        "[[rules]]\nname = \"bad\"\naction = \"linear.create_issue\"\nteam = \"H\"\n\
         assignee = \"someone\"\n",
    );
    let out = rastro(&home, &["rules", "check"]);
    assert!(!out.status.success());
    let error = stderr(&out);
    assert!(
        error.contains("bad") && error.contains("assignee"),
        "the error names neither the rule nor the field: {error}"
    );
}

// spec: A rule's grace comes from the table
#[test]
fn a_rules_grace_is_its_own_else_the_tables_else_the_default() {
    let cases = [
        ("no table, no own grace", rule("r", ""), "grace 10 min"),
        (
            "table sets medium",
            format!("[grace]\nmedium = \"3m\"\n\n{}", rule("r", "")),
            "grace 3 min",
        ),
        (
            "own grace beats the table",
            format!(
                "[grace]\nmedium = \"3m\"\n\n{}",
                rule("r", "grace = \"1m\"")
            ),
            "grace 1 min",
        ),
    ];
    for (case, config, expected) in cases {
        let out = rastro(&home(&config), &["rules", "check"]);
        assert!(out.status.success(), "{case}: {}", stderr(&out));
        assert!(
            stdout(&out).contains(expected),
            "{case}: expected {expected} in:\n{}",
            stdout(&out)
        );
    }
}

// spec: Owner rules live in config
#[test]
fn a_grace_too_long_to_count_fails_the_check_naming_the_rule_and_field() {
    let home = home(&rule("big", "grace = \"5124095576030432h\""));
    let out = rastro(&home, &["rules", "check"]);
    assert!(!out.status.success());
    let error = stderr(&out);
    assert!(
        error.contains("rule big") && error.contains("grace") && !error.contains("panicked"),
        "the error names neither the rule nor the field cleanly: {error}"
    );
}
