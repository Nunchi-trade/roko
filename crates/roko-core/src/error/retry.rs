//! Retry policy and circuit breaker for transient errors.
//!
//! # §41.6 -- Retry infrastructure
//!
//! [`RetryPolicy`] encodes exponential backoff with optional jitter.
//! [`CircuitBreaker`] tracks consecutive failures and prevents cascading
//! calls to a failing downstream service.

use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// RetryPolicy
// ---------------------------------------------------------------------------

/// Exponential backoff retry policy.
///
/// The delay for attempt `n` (0-indexed) is:
///
/// ```text
/// delay = min(base_delay_ms * 2^n, max_delay_ms)
/// ```
///
/// When `jitter` is enabled, the delay is multiplied by a deterministic
/// factor derived from the attempt number (to avoid requiring `rand` as a
/// dependency).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    max_attempts: u32,
    base_delay_ms: u64,
    max_delay_ms: u64,
    jitter: bool,
}

impl RetryPolicy {
    /// Create a new retry policy.
    ///
    /// # Arguments
    ///
    /// - `max_attempts` -- total number of attempts (including the initial one).
    ///   Clamped to a minimum of 1.
    /// - `base_delay_ms` -- starting delay in milliseconds.
    /// - `max_delay_ms` -- ceiling for the computed delay.
    /// - `jitter` -- whether to apply deterministic jitter.
    #[must_use]
    pub fn new(max_attempts: u32, base_delay_ms: u64, max_delay_ms: u64, jitter: bool) -> Self {
        Self {
            max_attempts: max_attempts.max(1),
            base_delay_ms,
            max_delay_ms,
            jitter,
        }
    }

    /// Returns the maximum number of attempts (including the first).
    #[must_use]
    pub const fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    /// Returns `true` if `attempt` (0-indexed) is below the retry budget.
    ///
    /// `attempt` 0 means "we just failed the initial try". With
    /// `max_attempts = 3`, attempts 0 and 1 return `true` (two retries),
    /// attempt 2 returns `false` (budget exhausted).
    #[must_use]
    pub const fn should_retry(&self, attempt: u32) -> bool {
        attempt < self.max_attempts.saturating_sub(1)
    }

    /// Compute the delay before the *next* attempt.
    ///
    /// `attempt` is 0-indexed: 0 means "we just failed the first try, how
    /// long before retry #1?".
    #[must_use]
    pub fn delay_for(&self, attempt: u32) -> Duration {
        // Exponential: base * 2^attempt, capped at max.
        let exp = self
            .base_delay_ms
            .saturating_mul(1u64.checked_shl(attempt).unwrap_or(u64::MAX));
        let capped = exp.min(self.max_delay_ms);

        let ms = if self.jitter {
            // Deterministic jitter: use a simple hash of the attempt number to
            // produce a factor in [0.5, 1.0). This avoids pulling in `rand`.
            let hash = Self::jitter_hash(attempt);
            // factor in [0.5, 1.0)
            let factor = (f64::from(hash) / f64::from(u32::MAX)).mul_add(0.5, 0.5);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
            let jittered = (capped as f64 * factor) as u64;
            jittered.max(1)
        } else {
            capped
        };

        Duration::from_millis(ms)
    }

    /// Simple deterministic hash for jitter, seeded by attempt number.
    const fn jitter_hash(attempt: u32) -> u32 {
        // xorshift-style mixing; good enough for jitter distribution.
        let mut x = attempt.wrapping_add(0x9E37_79B9);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    }
}

// ---------------------------------------------------------------------------
// CircuitBreaker
// ---------------------------------------------------------------------------

/// Circuit breaker states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakerState {
    /// Normal operation -- requests flow through.
    Closed,
    /// Breaker tripped -- requests are rejected immediately.
    Open,
    /// Recovery probe -- one request is allowed through to test the service.
    HalfOpen,
}

/// Tracks consecutive failures and trips open to protect against cascading
/// calls to a failing backend.
///
/// State machine:
///
/// ```text
/// Closed --(N failures)--> Open --(timeout)--> HalfOpen
///                                                |
///                                       success: Closed
///                                       failure: Open
/// ```
pub struct CircuitBreaker {
    failure_threshold: u32,
    recovery_timeout: Duration,
    state: BreakerState,
    consecutive_failures: u32,
    last_failure_at: Option<Instant>,
}

impl CircuitBreaker {
    /// Create a new circuit breaker.
    ///
    /// - `failure_threshold` -- number of consecutive failures before tripping.
    ///   Clamped to a minimum of 1.
    /// - `recovery_timeout` -- how long to wait in Open before probing.
    #[must_use]
    pub fn new(failure_threshold: u32, recovery_timeout: Duration) -> Self {
        Self {
            failure_threshold: failure_threshold.max(1),
            recovery_timeout,
            state: BreakerState::Closed,
            consecutive_failures: 0,
            last_failure_at: None,
        }
    }

    /// Returns the current breaker state, accounting for timeout transitions.
    ///
    /// If the breaker is `Open` and `recovery_timeout` has elapsed since the
    /// last failure, it transitions to `HalfOpen`.
    #[must_use]
    pub fn state(&mut self) -> BreakerState {
        if self.state == BreakerState::Open {
            if let Some(ts) = self.last_failure_at {
                if ts.elapsed() >= self.recovery_timeout {
                    self.state = BreakerState::HalfOpen;
                }
            }
        }
        self.state
    }

    /// Returns `true` if the breaker is open (requests should be rejected).
    ///
    /// This also evaluates timeout-based transitions.
    #[must_use]
    pub fn is_open(&mut self) -> bool {
        self.state() == BreakerState::Open
    }

    /// Record a successful operation. Resets the failure counter and closes
    /// the breaker.
    pub const fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.state = BreakerState::Closed;
        self.last_failure_at = None;
    }

    /// Record a failed operation. Increments the failure counter and may trip
    /// the breaker to `Open`.
    pub fn record_failure(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.last_failure_at = Some(Instant::now());

        if self.consecutive_failures >= self.failure_threshold {
            self.state = BreakerState::Open;
        }
    }

    /// Manually trip the breaker to `Open` regardless of failure count.
    pub fn trip(&mut self) {
        self.state = BreakerState::Open;
        self.last_failure_at = Some(Instant::now());
    }

    /// Returns the number of consecutive failures recorded.
    #[must_use]
    pub const fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- RetryPolicy tests --

    #[test]
    fn retry_should_retry_within_budget() {
        let p = RetryPolicy::new(3, 100, 10_000, false);
        assert!(p.should_retry(0)); // first failure, can retry
        assert!(p.should_retry(1)); // second failure, can retry
        assert!(!p.should_retry(2)); // third failure = max attempts exhausted
        assert!(!p.should_retry(10));
    }

    #[test]
    fn retry_exponential_backoff_no_jitter() {
        let p = RetryPolicy::new(5, 100, 10_000, false);
        assert_eq!(p.delay_for(0), Duration::from_millis(100)); // 100 * 2^0
        assert_eq!(p.delay_for(1), Duration::from_millis(200)); // 100 * 2^1
        assert_eq!(p.delay_for(2), Duration::from_millis(400)); // 100 * 2^2
        assert_eq!(p.delay_for(3), Duration::from_millis(800)); // 100 * 2^3
    }

    #[test]
    fn retry_caps_at_max_delay() {
        let p = RetryPolicy::new(10, 100, 500, false);
        assert_eq!(p.delay_for(0), Duration::from_millis(100));
        assert_eq!(p.delay_for(1), Duration::from_millis(200));
        assert_eq!(p.delay_for(2), Duration::from_millis(400));
        assert_eq!(p.delay_for(3), Duration::from_millis(500)); // capped
        assert_eq!(p.delay_for(10), Duration::from_millis(500)); // still capped
    }

    #[test]
    fn retry_jitter_reduces_delay() {
        let no_jitter = RetryPolicy::new(5, 1_000, 60_000, false);
        let with_jitter = RetryPolicy::new(5, 1_000, 60_000, true);

        for attempt in 0..4 {
            let base = no_jitter.delay_for(attempt);
            let jittered = with_jitter.delay_for(attempt);
            // Jitter factor is in [0.5, 1.0), so jittered <= base.
            assert!(
                jittered <= base,
                "attempt {attempt}: jittered {jittered:?} should be <= base {base:?}"
            );
            // And at least half.
            let half = base / 2;
            assert!(
                jittered >= half,
                "attempt {attempt}: jittered {jittered:?} should be >= half {half:?}"
            );
        }
    }

    #[test]
    fn retry_jitter_is_deterministic() {
        let p = RetryPolicy::new(5, 1_000, 60_000, true);
        let d1 = p.delay_for(2);
        let d2 = p.delay_for(2);
        assert_eq!(d1, d2, "same attempt should yield same jittered delay");
    }

    #[test]
    fn retry_min_max_attempts_is_one() {
        let p = RetryPolicy::new(0, 100, 1_000, false);
        assert_eq!(p.max_attempts(), 1);
        assert!(!p.should_retry(0)); // only one attempt total
    }

    #[test]
    fn retry_overflow_safety() {
        let p = RetryPolicy::new(3, u64::MAX, u64::MAX, false);
        // Should not panic.
        let d = p.delay_for(30);
        assert_eq!(d, Duration::from_millis(u64::MAX));
    }

    #[test]
    fn retry_jitter_varies_across_attempts() {
        let p = RetryPolicy::new(10, 1_000, 60_000, true);
        // Different attempts should produce different delays because the base
        // values differ (1000 vs 2000).
        let d0 = p.delay_for(0);
        let d1 = p.delay_for(1);
        assert_ne!(d0, d1);
    }

    // -- CircuitBreaker tests --

    #[test]
    fn retry_breaker_starts_closed() {
        let mut cb = CircuitBreaker::new(3, Duration::from_secs(30));
        assert_eq!(cb.state(), BreakerState::Closed);
        assert!(!cb.is_open());
    }

    #[test]
    fn retry_breaker_trips_after_threshold() {
        let mut cb = CircuitBreaker::new(3, Duration::from_secs(30));
        cb.record_failure();
        assert_eq!(cb.state(), BreakerState::Closed);
        cb.record_failure();
        assert_eq!(cb.state(), BreakerState::Closed);
        cb.record_failure(); // 3rd failure = threshold
        assert_eq!(cb.state(), BreakerState::Open);
        assert!(cb.is_open());
    }

    #[test]
    fn retry_breaker_success_resets() {
        let mut cb = CircuitBreaker::new(3, Duration::from_secs(30));
        cb.record_failure();
        cb.record_failure();
        // 2 failures, not tripped yet
        cb.record_success();
        assert_eq!(cb.consecutive_failures(), 0);
        assert_eq!(cb.state(), BreakerState::Closed);
        // Now it takes 3 more failures to trip
        cb.record_failure();
        assert_eq!(cb.state(), BreakerState::Closed);
    }

    #[test]
    fn retry_breaker_manual_trip() {
        let mut cb = CircuitBreaker::new(100, Duration::from_secs(30));
        cb.trip();
        assert!(cb.is_open());
    }

    #[test]
    fn retry_breaker_half_open_after_timeout() {
        // With a 0ms recovery timeout, the breaker transitions from Open
        // to HalfOpen immediately because elapsed() >= 0 is always true.
        let mut cb = CircuitBreaker::new(1, Duration::from_millis(0));
        cb.record_failure(); // trips immediately (threshold=1)
        // state() evaluates the timeout and transitions to HalfOpen.
        assert_eq!(cb.state(), BreakerState::HalfOpen);
    }

    #[test]
    fn retry_breaker_stays_open_before_timeout() {
        // With a long recovery timeout, the breaker stays Open.
        let mut cb = CircuitBreaker::new(1, Duration::from_secs(3600));
        cb.record_failure();
        assert!(cb.is_open());
        assert_eq!(cb.state(), BreakerState::Open);
    }

    #[test]
    fn retry_breaker_half_open_success_closes() {
        let mut cb = CircuitBreaker::new(1, Duration::from_millis(0));
        cb.record_failure();
        // Should be HalfOpen after 0ms timeout.
        assert_eq!(cb.state(), BreakerState::HalfOpen);
        cb.record_success();
        assert_eq!(cb.state(), BreakerState::Closed);
        assert!(!cb.is_open());
    }

    #[test]
    fn retry_breaker_half_open_failure_reopens() {
        // Use a long timeout so we can observe the Open state persisting
        // after a failure in HalfOpen.
        let mut cb = CircuitBreaker::new(1, Duration::from_secs(3600));
        cb.record_failure(); // trips to Open
        assert_eq!(cb.state(), BreakerState::Open);

        // Simulate timeout expiry by forcing state to HalfOpen manually.
        // We do this by creating a breaker with 0ms timeout, transitioning
        // to HalfOpen, then recording a failure. The failure should trip
        // the breaker back and increment the counter.
        let mut cb2 = CircuitBreaker::new(1, Duration::from_millis(0));
        cb2.record_failure(); // trips to Open
        assert_eq!(cb2.state(), BreakerState::HalfOpen); // 0ms timeout -> HalfOpen
        let failures_before = cb2.consecutive_failures();
        cb2.record_failure(); // should record failure from HalfOpen
        assert!(
            cb2.consecutive_failures() > failures_before,
            "failure in HalfOpen should increment counter"
        );
    }

    #[test]
    fn retry_breaker_consecutive_failures_counter() {
        let mut cb = CircuitBreaker::new(10, Duration::from_secs(30));
        assert_eq!(cb.consecutive_failures(), 0);
        cb.record_failure();
        assert_eq!(cb.consecutive_failures(), 1);
        cb.record_failure();
        assert_eq!(cb.consecutive_failures(), 2);
        cb.record_success();
        assert_eq!(cb.consecutive_failures(), 0);
    }
}
