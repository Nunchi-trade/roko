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

pub mod cancel;
pub mod event_bus;
pub mod heartbeat;
pub mod heartbeat_attention;
pub mod heartbeat_probes;
pub mod lifecycle;
pub mod metrics;
pub mod process;
pub mod resource;

pub use lifecycle::{
    Agent, AgentLifecycleState, AgentState, ConfigDrift, DegradationStage, GitOpsConfig,
    GitOpsRetryPolicy, HealthProbeConfig, HookSpec, LifecycleHooks, LifecycleTransition,
    LifecycleTransitionReason, MachineLifecycleState, MeshRegistered, NeuroInitialized,
    ProbeHandler, ProbeSpec, Ready, ResourcesAllocated, RestartBackoff, RoutingConfigured,
    ToolsLoaded, Unvalidated, Validated,
};
