//! Pareto frontier computation for model cost-quality tradeoffs.
//!
//! This module identifies which models are non-dominated with respect to
//! pass rate and cost per successful task. A model is Pareto-optimal if no
//! other model has both a higher pass rate and a lower cost per successful
//! task.
//!
//! The [`ParetoFrontier`] struct extends this to arbitrary multi-objective
//! optimization (LEARN-06), supporting cost vs quality vs latency tradeoffs.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Summary observation for one model.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelObservation {
    /// Fraction of tasks that passed.
    pub pass_rate: f64,
    /// Total cost divided by the number of successful tasks.
    pub cost_per_success: f64,
    /// Average latency in milliseconds.
    pub avg_latency_ms: f64,
    /// Number of observations contributing to this summary.
    pub observations: u64,
}

/// Compute the cost-quality Pareto frontier over the supplied observations.
///
/// A model is dominated when another model is at least as good on both
/// metrics and strictly better on one of them.
#[must_use]
pub fn compute_pareto_frontier(stats: &HashMap<String, ModelObservation>) -> Vec<String> {
    let mut frontier = Vec::new();

    for (slug_a, obs_a) in stats {
        let dominated = stats.iter().any(|(slug_b, obs_b)| {
            slug_b != slug_a
                && obs_b.pass_rate >= obs_a.pass_rate
                && obs_b.cost_per_success <= obs_a.cost_per_success
                && (obs_b.pass_rate > obs_a.pass_rate
                    || obs_b.cost_per_success < obs_a.cost_per_success)
        });

        if !dominated {
            frontier.push(slug_a.clone());
        }
    }

    frontier.sort();
    frontier
}

// ─── Multi-objective Pareto frontier (LEARN-06) ─────────────────────────────

/// A single solution in the multi-objective Pareto space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParetoSolution {
    /// Objective values (e.g., cost, quality, latency).
    pub values: Vec<f64>,
    /// Model identifier this solution corresponds to.
    pub model_id: String,
}

/// Multi-objective Pareto frontier over an arbitrary number of objectives.
///
/// Each objective is assumed to be *maximized*. Callers that want to minimize
/// a metric (e.g. cost) should negate it before constructing `ParetoSolution`s.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParetoFrontier {
    /// Objective names (e.g., `["cost", "quality", "latency"]`).
    pub objectives: Vec<String>,
    /// Non-dominated solutions on the frontier.
    pub solutions: Vec<ParetoSolution>,
}

impl ParetoFrontier {
    /// Create a new frontier with no solutions.
    #[must_use]
    pub fn new(objectives: Vec<String>) -> Self {
        Self {
            objectives,
            solutions: Vec::new(),
        }
    }

    /// Compute the Pareto frontier from a set of candidate solutions.
    ///
    /// Returns the subset of candidates that are non-dominated.
    #[must_use]
    pub fn compute(objectives: Vec<String>, candidates: &[ParetoSolution]) -> Self {
        let solutions = compute_pareto_frontier_multi(candidates);
        Self {
            objectives,
            solutions,
        }
    }

    /// Number of solutions on the frontier.
    #[must_use]
    pub fn len(&self) -> usize {
        self.solutions.len()
    }

    /// Whether the frontier is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.solutions.is_empty()
    }

    /// Return the model IDs on the frontier.
    #[must_use]
    pub fn model_ids(&self) -> Vec<String> {
        self.solutions.iter().map(|s| s.model_id.clone()).collect()
    }
}

/// Returns `true` if `a` dominates `b` — i.e. `a` is at least as good in
/// every objective and strictly better in at least one.
#[must_use]
pub fn is_dominated(a: &ParetoSolution, b: &ParetoSolution) -> bool {
    let len = a.values.len().min(b.values.len());
    if len == 0 {
        return false;
    }

    let mut all_geq = true;
    let mut any_gt = false;

    for i in 0..len {
        if a.values[i] < b.values[i] {
            all_geq = false;
            break;
        }
        if a.values[i] > b.values[i] {
            any_gt = true;
        }
    }

    all_geq && any_gt
}

/// Compute the non-dominated set from a list of candidate solutions.
#[must_use]
pub fn compute_pareto_frontier_multi(candidates: &[ParetoSolution]) -> Vec<ParetoSolution> {
    let mut frontier = Vec::new();

    for candidate in candidates {
        let dominated = candidates
            .iter()
            .any(|other| other.model_id != candidate.model_id && is_dominated(other, candidate));

        if !dominated {
            frontier.push(candidate.clone());
        }
    }

    frontier.sort_by(|a, b| a.model_id.cmp(&b.model_id));
    frontier
}

#[cfg(test)]
mod tests {
    use super::{ModelObservation, compute_pareto_frontier};
    use std::collections::HashMap;

    #[test]
    fn pareto_frontier_keeps_non_dominated_models() {
        let mut stats = HashMap::new();
        stats.insert(
            "model-a".to_string(),
            ModelObservation {
                pass_rate: 0.90,
                cost_per_success: 10.0,
                avg_latency_ms: 1000.0,
                observations: 20,
            },
        );
        stats.insert(
            "model-b".to_string(),
            ModelObservation {
                pass_rate: 0.70,
                cost_per_success: 12.0,
                avg_latency_ms: 900.0,
                observations: 20,
            },
        );
        stats.insert(
            "model-c".to_string(),
            ModelObservation {
                pass_rate: 0.80,
                cost_per_success: 9.0,
                avg_latency_ms: 1100.0,
                observations: 20,
            },
        );

        let frontier = compute_pareto_frontier(&stats);

        assert_eq!(frontier, vec!["model-a".to_string(), "model-c".to_string()]);
    }

    #[test]
    fn is_dominated_basic() {
        use super::{ParetoSolution, is_dominated};

        let a = ParetoSolution {
            values: vec![0.9, 0.8, 0.7],
            model_id: "a".into(),
        };
        let b = ParetoSolution {
            values: vec![0.5, 0.5, 0.5],
            model_id: "b".into(),
        };
        // a dominates b (better in all objectives)
        assert!(is_dominated(&a, &b));
        // b does not dominate a
        assert!(!is_dominated(&b, &a));
    }

    #[test]
    fn multi_objective_frontier() {
        use super::{ParetoFrontier, ParetoSolution};

        let candidates = vec![
            // High quality, high cost (negate cost so higher = cheaper).
            ParetoSolution { values: vec![0.9, 0.3], model_id: "high-quality".into() },
            // Low quality, low cost.
            ParetoSolution { values: vec![0.3, 0.9], model_id: "low-cost".into() },
            // Dominated: worse quality than high-quality AND worse cost than low-cost,
            // but NOT dominated by either individually since each is only better on one axis.
            // To actually be dominated we need a point that loses on ALL axes to another.
            ParetoSolution { values: vec![0.2, 0.2], model_id: "dominated".into() },
        ];

        let frontier = ParetoFrontier::compute(
            vec!["quality".into(), "neg_cost".into()],
            &candidates,
        );

        // "dominated" (0.2, 0.2) is dominated by both "high-quality" (0.9>0.2, 0.3>0.2)
        // and "low-cost" (0.3>0.2, 0.9>0.2).
        assert_eq!(frontier.len(), 2);
        let ids = frontier.model_ids();
        assert!(ids.contains(&"high-quality".to_string()));
        assert!(ids.contains(&"low-cost".to_string()));
        assert!(!ids.contains(&"dominated".to_string()));
    }

    #[test]
    fn empty_frontier() {
        use super::ParetoFrontier;
        let frontier = ParetoFrontier::compute(vec!["a".into()], &[]);
        assert!(frontier.is_empty());
    }
}
