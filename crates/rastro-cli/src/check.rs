//! `rastro check <source>`: whether a source's credentials are accepted. One cheap call through
//! the connector; nothing stored is read or written. It is a CLI command only, never an
//! operation, so no MCP tool exposes it.
use rastro_core::{
    Config, Locations, Source, check::SourceCheck, check::check_source, display_safe,
};
use rastro_operations::envelope;

fn line(check: &SourceCheck) -> String {
    let mut text = format!("Source {}: ", display_safe(&check.source));
    if check.authenticated {
        text += "authenticated";
    } else {
        text += &format!("could not authenticate ({})", check.result);
    }
    if let Some(credential) = &check.credential {
        text += &format!(" | Credential: {credential}");
    }
    if let Some(reason) = &check.reason {
        text += &format!(" | Reason: {reason}");
    }
    if let Some(fix) = &check.fix {
        text += &format!(" | Fix: {fix}");
    }
    text
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
