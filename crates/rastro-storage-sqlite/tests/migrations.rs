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
                .execute_batch(include_str!("../migrations/0001.sql"))
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
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);
    }
}
