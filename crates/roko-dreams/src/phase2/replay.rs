//! Phase 2 NREM replay stubs.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::phase2::shared::{HdcVector, InsightRecord};

/// Replay modes described by the docs' prioritized NREM replay design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayMode {
    /// Replay the episode in its original temporal direction.
    Forward,
    /// Replay the episode backward from outcome to cause.
    Reverse,
    /// Replay a controlled perturbation of the original episode.
    Perturbed,
    /// Compress multiple related episodes into one structural replay.
    CompressedBatch,
}

/// Relationship between a replay insight and existing knowledge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InsightRelation {
    /// The replay confirms an existing knowledge entry.
    Confirms(String),
    /// The replay contradicts an existing knowledge entry.
    Contradicts(String),
    /// The replay appears novel relative to current knowledge.
    Novel,
}

/// Placeholder consolidator for replay insights entering durable storage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightConsolidator {
    /// Minimum confidence required before consolidation is attempted.
    pub min_confidence: f64,
    /// Similarity threshold for merging with an existing entry.
    pub merge_threshold: f32,
    /// Maximum number of insights processed per dream cycle.
    pub max_insights_per_cycle: usize,
}

/// Replay fidelity mode for an individual replayed episode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ReplayFidelity {
    /// Replay the episode as observed.
    Exact,
    /// Apply controlled perturbations within observed variance.
    Perturbed {
        /// Standard deviation of the perturbation process.
        perturbation_sigma: f64,
    },
    /// Generate a synthetic but structurally similar replay.
    Generative {
        /// Minimum structural similarity to the source episode.
        structural_similarity_floor: f32,
    },
}

/// Batch-level configuration for replay fidelity assignment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayFidelityConfig {
    /// Default fidelity mode used for standard replay.
    pub default_mode: ReplayFidelity,
    /// Fraction of exact replays reserved for anchor memories.
    pub exact_fraction: f64,
    /// Fraction of generative replays reserved for exploration.
    pub generative_fraction: f64,
    /// Minimum replay compression ratio.
    pub min_compression_ratio: f64,
    /// Maximum replay compression ratio.
    pub max_compression_ratio: f64,
}

/// SM-2-inspired replay scheduling configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayScheduleConfig {
    /// Initial easiness factor for new episodes.
    pub initial_easiness: f64,
    /// Minimum review interval.
    pub min_interval_hours: f64,
    /// Maximum review interval.
    pub max_interval_hours: f64,
    /// Quality score below which the interval resets.
    pub quality_reset_threshold: f64,
    /// Fraction of budget reserved for immediate replay.
    pub immediate_fraction: f64,
    /// Fraction of budget reserved for spaced review.
    pub spaced_fraction: f64,
    /// Fraction of budget reserved for exploratory review.
    pub exploration_fraction: f64,
}

/// Per-episode spacing state for spaced replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodicSpacingTracker {
    /// Replay history indexed by episode id.
    pub entries: HashMap<String, SpacingEntry>,
}

/// SM-2-like spacing state for one episode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpacingEntry {
    /// Episode identifier.
    pub episode_id: String,
    /// Easiness factor used to scale future intervals.
    pub easiness_factor: f64,
    /// Current review interval in hours.
    pub interval_hours: f64,
    /// Number of times this episode has been replayed.
    pub replay_count: u32,
    /// Quality score assigned on the most recent replay.
    pub last_quality: f64,
    /// Scheduled time for the next review.
    pub next_review_at: DateTime<Utc>,
}

/// Planning-integrated replay rollout configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdaptiveRolloutConfig {
    /// Minimum rollout length in imagined actions.
    pub min_rollout_length: usize,
    /// Maximum rollout length in imagined actions.
    pub max_rollout_length: usize,
    /// Prediction-error threshold above which longer rollouts are used.
    pub complexity_threshold: f64,
    /// Sampling temperature for imagined action sequences.
    pub rollout_temperature: f64,
    /// Whether to use forward and backward rollouts.
    pub bidirectional: bool,
    /// Fixed rollout cost used by the constant-cost model.
    pub fixed_rollout_cost_ms: u64,
    /// Per-step rollout cost used by the variable-cost model.
    pub per_step_cost_ms: u64,
    /// Whether to use the variable-cost model.
    pub use_variable_cost: bool,
}

/// Result of one planning-integrated replay rollout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RolloutResult {
    /// Episode being replayed.
    pub episode_id: String,
    /// Number of imagined steps taken.
    pub rollout_length: usize,
    /// Whether the rollout reached a goal state.
    pub goal_reached: bool,
    /// Change in the policy for the first imagined action.
    pub policy_delta: f64,
    /// Wall-clock cost of the rollout.
    pub rollout_cost_ms: u64,
    /// Insights extracted from the rollout.
    pub insights: Vec<InsightRecord>,
}

/// Goal-uncertain replay prioritization state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalEnsembleReplay {
    /// Maximum number of goals retained in the ensemble.
    pub max_goals: usize,
    /// Minimum probability for a goal to remain active.
    pub min_goal_probability: f64,
    /// Per-cycle decay factor for unsupported goals.
    pub goal_probability_decay: f64,
    /// Whether to allocate budget to goal-agnostic replay.
    pub enable_general_replay: bool,
    /// Fraction of budget allocated to general replay.
    pub general_replay_fraction: f64,
    /// Learning rate for replay-driven goal updates.
    pub replay_learning_rate: f64,
    /// Learning rate for behavioral tracking of active goals.
    pub behavioral_learning_rate: f64,
}

/// One goal hypothesis maintained by goal-ensemble replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalHypothesis {
    /// Stable goal identifier.
    pub goal_id: String,
    /// Human-readable goal description.
    pub description: String,
    /// Probability mass currently assigned to the goal.
    pub probability: f64,
    /// Optional centroid summarizing value under this goal.
    pub value_centroid: Option<HdcVector>,
    /// Number of supporting episodes observed so far.
    pub evidence_count: usize,
    /// Last time this goal received supporting evidence.
    pub last_evidence_at: Option<DateTime<Utc>>,
}

/// Slow-wave / spindle / ripple scheduling stub for grouped replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TripleCouplingScheduler {
    /// Interval between top-level dream cycles.
    pub so_period_mins: u64,
    /// Number of spindle bursts inside one slow-wave cycle.
    pub spindle_burst_count: usize,
    /// Number of replayed episodes per spindle burst.
    pub ripple_replay_per_burst: usize,
    /// How tightly ripples lock to spindle peaks.
    pub coupling_precision: f64,
    /// Whether replay timing is phase locked.
    pub phase_locked: bool,
}
