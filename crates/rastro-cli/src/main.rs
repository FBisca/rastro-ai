use anyhow::Result;
use clap::Parser;
use rastro_core::{
    Coverage, CoverageWindow, Locations, MAX_SEARCH_LIMIT, Rastro, display_escaped, display_safe,
    runs::{ExtractionState, RunReport},
};
use rastro_operations::{
    Request, Response, catalog, envelope,
    requests::{SourceCheckRequest, SourceRequest},
    results::{SearchResponse, WhyResult},
};
use rastro_storage_sqlite::SqliteStorage;
use std::ffi::OsString;

mod check;
mod mcp;
mod render;
mod tick;

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
    /// Sync every source past its freshness rule; a schedule calls this.
    Tick,
    /// Check that a source's credentials are accepted, with one cheap call and nothing stored.
    Check {
        #[command(flatten)]
        check: SourceCheckRequest,
    },
    /// Work with installed plugins.
    Plugin {
        #[command(subcommand)]
        command: PluginCommand,
    },
}
#[derive(clap::Subcommand)]
enum PluginCommand {
    /// Run an installed connector once through the host's validation, storing nothing, and
    /// name every protocol violation. Exits zero when the connector conforms.
    Check {
        /// The plugin id its manifest declares.
        id: String,
        /// A source option as key=value; repeat for more than one.
        #[arg(long = "option", value_name = "KEY=VALUE")]
        options: Vec<String>,
    },
}
/// One source's coverage as a block: its state and times, then each window as a field.
fn coverage_block(c: &Coverage) -> render::Block {
    let mut block = coverage_state_block(c);
    if c.windows.len() < c.windows_total {
        block = block.field(
            "Windows",
            format!(
                "showing {} of {} (--all-windows lists every one)",
                c.windows.len(),
                c.windows_total
            ),
        );
    }
    for window in &c.windows {
        block = block.field("Window", window_value(window));
    }
    block
}
/// One source's coverage as a search answer shows it: its state and times under its name, with
/// no window list — a search answer carries no windows.
fn coverage_state_block(c: &Coverage) -> render::Block {
    let header = if c.registered {
        display_safe(&c.source)
    } else {
        format!("{} (Not in config)", display_safe(&c.source))
    };
    render::Block::new(header)
        .field("Scope", display_safe(&c.scope.0))
        .field("Coverage", c.state.as_str())
        .maybe("Last checked", c.last_checked.map(render::time))
        .maybe("Fresh until", c.fresh_until.map(render::time))
        .maybe("Reason", c.reason.as_deref().map(display_safe))
}
/// One coverage window as its source block's field: the half-open interval, what it found,
/// its selection, and what a failed or partial look adds.
fn window_value(w: &CoverageWindow) -> String {
    let mut value = format!(
        "[{}, {}) {}",
        render::time(w.from),
        render::time(w.to),
        w.state.as_str()
    );
    value += &format!(", Selection: {}", display_safe(&w.selection));
    if let Some(reason) = &w.reason {
        value += &format!(", Reason: {}", display_safe(reason));
    }
    for gap in &w.gaps {
        value += &format!(
            ", Gap: {} ({})",
            display_safe(&gap.selection),
            display_safe(&gap.reason)
        );
    }
    if let Some(at) = w.retry_at {
        value += &format!(", Retry at: {}", render::time(at));
    }
    value
}
/// One source as a block: its name, then the fields its registration saved.
fn source_block(s: &rastro_core::Source) -> render::Block {
    render::Block::new(display_safe(&s.name))
        .field("Connector", display_safe(&s.connector))
        .field(
            "Selector",
            serde_json::to_string(&s.selector).expect("string map"),
        )
        .field("Scope", display_safe(&s.scope.0))
        .field("Freshness rule", &s.freshness_rule)
        .field("Window start", render::time(s.window_start))
}
/// One sync run as a block: how it ended, its plugin and tally, and the extraction it ran.
fn sync_block(run: &RunReport) -> render::Block {
    let mut block = render::Block::new(display_safe(&run.source))
        .field("Status", &run.status)
        .field(
            "Plugin",
            format!(
                "{} {}",
                display_safe(&run.plugin_id),
                display_safe(&run.plugin_version)
            ),
        )
        .field("Counts", tally(&run.counts))
        .field("Coverage", run.coverage.as_str())
        .maybe("Reason", run.reason.as_deref().map(display_safe));
    if let Some(e) = &run.extraction {
        block = block
            .field(
                "Extractor",
                format!(
                    "{} {}",
                    display_safe(&e.extractor),
                    display_safe(&e.extractor_version)
                ),
            )
            .field("Extraction", e.state.as_str())
            .field("Evidence", e.evidence)
            .field("Claims", tally(&e.counts));
    }
    block
}
/// One tally as a field value: the four counts in the order the owner reads them.
fn tally(counts: &rastro_core::Counts) -> String {
    format!(
        "submitted {}, accepted {}, rejected {}, unchanged {}",
        counts.submitted, counts.accepted, counts.rejected, counts.unchanged
    )
}
/// One source-and-scope's extraction state as a block: its name, then the state and extractor.
fn extraction_block(pair: &rastro_core::runs::ExtractionPair) -> render::Block {
    render::Block::new(display_safe(&pair.source))
        .field("Scope", display_safe(&pair.scope))
        .field("Extraction", pair.state.as_str())
        .field(
            "Extractor",
            format!(
                "{} {}",
                display_safe(&pair.extractor),
                display_safe(pair.extractor_version.as_deref().unwrap_or("unknown"))
            ),
        )
}
/// One claim as a block: its subject, predicate and object — each entity with its aliases and
/// cited label — as the header, then its status, id, revision and support count as fields.
fn claim_block(summary: &rastro_core::knowledge::ClaimSummary) -> render::Block {
    let c = &summary.claim;
    // An entity as its identifier, its aliases, then its label: the cited title or name, and its
    // state. All of it is source text, so every part is escaped.
    let entity =
        |identifier: &str, names: &[String], label: &Option<rastro_core::knowledge::Label>| {
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
            if let Some(label) = label {
                shown += &format!(
                    " \"{}{}\"",
                    display_safe(&label.text),
                    if label.clipped { "…" } else { "" }
                );
                if let Some(state) = &label.state {
                    shown += &format!(" [{}]", display_safe(state));
                }
            }
            shown
        };
    let object = match &c.object {
        rastro_core::claims::Object::Entity { identifier, .. } => {
            entity(identifier, &summary.object_aliases, &summary.object_label)
        }
        rastro_core::claims::Object::Literal { value, .. } => {
            format!("\"{}\"", display_safe(value))
        }
    };
    let header = format!(
        "{} {} {}",
        entity(
            &c.subject.identifier,
            &summary.subject_aliases,
            &summary.subject_label
        ),
        display_safe(&c.predicate),
        object
    );
    render::Block::new(header)
        .field("Status", &c.status)
        .field("Claim", &c.id)
        .field("Revision", &c.revision)
        .field("Supports", summary.support_count)
}
fn scopes_shown(scopes: &[rastro_core::Scope]) -> String {
    if scopes.is_empty() {
        return "none".into();
    }
    scopes
        .iter()
        .map(|s| display_safe(&s.0))
        .collect::<Vec<_>>()
        .join(", ")
}
/// What a read as of a past time says of itself: that it is a reconstruction, and the sources it
/// had not read yet.
fn reconstruction_lines(
    reconstruction: Option<&rastro_core::knowledge::Reconstruction>,
) -> Vec<String> {
    let Some(reconstruction) = reconstruction else {
        return Vec::new();
    };
    let mut lines = vec![display_safe(&reconstruction.statement)];
    lines.extend(reconstruction.not_read_yet.iter().map(|unread| {
        format!(
            "{}: not read yet at this time (scope {})",
            display_safe(&unread.source),
            display_safe(&unread.scope.0)
        )
    }));
    lines
}
fn about_text(about: &rastro_operations::results::AboutResult) -> String {
    let mut pieces: Vec<String> = Vec::new();
    let reconstruction = reconstruction_lines(about.reconstruction.as_ref());
    if !reconstruction.is_empty() {
        pieces.push(reconstruction.join("\n"));
    }
    if !about.resolved {
        pieces.push("No entity matches".into());
    }
    let mut accounts = std::collections::BTreeMap::new();
    if let Some(person) = &about.person {
        pieces.push(
            render::Block::new(format!("Person: {}", display_safe(&person.identifier)))
                .field("Id", &person.id)
                .to_string(),
        );
        for account in &person.accounts {
            accounts.insert(account.id.as_str(), account.identifier.as_str());
            let mut block =
                render::Block::new(format!("Account: {}", display_safe(&account.identifier)))
                    .field("Type", display_safe(&account.entity_type));
            if !account.aliases.is_empty() {
                block = block.field(
                    "Aliases",
                    account
                        .aliases
                        .iter()
                        .map(|a| display_safe(a))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
            for reason in &account.reasons {
                let kind = serde_json::to_value(reason.kind)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let mut value = kind;
                if let Some(intent) = &reason.intent {
                    value += &format!(" {}", display_safe(intent));
                }
                value += &format!(" (claim {})", reason.claim);
                block = block.field("Linked by", value);
            }
            pieces.push(block.to_string());
        }
    }
    for claim in &about.claims {
        let mut block = claim_block(&claim.summary);
        if let Some(account) = &claim.account {
            block = block.field(
                "Account",
                display_safe(accounts.get(account.as_str()).copied().unwrap_or(account)),
            );
        }
        pieces.push(block.to_string());
    }
    if about.resolved && about.claims.is_empty() {
        pieces.push("No claims found".into());
    }
    for group in &about.groups {
        pieces.push(
            render::Block::new(format!(
                "Group of {} equivalent claims: {}",
                group.members.len(),
                group
                    .members
                    .iter()
                    .map(|m| m.claim.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
            .field("Status", &group.status)
            .field("Revision", &group.revision)
            .to_string(),
        );
    }
    if about.truncated {
        pieces.push("More claims exist; raise --limit (at most 50)".into());
    }
    pieces.extend(about.coverage.iter().map(|c| coverage_block(c).to_string()));
    pieces.extend(
        about
            .extraction
            .iter()
            .map(|e| extraction_block(e).to_string()),
    );
    pieces.join("\n\n")
}
fn why_text(why: &WhyResult) -> String {
    let c = &why.claim.claim;
    let mut pieces: Vec<String> = Vec::new();
    let reconstruction = reconstruction_lines(why.reconstruction.as_ref());
    if !reconstruction.is_empty() {
        pieces.push(reconstruction.join("\n"));
    }
    let mut claim = claim_block(&why.claim).field("Valid time", "unknown");
    for interval in &c.known_intervals {
        claim = claim.field(
            "Known",
            format!(
                "[{}, {})",
                render::time(interval.start),
                interval
                    .end
                    .map(render::time)
                    .unwrap_or_else(|| "current".into())
            ),
        );
    }
    for (kind, links) in [
        ("Conflicts with", &c.conflicts_with),
        ("Supersedes", &c.supersedes),
        ("Superseded by", &c.superseded_by),
    ] {
        if !links.is_empty() {
            claim = claim.field(kind, links.join(", "));
        }
    }
    pieces.push(claim.to_string());
    for support in &why.supports {
        pieces.push(justification_block(support).to_string());
        // An owner-intent justification names its intent; the spans of a provenance justification
        // follow it as their own blocks.
        if support.intent.is_none() {
            pieces.extend(support.spans.iter().map(|s| citation_block(s).to_string()));
        }
    }
    if why.truncated {
        pieces.push("More justifications exist; raise --limit (at most 50)".into());
    }
    pieces.extend(why.coverage.iter().map(|c| coverage_block(c).to_string()));
    pieces.extend(
        why.extraction
            .iter()
            .map(|e| extraction_block(e).to_string()),
    );
    pieces.join("\n\n")
}
/// One justification as a block: its id and kind, then its provenance — an owner intent names the
/// intent and its issuer; an extraction names its extractor, rule and vocabulary.
fn justification_block(support: &rastro_core::knowledge::JustificationView) -> render::Block {
    let block = render::Block::new(format!("Justification: {}", support.justification_id))
        .field("Kind", display_safe(&support.justification_kind));
    if let Some(intent) = &support.intent {
        return block
            .field("Rule", display_safe(&support.extraction_rule))
            .field("Intent", display_safe(&intent.id))
            .field("Issued", render::time(intent.issued_at))
            .field(
                "Issuer",
                format!(
                    "{} via {}",
                    display_safe(&intent.issuer.principal),
                    display_safe(&intent.issuer.surface)
                ),
            )
            .field("Scopes", scopes_shown(&intent.issuer.scopes))
            .field("Current", support.current);
    }
    block
        .field(
            "Extractor",
            format!(
                "{} {}",
                display_safe(&support.extractor),
                display_safe(&support.extractor_version)
            ),
        )
        .field("Rule", display_safe(&support.extraction_rule))
        .field("Vocabulary", display_safe(&support.vocabulary_version))
        .field("Current", support.current)
}
/// One cited span as a block: its quote verbatim and escaped with the trust class beside it, then
/// every citation field on its own indented line.
fn citation_block(s: &rastro_core::knowledge::Citation) -> render::Block {
    // The whole quote when `--full` asked for it; else the excerpt, with … where cut.
    let shown = match &s.quote {
        Some(quote) => display_escaped(quote),
        None => format!(
            "{}{}{}",
            if s.excerpt.clipped_before { "…" } else { "" },
            display_safe(&s.excerpt.text),
            if s.excerpt.clipped_after { "…" } else { "" }
        ),
    };
    render::Block::new(format!("\"{shown}\""))
        .field("Trust class", s.trust_class.as_str())
        .field("Link", display_safe(&s.location))
        .field("Field", display_safe(&s.field))
        .field("Span", format!("[{}, {})", s.start, s.end))
        .field("Captured", render::time(s.captured_at))
        .field(
            "Source updated",
            s.source_updated_at
                .map(render::time)
                .unwrap_or_else(|| "unknown".into()),
        )
        .field("Scope", display_safe(&s.scope.0))
        .field("Source", display_safe(&s.source))
}
fn search_text(text: &str, found: &SearchResponse) -> String {
    if found.results.is_empty() {
        let mut text = render::empty(&format!("evidence found for \"{text}\""));
        if !found.coverage.is_empty() {
            text += &format!(
                "\n\n{}",
                render::records(found.coverage.iter().map(coverage_state_block))
            );
        }
        return text;
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
            let mut block = render::Block::new(format!("\"{shown}\""))
                .field("Trust class", r.trust_class.as_str())
                .field("Link", display_safe(&r.location))
                .field("Field", display_safe(&r.field))
                .field("Span", format!("[{}, {})", r.start, r.end))
                .field("Captured", render::time(r.captured_at))
                .field("Scope", display_safe(&r.scope.0))
                .field("Source", display_safe(&r.source));
            if let Some(c) = &r.coverage {
                let state = if c.registered {
                    c.state.as_str().to_string()
                } else {
                    format!("{} (Not in config)", c.state.as_str())
                };
                block = block
                    .field("Coverage", state)
                    .maybe("Last checked", c.last_checked.map(render::time))
                    .maybe("Fresh until", c.fresh_until.map(render::time))
                    .maybe("Reason", c.reason.as_deref().map(display_safe));
            }
            block
        })
        .collect();
    let mut text = render::records(blocks);
    if found.truncated {
        text += &format!(
            "\n\nMore evidence matches than the {} shown; raise --limit (at most {MAX_SEARCH_LIMIT}) or narrow the query",
            found.results.len()
        );
    }
    text
}
/// One owner intent as a block: its id on the header line, then its acceptance order, kind,
/// when it was issued and where it stands, what it undoes, and who issued it over which scopes.
fn intent_block(intent: &rastro_core::identity::IntentView) -> render::Block {
    let status = serde_json::to_value(intent.status)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();
    render::Block::new(format!("Intent {}", display_safe(&intent.id)))
        .field("Seq", intent.append_seq)
        .field("Kind", intent.kind.as_str())
        .field("Issued", render::time(intent.issued_at))
        .field("Status", status)
        .maybe("Status by", intent.status_by.as_deref().map(display_safe))
        .maybe("Undoes", intent.undo_of.as_deref().map(display_safe))
        .field("Domain", display_safe(&intent.domain_key))
        .field(
            "Issuer",
            format!(
                "{} via {}",
                display_safe(&intent.issuer.principal),
                display_safe(&intent.issuer.surface)
            ),
        )
        .field("Scopes", scopes_shown(&intent.issuer.scopes))
}
fn intents_text(intents: &[rastro_core::identity::IntentView], none: &str) -> String {
    if intents.is_empty() {
        return none.into();
    }
    render::records(intents.iter().map(intent_block))
}
fn person_text(result: &rastro_operations::results::IdentityPersonResult) -> String {
    let mut text = render::Block::new(format!(
        "Person: {}",
        display_safe(&result.person.identifier)
    ))
    .field("Id", &result.person.id)
    .to_string();
    if !result.intents.is_empty() {
        text += &format!(
            "\n\n{}",
            render::records(result.intents.iter().map(intent_block))
        );
    }
    if !result.created {
        text += &format!("\n\n{}", render::nothing_recorded("Already existed"));
    }
    text
}
/// One block per candidate pair: both accounts (with the Persons holding them) on the header
/// line, then each reason with the value each account gave, and the name a yes would give a new
/// Person.
fn candidates_text(result: &rastro_operations::results::IdentityCandidatesResult) -> String {
    if result.candidates.is_empty() {
        return render::empty("candidates");
    }
    let account = |a: &rastro_core::identity::CandidateAccount| {
        let mut text = display_safe(&a.identifier);
        if !a.held_by.is_empty() {
            let held: Vec<String> = a.held_by.iter().map(|p| display_safe(p)).collect();
            text += &format!(" (held by {})", held.join(", "));
        }
        text
    };
    let reason = |r: &rastro_core::identity::CandidateReason| {
        let name = serde_json::to_value(r.reason).ok();
        format!(
            "{}: {} = {}",
            name.as_ref().and_then(|v| v.as_str()).unwrap_or_default(),
            display_safe(&r.values[0]),
            display_safe(&r.values[1])
        )
    };
    render::records(result.candidates.iter().map(|c| {
        let mut block = render::Block::new(format!(
            "{} ~ {}",
            account(&c.accounts[0]),
            account(&c.accounts[1])
        ));
        for r in &c.reasons {
            block = block.field("Reason", reason(r));
        }
        block.field("Name", display_safe(&c.name))
    }))
}
fn party_text(party: &rastro_core::meeting::Party) -> String {
    let label = party.label.as_deref().map(display_safe);
    match (&party.identity, label) {
        (Some(identity), Some(label)) => {
            format!("{} ({label})", display_safe(&identity.identifier))
        }
        (Some(identity), None) => display_safe(&identity.identifier),
        (None, Some(label)) => format!("unknown ({label})"),
        (None, None) => "unknown".into(),
    }
}
/// The meeting read: the notice as preamble, then the meeting as one block — its scope and
/// captured time, how the latest extraction and coverage stand, and whether the transcript is
/// kept — then each hypothesis as a block with the quote it rests on. A look that found nothing
/// ends with the one empty voice.
fn meeting_text(m: &rastro_core::meeting::MeetingCommitments) -> String {
    let e = &m.extraction;
    let block = render::Block::new(format!("Meeting: {}", display_safe(&m.meeting.id)))
        .field("Scope", display_safe(&m.meeting.scope))
        .field("Captured", render::time(m.meeting.captured_at))
        .field("Extraction", e.state.as_str())
        .field("Run", e.run)
        .field("Finished", render::time(e.finished_at))
        .maybe("Extractor", e.extractor.as_deref().map(display_safe))
        .maybe(
            "Replay",
            e.replay_of.map(|run| format!("run {run}, no model call")),
        )
        .field("Excluded", e.excluded)
        .maybe("Cause", e.cause.map(|c| c.as_str()))
        .maybe("Reason", e.reason.as_deref().map(display_safe))
        .field("Coverage", m.coverage.state.as_str())
        .field(
            "Turns read",
            format!("{} of {}", m.coverage.turns_read, m.coverage.turns_total),
        )
        .maybe(
            "Stale",
            m.results_from.as_ref().map(|from| {
                format!(
                    "the hypotheses below come from the last good run {} ({}), not the latest",
                    from.run,
                    render::time(from.finished_at)
                )
            }),
        )
        .field(
            "Transcript",
            if m.transcript_retained {
                "retained (opt-in)"
            } else {
                "not retained"
            },
        );
    let mut text = format!("{}\n\n{}", m.notice, block);
    if m.hypotheses.is_empty() {
        text += &format!("\n\n{}", render::empty("hypotheses"));
    } else {
        text += &format!(
            "\n\n{}",
            render::records(m.hypotheses.iter().map(meeting_hypothesis_block))
        );
    }
    text
}
/// One meeting hypothesis as a block: its parties, promise and date, then the quote it rests on
/// with its turn, speaker and span.
fn meeting_hypothesis_block(h: &rastro_core::meeting::Hypothesis) -> render::Block {
    let date = match &h.stated_date.text {
        Some(text) => display_safe(text),
        None => "unknown".into(),
    };
    render::Block::new("Hypothesis")
        .field("From", party_text(&h.responsible))
        .field("To", party_text(&h.recipient))
        .field("Promise", display_safe(&h.promise))
        .field("Date", date)
        .field("Quote", format!("\"{}\"", display_escaped(&h.quote.text)))
        .field("Turn", h.quote.turn)
        .field("Speaker", display_safe(&h.quote.speaker))
        .field("Span", format!("[{}, {})", h.quote.start, h.quote.end))
}
fn commitment_party_text(party: &rastro_core::commitments::Party) -> String {
    let label = party.label.as_deref().map(display_safe);
    match (&party.identity, label) {
        (Some(identity), Some(label)) => {
            format!("{} ({label})", display_safe(&identity.identifier))
        }
        (Some(identity), None) => display_safe(&identity.identifier),
        (None, Some(label)) => format!("unknown ({label})"),
        (None, None) => "unknown".into(),
    }
}
/// The commitments list: the notice as preamble, then every source as a block — how its
/// extraction and coverage stand, and what it found or why it could not look — then its
/// hypotheses as blocks.
fn commitments_text(list: &rastro_core::commitments::Commitments) -> String {
    use rastro_core::meeting::RunState;
    if list.sources.is_empty() {
        return format!("{}\n\n{}", list.notice, render::empty("sources"));
    }
    let mut blocks = Vec::new();
    for source in &list.sources {
        let e = &source.extraction;
        let c = &source.coverage;
        let kind = serde_json::to_value(source.kind)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();
        let mut block = render::Block::new(display_safe(&source.name))
            .field("Kind", kind)
            .field("Scope", display_safe(&source.scope))
            .field("Extraction", e.state.as_str())
            .field("Run", &e.run)
            .field("Finished", render::time(e.finished_at))
            .maybe("Adapter", e.adapter.as_deref().map(display_safe))
            .maybe(
                "Replay",
                e.replay_of
                    .as_ref()
                    .map(|run| format!("run {run}, no model call")),
            )
            .field("Excluded", e.excluded)
            .maybe("Cause", e.cause.map(|c| c.as_str()))
            .maybe("Reason", e.reason.as_deref().map(display_safe))
            .field("Coverage", c.state.as_str())
            .field(
                "Turns read",
                format!("{} of {}", c.turns_read, c.turns_total),
            )
            .maybe("Coverage reason", c.reason.as_deref().map(display_safe));
        if source.stale
            && let Some(from) = &source.results_from
        {
            block = block.field(
                "Stale",
                format!(
                    "the hypotheses below come from run {} (imported {}), not the latest",
                    from.run,
                    render::time(from.imported_at)
                ),
            );
        }
        if source.hypotheses.is_empty() {
            block = block.field(
                "Hypotheses",
                match e.state {
                    RunState::Succeeded => "found nothing",
                    RunState::Failed => "couldn't look",
                },
            );
        }
        blocks.push(block);
        blocks.extend(source.hypotheses.iter().map(commitment_hypothesis_block));
    }
    format!("{}\n\n{}", list.notice, render::records(blocks))
}
/// One commitments hypothesis as a block: its kind, parties, promise, date and terms, then the
/// quote it rests on with its turn, speaker and span.
fn commitment_hypothesis_block(h: &rastro_core::commitments::Hypothesis) -> render::Block {
    let date = match &h.stated_date.text {
        Some(text) => display_safe(text),
        None => "unknown".into(),
    };
    render::Block::new("Hypothesis")
        .field("Kind", h.kind.as_str())
        .field("From", commitment_party_text(&h.responsible))
        .field("To", commitment_party_text(&h.recipient))
        .field("Promise", display_safe(&h.promise))
        .field("Date", date)
        .field("Terms", h.terms.join(", "))
        .field("Quote", format!("\"{}\"", display_escaped(&h.quote.text)))
        .field("Turn", h.quote.turn)
        .field("Speaker", display_safe(&h.quote.speaker))
        .field("Span", format!("[{}, {})", h.quote.start, h.quote.end))
}
/// The focus report for a terminal: the recommended focus and the needs-you items as blocks with
/// their citations, the queue count, each gap as a block, and the as-of time.
fn focus_text(report: &rastro_core::focus::Report) -> String {
    use rastro_core::focus::GapReason;
    let mut blocks = Vec::new();
    if let Some(item) = &report.focus {
        blocks.push(focus_item_block("Focus", item));
    }
    for item in &report.needs_you {
        blocks.push(focus_item_block("Needs you", item));
    }
    let mut text = if blocks.is_empty() {
        display_safe(
            report
                .notice
                .as_deref()
                .unwrap_or(rastro_core::focus::NOTHING_NEEDS_YOU),
        )
    } else {
        render::records(blocks)
    };
    text += &format!("\n\nQueue: {} waiting", report.queue_count);
    for gap in &report.gaps {
        let why = match gap.reason {
            GapReason::SyncFailed => "its last sync failed",
            GapReason::ExtractionFailed { .. } => "its extraction failed",
        };
        let as_of = gap
            .as_of
            .map_or_else(|| "never read".to_string(), render::time);
        let block = render::Block::new(format!("Gap: {}", display_safe(&gap.name)))
            .field("Reason", why)
            .field("As of", as_of);
        text += &format!("\n\n{}", block);
    }
    text += &format!("\n\nAs of: {}", render::time(report.as_of));
    text
}

/// One focus item as a block: its rank and title as the header, then its citation fields.
fn focus_item_block(rank: &str, item: &rastro_core::focus::Item) -> render::Block {
    use rastro_core::focus::SourceKind;
    let kind = match item.citation.source {
        SourceKind::Linear => "linear",
        SourceKind::Meeting => "meeting",
    };
    render::Block::new(format!("{rank}: {}", display_safe(&item.title)))
        .field(
            "Source",
            format!("{kind} {}", display_safe(&item.citation.name)),
        )
        .field("Location", display_safe(&item.citation.location))
        .field("Observed", render::time(item.citation.observed_at))
}

/// The name a queue source's kind goes by in the owner's reads.
fn queue_source_kind(kind: rastro_core::queue::SourceKind) -> &'static str {
    match kind {
        rastro_core::queue::SourceKind::Meeting => "meeting",
        rastro_core::queue::SourceKind::Identity => "identity",
        rastro_core::queue::SourceKind::Linear => "linear",
    }
}

/// What `actions propose` recorded: the proposal as a block — its id on the header line, its
/// source and fields — then the run a matching rule started, or the queue it waits in. A
/// proposal that already has a run records nothing, and its answer says so.
fn actions_propose_text(proposed: &rastro_core::actions::Proposed) -> String {
    let filed = &proposed.filed;
    let block = render::Block::new(format!("Proposed {}", filed.id))
        .field(
            "Source",
            format!(
                "{} {}",
                queue_source_kind(filed.source.kind),
                display_safe(&filed.source.name)
            ),
        )
        .field("Scope", display_safe(&filed.source.scope));
    match &proposed.run {
        Some(run) if run.status == rastro_core::actions::RunStatus::Pending => {
            let value = match run.due_at {
                Some(due) => format!(
                    "{} runs at {} unless stopped: {}",
                    run.id,
                    render::time(due),
                    rastro_core::actions::stop_command(&run.id)
                ),
                None => run.id.clone(),
            };
            let block = block.field("Run", value);
            action_fields(block, &filed.proposal).to_string()
        }
        Some(run) => {
            let block = action_fields(block, &filed.proposal);
            format!(
                "{}\n\n{}",
                render::records([block, action_run_block(run)]),
                render::nothing_recorded("Already has a run")
            )
        }
        None => {
            let waiting = match &proposed.cap {
                Some(wait) => format!(
                    "waiting in the queue: the rule {} reached its daily cap of {}",
                    display_safe(&wait.rule),
                    wait.cap
                ),
                None => "waiting in the queue; nothing has run".into(),
            };
            let block = block.field("Status", waiting);
            action_fields(block, &filed.proposal).to_string()
        }
    }
}

/// The queue: its explanatory preamble, then each proposal or question as a block — its id on its
/// own line so the owner can copy it straight into an answer — then each source's gap as a block.
fn queue_text(queue: &rastro_core::queue::Queue) -> String {
    let mut text = queue.notice.clone();
    if queue.items.is_empty() && queue.gaps.is_empty() {
        text += &format!("\n{}", render::empty("proposals or questions waiting"));
        return text;
    }
    let blocks = queue
        .items
        .iter()
        .map(queue_item_block)
        .chain(queue.gaps.iter().map(queue_gap_block));
    text += &format!("\n\n{}", render::records(blocks));
    text
}

/// One queue item as a block: its kind as the header, its content-addressed id on its own line,
/// then its source and the fields its kind carries.
#[expect(
    clippy::too_many_lines,
    reason = "each kind's fields are written in one pass, so what a proposal, question or Act shows reads at a glance"
)]
fn queue_item_block(item: &rastro_core::queue::Item) -> render::Block {
    use rastro_core::queue::ItemKind;
    let header = match item.kind {
        ItemKind::ActionProposal => {
            if item.act.is_some() {
                "Pending Act"
            } else {
                "Action proposal"
            }
        }
        ItemKind::Proposal => "Proposal",
        ItemKind::Question => "Question",
    };
    let source = format!(
        "{} {}",
        queue_source_kind(item.source.kind),
        display_safe(&item.source.name)
    );
    let mut block = render::Block::new(header)
        .field("Id", &item.id)
        .field("Source", source);
    match item.kind {
        ItemKind::ActionProposal => {
            let action = item.action.as_ref().expect("an action item carries one");
            block = block.field("Scope", display_safe(&item.source.scope));
            if let Some(run) = &item.act {
                let when = match item.runs_in_seconds {
                    Some(seconds) if seconds > 0 => {
                        format!("runs in {}", grace_text(seconds.unsigned_abs()))
                    }
                    _ => "due now".into(),
                };
                block = block.field("When", when).field(
                    "Stop",
                    run.stop_command
                        .clone()
                        .unwrap_or_else(|| rastro_core::actions::stop_command(&run.id)),
                );
            }
            block = action_fields(block, action);
            if !item.possible_duplicates.is_empty() {
                block = block.field(
                    "Warning",
                    format!(
                        "possible duplicate of {}",
                        item.possible_duplicates
                            .iter()
                            .map(|id| display_safe(id))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
            }
        }
        ItemKind::Proposal => {
            let h = item.proposal.as_ref().expect("a proposal item carries one");
            let date = match &h.stated_date.text {
                Some(text) => display_safe(text),
                None => "unknown".into(),
            };
            block = block
                .field("Scope", display_safe(&item.source.scope))
                .field("Kind", h.kind.as_str())
                .field("From", commitment_party_text(&h.responsible))
                .field("To", commitment_party_text(&h.recipient))
                .field("Promise", display_safe(&h.promise))
                .field("Date", date)
                .field("Terms", h.terms.join(", "))
                .field("Quote", format!("\"{}\"", display_escaped(&h.quote.text)))
                .field("Turn", h.quote.turn)
                .field("Speaker", display_safe(&h.quote.speaker))
                .field("Span", format!("[{}, {})", h.quote.start, h.quote.end));
        }
        ItemKind::Question if item.correction.is_some() => {
            let c = item.correction.as_ref().expect("checked by the arm");
            let claim = &c.claim.claim;
            let (kind, mark) = match c.kind {
                rastro_core::queue::CorrectionKind::LapsedRetraction => {
                    ("retraction lapsed", "earlier retracted")
                }
                rastro_core::queue::CorrectionKind::StaleConfirmation => {
                    ("confirmation stale", "stale")
                }
            };
            block = block
                .field("Kind", kind)
                .field("Claim", &claim.id)
                .field("Status", format!("{} ({mark})", claim.status))
                .field("Intent", &c.intent)
                .field("Answer", "yes to hold it again");
        }
        ItemKind::Question if item.action.is_some() => {
            let action = item.action.as_ref().expect("an Ask question carries one");
            block = block
                .field("Scope", display_safe(&item.source.scope))
                .field("Ask", "yes to run it once, no to reject it")
                .field("Action", display_safe(&action.action))
                .field("Title", display_safe(&action.fields.title))
                .field("Team", display_safe(&action.fields.team));
        }
        ItemKind::Question => {
            let q = item.question.as_ref().expect("a question item carries one");
            let account = |a: &rastro_core::queue::Account| {
                let mut text = display_safe(&a.identifier);
                if !a.names.is_empty() {
                    text += &format!(
                        " ({})",
                        a.names
                            .iter()
                            .map(|n| display_safe(n))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                text
            };
            let reason = |r: &rastro_core::identity::CandidateReason| {
                let name = serde_json::to_value(r.reason)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                format!(
                    "{name}: {} = {}",
                    display_safe(&r.values[0]),
                    display_safe(&r.values[1])
                )
            };
            block = block.field(
                "Accounts",
                format!("{} ~ {}", account(&q.accounts[0]), account(&q.accounts[1])),
            );
            for r in &q.reasons {
                block = block.field("Reason", reason(r));
            }
            block = block.field("Name", display_safe(&q.name));
        }
    }
    block
}

/// An action proposal's fields on a block: what it would do, the assignee and why, and what it
/// cites.
fn action_fields(
    mut block: render::Block,
    action: &rastro_core::actions::Proposal,
) -> render::Block {
    let fields = &action.fields;
    block = block
        .field("Action", display_safe(&action.action))
        .field("Title", display_safe(&fields.title))
        .field("Team", display_safe(&fields.team))
        .field("Assignee", display_safe(&fields.assignee))
        .field("Why", display_safe(&action.why));
    if let Some(description) = &fields.description {
        block = block.field("Description", display_safe(description));
    }
    if !action.cites.is_empty() {
        block = block.field(
            "Cites",
            action
                .cites
                .iter()
                .map(|c| display_safe(c))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    block
}

/// One source's gap as a block: its name, then what the run found and why.
fn queue_gap_block(gap: &rastro_core::queue::Gap) -> render::Block {
    let state = match gap.state {
        rastro_core::queue::GapState::FoundNothing => "Found nothing",
        rastro_core::queue::GapState::Failed => "Failed",
        rastro_core::queue::GapState::CouldNotLook => "Couldn't look",
    };
    render::Block::new(format!("Gap: {}", display_safe(&gap.source.name)))
        .field("Kind", queue_source_kind(gap.source.kind))
        .field("Scope", display_safe(&gap.source.scope))
        .field("Run", &gap.run)
        .field("State", state)
        .maybe("Cause", gap.cause.map(|c| c.as_str()))
        .maybe("Reason", gap.reason.as_deref().map(display_safe))
}

/// The outcomes: what happened to each item as a block — its id first, then what and when — then
/// the totals as their own block.
fn outcomes_text(outcomes: &rastro_core::queue::Outcomes) -> String {
    let body = if outcomes.outcomes.is_empty() {
        render::empty("outcomes yet")
    } else {
        render::records(outcomes.outcomes.iter().map(outcome_block))
    };
    let totals = render::Block::new("Totals")
        .field("Shown", outcomes.totals.shown)
        .field("Acted on", outcomes.totals.acted_on);
    format!("{}\n\n{}\n\n{}", outcomes.notice, body, totals)
}

/// One outcome as a block: the item it happened to, then what happened and when.
fn outcome_block(outcome: &rastro_core::queue::Outcome) -> render::Block {
    let kind = match outcome.kind {
        rastro_core::queue::OutcomeKind::Accepted => "accepted",
        rastro_core::queue::OutcomeKind::AcceptedWithEdits => "accepted with edits",
        rastro_core::queue::OutcomeKind::Rejected => "rejected",
        rastro_core::queue::OutcomeKind::Answered => "answered",
        rastro_core::queue::OutcomeKind::BecameMoot => "became moot",
        rastro_core::queue::OutcomeKind::Ignored => "ignored",
    };
    render::Block::new(outcome.item.as_str())
        .field("Kind", kind)
        .field("At", render::time(outcome.at))
        .maybe("Claim", outcome.claim.as_deref())
        .maybe("Waited", outcome.waited_seconds.map(|s| format!("{s}s")))
}

/// What a queue answer recorded: the item it named, then each owner intent, then the run it
/// started as a block.
fn queue_answer_text(result: &rastro_operations::results::QueueAnswerResult) -> String {
    let mut lines = Vec::new();
    if let Some(item) = &result.item {
        lines.push(format!("Item: {item}"));
    }
    lines.push(intents_text(
        &result.intents,
        &render::empty("intents recorded"),
    ));
    let mut text = lines.join("\n");
    if let Some(run) = &result.run {
        text += &format!("\n\n{}", action_run_block(run));
    }
    text
}

/// One action run as a block: its id on the header line, then its status, level, what it wrote
/// and where, the issue it created, and what ended or undid it.
fn action_run_block(run: &rastro_core::actions::Run) -> render::Block {
    let mut block = render::Block::new(format!("Run {}", run.id))
        .field("Status", run.status.as_str())
        .field("Level", run.level.as_str())
        .field("Action", display_safe(&run.action))
        .field(
            "Source",
            format!(
                "{} {}",
                queue_source_kind(run.source.kind),
                display_safe(&run.source.name)
            ),
        )
        .field("Scope", display_safe(&run.source.scope))
        .field("Title", display_safe(&run.fields.title))
        .field("Team", display_safe(&run.fields.team))
        .field("Assignee", display_safe(&run.fields.assignee));
    if let (Some(identifier), Some(url)) = (&run.identifier, &run.url) {
        block = block.field(
            "Issue",
            format!("{} {}", display_safe(identifier), display_safe(url)),
        );
    }
    if let Some(due) = run.due_at {
        let label = if run.status == rastro_core::actions::RunStatus::Pending {
            "Runs at"
        } else {
            "Grace ended"
        };
        block = block.field(label, render::time(due));
    }
    if let Some(stop) = &run.stop_command {
        block = block.field("Stop", stop);
    }
    if let Some(error) = &run.error {
        block = block
            .field("Error", display_safe(error))
            .field("Attempts", run.attempts);
    }
    if let Some(undo) = &run.undo {
        block = block.field(
            "Undone",
            format!(
                "intent {}, {}",
                display_safe(&undo.intent),
                render::time(undo.at)
            ),
        );
    }
    block
}

/// What `actions` lists: each run as a block, or that none has run; a shown run carries its why
/// as a second block.
fn actions_text(runs: &rastro_core::actions::Runs) -> String {
    if runs.runs.is_empty() {
        return render::empty("action has run");
    }
    let mut blocks: Vec<_> = runs.runs.iter().map(action_run_block).collect();
    if let Some(why) = &runs.why {
        blocks.push(action_why_block(
            why,
            runs.runs.first().and_then(|run| run.rule.as_ref()),
        ));
    }
    render::records(blocks)
}

/// Why one run happened, as its own block: the why on the header line, then each cited claim,
/// the rule or the accept that authorized it, where the proposal stands in the queue, and the
/// cell's level and risk.
fn action_why_block(
    why: &rastro_core::actions::RunWhy,
    rule: Option<&rastro_core::rules::RuleCopy>,
) -> render::Block {
    let mut block = render::Block::new(format!("Why: {}", display_safe(&why.proposal.why)));
    for claim in &why.cited {
        block = block.field(
            "Cites",
            format!(
                "{} ({} support{})",
                display_safe(&claim.claim.claim.id),
                claim.claim.support_count,
                if claim.claim.support_count == 1 {
                    ""
                } else {
                    "s"
                }
            ),
        );
    }
    if let Some(rule) = rule {
        let mut value = format!(
            "{}, {}, team {}, assignee {}, grace {}",
            display_safe(&rule.name),
            display_safe(&rule.action),
            display_safe(&rule.team),
            display_safe(&rule.assignee),
            grace_text(rule.grace_seconds)
        );
        if let Some(cap) = rule.daily_cap {
            value += &format!(", cap {cap} a day");
        }
        block = block.field("Rule", value);
    }
    if let Some(accept) = &why.accept {
        block = block.field(
            "Accepted",
            format!("intent {}, {}", accept.intent, render::time(accept.at)),
        );
    }
    block
        .field("Queue", standing_text(&why.queue))
        .field("Level", why.level)
        .field(
            "Cell",
            format!(
                "{} x {}",
                display_safe(&why.cell.action),
                display_safe(&why.cell.team)
            ),
        )
        .field("Reach", why.reach)
        .field("Reversibility", why.reversibility)
}

/// Where the run's proposal stands in the queue, in the owner's words: one line, by the same
/// decision the queue read uses.
fn standing_text(standing: &rastro_core::queue::Standing) -> String {
    use rastro_core::queue::Standing;
    match standing {
        Standing::Waiting => "waiting for the owner".into(),
        Standing::PendingAct { run } => format!("waiting as pending Act {run}"),
        Standing::Answered { kind, intent } => {
            format!("out, the owner's {} (intent {intent})", kind.as_str())
        }
        Standing::Settled { run, status } => format!("out, run {run} is {}", status.as_str()),
    }
}

/// The result of an operation, its human form, and whether it succeeded.
type Outcome = (Response, String);

/// What a claim correction says: the correction as a block — what it did on the header line,
/// then the intent it recorded and its issued time — or that the claim already was so.
fn correction_text(r: &rastro_operations::results::ClaimWriteResult, did: &str) -> String {
    match r.intents.first() {
        Some(intent) => render::Block::new(format!("{did} {}", r.claim))
            .field("Intent", &intent.id)
            .field("Issued", render::time(intent.issued_at))
            .to_string(),
        None => render::nothing_recorded(&format!("{} is already {}", r.claim, did.to_lowercase())),
    }
}
/// What an undo of a claim correction says: the intent it undid as a block, or that there was
/// nothing to undo.
fn undo_text(r: &rastro_operations::results::ClaimWriteResult) -> String {
    match r.intents.first() {
        Some(intent) => render::Block::new(format!(
            "Undid intent {} on {}",
            intent.undo_of.as_deref().unwrap_or_default(),
            r.claim
        ))
        .field("Intent", &intent.id)
        .field("Issued", render::time(intent.issued_at))
        .to_string(),
        None => render::nothing_recorded(&format!("{} has nothing to undo", r.claim)),
    }
}
/// What a fix of a claim says: the claim it retracted and the one it asserted, as a block with
/// the intent it recorded, or that the claim already says that.
fn edit_text(r: &rastro_operations::results::ClaimWriteResult) -> String {
    match (&r.asserted, r.intents.first()) {
        (Some(asserted), Some(intent)) => {
            render::Block::new(format!("Retracted {}, asserted {asserted}", r.claim))
                .field("Intent", &intent.id)
                .field("Issued", render::time(intent.issued_at))
                .to_string()
        }
        _ => render::nothing_recorded(&format!("{} already says that", r.claim)),
    }
}
/// A grace window in the owner's words: whole minutes, else seconds.
fn grace_text(seconds: u64) -> String {
    if seconds >= 60 && seconds.is_multiple_of(60) {
        format!("{} min", seconds / 60)
    } else {
        format!("{seconds} s")
    }
}
/// One block per rule, in the order the owner wrote them: its action, team, assignee, grace and
/// today's count against its cap, with the queued proposals it matches as fields; each pending
/// Act it started follows as its own block.
fn rules_check_text(r: &rastro_operations::results::RulesCheckResult) -> String {
    if r.rules.is_empty() {
        return render::empty("rules in config.toml");
    }
    let mut blocks = Vec::new();
    for rule in &r.rules {
        let used = match rule.daily_cap {
            Some(cap) => format!("{} of {cap}", rule.used_today),
            None => format!("{}", rule.used_today),
        };
        let mut block = render::Block::new(display_safe(&rule.name))
            .field("Action", display_safe(&rule.action))
            .field("Team", display_safe(&rule.team))
            .field("Assignee", display_safe(&rule.assignee))
            .field("Grace", grace_text(rule.grace_seconds))
            .field("Used today", used);
        for filed in &rule.matches {
            block = block.field(
                "Matches",
                format!(
                    "{} ({})",
                    filed.id,
                    display_safe(&filed.proposal.fields.title)
                ),
            );
        }
        blocks.push(block);
        blocks.extend(rule.pending.iter().map(action_run_block));
    }
    render::records(blocks)
}
/// What `trust` lists: each cell as a block — its level, the counts behind it, its last change
/// and what the next promotion needs, with the proposed rule entry verbatim when one is offered
/// — then each plugin's engagement, time to action and cost; an empty store says it holds no
/// trust history.
fn trust_text(trust: &rastro_core::trust::Trust) -> String {
    if trust.cells.is_empty() && trust.plugins.is_empty() {
        return render::empty("trust history: no action has run");
    }
    let mut records = Vec::new();
    for cell in &trust.cells {
        let mut text = trust_cell_block(cell).to_string();
        if let Some(rule) = &cell.promotion.rule_text {
            // The exact entry to paste into config.toml, verbatim under its cell.
            text += &format!("\n{rule}");
        }
        records.push(text);
    }
    for plugin in &trust.plugins {
        records.push(trust_plugin_block(plugin).to_string());
    }
    records.join("\n\n")
}

/// One cell's trust as a block: its action on the header line, then its team, level, the counts
/// behind the level, its last change and what the next promotion needs.
fn trust_cell_block(cell: &rastro_core::trust::TrustCell) -> render::Block {
    let mut block = render::Block::new(display_safe(&cell.action))
        .field("Team", display_safe(&cell.team))
        .field("Level", cell.level.as_str())
        .field("Clean", cell.clean)
        .field("Undos", cell.undos);
    if let Some(change) = &cell.last_change {
        block = block.field(
            "Last change",
            format!(
                "{} at {}",
                display_safe(&change.cause),
                render::time(change.at)
            ),
        );
    }
    block = block.field("Next promotion", display_safe(&cell.promotion.needs));
    if let Some(reset) = &cell.promotion.reset_command {
        block = block.field("Reset", reset);
    }
    block
}

/// One plugin's usefulness and cost as a block: its name on the header line, then its
/// engagement, time to action, cost and cost per accepted item.
fn trust_plugin_block(plugin: &rastro_core::trust::TrustPlugin) -> render::Block {
    use rastro_core::trust::{CostPerAccepted, Engagement};
    let engagement = match plugin.engagement {
        Engagement::Measured { acted, seen } => format!("{acted} of {seen}"),
        Engagement::Unmeasured(_) => "unmeasured".into(),
    };
    let mut block =
        render::Block::new(display_safe(&plugin.plugin)).field("Engagement", engagement);
    if let Some(seconds) = plugin.time_to_action_seconds {
        block = block.field("Time to action", format!("{seconds}s"));
    }
    block = block.field(
        "Cost",
        format!(
            "{}, {} turns",
            wall_text(plugin.cost.wall_ms),
            plugin.cost.turns
        ),
    );
    let per_accepted = match plugin.cost_per_accepted {
        CostPerAccepted::Measured(cost) => {
            format!("{}, {} turns", wall_text(cost.wall_ms), cost.turns)
        }
        CostPerAccepted::Dash(_) => "—".into(),
    };
    block.field("Per accepted", per_accepted)
}

/// A wall time the owner reads: whole seconds when they are whole, milliseconds otherwise.
fn wall_text(ms: i64) -> String {
    if ms % 1000 == 0 {
        format!("{}s", ms / 1000)
    } else {
        format!("{ms}ms")
    }
}

/// One cell's trust as the terminal reads it: the row `trust` prints, on its own.
fn trust_cell_text(cell: &rastro_core::trust::TrustCell) -> String {
    let mut trust = rastro_core::trust::Trust::empty();
    trust.cells.push(cell.clone());
    trust_text(&trust)
}
/// What `trust journal` lists: every level change as a block, newest first — its time on the
/// header line, then its cell, the levels it moved between and its cause; an empty store says
/// no level has changed.
fn trust_journal_text(journal: &rastro_core::trust::TrustJournal) -> String {
    if journal.changes.is_empty() {
        return render::empty("trust history: no level has changed");
    }
    render::records(journal.changes.iter().map(|change| {
        render::Block::new(render::time(change.at))
            .field(
                "Cell",
                format!(
                    "{} x {}",
                    display_safe(&change.cell.action),
                    display_safe(&change.cell.team)
                ),
            )
            .field(
                "Change",
                format!("{} -> {}", change.from.as_str(), change.to.as_str()),
            )
            .field("Cause", display_safe(&change.cause))
    }))
}
/// A rebuild's summary: the claim count, then one line per carried-over pair — a pair that is not
/// current never started, its stored output was carried over.
fn claims_rebuild_text(r: &rastro_operations::results::ClaimsRebuildResult) -> String {
    let mut text = format!("Claims rebuilt: {} claims", r.claims);
    for pair in r
        .pairs
        .iter()
        .filter(|p| p.state != ExtractionState::Current)
    {
        let extractor = format!(
            "{} {}",
            display_safe(&pair.extractor),
            pair.extractor_version
        );
        text += &format!(
            "\n  Carried over: {} ({}), Extractor: {}, {}",
            display_safe(&pair.source),
            display_safe(&pair.scope),
            extractor.trim(),
            pair.state.as_str(),
        );
        if let Some(reason) = pair.reason.as_deref().filter(|r| *r != pair.state.as_str()) {
            text += &format!(", Reason: {}", display_safe(reason));
        }
    }
    text
}
fn human(response: &Response) -> String {
    let or_none = |blocks: Vec<render::Block>| {
        if blocks.is_empty() {
            render::empty("sources registered")
        } else {
            render::records(blocks)
        }
    };
    match response {
        Response::SourceAdd(r) => format!("Source registered: {}", r.source.name),
        Response::SourceCheck(r) => check::line(r),
        Response::SourceList(r) => or_none(r.sources.iter().map(source_block).collect()),
        Response::Coverage(r) => or_none(r.coverage.iter().map(coverage_block).collect()),
        Response::Sync(r) => or_none(r.runs.iter().map(sync_block).collect()),
        Response::ClaimsRebuild(r) => claims_rebuild_text(r),
        Response::IndexRebuild(r) => format!("Index rebuilt: {} observations", r.indexed),
        Response::Search(r) => search_text(&r.query, r),
        Response::Why(r) => why_text(r),
        Response::About(r) => about_text(r),
        Response::Commitments(r) => commitments_text(r),
        Response::Queue(r) => queue_text(r),
        Response::Actions(r) => actions_text(r),
        Response::ActionsShow(r) => actions_text(r),
        Response::ActionsUndo(r) => format!("Undone: {}", action_run_block(r)),
        Response::ActionsStop(r) => format!("Stopped: {}", action_run_block(r)),
        Response::ActionsPropose(r) => actions_propose_text(r),
        Response::RulesCheck(r) => rules_check_text(r),
        Response::Focus(r) => focus_text(r),
        Response::QueueOutcomes(r) => outcomes_text(r),
        Response::Trust(r) => trust_text(r),
        Response::TrustJournal(r) => trust_journal_text(r),
        Response::TrustPin(r) => format!("Pinned:\n{}", trust_cell_text(r)),
        Response::TrustReset(r) => format!("Reset:\n{}", trust_cell_text(r)),
        Response::QueueAccept(r)
        | Response::QueueEdit(r)
        | Response::QueueReject(r)
        | Response::QueueAnswer(r)
        | Response::QueueMissed(r) => queue_answer_text(r),
        Response::MeetingImport(r)
        | Response::MeetingCommitments(r)
        | Response::MeetingReplay(r) => meeting_text(r),
        Response::MeetingForget(r) => {
            let removed = &r.removed;
            format!(
                "Forgot meeting {}: {} runs, {} hypotheses, {} spans, {} recorded outputs{}",
                display_safe(&r.meeting),
                removed.runs,
                removed.hypotheses,
                removed.spans,
                removed.recorded_output,
                if removed.transcript {
                    ", the retained transcript"
                } else {
                    ""
                }
            )
        }
        Response::MeetingExport(r) => {
            serde_json::to_string_pretty(r).expect("an export serialises")
        }
        Response::IdentityMe(r) | Response::IdentityPersonCreate(r) => person_text(r),
        Response::IdentityLink(r)
        | Response::IdentityMerge(r)
        | Response::IdentityUndo(r)
        | Response::IdentityAnswer(r) => {
            intents_text(&r.intents, &render::nothing_recorded("Already in force"))
        }
        Response::ClaimConfirm(r) => correction_text(r, "Confirmed"),
        Response::ClaimReject(r) => correction_text(r, "Retracted"),
        Response::ClaimUndo(r) => undo_text(r),
        Response::ClaimEdit(r) => edit_text(r),
        Response::IdentityCandidates(r) => candidates_text(r),
        Response::IdentityHistory(r) => match &r.entity {
            Some(entity) => format!(
                "{}\n\n{}",
                display_safe(&entity.identifier),
                intents_text(&r.intents, &render::empty("intents name it"))
            ),
            None => render::empty("entity matches"),
        },
    }
}
fn execute(request: Request, json: bool) -> Result<Outcome> {
    let locations = Locations::resolve()?;
    // The terminal queue read records what it printed (learning.md §3); a machine read — `--json`
    // or an MCP call — records nothing, so automation can never make an item look ignored.
    let printed = !json && matches!(request, Request::Queue { command: None, .. });
    if !printed
        && catalog::find(request.name()).is_some_and(|spec| spec.read_only && spec.read_path)
    {
        let mut reader = Rastro::open_read_only(locations, SqliteStorage::open_read_only)?;
        let response = request.run_read(&mut reader)?;
        let text = human(&response);
        return Ok((response, text));
    }
    let storage = SqliteStorage::open(&locations.database())?;
    let mut rastro = Rastro::open(locations, storage)?;
    // Progress goes to stderr whatever the output format, never to stdout.
    let response =
        request.run_observed(&mut rastro, &mut rastro_core::runs::StderrProgress::new())?;
    if printed && let Response::Queue(queue) = &response {
        rastro.queue_shown(queue)?;
    }
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
                Command::Tick => "tick",
                Command::Check { .. } => "source.check",
                Command::Plugin { .. } => "plugin.check",
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
/// A line on stderr. A stderr that is closed or broken loses it; `eprintln!` would panic on a
/// broken one, and the run's exit code would no longer say how it went.
fn stderr_line(text: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "{text}");
}
/// `rastro plugin check <id>`: one synthetic connector run through the host's validation,
/// storing nothing. The exit code says whether the connector conformed. The human output is
/// one block through the shared render pattern; `--json` keeps the envelope.
fn plugin_check(id: &str, options: &[String], json: bool) -> i32 {
    let fail = |message: String| {
        if json {
            println!("{}", envelope::failure("plugin.check", message));
        } else {
            stderr_line(&format!("error: {message}"));
        }
        1
    };
    let mut selector = std::collections::BTreeMap::new();
    for option in options {
        let Some((key, value)) = option.split_once('=') else {
            return fail(format!("option {option} is not key=value"));
        };
        selector.insert(key.to_string(), value.to_string());
    }
    let locations = match Locations::resolve() {
        Ok(locations) => locations,
        Err(error) => return fail(format!("{error:#}")),
    };
    let report = rastro_core::host::plugin_check::run(&locations, id, selector);
    if json {
        println!("{}", envelope::success("plugin.check", &report));
    } else {
        let mut block = render::Block::new(display_safe(&report.plugin_id))
            .field("Counts", tally(&report.counts))
            .field("Coverage", report.coverage_label());
        for warning in &report.warnings {
            block = block.field("Warning", warning);
        }
        for violation in &report.violations {
            block = block.field("Violation", violation);
        }
        println!("{block}");
    }
    i32::from(!report.violations.is_empty())
}
fn main() {
    let (cli, operation) = parse();
    let request = match cli.command {
        // The credential probe never opens the store: opening it would create or touch stored
        // state, and a check must leave everything as it was.
        Command::Operation(Request::Source {
            command: SourceRequest::Check(check),
        }) => std::process::exit(check::run(&check.source, cli.json)),
        Command::Operation(request) => request,
        Command::Tick => std::process::exit(tick::run()),
        Command::Check { check } => std::process::exit(check::run(&check.source, cli.json)),
        Command::Plugin {
            command: PluginCommand::Check { id, options },
        } => std::process::exit(plugin_check(&id, &options, cli.json)),
        Command::Mcp { scopes } => match mcp::run(scopes) {
            Ok(()) => return,
            Err(error) => {
                stderr_line(&format!("error: {}", mcp::bounded(&format!("{error:#}"))));
                std::process::exit(1);
            }
        },
    };
    match execute(request, cli.json) {
        Ok((response, text)) => {
            if cli.json {
                println!("{}", response.envelope());
            } else {
                println!("{text}");
            }
            if let Response::SourceAdd(added) = &response {
                // Setup keeps the source whatever the check says; the owner sees it either way,
                // on stderr when stdout is the machine-readable envelope.
                let report = check::setup_report(&added.source);
                if cli.json {
                    stderr_line(&report);
                } else {
                    println!("{report}");
                }
            }
            if !response.succeeded() {
                std::process::exit(1);
            }
        }
        Err(error) => {
            if cli.json {
                println!("{}", envelope::failure(operation, format!("{error:#}")));
            } else {
                stderr_line(&format!("error: {error:#}"));
            }
            std::process::exit(1);
        }
    }
}
