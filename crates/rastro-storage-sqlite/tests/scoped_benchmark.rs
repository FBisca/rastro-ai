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
    db.execute("INSERT INTO claim SELECT printf('c_%032x',x), printf('e_%032x',x), 'code:in_repository', CASE WHEN x<=20000 THEN ?1 ELSE printf('e_%032x',x) END, NULL, NULL, 0, 'observed', '', '', '[]' FROM numbers", [&hub]).unwrap();
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
    let limit = Duration::from_millis(200);
    for names in [vec!["personal"], vec!["personal", "work"]] {
        let scopes: Vec<_> = names.iter().map(|s| Scope((*s).into())).collect();
        let run = |storage: &mut SqliteStorage| {
            let start = Instant::now();
            let answer = storage
                .read(|view| view.about_scoped(HUB, &AboutOptions::default(), &scopes))
                .unwrap();
            assert_eq!(answer.len(), 21);
            start.elapsed()
        };
        // Cold: a fresh read-only connection per sample, no process-local cache exists.
        let mut cold: Vec<_> = (0..SAMPLES)
            .map(|_| run(&mut SqliteStorage::open_read_only(&path).unwrap().unwrap()))
            .collect();
        // Warm: repeated reads on one connection in the same process.
        let mut storage = SqliteStorage::open_read_only(&path).unwrap().unwrap();
        run(&mut storage);
        let mut warm: Vec<_> = (0..SAMPLES).map(|_| run(&mut storage)).collect();
        let (cold, warm) = (p95(&mut cold), p95(&mut warm));
        println!("scoped about {names:?}: cold p95 {cold:?}, warm p95 {warm:?}");
        assert!(
            cold < limit && warm < limit,
            "scoped read misses p95 < 200 ms"
        );
    }
}

const SAMPLES: usize = 20;

fn p95(samples: &mut [Duration]) -> Duration {
    samples.sort();
    samples[(samples.len() * 95).div_ceil(100) - 1]
}
