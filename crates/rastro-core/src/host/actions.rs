//! The action provider role: which actions the installed providers offer (ADR 0015).
//!
//! A provider offers its actions only when its manifest passes
//! [`Manifest::check_action_provider`]: it is that kind alone, holds no `submit` permission and
//! declares each action's reach and reversibility. A provider that fails is refused, with the
//! reason on stderr, and none of its actions is offered.
use super::{Plugin, check, notice, scan};
use crate::Locations;
use anyhow::{Result, bail};
use rastro_plugin_sdk::{ACTION_PROVIDER_KIND, Manifest};
pub use rastro_plugin_sdk::{Reach, Reversibility};

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
/// offers and refusals. A manifest of any other kind is no business of this role, even one that
/// declares actions: it is skipped here, and refused, with the reason on stderr, when its own
/// role loads it.
pub fn offered(locations: &Locations) -> Offers {
    let mut offers = Offers::default();
    scan(locations, |manifest, dir| {
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
