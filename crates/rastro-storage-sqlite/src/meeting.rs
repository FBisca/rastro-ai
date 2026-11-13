//! Meetings, their runs and their hypotheses (ADR 0011).
use crate::{View, parse_time, stamp};
use anyhow::{Result, bail};
use rastro_core::Scope;
use rastro_core::meeting::{
    Cause, EXPORT_FORMAT, EXPORT_NOTICE, ExportedSpeaker, Hypothesis, Identity, Known,
    MeetingExport, MeetingRef, Party, PartyState, Quote, Recording, Removed, RunRecord, RunRow,
    RunState, Span, StatedDate, StoredMeeting, TranscriptKeep,
};
use rusqlite::{Connection, OptionalExtension, params};

/// A count column, read as the unsigned number it is.
fn count(row: &rusqlite::Row, index: usize) -> rusqlite::Result<usize> {
    Ok(row.get::<_, i64>(index)?.max(0) as usize)
}

fn run_row(connection: &Connection, id: i64) -> Result<RunRow> {
    let row = connection.query_row(
        "SELECT finished_at, state, cause, reason, extractor, turns_total, turns_read, excluded,
                replay_of
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
                r.get::<_, Option<i64>>(8)?,
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
        replay_of: row.8,
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
    // A replay checks an older record, so the good results are as old as the import they trace
    // back to; they are stale when a later import ran, or when the latest run is not the good one.
    let origin: Option<i64> = match good {
        None => None,
        Some(good) => c.query_row(
            "WITH RECURSIVE chain (id, replay_of) AS (
                 SELECT id, replay_of FROM meeting_run WHERE id = ?1
                 UNION ALL
                 SELECT r.id, r.replay_of FROM meeting_run r JOIN chain ON r.id = chain.replay_of)
             SELECT id FROM chain WHERE replay_of IS NULL",
            [good],
            |r| r.get(0),
        )?,
    };
    let latest_import: Option<i64> = c.query_row(
        "SELECT max(id) FROM meeting_run WHERE meeting_id = ?1 AND replay_of IS NULL",
        [meeting],
        |r| r.get(0),
    )?;
    let stale = match (good, origin, latest_import) {
        (Some(good), Some(origin), Some(import)) => good != latest || import > origin,
        (Some(good), _, _) => good != latest,
        _ => false,
    };
    Ok(Some(StoredMeeting {
        meeting: MeetingRef {
            id: meeting.into(),
            scope,
            captured_at: parse_time(captured_at)?,
        },
        latest: run_row(c, latest)?,
        good: good.map(|id| run_row(c, id)).transpose()?,
        origin: origin.map(|id| run_row(c, id)).transpose()?,
        stale,
        hypotheses,
        transcript_retained: c.query_row(
            "SELECT EXISTS (SELECT 1 FROM meeting_transcript WHERE meeting_id = ?1)",
            [meeting],
            |r| r.get(0),
        )?,
    }))
}

fn speakers_of(c: &Connection, run: i64) -> Result<Vec<(String, Identity)>> {
    Ok(c.prepare(
        "SELECT label, entity, identifier FROM meeting_speaker WHERE run_id = ?1 ORDER BY label",
    )?
    .query_map([run], |r| {
        Ok((
            r.get::<_, String>(0)?,
            Identity {
                id: r.get(1)?,
                identifier: r.get(2)?,
            },
        ))
    })?
    .collect::<Result<_, _>>()?)
}

/// The latest good run's record, for replay.
pub(crate) fn recording(view: &View, meeting: &str) -> Result<Option<Recording>> {
    let c = view.0;
    let Some((run, extractor, owner, total, excluded, scope, captured, candidates)) = c
        .query_row(
            "SELECT r.id, r.extractor, r.owner_speaker, r.turns_total, r.excluded, m.scope,
                    m.captured_at, o.candidates
             FROM meeting_run r JOIN meeting m ON m.id = r.meeting_id
             JOIN meeting_output o ON o.run_id = r.id
             WHERE r.meeting_id = ?1 AND r.state = 'succeeded'
             ORDER BY r.id DESC LIMIT 1",
            [meeting],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    count(r, 3)?,
                    count(r, 4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };
    let Some(owner_speaker) = owner else {
        return Ok(None);
    };
    let spans = c
        .prepare("SELECT turn, speaker, text FROM meeting_span WHERE run_id = ?1 ORDER BY turn")?
        .query_map([run], |r| {
            Ok(Span {
                turn: count(r, 0)?,
                speaker: r.get(1)?,
                text: r.get(2)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(Some(Recording {
        meeting: MeetingRef {
            id: meeting.into(),
            scope,
            captured_at: parse_time(captured)?,
        },
        run,
        extractor,
        owner_speaker,
        turns_total: total,
        excluded,
        candidates,
        spans,
        speakers: speakers_of(c, run)?,
    }))
}

/// The meeting as the owner supplied it: no hypothesis, recorded output or span.
pub(crate) fn export(view: &View, meeting: &str) -> Result<Option<MeetingExport>> {
    let c = view.0;
    let Some((scope, captured)) = c
        .query_row(
            "SELECT scope, captured_at FROM meeting WHERE id = ?1",
            [meeting],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let owner: Option<String> = c
        .query_row(
            "SELECT owner_speaker FROM meeting_run
             WHERE meeting_id = ?1 AND owner_speaker IS NOT NULL ORDER BY id DESC LIMIT 1",
            [meeting],
            |r| r.get(0),
        )
        .optional()?;
    let good: Option<i64> = c.query_row(
        "SELECT max(id) FROM meeting_run WHERE meeting_id = ?1 AND state = 'succeeded'",
        [meeting],
        |r| r.get(0),
    )?;
    let speakers = match good {
        Some(run) => speakers_of(c, run)?
            .into_iter()
            .filter(|(label, _)| Some(label) != owner.as_ref())
            .map(|(label, identity)| ExportedSpeaker { label, identity })
            .collect(),
        None => Vec::new(),
    };
    let transcript: Option<String> = c
        .query_row(
            "SELECT text FROM meeting_transcript WHERE meeting_id = ?1",
            [meeting],
            |r| r.get(0),
        )
        .optional()?;
    Ok(Some(MeetingExport {
        format: EXPORT_FORMAT.into(),
        notice: EXPORT_NOTICE.into(),
        meeting: MeetingRef {
            id: meeting.into(),
            scope,
            captured_at: parse_time(captured)?,
        },
        owner_speaker: owner,
        speakers,
        transcript,
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
         extractor, owner_speaker, turns_total, turns_read, excluded, replay_of)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
            run.excluded as i64,
            run.replay_of
        ],
    )?;
    let run_id = transaction.last_insert_rowid();
    if run.state == RunState::Succeeded {
        // The new good run replaces the earlier ones' results.
        for table in [
            "meeting_hypothesis",
            "meeting_output",
            "meeting_span",
            "meeting_speaker",
        ] {
            transaction.execute(
                &format!(
                    "DELETE FROM {table} WHERE run_id IN
                     (SELECT id FROM meeting_run WHERE meeting_id = ?1 AND id <> ?2)"
                ),
                params![id, run_id],
            )?;
        }
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
    if let Some(recorded) = &run.recorded {
        transaction.execute(
            "INSERT INTO meeting_output (run_id, candidates) VALUES (?1, ?2)",
            params![run_id, recorded.candidates],
        )?;
        for span in &recorded.spans {
            transaction.execute(
                "INSERT INTO meeting_span (run_id, turn, speaker, text) VALUES (?1, ?2, ?3, ?4)",
                params![run_id, span.turn as i64, span.speaker, span.text],
            )?;
        }
        for (label, identity) in &recorded.speakers {
            transaction.execute(
                "INSERT INTO meeting_speaker (run_id, label, entity, identifier)
                 VALUES (?1, ?2, ?3, ?4)",
                params![run_id, label, identity.id, identity.identifier],
            )?;
        }
    }
    match &run.transcript {
        TranscriptKeep::Untouched => {}
        TranscriptKeep::Drop => {
            transaction.execute("DELETE FROM meeting_transcript WHERE meeting_id = ?1", [id])?;
        }
        TranscriptKeep::Store(text) => {
            transaction.execute(
                "INSERT INTO meeting_transcript (meeting_id, run_id, text) VALUES (?1, ?2, ?3)
                 ON CONFLICT (meeting_id) DO UPDATE SET run_id = ?2, text = ?3",
                params![id, run_id, text],
            )?;
        }
    }
    transaction.commit()?;
    Ok(run_id)
}

/// Deletes a meeting and all that is kept for it, then truncates the write-ahead log so no copy
/// of the forgotten text stays behind.
pub(crate) fn forget(connection: &mut Connection, meeting: &str) -> Result<Option<Removed>> {
    let transaction = connection.transaction()?;
    let known: bool = transaction.query_row(
        "SELECT EXISTS (SELECT 1 FROM meeting WHERE id = ?1)",
        [meeting],
        |r| r.get(0),
    )?;
    if !known {
        return Ok(None);
    }
    let of_runs = "run_id IN (SELECT id FROM meeting_run WHERE meeting_id = ?1)";
    let delete = |table: &str, filter: &str| -> Result<usize> {
        Ok(transaction.execute(&format!("DELETE FROM {table} WHERE {filter}"), [meeting])?)
    };
    let hypotheses = delete("meeting_hypothesis", of_runs)?;
    let recorded_output = delete("meeting_output", of_runs)?;
    let spans = delete("meeting_span", of_runs)?;
    delete("meeting_speaker", of_runs)?;
    let transcript = delete("meeting_transcript", "meeting_id = ?1")? > 0;
    let runs = delete("meeting_run", "meeting_id = ?1")?;
    delete("meeting", "id = ?1")?;
    transaction.commit()?;
    // A busy checkpoint (another reader holds the log) only means it is cleaned up later.
    let _ = connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    Ok(Some(Removed {
        runs,
        hypotheses,
        spans,
        recorded_output,
        transcript,
    }))
}
