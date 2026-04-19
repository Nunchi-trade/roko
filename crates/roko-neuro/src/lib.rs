//! Knowledge and memory subsystems for Roko.

#![deny(missing_docs)]
#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::clone_on_copy,
    clippy::default_trait_access,
    clippy::derivable_impls,
    clippy::derive_partial_eq_without_eq,
    clippy::doc_markdown,
    clippy::format_push_string,
    clippy::implicit_clone,
    clippy::manual_pattern_char_comparison,
    clippy::map_unwrap_or,
    clippy::match_same_arms,
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_value,
    clippy::option_if_let_else,
    clippy::ptr_arg,
    clippy::redundant_clone,
    clippy::redundant_closure,
    clippy::redundant_closure_for_method_calls,
    clippy::suboptimal_flops,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::unused_self,
    clippy::unwrap_or_default,
    clippy::use_self
)]

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

/// Narrative shape of how emotionally tagged evidence evolved over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationArc {
    /// Negative or failure-heavy evidence that later resolved positively.
    Redemptive,
    /// Initially positive evidence that degraded into a negative outcome.
    Contaminating,
    /// Mostly steady evidence without a strong directional change.
    Stable,
    /// Gradual improvement without a sharp redemption boundary.
    Progressive,
}

/// Emotional reliability metadata derived from the supporting episodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmotionalProvenance {
    /// Mean PAD signal across the supporting episodes.
    pub average_pad: PadVector,
    /// Coarse PAD-derived emotional label at first discovery.
    pub discovery_emotion: String,
    /// Narrative arc inferred from the emotional trajectory over time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_arc: Option<ValidationArc>,
    /// Normalized Shannon entropy across coarse emotional labels.
    pub emotional_diversity: f64,
}

impl EmotionalProvenance {
    /// Build provenance metadata from a single emotional observation.
    #[must_use]
    pub fn from_tag(tag: &EmotionalTag) -> Self {
        Self {
            average_pad: tag.pad,
            discovery_emotion: Self::coarse_emotion_label(tag.pad),
            validation_arc: None,
            emotional_diversity: 0.0,
        }
    }

    /// Derive a coarse PAD-based emotional bucket.
    #[must_use]
    pub fn coarse_emotion_label(pad: PadVector) -> String {
        let valence = if pad.pleasure >= 0.2 {
            "positive"
        } else if pad.pleasure <= -0.2 {
            "negative"
        } else {
            "neutral"
        };
        let arousal = if pad.arousal >= 0.35 {
            "high_arousal"
        } else if pad.arousal <= -0.35 {
            "low_arousal"
        } else {
            "mid_arousal"
        };
        format!("{valence}_{arousal}")
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
    /// Optional emotional reliability metadata derived from the support set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotional_provenance: Option<EmotionalProvenance>,
    /// Optional HDC fingerprint for similarity search.
    #[serde(default)]
    pub hdc_vector: Option<Vec<u8>>,
    /// Number of independent confirmations from different episodes.
    /// Used for tier promotion: 2+ for Transient->Working.
    #[serde(default)]
    pub confirmation_count: u32,
    /// Distinct context IDs (e.g. plan/task combos) that confirmed this entry.
    /// Used for tier promotion: 3+ distinct contexts for Working->Consolidated.
    #[serde(default)]
    pub distinct_contexts: Vec<String>,
    /// Whether this entry has been explicitly deprecated.
    /// Required for demoting Persistent entries.
    #[serde(default)]
    pub deprecated: bool,
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

    /// Consolidation multiplier derived from emotional provenance.
    ///
    /// Entries that were validated across varied emotional states and
    /// resolved through a redemptive or progressive arc are retained
    /// slightly more aggressively.
    #[must_use]
    pub fn emotional_consolidation_boost(&self) -> f64 {
        let mut boost = 1.0;

        if let Some(provenance) = self.emotional_provenance.as_ref() {
            boost *= 1.0 + provenance.emotional_diversity.clamp(0.0, 1.0) * 0.15;
            boost *= match provenance.validation_arc {
                Some(ValidationArc::Redemptive) => 1.06,
                Some(ValidationArc::Progressive) => 1.04,
                Some(ValidationArc::Stable) | None => 1.0,
                Some(ValidationArc::Contaminating) => 0.94,
            };
        }

        if let Some(tag) = self.emotional_tag.as_ref() {
            boost *= 1.0 + f64::from(tag.intensity).clamp(0.0, 1.0) * 0.05;
        }

        boost.max(0.1)
    }

    /// Retrieval multiplier derived from emotional congruence and intensity.
    ///
    /// This is intentionally a little stronger than the consolidation boost
    /// because retrieval should surface affect-laden knowledge sooner when it
    /// matches the current search conditions.
    #[must_use]
    pub fn emotional_retrieval_boost(&self) -> f64 {
        let mut boost = self.emotional_consolidation_boost();

        if let Some(tag) = self.emotional_tag.as_ref() {
            boost *= 1.0 + f64::from(tag.intensity).clamp(0.0, 1.0) * 0.08;
        }

        boost.max(0.1)
    }

    /// Backwards-compatible emotional reliability boost used by older call sites.
    #[must_use]
    pub fn emotional_reliability_boost(&self) -> f64 {
        self.emotional_consolidation_boost()
    }
}

// ---------------------------------------------------------------------------
// NEURO-07: Source channel confidence discounting.
// ---------------------------------------------------------------------------

/// Provenance channel for ingested knowledge entries.
///
/// Each channel carries a different trust discount reflecting the reliability
/// of the data source. On ingest, the entry's confidence is multiplied by
/// the channel's discount factor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceChannel {
    /// Direct user input -- fully trusted.
    UserInput,
    /// Gate verdicts from the validation pipeline.
    GateVerdict,
    /// Output produced by an LLM agent.
    AgentOutput,
    /// Data retrieved from an external API.
    ExternalApi,
    /// Speculative knowledge from dream consolidation.
    DreamConsolidation,
}

impl SourceChannel {
    /// Default trust discount factor for this channel.
    ///
    /// The entry's raw confidence is multiplied by this value on ingest.
    #[must_use]
    pub const fn discount_factor(self) -> f64 {
        match self {
            Self::UserInput => 1.0,
            Self::GateVerdict => 0.95,
            Self::AgentOutput => 0.8,
            Self::ExternalApi => 0.6,
            Self::DreamConsolidation => 0.5,
        }
    }

    /// Apply the channel's discount to a raw confidence value.
    #[must_use]
    pub fn apply(self, confidence: f64) -> f64 {
        (confidence * self.discount_factor()).clamp(0.0, 1.0)
    }

    /// Infer the source channel from a source label string.
    #[must_use]
    pub fn from_source_label(label: &str) -> Self {
        let normalized = label.trim().to_ascii_lowercase();
        if normalized.contains("user") || normalized.contains("manual") {
            Self::UserInput
        } else if normalized.contains("gate") || normalized.contains("verdict") {
            Self::GateVerdict
        } else if normalized.contains("agent") || normalized.contains("llm") {
            Self::AgentOutput
        } else if normalized.contains("api") || normalized.contains("external") {
            Self::ExternalApi
        } else if normalized.contains("dream") || normalized.contains("consolidat") {
            Self::DreamConsolidation
        } else {
            // Default to agent output for unknown sources.
            Self::AgentOutput
        }
    }
}

/// Apply source-channel confidence discounting to a batch of entries.
///
/// Each entry's confidence is multiplied by the discount factor of the
/// given source channel.
pub fn apply_source_discount(entries: &mut [KnowledgeEntry], channel: SourceChannel) {
    let factor = channel.discount_factor();
    for entry in entries.iter_mut() {
        entry.confidence = (entry.confidence * factor).clamp(0.0, 1.0);
    }
}

// ---------------------------------------------------------------------------
// NEURO-08: Worldview clustering and cold-tier preservation.
// ---------------------------------------------------------------------------

/// A cluster of related knowledge entries grouped by tag similarity.
///
/// During garbage collection, if an entry is the last representative of
/// its worldview cluster, it is preserved to prevent losing an entire
/// conceptual domain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldviewCluster {
    /// Stable identifier for this cluster.
    pub id: String,
    /// Representative tags that define this worldview.
    pub representative_tags: Vec<String>,
    /// Number of entries currently in this cluster.
    pub entry_count: usize,
}

/// Assign each knowledge entry to a worldview cluster based on tag overlap.
///
/// Two entries share a cluster when they have at least `min_tag_overlap`
/// tags in common. The algorithm uses a simple union-find approach:
/// entries with shared tags get merged into the same cluster.
#[must_use]
pub fn cluster_worldviews(
    entries: &[KnowledgeEntry],
    min_tag_overlap: usize,
) -> Vec<WorldviewCluster> {
    fn uf_find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]]; // path compression
            i = parent[i];
        }
        i
    }

    fn uf_union(parent: &mut [usize], a: usize, b: usize) {
        let ra = uf_find(parent, a);
        let rb = uf_find(parent, b);
        if ra != rb {
            parent[rb] = ra;
        }
    }

    if entries.is_empty() {
        return Vec::new();
    }

    let min_overlap = min_tag_overlap.max(1);
    let mut parent: Vec<usize> = (0..entries.len()).collect();

    // O(n^2) pairwise tag overlap -- acceptable for typical knowledge store sizes.
    for i in 0..entries.len() {
        for j in (i + 1)..entries.len() {
            let overlap = entries[i]
                .tags
                .iter()
                .filter(|tag| entries[j].tags.contains(tag))
                .count();
            if overlap >= min_overlap {
                uf_union(&mut parent, i, j);
            }
        }
    }

    // Group entries by their root representative.
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    for i in 0..entries.len() {
        let root = uf_find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }

    groups
        .into_iter()
        .map(|(root, members)| {
            // Collect the union of all tags in this cluster.
            let mut all_tags: Vec<String> = Vec::new();
            for &idx in &members {
                for tag in &entries[idx].tags {
                    if !all_tags.contains(tag) {
                        all_tags.push(tag.clone());
                    }
                }
            }
            all_tags.sort();

            WorldviewCluster {
                id: format!("wv-{}", entries[root].id),
                representative_tags: all_tags,
                entry_count: members.len(),
            }
        })
        .collect()
}

/// Filter entries for GC, preserving the last representative of each
/// worldview cluster to prevent losing entire conceptual domains.
///
/// Returns entries that should be retained (those above threshold plus
/// any "last survivor" entries from worldview clusters).
#[must_use]
pub fn gc_with_worldview_preservation(
    entries: Vec<KnowledgeEntry>,
    min_confidence: f64,
    min_tag_overlap: usize,
) -> Vec<KnowledgeEntry> {
    fn uf_find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }

    fn uf_union(parent: &mut [usize], a: usize, b: usize) {
        let ra = uf_find(parent, a);
        let rb = uf_find(parent, b);
        if ra != rb {
            parent[rb] = ra;
        }
    }

    if entries.is_empty() {
        return Vec::new();
    }

    let threshold = min_confidence.max(0.0);

    // Determine which entries survive the confidence threshold.
    let mut surviving_indices: Vec<bool> = entries
        .iter()
        .map(|entry| {
            entry.kind == KnowledgeKind::AntiKnowledge
                || effective_confidence_for_gc(entry) >= threshold
        })
        .collect();

    // Build union-find clusters from tag overlap.
    let mut parent: Vec<usize> = (0..entries.len()).collect();
    let min_overlap = min_tag_overlap.max(1);
    for i in 0..entries.len() {
        for j in (i + 1)..entries.len() {
            let overlap = entries[i]
                .tags
                .iter()
                .filter(|tag| entries[j].tags.contains(tag))
                .count();
            if overlap >= min_overlap {
                uf_union(&mut parent, i, j);
            }
        }
    }

    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    for i in 0..entries.len() {
        let root = uf_find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }

    // For each cluster, if no entry survives, preserve the best one.
    for members in groups.values() {
        let any_survive = members.iter().any(|&idx| surviving_indices[idx]);
        if !any_survive && !members.is_empty() {
            let best = members
                .iter()
                .copied()
                .max_by(|&a, &b| {
                    entries[a]
                        .confidence
                        .partial_cmp(&entries[b].confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or(members[0]);
            surviving_indices[best] = true;
        }
    }

    entries
        .into_iter()
        .zip(surviving_indices)
        .filter(|(_, survives)| *survives)
        .map(|(entry, _)| entry)
        .collect()
}

/// Effective confidence used for GC threshold comparison.
///
/// This mirrors the knowledge store's internal effective_confidence logic.
fn effective_confidence_for_gc(entry: &KnowledgeEntry) -> f64 {
    let base = entry.confidence.max(0.0);
    let boost = entry.emotional_consolidation_boost();
    let confirmation_factor = if entry.confirmation_count >= 2 {
        1.5
    } else {
        1.0
    };
    base * boost * confirmation_factor
}

/// Single entry point for durable knowledge storage backends.
pub trait NeuroStore: Sized {
    /// Initialize a store at the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot initialize or load its durable
    /// state from `path`.
    fn init(path: &Path) -> Result<Self>;

    /// Query a topic for relevant knowledge entries.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot read or decode the stored
    /// knowledge entries needed to answer the query.
    fn query(&self, topic: &str, limit: usize) -> Result<Vec<KnowledgeEntry>>;

    /// Query by a serialized 10,240-bit fingerprint and return the nearest
    /// durable records.
    ///
    /// # Errors
    ///
    /// Returns an error if the fingerprint length is invalid or the backend
    /// cannot read the stored knowledge entries needed to answer the query.
    fn query_similar(
        &self,
        fingerprint: &[u8],
        limit: usize,
    ) -> Result<Vec<crate::knowledge_store::KnowledgeSimilarityHit>>;

    /// Ingest a batch of knowledge entries.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot persist the provided entries.
    fn ingest(&mut self, entries: Vec<KnowledgeEntry>) -> Result<()>;

    /// Apply decay and return the number of entries processed.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot load or persist the decayed
    /// entries.
    fn decay(&mut self) -> Result<usize>;

    /// Garbage-collect low-confidence entries and return the number removed.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot load or persist the filtered
    /// entries.
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
    AntiKnowledgeConflict, BackupHeader, DEFAULT_GC_MIN_CONFIDENCE, ExportFilter, ImportOptions,
    KnowledgeConfirmationRecord, KnowledgeQueryBreakdown, KnowledgeQueryHit,
    KnowledgeSimilarityHit, KnowledgeStats, KnowledgeStore, QUERY_SCORE_FLOOR,
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
            emotional_provenance: None,
            hdc_vector: None,

            confirmation_count: 0,

            distinct_contexts: Vec::new(),

            deprecated: false,
        };

        assert_eq!(entry.effective_half_life_days(), 100.0);
    }

    #[test]
    fn missing_knowledge_tier_defaults_to_transient() {
        #[derive(Deserialize)]
        struct Wrapper {
            entry: KnowledgeEntry,
        }

        let decoded: Wrapper = serde_json::from_str(
            r#"{
                "entry": {
                    "kind": "insight",
                    "content": "Keep the default tier small.",
                    "confidence": 0.8,
                    "source_episodes": ["ep-1"],
                    "tags": ["memory"],
                    "half_life_days": 12.0
                }
            }"#,
        )
        .expect("deserialize entry");

        assert_eq!(decoded.entry.tier, KnowledgeTier::Transient);
        assert!((decoded.entry.effective_half_life_days() - 1.2).abs() < 1e-6);
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
