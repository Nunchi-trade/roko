//! Phase 2 hypnagogia stubs.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::phase2::shared::{HdcVector, ModelTier};

/// A single fragment produced during hypnagogia.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HypnagogicFragment {
    /// Stable fragment identifier.
    pub id: String,
    /// Raw fragment text emitted by the generator.
    pub raw_text: String,
    /// Source knowledge entries that triggered the fragment.
    pub source_entries: Vec<String>,
    /// Novelty score assigned by the observer.
    pub novelty: f64,
    /// Relevance score assigned by the observer.
    pub relevance: f64,
    /// Coherence score assigned by the observer.
    pub coherence: f64,
    /// Distilled one-line summary, when retained.
    pub distilled: Option<String>,
    /// HDC vector encoding the fragment.
    pub hdc_vector: HdcVector,
    /// When the fragment was created.
    pub created_at: DateTime<Utc>,
    /// Whether the observer retained the fragment.
    pub retained: bool,
}

/// Complete session record for one hypnagogic pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HypnagogicSession {
    /// Stable session identifier.
    pub id: String,
    /// Focus vector at session start.
    pub focus_vector: HdcVector,
    /// All fragments generated during the session.
    pub fragments: Vec<HypnagogicFragment>,
    /// Identifiers of retained fragments.
    pub retained_fragments: Vec<String>,
    /// Wall-clock duration of the session.
    pub duration: Duration,
    /// Token and cost budget consumed.
    pub budget: HypnagogiaBudget,
    /// Configuration used by the session.
    pub config: HypnagogiaConfig,
}

/// Budget accounting for one hypnagogia session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HypnagogiaBudget {
    /// Total input tokens consumed.
    pub input_tokens: usize,
    /// Total output tokens consumed.
    pub output_tokens: usize,
    /// Estimated session cost in USD.
    pub estimated_cost: f64,
}

/// Runtime configuration for the hypnagogia engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HypnagogiaConfig {
    /// Whether hypnagogia is enabled.
    pub enabled: bool,
    /// Number of thalamic fragments sampled.
    pub thalamic_fragments: usize,
    /// Number of Dali-interrupt fragments sampled.
    pub dali_fragments: usize,
    /// Maximum tokens per Dali fragment.
    pub dali_max_tokens: usize,
    /// Minimum novelty required for retention.
    pub min_novelty: f64,
    /// Minimum relevance required for retention.
    pub min_relevance: f64,
    /// Minimum coherence required for retention.
    pub min_coherence: f64,
    /// Maximum budget spent per session.
    pub max_budget: f64,
    /// Whether to gate hypnagogia on remaining dream budget.
    pub budget_gate_enabled: bool,
    /// Minimum remaining budget fraction required to run.
    pub budget_gate_threshold: f64,
}

/// Archive-based novelty filter for retained fragments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoveltyFilter {
    /// Number of neighbors considered during novelty scoring.
    pub k_neighbors: usize,
    /// Minimum novelty score for archive inclusion.
    pub novelty_threshold: f64,
    /// Maximum archive size.
    pub max_archive_size: usize,
    /// Weight assigned to novelty in the serendipity score.
    pub serendipity_novelty_weight: f64,
    /// Weight assigned to relevance in the serendipity score.
    pub serendipity_relevance_weight: f64,
    /// Minimum serendipity score required for retention.
    pub min_serendipity: f64,
}

/// Pipeline turning retained fragments into actionable hypotheses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HypnagogiaInsightPipeline {
    /// Whether retained fragments are semantically elaborated.
    pub elaborate_fragments: bool,
    /// Maximum elaboration tokens per fragment.
    pub elaboration_max_tokens: usize,
    /// Model tier used for elaboration.
    pub elaboration_model_tier: ModelTier,
    /// Initial confidence assigned to hypnagogia hypotheses.
    pub initial_confidence: f64,
    /// Maximum elaborations performed per session.
    pub max_elaborations: usize,
    /// Strength of goal priming during retrieval.
    pub goal_priming_weight: f64,
}

/// Temperature curve controlling the alpha-to-theta transition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HypnagogicTemperatureCurve {
    /// Temperature at the alpha end of the transition.
    pub alpha_temperature: f64,
    /// Temperature at the theta end of the transition.
    pub theta_temperature: f64,
    /// Progress point where the midpoint temperature is reached.
    pub transition_midpoint: f64,
    /// Steepness of the transition curve.
    pub transition_steepness: f64,
    /// Whether to add biological-looking micro-oscillations.
    pub micro_oscillations: bool,
    /// Amplitude of the micro-oscillations.
    pub oscillation_amplitude: f64,
}

impl HypnagogicTemperatureCurve {
    /// Compute temperature for a normalized progress value.
    #[must_use]
    pub fn temperature_at(&self, progress: f64) -> f64 {
        let logistic = 1.0
            / (1.0 + (-(progress - self.transition_midpoint) * self.transition_steepness).exp());
        let base =
            self.alpha_temperature + (self.theta_temperature - self.alpha_temperature) * logistic;
        if self.micro_oscillations {
            base + self.oscillation_amplitude * (progress * std::f64::consts::TAU * 3.0).sin()
        } else {
            base
        }
    }
}

/// Configuration for targeted dream incubation during hypnagogia.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetedDreamIncubation {
    /// Whether targeted incubation is enabled.
    pub enabled: bool,
    /// Source of the incubation cue.
    pub cue_source: IncubationCueSource,
    /// Number of fragments between cue reinforcements.
    pub reinforcement_interval: usize,
    /// Desired semantic distance from the cue topic.
    pub semantic_distance_target: f64,
}

/// Source of the cue used for targeted dream incubation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IncubationCueSource {
    /// Use the current active task.
    CurrentTask,
    /// Use the most recent failure.
    RecentFailure,
    /// Use the topic with highest unresolved prediction error.
    HighestPredictionError,
    /// Use a manually supplied topic.
    Manual {
        /// Topic that should seed the incubation pass.
        topic: String,
    },
}
