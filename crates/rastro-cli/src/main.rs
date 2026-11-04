use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use clap::{Parser, Subcommand};
use rastro_core::{
    Coverage, Locations, Rastro, Scope, Source, display_safe,
    runs::{RunReport, SearchResults},
};
use rastro_storage_sqlite::SqliteStorage;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "rastro", version)]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Operation,
}
#[derive(Subcommand)]
enum Operation {
    Source {
        #[command(subcommand)]
        command: SourceCommand,
    },
    Coverage {
        name: Option<String>,
    },
    /// Run the connector of one source, or of every source.
    Sync {
        name: Option<String>,
    },
    /// Maintain the search index.
    Index {
        #[command(subcommand)]
        command: IndexCommand,
    },
    /// Full-text search over evidence, each result cited.
    Search {
        text: String,
        /// Every observation, not only the latest at each location.
        #[arg(long)]
        history: bool,
    },
}
#[derive(Subcommand)]
enum IndexCommand {
    /// Rebuild the full-text index from the stored evidence.
    Rebuild,
}
#[derive(Subcommand)]
enum SourceCommand {
    Add {
        name: String,
        #[arg(long)]
        connector: String,
        #[arg(long)]
        repo: String,
        #[arg(long)]
        scope: String,
        #[arg(long, default_value = "1h")]
        freshness_rule: String,
        #[arg(long)]
        window_start: Option<String>,
        #[arg(long)]
        keep_original: bool,
    },
    List,
}
impl Operation {
    fn name(&self) -> &'static str {
        match self {
            Self::Source {
                command: SourceCommand::Add { .. },
            } => "source.add",
            Self::Source {
                command: SourceCommand::List,
            } => "source.list",
            Self::Coverage { .. } => "coverage",
            Self::Sync { .. } => "sync",
            Self::Search { .. } => "search",
            Self::Index {
                command: IndexCommand::Rebuild,
            } => "index.rebuild",
        }
    }
}
fn stamp(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Millis, true)
}
fn coverage_line(c: &Coverage) -> String {
    let mut line = format!(
        "{} | Scope: {} | Coverage: {}",
        c.source,
        c.scope.0,
        c.state.as_str()
    );
    if let Some(at) = c.last_checked {
        line += &format!(" | Last checked: {}", stamp(at));
    }
    if let Some(until) = c.fresh_until {
        line += &format!(" | Fresh until: {}", stamp(until));
    }
    if let Some(reason) = &c.reason {
        line += &format!(" | Reason: {}", display_safe(reason));
    }
    line
}
fn sync_line(run: &RunReport) -> String {
    let counts = &run.counts;
    let mut line = format!(
        "{} | {} | Plugin: {} {} | submitted {}, accepted {}, rejected {}, unchanged {} | Coverage: {}",
        run.source,
        run.status,
        run.plugin_id,
        run.plugin_version,
        counts.submitted,
        counts.accepted,
        counts.rejected,
        counts.unchanged,
        run.coverage.as_str()
    );
    if let Some(reason) = &run.reason {
        line += &format!(" | Reason: {}", display_safe(reason));
    }
    line
}
fn search_text(text: &str, found: &SearchResults) -> String {
    if found.results.is_empty() {
        let lines: Vec<_> = found.coverage.iter().map(coverage_line).collect();
        return format!("no evidence found for \"{text}\"\n{}", lines.join("\n"));
    }
    let blocks: Vec<_> = found
        .results
        .iter()
        .map(|r| {
            let e = &r.hit.evidence;
            let mut block = format!(
                "\"{}\"\n  Link: {} | Field: {} | Span: [{}, {}) | Captured: {} | Trust class: {} | Scope: {} | Source: {}",
                display_safe(&e.quote), display_safe(&e.location), display_safe(&e.field), e.start, e.end, stamp(e.captured_at),
                e.trust_class.as_str(), e.scope.0, e.source
            );
            if let Some(c) = &r.coverage {
                block += &format!("\n  Coverage: {}", coverage_line(c));
            }
            block
        })
        .collect();
    blocks.join("\n")
}
/// The output, its human form, and whether the operation succeeded.
type Outcome = (Value, String, bool);

fn execute(operation: Operation) -> Result<Outcome> {
    let (value, human) = run(operation)?;
    let ok = value["runs"]
        .as_array()
        .is_none_or(|runs| runs.iter().all(|r| r["status"] == "ok"));
    Ok((value, human, ok))
}
fn run(operation: Operation) -> Result<(Value, String)> {
    let locations = Locations::resolve()?;
    let storage = SqliteStorage::open(&locations.database())?;
    let mut rastro = Rastro::open(locations, storage)?;
    match operation {
        Operation::Source {
            command:
                SourceCommand::Add {
                    name,
                    connector,
                    repo,
                    scope,
                    freshness_rule,
                    window_start,
                    keep_original,
                },
        } => {
            let mut source = Source::github(name, repo, Scope(scope));
            source.connector = connector;
            source.freshness_rule = freshness_rule;
            source.keep_original = keep_original;
            if let Some(start) = window_start {
                source.window_start = start.parse()?;
            }
            let source = rastro.add_source(source)?;
            let human = format!("Source registered: {}", source.name);
            Ok((json!({"source": source}), human))
        }
        Operation::Source {
            command: SourceCommand::List,
        } => {
            let lines: Vec<_> = rastro.sources().iter().map(|s| format!(
                "{} | Connector: {} | selector: {} | Scope: {} | Freshness rule: {} | window start: {}",
                s.name, s.connector, serde_json::to_string(&s.selector).expect("string map"), s.scope.0, s.freshness_rule, s.window_start.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
            )).collect();
            let human = if lines.is_empty() {
                "No sources registered".into()
            } else {
                lines.join("\n")
            };
            Ok((json!({"sources": rastro.sources()}), human))
        }
        Operation::Coverage { name } => {
            let coverage = rastro.coverage(name.as_deref())?;
            let lines: Vec<_> = coverage.iter().map(coverage_line).collect();
            let human = if lines.is_empty() {
                "No sources registered".into()
            } else {
                lines.join("\n")
            };
            Ok((json!({"coverage": coverage}), human))
        }
        Operation::Sync { name } => {
            let runs = rastro.sync(name.as_deref())?;
            let lines: Vec<_> = runs.iter().map(sync_line).collect();
            let human = if lines.is_empty() {
                "No sources registered".into()
            } else {
                lines.join("\n")
            };
            Ok((json!({"runs": runs}), human))
        }
        Operation::Index {
            command: IndexCommand::Rebuild,
        } => {
            let indexed = rastro.rebuild_index()?;
            Ok((
                json!({"indexed": indexed}),
                format!("Index rebuilt: {indexed} observations"),
            ))
        }
        Operation::Search { text, history } => {
            let found = rastro.search(&text, history)?;
            let human = search_text(&text, &found);
            Ok((
                json!({"query": text, "results": found.results, "coverage": found.coverage}),
                human,
            ))
        }
    }
}
/// The operation a command line names, read without clap so a usage error can still say which.
fn named_operation(args: &[String]) -> Option<&'static str> {
    let mut words = args.iter().skip(1).filter(|a| !a.starts_with('-'));
    match (words.next()?.as_str(), words.next().map(String::as_str)) {
        ("source", Some("add")) => Some("source.add"),
        ("source", Some("list")) => Some("source.list"),
        ("coverage", _) => Some("coverage"),
        ("sync", _) => Some("sync"),
        ("search", _) => Some("search"),
        ("index", Some("rebuild")) => Some("index.rebuild"),
        _ => None,
    }
}
fn parse() -> (Cli, &'static str) {
    let args: Vec<String> = std::env::args().collect();
    match Cli::try_parse_from(&args) {
        Ok(cli) => {
            let operation = cli.command.name();
            (cli, operation)
        }
        Err(error) => {
            use clap::error::ErrorKind;
            let json = args
                .iter()
                .take_while(|a| *a != "--")
                .any(|a| a == "--json");
            let shown = !matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            );
            match named_operation(&args).filter(|_| json && shown) {
                Some(operation) => {
                    let text = error.to_string();
                    let first = text.lines().next().unwrap_or_default();
                    let message = first.strip_prefix("error: ").unwrap_or(first);
                    println!(
                        "{}",
                        json!({"schema_version": 1, "operation": operation, "error": format!("usage: {message}")})
                    );
                    std::process::exit(2);
                }
                None => error.exit(),
            }
        }
    }
}
fn main() {
    let (cli, operation) = parse();
    match execute(cli.command) {
        Ok((result, human, ok)) => {
            if cli.json {
                println!(
                    "{}",
                    json!({"schema_version": 1, "operation": operation, "result": result})
                );
            } else {
                println!("{human}");
            }
            if !ok {
                std::process::exit(1);
            }
        }
        Err(error) => {
            if cli.json {
                println!(
                    "{}",
                    json!({"schema_version": 1, "operation": operation, "error": format!("{error:#}")})
                );
            } else {
                eprintln!("error: {error:#}");
            }
            std::process::exit(1);
        }
    }
}
