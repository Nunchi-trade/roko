//! Server-side events emitted during plan execution, agent runs, and other
//! operations. These flow through the [`EventBus`] and are streamed to
//! connected SSE / WebSocket clients.

use serde::{Deserialize, Serialize};

/// Progress emitted by the execution loop as plans move through phases,
/// complete tasks, and finish gate checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionEvent {
    /// Plan identifier.
    pub plan_id: String,
    /// Task identifier, if applicable.
    pub task_id: String,
    /// Phase the execution is in or transitioning to.
    pub phase: String,
    /// Progress status, such as `transitioned`, `completed`, `passed`, or `failed`.
    pub status: String,
    /// ISO-8601 UTC timestamp.
    pub timestamp: String,
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
        #[serde(flatten)]
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
}
