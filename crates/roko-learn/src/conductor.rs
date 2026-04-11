//! Learned conductor intervention policy backed by a contextual bandit.
//!
//! The conductor sits above the normal retry loop and decides whether a
//! failing task should continue, receive a hint, escalate, restart, or abort.
//! This module keeps that choice data-driven by combining:
//!
//! - per-action Thompson posteriors from [`crate::model_router::ThompsonArm`]
//! - a lightweight linear context model over the current task state
//! - reward shaping that treats "fail fast" interventions as useful when they
//!   avoid obviously wasted retries

use crate::model_router::ThompsonArm;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::Path;

const CONTEXT_DIM: usize = 19;
const WEIGHT_LEARNING_RATE: f64 = 0.35;
const WEIGHT_CLAMP: f64 = 4.0;
const ITERATION_SCALE: f64 = 6.0;
const FAILURE_SCALE: f64 = 4.0;
const ELAPSED_SCALE_MS: f64 = 120_000.0;
const COST_SCALE_USD: f64 = 1.0;
const ACTION_BLEND_THOMPSON: f64 = 0.65;
const ACTION_BLEND_CONTEXT: f64 = 0.35;

const ACTIONS: [ConductorAction; 7] = [
    ConductorAction::Continue,
    ConductorAction::InjectHint(HintType::ErrorDigest),
    ConductorAction::InjectHint(HintType::SkillSuggestion),
    ConductorAction::InjectHint(HintType::SimplifyApproach),
    ConductorAction::SwitchModel,
    ConductorAction::Restart,
    ConductorAction::Abort,
];

/// Current conductor decision context.
#[derive(Debug, Clone)]
pub struct ConductorState {
    /// Current attempt number for the task or gate.
    pub iteration: u32,
    /// Number of failed attempts in a row.
    pub consecutive_failures: u32,
    /// Coarse classification of the failure shape.
    pub error_pattern: ErrorPattern,
    /// Wall-clock time already spent on this task.
    pub elapsed_ms: u64,
    /// Accumulated spend for the task so far.
    pub cost_so_far_usd: f64,
    /// Active model tier label such as `fast`, `standard`, or `premium`.
    pub model_tier: String,
    /// Task tier label such as `mechanical` or `architectural`.
    pub task_complexity: String,
}

/// Coarse error taxonomy used for contextual features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorPattern {
    /// No reliable diagnosis is available yet.
    Unknown,
    /// Compiler or static-analysis failure.
    Compile,
    /// Tests or assertions failed.
    Test,
    /// Tool invocation or tool-result mismatch failed.
    ToolCall,
    /// A timeout or long-running stall was observed.
    Timeout,
    /// Provider or infrastructure rate limiting occurred.
    RateLimit,
    /// The model exceeded context or output limits.
    ContextOverflow,
    /// The model refused or safety-filtered the task.
    Refusal,
    /// The system is stuck repeating the same failure mode.
    LoopDetected,
    /// Filesystem, process, or environment failure.
    Infrastructure,
}

/// Intervention the conductor may take for a failing task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConductorAction {
    /// Keep retrying without extra intervention.
    Continue,
    /// Inject a targeted hint into the next retry.
    InjectHint(HintType),
    /// Escalate to a different model tier.
    SwitchModel,
    /// Reset the attempt and try again from a cleaner starting point.
    Restart,
    /// Stop spending more retries on the task.
    Abort,
}

/// Specific hint variant the conductor can inject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HintType {
    /// Inject an enriched error summary before retrying.
    ErrorDigest,
    /// Suggest a relevant reusable skill or recipe.
    SkillSuggestion,
    /// Ask the agent to take a simpler implementation path.
    SimplifyApproach,
}

/// Contextual Thompson bandit over conductor interventions.
#[derive(Debug, Clone)]
pub struct ConductorBandit {
    arms: HashMap<ConductorAction, ThompsonArm>,
    arm_weights: HashMap<ConductorAction, Vec<f64>>,
    context_dim: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConductorBanditSnapshot {
    arms: HashMap<String, ThompsonArm>,
    arm_weights: HashMap<String, Vec<f64>>,
    context_dim: usize,
}

impl ConductorBandit {
    /// Create a fresh conductor policy with one Thompson arm per action.
    #[must_use]
    pub fn new() -> Self {
        let arms = ACTIONS
            .into_iter()
            .map(|action| (action, ThompsonArm::new(action.label())))
            .collect::<HashMap<_, _>>();
        let arm_weights = ACTIONS
            .into_iter()
            .map(|action| (action, vec![0.0; CONTEXT_DIM]))
            .collect::<HashMap<_, _>>();

        Self {
            arms,
            arm_weights,
            context_dim: CONTEXT_DIM,
        }
    }

    /// Load a persisted conductor policy or return a fresh policy when the
    /// snapshot is missing or invalid.
    #[must_use]
    pub fn load_or_new(path: &Path) -> Self {
        let Some(snapshot) = std::fs::read_to_string(path)
            .ok()
            .and_then(|contents| serde_json::from_str::<ConductorBanditSnapshot>(&contents).ok())
        else {
            return Self::new();
        };

        let mut bandit = Self::new();
        bandit.context_dim = snapshot.context_dim.max(CONTEXT_DIM);

        for (label, arm) in snapshot.arms {
            if let Some(action) = ConductorAction::from_label(&label) {
                bandit.arms.insert(action, arm);
            }
        }

        for (label, weights) in snapshot.arm_weights {
            if let Some(action) = ConductorAction::from_label(&label) {
                let mut resized = weights;
                resized.resize(bandit.context_dim, 0.0);
                bandit.arm_weights.insert(action, resized);
            }
        }

        bandit
    }

    /// Persist the current conductor policy as JSON.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let snapshot = ConductorBanditSnapshot {
            arms: self
                .arms
                .iter()
                .map(|(action, arm)| (action.label().to_string(), arm.clone()))
                .collect(),
            arm_weights: self
                .arm_weights
                .iter()
                .map(|(action, weights)| (action.label().to_string(), weights.clone()))
                .collect(),
            context_dim: self.context_dim,
        };
        let json = serde_json::to_string_pretty(&snapshot)
            .map_err(|err| io::Error::other(format!("serialize conductor snapshot: {err}")))?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Choose the highest-scoring intervention for the current state.
    #[must_use]
    pub fn select_action(&self, state: &ConductorState) -> ConductorAction {
        let features = self.encode_state(state);
        let mut best_action = ConductorAction::Continue;
        let mut best_score = f64::NEG_INFINITY;

        for action in ACTIONS {
            let score = self.sampled_action_score(action, &features);
            if score > best_score {
                best_score = score;
                best_action = action;
            }
        }

        best_action
    }

    /// Feed the observed outcome from a chosen action back into the policy.
    pub fn record_outcome(
        &mut self,
        state: &ConductorState,
        action: ConductorAction,
        success: bool,
    ) {
        let features = self.encode_state(state);
        let reward = self.reward_for_outcome(state, action, success);
        let effective_success = success || reward >= 0.5;

        let observations = if let Some(arm) = self.arms.get_mut(&action) {
            arm.update(reward, effective_success);
            arm.observations
        } else {
            0
        };

        if let Some(weights) = self.arm_weights.get_mut(&action) {
            let predicted = sigmoid(dot(weights, &features));
            let error = reward - predicted;
            let step = WEIGHT_LEARNING_RATE / (1.0 + observations as f64 * 0.05);

            for (weight, feature) in weights.iter_mut().zip(features) {
                *weight = (*weight + step * error * feature).clamp(-WEIGHT_CLAMP, WEIGHT_CLAMP);
            }
        }
    }

    fn encode_state(&self, state: &ConductorState) -> Vec<f64> {
        let mut x = vec![0.0; self.context_dim];

        let iteration = normalize(state.iteration as f64, ITERATION_SCALE);
        let failures = normalize(state.consecutive_failures as f64, FAILURE_SCALE);
        let failure_ge_3 = f64::from(state.consecutive_failures >= 3);
        let elapsed = normalize(state.elapsed_ms as f64, ELAPSED_SCALE_MS);
        let cost = normalize(state.cost_so_far_usd, COST_SCALE_USD);

        let tier = model_tier_bucket(&state.model_tier);
        let complexity = complexity_bucket(&state.task_complexity);

        x[0] = 1.0;
        x[1] = iteration;
        x[2] = failures;
        x[3] = failure_ge_3;
        x[4] = elapsed;
        x[5] = cost;
        x[6] = f64::from(matches!(tier, ModelTierBucket::Fast));
        x[7] = f64::from(matches!(tier, ModelTierBucket::Standard));
        x[8] = f64::from(matches!(tier, ModelTierBucket::Premium));
        x[9] = f64::from(matches!(complexity, ComplexityBucket::Mechanical));
        x[10] = f64::from(matches!(complexity, ComplexityBucket::Focused));
        x[11] = f64::from(matches!(complexity, ComplexityBucket::Integrative));
        x[12] = f64::from(matches!(complexity, ComplexityBucket::Architectural));
        x[13] = f64::from(matches!(
            state.error_pattern,
            ErrorPattern::Compile | ErrorPattern::Test | ErrorPattern::ToolCall
        ));
        x[14] = f64::from(matches!(
            state.error_pattern,
            ErrorPattern::Timeout | ErrorPattern::RateLimit | ErrorPattern::Infrastructure
        ));
        x[15] = f64::from(matches!(
            state.error_pattern,
            ErrorPattern::ContextOverflow | ErrorPattern::Refusal | ErrorPattern::LoopDetected
        ));
        x[16] = x[3] * x[9];
        x[17] = x[3] * x[6];
        x[18] = x[4] * x[5];

        x
    }

    fn sampled_action_score(&self, action: ConductorAction, features: &[f64]) -> f64 {
        let thompson = self.arms.get(&action).map_or(0.5, ThompsonArm::sample);
        let context = self.context_score(action, features);
        ACTION_BLEND_THOMPSON * thompson + ACTION_BLEND_CONTEXT * context
    }

    fn context_score(&self, action: ConductorAction, features: &[f64]) -> f64 {
        self.arm_weights
            .get(&action)
            .map_or(0.5, |weights| sigmoid(dot(weights, features)))
    }

    fn reward_for_outcome(
        &self,
        state: &ConductorState,
        action: ConductorAction,
        success: bool,
    ) -> f64 {
        if success {
            return match action {
                ConductorAction::Continue => 1.0,
                ConductorAction::InjectHint(_) => 0.92,
                ConductorAction::SwitchModel => 0.88,
                ConductorAction::Restart => 0.82,
                ConductorAction::Abort => 0.0,
            };
        }

        let futility = futility_score(state);
        let complexity = complexity_bucket(&state.task_complexity);

        match action {
            ConductorAction::Continue => 0.15 * (1.0 - futility),
            ConductorAction::InjectHint(hint) => hint_failure_reward(hint, state, futility),
            ConductorAction::SwitchModel => {
                (0.10 + 0.65 * futility * switch_bias(complexity)).clamp(0.0, 0.95)
            }
            ConductorAction::Restart => {
                (0.10 + 0.60 * futility * restart_bias(state.error_pattern)).clamp(0.0, 0.95)
            }
            ConductorAction::Abort => {
                (0.05 + 0.75 * futility * abort_bias(complexity)).clamp(0.0, 0.95)
            }
        }
    }

    #[cfg(test)]
    fn best_action_for_state(&self, state: &ConductorState) -> ConductorAction {
        let features = self.encode_state(state);
        ACTIONS
            .into_iter()
            .max_by(|a, b| {
                self.expected_action_score(*a, &features)
                    .partial_cmp(&self.expected_action_score(*b, &features))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(ConductorAction::Continue)
    }

    #[cfg(test)]
    fn expected_action_score(&self, action: ConductorAction, features: &[f64]) -> f64 {
        let posterior_mean = self
            .arms
            .get(&action)
            .map_or(0.5, |arm| arm.alpha / (arm.alpha + arm.beta));
        let context = self.context_score(action, features);
        ACTION_BLEND_THOMPSON * posterior_mean + ACTION_BLEND_CONTEXT * context
    }
}

impl Default for ConductorBandit {
    fn default() -> Self {
        Self::new()
    }
}

impl ConductorAction {
    fn label(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::InjectHint(HintType::ErrorDigest) => "inject_error_digest",
            Self::InjectHint(HintType::SkillSuggestion) => "inject_skill_suggestion",
            Self::InjectHint(HintType::SimplifyApproach) => "inject_simplify_approach",
            Self::SwitchModel => "switch_model",
            Self::Restart => "restart",
            Self::Abort => "abort",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "continue" => Some(Self::Continue),
            "inject_error_digest" => Some(Self::InjectHint(HintType::ErrorDigest)),
            "inject_skill_suggestion" => Some(Self::InjectHint(HintType::SkillSuggestion)),
            "inject_simplify_approach" => Some(Self::InjectHint(HintType::SimplifyApproach)),
            "switch_model" => Some(Self::SwitchModel),
            "restart" => Some(Self::Restart),
            "abort" => Some(Self::Abort),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelTierBucket {
    Fast,
    Standard,
    Premium,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComplexityBucket {
    Mechanical,
    Focused,
    Integrative,
    Architectural,
    Other,
}

fn model_tier_bucket(label: &str) -> ModelTierBucket {
    match label.trim().to_ascii_lowercase().as_str() {
        "fast" | "cheap" | "haiku" | "mini" => ModelTierBucket::Fast,
        "standard" | "balanced" | "medium" => ModelTierBucket::Standard,
        "premium" | "complex" | "high" | "opus" => ModelTierBucket::Premium,
        _ => ModelTierBucket::Other,
    }
}

fn complexity_bucket(label: &str) -> ComplexityBucket {
    match label.trim().to_ascii_lowercase().as_str() {
        "mechanical" | "fast" | "low" => ComplexityBucket::Mechanical,
        "focused" | "standard" | "medium" => ComplexityBucket::Focused,
        "integrative" => ComplexityBucket::Integrative,
        "architectural" | "complex" | "high" => ComplexityBucket::Architectural,
        _ => ComplexityBucket::Other,
    }
}

fn futility_score(state: &ConductorState) -> f64 {
    let failure_pressure = if state.consecutive_failures >= 3 {
        1.0
    } else {
        normalize(state.consecutive_failures as f64, 3.0)
    };

    let iteration_pressure = normalize(state.iteration as f64, ITERATION_SCALE);
    let time_pressure = normalize(state.elapsed_ms as f64, ELAPSED_SCALE_MS);
    let cost_pressure = normalize(state.cost_so_far_usd, COST_SCALE_USD);

    let error_pressure = match state.error_pattern {
        ErrorPattern::Unknown => 0.30,
        ErrorPattern::Compile => 0.35,
        ErrorPattern::Test => 0.40,
        ErrorPattern::ToolCall => 0.50,
        ErrorPattern::Timeout => 0.60,
        ErrorPattern::RateLimit => 0.55,
        ErrorPattern::ContextOverflow => 0.70,
        ErrorPattern::Refusal => 0.65,
        ErrorPattern::LoopDetected => 1.0,
        ErrorPattern::Infrastructure => 0.80,
    };

    (0.55 * failure_pressure
        + 0.20 * iteration_pressure
        + 0.10 * time_pressure
        + 0.05 * cost_pressure
        + 0.10 * error_pressure)
        .clamp(0.0, 1.0)
}

fn hint_failure_reward(hint: HintType, state: &ConductorState, futility: f64) -> f64 {
    let complexity = complexity_bucket(&state.task_complexity);
    let centered = (1.0 - ((futility - 0.45).abs() / 0.45)).clamp(0.0, 1.0);

    let hint_fit = match hint {
        HintType::ErrorDigest => match state.error_pattern {
            ErrorPattern::Compile | ErrorPattern::Test | ErrorPattern::ToolCall => 1.0,
            ErrorPattern::Unknown => 0.8,
            _ => 0.55,
        },
        HintType::SkillSuggestion => match complexity {
            ComplexityBucket::Focused | ComplexityBucket::Integrative => 0.95,
            ComplexityBucket::Architectural => 0.75,
            ComplexityBucket::Mechanical => 0.45,
            ComplexityBucket::Other => 0.55,
        },
        HintType::SimplifyApproach => match complexity {
            ComplexityBucket::Mechanical => 1.0,
            ComplexityBucket::Focused => 0.85,
            ComplexityBucket::Integrative => 0.55,
            ComplexityBucket::Architectural => 0.35,
            ComplexityBucket::Other => 0.50,
        },
    };

    (0.15 + 0.40 * centered * hint_fit).clamp(0.0, 0.70)
}

fn switch_bias(complexity: ComplexityBucket) -> f64 {
    match complexity {
        ComplexityBucket::Mechanical => 0.25,
        ComplexityBucket::Focused => 0.60,
        ComplexityBucket::Integrative => 0.85,
        ComplexityBucket::Architectural => 1.0,
        ComplexityBucket::Other => 0.50,
    }
}

fn restart_bias(pattern: ErrorPattern) -> f64 {
    match pattern {
        ErrorPattern::LoopDetected | ErrorPattern::ToolCall | ErrorPattern::Infrastructure => 1.0,
        ErrorPattern::Timeout | ErrorPattern::RateLimit => 0.75,
        ErrorPattern::Compile | ErrorPattern::Test | ErrorPattern::Unknown => 0.35,
        ErrorPattern::ContextOverflow | ErrorPattern::Refusal => 0.45,
    }
}

fn abort_bias(complexity: ComplexityBucket) -> f64 {
    match complexity {
        ComplexityBucket::Mechanical => 1.0,
        ComplexityBucket::Focused => 0.45,
        ComplexityBucket::Integrative => 0.20,
        ComplexityBucket::Architectural => 0.10,
        ComplexityBucket::Other => 0.30,
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(lhs, rhs)| lhs * rhs).sum()
}

fn normalize(value: f64, scale: f64) -> f64 {
    if scale <= 0.0 {
        0.0
    } else {
        (value / scale).clamp(0.0, 1.0)
    }
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x.clamp(-20.0, 20.0)).exp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mechanical_failure_state() -> ConductorState {
        ConductorState {
            iteration: 4,
            consecutive_failures: 4,
            error_pattern: ErrorPattern::Compile,
            elapsed_ms: 35_000,
            cost_so_far_usd: 0.03,
            model_tier: "fast".to_string(),
            task_complexity: "mechanical".to_string(),
        }
    }

    #[test]
    fn conductor_bandit_learns_abort_after_repeated_mechanical_failures() {
        let state = mechanical_failure_state();
        let mut bandit = ConductorBandit::new();

        for _ in 0..50 {
            bandit.record_outcome(&state, ConductorAction::Continue, false);
            bandit.record_outcome(&state, ConductorAction::Abort, false);
        }

        assert_eq!(bandit.best_action_for_state(&state), ConductorAction::Abort);

        let continue_arm = bandit
            .arms
            .get(&ConductorAction::Continue)
            .expect("continue arm");
        let abort_arm = bandit.arms.get(&ConductorAction::Abort).expect("abort arm");

        assert!(continue_arm.beta > continue_arm.alpha);
        assert!(abort_arm.alpha > abort_arm.beta);
    }

    #[test]
    fn conductor_bandit_sampling_tracks_learned_abort_preference() {
        let state = mechanical_failure_state();
        let mut bandit = ConductorBandit::new();

        for _ in 0..50 {
            bandit.record_outcome(&state, ConductorAction::Continue, false);
            bandit.record_outcome(&state, ConductorAction::Abort, false);
        }

        let mut abort_count = 0;
        for _ in 0..32 {
            if bandit.select_action(&state) == ConductorAction::Abort {
                abort_count += 1;
            }
        }

        assert!(
            abort_count >= 24,
            "expected abort to dominate Thompson samples, got {abort_count}/32"
        );
    }
}
