//! Action providers as a plugin role: the host loads one that declares its actions, each with its
//! reach and reversibility, and offers nothing of one that also submits evidence or leaves its
//! risk undeclared. The provider is a shell script, so no outside system is ever called. An agent
//! proposes an action through the CLI: the proposal waits in the queue, bound to the source it
//! writes to, and nothing reaches the provider.
mod support;
use rastro_core::{
    Locations, host,
    host::actions::{Reach, Reversibility},
};
use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;

struct Env {
    home: TempDir,
}

impl Env {
    /// A home whose installed provider's manifest is the fake one with `edit` applied.
    fn with_provider(edit: impl FnOnce(&mut toml::Table)) -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
        };
        support::provider::install(&env.locations().config, edit);
        env
    }
    fn locations(&self) -> Locations {
        Locations {
            data: self.home.path().join("data"),
            config: self.home.path().join("config"),
        }
    }

    /// A home with the fake provider installed, the Linear source `linear-work` in scope `work`
    /// and, when given, `[actions.linear] default_team`.
    fn configured(default_team: Option<&str>) -> Self {
        let env = Self::with_provider(|_| {});
        let team = default_team
            .map(|team| format!("[actions.linear]\ndefault_team = \"{team}\"\n"))
            .unwrap_or_default();
        fs::write(
            env.locations().config.join("config.toml"),
            format!(
                "[[sources]]\nname = \"linear-work\"\nconnector = \"linear\"\nselector = {{}}\n\
                 scope = \"work\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
                 keep_original = false\n\n{team}"
            ),
        )
        .unwrap();
        env
    }

    fn rastro(&self, args: &[&str]) -> Output {
        Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .args(args)
            .output()
            .unwrap()
    }

    /// The `result` of a command that must succeed, read with `--json`.
    fn ok_json(&self, args: &[&str]) -> Value {
        let out = self.rastro(&[args, &["--json"]].concat());
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"].clone()
    }

    /// The queue's items, as a machine reads them.
    fn queue_items(&self) -> Vec<Value> {
        self.ok_json(&["queue"])["items"]
            .as_array()
            .unwrap()
            .clone()
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The arguments of a proposal of `linear.create_issue` titled `title`, with the why and `extra`.
fn propose<'a>(title: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "actions",
        "propose",
        "linear.create_issue",
        "--title",
        title,
    ];
    args.extend_from_slice(extra);
    args
}

/// The refusal for `action`, which must exist, as the owner reads it.
fn refusal(env: &Env, action: &str) -> String {
    let locations = env.locations();
    let error = host::actions::offer(&locations, action).expect_err("the action must be refused");
    assert!(
        host::actions::offered(&locations).offered.is_empty(),
        "a refused provider's action is offered"
    );
    format!("{error:#}")
}

// spec: Action providers are a plugin role
#[test]
fn a_provider_declaring_its_actions_and_no_evidence_permission_is_loaded() {
    let env = Env::with_provider(|_| {});
    let offer = host::actions::offer(&env.locations(), "linear.create_issue").unwrap();
    assert_eq!(offer.provider, support::provider::ID);
    assert_eq!(offer.action, "linear.create_issue");
    assert_eq!(offer.reach, Reach::OthersSeeIt);
    assert_eq!(offer.reversibility, Reversibility::CanBeUndone);
}

// spec: Action providers are a plugin role
#[test]
fn a_provider_that_also_submits_evidence_is_refused_naming_the_permission() {
    let env = Env::with_provider(|manifest| {
        manifest["permissions"]["submit"] = toml::Value::Array(vec!["issue".into()]);
    });
    let message = refusal(&env, "linear.create_issue");
    assert!(message.contains(support::provider::ID), "{message}");
    assert!(message.contains("`submit`"), "{message}");
}

// spec: Action providers are a plugin role
#[test]
fn a_shadowed_copy_of_a_refused_provider_offers_nothing() {
    // The copy at the plugins root is found first, so it is the one the host would run; it is
    // refused, and the valid copy it shadows must not bring the action back.
    let env = Env::with_provider(|_| {});
    let mut winning = support::provider::manifest();
    winning["permissions"]["submit"] = toml::Value::Array(vec!["issue".into()]);
    let root = env.locations().config.join("plugins");
    std::fs::write(root.join("rastro-plugin.toml"), winning.to_string()).unwrap();
    let message = refusal(&env, "linear.create_issue");
    assert!(message.contains("`submit`"), "{message}");
}

// spec: Action providers are a plugin role
#[test]
fn a_provider_of_another_kind_too_is_refused() {
    let env = Env::with_provider(|manifest| {
        manifest["kinds"] = toml::Value::Array(vec!["action_provider".into(), "extractor".into()]);
    });
    let message = refusal(&env, "linear.create_issue");
    assert!(message.contains(support::provider::ID), "{message}");
    assert!(message.contains("extractor"), "{message}");
}

// spec: Action providers are a plugin role
#[test]
fn a_connector_declaring_actions_is_refused() {
    let env = Env::with_provider(|manifest| {
        manifest["kinds"] = toml::Value::Array(vec!["connector".into()]);
        manifest["permissions"]["actions"] = toml::Value::Array(vec![]);
    });
    let locations = env.locations();
    assert!(host::discover(&locations, support::provider::ID, "connector").is_err());
    assert!(host::actions::offered(&locations).offered.is_empty());
}

// spec: Each action declares its reach and reversibility
#[test]
fn an_action_without_reversibility_is_refused_naming_it() {
    let env = Env::with_provider(|manifest| {
        manifest["action"].as_array_mut().unwrap()[0]
            .as_table_mut()
            .unwrap()
            .remove("reversibility");
    });
    let message = refusal(&env, "linear.create_issue");
    assert!(message.contains("linear.create_issue"), "{message}");
    assert!(message.contains("`reversibility`"), "{message}");
}

// spec: Each action declares its reach and reversibility
#[test]
fn an_action_without_reach_is_refused_naming_it() {
    let env = Env::with_provider(|manifest| {
        manifest["action"].as_array_mut().unwrap()[0]
            .as_table_mut()
            .unwrap()
            .remove("reach");
    });
    let message = refusal(&env, "linear.create_issue");
    assert!(message.contains("linear.create_issue"), "{message}");
    assert!(message.contains("`reach`"), "{message}");
}

// spec: An agent proposes an action
#[test]
fn a_proposal_waits_in_the_queue_and_nothing_reaches_the_provider() {
    let env = Env::configured(Some("ENG"));
    let proposed = env.ok_json(&propose(
        "Flaky sync test",
        &["--why", "owner asked in session"],
    ));
    let id = proposed["id"].as_str().unwrap().to_string();

    let items = env.queue_items();
    assert_eq!(items.len(), 1, "{items:?}");
    let item = &items[0];
    assert_eq!(item["id"], id.as_str());
    assert!(id.starts_with("q_"), "{id}");
    assert_eq!(item["kind"], "action_proposal");
    assert_eq!(
        item["source"],
        json!({"kind": "linear", "name": "linear-work", "scope": "work"})
    );
    assert_eq!(
        item["action"],
        json!({
            "action": "linear.create_issue",
            "fields": {"title": "Flaky sync test", "team": "ENG", "assignee": "me"},
            "why": "owner asked in session",
            "cites": []
        })
    );

    // The terminal line names the action, the title, the team, the assignee, the why and the
    // source with its scope.
    let printed = env.rastro(&["queue"]);
    let text = String::from_utf8_lossy(&printed.stdout).into_owned();
    let line = text
        .lines()
        .find(|line| line.starts_with("Action proposal"))
        .unwrap_or_else(|| panic!("no action proposal line in: {text}"));
    assert!(line.contains(&format!("Id: {id}")), "{line}");
    assert!(
        line.contains("Source: linear linear-work | Scope: work"),
        "{line}"
    );
    let detail = text.lines().skip_while(|l| *l != line).nth(1).unwrap();
    for part in [
        "linear.create_issue",
        "Flaky sync test",
        "Team: ENG",
        "Assignee: me",
        "Why: owner asked in session",
    ] {
        assert!(detail.contains(part), "{part} missing from: {detail}");
    }

    // The id addresses the content: the same proposal is the same item, another title is another.
    let again = env.ok_json(&propose(
        "Flaky sync test",
        &["--why", "owner asked in session"],
    ));
    assert_eq!(again["id"], id.as_str());
    assert_eq!(env.queue_items().len(), 1);
    let other = env.ok_json(&propose(
        "Another title",
        &["--why", "owner asked in session"],
    ));
    assert_ne!(other["id"], id.as_str());
    assert_eq!(env.queue_items().len(), 2);

    assert_eq!(support::provider::calls(&env.locations().config), "");
}

// spec: An agent proposes an action
#[test]
fn a_proposal_without_a_why_is_refused_and_nothing_is_recorded() {
    let env = Env::configured(Some("ENG"));
    let out = env.rastro(&propose("Flaky sync test", &[]));
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--why"), "{}", stderr(&out));
    assert!(env.queue_items().is_empty());
    // A blank why says nothing either.
    let blank = env.rastro(&propose("Flaky sync test", &["--why", "  "]));
    assert!(!blank.status.success());
    assert!(stderr(&blank).contains("--why"), "{}", stderr(&blank));
    assert!(env.queue_items().is_empty());
}

// spec: Action providers are a plugin role
#[test]
fn an_action_no_provider_declares_is_refused_naming_it() {
    let env = Env::configured(Some("ENG"));
    let out = env.rastro(&[
        "actions",
        "propose",
        "linear.create_cycle",
        "--title",
        "T",
        "--why",
        "W",
    ]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("linear.create_cycle"),
        "{}",
        stderr(&out)
    );
    assert!(env.queue_items().is_empty());
}

// spec: A proposal names its team
#[test]
fn a_proposal_with_no_team_anywhere_is_refused_naming_both_places() {
    let env = Env::configured(None);
    let out = env.rastro(&propose("Flaky sync test", &["--why", "W"]));
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let message = stderr(&out);
    assert!(message.contains("--team"), "{message}");
    assert!(
        message.contains("[actions.linear] default_team"),
        "{message}"
    );
    assert!(env.queue_items().is_empty());

    // Naming the team on the command line is enough.
    let proposed = env.ok_json(&propose(
        "Flaky sync test",
        &["--why", "W", "--team", "OPS"],
    ));
    assert_eq!(env.queue_items()[0]["id"], proposed["id"]);
    assert_eq!(env.queue_items()[0]["action"]["fields"]["team"], "OPS");
}

// spec: An agent proposes an action
#[test]
fn two_linear_sources_make_the_proposal_ambiguous_and_nothing_is_recorded() {
    let env = Env::configured(Some("ENG"));
    let config = env.locations().config.join("config.toml");
    let mut text = fs::read_to_string(&config).unwrap();
    text.push_str(
        "\n[[sources]]\nname = \"linear-personal\"\nconnector = \"linear\"\nselector = {}\n\
         scope = \"personal\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
         keep_original = false\n",
    );
    fs::write(config, text).unwrap();
    let out = env.rastro(&propose("Flaky sync test", &["--why", "W"]));
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let message = stderr(&out);
    assert!(message.contains("linear-work"), "{message}");
    assert!(message.contains("linear-personal"), "{message}");
    assert!(env.queue_items().is_empty());
}
