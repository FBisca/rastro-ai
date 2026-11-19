//! Action runs: what the owner's accept of an action proposal starts.
//!
//! Accepting records the accept intent and one run in a single transaction. The run holds the
//! random issue id the outside system is asked to create the issue under, fixed before any
//! request is sent, so what the run did can always be found by it. Executing a run takes the run's
//! lock, asks the provider whether the issue under that id exists and only if it does not to create
//! it, and records the issue's identifier and URL; a run with such a result reads `done`, and one
//! without reads `accepted`.
//!
//! Only [`crate::actions::IssueFields`], the content the owner accepted, and the run's random
//! issue id go to the provider: the why, the citations and every Rastro id stay here.
use super::IssueFields;
use crate::queue::Source as QueueSource;
use crate::{Rastro, Storage, files, host};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use rastro_plugin_sdk::{ACTION_EXECUTE, ACTION_LOOKUP, ActionCall, ActionResult};
use schemars::JsonSchema;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::time::Duration;
use std::{collections::BTreeMap, fs::TryLockError};

/// How long a provider may take to answer one action message.
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(60);

/// Where a run stands, derived from what is recorded for it.
#[derive(Clone, Copy, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// The accept is recorded and the issue has not been created.
    Accepted,
    /// The provider created the issue, and the run holds its identifier and URL.
    Done,
}

impl RunStatus {
    /// The word the owner reads and the JSON carries.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Done => "done",
        }
    }
}

/// An action run, with its status and, once done, the issue it created.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[schemars(rename = "ActionRun")]
pub struct Run {
    pub id: String,
    /// The queue item the owner accepted.
    pub proposal: String,
    /// The owner intent that accepted it.
    pub accept_intent: String,
    pub action: String,
    /// The fields the owner accepted: the content written for the outside system.
    pub fields: IssueFields,
    /// The source the run writes to, with that source's scope.
    pub source: QueueSource,
    /// The plugin id of the provider that executes it.
    pub provider: String,
    /// The random id the issue is created under. UUID v4.
    pub issue_id: String,
    pub accepted_at: DateTime<Utc>,
    pub status: RunStatus,
    /// The created issue's identifier; present once the run is done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    /// The created issue's URL; present once the run is done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// What an accept records beside its intent: the run it starts. Storage adds the intent's time.
#[derive(Clone, Debug)]
pub struct NewRun {
    pub id: String,
    pub proposal: String,
    /// The id of the accept intent recorded in the same transaction.
    pub accept_intent: String,
    pub action: String,
    pub provider: String,
    pub fields: IssueFields,
    pub source: QueueSource,
    pub issue_id: String,
}

/// What a run records once its issue exists.
#[derive(Clone, Debug)]
pub struct RunResult {
    pub identifier: String,
    pub url: String,
    pub finished_at: DateTime<Utc>,
}

/// The id of the run an accept intent starts: its content address, so the same accept is the same
/// run.
pub(crate) fn run_id(accept_intent: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"rastro/action-run/v1");
    hash.update(accept_intent.as_bytes());
    let hex: String = hash.finalize()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("r_{hex}")
}

/// A random UUID v4: the id a run's issue is created under.
pub(crate) fn random_issue_id() -> Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut bytes))
        .context("reading random bytes for the issue id")?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        hex[..4].concat(),
        hex[4..6].concat(),
        hex[6..8].concat(),
        hex[8..10].concat(),
        hex[10..].concat()
    ))
}

/// The fields a provider is sent for an issue: the accepted content and nothing else.
fn call_fields(fields: &IssueFields) -> BTreeMap<String, String> {
    let mut sent = BTreeMap::from([
        ("title".to_string(), fields.title.clone()),
        ("team".to_string(), fields.team.clone()),
        ("assignee".to_string(), fields.assignee.clone()),
    ]);
    if let Some(description) = &fields.description {
        sent.insert("description".into(), description.clone());
    }
    sent
}

/// The lock that lets one process at a time execute a run: a file under the data directory named
/// for the run, held while it executes.
fn lock_path(locations: &crate::Locations, run: &str) -> std::path::PathBuf {
    locations.data.join(format!("action-{run}.lock"))
}

impl<S: Storage> Rastro<S> {
    /// The runs that were accepted and have not finished, oldest first: what `tick` finishes.
    pub(crate) fn unfinished_runs(&self) -> Result<Vec<Run>> {
        let runs = self.storage.snapshot(|view| view.action_runs(&[]))?;
        Ok(runs
            .into_iter()
            .filter(|run| run.status == RunStatus::Accepted)
            .collect())
    }

    /// Executes `run` so that its issue exists once and the run reads `done`, and returns the run
    /// as it then reads. Only one process executes a run at a time: when another holds it, this
    /// leaves the run untouched and returns `None`.
    ///
    /// Under the lock it reads the run again, and a run already done is returned as it is. Then it
    /// asks the provider whether the issue under the run's id exists, and only when it does not
    /// asks it to create it, so a run cut off before it sent anything is created, and one whose
    /// answer was lost is found, never created twice. Fails, recording nothing, when the provider
    /// is refused, fails or does not confirm the issue.
    pub(crate) fn execute_run(&mut self, run: &Run) -> Result<Option<Run>> {
        files::private_directory(&self.locations.data)?;
        let lock = files::private_file(&lock_path(&self.locations, &run.id))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
        let current = self.stored_run(&run.id)?;
        if current.status == RunStatus::Done {
            return Ok(Some(current));
        }
        let call = ActionCall {
            action: run.action.clone(),
            key: run.issue_id.clone(),
            fields: call_fields(&run.fields),
        };
        let mut answer = self.ask_provider(run, ACTION_LOOKUP, &call)?;
        if !answer.found {
            answer = self.ask_provider(run, ACTION_EXECUTE, &call)?;
        }
        let (true, Some(identifier), Some(url)) = (answer.found, answer.identifier, answer.url)
        else {
            bail!(
                "the provider {} did not confirm the issue for run {}",
                run.provider,
                run.id
            );
        };
        let result = RunResult {
            identifier,
            url,
            finished_at: crate::coverage::now()?,
        };
        self.storage.record_action_result(&run.id, &result)?;
        let done = self.stored_run(&run.id)?;
        drop(lock);
        Ok(Some(done))
    }

    fn ask_provider(&self, run: &Run, method: &str, call: &ActionCall) -> Result<ActionResult> {
        host::actions::call(
            &self.locations,
            &run.provider,
            method,
            call,
            PROVIDER_TIMEOUT,
        )
    }

    /// The run `id`, as it reads now.
    fn stored_run(&self, id: &str) -> Result<Run> {
        let runs = self.storage.snapshot(|view| view.action_runs(&[]))?;
        runs.into_iter()
            .find(|run| run.id == id)
            .with_context(|| format!("run {id} is not recorded"))
    }
}
