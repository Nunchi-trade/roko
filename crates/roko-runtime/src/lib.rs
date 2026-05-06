//! `roko-runtime` — shared async runtime primitives for Roko.
//!
//! This crate extracts the foundational runtime concerns that Mori (and other Roko
//! applications) depend on:
//!
//! - **[`event_bus`]**: A typed, bounded broadcast channel with replay support.
//!   Generalises the ad-hoc `mpsc` channels scattered through `apps/mori`.
//!
//! - **[`process`]**: Process lifecycle management — spawn, track, kill, reap.
//!   Extracts the core supervision patterns from `agent/connection.rs`.
//!
//! - **[`cancel`]**: Cooperative cancellation tokens and shutdown coordination.
//!
//! - **[`metrics`]**: Append-only structured metric recording (JSONL).
//!
//! # Design principles
//!
//! 1. **No domain types.** This crate knows nothing about agents, plans, gates, or TUI.
//!    It provides generic infrastructure that higher layers parameterise.
//! 2. **Tokio-native.** All primitives are `Send + Sync + 'static` and designed for
//!    multi-task Tokio runtimes.
//! 3. **Zero unsafe.** All concurrency goes through `tokio::sync` or `std::sync::atomic`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::missing_const_for_fn,
    clippy::unnecessary_map_or,
    clippy::doc_markdown,
    clippy::too_long_first_doc_paragraph,
    clippy::suboptimal_flops,
    clippy::needless_range_loop,
    clippy::match_same_arms,
    clippy::derive_partial_eq_without_eq,
    clippy::return_self_not_must_use,
    clippy::map_unwrap_or
)]

pub mod cancel;
pub mod delta_consumer;
pub mod demurrage_consumer;
pub mod effect_driver;
/// Cognitive energy model -- metabolic costs for cognitive operations.
pub mod energy;
pub mod event_bus;
pub mod heartbeat;
pub mod heartbeat_attention;
pub mod heartbeat_probes;
pub mod http_event_sink;
pub mod jsonl_logger;
pub mod lifecycle;
pub mod metrics;
pub mod pipeline_state;
pub mod process;
pub mod projection;
pub mod pulse_bus;
pub mod resource;
pub mod run_ledger;
pub mod state_hub;
pub mod state_snapshot;
pub mod task_scheduler;
pub mod theta_consumer;
pub mod workflow_engine;

pub use cancel::CancelToken;
pub use effect_driver::{EffectDriver, EffectServices};
pub use http_event_sink::HttpEventSink;
// Foundation types re-exported from roko-core for backwards compatibility
pub use jsonl_logger::JsonlLogger;
pub use lifecycle::{
    Agent, AgentLifecycleState, AgentState, ConfigDrift, DegradationStage, GitOpsConfig,
    GitOpsRetryPolicy, HealthProbeConfig, HookSpec, LifecycleHooks, LifecycleTransition,
    LifecycleTransitionReason, MachineLifecycleState, MeshRegistered, NeuroInitialized,
    ProbeHandler, ProbeSpec, Ready, ResourcesAllocated, RestartBackoff, RoutingConfigured,
    ToolsLoaded, Unvalidated, Validated,
};
pub use pipeline_state::{
    CommitOutcome, Phase, PipelineInput, PipelineOutput, PipelineStateV2, WorkflowConfig,
    WorkflowOutcome,
};
pub use projection::{RunSummary, RuntimeProjection};
pub use pulse_bus::{PulseBus, PulseBusReceiver};
pub use roko_core::RuntimeEvent;
pub use roko_core::foundation::{
    ChatMessage, EventConsumer, FeedbackEvent, FeedbackSink, GateConfig, GateReport, GateRunner,
    GateVerdict, MessageRole, ModelCallRequest, ModelCallResponse, ModelCaller, PromptAssembler,
    PromptSpec, ShellGateCommand, TokenUsage,
};
pub use run_ledger::{
    AgentOutcome, ArtifactOutcome, CancellationOutcome, EffectErrorKind, EventPersistenceHealth,
    GateRunOutcome, PhaseTransitionRecord, RunLedger,
};
pub use state_hub::{SharedStateHub, StateHub, StateHubSender, shared_state_hub};
pub use state_snapshot::{STATE_SNAPSHOT_VERSION, StateSnapshot};
pub use task_scheduler::{SchedulableTask, TaskScheduler, TaskStatus};
pub use workflow_engine::{
    GateOutcome, WorkflowEngine, WorkflowResult, WorkflowRunConfig, WorkflowRunReport,
};

#[cfg(test)]
mod contract_guards {
    use roko_core::RuntimeEvent;
    use roko_core::runtime_event::{RuntimeEventEnvelope, WorkflowOutcome};
    use std::path::{Path, PathBuf};

    fn runtime_src_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
    }

    fn rust_source_files(dir: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files.extend(rust_source_files(&path));
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
        files
    }

    fn runtime_source() -> String {
        rust_source_files(&runtime_src_dir())
            .into_iter()
            .map(|path| std::fs::read_to_string(path).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn runtime_does_not_define_duplicate_foundation_contracts() {
        let source = runtime_source();
        let trait_prefix = "pub trait ";
        let struct_prefix = "pub struct ";

        for forbidden in [
            [trait_prefix, "AffectPolicy"].concat(),
            [trait_prefix, "DispatchModulation"].concat(),
            [struct_prefix, "DispatchModulation"].concat(),
        ] {
            assert!(
                !source.contains(&forbidden),
                "runtime must import foundation contract `{forbidden}` from roko-core"
            );
        }
    }

    #[test]
    fn jsonl_logger_does_not_serialize_events_as_debug_strings() {
        let source = std::fs::read_to_string(runtime_src_dir().join("jsonl_logger.rs")).unwrap();
        let debug_format = ["format!(\"", "{:", "?}", "\""].concat();
        let captured_debug = ["{", "event", ":", "?", "}"].concat();

        assert!(
            !source.contains(&debug_format),
            "jsonl logger must serialize events with serde_json, not Debug formatting"
        );
        assert!(
            !source.contains(&captured_debug),
            "jsonl logger must not use captured Debug formatting as an event contract"
        );
    }

    #[test]
    fn runtime_event_envelopes_round_trip_as_json() {
        let events = vec![
            RuntimeEvent::WorkflowStarted {
                run_id: "run-1".into(),
                template: "express".into(),
                prompt: "fix bug".into(),
            },
            RuntimeEvent::PhaseTransition {
                run_id: "run-1".into(),
                from: "plan".into(),
                to: "execute".into(),
            },
            RuntimeEvent::AgentSpawned {
                run_id: "run-1".into(),
                agent_id: "agent-1".into(),
                role: "implementer".into(),
                model: "model-1".into(),
            },
            RuntimeEvent::AgentOutput {
                run_id: "run-1".into(),
                agent_id: "agent-1".into(),
                chunk: "partial".into(),
            },
            RuntimeEvent::AgentCompleted {
                run_id: "run-1".into(),
                agent_id: "agent-1".into(),
                output: "done".into(),
                tokens_used: 42,
                cost_usd: 0.12,
            },
            RuntimeEvent::AgentFailed {
                run_id: "run-1".into(),
                agent_id: "agent-2".into(),
                error: "failed".into(),
            },
            RuntimeEvent::GateStarted {
                run_id: "run-1".into(),
                gate_name: "compile".into(),
                rung: 1,
            },
            RuntimeEvent::GatePassed {
                run_id: "run-1".into(),
                gate_name: "compile".into(),
                duration_ms: 250,
            },
            RuntimeEvent::GateFailed {
                run_id: "run-1".into(),
                gate_name: "test".into(),
                output: "failure".into(),
                duration_ms: 500,
            },
            RuntimeEvent::FeedbackRecorded {
                run_id: "run-1".into(),
                kind: "model_call".into(),
                summary: "recorded".into(),
            },
            RuntimeEvent::StateCheckpointed {
                run_id: "run-1".into(),
                path: ".roko/checkpoint.json".into(),
            },
            RuntimeEvent::WorkflowCompleted {
                run_id: "run-1".into(),
                outcome: WorkflowOutcome::Success {
                    commit_hash: Some("abc123".into()),
                },
            },
            RuntimeEvent::InferenceFirstToken {
                run_id: "run-1".into(),
                request_id: "req-ft".into(),
                model: "claude-sonnet".into(),
                agent_id: "agent-1".into(),
                ttft_ms: 1823,
            },
            RuntimeEvent::ToolCallStarted {
                run_id: "run-1".into(),
                agent_id: "agent-1".into(),
                tool: "read_file".into(),
                iteration: 1,
            },
            RuntimeEvent::ToolCallCompleted {
                run_id: "run-1".into(),
                agent_id: "agent-1".into(),
                tool: "read_file".into(),
                duration_ms: 12,
                success: true,
            },
            RuntimeEvent::TaskStarted {
                run_id: "run-1".into(),
                plan_id: "plan-1".into(),
                task_id: "task-1".into(),
                task_title: "Wire progress events".into(),
                role: "implementer".into(),
            },
            RuntimeEvent::TaskCompleted {
                run_id: "run-1".into(),
                plan_id: "plan-1".into(),
                task_id: "task-1".into(),
                passed: true,
                duration_ms: 47200,
            },
            RuntimeEvent::PipelinePhase {
                run_id: "run-1".into(),
                phase: "execute".into(),
                status: "started".into(),
            },
        ];

        for (seq, event) in events.into_iter().enumerate() {
            let run_id = event.run_id().to_string();
            let envelope = RuntimeEventEnvelope::new(run_id, seq as u64, "test", event);
            let json = serde_json::to_string(&envelope).unwrap();
            let round_tripped: RuntimeEventEnvelope = serde_json::from_str(&json).unwrap();
            assert_eq!(round_tripped, envelope);
        }
    }
}
