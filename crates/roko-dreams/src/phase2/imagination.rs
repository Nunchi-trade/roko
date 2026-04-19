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
///
/// Implements a simplified Pearl Structural Causal Model (SCM) with three
/// inference levels: Association (L1), Intervention (L2), Counterfactual (L3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CausalGraph {
    /// Directed edges `(cause -> effect)` with evidence counts.
    pub edges: Vec<CausalEdge>,
}

/// A single causal variable in the SCM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CausalVariable {
    /// Variable name (e.g. "model", "gate\_verdict", "task\_outcome").
    pub name: String,
    /// Observed value in the original episode.
    pub observed_value: serde_json::Value,
}

/// Result of a counterfactual generation step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CounterfactualResult {
    /// Pearl level used for generation.
    pub level: GenerationMode,
    /// The intervention applied.
    pub intervention: CausalVariable,
    /// Projected outcome after propagation.
    pub projected_outcome: String,
    /// Confidence in the projection.
    pub confidence: f64,
    /// Plausibility score (0-1, higher = more plausible).
    pub plausibility: f64,
    /// Source episode ID.
    pub source_episode_id: String,
}

impl CausalGraph {
    /// Build a causal graph from observed episodes by inferring
    /// co-occurrence edges between episode features.
    #[must_use]
    pub fn from_episodes(episodes: &[CausalVariable], outcomes: &[(&str, bool)]) -> Self {
        let mut edges = Vec::new();
        // Infer model -> outcome edges
        let mut model_outcomes: HashMap<String, (usize, usize)> = HashMap::new();
        for (ep_id, success) in outcomes {
            let model = episodes
                .iter()
                .find(|v| v.name == "model")
                .map(|v| v.observed_value.as_str().unwrap_or("unknown").to_string())
                .unwrap_or_else(|| ep_id.to_string());
            let entry = model_outcomes.entry(model).or_insert((0, 0));
            entry.0 += 1;
            if *success {
                entry.1 += 1;
            }
        }
        for (model, (total, successes)) in &model_outcomes {
            let strength = *successes as f64 / (*total).max(1) as f64;
            edges.push(CausalEdge {
                cause: format!("model:{model}"),
                effect: "outcome".to_string(),
                strength,
                evidence_count: *total,
            });
        }
        Self { edges }
    }

    /// Find all direct effects of a given cause variable.
    #[must_use]
    pub fn effects_of(&self, cause: &str) -> Vec<&CausalEdge> {
        self.edges
            .iter()
            .filter(|e| e.cause == cause)
            .collect()
    }

    /// Find all direct causes of a given effect variable.
    #[must_use]
    pub fn causes_of(&self, effect: &str) -> Vec<&CausalEdge> {
        self.edges
            .iter()
            .filter(|e| e.effect == effect)
            .collect()
    }

    /// Propagate an intervention through the graph, returning affected
    /// variables and their projected changes.
    #[must_use]
    pub fn propagate_intervention(
        &self,
        intervention_cause: &str,
        max_depth: usize,
    ) -> Vec<(String, f64)> {
        let mut affected = Vec::new();
        let mut frontier = vec![(intervention_cause.to_string(), 1.0_f64)];
        let mut visited = std::collections::HashSet::new();
        let mut depth = 0;

        while !frontier.is_empty() && depth < max_depth {
            let mut next_frontier = Vec::new();
            for (cause, strength_so_far) in &frontier {
                if !visited.insert(cause.clone()) {
                    continue;
                }
                for edge in self.effects_of(cause) {
                    let propagated_strength = strength_so_far * edge.strength;
                    affected.push((edge.effect.clone(), propagated_strength));
                    next_frontier.push((edge.effect.clone(), propagated_strength));
                }
            }
            frontier = next_frontier;
            depth += 1;
        }
        affected
    }
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
///
/// Implements Pearl's three-level causal hierarchy:
/// - Level 1 (Association): "What patterns continue?" -- observational
/// - Level 2 (Intervention): "What if we change X?" -- do-calculus
/// - Level 3 (Counterfactual): "What would have happened?" -- backtracking
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CounterfactualEngine {
    /// Maximum latent variables to infer during abduction.
    pub max_latent_vars: usize,
    /// Search radius for pruning off-manifold counterfactuals.
    pub pruning_radius: f32,
    /// Maximum causal-chain traversal depth.
    pub max_chain_depth: usize,
}

impl CounterfactualEngine {
    /// Create a default engine configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            max_latent_vars: 4,
            pruning_radius: 0.5,
            max_chain_depth: 3,
        }
    }

    /// Pearl Level 1: Association -- project trends from replayed episodes.
    ///
    /// Finds recurring patterns and extrapolates them.  Pure observation,
    /// no intervention.
    #[must_use]
    pub fn generate_association(
        &self,
        graph: &CausalGraph,
        episode_id: &str,
        variable: &str,
    ) -> CounterfactualResult {
        let effects = graph.effects_of(variable);
        let avg_strength = if effects.is_empty() {
            0.5
        } else {
            effects.iter().map(|e| e.strength).sum::<f64>() / effects.len() as f64
        };
        let total_evidence: usize = effects.iter().map(|e| e.evidence_count).sum();

        CounterfactualResult {
            level: GenerationMode::Association,
            intervention: CausalVariable {
                name: variable.to_string(),
                observed_value: serde_json::Value::String("trend_projection".to_string()),
            },
            projected_outcome: format!(
                "Pattern {variable} has average strength {avg_strength:.2} across {total_evidence} observations"
            ),
            confidence: (avg_strength * 0.8).clamp(0.1, 0.9),
            plausibility: (0.5 + total_evidence.min(10) as f64 * 0.04).clamp(0.0, 1.0),
            source_episode_id: episode_id.to_string(),
        }
    }

    /// Pearl Level 2: Intervention -- mutate one variable, propagate effects.
    ///
    /// do(X = x'): hold everything else fixed, change X, and observe the
    /// downstream consequences through the causal graph.
    #[must_use]
    pub fn generate_intervention(
        &self,
        graph: &CausalGraph,
        episode_id: &str,
        variable: &str,
        new_value: &str,
    ) -> CounterfactualResult {
        let affected = graph.propagate_intervention(variable, self.max_chain_depth);
        let total_impact: f64 = affected.iter().map(|(_, s)| s).sum();
        let affected_names: Vec<&str> = affected.iter().map(|(n, _)| n.as_str()).collect();

        CounterfactualResult {
            level: GenerationMode::Intervention,
            intervention: CausalVariable {
                name: variable.to_string(),
                observed_value: serde_json::Value::String(new_value.to_string()),
            },
            projected_outcome: format!(
                "do({variable} = {new_value}) affects {}: total impact {total_impact:.2}",
                if affected_names.is_empty() {
                    "nothing downstream".to_string()
                } else {
                    affected_names.join(", ")
                }
            ),
            confidence: (0.4 + total_impact * 0.3).clamp(0.1, 0.85),
            plausibility: if affected.is_empty() {
                0.3
            } else {
                (0.5 + total_impact * 0.2).clamp(0.2, 0.9)
            },
            source_episode_id: episode_id.to_string(),
        }
    }

    /// Pearl Level 3: Counterfactual -- backtracking from observed outcome.
    ///
    /// Given an observed failure, reason backwards: "what initial conditions
    /// would have produced a different outcome?"  Three steps:
    /// 1. Abduction: infer latent variables from the observed outcome
    /// 2. Action: apply the intervention
    /// 3. Prediction: propagate through the modified model
    #[must_use]
    pub fn generate_counterfactual(
        &self,
        graph: &CausalGraph,
        episode_id: &str,
        variable: &str,
        new_value: &str,
        original_success: bool,
    ) -> CounterfactualResult {
        // Step 1: Abduction -- find causes of the observed outcome
        let outcome_causes = graph.causes_of("outcome");
        let relevant_cause = outcome_causes
            .iter()
            .find(|e| e.cause.contains(variable))
            .or_else(|| outcome_causes.first());

        // Step 2: Action -- apply intervention to the cause
        let abduced_strength = relevant_cause.map(|e| e.strength).unwrap_or(0.5);

        // Step 3: Prediction -- would the outcome have differed?
        let counterfactual_success_prob = if original_success {
            // Was successful; would changing X have caused failure?
            (abduced_strength * 0.6).clamp(0.1, 0.8)
        } else {
            // Was failure; would changing X have fixed it?
            (1.0 - abduced_strength * 0.4).clamp(0.2, 0.9)
        };

        let outcome_change = if original_success {
            "might have failed"
        } else {
            "might have succeeded"
        };

        CounterfactualResult {
            level: GenerationMode::Counterfactual,
            intervention: CausalVariable {
                name: variable.to_string(),
                observed_value: serde_json::Value::String(new_value.to_string()),
            },
            projected_outcome: format!(
                "Had {variable} been {new_value}, episode {outcome_change} \
                 (P={counterfactual_success_prob:.2}, abduced strength={abduced_strength:.2})"
            ),
            confidence: (counterfactual_success_prob * 0.7).clamp(0.1, 0.8),
            plausibility: (0.3 + abduced_strength * 0.4).clamp(0.2, 0.85),
            source_episode_id: episode_id.to_string(),
        }
    }
}

impl Default for CounterfactualEngine {
    fn default() -> Self {
        Self::new()
    }
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
