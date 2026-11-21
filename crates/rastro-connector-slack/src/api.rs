//! A blocking Slack Web API client for the connector's fixed calls, the credential it reads,
//! and the typed "couldn't look" reasons.
use chrono::{SecondsFormat, Utc};
use serde_json::Value;
use std::{process::Command, time::Duration};

/// Why a read failed: a typed "couldn't look" reason, never raw error text.
#[derive(Debug)]
pub struct Failure {
    pub reason: &'static str,
    pub retry_at: Option<String>,
}
impl Failure {
    pub fn new(reason: &'static str) -> Self {
        Self {
            reason,
            retry_at: None,
        }
    }
}

pub struct Client {
    agent: ureq::Agent,
    base: String,
    token: String,
}
impl Client {
    pub fn new(base: &str, token: String) -> Self {
        // No redirects, so the token can't follow a response to another host.
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        Self {
            agent,
            base: base.trim_end_matches('/').to_string(),
            token,
        }
    }

    /// `auth.test`: the one cheap authenticated call. Returns the scopes Slack reports for the
    /// token in `X-OAuth-Scopes`, or `None` when the answer carries no such header.
    pub fn auth_test(&self) -> Result<Option<String>, Failure> {
        let (_, scopes) = self.get("auth.test", &[])?;
        Ok(scopes)
    }

    /// `conversations.info`: the conversation's flags, read before any history.
    pub fn conversations_info(&self, channel: &str) -> Result<Value, Failure> {
        let (body, _) = self.get("conversations.info", &[("channel", channel)])?;
        Ok(body)
    }

    /// One page of `conversations.history`, with the call's fixed params.
    pub fn conversations_history(&self, params: &[(&str, &str)]) -> Result<Value, Failure> {
        let (body, _) = self.get("conversations.history", params)?;
        Ok(body)
    }

    /// Whose token it is: `auth.test`'s `user_id`.
    pub fn viewer_id(&self) -> Result<String, Failure> {
        let (body, _) = self.get("auth.test", &[])?;
        body["user_id"]
            .as_str()
            .map(str::to_string)
            .ok_or(Failure::new("protocol"))
    }

    /// `users.info`: one author's profile.
    pub fn users_info(&self, user: &str) -> Result<Value, Failure> {
        let (body, _) = self.get("users.info", &[("user", user)])?;
        Ok(body)
    }

    /// One fixed call. Slack answers `ok: false` with an `error` code even on HTTP 200; both
    /// the code and the status map to typed reasons, never raw error text.
    fn get(
        &self,
        method: &str,
        params: &[(&str, &str)],
    ) -> Result<(Value, Option<String>), Failure> {
        let mut request = self
            .agent
            .get(&format!("{}/{method}", self.base))
            .header("Authorization", &format!("Bearer {}", self.token))
            .header("User-Agent", "rastro-connector-slack");
        for (name, value) in params {
            request = request.query(name, value);
        }
        let mut response = request.call().map_err(|error| match error {
            ureq::Error::Timeout(_) => Failure::new("timeout"),
            ureq::Error::TooManyRedirects => Failure::new("protocol"),
            _ => Failure::new("network"),
        })?;
        let status = response.status().as_u16();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let scopes = header("x-oauth-scopes");
        let retry_at = retry_at(&header);
        let body: Option<Value> = response.body_mut().read_json().ok();
        let failure = |reason: &'static str| match reason {
            "rate_limited" => Failure {
                reason,
                retry_at: retry_at.clone(),
            },
            _ => Failure::new(reason),
        };
        if status == 429 {
            return Err(failure("rate_limited"));
        }
        if let Some(error) = body.as_ref().and_then(|b| b["error"].as_str()) {
            return Err(failure(match error {
                "invalid_auth" | "not_authed" | "token_revoked" | "token_expired"
                | "account_inactive" => "auth",
                "missing_scope" => "forbidden",
                "channel_not_found" | "not_in_channel" | "thread_not_found" => {
                    "not_found_or_hidden"
                }
                "ratelimited" => "rate_limited",
                _ => "protocol",
            }));
        }
        match status {
            200..=299 => {}
            500..=599 => return Err(failure("network")),
            _ => return Err(failure("protocol")),
        }
        match body {
            Some(body) if body["ok"] == true => Ok((body, scopes)),
            _ => Err(Failure::new("protocol")),
        }
    }
}

/// When to retry after a rate limit: `Retry-After` in seconds from the response. The connector
/// reports the time instead of waiting: the host kills a connector silent for 60 seconds.
fn retry_at(header: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    header("retry-after")
        .and_then(|v| v.parse::<i64>().ok())
        .map(|seconds| {
            (Utc::now() + chrono::Duration::seconds(seconds))
                .to_rfc3339_opts(SecondsFormat::Millis, true)
        })
}

/// Where the user token came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Credential {
    Environment,
    Keychain,
}
impl Credential {
    /// What the owner would change to replace this credential; never its value.
    pub fn name(self) -> &'static str {
        match self {
            Self::Environment => "SLACK_USER_TOKEN",
            Self::Keychain => "keychain item rastro-slack",
        }
    }
}

/// `SLACK_USER_TOKEN`, else the keychain item `rastro-slack`, with where it came from. Read at
/// check and sync, never stored.
pub fn credential() -> Option<(Credential, String)> {
    let from_env = std::env::var("SLACK_USER_TOKEN")
        .ok()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty());
    if let Some(token) = from_env {
        return Some((Credential::Environment, token));
    }
    let output = Command::new("security")
        .args(["find-generic-password", "-s", "rastro-slack", "-w"])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    let token = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (output.status.success() && !token.is_empty()).then_some((Credential::Keychain, token))
}

/// Whether `base` is an http(s) URL on a loopback host: `127.0.0.1`, `[::1]` or `localhost`.
/// The API base URL override exists for tests; it must never aim the token elsewhere.
pub fn loopback(base: &str) -> bool {
    let Some(rest) = base
        .strip_prefix("http://")
        .or_else(|| base.strip_prefix("https://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // Userinfo would let `127.0.0.1@elsewhere` pass for a prefix match.
    if authority.contains('@') {
        return false;
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => match v6.split_once(']') {
            Some((host, port)) if port.is_empty() || port.starts_with(':') => host,
            _ => return false,
        },
        None => authority.split(':').next().unwrap_or_default(),
    };
    matches!(
        host.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "::1" | "localhost"
    )
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::loopback;

    /// Only loopback hosts are accepted as the API override
    #[test]
    fn only_loopback_hosts_are_accepted_so_the_token_stays_local() {
        for ok in [
            "http://127.0.0.1:8080",
            "http://localhost:1/x",
            "https://LOCALHOST",
            "http://[::1]:9",
        ] {
            assert!(loopback(ok), "{ok}");
        }
        for bad in [
            "https://slack.com/api",
            "http://127.0.0.1.example.com",
            "http://127.0.0.1@example.com",
            "http://localhost.example.com",
            "http://[::1]x",
            "ftp://127.0.0.1",
            "127.0.0.1:80",
            "",
        ] {
            assert!(!loopback(bad), "{bad}");
        }
    }
}
