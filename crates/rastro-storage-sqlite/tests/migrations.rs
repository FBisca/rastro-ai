#![allow(non_snake_case)]
use rastro_core::{Scope, Source, Storage};
use rastro_storage_sqlite::SqliteStorage;
use rusqlite::Connection;
use std::fs;

/// evidence/R4: stored evidence rejects updates and deletes
#[test]
fn stored_evidence_rejects_updates_and_deletes() {
    for legacy in [false, true] {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("rastro.sqlite3");
        if legacy {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(include_str!("../migrations/202611041200_initial.sql"))
                .unwrap();
            connection.pragma_update(None, "user_version", 1).unwrap();
        }
        let mut storage = SqliteStorage::open(&path).unwrap();
        storage
            .project_sources(&[Source::github(
                "project".into(),
                "o/p".into(),
                Scope("personal".into()),
            )])
            .unwrap();
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("PRAGMA recursive_triggers = OFF;")
            .unwrap();
        connection.execute("INSERT INTO evidence (source, scope, location, field, start, end, quote, captured_at, trust_class) VALUES ('project', 'personal', 'https://example.org', '/body', 0, 2, 'é', '2026-11-04T00:00:00Z', 'untrusted content')", []).unwrap();
        assert!(
            connection
                .execute("UPDATE evidence SET quote='ab'", [])
                .is_err()
        );
        assert!(connection.execute("DELETE FROM evidence", []).is_err());
        for key in ["id", "rowid"] {
            assert!(connection.execute(&format!("INSERT OR REPLACE INTO evidence ({key}, source, scope, location, field, start, end, quote, captured_at, trust_class) VALUES (1, 'project', 'personal', 'https://example.org', '/body', 0, 2, 'bb', '2026-11-04T00:00:00Z', 'untrusted content')"), []).is_err(), "REPLACE via {key} must fail");
        }
        drop(storage);
        SqliteStorage::open(&path).unwrap();
        let quote: String = connection
            .query_row("SELECT quote FROM evidence", [], |row| row.get(0))
            .unwrap();
        assert_eq!(quote, "é");
        let version: u32 = connection
            .query_row("SELECT count(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, 15);
    }
}

/// evidence/R5: migrations apply once in timestamp order and unknown ones are refused
#[test]
fn migrations_apply_once_in_timestamp_order_and_unknown_ones_are_refused() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("rastro.sqlite3");
    let a = ("202611040001_a", "CREATE TABLE a (x)");
    let b = ("202611040002_b", "CREATE TABLE b (x)");
    let c = ("202611040003_c", "CREATE TABLE c (x)");
    SqliteStorage::open_with(&path, &[a, c]).unwrap();
    // A branch merged later brings a migration older than the latest applied one.
    SqliteStorage::open_with(&path, &[c, b, a]).unwrap();
    SqliteStorage::open_with(&path, &[a, b, c]).unwrap();
    let connection = Connection::open(&path).unwrap();
    let names = |connection: &Connection| -> Vec<String> {
        connection
            .prepare("SELECT name FROM schema_migrations ORDER BY rowid")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(names(&connection), [a.0, c.0, b.0]);
    // A new database applies them in timestamp order whatever order they are listed in.
    let fresh = home.path().join("fresh.sqlite3");
    SqliteStorage::open_with(&fresh, &[c, a, b]).unwrap();
    assert_eq!(names(&Connection::open(&fresh).unwrap()), [a.0, b.0, c.0]);
    // A database holding a migration this binary doesn't know is refused.
    let error = SqliteStorage::open_with(&path, &[a, b]).err().unwrap();
    assert!(format!("{error:#}").contains(c.0), "{error:#}");
    // The built-in set upgrades a database from before migrations were tracked by name.
    let legacy = home.path().join("legacy.sqlite3");
    let connection = Connection::open(&legacy).unwrap();
    connection
        .execute_batch(include_str!("../migrations/202611041200_initial.sql"))
        .unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
    SqliteStorage::open(&legacy).unwrap();
    assert_eq!(names(&connection).len(), 15);
}

/// evidence/R5: legacy user version maps to the migrations it meant
#[test]
fn legacy_user_version_maps_to_the_migrations_it_meant() {
    let initial = include_str!("../migrations/202611041200_initial.sql");
    let guard = include_str!("../migrations/202611041500_evidence_insert_guard.sql");
    let legacy = |version: i64, sql: &[&str]| {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("rastro.sqlite3");
        let connection = Connection::open(&path).unwrap();
        for sql in sql {
            connection.execute_batch(sql).unwrap();
        }
        connection
            .pragma_update(None, "user_version", version)
            .unwrap();
        (home, path)
    };
    let (_home, path) = legacy(2, &[initial, guard]);
    SqliteStorage::open(&path).unwrap();
    let connection = Connection::open(&path).unwrap();
    let count: i64 = connection
        .query_row("SELECT count(*) FROM evidence_fts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
    let (_home, path) = legacy(3, &[initial, guard]);
    let error = SqliteStorage::open(&path).err().unwrap();
    assert!(format!("{error:#}").contains("user_version 3"), "{error:#}");
}

/// claims/R1: evidence appended while the clock reads earlier than an extracted observation is still
/// pending for the extractor: capture times never go backwards within a (source, scope)
#[test]
fn evidence_captured_with_an_earlier_clock_is_still_extracted() {
    use chrono::{DateTime, Utc};
    use rastro_core::{Evidence, TrustClass, claims::Checkpoint};
    let home = tempfile::tempdir().unwrap();
    let mut storage = SqliteStorage::open(&home.path().join("rastro.sqlite3")).unwrap();
    let source = Source::github("project".into(), "o/p".into(), Scope("personal".into()));
    storage
        .project_sources(std::slice::from_ref(&source))
        .unwrap();
    let at = |text: &str| text.parse::<DateTime<Utc>>().unwrap();
    let observed = |location: &str, captured_at: DateTime<Utc>| Evidence {
        source: "project".into(),
        scope: Scope("personal".into()),
        location: location.into(),
        field: "/state".into(),
        start: 0,
        end: 4,
        quote: "open".into(),
        trust_class: TrustClass::Structured,
        captured_at,
        source_updated_at: None,
        original: None,
    };
    let window = (at("2026-11-01T00:00:00Z"), at("2026-11-02T00:00:00Z"));
    let run = storage
        .start_run(&source, ("test", "0.1.0"), window.0, window)
        .unwrap();
    storage
        .append_evidence(run, &observed("https://x/1", at("2026-11-04T12:00:00Z")))
        .unwrap();
    let extractor = ("test-extractor", "0.1.0");
    let pending = storage.pending_evidence(extractor, &source).unwrap();
    let (id, first) = &pending[0];
    let extraction = storage
        .start_extraction(&source, extractor, window.0)
        .unwrap();
    let checkpoint = Checkpoint {
        extractor: extractor.0.into(),
        extractor_version: extractor.1.into(),
        source: "project".into(),
        scope: Scope("personal".into()),
        vocabulary_version: "1".into(),
        last_portable_key: rastro_core::claims::portable_evidence_key(first),
        last: (first.captured_at, *id),
    };
    storage
        .record_claims(extraction, extractor, &[], Some(&checkpoint))
        .unwrap();
    // The clock went back an hour before the next observation.
    storage
        .append_evidence(run, &observed("https://x/2", at("2026-11-04T11:00:00Z")))
        .unwrap();
    let pending = storage.pending_evidence(extractor, &source).unwrap();
    let locations: Vec<_> = pending.iter().map(|(_, e)| e.location.as_str()).collect();
    assert_eq!(locations, ["https://x/2"]);
    assert!(pending[0].1.captured_at >= first.captured_at);
}

/// claims/R24: projection publication receipts reject updates and deletes
#[test]
fn projection_publication_receipts_reject_updates_and_deletes() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("rastro.sqlite3");
    SqliteStorage::open(&path).unwrap();
    let db = Connection::open(path).unwrap();
    db.execute("INSERT INTO projection_publication (extractor_versions, vocabulary_versions, evidence_high_water) VALUES ('{}', '{}', '[]')", []).unwrap();
    assert!(
        db.execute(
            "UPDATE projection_publication SET evidence_high_water = '[]'",
            []
        )
        .is_err()
    );
    assert!(
        db.execute("DELETE FROM projection_publication", [])
            .is_err()
    );
}

/// claims/R17: entity types are never stored as a column
#[test]
fn entity_types_are_never_stored_as_a_column() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("rastro.sqlite3");
    SqliteStorage::open(&path).unwrap();
    let db = Connection::open(path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM pragma_table_info('entity') WHERE name = 'type'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

/// claims/R29: temp shadow tables match the migrated projection tables
#[test]
fn temp_shadow_tables_match_the_migrated_projection_tables() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("rastro.sqlite3");
    SqliteStorage::open(&path).unwrap();
    let db = Connection::open(path).unwrap();
    let ddl = include_str!("../src/projection_temp.sql");
    db.execute_batch(ddl).unwrap();
    let shape = |table: &str, schema: &str| -> Vec<(String, String, i64, Option<String>, i64)> {
        let mut stmt = db
            .prepare(&format!(
                "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info('{table}', '{schema}') ORDER BY cid"
            ))
            .unwrap();
        stmt.query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    };
    let mut seen = 0;
    for line in ddl.lines() {
        if let Some(rest) = line.trim().strip_prefix("CREATE TEMP TABLE ") {
            let table = rest.split_whitespace().next().unwrap();
            let main = shape(table, "main");
            assert!(!main.is_empty(), "{table} missing from the migration");
            assert_eq!(main, shape(table, "temp"), "{table} shape differs");
            seen += 1;
        }
    }
    assert!(seen >= 6, "expected the projection tables, saw {seen}");
}

/// claims/R58: the owner-intent migration adds the insert-only intent table and leaves the projection a
/// database already holds in place: no claim is lost to the upgrade
#[test]
fn the_owner_intent_migration_keeps_the_existing_projection() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("rastro.sqlite3");
    let before: [(&str, &str); 8] = [
        (
            "202611041200_initial",
            include_str!("../migrations/202611041200_initial.sql"),
        ),
        (
            "202611041500_evidence_insert_guard",
            include_str!("../migrations/202611041500_evidence_insert_guard.sql"),
        ),
        (
            "202611041700_evidence_full_text_index",
            include_str!("../migrations/202611041700_evidence_full_text_index.sql"),
        ),
        (
            "202611050900_run_window",
            include_str!("../migrations/202611050900_run_window.sql"),
        ),
        (
            "202611051100_run_evidence",
            include_str!("../migrations/202611051100_run_evidence.sql"),
        ),
        (
            "202611051300_coverage_pair_indexes",
            include_str!("../migrations/202611051300_coverage_pair_indexes.sql"),
        ),
        (
            "202611061000_claims_projection",
            include_str!("../migrations/202611061000_claims_projection.sql"),
        ),
        (
            "202611061200_knowledge_v2",
            include_str!("../migrations/202611061200_knowledge_v2.sql"),
        ),
    ];
    drop(SqliteStorage::open_with(&path, &before).unwrap());
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "INSERT INTO entity (id, identifier) VALUES ('e_1', 'github:account/1');
             INSERT INTO claim (id, subject, predicate, object_value, object_datatype, scalar, status, revision)
             VALUES ('c_1', 'e_1', 'rastro:alias', 'octocat', 'login', 1, 'observed', 'r_1');",
        )
        .unwrap();
    drop(SqliteStorage::open(&path).unwrap());
    let count = |sql: &str| -> i64 { connection.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(count("SELECT count(*) FROM claim"), 1);
    assert_eq!(count("SELECT count(*) FROM entity"), 1);
    assert_eq!(count("SELECT count(*) FROM intent"), 0);
    assert_eq!(count("SELECT count(*) FROM intent_justification"), 0);
    assert_eq!(
        count("SELECT count(*) FROM sqlite_master WHERE type = 'trigger' AND tbl_name = 'intent'"),
        3
    );
}

/// The migrations before the one recording when an intent's contribution ended.
fn before_intent_end() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "202611041200_initial",
            include_str!("../migrations/202611041200_initial.sql"),
        ),
        (
            "202611041500_evidence_insert_guard",
            include_str!("../migrations/202611041500_evidence_insert_guard.sql"),
        ),
        (
            "202611041700_evidence_full_text_index",
            include_str!("../migrations/202611041700_evidence_full_text_index.sql"),
        ),
        (
            "202611050900_run_window",
            include_str!("../migrations/202611050900_run_window.sql"),
        ),
        (
            "202611051100_run_evidence",
            include_str!("../migrations/202611051100_run_evidence.sql"),
        ),
        (
            "202611051300_coverage_pair_indexes",
            include_str!("../migrations/202611051300_coverage_pair_indexes.sql"),
        ),
        (
            "202611061000_claims_projection",
            include_str!("../migrations/202611061000_claims_projection.sql"),
        ),
        (
            "202611061200_knowledge_v2",
            include_str!("../migrations/202611061200_knowledge_v2.sql"),
        ),
        (
            "202611071000_owner_intents",
            include_str!("../migrations/202611071000_owner_intents.sql"),
        ),
    ]
}

/// The intents of the upgrade fixture: a Person, a link that stands, and a link undone (with
/// its cannot-link).
fn upgrade_intents() -> Vec<rastro_core::identity::Intent> {
    use rastro_core::claims::entity_id;
    use rastro_core::identity::{
        EntityTarget, FORMAT_VERSION, Intent, IntentKind, IssuerContext, OWNER, Payload,
        SHAPE_VERSION, Targets,
    };
    let target = |identifier: &str, role: &str| EntityTarget {
        id: entity_id(identifier),
        identifier: identifier.into(),
        role: role.into(),
    };
    let person = target("rastro:person/1", "person");
    let (undone, kept) = (
        target("github:account/7", "account"),
        target("github:account/8", "account"),
    );
    let at = |minute: i64| {
        "2026-11-01T00:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
            + chrono::Duration::minutes(minute)
    };
    let intent = |n: i64, kind: IntentKind, targets: Targets, undo_of: Option<&str>| {
        let mut i = Intent {
            id: format!("i_{n}"),
            append_seq: n,
            format_version: FORMAT_VERSION,
            kind,
            owner_id: OWNER.into(),
            issuer_context: IssuerContext::owner_cli(&[]),
            issued_at: at(n),
            domain_key: String::new(),
            targets,
            payload: Payload {
                version: SHAPE_VERSION,
                name: None,
            },
            undo_of: undo_of.map(Into::into),
            supersedes: None,
        };
        i.domain_key = i.expected_domain().unwrap_or_default();
        i
    };
    let link = |account: &EntityTarget| Targets::Pair {
        version: SHAPE_VERSION,
        symmetric: false,
        endpoints: [account.clone(), person.clone()],
    };
    let mut refusal = [undone.clone(), person.clone()];
    refusal.sort();
    let first = intent(2, IntentKind::Link, link(&undone), None);
    let mut undo = intent(3, IntentKind::Undo, link(&undone), Some("i_2"));
    undo.domain_key = first.domain_key.clone();
    vec![
        intent(
            1,
            IntentKind::CreatePerson,
            Targets::Entity {
                version: SHAPE_VERSION,
                entity: person.clone(),
            },
            None,
        ),
        first,
        undo,
        intent(
            4,
            IntentKind::CannotLink,
            Targets::Pair {
                version: SHAPE_VERSION,
                symmetric: true,
                endpoints: refusal,
            },
            None,
        ),
        intent(5, IntentKind::Link, link(&kept), None),
    ]
}

/// A database as main left it before the intent-end migration: an evidence claim, a claim no
/// justification and no intent backs, and, `with_intents`, the upgrade intents with the
/// projection rows main wrote for them (the claims of the intents in force, one
/// `intent_justification` row each, and nothing for the undone link).
fn seeded_before_intent_end(path: &std::path::Path, with_intents: bool) {
    use rastro_core::claims::{Object, claim_id, entity_id, intent_justification_id};
    drop(SqliteStorage::open_with(path, &before_intent_end()).unwrap());
    let db = Connection::open(path).unwrap();
    db.execute_batch(
        "INSERT INTO sources (name,connector,selector,scope,freshness_rule,window_start,keep_original) VALUES ('s', 'github', '{}', 'personal', '1d', '2026-11-01T00:00:00Z', 0);
         INSERT INTO runs (id, source, scope, plugin_id, plugin_version, started_at, finished_at, status) VALUES (1, 's', 'personal', 'fixture', '1', '2026-11-01T00:00:00Z', '2026-11-01T00:00:00Z', 'ok');
         INSERT INTO evidence (id, source, scope, location, field, start, end, quote, captured_at, source_updated_at, trust_class) VALUES (1, 's', 'personal', 'https://example.org/a', '/type', 0, 4, 'User', '2026-11-01T00:00:00.000Z', '2026-11-01T00:00:00.000Z', 'structured');
         INSERT INTO entity (id, identifier) VALUES ('e_a', 'https://example.org/a'), ('e_b', 'https://example.org/b');
         INSERT INTO claim (id, subject, predicate, object_value, object_datatype, scalar, status, revision, known_since) VALUES ('c_evidence', 'e_a', 'rastro:type', 'code:Account', 'rastro:type', 1, 'observed', 'r_1', '2026-11-01T00:00:00.000Z');
         INSERT INTO justification (id, claim_id, kind, extractor, extractor_version, vocabulary_version, derivation_key, extraction_rule, run_id, current) VALUES ('j_evidence', 'c_evidence', 'provenance', 'fixture', '1', '1', 'd', 'rule', 1, 1);
         INSERT INTO justification_span (justification_id, evidence_id, portable_key, field, start, end) VALUES ('j_evidence', 1, 'v_1', '/type', 0, 4);
         INSERT INTO claim (id, subject, predicate, object_value, object_datatype, scalar, status, revision) VALUES ('c_bare', 'e_b', 'rastro:alias', 'b', 'login', 1, 'observed', 'r_2');",
    )
    .unwrap();
    if !with_intents {
        return;
    }
    let intents = upgrade_intents();
    for i in &intents {
        db.execute(
            "INSERT INTO intent (append_seq, id, format_version, kind, owner_id, issuer_context, issued_at, \
             domain_key, targets_json, payload_json, undo_of) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                i.append_seq,
                i.id,
                i.format_version,
                i.kind.as_str(),
                i.owner_id,
                serde_json::to_string(&i.issuer_context).unwrap(),
                i.issued_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                i.domain_key,
                serde_json::to_string(&i.targets).unwrap(),
                serde_json::to_string(&i.payload).unwrap(),
                i.undo_of
            ],
        )
        .unwrap();
    }
    // What main's replay wrote: the Person's type and the standing link, each from its intent.
    let person = entity_id("rastro:person/1");
    let kept = entity_id("github:account/8");
    let typed = claim_id(
        &person,
        "rastro:type",
        &Object::Literal {
            value: "rastro:Person".into(),
            datatype: "rastro:type".into(),
        },
    );
    let held = claim_id(
        &kept,
        "rastro:held_by",
        &Object::Entity {
            id: person.clone(),
            identifier: "rastro:person/1".into(),
            entity_type: "rastro:Person".into(),
        },
    );
    db.execute_batch(&format!(
        "INSERT INTO entity (id, identifier) VALUES ('{person}', 'rastro:person/1'), ('{kept}', 'github:account/8');
         INSERT INTO claim (id, subject, predicate, object_value, object_datatype, scalar, status, revision, known_since) VALUES ('{typed}', '{person}', 'rastro:type', 'rastro:Person', 'rastro:type', 1, 'observed', 'r_t', '2026-11-01T00:01:00.000Z');
         INSERT INTO claim (id, subject, predicate, object_entity, scalar, status, revision, known_since) VALUES ('{held}', '{kept}', 'rastro:held_by', '{person}', 0, 'observed', 'r_h', '2026-11-01T00:05:00.000Z');
         INSERT INTO intent_justification (id, claim_id, intent_id) VALUES ('{}', '{typed}', 'i_1'), ('{}', '{held}', 'i_5');",
        intent_justification_id(&typed, "i_1"),
        intent_justification_id(&held, "i_5"),
    ))
    .unwrap();
}
/// The rows the intents back: their claims and justifications.
fn intent_rows(path: &std::path::Path) -> Vec<String> {
    let db = Connection::open(path).unwrap();
    let mut rows = Vec::new();
    for sql in [
        "SELECT * FROM intent_justification ORDER BY id",
        "SELECT * FROM claim WHERE id IN (SELECT claim_id FROM intent_justification) ORDER BY id",
    ] {
        let mut statement = db.prepare(sql).unwrap();
        let width = statement.column_count();
        rows.extend(
            statement
                .query_map([], |row| {
                    (0..width)
                        .map(|i| {
                            row.get::<_, rusqlite::types::Value>(i)
                                .map(|v| format!("{v:?}"))
                        })
                        .collect::<Result<Vec<_>, _>>()
                        .map(|cells| cells.join("|"))
                })
                .unwrap()
                .map(Result::unwrap),
        );
    }
    rows
}

/// identity/R33: opening a database that holds intents applies the intent-end migration, then replays
/// the intents once, after every migration: a link undone before the upgrade is a superseded
/// claim at once; the justification ids main wrote stay; an evidence claim and a claim no intent
/// ever backed stay; the projection is published once, and a second open publishes nothing. The
/// rows equal those a fresh database writes for the same intents
#[test]
fn the_intent_end_migration_replays_existing_intents() {
    use rastro_core::claims::{entity_id, intent_justification_id};
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("rastro.sqlite3");
    seeded_before_intent_end(&path, true);
    let connection = Connection::open(&path).unwrap();
    let count = |sql: &str| -> i64 { connection.query_row(sql, [], |r| r.get(0)).unwrap() };
    let publications = count("SELECT count(*) FROM projection_publication");
    let ids_before: Vec<String> = connection
        .prepare("SELECT id FROM intent_justification ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    drop(SqliteStorage::open(&path).unwrap());
    let undone: (String, Option<String>, String) = connection
        .query_row(
            "SELECT c.status, ij.ended_at, ij.id FROM claim c JOIN intent_justification ij ON ij.claim_id = c.id \
             WHERE c.predicate = 'rastro:held_by' AND c.subject = ?1",
            [entity_id("github:account/7")],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(undone.0, "superseded");
    assert_eq!(undone.1.as_deref(), Some("2026-11-01T00:03:00.000Z"));
    let ids_after: Vec<String> = connection
        .prepare("SELECT id FROM intent_justification WHERE id <> ?1 ORDER BY id")
        .unwrap()
        .query_map([&undone.2], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(ids_after, ids_before);
    let held = intent_justification_id(
        &connection
            .query_row(
                "SELECT id FROM claim WHERE predicate = 'rastro:held_by' AND status = 'observed'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap(),
        "i_5",
    );
    assert!(ids_after.contains(&held), "{ids_after:?}");
    assert_eq!(
        count("SELECT count(*) FROM claim WHERE id IN ('c_evidence', 'c_bare')"),
        2
    );
    assert_eq!(
        count("SELECT count(*) FROM justification WHERE id = 'j_evidence'"),
        1
    );
    assert_eq!(count("SELECT count(*) FROM intent"), 5);
    assert_eq!(
        count("SELECT count(*) FROM projection_publication"),
        publications + 1
    );
    drop(SqliteStorage::open(&path).unwrap());
    assert_eq!(
        count("SELECT count(*) FROM projection_publication"),
        publications + 1
    );
    // The full chain ends where a fresh database given the same intents does.
    let fresh = home.path().join("fresh.sqlite3");
    let mut storage = SqliteStorage::open(&fresh).unwrap();
    // Fresh intent admission requires the account types that predate these legacy intents.
    let fresh_db = Connection::open(&fresh).unwrap();
    for n in [7, 8] {
        let identifier = format!("github:account/{n}");
        let id = entity_id(&identifier);
        fresh_db
            .execute(
                "INSERT INTO entity (id, identifier) VALUES (?1, ?2)",
                [&id, &identifier],
            )
            .unwrap();
        fresh_db.execute(
            "INSERT INTO claim (id, subject, predicate, object_value, object_datatype, scalar, status, revision, type_ancestors) VALUES (?1, ?2, 'rastro:type', 'code:Account', 'rastro:type', 1, 'observed', 'fixture', '[\"rastro:Account\"]')",
            [format!("account_type_{n}"), id],
        ).unwrap();
    }
    drop(fresh_db);
    storage
        .issue_intents(&[], |_| Ok((upgrade_intents(), ())))
        .unwrap();
    assert_eq!(intent_rows(&path), intent_rows(&fresh));
}

/// identity/R33: a database without intents upgrades without a replay: nothing is published
#[test]
fn the_intent_end_migration_publishes_nothing_without_intents() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("rastro.sqlite3");
    seeded_before_intent_end(&path, false);
    let connection = Connection::open(&path).unwrap();
    let count = |sql: &str| -> i64 { connection.query_row(sql, [], |r| r.get(0)).unwrap() };
    let publications = count("SELECT count(*) FROM projection_publication");
    drop(SqliteStorage::open(&path).unwrap());
    assert_eq!(
        count("SELECT count(*) FROM projection_publication"),
        publications
    );
    assert_eq!(count("SELECT count(*) FROM claim"), 2);
}

/// identity/R33: the replay runs after every migration of the open, so it meets the final schema: a later
/// migration that resets intent-backed projection rows doesn't undo it
#[test]
fn the_intent_replay_runs_after_every_migration() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("rastro.sqlite3");
    seeded_before_intent_end(&path, true);
    let mut migrations = before_intent_end();
    migrations.push((
        "202611081000_intent_justification_end",
        include_str!("../migrations/202611081000_intent_justification_end.sql"),
    ));
    // A later migration that changes the projection: it drops what intents back.
    migrations.push((
        "209901010000_later",
        "DELETE FROM intent_justification; DELETE FROM claim WHERE predicate = 'rastro:held_by';",
    ));
    drop(SqliteStorage::open_with(&path, &migrations).unwrap());
    let connection = Connection::open(&path).unwrap();
    let statuses: Vec<String> = connection
        .prepare("SELECT status FROM claim WHERE predicate = 'rastro:held_by' ORDER BY status")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(statuses, ["observed", "superseded"]);
}

/// labels/S1: a label is read by location and field across sources and scopes, through an index
/// rather than a scan of every observation
#[test]
fn a_label_lookup_by_location_and_field_uses_an_index() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("rastro.sqlite3");
    drop(SqliteStorage::open(&path).unwrap());
    let connection = Connection::open(&path).unwrap();
    let plan: Vec<String> = connection
        .prepare(
            "EXPLAIN QUERY PLAN SELECT id, quote, captured_at, trust_class FROM evidence \
             WHERE location = ?1 AND field = ?2 \
             AND (?3 IS NULL OR scope IN (SELECT value FROM json_each(?3))) \
             ORDER BY captured_at DESC, id DESC LIMIT 1",
        )
        .unwrap()
        .query_map(["https://github.com/o/p/pull/1", "/title", "null"], |row| {
            row.get(3)
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        plan.iter()
            .any(|step| step.contains("SEARCH evidence USING INDEX evidence_by_location")),
        "{plan:?}"
    );
}

/// A conversation, imported through a fake adapter that records whether it was ever called, so a
/// replay or a rebuild can be shown to need no model.
mod recorded {
    use rastro_core::Storage;
    use rastro_core::{
        Locations, Rastro, Scope,
        meeting::{ImportRequest, MeetingCommitments, RunState},
    };
    use rastro_storage_sqlite::SqliteStorage;
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
    use tempfile::TempDir;

    const TRANSCRIPT: &str = "Alice: I'll send Bob the budget by Friday.\n\
Bob: Thanks. I'll review it next week for you if the numbers are final.\n\
Alice: Bob, can you share the roadmap?\n";
    const CANDIDATES: &str = r#"[{"turn":1,"kind":"promise","responsible":"Alice","recipient":"Bob","promise":"send Bob the budget","stated_date":"by Friday","quote":"I'll send Bob the budget by Friday"}]"#;
    const MANIFEST: &str = r#"id = "test-adapter"
name = "Rastro test agent tool adapter"
version = "0.1.0"
author = "Rastro"
kinds = ["agent-tool-adapter"]
protocol = 1
command = "adapter"
agent_tool = { capabilities = ["judge_text"] }

[permissions]
submit = []
scopes = []
env = []
actions = []
"#;

    pub struct Env {
        pub home: TempDir,
    }
    impl Env {
        pub fn new() -> Self {
            let env = Self {
                home: tempfile::tempdir().unwrap(),
            };
            let plugin = env.path("config/plugins/test-adapter");
            fs::create_dir_all(&plugin).unwrap();
            fs::write(
                env.path("config/config.toml"),
                "[commitment_adapter]\nplugin = \"test-adapter\"\n",
            )
            .unwrap();
            fs::write(plugin.join("rastro-plugin.toml"), MANIFEST).unwrap();
            fs::write(env.path("candidates.json"), CANDIDATES).unwrap();
            // Every call of the adapter leaves a line in `called`.
            let script = format!(
                r#"#!/bin/sh
echo called >> "{called}"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"protocol":"1.0","manifest_id":"test-adapter"}}}}\n' "$id"
      ;;
    *'"method":"judgment.request"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"candidates":' "$id"
      cat "{candidates}"
      printf '}}}}\n'
      ;;
    *'"method":"run.finish"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"status":"ok","submitted":1}}}}\n' "$id"
      ;;
  esac
done
"#,
                called = env.path("called").display(),
                candidates = env.path("candidates.json").display(),
            );
            let adapter = plugin.join("adapter");
            fs::write(&adapter, script).unwrap();
            fs::set_permissions(&adapter, fs::Permissions::from_mode(0o755)).unwrap();
            fs::write(env.path("transcript.txt"), TRANSCRIPT).unwrap();
            env
        }
        pub fn path(&self, name: &str) -> PathBuf {
            self.home.path().join(name)
        }
        pub fn database(&self) -> PathBuf {
            self.path("data/rastro.sqlite3")
        }
        pub fn open(&self) -> Rastro<SqliteStorage> {
            fs::create_dir_all(self.path("data")).unwrap();
            Rastro::open(
                Locations {
                    data: self.path("data"),
                    config: self.path("config"),
                },
                SqliteStorage::open(&self.database()).unwrap(),
            )
            .unwrap()
        }
        /// What the database keeps of the latest good run, read through a second connection.
        pub fn recording(&self, meeting: &str) -> Option<rastro_core::meeting::Recording> {
            SqliteStorage::open(&self.database())
                .unwrap()
                .snapshot(|view| view.meeting_recording(meeting))
                .unwrap()
        }
        /// How many times the adapter was started.
        pub fn calls(&self) -> usize {
            fs::read_to_string(self.path("called")).map_or(0, |text| text.lines().count())
        }
        pub fn import(
            &self,
            rastro: &mut Rastro<SqliteStorage>,
            meeting: &str,
        ) -> MeetingCommitments {
            rastro.identity_me().unwrap();
            let imported = rastro
                .import_meeting(&ImportRequest {
                    meeting: meeting.into(),
                    scope: Scope("work".into()),
                    captured_at: "2026-11-10T10:00:00Z".parse().unwrap(),
                    file: self.path("transcript.txt"),
                    owner_speaker: Some("Alice".into()),
                    speakers: Vec::new(),
                    retain_transcript: false,
                })
                .unwrap();
            assert_eq!(
                imported.extraction.state,
                RunState::Succeeded,
                "{imported:?}"
            );
            assert_eq!(imported.hypotheses.len(), 1, "{imported:?}");
            imported
        }
    }
}

/// commitments/S8: a successful run records the interpretation with the spans it cites, and
/// nothing beyond them.
#[test]
fn S8_the_interpretation_is_recorded_with_the_spans_it_cites() {
    let env = recorded::Env::new();
    let mut rastro = env.open();
    env.import(&mut rastro, "weekly-1");
    let recording = env
        .recording("weekly-1")
        .expect("a successful run records its interpretation");
    assert!(
        recording
            .candidates
            .contains("I'll send Bob the budget by Friday")
    );
    let cited: Vec<_> = recording
        .spans
        .iter()
        .filter(|span| !span.text.is_empty())
        .collect();
    assert_eq!(cited.len(), 1, "{:?}", recording.spans);
    assert_eq!(cited[0].turn, 1);
    assert!(cited[0].text.contains("I'll send Bob the budget by Friday"));
    // The turn beside it is kept for its speaker only; turn 3 is neither cited nor beside it.
    assert!(
        recording
            .spans
            .iter()
            .all(|span| span.turn == 1 || span.text.is_empty()),
        "{:?}",
        recording.spans
    );
    assert!(recording.spans.iter().all(|span| span.turn != 3));
}

/// commitments/S8: a replay checks the record again with no model call, and a replay of a record
/// that no longer validates fails and marks the results stale.
#[test]
fn S8_a_replay_checks_the_record_with_no_model_call_and_a_broken_record_fails_stale() {
    use rastro_core::{
        commitments::SourceKind,
        meeting::{Cause, RunState},
    };
    let env = recorded::Env::new();
    let mut rastro = env.open();
    let imported = env.import(&mut rastro, "weekly-1");
    assert_eq!(env.calls(), 1);

    // The adapter is gone: the replay still checks the record.
    fs::remove_file(env.path("config/plugins/test-adapter/adapter")).unwrap();
    let replayed = rastro.replay_commitments(None).unwrap();
    assert_eq!(replayed.sources.len(), 1, "{replayed:?}");
    let source = &replayed.sources[0];
    assert_eq!(
        (source.kind, source.name.as_str()),
        (SourceKind::Meeting, "weekly-1")
    );
    assert_eq!(source.state, RunState::Succeeded, "{source:?}");
    assert!(!source.stale, "{source:?}");
    assert_eq!(env.calls(), 1, "a replay called the adapter");
    let read = rastro.commitments(None, None).unwrap();
    assert_eq!(read.sources[0].hypotheses.len(), 1);
    assert_eq!(
        read.sources[0].extraction.replay_of.as_deref(),
        Some(imported.extraction.run.to_string().as_str())
    );

    // A record that no longer validates fails the replay and leaves the results stale.
    rusqlite::Connection::open(env.database())
        .unwrap()
        .execute(
            "UPDATE meeting_output SET candidates = replace(candidates, 'budget by Friday', 'budget by Never')",
            [],
        )
        .unwrap();
    let failed = rastro.replay_commitments(Some("weekly-1")).unwrap();
    let source = &failed.sources[0];
    assert_eq!(source.state, RunState::Failed, "{source:?}");
    assert_eq!(source.cause, Some(Cause::InvalidQuote), "{source:?}");
    assert!(source.stale, "{source:?}");
    let read = rastro.commitments(None, None).unwrap();
    assert!(read.sources[0].stale);
    assert_eq!(
        read.sources[0].hypotheses.len(),
        1,
        "the last good results stay"
    );
    assert_eq!(env.calls(), 1);

    // A source with nothing recorded can't be replayed by name, and is skipped in a sweep.
    assert!(rastro.replay_commitments(Some("nope")).is_err());
}

/// commitments/S8: a rebuild never calls a model: the claims rebuild leaves the hypotheses and
/// their record as they were, and re-deriving the hypotheses reads the record only.
#[test]
fn S8_a_rebuild_never_calls_a_model() {
    let env = recorded::Env::new();
    let mut rastro = env.open();
    env.import(&mut rastro, "weekly-1");
    let before = rastro.commitments(None, None).unwrap();
    let recorded_before = env.recording("weekly-1").unwrap();
    assert_eq!(env.calls(), 1);

    rastro.rebuild_claims().unwrap();
    let after = rastro.commitments(None, None).unwrap();
    assert_eq!(
        format!("{:?}", after.sources[0].hypotheses),
        format!("{:?}", before.sources[0].hypotheses)
    );
    assert_eq!(
        after.sources[0].extraction.run,
        before.sources[0].extraction.run
    );
    let recorded_after = env.recording("weekly-1").unwrap();
    assert_eq!(recorded_after.candidates, recorded_before.candidates);
    assert_eq!(recorded_after.spans, recorded_before.spans);

    // Re-deriving the hypotheses from the record, with the adapter configured away, needs none.
    fs::write(env.path("config/config.toml"), "").unwrap();
    let mut rastro = env.open();
    rastro.rebuild_claims().unwrap();
    let rebuilt = rastro.replay_commitments(None).unwrap();
    assert_eq!(rebuilt.sources.len(), 1, "{rebuilt:?}");
    assert_eq!(env.calls(), 1, "a rebuild called the adapter");
}

/// commitments/S8: an export leaves the recorded output and the spans out.
#[test]
fn S8_an_export_leaves_the_recorded_output_and_the_spans_out() {
    let env = recorded::Env::new();
    let mut rastro = env.open();
    env.import(&mut rastro, "weekly-1");
    let exported = serde_json::to_value(rastro.export_meeting("weekly-1").unwrap()).unwrap();
    // Only what the owner supplied: no hypothesis, no recorded output, no span.
    let mut keys: Vec<_> = exported.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        ["format", "meeting", "notice", "owner_speaker", "speakers"],
        "{exported}"
    );
    let text = exported.to_string();
    for derived in ["budget", "Friday", "roadmap"] {
        assert!(!text.contains(derived), "{derived} leaked: {text}");
    }
}
