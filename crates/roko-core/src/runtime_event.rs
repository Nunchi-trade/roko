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
        run_id: String,
        request_id: String,
        model: String,
        agent_id: String,
        auto_routed: bool,
    },
    InferenceCompleted {
        run_id: String,
        request_id: String,
        model: String,
        agent_id: String,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        duration_ms: u64,
    },
    InferenceFailed {
        run_id: String,
        request_id: String,
        model: String,
        agent_id: String,
        error: String,
    },

    // Agent traces
    AgentTrace {
        run_id: String,
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
        run_id: String,
        entry_id: String,
        topic: String,
        source_agent: String,
    },
    KnowledgeConsumed {
        run_id: String,
        entry_id: String,
        topic: String,
        consuming_agent: String,
    },

    // Progress tracking
    /// First token received from an inference call — carries TTFT for latency dashboards.
    InferenceFirstToken {
        run_id: String,
        request_id: String,
        model: String,
        agent_id: String,
        /// Time-to-first-token in milliseconds.
        ttft_ms: u64,
    },
    /// A tool call has started executing.
    ToolCallStarted {
        run_id: String,
        agent_id: String,
        tool: String,
        iteration: u32,
    },
    /// A tool call has finished executing.
    ToolCallCompleted {
        run_id: String,
        agent_id: String,
        tool: String,
        duration_ms: u64,
        success: bool,
    },
    /// A plan task has started executing.
    TaskStarted {
        run_id: String,
        plan_id: String,
        task_id: String,
        task_title: String,
        role: String,
    },
    /// A plan task has finished executing.
    TaskCompleted {
        run_id: String,
        plan_id: String,
        task_id: String,
        passed: bool,
        duration_ms: u64,
    },
    /// The overall pipeline entered a new phase.
    PipelinePhase {
        run_id: String,
        phase: String,
        /// "started", "complete", or "failed".
        status: String,
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
            | Self::InferenceStarted { run_id, .. }
            | Self::InferenceCompleted { run_id, .. }
            | Self::InferenceFailed { run_id, .. }
            | Self::AgentTrace { run_id, .. }
            | Self::RunStarted { run_id, .. }
            | Self::RunCompleted { run_id, .. }
            | Self::KnowledgeIngested { run_id, .. }
            | Self::KnowledgeConsumed { run_id, .. }
            | Self::InferenceFirstToken { run_id, .. }
            | Self::ToolCallStarted { run_id, .. }
            | Self::ToolCallCompleted { run_id, .. }
            | Self::TaskStarted { run_id, .. }
            | Self::TaskCompleted { run_id, .. }
            | Self::PipelinePhase { run_id, .. } => run_id,
            Self::TaskFailed { plan_id, .. } => plan_id,
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
            Self::InferenceFirstToken { .. } => "inference_first_token",
            Self::ToolCallStarted { .. } => "tool_call_started",
            Self::ToolCallCompleted { .. } => "tool_call_completed",
            Self::TaskStarted { .. } => "task_started",
            Self::TaskCompleted { .. } => "task_completed",
            Self::PipelinePhase { .. } => "pipeline_phase",
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
                    run_id: "run-1".into(),
                    request_id: "req-1".into(),
                    model: "claude-sonnet".into(),
                    agent_id: "agent-1".into(),
                    auto_routed: true,
                },
                "inference_started",
            ),
            (
                RuntimeEvent::InferenceCompleted {
                    run_id: "run-1".into(),
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
                    run_id: "run-1".into(),
                    request_id: "req-2".into(),
                    model: "claude-sonnet".into(),
                    agent_id: "agent-1".into(),
                    error: "rate limited".into(),
                },
                "inference_failed",
            ),
            (
                RuntimeEvent::AgentTrace {
                    run_id: "run-1".into(),
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
                    run_id: "run-1".into(),
                    entry_id: "entry-1".into(),
                    topic: "event architecture".into(),
                    source_agent: "agent-1".into(),
                },
                "knowledge_ingested",
            ),
            (
                RuntimeEvent::KnowledgeConsumed {
                    run_id: "run-1".into(),
                    entry_id: "entry-1".into(),
                    topic: "event architecture".into(),
                    consuming_agent: "agent-2".into(),
                },
                "knowledge_consumed",
            ),
            (
                RuntimeEvent::InferenceFirstToken {
                    run_id: "run-1".into(),
                    request_id: "req-ft".into(),
                    model: "claude-sonnet".into(),
                    agent_id: "agent-1".into(),
                    ttft_ms: 1823,
                },
                "inference_first_token",
            ),
            (
                RuntimeEvent::ToolCallStarted {
                    run_id: "run-1".into(),
                    agent_id: "agent-1".into(),
                    tool: "read_file".into(),
                    iteration: 3,
                },
                "tool_call_started",
            ),
            (
                RuntimeEvent::ToolCallCompleted {
                    run_id: "run-1".into(),
                    agent_id: "agent-1".into(),
                    tool: "read_file".into(),
                    duration_ms: 12,
                    success: true,
                },
                "tool_call_completed",
            ),
            (
                RuntimeEvent::TaskStarted {
                    run_id: "run-1".into(),
                    plan_id: "plan-1".into(),
                    task_id: "task-1".into(),
                    task_title: "Implement progress events".into(),
                    role: "implementer".into(),
                },
                "task_started",
            ),
            (
                RuntimeEvent::TaskCompleted {
                    run_id: "run-1".into(),
                    plan_id: "plan-1".into(),
                    task_id: "task-1".into(),
                    passed: true,
                    duration_ms: 47200,
                },
                "task_completed",
            ),
            (
                RuntimeEvent::PipelinePhase {
                    run_id: "run-1".into(),
                    phase: "execute".into(),
                    status: "started".into(),
                },
                "pipeline_phase",
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
