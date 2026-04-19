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
/// WebSocket relay for mesh-scope pheromone synchronization.
pub mod mesh_relay;
pub mod event_log;
pub mod executor;
pub mod merge_queue;
pub mod plan_discovery;
pub mod post_merge;
pub mod progress;
pub mod replan;
pub mod safety;
pub mod worktree;

pub use coordination::{
    AgentId, CohortMetrics, CohortWeights, CollectiveId, CustomKindError, MorphogeneticParams,
    MorphogeneticState, Pheromone, PheromoneKind, PheromoneScope, PromotionConfig,
    ResponseThresholds, STRATEGY_DIMS, SpecializationIndex, SubnetId, SubnetIdError, WisdomGate,
    c_factor, check_promotion, current_intensity, effective_confirmations, hill_response,
    pheromone_decay, specialization_index, validate_custom_kind,
};
pub use mesh_relay::{MeshRelay, PeerState, SeqNo, SequencedPheromone};
pub use dag::{
    CpmAnalysis, DagConfig, DagError, DagMutation, DagMutationError, DagStats, Durability,
    ExecutionWave, FusionConfig, IncrementalDag, UnifiedTaskDag, detect_cycle_nodes,
};
pub use event_log::{EventEntry, EventKind, EventLog, EventLogSnapshot, IntegrityError};
pub use executor::{
    CURRENT_SCHEMA_VERSION, DeltaSnapshot, ExecutorAction, ExecutorConfig, ExecutorEvent,
    ExecutorSnapshot, GateResult, ParallelExecutor, PersistedCircuitBreakerFailureRecord,
    PersistedCircuitBreakerState, PlanState, PlanStateMachine, SnapshotConfig,
    SnapshotIntegrityError, SnapshotVerifier, SpeculativeExecution, TransitionError,
    current_schema_version,
};
pub use merge_queue::{MergeQueue, MergeRequest};
pub use plan_discovery::{
    DiscoveryError, PlanFrontmatter, PlanInfo, ValidationError, discover_plans, parse_frontmatter,
    rank_plans, validate_frontmatter,
};
pub use post_merge::{PostMergeCheck, PostMergeResult, PostMergeRunner};
pub use progress::{
    ErrorEvent, ProgressError, ProgressTracker, ValidationError as EnrichmentValidationError,
    publish_error, validate_enrichment,
};
pub use replan::{ReplanResult, ReplanStrategy};
