//! `tick`: the one entry point a schedule calls. It finishes every accepted Action run that never
//! completed and every pending Act whose grace has ended, syncs every source whose last complete
//! check is older than its freshness rule, extracts the chat threads of every opted-in source
//! that are new or whose latest run failed, and runs at most once at a time.
use crate::{
    CompleteCheck, Locations, Rastro, Source, Storage, coverage, files, host,
    runs::{Silent, StoredTwins},
};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use std::{fs::TryLockError, path::PathBuf};

/// Held for as long as a tick runs, so a second tick can tell it is not alone. It is separate
/// from the sync lock, which a sync inside the tick takes and lets go of on its own.
pub fn lock_path(locations: &Locations) -> PathBuf {
    locations.data.join("tick.lock")
}

/// Whether a source with `check` as its last complete check needs a sync at `now`: one that
/// was never checked does, and one checked a whole freshness rule ago does, the instant
/// coverage stops calling it fresh.
pub fn is_stale(check: Option<&CompleteCheck>, freshness: Duration, now: DateTime<Utc>) -> bool {
    check.is_none_or(|check| now >= check.checked_at + freshness)
}

/// What a tick did.
#[derive(Debug)]
pub enum Tick {
    /// Another tick held the lock, so this one did nothing.
    AlreadyRunning,
    /// This tick held the lock and went over every source.
    Ran(TickReport),
}

/// What a tick that ran went over: the Action runs it finished and the sources.
#[derive(Debug, Default)]
pub struct TickReport {
    /// Action runs this tick executed to `done`. A run another process was executing is left to
    /// it and is not listed.
    pub runs_finished: Vec<String>,
    /// Action runs whose execution failed, each with why. Each reads `failed` with that error until it has made 3 attempts.
    pub runs_failed: Vec<(String, String)>,
    /// Sources synced.
    pub synced: Vec<String>,
    /// Sources within their freshness rule, left alone.
    pub fresh: Vec<String>,
    /// Sources whose sync failed, each with why. The sources after one still synced.
    pub failed: Vec<(String, String)>,
}

impl<S: Storage + StoredTwins> Rastro<S> {
    /// Executes each Action run that is not done and has made fewer than 3 failed attempts — a
    /// pending Act only once its grace has ended — then syncs each source past its freshness rule,
    /// one after another, and skips the fresh ones. A run or a source that fails is recorded in the
    /// report and the next one still goes. After the syncs, every source added with
    /// `--extract-commitments` has its new threads — and any whose latest run failed — extracted
    /// and judged, fresh or just synced; a thread's failed run is recorded and named in the
    /// commitments list, and never fails the tick.
    ///
    /// Returns [`Tick::AlreadyRunning`] without recording anything when another tick holds
    /// the tick lock. Fails only when the lock or the source list cannot be read.
    pub fn tick(&mut self) -> Result<Tick> {
        files::private_directory(&self.locations.data)?;
        let lock = files::private_file(&lock_path(&self.locations))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(Tick::AlreadyRunning),
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
        let now = coverage::now()?;
        let sources: Vec<Source> = self.config.sources.clone();
        let mut report = TickReport::default();
        for run in self.unfinished_runs(now)? {
            match self.execute_run(&run) {
                Ok(Some(_)) => report.runs_finished.push(run.id),
                Ok(None) => {}
                Err(error) => report.runs_failed.push((run.id, format!("{error:#}"))),
            }
        }
        for source in &sources {
            let check = self.storage.last_complete_check(source)?;
            if !is_stale(check.as_ref(), source.freshness(), now) {
                report.fresh.push(source.name.clone());
                continue;
            }
            match self.sync_observed(Some(&source.name), false, &mut Silent) {
                Ok(synced) => match synced.runs.iter().find(|run| run.status == "failed") {
                    Some(run) => report.failed.push((
                        source.name.clone(),
                        run.reason.clone().unwrap_or_else(|| "failed".into()),
                    )),
                    None => report.synced.push(source.name.clone()),
                },
                Err(error) => report
                    .failed
                    .push((source.name.clone(), format!("{error:#}"))),
            }
        }
        // The opted-in sources' new threads, after the syncs: a manual sync may have stored
        // them just now, so a fresh source is extracted too. A pass-level failure is a
        // diagnostic; each thread's own failure is recorded as its run.
        for source in &sources {
            if !source.extract_commitments {
                continue;
            }
            if let Err(error) = self.extract_chat_threads(source) {
                host::notice(&format!(
                    "chat extraction for source {} failed: {error:#}",
                    source.name
                ));
            }
        }
        drop(lock);
        Ok(Tick::Ran(report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_at(at: &str) -> CompleteCheck {
        let at: DateTime<Utc> = at.parse().unwrap();
        CompleteCheck {
            window_end: at,
            checked_at: at,
        }
    }

    /// spec: One tick syncs every stale source
    #[test]
    fn a_source_is_stale_when_never_checked_or_a_whole_rule_has_passed() {
        let hour = Duration::hours(1);
        let now: DateTime<Utc> = "2026-11-19T12:00:00Z".parse().unwrap();
        assert!(is_stale(None, hour, now));
        assert!(!is_stale(
            Some(&check_at("2026-11-19T11:00:01Z")),
            hour,
            now
        ));
        assert!(is_stale(Some(&check_at("2026-11-19T11:00:00Z")), hour, now));
    }
}
