//! Unified event types shared across learning subsystems.
//!
//! These events are intentionally lightweight and provider-agnostic so runtime
//! components can publish one stream that downstream learning systems consume.

use crate::anomaly::Anomaly;
use crate::provider_health::ErrorClass;
use roko_agent::chat_types::FinishReason;
use roko_agent::{StreamChunk, Usage};

/// Canonical event payload emitted by the learning/runtime feedback pipeline.
#[allow(missing_docs)]
#[derive(Debug, Clone)]
pub enum AgentEvent {
    TurnStarted {
        task_id: String,
        model: String,
        provider: String,
        timestamp_ms: i64,
    },
    ToolCallExecuted {
        tool_name: String,
        duration_ms: u64,
        success: bool,
        result_tokens: u64,
    },
    TurnCompleted {
        turn: u32,
        usage: Usage,
        tool_call_count: usize,
        gate_passed: Option<bool>,
        finish_reason: FinishReason,
    },
    GateResult {
        gate_name: String,
        passed: bool,
        score: f32,
        duration_ms: u64,
    },
    ProviderError {
        provider_id: String,
        error_class: ErrorClass,
        status: u16,
    },
    CostRecorded {
        model: String,
        provider: String,
        cost_usd: f64,
        tokens: u64,
    },
    AnomalyDetected {
        anomaly: Anomaly,
    },
    ExperimentAssigned {
        experiment_id: String,
        variant_id: String,
    },
    SessionEstablished {
        session_id: String,
        provider: String,
    },
    ModelSelected {
        model: String,
        stage: String,
        score: f64,
    },
    StreamChunk {
        chunk: StreamChunk,
    },
}
