//! Runs: one execution of a connector for a source, and the log it leaves.
use crate::{Counts, CoverageState, Rastro, Source, Storage, files, host};
use anyhow::{Result, bail};
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
    pub status: String,
    pub reason: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    #[schemars(with = "RunWindow")]
    pub window: Window,
    #[schemars(with = "RunCounts")]
    pub counts: Counts,
    pub coverage: CoverageState,
}

impl<S: Storage> Rastro<S> {
    /// Runs the connector of one source, or of every source, one after another.
    pub fn sync(&mut self, name: Option<&str>) -> Result<Vec<RunReport>> {
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
            bail!("unknown source: {name}");
        }
        files::private_directory(&self.locations.data)?;
        let lock = files::private_file(&self.locations.sync_lock())?;
        lock.lock()?;
        let mut reports = Vec::new();
        for source in &sources {
            let plugin = host::discover(&self.locations, &source.connector);
            let from = match self.storage.last_complete_check(source)? {
                Some(check) => check.window_end,
                None => source.window_start,
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
    pub fn search(&self, text: &str, history: bool) -> Result<SearchResults> {
        let terms: Vec<_> = text
            .split_whitespace()
            .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
            .collect();
        let hits = if terms.is_empty() {
            Vec::new()
        } else {
            self.storage.search(&terms.join(" "), 50, history)?
        };
        let mut coverage = self.coverage(None)?;
        // A search names each source's answer; the window rows belong to `coverage`.
        for answer in &mut coverage {
            answer.windows.clear();
        }
        let results = hits
            .into_iter()
            .map(|hit| {
                let covering = coverage
                    .iter()
                    .position(|c| c.source == hit.evidence.source);
                (hit, covering)
            })
            .collect::<Vec<_>>();
        let results = results
            .into_iter()
            .map(|(hit, covering)| SearchResult {
                hit,
                coverage: covering.map(|i| coverage[i].clone()),
            })
            .collect();
        Ok(SearchResults { results, coverage })
    }
}

#[derive(Debug, JsonSchema, Serialize)]
pub struct SearchResult {
    #[serde(flatten)]
    pub hit: crate::Hit,
    pub coverage: Option<crate::Coverage>,
}
/// Matches, plus the coverage of every source searched, so an empty answer says what it covered.
#[derive(Debug, JsonSchema, Serialize)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
    pub coverage: Vec<crate::Coverage>,
}

// Schema of `rastro_plugin_sdk::Window`, which the SDK does not derive.
#[derive(JsonSchema)]
#[allow(dead_code)]
struct RunWindow {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
}
// Schema of `rastro_plugin_sdk::Counts`, which the SDK does not derive.
#[derive(JsonSchema)]
#[allow(dead_code)]
struct RunCounts {
    submitted: u64,
    accepted: u64,
    rejected: u64,
    unchanged: u64,
}
