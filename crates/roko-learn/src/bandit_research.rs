//! Research-oriented bandit shells for routing experiments.
//!
//! These types cover the documented Thompson-with-drift, neural bandit,
//! ensemble, and diagnostic sketches that sit beyond the core shipped
//! bandit module.

#![allow(dead_code)]

use crate::bandits::UcbBandit;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Thompson-sampling arm state with discounted updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThompsonArm {
    /// Model slug this arm represents.
    model: String,
    /// Success count plus Beta prior.
    alpha: f64,
    /// Failure count plus Beta prior.
    beta: f64,
    /// Total observations recorded for this arm.
    total_observations: u64,
}

impl ThompsonArm {
    /// Create a fresh Thompson arm with Beta(1, 1) priors.
    #[must_use]
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            alpha: 1.0,
            beta: 1.0,
            total_observations: 0,
        }
    }

    /// Apply a discounted reward update.
    pub fn apply_update(&mut self, reward: f64, gamma: f64) {
        let reward = reward.clamp(0.0, 1.0);
        let gamma = if gamma.is_finite() {
            gamma.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.alpha = gamma * self.alpha + reward;
        self.beta = gamma * self.beta + (1.0 - reward);
        self.total_observations = self.total_observations.saturating_add(1);
    }

    /// Reset the arm to an uninformative prior.
    pub fn reset(&mut self) {
        self.alpha = 1.0;
        self.beta = 1.0;
        self.total_observations = 0;
    }

    /// Read the model slug represented by this arm.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Read the total observation count.
    #[must_use]
    pub const fn total_observations(&self) -> u64 {
        self.total_observations
    }

    /// Read the current Beta parameters.
    #[must_use]
    pub const fn beta_parameters(&self) -> (f64, f64) {
        (self.alpha, self.beta)
    }
}

/// Neural reward network used by the NeuralUCB router.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeuralRewardNet {
    /// Input dimension (same as LinUCB: 18).
    input_dim: usize,
    /// Hidden layer sizes (default: [64, 32]).
    hidden_dims: Vec<usize>,
    /// Output dimension.
    output_dim: usize,
    /// Network parameters θ.
    params: Vec<f64>,
}

impl NeuralRewardNet {
    /// Create a network shell with explicit dimensions.
    #[must_use]
    pub fn new(input_dim: usize, hidden_dims: Vec<usize>, output_dim: usize) -> Self {
        Self {
            input_dim,
            hidden_dims,
            output_dim,
            params: Vec::new(),
        }
    }

    /// Replace the parameter vector.
    #[must_use]
    pub fn with_params(mut self, params: Vec<f64>) -> Self {
        self.params = params;
        self
    }

    /// Total number of trainable parameters currently stored.
    #[must_use]
    pub fn parameter_count(&self) -> usize {
        self.params.len()
    }
}

/// NeuralUCB router shell for nonlinear contextual bandits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeuralUCBRouter {
    /// Neural network f(x; θ) mapping context to predicted reward per arm.
    network: NeuralRewardNet,
    /// Per-arm gradient covariance matrix for exploration.
    gradient_covariance: HashMap<String, Vec<Vec<f64>>>,
    /// Exploration parameter (analogous to alpha in LinUCB).
    pub nu: f64,
    /// Regularization parameter (default: 1.0).
    pub lambda: f64,
    /// Training buffer for periodic network updates.
    training_buffer: Vec<(Vec<f64>, String, f64)>,
    /// Retrain every N observations (default: 50).
    pub retrain_interval: u32,
}

impl NeuralUCBRouter {
    /// Create a router shell around a neural reward model.
    #[must_use]
    pub fn new(network: NeuralRewardNet) -> Self {
        Self {
            network,
            gradient_covariance: HashMap::new(),
            nu: 1.0,
            lambda: 1.0,
            training_buffer: Vec::new(),
            retrain_interval: 50,
        }
    }

    /// Override the exploration parameter.
    #[must_use]
    pub fn with_nu(mut self, nu: f64) -> Self {
        self.nu = if nu.is_finite() { nu.max(0.0) } else { 1.0 };
        self
    }

    /// Override the regularization parameter.
    #[must_use]
    pub fn with_lambda(mut self, lambda: f64) -> Self {
        self.lambda = if lambda.is_finite() {
            lambda.max(0.0)
        } else {
            1.0
        };
        self
    }

    /// Replace the retraining interval.
    #[must_use]
    pub fn with_retrain_interval(mut self, retrain_interval: u32) -> Self {
        self.retrain_interval = retrain_interval.max(1);
        self
    }

    /// Buffer a context/arm/reward observation for later training.
    pub fn buffer_training_example(
        &mut self,
        context: Vec<f64>,
        arm: impl Into<String>,
        reward: f64,
    ) {
        self.training_buffer.push((context, arm.into(), reward));
    }

    /// Access the underlying network shell.
    #[must_use]
    pub const fn network(&self) -> &NeuralRewardNet {
        &self.network
    }

    /// Number of buffered training examples.
    #[must_use]
    pub fn buffered_examples(&self) -> usize {
        self.training_buffer.len()
    }
}

/// Marker trait for research bandit strategies.
pub trait BanditStrategy: Send + Sync {}

/// Ensemble of bandit strategies and a meta-bandit.
pub struct BanditEnsemble {
    /// Available bandit strategies.
    strategies: Vec<Box<dyn BanditStrategy>>,
    /// Meta-bandit that selects which strategy to use.
    meta_bandit: UcbBandit,
    /// Per-strategy performance tracking.
    strategy_stats: Vec<StrategyStats>,
    /// Correlation matrix between strategies.
    correlation_matrix: Vec<Vec<f64>>,
    /// Ensemble combination mode.
    pub mode: EnsembleMode,
}

impl BanditEnsemble {
    /// Create an empty ensemble shell.
    #[must_use]
    pub fn new(meta_bandit: UcbBandit, mode: EnsembleMode) -> Self {
        Self {
            strategies: Vec::new(),
            meta_bandit,
            strategy_stats: Vec::new(),
            correlation_matrix: Vec::new(),
            mode,
        }
    }

    /// Register a strategy and its initial statistics.
    pub fn add_strategy(&mut self, strategy: Box<dyn BanditStrategy>, stats: StrategyStats) {
        self.strategies.push(strategy);
        self.strategy_stats.push(stats);
        let next_size = self.strategies.len();
        for row in &mut self.correlation_matrix {
            row.resize(next_size, 0.0);
        }
        self.correlation_matrix.push(vec![0.0; next_size]);
    }

    /// Number of registered strategies.
    #[must_use]
    pub fn strategy_count(&self) -> usize {
        self.strategies.len()
    }

    /// Access the meta-bandit.
    #[must_use]
    pub const fn meta_bandit(&self) -> &UcbBandit {
        &self.meta_bandit
    }
}

/// Ensemble combination mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EnsembleMode {
    /// Meta-bandit selects one strategy per decision.
    MetaSelect,
    /// Weighted vote across all strategies.
    WeightedVote,
    /// Majority vote with tie-breaking by meta-bandit.
    MajorityVote,
    /// Switch strategy when current strategy's regret exceeds threshold.
    AdaptiveSwitch {
        /// Regret threshold that triggers a strategy switch.
        regret_threshold: f64,
    },
}

/// Per-strategy performance tracking.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StrategyStats {
    /// Strategy name.
    pub name: String,
    /// Cumulative reward under this strategy.
    pub cumulative_reward: f64,
    /// Number of times this strategy was selected.
    pub selections: u64,
    /// Running regret estimate.
    pub estimated_regret: f64,
    /// Recent performance over the last 50 decisions.
    pub recent_reward_rate: f64,
}

impl StrategyStats {
    /// Create empty statistics for a strategy.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }
}

/// Cumulative regret tracking for a bandit.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegretTracker {
    /// Per-decision regret: best-arm reward minus chosen-arm reward.
    pub per_decision_regret: Vec<f64>,
    /// Cumulative regret over time.
    pub cumulative_regret: Vec<f64>,
    /// Theoretical O(sqrt(T ln T)) bound for comparison.
    pub theoretical_bound: Vec<f64>,
}

impl RegretTracker {
    /// Create an empty regret tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a new regret sample and its comparison bound.
    pub fn record(&mut self, regret: f64, theoretical_bound: f64) {
        let regret = regret.max(0.0);
        let next_cumulative = self.cumulative_regret.last().copied().unwrap_or(0.0) + regret;
        self.per_decision_regret.push(regret);
        self.cumulative_regret.push(next_cumulative);
        self.theoretical_bound.push(theoretical_bound.max(0.0));
    }
}

/// Feature-importance summary for LinUCB-style models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureImportance {
    /// Feature name.
    pub feature_name: String,
    /// Feature dimension in the context vector.
    pub dimension: usize,
    /// Average absolute weight across all arms.
    pub avg_abs_weight: f64,
    /// Variance of weight across arms.
    pub weight_variance: f64,
}

impl FeatureImportance {
    /// Create a feature-importance record.
    #[must_use]
    pub fn new(
        feature_name: impl Into<String>,
        dimension: usize,
        avg_abs_weight: f64,
        weight_variance: f64,
    ) -> Self {
        Self {
            feature_name: feature_name.into(),
            dimension,
            avg_abs_weight: avg_abs_weight.max(0.0),
            weight_variance: weight_variance.max(0.0),
        }
    }
}

/// Diagnostic anomaly emitted by a bandit monitor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BanditAnomaly {
    /// One arm is selected more than 80% of the time.
    ArmLockIn {
        /// Locked-in arm identifier.
        arm: String,
        /// Selection rate for the locked-in arm.
        selection_rate: f64,
    },
    /// Exploration dropped below 5% before convergence.
    PrematureExploitation {
        /// Observed exploration rate.
        exploration_rate: f64,
        /// Number of observations seen.
        observations: u64,
    },
    /// Regret is growing faster than the theoretical bound.
    SuperlinearRegret {
        /// Actual regret observed.
        actual: f64,
        /// Theoretical regret bound.
        bound: f64,
    },
    /// Arm performance suddenly changed, likely due to a provider update.
    ArmPerformanceShift {
        /// Arm identifier.
        arm: String,
        /// Previous performance rate.
        old_rate: f64,
        /// Current performance rate.
        new_rate: f64,
    },
    /// Arms have similar performance and the bandit cannot distinguish them.
    IndistinguishableArms {
        /// Maximum observed performance gap.
        max_gap: f64,
    },
}
