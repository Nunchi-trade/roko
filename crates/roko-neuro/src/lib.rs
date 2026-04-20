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

fn default_balance() -> f64 {
    1.0
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
    /// NEURO-10: Freshness reserve balance for demurrage model.
    ///
    /// Balance represents the entry's freshness reserve. It decreases via
    /// demurrage tax over time and increases via reinforcement signals
    /// (Retrieved, Cited, Gated, Surprised, AgentQuoted). Initial value 1.0.
    #[serde(default = "default_balance")]
    pub balance: f64,
    /// NEURO-11: Whether this entry has been frozen into cold storage.
    ///
    /// Frozen entries are excluded from hot query results but retain their
    /// content address, lineage, and provenance. They can be thawed to
    /// restore a starter balance.
    #[serde(default)]
    pub frozen: bool,
}

impl Default for KnowledgeEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: KnowledgeKind::default(),
            source: None,
            content: String::new(),
            confidence: default_confidence(),
            confidence_weight: default_confidence_weight(),
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes: Vec::new(),
            tags: Vec::new(),
            source_model: None,
            model_generality: default_model_generality(),
            created_at: Utc::now(),
            half_life_days: default_half_life_days(),
            tier: KnowledgeTier::default(),
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,
            confirmation_count: 0,
            distinct_contexts: Vec::new(),
            deprecated: false,
            balance: default_balance(),
            frozen: false,
        }
    }
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

    /// NEURO-10: Apply a reinforcement signal to bump this entry's balance.
    ///
    /// The bump is `signal.base_value() * (1.0 + novelty)` where `novelty`
    /// is typically `1.0 - max_hdc_similarity` against top-K neighbors.
    /// Common entries get small bumps; rare-but-useful ones get larger bumps.
    pub fn reinforce(&mut self, signal: ReinforcementSignal, novelty: f64) {
        let bump = signal.base_value() * (1.0 + novelty.clamp(0.0, 1.0));
        self.balance = (self.balance + bump).min(5.0);
    }

    /// NEURO-10: Apply demurrage tax, deducting balance proportionally to
    /// elapsed time.
    ///
    /// The demurrage rate is `DEMURRAGE_RATE_PER_HOUR` (default 0.005).
    pub fn apply_demurrage(&mut self, elapsed_hours: f64) {
        if elapsed_hours <= 0.0 {
            return;
        }
        let deduction = DEMURRAGE_RATE_PER_HOUR * elapsed_hours;
        self.balance = (self.balance - deduction).max(0.0);
    }

    /// NEURO-10: Freshness score combining balance with Ebbinghaus decay.
    ///
    /// `freshness(t) = balance(t) * ebbinghaus_weight(age, type_half_life, tier_multiplier)`
    #[must_use]
    pub fn freshness(&self, now: DateTime<Utc>) -> f64 {
        let age_hours = now
            .signed_duration_since(self.created_at)
            .num_seconds() as f64
            / 3600.0;
        if age_hours <= 0.0 {
            return self.balance;
        }
        let half_life_hours = self.effective_half_life_days() * 24.0;
        let ebbinghaus = if half_life_hours > 0.0 {
            (-(age_hours * 2.0_f64.ln()) / half_life_hours).exp()
        } else {
            0.0
        };
        self.balance * ebbinghaus
    }

    /// NEURO-11: Freeze this entry into cold storage.
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    /// NEURO-11: Thaw this entry from cold storage with a starter balance.
    pub fn thaw(&mut self, starter_balance: f64) {
        self.frozen = false;
        self.balance = starter_balance.clamp(0.0, 5.0);
    }
}

// ---------------------------------------------------------------------------
// NEURO-10: Demurrage balance model and reinforcement signals.
// ---------------------------------------------------------------------------

/// Hourly demurrage rate deducted from knowledge entry balances.
pub const DEMURRAGE_RATE_PER_HOUR: f64 = 0.005;

/// Default balance floor below which entries are frozen by GC.
pub const BALANCE_GC_FLOOR: f64 = 0.05;

/// Default starter balance for thawed entries.
pub const THAW_STARTER_BALANCE: f64 = 0.3;

/// Reinforcement signal types that bump a knowledge entry's balance.
///
/// The 5 signals correspond to different ways knowledge proves its worth:
/// being retrieved, being cited, surviving a gate check, explaining a novel
/// outcome, or being explicitly reused by an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReinforcementSignal {
    /// Entry was selected for use during context assembly.
    Retrieved,
    /// Another entry references this one.
    Cited,
    /// Entry survived a verification gate.
    Gated,
    /// Entry explained a novel or unexpected outcome.
    Surprised,
    /// Agent explicitly reused this entry's content.
    AgentQuoted,
}

impl ReinforcementSignal {
    /// Base balance bump for each signal type.
    #[must_use]
    pub const fn base_value(self) -> f64 {
        match self {
            Self::Retrieved => 0.05,
            Self::Cited => 0.08,
            Self::Gated => 0.10,
            Self::Surprised => 0.15,
            Self::AgentQuoted => 0.12,
        }
    }

    /// Human-readable label for this signal type.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Retrieved => "retrieved",
            Self::Cited => "cited",
            Self::Gated => "gated",
            Self::Surprised => "surprised",
            Self::AgentQuoted => "agent_quoted",
        }
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

// ---------------------------------------------------------------------------
// NEURO-09: Distillation D2 HDC clustering.
// ---------------------------------------------------------------------------

/// A cluster of knowledge entries grouped by HDC vector similarity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HdcCluster {
    /// Cluster identifier (0-indexed).
    pub id: usize,
    /// Entry IDs belonging to this cluster.
    pub entry_ids: Vec<String>,
    /// Number of entries in this cluster.
    pub entry_count: usize,
}

/// Group knowledge entries by HDC vector similarity using k-means with
/// Hamming distance.
///
/// Entries without a valid HDC vector are assigned to cluster 0.
/// The algorithm runs for at most 20 iterations or until convergence.
#[must_use]
pub fn hdc_cluster(entries: &[KnowledgeEntry], k: usize) -> Vec<HdcCluster> {
    const HDC_BYTES: usize = 1280;
    const MAX_ITERATIONS: usize = 20;

    if entries.is_empty() || k == 0 {
        return Vec::new();
    }

    let k = k.min(entries.len());

    // Extract valid HDC vectors; entries without vectors get mapped to cluster 0.
    let vectors: Vec<Option<&[u8]>> = entries
        .iter()
        .map(|e| {
            e.hdc_vector
                .as_deref()
                .filter(|v| v.len() == HDC_BYTES)
        })
        .collect();

    // Count entries with valid vectors.
    let valid_count = vectors.iter().filter(|v| v.is_some()).count();
    if valid_count == 0 || k <= 1 {
        return vec![HdcCluster {
            id: 0,
            entry_ids: entries.iter().map(|e| e.id.clone()).collect(),
            entry_count: entries.len(),
        }];
    }

    // Initialize centroids: pick first k entries with valid vectors.
    let mut centroids: Vec<Vec<u8>> = vectors
        .iter()
        .filter_map(|v| v.map(|bytes| bytes.to_vec()))
        .take(k)
        .collect();

    let mut assignments = vec![0_usize; entries.len()];

    for _iter in 0..MAX_ITERATIONS {
        let mut changed = false;

        // Assignment step: assign each entry to nearest centroid by Hamming distance.
        for (i, vec_opt) in vectors.iter().enumerate() {
            let Some(vec) = vec_opt else {
                if assignments[i] != 0 {
                    assignments[i] = 0;
                    changed = true;
                }
                continue;
            };

            let mut best_cluster = 0;
            let mut best_dist = u32::MAX;
            for (c, centroid) in centroids.iter().enumerate() {
                let dist: u32 = vec
                    .iter()
                    .zip(centroid.iter())
                    .map(|(a, b)| (a ^ b).count_ones())
                    .sum();
                if dist < best_dist {
                    best_dist = dist;
                    best_cluster = c;
                }
            }

            if assignments[i] != best_cluster {
                assignments[i] = best_cluster;
                changed = true;
            }
        }

        if !changed {
            break;
        }

        // Update step: majority vote for each centroid bit.
        for (c, centroid) in centroids.iter_mut().enumerate() {
            let members: Vec<&[u8]> = assignments
                .iter()
                .zip(vectors.iter())
                .filter_map(|(&a, v)| {
                    if a == c {
                        v.as_deref()
                    } else {
                        None
                    }
                })
                .collect();

            if members.is_empty() {
                continue;
            }

            for byte_idx in 0..HDC_BYTES {
                let mut new_byte = 0u8;
                for bit in 0..8 {
                    let ones: usize = members
                        .iter()
                        .filter(|m| (m[byte_idx] >> bit) & 1 == 1)
                        .count();
                    if ones * 2 > members.len() {
                        new_byte |= 1 << bit;
                    }
                }
                centroid[byte_idx] = new_byte;
            }
        }
    }

    // Build cluster output.
    let mut clusters: std::collections::BTreeMap<usize, Vec<String>> =
        std::collections::BTreeMap::new();
    for (i, &cluster_id) in assignments.iter().enumerate() {
        clusters
            .entry(cluster_id)
            .or_default()
            .push(entries[i].id.clone());
    }

    clusters
        .into_iter()
        .map(|(id, entry_ids)| HdcCluster {
            id,
            entry_count: entry_ids.len(),
            entry_ids,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// NEURO-10: Demurrage balance model for knowledge entries.
// ---------------------------------------------------------------------------

/// Apply demurrage (relevance decay) to knowledge entries.
///
/// Each entry loses `decay_rate` fraction of its confidence per tick unless
/// actively referenced. Returns the number of entries that had their
/// confidence reduced.
#[must_use]
pub fn apply_demurrage(entries: &mut [KnowledgeEntry], decay_rate: f64) -> usize {
    let rate = decay_rate.clamp(0.0, 1.0);
    let mut count = 0;
    for entry in entries.iter_mut() {
        let old = entry.confidence;
        entry.confidence = (entry.confidence * (1.0 - rate)).max(0.0);
        if (old - entry.confidence).abs() > f64::EPSILON {
            count += 1;
        }
    }
    count
}

/// Boost a knowledge entry's confidence when it is actively referenced.
///
/// This is the counterpart to demurrage: entries that are used regain
/// relevance. The boost is additive and capped at 1.0.
pub fn demurrage_reference_boost(entry: &mut KnowledgeEntry, boost: f64) {
    entry.confidence = (entry.confidence + boost.abs()).min(1.0);
}

// ---------------------------------------------------------------------------
// NEURO-11: Cold-tier freeze/thaw.
// ---------------------------------------------------------------------------

/// Freeze a knowledge entry for cold storage.
///
/// Frozen entries are marked with `deprecated = true` and their confidence
/// is preserved but they should not appear in active search results.
/// The original confidence is recorded in a tag for thaw restoration.
pub fn freeze_entry(entry: &mut KnowledgeEntry) {
    if entry.deprecated {
        return; // Already frozen.
    }
    // Record pre-freeze confidence as a tag for thaw.
    entry
        .tags
        .push(format!("__frozen_confidence:{:.6}", entry.confidence));
    entry.deprecated = true;
}

/// Thaw a frozen knowledge entry, restoring it to active status.
///
/// Restores the pre-freeze confidence from the saved tag.
/// Returns `true` if the entry was actually thawed.
pub fn thaw_entry(entry: &mut KnowledgeEntry) -> bool {
    if !entry.deprecated {
        return false; // Not frozen.
    }

    // Restore pre-freeze confidence.
    let mut restored_confidence = None;
    entry.tags.retain(|tag| {
        if let Some(val) = tag.strip_prefix("__frozen_confidence:") {
            restored_confidence = val.parse::<f64>().ok();
            false
        } else {
            true
        }
    });

    if let Some(conf) = restored_confidence {
        entry.confidence = conf.clamp(0.0, 1.0);
    }
    entry.deprecated = false;
    true
}

/// Check if a knowledge entry is frozen (in cold storage).
#[must_use]
pub fn is_frozen(entry: &KnowledgeEntry) -> bool {
    entry.deprecated && entry.tags.iter().any(|t| t.starts_with("__frozen_confidence:"))
}

// ---------------------------------------------------------------------------
// NEURO-12: Falsifier records.
// ---------------------------------------------------------------------------

/// Record tracking when a knowledge entry is contradicted by new evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FalsifierRecord {
    /// ID of the knowledge entry that was falsified.
    pub entry_id: String,
    /// Evidence text explaining the contradiction.
    pub evidence: String,
    /// Timestamp when the falsification was recorded.
    pub falsified_at_ms: i64,
}

/// Append a falsification record to a collection and mark the entry.
///
/// When a falsified entry is later referenced, callers should check the
/// records and surface a warning.
pub fn record_falsification(
    records: &mut Vec<FalsifierRecord>,
    entries: &mut [KnowledgeEntry],
    entry_id: &str,
    evidence: &str,
) {
    records.push(FalsifierRecord {
        entry_id: entry_id.to_string(),
        evidence: evidence.to_string(),
        falsified_at_ms: Utc::now().timestamp_millis(),
    });

    // Halve the confidence of the falsified entry.
    if let Some(entry) = entries.iter_mut().find(|e| e.id == entry_id) {
        entry.confidence *= 0.5;
    }
}

/// Check if a knowledge entry has been falsified and return the warning.
#[must_use]
pub fn falsification_warning<'a>(
    records: &'a [FalsifierRecord],
    entry_id: &str,
) -> Option<&'a FalsifierRecord> {
    records.iter().find(|r| r.entry_id == entry_id)
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
            balance: 1.0,
            frozen: false,
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
    fn hdc_cluster_single_cluster_when_k1() {
        let entries = vec![
            KnowledgeEntry {
                id: "e1".to_string(),
                kind: KnowledgeKind::Insight,
                content: "test".to_string(),
                confidence: 0.8,
                ..Default::default()
            },
            KnowledgeEntry {
                id: "e2".to_string(),
                kind: KnowledgeKind::Heuristic,
                content: "test2".to_string(),
                confidence: 0.7,
                ..Default::default()
            },
        ];
        let clusters = hdc_cluster(&entries, 1);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].entry_count, 2);
    }

    #[test]
    fn hdc_cluster_empty_input() {
        let clusters = hdc_cluster(&[], 3);
        assert!(clusters.is_empty());
    }

    #[test]
    fn demurrage_reduces_confidence() {
        let mut entries = vec![KnowledgeEntry {
            id: "e1".to_string(),
            confidence: 1.0,
            ..Default::default()
        }];
        let affected = apply_demurrage(&mut entries, 0.1);
        assert_eq!(affected, 1);
        assert!((entries[0].confidence - 0.9).abs() < 1e-10);
    }

    #[test]
    fn demurrage_reference_boost_restores() {
        let mut entry = KnowledgeEntry {
            id: "e1".to_string(),
            confidence: 0.5,
            ..Default::default()
        };
        demurrage_reference_boost(&mut entry, 0.3);
        assert!((entry.confidence - 0.8).abs() < 1e-10);
    }

    #[test]
    fn freeze_thaw_roundtrip() {
        let mut entry = KnowledgeEntry {
            id: "e1".to_string(),
            confidence: 0.75,
            ..Default::default()
        };
        assert!(!is_frozen(&entry));
        freeze_entry(&mut entry);
        assert!(is_frozen(&entry));
        assert!(entry.deprecated);

        let thawed = thaw_entry(&mut entry);
        assert!(thawed);
        assert!(!is_frozen(&entry));
        assert!(!entry.deprecated);
        assert!((entry.confidence - 0.75).abs() < 1e-4);
    }

    #[test]
    fn freeze_is_idempotent() {
        let mut entry = KnowledgeEntry {
            id: "e1".to_string(),
            confidence: 0.5,
            ..Default::default()
        };
        freeze_entry(&mut entry);
        let tag_count = entry.tags.len();
        freeze_entry(&mut entry); // second freeze should be no-op
        assert_eq!(entry.tags.len(), tag_count);
    }

    #[test]
    fn falsifier_record_halves_confidence() {
        let mut records = Vec::new();
        let mut entries = vec![KnowledgeEntry {
            id: "e1".to_string(),
            confidence: 0.8,
            ..Default::default()
        }];
        record_falsification(&mut records, &mut entries, "e1", "new data contradicts");
        assert_eq!(records.len(), 1);
        assert!((entries[0].confidence - 0.4).abs() < 1e-10);

        let warning = falsification_warning(&records, "e1");
        assert!(warning.is_some());
        assert_eq!(warning.unwrap().evidence, "new data contradicts");

        let no_warning = falsification_warning(&records, "e2");
        assert!(no_warning.is_none());
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
