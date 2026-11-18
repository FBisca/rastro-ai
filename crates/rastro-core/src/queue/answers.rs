//! The queue's answers: accept, edit, reject, answer and missed.
//!
//! Each answer is recorded as exactly one owner intent, and an answer naming an item the queue
//! no longer holds is refused, naming the item and recording nothing. An accepted, edited or
//! rejected proposal leaves the queue by that record; an answered question leaves because the
//! identity graph no longer holds its pair as a candidate; a missed report leaves its item
//! waiting. A rejection mints no claim; an accepted or edited proposal becomes one observed
//! commitment, its standard terms justified by that intent alone.
use super::{Item, read};
use crate::identity::{
    self, AnswerPerson, EntityTarget, Intent, IntentKind, Payload, Resolution, StandardTerm,
    StatementSnapshot, Targets, plan_answer, resolve,
};
use crate::read_only::Snapshot;
use crate::{Rastro, Storage, commitments};
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

/// The entity an accepted proposal's statements are about: the party the item maps to the
/// owner's Person, so `about me` shows what the owner accepted, and otherwise the first party
/// the owner mapped. A proposal whose parties name no Person can't become a claim.
fn subject(view: &dyn Snapshot, item: &Item) -> Result<EntityTarget> {
    proposal(item)?;
    let hypothesis = item.proposal.as_ref().expect("a proposal item carries one");
    let person = |side: &commitments::Party| {
        side.identity.as_ref().map(|identity| EntityTarget {
            id: identity.id.clone(),
            identifier: identity.identifier.clone(),
            role: "person".into(),
        })
    };
    let me = Resolution::of(&view.intents()?).me;
    let parties = [
        person(&hypothesis.responsible),
        person(&hypothesis.recipient),
    ];
    (parties.iter().flatten())
        .find(|party| me.as_ref() == Some(&party.id))
        .or_else(|| parties.iter().flatten().next())
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "the queue can't accept {}: it names no party the owner mapped to a person",
                item.id
            )
        })
}

/// The standard terms an accepted proposal states, as the owner accepted them: the item's
/// statements, with the stated date replaced by the owner's edit when they gave one. The
/// standard vocabulary has no predicate for the promise, so an edited promise stays in the
/// intent's snapshots rather than on a claim.
fn accepted_terms(item: &Item, date: Option<&str>) -> Result<Vec<StandardTerm>> {
    proposal(item)?;
    let hypothesis = item.proposal.as_ref().expect("a proposal item carries one");
    let mut terms: Vec<StandardTerm> = (super::statements(hypothesis))
        .into_iter()
        .map(|statement| StandardTerm {
            predicate: statement.predicate,
            value: statement.value,
        })
        .collect();
    if let Some(date) = date {
        match (terms.iter_mut()).find(|term| term.predicate == commitments::DUE) {
            Some(term) => term.value = date.into(),
            None => terms.push(StandardTerm {
                predicate: commitments::DUE.into(),
                value: date.into(),
            }),
        }
    }
    Ok(terms)
}

/// What an answer about a proposal records about the item it names: the statements the owner
/// saw and made, the commitment an accept or an edit mints, and the item's source scope.
#[derive(Default)]
struct ProposalAnswer {
    seen: Option<StatementSnapshot>,
    edited: Option<StatementSnapshot>,
    subject: Option<EntityTarget>,
    terms: Vec<StandardTerm>,
    scope: Option<String>,
}

/// The intent an answer about a proposal records.
fn proposal_intent(
    kind: IntentKind,
    item: Option<String>,
    text: Option<String>,
    answer: ProposalAnswer,
) -> Result<Intent> {
    let targets = Targets::Proposal {
        version: identity::SHAPE_VERSION,
        item,
        text,
        seen: answer.seen,
        edited: answer.edited,
        subject: answer.subject,
        terms: answer.terms,
        scope: answer.scope,
    };
    identity::new_intent(kind, String::new(), targets, Payload::empty(), None)
}

impl<S: Storage> Rastro<S> {
    /// Accepts a proposal unchanged: one intent, snapshotting what the owner saw and the
    /// commitment it becomes.
    pub fn queue_accept(&mut self, item: &str) -> Result<Answered> {
        let id = item.to_string();
        let issued = self.issue(|view, _| {
            let pending = still_queued(view, &id)?;
            let intent = proposal_intent(
                IntentKind::Accept,
                Some(pending.id.clone()),
                None,
                ProposalAnswer {
                    seen: Some(statement(&pending)?),
                    subject: Some(subject(view, &pending)?),
                    terms: accepted_terms(&pending, None)?,
                    scope: Some(pending.source.scope.clone()),
                    ..ProposalAnswer::default()
                },
            )?;
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
                Some(pending.id.clone()),
                None,
                ProposalAnswer {
                    seen: Some(seen),
                    edited: Some(edited),
                    subject: Some(subject(view, &pending)?),
                    terms: accepted_terms(&pending, date.as_deref())?,
                    scope: Some(pending.source.scope.clone()),
                },
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
                Some(pending.id.clone()),
                Some(reason.clone()),
                ProposalAnswer {
                    scope: Some(pending.source.scope.clone()),
                    ..ProposalAnswer::default()
                },
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
                Some(id) => Some(still_queued(view, id)?),
                None => None,
            };
            let scope = (named.as_ref()).map(|item| item.source.scope.clone());
            let named = (named.as_ref()).map(|item| item.id.clone());
            let intent = proposal_intent(
                IntentKind::Missed,
                named,
                Some(text.clone()),
                ProposalAnswer {
                    scope,
                    ..ProposalAnswer::default()
                },
            )?;
            Ok((vec![intent], None, false))
        })?;
        Ok(Answered {
            item: id,
            intents: issued.intents,
        })
    }

    /// Answers a question yes or no, naming the item. One intent records the whole answer: the
    /// verdict, and for a yes the Person that holds the accounts and, when this answer creates
    /// it, the name it gives. The identity graph's own planner decides who holds whom, so a yes
    /// follows `rastro identity answer`'s rules; what differs is that the decision is one row.
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
            let (planned, person, created) = plan_answer(
                a.clone(),
                b.clone(),
                &view.identity_accounts()?,
                resolution,
                yes,
                name.as_deref(),
            )?;
            // The name a yes gives the Person it creates is the one its plan chose, and the
            // accounts it links are the ones its plan links: recorded here, so a replay never
            // re-decides them from what later intents made of those accounts.
            let named = (planned.iter())
                .find(|intent| intent.kind == IntentKind::Name)
                .and_then(|intent| intent.payload.name.clone());
            let linked: Vec<identity::EntityTarget> = (planned.iter())
                .filter(|intent| intent.kind == IntentKind::Link)
                .filter_map(|intent| {
                    intent
                        .targets
                        .endpoints()
                        .map(|[account, _]| account.clone())
                })
                .collect();
            let targets = Targets::Answer {
                version: identity::SHAPE_VERSION,
                item: pending.id.clone(),
                endpoints: [a, b],
                yes,
                person: person.map(|entity| AnswerPerson {
                    entity,
                    name: named,
                    created,
                }),
                linked,
            };
            let intent = identity::new_intent(
                IntentKind::Answer,
                String::new(),
                targets,
                Payload::empty(),
                None,
            )?;
            Ok((vec![intent], None, false))
        })?;
        Ok(Answered {
            item: Some(id),
            intents: issued.intents,
        })
    }
}
