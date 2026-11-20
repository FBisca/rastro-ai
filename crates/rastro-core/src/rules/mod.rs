//! Owner rules: what the owner lets Rastro do without asking, written as `[[rules]]` in
//! `config.toml` and checked on load.
//!
//! A rule names an action, a team and the assignee `me`, and may carry its own `grace`. A rule's
//! grace window is its own `grace` when set, else its action's sensitivity in `[grace]`, else the
//! default table: `linear.create_issue` is medium, 10 minutes. A rule may also set a `daily_cap`:
//! at most that many Act runs per local calendar day, counting every run the rule started. A rule
//! that fails a check fails the command that read the config, naming the rule and the bad field.
use crate::{
    Config, Locations, Rastro, ReadStorage, Reader, Scope, Snapshot, Storage, actions, host,
};
use anyhow::{Result, bail};
use chrono::{DateTime, Local, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashSet},
    time::Duration,
};

/// One `[[rules]]` entry, as the owner wrote it. A field the owner left out stays `None` until
/// [`validate`] names it.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// This rule's own grace window, such as `"1m"`; the `[grace]` table's when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grace: Option<String>,
    /// At most this many Act runs a day, counted per local calendar day over every run the rule
    /// started; no cap when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_cap: Option<u32>,
}

/// `[grace]`: how long a rule waits, by its action's sensitivity, before it runs. Each omitted
/// entry keeps its default.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraceTable {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medium: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long: Option<String>,
}
impl GraceTable {
    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// How long an action waits before it runs, named by what it costs when wrong.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Sensitivity {
    Short,
    Medium,
    Long,
}
impl Sensitivity {
    const ALL: [Self; 3] = [Self::Short, Self::Medium, Self::Long];
    fn default_grace(self) -> Duration {
        Duration::from_secs(match self {
            Self::Short => 2 * 60,
            Self::Medium => 10 * 60,
            Self::Long => 60 * 60,
        })
    }
    fn key(self) -> &'static str {
        match self {
            Self::Short => "short",
            Self::Medium => "medium",
            Self::Long => "long",
        }
    }
}

/// The sensitivity of an action a rule can act on, or `None` for an action Rastro can't run by
/// rule.
fn sensitivity(action: &str) -> Option<Sensitivity> {
    (action == actions::LINEAR_CREATE_ISSUE).then_some(Sensitivity::Medium)
}

/// A duration the owner wrote: whole seconds, minutes or hours, as `30s`, `3m` or `1h`.
fn parse_duration(text: &str) -> Result<Duration, String> {
    let bad = || {
        format!(
            "{text:?} is not a duration: write whole seconds, minutes or hours, such as \"30s\", \"3m\" or \"1h\""
        )
    };
    let text = text.trim();
    let split = text.len().saturating_sub(1);
    let (number, unit) = text.split_at_checked(split).ok_or_else(bad)?;
    let unit = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        _ => return Err(bad()),
    };
    match number.parse::<u64>().ok().filter(|number| *number > 0) {
        Some(number) => number
            .checked_mul(unit)
            .map(Duration::from_secs)
            .ok_or_else(bad),
        None => Err(bad()),
    }
}

impl GraceTable {
    /// The window `sensitivity` waits: the owner's entry, else the default.
    fn window(&self, sensitivity: Sensitivity) -> Result<Duration, String> {
        let entry = match sensitivity {
            Sensitivity::Short => &self.short,
            Sensitivity::Medium => &self.medium,
            Sensitivity::Long => &self.long,
        };
        match entry {
            Some(text) => parse_duration(text),
            None => Ok(sensitivity.default_grace()),
        }
    }
}

/// Checks `[grace]` and every rule, naming the rule and the field that fails. The action must be
/// one an installed provider declares and Rastro can run by rule. The providers are scanned only
/// when a rule passed the cheaper checks.
pub(crate) fn validate(config: &Config, locations: &Locations) -> Result<()> {
    for sensitivity in Sensitivity::ALL {
        if let Err(reason) = config.grace.window(sensitivity) {
            bail!("[grace] {}: {reason}", sensitivity.key());
        }
    }
    let mut names = HashSet::new();
    for rule in &config.rules {
        let name = &rule.name;
        if name.trim().is_empty() {
            bail!("a rule has an empty name: give each [[rules]] entry a `name`");
        }
        if !names.insert(name.as_str()) {
            bail!("rule {name}: the field `name` repeats: each rule needs its own name");
        }
        let present = |field: &'static str, value: &Option<String>| -> Result<()> {
            match value.as_deref() {
                Some(text) if !text.trim().is_empty() => Ok(()),
                _ => bail!("rule {name}: the field `{field}` is missing"),
            }
        };
        present("action", &rule.action)?;
        present("team", &rule.team)?;
        present("assignee", &rule.assignee)?;
        if rule.assignee.as_deref() != Some(actions::OWNER) {
            bail!(
                "rule {name}: the field `assignee` must be \"{}\": a rule acts for the owner only",
                actions::OWNER
            );
        }
        if let Some(text) = &rule.grace
            && let Err(reason) = parse_duration(text)
        {
            bail!("rule {name}: the field `grace` {reason}");
        }
    }
    if config.rules.is_empty() {
        return Ok(());
    }
    let offered = host::actions::offered(locations).offered;
    for rule in &config.rules {
        let action = rule.action.as_deref().unwrap_or_default();
        if !offered.iter().any(|offer| offer.action == action) {
            bail!(
                "rule {}: the field `action` names {action}, which no installed action provider declares",
                rule.name
            );
        }
        if sensitivity(action).is_none() {
            bail!(
                "rule {}: the field `action` names {action}, which Rastro can't run by rule yet",
                rule.name
            );
        }
    }
    Ok(())
}

impl Config {
    /// The first rule that matches `proposal` on action, team and assignee. A rule acts for the
    /// owner only, so a proposal naming another assignee matches none.
    pub(crate) fn matching_rule(&self, proposal: &actions::Proposal) -> Option<&Rule> {
        self.rules.iter().find(|rule| {
            rule.action.as_deref() == Some(proposal.action.as_str())
                && rule.team.as_deref() == Some(proposal.fields.team.as_str())
                && rule.assignee.as_deref() == Some(proposal.fields.assignee.as_str())
        })
    }

    /// The rule as a run stores it: its match and resolved grace, as they are now.
    pub(crate) fn rule_copy(&self, rule: &Rule) -> RuleCopy {
        RuleCopy {
            name: rule.name.clone(),
            action: rule.action.clone().unwrap_or_default(),
            team: rule.team.clone().unwrap_or_default(),
            assignee: rule.assignee.clone().unwrap_or_default(),
            grace_seconds: self.grace_of(rule).as_secs(),
            daily_cap: rule.daily_cap,
        }
    }

    /// The grace window of a rule: its own `grace`, else its action's sensitivity in `[grace]`,
    /// else the default table. A loaded config has passed [`validate`], so the text parses.
    pub(crate) fn grace_of(&self, rule: &Rule) -> Duration {
        let window = rule
            .grace
            .as_deref()
            .map(parse_duration)
            .or_else(|| {
                rule.action
                    .as_deref()
                    .and_then(sensitivity)
                    .map(|sensitivity| self.grace.window(sensitivity))
            })
            .unwrap_or_else(|| Ok(Sensitivity::Medium.default_grace()));
        window.unwrap_or_else(|_| Sensitivity::Medium.default_grace())
    }
}

/// The Act runs `rule` started on `now`'s local calendar day, every status counted: a stopped,
/// returned or undone run still used the day's budget.
pub(crate) fn used_today(runs: &[actions::Run], rule: &Rule, now: DateTime<Utc>) -> usize {
    let today = now.with_timezone(&Local).date_naive();
    runs.iter()
        .filter(|run| {
            run.level == actions::RunLevel::Act
                && run.rule.as_ref().is_some_and(|copy| copy.name == rule.name)
                && run.accepted_at.with_timezone(&Local).date_naive() == today
        })
        .count()
}

/// The rule as it stood when a run started under it: copied into the run, so a later change to
/// `config.toml` never revises what authorized it.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "ActionRule")]
pub struct RuleCopy {
    pub name: String,
    pub action: String,
    pub team: String,
    pub assignee: String,
    /// How long the run waits before it executes, in seconds.
    pub grace_seconds: u64,
    /// The rule's daily cap when it set one; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_cap: Option<u32>,
}

/// One rule as `rules check` shows it: its match and grace, its cap and today's count, its
/// pending Acts and the queued proposals it matches.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "RuleCheck")]
pub struct RuleView {
    pub name: String,
    pub action: String,
    pub team: String,
    pub assignee: String,
    /// How long a run this rule starts waits before it executes, in seconds.
    pub grace_seconds: u64,
    /// The rule's daily cap when it set one; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_cap: Option<u32>,
    /// The Act runs the rule started on the read's local calendar day, every status counted: a
    /// stopped, returned or undone run still used the day's budget.
    pub used_today: usize,
    /// The rule's pending Act runs, oldest first, each with the time it runs and its stop
    /// command.
    pub pending: Vec<actions::Run>,
    /// The queued proposals the rule matches on action, team and assignee, oldest first: each
    /// waits in the queue with no pending Act of its own. A pending Act's proposal is listed
    /// under `pending`, a stopped Act's never returns to the queue, and a returned or undone
    /// Act's proposal waits here as a normal proposal.
    pub matches: Vec<actions::Filed>,
}

/// What `rules check` returns: every rule in `config.toml`, in the order the owner wrote them.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "RulesCheck")]
pub struct Check {
    pub rules: Vec<RuleView>,
}

/// What `rules check` reads from one snapshot: each rule's count, pending Acts and the queued
/// proposals it matches. The proposals are the ones the queue holds — an answer took the answered
/// ones out, and a stopped Act's proposal never returns — less the pending Acts' proposals, which
/// are listed under `pending`. A returned or undone Act's proposal waits in the queue, so a
/// matching one is listed here. Records nothing.
fn check(
    config: &Config,
    view: Option<&dyn Snapshot>,
    scopes: &[Scope],
    now: DateTime<Utc>,
) -> Result<Check> {
    let (runs, proposals, answered) = match view {
        Some(view) => (
            view.action_runs(scopes)?,
            view.action_proposals(scopes)?,
            crate::queue::answered_items(&view.intents()?),
        ),
        None => (Vec::new(), Vec::new(), BTreeSet::new()),
    };
    // The queue holds a proposal unless an answer took it out or its Act was stopped; a pending
    // Act's proposal is listed under `pending`, not here. Every other run — returned, undone,
    // done, failed — leaves its proposal waiting, so a matching one is listed.
    let mut listed_under_run: BTreeSet<&str> = BTreeSet::new();
    for run in &runs {
        if matches!(
            run.status,
            actions::RunStatus::Pending | actions::RunStatus::Stopped
        ) {
            listed_under_run.insert(run.proposal.as_str());
        }
    }
    let waiting: Vec<&actions::Filed> = proposals
        .iter()
        .filter(|filed| {
            !answered.contains(&filed.id) && !listed_under_run.contains(filed.id.as_str())
        })
        .collect();
    Ok(Check {
        rules: config
            .rules
            .iter()
            .map(|rule| RuleView {
                name: rule.name.clone(),
                action: rule.action.clone().unwrap_or_default(),
                team: rule.team.clone().unwrap_or_default(),
                assignee: rule.assignee.clone().unwrap_or_default(),
                grace_seconds: config.grace_of(rule).as_secs(),
                daily_cap: rule.daily_cap,
                used_today: used_today(&runs, rule, now),
                pending: runs
                    .iter()
                    .filter(|run| {
                        run.level == actions::RunLevel::Act
                            && run.status == actions::RunStatus::Pending
                            && run.rule.as_ref().is_some_and(|copy| copy.name == rule.name)
                    })
                    .cloned()
                    .collect(),
                matches: waiting
                    .iter()
                    .filter(|filed| {
                        config
                            .matching_rule(&filed.proposal)
                            .is_some_and(|matched| matched.name == rule.name)
                    })
                    .map(|filed| (*filed).clone())
                    .collect(),
            })
            .collect(),
    })
}

impl<S: Storage> Rastro<S> {
    /// Each rule with its match, grace, cap, today's count, pending Acts and the queued
    /// proposals it matches. Records nothing.
    pub fn rules_check(&mut self) -> Result<Check> {
        let now = crate::coverage::now()?;
        let config = &self.config;
        self.storage
            .snapshot(|view| check(config, Some(view), &[], now))
    }
}

impl<S: ReadStorage> Reader<S> {
    /// Each rule with its match, grace, cap, today's count, pending Acts and the queued
    /// proposals it matches, over the runs and proposals the reader's scopes see. With no
    /// database, every rule reads zero used, nothing pending and nothing matching. Records
    /// nothing.
    pub fn rules_check(&mut self) -> Result<Check> {
        let now = crate::coverage::now()?;
        let Self {
            locations,
            config,
            storage,
            scopes,
        } = self;
        crate::read_only::snapshot(locations, storage, scopes, |view, _| {
            check(config, view, scopes, now)
        })
    }
}
