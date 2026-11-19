//! Action proposals: the insert-only record of what an agent proposed to do in an outside system,
//! bound to the source it writes to and that source's scope.
use crate::{View, parse_time, scope_parameter, stamp};
use anyhow::Result;
use rastro_core::{
    Scope,
    actions::{Filed, IssueFields, Proposal},
    queue::{Source, SourceKind},
};
use rusqlite::{Connection, params};

/// Records a proposal. One already recorded with the same content address is the same proposal,
/// so it is left as it is and nothing is added: the insert selects no row, so the table's
/// insert-only guard (which refuses a repeated id) is never reached.
pub(crate) fn record(connection: &mut Connection, filed: &Filed) -> Result<()> {
    connection.execute(
        "INSERT INTO action_proposal
             (id, action, provider, fields, why, cites, source, scope, proposed_at)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9
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
        ],
    )?;
    Ok(())
}

/// Every proposal whose source is in `scopes` (empty: every scope), oldest first.
pub(crate) fn proposals(view: &View, scopes: &[Scope]) -> Result<Vec<Filed>> {
    let mut statement = view.0.prepare(
        "SELECT id, action, provider, fields, why, cites, source, scope, proposed_at
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
        ))
    })?;
    let mut filed = Vec::new();
    for row in rows {
        let (id, action, provider, fields, why, cites, source, scope, proposed_at) = row?;
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
            proposed_at: parse_time(proposed_at)?,
        });
    }
    Ok(filed)
}
