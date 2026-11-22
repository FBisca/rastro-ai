#![allow(non_snake_case)]
//! The Slack extractor end to end: the real connector reads the scripted Slack API, the real
//! extractor turns its evidence into `slack` claims, and `rastro` reads them back.
mod support;
use serde_json::json;
use std::collections::BTreeSet;
use support::*;

/// A second human author, beside the fixture's Ana.
const BOB: &str = "U0BOB";

/// Two human authors with display names, the owner, and a deploy bot.
fn world() -> World {
    let mut ana = user(ANA, "Ana Silva", false);
    ana["profile"] = json!({"display_name": "Ana"});
    let mut bob = user(BOB, "Bob Lima", false);
    bob["profile"] = json!({"display_name": "Bob"});
    World {
        users: vec![
            user(VIEWER, "Owner Person", false),
            ana,
            bob,
            user(BOT, "Deploy Bot", true),
        ],
        messages: vec![
            message(ANA, "hi", in_window(1)),
            message(BOB, "hello", in_window(2)),
        ],
        ..World::default()
    }
}
fn account(id: &str) -> String {
    format!("slack:user/{id}")
}

/// Claims (predicate, object, status) about `entity`, sorted, from `rastro about`.
fn about(env: &Env, entity: &str) -> BTreeSet<(String, String, String)> {
    let found = env.json(&["about", entity, "--limit", "50"]);
    found["claims"]
        .as_array()
        .unwrap_or_else(|| panic!("{entity}: {found}"))
        .iter()
        .filter(|c| c["subject"]["identifier"] == entity)
        .map(|c| {
            let object = match c["object"]["kind"].as_str() {
                Some("entity") => c["object"]["identifier"].as_str(),
                _ => c["object"]["value"].as_str(),
            };
            (
                c["predicate"].as_str().unwrap().to_string(),
                object.unwrap().to_string(),
                c["status"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}
fn claim(predicate: &str, object: &str, status: &str) -> (String, String, String) {
    (predicate.into(), object.into(), status.into())
}

/// spec: A Slack sync records accounts with their names
/// WHEN a Slack source syncs messages from two authors against the scripted API, THEN each
/// author is a `slack:user/<id>` entity typed `slack:Account` with its name and display-name
/// aliases, and the source is bound to the Slack extractor
#[test]
fn authors_become_slack_accounts_with_their_names() {
    let fixture = Fixture::start(world());
    let env = env(&fixture.base);
    let run = env.sync(&[]);
    assert_eq!(run["status"], "ok", "{run}");
    let extraction = &run["extraction"];
    assert_eq!(extraction["extractor"], "rastro-extractor-slack", "{run}");
    assert_eq!(extraction["state"], "current", "{extraction}");
    assert_eq!(extraction["counts"]["rejected"], 0, "{extraction}");

    for (id, full, username, display) in [
        (ANA, "Ana Silva", "ana.silva", "Ana"),
        (BOB, "Bob Lima", "bob.lima", "Bob"),
    ] {
        let user = account(id);
        let claims = about(&env, &user);
        for expected in [
            claim("rastro:type", "slack:Account", "observed"),
            claim("rastro:alias", full, "observed"),
            claim("rastro:alias", username, "observed"),
            claim("rastro:alias", display, "observed"),
        ] {
            assert!(
                claims.contains(&expected),
                "{user} {expected:?}: {claims:?}"
            );
        }
        // Each alias kind looks the account up.
        for alias in [
            format!("slack:name/{full}"),
            format!("slack:display_name/{display}"),
        ] {
            let found = env.json(&["about", &alias]);
            assert!(
                found["claims"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c["subject"]["identifier"] == user.as_str()),
                "{alias}: {found}"
            );
        }
    }
    let datatypes = env.column(&format!(
        "SELECT c.object_datatype FROM claim c JOIN entity e ON e.id = c.subject
         WHERE e.identifier = '{}' AND c.predicate = 'rastro:alias' ORDER BY c.object_datatype",
        account(ANA)
    ));
    assert_eq!(datatypes, ["display_name", "name", "name"]);
}
/// spec: The Slack token owner holds me
/// WHEN `me` exists and a Slack source syncs against the scripted API, THEN the token owner's
/// `slack:user/<id>` account is `held_by` `me` with a token-owner justification
#[test]
fn the_token_owner_account_is_held_by_me() {
    let fixture = Fixture::start(world());
    let env = env(&fixture.base);
    env.ok(&["identity", "me", "--json"]);
    let run = env.sync(&[]);
    assert_eq!(run["status"], "ok", "{run}");
    assert_eq!(run["extraction"]["state"], "current", "{run}");
    assert_eq!(run["extraction"]["counts"]["rejected"], 0, "{run}");

    let me = env.json(&["about", "me"]);
    let person = me["person"]["identifier"].as_str().unwrap().to_string();
    let owner = account(VIEWER);
    let linked = me["person"]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["identifier"] == owner)
        .expect("viewer linked to me");
    assert!(
        linked["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == "token_owner"),
        "{me}"
    );
    let claims = about(&env, &owner);
    assert!(
        claims.contains(&claim("rastro:held_by", &person, "observed")),
        "{claims:?}"
    );
    assert!(
        claims.contains(&claim("rastro:token_owner", "true", "observed")),
        "{claims:?}"
    );
}

/// spec: A Slack bot never links
/// WHEN a Slack sync records an author whose profile says it is a bot, THEN the account is
/// typed `slack:Bot`, no Person holds it, and a link to it is refused, while a human author
/// still links
#[test]
fn a_bot_author_is_typed_slack_bot_and_no_person_holds_it() {
    let mut world = world();
    world.messages.push(bot_message("deployed", in_window(3)));
    let fixture = Fixture::start(world);
    let env = env(&fixture.base);
    env.ok(&["identity", "me", "--json"]);
    let run = env.sync(&[]);
    assert_eq!(run["status"], "ok", "{run}");
    assert_eq!(run["extraction"]["state"], "current", "{run}");
    assert_eq!(run["extraction"]["counts"]["rejected"], 0, "{run}");

    let bot = account(BOT);
    let claims = about(&env, &bot);
    assert!(
        claims.contains(&claim("rastro:type", "slack:Bot", "observed")),
        "{claims:?}"
    );
    assert!(
        !claims.iter().any(|c| c.0 == "rastro:held_by"),
        "{claims:?}"
    );
    // The bot keeps its name.
    assert!(
        claims.contains(&claim("rastro:alias", "Deploy Bot", "observed")),
        "{claims:?}"
    );

    let out = env.run(&["identity", "link", &bot, "me", "--json"]);
    assert!(!out.status.success());
    let refused: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let error = refused["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("is a slack:Bot") && error.contains("only an account is held by a Person"),
        "{refused}"
    );

    // A human author still links.
    env.json(&["identity", "link", &account(ANA), "me"]);
    assert!(
        about(&env, &account(ANA))
            .iter()
            .any(|c| c.0 == "rastro:held_by")
    );
}

/// A Linear user named like the Slack deploy bot.
const NAMESAKE: &str = "0b5a8c1e-0000-4000-8000-00000000000a";

/// spec: A Slack bot never links
/// WHEN a Slack bot's folded name equals the name of another source's account, THEN no link
/// candidate is offered for the pair, while two human accounts sharing a name are
#[test]
fn a_bot_sharing_a_name_is_never_a_link_candidate() {
    let mut slack_world = world();
    slack_world
        .messages
        .push(bot_message("deployed", in_window(3)));
    let slack = Fixture::start(slack_world);
    let linear = linear_api::Fixture::start(linear_api::World {
        users: vec![
            linear_api::user(
                linear_api::VIEWER,
                "Owner Person",
                "owner@example.com",
                true,
            ),
            linear_api::user(linear_api::ANA, "Ana Silva", "ana@example.com", true),
            linear_api::user(NAMESAKE, "Deploy Bot", "deploy@example.com", true),
        ],
        ..linear_api::World::default()
    });
    let mut env = env_with_linear(&slack.base);
    env.linear(&linear.base);
    env.ok(&["identity", "me", "--json"]);
    let run = env.sync(&[]);
    assert_eq!(run["status"], "ok", "{run}");
    assert_eq!(run["extraction"]["counts"]["rejected"], 0, "{run}");
    let run = env.sync_linear(&[]);
    assert_eq!(run["status"], "ok", "{run}");
    assert_eq!(run["extraction"]["counts"]["rejected"], 0, "{run}");

    let listed = env.json(&["identity", "candidates"]);
    let pairs: Vec<(String, String)> = listed["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["accounts"][0]["identifier"].as_str().unwrap().to_string(),
                c["accounts"][1]["identifier"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let bot = account(BOT);
    assert!(
        !pairs.iter().any(|(a, b)| *a == bot || *b == bot),
        "{pairs:?}"
    );
    // The control: the two human accounts sharing Ana's name are a candidate, so the bot's
    // absence is its typing, not a silent rule.
    let ana_pair = (format!("linear:user/{}", linear_api::ANA), account(ANA));
    assert!(pairs.contains(&ana_pair), "{pairs:?}");
}
