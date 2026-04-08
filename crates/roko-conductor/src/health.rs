//! Health monitors: track overall system health via composable checks.
//!
//! The [`HealthMonitor`] runs a set of [`HealthCheck`] functions and
//! reports the overall system status. Each check is a pure function
//! that examines provided state and returns a status.
//!
//! # Usage
//!
//! ```rust
//! use roko_conductor::health::{HealthMonitor, HealthStatus, HealthCheckResult};
//!
//! let monitor = HealthMonitor::default();
//! let snapshot = roko_conductor::health::SystemSnapshot::default();
//! let checks = monitor.check_all(&snapshot);
//! let status = monitor.overall_status(&snapshot);
//! assert_eq!(status, HealthStatus::Healthy);
//! ```

use serde::{Deserialize, Serialize};

// ---- HealthStatus -----------------------------------------------------------

/// Overall health level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    /// All checks pass.
    Healthy = 0,
    /// Some checks show warnings; system may still function.
    Degraded = 1,
    /// Critical checks failed; system needs intervention.
    Critical = 2,
}

// ---- HealthCheckResult ------------------------------------------------------

/// Result of a single health check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthCheckResult {
    /// Name of the check.
    pub name: String,
    /// Status of this check.
    pub status: HealthStatus,
    /// Human-readable message describing the finding.
    pub message: String,
    /// Unix milliseconds when this check was performed.
    pub checked_at_ms: i64,
}

impl HealthCheckResult {
    /// Create a healthy check result.
    #[must_use]
    pub fn healthy(name: impl Into<String>, message: impl Into<String>, at_ms: i64) -> Self {
        Self {
            name: name.into(),
            status: HealthStatus::Healthy,
            message: message.into(),
            checked_at_ms: at_ms,
        }
    }

    /// Create a degraded check result.
    #[must_use]
    pub fn degraded(name: impl Into<String>, message: impl Into<String>, at_ms: i64) -> Self {
        Self {
            name: name.into(),
            status: HealthStatus::Degraded,
            message: message.into(),
            checked_at_ms: at_ms,
        }
    }

    /// Create a critical check result.
    #[must_use]
    pub fn critical(name: impl Into<String>, message: impl Into<String>, at_ms: i64) -> Self {
        Self {
            name: name.into(),
            status: HealthStatus::Critical,
            message: message.into(),
            checked_at_ms: at_ms,
        }
    }
}

// ---- SystemSnapshot ---------------------------------------------------------

/// A snapshot of system state that health checks evaluate against.
///
/// This is a plain data struct populated by the orchestrator before
/// passing to the health monitor. Health checks are pure functions
/// over this data.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemSnapshot {
    /// Number of active agent processes.
    pub active_agents: u32,
    /// Number of expected agent processes (based on plan).
    pub expected_agents: u32,
    /// Unix milliseconds of the last agent heartbeat (0 = never).
    pub last_agent_heartbeat_ms: i64,
    /// Whether the chain connection is alive.
    pub chain_connected: bool,
    /// Whether any chain connection was expected.
    pub chain_expected: bool,
    /// Hash of the plan spec when agents started.
    pub spec_hash_at_start: String,
    /// Hash of the current plan spec on disk.
    pub spec_hash_current: String,
    /// Recent test coverage percentages (newest last). Empty = no data.
    pub coverage_history: Vec<f64>,
    /// Current timestamp in unix milliseconds.
    pub now_ms: i64,
    /// Heartbeat staleness threshold in milliseconds (default: `60_000`).
    pub heartbeat_timeout_ms: i64,
}

// ---- HealthMonitor ----------------------------------------------------------

/// The health monitor: runs a set of checks against a [`SystemSnapshot`].
///
/// Thread-safe and immutable. All checks are pure functions.
#[derive(Debug, Clone)]
pub struct HealthMonitor {
    /// Which checks to run. Each is a named function.
    checks: Vec<NamedCheck>,
}

/// A named health check function.
#[derive(Clone)]
struct NamedCheck {
    name: &'static str,
    check_fn: fn(&SystemSnapshot) -> HealthCheckResult,
}

impl std::fmt::Debug for NamedCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NamedCheck")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Default for HealthMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl HealthMonitor {
    /// Create a monitor with the built-in checks.
    #[must_use]
    pub fn new() -> Self {
        Self {
            checks: vec![
                NamedCheck {
                    name: "terminal_liveness",
                    check_fn: check_terminal_liveness,
                },
                NamedCheck {
                    name: "golem_status",
                    check_fn: check_golem_status,
                },
                NamedCheck {
                    name: "spec_drift",
                    check_fn: check_spec_drift,
                },
                NamedCheck {
                    name: "coverage_trend",
                    check_fn: check_coverage_trend,
                },
            ],
        }
    }

    /// Number of registered checks.
    #[must_use]
    pub fn check_count(&self) -> usize {
        self.checks.len()
    }

    /// Run all checks and return results.
    #[must_use]
    pub fn check_all(&self, snapshot: &SystemSnapshot) -> Vec<HealthCheckResult> {
        self.checks.iter().map(|c| (c.check_fn)(snapshot)).collect()
    }

    /// Compute the overall status (worst of all checks).
    #[must_use]
    pub fn overall_status(&self, snapshot: &SystemSnapshot) -> HealthStatus {
        self.check_all(snapshot)
            .iter()
            .map(|c| c.status)
            .max()
            .unwrap_or(HealthStatus::Healthy)
    }
}

// ---- Built-in checks --------------------------------------------------------

/// Check that agent processes are alive and responsive.
#[allow(clippy::cast_precision_loss)]
fn check_terminal_liveness(snapshot: &SystemSnapshot) -> HealthCheckResult {
    let now = snapshot.now_ms;

    // If no agents are expected, skip.
    if snapshot.expected_agents == 0 {
        return HealthCheckResult::healthy("terminal_liveness", "no agents expected", now);
    }

    // Check if active agents match expected.
    if snapshot.active_agents == 0 && snapshot.expected_agents > 0 {
        return HealthCheckResult::critical(
            "terminal_liveness",
            format!("0/{} expected agents active", snapshot.expected_agents),
            now,
        );
    }

    if snapshot.active_agents < snapshot.expected_agents {
        return HealthCheckResult::degraded(
            "terminal_liveness",
            format!(
                "{}/{} expected agents active",
                snapshot.active_agents, snapshot.expected_agents
            ),
            now,
        );
    }

    // Check heartbeat staleness.
    let timeout = if snapshot.heartbeat_timeout_ms > 0 {
        snapshot.heartbeat_timeout_ms
    } else {
        60_000
    };

    if snapshot.last_agent_heartbeat_ms > 0 {
        let staleness = now - snapshot.last_agent_heartbeat_ms;
        if staleness > timeout {
            return HealthCheckResult::degraded(
                "terminal_liveness",
                format!("agent heartbeat stale by {:.0}s", staleness as f64 / 1000.0),
                now,
            );
        }
    }

    HealthCheckResult::healthy(
        "terminal_liveness",
        format!(
            "{}/{} agents active",
            snapshot.active_agents, snapshot.expected_agents
        ),
        now,
    )
}

/// Check chain / golem connection status.
fn check_golem_status(snapshot: &SystemSnapshot) -> HealthCheckResult {
    let now = snapshot.now_ms;

    if !snapshot.chain_expected {
        return HealthCheckResult::healthy("golem_status", "chain not required", now);
    }

    if snapshot.chain_connected {
        HealthCheckResult::healthy("golem_status", "chain connected", now)
    } else {
        HealthCheckResult::degraded("golem_status", "chain connection lost", now)
    }
}

/// Check if the plan spec has drifted since agents started.
fn check_spec_drift(snapshot: &SystemSnapshot) -> HealthCheckResult {
    let now = snapshot.now_ms;

    if snapshot.spec_hash_at_start.is_empty() || snapshot.spec_hash_current.is_empty() {
        return HealthCheckResult::healthy("spec_drift", "no spec tracking", now);
    }

    if snapshot.spec_hash_at_start == snapshot.spec_hash_current {
        HealthCheckResult::healthy("spec_drift", "spec unchanged", now)
    } else {
        HealthCheckResult::degraded(
            "spec_drift",
            "spec changed since agents started; agents may be working on stale requirements",
            now,
        )
    }
}

/// Check test coverage trend direction.
fn check_coverage_trend(snapshot: &SystemSnapshot) -> HealthCheckResult {
    let now = snapshot.now_ms;

    if snapshot.coverage_history.len() < 2 {
        return HealthCheckResult::healthy(
            "coverage_trend",
            "insufficient data for trend analysis",
            now,
        );
    }

    let len = snapshot.coverage_history.len();
    let recent = snapshot.coverage_history[len - 1];
    let previous = snapshot.coverage_history[len - 2];
    let delta = recent - previous;

    if delta < -5.0 {
        HealthCheckResult::critical(
            "coverage_trend",
            format!("coverage dropped sharply: {previous:.1}% -> {recent:.1}% ({delta:+.1}%)"),
            now,
        )
    } else if delta < -1.0 {
        HealthCheckResult::degraded(
            "coverage_trend",
            format!("coverage declining: {previous:.1}% -> {recent:.1}% ({delta:+.1}%)"),
            now,
        )
    } else {
        HealthCheckResult::healthy(
            "coverage_trend",
            format!("coverage stable/improving: {previous:.1}% -> {recent:.1}% ({delta:+.1}%)"),
            now,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor() -> HealthMonitor {
        HealthMonitor::default()
    }

    fn healthy_snapshot() -> SystemSnapshot {
        SystemSnapshot {
            active_agents: 2,
            expected_agents: 2,
            last_agent_heartbeat_ms: 99_000,
            chain_connected: true,
            chain_expected: true,
            spec_hash_at_start: "abc123".into(),
            spec_hash_current: "abc123".into(),
            coverage_history: vec![80.0, 82.0],
            now_ms: 100_000,
            heartbeat_timeout_ms: 60_000,
        }
    }

    // ---- Overall status ----

    #[test]
    fn healthy_snapshot_is_healthy() {
        let status = monitor().overall_status(&healthy_snapshot());
        assert_eq!(status, HealthStatus::Healthy);
    }

    #[test]
    fn default_snapshot_is_healthy() {
        // Default snapshot has no agents expected, no chain, no spec -> all healthy.
        let status = monitor().overall_status(&SystemSnapshot::default());
        assert_eq!(status, HealthStatus::Healthy);
    }

    // ---- Terminal liveness ----

    #[test]
    fn no_agents_when_expected_is_critical() {
        let snap = SystemSnapshot {
            active_agents: 0,
            expected_agents: 3,
            now_ms: 100_000,
            ..SystemSnapshot::default()
        };
        let result = check_terminal_liveness(&snap);
        assert_eq!(result.status, HealthStatus::Critical);
        assert!(result.message.contains("0/3"));
    }

    #[test]
    fn partial_agents_is_degraded() {
        let snap = SystemSnapshot {
            active_agents: 1,
            expected_agents: 3,
            now_ms: 100_000,
            ..SystemSnapshot::default()
        };
        let result = check_terminal_liveness(&snap);
        assert_eq!(result.status, HealthStatus::Degraded);
    }

    #[test]
    fn stale_heartbeat_is_degraded() {
        let snap = SystemSnapshot {
            active_agents: 2,
            expected_agents: 2,
            last_agent_heartbeat_ms: 10_000,
            now_ms: 100_000,
            heartbeat_timeout_ms: 60_000,
            ..SystemSnapshot::default()
        };
        let result = check_terminal_liveness(&snap);
        assert_eq!(result.status, HealthStatus::Degraded);
        assert!(result.message.contains("stale"));
    }

    // ---- Golem status ----

    #[test]
    fn chain_disconnected_is_degraded() {
        let snap = SystemSnapshot {
            chain_connected: false,
            chain_expected: true,
            now_ms: 100_000,
            ..SystemSnapshot::default()
        };
        let result = check_golem_status(&snap);
        assert_eq!(result.status, HealthStatus::Degraded);
    }

    #[test]
    fn chain_not_expected_is_healthy() {
        let snap = SystemSnapshot {
            chain_connected: false,
            chain_expected: false,
            now_ms: 100_000,
            ..SystemSnapshot::default()
        };
        let result = check_golem_status(&snap);
        assert_eq!(result.status, HealthStatus::Healthy);
    }

    // ---- Spec drift ----

    #[test]
    fn spec_drift_detected() {
        let snap = SystemSnapshot {
            spec_hash_at_start: "abc".into(),
            spec_hash_current: "def".into(),
            now_ms: 100_000,
            ..SystemSnapshot::default()
        };
        let result = check_spec_drift(&snap);
        assert_eq!(result.status, HealthStatus::Degraded);
        assert!(result.message.contains("changed"));
    }

    #[test]
    fn spec_unchanged_is_healthy() {
        let snap = SystemSnapshot {
            spec_hash_at_start: "abc".into(),
            spec_hash_current: "abc".into(),
            now_ms: 100_000,
            ..SystemSnapshot::default()
        };
        let result = check_spec_drift(&snap);
        assert_eq!(result.status, HealthStatus::Healthy);
    }

    // ---- Coverage trend ----

    #[test]
    fn coverage_drop_is_critical() {
        let snap = SystemSnapshot {
            coverage_history: vec![80.0, 72.0],
            now_ms: 100_000,
            ..SystemSnapshot::default()
        };
        let result = check_coverage_trend(&snap);
        assert_eq!(result.status, HealthStatus::Critical);
        assert!(result.message.contains("dropped"));
    }

    #[test]
    fn coverage_slight_decline_is_degraded() {
        let snap = SystemSnapshot {
            coverage_history: vec![80.0, 77.5],
            now_ms: 100_000,
            ..SystemSnapshot::default()
        };
        let result = check_coverage_trend(&snap);
        assert_eq!(result.status, HealthStatus::Degraded);
    }

    #[test]
    fn coverage_improving_is_healthy() {
        let snap = SystemSnapshot {
            coverage_history: vec![80.0, 85.0],
            now_ms: 100_000,
            ..SystemSnapshot::default()
        };
        let result = check_coverage_trend(&snap);
        assert_eq!(result.status, HealthStatus::Healthy);
    }

    #[test]
    fn coverage_insufficient_data_is_healthy() {
        let snap = SystemSnapshot {
            coverage_history: vec![80.0],
            now_ms: 100_000,
            ..SystemSnapshot::default()
        };
        let result = check_coverage_trend(&snap);
        assert_eq!(result.status, HealthStatus::Healthy);
    }

    // ---- check_all ----

    #[test]
    fn check_all_returns_all_checks() {
        let m = monitor();
        let results = m.check_all(&healthy_snapshot());
        assert_eq!(results.len(), m.check_count());
        // All should be healthy.
        for r in &results {
            assert_eq!(r.status, HealthStatus::Healthy, "check {} failed", r.name);
        }
    }

    // ---- Worst wins ----

    #[test]
    fn overall_status_picks_worst() {
        let snap = SystemSnapshot {
            active_agents: 0,
            expected_agents: 2,
            chain_connected: true,
            chain_expected: true,
            spec_hash_at_start: "abc".into(),
            spec_hash_current: "abc".into(),
            coverage_history: vec![80.0, 82.0],
            now_ms: 100_000,
            ..SystemSnapshot::default()
        };
        // Terminal liveness is critical, rest are healthy.
        let status = monitor().overall_status(&snap);
        assert_eq!(status, HealthStatus::Critical);
    }

    // ---- HealthStatus ordering ----

    #[test]
    fn health_status_ordering() {
        assert!(HealthStatus::Healthy < HealthStatus::Degraded);
        assert!(HealthStatus::Degraded < HealthStatus::Critical);
    }

    // ---- Serde ----

    #[test]
    fn health_check_result_serde_roundtrip() {
        let result = HealthCheckResult::degraded("test", "msg", 42_000);
        let json = serde_json::to_string(&result).expect("serialize");
        let decoded: HealthCheckResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.name, "test");
        assert_eq!(decoded.status, HealthStatus::Degraded);
    }

    #[test]
    fn system_snapshot_serde_roundtrip() {
        let snap = healthy_snapshot();
        let json = serde_json::to_string(&snap).expect("serialize");
        let decoded: SystemSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.active_agents, snap.active_agents);
    }
}
