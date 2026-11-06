//! A blocking Linear GraphQL client that only sends the connector's fixed queries.
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Value, json};
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
    endpoint: String,
    key: String,
}
impl Client {
    pub fn new(base: &str, key: String) -> Self {
        // No redirects, so the key can't follow a response to another host.
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        Self {
            agent,
            endpoint: format!("{}/graphql", base.trim_end_matches('/')),
            key,
        }
    }

    /// Runs one query and returns its `data`. A response with GraphQL `errors` is a failure,
    /// even when it carries partial data.
    pub fn query(&self, query: &str, variables: Value) -> Result<Value, Failure> {
        let mut response = self
            .agent
            .post(&self.endpoint)
            // A personal API key goes as is, without a scheme.
            .header("Authorization", &self.key)
            .header("User-Agent", "rastro-connector-linear")
            .send_json(json!({"query": query, "variables": variables}))
            .map_err(|error| match error {
                ureq::Error::Timeout(_) => Failure::new("timeout"),
                ureq::Error::TooManyRedirects => Failure::new("protocol"),
                _ => Failure::new("network"),
            })?;
        let status = response.status().as_u16();
        let retry_at = retry_at(&|name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        });
        let body: Option<Value> = response.body_mut().read_json().ok();
        let code = body.as_ref().and_then(|b| {
            b["errors"]
                .as_array()
                .filter(|errors| !errors.is_empty())
                .map(|errors| errors[0]["extensions"]["code"].as_str().unwrap_or(""))
        });
        let failure = |reason: &'static str| match reason {
            "rate_limited" => Failure {
                reason,
                retry_at: retry_at.clone(),
            },
            _ => Failure::new(reason),
        };
        if let Some(code) = code {
            return Err(failure(match code {
                "AUTHENTICATION_ERROR" => "auth",
                "FORBIDDEN" => "forbidden",
                "RATELIMITED" => "rate_limited",
                _ if status == 401 => "auth",
                _ => "protocol",
            }));
        }
        match status {
            200..=299 => {}
            401 => return Err(failure("auth")),
            403 => return Err(failure("forbidden")),
            429 => return Err(failure("rate_limited")),
            500..=599 => return Err(failure("network")),
            _ => return Err(failure("protocol")),
        }
        match body {
            Some(Value::Object(mut body)) => match body.remove("data") {
                Some(data @ Value::Object(_)) => Ok(data),
                _ => Err(Failure::new("protocol")),
            },
            _ => Err(Failure::new("protocol")),
        }
    }
}

/// When to retry after a rate limit: Linear's reset time in epoch milliseconds, else
/// `Retry-After` in seconds.
fn retry_at(header: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let at = |t: DateTime<Utc>| t.to_rfc3339_opts(SecondsFormat::Secs, true);
    if let Some(ms) = header("x-ratelimit-requests-reset").and_then(|v| v.parse::<i64>().ok()) {
        return DateTime::from_timestamp_millis(ms).map(at);
    }
    header("retry-after")
        .and_then(|v| v.parse::<i64>().ok())
        .map(|seconds| at(Utc::now() + chrono::Duration::seconds(seconds)))
}

/// `LINEAR_API_KEY`, else the keychain item `rastro-linear`. Read at sync, never stored.
pub fn api_key() -> Option<String> {
    let from_env = std::env::var("LINEAR_API_KEY")
        .ok()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty());
    from_env.or_else(|| {
        let output = Command::new("security")
            .args(["find-generic-password", "-s", "rastro-linear", "-w"])
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        let key = String::from_utf8(output.stdout).ok()?.trim().to_string();
        (output.status.success() && !key.is_empty()).then_some(key)
    })
}

/// Whether `base` is an http(s) URL on a loopback host: `127.0.0.1`, `[::1]` or `localhost`.
/// The API base URL override exists for tests; it must never aim the key elsewhere.
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

    /// identity/R20: only loopback hosts are accepted as the API override
    #[test]
    fn only_loopback_hosts_are_accepted_so_the_key_stays_local() {
        for ok in [
            "http://127.0.0.1:8080",
            "http://localhost:1/x",
            "https://LOCALHOST",
            "http://[::1]:9",
        ] {
            assert!(loopback(ok), "{ok}");
        }
        for bad in [
            "https://api.linear.app",
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
