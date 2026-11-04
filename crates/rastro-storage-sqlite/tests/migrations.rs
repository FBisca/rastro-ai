#![allow(non_snake_case)]
use rastro_core::{Scope, Source, Storage};
use rastro_storage_sqlite::SqliteStorage;
use rusqlite::Connection;

#[test]
fn S3a_stored_evidence_rejects_updates_and_deletes() {
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
        assert_eq!(version, 3);
    }
}

#[test]
fn S3b_migrations_apply_once_in_timestamp_order_and_unknown_ones_are_refused() {
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
    assert_eq!(names(&connection).len(), 3);
}

#[test]
fn S3b_legacy_user_version_maps_to_the_migrations_it_meant() {
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
