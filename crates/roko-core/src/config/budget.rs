//! Spend / token budget configuration section.

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
