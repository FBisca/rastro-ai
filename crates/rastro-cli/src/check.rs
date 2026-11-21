//! `rastro check <source>` (the catalog's `source.check`, also run as `rastro source check`):
//! whether a source's credentials are accepted. One cheap call through the connector; nothing
//! stored is read or written. It is CLI-only, so no MCP tool exposes it.
use rastro_core::{
    Config, Locations, Source, check::SourceCheck, check::check_source, display_safe,
};
use rastro_operations::envelope;

use crate::render;

/// The check as a block: the source and its result in the header, then the credential, the
/// reason and the fix, each on its own line when present.
pub(crate) fn line(check: &SourceCheck) -> String {
    let header = if check.authenticated {
        format!("Source {}: authenticated", display_safe(&check.source))
    } else {
        format!(
            "Source {}: could not authenticate ({})",
            display_safe(&check.source),
            check.result
        )
    };
    render::Block::new(header)
        .maybe("Credential", check.credential.as_deref())
        .maybe("Reason", check.reason.as_deref())
        .maybe("Fix", check.fix.as_deref())
        .to_string()
}

/// What source setup says after it saved `source`: the check's line.
pub fn setup_report(source: &Source) -> String {
    match Locations::resolve() {
        Ok(locations) => line(&check_source(&locations, source)),
        Err(error) => format!("Authentication not checked: {error:#}"),
    }
}

fn find(name: &str) -> anyhow::Result<(Locations, Source)> {
    let locations = Locations::resolve()?;
    let config = Config::load(&locations)?;
    let source = config
        .sources
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| anyhow::anyhow!("no source named {}", display_safe(name)))?;
    Ok((locations, source))
}

/// Runs the check and returns the process's exit code: 0 when authenticated, 1 otherwise.
pub fn run(name: &str, json: bool) -> i32 {
    match find(name) {
        Ok((locations, source)) => {
            let check = check_source(&locations, &source);
            if json {
                println!("{}", envelope::success("source.check", &check));
            } else {
                println!("{}", line(&check));
            }
            i32::from(!check.authenticated)
        }
        Err(error) => {
            if json {
                println!(
                    "{}",
                    envelope::failure("source.check", format!("{error:#}"))
                );
            } else {
                eprintln!("error: {error:#}");
            }
            1
        }
    }
}
