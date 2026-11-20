#![allow(non_snake_case)]
//! A retraction carries through to what rests on it: rejecting the premise of an automatic link
//! ends the link unless an owner link also holds it, rejecting the anchor of an accepted
//! commitment takes the slots only that anchor justified, and a retracted statement is no longer
//! known to the queue. The agent tool adapter is a shell script that answers the plugin protocol
//! with the candidates in `candidates.json`, so no model is ever called; accounts and evidence
//! are seeded straight into the projection.
mod support;
use chrono::{DateTime, Utc};
use rastro_core::claims::{self, Entity, Object, ObservedClaim, Support};
use rastro_core::{Evidence, Scope, Source, Storage, TrustClass};
use rastro_storage_sqlite::SqliteStorage;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet, fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command,
};
use tempfile::TempDir;

/// The extractor a seeded source is bound to; it is never installed, so nothing re-reads it.
const MISSING: &str = "missing-extractor";

const TRANSCRIPT: &str = "Alice: I'll send Bob the budget by Friday.\n\
Bob: Thanks. I'll review it next week for you if the numbers are final.\n";

const MANIFEST: &str = r#"id = "test-adapter"
name = "Rastro test agent tool adapter"
version = "0.1.0"
author = "Rastro"
kinds = ["agent-tool-adapter"]
protocol = 1
command = "adapter"
agent_tool = { capabilities = ["judge_text"] }
enforcement = ["tools_off", "no_mcp_servers", "no_setting_sources"]

[permissions]
submit = []
scopes = []
env = []
actions = []
"#;

/// A fake agent tool adapter: it answers the plugin protocol and returns the candidates in
/// `$RASTRO_HOME/candidates.json`.
const ADAPTER: &str = r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocol":"1.0","manifest_id":"test-adapter"}}\n' "$id"
      ;;
    *'"method":"judgment.request"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"candidates":' "$id"
      cat "$RASTRO_HOME/candidates.json"
      printf '}}\n'
      ;;
    *'"method":"run.finish"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"status":"ok","submitted":1}}\n' "$id"
      ;;
  esac
done
"#;

fn time() -> DateTime<Utc> {
    "2026-11-01T00:00:00Z".parse().unwrap()
}

struct Env {
    home: TempDir,
}

impl Env {
    /// A store with seeded evidence only.
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().unwrap(),
        }
    }

    /// A store whose meeting imports are read by the fake adapter: the owner's Person exists and
    /// the adapter proposes the promise and the waiting-on of `TRANSCRIPT`.
    fn with_adapter() -> Self {
        let env = Self::new();
        fs::create_dir_all(env.path("config/plugins/test-adapter")).unwrap();
        fs::write(
            env.path("config/config.toml"),
            "[commitment_adapter]\nplugin = \"test-adapter\"\n",
        )
        .unwrap();
        fs::write(
            env.path("config/plugins/test-adapter/rastro-plugin.toml"),
            MANIFEST,
        )
        .unwrap();
        let adapter = env.path("config/plugins/test-adapter/adapter");
        fs::write(&adapter, ADAPTER).unwrap();
        fs::set_permissions(&adapter, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(env.path("transcript.txt"), TRANSCRIPT).unwrap();
        let candidates = json!([
            {"turn": 1, "kind": "promise", "responsible": "Alice", "recipient": "Bob",
             "promise": "send Bob the budget", "stated_date": "by Friday",
             "quote": "I'll send Bob the budget by Friday"},
            {"turn": 2, "kind": "waiting_on", "responsible": "Bob", "recipient": "Alice",
             "promise": "review the numbers", "stated_date": "next week",
             "quote": "I'll review it next week for you if the numbers are final."},
        ]);
        fs::write(env.path("candidates.json"), candidates.to_string()).unwrap();
        env.ok(&["identity", "me"]);
        env
    }

    fn path(&self, name: &str) -> PathBuf {
        self.home.path().join(name)
    }

    fn run(&self, args: &[&str]) -> (bool, Value) {
        let out = Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .args(args)
            .arg("--json")
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
            panic!(
                "{args:?}: {}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        });
        let schema: Value =
            serde_json::from_str(include_str!("../../../schema/operations-v2.schema.json"))
                .unwrap();
        jsonschema::validator_for(&schema)
            .unwrap()
            .validate(&value)
            .unwrap_or_else(|error| panic!("{error}: {value}"));
        (out.status.success(), value)
    }

    fn ok(&self, args: &[&str]) -> Value {
        let (ok, value) = self.run(args);
        assert!(ok, "{args:?}: {value}");
        value["result"].clone()
    }

    fn import(&self, meeting: &str, captured_at: &str) {
        let file = self.path("transcript.txt");
        self.ok(&[
            "meeting",
            "import",
            file.to_str().unwrap(),
            "--meeting",
            meeting,
            "--scope",
            "work",
            "--captured-at",
            captured_at,
            "--owner-speaker",
            "Alice",
        ]);
    }

    /// Records one claim read from `location` and `field` by source `project` (scope
    /// `personal`), bound to an extractor that isn't installed.
    fn record(&self, subject: &Entity, predicate: &str, object: Object, at: (&str, &str)) {
        let (location, field) = at;
        let source = Source::github("project".into(), "o/p".into(), Scope("personal".into()));
        let mut db = SqliteStorage::open(&self.path("data/rastro.sqlite3")).unwrap();
        db.project_sources(std::slice::from_ref(&source)).unwrap();
        db.bind_extractor(&source.name, Some(MISSING)).unwrap();
        let run = db
            .start_run(&source, (MISSING, "1"), time(), (time(), time()))
            .unwrap();
        let quote = match &object {
            Object::Literal { value, .. } => value.clone(),
            Object::Entity { identifier, .. } => identifier.clone(),
        };
        let e = Evidence {
            source: source.name.clone(),
            scope: source.scope.clone(),
            location: location.into(),
            field: field.into(),
            start: 0,
            end: quote.len() as u64,
            quote,
            trust_class: TrustClass::Structured,
            captured_at: time(),
            source_updated_at: Some(time()),
            original: None,
        };
        db.append_evidence(run, &e).unwrap();
        let (evidence_id, _) = db
            .pending_evidence((MISSING, "1"), &source)
            .unwrap()
            .into_iter()
            .rev()
            .find(|(_, item)| item.location == location && item.field == field)
            .unwrap();
        let account = matches!(&object, Object::Literal { value, .. }
            if predicate == claims::TYPE_PREDICATE && value.ends_with(":Account"));
        let observed = ObservedClaim {
            id: claims::claim_id(&subject.id, predicate, &object),
            subject: subject.clone(),
            predicate: predicate.into(),
            object,
            // As the core validates them: identity premises are never scalar.
            scalar: ![
                rastro_core::identity::VERIFIED_EMAIL,
                rastro_core::identity::TOKEN_OWNER,
            ]
            .contains(&predicate),
            type_ancestors: if account {
                vec![rastro_core::identity::ACCOUNT_TYPE.into()]
            } else {
                Vec::new()
            },
            supports: BTreeSet::from([Support {
                portable_key: claims::portable_evidence_key(&e),
                evidence_id,
                field: field.into(),
                start: 0,
                end: e.end,
                extraction_rule: "fixture-rule".into(),
            }]),
        };
        db.record_claims(run, (MISSING, "1"), &[observed], None)
            .unwrap();
    }

    /// A GitHub-shaped account `github:account/<n>`, typed `code:Account`, with a verified email
    /// read at its own location.
    fn account(&self, n: u32, email: &str) -> String {
        let identifier = format!("github:account/{n}");
        let subject = Entity::new(identifier.clone(), "code:Account".into());
        let location = format!("https://api.github.com/users/{n}");
        let typed = Object::Literal {
            value: "code:Account".into(),
            datatype: claims::TYPE_DATATYPE.into(),
        };
        self.record(
            &subject,
            claims::TYPE_PREDICATE,
            typed,
            (&location, "/type"),
        );
        let email = Object::Literal {
            value: email.into(),
            datatype: "email".into(),
        };
        self.record(
            &subject,
            rastro_core::identity::VERIFIED_EMAIL,
            email,
            (&location, "/verified-email"),
        );
        identifier
    }

    /// A new named Person; returns its canonical identifier.
    fn person(&self, name: &str) -> String {
        let created = self.ok(&["identity", "person", "create", "--name", name]);
        created["person"]["identifier"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn about(&self, entity: &str) -> Value {
        self.ok(&["about", entity])
    }

    /// The claim of `predicate` that `entity`'s `about` shows, with the object whose identifier
    /// or value is `object` (any object when empty).
    fn claim(&self, entity: &str, predicate: &str, object: &str) -> Value {
        let about = self.about(entity);
        about["claims"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| {
                c["predicate"] == predicate
                    && (object.is_empty()
                        || c["object"]["identifier"] == object
                        || c["object"]["value"] == object)
            })
            .unwrap_or_else(|| panic!("no {predicate} {object}: {about}"))
            .clone()
    }

    fn reject(&self, claim: &Value) -> Value {
        self.ok(&[
            "claim",
            "reject",
            claim["id"].as_str().unwrap(),
            "--seen",
            claim["revision"].as_str().unwrap(),
        ])
    }

    /// The queue's proposal items, each with its source's name.
    fn queue(&self) -> Value {
        self.ok(&["queue"])
    }
}

/// The accounts a Person's `about` lists as held, by identifier.
fn held(about: &Value) -> BTreeSet<String> {
    about["person"]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|account| account["identifier"].as_str().unwrap().to_string())
        .collect()
}

/// Two accounts share a verified email and the owner linked the first to a Person, so the
/// second is held by an automatic link. Returns both identifiers and the Person's.
fn linked_by_email(env: &Env) -> (String, String, String) {
    let first = env.account(101, "ana@example.com");
    let second = env.account(102, "ANA@example.com");
    let person = env.person("Ana");
    env.ok(&["identity", "link", &first, &person]);
    (first, second, person)
}

// spec: A retraction ends only what rests on it
#[test]
fn rejecting_the_premise_of_an_automatic_link_ends_the_link() {
    let env = Env::new();
    let (first, second, person) = linked_by_email(&env);
    assert_eq!(
        held(&env.about(&person)),
        BTreeSet::from([first.clone(), second.clone()]),
        "the automatic link holds before the correction"
    );

    let premise = env.claim(&second, rastro_core::identity::VERIFIED_EMAIL, "");
    env.reject(&premise);

    assert_eq!(
        held(&env.about(&person)),
        BTreeSet::from([first]),
        "{}",
        env.about(&person)
    );
}

// spec: A retraction ends only what rests on it
#[test]
fn a_link_the_owner_also_made_stands_when_its_premise_is_rejected() {
    let env = Env::new();
    let (first, second, person) = linked_by_email(&env);
    env.ok(&["identity", "link", &second, &person]);

    let premise = env.claim(&second, rastro_core::identity::VERIFIED_EMAIL, "");
    env.reject(&premise);

    let about = env.about(&person);
    assert_eq!(
        held(&about),
        BTreeSet::from([first, second.clone()]),
        "{about}"
    );
    let link = (about["person"]["accounts"].as_array().unwrap().iter())
        .find(|account| account["identifier"] == second)
        .unwrap();
    let reasons: Vec<&str> = link["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|reason| reason["kind"].as_str().unwrap())
        .collect();
    assert_eq!(reasons, ["owner_intent"], "{about}");
    let held_by = link["reasons"][0]["claim"].as_str().unwrap();
    let why = env.ok(&["why", held_by]);
    let current: Vec<&str> = (why["supports"].as_array().unwrap().iter())
        .filter(|support| support["current"] == true)
        .map(|support| support["justification_kind"].as_str().unwrap())
        .collect();
    assert_eq!(current, ["owner_intent"], "{why}");
}

/// An accepted promise: returns the commitment entity's identifier and the id of the item it
/// was accepted from. The accept itself asserts the commitment's anchor, its `std:Commitment` type claim.
fn accepted_commitment(env: &Env) -> (String, String) {
    env.import("weekly-1", "2026-11-10T10:00:00Z");
    let queue = env.queue();
    let proposal = (queue["items"].as_array().unwrap().iter())
        .find(|item| item["proposal"]["kind"] == "promise")
        .unwrap_or_else(|| panic!("no promise: {queue}"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    env.ok(&["queue", "accept", &proposal]);
    let identifier = format!("std:commitment/{proposal}");
    (identifier, proposal)
}

// spec: A retraction ends only what rests on it
#[test]
fn rejecting_a_commitments_anchor_takes_the_slots_only_it_justified() {
    let env = Env::with_adapter();
    let (commitment, _) = accepted_commitment(&env);
    // The due date has a justification of its own: a source observed it too.
    let subject = Entity::new(commitment.clone(), "std:Commitment".into());
    let due = Object::Literal {
        value: "by Friday".into(),
        datatype: "string".into(),
    };
    env.record(
        &subject,
        "std:due",
        due,
        ("https://meetings.test/weekly-1", "/due"),
    );
    let slots = |about: &Value| -> BTreeSet<String> {
        about["claims"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["predicate"].as_str().unwrap().starts_with("std:"))
            .map(|c| c["predicate"].as_str().unwrap().to_string())
            .collect()
    };
    let before = env.about(&commitment);
    let anchor = env.claim(&commitment, claims::TYPE_PREDICATE, "std:Commitment");
    assert_eq!(
        slots(&before),
        BTreeSet::from(["std:owes".into(), "std:owed_to".into(), "std:due".into()]),
        "{before}"
    );

    env.reject(&anchor);

    let after = env.about(&commitment);
    assert_eq!(slots(&after), BTreeSet::from(["std:due".into()]), "{after}");

    // A rebuild replays the acceptance and the retraction: the same slots stay.
    env.ok(&["claims", "rebuild"]);
    let rebuilt = env.about(&commitment);
    assert_eq!(
        slots(&rebuilt),
        BTreeSet::from(["std:due".into()]),
        "{rebuilt}"
    );
}

// spec: A retracted commitment is no longer known
#[test]
fn a_proposal_for_a_retracted_commitment_returns_marked_earlier_retracted() {
    let env = Env::with_adapter();
    let (commitment, _) = accepted_commitment(&env);
    // Another conversation states the same promise: the accepted commitment holds it.
    env.import("weekly-2", "2026-11-11T10:00:00Z");
    let before = env.queue();
    assert_eq!(before["already_known"], 1, "{before}");

    for predicate in ["std:owes", "std:owed_to", "std:due"] {
        let slot = env.claim(&commitment, predicate, "");
        env.reject(&slot);
    }

    let queue = env.queue();
    assert_eq!(queue["already_known"], 0, "{queue}");
    let promise = (queue["items"].as_array().unwrap().iter())
        .find(|item| item["source"]["name"] == "weekly-2" && item["proposal"]["kind"] == "promise")
        .unwrap_or_else(|| panic!("the promise is back: {queue}"));
    assert_eq!(promise["earlier_retracted"], true, "{queue}");
    let waiting = (queue["items"].as_array().unwrap().iter())
        .find(|item| item["proposal"]["kind"] == "waiting_on")
        .unwrap();
    assert!(waiting.get("earlier_retracted").is_none(), "{queue}");
}

// spec: A retracted commitment is no longer known
#[test]
fn a_proposal_for_a_statement_the_owner_fixed_away_returns_marked_earlier_retracted() {
    let env = Env::with_adapter();
    let (commitment, _) = accepted_commitment(&env);
    let due = env.claim(&commitment, "std:due", "by Friday");
    env.ok(&[
        "claim",
        "edit",
        due["id"].as_str().unwrap(),
        "--seen",
        due["revision"].as_str().unwrap(),
        "--object",
        "by Monday",
    ]);
    // Another conversation states the promise as it was first accepted, due Friday.
    env.import("weekly-2", "2026-11-11T10:00:00Z");

    let queue = env.queue();
    let promise = (queue["items"].as_array().unwrap().iter())
        .find(|item| item["source"]["name"] == "weekly-2" && item["proposal"]["kind"] == "promise")
        .unwrap_or_else(|| panic!("the promise is proposed again: {queue}"));
    assert_eq!(promise["earlier_retracted"], true, "{queue}");
}

/// Another conversation, the same speaker and recipient, a different promise and date: it shares
/// two statements with the accepted commitment and expresses a third of its own.
fn import_a_different_promise(env: &Env) {
    fs::write(
        env.path("transcript.txt"),
        "Alice: I'll send Bob the report by Monday.\n",
    )
    .unwrap();
    let candidate = json!([{"turn": 1, "kind": "promise", "responsible": "Alice",
        "recipient": "Bob", "promise": "send Bob the report", "stated_date": "by Monday",
        "quote": "I'll send Bob the report by Monday"}]);
    fs::write(env.path("candidates.json"), candidate.to_string()).unwrap();
    env.import("weekly-2", "2026-11-11T10:00:00Z");
}

/// The queue's promise proposal read from `weekly-2`.
fn weekly_2_promise(queue: &Value) -> &Value {
    (queue["items"].as_array().unwrap().iter())
        .find(|item| item["source"]["name"] == "weekly-2")
        .unwrap_or_else(|| panic!("no proposal from weekly-2: {queue}"))
}

// spec: A retracted commitment is no longer known
#[test]
fn a_proposal_that_only_shares_a_retracted_party_is_not_marked() {
    let env = Env::with_adapter();
    let (commitment, _) = accepted_commitment(&env);
    let recipient = env.claim(&commitment, "std:owed_to", "Bob");
    env.reject(&recipient);
    import_a_different_promise(&env);

    let queue = env.queue();
    let proposal = weekly_2_promise(&queue);
    assert!(proposal.get("earlier_retracted").is_none(), "{queue}");
}

// spec: A retracted commitment is no longer known
#[test]
fn a_proposal_that_only_shares_a_party_with_a_rejected_anchor_is_not_marked() {
    let env = Env::with_adapter();
    let (commitment, _) = accepted_commitment(&env);
    let anchor = env.claim(&commitment, claims::TYPE_PREDICATE, "std:Commitment");
    env.reject(&anchor);
    import_a_different_promise(&env);

    let queue = env.queue();
    let proposal = weekly_2_promise(&queue);
    assert!(proposal.get("earlier_retracted").is_none(), "{queue}");
}
