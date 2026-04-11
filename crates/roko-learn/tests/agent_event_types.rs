//! Compile coverage for `AgentEvent` variants.

use roko_agent::chat_types::FinishReason;
use roko_agent::{StreamChunk, Usage};
use roko_learn::anomaly::Anomaly;
use roko_learn::events::AgentEvent;
use roko_learn::provider_health::ErrorClass;

#[test]
fn agent_event_types_construct_all_variants() {
    let usage = Usage {
        input_tokens: 120,
        output_tokens: 45,
        cache_read_tokens: 10,
        cache_create_tokens: 0,
        cost_usd: 0.12,
        wall_ms: 850,
    };

    let events = vec![
        AgentEvent::TurnStarted {
            task_id: "task-2k20".into(),
            model: "glm-5.1".into(),
            provider: "zai".into(),
            timestamp_ms: 1_700_000_000_000,
        },
        AgentEvent::ToolCallExecuted {
            tool_name: "read_file".into(),
            duration_ms: 33,
            success: true,
            result_tokens: 128,
        },
        AgentEvent::TurnCompleted {
            turn: 2,
            usage,
            tool_call_count: 1,
            gate_passed: Some(true),
            finish_reason: FinishReason::ToolCalls,
        },
        AgentEvent::GateResult {
            gate_name: "compile".into(),
            passed: true,
            score: 0.98,
            duration_ms: 412,
        },
        AgentEvent::ProviderError {
            provider_id: "zai".into(),
            error_class: ErrorClass::RateLimit,
            status: 429,
        },
        AgentEvent::CostRecorded {
            model: "glm-5.1".into(),
            provider: "zai".into(),
            cost_usd: 0.42,
            tokens: 2_048,
        },
        AgentEvent::AnomalyDetected {
            anomaly: Anomaly::PromptLoop { repeated_count: 5 },
        },
        AgentEvent::ExperimentAssigned {
            experiment_id: "router-exp".into(),
            variant_id: "variant-b".into(),
        },
        AgentEvent::SessionEstablished {
            session_id: "session-123".into(),
            provider: "zai".into(),
        },
        AgentEvent::ModelSelected {
            model: "glm-5.1".into(),
            stage: "primary".into(),
            score: 0.91,
        },
        AgentEvent::StreamChunk {
            chunk: StreamChunk::Done(FinishReason::Stop),
        },
    ];

    assert_eq!(events.len(), 11);
    assert!(matches!(
        events.last(),
        Some(AgentEvent::StreamChunk {
            chunk: StreamChunk::Done(FinishReason::Stop),
        })
    ));
}
