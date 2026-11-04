use anyhow::{Result, bail, ensure};
use rastro_core::{Coverage, CoverageState, Source, Storage};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

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
            version <= 2,
            "database schema {version} is newer than supported version 2"
        );
        if version == 0 {
            transaction.execute_batch(include_str!("../migrations/0001.sql"))?;
        }
        if version < 2 {
            transaction.execute_batch(include_str!("../migrations/0002.sql"))?;
            transaction.pragma_update(None, "user_version", 2)?;
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
        })
    }
}
