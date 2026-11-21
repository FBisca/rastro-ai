//! A scripted Slack Web API on loopback: it answers the connector's fixed calls from a [`World`]
//! and records every request. It applies no window filter: the messages it holds are the ones
//! the call is assumed to select, so the connector's own filtering is what a test observes.
#![allow(dead_code)]
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

pub const TOKEN: &str = "xoxp-sync-fixture-secret";
pub const CHANNEL: &str = "C0123ABCDE";
pub const VIEWER: &str = "U0OWNER";
pub const ANA: &str = "U0ANA";
pub const BOT: &str = "U0BOT";
/// The five read scopes, as the spec names them.
pub const READ_SCOPES: &str =
    "channels:read,channels:history,groups:read,groups:history,users:read";

#[derive(Clone, Debug)]
pub struct Req {
    /// The Slack method: the first path segment, e.g. `conversations.history`.
    pub method: String,
    pub params: BTreeMap<String, String>,
    pub auth: Option<String>,
}

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}
impl Reply {
    pub fn ok(body: Value) -> Self {
        Self::status(200, body)
    }
    pub fn status(status: u16, body: Value) -> Self {
        Self {
            status,
            headers: vec![],
            body,
        }
    }
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
    /// Slack answers `ok: false` with an `error` code even on HTTP 200.
    pub fn error(code: &str) -> Self {
        Self::ok(json!({"ok": false, "error": code}))
    }
}

/// What the API holds. Messages are kept oldest first and served newest first, as Slack does;
/// pages are `page_size` long and a cursor is the offset of the next page. Thread replies live
/// in `replies`, served only by `conversations.replies` (oldest first, the parent leading, as
/// Slack does); `conversations.history` serves the messages, thread parents included.
#[derive(Clone)]
pub struct World {
    pub viewer: String,
    pub channel: Value,
    pub messages: Vec<Value>,
    pub replies: Vec<Value>,
    pub users: Vec<Value>,
    pub page_size: usize,
}
impl Default for World {
    fn default() -> Self {
        Self {
            viewer: VIEWER.into(),
            channel: channel(),
            messages: vec![],
            replies: vec![],
            users: vec![
                user(VIEWER, "Owner Person", false),
                user(ANA, "Ana Silva", false),
                user(BOT, "Deploy Bot", true),
            ],
            page_size: 100,
        }
    }
}
impl World {
    pub fn reply(&self, req: &Req) -> Reply {
        match req.method.as_str() {
            "auth.test" => Reply::ok(json!({
                "ok": true, "url": "https://acme.slack.com/", "team": "Acme",
                "user": "owner", "team_id": "T0TEAM", "user_id": self.viewer,
            }))
            .header("X-OAuth-Scopes", READ_SCOPES),
            "conversations.info" => Reply::ok(json!({"ok": true, "channel": self.channel})),
            "conversations.history" => {
                let newest_first: Vec<&Value> = self.messages.iter().rev().collect();
                let from: usize = req
                    .params
                    .get("cursor")
                    .and_then(|c| c.parse().ok())
                    .unwrap_or(0);
                let to = (from + self.page_size).min(newest_first.len());
                let has_more = to < newest_first.len();
                Reply::ok(json!({
                    "ok": true,
                    "messages": newest_first[from.min(to)..to],
                    "has_more": has_more,
                    "response_metadata": {
                        "next_cursor": if has_more { to.to_string() } else { String::new() },
                    },
                }))
            }
            "conversations.replies" => {
                let thread_ts = req.params.get("ts").map(String::as_str).unwrap_or("");
                let Some(parent) = self
                    .messages
                    .iter()
                    .find(|m| m["ts"].as_str() == Some(thread_ts))
                else {
                    return Reply::error("thread_not_found");
                };
                let mut messages = vec![parent.clone()];
                messages.extend(
                    self.replies
                        .iter()
                        .filter(|r| r["thread_ts"].as_str() == Some(thread_ts))
                        .cloned(),
                );
                let from: usize = req
                    .params
                    .get("cursor")
                    .and_then(|c| c.parse().ok())
                    .unwrap_or(0);
                let to = (from + self.page_size).min(messages.len());
                let has_more = to < messages.len();
                Reply::ok(json!({
                    "ok": true,
                    "messages": messages[from.min(to)..to],
                    "has_more": has_more,
                    "response_metadata": {
                        "next_cursor": if has_more { to.to_string() } else { String::new() },
                    },
                }))
            }
            "users.info" => {
                let id = req.params.get("user").map(String::as_str).unwrap_or("");
                match self.users.iter().find(|u| u["id"] == id) {
                    Some(user) => Reply::ok(json!({"ok": true, "user": user})),
                    None => Reply::error("user_not_found"),
                }
            }
            _ => Reply::error("unknown_method"),
        }
    }
}

/// The window start the test home configures for its source.
pub fn window_start() -> DateTime<Utc> {
    "2026-08-01T00:00:00Z".parse().unwrap()
}
/// An instant inside the test window: `hours` after its start.
pub fn in_window(hours: i64) -> DateTime<Utc> {
    window_start() + chrono::Duration::hours(hours)
}
/// Slack's message time: epoch seconds with a microsecond fraction.
pub fn ts(at: DateTime<Utc>) -> String {
    format!("{}.{:06}", at.timestamp(), at.timestamp_subsec_micros())
}
/// The instant a Slack ts names: the inverse of [`ts`].
pub fn from_ts(ts: &str) -> DateTime<Utc> {
    let (secs, frac) = ts.split_once('.').unwrap_or((ts, "0"));
    let secs: i64 = secs.parse().unwrap();
    let mut frac = frac.to_string();
    while frac.len() < 6 {
        frac.push('0');
    }
    let micros: u32 = frac[..6].parse().unwrap();
    DateTime::from_timestamp(secs, micros * 1000).unwrap()
}
/// A plain public channel the token owner can read.
pub fn channel() -> Value {
    json!({
        "id": CHANNEL, "name": "eng", "is_channel": true, "is_private": false,
        "is_im": false, "is_mpim": false, "is_shared": false, "is_ext_shared": false,
        "is_archived": false,
    })
}
pub fn user(id: &str, name: &str, is_bot: bool) -> Value {
    json!({
        "id": id, "name": name.to_lowercase().replace(' ', "."), "real_name": name,
        "is_bot": is_bot, "deleted": false, "team_id": "T0TEAM",
    })
}
pub fn message(author: &str, text: &str, at: DateTime<Utc>) -> Value {
    json!({"type": "message", "user": author, "text": text, "ts": ts(at)})
}
/// A bot message: Slack marks it with the `bot_message` subtype.
pub fn bot_message(text: &str, at: DateTime<Utc>) -> Value {
    let mut msg = message(BOT, text, at);
    msg["subtype"] = json!("bot_message");
    msg["bot_id"] = json!("B0DEPLOY");
    msg
}
/// A system message: someone joined the channel.
pub fn join_message(at: DateTime<Utc>) -> Value {
    let mut msg = message(ANA, "<@U0ANA> has joined the channel", at);
    msg["subtype"] = json!("channel_join");
    msg
}
/// A reply in a thread: the author's words, with the thread it answers.
pub fn reply_message(author: &str, text: &str, at: DateTime<Utc>, thread_ts: &str) -> Value {
    let mut msg = message(author, text, at);
    msg["thread_ts"] = json!(thread_ts);
    msg
}
/// A thread parent as history serves it: its own ts as `thread_ts`, with the `reply_count` and
/// `latest_reply` Slack reports, computed from the replies the world holds. Slack's ts is
/// fixed-width epoch seconds, so the greatest ts string is the latest reply.
pub fn thread_parent(author: &str, text: &str, at: DateTime<Utc>, replies: &[Value]) -> Value {
    let mut msg = message(author, text, at);
    msg["thread_ts"] = json!(ts(at));
    msg["reply_count"] = json!(replies.len());
    if let Some(latest) = replies.iter().filter_map(|r| r["ts"].as_str()).max() {
        msg["latest_reply"] = json!(latest);
    }
    msg
}
/// A reply broadcast to the whole channel: the author's words, with the thread it answers.
pub fn broadcast_message(text: &str, at: DateTime<Utc>, thread_ts: &str) -> Value {
    let mut msg = message(ANA, text, at);
    msg["subtype"] = json!("thread_broadcast");
    msg["thread_ts"] = json!(thread_ts);
    msg
}
/// A file shared with the author's message text; the file itself is never read.
pub fn file_share_message(text: &str, at: DateTime<Utc>) -> Value {
    let mut msg = message(ANA, text, at);
    msg["subtype"] = json!("file_share");
    msg["files"] = json!([{"id": "F0FILE", "name": "report.pdf"}]);
    msg
}
/// A /me message: the author's words in the third person.
pub fn me_message(text: &str, at: DateTime<Utc>) -> Value {
    let mut msg = message(ANA, text, at);
    msg["subtype"] = json!("me_message");
    msg
}

type Route = dyn Fn(&Req, &World) -> Option<Reply> + Send + Sync;

/// Serves the world (or what `route` answers first) and records every request.
pub struct Fixture {
    pub base: String,
    pub world: Arc<Mutex<World>>,
    log: Arc<Mutex<Vec<Req>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Fixture {
    pub fn start(world: World) -> Self {
        Self::with_route(world, |_, _| None)
    }
    pub fn with_route(
        world: World,
        route: impl Fn(&Req, &World) -> Option<Reply> + Send + Sync + 'static,
    ) -> Self {
        let route: Arc<Route> = Arc::new(route);
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.server_addr().to_ip().unwrap());
        let world = Arc::new(Mutex::new(world));
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_world, thread_log, thread_stop) = (world.clone(), log.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(50)) else {
                    continue;
                };
                let url = request.url().to_string();
                let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
                // The connector's params are ids, times and cursors: all URL-safe.
                let params = query
                    .split('&')
                    .filter(|pair| !pair.is_empty())
                    .filter_map(|pair| pair.split_once('='))
                    .map(|(name, value)| (name.to_string(), value.to_string()))
                    .collect();
                let req = Req {
                    method: path.trim_start_matches('/').to_string(),
                    params,
                    auth: request
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv("Authorization"))
                        .map(|h| h.value.to_string()),
                };
                thread_log.lock().unwrap().push(req.clone());
                let world = thread_world.lock().unwrap().clone();
                let reply = route(&req, &world).unwrap_or_else(|| world.reply(&req));
                let mut response = tiny_http::Response::from_string(reply.body.to_string())
                    .with_status_code(reply.status);
                for (name, value) in reply
                    .headers
                    .iter()
                    .chain(&[("Content-Type".to_string(), "application/json".to_string())])
                {
                    response.add_header(
                        tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap(),
                    );
                }
                let _ = request.respond(response);
            }
        });
        Self {
            base,
            world,
            log,
            stop,
            thread: Some(thread),
        }
    }
    pub fn requests(&self) -> Vec<Req> {
        self.log.lock().unwrap().clone()
    }
    /// Whether any request called `method`.
    pub fn called(&self, method: &str) -> bool {
        self.requests().iter().any(|r| r.method == method)
    }
    /// Changes what the API answers from now on.
    pub fn change(&self, edit: impl FnOnce(&mut World)) {
        edit(&mut self.world.lock().unwrap());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
