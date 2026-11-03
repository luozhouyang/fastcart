//! Bounded in-process rate limiting backed by `governor` and `moka`.
//!
//! Keys are kept in a bounded, idle-expiring cache so attacker-controlled
//! identifiers cannot grow process memory without limit.

use std::fmt;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use governor::clock::DefaultClock;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter as GovernorLimiter};
use moka::sync::Cache;

type DirectLimiter = GovernorLimiter<NotKeyed, InMemoryState, DefaultClock>;

/// A keyed limiter allowing `limit` events per `window`.
#[derive(Clone)]
pub struct RateLimiter {
    entries: Cache<String, Arc<DirectLimiter>>,
    quota: Quota,
    limit: usize,
    window: Duration,
}

impl fmt::Debug for RateLimiter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RateLimiter")
            .field("limit", &self.limit)
            .field("window", &self.window)
            .field("entries", &self.entries.entry_count())
            .finish()
    }
}

impl RateLimiter {
    /// Creates a limiter allowing `limit` events per `window`.
    #[must_use]
    pub fn new(limit: usize, window: Duration) -> Self {
        let limit = limit.max(1);
        let limit_u32 = u32::try_from(limit).unwrap_or(u32::MAX);
        let burst = NonZeroU32::new(limit_u32).expect("limit is non-zero");
        let period = window
            .checked_div(limit_u32)
            .filter(|period| !period.is_zero())
            .unwrap_or(Duration::from_secs(1));
        let quota = Quota::with_period(period)
            .expect("period is non-zero")
            .allow_burst(burst);
        let entries = Cache::builder()
            .max_capacity(100_000)
            .time_to_idle(window.saturating_mul(2))
            .build();
        Self {
            entries,
            quota,
            limit,
            window,
        }
    }

    /// Records an attempt and returns true when it is allowed.
    pub fn check(&self, key: &str) -> bool {
        let limiter = self.entries.get_with(key.to_owned(), || {
            Arc::new(GovernorLimiter::direct(self.quota))
        });
        limiter.check().is_ok()
    }

    /// Removes all recorded attempts.
    pub fn clear(&self) {
        self.entries.invalidate_all();
    }

    /// Removes attempts for one key.
    pub fn reset(&self, key: &str) {
        self.entries.invalidate(key);
    }

    /// Maximum number of attempts per window.
    #[must_use]
    pub fn limit(&self) -> usize {
        self.limit
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::RateLimiter;

    #[test]
    fn blocks_after_the_limit() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        assert!(limiter.check("ip"));
        assert!(limiter.check("ip"));
        assert!(!limiter.check("ip"));
        assert!(limiter.check("other"));
    }

    #[test]
    fn reset_allows_attempts_again() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        assert!(limiter.check("ip"));
        assert!(!limiter.check("ip"));
        limiter.reset("ip");
        assert!(limiter.check("ip"));
    }

    #[test]
    fn keys_are_bounded() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        for index in 0..1_000 {
            assert!(limiter.check(&format!("key-{index}")));
        }
        assert!(limiter.entries.entry_count() <= 100_000);
    }
}
