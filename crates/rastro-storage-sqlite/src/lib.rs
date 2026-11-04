use anyhow::{Result, bail, ensure};
use chrono::{DateTime, SecondsFormat, Utc};
use rastro_core::{
    CompleteCheck, Coverage, CoverageRecord, CoverageState, Evidence, Hit, RunEnd, Scope, Source,
    Storage, TrustClass,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

/// Applied in order; never edit one that has shipped, add the next.
const MIGRATIONS: [&str; 3] = [
    include_str!("../migrations/0001.sql"),
    include_str!("../migrations/0002.sql"),
    include_str!("../migrations/0003.sql"),
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
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            rastro_core::files::private_directory(parent)?;
        }
        let _file = rastro_core::files::private_file(path)?;
        let mut connection = Connection::open(path)?;
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: u32 = transaction.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        ensure!(
            version as usize <= MIGRATIONS.len(),
            "database schema {version} is newer than supported version {}",
            MIGRATIONS.len()
        );
        for (index, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            transaction.execute_batch(sql)?;
            transaction.pragma_update(None, "user_version", (index + 1) as i64)?;
        }
        transaction.commit()?;
        Ok(Self { connection })
    }
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
    ) -> Result<i64> {
        self.connection.execute(
            "INSERT INTO runs (source, scope, plugin_id, plugin_version, started_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5, 'running')",
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
