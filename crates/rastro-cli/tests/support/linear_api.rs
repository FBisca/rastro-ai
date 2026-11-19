//! A fake Linear GraphQL API on loopback: it answers an issue create and records every request it
//! receives, so a test sees exactly what reached Linear. It holds the issues it created by the id
//! the caller gave, and numbers them `ENG-1`, `ENG-2`, … in creation order. It answers a lookup of
//! an issue by id, and can turn the response to the next create into an error or hold every
//! create's response until a test lets it go: the issue exists either way. It answers an issue
//! delete by moving the issue to the trash (`trashed`, `archivedAt`), where a lookup by id still
//! finds it, and can lose the response to the next delete.
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
    /// How many issues were ever created, so a deleted one's number is never reused.
    created: usize,
    /// The operation whose next request is made, but what the caller gets back is an error.
    lose_next: Option<&'static str>,
    /// A create is made, but its response waits for [`Fixture::release_creates`].
    hold_creates: bool,
    /// Every create is refused with a server error and makes no issue.
    refuse_creates: bool,
}

/// What happens to the response to a request, once the fake has done what it was asked.
enum Fate {
    Respond,
    /// The request is answered with a server error, so the caller cannot tell what it did.
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
            "IssueCreate" if self.refuse_creates => (
                500,
                json!({"errors": [{"message": "Linear is down", "extensions": {"code": "INTERNAL_SERVER_ERROR"}}]}),
            ),
            "IssueCreate" => {
                let input = req.input();
                self.created += 1;
                let number = self.created;
                let issue = json!({
                    "id": input["id"],
                    "identifier": format!("ENG-{number}"),
                    "url": format!("https://linear.example/acme/issue/ENG-{number}"),
                    "title": input["title"],
                });
                self.issues.push(issue.clone());
                if self.lose_next.take_if(|op| *op == "IssueCreate").is_some() {
                    fate = Fate::Lose;
                } else if self.hold_creates {
                    fate = Fate::Hold;
                }
                (
                    200,
                    json!({"data": {"issueCreate": {"success": true, "issue": issue}}}),
                )
            }
            "IssueDelete" => {
                let id = &req.body["variables"]["id"];
                // As Linear does: the issue goes to the trash, archived, and is still held by id.
                let live = (self.issues.iter_mut())
                    .find(|issue| issue["id"] == *id && issue["trashed"] != true);
                let found = live.is_some();
                if let Some(issue) = live {
                    issue["trashed"] = true.into();
                    issue["archivedAt"] = "2026-11-19T10:00:00.000Z".into();
                }
                if found && self.lose_next.take_if(|op| *op == "IssueDelete").is_some() {
                    fate = Fate::Lose;
                }
                (200, json!({"data": {"issueDelete": {"success": found}}}))
            }
            "IssueLookup" => match self
                .issues
                .iter()
                .find(|issue| issue["id"] == req.body["variables"]["id"])
            {
                Some(issue) => {
                    // Only the fields the document selects, as GraphQL answers.
                    let mut issue = issue.clone();
                    if !req.body["query"]
                        .as_str()
                        .is_some_and(|q| q.contains("trashed"))
                    {
                        issue.as_object_mut().unwrap().remove("trashed");
                        issue.as_object_mut().unwrap().remove("archivedAt");
                    }
                    (200, json!({"data": {"issue": issue}}))
                }
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
        self.world.lock().unwrap().lose_next = Some("IssueCreate");
    }

    /// The next issue delete is made, but the caller gets an error back instead of the answer.
    pub fn lose_next_delete_response(&self) {
        self.world.lock().unwrap().lose_next = Some("IssueDelete");
    }

    /// Refuses every issue create with a server error, making no issue, until
    /// [`Fixture::accept_creates`].
    pub fn refuse_creates(&self) {
        self.world.lock().unwrap().refuse_creates = true;
    }

    pub fn accept_creates(&self) {
        self.world.lock().unwrap().refuse_creates = false;
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

    /// Archives every issue the fake holds without trashing it, as Linear does to a closed issue
    /// after a while or when a user archives one by hand: it is still there to delete.
    pub fn archive_issues(&self) {
        for issue in &mut self.world.lock().unwrap().issues {
            issue["archivedAt"] = "2026-11-18T10:00:00.000Z".into();
        }
    }

    /// The issue deletes received so far.
    pub fn deletes(&self) -> Vec<Req> {
        (self.requests().into_iter())
            .filter(|req| req.operation == "IssueDelete")
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
