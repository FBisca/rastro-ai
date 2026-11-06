//! The GitHub connector: issues, pull requests and comments involving the owner.
//!
//! It speaks the Rastro plugin protocol on stdin/stdout and calls only fixed GET endpoints
//! of the GitHub REST API. Author logins select profile requests only after validation;
//! `Link: rel="next"` pagination stays on the same host.
mod api;

use anyhow::{Context, Result, bail};
use api::{Client, Failure};
use chrono::{DateTime, Utc};
use rastro_plugin_sdk::{
    COVERAGE_REPORT, CoverageGap, CoverageReport, CoverageStatus, EVIDENCE_SUBMIT, Envelope,
    InitializeParams, InitializeResult, PROTOCOL_VERSION, RUN_FINISH, RunBinding, RunFinish,
    Transport,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::{self, BufRead, Write},
};

/// Where an observation came from: when its item or comment was updated, and that item or comment.
#[derive(Clone, Copy)]
struct Origin<'a> {
    updated: &'a str,
    original: &'a Value,
}

const DEFAULT_API: &str = "https://api.github.com";
/// GitHub's search API returns at most this many results per query.
const SEARCH_CAP: u64 = 1000;
const QUALIFIERS: [&str; 4] = ["author", "assignee", "review-requested", "mentions"];

struct Run<'a, R, W> {
    transport: &'a mut Transport<R, W>,
    client: Client,
    window_from: DateTime<Utc>,
    window_to: DateTime<Utc>,
    repo: String,
    seen: HashSet<u64>,
    profiles: HashMap<u64, Value>,
    submitted: u64,
    capped: bool,
    /// The host said the source keeps originals: attach the raw item or comment.
    keep_original: bool,
    gaps: Vec<CoverageGap>,
}

fn main() -> Result<()> {
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
    let mut gaps = Vec::new();
    let outcome = sync(&mut transport, &params.run, &mut submitted, &mut gaps);
    let failure = outcome.err();
    let report = CoverageReport {
        gaps,
        status: if failure.is_none() {
            CoverageStatus::Complete
        } else {
            CoverageStatus::CouldntLook
        },
        selection: Some("involving the owner".into()),
        reason: failure.as_ref().map(|f| f.reason.to_string()),
        retry_at: failure.as_ref().and_then(|f| f.retry_at.clone()),
        source: None,
        scope: None,
        window: None,
    };
    transport.send(&Envelope::notification(COVERAGE_REPORT, report)?)?;
    if let Some(failure) = &failure {
        eprintln!("github connector: could not look: {}", failure.reason);
    }
    let finish = RunFinish {
        status: if failure.is_none() { "ok" } else { "failed" }.into(),
        submitted,
        reason: failure.map(|f| f.reason.to_string()),
    };
    transport.send(&Envelope::request(1, RUN_FINISH, finish)?)?;
    if transport.read()?.is_none() {
        bail!("core closed before answering run.finish");
    }
    Ok(())
}

/// Reads everything involving the owner in the window. Evidence goes out as each page is
/// read, so a failure part-way keeps what was already submitted.
fn sync<R: BufRead, W: Write>(
    transport: &mut Transport<R, W>,
    binding: &RunBinding,
    submitted: &mut u64,
    gaps: &mut Vec<CoverageGap>,
) -> Result<(), Failure> {
    let repo = binding
        .selector
        .get("repo")
        .ok_or(Failure::new("protocol"))?
        .clone();
    // The override is for tests: it may only point at this machine, so no real token or
    // request can be sent anywhere else.
    let base = match std::env::var("RASTRO_GITHUB_API_URL") {
        Ok(base) if api::loopback(&base) => base,
        Ok(_) => return Err(Failure::new("protocol")),
        Err(_) => DEFAULT_API.into(),
    };
    let token = api::token().ok_or(Failure::new("auth"))?;
    let mut run = Run {
        transport,
        client: Client::new(&base, token),
        window_from: binding
            .window
            .from
            .parse()
            .map_err(|_| Failure::new("protocol"))?,
        window_to: binding
            .window
            .to
            .parse()
            .map_err(|_| Failure::new("protocol"))?,
        repo,
        seen: HashSet::new(),
        profiles: HashMap::new(),
        submitted: 0,
        capped: false,
        keep_original: binding.keep_original,
        gaps: Vec::new(),
    };
    let result = run.read_all(&binding.window.from, &binding.window.to);
    *submitted = run.submitted;
    *gaps = std::mem::take(&mut run.gaps);
    result?;
    if run.capped {
        return Err(Failure::new("result_cap"));
    }
    Ok(())
}

impl<R: BufRead, W: Write> Run<'_, R, W> {
    /// An author profile couldn't be read: reported once per reason, and the run goes on (D45).
    fn profile_gap(&mut self, reason: &str) {
        if !self
            .gaps
            .iter()
            .any(|g| g.selection == "author profiles" && g.reason == reason)
        {
            self.gaps.push(CoverageGap {
                selection: "author profiles".into(),
                reason: reason.into(),
            });
        }
    }

    fn read_all(&mut self, from: &str, to: &str) -> Result<(), Failure> {
        let user = self.client.get("/user", &[])?;
        let viewer_id = user.body["id"].as_u64().map(|id| id.to_string());
        // Kept as the original of the viewer's observations: only what identifies the token
        // owner, never the rest of its profile.
        let viewer = json!({"id": user.body["id"], "type": user.body["type"]});
        self.submit(
            "viewer",
            "github:viewer",
            "/id",
            viewer_id.as_deref(),
            "structured",
            Origin {
                updated: "",
                original: &viewer,
            },
        )?;
        self.submit(
            "viewer",
            "github:viewer",
            "/type",
            user.body["type"].as_str(),
            "structured",
            Origin {
                updated: "",
                original: &viewer,
            },
        )?;
        let emails = (|| -> Result<Vec<String>, Failure> {
            let mut page = self.client.get("/user/emails", &[("per_page", "100")])?;
            let mut addresses = Vec::new();
            loop {
                let entries = page.body.as_array().ok_or(Failure::new("protocol"))?;
                for entry in entries {
                    if entry["verified"] == true {
                        addresses.push(
                            entry["email"]
                                .as_str()
                                .ok_or(Failure::new("protocol"))?
                                .to_string(),
                        );
                    }
                }
                match page.next {
                    Some(url) => page = self.client.get_url(&url)?,
                    None => break,
                }
            }
            addresses.sort();
            addresses.dedup();
            Ok(addresses)
        })();
        match emails {
            Ok(addresses) => {
                let snapshot = json!({"emails": addresses});
                let mut value = snapshot["emails"].to_string();
                let addresses = snapshot["emails"]
                    .as_array()
                    .ok_or(Failure::new("protocol"))?;
                // The JSON list determines the snapshot; raw concatenated addresses after its
                // newline give exact citation spans even when JSON has to escape a character.
                if !addresses.is_empty() {
                    value.push('\n');
                    for address in addresses {
                        value.push_str(address.as_str().ok_or(Failure::new("protocol"))?);
                    }
                }
                self.submit(
                    "viewer",
                    "github:viewer",
                    "/emails/verified",
                    Some(&value),
                    "structured",
                    Origin {
                        updated: "",
                        original: &snapshot,
                    },
                )?;
            }
            Err(failure) if matches!(failure.status, Some(401 | 403 | 404 | 429)) => {
                self.gaps.push(CoverageGap {
                    selection: "user emails".into(),
                    reason: failure.reason.into(),
                });
            }
            Err(failure) => return Err(failure),
        }
        let login = user.body["login"]
            .as_str()
            .ok_or(Failure::new("protocol"))?
            .to_string();
        for qualifier in QUALIFIERS {
            let q = format!(
                "repo:{} {qualifier}:{login} updated:{from}..{to}",
                self.repo
            );
            let mut page = self.client.get(
                "/search/issues",
                &[
                    ("q", &q),
                    ("sort", "updated"),
                    ("order", "desc"),
                    ("per_page", "100"),
                ],
            )?;
            loop {
                let total = page.body["total_count"].as_u64().unwrap_or(0);
                if total > SEARCH_CAP || page.body["incomplete_results"] == true {
                    self.capped = true;
                }
                let items = page.body["items"]
                    .as_array()
                    .ok_or(Failure::new("protocol"))?
                    .clone();
                for item in &items {
                    self.read_item(item)?;
                }
                match page.next {
                    Some(url) => page = self.client.get_url(&url)?,
                    None => break,
                }
            }
        }
        Ok(())
    }

    fn read_item(&mut self, item: &Value) -> Result<(), Failure> {
        let number = item["number"].as_u64().ok_or(Failure::new("protocol"))?;
        let updated = item["updated_at"]
            .as_str()
            .ok_or(Failure::new("protocol"))?;
        let updated_at: DateTime<Utc> = updated.parse().map_err(|_| Failure::new("protocol"))?;
        // The window is half-open: an item updated at or after `to` belongs to a later one.
        if updated_at >= self.window_to || !self.seen.insert(number) {
            return Ok(());
        }
        let url = item["html_url"].as_str().ok_or(Failure::new("protocol"))?;
        let kind = if item.get("pull_request").is_some() {
            "pull_request"
        } else {
            "issue"
        };
        self.submit(
            kind,
            url,
            "/title",
            item["title"].as_str(),
            "untrusted content",
            Origin {
                updated,
                original: item,
            },
        )?;
        self.submit(
            kind,
            url,
            "/body",
            item["body"].as_str(),
            "untrusted content",
            Origin {
                updated,
                original: item,
            },
        )?;
        self.submit(
            kind,
            url,
            "/state",
            item["state"].as_str(),
            "structured",
            Origin {
                updated,
                original: item,
            },
        )?;
        for (key, name) in [("labels", "name"), ("assignees", "login")] {
            let all = item[key].as_array().map_or(&[][..], Vec::as_slice);
            for (index, entry) in all.iter().enumerate() {
                let field = format!("/{key}/{index}/{name}");
                self.submit(
                    kind,
                    url,
                    &field,
                    entry[name].as_str(),
                    "structured",
                    Origin {
                        updated,
                        original: item,
                    },
                )?;
            }
        }
        self.submit_author(
            kind,
            url,
            Origin {
                updated,
                original: item,
            },
        )?;
        self.read_comments(number)
    }

    fn read_comments(&mut self, number: u64) -> Result<(), Failure> {
        let path = format!("/repos/{}/issues/{number}/comments", self.repo);
        let mut page = self.client.get(&path, &[("per_page", "100")])?;
        loop {
            for comment in page.body.as_array().ok_or(Failure::new("protocol"))? {
                let url = comment["html_url"]
                    .as_str()
                    .ok_or(Failure::new("protocol"))?;
                let updated = comment["updated_at"]
                    .as_str()
                    .ok_or(Failure::new("protocol"))?;
                let at: DateTime<Utc> = updated.parse().map_err(|_| Failure::new("protocol"))?;
                // Comments follow the same half-open window as items.
                if at < self.window_from || at >= self.window_to {
                    continue;
                }
                self.submit(
                    "comment",
                    url,
                    "/body",
                    comment["body"].as_str(),
                    "untrusted content",
                    Origin {
                        updated,
                        original: comment,
                    },
                )?;
                self.submit_author(
                    "comment",
                    url,
                    Origin {
                        updated,
                        original: comment,
                    },
                )?;
            }
            match page.next {
                Some(url) => page = self.client.get_url(&url)?,
                None => return Ok(()),
            }
        }
    }

    /// Author fields are structured observations at the item's own location.
    fn submit_author(&mut self, kind: &str, location: &str, origin: Origin) -> Result<(), Failure> {
        let user = &origin.original["user"];
        let id = user["id"].as_u64().map(|id| id.to_string());
        for (field, text) in [
            ("/user/id", id.as_deref()),
            ("/user/login", user["login"].as_str()),
            ("/user/type", user["type"].as_str()),
        ] {
            self.submit(kind, location, field, text, "structured", origin)?;
        }
        if let (Some(id), Some(login)) = (user["id"].as_u64(), user["login"].as_str()) {
            let base = login.strip_suffix("[bot]").unwrap_or(login);
            if !base.is_empty() && base.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                if !self.profiles.contains_key(&id) {
                    let path = format!("/users/{}", login.replace('[', "%5B").replace(']', "%5D"));
                    let profile = match self.client.get(&path, &[]) {
                        Ok(page) if page.body["id"].as_u64() == Some(id) => page.body,
                        Ok(_) => {
                            self.profile_gap("protocol");
                            Value::Null
                        }
                        Err(failure) if failure.status == Some(404) => {
                            self.profile_gap(failure.reason);
                            Value::Null
                        }
                        Err(failure) => return Err(failure),
                    };
                    self.profiles.insert(id, profile);
                }
                let profile = self.profiles[&id].clone();
                if profile.is_null() {
                    return Ok(());
                }
                self.submit(
                    kind,
                    location,
                    "/user/name",
                    profile["name"].as_str(),
                    "structured",
                    Origin {
                        updated: "",
                        original: &profile,
                    },
                )?;
                self.submit(
                    kind,
                    location,
                    "/user/email",
                    profile["email"].as_str(),
                    "structured",
                    Origin {
                        updated: "",
                        original: &profile,
                    },
                )?;
            }
        }
        Ok(())
    }

    /// One observation: the whole text of one field. Empty or missing text is skipped.
    fn submit(
        &mut self,
        kind: &str,
        location: &str,
        field: &str,
        text: Option<&str>,
        trust_class: &str,
        origin: Origin,
    ) -> Result<(), Failure> {
        let Some(text) = text.filter(|t| !t.is_empty()) else {
            return Ok(());
        };
        let Origin { updated, original } = origin;
        let mut params = json!({
            "kind": kind, "location": location, "field": field, "start": 0,
            "end": text.len(), "quote": text, "trust_class": trust_class,
        });
        if !updated.is_empty() {
            params["source_updated_at"] = json!(updated);
        }
        if self.keep_original {
            params["original"] = original.clone();
        }
        Envelope::notification(EVIDENCE_SUBMIT, params)
            .and_then(|m| self.transport.send(&m))
            .map_err(|_| Failure::new("protocol"))?;
        self.submitted += 1;
        Ok(())
    }
}
