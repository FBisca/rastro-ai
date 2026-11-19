//! A fake Linear GraphQL API on loopback: it answers an issue create and records every request it
//! receives, so a test sees exactly what reached Linear. It holds the issues it created by the id
//! the caller gave, and numbers them `ENG-1`, `ENG-2`, … in creation order. It answers a lookup of
//! an issue by id, and can turn the response to the next create into an error or hold every create's response
//! until a test lets it go: the issue exists either way.
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

/// The API key the fake accepts, as the test sets it in the provider's environment.
pub const KEY: &str = "lin_api_fixture_secret_key";

/// One request the fake received.
#[derive(Clone, Debug)]
pub struct Req {
    pub method: String,
    pub path: String,
    pub auth: Option<String>,
    /// The operation name of the GraphQL document, e.g. `IssueCreate`.
    pub operation: String,
    /// The request body as it arrived.
    pub body: Value,
}
impl Req {
    /// The `input` of an issue create.
    pub fn input(&self) -> &Value {
        &self.body["variables"]["input"]
    }
}

/// What the fake holds and how it answers.
#[derive(Default)]
struct World {
    issues: Vec<Value>,
    /// The next create is made, but what the caller gets back is an error.
    lose_next_create: bool,
    /// A create is made, but its response waits for [`Fixture::release_creates`].
    hold_creates: bool,
}

/// What happens to the response to a request, once the fake has done what it was asked.
enum Fate {
    Respond,
    /// The create is answered with a server error, so the caller cannot tell the issue was made.
    Lose,
    /// The response waits until the fake's creates are released.
    Hold,
}

pub struct Fixture {
    pub base: String,
    world: Arc<Mutex<World>>,
    log: Arc<Mutex<Vec<Req>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// Answers `request` with `status` and `reply`.
fn respond(request: tiny_http::Request, status: u16, reply: &Value) {
    let mut response = tiny_http::Response::from_string(reply.to_string()).with_status_code(status);
    response.add_header(tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap());
    let _ = request.respond(response);
}

fn operation(query: &str) -> String {
    query
        .strip_prefix("mutation ")
        .or_else(|| query.strip_prefix("query "))
        .and_then(|rest| rest.split(['(', ' ', '{']).next())
        .unwrap_or_default()
        .to_string()
}

impl World {
    fn reply(&mut self, req: &Req) -> (u16, Value, Fate) {
        let mut fate = Fate::Respond;
        let (status, body) = match req.operation.as_str() {
            "IssueCreate" => {
                let input = req.input();
                let number = self.issues.len() + 1;
                let issue = json!({
                    "id": input["id"],
                    "identifier": format!("ENG-{number}"),
                    "url": format!("https://linear.example/acme/issue/ENG-{number}"),
                    "title": input["title"],
                });
                self.issues.push(issue.clone());
                if std::mem::take(&mut self.lose_next_create) {
                    fate = Fate::Lose;
                } else if self.hold_creates {
                    fate = Fate::Hold;
                }
                (
                    200,
                    json!({"data": {"issueCreate": {"success": true, "issue": issue}}}),
                )
            }
            "IssueLookup" => match self
                .issues
                .iter()
                .find(|issue| issue["id"] == req.body["variables"]["id"])
            {
                Some(issue) => (200, json!({"data": {"issue": issue}})),
                // As Linear answers an id it does not hold.
                None => (
                    200,
                    json!({"data": null, "errors": [{
                        "message": "Entity not found: Issue - Could not find referenced Issue.",
                        "extensions": {"type": "invalid input", "code": "INPUT_ERROR"}
                    }]}),
                ),
            },
            _ => (
                400,
                json!({"errors": [{"message": "no", "extensions": {"code": "GRAPHQL_VALIDATION_FAILED"}}]}),
            ),
        };
        (status, body, fate)
    }
}

impl Fixture {
    pub fn start() -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.server_addr().to_ip().unwrap());
        let world = Arc::new(Mutex::new(World::default()));
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
                let req = Req {
                    method: request.method().to_string(),
                    path: request.url().to_string(),
                    auth: request
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv("Authorization"))
                        .map(|h| h.value.to_string()),
                    operation: operation(body["query"].as_str().unwrap_or_default()),
                    body,
                };
                thread_log.lock().unwrap().push(req.clone());
                let (status, reply, fate) = thread_world.lock().unwrap().reply(&req);
                match fate {
                    Fate::Respond => respond(request, status, &reply),
                    Fate::Lose => respond(request, 502, &json!({})),
                    Fate::Hold => {
                        let (world, stop) = (thread_world.clone(), thread_stop.clone());
                        std::thread::spawn(move || {
                            while world.lock().unwrap().hold_creates
                                && !stop.load(Ordering::Relaxed)
                            {
                                std::thread::sleep(Duration::from_millis(5));
                            }
                            respond(request, status, &reply);
                        });
                    }
                }
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

    /// The next issue create is made, but the caller gets an error back instead of the issue.
    pub fn lose_next_create_response(&self) {
        self.world.lock().unwrap().lose_next_create = true;
    }

    /// Holds the response to every issue create, though each issue is made, until
    /// [`Fixture::release_creates`].
    pub fn hold_create_responses(&self) {
        self.world.lock().unwrap().hold_creates = true;
    }

    pub fn release_creates(&self) {
        self.world.lock().unwrap().hold_creates = false;
    }

    /// Waits until the fake has received `count` issue creates; panics if they do not come.
    pub fn wait_for_creates(&self, count: usize) {
        for _ in 0..2000 {
            if self.creates().len() >= count {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the fake Linear never received {count} issue create(s)");
    }

    /// Every request received so far, oldest first.
    pub fn requests(&self) -> Vec<Req> {
        self.log.lock().unwrap().clone()
    }

    /// The issue creates received so far.
    pub fn creates(&self) -> Vec<Req> {
        (self.requests().into_iter())
            .filter(|req| req.operation == "IssueCreate")
            .collect()
    }

    /// The issues the fake holds, as it created them.
    pub fn issues(&self) -> Vec<Value> {
        self.world.lock().unwrap().issues.clone()
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
