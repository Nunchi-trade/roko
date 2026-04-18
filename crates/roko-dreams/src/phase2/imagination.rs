//! Phase 2 REM imagination and creativity stubs.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::phase2::shared::{HdcVector, ModelTier};

/// Configuration for backtracking counterfactual generation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BacktrackingCounterfactualConfig {
    /// Maximum number of backtracking steps to trace.
    pub max_backtrack_depth: usize,
    /// Whether to sample stochastically during backtracking.
    pub stochastic_mode: bool,
    /// Number of posterior samples to draw.
    pub posterior_samples: usize,
    /// Initial confidence assigned to backtracking hypotheses.
    pub initial_confidence: f64,
    /// Fraction of Level 3 budget allocated to backtracking.
    pub budget_fraction: f64,
}

/// Hypothesis emitted by REM imagination.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CounterfactualHypothesis {
    /// Stable hypothesis identifier.
    pub id: String,
    /// Human-readable hypothesis content.
    pub content: String,
    /// Initial dream confidence.
    pub confidence: f64,
    /// Generator that produced the hypothesis.
    pub generation_mode: GenerationMode,
    /// Episodes that contributed to the hypothesis.
    pub source_episodes: Vec<String>,
    /// Existing knowledge contradicted by the hypothesis, if any.
    pub contradicts: Option<String>,
    /// HDC vector used for similarity comparison.
    pub hdc_vector: HdcVector,
    /// Novelty relative to existing knowledge.
    pub novelty: f64,
}

/// Generation sources used by REM imagination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationMode {
    /// Pearl SCM Level 1 association.
    Association,
    /// Pearl SCM Level 2 intervention.
    Intervention,
    /// Pearl SCM Level 3 counterfactual.
    Counterfactual,
    /// Boden combinational creativity.
    Combinational,
    /// Boden exploratory creativity.
    Exploratory,
    /// Boden transformational creativity.
    Transformational,
    /// Byrne fault-line analysis.
    FaultLine,
    /// Fauconnier-Turner conceptual blending.
    ConceptualBlend,
}

/// Association discovery configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssociationEngine {
    /// Minimum strength required to emit an association.
    pub strength_threshold: f64,
    /// Maximum associations returned per batch.
    pub max_correlations: usize,
    /// Minimum supporting episodes required.
    pub min_support: usize,
}

/// Association detected across replayed episodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Association {
    /// First feature in the association.
    pub feature_a: String,
    /// Second feature in the association.
    pub feature_b: String,
    /// Co-occurrence strength.
    pub strength: f64,
    /// Number of supporting episodes.
    pub support: usize,
    /// Coarse strength classification.
    pub classification: AssociationStrength,
}

/// Coarse strength bands for detected associations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssociationStrength {
    /// Strong association.
    Strong,
    /// Moderate association.
    Moderate,
    /// Weak association.
    Weak,
}

/// Intervention planning stub using a causal graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterventionEngine {
    /// Causal structure used during intervention search.
    pub causal_model: CausalGraph,
    /// Maximum alternative actions to simulate per episode.
    pub max_alternatives: usize,
}

/// Directed causal structure inferred from replayed episodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CausalGraph {
    /// Directed edges `(cause -> effect)` with evidence counts.
    pub edges: Vec<CausalEdge>,
}

/// One directed causal edge in the inferred graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CausalEdge {
    /// Cause feature.
    pub cause: String,
    /// Effect feature.
    pub effect: String,
    /// Strength of the inferred relation.
    pub strength: f64,
    /// Number of supporting episodes.
    pub evidence_count: usize,
}

/// Counterfactual search configuration over the causal graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CounterfactualEngine {
    /// Maximum latent variables to infer during abduction.
    pub max_latent_vars: usize,
    /// Search radius for pruning off-manifold counterfactuals.
    pub pruning_radius: f32,
    /// Maximum causal-chain traversal depth.
    pub max_chain_depth: usize,
}

/// Configuration for combinational creativity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CombinationalConfig {
    /// Minimum HDC distance required between paired episodes.
    pub dissimilarity_threshold: f32,
    /// Maximum episode pairs evaluated per cycle.
    pub max_pairs: usize,
    /// Minimum analogies requested from the generator.
    pub min_analogies: usize,
}

/// Configuration for exploratory creativity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExploratoryConfig {
    /// Multiplier for aggressive exploration.
    pub extreme_multiplier: f64,
    /// Divisor for conservative exploration.
    pub conservative_divisor: f64,
    /// Maximum heuristics explored per cycle.
    pub max_heuristics: usize,
}

/// Configuration for transformational creativity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformationalConfig {
    /// Maximum assumptions enumerated per heuristic.
    pub max_assumptions: usize,
    /// Minimum heuristic confidence required before transformation.
    pub min_heuristic_confidence: f64,
}

/// Emotional depotentiation parameters applied during REM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DepotentiationConfig {
    /// Minimum arousal reduction per cycle.
    pub delta_min: f64,
    /// Maximum arousal reduction per cycle.
    pub delta_max: f64,
    /// Floor for post-dream arousal.
    pub arousal_floor: f64,
}

/// Validation configuration for imagined strategies and rollouts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImaginationValidator {
    /// Maximum allowed KL drift per step.
    pub max_drift_per_step: f64,
    /// Learning rate for trust-region adjustment.
    pub trust_region_lr: f64,
    /// Minimum trust-region radius.
    pub trust_region_min: f64,
    /// Maximum trust-region radius.
    pub trust_region_max: f64,
    /// Maximum depth allowed before termination.
    pub max_imagination_depth: usize,
    /// Minimum plausibility required for acceptance.
    pub plausibility_threshold: f64,
    /// Whether cross-modal grounding is enabled.
    pub cross_modal_grounding: bool,
}

/// Summary of one validation pass over imagined strategies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImaginationQualityReport {
    /// Number of counterfactuals considered.
    pub total_counterfactuals: usize,
    /// Number accepted by the validator.
    pub accepted: usize,
    /// Number rejected for excessive drift.
    pub rejected_drift: usize,
    /// Number rejected for low plausibility.
    pub rejected_plausibility: usize,
    /// Mean drift across evaluated rollouts.
    pub mean_drift: f64,
    /// Mean plausibility across evaluated rollouts.
    pub mean_plausibility: f64,
    /// Maximum depth reached in the batch.
    pub max_depth_reached: usize,
}

/// REM-phase budget configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImaginationBudget {
    /// Total REM budget for the cycle.
    pub total_budget_usd: f64,
    /// Per-mode allocation fractions.
    pub mode_allocations: ImaginationModeAllocations,
    /// Maximum causal-chain exploration depth.
    pub max_chain_depth: usize,
    /// Maximum counterfactuals generated this cycle.
    pub max_counterfactuals: usize,
    /// Whether allocation adapts based on prior ROI.
    pub adaptive_allocation: bool,
    /// Minimum allocation preserved for any single mode.
    pub min_mode_fraction: f64,
}

/// Fractional allocation across REM generation modes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImaginationModeAllocations {
    /// Association budget fraction.
    pub association: f64,
    /// Intervention budget fraction.
    pub intervention: f64,
    /// Counterfactual budget fraction.
    pub counterfactual: f64,
    /// Combinational budget fraction.
    pub combinational: f64,
    /// Exploratory budget fraction.
    pub exploratory: f64,
    /// Transformational budget fraction.
    pub transformational: f64,
}

/// ROI tracker for adaptive REM budgeting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImaginationROITracker {
    /// Per-mode return-on-investment statistics.
    pub mode_stats: HashMap<GenerationMode, ModeROI>,
}

/// ROI summary for one generation mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeROI {
    /// Generation mode being tracked.
    pub mode: GenerationMode,
    /// Total hypotheses produced by the mode.
    pub total_hypotheses: usize,
    /// Hypotheses eventually promoted to durable knowledge.
    pub promoted_hypotheses: usize,
    /// Promotion rate for the mode.
    pub promotion_rate: f64,
    /// Mean confidence of promoted hypotheses.
    pub mean_confidence_at_promotion: f64,
    /// Total budget spent on the mode.
    pub total_budget_spent: f64,
    /// Cost per promoted hypothesis.
    pub cost_per_promoted_hypothesis: f64,
}

/// World-model configuration used by model-based REM planning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldModelConfig {
    /// Latent state dimensionality.
    pub latent_dim: usize,
    /// Number of rollout steps in imagined trajectories.
    pub imagination_horizon: usize,
    /// Discount factor applied to imagined futures.
    pub imagination_discount: f64,
    /// Whether symlog encoding is enabled.
    pub symlog_encoding: bool,
    /// KL balance coefficient for latent regularization.
    pub kl_balance: f64,
    /// Free-bits threshold before KL penalty applies.
    pub free_bits: f64,
}

/// Delta encoding for efficient episode/world-model replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeltaEpisodeEncoder {
    /// Whether delta encoding is enabled.
    pub use_delta_encoding: bool,
    /// Maximum number of delta tokens before fallback.
    pub max_delta_tokens: usize,
    /// Similarity threshold above which delta encoding is used.
    pub delta_similarity_threshold: f32,
    /// Number of summary tokens retained as context.
    pub summary_tokens: usize,
}

/// Interactive counterfactual environment configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractiveCounterfactualConfig {
    /// Maximum exploration steps per generated environment.
    pub max_exploration_steps: usize,
    /// Whether branching is allowed inside the environment.
    pub allow_branching: bool,
    /// Maximum branch depth.
    pub max_branch_depth: usize,
    /// Model tier used for environment generation.
    pub environment_model_tier: ModelTier,
    /// Whether generated environments are persisted for reuse.
    pub persist_environment: bool,
}

/// Extended creativity taxonomy including integrational resonance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CreativityMode {
    /// Combine familiar elements from unrelated domains.
    Combinational,
    /// Push a known strategy space toward its limits.
    Exploratory,
    /// Violate a core constraint to open a new possibility space.
    Transformational {
        /// Constraint being violated.
        target_constraint: String,
        /// Distance to the nearest accessible cluster after violation.
        possibility_distance: f32,
    },
    /// Mutual resonance across multiple active knowledge clusters.
    Integrational {
        /// Clusters participating in the resonance pattern.
        resonating_clusters: Vec<String>,
        /// Mean pairwise resonance strength above baseline.
        resonance_strength: f64,
    },
}
