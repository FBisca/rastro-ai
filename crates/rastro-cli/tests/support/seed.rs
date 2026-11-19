//! Observed claims seeded straight into a home's store, so a proposal can cite them without any
//! source being called.
#![allow(dead_code)]
use chrono::{DateTime, Utc};
use rastro_core::claims::{self, Entity, Object, ObservedClaim, Support};
use rastro_core::{Appended, Evidence, Scope, Source, Storage, TrustClass};
use rastro_storage_sqlite::SqliteStorage;
use std::{collections::BTreeSet, path::Path};

fn time() -> DateTime<Utc> {
    "2026-11-18T00:00:00Z".parse().unwrap()
}

/// A claim observed in `scope` only, about `subject`, and its id. `home` is the `RASTRO_HOME`.
pub fn claim_in(home: &Path, scope: &str, subject: &str) -> String {
    let mut db = SqliteStorage::open(&home.join("data/rastro.sqlite3")).unwrap();
    let source = Source::github(format!("notes-{scope}"), "o/p".into(), Scope(scope.into()));
    db.project_sources(std::slice::from_ref(&source)).unwrap();
    let run = db
        .start_run(&source, ("fixture", "1"), time(), (time(), time()))
        .unwrap();
    let evidence = Evidence {
        source: source.name.clone(),
        scope: source.scope.clone(),
        location: subject.into(),
        field: "/state".into(),
        start: 0,
        end: 4,
        quote: "open".into(),
        trust_class: TrustClass::Structured,
        captured_at: time(),
        source_updated_at: Some(time()),
        original: None,
    };
    let evidence_id = match db.append_evidence(run, &evidence).unwrap() {
        Appended::New(id) | Appended::Unchanged(id) => id,
    };
    let object = Object::Literal {
        value: "open".into(),
        datatype: "string".into(),
    };
    let entity = Entity::new(subject.into(), "code:PullRequest".into());
    let id = claims::claim_id(&entity.id, "code:state", &object);
    let run = db
        .start_run(&source, ("fixture", "1"), time(), (time(), time()))
        .unwrap();
    db.record_claims(
        run,
        ("fixture", "1"),
        &[ObservedClaim {
            id: id.clone(),
            subject: entity,
            predicate: "code:state".into(),
            object,
            scalar: true,
            type_ancestors: Vec::new(),
            supports: BTreeSet::from([Support {
                portable_key: claims::portable_evidence_key(&evidence),
                evidence_id,
                field: "/state".into(),
                start: 0,
                end: 4,
                extraction_rule: "fixture-rule".into(),
            }]),
        }],
        None,
    )
    .unwrap();
    id
}
