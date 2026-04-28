//! Runtime events emitted by the workflow engine.
//!
//! All observers (ACP adapter, SSE adapter, JSONL logger, TUI bridge)
//! consume these via `EventBus<RuntimeEvent>`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeEventEnvelope {
    pub run_id: String,
    pub seq: u64,
    pub ts: DateTime<Utc>,
    pub schema_version: u8,
    pub source: String,
    pub payload: RuntimeEvent,
}

impl RuntimeEventEnvelope {
    pub fn new(
        run_id: impl Into<String>,
        seq: u64,
        source: impl Into<String>,
        payload: RuntimeEvent,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            seq,
            ts: Utc::now(),
            schema_version: 1,
            source: source.into(),
            payload,
        }
    }
}

/// Outcome of a completed workflow run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WorkflowOutcome {
    /// Workflow completed successfully, optionally with a commit hash.
    Success { commit_hash: Option<String> },
    /// Workflow halted due to an error or resource limit.
    Halted { reason: String },
    /// Workflow was cancelled by the user.
    Cancelled,
}

/// Every event the workflow engine can emit.
///
/// These events are fire-and-forget: the engine emits them and does not wait
/// for observers to process them. Observers subscribe via
/// `EventBus<RuntimeEvent>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum RuntimeEvent {
    // Lifecycle
    WorkflowStarted {
        run_id: String,
        template: String,
        prompt: String,
    },
    PhaseTransition {
        run_id: String,
        from: String,
        to: String,
    },
    WorkflowCompleted {
        run_id: String,
        outcome: WorkflowOutcome,
    },

    // Agent
    AgentSpawned {
        run_id: String,
        agent_id: String,
        role: String,
        model: String,
    },
    AgentOutput {
        run_id: String,
        agent_id: String,
        chunk: String,
    },
    AgentCompleted {
        run_id: String,
        agent_id: String,
        output: String,
        tokens_used: u64,
        cost_usd: f64,
    },
    AgentFailed {
        run_id: String,
        agent_id: String,
        error: String,
    },

    // Gates
    GateStarted {
        run_id: String,
        gate_name: String,
        rung: u8,
    },
    GatePassed {
        run_id: String,
        gate_name: String,
        duration_ms: u64,
    },
    GateFailed {
        run_id: String,
        gate_name: String,
        output: String,
        duration_ms: u64,
    },

    // Feedback
    FeedbackRecorded {
        run_id: String,
        kind: String,
        summary: String,
    },

    // Persistence
    StateCheckpointed {
        run_id: String,
        path: String,
    },
}

impl RuntimeEvent {
    /// Returns the run_id common to all event variants.
    pub fn run_id(&self) -> &str {
        match self {
            Self::WorkflowStarted { run_id, .. }
            | Self::PhaseTransition { run_id, .. }
            | Self::WorkflowCompleted { run_id, .. }
            | Self::AgentSpawned { run_id, .. }
            | Self::AgentOutput { run_id, .. }
            | Self::AgentCompleted { run_id, .. }
            | Self::AgentFailed { run_id, .. }
            | Self::GateStarted { run_id, .. }
            | Self::GatePassed { run_id, .. }
            | Self::GateFailed { run_id, .. }
            | Self::FeedbackRecorded { run_id, .. }
            | Self::StateCheckpointed { run_id, .. } => run_id,
        }
    }

    /// Human-readable event kind label.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::WorkflowStarted { .. } => "workflow_started",
            Self::PhaseTransition { .. } => "phase_transition",
            Self::WorkflowCompleted { .. } => "workflow_completed",
            Self::AgentSpawned { .. } => "agent_spawned",
            Self::AgentOutput { .. } => "agent_output",
            Self::AgentCompleted { .. } => "agent_completed",
            Self::AgentFailed { .. } => "agent_failed",
            Self::GateStarted { .. } => "gate_started",
            Self::GatePassed { .. } => "gate_passed",
            Self::GateFailed { .. } => "gate_failed",
            Self::FeedbackRecorded { .. } => "feedback_recorded",
            Self::StateCheckpointed { .. } => "state_checkpointed",
        }
    }
}

impl fmt::Display for RuntimeEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.run_id(), self.kind())
    }
}

impl fmt::Display for WorkflowOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Success {
                commit_hash: Some(hash),
            } => write!(f, "success ({hash})"),
            Self::Success { commit_hash: None } => write!(f, "success"),
            Self::Halted { reason } => write!(f, "halted: {reason}"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_id_accessor() {
        let event = RuntimeEvent::WorkflowStarted {
            run_id: "r1".into(),
            template: "express".into(),
            prompt: "fix bug".into(),
        };

        assert_eq!(event.run_id(), "r1");
        assert_eq!(event.kind(), "workflow_started");
    }

    #[test]
    fn display_formats() {
        let outcome = WorkflowOutcome::Success {
            commit_hash: Some("abc123".into()),
        };

        assert!(outcome.to_string().contains("abc123"));
    }
}
