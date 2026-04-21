//! Circuit breaker: per-plan failure budget tracking.
//!
//! After [`MAX_PLAN_FAILURES`] failures on a single plan, the circuit
//! breaker trips and the plan is aborted. Uses [`DashMap`] for lock-free
//! concurrent access from multiple watcher threads.
//!
//! ## Predictive circuit breaking (COND-08)
//!
//! The [`HoltForecaster`] uses Holt exponential smoothing (level + trend)
//! to forecast error rates and proactively trip the breaker before the
//! count threshold is actually reached. This avoids the cost of the Nth
//! failure. The existing count-based trip is preserved as a fallback.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Maximum number of failures allowed per plan before the circuit breaks.
pub const MAX_PLAN_FAILURES: u32 = 2;

/// Per-plan failure record.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureRecord {
    /// Number of failures recorded.
    pub count: u32,
    /// Unix milliseconds of the last failure.
    pub last_failure_ms: Option<i64>,
    /// Descriptions of each failure (most recent last).
    pub reasons: Vec<String>,
}

// ─── Holt Exponential Smoothing Forecaster (COND-08) ─────────────────

/// Holt exponential smoothing (double exponential) forecaster for error rate
/// trend projection.
///
/// Two equations updated on each observation:
/// ```text
/// level(t) = alpha * observation(t) + (1 - alpha) * (level(t-1) + trend(t-1))
/// trend(t) = beta  * (level(t) - level(t-1)) + (1 - beta) * trend(t-1)
/// forecast(t+h) = level(t) + h * trend(t)
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoltForecaster {
    /// Smoothed level component.
    pub level: f64,
    /// Trend component.
    pub trend: f64,
    /// Level smoothing factor (default 0.3).
    pub alpha: f64,
    /// Trend smoothing factor (default 0.1).
    pub beta: f64,
    /// Total observations fed to the forecaster.
    pub observations: u32,
}

impl Default for HoltForecaster {
    fn default() -> Self {
        Self {
            level: 0.0,
            trend: 0.0,
            alpha: 0.3,
            beta: 0.1,
            observations: 0,
        }
    }
}

impl HoltForecaster {
    /// Create a forecaster with custom smoothing parameters.
    #[must_use]
    pub fn new(alpha: f64, beta: f64) -> Self {
        Self {
            alpha: alpha.clamp(0.0, 1.0),
            beta: beta.clamp(0.0, 1.0),
            ..Default::default()
        }
    }

    /// Update the forecaster with a new observation.
    pub fn update(&mut self, observation: f64) {
        if self.observations == 0 {
            self.level = observation;
            self.trend = 0.0;
        } else {
            let prev_level = self.level;
            self.level = self.alpha * observation + (1.0 - self.alpha) * (self.level + self.trend);
            self.trend = self.beta * (self.level - prev_level) + (1.0 - self.beta) * self.trend;
        }
        self.observations += 1;
    }

    /// Forecast the value at `horizon` steps ahead.
    #[must_use]
    pub fn forecast(&self, horizon: usize) -> f64 {
        self.level + (horizon as f64) * self.trend
    }

    /// Number of observations recorded.
    #[must_use]
    pub const fn observation_count(&self) -> u32 {
        self.observations
    }
}

/// Proactive warning from the Holt forecaster.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProactiveTripSignal {
    /// Forecast at horizon 3 exceeds threshold -- early warning.
    Warning {
        /// Plan that is trending toward tripping.
        plan_id: String,
        /// Forecasted error rate at horizon 3.
        forecast_h3: f64,
    },
    /// Forecast at horizon 1 exceeds threshold -- proactive trip.
    ProactiveTrip {
        /// Plan proactively tripped.
        plan_id: String,
        /// Forecasted error rate at horizon 1.
        forecast_h1: f64,
    },
}

/// Serializable circuit-breaker state for persistence across restarts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CircuitBreakerState {
    /// Maximum failures threshold active when the snapshot was captured.
    pub max_failures: u32,
    /// Per-plan failure records keyed by `plan_id`.
    #[serde(default)]
    pub records: HashMap<String, FailureRecord>,
}

/// Circuit breaker that tracks per-plan failures using a concurrent map.
///
/// Thread-safe: safe to call from multiple watcher threads simultaneously.
///
/// ## Predictive mode (COND-08)
///
/// When predictive mode is enabled, each plan also tracks a [`HoltForecaster`]
/// that projects the error rate forward. If the forecast exceeds the trip
/// threshold at horizon 1, the breaker proactively opens. At horizon 3, a
/// warning is emitted. The existing count-based trip is always retained as
/// a fallback safety mechanism.
#[derive(Debug)]
pub struct CircuitBreaker {
    /// Maximum failures before tripping (configurable, defaults to [`MAX_PLAN_FAILURES`]).
    max_failures: u32,
    /// Per-plan failure records.
    records: DashMap<String, FailureRecord>,
    /// Per-plan Holt forecasters for predictive tripping (COND-08).
    forecasters: DashMap<String, HoltForecaster>,
    /// Per-plan total evaluation count (successes + failures) for error rate.
    eval_counts: DashMap<String, (u32, u32)>,
    /// Whether predictive mode is enabled.
    predictive: bool,
    /// Trip threshold for the forecasted error rate (default: same as
    /// `max_failures` converted to a rate, typically ~0.5).
    forecast_trip_threshold: f64,
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
            forecasters: DashMap::new(),
            eval_counts: DashMap::new(),
            predictive: false,
            forecast_trip_threshold: 0.5,
        }
    }

    /// Enable predictive circuit breaking using Holt exponential smoothing.
    ///
    /// The `trip_threshold` is the forecasted error rate (0.0 - 1.0) above
    /// which the breaker proactively trips. Default: 0.5.
    #[must_use]
    pub fn with_predictive(mut self, trip_threshold: f64) -> Self {
        self.predictive = true;
        self.forecast_trip_threshold = trip_threshold.clamp(0.01, 1.0);
        self
    }

    /// Whether predictive mode is active.
    #[must_use]
    pub const fn is_predictive(&self) -> bool {
        self.predictive
    }

    /// Rebuild a circuit breaker from a previously persisted state snapshot.
    #[must_use]
    pub fn from_state(state: CircuitBreakerState) -> Self {
        let cb = Self::new(state.max_failures);
        for (plan_id, record) in state.records {
            cb.records.insert(plan_id, record);
        }
        cb
    }

    /// Record a failure for the given plan.
    ///
    /// Returns `true` if this failure trips the circuit (i.e. the plan
    /// has now reached `max_failures` or the forecaster proactively trips).
    pub fn record_failure(&self, plan_id: &str, reason: impl Into<String>, now_ms: i64) -> bool {
        let mut entry = self.records.entry(plan_id.to_owned()).or_default();
        entry.count += 1;
        entry.last_failure_ms = Some(now_ms);
        entry.reasons.push(reason.into());
        let count = entry.count;
        drop(entry);

        // Update Holt forecaster with failure observation (1.0).
        if self.predictive {
            self.update_forecaster(plan_id, 1.0);
        }

        // Count-based trip check (fallback always active).
        if count >= self.max_failures {
            return true;
        }

        // Predictive trip: if forecast(1) exceeds threshold, proactively trip.
        // Requires at least 2 observations to have a meaningful trend.
        if self.predictive {
            if let Some(f) = self.forecasters.get(plan_id) {
                if f.observation_count() >= 2 && f.forecast(1) >= self.forecast_trip_threshold {
                    return true;
                }
            }
        }

        false
    }

    /// Record a success for the given plan (no failure).
    ///
    /// In predictive mode this updates the Holt forecaster with a success
    /// observation (0.0), improving the error rate forecast.
    pub fn record_success(&self, plan_id: &str) {
        if self.predictive {
            self.update_forecaster(plan_id, 0.0);
        }
    }

    /// Update the forecaster for a plan with an observation.
    fn update_forecaster(&self, plan_id: &str, observation: f64) {
        let mut entry = self.eval_counts.entry(plan_id.to_owned()).or_default();
        if observation > 0.5 {
            entry.0 += 1; // failures
        }
        entry.1 += 1; // total
        drop(entry);

        self.forecasters
            .entry(plan_id.to_owned())
            .or_default()
            .update(observation);
    }

    /// Check for proactive trip signals from the Holt forecaster.
    ///
    /// Returns `None` if predictive mode is off or the forecaster does not
    /// project any threshold breach. Returns a warning at horizon 3 or a
    /// proactive trip at horizon 1.
    #[must_use]
    pub fn check_proactive(&self, plan_id: &str) -> Option<ProactiveTripSignal> {
        if !self.predictive {
            return None;
        }
        let f = self.forecasters.get(plan_id)?;
        if f.observation_count() < 2 {
            // Need at least 2 observations to have a meaningful trend.
            return None;
        }

        let h1 = f.forecast(1);
        let h3 = f.forecast(3);

        if h1 >= self.forecast_trip_threshold {
            return Some(ProactiveTripSignal::ProactiveTrip {
                plan_id: plan_id.to_owned(),
                forecast_h1: h1,
            });
        }

        if h3 >= self.forecast_trip_threshold {
            return Some(ProactiveTripSignal::Warning {
                plan_id: plan_id.to_owned(),
                forecast_h3: h3,
            });
        }

        None
    }

    /// Get the Holt forecaster for a plan, if one exists.
    #[must_use]
    pub fn get_forecaster(&self, plan_id: &str) -> Option<HoltForecaster> {
        self.forecasters.get(plan_id).map(|f| f.value().clone())
    }

    /// Check if the circuit has tripped for this plan (failures >= max).
    #[must_use]
    pub fn is_tripped(&self, plan_id: &str) -> bool {
        // Count-based trip (always checked).
        if self
            .records
            .get(plan_id)
            .is_some_and(|r| r.count >= self.max_failures)
        {
            return true;
        }

        // Predictive trip: forecast(1) exceeds threshold.
        if self.predictive {
            if let Some(f) = self.forecasters.get(plan_id) {
                if f.observation_count() >= 2 && f.forecast(1) >= self.forecast_trip_threshold {
                    return true;
                }
            }
        }

        false
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
        self.forecasters.remove(plan_id);
        self.eval_counts.remove(plan_id);
    }

    /// Reset all failure records.
    pub fn reset_all(&self) {
        self.records.clear();
        self.forecasters.clear();
        self.eval_counts.clear();
    }

    /// Get a snapshot of the failure record for a plan, if any.
    #[must_use]
    pub fn get_record(&self, plan_id: &str) -> Option<FailureRecord> {
        self.records.get(plan_id).map(|r| r.value().clone())
    }

    /// Capture a serializable snapshot of the current breaker state.
    #[must_use]
    pub fn snapshot_state(&self) -> CircuitBreakerState {
        CircuitBreakerState {
            max_failures: self.max_failures,
            records: self
                .records
                .iter()
                .map(|entry| (entry.key().clone(), entry.value().clone()))
                .collect(),
        }
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

    #[test]
    fn snapshot_state_roundtrip_preserves_threshold_and_records() {
        let cb = CircuitBreaker::new(3);
        cb.record_failure("plan-1", "compile", 100);
        cb.record_failure("plan-1", "tests", 200);
        cb.record_failure("plan-2", "timeout", 300);

        let restored = CircuitBreaker::from_state(cb.snapshot_state());

        assert_eq!(restored.max_failures(), 3);
        assert_eq!(restored.failure_count("plan-1"), 2);
        assert_eq!(restored.failure_count("plan-2"), 1);
        assert_eq!(
            restored
                .get_record("plan-1")
                .expect("plan-1 record should exist")
                .reasons,
            vec!["compile", "tests"]
        );
        assert!(!restored.is_tripped("plan-1"));
    }

    // ── Holt Forecaster tests (COND-08) ─────────────────────────────

    #[test]
    fn holt_forecaster_default_state() {
        let f = HoltForecaster::default();
        assert_eq!(f.observation_count(), 0);
        assert!((f.level - 0.0).abs() < f64::EPSILON);
        assert!((f.trend - 0.0).abs() < f64::EPSILON);
        assert!((f.alpha - 0.3).abs() < f64::EPSILON);
        assert!((f.beta - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn holt_forecaster_first_observation_sets_level() {
        let mut f = HoltForecaster::default();
        f.update(0.4);
        assert!((f.level - 0.4).abs() < f64::EPSILON);
        assert!((f.trend - 0.0).abs() < f64::EPSILON);
        assert_eq!(f.observation_count(), 1);
    }

    #[test]
    fn holt_forecaster_second_observation_updates_trend() {
        let mut f = HoltForecaster::default();
        f.update(0.2);
        f.update(0.4);
        // After 2 observations, trend should be positive (rising errors).
        assert!(f.trend > 0.0, "trend should be positive, got {}", f.trend);
        assert_eq!(f.observation_count(), 2);
    }

    #[test]
    fn holt_forecaster_forecast_increases_with_positive_trend() {
        let mut f = HoltForecaster::default();
        // Feed increasing observations.
        for i in 1..=5 {
            f.update(i as f64 * 0.1);
        }
        let h1 = f.forecast(1);
        let h3 = f.forecast(3);
        assert!(
            h3 > h1,
            "forecast at h3 ({h3}) should exceed h1 ({h1}) with positive trend"
        );
    }

    #[test]
    fn holt_forecaster_forecast_decreases_with_negative_trend() {
        let mut f = HoltForecaster::default();
        // Feed decreasing observations.
        for i in (1..=5).rev() {
            f.update(i as f64 * 0.2);
        }
        let h1 = f.forecast(1);
        let h3 = f.forecast(3);
        assert!(
            h3 < h1,
            "forecast at h3 ({h3}) should be below h1 ({h1}) with negative trend"
        );
    }

    #[test]
    fn holt_forecaster_serde_roundtrip() {
        let mut f = HoltForecaster::default();
        f.update(0.3);
        f.update(0.5);
        let json = serde_json::to_string(&f).expect("serialize");
        let decoded: HoltForecaster = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.observation_count(), 2);
        assert!((decoded.level - f.level).abs() < f64::EPSILON);
    }

    #[test]
    fn predictive_breaker_not_active_by_default() {
        let cb = CircuitBreaker::default();
        assert!(!cb.is_predictive());
        assert!(cb.check_proactive("any-plan").is_none());
    }

    #[test]
    fn predictive_breaker_warns_on_rising_error_rate() {
        let cb = CircuitBreaker::new(10).with_predictive(0.5);
        assert!(cb.is_predictive());

        // Feed a stream of failures to build a rising trend.
        for i in 0..5 {
            cb.record_failure("plan-1", format!("err-{i}"), i64::from(i * 100));
        }

        // With 5 consecutive failures, the forecaster should project high error rates.
        let signal = cb.check_proactive("plan-1");
        assert!(
            signal.is_some(),
            "expected proactive signal after 5 consecutive failures"
        );
    }

    #[test]
    fn predictive_breaker_no_warning_on_successes() {
        let cb = CircuitBreaker::new(10).with_predictive(0.5);

        // Record successes only.
        for _ in 0..5 {
            cb.record_success("plan-1");
        }

        let signal = cb.check_proactive("plan-1");
        assert!(signal.is_none(), "no signal expected with only successes");
    }

    #[test]
    fn predictive_breaker_preserves_count_based_fallback() {
        // Even with predictive mode, count-based trip still works.
        let cb = CircuitBreaker::new(2).with_predictive(0.9);
        assert!(!cb.record_failure("plan-1", "first", 100));
        assert!(cb.record_failure("plan-1", "second", 200));
        assert!(cb.is_tripped("plan-1"));
    }

    #[test]
    fn predictive_breaker_reset_clears_forecaster() {
        let cb = CircuitBreaker::new(10).with_predictive(0.5);
        cb.record_failure("plan-1", "err", 100);
        assert!(cb.get_forecaster("plan-1").is_some());
        cb.reset("plan-1");
        assert!(cb.get_forecaster("plan-1").is_none());
    }

    #[test]
    fn predictive_breaker_record_success_updates_forecaster() {
        let cb = CircuitBreaker::new(10).with_predictive(0.5);
        cb.record_success("plan-1");
        let f = cb
            .get_forecaster("plan-1")
            .expect("forecaster should exist");
        assert_eq!(f.observation_count(), 1);
        assert!((f.level - 0.0).abs() < f64::EPSILON);
    }
}
