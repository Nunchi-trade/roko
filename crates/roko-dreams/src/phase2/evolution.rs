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
                let update_count = grid.get(&key).map(|c| c.update_count + 1).unwrap_or(1);
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

    /// QD-score: sum of quality across all occupied cells.
    ///
    /// Higher QD-score indicates more diverse, high-quality strategy coverage.
    #[must_use]
    pub fn qd_score(grid: &std::collections::HashMap<Vec<usize>, ArchiveCell>) -> f64 {
        grid.values().map(|c| c.quality).sum()
    }

    /// Run a MAP-Elites evolution pass over an initial candidate set.
    ///
    /// Algorithm:
    /// 1. Place initial candidates into the archive grid
    /// 2. For `max_generations`: pick a random occupied cell, mutate the
    ///    occupant's descriptors, evaluate quality, and insert if better
    /// 3. Return all archive occupants as the evolved population
    ///
    /// `quality_fn` assigns a fitness to each strategy. `mutate_fn`
    /// creates a variation of an existing strategy.
    pub fn evolve<Q, M>(
        &self,
        candidates: Vec<(EvolutionaryStrategy, Vec<f64>, f64)>,
        _config: &EvolutionaryStrategy,
        max_generations: usize,
        grid: &mut std::collections::HashMap<Vec<usize>, ArchiveCell>,
        quality_fn: Q,
        mutate_fn: M,
    ) -> EvolutionResult
    where
        Q: Fn(&EvolutionaryStrategy) -> f64,
        M: Fn(&EvolutionaryStrategy, usize) -> EvolutionaryStrategy,
    {
        let mut insertions = 0usize;
        let mut replacements = 0usize;

        // Phase 1: seed the archive with initial candidates.
        for (strategy, descriptors, quality) in candidates {
            match self.insert(strategy, descriptors, quality, grid) {
                InsertResult::NewCell => insertions += 1,
                InsertResult::Replaced => replacements += 1,
                InsertResult::Rejected => {}
            }
        }

        // Phase 2: iterative improvement.
        for generation in 0..max_generations {
            if grid.is_empty() {
                break;
            }

            // Pick an occupied cell deterministically based on generation index.
            let keys: Vec<Vec<usize>> = grid.keys().cloned().collect();
            let pick_idx = generation % keys.len();
            let parent_key = &keys[pick_idx];

            let parent = match grid.get(parent_key) {
                Some(cell) => cell.clone(),
                None => continue,
            };

            // Mutate the parent strategy.
            let child = mutate_fn(&parent.strategy, generation);
            let child_quality = quality_fn(&child);

            // Perturb the descriptors slightly for diversity.
            let child_descriptors: Vec<f64> = parent
                .descriptors
                .iter()
                .enumerate()
                .map(|(i, &d)| {
                    // Deterministic perturbation based on generation and dimension.
                    let offset = ((generation * 7 + i * 13) % 100) as f64 / 500.0 - 0.1;
                    (d + offset * self.mutation_rate).clamp(0.0, 1.0)
                })
                .collect();

            match self.insert(child, child_descriptors, child_quality, grid) {
                InsertResult::NewCell => insertions += 1,
                InsertResult::Replaced => replacements += 1,
                InsertResult::Rejected => {}
            }
        }

        let qd_score = Self::qd_score(grid);
        let coverage = Self::coverage(grid);
        let best_quality = Self::best_quality(grid);

        EvolutionResult {
            qd_score,
            coverage,
            best_quality,
            insertions,
            replacements,
            generations_run: max_generations,
        }
    }
}

/// Summary of a MAP-Elites evolution run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvolutionResult {
    /// QD-score: sum of quality across all occupied cells.
    pub qd_score: f64,
    /// Number of occupied cells in the archive.
    pub coverage: usize,
    /// Best quality found across all cells.
    pub best_quality: Option<f64>,
    /// Number of new cells populated during evolution.
    pub insertions: usize,
    /// Number of cells where the occupant was replaced by a better candidate.
    pub replacements: usize,
    /// Number of generations executed.
    pub generations_run: usize,
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
        let result = archive.insert(test_strategy("s1"), vec![0.5, 0.5], 0.8, &mut grid);
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

    #[test]
    fn qd_score_sums_quality() {
        let archive = test_archive();
        let mut grid = HashMap::new();
        archive.insert(test_strategy("s1"), vec![0.1, 0.1], 0.5, &mut grid);
        archive.insert(test_strategy("s2"), vec![0.9, 0.9], 0.3, &mut grid);
        let qd = MapElitesArchive::qd_score(&grid);
        assert!((qd - 0.8).abs() < 1e-9);
    }

    #[test]
    fn evolve_populates_archive_from_candidates() {
        let archive = test_archive();
        let mut grid = HashMap::new();
        let config = test_strategy("config");

        let candidates = vec![
            (test_strategy("a"), vec![0.2, 0.3], 0.6),
            (test_strategy("b"), vec![0.7, 0.8], 0.4),
            (test_strategy("c"), vec![0.1, 0.9], 0.9),
        ];

        let result = archive.evolve(
            candidates,
            &config,
            5,
            &mut grid,
            |s| {
                // Simple quality: higher-id strategies get slightly higher scores.
                0.5 + s.id.len() as f64 * 0.01
            },
            |parent, generation| EvolutionaryStrategy {
                id: format!("{}-mut-{generation}", parent.id),
                description: format!("mutated from {} at generation {generation}", parent.id),
                parent_knowledge_ids: vec![parent.id.clone()],
                descriptors: parent.descriptors.clone(),
            },
        );

        assert!(result.qd_score > 0.0);
        assert!(result.coverage >= 3);
        assert!(result.best_quality.is_some());
        assert!(result.insertions > 0);
        assert_eq!(result.generations_run, 5);
    }

    #[test]
    fn evolve_on_empty_candidates_returns_zero_coverage() {
        let archive = test_archive();
        let mut grid = HashMap::new();
        let config = test_strategy("config");

        let result = archive.evolve(
            vec![],
            &config,
            10,
            &mut grid,
            |_| 0.5,
            |p, idx| EvolutionaryStrategy {
                id: format!("{}-{idx}", p.id),
                ..p.clone()
            },
        );

        assert_eq!(result.coverage, 0);
        assert_eq!(result.qd_score, 0.0);
        assert_eq!(result.best_quality, None);
    }
}
