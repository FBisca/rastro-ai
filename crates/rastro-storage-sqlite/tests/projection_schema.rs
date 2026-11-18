//! A claims projection rebuild builds its shadow tables with the active projection's column
//! shapes, so publishing them replaces one shape with the same one.

use rastro_core::Storage;
use rastro_storage_sqlite::SqliteStorage;
use rusqlite::params;

/// claims/R29: shadow tables match the active projection column shapes
#[test]
fn shadow_tables_match_the_active_projection_column_shapes() {
    let home = tempfile::tempdir().unwrap();
    let mut storage = SqliteStorage::open(&home.path().join("db")).unwrap();
    storage.begin_claims_rebuild().unwrap();
    let db = storage.connection();
    let tables = db
        .prepare("SELECT name FROM sqlite_temp_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(tables.len(), 9);
    for table in tables {
        assert!(
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [&table],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
        let columns = |schema: &str| {
            db.prepare("SELECT name, type, \"notnull\", dflt_value, pk, hidden FROM pragma_table_xinfo(?1, ?2) ORDER BY cid").unwrap()
                .query_map(params![table, schema], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, i64>(4)?, r.get::<_, i64>(5)?))).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
        };
        assert_eq!(columns("main"), columns("temp"), "{table}");
    }
    storage.finish_claims_rebuild(false).unwrap();
}
