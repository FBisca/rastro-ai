//! Replay the affected subject/predicate components in evidence append order. A derivation
//! is live only while every span is current; derivations of one claim are OR alternatives.
use crate::parse_time;
use anyhow::Result;
use rastro_core::claims::{
    KnownInterval, Support, occurrence_id_with_ordinal, revision_with_links,
};
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, BTreeSet};

type Event = (String, i64);
type Component = (String, String);
type Slot = (String, String, String, String, String, Option<String>);
#[derive(Clone)]
struct Span {
    source: String,
    scope: String,
    location: String,
    event: Event,
    updated: Option<chrono::DateTime<chrono::Utc>>,
    support: Support,
}
impl Span {
    fn slot(&self, derivation: &Derivation) -> Slot {
        (
            self.source.clone(),
            self.scope.clone(),
            self.location.clone(),
            self.support.field.clone(),
            self.support.extraction_rule.clone(),
            (!derivation.scalar).then(|| derivation.claim.clone()),
        )
    }
    fn same_span(&self, other: &Self) -> bool {
        self.support.portable_key == other.support.portable_key
            && self.support.field == other.support.field
            && self.support.start == other.support.start
            && self.support.end == other.support.end
    }
    fn same_field(&self, other: &Self) -> bool {
        self.location == other.location && self.support.field == other.support.field
    }
}
struct Derivation {
    id: String,
    claim: String,
    scalar: bool,
    spans: Vec<Span>,
}

pub(crate) fn refresh(db: &Connection, affected: &BTreeSet<Component>) -> Result<()> {
    for (subject, predicate) in affected {
        recompute(db, subject, predicate)?;
    }
    Ok(())
}

fn recompute(db: &Connection, subject: &str, predicate: &str) -> Result<()> {
    let mut derivations: BTreeMap<String, Derivation> = BTreeMap::new();
    let rows = db.prepare(
        "SELECT j.id, j.claim_id, c.scalar, e.source, e.scope, e.location, e.captured_at,
         e.source_updated_at, s.evidence_id, s.field, s.start, s.end, s.portable_key, j.extraction_rule
         FROM justification j JOIN claim c ON c.id = j.claim_id
         JOIN justification_span s ON s.justification_id = j.id JOIN evidence e ON e.id = s.evidence_id
         WHERE c.subject = ?1 AND c.predicate = ?2 ORDER BY j.id, s.portable_key")?
        .query_map(params![subject, predicate], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?, Span {
                source: r.get(3)?, scope: r.get(4)?, location: r.get(5)?,
                event: (r.get(6)?, r.get(8)?),
                updated: r.get::<_, Option<String>>(7)?.map(|s| s.parse().expect("validated evidence timestamp")),
                support: Support { evidence_id: r.get(8)?, field: r.get(9)?, start: r.get::<_, i64>(10)? as u64,
                    end: r.get::<_, i64>(11)? as u64, portable_key: r.get(12)?, extraction_rule: r.get(13)? },
            }))
        })?.collect::<Result<Vec<_>, _>>()?;
    for (id, claim, scalar, span) in rows {
        derivations
            .entry(id.clone())
            .or_insert_with(|| Derivation {
                id,
                claim,
                scalar,
                spans: vec![],
            })
            .spans
            .push(span);
    }
    db.execute("DELETE FROM occurrence WHERE justification_id IN (SELECT j.id FROM justification j JOIN claim c ON c.id = j.claim_id WHERE c.subject = ?1 AND c.predicate = ?2)", params![subject, predicate])?;
    db.execute("DELETE FROM claim_link WHERE from_claim IN (SELECT id FROM claim WHERE subject = ?1 AND predicate = ?2) OR to_claim IN (SELECT id FROM claim WHERE subject = ?1 AND predicate = ?2)", params![subject, predicate])?;
    db.execute("UPDATE justification SET current = 0 WHERE claim_id IN (SELECT id FROM claim WHERE subject = ?1 AND predicate = ?2)", params![subject, predicate])?;
    let mut events = BTreeMap::<Event, Vec<(&Derivation, &Span)>>::new();
    for d in derivations.values() {
        for span in &d.spans {
            events
                .entry(span.event.clone())
                .or_default()
                .push((d, span));
        }
    }
    let mut latest = BTreeMap::<Slot, Event>::new();
    let mut previous = BTreeSet::<String>::new();
    let mut current = Vec::new();
    let mut ordinals = BTreeMap::<(String, String, String), u64>::new();
    for (event, observations) in events {
        let portable_key = observations[0].1.support.portable_key.clone();
        for (d, span) in observations {
            latest.insert(span.slot(d), event.clone());
        }
        let candidates: Vec<_> = derivations
            .values()
            .filter(|d| {
                d.spans.iter().all(|s| {
                    d.spans.iter().any(|other| {
                        s.same_span(other) && latest.get(&other.slot(d)) == Some(&other.event)
                    })
                })
            })
            .collect();
        current = candidates
            .iter()
            .copied()
            .filter(|old| {
                !old.scalar
                    || !candidates.iter().any(|new| {
                        old.claim != new.claim
                            && old.spans.iter().filter(|a| latest.get(&a.slot(old)) == Some(&a.event)).any(|a| {
                                new.spans.iter().filter(|b| latest.get(&b.slot(new)) == Some(&b.event)).any(|b|
                a.same_field(b) && matches!((a.updated, b.updated), (Some(a), Some(b)) if b > a))
                            })
                    })
            })
            .collect();
        let now: BTreeSet<_> = current.iter().map(|d| d.id.clone()).collect();
        for id in previous.difference(&now) {
            db.execute("UPDATE occurrence SET end = ?2, end_evidence_id = ?3 WHERE justification_id = ?1 AND end IS NULL", params![id, event.0, event.1])?;
        }
        for id in now.difference(&previous) {
            let d = &derivations[id];
            let span = d
                .spans
                .iter()
                .filter(|s| latest.get(&s.slot(d)) == Some(&s.event))
                .max_by_key(|s| &s.event)
                .expect("nonempty derivation");
            let ordinal = ordinals
                .entry((id.clone(), event.0.clone(), portable_key.clone()))
                .or_default();
            let occurrence = occurrence_id_with_ordinal(id, &event.0, &portable_key, *ordinal);
            *ordinal += 1;
            db.execute("INSERT INTO occurrence (id, justification_id, source, scope, location, field,
                captured_at, source_updated_at, portable_key, evidence_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![occurrence, id, span.source, span.scope, span.location,
                    span.support.field, event.0, span.updated.map(crate::stamp), portable_key, event.1])?;
        }
        previous = now;
    }
    let current_claims: BTreeSet<_> = current.iter().map(|d| &d.claim).collect();
    let same_field = |a: &Derivation, b: &Derivation| {
        a.spans
            .iter()
            .any(|s| b.spans.iter().any(|t| s.same_field(t)))
    };
    let mut edges = BTreeSet::new();
    for old in derivations.values() {
        if !old.scalar || current_claims.contains(&old.claim) {
            continue;
        }
        for new in &current {
            if old.claim != new.claim && same_field(old, new) {
                edges.insert((&new.claim, &old.claim, "supersedes"));
            }
        }
    }
    for left in &current {
        if !left.scalar {
            continue;
        }
        for right in &current {
            if left.claim != right.claim && same_field(left, right) {
                edges.insert((&left.claim, &right.claim, "conflicts_with"));
            }
        }
    }
    for (from, to, kind) in edges {
        db.execute(
            "INSERT INTO claim_link VALUES (?1, ?2, ?3)",
            params![from, to, kind],
        )?;
    }
    for d in &current {
        db.execute(
            "UPDATE justification SET current = 1 WHERE id = ?1",
            [&d.id],
        )?;
    }
    let ids = db
        .prepare("SELECT id FROM claim WHERE subject = ?1 AND predicate = ?2 ORDER BY id")?
        .query_map(params![subject, predicate], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for id in ids {
        let supports: Vec<_> = current
            .iter()
            .filter(|d| d.claim == id)
            .flat_map(|d| d.spans.iter().map(|s| s.support.clone()))
            .collect();
        let status = if supports.is_empty() {
            "superseded"
        } else {
            "observed"
        };
        let mut visible_links = Vec::new();
        for (kind, inverse) in [
            ("conflicts_with", false),
            ("supersedes", false),
            ("supersedes", true),
        ] {
            for target in links(db, &id, kind, inverse)? {
                visible_links.push((
                    if inverse { "superseded_by" } else { kind }.to_string(),
                    target,
                ));
            }
        }
        db.execute("UPDATE claim SET status = ?2, revision = ?3, known_since = coalesce((
            SELECT max(o.captured_at) FROM occurrence o JOIN justification j ON j.id = o.justification_id WHERE j.claim_id = ?1), '') WHERE id = ?1",
            params![id, status, revision_with_links(status, &supports, &visible_links)])?;
    }
    Ok(())
}

pub(crate) fn intervals(db: &Connection, id: &str) -> Result<Vec<KnownInterval>> {
    let rows = db.prepare("SELECT o.captured_at, o.evidence_id, o.end, o.end_evidence_id FROM occurrence o
        JOIN justification j ON j.id = o.justification_id WHERE j.claim_id = ?1 ORDER BY o.captured_at, o.evidence_id, o.id")?
        .query_map([id], |r| Ok(((r.get::<_, String>(0)?, r.get::<_, i64>(1)?), r.get::<_, Option<String>>(2)?.zip(r.get::<_, Option<i64>>(3)?))))?
        .collect::<Result<Vec<_>, _>>()?;
    // Union on event keys, preserving a gap even if its timestamps are equal.
    let mut intervals: Vec<(Event, Option<Event>)> = Vec::new();
    for (start, end) in rows {
        if let Some((_, last_end)) = intervals.last_mut()
            && last_end.as_ref().is_none_or(|e| e >= &start)
        {
            if end.is_none()
                || last_end
                    .as_ref()
                    .is_some_and(|old| end.as_ref().is_some_and(|new| new > old))
            {
                *last_end = end;
            }
            continue;
        }
        intervals.push((start, end));
    }
    intervals
        .into_iter()
        .map(|(start, end)| {
            Ok(KnownInterval {
                start: parse_time(start.0)?,
                end: end.map(|e| parse_time(e.0)).transpose()?,
            })
        })
        .collect()
}
pub(crate) fn links(db: &Connection, id: &str, kind: &str, inverse: bool) -> Result<Vec<String>> {
    let sql = if inverse {
        "SELECT from_claim FROM claim_link WHERE to_claim = ?1 AND kind = ?2 ORDER BY from_claim"
    } else {
        "SELECT to_claim FROM claim_link WHERE from_claim = ?1 AND kind = ?2 ORDER BY to_claim"
    };
    Ok(db
        .prepare(sql)?
        .query_map(params![id, kind], |r| r.get(0))?
        .collect::<Result<_, _>>()?)
}

const TABLES: &[&str] = &[
    "entity",
    "claim",
    "justification",
    "justification_span",
    "occurrence",
    "claim_link",
    "extraction_checkpoint",
];
pub(crate) fn staging(db: &Connection) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_temp_master WHERE name = 'claim')",
        [],
        |r| r.get(0),
    )?)
}
pub(crate) fn publish(db: &Connection) -> Result<()> {
    let extractors: Vec<(String, String)> = db.prepare("SELECT DISTINCT extractor, extractor_version FROM extraction_checkpoint ORDER BY extractor, extractor_version")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    let vocabularies: Vec<(String, String)> = db.prepare("SELECT DISTINCT extractor, vocabulary_version FROM extraction_checkpoint ORDER BY extractor, vocabulary_version")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    let high_water: Vec<(String, String, String, String)> = db.prepare("SELECT source, scope, last_captured_at, last_portable_key FROM extraction_checkpoint ORDER BY source, scope, extractor")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
    db.execute("INSERT INTO main.projection_publication (extractor_versions, vocabulary_versions, evidence_high_water) VALUES (?1, ?2, ?3)",
        params![serde_json::to_string(&extractors)?, serde_json::to_string(&vocabularies)?, serde_json::to_string(&high_water)?])?;
    for table in TABLES {
        db.execute_batch(&format!("ANALYZE main.{table}"))?;
    }
    Ok(())
}
pub(crate) fn begin_rebuild(db: &Connection) -> Result<()> {
    db.execute_batch(include_str!("projection_temp.sql"))?;
    Ok(())
}
pub(crate) fn finish_rebuild(db: &mut Connection, publish_projection: bool) -> Result<usize> {
    let transaction = db.transaction()?;
    let count: i64 = transaction.query_row("SELECT count(*) FROM temp.claim", [], |r| r.get(0))?;
    if publish_projection {
        for table in TABLES.iter().rev() {
            transaction.execute(&format!("DELETE FROM main.{table}"), [])?;
        }
        for table in TABLES {
            transaction.execute(
                &format!("INSERT INTO main.{table} SELECT * FROM temp.{table}"),
                [],
            )?;
        }
    }
    for table in TABLES.iter().rev() {
        transaction.execute_batch(&format!("DROP TABLE temp.{table}"))?;
    }
    if publish_projection {
        publish(&transaction)?;
    }
    transaction.commit()?;
    Ok(count as usize)
}
