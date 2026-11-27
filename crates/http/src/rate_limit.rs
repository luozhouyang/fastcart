//! Bounded in-process rate limiting backed by `governor` and `moka`.
//!
//! Keys are kept in a bounded, idle-expiring cache so attacker-controlled
//! identifiers cannot grow process memory without limit.

use std::fmt;
use std::fmt::Write as _;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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
    rejections: Arc<AtomicU64>,
}

impl fmt::Debug for RateLimiter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RateLimiter")
            .field("limit", &self.limit)
            .field("window", &self.window)
            .field("entries", &self.entries.entry_count())
            .field("rejections", &self.rejections())
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
            rejections: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Records an attempt and returns true when it is allowed.
    pub fn check(&self, key: &str) -> bool {
        let limiter = self.entries.get_with(key.to_owned(), || {
            Arc::new(GovernorLimiter::direct(self.quota))
        });
        let allowed = limiter.check().is_ok();
        if !allowed {
            self.rejections.fetch_add(1, Ordering::Relaxed);
        }
        allowed
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

    /// Attempts rejected since the process started.
    ///
    /// The counter is cumulative: [`Self::clear`] and [`Self::reset`] only
    /// forget attempt timestamps, not the rejection history.
    #[must_use]
    pub fn rejections(&self) -> u64 {
        self.rejections.load(Ordering::Relaxed)
    }

    /// Keys currently held in the bounded cache.
    ///
    /// The cache reports this approximately while it drains idle entries.
    #[must_use]
    pub fn entries(&self) -> u64 {
        self.entries.entry_count()
    }
}

/// Renders Prometheus gauges and counters for a set of named limiters.
///
/// The caller supplies a fixed list, so limiter labels cannot grow with
/// traffic.
#[must_use]
pub fn render_rate_limit_rejections<'a>(
    limiters: impl IntoIterator<Item = (&'a str, &'a RateLimiter)>,
) -> String {
    let sampled: Vec<(&str, u64, u64)> = limiters
        .into_iter()
        .map(|(name, limiter)| (name, limiter.entries(), limiter.rejections()))
        .collect();
    let mut output = String::new();
    let _ = writeln!(
        output,
        "# HELP fastcart_rate_limit_entries Keys currently tracked by a rate limiter."
    );
    let _ = writeln!(output, "# TYPE fastcart_rate_limit_entries gauge");
    for (name, entries, _) in &sampled {
        let _ = writeln!(
            output,
            "fastcart_rate_limit_entries{{limiter=\"{name}\"}} {entries}"
        );
    }
    let _ = writeln!(
        output,
        "# HELP fastcart_rate_limit_rejections_total Attempts rejected by a rate limiter."
    );
    let _ = writeln!(
        output,
        "# TYPE fastcart_rate_limit_rejections_total counter"
    );
    for (name, _, rejections) in &sampled {
        let _ = writeln!(
            output,
            "fastcart_rate_limit_rejections_total{{limiter=\"{name}\"}} {rejections}"
        );
    }
    output
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

    #[test]
    fn counts_rejections_without_forgetting_them_on_reset() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        assert!(limiter.check("ip"));
        assert!(limiter.check("ip"));
        assert_eq!(limiter.rejections(), 0);
        assert!(!limiter.check("ip"));
        assert!(!limiter.check("ip"));
        assert_eq!(limiter.rejections(), 2);
        assert!(limiter.check("other"));
        assert_eq!(limiter.rejections(), 2, "other keys are independent");

        limiter.reset("ip");
        assert_eq!(limiter.rejections(), 2, "the counter is cumulative");
        limiter.clear();
        assert_eq!(limiter.rejections(), 2);
    }

    #[test]
    fn renders_rejections_and_entries_per_limiter() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        assert!(limiter.check("ip"));
        assert!(!limiter.check("ip"));

        let rendered = super::render_rate_limit_rejections([("login", &limiter)]);
        assert!(rendered.contains("# TYPE fastcart_rate_limit_entries gauge"));
        // `entry_count` is approximate, so only the series name is asserted.
        assert!(rendered.contains("fastcart_rate_limit_entries{limiter=\"login\"} "));
        assert!(rendered.contains("# TYPE fastcart_rate_limit_rejections_total counter"));
        assert!(rendered.contains("fastcart_rate_limit_rejections_total{limiter=\"login\"} 1"));
    }
}
