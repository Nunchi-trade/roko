//! Phase 2 EVOLUTION stubs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::phase2::shared::EvolutionaryStrategy;

/// Bayesian memetic fitness evaluator for the EVOLUTION phase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BayesianMemeticFitness {
    /// Prior belief about baseline fitness.
    pub prior_mean: f64,
    /// Prior uncertainty.
    pub prior_std: f64,
    /// Minimum observations before evaluating fitness.
    pub min_observations: usize,
    /// Posterior confidence required to mark a heuristic beneficial.
    pub confidence_threshold: f64,
    /// Whether to account for confounding active heuristics.
    pub control_for_confounders: bool,
    /// Maximum confounders considered during evaluation.
    pub max_confounders: usize,
}

/// Fitness evaluation result with uncertainty quantification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FitnessEvaluation {
    /// Heuristic being evaluated.
    pub heuristic_id: String,
    /// Point estimate of fitness.
    pub fitness_point_estimate: f64,
    /// Posterior probability that fitness is beneficial.
    pub prob_beneficial: f64,
    /// Credible interval for the fitness estimate.
    pub credible_interval_90: (f64, f64),
    /// Number of episodes referencing the heuristic.
    pub n_referenced: usize,
    /// Number of episodes not referencing the heuristic.
    pub n_unreferenced: usize,
    /// Resulting classification.
    pub classification: FitnessClassification,
}

/// Classification assigned to one fitness evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FitnessClassification {
    /// Heuristic appears beneficial.
    Beneficial,
    /// Heuristic appears harmful.
    Harmful,
    /// Evidence remains insufficient.
    Uncertain,
}

/// Tournament selection settings for dream-time recombination.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TournamentRecombination {
    /// Number of candidates sampled per tournament.
    pub tournament_size: usize,
    /// Fraction of elites guaranteed to survive.
    pub elitism_fraction: f64,
    /// Probability that two selected parents recombine.
    pub crossover_rate: f64,
    /// Probability of post-crossover mutation.
    pub mutation_rate: f64,
    /// Maximum active strategy population.
    pub max_population: usize,
}

/// MAP-Elites archive for quality-diversity strategy evolution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapElitesArchive {
    /// Descriptor axes used for archive indexing.
    pub descriptor_dimensions: Vec<DescriptorDimension>,
    /// Number of bins per descriptor dimension.
    pub bins_per_dimension: usize,
    /// Maximum size of the archive grid.
    pub max_archive_size: usize,
    /// Mutation rate for generating candidate strategies.
    pub mutation_rate: f64,
    /// Whether behavioral descriptors are derived from HDC structure.
    pub hdc_descriptors: bool,
    /// Minimum quality required to enter the archive.
    pub min_quality_threshold: f64,
}

/// One descriptor axis in the MAP-Elites archive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DescriptorDimension {
    /// Human-readable descriptor name.
    pub name: String,
    /// Minimum descriptor value covered by the archive.
    pub min_value: f64,
    /// Maximum descriptor value covered by the archive.
    pub max_value: f64,
}

/// One populated cell in the MAP-Elites archive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveCell {
    /// Strategy occupying the cell.
    pub strategy: EvolutionaryStrategy,
    /// Quality assigned to the strategy.
    pub quality: f64,
    /// Descriptor coordinates for the strategy.
    pub descriptors: Vec<f64>,
    /// Number of times the cell has been updated.
    pub update_count: usize,
    /// Most recent update time.
    pub last_updated: DateTime<Utc>,
}
