//! Composite C-Factor metrics for dashboard and learning feedback.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Composite C-Factor snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CFactor {
    /// 0.0-1.0 composite score.
    pub overall: f64,
    /// Component breakdown for the score.
    pub components: CFactorComponents,
    /// Timestamp when the score was computed.
    pub computed_at: DateTime<Utc>,
    /// Number of episodes used in the calculation.
    pub episode_count: usize,
}

/// Individual C-Factor components.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CFactorComponents {
    /// % of tasks passing gates on first attempt.
    pub gate_pass_rate: f64,
    /// Inverse of cost per successful task, normalized.
    pub cost_efficiency: f64,
    /// Inverse of time per successful task, normalized.
    pub speed: f64,
    /// % of tasks succeeding without re-plan.
    pub first_try_rate: f64,
    /// Rate of new knowledge entries per episode.
    pub knowledge_growth: f64,
}

impl Default for CFactorComponents {
    fn default() -> Self {
        Self {
            gate_pass_rate: 0.0,
            cost_efficiency: 0.0,
            speed: 0.0,
            first_try_rate: 0.0,
            knowledge_growth: 0.0,
        }
    }
}

impl Default for CFactor {
    fn default() -> Self {
        Self {
            overall: 0.0,
            components: CFactorComponents::default(),
            computed_at: Utc::now(),
            episode_count: 0,
        }
    }
}
