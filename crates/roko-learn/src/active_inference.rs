//! Active inference helpers for tier routing.
//!
//! This module keeps the math small but concrete: a belief state over a
//! factorized latent space and an expected-free-energy style tier selector.
//! It is sufficient for routing support and for future integration into the
//! cascade router without introducing a new planning framework.

use roko_core::agent::{ModelTier, TaskRequirements};
use serde::{Deserialize, Serialize};

const STATE_COUNT: usize = 90;
const SKILL_LEVELS: usize = 3;
const CONFIDENCE_LEVELS: usize = 10;

/// Belief distribution over the 90 latent routing states.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeliefState {
    /// Flattened 3 x 3 x 10 probability table.
    pub probabilities: Vec<f64>,
    /// Number of updates incorporated into this belief state.
    pub updates: u64,
}

impl BeliefState {
    /// Uniform prior over all latent states.
    #[must_use]
    pub fn uniform() -> Self {
        Self {
            probabilities: vec![1.0 / STATE_COUNT as f64; STATE_COUNT],
            updates: 0,
        }
    }

    /// Renormalize the distribution after a Bayesian update.
    pub fn normalize(&mut self) {
        let total: f64 = self.probabilities.iter().sum();
        if total <= 0.0 || !total.is_finite() {
            self.probabilities.fill(1.0 / STATE_COUNT as f64);
            return;
        }

        for probability in &mut self.probabilities {
            *probability = (*probability / total).clamp(0.0, 1.0);
        }
    }

    /// Update the belief state after observing an outcome.
    pub fn observe(
        &mut self,
        requirements: &TaskRequirements,
        selected_tier: ModelTier,
        success: bool,
        cost_usd: f64,
        latency_ms: f64,
    ) {
        let task_difficulty = task_difficulty(requirements);
        for idx in 0..self.probabilities.len() {
            let (difficulty, skill, confidence) = decode_state(idx);
            let success_likelihood = success_likelihood(
                selected_tier,
                difficulty,
                skill,
                confidence,
                task_difficulty,
            );
            let cost_penalty = cost_penalty(selected_tier, cost_usd);
            let latency_penalty = latency_penalty(selected_tier, latency_ms);
            let likelihood = if success {
                success_likelihood
            } else {
                1.0 - success_likelihood
            } * (1.0 - cost_penalty)
                * (1.0 - latency_penalty);
            self.probabilities[idx] *= likelihood.clamp(0.01, 1.0);
        }
        self.updates += 1;
        self.normalize();
    }
}

/// Select the model tier that minimizes expected free energy.
#[must_use]
pub fn select_tier(belief: &BeliefState, requirements: &TaskRequirements) -> ModelTier {
    let task_difficulty = task_difficulty(requirements);
    if task_difficulty >= 2 {
        return ModelTier::Premium;
    }
    let tiers = [ModelTier::Fast, ModelTier::Standard, ModelTier::Premium];
    tiers
        .into_iter()
        .min_by(|left, right| {
            expected_free_energy(belief, *left, task_difficulty)
                .partial_cmp(&expected_free_energy(belief, *right, task_difficulty))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(ModelTier::Standard)
}

fn expected_free_energy(belief: &BeliefState, tier: ModelTier, task_difficulty: usize) -> f64 {
    let mut risk = 0.0;
    let mut ambiguity = 0.0;
    let mut evidence = 0.0;

    for (idx, probability) in belief.probabilities.iter().copied().enumerate() {
        let (difficulty, skill, confidence) = decode_state(idx);
        let success = success_likelihood(tier, difficulty, skill, confidence, task_difficulty);
        risk += probability * (1.0 - success);
        ambiguity += probability * (1.0 - confidence as f64 / (CONFIDENCE_LEVELS as f64 - 1.0));
        evidence += probability * (tier_cost(tier) + tier_latency(tier));
    }

    (risk + 0.20 * ambiguity + 0.10 * evidence).clamp(0.0, 1.5)
}

fn task_difficulty(requirements: &TaskRequirements) -> usize {
    let mut score = 0usize;
    if requirements.needs_web_search {
        score += 1;
    }
    if requirements.needs_code_execution {
        score += 1;
    }
    if requirements.needs_thinking {
        score += 1;
    }
    if requirements.needs_vision {
        score += 1;
    }
    if requirements.needs_structured_output {
        score += 1;
    }
    if requirements.min_context_window >= 120_000 {
        score += 1;
    }

    match score {
        0..=1 => 0,
        2 => 1,
        _ => 2,
    }
}

fn decode_state(index: usize) -> (usize, usize, usize) {
    let difficulty = index / (SKILL_LEVELS * CONFIDENCE_LEVELS);
    let skill = (index / CONFIDENCE_LEVELS) % SKILL_LEVELS;
    let confidence = index % CONFIDENCE_LEVELS;
    (difficulty, skill, confidence)
}

fn success_likelihood(
    tier: ModelTier,
    difficulty: usize,
    skill: usize,
    confidence: usize,
    task_difficulty: usize,
) -> f64 {
    let tier_strength = match tier {
        ModelTier::Fast => 0,
        ModelTier::Standard => 1,
        ModelTier::Premium => 2,
        _ => 1,
    } as isize;
    let skill_strength = skill as isize;
    let difficulty_gap = (task_difficulty as isize - tier_strength).abs() as f64;
    let latent_gap = (difficulty as isize - skill_strength).abs() as f64;
    let confidence_boost = confidence as f64 / (CONFIDENCE_LEVELS as f64 - 1.0);

    (0.8 - difficulty_gap * 0.18 - latent_gap * 0.12 + confidence_boost * 0.2).clamp(0.05, 0.95)
}

fn tier_cost(tier: ModelTier) -> f64 {
    match tier {
        ModelTier::Fast => 0.08,
        ModelTier::Standard => 0.18,
        ModelTier::Premium => 0.28,
        _ => 0.18,
    }
}

fn tier_latency(tier: ModelTier) -> f64 {
    match tier {
        ModelTier::Fast => 0.10,
        ModelTier::Standard => 0.18,
        ModelTier::Premium => 0.26,
        _ => 0.18,
    }
}

fn cost_penalty(tier: ModelTier, cost_usd: f64) -> f64 {
    let budget = match tier {
        ModelTier::Fast => 0.35,
        ModelTier::Standard => 0.65,
        ModelTier::Premium => 1.0,
        _ => 0.65,
    };
    (cost_usd / budget).clamp(0.0, 1.0) * 0.05
}

fn latency_penalty(tier: ModelTier, latency_ms: f64) -> f64 {
    let budget = match tier {
        ModelTier::Fast => 20_000.0,
        ModelTier::Standard => 60_000.0,
        ModelTier::Premium => 120_000.0,
        _ => 60_000.0,
    };
    (latency_ms / budget).clamp(0.0, 1.0) * 0.05
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requirements(
        needs_code_execution: bool,
        needs_thinking: bool,
        min_context_window: u64,
    ) -> TaskRequirements {
        TaskRequirements {
            needs_web_search: false,
            needs_code_execution,
            needs_thinking,
            needs_vision: false,
            needs_structured_output: false,
            min_context_window,
            max_cost_output_per_m: None,
            max_latency_ms: None,
        }
    }

    #[test]
    fn easy_requirements_choose_fast_tier() {
        let belief = BeliefState::uniform();
        let tier = select_tier(&belief, &requirements(false, false, 8_000));
        assert_eq!(tier, ModelTier::Fast);
    }

    #[test]
    fn harder_requirements_choose_premium_tier() {
        let belief = BeliefState::uniform();
        let tier = select_tier(&belief, &requirements(true, true, 160_000));
        assert_eq!(tier, ModelTier::Premium);
    }

    #[test]
    fn observation_updates_beliefs() {
        let mut belief = BeliefState::uniform();
        let req = requirements(true, true, 160_000);
        let before = belief.probabilities.clone();
        belief.observe(&req, ModelTier::Premium, true, 0.4, 8_000.0);

        assert_eq!(belief.updates, 1);
        assert_ne!(belief.probabilities, before);
        assert!((belief.probabilities.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    }
}
