//! Per-model pricing tables and cost normalization utilities.

use std::collections::HashMap;

use roko_agent::Usage;
use roko_core::config::schema::ModelProfile;
use serde::{Deserialize, Serialize};

/// Pricing for a single model slug.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelPricing {
    /// Cost in USD per million input tokens.
    pub input_per_m: f64,
    /// Cost in USD per million output tokens.
    pub output_per_m: f64,
    /// Cost in USD per million cache-read tokens.
    pub cache_read_per_m: f64,
    /// Cost in USD per million cache-write tokens.
    pub cache_write_per_m: f64,
    /// Tokenizer size ratio relative to OpenAI `o200k_base`.
    pub tokenizer_ratio: f64,
}

/// Per-model pricing table keyed by canonical model slug.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostTable {
    /// Pricing entries keyed by model slug.
    pub models: HashMap<String, ModelPricing>,
}

impl CostTable {
    /// Calculate request cost from raw token counts.
    #[must_use]
    pub fn calculate(&self, model_slug: &str, usage: &Usage) -> f64 {
        let pricing = match self.models.get(model_slug) {
            Some(pricing) => pricing,
            None => return 0.0,
        };

        (usage.input_tokens as f64 * pricing.input_per_m / 1_000_000.0)
            + (usage.output_tokens as f64 * pricing.output_per_m / 1_000_000.0)
            + (usage.cache_read_tokens as f64 * pricing.cache_read_per_m / 1_000_000.0)
            + (usage.cache_create_tokens as f64 * pricing.cache_write_per_m / 1_000_000.0)
    }

    /// Return the blended per-million-token cost normalized for tokenizer size.
    ///
    /// The blended value uses a 3:1 input/output weighting, matching the
    /// Artificial Analysis methodology described in the routing plan.
    #[must_use]
    pub fn blended_cost_per_m(&self, model_slug: &str) -> f64 {
        let pricing = match self.models.get(model_slug) {
            Some(pricing) => pricing,
            None => return 0.0,
        };

        ((3.0 * pricing.input_per_m + pricing.output_per_m) / 4.0) * pricing.tokenizer_ratio
    }

    /// Load pricing rows from config model profiles.
    #[must_use]
    pub fn from_config(models: &HashMap<String, ModelProfile>) -> Self {
        let mut table = HashMap::new();

        for profile in models.values() {
            if let (Some(input), Some(output)) =
                (profile.cost_input_per_m, profile.cost_output_per_m)
            {
                table.insert(
                    profile.slug.clone(),
                    ModelPricing {
                        input_per_m: input,
                        output_per_m: output,
                        cache_read_per_m: profile.cost_cache_read_per_m.unwrap_or(input * 0.5),
                        cache_write_per_m: profile.cost_cache_write_per_m.unwrap_or(input * 1.25),
                        tokenizer_ratio: profile.tokenizer_ratio.unwrap_or(1.0),
                    },
                );
            }
        }

        Self { models: table }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glm_5_1_profile() -> ModelProfile {
        ModelProfile {
            provider: "zai".into(),
            slug: "glm-5.1".into(),
            context_window: 200_000,
            max_output: Some(131_072),
            supports_tools: true,
            supports_thinking: true,
            supports_vision: false,
            supports_web_search: false,
            supports_mcp_tools: false,
            supports_partial: false,
            provider_routing: None,
            tool_format: "openai_json".into(),
            cost_input_per_m: Some(1.40),
            cost_output_per_m: Some(4.40),
            cost_cache_read_per_m: Some(0.26),
            cost_cache_write_per_m: Some(1.75),
            max_tools: None,
            tokenizer_ratio: Some(1.05),
        }
    }

    #[test]
    fn cost_table_calculate() {
        let mut models = HashMap::new();
        models.insert(
            "glm-5.1".into(),
            ModelPricing {
                input_per_m: 1.40,
                output_per_m: 4.40,
                cache_read_per_m: 0.26,
                cache_write_per_m: 1.75,
                tokenizer_ratio: 1.05,
            },
        );

        let table = CostTable { models };
        let usage = Usage {
            input_tokens: 1_000,
            output_tokens: 200,
            cache_read_tokens: 100,
            cache_create_tokens: 50,
            ..Usage::default()
        };

        let cost = table.calculate("glm-5.1", &usage);
        assert!((cost - 0.002_393_5).abs() < 1e-12);
    }

    #[test]
    fn blended_cost_uses_tokenizer_ratio() {
        let mut models = HashMap::new();
        models.insert(
            "glm-5.1".into(),
            ModelPricing {
                input_per_m: 1.40,
                output_per_m: 4.40,
                cache_read_per_m: 0.26,
                cache_write_per_m: 1.75,
                tokenizer_ratio: 1.05,
            },
        );

        let table = CostTable { models };
        let blended = table.blended_cost_per_m("glm-5.1");
        assert!((blended - 1.995).abs() < 1e-12);
    }

    #[test]
    fn from_config_loads_pricing_rows() {
        let mut profiles = HashMap::new();
        profiles.insert("glm-5.1".into(), glm_5_1_profile());

        let table = CostTable::from_config(&profiles);
        let pricing = table.models.get("glm-5.1").expect("pricing row");

        assert!((pricing.input_per_m - 1.40).abs() < 1e-12);
        assert!((pricing.output_per_m - 4.40).abs() < 1e-12);
        assert!((pricing.cache_read_per_m - 0.26).abs() < 1e-12);
        assert!((pricing.cache_write_per_m - 1.75).abs() < 1e-12);
        assert!((pricing.tokenizer_ratio - 1.05).abs() < 1e-12);
    }
}
