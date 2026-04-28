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
/// Cognitive energy model -- metabolic costs for cognitive operations.
pub mod energy;
pub mod event_bus;
pub mod heartbeat;
pub mod heartbeat_attention;
pub mod heartbeat_probes;
pub mod lifecycle;
pub mod metrics;
pub mod pipeline_state;
pub mod process;
pub mod resource;
pub mod task_scheduler;
pub mod theta_consumer;

pub use lifecycle::{
    Agent, AgentLifecycleState, AgentState, ConfigDrift, DegradationStage, GitOpsConfig,
    GitOpsRetryPolicy, HealthProbeConfig, HookSpec, LifecycleHooks, LifecycleTransition,
    LifecycleTransitionReason, MachineLifecycleState, MeshRegistered, NeuroInitialized,
    ProbeHandler, ProbeSpec, Ready, ResourcesAllocated, RestartBackoff, RoutingConfigured,
    ToolsLoaded, Unvalidated, Validated,
};
pub use pipeline_state::{
    Phase, PipelineInput, PipelineOutput, PipelineStateV2, WorkflowConfig, WorkflowOutcome,
};
pub use task_scheduler::{SchedulableTask, TaskScheduler, TaskStatus};
