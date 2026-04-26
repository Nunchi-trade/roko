//! Spend / token budget and energy configuration sections.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---- [budget] ------------------------------------------------------------

/// Spend / token budget settings.
#[allow(clippy::derive_partial_eq_without_eq)] // contains f32
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetConfig {
    /// Max dollars to spend per plan.
    #[serde(default = "default_max_plan_usd")]
    pub max_plan_usd: f32,
    /// Max dollars per single agent turn.
    #[serde(default = "default_max_turn_usd")]
    pub max_turn_usd: f32,
    /// Token budget for prompt composition.
    #[serde(default = "default_prompt_token_budget")]
    pub prompt_token_budget: usize,
}

const fn default_max_plan_usd() -> f32 {
    25.0
}

const fn default_max_turn_usd() -> f32 {
    3.0
}

const fn default_prompt_token_budget() -> usize {
    10_000
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_plan_usd: default_max_plan_usd(),
            max_turn_usd: default_max_turn_usd(),
            prompt_token_budget: default_prompt_token_budget(),
        }
    }
}

// ---- [energy] ------------------------------------------------------------

/// Compute budget and cost caps per model tier.
///
/// Controls how much compute budget is available and how costs are capped
/// across different model tiers (cheap, standard, premium).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnergyConfig {
    /// Total compute budget pool in USD.
    #[serde(default = "default_energy_pool_usd")]
    pub pool_usd: f64,
    /// Per-task cost cap in USD (0.0 = no cap).
    #[serde(default)]
    pub per_task_cap_usd: f64,
    /// Per-tier cost multipliers keyed by tier name (e.g., "cheap": 0.5).
    #[serde(default)]
    pub tier_caps: HashMap<String, f64>,
    /// Metabolism rate: fraction of budget replenished per hour.
    #[serde(default = "default_energy_metabolism_rate")]
    pub metabolism_rate: f64,
}

const fn default_energy_pool_usd() -> f64 {
    50.0
}

const fn default_energy_metabolism_rate() -> f64 {
    0.1
}

impl Default for EnergyConfig {
    fn default() -> Self {
        Self {
            pool_usd: default_energy_pool_usd(),
            per_task_cap_usd: 0.0,
            tier_caps: HashMap::new(),
            metabolism_rate: default_energy_metabolism_rate(),
        }
    }
}
