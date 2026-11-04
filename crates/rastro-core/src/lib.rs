use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, env, fs, io::Write, path::PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Scope(pub String);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Source {
    pub name: String,
    pub connector: String,
    pub selector: BTreeMap<String, String>,
    pub scope: Scope,
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
            matches!(unit, "s" | "m" | "h" | "d") && number.parse::<u64>().is_ok_and(|n| n > 0),
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
            return Ok(Self {
                data: home.join("data"),
                config: home.join("config"),
            });
        }
        let directory = |key, fallback| -> Result<PathBuf> {
            match variable(key) {
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
    pub fn database(&self) -> PathBuf {
        self.data.join("rastro.sqlite3")
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
        fs::create_dir_all(&locations.config)?;
        let mut file = tempfile::NamedTempFile::new_in(&locations.config)?;
        file.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(locations.config_file())?;
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
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
#[derive(Debug, Serialize)]
pub struct Coverage {
    pub source: String,
    pub scope: Scope,
    pub state: CoverageState,
}

pub trait Storage {
    /// Config is authoritative; retain historical source rows referenced by evidence.
    fn project_sources(&mut self, sources: &[Source]) -> Result<()>;
    fn coverage(&self, source: &Source) -> Result<Coverage>;
}

pub struct Rastro<S> {
    locations: Locations,
    config: Config,
    storage: S,
}
impl<S: Storage> Rastro<S> {
    pub fn open(locations: Locations, mut storage: S) -> Result<Self> {
        let config = Config::load(&locations)?;
        storage.project_sources(&config.sources)?;
        Ok(Self {
            locations,
            config,
            storage,
        })
    }
    pub fn add_source(&mut self, source: Source) -> Result<Source> {
        source.validate()?;
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
    pub fn coverage(&self, name: Option<&str>) -> Result<Vec<Coverage>> {
        let sources: Vec<_> = self
            .config
            .sources
            .iter()
            .filter(|s| name.is_none_or(|n| s.name == n))
            .collect();
        if sources.is_empty()
            && let Some(name) = name
        {
            bail!("unknown source: {name}");
        }
        sources
            .into_iter()
            .map(|s| self.storage.coverage(s))
            .collect()
    }
}
