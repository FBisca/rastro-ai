//! Action proposals: the insert-only record of what an agent proposed to do in an outside system,
//! bound to the source it writes to and that source's scope.
use crate::{View, parse_time, scope_parameter, stamp};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rastro_core::{
    Scope,
    actions::{
        Filed, IssueFields, NewRun, ObservedIssue, Proposal, Reach, Reversibility, Run, RunResult,
        RunStatus,
    },
    queue::{Source, SourceKind},
};
use rusqlite::{Connection, params};

/// Records a proposal. One already recorded with the same content address is the same proposal,
/// so it is left as it is and nothing is added: the insert selects no row, so the table's
/// insert-only guard (which refuses a repeated id) is never reached.
pub(crate) fn record(connection: &mut Connection, filed: &Filed) -> Result<()> {
    connection.execute(
        "INSERT INTO action_proposal
             (id, action, provider, fields, why, cites, source, scope, proposed_at, reach, reversibility)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11
         WHERE NOT EXISTS (SELECT 1 FROM action_proposal WHERE id = ?1)",
        params![
            filed.id,
            filed.proposal.action,
            filed.provider,
            serde_json::to_string(&filed.proposal.fields)?,
            filed.proposal.why,
            serde_json::to_string(&filed.proposal.cites)?,
            filed.source.name,
            filed.source.scope,
            stamp(filed.proposed_at),
            serde_json::to_value(filed.reach)?.as_str(),
            serde_json::to_value(filed.reversibility)?.as_str(),
        ],
    )?;
    Ok(())
}

/// Every proposal whose source is in `scopes` (empty: every scope), oldest first.
pub(crate) fn proposals(view: &View, scopes: &[Scope]) -> Result<Vec<Filed>> {
    let mut statement = view.0.prepare(
        "SELECT id, action, provider, fields, why, cites, source, scope, proposed_at,
                reach, reversibility
         FROM action_proposal
         WHERE ?1 IS NULL OR scope IN (SELECT value FROM json_each(?1))
         ORDER BY append_seq",
    )?;
    let rows = statement.query_map([scope_parameter(scopes)?], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, String>(9)?,
            row.get::<_, String>(10)?,
        ))
    })?;
    let mut filed = Vec::new();
    for row in rows {
        let (
            id,
            action,
            provider,
            fields,
            why,
            cites,
            source,
            scope,
            proposed_at,
            reach,
            reversibility,
        ) = row?;
        filed.push(Filed {
            id,
            proposal: Proposal {
                action,
                fields: serde_json::from_str::<IssueFields>(&fields)?,
                why,
                cites: serde_json::from_str(&cites)?,
            },
            source: Source {
                kind: SourceKind::Linear,
                name: source,
                scope,
            },
            provider,
            reach: Reach::parse(&reach).with_context(|| format!("unknown reach {reach}"))?,
            reversibility: Reversibility::parse(&reversibility)
                .with_context(|| format!("unknown reversibility {reversibility}"))?,
            proposed_at: parse_time(proposed_at)?,
        });
    }
    Ok(filed)
}

/// The issues a Linear source observed: the latest evidence of each field of each
/// `linear:issue/…` location, in the scopes given (empty: every scope).
pub(crate) fn linear_issues(
    view: &View,
    source: &str,
    scopes: &[Scope],
) -> Result<Vec<ObservedIssue>> {
    let mut statement = view.0.prepare(
        "SELECT e.location, e.field, e.quote FROM evidence e
         WHERE e.source = ?1 AND e.location LIKE 'linear:issue/%'
           AND e.field IN ('/identifier', '/title', '/state/type')
           AND (?2 IS NULL OR e.scope IN (SELECT value FROM json_each(?2)))
           AND e.id = (SELECT max(x.id) FROM evidence x WHERE x.source = e.source
                       AND x.scope = e.scope AND x.location = e.location AND x.field = e.field)
         ORDER BY e.location, e.field",
    )?;
    let rows = statement.query_map(params![source, scope_parameter(scopes)?], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut issues: std::collections::BTreeMap<String, ObservedIssue> = Default::default();
    for row in rows {
        let (location, field, quote) = row?;
        let issue = issues.entry(location).or_insert_with(|| ObservedIssue {
            identifier: String::new(),
            title: String::new(),
            state: String::new(),
        });
        match field.as_str() {
            "/identifier" => issue.identifier = quote,
            "/title" => issue.title = quote,
            _ => issue.state = quote,
        }
    }
    Ok(issues
        .into_values()
        .filter(|issue| !issue.identifier.is_empty() && !issue.title.is_empty())
        .collect())
}

/// Records the run an accept starts, in the accept's own transaction. Returns it as it reads
/// then: `accepted`, with no issue.
pub(crate) fn record_run(
    connection: &Connection,
    run: &NewRun,
    accepted_at: DateTime<Utc>,
) -> Result<Run> {
    connection.execute(
        "INSERT INTO action_run
             (id, proposal, accept_intent, action, provider, fields, source, scope, issue_id,
              accepted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            run.id,
            run.proposal,
            run.accept_intent,
            run.action,
            run.provider,
            serde_json::to_string(&run.fields)?,
            run.source.name,
            run.source.scope,
            run.issue_id,
            stamp(accepted_at),
        ],
    )?;
    Ok(Run {
        id: run.id.clone(),
        proposal: run.proposal.clone(),
        accept_intent: run.accept_intent.clone(),
        action: run.action.clone(),
        fields: run.fields.clone(),
        source: run.source.clone(),
        provider: run.provider.clone(),
        issue_id: run.issue_id.clone(),
        accepted_at,
        status: RunStatus::Accepted,
        identifier: None,
        url: None,
    })
}

/// Records the issue a run created. A run that already holds a result keeps it, and nothing is
/// added: the insert selects no row, so the table's insert-only guard is never reached.
pub(crate) fn record_result(
    connection: &mut Connection,
    run: &str,
    result: &RunResult,
) -> Result<()> {
    connection.execute(
        "INSERT INTO action_result (run_id, identifier, url, finished_at)
         SELECT ?1, ?2, ?3, ?4
         WHERE NOT EXISTS (SELECT 1 FROM action_result WHERE run_id = ?1)",
        params![
            run,
            result.identifier,
            result.url,
            stamp(result.finished_at)
        ],
    )?;
    Ok(())
}

/// Every run whose source is in `scopes` (empty: every scope), oldest first, with the issue it
/// created when it did.
pub(crate) fn runs(view: &View, scopes: &[Scope]) -> Result<Vec<Run>> {
    let mut statement = view.0.prepare(
        "SELECT r.id, r.proposal, r.accept_intent, r.action, r.provider, r.fields, r.source,
                r.scope, r.issue_id, r.accepted_at, d.identifier, d.url
         FROM action_run r LEFT JOIN action_result d ON d.run_id = r.id
         WHERE ?1 IS NULL OR r.scope IN (SELECT value FROM json_each(?1))
         ORDER BY r.append_seq",
    )?;
    let rows = statement.query_map([scope_parameter(scopes)?], |row| {
        Ok((
            [
                row.get::<_, String>(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
            ],
            row.get::<_, Option<String>>(10)?,
            row.get::<_, Option<String>>(11)?,
        ))
    })?;
    let mut runs = Vec::new();
    for row in rows {
        let (text, identifier, url) = row?;
        let [
            id,
            proposal,
            accept_intent,
            action,
            provider,
            fields,
            source,
            scope,
            issue_id,
            accepted_at,
        ] = text;
        runs.push(Run {
            id,
            proposal,
            accept_intent,
            action,
            fields: serde_json::from_str::<IssueFields>(&fields)?,
            source: Source {
                kind: SourceKind::Linear,
                name: source,
                scope,
            },
            provider,
            issue_id,
            accepted_at: parse_time(accepted_at)?,
            status: if identifier.is_some() {
                RunStatus::Done
            } else {
                RunStatus::Accepted
            },
            identifier,
            url,
        });
    }
    Ok(runs)
}
