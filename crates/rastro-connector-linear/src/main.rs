//! The Linear connector: workspace users, and the issues the API-key user created, is assigned
//! or subscribes to.
//!
//! It speaks the Rastro plugin protocol on stdin/stdout and sends only three fixed GraphQL
//! queries to Linear. Nothing it reads chooses what it requests next, except the API's own
//! page cursor for the same query.
//!
//! # What it records
//! Each field is one observation at a location that is the record's canonical identifier, so a
//! changed title or team never moves it:
//! - a user at `linear:user/<id>`: `/id`, `/app`, `/name`, `/displayName`, `/email`, `/active`;
//! - an issue at `linear:issue/<id>`: `/id`, `/identifier`, `/url`, `/title`, `/description`,
//!   `/state/name`, `/state/type`, `/creator/app`, `/creator/id`, `/assignee/app`,
//!   `/assignee/id`, `/team/id`, `/team/key`, `/team/name`, `/createdAt`. Title, description
//!   and URL (whose slug is made from the title) are untrusted content, the rest structured;
//! - `app` is Linear's flag for an app (integration) user, `true` or `false`. An issue's
//!   creator or assignee flag is recorded just before its id, so a reader going in order never
//!   pairs a new actor's id with the previous actor's flag. The field read is `User.app`; the
//!   tests run against a scripted API, so that name is not yet checked against the live one;
//! - whose API key it is, at `linear:viewer`: `/id`. One slot, so a key of another user
//!   replaces it.
//!
//! Every user is read on every sync, deactivated ones too: a user's state matters whenever it
//! last changed, and an unchanged field adds no evidence. Issues are read when updated in the
//! window.
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

const DEFAULT_API: &str = "https://api.linear.app";

const VIEWER: &str = "query Viewer { viewer { id } }";
const USERS: &str = "query Users($after: String) { users(first: 100, after: $after, \
    includeDisabled: true) { nodes { id name displayName email active app updatedAt } \
    pageInfo { hasNextPage endCursor } } }";
const ISSUES: &str = "query Issues($after: String, $from: DateTimeOrDuration!, \
    $to: DateTimeOrDuration!) { issues(first: 100, after: $after, orderBy: updatedAt, \
    filter: { updatedAt: { gte: $from, lt: $to }, or: [{ creator: { isMe: { eq: true } } }, \
    { assignee: { isMe: { eq: true } } }, { subscribers: { some: { isMe: { eq: true } } } }] }) \
    { nodes { id identifier title description url createdAt updatedAt state { name type } \
    creator { id app } assignee { id app } team { id key name } } \
    pageInfo { hasNextPage endCursor } } }";

const STRUCTURED: &str = "structured";
const UNTRUSTED: &str = "untrusted content";

/// The record an observation belongs to: its kind and location, when the source last updated
/// it, and the record as read.
struct Record<'a> {
    kind: &'a str,
    location: String,
    updated: Option<&'a str>,
    original: &'a Value,
}

struct Run<'a, R, W> {
    transport: &'a mut Transport<R, W>,
    client: Client,
    window_from: DateTime<Utc>,
    window_to: DateTime<Utc>,
    seen: HashSet<String>,
    submitted: u64,
    /// The host said the source keeps originals: attach the raw record.
    keep_original: bool,
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
        selection: Some("workspace users and issues involving the API-key user".into()),
        reason: failure.as_ref().map(|f| f.reason.to_string()),
        retry_at: failure.as_ref().and_then(|f| f.retry_at.clone()),
        source: None,
        scope: None,
        window: None,
    };
    transport.send(&Envelope::notification(COVERAGE_REPORT, report)?)?;
    if let Some(failure) = &failure {
        eprintln!("linear connector: could not look: {}", failure.reason);
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

/// Reads the key's user, every user and the involved issues of the window. Evidence goes out
/// as each page is read, so a failure part-way keeps what was already submitted.
fn sync<R: BufRead, W: Write>(
    transport: &mut Transport<R, W>,
    binding: &RunBinding,
    submitted: &mut u64,
) -> Result<(), Failure> {
    // The override is for tests: it may only point at this machine, so no real key or
    // request can be sent anywhere else.
    let base = match std::env::var("RASTRO_LINEAR_API_URL") {
        Ok(base) if api::loopback(&base) => base,
        Ok(_) => return Err(Failure::new("protocol")),
        Err(_) => DEFAULT_API.into(),
    };
    // A key no HTTP header can carry is the key's fault, not the network's.
    let key = api::api_key()
        .filter(|key| key.bytes().all(|b| b.is_ascii_graphic()))
        .ok_or(Failure::new("auth"))?;
    let time = |t: &str| t.parse().map_err(|_| Failure::new("protocol"));
    let mut run = Run {
        transport,
        client: Client::new(&base, key),
        window_from: time(&binding.window.from)?,
        window_to: time(&binding.window.to)?,
        seen: HashSet::new(),
        submitted: 0,
        keep_original: binding.keep_original,
    };
    let result = run.read_all(&binding.window.from, &binding.window.to);
    *submitted = run.submitted;
    result
}

/// The text of a string field, or `None` when it is missing, null or not a string.
fn text<'a>(node: &'a Value, pointer: &str) -> Option<&'a str> {
    node.pointer(pointer).and_then(Value::as_str)
}
/// A boolean field as `true` or `false`, or `None` when it is missing, null or not a boolean.
fn flag(node: &Value, pointer: &str) -> Option<String> {
    node.pointer(pointer)
        .and_then(Value::as_bool)
        .map(|b| b.to_string())
}

impl<R: BufRead, W: Write> Run<'_, R, W> {
    fn read_all(&mut self, from: &str, to: &str) -> Result<(), Failure> {
        let data = self.client.query(VIEWER, json!({}))?;
        let viewer = &data["viewer"];
        let id = text(viewer, "/id").ok_or(Failure::new("protocol"))?;
        let record = Record {
            kind: "viewer",
            location: "linear:viewer".into(),
            updated: None,
            original: viewer,
        };
        self.submit(&record, "/id", Some(id), STRUCTURED)?;
        self.pages(USERS, json!({}), "users", Self::read_user)?;
        self.pages(
            ISSUES,
            json!({"from": from, "to": to}),
            "issues",
            Self::read_issue,
        )
    }

    /// Every page of a connection, following the API's cursor until it says there is no more.
    fn pages(
        &mut self,
        query: &str,
        mut variables: Value,
        connection: &str,
        read: fn(&mut Self, &Value) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        let mut cursors = HashSet::new();
        loop {
            let data = self.client.query(query, variables.clone())?;
            let page = &data[connection];
            let nodes = page["nodes"].as_array().ok_or(Failure::new("protocol"))?;
            for node in nodes {
                read(self, node)?;
            }
            if page["pageInfo"]["hasNextPage"] != true {
                return Ok(());
            }
            // A cursor seen before would page forever.
            let cursor = text(page, "/pageInfo/endCursor")
                .filter(|c| cursors.insert(c.to_string()))
                .ok_or(Failure::new("protocol"))?;
            variables["after"] = json!(cursor);
        }
    }

    fn read_user(&mut self, user: &Value) -> Result<(), Failure> {
        let id = text(user, "/id").ok_or(Failure::new("protocol"))?;
        let record = Record {
            kind: "user",
            location: format!("linear:user/{id}"),
            updated: text(user, "/updatedAt"),
            original: user,
        };
        let active = flag(user, "/active");
        let app = flag(user, "/app");
        for (field, value) in [
            ("/id", Some(id)),
            ("/app", app.as_deref()),
            ("/name", text(user, "/name")),
            ("/displayName", text(user, "/displayName")),
            ("/email", text(user, "/email")),
            ("/active", active.as_deref()),
        ] {
            self.submit(&record, field, value, STRUCTURED)?;
        }
        Ok(())
    }

    fn read_issue(&mut self, issue: &Value) -> Result<(), Failure> {
        let id = text(issue, "/id").ok_or(Failure::new("protocol"))?;
        let updated = text(issue, "/updatedAt").ok_or(Failure::new("protocol"))?;
        let updated_at: DateTime<Utc> = updated.parse().map_err(|_| Failure::new("protocol"))?;
        // The window is half-open: an issue updated at or after `to` belongs to a later one.
        if updated_at < self.window_from
            || updated_at >= self.window_to
            || !self.seen.insert(id.to_string())
        {
            return Ok(());
        }
        let record = Record {
            kind: "issue",
            location: format!("linear:issue/{id}"),
            updated: Some(updated),
            original: issue,
        };
        let creator_app = flag(issue, "/creator/app");
        let assignee_app = flag(issue, "/assignee/app");
        let value = |field: &str| match field {
            "/creator/app" => creator_app.as_deref(),
            "/assignee/app" => assignee_app.as_deref(),
            _ => text(issue, field),
        };
        for (field, trust_class) in [
            ("/id", STRUCTURED),
            ("/identifier", STRUCTURED),
            // The URL ends in a slug made from the title, so it is as untrusted as the title.
            ("/url", UNTRUSTED),
            ("/title", UNTRUSTED),
            ("/description", UNTRUSTED),
            ("/state/name", STRUCTURED),
            ("/state/type", STRUCTURED),
            // Each actor's flag just before its id (see the module docs).
            ("/creator/app", STRUCTURED),
            ("/creator/id", STRUCTURED),
            ("/assignee/app", STRUCTURED),
            ("/assignee/id", STRUCTURED),
            ("/team/id", STRUCTURED),
            ("/team/key", STRUCTURED),
            ("/team/name", STRUCTURED),
            ("/createdAt", STRUCTURED),
        ] {
            self.submit(&record, field, value(field), trust_class)?;
        }
        Ok(())
    }

    /// One observation: the whole text of one field. Empty or missing text is skipped.
    fn submit(
        &mut self,
        record: &Record,
        field: &str,
        text: Option<&str>,
        trust_class: &str,
    ) -> Result<(), Failure> {
        let Some(text) = text.filter(|t| !t.is_empty()) else {
            return Ok(());
        };
        let mut params = json!({
            "kind": record.kind, "location": record.location, "field": field, "start": 0,
            "end": text.len(), "quote": text, "trust_class": trust_class,
        });
        if let Some(updated) = record.updated {
            params["source_updated_at"] = json!(updated);
        }
        if self.keep_original {
            params["original"] = record.original.clone();
        }
        Envelope::notification(EVIDENCE_SUBMIT, params)
            .and_then(|m| self.transport.send(&m))
            .map_err(|_| Failure::new("protocol"))?;
        self.submitted += 1;
        Ok(())
    }
}
