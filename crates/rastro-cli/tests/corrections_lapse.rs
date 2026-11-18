#![allow(non_snake_case)]
//! A correction follows its claim's lifecycle fingerprint (ADR 0014), end to end: a retraction
//! holds while the primary justification it was bound to holds and lapses when that one ends,
//! leaving the claim back with its evidence-derived status marked earlier retracted; a second
//! source corroborating the claim changes nothing the retraction is bound to; a confirmation
//! stays in force when its only evidence ends, marked stale, with the ended justification in
//! `why`'s history.
mod support;
use chrono::{DateTime, Utc};
use rastro_core::claims::{self, Entity, Object, ObservedClaim, Support};
use rastro_core::{Evidence, Scope, Source, Storage, TrustClass};
use rastro_storage_sqlite::SqliteStorage;
use serde_json::Value;
use std::{collections::BTreeSet, process::Command};
use tempfile::TempDir;

/// The extractor a fixture source is bound to; it is never installed.
const EXTRACTOR: &str = "missing-extractor";
const ACCOUNT: &str = "github:account/7";

fn time(stamp: &str) -> DateTime<Utc> {
    stamp.parse().unwrap()
}

struct Env {
    home: TempDir,
}
impl Env {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().unwrap(),
        }
    }
    fn ok(&self, args: &[&str]) -> Value {
        let out = Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.home.path().join("no-plugins"))
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
        assert!(out.status.success(), "{args:?}: {value}");
        value["result"].clone()
    }
    /// The login `value` of account 7, read at `location` by `source` on `day` (2026-11-<day>).
    /// A later capture at the same location replaces the earlier one.
    fn login(&self, source: &str, location: &str, value: &str, day: u32) {
        let captured = time(&format!("2026-11-{day:02}T00:00:00Z"));
        let source = Source::github(source.into(), "o/p".into(), Scope("personal".into()));
        let subject = Entity::new(ACCOUNT.into(), "code:Account".into());
        let object = Object::Literal {
            value: value.into(),
            datatype: "login".into(),
        };
        let mut db = SqliteStorage::open(&self.home.path().join("data/rastro.sqlite3")).unwrap();
        db.project_sources(std::slice::from_ref(&source)).unwrap();
        db.bind_extractor(&source.name, Some(EXTRACTOR)).unwrap();
        let run = db
            .start_run(&source, (EXTRACTOR, "1"), captured, (captured, captured))
            .unwrap();
        let evidence = Evidence {
            source: source.name.clone(),
            scope: source.scope.clone(),
            location: location.into(),
            field: "/login".into(),
            start: 0,
            end: value.len() as u64,
            quote: value.into(),
            trust_class: TrustClass::Structured,
            captured_at: captured,
            source_updated_at: Some(captured),
            original: None,
        };
        db.append_evidence(run, &evidence).unwrap();
        let (evidence_id, _) = db
            .pending_evidence((EXTRACTOR, "1"), &source)
            .unwrap()
            .into_iter()
            .rev()
            .find(|(_, item)| item.location == location && item.field == "/login")
            .unwrap();
        let observed = ObservedClaim {
            id: claims::claim_id(&subject.id, claims::ALIAS_PREDICATE, &object),
            subject,
            predicate: claims::ALIAS_PREDICATE.into(),
            object,
            scalar: true,
            type_ancestors: Vec::new(),
            supports: BTreeSet::from([Support {
                portable_key: claims::portable_evidence_key(&evidence),
                evidence_id,
                field: "/login".into(),
                start: 0,
                end: evidence.end,
                extraction_rule: "fixture-rule".into(),
            }]),
        };
        db.record_claims(run, (EXTRACTOR, "1"), &[observed], None)
            .unwrap();
    }
    /// The claim `about` lists for account 7's login `value`, if it lists one.
    fn listed(&self, value: &str) -> Option<Value> {
        let about = self.ok(&["about", ACCOUNT]);
        about["claims"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["predicate"] == claims::ALIAS_PREDICATE && c["object"]["value"] == value)
            .cloned()
    }
    fn seen(&self, value: &str) -> Value {
        self.listed(value)
            .unwrap_or_else(|| panic!("{value} is not listed"))
    }
    /// Records one correction of `claim` as the owner read it.
    fn correct(&self, verb: &str, claim: &Value) {
        self.ok(&[
            "claim",
            verb,
            claim["id"].as_str().unwrap(),
            "--seen",
            claim["revision"].as_str().unwrap(),
        ]);
    }
}

// spec: A correction follows its claim's lifecycle state
#[test]
fn a_retracted_claim_returns_marked_when_its_bound_occurrence_closes() {
    let env = Env::new();
    // Two sources say the login; the older reading is the one the retraction binds to.
    env.login("project", "https://api.github.com/users/7", "octocat", 1);
    env.login("mirror", "https://example.test/mirror/7", "octocat", 2);
    let seen = env.seen("octocat");
    env.correct("reject", &seen);
    assert!(env.listed("octocat").is_none(), "a retraction hides it");

    // The older reading is replaced: its occurrence closes, the mirror's still holds the claim.
    env.login("project", "https://api.github.com/users/7", "octocat2", 3);
    let returned = env
        .listed("octocat")
        .expect("the retraction lapsed with its occurrence");
    assert_eq!(returned["status"], "observed", "{returned}");
    assert_eq!(returned["earlier_retracted"], true, "{returned}");
    assert!(returned.get("stale").is_none(), "{returned}");
    // The claim the owner never rejected carries no mark.
    assert!(env.seen("octocat2").get("earlier_retracted").is_none());
}

// spec: A correction follows its claim's lifecycle state
#[test]
fn corroboration_from_another_source_keeps_a_retracted_claim_out() {
    let env = Env::new();
    env.login("project", "https://api.github.com/users/7", "octocat", 1);
    env.correct("reject", &env.seen("octocat"));
    assert!(env.listed("octocat").is_none());

    env.login("mirror", "https://example.test/mirror/7", "octocat", 2);
    assert!(
        env.listed("octocat").is_none(),
        "a second source changes the revision, not the lifecycle the retraction is bound to"
    );
}

// spec: A correction follows its claim's lifecycle state
#[test]
fn a_confirmed_claim_whose_only_evidence_ends_is_listed_confirmed_and_stale() {
    let env = Env::new();
    env.login("project", "https://api.github.com/users/7", "octocat", 1);
    let seen = env.seen("octocat");
    env.correct("confirm", &seen);
    assert!(env.seen("octocat").get("stale").is_none(), "fresh");

    env.login("project", "https://api.github.com/users/7", "octocat2", 3);
    let kept = env
        .listed("octocat")
        .expect("a confirmation outlives its evidence");
    assert_eq!(kept["status"], "confirmed", "{kept}");
    assert_eq!(kept["stale"], true, "{kept}");
    assert!(kept.get("earlier_retracted").is_none(), "{kept}");

    let why = env.ok(&["why", seen["id"].as_str().unwrap()]);
    assert_eq!(why["claim"]["stale"], true, "{why}");
    let provenance: Vec<&Value> = why["supports"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["justification_kind"] == "provenance")
        .collect();
    assert_eq!(provenance.len(), 1, "{why}");
    assert_eq!(provenance[0]["current"], false, "{why}");
}
