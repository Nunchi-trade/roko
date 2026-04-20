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
    /// Fraction of tasks that passed (higher is better).
    pub pass_rate: f64,
    /// Total cost divided by the number of successful tasks (lower is better).
    pub cost_per_success: f64,
    /// Average latency in milliseconds (lower is better).
    pub avg_latency_ms: f64,
    /// Fraction of non-error responses (higher is better).
    /// Derived as `non_error_responses / total_responses`.
    pub reliability: f64,
    /// Number of observations contributing to this summary.
    pub observations: u64,
}

/// Configurable weights for Pareto scalarization.
///
/// Allows operators to prioritize certain objectives over others
/// (e.g. cost over latency). All weights should be non-negative;
/// they are internally normalized.
#[derive(Debug, Clone, PartialEq)]
pub struct ParetoWeights {
    /// Weight for quality (pass rate).
    pub quality: f64,
    /// Weight for cost (inverted: lower cost is better).
    pub cost: f64,
    /// Weight for latency (inverted: lower latency is better).
    pub latency: f64,
    /// Weight for reliability (higher is better).
    pub reliability: f64,
}

impl Default for ParetoWeights {
    fn default() -> Self {
        Self {
            quality: 1.0,
            cost: 1.0,
            latency: 1.0,
            reliability: 1.0,
        }
    }
}

impl ParetoWeights {
    /// Create equal weights.
    #[must_use]
    pub fn equal() -> Self {
        Self::default()
    }

    /// Create weights emphasizing quality.
    #[must_use]
    pub fn quality_first() -> Self {
        Self {
            quality: 3.0,
            cost: 1.0,
            latency: 1.0,
            reliability: 1.0,
        }
    }

    /// Create weights emphasizing cost.
    #[must_use]
    pub fn cost_first() -> Self {
        Self {
            quality: 1.0,
            cost: 3.0,
            latency: 1.0,
            reliability: 1.0,
        }
    }
}

/// Scalarize a `ModelObservation` into a single score using weighted combination.
///
/// Quality and reliability are maximized; cost and latency are minimized.
/// Cost and latency are inverted (1 - normalized) so that higher = better.
#[must_use]
pub fn scalarize(obs: &ModelObservation, weights: &ParetoWeights) -> f64 {
    let total_weight = (weights.quality + weights.cost + weights.latency + weights.reliability)
        .max(f64::EPSILON);

    // Normalize cost_per_success: assume 100.0 as a reasonable max.
    let cost_normalized = 1.0 - (obs.cost_per_success / 100.0).clamp(0.0, 1.0);
    // Normalize latency: assume 60_000 ms as a reasonable max.
    let latency_normalized = 1.0 - (obs.avg_latency_ms / 60_000.0).clamp(0.0, 1.0);

    (weights.quality * obs.pass_rate
        + weights.cost * cost_normalized
        + weights.latency * latency_normalized
        + weights.reliability * obs.reliability)
        / total_weight
}

/// Compute the 4-objective Pareto frontier over the supplied observations.
///
/// A model is dominated when another model is at least as good on ALL FOUR
/// objectives and strictly better on at least one:
/// - pass_rate: higher is better
/// - cost_per_success: lower is better
/// - avg_latency_ms: lower is better
/// - reliability: higher is better
#[must_use]
pub fn compute_pareto_frontier(stats: &HashMap<String, ModelObservation>) -> Vec<String> {
    let mut frontier = Vec::new();

    for (slug_a, obs_a) in stats {
        let dominated = stats.iter().any(|(slug_b, obs_b)| {
            if slug_b == slug_a {
                return false;
            }
            // b must be >= a on all objectives (using correct direction).
            let quality_ok = obs_b.pass_rate >= obs_a.pass_rate;
            let cost_ok = obs_b.cost_per_success <= obs_a.cost_per_success;
            let latency_ok = obs_b.avg_latency_ms <= obs_a.avg_latency_ms;
            let reliability_ok = obs_b.reliability >= obs_a.reliability;

            // Must be at least as good on all.
            let all_geq = quality_ok && cost_ok && latency_ok && reliability_ok;

            // Must be strictly better on at least one.
            let any_strictly_better = obs_b.pass_rate > obs_a.pass_rate
                || obs_b.cost_per_success < obs_a.cost_per_success
                || obs_b.avg_latency_ms < obs_a.avg_latency_ms
                || obs_b.reliability > obs_a.reliability;

            all_geq && any_strictly_better
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
    use super::{ModelObservation, ParetoWeights, compute_pareto_frontier, scalarize};
    use std::collections::HashMap;

    #[test]
    fn pareto_frontier_keeps_non_dominated_models_4d() {
        let mut stats = HashMap::new();
        // model-a: best quality but worst latency and cost.
        stats.insert(
            "model-a".to_string(),
            ModelObservation {
                pass_rate: 0.95,
                cost_per_success: 15.0,
                avg_latency_ms: 2000.0,
                reliability: 0.90,
                observations: 20,
            },
        );
        // model-b: worst on everything.
        stats.insert(
            "model-b".to_string(),
            ModelObservation {
                pass_rate: 0.60,
                cost_per_success: 20.0,
                avg_latency_ms: 3000.0,
                reliability: 0.70,
                observations: 20,
            },
        );
        // model-c: best cost and latency but lower quality.
        stats.insert(
            "model-c".to_string(),
            ModelObservation {
                pass_rate: 0.80,
                cost_per_success: 5.0,
                avg_latency_ms: 500.0,
                reliability: 0.95,
                observations: 20,
            },
        );

        let frontier = compute_pareto_frontier(&stats);

        // model-a and model-c are non-dominated (tradeoff between quality and cost/latency).
        // model-b is dominated by model-c on ALL 4 objectives.
        assert!(frontier.contains(&"model-a".to_string()));
        assert!(frontier.contains(&"model-c".to_string()));
        assert!(!frontier.contains(&"model-b".to_string()));
    }

    #[test]
    fn pareto_frontier_all_on_frontier_when_no_dominance() {
        let mut stats = HashMap::new();
        stats.insert(
            "model-a".to_string(),
            ModelObservation {
                pass_rate: 0.90,
                cost_per_success: 10.0,
                avg_latency_ms: 1000.0,
                reliability: 0.80,
                observations: 20,
            },
        );
        stats.insert(
            "model-b".to_string(),
            ModelObservation {
                pass_rate: 0.70,
                cost_per_success: 12.0,
                avg_latency_ms: 900.0,
                reliability: 0.95,
                observations: 20,
            },
        );

        let frontier = compute_pareto_frontier(&stats);
        // Neither dominates the other (model-a better quality, model-b better latency + reliability).
        assert_eq!(frontier.len(), 2);
    }

    #[test]
    fn scalarize_equal_weights() {
        let obs = ModelObservation {
            pass_rate: 0.90,
            cost_per_success: 10.0,
            avg_latency_ms: 1000.0,
            reliability: 0.95,
            observations: 20,
        };
        let score = scalarize(&obs, &ParetoWeights::equal());
        assert!(score > 0.0 && score <= 1.0, "scalarized score = {score}");
    }

    #[test]
    fn scalarize_quality_first_favors_high_pass_rate() {
        let high_quality = ModelObservation {
            pass_rate: 0.95,
            cost_per_success: 50.0,
            avg_latency_ms: 5000.0,
            reliability: 0.90,
            observations: 20,
        };
        let low_quality = ModelObservation {
            pass_rate: 0.40,
            cost_per_success: 5.0,
            avg_latency_ms: 500.0,
            reliability: 0.95,
            observations: 20,
        };
        let qw = ParetoWeights::quality_first();
        assert!(
            scalarize(&high_quality, &qw) > scalarize(&low_quality, &qw),
            "quality-first should prefer high pass rate"
        );
    }

    #[test]
    fn scalarize_cost_first_favors_cheap_model() {
        let expensive = ModelObservation {
            pass_rate: 0.95,
            cost_per_success: 80.0,
            avg_latency_ms: 1000.0,
            reliability: 0.90,
            observations: 20,
        };
        let cheap = ModelObservation {
            pass_rate: 0.60,
            cost_per_success: 2.0,
            avg_latency_ms: 1000.0,
            reliability: 0.90,
            observations: 20,
        };
        let cw = ParetoWeights::cost_first();
        assert!(
            scalarize(&cheap, &cw) > scalarize(&expensive, &cw),
            "cost-first should prefer cheap model"
        );
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
