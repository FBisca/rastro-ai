use anyhow::{Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use rastro_core::{
    Appended, COVERAGE_WINDOWS_SHOWN, CompleteCheck, Coverage, CoverageRecord, CoverageState,
    CoverageWindow, Evidence, Hit, ReadStorage, Refusal, RunEnd, Scope, Snapshot, Source, Storage,
    TrustClass, UnfinishedRun,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::path::Path;

/// How long a connection waits on another process's lock before giving up.
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// What `PRAGMA user_version` 1 and 2 meant, before migrations were tracked by name. Frozen.
const LEGACY: [&str; 2] = ["202611041200_initial", "202611041500_evidence_insert_guard"];

/// `(name, sql)`, named `YYYYMMDDHHmm_<what>` so migrations from different branches
/// order by time. Never edit one that has shipped; add a new one.
const MIGRATIONS: [(&str, &str); 5] = [
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
    fn coverage(&self, source: &Source, all_windows: bool) -> Result<Coverage> {
        View(&self.connection).coverage(source, all_windows)
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
                stamp(e.captured_at),
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
        View(&self.connection).last_complete_check(source)
    }
    fn interrupt_unfinished_runs(&mut self, at: DateTime<Utc>) -> Result<usize> {
        let transaction = self.connection.transaction()?;
        let at = stamp(at);
        // The window each lost run was bound to is "couldn't look: interrupted".
        transaction.execute(
            "INSERT INTO coverage (run_id, source, scope, window_start, window_end, selection,
             state, reason, checked_at)
             SELECT id, source, scope, window_start, window_end, 'all', 'couldn''t look',
             'interrupted', ?1 FROM runs WHERE finished_at IS NULL AND window_start IS NOT NULL",
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
        View(&self.connection).search(query, limit, history)
    }
}

impl View<'_> {
    fn last_complete_check(&self, source: &Source) -> Result<Option<CompleteCheck>> {
        let row: Option<(String, String)> = self
            .0
            .query_row(
                "SELECT window_end, checked_at FROM coverage
                 WHERE source=?1 AND scope=?2 AND state IN ('found', 'empty')
                 ORDER BY id DESC LIMIT 1",
                params![source.name, source.scope.0],
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
    fn coverage(&self, source: &Source, all_windows: bool) -> Result<Coverage> {
        // The overall state is judged over every window, in SQL; only the listing is capped.
        let (windows_total, any_found): (i64, bool) = self.0.query_row(
            "SELECT count(*), coalesce(max(state = 'found'), 0) FROM coverage
             WHERE source=?1 AND scope=?2",
            params![source.name, source.scope.0],
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
        let rows = statement.query_map(params![source.name, source.scope.0, limit], |row| {
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
            source: source.name.clone(),
            scope: source.scope.clone(),
            state,
            last_checked: self.last_complete_check(source)?.map(|c| c.checked_at),
            fresh_until: None,
            reason,
            windows_total: windows_total as usize,
            windows,
        })
    }
    fn search(&self, query: &str, limit: usize, history: bool) -> Result<Vec<Hit>> {
        let mut statement = self.0.prepare(
            "SELECT e.id, e.source, e.scope, e.location, e.field, e.start as i64, e.end as i64, e.quote,
             e.trust_class, e.captured_at, e.source_updated_at, e.original
             FROM evidence_fts JOIN evidence e ON e.id = evidence_fts.rowid
             WHERE evidence_fts MATCH ?1
             AND (?3 OR e.id = (SELECT max(l.id) FROM evidence l WHERE l.source = e.source
                  AND l.scope = e.scope AND l.location = e.location AND l.field = e.field))
             ORDER BY bm25(evidence_fts), e.id LIMIT ?2",
        )?;
        let rows = statement.query_map(params![query, limit as i64, history], |row| {
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
            Ok(Hit {
                id,
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
    fn unfinished_runs(&self) -> Result<Vec<UnfinishedRun>> {
        let mut statement = self.0.prepare(
            "SELECT source, scope, window_start, window_end FROM runs
             WHERE finished_at IS NULL AND window_start IS NOT NULL ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
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
