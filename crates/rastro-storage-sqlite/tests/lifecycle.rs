#![allow(non_snake_case)]
use chrono::{DateTime, Utc};
use rastro_core::claims::{self, Entity, Object, ObservedClaim, Support};
use rastro_core::{Evidence, Scope, Source, Storage, TrustClass};
use rastro_storage_sqlite::SqliteStorage;
use std::collections::BTreeSet;

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
        .find(|(_, item)| item.quote == e.quote)
        .unwrap()
        .0;
    let claim = accepted(id, e, predicate, value);
    let result = claim.id.clone();
    db.record_claims(run, ("extractor", "1"), &[claim], None)
        .unwrap();
    result
}
#[test]
fn S9a_equal_capture_recurrence_keeps_separate_ordered_occurrences() {
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
    let why = db.why(&open).unwrap().unwrap();
    assert_eq!(why.claim.status, "observed");
    assert_eq!(why.claim.known_intervals.len(), 2);
    assert_eq!(why.claim.known_intervals[0].end, Some(time()));
    assert_eq!(why.claim.known_intervals[1].start, time());
    let why = db.why(&closed_id).unwrap().unwrap();
    assert_eq!(why.claim.known_intervals.len(), 1);
    assert_eq!(why.claim.known_intervals[0].end, Some(time()));
}
#[test]
fn S11a_one_span_can_justify_two_conclusions_without_collision() {
    let home = tempfile::tempdir().unwrap();
    let mut db = SqliteStorage::open(&home.path().join("db")).unwrap();
    let e = evidence("source", "open");
    let a = store(&mut db, &e, "code:state", "open");
    let b = store(&mut db, &e, "rastro:type", "code:PullRequest");
    for id in [a, b] {
        let why = db.why(&id).unwrap().unwrap();
        assert_eq!(why.claim.status, "observed");
        assert_eq!(why.supports.len(), 1);
    }
}

#[test]
fn S8a_fresh_databases_with_other_local_ids_keep_revisions_and_ids() {
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
    let a = a.why(&first).unwrap().unwrap();
    let b = b.why(&replay).unwrap().unwrap();
    assert_ne!(a.supports[0].evidence_id, b.supports[0].evidence_id);
    assert_eq!(a.claim.revision, b.claim.revision);
}

#[test]
fn S4b_all_required_spans_of_one_evidence_are_retained_as_an_and_bundle() {
    let home = tempfile::tempdir().unwrap();
    let mut db = SqliteStorage::open(&home.path().join("db")).unwrap();
    let e = evidence("source", "open");
    let first = store(&mut db, &e, "code:state", "open");
    let why = db.why(&first).unwrap().unwrap();
    let mut bundle = accepted(why.supports[0].evidence_id, &e, "code:state", "other");
    let mut second = bundle.supports.first().unwrap().clone();
    second.start = 1;
    second.end = 3;
    bundle.supports.insert(second);
    let id = bundle.id.clone();
    db.record_claims(1, ("extractor", "1"), &[bundle], None)
        .unwrap();
    let why = db.why(&id).unwrap().unwrap();
    assert_eq!(why.supports.len(), 2);
    assert_eq!(
        why.supports[0].justification_id,
        why.supports[1].justification_id
    );
}

#[test]
fn S16b_equal_time_late_append_does_not_regress_the_portable_high_water() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("db");
    let mut db = SqliteStorage::open(&path).unwrap();
    let source = Source::github("source".into(), "o/p".into(), Scope("work".into()));
    db.project_sources(std::slice::from_ref(&source)).unwrap();
    let run = db
        .start_extraction(&source, ("extractor", "1"), time())
        .unwrap();
    for key in ["v_z", "v_a"] {
        let checkpoint = claims::Checkpoint {
            extractor: "extractor".into(),
            extractor_version: "1".into(),
            source: "source".into(),
            scope: source.scope.clone(),
            last: (time(), 1),
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
    assert_eq!(key, "v_z");
}
