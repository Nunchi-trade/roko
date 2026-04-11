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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}
