use std::collections::{HashMap, VecDeque};

use chrono::{DateTime, Utc};
use roko_core::{BehavioralState, ContentHash, PadVector};
use serde::{Deserialize, Serialize};

use super::{
    AffectState, CONTRARIAN_FRACTION, DaimonState, DispatchStrategy, STRATEGY_DIMENSIONS,
    SomaticLandscape, SomaticMarker,
};

/// Stable identifier for a peer agent in a shared fleet.
pub type AgentId = String;

/// Behavioral strategy alias used by the legacy modulation table.
pub type AffectBehaviorStrategy = DispatchStrategy;

/// Named PAD octant for logging and dashboard display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AffectOctant {
    /// Positive pleasure, positive arousal, positive dominance.
    Excited,
    /// Positive pleasure, positive arousal, negative dominance.
    Surprised,
    /// Positive pleasure, negative arousal, positive dominance.
    Confident,
    /// Positive pleasure, negative arousal, negative dominance.
    Relaxed,
    /// Negative pleasure, positive arousal, positive dominance.
    Angry,
    /// Negative pleasure, positive arousal, negative dominance.
    Anxious,
    /// Negative pleasure, negative arousal, positive dominance.
    Bored,
    /// Negative pleasure, negative arousal, negative dominance.
    Depressed,
}

impl AffectOctant {
    /// Classify a PAD vector into one of the eight named octants.
    #[must_use]
    pub const fn from_pad(pleasure: f64, arousal: f64, dominance: f64) -> Self {
        if pleasure == 0.0 && arousal == 0.0 && dominance == 0.0 {
            return Self::Relaxed;
        }

        let positive_pleasure = !pleasure.is_sign_negative();
        let positive_arousal = !arousal.is_sign_negative();
        let positive_dominance = !dominance.is_sign_negative();

        match (positive_pleasure, positive_arousal, positive_dominance) {
            (true, true, true) => Self::Excited,
            (true, true, false) => Self::Surprised,
            (true, false, true) => Self::Confident,
            (true, false, false) => Self::Relaxed,
            (false, true, true) => Self::Angry,
            (false, true, false) => Self::Anxious,
            (false, false, true) => Self::Bored,
            (false, false, false) => Self::Depressed,
        }
    }

    /// Map the octant to the documented behavioral modulation profile.
    #[must_use]
    pub fn behavior_modulation(self) -> AffectBehaviorModulation {
        match self {
            Self::Excited | Self::Confident => AffectBehaviorModulation::confident(),
            Self::Surprised | Self::Relaxed => AffectBehaviorModulation::balanced(),
            Self::Angry => AffectBehaviorModulation::angry(),
            Self::Anxious | Self::Depressed => AffectBehaviorModulation::anxious(),
            Self::Bored => AffectBehaviorModulation::bored(),
        }
    }
}

/// PAD cosine similarity mapped into `[0.0, 1.0]`.
#[must_use]
pub fn pad_cosine_similarity(a: &PadVector, b: &PadVector) -> f64 {
    (*a).cosine_similarity(*b)
}

/// Rich behavioral-policy settings derived from affect state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AffectBehaviorModulation {
    /// Primary dispatch strategy to favor.
    pub strategy: AffectBehaviorStrategy,
    /// Exploration rate in `[0.0, 1.0]`.
    pub exploration_rate: f64,
    /// Whether to prefer proven playbooks over novel paths.
    pub prefer_proven_playbooks: bool,
    /// Additional tier escalation pressure.
    pub model_tier_escalation: u8,
    /// Extra retry budget granted by this modulation.
    pub extra_retries: u32,
    /// Whether to trigger dream or replay maintenance.
    pub trigger_dream_cycles: bool,
    /// Whether background maintenance work is preferred.
    pub run_maintenance_tasks: bool,
}

impl AffectBehaviorModulation {
    /// Baseline balanced modulation profile.
    #[must_use]
    pub const fn balanced() -> Self {
        Self {
            strategy: DispatchStrategy::Balanced,
            exploration_rate: 0.20,
            prefer_proven_playbooks: true,
            model_tier_escalation: 0,
            extra_retries: 0,
            trigger_dream_cycles: false,
            run_maintenance_tasks: false,
        }
    }

    /// Conservative profile for anxious or negative conditions.
    #[must_use]
    pub const fn anxious() -> Self {
        Self {
            strategy: DispatchStrategy::Conservative,
            exploration_rate: 0.05,
            prefer_proven_playbooks: true,
            model_tier_escalation: 0,
            extra_retries: 0,
            trigger_dream_cycles: false,
            run_maintenance_tasks: false,
        }
    }

    /// Escalating profile for frustrated but still forceful conditions.
    #[must_use]
    pub const fn angry() -> Self {
        Self {
            strategy: DispatchStrategy::Escalating,
            exploration_rate: 0.10,
            prefer_proven_playbooks: true,
            model_tier_escalation: 1,
            extra_retries: 2,
            trigger_dream_cycles: false,
            run_maintenance_tasks: false,
        }
    }

    /// Exploratory profile for high-confidence conditions.
    #[must_use]
    pub const fn confident() -> Self {
        Self {
            strategy: DispatchStrategy::Exploratory,
            exploration_rate: 0.35,
            prefer_proven_playbooks: false,
            model_tier_escalation: 0,
            extra_retries: 0,
            trigger_dream_cycles: false,
            run_maintenance_tasks: false,
        }
    }

    /// Proactive maintenance profile for idle or low-pressure conditions.
    #[must_use]
    pub const fn bored() -> Self {
        Self {
            strategy: DispatchStrategy::Proactive,
            exploration_rate: 0.15,
            prefer_proven_playbooks: true,
            model_tier_escalation: 0,
            extra_retries: 0,
            trigger_dream_cycles: true,
            run_maintenance_tasks: true,
        }
    }
}

impl Default for AffectBehaviorModulation {
    fn default() -> Self {
        Self::balanced()
    }
}

/// Entry and exit thresholds used to add hysteresis to state classification.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BehavioralStateThresholds {
    /// Confidence below this enters [`BehavioralState::Struggling`].
    pub struggling_entry_confidence: f64,
    /// Confidence above this exits [`BehavioralState::Struggling`].
    pub struggling_exit_confidence: f64,
    /// Dominance below this enters [`BehavioralState::Struggling`].
    pub struggling_entry_dominance: f64,
    /// Dominance above this exits [`BehavioralState::Struggling`].
    pub struggling_exit_dominance: f64,
    /// Pleasure above this enters [`BehavioralState::Coasting`].
    pub coasting_entry_pleasure: f64,
    /// Pleasure below this exits [`BehavioralState::Coasting`].
    pub coasting_exit_pleasure: f64,
    /// Arousal below this enters [`BehavioralState::Resting`].
    pub resting_entry_arousal: f64,
    /// Arousal above this exits [`BehavioralState::Resting`].
    pub resting_exit_arousal: f64,
}

impl Default for BehavioralStateThresholds {
    fn default() -> Self {
        Self {
            struggling_entry_confidence: 0.30,
            struggling_exit_confidence: 0.40,
            struggling_entry_dominance: -0.25,
            struggling_exit_dominance: -0.15,
            coasting_entry_pleasure: 0.35,
            coasting_exit_pleasure: 0.25,
            resting_entry_arousal: -0.20,
            resting_exit_arousal: -0.10,
        }
    }
}

/// Classify the next behavioral state using documented hysteresis thresholds.
#[must_use]
pub fn classify_with_hysteresis(
    state: &AffectState,
    current: BehavioralState,
    thresholds: &BehavioralStateThresholds,
) -> BehavioralState {
    let pad = state.pad;
    let confidence = state.confidence.clamp(0.0, 1.0);

    if current == BehavioralState::Struggling {
        let exits_struggling = confidence > thresholds.struggling_exit_confidence
            && pad.dominance > thresholds.struggling_exit_dominance;
        if !exits_struggling {
            return BehavioralState::Struggling;
        }
    } else if confidence < thresholds.struggling_entry_confidence
        || pad.dominance < thresholds.struggling_entry_dominance
        || (pad.pleasure < -0.30 && pad.arousal > 0.30)
    {
        return BehavioralState::Struggling;
    }

    if current == BehavioralState::Coasting {
        if pad.pleasure >= thresholds.coasting_exit_pleasure && confidence > 0.65 {
            return BehavioralState::Coasting;
        }
    } else if pad.pleasure > thresholds.coasting_entry_pleasure && confidence > 0.65 {
        return BehavioralState::Coasting;
    }

    if current == BehavioralState::Resting {
        if pad.arousal <= thresholds.resting_exit_arousal {
            return BehavioralState::Resting;
        }
    } else if pad.arousal < thresholds.resting_entry_arousal {
        return BehavioralState::Resting;
    }

    BehavioralState::classify(pad, confidence)
}

/// Stateful tracker that applies hysteresis and minimum dwell time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BehavioralStateTracker {
    /// Current stable state.
    pub current_state: BehavioralState,
    /// Tick at which the current state was entered.
    pub entered_at: u64,
    /// Minimum ticks to remain in a state before another transition is allowed.
    pub min_dwell_ticks: u64,
    /// Threshold configuration for hysteresis.
    pub thresholds: BehavioralStateThresholds,
}

impl Default for BehavioralStateTracker {
    fn default() -> Self {
        Self {
            current_state: BehavioralState::Engaged,
            entered_at: 0,
            min_dwell_ticks: 10,
            thresholds: BehavioralStateThresholds::default(),
        }
    }
}

impl BehavioralStateTracker {
    /// Update the tracked state for the current tick.
    pub fn update(&mut self, state: &AffectState, current_tick: u64) -> BehavioralState {
        let candidate = classify_with_hysteresis(state, self.current_state, &self.thresholds);
        if candidate != self.current_state {
            let dwell = current_tick.saturating_sub(self.entered_at);
            if dwell >= self.min_dwell_ticks {
                self.current_state = candidate;
                self.entered_at = current_tick;
            }
        }
        self.current_state
    }
}

/// Threshold deltas applied to cascade-routing cutoffs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TierBias {
    /// Delta added to the T0→T1 threshold.
    pub t0_threshold_delta: f64,
    /// Delta added to the T1→T2 threshold.
    pub t1_threshold_delta: f64,
}

impl TierBias {
    /// Zero bias.
    pub const ZERO: Self = Self {
        t0_threshold_delta: 0.0,
        t1_threshold_delta: 0.0,
    };
}

impl Default for TierBias {
    fn default() -> Self {
        Self::ZERO
    }
}

/// Absolute routing thresholds derived from behavioral state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TierThresholds {
    /// Maximum prediction error that stays on T0.
    pub t0_ceiling: f64,
    /// Maximum prediction error that stays on T1.
    pub t1_ceiling: f64,
}

impl Default for TierThresholds {
    fn default() -> Self {
        Self {
            t0_ceiling: 0.20,
            t1_ceiling: 0.60,
        }
    }
}

/// Return the documented tier thresholds for a behavioral state.
#[must_use]
pub const fn adjusted_thresholds(state: &BehavioralState) -> TierThresholds {
    match state {
        BehavioralState::Struggling => TierThresholds {
            t0_ceiling: 0.10,
            t1_ceiling: 0.40,
        },
        BehavioralState::Coasting => TierThresholds {
            t0_ceiling: 0.30,
            t1_ceiling: 0.80,
        },
        BehavioralState::Focused => TierThresholds {
            t0_ceiling: 0.25,
            t1_ceiling: 0.70,
        },
        BehavioralState::Exploring => TierThresholds {
            t0_ceiling: 0.15,
            t1_ceiling: 0.55,
        },
        BehavioralState::Resting => TierThresholds {
            t0_ceiling: 0.20,
            t1_ceiling: 0.90,
        },
        BehavioralState::Engaged => TierThresholds {
            t0_ceiling: 0.20,
            t1_ceiling: 0.60,
        },
    }
}

/// Lightweight efficiency telemetry payload keyed off Daimon effort selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EfficiencyEvent {
    /// Task identifier.
    pub task_id: String,
    /// Model slug used for the run.
    pub model_used: String,
    /// Effort label selected by the Daimon.
    pub effort_label: String,
    /// Input token count.
    pub tokens_in: u64,
    /// Output token count.
    pub tokens_out: u64,
    /// Total wall-clock time in milliseconds.
    pub wall_time_ms: u64,
    /// Whether the task ultimately passed its gate.
    pub gate_passed: bool,
}

/// Event emitted when a strong somatic marker materially biases behavior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SomaticMarkerFiredEvent {
    /// Situation description that triggered the marker.
    pub situation: String,
    /// Valence of the fired marker.
    pub valence: f64,
    /// Episodes that contributed to the marker.
    pub source_episodes: Vec<ContentHash>,
    /// Strategy parameter influenced by the marker.
    pub strategy_param: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct ContrarianEvent {
    tick: u64,
    was_contrarian: bool,
}

/// Runtime settings for contrarian retrieval.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ContrarianConfig {
    /// Rolling window size in ticks.
    pub window_size: usize,
    /// Minimum contrarian fraction within the window.
    pub min_contrarian_fraction: f64,
    /// Blend weight for contrarian somatic neighbours.
    pub somatic_blend_weight: f64,
    /// Minimum pleasure-distance required to count as contrarian.
    pub min_valence_delta: f64,
    /// Affect-weight override for contrarian knowledge queries.
    pub contrarian_alpha: f64,
}

impl Default for ContrarianConfig {
    fn default() -> Self {
        Self {
            window_size: 200,
            min_contrarian_fraction: CONTRARIAN_FRACTION,
            somatic_blend_weight: CONTRARIAN_FRACTION,
            min_valence_delta: 0.10,
            contrarian_alpha: 0.5,
        }
    }
}

/// Rolling tracker that enforces a minimum contrarian retrieval rate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContrarianTracker {
    /// Recent retrievals within the rolling window.
    window: VecDeque<ContrarianEvent>,
    /// Maximum tracked window size.
    pub window_size: usize,
    /// Minimum contrarian fraction to maintain.
    pub min_contrarian_fraction: f64,
}

impl Default for ContrarianTracker {
    fn default() -> Self {
        let config = ContrarianConfig::default();
        Self {
            window: VecDeque::new(),
            window_size: config.window_size,
            min_contrarian_fraction: config.min_contrarian_fraction,
        }
    }
}

impl ContrarianTracker {
    /// Determine whether the next retrieval should be forced contrarian.
    #[must_use]
    pub fn should_inject(&self, current_tick: u64) -> bool {
        let window_start = current_tick.saturating_sub(self.window_size as u64);
        let recent = self
            .window
            .iter()
            .filter(|event| event.tick >= window_start)
            .collect::<Vec<_>>();

        if recent.is_empty() {
            return true;
        }

        let contrarian_count = recent.iter().filter(|event| event.was_contrarian).count();
        let contrarian_rate = contrarian_count as f64 / recent.len() as f64;
        contrarian_rate < self.min_contrarian_fraction
    }

    /// Record a retrieval outcome into the rolling window.
    pub fn record(&mut self, tick: u64, was_contrarian: bool) {
        self.window.push_back(ContrarianEvent {
            tick,
            was_contrarian,
        });
        while self.window.len() > self.window_size.saturating_mul(2) {
            let _ = self.window.pop_front();
        }
    }
}

/// Affect-aware retrieval score returned by memory backends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoredEntry {
    /// Retrieved content identifier.
    pub content_hash: ContentHash,
    /// Semantic similarity component.
    pub semantic_score: f64,
    /// Emotional congruence component.
    pub emotional_score: f64,
    /// Final combined ranking score.
    pub combined_score: f64,
    /// Pleasure component of the retrieved entry.
    pub valence: f64,
}

/// Trait implemented by affect-aware retrieval stores.
pub trait AffectWeightedQuery {
    /// Query entries that are both semantically relevant and emotionally congruent.
    fn query_with_affect(
        &self,
        query_embedding: &[f32],
        pad: &PadVector,
        limit: usize,
    ) -> Vec<ScoredEntry>;
}

/// Domain-specific registration of a strategy-space layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainRegistration {
    /// Domain name used for routing and transfer lookups.
    pub name: String,
    /// Dimension definitions for the fixed 8D space.
    pub dimensions: [DimensionDef; STRATEGY_DIMENSIONS],
}

/// One registered strategy-space dimension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DimensionDef {
    /// Human-readable dimension name.
    pub name: String,
    /// Source subsystem responsible for extracting the raw value.
    pub source: DimensionSource,
    /// Weight used for distance calculations.
    pub weight: f64,
}

/// Built-in and custom extraction sources for strategy dimensions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DimensionSource {
    /// Code-analysis derived signal.
    TaskAnalysis,
    /// Test coverage and gate configuration.
    CoverageAndGates,
    /// Knowledge-store similarity.
    NeuroSimilarity,
    /// Daimon affect state itself.
    DaimonState,
    /// Scheduler metadata.
    Scheduler,
    /// Diff-analysis derived signal.
    DiffAnalysis,
    /// Dependency graph analysis.
    DepGraph,
    /// Named custom extractor.
    Custom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransferRole {
    Difficulty,
    Danger,
    Familiarity,
    SelfAssessment,
    Urgency,
    Breadth,
    Recoverability,
    Coupling,
}

fn classify_transfer_role(label: &str, index: usize) -> TransferRole {
    let normalized = label.trim().to_ascii_lowercase();

    if contains_any(
        &normalized,
        &["complex", "difficulty", "volatility", "unstable"],
    ) {
        return TransferRole::Difficulty;
    }
    if contains_any(
        &normalized,
        &[
            "risk", "danger", "leverage", "exposure", "blast", "slippage",
        ],
    ) {
        return TransferRole::Danger;
    }
    if contains_any(
        &normalized,
        &[
            "novel",
            "familiar",
            "correlation",
            "similarity",
            "ambiguity",
        ],
    ) {
        return TransferRole::Familiarity;
    }
    if contains_any(&normalized, &["confidence", "conviction", "certainty"]) {
        return TransferRole::SelfAssessment;
    }
    if contains_any(
        &normalized,
        &["time", "deadline", "horizon", "urgency", "latency"],
    ) {
        return TransferRole::Urgency;
    }
    if contains_any(
        &normalized,
        &["scope", "breadth", "concentration", "liquidity", "surface"],
    ) {
        return TransferRole::Breadth;
    }
    if contains_any(
        &normalized,
        &[
            "revers",
            "rollback",
            "recover",
            "counterparty",
            "exit",
            "undo",
        ],
    ) {
        return TransferRole::Recoverability;
    }
    if contains_any(
        &normalized,
        &[
            "dependency",
            "coupling",
            "regulatory",
            "compliance",
            "integration",
        ],
    ) {
        return TransferRole::Coupling;
    }

    match index {
        0 => TransferRole::Difficulty,
        1 => TransferRole::Danger,
        2 => TransferRole::Familiarity,
        3 => TransferRole::SelfAssessment,
        4 => TransferRole::Urgency,
        5 => TransferRole::Breadth,
        6 => TransferRole::Recoverability,
        _ => TransferRole::Coupling,
    }
}

fn contains_any(label: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| label.contains(needle))
}

/// Mapper that transfers coordinates between registered domains.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategyTransferMapper {
    /// Source-to-target dimension correspondences.
    pub dimension_map: [(usize, usize); STRATEGY_DIMENSIONS],
}

impl StrategyTransferMapper {
    /// Build a mapping between two domains based on dimension roles.
    #[must_use]
    pub fn from_domains(source: &DomainRegistration, target: &DomainRegistration) -> Self {
        let mut dimension_map = [(0_usize, 0_usize); STRATEGY_DIMENSIONS];
        for (src_idx, src_dim) in source.dimensions.iter().enumerate() {
            let role = classify_transfer_role(&src_dim.name, src_idx);
            let target_idx = target
                .dimensions
                .iter()
                .enumerate()
                .find_map(|(candidate_idx, dimension)| {
                    (classify_transfer_role(&dimension.name, candidate_idx) == role)
                        .then_some(candidate_idx)
                })
                .unwrap_or(src_idx);
            dimension_map[src_idx] = (src_idx, target_idx);
        }
        Self { dimension_map }
    }

    /// Transfer coordinates from the source layout into the target layout.
    #[must_use]
    pub fn transfer(
        &self,
        source_coords: &[f64; STRATEGY_DIMENSIONS],
    ) -> [f64; STRATEGY_DIMENSIONS] {
        let mut target_coords = [0.5; STRATEGY_DIMENSIONS];
        for &(src_idx, target_idx) in &self.dimension_map {
            target_coords[target_idx] = source_coords[src_idx];
        }
        target_coords
    }
}

/// Per-dimension weights applied before distance calculations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DimensionWeights {
    /// Weight for each of the eight dimensions.
    pub weights: [f64; STRATEGY_DIMENSIONS],
}

impl Default for DimensionWeights {
    fn default() -> Self {
        Self {
            weights: [1.0; STRATEGY_DIMENSIONS],
        }
    }
}

impl DimensionWeights {
    /// Apply the configured weights to a coordinate vector.
    #[must_use]
    pub fn apply(&self, coords: &[f64; STRATEGY_DIMENSIONS]) -> [f64; STRATEGY_DIMENSIONS] {
        let mut weighted = [0.0; STRATEGY_DIMENSIONS];
        for index in 0..STRATEGY_DIMENSIONS {
            weighted[index] = coords[index] * self.weights[index].sqrt();
        }
        weighted
    }
}

/// Resource-pressure model used to compress strategy coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ResourcePressure {
    /// Token budget remaining as a fraction of total budget.
    pub token_budget_remaining: f64,
    /// Time budget remaining as a fraction of total budget.
    pub time_budget_remaining: f64,
}

impl ResourcePressure {
    /// Compute the scalar compression factor induced by resource pressure.
    #[must_use]
    pub fn scalar(&self) -> f64 {
        self.token_budget_remaining
            .min(self.time_budget_remaining)
            .sqrt()
            .clamp(0.0, 1.0)
    }

    /// Compress coordinates toward the neutral midpoint as pressure rises.
    #[must_use]
    pub fn apply(&self, coords: &[f64; STRATEGY_DIMENSIONS]) -> [f64; STRATEGY_DIMENSIONS] {
        let scalar = self.scalar();
        let mut compressed = [0.0; STRATEGY_DIMENSIONS];
        for index in 0..STRATEGY_DIMENSIONS {
            compressed[index] = scalar * coords[index] + (1.0 - scalar) * 0.5;
        }
        compressed
    }
}

/// Emotional provenance transferred from episodes into derived knowledge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmotionalProvenance {
    /// Average PAD vector observed across supporting evidence.
    pub average_pad: PadVector,
    /// Human-readable emotion label at initial discovery time.
    pub discovery_emotion: String,
    /// Narrative validation arc across supporting episodes.
    pub validation_arc: Option<ValidationArc>,
    /// Normalized emotional diversity across supporting evidence.
    pub emotional_diversity: f64,
}

impl Default for EmotionalProvenance {
    fn default() -> Self {
        Self {
            average_pad: PadVector::neutral(),
            discovery_emotion: "neutral".to_string(),
            validation_arc: None,
            emotional_diversity: 0.0,
        }
    }
}

/// Validation-trajectory labels used for emotional provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationArc {
    /// Adversity leading to a positive outcome.
    Redemptive,
    /// Initial success followed by failure.
    Contaminating,
    /// Consistent tone throughout validation.
    Stable,
    /// Gradual improvement over time.
    Progressive,
}

/// Error-category familiarity tracker for appraisal scaling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ErrorPatternTracker {
    /// Error category to `(seen_count, resolved_count)` counts.
    pub patterns: HashMap<String, (u32, u32)>,
}

impl ErrorPatternTracker {
    /// Compute familiarity with a particular error category.
    #[must_use]
    pub fn familiarity(&self, error_category: &str) -> f64 {
        let (seen, resolved) = self.patterns.get(error_category).copied().unwrap_or((0, 0));
        if seen == 0 {
            return 0.0;
        }
        let resolution_rate = resolved as f64 / seen as f64;
        let experience = (seen as f64 / 10.0).min(1.0);
        (resolution_rate * experience).clamp(0.0, 1.0)
    }

    /// Scale a gate-failure delta based on prior familiarity.
    #[must_use]
    pub fn scale_gate_failure(
        &self,
        error_category: &str,
        base_delta: (f64, f64, f64, f64),
    ) -> (f64, f64, f64, f64) {
        let scale = 1.5 - self.familiarity(error_category);
        (
            base_delta.0 * scale,
            base_delta.1 * scale,
            base_delta.2 * scale,
            base_delta.3 * scale,
        )
    }
}

/// Per-task fatigue state used by the coding-agent integration stubs.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FatigueState {
    /// Consecutive failures recorded for the task.
    pub consecutive_failures: u32,
    /// Timestamp of the first failure in the active streak.
    pub first_failure_at: DateTime<Utc>,
    /// Timestamp of the most recent failure in the active streak.
    pub last_failure_at: DateTime<Utc>,
    /// Pleasure at the start of the streak.
    pub pleasure_at_start: f64,
    /// Current pleasure after the latest failure.
    pub current_pleasure: f64,
}

/// Failure-streak detector for repeated task frustration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FatigueDetector {
    /// Per-task fatigue state.
    pub task_failures: HashMap<String, FatigueState>,
}

impl FatigueDetector {
    /// Return whether a task currently satisfies the fatigue heuristic.
    #[must_use]
    pub fn is_fatigued(&self, task_id: &str) -> bool {
        let Some(state) = self.task_failures.get(task_id) else {
            return false;
        };

        let many_failures = state.consecutive_failures >= 3;
        let pleasure_drop = state.pleasure_at_start - state.current_pleasure > 0.15;
        let duration_hours = state
            .last_failure_at
            .signed_duration_since(state.first_failure_at)
            .num_minutes() as f64
            / 60.0;
        let rapid_failures = duration_hours < 2.0;

        many_failures && pleasure_drop && rapid_failures
    }
}

/// Response options triggered by fatigue detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FatigueAction {
    /// Escalate to stronger models or more budget.
    Escalate,
    /// Re-plan with a different strategy.
    Replan,
    /// Trigger dream-style consolidation or replay.
    DreamCycle,
    /// Lower priority and work elsewhere first.
    Deprioritize,
    /// Ask an external operator or peer for help.
    HelpRequest,
}

/// Select the documented fatigue response for a behavioral state.
#[must_use]
pub const fn fatigue_response(state: &BehavioralState) -> FatigueAction {
    match state {
        BehavioralState::Struggling => FatigueAction::Escalate,
        BehavioralState::Exploring => FatigueAction::Replan,
        BehavioralState::Resting => FatigueAction::DreamCycle,
        _ => FatigueAction::Deprioritize,
    }
}

/// Collective-emotion trigger propagated across the mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContagionTrigger {
    /// A peer shared a warning.
    WarningPush,
    /// A peer emitted a critical alert.
    PeerAlert,
    /// A peer reported sustained success.
    PeerSustainedSuccess,
    /// A peer reported sustained failure.
    PeerSustainedFailure,
    /// A peer shared a dream-derived insight.
    PeerDreamInsight,
}

/// One peer-derived contagion event received from the mesh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContagionEvent {
    /// Source agent identifier.
    pub source: AgentId,
    /// Trigger category for the event.
    pub trigger: ContagionTrigger,
    /// Source PAD snapshot used for contagion attenuation.
    pub source_pad: PadVector,
}

/// Borrowed affect contribution awaiting decay or replacement by local evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BorrowedAffect {
    /// Source agent that originated the affect.
    pub source: AgentId,
    /// Borrowed pleasure delta after attenuation.
    pub p_delta: f64,
    /// Borrowed arousal delta after attenuation and capping.
    pub a_delta: f64,
    /// Timestamp at which the borrowed affect was applied.
    pub applied_at: DateTime<Utc>,
}

/// Aggregate somatic field formed from multiple agents' marker sets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SomaticField {
    /// Merged somatic landscape across peers.
    pub landscape: SomaticLandscape,
    /// Contribution weight per peer agent.
    pub agent_weights: HashMap<AgentId, f64>,
}

impl SomaticField {
    /// Merge one peer's markers into the shared field.
    pub fn merge(&mut self, agent_id: AgentId, markers: &[SomaticMarker]) {
        let weight = *self.agent_weights.entry(agent_id).or_insert(1.0);
        for marker in markers {
            let mut weighted = marker.clone();
            weighted.valence *= weight;
            weighted.intensity *= weight;
            self.landscape.record_marker(weighted);
        }
    }
}

impl AffectState {
    /// Return the current PAD octant label for dashboard and logging surfaces.
    #[must_use]
    pub fn octant(&self) -> AffectOctant {
        AffectOctant::from_pad(self.pad.pleasure, self.pad.arousal, self.pad.dominance)
    }
}

impl DaimonState {
    /// Return the current cascade thresholds implied by the live behavioral state.
    #[must_use]
    pub fn tier_thresholds(&self) -> TierThresholds {
        adjusted_thresholds(&self.state.behavioral_state)
    }

    /// Return the stored confidence hint for one crate or module.
    #[must_use]
    pub fn crate_confidence(&self, crate_name: &str) -> f64 {
        self.crate_confidence_map
            .get(crate_name)
            .copied()
            .unwrap_or(0.50)
    }

    /// Apply peer-derived emotional contagion with the documented attenuation rules.
    pub fn apply_contagion(&mut self, event: ContagionEvent) {
        let p_delta = event.source_pad.pleasure * 0.3;
        let a_delta = (event.source_pad.arousal * 0.3).min(0.3);
        let now = Utc::now();

        self.state.apply_delta(p_delta, a_delta, 0.0, 0.0, now);
        self.borrowed_affect.push(BorrowedAffect {
            source: event.source,
            p_delta,
            a_delta,
            applied_at: now,
        });
        self.autosave();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_cosine_similarity_uses_neutral_fallback() {
        assert_eq!(
            pad_cosine_similarity(&PadVector::neutral(), &PadVector::new(1.0, 0.0, 0.0)),
            0.5
        );
    }

    #[test]
    fn contrarian_tracker_bootstraps_when_empty() {
        let tracker = ContrarianTracker::default();
        assert!(tracker.should_inject(42));
    }

    #[test]
    fn resource_pressure_compresses_toward_midpoint() {
        let pressure = ResourcePressure {
            token_budget_remaining: 0.25,
            time_budget_remaining: 1.0,
        };

        let compressed = pressure.apply(&[1.0; STRATEGY_DIMENSIONS]);

        assert!(compressed.iter().all(|value| *value < 1.0));
        assert!(compressed.iter().all(|value| *value > 0.5));
    }
}
