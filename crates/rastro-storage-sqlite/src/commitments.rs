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
/// ever matches a hypothesis against one commitment; a name or a text match drops nothing. An
/// accepted proposal's claims count too, in the scopes that see the item's source: what the owner
/// accepted is knowledge the queue never proposes again. A statement the owner retracted, or a
/// slot a retracted anchor took, is no longer known: it is listed beside, so the queue can mark a
/// proposal that returns it.
fn known(view: &View, scopes: &[Scope]) -> Result<Vec<KnownCommitment>> {
    let every = scopes.is_empty();
    let scopes = serde_json::to_string(scopes)?;
    let mut held: BTreeMap<String, (BTreeSet<Statement>, BTreeSet<Statement>)> = BTreeMap::new();
    for (current, status) in [
        (true, "status IN ('observed', 'confirmed')"),
        (
            false,
            "(status = 'retracted' OR (status = 'superseded' AND EXISTS (SELECT 1 FROM intent_justification ij JOIN intent i ON i.id = ij.intent_id WHERE ij.claim_id = c.id AND ij.ended_at IS NOT NULL AND i.kind IN ('accept', 'edit') AND json_extract(i.targets_json, '$.shape') = 'proposal')))",
        ),
    ] {
        let mut statement = view.0.prepare_cached(&format!(
            "SELECT subject, predicate, object_value FROM claim c
             WHERE predicate LIKE 'std:%' AND {status} AND object_value IS NOT NULL
             AND (?1 OR {} OR {})",
            scoped::VISIBLE_CLAIM,
            scoped::VISIBLE_INTENT_CLAIM
        ))?;
        let rows = statement.query_map(rusqlite::params![every, scopes], |row| {
            Ok((
                row.get::<_, String>(0)?,
                Statement {
                    predicate: row.get(1)?,
                    value: row.get(2)?,
                },
            ))
        })?;
        for row in rows {
            let (subject, statement) = row?;
            let (known, retracted) = held.entry(subject).or_default();
            if current { known } else { retracted }.insert(statement);
        }
    }
    Ok((held.into_iter())
        .map(|(subject, (statements, retracted))| KnownCommitment {
            subject,
            statements,
            retracted,
        })
        .collect())
}
