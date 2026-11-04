use rastro_core::{Scope, Source, Storage};
use rastro_storage_sqlite::SqliteStorage;
use rusqlite::Connection;

#[test]
fn migrations_preserve_evidence_and_reject_updates_and_deletes() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("rastro.sqlite3");
    let mut storage = SqliteStorage::open(&path).unwrap();
    storage
        .project_sources(&[Source::github(
            "project".into(),
            "o/p".into(),
            Scope("personal".into()),
        )])
        .unwrap();
    let connection = Connection::open(&path).unwrap();
    connection.execute("INSERT INTO evidence (source, scope, location, field, start, end, quote, captured_at, trust_class) VALUES ('project', 'personal', 'https://example.org', '/body', 0, 2, 'é', '2026-11-04T00:00:00Z', 'untrusted content')", []).unwrap();
    assert!(
        connection
            .execute("UPDATE evidence SET quote='ab'", [])
            .is_err()
    );
    assert!(connection.execute("DELETE FROM evidence", []).is_err());
    drop(storage);
    SqliteStorage::open(&path).unwrap();
    let quote: String = connection
        .query_row("SELECT quote FROM evidence", [], |row| row.get(0))
        .unwrap();
    assert_eq!(quote, "é");
    let version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 1);
}
