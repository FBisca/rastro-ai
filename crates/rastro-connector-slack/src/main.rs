//! The Slack connector: the messages of the one channel a source names, read with the owner's
//! own read-only user token.
//!
//! It speaks the Rastro plugin protocol on stdin/stdout. The token comes from `SLACK_USER_TOKEN`,
//! else the macOS keychain item `rastro-slack`, and is never stored.
mod api;

use anyhow::{Context, Result, bail};
use api::{Client, Failure};
use rastro_plugin_sdk::{
    COVERAGE_REPORT, CoverageReport, CoverageStatus, Envelope, InitializeParams, InitializeResult,
    PROTOCOL_VERSION, RUN_FINISH, RunBinding, RunFinish, Transport,
};
use serde_json::json;
use std::{
    collections::HashSet,
    io::{self, BufRead, Write},
};

const DEFAULT_API: &str = "https://slack.com/api";

/// The only scopes a Slack token may hold: the five read scopes, and no others.
const READ_SCOPES: [&str; 5] = [
    "channels:read",
    "channels:history",
    "groups:read",
    "groups:history",
    "users:read",
];

/// Whether the scopes Slack reports are exactly the read scopes; a refusal names what is
/// extra or missing.
fn scope_verdict(reported: &str) -> Result<(), String> {
    let reported: HashSet<&str> = reported
        .split(',')
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .collect();
    let mut extra: Vec<&str> = reported
        .iter()
        .copied()
        .filter(|scope| !READ_SCOPES.contains(scope))
        .collect();
    extra.sort_unstable();
    let missing: Vec<&str> = READ_SCOPES
        .iter()
        .copied()
        .filter(|scope| !reported.contains(scope))
        .collect();
    let mut problems = Vec::new();
    if !extra.is_empty() {
        problems.push(format!("holds extra scopes: {}", extra.join(", ")));
    }
    if !missing.is_empty() {
        problems.push(format!("is missing read scopes: {}", missing.join(", ")));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("the token {}", problems.join(" and ")))
    }
}

/// `--check`: one cheap authenticated call, and one line on stdout saying where the credential
/// came from and whether Slack accepted it with exactly the read scopes. Reads nothing else,
/// writes nothing, and never prints the token.
fn check() {
    let base = std::env::var("RASTRO_SLACK_API_URL").unwrap_or_else(|_| DEFAULT_API.into());
    let found = api::credential().filter(|(_, token)| token.bytes().all(|b| b.is_ascii_graphic()));
    let report = match found {
        None => json!({
            "result": "missing",
            "reason": "no Slack user token was found",
            "fix": "Set SLACK_USER_TOKEN in the environment rastro runs with, or store the \
                    token in the macOS keychain item rastro-slack",
        }),
        Some(_) if !api::loopback(&base) && base != DEFAULT_API => json!({
            "result": "failed",
            "reason": "RASTRO_SLACK_API_URL is not a loopback address",
            "fix": "Unset RASTRO_SLACK_API_URL",
        }),
        Some((credential, token)) => {
            let name = credential.name();
            let fix = format!(
                "Create a new user token from the shipped Slack app manifest template and \
                 update the {name}"
            );
            match Client::new(&base, token).auth_test() {
                Ok(Some(scopes)) => match scope_verdict(&scopes) {
                    Ok(()) => json!({"result": "authenticated", "credential": name}),
                    Err(problem) => json!({
                        "result": "rejected",
                        "credential": name,
                        "reason": problem,
                        "fix": fix,
                    }),
                },
                Ok(None) => json!({
                    "result": "failed",
                    "credential": name,
                    "reason": "Slack reported no scopes for the token",
                    "fix": "Run the check again",
                }),
                Err(Failure {
                    reason: "auth" | "forbidden",
                }) => json!({
                    "result": "rejected",
                    "credential": name,
                    "reason": "Slack rejected the user token",
                    "fix": fix,
                }),
                Err(failure) => json!({
                    "result": "failed",
                    "credential": name,
                    "reason": format!("could not reach Slack ({})", failure.reason),
                    "fix": "Check the network and run the check again",
                }),
            }
        }
    };
    println!("{report}");
}

fn main() -> Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--check") {
        check();
        return Ok(());
    }
    let mut transport = Transport::new(io::stdin().lock(), io::stdout().lock());
    let request = transport.read()?.context("core closed before initialize")?;
    let params: InitializeParams = request.params()?;
    transport.send(&Envelope::response(
        request.id,
        InitializeResult {
            protocol: PROTOCOL_VERSION.into(),
            manifest_id: params.manifest_id,
        },
    )?)?;

    let mut submitted = 0;
    let outcome = sync(&mut transport, &params.run, &mut submitted);
    let failure = outcome.err();
    let report = CoverageReport {
        gaps: Vec::new(),
        status: if failure.is_none() {
            CoverageStatus::Complete
        } else {
            CoverageStatus::CouldntLook
        },
        selection: None,
        reason: failure.map(str::to_string),
        retry_at: None,
        source: None,
        scope: None,
        window: None,
    };
    transport.send(&Envelope::notification(COVERAGE_REPORT, report)?)?;
    if let Some(reason) = &failure {
        eprintln!("slack connector: could not look: {reason}");
    }
    let finish = RunFinish {
        status: if failure.is_none() { "ok" } else { "failed" }.into(),
        submitted,
        reason: failure.map(str::to_string),
    };
    transport.send(&Envelope::request(1, RUN_FINISH, finish)?)?;
    if transport.read()?.is_none() {
        bail!("core closed before answering run.finish");
    }
    Ok(())
}

/// Reading is not built yet: a run fails typed rather than claiming coverage it never read.
fn sync<R: BufRead, W: Write>(
    _transport: &mut Transport<R, W>,
    _binding: &RunBinding,
    _submitted: &mut u64,
) -> Result<(), &'static str> {
    Err("protocol")
}
#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::scope_verdict;

    /// The five read scopes, as the spec names them: the expected set is not derived from the
    /// code under test.
    const FIVE: &str = "channels:read,channels:history,groups:read,groups:history,users:read";

    /// spec: The Slack check accepts only the read scopes
    #[test]
    fn the_read_scope_set_is_enforced_exactly() {
        assert!(scope_verdict(FIVE).is_ok(), "{FIVE}");
        let extra = scope_verdict(&format!("{FIVE},chat:write")).unwrap_err();
        assert!(extra.contains("chat:write"), "{extra}");
        let missing =
            scope_verdict("channels:read,channels:history,groups:read,users:read").unwrap_err();
        assert!(missing.contains("groups:history"), "{missing}");
        // Both at once: each is named.
        let both = scope_verdict("channels:read,chat:write").unwrap_err();
        assert!(
            both.contains("chat:write") && both.contains("groups:history"),
            "{both}"
        );
        // An empty report holds none of the read scopes.
        assert!(scope_verdict("").is_err());
        assert!(scope_verdict(" , ,").is_err());
        // Duplicates and stray whitespace are still exactly the five.
        let spaced = format!(" {FIVE} , channels:read ");
        assert!(scope_verdict(&spaced).is_ok(), "{spaced}");
    }
}
