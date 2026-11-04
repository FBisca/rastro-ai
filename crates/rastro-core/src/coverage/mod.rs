//! Coverage answers: what a source's last attempts let the core claim, and for how long.
use crate::{Coverage, CoverageState};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use std::env;

/// The instant coverage is judged at. `RASTRO_NOW` (RFC 3339) overrides the clock so tests
/// can age a check without waiting; only the core reads it.
pub fn now() -> Result<DateTime<Utc>> {
    match env::var("RASTRO_NOW") {
        Ok(text) if !text.is_empty() => text.parse().context("RASTRO_NOW must be an RFC 3339 time"),
        _ => Ok(Utc::now()),
    }
}

impl Coverage {
    /// Applies the freshness rule to what storage recorded. Past the rule, or after a failed
    /// latest attempt, the answer is "unknown"; the last successful check and the failure
    /// reason stay visible.
    pub fn judged(mut self, freshness: Duration, now: DateTime<Utc>) -> Self {
        self.fresh_until = self.last_checked.map(|at| at + freshness);
        let stale = self.fresh_until.is_some_and(|until| now >= until);
        match self.state {
            CoverageState::Found | CoverageState::Empty if stale => {
                self.state = CoverageState::Unknown;
            }
            CoverageState::CouldNotLook => self.state = CoverageState::Unknown,
            _ => {}
        }
        self
    }
}
