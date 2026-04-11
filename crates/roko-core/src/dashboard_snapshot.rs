//! Materialized dashboard state driven by events.
//!
//! [`DashboardSnapshot`] is the single source of truth for all dashboard
//! consumers (TUI, WebSocket, SSE, REST). It is updated atomically via
//! [`apply`] when the [`StateHub`](super::state_hub::StateHub) receives events.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Event type — the only thing DashboardSnapshot knows how to ingest
// ---------------------------------------------------------------------------

/// Events that mutate the dashboard snapshot.
///
/// These map 1:1 to the interesting subset of `ServerEvent` variants from
/// `roko-serve`. The conversion happens at the call-site so that `roko-core`
/// stays free of server dependencies.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DashboardEvent {
    /// A plan execution started.
    PlanStarted { plan_id: String },
    /// A plan execution completed.
    PlanCompleted { plan_id: String, success: bool },
    /// A task started executing.
    TaskStarted {
        plan_id: String,
        task_id: String,
        phase: String,
    },
    /// A task completed.
    TaskCompleted {
        plan_id: String,
        task_id: String,
        outcome: String,
    },
    /// A task changed phase.
    TaskPhaseChanged {
        plan_id: String,
        task_id: String,
        old_phase: String,
        new_phase: String,
    },
    /// An agent was spawned.
    AgentSpawned { agent_id: String, role: String },
    /// Incremental agent output.
    AgentOutput { agent_id: String, content: String },
    /// A gate check completed.
    GateResult {
        plan_id: String,
        task_id: String,
        gate: String,
        passed: bool,
    },
    /// The plan transitioned between phases.
    PhaseTransition {
        plan_id: String,
        from: String,
        to: String,
    },
    /// An efficiency metric was recorded.
    EfficiencyEvent {
        plan_id: String,
        task_id: String,
        metric: String,
        value: f64,
    },
    /// An error occurred.
    Error { message: String },
}

// ---------------------------------------------------------------------------
// Snapshot types
// ---------------------------------------------------------------------------

/// A single plan's live state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanState {
    /// Plan identifier.
    pub plan_id: String,
    /// Current phase name.
    pub phase: String,
    /// Total tasks registered for this plan.
    pub tasks_total: usize,
    /// Tasks that completed successfully.
    pub tasks_done: usize,
    /// Tasks that failed.
    pub tasks_failed: usize,
    /// Whether the plan is still executing.
    pub active: bool,
}

/// A single task's live state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskState {
    /// Task identifier.
    pub task_id: String,
    /// Parent plan identifier.
    pub plan_id: String,
    /// Current phase name.
    pub phase: String,
    /// Outcome string, if completed.
    pub outcome: Option<String>,
}

/// A single agent's live state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    /// Agent identifier.
    pub agent_id: String,
    /// Agent role.
    pub role: String,
    /// Whether the agent is still running.
    pub active: bool,
    /// Byte count of output received so far.
    pub output_bytes: usize,
}

/// A single gate verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateVerdict {
    /// Plan identifier.
    pub plan_id: String,
    /// Task identifier.
    pub task_id: String,
    /// Gate name.
    pub gate: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Unix timestamp in milliseconds.
    pub ts_millis: u64,
}

/// Recent error entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEntry {
    /// Error message.
    pub message: String,
    /// Unix timestamp in milliseconds.
    pub ts_millis: u64,
}

/// The full materialized dashboard state.
///
/// Updated atomically by [`StateHub`](super::state_hub::StateHub) via
/// `watch::Sender::send_modify`. Consumers (TUI, web, API) borrow this
/// through a `watch::Receiver` for zero-copy reads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    /// Active and recently completed plans.
    pub plans: HashMap<String, PlanState>,
    /// Active tasks keyed by `"{plan_id}/{task_id}"`.
    pub tasks: HashMap<String, TaskState>,
    /// Active agents keyed by agent_id.
    pub agents: HashMap<String, AgentState>,
    /// Recent gate verdicts (ring of last 256).
    pub gates: Vec<GateVerdict>,
    /// Recent errors (ring of last 64).
    pub errors: Vec<ErrorEntry>,
    /// Overall counts.
    pub stats: SnapshotStats,
}

/// Aggregate counters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnapshotStats {
    /// Number of plans currently executing.
    pub plans_active: usize,
    /// Number of plans that completed successfully.
    pub plans_completed: usize,
    /// Number of plans that failed.
    pub plans_failed: usize,
    /// Number of tasks currently executing.
    pub tasks_active: usize,
    /// Number of tasks that completed successfully.
    pub tasks_completed: usize,
    /// Number of tasks that failed.
    pub tasks_failed: usize,
    /// Number of agents currently running.
    pub agents_active: usize,
    /// Number of gates that passed.
    pub gates_passed: usize,
    /// Number of gates that failed.
    pub gates_failed: usize,
    /// Total errors recorded.
    pub errors_total: usize,
}

// ---------------------------------------------------------------------------
// Ring buffer limits
// ---------------------------------------------------------------------------

const MAX_GATES: usize = 256;
const MAX_ERRORS: usize = 64;

// ---------------------------------------------------------------------------
// apply()
// ---------------------------------------------------------------------------

impl DashboardSnapshot {
    /// Apply a single event, mutating the snapshot in place.
    ///
    /// This is called inside `watch::Sender::send_modify` so that all
    /// subscribers see a consistent view after each event.
    pub fn apply(&mut self, event: &DashboardEvent) {
        self.apply_with_ts(event, current_ts_millis());
    }

    /// Apply with an explicit timestamp (for testing).
    pub fn apply_with_ts(&mut self, event: &DashboardEvent, ts: u64) {
        match event {
            DashboardEvent::PlanStarted { plan_id } => {
                self.stats.plans_active += 1;
                self.plans.insert(
                    plan_id.clone(),
                    PlanState {
                        plan_id: plan_id.clone(),
                        phase: "started".into(),
                        active: true,
                        ..Default::default()
                    },
                );
            }
            DashboardEvent::PlanCompleted { plan_id, success } => {
                if let Some(plan) = self.plans.get_mut(plan_id) {
                    plan.active = false;
                    plan.phase = if *success {
                        "completed".into()
                    } else {
                        "failed".into()
                    };
                }
                self.stats.plans_active = self.stats.plans_active.saturating_sub(1);
                if *success {
                    self.stats.plans_completed += 1;
                } else {
                    self.stats.plans_failed += 1;
                }
            }
            DashboardEvent::TaskStarted {
                plan_id,
                task_id,
                phase,
            } => {
                self.stats.tasks_active += 1;
                let key = format!("{plan_id}/{task_id}");
                self.tasks.insert(
                    key,
                    TaskState {
                        task_id: task_id.clone(),
                        plan_id: plan_id.clone(),
                        phase: phase.clone(),
                        outcome: None,
                    },
                );
                if let Some(plan) = self.plans.get_mut(plan_id) {
                    plan.tasks_total += 1;
                }
            }
            DashboardEvent::TaskCompleted {
                plan_id,
                task_id,
                outcome,
            } => {
                let key = format!("{plan_id}/{task_id}");
                let failed = outcome.contains("fail") || outcome.contains("error");
                if let Some(task) = self.tasks.get_mut(&key) {
                    task.phase = "completed".into();
                    task.outcome = Some(outcome.clone());
                }
                self.stats.tasks_active = self.stats.tasks_active.saturating_sub(1);
                if failed {
                    self.stats.tasks_failed += 1;
                    if let Some(plan) = self.plans.get_mut(plan_id) {
                        plan.tasks_failed += 1;
                    }
                } else {
                    self.stats.tasks_completed += 1;
                    if let Some(plan) = self.plans.get_mut(plan_id) {
                        plan.tasks_done += 1;
                    }
                }
            }
            DashboardEvent::TaskPhaseChanged {
                plan_id,
                task_id,
                new_phase,
                ..
            } => {
                let key = format!("{plan_id}/{task_id}");
                if let Some(task) = self.tasks.get_mut(&key) {
                    task.phase = new_phase.clone();
                }
            }
            DashboardEvent::AgentSpawned { agent_id, role } => {
                self.stats.agents_active += 1;
                self.agents.insert(
                    agent_id.clone(),
                    AgentState {
                        agent_id: agent_id.clone(),
                        role: role.clone(),
                        active: true,
                        output_bytes: 0,
                    },
                );
            }
            DashboardEvent::AgentOutput { agent_id, content } => {
                if let Some(agent) = self.agents.get_mut(agent_id) {
                    agent.output_bytes += content.len();
                }
            }
            DashboardEvent::GateResult {
                plan_id,
                task_id,
                gate,
                passed,
            } => {
                if *passed {
                    self.stats.gates_passed += 1;
                } else {
                    self.stats.gates_failed += 1;
                }
                if self.gates.len() >= MAX_GATES {
                    self.gates.remove(0);
                }
                self.gates.push(GateVerdict {
                    plan_id: plan_id.clone(),
                    task_id: task_id.clone(),
                    gate: gate.clone(),
                    passed: *passed,
                    ts_millis: ts,
                });
            }
            DashboardEvent::PhaseTransition { plan_id, to, .. } => {
                if let Some(plan) = self.plans.get_mut(plan_id) {
                    plan.phase = to.clone();
                }
            }
            DashboardEvent::EfficiencyEvent { .. } => {
                // Efficiency metrics are tracked separately by the learn subsystem.
            }
            DashboardEvent::Error { message } => {
                self.stats.errors_total += 1;
                if self.errors.len() >= MAX_ERRORS {
                    self.errors.remove(0);
                }
                self.errors.push(ErrorEntry {
                    message: message.clone(),
                    ts_millis: ts,
                });
            }
        }
    }
}

#[allow(clippy::cast_possible_truncation)]
fn current_ts_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_lifecycle() {
        let mut snap = DashboardSnapshot::default();

        snap.apply(&DashboardEvent::PlanStarted {
            plan_id: "p1".into(),
        });
        assert_eq!(snap.stats.plans_active, 1);
        assert!(snap.plans["p1"].active);

        snap.apply(&DashboardEvent::TaskStarted {
            plan_id: "p1".into(),
            task_id: "t1".into(),
            phase: "compose".into(),
        });
        assert_eq!(snap.stats.tasks_active, 1);
        assert_eq!(snap.plans["p1"].tasks_total, 1);

        snap.apply(&DashboardEvent::GateResult {
            plan_id: "p1".into(),
            task_id: "t1".into(),
            gate: "compile".into(),
            passed: true,
        });
        assert_eq!(snap.stats.gates_passed, 1);

        snap.apply(&DashboardEvent::TaskCompleted {
            plan_id: "p1".into(),
            task_id: "t1".into(),
            outcome: "success".into(),
        });
        assert_eq!(snap.stats.tasks_completed, 1);
        assert_eq!(snap.plans["p1"].tasks_done, 1);

        snap.apply(&DashboardEvent::PlanCompleted {
            plan_id: "p1".into(),
            success: true,
        });
        assert_eq!(snap.stats.plans_active, 0);
        assert_eq!(snap.stats.plans_completed, 1);
        assert!(!snap.plans["p1"].active);
    }

    #[test]
    fn gate_ring_eviction() {
        let mut snap = DashboardSnapshot::default();
        for i in 0..300 {
            snap.apply(&DashboardEvent::GateResult {
                plan_id: "p1".into(),
                task_id: format!("t{i}"),
                gate: "compile".into(),
                passed: true,
            });
        }
        assert_eq!(snap.gates.len(), MAX_GATES);
        assert_eq!(snap.stats.gates_passed, 300);
    }

    #[test]
    fn error_ring_eviction() {
        let mut snap = DashboardSnapshot::default();
        for i in 0..100 {
            snap.apply(&DashboardEvent::Error {
                message: format!("err {i}"),
            });
        }
        assert_eq!(snap.errors.len(), MAX_ERRORS);
        assert_eq!(snap.stats.errors_total, 100);
    }

    #[test]
    fn agent_output_accumulates() {
        let mut snap = DashboardSnapshot::default();
        snap.apply(&DashboardEvent::AgentSpawned {
            agent_id: "a1".into(),
            role: "coder".into(),
        });
        snap.apply(&DashboardEvent::AgentOutput {
            agent_id: "a1".into(),
            content: "hello world".into(),
        });
        assert_eq!(snap.agents["a1"].output_bytes, 11);
    }
}
