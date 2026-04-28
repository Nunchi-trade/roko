//! Roko kernel — the universal Engram type and six traits that compose to express
//! every capability in the Roko orchestration system.
//!
//! # Architecture
//!
//! The entire Roko system is built from **one noun** ([`Engram`]) and **six verbs**:
//!
//! | Trait | Purpose |
//! |---|---|
//! | [`Store`] | Store and query engrams |
//! | [`Score`](traits::Score) | Rate engrams along multi-dimensional axes |
//! | [`Verify`] | Verify engrams against ground truth |
//! | [`Route`] | Select one engram from many candidates |
//! | [`Compose`] | Combine engrams into a new engram under a budget |
//! | [`React`] | Watch engram streams and emit new engrams (interventions) |
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

#![allow(
    missing_docs,
    clippy::cloned_ref_to_slice_refs,
    clippy::collapsible_str_replace,
    clippy::derivable_impls,
    clippy::expect_used,
    clippy::module_name_repetitions,
    clippy::nursery,
    clippy::obfuscated_if_else,
    clippy::pedantic
)]

pub mod affect;
pub mod agent;
/// Cross-cut arbitration protocol for resolving Daimon/Neuro/Dreams conflicts (INT-21).
pub mod arbitration;
pub mod attestation;
pub mod body;
pub mod build;
/// Additional Bus backend implementations: BroadcastBus, MemoryBus, MultiBus.
pub mod bus_backends;
pub mod catalyst;
/// The Cell trait — universal computation unit for all protocol implementations.
pub mod cell;
pub mod cfactor;
/// Canonical provider-agnostic chat message types.
pub mod chat_types;
pub mod cognitive_workspace;
pub mod conductor;
pub mod config;
/// Connector trait for external system I/O (MCP, API, Database, Blockchain, Feed, Custom).
pub mod connector;
pub mod context;
pub mod dashboard_snapshot;
pub mod datum;
pub mod decay;
pub mod demurrage;
/// Domain profiles for agent specialization: gate defaults, tool sets, context templates.
pub mod domain_profile;
pub mod engram;
pub mod error;
pub mod extension;
/// Feed trait for agent-produced data streams (Raw, Derived, Composite, Meta).
pub mod feed;
/// Forensic replay engine for causal decision reconstruction (SAFE-12).
pub mod forensic;
pub mod foundation;
pub mod hash;
/// Heartbeat protocol types for health monitoring.
pub mod heartbeat;
/// Cognitive immune system -- quarantine, anomaly detection, incident linking.
pub mod immune;
/// Marketplace job types shared between serve, TUI, and CLI.
pub mod job;
pub mod kind;
pub mod language;
pub mod loop_tick;
pub mod metric;
pub mod namespace;
pub mod obs {
    #[path = "../obs/health.rs"]
    pub mod health;
    #[path = "../obs/histograms.rs"]
    pub mod histograms;
    #[path = "../obs/metrics.rs"]
    pub mod metrics;
    #[path = "../obs/schema.rs"]
    pub mod schema;
    #[path = "../obs/scrub.rs"]
    pub mod scrub;

    pub use health::{
        AlwaysUpProbe, DegradedReason, HealthStatus, NamedProbe, Probe, ProbeRegistry,
        ReadinessStatus,
    };
    pub use histograms::{Histogram, HistogramSnapshot, LLM_LATENCY_BUCKETS};
    pub use metrics::{
        Counter, Gauge, LabelSet, MetricKind, MetricRegistry, MetricSnapshot, MetricValue,
        STANDARD_METRICS, register_standard_metrics,
    };
    pub use schema::{CanonicalMetricSchema, MetricDescriptor, MetricSchema, SCHEMA_VERSION};
    pub use scrub::{LogScrubber, REDACTED};
}
pub mod operating_frequency;
pub mod phase;
pub mod policy_manifest;
pub mod polyglot;
pub mod prediction;
pub mod project;
pub mod provenance;
pub mod pulse;
pub mod query;
pub mod runtime_event;
pub mod score;
pub mod secrets;
pub mod shutdown;
/// Signal — forward-compatible alias for `Engram` (Phase 1 prep).
pub mod signal;
pub mod signal_kinds;
pub mod task;
pub mod temperament;
pub mod tool;
pub mod traits;
pub mod verdict;

pub use affect::{BehavioralState, DaimonPolicy, EmotionalTag, PadVector};
pub use agent::{
    AgentBackend, AgentRole, ModelSpec, ModelTier, ProviderKind, ReasoningEffort, TaskRequirements,
    ToolPermissions, TurnBudget, score_model_for_task, select_model_for_task,
    select_model_for_task_with_bonus,
};
pub use arbitration::{
    ArbitrationConfig, ArbitrationOutcome, Arbitrator, Subsystem, SubsystemGuidance,
};
pub use attestation::{Attestation, ChainAttestation, Ed25519Signature, PublicKey};
pub use body::Body;
pub use build::{BuildCommand, BuildSystem};
pub use bus_backends::{
    BroadcastBus, BroadcastBusReceiver, BusErased, MemoryBus, MemoryBusReceiver, MultiBus,
};
pub use catalyst::{CatalystImpactSummary, CatalystScorer, CatalystSignalSource};
pub use cell::*;
pub use cfactor::{CFactorPolicy, CFactorSource, CFactorSummary};
pub use chat_types::{
    ChatMessage, ChatRequest, ChatResponse, ContentBlock, FinishReason, ImageUrl, MessageContent,
    RequestOptions, ResponseFormat, ResponseMetadata, SessionState, ToolCallFunction,
    ToolCallMessage, ToolChoice, Usage,
};
pub use cognitive_workspace::{
    COGNITIVE_WORKSPACE_SCHEMA_VERSION, CapabilityGrant, CognitiveWorkspace, ContextPolicyAuditRef,
    ContextRejectionAudit, ContextRejectionAuditReason, ContextScopeAudit, ContextSectionAudit,
    InvocationGateOutcome, InvocationReviewVerdictOutcome, ModelChoice, OutputParseResult,
    PolicyVersionRef, PromptSectionAudit, RewardObservation, TaskInvocationContract,
};
pub use conductor::{CognitiveSignal, ConductorDecision, ConductorEvaluation};
pub use connector::{
    ConnectorConfig, ConnectorHealth, ConnectorInfo, ConnectorKind, ConnectorRegistry,
    ConnectorStatus,
};
pub use context::Context;
pub use datum::Datum;
pub use decay::Decay;
pub use demurrage::Demurrage;
pub use domain_profile::{DomainProfile, TypedContext};
pub use engram::{Engram, EngramBuilder, HdcFingerprint};
pub use error::{Result, RokoError};
pub use feed::{FeedAccess, FeedInfo, FeedKind, FeedRegistry};
pub use forensic::{
    ForensicReplay, ForensicReplayLogger, GateVerdictRecord, PolicyDecisionRecord, PolicyOutcome,
    ReconstructionStep, RouterAlternative, RouterDecisionRecord, ScoredReference, StepStatus,
};
pub use foundation::{
    ChatMessage as FoundationChatMessage, Effect, EffectExecutor, EffectOutcome, EventConsumer,
    FeedbackEvent, FeedbackSink, GateConfig, GateReport, GateRunner, GateVerdict, MessageRole,
    ModelCallRequest, ModelCallResponse, ModelCaller, PromptAssembler, PromptSpec, TokenUsage,
};
pub use hash::ContentHash;
pub use heartbeat::{
    AgentEndpoints, DEFAULT_HEARTBEAT_INTERVAL_SECS, HEARTBEAT_RING_CAPACITY, HeartbeatPayload,
    NetworkStats, SenderInfo,
};
pub use immune::{
    AnomalyScore, ImmuneResponse, IncidentLink, IncidentRelation, QuarantineDecision,
    QuarantineEntry, QuarantineStatus, QuarantineVault, ResponseAction,
};
pub use kind::Kind;
pub use language::{Import, ImportKind, LanguageProvider, Symbol, SymbolKind, Visibility};
pub use loop_tick::{TickConfig, TickOutcome, loop_tick, loop_tick_with_config};
pub use metric::{ConfigHash, Headlines, TaskMetric, compute_headlines};
pub use operating_frequency::{
    OperatingFrequency, OperatingFrequencyAffect, OperatingFrequencyScheduleContext,
    OperatingFrequencyScheduler,
};
pub use phase::{FailureKind, PhaseKind, PlanPhase, is_monotonic_progression, valid_transitions};
pub use policy_manifest::{
    BUILTIN_ROLE_POLICY_MANIFEST_PATH, BUILTIN_ROLE_POLICY_MANIFEST_TOML,
    CURRENT_POLICY_MANIFEST_SCHEMA_VERSION, ContextPolicyRef, FallbackBehavior, GateExpectation,
    InclusionMode, InclusionRule, MANIFEST_BACKED_BUILTIN_ROLE_IDS, ManifestError,
    ManifestLookupError, ManifestValidationError, OutputFormat, OutputSchemaExpectation,
    PolicyProvenance, PromptBudgetPolicy, PromptPolicy, PromptPolicySection, PromptSectionSource,
    RolePolicyManifest, RoleProfile, RoleSafetyPolicy, SectionBudget, ToolCapabilityPolicy,
};
pub use polyglot::{PolyglotProject, detect_polyglot};
pub use prediction::{
    AccuracyStats, CalibrationStats, CalibrationTracker, ChainCondition, ChainMetric,
    ChainQueryPayload, ChainTarget, ChangeContext, CodingMetric, CodingQueryPayload, CodingScope,
    ExponentialMovingAverage, OperationsMetric, OperationsQueryPayload, Oracle, OracleDomain,
    OracleQuery, PredictedValue, Prediction, PredictionAccuracy, PredictionCalibrationSource,
    PredictionCalibrationSummary, PredictionInterval, PredictionOutcome, PredictionPolicy,
    PredictionProvenance, PredictionStore, PredictiveScorer, QueryPayload, ResearchMetric,
    ResearchQueryPayload, ResidualCorrector, SourceReference,
};
pub use project::{
    DetectedBuildSystem, Language, ProjectInfo, detect_from_files,
    detect_from_files_with_cargo_toml,
};
pub use provenance::{
    Provenance, ProvenanceCoherenceCheck, ProvenanceCoherenceIssue, Taint, TaintInfo,
};
pub use pulse::{PolicyOutputs, Pulse, PulseBuilder, Topic, TopicFilter};
pub use query::{Budget, Query};
pub use roko_primitives::HdcVector;
pub use runtime_event::{RuntimeEvent, WorkflowOutcome};
pub use score::Score;
pub use signal::{Signal, SignalBuilder};
pub use signal_kinds::*;
pub use task::{
    GlobalTaskId, PlanStatus, Task, TaskCategory, TaskComplexityBand, TaskContextWeight,
    TaskDomain, TaskMeta, TaskQualityProfile, TaskReasoningLevel, TaskSpeedPriority, TaskStatus,
};
// Note: tool::FailureKind (for tool-call failures) is NOT re-exported here to avoid
// collision with phase::FailureKind (for PlanPhase failures); reach it via
// `roko_core::tool::FailureKind`.
pub use dashboard_snapshot::{
    AgentState as DashboardAgentState, AgentTopology, AgentTopologyEdge, AgentTopologyNode,
    CFactorBucket, DashboardEvent, DashboardEventLogEntry, DashboardSnapshot, DiagnosisSeverity,
    DiagnosisSummary, EfficiencyBucket, ExperimentWinnerSummary, FailureEntry,
    KnowledgeBrowseEntry as DashboardKnowledgeBrowseEntry, SnapshotStats, TrendBucket,
    TrendBuckets,
};
pub use job::{
    CreateJobRequest, FileJobStore, JobError, JobEvaluation, JobFilter, JobGateResult,
    JobProgressEntry, JobStats, JobStatus, JobSubmission, JobType, MarketplaceJob, PrdSummary,
    TaskSummary as JobTaskSummary,
};
pub use namespace::{
    Channel, ChannelDirection, CognitiveNamespace, NamespaceAcl, NamespaceRegistry, RateLimitConfig,
};
pub use temperament::Temperament;
pub use tool::{
    ArmEntry, Artifact, AuditSink, BanditKey, CancelSource, CancelToken, EpsilonGreedyBandit,
    FailureTrace, FormatBandit, KeywordOverlapScorer, MemoryPointer, MetricsKey, MetricsSink,
    ProfileBandit, RewardConfig, ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef,
    ToolError, ToolFormat, ToolFormatProfile, ToolHandler, ToolMetrics, ToolOutcome,
    ToolPermission, ToolRegistry, ToolRelevanceScorer, ToolResult, ToolSchema, ToolSource,
    ToolTrace, ToolTraceEvent, TraceBuilder, TraceId, TraceSink, TraceStep, VecToolRegistry,
    classify_tool_error, compute_reward, galileo_tsq, profile_for_model,
};
pub use traits::{Bus, ColdStore, Compose, Connect, Observe, React, Route, Store, Trigger, Verify};
// Note: The `Score` protocol trait (formerly `Scorer`) is NOT re-exported at
// the crate root to avoid colliding with the `Score` value struct. Access it
// via `roko_core::traits::Score` or import with an alias.
pub use verdict::{Outcome, Selection, TestCount, Verdict};
