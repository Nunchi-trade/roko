//! Per-provider request throttling for HTTP-backed agents.
//!
//! This limiter is keyed by provider ID so concurrent tasks can share a single
//! client-side rate budget for each upstream provider.

use std::num::NonZeroU32;

use governor::{Quota, RateLimiter, clock::DefaultClock, state::keyed::DefaultKeyedStateStore};

#[cfg(test)]
use std::time::Instant;

/// Async keyed rate limiter using a shared requests-per-minute budget.
#[derive(Debug)]
pub struct ProviderRateLimiter {
    rpm_limiter: RateLimiter<String, DefaultKeyedStateStore<String>, DefaultClock>,
}

impl ProviderRateLimiter {
    /// Construct a keyed limiter with a shared default RPM budget.
    #[must_use]
    pub fn new(default_rpm: u32) -> Self {
        let default_rpm = NonZeroU32::new(default_rpm)
            .unwrap_or_else(|| NonZeroU32::new(60).expect("default RPM must be non-zero"));
        Self {
            rpm_limiter: RateLimiter::keyed(Quota::per_minute(default_rpm)),
        }
    }

    /// Wait until the next request for `provider_id` can proceed.
    pub async fn acquire(&self, provider_id: &str) {
        self.rpm_limiter
            .until_key_ready(&provider_id.to_string())
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::join;

    fn test_limiter_per_second(rps: u32) -> ProviderRateLimiter {
        let rps = NonZeroU32::new(rps).expect("test RPS must be non-zero");
        ProviderRateLimiter {
            rpm_limiter: RateLimiter::keyed(Quota::per_second(rps)),
        }
    }

    #[tokio::test]
    async fn provider_rate_limiter_uses_default_rpm_when_zero() {
        let limiter = ProviderRateLimiter::new(0);
        let start = Instant::now();

        limiter.acquire("zai").await;
        limiter.acquire("zai").await;

        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "fallback limiter should not block early requests"
        );
    }

    #[tokio::test]
    async fn provider_rate_limiter_spreads_rapid_requests_for_same_provider() {
        let limiter = test_limiter_per_second(5);
        let start = Instant::now();

        for _ in 0..10 {
            limiter.acquire("zai").await;
        }

        let elapsed = start.elapsed();
        assert!(
            elapsed >= std::time::Duration::from_millis(900),
            "expected throttling to spread 10 requests, got {elapsed:?}"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(4),
            "expected test limiter to finish promptly, got {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn provider_rate_limiter_tracks_each_provider_independently() {
        let limiter = test_limiter_per_second(1);
        let start = Instant::now();

        let ((), ()) = join!(limiter.acquire("zai"), limiter.acquire("openrouter"));

        assert!(
            start.elapsed() < std::time::Duration::from_millis(250),
            "different providers should not contend for the same budget"
        );
    }
}
