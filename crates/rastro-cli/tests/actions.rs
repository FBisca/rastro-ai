//! Action providers as a plugin role: the host loads one that declares its actions, each with its
//! reach and reversibility, and offers nothing of one that also submits evidence or leaves its
//! risk undeclared. The provider is a shell script, so no outside system is ever called.
mod support;
use rastro_core::{
    Locations, host,
    host::actions::{Reach, Reversibility},
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
