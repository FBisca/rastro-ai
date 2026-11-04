//! Coverage answers: what a source's last attempts let the core claim, and for how long.
use crate::{Coverage, CoverageState, Pair, Source, UnknownSource};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};

/// The instant coverage is judged at. `RASTRO_NOW` (RFC 3339) overrides the clock so tests
/// can age a check without waiting; only the core reads it.
pub fn now() -> Result<DateTime<Utc>> {
    now_from(crate::test_override("RASTRO_NOW"))
}
fn now_from(knob: Option<String>) -> Result<DateTime<Utc>> {
    match knob {
        Some(text) if !text.is_empty() => {
            text.parse().context("RASTRO_NOW must be an RFC 3339 time")
        }
        _ => Ok(Utc::now()),
    }
}

/// The pairs coverage answers for, each with whether it is in the config: the configured
/// sources first, then the pairs only storage knows. `name` narrows to one source.
pub(crate) fn pairs(
    config: &[Source],
    stored: Vec<Pair>,
    name: Option<&str>,
) -> Result<Vec<(Pair, bool)>> {
    let mut pairs: Vec<(Pair, bool)> = config.iter().map(|s| (Pair::from(s), true)).collect();
    for pair in stored {
        if !pairs
            .iter()
            .any(|(p, _)| p.source == pair.source && p.scope == pair.scope)
        {
            pairs.push((pair, false));
        }
    }
    pairs.retain(|(p, _)| name.is_none_or(|n| p.source == n));
    if pairs.is_empty()
        && let Some(name) = name
    {
        return Err(UnknownSource(name.into()).into());
    }
    Ok(pairs)
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

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    /// evidence/R71: rastro now is ignored unless this is a debug build
    #[test]
    fn rastro_now_is_ignored_unless_this_is_a_debug_build() {
        let knob = || Some("2001-02-03T04:05:06Z".to_string());
        let allowed = |debug| crate::overrides_allowed(debug).then(knob).flatten();
        assert_eq!(
            now_from(allowed(true)).unwrap(),
            "2001-02-03T04:05:06Z".parse::<DateTime<Utc>>().unwrap()
        );
        let ignored = now_from(allowed(false)).unwrap();
        assert!((Utc::now() - ignored).num_seconds().abs() < 5);
    }
}
