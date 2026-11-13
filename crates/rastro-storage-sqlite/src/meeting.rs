//! Meetings, their runs and their hypotheses (ADR 0011).
use crate::{View, parse_time, stamp};
use anyhow::{Result, bail};
use rastro_core::Scope;
use rastro_core::meeting::{
    Cause, Hypothesis, Identity, Known, MeetingRef, Party, PartyState, Quote, RunRecord, RunRow,
    RunState, StatedDate, StoredMeeting,
};
use rusqlite::{Connection, OptionalExtension, params};

/// A count column, read as the unsigned number it is.
fn count(row: &rusqlite::Row, index: usize) -> rusqlite::Result<usize> {
    Ok(row.get::<_, i64>(index)?.max(0) as usize)
}

fn run_row(connection: &Connection, id: i64) -> Result<RunRow> {
    let row = connection.query_row(
        "SELECT finished_at, state, cause, reason, extractor, turns_total, turns_read, excluded
         FROM meeting_run WHERE id = ?1",
        [id],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                count(r, 5)?,
                count(r, 6)?,
                count(r, 7)?,
            ))
        },
    )?;
    Ok(RunRow {
        id,
        finished_at: parse_time(row.0)?,
        state: if row.1 == "succeeded" {
            RunState::Succeeded
        } else {
            RunState::Failed
        },
        cause: row.2.as_deref().and_then(Cause::parse),
        reason: row.3,
        extractor: row.4,
        turns_total: row.5,
        turns_read: row.6,
        excluded: row.7,
    })
}

fn party(label: Option<String>, entity: Option<String>, identifier: Option<String>) -> Party {
    match (entity, identifier) {
        (Some(id), Some(identifier)) => Party {
            state: PartyState::Mapped,
            label,
            identity: Some(Identity { id, identifier }),
        },
        _ => Party {
            state: PartyState::Unknown,
            label,
            identity: None,
        },
    }
}

pub(crate) fn load(view: &View, meeting: &str, scopes: &[Scope]) -> Result<Option<StoredMeeting>> {
    let c = view.0;
    let Some((scope, captured_at)) = c
        .query_row(
            "SELECT scope, captured_at FROM meeting WHERE id = ?1",
            [meeting],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    if !scopes.is_empty() && !scopes.iter().any(|s| s.0 == scope) {
        return Ok(None);
    }
    let latest: i64 = c.query_row(
        "SELECT max(id) FROM meeting_run WHERE meeting_id = ?1",
        [meeting],
        |r| r.get(0),
    )?;
    let good: Option<i64> = c.query_row(
        "SELECT max(id) FROM meeting_run WHERE meeting_id = ?1 AND state = 'succeeded'",
        [meeting],
        |r| r.get(0),
    )?;
    let hypotheses = match good {
        None => Vec::new(),
        Some(good) => c
            .prepare(
                "SELECT responsible_label, responsible_entity, responsible_identifier,
                        recipient_label, recipient_entity, recipient_identifier,
                        promise, stated_date, turn, speaker, quote, quote_start, quote_end
                 FROM meeting_hypothesis WHERE run_id = ?1 ORDER BY ordinal",
            )?
            .query_map([good], |r| {
                Ok(Hypothesis {
                    status: "hypothesis".into(),
                    responsible: party(r.get(0)?, r.get(1)?, r.get(2)?),
                    recipient: party(r.get(3)?, r.get(4)?, r.get(5)?),
                    promise: r.get(6)?,
                    stated_date: match r.get::<_, Option<String>>(7)? {
                        Some(text) => StatedDate {
                            state: Known::Stated,
                            text: Some(text),
                        },
                        None => StatedDate {
                            state: Known::Unknown,
                            text: None,
                        },
                    },
                    quote: Quote {
                        turn: count(r, 8)?,
                        speaker: r.get(9)?,
                        text: r.get(10)?,
                        start: count(r, 11)?,
                        end: count(r, 12)?,
                    },
                })
            })?
            .collect::<Result<_, _>>()?,
    };
    Ok(Some(StoredMeeting {
        meeting: MeetingRef {
            id: meeting.into(),
            scope,
            captured_at: parse_time(captured_at)?,
        },
        latest: run_row(c, latest)?,
        good: good.map(|id| run_row(c, id)).transpose()?,
        hypotheses,
    }))
}

pub(crate) fn record(connection: &mut Connection, run: &RunRecord) -> Result<i64> {
    let transaction = connection.transaction()?;
    let id = &run.meeting.id;
    let existing: Option<String> = transaction
        .query_row("SELECT scope FROM meeting WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    match existing {
        Some(scope) if scope != run.meeting.scope => {
            bail!("meeting {id} already exists in another scope; import it in that scope")
        }
        Some(_) => {
            transaction.execute(
                "UPDATE meeting SET captured_at = ?2 WHERE id = ?1",
                params![id, stamp(run.meeting.captured_at)],
            )?;
        }
        None => {
            transaction.execute(
                "INSERT INTO meeting (id, scope, captured_at, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![
                    id,
                    run.meeting.scope,
                    stamp(run.meeting.captured_at),
                    stamp(run.finished_at)
                ],
            )?;
        }
    }
    transaction.execute(
        "INSERT INTO meeting_run (meeting_id, started_at, finished_at, state, cause, reason,
         extractor, owner_speaker, turns_total, turns_read, excluded)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            id,
            stamp(run.started_at),
            stamp(run.finished_at),
            run.state.as_str(),
            run.cause.map(Cause::as_str),
            run.reason,
            run.extractor,
            run.owner_speaker,
            run.turns_total as i64,
            run.turns_read as i64,
            run.excluded as i64
        ],
    )?;
    let run_id = transaction.last_insert_rowid();
    if run.state == RunState::Succeeded {
        // The new good run replaces the earlier ones' results.
        transaction.execute(
            "DELETE FROM meeting_hypothesis WHERE run_id IN
             (SELECT id FROM meeting_run WHERE meeting_id = ?1 AND id <> ?2)",
            params![id, run_id],
        )?;
        for (ordinal, h) in run.hypotheses.iter().enumerate() {
            let side = |p: &Party| {
                (
                    p.label.clone(),
                    p.identity.as_ref().map(|i| i.id.clone()),
                    p.identity.as_ref().map(|i| i.identifier.clone()),
                )
            };
            let (rl, re, ri) = side(&h.responsible);
            let (cl, ce, ci) = side(&h.recipient);
            transaction.execute(
                "INSERT INTO meeting_hypothesis (run_id, ordinal, responsible_label,
                 responsible_entity, responsible_identifier, recipient_label, recipient_entity,
                 recipient_identifier, promise, stated_date, turn, speaker, quote, quote_start,
                 quote_end) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    run_id,
                    ordinal as i64,
                    rl,
                    re,
                    ri,
                    cl,
                    ce,
                    ci,
                    h.promise,
                    h.stated_date.text,
                    h.quote.turn as i64,
                    h.quote.speaker,
                    h.quote.text,
                    h.quote.start as i64,
                    h.quote.end as i64
                ],
            )?;
        }
    }
    transaction.commit()?;
    Ok(run_id)
}
