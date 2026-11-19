//! The queue's answers: accept, edit, reject, answer and missed.
//!
//! Each answer is recorded as exactly one owner intent, and an answer naming an item the queue
//! no longer holds is refused, naming the item and recording nothing. An accepted, edited or
//! rejected proposal leaves the queue by that record; an answered question leaves because the
//! identity graph no longer holds its pair as a candidate; a missed report leaves its item
//! waiting. A rejection mints no claim; an accepted or edited proposal becomes one observed
//! commitment, its standard terms justified by that intent alone. A yes to a question about a
//! lapsed retraction or a stale confirmation records the earlier correction again, bound to the
//! lifecycle state the claim has now.
use super::{CorrectionKind, CorrectionQuestion, Item, read};
use crate::actions::{self, NewRun, Run, RunStatus};
use crate::identity::{
    self, AnswerPerson, ClaimStatement, ClaimTarget, EntityTarget, Intent, IntentKind,
    IssuerContext, Payload, StandardTerm, StatementSnapshot, Targets, commitment_target,
    plan_answer, resolve,
};
use crate::read_only::Snapshot;
use crate::{Rastro, Storage, commitments};
use anyhow::{Context, Result, bail, ensure};
use std::collections::BTreeSet;

/// What an answer recorded: the item it named and the owner intents it accepted.
#[derive(Clone, Debug)]
pub struct Answered {
    /// The item's id as the queue read gave it; none for a missed report about nothing queued.
    pub item: Option<String>,
    /// The intents the answer accepted, in acceptance order.
    pub intents: Vec<Intent>,
    /// The run an accepted action proposal started, as it stands after the provider answered.
    pub run: Option<Run>,
}

/// The owner's changes to an action proposal's fields; each keeps the proposal's own when none.
#[derive(Clone, Debug, Default)]
pub struct ActionEdits {
    pub title: Option<String>,
    pub description: Option<String>,
    pub team: Option<String>,
    pub assignee: Option<String>,
}

impl ActionEdits {
    /// Whether the owner changed nothing.
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.description.is_none()
            && self.team.is_none()
            && self.assignee.is_none()
    }

    /// The fields as the owner leaves them: a change replaces, and a title, team or assignee
    /// can't be blank.
    fn apply(&self, mut fields: actions::IssueFields) -> Result<actions::IssueFields> {
        for (name, value) in [
            ("title", &self.title),
            ("team", &self.team),
            ("assignee", &self.assignee),
        ] {
            ensure!(
                value.as_ref().is_none_or(|value| !value.trim().is_empty()),
                "an edit can't leave the {name} blank"
            );
        }
        fields.title = self.title.clone().unwrap_or(fields.title);
        fields.team = self.team.clone().unwrap_or(fields.team);
        fields.assignee = self.assignee.clone().unwrap_or(fields.assignee);
        if let Some(description) = &self.description {
            fields.description = Some(description.clone()).filter(|text| !text.trim().is_empty());
        }
        Ok(fields)
    }
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

/// Refuses a question, and an action proposal, where a commitment proposal was expected.
fn proposal(item: &Item) -> Result<()> {
    if item.action.is_some() {
        bail!(
            "{} is an action proposal: change it with --title, --description, --team or \
             --assignee, or accept or reject it",
            item.id
        )
    }
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

/// The commitment an accepted proposal becomes and the Persons the item maps to parties: one
/// `std:Commitment` of its own, named by the item's id, so each acceptance keeps its own
/// statement set, and its parties carry it into their view, so `about me` shows what the owner
/// accepted. A proposal whose parties name no Person can't become a claim.
fn commitment(item: &Item) -> Result<(EntityTarget, Vec<EntityTarget>)> {
    proposal(item)?;
    let hypothesis = item.proposal.as_ref().expect("a proposal item carries one");
    let parties: Vec<EntityTarget> = (hypothesis.responsible.identity.iter())
        .chain(hypothesis.recipient.identity.iter())
        .map(|identity| EntityTarget {
            id: identity.id.clone(),
            identifier: identity.identifier.clone(),
            role: "person".into(),
        })
        .collect();
    ensure!(
        !parties.is_empty(),
        "the queue can't accept {}: it names no party the owner mapped to a person",
        item.id
    );
    Ok((commitment_target(&item.id), parties))
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

/// The intent a yes to a correction question records: the earlier correction again, as the owner
/// reads the claim now, so the claim's lifecycle state is the one it binds to and the question
/// is no longer asked. Unlike `claim reject` and `claim confirm` it records one even when the
/// claim already reads as the correction says: a stale confirmation still reads `confirmed`.
fn repeat_correction(view: &dyn Snapshot, correction: &CorrectionQuestion) -> Result<Intent> {
    let current = &correction.claim.claim;
    let state = view
        .claim_lifecycle(&current.id)?
        .ok_or_else(|| anyhow::anyhow!("unknown claim: {}", current.id))?;
    let statement = ClaimStatement {
        subject: EntityTarget {
            id: current.subject.id.clone(),
            identifier: current.subject.identifier.clone(),
            role: "subject".into(),
        },
        predicate: current.predicate.clone(),
        object: current.object.clone(),
        scalar: state.scalar,
    };
    let review = ClaimTarget::review(current.revision.clone(), &statement, state.fingerprint)?;
    let kind = match correction.kind {
        CorrectionKind::LapsedRetraction => IntentKind::Reject,
        CorrectionKind::StaleConfirmation => IntentKind::Confirm,
    };
    let anchor = review.id.clone();
    identity::new_intent(
        kind,
        String::new(),
        Targets::Claim {
            version: identity::SHAPE_VERSION,
            claim: review,
            anchor,
        },
        Payload::empty(),
        None,
    )
}

/// What an answer about a proposal records about the item it names: the statements the owner
/// saw and made, the commitment an accept or an edit mints, and the item's source scope.
#[derive(Default)]
struct ProposalAnswer {
    seen: Option<StatementSnapshot>,
    edited: Option<StatementSnapshot>,
    subject: Option<EntityTarget>,
    parties: Vec<EntityTarget>,
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
        parties: answer.parties,
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
        if self
            .storage
            .snapshot(|view| still_queued(view, &id))?
            .action
            .is_some()
        {
            return self.accept_action(&id, None);
        }
        let issued = self.issue(|view, _| {
            let pending = still_queued(view, &id)?;
            let (subject, parties) = commitment(&pending)?;
            let intent = proposal_intent(
                IntentKind::Accept,
                Some(pending.id.clone()),
                None,
                ProposalAnswer {
                    seen: Some(statement(&pending)?),
                    subject: Some(subject),
                    parties,
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
            run: None,
        })
    }

    /// Accepts an action proposal: the accept intent and the run it starts are recorded in one
    /// transaction, then the provider is asked to create the issue and the run records what it
    /// created. When the provider fails, the accept and the run stay recorded and the run is not
    /// done; the error says so.
    fn accept_action(&mut self, item: &str, edits: Option<&ActionEdits>) -> Result<Answered> {
        let issuer = IssuerContext::owner_cli(&self.config.sources);
        let id = item.to_string();
        let kind = if edits.is_some() {
            IntentKind::Edit
        } else {
            IntentKind::Accept
        };
        let (intent, mut run) = self.storage.accept_action(|view| {
            let pending = still_queued(view, &id)?;
            let filed = (view.action_proposals(&[])?)
                .into_iter()
                .find(|filed| filed.id == pending.id)
                .ok_or_else(|| anyhow::anyhow!("{} is not an action proposal", pending.id))?;
            let fields = match edits {
                Some(edits) => edits.apply(filed.proposal.fields)?,
                None => filed.proposal.fields,
            };
            let mut intent = proposal_intent(
                kind,
                Some(pending.id.clone()),
                None,
                ProposalAnswer {
                    scope: Some(pending.source.scope.clone()),
                    ..ProposalAnswer::default()
                },
            )?;
            intent.issuer_context = issuer;
            let run = NewRun {
                id: actions::run_id(&intent.id),
                proposal: pending.id,
                accept_intent: intent.id.clone(),
                action: filed.proposal.action,
                provider: filed.provider,
                fields,
                source: filed.source,
                issue_id: actions::random_issue_id()?,
            };
            Ok((intent, run))
        })?;
        let result = self.execute_run(&run).with_context(|| {
            format!(
                "the accept is recorded as run {}, but its issue was not created",
                run.id
            )
        })?;
        run.status = RunStatus::Done;
        run.identifier = Some(result.identifier);
        run.url = Some(result.url);
        Ok(Answered {
            item: Some(id),
            intents: vec![intent],
            run: Some(run),
        })
    }

    /// Accepts a proposal with the owner's edits. `promise` and `date` replace what a commitment
    /// proposal says; each keeps the item's own when it is none. `fields` change an action
    /// proposal's title, description, team or assignee, and the run it starts creates the issue
    /// as edited. At least one change must be given, of the kind the item takes, or there is
    /// nothing to edit.
    pub fn queue_edit(
        &mut self,
        item: &str,
        promise: Option<&str>,
        date: Option<&str>,
        fields: &ActionEdits,
    ) -> Result<Answered> {
        let id = item.to_string();
        let is_action = (self.storage.snapshot(|view| still_queued(view, &id)))?
            .action
            .is_some();
        if is_action {
            ensure!(
                promise.is_none() && date.is_none(),
                "{id} is an action proposal: it takes --title, --description, --team and \
                 --assignee, not a promise or a date"
            );
            ensure!(
                !fields.is_empty(),
                "an edit changes the title, description, team or assignee; to accept the item \
                 unchanged, accept it"
            );
            return self.accept_action(&id, Some(fields));
        }
        ensure!(
            fields.is_empty(),
            "{id} is a commitment proposal: it takes --promise and --date, not an action's fields"
        );
        ensure!(
            promise.is_some() || date.is_some(),
            "an edit changes the promise, the stated date, or both; to accept the item unchanged, \
             accept it"
        );
        let (promise, date) = (promise.map(String::from), date.map(String::from));
        let issued = self.issue(|view, _| {
            let pending = still_queued(view, &id)?;
            let seen = statement(&pending)?;
            let edited = StatementSnapshot {
                promise: promise.clone().unwrap_or_else(|| seen.promise.clone()),
                stated_date: date.clone().or_else(|| seen.stated_date.clone()),
            };
            let (subject, parties) = commitment(&pending)?;
            let intent = proposal_intent(
                IntentKind::Edit,
                Some(pending.id.clone()),
                None,
                ProposalAnswer {
                    seen: Some(seen),
                    edited: Some(edited),
                    subject: Some(subject),
                    parties,
                    terms: accepted_terms(&pending, date.as_deref())?,
                    scope: Some(pending.source.scope.clone()),
                },
            )?;
            Ok((vec![intent], None, false))
        })?;
        Ok(Answered {
            item: Some(id),
            intents: issued.intents,
            run: None,
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
            if pending.action.is_none() {
                proposal(&pending)?;
            }
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
            run: None,
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
            run: None,
        })
    }

    /// Answers a question yes or no, naming the item. A question about a corrected claim takes
    /// only a yes, which records the earlier correction again. One intent records the whole answer: the
    /// verdict, and for a yes the Person that holds the accounts and, when this answer creates
    /// it, the name it gives. The identity graph's own planner decides who holds whom, so a yes
    /// follows `rastro identity answer`'s rules; what differs is that the decision is one row.
    pub fn queue_answer(&mut self, item: &str, yes: bool, name: Option<&str>) -> Result<Answered> {
        let id = item.to_string();
        let name = name.map(String::from);
        let issued = self.issue(|view, resolution| {
            let pending = still_queued(view, &id)?;
            if let Some(correction) = &pending.correction {
                ensure!(
                    yes,
                    "{} asks about claim {}: answer yes to hold your earlier correction again; \
                     to correct it the other way, use `rastro claim confirm` or `rastro claim \
                     reject`",
                    pending.id,
                    correction.claim.claim.id
                );
                return Ok((vec![repeat_correction(view, correction)?], None, false));
            }
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
            run: None,
        })
    }
}
