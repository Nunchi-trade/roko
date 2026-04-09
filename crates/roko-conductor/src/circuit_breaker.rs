//! Circuit breaker: per-plan failure budget tracking.
//!
//! After [`MAX_PLAN_FAILURES`] failures on a single plan, the circuit
//! breaker trips and the plan is aborted. Uses [`DashMap`] for lock-free
//! concurrent access from multiple watcher threads.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

/// Maximum number of failures allowed per plan before the circuit breaks.
pub const MAX_PLAN_FAILURES: u32 = 2;

/// Per-plan failure record.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FailureRecord {
    /// Number of failures recorded.
    pub count: u32,
    /// Unix milliseconds of the last failure.
    pub last_failure_ms: Option<i64>,
    /// Descriptions of each failure (most recent last).
    pub reasons: Vec<String>,
}

/// Circuit breaker that tracks per-plan failures using a concurrent map.
///
/// Thread-safe: safe to call from multiple watcher threads simultaneously.
#[derive(Debug)]
pub struct CircuitBreaker {
    /// Maximum failures before tripping (configurable, defaults to [`MAX_PLAN_FAILURES`]).
    max_failures: u32,
    /// Per-plan failure records.
    records: DashMap<String, FailureRecord>,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new(MAX_PLAN_FAILURES)
    }
}

impl CircuitBreaker {
    /// Create a circuit breaker with a custom max-failures threshold.
    #[must_use]
    pub fn new(max_failures: u32) -> Self {
        Self {
            max_failures,
            records: DashMap::new(),
        }
    }

    /// Record a failure for the given plan.
    ///
    /// Returns `true` if this failure trips the circuit (i.e. the plan
    /// has now reached `max_failures`).
    pub fn record_failure(&self, plan_id: &str, reason: impl Into<String>, now_ms: i64) -> bool {
        let mut entry = self.records.entry(plan_id.to_owned()).or_default();
        entry.count += 1;
        entry.last_failure_ms = Some(now_ms);
        entry.reasons.push(reason.into());
        let count = entry.count;
        drop(entry);
        count >= self.max_failures
    }

    /// Check if the circuit has tripped for this plan (failures >= max).
    #[must_use]
    pub fn is_tripped(&self, plan_id: &str) -> bool {
        self.records
            .get(plan_id)
            .is_some_and(|r| r.count >= self.max_failures)
    }

    /// Check if the circuit is broken for this plan.
    ///
    /// Alias for [`Self::is_tripped`]; kept for the runtime wiring
    /// described in the checklist.
    #[must_use]
    pub fn is_broken(&self, plan_id: &str) -> bool {
        self.is_tripped(plan_id)
    }

    /// Get the current failure count for a plan.
    #[must_use]
    pub fn failure_count(&self, plan_id: &str) -> u32 {
        self.records.get(plan_id).map_or(0, |r| r.count)
    }

    /// Reset the failure record for a plan (e.g. after operator override).
    pub fn reset(&self, plan_id: &str) {
        self.records.remove(plan_id);
    }

    /// Reset all failure records.
    pub fn reset_all(&self) {
        self.records.clear();
    }

    /// Get a snapshot of the failure record for a plan, if any.
    #[must_use]
    pub fn get_record(&self, plan_id: &str) -> Option<FailureRecord> {
        self.records.get(plan_id).map(|r| r.value().clone())
    }

    /// Number of plans currently tracked.
    #[must_use]
    pub fn tracked_plans(&self) -> usize {
        self.records.len()
    }

    /// Maximum failures threshold.
    #[must_use]
    pub const fn max_failures(&self) -> u32 {
        self.max_failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn default_max_failures() {
        let cb = CircuitBreaker::default();
        assert_eq!(cb.max_failures(), MAX_PLAN_FAILURES);
    }

    #[test]
    fn no_failures_not_tripped() {
        let cb = CircuitBreaker::default();
        assert!(!cb.is_tripped("plan-1"));
        assert_eq!(cb.failure_count("plan-1"), 0);
    }

    #[test]
    fn single_failure_below_threshold() {
        let cb = CircuitBreaker::default();
        let tripped = cb.record_failure("plan-1", "compile error", 1000);
        assert!(!tripped);
        assert!(!cb.is_tripped("plan-1"));
        assert_eq!(cb.failure_count("plan-1"), 1);
    }

    #[test]
    fn trips_at_max_failures() {
        let cb = CircuitBreaker::new(2);
        assert!(!cb.record_failure("plan-1", "first", 1000));
        assert!(cb.record_failure("plan-1", "second", 2000));
        assert!(cb.is_tripped("plan-1"));
        assert!(cb.is_broken("plan-1"));
        assert_eq!(cb.failure_count("plan-1"), 2);
    }

    #[test]
    fn different_plans_independent() {
        let cb = CircuitBreaker::new(2);
        cb.record_failure("plan-a", "err", 100);
        cb.record_failure("plan-b", "err", 200);
        assert!(!cb.is_tripped("plan-a"));
        assert!(!cb.is_tripped("plan-b"));
        cb.record_failure("plan-a", "err2", 300);
        assert!(cb.is_tripped("plan-a"));
        assert!(!cb.is_tripped("plan-b"));
    }

    #[test]
    fn reset_clears_failures() {
        let cb = CircuitBreaker::new(2);
        cb.record_failure("plan-1", "err", 100);
        cb.record_failure("plan-1", "err2", 200);
        assert!(cb.is_tripped("plan-1"));
        cb.reset("plan-1");
        assert!(!cb.is_tripped("plan-1"));
        assert_eq!(cb.failure_count("plan-1"), 0);
    }

    #[test]
    fn reset_all_clears_everything() {
        let cb = CircuitBreaker::new(2);
        cb.record_failure("plan-1", "err", 100);
        cb.record_failure("plan-2", "err", 200);
        assert_eq!(cb.tracked_plans(), 2);
        cb.reset_all();
        assert_eq!(cb.tracked_plans(), 0);
    }

    #[test]
    fn get_record_returns_reasons() {
        let cb = CircuitBreaker::new(3);
        cb.record_failure("plan-1", "compile", 100);
        cb.record_failure("plan-1", "test fail", 200);
        let rec = cb.get_record("plan-1").expect("should exist");
        assert_eq!(rec.count, 2);
        assert_eq!(rec.reasons, vec!["compile", "test fail"]);
        assert_eq!(rec.last_failure_ms, Some(200));
    }

    #[test]
    fn get_record_missing_plan() {
        let cb = CircuitBreaker::default();
        assert!(cb.get_record("nonexistent").is_none());
    }

    #[test]
    fn concurrent_access_is_safe() {
        let cb = Arc::new(CircuitBreaker::new(100));
        let mut handles = Vec::new();
        for i in 0..10 {
            let cb = Arc::clone(&cb);
            handles.push(thread::spawn(move || {
                for j in 0..10 {
                    cb.record_failure("shared-plan", format!("t{i}-{j}"), i64::from(i * 10 + j));
                }
            }));
        }
        for h in handles {
            h.join().expect("thread panicked");
        }
        assert_eq!(cb.failure_count("shared-plan"), 100);
        assert!(cb.is_tripped("shared-plan"));
    }

    #[test]
    fn tripped_stays_tripped_on_more_failures() {
        let cb = CircuitBreaker::new(2);
        cb.record_failure("p", "a", 1);
        cb.record_failure("p", "b", 2);
        assert!(cb.is_tripped("p"));
        // Additional failures don't un-trip.
        cb.record_failure("p", "c", 3);
        assert!(cb.is_tripped("p"));
        assert_eq!(cb.failure_count("p"), 3);
    }

    #[test]
    fn custom_threshold() {
        let cb = CircuitBreaker::new(5);
        for i in 0..4 {
            assert!(!cb.record_failure("p", format!("err{i}"), i64::from(i)));
        }
        assert!(cb.record_failure("p", "err4", 4));
        assert!(cb.is_tripped("p"));
    }

    #[test]
    fn failure_record_serde_roundtrip() {
        let rec = FailureRecord {
            count: 3,
            last_failure_ms: Some(42_000),
            reasons: vec!["a".into(), "b".into(), "c".into()],
        };
        let json = serde_json::to_string(&rec).expect("serialize");
        let decoded: FailureRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.count, 3);
        assert_eq!(decoded.reasons.len(), 3);
    }
}
