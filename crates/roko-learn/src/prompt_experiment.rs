//! Lightweight A/B testing framework for prompt section variants.
//!
//! Each experiment tests multiple variants of a prompt section (e.g. a
//! system-prompt paragraph). Variant selection is bandit-driven: exploration
//! favours under-sampled arms, then converges on the best performer once
//! evidence is strong.
//!
//! Persistence is a single JSON file managed by [`ExperimentStore`].

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// ─── Types ──────────────────────────────────────────────────────────────────

/// A single prompt variant within an experiment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptVariant {
    /// Unique identifier for this variant (e.g. "concise-v2").
    pub id: String,
    /// Human-readable label.
    pub name: String,
    /// The prompt section name this replaces (e.g. "constraints").
    pub section_name: String,
    /// The actual prompt text content.
    pub content: String,
    /// Optional model slug when the experiment is selecting among models.
    #[serde(default)]
    pub slug: Option<String>,
    /// Whether this variant is still eligible for selection.
    pub active: bool,
}

/// Per-variant outcome tracker.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VariantStats {
    /// Total number of times this variant has been assigned.
    pub trials: u64,
    /// Number of successful outcomes.
    pub successes: u64,
}

impl VariantStats {
    /// Empirical success rate.
    #[allow(clippy::cast_precision_loss)]
    pub fn success_rate(&self) -> f64 {
        if self.trials == 0 {
            0.0
        } else {
            self.successes as f64 / self.trials as f64
        }
    }

    /// UCB1-style score for arm selection (upper confidence bound).
    #[allow(clippy::cast_precision_loss)]
    fn ucb_score(&self, total_trials: u64) -> f64 {
        if self.trials == 0 {
            return f64::MAX; // Explore unsampled arms first.
        }
        let mean = self.successes as f64 / self.trials as f64;
        let exploration = (2.0 * (total_trials as f64).ln() / self.trials as f64).sqrt();
        mean + exploration
    }
}

/// Per-variant metric tracker.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct VariantMetricStats {
    /// Number of metric observations.
    samples: u64,
    /// Sum of all recorded metric values.
    sum: f64,
    /// Most recent metric observation.
    last: Option<f64>,
}

impl VariantMetricStats {
    /// Record one metric observation.
    fn record(&mut self, value: f64) {
        self.samples += 1;
        self.sum += value;
        self.last = Some(value);
    }
}

/// Status of a prompt experiment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExperimentStatus {
    /// Experiment is actively assigning variants.
    Running,
    /// A winner has been identified and the experiment is concluded.
    Concluded,
}

/// A prompt experiment tracks multiple variants for one prompt section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptExperiment {
    /// Unique experiment identifier.
    pub experiment_id: String,
    /// The prompt section under test.
    pub section_name: String,
    /// Optional agent role label when the experiment is selecting a role model.
    #[serde(default)]
    pub role: Option<String>,
    /// Available variants.
    pub variants: Vec<PromptVariant>,
    /// Per-variant statistics, keyed by variant id.
    pub stats: HashMap<String, VariantStats>,
    /// Per-variant metric observations, keyed by variant id.
    #[serde(default)]
    metric_stats: HashMap<String, VariantMetricStats>,
    /// Current experiment status.
    pub status: ExperimentStatus,
    /// Variant id of the winner, if concluded.
    pub winner_id: Option<String>,
    /// Minimum trials per variant before considering conclusion.
    pub min_trials_per_variant: u64,
    /// Required difference in success rate to declare a winner.
    pub min_effect_size: f64,
}

impl PromptExperiment {
    /// Create a new running experiment.
    pub fn new(
        experiment_id: impl Into<String>,
        section_name: impl Into<String>,
        variants: Vec<PromptVariant>,
    ) -> Self {
        let stats: HashMap<String, VariantStats> = variants
            .iter()
            .map(|v| (v.id.clone(), VariantStats::default()))
            .collect();
        Self {
            experiment_id: experiment_id.into(),
            section_name: section_name.into(),
            role: None,
            variants,
            stats,
            metric_stats: HashMap::new(),
            status: ExperimentStatus::Running,
            winner_id: None,
            min_trials_per_variant: 10,
            min_effect_size: 0.1,
        }
    }

    /// Select the next variant to use via UCB1.
    ///
    /// Returns `None` if the experiment is concluded.
    pub fn assign_variant(&self) -> Option<&PromptVariant> {
        if self.status == ExperimentStatus::Concluded {
            // Return the winner if concluded.
            return self
                .winner_id
                .as_ref()
                .and_then(|wid| self.variants.iter().find(|v| v.id == *wid));
        }

        let total: u64 = self.stats.values().map(|s| s.trials).sum();
        let mut best_variant = None;
        let mut best_score = f64::NEG_INFINITY;

        for variant in &self.variants {
            if !variant.active {
                continue;
            }
            let stats = self.stats.get(&variant.id).cloned().unwrap_or_default();
            let score = stats.ucb_score(total);
            if score > best_score {
                best_score = score;
                best_variant = Some(variant);
            }
        }

        best_variant
    }

    /// Record an outcome for a variant. Returns true if the experiment concluded.
    pub fn record_outcome(&mut self, variant_id: &str, success: bool) -> bool {
        if let Some(stats) = self.stats.get_mut(variant_id) {
            stats.trials += 1;
            if success {
                stats.successes += 1;
            }
        }

        // Check for conclusion.
        if self.status == ExperimentStatus::Running {
            if let Some(winner) = self.check_conclusion() {
                self.status = ExperimentStatus::Concluded;
                self.winner_id = Some(winner);
                return true;
            }
        }
        false
    }

    /// Record a numeric metric for a variant.
    pub fn record_metric(&mut self, variant_id: &str, metric: f64) {
        if !metric.is_finite() {
            return;
        }

        if self.stats.contains_key(variant_id) {
            self.metric_stats
                .entry(variant_id.to_string())
                .or_default()
                .record(metric);
        }
    }

    /// Check if we have enough data to declare a winner.
    ///
    /// Requires all active variants to have at least `min_trials_per_variant`
    /// and the best variant to lead the second-best by `min_effect_size`.
    fn check_conclusion(&self) -> Option<String> {
        let active_stats: Vec<(&str, &VariantStats)> = self
            .variants
            .iter()
            .filter(|v| v.active)
            .filter_map(|v| self.stats.get(&v.id).map(|s| (v.id.as_str(), s)))
            .collect();

        if active_stats.len() < 2 {
            return active_stats.first().map(|(id, _)| (*id).to_string());
        }

        // All variants must meet minimum trials.
        if active_stats
            .iter()
            .any(|(_, s)| s.trials < self.min_trials_per_variant)
        {
            return None;
        }

        // Sort by success rate descending.
        let mut ranked: Vec<_> = active_stats
            .iter()
            .map(|(id, s)| (*id, s.success_rate()))
            .collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let (best_id, best_rate) = ranked[0];
        let (_, second_rate) = ranked[1];

        if best_rate - second_rate >= self.min_effect_size {
            Some(best_id.to_string())
        } else {
            None
        }
    }
}

// ─── Store ──────────────────────────────────────────────────────────────────

/// Persisted experiment store: manages all active and concluded experiments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentStore {
    experiments: HashMap<String, PromptExperiment>,
}

impl ExperimentStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self {
            experiments: HashMap::new(),
        }
    }

    /// Load from a JSON file, or create empty if missing/corrupt.
    pub fn load_or_new(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Save to a JSON file (atomic write).
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Register a new experiment. No-op if `experiment_id` already exists.
    pub fn register(&mut self, experiment: PromptExperiment) {
        self.experiments
            .entry(experiment.experiment_id.clone())
            .or_insert(experiment);
    }

    /// Look up an experiment by id.
    pub fn get(&self, experiment_id: &str) -> Option<&PromptExperiment> {
        self.experiments.get(experiment_id)
    }

    /// Find a running experiment for the given prompt section name.
    pub fn active_for_section(&self, section_name: &str) -> Option<&PromptExperiment> {
        self.experiments
            .values()
            .find(|e| e.section_name == section_name && e.status == ExperimentStatus::Running)
    }

    /// Assign a variant for a given prompt section, if an active experiment exists.
    ///
    /// Returns `(variant_id, variant_content)` or `None` if no experiment.
    pub fn assign_variant(&self, experiment_name: &str) -> Option<(String, String)> {
        let experiment = self
            .experiments
            .values()
            .find(|e| e.experiment_id == experiment_name || e.section_name == experiment_name)?;
        let variant = experiment.assign_variant()?;
        Some((variant.id.clone(), variant.content.clone()))
    }

    /// Assign a variant for a given prompt section, if an active experiment exists.
    ///
    /// Returns `(variant_id, variant_content)` or `None` if no experiment.
    pub fn assign_variant_for_section(&self, section_name: &str) -> Option<(String, String)> {
        self.assign_variant(section_name)
    }

    /// Record an outcome by `variant_id` (searches all experiments).
    pub fn record_outcome(&mut self, variant_id: &str, success: bool) {
        for experiment in self.experiments.values_mut() {
            if experiment.stats.contains_key(variant_id) {
                experiment.record_outcome(variant_id, success);
                return;
            }
        }
    }

    /// Record a numeric metric for a variant within a specific experiment.
    pub fn record_metric(&mut self, experiment_id: &str, variant_id: &str, metric: f64) {
        if let Some(experiment) = self.experiments.get_mut(experiment_id) {
            experiment.record_metric(variant_id, metric);
        }
    }

    /// All experiments (for reporting).
    #[must_use]
    pub const fn experiments(&self) -> &HashMap<String, PromptExperiment> {
        &self.experiments
    }

    /// Running experiments count.
    pub fn running_count(&self) -> usize {
        self.experiments
            .values()
            .filter(|e| e.status == ExperimentStatus::Running)
            .count()
    }

    /// Concluded experiments count.
    pub fn concluded_count(&self) -> usize {
        self.experiments
            .values()
            .filter(|e| e.status == ExperimentStatus::Concluded)
            .count()
    }

    /// Iterate over all experiments.
    pub fn iter(&self) -> impl Iterator<Item = &PromptExperiment> {
        self.experiments.values()
    }
}

impl Default for ExperimentStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_variants(section: &str) -> Vec<PromptVariant> {
        vec![
            PromptVariant {
                id: "a".into(),
                name: "Variant A".into(),
                section_name: section.into(),
                content: "Be concise.".into(),
                slug: None,
                active: true,
            },
            PromptVariant {
                id: "b".into(),
                name: "Variant B".into(),
                section_name: section.into(),
                content: "Be verbose and thorough.".into(),
                slug: None,
                active: true,
            },
        ]
    }

    #[test]
    fn experiment_selects_unsampled_first() {
        let exp = PromptExperiment::new("test-1", "constraints", make_variants("constraints"));
        // Both unsampled — should return first variant.
        let v = exp.assign_variant().unwrap();
        assert!(v.id == "a" || v.id == "b");
    }

    #[test]
    fn experiment_concludes_when_gap_sufficient() {
        let mut exp = PromptExperiment::new("test-2", "style", make_variants("style"));
        exp.min_trials_per_variant = 5;
        exp.min_effect_size = 0.1;

        // Give variant "a" 100% success, "b" 0%.
        for _ in 0..5 {
            exp.record_outcome("a", true);
            exp.record_outcome("b", false);
        }

        assert_eq!(exp.status, ExperimentStatus::Concluded);
        assert_eq!(exp.winner_id.as_deref(), Some("a"));
    }

    #[test]
    fn store_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("experiments.json");

        let mut store = ExperimentStore::new();
        let exp = PromptExperiment::new("exp-1", "constraints", make_variants("constraints"));
        store.register(exp);

        store.save(&path).unwrap();
        let loaded = ExperimentStore::load_or_new(&path);
        assert_eq!(loaded.experiments().len(), 1);
        assert!(loaded.get("exp-1").is_some());
    }

    #[test]
    fn assign_variant_for_section_works() {
        let mut store = ExperimentStore::new();
        let exp = PromptExperiment::new("exp-1", "constraints", make_variants("constraints"));
        store.register(exp);

        let result = store.assign_variant_for_section("constraints");
        assert!(result.is_some());
        let (id, _content) = result.unwrap();
        assert!(id == "a" || id == "b");

        // No experiment for unknown section.
        assert!(store.assign_variant_for_section("unknown").is_none());
    }

    #[test]
    fn record_metric_updates_existing_variant_only() {
        let mut store = ExperimentStore::new();
        let exp = PromptExperiment::new("exp-1", "constraints", make_variants("constraints"));
        store.register(exp);

        store.record_metric("exp-1", "a", 0.75);
        store.record_metric("exp-1", "missing", 0.2);

        let experiment = store.get("exp-1").expect("experiment exists");
        let stats = experiment.metric_stats.get("a").expect("variant metrics");
        assert_eq!(stats.samples, 1);
        assert_eq!(stats.last, Some(0.75));
        assert_eq!(stats.sum, 0.75);
        assert!(!experiment.metric_stats.contains_key("missing"));
    }
}
