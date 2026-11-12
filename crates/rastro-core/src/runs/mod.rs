//! Runs: one execution of a connector for a source, and the log it leaves.
use crate::search::{SearchOptions, assemble_search, search_query};
use crate::{Counts, CoverageState, Rastro, Source, Storage, files, host};
use anyhow::Result;
mod progress;
use chrono::{DateTime, Utc};
pub use progress::{COUNT_EVERY, Observer, Silent, Stage, StartReason, StderrProgress, Throttle};
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
    /// When the connector said to try again, if it ended `rate_limited`. It is in the
    /// coverage the run recorded; the report carries it for the sync's progress.
    #[serde(skip)]
    pub retry_at: Option<DateTime<Utc>>,
    /// The run of the extractor the source is bound to, after ingestion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extraction: Option<ExtractionReport>,
}

/// How an extractor run ended.
#[derive(Clone, Copy, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionState {
    Current,
    Failed,
    NotInstalled,
}
impl ExtractionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Failed => "failed",
            Self::NotInstalled => "not_installed",
        }
    }
}
/// Where an extractor stands for one (source, scope) pair at its installed version.
#[derive(Clone, Copy, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionStatus {
    /// No installed extractor provides the bound id.
    NotInstalled,
    /// Evidence, or the whole replay for a new version, is not yet extracted.
    Behind,
    /// A run is in progress.
    Running,
    /// The latest run at this version failed; reads serve the last good projection.
    Failed,
    /// Every observation of the pair has been extracted at this version.
    Current,
}
impl ExtractionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotInstalled => "not_installed",
            Self::Behind => "behind",
            Self::Running => "running",
            Self::Failed => "failed",
            Self::Current => "current",
        }
    }
}
/// What storage knows of one pair's extractor at one version.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExtractionFacts {
    /// An extractor run for the pair, of any version, has not finished.
    pub running: bool,
    /// The latest finished run at this version failed.
    pub failed: bool,
    /// Observations lie past the pair's checkpoint at this version.
    pub pending: bool,
}
/// The extraction state of one (source, scope) pair.
#[derive(Clone, Debug, JsonSchema, Serialize)]
pub struct ExtractionPair {
    pub source: String,
    pub scope: String,
    pub extractor: String,
    /// The installed version; `None` when the extractor is not installed.
    pub extractor_version: Option<String>,
    pub state: ExtractionStatus,
}

/// One extractor run over a source's pending evidence. Its counts are of claims.
#[derive(Debug, JsonSchema, Serialize)]
pub struct ExtractionReport {
    pub run_id: i64,
    pub extractor: String,
    pub extractor_version: String,
    pub state: ExtractionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Observations sent to the extractor.
    pub evidence: usize,
    pub counts: Counts,
    /// The extractor passed its check and its scope, so a failure is its own run failing.
    /// A pair that never started has no new output to replace the stored one.
    #[serde(skip)]
    pub started: bool,
}

/// One (source, scope) pair of a claims rebuild: how its extractor ended. A pair that is not
/// `current` never started, and its stored output was carried over.
#[derive(Debug, JsonSchema, Serialize)]
pub struct RebuildPair {
    pub source: String,
    pub scope: String,
    pub run_id: i64,
    pub extractor: String,
    pub extractor_version: String,
    pub state: ExtractionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Claim counts of the pair's run, as the run log records them.
    pub counts: Counts,
}

/// A finished claims rebuild.
#[derive(Debug)]
pub struct Rebuilt {
    pub claims: usize,
    pub pairs: Vec<RebuildPair>,
}

impl<S: Storage> Rastro<S> {
    /// Runs the connector of one source, or of every source, one after another.
    pub fn sync(&mut self, name: Option<&str>) -> Result<Vec<RunReport>> {
        self.sync_with_reread(name, false)
    }

    /// Runs connectors from the registered window start when `reread` is true.
    /// Re-reading uses the same append, unchanged and coverage rules as any sync.
    pub fn sync_with_reread(&mut self, name: Option<&str>, reread: bool) -> Result<Vec<RunReport>> {
        self.sync_observed(name, reread, &mut Silent)
    }

    /// `sync_with_reread`, telling `observer` what each source does as it does it. The
    /// observer can't change a run: it is told, and nothing it does fails or delays one.
    pub fn sync_observed(
        &mut self,
        name: Option<&str>,
        reread: bool,
        observer: &mut dyn Observer,
    ) -> Result<Vec<RunReport>> {
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
            let plugin = host::discover(&self.locations, &source.connector, "connector").and_then(
                |plugin| {
                    // A stored source is checked like a new one, before the connector runs.
                    host::check_source_options(&self.locations, source, Some(&plugin.manifest))
                        .map_err(|error| {
                            host::notice(&format!("source {}: {error:#}", source.name));
                            "protocol"
                        })?;
                    // A single-source connector reads only the first of its sources in the
                    // config; a later one (stored before the refusal, or while the connector
                    // wasn't installed) fails before the connector runs.
                    if plugin.manifest.single_source
                        && let Some(first) = self
                            .config
                            .sources
                            .iter()
                            .find(|s| s.connector == source.connector)
                        && first.name != source.name
                    {
                        host::notice(&format!(
                            "source {}: connector {} takes a single source, and source {} already uses it",
                            source.name, source.connector, first.name
                        ));
                        return Err("protocol");
                    }
                    Ok(plugin)
                },
            );
            let (from, why) = if reread {
                (
                    source.window_start,
                    StartReason::Reread(source.window_start),
                )
            } else {
                match self.storage.last_complete_check(source)? {
                    Some(check) => (check.window_end, StartReason::Incremental(check.window_end)),
                    None => (
                        source.window_start,
                        StartReason::FirstRead(source.window_start),
                    ),
                }
            };
            observer.started(&source.name, &why);
            let mut report = host::run(
                &mut self.storage,
                source,
                plugin.as_ref().map_err(|reason| *reason),
                from,
                Utc::now(),
                observer,
            )?;
            // The connector binds its sources to an extractor; only that one ever reads them.
            if let Ok(plugin) = &plugin {
                let bound = plugin.manifest.default_extractor.as_deref();
                self.storage.bind_extractor(&source.name, bound)?;
                if let Some(extractor) = bound {
                    let extraction = host::extract(
                        &mut self.storage,
                        &self.locations,
                        source,
                        extractor,
                        observer,
                        report.counts.accepted,
                    )?;
                    report.extraction = Some(extraction);
                }
            }
            observer.finished(&report);
            reports.push(report);
        }
        drop(lock);
        Ok(reports)
    }

    /// Replays evidence through its bound extractors into a private generation and publishes
    /// only after every extractor that starts succeeds. A pair whose extractor never starts
    /// (not installed, failing its check, scope forbidden) keeps its stored output, carried
    /// into the new generation. Readers keep the previous generation on failure.
    pub fn rebuild_claims(&mut self) -> Result<Rebuilt> {
        let lock = files::private_file(&self.locations.sync_lock())?;
        lock.lock()?;
        self.storage.interrupt_unfinished_runs(Utc::now())?;
        let sources = self.storage.extraction_sources()?;
        self.storage.begin_claims_rebuild()?;
        let mut pairs = Vec::new();
        let replay = (|| -> Result<()> {
            for (source, extractor) in &sources {
                let report = host::extract(
                    &mut self.storage,
                    &self.locations,
                    source,
                    extractor,
                    &mut Silent,
                    0,
                )?;
                if !report.started && report.state != ExtractionState::Current {
                    self.storage.carry_over_pair(source, extractor)?;
                } else {
                    anyhow::ensure!(
                        report.state == ExtractionState::Current,
                        "claims rebuild: extractor {extractor} for {} failed: {}",
                        source.name,
                        report.reason.as_deref().unwrap_or("unknown")
                    );
                }
                pairs.push(RebuildPair {
                    source: source.name.clone(),
                    scope: source.scope.0.clone(),
                    run_id: report.run_id,
                    extractor: extractor.clone(),
                    extractor_version: report.extractor_version,
                    state: report.state,
                    reason: report.reason,
                    counts: report.counts,
                });
            }
            Ok(())
        })();
        let claims = match replay {
            Ok(()) => match self.storage.finish_claims_rebuild(true) {
                Ok(count) => count,
                Err(error) => {
                    self.storage.finish_claims_rebuild(false)?;
                    return Err(error);
                }
            },
            Err(error) => {
                self.storage.finish_claims_rebuild(false)?;
                return Err(error);
            }
        };
        Ok(Rebuilt { claims, pairs })
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

/// The state of one pair at the installed version of its extractor. `live` says whether a
/// sync holds the lock now: a run left unfinished while none does was interrupted, so it reads
/// `failed`, as the next writing open records it.
pub fn extraction_pair(
    locations: &crate::Locations,
    source: &Source,
    extractor: &str,
    live: bool,
    facts: impl FnOnce(&str, &str) -> Result<ExtractionFacts>,
) -> Result<ExtractionPair> {
    let installed = host::discover(locations, extractor, "extractor");
    // Even an extractor that fails its check has a version to name.
    let version = host::find(locations, extractor).map(|(manifest, _)| manifest.version);
    let state = match (&installed, &version) {
        (Err("not_installed"), _) => ExtractionStatus::NotInstalled,
        // An installed extractor that fails its check cannot run.
        (Err(_), _) | (_, None) => ExtractionStatus::Failed,
        (Ok(plugin), Some(version)) => match host::vocabulary(plugin) {
            // A vocabulary that cannot be read cannot be extracted with.
            Err(_) => ExtractionStatus::Failed,
            Ok(vocabulary) => {
                let facts = facts(version, &vocabulary.version)?;
                if facts.running && live {
                    ExtractionStatus::Running
                } else if facts.running || facts.failed {
                    ExtractionStatus::Failed
                } else if facts.pending {
                    ExtractionStatus::Behind
                } else {
                    ExtractionStatus::Current
                }
            }
        },
    };
    Ok(ExtractionPair {
        source: source.name.clone(),
        scope: source.scope.0.clone(),
        extractor: extractor.into(),
        extractor_version: version,
        state,
    })
}
