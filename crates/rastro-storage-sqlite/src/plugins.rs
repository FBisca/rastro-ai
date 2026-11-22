//! Per-plugin run facts for the trust view: the wall time of a plugin's finished runs and the
//! adapter turns they used. Connector and extractor runs live in `runs` by `plugin_id`; meeting
//! judgment runs live in `conversation_run` by `extractor`, and their turns are the `session_turn`
//! rows of the run's session. A meeting run's id is what a shown report names for the items its
//! content carried; a connector run's content names no queue item.
use crate::{View, parse_time, scope_parameter};
use anyhow::Result;
use rastro_core::{Scope, trust::PluginRuns};
use std::collections::BTreeMap;

#[derive(Default)]
struct Facts {
    wall_ms: i64,
    turns: usize,
    runs: Vec<String>,
}

pub(crate) fn plugin_runs(view: &View, scopes: &[Scope]) -> Result<Vec<PluginRuns>> {
    let scope = scope_parameter(scopes)?;
    let mut by_plugin: BTreeMap<String, Facts> = BTreeMap::new();
    // Connector and extractor runs: wall time only, and only of runs that finished.
    let mut statement = view.0.prepare(
        "SELECT plugin_id, started_at, finished_at FROM runs
         WHERE finished_at IS NOT NULL
           AND (?1 IS NULL OR scope IN (SELECT value FROM json_each(?1)))",
    )?;
    let rows = statement.query_map([&scope], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (plugin, started, finished) = row?;
        let facts = by_plugin.entry(plugin).or_default();
        facts.wall_ms += (parse_time(finished)? - parse_time(started)?).num_milliseconds();
    }
    // Meeting judgment runs: wall time plus the session's adapter turns, failed turns included.
    let mut statement = view.0.prepare(
        "SELECT mr.extractor, mr.id, mr.started_at, mr.finished_at,
                (SELECT count(*) FROM session_turn st WHERE st.run_id = mr.id)
         FROM conversation_run mr JOIN conversation m ON m.id = mr.conversation_id
         WHERE mr.extractor IS NOT NULL
           AND (?1 IS NULL OR m.scope IN (SELECT value FROM json_each(?1)))",
    )?;
    let rows = statement.query_map([&scope], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;
    for row in rows {
        let (plugin, run, started, finished, turns) = row?;
        let facts = by_plugin.entry(plugin).or_default();
        facts.wall_ms += (parse_time(finished)? - parse_time(started)?).num_milliseconds();
        facts.turns += turns.max(0) as usize;
        facts.runs.push(run.to_string());
    }
    Ok(by_plugin
        .into_iter()
        .map(|(plugin, facts)| PluginRuns {
            plugin,
            wall_ms: facts.wall_ms,
            turns: facts.turns,
            runs: facts.runs,
        })
        .collect())
}
