//! Roko kernel — the universal Engram type and six traits that compose to express
//! every capability in the Roko orchestration system.
//!
//! # Architecture
//!
//! The entire Roko system is built from **one noun** ([`Engram`]) and **six verbs**:
//!
//! | Trait | Purpose |
//! |---|---|
//! | [`Substrate`] | Store and query engrams |
//! | [`Scorer`] | Rate engrams along multi-dimensional axes |
//! | [`Gate`] | Verify engrams against ground truth |
//! | [`Router`] | Select one engram from many candidates |
//! | [`Composer`] | Combine engrams into a new engram under a budget |
//! | [`Policy`] | Watch engram streams and emit new engrams (interventions) |
//!
//! Every capability — coding-agent spawning, verification gates, context assembly,
//! model routing, memory retrieval, chain participation, bounty markets, HDC search
//! — is one of these six verbs operating on Engrams.
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

pub mod affect;
pub mod agent;
pub mod attestation;
pub mod body;
pub mod build;
pub mod catalyst;
/// Canonical provider-agnostic chat message types.
pub mod chat_types;
pub mod cfactor;
pub mod conductor;
pub mod config;
pub mod context;
pub mod dashboard_snapshot;
pub mod decay;
pub mod engram;
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
pub mod prediction;
pub mod query;
pub mod score;
pub mod secrets;
pub mod shutdown;
pub mod signal_kinds;
pub mod state_hub;
pub mod task;
pub mod tool;
pub mod traits;
pub mod verdict;

pub use affect::{BehavioralState, DaimonPolicy, EmotionalTag, PadVector};
pub use agent::{
    AgentBackend, AgentRole, ModelSpec, ModelTier, ProviderKind, ReasoningEffort, TaskRequirements,
    ToolPermissions, TurnBudget, score_model_for_task, select_model_for_task,
    select_model_for_task_with_bonus,
};
pub use attestation::{Attestation, ChainAttestation, Ed25519Signature, PublicKey};
pub use body::Body;
pub use build::{BuildCommand, BuildSystem};
pub use catalyst::{CatalystImpactSummary, CatalystScorer, CatalystSignalSource};
pub use chat_types::{
    ChatMessage, ContentBlock, ImageUrl, MessageContent, ToolCallFunction, ToolCallMessage,
};
pub use cfactor::{CFactorPolicy, CFactorSource, CFactorSummary};
pub use conductor::ConductorDecision;
pub use context::Context;
pub use decay::Decay;
pub use engram::{Engram, EngramBuilder};
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
pub use prediction::{
    PredictiveScorer, PredictionCalibrationSource, PredictionCalibrationSummary, PredictionPolicy,
};
pub use query::{Budget, Query};
pub use score::Score;
pub use signal_kinds::*;
pub use task::{
    GlobalTaskId, PlanStatus, Task, TaskCategory, TaskComplexityBand, TaskContextWeight, TaskMeta,
    TaskQualityProfile, TaskReasoningLevel, TaskSpeedPriority, TaskStatus,
};
// Note: tool::FailureKind (for tool-call failures) is NOT re-exported here to avoid
// collision with phase::FailureKind (for PlanPhase failures); reach it via
// `roko_core::tool::FailureKind`.
pub use dashboard_snapshot::{DashboardEvent, DashboardSnapshot};
pub use state_hub::{SharedStateHub, StateHub, StateHubSender, shared_state_hub};
pub use tool::{
    ArmEntry, Artifact, AuditSink, BanditKey, CancelSource, CancelToken, EpsilonGreedyBandit,
    FailureTrace, FormatBandit, KeywordOverlapScorer, MemoryPointer, MetricsKey, MetricsSink,
    ProfileBandit, RewardConfig, ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef,
    ToolError, ToolFormat, ToolFormatProfile, ToolHandler, ToolMetrics, ToolOutcome,
    ToolPermission, ToolRegistry, ToolRelevanceScorer, ToolResult, ToolSchema, ToolSource,
    ToolTrace, ToolTraceEvent, TraceBuilder, TraceId, TraceSink, TraceStep, VecToolRegistry,
    compute_reward, galileo_tsq, profile_for_model,
};
pub use traits::{Composer, Gate, Policy, Router, Scorer, Substrate};
pub use verdict::{Outcome, Selection, TestCount, Verdict};
