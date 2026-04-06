//! Failure ratcheting — prevents rung regression in the gate pipeline.
//!
//! Once a plan has passed rung N, it should never be allowed to regress
//! to rung N-1. [`GateRatchet`] tracks the highest rung each plan has
//! passed and provides a `can_regress` check that the conductor uses
//! before accepting a lower verdict.
//!
//! This protects against convergence loops that thrash: an agent fixes
//! the compile error but breaks lint, then fixes lint but breaks compile
//! again. The ratchet makes the second regression visible and blockable.

use std::collections::HashMap;

/// Tracks the highest rung passed per plan, preventing regression.
#[derive(Clone, Debug, Default)]
pub struct GateRatchet {
    /// Map from plan identifier to the highest rung that plan has passed.
    passes: HashMap<String, u8>,
}

impl GateRatchet {
    /// Create an empty ratchet with no recorded passes.
    #[must_use]
    pub fn new() -> Self {
        Self {
            passes: HashMap::new(),
        }
    }

    /// Record that `plan_id` passed `rung`.
    ///
    /// Only updates the record if `rung` is higher than the previously
    /// recorded highest pass (or if no pass has been recorded yet).
    pub fn record_pass(&mut self, plan_id: impl Into<String>, rung: u8) {
        let key = plan_id.into();
        let entry = self.passes.entry(key).or_insert(0);
        if rung > *entry {
            *entry = rung;
        }
    }

    /// Get the highest rung that `plan_id` has passed, or `None` if no
    /// pass has been recorded.
    #[must_use]
    pub fn highest_pass(&self, plan_id: &str) -> Option<u8> {
        self.passes.get(plan_id).copied()
    }

    /// Returns `false` if `plan_id` has already passed a rung strictly
    /// higher than `rung`. In that case, accepting `rung` as the new
    /// highest would be a regression.
    ///
    /// Returns `true` if:
    /// - The plan has never been recorded (no regression possible)
    /// - The plan's highest pass is equal to or lower than `rung`
    #[must_use]
    pub fn can_regress(&self, plan_id: &str, rung: u8) -> bool {
        match self.passes.get(plan_id) {
            None => true,
            Some(&highest) => rung >= highest,
        }
    }

    /// Number of plans tracked by this ratchet.
    #[must_use]
    pub fn plan_count(&self) -> usize {
        self.passes.len()
    }

    /// Remove all recorded passes, resetting the ratchet.
    pub fn clear(&mut self) {
        self.passes.clear();
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratchet_new_is_empty() {
        let ratchet = GateRatchet::new();
        assert_eq!(ratchet.plan_count(), 0);
        assert!(ratchet.highest_pass("any").is_none());
    }

    #[test]
    fn ratchet_record_and_query() {
        let mut ratchet = GateRatchet::new();
        ratchet.record_pass("plan-1", 2);
        assert_eq!(ratchet.highest_pass("plan-1"), Some(2));
        assert_eq!(ratchet.plan_count(), 1);
    }

    #[test]
    fn ratchet_only_advances() {
        let mut ratchet = GateRatchet::new();
        ratchet.record_pass("plan-1", 3);
        ratchet.record_pass("plan-1", 1); // lower, should be ignored
        assert_eq!(ratchet.highest_pass("plan-1"), Some(3));
    }

    #[test]
    fn ratchet_can_regress_prevents_regression() {
        let mut ratchet = GateRatchet::new();
        ratchet.record_pass("plan-1", 4);

        // Rung 3 is below highest pass 4 -> regression
        assert!(!ratchet.can_regress("plan-1", 3));
        // Rung 0 is way below -> regression
        assert!(!ratchet.can_regress("plan-1", 0));
    }

    #[test]
    fn ratchet_can_regress_allows_same_or_higher() {
        let mut ratchet = GateRatchet::new();
        ratchet.record_pass("plan-1", 2);

        // Same rung is allowed (not a regression)
        assert!(ratchet.can_regress("plan-1", 2));
        // Higher rung is fine
        assert!(ratchet.can_regress("plan-1", 5));
    }

    #[test]
    fn ratchet_can_regress_unknown_plan_returns_true() {
        let ratchet = GateRatchet::new();
        assert!(ratchet.can_regress("unknown", 0));
        assert!(ratchet.can_regress("unknown", 6));
    }

    #[test]
    fn ratchet_multiple_plans_independent() {
        let mut ratchet = GateRatchet::new();
        ratchet.record_pass("plan-a", 3);
        ratchet.record_pass("plan-b", 5);

        assert_eq!(ratchet.highest_pass("plan-a"), Some(3));
        assert_eq!(ratchet.highest_pass("plan-b"), Some(5));
        assert_eq!(ratchet.plan_count(), 2);

        // plan-a at rung 1 is regression, plan-b at rung 1 is also regression
        assert!(!ratchet.can_regress("plan-a", 1));
        assert!(!ratchet.can_regress("plan-b", 1));

        // plan-a at rung 4 is fine, plan-b at rung 4 is regression
        assert!(ratchet.can_regress("plan-a", 4));
        assert!(!ratchet.can_regress("plan-b", 4));
    }

    #[test]
    fn ratchet_clear_resets_all() {
        let mut ratchet = GateRatchet::new();
        ratchet.record_pass("plan-1", 3);
        ratchet.record_pass("plan-2", 5);
        assert_eq!(ratchet.plan_count(), 2);

        ratchet.clear();
        assert_eq!(ratchet.plan_count(), 0);
        assert!(ratchet.highest_pass("plan-1").is_none());
        assert!(ratchet.can_regress("plan-1", 0));
    }

    #[test]
    fn ratchet_default_is_new() {
        let ratchet = GateRatchet::default();
        assert_eq!(ratchet.plan_count(), 0);
    }

    #[test]
    fn ratchet_record_pass_zero_rung() {
        let mut ratchet = GateRatchet::new();
        ratchet.record_pass("plan-1", 0);
        assert_eq!(ratchet.highest_pass("plan-1"), Some(0));
        // Can't regress below 0 — but 0 itself is still fine
        assert!(ratchet.can_regress("plan-1", 0));
    }

    #[test]
    fn ratchet_monotonic_sequence() {
        let mut ratchet = GateRatchet::new();
        for rung in 0..=6 {
            ratchet.record_pass("plan-1", rung);
            assert_eq!(ratchet.highest_pass("plan-1"), Some(rung));
        }
        // Final highest is 6
        assert_eq!(ratchet.highest_pass("plan-1"), Some(6));
        // Nothing can regress
        for rung in 0..6 {
            assert!(!ratchet.can_regress("plan-1", rung));
        }
        // Only 6 is allowed
        assert!(ratchet.can_regress("plan-1", 6));
    }

    #[test]
    fn ratchet_string_plan_ids() {
        let mut ratchet = GateRatchet::new();
        ratchet.record_pass(String::from("owned-id"), 2);
        ratchet.record_pass("borrowed-id", 3);
        assert_eq!(ratchet.highest_pass("owned-id"), Some(2));
        assert_eq!(ratchet.highest_pass("borrowed-id"), Some(3));
    }
}
