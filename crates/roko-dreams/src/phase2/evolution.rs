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

/// Result of inserting a solution into the MAP-Elites archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InsertResult {
    /// Solution was placed into an empty cell.
    NewCell,
    /// Solution replaced an existing lower-quality occupant.
    Replaced,
    /// Solution was rejected (below quality threshold or worse than occupant).
    Rejected,
}

impl MapElitesArchive {
    /// Discretize a behavioral descriptor value into a bin index.
    fn bin_index(&self, dim_idx: usize, value: f64) -> usize {
        if dim_idx >= self.descriptor_dimensions.len() {
            return 0;
        }
        let dim = &self.descriptor_dimensions[dim_idx];
        let range = dim.max_value - dim.min_value;
        if range <= 0.0 {
            return 0;
        }
        let normalized = ((value - dim.min_value) / range).clamp(0.0, 1.0);
        let bin = (normalized * self.bins_per_dimension as f64) as usize;
        bin.min(self.bins_per_dimension.saturating_sub(1))
    }

    /// Compute the grid key for a behavioral descriptor vector.
    #[must_use]
    pub fn grid_key(&self, descriptors: &[f64]) -> Vec<usize> {
        (0..self.descriptor_dimensions.len())
            .map(|i| {
                let v = descriptors.get(i).copied().unwrap_or(0.0);
                self.bin_index(i, v)
            })
            .collect()
    }

    /// Try to insert a solution into the archive.
    ///
    /// The solution is placed into the cell identified by its behavioral
    /// descriptors. If the cell is empty, the solution is accepted. If
    /// occupied, the solution replaces the occupant only when its quality
    /// is strictly higher.
    pub fn insert(
        &self,
        strategy: EvolutionaryStrategy,
        descriptors: Vec<f64>,
        quality: f64,
        grid: &mut std::collections::HashMap<Vec<usize>, ArchiveCell>,
    ) -> InsertResult {
        if quality < self.min_quality_threshold {
            return InsertResult::Rejected;
        }

        let key = self.grid_key(&descriptors);

        match grid.get(&key) {
            Some(existing) if existing.quality >= quality => InsertResult::Rejected,
            _ => {
                let update_count = grid
                    .get(&key)
                    .map(|c| c.update_count + 1)
                    .unwrap_or(1);
                let result = if grid.contains_key(&key) {
                    InsertResult::Replaced
                } else {
                    InsertResult::NewCell
                };
                grid.insert(
                    key,
                    ArchiveCell {
                        strategy,
                        quality,
                        descriptors,
                        update_count,
                        last_updated: Utc::now(),
                    },
                );
                result
            }
        }
    }

    /// Return the number of occupied cells in the grid.
    #[must_use]
    pub fn coverage(grid: &std::collections::HashMap<Vec<usize>, ArchiveCell>) -> usize {
        grid.len()
    }

    /// Return the best quality across all occupied cells.
    #[must_use]
    pub fn best_quality(grid: &std::collections::HashMap<Vec<usize>, ArchiveCell>) -> Option<f64> {
        grid.values().map(|c| c.quality).reduce(f64::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_archive() -> MapElitesArchive {
        MapElitesArchive {
            descriptor_dimensions: vec![
                DescriptorDimension {
                    name: "speed".into(),
                    min_value: 0.0,
                    max_value: 1.0,
                },
                DescriptorDimension {
                    name: "quality".into(),
                    min_value: 0.0,
                    max_value: 1.0,
                },
            ],
            bins_per_dimension: 10,
            max_archive_size: 100,
            mutation_rate: 0.1,
            hdc_descriptors: false,
            min_quality_threshold: 0.1,
        }
    }

    fn test_strategy(id: &str) -> EvolutionaryStrategy {
        EvolutionaryStrategy {
            id: id.to_string(),
            description: format!("strategy {id}"),
            parent_knowledge_ids: vec![],
            descriptors: vec![],
        }
    }

    #[test]
    fn insert_into_empty_cell() {
        let archive = test_archive();
        let mut grid = HashMap::new();
        let result = archive.insert(
            test_strategy("s1"),
            vec![0.5, 0.5],
            0.8,
            &mut grid,
        );
        assert_eq!(result, InsertResult::NewCell);
        assert_eq!(grid.len(), 1);
    }

    #[test]
    fn replace_with_higher_quality() {
        let archive = test_archive();
        let mut grid = HashMap::new();
        archive.insert(test_strategy("s1"), vec![0.5, 0.5], 0.5, &mut grid);
        let result = archive.insert(test_strategy("s2"), vec![0.5, 0.5], 0.9, &mut grid);
        assert_eq!(result, InsertResult::Replaced);
        let key = archive.grid_key(&[0.5, 0.5]);
        assert_eq!(grid[&key].strategy.id, "s2");
    }

    #[test]
    fn reject_lower_quality() {
        let archive = test_archive();
        let mut grid = HashMap::new();
        archive.insert(test_strategy("s1"), vec![0.5, 0.5], 0.9, &mut grid);
        let result = archive.insert(test_strategy("s2"), vec![0.5, 0.5], 0.5, &mut grid);
        assert_eq!(result, InsertResult::Rejected);
        let key = archive.grid_key(&[0.5, 0.5]);
        assert_eq!(grid[&key].strategy.id, "s1");
    }

    #[test]
    fn reject_below_threshold() {
        let archive = test_archive();
        let mut grid = HashMap::new();
        let result = archive.insert(test_strategy("s1"), vec![0.5, 0.5], 0.05, &mut grid);
        assert_eq!(result, InsertResult::Rejected);
        assert!(grid.is_empty());
    }

    #[test]
    fn different_descriptors_go_to_different_cells() {
        let archive = test_archive();
        let mut grid = HashMap::new();
        archive.insert(test_strategy("s1"), vec![0.1, 0.1], 0.5, &mut grid);
        archive.insert(test_strategy("s2"), vec![0.9, 0.9], 0.5, &mut grid);
        assert_eq!(grid.len(), 2);
    }

    #[test]
    fn best_quality_across_grid() {
        let archive = test_archive();
        let mut grid = HashMap::new();
        archive.insert(test_strategy("s1"), vec![0.1, 0.1], 0.5, &mut grid);
        archive.insert(test_strategy("s2"), vec![0.9, 0.9], 0.95, &mut grid);
        assert_eq!(MapElitesArchive::best_quality(&grid), Some(0.95));
    }
}
