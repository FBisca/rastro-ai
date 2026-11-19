//! The action provider role: which actions the installed providers offer (ADR 0015).
//!
//! A provider offers its actions only when its manifest passes
//! [`Manifest::check_action_provider`]: it is that kind alone, holds no `submit` permission and
//! declares each action's reach and reversibility. A provider that fails is refused, with the
//! reason on stderr, and none of its actions is offered.
use super::{
    Connector, Limits, Plugin, Wire, allowed_environment, check, find, handshake, notice, resolve,
    scan,
};
use crate::Locations;
use anyhow::{Result, bail};
use rastro_plugin_sdk::{
    ACTION_PROVIDER_KIND, ActionCall, ActionInitializeParams, ActionResult, Envelope, Manifest,
    PROTOCOL_VERSION, Role,
};
pub use rastro_plugin_sdk::{Reach, Reversibility};
use std::{
    process::Command,
    time::{Duration, Instant},
};

/// The most the core reads of a provider's answer, in one line.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
/// The id of the one request after `initialize`.
const REQUEST_ID: u64 = 2;

/// One action an installed, accepted provider offers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionOffer {
    pub action: String,
    /// The id of the provider plugin that declares it.
    pub provider: String,
    pub reach: Reach,
    pub reversibility: Reversibility,
}

/// The actions on offer and the providers refused, each with its reason.
#[derive(Debug, Default)]
pub struct Offers {
    pub offered: Vec<ActionOffer>,
    pub refused: Vec<String>,
}

/// Reads every installed manifest that declares the `action_provider` kind and sorts them into
/// offers and refusals, the first manifest of each plugin id only. A manifest of any other kind
/// is no business of this role, even one that declares actions: it is skipped here, and refused,
/// with the reason on stderr, when its own role loads it.
pub fn offered(locations: &Locations) -> Offers {
    let mut offers = Offers::default();
    let mut seen = std::collections::HashSet::new();
    scan(locations, |manifest, dir| {
        // First wins, as `find` resolves an id: a shadowed copy is never the plugin the host
        // runs, so it is neither offered nor refused.
        if !seen.insert(manifest.id.clone()) {
            return false;
        }
        let claims_role = manifest.kinds.iter().any(|k| k == ACTION_PROVIDER_KIND);
        if !claims_role {
            return false;
        }
        match accept(manifest, dir) {
            Ok((plugin, declared)) => {
                offers
                    .offered
                    .extend(declared.into_iter().map(|d| ActionOffer {
                        action: d.name,
                        provider: plugin.manifest.id.clone(),
                        reach: d.reach,
                        reversibility: d.reversibility,
                    }))
            }
            Err(reason) => {
                notice(&reason);
                offers.refused.push(reason);
            }
        }
        false
    });
    offers
}

/// The provider's check, with a refusal naming the plugin.
fn accept(
    manifest: Manifest,
    dir: std::path::PathBuf,
) -> std::result::Result<(Plugin, Vec<rastro_plugin_sdk::DeclaredAction>), String> {
    let plugin = check(manifest, dir, ACTION_PROVIDER_KIND).map_err(|e| format!("{e:#}"))?;
    let declared = plugin
        .manifest
        .check_action_provider()
        .map_err(|e| format!("{e:#}"))?;
    Ok((plugin, declared))
}

/// The offer of `action`. Fails naming the action when no installed provider offers it, and
/// carrying each refusal when a provider that declares it was refused.
pub fn offer(locations: &Locations, action: &str) -> Result<ActionOffer> {
    let offers = offered(locations);
    if let Some(found) = offers.offered.into_iter().find(|o| o.action == action) {
        return Ok(found);
    }
    if offers.refused.is_empty() {
        bail!("no installed action provider offers the action {action}");
    }
    bail!(
        "no installed action provider offers the action {action}; refused: {}",
        offers.refused.join("; ")
    )
}

/// Sends one action message, `method` with `call`, to the installed provider `id` and returns its
/// answer, within `timeout`.
///
/// The manifest is checked first, as for an offer: a refused provider is never started. The
/// provider starts with a cleared environment plus the basics, what its manifest lists (its own
/// credentials, which no other plugin is given) and `RASTRO_HOME` when the core has one.
///
/// # Errors
/// When the provider is not installed, is refused, can't start, exits early, answers with an
/// error or something that is not an action result, or runs out of time.
pub fn call(
    locations: &Locations,
    id: &str,
    method: &str,
    call: &ActionCall,
    timeout: Duration,
) -> Result<ActionResult> {
    let (manifest, dir) = find(locations, id).ok_or_else(|| {
        anyhow::anyhow!(
            "the action provider {id} is not installed; put its rastro-plugin.toml under {}",
            locations.plugins().display()
        )
    })?;
    let plugin = check(manifest, dir, ACTION_PROVIDER_KIND)
        .map_err(|error| anyhow::anyhow!("the action provider {id} is refused: {error:#}"))?;
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
    let Ok(mut provider) = Connector::launch(command, &plugin.manifest.id, limits) else {
        bail!("the action provider {id} can't start");
    };
    let outcome = converse(&mut provider.wire, &plugin, method, call, timeout);
    provider.teardown(outcome.is_ok());
    outcome
}

fn converse(
    wire: &mut Wire,
    plugin: &Plugin,
    method: &str,
    call: &ActionCall,
    timeout: Duration,
) -> Result<ActionResult> {
    let id = &plugin.manifest.id;
    let broke = |reason: &str| match reason {
        "protocol" => anyhow::anyhow!(
            "the action provider {id} sent a line that is not a JSON-RPC message, or one over {MAX_RESPONSE_BYTES} bytes"
        ),
        "timeout" => anyhow::anyhow!(
            "the action provider {id} did not answer within {} s and was stopped",
            timeout.as_secs()
        ),
        _ => anyhow::anyhow!(
            "the action provider {id} exited or closed its output before it answered"
        ),
    };
    let started = Instant::now();
    let init = ActionInitializeParams {
        protocol: PROTOCOL_VERSION.into(),
        manifest_id: id.clone(),
        role: Role::ActionProvider,
    };
    match handshake(wire, plugin, init)? {
        Ok(()) => {}
        Err("protocol") => bail!(
            "the action provider {id} did not answer `initialize` with this protocol and its manifest id"
        ),
        Err(reason) => return Err(broke(reason)),
    }
    wire.send(&Envelope::request(REQUEST_ID, method, call)?)
        .map_err(broke)?;
    let reply = wire.recv_by(started + timeout).map_err(broke)?;
    if reply.id != Some(REQUEST_ID.into()) || reply.method.is_some() {
        bail!("the action provider {id} sent something other than the response to `{method}`");
    }
    if let Some(error) = &reply.error {
        bail!(
            "the action provider {id} answered with an error: {}",
            crate::display_safe(error["message"].as_str().unwrap_or("no message"))
        );
    }
    reply.result::<ActionResult>().map_err(|error| {
        anyhow::anyhow!(
            "the action provider {id}'s answer is not an action result: {}",
            crate::display_safe(&format!("{error:#}"))
        )
    })
}
