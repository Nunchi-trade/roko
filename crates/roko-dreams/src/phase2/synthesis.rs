//! Phase 2 HDC counterfactual synthesis stubs.

use serde::{Deserialize, Serialize};

use crate::phase2::imagination::CounterfactualHypothesis;
use crate::phase2::shared::HdcVector;

/// K-medoids configuration for HDC clustering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KMedoidsConfig {
    /// Fixed `k`, or `None` for automatic selection.
    pub k: Option<usize>,
    /// Maximum `k` tested during automatic selection.
    pub max_k: usize,
    /// Maximum iterations per clustering run.
    pub max_iterations: usize,
    /// Convergence tolerance for clustering cost.
    pub convergence_epsilon: f64,
}

/// Configuration for generating a diverse counterfactual set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CounterfactualDiversityConfig {
    /// Number of counterfactuals to generate per episode.
    pub k: usize,
    /// Weight pulling counterfactuals toward the original.
    pub proximity_weight: f64,
    /// Weight encouraging pairwise diversity.
    pub diversity_weight: f64,
    /// Minimum pairwise HDC distance between outputs.
    pub min_pairwise_distance: f32,
    /// Maximum features changed per counterfactual.
    pub max_features_changed: usize,
}

/// Diverse set of counterfactuals generated from one episode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CounterfactualSet {
    /// Episode that seeded the set.
    pub original_episode: String,
    /// Counterfactuals generated for the episode.
    pub counterfactuals: Vec<CounterfactualHypothesis>,
    /// Diversity score assigned to the set.
    pub diversity_score: f64,
    /// Coverage of the nearby possibility space.
    pub coverage_score: f64,
    /// Mean proximity to the original episode.
    pub mean_proximity: f64,
    /// Mean sparsity across the generated set.
    pub mean_sparsity: f64,
}

/// Plausibility scoring configuration for counterfactuals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlausibilityScorer {
    /// Minimum density score required for acceptance.
    pub min_density_score: f64,
    /// Maximum local-outlier-factor tolerated.
    pub max_lof: f64,
    /// Whether to check causal consistency.
    pub check_causal_consistency: bool,
    /// Minimum manifold proximity required for acceptance.
    pub manifold_proximity_threshold: f32,
    /// Weight of path-based plausibility.
    pub path_weight: f64,
    /// Weight of density-based plausibility.
    pub density_weight: f64,
    /// Weight of causal consistency.
    pub causal_weight: f64,
}

/// Plausibility report for one generated counterfactual.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlausibilityReport {
    /// Counterfactual being scored.
    pub counterfactual_id: String,
    /// Density score assigned by the scorer.
    pub density_score: f64,
    /// Local outlier factor.
    pub lof_score: f64,
    /// Whether the candidate is causally consistent.
    pub causal_consistency: bool,
    /// Proximity to the known HDC manifold.
    pub manifold_proximity: f32,
    /// Composite plausibility score.
    pub composite_plausibility: f64,
    /// Whether the counterfactual is accepted.
    pub accepted: bool,
}

/// Population-level counterfactual configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalCounterfactualConfig {
    /// Number of candidate translation directions evaluated.
    pub n_candidates: usize,
    /// Minimum coverage required for a valid global counterfactual.
    pub min_coverage: f64,
    /// Maximum translation magnitude in HDC space.
    pub max_translation_magnitude: f32,
    /// Whether HDC permutation directions may seed candidates.
    pub use_hdc_permutation_candidates: bool,
}

/// Population-level counterfactual represented as a translation vector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalCounterfactual {
    /// Translation direction in HDC space.
    pub translation_direction: HdcVector,
    /// Magnitude of the translation.
    pub magnitude: f32,
    /// Fraction of the population covered by the translation.
    pub coverage: f64,
    /// Mean proximity of affected episodes after translation.
    pub mean_proximity: f64,
    /// Episode ids affected by the counterfactual direction.
    pub affected_episode_ids: Vec<String>,
}

/// Transport-based counterfactual generation configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransportCounterfactualConfig {
    /// Entropic regularization for the transport plan.
    pub sinkhorn_regularization: f64,
    /// Maximum solver iterations.
    pub max_iterations: usize,
    /// Convergence threshold for the transport solver.
    pub convergence_threshold: f64,
    /// Whether entropic optimal transport is used.
    pub use_entropic_ot: bool,
    /// Cost function used for transport.
    pub cost_function: TransportCostFunction,
}

/// Cost function used by transport-based counterfactuals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportCostFunction {
    /// Native HDC Hamming distance.
    Hamming,
    /// L2 distance in embedding space.
    L2Embedding,
    /// Mahalanobis distance with covariance structure.
    Mahalanobis,
}
