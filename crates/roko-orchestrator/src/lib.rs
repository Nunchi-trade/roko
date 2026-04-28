//! Orchestration layer for Roko: plan discovery, task DAG, worktree
//! management, and parallel execution.
//!
//! This crate is the **entry point to the whole orchestrator** —
//! everything downstream (conductor, executor, merge queue) reads plans
//! and tasks from the types exposed here.
//!
//! # Subsystems shipped so far
//!
//! - [`plan_discovery`] — scan a plans directory, parse YAML
//!   frontmatter, rank plans by priority (§1.1–§1.5).

#![deny(missing_docs)]
#![allow(
    clippy::assigning_clones,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::derive_partial_eq_without_eq,
    clippy::doc_markdown,
    clippy::match_same_arms,
    clippy::missing_const_for_fn,
    clippy::missing_panics_doc,
    clippy::needless_pass_by_value,
    clippy::or_fun_call,
    clippy::significant_drop_tightening,
    clippy::suboptimal_flops,
    clippy::too_many_lines,
    clippy::unnecessary_map_or,
    clippy::unused_self,
    clippy::unwrap_used
)]

/// Coordination primitives derived from the coordination docs.
pub mod coordination;
pub mod dag;
pub mod event_log;
pub mod executor;
pub mod merge_queue;
/// WebSocket relay for mesh-scope pheromone synchronization.
pub mod mesh_relay;
pub mod plan_discovery;
pub mod post_merge;
pub mod progress;
pub mod repair;
pub mod replan;
pub mod runtime_snapshot;
pub mod safety;
/// Shared construction path for workflow services used by surfaces.
pub mod service_factory;
pub mod worktree;

pub use coordination::{
    AccessModel, AgentId, CohortMetrics, CohortWeights, CollectiveId, CompetitionOutcome,
    Confirmation, CustomKindError, MorphogeneticParams, MorphogeneticState, Pheromone,
    PheromoneKind, PheromoneScope, PromotionConfig, PromotionGate, ResponseThresholds,
    STRATEGY_DIMS, ScopePromotionConfig, SpecializationIndex, SubnetId, SubnetIdError,
    SubnetMembership, SubnetPermissions, TRUST_DISCOUNT, WisdomGate, c_factor, check_promotion,
    check_scope_promotion, compete, current_intensity, effective_confirmations, hill_response,
    pheromone_decay, pheromone_decay_for_kind, specialization_index, trust_discounted_intensity,
    validate_custom_kind,
};
pub use dag::{
    CpmAnalysis, DAG_EXECUTION_SNAPSHOT_SCHEMA_VERSION, DagConfig, DagError, DagExecutionSnapshot,
    DagMutation, DagMutationError, DagPartition, DagStats, DagTaskExecutionMetadata,
    DagTaskExecutionStatus, Durability, ExecutionWave, FusionConfig, IncrementalDag,
    UnifiedTaskDag, detect_cycle_nodes,
};
pub use event_log::{EventEntry, EventKind, EventLog, EventLogSnapshot, IntegrityError};
pub use executor::{
    CURRENT_SCHEMA_VERSION, DeltaSnapshot, EffectivePriorityTracker, ExecutorAction,
    ExecutorConfig, ExecutorEvent, ExecutorSnapshot, GateResult, ParallelExecutor,
    PersistedCircuitBreakerFailureRecord, PersistedCircuitBreakerState, PlanResourceInfo,
    PlanResumeDirective, PlanState, PlanStateMachine, PriorityCeiling, RecoveredPlanResume,
    RecoveredState, RecoveryEngine, RecoveryError, RecoveryResumePlan, RecoveryWarning,
    ResourceBudget, ResourceId, ResourceUsage, SnapshotConfig, SnapshotIntegrityError,
    SnapshotVerifier, SpeculativeExecution, TransitionError, WarningSeverity,
    current_schema_version,
};
pub use merge_queue::{
    DEFAULT_MAX_MERGE_RETRIES, MergeConflict, MergeQueue, MergeQueueEntrySnapshot,
    MergeQueueMetrics, MergeQueueSnapshot, MergeRequest, MergeStatus,
};
pub use mesh_relay::{MeshRelay, PeerState, SeqNo, SequencedPheromone};
pub use plan_discovery::{
    DiscoveryError, PlanFrontmatter, PlanInfo, ValidationError, discover_plans, parse_frontmatter,
    rank_plans, validate_frontmatter,
};
pub use post_merge::{PostMergeCheck, PostMergeResult, PostMergeRunner};
pub use progress::{
    ErrorEvent, ProgressError, ProgressTracker, ValidationError as EnrichmentValidationError,
    publish_error, validate_enrichment,
};
pub use repair::{
    FailureContext, RepairAction, RepairConfig, RepairDecision, RepairEngine, RepairLevel,
    StabilityMetric,
};
pub use replan::{
    FailureDisposition, PlanRevisionEvidence, PlanRevisionRequest, ReplanResult, ReplanStrategy,
};
pub use runtime_snapshot::{
    ORCHESTRATOR_SNAPSHOT_SCHEMA_VERSION, OrchestratorSnapshot,
    orchestrator_snapshot_schema_version,
};
pub use service_factory::{ServiceBundle, ServiceConfig, ServiceFactory};
pub use worktree::{
    WorktreeConfig, WorktreeError, WorktreeHandle, WorktreeHealth, WorktreeIsolationStatus,
    WorktreeManager, WorktreeSnapshot, format_branch_name,
};
