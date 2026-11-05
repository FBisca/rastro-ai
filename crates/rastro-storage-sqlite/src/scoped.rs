//! Exact replay in private memory. No stored status, occurrence, revision or link is copied.
//! The source connection stays in its pinned read transaction throughout the computation.
use super::*;
use rusqlite::{params_from_iter, types::Value};
use std::collections::BTreeSet;

pub(super) enum Target<'a> {
    Entity(&'a str),
    Claim(&'a str),
}

fn copy(
    db: &Connection,
    target: &Connection,
    table: &str,
    sql: &str,
    arguments: &[String],
) -> Result<()> {
    let mut source = db.prepare(sql)?;
    let columns = source.column_count();
    let placeholders = vec!["?"; columns].join(",");
    let mut insert = target.prepare(&format!("INSERT INTO {table} VALUES ({placeholders})"))?;
    let mut rows = source.query(params_from_iter(arguments))?;
    while let Some(row) = rows.next()? {
        let values = (0..columns)
            .map(|i| row.get::<_, Value>(i))
            .collect::<Result<Vec<_>, _>>()?;
        insert.execute(params_from_iter(values))?;
    }
    Ok(())
}
fn ids(db: &Connection, sql: &str) -> Result<String> {
    let rows = db
        .prepare(sql)?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::to_string(&rows)?)
}

pub(super) fn projection(
    db: &Connection,
    scopes: &[Scope],
    target: Target<'_>,
) -> Result<Connection> {
    // Replay complete subject/predicate components, including competing claims. One-hop
    // candidates are selected before replay, but status, recency and the limit are not.
    let (root, argument) = match target {
        Target::Entity(entity) => (
            "SELECT id FROM entity WHERE id = ?1 OR identifier = ?1 UNION SELECT subject FROM claim WHERE predicate = 'rastro:alias' AND object_value = ?1",
            entity,
        ),
        Target::Claim(claim) => ("SELECT subject FROM claim WHERE id = ?1", claim),
    };
    let candidate_sql = format!("WITH roots AS ({root}), components AS (
        SELECT subject, predicate FROM claim WHERE subject IN roots UNION ALL SELECT subject, predicate FROM claim WHERE object_entity IN roots
    ), neighbors AS (
        SELECT c.id, c.subject, c.object_entity FROM claim c JOIN components k ON c.subject = k.subject AND c.predicate = k.predicate
    ) SELECT id FROM neighbors UNION SELECT id FROM claim WHERE predicate IN ('rastro:type','rastro:alias') AND subject IN (SELECT subject FROM neighbors UNION SELECT object_entity FROM neighbors)");
    let candidates = db
        .prepare(&candidate_sql)?
        .query_map([argument], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let candidates = serde_json::to_string(&candidates)?;
    let scopes = serde_json::to_string(scopes)?;
    let result = Connection::open_in_memory()?;
    result.execute_batch("PRAGMA foreign_keys = OFF;")?;
    result.execute_batch(include_str!("projection_temp.sql"))?;
    result.execute_batch("BEGIN")?;
    // Evidence is immutable; only cited rows are needed by the private replay and citations.
    result.execute_batch("CREATE TABLE evidence (id INTEGER PRIMARY KEY, source TEXT, scope TEXT, location TEXT, field TEXT, start INTEGER, end INTEGER, quote TEXT, captured_at TEXT, source_updated_at TEXT, trust_class TEXT, original TEXT)")?;
    // Every required span must be visible. Partial bundles never become weaker proofs.
    copy(
        db,
        &result,
        "justification",
        "SELECT j.* FROM justification j WHERE j.claim_id IN (SELECT value FROM json_each(?1)) AND EXISTS (SELECT 1 FROM justification_span s WHERE s.justification_id = j.id) AND NOT EXISTS (SELECT 1 FROM justification_span s JOIN evidence e ON e.id = s.evidence_id WHERE s.justification_id = j.id AND e.scope NOT IN (SELECT value FROM json_each(?2)))",
        &[candidates, scopes],
    )?;
    let justifications = ids(&result, "SELECT id FROM justification")?;
    copy(
        db,
        &result,
        "justification_span",
        "SELECT * FROM justification_span WHERE justification_id IN (SELECT value FROM json_each(?1))",
        &[justifications],
    )?;
    let claims = ids(&result, "SELECT DISTINCT claim_id FROM justification")?;
    copy(
        db,
        &result,
        "claim",
        "SELECT * FROM claim WHERE id IN (SELECT value FROM json_each(?1))",
        &[claims],
    )?;
    let entities = ids(
        &result,
        "SELECT subject FROM claim UNION SELECT object_entity FROM claim WHERE object_entity IS NOT NULL",
    )?;
    copy(
        db,
        &result,
        "entity",
        "SELECT * FROM entity WHERE id IN (SELECT value FROM json_each(?1))",
        &[entities],
    )?;
    let evidence = ids(
        &result,
        "SELECT DISTINCT CAST(evidence_id AS TEXT) FROM justification_span",
    )?;
    copy(
        db,
        &result,
        "evidence",
        "SELECT id,source,scope,location,field,start,end,quote,captured_at,source_updated_at,trust_class,original FROM evidence WHERE id IN (SELECT value FROM json_each(?1))",
        &[evidence],
    )?;
    replay(&result)?;
    result.execute_batch("COMMIT")?;
    Ok(result)
}

// A component with exactly one required span has no competing event or alternative
// derivation: it opens once and remains observed. Batch this common case; replay all
// multi-span/alternative/replacement components through the lifecycle oracle.
fn replay(db: &Connection) -> Result<()> {
    use rastro_core::claims::{Support, occurrence_id_with_ordinal, revision_with_links};
    let mut affected = db
        .prepare("SELECT DISTINCT subject, predicate FROM claim")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<BTreeSet<_>, _>>()?;
    let mut rows = db.prepare("SELECT c.subject,c.predicate,c.id,j.id,s.evidence_id,s.portable_key,s.field,s.start,s.end,j.extraction_rule,e.source,e.scope,e.location,e.captured_at,e.source_updated_at FROM claim c JOIN justification j ON j.claim_id = c.id JOIN justification_span s ON s.justification_id = j.id JOIN evidence e ON e.id = s.evidence_id GROUP BY c.subject,c.predicate HAVING count(*) = 1")?;
    let mut rows = rows.query([])?;
    let mut claim =
        db.prepare("UPDATE claim SET status='observed', revision=?2, known_since=?3 WHERE id=?1")?;
    let mut current = db.prepare("UPDATE justification SET current=1 WHERE id=?1")?;
    let mut occurrence = db.prepare("INSERT INTO occurrence (id,justification_id,source,scope,location,field,captured_at,source_updated_at,portable_key,evidence_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)")?;
    while let Some(row) = rows.next()? {
        let subject: String = row.get(0)?;
        let predicate: String = row.get(1)?;
        let id: String = row.get(2)?;
        let justification: String = row.get(3)?;
        let support = Support {
            evidence_id: row.get(4)?,
            portable_key: row.get(5)?,
            field: row.get(6)?,
            start: row.get::<_, i64>(7)? as u64,
            end: row.get::<_, i64>(8)? as u64,
            extraction_rule: row.get(9)?,
        };
        let captured: String = row.get(13)?;
        claim.execute(params![
            id,
            revision_with_links("observed", std::slice::from_ref(&support), &[]),
            captured
        ])?;
        current.execute([&justification])?;
        occurrence.execute(params![
            occurrence_id_with_ordinal(&justification, &captured, &support.portable_key, 0),
            justification,
            row.get::<_, String>(10)?,
            row.get::<_, String>(11)?,
            row.get::<_, String>(12)?,
            support.field,
            captured,
            row.get::<_, Option<String>>(14)?,
            support.portable_key,
            support.evidence_id
        ])?;
        affected.remove(&(subject, predicate));
    }
    lifecycle::refresh(db, &affected)
}
