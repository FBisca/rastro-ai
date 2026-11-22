//! The Slack connector: the messages of the one channel a source names, read with the owner's
//! own read-only user token.
//!
//! It speaks the Rastro plugin protocol on stdin/stdout. The token comes from `SLACK_USER_TOKEN`,
//! else the macOS keychain item `rastro-slack`, and is never stored.
//!
//! # What it records
//! Each field is one observation at a location that is the record's canonical identifier:
//! - a message at `slack:message/<channel>/<ts>`: `/text` as untrusted content, stored raw with
//!   its mention markup; `/user`, `/thread_ts`, `/subtype` and `/edited/ts` structured. The
//!   update time is the edit time when the message was edited, else the post time;
//! - an author at `slack:user/<id>`: `/id`, `/is_bot`, `/name`, `/real_name` and
//!   `/profile/display_name` structured;
//! - whose token it is, at `slack:viewer`: `/id`. One slot, so another token's owner replaces it.
//!
//! The conversation is looked up before any history read: a channel shared outside the
//! workspace, a direct or a group message is never read. System events (joins, renames, topic
//! and purpose changes, archives, pins, edit and delete notices) are skipped; messages that
//! carry a person's words — bot messages, broadcast replies, file shares, /me messages — are
//! kept as evidence. An author profile the API won't give is a coverage gap, and the messages
//! stay; a rate limit, a rejected token or a missing scope stops the run typed.
//!
//! Thread replies are read through a 30-day lookback (`docs/decisions/plugins.md` §50): the
//! history read spans the window plus the lookback, and the replies posted inside the window
//! are read for the threads started in that span. A thread with no reply in the window gets no
//! replies call. The window is half-open, so a message posted at its start belongs to it alone
//! and is stored exactly once across consecutive syncs.
mod api;

use anyhow::{Context, Result, bail};
use api::{Client, Failure};
use chrono::{DateTime, SecondsFormat, Utc};
use rastro_plugin_sdk::{
    COVERAGE_REPORT, CoverageGap, CoverageReport, CoverageStatus, EVIDENCE_SUBMIT, Envelope,
    InitializeParams, InitializeResult, PROTOCOL_VERSION, RUN_FINISH, RunBinding, RunFinish,
    Transport,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashSet},
    io::{self, BufRead, Write},
};

const DEFAULT_API: &str = "https://slack.com/api";

/// How far before the window a thread may have started and still have its replies read
/// (`docs/decisions/plugins.md` §50).
const LOOKBACK_DAYS: i64 = 30;

const STRUCTURED: &str = "structured";
const UNTRUSTED: &str = "untrusted content";

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
                    ..
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
        selection: None,
        reason: failure.as_ref().map(|f| f.reason.to_string()),
        retry_at: failure.as_ref().and_then(|f| f.retry_at.clone()),
        source: None,
        scope: None,
        window: None,
    };
    transport.send(&Envelope::notification(COVERAGE_REPORT, report)?)?;
    if let Some(failure) = &failure {
        eprintln!("slack connector: could not look: {}", failure.reason);
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

/// The read of one run: the channel's messages in the window, its authors and the token owner.
struct Run<'a, R, W> {
    transport: &'a mut Transport<R, W>,
    client: Client,
    channel: String,
    window_from: DateTime<Utc>,
    window_to: DateTime<Utc>,
    /// The window's start minus the lookback: thread starts from here on are read.
    lookback_from: DateTime<Utc>,
    seen: HashSet<String>,
    /// The threads started in the lookback whose replies may fall in the window.
    thread_starts: BTreeSet<String>,
    authors: BTreeSet<String>,
    submitted: u64,
    /// The host said the source keeps originals: attach the raw record.
    keep_original: bool,
    gaps: Vec<CoverageGap>,
}

/// The record an observation belongs to: its kind and location, when the source last updated
/// it, and the record as read.
struct Record<'a> {
    kind: &'a str,
    location: String,
    updated: Option<String>,
    original: &'a Value,
}

/// Reads the named channel's messages in the window. Evidence goes out as each page is read,
/// so a failure part-way keeps what was already submitted.
fn sync<R: BufRead, W: Write>(
    transport: &mut Transport<R, W>,
    binding: &RunBinding,
    submitted: &mut u64,
    gaps: &mut Vec<CoverageGap>,
) -> Result<(), Failure> {
    let channel = binding
        .selector
        .get("channel")
        .ok_or(Failure::new("protocol"))?
        .clone();
    // The override is for tests: it may only point at this machine, so no real token or
    // request can be sent anywhere else.
    let base = match std::env::var("RASTRO_SLACK_API_URL") {
        Ok(base) if api::loopback(&base) => base,
        Ok(_) => return Err(Failure::new("protocol")),
        Err(_) => DEFAULT_API.into(),
    };
    // A token no HTTP header can carry is the token's fault, not the network's.
    let token = api::credential()
        .filter(|(_, token)| token.bytes().all(|b| b.is_ascii_graphic()))
        .map(|(_, token)| token)
        .ok_or(Failure::new("auth"))?;
    let time = |t: &str| t.parse().map_err(|_| Failure::new("protocol"));
    let window_from: DateTime<Utc> = time(&binding.window.from)?;
    let mut run = Run {
        transport,
        client: Client::new(&base, token),
        channel,
        window_from,
        window_to: time(&binding.window.to)?,
        lookback_from: window_from - chrono::Duration::days(LOOKBACK_DAYS),
        seen: HashSet::new(),
        thread_starts: BTreeSet::new(),
        authors: BTreeSet::new(),
        submitted: 0,
        keep_original: binding.keep_original,
        gaps: Vec::new(),
    };
    let result = run.read_all();
    *submitted = run.submitted;
    *gaps = std::mem::take(&mut run.gaps);
    result
}

impl<R: BufRead, W: Write> Run<'_, R, W> {
    /// The conversation is looked up before any history read; then every page of the window's
    /// messages goes out as it is read, so a failure part-way keeps what was already submitted.
    fn read_all(&mut self) -> Result<(), Failure> {
        let info = self.client.conversations_info(&self.channel)?;
        let channel = &info["channel"];
        if !channel.is_object() {
            return Err(Failure::new("protocol"));
        }
        // A conversation shared outside the workspace, a direct or a group message is never
        // read, whatever its id prefix.
        let flag = |name: &str| channel[name].as_bool().unwrap_or(false);
        if flag("is_ext_shared") || flag("is_shared") || flag("is_im") || flag("is_mpim") {
            return Err(Failure::new("forbidden"));
        }
        // Whose token it is: one observation in a fixed slot, so another token's owner
        // replaces it.
        let viewer_id = self.client.viewer_id()?;
        let viewer = json!({"user_id": viewer_id});
        let record = Record {
            kind: "viewer",
            location: "slack:viewer".into(),
            updated: None,
            original: &viewer,
        };
        self.submit(&record, "/id", Some(&viewer_id), STRUCTURED)?;
        self.history_pages()?;
        self.read_threads()?;
        self.read_authors()
    }

    /// Each message author's profile, read once per author. The messages are already stored;
    /// a profile the API won't give is a coverage gap, not a lost run — but a rate limit, a
    /// rejected token or a missing scope stops the run typed, like any other read.
    fn read_authors(&mut self) -> Result<(), Failure> {
        for id in std::mem::take(&mut self.authors) {
            let body = match self.client.users_info(&id) {
                Ok(body) => body,
                Err(failure) if matches!(failure.reason, "rate_limited" | "auth" | "forbidden") => {
                    return Err(failure);
                }
                Err(failure) => {
                    self.profile_gap(failure.reason);
                    continue;
                }
            };
            let user = &body["user"];
            if user["id"].as_str() != Some(id.as_str()) {
                self.profile_gap("protocol");
                continue;
            }
            let record = Record {
                kind: "user",
                location: format!("slack:user/{id}"),
                updated: None,
                original: user,
            };
            let is_bot = user["is_bot"].as_bool().map(|b| b.to_string());
            self.submit(&record, "/id", Some(&id), STRUCTURED)?;
            self.submit(&record, "/is_bot", is_bot.as_deref(), STRUCTURED)?;
            self.submit(&record, "/name", text(user, "/name"), STRUCTURED)?;
            self.submit(&record, "/real_name", text(user, "/real_name"), STRUCTURED)?;
            self.submit(
                &record,
                "/profile/display_name",
                text(user, "/profile/display_name"),
                STRUCTURED,
            )?;
        }
        Ok(())
    }

    /// An author profile couldn't be read: reported once per reason, and the run goes on.
    fn profile_gap(&mut self, reason: &str) {
        if !self
            .gaps
            .iter()
            .any(|g| g.selection == "user profiles" && g.reason == reason)
        {
            self.gaps.push(CoverageGap {
                selection: "user profiles".into(),
                reason: reason.into(),
            });
        }
    }

    /// Every page of the window's history plus the lookback, following the API's cursor until
    /// it says there is no more. The messages in the window go out as they are read; the thread
    /// starts in the lookback are collected for their replies.
    fn history_pages(&mut self) -> Result<(), Failure> {
        let channel = self.channel.clone();
        let oldest = slack_ts(self.lookback_from);
        let latest = slack_ts(self.window_to);
        let mut cursor = String::new();
        let mut cursors = HashSet::new();
        loop {
            let mut params = vec![
                ("channel", channel.as_str()),
                ("oldest", oldest.as_str()),
                ("latest", latest.as_str()),
                // Slack's bounds are exclusive by default: inclusive returns [oldest, latest],
                // so a thread started exactly at the lookback's edge is still found, and the
                // half-open filter below drops the latest edge.
                ("inclusive", "true"),
                ("limit", "200"),
            ];
            if !cursor.is_empty() {
                params.push(("cursor", cursor.as_str()));
            }
            let page = self.client.conversations_history(&params)?;
            let messages = page["messages"]
                .as_array()
                .ok_or(Failure::new("protocol"))?;
            for message in messages {
                self.collect_thread_start(message);
                self.read_message(message)?;
            }
            if page["has_more"] != true {
                return Ok(());
            }
            // A cursor seen before would page forever.
            let next = text(&page, "/response_metadata/next_cursor")
                .filter(|c| !c.is_empty() && cursors.insert(c.to_string()))
                .ok_or(Failure::new("protocol"))?;
            cursor = next.to_string();
        }
    }

    /// A thread started in the lookback whose replies may fall in the window: its start, kept
    /// once. A thread started before the lookback or at/after the window's end is not read, and
    /// a thread whose latest reply predates the window has no reply in it: no replies call.
    fn collect_thread_start(&mut self, message: &Value) {
        let Some(ts) = text(message, "/ts") else {
            return;
        };
        let Some(at) = slack_time(ts) else {
            return;
        };
        if at < self.lookback_from || at >= self.window_to {
            return;
        }
        if message["reply_count"].as_u64().unwrap_or(0) == 0 {
            return;
        }
        if let Some(latest) = text(message, "/latest_reply").and_then(slack_time)
            && latest < self.window_from
        {
            return;
        }
        self.thread_starts.insert(ts.to_string());
    }

    /// The replies posted in the window to each thread started in the lookback. Every message
    /// goes through the same window filter and seen set as the history read, so a message at
    /// the window's start is stored exactly once across consecutive syncs.
    fn read_threads(&mut self) -> Result<(), Failure> {
        for ts in std::mem::take(&mut self.thread_starts) {
            self.replies_pages(&ts)?;
        }
        Ok(())
    }

    /// Every page of one thread's replies in the window, following the API's cursor until it
    /// says there is no more. `inclusive` asks Slack for `[oldest, latest]`: its bounds are
    /// exclusive by default, which would leave out a reply posted exactly at the window's
    /// start — the previous sync left it out too, so it would never be stored. The half-open
    /// filter in `read_message` drops the latest edge instead.
    fn replies_pages(&mut self, thread_ts: &str) -> Result<(), Failure> {
        let channel = self.channel.clone();
        let oldest = slack_ts(self.window_from);
        let latest = slack_ts(self.window_to);
        let mut cursor = String::new();
        let mut cursors = HashSet::new();
        loop {
            let mut params = vec![
                ("channel", channel.as_str()),
                ("ts", thread_ts),
                ("oldest", oldest.as_str()),
                ("latest", latest.as_str()),
                ("inclusive", "true"),
                ("limit", "200"),
            ];
            if !cursor.is_empty() {
                params.push(("cursor", cursor.as_str()));
            }
            let page = self.client.conversations_replies(&params)?;
            let messages = page["messages"]
                .as_array()
                .ok_or(Failure::new("protocol"))?;
            for message in messages {
                self.read_message(message)?;
            }
            if page["has_more"] != true {
                return Ok(());
            }
            // A cursor seen before would page forever.
            let next = text(&page, "/response_metadata/next_cursor")
                .filter(|c| !c.is_empty() && cursors.insert(c.to_string()))
                .ok_or(Failure::new("protocol"))?;
            cursor = next.to_string();
        }
    }

    fn read_message(&mut self, message: &Value) -> Result<(), Failure> {
        let ts = text(message, "/ts").ok_or(Failure::new("protocol"))?;
        let at = slack_time(ts).ok_or(Failure::new("protocol"))?;
        // The window is half-open: a message posted at or after `to` belongs to a later one.
        if at < self.window_from || at >= self.window_to || !self.seen.insert(ts.to_string()) {
            return Ok(());
        }
        let subtype = text(message, "/subtype");
        if system_message(subtype) {
            return Ok(());
        }
        if let Some(author) = text(message, "/user") {
            self.authors.insert(author.to_string());
        }
        let record = Record {
            kind: "message",
            location: format!("slack:message/{}/{}", self.channel, ts),
            // The edit time when the message was edited, else the post time.
            updated: Some(
                text(message, "/edited/ts")
                    .and_then(slack_time)
                    .unwrap_or(at)
                    .to_rfc3339_opts(SecondsFormat::Millis, true),
            ),
            original: message,
        };
        // The text is stored raw, mention markup and all: a quote must equal the stored bytes.
        self.submit(&record, "/text", text(message, "/text"), UNTRUSTED)?;
        self.submit(&record, "/user", text(message, "/user"), STRUCTURED)?;
        self.submit(
            &record,
            "/thread_ts",
            text(message, "/thread_ts"),
            STRUCTURED,
        )?;
        self.submit(
            &record,
            "/edited/ts",
            text(message, "/edited/ts"),
            STRUCTURED,
        )?;
        self.submit(&record, "/subtype", subtype, STRUCTURED)?;
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
        if let Some(updated) = &record.updated {
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

/// The text of a string field, or `None` when it is missing, null or not a string.
fn text<'a>(node: &'a Value, pointer: &str) -> Option<&'a str> {
    node.pointer(pointer).and_then(Value::as_str)
}

/// Whether a message is a system event — a join or leave, a rename, a topic or purpose
/// change, an archive, a pin or unpin, a bot added or removed, a reminder, an edit or delete
/// notice, a legacy reply notice, an EKM denial or a Slack Connect room — rather than
/// someone's words. Subtypes that carry a person's text (a bot message, a broadcast thread
/// reply, a file share, a /me message) are kept as evidence.
fn system_message(subtype: Option<&str>) -> bool {
    subtype.is_some_and(|s| {
        s.starts_with("channel_")
            || s.starts_with("group_")
            || matches!(
                s,
                "message_changed"
                    | "message_deleted"
                    | "message_replied"
                    | "pinned_item"
                    | "unpinned_item"
                    | "bot_add"
                    | "bot_remove"
                    | "reminder_add"
                    | "ekm_access_denied"
                    | "sh_room_created"
            )
    })
}

/// Slack's time: epoch seconds with a fractional part, e.g. `1785542400.000100`.
fn slack_time(ts: &str) -> Option<DateTime<Utc>> {
    let (secs, frac) = ts.split_once('.').unwrap_or((ts, "0"));
    let secs: i64 = secs.parse().ok()?;
    let mut frac: String = frac.chars().take(9).collect();
    while frac.len() < 9 {
        frac.push('0');
    }
    let nanos: u32 = frac.parse().ok()?;
    DateTime::from_timestamp(secs, nanos)
}

/// A window bound as Slack's `oldest`/`latest`: epoch seconds with a microsecond fraction.
fn slack_ts(at: DateTime<Utc>) -> String {
    format!("{}.{:06}", at.timestamp(), at.timestamp_subsec_micros())
}
#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::{scope_verdict, system_message};

    /// spec: A Slack sync stores each message's fields as evidence
    #[test]
    fn only_system_event_subtypes_are_skipped_and_word_carrying_ones_kept() {
        assert!(!system_message(None));
        for kept in [
            "bot_message",
            "thread_broadcast",
            "file_share",
            "me_message",
        ] {
            assert!(
                !system_message(Some(kept)),
                "{kept} carries a person's words"
            );
        }
        for skipped in [
            "channel_join",
            "channel_leave",
            "channel_topic",
            "channel_purpose",
            "channel_name",
            "channel_archive",
            "channel_unarchive",
            "group_join",
            "group_topic",
            "group_name",
            "message_changed",
            "message_deleted",
            "pinned_item",
            "unpinned_item",
            "bot_add",
            "bot_remove",
            "reminder_add",
            "ekm_access_denied",
            "message_replied",
            "sh_room_created",
        ] {
            assert!(system_message(Some(skipped)), "{skipped} is a system event");
        }
    }

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
