//! Undoing an action run: the owner has the provider delete what the run created.
//!
//! A run is undone only when it is `done`: the provider is asked to undo it, and only when it
//! answers is the owner's undo intent recorded, so a run reads `undone` exactly when its provider
//! has undone it. An answer that never arrived records nothing and leaves the run `done`; undoing
//! it again asks the provider again, which finds the thing already gone and does not undo twice.
use super::runs::lock_path;
use super::{Reversibility, Run, RunStatus};
use crate::identity::{self, IntentKind, Payload, Targets};
use crate::{Rastro, Storage, files};
use anyhow::{Result, bail};
use rastro_plugin_sdk::{ACTION_UNDO, ActionCall};
use std::{collections::BTreeMap, fs::TryLockError};

impl<S: Storage> Rastro<S> {
    /// Undoes the done run `run`: the provider deletes what it created, the owner's undo intent is
    /// recorded in the intent table, and the run is returned as it then reads, `undone`.
    ///
    /// Fails, with nothing recorded and nothing asked of the provider, when no such run is
    /// recorded, when it is not `done` (the error names its status), when its action cannot be
    /// undone, or when another process is executing it. Fails with nothing recorded when the
    /// provider is refused, fails or does not answer, so the run still reads `done` and the undo
    /// can be run again.
    pub fn undo_action(&mut self, run: &str) -> Result<Run> {
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
            let proposal = (view.action_proposals(&[])?)
                .into_iter()
                .find(|filed| filed.id == current.proposal);
            Ok((current, proposal.map(|filed| filed.reversibility)))
        })?;
        let (current, reversibility) = current;
        if current.status != RunStatus::Done {
            bail!(
                "run {run} is {}: only a done run can be undone",
                current.status.as_str()
            );
        }
        if reversibility.is_some_and(|how| how != Reversibility::CanBeUndone) {
            bail!("run {run}'s action {} can't be undone", current.action);
        }
        let call = ActionCall {
            action: current.action.clone(),
            key: current.issue_id.clone(),
            fields: BTreeMap::new(),
        };
        self.ask_provider(&current, ACTION_UNDO, &call)?;
        self.issue(|view, _| {
            let now = (view.action_runs(&[])?)
                .into_iter()
                .find(|candidate| candidate.id == run);
            if now.is_none_or(|now| now.status != RunStatus::Done) {
                bail!("run {run} changed while it was being undone");
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
                IntentKind::UndoAction,
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
