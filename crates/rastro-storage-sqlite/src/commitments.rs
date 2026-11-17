//! The commitments list: every kind of source that has hypotheses, read as source-neutral
//! entries. Meetings are the only kind today.
use crate::{View, meeting};
use anyhow::Result;
use rastro_core::{Scope, commitments::Source, queue::Origin};

/// Every source visible to `scopes` (empty: every scope), most recently captured first, each
/// with the time its content belongs to.
pub(crate) fn origins(view: &View, scopes: &[Scope]) -> Result<Vec<Origin>> {
    let meetings: Vec<String> = view
        .0
        .prepare("SELECT id FROM meeting ORDER BY captured_at DESC, id")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let mut origins = Vec::new();
    for id in meetings {
        if let Some(stored) = meeting::load(view, &id, scopes)? {
            origins.push(Origin {
                at: stored.meeting.captured_at,
                source: stored.source(),
            });
        }
    }
    Ok(origins)
}

/// Every source visible to `scopes` (empty: every scope), most recently captured first.
pub(crate) fn sources(view: &View, scopes: &[Scope]) -> Result<Vec<Source>> {
    Ok(origins(view, scopes)?
        .into_iter()
        .map(|origin| origin.source)
        .collect())
}
