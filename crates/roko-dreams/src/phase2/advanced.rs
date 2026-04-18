//! Phase 2 advanced dream-concept stubs.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::phase2::shared::{Hypothesis, ModelTier};
use crate::DreamTrigger;

/// Nightmare classes recognized by the advanced dream safety layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NightmareClass {
    /// Harmful strategy generation.
    HarmfulStrategyGeneration,
    /// Discovery of a dangerous tool chain.
    DangerousToolChainDiscovery,
    /// Safety-constraint bypass.
    SafetyConstraintBypass,
    /// Direct policy violation.
    PolicyViolation,
}

/// Final decision applied to a detected nightmare.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NightmareDecision {
    /// Reject the nightmare output.
    Rejected,
    /// Approve only after modification.
    ApprovedWithModification {
        /// Modified hypothesis allowed to proceed.
        modified_hypothesis: String,
    },
    /// Approve without changes.
    ApprovedAsIs,
}

/// Nightmare detector configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NightmareDetector {
    /// Model tier used by the harmful-content classifier.
    pub classifier_model_tier: ModelTier,
    /// Whether a domain-specific safety check is enabled.
    pub enable_domain_check: bool,
    /// Capability delta threshold for Stage 3 escalation.
    pub capability_delta_threshold: f64,
    /// Entropy threshold above which human review is required.
    pub escalation_entropy_threshold: f64,
    /// Path to the nightmare log.
    pub nightmare_log_path: PathBuf,
    /// Number of cycles to cool down after a nightmare.
    pub post_nightmare_cooldown_cycles: usize,
}

/// Report emitted when the nightmare detector fires.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NightmareReport {
    /// Stable nightmare identifier.
    pub nightmare_id: String,
    /// Dream cycle in which the nightmare was detected.
    pub cycle_id: String,
    /// Agent that produced the nightmare.
    pub agent_id: String,
    /// Detection time.
    pub detected_at: DateTime<Utc>,
    /// Human-readable summary of the hypothesis.
    pub hypothesis_summary: String,
    /// Detection stage that raised the flag.
    pub detection_stage: u8,
    /// Nightmare class assigned by the detector.
    pub nightmare_class: NightmareClass,
    /// Classifier score for the hypothesis.
    pub classifier_score: f64,
    /// Capability delta if one was measured.
    pub capability_delta: Option<f64>,
    /// Escalation entropy if one was measured.
    pub escalation_entropy: Option<f64>,
    /// Whether a human has reviewed the nightmare.
    pub human_reviewed: bool,
    /// Final human decision, if any.
    pub human_decision: Option<NightmareDecision>,
}

/// Containment state for detected nightmares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NightmareContainment {
    /// Quarantined hypotheses by summary or id.
    pub quarantined_hypotheses: Vec<String>,
    /// Reports pending human review.
    pub pending_human_reviews: Vec<NightmareReport>,
    /// Remaining cooldown cycles after a nightmare.
    pub cooldown_remaining: usize,
    /// Path where nightmare events are logged.
    pub log_path: PathBuf,
}

impl NightmareContainment {
    /// Queue a nightmare for containment and human review.
    pub async fn quarantine(&mut self, report: NightmareReport) -> anyhow::Result<()> {
        self.quarantined_hypotheses
            .push(report.hypothesis_summary.clone());
        self.pending_human_reviews.push(report);
        self.cooldown_remaining = self.cooldown_remaining.max(1);
        todo!("Phase 2+: persist nightmare reports to JSONL and notify human review")
    }
}

/// Persistent dream journal configuration and state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamJournal {
    /// Path to the JSONL journal file.
    pub journal_path: PathBuf,
    /// In-memory cycle index for fast lookup.
    pub cycle_index: Vec<String>,
    /// Cached trend analysis.
    pub cached_trend: Option<DreamTrendAnalysis>,
    /// Number of cycles between trend recomputation.
    pub trend_recompute_interval: usize,
}

/// One persistent journal entry for a completed dream cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamJournalEntry {
    /// Dream cycle identifier.
    pub cycle_id: String,
    /// Agent that ran the cycle.
    pub agent_id: String,
    /// Start time of the cycle.
    pub cycle_start: DateTime<Utc>,
    /// End time of the cycle.
    pub cycle_end: DateTime<Utc>,
    /// Trigger that started the cycle.
    pub trigger: DreamTrigger,
    /// NREM duration in seconds.
    pub nrem_duration_secs: u64,
    /// REM duration in seconds.
    pub rem_duration_secs: u64,
    /// Integration duration in seconds.
    pub consolidation_duration_secs: u64,
    /// Total hypotheses generated.
    pub hypotheses_generated: usize,
    /// Total hypotheses staged.
    pub hypotheses_staged: usize,
    /// Total hypotheses promoted.
    pub hypotheses_promoted: usize,
    /// Total hypotheses later refuted.
    pub hypotheses_refuted: usize,
    /// Number of nightmares detected during the cycle.
    pub nightmares_detected: usize,
    /// Whether human review was required.
    pub human_review_required: bool,
    /// Mean pairwise HDC diversity across generated hypotheses.
    pub hypothesis_diversity: f64,
    /// Total token-equivalent compute consumed by the cycle.
    pub total_tokens: u64,
    /// Whether the cycle terminated early.
    pub early_termination: bool,
    /// Reason for early termination, if any.
    pub early_termination_reason: Option<String>,
}

/// Trend analysis over dream-journal history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamTrendAnalysis {
    /// Analysis timestamp.
    pub analyzed_at: DateTime<Utc>,
    /// Number of cycles included in the analysis.
    pub cycle_count: usize,
    /// Promotion rate per creativity mode.
    pub promotion_rate_by_mode: HashMap<String, f64>,
    /// Cycle duration that maximized promotion rate.
    pub optimal_duration_secs: u64,
    /// Mean diversity across analyzed cycles.
    pub mean_diversity: f64,
    /// Nightmares per cycle.
    pub nightmare_rate: f64,
    /// Whether nightmare rate is trending upward.
    pub nightmare_rate_increasing: bool,
    /// Promotion rate for failure-triggered cycles.
    pub failure_trigger_promotion_rate: f64,
    /// Promotion rate for scheduled cycles.
    pub scheduled_trigger_promotion_rate: f64,
}

/// Monitor for mid-cycle degeneration or lucid-dream drift.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LucidDreamMonitor {
    /// Minimum diversity tolerated before a warning.
    pub diversity_threshold: f64,
    /// Minimum novelty tolerated over the rolling window.
    pub novelty_decay_threshold: f64,
    /// Number of recent hypotheses considered for novelty decay.
    pub novelty_window_size: usize,
    /// Whether coherence-collapse checks are enabled.
    pub enable_coherence_check: bool,
    /// Number of failing signals required for early termination.
    pub early_termination_signal_count: usize,
    /// Number of hypotheses between checks.
    pub check_interval: usize,
}

impl LucidDreamMonitor {
    /// Evaluate the current cycle state and optionally request termination.
    #[must_use]
    pub fn evaluate(&self, hypotheses: &[Hypothesis]) -> Option<String> {
        if hypotheses.len() >= self.check_interval && hypotheses.len() < self.novelty_window_size {
            return Some("phase-2 lucid monitoring pending fuller diversity analysis".to_string());
        }
        None
    }
}

/// Neuro-informed extension of lucid-dream monitoring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NeuroinformedLucidMonitor {
    /// Minimum metacognitive microstate ratio required for lucidity.
    pub min_metacognitive_ratio: f64,
    /// Number of hypotheses considered in the microstate window.
    pub microstate_window: usize,
    /// Minimum information density per hypothesis.
    pub min_information_density: f64,
    /// Whether the monitor should auto-intervene.
    pub auto_intervene: bool,
    /// Prompt injected when intervention occurs.
    pub intervention_prompt: String,
}

/// Computational microstate used by lucid-dream monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputationalMicrostate {
    /// Self-referential reasoning.
    SelfReferential,
    /// Structured executive reasoning.
    Executive,
    /// Emotionally driven reasoning.
    Emotional,
    /// Default-mode associative drift.
    DefaultMode,
    /// Replay-dominated sensory reasoning.
    SensoryReplay,
}

/// Validity tracker for temporally drifting shared dream insights.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemporalValidityTracker {
    /// Maximum age before revalidation is required.
    pub max_age_before_revalidation_hours: u64,
    /// Drift threshold used to flag stale insights.
    pub drift_threshold: f64,
    /// Number of recent episodes used for drift detection.
    pub drift_detection_window: usize,
    /// Whether aged insights are downgraded automatically.
    pub auto_downgrade: bool,
    /// Confidence reduction per failed revalidation.
    pub revalidation_failure_penalty: f64,
}

/// Environment snapshot captured when a dream insight was generated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightEnvironmentSnapshot {
    /// Mean episode success rate at generation time.
    pub success_rate: f64,
    /// Task-type distribution at generation time.
    pub task_type_distribution: HashMap<String, f64>,
    /// Active tools available when the insight was generated.
    pub active_tools: Vec<String>,
    /// Gate thresholds in force at generation time.
    pub gate_thresholds: HashMap<String, f64>,
    /// Snapshot timestamp.
    pub snapshot_at: DateTime<Utc>,
}

/// Constitutional self-critique chain for nightmare screening.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstitutionalSelfCritique {
    /// Number of critique rounds run before external classification.
    pub critique_rounds: usize,
    /// Temperature used for self-critique.
    pub critique_temperature: f64,
    /// Principles checked during the critique.
    pub constitutional_principles: Vec<ConstitutionalPrinciple>,
    /// Whether critique reasoning may use chain-of-thought.
    pub use_chain_of_thought: bool,
    /// Minimum agreement required across critique rounds.
    pub min_agreement_ratio: f64,
}

/// One constitutional principle used during self-critique.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstitutionalPrinciple {
    /// Stable principle identifier.
    pub id: String,
    /// Human-readable principle name.
    pub name: String,
    /// Detailed description of the principle.
    pub description: String,
    /// Severity of violating the principle.
    pub severity: PrincipleSeverity,
    /// Prompt template used to test the principle.
    pub check_prompt: String,
}

/// Severity of a constitutional principle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipleSeverity {
    /// Hard constraint with immediate containment.
    Hard,
    /// Soft constraint requiring review.
    Soft,
    /// Advisory-only guidance.
    Advisory,
}
