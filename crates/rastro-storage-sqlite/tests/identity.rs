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
/// Accepts intents as their issuer, who sees the scopes its context lists (none: every scope).
fn issue(db: &mut SqliteStorage, intents: Vec<Intent>) -> anyhow::Result<Vec<Intent>> {
    let sees = intents
        .first()
        .map(|i| i.issuer_context.scopes.clone())
        .unwrap_or_default();
    db.issue_intents(&sees, |_| Ok((intents, ())))
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
    // The Person is visible in `personal` now, through its linked account, whose claims its
    // view shows (S28).
    let person = db
        .snapshot(|view| view.about_scoped(&ana.identifier, &AboutOptions::default(), &scopes))
        .unwrap();
    assert_eq!(person.len(), 3);
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
fn S8_the_intent_table_is_insert_only() {
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
fn S10_a_sync_keeps_the_claims_intents_back() {
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
    // The Person's type, its link and, in its view, the linked account's type (S28).
    assert_eq!(claims(&db), 3);
    account(&mut db, 2, "personal");
    assert_eq!(claims(&db), 3);
}

/// `me` is visible in every scope; a Person with no linked account in none but the unscoped read
#[test]
fn S9_me_is_visible_in_every_scope_and_an_unlinked_person_in_none() {
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

/// An undo naming `undone`, from an issuer seeing `scopes`.
fn undo(id: &str, undone: &Intent, scopes: &[&str]) -> Intent {
    let mut undo = intent(id, IntentKind::Undo, undone.targets.clone(), scopes);
    undo.domain_key = undone.domain_key.clone();
    undo.undo_of = Some(undone.id.clone());
    undo
}
/// The cannot-link an undo of `pair` records, from an issuer seeing `scopes`.
fn cannot_link(id: &str, a: &EntityTarget, b: &EntityTarget, scopes: &[&str]) -> Intent {
    let (first, second) = if a.id < b.id { (a, b) } else { (b, a) };
    intent(
        id,
        IntentKind::CannotLink,
        Targets::Pair {
            version: SHAPE_VERSION,
            symmetric: true,
            endpoints: [first.clone(), second.clone()],
        },
        scopes,
    )
}

/// S9: an issuer that sees a Person only through its one link can undo that link: the undo and
/// its cannot-link are checked against the state the issuer saw, not one the batch changed
#[test]
fn S9_a_scoped_issuer_can_undo_a_persons_only_link() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("db");
    let mut db = SqliteStorage::open(&path).unwrap();
    let octocat = account(&mut db, 1, "personal");
    let ana = person(1);
    let linked = link("i_2", &octocat, &ana, &[]);
    issue(&mut db, vec![create("i_1", &ana), linked.clone()]).unwrap();
    issue(
        &mut db,
        vec![
            undo("i_3", &linked, &["personal"]),
            cannot_link("i_4", &octocat, &ana, &["personal"]),
        ],
    )
    .unwrap();
    assert_eq!(count(&path), 4);
}

/// S8: storage refuses what the ADR refuses, whoever builds the intent: a link from a Person or
/// to an account, a merge of entities that aren't both Persons, an undo of a cannot-link, an undo
/// or a Person's creation, a second `me`, and a second creation of one Person
#[test]
fn S8_storage_refuses_invalid_intents_whoever_builds_them() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("db");
    let mut db = SqliteStorage::open(&path).unwrap();
    let octocat = account(&mut db, 1, "personal");
    let hubot = account(&mut db, 2, "personal");
    let (ana, silva) = (person(1), person(2));
    let mut me = create("i_3", &person(3));
    me.domain_key = rastro_core::identity::ME_DOMAIN.into();
    let linked = link("i_4", &octocat, &ana, &[]);
    issue(
        &mut db,
        vec![
            create("i_1", &ana),
            create("i_2", &silva),
            me,
            linked.clone(),
        ],
    )
    .unwrap();
    let undone = undo("i_5", &linked, &[]);
    let refusal = cannot_link("i_6", &octocat, &ana, &[]);
    issue(&mut db, vec![undone.clone(), refusal.clone()]).unwrap();
    let before = count(&path);
    let as_account = |e: &EntityTarget| EntityTarget {
        role: "account".into(),
        ..e.clone()
    };
    let as_person = |e: &EntityTarget| EntityTarget {
        role: "person".into(),
        ..e.clone()
    };
    let merge = |a: &EntityTarget, b: &EntityTarget| {
        let mut m = cannot_link("i_9", &as_person(a), &as_person(b), &[]);
        m.kind = IntentKind::SameAs;
        m.domain_key = m.expected_domain().unwrap();
        m
    };
    let mut second_me = create("i_9", &person(4));
    second_me.domain_key = rastro_core::identity::ME_DOMAIN.into();
    for (intent, reason) in [
        (link("i_9", &as_account(&ana), &silva, &[]), "is a Person"),
        (
            link("i_9", &hubot, &as_person(&octocat), &[]),
            "is not a Person",
        ),
        (merge(&octocat, &hubot), "accounts never merge"),
        (merge(&octocat, &silva), "only Persons merge"),
        (undo("i_9", &refusal, &[]), "cannot-link can't be undone"),
        (undo("i_9", &undone, &[]), "an undo can't be undone"),
        (
            undo("i_9", &create("i_1", &ana), &[]),
            "creating a Person can't be undone",
        ),
        (undo("i_9", &linked, &[]), "already undone"),
        (second_me, "already exists"),
        (create("i_9", &silva), "already exists"),
    ] {
        let error = issue(&mut db, vec![intent]).unwrap_err().to_string();
        assert!(error.contains(reason), "{reason}: {error}");
        assert_eq!(count(&path), before, "{reason}");
    }
}

/// S11: an intent write publishes the projection, as any other projection write does
#[test]
fn S11_an_intent_write_publishes_the_projection() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("db");
    let mut db = SqliteStorage::open(&path).unwrap();
    let publications = || -> i64 {
        Connection::open(&path)
            .unwrap()
            .query_row("SELECT count(*) FROM projection_publication", [], |r| {
                r.get(0)
            })
            .unwrap()
    };
    let before = publications();
    issue(&mut db, vec![create("i_1", &person(1))]).unwrap();
    assert_eq!(publications(), before + 1);
}

fn me(n: u32) -> (EntityTarget, Intent) {
    let me = person(n);
    let mut minted = create(&format!("i_me{n}"), &me);
    minted.domain_key = rastro_core::identity::ME_DOMAIN.into();
    (me, minted)
}
fn named(id: &str, person: &EntityTarget, name: &str, minutes: i64) -> Intent {
    let mut named = intent(
        id,
        IntentKind::Name,
        Targets::Entity {
            version: SHAPE_VERSION,
            entity: person.clone(),
        },
        &[],
    );
    named.payload.name = Some(name.into());
    named.issued_at = time() + chrono::Duration::minutes(minutes);
    named
}

/// S33: `about me` whose accounts' claims exceed the work budget fails with the budget error,
/// scoped or not, instead of answering partially; a merged Person's whole class is under the
/// same budget
#[test]
fn S33_about_me_over_the_work_budget_fails_instead_of_answering_partially() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("db");
    let mut db = SqliteStorage::open(&path).unwrap();
    let octocat = account(&mut db, 1, "personal");
    let hubot = account(&mut db, 2, "personal");
    let (me, minted) = me(9);
    let ana = person(1);
    issue(
        &mut db,
        vec![
            minted,
            create("i_1", &ana),
            link("i_2", &octocat, &me, &[]),
            link("i_3", &hubot, &ana, &[]),
        ],
    )
    .unwrap();
    let personal = [Scope("personal".into())];
    let options = AboutOptions::default();
    // One visible span per account: `me` alone costs one, merged with Ana two.
    for scopes in [&[][..], &personal[..]] {
        assert_eq!(
            db.about_scoped_within("me", &options, scopes, 1)
                .unwrap()
                .len(),
            3,
            "{scopes:?}"
        );
        let error = db
            .about_scoped_within("me", &options, scopes, 0)
            .unwrap_err();
        assert!(error.to_string().contains("work budget"), "{error}");
    }
    let merge = {
        let mut m = cannot_link("i_4", &me, &ana, &[]);
        m.kind = IntentKind::SameAs;
        m.domain_key = m.expected_domain().unwrap();
        m
    };
    issue(&mut db, vec![merge]).unwrap();
    for scopes in [&[][..], &personal[..]] {
        let error = db
            .about_scoped_within("me", &options, scopes, 1)
            .unwrap_err();
        assert!(
            error.to_string().contains("work budget"),
            "{scopes:?}: {error}"
        );
        assert!(db.about_scoped_within("me", &options, scopes, 2).is_ok());
    }
}

/// S36: naming a Person again keeps the older name as a superseded alias, its interval closed at
/// the newer name's time; the older name still resolves to the Person (historical), and so does
/// the newer (current)
#[test]
fn S36_an_older_name_stays_superseded_and_still_resolves() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("db");
    let mut db = SqliteStorage::open(&path).unwrap();
    let ana = person(1);
    issue(
        &mut db,
        vec![
            create("i_1", &ana),
            named("i_2", &ana, "Ana", 1),
            named("i_3", &ana, "Ana Silva", 2),
        ],
    )
    .unwrap();
    let history = AboutOptions {
        history: true,
        ..AboutOptions::default()
    };
    let claims = db
        .snapshot(|view| view.about(&ana.identifier, &history))
        .unwrap();
    let name = |value: &str| {
        claims
            .iter()
            .find(|c| {
                c.claim.predicate == claims::ALIAS_PREDICATE
                    && matches!(&c.claim.object, Object::Literal { value: v, .. } if v == value)
            })
            .unwrap_or_else(|| panic!("no name {value}"))
    };
    let older = name("Ana");
    assert_eq!(older.claim.status, "superseded");
    assert_eq!(older.claim.known_intervals.len(), 1);
    assert_eq!(
        older.claim.known_intervals[0].start,
        time() + chrono::Duration::minutes(1)
    );
    assert_eq!(
        older.claim.known_intervals[0].end,
        Some(time() + chrono::Duration::minutes(2))
    );
    let newer = name("Ana Silva");
    assert_eq!(newer.claim.status, "observed");
    assert_eq!(newer.claim.known_intervals[0].end, None);
    // Without history only the current name shows.
    let current = db
        .snapshot(|view| view.about(&ana.identifier, &AboutOptions::default()))
        .unwrap();
    assert_eq!(
        current
            .iter()
            .filter(|c| c.claim.predicate == claims::ALIAS_PREDICATE)
            .count(),
        1
    );
    // Both names resolve to the Person.
    for form in ["rastro:name/Ana", "rastro:name/Ana Silva"] {
        let found = db
            .snapshot(|view| view.resolve_entity(form))
            .unwrap()
            .map(|(id, _)| id);
        assert_eq!(found.as_deref(), Some(ana.id.as_str()), "{form}");
        let claims = db
            .snapshot(|view| view.about(form, &AboutOptions::default()))
            .unwrap();
        assert!(!claims.is_empty(), "{form}");
    }
}

fn same_as(id: &str, a: &EntityTarget, b: &EntityTarget) -> Intent {
    let mut merge = cannot_link(id, a, b, &[]);
    merge.kind = IntentKind::SameAs;
    merge.domain_key = merge.expected_domain().unwrap();
    merge
}

/// S33: a merge chain through a Person the scope set can't see costs nothing past it: `about P`
/// in `personal`, with P–Q–R merged and Q held only through a `work` account, replays only P's
/// own account, so the budget P alone needs is enough
#[test]
fn S33_a_merge_chain_through_a_hidden_person_adds_no_work_past_it() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("db");
    let mut db = SqliteStorage::open(&path).unwrap();
    let (a1, a2, a3) = (
        account(&mut db, 1, "personal"),
        account(&mut db, 2, "work"),
        account(&mut db, 3, "personal"),
    );
    let (p, q, r) = (person(1), person(2), person(3));
    issue(
        &mut db,
        vec![
            create("i_1", &p),
            create("i_2", &q),
            create("i_3", &r),
            link("i_4", &a1, &p, &[]),
            link("i_5", &a2, &q, &[]),
            link("i_6", &a3, &r, &[]),
            same_as("i_7", &p, &q),
            same_as("i_8", &q, &r),
        ],
    )
    .unwrap();
    let personal = [Scope("personal".into())];
    let options = AboutOptions::default();
    let claims = db
        .about_scoped_within(&p.identifier, &options, &personal, 1)
        .unwrap();
    // P's type, its link and its account's type; nothing of Q or R.
    assert_eq!(claims.len(), 3, "{claims:?}");
    // Unscoped, the whole chain is one class: three spans.
    assert!(
        db.about_scoped_within(&p.identifier, &options, &[], 2)
            .is_err()
    );
    assert!(
        db.about_scoped_within(&p.identifier, &options, &[], 3)
            .is_ok()
    );
}

/// S36: undoing a name a newer one had already replaced ends it at the replacement, not at the
/// undo, so the two names never overlap
#[test]
fn S36_undoing_an_already_replaced_name_ends_it_at_the_replacement() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("db");
    let mut db = SqliteStorage::open(&path).unwrap();
    let ana = person(1);
    let older = named("i_2", &ana, "Ana", 1);
    let mut undone = undo("i_4", &older, &[]);
    undone.issued_at = time() + chrono::Duration::minutes(3);
    issue(
        &mut db,
        vec![
            create("i_1", &ana),
            older,
            named("i_3", &ana, "Ana Silva", 2),
            undone,
        ],
    )
    .unwrap();
    let history = AboutOptions {
        history: true,
        ..AboutOptions::default()
    };
    let claims = db
        .snapshot(|view| view.about(&ana.identifier, &history))
        .unwrap();
    let interval = |value: &str| {
        let claim = claims
            .iter()
            .find(|c| matches!(&c.claim.object, Object::Literal { value: v, .. } if v == value))
            .unwrap_or_else(|| panic!("no name {value}"));
        (
            claim.claim.status.clone(),
            claim.claim.known_intervals.clone(),
        )
    };
    let (status, intervals) = interval("Ana");
    assert_eq!(status, "superseded");
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals[0].start, time() + chrono::Duration::minutes(1));
    assert_eq!(
        intervals[0].end,
        Some(time() + chrono::Duration::minutes(2))
    );
    let (status, intervals) = interval("Ana Silva");
    assert_eq!(status, "observed");
    assert_eq!(intervals[0].start, time() + chrono::Duration::minutes(2));
}
