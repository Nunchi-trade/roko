//! Property-based tests for roko-conductor core types.

use proptest::prelude::*;
use roko_conductor::circuit_breaker::{CircuitBreaker, CircuitBreakerState};
use roko_conductor::yerkes_dodson::YerkesDodson;

// ── CircuitBreaker properties ────────────────────────────────────────────────

proptest! {
    /// Recording exactly max_failures failures always trips the breaker.
    #[test]
    fn breaker_trips_at_max_failures(max_failures in 1u32..10) {
        let cb = CircuitBreaker::new(max_failures);
        for i in 0..max_failures {
            let tripped = cb.record_failure("plan-x", format!("fail-{i}"), i as i64 * 1000);
            if i + 1 < max_failures {
                prop_assert!(!tripped, "should not trip at failure {}", i + 1);
            } else {
                prop_assert!(tripped, "should trip at failure {}", i + 1);
            }
        }
        prop_assert!(cb.is_tripped("plan-x"));
    }

    /// Failure count matches number of record_failure calls.
    #[test]
    fn failure_count_tracks_calls(n in 0u32..20) {
        let cb = CircuitBreaker::new(100); // high threshold so it never trips
        for i in 0..n {
            cb.record_failure("plan-a", format!("reason-{i}"), i as i64);
        }
        prop_assert_eq!(cb.failure_count("plan-a"), n);
    }

    /// Reset clears all state for a plan.
    #[test]
    fn reset_clears_plan(n in 1u32..10) {
        let cb = CircuitBreaker::new(100);
        for i in 0..n {
            cb.record_failure("plan-b", format!("reason-{i}"), i as i64);
        }
        cb.reset("plan-b");
        prop_assert_eq!(cb.failure_count("plan-b"), 0);
        prop_assert!(!cb.is_tripped("plan-b"));
    }

    /// Snapshot-restore roundtrip preserves circuit breaker state.
    #[test]
    fn snapshot_restore_roundtrip(failures_a in 0u32..5, failures_b in 0u32..5) {
        let cb = CircuitBreaker::new(10);
        for i in 0..failures_a {
            cb.record_failure("plan-a", format!("a-{i}"), i as i64);
        }
        for i in 0..failures_b {
            cb.record_failure("plan-b", format!("b-{i}"), i as i64);
        }
        let state = cb.snapshot_state();
        let restored = CircuitBreaker::from_state(state);
        prop_assert_eq!(restored.failure_count("plan-a"), failures_a);
        prop_assert_eq!(restored.failure_count("plan-b"), failures_b);
    }

    /// Snapshot serialization roundtrip via serde.
    #[test]
    fn snapshot_serde_roundtrip(n in 0u32..5) {
        let cb = CircuitBreaker::new(7);
        for i in 0..n {
            cb.record_failure("plan-s", format!("reason-{i}"), i as i64 * 100);
        }
        let state = cb.snapshot_state();
        let json = serde_json::to_string(&state).expect("serialize");
        let parsed: CircuitBreakerState = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(state, parsed);
    }
}

// ── YerkesDodson properties ──────────────────────────────────────────────────

proptest! {
    /// Performance multiplier is always in [0, 1].
    #[test]
    fn performance_in_unit_range(
        pressure in 0.0f64..=1.0,
        optimal in 0.0f64..=1.0,
        width in 0.01f64..=1.0,
    ) {
        let yd = YerkesDodson { pressure, optimal, width };
        let perf = yd.performance_multiplier();
        prop_assert!(perf >= 0.0 && perf <= 1.0,
            "performance {perf} out of [0, 1] for pressure={pressure}, optimal={optimal}, width={width}");
    }

    /// Peak performance is at the optimal pressure.
    #[test]
    fn peak_at_optimal(
        optimal in 0.1f64..=0.9,
        width in 0.05f64..=0.5,
    ) {
        let peak = YerkesDodson { pressure: optimal, optimal, width };
        let off_by_01 = YerkesDodson { pressure: (optimal + 0.1).min(1.0), optimal, width };
        prop_assert!(peak.performance_multiplier() >= off_by_01.performance_multiplier(),
            "peak {} should be >= off-by-0.1 {}", peak.performance_multiplier(), off_by_01.performance_multiplier());
    }

    /// Intervention aggressiveness is complement of performance.
    #[test]
    fn aggressiveness_complement_of_performance(
        pressure in 0.0f64..=1.0,
        optimal in 0.0f64..=1.0,
        width in 0.01f64..=1.0,
    ) {
        let yd = YerkesDodson { pressure, optimal, width };
        let sum = yd.performance_multiplier() + yd.intervention_aggressiveness();
        prop_assert!((sum - 1.0).abs() < 1e-10,
            "performance + aggressiveness should equal 1.0, got {sum}");
    }

    /// Symmetry: equal distance from optimal gives equal performance.
    #[test]
    fn symmetric_around_optimal(
        optimal in 0.2f64..=0.8,
        delta in 0.0f64..=0.2,
        width in 0.05f64..=0.5,
    ) {
        let left = YerkesDodson { pressure: optimal - delta, optimal, width };
        let right = YerkesDodson { pressure: optimal + delta, optimal, width };
        prop_assert!((left.performance_multiplier() - right.performance_multiplier()).abs() < 1e-10,
            "symmetric points should match: {} vs {}", left.performance_multiplier(), right.performance_multiplier());
    }

    /// compute_pressure is always in [0, 1].
    #[test]
    fn compute_pressure_in_unit_range(
        cost in 0.0f64..=2.0,
        time in 0.0f64..=2.0,
        failure in 0.0f64..=2.0,
        stuck in 0.0f64..=2.0,
    ) {
        let p = YerkesDodson::compute_pressure(cost, time, failure, stuck);
        prop_assert!(p >= 0.0 && p <= 1.0,
            "compute_pressure({cost}, {time}, {failure}, {stuck}) = {p} out of [0, 1]");
    }

    /// set_pressure clamps to [0, 1].
    #[test]
    fn set_pressure_clamps(raw in -2.0f64..=3.0) {
        let mut yd = YerkesDodson::default();
        yd.set_pressure(raw);
        prop_assert!(yd.pressure >= 0.0 && yd.pressure <= 1.0,
            "set_pressure({raw}) resulted in {}", yd.pressure);
    }

    /// YerkesDodson serde roundtrip.
    #[test]
    fn yerkes_dodson_serde_roundtrip(
        pressure in 0.0f64..=1.0,
        optimal in 0.0f64..=1.0,
        width in 0.01f64..=1.0,
    ) {
        let yd = YerkesDodson { pressure, optimal, width };
        let json = serde_json::to_string(&yd).expect("serialize");
        let parsed: YerkesDodson = serde_json::from_str(&json).expect("deserialize");
        prop_assert!((yd.pressure - parsed.pressure).abs() < 1e-10);
        prop_assert!((yd.optimal - parsed.optimal).abs() < 1e-10);
        prop_assert!((yd.width - parsed.width).abs() < 1e-10);
    }
}
