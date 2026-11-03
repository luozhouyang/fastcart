//! Small in-process fixed-window rate limiter for authentication endpoints.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A fixed-window limiter keyed by client or account identifier.
#[derive(Clone, Debug)]
pub struct RateLimiter {
    inner: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
    limit: usize,
    window: Duration,
}

impl RateLimiter {
    /// Creates a limiter allowing `limit` events per `window`.
    #[must_use]
    pub fn new(limit: usize, window: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            limit: limit.max(1),
            window,
        }
    }

    /// Records an attempt and returns true when it is allowed.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut entries = match self.inner.lock() {
            Ok(entries) => entries,
            Err(poisoned) => poisoned.into_inner(),
        };
        let window = self.window;
        entries.retain(|_, attempts| {
            while attempts
                .front()
                .is_some_and(|attempt| now.duration_since(*attempt) > window)
            {
                attempts.pop_front();
            }
            !attempts.is_empty()
        });
        let attempts = entries.entry(key.to_owned()).or_default();
        if attempts.len() >= self.limit {
            return false;
        }
        attempts.push_back(now);
        true
    }

    /// Removes all recorded attempts.
    pub fn clear(&self) {
        if let Ok(mut entries) = self.inner.lock() {
            entries.clear();
        }
    }

    /// Removes attempts for one key.
    pub fn reset(&self, key: &str) {
        if let Ok(mut entries) = self.inner.lock() {
            entries.remove(key);
        }
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
}
