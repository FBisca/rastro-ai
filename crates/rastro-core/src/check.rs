//! Checking a source's credentials: one cheap authenticated call through its connector, and a
//! report of where the credential came from and what the provider said. It reads no stored state
//! and writes none: no evidence, no coverage, no last complete check.
use crate::{Locations, Source, display_safe, host};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What the check of one source found.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct SourceCheck {
    pub source: String,
    pub connector: String,
    /// Whether the provider accepted the credential.
    pub authenticated: bool,
    /// The kind of failure, `authenticated` when there was none: `missing`, `rejected` or `failed`.
    pub result: String,
    /// Where the credential came from, by the name the owner would change; never its value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    /// What went wrong, when something did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// What the owner can do about it, when something went wrong.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

/// What a connector prints for `--check`.
#[derive(Deserialize)]
struct Answer {
    result: String,
    credential: Option<String>,
    reason: Option<String>,
    fix: Option<String>,
}

/// Checks `source` through its installed connector. Never fails: a connector that is missing,
/// broken or silent is a failed check with a fix.
pub fn check_source(locations: &Locations, source: &Source) -> SourceCheck {
    let id = source.connector.as_str();
    let failed = |reason: &str, fix: String| SourceCheck {
        source: source.name.clone(),
        connector: source.connector.clone(),
        authenticated: false,
        result: "failed".into(),
        credential: None,
        reason: Some(reason.into()),
        fix: Some(fix),
    };
    let plugin = match host::discover(locations, id, "connector") {
        Ok(plugin) => plugin,
        Err(_) => {
            return failed(
                &format!("the {id} connector is not installed or fails its check"),
                format!("Install the {id} connector plugin under the plugins directory"),
            );
        }
    };
    let answer = host::check::probe(&plugin)
        .and_then(|line| serde_json::from_str::<Answer>(&line).ok())
        .filter(|a| {
            matches!(
                a.result.as_str(),
                "authenticated" | "missing" | "rejected" | "failed"
            )
        });
    let Some(answer) = answer else {
        return failed(
            &format!("the {id} connector gave no answer to a credential check"),
            format!("Update the {id} connector"),
        );
    };
    SourceCheck {
        source: source.name.clone(),
        connector: source.connector.clone(),
        authenticated: answer.result == "authenticated",
        result: answer.result,
        credential: answer.credential.as_deref().map(display_safe),
        reason: answer.reason.as_deref().map(display_safe),
        fix: answer.fix.as_deref().map(display_safe),
    }
}
