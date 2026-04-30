//! Model routing configuration.

use serde::{Deserialize, Serialize};

// ---- [routing] -----------------------------------------------------------

/// Routing algorithm for model selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoutingAlgorithm {
    /// Contextual bandit using upper-confidence bounds.
    LinUcb,
    /// Discounted Thompson sampling for non-stationary routing.
    Thompson,
}

impl RoutingAlgorithm {
    /// Stable config label used in TOML.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::LinUcb => "linucb",
            Self::Thompson => "thompson",
        }
    }
}

impl Default for RoutingAlgorithm {
    fn default() -> Self {
        Self::LinUcb
    }
}

/// Reward weights used to scalarize quality, cost, and latency signals.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RewardWeights {
    /// Relative weight for quality / success.
    #[serde(default = "default_reward_weight_quality")]
    pub quality: f64,
    /// Relative weight for low cost.
    #[serde(default = "default_reward_weight_cost")]
    pub cost: f64,
    /// Relative weight for low latency.
    #[serde(default = "default_reward_weight_latency")]
    pub latency: f64,
    /// Relative weight for knowledge-informed routing bias.
    /// Falls back to `latency` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_bias: Option<f64>,
}

const fn default_reward_weight_quality() -> f64 {
    0.5
}

const fn default_reward_weight_cost() -> f64 {
    0.3
}

const fn default_reward_weight_latency() -> f64 {
    0.2
}

impl Default for RewardWeights {
    fn default() -> Self {
        Self {
            quality: default_reward_weight_quality(),
            cost: default_reward_weight_cost(),
            latency: default_reward_weight_latency(),
            knowledge_bias: None,
        }
    }
}

/// Per-tier reward-weight overrides for routing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutingRewardWeightsConfig {
    /// Default weights used when a tier has no explicit override.
    #[serde(flatten)]
    pub default: RewardWeights,
    /// Optional override for mechanical tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mechanical: Option<RewardWeights>,
    /// Optional override for focused tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused: Option<RewardWeights>,
    /// Optional override for integrative tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrative: Option<RewardWeights>,
    /// Optional override for architectural tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architectural: Option<RewardWeights>,
}

impl RoutingRewardWeightsConfig {
    /// Resolve the effective weights for a task tier.
    #[must_use]
    pub fn for_tier(&self, tier: &str) -> RewardWeights {
        match tier {
            "mechanical" => self.mechanical.unwrap_or(self.default),
            "focused" => self.focused.unwrap_or(self.default),
            "integrative" => self.integrative.unwrap_or(self.default),
            "architectural" => self.architectural.unwrap_or(self.default),
            _ => self.default,
        }
    }
}

impl Default for RoutingRewardWeightsConfig {
    fn default() -> Self {
        Self {
            default: RewardWeights::default(),
            mechanical: None,
            focused: None,
            integrative: None,
            architectural: None,
        }
    }
}

/// Model routing configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// Routing mode (`"auto_override"`).
    #[serde(default = "default_routing_mode")]
    pub mode: String,
    /// Online learning algorithm used by the router.
    #[serde(default)]
    pub algorithm: RoutingAlgorithm,
    /// Discount factor for Thompson sampling in non-stationary environments.
    #[serde(default = "default_routing_discount_factor")]
    pub discount_factor: f64,
    /// Model for low-complexity tasks.
    #[serde(default = "default_fast_model")]
    pub fast_task_model: String,
    /// Model for standard-complexity tasks.
    #[serde(default = "default_standard_model")]
    pub standard_task_model: String,
    /// Model for high-complexity / retry tasks.
    #[serde(default = "default_complex_model")]
    pub complex_task_model: String,
    /// Reward scalarization weights with optional per-tier overrides.
    #[serde(default)]
    pub weights: RoutingRewardWeightsConfig,
    /// Context strategy (`"mcp_first"`, `"hybrid"`, `"inline_heavy"`).
    #[serde(default = "default_context_strategy")]
    pub context_strategy: String,
}

fn default_routing_mode() -> String {
    "auto_override".into()
}

fn default_fast_model() -> String {
    "claude-haiku-4-5".into()
}

fn default_standard_model() -> String {
    "claude-sonnet-4-6".into()
}

fn default_complex_model() -> String {
    "claude-opus-4-6".into()
}

fn default_context_strategy() -> String {
    "mcp_first".into()
}

const fn default_routing_discount_factor() -> f64 {
    0.99
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            mode: default_routing_mode(),
            algorithm: RoutingAlgorithm::default(),
            discount_factor: default_routing_discount_factor(),
            fast_task_model: default_fast_model(),
            standard_task_model: default_standard_model(),
            complex_task_model: default_complex_model(),
            weights: RoutingRewardWeightsConfig::default(),
            context_strategy: default_context_strategy(),
        }
    }
}
