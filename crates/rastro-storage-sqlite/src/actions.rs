//! Action proposals: the insert-only record of what an agent proposed to do in an outside system,
//! bound to the source it writes to and that source's scope.
use crate::{View, parse_time, scope_parameter, stamp};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rastro_core::{
    Scope,
    actions::{
        Filed, IssueFields, NewAct, NewRun, ObservedIssue, Proposal, Reach, Reversibility, Run,
        RunFailure, RunLevel, RunResult, RunReturn, RunStatus, Undo,
    },
    queue::{Source, SourceKind},
    rules::RuleCopy,
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
        accept_intent: Some(run.accept_intent.clone()),
        action: run.action.clone(),
        fields: run.fields.clone(),
        source: run.source.clone(),
        provider: run.provider.clone(),
        issue_id: run.issue_id.clone(),
        accepted_at,
        status: RunStatus::Accepted,
        level: RunLevel::Propose,
        rule: None,
        due_at: None,
        identifier: None,
        url: None,
        attempts: 0,
        error: None,
        undo: None,
    })
}

/// Records the Act run a matching rule starts: no accept intent, the rule copy and the due time.
/// Returns it as it reads then: `pending`, with no issue.
pub(crate) fn record_act(
    connection: &Connection,
    act: &NewAct,
    started_at: DateTime<Utc>,
) -> Result<Run> {
    connection.execute(
        "INSERT INTO action_run
             (id, proposal, accept_intent, action, provider, fields, source, scope, issue_id,
              accepted_at, level, rule, due_at)
         VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'act', ?10, ?11)",
        params![
            act.id,
            act.proposal,
            act.action,
            act.provider,
            serde_json::to_string(&act.fields)?,
            act.source.name,
            act.source.scope,
            act.issue_id,
            stamp(started_at),
            serde_json::to_string(&act.rule)?,
            stamp(act.due_at),
        ],
    )?;
    Ok(Run {
        id: act.id.clone(),
        proposal: act.proposal.clone(),
        accept_intent: None,
        action: act.action.clone(),
        fields: act.fields.clone(),
        source: act.source.clone(),
        provider: act.provider.clone(),
        issue_id: act.issue_id.clone(),
        accepted_at: started_at,
        status: RunStatus::Pending,
        level: RunLevel::Act,
        rule: Some(act.rule.clone()),
        due_at: Some(act.due_at),
        identifier: None,
        url: None,
        attempts: 0,
        error: None,
        undo: None,
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

/// Records a failed execution of a run as its next attempt. A run that already holds a result is
/// done and keeps reading so, and nothing is added.
pub(crate) fn record_failure(
    connection: &mut Connection,
    run: &str,
    failure: &RunFailure,
) -> Result<()> {
    connection.execute(
        "INSERT INTO action_attempt (run_id, attempt, error, failed_at)
         SELECT ?1, 1 + (SELECT count(*) FROM action_attempt WHERE run_id = ?1), ?2, ?3
         WHERE NOT EXISTS (SELECT 1 FROM action_result WHERE run_id = ?1)",
        params![run, failure.error, stamp(failure.failed_at)],
    )?;
    Ok(())
}

/// Records that a pending Act's re-check at grace end failed, with its reason. A run that already
/// holds a result is done and keeps reading so, and a run already returned keeps its first return:
/// the insert selects no row, so the table's insert-only guard is never reached.
pub(crate) fn record_return(
    connection: &mut Connection,
    run: &str,
    returned: &RunReturn,
) -> Result<()> {
    connection.execute(
        "INSERT INTO action_return (run_id, reason, returned_at)
         SELECT ?1, ?2, ?3
         WHERE NOT EXISTS (SELECT 1 FROM action_return WHERE run_id = ?1)
           AND NOT EXISTS (SELECT 1 FROM action_result WHERE run_id = ?1)",
        params![run, returned.reason, stamp(returned.returned_at)],
    )?;
    Ok(())
}

/// Every run whose source is in `scopes` (empty: every scope), oldest first, with the issue it
/// created when it did.
#[expect(
    clippy::too_many_lines,
    reason = "one pass reads each run with its result, attempts, undo, stop, return, level, rule copy and due time, so the status derivation stays in sight of what it rests on"
)]
pub(crate) fn runs(view: &View, scopes: &[Scope]) -> Result<Vec<Run>> {
    let mut statement = view.0.prepare(
        "SELECT r.id, r.proposal, r.accept_intent, r.action, r.provider, r.fields, r.source,
                r.scope, r.issue_id, r.accepted_at, d.identifier, d.url,
                (SELECT count(*) FROM action_attempt a WHERE a.run_id = r.id),
                (SELECT a.error FROM action_attempt a WHERE a.run_id = r.id
                 ORDER BY a.attempt DESC LIMIT 1),
                (SELECT i.id FROM intent i WHERE i.kind = 'undo_action'
                   AND i.domain_key = 'action_run/' || r.id ORDER BY i.append_seq LIMIT 1),
                (SELECT i.issued_at FROM intent i WHERE i.kind = 'undo_action'
                   AND i.domain_key = 'action_run/' || r.id ORDER BY i.append_seq LIMIT 1),
                r.level, r.rule, r.due_at,
                (SELECT 1 FROM action_return t WHERE t.run_id = r.id),
                (SELECT 1 FROM intent i WHERE i.kind = 'stop'
                   AND i.domain_key = 'action_run/' || r.id)
         FROM action_run r LEFT JOIN action_result d ON d.run_id = r.id
         WHERE ?1 IS NULL OR r.scope IN (SELECT value FROM json_each(?1))
         ORDER BY r.append_seq",
    )?;
    let rows = statement.query_map([scope_parameter(scopes)?], |row| {
        Ok((
            [
                row.get::<_, String>(0)?,
                row.get(1)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
            ],
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(10)?,
            row.get::<_, Option<String>>(11)?,
            row.get::<_, u32>(12)?,
            row.get::<_, Option<String>>(13)?,
            row.get::<_, Option<String>>(14)?,
            row.get::<_, Option<String>>(15)?,
            row.get::<_, String>(16)?,
            row.get::<_, Option<String>>(17)?,
            row.get::<_, Option<String>>(18)?,
            row.get::<_, Option<i64>>(19)?,
            row.get::<_, Option<i64>>(20)?,
        ))
    })?;
    let mut runs = Vec::new();
    for row in rows {
        let (
            text,
            accept_intent,
            identifier,
            url,
            attempts,
            error,
            undo_intent,
            undone_at,
            level,
            rule,
            due_at,
            returned,
            stopped,
        ) = row?;
        let [
            id,
            proposal,
            action,
            provider,
            fields,
            source,
            scope,
            issue_id,
            accepted_at,
        ] = text;
        let level = match level.as_str() {
            "act" => RunLevel::Act,
            _ => RunLevel::Propose,
        };
        let rule = rule
            .map(|text| serde_json::from_str::<RuleCopy>(&text))
            .transpose()?;
        let due_at = due_at.map(parse_time).transpose()?;
        let undo = match (undo_intent, undone_at) {
            (Some(intent), Some(at)) => Some(Undo {
                intent,
                at: parse_time(at)?,
            }),
            _ => None,
        };
        let status = if undo.is_some() {
            RunStatus::Undone
        } else if identifier.is_some() {
            RunStatus::Done
        } else if stopped.is_some() {
            RunStatus::Stopped
        } else if returned.is_some() {
            RunStatus::Returned
        } else if attempts > 0 {
            RunStatus::Failed
        } else if level == RunLevel::Act {
            RunStatus::Pending
        } else {
            RunStatus::Accepted
        };
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
            status,
            level,
            rule,
            due_at,
            identifier,
            url,
            attempts,
            // A run that went on to finish no longer carries the error of an earlier attempt.
            error: error.filter(|_| status == RunStatus::Failed),
            undo,
        });
    }
    Ok(runs)
}
