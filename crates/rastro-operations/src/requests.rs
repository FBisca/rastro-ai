//! Request types: serde (JSON input), schemars (schema) and clap (command line) from one definition.
//! A doc comment is both the CLI help and the schema description.
use crate::{catalog::Surface, results::*};
use anyhow::Result;
use clap::{Args, Subcommand};
use rastro_core::{Rastro, Scope, Source, Storage};
use schemars::JsonSchema;
use serde::Deserialize;

/// Freshness rule of a source added without `--freshness-rule`.
pub const DEFAULT_FRESHNESS_RULE: &str = "1h";

/// An operation's name, the surfaces that expose it, and what it returns.
pub trait Operation {
    const NAME: &'static str;
    const SURFACES: &'static [Surface];
    type Output: JsonSchema + serde::Serialize;
}

fn default_freshness_rule() -> String {
    DEFAULT_FRESHNESS_RULE.into()
}

/// Register a source of evidence.
#[derive(Args, Debug, Deserialize, JsonSchema)]
pub struct SourceAddRequest {
    /// Name of the source.
    pub name: String,
    /// Connector plugin that reads the source.
    #[arg(long)]
    pub connector: String,
    /// Repository to read, as owner/repository.
    #[arg(long)]
    pub repo: String,
    /// Scope the source's evidence belongs to.
    #[arg(long)]
    pub scope: String,
    /// How long a check stays fresh, as a number and a unit (s, m, h, d).
    #[arg(long, default_value = DEFAULT_FRESHNESS_RULE)]
    #[serde(default = "default_freshness_rule")]
    pub freshness_rule: String,
    /// Start of the first window to read, as an RFC 3339 time; default is 90 days ago.
    #[arg(long)]
    #[serde(default)]
    pub window_start: Option<String>,
    /// Keep the full original of every observation.
    #[arg(long)]
    #[serde(default)]
    pub keep_original: bool,
}
impl Operation for SourceAddRequest {
    const NAME: &'static str = "source.add";
    const SURFACES: &'static [Surface] = &[Surface::Cli];
    type Output = SourceAddResult;
}

/// List the registered sources.
#[derive(Args, Debug, Deserialize, JsonSchema)]
pub struct SourceListRequest {}
impl Operation for SourceListRequest {
    const NAME: &'static str = "source.list";
    const SURFACES: &'static [Surface] = &[Surface::Cli];
    type Output = SourceListResult;
}

/// Show what each source has been looked at, and how fresh that is.
#[derive(Args, Debug, Deserialize, JsonSchema)]
pub struct CoverageRequest {
    /// Only this source; every source when omitted.
    #[serde(default)]
    pub name: Option<String>,
}
impl Operation for CoverageRequest {
    const NAME: &'static str = "coverage";
    const SURFACES: &'static [Surface] = &[Surface::Cli, Surface::Mcp];
    type Output = CoverageResult;
}

/// Run the connector of one source, or of every source.
#[derive(Args, Debug, Deserialize, JsonSchema)]
pub struct SyncRequest {
    /// Only this source; every source when omitted.
    #[serde(default)]
    pub name: Option<String>,
}
impl Operation for SyncRequest {
    const NAME: &'static str = "sync";
    const SURFACES: &'static [Surface] = &[Surface::Cli];
    type Output = SyncResult;
}

/// Full-text search over evidence, each result cited.
#[derive(Args, Debug, Deserialize, JsonSchema)]
pub struct SearchRequest {
    /// Words to search for.
    pub text: String,
    /// Every observation, not only the latest at each location.
    #[arg(long)]
    #[serde(default)]
    pub history: bool,
}
impl Operation for SearchRequest {
    const NAME: &'static str = "search";
    const SURFACES: &'static [Surface] = &[Surface::Cli, Surface::Mcp];
    type Output = SearchResponse;
}

/// Rebuild the full-text index from the stored evidence.
#[derive(Args, Debug, Deserialize, JsonSchema)]
pub struct IndexRebuildRequest {}
impl Operation for IndexRebuildRequest {
    const NAME: &'static str = "index.rebuild";
    const SURFACES: &'static [Surface] = &[Surface::Cli];
    type Output = IndexRebuildResult;
}

#[derive(Subcommand, Debug)]
pub enum SourceRequest {
    Add(SourceAddRequest),
    List(SourceListRequest),
}
#[derive(Subcommand, Debug)]
pub enum IndexRequest {
    Rebuild(IndexRebuildRequest),
}

/// A parsed command line: one request per operation.
#[derive(Subcommand, Debug)]
pub enum Request {
    /// Register and list sources.
    Source {
        #[command(subcommand)]
        command: SourceRequest,
    },
    Coverage(CoverageRequest),
    Sync(SyncRequest),
    /// Maintain the search index.
    Index {
        #[command(subcommand)]
        command: IndexRequest,
    },
    Search(SearchRequest),
}

impl Request {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Source {
                command: SourceRequest::Add(_),
            } => SourceAddRequest::NAME,
            Self::Source {
                command: SourceRequest::List(_),
            } => SourceListRequest::NAME,
            Self::Coverage(_) => CoverageRequest::NAME,
            Self::Sync(_) => SyncRequest::NAME,
            Self::Search(_) => SearchRequest::NAME,
            Self::Index {
                command: IndexRequest::Rebuild(_),
            } => IndexRebuildRequest::NAME,
        }
    }

    /// Opens the store and runs the operation.
    pub fn run<S: Storage>(self, rastro: &mut Rastro<S>) -> Result<Response> {
        Ok(match self {
            Self::Source {
                command: SourceRequest::Add(add),
            } => {
                let mut source = Source::github(add.name, add.repo, Scope(add.scope));
                source.connector = add.connector;
                source.freshness_rule = add.freshness_rule;
                source.keep_original = add.keep_original;
                if let Some(start) = add.window_start {
                    source.window_start = start.parse()?;
                }
                Response::SourceAdd(SourceAddResult {
                    source: rastro.add_source(source)?,
                })
            }
            Self::Source {
                command: SourceRequest::List(_),
            } => Response::SourceList(SourceListResult {
                sources: rastro.sources().to_vec(),
            }),
            Self::Coverage(request) => Response::Coverage(CoverageResult {
                coverage: rastro.coverage(request.name.as_deref())?,
            }),
            Self::Sync(request) => Response::Sync(SyncResult {
                runs: rastro.sync(request.name.as_deref())?,
            }),
            Self::Index {
                command: IndexRequest::Rebuild(_),
            } => Response::IndexRebuild(IndexRebuildResult {
                indexed: rastro.rebuild_index()?,
            }),
            Self::Search(request) => {
                let found = rastro.search(&request.text, request.history)?;
                Response::Search(SearchResponse {
                    query: request.text,
                    results: found.results,
                    coverage: found.coverage,
                })
            }
        })
    }
}
