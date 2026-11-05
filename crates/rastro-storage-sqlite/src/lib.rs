use anyhow::{Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use rastro_core::claims::{Checkpoint, ClaimView, Entity, Object, ObservedClaim, SupportView, Why};
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

/// `(name, sql)`, named `YYYYMMDDHHmm_<what>` so migrations from different branches
/// order by time. Never edit one that has shipped; add a new one.
mod lifecycle;

const MIGRATIONS: [(&str, &str); 8] = [
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
                mark(&transaction, name)?;
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

impl ReadStorage for SqliteStorage {
    fn read<T>(&mut self, work: impl FnOnce(&dyn Snapshot) -> Result<T>) -> Result<T> {
        // A deferred transaction pins its snapshot at the first read, which is the migration
        // check below, so everything `work` reads is as of that moment.
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
            .map_err(unreadable)?;
        check_migrations(&transaction, &MIGRATIONS).map_err(unreadable)?;
        work(&View(&transaction))
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
        let mut statement = self.connection.prepare(
            "SELECT e.id, e.location, e.field, e.start, e.end, e.quote, e.trust_class,
             e.captured_at, e.source_updated_at FROM evidence e
             LEFT JOIN extraction_checkpoint c ON c.extractor = ?3 AND c.extractor_version = ?4
                 AND c.source = e.source AND c.scope = e.scope
             WHERE e.source = ?1 AND e.scope = ?2 AND (c.source IS NULL
                 OR (e.captured_at, e.id) > (c.last_captured_at, c.last_evidence_id))
             ORDER BY e.captured_at, e.id",
        )?;
        let rows = statement.query_map(
            params![source.name, source.scope.0, extractor.0, extractor.1],
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
        rows.map(|row| {
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
        .collect()
    }
    fn record_claims(
        &mut self,
        run_id: i64,
        extractor: (&str, &str),
        claims: &[ObservedClaim],
        checkpoint: Option<&Checkpoint>,
    ) -> Result<Counts> {
        let transaction = self.connection.transaction()?;
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
            // One identifier is one entity of one type; a claim naming it as another is refused.
            let mut clash = false;
            for entity in &entities {
                let stored: Option<String> = transaction
                    .query_row(
                        "SELECT type FROM entity WHERE id = ?1",
                        [&entity.id],
                        |row| row.get(0),
                    )
                    .optional()?;
                clash |= stored.is_some_and(|t| t != entity.entity_type);
            }
            if clash {
                counts.rejected += 1;
                continue;
            }
            for entity in &entities {
                transaction.execute(
                    "INSERT OR IGNORE INTO entity (id, type, identifier) VALUES (?1, ?2, ?3)",
                    params![entity.id, entity.entity_type, entity.identifier],
                )?;
                transaction.execute(
                    "INSERT OR IGNORE INTO alias (entity_id, alias, kind)
                     VALUES (?1, ?2, 'canonical')",
                    params![entity.id, entity.identifier],
                )?;
            }
            transaction.execute(
                "INSERT OR IGNORE INTO claim (id, subject, predicate, object_entity, object_value,
                 object_datatype, status, revision, scalar) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'observed', '', ?7)",
                params![
                    claim.id,
                    claim.subject.id,
                    claim.predicate,
                    object_entity,
                    object_value,
                    object_datatype,
                    claim.scalar
                ],
            )?;
            let mut added = 0;
            for s in &claim.supports {
                added += transaction.execute(
                    "INSERT OR IGNORE INTO claim_support (claim_id, evidence_id, field, start, end,
                     extraction_rule, extractor, extractor_version, run_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        claim.id,
                        s.evidence_id,
                        s.field,
                        s.start as i64,
                        s.end as i64,
                        s.extraction_rule,
                        extractor.0,
                        extractor.1,
                        run_id
                    ],
                )?;
            }
            if added == 0 {
                counts.unchanged += 1;
                continue;
            }
            counts.accepted += 1;
        }
        lifecycle::refresh(&transaction)?;
        if let Some(c) = checkpoint {
            transaction.execute(
                "INSERT INTO extraction_checkpoint (extractor, extractor_version, source, scope,
                 last_captured_at, last_evidence_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT DO UPDATE SET last_captured_at = excluded.last_captured_at,
                 last_evidence_id = excluded.last_evidence_id",
                params![
                    c.extractor,
                    c.extractor_version,
                    c.source,
                    c.scope.0,
                    stamp(c.last.0),
                    c.last.1
                ],
            )?;
        }
        transaction.commit()?;
        Ok(counts)
    }
    fn extraction_sources(&self) -> Result<Vec<(Source, String)>> {
        let rows = self.connection.prepare("SELECT name, scope, extractor FROM sources WHERE extractor IS NOT NULL
             UNION SELECT s.name, e.scope, s.extractor FROM sources s JOIN evidence e ON e.source = s.name
             WHERE s.extractor IS NOT NULL ORDER BY name, scope")?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .map(|(name, scope, extractor)| {
                (Source::github(name, String::new(), Scope(scope)), extractor)
            })
            .collect())
    }
    fn begin_claims_rebuild(&mut self) -> Result<()> {
        lifecycle::begin_rebuild(&self.connection)
    }
    fn finish_claims_rebuild(&mut self, publish: bool) -> Result<usize> {
        lifecycle::finish_rebuild(&mut self.connection, publish)
    }
    fn why(&self, claim: &str) -> Result<Option<Why>> {
        let snapshot = self.connection.unchecked_transaction()?;
        let result = View(&snapshot).why(claim)?;
        snapshot.commit()?;
        Ok(result)
    }
}

fn trust_class(text: &str) -> Result<TrustClass> {
    TrustClass::parse(text).ok_or_else(|| anyhow::anyhow!("invalid trust class: {text}"))
}

impl View<'_> {
    fn why(&self, claim: &str) -> Result<Option<Why>> {
        let found = self
            .0
            .query_row(
                "SELECT c.id, c.predicate, c.status, c.revision, s.identifier, s.type,
                 o.identifier, o.type, c.object_value, c.object_datatype
                 FROM claim c JOIN entity s ON s.id = c.subject
                 LEFT JOIN entity o ON o.id = c.object_entity WHERE c.id = ?1",
                [claim],
                |row| {
                    let text = |i| row.get::<_, Option<String>>(i);
                    Ok((
                        [
                            row.get::<_, String>(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                        ],
                        [row.get::<_, String>(4)?, row.get(5)?],
                        [text(6)?, text(7)?, text(8)?, text(9)?],
                    ))
                },
            )
            .optional()?;
        let Some(([id, predicate, status, revision], [subject, subject_type], object)) = found
        else {
            return Ok(None);
        };
        let object = match object {
            [Some(identifier), Some(entity_type), None, None] => {
                let Entity { id, .. } = Entity::new(identifier.clone(), entity_type.clone());
                Object::Entity {
                    id,
                    identifier,
                    entity_type,
                }
            }
            [None, None, Some(value), Some(datatype)] => Object::Literal { value, datatype },
            _ => bail!("claim {id} has no object"),
        };
        let mut statement = self.0.prepare(
            "SELECT e.id, e.source, e.scope, e.location, s.field, s.start, s.end, e.start,
             e.quote, e.trust_class, e.captured_at, e.source_updated_at, s.extraction_rule,
             s.extractor, s.extractor_version, s.current
             FROM claim_support s JOIN evidence e ON e.id = s.evidence_id
             WHERE s.claim_id = ?1 ORDER BY e.captured_at, e.id",
        )?;
        let rows = statement.query_map([&id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                [
                    row.get::<_, String>(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ],
                [
                    row.get::<_, i64>(5)? as u64,
                    row.get::<_, i64>(6)? as u64,
                    row.get::<_, i64>(7)? as u64,
                ],
                [row.get::<_, String>(8)?, row.get(9)?, row.get(10)?],
                row.get::<_, Option<String>>(11)?,
                [row.get::<_, String>(12)?, row.get(13)?, row.get(14)?],
                row.get::<_, bool>(15)?,
            ))
        })?;
        let supports = rows
            .map(|row| {
                let (
                    evidence_id,
                    [source, scope, location, field],
                    [start, end, from],
                    [quote, trust, captured],
                    updated,
                    [extraction_rule, extractor, extractor_version],
                    current,
                ) = row?;
                let quote = quote
                    .get((start - from) as usize..(end - from) as usize)
                    .unwrap_or_default()
                    .to_string();
                Ok(SupportView {
                    current,
                    evidence_id,
                    source,
                    scope: Scope(scope),
                    location,
                    field,
                    start,
                    end,
                    quote,
                    trust_class: trust_class(&trust)?,
                    captured_at: parse_time(captured)?,
                    source_updated_at: updated.map(parse_time).transpose()?,
                    extraction_rule,
                    extractor,
                    extractor_version,
                })
            })
            .collect::<Result<_>>()?;
        Ok(Some(Why {
            claim: ClaimView {
                id: id.clone(),
                subject: Entity::new(subject, subject_type),
                predicate,
                object,
                status,
                revision,
                known_intervals: lifecycle::intervals(self.0, &id)?,
                valid_time: rastro_core::claims::ValidTime::Unknown,
                conflicts_with: lifecycle::links(self.0, &id, "conflicts_with", false)?,
                supersedes: lifecycle::links(self.0, &id, "supersedes", false)?,
                superseded_by: lifecycle::links(self.0, &id, "supersedes", true)?,
            },
            supports,
        }))
    }
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
impl Snapshot for View<'_> {
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
