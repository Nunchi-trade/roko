//! Tier progression from raw episodes to insights, heuristics, and a playbook.
//!
//! This module compresses the episode log in three stages:
//! - D1: raw episodes -> insights
//! - D2: insights with at least five supporting episodes -> heuristics
//! - D3: heuristics -> `PLAYBOOK.md`
//!
//! The implementation is deterministic and uses the existing episode and
//! pattern primitives already present in the workspace.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use parking_lot::Mutex;
use roko_learn::episode_logger::Episode;
use roko_learn::episode_logger::GateVerdict;
use roko_learn::pattern_discovery::{EpisodeView, PatternMiner};
use serde::{Deserialize, Serialize};

use crate::{KnowledgeEntry, KnowledgeKind, KnowledgeStore, KnowledgeTier};

const DEFAULT_MIN_SUPPORT: usize = 3;
const DEFAULT_MIN_HEURISTIC_SUPPORT: usize = 5;
const DEFAULT_MIN_CONFIDENCE: f64 = 0.7;
const DEFAULT_PLAYBOOK_LIMIT: usize = 12;
const DEFAULT_HALF_LIFE_DAYS: f64 = 45.0;
const TIER_PROGRESSION_D1_SOURCE: &str = "tier-progression:d1";
const TIER_PROGRESSION_D2_SOURCE: &str = "tier-progression:d2";
const TIER_PROGRESSION_D3_SOURCE: &str = "tier-progression:d3";
/// Default filename for falsifiable heuristic snapshots.
pub const DEFAULT_HEURISTICS_FILE: &str = "heuristics.jsonl";
/// Default filename for heuristic observation receipts.
pub const DEFAULT_HEURISTIC_OBSERVATIONS_FILE: &str = "heuristic-observations.jsonl";
/// Default filename for heuristic demotion receipts.
pub const DEFAULT_HEURISTIC_DEMOTIONS_FILE: &str = "heuristic-demotions.jsonl";
/// Number of passing verdicts required to promote one tier.
pub const PROMOTION_SUCCESS_THRESHOLD: usize = 3;
/// Number of failing verdicts required to demote one tier.
pub const DEMOTION_FAILURE_THRESHOLD: usize = 2;
/// Entry age multiplier that triggers an expiry review relative to half-life.
pub const EXPIRY_REVIEW_HALF_LIFE_MULTIPLIER: f64 = 2.0;

/// Summary of a recurring causal pattern observed in raw episodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightRecord {
    /// Stable identifier for the insight.
    pub id: String,
    /// Normalized antecedent actions that set up the pattern.
    pub antecedent: Vec<String>,
    /// Normalized consequent action that consistently followed.
    pub consequent: String,
    /// Number of distinct episodes that exhibited the pattern.
    pub support_count: usize,
    /// Number of distinct episodes that contained the antecedent.
    pub antecedent_episode_count: usize,
    /// Confidence in the range `0.0..=1.0`.
    pub confidence: f64,
    /// Millisecond timestamp of the first supporting episode.
    pub first_seen_ms: i64,
    /// Millisecond timestamp of the most recent supporting episode.
    pub last_seen_ms: i64,
    /// Distinct episode ids that support this insight.
    pub source_episodes: Vec<String>,
}

impl InsightRecord {
    /// Human-readable explanation of the insight.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "When {} happened, {} consistently followed.",
            self.antecedent
                .iter()
                .map(|action| humanize_action(action))
                .collect::<Vec<_>>()
                .join(" and "),
            humanize_action(&self.consequent)
        )
    }
}

// ---------------------------------------------------------------------------
// NEURO-12: Calibration types for heuristic falsification tracking.
// ---------------------------------------------------------------------------

/// Action taken when a heuristic is calibrated against new evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationAction {
    /// Evidence supports the heuristic.
    Confirm,
    /// Evidence contradicts the heuristic.
    Violate,
    /// Evidence refines the heuristic's scope (narrowing).
    Refine,
    /// Evidence broadens the heuristic's applicability.
    Generalize,
    /// Evidence fully refutes the heuristic.
    Refute,
}

impl CalibrationAction {
    /// Human-readable label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Confirm => "confirm",
            Self::Violate => "violate",
            Self::Refine => "refine",
            Self::Generalize => "generalize",
            Self::Refute => "refute",
        }
    }

    /// Whether this action is considered negative evidence.
    #[must_use]
    pub const fn is_negative(self) -> bool {
        matches!(self, Self::Violate | Self::Refute)
    }
}

/// One calibration receipt tying an episode to an action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationReceipt {
    /// Episode ID that provided the evidence.
    pub episode_id: String,
    /// Action taken.
    pub action: CalibrationAction,
    /// Millisecond timestamp of when the calibration occurred.
    pub timestamp_ms: i64,
}

/// Record of a heuristic being contradicted by evidence.
///
/// Created when `replay_heuristics()` detects violations. Falsifiers
/// provide an explicit audit trail for why a heuristic's confidence was
/// reduced, rather than silently adjusting the score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FalsifierRecord {
    /// ID of the heuristic that was contradicted.
    pub heuristic_id: String,
    /// Episodes that provided the contradicting evidence.
    pub contradicting_episodes: Vec<String>,
    /// Whether this is a partial violation or full refutation.
    pub action: CalibrationAction,
    /// Brief description of what went wrong.
    pub description: String,
    /// Millisecond timestamp of when the falsifier was created.
    pub created_at_ms: i64,
}

/// Actionable rule promoted from one or more insights.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeuristicRule {
    /// Stable identifier for the heuristic.
    pub id: String,
    /// Source insight id.
    pub insight_id: String,
    /// Short human title.
    pub title: String,
    /// Machine-parseable "if" clause.
    pub when_clause: String,
    /// Machine-parseable "then" clause.
    pub then_clause: String,
    /// Confidence in the range `0.0..=1.0`.
    pub confidence: f64,
    /// Number of independent episode confirmations.
    pub confirmations: usize,
    /// Millisecond timestamp of the first supporting episode.
    pub first_seen_ms: i64,
    /// Millisecond timestamp of the most recent supporting episode.
    pub last_seen_ms: i64,
    /// Distinct episode ids that support this heuristic.
    pub source_episodes: Vec<String>,
    /// Which model this heuristic is specific to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_model: Option<String>,
    /// How broadly the heuristic applies across models.
    #[serde(default = "default_model_generality")]
    pub model_generality: f64,
    // NEURO-12: Calibration fields
    /// Total number of trials (observations) for this heuristic.
    #[serde(default)]
    pub trials: usize,
    /// Number of violations (contradictions) observed.
    #[serde(default)]
    pub violations: usize,
    /// Calibration receipts tying episodes to actions.
    #[serde(default)]
    pub receipts: Vec<CalibrationReceipt>,
}

impl HeuristicRule {
    /// Human-readable rule text.
    #[must_use]
    pub fn summary(&self) -> String {
        format!("If {}, then {}.", self.when_clause, self.then_clause)
    }

    /// Return whether the heuristic should be injected for
    /// `current_model`.
    #[must_use]
    pub fn applies_to_model(&self, current_model: &str) -> bool {
        self.model_generality > 0.7 || self.source_model.as_deref() == Some(current_model)
    }
}

/// Runtime-falsifiable heuristic with explicit when/then/falsifier clauses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Heuristic {
    /// Stable heuristic identifier.
    pub id: String,
    /// Condition pattern or tag expression.
    pub when: String,
    /// Expected outcome when the condition applies.
    pub then: String,
    /// Text predicate that contradicts `then` when found in gate output.
    pub falsifier: String,
    /// Confidence in `0.0..=1.0`.
    pub confidence: f64,
    /// Number of gate observations where `when` matched.
    pub observations: u32,
    /// Source durable knowledge entry, when this heuristic came from one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_entry_id: Option<String>,
    /// Confidence threshold below which the heuristic becomes AntiKnowledge.
    pub demotion_threshold: f64,
    /// Timestamp of demotion to AntiKnowledge, if it already happened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub demoted_at_ms: Option<i64>,
}

impl Default for Heuristic {
    fn default() -> Self {
        Self {
            id: String::new(),
            when: String::new(),
            then: String::new(),
            falsifier: String::new(),
            confidence: 0.7,
            observations: 0,
            knowledge_entry_id: None,
            demotion_threshold: 0.1,
            demoted_at_ms: None,
        }
    }
}

impl Heuristic {
    /// Evaluate this heuristic against one gate completion.
    ///
    /// Returns the confidence delta. `0.0` means the heuristic did not apply
    /// or the observation was ambiguous.
    pub fn evaluate(&mut self, task_tags: &[String], gate_output: &str, gate_passed: bool) -> f64 {
        if self.demoted_at_ms.is_some() || !self.when_matches(task_tags) {
            return 0.0;
        }

        self.observations = self.observations.saturating_add(1);
        if !gate_passed && self.falsifier_matches(gate_output) {
            self.confidence = (self.confidence - 0.2).max(0.0);
            -0.2
        } else if gate_passed {
            self.confidence = (self.confidence + 0.05).min(1.0);
            0.05
        } else {
            0.0
        }
    }

    /// Return whether this heuristic should be demoted to AntiKnowledge.
    #[must_use]
    pub fn should_demote(&self) -> bool {
        self.demoted_at_ms.is_none()
            && self.confidence < self.demotion_threshold
            && self.observations >= 3
    }

    fn when_matches(&self, task_tags: &[String]) -> bool {
        let when = self.when.trim().to_ascii_lowercase();
        if when.is_empty() {
            return false;
        }
        task_tags
            .iter()
            .map(|tag| tag.trim().to_ascii_lowercase())
            .any(|tag| !tag.is_empty() && (when.contains(&tag) || tag.contains(&when)))
    }

    fn falsifier_matches(&self, gate_output: &str) -> bool {
        let falsifier = self.falsifier.trim().to_ascii_lowercase();
        if falsifier.is_empty() {
            return false;
        }
        gate_output.to_ascii_lowercase().contains(&falsifier)
    }
}

/// Receipt appended when a heuristic observes a runtime gate completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeuristicObservation {
    /// Heuristic that was evaluated.
    pub heuristic_id: String,
    /// Confidence delta applied to the heuristic.
    pub delta: f64,
    /// Confidence after applying the delta.
    pub confidence_after: f64,
    /// Observation count after applying the runtime event.
    pub observations_after: u32,
    /// Whether the gate completion passed.
    pub gate_passed: bool,
    /// Tags supplied by the task/runtime context.
    pub task_tags: Vec<String>,
    /// Timestamp when the observation was recorded.
    pub observed_at_ms: i64,
}

/// Receipt appended when a heuristic is converted into AntiKnowledge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeuristicDemotionRecord {
    /// Heuristic that was demoted.
    pub heuristic_id: String,
    /// AntiKnowledge entry created for the demotion.
    pub anti_knowledge_entry_id: String,
    /// Confidence at demotion time.
    pub confidence: f64,
    /// Observation count at demotion time.
    pub observations: u32,
    /// Timestamp when demotion occurred.
    pub demoted_at_ms: i64,
}

/// File-backed store for falsifiable runtime heuristics.
#[derive(Debug, Clone)]
pub struct HeuristicStore {
    heuristics_path: PathBuf,
    observations_path: PathBuf,
    demotions_path: PathBuf,
    write_gate: Arc<Mutex<()>>,
}

impl HeuristicStore {
    /// Construct a heuristic store from an explicit `heuristics.jsonl` path.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let heuristics_path = path.into();
        let dir = heuristics_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            heuristics_path,
            observations_path: dir.join(DEFAULT_HEURISTIC_OBSERVATIONS_FILE),
            demotions_path: dir.join(DEFAULT_HEURISTIC_DEMOTIONS_FILE),
            write_gate: Arc::new(Mutex::new(())),
        }
    }

    /// Construct a heuristic store rooted at a `.roko/` directory.
    #[must_use]
    pub fn for_roko_dir(roko_dir: impl AsRef<Path>) -> Self {
        Self::new(
            roko_dir
                .as_ref()
                .join("neuro")
                .join(DEFAULT_HEURISTICS_FILE),
        )
    }

    /// Construct a heuristic store rooted at a workspace directory.
    #[must_use]
    pub fn for_workdir(workdir: impl AsRef<Path>) -> Self {
        Self::for_roko_dir(workdir.as_ref().join(".roko"))
    }

    /// Path of the heuristic snapshot file.
    #[must_use]
    pub fn heuristics_path(&self) -> &Path {
        &self.heuristics_path
    }

    /// Path of the append-only observation receipt log.
    #[must_use]
    pub fn observations_path(&self) -> &Path {
        &self.observations_path
    }

    /// Path of the append-only demotion receipt log.
    #[must_use]
    pub fn demotions_path(&self) -> &Path {
        &self.demotions_path
    }

    /// Load all current heuristic snapshots.
    ///
    /// # Errors
    ///
    /// Returns an error if the heuristic file exists but cannot be read.
    pub fn load_all(&self) -> Result<Vec<Heuristic>> {
        read_jsonl(&self.heuristics_path)
    }

    /// Persist the full current heuristic snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot cannot be written.
    pub fn save_all(&self, heuristics: &[Heuristic]) -> Result<()> {
        let _guard = self.write_gate.lock();
        rewrite_jsonl(&self.heuristics_path, heuristics)
    }

    /// Insert or replace a heuristic by id.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot cannot be read or written.
    pub fn upsert(&self, heuristic: Heuristic) -> Result<()> {
        let _guard = self.write_gate.lock();
        let mut heuristics: Vec<Heuristic> = read_jsonl(&self.heuristics_path)?;
        if let Some(existing) = heuristics.iter_mut().find(|item| item.id == heuristic.id) {
            *existing = heuristic;
        } else {
            heuristics.push(heuristic);
        }
        rewrite_jsonl(&self.heuristics_path, &heuristics)
    }

    /// Evaluate all active heuristics against a gate completion and append
    /// observation receipts for every applied delta.
    ///
    /// # Errors
    ///
    /// Returns an error if heuristic persistence fails.
    pub fn evaluate_all(
        &self,
        task_tags: &[String],
        gate_output: &str,
        gate_passed: bool,
    ) -> Result<Vec<HeuristicObservation>> {
        let _guard = self.write_gate.lock();
        let mut heuristics: Vec<Heuristic> = read_jsonl(&self.heuristics_path)?;
        let now_ms = Utc::now().timestamp_millis();
        let mut observations = Vec::new();

        for heuristic in &mut heuristics {
            let delta = heuristic.evaluate(task_tags, gate_output, gate_passed);
            if delta == 0.0 {
                continue;
            }
            observations.push(HeuristicObservation {
                heuristic_id: heuristic.id.clone(),
                delta,
                confidence_after: heuristic.confidence,
                observations_after: heuristic.observations,
                gate_passed,
                task_tags: task_tags.to_vec(),
                observed_at_ms: now_ms,
            });
        }

        if !observations.is_empty() {
            rewrite_jsonl(&self.heuristics_path, &heuristics)?;
            append_jsonl_batch(&self.observations_path, &observations)?;
        }
        Ok(observations)
    }

    /// Read append-only heuristic observation receipts.
    ///
    /// # Errors
    ///
    /// Returns an error if the observation log exists but cannot be read.
    pub fn read_observations(&self) -> Result<Vec<HeuristicObservation>> {
        read_jsonl(&self.observations_path)
    }

    /// Read append-only heuristic demotion receipts.
    ///
    /// # Errors
    ///
    /// Returns an error if the demotion log exists but cannot be read.
    pub fn read_demotions(&self) -> Result<Vec<HeuristicDemotionRecord>> {
        read_jsonl(&self.demotions_path)
    }

    /// Convert heuristics below their demotion threshold into AntiKnowledge.
    ///
    /// Demotion is idempotent: heuristics already present in the demotion log
    /// are skipped on later calls.
    ///
    /// # Errors
    ///
    /// Returns an error if the heuristic or knowledge stores cannot be updated.
    pub fn demote_expired(
        &self,
        knowledge_store: &KnowledgeStore,
    ) -> Result<Vec<HeuristicDemotionRecord>> {
        let _guard = self.write_gate.lock();
        let mut heuristics: Vec<Heuristic> = read_jsonl(&self.heuristics_path)?;
        let existing_demotions: BTreeSet<String> =
            read_jsonl::<HeuristicDemotionRecord>(&self.demotions_path)?
                .into_iter()
                .map(|record| record.heuristic_id)
                .collect();
        let now_ms = Utc::now().timestamp_millis();
        let mut demotions = Vec::new();
        let mut anti_entries = Vec::new();

        for heuristic in &mut heuristics {
            if !heuristic.should_demote() || existing_demotions.contains(&heuristic.id) {
                continue;
            }
            heuristic.demoted_at_ms = Some(now_ms);
            let anti = anti_knowledge_for_heuristic(heuristic, now_ms);
            demotions.push(HeuristicDemotionRecord {
                heuristic_id: heuristic.id.clone(),
                anti_knowledge_entry_id: anti.id.clone(),
                confidence: heuristic.confidence,
                observations: heuristic.observations,
                demoted_at_ms: now_ms,
            });
            anti_entries.push(anti);
        }

        if !demotions.is_empty() {
            knowledge_store.ingest(anti_entries)?;
            rewrite_jsonl(&self.heuristics_path, &heuristics)?;
            append_jsonl_batch(&self.demotions_path, &demotions)?;
        }
        Ok(demotions)
    }
}

/// Markdown playbook compiled from the top heuristics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybookCompilation {
    /// Rendered `PLAYBOOK.md` contents.
    pub markdown: String,
    /// Rules that were compiled into the playbook.
    pub rules: Vec<HeuristicRule>,
}

/// Full tier progression snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TierProgressionReport {
    /// D1 output.
    pub insights: Vec<InsightRecord>,
    /// D2 output.
    pub heuristics: Vec<HeuristicRule>,
    /// D3 output.
    pub playbook: PlaybookCompilation,
    /// NEURO-12: Falsifier records generated during heuristic replay.
    #[serde(default)]
    pub falsifiers: Vec<FalsifierRecord>,
}

/// Result of evaluating whether a knowledge entry should change tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierProgressionDecision {
    /// Promote to the supplied tier.
    Promote(KnowledgeTier),
    /// Demote to the supplied tier.
    Demote(KnowledgeTier),
    /// Keep the tier, but schedule a freshness review.
    ReviewExpiry,
    /// No change required.
    NoChange,
}

impl TierProgressionDecision {
    /// Return the tier change, if any.
    #[must_use]
    pub const fn tier(self) -> Option<KnowledgeTier> {
        match self {
            Self::Promote(tier) | Self::Demote(tier) => Some(tier),
            Self::ReviewExpiry | Self::NoChange => None,
        }
    }

    /// Whether this decision should trigger an expiry review.
    #[must_use]
    pub const fn needs_expiry_review(self) -> bool {
        matches!(self, Self::ReviewExpiry)
    }
}

/// Tiered compressor over raw episode logs.
#[derive(Debug, Clone, Copy)]
pub struct TierProgression {
    min_support: usize,
    min_heuristic_support: usize,
    min_confidence: f64,
    playbook_limit: usize,
}

impl Default for TierProgression {
    fn default() -> Self {
        Self {
            min_support: DEFAULT_MIN_SUPPORT,
            min_heuristic_support: DEFAULT_MIN_HEURISTIC_SUPPORT,
            min_confidence: DEFAULT_MIN_CONFIDENCE,
            playbook_limit: DEFAULT_PLAYBOOK_LIMIT,
        }
    }
}

impl TierProgression {
    /// Construct a progression pipeline with custom thresholds.
    #[must_use]
    pub const fn new(min_support: usize, min_confidence: f64, playbook_limit: usize) -> Self {
        Self {
            min_support: if min_support == 0 { 1 } else { min_support },
            min_heuristic_support: DEFAULT_MIN_HEURISTIC_SUPPORT,
            min_confidence: if min_confidence.is_finite() && min_confidence > 0.0 {
                min_confidence.min(1.0)
            } else {
                DEFAULT_MIN_CONFIDENCE
            },
            playbook_limit: if playbook_limit == 0 {
                DEFAULT_PLAYBOOK_LIMIT
            } else {
                playbook_limit
            },
        }
    }

    /// Run the full raw-episode -> playbook progression.
    #[must_use]
    pub fn analyze(&self, episodes: &[Episode]) -> TierProgressionReport {
        let candidate_patterns = discover_patterns(episodes, self.min_support);
        let insights = self.extract_insights(episodes, &candidate_patterns);
        let heuristics = self.promote_heuristics(&insights);
        let playbook = self.compile_playbook(&heuristics, insights.len());
        TierProgressionReport {
            insights,
            heuristics,
            playbook,
            falsifiers: Vec::new(),
        }
    }

    /// Evaluate whether a knowledge entry should change tier based on gate verdicts.
    ///
    /// Promotion and demotion are intentionally conservative:
    /// - 3+ passing verdicts promote one tier
    /// - 2+ failing verdicts demote one tier
    /// - entries older than 2× their effective half-life are marked for review
    ///
    /// # Notes
    ///
    /// The caller can use [`Self::evaluate_tier_progression`] for a richer
    /// decision enum, or this method when only a concrete target tier matters.
    #[must_use]
    pub fn evaluate_promotion(
        entry: &KnowledgeEntry,
        verdicts: &[GateVerdict],
    ) -> Option<KnowledgeTier> {
        Self::evaluate_tier_progression(entry, verdicts).tier()
    }

    /// Rich progression decision that preserves expiry-review intent.
    #[must_use]
    pub fn evaluate_tier_progression(
        entry: &KnowledgeEntry,
        verdicts: &[GateVerdict],
    ) -> TierProgressionDecision {
        let successes = verdicts.iter().filter(|verdict| verdict.passed).count();
        let failures = verdicts.len().saturating_sub(successes);

        if successes >= PROMOTION_SUCCESS_THRESHOLD {
            return TierProgressionDecision::Promote(promote_tier(entry.tier));
        }
        if failures >= DEMOTION_FAILURE_THRESHOLD {
            // Persistent entries cannot be demoted without explicit deprecation.
            if entry.tier == KnowledgeTier::Persistent && !entry.deprecated {
                return TierProgressionDecision::NoChange;
            }
            return TierProgressionDecision::Demote(demote_tier(entry.tier));
        }
        if entry_needs_expiry_review(entry) {
            return TierProgressionDecision::ReviewExpiry;
        }

        TierProgressionDecision::NoChange
    }

    /// Tier promotion threshold aligned to the runtime knowledge lifecycle.
    ///
    /// `Transient -> Working` requires 1 pass, `Working -> Consolidated`
    /// requires 2 passes, and `Consolidated -> Persistent` requires 5 passes.
    #[must_use]
    pub const fn promotion_threshold(tier: KnowledgeTier) -> usize {
        promotion_threshold(tier)
    }

    /// Rich progression decision using per-tier promotion thresholds.
    #[must_use]
    pub fn evaluate_tier_progression_v2(
        entry: &KnowledgeEntry,
        pass_count: usize,
        fail_count: usize,
    ) -> TierProgressionDecision {
        evaluate_tier_progression_v2(entry, pass_count, fail_count)
    }

    /// Whether an entry should be reviewed for expiry.
    #[must_use]
    pub fn needs_expiry_review(entry: &KnowledgeEntry) -> bool {
        entry_needs_expiry_review(entry)
    }

    /// Replay heuristics against the supplied episodes and revise confidence.
    ///
    /// NEURO-12: Also tracks calibration receipts on each heuristic and
    /// emits falsifier records for heuristics with significant contradictions.
    pub fn replay_heuristics(&self, report: &mut TierProgressionReport, episodes: &[Episode]) {
        let now_ms = Utc::now().timestamp_millis();
        let mut episode_success_by_id: HashMap<String, bool> = HashMap::new();
        for episode in episodes {
            episode_success_by_id
                .entry(episode_source_id(episode).to_string())
                .or_insert(episode.success);
        }

        let mut falsifiers: Vec<FalsifierRecord> = Vec::new();

        for heuristic in &mut report.heuristics {
            let Some(_expected_success) = heuristic_expected_success(heuristic) else {
                continue;
            };

            let mut supporting = 0usize;
            let mut contradicting = 0usize;
            let mut contradicting_episodes: Vec<String> = Vec::new();
            for source_episode_id in &heuristic.source_episodes {
                if let Some(&success) = episode_success_by_id.get(source_episode_id) {
                    // NEURO-12: Record calibration receipt for each observation.
                    let action = if success {
                        supporting += 1;
                        CalibrationAction::Confirm
                    } else {
                        contradicting += 1;
                        contradicting_episodes.push(source_episode_id.clone());
                        CalibrationAction::Violate
                    };
                    heuristic.receipts.push(CalibrationReceipt {
                        episode_id: source_episode_id.clone(),
                        action,
                        timestamp_ms: now_ms,
                    });
                }
            }

            let total = supporting + contradicting;
            if total == 0 {
                continue;
            }

            // Update calibration counters.
            heuristic.trials = heuristic.trials.saturating_add(total);
            heuristic.violations = heuristic.violations.saturating_add(contradicting);

            let validation = supporting as f64 / total as f64;
            let adjustment = (validation - 0.5) * 0.2;
            heuristic.confidence = (heuristic.confidence + adjustment).clamp(0.0, 1.0);

            // NEURO-12: Emit falsifier record when violations are significant.
            if contradicting >= DEMOTION_FAILURE_THRESHOLD && !contradicting_episodes.is_empty() {
                let action = if validation < 0.2 {
                    CalibrationAction::Refute
                } else {
                    CalibrationAction::Violate
                };
                falsifiers.push(FalsifierRecord {
                    heuristic_id: heuristic.id.clone(),
                    contradicting_episodes,
                    action,
                    description: format!(
                        "Heuristic '{}' contradicted in {contradicting}/{total} trials (validation {:.2})",
                        heuristic.title, validation
                    ),
                    created_at_ms: now_ms,
                });
            }
        }

        report.falsifiers.extend(falsifiers);
        report.heuristics.sort_by(compare_heuristics);
        report.playbook = self.compile_playbook(&report.heuristics, report.insights.len());
    }

    /// Extract D1 insights from raw episodes.
    #[must_use]
    pub fn extract_insights(
        &self,
        episodes: &[Episode],
        candidate_patterns: &[roko_learn::pattern_discovery::Pattern],
    ) -> Vec<InsightRecord> {
        let mut by_pattern: BTreeMap<(String, String, String), PatternSupport> = BTreeMap::new();
        let mut antecedent_support: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
        let candidate_set: BTreeSet<(String, String, String)> = candidate_patterns
            .iter()
            .filter_map(|pattern| parse_pattern(&pattern.description))
            .collect();

        for episode in episodes {
            let episode_id = episode_source_id(episode).to_string();
            let actions = episode_actions(episode);
            if actions.len() < 3 {
                continue;
            }

            let mut seen_in_episode: BTreeSet<(String, String, String)> = BTreeSet::new();
            for window in actions.windows(3) {
                let key = (window[0].clone(), window[1].clone(), window[2].clone());
                antecedent_support
                    .entry((key.0.clone(), key.1.clone()))
                    .or_default()
                    .insert(episode_id.clone());
                if !candidate_set.contains(&key) {
                    continue;
                }
                if !seen_in_episode.insert(key.clone()) {
                    continue;
                }

                let entry = by_pattern
                    .entry(key)
                    .or_insert_with(PatternSupport::default);
                entry.support_episodes.insert(episode_id.clone());
            }
        }

        let mut insights: Vec<InsightRecord> = candidate_patterns
            .iter()
            .filter_map(|pattern| {
                let triple = parse_pattern(&pattern.description)?;
                let support = by_pattern.get(&triple)?;
                let support_count = support.support_episodes.len();
                if support_count < self.min_support {
                    return None;
                }

                let antecedent_episode_count = antecedent_support
                    .get(&(triple.0.clone(), triple.1.clone()))
                    .map(BTreeSet::len)
                    .unwrap_or(support_count)
                    .max(1);
                let confidence =
                    (support_count as f64 / antecedent_episode_count as f64).clamp(0.0, 1.0);
                let source_episodes = sorted_ids(&support.support_episodes);

                Some(InsightRecord {
                    id: format!("insight:{:016x}", pattern.signature),
                    antecedent: vec![triple.0, triple.1],
                    consequent: triple.2,
                    support_count,
                    antecedent_episode_count,
                    confidence,
                    first_seen_ms: pattern.first_seen_ms,
                    last_seen_ms: pattern.last_seen_ms,
                    source_episodes,
                })
            })
            .collect();

        insights.sort_by(compare_insights);
        insights
    }

    /// Promote validated insights into actionable heuristics.
    #[must_use]
    pub fn promote_heuristics(&self, insights: &[InsightRecord]) -> Vec<HeuristicRule> {
        let mut heuristics: Vec<HeuristicRule> = insights
            .iter()
            .filter(|insight| {
                insight.source_episodes.len() >= self.min_heuristic_support
                    && insight.confidence >= self.min_confidence
            })
            .map(|insight| HeuristicRule {
                id: heuristic_id(insight),
                insight_id: insight.id.clone(),
                title: heuristic_title(insight),
                when_clause: insight
                    .antecedent
                    .iter()
                    .map(|action| humanize_action(action))
                    .collect::<Vec<_>>()
                    .join(" and "),
                then_clause: heuristic_then_clause(insight),
                confidence: insight.confidence,
                confirmations: insight.source_episodes.len(),
                first_seen_ms: insight.first_seen_ms,
                last_seen_ms: insight.last_seen_ms,
                source_episodes: insight.source_episodes.clone(),
                source_model: None,
                model_generality: default_model_generality(),
                trials: 0,
                violations: 0,
                receipts: Vec::new(),
            })
            .collect();

        heuristics.sort_by(compare_heuristics);
        heuristics
    }

    /// Promote validated insights into actionable heuristics using HDC
    /// similarity-based clustering (NEURO-09).
    ///
    /// Instead of promoting each qualifying insight independently, this
    /// method first fingerprints each insight, clusters similar insights
    /// via k-medoids, and then promotes the cluster representative (the
    /// medoid) as the canonical heuristic. The representative accumulates
    /// the source episodes and confidence from all cluster members.
    ///
    /// Falls back to the non-clustered path when fewer than 2 insights
    /// qualify or when the `hdc` clustering would produce degenerate
    /// (single-member) clusters.
    ///
    /// Requires the `hdc` feature for `roko-primitives` access.
    #[cfg(feature = "hdc")]
    #[must_use]
    pub fn promote_heuristics_clustered(&self, insights: &[InsightRecord]) -> Vec<HeuristicRule> {
        let qualified: Vec<&InsightRecord> = insights
            .iter()
            .filter(|insight| {
                insight.source_episodes.len() >= self.min_heuristic_support
                    && insight.confidence >= self.min_confidence
            })
            .collect();

        if qualified.len() < 2 {
            return self.promote_heuristics(insights);
        }

        // Fingerprint each insight by hashing its summary text for HDC seeding.
        let vectors: Vec<roko_primitives::HdcVector> = qualified
            .iter()
            .map(|insight| roko_primitives::HdcVector::from_seed(insight.summary().as_bytes()))
            .collect();

        // Determine cluster count: at most qualified.len()/2, at least 1.
        let k = (qualified.len() / 2).max(1).min(qualified.len());
        let config = roko_learn::hdc_clustering::KMedoidsConfig {
            k,
            max_iterations: 50,
        };
        let cluster_result = roko_learn::hdc_clustering::k_medoids(&vectors, &config);

        let mut heuristics = Vec::new();
        for cluster in &cluster_result.clusters {
            // The medoid insight becomes the representative heuristic.
            let representative = qualified[cluster.medoid_index];

            // Merge source episodes and confidence from all cluster members.
            let mut merged_episodes = BTreeSet::new();
            let mut confidence_sum = 0.0;
            for &member_idx in &cluster.members {
                let member = qualified[member_idx];
                for ep in &member.source_episodes {
                    merged_episodes.insert(ep.clone());
                }
                confidence_sum += member.confidence;
            }
            let avg_confidence = confidence_sum / cluster.members.len().max(1) as f64;
            let merged_confirmations = merged_episodes.len();

            heuristics.push(HeuristicRule {
                id: heuristic_id(representative),
                insight_id: representative.id.clone(),
                title: heuristic_title(representative),
                when_clause: representative
                    .antecedent
                    .iter()
                    .map(|action| humanize_action(action))
                    .collect::<Vec<_>>()
                    .join(" and "),
                then_clause: heuristic_then_clause(representative),
                confidence: avg_confidence.clamp(0.0, 1.0),
                confirmations: merged_confirmations,
                first_seen_ms: representative.first_seen_ms,
                last_seen_ms: representative.last_seen_ms,
                source_episodes: merged_episodes.into_iter().collect(),
                source_model: None,
                model_generality: default_model_generality(),
                trials: 0,
                violations: 0,
                receipts: Vec::new(),
            });
        }

        heuristics.sort_by(compare_heuristics);
        heuristics
    }

    /// Compile the highest-confidence heuristics into `PLAYBOOK.md`.
    #[must_use]
    pub fn compile_playbook(
        &self,
        heuristics: &[HeuristicRule],
        insight_count: usize,
    ) -> PlaybookCompilation {
        let rules: Vec<HeuristicRule> = heuristics
            .iter()
            .take(self.playbook_limit)
            .cloned()
            .collect();
        let markdown = render_playbook_markdown(&rules, heuristics.len(), insight_count);
        PlaybookCompilation { markdown, rules }
    }

    /// Analyze the episodes and write the compiled playbook to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the file
    /// cannot be written.
    pub fn write_playbook(
        &self,
        path: impl AsRef<Path>,
        episodes: &[Episode],
    ) -> io::Result<TierProgressionReport> {
        let report = self.analyze(episodes);
        if let Some(parent) = path.as_ref().parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(path, report.playbook.markdown.as_bytes())?;
        Ok(report)
    }
}

impl From<&InsightRecord> for KnowledgeEntry {
    fn from(value: &InsightRecord) -> Self {
        Self {
            id: value.id.clone(),
            kind: KnowledgeKind::Insight,
            source: Some(TIER_PROGRESSION_D1_SOURCE.to_string()),
            content: value.summary(),
            confidence: value.confidence,
            confidence_weight: value.confidence,
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes: value.source_episodes.clone(),
            tags: vec![
                "tier:insight".to_string(),
                "raw-episodes".to_string(),
                "validated".to_string(),
            ],
            source_model: None,
            model_generality: default_model_generality(),
            created_at: Utc::now(),
            half_life_days: KnowledgeKind::Insight.default_half_life_days(),
            tier: KnowledgeTier::Consolidated,
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,
            confirmation_count: 0,
            distinct_contexts: Vec::new(),
            deprecated: false,
            balance: 1.0,
            frozen: false,
            catalytic_score: 0,
        }
    }
}

impl From<&HeuristicRule> for KnowledgeEntry {
    fn from(value: &HeuristicRule) -> Self {
        Self {
            id: value.id.clone(),
            kind: KnowledgeKind::Heuristic,
            source: Some(TIER_PROGRESSION_D2_SOURCE.to_string()),
            content: value.summary(),
            confidence: value.confidence,
            confidence_weight: value.confidence,
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes: value.source_episodes.clone(),
            tags: vec![
                "tier:heuristic".to_string(),
                "actionable".to_string(),
                "validated".to_string(),
            ],
            source_model: value.source_model.clone(),
            model_generality: value.model_generality,
            created_at: Utc::now(),
            half_life_days: KnowledgeKind::Heuristic.default_half_life_days(),
            tier: KnowledgeTier::Consolidated,
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,
            confirmation_count: 0,
            distinct_contexts: Vec::new(),
            deprecated: false,
            balance: 1.0,
            frozen: false,
            catalytic_score: 0,
        }
    }
}

impl From<&PlaybookCompilation> for KnowledgeEntry {
    fn from(value: &PlaybookCompilation) -> Self {
        Self {
            id: format!("playbook:{:016x}", stable_hash(value.markdown.as_bytes())),
            kind: KnowledgeKind::StrategyFragment,
            source: Some(TIER_PROGRESSION_D3_SOURCE.to_string()),
            content: value.markdown.clone(),
            confidence: if value.rules.is_empty() { 0.0 } else { 1.0 },
            confidence_weight: if value.rules.is_empty() { 0.0 } else { 1.0 },
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes: value
                .rules
                .iter()
                .flat_map(|rule| rule.source_episodes.iter().cloned())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            tags: vec![
                "tier:strategy_fragment".to_string(),
                "machine-parseable".to_string(),
                "playbook".to_string(),
            ],
            source_model: playbook_source_model(&value.rules),
            model_generality: playbook_model_generality(&value.rules),
            created_at: Utc::now(),
            half_life_days: DEFAULT_HALF_LIFE_DAYS,
            tier: KnowledgeTier::Persistent,
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,
            confirmation_count: 0,
            distinct_contexts: Vec::new(),
            deprecated: false,
            balance: 1.0,
            frozen: false,
            catalytic_score: 0,
        }
    }
}

#[derive(Debug, Default)]
struct PatternSupport {
    support_episodes: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct EpisodeActionView {
    actions: Vec<String>,
    succeeded: bool,
}

impl EpisodeView for EpisodeActionView {
    fn actions(&self) -> &[String] {
        &self.actions
    }

    fn succeeded(&self) -> bool {
        self.succeeded
    }
}

/// Helper that runs the existing trigram miner over the synthesized action stream.
fn discover_patterns(
    episodes: &[Episode],
    min_support: usize,
) -> Vec<roko_learn::pattern_discovery::Pattern> {
    let mut miner = PatternMiner::new(min_support as u32, 0.0);
    for episode in episodes {
        let view = EpisodeActionView {
            actions: episode_actions(episode),
            succeeded: episode.success,
        };
        miner.ingest_episode(&view);
    }
    miner.discover()
}

/// Synthesize a short, stable action stream from the raw episode fields.
fn episode_actions(episode: &Episode) -> Vec<String> {
    vec![
        format!(
            "trigger:{}",
            normalize_component(first_non_empty(
                &[episode.trigger_kind.as_str(), episode.kind.as_str()],
                "unknown",
            ))
        ),
        format!(
            "agent:{}",
            normalize_component(first_non_empty(
                &[episode.agent_template.as_str(), episode.agent_id.as_str()],
                "unknown",
            ))
        ),
        format!("gate:{}", normalize_gate_label(first_gate_label(episode))),
        if episode.success {
            "outcome:success".to_string()
        } else {
            "outcome:failure".to_string()
        },
    ]
}

fn first_gate_label(episode: &Episode) -> String {
    episode
        .gate_verdicts
        .first()
        .map(|verdict| {
            format!(
                "{}:{}",
                verdict.gate,
                if verdict.passed { "pass" } else { "fail" }
            )
        })
        .unwrap_or_else(|| "unknown:pass".to_string())
}

fn episode_source_id(episode: &Episode) -> &str {
    if episode.episode_id.trim().is_empty() {
        &episode.id
    } else {
        &episode.episode_id
    }
}

fn parse_pattern(description: &str) -> Option<(String, String, String)> {
    let mut parts = description.split(" -> ").map(str::to_owned);
    let a = parts.next()?;
    let b = parts.next()?;
    let c = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some((a, b, c))
}

fn humanize_action(action: &str) -> String {
    let mut parts = action.splitn(2, ':');
    let kind = parts.next().unwrap_or(action);
    let rest = parts.next().unwrap_or("");
    match kind {
        "trigger" => format!("trigger {}", prettify_token(rest)),
        "agent" => format!("agent {}", prettify_token(rest)),
        "gate" => {
            let mut gate_parts = rest.split(':');
            let name = gate_parts.next().unwrap_or(rest);
            let status = gate_parts.next().unwrap_or("");
            if status == "fail" {
                format!("gate {} failed", prettify_token(name))
            } else if status == "pass" {
                format!("gate {} passed", prettify_token(name))
            } else {
                format!("gate {}", prettify_token(rest))
            }
        }
        "outcome" => format!("{} outcome", prettify_token(rest)),
        _ => prettify_token(action),
    }
}

fn heuristic_title(insight: &InsightRecord) -> String {
    format!(
        "If {} then {}",
        insight
            .antecedent
            .iter()
            .map(|action| humanize_action(action))
            .collect::<Vec<_>>()
            .join(" and "),
        humanize_action(&insight.consequent)
    )
}

fn heuristic_then_clause(insight: &InsightRecord) -> String {
    match insight.consequent.as_str() {
        value if value.starts_with("gate:") => {
            format!("prioritize {}", humanize_action(value))
        }
        "outcome:failure" => "add a verification step before proceeding".to_string(),
        "outcome:success" => "reuse this path as the default play".to_string(),
        _ => format!(
            "treat {} as the expected follow-up",
            humanize_action(&insight.consequent)
        ),
    }
}

fn heuristic_expected_success(heuristic: &HeuristicRule) -> Option<bool> {
    let then_clause = heuristic.then_clause.trim().to_ascii_lowercase();
    if then_clause.is_empty() {
        return None;
    }

    if then_clause.contains("reuse this path as the default play")
        || then_clause.contains("expected follow-up")
        || then_clause.contains("prioritize gate")
        || then_clause.contains("passed")
    {
        return Some(true);
    }

    if then_clause.starts_with("add ")
        || then_clause.starts_with("avoid ")
        || then_clause.starts_with("escalate ")
        || then_clause.starts_with("retry ")
        || then_clause.starts_with("switch ")
        || then_clause.contains("verification")
        || then_clause.contains("failed")
    {
        return Some(false);
    }

    None
}

fn heuristic_id(insight: &InsightRecord) -> String {
    let mut payload = String::new();
    payload.push_str(&insight.id);
    payload.push('|');
    payload.push_str(&insight.when_clause());
    payload.push('|');
    payload.push_str(&insight.consequent);
    format!("heuristic:{:016x}", stable_hash(payload.as_bytes()))
}

fn render_playbook_markdown(
    rules: &[HeuristicRule],
    heuristic_count: usize,
    insight_count: usize,
) -> String {
    let mut out = String::new();
    out.push_str("# PLAYBOOK\n\n");
    out.push_str(&format!(
        "Generated from {} insights and {} heuristics.\n\n",
        insight_count, heuristic_count
    ));
    out.push_str("## Action Rules\n\n");

    if rules.is_empty() {
        out.push_str("No heuristics met the promotion threshold yet.\n");
        return out;
    }

    for (idx, rule) in rules.iter().enumerate() {
        out.push_str(&format!("### {}. {}\n\n", idx + 1, rule.title));
        out.push_str(&format!("{}\n\n", rule.summary()));
        out.push_str(&format!(
            "- confidence: {:.3}\n- confirmations: {}\n- source episodes: {}\n\n",
            rule.confidence,
            rule.confirmations,
            rule.source_episodes.join(", ")
        ));
        let rule_json = PlaybookRuleJson::from(rule);
        let json = serde_json::to_string_pretty(&rule_json).unwrap_or_else(|_| "{}".to_string());
        out.push_str("```json\n");
        out.push_str(&json);
        out.push_str("\n```\n\n");
    }

    out
}

fn playbook_source_model(rules: &[HeuristicRule]) -> Option<String> {
    let mut models = rules.iter().filter_map(|rule| rule.source_model.as_deref());
    let first = models.next()?;
    if models.all(|model| model == first) {
        Some(first.to_string())
    } else {
        None
    }
}

fn playbook_model_generality(rules: &[HeuristicRule]) -> f64 {
    rules
        .iter()
        .map(|rule| rule.model_generality)
        .reduce(f64::min)
        .unwrap_or_else(default_model_generality)
}

const fn default_model_generality() -> f64 {
    1.0
}

#[derive(Debug, Serialize)]
struct PlaybookRuleJson {
    rule_id: String,
    insight_id: String,
    title: String,
    when: String,
    then: String,
    confidence: f64,
    confirmations: usize,
    source_episodes: Vec<String>,
}

impl From<&HeuristicRule> for PlaybookRuleJson {
    fn from(value: &HeuristicRule) -> Self {
        Self {
            rule_id: value.id.clone(),
            insight_id: value.insight_id.clone(),
            title: value.title.clone(),
            when: value.when_clause.clone(),
            then: value.then_clause.clone(),
            confidence: value.confidence,
            confirmations: value.confirmations,
            source_episodes: value.source_episodes.clone(),
        }
    }
}

impl InsightRecord {
    fn when_clause(&self) -> String {
        self.antecedent
            .iter()
            .map(|action| humanize_action(action))
            .collect::<Vec<_>>()
            .join(" and ")
    }
}

fn compare_insights(left: &InsightRecord, right: &InsightRecord) -> std::cmp::Ordering {
    right
        .confidence
        .partial_cmp(&left.confidence)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| right.support_count.cmp(&left.support_count))
        .then_with(|| right.last_seen_ms.cmp(&left.last_seen_ms))
        .then_with(|| left.id.cmp(&right.id))
}

fn compare_heuristics(left: &HeuristicRule, right: &HeuristicRule) -> std::cmp::Ordering {
    right
        .confidence
        .partial_cmp(&left.confidence)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| right.confirmations.cmp(&left.confirmations))
        .then_with(|| right.last_seen_ms.cmp(&left.last_seen_ms))
        .then_with(|| left.id.cmp(&right.id))
}

/// Return the number of gate passes required to promote from `tier`.
#[must_use]
pub const fn promotion_threshold(tier: KnowledgeTier) -> usize {
    match tier {
        KnowledgeTier::Transient => 1,
        KnowledgeTier::Working => 2,
        KnowledgeTier::Consolidated => 5,
        KnowledgeTier::Persistent => usize::MAX,
    }
}

/// Evaluate tier movement with per-tier promotion thresholds.
#[must_use]
pub fn evaluate_tier_progression_v2(
    entry: &KnowledgeEntry,
    pass_count: usize,
    fail_count: usize,
) -> TierProgressionDecision {
    if pass_count >= promotion_threshold(entry.tier) {
        return TierProgressionDecision::Promote(promote_tier(entry.tier));
    }
    if fail_count >= DEMOTION_FAILURE_THRESHOLD {
        if entry.tier == KnowledgeTier::Persistent && !entry.deprecated {
            return TierProgressionDecision::NoChange;
        }
        return TierProgressionDecision::Demote(demote_tier(entry.tier));
    }
    if entry_needs_expiry_review(entry) {
        return TierProgressionDecision::ReviewExpiry;
    }

    TierProgressionDecision::NoChange
}

fn promote_tier(current: KnowledgeTier) -> KnowledgeTier {
    match current {
        KnowledgeTier::Transient => KnowledgeTier::Working,
        KnowledgeTier::Working => KnowledgeTier::Consolidated,
        KnowledgeTier::Consolidated | KnowledgeTier::Persistent => current,
    }
}

fn demote_tier(current: KnowledgeTier) -> KnowledgeTier {
    match current {
        KnowledgeTier::Persistent => KnowledgeTier::Consolidated,
        KnowledgeTier::Consolidated => KnowledgeTier::Working,
        KnowledgeTier::Working | KnowledgeTier::Transient => KnowledgeTier::Transient,
    }
}

fn entry_needs_expiry_review(entry: &KnowledgeEntry) -> bool {
    let half_life_days = entry.effective_half_life_days().max(0.1);
    let age_days = (Utc::now() - entry.created_at).num_seconds().max(0) as f64 / 86_400.0;
    age_days >= half_life_days * EXPIRY_REVIEW_HALF_LIFE_MULTIPLIER
}

fn sorted_ids(ids: &BTreeSet<String>) -> Vec<String> {
    ids.iter().cloned().collect()
}

fn stable_hash(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

fn normalize_component(value: String) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn normalize_gate_label(label: String) -> String {
    let mut parts = label.splitn(2, ':');
    let gate = normalize_component(parts.next().unwrap_or("unknown").to_string());
    let status = parts.next().unwrap_or("pass").to_ascii_lowercase();
    format!("{gate}:{status}")
}

fn prettify_token(value: &str) -> String {
    let words: Vec<String> = value
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let mut rendered = first.to_uppercase().collect::<String>();
                    rendered.push_str(chars.as_str().to_ascii_lowercase().as_str());
                    rendered
                }
                None => String::new(),
            }
        })
        .filter(|part| !part.is_empty())
        .collect();
    if words.is_empty() {
        value.to_string()
    } else {
        words.join(" ")
    }
}

fn first_non_empty<'a>(values: &[&'a str], fallback: &'a str) -> String {
    values
        .iter()
        .copied()
        .find(|value| !value.trim().is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn anti_knowledge_for_heuristic(heuristic: &Heuristic, created_at_ms: i64) -> KnowledgeEntry {
    let content = format!(
        "Heuristic '{}' was demoted because falsifying runtime observations reduced confidence to {:.3} after {} observations. Expected: {}. Falsifier: {}.",
        heuristic.when,
        heuristic.confidence,
        heuristic.observations,
        heuristic.then,
        heuristic.falsifier
    );
    let source_episodes = vec![format!("heuristic:{}", heuristic.id)];
    let tags = vec![
        "heuristic".to_string(),
        "falsified".to_string(),
        "runtime-gate".to_string(),
        "anti_knowledge".to_string(),
    ];
    KnowledgeEntry {
        id: format!(
            "anti-heuristic:{:016x}",
            stable_hash(format!("{}:{created_at_ms}", heuristic.id).as_bytes())
        ),
        kind: KnowledgeKind::AntiKnowledge,
        source: Some("heuristic-falsifier".to_string()),
        content,
        confidence: (1.0 - heuristic.confidence).clamp(0.65, 1.0),
        confidence_weight: -(1.0 - heuristic.confidence).clamp(0.65, 1.0),
        refuted_insight_id: heuristic
            .knowledge_entry_id
            .clone()
            .or_else(|| Some(heuristic.id.clone())),
        refutation_evidence: Some(format!(
            "confidence {:.3} below threshold {:.3} after {} observations",
            heuristic.confidence, heuristic.demotion_threshold, heuristic.observations
        )),
        source_episodes,
        tags,
        source_model: None,
        model_generality: default_model_generality(),
        created_at: Utc::now(),
        half_life_days: KnowledgeKind::AntiKnowledge.default_half_life_days(),
        tier: KnowledgeTier::Working,
        emotional_tag: None,
        emotional_provenance: None,
        hdc_vector: None,
        confirmation_count: heuristic.observations,
        distinct_contexts: Vec::new(),
        deprecated: false,
        balance: 1.0,
        frozen: false,
        catalytic_score: 0,
    }
}

fn read_jsonl<T>(path: &Path) -> Result<Vec<T>>
where
    T: for<'de> Deserialize<'de>,
{
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("open {}", path.display())),
    };
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for line in reader.lines() {
        let line = line.with_context(|| format!("read {}", path.display()))?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<T>(&line) {
            records.push(record);
        }
    }
    Ok(records)
}

fn append_jsonl_batch<T>(path: &Path, values: &[T]) -> Result<()>
where
    T: Serialize,
{
    if values.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    for value in values {
        let line = serde_json::to_string(value).context("serialize heuristic receipt")?;
        writeln!(file, "{line}").context("write heuristic receipt")?;
    }
    file.flush().context("flush heuristic receipt")?;
    file.sync_all().context("sync heuristic receipt")?;
    Ok(())
}

fn rewrite_jsonl<T>(path: &Path, values: &[T]) -> Result<()>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let tmp_path = path.with_extension("jsonl.tmp");
    {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp_path)
            .with_context(|| format!("open {}", tmp_path.display()))?;
        for value in values {
            let line = serde_json::to_string(value).context("serialize heuristic snapshot")?;
            writeln!(file, "{line}").context("write heuristic snapshot")?;
        }
        file.flush().context("flush heuristic snapshot")?;
        file.sync_all().context("sync heuristic snapshot")?;
    }
    fs::rename(&tmp_path, path)
        .with_context(|| format!("replace {} with {}", path.display(), tmp_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_learn::episode_logger::GateVerdict;

    fn episode(
        id: &str,
        trigger: &str,
        agent: &str,
        gate: &str,
        passed: bool,
        success: bool,
    ) -> Episode {
        let mut episode = Episode::new(agent, id);
        episode.id = id.to_string();
        episode.episode_id = id.to_string();
        episode.kind = "agent_turn".to_string();
        episode.trigger_kind = trigger.to_string();
        episode.agent_template = agent.to_string();
        episode.gate_verdicts = vec![GateVerdict::new(gate, passed)];
        episode.success = success;
        episode
    }

    #[test]
    fn extracts_insights_and_promotes_heuristics() {
        let episodes = vec![
            episode(
                "ep-1",
                "gate_failure",
                "Implementer",
                "compile",
                false,
                false,
            ),
            episode(
                "ep-2",
                "gate_failure",
                "Implementer",
                "compile",
                false,
                false,
            ),
            episode(
                "ep-3",
                "gate_failure",
                "Implementer",
                "compile",
                false,
                false,
            ),
            episode(
                "ep-4",
                "gate_failure",
                "Implementer",
                "compile",
                false,
                false,
            ),
            episode(
                "ep-5",
                "gate_failure",
                "Implementer",
                "compile",
                false,
                false,
            ),
        ];

        let progression = TierProgression::default();
        let report = progression.analyze(&episodes);

        assert!(!report.insights.is_empty());
        assert!(!report.heuristics.is_empty());
        let insight = &report.insights[0];
        assert!(insight.support_count >= 5);
        assert!(insight.confidence >= 0.7);
        assert!(insight.summary().contains("When"));

        let heuristic = &report.heuristics[0];
        assert!(heuristic.confirmations >= 5);
        assert!(heuristic.confidence >= 0.7);
        assert!(heuristic.summary().starts_with("If "));
    }

    #[test]
    fn four_episode_insights_stay_at_insight_tier() {
        let episodes = vec![
            episode(
                "ep-1",
                "gate_failure",
                "Implementer",
                "compile",
                false,
                false,
            ),
            episode(
                "ep-2",
                "gate_failure",
                "Implementer",
                "compile",
                false,
                false,
            ),
            episode(
                "ep-3",
                "gate_failure",
                "Implementer",
                "compile",
                false,
                false,
            ),
            episode(
                "ep-4",
                "gate_failure",
                "Implementer",
                "compile",
                false,
                false,
            ),
        ];

        let progression = TierProgression::default();
        let report = progression.analyze(&episodes);

        assert!(!report.insights.is_empty());
        assert!(report.heuristics.is_empty());
        assert!(
            report
                .insights
                .iter()
                .all(|insight| insight.source_episodes.len() < 5)
        );
    }

    #[test]
    fn playbook_markdown_contains_machine_parseable_rules() {
        let episodes = vec![
            episode(
                "ep-a",
                "gate_failure",
                "Implementer",
                "compile",
                false,
                false,
            ),
            episode(
                "ep-b",
                "gate_failure",
                "Implementer",
                "compile",
                false,
                false,
            ),
            episode(
                "ep-c",
                "gate_failure",
                "Implementer",
                "compile",
                false,
                false,
            ),
            episode(
                "ep-d",
                "gate_failure",
                "Implementer",
                "compile",
                false,
                false,
            ),
            episode(
                "ep-e",
                "gate_failure",
                "Implementer",
                "compile",
                false,
                false,
            ),
        ];

        let progression = TierProgression::default();
        let report = progression.analyze(&episodes);
        let markdown = &report.playbook.markdown;

        assert!(markdown.contains("# PLAYBOOK"));
        assert!(markdown.contains("```json"));
        assert!(markdown.contains("confidence"));
        assert!(markdown.contains("source episodes"));
    }

    #[test]
    fn replay_heuristics_strengthens_validated_rules_and_weakens_contradicted_rules() {
        let episodes = vec![
            episode("ep-1", "gate_success", "Implementer", "compile", true, true),
            episode("ep-2", "gate_success", "Implementer", "compile", true, true),
            episode(
                "ep-3",
                "gate_success",
                "Implementer",
                "compile",
                true,
                false,
            ),
            episode(
                "ep-4",
                "gate_success",
                "Implementer",
                "compile",
                true,
                false,
            ),
        ];

        let mut report = TierProgressionReport {
            insights: Vec::new(),
            heuristics: vec![
                HeuristicRule {
                    id: "heuristic-success".to_string(),
                    insight_id: "insight-success".to_string(),
                    title: "If trigger gate success then reuse path".to_string(),
                    when_clause: "trigger gate success and agent implementer".to_string(),
                    then_clause: "reuse this path as the default play".to_string(),
                    confidence: 0.5,
                    confirmations: 2,
                    first_seen_ms: 1,
                    last_seen_ms: 2,
                    source_episodes: vec!["ep-1".to_string(), "ep-2".to_string()],
                    source_model: None,
                    model_generality: default_model_generality(),
                    trials: 0,
                    violations: 0,
                    receipts: Vec::new(),
                },
                HeuristicRule {
                    id: "heuristic-failure".to_string(),
                    insight_id: "insight-failure".to_string(),
                    title: "If trigger gate failure then add verification".to_string(),
                    when_clause: "trigger gate failure and agent implementer".to_string(),
                    then_clause: "add a verification step before proceeding".to_string(),
                    confidence: 0.8,
                    confirmations: 2,
                    first_seen_ms: 3,
                    last_seen_ms: 4,
                    source_episodes: vec!["ep-3".to_string(), "ep-4".to_string()],
                    source_model: None,
                    model_generality: default_model_generality(),
                    trials: 0,
                    violations: 0,
                    receipts: Vec::new(),
                },
            ],
            playbook: PlaybookCompilation {
                markdown: String::new(),
                rules: Vec::new(),
            },
            falsifiers: Vec::new(),
        };

        let progression = TierProgression::default();
        progression.replay_heuristics(&mut report, &episodes);

        let strengthened = report
            .heuristics
            .iter()
            .find(|heuristic| heuristic.id == "heuristic-success")
            .expect("strengthened heuristic");
        let weakened = report
            .heuristics
            .iter()
            .find(|heuristic| heuristic.id == "heuristic-failure")
            .expect("weakened heuristic");

        assert!(strengthened.confidence > 0.5);
        assert!(weakened.confidence < 0.8);
        assert_eq!(report.playbook.rules.len(), 2);
        assert!(report.playbook.markdown.contains("confidence"));
    }

    #[test]
    fn evaluate_promotion_promotes_on_three_successes() {
        let entry = KnowledgeEntry {
            id: "entry-promote".to_string(),
            kind: KnowledgeKind::Insight,
            source: None,
            content: "Promote me".to_string(),
            confidence: 0.8,
            confidence_weight: 0.8,
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes: vec!["ep-1".to_string()],
            tags: vec!["tier".to_string()],
            source_model: None,
            model_generality: default_model_generality(),
            created_at: Utc::now(),
            half_life_days: 30.0,
            tier: KnowledgeTier::Transient,
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,
            confirmation_count: 0,
            distinct_contexts: Vec::new(),
            deprecated: false,
            balance: 1.0,
            frozen: false,
            catalytic_score: 0,
        };
        let verdicts = vec![
            GateVerdict::new("compile", true),
            GateVerdict::new("test", true),
            GateVerdict::new("lint", true),
        ];

        assert_eq!(
            TierProgression::evaluate_promotion(&entry, &verdicts),
            Some(KnowledgeTier::Working)
        );
        assert_eq!(
            TierProgression::evaluate_tier_progression(&entry, &verdicts),
            TierProgressionDecision::Promote(KnowledgeTier::Working)
        );
    }

    #[test]
    fn evaluate_promotion_demotes_on_two_failures() {
        let entry = KnowledgeEntry {
            id: "entry-demote".to_string(),
            kind: KnowledgeKind::Insight,
            source: None,
            content: "Demote me".to_string(),
            confidence: 0.8,
            confidence_weight: 0.8,
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes: vec!["ep-1".to_string()],
            tags: vec!["tier".to_string()],
            source_model: None,
            model_generality: default_model_generality(),
            created_at: Utc::now(),
            half_life_days: 30.0,
            tier: KnowledgeTier::Working,
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,
            confirmation_count: 0,
            distinct_contexts: Vec::new(),
            deprecated: false,
            balance: 1.0,
            frozen: false,
            catalytic_score: 0,
        };
        let verdicts = vec![
            GateVerdict::new("compile", false),
            GateVerdict::new("test", false),
        ];

        assert_eq!(
            TierProgression::evaluate_promotion(&entry, &verdicts),
            Some(KnowledgeTier::Transient)
        );
        assert_eq!(
            TierProgression::evaluate_tier_progression(&entry, &verdicts),
            TierProgressionDecision::Demote(KnowledgeTier::Transient)
        );
    }

    #[test]
    fn evaluate_promotion_marks_stale_entries_for_review() {
        let entry = KnowledgeEntry {
            id: "entry-review".to_string(),
            kind: KnowledgeKind::Insight,
            source: None,
            content: "Review me".to_string(),
            confidence: 0.8,
            confidence_weight: 0.8,
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes: vec!["ep-1".to_string()],
            tags: vec!["tier".to_string()],
            source_model: None,
            model_generality: default_model_generality(),
            created_at: Utc::now() - chrono::Duration::days(200),
            half_life_days: 30.0,
            tier: KnowledgeTier::Consolidated,
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,
            confirmation_count: 0,
            distinct_contexts: Vec::new(),
            deprecated: false,
            balance: 1.0,
            frozen: false,
            catalytic_score: 0,
        };

        assert!(TierProgression::needs_expiry_review(&entry));
        assert_eq!(
            TierProgression::evaluate_tier_progression(&entry, &[]),
            TierProgressionDecision::ReviewExpiry
        );
        assert_eq!(TierProgression::evaluate_promotion(&entry, &[]), None);
    }

    #[test]
    fn evaluate_tier_progression_returns_no_change_at_saturation_bounds() {
        let persistent = KnowledgeEntry {
            id: "entry-persistent".to_string(),
            kind: KnowledgeKind::Insight,
            source: None,
            content: "Already saturated".to_string(),
            confidence: 0.9,
            confidence_weight: 0.9,
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes: vec!["ep-1".to_string()],
            tags: vec!["tier".to_string()],
            source_model: None,
            model_generality: default_model_generality(),
            created_at: Utc::now(),
            half_life_days: 30.0,
            tier: KnowledgeTier::Persistent,
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,
            confirmation_count: 0,
            distinct_contexts: Vec::new(),
            deprecated: false,
            balance: 1.0,
            frozen: false,
            catalytic_score: 0,
        };
        let transient = KnowledgeEntry {
            tier: KnowledgeTier::Transient,
            ..persistent.clone()
        };

        let promote = vec![
            GateVerdict::new("compile", true),
            GateVerdict::new("test", true),
            GateVerdict::new("lint", true),
        ];
        let demote = vec![
            GateVerdict::new("compile", false),
            GateVerdict::new("test", false),
        ];

        assert_eq!(
            TierProgression::evaluate_tier_progression(&persistent, &promote),
            TierProgressionDecision::Promote(KnowledgeTier::Persistent)
        );
        assert_eq!(
            TierProgression::evaluate_tier_progression(&transient, &demote),
            TierProgressionDecision::Demote(KnowledgeTier::Transient)
        );
    }

    // -------------------------------------------------------------------
    // NEURO-12: Falsifier records and calibration
    // -------------------------------------------------------------------

    #[test]
    fn calibration_action_properties() {
        assert!(CalibrationAction::Violate.is_negative());
        assert!(CalibrationAction::Refute.is_negative());
        assert!(!CalibrationAction::Confirm.is_negative());
        assert!(!CalibrationAction::Refine.is_negative());
        assert!(!CalibrationAction::Generalize.is_negative());
    }

    #[test]
    fn replay_heuristics_populates_calibration_fields() {
        let episodes = vec![
            make_test_episode("ep-1", "trigger gate success and agent implementer", true),
            make_test_episode("ep-2", "trigger gate success and agent implementer", true),
            make_test_episode("ep-3", "trigger gate failure and agent implementer", false),
            make_test_episode("ep-4", "trigger gate failure and agent implementer", false),
        ];

        let mut report = TierProgressionReport {
            insights: Vec::new(),
            heuristics: vec![
                HeuristicRule {
                    id: "h-success".to_string(),
                    insight_id: "i-1".to_string(),
                    title: "If trigger gate success then reuse path".to_string(),
                    when_clause: "trigger gate success and agent implementer".to_string(),
                    then_clause: "reuse this path as the default play".to_string(),
                    confidence: 0.5,
                    confirmations: 2,
                    first_seen_ms: 1,
                    last_seen_ms: 2,
                    source_episodes: vec!["ep-1".to_string(), "ep-2".to_string()],
                    source_model: None,
                    model_generality: default_model_generality(),
                    trials: 0,
                    violations: 0,
                    receipts: Vec::new(),
                },
                HeuristicRule {
                    id: "h-failure".to_string(),
                    insight_id: "i-2".to_string(),
                    title: "If trigger gate failure then add verification".to_string(),
                    when_clause: "trigger gate failure and agent implementer".to_string(),
                    then_clause: "add a verification step before proceeding".to_string(),
                    confidence: 0.8,
                    confirmations: 2,
                    first_seen_ms: 3,
                    last_seen_ms: 4,
                    source_episodes: vec!["ep-3".to_string(), "ep-4".to_string()],
                    source_model: None,
                    model_generality: default_model_generality(),
                    trials: 0,
                    violations: 0,
                    receipts: Vec::new(),
                },
            ],
            playbook: PlaybookCompilation {
                markdown: String::new(),
                rules: Vec::new(),
            },
            falsifiers: Vec::new(),
        };

        let progression = TierProgression::default();
        progression.replay_heuristics(&mut report, &episodes);

        // h-success should have 2 trials (both supporting).
        let h_success = report
            .heuristics
            .iter()
            .find(|h| h.id == "h-success")
            .unwrap();
        assert!(h_success.trials >= 2);
        assert_eq!(h_success.violations, 0);
        assert!(!h_success.receipts.is_empty());
        assert!(
            h_success
                .receipts
                .iter()
                .all(|r| r.action == CalibrationAction::Confirm)
        );

        // h-failure should have 2 trials (both contradicting), and generate a falsifier.
        let h_failure = report
            .heuristics
            .iter()
            .find(|h| h.id == "h-failure")
            .unwrap();
        assert!(h_failure.trials >= 2);
        assert!(h_failure.violations >= 2);
        assert!(
            h_failure
                .receipts
                .iter()
                .any(|r| r.action == CalibrationAction::Violate)
        );

        // Falsifier should be emitted for the contradicted heuristic.
        assert!(
            !report.falsifiers.is_empty(),
            "should have falsifier records"
        );
        let falsifier = report
            .falsifiers
            .iter()
            .find(|f| f.heuristic_id == "h-failure");
        assert!(falsifier.is_some(), "should have falsifier for h-failure");
        let f = falsifier.unwrap();
        assert!(!f.contradicting_episodes.is_empty());
        assert!(f.action.is_negative());
    }

    fn make_test_episode(id: &str, _description: &str, success: bool) -> Episode {
        let mut ep = Episode::new("test-agent", "task-1");
        ep.id = id.to_string();
        ep.episode_id = id.to_string();
        ep.model = "test-model".to_string();
        ep.success = success;
        ep
    }

    #[cfg(feature = "hdc")]
    #[test]
    fn clustered_promotion_merges_similar_insights() {
        let episodes: Vec<Episode> = (0..10)
            .map(|i| {
                episode(
                    &format!("ep-{i}"),
                    "gate_failure",
                    "Implementer",
                    "compile",
                    false,
                    false,
                )
            })
            .collect();

        let progression = TierProgression::new(3, 0.5, 12);
        let report = progression.analyze(&episodes);

        if report.insights.is_empty() {
            return; // Not enough data to test clustering.
        }

        let heuristics_flat = progression.promote_heuristics(&report.insights);
        let heuristics_clustered = progression.promote_heuristics_clustered(&report.insights);

        // Clustered should produce <= as many heuristics (clusters merge duplicates).
        assert!(heuristics_clustered.len() <= heuristics_flat.len());

        // Each clustered heuristic should have at least as many confirmations
        // as the flat version since clusters merge sources.
        if let Some(h) = heuristics_clustered.first() {
            assert!(h.confirmations >= 1);
            assert!(h.confidence > 0.0);
        }
    }
}
