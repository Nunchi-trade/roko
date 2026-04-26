//! Lightweight A/B testing framework for prompt section variants.
//!
//! Each experiment tests multiple variants of a prompt section (e.g. a
//! system-prompt paragraph). Variant selection is bandit-driven: exploration
//! favours under-sampled arms, then converges on the best performer once
//! evidence is strong.
//!
//! Persistence is a single JSON file managed by [`ExperimentStore`].

use roko_core::ExperimentWinnerSummary;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::Path;

/// Default path for persisted static overrides derived from concluded experiments.
pub const DEFAULT_STATIC_OVERRIDES_PATH: &str = ".roko/learn/static-overrides.json";

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

/// Winner derived from a concluded prompt experiment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperimentWinner {
    /// Experiment identifier that produced the winner.
    pub experiment_id: String,
    /// Parameter being overridden, typically a role or section name.
    pub parameter: String,
    /// Winning value that should become the new default.
    pub winning_value: String,
    /// Derived confidence in `[0.0, 1.0]`.
    pub confidence: f64,
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

    /// Wilson 95% confidence interval for the empirical success rate.
    #[allow(clippy::cast_precision_loss)]
    fn confidence_interval_95(&self) -> (f64, f64) {
        if self.trials == 0 {
            return (0.0, 0.0);
        }

        let n = self.trials as f64;
        let p = self.success_rate();
        let z = 1.96_f64;
        let z_sq = z * z;
        let denom = 1.0 + z_sq / n;
        let center = (p + z_sq / (2.0 * n)) / denom;
        let margin = (z / denom) * ((p * (1.0 - p) / n + z_sq / (4.0 * n * n)).sqrt());
        (
            (center - margin).clamp(0.0, 1.0),
            (center + margin).clamp(0.0, 1.0),
        )
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

    /// Return a concluded winner when the experiment has enough evidence.
    #[must_use]
    pub fn concluded_winner(&self) -> Option<ExperimentWinner> {
        if self.status != ExperimentStatus::Concluded {
            return None;
        }

        let winner_id = self.winner_id.as_deref()?;
        let winner = self
            .variants
            .iter()
            .find(|variant| variant.id == winner_id)?;
        let confidence = self.winner_confidence(winner_id)?;
        if confidence < 0.95 {
            return None;
        }

        Some(ExperimentWinner {
            experiment_id: self.experiment_id.clone(),
            parameter: self
                .role
                .clone()
                .unwrap_or_else(|| self.section_name.clone()),
            winning_value: winner
                .slug
                .clone()
                .unwrap_or_else(|| winner.content.clone()),
            confidence,
        })
    }

    /// Return a detailed summary for dashboard rendering.
    #[must_use]
    pub fn winner_summary(&self) -> Option<ExperimentWinnerSummary> {
        let winner = self.concluded_winner()?;
        let winner_id = self.winner_id.as_deref()?;
        let winner_variant = self
            .variants
            .iter()
            .find(|variant| variant.id == winner_id)?;
        let winner_stats = self.stats.get(winner_id)?;
        let (ci_lower, ci_upper) = winner_stats.confidence_interval_95();

        Some(ExperimentWinnerSummary {
            experiment_id: self.experiment_id.clone(),
            parameter: winner.parameter,
            winner: winner_variant_label(winner_variant),
            winner_variant_id: winner_variant.id.clone(),
            win_rate: winner_stats.success_rate(),
            sample_size: winner_stats.trials,
            ci_lower,
            ci_upper,
            confidence: winner.confidence,
        })
    }

    fn winner_confidence(&self, winner_id: &str) -> Option<f64> {
        let mut ranked: Vec<(&str, &VariantStats, f64)> = self
            .variants
            .iter()
            .filter(|variant| variant.active)
            .filter_map(|variant| {
                self.stats
                    .get(&variant.id)
                    .map(|stats| (variant.id.as_str(), stats, stats.success_rate()))
            })
            .collect();
        if ranked.is_empty() {
            return None;
        }

        ranked.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        let (winner_ranked_id, winner_stats, winner_rate) = ranked
            .iter()
            .find(|(id, _, _)| *id == winner_id)
            .copied()
            .unwrap_or(ranked[0]);
        let second = ranked.iter().find(|(id, _, _)| *id != winner_ranked_id);
        let second_rate = second.map_or(0.0, |(_, _, rate)| *rate);

        let second_stats = second.map(|(_, stats, _)| *stats);
        let se = match second_stats {
            Some(second_stats) => {
                let winner_trials = winner_stats.trials.max(1) as f64;
                let second_trials = second_stats.trials.max(1) as f64;
                let winner_var = winner_rate * (1.0 - winner_rate) / winner_trials;
                let second_var = second_rate * (1.0 - second_rate) / second_trials;
                (winner_var + second_var).sqrt()
            }
            None => 0.0,
        };
        let gap = (winner_rate - second_rate).max(0.0);
        if se == 0.0 {
            Some(1.0)
        } else {
            Some((gap / (gap + se)).clamp(0.0, 1.0))
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

    // TODO: migrate remaining atomic write sites to roko_fs::atomic_write_json
    /// Save to a JSON file (atomic write).
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be serialized or if the output
    /// file cannot be created, written, or renamed.
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

    /// Return all concluded experiments with sufficiently high confidence.
    #[must_use]
    pub fn concluded_winners(&self) -> Vec<ExperimentWinner> {
        let mut winners: Vec<_> = self
            .experiments
            .values()
            .filter_map(PromptExperiment::concluded_winner)
            .collect();
        winners.sort_by(|a, b| {
            b.confidence
                .total_cmp(&a.confidence)
                .then_with(|| a.experiment_id.cmp(&b.experiment_id))
        });
        winners
    }

    /// Return concluded winners with confidence intervals for dashboard rendering.
    #[must_use]
    pub fn winner_summaries(&self) -> Vec<ExperimentWinnerSummary> {
        let mut winners = self
            .experiments
            .values()
            .filter_map(PromptExperiment::winner_summary)
            .collect::<Vec<_>>();
        winners.sort_by(|a, b| a.experiment_id.cmp(&b.experiment_id));
        winners
    }

    /// Return the winning variant of a concluded experiment, if it reached
    /// statistical significance (confidence >= 0.95).
    ///
    /// This is a convenience accessor; the auto-promotion in
    /// `LearningRuntime::record_completed_run` calls `on_experiment_concluded`
    /// which already promotes winners into the cascade router. This method
    /// exposes the winner for callers that need the variant content directly.
    pub fn promote_winner(&self, experiment_id: &str) -> Option<ExperimentWinner> {
        let experiment = self.experiments.get(experiment_id)?;
        let winner = experiment.concluded_winner()?;
        if winner.confidence >= 0.95 {
            Some(winner)
        } else {
            None
        }
    }

    /// Write concluded winners to the static-overrides file.
    ///
    /// # Errors
    ///
    /// Returns an error if the static-overrides file cannot be written.
    pub fn apply_winners(&self, winners: &[ExperimentWinner]) -> io::Result<()> {
        self.apply_winners_to(winners, Path::new(DEFAULT_STATIC_OVERRIDES_PATH))
    }

    /// Write concluded winners to `path`.
    ///
    /// # Errors
    ///
    /// Returns an error if the existing overrides cannot be parsed, or if the
    /// new overrides cannot be serialized, written, or renamed.
    pub fn apply_winners_to(&self, winners: &[ExperimentWinner], path: &Path) -> io::Result<()> {
        if winners.is_empty() {
            return Ok(());
        }

        let mut overrides: BTreeMap<String, String> = self
            .load_static_overrides_path(path)
            .unwrap_or_default()
            .into_iter()
            .collect();

        for winner in winners.iter().filter(|winner| winner.confidence >= 0.95) {
            overrides.insert(winner.parameter.clone(), winner.winning_value.clone());
        }

        write_static_overrides(path, &overrides)
    }

    fn load_static_overrides_path(&self, path: &Path) -> io::Result<HashMap<String, String>> {
        let contents = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(HashMap::new()),
            Err(err) => return Err(err),
        };
        let map = serde_json::from_str::<HashMap<String, String>>(&contents)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        Ok(map)
    }

    /// Promote all concluded experiment winners to the static config overrides
    /// file (INT-10: Experiments -> Static config).
    ///
    /// Returns the number of winners promoted. Only winners with confidence
    /// >= 0.95 are written.
    ///
    /// # Errors
    ///
    /// Returns an error if the static-overrides file cannot be written.
    pub fn promote_all_to_config(&self) -> io::Result<usize> {
        self.promote_all_to_config_at(Path::new(DEFAULT_STATIC_OVERRIDES_PATH))
    }

    /// Promote all concluded experiment winners to a specific path.
    ///
    /// # Errors
    ///
    /// Returns an error if the overrides file cannot be written.
    pub fn promote_all_to_config_at(&self, path: &Path) -> io::Result<usize> {
        let winners = self.concluded_winners();
        let promotable: Vec<_> = winners
            .into_iter()
            .filter(|w| w.confidence >= 0.95)
            .collect();
        let count = promotable.len();
        self.apply_winners_to(&promotable, path)?;
        Ok(count)
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

fn winner_variant_label(variant: &PromptVariant) -> String {
    variant
        .slug
        .clone()
        .filter(|slug| !slug.trim().is_empty())
        .or_else(|| (!variant.name.trim().is_empty()).then(|| variant.name.clone()))
        .unwrap_or_else(|| variant.id.clone())
}

fn write_static_overrides(path: &Path, overrides: &BTreeMap<String, String>) -> io::Result<()> {
    let json = serde_json::to_string_pretty(overrides)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
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
    fn concluded_winners_only_return_high_confidence_results() {
        let mut store = ExperimentStore::new();
        let mut exp = PromptExperiment::new(
            "exp-role",
            "model-routing",
            vec![PromptVariant {
                id: "winner".into(),
                name: "Winner".into(),
                section_name: "model-routing".into(),
                content: "claude-sonnet-4-6".into(),
                slug: Some("claude-sonnet-4-6".into()),
                active: true,
            }],
        );
        exp.role = Some("implementer".into());
        exp.status = ExperimentStatus::Concluded;
        exp.winner_id = Some("winner".into());
        store.register(exp);

        let winners = store.concluded_winners();
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].parameter, "implementer");
        assert_eq!(winners[0].winning_value, "claude-sonnet-4-6");
        assert!(winners[0].confidence >= 0.95);
    }

    #[test]
    fn winner_summaries_include_ci_and_stable_ordering() {
        let mut store = ExperimentStore::new();

        let mut exp_b = PromptExperiment::new("exp-b", "constraints", make_variants("constraints"));
        exp_b.status = ExperimentStatus::Concluded;
        exp_b.winner_id = Some("b".into());
        exp_b.stats.insert(
            "a".into(),
            VariantStats {
                trials: 80,
                successes: 8,
            },
        );
        exp_b.stats.insert(
            "b".into(),
            VariantStats {
                trials: 80,
                successes: 76,
            },
        );

        let mut exp_a = PromptExperiment::new("exp-a", "constraints", make_variants("constraints"));
        exp_a.status = ExperimentStatus::Concluded;
        exp_a.winner_id = Some("a".into());
        exp_a.stats.insert(
            "a".into(),
            VariantStats {
                trials: 96,
                successes: 92,
            },
        );
        exp_a.stats.insert(
            "b".into(),
            VariantStats {
                trials: 96,
                successes: 12,
            },
        );

        store.register(exp_b);
        store.register(exp_a);

        let winners = store.winner_summaries();
        assert_eq!(winners.len(), 2);
        assert_eq!(winners[0].experiment_id, "exp-a");
        assert_eq!(winners[1].experiment_id, "exp-b");
        assert_eq!(winners[0].winner_variant_id, "a");
        assert_eq!(winners[0].winner, "Variant A");
        assert_eq!(winners[0].sample_size, 96);
        assert!((winners[0].win_rate - (92.0 / 96.0)).abs() < f64::EPSILON);
        assert!(winners[0].ci_lower <= winners[0].win_rate);
        assert!(winners[0].ci_upper >= winners[0].win_rate);
    }

    #[test]
    fn apply_winners_writes_static_overrides() {
        let store = ExperimentStore::new();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("static-overrides.json");
        let winners = vec![ExperimentWinner {
            experiment_id: "exp-role".into(),
            parameter: "implementer".into(),
            winning_value: "claude-sonnet-4-6".into(),
            confidence: 0.99,
        }];

        store.apply_winners_to(&winners, &path).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let overrides: HashMap<String, String> = serde_json::from_str(&contents).unwrap();
        assert_eq!(
            overrides.get("implementer"),
            Some(&"claude-sonnet-4-6".to_string())
        );
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
