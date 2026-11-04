//! Deterministic lifecycle projection. Replaying support observations in evidence order makes
//! intervals and revisions independent of extractor submission order and batching.
use crate::parse_time;
use anyhow::Result;
use rastro_core::claims::{KnownInterval, Support, revision_with_links};
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, BTreeSet};

type Context = (String, String, String, String, String);
#[derive(Clone)]
struct Observation {
    claim: String,
    source: String,
    scope: String,
    context: Context,
    scalar: bool,
    captured: String,
    updated: Option<chrono::DateTime<chrono::Utc>>,
    support: Support,
}
impl Observation {
    // Extraction rules have independent support streams, but cannot hide a disagreement
    // about the same resource field merely by naming their rules differently.
    fn field(&self) -> (&str, &str, &str, &str) {
        (
            &self.context.0,
            &self.context.1,
            &self.context.2,
            &self.context.3,
        )
    }
    fn same_field(&self, other: &Self) -> bool {
        self.field() == other.field()
    }
    fn slot(&self) -> (String, String, Context, Option<String>) {
        (
            self.source.clone(),
            self.scope.clone(),
            self.context.clone(),
            (!self.scalar).then(|| self.claim.clone()),
        )
    }
}

pub(crate) fn refresh(db: &Connection) -> Result<()> {
    let observations = db.prepare(
        "SELECT s.claim_id, e.source, e.scope, c.subject, c.predicate, e.location,
         s.field, s.extraction_rule, c.scalar, e.captured_at, s.evidence_id, s.start, s.end, e.source_updated_at
         FROM claim_support s JOIN evidence e ON e.id = s.evidence_id
         JOIN claim c ON c.id = s.claim_id ORDER BY e.captured_at, e.id, s.claim_id,
         s.extraction_rule, s.start, s.end"
    )?.query_map([], |r| Ok(Observation {
        claim: r.get(0)?, source: r.get(1)?, scope: r.get(2)?,
        context: (r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?),
        scalar: r.get(8)?, captured: r.get(9)?,
        updated: r.get::<_, Option<String>>(13)?.map(|v| v.parse().expect("validated evidence timestamp")),
        support: Support { evidence_id: r.get(10)?, field: r.get(6)?,
            extraction_rule: r.get(7)?, start: r.get::<_, i64>(11)? as u64,
            end: r.get::<_, i64>(12)? as u64 },
    }))?.collect::<Result<Vec<_>, _>>()?;
    db.execute_batch(
        "DELETE FROM known_interval; DELETE FROM claim_link; UPDATE claim_support SET current = 0;",
    )?;
    let mut latest = BTreeMap::new();
    let mut previous = BTreeSet::new();
    let mut current = Vec::new();
    // All claims on one evidence observation become visible together.
    let mut offset = 0;
    while offset < observations.len() {
        let first = &observations[offset];
        let mut end = offset + 1;
        while end < observations.len()
            && observations[end].support.evidence_id == first.support.evidence_id
        {
            end += 1;
        }
        for observation in &observations[offset..end] {
            let slot = observation.slot();
            let entries: &mut Vec<&Observation> = latest.entry(slot).or_default();
            if entries
                .first()
                .is_some_and(|old| old.support.evidence_id != observation.support.evidence_id)
            {
                entries.clear();
            }
            entries.push(observation);
        }
        current = latest.values().flatten().copied().collect::<Vec<_>>();
        let mut candidates = BTreeMap::<_, Vec<_>>::new();
        for &observation in &current {
            candidates
                .entry(observation.field())
                .or_default()
                .push(observation);
        }
        current.retain(|old| {
            !old.scalar
                || !candidates[&old.field()].iter().any(|new| {
                    old.claim != new.claim
                        && matches!((old.updated, new.updated), (Some(a), Some(b)) if b > a)
                })
        });
        let now: BTreeSet<_> = current.iter().map(|o| o.claim.clone()).collect();
        for id in previous.difference(&now) {
            db.execute(
                "UPDATE known_interval SET end = ?2 WHERE claim_id = ?1 AND end IS NULL",
                params![id, first.captured],
            )?;
        }
        for id in now.difference(&previous) {
            db.execute(
                "INSERT INTO known_interval (claim_id, start, evidence_id) VALUES (?1, ?2, ?3)",
                params![id, first.captured, first.support.evidence_id],
            )?;
        }
        previous = now;
        offset = end;
    }
    // Only scalar replacement establishes a supersession link. A revived claim no longer
    // points at the value which previously replaced it.
    let mut edges = BTreeSet::new();
    for old in &observations {
        if previous.contains(&old.claim) || !old.scalar {
            continue;
        }
        for new in &current {
            if old.same_field(new) && old.claim != new.claim {
                edges.insert((new.claim.clone(), old.claim.clone(), "supersedes"));
            }
        }
    }
    for left in &current {
        if !left.scalar {
            continue;
        }
        for right in &current {
            if left.same_field(right) && left.claim != right.claim {
                edges.insert((left.claim.clone(), right.claim.clone(), "conflicts_with"));
            }
        }
    }
    for (from, to, kind) in edges {
        db.execute(
            "INSERT INTO claim_link VALUES (?1, ?2, ?3)",
            params![from, to, kind],
        )?;
    }
    for o in &current {
        let s = &o.support;
        db.execute(
            "UPDATE claim_support SET current = 1 WHERE claim_id = ?1 AND evidence_id = ?2
            AND field = ?3 AND start = ?4 AND end = ?5 AND extraction_rule = ?6",
            params![
                o.claim,
                s.evidence_id,
                s.field,
                s.start as i64,
                s.end as i64,
                s.extraction_rule
            ],
        )?;
    }
    let ids = db
        .prepare("SELECT id FROM claim ORDER BY id")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for id in ids {
        let supports: Vec<_> = current
            .iter()
            .filter(|o| o.claim == id)
            .map(|o| o.support.clone())
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
        db.execute(
            "UPDATE claim SET status = ?2, revision = ?3 WHERE id = ?1",
            params![
                id,
                status,
                revision_with_links(status, &supports, &visible_links)
            ],
        )?;
    }
    Ok(())
}

pub(crate) fn intervals(db: &Connection, id: &str) -> Result<Vec<KnownInterval>> {
    let rows = db
        .prepare(
            "SELECT start, end FROM known_interval WHERE claim_id = ?1 ORDER BY start, evidence_id",
        )?
        .query_map([id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(start, end)| {
            Ok(KnownInterval {
                start: parse_time(start)?,
                end: end.map(parse_time).transpose()?,
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

// These tables together form one generation. A rebuild's TEMP tables shadow the active
// tables on this connection only. Readers on other connections keep reading main until the
// publication transaction commits. Extractor runs always go to the persistent audit log.
const TABLES: &[&str] = &[
    "entity",
    "alias",
    "claim",
    "claim_support",
    "known_interval",
    "claim_link",
    "extraction_checkpoint",
];
pub(crate) fn begin_rebuild(db: &Connection) -> Result<()> {
    let transaction = db.unchecked_transaction()?;
    for table in TABLES {
        let sql: String = transaction.query_row(
            "SELECT sql FROM main.sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |r| r.get(0),
        )?;
        let mut sql = sql.replacen("CREATE TABLE", "CREATE TEMP TABLE", 1);
        // SQLite cannot enforce references from TEMP to main. Replay only uses core-validated
        // submissions; the publication transaction enforces the active generation's FKs.
        for target in ["entity(id)", "claim(id)", "evidence(id)", "runs(id)"] {
            sql = sql.replace(&format!(" REFERENCES {target}"), "");
        }
        transaction.execute_batch(&sql)?;
    }
    transaction.commit()?;
    Ok(())
}
pub(crate) fn finish_rebuild(db: &mut Connection, publish: bool) -> Result<usize> {
    let transaction = db.transaction()?;
    let count: i64 = transaction.query_row("SELECT count(*) FROM temp.claim", [], |r| r.get(0))?;
    if publish {
        for table in TABLES.iter().rev() {
            transaction.execute(&format!("DELETE FROM main.{table}"), [])?;
        }
        for table in TABLES {
            transaction.execute(
                &format!("INSERT INTO main.{table} SELECT * FROM temp.{table}"),
                [],
            )?;
        }
        transaction.execute("UPDATE main.projection_generation SET id = id + 1", [])?;
    }
    for table in TABLES.iter().rev() {
        transaction.execute_batch(&format!("DROP TABLE temp.{table}"))?;
    }
    transaction.commit()?;
    Ok(count as usize)
}
