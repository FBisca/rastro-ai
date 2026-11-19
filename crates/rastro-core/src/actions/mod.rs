//! Action proposals: what an agent asks Rastro to do in an outside system, before anything runs.
//!
//! `rastro actions propose` records one pending proposal bound to the source it writes to and
//! that source's scope. It carries the action, the fields written for the outside system, the
//! proposer's why and the claims it cited, and it runs nothing: the proposal waits in the queue as
//! an item whose content-addressed id names all of that, so the same proposal is the same item.
//!
//! The action must be one an installed provider declares ([`crate::host::actions`]). The why, the
//! cited claims and the id stay in Rastro: [`IssueFields`] is the only part written for the
//! outside system.
use crate::queue::{self, Source as QueueSource, SourceKind};
use crate::{Config, Rastro, Source, Storage, host};
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
    pub proposed_at: DateTime<Utc>,
}

/// What `actions propose` returns: the proposal as it now waits in the queue.
pub type Proposed = Filed;

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

/// The queue's name for the source an action writes to.
fn source_of(source: &Source) -> QueueSource {
    QueueSource {
        kind: SourceKind::Linear,
        name: source.name.clone(),
        scope: source.scope.0.clone(),
    }
}

impl<S: Storage> Rastro<S> {
    /// Records one pending action proposal and runs nothing. Fails, recording nothing, when no
    /// installed provider offers the action, or [`plan`]'s checks fail.
    pub fn propose_action(&mut self, request: &ProposeRequest) -> Result<Proposed> {
        let offer = host::actions::offer(&self.locations, &request.action)?;
        let filed = plan(request, &offer, &self.config, crate::coverage::now()?)?;
        self.storage.record_action_proposal(&filed)?;
        Ok(filed)
    }
}
