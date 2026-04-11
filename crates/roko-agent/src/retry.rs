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
    /// Returns a full-jitter delay for the given attempt number.
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> u64 {
        let mut rng = rand::thread_rng();
        self.delay_for_attempt_with_rng(attempt, &mut rng)
    }

    fn delay_for_attempt_with_rng<R: Rng + ?Sized>(&self, attempt: u32, rng: &mut R) -> u64 {
        let exp_delay = self.base_delay_ms.saturating_mul(1u64 << attempt.min(10));
        let capped = exp_delay.min(self.max_delay_ms);
        rng.gen_range(0..=capped)
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
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay_ms: 1_000,
            max_delay_ms: 60_000,
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
    fn full_jitter_distribution() {
        let policy = RetryPolicy {
            base_delay_ms: 1_000,
            max_delay_ms: 60_000,
            ..RetryPolicy::default()
        };
        let capped = policy.base_delay_ms;
        let bucket_width = (capped + 1) / 10;
        let mut buckets = [0usize; 10];
        let mut rng = StdRng::seed_from_u64(7);
        let mut unique = std::collections::BTreeSet::new();

        for _ in 0..100 {
            let delay = policy.delay_for_attempt_with_rng(0, &mut rng);
            assert!(delay <= capped);
            unique.insert(delay);

            let bucket = (delay / bucket_width).min(9);
            buckets[bucket as usize] += 1;
        }

        assert!(
            unique.len() >= 80,
            "expected spread across the window, got {} unique delays",
            unique.len()
        );
        assert!(
            buckets.iter().all(|count| *count > 0),
            "expected every bucket to receive samples, got {:?}",
            buckets
        );
        let max_bucket = buckets.iter().copied().max().unwrap_or_default();
        let min_bucket = buckets.iter().copied().min().unwrap_or_default();
        assert!(
            max_bucket - min_bucket <= 12,
            "expected roughly uniform distribution, got {:?}",
            buckets
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
}
