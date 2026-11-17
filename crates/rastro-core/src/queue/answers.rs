//! The queue's answers: accept, edit, reject, answer and missed.
//!
//! Each answer is recorded as exactly one owner intent, and an answer naming an item the queue
//! no longer holds is refused, naming the item and recording nothing. An accepted, edited or
//! rejected proposal leaves the queue by that record; an answered question leaves because the
//! identity graph no longer holds its pair as a candidate; a missed report leaves its item
//! waiting. A rejection mints no claim.
use super::{Item, read};
use crate::identity::{
    self, Intent, IntentKind, Payload, StatementSnapshot, Targets, plan_answer, resolve,
};
use crate::read_only::Snapshot;
use crate::{Rastro, Storage};
use anyhow::{Result, bail, ensure};
use std::collections::BTreeSet;

/// What an answer recorded: the item it named and the owner intents it accepted.
#[derive(Clone, Debug)]
pub struct Answered {
    /// The item's id as the queue read gave it; none for a missed report about nothing queued.
    pub item: Option<String>,
    /// The intents the answer accepted, in acceptance order.
    pub intents: Vec<Intent>,
}

/// The item ids an answer has taken out of the queue: an accepted, edited or rejected proposal.
/// An answered question leaves because the identity graph no longer holds its pair as a
/// candidate, and a missed report leaves its item waiting.
pub fn answered_items(intents: &[Intent]) -> BTreeSet<String> {
    (intents.iter())
        .filter(|intent| {
            matches!(
                intent.kind,
                IntentKind::Accept | IntentKind::Edit | IntentKind::Reject
            )
        })
        .filter_map(|intent| intent.queue_item().map(String::from))
        .collect()
}

/// The item an answer names, which must still wait in the queue.
fn still_queued(view: &dyn Snapshot, item: &str) -> Result<Item> {
    (read(view, &[])?.items)
        .into_iter()
        .find(|pending| pending.id == item)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "the queue no longer holds {item}: it was answered, or the source replaced it; \
                 `rastro queue` lists what is waiting"
            )
        })
}

/// Refuses a question where a proposal was expected.
fn proposal(item: &Item) -> Result<()> {
    if item.proposal.is_none() {
        bail!(
            "{} is a question; answer it with `rastro queue answer`",
            item.id
        )
    }
    Ok(())
}

/// The proposal's statement, as the owner saw it.
fn statement(item: &Item) -> Result<StatementSnapshot> {
    proposal(item)?;
    let hypothesis = item.proposal.as_ref().expect("a proposal item carries one");
    Ok(StatementSnapshot {
        promise: hypothesis.promise.clone(),
        stated_date: hypothesis.stated_date.text.clone(),
    })
}

/// The intent an answer about a proposal records.
fn proposal_intent(
    kind: IntentKind,
    item: Option<String>,
    text: Option<String>,
    seen: Option<StatementSnapshot>,
    edited: Option<StatementSnapshot>,
) -> Result<Intent> {
    let targets = Targets::Proposal {
        version: identity::SHAPE_VERSION,
        item,
        text,
        seen,
        edited,
    };
    identity::new_intent(kind, String::new(), targets, Payload::empty(), None)
}

impl<S: Storage> Rastro<S> {
    /// Accepts a proposal unchanged: one intent, snapshotting what the owner saw.
    pub fn queue_accept(&mut self, item: &str) -> Result<Answered> {
        let id = item.to_string();
        let issued = self.issue(|view, _| {
            let pending = still_queued(view, &id)?;
            let seen = statement(&pending)?;
            let intent =
                proposal_intent(IntentKind::Accept, Some(pending.id), None, Some(seen), None)?;
            Ok((vec![intent], None, false))
        })?;
        Ok(Answered {
            item: Some(id),
            intents: issued.intents,
        })
    }

    /// Accepts a proposal with the owner's edits. `promise` and `date` replace what the item
    /// says; each keeps the item's own when it is none. At least one must be given, or there is
    /// nothing to edit.
    pub fn queue_edit(
        &mut self,
        item: &str,
        promise: Option<&str>,
        date: Option<&str>,
    ) -> Result<Answered> {
        ensure!(
            promise.is_some() || date.is_some(),
            "an edit changes the promise, the stated date, or both; to accept the item unchanged, \
             accept it"
        );
        let id = item.to_string();
        let (promise, date) = (promise.map(String::from), date.map(String::from));
        let issued = self.issue(|view, _| {
            let pending = still_queued(view, &id)?;
            let seen = statement(&pending)?;
            let edited = StatementSnapshot {
                promise: promise.clone().unwrap_or_else(|| seen.promise.clone()),
                stated_date: date.clone().or_else(|| seen.stated_date.clone()),
            };
            let intent = proposal_intent(
                IntentKind::Edit,
                Some(pending.id),
                None,
                Some(seen),
                Some(edited),
            )?;
            Ok((vec![intent], None, false))
        })?;
        Ok(Answered {
            item: Some(id),
            intents: issued.intents,
        })
    }

    /// Rejects a proposal with the owner's reason: one intent, and no claim.
    pub fn queue_reject(&mut self, item: &str, reason: &str) -> Result<Answered> {
        let reason = reason.trim();
        ensure!(!reason.is_empty(), "a rejection gives a reason");
        let id = item.to_string();
        let reason = reason.to_string();
        let issued = self.issue(|view, _| {
            let pending = still_queued(view, &id)?;
            proposal(&pending)?;
            let intent = proposal_intent(
                IntentKind::Reject,
                Some(pending.id),
                Some(reason.clone()),
                None,
                None,
            )?;
            Ok((vec![intent], None, false))
        })?;
        Ok(Answered {
            item: Some(id),
            intents: issued.intents,
        })
    }

    /// Reports something Rastro should have caught, in the owner's words, about a queue item
    /// when one is named. The item, if any, stays pending.
    pub fn queue_missed(&mut self, text: &str, item: Option<&str>) -> Result<Answered> {
        let text = text.trim();
        ensure!(
            !text.is_empty(),
            "a missed report says what Rastro should have caught"
        );
        let id = item.map(String::from);
        let text = text.to_string();
        let issued = self.issue(|view, _| {
            let named = match &id {
                Some(id) => Some(still_queued(view, id)?.id),
                None => None,
            };
            let intent =
                proposal_intent(IntentKind::Missed, named, Some(text.clone()), None, None)?;
            Ok((vec![intent], None, false))
        })?;
        Ok(Answered {
            item: id,
            intents: issued.intents,
        })
    }

    /// Answers a question yes or no, naming the item: a no records one cannot-link, a yes the
    /// links that hold its accounts, as `rastro identity answer` records them. Each link and the
    /// cannot-link also records the item it settles.
    pub fn queue_answer(&mut self, item: &str, yes: bool, name: Option<&str>) -> Result<Answered> {
        let id = item.to_string();
        let name = name.map(String::from);
        let issued = self.issue(|view, resolution| {
            let pending = still_queued(view, &id)?;
            let Some(question) = &pending.question else {
                bail!(
                    "{} is a proposal; accept, edit or reject it, or file it as missed",
                    pending.id
                );
            };
            let a = resolve(
                view,
                resolution,
                &question.accounts[0].identifier,
                "account",
            )?;
            let b = resolve(
                view,
                resolution,
                &question.accounts[1].identifier,
                "account",
            )?;
            let (intents, person, created) = plan_answer(
                a,
                b,
                &view.identity_accounts()?,
                resolution,
                yes,
                name.as_deref(),
            )?;
            let mut recorded = Vec::with_capacity(intents.len());
            for mut intent in intents {
                // A yesses's minted Person and its name are the answer's consequence; the link
                // and the cannot-link are what records which item was answered.
                if matches!(intent.kind, IntentKind::Link | IntentKind::CannotLink)
                    && let Some(endpoints) = intent.targets.endpoints().cloned()
                {
                    intent.targets = Targets::Question {
                        version: identity::SHAPE_VERSION,
                        item: pending.id.clone(),
                        endpoints,
                    };
                    intent.check()?;
                }
                recorded.push(intent);
            }
            Ok((recorded, person, created))
        })?;
        Ok(Answered {
            item: Some(id),
            intents: issued.intents,
        })
    }
}
