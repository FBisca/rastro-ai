use anyhow::{Result, ensure};
use clap::{Parser, Subcommand};
use rastro_core::{Locations, Rastro, Scope, Source};
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
        }
    }
}
fn execute(operation: Operation) -> Result<(Value, String)> {
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
            ensure!(connector == "github", "unsupported connector: {connector}");
            let mut source = Source::github(name, repo, Scope(scope));
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
            let lines: Vec<_> = coverage
                .iter()
                .map(|c| {
                    format!(
                        "{} | Scope: {} | Coverage: {}",
                        c.source,
                        c.scope.0,
                        c.state.as_str()
                    )
                })
                .collect();
            let human = if lines.is_empty() {
                "No sources registered".into()
            } else {
                lines.join("\n")
            };
            Ok((json!({"coverage": coverage}), human))
        }
    }
}
fn main() {
    let cli = Cli::parse();
    let operation = cli.command.name();
    match execute(cli.command) {
        Ok((result, human)) => {
            if cli.json {
                println!(
                    "{}",
                    json!({"schema_version": 1, "operation": operation, "result": result})
                );
            } else {
                println!("{human}");
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
