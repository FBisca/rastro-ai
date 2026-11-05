//! Exact replay in private memory. No stored status, occurrence, revision or link is copied.
//! The source connection stays in its pinned read transaction throughout the computation.
//!
//! Cache invariant: every write to the projection tables publishes a `projection_publication`
//! row (and new evidence or justifications raise their high-water marks), so a cached replay is
//! valid exactly while the key (database file, scope set, target, those marks) is unchanged.
//! The work budget counts only justification spans visible to the scope set, never hidden ones,
//! so an over-budget hidden-only target answers like an unknown one.
use super::*;
use anyhow::ensure;
use rusqlite::params_from_iter;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

/// Most visible justification spans one scoped read may replay. Past it the read fails with an
/// operation error; it never answers from a weaker or partial replay.
pub(super) const WORK_BUDGET: usize = 500_000;
const CACHE_ENTRIES: usize = 4;

/// A finished replay is exact for one visible scope set, one target and one state of the
/// database, so it can be reused until any of those changes.
#[derive(PartialEq, Eq)]
struct Key {
    database: String,
    file: (u64, u64),
    scopes: BTreeSet<String>,
    target: (bool, String),
    publication: i64,
    evidence: i64,
    justification: i64,
}
type Shared = Arc<Mutex<Connection>>;
/// Each entry keeps the visible work it cost, so a hit is held to the caller's budget too.
static CACHE: Mutex<Vec<(Key, usize, Shared)>> = Mutex::new(Vec::new());

fn key(db: &Connection, scopes: &[Scope], target: &Target<'_>) -> Result<Option<Key>> {
    let Some(database) = db.path().filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    use std::os::unix::fs::MetadataExt;
    let file = std::fs::metadata(database)?;
    let high = |sql: &str| db.query_row(sql, [], |r| r.get::<_, Option<i64>>(0));
    Ok(Some(Key {
        database: database.to_string(),
        file: (file.dev(), file.ino()),
        scopes: scopes.iter().map(|s| s.0.clone()).collect(),
        target: match target {
            Target::Entity(value) => (true, value.to_string()),
            Target::Claim(value) => (false, value.to_string()),
        },
        publication: high("SELECT max(id) FROM projection_publication")?.unwrap_or(0),
        evidence: high("SELECT max(id) FROM evidence")?.unwrap_or(0),
        justification: high("SELECT max(rowid) FROM justification")?.unwrap_or(0),
    }))
}

/// The cache holds only the output of the exact recompute; a miss recomputes.
pub(super) fn projection(
    db: &Connection,
    scopes: &[Scope],
    target: Target<'_>,
    budget: usize,
) -> Result<Shared> {
    let key = key(db, scopes, &target)?;
    if let Some(key) = &key {
        let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, work, hit)) = cache.iter().find(|(k, _, _)| k == key) {
            ensure!(*work <= budget, OVER_BUDGET);
            return Ok(hit.clone());
        }
    }
    let (connection, work) = build(db, scopes, target, budget)?;
    let built = Arc::new(Mutex::new(connection));
    if let Some(key) = key {
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= CACHE_ENTRIES {
            cache.remove(0);
        }
        cache.push((key, work, built.clone()));
    }
    Ok(built)
}

/// A justification of `j` whose every required span is visible to the scope set `?2`.
const VISIBLE_JUSTIFICATION: &str = "EXISTS (SELECT 1 FROM justification_span s WHERE s.justification_id = j.id) AND NOT EXISTS (SELECT 1 FROM justification_span s JOIN evidence e ON e.id = s.evidence_id WHERE s.justification_id = j.id AND e.scope NOT IN (SELECT value FROM json_each(?2)))";
/// The claim `c` has a justification visible to the scope set `?2`.
const VISIBLE_CLAIM: &str = "EXISTS (SELECT 1 FROM justification j WHERE j.claim_id = c.id AND EXISTS (SELECT 1 FROM justification_span s WHERE s.justification_id = j.id) AND NOT EXISTS (SELECT 1 FROM justification_span s JOIN evidence e ON e.id = s.evidence_id WHERE s.justification_id = j.id AND e.scope NOT IN (SELECT value FROM json_each(?2))))";
const OVER_BUDGET: &str = "this read is too large to answer within its work budget";

/// Empties the process-local scoped cache; a benchmark's cold samples start from here.
#[doc(hidden)]
pub fn clear_cache() {
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

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
        insert.execute(params_from_iter(
            (0..columns)
                .map(|i| row.get_ref(i).map(rusqlite::types::ToSqlOutput::Borrowed))
                .collect::<Result<Vec<_>, _>>()?,
        ))?;
    }
    Ok(())
}
fn ids(db: &Connection, sql: &str, arguments: &[String]) -> Result<String> {
    let rows = db
        .prepare(sql)?
        .query_map(params_from_iter(arguments), |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::to_string(&rows)?)
}

fn build(
    db: &Connection,
    scopes: &[Scope],
    target: Target<'_>,
    budget: usize,
) -> Result<(Connection, usize)> {
    // Replay complete subject/predicate components, including competing claims. One-hop
    // candidates are selected before replay, but status, recency and the limit are not.
    let scopes = serde_json::to_string(scopes)?;
    let (roots, components, arguments) = match target {
        Target::Entity(entity) => (
            // Alias roots are the matching alias claims with a fully visible justification;
            // stored status is full-scope, so it plays no part here. The replay applies the
            // status set, the ambiguity and its marks.
            format!(
                "SELECT id FROM entity WHERE id = ?1 OR identifier = ?1 UNION SELECT subject FROM ({}) a \
                 WHERE EXISTS (SELECT 1 FROM justification j WHERE j.claim_id = a.claim AND {VISIBLE_JUSTIFICATION})",
                super::knowledge::ALIAS_CLAIMS
            ),
            "SELECT subject, predicate FROM claim c WHERE subject IN roots AND {VISIBLE_CLAIM} UNION ALL SELECT subject, predicate FROM claim c WHERE object_entity IN roots AND {VISIBLE_CLAIM}",
            vec![entity.to_string(), scopes.clone()],
        ),
        // A claim's own slot only, plus its endpoints' type and alias claims below.
        Target::Claim(claim) => (
            "SELECT 1 WHERE 0".to_string(),
            "SELECT subject, predicate FROM claim c WHERE id = ?1 AND {VISIBLE_CLAIM}",
            vec![claim.to_string(), scopes.clone()],
        ),
    };
    // Only claims with a visible justification select slots, so hidden claims never pull
    // visible ones into a replay and their budget.
    let components = components.replace("{VISIBLE_CLAIM}", VISIBLE_CLAIM);
    let candidate_sql = format!("WITH roots AS ({roots}), components AS ({components}),
    neighbors AS (
        SELECT c.id, c.subject, c.object_entity, {VISIBLE_CLAIM} AS seen FROM claim c JOIN components k ON c.subject = k.subject AND c.predicate = k.predicate
    ) SELECT id FROM neighbors UNION SELECT id FROM claim WHERE predicate IN ('rastro:type','rastro:alias') AND subject IN (SELECT subject FROM neighbors WHERE seen UNION SELECT object_entity FROM neighbors WHERE seen)");
    let candidates = db
        .prepare(&candidate_sql)?
        .query_map(params_from_iter(&arguments), |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let candidates = serde_json::to_string(&candidates)?;
    // Every required span must be visible. Partial bundles never become weaker proofs.
    let visible = ids(
        db,
        &format!(
            "SELECT j.id FROM justification j WHERE j.claim_id IN (SELECT value FROM json_each(?1)) AND {VISIBLE_JUSTIFICATION}"
        ),
        &[candidates, scopes],
    )?;
    // The budget counts visible spans only: hidden work never shows in the outcome.
    let work: usize = db.query_row(
        "SELECT count(*) FROM (SELECT 1 FROM justification_span WHERE justification_id IN (SELECT value FROM json_each(?1)) LIMIT ?2)",
        rusqlite::params![visible, budget.saturating_add(1) as i64],
        |r| r.get::<_, i64>(0),
    )? as usize;
    ensure!(work <= budget, OVER_BUDGET);
    let result = Connection::open_in_memory()?;
    result.execute_batch("PRAGMA foreign_keys = OFF;")?;
    result.execute_batch(include_str!("projection_temp.sql"))?;
    result.execute_batch("BEGIN")?;
    // Evidence is immutable; only cited rows are needed by the private replay and citations.
    result.execute_batch("CREATE TABLE evidence (id INTEGER PRIMARY KEY, source TEXT, scope TEXT, location TEXT, field TEXT, start INTEGER, end INTEGER, quote TEXT, captured_at TEXT, source_updated_at TEXT, trust_class TEXT)")?;
    copy(
        db,
        &result,
        "justification",
        "SELECT * FROM justification WHERE id IN (SELECT value FROM json_each(?1))",
        std::slice::from_ref(&visible),
    )?;
    copy(
        db,
        &result,
        "justification_span",
        "SELECT * FROM justification_span WHERE justification_id IN (SELECT value FROM json_each(?1))",
        &[visible],
    )?;
    let claims = ids(&result, "SELECT DISTINCT claim_id FROM justification", &[])?;
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
        &[],
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
        &[],
    )?;
    copy(
        db,
        &result,
        "evidence",
        "SELECT id,source,scope,location,field,start,end,quote,captured_at,source_updated_at,trust_class FROM evidence WHERE id IN (SELECT value FROM json_each(?1))",
        &[evidence],
    )?;
    replay(&result)?;
    result.execute_batch("COMMIT")?;
    Ok((result, work))
}

// A component with exactly one required span has no competing event or alternative
// derivation: it opens once and remains observed. Batch this common case; replay all
// multi-span/alternative/replacement components through the lifecycle oracle.
fn replay(db: &Connection) -> Result<()> {
    use rastro_core::claims::{Support, occurrence_id_with_ordinal, revision_with_links};
    use rusqlite::functions::FunctionFlags;
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    db.create_scalar_function("single_revision", 1, flags, |ctx| {
        let support = Support {
            evidence_id: 0,
            portable_key: ctx.get::<String>(0)?,
            field: String::new(),
            start: 0,
            end: 0,
            extraction_rule: String::new(),
        };
        Ok(revision_with_links(
            "observed",
            std::slice::from_ref(&support),
            &[],
        ))
    })?;
    db.create_scalar_function("single_occurrence", 3, flags, |ctx| {
        Ok(occurrence_id_with_ordinal(
            &ctx.get::<String>(0)?,
            &ctx.get::<String>(1)?,
            &ctx.get::<String>(2)?,
            0,
        ))
    })?;
    let step = |_name: &str, sql: &str| db.execute_batch(sql);
    step(
        "single",
        "CREATE TEMP TABLE single AS SELECT c.subject AS subject, c.predicate AS predicate, c.id AS claim, j.id AS justification, s.evidence_id AS evidence_id, s.portable_key AS portable_key, s.field AS field, e.source AS source, e.scope AS scope, e.location AS location, e.captured_at AS captured_at, e.source_updated_at AS source_updated_at FROM claim c JOIN justification j ON j.claim_id = c.id JOIN justification_span s ON s.justification_id = j.id JOIN evidence e ON e.id = s.evidence_id GROUP BY c.subject, c.predicate HAVING count(*) = 1",
    )?;
    step(
        "idx",
        "CREATE INDEX single_by_claim ON single (claim); CREATE INDEX single_by_slot ON single (subject, predicate)",
    )?;
    step(
        "claim",
        "UPDATE claim SET status = 'observed', revision = single_revision(single.portable_key), known_since = single.captured_at FROM single WHERE claim.id = single.claim",
    )?;
    step(
        "just",
        "UPDATE justification SET current = 1 WHERE id IN (SELECT justification FROM single)",
    )?;
    step(
        "occ",
        "INSERT INTO occurrence (id, justification_id, source, scope, location, field, captured_at, source_updated_at, portable_key, evidence_id) SELECT single_occurrence(justification, captured_at, portable_key), justification, source, scope, location, field, captured_at, source_updated_at, portable_key, evidence_id FROM single",
    )?;
    let affected = db
        .prepare("SELECT DISTINCT subject, predicate FROM claim WHERE NOT EXISTS (SELECT 1 FROM single WHERE single.subject = claim.subject AND single.predicate = claim.predicate)")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<BTreeSet<_>, _>>()?;
    db.execute_batch("DROP TABLE single")?;
    lifecycle::refresh(db, &affected)
}

/// Entries currently cached for one database file (tests only).
#[doc(hidden)]
pub fn cache_len(database: &std::path::Path) -> usize {
    let database = std::fs::canonicalize(database).unwrap_or_else(|_| database.to_path_buf());
    let database = database.to_string_lossy();
    let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache
        .iter()
        .filter(|(k, _, _)| k.database == database)
        .count()
}
