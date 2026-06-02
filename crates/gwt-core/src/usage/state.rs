//! Usage freshness and polling-cadence helpers (SPEC-2970 FR-006/FR-007/FR-012).
//!
//! Pure functions with injected `now` so the state machine is deterministic
//! under test.

use chrono::{DateTime, Duration, Utc};

use super::types::UsageState;

/// Minimum interval between Claude account-usage fetches. The undocumented
/// `/api/oauth/usage` endpoint 429s aggressively below ~180s, so this is a
/// hard floor.
pub const CLAUDE_MIN_FETCH_SECS: i64 = 180;

/// Data older than this is rendered as [`UsageState::Stale`]. Chosen well above
/// the Claude floor so a single skipped poll does not flap the UI.
pub const DEFAULT_STALE_AFTER_SECS: i64 = 600;

/// Whether a Claude account fetch is allowed now given the last fetch instant.
/// Returns `true` when never fetched or when at least [`CLAUDE_MIN_FETCH_SECS`]
/// have elapsed.
pub fn should_fetch_claude(last_fetch: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    match last_fetch {
        None => true,
        Some(last) => now.signed_duration_since(last) >= Duration::seconds(CLAUDE_MIN_FETCH_SECS),
    }
}

/// Age in whole seconds since `fetched_at`, clamped to `0`.
pub fn age_secs(fetched_at: DateTime<Utc>, now: DateTime<Utc>) -> u64 {
    now.signed_duration_since(fetched_at).num_seconds().max(0) as u64
}

/// True when `fetched_at` is older than `stale_after_secs`.
pub fn is_stale(fetched_at: DateTime<Utc>, now: DateTime<Utc>, stale_after_secs: i64) -> bool {
    now.signed_duration_since(fetched_at) > Duration::seconds(stale_after_secs)
}

/// Promote an `Ok` state to `Stale` when its data has aged past the threshold.
/// Non-`Ok` states pass through unchanged.
pub fn apply_staleness(
    state: UsageState,
    fetched_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    stale_after_secs: i64,
) -> UsageState {
    match (&state, fetched_at) {
        (UsageState::Ok, Some(at)) if is_stale(at, now, stale_after_secs) => UsageState::Stale {
            age_secs: age_secs(at, now),
        },
        _ => state,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_780_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn first_fetch_always_allowed() {
        assert!(should_fetch_claude(None, t(0)));
    }

    #[test]
    fn fetch_blocked_within_floor() {
        assert!(!should_fetch_claude(Some(t(0)), t(120)));
        assert!(should_fetch_claude(Some(t(0)), t(180)));
        assert!(should_fetch_claude(Some(t(0)), t(500)));
    }

    #[test]
    fn age_never_negative() {
        assert_eq!(age_secs(t(100), t(50)), 0);
        assert_eq!(age_secs(t(0), t(42)), 42);
    }

    #[test]
    fn ok_becomes_stale_after_threshold() {
        let out = apply_staleness(UsageState::Ok, Some(t(0)), t(700), DEFAULT_STALE_AFTER_SECS);
        assert_eq!(out, UsageState::Stale { age_secs: 700 });
    }

    #[test]
    fn fresh_ok_unchanged() {
        let out = apply_staleness(UsageState::Ok, Some(t(0)), t(120), DEFAULT_STALE_AFTER_SECS);
        assert_eq!(out, UsageState::Ok);
    }

    #[test]
    fn non_ok_states_pass_through() {
        let out = apply_staleness(
            UsageState::Disabled,
            Some(t(0)),
            t(10_000),
            DEFAULT_STALE_AFTER_SECS,
        );
        assert_eq!(out, UsageState::Disabled);
    }
}
