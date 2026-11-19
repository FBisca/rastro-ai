//! A blocking Linear GraphQL client that only sends the provider's fixed documents.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::time::Duration;

const DEFAULT_API: &str = "https://api.linear.app";

/// The one document an issue create sends. Linear takes the issue's id from the input, so a
/// repeated create with the same id names the same issue.
const ISSUE_CREATE: &str = "mutation IssueCreate($input: IssueCreateInput!) { \
    issueCreate(input: $input) { success issue { id identifier url } } }";

/// What Linear says of an issue it created.
#[derive(Debug, Eq, PartialEq)]
pub struct Created {
    pub identifier: String,
    pub url: String,
}

/// The content an issue is created with: what the owner accepted, and nothing else.
pub struct Issue<'a> {
    /// The run's random id, which Linear keeps as the issue's own.
    pub id: &'a str,
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub team: &'a str,
    pub assignee: &'a str,
}
impl Issue<'_> {
    fn input(&self) -> Value {
        let mut input = json!({
            "id": self.id,
            "title": self.title,
            "teamId": self.team,
            "assigneeId": self.assignee,
        });
        if let Some(description) = self.description {
            input["description"] = description.into();
        }
        input
    }
}

pub struct Client {
    agent: ureq::Agent,
    endpoint: String,
    key: String,
}
impl Client {
    /// The client for the key in `LINEAR_API_KEY`, at `RASTRO_LINEAR_API_URL` when it is set.
    /// Fails, saying so, when there is no key.
    pub fn from_env() -> Result<Self> {
        let key = std::env::var("LINEAR_API_KEY")
            .ok()
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty())
            .context("LINEAR_API_KEY is not set")?;
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

    /// Creates the issue. Fails, without the key in the message, when Linear is unreachable,
    /// answers with an error or a status that is not a success, or does not confirm the issue.
    pub fn create_issue(&self, issue: &Issue) -> Result<Created> {
        let data = self.query(ISSUE_CREATE, json!({"input": issue.input()}))?;
        let created = &data["issueCreate"];
        if created["success"] != true {
            bail!("Linear did not confirm the issue");
        }
        let text = |field: &str| created["issue"][field].as_str().map(String::from);
        match (text("identifier"), text("url")) {
            (Some(identifier), Some(url)) => Ok(Created { identifier, url }),
            _ => bail!("Linear confirmed the issue without its identifier and url"),
        }
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
