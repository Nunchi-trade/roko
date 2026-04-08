//! Regression detection: compare current metrics against a historical baseline
//! and fire alerts when key indicators breach configurable thresholds.
//!
//! The regression detector answers: "did this configuration change make things
//! worse?" by comparing a fresh batch of [`TaskMetric`] records against a
//! previously computed [`Baseline`].
//!
//! # Thresholds
//!
//! Default thresholds (from §1 of `roko-continuous-tuning.md`):
//!
//! - **Pass rate drop > 15%** → alert
//! - **Cost increase > 20%** → alert
//! - **Duration increase > 30%** → warning
//! - **Iterations increase > 25%** → warning

use roko_core::metric::TaskMetric;
use serde::{Deserialize, Serialize};

use crate::baseline::{Baseline, compute_baseline};

// ─── Config ─────────────────────────────────────────────────────────────────

/// Configurable thresholds for regression detection.
///
/// Each threshold is expressed as a fractional change (e.g. 0.15 = 15%).
/// Positive values mean the metric worsened (pass rate dropped, cost rose).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegressionThresholds {
    /// Maximum allowed drop in first-attempt pass rate (e.g. 0.15 = 15%).
    pub pass_rate_drop: f64,
    /// Maximum allowed increase in average cost (e.g. 0.20 = 20%).
    pub cost_increase: f64,
    /// Maximum allowed increase in average duration (e.g. 0.30 = 30%).
    pub duration_increase: f64,
    /// Maximum allowed increase in average iterations (e.g. 0.25 = 25%).
    pub iterations_increase: f64,
    /// Minimum number of records needed before regression detection fires.
    pub min_records: usize,
}

impl Default for RegressionThresholds {
    fn default() -> Self {
        Self {
            pass_rate_drop: 0.15,
            cost_increase: 0.20,
            duration_increase: 0.30,
            iterations_increase: 0.25,
            min_records: 5,
        }
    }
}

// ─── RegressionAlert ────────────────────────────────────────────────────────

/// Severity of a regression alert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertSeverity {
    /// A key metric has degraded past the threshold (pass rate, cost).
    Alert,
    /// A secondary metric has degraded past the threshold (duration, iterations).
    Warning,
    /// Metric improved relative to baseline.
    Improvement,
}

/// A single regression alert describing one breached threshold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegressionAlert {
    /// Which metric regressed (e.g. `"pass_rate"`, `"cost"`).
    pub metric_name: String,
    /// Severity of this alert.
    pub severity: AlertSeverity,
    /// Baseline value.
    pub baseline_value: f64,
    /// Current (observed) value.
    pub current_value: f64,
    /// Fractional change (positive = worsened, negative = improved).
    /// For pass rate: `(baseline - current) / baseline`.
    /// For cost/duration/iterations: `(current - baseline) / baseline`.
    pub change_fraction: f64,
    /// The threshold that was breached.
    pub threshold: f64,
    /// Human-readable description of the regression.
    pub description: String,
    /// Optional (role, complexity) slice this applies to.
    /// `None` means overall.
    pub slice: Option<(String, String)>,
}

/// Result of a regression check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegressionReport {
    /// List of all alerts (both breaches and improvements).
    pub alerts: Vec<RegressionAlert>,
    /// Whether any alert has severity `Alert`.
    pub has_regressions: bool,
    /// Whether the current data set has enough records.
    pub sufficient_data: bool,
    /// Number of current records analyzed.
    pub current_records: usize,
    /// Number of baseline records.
    pub baseline_records: usize,
}

impl RegressionReport {
    /// Return only the alerts with `Alert` severity.
    pub fn regressions(&self) -> Vec<&RegressionAlert> {
        self.alerts
            .iter()
            .filter(|a| a.severity == AlertSeverity::Alert)
            .collect()
    }

    /// Return only the alerts with `Warning` severity.
    pub fn warnings(&self) -> Vec<&RegressionAlert> {
        self.alerts
            .iter()
            .filter(|a| a.severity == AlertSeverity::Warning)
            .collect()
    }

    /// Return only the improvements.
    pub fn improvements(&self) -> Vec<&RegressionAlert> {
        self.alerts
            .iter()
            .filter(|a| a.severity == AlertSeverity::Improvement)
            .collect()
    }
}

// ─── Detection ──────────────────────────────────────────────────────────────

/// Compare `current` metrics against `baseline` and detect regressions.
///
/// Produces a [`RegressionReport`] with alerts for each breached threshold.
/// If either dataset is below the `min_records` threshold, the report has
/// `sufficient_data = false` and no alerts.
#[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
pub fn detect_regressions(
    baseline: &Baseline,
    current: &[TaskMetric],
    thresholds: &RegressionThresholds,
) -> RegressionReport {
    let mut alerts = Vec::new();
    let sufficient =
        baseline.total_records >= thresholds.min_records && current.len() >= thresholds.min_records;

    if !sufficient {
        return RegressionReport {
            alerts,
            has_regressions: false,
            sufficient_data: false,
            current_records: current.len(),
            baseline_records: baseline.total_records,
        };
    }

    let current_baseline = compute_baseline(current, thresholds.min_records);

    // ── Overall pass rate ───────────────────────────────────────────
    if baseline.overall_pass_rate > 0.0 {
        let drop = (baseline.overall_pass_rate - current_baseline.overall_pass_rate)
            / baseline.overall_pass_rate;

        if drop > thresholds.pass_rate_drop {
            alerts.push(RegressionAlert {
                metric_name: "pass_rate".into(),
                severity: AlertSeverity::Alert,
                baseline_value: baseline.overall_pass_rate,
                current_value: current_baseline.overall_pass_rate,
                change_fraction: drop,
                threshold: thresholds.pass_rate_drop,
                description: format!(
                    "First-attempt pass rate dropped {:.1}% (baseline {:.1}% -> current {:.1}%)",
                    drop * 100.0,
                    baseline.overall_pass_rate * 100.0,
                    current_baseline.overall_pass_rate * 100.0,
                ),
                slice: None,
            });
        } else if drop < -0.01 {
            // Improvement
            alerts.push(RegressionAlert {
                metric_name: "pass_rate".into(),
                severity: AlertSeverity::Improvement,
                baseline_value: baseline.overall_pass_rate,
                current_value: current_baseline.overall_pass_rate,
                change_fraction: drop,
                threshold: thresholds.pass_rate_drop,
                description: format!(
                    "First-attempt pass rate improved {:.1}% (baseline {:.1}% -> current {:.1}%)",
                    (-drop) * 100.0,
                    baseline.overall_pass_rate * 100.0,
                    current_baseline.overall_pass_rate * 100.0,
                ),
                slice: None,
            });
        }
    }

    // ── Overall cost ────────────────────────────────────────────────
    if baseline.overall_avg_cost > 0.0 {
        let increase = (current_baseline.overall_avg_cost - baseline.overall_avg_cost)
            / baseline.overall_avg_cost;

        if increase > thresholds.cost_increase {
            alerts.push(RegressionAlert {
                metric_name: "cost".into(),
                severity: AlertSeverity::Alert,
                baseline_value: baseline.overall_avg_cost,
                current_value: current_baseline.overall_avg_cost,
                change_fraction: increase,
                threshold: thresholds.cost_increase,
                description: format!(
                    "Average cost increased {:.1}% (baseline ${:.4} -> current ${:.4})",
                    increase * 100.0,
                    baseline.overall_avg_cost,
                    current_baseline.overall_avg_cost,
                ),
                slice: None,
            });
        } else if increase < -0.01 {
            alerts.push(RegressionAlert {
                metric_name: "cost".into(),
                severity: AlertSeverity::Improvement,
                baseline_value: baseline.overall_avg_cost,
                current_value: current_baseline.overall_avg_cost,
                change_fraction: increase,
                threshold: thresholds.cost_increase,
                description: format!(
                    "Average cost decreased {:.1}% (baseline ${:.4} -> current ${:.4})",
                    (-increase) * 100.0,
                    baseline.overall_avg_cost,
                    current_baseline.overall_avg_cost,
                ),
                slice: None,
            });
        }
    }

    // ── Overall duration ────────────────────────────────────────────
    if baseline.overall_avg_duration_ms > 0.0 {
        let increase = (current_baseline.overall_avg_duration_ms
            - baseline.overall_avg_duration_ms)
            / baseline.overall_avg_duration_ms;

        if increase > thresholds.duration_increase {
            alerts.push(RegressionAlert {
                metric_name: "duration".into(),
                severity: AlertSeverity::Warning,
                baseline_value: baseline.overall_avg_duration_ms,
                current_value: current_baseline.overall_avg_duration_ms,
                change_fraction: increase,
                threshold: thresholds.duration_increase,
                description: format!(
                    "Average duration increased {:.1}% (baseline {:.0}ms -> current {:.0}ms)",
                    increase * 100.0,
                    baseline.overall_avg_duration_ms,
                    current_baseline.overall_avg_duration_ms,
                ),
                slice: None,
            });
        }
    }

    let has_regressions = alerts.iter().any(|a| a.severity == AlertSeverity::Alert);

    RegressionReport {
        alerts,
        has_regressions,
        sufficient_data: true,
        current_records: current.len(),
        baseline_records: baseline.total_records,
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_metric::make_rich_metric;

    fn baseline_records() -> Vec<TaskMetric> {
        vec![
            make_rich_metric(
                "p1", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p2", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p3", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p4", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p5", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 10000,
            ),
        ]
    }

    #[test]
    fn regression_no_regression_on_stable_metrics() {
        let base = baseline_records();
        let b = compute_baseline(&base, 5);
        // Current records with same characteristics.
        let current = baseline_records();
        let thresholds = RegressionThresholds::default();
        let report = detect_regressions(&b, &current, &thresholds);

        assert!(report.sufficient_data);
        assert!(!report.has_regressions);
        assert!(report.regressions().is_empty());
    }

    #[test]
    fn regression_pass_rate_drop_fires_alert() {
        let base = baseline_records(); // 100% pass rate
        let b = compute_baseline(&base, 5);

        // Current: only 2/5 pass = 40% pass rate → 60% drop > 15% threshold
        let current = vec![
            make_rich_metric(
                "p1", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p2", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p3", "t1", "Impl", "s", "std", "compile", false, 1, 0.50, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p4", "t1", "Impl", "s", "std", "compile", false, 1, 0.50, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p5", "t1", "Impl", "s", "std", "compile", false, 1, 0.50, 1000, 200, 10000,
            ),
        ];

        let report = detect_regressions(&b, &current, &RegressionThresholds::default());
        assert!(report.has_regressions);
        let regs = report.regressions();
        assert!(!regs.is_empty());
        assert_eq!(regs[0].metric_name, "pass_rate");
    }

    #[test]
    fn regression_cost_increase_fires_alert() {
        let base = baseline_records(); // avg cost $0.50
        let b = compute_baseline(&base, 5);

        // Current: avg cost $0.80 → 60% increase > 20% threshold
        let current = vec![
            make_rich_metric(
                "p1", "t1", "Impl", "s", "std", "compile", true, 1, 0.80, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p2", "t1", "Impl", "s", "std", "compile", true, 1, 0.80, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p3", "t1", "Impl", "s", "std", "compile", true, 1, 0.80, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p4", "t1", "Impl", "s", "std", "compile", true, 1, 0.80, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p5", "t1", "Impl", "s", "std", "compile", true, 1, 0.80, 1000, 200, 10000,
            ),
        ];

        let report = detect_regressions(&b, &current, &RegressionThresholds::default());
        assert!(report.has_regressions);
        let regs = report.regressions();
        assert!(regs.iter().any(|a| a.metric_name == "cost"));
    }

    #[test]
    fn regression_duration_increase_fires_warning() {
        let base = baseline_records(); // avg duration 10000ms
        let b = compute_baseline(&base, 5);

        // Current: avg duration 15000ms → 50% increase > 30% threshold
        let current = vec![
            make_rich_metric(
                "p1", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 15000,
            ),
            make_rich_metric(
                "p2", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 15000,
            ),
            make_rich_metric(
                "p3", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 15000,
            ),
            make_rich_metric(
                "p4", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 15000,
            ),
            make_rich_metric(
                "p5", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 15000,
            ),
        ];

        let report = detect_regressions(&b, &current, &RegressionThresholds::default());
        let warnings = report.warnings();
        assert!(!warnings.is_empty());
        assert!(warnings.iter().any(|a| a.metric_name == "duration"));
    }

    #[test]
    fn regression_insufficient_data_skips_detection() {
        let base = vec![make_rich_metric(
            "p1", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 10000,
        )];
        let b = compute_baseline(&base, 5);

        let current = vec![make_rich_metric(
            "p1", "t1", "Impl", "s", "std", "compile", false, 1, 0.50, 1000, 200, 10000,
        )];

        let report = detect_regressions(&b, &current, &RegressionThresholds::default());
        assert!(!report.sufficient_data);
        assert!(!report.has_regressions);
        assert!(report.alerts.is_empty());
    }

    #[test]
    fn regression_custom_thresholds() {
        let base = baseline_records();
        let b = compute_baseline(&base, 5);

        // 4/5 pass = 80% → 20% drop from 100%
        let current = vec![
            make_rich_metric(
                "p1", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p2", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p3", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p4", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p5", "t1", "Impl", "s", "std", "compile", false, 1, 0.50, 1000, 200, 10000,
            ),
        ];

        // With strict threshold (10%), 20% drop should fire.
        let strict = RegressionThresholds {
            pass_rate_drop: 0.10,
            ..RegressionThresholds::default()
        };
        let report = detect_regressions(&b, &current, &strict);
        assert!(report.has_regressions);

        // With lenient threshold (25%), 20% drop should NOT fire.
        let lenient = RegressionThresholds {
            pass_rate_drop: 0.25,
            ..RegressionThresholds::default()
        };
        let report = detect_regressions(&b, &current, &lenient);
        assert!(!report.has_regressions);
    }

    #[test]
    fn regression_improvement_detected() {
        // Baseline with 60% pass rate
        let base = vec![
            make_rich_metric(
                "p1", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p2", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p3", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p4", "t1", "Impl", "s", "std", "compile", false, 1, 0.50, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p5", "t1", "Impl", "s", "std", "compile", false, 1, 0.50, 1000, 200, 10000,
            ),
        ];
        let b = compute_baseline(&base, 5);

        // Current: 100% pass rate + lower cost → improvements
        let current = vec![
            make_rich_metric(
                "p1", "t1", "Impl", "s", "std", "compile", true, 1, 0.30, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p2", "t1", "Impl", "s", "std", "compile", true, 1, 0.30, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p3", "t1", "Impl", "s", "std", "compile", true, 1, 0.30, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p4", "t1", "Impl", "s", "std", "compile", true, 1, 0.30, 1000, 200, 10000,
            ),
            make_rich_metric(
                "p5", "t1", "Impl", "s", "std", "compile", true, 1, 0.30, 1000, 200, 10000,
            ),
        ];

        let report = detect_regressions(&b, &current, &RegressionThresholds::default());
        assert!(!report.has_regressions);
        let improvements = report.improvements();
        assert!(!improvements.is_empty());
    }

    #[test]
    fn regression_report_accessors() {
        let base = baseline_records();
        let b = compute_baseline(&base, 5);
        let current = baseline_records();
        let report = detect_regressions(&b, &current, &RegressionThresholds::default());

        assert_eq!(report.current_records, 5);
        assert_eq!(report.baseline_records, 5);
        assert!(report.regressions().is_empty());
        assert!(report.warnings().is_empty());
    }

    #[test]
    fn regression_thresholds_serialization() {
        let t = RegressionThresholds::default();
        let json = serde_json::to_string(&t).expect("serialize");
        let t2: RegressionThresholds = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(t, t2);
    }
}
