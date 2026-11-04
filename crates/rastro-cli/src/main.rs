use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use clap::Parser;
use rastro_core::{Coverage, CoverageWindow, Locations, Rastro, display_safe, runs::RunReport};
use rastro_operations::{Request, Response, catalog, envelope, results::SearchResponse};
use rastro_storage_sqlite::SqliteStorage;
use std::ffi::OsString;

#[derive(Parser)]
#[command(
    name = "rastro",
    version,
    about = "Collect evidence from sources and search it, each result cited"
)]
struct Cli {
    /// Print the result as a JSON envelope.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Request,
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
fn window_line(w: &CoverageWindow) -> String {
    let mut line = format!(
        "  [{}, {}) | {} | Selection: {}",
        stamp(w.from),
        stamp(w.to),
        w.state.as_str(),
        display_safe(&w.selection)
    );
    if let Some(reason) = &w.reason {
        line += &format!(" | Reason: {}", display_safe(reason));
    }
    if let Some(at) = w.retry_at {
        line += &format!(" | Retry at: {}", stamp(at));
    }
    line
}
fn coverage_block(c: &Coverage) -> String {
    let mut lines = vec![coverage_line(c)];
    if c.windows.len() < c.windows_total {
        lines.push(format!(
            "  Windows: showing {} of {} (--all-windows lists every one)",
            c.windows.len(),
            c.windows_total
        ));
    }
    lines.extend(c.windows.iter().map(window_line));
    lines.join("\n")
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
fn search_text(text: &str, found: &SearchResponse) -> String {
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
/// The result of an operation, its human form, and whether it succeeded.
type Outcome = (Response, String);

fn human(response: &Response) -> String {
    let or_none = |lines: Vec<String>| {
        if lines.is_empty() {
            "No sources registered".into()
        } else {
            lines.join("\n")
        }
    };
    match response {
        Response::SourceAdd(r) => format!("Source registered: {}", r.source.name),
        Response::SourceList(r) => or_none(r.sources.iter().map(|s| format!(
            "{} | Connector: {} | selector: {} | Scope: {} | Freshness rule: {} | window start: {}",
            s.name, s.connector, serde_json::to_string(&s.selector).expect("string map"), s.scope.0, s.freshness_rule, s.window_start.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
        )).collect()),
        Response::Coverage(r) => or_none(r.coverage.iter().map(coverage_block).collect()),
        Response::Sync(r) => or_none(r.runs.iter().map(sync_line).collect()),
        Response::IndexRebuild(r) => format!("Index rebuilt: {} observations", r.indexed),
        Response::Search(r) => search_text(&r.query, r),
    }
}
fn execute(request: Request) -> Result<Outcome> {
    let locations = Locations::resolve()?;
    let storage = SqliteStorage::open(&locations.database())?;
    let mut rastro = Rastro::open(locations, storage)?;
    let response = request.run(&mut rastro)?;
    let text = human(&response);
    Ok((response, text))
}
/// The operation a command line names, read without clap so a usage error can still say which.
fn named_operation(args: &[OsString]) -> Option<&'static str> {
    let words: Vec<_> = args
        .iter()
        .skip(1)
        .filter(|a| !a.as_encoded_bytes().starts_with(b"-"))
        .filter_map(|a| a.to_str())
        .collect();
    catalog::find_path(&words).map(|spec| spec.name)
}
fn parse() -> (Cli, &'static str) {
    let args: Vec<OsString> = std::env::args_os().collect();
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
                    println!("{}", envelope::usage_error(operation, message));
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
        Ok((response, text)) => {
            if cli.json {
                println!("{}", response.envelope());
            } else {
                println!("{text}");
            }
            if !response.succeeded() {
                std::process::exit(1);
            }
        }
        Err(error) => {
            if cli.json {
                println!("{}", envelope::failure(operation, format!("{error:#}")));
            } else {
                eprintln!("error: {error:#}");
            }
            std::process::exit(1);
        }
    }
}
