//! The GitHub connector: issues, pull requests and comments involving the owner.
//!
//! It speaks the Rastro plugin protocol on stdin/stdout and calls only fixed GET endpoints
//! of the GitHub REST API. Nothing it reads from GitHub chooses what it requests next,
//! except `Link: rel="next"` pagination on the same host.
mod api;

use anyhow::{Context, Result, bail};
use api::{Client, Failure};
use chrono::{DateTime, Utc};
use rastro_plugin_sdk::{
    COVERAGE_REPORT, CoverageReport, CoverageStatus, EVIDENCE_SUBMIT, Envelope, InitializeParams,
    InitializeResult, PROTOCOL_VERSION, RUN_FINISH, RunBinding, RunFinish, Transport,
};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    io::{self, BufRead, Write},
};

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
    submitted: u64,
    capped: bool,
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
    let outcome = sync(&mut transport, &params.run, &mut submitted);
    let failure = outcome.err();
    let report = CoverageReport {
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
) -> Result<(), Failure> {
    let repo = binding
        .selector
        .get("repo")
        .ok_or(Failure::new("protocol"))?
        .clone();
    let token = api::token().ok_or(Failure::new("auth"))?;
    let base = std::env::var("RASTRO_GITHUB_API_URL").unwrap_or_else(|_| DEFAULT_API.into());
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
        submitted: 0,
        capped: false,
    };
    let result = run.read_all(&binding.window.from, &binding.window.to);
    *submitted = run.submitted;
    result?;
    if run.capped {
        return Err(Failure::new("result_cap"));
    }
    Ok(())
}

impl<R: BufRead, W: Write> Run<'_, R, W> {
    fn read_all(&mut self, from: &str, to: &str) -> Result<(), Failure> {
        let user = self.client.get("/user", &[])?;
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
            updated,
        )?;
        self.submit(
            kind,
            url,
            "/body",
            item["body"].as_str(),
            "untrusted content",
            updated,
        )?;
        self.submit(
            kind,
            url,
            "/state",
            item["state"].as_str(),
            "structured",
            updated,
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
                    updated,
                )?;
            }
        }
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
                    updated,
                )?;
            }
            match page.next {
                Some(url) => page = self.client.get_url(&url)?,
                None => return Ok(()),
            }
        }
    }

    /// One observation: the whole text of one field. Empty or missing text is skipped.
    fn submit(
        &mut self,
        kind: &str,
        location: &str,
        field: &str,
        text: Option<&str>,
        trust_class: &str,
        source_updated_at: &str,
    ) -> Result<(), Failure> {
        let Some(text) = text.filter(|t| !t.is_empty()) else {
            return Ok(());
        };
        Envelope::notification(
            EVIDENCE_SUBMIT,
            json!({
                "kind": kind, "location": location, "field": field, "start": 0,
                "end": text.len(), "quote": text, "trust_class": trust_class,
                "source_updated_at": source_updated_at,
            }),
        )
        .and_then(|m| self.transport.send(&m))
        .map_err(|_| Failure::new("protocol"))?;
        self.submitted += 1;
        Ok(())
    }
}
