//! What a sync tells while it runs: the seam `sync_observed` and `host::run` report through,
//! and the observer that prints it on stderr.
use super::RunReport;
use crate::{display_safe, host};
use chrono::{DateTime, SecondsFormat, Utc};
use std::time::{Duration, Instant};

/// Why a source reads the window it reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartReason {
    /// No complete check yet: from the source's `window_start`.
    FirstRead(DateTime<Utc>),
    /// Since the end of the last complete check.
    Incremental(DateTime<Utc>),
    /// `--reread`: from `window_start`, whatever was read before.
    Reread(DateTime<Utc>),
}

/// A step of one source's sync, in the order they happen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Stage {
    /// The connector reads and submits evidence.
    Read,
    /// The extractor reads `evidence` observations. A replay reads stored evidence the read
    /// just now did not bring, as when the extractor's version changed.
    Extract {
        extractor: String,
        version: String,
        evidence: usize,
        replay: bool,
    },
    /// The extractor finished and its `claims` are stored as the source's projection.
    Project { claims: usize },
}

/// Hears a sync as it goes. Every method has a default that ignores the event, and none can
/// fail a run: an observer that can't tell, or won't, changes nothing.
pub trait Observer {
    /// Something the owner should know before a source is read.
    fn warning(&mut self, _text: &str) {}
    /// A source is about to be read.
    fn started(&mut self, _source: &str, _why: &StartReason) {}
    fn stage(&mut self, _source: &str, _stage: &Stage) {}
    /// The connector has submitted `items` observations so far. Called for each one; an
    /// observer that prints decides how often.
    fn read_items(&mut self, _source: &str, _items: u64) {}
    /// The source's run, and its extraction, are over.
    fn finished(&mut self, _report: &RunReport) {}
}

/// Hears nothing.
pub struct Silent;
impl Observer for Silent {}

/// How often a running count may be printed.
pub const COUNT_EVERY: Duration = Duration::from_secs(2);

/// Lets one event through per `every`, the first one only once `every` has passed since the
/// throttle began.
pub struct Throttle {
    every: Duration,
    last: Instant,
}
impl Throttle {
    pub fn new(every: Duration, now: Instant) -> Self {
        Self { every, last: now }
    }
    pub fn allows(&mut self, now: Instant) -> bool {
        let open = now.saturating_duration_since(self.last) >= self.every;
        if open {
            self.last = now;
        }
        open
    }
}

fn stamp(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Prints the sync's progress on stderr, one line per event, through the host's printer: a
/// stderr that is closed, broken or never read neither fails nor stalls the sync.
pub struct StderrProgress {
    count: Option<Throttle>,
}
impl StderrProgress {
    pub fn new() -> Self {
        Self { count: None }
    }
}
impl Default for StderrProgress {
    fn default() -> Self {
        Self::new()
    }
}
impl Observer for StderrProgress {
    fn warning(&mut self, text: &str) {
        host::progress_line(&format!("warning: {}", display_safe(text)));
    }
    fn started(&mut self, source: &str, why: &StartReason) {
        let why = match why {
            StartReason::FirstRead(from) => format!("first read from {}", stamp(*from)),
            StartReason::Incremental(since) => format!("incremental since {}", stamp(*since)),
            StartReason::Reread(from) => format!("re-read from {}", stamp(*from)),
        };
        host::progress_line(&format!("{}: {why}", display_safe(source)));
    }
    fn stage(&mut self, source: &str, stage: &Stage) {
        let source = display_safe(source);
        let line = match stage {
            Stage::Read => {
                self.count = Some(Throttle::new(COUNT_EVERY, Instant::now()));
                format!("{source}: stage read")
            }
            Stage::Extract {
                extractor,
                version,
                evidence,
                replay,
            } => {
                let extractor = display_safe(extractor);
                if *replay {
                    format!(
                        "{source}: stage extract, replaying {evidence} stored evidence \
                         ({extractor} {version})"
                    )
                } else {
                    format!("{source}: stage extract, {evidence} evidence ({extractor} {version})")
                }
            }
            Stage::Project { claims } => format!("{source}: stage project, {claims} claims"),
        };
        host::progress_line(&line);
    }
    fn read_items(&mut self, source: &str, items: u64) {
        if let Some(count) = &mut self.count
            && count.allows(Instant::now())
        {
            host::progress_line(&format!(
                "{}: read {items} items so far",
                display_safe(source)
            ));
        }
    }
    fn finished(&mut self, report: &RunReport) {
        let counts = &report.counts;
        let mut line = format!(
            "{}: {} | submitted {}, accepted {}, unchanged {}",
            display_safe(&report.source),
            report.status,
            counts.submitted,
            counts.accepted,
            counts.unchanged
        );
        if counts.rejected > 0 {
            line += &format!(", rejected {}", counts.rejected);
        }
        if let Some(reason) = &report.reason {
            if reason == "rate_limited" {
                line += " | rate-limited";
                if let Some(at) = report.retry_at {
                    line += &format!(", retry at {}", stamp(at));
                }
            } else {
                line += &format!(" | {}", display_safe(reason));
            }
        }
        host::progress_line(&line);
        host::flush_progress();
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    /// sync-progress/S15: a throttle lets one count through per interval
    #[test]
    fn S15_a_throttle_lets_one_count_through_per_interval() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut throttle = Throttle::new(COUNT_EVERY, start);
        let shown: Vec<u64> = (0..=7000)
            .step_by(100)
            .filter(|ms| throttle.allows(at(*ms)))
            .collect();
        assert_eq!(shown, [2000, 4000, 6000]);
        // Never earlier than a full interval after the last one.
        assert!(shown.windows(2).all(|w| w[1] - w[0] >= 2000));
    }
}
