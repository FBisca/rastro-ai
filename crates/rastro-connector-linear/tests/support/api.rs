//! A scripted Linear GraphQL API on loopback: it answers the connector's three operations from
//! a [`World`] and records every request. It applies no filter: the issues it holds are the ones
//! the query is assumed to select.
#![allow(dead_code)]
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

pub const KEY: &str = "lin_api_fixture_secret_key";
pub const VIEWER: &str = "0b5a8c1e-0000-4000-8000-000000000001";
pub const ANA: &str = "0b5a8c1e-0000-4000-8000-000000000002";
pub const BOB: &str = "0b5a8c1e-0000-4000-8000-000000000003";

#[derive(Clone, Debug)]
pub struct Req {
    pub method: String,
    pub path: String,
    pub auth: Option<String>,
    /// The operation name of the GraphQL document, e.g. `Users`.
    pub operation: String,
    pub query: String,
    pub variables: Value,
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
    /// A GraphQL error with Linear's `extensions.code`.
    pub fn error(status: u16, code: &str) -> Self {
        Self::status(
            status,
            json!({"errors": [{"message": "no", "extensions": {"code": code}}]}),
        )
    }
}

/// What the API holds. Pages are `page_size` long; a cursor is the offset of the next page.
#[derive(Clone)]
pub struct World {
    pub viewer: String,
    pub users: Vec<Value>,
    pub issues: Vec<Value>,
    pub page_size: usize,
}
impl Default for World {
    fn default() -> Self {
        Self {
            viewer: VIEWER.into(),
            users: vec![],
            issues: vec![],
            page_size: 100,
        }
    }
}
impl World {
    fn page(&self, nodes: &[Value], req: &Req) -> Value {
        let from: usize = req.variables["after"]
            .as_str()
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        let to = (from + self.page_size).min(nodes.len());
        json!({
            "nodes": nodes[from.min(to)..to],
            "pageInfo": {"hasNextPage": to < nodes.len(), "endCursor": to.to_string()},
        })
    }
    pub fn reply(&self, req: &Req) -> Reply {
        match req.operation.as_str() {
            "Viewer" => Reply::ok(json!({"data": {"viewer": {"id": self.viewer}}})),
            "Users" => Reply::ok(json!({"data": {"users": self.page(&self.users, req)}})),
            "Issues" => Reply::ok(json!({"data": {"issues": self.page(&self.issues, req)}})),
            _ => Reply::error(400, "GRAPHQL_VALIDATION_FAILED"),
        }
    }
}

pub fn days_ago(days: i64) -> String {
    (chrono::Utc::now() - chrono::Duration::days(days))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
pub fn user(id: &str, name: &str, email: &str, active: bool) -> Value {
    json!({
        "id": id, "name": name, "displayName": name.split(' ').next().unwrap().to_lowercase(),
        "email": email, "active": active, "updatedAt": days_ago(30),
    })
}
/// An issue in team ENG created by `creator` and assigned to `assignee`.
pub fn issue(
    id: &str,
    number: u64,
    title: &str,
    description: &str,
    creator: Option<&str>,
    assignee: Option<&str>,
) -> Value {
    let identifier = format!("ENG-{number}");
    json!({
        "id": id, "identifier": identifier, "title": title, "description": description,
        "url": format!("https://linear.app/acme/issue/{identifier}/slug"),
        "createdAt": days_ago(3), "updatedAt": days_ago(1),
        "state": {"name": "In Progress", "type": "started"},
        "creator": creator.map(|id| json!({"id": id})),
        "assignee": assignee.map(|id| json!({"id": id})),
        "team": {"id": "0b5a8c1e-0000-4000-8000-0000000000e1", "key": "ENG", "name": "Engineering"},
    })
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
                let Ok(Some(mut request)) = server.recv_timeout(Duration::from_millis(50)) else {
                    continue;
                };
                let mut text = String::new();
                let _ = request.as_reader().read_to_string(&mut text);
                let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let query = body["query"].as_str().unwrap_or_default().to_string();
                let operation = query
                    .strip_prefix("query ")
                    .and_then(|q| q.split(['(', ' ', '{']).next())
                    .unwrap_or_default()
                    .to_string();
                let req = Req {
                    method: request.method().to_string(),
                    path: request.url().to_string(),
                    auth: request
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv("Authorization"))
                        .map(|h| h.value.to_string()),
                    operation,
                    query,
                    variables: body["variables"].clone(),
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
