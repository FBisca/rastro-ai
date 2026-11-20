//! The agent tool adapter role: the core opens one judgment session with an adapter plugin
//! and sends the turns of one capped run over it (ADR 0012, ADR-0016). The adapter reads
//! untrusted text outside the core, so the core refuses a manifest that does not declare the
//! kind and its capabilities before it starts anything, and treats every answer as data to
//! check, never as instructions. The session's process lives exactly as long as its run: no
//! process outlives it (ADR-0016 §4).
use super::{Connector, Limits, Plugin, allowed_environment, check, find, handshake, resolve};
use crate::Locations;
use rastro_plugin_sdk::{
    AGENT_TOOL_ADAPTER_KIND, AdapterInitializeParams, Envelope, JUDGMENT_FLOOR, JUDGMENT_REQUEST,
    JudgmentRequest, PROTOCOL_VERSION, Role,
};
use serde_json::Value;
use std::{
    process::Command,
    time::{Duration, Instant},
};

/// The most the core reads of an adapter's answer, in one line.
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// Why a judgment turn produced no answer to check.
#[derive(Debug)]
pub enum JudgmentFailure {
    /// The adapter was refused, could not start, ended early, answered with an error, or timed
    /// out.
    Failed(String),
    /// The adapter answered, but not in the shape the protocol expects.
    Invalid(String),
}

/// One judgment session: one checked, initialized adapter process that answers the turns of
/// one run. The core sends each turn as a `judgment.request` over the same conversation; a
/// retry turn carries its session object, and the adapter resumes what the first turn began.
/// Close it when the run ends: a clean finish lets the process exit on its own, a failed run
/// kills it.
pub struct JudgmentSession {
    adapter: Connector,
    id: String,
    timeout: Duration,
    next_request: u64,
}
impl JudgmentSession {
    /// Opens a session with the adapter plugin `id`, each of its turns bounded by `timeout`.
    ///
    /// The manifest is checked first (protocol major, the kind `agent-tool-adapter`, the
    /// declared agent-tool capabilities, no action permission, and the judgment floor's controls
    /// it declares it can enforce): a refused adapter is never started. The adapter starts with a
    /// cleared environment plus the basics, what its manifest lists, and `RASTRO_HOME` when the
    /// core has one, and answers `initialize` before any turn.
    ///
    /// # Errors
    /// [`JudgmentFailure::Failed`] when the adapter is not installed, is refused, can't start,
    /// exits early or runs out of time; [`JudgmentFailure::Invalid`] when it does not answer
    /// `initialize` with this protocol and its manifest id.
    pub fn open(
        locations: &Locations,
        id: &str,
        timeout: Duration,
    ) -> Result<Self, JudgmentFailure> {
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
        let init = AdapterInitializeParams {
            protocol: PROTOCOL_VERSION.into(),
            manifest_id: plugin.manifest.id.clone(),
            role: Role::AgentToolAdapter,
        };
        let id = plugin.manifest.id.clone();
        let failure = match handshake(&mut adapter.wire, &plugin, init) {
            Ok(Ok(())) => None,
            Ok(Err("protocol")) => Some(JudgmentFailure::Invalid(format!(
                "the adapter {id} did not answer `initialize` with this protocol and its manifest id"
            ))),
            Ok(Err(reason)) => Some(broke(&id, reason, timeout)),
            Err(error) => Some(JudgmentFailure::Failed(format!("{error:#}"))),
        };
        if let Some(failure) = failure {
            adapter.teardown(false);
            return Err(failure);
        }
        Ok(Self {
            adapter,
            id,
            timeout,
            next_request: 2,
        })
    }

    /// Sends one turn of the session and reads its answer: the raw `result` JSON, for the
    /// core's own gates to check. The turn gets the full per-turn timeout, however long the
    /// turns before it took.
    ///
    /// # Errors
    /// [`JudgmentFailure::Failed`] when the adapter exits early, sends a JSON-RPC error or
    /// runs out of time; [`JudgmentFailure::Invalid`] when a line it sends is not a JSON-RPC
    /// message, or it sends anything but the response to this turn's `judgment.request`.
    pub fn turn(&mut self, request: &JudgmentRequest) -> Result<Value, JudgmentFailure> {
        let request_id = self.next_request;
        self.next_request += 1;
        let message = Envelope::request(request_id, JUDGMENT_REQUEST, request)
            .map_err(|error| JudgmentFailure::Failed(format!("{error:#}")))?;
        let broke = |reason: &str| broke(&self.id, reason, self.timeout);
        self.adapter.wire.send(&message).map_err(broke)?;
        let reply = self
            .adapter
            .wire
            .recv_by(Instant::now() + self.timeout)
            .map_err(broke)?;
        if reply.id != Some(request_id.into()) || reply.method.is_some() {
            return Err(JudgmentFailure::Invalid(format!(
                "the adapter {} sent something other than the response to `judgment.request`",
                self.id
            )));
        }
        if let Some(error) = &reply.error {
            return Err(JudgmentFailure::Failed(format!(
                "the adapter {} answered with an error: {}",
                self.id,
                crate::display_safe(&error.to_string())
            )));
        }
        reply.result.ok_or_else(|| {
            JudgmentFailure::Invalid(format!("the adapter {} answered without a result", self.id))
        })
    }

    /// Ends the session's process: after a clean finish it gets the shutdown deadline to exit
    /// on its own; a failed run kills its whole process group.
    pub fn close(self, succeeded: bool) {
        self.adapter.teardown(succeeded);
    }
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
    // The judgment floor is non-negotiable (ADR-0016 §6): a manifest that cannot enforce one of
    // its controls refuses the run naming it, before the adapter starts. There is no
    // degraded-security mode.
    let missing: Vec<&str> = JUDGMENT_FLOOR
        .iter()
        .filter(|control| !plugin.manifest.enforcement.iter().any(|c| c == *control))
        .copied()
        .collect();
    if !missing.is_empty() {
        return Err(refused(format!(
            "the judgment floor requires {}, which its manifest does not declare it can enforce",
            missing.join(", ")
        )));
    }
    Ok(plugin)
}

/// A conversation that broke before an answer arrived.
fn broke(id: &str, reason: &str, timeout: Duration) -> JudgmentFailure {
    match reason {
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
    }
}
