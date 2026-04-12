//! Three-stage cascade router for model selection (section 13.8-13.11).
//!
//! The cascade combines three routing strategies, automatically transitioning
//! as observation count grows:
//!
//! | Stage | Name | Observations | Strategy |
//! |-------|------|-------------|----------|
//! | 1 | Static | < 50 | Hardcoded role -> model table |
//! | 2 | Confidence | 50 - 200 | Empirical pass rates + confidence interval |
//! | 3 | UCB1 | > 200 | Full `LinUCB` contextual bandit |
//!
//! # [`CascadeModel`]
//!
//! The router returns a [`CascadeModel`] containing a primary model,
//! an ordered fallback chain, an optional context-overflow fallback,
//! and a latency SLA in milliseconds.
//!
//! # Thread safety
//!
//! The cascade wraps a [`LinUCBRouter`] and an additional
//! [`parking_lot::Mutex`] for confidence-stage statistics.

use parking_lot::Mutex;
use roko_agent::provider::ProviderError;
use roko_core::OperatingFrequency;
use roko_core::agent::{AgentRole, ModelSpec, ModelTier};
use roko_core::config::schema::RewardWeights;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::cfactor::{AgentDispatchBias, CFactor};
use crate::model_experiment::ModelExperimentStore;
use crate::model_router::{COLD_START_THRESHOLD, CONTEXT_DIM, LinUCBRouter, RoutingContext};
use crate::pareto::{ModelObservation, compute_pareto_frontier};
use crate::provider_health::ProviderHealthRegistry;

// ─── CascadeStage ───────────────────────────────────────────────────────────

/// Which routing stage is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CascadeStage {
    /// Stage 1: hardcoded role -> model table (< 50 observations).
    Static,
    /// Stage 2: empirical pass rates with confidence interval (50-200 observations).
    Confidence,
    /// Stage 3: full `LinUCB` contextual bandit (> 200 observations).
    Ucb,
}

impl CascadeStage {
    /// Human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Confidence => "confidence",
            Self::Ucb => "ucb",
        }
    }
}

impl std::fmt::Display for CascadeStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

// ─── CascadeModel ───────────────────────────────────────────────────────────

/// Routing recommendation from the cascade.
#[derive(Debug, Clone)]
pub struct CascadeModel {
    /// Primary model to use.
    pub primary: ModelSpec,
    /// Ordered fallback models to try after general failures.
    pub fallback_chain: Vec<ModelSpec>,
    /// Larger-context model to try when the primary overflows context.
    pub context_overflow_fallback: Option<ModelSpec>,
    /// Latency SLA in milliseconds.
    pub latency_sla_ms: u64,
    /// Which cascade stage produced this recommendation.
    pub stage: CascadeStage,
}

impl CascadeModel {
    /// Return the model to use for the given attempt number.
    ///
    /// Attempt 0 is the primary model. Subsequent attempts walk the fallback
    /// chain in order until it is exhausted.
    #[must_use]
    pub fn model_for_attempt(&self, attempt: usize) -> Option<&ModelSpec> {
        match attempt {
            0 => Some(&self.primary),
            _ => self.fallback_chain.get(attempt - 1),
        }
    }

    /// Return the best fallback to use for a provider-specific failure.
    #[must_use]
    pub fn fallback_for_error(&self, error: &ProviderError) -> Option<&ModelSpec> {
        match error {
            ProviderError::ContextOverflow => self.context_overflow_fallback.as_ref(),
            ProviderError::RateLimit { .. } => self
                .fallback_chain
                .iter()
                .find(|model| model.backend != self.primary.backend)
                .or_else(|| self.fallback_chain.first()),
            _ => self.fallback_chain.first(),
        }
    }
}

/// Selection result for raw-context routing.
#[derive(Debug, Clone)]
pub struct CascadeSelection {
    /// Model chosen by the router.
    pub model: ModelSpec,
    /// Total observations accumulated by the router when this selection was made.
    pub observations: u64,
    /// Which cascade stage produced the recommendation.
    pub stage: CascadeStage,
}

// ─── Confidence-stage stats ─────────────────────────────────────────────────

/// Threshold for transitioning from Confidence to UCB stage.
const CONFIDENCE_TO_UCB_THRESHOLD: u64 = 200;
/// Affect confidence below which the router biases toward stronger models.
const LOW_AFFECT_CONFIDENCE_THRESHOLD: f64 = 0.3;
/// C-Factor above which the router biases toward cheaper models.
const HIGH_CFACTOR_THRESHOLD: f64 = 0.8;
/// C-Factor below which the router biases toward stronger models.
const LOW_CFACTOR_THRESHOLD: f64 = 0.4;
/// Cold-start bonus for reusing the previous model.
const CACHE_AFFINITY_BONUS: f64 = 0.15;
/// Recompute the Pareto frontier after every 50 observations.
const PARETO_RECOMPUTE_INTERVAL: u64 = 50;

/// Per-model observation record for the confidence stage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ModelStats {
    /// Number of trials (selections) for this model.
    trials: u64,
    /// Number of successes (gate passes).
    successes: u64,
}

impl ModelStats {
    /// Empirical pass rate.
    #[allow(clippy::cast_precision_loss)]
    fn pass_rate(&self) -> f64 {
        if self.trials == 0 {
            0.0
        } else {
            self.successes as f64 / self.trials as f64
        }
    }

    /// Width of the 95% Wilson confidence interval (approximate).
    ///
    /// Uses a normal approximation: `1.96 * sqrt(p * (1-p) / n)`.
    /// Returns `f64::INFINITY` for zero trials.
    #[allow(clippy::cast_precision_loss)]
    fn confidence_width(&self) -> f64 {
        if self.trials == 0 {
            return f64::INFINITY;
        }
        let p = self.pass_rate();
        let n = self.trials as f64;
        1.96 * (p * (1.0 - p) / n).sqrt()
    }

    /// Upper confidence bound on the pass rate.
    fn upper_bound(&self) -> f64 {
        (self.pass_rate() + self.confidence_width()).min(1.0)
    }
}

// ─── Static role -> model table ─────────────────────────────────────────────

/// Build the default static role-to-model mapping.
///
/// Fast-tier roles get haiku, Standard-tier roles prefer Kimi or sonnet,
/// Premium-tier roles get opus.
fn default_role_model_table(model_slugs: &[String]) -> HashMap<AgentRole, String> {
    let mut table = HashMap::new();
    let all_roles: Vec<AgentRole> = std::iter::once(AgentRole::Conductor)
        .chain(AgentRole::ALL_AGENTS.iter().copied())
        .collect();
    for role in all_roles {
        let slug = match role.model_tier() {
            ModelTier::Fast => pick_static_slug(model_slugs, &["claude-haiku-3-5"]),
            ModelTier::Premium => pick_static_slug(model_slugs, &["claude-opus-4"]),
            // Standard and forward-compat
            _ => pick_static_slug(
                model_slugs,
                &[
                    "kimi-k2.5",
                    "kimi-k2-thinking",
                    "claude-sonnet-4-6",
                    "claude-sonnet-4-5",
                ],
            ),
        };
        table.insert(role, slug);
    }
    table
}

fn pick_static_slug(model_slugs: &[String], candidates: &[&str]) -> String {
    for candidate in candidates {
        if let Some(slug) = model_slugs
            .iter()
            .find(|slug| slugs_match(slug, candidate))
            .cloned()
        {
            return slug;
        }
    }
    candidates[0].to_string()
}

fn pick_available_static_slug(model_slugs: &[String], candidates: &[&str]) -> String {
    for candidate in candidates {
        if let Some(slug) = model_slugs
            .iter()
            .find(|slug| slugs_match(slug, candidate))
            .cloned()
        {
            return slug;
        }
    }

    model_slugs
        .first()
        .cloned()
        .unwrap_or_else(|| candidates[0].to_string())
}

/// Default latency SLA for a model tier (milliseconds).
const fn default_latency_sla(tier: ModelTier) -> u64 {
    match tier {
        ModelTier::Fast => 10_000,
        ModelTier::Premium => 120_000,
        // Standard and forward-compat
        _ => 30_000,
    }
}

/// Map a model slug to an approximate tier for SLA purposes.
fn slug_to_tier(slug: &str) -> ModelTier {
    if slug.contains("haiku") {
        ModelTier::Fast
    } else if slug.contains("opus") || slug.contains("premium") {
        ModelTier::Premium
    } else {
        ModelTier::Standard
    }
}

/// Build the ordered fallback chain for a routed primary model.
fn fallback_chain_for_model(model_slugs: &[String], primary_slug: &str) -> Vec<ModelSpec> {
    let primary_tier = slug_to_tier(primary_slug);

    if matches!(primary_tier, ModelTier::Fast) {
        return Vec::new();
    }

    let mut grouped = [Vec::new(), Vec::new(), Vec::new()];

    for slug in model_slugs {
        if slugs_match(slug, primary_slug) {
            continue;
        }

        let bucket = match primary_tier {
            ModelTier::Standard => match slug_to_tier(slug) {
                ModelTier::Fast => 0,
                ModelTier::Standard => 1,
                ModelTier::Premium => 2,
                _ => 1,
            },
            ModelTier::Premium => match slug_to_tier(slug) {
                ModelTier::Standard => 0,
                ModelTier::Fast => 1,
                ModelTier::Premium => 2,
                _ => 0,
            },
            _ => 0,
        };

        grouped[bucket].push(ModelSpec::from_slug(slug));
    }

    grouped.into_iter().flatten().collect()
}

/// Find a stronger model to use when the selected model overflows context.
fn context_overflow_fallback_for_model(
    model_slugs: &[String],
    primary_slug: &str,
) -> Option<ModelSpec> {
    let primary_rank = model_tier_rank(slug_to_tier(primary_slug));

    model_slugs
        .iter()
        .find(|slug| model_tier_rank(slug_to_tier(slug)) > primary_rank)
        .map(ModelSpec::from_slug)
}

fn low_confidence_tier_bonus(tier: ModelTier) -> f64 {
    match tier {
        ModelTier::Premium => 0.15,
        ModelTier::Standard => 0.05,
        ModelTier::Fast => 0.0,
        _ => 0.05,
    }
}

fn apply_cache_affinity(scores: &mut [(String, f64)], previous_model: Option<&str>) {
    if let Some(prev) = previous_model {
        for (slug, score) in scores.iter_mut() {
            if slug == prev {
                *score += CACHE_AFFINITY_BONUS;
            }
        }
    }
}

fn model_tier_rank(tier: ModelTier) -> u8 {
    match tier {
        ModelTier::Premium => 2,
        ModelTier::Standard => 1,
        ModelTier::Fast => 0,
        _ => 1,
    }
}

fn slugs_match(lhs: &str, rhs: &str) -> bool {
    lhs == rhs || slug_family(lhs).is_some_and(|family| slug_family(rhs) == Some(family))
}

fn slug_family(slug: &str) -> Option<&'static str> {
    if slug.starts_with("kimi-k2") {
        Some("kimi-k2")
    } else if slug.contains("haiku") {
        Some("haiku")
    } else if slug.contains("sonnet") {
        Some("sonnet")
    } else if slug.contains("opus") {
        Some("opus")
    } else {
        None
    }
}

// ─── CascadeRouter ──────────────────────────────────────────────────────────

/// Three-stage cascade router: Static -> Confidence -> UCB.
///
/// Thread-safe: wrap in `Arc` for shared access.
pub struct CascadeRouter {
    /// The `LinUCB` router used for stage 3 (and observations from all stages).
    linucb: LinUCBRouter,
    /// Per-model statistics for the confidence stage.
    confidence_stats: Mutex<HashMap<String, ModelStats>>,
    /// Cached Pareto frontier used to down-weight dominated models during UCB.
    pareto_frontier: Mutex<ParetoFrontierState>,
    /// Static role -> model table for stage 1.
    role_table: HashMap<AgentRole, String>,
    /// Ordered list of model slugs (arms available to the router).
    model_slugs: Vec<String>,
}

/// Cached Pareto frontier state.
#[derive(Debug, Clone, Default)]
struct ParetoFrontierState {
    frontier: Vec<String>,
    bucket: u64,
}

impl CascadeRouter {
    /// Create a cascade router with the given model slugs.
    ///
    /// # Panics
    ///
    /// Panics if `model_slugs` is empty.
    pub fn new(model_slugs: Vec<String>) -> Self {
        assert!(
            !model_slugs.is_empty(),
            "CascadeRouter: need at least one model"
        );
        Self {
            linucb: LinUCBRouter::new(model_slugs.clone()),
            confidence_stats: Mutex::new(HashMap::new()),
            pareto_frontier: Mutex::new(ParetoFrontierState::default()),
            role_table: default_role_model_table(&model_slugs),
            model_slugs,
        }
    }

    /// Override the static role table (builder pattern).
    #[must_use]
    pub fn with_role_table(mut self, table: HashMap<AgentRole, String>) -> Self {
        self.role_table = table;
        self
    }

    /// Update the Stage 1 static role-to-model mapping for one role.
    pub fn set_static_role_model(&mut self, role: AgentRole, model_slug: impl Into<String>) {
        self.role_table.insert(role, model_slug.into());
    }

    /// Override the `LinUCB` router (builder pattern, for injecting pre-trained state).
    #[must_use]
    pub fn with_linucb(mut self, linucb: LinUCBRouter) -> Self {
        self.linucb = linucb;
        self
    }

    /// Determine the current cascade stage based on total observations.
    #[must_use]
    pub fn current_stage(&self) -> CascadeStage {
        stage_for_observations(self.linucb.total_observations())
    }

    /// Total observations recorded across all stages.
    #[must_use]
    pub fn total_observations(&self) -> u64 {
        self.linucb.total_observations()
    }

    /// Select a model from a raw context vector.
    #[must_use]
    pub fn select(&self, context_vec: Vec<f64>) -> CascadeSelection {
        let observations = self.total_observations();
        let stage = stage_for_observations(observations);
        let model = self.linucb.select_features(&context_vec);
        CascadeSelection {
            model,
            observations,
            stage,
        }
    }

    /// Select a model for a given operating frequency.
    ///
    /// - `Gamma` returns `None` because reactive work is pure logic and should
    ///   not dispatch an LLM turn.
    /// - `Theta` uses the existing cascade router selection.
    /// - `Delta` always uses the strongest available model in the router.
    #[must_use]
    pub fn select_for_frequency(
        &self,
        frequency: OperatingFrequency,
        ctx: Option<&RoutingContext>,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> Option<ModelSpec> {
        match frequency {
            OperatingFrequency::Gamma => None,
            OperatingFrequency::Theta => {
                ctx.map(|ctx| self.route_with_cfactor(ctx, cfactor, agent_id).primary)
            }
            OperatingFrequency::Delta => {
                Some(self.bias_model_for_cfactor(self.strongest_model(), cfactor, agent_id))
            }
        }
    }

    /// Return the strongest model currently available to the router.
    ///
    /// Preference order is premium > standard > fast. Within the same tier,
    /// the first slug wins so the choice stays stable.
    #[must_use]
    pub fn strongest_model(&self) -> ModelSpec {
        let mut best_slug = self
            .model_slugs
            .first()
            .cloned()
            .expect("CascadeRouter: need at least one model");
        let mut best_rank = model_tier_rank(slug_to_tier(&best_slug));

        for slug in self.model_slugs.iter().skip(1) {
            let rank = model_tier_rank(slug_to_tier(slug));
            if rank > best_rank {
                best_rank = rank;
                best_slug.clone_from(slug);
            }
        }

        ModelSpec::from_slug(best_slug)
    }

    /// Return the cheapest model currently available to the router.
    ///
    /// Preference order is fast < standard < premium. Within the same tier,
    /// the first slug wins so the choice stays stable.
    #[must_use]
    pub fn cheapest_model(&self) -> ModelSpec {
        let mut best_slug = self
            .model_slugs
            .first()
            .cloned()
            .expect("CascadeRouter: need at least one model");
        let mut best_rank = model_tier_rank(slug_to_tier(&best_slug));

        for slug in self.model_slugs.iter().skip(1) {
            let rank = model_tier_rank(slug_to_tier(slug));
            if rank < best_rank {
                best_rank = rank;
                best_slug.clone_from(slug);
            }
        }

        ModelSpec::from_slug(best_slug)
    }

    /// Return the index of `slug` in the router's model list.
    #[must_use]
    pub fn model_index_for_slug(&self, slug: &str) -> Option<usize> {
        self.linucb.model_index(slug)
    }

    /// Route a context through the cascade, returning a recommendation.
    pub fn route(&self, ctx: &RoutingContext) -> CascadeModel {
        self.route_with_cfactor(ctx, None, None)
    }

    /// Route a context through the cascade, overriding selection when a model
    /// experiment is active for the current role and task category.
    pub fn route_with_experiments(
        &self,
        ctx: &RoutingContext,
        experiments: &ModelExperimentStore,
    ) -> CascadeModel {
        if let Some(variant) = experiments.assign_model(ctx.role.label(), ctx.task_category.label())
        {
            return CascadeModel {
                primary: ModelSpec::from_slug(&variant.slug),
                fallback_chain: Vec::new(),
                context_overflow_fallback: None,
                latency_sla_ms: 30_000,
                stage: CascadeStage::Static,
            };
        }

        self.route(ctx)
    }

    /// Route a context through the cascade, excluding models whose provider
    /// is currently unavailable.
    ///
    /// Unknown providers are treated as available so unannotated models keep
    /// participating in routing.
    pub fn route_with_health(
        &self,
        ctx: &RoutingContext,
        health: &ProviderHealthRegistry,
        model_providers: &HashMap<String, String>,
    ) -> CascadeModel {
        let available: Vec<String> = self
            .model_slugs
            .iter()
            .filter(|slug| {
                model_providers
                    .get(slug.as_str())
                    .map(|provider_id| health.is_available(provider_id))
                    .unwrap_or(true)
            })
            .cloned()
            .collect();

        if available.is_empty() {
            return self.route(ctx);
        }

        match self.current_stage() {
            CascadeStage::Static => self.route_static_filtered(ctx, &available),
            CascadeStage::Confidence => self.route_confidence_filtered(ctx, &available),
            CascadeStage::Ucb => self.route_ucb_filtered(ctx, &available),
        }
    }

    /// Route a context through the cascade, optionally biasing by C-Factor.
    pub fn route_with_cfactor(
        &self,
        ctx: &RoutingContext,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        match self.current_stage() {
            CascadeStage::Static => self.route_static(ctx, cfactor, agent_id),
            CascadeStage::Confidence => self.route_confidence(ctx, cfactor, agent_id),
            CascadeStage::Ucb => self.route_ucb(ctx, cfactor, agent_id),
        }
    }

    /// Record an observation (updates both confidence stats and `LinUCB`).
    pub fn record_observation(
        &self,
        ctx: &RoutingContext,
        model_slug: &str,
        reward: f64,
        success: bool,
    ) {
        let Some(model_idx) = self.model_index_for_slug(model_slug) else {
            return;
        };
        self.observe_internal(&ctx.to_features(), model_idx, reward, success);
    }

    /// Record a binary outcome for `model_slug` without a full routing context.
    ///
    /// This is used by event-driven feedback paths that only know which model
    /// produced the episode, not the original routing features.
    pub fn record_outcome(&self, model_slug: &str, success: bool) -> bool {
        let Some(model_idx) = self.model_index_for_slug(model_slug) else {
            return false;
        };

        let reward = if success { 1.0 } else { 0.0 };
        let context = [0.0; CONTEXT_DIM];
        self.observe_internal(&context, model_idx, reward, success);
        true
    }

    /// Record a successful observation from a raw 18-dim context vector.
    ///
    /// This is the success-path entry point used by orchestration when the
    /// caller already has the model index in the router's arm list.
    pub fn observe(&self, context_vec: Vec<f64>, model_idx: usize, reward: f64) {
        self.observe_internal(&context_vec, model_idx, reward, true);
    }

    /// Record a successful multi-objective observation from a raw context vector.
    pub fn observe_multi_objective(
        &self,
        context_vec: Vec<f64>,
        model_idx: usize,
        quality: f64,
        normalized_cost: f64,
        normalized_latency: f64,
        weights: &RewardWeights,
    ) {
        let Some(slug) = self.model_slugs.get(model_idx) else {
            return;
        };

        let mut stats = self.confidence_stats.lock();
        let entry = stats.entry(slug.clone()).or_default();
        entry.trials += 1;
        entry.successes += 1;
        drop(stats);

        self.linucb.update_features_multi_objective(
            &context_vec,
            model_idx,
            quality,
            normalized_cost,
            normalized_latency,
            weights,
        );
    }

    fn observe_internal(&self, context_vec: &[f64], model_idx: usize, reward: f64, success: bool) {
        let Some(slug) = self.model_slugs.get(model_idx) else {
            return;
        };

        // Update confidence stats.
        let mut stats = self.confidence_stats.lock();
        let entry = stats.entry(slug.clone()).or_default();
        entry.trials += 1;
        if success {
            entry.successes += 1;
        }
        drop(stats);

        // Update LinUCB (always, so it's ready when stage transitions).
        self.linucb.update_features(context_vec, model_idx, reward);
    }

    /// Access the underlying `LinUCB` router (for introspection / persistence).
    pub const fn linucb(&self) -> &LinUCBRouter {
        &self.linucb
    }

    /// Snapshot of confidence-stage statistics.
    pub fn confidence_snapshot(&self) -> HashMap<String, (u64, u64)> {
        self.confidence_stats
            .lock()
            .iter()
            .map(|(k, v)| (k.clone(), (v.trials, v.successes)))
            .collect()
    }

    /// Save confidence stats, model slugs, and total observation count to a JSON file.
    ///
    /// `LinUCB` arm weights are not persisted (they re-learn from new observations).
    /// Confidence stats represent the accumulated pass-rate history needed for
    /// stage-2 routing, and the total observation count determines which cascade
    /// stage is active after reload.
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let snapshot = CascadeSnapshot {
            model_slugs: self.model_slugs.clone(),
            confidence_stats: self
                .confidence_stats
                .lock()
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        PersistedModelStats {
                            trials: v.trials,
                            successes: v.successes,
                        },
                    )
                })
                .collect(),
            total_observations: self.linucb.total_observations(),
            role_table: self.role_table.clone(),
        };
        let json = serde_json::to_string_pretty(&snapshot)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Load a cascade router from a persisted JSON file, or create a new one.
    ///
    /// If the file exists and parses correctly, the confidence stats are restored
    /// and the model slugs from the file are merged with the provided `model_slugs`
    /// (the union is used). If the file doesn't exist or fails to parse, a fresh
    /// router is created with the given `model_slugs`.
    ///
    /// # Panics
    ///
    /// Panics if `model_slugs` is empty and no persisted state exists.
    pub fn load_or_new(path: &Path, model_slugs: Vec<String>) -> Self {
        let snapshot = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<CascadeSnapshot>(&s).ok());

        match snapshot {
            Some(CascadeSnapshot {
                model_slugs: persisted_model_slugs,
                confidence_stats,
                total_observations,
                role_table,
            }) => {
                // Merge model sets: union of persisted + provided.
                let mut slugs: Vec<String> = persisted_model_slugs;
                for s in &model_slugs {
                    if !slugs.contains(s) {
                        slugs.push(s.clone());
                    }
                }
                if slugs.is_empty() {
                    slugs = model_slugs;
                }
                assert!(!slugs.is_empty(), "CascadeRouter: need at least one model");
                let mut router = Self::new(slugs);
                // Restore confidence stats.
                let mut stats = router.confidence_stats.lock();
                for (model, persisted) in &confidence_stats {
                    stats.insert(
                        model.clone(),
                        ModelStats {
                            trials: persisted.trials,
                            successes: persisted.successes,
                        },
                    );
                }
                drop(stats);

                // Restore total observation count so the cascade stage is correct.
                // If the snapshot predates the `total_observations` field (default 0),
                // recompute from the sum of per-model trials.
                let total = if total_observations > 0 {
                    total_observations
                } else {
                    confidence_stats.values().map(|s| s.trials).sum()
                };
                router.linucb.set_total_observations(total);
                if !role_table.is_empty() {
                    for (role, slug) in role_table {
                        router.role_table.insert(role, slug);
                    }
                }

                router
            }
            None => Self::new(model_slugs),
        }
    }

    // ── Internal routing per stage ──────────────────────────────────────

    fn route_static(
        &self,
        ctx: &RoutingContext,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        let slug = self
            .role_table
            .get(&ctx.role)
            .cloned()
            .unwrap_or_else(|| "claude-sonnet-4-5".to_string());

        let selected = self.bias_model_for_cfactor(ModelSpec::from_slug(&slug), cfactor, agent_id);
        let tier = slug_to_tier(&selected.slug);
        let fallback_chain = fallback_chain_for_model(&self.model_slugs, &selected.slug);
        let context_overflow_fallback =
            context_overflow_fallback_for_model(&self.model_slugs, &selected.slug);

        CascadeModel {
            primary: selected,
            fallback_chain,
            context_overflow_fallback,
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Static,
        }
    }

    fn route_static_filtered(&self, ctx: &RoutingContext, candidates: &[String]) -> CascadeModel {
        let slug = self
            .role_table
            .get(&ctx.role)
            .cloned()
            .unwrap_or_else(|| "claude-sonnet-4-5".to_string());

        let selected_slug = if candidates
            .iter()
            .any(|candidate| slugs_match(candidate, &slug))
        {
            slug
        } else {
            let tier_candidates: &[&str] = match ctx.role.model_tier() {
                ModelTier::Fast => &["claude-haiku-3-5"],
                ModelTier::Premium => &["claude-opus-4"],
                _ => &[
                    "kimi-k2.5",
                    "kimi-k2-thinking",
                    "claude-sonnet-4-6",
                    "claude-sonnet-4-5",
                ],
            };
            pick_available_static_slug(candidates, tier_candidates)
        };
        let selected = ModelSpec::from_slug(selected_slug);
        let tier = slug_to_tier(&selected.slug);
        let fallback_chain = fallback_chain_for_model(candidates, &selected.slug);
        let context_overflow_fallback =
            context_overflow_fallback_for_model(candidates, &selected.slug);

        CascadeModel {
            primary: selected,
            fallback_chain,
            context_overflow_fallback,
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Static,
        }
    }

    fn route_confidence(
        &self,
        ctx: &RoutingContext,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        let scores = self.confidence_scores(&self.model_slugs, ctx);
        let best_slug = choose_best_scored_slug(scores);

        let selected =
            self.bias_model_for_cfactor(ModelSpec::from_slug(&best_slug), cfactor, agent_id);
        let tier = slug_to_tier(&selected.slug);
        let fallback_chain = fallback_chain_for_model(&self.model_slugs, &selected.slug);
        let context_overflow_fallback =
            context_overflow_fallback_for_model(&self.model_slugs, &selected.slug);

        CascadeModel {
            primary: selected,
            fallback_chain,
            context_overflow_fallback,
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Confidence,
        }
    }

    fn route_confidence_filtered(
        &self,
        ctx: &RoutingContext,
        candidates: &[String],
    ) -> CascadeModel {
        let scores = self.confidence_scores(candidates, ctx);
        let best_slug = choose_best_scored_slug(scores);

        let selected = ModelSpec::from_slug(best_slug);
        let tier = slug_to_tier(&selected.slug);
        let fallback_chain = fallback_chain_for_model(candidates, &selected.slug);
        let context_overflow_fallback =
            context_overflow_fallback_for_model(candidates, &selected.slug);

        CascadeModel {
            primary: selected,
            fallback_chain,
            context_overflow_fallback,
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Confidence,
        }
    }

    fn route_ucb(
        &self,
        ctx: &RoutingContext,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        let model = self.select_ucb_model(ctx, &self.model_slugs);
        let selected = self.bias_model_for_cfactor(model, cfactor, agent_id);
        let tier = slug_to_tier(&selected.slug);
        let fallback_chain = fallback_chain_for_model(&self.model_slugs, &selected.slug);
        let context_overflow_fallback =
            context_overflow_fallback_for_model(&self.model_slugs, &selected.slug);

        CascadeModel {
            primary: selected,
            fallback_chain,
            context_overflow_fallback,
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Ucb,
        }
    }

    fn route_ucb_filtered(&self, ctx: &RoutingContext, candidates: &[String]) -> CascadeModel {
        let model = self.select_ucb_model(ctx, candidates);
        let selected = model;
        let tier = slug_to_tier(&selected.slug);
        let fallback_chain = fallback_chain_for_model(candidates, &selected.slug);
        let context_overflow_fallback =
            context_overflow_fallback_for_model(candidates, &selected.slug);

        CascadeModel {
            primary: selected,
            fallback_chain,
            context_overflow_fallback,
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Ucb,
        }
    }

    /// Apply a C-Factor-based bias to a selected model.
    fn bias_model_for_cfactor(
        &self,
        model: ModelSpec,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> ModelSpec {
        let Some(cfactor) = cfactor else {
            return model;
        };

        if let Some(agent_id) = agent_id {
            match cfactor.dispatch_bias_for_agent(agent_id) {
                AgentDispatchBias::PreferStronger => return self.strongest_model(),
                AgentDispatchBias::PreferCheaper => return self.cheapest_model(),
                AgentDispatchBias::Neutral => {}
            }
        }

        if cfactor.overall > HIGH_CFACTOR_THRESHOLD {
            self.cheapest_model()
        } else if cfactor.overall < LOW_CFACTOR_THRESHOLD {
            self.strongest_model()
        } else {
            model
        }
    }

    fn confidence_scores(&self, candidates: &[String], ctx: &RoutingContext) -> Vec<(String, f64)> {
        let stats = self.confidence_stats.lock();
        let low_confidence = ctx.affect_confidence < LOW_AFFECT_CONFIDENCE_THRESHOLD;

        let mut scores: Vec<(String, f64)> = candidates
            .iter()
            .map(|slug| {
                let s = stats.get(slug).cloned().unwrap_or_default();
                let tier_bonus = if low_confidence {
                    low_confidence_tier_bonus(slug_to_tier(slug))
                } else {
                    0.0
                };
                (slug.clone(), s.upper_bound() + tier_bonus)
            })
            .collect();
        drop(stats);

        apply_cache_affinity(&mut scores, ctx.previous_model.as_deref());
        scores
    }

    fn select_ucb_model(&self, ctx: &RoutingContext, candidates: &[String]) -> ModelSpec {
        self.refresh_pareto_frontier_if_needed();

        let frontier = {
            let state = self.pareto_frontier.lock();
            if state.bucket == 0 || state.frontier.is_empty() {
                None
            } else {
                Some(state.frontier.clone())
            }
        };

        if let Some(frontier) = frontier {
            let base_alpha = self.linucb.current_alpha();
            self.linucb
                .select_features_from_candidates_with_alpha_adjuster(ctx, candidates, |slug| {
                    pareto_adjusted_alpha(base_alpha, slug, &frontier)
                })
        } else if candidates.len() == self.model_slugs.len() {
            self.linucb.select_model(ctx)
        } else {
            self.linucb.select_features_from_candidates(ctx, candidates)
        }
    }

    fn refresh_pareto_frontier_if_needed(&self) {
        let total = self.total_observations();
        if total < PARETO_RECOMPUTE_INTERVAL {
            return;
        }

        let bucket = total / PARETO_RECOMPUTE_INTERVAL;
        let needs_refresh = {
            let state = self.pareto_frontier.lock();
            state.bucket < bucket || state.frontier.is_empty()
        };

        if !needs_refresh {
            return;
        }

        let frontier = self.recompute_pareto_frontier();
        let mut state = self.pareto_frontier.lock();
        if state.bucket < bucket || state.frontier.is_empty() {
            state.frontier = frontier;
            state.bucket = bucket;
        }
    }

    fn recompute_pareto_frontier(&self) -> Vec<String> {
        let stats = self.confidence_stats.lock();
        let mut observations = HashMap::new();
        let mut unobserved = Vec::new();

        for slug in &self.model_slugs {
            match stats.get(slug) {
                Some(model_stats) if model_stats.trials > 0 => {
                    observations.insert(
                        slug.clone(),
                        ModelObservation {
                            pass_rate: model_stats.pass_rate(),
                            cost_per_success: pareto_cost_proxy(slug)
                                / model_stats.pass_rate().max(0.01),
                            avg_latency_ms: pareto_latency_proxy(slug),
                            observations: model_stats.trials,
                        },
                    );
                }
                _ => unobserved.push(slug.clone()),
            }
        }
        drop(stats);

        let mut frontier = if observations.is_empty() {
            Vec::new()
        } else {
            compute_pareto_frontier(&observations)
        };
        frontier.extend(unobserved);
        frontier.sort();
        frontier.dedup();
        frontier
    }
}

fn choose_best_scored_slug(scores: Vec<(String, f64)>) -> String {
    let mut iter = scores.into_iter();
    let Some((mut best_slug, mut best_score)) = iter.next() else {
        unreachable!("CascadeRouter: confidence scoring requires at least one candidate");
    };

    for (slug, score) in iter {
        if score > best_score {
            best_score = score;
            best_slug = slug;
        }
    }

    best_slug
}

fn pareto_adjusted_alpha(base_alpha: f64, slug: &str, frontier: &[String]) -> f64 {
    if frontier.iter().any(|frontier_slug| frontier_slug == slug) {
        base_alpha
    } else {
        base_alpha * 0.1
    }
}

fn pareto_cost_proxy(slug: &str) -> f64 {
    match slug_family(slug) {
        Some("haiku") => 1.0,
        Some("sonnet") => 3.0,
        Some("opus") => 9.0,
        Some("kimi-k2") => 2.5,
        _ => match slug_to_tier(slug) {
            ModelTier::Fast => 1.0,
            ModelTier::Premium => 9.0,
            _ => 3.0,
        },
    }
}

fn pareto_latency_proxy(slug: &str) -> f64 {
    default_latency_sla(slug_to_tier(slug)) as f64
}

/// Determine the cascade stage from observation count.
const fn stage_for_observations(obs: u64) -> CascadeStage {
    if obs < COLD_START_THRESHOLD {
        CascadeStage::Static
    } else if obs < CONFIDENCE_TO_UCB_THRESHOLD {
        CascadeStage::Confidence
    } else {
        CascadeStage::Ucb
    }
}

// ─── Persistence ────────────────────────────────────────────────────────────

/// Persisted snapshot of cascade router state.
#[derive(Serialize, Deserialize)]
struct CascadeSnapshot {
    model_slugs: Vec<String>,
    confidence_stats: HashMap<String, PersistedModelStats>,
    /// Total observations across all models (used to restore cascade stage).
    ///
    /// Defaults to 0 for backward compatibility with snapshots written before
    /// this field was added; in that case `load_or_new` recomputes the total
    /// from the sum of per-model trials.
    #[serde(default)]
    total_observations: u64,
    #[serde(default)]
    role_table: HashMap<AgentRole, String>,
}

/// Serializable form of per-model confidence stats.
#[derive(Serialize, Deserialize)]
struct PersistedModelStats {
    trials: u64,
    successes: u64,
}

// ─── Tests ────────────────────────────────────────��─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_experiment::{ModelExperiment, ModelExperimentStore, ModelVariant};
    use crate::prompt_experiment::ExperimentStatus;
    use crate::provider_health::{ErrorClass, ProviderHealthRegistry};
    use roko_core::task::{TaskCategory, TaskComplexityBand};
    use std::collections::HashMap;

    fn test_slugs() -> Vec<String> {
        vec![
            "claude-haiku-3-5".to_string(),
            "claude-sonnet-4-5".to_string(),
            "claude-opus-4".to_string(),
        ]
    }

    fn default_ctx() -> RoutingContext {
        RoutingContext {
            task_category: TaskCategory::Implementation,
            complexity: TaskComplexityBand::Standard,
            iteration: 0,
            role: AgentRole::Implementer,
            crate_familiarity: 0.5,
            has_prior_failure: false,
            affect_confidence: 0.5,
            previous_model: None,
            plan_context_tokens: None,
        }
    }

    // ── Test 1: starts in Static stage ──────────────────────────────────

    #[test]
    fn starts_in_static_stage() {
        let cascade = CascadeRouter::new(test_slugs());
        assert_eq!(cascade.current_stage(), CascadeStage::Static);
    }

    // ── Test 2: static stage uses role table ────────────────────────────

    #[test]
    fn static_stage_uses_role_table() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();
        let result = cascade.route(&ctx);

        // Implementer has Standard tier -> sonnet
        assert_eq!(result.stage, CascadeStage::Static);
        assert_eq!(result.primary.slug, "claude-sonnet-4-5");
    }

    // ── Test 3: static stage gives correct fallback ─────────────────────

    #[test]
    fn static_stage_fallback_chain_for_standard() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();
        let result = cascade.route(&ctx);

        assert_eq!(result.fallback_chain.len(), 2);
        assert_eq!(result.fallback_chain[0].slug, "claude-haiku-3-5");
        assert_eq!(result.fallback_chain[1].slug, "claude-opus-4");
        assert_eq!(
            result.context_overflow_fallback.as_ref().unwrap().slug,
            "claude-opus-4"
        );
    }

    #[test]
    fn experiment_override_for_active_model_experiment() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();

        let mut store = ModelExperimentStore::default();
        store.register(ModelExperiment {
            experiment_id: "impl-model-ab".into(),
            description: "Override implementer implementation routing".into(),
            role: Some("implementer".into()),
            task_category: Some("implementation".into()),
            variants: vec![ModelVariant {
                id: "override".into(),
                model_key: "override-model".into(),
                slug: "override-model-slug".into(),
                provider: "test-provider".into(),
            }],
            stats: HashMap::new(),
            status: ExperimentStatus::Running,
            winner_id: None,
            min_trials_per_variant: 1,
            min_effect_size: 0.05,
            created_at: "2026-04-11T00:00:00Z".into(),
        });

        let routed = cascade.route_with_experiments(&ctx, &store);

        assert_eq!(routed.primary.slug, "override-model-slug");
        assert!(routed.fallback_chain.is_empty());
        assert_eq!(routed.context_overflow_fallback, None);
        assert_eq!(routed.latency_sla_ms, 30_000);
        assert_eq!(routed.stage, CascadeStage::Static);
    }

    // ── Test 4: fast tier has no fallback ────────────────────────────────

    #[test]
    fn fast_tier_no_fallback() {
        let cascade = CascadeRouter::new(test_slugs());
        let mut ctx = default_ctx();
        ctx.role = AgentRole::Conductor; // Fast tier

        let result = cascade.route(&ctx);
        assert!(result.fallback_chain.is_empty());
    }

    // ── Test 5: transitions to Confidence at 50 observations ────────────

    #[test]
    fn transitions_to_confidence_stage() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();

        // Feed 50 observations to cross the threshold.
        for _ in 0..50 {
            cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.8, true);
        }

        assert_eq!(cascade.current_stage(), CascadeStage::Confidence);
        let result = cascade.route(&ctx);
        assert_eq!(result.stage, CascadeStage::Confidence);
    }

    // ── Test 6: transitions to UCB at 200 observations ──────────────────

    #[test]
    fn transitions_to_ucb_stage() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();

        // Feed 200 observations.
        for _ in 0..200 {
            cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.8, true);
        }

        assert_eq!(cascade.current_stage(), CascadeStage::Ucb);
        let result = cascade.route(&ctx);
        assert_eq!(result.stage, CascadeStage::Ucb);
    }

    // ── Test 7: confidence stage prefers high-success model ─────────────

    #[test]
    fn confidence_stage_prefers_high_success_model() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();

        // Build up observations: sonnet mostly succeeds, haiku mostly fails.
        for i in 0..80 {
            if i < 25 {
                cascade.record_observation(&ctx, "claude-haiku-3-5", 0.2, false);
            } else if i < 50 {
                cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.9, true);
            } else if i < 65 {
                cascade.record_observation(&ctx, "claude-haiku-3-5", 0.2, false);
            } else {
                cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.9, true);
            }
        }

        assert_eq!(cascade.current_stage(), CascadeStage::Confidence);

        let result = cascade.route(&ctx);
        // Sonnet should have higher upper bound than haiku
        // (sonnet: 25/25 = 100%, haiku: 0/40 = 0%)
        assert_eq!(
            result.primary.slug, "claude-sonnet-4-5",
            "confidence stage should prefer the high-pass-rate model"
        );
    }

    // ── Test 7b: low affect confidence biases toward stronger model ──────

    #[test]
    fn low_affect_confidence_prefers_opus_over_sonnet() {
        let cascade = CascadeRouter::new(test_slugs());
        let mut ctx = default_ctx();

        for _ in 0..20 {
            cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.9, true);
        }
        for _ in 0..15 {
            cascade.record_observation(&ctx, "claude-opus-4", 0.9, true);
        }
        for _ in 0..10 {
            cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.1, false);
        }
        for _ in 0..5 {
            cascade.record_observation(&ctx, "claude-opus-4", 0.1, false);
        }

        assert_eq!(cascade.current_stage(), CascadeStage::Confidence);

        ctx.affect_confidence = 0.2;
        let low_confidence = cascade.route(&ctx);
        assert_eq!(
            low_confidence.primary.slug, "claude-opus-4",
            "low affect confidence should bias toward the stronger premium model"
        );

        ctx.affect_confidence = 0.9;
        let high_confidence = cascade.route(&ctx);
        // High confidence allows routing to cheaper models
        assert!(
            ["claude-haiku-3-5", "claude-sonnet-4-5"]
                .contains(&high_confidence.primary.slug.as_str()),
            "high confidence should allow cheaper model, got: {}",
            high_confidence.primary.slug
        );
    }

    #[test]
    fn cache_affinity_bonus() {
        let cascade = CascadeRouter::new(vec![
            "claude-sonnet-4-5".to_string(),
            "claude-sonnet-4-6".to_string(),
        ]);
        let mut ctx = default_ctx();

        for _ in 0..80 {
            cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.8, true);
        }
        for _ in 0..10 {
            cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.2, false);
        }
        for _ in 0..82 {
            cascade.record_observation(&ctx, "claude-sonnet-4-6", 0.8, true);
        }
        for _ in 0..8 {
            cascade.record_observation(&ctx, "claude-sonnet-4-6", 0.2, false);
        }

        assert_eq!(cascade.current_stage(), CascadeStage::Confidence);

        let no_affinity = cascade.route(&ctx);
        assert_eq!(no_affinity.primary.slug, "claude-sonnet-4-6");

        ctx.previous_model = Some("claude-sonnet-4-5".to_string());
        let with_affinity = cascade.route(&ctx);
        assert_eq!(with_affinity.primary.slug, "claude-sonnet-4-5");
    }

    // ── Test 7c: health-aware routing skips unhealthy providers ─────────

    #[test]
    fn cascade_health_aware_excludes_unhealthy_provider_models() {
        let cascade = CascadeRouter::new(vec![
            "claude-sonnet-4-5".to_string(),
            "claude-opus-4".to_string(),
        ]);
        let ctx = default_ctx();

        // Push the router into UCB so the candidate-aware LinUCB path is exercised.
        for _ in 0..200 {
            cascade.record_observation(&ctx, "claude-sonnet-4-5", 1.0, true);
        }
        assert_eq!(cascade.current_stage(), CascadeStage::Ucb);
        assert_eq!(cascade.route(&ctx).primary.slug, "claude-sonnet-4-5");

        let health = ProviderHealthRegistry::new();
        for _ in 0..3 {
            health.record_failure("anthropic", ErrorClass::ServerError);
        }

        let mut model_providers = HashMap::new();
        model_providers.insert("claude-sonnet-4-5".to_string(), "anthropic".to_string());
        model_providers.insert("claude-opus-4".to_string(), "openai".to_string());

        let routed = cascade.route_with_health(&ctx, &health, &model_providers);
        assert_eq!(
            routed.primary.slug, "claude-opus-4",
            "unhealthy providers should be excluded from cascade selection"
        );
    }

    // ── Test 8: stage labels are correct ────────────────────────────────

    #[test]
    fn stage_labels() {
        assert_eq!(CascadeStage::Static.label(), "static");
        assert_eq!(CascadeStage::Confidence.label(), "confidence");
        assert_eq!(CascadeStage::Ucb.label(), "ucb");
    }

    // ── Test 9: frequency routing follows the frequency policy ─────────

    #[test]
    fn frequency_routing_uses_expected_policy() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();

        assert_eq!(
            cascade.select_for_frequency(OperatingFrequency::Gamma, Some(&ctx), None, None),
            None
        );

        let theta = cascade
            .select_for_frequency(OperatingFrequency::Theta, Some(&ctx), None, None)
            .expect("theta should route");
        assert_eq!(theta.slug, "claude-sonnet-4-5");

        let delta = cascade
            .select_for_frequency(OperatingFrequency::Delta, Some(&ctx), None, None)
            .expect("delta should route");
        assert_eq!(delta.slug, "claude-opus-4");
    }

    #[test]
    fn high_cfactor_prefers_cheapest_model() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();
        let cfactor = CFactor {
            overall: 0.9,
            ..CFactor::default()
        };

        let selected = cascade
            .select_for_frequency(
                OperatingFrequency::Theta,
                Some(&ctx),
                Some(&cfactor),
                Some("Implementer"),
            )
            .expect("theta should route");

        assert_eq!(selected.slug, "claude-haiku-3-5");
    }

    #[test]
    fn low_cfactor_prefers_strongest_model() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();
        let cfactor = CFactor {
            overall: 0.2,
            ..CFactor::default()
        };

        let selected = cascade
            .select_for_frequency(
                OperatingFrequency::Theta,
                Some(&ctx),
                Some(&cfactor),
                Some("Implementer"),
            )
            .expect("theta should route");

        assert_eq!(selected.slug, "claude-opus-4");
    }

    #[test]
    fn strongest_model_falls_back_to_best_available_slug() {
        let cascade = CascadeRouter::new(vec![
            "claude-haiku-3-5".to_string(),
            "claude-sonnet-4-5".to_string(),
        ]);

        assert_eq!(cascade.strongest_model().slug, "claude-sonnet-4-5");
    }

    // ── Test 11: observation count is consistent ────────────────────────

    #[test]
    fn observation_count_tracks_correctly() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();

        assert_eq!(cascade.total_observations(), 0);

        cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.8, true);
        cascade.record_observation(&ctx, "claude-haiku-3-5", 0.3, false);
        cascade.record_observation(&ctx, "claude-opus-4", 0.9, true);

        assert_eq!(cascade.total_observations(), 3);
    }

    // ── Test 12: confidence snapshot tracks trials ──────────────────────

    #[test]
    fn confidence_snapshot_accurate() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();

        cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.8, true);
        cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.5, false);
        cascade.record_observation(&ctx, "claude-haiku-3-5", 0.9, true);

        let snap = cascade.confidence_snapshot();
        assert_eq!(snap.get("claude-sonnet-4-5"), Some(&(2, 1)));
        assert_eq!(snap.get("claude-haiku-3-5"), Some(&(1, 1)));
    }

    // ── Test 11: latency SLA varies by tier ─────────────────────────────

    #[test]
    fn latency_sla_by_tier() {
        let cascade = CascadeRouter::new(test_slugs());

        let mut ctx = default_ctx();
        ctx.role = AgentRole::Conductor; // Fast
        let fast = cascade.route(&ctx);

        ctx.role = AgentRole::Implementer; // Standard
        let standard = cascade.route(&ctx);

        ctx.role = AgentRole::Architect; // Premium
        let premium = cascade.route(&ctx);

        assert!(fast.latency_sla_ms < standard.latency_sla_ms);
        assert!(standard.latency_sla_ms < premium.latency_sla_ms);
    }

    // ── Test 12: stage_for_observations boundaries ──────────────────────

    #[test]
    fn stage_boundaries() {
        assert_eq!(stage_for_observations(0), CascadeStage::Static);
        assert_eq!(stage_for_observations(49), CascadeStage::Static);
        assert_eq!(stage_for_observations(50), CascadeStage::Confidence);
        assert_eq!(stage_for_observations(199), CascadeStage::Confidence);
        assert_eq!(stage_for_observations(200), CascadeStage::Ucb);
        assert_eq!(stage_for_observations(1000), CascadeStage::Ucb);
    }

    // ── Test 13: model_stats pass_rate computation ──────────────────────

    #[test]
    fn model_stats_pass_rate() {
        let mut s = ModelStats::default();
        assert!((s.pass_rate() - 0.0).abs() < f64::EPSILON);

        s.trials = 10;
        s.successes = 7;
        assert!((s.pass_rate() - 0.7).abs() < f64::EPSILON);
    }

    // ── Test 14: confidence width shrinks with more data ────────────────

    #[test]
    fn confidence_width_shrinks() {
        let s10 = ModelStats {
            trials: 10,
            successes: 7,
        };
        let s100 = ModelStats {
            trials: 100,
            successes: 70,
        };
        let s1000 = ModelStats {
            trials: 1000,
            successes: 700,
        };

        assert!(s10.confidence_width() > s100.confidence_width());
        assert!(s100.confidence_width() > s1000.confidence_width());
    }

    // ── Test 15: premium role uses opus in static stage ─────────────────

    #[test]
    fn premium_role_gets_opus() {
        let cascade = CascadeRouter::new(test_slugs());
        let mut ctx = default_ctx();
        ctx.role = AgentRole::Architect; // Premium tier

        let result = cascade.route(&ctx);
        assert_eq!(result.primary.slug, "claude-opus-4");
        assert_eq!(result.fallback_chain[0].slug, "claude-sonnet-4-5");
        assert_eq!(result.fallback_chain[1].slug, "claude-haiku-3-5");
        assert_eq!(result.context_overflow_fallback, None);
    }

    #[test]
    fn fallback_chain_tries_each_model_in_order() {
        let cascade = CascadeModel {
            primary: ModelSpec::from_slug("primary-model"),
            fallback_chain: vec![
                ModelSpec::from_slug("fallback-1"),
                ModelSpec::from_slug("fallback-2"),
                ModelSpec::from_slug("fallback-3"),
            ],
            context_overflow_fallback: Some(ModelSpec::from_slug("larger-context")),
            latency_sla_ms: 30_000,
            stage: CascadeStage::Static,
        };

        assert_eq!(cascade.model_for_attempt(0).unwrap().slug, "primary-model");
        assert_eq!(cascade.model_for_attempt(1).unwrap().slug, "fallback-1");
        assert_eq!(cascade.model_for_attempt(2).unwrap().slug, "fallback-2");
        assert_eq!(cascade.model_for_attempt(3).unwrap().slug, "fallback-3");
        assert!(cascade.model_for_attempt(4).is_none());
    }

    #[test]
    fn error_specific_fallback_routes_by_error_type() {
        let cascade = CascadeModel {
            primary: ModelSpec::from_slug("gpt-5"),
            fallback_chain: vec![
                ModelSpec::from_slug("glm-5.1"),
                ModelSpec::from_slug("claude-sonnet-4-5"),
                ModelSpec::from_slug("ollama/llama3"),
            ],
            context_overflow_fallback: Some(ModelSpec::from_slug("claude-opus-4")),
            latency_sla_ms: 30_000,
            stage: CascadeStage::Static,
        };

        assert_eq!(
            cascade
                .fallback_for_error(&ProviderError::ContextOverflow)
                .unwrap()
                .slug,
            "claude-opus-4"
        );
        assert_eq!(
            cascade
                .fallback_for_error(&ProviderError::RateLimit {
                    retry_after_ms: Some(1_000),
                })
                .unwrap()
                .slug,
            "claude-sonnet-4-5"
        );
        assert_eq!(
            cascade
                .fallback_for_error(&ProviderError::ServerError(503))
                .unwrap()
                .slug,
            "glm-5.1"
        );
    }

    // ── Test 16: display impl for CascadeStage ──────────────────────────

    #[test]
    fn cascade_stage_display() {
        assert_eq!(format!("{}", CascadeStage::Static), "static");
        assert_eq!(format!("{}", CascadeStage::Ucb), "ucb");
    }

    // ── Test 17: custom role table ──────────────────────────────────────

    #[test]
    fn custom_role_table() {
        let mut table = HashMap::new();
        table.insert(AgentRole::Implementer, "gpt-5".to_string());

        let cascade = CascadeRouter::new(test_slugs()).with_role_table(table);
        let ctx = default_ctx();
        let result = cascade.route(&ctx);

        assert_eq!(result.primary.slug, "gpt-5");
    }

    #[test]
    fn cascade_router_kimi_selects_kimi_in_static_stage() {
        let cascade = CascadeRouter::new(vec!["kimi-k2.5".to_string()]);
        let ctx = default_ctx();

        let result = cascade.route(&ctx);
        assert_eq!(result.stage, CascadeStage::Static);
        assert_eq!(result.primary.slug, "kimi-k2.5");
    }

    // ── Test 18: UCB stage uses linucb selection ────────────────────────

    #[test]
    fn ucb_stage_uses_trained_linucb() {
        let cascade = CascadeRouter::new(test_slugs());
        let ctx = default_ctx();

        // Train haiku as the best arm with many observations.
        for _ in 0..250 {
            cascade.record_observation(&ctx, "claude-haiku-3-5", 1.0, true);
            // Give some data to other arms too so LinUCB has seen them.
            if cascade.total_observations() % 10 == 0 {
                cascade.record_observation(&ctx, "claude-sonnet-4-5", 0.1, false);
                cascade.record_observation(&ctx, "claude-opus-4", 0.1, false);
            }
        }

        assert_eq!(cascade.current_stage(), CascadeStage::Ucb);
        let result = cascade.route(&ctx);
        // LinUCB should prefer the highly-rewarded arm
        assert_eq!(result.primary.slug, "claude-haiku-3-5");
    }

    #[test]
    fn record_outcome_updates_model_statistics() {
        let cascade = CascadeRouter::new(test_slugs());

        assert!(cascade.record_outcome("claude-sonnet-4-5", true));
        assert_eq!(cascade.total_observations(), 1);

        let stats = cascade.confidence_snapshot();
        assert_eq!(stats.get("claude-sonnet-4-5"), Some(&(1, 1)));
    }

    #[test]
    fn pareto_pruning_reduces_alpha_for_dominated_models() {
        let frontier = vec!["claude-sonnet-4-5".to_string()];
        let base_alpha = 0.8;

        assert!(
            (pareto_adjusted_alpha(base_alpha, "claude-sonnet-4-5", &frontier) - base_alpha).abs()
                < f64::EPSILON
        );
        assert!(
            (pareto_adjusted_alpha(base_alpha, "claude-haiku-3-5", &frontier) - base_alpha * 0.1)
                .abs()
                < f64::EPSILON
        );
    }

    #[test]
    fn pareto_frontier_refreshes_every_50_observations() {
        let cascade = CascadeRouter::new(vec![
            "claude-haiku-3-5".to_string(),
            "claude-sonnet-4-5".to_string(),
        ]);
        let ctx = default_ctx();

        for _ in 0..50 {
            cascade.record_observation(&ctx, "claude-sonnet-4-5", 1.0, true);
        }

        assert_eq!(cascade.pareto_frontier.lock().bucket, 1);
        let frontier = cascade.pareto_frontier.lock().frontier.clone();
        assert!(frontier.contains(&"claude-haiku-3-5".to_string()));
        assert!(frontier.contains(&"claude-sonnet-4-5".to_string()));

        for _ in 0..50 {
            cascade.record_observation(&ctx, "claude-haiku-3-5", 0.0, false);
        }

        assert_eq!(cascade.pareto_frontier.lock().bucket, 2);
        let frontier = cascade.pareto_frontier.lock().frontier.clone();
        assert!(frontier.contains(&"claude-sonnet-4-5".to_string()));
        assert!(
            !frontier.contains(&"claude-haiku-3-5".to_string()),
            "dominated models should be pruned from the frontier after refresh"
        );
    }
}
