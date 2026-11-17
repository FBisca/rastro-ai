//! The commitments list: every kind of source that has hypotheses, read as source-neutral
//! entries. Meetings are the only kind today.
use crate::{View, meeting, scoped};
use anyhow::Result;
use rastro_core::{
    Scope,
    commitments::Source,
    meeting::StoredMeeting,
    queue::{KnownCommitment, Origin, Statement},
};
use std::collections::{BTreeMap, BTreeSet};

/// Every source visible to `scopes` (empty: every scope), most recently captured first.
pub(crate) fn sources(view: &View, scopes: &[Scope]) -> Result<Vec<Source>> {
    Ok(loaded(view, scopes)?
        .into_iter()
        .map(|stored| stored.source())
        .collect())
}

/// The same sources, each with the time its content belongs to and the commitments observed
/// claims already hold: what the queue reads to leave out an item it can tell is known.
pub(crate) fn origins(view: &View, scopes: &[Scope]) -> Result<Vec<Origin>> {
    let known = known(view, scopes)?;
    Ok(loaded(view, scopes)?
        .into_iter()
        .map(|stored| Origin {
            at: stored.meeting.captured_at,
            source: stored.source(),
            known: known.clone(),
        })
        .collect())
}

/// Every meeting visible to `scopes`, most recently captured first.
fn loaded(view: &View, scopes: &[Scope]) -> Result<Vec<StoredMeeting>> {
    let meetings: Vec<String> = view
        .0
        .prepare("SELECT id FROM meeting ORDER BY captured_at DESC, id")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    (meetings.iter())
        .filter_map(|id| meeting::load(view, id, scopes).transpose())
        .collect()
}

/// The commitments observed `std:` claims hold, visible to `scopes` (empty: every scope), each
/// with the statements its claims carry. Claims are grouped by their subject, so the queue only
/// ever matches a hypothesis against one commitment; a name or a text match drops nothing.
fn known(view: &View, scopes: &[Scope]) -> Result<Vec<KnownCommitment>> {
    let mut statement = view.0.prepare_cached(&format!(
        "SELECT subject, predicate, object_value FROM claim c
         WHERE predicate LIKE 'std:%' AND status = 'observed' AND object_value IS NOT NULL
         AND (?1 OR {})",
        scoped::VISIBLE_CLAIM
    ))?;
    let every = scopes.is_empty();
    let scopes = serde_json::to_string(scopes)?;
    let rows = statement.query_map(rusqlite::params![every, scopes], |row| {
        Ok((
            row.get::<_, String>(0)?,
            Statement {
                predicate: row.get(1)?,
                value: row.get(2)?,
            },
        ))
    })?;
    let mut held: BTreeMap<String, BTreeSet<Statement>> = BTreeMap::new();
    for row in rows {
        let (subject, statement) = row?;
        held.entry(subject).or_default().insert(statement);
    }
    Ok((held.into_iter())
        .map(|(subject, statements)| KnownCommitment {
            subject,
            statements,
        })
        .collect())
}
