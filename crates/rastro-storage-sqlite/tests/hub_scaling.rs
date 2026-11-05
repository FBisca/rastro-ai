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
/// claims/R22: incremental record on a hub is near linear and equals the full replay
#[test]
fn incremental_record_on_a_hub_is_near_linear_and_equals_the_full_replay() {
    let n: usize = 4000;
    let scalar = true;
    let home = tempfile::tempdir().unwrap();
    let mut db = SqliteStorage::open(&home.path().join("db")).unwrap();
    let mut source = Source::github("s".into(), "o/p".into(), Scope("work".into()));
    source.connector = "scripted".into();
    db.project_sources(std::slice::from_ref(&source)).unwrap();
    let run = db
        .start_run(&source, ("scripted", "1"), time(), (time(), time()))
        .unwrap();
    for i in 0..n {
        let e = Evidence {
            source: "s".into(),
            scope: Scope("work".into()),
            location: format!("https://x/{i}"),
            field: "/user/type".into(),
            start: 0,
            end: 4,
            quote: "User".into(),
            trust_class: TrustClass::Structured,
            captured_at: time() + chrono::Duration::seconds(i as i64),
            source_updated_at: Some(time()),
            original: None,
        };
        db.append_evidence(run, &e).unwrap();
    }
    let pending = db.pending_evidence(("x", "1"), &source).unwrap();
    let subject = Entity::new("github:account/1".into(), "code:Account".into());
    let object = Object::Literal {
        value: "code:Account".into(),
        datatype: "string".into(),
    };
    let id = claims::claim_id(&subject.id, "rastro:type", &object);
    let mut claims: Vec<_> = pending
        .iter()
        .map(|(eid, e)| ObservedClaim {
            type_ancestors: Vec::new(),
            scalar,
            id: id.clone(),
            subject: subject.clone(),
            predicate: "rastro:type".into(),
            object: object.clone(),
            supports: BTreeSet::from([Support {
                portable_key: claims::portable_evidence_key(e),
                evidence_id: *eid,
                field: e.field.clone(),
                start: 0,
                end: 4,
                extraction_rule: "r".into(),
            }]),
        })
        .collect();
    let t = std::time::Instant::now();
    db.record_claims(run, ("x", "1"), &claims, None).unwrap();
    eprintln!("N={n} scalar={scalar} record_claims: {:?}", t.elapsed());
    let mut observation = pending.last().unwrap().1.clone();
    observation.location = format!("https://x/{n}");
    observation.captured_at += chrono::Duration::seconds(1);
    let eid = match db.append_evidence(run, &observation).unwrap() {
        rastro_core::Appended::New(id) => id,
        _ => panic!("new location must append"),
    };
    let mut extra = claims[0].clone();
    extra.supports = BTreeSet::from([Support {
        portable_key: claims::portable_evidence_key(&observation),
        evidence_id: eid,
        field: observation.field.clone(),
        start: 0,
        end: 4,
        extraction_rule: "r".into(),
    }]);
    let t = std::time::Instant::now();
    db.record_claims(run, ("x", "1"), std::slice::from_ref(&extra), None)
        .unwrap();
    let elapsed = t.elapsed();
    eprintln!("N={n} scalar={scalar} incremental 1 claim: {elapsed:?}");
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "incremental publication replayed the hub: {elapsed:?}"
    );
    claims.push(extra);
    let before = serde_json::to_value(explained(&db, &id)).unwrap();
    db.begin_claims_rebuild().unwrap();
    db.record_claims(run, ("x", "1"), &claims, None).unwrap();
    db.finish_claims_rebuild(true).unwrap();
    assert_eq!(before, serde_json::to_value(explained(&db, &id)).unwrap());
}

/// claims/R52: a new observation on an entity typed at many locations replays only its own slots, the
/// entity-wide type conflict stays in place, and a rebuild equals the incremental projection
#[test]
fn a_new_observation_on_a_hub_replays_only_its_slots_and_equals_a_rebuild() {
    let n: usize = 4000;
    let home = tempfile::tempdir().unwrap();
    let mut db = SqliteStorage::open(&home.path().join("db")).unwrap();
    let mut source = Source::github("s".into(), "o/p".into(), Scope("work".into()));
    source.connector = "scripted".into();
    db.project_sources(std::slice::from_ref(&source)).unwrap();
    let run = db
        .start_run(&source, ("scripted", "1"), time(), (time(), time()))
        .unwrap();
    let evidence = |i: usize, quote: &str| Evidence {
        source: "s".into(),
        scope: Scope("work".into()),
        location: format!("https://x/{i}"),
        field: "/user/type".into(),
        start: 0,
        end: quote.len() as u64,
        quote: quote.into(),
        trust_class: TrustClass::Structured,
        captured_at: time() + chrono::Duration::seconds(i as i64),
        source_updated_at: Some(time()),
        original: None,
    };
    let subject = Entity::new("github:account/1".into(), "code:Account".into());
    let claim_for = |kind: &str, eid: i64, e: &Evidence| {
        let object = Object::Literal {
            value: kind.into(),
            datatype: "string".into(),
        };
        ObservedClaim {
            type_ancestors: Vec::new(),
            scalar: true,
            id: claims::claim_id(&subject.id, "rastro:type", &object),
            subject: subject.clone(),
            predicate: "rastro:type".into(),
            object,
            supports: BTreeSet::from([Support {
                portable_key: claims::portable_evidence_key(e),
                evidence_id: eid,
                field: e.field.clone(),
                start: 0,
                end: e.end,
                extraction_rule: "r".into(),
            }]),
        }
    };
    for i in 0..n {
        db.append_evidence(run, &evidence(i, "Account")).unwrap();
    }
    db.append_evidence(run, &evidence(n, "Bot")).unwrap();
    let pending = db.pending_evidence(("x", "1"), &source).unwrap();
    let mut all: Vec<_> = pending
        .iter()
        .map(|(eid, e)| {
            claim_for(
                if e.quote == "Bot" {
                    "code:Bot"
                } else {
                    "code:Account"
                },
                *eid,
                e,
            )
        })
        .collect();
    db.record_claims(run, ("x", "1"), &all, None).unwrap();
    let (account, bot) = (all[0].id.clone(), all[n].id.clone());
    assert_eq!(
        explained(&db, &account).claim.claim.conflicts_with,
        vec![bot.clone()]
    );
    let fresh = evidence(n + 1, "Account");
    let eid = match db.append_evidence(run, &fresh).unwrap() {
        rastro_core::Appended::New(id) => id,
        _ => panic!("new location must append"),
    };
    let extra = claim_for("code:Account", eid, &fresh);
    let database = rusqlite::Connection::open(home.path().join("db")).unwrap();
    let max_rowid = |table: &str| -> i64 {
        database
            .query_row(
                &format!("SELECT coalesce(max(rowid), 0) FROM {table}"),
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    let (occurrences, justifications) = (max_rowid("occurrence"), max_rowid("justification"));
    let t = std::time::Instant::now();
    db.record_claims(run, ("x", "1"), std::slice::from_ref(&extra), None)
        .unwrap();
    let elapsed = t.elapsed();
    // Structure, not time: only the new location's slot replays, so a handful of rows are
    // written, not one per location of the hub.
    for (table, before) in [
        ("occurrence", occurrences),
        ("justification", justifications),
    ] {
        let written: i64 = database
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE rowid > ?1"),
                [before],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            written <= 4,
            "{table}: {written} rows rewritten for one observation"
        );
    }
    eprintln!("N={n} incremental 1 claim: {elapsed:?}");
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "incremental publication replayed the hub: {elapsed:?}"
    );
    all.push(extra);
    let before = [&account, &bot].map(|id| serde_json::to_value(explained(&db, id)).unwrap());
    assert_eq!(
        before[1]["claim"]["conflicts_with"],
        serde_json::json!([account])
    );
    db.begin_claims_rebuild().unwrap();
    db.record_claims(run, ("x", "1"), &all, None).unwrap();
    db.finish_claims_rebuild(true).unwrap();
    let after = [&account, &bot].map(|id| serde_json::to_value(explained(&db, id)).unwrap());
    assert_eq!(before, after);
}
