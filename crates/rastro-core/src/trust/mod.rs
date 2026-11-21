//! The trust view: each cell's level and the evidence behind it, derived at read from the runs
//! and owner intents the visible scopes hold. Nothing stores a cell or a level: a cell is the
//! run's action and its team, and its level, counts and eligibility are computed from what
//! happened to its runs, under the thresholds of `[learning]`.
//!
//! Promotion is proposed, never self-applied: an eligible cell with no rule is shown the exact
//! `[[rules]]` entry that raises it, and a demoted cell the reset command that restores its
//! baseline. Demotion is automatic: an undo or a correction of a claim a done run cited drops
//! the cell one level, repeated undos in its last outcomes drop it to Ask, and a cell idle past
//! the decay window drops a level at its next use. Every level change is derived with its cause
//! and time, so the cell carries its journal and the row names the last change. A cell no
//! visible run touches is absent, and an empty store lists no cells.
use crate::actions::{Filed, Run, RunStatus};
use crate::identity::{self, Intent, IntentKind, IntentStatus, Payload, Resolution, Targets};
use crate::queue::ShownReport;
use crate::read_only::Snapshot;
use crate::{Config, Rastro, ReadStorage, Reader, Scope, Storage};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// `[learning]`: the thresholds trust derives under. Each omitted entry keeps its default:
/// 10 clean outcomes over 7 days with 0 undos in the last 20, repeated undos from 2, decay after
/// 30 days.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct LearningConfig {
    /// How many clean outcomes a cell needs to become eligible for promotion.
    pub clean_min: usize,
    /// How many days the clean outcomes must span, first to last.
    pub clean_days: i64,
    /// How many of the cell's last outcomes the undo count reads.
    pub undo_window: usize,
    /// How many undos in that window drop the cell to Ask.
    pub repeat_undos: usize,
    /// How many idle days drop a level at the cell's next use.
    pub decay_days: i64,
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            clean_min: 10,
            clean_days: 7,
            undo_window: 20,
            repeat_undos: 2,
            decay_days: 30,
        }
    }
}

impl LearningConfig {
    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Every threshold is a positive count of outcomes or days: zero would read no evidence or
    /// no window at all.
    pub(crate) fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("clean_min", self.clean_min as i64),
            ("clean_days", self.clean_days),
            ("undo_window", self.undo_window as i64),
            ("repeat_undos", self.repeat_undos as i64),
            ("decay_days", self.decay_days),
        ] {
            anyhow::ensure!(value > 0, "[learning] {name} must be positive");
        }
        Ok(())
    }
}

/// A cell's level: Act, Propose or Ask (levels §1).
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Ask,
    Propose,
    Act,
}

impl Level {
    /// The word the owner reads and the JSON carries.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Propose => "propose",
            Self::Act => "act",
        }
    }

    /// The level a word names, as the owner types it on the command line.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ask" => Some(Self::Ask),
            "propose" => Some(Self::Propose),
            "act" => Some(Self::Act),
            _ => None,
        }
    }

    /// One level down; Ask is the floor.
    fn below(self) -> Self {
        match self {
            Self::Act => Self::Propose,
            Self::Propose | Self::Ask => Self::Ask,
        }
    }
}

/// A level change: the levels it moved between, what caused it and when. One entry of the
/// cell's journal; the trust view names the last of them.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "TrustLevelChange")]
pub struct LevelChange {
    pub from: Level,
    pub to: Level,
    pub cause: String,
    pub at: DateTime<Utc>,
}

/// What the cell's next promotion needs: whether the evidence qualifies it, the sentence saying
/// what to do, and the exact rule text or reset command when one is proposed.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "TrustPromotion")]
pub struct Promotion {
    /// Whether the cell's outcomes meet the promotion bar: enough clean outcomes over enough
    /// days with no undo in its last outcomes.
    pub eligible: bool,
    /// What the next promotion needs, in words.
    pub needs: String,
    /// The exact `[[rules]]` entry to add to config.toml; present for an eligible cell with no
    /// rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_text: Option<String>,
    /// The command that restores a demoted cell's baseline; present for an eligible demoted
    /// cell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_command: Option<String>,
}

/// One cell's trust: its level, the evidence counts behind it, its last level change and what
/// the next promotion needs.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "TrustCell")]
pub struct TrustCell {
    pub action: String,
    pub team: String,
    pub level: Level,
    /// How many clean outcomes the cell holds: kept runs the owner accepted unchanged, or a
    /// rule started.
    pub clean: usize,
    /// How many of the cell's last outcomes are undos.
    pub undos: usize,
    /// The cell's journal: every level change it derived, oldest first, each with the levels it
    /// moved between, its cause and its time. Empty while the level never changed.
    pub changes: Vec<LevelChange>,
    /// The last level change with its cause; absent when the level never changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_change: Option<LevelChange>,
    pub promotion: Promotion,
}

/// What `trust` returns: every visible cell, by action and team, and every visible plugin's
/// usefulness and cost.
#[derive(Debug, JsonSchema, Serialize)]
#[schemars(rename = "Trust")]
pub struct Trust {
    pub cells: Vec<TrustCell>,
    pub plugins: Vec<TrustPlugin>,
}

impl Trust {
    pub fn empty() -> Self {
        Self {
            cells: Vec::new(),
            plugins: Vec::new(),
        }
    }
}

/// What storage reads for one plugin's cost: the wall time of its finished runs in
/// milliseconds, the adapter turns they used, and the ids of the runs whose content a shown
/// report names for the plugin's items.
#[derive(Clone, Debug)]
pub struct PluginRuns {
    pub plugin: String,
    pub wall_ms: i64,
    pub turns: usize,
    pub runs: Vec<String>,
}

/// A plugin's engagement: acted on over seen, each item counted once, from the terminal's
/// shown reports; `unmeasured` where no shown report names the plugin's runs — unseen is not
/// ignored, and never reads 0.
#[derive(Clone, Copy, Debug, JsonSchema, Serialize)]
#[serde(untagged)]
pub enum Engagement {
    Measured { acted: usize, seen: usize },
    Unmeasured(Unmeasured),
}

/// The word an unmeasured engagement reads, in the JSON as on the terminal.
#[derive(Clone, Copy, Debug, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unmeasured {
    Unmeasured,
}

/// What a plugin's runs cost: their wall time in milliseconds plus the adapter turns they
/// used.
#[derive(Clone, Copy, Debug, JsonSchema, Serialize)]
#[schemars(rename = "TrustCost")]
pub struct Cost {
    pub wall_ms: i64,
    pub turns: usize,
}

/// One plugin's usefulness and cost: its engagement, the time to action, what its runs cost
/// and what that cost reads per accepted item.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "TrustPlugin")]
pub struct TrustPlugin {
    pub plugin: String,
    pub engagement: Engagement,
    /// The mean seconds from an item's first shown report to the intent that answered it,
    /// over the plugin's acted-on items; absent when nothing was acted on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_action_seconds: Option<f64>,
    pub cost: Cost,
    /// The cost over the plugin's accepted items — an accept unchanged or with edits — in
    /// whole milliseconds and turns, divided down; a dash when nothing was accepted.
    pub cost_per_accepted: CostPerAccepted,
}

/// The cost per accepted item: the cost divided by the accepts, or a dash when nothing was
/// accepted — never a division by zero.
#[derive(Clone, Copy, Debug, JsonSchema, Serialize)]
#[serde(untagged)]
pub enum CostPerAccepted {
    Measured(Cost),
    Dash(Dash),
}

/// The dash a cost per accepted reads when nothing was accepted, in the JSON as on the
/// terminal.
#[derive(Clone, Copy, Debug, JsonSchema, Serialize)]
pub enum Dash {
    #[serde(rename = "—")]
    Dash,
}

/// The cell a journal change names: one action and team.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "TrustJournalCell")]
pub struct JournalCell {
    pub action: String,
    pub team: String,
}

/// One level change as the journal lists it: the cell it moved, the levels it moved between,
/// what caused it and when.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "TrustJournalChange")]
pub struct JournalChange {
    pub cell: JournalCell,
    pub from: Level,
    pub to: Level,
    pub cause: String,
    pub at: DateTime<Utc>,
}

/// What `trust journal` returns: every level change the visible cells derived, newest first.
#[derive(Debug, JsonSchema, Serialize)]
#[schemars(rename = "TrustJournal")]
pub struct TrustJournal {
    pub changes: Vec<JournalChange>,
}

impl TrustJournal {
    pub fn empty() -> Self {
        Self {
            changes: Vec::new(),
        }
    }
}

/// Derives the journal from the same evidence as the trust view: every visible cell's changes,
/// newest first. A change held at the same instant as another reads after it, the reverse of
/// the order the cells derived them in.
pub(crate) fn trust_journal(
    config: &Config,
    view: Option<&dyn Snapshot>,
    scopes: &[Scope],
) -> Result<TrustJournal> {
    let trust = trust_view(config, view, scopes)?;
    let mut changes: Vec<JournalChange> = trust
        .cells
        .iter()
        .flat_map(|cell| {
            cell.changes.iter().map(move |change| JournalChange {
                cell: JournalCell {
                    action: cell.action.clone(),
                    team: cell.team.clone(),
                },
                from: change.from,
                to: change.to,
                cause: change.cause.clone(),
                at: change.at,
            })
        })
        .collect();
    changes.sort_by_key(|change| change.at);
    changes.reverse();
    Ok(TrustJournal { changes })
}

/// Derives the trust view from one snapshot: the runs and proposals the scopes see, grouped by
/// cell.
pub(crate) fn trust_view(
    config: &Config,
    view: Option<&dyn Snapshot>,
    scopes: &[Scope],
) -> Result<Trust> {
    let Some(view) = view else {
        return Ok(Trust::empty());
    };
    let runs = view.action_runs(scopes)?;
    let proposals = view.action_proposals(scopes)?;
    let intents = view.intents()?;
    let plugin_runs = view.plugin_runs(scopes)?;
    let shown = view.shown_reports()?;
    let resolution = Resolution::of(&intents);
    let mut cells: BTreeMap<(String, String), Vec<&Run>> = BTreeMap::new();
    for run in &runs {
        cells
            .entry((run.action.clone(), run.fields.team.clone()))
            .or_default()
            .push(run);
    }
    let mut filed: BTreeMap<(String, String), Vec<&Filed>> = BTreeMap::new();
    for proposal in &proposals {
        filed
            .entry((
                proposal.proposal.action.clone(),
                proposal.proposal.fields.team.clone(),
            ))
            .or_default()
            .push(proposal);
    }
    Ok(Trust {
        cells: cells
            .into_iter()
            .map(|((action, team), runs)| {
                let proposals = filed
                    .get(&(action.clone(), team.clone()))
                    .map_or(Vec::new(), |filed| filed.clone());
                derive_cell(
                    config,
                    &action,
                    &team,
                    &runs,
                    &proposals,
                    &intents,
                    &resolution,
                )
            })
            .collect::<Result<Vec<_>>>()?,
        plugins: derive_plugins(&plugin_runs, &shown, &intents),
    })
}

/// The level one cell sits at now, derived from the same evidence as the trust view: the gate a
/// matching rule and a pending Act's re-check read before anything runs.
pub(crate) fn cell_level(
    config: &Config,
    view: &dyn Snapshot,
    scopes: &[Scope],
    action: &str,
    team: &str,
) -> Result<Level> {
    Ok(derive_cell_from_view(config, view, scopes, action, team)?.level)
}

/// One cell's trust from the snapshot's runs, proposals and intents, whether or not any run
/// touches it yet: a pin or a reset returns the cell it just changed.
fn derive_cell_from_view(
    config: &Config,
    view: &dyn Snapshot,
    scopes: &[Scope],
    action: &str,
    team: &str,
) -> Result<TrustCell> {
    let runs = view.action_runs(scopes)?;
    let proposals = view.action_proposals(scopes)?;
    let intents = view.intents()?;
    let resolution = Resolution::of(&intents);
    let runs: Vec<&Run> = runs
        .iter()
        .filter(|run| run.action == action && run.fields.team == team)
        .collect();
    let proposals: Vec<&Filed> = proposals
        .iter()
        .filter(|filed| filed.proposal.action == action && filed.proposal.fields.team == team)
        .collect();
    derive_cell(
        config,
        action,
        team,
        &runs,
        &proposals,
        &intents,
        &resolution,
    )
}

/// The plugins' usefulness from the shown reports and the intents that answered them: each
/// item counts once, attributed to the plugin whose run carried its content. An item no
/// terminal read showed attributes to no plugin, and a question, which has no source run, to
/// none. The time to action is the mean wait of the acted-on items, from the first report that
/// showed each to the intent that answered it. Cost is the wall time of the plugin's runs plus
/// their adapter turns; per accepted item it divides by the accepts, an accept unchanged or
/// with edits, and reads a dash when nothing was accepted.
fn derive_plugins(
    plugin_runs: &[PluginRuns],
    shown: &[ShownReport],
    intents: &[Intent],
) -> Vec<TrustPlugin> {
    let mut run_plugin: BTreeMap<&str, &str> = BTreeMap::new();
    for facts in plugin_runs {
        for run in &facts.runs {
            run_plugin.insert(run.as_str(), facts.plugin.as_str());
        }
    }
    // Each shown item's plugin and the first moment a terminal read printed it.
    let mut first_shown: BTreeMap<&str, (&str, DateTime<Utc>)> = BTreeMap::new();
    for report in shown {
        let Some(&plugin) = run_plugin.get(report.run.as_str()) else {
            continue;
        };
        first_shown
            .entry(report.item.as_str())
            .and_modify(|(_, at)| *at = (*at).min(report.at))
            .or_insert((plugin, report.at));
    }
    // The latest intent that answered each item, in acceptance order: an accept, an edit, a
    // reject or an answer, the same kinds the queue's outcomes read.
    let mut answers: BTreeMap<&str, &Intent> = BTreeMap::new();
    for intent in intents {
        if matches!(
            intent.kind,
            IntentKind::Accept | IntentKind::Edit | IntentKind::Reject | IntentKind::Answer
        ) && let Some(item) = intent.queue_item()
        {
            answers.insert(item, intent);
        }
    }
    #[derive(Clone, Copy, Default)]
    struct EngagementCounts {
        seen: usize,
        acted: usize,
        waited_ms: i64,
        accepted: usize,
    }
    let mut by_plugin: BTreeMap<&str, EngagementCounts> = BTreeMap::new();
    for (item, (plugin, first)) in first_shown {
        let counts = by_plugin.entry(plugin).or_default();
        counts.seen += 1;
        if let Some(intent) = answers.get(item) {
            counts.acted += 1;
            counts.waited_ms += (intent.issued_at - first).num_milliseconds();
            if matches!(intent.kind, IntentKind::Accept | IntentKind::Edit) {
                counts.accepted += 1;
            }
        }
    }
    plugin_runs
        .iter()
        .map(|facts| {
            let counts = by_plugin
                .get(facts.plugin.as_str())
                .copied()
                .unwrap_or_default();
            // A plugin no terminal read showed is unmeasured, never 0: unseen is not ignored.
            let engagement = if counts.seen == 0 {
                Engagement::Unmeasured(Unmeasured::Unmeasured)
            } else {
                Engagement::Measured {
                    acted: counts.acted,
                    seen: counts.seen,
                }
            };
            TrustPlugin {
                plugin: facts.plugin.clone(),
                engagement,
                time_to_action_seconds: (counts.acted > 0)
                    .then(|| counts.waited_ms as f64 / 1000.0 / counts.acted as f64),
                cost: Cost {
                    wall_ms: facts.wall_ms,
                    turns: facts.turns,
                },
                cost_per_accepted: if counts.accepted > 0 {
                    CostPerAccepted::Measured(Cost {
                        wall_ms: facts.wall_ms / counts.accepted as i64,
                        turns: facts.turns / counts.accepted,
                    })
                } else {
                    CostPerAccepted::Dash(Dash::Dash)
                },
            }
        })
        .collect()
}

/// What happened to one run, terminally: a kept run is a clean outcome unless the owner accepted
/// it with edits, and an undone run is an undo. A correction landing on a claim a done run cited
/// counts as an undo too. A run still accepted, pending, returned, stopped or failed is no
/// outcome: nothing terminal happened to it.
#[derive(Clone, Copy)]
enum OutcomeKind {
    Clean,
    Edited,
    Undo,
}

struct Outcome {
    kind: OutcomeKind,
    at: DateTime<Utc>,
    /// What names the undo in the journal; absent for a clean or edited outcome.
    cause: Option<String>,
}

/// A correction landing on a claim: a retraction or an edit, with its time and the word the
/// journal names it by.
struct Correction {
    at: DateTime<Utc>,
    word: &'static str,
}

/// The corrections that land on each claim, oldest first: a `reject` retracts it and an `edit`
/// fixes its object, both ending what rests on it. An undone intent corrects nothing; a later
/// correction supersedes an earlier one without erasing the change it caused.
fn corrections<'a>(
    intents: &'a [Intent],
    resolution: &Resolution,
) -> BTreeMap<&'a str, Vec<Correction>> {
    let mut map: BTreeMap<&str, Vec<Correction>> = BTreeMap::new();
    for intent in intents {
        let word = match intent.kind {
            IntentKind::Reject => "retraction",
            IntentKind::Edit => "edit",
            _ => continue,
        };
        let Some(claim) = intent.corrects() else {
            continue;
        };
        if matches!(
            resolution.status.get(&intent.id),
            Some((IntentStatus::Undone, _))
        ) {
            continue;
        }
        map.entry(claim).or_default().push(Correction {
            at: intent.issued_at,
            word,
        });
    }
    for corrections in map.values_mut() {
        corrections.sort_by_key(|correction| correction.at);
    }
    map
}

/// The cell's outcomes, oldest first: one per run that reached a terminal state, plus one per
/// correction that landed on a claim a done run cited after the run started.
fn outcomes(
    runs: &[&Run],
    proposals: &[&Filed],
    kinds: &BTreeMap<&str, IntentKind>,
    corrections: &BTreeMap<&str, Vec<Correction>>,
) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    for run in runs {
        match run.status {
            RunStatus::Done => {
                let edited = run
                    .accept_intent
                    .as_deref()
                    .is_some_and(|id| matches!(kinds.get(id), Some(IntentKind::Edit)));
                outcomes.push(Outcome {
                    kind: if edited {
                        OutcomeKind::Edited
                    } else {
                        OutcomeKind::Clean
                    },
                    at: run.accepted_at,
                    cause: None,
                });
                // A correction of a claim this done run cited counts as an undo: the owner
                // ended what the run rested on. Only a done run cited it to any effect; a
                // pending Act the re-check returns demotes nothing.
                let cites = proposals
                    .iter()
                    .find(|filed| filed.id == run.proposal)
                    .map(|filed| &filed.proposal.cites);
                for claim in cites.into_iter().flatten() {
                    for correction in corrections.get(claim.as_str()).into_iter().flatten() {
                        if correction.at > run.accepted_at {
                            outcomes.push(Outcome {
                                kind: OutcomeKind::Undo,
                                at: correction.at,
                                cause: Some(format!("{} of claim {claim}", correction.word)),
                            });
                        }
                    }
                }
            }
            RunStatus::Undone => {
                // An undone run holds its undo intent; storage derives the status from it.
                if let Some(undo) = &run.undo {
                    outcomes.push(Outcome {
                        kind: OutcomeKind::Undo,
                        at: undo.at,
                        cause: Some("undo".into()),
                    });
                }
            }
            RunStatus::Accepted
            | RunStatus::Pending
            | RunStatus::Returned
            | RunStatus::Stopped
            | RunStatus::Failed => {}
        }
    }
    outcomes.sort_by_key(|outcome| outcome.at);
    outcomes
}

/// Whether the owner's rules already raise this cell: a rule matching its action and team.
fn has_rule(config: &Config, action: &str, team: &str) -> bool {
    config
        .rules
        .iter()
        .any(|rule| rule.action.as_deref() == Some(action) && rule.team.as_deref() == Some(team))
}

/// The exact `[[rules]]` entry that raises the cell: a real rule, serialized as the owner would
/// paste it, so adding it to config.toml loads and matches.
fn proposed_rule(action: &str, team: &str) -> Result<String> {
    let rule = crate::rules::Rule {
        name: format!("trust-{action}-{team}"),
        action: Some(action.into()),
        team: Some(team.into()),
        assignee: Some(crate::actions::OWNER.into()),
        grace: None,
        daily_cap: None,
    };
    Ok(format!(
        "[[rules]]\n{}",
        toml::to_string_pretty(&rule).context("serializing the proposed rule")?
    ))
}

/// A pin's stretch on a cell: the level it holds, from its intent's time until the intent that
/// superseded it, or none while it still holds. An undone pin holds nothing: the replay removes
/// its contribution, as it does for an undone correction.
struct PinStretch {
    level: Level,
    start: DateTime<Utc>,
    end: Option<DateTime<Utc>>,
}

/// The cell's owner controls from the intent log: its pin stretches, oldest first, and the
/// times its resets cleared its learned history. An undone control contributes nothing.
fn controls(
    intents: &[Intent],
    resolution: &Resolution,
    action: &str,
    team: &str,
) -> (Vec<PinStretch>, Vec<DateTime<Utc>>) {
    let mut pins: Vec<PinStretch> = Vec::new();
    let mut resets: Vec<DateTime<Utc>> = Vec::new();
    for intent in intents {
        if intent.targets.cell() != Some((action, team)) {
            continue;
        }
        let status = resolution.status.get(&intent.id);
        if matches!(status, Some((IntentStatus::Undone, _))) {
            continue;
        }
        match intent.kind {
            IntentKind::Pin => {
                let Some(level) = intent.targets.pinned_level() else {
                    continue;
                };
                let end = match status {
                    Some((IntentStatus::Superseded, Some(by))) => intents
                        .iter()
                        .find(|candidate| candidate.id == *by)
                        .map(|candidate| candidate.issued_at),
                    _ => None,
                };
                pins.push(PinStretch {
                    level,
                    start: intent.issued_at,
                    end,
                });
            }
            IntentKind::Reset => resets.push(intent.issued_at),
            _ => {}
        }
    }
    pins.sort_by_key(|pin| pin.start);
    resets.sort();
    (pins, resets)
}

/// A demotion a pin holds lands in the journal as a held entry naming the pin and the cause it
/// held; without a pin it moves the level when it moves at all.
fn hold_or_move(
    level: &mut Level,
    pinned: Option<Level>,
    changes: &mut Vec<LevelChange>,
    to: Level,
    cause: String,
    at: DateTime<Utc>,
) {
    if let Some(held) = pinned {
        changes.push(LevelChange {
            from: *level,
            to: *level,
            cause: format!("pin holds at {}: {}", held.as_str(), cause),
            at,
        });
    } else if to != *level {
        changes.push(LevelChange {
            from: *level,
            to,
            cause,
            at,
        });
        *level = to;
    }
}

/// The level a cell sits at and its journal, folded from its outcomes, its uses and its owner
/// controls in time order. Every undo or correction drops one level, and `repeat_undos` undos
/// inside the last `undo_window` outcomes drop straight to Ask; a gap of `decay_days` or more
/// between two uses decays the cell one level at the later use, naming the idle days and the
/// window. A pin holds the exact level against learning in both directions while its stretch
/// lasts, and a reset clears the learned history back to the baseline. The fold only ever moves
/// down on its own — promotion is never self-applied — and each move it makes, and each demotion
/// a pin holds, is one entry of the journal, oldest first.
fn fold_level(
    baseline: Level,
    learning: &LearningConfig,
    outcomes: &[Outcome],
    uses: &[DateTime<Utc>],
    pins: &[PinStretch],
    resets: &[DateTime<Utc>],
) -> (Level, Vec<LevelChange>) {
    enum FoldEvent<'a> {
        Outcome(&'a Outcome),
        Decay(String),
        Pin(Level),
        PinEnd,
        Reset,
    }
    // At one instant a pin's stretch ends before the next begins, a reset clears before the
    // outcomes at that instant count, a pin holds before an outcome lands, and an outcome
    // precedes the decay its use completes.
    fn rank(event: &FoldEvent) -> u8 {
        match event {
            FoldEvent::PinEnd => 0,
            FoldEvent::Reset => 1,
            FoldEvent::Pin(_) => 2,
            FoldEvent::Outcome(_) => 3,
            FoldEvent::Decay(_) => 4,
        }
    }
    let decays = uses.windows(2).filter_map(|pair| {
        let idle_days = (pair[1] - pair[0]).num_days();
        // A reset between two uses forgets the earlier one: the gap spans no history.
        let spans_reset = resets
            .iter()
            .any(|reset| *reset > pair[0] && *reset <= pair[1]);
        (idle_days >= learning.decay_days && !spans_reset).then(|| {
            (
                pair[1],
                format!(
                    "decay after {idle_days} idle days (window {} days)",
                    learning.decay_days
                ),
            )
        })
    });
    let mut events: Vec<(DateTime<Utc>, FoldEvent)> = outcomes
        .iter()
        .map(|outcome| (outcome.at, FoldEvent::Outcome(outcome)))
        .chain(decays.map(|(at, cause)| (at, FoldEvent::Decay(cause))))
        .chain(pins.iter().flat_map(|pin| {
            std::iter::once((pin.start, FoldEvent::Pin(pin.level)))
                .chain(pin.end.map(|at| (at, FoldEvent::PinEnd)))
        }))
        .chain(resets.iter().map(|at| (*at, FoldEvent::Reset)))
        .collect();
    events.sort_by_key(|(at, event)| (*at, rank(event)));
    let mut level = baseline;
    let mut pinned: Option<Level> = None;
    let mut changes: Vec<LevelChange> = Vec::new();
    let mut recent: Vec<&Outcome> = Vec::new();
    for (at, event) in events {
        match event {
            FoldEvent::PinEnd => pinned = None,
            FoldEvent::Pin(to) => {
                pinned = Some(to);
                if to != level {
                    changes.push(LevelChange {
                        from: level,
                        to,
                        cause: format!("pin at {}", to.as_str()),
                        at,
                    });
                    level = to;
                }
            }
            FoldEvent::Reset => {
                recent.clear();
                // A pin, if any, still holds: a reset clears learning, not the owner's freeze.
                if pinned.is_none() && level != baseline {
                    changes.push(LevelChange {
                        from: level,
                        to: baseline,
                        cause: "reset".into(),
                        at,
                    });
                    level = baseline;
                }
            }
            FoldEvent::Outcome(outcome) => {
                recent.push(outcome);
                if recent.len() > learning.undo_window {
                    recent.remove(0);
                }
                if !matches!(outcome.kind, OutcomeKind::Undo) {
                    continue;
                }
                let window_undos = recent
                    .iter()
                    .filter(|outcome| matches!(outcome.kind, OutcomeKind::Undo))
                    .count();
                let to = if window_undos >= learning.repeat_undos {
                    Level::Ask
                } else {
                    level.below()
                };
                let cause = outcome.cause.clone().expect("an undo names its cause");
                hold_or_move(&mut level, pinned, &mut changes, to, cause, at);
            }
            FoldEvent::Decay(cause) => {
                let to = level.below();
                hold_or_move(&mut level, pinned, &mut changes, to, cause, at);
            }
        }
    }
    (level, changes)
}

/// One cell's trust from its runs and proposals, oldest first.
fn derive_cell(
    config: &Config,
    action: &str,
    team: &str,
    runs: &[&Run],
    proposals: &[&Filed],
    intents: &[Intent],
    resolution: &Resolution,
) -> Result<TrustCell> {
    let learning = &config.learning;
    let kinds: BTreeMap<&str, IntentKind> = intents
        .iter()
        .map(|intent| (intent.id.as_str(), intent.kind))
        .collect();
    let corrections = corrections(intents, resolution);
    let (pins, resets) = controls(intents, resolution, action, team);
    let outcomes = outcomes(runs, proposals, &kinds, &corrections);
    // A reset clears the learned history: only outcomes at or after the last reset count as
    // the cell's evidence. The fold still sees them all — the journal keeps every change it
    // derived, and the reset event clears the undo window at its time.
    let last_reset = resets.last().copied();
    let counted: Vec<&Outcome> = outcomes
        .iter()
        .filter(|outcome| last_reset.is_none_or(|reset| outcome.at >= reset))
        .collect();
    let clean: Vec<&Outcome> = counted
        .iter()
        .copied()
        .filter(|outcome| matches!(outcome.kind, OutcomeKind::Clean))
        .collect();
    let window: Vec<&Outcome> = counted
        .iter()
        .rev()
        .take(learning.undo_window)
        .copied()
        .collect();
    let undos = window
        .iter()
        .filter(|outcome| matches!(outcome.kind, OutcomeKind::Undo))
        .count();
    let span_days = match (clean.first(), clean.last()) {
        (Some(first), Some(last)) => (last.at - first.at).num_days(),
        _ => 0,
    };
    let eligible =
        clean.len() >= learning.clean_min && span_days >= learning.clean_days && undos == 0;
    // The baseline: the owner's rule raises the cell to Act; without one it sits at Propose.
    let baseline = if has_rule(config, action, team) {
        Level::Act
    } else {
        Level::Propose
    };
    // Demotion is automatic and fast: every undo, correction or decay drops the cell one level,
    // and repeated undos in its last outcomes drop it straight to Ask. The level never rises
    // from outcomes — promotion is the owner's rule or reset, never self-applied.
    let mut uses: Vec<DateTime<Utc>> = runs
        .iter()
        .map(|run| run.accepted_at)
        .chain(proposals.iter().map(|filed| filed.proposed_at))
        .collect();
    uses.sort();
    let (level, changes) = fold_level(baseline, learning, &outcomes, &uses, &pins, &resets);
    let demoted = level != baseline;
    let last_change = changes.last().cloned();
    let rule_text = (eligible && !demoted && !has_rule(config, action, team))
        .then(|| proposed_rule(action, team))
        .transpose()?;
    let reset_command =
        (eligible && demoted).then(|| format!("rastro trust reset {action} {team}"));
    let needs = if level == Level::Act {
        "Nothing to promote: the cell is at Act".to_string()
    } else if reset_command.is_some() {
        "Reset the cell to restore its baseline".to_string()
    } else if rule_text.is_some() {
        "Add the proposed rule to config.toml to raise the cell to Act".to_string()
    } else {
        format!(
            "{} clean outcomes over {} days with 0 undos in the last {}: now {} clean over \
             {span_days} days, {undos} undos",
            learning.clean_min,
            learning.clean_days,
            learning.undo_window,
            clean.len()
        )
    };
    Ok(TrustCell {
        action: action.into(),
        team: team.into(),
        level,
        clean: clean.len(),
        undos,
        changes,
        last_change,
        promotion: Promotion {
            eligible,
            needs,
            rule_text,
            reset_command,
        },
    })
}

impl<S: Storage> Rastro<S> {
    /// The trust view over every scope, on a writing open.
    pub fn trust(&mut self) -> Result<Trust> {
        let config = &self.config;
        self.storage
            .snapshot(|view| trust_view(config, Some(view), &[]))
    }

    /// Pins the cell `action` × `team` at `level`: the owner's intent holds the level against
    /// learning in both directions until a newer pin takes its place or the owner undoes it.
    /// A rebuild replays it. Returns the cell as it then reads.
    pub fn trust_pin(&mut self, action: &str, team: &str, level: Level) -> Result<TrustCell> {
        self.issue(|_, _| {
            let intent = identity::new_intent(
                IntentKind::Pin,
                String::new(),
                Targets::Cell {
                    version: identity::SHAPE_VERSION,
                    action: action.into(),
                    team: team.into(),
                    level: Some(level),
                },
                Payload::empty(),
                None,
            )?;
            Ok((vec![intent], None, false))
        })?;
        self.trust_cell(action, team)
    }

    /// Resets the cell `action` × `team`: the owner's intent clears its learned history back to
    /// its baseline — Act with a matching rule, Propose without. A rebuild replays it. Returns
    /// the cell as it then reads.
    pub fn trust_reset(&mut self, action: &str, team: &str) -> Result<TrustCell> {
        self.issue(|_, _| {
            let intent = identity::new_intent(
                IntentKind::Reset,
                String::new(),
                Targets::Cell {
                    version: identity::SHAPE_VERSION,
                    action: action.into(),
                    team: team.into(),
                    level: None,
                },
                Payload::empty(),
                None,
            )?;
            Ok((vec![intent], None, false))
        })?;
        self.trust_cell(action, team)
    }

    /// One cell's trust over every scope, derived even when no visible run touches it yet.
    fn trust_cell(&mut self, action: &str, team: &str) -> Result<TrustCell> {
        let config = &self.config;
        self.storage
            .snapshot(|view| derive_cell_from_view(config, view, &[], action, team))
    }

    /// The learning journal over every scope, on a writing open.
    pub fn trust_journal(&mut self) -> Result<TrustJournal> {
        let config = &self.config;
        self.storage
            .snapshot(|view| trust_journal(config, Some(view), &[]))
    }
}

impl<S: ReadStorage> Reader<S> {
    /// The trust view the reader's scopes see.
    pub fn trust(&mut self) -> Result<Trust> {
        let Self {
            locations,
            config,
            storage,
            scopes,
        } = self;
        crate::read_only::snapshot(locations, storage, scopes, |view, _| {
            trust_view(config, view, scopes)
        })
    }

    /// The learning journal the reader's scopes see.
    pub fn trust_journal(&mut self) -> Result<TrustJournal> {
        let Self {
            locations,
            config,
            storage,
            scopes,
        } = self;
        crate::read_only::snapshot(locations, storage, scopes, |view, _| {
            trust_journal(config, view, scopes)
        })
    }
}
