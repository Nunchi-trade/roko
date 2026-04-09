//! Per-tool / per-role rate limiting (§36.51).
//!
//! Sliding-window counter keyed by `(role, tool_name)`: a call is
//! admitted iff fewer than `max_calls_per_window` calls have been
//! recorded for this key within the last `window_duration`.
//!
//! Internally uses `parking_lot::Mutex<HashMap<Key, VecDeque<Instant>>>`
//! so the critical section is minimal and never async. Timestamps are
//! ordered oldest-first in the deque; stale entries are pruned from the
//! front on every operation, keeping memory bounded to `cap` entries per
//! key in steady state.
//!
//! # Thread safety
//!
//! `RateLimiter` is `Send + Sync`. `check_and_record` is a single
//! `Mutex` lock-and-release — no possibility of TOCTOU between the cap
//! check and the push.
//!
//! # Example
//!
//! ```ignore
//! use roko_agent::safety::rate_limit::{RateLimiter, RateLimitKey, RateLimitPolicy};
//! use std::time::Duration;
//!
//! let limiter = RateLimiter::new(RateLimitPolicy {
//!     max_calls_per_window: 10,
//!     window_duration: Duration::from_secs(1),
//! });
//! let key = RateLimitKey { role: "implementer".into(), tool: "bash".into() };
//! limiter.check_and_record(&key).expect("first call admitted");
//! ```

use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use roko_core::tool::ToolError;

// ─── Public types ─────────────────────────────────────────────────────────

/// Key under which rate-limit counters are tracked.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RateLimitKey {
    /// Role making the call (e.g. `"Implementer"`, `"Auditor"`).
    pub role: String,
    /// Canonical tool name.
    pub tool: String,
}

/// Rate-limit policy: sliding window + cap.
#[derive(Debug, Clone)]
pub struct RateLimitPolicy {
    /// Maximum calls allowed per key per window. Default: 60.
    pub max_calls_per_window: usize,
    /// Length of the sliding window. Default: 60 s.
    pub window_duration: Duration,
}

impl Default for RateLimitPolicy {
    fn default() -> Self {
        Self {
            max_calls_per_window: 60,
            window_duration: Duration::from_secs(60),
        }
    }
}

// ─── RateLimiter ──────────────────────────────────────────────────────────

/// Thread-safe sliding-window rate limiter keyed by `(role, tool)`.
///
/// The internal state is a `Mutex<HashMap<RateLimitKey, VecDeque<Instant>>>`.
/// Each deque holds the admission timestamps for one key, oldest first.
/// Stale entries (older than `now - window_duration`) are pruned before
/// every admission check, so the deque length is always bounded by
/// `max_calls_per_window` in steady state.
#[derive(Debug)]
pub struct RateLimiter {
    policy: RateLimitPolicy,
    state: Mutex<HashMap<RateLimitKey, VecDeque<Instant>>>,
}

impl RateLimiter {
    /// Construct a limiter with the given policy.
    #[must_use]
    pub fn new(policy: RateLimitPolicy) -> Self {
        Self {
            policy,
            state: Mutex::new(HashMap::new()),
        }
    }

    /// Construct a limiter with the default policy (60 calls / 60 s).
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(RateLimitPolicy::default())
    }

    /// Check whether a call for `key` would be admitted **without**
    /// recording a timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError::Other`] if the cap has been reached.
    #[allow(clippy::significant_drop_tightening)]
    pub fn check(&self, key: &RateLimitKey) -> Result<(), ToolError> {
        let now = Instant::now();
        let window_start = now.checked_sub(self.policy.window_duration).unwrap_or(now);
        let (len, cap) = {
            let mut guard = self.state.lock();
            let deque = guard.entry(key.clone()).or_default();
            prune(deque, window_start);
            (deque.len(), self.policy.max_calls_per_window)
        };
        if len >= cap {
            return Err(self.rate_limit_error(key, len));
        }
        Ok(())
    }

    /// Record a timestamp for `key` **without** checking the cap.
    ///
    /// Prunes stale entries before pushing so the deque stays tidy.
    #[allow(clippy::significant_drop_tightening)]
    pub fn record(&self, key: &RateLimitKey) {
        let now = Instant::now();
        let window_start = now.checked_sub(self.policy.window_duration).unwrap_or(now);
        let mut guard = self.state.lock();
        let deque = guard.entry(key.clone()).or_default();
        prune(deque, window_start);
        deque.push_back(now);
    }

    /// Atomically check the cap and, if admitted, record the timestamp.
    ///
    /// Both the check and the push happen under a single mutex lock, so
    /// there is no TOCTOU gap between them.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError::Other`] if the cap has been reached.
    #[allow(clippy::significant_drop_tightening)]
    pub fn check_and_record(&self, key: &RateLimitKey) -> Result<(), ToolError> {
        let now = Instant::now();
        let window_start = now.checked_sub(self.policy.window_duration).unwrap_or(now);
        let (len, cap) = {
            let mut guard = self.state.lock();
            let deque = guard.entry(key.clone()).or_default();
            prune(deque, window_start);
            let len = deque.len();
            let cap = self.policy.max_calls_per_window;
            if len < cap {
                deque.push_back(now);
            }
            (len, cap)
        };
        if len >= cap {
            return Err(self.rate_limit_error(key, len));
        }
        Ok(())
    }

    /// Return the number of live timestamps in the window for `key`.
    ///
    /// Prunes stale entries before counting so the result is current.
    /// Only available in test builds.
    #[cfg(test)]
    #[allow(clippy::significant_drop_tightening)]
    pub fn window_size(&self, key: &RateLimitKey) -> usize {
        let now = Instant::now();
        let window_start = now.checked_sub(self.policy.window_duration).unwrap_or(now);
        let mut guard = self.state.lock();
        let deque = guard.entry(key.clone()).or_default();
        prune(deque, window_start);
        deque.len()
    }

    // ─── helpers ──────────────────────────────────────────────────────

    fn rate_limit_error(&self, key: &RateLimitKey, current: usize) -> ToolError {
        let cap = self.policy.max_calls_per_window;
        let window_ms = self.policy.window_duration.as_millis();
        ToolError::Other(format!(
            "rate limit exceeded for role={} tool={}: {}/{} in {}ms window",
            key.role, key.tool, current, cap, window_ms,
        ))
    }
}

// ─── Internal helpers ─────────────────────────────────────────────────────

/// Remove all entries from the front of `deque` that are `<= window_start`
/// (i.e. outside the sliding window). Entries are stored oldest-first.
fn prune(deque: &mut VecDeque<Instant>, window_start: Instant) {
    while deque.front().is_some_and(|t| *t <= window_start) {
        deque.pop_front();
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
        time::Duration,
    };

    use super::*;

    fn key(role: &str, tool: &str) -> RateLimitKey {
        RateLimitKey {
            role: role.into(),
            tool: tool.into(),
        }
    }

    // 1 — admit under cap
    #[test]
    fn fresh_limiter_admits_under_cap() {
        let limiter = RateLimiter::new(RateLimitPolicy {
            max_calls_per_window: 5,
            window_duration: Duration::from_secs(60),
        });
        let k = key("role", "tool");
        for _ in 0..4 {
            limiter
                .check_and_record(&k)
                .expect("should be admitted under cap");
        }
    }

    // 2 — admit exactly cap, reject cap+1
    #[test]
    fn cap_boundary_admits_exactly_cap() {
        let limiter = RateLimiter::new(RateLimitPolicy {
            max_calls_per_window: 5,
            window_duration: Duration::from_secs(60),
        });
        let k = key("role", "tool");
        for i in 0..5 {
            limiter
                .check_and_record(&k)
                .unwrap_or_else(|e| panic!("call {i} should succeed: {e}"));
        }
        let err = limiter
            .check_and_record(&k)
            .expect_err("6th call should be rejected");
        assert!(
            matches!(err, ToolError::Other(_)),
            "expected Other, got {err:?}"
        );
    }

    // 3 — window expiry re-admits
    #[test]
    fn window_expiry_admits_after_delay() {
        let limiter = RateLimiter::new(RateLimitPolicy {
            max_calls_per_window: 2,
            window_duration: Duration::from_millis(50),
        });
        let k = key("role", "tool");
        limiter.check_and_record(&k).expect("1st ok");
        limiter.check_and_record(&k).expect("2nd ok");
        // At cap — should fail now.
        limiter
            .check_and_record(&k)
            .expect_err("3rd should fail before expiry");
        // Wait for window to expire.
        thread::sleep(Duration::from_millis(100));
        limiter
            .check_and_record(&k)
            .expect("3rd ok after window expired");
    }

    // 4 — different keys are independent
    #[test]
    fn different_keys_are_independent() {
        let limiter = RateLimiter::new(RateLimitPolicy {
            max_calls_per_window: 2,
            window_duration: Duration::from_secs(60),
        });
        let ka = key("A", "tool_x");
        let kb = key("B", "tool_x");
        limiter.check_and_record(&ka).expect("A:1 ok");
        limiter.check_and_record(&ka).expect("A:2 ok");
        // A is at cap.
        limiter.check_and_record(&ka).expect_err("A:3 should fail");
        // B is independent — should still admit.
        limiter
            .check_and_record(&kb)
            .expect("B:1 ok regardless of A");
    }

    // 5 — check without record doesn't increment
    #[test]
    fn check_without_record_does_not_increment() {
        let limiter = RateLimiter::new(RateLimitPolicy {
            max_calls_per_window: 3,
            window_duration: Duration::from_secs(60),
        });
        let k = key("role", "tool");
        // call check() cap times — none should record.
        for i in 0..3 {
            limiter
                .check(&k)
                .unwrap_or_else(|e| panic!("check {i} failed: {e}"));
        }
        // A real check_and_record should still succeed (window is empty).
        limiter
            .check_and_record(&k)
            .expect("should admit: check() never incremented");
    }

    // 6 — record without check does increment
    #[test]
    fn record_without_check_increments() {
        let limiter = RateLimiter::new(RateLimitPolicy {
            max_calls_per_window: 2,
            window_duration: Duration::from_secs(60),
        });
        let k = key("role", "tool");
        limiter.record(&k);
        limiter.record(&k);
        // Window is now full — check should fail.
        limiter.check(&k).expect_err("cap reached via record()");
    }

    // 7 — error message contains role and tool
    #[test]
    fn error_message_contains_role_and_tool() {
        let limiter = RateLimiter::new(RateLimitPolicy {
            max_calls_per_window: 1,
            window_duration: Duration::from_secs(60),
        });
        let k = key("implementer", "bash");
        limiter.check_and_record(&k).expect("first ok");
        let err = limiter.check_and_record(&k).expect_err("second rejected");
        let msg = match err {
            ToolError::Other(m) => m,
            other => panic!("expected Other, got {other:?}"),
        };
        assert!(msg.contains("implementer"), "role missing from: {msg}");
        assert!(msg.contains("bash"), "tool missing from: {msg}");
    }

    // 8 — defaults match documented values
    #[test]
    fn defaults_match_documented_values() {
        let policy = RateLimitPolicy::default();
        assert_eq!(policy.max_calls_per_window, 60);
        assert_eq!(policy.window_duration, Duration::from_secs(60));
    }

    // 9 — window_size helper reflects pushes
    #[test]
    fn window_size_helper_reflects_pushes() {
        let limiter = RateLimiter::new(RateLimitPolicy {
            max_calls_per_window: 10,
            window_duration: Duration::from_secs(60),
        });
        let k = key("r", "t");
        assert_eq!(limiter.window_size(&k), 0, "fresh limiter has 0 in window");
        limiter.record(&k);
        limiter.record(&k);
        limiter.record(&k);
        assert_eq!(limiter.window_size(&k), 3, "after 3 records");
    }

    // 10 — window_size reflects expiries
    #[test]
    fn window_size_reflects_expiries() {
        let limiter = RateLimiter::new(RateLimitPolicy {
            max_calls_per_window: 10,
            window_duration: Duration::from_millis(50),
        });
        let k = key("r", "t");
        limiter.record(&k);
        limiter.record(&k);
        assert_eq!(limiter.window_size(&k), 2, "before expiry");
        thread::sleep(Duration::from_millis(100));
        assert_eq!(limiter.window_size(&k), 0, "after expiry all entries gone");
    }

    // 11 — new vs with_defaults are equivalent
    #[test]
    fn new_vs_with_defaults_equivalent() {
        let a = RateLimiter::new(RateLimitPolicy::default());
        let b = RateLimiter::with_defaults();
        assert_eq!(
            a.policy.max_calls_per_window, b.policy.max_calls_per_window,
            "cap must match"
        );
        assert_eq!(
            a.policy.window_duration, b.policy.window_duration,
            "window must match"
        );
    }

    // 12 — concurrent threads stay under cap (the stress test)
    #[test]
    fn concurrent_threads_stay_under_cap() {
        const THREADS: usize = 20;
        const CALLS_PER_THREAD: usize = 10;
        const CAP: usize = 50;

        let limiter = Arc::new(RateLimiter::new(RateLimitPolicy {
            max_calls_per_window: CAP,
            window_duration: Duration::from_secs(3600), // effectively infinite
        }));
        let successes = Arc::new(AtomicUsize::new(0));
        let failures = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::with_capacity(THREADS);
        for _ in 0..THREADS {
            let lim = Arc::clone(&limiter);
            let suc = Arc::clone(&successes);
            let fail = Arc::clone(&failures);
            handles.push(thread::spawn(move || {
                let k = key("stress-role", "stress-tool");
                for _ in 0..CALLS_PER_THREAD {
                    match lim.check_and_record(&k) {
                        Ok(()) => {
                            suc.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(_) => {
                            fail.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }));
        }
        for h in handles {
            h.join().expect("thread should not panic");
        }

        let total_calls = THREADS * CALLS_PER_THREAD; // 200
        let got_suc = successes.load(Ordering::Relaxed);
        let got_fail = failures.load(Ordering::Relaxed);

        assert_eq!(
            got_suc + got_fail,
            total_calls,
            "all calls must be tallied: {got_suc} + {got_fail} != {total_calls}"
        );
        assert_eq!(
            got_suc, CAP,
            "exactly cap={CAP} must succeed, got {got_suc}"
        );
        assert_eq!(
            got_fail,
            total_calls - CAP,
            "exactly {} must fail, got {got_fail}",
            total_calls - CAP
        );
    }

    // 13 — concurrent different keys are independent
    #[test]
    fn concurrent_different_keys_independent() {
        const THREADS_PER_KEY: usize = 4; // but 1 thread per key in practice
        const KEYS: usize = 4;
        const CALLS_PER_THREAD: usize = 10;
        const CAP: usize = 5;
        // Each key allows 5 calls; we run 4 threads each using a unique key,
        // doing 10 calls each. Exactly 5 per key (= 20 total) must succeed.

        let limiter = Arc::new(RateLimiter::new(RateLimitPolicy {
            max_calls_per_window: CAP,
            window_duration: Duration::from_secs(3600),
        }));
        let successes = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::with_capacity(THREADS_PER_KEY * KEYS);
        for key_idx in 0..KEYS {
            // Spawn 1 thread per key so each key is only accessed by 1 thread
            // (keys are independent; the interesting part is that they don't
            // interfere with each other's counters).
            let lim = Arc::clone(&limiter);
            let suc = Arc::clone(&successes);
            handles.push(thread::spawn(move || {
                let k = key(&format!("role-{key_idx}"), &format!("tool-{key_idx}"));
                for _ in 0..CALLS_PER_THREAD {
                    if lim.check_and_record(&k).is_ok() {
                        suc.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }));
        }
        for h in handles {
            h.join().expect("thread should not panic");
        }

        let got_suc = successes.load(Ordering::Relaxed);
        assert_eq!(
            got_suc,
            KEYS * CAP,
            "each of the {KEYS} keys should admit exactly {CAP}: expected {}, got {got_suc}",
            KEYS * CAP,
        );
    }
}
