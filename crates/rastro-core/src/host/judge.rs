//! The agent tool adapter role: the core sends one `judgment.request` to an adapter plugin and
//! reads back its candidates (ADR 0012). The adapter reads untrusted text outside the core, so
//! the core refuses a manifest that does not declare the kind and its capabilities before it
//! starts anything, and treats the answer as data to check, never as instructions.
use super::{
    Connector, Limits, Plugin, Wire, allowed_environment, check, find, handshake, resolve,
};
use crate::Locations;
use rastro_plugin_sdk::{
    AGENT_TOOL_ADAPTER_KIND, AdapterInitializeParams, Envelope, JUDGMENT_REQUEST, JudgmentRequest,
    JudgmentResponse, PROTOCOL_VERSION, Role,
};
use std::{
    process::Command,
    time::{Duration, Instant},
};

/// The most the core reads of an adapter's answer, in one line.
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// The id of the one request after `initialize`.
const REQUEST_ID: u64 = 2;

/// Why a judgment produced no response.
#[derive(Debug)]
pub enum JudgmentFailure {
    /// The adapter was refused, could not start, ended early, answered with an error, or timed
    /// out.
    Failed(String),
    /// The adapter answered, but not in the shape the protocol expects.
    Invalid(String),
}

/// Asks the adapter plugin `id` to judge `request`, within `timeout`.
///
/// The manifest is checked first (protocol major, the kind `agent-tool-adapter`, the declared
/// agent-tool capabilities, no action permission): a refused adapter is never started. The
/// adapter starts with a cleared environment plus the basics, what its manifest lists, and
/// `RASTRO_HOME` when the core has one.
///
/// # Errors
/// [`JudgmentFailure::Failed`] when the adapter is not installed, is refused, can't start, exits
/// early, sends a JSON-RPC error or runs out of time; [`JudgmentFailure::Invalid`] when a line it
/// sends is not a JSON-RPC message, the answer is not the response shape, or it sends anything
/// but the one response.
pub fn judge(
    locations: &Locations,
    id: &str,
    timeout: Duration,
    request: &JudgmentRequest,
) -> Result<JudgmentResponse, JudgmentFailure> {
    let plugin = adapter(locations, id)?;
    let mut command = Command::new(resolve(&plugin.dir, &plugin.manifest.command));
    command
        .env_clear()
        .envs(allowed_environment(&plugin.manifest))
        .envs(std::env::var_os("RASTRO_HOME").map(|home| ("RASTRO_HOME", home)));
    let limits = Limits {
        max_line: MAX_RESPONSE_BYTES,
        silence: timeout,
        extraction_time: timeout,
        ..Limits::from_env()
    };
    let Ok(mut adapter) = Connector::launch(command, &plugin.manifest.id, limits) else {
        return Err(JudgmentFailure::Failed(format!(
            "the adapter {id} can't start"
        )));
    };
    let outcome = converse(&mut adapter.wire, &plugin, request, timeout);
    adapter.teardown(outcome.is_ok());
    outcome
}

/// The installed adapter `id`, if it passes its checks.
fn adapter(locations: &Locations, id: &str) -> Result<Plugin, JudgmentFailure> {
    let refused =
        |why: String| JudgmentFailure::Failed(format!("the adapter {id} is refused: {why}"));
    let (manifest, dir) = find(locations, id).ok_or_else(|| {
        JudgmentFailure::Failed(format!(
            "the adapter {id} is not installed; put its rastro-plugin.toml under {}",
            locations.plugins().display()
        ))
    })?;
    let plugin = check(manifest, dir, AGENT_TOOL_ADAPTER_KIND)
        .map_err(|error| refused(format!("{error:#}")))?;
    plugin
        .manifest
        .check_agent_tool()
        .map_err(|error| refused(format!("{error:#}")))?;
    Ok(plugin)
}

fn converse(
    wire: &mut Wire,
    plugin: &Plugin,
    request: &JudgmentRequest,
    timeout: Duration,
) -> Result<JudgmentResponse, JudgmentFailure> {
    let id = &plugin.manifest.id;
    let broke = |reason: &str| match reason {
        "protocol" => JudgmentFailure::Invalid(format!(
            "the adapter {id} sent a line that is not a JSON-RPC message, or one over {MAX_RESPONSE_BYTES} bytes"
        )),
        "timeout" => JudgmentFailure::Failed(format!(
            "the adapter {id} did not answer within {} s and was stopped",
            timeout.as_secs()
        )),
        _ => JudgmentFailure::Failed(format!(
            "the adapter {id} exited or closed its output before it answered"
        )),
    };
    let started = Instant::now();
    let init = AdapterInitializeParams {
        protocol: PROTOCOL_VERSION.into(),
        manifest_id: id.clone(),
        role: Role::AgentToolAdapter,
    };
    match handshake(wire, plugin, init) {
        Ok(Ok(())) => {}
        Ok(Err("protocol")) => {
            return Err(JudgmentFailure::Invalid(format!(
                "the adapter {id} did not answer `initialize` with this protocol and its manifest id"
            )));
        }
        Ok(Err(reason)) => return Err(broke(reason)),
        Err(error) => return Err(JudgmentFailure::Failed(format!("{error:#}"))),
    }
    let message = Envelope::request(REQUEST_ID, JUDGMENT_REQUEST, request)
        .map_err(|error| JudgmentFailure::Failed(format!("{error:#}")))?;
    wire.send(&message).map_err(broke)?;
    let reply = wire.recv_by(started + timeout).map_err(broke)?;
    if reply.id != Some(REQUEST_ID.into()) || reply.method.is_some() {
        return Err(JudgmentFailure::Invalid(format!(
            "the adapter {id} sent something other than the response to `judgment.request`"
        )));
    }
    if let Some(error) = &reply.error {
        return Err(JudgmentFailure::Failed(format!(
            "the adapter {id} answered with an error: {}",
            crate::display_safe(&error.to_string())
        )));
    }
    reply.result::<JudgmentResponse>().map_err(|error| {
        JudgmentFailure::Invalid(format!(
            "the adapter {id}'s response is not the judgment shape: {}",
            crate::display_safe(&format!("{error:#}"))
        ))
    })
}
