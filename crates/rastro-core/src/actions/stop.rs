//! Stopping a pending Act: the owner ends the run before its grace ends.
//!
//! The stop is one owner intent in the insert-only intent table, naming the run and its source's
//! scope: a run with such an intent reads `stopped` and never executes. Nothing is sent to the
//! outside system, and the run's proposal leaves the queue for good. A run that is not pending
//! is refused with nothing recorded: a done run points to `rastro actions undo`.
use super::runs::lock_path;
use super::{Run, RunStatus};
use crate::identity::{self, IntentKind, Payload, Targets};
use crate::{Rastro, Storage, files};
use anyhow::{Result, bail};
use std::fs::TryLockError;

impl<S: Storage> Rastro<S> {
    /// Stops the pending Act run `run`: the owner's stop intent is recorded in the intent table,
    /// and the run is returned as it then reads, `stopped`. It never executes, and nothing is
    /// sent to the outside system.
    ///
    /// Fails, with nothing recorded, when no such run is recorded, when it is not `pending` (a
    /// done run's error names `rastro actions undo <run>`; any other status is named), or when
    /// another process is executing it.
    pub fn stop_action(&mut self, run: &str) -> Result<Run> {
        files::private_directory(&self.locations.data)?;
        let lock = files::private_file(&lock_path(&self.locations, run))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                bail!("run {run} is being executed by another process: try again in a moment")
            }
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
        let current = self.storage.snapshot(|view| {
            let Some(current) = (view.action_runs(&[])?)
                .into_iter()
                .find(|candidate| candidate.id == run)
            else {
                bail!("no action run {run} is recorded: list the runs with `actions`");
            };
            Ok(current)
        })?;
        if current.status != RunStatus::Pending {
            if current.status == RunStatus::Done {
                bail!(
                    "run {run} is done: only a pending Act can be stopped; undo it with \
                     `rastro actions undo {run}`"
                );
            }
            bail!(
                "run {run} is {}: only a pending Act can be stopped",
                current.status.as_str()
            );
        }
        self.issue(|view, _| {
            let now = (view.action_runs(&[])?)
                .into_iter()
                .find(|candidate| candidate.id == run);
            if now.is_none_or(|now| now.status != RunStatus::Pending) {
                bail!("run {run} changed while it was being stopped");
            }
            let targets = Targets::Proposal {
                version: identity::SHAPE_VERSION,
                item: Some(current.id.clone()),
                text: None,
                seen: None,
                edited: None,
                subject: None,
                parties: Vec::new(),
                terms: Vec::new(),
                scope: Some(current.source.scope.clone()),
            };
            let intent = identity::new_intent(
                IntentKind::Stop,
                String::new(),
                targets,
                Payload::empty(),
                None,
            )?;
            Ok((vec![intent], None, false))
        })?;
        drop(lock);
        self.stored_run(run)
    }
}
