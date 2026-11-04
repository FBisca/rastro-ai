//! Runs: one execution of a connector for a source, and the log it leaves.
use crate::search::{SearchOptions, assemble_search, search_query};
use crate::{Counts, CoverageState, Rastro, Source, Storage, files, host};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rastro_plugin_sdk::Window;
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, JsonSchema, Serialize)]
pub struct RunReport {
    pub run_id: i64,
    pub source: String,
    pub scope: String,
    pub plugin_id: String,
    pub plugin_version: String,
    /// `ok` or `failed`.
    #[schemars(extend("enum" = ["ok", "failed"]))]
    pub status: String,
    pub reason: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub window: Window,
    pub counts: Counts,
    pub coverage: CoverageState,
}

impl<S: Storage> Rastro<S> {
    /// Runs the connector of one source, or of every source, one after another.
    pub fn sync(&mut self, name: Option<&str>) -> Result<Vec<RunReport>> {
        self.sync_with_reread(name, false)
    }

    /// Runs connectors from the registered window start when `reread` is true.
    /// Re-reading uses the same append, unchanged and coverage rules as any sync.
    pub fn sync_with_reread(&mut self, name: Option<&str>, reread: bool) -> Result<Vec<RunReport>> {
        let sources: Vec<Source> = self
            .config
            .sources
            .iter()
            .filter(|s| name.is_none_or(|n| s.name == n))
            .cloned()
            .collect();
        if sources.is_empty()
            && let Some(name) = name
        {
            return Err(crate::UnknownSource(name.into()).into());
        }
        files::private_directory(&self.locations.data)?;
        let lock = files::private_file(&self.locations.sync_lock())?;
        lock.lock()?;
        // Holding the lock exclusively proves no other sync is live, so a run still unfinished
        // was left by a crash. Record it now, before this sync's own runs, so the latest
        // attempt stays the latest.
        self.storage.interrupt_unfinished_runs(Utc::now())?;
        let mut reports = Vec::new();
        for source in &sources {
            let plugin = host::discover(&self.locations, &source.connector);
            let from = if reread {
                source.window_start
            } else {
                match self.storage.last_complete_check(source)? {
                    Some(check) => check.window_end,
                    None => source.window_start,
                }
            };
            reports.push(host::run(
                &mut self.storage,
                source,
                plugin.as_ref().map_err(|reason| *reason),
                from,
                Utc::now(),
            )?);
        }
        drop(lock);
        Ok(reports)
    }

    /// Rebuilds the search index from the stored evidence.
    pub fn rebuild_index(&mut self) -> Result<usize> {
        self.storage.rebuild_index()
    }

    /// Full-text search over evidence quotes, each hit with its source's coverage.
    pub fn search(&self, text: &str, options: &SearchOptions) -> Result<SearchResults> {
        let hits = match search_query(text) {
            // One more than asked for, to learn whether the answer was cut.
            Some(query) => self
                .storage
                .search(&query, options.limit + 1, options.history)?,
            None => Vec::new(),
        };
        let mut coverage = self.coverage(None, false)?;
        // A search names each source's answer; the window rows belong to `coverage`.
        for answer in &mut coverage {
            answer.windows.clear();
        }
        Ok(assemble_search(hits, coverage, options))
    }
}

pub use crate::search::{SearchResult, SearchResults};
