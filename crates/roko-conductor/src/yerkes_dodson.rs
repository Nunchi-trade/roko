//! Yerkes-Dodson pressure framework (COND-04).
//!
//! Implements the inverted-U relationship between pressure and performance:
//! performance peaks at moderate pressure and declines at both extremes.
//!
//! The conductor uses this to adjust intervention aggressiveness:
//! - Low pressure → interventions are light (let the agent explore)
//! - Optimal pressure → peak performance, minimal intervention
//! - High pressure → interventions become more aggressive (risk of failure)

use serde::{Deserialize, Serialize};

/// Yerkes-Dodson inverted-U pressure model.
///
/// Models the empirical observation that performance peaks at moderate
/// arousal/pressure and degrades at both low and high extremes.
///
/// The curve is a Gaussian: `exp(-((pressure - optimal)^2) / (2 * width^2))`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YerkesDodson {
    /// Current pressure level (0.0 = no pressure, 1.0 = maximum).
    pub pressure: f64,
    /// Pressure level at which performance peaks.
    pub optimal: f64,
    /// Width of the Gaussian curve (controls how quickly performance
    /// drops off from the optimal point).
    pub width: f64,
}

impl Default for YerkesDodson {
    fn default() -> Self {
        Self {
            pressure: 0.5,
            optimal: 0.5,
            width: 0.25,
        }
    }
}

impl YerkesDodson {
    /// Create a new Yerkes-Dodson model with the given optimal pressure and width.
    #[must_use]
    pub fn new(optimal: f64, width: f64) -> Self {
        Self {
            pressure: 0.0,
            optimal,
            width: width.max(0.001), // avoid division by zero
        }
    }

    /// Set the current pressure level (clamped to [0, 1]).
    pub fn set_pressure(&mut self, pressure: f64) {
        self.pressure = pressure.clamp(0.0, 1.0);
    }

    /// Compute the performance multiplier at the current pressure.
    ///
    /// Returns a value in (0, 1] where 1.0 is peak performance (at optimal).
    /// The curve is Gaussian: `exp(-((pressure - optimal)^2) / (2 * width^2))`.
    #[must_use]
    pub fn performance_multiplier(&self) -> f64 {
        let diff = self.pressure - self.optimal;
        let exponent = -(diff * diff) / (2.0 * self.width * self.width);
        exponent.exp()
    }

    /// Compute the intervention aggressiveness at the current pressure.
    ///
    /// Inverse of performance: when performance is low, interventions
    /// should be more aggressive. Returns a value in [0, 1).
    #[must_use]
    pub fn intervention_aggressiveness(&self) -> f64 {
        1.0 - self.performance_multiplier()
    }

    /// Suggest a pressure adjustment to move toward optimal.
    ///
    /// Returns a signed delta: positive means "increase pressure"
    /// (e.g., tighten deadlines), negative means "decrease pressure"
    /// (e.g., extend budgets).
    #[must_use]
    pub fn pressure_delta(&self) -> f64 {
        self.optimal - self.pressure
    }

    /// Whether we are in the "danger zone" — performance is below 50%.
    #[must_use]
    pub fn is_danger_zone(&self) -> bool {
        self.performance_multiplier() < 0.5
    }

    /// Derive pressure from conductor signals.
    ///
    /// Inputs are normalized counts (0.0 = none, 1.0 = saturated):
    /// - `cost_pressure`: fraction of budget consumed
    /// - `time_pressure`: fraction of time budget consumed
    /// - `failure_rate`: recent gate failure rate
    /// - `stuck_signals`: number of stuck/loop watcher triggers
    #[must_use]
    pub fn compute_pressure(
        cost_pressure: f64,
        time_pressure: f64,
        failure_rate: f64,
        stuck_signals: f64,
    ) -> f64 {
        // Weighted average of pressure signals.
        let raw = cost_pressure * 0.25
            + time_pressure * 0.25
            + failure_rate * 0.30
            + stuck_signals * 0.20;
        raw.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_performance_at_optimal() {
        let yd = YerkesDodson {
            pressure: 0.5,
            optimal: 0.5,
            width: 0.25,
        };
        let perf = yd.performance_multiplier();
        assert!((perf - 1.0).abs() < 1e-10, "expected 1.0, got {perf}");
    }

    #[test]
    fn performance_degrades_away_from_optimal() {
        let mut yd = YerkesDodson::new(0.5, 0.25);
        yd.set_pressure(0.5);
        let peak = yd.performance_multiplier();

        yd.set_pressure(0.0);
        let low = yd.performance_multiplier();

        yd.set_pressure(1.0);
        let high = yd.performance_multiplier();

        assert!(peak > low, "peak {peak} should exceed low-pressure {low}");
        assert!(
            peak > high,
            "peak {peak} should exceed high-pressure {high}"
        );
    }

    #[test]
    fn symmetric_around_optimal() {
        let mut yd = YerkesDodson::new(0.5, 0.25);
        yd.set_pressure(0.3);
        let left = yd.performance_multiplier();
        yd.set_pressure(0.7);
        let right = yd.performance_multiplier();
        assert!(
            (left - right).abs() < 1e-10,
            "symmetric points should have equal performance: {left} vs {right}"
        );
    }

    #[test]
    fn intervention_aggressiveness_inverse() {
        let mut yd = YerkesDodson::new(0.5, 0.25);
        yd.set_pressure(0.5);
        let aggr = yd.intervention_aggressiveness();
        assert!(
            aggr < 0.01,
            "at optimal, aggressiveness should be ~0, got {aggr}"
        );

        yd.set_pressure(0.0);
        let aggr_low = yd.intervention_aggressiveness();
        assert!(
            aggr_low > 0.5,
            "far from optimal, aggressiveness should be high: {aggr_low}"
        );
    }

    #[test]
    fn pressure_delta_points_toward_optimal() {
        let mut yd = YerkesDodson::new(0.5, 0.25);
        yd.set_pressure(0.2);
        assert!(
            yd.pressure_delta() > 0.0,
            "below optimal, delta should be positive"
        );

        yd.set_pressure(0.8);
        assert!(
            yd.pressure_delta() < 0.0,
            "above optimal, delta should be negative"
        );
    }

    #[test]
    fn danger_zone_detection() {
        let mut yd = YerkesDodson::new(0.5, 0.15);
        yd.set_pressure(0.5);
        assert!(!yd.is_danger_zone());

        yd.set_pressure(0.0);
        assert!(yd.is_danger_zone());

        yd.set_pressure(1.0);
        assert!(yd.is_danger_zone());
    }

    #[test]
    fn compute_pressure_clamps() {
        let p = YerkesDodson::compute_pressure(1.0, 1.0, 1.0, 1.0);
        assert!((p - 1.0).abs() < 1e-10);

        let p = YerkesDodson::compute_pressure(0.0, 0.0, 0.0, 0.0);
        assert!(p.abs() < 1e-10);
    }

    #[test]
    fn width_prevents_division_by_zero() {
        let yd = YerkesDodson::new(0.5, 0.0);
        assert!(yd.width > 0.0);
        let _ = yd.performance_multiplier(); // should not panic
    }

    #[test]
    fn default_is_at_moderate_pressure() {
        let yd = YerkesDodson::default();
        assert!((yd.pressure - 0.5).abs() < 1e-10);
        assert!((yd.optimal - 0.5).abs() < 1e-10);
    }

    #[test]
    fn narrow_width_makes_sharper_curve() {
        let mut narrow = YerkesDodson::new(0.5, 0.1);
        let mut wide = YerkesDodson::new(0.5, 0.4);

        narrow.set_pressure(0.3);
        wide.set_pressure(0.3);

        // Narrow curve drops off faster.
        assert!(
            narrow.performance_multiplier() < wide.performance_multiplier(),
            "narrow {} should be less than wide {}",
            narrow.performance_multiplier(),
            wide.performance_multiplier()
        );
    }
}
