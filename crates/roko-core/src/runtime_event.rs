//! Runtime events emitted by the workflow engine.
//!
//! All observers (ACP adapter, SSE adapter, JSONL logger, TUI bridge)
//! consume these via `EventBus<RuntimeEvent>`.

use crate::foundation::TokenUsage;
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

/// Summary of a tool call captured during an agent turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallSummary {
    pub name: String,
    pub result_preview: String,
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

    // Inference tracking
    InferenceStarted {
        request_id: String,
        model: String,
        agent_id: String,
        auto_routed: bool,
    },
    InferenceCompleted {
        request_id: String,
        model: String,
        agent_id: String,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        duration_ms: u64,
    },
    InferenceFailed {
        request_id: String,
        model: String,
        agent_id: String,
        error: String,
    },

    // Agent traces
    AgentTrace {
        agent_id: String,
        turn: u32,
        tool_calls: Vec<ToolCallSummary>,
        reasoning: Option<String>,
        usage: TokenUsage,
    },

    // Demo run and task lifecycle
    TaskFailed {
        plan_id: String,
        task_id: String,
        error: String,
        gate_failure: bool,
    },
    RunStarted {
        run_id: String,
        prompt: String,
        complexity: String,
    },
    RunCompleted {
        run_id: String,
        success: bool,
        cost_usd: f64,
        duration_ms: u64,
    },

    // Knowledge flow
    KnowledgeIngested {
        entry_id: String,
        topic: String,
        source_agent: String,
    },
    KnowledgeConsumed {
        entry_id: String,
        topic: String,
        consuming_agent: String,
    },
}

impl RuntimeEvent {
    /// Returns the run-scoped identifier when this event carries one.
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
            | Self::StateCheckpointed { run_id, .. }
            | Self::RunStarted { run_id, .. }
            | Self::RunCompleted { run_id, .. } => run_id,
            Self::TaskFailed { plan_id, .. } => plan_id,
            Self::InferenceStarted { .. }
            | Self::InferenceCompleted { .. }
            | Self::InferenceFailed { .. }
            | Self::AgentTrace { .. }
            | Self::KnowledgeIngested { .. }
            | Self::KnowledgeConsumed { .. } => "",
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
            Self::InferenceStarted { .. } => "inference_started",
            Self::InferenceCompleted { .. } => "inference_completed",
            Self::InferenceFailed { .. } => "inference_failed",
            Self::AgentTrace { .. } => "agent_trace",
            Self::TaskFailed { .. } => "task_failed",
            Self::RunStarted { .. } => "run_started",
            Self::RunCompleted { .. } => "run_completed",
            Self::KnowledgeIngested { .. } => "knowledge_ingested",
            Self::KnowledgeConsumed { .. } => "knowledge_consumed",
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

    #[test]
    fn new_runtime_event_variants_serialize_roundtrip() {
        let events = vec![
            (
                RuntimeEvent::InferenceStarted {
                    request_id: "req-1".into(),
                    model: "claude-sonnet".into(),
                    agent_id: "agent-1".into(),
                    auto_routed: true,
                },
                "inference_started",
            ),
            (
                RuntimeEvent::InferenceCompleted {
                    request_id: "req-1".into(),
                    model: "claude-sonnet".into(),
                    agent_id: "agent-1".into(),
                    input_tokens: 100,
                    output_tokens: 50,
                    cost_usd: 0.0123,
                    duration_ms: 1200,
                },
                "inference_completed",
            ),
            (
                RuntimeEvent::InferenceFailed {
                    request_id: "req-2".into(),
                    model: "claude-sonnet".into(),
                    agent_id: "agent-1".into(),
                    error: "rate limited".into(),
                },
                "inference_failed",
            ),
            (
                RuntimeEvent::AgentTrace {
                    agent_id: "agent-1".into(),
                    turn: 2,
                    tool_calls: vec![ToolCallSummary {
                        name: "read_file".into(),
                        result_preview: "loaded runtime_event.rs".into(),
                    }],
                    reasoning: Some("checking event coverage".into()),
                    usage: TokenUsage {
                        input_tokens: 200,
                        output_tokens: 75,
                        total_tokens: 275,
                        cost_usd: 0.025,
                    },
                },
                "agent_trace",
            ),
            (
                RuntimeEvent::TaskFailed {
                    plan_id: "plan-1".into(),
                    task_id: "task-1".into(),
                    error: "gate failed".into(),
                    gate_failure: true,
                },
                "task_failed",
            ),
            (
                RuntimeEvent::RunStarted {
                    run_id: "run-1".into(),
                    prompt: "ship demo".into(),
                    complexity: "standard".into(),
                },
                "run_started",
            ),
            (
                RuntimeEvent::RunCompleted {
                    run_id: "run-1".into(),
                    success: true,
                    cost_usd: 0.42,
                    duration_ms: 9000,
                },
                "run_completed",
            ),
            (
                RuntimeEvent::KnowledgeIngested {
                    entry_id: "entry-1".into(),
                    topic: "event architecture".into(),
                    source_agent: "agent-1".into(),
                },
                "knowledge_ingested",
            ),
            (
                RuntimeEvent::KnowledgeConsumed {
                    entry_id: "entry-1".into(),
                    topic: "event architecture".into(),
                    consuming_agent: "agent-2".into(),
                },
                "knowledge_consumed",
            ),
        ];

        for (event, expected_kind) in events {
            let value = serde_json::to_value(&event).expect("serialize runtime event");
            assert_eq!(value["kind"], expected_kind);
            assert!(value.get("data").is_some());

            let decoded: RuntimeEvent =
                serde_json::from_value(value).expect("deserialize runtime event");
            assert_eq!(decoded, event);
            assert_eq!(decoded.kind(), expected_kind);
        }
    }
}
