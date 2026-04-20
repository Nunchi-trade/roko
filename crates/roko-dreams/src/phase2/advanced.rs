//! Phase 2 advanced dream-concept stubs.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::DreamTrigger;
use crate::phase2::shared::{Hypothesis, ModelTier};

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
    pub fn quarantine(&mut self, report: NightmareReport) {
        self.quarantined_hypotheses
            .push(report.hypothesis_summary.clone());
        self.cooldown_remaining = self
            .cooldown_remaining
            .max(report.detection_stage as usize);
        self.pending_human_reviews.push(report);
    }
}

/// Screening verdict from the 4-stage nightmare detection pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreeningVerdict {
    /// Whether the output passed all stages.
    pub passed: bool,
    /// Maximum severity detected across stages.
    pub max_severity: PrincipleSeverity,
    /// Per-stage results.
    pub stage_results: Vec<StageResult>,
}

/// Result from one screening stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageResult {
    /// Stage number (1-4).
    pub stage: u8,
    /// Stage name.
    pub name: String,
    /// Whether this stage passed.
    pub passed: bool,
    /// Severity if a finding was detected.
    pub severity: PrincipleSeverity,
    /// Human-readable finding description.
    pub finding: Option<String>,
}

/// Harmful behavior patterns checked in stage 1.
const HARM_PATTERNS: &[&str] = &[
    "ignore safety",
    "bypass verification",
    "delete all",
    "disable gate",
    "skip checks",
    "override policy",
    "rm -rf",
    "drop table",
    "remove constraint",
];

/// CBRN keyword patterns checked in stage 2.
const CBRN_PATTERNS: &[&str] = &[
    "chemical weapon",
    "biological agent",
    "radiological",
    "nuclear device",
    "synthesize pathogen",
    "enrichment process",
    "weaponize",
];

/// Prompt injection patterns checked in stage 4.
const INJECTION_PATTERNS: &[&str] = &[
    "ignore previous instructions",
    "system prompt override",
    "you are now",
    "disregard all",
    "new instructions:",
    "forget everything",
    "act as if",
];

impl NightmareDetector {
    /// Run the 4-stage safety pipeline on a dream output.
    ///
    /// Returns a [`ScreeningVerdict`] indicating whether the output may
    /// enter the staging buffer. Any `Hard` severity blocks the output.
    #[must_use]
    pub fn screen(&self, content: &str, hdc_similarity: Option<f64>) -> ScreeningVerdict {
        let mut stages = Vec::with_capacity(4);

        // Stage 1: Harm classifier
        stages.push(self.stage_harm_classifier(content));

        // Stage 2: CBRN check
        stages.push(self.stage_cbrn_check(content));

        // Stage 3: Novelty-divergence check
        stages.push(self.stage_novelty_divergence(hdc_similarity));

        // Stage 4: Gradient attack / prompt injection detection
        stages.push(self.stage_injection_detection(content));

        let max_severity = stages
            .iter()
            .map(|s| s.severity)
            .max_by_key(|s| match s {
                PrincipleSeverity::Advisory => 0,
                PrincipleSeverity::Soft => 1,
                PrincipleSeverity::Hard => 2,
            })
            .unwrap_or(PrincipleSeverity::Advisory);

        let passed = stages.iter().all(|s| s.passed);

        ScreeningVerdict {
            passed,
            max_severity,
            stage_results: stages,
        }
    }

    fn stage_harm_classifier(&self, content: &str) -> StageResult {
        let lower = content.to_lowercase();
        let finding = HARM_PATTERNS
            .iter()
            .find(|p| lower.contains(**p))
            .map(|p| format!("harmful pattern detected: {p}"));

        StageResult {
            stage: 1,
            name: "harm_classifier".to_string(),
            passed: finding.is_none(),
            severity: if finding.is_some() {
                PrincipleSeverity::Hard
            } else {
                PrincipleSeverity::Advisory
            },
            finding,
        }
    }

    fn stage_cbrn_check(&self, content: &str) -> StageResult {
        let lower = content.to_lowercase();
        let finding = CBRN_PATTERNS
            .iter()
            .find(|p| lower.contains(**p))
            .map(|p| format!("CBRN content detected: {p}"));

        StageResult {
            stage: 2,
            name: "cbrn_check".to_string(),
            passed: finding.is_none(),
            severity: if finding.is_some() {
                PrincipleSeverity::Hard
            } else {
                PrincipleSeverity::Advisory
            },
            finding,
        }
    }

    fn stage_novelty_divergence(&self, hdc_similarity: Option<f64>) -> StageResult {
        match hdc_similarity {
            Some(sim) if sim > 0.95 => StageResult {
                stage: 3,
                name: "novelty_divergence".to_string(),
                passed: false,
                severity: PrincipleSeverity::Soft,
                finding: Some(format!(
                    "redundant output (HDC similarity {sim:.3} > 0.95)"
                )),
            },
            Some(sim) if sim < 0.3 => StageResult {
                stage: 3,
                name: "novelty_divergence".to_string(),
                passed: false,
                severity: PrincipleSeverity::Soft,
                finding: Some(format!(
                    "likely hallucination (HDC similarity {sim:.3} < 0.3)"
                )),
            },
            _ => StageResult {
                stage: 3,
                name: "novelty_divergence".to_string(),
                passed: true,
                severity: PrincipleSeverity::Advisory,
                finding: None,
            },
        }
    }

    fn stage_injection_detection(&self, content: &str) -> StageResult {
        let lower = content.to_lowercase();
        let finding = INJECTION_PATTERNS
            .iter()
            .find(|p| lower.contains(**p))
            .map(|p| format!("prompt injection pattern detected: {p}"));

        StageResult {
            stage: 4,
            name: "injection_detection".to_string(),
            passed: finding.is_none(),
            severity: if finding.is_some() {
                PrincipleSeverity::Hard
            } else {
                PrincipleSeverity::Advisory
            },
            finding,
        }
    }
}

/// Persistent dream journal configuration and state (DREAM-14).
///
/// Appends `DreamJournalEntry` records to `.roko/dreams/journal.jsonl`
/// after each dream cycle completes.
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

impl DreamJournal {
    /// Create a journal backed by the given path.
    #[must_use]
    pub fn new(journal_path: PathBuf) -> Self {
        Self {
            journal_path,
            cycle_index: Vec::new(),
            cached_trend: None,
            trend_recompute_interval: 10,
        }
    }

    /// Create a journal at the standard `.roko/dreams/journal.jsonl` path
    /// under the given workspace root.
    #[must_use]
    pub fn standard(workdir: &std::path::Path) -> Self {
        Self::new(workdir.join(".roko").join("dreams").join("journal.jsonl"))
    }

    /// Append a journal entry to disk.
    ///
    /// Creates parent directories if they do not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if directory creation or file I/O fails.
    pub fn append(&mut self, entry: &DreamJournalEntry) -> Result<(), std::io::Error> {
        if let Some(parent) = self.journal_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.journal_path)?;
        let line = serde_json::to_string(entry).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        })?;
        use std::io::Write;
        writeln!(file, "{line}")?;
        self.cycle_index.push(entry.cycle_id.clone());
        Ok(())
    }

    /// Read all journal entries from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn read_all(&self) -> Result<Vec<DreamJournalEntry>, std::io::Error> {
        let content = std::fs::read_to_string(&self.journal_path)?;
        let mut entries = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let entry: DreamJournalEntry = serde_json::from_str(line).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, e)
            })?;
            entries.push(entry);
        }
        Ok(entries)
    }

    /// Read the most recent N entries.
    pub fn read_recent(&self, n: usize) -> Result<Vec<DreamJournalEntry>, std::io::Error> {
        let all = self.read_all()?;
        let start = all.len().saturating_sub(n);
        Ok(all[start..].to_vec())
    }

    /// Number of entries in the index (may be stale if not synced).
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.cycle_index.len()
    }
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
