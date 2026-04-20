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
pub mod safety;
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
    CpmAnalysis, DagConfig, DagError, DagMutation, DagMutationError, DagPartition, DagStats,
    Durability, ExecutionWave, FusionConfig, IncrementalDag, UnifiedTaskDag, detect_cycle_nodes,
};
pub use event_log::{EventEntry, EventKind, EventLog, EventLogSnapshot, IntegrityError};
pub use executor::{
    CURRENT_SCHEMA_VERSION, DeltaSnapshot, EffectivePriorityTracker, ExecutorAction,
    ExecutorConfig, ExecutorEvent, ExecutorSnapshot, GateResult, ParallelExecutor,
    PersistedCircuitBreakerFailureRecord, PersistedCircuitBreakerState, PlanResourceInfo,
    PlanState, PlanStateMachine, PriorityCeiling, ResourceBudget, ResourceId, ResourceUsage,
    SnapshotConfig, SnapshotIntegrityError, SnapshotVerifier, SpeculativeExecution,
    TransitionError, current_schema_version,
};
pub use merge_queue::{MergeQueue, MergeRequest};
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
pub use replan::{ReplanResult, ReplanStrategy};
