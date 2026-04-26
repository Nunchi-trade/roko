//! Bridge between runner state changes and the TUI dashboard.
//!
//! Wraps `StateHubSender` with convenience methods that publish
//! `DashboardEvent` variants for each significant runner event.

use roko_core::dashboard_snapshot::DashboardEvent;
use roko_core::state_hub::StateHubSender;

/// Publishes runner events to the TUI / dashboard via `StateHub`.
#[derive(Clone)]
pub struct TuiBridge {
    sender: StateHubSender,
}

impl TuiBridge {
    /// Create a new bridge from a `StateHubSender`.
    pub fn new(sender: StateHubSender) -> Self {
        Self { sender }
    }

    /// A plan has started execution.
    pub fn plan_started(&self, plan_id: &str) {
        self.sender.publish(DashboardEvent::PlanStarted {
            plan_id: plan_id.to_string(),
        });
    }

    /// A plan has completed (successfully or not).
    pub fn plan_completed(&self, plan_id: &str, success: bool) {
        self.sender.publish(DashboardEvent::PlanCompleted {
            plan_id: plan_id.to_string(),
            success,
        });
    }

    /// A task has started.
    pub fn task_started(&self, plan_id: &str, task_id: &str, title: &str, phase: &str) {
        self.sender.publish(DashboardEvent::TaskStarted {
            plan_id: plan_id.to_string(),
            task_id: task_id.to_string(),
            title: title.to_string(),
            phase: phase.to_string(),
        });
    }

    /// A task has completed.
    pub fn task_completed(&self, plan_id: &str, task_id: &str, outcome: &str) {
        self.sender.publish(DashboardEvent::TaskCompleted {
            plan_id: plan_id.to_string(),
            task_id: task_id.to_string(),
            outcome: outcome.to_string(),
        });
    }

    /// An agent has been spawned.
    pub fn agent_spawned(&self, agent_id: &str, role: &str) {
        self.sender.publish(DashboardEvent::AgentSpawned {
            agent_id: agent_id.to_string(),
            role: role.to_string(),
        });
    }

    /// Agent produced text output (streamed).
    pub fn agent_output(&self, agent_id: &str, content: &str) {
        self.sender.publish(DashboardEvent::AgentOutput {
            agent_id: agent_id.to_string(),
            content: content.to_string(),
        });
    }

    /// Agent has finished.
    pub fn agent_completed(&self, agent_id: &str) {
        self.sender.publish(DashboardEvent::AgentCompleted {
            agent_id: agent_id.to_string(),
        });
    }

    /// A gate verdict.
    pub fn gate_result(&self, plan_id: &str, task_id: &str, gate: &str, passed: bool) {
        self.sender.publish(DashboardEvent::GateResult {
            plan_id: plan_id.to_string(),
            task_id: task_id.to_string(),
            gate: gate.to_string(),
            passed,
        });
    }

    /// Phase transition within a plan.
    pub fn phase_transition(&self, plan_id: &str, from: &str, to: &str) {
        self.sender.publish(DashboardEvent::PhaseTransition {
            plan_id: plan_id.to_string(),
            from: from.to_string(),
            to: to.to_string(),
        });
    }

    /// Efficiency metric for a task.
    pub fn efficiency_event(&self, plan_id: &str, task_id: &str, metric: &str, value: f64) {
        self.sender.publish(DashboardEvent::EfficiencyEvent {
            plan_id: plan_id.to_string(),
            task_id: task_id.to_string(),
            metric: metric.to_string(),
            value,
        });
    }

    /// Error event.
    pub fn error(&self, message: &str) {
        self.sender.publish(DashboardEvent::Error {
            message: message.to_string(),
        });
    }
}
