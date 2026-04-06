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

pub mod dag;
pub mod event_log;
pub mod plan_discovery;
pub mod progress;
pub mod safety;
pub mod worktree;

pub use dag::{DagConfig, DagError, DagStats, ExecutionWave, UnifiedTaskDag};
pub use event_log::{EventEntry, EventKind, EventLog, EventLogSnapshot, IntegrityError};
pub use plan_discovery::{
    discover_plans, parse_frontmatter, rank_plans, validate_frontmatter, DiscoveryError, PlanInfo,
    PlanFrontmatter, ValidationError,
};
pub use progress::{
    publish_error, validate_enrichment, ErrorEvent, ProgressError, ProgressTracker,
    ValidationError as EnrichmentValidationError,
};
