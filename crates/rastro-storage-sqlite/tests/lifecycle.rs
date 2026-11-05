#![allow(non_snake_case)]
use chrono::{DateTime, Utc};
use rastro_core::claims::{self, Entity, Object, ObservedClaim, Support};
use rastro_core::knowledge::{Explanation, WhyOptions};
use rastro_core::{Evidence, Scope, Source, Storage, TrustClass};
use rastro_storage_sqlite::SqliteStorage;
use std::collections::BTreeSet;

/// Every justification of a claim, uncapped and with full citations.
fn explained(db: &SqliteStorage, claim: &str) -> Explanation {
    let explained = db
        .snapshot(|view| {
            view.explain(
                claim,
                &WhyOptions {
                    limit: 10_000,
                    full: true,
                },
            )
        })
        .unwrap()
        .unwrap();
    assert!(
        !explained.truncated,
        "the explanation must list every support"
    );
    explained
}
fn time() -> DateTime<Utc> {
    "2026-11-01T00:00:00Z".parse().unwrap()
}
fn evidence(source: &str, value: &str) -> Evidence {
    Evidence {
        source: source.into(),
        scope: Scope("work".into()),
        location: "https://example.org/item".into(),
        field: "/state".into(),
        start: 0,
        end: value.len() as u64,
        quote: value.into(),
        trust_class: TrustClass::Structured,
        captured_at: time(),
        source_updated_at: Some(time()),
        original: None,
    }
}
fn accepted(id: i64, e: &Evidence, predicate: &str, value: &str) -> ObservedClaim {
    let subject = Entity::new(e.location.clone(), "code:PullRequest".into());
    let object = Object::Literal {
        value: value.into(),
        datatype: "string".into(),
    };
    ObservedClaim {
        type_ancestors: Vec::new(),
        id: claims::claim_id(&subject.id, predicate, &object),
        subject,
        predicate: predicate.into(),
        object,
        scalar: true,
        supports: BTreeSet::from([Support {
            portable_key: claims::portable_evidence_key(e),
            evidence_id: id,
            field: e.field.clone(),
            start: e.start,
            end: e.end,
            extraction_rule: "one-rule".into(),
        }]),
    }
}
fn store(db: &mut SqliteStorage, e: &Evidence, predicate: &str, value: &str) -> String {
    let mut source = Source::github(e.source.clone(), "o/p".into(), e.scope.clone());
    source.connector = "scripted".into();
    db.project_sources(std::slice::from_ref(&source)).unwrap();
    let run = db
        .start_run(&source, ("scripted", "1"), time(), (time(), time()))
        .unwrap();
    db.append_evidence(run, e).unwrap();
    let pending = db.pending_evidence(("extractor", "1"), &source).unwrap();
    let id = pending
        .iter()
        .rev()
        .find(|(_, item)| item.quote == e.quote)
        .unwrap()
        .0;
    let claim = accepted(id, e, predicate, value);
    let result = claim.id.clone();
    db.record_claims(run, ("extractor", "1"), &[claim], None)
        .unwrap();
    result
}
/// claims/R12: equal capture recurrence keeps separate ordered occurrences
#[test]
fn equal_capture_recurrence_keeps_separate_ordered_occurrences() {
    let home = tempfile::tempdir().unwrap();
    let mut db = SqliteStorage::open(&home.path().join("db")).unwrap();
    let mut opens: Vec<_> = (0..100)
        .map(|n| evidence(&format!("source{n}"), "open"))
        .collect();
    opens.sort_by_key(claims::portable_evidence_key);
    let closed = evidence("closed-source", "closed");
    let middle = claims::portable_evidence_key(&closed);
    let mut first = opens
        .iter()
        .find(|e| claims::portable_evidence_key(e) < middle)
        .unwrap()
        .clone();
    let mut last = opens
        .iter()
        .rev()
        .find(|e| claims::portable_evidence_key(e) > middle)
        .unwrap()
        .clone();
    first.source_updated_at = Some(time() - chrono::Duration::days(1));
    last.source_updated_at = Some(time() + chrono::Duration::days(1));
    let open = store(&mut db, &first, "code:state", "open");
    let closed_id = store(&mut db, &closed, "code:state", "closed");
    store(&mut db, &last, "code:state", "open");
    let why = explained(&db, &open);
    assert_eq!(why.claim.claim.status, "observed");
    assert_eq!(why.claim.claim.known_intervals.len(), 2);
    assert_eq!(why.claim.claim.known_intervals[0].end, Some(time()));
    assert_eq!(why.claim.claim.known_intervals[1].start, time());
    let why = explained(&db, &closed_id);
    assert_eq!(why.claim.claim.known_intervals.len(), 1);
    assert_eq!(why.claim.claim.known_intervals[0].end, Some(time()));
}
/// claims/R15: one span can justify two conclusions without collision
#[test]
fn one_span_can_justify_two_conclusions_without_collision() {
    let home = tempfile::tempdir().unwrap();
    let mut db = SqliteStorage::open(&home.path().join("db")).unwrap();
    let e = evidence("source", "open");
    let a = store(&mut db, &e, "code:state", "open");
    let b = store(&mut db, &e, "rastro:type", "code:PullRequest");
    for id in [a, b] {
        let why = explained(&db, &id);
        assert_eq!(why.claim.claim.status, "observed");
        assert_eq!(why.supports.len(), 1);
    }
}

/// claims/R10: fresh databases with other local ids keep revisions and ids
#[test]
fn fresh_databases_with_other_local_ids_keep_revisions_and_ids() {
    let home = tempfile::tempdir().unwrap();
    let mut a = SqliteStorage::open(&home.path().join("a")).unwrap();
    let mut b = SqliteStorage::open(&home.path().join("b")).unwrap();
    let mut filler = evidence("filler", "closed");
    filler.location = "https://example.org/unrelated".into();
    store(&mut b, &filler, "code:state", "closed");
    let e = evidence("source", "open");
    let first = store(&mut a, &e, "code:state", "open");
    let replay = store(&mut b, &e, "code:state", "open");
    assert_eq!(first, replay);
    let initial_a = explained(&a, &first);
    let initial_b = explained(&b, &replay);
    assert_ne!(
        initial_a.supports[0].spans[0].evidence_id,
        initial_b.supports[0].spans[0].evidence_id
    );
    assert_eq!(
        initial_a.claim.claim.revision,
        initial_b.claim.claim.revision
    );
    // The portable recurrence ordinal also survives a local-id remap.
    for db in [&mut a, &mut b] {
        store(db, &evidence("source", "closed"), "code:state", "closed");
        store(db, &e, "code:state", "open");
    }
    let final_a = explained(&a, &first);
    let final_b = explained(&b, &replay);
    assert_eq!(
        serde_json::to_value(final_a.claim).unwrap(),
        serde_json::to_value(final_b.claim).unwrap()
    );
    let occurrences = |name| {
        let connection = rusqlite::Connection::open(home.path().join(name)).unwrap();
        connection.prepare("SELECT o.id FROM occurrence o JOIN justification j ON j.id = o.justification_id WHERE j.claim_id = ?1 ORDER BY o.captured_at, o.evidence_id").unwrap()
            .query_map([&first], |r| r.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
    };
    assert_eq!(occurrences("a").len(), 2);
    assert_eq!(occurrences("a"), occurrences("b"));
}

/// claims/R4: all required spans of one evidence are retained as an and bundle
#[test]
fn all_required_spans_of_one_evidence_are_retained_as_an_and_bundle() {
    let home = tempfile::tempdir().unwrap();
    let mut db = SqliteStorage::open(&home.path().join("db")).unwrap();
    let e = evidence("source", "open");
    let first = store(&mut db, &e, "code:state", "open");
    let why = explained(&db, &first);
    let mut bundle = accepted(
        why.supports[0].spans[0].evidence_id,
        &e,
        "code:state",
        "other",
    );
    let mut second = bundle.supports.first().unwrap().clone();
    second.start = 1;
    second.end = 3;
    bundle.supports.insert(second);
    let id = bundle.id.clone();
    db.record_claims(1, ("extractor", "1"), &[bundle], None)
        .unwrap();
    let why = explained(&db, &id);
    assert_eq!(why.supports.len(), 1);
    assert_eq!(why.supports[0].spans.len(), 2);
}

/// claims/R1: late append advances boundary even when its portable key sorts before previous
#[test]
fn late_append_advances_boundary_even_when_its_portable_key_sorts_before_previous() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("db");
    let mut db = SqliteStorage::open(&path).unwrap();
    let source = Source::github("source".into(), "o/p".into(), Scope("work".into()));
    db.project_sources(std::slice::from_ref(&source)).unwrap();
    let run = db
        .start_extraction(&source, ("extractor", "1"), time())
        .unwrap();
    for (id, key) in [(1, "v_z"), (2, "v_a")] {
        let checkpoint = claims::Checkpoint {
            extractor: "extractor".into(),
            extractor_version: "1".into(),
            source: "source".into(),
            scope: source.scope.clone(),
            last: (time(), id),
            last_portable_key: key.into(),
            vocabulary_version: "1".into(),
        };
        db.record_claims(run, ("extractor", "1"), &[], Some(&checkpoint))
            .unwrap();
    }
    let db = rusqlite::Connection::open(path).unwrap();
    let key: String = db
        .query_row(
            "SELECT last_portable_key FROM extraction_checkpoint",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(key, "v_a");
}

/// claims/R12: same pair recurrence at equal capture stays separate
#[test]
fn same_pair_recurrence_at_equal_capture_stays_separate() {
    let home = tempfile::tempdir().unwrap();
    let mut db = SqliteStorage::open(&home.path().join("db")).unwrap();
    let e = evidence("source", "open");
    let open = store(&mut db, &e, "code:state", "open");
    let closed = store(
        &mut db,
        &evidence("source", "closed"),
        "code:state",
        "closed",
    );
    store(&mut db, &e, "code:state", "open");
    let why = explained(&db, &open);
    assert_eq!(why.claim.claim.status, "observed");
    assert_eq!(why.claim.claim.known_intervals.len(), 2);
    assert_eq!(why.claim.claim.known_intervals[0].end, Some(time()));
    assert_eq!(why.claim.claim.known_intervals[1].start, time());
    let why = explained(&db, &closed);
    assert_eq!(why.claim.claim.status, "superseded");
    assert_eq!(why.claim.claim.known_intervals.len(), 1);
    assert_eq!(why.claim.claim.known_intervals[0].end, Some(time()));
    let before = serde_json::to_value(explained(&db, &open)).unwrap();
    db.begin_claims_rebuild().unwrap();
    let source = Source::github("source".into(), "o/p".into(), Scope("work".into()));
    let pending = db.pending_evidence(("extractor", "1"), &source).unwrap();
    let claims: Vec<_> = pending
        .iter()
        .map(|(id, e)| accepted(*id, e, "code:state", &e.quote))
        .collect();
    db.record_claims(1, ("extractor", "1"), &claims, None)
        .unwrap();
    db.finish_claims_rebuild(true).unwrap();
    assert_eq!(before, serde_json::to_value(explained(&db, &open)).unwrap());
}

/// claims/R22: vocabulary and run metadata match replay without extractor bump
#[test]
fn vocabulary_and_run_metadata_match_replay_without_extractor_bump() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("db");
    let mut db = SqliteStorage::open(&path).unwrap();
    let e = evidence("source", "open");
    let id = store(&mut db, &e, "code:state", "open");
    let why = explained(&db, &id);
    let claim = accepted(
        why.supports[0].spans[0].evidence_id,
        &e,
        "code:state",
        "open",
    );
    let source = Source::github("source".into(), "o/p".into(), e.scope.clone());
    let run = db
        .start_extraction(&source, ("extractor", "1"), time())
        .unwrap();
    let checkpoint = claims::Checkpoint {
        extractor: "extractor".into(),
        extractor_version: "1".into(),
        source: "source".into(),
        scope: e.scope.clone(),
        last: (time(), why.supports[0].spans[0].evidence_id),
        last_portable_key: claims::portable_evidence_key(&e),
        vocabulary_version: "2".into(),
    };
    db.record_claims(
        run,
        ("extractor", "1"),
        std::slice::from_ref(&claim),
        Some(&checkpoint),
    )
    .unwrap();
    let incremental = serde_json::to_value(explained(&db, &id)).unwrap();
    assert_eq!(incremental["supports"][0]["vocabulary_version"], "2");
    let connection = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT run_id FROM justification", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        run
    );
    db.begin_claims_rebuild().unwrap();
    db.record_claims(run, ("extractor", "1"), &[claim], Some(&checkpoint))
        .unwrap();
    db.finish_claims_rebuild(true).unwrap();
    assert_eq!(
        incremental,
        serde_json::to_value(explained(&db, &id)).unwrap()
    );
}

/// claims/R16: corroboration keeps the start of the claim union interval
#[test]
fn corroboration_keeps_the_start_of_the_claim_union_interval() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("db");
    let mut db = SqliteStorage::open(&path).unwrap();
    let first = evidence("first", "open");
    let id = store(&mut db, &first, "code:state", "open");
    let mut later = evidence("later", "open");
    later.captured_at += chrono::Duration::days(1);
    store(&mut db, &later, "code:state", "open");
    let why = explained(&db, &id);
    assert_eq!(why.claim.claim.known_intervals.len(), 1);
    let connection = rusqlite::Connection::open(path).unwrap();
    let since: String = connection
        .query_row("SELECT known_since FROM claim WHERE id = ?1", [&id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(since.parse::<DateTime<Utc>>().unwrap(), first.captured_at);
}

/// claims/R14: related types do not conflict but unrelated types and replacement do
#[test]
fn related_types_do_not_conflict_but_unrelated_types_and_replacement_do() {
    let home = tempfile::tempdir().unwrap();
    let mut db = SqliteStorage::open(&home.path().join("db")).unwrap();
    let account = store(
        &mut db,
        &evidence("a", "Account"),
        "rastro:type",
        "code:Account",
    );
    let e = evidence("b", "User");
    let user = store(&mut db, &e, "rastro:type", "code:User");
    let row = explained(&db, &user);
    let mut typed = accepted(
        row.supports[0].spans[0].evidence_id,
        &e,
        "rastro:type",
        "code:User",
    );
    typed.type_ancestors = vec!["code:Account".into()];
    db.record_claims(1, ("extractor", "1"), &[typed], None)
        .unwrap();
    assert!(explained(&db, &user).claim.claim.conflicts_with.is_empty());
    assert!(
        explained(&db, &account)
            .claim
            .claim
            .conflicts_with
            .is_empty()
    );
    let bot = store(&mut db, &evidence("c", "Bot"), "rastro:type", "code:Bot");
    assert!(
        explained(&db, &user)
            .claim
            .claim
            .conflicts_with
            .contains(&bot)
    );
    assert!(
        explained(&db, &bot)
            .claim
            .claim
            .conflicts_with
            .contains(&user)
    );
    store(&mut db, &evidence("b", "Bot"), "rastro:type", "code:Bot");
    let old = explained(&db, &user);
    assert_eq!(old.claim.claim.status, "superseded");
    assert!(old.claim.claim.superseded_by.contains(&bot));
}

/// S12: a claim justified by a carried-over pair and a rebuilt pair equals the incremental one,
/// whichever is staged first, and the carried pair keeps its checkpoint
#[test]
fn S12_a_claim_justified_by_a_carried_pair_and_a_rebuilt_pair_equals_the_incremental_one() {
    for carry_first in [true, false] {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("db");
        let mut db = SqliteStorage::open(&path).unwrap();
        let carried_evidence = evidence("carried", "open");
        let rebuilt_evidence = evidence("rebuilt", "open");
        let open = store(&mut db, &carried_evidence, "code:state", "open");
        store(
            &mut db,
            &evidence("carried", "closed"),
            "code:state",
            "closed",
        );
        store(&mut db, &rebuilt_evidence, "code:state", "open");
        let carried = Source::github("carried".into(), "o/p".into(), Scope("work".into()));
        let rebuilt = Source::github("rebuilt".into(), "o/p".into(), Scope("work".into()));
        let checkpoint = |source: &Source, e: &Evidence| {
            let pending = db.pending_evidence(("extractor", "1"), source).unwrap();
            claims::Checkpoint {
                extractor: "extractor".into(),
                extractor_version: "1".into(),
                source: source.name.clone(),
                scope: source.scope.clone(),
                last: (time(), pending.iter().map(|(id, _)| *id).max().unwrap()),
                last_portable_key: claims::portable_evidence_key(e),
                // The vocabulary the stored justifications were recorded under, so none retires.
                vocabulary_version: String::new(),
            }
        };
        let carried_checkpoint = checkpoint(&carried, &carried_evidence);
        let rebuilt_checkpoint = checkpoint(&rebuilt, &rebuilt_evidence);
        let run = db
            .start_extraction(&carried, ("extractor", "1"), time())
            .unwrap();
        db.record_claims(run, ("extractor", "1"), &[], Some(&carried_checkpoint))
            .unwrap();
        let incremental = serde_json::to_value(explained(&db, &open)).unwrap();
        assert_eq!(incremental["supports"].as_array().unwrap().len(), 2);
        let checkpoints = || -> Vec<String> {
            let connection = rusqlite::Connection::open(&path).unwrap();
            let mut statement = connection
                .prepare("SELECT source || last_evidence_id FROM extraction_checkpoint ORDER BY 1")
                .unwrap();
            statement
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        let carried_before = checkpoints();
        db.begin_claims_rebuild().unwrap();
        let rebuild = |db: &mut SqliteStorage| {
            let run = db
                .start_extraction(&rebuilt, ("extractor", "1"), time())
                .unwrap();
            let pending = db.pending_evidence(("extractor", "1"), &rebuilt).unwrap();
            let claims: Vec<_> = pending
                .iter()
                .map(|(id, e)| accepted(*id, e, "code:state", &e.quote))
                .collect();
            db.record_claims(run, ("extractor", "1"), &claims, Some(&rebuilt_checkpoint))
                .unwrap();
        };
        if carry_first {
            assert_eq!(db.carry_over_pair(&carried, "extractor").unwrap(), 2);
            rebuild(&mut db);
        } else {
            rebuild(&mut db);
            assert_eq!(db.carry_over_pair(&carried, "extractor").unwrap(), 2);
        }
        db.finish_claims_rebuild(true).unwrap();
        assert_eq!(
            incremental,
            serde_json::to_value(explained(&db, &open)).unwrap(),
            "carry first: {carry_first}"
        );
        assert!(
            checkpoints()
                .iter()
                .any(|c| carried_before.contains(c) && c.starts_with("carried")),
            "the carried pair keeps its checkpoint"
        );
    }
}
