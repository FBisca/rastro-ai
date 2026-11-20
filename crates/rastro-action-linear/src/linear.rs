//! A blocking Linear GraphQL client that only sends the provider's fixed documents.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::time::Duration;

const DEFAULT_API: &str = "https://api.linear.app";

/// The one document an issue create sends. Linear takes the issue's id from the input, so a
/// repeated create with the same id names the same issue.
const ISSUE_CREATE: &str = "mutation IssueCreate($input: IssueCreateInput!) { \
    issueCreate(input: $input) { success issue { id identifier url } } }";

/// The one document an issue lookup sends: the issue by the id it was created under.
/// It also reads `trashed`: Linear keeps a deleted issue in its trash, and still answers a lookup of
/// it by id.
const ISSUE_LOOKUP: &str =
    "query IssueLookup($id: String!) { issue(id: $id) { id identifier url trashed } }";

/// The one document an issue delete sends: Linear moves the issue to its trash.
const ISSUE_DELETE: &str =
    "mutation IssueDelete($id: String!) { issueDelete(id: $id) { success } }";

/// The one document that asks whose key this is: the owner's.
const VIEWER: &str = "query Viewer { viewer { id name } }";

/// The one document that finds a team's Linear id by its key.
const TEAM_BY_KEY: &str =
    "query Teams($key: String!) { teams(filter: {key: {eq: $key}}) { nodes { id key } } }";

/// The assignee that names the key's owner.
const ME: &str = "me";

/// The one document a read of an issue's comments sends, one page at a time.
const ISSUE_COMMENTS: &str = "query IssueComments($id: String!, $after: String) { \
    issue(id: $id) { comments(first: 100, after: $after) { \
    nodes { id body user { id name } botActor { name } } pageInfo { hasNextPage endCursor } } } }";

/// The one document a read of an issue's change history sends, one page at a time.
const ISSUE_HISTORY: &str = "query IssueHistory($id: String!, $after: String) { \
    issue(id: $id) { history(first: 100, after: $after) { \
    nodes { id actor { id name } botActor { name } descriptionUpdatedBy { id name } fromTitle toTitle updatedDescription fromStateId toStateId \
    fromAssigneeId toAssigneeId fromPriority toPriority addedLabelIds removedLabelIds } \
    pageInfo { hasNextPage endCursor } } } }";

/// How many pages of one read are followed before the read is given up as not ending.
const MAX_PAGES: usize = 1000;

/// What an undo says when it can't tell whether anyone else touched the issue.
const UNCHECKED: &str = "the issue's history could not be checked, so it was left alone";

/// Linear's answer to an id it holds no entity for. A lookup reads it as "no such issue"; any
/// other document reads it as the refusal it is.
#[derive(Debug)]
struct NotFound(String);
impl std::fmt::Display for NotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Linear refused the request: {}", self.0)
    }
}
impl std::error::Error for NotFound {}

/// What Linear says of an issue it created.
#[derive(Debug, Eq, PartialEq)]
pub struct Created {
    pub identifier: String,
    pub url: String,
    /// Whether the issue is in Linear's trash: deleted, though a lookup still finds it.
    pub trashed: bool,
}

/// The content an issue is created with: what the owner accepted, and nothing else. The team is
/// its key and the assignee is `me` or a Linear user id; the client turns them into Linear ids.
pub struct Issue<'a> {
    /// The run's random id, which Linear keeps as the issue's own.
    pub id: &'a str,
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub team: &'a str,
    pub assignee: &'a str,
}
impl Issue<'_> {
    /// The create's input, with the team and assignee as the Linear ids `team_id` and `assignee_id`.
    fn input(&self, team_id: &str, assignee_id: &str) -> Value {
        let mut input = json!({
            "id": self.id,
            "title": self.title,
            "teamId": team_id,
            "assigneeId": assignee_id,
        });
        if let Some(description) = self.description {
            input["description"] = description.into();
        }
        input
    }
}

/// `LINEAR_API_KEY`, else the keychain item `rastro-linear`, as the connector reads it.
fn api_key() -> Option<String> {
    let from_env = std::env::var("LINEAR_API_KEY")
        .ok()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty());
    from_env.or_else(|| {
        let output = std::process::Command::new("security")
            .args(["find-generic-password", "-s", "rastro-linear", "-w"])
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        let key = String::from_utf8(output.stdout).ok()?.trim().to_string();
        (output.status.success() && !key.is_empty()).then_some(key)
    })
}

pub struct Client {
    agent: ureq::Agent,
    endpoint: String,
    key: String,
}
impl Client {
    /// The client for the key in `LINEAR_API_KEY`, else the keychain item `rastro-linear` the
    /// connector reads, at `RASTRO_LINEAR_API_URL` when it is set. Fails, saying where it looked
    /// and never what it found, when there is no key.
    pub fn from_env() -> Result<Self> {
        let key = api_key().context(
            "Linear authentication failed: no key in LINEAR_API_KEY or the rastro-linear keychain item",
        )?;
        let base = std::env::var("RASTRO_LINEAR_API_URL").unwrap_or_else(|_| DEFAULT_API.into());
        // No redirects, so the key can't follow a response to another host.
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        Ok(Self {
            agent,
            endpoint: format!("{}/graphql", base.trim_end_matches('/')),
            key,
        })
    }

    /// Creates the issue under the Linear id of the team its key names and, for the assignee `me`,
    /// the key owner's user id; any other assignee is sent as it is. Fails, sending no create,
    /// when Linear holds no team by the key. Fails, without the key in the message, when Linear is
    /// unreachable, answers with an error or a status that is not a success, or does not confirm
    /// the issue.
    pub fn create_issue(&self, issue: &Issue) -> Result<Created> {
        let team = self.team_id(issue.team)?;
        let assignee = match issue.assignee {
            ME => self.owner()?,
            other => other.to_string(),
        };
        let data = self.query(
            ISSUE_CREATE,
            json!({"input": issue.input(&team, &assignee)}),
        )?;
        let created = &data["issueCreate"];
        if created["success"] != true {
            bail!("Linear did not confirm the issue");
        }
        confirmed(&created["issue"])
    }

    /// The issue created under `id`, or `None` when Linear holds no issue by that id. Fails when
    /// Linear cannot be asked, or answers with anything but the issue or its absence.
    pub fn find_issue(&self, id: &str) -> Result<Option<Created>> {
        match self.query(ISSUE_LOOKUP, json!({"id": id})) {
            Ok(data) if data["issue"].is_null() => Ok(None),
            Ok(data) => confirmed(&data["issue"]).map(Some),
            Err(error) if error.is::<NotFound>() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Deletes the issue created under `id`, unless Linear already holds none by that id. Returns
    /// whether it deleted one: `false` means the issue was already gone, and nothing was sent but
    /// the lookup. Fails, sending no delete, when anyone but the key's owner commented on or
    /// edited the issue, or when its comments and history cannot be read whole. Fails when Linear
    /// cannot be asked or does not confirm the delete.
    pub fn delete_issue(&self, id: &str) -> Result<bool> {
        // A trashed issue is already deleted: a second delete would be one too many.
        if self.find_issue(id)?.is_none_or(|issue| issue.trashed) {
            return Ok(false);
        }
        self.refuse_if_touched(id)?;
        let data = self.query(ISSUE_DELETE, json!({"id": id}))?;
        if data["issueDelete"]["success"] != true {
            bail!("Linear did not confirm the delete");
        }
        Ok(true)
    }

    /// Reads the issue's whole comment and change history, page by page, and fails naming the
    /// first user other than the key's owner who commented on or edited it. Fails as unchecked
    /// when the owner, or any page, cannot be read.
    fn refuse_if_touched(&self, id: &str) -> Result<()> {
        let owner = self.owner().context(UNCHECKED)?;
        let comments = self
            .pages(ISSUE_COMMENTS, "comments", id)
            .context(UNCHECKED)?;
        let edits = self
            .pages(ISSUE_HISTORY, "history", id)
            .context(UNCHECKED)?;
        for comment in &comments {
            if let Some(who) = stranger(&comment["user"], &comment["botActor"], &owner) {
                let body = comment["body"].as_str().unwrap_or_default();
                bail!(
                    "{who} commented on the issue (\"{}\"), so it was left alone",
                    excerpt(body)
                );
            }
        }
        for edit in &edits {
            // A grouped entry names its actor and, apart, everyone whose description edits joined it.
            let editors = edit["descriptionUpdatedBy"]
                .as_array()
                .into_iter()
                .flatten();
            let who = std::iter::once(&edit["actor"])
                .chain(editors)
                .find_map(|user| stranger(user, &edit["botActor"], &owner));
            if let Some(who) = who {
                bail!(
                    "{who} edited the issue ({}), so it was left alone",
                    change(edit)
                );
            }
        }
        Ok(())
    }

    /// The Linear id of the team whose key is `key`. Fails, naming the key, when Linear holds none.
    fn team_id(&self, key: &str) -> Result<String> {
        let data = self.query(TEAM_BY_KEY, json!({"key": key}))?;
        let teams = data["teams"]["nodes"].as_array();
        let found = teams.into_iter().flatten().find(|team| team["key"] == key);
        match found.and_then(|team| team["id"].as_str()) {
            Some(id) => Ok(id.into()),
            None => bail!("Linear holds no team with the key {key}"),
        }
    }

    /// The id of the user the key belongs to.
    fn owner(&self) -> Result<String> {
        let data = self.query(VIEWER, json!({}))?;
        match data["viewer"]["id"].as_str() {
            Some(id) => Ok(id.into()),
            None => bail!("Linear did not say whose key this is"),
        }
    }

    /// Every node of the issue's connection `field`, following its pages to the last. Fails when a
    /// page cannot be read or says there is more without saying where.
    fn pages(&self, document: &str, field: &str, id: &str) -> Result<Vec<Value>> {
        let mut nodes = Vec::new();
        let mut after = Value::Null;
        for _ in 0..MAX_PAGES {
            let data = self.query(document, json!({"id": id, "after": after}))?;
            let page = &data["issue"][field];
            let Some(found) = page["nodes"].as_array() else {
                bail!("Linear answered without the issue's {field}");
            };
            nodes.extend(found.iter().cloned());
            if page["pageInfo"]["hasNextPage"] != true {
                return Ok(nodes);
            }
            after = match page["pageInfo"]["endCursor"].as_str() {
                Some(cursor) => cursor.into(),
                None => bail!("Linear did not say where the issue's {field} go on"),
            };
        }
        bail!("the issue's {field} did not end")
    }

    /// Runs one document and returns its `data`. A response with GraphQL `errors` is a failure,
    /// even when it carries partial data.
    fn query(&self, query: &str, variables: Value) -> Result<Value> {
        let mut response = self
            .agent
            .post(&self.endpoint)
            // A personal API key goes as is, without a scheme.
            .header("Authorization", &self.key)
            .send_json(json!({"query": query, "variables": variables}))
            .map_err(|error| match error {
                ureq::Error::Timeout(_) => anyhow::anyhow!("Linear did not answer in time"),
                _ => anyhow::anyhow!("Linear could not be reached"),
            })?;
        let status = response.status().as_u16();
        let body: Option<Value> = response.body_mut().read_json().ok();
        if let Some(message) = (body.as_ref())
            .and_then(|body| body["errors"].as_array())
            .and_then(|errors| errors.first())
        {
            let code = message["extensions"]["code"].as_str().unwrap_or("unknown");
            let not_found = (message["message"].as_str())
                .is_some_and(|text| text.to_lowercase().starts_with("entity not found"));
            if not_found {
                return Err(NotFound(code.into()).into());
            }
            bail!("Linear refused the request: {code}");
        }
        if !(200..300).contains(&status) {
            bail!("Linear answered with status {status}");
        }
        match body {
            Some(Value::Object(mut body)) => match body.remove("data") {
                Some(data @ Value::Object(_)) => Ok(data),
                _ => bail!("Linear answered without data"),
            },
            _ => bail!("Linear answered with something that is not JSON"),
        }
    }
}

/// The identifier and URL of an issue Linear confirmed.
fn confirmed(issue: &Value) -> Result<Created> {
    let text = |field: &str| issue[field].as_str().map(String::from);
    match (text("identifier"), text("url")) {
        (Some(identifier), Some(url)) => Ok(Created {
            identifier,
            url,
            // Only `trashed` says deleted: Linear archives a closed issue too, and it is still there.
            trashed: issue["trashed"] == true,
        }),
        _ => bail!("Linear confirmed the issue without its identifier and url"),
    }
}

/// The name of `user` when they are not the owner. Linear names no user for an automated or
/// integration change, and credits a bot instead: the bot's name stands in, or a last stand-in
/// when Linear names neither.
fn stranger(user: &Value, bot: &Value, owner: &str) -> Option<String> {
    match (user["id"].as_str(), bot["name"].as_str()) {
        (Some(id), _) if id == owner => None,
        (Some(_), _) => Some(user["name"].as_str().unwrap_or("another user").into()),
        (None, Some(bot)) => Some(format!("the bot {bot}")),
        (None, None) => Some("an unknown user".into()),
    }
}

/// The start of a comment's text, on one line.
fn excerpt(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match line.char_indices().nth(80) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line,
    }
}

/// What one history entry changed, as far as the history document selects it.
fn change(entry: &Value) -> String {
    let changed = |field: &str| !entry[field].is_null();
    let mut what = Vec::new();
    if let Some(title) = entry["toTitle"].as_str() {
        what.push(format!("title to \"{}\"", excerpt(title)));
    }
    for (name, fields) in [
        ("state", &["fromStateId", "toStateId"]),
        ("assignee", &["fromAssigneeId", "toAssigneeId"]),
        ("priority", &["fromPriority", "toPriority"]),
        ("labels", &["addedLabelIds", "removedLabelIds"]),
    ] {
        if fields.iter().any(|field| changed(field)) {
            what.push(name.into());
        }
    }
    if entry["updatedDescription"] == true {
        what.push("description".into());
    }
    if what.is_empty() {
        "a change to it".into()
    } else {
        format!("changed the {}", what.join(", "))
    }
}
