use anyhow::{Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use rastro_core::claims::{Checkpoint, ClaimView, Entity, Object, ObservedClaim};
use rastro_core::{
    Appended, COVERAGE_WINDOWS_SHOWN, CompleteCheck, Counts, Coverage, CoverageRecord,
    CoverageState, CoverageWindow, Evidence, Hit, Pair, ReadStorage, Refusal, RunEnd, Scope,
    Snapshot, Source, Storage, TrustClass, UnfinishedRun,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::path::Path;

/// How long a connection waits on another process's lock before giving up.
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// What `PRAGMA user_version` 1 and 2 meant, before migrations were tracked by name. Frozen.
const LEGACY: [&str; 2] = ["202611041200_initial", "202611041500_evidence_insert_guard"];

mod identity;
mod knowledge;
/// `(name, sql)`, named `YYYYMMDDHHmm_<what>` so migrations from different branches
/// order by time. Never edit one that has shipped; add a new one.
mod lifecycle;
mod scoped;
#[doc(hidden)]
pub use scoped::{cache_len as scoped_cache_len, clear_cache as clear_scoped_cache};

/// The migration recording when an intent's contribution ended; an open that applies it replays
/// the stored intents once all migrations are in.
const INTENT_END: &str = "202611081000_intent_justification_end";
const MIGRATIONS: [(&str, &str); 10] = [
    (
        "202611041200_initial",
        include_str!("../migrations/202611041200_initial.sql"),
    ),
    (
        "202611041500_evidence_insert_guard",
        include_str!("../migrations/202611041500_evidence_insert_guard.sql"),
    ),
    (
        "202611041700_evidence_full_text_index",
        include_str!("../migrations/202611041700_evidence_full_text_index.sql"),
    ),
    (
        "202611050900_run_window",
        include_str!("../migrations/202611050900_run_window.sql"),
    ),
    (
        "202611051100_run_evidence",
        include_str!("../migrations/202611051100_run_evidence.sql"),
    ),
    (
        "202611051300_coverage_pair_indexes",
        include_str!("../migrations/202611051300_coverage_pair_indexes.sql"),
    ),
    (
        "202611061000_claims_projection",
        include_str!("../migrations/202611061000_claims_projection.sql"),
    ),
    (
        "202611061200_knowledge_v2",
        include_str!("../migrations/202611061200_knowledge_v2.sql"),
    ),
    (
        "202611071000_owner_intents",
        include_str!("../migrations/202611071000_owner_intents.sql"),
    ),
    (
        INTENT_END,
        include_str!("../migrations/202611081000_intent_justification_end.sql"),
    ),
];

fn stamp(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Millis, true)
}
fn parse_time(text: String) -> Result<DateTime<Utc>> {
    Ok(text.parse()?)
}

pub struct SqliteStorage {
    connection: Connection,
}
impl SqliteStorage {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with(path, &MIGRATIONS)
    }
    /// Opens with an explicit migration list. Each pending migration applies once, in name
    /// order, in its own transaction. A database holding a migration not in the list is refused.
    pub fn open_with(path: &Path, migrations: &[(&str, &str)]) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            rastro_core::files::private_directory(parent)?;
        }
        let _file = rastro_core::files::private_file(path)?;
        let mut connection = Connection::open(path)?;
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        // WAL lets a read-only process read while this one writes. It sticks to the file, so
        // this is a no-op once set; an older database switches on its next writing open. If
        // the file can't switch (say, a filesystem without shared memory) the open still
        // works, says so on stderr, and the next writing open tries again.
        let mode: String =
            connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            eprintln!("rastro: database stays in {mode} mode; reads may wait for a sync");
        }
        let mut migrations = migrations.to_vec();
        migrations.sort_by_key(|(name, _)| *name);
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let tracked: bool = transaction.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = 'schema_migrations')",
            [],
            |row| row.get(0),
        )?;
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                name TEXT PRIMARY KEY, applied_at TEXT NOT NULL)",
        )?;
        if !tracked {
            // A database from before migrations were tracked by name: user_version N
            // means the first N migrations are in.
            let version: i64 =
                transaction.query_row("PRAGMA user_version", [], |row| row.get(0))?;
            let meant: &[&str] = match version {
                0 => &[],
                1 => &LEGACY[..1],
                2 => &LEGACY[..2],
                other => bail!("unsupported legacy database (user_version {other})"),
            };
            for name in meant {
                mark(&transaction, name)?;
            }
        }
        let applied: Vec<String> = transaction
            .prepare("SELECT name FROM schema_migrations")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        if let Some(unknown) = applied
            .iter()
            .find(|name| !migrations.iter().any(|(known, _)| known == name))
        {
            bail!("database has migration {unknown}, which this version of rastro doesn't know");
        }
        transaction.commit()?;
        let mut replay = false;
        for (name, sql) in migrations {
            let transaction =
                connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let done: bool = transaction.query_row(
                "SELECT EXISTS (SELECT 1 FROM schema_migrations WHERE name = ?1)",
                [name],
                |row| row.get(0),
            )?;
            if !done {
                transaction.execute_batch(sql)?;
                replay |= name == INTENT_END;
                mark(&transaction, name)?;
            }
            transaction.commit()?;
        }
        if replay {
            // The intents already stored replay once, after every migration, so today's replay
            // meets today's schema: what they ended reads as history right after the upgrade,
            // without waiting for a rebuild. A database with none publishes nothing.
            let transaction =
                connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let intents: bool =
                transaction.query_row("SELECT EXISTS (SELECT 1 FROM intent)", [], |r| r.get(0))?;
            if intents {
                identity::apply(&transaction)?;
                lifecycle::publish(&transaction)?;
            }
            transaction.commit()?;
        }
        Ok(Self { connection })
    }
    /// Opens an existing database for reading only: the database file is never created and
    /// nothing is migrated. `None` when there is no database file. (SQLite may still create
    /// the `-wal`/`-shm` side files of a WAL database; where it can't, the read is refused
    /// as `Refusal::NotReadable`.)
    pub fn open_read_only(path: &Path) -> Result<Option<Self>> {
        if !path.try_exists()? {
            return Ok(None);
        }
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(Some(Self { connection }))
    }
}
/// Reads through one connection; for a read-only open, inside one transaction.
struct View<'a>(&'a Connection);

/// Claims no justification holds any more: no evidence justification and no owner intent.
pub(crate) const UNJUSTIFIED: &str = "SELECT id FROM claim c WHERE NOT EXISTS (SELECT 1 FROM justification j WHERE j.claim_id = c.id) \
    AND NOT EXISTS (SELECT 1 FROM intent_justification i WHERE i.claim_id = c.id)";

/// The observations of a pair past an extractor version's checkpoint (`?1` source, `?2` scope,
/// `?3` extractor, `?4` version, `?5` vocabulary version or NULL for any): what the next run
/// reads and what makes the pair `behind`.
const PENDING_EVIDENCE: &str = "FROM evidence e
    LEFT JOIN extraction_checkpoint c ON c.extractor = ?3 AND c.extractor_version = ?4
        AND c.source = e.source AND c.scope = e.scope
        AND (?5 IS NULL OR c.vocabulary_version = ?5)
    WHERE e.source = ?1 AND e.scope = ?2 AND (c.source IS NULL OR e.id > c.last_evidence_id)";

impl ReadStorage for SqliteStorage {
    fn read<T>(&mut self, work: impl FnOnce(&dyn Snapshot) -> Result<T>) -> Result<T> {
        self.read_view(|view| work(view))
    }
}
impl SqliteStorage {
    fn read_view<T>(&mut self, work: impl FnOnce(&View) -> Result<T>) -> Result<T> {
        // A deferred transaction pins its snapshot at the first read, which is the migration
        // check below, so everything `work` reads is as of that moment.
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
            .map_err(unreadable)?;
        check_migrations(&transaction, &MIGRATIONS).map_err(unreadable)?;
        work(&View(&transaction))
    }

    /// A scoped `about` under an explicit work budget, measured in visible justification
    /// spans (tests only; the trait method uses the default budget).
    #[doc(hidden)]
    pub fn about_scoped_within(
        &mut self,
        entity: &str,
        options: &rastro_core::knowledge::AboutOptions,
        scopes: &[Scope],
        budget: usize,
    ) -> Result<Vec<rastro_core::knowledge::ClaimSummary>> {
        self.read_view(|view| {
            Ok(view
                .about_view_within(entity, options, scopes, budget)?
                .claims
                .into_iter()
                .map(|c| c.summary)
                .collect())
        })
    }
}
/// A read that failed because SQLite needed to write (to create the WAL side files) becomes a
/// typed refusal; anything else passes through.
fn unreadable(error: impl Into<anyhow::Error>) -> anyhow::Error {
    let error = error.into();
    let why = match error.downcast_ref::<rusqlite::Error>() {
        Some(rusqlite::Error::SqliteFailure(failure, _))
            if matches!(
                failure.code,
                rusqlite::ErrorCode::ReadOnly | rusqlite::ErrorCode::CannotOpen
            ) =>
        {
            error.to_string()
        }
        _ => return error,
    };
    Refusal::NotReadable(why).into()
}
/// Refuses a database that is behind this build (pending) or ahead of it (unknown).
fn check_migrations(connection: &Connection, known: &[(&str, &str)]) -> Result<()> {
    let tracked: bool = connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = 'schema_migrations')",
        [],
        |row| row.get(0),
    )?;
    let applied: Vec<String> = if tracked {
        connection
            .prepare("SELECT name FROM schema_migrations")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?
    } else {
        Vec::new()
    };
    if let Some(unknown) = applied
        .iter()
        .find(|name| !known.iter().any(|(k, _)| k == name))
    {
        return Err(Refusal::UnknownMigration(unknown.clone()).into());
    }
    let mut pending: Vec<String> = known
        .iter()
        .filter(|(name, _)| !applied.iter().any(|a| a == name))
        .map(|(name, _)| name.to_string())
        .collect();
    pending.sort();
    if !pending.is_empty() {
        return Err(Refusal::PendingMigrations(pending).into());
    }
    Ok(())
}
fn mark(connection: &Connection, name: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO schema_migrations (name, applied_at) VALUES (?1, ?2)",
        params![name, stamp(Utc::now())],
    )?;
    Ok(())
}
impl Storage for SqliteStorage {
    fn project_sources(&mut self, sources: &[Source]) -> Result<()> {
        let transaction = self.connection.transaction()?;
        for source in sources {
            let selector = serde_json::to_string(&source.selector)?;
            transaction.execute(
                "INSERT INTO sources (name, connector, selector, scope, freshness_rule, window_start, keep_original)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(name) DO UPDATE SET
                 connector=excluded.connector, selector=excluded.selector, scope=excluded.scope,
                 freshness_rule=excluded.freshness_rule, window_start=excluded.window_start,
                 keep_original=excluded.keep_original",
                params![source.name, source.connector, selector, source.scope.0, source.freshness_rule, source.window_start.to_rfc3339(), source.keep_original],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
    fn snapshot<T>(&self, work: impl FnOnce(&dyn Snapshot) -> Result<T>) -> Result<T> {
        // One read transaction: the pair list and every answer come from one snapshot.
        let transaction = self.connection.unchecked_transaction()?;
        work(&View(&transaction))
    }
    fn start_run(
        &mut self,
        source: &Source,
        plugin: (&str, &str),
        started_at: DateTime<Utc>,
        window: (DateTime<Utc>, DateTime<Utc>),
    ) -> Result<i64> {
        self.connection.execute(
            "INSERT INTO runs (source, scope, plugin_id, plugin_version, started_at, status,
             window_start, window_end) VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6, ?7)",
            params![
                source.name,
                source.scope.0,
                plugin.0,
                plugin.1,
                stamp(started_at),
                stamp(window.0),
                stamp(window.1)
            ],
        )?;
        Ok(self.connection.last_insert_rowid())
    }
    fn append_evidence(&mut self, run_id: i64, e: &Evidence) -> Result<Appended> {
        let transaction = self.connection.transaction()?;
        // The latest observation of this field at this location; identical content is linked,
        // never rewritten.
        let latest: Option<(i64, u64, u64, String, String)> = transaction
            .query_row(
                "SELECT id, start, end, quote, trust_class FROM evidence
                 WHERE source=?1 AND scope=?2 AND location=?3 AND field=?4
                 ORDER BY id DESC LIMIT 1",
                params![e.source, e.scope.0, e.location, e.field],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get::<_, i64>(1)? as u64,
                        row.get::<_, i64>(2)? as u64,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        if let Some((id, start, end, quote, trust)) = latest
            && (start, end, &quote, trust.as_str())
                == (e.start, e.end, &e.quote, e.trust_class.as_str())
        {
            transaction.execute(
                "INSERT OR IGNORE INTO run_evidence (run_id, evidence_id) VALUES (?1, ?2)",
                params![run_id, id],
            )?;
            transaction.commit()?;
            return Ok(Appended::Unchanged(id));
        }
        // Capture times never go backwards within a pair, so id order is replay order and an
        // extraction checkpoint can't skip an observation appended under an earlier clock.
        let floor: Option<String> = transaction.query_row(
            "SELECT max(captured_at) FROM evidence WHERE source = ?1 AND scope = ?2",
            params![e.source, e.scope.0],
            |row| row.get(0),
        )?;
        let captured_at = match floor.map(parse_time).transpose()? {
            Some(floor) if floor > e.captured_at => floor,
            _ => e.captured_at,
        };
        transaction.execute(
            "INSERT INTO evidence (source, scope, location, field, start, end, quote, captured_at,
             source_updated_at, trust_class, original) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                e.source,
                e.scope.0,
                e.location,
                e.field,
                e.start as i64,
                e.end as i64,
                e.quote,
                stamp(captured_at),
                e.source_updated_at.map(stamp),
                e.trust_class.as_str(),
                e.original.as_ref().map(|o| o.to_string())
            ],
        )?;
        let id = transaction.last_insert_rowid();
        transaction.execute(
            "INSERT INTO evidence_fts (rowid, quote) VALUES (?1, ?2)",
            params![id, e.quote],
        )?;
        transaction.execute(
            "INSERT INTO run_evidence (run_id, evidence_id) VALUES (?1, ?2)",
            params![run_id, id],
        )?;
        transaction.commit()?;
        Ok(Appended::New(id))
    }
    fn record_coverage(&mut self, r: &CoverageRecord) -> Result<()> {
        self.connection.execute(
            "INSERT INTO coverage (run_id, source, scope, window_start, window_end, selection,
             state, reason, retry_at, checked_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                r.run_id,
                r.source,
                r.scope.0,
                stamp(r.window.0),
                stamp(r.window.1),
                r.selection,
                r.state.as_str(),
                r.reason,
                r.retry_at.map(stamp),
                stamp(r.checked_at)
            ],
        )?;
        Ok(())
    }
    fn finish_run(&mut self, run_id: i64, end: &RunEnd) -> Result<()> {
        self.connection.execute(
            "UPDATE runs SET finished_at=?2, status=?3, reason=?4, submitted=?5, accepted=?6,
             rejected=?7, unchanged=?8 WHERE id=?1",
            params![
                run_id,
                stamp(end.finished_at),
                end.status,
                end.reason,
                end.counts.submitted as i64,
                end.counts.accepted as i64,
                end.counts.rejected as i64,
                end.counts.unchanged as i64
            ],
        )?;
        Ok(())
    }
    fn last_complete_check(&self, source: &Source) -> Result<Option<CompleteCheck>> {
        View(&self.connection).last_complete_check(&source.name, &source.scope)
    }
    fn interrupt_unfinished_runs(&mut self, at: DateTime<Utc>) -> Result<usize> {
        let transaction = self.connection.transaction()?;
        let at = stamp(at);
        // The window each lost run was bound to is "couldn't look: interrupted".
        transaction.execute(
            "INSERT INTO coverage (run_id, source, scope, window_start, window_end, selection,
             state, reason, checked_at)
             SELECT id, source, scope, window_start, window_end, 'all', 'couldn''t look',
             'interrupted', ?1 FROM runs
             WHERE finished_at IS NULL AND window_start IS NOT NULL AND role = 'connector'",
            [&at],
        )?;
        let marked = transaction.execute(
            "UPDATE runs SET status='failed', reason='interrupted', finished_at=?1
             WHERE finished_at IS NULL",
            [&at],
        )?;
        transaction.commit()?;
        Ok(marked)
    }
    fn rebuild_index(&mut self) -> Result<usize> {
        let transaction = self.connection.transaction()?;
        // The full-text migration is the one definition of the index; run it again.
        transaction.execute_batch("DROP TABLE IF EXISTS evidence_fts")?;
        transaction.execute_batch(include_str!(
            "../migrations/202611041700_evidence_full_text_index.sql"
        ))?;
        let indexed: i64 =
            transaction.query_row("SELECT count(*) FROM evidence", [], |row| row.get(0))?;
        transaction.commit()?;
        Ok(indexed as usize)
    }
    fn search(&self, query: &str, limit: usize, history: bool) -> Result<Vec<Hit>> {
        View(&self.connection).search(query, limit, history, &[])
    }
    fn bind_extractor(&mut self, source: &str, extractor: Option<&str>) -> Result<()> {
        self.connection.execute(
            "UPDATE sources SET extractor = ?2 WHERE name = ?1",
            params![source, extractor],
        )?;
        Ok(())
    }
    fn start_extraction(
        &mut self,
        source: &Source,
        plugin: (&str, &str),
        started_at: DateTime<Utc>,
    ) -> Result<i64> {
        self.connection.execute(
            "INSERT INTO runs (source, scope, plugin_id, plugin_version, started_at, status, role)
             VALUES (?1, ?2, ?3, ?4, ?5, 'running', 'extractor')",
            params![
                source.name,
                source.scope.0,
                plugin.0,
                plugin.1,
                stamp(started_at)
            ],
        )?;
        Ok(self.connection.last_insert_rowid())
    }
    fn pending_evidence(
        &self,
        extractor: (&str, &str),
        source: &Source,
    ) -> Result<Vec<(i64, Evidence)>> {
        self.evidence_after_checkpoint(extractor, source, None)
    }
    fn pending_evidence_for_vocabulary(
        &self,
        extractor: (&str, &str),
        vocabulary_version: &str,
        source: &Source,
    ) -> Result<Vec<(i64, Evidence)>> {
        self.evidence_after_checkpoint(extractor, source, Some(vocabulary_version))
    }
    fn location_context(
        &self,
        source: &Source,
        locations: &[String],
        high_water: (DateTime<Utc>, i64),
    ) -> Result<Vec<(i64, Evidence)>> {
        let mut statement = self.connection.prepare(
            "SELECT id, location, field, start, end, quote, trust_class, captured_at,
             source_updated_at FROM (
                 SELECT e.*, row_number() OVER (PARTITION BY e.location, e.field
                     ORDER BY e.captured_at DESC, e.id DESC) AS newest
                 FROM evidence e
                 WHERE e.source = ?1 AND e.scope = ?2 AND e.trust_class = 'structured'
                     AND e.location IN (SELECT value FROM json_each(?3))
                     AND (e.captured_at, e.id) <= (?4, ?5)
             ) WHERE newest = 1 ORDER BY captured_at, id",
        )?;
        let locations = serde_json::to_string(locations)?;
        let rows = statement.query_map(
            params![
                source.name,
                source.scope.0,
                locations,
                stamp(high_water.0),
                high_water.1
            ],
            observation,
        )?;
        rows.map(|row| row?.into_evidence(source)).collect()
    }
    fn record_claims(
        &mut self,
        run_id: i64,
        extractor: (&str, &str),
        claims: &[ObservedClaim],
        checkpoint: Option<&Checkpoint>,
    ) -> Result<Counts> {
        use rastro_core::claims::{derivation_key, justification_id};
        let transaction = self.connection.transaction()?;
        let mut affected = std::collections::BTreeSet::new();
        // Retirement happens only on a successful complete run, inside this publication.
        let (source, scope): (String, String) = transaction.query_row(
            "SELECT source, scope FROM runs WHERE id = ?1",
            [run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let vocabulary_version = checkpoint.map(|c| c.vocabulary_version.as_str());
        let retired = transaction.prepare(
            "SELECT DISTINCT c.subject, c.predicate, e.location, s.field FROM justification j JOIN claim c ON c.id = j.claim_id
             JOIN justification_span s ON s.justification_id = j.id JOIN evidence e ON e.id = s.evidence_id
             WHERE j.extractor = ?1 AND (j.extractor_version != ?2 OR (?5 IS NOT NULL AND j.vocabulary_version != ?5)) AND e.source = ?3 AND e.scope = ?4")?
            .query_map(params![extractor.0, extractor.1, source, scope, vocabulary_version], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        affected.extend(retired);
        transaction.execute("DELETE FROM occurrence WHERE justification_id IN (
            SELECT j.id FROM justification j JOIN justification_span s ON s.justification_id = j.id
            JOIN evidence e ON e.id = s.evidence_id WHERE j.extractor = ?1 AND (j.extractor_version != ?2 OR (?5 IS NOT NULL AND j.vocabulary_version != ?5))
            AND e.source = ?3 AND e.scope = ?4)", params![extractor.0, extractor.1, source, scope, vocabulary_version])?;
        transaction.execute(
            "DELETE FROM justification_span WHERE justification_id IN (
            SELECT j.id FROM justification j WHERE j.extractor = ?1 AND (j.extractor_version != ?2 OR (?5 IS NOT NULL AND j.vocabulary_version != ?5)))
            AND evidence_id IN (SELECT id FROM evidence WHERE source = ?3 AND scope = ?4)",
            params![extractor.0, extractor.1, source, scope, vocabulary_version],
        )?;
        transaction.execute(
            "DELETE FROM justification WHERE NOT EXISTS
            (SELECT 1 FROM justification_span s WHERE s.justification_id = justification.id)",
            [],
        )?;
        transaction.execute("DELETE FROM extraction_checkpoint WHERE extractor = ?1 AND extractor_version != ?2 AND source = ?3 AND scope = ?4",
            params![extractor.0, extractor.1, source, scope])?;
        let mut counts = Counts::default();
        for claim in claims {
            let mut entities = vec![claim.subject.clone()];
            let (object_entity, object_value, object_datatype) = match &claim.object {
                Object::Entity {
                    id,
                    identifier,
                    entity_type,
                } => {
                    entities.push(Entity::new(identifier.clone(), entity_type.clone()));
                    (Some(id), None, None)
                }
                Object::Literal { value, datatype } => (None, Some(value), Some(datatype)),
            };
            for entity in &entities {
                transaction.execute(
                    "INSERT OR IGNORE INTO entity (id, identifier) VALUES (?1, ?2)",
                    params![entity.id, entity.identifier],
                )?;
            }
            transaction.execute(
                "INSERT INTO claim (id, subject, predicate, object_entity, object_value,
                 object_datatype, status, revision, scalar, type_ancestors) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'observed', '', ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET scalar = excluded.scalar, type_ancestors = excluded.type_ancestors",
                params![claim.id, claim.subject.id, claim.predicate, object_entity, object_value, object_datatype, claim.scalar,
                    serde_json::to_string(&claim.type_ancestors.iter().collect::<std::collections::BTreeSet<_>>())?])?;
            let derivation = derivation_key(&claim.supports);
            let id = justification_id(&claim.id, extractor.0, &derivation);
            let rule = claim
                .supports
                .iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("empty justification"))?
                .extraction_rule
                .clone();
            let added = transaction.execute(
                "INSERT OR IGNORE INTO justification (id, claim_id, kind, extractor, extractor_version,
                 vocabulary_version, derivation_key, extraction_rule, run_id)
                 VALUES (?1, ?2, 'provenance', ?3, ?4, ?5, ?6, ?7, ?8)",
                params![id, claim.id, extractor.0, extractor.1, checkpoint.map_or("", |c| &c.vocabulary_version), derivation, rule, run_id])?;
            if added == 0 {
                transaction.execute(
                    "UPDATE justification SET vocabulary_version = ?2, run_id = ?3 WHERE id = ?1",
                    params![id, checkpoint.map_or("", |c| &c.vocabulary_version), run_id],
                )?;
            }
            let mut spans_added = 0;
            for span in &claim.supports {
                let location: String = transaction.query_row(
                    "SELECT location FROM evidence WHERE id = ?1",
                    [span.evidence_id],
                    |r| r.get(0),
                )?;
                affected.insert((
                    claim.subject.id.clone(),
                    claim.predicate.clone(),
                    location,
                    span.field.clone(),
                ));
                spans_added += transaction.execute("INSERT OR IGNORE INTO justification_span
                    (justification_id, evidence_id, portable_key, field, start, end) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![id, span.evidence_id, span.portable_key, span.field, span.start as i64, span.end as i64])?;
            }
            if added == 0 && spans_added == 0 {
                counts.unchanged += 1;
            } else {
                counts.accepted += 1;
            }
        }
        // Claims removed by retirement disappear, together with dangling presentation links.
        // An intent-backed claim stays: the intents, not the evidence, hold it.
        transaction.execute(&format!("DELETE FROM claim_link WHERE from_claim IN ({UNJUSTIFIED}) OR to_claim IN ({UNJUSTIFIED})"), [])?;
        transaction.execute(
            &format!("DELETE FROM claim WHERE id IN ({UNJUSTIFIED})"),
            [],
        )?;
        if lifecycle::staging(&transaction)? {
            let components = affected
                .iter()
                .map(|(subject, predicate, _, _)| (subject.clone(), predicate.clone()))
                .collect();
            lifecycle::refresh(&transaction, &components)?;
        } else {
            lifecycle::refresh_groups(&transaction, &affected)?;
        }
        transaction.execute("DELETE FROM entity WHERE id NOT IN (SELECT subject FROM claim UNION SELECT object_entity FROM claim WHERE object_entity IS NOT NULL)", [])?;
        if let Some(c) = checkpoint {
            transaction.execute(
                "INSERT INTO extraction_checkpoint (extractor, extractor_version, source, scope,
                 last_captured_at, last_portable_key, last_evidence_id, vocabulary_version) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT DO UPDATE SET
                 last_captured_at = CASE WHEN excluded.last_evidence_id >= last_evidence_id THEN excluded.last_captured_at ELSE last_captured_at END,
                 last_portable_key = CASE WHEN excluded.last_evidence_id >= last_evidence_id THEN excluded.last_portable_key ELSE last_portable_key END,
                 last_evidence_id = max(last_evidence_id, excluded.last_evidence_id),
                 vocabulary_version = excluded.vocabulary_version",
                params![c.extractor, c.extractor_version, c.source, c.scope.0, stamp(c.last.0),
                    c.last_portable_key, c.last.1, c.vocabulary_version])?;
        }
        if !lifecycle::staging(&transaction)? {
            lifecycle::publish(&transaction)?;
        }
        transaction.commit()?;
        Ok(counts)
    }
    fn extraction_sources(&self) -> Result<Vec<(Source, String)>> {
        View(&self.connection).extraction_sources(&[])
    }
    fn begin_claims_rebuild(&mut self) -> Result<()> {
        lifecycle::begin_rebuild(&self.connection)
    }
    fn finish_claims_rebuild(&mut self, publish: bool) -> Result<usize> {
        lifecycle::finish_rebuild(&mut self.connection, publish)
    }
    fn issue_intents<T>(
        &mut self,
        sees: &[Scope],
        plan: impl FnOnce(&dyn Snapshot) -> Result<(Vec<rastro_core::identity::Intent>, T)>,
    ) -> Result<(Vec<rastro_core::identity::Intent>, T)> {
        // One immediate transaction: the plan, the checks, the rows and the claims they back
        // see and write one state.
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (intents, out) = plan(&View(&transaction))?;
        let stored = identity::insert_all(&transaction, sees, intents)?;
        identity::apply(&transaction)?;
        // An intent write changes the projection, so it publishes like any other projection write.
        lifecycle::publish(&transaction)?;
        transaction.commit()?;
        Ok((stored, out))
    }
    fn carry_over_pair(&mut self, source: &Source, extractor: &str) -> Result<usize> {
        let transaction = self.connection.transaction()?;
        let carried =
            lifecycle::carry_over(&transaction, &source.name, &source.scope.0, extractor)?;
        transaction.commit()?;
        Ok(carried)
    }
}

/// One evidence row as the extraction queries select it: id, location, field, start, end,
/// quote, trust class, capture time and source update time.
struct Observation {
    id: i64,
    text: [String; 5],
    span: [u64; 2],
    updated: Option<String>,
}
fn observation(row: &rusqlite::Row) -> rusqlite::Result<Observation> {
    Ok(Observation {
        id: row.get(0)?,
        text: [
            row.get(1)?,
            row.get(2)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
        ],
        span: [row.get::<_, i64>(3)? as u64, row.get::<_, i64>(4)? as u64],
        updated: row.get(8)?,
    })
}
impl Observation {
    fn into_evidence(self, source: &Source) -> Result<(i64, Evidence)> {
        let [location, field, quote, trust, captured] = self.text;
        let [start, end] = self.span;
        let evidence = Evidence {
            source: source.name.clone(),
            scope: source.scope.clone(),
            location,
            field,
            start,
            end,
            quote,
            trust_class: trust_class(&trust)?,
            captured_at: parse_time(captured)?,
            source_updated_at: self.updated.map(parse_time).transpose()?,
            original: None,
        };
        Ok((self.id, evidence))
    }
}

fn trust_class(text: &str) -> Result<TrustClass> {
    TrustClass::parse(text).ok_or_else(|| anyhow::anyhow!("invalid trust class: {text}"))
}

impl View<'_> {
    fn last_complete_check(&self, name: &str, scope: &Scope) -> Result<Option<CompleteCheck>> {
        let row: Option<(String, String)> = self
            .0
            .query_row(
                "SELECT window_end, checked_at FROM coverage
                 WHERE source=?1 AND scope=?2 AND state IN ('found', 'empty')
                 ORDER BY id DESC LIMIT 1",
                params![name, scope.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(end, at)| {
            Ok(CompleteCheck {
                window_end: parse_time(end)?,
                checked_at: parse_time(at)?,
            })
        })
        .transpose()
    }
}
impl View<'_> {
    fn about_view_within(
        &self,
        entity: &str,
        options: &rastro_core::knowledge::AboutOptions,
        scopes: &[Scope],
        budget: usize,
    ) -> Result<rastro_core::knowledge::AboutClaims> {
        if scopes.is_empty() {
            return self.about_view(entity, options, budget);
        }
        let projection =
            scoped::projection(self.0, scopes, scoped::Target::Entity(entity), budget)?;
        let projection = projection.lock().unwrap_or_else(|e| e.into_inner());
        View(&projection).about_view(entity, options, budget)
    }
}
impl Snapshot for View<'_> {
    fn about_view(
        &self,
        entity: &str,
        options: &rastro_core::knowledge::AboutOptions,
        scopes: &[Scope],
    ) -> Result<rastro_core::knowledge::AboutClaims> {
        self.about_view_within(entity, options, scopes, scoped::WORK_BUDGET)
    }
    fn explain_scoped(
        &self,
        claim: &str,
        options: &rastro_core::knowledge::WhyOptions,
        scopes: &[Scope],
    ) -> Result<Option<rastro_core::knowledge::Explanation>> {
        if scopes.is_empty() {
            return self.explanation(claim, options);
        }
        let projection = scoped::projection(
            self.0,
            scopes,
            scoped::Target::Claim(claim),
            scoped::WORK_BUDGET,
        )?;
        let projection = projection.lock().unwrap_or_else(|e| e.into_inner());
        View(&projection).explanation(claim, options)
    }

    fn explain(
        &self,
        claim: &str,
        options: &rastro_core::knowledge::WhyOptions,
    ) -> Result<Option<rastro_core::knowledge::Explanation>> {
        self.explanation(claim, options)
    }

    fn intents(&self) -> Result<Vec<rastro_core::identity::Intent>> {
        identity::load(self.0)
    }
    fn resolve_entity(&self, form: &str) -> Result<Option<(String, String)>> {
        let Some(id) = self.resolve(form)? else {
            return Ok(None);
        };
        let identifier: String =
            self.0
                .query_row("SELECT identifier FROM entity WHERE id = ?1", [&id], |r| {
                    r.get(0)
                })?;
        Ok(Some((id, identifier)))
    }
    fn stored_pairs(&self, scopes: &[Scope]) -> Result<Vec<Pair>> {
        // A source row or a run names a pair: every window and every evidence row comes from
        // a run, so those legs would add nothing. The rule is the one last stored for the
        // source (a removed source keeps its row).
        let mut statement = self.0.prepare(
            "SELECT p.source, p.scope, s.freshness_rule FROM (
                 SELECT name AS source, scope FROM sources
                 UNION SELECT source, scope FROM runs
             ) p JOIN sources s ON s.name = p.source
             WHERE (?1 IS NULL OR p.scope IN (SELECT value FROM json_each(?1)))
             ORDER BY p.source, p.scope",
        )?;
        let scopes = scope_parameter(scopes)?;
        let rows = statement.query_map([scopes], |row| {
            Ok(Pair {
                source: row.get(0)?,
                scope: Scope(row.get(1)?),
                freshness_rule: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
    fn coverage(&self, pair: &Pair, all_windows: bool) -> Result<Coverage> {
        // The overall state is judged over every window, in SQL; only the listing is capped.
        let (windows_total, any_found): (i64, bool) = self.0.query_row(
            "SELECT count(*), coalesce(max(state = 'found'), 0) FROM coverage
             WHERE source=?1 AND scope=?2",
            params![pair.source, pair.scope.0],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let limit = if all_windows {
            -1
        } else {
            COVERAGE_WINDOWS_SHOWN as i64
        };
        let mut statement = self.0.prepare(
            "SELECT window_start, window_end, state, selection, reason, retry_at, checked_at
             FROM coverage WHERE source=?1 AND scope=?2 ORDER BY id DESC LIMIT ?3",
        )?;
        let rows = statement.query_map(params![pair.source, pair.scope.0, limit], |row| {
            Ok((
                [row.get::<_, String>(0)?, row.get(1)?],
                [row.get::<_, String>(2)?, row.get(3)?],
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?;
        let mut windows = Vec::new();
        for row in rows {
            let ([from, to], [state, selection], reason, retry_at, checked_at) = row?;
            let state = match state.as_str() {
                "found" => CoverageState::Found,
                "empty" => CoverageState::Empty,
                "couldn't look" => CoverageState::CouldNotLook,
                other => bail!("invalid coverage state: {other}"),
            };
            windows.push(CoverageWindow {
                from: parse_time(from)?,
                to: parse_time(to)?,
                state,
                selection,
                reason,
                retry_at: retry_at.map(parse_time).transpose()?,
                checked_at: parse_time(checked_at)?,
            });
        }
        // The latest attempt (listed first) decides whether the source could be looked at;
        // otherwise any complete window that found evidence makes the source "found", and
        // it is "empty" only when every complete window was.
        let state = match windows.first() {
            None => CoverageState::NeverLooked,
            Some(latest) if latest.state == CoverageState::CouldNotLook => {
                CoverageState::CouldNotLook
            }
            Some(_) if any_found => CoverageState::Found,
            Some(_) => CoverageState::Empty,
        };
        let reason = windows.first().and_then(|w| w.reason.clone());
        Ok(Coverage {
            source: pair.source.clone(),
            scope: pair.scope.clone(),
            registered: false,
            state,
            last_checked: self
                .last_complete_check(&pair.source, &pair.scope)?
                .map(|c| c.checked_at),
            fresh_until: None,
            reason,
            windows_total: windows_total as usize,
            windows,
        })
    }
    /// Rank statistics span every scope in the global FTS corpus.
    fn search(
        &self,
        query: &str,
        limit: usize,
        history: bool,
        scopes: &[Scope],
    ) -> Result<Vec<Hit>> {
        let mut statement = self.0.prepare(
            "SELECT e.id, e.source, e.scope, e.location, e.field, e.start as i64, e.end as i64, e.quote,
             e.trust_class, e.captured_at, e.source_updated_at, e.original
             FROM evidence_fts JOIN evidence e ON e.id = evidence_fts.rowid
             WHERE evidence_fts MATCH ?1
             AND (?4 IS NULL OR e.scope IN (SELECT value FROM json_each(?4)))
             AND (?3 OR e.id = (SELECT max(l.id) FROM evidence l WHERE l.source = e.source
                  AND l.scope = e.scope AND l.location = e.location AND l.field = e.field))
             ORDER BY bm25(evidence_fts), e.id LIMIT ?2",
        )?;
        let mut highlight = self.0.prepare(
            "SELECT highlight(evidence_fts, 0, char(1), char(2)) FROM evidence_fts
             WHERE evidence_fts MATCH ?1 AND rowid = ?2",
        )?;
        let scopes = scope_parameter(scopes)?;
        let rows = statement.query_map(params![query, limit as i64, history, scopes], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                [
                    row.get::<_, String>(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ],
                [row.get::<_, i64>(5)? as u64, row.get::<_, i64>(6)? as u64],
                [row.get::<_, String>(7)?, row.get(8)?, row.get(9)?],
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
            ))
        })?;
        rows.map(|row| {
            let (
                id,
                [source, scope, location, field],
                [start, end],
                [quote, trust, captured],
                updated,
                original,
            ) = row?;
            // Only the rows kept pay for highlighting (it re-tokenizes the quote).
            let marked: String = highlight.query_row(params![query, id], |row| row.get(0))?;
            let matched = first_match(&quote, &marked);
            Ok(Hit {
                id,
                matched,
                evidence: Evidence {
                    source,
                    scope: Scope(scope),
                    location,
                    field,
                    start,
                    end,
                    quote,
                    trust_class: TrustClass::parse(&trust)
                        .ok_or_else(|| anyhow::anyhow!("invalid trust class: {trust}"))?,
                    captured_at: parse_time(captured)?,
                    source_updated_at: updated.map(parse_time).transpose()?,
                    original: original
                        .map(|text| serde_json::from_str(&text))
                        .transpose()?,
                },
            })
        })
        .collect()
    }
    fn extraction_facts(
        &self,
        source: &Source,
        extractor: &str,
        version: &str,
        vocabulary_version: &str,
    ) -> Result<rastro_core::runs::ExtractionFacts> {
        let (name, scope) = (&source.name, &source.scope.0);
        // Any version's unfinished run counts: only one extractor run of a pair is live.
        let running = self.0.query_row(
            "SELECT EXISTS (SELECT 1 FROM runs WHERE role = 'extractor'
             AND source = ?1 AND scope = ?2 AND plugin_id = ?3 AND finished_at IS NULL)",
            params![name, scope, extractor],
            |row| row.get(0),
        )?;
        // Only the latest finished run at this version decides: a retry that succeeds clears it.
        let failed = self.0.query_row(
            "SELECT COALESCE((SELECT status = 'failed' FROM runs
             WHERE role = 'extractor' AND source = ?1 AND scope = ?2 AND plugin_id = ?3
             AND plugin_version = ?4 AND finished_at IS NOT NULL
             ORDER BY id DESC LIMIT 1), 0)",
            params![name, scope, extractor, version],
            |row| row.get(0),
        )?;
        let pending = self.0.query_row(
            &format!("SELECT EXISTS (SELECT 1 {PENDING_EVIDENCE})"),
            params![name, scope, extractor, version, vocabulary_version],
            |row| row.get(0),
        )?;
        Ok(rastro_core::runs::ExtractionFacts {
            running,
            failed,
            pending,
        })
    }
    fn extraction_sources(&self, scopes: &[Scope]) -> Result<Vec<(Source, String)>> {
        let scopes = scope_parameter(scopes)?;
        let rows = self
            .0
            .prepare(
                "SELECT s.name, p.scope, s.extractor, s.connector, s.selector,
            s.freshness_rule, s.window_start, s.keep_original FROM sources s JOIN
            (SELECT name AS source, scope FROM sources UNION SELECT source, scope FROM evidence) p
            ON p.source = s.name WHERE s.extractor IS NOT NULL
            AND (?1 IS NULL OR p.scope IN (SELECT value FROM json_each(?1)))
            ORDER BY s.name, p.scope",
            )?
            .query_map([scopes], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, bool>(7)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(
                |(
                    name,
                    scope,
                    extractor,
                    connector,
                    selector,
                    freshness_rule,
                    window_start,
                    keep_original,
                )| {
                    Ok((
                        Source {
                            name,
                            scope: Scope(scope),
                            connector,
                            selector: serde_json::from_str(&selector)?,
                            freshness_rule,
                            window_start: parse_time(window_start)?,
                            keep_original,
                        },
                        extractor,
                    ))
                },
            )
            .collect()
    }
    fn unfinished_runs(&self, scopes: &[Scope]) -> Result<Vec<UnfinishedRun>> {
        let mut statement = self.0.prepare(
            "SELECT source, scope, window_start, window_end FROM runs
             WHERE finished_at IS NULL AND window_start IS NOT NULL AND role = 'connector'
             AND (?1 IS NULL OR scope IN (SELECT value FROM json_each(?1))) ORDER BY id",
        )?;
        let scopes = scope_parameter(scopes)?;
        let rows = statement.query_map([scopes], |row| {
            Ok([
                row.get::<_, String>(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
            ])
        })?;
        rows.map(|row| {
            let [source, scope, from, to] = row?;
            Ok(UnfinishedRun {
                source,
                scope: Scope(scope),
                window: (parse_time(from)?, parse_time(to)?),
            })
        })
        .collect()
    }
}

/// A bound JSON allowlist, or NULL to allow every scope.
fn scope_parameter(scopes: &[Scope]) -> Result<Option<String>> {
    if scopes.is_empty() {
        Ok(None)
    } else {
        Ok(Some(serde_json::to_string(scopes)?))
    }
}

/// Byte range in `quote` of the first match, from the index's own `highlight()` of it: the
/// tokenizer (stemming, diacritic folding) decides what matched, and markers say where. `None`
/// when the quote holds a marker byte itself, or the marked text isn't the quote with markers
/// added, since the markers can't then be trusted.
fn first_match(quote: &str, marked: &str) -> Option<(usize, usize)> {
    const OPEN: char = '\u{1}';
    const CLOSE: char = '\u{2}';
    if quote.contains([OPEN, CLOSE]) || marked.replace([OPEN, CLOSE], "") != quote {
        return None;
    }
    let start = marked.find(OPEN)?;
    let end = start + OPEN.len_utf8() + marked[start + 1..].find(CLOSE)?;
    // No marker precedes the first, so `start` is already an offset in the quote; the open
    // marker sits between `start` and the text.
    Some((start, end - OPEN.len_utf8()))
}

impl SqliteStorage {
    fn evidence_after_checkpoint(
        &self,
        extractor: (&str, &str),
        source: &Source,
        vocabulary_version: Option<&str>,
    ) -> Result<Vec<(i64, Evidence)>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT e.id, e.location, e.field, e.start, e.end, e.quote, e.trust_class,
             e.captured_at, e.source_updated_at {PENDING_EVIDENCE}
             ORDER BY e.captured_at, e.id"
        ))?;
        let rows = statement.query_map(
            params![
                source.name,
                source.scope.0,
                extractor.0,
                extractor.1,
                vocabulary_version
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    [row.get::<_, String>(1)?, row.get(2)?],
                    [row.get::<_, i64>(3)? as u64, row.get::<_, i64>(4)? as u64],
                    [row.get::<_, String>(5)?, row.get(6)?, row.get(7)?],
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )?;
        let mut evidence = rows
            .map(|row| {
                let (id, [location, field], [start, end], [quote, trust, captured], updated) = row?;
                let evidence = Evidence {
                    source: source.name.clone(),
                    scope: source.scope.clone(),
                    location,
                    field,
                    start,
                    end,
                    quote,
                    trust_class: trust_class(&trust)?,
                    captured_at: parse_time(captured)?,
                    source_updated_at: updated.map(parse_time).transpose()?,
                    original: None,
                };
                Ok((id, evidence))
            })
            .collect::<Result<Vec<_>>>()?;
        evidence.sort_by_key(|(id, e)| (e.captured_at, *id));
        Ok(evidence)
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod projection_schema_tests {
    use super::*;

    /// claims/R29: shadow tables match the active projection column shapes
    #[test]
    fn shadow_tables_match_the_active_projection_column_shapes() {
        let home = tempfile::tempdir().unwrap();
        let mut storage = SqliteStorage::open(&home.path().join("db")).unwrap();
        storage.begin_claims_rebuild().unwrap();
        let db = &storage.connection;
        let tables = db
            .prepare("SELECT name FROM sqlite_temp_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(tables.len(), 8);
        for table in tables {
            assert!(
                db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                    [&table],
                    |r| r.get::<_, bool>(0)
                )
                .unwrap()
            );
            let columns = |schema: &str| {
                db.prepare("SELECT name, type, \"notnull\", dflt_value, pk, hidden FROM pragma_table_xinfo(?1, ?2) ORDER BY cid").unwrap()
                    .query_map(params![table, schema], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, i64>(4)?, r.get::<_, i64>(5)?))).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
            };
            assert_eq!(columns("main"), columns("temp"), "{table}");
        }
        storage.finish_claims_rebuild(false).unwrap();
    }
}
