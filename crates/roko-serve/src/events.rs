//! Server-side events emitted during plan execution, agent runs, and other
//! operations. These flow through the shared event bus and are streamed to
//! connected SSE / WebSocket clients.

use serde::{Deserialize, Serialize};
use roko_core::Signal;

/// Progress emitted by the execution loop as plans move through phases,
/// complete tasks, and finish gate checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionEvent {
    /// A plan has begun execution.
    PlanStarted,

    /// A task has entered its first active phase.
    TaskStarted {
        /// Task identifier.
        task_id: String,
        /// Phase the task is starting in.
        phase: String,
    },

    /// A task transitioned between phases.
    TaskPhaseChanged {
        /// Task identifier.
        task_id: String,
        /// Previous phase name.
        old_phase: String,
        /// New phase name.
        new_phase: String,
    },

    /// A gate completed for a task.
    GateResult {
        /// Task identifier.
        task_id: String,
        /// Gate name.
        gate: String,
        /// Whether the gate passed.
        passed: bool,
        /// Human-readable message or failure summary.
        message: String,
    },

    /// A task has completed.
    TaskCompleted {
        /// Task identifier.
        task_id: String,
        /// Outcome summary.
        outcome: String,
    },

    /// A plan has completed.
    PlanCompleted {
        /// Plan outcome summary.
        outcome: String,
        /// Execution statistics for the plan.
        stats: serde_json::Value,
    },

    /// A re-plan was triggered for a task.
    ReplanTriggered {
        /// Task identifier that caused the re-plan.
        task_id: String,
        /// Re-plan strategy or reason.
        strategy: String,
    },

    /// A watcher emitted an alert.
    WatcherAlert {
        /// Watcher name.
        watcher: String,
        /// Alert message.
        message: String,
    },
}

/// A tagged union of all events the HTTP server can emit.
#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    /// A plan execution has started.
    PlanStarted { plan_id: String },

    /// A plan execution has completed.
    PlanCompleted { plan_id: String, success: bool },

    /// An agent process was spawned.
    AgentSpawned { agent_id: String, role: String },

    /// Incremental agent output (streamed).
    AgentOutput { agent_id: String, content: String },

    /// A gate check completed for a task.
    GateResult {
        plan_id: String,
        task_id: String,
        gate: String,
        passed: bool,
    },

    /// Execution progress update streamed from the orchestrator.
    Execution {
        /// Plan identifier for the execution payload.
        plan_id: String,
        /// Nested execution event.
        event: ExecutionEvent,
    },

    /// The plan transitioned between execution phases.
    PhaseTransition {
        plan_id: String,
        from: String,
        to: String,
    },

    /// An episode (agent turn + gate result) was recorded.
    Episode {
        plan_id: String,
        task_id: String,
        passed: bool,
    },

    /// An efficiency metric was recorded for a task.
    EfficiencyEvent {
        plan_id: String,
        task_id: String,
        metric: String,
        value: f64,
    },

    /// A one-shot run was started.
    RunStarted { run_id: String, prompt: String },

    /// A one-shot run completed.
    RunCompleted { run_id: String, success: bool },

    /// A generic operation was started.
    OperationStarted { op_id: String, kind: String },

    /// A generic operation completed.
    OperationCompleted {
        op_id: String,
        kind: String,
        success: bool,
    },

    /// A cloud deployment was created.
    DeploymentCreated { id: String, name: String },

    /// A cloud deployment is ready and reachable.
    DeploymentReady { id: String, url: String },

    /// A cloud deployment failed.
    DeploymentFailed { id: String, reason: String },

    /// A cloud deployment was torn down.
    DeploymentTornDown { id: String },

    /// A worker started executing a task.
    WorkerTaskStarted {
        deployment_id: String,
        task_id: String,
    },

    /// A worker completed a task.
    WorkerTaskCompleted {
        deployment_id: String,
        task_id: String,
        success: bool,
    },

    /// The server is shutting down.
    ServerShutdown,

    /// An error occurred.
    Error { message: String },

    /// A webhook signal was accepted and published for downstream processing.
    WebhookReceived { signal: Signal },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_event_serializes_with_type_tag() {
        let event = ExecutionEvent::GateResult {
            task_id: "task-1".into(),
            gate: "compile".into(),
            passed: false,
            message: "compile failed".into(),
        };

        let json = serde_json::to_value(event).expect("serialize execution event");
        assert_eq!(json["type"], "gate_result");
        assert_eq!(json["task_id"], "task-1");
        assert_eq!(json["gate"], "compile");
        assert_eq!(json["passed"], false);
        assert_eq!(json["message"], "compile failed");
    }
}
