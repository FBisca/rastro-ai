//! The read-only open path: answers `search` and `coverage` without writing anything.
//!
//! Each open reads the config fresh under a shared lock and runs no migration, projection or
//! recovery. Each answer is computed inside one read transaction, so the hits and the coverage
//! in it come from the same snapshot.
use crate::search::{SearchOptions, SearchResults, assemble_search, search_query};
use crate::{
    Config, Coverage, CoverageState, CoverageWindow, Hit, Locations, Pair, Rastro, Scope, Source,
};
use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use std::{
    fs::{File, TryLockError},
    io::ErrorKind,
    path::Path,
};

/// A database this build must not read: it would have to migrate first, or it comes from a
/// newer build. Callers recover it with `downcast_ref` on the `anyhow::Error`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// The database can't be read without writing (a WAL database whose `-wal`/`-shm` files
    /// are missing, in a directory that can't be written). A writing open fixes it.
    NotReadable(String),
    /// Migrations this build knows that the database lacks; a writing open applies them.
    PendingMigrations(Vec<String>),
    /// A migration the database holds that this build doesn't know.
    UnknownMigration(String),
}
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PendingMigrations(names) => write!(
                f,
                "database has pending migrations ({}); run a rastro command that writes first",
                names.join(", ")
            ),
            Self::NotReadable(why) => write!(f, "database can't be read read-only: {why}"),
            Self::UnknownMigration(name) => write!(
                f,
                "database has migration {name}, which this version of rastro doesn't know"
            ),
        }
    }
}
impl std::error::Error for Refusal {}

/// A run that never finished, with the window it was bound to.
#[derive(Clone, Debug)]
pub struct UnfinishedRun {
    pub source: String,
    pub scope: Scope,
    pub window: (DateTime<Utc>, DateTime<Utc>),
}

/// What storage can answer inside one read transaction.
pub trait Snapshot {
    fn explain(
        &self,
        _claim: &str,
        _options: &crate::knowledge::WhyOptions,
    ) -> Result<Option<crate::knowledge::Explanation>> {
        anyhow::bail!("storage does not support why")
    }

    /// Exact claim state from the visible justifications. Empty scopes allow all.
    fn explain_scoped(
        &self,
        claim: &str,
        options: &crate::knowledge::WhyOptions,
        scopes: &[Scope],
    ) -> Result<Option<crate::knowledge::Explanation>> {
        anyhow::ensure!(scopes.is_empty(), "storage does not support scoped why");
        self.explain(claim, options)
    }
    /// What `about` reads, from the justifications and intents visible to `scopes` (empty:
    /// every scope): whether the form resolved, the claims (a Person's whole view, or up to one
    /// more than `limit`), and a Person's linked accounts. A read past the work budget fails.
    fn about_view(
        &self,
        _entity: &str,
        _options: &crate::knowledge::AboutOptions,
        _scopes: &[Scope],
    ) -> Result<crate::knowledge::AboutClaims> {
        anyhow::bail!("storage does not support about")
    }
    /// The claims of [`Snapshot::about_view`], ungrouped.
    fn about_scoped(
        &self,
        entity: &str,
        options: &crate::knowledge::AboutOptions,
        scopes: &[Scope],
    ) -> Result<Vec<crate::knowledge::ClaimSummary>> {
        Ok(self
            .about_view(entity, options, scopes)?
            .claims
            .into_iter()
            .map(|c| c.summary)
            .collect())
    }

    /// Unscoped stored claim state.
    fn about(
        &self,
        entity: &str,
        options: &crate::knowledge::AboutOptions,
    ) -> Result<Vec<crate::knowledge::ClaimSummary>> {
        self.about_scoped(entity, options, &[])
    }

    /// The labels (knowledge §69) of the entities that have one, keyed by entity id, from the
    /// observations and claims visible to `scopes` (empty: every scope). An entity whose label
    /// is hidden, or that has none, is absent.
    fn labels(
        &self,
        _entities: &[crate::claims::Entity],
        _scopes: &[Scope],
    ) -> Result<std::collections::BTreeMap<String, crate::knowledge::Label>> {
        Ok(Default::default())
    }

    /// The accepted owner intents, in acceptance order.
    fn intents(&self) -> Result<Vec<crate::identity::Intent>> {
        anyhow::bail!("storage does not support owner intents")
    }
    /// The entity (id, canonical identifier) a form names, resolved unscoped as `about` resolves
    /// it: an exact id or identifier, else a canonical alias. An ambiguous alias is an error.
    /// Whether the evidence says the entity `id` is a bot: one of its current types is a
    /// [`crate::commitments::is_bot_type`].
    fn is_bot(&self, _id: &str) -> Result<bool> {
        anyhow::bail!("storage does not support bot checks")
    }
    fn resolve_entity(&self, _form: &str) -> Result<Option<(String, String)>> {
        anyhow::bail!("storage does not support entity resolution")
    }
    /// What the fuzzy-candidate rules read: every entity with a current `name`, `login` or
    /// `email` alias, unscoped (only the CLI, which sees every scope, lists candidates).
    fn identity_accounts(&self) -> Result<Vec<crate::identity::AccountFacts>> {
        anyhow::bail!("storage does not support identity candidates")
    }
    /// A meeting with its latest run and the hypotheses of its last good run, when it exists and
    /// its scope is allowed (empty `scopes` allows every scope).
    fn meeting(
        &self,
        _meeting: &str,
        _scopes: &[Scope],
    ) -> Result<Option<crate::meeting::StoredMeeting>> {
        anyhow::bail!("storage does not support meetings")
    }
    /// What replay starts from: the latest good run's recorded output, spans and mappings.
    fn meeting_recording(&self, _meeting: &str) -> Result<Option<crate::meeting::Recording>> {
        anyhow::bail!("storage does not support meetings")
    }
    /// A meeting as the owner supplied it, for export.
    fn meeting_export(&self, _meeting: &str) -> Result<Option<crate::meeting::MeetingExport>> {
        anyhow::bail!("storage does not support meetings")
    }
    /// Every source of commitment hypotheses in `scopes` (empty: every scope), each with its
    /// latest run, its last good results and whether they are stale.
    fn commitment_sources(&self, _scopes: &[Scope]) -> Result<Vec<crate::commitments::Source>> {
        anyhow::bail!("storage does not support commitments")
    }
    /// What storage recorded for the source, before the freshness rule is applied.
    fn coverage(&self, pair: &Pair, all_windows: bool) -> Result<Coverage>;
    /// Allowed (source, scope) pairs storage holds anything for, with stored freshness rules.
    /// An empty scope set allows every scope. Filter before loading rows.
    fn stored_pairs(&self, scopes: &[Scope]) -> Result<Vec<Pair>>;
    /// Filters scopes before applying the limit; an empty set allows every scope.
    fn search(
        &self,
        query: &str,
        limit: usize,
        history: bool,
        scopes: &[Scope],
    ) -> Result<Vec<Hit>>;
    /// Runs with no end and a window; recovery would turn each into "couldn't look: interrupted".
    /// An empty scope set allows every scope. Filter before loading rows.
    fn unfinished_runs(&self, scopes: &[Scope]) -> Result<Vec<UnfinishedRun>>;
    /// Bound sources, including retained historical ones, with their bound extractor id.
    /// An empty scope set allows every scope. Filter before loading rows.
    fn extraction_sources(&self, scopes: &[Scope]) -> Result<Vec<(Source, String)>>;
    /// Whether the pair's extractor is running, last failed, or has evidence left to read.
    fn extraction_facts(
        &self,
        source: &Source,
        extractor: &str,
        version: &str,
        vocabulary_version: &str,
    ) -> Result<crate::runs::ExtractionFacts>;
}
/// Storage opened read-only.
pub trait ReadStorage {
    /// Runs `work` against one snapshot. Refuses (see [`Refusal`]) a database that needs
    /// migrating or is newer than this build. The snapshot must be pinned (the first read
    /// done) before `work` runs: the caller drops its `sync.lock` guard inside `work`.
    fn read<T>(&mut self, work: impl FnOnce(&dyn Snapshot) -> Result<T>) -> Result<T>;
}

/// Whether a sync is running now.
enum Sync {
    /// No sync holds `sync.lock`; the guard (if the file exists) keeps one from starting
    /// until the snapshot is pinned.
    Idle(Option<File>),
    Live,
}
impl Sync {
    fn probe(locations: &Locations) -> Result<Self> {
        let file = match File::open(locations.sync_lock()) {
            Ok(file) => file,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Self::Idle(None)),
            Err(e) => return Err(e.into()),
        };
        match file.try_lock_shared() {
            Ok(()) => Ok(Self::Idle(Some(file))),
            Err(TryLockError::WouldBlock) => Ok(Self::Live),
            Err(TryLockError::Error(e)) => Err(e.into()),
        }
    }
}

impl Locations {
    /// A shared lock on the config, or none when no core ever wrote one (nothing to wait for).
    fn shared_config_lock(&self) -> Result<Option<File>> {
        match File::open(self.config.join("config.lock")) {
            Ok(file) => {
                file.lock_shared()?;
                Ok(Some(file))
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

/// A read-only view of the store, as of the config read when it was opened.
pub struct Reader<S> {
    pub(crate) locations: Locations,
    pub(crate) config: Config,
    pub(crate) storage: Option<S>,
    pub(crate) scopes: Vec<Scope>,
}

impl<S: ReadStorage> Rastro<S> {
    /// Opens for reading only. The config is read fresh, under a shared lock; nothing is
    /// migrated, projected or recovered, and no file is created. `open_storage` returns the
    /// store opened read-only, or `None` when its database file doesn't exist.
    pub fn open_read_only(
        locations: Locations,
        open_storage: impl FnOnce(&Path) -> Result<Option<S>>,
    ) -> Result<Reader<S>> {
        Self::open_read_only_scoped(locations, open_storage, Vec::new())
    }

    /// Opens with an owner's launch scopes. An empty set allows every scope. Config errors
    /// are redacted when scoped because validation may name sources outside those scopes.
    pub fn open_read_only_scoped(
        locations: Locations,
        open_storage: impl FnOnce(&Path) -> Result<Option<S>>,
        scopes: Vec<Scope>,
    ) -> Result<Reader<S>> {
        let lock = locations.shared_config_lock()?;
        let config = Config::load(&locations).map_err(|error| {
            if scopes.is_empty() {
                error
            } else {
                anyhow!("the config can't be read; run `rastro source list` on the command line for details")
            }
        })?;
        drop(lock);
        let storage = open_storage(&locations.database())?;
        Ok(Reader {
            locations,
            config,
            storage,
            scopes,
        })
    }
}

impl<S: ReadStorage> Reader<S> {
    pub fn sources(&self) -> &[Source] {
        &self.config.sources
    }
    /// Each configured source's coverage (or the named one's), judged at the current time.
    pub fn coverage(&mut self, name: Option<&str>, all_windows: bool) -> Result<Vec<Coverage>> {
        let Self {
            locations,
            config,
            storage,
            scopes,
        } = self;
        snapshot(locations, storage, scopes, |view, unfinished| {
            coverage_of(view, unfinished, &config.sources, name, all_windows, scopes)
        })
    }
    /// The extraction state of every bound pair in scope, read without writing. A run left
    /// unfinished reads `running` while a sync holds the lock and `failed` (interrupted)
    /// when none does.
    pub fn extraction_states(&mut self) -> Result<Vec<crate::runs::ExtractionPair>> {
        let Self {
            locations,
            storage,
            scopes,
            ..
        } = self;
        snapshot_live(locations, storage, scopes, |view, _, live| {
            let Some(view) = view else {
                return Ok(Vec::new());
            };
            view.extraction_sources(scopes)?
                .into_iter()
                .map(|(source, extractor)| {
                    crate::runs::extraction_pair(
                        locations,
                        &source,
                        &extractor,
                        live,
                        |version, vocabulary| {
                            view.extraction_facts(&source, &extractor, version, vocabulary)
                        },
                    )
                })
                .collect()
        })
    }
    /// Full-text search over evidence quotes, each hit with its source's coverage.
    pub fn search(&mut self, text: &str, options: &SearchOptions) -> Result<SearchResults> {
        let Self {
            locations,
            config,
            storage,
            scopes,
        } = self;
        let query = search_query(text);
        snapshot(locations, storage, scopes, |view, unfinished| {
            let hits = match (&query, view) {
                (Some(query), Some(view)) => {
                    view.search(query, options.limit + 1, options.history, scopes)?
                }
                _ => Vec::new(),
            };
            let mut coverage = coverage_of(view, unfinished, &config.sources, None, false, scopes)?;
            for answer in &mut coverage {
                answer.windows.clear();
            }
            Ok(assemble_search(hits, coverage, options))
        })
    }
}

pub(crate) fn snapshot<S: ReadStorage, T>(
    locations: &Locations,
    storage: &mut Option<S>,
    scopes: &[Scope],
    work: impl FnOnce(Option<&dyn Snapshot>, &[UnfinishedRun]) -> Result<T>,
) -> Result<T> {
    snapshot_live(locations, storage, scopes, |view, unfinished, _| {
        work(view, unfinished)
    })
}

/// Like [`snapshot`], and tells `work` whether a sync held `sync.lock` when the snapshot was
/// pinned.
pub(crate) fn snapshot_live<S: ReadStorage, T>(
    locations: &Locations,
    storage: &mut Option<S>,
    scopes: &[Scope],
    work: impl FnOnce(Option<&dyn Snapshot>, &[UnfinishedRun], bool) -> Result<T>,
) -> Result<T> {
    let Some(storage) = storage else {
        return work(None, &[], false);
    };
    // Taken before the snapshot and held through it: while it is held no sync can start, so
    // an unfinished run seen in the snapshot belongs to no live sync.
    let sync = Sync::probe(locations)?;
    storage.read(|view| {
        // The snapshot is pinned: the lock has done its job (no sync started before the pin),
        // so let go, and a waiting sync isn't kept out by overlapping readers.
        let live = match sync {
            Sync::Live => true,
            Sync::Idle(Some(guard)) => {
                drop(guard);
                false
            }
            // No lock file when the call began: a sync may have created and taken it since.
            // Accepted race: a whole sync that creates the file, starts its run before the pin
            // and releases before this probe reads as interrupted; negligible.
            Sync::Idle(None) => matches!(Sync::probe(locations)?, Sync::Live),
        };
        let unfinished = if live {
            Vec::new()
        } else {
            view.unfinished_runs(scopes)?
        };
        work(Some(view), &unfinished, live)
    })
}

pub(crate) fn coverage_of(
    view: Option<&dyn Snapshot>,
    unfinished: &[UnfinishedRun],
    config: &[Source],
    name: Option<&str>,
    all_windows: bool,
    scopes: &[Scope],
) -> Result<Vec<Coverage>> {
    let now = crate::coverage::now()?;
    let stored = match view {
        Some(view) => view.stored_pairs(scopes)?,
        None => Vec::new(),
    };
    // Filter both legs before resolving names: a hidden-only source is unknown here.
    let config: Vec<_> = config
        .iter()
        .filter(|s| scopes.is_empty() || scopes.contains(&s.scope))
        .cloned()
        .collect();
    crate::coverage::pairs(&config, stored, name)?
        .into_iter()
        .map(|(pair, registered)| {
            let mut coverage = match view {
                Some(view) => view.coverage(&pair, all_windows)?,
                None => Coverage {
                    source: pair.source.clone(),
                    scope: pair.scope.clone(),
                    registered,
                    state: CoverageState::NeverLooked,
                    last_checked: None,
                    fresh_until: None,
                    reason: None,
                    windows_total: 0,
                    windows: Vec::new(),
                },
            };
            coverage.registered = registered;
            // What startup recovery would record, computed instead of written.
            for run in unfinished
                .iter()
                .filter(|r| r.source == pair.source && r.scope == pair.scope)
            {
                coverage.windows_total += 1;
                coverage.windows.insert(
                    0,
                    CoverageWindow {
                        from: run.window.0,
                        to: run.window.1,
                        state: CoverageState::CouldNotLook,
                        selection: "all".into(),
                        gaps: Vec::new(),
                        reason: Some("interrupted".into()),
                        retry_at: None,
                        checked_at: now,
                    },
                );
                coverage.state = CoverageState::CouldNotLook;
                coverage.reason = Some("interrupted".into());
            }
            if !all_windows {
                coverage.windows.truncate(crate::COVERAGE_WINDOWS_SHOWN);
            }
            Ok(coverage.judged(pair.freshness(), now))
        })
        .collect()
}
