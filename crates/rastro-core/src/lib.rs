pub mod claims;
pub mod coverage;
pub mod evidence;
pub mod files;
pub mod host;
pub mod read_only;
pub mod runs;
pub mod search;
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Duration, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, env, fs, io::Write, path::PathBuf};

/// Test-only knobs (`RASTRO_NOW`, `RASTRO_HOST_*`) are honoured only by debug builds, so a
/// stray variable can't change how a release build judges time or bounds a connector.
pub fn overrides_allowed(debug_build: bool) -> bool {
    debug_build
}
/// The value of a test-only knob, or `None` when this build ignores them.
pub(crate) fn test_override(name: &str) -> Option<String> {
    overrides_allowed(cfg!(debug_assertions))
        .then(|| env::var(name).ok())
        .flatten()
}

pub use evidence::{Appended, Evidence, Hit, TrustClass};
pub use rastro_plugin_sdk::Counts;
pub use read_only::{ReadStorage, Reader, Refusal, Snapshot, UnfinishedRun};
pub use search::{
    DEFAULT_SEARCH_LIMIT, EXCERPT_BYTES, MAX_SEARCH_LIMIT, Match, SearchOptions, SearchResult,
    SearchResults,
};

/// A command named a source that is unknown: for `coverage`, one neither in the config nor in
/// storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownSource(pub String);
impl std::fmt::Display for UnknownSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown source: {}", self.0)
    }
}
impl std::error::Error for UnknownSource {}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Scope(#[schemars(length(min = 1))] pub String);

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct Source {
    #[schemars(length(min = 1))]
    pub name: String,
    #[schemars(length(min = 1))]
    pub connector: String,
    pub selector: BTreeMap<String, String>,
    pub scope: Scope,
    #[schemars(regex(pattern = "^[0-9]+[smhd]$"))]
    pub freshness_rule: String,
    pub window_start: DateTime<Utc>,
    pub keep_original: bool,
}

impl Source {
    pub fn github(name: String, repo: String, scope: Scope) -> Self {
        Self {
            name,
            connector: "github".into(),
            selector: BTreeMap::from([("repo".into(), repo)]),
            scope,
            freshness_rule: "1h".into(),
            window_start: Utc::now() - Duration::days(90),
            keep_original: false,
        }
    }
    /// The freshness rule as a duration; `validate` guarantees the shape.
    pub fn freshness(&self) -> Duration {
        parse_freshness(&self.freshness_rule)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.trim().is_empty(),
            "source name must not be empty"
        );
        ensure!(
            !self.connector.trim().is_empty(),
            "connector must not be empty"
        );
        ensure!(!self.scope.0.trim().is_empty(), "scope must not be empty");
        if self.connector == "github" {
            let parts: Vec<_> = self
                .selector
                .get("repo")
                .context("github requires --repo")?
                .split('/')
                .collect();
            ensure!(
                parts.len() == 2 && parts.iter().all(|p| !p.trim().is_empty()),
                "repo must be owner/repository"
            );
        }
        let rule = &self.freshness_rule;
        ensure!(
            rule.len() > 1,
            "freshness rule must be a positive duration, e.g. 1h"
        );
        let (number, unit) = rule.split_at(rule.char_indices().last().unwrap().0);
        ensure!(
            matches!(unit, "s" | "m" | "h" | "d")
                && number.bytes().all(|b| b.is_ascii_digit())
                && number.parse::<u64>().is_ok_and(|n| n > 0),
            "freshness rule must be a positive duration, e.g. 1h"
        );
        Ok(())
    }
}

#[derive(Debug)]
pub struct Locations {
    pub data: PathBuf,
    pub config: PathBuf,
}
impl Locations {
    pub fn resolve() -> Result<Self> {
        let variable = |key| {
            env::var_os(key)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        };
        if let Some(home) = variable("RASTRO_HOME") {
            ensure!(home.is_absolute(), "RASTRO_HOME must be absolute");
            return Ok(Self {
                data: home.join("data"),
                config: home.join("config"),
            });
        }
        let directory = |key, fallback| -> Result<PathBuf> {
            match variable(key).filter(|path| path.is_absolute()) {
                Some(path) => Ok(path.join("rastro")),
                None => Ok(variable("HOME")
                    .context("HOME is required when XDG directories are unset")?
                    .join(fallback)),
            }
        };
        Ok(Self {
            data: directory("XDG_DATA_HOME", ".local/share/rastro")?,
            config: directory("XDG_CONFIG_HOME", ".config/rastro")?,
        })
    }
    fn config_lock(&self) -> Result<fs::File> {
        files::private_directory(&self.config)?;
        let file = files::private_file(&self.config.join("config.lock"))?;
        file.lock()?;
        Ok(file)
    }
    pub fn database(&self) -> PathBuf {
        self.data.join("rastro.sqlite3")
    }
    pub fn plugins(&self) -> PathBuf {
        self.config.join("plugins")
    }
    /// Held for as long as a sync runs, so a starting core can tell a crashed run from a live one.
    pub fn sync_lock(&self) -> PathBuf {
        self.data.join("sync.lock")
    }
    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.toml")
    }
}

#[derive(Default, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub sources: Vec<Source>,
}
impl Config {
    pub fn load(locations: &Locations) -> Result<Self> {
        let text = match fs::read_to_string(locations.config_file()) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.into()),
        };
        let config: Self = toml::from_str(&text).context("invalid config.toml")?;
        let mut names = std::collections::HashSet::new();
        for source in &config.sources {
            source.validate()?;
            ensure!(
                names.insert(&source.name),
                "duplicate source: {}",
                source.name
            );
        }
        Ok(config)
    }
    fn save(&self, locations: &Locations) -> Result<()> {
        files::private_directory(&locations.config)?;
        let mut file = tempfile::NamedTempFile::new_in(&locations.config)?;
        file.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(locations.config_file())?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub enum CoverageState {
    #[serde(rename = "never looked")]
    NeverLooked,
    #[serde(rename = "found")]
    Found,
    #[serde(rename = "empty")]
    Empty,
    #[serde(rename = "couldn't look")]
    CouldNotLook,
    #[serde(rename = "unknown")]
    Unknown,
}
impl CoverageState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NeverLooked => "never looked",
            Self::Found => "found",
            Self::Empty => "empty",
            Self::CouldNotLook => "couldn't look",
            Self::Unknown => "unknown",
        }
    }
}
/// A freshness rule as a duration. Stored rows are input, so a malformed rule (empty,
/// multi-byte, unknown unit) reads as zero: the pair is stale and answers "unknown".
fn parse_freshness(rule: &str) -> Duration {
    let Some(unit) = rule.chars().last() else {
        return Duration::zero();
    };
    let Ok(n) = rule[..rule.len() - unit.len_utf8()].parse::<i64>() else {
        return Duration::zero();
    };
    match unit {
        's' => Duration::seconds(n),
        'm' => Duration::minutes(n),
        'h' => Duration::hours(n),
        'd' => Duration::days(n),
        _ => Duration::zero(),
    }
}
/// What coverage is kept for: a source name in one scope, with the freshness rule that
/// judges it. A pair comes from the config or, once removed from it, from storage alone.
/// A pair no longer in the config takes the source's current freshness rule, the one stored
/// by name: a removed source keeps its last rule, and an old scope of a source takes the rule
/// the source has now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pair {
    pub source: String,
    pub scope: Scope,
    pub freshness_rule: String,
}
impl Pair {
    pub fn freshness(&self) -> Duration {
        parse_freshness(&self.freshness_rule)
    }
}
impl From<&Source> for Pair {
    fn from(source: &Source) -> Self {
        Self {
            source: source.name.clone(),
            scope: source.scope.clone(),
            freshness_rule: source.freshness_rule.clone(),
        }
    }
}
#[derive(Clone, Debug, JsonSchema, Serialize)]
pub struct Coverage {
    pub source: String,
    pub scope: Scope,
    /// Whether the source is in the config in this scope. A removed source, or one whose
    /// scope changed, keeps the coverage its stored windows give, with `registered: false`.
    pub registered: bool,
    pub state: CoverageState,
    /// When the source was last checked completely (found or empty).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked: Option<DateTime<Utc>>,
    /// Last complete check plus the freshness rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fresh_until: Option<DateTime<Utc>>,
    /// Why the latest attempt couldn't look, kept next to the last successful check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// How many windows the source has been looked at over: every window, listed or not.
    /// Search answers (per source and per hit) carry it too, with `windows` left empty.
    pub windows_total: usize,
    /// The newest windows the source was looked at, newest first (all of them when asked for). Empty in search answers.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub windows: Vec<CoverageWindow>,
}
/// How many windows `coverage` lists per source unless asked for every one.
pub const COVERAGE_WINDOWS_SHOWN: usize = 10;
/// One window `[from, to)` a run looked at, with what it found.
#[derive(Clone, Debug, JsonSchema, Serialize)]
pub struct CoverageWindow {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    /// A window records what a run saw, never "never looked" or "unknown".
    #[schemars(extend("enum" = ["found", "empty", "couldn't look"]))]
    pub state: CoverageState,
    pub selection: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<DateTime<Utc>>,
    pub checked_at: DateTime<Utc>,
}

/// One coverage row: what a run looked at, over the half-open window `[from, to)`.
pub struct CoverageRecord {
    pub run_id: i64,
    pub source: String,
    pub scope: Scope,
    pub window: (DateTime<Utc>, DateTime<Utc>),
    pub selection: String,
    pub state: CoverageState,
    pub reason: Option<String>,
    pub retry_at: Option<DateTime<Utc>>,
    pub checked_at: DateTime<Utc>,
}
/// How a run ended, as the core counted it.
pub struct RunEnd {
    pub status: String,
    pub reason: Option<String>,
    pub finished_at: DateTime<Utc>,
    pub counts: Counts,
}
/// The most recent complete check (found or empty) of a source.
pub struct CompleteCheck {
    pub window_end: DateTime<Utc>,
    pub checked_at: DateTime<Utc>,
}

pub trait Storage {
    /// Config is authoritative; retain historical source rows referenced by evidence.
    fn project_sources(&mut self, sources: &[Source]) -> Result<()>;
    /// The state is judged over every window; `all_windows` lists them all, otherwise only
    /// the newest `COVERAGE_WINDOWS_SHOWN`.
    /// Runs `work` against one read snapshot, so the pairs and every answer in it agree even
    /// when a sync in another process commits meanwhile.
    fn snapshot<T>(&self, work: impl FnOnce(&dyn Snapshot) -> Result<T>) -> Result<T>;
    fn start_run(
        &mut self,
        source: &Source,
        plugin: (&str, &str),
        started_at: DateTime<Utc>,
        window: (DateTime<Utc>, DateTime<Utc>),
    ) -> Result<i64>;
    /// Records what a run observed in one transaction: an observation identical to the latest
    /// one at its location and field is linked to the run, any other is appended (with its
    /// search index entry) and linked. Nothing stored is ever rewritten.
    fn append_evidence(&mut self, run_id: i64, evidence: &Evidence) -> Result<Appended>;
    fn record_coverage(&mut self, record: &CoverageRecord) -> Result<()>;
    fn finish_run(&mut self, run_id: i64, end: &RunEnd) -> Result<()>;
    fn last_complete_check(&self, source: &Source) -> Result<Option<CompleteCheck>>;
    /// Matches, one per location and field (the latest observation) unless `history` is set.
    fn search(&self, query: &str, limit: usize, history: bool) -> Result<Vec<Hit>>;
    /// Drops and rebuilds the full-text index from the stored evidence; returns how many
    /// observations it indexed. Evidence itself is untouched.
    fn rebuild_index(&mut self) -> Result<usize>;
    /// Marks every run that never finished as failed with reason `interrupted`; a connector
    /// run's window becomes "couldn't look", an extractor run adds no window.
    /// Returns how many it marked.
    fn interrupt_unfinished_runs(&mut self, at: DateTime<Utc>) -> Result<usize>;
    /// Records which extractor the source's connector binds it to.
    fn bind_extractor(&mut self, source: &str, extractor: Option<&str>) -> Result<()>;
    /// Logs an extractor run over one source; it has no window and no coverage.
    fn start_extraction(
        &mut self,
        source: &Source,
        plugin: (&str, &str),
        started_at: DateTime<Utc>,
    ) -> Result<i64>;
    /// The source's new observations past the local append checkpoint, sorted by
    /// (`captured_at`, evidence append id). Portable keys identify, but never order, input.
    fn pending_evidence(
        &self,
        extractor: (&str, &str),
        source: &Source,
    ) -> Result<Vec<(i64, Evidence)>>;
    /// A vocabulary definition change replays the pair even when ingestion added nothing.
    fn pending_evidence_for_vocabulary(
        &self,
        extractor: (&str, &str),
        vocabulary_version: &str,
        source: &Source,
    ) -> Result<Vec<(i64, Evidence)>> {
        let _ = vocabulary_version;
        self.pending_evidence(extractor, source)
    }
    /// Publishes accepted AND derivations and their checkpoint atomically. A successful
    /// version replay retires the pair's old justifications in the same transaction.
    /// Counts new and unchanged derivations; types and aliases are claims.
    fn record_claims(
        &mut self,
        run_id: i64,
        extractor: (&str, &str),
        claims: &[claims::ObservedClaim],
        checkpoint: Option<&claims::Checkpoint>,
    ) -> Result<Counts>;
    /// Whether the pair's extractor is running, last failed, or has evidence left to read, at
    /// this installed version.
    fn extraction_facts(
        &self,
        source: &Source,
        extractor: &str,
        version: &str,
    ) -> Result<runs::ExtractionFacts>;
    /// Bound sources, including retained historical sources, for a complete replay.
    fn extraction_sources(&self) -> Result<Vec<(Source, String)>>;
    /// Creates an empty private projection generation. Audit logs remain outside it.
    fn begin_claims_rebuild(&mut self) -> Result<()>;
    /// Atomically publishes the staged generation, or discards it on failure.
    fn finish_claims_rebuild(&mut self, publish: bool) -> Result<usize>;
    /// A claim and its supports, or `None` for an unknown id.
    fn why(&self, claim: &str) -> Result<Option<claims::Why>>;
}

pub struct Rastro<S> {
    locations: Locations,
    config: Config,
    storage: S,
}
impl<S: Storage> Rastro<S> {
    pub fn open(locations: Locations, mut storage: S) -> Result<Self> {
        let _lock = locations.config_lock()?;
        let config = Config::load(&locations)?;
        storage.project_sources(&config.sources)?;
        // A run still unfinished while no sync holds the lock was left by a crashed core.
        files::private_directory(&locations.data)?;
        let sync = files::private_file(&locations.sync_lock())?;
        // Readers hold the lock shared for a moment; wait them out (briefly) so recovery isn't
        // skipped. A live sync holds it exclusively, and `sync` recovers once it has the lock.
        for attempt in 0..50 {
            if sync.try_lock().is_ok() {
                storage.interrupt_unfinished_runs(Utc::now())?;
                break;
            }
            if attempt == 49 || sync.try_lock_shared().is_err() {
                break;
            }
            // Only shared holders: nobody is writing. Let go and try again.
            sync.unlock()?;
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        drop(sync);
        Ok(Self {
            locations,
            config,
            storage,
        })
    }
    pub fn add_source(&mut self, source: Source) -> Result<Source> {
        source.validate()?;
        let _lock = self.locations.config_lock()?;
        self.config = Config::load(&self.locations)?;
        ensure!(
            !self.config.sources.iter().any(|s| s.name == source.name),
            "source already exists: {}",
            source.name
        );
        self.config.sources.push(source.clone());
        if let Err(error) = self.config.save(&self.locations) {
            self.config.sources.pop();
            return Err(error);
        }
        self.storage.project_sources(&self.config.sources)?;
        Ok(source)
    }
    pub fn sources(&self) -> &[Source] {
        &self.config.sources
    }
    /// A claim with every support it has; an unknown id is an error.
    pub fn why(&self, claim: &str) -> Result<claims::Why> {
        self.storage
            .why(claim)?
            .with_context(|| format!("unknown claim: {claim}"))
    }
    pub fn coverage(&self, name: Option<&str>, all_windows: bool) -> Result<Vec<Coverage>> {
        self.storage.snapshot(|view| {
            read_only::coverage_of(
                Some(view),
                &[],
                &self.config.sources,
                name,
                all_windows,
                &[],
            )
        })
    }
}

/// Connector-supplied text made safe for a terminal: control characters escaped, length bounded.
pub fn display_safe(text: &str) -> String {
    const LIMIT: usize = 500;
    let mut out = String::new();
    for (index, c) in text.chars().enumerate() {
        if index == LIMIT {
            out.push('…');
            break;
        }
        push_escaped(&mut out, c);
    }
    out
}

/// Connector-supplied text made safe for a terminal, whole: control characters escaped, no clip.
pub fn display_escaped(text: &str) -> String {
    let mut out = String::new();
    text.chars().for_each(|c| push_escaped(&mut out, c));
    out
}

fn push_escaped(out: &mut String, c: char) {
    if c.is_control() {
        out.extend(c.escape_default());
    } else {
        out.push(c);
    }
}
