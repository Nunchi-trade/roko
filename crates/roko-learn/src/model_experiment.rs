//! Types for A/B testing model selection.
//!
//! This module defines the data model for model experiments. The execution
//! logic, assignment strategy, and persistence are added in later tasks.

use crate::cascade_router::CascadeRouter;
use crate::prompt_experiment::ExperimentStatus;
use roko_core::agent::AgentRole;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// A model A/B experiment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelExperiment {
    /// Unique experiment identifier.
    pub experiment_id: String,
    /// Human-readable description of the experiment.
    pub description: String,
    /// Optional role scope for the experiment.
    pub role: Option<String>,
    /// Optional task category scope for the experiment.
    pub task_category: Option<String>,
    /// Variants available in the experiment.
    pub variants: Vec<ModelVariant>,
    /// Per-variant statistics keyed by variant id.
    pub stats: HashMap<String, ModelVariantStats>,
    /// Current experiment status.
    pub status: ExperimentStatus,
    /// Winner variant id, if concluded.
    pub winner_id: Option<String>,
    /// Minimum trials per variant before the experiment can conclude.
    pub min_trials_per_variant: u64,
    /// Minimum effect size required to declare a winner.
    pub min_effect_size: f64,
    /// Experiment creation timestamp in ISO-8601 format.
    pub created_at: String,
}

/// A single model variant participating in an experiment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelVariant {
    /// Unique identifier for this variant.
    pub id: String,
    /// Key into the `[models.*]` configuration table.
    pub model_key: String,
    /// API model slug.
    pub slug: String,
    /// Provider key for the model.
    pub provider: String,
}

/// Per-variant stats for a model experiment.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelVariantStats {
    /// Number of trials run for this variant.
    pub trials: u64,
    /// Number of successful trials.
    pub successes: u64,
    /// Total cost accumulated in USD.
    pub total_cost_usd: f64,
    /// Total tokens consumed.
    pub total_tokens: u64,
    /// Total duration accumulated in milliseconds.
    pub total_duration_ms: u64,
    /// Success rate derived from `successes / trials`.
    pub pass_rate: f64,
    /// Average cost per trial in USD.
    pub avg_cost_usd: f64,
    /// Cost per successful trial in USD.
    pub cost_per_success: f64,
    /// Average duration per trial in milliseconds.
    pub avg_duration_ms: f64,
}

impl ModelVariantStats {
    /// Recompute derived metrics from the accumulated counters.
    fn recalculate(&mut self) {
        if self.trials == 0 {
            self.pass_rate = 0.0;
            self.avg_cost_usd = 0.0;
            self.cost_per_success = 0.0;
            self.avg_duration_ms = 0.0;
            return;
        }

        self.pass_rate = self.successes as f64 / self.trials as f64;
        self.avg_cost_usd = self.total_cost_usd / self.trials as f64;
        self.avg_duration_ms = self.total_duration_ms as f64 / self.trials as f64;
        self.cost_per_success = if self.successes == 0 {
            0.0
        } else {
            self.total_cost_usd / self.successes as f64
        };
    }

    /// UCB1 score for variant selection.
    #[allow(clippy::cast_precision_loss)]
    fn ucb_score(&self, total_trials: u64) -> f64 {
        if self.trials == 0 || total_trials == 0 {
            return f64::MAX;
        }

        let mean = self.successes as f64 / self.trials as f64;
        let exploration = (2.0 * (total_trials as f64).ln() / self.trials as f64).sqrt();
        mean + exploration
    }
}

impl ModelExperiment {
    /// Select the next model variant to use.
    ///
    /// Concluded experiments always return the winner. Running experiments
    /// use UCB1, with unsampled variants selected before sampled ones.
    pub fn assign_variant(&self) -> Option<&ModelVariant> {
        if self.status == ExperimentStatus::Concluded {
            return self
                .variants
                .iter()
                .find(|variant| Some(&variant.id) == self.winner_id.as_ref());
        }

        let total: u64 = self.stats.values().map(|stats| stats.trials).sum();
        let mut best = None;
        let mut best_score = f64::NEG_INFINITY;

        for variant in &self.variants {
            let score = self
                .stats
                .get(&variant.id)
                .map(|stats| stats.ucb_score(total))
                .unwrap_or(f64::MAX);
            if score > best_score {
                best_score = score;
                best = Some(variant);
            }
        }

        best
    }

    /// Record an outcome for a model variant and update experiment state.
    pub fn record_outcome(
        &mut self,
        variant_id: &str,
        success: bool,
        cost_usd: f64,
        tokens: u64,
        duration_ms: u64,
    ) -> bool {
        let stats = self.stats.entry(variant_id.to_string()).or_default();
        stats.trials += 1;
        if success {
            stats.successes += 1;
        }
        stats.total_cost_usd += cost_usd;
        stats.total_tokens += tokens;
        stats.total_duration_ms += duration_ms;
        stats.recalculate();

        if self.status == ExperimentStatus::Running {
            if let Some(winner_id) = self.check_conclusion() {
                self.status = ExperimentStatus::Concluded;
                self.winner_id = Some(winner_id);
                return true;
            }
        }

        false
    }

    /// Check whether the experiment has enough evidence to conclude.
    fn check_conclusion(&self) -> Option<String> {
        if self.variants.is_empty() {
            return None;
        }

        let mut ranked = Vec::with_capacity(self.variants.len());
        for variant in &self.variants {
            let stats = self.stats.get(&variant.id)?;
            if stats.trials < self.min_trials_per_variant {
                return None;
            }
            ranked.push((variant.id.as_str(), stats.pass_rate));
        }

        if ranked.len() == 1 {
            return Some(ranked[0].0.to_string());
        }

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

/// Persisted registry of model experiments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelExperimentStore {
    /// All experiments keyed by experiment id.
    #[serde(default)]
    experiments: HashMap<String, ModelExperiment>,
}

impl ModelExperimentStore {
    /// Load a store from disk, or create an empty store if the file is missing
    /// or invalid.
    pub fn load_or_new(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Save the store to disk using an atomic rename.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be serialized, if the target
    /// directory or snapshot file cannot be written, or if syncing the
    /// cascade-router mirror fails.
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, path)?;
        self.sync_cascade_router(path)?;
        Ok(())
    }

    /// Register a new experiment if it is not already present.
    pub fn register(&mut self, experiment: ModelExperiment) {
        self.experiments
            .entry(experiment.experiment_id.clone())
            .or_insert(experiment);
    }

    /// Find an active experiment scoped to a specific role.
    pub fn active_for_role(&self, role: &str) -> Option<&ModelExperiment> {
        self.experiments.values().find(|experiment| {
            experiment.status == ExperimentStatus::Running
                && experiment.role.as_deref() == Some(role)
        })
    }

    /// Find an active experiment scoped to a specific task category.
    pub fn active_for_category(&self, category: &str) -> Option<&ModelExperiment> {
        self.experiments.values().find(|experiment| {
            experiment.status == ExperimentStatus::Running
                && experiment.task_category.as_deref() == Some(category)
        })
    }

    fn applicable_experiment(&self, role: &str, category: &str) -> Option<&ModelExperiment> {
        self.experiments
            .values()
            .find(|experiment| {
                experiment.status == ExperimentStatus::Running
                    && experiment.role.as_deref() == Some(role)
                    && experiment.task_category.as_deref() == Some(category)
            })
            .or_else(|| self.active_for_role(role))
            .or_else(|| self.active_for_category(category))
    }

    /// Assign a model variant for the current role/category, if an active
    /// experiment applies.
    pub fn assign_model(&self, role: &str, category: &str) -> Option<ModelVariant> {
        self.assign_model_with_experiment(role, category)
            .map(|(_, variant)| variant)
    }

    /// Assign a model variant and return the owning experiment id.
    pub fn assign_model_with_experiment(
        &self,
        role: &str,
        category: &str,
    ) -> Option<(String, ModelVariant)> {
        let experiment = self.applicable_experiment(role, category)?;
        let variant = experiment.assign_variant()?.clone();
        Some((experiment.experiment_id.clone(), variant))
    }

    /// Record an outcome for a variant within a specific experiment.
    pub fn record_outcome(
        &mut self,
        experiment_id: &str,
        variant_id: &str,
        success: bool,
        cost: f64,
        tokens: u64,
        duration: u64,
    ) {
        let concluded = if let Some(experiment) = self.experiments.get_mut(experiment_id) {
            if experiment.record_outcome(variant_id, success, cost, tokens, duration) {
                Some(experiment.clone())
            } else {
                None
            }
        } else {
            None
        };

        if let Some(experiment) = concluded.as_ref() {
            self.on_conclusion(experiment);
        }
    }

    /// Number of currently running experiments.
    pub fn running_count(&self) -> usize {
        self.experiments
            .values()
            .filter(|experiment| experiment.status == ExperimentStatus::Running)
            .count()
    }

    /// Look up an experiment by id.
    pub fn get(&self, experiment_id: &str) -> Option<&ModelExperiment> {
        self.experiments.get(experiment_id)
    }

    /// All concluded experiments.
    pub fn concluded_experiments(&self) -> Vec<&ModelExperiment> {
        self.experiments
            .values()
            .filter(|experiment| experiment.status == ExperimentStatus::Concluded)
            .collect()
    }

    /// Iterate over all experiments.
    pub fn iter(&self) -> impl Iterator<Item = &ModelExperiment> {
        self.experiments.values()
    }

    fn on_conclusion(&self, experiment: &ModelExperiment) {
        if let Some(ref winner_id) = experiment.winner_id {
            tracing::info!(
                experiment = %experiment.experiment_id,
                winner = %winner_id,
                "model experiment concluded"
            );
        }
    }

    fn sync_cascade_router(&self, experiment_store_path: &Path) -> Result<(), std::io::Error> {
        let mut role_winners: HashMap<AgentRole, (String, String, String)> = HashMap::new();

        for experiment in self.experiments.values() {
            if experiment.status != ExperimentStatus::Concluded {
                continue;
            }

            let Some(role_raw) = experiment.role.as_deref() else {
                continue;
            };
            let Some(role) = parse_agent_role(role_raw) else {
                tracing::warn!(
                    experiment = %experiment.experiment_id,
                    role = role_raw,
                    "skipping concluded model experiment with unrecognized role"
                );
                continue;
            };
            let Some(winner_id) = experiment.winner_id.as_deref() else {
                continue;
            };
            let Some(winner) = experiment
                .variants
                .iter()
                .find(|variant| variant.id == winner_id)
            else {
                tracing::warn!(
                    experiment = %experiment.experiment_id,
                    winner = winner_id,
                    "skipping concluded model experiment with missing winner variant"
                );
                continue;
            };

            let should_replace = role_winners
                .get(&role)
                .map(|(created_at, _, _)| experiment.created_at >= *created_at)
                .unwrap_or(true);
            if should_replace {
                role_winners.insert(
                    role,
                    (
                        experiment.created_at.clone(),
                        winner.slug.clone(),
                        experiment.experiment_id.clone(),
                    ),
                );
            }
        }

        if role_winners.is_empty() {
            return Ok(());
        }

        let mut model_slugs: Vec<String> = self
            .experiments
            .values()
            .flat_map(|experiment| {
                experiment
                    .variants
                    .iter()
                    .map(|variant| variant.slug.clone())
                    .collect::<Vec<_>>()
            })
            .collect();
        model_slugs.sort();
        model_slugs.dedup();

        let router_path = experiment_store_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("cascade-router.json");
        let mut router = CascadeRouter::load_or_new(&router_path, model_slugs);

        for (role, (_, slug, experiment_id)) in role_winners {
            router.set_static_role_model(role, slug.clone());
            tracing::info!(
                experiment = %experiment_id,
                role = role.label(),
                winner_model = %slug,
                cascade_router = %router_path.display(),
                "updated cascade router static role mapping from concluded experiment"
            );
        }

        router.save(&router_path)
    }
}

fn parse_agent_role(raw: &str) -> Option<AgentRole> {
    if let Ok(role) = serde_json::from_str::<AgentRole>(&format!("\"{raw}\"")) {
        return Some(role);
    }

    std::iter::once(AgentRole::Conductor)
        .chain(AgentRole::ALL_AGENTS.iter().copied())
        .find(|role| raw == format!("{role:?}"))
}

impl Default for ModelExperimentStore {
    fn default() -> Self {
        Self {
            experiments: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cascade_router::{CascadeRouter, CascadeStage};
    use crate::model_router::RoutingContext;
    use roko_core::agent::AgentRole;
    use roko_core::task::{TaskCategory, TaskComplexityBand};

    fn make_variants() -> Vec<ModelVariant> {
        vec![
            ModelVariant {
                id: "a".into(),
                model_key: "model-a".into(),
                slug: "model-a".into(),
                provider: "provider-a".into(),
            },
            ModelVariant {
                id: "b".into(),
                model_key: "model-b".into(),
                slug: "model-b".into(),
                provider: "provider-b".into(),
            },
        ]
    }

    fn make_experiment(
        experiment_id: &str,
        role: Option<&str>,
        category: Option<&str>,
    ) -> ModelExperiment {
        ModelExperiment {
            experiment_id: experiment_id.into(),
            description: format!("Experiment {experiment_id}"),
            role: role.map(str::to_string),
            task_category: category.map(str::to_string),
            variants: make_variants(),
            stats: HashMap::new(),
            status: ExperimentStatus::Running,
            winner_id: None,
            min_trials_per_variant: 1,
            min_effect_size: 0.05,
            created_at: "2026-04-11T00:00:00Z".into(),
        }
    }

    #[test]
    fn model_experiment_types() {
        let experiment = ModelExperiment {
            experiment_id: "glm-vs-kimi".into(),
            description: "Compare models for implementer tasks".into(),
            role: Some("implementer".into()),
            task_category: Some("implementation".into()),
            variants: vec![
                ModelVariant {
                    id: "glm".into(),
                    model_key: "glm-5-1".into(),
                    slug: "glm-5.1".into(),
                    provider: "zai".into(),
                },
                ModelVariant {
                    id: "kimi".into(),
                    model_key: "kimi-k2-5".into(),
                    slug: "kimi-k2.5".into(),
                    provider: "moonshot".into(),
                },
            ],
            stats: HashMap::from([(
                "glm".into(),
                ModelVariantStats {
                    trials: 12,
                    successes: 9,
                    total_cost_usd: 2.4,
                    total_tokens: 18_000,
                    total_duration_ms: 54_000,
                    pass_rate: 0.75,
                    avg_cost_usd: 0.2,
                    cost_per_success: 0.266_666_666_7,
                    avg_duration_ms: 4_500.0,
                },
            )]),
            status: ExperimentStatus::Running,
            winner_id: None,
            min_trials_per_variant: 20,
            min_effect_size: 0.05,
            created_at: "2026-04-11T00:00:00Z".into(),
        };

        let json = serde_json::to_string(&experiment).expect("serialize");
        let decoded: ModelExperiment = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(decoded.experiment_id, "glm-vs-kimi");
        assert_eq!(decoded.variants.len(), 2);
        assert_eq!(decoded.stats["glm"].trials, 12);
        assert_eq!(decoded.status, ExperimentStatus::Running);
    }

    #[test]
    fn model_experiment_ucb() {
        let mut experiment = ModelExperiment {
            experiment_id: "glm-vs-kimi".into(),
            description: "Compare models for implementer tasks".into(),
            role: Some("implementer".into()),
            task_category: Some("implementation".into()),
            variants: vec![
                ModelVariant {
                    id: "glm".into(),
                    model_key: "glm-5-1".into(),
                    slug: "glm-5.1".into(),
                    provider: "zai".into(),
                },
                ModelVariant {
                    id: "kimi".into(),
                    model_key: "kimi-k2-5".into(),
                    slug: "kimi-k2.5".into(),
                    provider: "moonshot".into(),
                },
            ],
            stats: HashMap::new(),
            status: ExperimentStatus::Running,
            winner_id: None,
            min_trials_per_variant: 1,
            min_effect_size: 0.05,
            created_at: "2026-04-11T00:00:00Z".into(),
        };

        assert_eq!(
            experiment.assign_variant().map(|v| v.id.as_str()),
            Some("glm")
        );

        experiment.record_outcome("glm", true, 1.0, 100, 1_000);
        assert_eq!(
            experiment.assign_variant().map(|v| v.id.as_str()),
            Some("kimi")
        );

        experiment.record_outcome("kimi", false, 1.0, 100, 1_000);

        assert_eq!(experiment.status, ExperimentStatus::Concluded);
        assert_eq!(experiment.winner_id.as_deref(), Some("glm"));
        assert_eq!(
            experiment.assign_variant().map(|v| v.id.as_str()),
            Some("glm")
        );
        assert_eq!(experiment.stats["glm"].pass_rate, 1.0);
        assert_eq!(experiment.stats["glm"].avg_cost_usd, 1.0);
        assert_eq!(experiment.stats["glm"].cost_per_success, 1.0);
        assert_eq!(experiment.stats["glm"].avg_duration_ms, 1_000.0);
    }

    #[test]
    fn model_experiment_store_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model-experiments.json");

        let mut store = ModelExperimentStore::default();
        let mut concluded = make_experiment("category-exp", None, Some("implementation"));
        concluded.status = ExperimentStatus::Concluded;
        concluded.winner_id = Some("b".into());
        store.register(make_experiment("role-exp", Some("implementer"), None));
        store.register(concluded);

        store.save(&path).unwrap();

        let loaded = ModelExperimentStore::load_or_new(&path);
        assert_eq!(loaded.running_count(), 1);
        assert_eq!(loaded.concluded_experiments().len(), 1);
        assert_eq!(
            loaded
                .active_for_role("implementer")
                .map(|exp| exp.experiment_id.as_str()),
            Some("role-exp")
        );
        assert_eq!(
            loaded
                .active_for_category("implementation")
                .map(|exp| exp.experiment_id.as_str()),
            None
        );
        assert_eq!(
            loaded
                .assign_model("implementer", "implementation")
                .map(|variant| variant.id),
            Some("a".into())
        );
    }

    #[test]
    fn model_experiment_store_prefers_role_scope() {
        let mut store = ModelExperimentStore::default();
        store.register(make_experiment("role-exp", Some("implementer"), None));
        store.register(make_experiment(
            "category-exp",
            None,
            Some("implementation"),
        ));

        let assigned = store.assign_model("implementer", "implementation");
        assert_eq!(
            assigned.as_ref().map(|variant| variant.id.as_str()),
            Some("a")
        );
        assert_eq!(
            store
                .active_for_role("implementer")
                .map(|exp| exp.experiment_id.as_str()),
            Some("role-exp")
        );
        assert_eq!(
            store
                .active_for_category("implementation")
                .map(|exp| exp.experiment_id.as_str()),
            Some("category-exp")
        );
    }

    #[test]
    fn model_experiment_store_records_outcomes() {
        let mut store = ModelExperimentStore::default();
        let experiment =
            make_experiment("glm-vs-kimi", Some("implementer"), Some("implementation"));
        store.register(experiment);

        store.record_outcome("glm-vs-kimi", "a", true, 1.25, 120, 900);
        store.record_outcome("glm-vs-kimi", "b", false, 2.50, 240, 1_800);

        let concluded = store.concluded_experiments();
        assert_eq!(concluded.len(), 1);
        let experiment = concluded[0];
        assert_eq!(experiment.status, ExperimentStatus::Concluded);
        assert_eq!(experiment.winner_id.as_deref(), Some("a"));
        assert_eq!(experiment.stats["a"].trials, 1);
        assert_eq!(experiment.stats["a"].successes, 1);
        assert_eq!(experiment.stats["a"].total_cost_usd, 1.25);
        assert_eq!(experiment.stats["a"].total_tokens, 120);
        assert_eq!(experiment.stats["a"].total_duration_ms, 900);
        assert_eq!(experiment.stats["a"].pass_rate, 1.0);
        assert_eq!(experiment.stats["b"].trials, 1);
        assert_eq!(experiment.stats["b"].successes, 0);
        assert_eq!(store.running_count(), 0);
    }

    #[test]
    fn experiment_conclusion_updates_static_role_table() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("model-experiments.json");
        let router_path = dir.path().join("cascade-router.json");
        let router = CascadeRouter::new(vec![
            "claude-haiku-3-5".to_string(),
            "claude-sonnet-4-5".to_string(),
            "model-a".to_string(),
            "model-b".to_string(),
        ]);
        router.save(&router_path).unwrap();

        let mut store = ModelExperimentStore::default();
        store.register(make_experiment(
            "glm-vs-kimi",
            Some("implementer"),
            Some("implementation"),
        ));

        store.record_outcome("glm-vs-kimi", "a", true, 1.25, 120, 900);
        store.record_outcome("glm-vs-kimi", "b", false, 2.50, 240, 1_800);
        store.save(&store_path).unwrap();

        let reloaded = CascadeRouter::load_or_new(
            &router_path,
            vec![
                "claude-haiku-3-5".to_string(),
                "claude-sonnet-4-5".to_string(),
                "model-a".to_string(),
                "model-b".to_string(),
            ],
        );
        let routed = reloaded.route(&RoutingContext {
            task_category: TaskCategory::Implementation,
            complexity: TaskComplexityBand::Standard,
            iteration: 0,
            role: AgentRole::Implementer,
            crate_familiarity: 0.5,
            has_prior_failure: false,
            conductor_load: 0.0,
            active_agents: 0,
            ready_queue_depth: 0,
            max_queue_wait_hours: 0.0,
            daimon_policy: roko_core::DaimonPolicy::default(),
            thinking_level: None,
            temperament: None,
            previous_model: None,
            plan_context_tokens: None,
        });

        assert_eq!(routed.stage, CascadeStage::Static);
        assert_eq!(routed.primary.slug, "model-a");
    }
}
