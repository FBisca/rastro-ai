//! `rastro plugin check <id>`: one synthetic connector run through the validation a sync
//! applies, storing nothing (decisions/plugins.md §51). The run binds the last 24 hours, the
//! manifest's first declared scope, a throwaway source name and the command-line options; the
//! connector starts with the environment a sync would give it. It is a CLI command, never an
//! MCP tool.
use super::{Session, discover, drive};
use crate::evidence::{Evidence, Grant};
use crate::runs::Silent;
use crate::{Appended, Counts, Locations, Scope, display_safe};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rastro_plugin_sdk::{
    ACTION_PROVIDER_KIND, AGENT_TOOL_ADAPTER_KIND, CoverageStatus, RunBinding, Window,
};
use serde::Serialize;
use std::collections::BTreeMap;

/// The kinds the core knows how to run or read today. A manifest declaring any other kind is
/// warned about, naming it; the warning never fails a run that has no violation.
const KNOWN_KINDS: [&str; 5] = [
    "connector",
    "extractor",
    "vocabulary",
    ACTION_PROVIDER_KIND,
    AGENT_TOOL_ADAPTER_KIND,
];

/// What one synthetic run found: the counts a sync would record, each protocol violation with
/// its reason, and a warning per manifest kind the core does not know.
#[derive(Debug, Serialize)]
pub struct PluginCheck {
    pub plugin_id: String,
    pub counts: Counts,
    /// The coverage the connector reported, when the conversation reached one.
    pub coverage: Option<CoverageStatus>,
    pub violations: Vec<String>,
    pub warnings: Vec<String>,
}
impl PluginCheck {
    /// The coverage as the owner reads it.
    pub fn coverage_label(&self) -> &'static str {
        match self.coverage {
            Some(CoverageStatus::Complete) => "complete",
            Some(CoverageStatus::CouldntLook) => "couldn't look",
            None => "missing",
        }
    }
}

/// Runs the installed connector `id` once with `options` as its source options, applying the
/// validation a sync applies and storing nothing. Never fails: a connector that is missing,
/// misdeclared or broken is a check with violations, and the caller exits unsuccessfully when
/// there is one.
pub fn run(locations: &Locations, id: &str, options: BTreeMap<String, String>) -> PluginCheck {
    let mut check = PluginCheck {
        plugin_id: id.into(),
        counts: Counts::default(),
        coverage: None,
        violations: Vec::new(),
        warnings: Vec::new(),
    };
    let plugin = match discover(locations, id, "connector") {
        Ok(plugin) => plugin,
        Err(reason) => {
            check.violations.push(match reason {
                "not_installed" => format!("plugin {id} is not installed"),
                _ => format!("plugin {id} fails its check"),
            });
            return check;
        }
    };
    for kind in &plugin.manifest.kinds {
        if !KNOWN_KINDS.contains(&kind.as_str()) {
            check.warnings.push(format!("unknown kind: {kind}"));
        }
    }
    if let Err(error) = plugin.manifest.check_source_options(&options) {
        check.violations.push(format!("{error:#}"));
        return check;
    }
    let Some(scope) = plugin.manifest.permissions.scopes.first() else {
        check
            .violations
            .push(format!("plugin {id} declares no scope to bind"));
        return check;
    };
    let to = Utc::now();
    let window = Window {
        from: stamp(to - Duration::hours(24)),
        to: stamp(to),
    };
    let binding = RunBinding {
        run_id: 0,
        source: "plugin-check".into(),
        scope: scope.clone(),
        selector: options,
        window: window.clone(),
        keep_original: false,
    };
    let grant = Grant::new(
        &plugin.manifest,
        &binding.source,
        &Scope(scope.clone()),
        &window,
        false,
    )
    .expect("the bound scope is the manifest's first declared one");
    let mut session = Session {
        counts: Counts::default(),
        coverage: None,
        observer: &mut Silent,
        violations: Vec::new(),
    };
    // The check's one difference from a sync: accepted evidence is counted, never stored.
    let mut append = |_evidence: &Evidence| Ok(Appended::New(0));
    let outcome = drive(&mut append, &plugin, &binding, &grant, &mut session);
    // What the run itself says, after the violations the conversation already named.
    let mut verdicts = Vec::new();
    match outcome {
        Ok(Ok(finish)) => {
            if finish.status != "ok" {
                verdicts.push(format!(
                    "run finished {}: {}",
                    finish.status,
                    display_safe(finish.reason.as_deref().unwrap_or("no reason given"))
                ));
            }
            match &session.coverage {
                None => verdicts.push("no coverage report".into()),
                Some(report) if report.status == CoverageStatus::CouldntLook => {
                    verdicts.push(format!(
                        "coverage: couldn't look ({})",
                        display_safe(report.reason.as_deref().unwrap_or("no reason given"))
                    ));
                }
                Some(_) => {}
            }
        }
        Ok(Err(reason)) if session.violations.is_empty() => {
            // The conversation never started: the connector command could not be launched.
            verdicts.push(match reason {
                "not_installed" => "the connector command could not be started".into(),
                other => other.to_string(),
            });
        }
        Ok(Err(_)) => {}
        Err(error) => verdicts.push(format!("storage error: {error:#}")),
    }
    check.counts = session.counts;
    check.coverage = session.coverage.map(|report| report.status);
    check.violations.extend(session.violations);
    check.violations.extend(verdicts);
    check
}

fn stamp(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Millis, true)
}
