use anyhow::{Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use rastro_core::{
    CompleteCheck, Coverage, CoverageRecord, CoverageState, Evidence, Hit, RunEnd, Scope, Source,
    Storage, TrustClass,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

/// What `PRAGMA user_version` 1 and 2 meant, before migrations were tracked by name. Frozen.
const LEGACY: [&str; 2] = ["202611041200_initial", "202611041500_evidence_insert_guard"];

/// `(name, sql)`, named `YYYYMMDDHHmm_<what>` so migrations from different branches
/// order by time. Never edit one that has shipped; add a new one.
const MIGRATIONS: [(&str, &str); 4] = [
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
    fn coverage(&self, source: &Source) -> Result<Coverage> {
        let state: Option<String> = self
            .connection
            .query_row(
                "SELECT state FROM coverage WHERE source=?1 AND scope=?2 ORDER BY id DESC LIMIT 1",
                params![source.name, source.scope.0],
                |row| row.get(0),
            )
            .optional()?;
        let state = match state.as_deref() {
            None => CoverageState::NeverLooked,
            Some("found") => CoverageState::Found,
            Some("empty") => CoverageState::Empty,
            Some("couldn't look") => CoverageState::CouldNotLook,
            Some(other) => bail!("invalid coverage state: {other}"),
        };
        Ok(Coverage {
            source: source.name.clone(),
            scope: source.scope.clone(),
            state,
            last_checked: self.last_complete_check(source)?.map(|c| c.checked_at),
            fresh_until: None,
        })
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
    fn append_evidence(&mut self, _run_id: i64, e: &Evidence) -> Result<i64> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO evidence (source, scope, location, field, start, end, quote, captured_at,
             source_updated_at, trust_class) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
                e.trust_class.as_str()
            ],
        )?;
        let id = transaction.last_insert_rowid();
        transaction.execute(
            "INSERT INTO evidence_fts (rowid, quote) VALUES (?1, ?2)",
            params![id, e.quote],
        )?;
        transaction.commit()?;
        Ok(id)
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
        let row: Option<(String, String)> = self
            .connection
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
    fn search(&self, query: &str, limit: usize) -> Result<Vec<Hit>> {
        let mut statement = self.connection.prepare(
            "SELECT e.id, e.source, e.scope, e.location, e.field, e.start as i64, e.end as i64, e.quote,
             e.trust_class, e.captured_at, e.source_updated_at
             FROM evidence_fts JOIN evidence e ON e.id = evidence_fts.rowid
             WHERE evidence_fts MATCH ?1 ORDER BY bm25(evidence_fts), e.id LIMIT ?2",
        )?;
        let rows = statement.query_map(params![query, limit as i64], |row| {
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
            ))
        })?;
        rows.map(|row| {
            let (
                id,
                [source, scope, location, field],
                [start, end],
                [quote, trust, captured],
                updated,
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
                },
            })
        })
        .collect()
    }
}
