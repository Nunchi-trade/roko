//! Phase 2 dream-cycle and quality-reporting stubs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::phase2::shared::{DepotentiationSummary, InsightRecord, PatternRecord};
use crate::phase2::shared::ModelTier;

/// Three-phase dream state machine described by the docs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DreamPhase {
    /// No dream is currently in progress.
    Idle,
    /// NREM replay is processing prior episodes.
    NremReplay {
        /// Number of episodes selected for replay.
        episodes_to_replay: usize,
        /// Number of episodes already replayed.
        episodes_replayed: usize,
    },
    /// REM imagination is generating counterfactuals.
    RemImagination {
        /// Number of counterfactuals targeted for this phase.
        counterfactuals_to_generate: usize,
        /// Number of counterfactuals already generated.
        counterfactuals_generated: usize,
    },
    /// Integration is evaluating dream outputs.
    Integration {
        /// Number of hypotheses queued for evaluation.
        hypotheses_to_evaluate: usize,
        /// Number of hypotheses already evaluated.
        hypotheses_evaluated: usize,
    },
}

/// Extended dream state machine with transition phases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtendedDreamPhase {
    /// No dream is currently in progress.
    Idle,
    /// Waking-to-sleep transition where hypnagogia runs.
    HypnagogicTransition {
        /// Number of fragments emitted during transition.
        fragments_generated: usize,
        /// Number of fragments retained after filtering.
        fragments_retained: usize,
    },
    /// NREM replay phase.
    NremReplay {
        /// Number of episodes selected for replay.
        episodes_to_replay: usize,
        /// Number of episodes already replayed.
        episodes_replayed: usize,
    },
    /// Transition between NREM and REM.
    NremToRemTransition,
    /// REM imagination phase.
    RemImagination {
        /// Number of counterfactuals targeted for this phase.
        counterfactuals_to_generate: usize,
        /// Number of counterfactuals already generated.
        counterfactuals_generated: usize,
    },
    /// Integration phase.
    Integration {
        /// Number of hypotheses queued for evaluation.
        hypotheses_to_evaluate: usize,
        /// Number of hypotheses already evaluated.
        hypotheses_evaluated: usize,
    },
    /// Sleep-to-waking transition where dream summaries are prepared.
    HypnopompicTransition {
        /// Number of insights summarized for waking review.
        insights_summarized: usize,
    },
}

/// Short-form consolidation pass for brief idle gaps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MicroConsolidation {
    /// Minimum idle duration required to run micro-consolidation.
    pub min_idle_secs: u64,
    /// Maximum number of episodes replayed in a micro pass.
    pub max_micro_replays: usize,
    /// Whether micro-consolidation may stage new hypotheses.
    pub can_stage: bool,
    /// Preferred model tier for the short pass.
    pub model_tier: ModelTier,
}

/// Outcome event emitted when waking evidence updates a dream hypothesis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamOutcomeEvent {
    /// Hypothesis affected by the waking outcome.
    pub hypothesis_id: String,
    /// Whether the waking evidence validated the hypothesis.
    pub validated: bool,
    /// Updated confidence after incorporating the evidence.
    pub confidence: f64,
    /// Dream cycle identifier that produced the hypothesis.
    pub dream_cycle_origin: String,
    /// Waking episodes used for validation.
    pub validation_episodes: Vec<String>,
}

/// Aggregate dream quality metrics tracked across cycles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamQualityDashboard {
    /// Total cycles contributing to the dashboard.
    pub total_cycles: usize,
    /// Mean promotion rate for staged hypotheses.
    pub mean_promotion_rate: f64,
    /// Mean pairwise diversity across dream hypotheses.
    pub mean_hypothesis_diversity: f64,
    /// Estimated improvement in waking performance attributable to dreams.
    pub mean_waking_improvement: f64,
    /// Nightmare rate per dream cycle.
    pub nightmare_rate: f64,
    /// Waking improvement per dollar spent on dreaming.
    pub cost_efficiency: f64,
    /// Coarse quality trend over the recent window.
    pub trend: DreamQualityTrend,
}

/// Trend classification for the dream-quality dashboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DreamQualityTrend {
    /// Quality is trending upward.
    Improving {
        /// Estimated slope over the observation window.
        slope: f64,
    },
    /// Quality is effectively flat.
    Stable,
    /// Quality is trending downward.
    Declining {
        /// Estimated slope over the observation window.
        slope: f64,
    },
}

/// Target-state dream-cycle report from the Phase 2 docs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseTwoDreamCycleReport {
    /// When the dream cycle started.
    pub started_at: DateTime<Utc>,
    /// When the dream cycle completed.
    pub completed_at: DateTime<Utc>,
    /// Timestamp of the most recent episode processed.
    pub processed_through: Option<DateTime<Utc>>,
    /// Number of episodes replayed during NREM.
    pub episodes_replayed: usize,
    /// Number of counterfactuals generated during REM.
    pub counterfactuals_generated: usize,
    /// Insights extracted during the cycle.
    pub insights: Vec<InsightRecord>,
    /// Patterns discovered during consolidation.
    pub patterns: Vec<PatternRecord>,
    /// Number of staged hypotheses emitted by the cycle.
    pub staged_hypotheses: usize,
    /// Number of hypotheses promoted to durable knowledge.
    pub promoted_hypotheses: usize,
    /// Number of confidence updates applied to existing knowledge.
    pub confidence_updates: usize,
    /// Summary of emotional depotentiation.
    pub depotentiation: DepotentiationSummary,
}
