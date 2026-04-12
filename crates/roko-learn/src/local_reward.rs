//! Local reward functions that estimate global success from local decisions.
//!
//! Each learning subsystem can maintain its own [`LocalRewardFunction`]
//! keyed by a subsystem-specific decision string such as a routing choice or
//! prompt-section inclusion decision. The reward function learns from
//! `(local_decision, global_outcome)` pairs and returns an empirical estimate
//! of how strongly that local decision correlates with global task success.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Local reward function that predicts global outcome from local decisions.
///
/// Unknown decisions fall back to a neutral `0.5` prior until observations
/// accumulate.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalRewardFunction {
    /// Historical `(successes, total)` counts per local decision key.
    decision_outcomes: HashMap<String, (u64, u64)>,
}

impl LocalRewardFunction {
    /// Create an empty local reward function with no observations.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Estimate how good a local decision is from historical global outcomes.
    ///
    /// Returns a value in `[0.0, 1.0]`. Unknown decisions use a neutral
    /// `0.5` prior.
    #[allow(clippy::cast_precision_loss)]
    pub fn score(&self, decision_key: &str) -> f64 {
        self.decision_outcomes
            .get(decision_key)
            .map(|(successes, total)| *successes as f64 / (*total).max(1) as f64)
            .unwrap_or(0.5)
    }

    /// Update the reward function after observing the global task outcome.
    pub fn observe(&mut self, decision_key: &str, global_success: bool) {
        let entry = self
            .decision_outcomes
            .entry(decision_key.to_owned())
            .or_insert((0, 0));
        entry.1 = entry.1.saturating_add(1);
        if global_success {
            entry.0 = entry.0.saturating_add(1);
        }
    }

    /// Return the raw `(successes, total)` counts for one decision key.
    #[must_use]
    pub fn counts(&self, decision_key: &str) -> Option<(u64, u64)> {
        self.decision_outcomes.get(decision_key).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::LocalRewardFunction;

    #[test]
    fn local_reward_functions_use_neutral_prior_for_unknown_decisions() {
        let reward = LocalRewardFunction::new();
        assert_eq!(reward.score("glm-5.1:implementer:integrative"), 0.5);
        assert_eq!(reward.counts("glm-5.1:implementer:integrative"), None);
    }

    #[test]
    fn local_reward_functions_update_empirical_success_rates() {
        let mut reward = LocalRewardFunction::new();
        let decision = "workspace_map:included";

        reward.observe(decision, true);
        reward.observe(decision, false);
        reward.observe(decision, true);

        assert_eq!(reward.counts(decision), Some((2, 3)));
        assert!((reward.score(decision) - (2.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn local_reward_functions_predict_global_success_above_sixty_percent_after_100_episodes() {
        let mut reward = LocalRewardFunction::new();
        let good = "glm-5.1:implementer:integrative";
        let bad = "workspace_map:omitted";

        for episode in 0..100 {
            let (decision, success) = if episode % 2 == 0 {
                (good, episode % 10 != 0)
            } else {
                (bad, episode % 10 == 1)
            };
            reward.observe(decision, success);
        }

        let eval_episodes = [
            (good, true),
            (good, true),
            (good, true),
            (good, false),
            (good, true),
            (bad, false),
            (bad, false),
            (bad, false),
            (bad, true),
            (bad, false),
        ];

        let correct = eval_episodes
            .iter()
            .filter(|(decision, success)| (reward.score(decision) >= 0.5) == *success)
            .count();
        let accuracy = correct as f64 / eval_episodes.len() as f64;

        assert!(
            accuracy > 0.6,
            "expected >60% prediction accuracy after training, got {accuracy:.2}"
        );
        assert!(reward.score(good) > 0.6);
        assert!(reward.score(bad) < 0.4);
    }
}
