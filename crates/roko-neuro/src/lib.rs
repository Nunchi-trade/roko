//! Knowledge and memory subsystems for Roko.

#![deny(missing_docs)]

use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};
use roko_core::{EmotionalTag, PadVector};
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

/// Default half-life for insights, in days.
pub const INSIGHT_HALF_LIFE_DAYS: f64 = 30.0;
/// Default half-life for heuristics, in days.
pub const HEURISTIC_HALF_LIFE_DAYS: f64 = 90.0;
/// Default half-life for warnings, in days.
pub const WARNING_HALF_LIFE_DAYS: f64 = 7.0;
/// Default half-life for causal links, in days.
pub const CAUSAL_LINK_HALF_LIFE_DAYS: f64 = 60.0;
/// Default half-life for strategy fragments, in days.
pub const STRATEGY_FRAGMENT_HALF_LIFE_DAYS: f64 = 14.0;

/// Semantic category for a knowledge item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeKind {
    /// A compact causal observation distilled from multiple raw episodes.
    #[serde(alias = "fact", alias = "Fact")]
    Insight,
    /// A lightweight rule of thumb or learned tendency.
    #[serde(alias = "procedure", alias = "Procedure")]
    Heuristic,
    /// Negative knowledge describing what to avoid or what has failed.
    AntiKnowledge,
    /// A cautionary warning about a recurring failure mode or risk.
    #[serde(alias = "constraint", alias = "Constraint")]
    Warning,
    /// A causal relationship between two observations.
    CausalLink,
    /// A reusable approach fragment that can be composed into a larger plan.
    #[serde(alias = "playbook", alias = "Playbook")]
    StrategyFragment,
}

impl Default for KnowledgeKind {
    fn default() -> Self {
        Self::Insight
    }
}

impl KnowledgeKind {
    /// Default temporal half-life for this kind of knowledge.
    #[must_use]
    pub const fn default_half_life_days(self) -> f64 {
        match self {
            Self::Insight => INSIGHT_HALF_LIFE_DAYS,
            Self::Heuristic => HEURISTIC_HALF_LIFE_DAYS,
            Self::AntiKnowledge => default_half_life_days(),
            Self::Warning => WARNING_HALF_LIFE_DAYS,
            Self::CausalLink => CAUSAL_LINK_HALF_LIFE_DAYS,
            Self::StrategyFragment => STRATEGY_FRAGMENT_HALF_LIFE_DAYS,
        }
    }

    /// Stable string label for this knowledge kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Insight => "insight",
            Self::Heuristic => "heuristic",
            Self::AntiKnowledge => "anti_knowledge",
            Self::Warning => "warning",
            Self::CausalLink => "causal_link",
            Self::StrategyFragment => "strategy_fragment",
        }
    }
}

/// Retention tier for a knowledge entry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeTier {
    /// Short-lived entry that should decay aggressively.
    #[default]
    Transient,
    /// Active working memory worth keeping somewhat longer.
    Working,
    /// Validated knowledge that should decay at the base rate.
    Consolidated,
    /// Highly durable knowledge that should decay much more slowly.
    Persistent,
}

impl KnowledgeTier {
    /// Lifetime multiplier for the tier.
    #[must_use]
    pub const fn multiplier(&self) -> f32 {
        match self {
            Self::Transient => 0.1,
            Self::Working => 0.5,
            Self::Consolidated => 1.0,
            Self::Persistent => 5.0,
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
    /// Retention tier applied on top of the base half-life.
    #[serde(default)]
    pub tier: KnowledgeTier,
    /// Optional affect provenance transferred from supporting episodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotional_tag: Option<EmotionalTag>,
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

    /// Effective half-life after applying the retention tier multiplier.
    #[must_use]
    pub fn effective_half_life_days(&self) -> f64 {
        let base_half_life = if self.half_life_days.is_finite() && self.half_life_days > 0.0 {
            self.half_life_days
        } else {
            self.kind.default_half_life_days()
        };
        base_half_life * self.tier.multiplier() as f64
    }

    /// PAD vector used for mood-congruent retrieval.
    #[must_use]
    pub fn affect_pad(&self) -> PadVector {
        self.emotional_tag
            .as_ref()
            .map(|tag| tag.mood_snapshot)
            .unwrap_or_else(PadVector::neutral)
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
#[cfg(feature = "hdc")]
mod hdc;
pub mod knowledge_store;
/// Tier progression from raw episodes to playbooks.
pub mod tier_progression;

pub use context::{
    ContextAssembler, ContextChunk, ContextSource, EpisodeStore, PadState, ReadFileSpec, TaskInput,
    VerifySpec,
};
pub use distiller::{DistillationBackend, Distiller};
pub use episode_completion::spawn_episode_distillation;
pub use knowledge_store::{
    DEFAULT_GC_MIN_CONFIDENCE, KnowledgeConfirmationRecord, KnowledgeStats, KnowledgeStore,
};
#[cfg(feature = "hdc")]
pub use knowledge_store::{MemoryHit, MemoryIndex};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knowledge_tier_multiplier_matches_spec() {
        assert_eq!(KnowledgeTier::Transient.multiplier(), 0.1);
        assert_eq!(KnowledgeTier::Working.multiplier(), 0.5);
        assert_eq!(KnowledgeTier::Consolidated.multiplier(), 1.0);
        assert_eq!(KnowledgeTier::Persistent.multiplier(), 5.0);
    }

    #[test]
    fn effective_half_life_applies_tier_multiplier() {
        let entry = KnowledgeEntry {
            id: "kn-1".to_string(),
            kind: KnowledgeKind::Insight,
            source: None,
            content: "Prefer smaller retries after gate failures.".to_string(),
            confidence: 0.9,
            confidence_weight: 0.9,
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes: vec!["ep-1".to_string()],
            tags: vec!["insight".to_string()],
            source_model: None,
            model_generality: 1.0,
            created_at: Utc::now(),
            half_life_days: 20.0,
            tier: KnowledgeTier::Persistent,
            emotional_tag: None,
            hdc_vector: None,
        };

        assert_eq!(entry.effective_half_life_days(), 100.0);
    }

    #[test]
    fn new_knowledge_kinds_have_expected_defaults() {
        assert_eq!(KnowledgeKind::Warning.default_half_life_days(), 7.0);
        assert_eq!(KnowledgeKind::CausalLink.default_half_life_days(), 60.0);
        assert_eq!(
            KnowledgeKind::StrategyFragment.default_half_life_days(),
            14.0
        );
        assert_eq!(KnowledgeKind::Warning.as_str(), "warning");
        assert_eq!(KnowledgeKind::CausalLink.as_str(), "causal_link");
        assert_eq!(
            KnowledgeKind::StrategyFragment.as_str(),
            "strategy_fragment"
        );
    }

    #[test]
    fn legacy_knowledge_kind_names_deserialize_to_prd_variants() {
        #[derive(Deserialize)]
        struct Wrapper {
            kind: KnowledgeKind,
        }

        let cases = [
            (r#"{"kind":"Fact"}"#, KnowledgeKind::Insight),
            (r#"{"kind":"fact"}"#, KnowledgeKind::Insight),
            (r#"{"kind":"Procedure"}"#, KnowledgeKind::Heuristic),
            (r#"{"kind":"procedure"}"#, KnowledgeKind::Heuristic),
            (r#"{"kind":"Playbook"}"#, KnowledgeKind::StrategyFragment),
            (r#"{"kind":"playbook"}"#, KnowledgeKind::StrategyFragment),
            (r#"{"kind":"Constraint"}"#, KnowledgeKind::Warning),
            (r#"{"kind":"constraint"}"#, KnowledgeKind::Warning),
        ];

        for (json, expected) in cases {
            let decoded: Wrapper = serde_json::from_str(json).expect("deserialize legacy kind");
            assert_eq!(decoded.kind, expected);
        }
    }
}
