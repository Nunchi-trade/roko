//! Lightweight heartbeat scheduling and persistence for CLI orchestration.
//!
//! The heartbeat loop is deliberately zero-LLM: it inspects runtime state,
//! emits a periodic theta/delta snapshot, and persists the result under
//! `.roko/learn/` for dashboards, daemon lifecycle hooks, and post-mortem
//! debugging.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use roko_core::{
    OperatingFrequency, OperatingFrequencyScheduleContext, OperatingFrequencyScheduler,
};
use serde::{Deserialize, Serialize};

const DEFAULT_DELTA_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Stable zero-LLM heartbeat probe identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeartbeatProbeKind {
    /// Root shutdown token was tripped.
    ShutdownRequested,
    /// The watcher background task was cancelled.
    WatcherCancelled,
    /// No tasks are currently ready to run.
    NoReadyTasks,
    /// A ready task has been queued for too long.
    ReadyQueueStalled,
    /// Tasks are blocked on plan-level dependencies.
    CrossPlanBlocked,
    /// One or more trackers carry a recent gate failure.
    GateFailurePresent,
    /// One or more trackers accumulated repeated gate failures.
    RepeatedGateFailures,
    /// An explicit force-model override is armed.
    ForceModelOverrideArmed,
    /// The daimon confidence is low enough to justify a step-back.
    LowAffectConfidence,
    /// Active agents are still running.
    ActiveAgentsPresent,
    /// Health probes reported degraded readiness.
    HealthDegraded,
    /// Search enrichment is unavailable.
    SearchUnavailable,
    /// MCP clients were requested but none are active.
    McpUnavailable,
    /// The run has accumulated non-trivial spend.
    SessionSpendElevated,
    /// A theta reflection pass is due.
    ThetaDue,
    /// A delta consolidation pass is due.
    DeltaDue,
}

impl HeartbeatProbeKind {
    /// Stable machine label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ShutdownRequested => "shutdown_requested",
            Self::WatcherCancelled => "watcher_cancelled",
            Self::NoReadyTasks => "no_ready_tasks",
            Self::ReadyQueueStalled => "ready_queue_stalled",
            Self::CrossPlanBlocked => "cross_plan_blocked",
            Self::GateFailurePresent => "gate_failure_present",
            Self::RepeatedGateFailures => "repeated_gate_failures",
            Self::ForceModelOverrideArmed => "force_model_override_armed",
            Self::LowAffectConfidence => "low_affect_confidence",
            Self::ActiveAgentsPresent => "active_agents_present",
            Self::HealthDegraded => "health_degraded",
            Self::SearchUnavailable => "search_unavailable",
            Self::McpUnavailable => "mcp_unavailable",
            Self::SessionSpendElevated => "session_spend_elevated",
            Self::ThetaDue => "theta_due",
            Self::DeltaDue => "delta_due",
        }
    }
}

/// One zero-LLM probe result included in a heartbeat snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatProbeResult {
    /// Which probe ran.
    pub kind: HeartbeatProbeKind,
    /// Whether the probe fired.
    pub triggered: bool,
    /// Human-readable detail for logs and dashboards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl HeartbeatProbeResult {
    /// Construct a probe result.
    #[must_use]
    pub fn new(kind: HeartbeatProbeKind, triggered: bool, detail: Option<String>) -> Self {
        Self {
            kind,
            triggered,
            detail,
        }
    }
}

/// Persisted heartbeat snapshot for theta/delta cadence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeartbeatSnapshot {
    /// Wall-clock timestamp when the snapshot was captured.
    pub timestamp: DateTime<Utc>,
    /// Which operating frequency fired this heartbeat.
    pub frequency: OperatingFrequency,
    /// Number of unfinished tasks across all tracked plans.
    pub active_tasks: usize,
    /// Number of currently ready tasks.
    pub ready_tasks: usize,
    /// Number of completed tasks.
    pub completed_tasks: usize,
    /// Number of failed tasks.
    pub failed_tasks: usize,
    /// Aggregate completion rate in `[0.0, 1.0]`.
    pub completion_rate: f64,
    /// Number of active agent processes.
    pub active_agents: usize,
    /// Seconds since the last theta heartbeat.
    pub seconds_since_last_theta: u64,
    /// Configured delta interval in seconds.
    pub delta_interval_secs: u64,
    /// Which probes fired for this heartbeat.
    pub probes: Vec<HeartbeatProbeResult>,
}

impl HeartbeatSnapshot {
    /// Return stable labels for all triggered probes.
    #[must_use]
    pub fn triggered_probe_labels(&self) -> Vec<&'static str> {
        self.probes
            .iter()
            .filter(|probe| probe.triggered)
            .map(|probe| probe.kind.label())
            .collect()
    }
}

/// In-memory heartbeat scheduling state.
#[derive(Debug, Clone)]
pub struct HeartbeatClock {
    scheduler: OperatingFrequencyScheduler,
    delta_interval: Duration,
    last_theta_at: Instant,
    last_delta_at: Instant,
}

impl Default for HeartbeatClock {
    fn default() -> Self {
        Self::new()
    }
}

impl HeartbeatClock {
    /// Construct a heartbeat clock with the default adaptive theta scheduler.
    #[must_use]
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            scheduler: OperatingFrequencyScheduler::default(),
            delta_interval: DEFAULT_DELTA_INTERVAL,
            last_theta_at: now,
            last_delta_at: now,
        }
    }

    /// Override the delta cadence.
    #[must_use]
    pub fn with_delta_interval(mut self, delta_interval: Duration) -> Self {
        assert!(
            delta_interval > Duration::ZERO,
            "delta interval must be positive"
        );
        self.delta_interval = delta_interval;
        self
    }

    /// Seconds since the last theta heartbeat.
    #[must_use]
    pub fn seconds_since_last_theta(&self, now: Instant) -> u64 {
        now.duration_since(self.last_theta_at).as_secs()
    }

    /// Delta cadence in seconds.
    #[must_use]
    pub fn delta_interval_secs(&self) -> u64 {
        self.delta_interval.as_secs()
    }

    /// Return whether theta is due for the supplied context.
    #[must_use]
    pub fn theta_due(&self, now: Instant, mut context: OperatingFrequencyScheduleContext) -> bool {
        context.time_since_last_theta = now.duration_since(self.last_theta_at);
        self.scheduler.select(&context) == OperatingFrequency::Theta
    }

    /// Return whether delta is due for the supplied context.
    #[must_use]
    pub fn delta_due(&self, now: Instant, context: OperatingFrequencyScheduleContext) -> bool {
        let since_delta = now.duration_since(self.last_delta_at);
        context.is_idle() && since_delta >= self.delta_interval
    }

    /// Select the next due heartbeat, if any.
    #[must_use]
    pub fn next_due(
        &self,
        now: Instant,
        mut context: OperatingFrequencyScheduleContext,
    ) -> Option<OperatingFrequency> {
        if self.delta_due(now, context) {
            return Some(OperatingFrequency::Delta);
        }

        context.time_since_last_theta = now.duration_since(self.last_theta_at);
        match self.scheduler.select(&context) {
            OperatingFrequency::Theta => Some(OperatingFrequency::Theta),
            _ => None,
        }
    }

    /// Record that a heartbeat fired at `now`.
    pub fn record(&mut self, now: Instant, frequency: OperatingFrequency) {
        match frequency {
            OperatingFrequency::Gamma => {}
            OperatingFrequency::Theta => {
                self.last_theta_at = now;
            }
            OperatingFrequency::Delta => {
                self.last_theta_at = now;
                self.last_delta_at = now;
            }
        }
    }
}

/// Persist the latest heartbeat snapshot plus an append-only JSONL history.
pub fn persist_heartbeat_snapshot(workdir: &Path, snapshot: &HeartbeatSnapshot) -> Result<()> {
    let latest_path = heartbeat_snapshot_path(workdir);
    let history_path = heartbeat_history_path(workdir);
    if let Some(parent) = latest_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create heartbeat dir {}", parent.display()))?;
    }

    let json = serde_json::to_vec_pretty(snapshot).context("serialize heartbeat snapshot")?;
    let tmp_path = latest_path.with_extension("json.tmp");
    fs::write(&tmp_path, &json)
        .with_context(|| format!("write heartbeat tmp {}", tmp_path.display()))?;
    fs::rename(&tmp_path, &latest_path).with_context(|| {
        format!(
            "replace heartbeat snapshot {} with {}",
            latest_path.display(),
            tmp_path.display()
        )
    })?;

    let mut history = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&history_path)
        .with_context(|| format!("open heartbeat history {}", history_path.display()))?;
    let mut line = serde_json::to_string(snapshot).context("serialize heartbeat history line")?;
    line.push('\n');
    history
        .write_all(line.as_bytes())
        .context("append heartbeat history")?;
    history.flush().context("flush heartbeat history")?;
    Ok(())
}

/// Return the path for the latest heartbeat snapshot.
#[must_use]
pub fn heartbeat_snapshot_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("learn").join("heartbeat.json")
}

/// Return the path for the append-only heartbeat history.
#[must_use]
pub fn heartbeat_history_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("learn").join("heartbeat.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn context(
        active_tasks: usize,
        completion_rate: f64,
        confidence: f64,
    ) -> OperatingFrequencyScheduleContext {
        OperatingFrequencyScheduleContext {
            time_since_last_theta: Duration::ZERO,
            active_tasks,
            completion_rate,
            confidence,
            arousal: 0.4,
            dominance: -0.2,
        }
    }

    #[test]
    fn theta_becomes_due_when_scheduler_window_elapses() {
        let clock = HeartbeatClock::new();
        let now = Instant::now() + Duration::from_secs(181);
        assert!(clock.theta_due(now, context(2, 0.8, 0.8)));
        assert_eq!(
            clock.next_due(now, context(2, 0.8, 0.8)),
            Some(OperatingFrequency::Theta)
        );
    }

    #[test]
    fn delta_requires_idle_and_longer_interval() {
        let clock = HeartbeatClock::new();
        let now = Instant::now() + Duration::from_secs(60 * 60 + 5);
        assert!(clock.delta_due(now, context(0, 0.0, 0.7)));
        assert_eq!(
            clock.next_due(now, context(0, 0.0, 0.7)),
            Some(OperatingFrequency::Delta)
        );
        assert!(!clock.delta_due(now, context(2, 0.8, 0.7)));
    }

    #[test]
    fn persisting_snapshot_writes_latest_and_history() {
        let tmp = tempdir().expect("tempdir");
        let snapshot = HeartbeatSnapshot {
            timestamp: Utc::now(),
            frequency: OperatingFrequency::Theta,
            active_tasks: 3,
            ready_tasks: 1,
            completed_tasks: 2,
            failed_tasks: 0,
            completion_rate: 0.4,
            active_agents: 1,
            seconds_since_last_theta: 180,
            delta_interval_secs: 3600,
            probes: vec![HeartbeatProbeResult::new(
                HeartbeatProbeKind::ThetaDue,
                true,
                Some("theta cadence elapsed".to_string()),
            )],
        };

        persist_heartbeat_snapshot(tmp.path(), &snapshot).expect("persist heartbeat");

        let latest = fs::read_to_string(heartbeat_snapshot_path(tmp.path())).expect("latest");
        assert!(latest.contains("\"frequency\": \"theta\""));

        let history = fs::read_to_string(heartbeat_history_path(tmp.path())).expect("history");
        assert!(history.contains("\"theta_due\""));
    }
}
