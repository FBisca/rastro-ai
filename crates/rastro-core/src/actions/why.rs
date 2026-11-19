//! Why an action ran: one run with its proposal, the claims it cited and their evidence, the
//! owner's accept, its level, its cell and the risk its provider declares for it.
//!
//! Only a reader whose scopes see the source the run writes to gets the run: for any other
//! reader the run does not exist, and so is its why.
use super::{Proposal, Run};
use crate::knowledge::{Explanation, WhyOptions};
use crate::{Locations, Scope, Snapshot, host};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Serialize;

/// Every cell sits at Propose until the owner writes a rule that raises it (the glossary's Level).
const PROPOSE: &str = "propose";

/// The owner's accept of the proposal: the intent that recorded it and when.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "ActionAccept")]
pub struct Accept {
    pub intent: String,
    pub at: DateTime<Utc>,
}

/// The cell a run belongs to: its action and the context it hangs on, the Linear team.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "ActionCell")]
pub struct Cell {
    pub action: String,
    pub team: String,
}

/// What explains one run. The why and the citations live here and nowhere in the outside system.
#[derive(Debug, JsonSchema, Serialize)]
#[schemars(rename = "ActionRunWhy")]
pub struct RunWhy {
    /// The proposal as filed: the action, its fields, the proposer's why and the claims it cited.
    pub proposal: Proposal,
    /// Each cited claim as `why` explains it, with its evidence.
    pub cited: Vec<Explanation>,
    pub accept: Accept,
    /// The level the cell sat at when the owner accepted: `propose`.
    pub level: &'static str,
    pub cell: Cell,
    /// Who the action affects, in the owner's words.
    pub reach: &'static str,
    /// Whether the action can be undone, in the owner's words.
    pub reversibility: &'static str,
}

/// The run `id` among those `scopes` see, explained. Fails when no run of that id is visible,
/// without saying whether it exists elsewhere.
pub(crate) fn explain(
    view: &dyn Snapshot,
    locations: &Locations,
    scopes: &[Scope],
    id: &str,
) -> Result<(Run, RunWhy)> {
    let Some(run) = (view.action_runs(scopes)?)
        .into_iter()
        .find(|run| run.id == id)
    else {
        bail!("no action run {id} is visible here: list the runs with `actions`");
    };
    let filed = (view.action_proposals(scopes)?)
        .into_iter()
        .find(|filed| filed.id == run.proposal)
        .with_context(|| format!("the proposal of run {id} is not recorded"))?;
    let scope = [Scope(run.source.scope.clone())];
    let cited = filed
        .proposal
        .cites
        .iter()
        .map(|claim| {
            view.explain_scoped(claim, &WhyOptions::default(), &scope)?
                .with_context(|| {
                    format!(
                        "the claim {claim} cited by run {id} can't be read in the scope {}",
                        scope[0].0
                    )
                })
        })
        .collect::<Result<Vec<_>>>()?;
    let offer = host::actions::offer(locations, &run.action)
        .with_context(|| format!("the provider of run {id} no longer offers its action"))?;
    let why = RunWhy {
        proposal: filed.proposal,
        cited,
        accept: Accept {
            intent: run.accept_intent.clone(),
            at: run.accepted_at,
        },
        level: PROPOSE,
        cell: Cell {
            action: run.action.clone(),
            team: run.fields.team.clone(),
        },
        reach: offer.reach.label(),
        reversibility: offer.reversibility.label(),
    };
    Ok((run, why))
}
