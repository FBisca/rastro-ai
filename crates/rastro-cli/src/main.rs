use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use clap::Parser;
use rastro_core::{
    Coverage, CoverageWindow, Locations, MAX_SEARCH_LIMIT, Rastro, display_escaped, display_safe,
    runs::{ExtractionState, RunReport},
};
use rastro_operations::{
    Request, Response, catalog, envelope,
    results::{SearchResponse, WhyResult},
};
use rastro_storage_sqlite::SqliteStorage;
use std::ffi::OsString;

mod mcp;

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
    command: Command,
}
#[derive(clap::Subcommand)]
enum Command {
    #[command(flatten)]
    Operation(Request),
    /// Serve search and coverage to an MCP client over stdio.
    Mcp {
        /// Only expose these scopes; repeat to allow more than one.
        #[arg(long = "scope")]
        scopes: Vec<String>,
    },
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
    if !c.registered {
        line += " | Not in config";
    }
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
    if let Some(e) = &run.extraction {
        line += &format!(
            "\n  Extractor: {} {} | {} | evidence {} | claims submitted {}, accepted {}, rejected {}, unchanged {}",
            display_safe(&e.extractor),
            e.extractor_version,
            e.state.as_str(),
            e.evidence,
            e.counts.submitted,
            e.counts.accepted,
            e.counts.rejected,
            e.counts.unchanged
        );
    }
    line
}
fn extraction_line(pair: &rastro_core::runs::ExtractionPair) -> String {
    format!(
        "{} | Scope: {} | Extraction: {} | Extractor: {} {}",
        display_safe(&pair.source),
        display_safe(&pair.scope),
        pair.state.as_str(),
        display_safe(&pair.extractor),
        display_safe(pair.extractor_version.as_deref().unwrap_or("unknown"))
    )
}
fn claim_line(summary: &rastro_core::knowledge::ClaimSummary) -> String {
    let c = &summary.claim;
    let aliases = |identifier: &str, names: &[String]| {
        let mut shown = display_safe(identifier);
        if !names.is_empty() {
            shown += &format!(
                " ({})",
                names
                    .iter()
                    .map(|s| display_safe(s))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        shown
    };
    let object = match &c.object {
        rastro_core::claims::Object::Entity { identifier, .. } => {
            aliases(identifier, &summary.object_aliases)
        }
        rastro_core::claims::Object::Literal { value, .. } => {
            format!("\"{}\"", display_safe(value))
        }
    };
    format!(
        "{} {} {} | Status: {} | Claim: {} | Revision: {} | Supports: {}",
        aliases(&c.subject.identifier, &summary.subject_aliases),
        display_safe(&c.predicate),
        object,
        c.status,
        c.id,
        c.revision,
        summary.support_count
    )
}
fn about_text(about: &rastro_operations::results::AboutResult) -> String {
    let mut lines: Vec<_> = about.claims.iter().map(claim_line).collect();
    if lines.is_empty() {
        lines.push("No claims found".into());
    }
    for group in &about.groups {
        lines.push(format!(
            "Group of {} equivalent claims: {} | Status: {} | Revision: {}",
            group.members.len(),
            group
                .members
                .iter()
                .map(|m| m.claim.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            group.status,
            group.revision
        ));
    }
    if about.truncated {
        lines.push("More claims exist; raise --limit (at most 50)".into());
    }
    lines.extend(about.coverage.iter().map(coverage_line));
    lines.extend(about.extraction.iter().map(extraction_line));
    lines.join("\n")
}
fn why_text(why: &WhyResult) -> String {
    let c = &why.claim.claim;
    let mut lines = vec![claim_line(&why.claim)];
    lines.push("  Valid time: unknown".into());
    for interval in &c.known_intervals {
        lines.push(format!(
            "  Known: [{}, {})",
            stamp(interval.start),
            interval.end.map(stamp).unwrap_or_else(|| "current".into())
        ));
    }
    for (kind, links) in [
        ("Conflicts with", &c.conflicts_with),
        ("Supersedes", &c.supersedes),
        ("Superseded by", &c.superseded_by),
    ] {
        if !links.is_empty() {
            lines.push(format!("  {kind}: {}", links.join(", ")));
        }
    }
    for support in &why.supports {
        if let Some(intent) = &support.intent {
            lines.push(format!(
                "  Justification: {} | Kind: {} | Intent: {} {} | Issued: {} | Issuer: {} via {}",
                support.justification_id,
                display_safe(&support.justification_kind),
                display_safe(&support.extraction_rule),
                display_safe(&intent.id),
                stamp(intent.issued_at),
                display_safe(&intent.issuer.principal),
                display_safe(&intent.issuer.surface)
            ));
            continue;
        }
        lines.push(format!(
            "  Justification: {} | Kind: {} | Extractor: {} {} | Rule: {} | Vocabulary: {} | Current: {}",
            support.justification_id,
            display_safe(&support.justification_kind),
            display_safe(&support.extractor),
            display_safe(&support.extractor_version),
            display_safe(&support.extraction_rule),
            display_safe(&support.vocabulary_version),
            support.current
        ));
        for s in &support.spans {
            let shown = s.quote.as_deref().unwrap_or(&s.excerpt.text);
            lines.push(format!("  \"{}{}{}\" | Link: {} | Field: {} | Span: [{}, {}) | Captured: {} | Source updated: {} | Trust class: {} | Scope: {} | Source: {}",
                if s.quote.is_none() && s.excerpt.clipped_before { "…" } else { "" }, display_safe(shown), if s.quote.is_none() && s.excerpt.clipped_after { "…" } else { "" },
                display_safe(&s.location), display_safe(&s.field), s.start, s.end, stamp(s.captured_at), s.source_updated_at.map(stamp).unwrap_or_else(|| "unknown".into()), s.trust_class.as_str(), display_safe(&s.scope.0), display_safe(&s.source)));
        }
    }
    if why.truncated {
        lines.push("More justifications exist; raise --limit (at most 50)".into());
    }
    lines.extend(why.coverage.iter().map(coverage_line));
    lines.extend(why.extraction.iter().map(extraction_line));
    lines.join("\n")
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
            // The whole quote when `--full` asked for it; else the excerpt, with … where cut.
            let shown = match &r.quote {
                Some(quote) => display_escaped(quote),
                None => format!(
                    "{}{}{}",
                    if r.matched.clipped_before { "…" } else { "" },
                    display_safe(&r.matched.text),
                    if r.matched.clipped_after { "…" } else { "" }
                ),
            };
            let mut block = format!(
                "\"{shown}\"\n  Link: {} | Field: {} | Span: [{}, {}) | Captured: {} | Trust class: {} | Scope: {} | Source: {}",
                display_safe(&r.location), display_safe(&r.field), r.start, r.end, stamp(r.captured_at),
                r.trust_class.as_str(), r.scope.0, r.source
            );
            if let Some(c) = &r.coverage {
                block += &format!("\n  Coverage: {}", coverage_line(c));
            }
            block
        })
        .collect();
    let mut text = blocks.join("\n");
    if found.truncated {
        text += &format!(
            "\nMore evidence matches than the {} shown; raise --limit (at most {MAX_SEARCH_LIMIT}) or narrow the query",
            found.results.len()
        );
    }
    text
}
fn intent_line(intent: &rastro_core::identity::IntentView) -> String {
    let mut line = format!(
        "{} | {} | Intent: {} | Issued: {} | Status: {}",
        intent.append_seq,
        intent.kind.as_str(),
        display_safe(&intent.id),
        stamp(intent.issued_at),
        serde_json::to_value(intent.status)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default()
    );
    if let Some(by) = &intent.status_by {
        line += &format!(" by {}", display_safe(by));
    }
    if let Some(undone) = &intent.undo_of {
        line += &format!(" | Undoes: {}", display_safe(undone));
    }
    line += &format!(" | Domain: {}", display_safe(&intent.domain_key));
    line
}
fn intents_text(intents: &[rastro_core::identity::IntentView], none: &str) -> String {
    if intents.is_empty() {
        return none.into();
    }
    intents
        .iter()
        .map(intent_line)
        .collect::<Vec<_>>()
        .join("\n")
}
fn person_text(result: &rastro_operations::results::IdentityPersonResult) -> String {
    let mut text = format!(
        "Person: {} | Id: {}",
        display_safe(&result.person.identifier),
        result.person.id
    );
    if !result.created {
        text += " | Already existed; nothing recorded";
    }
    for intent in &result.intents {
        text += &format!("\n{}", intent_line(intent));
    }
    text
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
        Response::ClaimsRebuild(r) => {
            let mut text = format!("Claims rebuilt: {} claims", r.claims);
            // A pair that is not current never started; its stored output was carried over.
            for pair in r.pairs.iter().filter(|p| p.state != ExtractionState::Current) {
                text += &format!(
                    "\n  Carried over: {} | {} | Extractor: {} {} | {}",
                    pair.source,
                    pair.scope,
                    display_safe(&pair.extractor),
                    pair.extractor_version,
                    pair.state.as_str(),
                );
                if let Some(reason) = pair.reason.as_deref().filter(|r| *r != pair.state.as_str()) {
                    text += &format!(" | Reason: {}", display_safe(reason));
                }
            }
            text
        }
        Response::IndexRebuild(r) => format!("Index rebuilt: {} observations", r.indexed),
        Response::Search(r) => search_text(&r.query, r),
        Response::Why(r) => why_text(r),
        Response::About(r) => about_text(r),
        Response::IdentityMe(r) | Response::IdentityPersonCreate(r) => person_text(r),
        Response::IdentityLink(r) | Response::IdentityMerge(r) | Response::IdentityUndo(r) => {
            intents_text(&r.intents, "Already in force; nothing recorded")
        }
        Response::IdentityHistory(r) => match &r.entity {
            Some(entity) => format!(
                "{}\n{}",
                display_safe(&entity.identifier),
                intents_text(&r.intents, "No intents name it")
            ),
            None => "No entity matches".into(),
        },
    }
}
fn execute(request: Request) -> Result<Outcome> {
    let locations = Locations::resolve()?;
    if catalog::find(request.name()).is_some_and(|spec| spec.read_only && spec.read_path) {
        let mut reader = Rastro::open_read_only(locations, SqliteStorage::open_read_only)?;
        let response = request.run_read(&mut reader)?;
        let text = human(&response);
        return Ok((response, text));
    }
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
            let operation = match &cli.command {
                Command::Operation(request) => request.name(),
                Command::Mcp { .. } => "mcp",
            };
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
    let request = match cli.command {
        Command::Operation(request) => request,
        Command::Mcp { scopes } => match mcp::run(scopes) {
            Ok(()) => return,
            Err(error) => {
                eprintln!("error: {}", mcp::bounded(&format!("{error:#}")));
                std::process::exit(1);
            }
        },
    };
    match execute(request) {
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
