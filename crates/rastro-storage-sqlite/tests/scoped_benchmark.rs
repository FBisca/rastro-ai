#![allow(non_snake_case)]
use rastro_core::{ReadStorage, Scope, knowledge::AboutOptions};
use rastro_storage_sqlite::SqliteStorage;
use std::time::{Duration, Instant};

const HUB: &str = "https://example.org/hub";

fn fixture(path: &std::path::Path) {
    drop(SqliteStorage::open(path).unwrap());
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute_batch("BEGIN;
        INSERT INTO sources (name,connector,selector,scope,freshness_rule,window_start,keep_original) VALUES ('personal', 'scripted', '{}', 'personal', '1d', '2026-11-01T00:00:00Z', 0);
        INSERT INTO sources (name,connector,selector,scope,freshness_rule,window_start,keep_original) VALUES ('work', 'scripted', '{}', 'work', '1d', '2026-11-01T00:00:00Z', 0);
        INSERT INTO sources (name,connector,selector,scope,freshness_rule,window_start,keep_original) VALUES ('health', 'scripted', '{}', 'health', '1d', '2026-11-01T00:00:00Z', 0);
        INSERT INTO runs (id, source, scope, plugin_id, plugin_version, started_at, finished_at, status) VALUES (1, 'personal', 'personal', 'fixture', '1', '2026-11-01T00:00:00Z', '2026-11-01T00:00:00Z', 'ok');
        CREATE TEMP TABLE numbers AS WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100000) SELECT x FROM n;
        INSERT INTO entity SELECT printf('e_%032x', x), 'https://example.org/item/' || x FROM numbers;
        INSERT INTO evidence (id, source, scope, location, field, start, end, quote, captured_at, source_updated_at, trust_class)
        SELECT x, CASE x%3 WHEN 0 THEN 'personal' WHEN 1 THEN 'work' ELSE 'health' END,
            CASE x%3 WHEN 0 THEN 'personal' WHEN 1 THEN 'work' ELSE 'health' END,
            'https://example.org/item/' || x, '/url', 0, 1, 'x', '2026-11-01T00:00:00Z', '2026-11-01T00:00:00Z', 'structured' FROM numbers;
        COMMIT;") .unwrap();
    let hub = rastro_core::claims::entity_id(HUB);
    db.execute("INSERT INTO entity VALUES (?1, ?2)", [&hub, HUB])
        .unwrap();
    db.execute("INSERT INTO claim SELECT printf('c_%032x',x), printf('e_%032x',x), 'code:in_repository', CASE WHEN x<=20000 THEN ?1 ELSE printf('e_%032x',x) END, NULL, NULL, 0, 'observed', '', '' FROM numbers", [&hub]).unwrap();
    db.execute_batch("        INSERT INTO justification SELECT printf('j_%032x',x), printf('c_%032x',x), 'provenance', 'fixture', '1', '1', printf('d_%032x',x), 'rule', 1, 1 FROM numbers;
        INSERT INTO justification_span SELECT printf('j_%032x',x), x, printf('v_%032x',x), '/url', 0, 1 FROM numbers;
ANALYZE; PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM claim", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        100000
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM claim WHERE object_entity = ?1",
            [&hub],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        20000
    );
}

#[test]
#[ignore = "release benchmark: cargo test --release -p rastro-storage-sqlite --test scoped_benchmark -- --ignored --nocapture"]
fn S28b_scoped_about_hub_cold_and_warm_p95_under_200ms() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("bench.sqlite3");
    fixture(&path);
    for names in [vec!["personal"], vec!["personal", "work"]] {
        let scopes: Vec<_> = names.iter().map(|s| Scope((*s).into())).collect();
        let mut storage = SqliteStorage::open_read_only(&path).unwrap().unwrap();
        let start = Instant::now();
        let answer = storage
            .read(|view| view.about_scoped(HUB, &AboutOptions::default(), &scopes))
            .unwrap();
        let elapsed = start.elapsed();
        assert_eq!(answer.len(), 21);
        println!("uncached scoped about {names:?}: {elapsed:?}");
        assert!(
            elapsed < Duration::from_millis(200),
            "scoped replay needs the ADR cache fallback"
        );
    }
}
