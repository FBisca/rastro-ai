#![allow(non_snake_case)]
use rastro_core::{Scope, Source, Storage};
use rastro_storage_sqlite::SqliteStorage;
use rusqlite::Connection;

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
        assert_eq!(version, 10);
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
    assert_eq!(names(&connection).len(), 10);
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

/// The owner-intent migration adds the insert-only intent table and leaves the projection a
/// database already holds in place: no claim is lost to the upgrade
#[test]
fn S10_the_owner_intent_migration_keeps_the_existing_projection() {
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

/// S36: the migration that records when an intent's contribution ended replays the intents a
/// database already holds, so a link undone before the upgrade is a superseded claim right after
/// it, with no rebuild and no new intent
#[test]
fn S36_the_intent_end_migration_replays_existing_intents() {
    use rastro_core::claims::entity_id;
    use rastro_core::identity::{
        EntityTarget, FORMAT_VERSION, IntentKind, IssuerContext, OWNER, Payload, SHAPE_VERSION,
        Targets,
    };
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("rastro.sqlite3");
    let mut before: Vec<(&str, &str)> = vec![
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
    before.push((
        "202611071000_owner_intents",
        include_str!("../migrations/202611071000_owner_intents.sql"),
    ));
    drop(SqliteStorage::open_with(&path, &before).unwrap());
    let target = |identifier: &str, role: &str| EntityTarget {
        id: entity_id(identifier),
        identifier: identifier.into(),
        role: role.into(),
    };
    let (person, account) = (
        target("rastro:person/1", "person"),
        target("github:account/7", "account"),
    );
    let link = Targets::Pair {
        version: SHAPE_VERSION,
        symmetric: false,
        endpoints: [account.clone(), person.clone()],
    };
    let mut refusal = [account.clone(), person.clone()];
    refusal.sort();
    let link_domain = format!("held_by/{}/{}", account.id, person.id);
    let rows = [
        (
            "i_1",
            IntentKind::CreatePerson,
            Targets::Entity {
                version: SHAPE_VERSION,
                entity: person.clone(),
            },
            format!("person/{}", person.id),
            None,
            "2026-11-01T00:00:00.000Z",
        ),
        (
            "i_2",
            IntentKind::Link,
            link.clone(),
            link_domain.clone(),
            None,
            "2026-11-01T00:01:00.000Z",
        ),
        (
            "i_3",
            IntentKind::Undo,
            link,
            link_domain,
            Some("i_2"),
            "2026-11-01T00:02:00.000Z",
        ),
        (
            "i_4",
            IntentKind::CannotLink,
            Targets::Pair {
                version: SHAPE_VERSION,
                symmetric: true,
                endpoints: refusal.clone(),
            },
            format!("cannot_link/{}/{}", refusal[0].id, refusal[1].id),
            None,
            "2026-11-01T00:02:00.000Z",
        ),
    ];
    let connection = Connection::open(&path).unwrap();
    let issuer = serde_json::to_string(&IssuerContext::owner_cli(&[])).unwrap();
    let payload = serde_json::to_string(&Payload {
        version: SHAPE_VERSION,
        name: None,
    })
    .unwrap();
    for (id, kind, targets, domain, undo_of, at) in rows {
        connection
            .execute(
                "INSERT INTO intent (id, format_version, kind, owner_id, issuer_context, issued_at, \
                 domain_key, targets_json, payload_json, undo_of) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    id,
                    FORMAT_VERSION,
                    kind.as_str(),
                    OWNER,
                    issuer,
                    at,
                    domain,
                    serde_json::to_string(&targets).unwrap(),
                    payload,
                    undo_of
                ],
            )
            .unwrap();
    }
    let count = |sql: &str| -> i64 { connection.query_row(sql, [], |r| r.get(0)).unwrap() };
    let publications = count("SELECT count(*) FROM projection_publication");
    drop(SqliteStorage::open(&path).unwrap());
    let link: (String, Option<String>) = connection
        .query_row(
            "SELECT c.status, ij.ended_at FROM claim c JOIN intent_justification ij ON ij.claim_id = c.id \
             WHERE c.predicate = 'rastro:held_by'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        link,
        ("superseded".into(), Some("2026-11-01T00:02:00.000Z".into()))
    );
    assert_eq!(count("SELECT count(*) FROM intent"), 4);
    assert_eq!(
        count("SELECT count(*) FROM projection_publication"),
        publications + 1
    );
}
