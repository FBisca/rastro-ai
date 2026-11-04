//! A blocking GitHub REST client that only issues fixed GET requests.
use chrono::{DateTime, SecondsFormat, Utc};
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

pub struct Page {
    pub body: Value,
    pub next: Option<String>,
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
            base: base.trim_end_matches('/').into(),
            token,
        }
    }

    /// One page of `path` with `query`; follow `next` with [`Client::get_url`].
    pub fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Page, Failure> {
        let mut request = self.agent.get(format!("{}{path}", self.base));
        for (name, value) in query {
            request = request.query(name, value);
        }
        self.send(request)
    }
    pub fn get_url(&self, url: &str) -> Result<Page, Failure> {
        // A `next` link on another host is not followed: the token stays with the API.
        if !url.starts_with(&format!("{}/", self.base)) {
            return Err(Failure::new("protocol"));
        }
        self.send(self.agent.get(url))
    }

    fn send(
        &self,
        request: ureq::RequestBuilder<ureq::typestate::WithoutBody>,
    ) -> Result<Page, Failure> {
        let mut response = request
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "rastro-connector-github")
            .call()
            .map_err(|error| match error {
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
        if !(200..300).contains(&status) {
            return Err(match status {
                401 => Failure::new("auth"),
                403 | 429 => rate_limit(&header).map_or_else(
                    || {
                        Failure::new(if status == 429 {
                            "rate_limited"
                        } else {
                            "forbidden"
                        })
                    },
                    |retry_at| Failure {
                        reason: "rate_limited",
                        retry_at,
                    },
                ),
                404 => Failure::new("not_found_or_hidden"),
                300..=399 => Failure::new("protocol"),
                _ => Failure::new("network"),
            });
        }
        let next = header("link").and_then(|link| next_link(&link));
        let body = response
            .body_mut()
            .read_json()
            .map_err(|_| Failure::new("protocol"))?;
        Ok(Page { body, next })
    }
}

/// `Some(retry_at)` when the response carries rate-limit headers.
fn rate_limit(header: &dyn Fn(&str) -> Option<String>) -> Option<Option<String>> {
    let at = |t: DateTime<Utc>| t.to_rfc3339_opts(SecondsFormat::Secs, true);
    if let Some(seconds) = header("retry-after").and_then(|v| v.parse::<i64>().ok()) {
        return Some(Some(at(Utc::now() + chrono::Duration::seconds(seconds))));
    }
    // The primary limit is spent when `x-ratelimit-remaining` is 0; `reset` alone isn't a signal.
    (header("x-ratelimit-remaining").as_deref() == Some("0")).then(|| {
        header("x-ratelimit-reset")
            .and_then(|v| v.parse::<i64>().ok())
            .and_then(|r| DateTime::from_timestamp(r, 0))
            .map(at)
    })
}

fn next_link(link: &str) -> Option<String> {
    link.split(',').find_map(|part| {
        let (url, rel) = part.split_once(';')?;
        rel.contains("rel=\"next\"").then(|| {
            url.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_string()
        })
    })
}

/// `GH_TOKEN`, else `GITHUB_TOKEN`, else `gh auth token`.
pub fn token() -> Option<String> {
    let from_env = ["GH_TOKEN", "GITHUB_TOKEN"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .map(|t| t.trim().to_string())
        .find(|t| !t.is_empty());
    from_env.or_else(|| {
        let output = Command::new("gh")
            .args(["auth", "token"])
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        let token = String::from_utf8(output.stdout).ok()?.trim().to_string();
        (output.status.success() && !token.is_empty()).then_some(token)
    })
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
mod tests {
    use super::loopback;

    /// evidence/R70: only loopback hosts are accepted
    #[test]
    #[allow(non_snake_case)]
    fn only_loopback_hosts_are_accepted() {
        for ok in [
            "http://127.0.0.1:8080",
            "http://127.0.0.1",
            "http://localhost:1/x",
            "https://LOCALHOST",
            "http://[::1]:9",
            "http://[::1]/path",
        ] {
            assert!(loopback(ok), "{ok}");
        }
        for bad in [
            "https://api.github.com",
            "http://127.0.0.1.example.com",
            "http://127.0.0.1@example.com",
            "http://localhost.example.com",
            "http://[::2]:1",
            "http://[::1]x",
            "http://192.168.0.1",
            "ftp://127.0.0.1",
            "127.0.0.1:80",
            "",
        ] {
            assert!(!loopback(bad), "{bad}");
        }
    }
}
