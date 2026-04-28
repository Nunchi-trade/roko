//! Bridge between runner state changes and the TUI dashboard.
//!
//! Wraps `StateHubSender` with convenience methods that publish
//! `DashboardEvent` variants for each significant runner event.

use crate::state_hub::StateHubSender;
use roko_core::dashboard_snapshot::DashboardEvent;

use super::types::RunnerEvent;

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

    /// A task changed phase.
    pub fn task_phase_changed(
        &self,
        plan_id: &str,
        task_id: &str,
        old_phase: &str,
        new_phase: &str,
    ) {
        self.sender.publish(DashboardEvent::TaskPhaseChanged {
            plan_id: plan_id.to_string(),
            task_id: task_id.to_string(),
            old_phase: old_phase.to_string(),
            new_phase: new_phase.to_string(),
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
    pub fn agent_spawned(&self, agent_id: &str, role: &str, model: &str) {
        self.sender.publish(DashboardEvent::AgentSpawned {
            agent_id: agent_id.to_string(),
            role: role.to_string(),
            model: model.to_string(),
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

    /// Publish a typed runner lifecycle event into the dashboard event log.
    pub fn runner_event(&self, event: &RunnerEvent) {
        self.sender.publish(DashboardEvent::EventLogEntry {
            timestamp_ms: event.timestamp_ms(),
            event_type: event.event_type().to_string(),
            plan_id: event.plan_id().unwrap_or_default().to_string(),
            task_id: event.task_id().unwrap_or_default().to_string(),
            message: event.message(),
        });
    }

    /// Cascade router state updated after observation.
    pub fn cascade_router_updated(&self, snapshot_json: &str) {
        self.sender.publish(DashboardEvent::CascadeRouterUpdated {
            snapshot_json: snapshot_json.to_string(),
        });
    }

    /// Model was selected for a task dispatch.
    pub fn model_selected(&self, plan_id: &str, task_id: &str, model: &str, source: &str) {
        self.sender.publish(DashboardEvent::EventLogEntry {
            timestamp_ms: timestamp_now_ms(),
            event_type: "model_selected".to_string(),
            plan_id: plan_id.to_string(),
            task_id: task_id.to_string(),
            message: format!("model={model} source={source}"),
        });
    }

    /// Extension hook fired.
    pub fn extension_hook(&self, plan_id: &str, task_id: &str, hook: &str, success: bool) {
        self.sender.publish(DashboardEvent::EventLogEntry {
            timestamp_ms: timestamp_now_ms(),
            event_type: "extension_hook".to_string(),
            plan_id: plan_id.to_string(),
            task_id: task_id.to_string(),
            message: format!("hook={hook} success={success}"),
        });
    }
}

fn timestamp_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
