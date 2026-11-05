#![allow(non_snake_case)]
use chrono::{DateTime, Utc};
use rastro_core::{Evidence, Scope, Source, Storage, TrustClass};
use rastro_storage_sqlite::SqliteStorage;

fn at(second: u32) -> DateTime<Utc> {
    format!("2026-11-04T00:00:{second:02}Z").parse().unwrap()
}
fn observe(scope: &str, location: &str, field: &str, quote: &str, second: u32) -> Evidence {
    Evidence {
        source: "project".into(),
        scope: Scope(scope.into()),
        location: location.into(),
        field: field.into(),
        start: 0,
        end: quote.len() as u64,
        quote: quote.into(),
        trust_class: TrustClass::Structured,
        captured_at: at(second),
        source_updated_at: None,
        original: None,
    }
}
const PR: &str = "https://github.com/o/p/pull/1";
const OTHER: &str = "https://github.com/o/p/pull/2";

/// The same observations, appended in `order`, into a fresh database, after `before` other
/// observations that shift every id.
fn store(before: usize, order: &[usize]) -> (SqliteStorage, Source, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let mut storage = SqliteStorage::open(&home.path().join("rastro.sqlite3")).unwrap();
    let source = Source::github("project".into(), "o/p".into(), Scope("personal".into()));
    storage
        .project_sources(std::slice::from_ref(&source))
        .unwrap();
    let run = storage
        .start_run(&source, ("c", "1"), at(0), (at(0), at(59)))
        .unwrap();
    let all = [
        observe("personal", PR, "/user/id", "7", 1),
        observe("personal", PR, "/user/type", "User", 2),
        observe("personal", PR, "/user/login", "old", 3),
        observe("personal", PR, "/user/login", "new", 5),
        observe("personal", PR, "/state", "open", 6),
        observe("personal", OTHER, "/user/id", "9", 2),
    ];
    for n in 0..before {
        let shift = observe(
            "personal",
            "https://github.com/o/p/pull/99",
            "/state",
            "open",
            0,
        );
        let shift = Evidence {
            quote: "x".repeat(n + 1),
            end: n as u64 + 1,
            ..shift
        };
        storage.append_evidence(run, &shift).unwrap();
    }
    for index in order {
        storage.append_evidence(run, &all[*index]).unwrap();
    }
    (storage, source, home)
}
fn quotes(found: Vec<(i64, Evidence)>) -> Vec<(String, String, String)> {
    let mut found: Vec<_> = found
        .into_iter()
        .map(|(_, e)| (e.location, e.field, e.quote))
        .collect();
    found.sort();
    found
}

#[test]
fn S15b_a_changed_field_arrives_with_the_companion_fields_no_newer_than_the_high_water_mark() {
    let (storage, source, _home) = store(0, &[0, 1, 2, 3, 4, 5]);
    let at_pr = [PR.to_string()];
    // The batch ends at the new login (second 5): the state at second 6 is newer, the old login
    // is superseded, another location is not asked for.
    let found = storage
        .location_context(&source, &at_pr, (at(5), i64::MAX))
        .unwrap();
    let said: Vec<_> = quotes(found);
    let want = |location: &str, field: &str, quote: &str| {
        (location.to_string(), field.to_string(), quote.to_string())
    };
    assert_eq!(
        said,
        [
            want(PR, "/user/id", "7"),
            want(PR, "/user/login", "new"),
            want(PR, "/user/type", "User"),
        ]
    );
    // Before the new login, the old one is the latest.
    let earlier = storage
        .location_context(&source, &at_pr, (at(4), i64::MAX))
        .unwrap();
    assert!(quotes(earlier).contains(&want(PR, "/user/login", "old")));
}

#[test]
fn S15b_a_rebuild_delivers_the_same_context_whatever_the_ids() {
    let (one, source, _a) = store(0, &[0, 1, 2, 3, 4, 5]);
    // Another database, the same evidence behind other observations: other ids, same set.
    let (two, _, _b) = store(3, &[0, 1, 2, 3, 4, 5]);
    let at_pr = [PR.to_string()];
    let mark = (at(6), i64::MAX);
    let first = one.location_context(&source, &at_pr, mark).unwrap();
    let second = two.location_context(&source, &at_pr, mark).unwrap();
    assert_eq!(quotes(first.clone()), quotes(second));
    assert_eq!(
        quotes(first),
        quotes(one.location_context(&source, &at_pr, mark).unwrap())
    );
}

#[test]
fn S15b_context_never_crosses_scopes() {
    let (mut storage, source, _home) = store(0, &[0, 1]);
    let work = Source::github("project".into(), "o/p".into(), Scope("work".into()));
    storage.project_sources(&[source.clone(), work]).unwrap();
    let run = storage
        .start_run(&source, ("c", "1"), at(0), (at(0), at(59)))
        .unwrap();
    let mut other = observe("work", PR, "/user/login", "secret", 3);
    other.source = "project".into();
    storage.append_evidence(run, &other).unwrap();
    let found = storage
        .location_context(&source, &[PR.to_string()], (at(9), i64::MAX))
        .unwrap();
    assert!(found.iter().all(|(_, e)| e.scope.0 == "personal"));
    assert_eq!(found.len(), 2);
}
