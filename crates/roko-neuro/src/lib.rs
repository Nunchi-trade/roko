//! Knowledge and memory subsystems for Roko.

#![deny(missing_docs)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

fn default_confidence() -> f64 {
    1.0
}

fn default_half_life_days() -> f64 {
    30.0
}

/// Semantic category for a knowledge item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeKind {
    /// A declarative statement that is treated as true until contradicted.
    Fact,
    /// A compact causal observation distilled from multiple raw episodes.
    Insight,
    /// A step-by-step action pattern or recipe.
    Procedure,
    /// A lightweight rule of thumb or learned tendency.
    Heuristic,
    /// A compiled human-readable playbook of validated heuristics.
    Playbook,
    /// A hard restriction that should not be violated.
    Constraint,
    /// Negative knowledge describing what to avoid or what has failed.
    AntiKnowledge,
}

impl Default for KnowledgeKind {
    fn default() -> Self {
        Self::Fact
    }
}

/// A durable unit of knowledge used for retrieval and memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    /// Unique identifier for the knowledge item.
    #[serde(default)]
    pub id: String,
    /// Knowledge category.
    #[serde(default)]
    pub kind: KnowledgeKind,
    /// The actual knowledge content.
    #[serde(default)]
    pub content: String,
    /// Confidence score in the range `0.0..=1.0`.
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    /// Episode IDs that contributed to this knowledge.
    #[serde(default)]
    pub source_episodes: Vec<String>,
    /// Topic tags used for retrieval and filtering.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Creation timestamp for the knowledge entry.
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    /// Exponential decay half-life in days.
    #[serde(default = "default_half_life_days")]
    pub half_life_days: f64,
    /// Optional HDC fingerprint for similarity search.
    #[serde(default)]
    pub hdc_vector: Option<Vec<u8>>,
}

pub mod knowledge_store;
/// Episode distillation into durable knowledge candidates.
pub mod distiller;
/// Tier progression from raw episodes to playbooks.
pub mod tier_progression;
/// Helpers for asynchronously processing completed episodes.
pub mod episode_completion;

pub use distiller::{DistillationBackend, Distiller};
pub use episode_completion::spawn_episode_distillation;
pub use knowledge_store::{KnowledgeStats, KnowledgeStore, DEFAULT_GC_MIN_CONFIDENCE};
#[cfg(feature = "hdc")]
pub use knowledge_store::{MemoryHit, MemoryIndex};
