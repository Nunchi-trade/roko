//! Types for A/B testing model selection.
//!
//! This module defines the data model for model experiments. The execution
//! logic, assignment strategy, and persistence are added in later tasks.

use crate::prompt_experiment::ExperimentStatus;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A model A/B experiment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelExperiment {
    /// Unique experiment identifier.
    pub experiment_id: String,
    /// Human-readable description of the experiment.
    pub description: String,
    /// Optional role scope for the experiment.
    pub role: Option<String>,
    /// Optional task category scope for the experiment.
    pub task_category: Option<String>,
    /// Variants available in the experiment.
    pub variants: Vec<ModelVariant>,
    /// Per-variant statistics keyed by variant id.
    pub stats: HashMap<String, ModelVariantStats>,
    /// Current experiment status.
    pub status: ExperimentStatus,
    /// Winner variant id, if concluded.
    pub winner_id: Option<String>,
    /// Minimum trials per variant before the experiment can conclude.
    pub min_trials_per_variant: u64,
    /// Minimum effect size required to declare a winner.
    pub min_effect_size: f64,
    /// Experiment creation timestamp in ISO-8601 format.
    pub created_at: String,
}

/// A single model variant participating in an experiment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelVariant {
    /// Unique identifier for this variant.
    pub id: String,
    /// Key into the `[models.*]` configuration table.
    pub model_key: String,
    /// API model slug.
    pub slug: String,
    /// Provider key for the model.
    pub provider: String,
}

/// Per-variant stats for a model experiment.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelVariantStats {
    /// Number of trials run for this variant.
    pub trials: u64,
    /// Number of successful trials.
    pub successes: u64,
    /// Total cost accumulated in USD.
    pub total_cost_usd: f64,
    /// Total tokens consumed.
    pub total_tokens: u64,
    /// Total duration accumulated in milliseconds.
    pub total_duration_ms: u64,
    /// Success rate derived from `successes / trials`.
    pub pass_rate: f64,
    /// Average cost per trial in USD.
    pub avg_cost_usd: f64,
    /// Cost per successful trial in USD.
    pub cost_per_success: f64,
    /// Average duration per trial in milliseconds.
    pub avg_duration_ms: f64,
}

impl ModelVariantStats {
    /// Recompute derived metrics from the accumulated counters.
    fn recalculate(&mut self) {
        if self.trials == 0 {
            self.pass_rate = 0.0;
            self.avg_cost_usd = 0.0;
            self.cost_per_success = 0.0;
            self.avg_duration_ms = 0.0;
            return;
        }

        self.pass_rate = self.successes as f64 / self.trials as f64;
        self.avg_cost_usd = self.total_cost_usd / self.trials as f64;
        self.avg_duration_ms = self.total_duration_ms as f64 / self.trials as f64;
        self.cost_per_success = if self.successes == 0 {
            0.0
        } else {
            self.total_cost_usd / self.successes as f64
        };
    }

    /// UCB1 score for variant selection.
    #[allow(clippy::cast_precision_loss)]
    fn ucb_score(&self, total_trials: u64) -> f64 {
        if self.trials == 0 || total_trials == 0 {
            return f64::MAX;
        }

        let mean = self.successes as f64 / self.trials as f64;
        let exploration = (2.0 * (total_trials as f64).ln() / self.trials as f64).sqrt();
        mean + exploration
    }
}

impl ModelExperiment {
    /// Select the next model variant to use.
    ///
    /// Concluded experiments always return the winner. Running experiments
    /// use UCB1, with unsampled variants selected before sampled ones.
    pub fn assign_variant(&self) -> Option<&ModelVariant> {
        if self.status == ExperimentStatus::Concluded {
            return self
                .variants
                .iter()
                .find(|variant| Some(&variant.id) == self.winner_id.as_ref());
        }

        let total: u64 = self.stats.values().map(|stats| stats.trials).sum();
        let mut best = None;
        let mut best_score = f64::NEG_INFINITY;

        for variant in &self.variants {
            let score = self
                .stats
                .get(&variant.id)
                .map(|stats| stats.ucb_score(total))
                .unwrap_or(f64::MAX);
            if score > best_score {
                best_score = score;
                best = Some(variant);
            }
        }

        best
    }

    /// Record an outcome for a model variant and update experiment state.
    pub fn record_outcome(
        &mut self,
        variant_id: &str,
        success: bool,
        cost_usd: f64,
        tokens: u64,
        duration_ms: u64,
    ) {
        let stats = self
            .stats
            .entry(variant_id.to_string())
            .or_default();
        stats.trials += 1;
        if success {
            stats.successes += 1;
        }
        stats.total_cost_usd += cost_usd;
        stats.total_tokens += tokens;
        stats.total_duration_ms += duration_ms;
        stats.recalculate();

        if self.status == ExperimentStatus::Running {
            if let Some(winner_id) = self.check_conclusion() {
                self.status = ExperimentStatus::Concluded;
                self.winner_id = Some(winner_id);
            }
        }
    }

    /// Check whether the experiment has enough evidence to conclude.
    fn check_conclusion(&self) -> Option<String> {
        if self.variants.is_empty() {
            return None;
        }

        let mut ranked = Vec::with_capacity(self.variants.len());
        for variant in &self.variants {
            let stats = self.stats.get(&variant.id)?;
            if stats.trials < self.min_trials_per_variant {
                return None;
            }
            ranked.push((variant.id.as_str(), stats.pass_rate));
        }

        if ranked.len() == 1 {
            return Some(ranked[0].0.to_string());
        }

        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let (best_id, best_rate) = ranked[0];
        let (_, second_rate) = ranked[1];

        if best_rate - second_rate >= self.min_effect_size {
            Some(best_id.to_string())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_experiment_types() {
        let experiment = ModelExperiment {
            experiment_id: "glm-vs-kimi".into(),
            description: "Compare models for implementer tasks".into(),
            role: Some("implementer".into()),
            task_category: Some("implementation".into()),
            variants: vec![
                ModelVariant {
                    id: "glm".into(),
                    model_key: "glm-5-1".into(),
                    slug: "glm-5.1".into(),
                    provider: "zai".into(),
                },
                ModelVariant {
                    id: "kimi".into(),
                    model_key: "kimi-k2-5".into(),
                    slug: "kimi-k2.5".into(),
                    provider: "moonshot".into(),
                },
            ],
            stats: HashMap::from([(
                "glm".into(),
                ModelVariantStats {
                    trials: 12,
                    successes: 9,
                    total_cost_usd: 2.4,
                    total_tokens: 18_000,
                    total_duration_ms: 54_000,
                    pass_rate: 0.75,
                    avg_cost_usd: 0.2,
                    cost_per_success: 0.266_666_666_7,
                    avg_duration_ms: 4_500.0,
                },
            )]),
            status: ExperimentStatus::Running,
            winner_id: None,
            min_trials_per_variant: 20,
            min_effect_size: 0.05,
            created_at: "2026-04-11T00:00:00Z".into(),
        };

        let json = serde_json::to_string(&experiment).expect("serialize");
        let decoded: ModelExperiment = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(decoded.experiment_id, "glm-vs-kimi");
        assert_eq!(decoded.variants.len(), 2);
        assert_eq!(decoded.stats["glm"].trials, 12);
        assert_eq!(decoded.status, ExperimentStatus::Running);
    }

    #[test]
    fn model_experiment_ucb() {
        let mut experiment = ModelExperiment {
            experiment_id: "glm-vs-kimi".into(),
            description: "Compare models for implementer tasks".into(),
            role: Some("implementer".into()),
            task_category: Some("implementation".into()),
            variants: vec![
                ModelVariant {
                    id: "glm".into(),
                    model_key: "glm-5-1".into(),
                    slug: "glm-5.1".into(),
                    provider: "zai".into(),
                },
                ModelVariant {
                    id: "kimi".into(),
                    model_key: "kimi-k2-5".into(),
                    slug: "kimi-k2.5".into(),
                    provider: "moonshot".into(),
                },
            ],
            stats: HashMap::new(),
            status: ExperimentStatus::Running,
            winner_id: None,
            min_trials_per_variant: 1,
            min_effect_size: 0.05,
            created_at: "2026-04-11T00:00:00Z".into(),
        };

        assert_eq!(experiment.assign_variant().map(|v| v.id.as_str()), Some("glm"));

        experiment.record_outcome("glm", true, 1.0, 100, 1_000);
        assert_eq!(experiment.assign_variant().map(|v| v.id.as_str()), Some("kimi"));

        experiment.record_outcome("kimi", false, 1.0, 100, 1_000);

        assert_eq!(experiment.status, ExperimentStatus::Concluded);
        assert_eq!(experiment.winner_id.as_deref(), Some("glm"));
        assert_eq!(experiment.assign_variant().map(|v| v.id.as_str()), Some("glm"));
        assert_eq!(experiment.stats["glm"].pass_rate, 1.0);
        assert_eq!(experiment.stats["glm"].avg_cost_usd, 1.0);
        assert_eq!(experiment.stats["glm"].cost_per_success, 1.0);
        assert_eq!(experiment.stats["glm"].avg_duration_ms, 1_000.0);
    }
}
