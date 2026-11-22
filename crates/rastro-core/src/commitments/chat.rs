//! Chat commitments: an opted-in source's new threads, extracted after a sync (ADR 0011,
//! amendment 2026-11-22; decisions/surfaces.md §64).
//!
//! A thread is one conversation: its start message and the replies that name it, found in the
//! source's stored evidence at the message locations (`<namespace>:message/<channel>/<ts>`,
//! the replies' structured `/thread_ts` naming the start). A message with no replies is no
//! conversation. The core asks the source's bound extractor for the thread's turns over
//! `conversation.turns`, sends them through the bundled template to the configured agent tool
//! adapter, and records the run as a conversation of kind `chat`, named by the channel source
//! and the thread's start ts. A thread whose latest run succeeded is left alone; one whose
//! latest run failed is extracted again, and a failed run is recorded with its cause, so the
//! list names the gap and the last good results stay.
//!
//! The owner is the account their Person holds: the judgment names that account as the owner
//! speaker, and a party it names maps to the owner's identity. Every other speaker stays the
//! label the extractor resolved, unknown until the owner maps it.
use crate::{
    Evidence, Rastro, Source, Storage, TrustClass,
    commitments::{Identity, Kind, Known, PartyState, Quote, SourceKind, StatedDate},
    host::{self, JudgmentFailure},
    meeting::{
        self, Cause, Failure, Hypothesis, MeetingRef, Party, Recorded, RunRecord, RunState,
        SessionTurn, Span, TranscriptKeep,
    },
};
use anyhow::Result;
use rastro_plugin_sdk::{
    CandidateKind, ConversationSpeaker, ConversationTurn, ConversationTurnKind, JudgmentTurn,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One thread of a chat source: its start message's ts, and the latest evidence of its
/// messages — the start and its replies — in post order.
#[derive(Debug)]
struct Thread {
    ts: String,
    at: (i64, u32),
    messages: Vec<(i64, Evidence)>,
}

/// The message a location names: `<namespace>:message/<channel>/<ts>`, the ts epoch seconds
/// and a fraction of a second. Anything else names no message.
fn message_location(location: &str) -> Option<(&str, &str, &str, i64, u32)> {
    let (namespace, rest) = location.split_once(':')?;
    let path = rest.strip_prefix("message/")?;
    let (channel, ts) = path.rsplit_once('/')?;
    if namespace.is_empty() || channel.is_empty() {
        return None;
    }
    let (secs, micros) = parse_ts(ts)?;
    Some((namespace, channel, ts, secs, micros))
}

/// A message ts as epoch seconds and microseconds: digits, a point, digits.
fn parse_ts(ts: &str) -> Option<(i64, u32)> {
    let (secs, frac) = ts.split_once('.')?;
    if secs.is_empty() || !secs.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let secs: i64 = secs.parse().ok()?;
    let mut frac = frac.to_string();
    frac.truncate(6);
    while frac.len() < 6 {
        frac.push('0');
    }
    Some((secs, frac.parse().ok()?))
}

/// The source's threads, oldest first: a stored start message with at least one reply naming
/// it, each with the latest evidence of its messages in post order. Evidence arrives as the
/// latest observation of each location and field, so a re-read field reads as it stands now.
fn threads(evidence: &[(i64, Evidence)]) -> Vec<Thread> {
    let mut messages: BTreeMap<&str, Vec<&(i64, Evidence)>> = BTreeMap::new();
    let mut thread_of: BTreeMap<&str, &str> = BTreeMap::new();
    for pair in evidence {
        let e = &pair.1;
        if message_location(&e.location).is_none() {
            continue;
        }
        messages.entry(e.location.as_str()).or_default().push(pair);
        if e.field == "/thread_ts" && e.trust_class == TrustClass::Structured {
            thread_of.insert(e.location.as_str(), e.quote.as_str());
        }
    }
    // The replies group under the start ts they name; a message naming itself is no reply.
    let mut replies: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (location, ts) in &thread_of {
        let Some((_, _, own, ..)) = message_location(location) else {
            continue;
        };
        if own != *ts {
            replies.entry(ts).or_default().push(location);
        }
    }
    let mut found = Vec::new();
    for (ts, reply_locations) in replies {
        let Some((namespace, channel, _, secs, micros)) =
            reply_locations.first().and_then(|l| message_location(l))
        else {
            continue;
        };
        let start = format!("{namespace}:message/{channel}/{ts}");
        let Some(start_messages) = messages.get(start.as_str()) else {
            continue;
        };
        let mut items: Vec<(i64, Evidence)> = start_messages.iter().map(|p| (*p).clone()).collect();
        for location in reply_locations {
            if let Some(pairs) = messages.get(location) {
                items.extend(pairs.iter().map(|p| (*p).clone()));
            }
        }
        items.sort_by_key(|(id, e)| {
            let (_, _, _, secs, micros) =
                message_location(&e.location).expect("a thread's messages are message locations");
            (secs, micros, *id)
        });
        found.push(Thread {
            ts: ts.to_string(),
            at: (secs, micros),
            messages: items,
        });
    }
    found.sort_by_key(|thread| thread.at);
    found
}

/// The candidate shape the bundled template declares, as the core's mapping reads it.
#[derive(Clone, Deserialize, Serialize)]
struct Candidate {
    turn: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<CandidateKind>,
    #[serde(default)]
    responsible: Option<String>,
    #[serde(default)]
    recipient: Option<String>,
    promise: String,
    #[serde(default)]
    stated_date: Option<String>,
    quote: String,
}

/// What passed the runner's gates: the hypotheses, the proposals behind them, and the count
/// the declared exclusions matched.
struct Validated {
    hypotheses: Vec<Hypothesis>,
    proposed: Vec<Candidate>,
    excluded: usize,
}

/// Why a thread is skipped without a failed run, counted in its source's coverage.
/// The owner is in a thread when a turn's speaker or a mentioned account is held by the owner's
/// Person (decisions D13); a thread with neither is skipped, not failed.
const OWNER_ABSENT_REASON: &str =
    "owner absent: no speaker or mentioned account is held by the owner's Person";
/// A thread whose turns answer cannot be carried over the wire is skipped, not failed (decisions
/// D6). For chat the cap that counts is the host's line cap, not the meeting transcript cap: the
/// extractor answers `conversation.turns` in a single line.
const TOO_LARGE_REASON: &str = "too large: the thread's turns exceed the wire line cap";
/// The margin kept under the host's line cap for the turns answer's envelope — the JSON around the
/// turn text (speakers, kinds, times, citations, mentions). An answer that still overruns the cap
/// is refused as too long and skipped the same way, so the margin need not be exact.
const ANSWER_ENVELOPE_MARGIN: u64 = 64 * 1024;

/// The most stored message text a chat thread's turns answer can carry: the host's line cap less
/// the envelope margin. A thread above this is skipped before the extractor is asked.
fn turns_answer_cap() -> u64 {
    crate::host::DEFAULT_MAX_LINE as u64 - ANSWER_ENVELOPE_MARGIN
}
/// A turn the core cannot trace to a message of the thread is refused, and the thread skipped.
const TURN_REFUSED_REASON: &str = "a turn was refused: it does not cite a message of the thread";

/// Whether any turn cannot be traced to the thread: it cites no message, or a message outside
/// the thread. The core refuses such a turn rather than send it to the adapter (ADR 0011,
/// amendment 2026-11-22; decisions D11).
fn refuses_a_turn(turns: &[ConversationTurn], thread: &Thread) -> bool {
    let locations: BTreeSet<&str> = thread
        .messages
        .iter()
        .map(|(_, evidence)| evidence.location.as_str())
        .collect();
    turns.iter().any(|turn| {
        !turn
            .cites
            .as_deref()
            .is_some_and(|cite| locations.contains(cite))
    })
}

/// The thread's transcript size: the bytes of its messages' text, the untrusted content the
/// turns are made from. The core measures it from the evidence it already holds, before asking
/// the extractor, so an oversize thread is skipped without a turn answer one wire line could not
/// carry (the extractor's answer is a single line bounded by the host's line cap).
fn transcript_bytes(thread: &Thread) -> u64 {
    thread
        .messages
        .iter()
        .filter(|(_, evidence)| evidence.trust_class == TrustClass::UntrustedContent)
        .map(|(_, evidence)| evidence.quote.len() as u64)
        .sum()
}

/// How many messages the thread holds: one turn per message, so this is the turns a run would read.
fn message_count(thread: &Thread) -> usize {
    thread
        .messages
        .iter()
        .map(|(_, evidence)| evidence.location.as_str())
        .collect::<BTreeSet<_>>()
        .len()
}

/// How one thread's run ended: judged through the adapter, or skipped before it with a reason.
enum Outcome {
    Judged(Validated, Recorded),
    Skipped(String),
}

fn fail(cause: Cause, reason: impl Into<String>) -> Failure {
    Failure {
        cause,
        reason: reason.into(),
    }
}

/// The runner's failure as the run's typed cause, as a meeting import reads it.
fn task_failure(failure: host::task::TaskFailure) -> Failure {
    let cause = if let Some(gate) = failure.gate.as_deref() {
        if gate == "span" {
            Cause::InvalidQuote
        } else {
            Cause::ExtractorOutputInvalid
        }
    } else {
        match failure.system {
            Some(JudgmentFailure::Invalid(_)) => Cause::ExtractorOutputInvalid,
            _ => Cause::ExtractorFailed,
        }
    };
    Failure {
        cause,
        reason: failure.reason,
    }
}

/// The runner's turn history as the run's session history keeps it.
fn session_history(turns: &[host::task::TaskTurn]) -> Vec<SessionTurn> {
    turns
        .iter()
        .map(|turn| SessionTurn {
            turn: turn.turn as usize,
            request: turn.request.clone(),
            answer: turn.answer.clone(),
            verdict: turn.verdict.clone(),
        })
        .collect()
}

/// Maps the runner's validated candidates to hypotheses with their citations. A party the
/// thread's speakers identify — the owner's held account, or another author's account — carries
/// that identity; a label no account resolved stays unknown, shown by its label.
fn map_candidates(
    candidates: Vec<host::task::CheckedCandidate>,
    excluded: usize,
    turns: &[JudgmentTurn],
    parties: &BTreeMap<String, Identity>,
) -> Validated {
    let party = |label: Option<&str>| -> Party {
        let Some(label) = label.filter(|l| !l.trim().is_empty()) else {
            return Party {
                state: PartyState::Unknown,
                label: None,
                identity: None,
            };
        };
        match parties.get(label) {
            Some(identity) => Party {
                state: PartyState::Mapped,
                label: Some(label.into()),
                identity: Some(identity.clone()),
            },
            None => Party {
                state: PartyState::Unknown,
                label: Some(label.into()),
                identity: None,
            },
        }
    };
    let mut kept = Vec::new();
    let mut proposed = Vec::new();
    for checked in candidates {
        let candidate: Candidate = serde_json::from_value(checked.proposed)
            .expect("the runner kept only candidates in the template's declared shape");
        let turn = turns
            .get(candidate.turn.checked_sub(1).unwrap_or(usize::MAX))
            .expect("the span rule passed the candidate's turn");
        let start = turn
            .text
            .find(candidate.quote.as_str())
            .expect("the span rule passed the candidate's quote");
        let end = start + candidate.quote.len();
        proposed.push(candidate.clone());
        let stated_date = match candidate
            .stated_date
            .filter(|_| !checked.cleared_fields.iter().any(|f| f == "stated_date"))
        {
            Some(text) => StatedDate {
                state: Known::Stated,
                text: Some(text),
            },
            None => StatedDate {
                state: Known::Unknown,
                text: None,
            },
        };
        kept.push(Hypothesis {
            status: "hypothesis".into(),
            kind: candidate.kind.map_or(Kind::Promise, Kind::from),
            responsible: party(candidate.responsible.as_deref()),
            recipient: party(candidate.recipient.as_deref()),
            promise: candidate.promise,
            stated_date,
            quote: Quote {
                text: candidate.quote,
                turn: turn.turn as usize,
                speaker: turn.speaker.clone(),
                start,
                end,
            },
        });
    }
    Validated {
        hypotheses: kept,
        proposed,
        excluded,
    }
}

/// The record a successful run keeps for replay: the kept candidates, the turns they cite and
/// the speakers beside those turns, the account identities the run mapped, and the contract it
/// ran under.
fn record_of(
    validated: &Validated,
    turns: &[JudgmentTurn],
    parties: &BTreeMap<String, Identity>,
    contract: &str,
) -> Recorded {
    let cited: BTreeSet<usize> = validated.proposed.iter().map(|c| c.turn).collect();
    let mut wanted = cited.clone();
    for &turn in &cited {
        wanted.insert(turn.saturating_sub(1));
        wanted.insert(turn + 1);
    }
    let spans = wanted
        .into_iter()
        .filter_map(|turn| {
            let found = turn.checked_sub(1).and_then(|index| turns.get(index))?;
            Some(Span {
                turn,
                speaker: found.speaker.clone(),
                text: if cited.contains(&turn) {
                    found.text.clone()
                } else {
                    String::new()
                },
            })
        })
        .collect();
    let speakers = parties
        .iter()
        .map(|(label, identity)| (label.clone(), identity.clone()))
        .collect();
    Recorded {
        candidates: serde_json::to_string(&validated.proposed).expect("candidates serialise"),
        spans,
        speakers,
        contract: contract.to_string(),
    }
}

impl<S: Storage> Rastro<S> {
    /// Extracts the opted-in source's new threads: each thread whose latest run has not
    /// succeeded is asked of the bound extractor, judged through the bundled template, and
    /// recorded as a `chat` conversation named `<source>/<thread ts>`. A thread whose latest
    /// run succeeded is left alone; one whose latest run failed is extracted again, and a
    /// failed run keeps its cause, so the list names the gap and the last good results stay.
    /// Returns the threads this pass recorded, in extraction order. `Err` is a storage failure
    /// that stopped the pass; a single thread's failure never stops the next.
    pub fn extract_chat_threads(&mut self, source: &Source) -> Result<Vec<String>> {
        let Some(extractor) = host::find(&self.locations, &source.connector)
            .and_then(|(manifest, _)| manifest.default_extractor.clone())
        else {
            return Ok(Vec::new());
        };
        let evidence = self.storage.source_evidence(source)?;
        // The source's records beside the messages — authors, the viewer — ride with every
        // request, so the extractor can resolve speakers and mentions.
        let records: Vec<(i64, Evidence)> = evidence
            .iter()
            .filter(|(_, e)| message_location(&e.location).is_none())
            .cloned()
            .collect();
        let mut extracted = Vec::new();
        for thread in threads(&evidence) {
            let id = format!("{}/{}", source.name, thread.ts);
            let recorded = self.storage.snapshot(|view| view.meeting(&id, &[]))?;
            // A thread whose latest run succeeded is left alone; one whose latest run failed
            // is extracted again, so a failed first run never buries it.
            if recorded.is_some_and(|stored| stored.latest.state == RunState::Succeeded) {
                continue;
            }
            self.extract_thread(source, &extractor, &id, &thread, &records)?;
            extracted.push(id);
        }
        Ok(extracted)
    }

    /// One thread's run: the extractor's turns, the judgment session, and the run record.
    #[expect(
        clippy::too_many_lines,
        reason = "a thread's turns, judgment, mapping and run record share one borrow of storage and one place to fail"
    )]
    fn extract_thread(
        &mut self,
        source: &Source,
        extractor: &str,
        id: &str,
        thread: &Thread,
        records: &[(i64, Evidence)],
    ) -> Result<()> {
        let started_at = crate::coverage::now()?;
        let mut delivery: Vec<(i64, Evidence)> = records.to_vec();
        delivery.extend(thread.messages.iter().cloned());
        let mut turns_total = 0;
        let mut adapter = None;
        let mut owner_speaker = String::new();
        let mut history: Vec<SessionTurn> = Vec::new();
        let outcome = (|| -> Result<Outcome, Failure> {
            // An oversize thread is skipped before the extractor is asked: its turns answer is one
            // wire line, and a thread whose stored text already exceeds the line cap (less the
            // envelope margin) could not be carried. Measuring the stored transcript keeps the skip
            // a counted success on a real install, not a failed run retried every tick.
            if transcript_bytes(thread) > turns_answer_cap() {
                turns_total = message_count(thread);
                return Ok(Outcome::Skipped(TOO_LARGE_REASON.to_string()));
            }
            let turns = match host::extract::conversation_turns(
                &mut self.storage,
                &self.locations,
                source,
                extractor,
                &thread.ts,
                &delivery,
            ) {
                Ok(Ok(turns)) => turns,
                // The answer overran the line cap after all (a larger envelope than the margin
                // allows): the same counted too-large skip, never a failed run retried every tick.
                Ok(Err(reason)) if reason == "too_long" => {
                    turns_total = message_count(thread);
                    return Ok(Outcome::Skipped(TOO_LARGE_REASON.to_string()));
                }
                Ok(Err(reason)) => return Err(fail(Cause::ExtractorFailed, reason)),
                Err(error) => return Err(fail(Cause::StorageFailed, format!("{error:#}"))),
            };
            turns_total = turns.len();
            // The contract turns as the judgment wire carries them: an account speaker reads
            // as its identifier, a label as itself, counted from 1.
            let wire: Vec<JudgmentTurn> = turns
                .iter()
                .enumerate()
                .map(|(index, turn)| JudgmentTurn {
                    turn: (index + 1) as u64,
                    speaker: match &turn.speaker {
                        ConversationSpeaker::Label(label) => label.clone(),
                        ConversationSpeaker::Account(account) => account.identifier.clone(),
                    },
                    text: turn.text.clone(),
                })
                .collect();
            // The owner is the account their Person holds that speaks in this thread.
            let (accounts, owner) = self
                .storage
                .snapshot(|view| {
                    let about = view.about_view(
                        crate::identity::ME,
                        &crate::knowledge::AboutOptions::default(),
                        &[],
                    )?;
                    let resolution = crate::identity::Resolution::of(&view.intents()?);
                    let owner =
                        crate::identity::resolve(view, &resolution, crate::identity::ME, "person")
                            .ok()
                            .map(|target| Identity {
                                id: target.id,
                                identifier: target.identifier,
                            });
                    Ok((
                        about
                            .person
                            .map(|person| person.accounts)
                            .unwrap_or_default(),
                        owner,
                    ))
                })
                .map_err(|error| fail(Cause::StorageFailed, format!("{error:#}")))?;
            let speakers: BTreeSet<&str> = wire.iter().map(|t| t.speaker.as_str()).collect();
            owner_speaker = accounts
                .iter()
                .map(|account| account.identifier.as_str())
                .find(|identifier| speakers.contains(identifier))
                .unwrap_or_default()
                .to_string();
            // The parties the thread's speakers identify: an account the owner's Person holds is
            // the owner (`me`); every other account speaker is that account by id; a label no
            // account resolved stays unknown and is absent from this map.
            let held: BTreeSet<&str> = accounts
                .iter()
                .map(|account| account.identifier.as_str())
                .collect();
            let mut parties: BTreeMap<String, Identity> = BTreeMap::new();
            for turn in &turns {
                if let ConversationSpeaker::Account(account) = &turn.speaker {
                    let identity = if held.contains(account.identifier.as_str()) {
                        owner.clone()
                    } else {
                        Some(Identity {
                            id: crate::claims::entity_id(&account.identifier),
                            identifier: account.identifier.clone(),
                        })
                    };
                    if let Some(identity) = identity {
                        parties
                            .entry(account.identifier.clone())
                            .or_insert(identity);
                    }
                }
            }
            // A turn that cites outside the thread is refused: the whole thread is skipped
            // before the adapter is asked, and counted.
            if refuses_a_turn(&turns, thread) {
                return Ok(Outcome::Skipped(TURN_REFUSED_REASON.to_string()));
            }
            // The owner is in the thread when a speaker or a mentioned account is one the Person
            // holds. A thread with neither is skipped before the adapter is asked, and counted.
            let involved = turns.iter().any(|turn| {
                let speaker_held = match &turn.speaker {
                    ConversationSpeaker::Account(account) => {
                        held.contains(account.identifier.as_str())
                    }
                    ConversationSpeaker::Label(_) => false,
                };
                speaker_held
                    || turn
                        .mentions
                        .iter()
                        .any(|account| held.contains(account.identifier.as_str()))
            });
            if !involved {
                return Ok(Outcome::Skipped(OWNER_ABSENT_REASON.to_string()));
            }
            let Some(config) = self.config.commitment_adapter.clone() else {
                return Err(fail(
                    Cause::ExtractorUnconfigured,
                    "no agent tool adapter is configured; add a [commitment_adapter] table with the `plugin` id to config.toml",
                ));
            };
            adapter = Some(config.plugin.clone());
            let (template, contract) = meeting::bundled_contract();
            // A candidate whose party is a bot turn is excluded: the runner's bot rule reads the
            // speakers of the turns the extractor marked kind `bot`.
            let labels: BTreeSet<String> = turns
                .iter()
                .zip(&wire)
                .filter(|(turn, _)| turn.kind == ConversationTurnKind::Bot)
                .map(|(_, turn)| turn.speaker.clone())
                .collect();
            let input = host::task::TaskInput {
                owner_speaker: &owner_speaker,
                turns: &wire,
                labels: Some(&labels),
            };
            let judged = host::task::run(
                &self.locations,
                &config.plugin,
                &template,
                &input,
                config.max_turns(),
                config.turn_timeout(),
            );
            history = match &judged {
                Ok(output) => session_history(&output.history),
                Err(failure) => session_history(&failure.history),
            };
            let output = judged.map_err(task_failure)?;
            let validated = map_candidates(output.candidates, output.excluded, &wire, &parties);
            let recorded = record_of(&validated, &wire, &parties, &contract);
            Ok(Outcome::Judged(validated, recorded))
        })();
        let finished_at = crate::coverage::now()?;
        let captured_at = thread
            .messages
            .iter()
            .map(|(_, e)| e.captured_at)
            .max()
            .unwrap_or(started_at);
        let mut record = RunRecord {
            meeting: MeetingRef {
                id: id.into(),
                scope: source.scope.0.clone(),
                captured_at,
            },
            kind: SourceKind::Chat,
            started_at,
            finished_at,
            state: RunState::Failed,
            cause: None,
            reason: None,
            extractor: adapter,
            owner_speaker: Some(owner_speaker),
            turns_total,
            turns_read: 0,
            excluded: 0,
            hypotheses: Vec::new(),
            replay_of: None,
            recorded: None,
            transcript: TranscriptKeep::Untouched,
            turns: history,
        };
        match outcome {
            Ok(Outcome::Judged(validated, recorded)) => {
                record.state = RunState::Succeeded;
                record.turns_read = turns_total;
                record.excluded = validated.excluded;
                record.hypotheses = validated.hypotheses;
                record.recorded = Some(recorded);
            }
            Ok(Outcome::Skipped(reason)) => {
                // A skip is no failed run: the thread is counted in coverage with its reason,
                // and a later tick leaves it alone until its messages change.
                record.state = RunState::Succeeded;
                record.reason = Some(reason);
            }
            Err(failure) => {
                record.cause = Some(failure.cause);
                record.reason = Some(failure.reason);
            }
        }
        self.storage.record_meeting_run(&record)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scope;

    fn message(id: i64, channel: &str, ts: &str, field: &str, quote: &str) -> (i64, Evidence) {
        (
            id,
            Evidence {
                source: "eng".into(),
                scope: Scope("work".into()),
                location: format!("slack:message/{channel}/{ts}"),
                field: field.into(),
                start: 0,
                end: quote.len() as u64,
                quote: quote.into(),
                trust_class: if field == "/text" {
                    TrustClass::UntrustedContent
                } else {
                    TrustClass::Structured
                },
                captured_at: "2026-11-20T10:00:00Z".parse().unwrap(),
                source_updated_at: None,
                original: None,
            },
        )
    }

    /// A thread is a stored start message with at least one reply; a message with no replies
    /// and a reply whose start was never stored are no thread (decisions D2).
    #[test]
    fn a_thread_is_a_stored_start_message_with_at_least_one_reply() {
        let evidence = vec![
            message(1, "C1", "100.000100", "/text", "anyone around?"),
            message(2, "C1", "200.000100", "/text", "can you send the doc?"),
            message(3, "C1", "200.000100", "/user", "U0ANA"),
            message(4, "C1", "201.000200", "/text", "I'll send it Friday."),
            message(5, "C1", "201.000200", "/thread_ts", "200.000100"),
            // A reply whose start message was never stored is no thread of this source.
            message(6, "C1", "202.000300", "/thread_ts", "999.000900"),
        ];
        let found = threads(&evidence);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].ts, "200.000100");
        let locations: Vec<&str> = found[0]
            .messages
            .iter()
            .map(|(_, e)| e.location.as_str())
            .collect();
        assert_eq!(
            locations,
            [
                "slack:message/C1/200.000100",
                "slack:message/C1/200.000100",
                "slack:message/C1/201.000200",
                "slack:message/C1/201.000200",
            ]
        );
    }
}
