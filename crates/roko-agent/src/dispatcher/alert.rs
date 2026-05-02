//! Failure-rate alerting for tool invocations (checklist items 36.64-36.66).
//!
//! [`ToolFailureMonitor`] tracks per-tool success/failure counts and fires
//! [`ToolAlert`]s when any tool exceeds a configurable failure-rate
//! threshold over a minimum sample count.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Default failure-rate threshold (25%).
const DEFAULT_FAILURE_THRESHOLD: f64 = roko_core::defaults::DEFAULT_FAILURE_THRESHOLD;

/// Minimum number of calls before alerts can fire.
const DEFAULT_MIN_CALLS: u64 = roko_core::defaults::DEFAULT_ALERT_MIN_CALLS as u64;

// ─── AlertSeverity ───────────────────────────────────────────────────────

/// Severity level for a tool-health alert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    /// Failure rate exceeds threshold but is under 50%.
    Warning,
    /// Failure rate is 50% or higher.
    Critical,
}

// ─── ToolAlert ───────────────────────────────────────────────────────────

/// An alert fired when a tool's failure rate exceeds the configured threshold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolAlert {
    /// Canonical tool name that triggered the alert.
    pub tool_name: String,
    /// Observed failure rate in `[0, 1]`.
    pub failure_rate: f64,
    /// Total calls observed for this tool.
    pub total_calls: u64,
    /// Severity classification.
    pub severity: AlertSeverity,
}

// ─── ToolStats ───────────────────────────────────────────────────────────

/// Per-tool success/failure counters.
#[derive(Debug, Clone, Default)]
struct ToolStats {
    success: u64,
    failure: u64,
}

impl ToolStats {
    const fn total(&self) -> u64 {
        self.success + self.failure
    }

    #[allow(clippy::cast_precision_loss)]
    fn failure_rate(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            return 0.0;
        }
        self.failure as f64 / total as f64
    }
}

// ─── ToolFailureMonitor ──────────────────────────────────────────────────

/// Monitors per-tool failure rates and emits alerts when thresholds are
/// exceeded.
///
/// # Usage
///
/// ```
/// use roko_agent::dispatcher::alert::ToolFailureMonitor;
///
/// let mut monitor = ToolFailureMonitor::new();
/// for _ in 0..40 {
///     monitor.record("bash", true);
/// }
/// for _ in 0..20 {
///     monitor.record("bash", false);
/// }
/// let alerts = monitor.check_alerts();
/// assert_eq!(alerts.len(), 1);
/// assert_eq!(alerts[0].tool_name, "bash");
/// ```
#[derive(Debug, Clone)]
pub struct ToolFailureMonitor {
    stats: HashMap<String, ToolStats>,
    failure_threshold: f64,
    min_calls: u64,
}

impl Default for ToolFailureMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolFailureMonitor {
    /// Create a monitor with default thresholds (25% failure rate, 50 min calls).
    #[must_use]
    pub fn new() -> Self {
        Self {
            stats: HashMap::new(),
            failure_threshold: DEFAULT_FAILURE_THRESHOLD,
            min_calls: DEFAULT_MIN_CALLS,
        }
    }

    /// Create a monitor with custom thresholds.
    ///
    /// `failure_threshold` is clamped to `[0, 1]`.
    #[must_use]
    pub fn with_thresholds(failure_threshold: f64, min_calls: u64) -> Self {
        Self {
            stats: HashMap::new(),
            failure_threshold: failure_threshold.clamp(0.0, 1.0),
            min_calls,
        }
    }

    /// Record a tool invocation outcome.
    pub fn record(&mut self, tool_name: &str, success: bool) {
        let stats = self.stats.entry(tool_name.to_owned()).or_default();
        if success {
            stats.success += 1;
        } else {
            stats.failure += 1;
        }
    }

    /// Check all tracked tools and return alerts for those exceeding the
    /// failure-rate threshold with at least `min_calls` observations.
    #[must_use]
    pub fn check_alerts(&self) -> Vec<ToolAlert> {
        let mut alerts = Vec::new();
        for (name, stats) in &self.stats {
            if stats.total() < self.min_calls {
                continue;
            }
            let rate = stats.failure_rate();
            if rate > self.failure_threshold {
                let severity = if rate >= 0.5 {
                    AlertSeverity::Critical
                } else {
                    AlertSeverity::Warning
                };
                alerts.push(ToolAlert {
                    tool_name: name.clone(),
                    failure_rate: rate,
                    total_calls: stats.total(),
                    severity,
                });
            }
        }
        // Sort by failure rate descending for deterministic output.
        alerts.sort_by(|a, b| {
            b.failure_rate
                .partial_cmp(&a.failure_rate)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        alerts
    }

    /// Reset all tracked statistics.
    pub fn clear(&mut self) {
        self.stats.clear();
    }

    /// Return the failure rate for a specific tool, or `None` if untracked.
    #[must_use]
    pub fn failure_rate(&self, tool_name: &str) -> Option<f64> {
        self.stats.get(tool_name).map(ToolStats::failure_rate)
    }

    /// Return the total call count for a specific tool, or 0 if untracked.
    #[must_use]
    pub fn total_calls(&self, tool_name: &str) -> u64 {
        self.stats.get(tool_name).map_or(0, ToolStats::total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alert_no_alerts_below_min_calls() {
        let mut monitor = ToolFailureMonitor::new();
        // Record 49 calls (below threshold of 50), all failures.
        for _ in 0..49 {
            monitor.record("bash", false);
        }
        let alerts = monitor.check_alerts();
        assert!(alerts.is_empty(), "should not alert below min_calls");
    }

    #[test]
    fn alert_fires_at_threshold() {
        let mut monitor = ToolFailureMonitor::new();
        // 37 successes + 13 failures = 50 calls, failure rate = 26% > 25%
        for _ in 0..37 {
            monitor.record("bash", true);
        }
        for _ in 0..13 {
            monitor.record("bash", false);
        }
        let alerts = monitor.check_alerts();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].tool_name, "bash");
        assert!((alerts[0].failure_rate - 0.26).abs() < 0.01);
        assert_eq!(alerts[0].total_calls, 50);
        assert_eq!(alerts[0].severity, AlertSeverity::Warning);
    }

    #[test]
    fn alert_no_alert_at_or_below_threshold() {
        let mut monitor = ToolFailureMonitor::new();
        // 38 successes + 12 failures = 50 calls, rate = 24% < 25%
        for _ in 0..38 {
            monitor.record("read_file", true);
        }
        for _ in 0..12 {
            monitor.record("read_file", false);
        }
        let alerts = monitor.check_alerts();
        assert!(alerts.is_empty(), "24% should not trigger at 25% threshold");
    }

    #[test]
    fn alert_critical_severity_above_50_percent() {
        let mut monitor = ToolFailureMonitor::new();
        // 20 successes + 30 failures = 50 calls, rate = 60%
        for _ in 0..20 {
            monitor.record("bash", true);
        }
        for _ in 0..30 {
            monitor.record("bash", false);
        }
        let alerts = monitor.check_alerts();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].severity, AlertSeverity::Critical);
        assert!((alerts[0].failure_rate - 0.6).abs() < 0.01);
    }

    #[test]
    fn alert_multiple_tools() {
        let mut monitor = ToolFailureMonitor::new();
        // Tool A: 30/50 = 60% failure → critical
        for _ in 0..20 {
            monitor.record("tool_a", true);
        }
        for _ in 0..30 {
            monitor.record("tool_a", false);
        }
        // Tool B: 15/50 = 30% failure → warning
        for _ in 0..35 {
            monitor.record("tool_b", true);
        }
        for _ in 0..15 {
            monitor.record("tool_b", false);
        }
        // Tool C: healthy (5% failure)
        for _ in 0..95 {
            monitor.record("tool_c", true);
        }
        for _ in 0..5 {
            monitor.record("tool_c", false);
        }
        let alerts = monitor.check_alerts();
        assert_eq!(alerts.len(), 2, "only tool_a and tool_b should alert");
        // Sorted by failure_rate descending:
        assert_eq!(alerts[0].tool_name, "tool_a");
        assert_eq!(alerts[1].tool_name, "tool_b");
    }

    #[test]
    fn alert_custom_thresholds() {
        let mut monitor = ToolFailureMonitor::with_thresholds(0.10, 20);
        // 15 successes + 5 failures = 20 calls, 25% failure > 10%
        for _ in 0..15 {
            monitor.record("grep", true);
        }
        for _ in 0..5 {
            monitor.record("grep", false);
        }
        let alerts = monitor.check_alerts();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].tool_name, "grep");
    }

    #[test]
    fn alert_clear_resets() {
        let mut monitor = ToolFailureMonitor::new();
        for _ in 0..50 {
            monitor.record("bash", false);
        }
        assert!(!monitor.check_alerts().is_empty());
        monitor.clear();
        assert!(monitor.check_alerts().is_empty());
    }

    #[test]
    fn alert_failure_rate_query() {
        let mut monitor = ToolFailureMonitor::new();
        for _ in 0..75 {
            monitor.record("bash", true);
        }
        for _ in 0..25 {
            monitor.record("bash", false);
        }
        let rate = monitor.failure_rate("bash").unwrap();
        assert!((rate - 0.25).abs() < 0.01);
        assert!(monitor.failure_rate("nonexistent").is_none());
    }

    #[test]
    fn alert_total_calls_query() {
        let mut monitor = ToolFailureMonitor::new();
        for _ in 0..10 {
            monitor.record("grep", true);
        }
        assert_eq!(monitor.total_calls("grep"), 10);
        assert_eq!(monitor.total_calls("nonexistent"), 0);
    }

    #[test]
    fn alert_serde_roundtrip() {
        let alert = ToolAlert {
            tool_name: "bash".into(),
            failure_rate: 0.35,
            total_calls: 100,
            severity: AlertSeverity::Warning,
        };
        let json = serde_json::to_string(&alert).unwrap();
        let decoded: ToolAlert = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, alert);
    }
}
