//! Roko kernel — the universal Signal type and six traits that compose to express
//! every capability in the Roko orchestration system.
//!
//! # Architecture
//!
//! The entire Roko system is built from **one noun** ([`Signal`]) and **six verbs**:
//!
//! | Trait | Purpose |
//! |---|---|
//! | [`Substrate`] | Store and query signals |
//! | [`Scorer`] | Rate signals along multi-dimensional axes |
//! | [`Gate`] | Verify signals against ground truth |
//! | [`Router`] | Select one signal from many candidates |
//! | [`Composer`] | Combine signals into a new signal under a budget |
//! | [`Policy`] | Watch signal streams and emit new signals (interventions) |
//!
//! Every capability — coding-agent spawning, verification gates, context assembly,
//! model routing, memory retrieval, chain participation, bounty markets, HDC search
//! — is one of these six verbs operating on Signals.
//!
//! # The Universal Loop
//!
//! Every operation in Roko follows the same shape:
//!
//! ```text
//! query substrate → score → route/compose → gate verdict → write back → policy fires
//! ```
//!
//! See [`loop_tick`] for the concrete implementation.
//!
//! # Extensibility
//!
//! All enums marked `#[non_exhaustive]` can grow without breaking downstream impls.
//! New capabilities are added by implementing one of the six traits; no core
//! changes are needed.

#![allow(clippy::module_name_repetitions)]

pub mod agent;
pub mod body;
pub mod build;
pub mod conductor;
pub mod config;
pub mod context;
pub mod dashboard_snapshot;
pub mod decay;
pub mod error;
pub mod hash;
pub mod kind;
pub mod language;
pub mod loop_tick;
pub mod metric;
pub mod obs;
pub mod operating_frequency;
pub mod phase;
pub mod polyglot;
pub mod project;
pub mod provenance;
pub mod query;
pub mod score;
pub mod secrets;
pub mod shutdown;
pub mod signal;
pub mod signal_kinds;
pub mod state_hub;
pub mod task;
pub mod tool;
pub mod traits;
pub mod verdict;

pub use agent::{
    AgentBackend, AgentRole, ModelSpec, ModelTier, ProviderKind, ReasoningEffort, ToolPermissions,
    TurnBudget,
};
pub use body::Body;
pub use build::{BuildCommand, BuildSystem};
pub use conductor::ConductorDecision;
pub use context::Context;
pub use decay::Decay;
pub use error::{Result, RokoError};
pub use hash::ContentHash;
pub use kind::Kind;
pub use language::{Import, ImportKind, LanguageProvider, Symbol, SymbolKind, Visibility};
pub use loop_tick::{TickOutcome, loop_tick};
pub use metric::{ConfigHash, Headlines, TaskMetric, compute_headlines};
pub use operating_frequency::{
    OperatingFrequency, OperatingFrequencyAffect, OperatingFrequencyScheduleContext,
    OperatingFrequencyScheduler,
};
pub use phase::{FailureKind, PhaseKind, PlanPhase, is_monotonic_progression, valid_transitions};
pub use polyglot::{PolyglotProject, detect_polyglot};
pub use project::{
    DetectedBuildSystem, Language, ProjectInfo, detect_from_files,
    detect_from_files_with_cargo_toml,
};
pub use provenance::Provenance;
pub use query::{Budget, Query};
pub use score::Score;
pub use signal::Signal;
pub use signal_kinds::*;
pub use task::{
    GlobalTaskId, PlanStatus, Task, TaskCategory, TaskComplexityBand, TaskContextWeight, TaskMeta,
    TaskQualityProfile, TaskReasoningLevel, TaskSpeedPriority, TaskStatus,
};
// Note: tool::FailureKind (for tool-call failures) is NOT re-exported here to avoid
// collision with phase::FailureKind (for PlanPhase failures); reach it via
// `roko_core::tool::FailureKind`.
pub use tool::{
    ArmEntry, Artifact, AuditSink, BanditKey, CancelSource, CancelToken, EpsilonGreedyBandit,
    FailureTrace, FormatBandit, KeywordOverlapScorer, MemoryPointer, MetricsKey, MetricsSink,
    ProfileBandit, RewardConfig, ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef,
    ToolError, ToolFormat, ToolFormatProfile, ToolHandler, ToolMetrics, ToolOutcome,
    ToolPermission, ToolRegistry, ToolRelevanceScorer, ToolResult, ToolSchema, ToolTrace,
    ToolTraceEvent, TraceBuilder, TraceId, TraceSink, TraceStep, VecToolRegistry, compute_reward,
    galileo_tsq, profile_for_model,
};
pub use dashboard_snapshot::{DashboardEvent, DashboardSnapshot};
pub use state_hub::{SharedStateHub, StateHub, StateHubSender, shared_state_hub};
pub use traits::{Composer, Gate, Policy, Router, Scorer, Substrate};
pub use verdict::{Outcome, Selection, TestCount, Verdict};
