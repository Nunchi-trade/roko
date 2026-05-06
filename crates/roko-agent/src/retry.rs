//! Retry policy helpers for provider-backed agent requests.

use rand::Rng;

use crate::provider::ProviderError;

/// Canonical error classes used by retry policy configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Provider rejected the request because of rate limiting.
    RateLimit,
    /// Provider rejected the request because authentication failed.
    AuthFailure,
    /// Request timed out before completing.
    Timeout,
    /// Provider returned a transient 5xx-style failure.
    ServerError,
    /// Provider rejected the request for content-policy reasons.
    ContentPolicy,
    /// Request exceeded the model context window.
    ContextOverflow,
    /// Model or deployment slug does not exist.
    ModelNotFound,
    /// Fallback when the exact failure type is not otherwise classified.
    Unknown,
}

impl From<&ProviderError> for ErrorClass {
    fn from(error: &ProviderError) -> Self {
        match error {
            ProviderError::RateLimit { .. } => Self::RateLimit,
            ProviderError::AuthFailure => Self::AuthFailure,
            ProviderError::Timeout => Self::Timeout,
            ProviderError::ServerError(_) => Self::ServerError,
            ProviderError::ContentPolicy => Self::ContentPolicy,
            ProviderError::ContextOverflow => Self::ContextOverflow,
            ProviderError::ModelNotFound => Self::ModelNotFound,
            ProviderError::Other(_) => Self::Unknown,
        }
    }
}

/// Retry policy with AWS-style full-jitter exponential backoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Maximum number of failed attempts that should still trigger retry handling.
    pub max_attempts: u32,
    /// Base backoff delay in milliseconds.
    pub base_delay_ms: u64,
    /// Maximum backoff delay in milliseconds.
    pub max_delay_ms: u64,
    /// Error classes considered retryable by default.
    pub retryable_errors: Vec<ErrorClass>,
}

impl RetryPolicy {
    /// Build a policy tuned for provider rate-limit errors.
    ///
    /// Uses the rate-limit-specific defaults from `roko_core::defaults`:
    /// - 2 s base delay (doubles each attempt: 2 s, 4 s, 8 s, ...)
    /// - 3 max attempts
    /// - 60 s ceiling
    ///
    /// The delay always includes a guaranteed floor of `base_delay_ms / 2`
    /// so that jitter never produces a near-zero wait.
    #[must_use]
    pub fn for_rate_limit() -> Self {
        use roko_core::defaults::{
            DEFAULT_RATE_LIMIT_RETRY_BASE_DELAY_MS, DEFAULT_RATE_LIMIT_RETRY_MAX_BACKOFF_MS,
        };
        Self {
            max_attempts: 3,
            base_delay_ms: DEFAULT_RATE_LIMIT_RETRY_BASE_DELAY_MS, // 2_000
            max_delay_ms: DEFAULT_RATE_LIMIT_RETRY_MAX_BACKOFF_MS, // 60_000
            retryable_errors: vec![ErrorClass::RateLimit],
        }
    }

    /// Returns a full-jitter delay for the given attempt number.
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> u64 {
        let mut rng = rand::thread_rng();
        self.delay_for_attempt_with_rng(attempt, &mut rng)
    }

    fn delay_for_attempt_with_rng<R: Rng + ?Sized>(&self, attempt: u32, rng: &mut R) -> u64 {
        let exp_delay = self.base_delay_ms.saturating_mul(1u64 << attempt.min(10));
        let capped = exp_delay.min(self.max_delay_ms);
        // Guarantee a minimum floor of half the base delay so jitter never
        // produces a near-zero wait (critical for rate-limit backoff).
        let floor = self.base_delay_ms / 2;
        if capped <= floor {
            return capped;
        }
        rng.gen_range(floor..=capped)
    }

    /// Returns whether the given provider error should be retried.
    #[must_use]
    pub fn should_retry(&self, error: &ProviderError, attempt: u32) -> bool {
        if attempt >= self.max_attempts {
            return false;
        }

        match error {
            ProviderError::RateLimit { .. } => true,
            ProviderError::AuthFailure => false,
            ProviderError::ContentPolicy => false,
            ProviderError::Timeout => true,
            ProviderError::ServerError(_) => true,
            ProviderError::ContextOverflow => false,
            _ => attempt < 2,
        }
    }

    /// Returns a provider-specified retry delay when present, otherwise uses full jitter.
    #[must_use]
    pub fn delay_with_retry_after(&self, attempt: u32, retry_after_ms: Option<u64>) -> u64 {
        retry_after_ms.unwrap_or_else(|| self.delay_for_attempt(attempt))
    }

    /// Compute the backoff delay for a rate-limit error at the given attempt.
    ///
    /// Prefers `retry_after_ms` from the provider response when available,
    /// otherwise falls back to exponential backoff with the rate-limit base
    /// delay (2 s * 2^attempt, jittered, floored at base/2).
    #[must_use]
    pub fn rate_limit_delay(&self, attempt: u32, retry_after_ms: Option<u64>) -> u64 {
        if let Some(provider_hint) = retry_after_ms {
            // Respect the provider hint but never go below our floor.
            return provider_hint.max(self.base_delay_ms / 2);
        }
        self.delay_for_attempt(attempt)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        use roko_core::defaults::{
            DEFAULT_RETRY_ATTEMPTS, DEFAULT_RETRY_BASE_DELAY_MS, DEFAULT_RETRY_MAX_BACKOFF_MS,
        };
        Self {
            max_attempts: DEFAULT_RETRY_ATTEMPTS,
            base_delay_ms: DEFAULT_RETRY_BASE_DELAY_MS,
            max_delay_ms: DEFAULT_RETRY_MAX_BACKOFF_MS,
            retryable_errors: vec![
                ErrorClass::RateLimit,
                ErrorClass::Timeout,
                ErrorClass::ServerError,
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    #[test]
    fn full_jitter_distribution_respects_floor() {
        let policy = RetryPolicy {
            base_delay_ms: 1_000,
            max_delay_ms: 60_000,
            ..RetryPolicy::default()
        };
        let floor = policy.base_delay_ms / 2; // 500
        let capped = policy.base_delay_ms;
        let mut rng = StdRng::seed_from_u64(7);
        let mut unique = std::collections::BTreeSet::new();

        for _ in 0..200 {
            let delay = policy.delay_for_attempt_with_rng(0, &mut rng);
            assert!(delay >= floor, "delay {delay} below floor {floor}");
            assert!(delay <= capped, "delay {delay} above cap {capped}");
            unique.insert(delay);
        }

        assert!(
            unique.len() >= 50,
            "expected spread across the window, got {} unique delays",
            unique.len()
        );
    }

    #[test]
    fn delay_with_retry_after_prefers_provider_hint() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.delay_with_retry_after(2, Some(4_200)), 4_200);
    }

    #[test]
    fn should_retry_matches_provider_error_classification() {
        let policy = RetryPolicy::default();

        assert!(policy.should_retry(&ProviderError::Timeout, 0));
        assert!(policy.should_retry(&ProviderError::ServerError(503), 1));
        assert!(policy.should_retry(
            &ProviderError::RateLimit {
                retry_after_ms: Some(2_000)
            },
            2
        ));
        assert!(!policy.should_retry(&ProviderError::AuthFailure, 0));
        assert!(!policy.should_retry(&ProviderError::ContentPolicy, 0));
        assert!(!policy.should_retry(&ProviderError::ContextOverflow, 0));
        assert!(policy.should_retry(&ProviderError::Other("unknown".into()), 1));
        assert!(!policy.should_retry(&ProviderError::Other("unknown".into()), 2));
    }

    #[test]
    fn for_rate_limit_uses_higher_base_delay() {
        let policy = RetryPolicy::for_rate_limit();
        assert_eq!(policy.base_delay_ms, 2_000);
        assert_eq!(policy.max_attempts, 3);
        // Verify that attempt 0 gives delay in [1000, 2000]
        let mut rng = StdRng::seed_from_u64(42);
        for _ in 0..50 {
            let delay = policy.delay_for_attempt_with_rng(0, &mut rng);
            assert!(delay >= 1_000, "rate limit delay {delay} below floor 1000");
            assert!(delay <= 2_000, "rate limit delay {delay} above cap 2000");
        }
    }

    #[test]
    fn for_rate_limit_exponential_growth() {
        let policy = RetryPolicy::for_rate_limit();
        let mut rng = StdRng::seed_from_u64(99);
        // attempt 1: base * 2 = 4000, floor = 1000 -> [1000, 4000]
        for _ in 0..50 {
            let delay = policy.delay_for_attempt_with_rng(1, &mut rng);
            assert!(delay >= 1_000, "attempt 1 delay {delay} below floor");
            assert!(delay <= 4_000, "attempt 1 delay {delay} above cap");
        }
        // attempt 2: base * 4 = 8000, floor = 1000 -> [1000, 8000]
        for _ in 0..50 {
            let delay = policy.delay_for_attempt_with_rng(2, &mut rng);
            assert!(delay >= 1_000, "attempt 2 delay {delay} below floor");
            assert!(delay <= 8_000, "attempt 2 delay {delay} above cap");
        }
    }

    #[test]
    fn rate_limit_delay_respects_provider_hint() {
        let policy = RetryPolicy::for_rate_limit();
        // Provider says wait 5000ms
        assert_eq!(policy.rate_limit_delay(0, Some(5_000)), 5_000);
        // Provider says wait 500ms but floor is 1000ms
        assert_eq!(policy.rate_limit_delay(0, Some(500)), 1_000);
        // No provider hint -> falls back to jittered exponential
        let delay = policy.rate_limit_delay(0, None);
        assert!(delay >= 1_000);
        assert!(delay <= 2_000);
    }
}
