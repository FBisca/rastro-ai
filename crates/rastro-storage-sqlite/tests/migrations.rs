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
        assert_eq!(version, 8);
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
    assert_eq!(names(&connection).len(), 8);
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
