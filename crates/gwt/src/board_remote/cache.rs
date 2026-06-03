//! Time-window cache for remote read results (SPEC-2963 FR-009).
//!
//! Slack's `conversations.history` / `conversations.replies` are rate-limited to
//! 1 request/minute for non-Marketplace apps (from 2026-03-03). To keep
//! hook-driven read injection functional, remote read results are reused for a
//! short window. Time is passed in explicitly so the cache is deterministically
//! testable.

use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};

/// A single-slot cache that returns its value only within `ttl` of the last
/// `put`. Cheap and lock-guarded; intended for one snapshot per provider.
pub struct TimedCache<T> {
    ttl: Duration,
    slot: Mutex<Option<(DateTime<Utc>, T)>>,
}

impl<T: Clone> TimedCache<T> {
    /// Create a cache whose entries are valid for `ttl_seconds`.
    pub fn new(ttl_seconds: i64) -> Self {
        Self {
            ttl: Duration::seconds(ttl_seconds.max(0)),
            slot: Mutex::new(None),
        }
    }

    /// Return the cached value if it was stored within the TTL of `now`.
    pub fn get(&self, now: DateTime<Utc>) -> Option<T> {
        let guard = self.slot.lock().ok()?;
        let (stored_at, value) = guard.as_ref()?;
        if now.signed_duration_since(*stored_at) < self.ttl {
            Some(value.clone())
        } else {
            None
        }
    }

    /// Store `value` stamped at `now`.
    pub fn put(&self, now: DateTime<Utc>, value: T) {
        if let Ok(mut guard) = self.slot.lock() {
            *guard = Some((now, value));
        }
    }

    /// Drop any cached value (e.g. after a successful post).
    pub fn invalidate(&self) {
        if let Ok(mut guard) = self.slot.lock() {
            *guard = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    #[test]
    fn returns_value_within_ttl() {
        let cache = TimedCache::new(60);
        cache.put(at(1_000), vec![1, 2, 3]);
        assert_eq!(cache.get(at(1_030)), Some(vec![1, 2, 3]));
    }

    #[test]
    fn expires_after_ttl() {
        let cache = TimedCache::new(60);
        cache.put(at(1_000), vec![1]);
        assert_eq!(cache.get(at(1_061)), None);
    }

    #[test]
    fn boundary_at_exactly_ttl_is_expired() {
        let cache = TimedCache::new(60);
        cache.put(at(1_000), 7);
        // exactly ttl elapsed → not < ttl → expired.
        assert_eq!(cache.get(at(1_060)), None);
        assert_eq!(cache.get(at(1_059)), Some(7));
    }

    #[test]
    fn invalidate_clears_value() {
        let cache = TimedCache::new(60);
        cache.put(at(1_000), 1);
        cache.invalidate();
        assert_eq!(cache.get(at(1_001)), None);
    }

    #[test]
    fn empty_cache_returns_none() {
        let cache: TimedCache<i32> = TimedCache::new(60);
        assert_eq!(cache.get(at(1)), None);
    }
}
