#![allow(non_snake_case)]
use chrono::{DateTime, Utc};
use rastro_core::claims::{self, Entity, Object, ObservedClaim, Support};
use rastro_core::knowledge::AboutOptions;
use rastro_core::{Evidence, Scope, Source, Storage, TrustClass};
use rastro_storage_sqlite::SqliteStorage;
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn time() -> DateTime<Utc> {
    "2026-11-01T00:00:00Z".parse().unwrap()
}
/// Stores one claim about `location` read in `scope`; an alias is a `login` literal.
fn store(
    db: &mut SqliteStorage,
    scope: &str,
    location: &str,
    predicate: &str,
    value: &str,
    datatype: &str,
) -> String {
    let source = Source::github(format!("claims-{scope}"), "o/p".into(), Scope(scope.into()));
    db.project_sources(std::slice::from_ref(&source)).unwrap();
    let run = db
        .start_run(&source, ("fixture", "1"), time(), (time(), time()))
        .unwrap();
    let evidence = Evidence {
        source: source.name.clone(),
        scope: source.scope.clone(),
        location: location.into(),
        field: format!("/{predicate}"),
        start: 0,
        end: value.len() as u64,
        quote: value.into(),
        trust_class: TrustClass::Structured,
        captured_at: time(),
        source_updated_at: Some(time()),
        original: None,
    };
    db.append_evidence(run, &evidence).unwrap();
    let (evidence_id, evidence) = db
        .pending_evidence(("fixture", "1"), &source)
        .unwrap()
        .into_iter()
        .rev()
        .find(|(_, e)| e.field == evidence.field && e.quote == value)
        .unwrap();
    let subject = Entity::new(location.into(), "".into());
    let object = Object::Literal {
        value: value.into(),
        datatype: datatype.into(),
    };
    let id = claims::claim_id(&subject.id, predicate, &object);
    db.record_claims(
        run,
        ("fixture", "1"),
        &[ObservedClaim {
            id: id.clone(),
            subject,
            predicate: predicate.into(),
            object,
            scalar: true,
            type_ancestors: Vec::new(),
            supports: BTreeSet::from([Support {
                evidence_id,
                portable_key: claims::portable_evidence_key(&evidence),
                field: evidence.field,
                start: 0,
                end: evidence.end,
                extraction_rule: "fixture-rule".into(),
            }]),
        }],
        None,
    )
    .unwrap();
    id
}
fn about(db: &SqliteStorage, entity: &str) -> anyhow::Result<Value> {
    let claims = db.snapshot(|view| view.about(entity, &AboutOptions::default()))?;
    Ok(json!(claims.iter().map(|c| json!(c)).collect::<Vec<_>>()))
}

/// claims/R31: hypothesis and retracted alias claims are not lookup candidates
#[test]
fn S4_hypothesis_and_retracted_aliases_are_not_candidates() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("db");
    let mut db = SqliteStorage::open(&path).unwrap();
    for account in ["github:account/1", "github:account/2", "github:account/3"] {
        store(&mut db, "work", account, "rastro:alias", "shared", "login");
        store(&mut db, "work", account, "code:state", "active", "string");
    }
    let raw = rusqlite::Connection::open(&path).unwrap();
    // Today's schema only stores observed and superseded; a later slice adds the others.
    raw.execute_batch("PRAGMA ignore_check_constraints = ON")
        .unwrap();
    let set = |account: &str, status: &str| {
        raw.execute(
            "UPDATE claim SET status = ?2 WHERE predicate = 'rastro:alias' AND subject = (SELECT id FROM entity WHERE identifier = ?1)",
            [account, status],
        )
        .unwrap();
    };
    set("github:account/1", "hypothesis");
    set("github:account/2", "retracted");
    // Only account 3 is left, so the login resolves directly.
    let found = about(&db, "github:login/shared").unwrap();
    assert_eq!(found, about(&db, "github:account/3").unwrap());
    assert!(!found.as_array().unwrap().is_empty());
    set("github:account/3", "retracted");
    assert_eq!(about(&db, "github:login/shared").unwrap(), json!([]));
}

/// surfaces/R28: hidden alias claims never spend a scoped read's work budget
#[test]
fn S6_hidden_alias_claims_over_budget_answer_like_an_unknown_entity() {
    let home = tempfile::tempdir().unwrap();
    let mut db = SqliteStorage::open(&home.path().join("db")).unwrap();
    for n in 0..5 {
        let account = format!("github:account/{n}");
        store(&mut db, "work", &account, "rastro:alias", "shared", "login");
        store(
            &mut db,
            "personal",
            &account,
            "code:state",
            "active",
            "string",
        );
    }
    let scopes = [Scope("personal".into())];
    let run = |db: &mut SqliteStorage, entity: &str| {
        db.about_scoped_within(entity, &AboutOptions::default(), &scopes, 2)
            .map(|claims| json!(claims.iter().map(|c| json!(c)).collect::<Vec<_>>()))
            .map_err(|e| e.to_string())
    };
    let unknown = run(&mut db, "github:login/nobody");
    assert_eq!(unknown, Ok(json!([])));
    assert_eq!(run(&mut db, "github:login/shared"), unknown);
}
