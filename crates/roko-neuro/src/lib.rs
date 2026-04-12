//! Knowledge and memory subsystems for Roko.

#![deny(missing_docs)]

use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

fn default_confidence() -> f64 {
    1.0
}

fn default_confidence_weight() -> f64 {
    1.0
}

fn default_model_generality() -> f64 {
    1.0
}

const fn default_half_life_days() -> f64 {
    30.0
}

/// Default half-life for facts, in days.
pub const FACT_HALF_LIFE_DAYS: f64 = 365.0;
/// Default half-life for insights, in days.
pub const INSIGHT_HALF_LIFE_DAYS: f64 = 30.0;
/// Default half-life for heuristics, in days.
pub const HEURISTIC_HALF_LIFE_DAYS: f64 = 90.0;

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

impl KnowledgeKind {
    /// Default temporal half-life for this kind of knowledge.
    #[must_use]
    pub const fn default_half_life_days(self) -> f64 {
        match self {
            Self::Fact => FACT_HALF_LIFE_DAYS,
            Self::Insight => INSIGHT_HALF_LIFE_DAYS,
            Self::Heuristic => HEURISTIC_HALF_LIFE_DAYS,
            Self::Procedure | Self::Playbook | Self::Constraint | Self::AntiKnowledge => {
                default_half_life_days()
            }
        }
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
    /// Provenance label for the entry, if it came from a dedicated source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The actual knowledge content.
    #[serde(default)]
    pub content: String,
    /// Confidence score in the range `0.0..=1.0`.
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    /// Signed retrieval weight for the entry.
    #[serde(default = "default_confidence_weight")]
    pub confidence_weight: f64,
    /// ID of the insight this entry refutes, if it is AntiKnowledge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refuted_insight_id: Option<String>,
    /// Evidence explaining why the refuted insight was wrong.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refutation_evidence: Option<String>,
    /// Episode IDs that contributed to this knowledge.
    #[serde(default)]
    pub source_episodes: Vec<String>,
    /// Topic tags used for retrieval and filtering.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Which model originally produced the supporting episode(s), when
    /// this entry is model-specific.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_model: Option<String>,
    /// How broadly this entry applies across models (`1.0` = fully
    /// general, `0.0` = only valid for one model family).
    #[serde(default = "default_model_generality")]
    pub model_generality: f64,
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

impl KnowledgeEntry {
    /// Return the warning text for an AntiKnowledge entry, if available.
    #[must_use]
    pub fn refutation_warning(&self) -> Option<String> {
        if self.kind != KnowledgeKind::AntiKnowledge {
            return None;
        }

        let refuted_id = self.refuted_insight_id.as_deref()?.trim();
        if refuted_id.is_empty() {
            return None;
        }

        let evidence = self
            .refutation_evidence
            .as_deref()
            .unwrap_or(self.content.as_str())
            .trim()
            .trim_end_matches(|ch| matches!(ch, '.' | '!' | '?'));
        if evidence.is_empty() {
            return None;
        }

        Some(format!(
            "Previous insight {refuted_id} was wrong because {evidence}."
        ))
    }

    /// Return whether the entry should be injected for `current_model`.
    #[must_use]
    pub fn applies_to_model(&self, current_model: &str) -> bool {
        self.model_generality > 0.7 || self.source_model.as_deref() == Some(current_model)
    }
}

/// Single entry point for durable knowledge storage backends.
pub trait NeuroStore: Sized {
    /// Initialize a store at the given path.
    fn init(path: &Path) -> Result<Self>;

    /// Query a topic for relevant knowledge entries.
    fn query(&self, topic: &str, limit: usize) -> Result<Vec<KnowledgeEntry>>;

    /// Ingest a batch of knowledge entries.
    fn ingest(&mut self, entries: Vec<KnowledgeEntry>) -> Result<()>;

    /// Apply decay and return the number of entries processed.
    fn decay(&mut self) -> Result<usize>;

    /// Garbage-collect low-confidence entries and return the number removed.
    fn gc(&mut self, min_confidence: f64) -> Result<usize>;
}

pub mod context;
/// Episode distillation into durable knowledge candidates.
pub mod distiller;
/// Helpers for asynchronously processing completed episodes.
pub mod episode_completion;
pub mod knowledge_store;
/// Tier progression from raw episodes to playbooks.
pub mod tier_progression;

pub use context::{ContextAssembler, EpisodeStore};
pub use distiller::{DistillationBackend, Distiller};
pub use episode_completion::spawn_episode_distillation;
pub use knowledge_store::{
    DEFAULT_GC_MIN_CONFIDENCE, KnowledgeConfirmationRecord, KnowledgeStats, KnowledgeStore,
};
#[cfg(feature = "hdc")]
pub use knowledge_store::{MemoryHit, MemoryIndex};
