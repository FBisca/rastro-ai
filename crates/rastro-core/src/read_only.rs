//! The read-only open path: answers `search` and `coverage` without writing anything.
//!
//! Each open reads the config fresh under a shared lock and runs no migration, projection or
//! recovery. Each answer is computed inside one read transaction, so the hits and the coverage
//! in it come from the same snapshot.
use crate::UnknownSource;
use crate::search::{SearchOptions, SearchResults, assemble_search, search_query};
use crate::{
    Config, Coverage, CoverageState, CoverageWindow, Hit, Locations, Rastro, Scope, Source,
};
use anyhow::Result;
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
    /// What storage recorded for the source, before the freshness rule is applied.
    fn coverage(&self, source: &Source, all_windows: bool) -> Result<Coverage>;
    fn search(&self, query: &str, limit: usize, history: bool) -> Result<Vec<Hit>>;
    /// Runs with no end and a window; recovery would turn each into "couldn't look: interrupted".
    fn unfinished_runs(&self) -> Result<Vec<UnfinishedRun>>;
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
    locations: Locations,
    config: Config,
    storage: Option<S>,
}

impl<S: ReadStorage> Rastro<S> {
    /// Opens for reading only. The config is read fresh, under a shared lock; nothing is
    /// migrated, projected or recovered, and no file is created. `open_storage` returns the
    /// store opened read-only, or `None` when its database file doesn't exist.
    pub fn open_read_only(
        locations: Locations,
        open_storage: impl FnOnce(&Path) -> Result<Option<S>>,
    ) -> Result<Reader<S>> {
        let lock = locations.shared_config_lock()?;
        let config = Config::load(&locations)?;
        drop(lock);
        let storage = open_storage(&locations.database())?;
        Ok(Reader {
            locations,
            config,
            storage,
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
        } = self;
        let sources = select(config, name)?;
        snapshot(locations, storage, |view, unfinished| {
            coverage_of(view, unfinished, &sources, all_windows)
        })
    }
    /// Full-text search over evidence quotes, each hit with its source's coverage.
    pub fn search(&mut self, text: &str, options: &SearchOptions) -> Result<SearchResults> {
        let Self {
            locations,
            config,
            storage,
        } = self;
        let sources = select(config, None)?;
        let query = search_query(text);
        snapshot(locations, storage, |view, unfinished| {
            let hits = match (&query, view) {
                (Some(query), Some(view)) => {
                    view.search(query, options.limit + 1, options.history)?
                }
                _ => Vec::new(),
            };
            let mut coverage = coverage_of(view, unfinished, &sources, false)?;
            for answer in &mut coverage {
                answer.windows.clear();
            }
            Ok(assemble_search(hits, coverage, options))
        })
    }
}

fn snapshot<S: ReadStorage, T>(
    locations: &Locations,
    storage: &mut Option<S>,
    work: impl FnOnce(Option<&dyn Snapshot>, &[UnfinishedRun]) -> Result<T>,
) -> Result<T> {
    let Some(storage) = storage else {
        return work(None, &[]);
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
            view.unfinished_runs()?
        };
        work(Some(view), &unfinished)
    })
}

fn select<'a>(config: &'a Config, name: Option<&str>) -> Result<Vec<&'a Source>> {
    let sources: Vec<_> = config
        .sources
        .iter()
        .filter(|s| name.is_none_or(|n| s.name == n))
        .collect();
    if sources.is_empty()
        && let Some(name) = name
    {
        return Err(UnknownSource(name.into()).into());
    }
    Ok(sources)
}

fn coverage_of(
    view: Option<&dyn Snapshot>,
    unfinished: &[UnfinishedRun],
    sources: &[&Source],
    all_windows: bool,
) -> Result<Vec<Coverage>> {
    let now = crate::coverage::now()?;
    sources
        .iter()
        .map(|source| {
            let mut coverage = match view {
                Some(view) => view.coverage(source, all_windows)?,
                None => Coverage {
                    source: source.name.clone(),
                    scope: source.scope.clone(),
                    state: CoverageState::NeverLooked,
                    last_checked: None,
                    fresh_until: None,
                    reason: None,
                    windows_total: 0,
                    windows: Vec::new(),
                },
            };
            // What startup recovery would record, computed instead of written.
            for run in unfinished
                .iter()
                .filter(|r| r.source == source.name && r.scope == source.scope)
            {
                coverage.windows_total += 1;
                coverage.windows.insert(
                    0,
                    CoverageWindow {
                        from: run.window.0,
                        to: run.window.1,
                        state: CoverageState::CouldNotLook,
                        selection: "all".into(),
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
            Ok(coverage.judged(source.freshness(), now))
        })
        .collect()
}
