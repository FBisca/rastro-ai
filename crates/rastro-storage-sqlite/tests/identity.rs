#![allow(non_snake_case)]
//! Owner intents at the storage seam: the insert-only table, the issuer's visibility check, and
//! the scoped-read cache seeing an intent write.
use chrono::{DateTime, Utc};
use rastro_core::claims::{self, Entity, Object, ObservedClaim, Support};
use rastro_core::identity::{
    EntityTarget, FORMAT_VERSION, Intent, IntentKind, IssuerContext, OWNER, Payload, SHAPE_VERSION,
    Targets,
};
use rastro_core::knowledge::AboutOptions;
use rastro_core::{Evidence, Scope, Source, Storage, TrustClass};
use rastro_storage_sqlite::SqliteStorage;
use rusqlite::Connection;
use std::collections::BTreeSet;

fn time() -> DateTime<Utc> {
    "2026-11-01T00:00:00Z".parse().unwrap()
}
/// An account `github:account/<n>` typed `code:Account`, observed in `scope` only.
fn account(db: &mut SqliteStorage, n: u32, scope: &str) -> EntityTarget {
    let source = Source::github(format!("s-{scope}"), "o/p".into(), Scope(scope.into()));
    db.project_sources(std::slice::from_ref(&source)).unwrap();
    let run = db
        .start_run(&source, ("fixture", "1"), time(), (time(), time()))
        .unwrap();
    let location = format!("https://api.github.com/users/{n}");
    let e = Evidence {
        source: source.name.clone(),
        scope: source.scope.clone(),
        location: location.clone(),
        field: "/type".into(),
        start: 0,
        end: 4,
        quote: "User".into(),
        trust_class: TrustClass::Structured,
        captured_at: time(),
        source_updated_at: Some(time()),
        original: None,
    };
    db.append_evidence(run, &e).unwrap();
    let (evidence_id, _) = db
        .pending_evidence(("fixture", "1"), &source)
        .unwrap()
        .into_iter()
        .rev()
        .find(|(_, item)| item.location == location)
        .unwrap();
    let identifier = format!("github:account/{n}");
    let subject = Entity::new(identifier.clone(), "code:Account".into());
    let object = Object::Literal {
        value: "code:Account".into(),
        datatype: claims::TYPE_DATATYPE.into(),
    };
    db.record_claims(
        run,
        ("fixture", "1"),
        &[ObservedClaim {
            id: claims::claim_id(&subject.id, claims::TYPE_PREDICATE, &object),
            subject,
            predicate: claims::TYPE_PREDICATE.into(),
            object,
            scalar: true,
            type_ancestors: vec![],
            supports: BTreeSet::from([Support {
                portable_key: claims::portable_evidence_key(&e),
                evidence_id,
                field: "/type".into(),
                start: 0,
                end: 4,
                extraction_rule: "fixture-rule".into(),
            }]),
        }],
        None,
    )
    .unwrap();
    EntityTarget {
        id: claims::entity_id(&identifier),
        identifier,
        role: "account".into(),
    }
}
fn intent(id: &str, kind: IntentKind, targets: Targets, scopes: &[&str]) -> Intent {
    let mut intent = Intent {
        id: id.into(),
        append_seq: 0,
        format_version: FORMAT_VERSION,
        kind,
        owner_id: OWNER.into(),
        issuer_context: IssuerContext {
            principal: OWNER.into(),
            surface: "test".into(),
            scopes: scopes.iter().map(|s| Scope(s.to_string())).collect(),
        },
        issued_at: time(),
        domain_key: String::new(),
        targets,
        payload: Payload {
            version: SHAPE_VERSION,
            name: None,
        },
        undo_of: None,
        supersedes: None,
    };
    intent.domain_key = intent.expected_domain().unwrap_or_default();
    intent
}
fn person(n: u32) -> EntityTarget {
    let identifier = format!("rastro:person/{n}");
    EntityTarget {
        id: claims::entity_id(&identifier),
        identifier,
        role: "person".into(),
    }
}
fn create(id: &str, person: &EntityTarget) -> Intent {
    intent(
        id,
        IntentKind::CreatePerson,
        Targets::Entity {
            version: SHAPE_VERSION,
            entity: person.clone(),
        },
        &[],
    )
}
fn link(id: &str, account: &EntityTarget, person: &EntityTarget, scopes: &[&str]) -> Intent {
    intent(
        id,
        IntentKind::Link,
        Targets::Pair {
            version: SHAPE_VERSION,
            symmetric: false,
            endpoints: [account.clone(), person.clone()],
        },
        scopes,
    )
}
fn issue(db: &mut SqliteStorage, intents: Vec<Intent>) -> anyhow::Result<Vec<Intent>> {
    db.issue_intents(|_| Ok((intents, ())))
        .map(|(stored, ())| stored)
}
fn count(path: &std::path::Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row("SELECT count(*) FROM intent", [], |r| r.get(0))
        .unwrap()
}

/// S9: an intent issued from a context whose scopes can't see its target is refused at the
/// storage seam, and nothing is stored
#[test]
fn S9_an_intent_whose_issuer_cannot_see_its_target_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("db");
    let mut db = SqliteStorage::open(&path).unwrap();
    let private = account(&mut db, 1, "personal");
    let ana = person(1);
    issue(&mut db, vec![create("i_1", &ana)]).unwrap();
    // The account is visible only in `personal`; a `work` issuer can't see it.
    let error = issue(&mut db, vec![link("i_2", &private, &ana, &["work"])]).unwrap_err();
    assert!(error.to_string().contains("can't see"), "{error}");
    assert_eq!(count(&path), 1);
    // A Person with no linked account is visible in no scope (only `me` is everywhere), so
    // even an issuer that sees the account can't see that Person.
    let error = issue(&mut db, vec![link("i_2", &private, &ana, &["personal"])]).unwrap_err();
    assert!(error.to_string().contains("can't see"), "{error}");
    assert_eq!(count(&path), 1);
    // Unscoped, the link stands; then a `personal` issuer sees both ends, a `work` one neither.
    issue(&mut db, vec![link("i_2", &private, &ana, &[])]).unwrap();
    let other = account(&mut db, 2, "personal");
    issue(&mut db, vec![link("i_3", &other, &ana, &["personal"])]).unwrap();
    let work = account(&mut db, 3, "work");
    assert!(issue(&mut db, vec![link("i_4", &work, &ana, &["work"])]).is_err());
    assert_eq!(count(&path), 3);
    // An undo checks the targets of the intent it undoes, the same way.
    let mut undo = intent(
        "i_5",
        IntentKind::Undo,
        link("i_3", &other, &ana, &["work"]).targets,
        &["work"],
    );
    undo.domain_key = link("i_3", &other, &ana, &[]).domain_key;
    undo.undo_of = Some("i_3".into());
    assert!(issue(&mut db, vec![undo]).is_err());
    assert_eq!(count(&path), 3);
}

/// S11: an intent written after a scoped read was cached shows in the next scoped read
#[test]
fn S11_an_intent_written_after_a_cached_scoped_read_shows_in_the_next_one() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("db");
    let mut db = SqliteStorage::open(&path).unwrap();
    let octocat = account(&mut db, 1, "personal");
    let ana = person(1);
    issue(&mut db, vec![create("i_1", &ana)]).unwrap();
    let scopes = [Scope("personal".into())];
    let read = |db: &SqliteStorage| -> Vec<String> {
        db.snapshot(|view| {
            view.about_scoped(&octocat.identifier, &AboutOptions::default(), &scopes)
        })
        .unwrap()
        .into_iter()
        .map(|c| c.claim.predicate)
        .collect()
    };
    rastro_storage_sqlite::clear_scoped_cache();
    assert_eq!(read(&db), ["rastro:type"]);
    assert_eq!(rastro_storage_sqlite::scoped_cache_len(&path), 1);
    issue(&mut db, vec![link("i_2", &octocat, &ana, &[])]).unwrap();
    let mut after = read(&db);
    after.sort();
    assert_eq!(after, ["rastro:held_by", "rastro:type"]);
    // The Person is visible in `personal` now, through its linked account.
    let person = db
        .snapshot(|view| view.about_scoped(&ana.identifier, &AboutOptions::default(), &scopes))
        .unwrap();
    assert_eq!(person.len(), 2);
    // A scope that can't see the account sees neither the link nor the Person.
    let work = [Scope("work".into())];
    let hidden = db
        .snapshot(|view| view.about_scoped(&ana.identifier, &AboutOptions::default(), &work))
        .unwrap();
    assert!(hidden.is_empty());
}

/// The intent table is insert-only: no update, no delete, no replace, and a malformed row is
/// refused by the table itself
#[test]
fn the_intent_table_is_insert_only() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("db");
    let mut db = SqliteStorage::open(&path).unwrap();
    issue(&mut db, vec![create("i_1", &person(1))]).unwrap();
    let raw = Connection::open(&path).unwrap();
    assert!(raw.execute("UPDATE intent SET kind = 'name'", []).is_err());
    assert!(raw.execute("DELETE FROM intent", []).is_err());
    let row = "(id, format_version, kind, owner_id, issuer_context, issued_at, domain_key, targets_json, payload_json)";
    for key in ["append_seq", "rowid"] {
        assert!(
            raw.execute(&format!("INSERT OR REPLACE INTO intent ({key}, id, format_version, kind, owner_id, issuer_context, issued_at, domain_key, targets_json, payload_json) VALUES (1, 'i_9', 1, 'name', 'owner', '{{}}', 't', 'd', '{{}}', '{{}}')"), []).is_err(),
            "REPLACE via {key} must fail"
        );
    }
    assert!(raw.execute(&format!("INSERT OR REPLACE INTO intent {row} VALUES ('i_1', 1, 'name', 'owner', '{{}}', 't', 'd', '{{}}', '{{}}')"), []).is_err());
    for bad in [
        "('', 1, 'k', 'owner', '{}', 't', 'd', '{}', '{}')",
        "('i_9', 0, 'k', 'owner', '{}', 't', 'd', '{}', '{}')",
        "('i_9', 1, '', 'owner', '{}', 't', 'd', '{}', '{}')",
        "('i_9', 1, 'k', 'owner', 'not json', 't', 'd', '{}', '{}')",
        "('i_9', 1, 'k', 'owner', '{}', 't', '', '{}', '{}')",
        "('i_9', 1, 'k', 'owner', '{}', 't', 'd', '[]', '{}')",
        "('i_9', 1, 'k', 'owner', '{}', 't', 'd', '{}', '[]')",
    ] {
        assert!(
            raw.execute(&format!("INSERT INTO intent {row} VALUES {bad}"), [])
                .is_err(),
            "{bad}"
        );
    }
    // An undo names an earlier intent: one that doesn't exist is refused by the table too.
    raw.execute_batch("PRAGMA foreign_keys = ON").unwrap();
    assert!(raw.execute("INSERT INTO intent (id, format_version, kind, owner_id, issuer_context, issued_at, domain_key, targets_json, payload_json, undo_of) VALUES ('i_9', 1, 'undo', 'owner', '{}', 't', 'd', '{}', '{}', 'i_8')", []).is_err());
    assert_eq!(count(&path), 1);
    // A retry of a stored intent stores nothing; a different intent under its id is refused.
    issue(&mut db, vec![create("i_1", &person(1))]).unwrap();
    assert!(issue(&mut db, vec![create("i_1", &person(2))]).is_err());
    assert_eq!(count(&path), 1);
    // The core checks every row before it is stored: a forged target id is refused.
    let mut forged = person(3);
    forged.id = person(4).id;
    assert!(issue(&mut db, vec![create("i_2", &forged)]).is_err());
    assert_eq!(count(&path), 1);
}

/// A sync that touches nothing intent-backed keeps every intent-backed claim
#[test]
fn a_sync_keeps_the_claims_intents_back() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("db");
    let mut db = SqliteStorage::open(&path).unwrap();
    let octocat = account(&mut db, 1, "personal");
    let ana = person(1);
    issue(
        &mut db,
        vec![create("i_1", &ana), link("i_2", &octocat, &ana, &[])],
    )
    .unwrap();
    let claims = |db: &SqliteStorage| {
        db.snapshot(|view| view.about(&ana.identifier, &AboutOptions::default()))
            .unwrap()
            .len()
    };
    assert_eq!(claims(&db), 2);
    account(&mut db, 2, "personal");
    assert_eq!(claims(&db), 2);
}

/// `me` is visible in every scope; a Person with no linked account in none but the unscoped read
#[test]
fn me_is_visible_in_every_scope_and_an_unlinked_person_in_none() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("db");
    let mut db = SqliteStorage::open(&path).unwrap();
    account(&mut db, 1, "personal");
    let me = person(1);
    let mut minted = create("i_1", &me);
    minted.domain_key = rastro_core::identity::ME_DOMAIN.into();
    issue(&mut db, vec![minted, create("i_2", &person(2))]).unwrap();
    let about = |entity: &str, scopes: &[Scope]| {
        db.snapshot(|view| view.about_scoped(entity, &AboutOptions::default(), scopes))
            .unwrap()
            .len()
    };
    for scopes in [
        vec![],
        vec![Scope("work".into())],
        vec![Scope("personal".into())],
    ] {
        assert_eq!(about("me", &scopes), 1, "{scopes:?}");
        assert_eq!(about(&me.identifier, &scopes), 1, "{scopes:?}");
    }
    assert_eq!(about(&person(2).identifier, &[]), 1);
    assert_eq!(about(&person(2).identifier, &[Scope("personal".into())]), 0);
}
