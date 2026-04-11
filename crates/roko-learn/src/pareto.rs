//! Pareto frontier computation for model cost-quality tradeoffs.
//!
//! This module identifies which models are non-dominated with respect to
//! pass rate and cost per successful task. A model is Pareto-optimal if no
//! other model has both a higher pass rate and a lower cost per successful
//! task.

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

#[cfg(test)]
mod tests {
    use super::{compute_pareto_frontier, ModelObservation};
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
}
