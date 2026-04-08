//! Runaway-loop detection for the Roko orchestrator (parity §28.6).
//!
//! [`LoopGuard`] tracks a rolling window of action *fingerprints* and issues
//! verdicts that the conductor can use to short-circuit stuck agents before
//! they burn budget. Three failure modes are detected:
//!
//! * **`MaxIters`** — the cumulative number of actions recorded has exceeded
//!   [`LoopGuardConfig::max_iterations`].
//! * **`RepeatLoop`** — every fingerprint currently inside the rolling window
//!   is identical (the agent keeps doing the exact same thing).
//! * **`Stall`** — the window has drifted past the progress budget without a
//!   *new* fingerprint ever being observed (no forward progress).
//!
//! All other ticks return [`LoopVerdict::Continue`].
//!
//! # Concurrency
//!
//! The public surface is `&self`-only; all mutation lives behind a
//! [`parking_lot::Mutex`]. Clones of a [`LoopGuard`] share state via an
//! [`Arc`], which is convenient when the guard is handed to multiple agent
//! tasks.

use std::collections::VecDeque;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use chrono::Utc;
use parking_lot::Mutex;

/// Configuration for a [`LoopGuard`].
#[derive(Debug, Clone, Copy)]
pub struct LoopGuardConfig {
    /// Hard ceiling on total recorded actions. `MaxIters` fires once the
    /// `(max_iterations + 1)`-th action is recorded.
    pub max_iterations: u32,
    /// Number of recent fingerprints kept in the rolling window. Must be at
    /// least 2 to detect repetition meaningfully; the constructor clamps it.
    pub window_size: usize,
    /// Maximum duration (milliseconds) the window may span without a new
    /// fingerprint before a `Stall` is declared.
    pub progress_budget_ms: u64,
}

impl Default for LoopGuardConfig {
    fn default() -> Self {
        Self {
            max_iterations: 100,
            window_size: 8,
            progress_budget_ms: 60_000,
        }
    }
}

/// The verdict emitted by [`LoopGuard::record`] / [`LoopGuard::peek`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopVerdict {
    /// No anomaly detected; the caller may proceed.
    Continue,
    /// The rolling window has drifted past the progress budget without ever
    /// observing a new fingerprint.
    Stall,
    /// Every fingerprint in the current window is identical.
    RepeatLoop,
    /// The configured `max_iterations` has been exceeded.
    MaxIters,
}

/// Internal, mutable state guarded by the mutex.
#[derive(Debug)]
struct Inner {
    config: LoopGuardConfig,
    /// Rolling window of recent `(fingerprint_hash, ts_ms)` pairs.
    /// `VecDeque` gives O(1) `pop_front` vs `Vec::remove(0)`'s O(n).
    window: VecDeque<(u64, i64)>,
    /// Total count of actions observed (never decremented outside `reset`).
    total: u32,
}

impl Inner {
    fn new(config: LoopGuardConfig) -> Self {
        Self {
            config,
            window: VecDeque::with_capacity(config.window_size.max(2)),
            total: 0,
        }
    }

    fn push(&mut self, fingerprint: u64, ts_ms: i64) {
        let cap = self.config.window_size.max(2);
        if self.window.len() == cap {
            self.window.pop_front();
        }
        self.window.push_back((fingerprint, ts_ms));
        self.total = self.total.saturating_add(1);
    }

    fn evaluate(&self) -> LoopVerdict {
        if self.total > self.config.max_iterations {
            return LoopVerdict::MaxIters;
        }

        let cap = self.config.window_size.max(2);
        if self.window.len() >= cap {
            // Repeat-loop: all fingerprints in a full window are identical.
            if let Some(&(first, _)) = self.window.front() {
                if self.window.iter().all(|&(fp, _)| fp == first) {
                    return LoopVerdict::RepeatLoop;
                }
            }

            // Stall: the window spans past the progress budget without any
            // new fingerprint. "All identical" is strictly stronger and was
            // already caught above, so here we mean "the window is full yet
            // still made no forward progress in the allowed time".
            if let (Some(&(_, oldest_ts)), Some(&(_, newest_ts))) =
                (self.window.front(), self.window.back())
            {
                let span = newest_ts.saturating_sub(oldest_ts);
                if span >= 0 {
                    let span_u64 = u64::try_from(span).unwrap_or(u64::MAX);
                    let unique_fps = {
                        let mut fps: Vec<u64> = self.window.iter().map(|&(fp, _)| fp).collect();
                        fps.sort_unstable();
                        fps.dedup();
                        fps.len()
                    };
                    if unique_fps < cap && span_u64 >= self.config.progress_budget_ms {
                        return LoopVerdict::Stall;
                    }
                }
            }
        }

        LoopVerdict::Continue
    }
}

/// Runaway-loop detector shared across orchestrator tasks.
#[derive(Debug, Clone)]
pub struct LoopGuard {
    inner: Arc<Mutex<Inner>>,
}

impl LoopGuard {
    /// Construct a guard with the given configuration.
    pub fn new(config: LoopGuardConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::new(config))),
        }
    }

    /// Record an action by hashable descriptor and return the resulting
    /// verdict. The timestamp is taken from [`chrono::Utc`].
    pub fn record<A: Hash + ?Sized>(&self, action: &A) -> LoopVerdict {
        let ts_ms = Utc::now().timestamp_millis();
        self.record_at(action, ts_ms)
    }

    /// Record an action with a caller-supplied timestamp (milliseconds since
    /// the Unix epoch). Primarily useful for deterministic tests.
    pub fn record_at<A: Hash + ?Sized>(&self, action: &A, ts_ms: i64) -> LoopVerdict {
        let fingerprint = fingerprint_of(action);
        let mut inner = self.inner.lock();
        inner.push(fingerprint, ts_ms);
        inner.evaluate()
    }

    /// Inspect the current verdict without mutating state.
    pub fn peek(&self) -> LoopVerdict {
        self.inner.lock().evaluate()
    }

    /// Return the number of actions recorded since construction or the most
    /// recent [`reset`](Self::reset).
    pub fn total(&self) -> u32 {
        self.inner.lock().total
    }

    /// Number of fingerprints currently held in the rolling window.
    pub fn window_len(&self) -> usize {
        self.inner.lock().window.len()
    }

    /// Clear all tracked state. Useful after the conductor resolves a stuck
    /// phase and wants to give the agent a clean slate.
    pub fn reset(&self) {
        let mut inner = self.inner.lock();
        inner.window.clear();
        inner.total = 0;
    }
}

/// Produce a stable hash of any `Hash` descriptor. Centralised here so
/// callers do not need to build hashers themselves.
fn fingerprint_of<A: Hash + ?Sized>(action: &A) -> u64 {
    let mut hasher = DefaultHasher::new();
    action.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::{LoopGuard, LoopGuardConfig, LoopVerdict};

    fn cfg(max_iters: u32, window: usize, budget_ms: u64) -> LoopGuardConfig {
        LoopGuardConfig {
            max_iterations: max_iters,
            window_size: window,
            progress_budget_ms: budget_ms,
        }
    }

    #[test]
    fn distinct_actions_continue() {
        let guard = LoopGuard::new(cfg(100, 4, 10_000));
        let actions = ["a", "b", "c", "d", "e"];
        for (i, action) in actions.iter().enumerate() {
            let ts = i64::try_from(i).unwrap_or(0) * 100;
            let v = guard.record_at(action, ts);
            assert_eq!(v, LoopVerdict::Continue, "action #{i}");
        }
        assert_eq!(guard.total(), 5);
    }

    #[test]
    fn repeated_actions_trigger_repeat_loop() {
        let guard = LoopGuard::new(cfg(100, 3, 10_000));
        assert_eq!(guard.record_at("x", 0), LoopVerdict::Continue);
        assert_eq!(guard.record_at("x", 10), LoopVerdict::Continue);
        // Third identical fill fills the window with one fingerprint.
        assert_eq!(guard.record_at("x", 20), LoopVerdict::RepeatLoop);
    }

    #[test]
    fn repeat_loop_requires_full_window() {
        let guard = LoopGuard::new(cfg(100, 4, 10_000));
        assert_eq!(guard.record_at("x", 0), LoopVerdict::Continue);
        assert_eq!(guard.record_at("x", 1), LoopVerdict::Continue);
        assert_eq!(guard.record_at("x", 2), LoopVerdict::Continue);
        // Only on the fourth identical action does the window go full.
        assert_eq!(guard.record_at("x", 3), LoopVerdict::RepeatLoop);
    }

    #[test]
    fn max_iterations_overrides_other_verdicts() {
        let guard = LoopGuard::new(cfg(3, 4, 10_000));
        assert_eq!(guard.record_at("a", 0), LoopVerdict::Continue);
        assert_eq!(guard.record_at("b", 1), LoopVerdict::Continue);
        assert_eq!(guard.record_at("c", 2), LoopVerdict::Continue);
        // 4th record pushes total to 4 > max_iterations=3.
        assert_eq!(guard.record_at("d", 3), LoopVerdict::MaxIters);
    }

    #[test]
    fn progress_budget_exhausted_triggers_stall() {
        let guard = LoopGuard::new(cfg(100, 3, 500));
        // Two unique fps alternating so no repeat-loop, but window span
        // exceeds 500ms → Stall.
        assert_eq!(guard.record_at("a", 0), LoopVerdict::Continue);
        assert_eq!(guard.record_at("b", 300), LoopVerdict::Continue);
        assert_eq!(guard.record_at("a", 700), LoopVerdict::Stall);
    }

    #[test]
    fn fresh_progress_within_budget_continues() {
        let guard = LoopGuard::new(cfg(100, 3, 10_000));
        // Window fills with alternating fps, but span well below budget.
        assert_eq!(guard.record_at("a", 0), LoopVerdict::Continue);
        assert_eq!(guard.record_at("b", 10), LoopVerdict::Continue);
        assert_eq!(guard.record_at("a", 20), LoopVerdict::Continue);
        assert_eq!(guard.record_at("c", 30), LoopVerdict::Continue);
    }

    #[test]
    fn reset_clears_state() {
        let guard = LoopGuard::new(cfg(100, 3, 10_000));
        for i in 0..3 {
            let _ = guard.record_at("x", i);
        }
        assert_eq!(guard.peek(), LoopVerdict::RepeatLoop);
        guard.reset();
        assert_eq!(guard.total(), 0);
        assert_eq!(guard.window_len(), 0);
        assert_eq!(guard.peek(), LoopVerdict::Continue);
        // Post-reset the guard behaves like new.
        assert_eq!(guard.record_at("y", 100), LoopVerdict::Continue);
    }

    #[test]
    fn window_slides_and_old_entries_drop() {
        let guard = LoopGuard::new(cfg(100, 3, 10_000));
        let _ = guard.record_at("a", 0);
        let _ = guard.record_at("a", 1);
        let _ = guard.record_at("a", 2);
        assert_eq!(guard.peek(), LoopVerdict::RepeatLoop);
        // Introduce different fps; window slides, repeat loop clears.
        assert_eq!(guard.record_at("b", 3), LoopVerdict::Continue);
        assert_eq!(guard.record_at("c", 4), LoopVerdict::Continue);
        // Window now holds ["a", "b", "c"] — varied.
        assert_eq!(guard.peek(), LoopVerdict::Continue);
        assert_eq!(guard.window_len(), 3);
    }

    #[test]
    fn peek_does_not_mutate() {
        let guard = LoopGuard::new(cfg(100, 3, 10_000));
        let _ = guard.record_at("x", 0);
        let _ = guard.record_at("x", 1);
        let before = guard.total();
        let v1 = guard.peek();
        let v2 = guard.peek();
        assert_eq!(v1, v2);
        assert_eq!(guard.total(), before);
    }

    #[derive(Hash)]
    struct Action {
        kind: &'static str,
        arg: u32,
    }

    #[test]
    fn same_fingerprint_for_equal_actions() {
        let guard = LoopGuard::new(cfg(100, 2, 10_000));
        let a = Action {
            kind: "foo",
            arg: 1,
        };
        let b = Action {
            kind: "foo",
            arg: 1,
        };
        assert_eq!(guard.record_at(&a, 0), LoopVerdict::Continue);
        assert_eq!(guard.record_at(&b, 1), LoopVerdict::RepeatLoop);
    }

    #[test]
    fn shared_clone_shares_state() {
        let guard = LoopGuard::new(cfg(100, 2, 10_000));
        let clone = guard.clone();
        assert_eq!(guard.record_at("x", 0), LoopVerdict::Continue);
        assert_eq!(clone.record_at("x", 1), LoopVerdict::RepeatLoop);
        assert_eq!(guard.total(), 2);
        assert_eq!(clone.total(), 2);
    }

    #[test]
    fn default_config_values() {
        let c = LoopGuardConfig::default();
        assert!(c.max_iterations > 0);
        assert!(c.window_size >= 2);
        assert!(c.progress_budget_ms > 0);
    }

    #[test]
    fn tiny_window_clamped_to_two() {
        // window_size=1 gets clamped internally to 2 so repeat detection
        // remains meaningful.
        let guard = LoopGuard::new(cfg(100, 1, 10_000));
        assert_eq!(guard.record_at("x", 0), LoopVerdict::Continue);
        assert_eq!(guard.record_at("x", 1), LoopVerdict::RepeatLoop);
    }
}
