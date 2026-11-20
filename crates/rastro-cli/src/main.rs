use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
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
    for gap in &w.gaps {
        line += &format!(
            " | Gap: {} ({})",
            display_safe(&gap.selection),
            display_safe(&gap.reason)
        );
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
    format!(
        "{} {} {} | Status: {} | Claim: {} | Revision: {} | Supports: {}",
        entity(
            &c.subject.identifier,
            &summary.subject_aliases,
            &summary.subject_label
        ),
        display_safe(&c.predicate),
        object,
        c.status,
        c.id,
        c.revision,
        summary.support_count
    )
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
    let mut lines = reconstruction_lines(about.reconstruction.as_ref());
    if !about.resolved {
        lines.push("No entity matches".into());
    }
    let mut accounts = std::collections::BTreeMap::new();
    if let Some(person) = &about.person {
        lines.push(format!(
            "Person: {} | Id: {}",
            display_safe(&person.identifier),
            person.id
        ));
        for account in &person.accounts {
            accounts.insert(account.id.as_str(), account.identifier.as_str());
            let mut line = format!(
                "  Account: {} | Type: {}",
                display_safe(&account.identifier),
                display_safe(&account.entity_type)
            );
            if !account.aliases.is_empty() {
                line += &format!(
                    " | Aliases: {}",
                    account
                        .aliases
                        .iter()
                        .map(|a| display_safe(a))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            for reason in &account.reasons {
                let kind = serde_json::to_value(reason.kind)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                line += &format!(" | Linked by: {kind}");
                if let Some(intent) = &reason.intent {
                    line += &format!(" {}", display_safe(intent));
                }
                line += &format!(" (claim {})", reason.claim);
            }
            lines.push(line);
        }
    }
    for claim in &about.claims {
        let mut line = claim_line(&claim.summary);
        if let Some(account) = &claim.account {
            line += &format!(
                " | Account: {}",
                display_safe(accounts.get(account.as_str()).copied().unwrap_or(account))
            );
        }
        lines.push(line);
    }
    if about.resolved && about.claims.is_empty() {
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
    let mut lines = reconstruction_lines(why.reconstruction.as_ref());
    lines.push(claim_line(&why.claim));
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
                "  Justification: {} | Kind: {} | Intent: {} {} | Issued: {} | Issuer: {} via {} | Scopes: {} | Current: {}",
                support.justification_id,
                display_safe(&support.justification_kind),
                display_safe(&support.extraction_rule),
                display_safe(&intent.id),
                stamp(intent.issued_at),
                display_safe(&intent.issuer.principal),
                display_safe(&intent.issuer.surface),
                scopes_shown(&intent.issuer.scopes),
                support.current
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
    line += &format!(
        " | Domain: {} | Issuer: {} via {} | Scopes: {}",
        display_safe(&intent.domain_key),
        display_safe(&intent.issuer.principal),
        display_safe(&intent.issuer.surface),
        scopes_shown(&intent.issuer.scopes)
    );
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
/// One line per candidate pair: both accounts (with the Persons holding them), each reason with
/// the value each account gave, and the name a yes would give a new Person.
fn candidates_text(result: &rastro_operations::results::IdentityCandidatesResult) -> String {
    if result.candidates.is_empty() {
        return "No candidates".into();
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
    let lines: Vec<String> = (result.candidates.iter())
        .map(|c| {
            let reasons: Vec<String> = c.reasons.iter().map(reason).collect();
            format!(
                "{} ~ {} | {} | Name: {}",
                account(&c.accounts[0]),
                account(&c.accounts[1]),
                reasons.join(" | "),
                display_safe(&c.name)
            )
        })
        .collect();
    lines.join("\n")
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
fn meeting_text(m: &rastro_core::meeting::MeetingCommitments) -> String {
    let mut lines = vec![
        format!(
            "Meeting: {} | Scope: {} | Captured: {}",
            display_safe(&m.meeting.id),
            display_safe(&m.meeting.scope),
            stamp(m.meeting.captured_at)
        ),
        m.notice.clone(),
    ];
    let e = &m.extraction;
    let mut line = format!(
        "Extraction: {} | Run: {} | Finished: {}",
        e.state.as_str(),
        e.run,
        stamp(e.finished_at)
    );
    if let Some(extractor) = &e.extractor {
        line += &format!(" | Extractor: {}", display_safe(extractor));
    }
    if let Some(source) = e.replay_of {
        line += &format!(" | Replay of run {source}, no model call");
    }
    line += &format!(" | Excluded: {}", e.excluded);
    if let Some(cause) = e.cause {
        line += &format!(" | Cause: {}", cause.as_str());
    }
    if let Some(reason) = &e.reason {
        line += &format!(" | Reason: {}", display_safe(reason));
    }
    lines.push(line);
    lines.push(format!(
        "Coverage: {} | Turns read: {} of {}",
        m.coverage.state.as_str(),
        m.coverage.turns_read,
        m.coverage.turns_total
    ));
    if let Some(from) = &m.results_from {
        lines.push(format!(
            "STALE: the hypotheses below come from the last good run {} ({}), not the latest",
            from.run,
            stamp(from.finished_at)
        ));
    }
    lines.push(format!(
        "Transcript: {}",
        if m.transcript_retained {
            "retained (opt-in)"
        } else {
            "not retained"
        }
    ));
    if m.hypotheses.is_empty() {
        lines.push("No hypotheses".into());
    }
    for h in &m.hypotheses {
        let date = match &h.stated_date.text {
            Some(text) => display_safe(text),
            None => "unknown".into(),
        };
        lines.push(format!(
            "Hypothesis: {} -> {} | Promise: {} | Date: {}",
            party_text(&h.responsible),
            party_text(&h.recipient),
            display_safe(&h.promise),
            date
        ));
        lines.push(format!(
            "  \"{}\" | Turn: {} | Speaker: {} | Span: [{}, {})",
            display_escaped(&h.quote.text),
            h.quote.turn,
            display_safe(&h.quote.speaker),
            h.quote.start,
            h.quote.end
        ));
    }
    lines.join("\n")
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
/// The commitments list: every source with how its extraction and coverage stand, then its
/// hypotheses, or what it found instead.
fn commitments_text(list: &rastro_core::commitments::Commitments) -> String {
    use rastro_core::meeting::RunState;
    let mut lines = vec![list.notice.clone()];
    if list.sources.is_empty() {
        lines.push("No sources".into());
    }
    for source in &list.sources {
        lines.push(format!(
            "Source: {} | Kind: {} | Scope: {}",
            display_safe(&source.name),
            serde_json::to_value(source.kind)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default(),
            display_safe(&source.scope)
        ));
        let e = &source.extraction;
        let mut line = format!(
            "  Extraction: {} | Run: {} | Finished: {}",
            e.state.as_str(),
            e.run,
            stamp(e.finished_at)
        );
        if let Some(adapter) = &e.adapter {
            line += &format!(" | Adapter: {}", display_safe(adapter));
        }
        if let Some(run) = &e.replay_of {
            line += &format!(" | Replay of run {run}, no model call");
        }
        line += &format!(" | Excluded: {}", e.excluded);
        if let Some(cause) = e.cause {
            line += &format!(" | Cause: {}", cause.as_str());
        }
        if let Some(reason) = &e.reason {
            line += &format!(" | Reason: {}", display_safe(reason));
        }
        lines.push(line);
        let c = &source.coverage;
        let mut line = format!(
            "  Coverage: {} | Turns read: {} of {}",
            c.state.as_str(),
            c.turns_read,
            c.turns_total
        );
        if let Some(reason) = &c.reason {
            line += &format!(" | Reason: {}", display_safe(reason));
        }
        lines.push(line);
        if source.stale
            && let Some(from) = &source.results_from
        {
            lines.push(format!(
                "  STALE: the hypotheses below come from run {} (imported {}), not the latest",
                from.run,
                stamp(from.imported_at)
            ));
        }
        match (source.hypotheses.is_empty(), e.state) {
            (true, RunState::Succeeded) => lines.push("  Found nothing".into()),
            (true, RunState::Failed) => lines.push("  Couldn't look: no hypotheses".into()),
            (false, _) => {}
        }
        for h in &source.hypotheses {
            let date = match &h.stated_date.text {
                Some(text) => display_safe(text),
                None => "unknown".into(),
            };
            lines.push(format!(
                "  Hypothesis: {} | {} -> {} | Promise: {} | Date: {} | Terms: {}",
                h.kind.as_str(),
                commitment_party_text(&h.responsible),
                commitment_party_text(&h.recipient),
                display_safe(&h.promise),
                date,
                h.terms.join(", ")
            ));
            lines.push(format!(
                "    \"{}\" | Turn: {} | Speaker: {} | Span: [{}, {})",
                display_escaped(&h.quote.text),
                h.quote.turn,
                display_safe(&h.quote.speaker),
                h.quote.start,
                h.quote.end
            ));
        }
    }
    lines.join("\n")
}
/// The focus report for a terminal: the recommended focus, the needs-you items with their
/// citations, the queue count, the gaps and the as-of time.
fn focus_text(report: &rastro_core::focus::Report) -> String {
    use rastro_core::focus::{GapReason, Item, SourceKind};
    let cited = |item: &Item| {
        format!(
            "{} ({} {} | observed {})",
            display_safe(&item.title),
            match item.citation.source {
                SourceKind::Linear => "linear",
                SourceKind::Meeting => "meeting",
            },
            display_safe(&item.citation.location),
            item.citation.observed_at.to_rfc3339()
        )
    };
    let mut lines = Vec::new();
    match &report.focus {
        Some(item) => lines.push(format!("Focus: {}", cited(item))),
        None => lines.push(display_safe(
            report
                .notice
                .as_deref()
                .unwrap_or(rastro_core::focus::NOTHING_NEEDS_YOU),
        )),
    }
    lines.extend(
        report
            .needs_you
            .iter()
            .map(|item| format!("Needs you: {}", cited(item))),
    );
    lines.push(format!("Queue: {} waiting", report.queue_count));
    for gap in &report.gaps {
        let why = match gap.reason {
            GapReason::SyncFailed => "its last sync failed".to_string(),
            GapReason::ExtractionFailed { .. } => "its extraction failed".to_string(),
        };
        let as_of = gap.as_of.map_or_else(
            || "never read".to_string(),
            |t| format!("as of {}", t.to_rfc3339()),
        );
        lines.push(format!(
            "Gap: {} | {why} | {as_of}",
            display_safe(&gap.name)
        ));
    }
    lines.push(format!("As of {}", report.as_of.to_rfc3339()));
    lines.join("\n")
}

/// The name a queue source's kind goes by in the owner's reads.
fn queue_source_kind(kind: rastro_core::queue::SourceKind) -> &'static str {
    match kind {
        rastro_core::queue::SourceKind::Meeting => "meeting",
        rastro_core::queue::SourceKind::Identity => "identity",
        rastro_core::queue::SourceKind::Linear => "linear",
    }
}

/// The lines of an action proposal: what it would do, the assignee and why, and what it cites.
fn action_proposal_lines(action: &rastro_core::actions::Proposal) -> Vec<String> {
    let fields = &action.fields;
    let mut lines = vec![format!(
        "  {} | Title: {} | Team: {} | Assignee: {} | Why: {}",
        display_safe(&action.action),
        display_safe(&fields.title),
        display_safe(&fields.team),
        display_safe(&fields.assignee),
        display_safe(&action.why)
    )];
    if let Some(description) = &fields.description {
        lines.push(format!("  Description: {}", display_safe(description)));
    }
    if !action.cites.is_empty() {
        let cites: Vec<String> = action.cites.iter().map(|c| display_safe(c)).collect();
        lines.push(format!("  Cites: {}", cites.join(", ")));
    }
    lines
}

/// What `actions propose` recorded: the proposal's id, what it would do, and the run a matching
/// rule started or the queue it waits in.
fn actions_propose_text(proposed: &rastro_core::actions::Proposed) -> String {
    let filed = &proposed.filed;
    let head = format!(
        "Proposed {} | Source: {} {} | Scope: {}",
        filed.id,
        queue_source_kind(filed.source.kind),
        display_safe(&filed.source.name),
        display_safe(&filed.source.scope)
    );
    let mut lines = match &proposed.run {
        Some(run) if run.status == rastro_core::actions::RunStatus::Pending => match run.due_at {
            Some(due) => vec![format!(
                "{head} | Run {} runs at {} unless stopped: {}",
                run.id,
                due.format("%Y-%m-%d %H:%M"),
                rastro_core::actions::stop_command(&run.id)
            )],
            None => vec![format!("{head} | Run {}", run.id)],
        },
        Some(run) => vec![format!("{head} | Already has a run"), action_run_text(run)],
        None => match &proposed.cap {
            Some(wait) => vec![format!(
                "{head} | Waiting in the queue: the rule {} reached its daily cap of {}",
                display_safe(&wait.rule),
                wait.cap
            )],
            None => vec![format!("{head} | Waiting in the queue; nothing has run")],
        },
    };
    lines.extend(action_proposal_lines(&filed.proposal));
    lines.join("\n")
}

/// The queue: each proposal or question with its id and source, then each source's gap.
#[expect(
    clippy::too_many_lines,
    reason = "each item, its source and the gaps after them are written in one pass, so their order reads at a glance"
)]
fn queue_text(queue: &rastro_core::queue::Queue) -> String {
    use rastro_core::queue::ItemKind;
    let mut lines = vec![queue.notice.clone()];
    if queue.items.is_empty() && queue.gaps.is_empty() {
        lines.push("Nothing is waiting".into());
    }
    for item in &queue.items {
        let source = format!(
            "{} {}",
            queue_source_kind(item.source.kind),
            display_safe(&item.source.name)
        );
        match item.kind {
            ItemKind::ActionProposal => {
                let action = item.action.as_ref().expect("an action item carries one");
                match &item.act {
                    Some(run) => {
                        let left = match item.runs_in_seconds {
                            Some(seconds) if seconds > 0 => {
                                format!("runs in {}", grace_text(seconds.unsigned_abs()))
                            }
                            _ => "due now".into(),
                        };
                        lines.push(format!(
                            "Pending Act | {} | {left} | {}",
                            display_safe(&action.fields.title),
                            run.stop_command
                                .clone()
                                .unwrap_or_else(|| rastro_core::actions::stop_command(&run.id)),
                        ));
                        lines.push(format!(
                            "  Id: {} | Source: {source} | Scope: {}",
                            item.id,
                            display_safe(&item.source.scope)
                        ));
                    }
                    None => lines.push(format!(
                        "Action proposal | Id: {} | Source: {source} | Scope: {}",
                        item.id,
                        display_safe(&item.source.scope)
                    )),
                }
                lines.extend(action_proposal_lines(action));
                if !item.possible_duplicates.is_empty() {
                    lines.push(format!(
                        "  Warning: possible duplicate of {}",
                        (item.possible_duplicates.iter())
                            .map(|id| display_safe(id))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            ItemKind::Proposal => {
                let h = item.proposal.as_ref().expect("a proposal item carries one");
                lines.push(format!(
                    "Proposal | Id: {} | Source: {source} | Scope: {}",
                    item.id,
                    display_safe(&item.source.scope)
                ));
                let date = match &h.stated_date.text {
                    Some(text) => display_safe(text),
                    None => "unknown".into(),
                };
                lines.push(format!(
                    "  {} | {} -> {} | Promise: {} | Date: {} | Terms: {}",
                    h.kind.as_str(),
                    commitment_party_text(&h.responsible),
                    commitment_party_text(&h.recipient),
                    display_safe(&h.promise),
                    date,
                    h.terms.join(", ")
                ));
                lines.push(format!(
                    "  \"{}\" | Turn: {} | Speaker: {} | Span: [{}, {})",
                    display_escaped(&h.quote.text),
                    h.quote.turn,
                    display_safe(&h.quote.speaker),
                    h.quote.start,
                    h.quote.end
                ));
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
                lines.push(format!("Question | Id: {} | Source: {source}", item.id));
                lines.push(format!(
                    "  {kind} | Claim: {} | Status: {} ({mark}) | Intent: {} | Answer yes to hold it again",
                    claim.id, claim.status, c.intent
                ));
            }
            ItemKind::Question => {
                let q = item.question.as_ref().expect("a question item carries one");
                lines.push(format!("Question | Id: {} | Source: {source}", item.id));
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
                let reasons: Vec<String> = q.reasons.iter().map(reason).collect();
                lines.push(format!(
                    "  {} ~ {} | {} | Name: {}",
                    account(&q.accounts[0]),
                    account(&q.accounts[1]),
                    reasons.join(" | "),
                    display_safe(&q.name)
                ));
            }
        }
    }
    for gap in &queue.gaps {
        let mut line = format!(
            "Gap: {} | Kind: {} | Scope: {} | Run: {}",
            display_safe(&gap.source.name),
            queue_source_kind(gap.source.kind),
            display_safe(&gap.source.scope),
            gap.run
        );
        line += match gap.state {
            rastro_core::queue::GapState::FoundNothing => " | Found nothing",
            rastro_core::queue::GapState::Failed => " | Failed",
            rastro_core::queue::GapState::CouldNotLook => " | Couldn't look",
        };
        if let Some(cause) = gap.cause {
            line += &format!(" | Cause: {}", cause.as_str());
        }
        if let Some(reason) = &gap.reason {
            line += &format!(" | Reason: {}", display_safe(reason));
        }
        lines.push(line);
    }
    lines.join("\n")
}

/// The outcomes: each item with what happened to it and when, then what was shown and acted on.
fn outcomes_text(outcomes: &rastro_core::queue::Outcomes) -> String {
    let mut lines = vec![outcomes.notice.clone()];
    if outcomes.outcomes.is_empty() {
        lines.push("Nothing has happened yet".into());
    }
    for outcome in &outcomes.outcomes {
        let kind = match outcome.kind {
            rastro_core::queue::OutcomeKind::Accepted => "accepted",
            rastro_core::queue::OutcomeKind::AcceptedWithEdits => "accepted with edits",
            rastro_core::queue::OutcomeKind::Rejected => "rejected",
            rastro_core::queue::OutcomeKind::Answered => "answered",
            rastro_core::queue::OutcomeKind::BecameMoot => "became moot",
            rastro_core::queue::OutcomeKind::Ignored => "ignored",
        };
        let mut line = format!(
            "Item: {} | Kind: {kind} | At: {}",
            outcome.item,
            stamp(outcome.at)
        );
        if let Some(claim) = &outcome.claim {
            line += &format!(" | Claim: {claim}");
        }
        if let Some(waited) = outcome.waited_seconds {
            line += &format!(" | Waited: {waited}s");
        }
        lines.push(line);
    }
    lines.push(format!(
        "Shown: {} | Acted on: {}",
        outcomes.totals.shown, outcomes.totals.acted_on
    ));
    lines.join("\n")
}

/// What a queue answer recorded: the item it named, then each owner intent.
fn queue_answer_text(result: &rastro_operations::results::QueueAnswerResult) -> String {
    let mut lines = Vec::new();
    if let Some(item) = &result.item {
        lines.push(format!("Item: {item}"));
    }
    lines.push(intents_text(&result.intents, "Nothing recorded"));
    lines.extend(result.run.iter().map(action_run_text));
    lines.join("\n")
}

/// One action run: its id, status, level, the action and where it wrote, and the issue it created.
fn action_run_text(run: &rastro_core::actions::Run) -> String {
    let mut line = format!(
        "Run {} | {} | Level: {} | {} | Source: {} {} | Scope: {} | Title: {} | Team: {} | Assignee: {}",
        run.id,
        run.status.as_str(),
        run.level.as_str(),
        display_safe(&run.action),
        queue_source_kind(run.source.kind),
        display_safe(&run.source.name),
        display_safe(&run.source.scope),
        display_safe(&run.fields.title),
        display_safe(&run.fields.team),
        display_safe(&run.fields.assignee)
    );
    if let (Some(identifier), Some(url)) = (&run.identifier, &run.url) {
        line += &format!(
            " | Issue: {} {}",
            display_safe(identifier),
            display_safe(url)
        );
    }
    if let Some(due) = run.due_at {
        let label = if run.status == rastro_core::actions::RunStatus::Pending {
            "Runs at"
        } else {
            "Grace ended"
        };
        line += &format!(" | {label}: {}", stamp(due));
    }
    if let Some(stop) = &run.stop_command {
        line += &format!(" | Stop: {stop}");
    }
    if let Some(error) = &run.error {
        line += &format!(
            " | Error: {} | Attempts: {}",
            display_safe(error),
            run.attempts
        );
    }
    if let Some(undo) = &run.undo {
        line += &format!(
            " | Undone: intent {}, {}",
            display_safe(&undo.intent),
            undo.at.format("%Y-%m-%d %H:%M")
        );
    }
    line
}

/// What `actions` lists: each run, or that none has run.
fn actions_text(runs: &rastro_core::actions::Runs) -> String {
    if runs.runs.is_empty() {
        return "No action has run".into();
    }
    let mut text = (runs.runs.iter())
        .map(action_run_text)
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(why) = &runs.why {
        text += &action_why_text(why, runs.runs.first().and_then(|run| run.rule.as_ref()));
    }
    text
}

/// Why one run happened, under its line: the why, each cited claim, the rule or the accept that
/// authorized it, and the risk.
fn action_why_text(
    why: &rastro_core::actions::RunWhy,
    rule: Option<&rastro_core::rules::RuleCopy>,
) -> String {
    let mut text = format!("\nWhy: {}", display_safe(&why.proposal.why));
    for claim in &why.cited {
        text += &format!(
            "\nCites: {} ({} support{})",
            display_safe(&claim.claim.claim.id),
            claim.claim.support_count,
            if claim.claim.support_count == 1 {
                ""
            } else {
                "s"
            }
        );
    }
    if let Some(rule) = rule {
        text += &format!(
            "\nRule: {} | {} | team {} | assignee {} | grace {}",
            display_safe(&rule.name),
            display_safe(&rule.action),
            display_safe(&rule.team),
            display_safe(&rule.assignee),
            grace_text(rule.grace_seconds)
        );
        if let Some(cap) = rule.daily_cap {
            text += &format!(" | cap {cap} a day");
        }
    }
    if let Some(accept) = &why.accept {
        text += &format!(
            "\nAccepted: intent {}, {}",
            accept.intent,
            accept.at.format("%Y-%m-%d %H:%M")
        );
    }
    text += &format!("\nQueue: {}", standing_text(&why.queue));
    text += &format!(
        "\nLevel: {} | Cell: {} x {} | Reach: {} | Reversibility: {}",
        why.level,
        display_safe(&why.cell.action),
        display_safe(&why.cell.team),
        why.reach,
        why.reversibility
    );
    text
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

/// What a claim correction says: what it did, or that the claim already was so.
fn correction_text(r: &rastro_operations::results::ClaimWriteResult, did: &str) -> String {
    match r.intents.first() {
        Some(intent) => format!(
            "{did} {} (intent {}, {})",
            r.claim,
            intent.id,
            intent.issued_at.format("%Y-%m-%d %H:%M")
        ),
        None => format!("{} is already {did}; nothing recorded", r.claim),
    }
}
/// What an undo of a claim correction says.
fn undo_text(r: &rastro_operations::results::ClaimWriteResult) -> String {
    match r.intents.first() {
        Some(intent) => format!(
            "undid intent {} on {} (intent {}, {})",
            intent.undo_of.as_deref().unwrap_or_default(),
            r.claim,
            intent.id,
            intent.issued_at.format("%Y-%m-%d %H:%M")
        ),
        None => format!("{} has nothing to undo; nothing recorded", r.claim),
    }
}
/// What a fix of a claim says: the claim it retracted and the one it asserted.
fn edit_text(r: &rastro_operations::results::ClaimWriteResult) -> String {
    match (&r.asserted, r.intents.first()) {
        (Some(asserted), Some(intent)) => format!(
            "retracted {}, asserted {asserted} (intent {}, {})",
            r.claim,
            intent.id,
            intent.issued_at.format("%Y-%m-%d %H:%M")
        ),
        _ => format!("{} already says that; nothing recorded", r.claim),
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
/// One line per rule, in the order the owner wrote them, with its cap and today's count, then
/// its pending Acts and the queued proposals it matches.
fn rules_check_text(r: &rastro_operations::results::RulesCheckResult) -> String {
    if r.rules.is_empty() {
        return "No rules in config.toml".into();
    }
    let mut lines = Vec::new();
    for rule in &r.rules {
        let mut line = format!(
            "{} | {} | team {} | assignee {} | grace {}",
            display_safe(&rule.name),
            display_safe(&rule.action),
            display_safe(&rule.team),
            display_safe(&rule.assignee),
            grace_text(rule.grace_seconds)
        );
        match rule.daily_cap {
            Some(cap) => line += &format!(" | {} of {cap} used today", rule.used_today),
            None => line += &format!(" | {} used today", rule.used_today),
        }
        lines.push(line);
        lines.extend(
            rule.pending
                .iter()
                .map(|run| format!("  Pending: {}", action_run_text(run))),
        );
        lines.extend(rule.matches.iter().map(|filed| {
            format!(
                "  Matches: {} | {}",
                filed.id,
                display_safe(&filed.proposal.fields.title)
            )
        }));
    }
    lines.join("\n")
}
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
        Response::SourceCheck(r) => check::line(r),
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
        Response::Commitments(r) => commitments_text(r),
        Response::Queue(r) => queue_text(r),
        Response::Actions(r) => actions_text(r),
        Response::ActionsShow(r) => actions_text(r),
        Response::ActionsUndo(r) => format!("Undone: {}", action_run_text(r)),
        Response::ActionsStop(r) => format!("Stopped: {}", action_run_text(r)),
        Response::ActionsPropose(r) => actions_propose_text(r),
        Response::RulesCheck(r) => rules_check_text(r),
        Response::Focus(r) => focus_text(r),
        Response::QueueOutcomes(r) => outcomes_text(r),
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
            intents_text(&r.intents, "Already in force; nothing recorded")
        }
        Response::ClaimConfirm(r) => correction_text(r, "confirmed"),
        Response::ClaimReject(r) => correction_text(r, "retracted"),
        Response::ClaimUndo(r) => undo_text(r),
        Response::ClaimEdit(r) => edit_text(r),
        Response::IdentityCandidates(r) => candidates_text(r),
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
