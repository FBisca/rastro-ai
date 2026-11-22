//! The commitments list: every kind of source that has hypotheses, read as source-neutral
//! entries: meetings and the chat threads of opted-in channel sources.
use crate::{View, meeting, scoped};
use anyhow::Result;
use rastro_core::{
    Scope,
    commitments::{Source, SourceKind},
    focus::LinearRead,
    meeting::StoredMeeting,
    queue::{KnownCommitment, Origin, Statement},
};
use rusqlite::OptionalExtension;
use std::collections::{BTreeMap, BTreeSet};

/// Every source visible to `scopes` (empty: every scope), most recently captured first.
pub(crate) fn sources(view: &View, scopes: &[Scope]) -> Result<Vec<Source>> {
    let conversations: Vec<(String, String)> = view
        .0
        .prepare("SELECT id, kind FROM conversation ORDER BY captured_at DESC, id")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut sources = Vec::new();
    for (id, kind) in conversations {
        let Some(stored) = meeting::load(view, &id, scopes)? else {
            continue;
        };
        let mut source = stored.source();
        source.kind = if kind == "chat" {
            SourceKind::Chat
        } else {
            SourceKind::Meeting
        };
        sources.push(source);
    }
    Ok(sources)
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
        .prepare(
            "SELECT id FROM conversation WHERE kind = 'meeting' ORDER BY captured_at DESC, id",
        )?
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

/// A Linear source's latest connector read and its issues as the latest evidence of each field
/// says them. A run that never finished is not a read; the last good read stamps a failed one.
pub(crate) fn linear_read(
    view: &View,
    source: &str,
    scopes: &[Scope],
) -> Result<Option<LinearRead>> {
    use rastro_core::focus::{GapReason, Read, Task, TaskState, Urgency};
    let latest: Option<(String, String)> = view
        .0
        .query_row(
            "SELECT status, finished_at FROM runs
             WHERE role = 'connector' AND source = ?1 AND finished_at IS NOT NULL
             ORDER BY id DESC LIMIT 1",
            [source],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((status, finished)) = latest else {
        return Ok(None);
    };
    let last_good: Option<String> = view.0.query_row(
        "SELECT max(finished_at) FROM runs
         WHERE role = 'connector' AND source = ?1 AND status = 'ok' AND finished_at IS NOT NULL",
        [source],
        |row| row.get(0),
    )?;
    let at = |text: String| -> Result<chrono::DateTime<chrono::Utc>> { Ok(text.parse()?) };
    let last_good = last_good.map(at).transpose()?;
    let read = if status == "failed" {
        Read::Failed {
            reason: GapReason::SyncFailed,
            last_good,
        }
    } else {
        Read::Good { at: at(finished)? }
    };
    let mut statement = view.0.prepare(
        "SELECT e.location, e.field, e.quote FROM evidence e
         WHERE e.source = ?1 AND e.location LIKE 'linear:issue/%'
           AND e.field IN ('/identifier', '/url', '/title', '/state/type', '/assignee/id')
           AND (?2 IS NULL OR e.scope IN (SELECT value FROM json_each(?2)))
           AND e.id = (SELECT max(x.id) FROM evidence x WHERE x.source = e.source
                       AND x.scope = e.scope AND x.location = e.location AND x.field = e.field)
         ORDER BY e.location, e.field",
    )?;
    let rows = statement.query_map(
        rusqlite::params![source, crate::scope_parameter(scopes)?],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    )?;
    let mut tasks: BTreeMap<String, Task> = BTreeMap::new();
    let mut assignees: BTreeMap<String, String> = BTreeMap::new();
    for row in rows {
        let (location, field, quote) = row?;
        let task = tasks.entry(location.clone()).or_insert_with(|| Task {
            location: location.clone(),
            identifier: String::new(),
            title: String::new(),
            url: None,
            state: TaskState::Open,
            due: None,
            urgency: Urgency::None,
        });
        match field.as_str() {
            "/assignee/id" => {
                assignees.insert(location, quote);
            }
            "/identifier" => task.identifier = quote,
            "/url" => task.url = Some(quote),
            "/title" => task.title = quote,
            _ => {
                task.state = match quote.as_str() {
                    "started" => TaskState::InProgress,
                    "completed" | "canceled" => TaskState::Closed,
                    _ => TaskState::Open,
                };
            }
        }
    }
    // Only what the owner holds needs the owner: the connector also syncs issues the owner filed
    // or follows. Without the owner's own Linear id nothing can be told apart, so nothing is kept.
    let viewer: Option<String> = view
        .0
        .query_row(
            "SELECT quote FROM evidence WHERE source = ?1 AND location = 'linear:viewer'
             AND field = '/id' ORDER BY id DESC LIMIT 1",
            [source],
            |row| row.get(0),
        )
        .optional()?;
    let tasks = tasks
        .into_iter()
        .filter(|(location, _)| viewer.is_some() && assignees.get(location) == viewer.as_ref())
        .map(|(_, task)| task)
        .collect();
    Ok(Some(LinearRead { read, tasks }))
}
