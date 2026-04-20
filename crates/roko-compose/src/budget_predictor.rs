//! COMP-10: Budget prediction from task features and section influence scoring.
//!
//! [`BudgetPredictor`] predicts the optimal token budget for a task based on
//! its features (complexity, role, domain) and historical efficiency data. It
//! uses exponential moving average (EMA) of actual token usage per feature
//! combination to converge on a good budget without over- or under-allocating.
//!
//! [`SectionInfluence`] measures each prompt section's impact on task success
//! via a leave-one-out approximation: for each section, it tracks the success
//! rate of tasks that included the section vs the global baseline. Sections
//! with positive lift get higher weights; sections with negative lift get
//! lower weights or are dropped.
//!
//! # Integration
//!
//! The predictor is meant to be called from the composition layer before
//! assembling the prompt:
//!
//! 1. `BudgetPredictor::predict()` returns an estimated token budget.
//! 2. `SectionInfluence::weights()` returns per-section multipliers.
//! 3. These feed into `PromptComposer` to adjust section token caps and
//!    prioritization.
//!
//! # Persistence
//!
//! Both structs are serde-serializable and intended for storage in
//! `.roko/learn/budget-predictor.json` and `.roko/learn/section-influence.json`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Task features
// ---------------------------------------------------------------------------

/// Features describing a task for budget prediction.
///
/// These are the inputs to the predictor. The combination of `role`,
/// `complexity`, and `domain` forms the feature key.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TaskFeatures {
    /// Agent role (e.g., "Implementer", "Reviewer", "Researcher").
    pub role: String,
    /// Complexity band (e.g., "trivial", "standard", "complex").
    pub complexity: String,
    /// Task domain (e.g., "code", "research", "docs", "chain").
    pub domain: String,
}

impl TaskFeatures {
    /// The canonical key used for lookups.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}:{}:{}", self.role, self.complexity, self.domain)
    }

    /// Create features from component strings.
    #[must_use]
    pub fn new(
        role: impl Into<String>,
        complexity: impl Into<String>,
        domain: impl Into<String>,
    ) -> Self {
        Self {
            role: role.into(),
            complexity: complexity.into(),
            domain: domain.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// BudgetPredictor
// ---------------------------------------------------------------------------

/// Observation record for one task execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct BudgetObservation {
    /// EMA of actual tokens used for this feature combination.
    ema_tokens: f64,
    /// EMA of success rate (0.0..1.0).
    ema_success: f64,
    /// Number of observations.
    count: u32,
}

/// Predicts optimal token budgets from historical task data.
///
/// Uses per-feature-key EMA (exponential moving average) of actual token
/// usage, weighted by task success. When a task succeeds within budget,
/// the EMA converges toward the actual usage. When a task fails, the
/// predictor inflates the budget for that feature key.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BudgetPredictor {
    /// Per-feature-key observations.
    observations: HashMap<String, BudgetObservation>,
    /// EMA smoothing factor (0.0..1.0). Higher = more weight on recent data.
    /// Default: 0.3.
    #[serde(default = "default_alpha")]
    pub alpha: f64,
    /// Fallback budget when no history is available.
    #[serde(default = "default_fallback_tokens")]
    pub fallback_tokens: u64,
    /// Inflation factor applied when a task fails (budget was too small).
    #[serde(default = "default_failure_inflation")]
    pub failure_inflation: f64,
}

impl Default for BudgetPredictor {
    fn default() -> Self {
        Self {
            observations: HashMap::new(),
            alpha: default_alpha(),
            fallback_tokens: default_fallback_tokens(),
            failure_inflation: default_failure_inflation(),
        }
    }
}

fn default_alpha() -> f64 {
    0.3
}

fn default_fallback_tokens() -> u64 {
    100_000
}

fn default_failure_inflation() -> f64 {
    1.3
}

impl BudgetPredictor {
    /// Create a new predictor with default parameters.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Predict the optimal token budget for a task.
    ///
    /// Returns the EMA of actual usage for the feature key, inflated by
    /// 20% as a safety margin. If no history exists, returns the fallback
    /// budget.
    #[must_use]
    pub fn predict(&self, features: &TaskFeatures) -> u64 {
        let key = features.key();
        if let Some(obs) = self.observations.get(&key) {
            // Add 20% safety margin over the EMA.
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let predicted = (obs.ema_tokens * 1.2) as u64;
            predicted.max(1000) // minimum 1K tokens
        } else {
            // Try partial matches: same role+complexity, any domain.
            let partial_key = format!("{}:{}:", features.role, features.complexity);
            let partial_matches: Vec<&BudgetObservation> = self
                .observations
                .iter()
                .filter(|(k, _)| k.starts_with(&partial_key))
                .map(|(_, v)| v)
                .collect();

            if partial_matches.is_empty() {
                self.fallback_tokens
            } else {
                let avg_tokens: f64 = partial_matches.iter().map(|o| o.ema_tokens).sum::<f64>()
                    / partial_matches.len() as f64;
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let predicted = (avg_tokens * 1.2) as u64;
                predicted.max(1000)
            }
        }
    }

    /// Record the outcome of a task execution.
    ///
    /// Updates the EMA for the feature key based on actual token usage
    /// and whether the task succeeded.
    pub fn record(&mut self, features: &TaskFeatures, actual_tokens: u64, success: bool) {
        let key = features.key();
        #[allow(clippy::cast_precision_loss)]
        let actual = actual_tokens as f64;

        if let Some(obs) = self.observations.get_mut(&key) {
            // Existing observation: EMA update.
            let prev = obs.ema_tokens;
            obs.count += 1;
            obs.ema_tokens = self.alpha.mul_add(actual, (1.0 - self.alpha) * prev);
            let success_val = if success { 1.0 } else { 0.0 };
            obs.ema_success = self.alpha * success_val + (1.0 - self.alpha) * obs.ema_success;
        } else {
            // First observation: use actuals directly.
            self.observations.insert(
                key.clone(),
                BudgetObservation {
                    ema_tokens: actual,
                    ema_success: if success { 1.0 } else { 0.0 },
                    count: 1,
                },
            );
        }

        // If the task failed, inflate the EMA to encourage a larger budget next time.
        if !success {
            if let Some(obs) = self.observations.get_mut(&key) {
                obs.ema_tokens *= self.failure_inflation;
            }
        }
    }

    /// Number of unique feature keys with observations.
    #[must_use]
    pub fn observation_count(&self) -> usize {
        self.observations.len()
    }

    /// Whether there is any history for the given features.
    #[must_use]
    pub fn has_history(&self, features: &TaskFeatures) -> bool {
        self.observations.contains_key(&features.key())
    }
}

// ---------------------------------------------------------------------------
// SectionInfluence
// ---------------------------------------------------------------------------

/// Tracks per-section success statistics for leave-one-out influence scoring.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct SectionRecord {
    /// Number of tasks that included this section and succeeded.
    successes_with: u32,
    /// Number of tasks that included this section and failed.
    failures_with: u32,
    /// Number of tasks that excluded this section and succeeded.
    successes_without: u32,
    /// Number of tasks that excluded this section and failed.
    failures_without: u32,
}

impl SectionRecord {
    /// Success rate when section is included.
    fn rate_with(&self) -> f64 {
        let total = self.successes_with + self.failures_with;
        if total == 0 {
            0.5 // neutral prior
        } else {
            f64::from(self.successes_with) / f64::from(total)
        }
    }

    /// Success rate when section is excluded.
    fn rate_without(&self) -> f64 {
        let total = self.successes_without + self.failures_without;
        if total == 0 {
            0.5 // neutral prior
        } else {
            f64::from(self.successes_without) / f64::from(total)
        }
    }

    /// Lift: how much the section improves success rate.
    ///
    /// Positive lift means the section helps; negative means it hurts.
    fn lift(&self) -> f64 {
        self.rate_with() - self.rate_without()
    }

    /// Total observations for this section.
    fn total_obs(&self) -> u32 {
        self.successes_with + self.failures_with + self.successes_without + self.failures_without
    }
}

/// Leave-one-out section influence scorer (COMP-10).
///
/// For each prompt section, tracks whether its presence correlates with
/// task success. Sections with positive lift should receive higher token
/// budgets; sections with negative lift should be dropped or deprioritized.
///
/// # Approximation
///
/// True leave-one-out requires re-running each task without each section,
/// which is prohibitively expensive. Instead, we observe natural variation:
/// some tasks include a section (e.g., because it was available), others
/// do not (e.g., because context was missing). Over many observations,
/// this approximates the causal effect.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SectionInfluence {
    /// Per-section statistics, keyed by section name (e.g., "prd2", "context").
    sections: HashMap<String, SectionRecord>,
    /// Minimum observations before influence scores are trusted.
    #[serde(default = "default_min_obs")]
    pub min_observations: u32,
}

impl Default for SectionInfluence {
    fn default() -> Self {
        Self {
            sections: HashMap::new(),
            min_observations: default_min_obs(),
        }
    }
}

fn default_min_obs() -> u32 {
    10
}

impl SectionInfluence {
    /// Create a new influence tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the outcome of a task execution.
    ///
    /// `included_sections` is the set of section names that were present
    /// in the prompt. `all_sections` is the set of all known section names.
    /// `success` indicates whether the task succeeded.
    pub fn record(&mut self, included_sections: &[String], all_sections: &[String], success: bool) {
        let included_set: std::collections::HashSet<&String> = included_sections.iter().collect();

        for section in all_sections {
            let record = self.sections.entry(section.clone()).or_default();
            if included_set.contains(section) {
                if success {
                    record.successes_with += 1;
                } else {
                    record.failures_with += 1;
                }
            } else if success {
                record.successes_without += 1;
            } else {
                record.failures_without += 1;
            }
        }
    }

    /// Compute per-section weight multipliers.
    ///
    /// Returns a map from section name to a multiplier in `[0.5, 1.5]`.
    /// Sections with insufficient observations get 1.0 (neutral).
    /// Positive lift maps to >1.0; negative lift maps to <1.0.
    #[must_use]
    pub fn weights(&self) -> HashMap<String, f64> {
        self.sections
            .iter()
            .map(|(name, record)| {
                let weight = if record.total_obs() < self.min_observations {
                    1.0 // not enough data
                } else {
                    // Map lift [-1.0, 1.0] to weight [0.5, 1.5].
                    (1.0 + record.lift()).clamp(0.5, 1.5)
                };
                (name.clone(), weight)
            })
            .collect()
    }

    /// Get the raw lift value for a section (rate_with - rate_without).
    ///
    /// Returns `None` if the section has no observations.
    #[must_use]
    pub fn lift_for(&self, section: &str) -> Option<f64> {
        self.sections.get(section).map(SectionRecord::lift)
    }

    /// Number of tracked sections.
    #[must_use]
    pub fn section_count(&self) -> usize {
        self.sections.len()
    }
}

// ---------------------------------------------------------------------------
// Persistence helpers
// ---------------------------------------------------------------------------

/// Default persistence path for the budget predictor.
pub const BUDGET_PREDICTOR_FILENAME: &str = "budget-predictor.json";

/// Default persistence path for section influence data.
pub const SECTION_INFLUENCE_FILENAME: &str = "section-influence.json";

/// Persist the budget predictor to a JSON file under `learn_dir`.
///
/// # Errors
///
/// Returns an error if serialization or I/O fails.
pub fn persist_predictor(
    predictor: &BudgetPredictor,
    learn_dir: &std::path::Path,
) -> std::io::Result<()> {
    std::fs::create_dir_all(learn_dir)?;
    let path = learn_dir.join(BUDGET_PREDICTOR_FILENAME);
    let json = serde_json::to_string_pretty(predictor)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, json)
}

/// Load the budget predictor from a JSON file under `learn_dir`.
///
/// Returns `Ok(None)` if the file does not exist.
///
/// # Errors
///
/// Returns an error if the file exists but cannot be parsed.
pub fn load_predictor(learn_dir: &std::path::Path) -> std::io::Result<Option<BudgetPredictor>> {
    let path = learn_dir.join(BUDGET_PREDICTOR_FILENAME);
    if !path.exists() {
        return Ok(None);
    }
    let data = std::fs::read_to_string(&path)?;
    let predictor: BudgetPredictor = serde_json::from_str(&data)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(Some(predictor))
}

/// Persist the section influence data to a JSON file under `learn_dir`.
///
/// # Errors
///
/// Returns an error if serialization or I/O fails.
pub fn persist_influence(
    influence: &SectionInfluence,
    learn_dir: &std::path::Path,
) -> std::io::Result<()> {
    std::fs::create_dir_all(learn_dir)?;
    let path = learn_dir.join(SECTION_INFLUENCE_FILENAME);
    let json = serde_json::to_string_pretty(influence)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, json)
}

/// Load the section influence data from a JSON file under `learn_dir`.
///
/// Returns `Ok(None)` if the file does not exist.
///
/// # Errors
///
/// Returns an error if the file exists but cannot be parsed.
pub fn load_influence(learn_dir: &std::path::Path) -> std::io::Result<Option<SectionInfluence>> {
    let path = learn_dir.join(SECTION_INFLUENCE_FILENAME);
    if !path.exists() {
        return Ok(None);
    }
    let data = std::fs::read_to_string(&path)?;
    let influence: SectionInfluence = serde_json::from_str(&data)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(Some(influence))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // ── BudgetPredictor ──

    #[test]
    fn predict_returns_fallback_when_empty() {
        let predictor = BudgetPredictor::new();
        let features = TaskFeatures::new("Implementer", "standard", "code");
        assert_eq!(predictor.predict(&features), predictor.fallback_tokens);
    }

    #[test]
    fn predict_returns_ema_after_observations() {
        let mut predictor = BudgetPredictor::new();
        let features = TaskFeatures::new("Implementer", "standard", "code");

        predictor.record(&features, 80_000, true);
        predictor.record(&features, 120_000, true);

        let predicted = predictor.predict(&features);
        // Should be close to the EMA * 1.2 (safety margin).
        assert!(predicted > 80_000);
        assert!(predicted < 200_000);
    }

    #[test]
    fn failure_inflates_budget() {
        let mut predictor = BudgetPredictor::new();
        let features = TaskFeatures::new("Implementer", "standard", "code");

        predictor.record(&features, 100_000, true);
        let predicted_after_success = predictor.predict(&features);

        predictor.record(&features, 100_000, false);
        let predicted_after_failure = predictor.predict(&features);

        // After a failure, the budget should be inflated.
        assert!(predicted_after_failure > predicted_after_success);
    }

    #[test]
    fn partial_match_uses_same_role_complexity() {
        let mut predictor = BudgetPredictor::new();
        let code_features = TaskFeatures::new("Implementer", "standard", "code");
        predictor.record(&code_features, 90_000, true);

        let docs_features = TaskFeatures::new("Implementer", "standard", "docs");
        let predicted = predictor.predict(&docs_features);
        // Should use the partial match from code domain.
        assert!(predicted > 50_000);
        assert!(predicted < 200_000);
    }

    #[test]
    fn observation_count_tracks_keys() {
        let mut predictor = BudgetPredictor::new();
        assert_eq!(predictor.observation_count(), 0);

        predictor.record(&TaskFeatures::new("A", "s", "c"), 100_000, true);
        assert_eq!(predictor.observation_count(), 1);

        predictor.record(&TaskFeatures::new("B", "s", "c"), 100_000, true);
        assert_eq!(predictor.observation_count(), 2);

        // Same key, no new observation count.
        predictor.record(&TaskFeatures::new("A", "s", "c"), 100_000, true);
        assert_eq!(predictor.observation_count(), 2);
    }

    #[test]
    fn predictor_serializes_and_deserializes() {
        let mut predictor = BudgetPredictor::new();
        predictor.record(&TaskFeatures::new("R", "s", "d"), 80_000, true);

        let json = serde_json::to_string(&predictor).unwrap();
        let restored: BudgetPredictor = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.observation_count(), 1);
    }

    // ── SectionInfluence ──

    #[test]
    fn no_observations_returns_neutral_weights() {
        let influence = SectionInfluence::new();
        let weights = influence.weights();
        assert!(weights.is_empty());
    }

    #[test]
    fn section_with_positive_lift_gets_higher_weight() {
        let mut influence = SectionInfluence {
            min_observations: 2,
            ..SectionInfluence::default()
        };

        let all = vec!["prd2".into(), "context".into()];

        // Tasks with "prd2" succeed; tasks without it fail.
        for _ in 0..5 {
            influence.record(&["prd2".into(), "context".into()], &all, true);
            influence.record(&["context".into()], &all, false);
        }

        let weights = influence.weights();
        // "prd2" has positive lift (always present when success).
        assert!(weights["prd2"] > 1.0);
    }

    #[test]
    fn section_with_negative_lift_gets_lower_weight() {
        let mut influence = SectionInfluence {
            min_observations: 2,
            ..SectionInfluence::default()
        };

        let all = vec!["noise".into(), "core".into()];

        // Tasks with "noise" fail; tasks without it succeed.
        for _ in 0..5 {
            influence.record(&["noise".into(), "core".into()], &all, false);
            influence.record(&["core".into()], &all, true);
        }

        let weights = influence.weights();
        assert!(weights["noise"] < 1.0);
    }

    #[test]
    fn insufficient_observations_return_neutral() {
        let mut influence = SectionInfluence::new(); // min_observations = 10
        let all = vec!["sec".into()];

        // Only 4 observations, below threshold.
        for _ in 0..4 {
            influence.record(&["sec".into()], &all, true);
        }

        let weights = influence.weights();
        assert!((weights["sec"] - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn lift_for_returns_correct_value() {
        let mut influence = SectionInfluence {
            min_observations: 1,
            ..SectionInfluence::default()
        };
        let all = vec!["a".into()];

        // 3 successes with, 1 failure with, 0 without.
        influence.record(&["a".into()], &all, true);
        influence.record(&["a".into()], &all, true);
        influence.record(&["a".into()], &all, true);
        influence.record(&["a".into()], &all, false);

        let lift = influence.lift_for("a").unwrap();
        // rate_with = 3/4 = 0.75, rate_without = 0.5 (neutral prior), lift = 0.25
        assert!((lift - 0.25).abs() < 0.01);
    }

    #[test]
    fn influence_serializes_and_deserializes() {
        let mut influence = SectionInfluence::new();
        let all = vec!["sec".into()];
        influence.record(&["sec".into()], &all, true);

        let json = serde_json::to_string(&influence).unwrap();
        let restored: SectionInfluence = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.section_count(), 1);
    }

    // ── Persistence ──

    #[test]
    fn predictor_persist_and_load_roundtrips() {
        let dir = std::env::temp_dir().join("roko-test-bp");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut predictor = BudgetPredictor::new();
        predictor.record(&TaskFeatures::new("R", "s", "d"), 80_000, true);
        persist_predictor(&predictor, &dir).unwrap();

        let loaded = load_predictor(&dir).unwrap().unwrap();
        assert_eq!(loaded.observation_count(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn influence_persist_and_load_roundtrips() {
        let dir = std::env::temp_dir().join("roko-test-si");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut influence = SectionInfluence::new();
        influence.record(&["s".into()], &["s".into()], true);
        persist_influence(&influence, &dir).unwrap();

        let loaded = load_influence(&dir).unwrap().unwrap();
        assert_eq!(loaded.section_count(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_files_returns_none() {
        let dir = std::env::temp_dir().join("roko-test-missing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        assert!(load_predictor(&dir).unwrap().is_none());
        assert!(load_influence(&dir).unwrap().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
