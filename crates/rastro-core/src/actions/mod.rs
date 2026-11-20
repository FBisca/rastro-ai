//! Action proposals: what an agent asks Rastro to do in an outside system, before anything runs.
//!
//! `rastro actions propose` records one pending proposal bound to the source it writes to and
//! that source's scope. It carries the action, the fields written for the outside system, the
//! proposer's why and the claims it cited. When an owner rule matches it, the proposal starts a
//! pending Act run that waits out the rule's grace window; otherwise it runs nothing and waits
//! in the queue as an item whose content-addressed id names all of that, so the same proposal is
//! the same item.
//!
//! The action must be one an installed provider declares ([`crate::host::actions`]). The why, the
//! cited claims and the id stay in Rastro: [`IssueFields`] is the only part written for the
//! outside system.
mod runs;
mod stop;
mod undo;
mod why;
pub use rastro_plugin_sdk::{Reach, Reversibility};
pub use runs::{
    MAX_ATTEMPTS, NewAct, NewRun, Run, RunFailure, RunLevel, RunResult, RunReturn, RunStatus, Undo,
};
pub(crate) use runs::{act_run_id, random_issue_id, run_id};
pub use why::{Accept, Cell, RunWhy};

use crate::queue::{self, Source as QueueSource, SourceKind};
use crate::{
    Config, Rastro, ReadStorage, Reader, Scope, Snapshot, Source, Storage, host,
    knowledge::WhyOptions,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The action that creates an issue in Linear.
pub const LINEAR_CREATE_ISSUE: &str = "linear.create_issue";
/// The id of the connector whose source an action on Linear writes to.
const LINEAR_CONNECTOR: &str = "linear";
/// The assignee of a proposal that names none: the owner, as `identity` names them.
pub const OWNER: &str = "me";

/// What `config.toml` says about actions, under `[actions]`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActionsConfig {
    /// `[actions.linear]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linear: Option<LinearActions>,
}
impl ActionsConfig {
    pub(crate) fn is_empty(&self) -> bool {
        self.linear.is_none()
    }
}

/// `[actions.linear]`: the defaults of a proposal for Linear.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LinearActions {
    /// The team a `linear.create_issue` proposal takes when it names none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_team: Option<String>,
}

/// What the owner asks `actions propose` to record.
#[derive(Clone, Debug)]
pub struct ProposeRequest {
    pub action: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub team: Option<String>,
    pub assignee: Option<String>,
    pub why: String,
    pub cites: Vec<String>,
}

/// The fields of a `linear.create_issue` proposal: the content written for Linear, and nothing
/// else. The assignee is `me` for the owner, or the name the proposal gave.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "ActionIssueFields")]
pub struct IssueFields {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub team: String,
    pub assignee: String,
}

/// An action proposal: the action, its fields, why it is proposed and the claims it cites. The
/// why and the citations stay in Rastro and are never sent to the outside system.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "ActionProposal")]
pub struct Proposal {
    pub action: String,
    pub fields: IssueFields,
    pub why: String,
    /// The claims the proposer cited, sorted and without repeats.
    pub cites: Vec<String>,
}

/// A recorded proposal: its content address, the source it writes to with that source's scope,
/// the provider that declares its action, and when it was proposed.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "FiledActionProposal")]
pub struct Filed {
    /// The queue item's id: the content address of everything below except `proposed_at`.
    pub id: String,
    pub proposal: Proposal,
    pub source: QueueSource,
    /// The plugin id of the provider that declares the action.
    pub provider: String,
    /// Who the action affects, as the provider declared when it was proposed. Kept so the why of
    /// a run never depends on the plugin still being installed; not part of the answer.
    #[serde(skip)]
    #[schemars(skip)]
    pub reach: Reach,
    /// Whether the action can be undone, as the provider declared when it was proposed.
    #[serde(skip)]
    #[schemars(skip)]
    pub reversibility: Reversibility,
    pub proposed_at: DateTime<Utc>,
}

/// An issue a Linear source observed: what its evidence says of its identifier, title and state
/// type (`backlog`, `unstarted`, `started`, `completed`, `canceled`, …).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservedIssue {
    pub identifier: String,
    pub title: String,
    pub state: String,
}

/// Whether an observed issue's state type is one Linear counts as closed: Done or Canceled.
fn is_closed(state: &str) -> bool {
    matches!(state, "completed" | "canceled")
}

/// The identifiers of the open issues `source` observed, visible in its scope, whose title equals
/// the proposal's exactly, sorted. Empty for any action but `linear.create_issue`.
pub(crate) fn duplicates(
    view: &dyn Snapshot,
    source: &QueueSource,
    proposal: &Proposal,
) -> Result<Vec<String>> {
    if proposal.action != LINEAR_CREATE_ISSUE {
        return Ok(Vec::new());
    }
    let mut identifiers: Vec<String> = (view
        .linear_issues(&source.name, &[Scope(source.scope.clone())])?)
    .into_iter()
    .filter(|issue| issue.title == proposal.fields.title && !is_closed(&issue.state))
    .map(|issue| issue.identifier)
    .collect();
    identifiers.sort();
    identifiers.dedup();
    Ok(identifiers)
}

/// What `actions propose` returns: the proposal as it now waits in the queue, and the Act run a
/// matching rule started for it, when one did.
#[derive(Debug, JsonSchema, Serialize)]
#[schemars(rename = "ActionProposed")]
pub struct Proposed {
    #[serde(flatten)]
    pub filed: Filed,
    /// The Act run the proposal started; absent when it waits in the queue for the owner. Boxed
    /// so the answer stays the size of the proposal it carries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<Box<Run>>,
}

/// The proposal `request` makes, once checked against what the host offers and the config says:
/// the action must be offered and named `linear.create_issue`, the why and the title must say
/// something, the team comes from the request or `[actions.linear] default_team`, and a source of
/// the Linear connector must be registered. Fails naming the missing piece; nothing is recorded.
fn plan(
    request: &ProposeRequest,
    offer: &host::actions::ActionOffer,
    config: &Config,
    proposed_at: DateTime<Utc>,
) -> Result<Filed> {
    if offer.action != LINEAR_CREATE_ISSUE {
        bail!(
            "the action {} is offered by {}, but Rastro can't propose it yet",
            offer.action,
            offer.provider
        );
    }
    if request.why.trim().is_empty() {
        bail!("an action proposal needs --why: say why it is proposed");
    }
    let title = request
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
        .with_context(|| format!("{LINEAR_CREATE_ISSUE} needs --title"))?;
    let team = [
        request.team.as_deref(),
        (config.actions.linear.as_ref()).and_then(|linear| linear.default_team.as_deref()),
    ]
    .into_iter()
    .flatten()
    .find(|team| !team.trim().is_empty())
    .with_context(|| {
        format!(
            "{LINEAR_CREATE_ISSUE} needs a team: pass --team, or set [actions.linear] default_team in config.toml"
        )
    })?;
    let source = linear_source(&config.sources)?;
    let mut cites = request.cites.clone();
    cites.sort();
    cites.dedup();
    let proposal = Proposal {
        action: offer.action.clone(),
        fields: IssueFields {
            title: title.into(),
            description: (request.description.clone()).filter(|text| !text.trim().is_empty()),
            team: team.into(),
            assignee: (request.assignee.clone())
                .filter(|who| !who.trim().is_empty())
                .unwrap_or_else(|| OWNER.into()),
        },
        why: request.why.clone(),
        cites,
    };
    Ok(Filed {
        id: queue::action_item_id(&source, &proposal),
        proposal,
        source,
        provider: offer.provider.clone(),
        reach: offer.reach,
        reversibility: offer.reversibility,
        proposed_at,
    })
}

/// The one source of the Linear connector, which `linear.create_issue` writes to. Fails when none
/// is registered, and when several are: a proposal is bound to the source it writes to and its
/// scope, so Rastro never picks one for the owner.
fn linear_source(sources: &[Source]) -> Result<QueueSource> {
    let mut linear = sources
        .iter()
        .filter(|source| source.connector == LINEAR_CONNECTOR);
    match (linear.next(), linear.next()) {
        (Some(source), None) => Ok(source_of(source)),
        (None, _) => bail!(
            "{LINEAR_CREATE_ISSUE} writes to a Linear source, and none is registered: add one with `rastro source add`"
        ),
        (Some(first), Some(second)) => {
            let names: Vec<&str> = [first, second]
                .into_iter()
                .chain(linear)
                .map(|source| source.name.as_str())
                .collect();
            bail!(
                "{LINEAR_CREATE_ISSUE} writes to a Linear source, and {} are registered ({}): keep one",
                names.len(),
                names.join(", ")
            )
        }
    }
}

/// Refuses a citation the source's scope cannot see, naming the claim and the scope. A claim that
/// scope cannot see would reach a reader of that scope through the proposal, so the proposal is
/// not recorded.
fn check_citations(view: &dyn Snapshot, cites: &[String], source: &QueueSource) -> Result<()> {
    let scope = [Scope(source.scope.clone())];
    for claim in cites {
        if view
            .explain_scoped(claim, &WhyOptions::default(), &scope)?
            .is_none()
        {
            bail!(
                "the claim {claim} is not visible in the scope {} of the source {} the action writes to: cite only claims that scope can see",
                source.scope,
                source.name
            );
        }
    }
    Ok(())
}

/// The queue's name for the source an action writes to.
fn source_of(source: &Source) -> QueueSource {
    QueueSource {
        kind: SourceKind::Linear,
        name: source.name.clone(),
        scope: source.scope.0.clone(),
    }
}

impl<S: Storage> Rastro<S> {
    /// Records one pending action proposal. When an owner rule matches it on action, team and
    /// assignee and no open observed issue holds its title, starts the pending Act run the rule
    /// authorizes, due when its grace ends, and sends nothing; otherwise the proposal waits in
    /// the queue. A proposal that already has an Act run keeps it. Fails, recording nothing,
    /// when no installed provider offers the action, or [`plan`]'s checks fail.
    pub fn propose_action(&mut self, request: &ProposeRequest) -> Result<Proposed> {
        let offer = host::actions::offer(&self.locations, &request.action)?;
        let now = crate::coverage::now()?;
        let filed = plan(request, &offer, &self.config, now)?;
        self.storage
            .snapshot(|view| check_citations(view, &filed.proposal.cites, &filed.source))?;
        self.storage.record_action_proposal(&filed)?;
        let (dups, existing) = self.storage.snapshot(|view| {
            Ok((
                duplicates(view, &filed.source, &filed.proposal)?,
                view.action_runs(&[])?
                    .into_iter()
                    .find(|run| run.proposal == filed.id),
            ))
        })?;
        // One proposal never has two runs: one that already has an Act run keeps it.
        if let Some(run) = existing {
            let run = (run.level == RunLevel::Act).then_some(Box::new(run));
            return Ok(Proposed { filed, run });
        }
        let rule = match self.config.matching_rule(&filed.proposal) {
            Some(rule) if dups.is_empty() => rule,
            _ => return Ok(Proposed { filed, run: None }),
        };
        let grace = chrono::Duration::from_std(self.config.grace_of(rule))
            .context("the rule's grace is too long to count")?;
        let due_at = now
            .checked_add_signed(grace)
            .context("the rule's grace ends beyond the calendar")?;
        let act = NewAct {
            id: act_run_id(&filed.id),
            proposal: filed.id.clone(),
            action: filed.proposal.action.clone(),
            provider: filed.provider.clone(),
            fields: filed.proposal.fields.clone(),
            source: filed.source.clone(),
            issue_id: random_issue_id()?,
            rule: self.config.rule_copy(rule),
            due_at,
        };
        let run = self.storage.start_act(&act, filed.proposed_at)?;
        Ok(Proposed {
            filed,
            run: Some(Box::new(run)),
        })
    }
}

/// What `actions` lists: the runs the reader sees, oldest first, each with its status. Given a
/// run, `runs` holds that run alone and `why` explains it.
#[derive(Debug, JsonSchema, Serialize)]
#[schemars(rename = "ActionRuns")]
pub struct Runs {
    pub runs: Vec<Run>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<RunWhy>,
}

impl Runs {
    /// The runs `scopes` see, or the one run `run` names with its why.
    fn read(view: &dyn Snapshot, scopes: &[Scope], run: Option<&str>) -> Result<Self> {
        Ok(match run {
            None => Self {
                runs: view.action_runs(scopes)?,
                why: None,
            },
            Some(id) => {
                let (run, why) = why::explain(view, scopes, id)?;
                Self {
                    runs: vec![run],
                    why: Some(why),
                }
            }
        })
    }
}

impl<S: Storage> Rastro<S> {
    /// The runs over every scope, on a writing open; or the one run `run` names, explained.
    pub fn action_runs(&mut self, run: Option<&str>) -> Result<Runs> {
        self.storage.snapshot(|view| Runs::read(view, &[], run))
    }
}

impl<S: ReadStorage> Reader<S> {
    /// The runs of the sources the reader's scopes see: a run in another scope is absent, and the
    /// read still succeeds. With `run`, the one run it names and why it ran; a run these scopes
    /// do not see is an error.
    pub fn action_runs(&mut self, run: Option<&str>) -> Result<Runs> {
        let Self {
            locations,
            storage,
            scopes,
            ..
        } = self;
        crate::read_only::snapshot(locations, storage, scopes, |view, _| match view {
            Some(view) => Runs::read(view, scopes, run),
            None => match run {
                None => Ok(Runs {
                    runs: Vec::new(),
                    why: None,
                }),
                Some(id) => {
                    bail!("no action run {id} is visible here: list the runs with `actions`")
                }
            },
        })
    }
}
