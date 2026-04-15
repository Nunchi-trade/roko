//! Chain extensions: HDC-indexed knowledge, stigmergy, and projection primitives.
//!
//! This module is gated behind the `chain` cargo feature. With the feature off,
//! `mirage-rs` is a pure EVM fork simulator with no dependency on HDC types. With
//! it on, mirage gains the building blocks for an agent-coordination substrate:
//!
//! - [`insight`]: `InsightEntry` with six knowledge types and a decay state machine
//! - [`projection`]: project bytes / text / float embeddings into 10,240-bit HDC vectors
//! - [`hdc_index`]: brute-force top-K Hamming similarity index
//! - [`hnsw`]: binary HNSW for sub-linear approximate search at 10M+ scale
//! - [`knowledge`]: `KnowledgeStore` unifying post / confirm / challenge / decay / search
//! - [`pheromone`]: decaying THREAT/OPPORTUNITY/WISDOM signals with HDC retrieval
//!
//! None of this touches the EVM — it layers on top so golems can simulate (via the
//! fork) AND share semantic knowledge about what they learned (via this module).
//!
//! # Settlement unit
//!
//! The design docs describe a token named `GNOS` with bespoke tokenomics. This
//! POC intentionally collapses that to plain ETH (wei). Callers that need
//! richer demurrage / slash schedules can build those on top of the plain
//! `stake_wei` / `base_reward_wei` scalars.

pub mod agent;
pub mod hdc_index;
pub mod hnsw;
pub mod insight;
pub mod knowledge;
pub mod pheromone;
pub mod projection;
pub mod task;

pub use agent::{
    AgentEntry, AgentEvent, AgentRegistry, AgentStats, AgentTrace, CognitivePhase, SkillConfig,
};
pub use hdc_index::{HdcIndex, Hit, IndexedVector};
pub use hnsw::{HnswBinaryIndex, HnswConfig};
pub use insight::{InsightEntry, InsightId, KnowledgeKind, KnowledgeState};
pub use knowledge::{
    DUPLICATE_SIMILARITY_THRESHOLD, KnowledgeError, KnowledgeSnapshot, KnowledgeStore, PostOutcome,
};
pub use pheromone::{
    DECAY_BUCKETS, Pheromone, PheromoneField, PheromoneHit, PheromoneId, PheromoneKind,
};
pub use projection::{
    DEFAULT_EMBEDDING_DIM, HDC_BITS, ProjectionMatrix, project_bytes, project_tokens,
};
pub use task::{
    CompletionMetadata, TaskArtifact, TaskEntry, TaskError, TaskEvent, TaskId, TaskPriority,
    TaskState, TaskStats, TaskStore,
};
