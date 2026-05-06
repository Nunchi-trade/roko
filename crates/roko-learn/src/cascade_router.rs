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

use chrono::Utc;
use indexmap::IndexMap;
use parking_lot::Mutex;
use roko_agent::AgentResult;
use roko_core::DaimonPolicy;
use roko_core::OperatingFrequency;
use roko_core::agent::TaskRequirements;
use roko_core::agent::{AgentRole, ModelSpec, ModelTier};
use roko_core::config::schema::RewardWeights;
use roko_core::task::{TaskCategory, TaskComplexityBand};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::active_inference::{BeliefState, select_tier as select_tier_with_belief};
// Re-export public types from cascade submodules so that
// `crate::cascade_router::CascadeRouter` etc. still works for downstream crates.
pub use crate::cascade::helpers::slug_family;
pub use crate::cascade::types::{
    CascadeCandidateScore, CascadeModel, CascadeObservationStats, CascadeRouteExplanation,
    CascadeRoutingCandidate, CascadeRoutingExplanation, CascadeSelection, CascadeStage,
    GeminiContextTier, GeminiObservation, KnowledgeHint, KnowledgeRoutingAdvice,
    PerplexityObservation, RoutingBias, ShadowModelRunner, StageTransition,
};

use crate::cascade::helpers::{
    ProviderHealthSnapshotKey, ThinkingPreference, apply_cache_affinity,
    behavioral_state_tier_shift, conductor_load_tier_shift, context_overflow_fallback_for_model,
    cost_pressure_factor, default_latency_sla, default_role_model_table, estimate_total_cost_usd,
    fallback_chain_for_model, infer_shadow_routing_context, is_free_tier_gemini_model,
    low_confidence_tier_bonus, model_tier_rank, pareto_adjusted_alpha, pareto_cost_proxy,
    pareto_latency_proxy, parse_agent_role, pick_available_static_slug, pick_tier_extreme,
    routing_tier_bias_factor, select_with_hysteresis, shadow_quality_score, slug_to_tier,
    slugs_match, stage_for_observations, target_tier_rank, temperament_exploration_multiplier,
    temperament_tier_shift, thinking_filtered_candidates, thinking_preference,
};
use crate::cascade::persistence::{
    CascadeSnapshot, PersistedModelStats, detect_version_changes, migrated_confidence_stats,
    remap_role_table_entry,
};
use crate::cascade::types::{
    GeminiObservationTotals, HIGH_CFACTOR_THRESHOLD, LOW_AFFECT_CONFIDENCE_THRESHOLD,
    LOW_CFACTOR_THRESHOLD, ModelStats, OVERRIDE_LEARNING_RATE, PARETO_RECOMPUTE_INTERVAL,
    ParetoFrontierState, PerplexityObservationTotals, StageTracking,
};
use crate::cfactor::{AgentDispatchBias, CFactor};
use crate::latency::LatencyTracker;
use crate::model_experiment::ModelExperimentStore;
use crate::model_router::{
    CONTEXT_DIM, CandidateArmScore, LinUCBRouter, RoutingContext, compute_routing_reward_v2,
};
use crate::pareto::{ModelObservation, compute_pareto_frontier};
use crate::provider_health::ProviderHealthRegistry;
use crate::routing_log::{CandidateEntry, RoutingDecisionLog, RoutingDecisionMeta, RoutingLogger};

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
    role_table: Mutex<HashMap<AgentRole, String>>,
    /// Ordered list of model slugs tracked by the router.
    model_slugs: Vec<String>,
    /// Config-sourced tier map: slug → ModelTier from `roko.toml`.
    ///
    /// Used by [`Self::tier_for_slug`] to resolve tiers from config before
    /// falling back to substring heuristics. Populated from `ModelProfile.tier`
    /// fields at construction time.
    tier_map: HashMap<String, ModelTier>,
    /// Active stage and recorded stage-transition history.
    stage_tracking: Mutex<StageTracking>,
    /// Optional free-tier Gemini runner used for shadow evaluation.
    free_tier_shadow_runner: Option<Arc<dyn ShadowModelRunner>>,
}

impl roko_core::Cell for CascadeRouter {
    fn cell_id(&self) -> &str {
        "cascade-router"
    }
    fn cell_name(&self) -> &str {
        "CascadeRouter"
    }
    fn protocols(&self) -> &[&str] {
        &["Route"]
    }
}

impl Default for RoutingContext {
    fn default() -> Self {
        Self {
            task_category: TaskCategory::Implementation,
            complexity: TaskComplexityBand::Standard,
            iteration: 1,
            role: AgentRole::Implementer,
            crate_familiarity: 0.5,
            has_prior_failure: false,
            conductor_load: 0.0,
            active_agents: 0,
            ready_queue_depth: 0,
            max_queue_wait_hours: 0.0,
            daimon_policy: DaimonPolicy::default(),
            thinking_level: None,
            temperament: None,
            previous_model: None,
            plan_context_tokens: None,
            tier_thresholds: None,
        }
    }
}

impl roko_agent::model_call_service::ForceBackendOverrideRecorder for CascadeRouter {
    fn record_override_outcome(&self, model_slug: &str, success: bool) -> bool {
        CascadeRouter::record_confidence_outcome(self, model_slug, success)
    }
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
            role_table: Mutex::new(default_role_model_table(&model_slugs)),
            tier_map: HashMap::new(),
            model_slugs,
            stage_tracking: Mutex::new(StageTracking {
                current: CascadeStage::Static,
                transitions: Vec::new(),
            }),
            free_tier_shadow_runner: None,
        }
    }

    /// Set config-sourced tier assignments (builder pattern).
    ///
    /// Pass the `models` map from `RokoConfig` so the router uses explicit
    /// `tier` fields instead of substring heuristics.
    #[must_use]
    pub fn with_model_tiers(
        mut self,
        models: &IndexMap<String, roko_core::config::ModelProfile>,
    ) -> Self {
        self.tier_map = models
            .values()
            .filter_map(|p| p.tier.map(|t| (p.slug.clone(), t)))
            .collect();
        self
    }

    /// Set config-sourced tier assignments (mutable version).
    ///
    /// Use this when the router is already constructed (e.g. after
    /// `LearningRuntime::open`) and you want to inject config tiers.
    pub fn set_model_tiers(&mut self, models: &IndexMap<String, roko_core::config::ModelProfile>) {
        self.tier_map = models
            .values()
            .filter_map(|p| p.tier.map(|t| (p.slug.clone(), t)))
            .collect();
    }

    /// Resolve a model's tier, preferring config over heuristic.
    pub fn tier_for_slug(&self, slug: &str) -> ModelTier {
        slug_to_tier(slug, &self.tier_map)
    }

    /// Override the static role table (builder pattern).
    #[must_use]
    pub fn with_role_table(mut self, table: HashMap<AgentRole, String>) -> Self {
        self.role_table = Mutex::new(table);
        self
    }

    /// Update the Stage 1 static role-to-model mapping for one role.
    pub fn set_static_role_model(&mut self, role: AgentRole, model_slug: impl Into<String>) {
        self.role_table.lock().insert(role, model_slug.into());
    }

    /// Update the static role -> model table used during the cold-start stage.
    pub fn update_static_table(&self, role: AgentRole, model_slug: impl Into<String>) -> bool {
        let model_slug = model_slug.into();
        let mut role_table = self.role_table.lock();
        if role_table
            .get(&role)
            .is_some_and(|current| current == &model_slug)
        {
            return false;
        }
        role_table.insert(role, model_slug);
        true
    }

    /// Override the `LinUCB` router (builder pattern, for injecting pre-trained state).
    #[must_use]
    pub fn with_linucb(mut self, linucb: LinUCBRouter) -> Self {
        self.linucb = linucb;
        self
    }

    /// Enable free-tier Gemini shadow evaluation with the provided runner.
    #[must_use]
    pub fn with_free_tier_shadow_runner(mut self, runner: Arc<dyn ShadowModelRunner>) -> Self {
        self.free_tier_shadow_runner = Some(runner);
        self
    }

    /// Determine the current cascade stage based on total observations.
    #[must_use]
    pub fn current_stage(&self) -> CascadeStage {
        self.stage_tracking.lock().current
    }

    /// Read the ordered model slug history retained by the router.
    #[must_use]
    pub fn model_slugs(&self) -> &[String] {
        &self.model_slugs
    }

    /// Return tracked slugs paired with display availability.
    ///
    /// A slug is considered available when its wire slug appears in
    /// `configured_slugs` or it has at least one successful observation.
    /// `ModelStats` does not retain timestamps, so `successes > 0` is the
    /// best available proxy for "used successfully" without deleting history.
    #[must_use]
    pub fn model_slugs_with_availability(
        &self,
        configured_slugs: &[String],
    ) -> Vec<(String, bool)> {
        let configured: std::collections::HashSet<&str> =
            configured_slugs.iter().map(String::as_str).collect();
        let stats = self.confidence_stats.lock();

        self.model_slugs
            .iter()
            .map(|slug| {
                let available = configured.contains(slug.as_str())
                    || stats.get(slug).map_or(false, |entry| entry.successes > 0);
                (slug.clone(), available)
            })
            .collect()
    }

    /// Total observations recorded across all stages.
    #[must_use]
    pub fn total_observations(&self) -> u64 {
        self.linucb.total_observations()
    }

    /// Record a stage transition when the observation count crosses a stage boundary.
    pub fn check_stage_transition(&self) -> Option<StageTransition> {
        let obs = self.total_observations();
        let next = stage_for_observations(obs);

        let transition = {
            let mut tracking = self.stage_tracking.lock();
            if next == tracking.current {
                return None;
            }

            let transition = StageTransition {
                from: tracking.current,
                to: next,
                observations: obs,
                timestamp: Utc::now(),
            };
            tracking.current = next;
            tracking.transitions.push(transition.clone());
            transition
        };

        tracing::info!(
            from = %transition.from,
            to = %transition.to,
            observations = transition.observations,
            timestamp = %transition.timestamp.to_rfc3339(),
            "cascade router stage transition"
        );

        Some(transition)
    }

    /// Return the recorded stage-transition history.
    #[must_use]
    pub fn stage_transitions(&self) -> Vec<StageTransition> {
        self.stage_tracking.lock().transitions.clone()
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

    /// Select a model for a given operating frequency from a candidate subset.
    #[must_use]
    pub fn select_for_frequency_among(
        &self,
        frequency: OperatingFrequency,
        ctx: Option<&RoutingContext>,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
        candidates: &[String],
    ) -> Option<ModelSpec> {
        let candidates = if candidates.is_empty() {
            &self.model_slugs
        } else {
            candidates
        };

        match frequency {
            OperatingFrequency::Gamma => None,
            OperatingFrequency::Theta => ctx.map(|ctx| {
                self.route_with_cfactor_among(ctx, candidates, cfactor, agent_id)
                    .primary
            }),
            OperatingFrequency::Delta => Some(self.bias_model_for_cfactor_among(
                self.strongest_model_among(candidates),
                cfactor,
                agent_id,
                candidates,
            )),
        }
    }

    /// Select a tier using the active-inference belief state.
    #[must_use]
    pub fn select_tier_with_active_inference(
        &self,
        belief: &BeliefState,
        requirements: &TaskRequirements,
    ) -> ModelTier {
        let _ = self;
        select_tier_with_belief(belief, requirements)
    }

    /// Return the strongest model currently available to the router.
    #[must_use]
    pub fn strongest_model(&self) -> ModelSpec {
        let mut best_slug = self
            .model_slugs
            .first()
            .cloned()
            .expect("CascadeRouter: need at least one model");
        let mut best_rank = model_tier_rank(slug_to_tier(&best_slug, &self.tier_map));

        for slug in self.model_slugs.iter().skip(1) {
            let rank = model_tier_rank(slug_to_tier(slug, &self.tier_map));
            if rank > best_rank {
                best_rank = rank;
                best_slug.clone_from(slug);
            }
        }

        ModelSpec::from_slug(best_slug)
    }

    /// Return the cheapest model currently available to the router.
    #[must_use]
    pub fn cheapest_model(&self) -> ModelSpec {
        let mut best_slug = self
            .model_slugs
            .first()
            .cloned()
            .expect("CascadeRouter: need at least one model");
        let mut best_rank = model_tier_rank(slug_to_tier(&best_slug, &self.tier_map));

        for slug in self.model_slugs.iter().skip(1) {
            let rank = model_tier_rank(slug_to_tier(slug, &self.tier_map));
            if rank < best_rank {
                best_rank = rank;
                best_slug.clone_from(slug);
            }
        }

        ModelSpec::from_slug(best_slug)
    }

    /// Return the strongest model from `candidates`.
    #[must_use]
    pub fn strongest_model_among(&self, candidates: &[String]) -> ModelSpec {
        let mut best_slug = candidates
            .first()
            .cloned()
            .unwrap_or_else(|| self.strongest_model().slug);
        let mut best_rank = model_tier_rank(slug_to_tier(&best_slug, &self.tier_map));

        for slug in candidates.iter().skip(1) {
            let rank = model_tier_rank(slug_to_tier(slug, &self.tier_map));
            if rank > best_rank {
                best_rank = rank;
                best_slug.clone_from(slug);
            }
        }

        ModelSpec::from_slug(best_slug)
    }

    /// Return the cheapest model from `candidates`.
    #[must_use]
    pub fn cheapest_model_among(&self, candidates: &[String]) -> ModelSpec {
        let mut best_slug = candidates
            .first()
            .cloned()
            .unwrap_or_else(|| self.cheapest_model().slug);
        let mut best_rank = model_tier_rank(slug_to_tier(&best_slug, &self.tier_map));

        for slug in candidates.iter().skip(1) {
            let rank = model_tier_rank(slug_to_tier(slug, &self.tier_map));
            if rank < best_rank {
                best_rank = rank;
                best_slug.clone_from(slug);
            }
        }

        ModelSpec::from_slug(best_slug)
    }

    fn retarget_route_primary(
        &self,
        mut route: CascadeModel,
        candidates: &[String],
        primary: ModelSpec,
    ) -> CascadeModel {
        let tier = slug_to_tier(&primary.slug, &self.tier_map);
        route.fallback_chain = fallback_chain_for_model(candidates, &primary.slug, &self.tier_map);
        route.context_overflow_fallback =
            context_overflow_fallback_for_model(candidates, &primary.slug, &self.tier_map);
        route.latency_sla_ms = default_latency_sla(tier);
        route.primary = primary;
        route
    }

    fn bias_model_for_behavioral_state_among(
        &self,
        model: ModelSpec,
        ctx: &RoutingContext,
        candidates: &[String],
    ) -> ModelSpec {
        let shift = behavioral_state_tier_shift(ctx);
        if shift == 0 {
            return model;
        }

        let current_rank = model_tier_rank(slug_to_tier(&model.slug, &self.tier_map));
        let target_rank = target_tier_rank(current_rank, shift);
        candidates
            .iter()
            .find(|slug| model_tier_rank(slug_to_tier(slug, &self.tier_map)) == target_rank)
            .map(ModelSpec::from_slug)
            .unwrap_or(model)
    }

    fn bias_model_for_conductor_load_among(
        &self,
        model: ModelSpec,
        ctx: &RoutingContext,
        candidates: &[String],
    ) -> ModelSpec {
        let shift = conductor_load_tier_shift(ctx);
        if shift == 0 {
            return model;
        }

        let current_rank = model_tier_rank(slug_to_tier(&model.slug, &self.tier_map));
        let target_rank = target_tier_rank(current_rank, shift);
        candidates
            .iter()
            .find(|slug| model_tier_rank(slug_to_tier(slug, &self.tier_map)) == target_rank)
            .map(ModelSpec::from_slug)
            .unwrap_or(model)
    }

    fn bias_model_for_temperament_among(
        &self,
        model: ModelSpec,
        ctx: &RoutingContext,
        candidates: &[String],
    ) -> ModelSpec {
        let shift = temperament_tier_shift(ctx);
        if shift == 0 {
            return model;
        }

        let current_rank = model_tier_rank(slug_to_tier(&model.slug, &self.tier_map));
        let target_rank = target_tier_rank(current_rank, shift);
        candidates
            .iter()
            .find(|slug| model_tier_rank(slug_to_tier(slug, &self.tier_map)) == target_rank)
            .map(ModelSpec::from_slug)
            .unwrap_or(model)
    }

    fn apply_context_biases_among(
        &self,
        route: CascadeModel,
        ctx: &RoutingContext,
        candidates: &[String],
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        let primary =
            self.bias_model_for_behavioral_state_among(route.primary.clone(), ctx, candidates);
        let route = self.retarget_route_primary(route, candidates, primary);
        let primary = self.bias_model_for_temperament_among(route.primary.clone(), ctx, candidates);
        let route = self.retarget_route_primary(route, candidates, primary);
        let primary =
            self.bias_model_for_cfactor_among(route.primary.clone(), cfactor, agent_id, candidates);
        let route = self.retarget_route_primary(route, candidates, primary);
        let primary =
            self.bias_model_for_conductor_load_among(route.primary.clone(), ctx, candidates);
        self.retarget_route_primary(route, candidates, primary)
    }

    fn apply_knowledge_to_route(
        &self,
        ctx: &RoutingContext,
        route: CascadeModel,
        candidates: &[String],
        knowledge: Option<&KnowledgeRoutingAdvice>,
    ) -> CascadeModel {
        let Some(knowledge) = knowledge else {
            return route;
        };
        if !knowledge.has_signal {
            return route;
        }

        let _primary_hint = knowledge.hint_for(&route.primary.slug);
        let frontier = self.current_pareto_frontier();
        let linucb_scores: HashMap<_, _> = self
            .ucb_scores(ctx, candidates, frontier.as_deref())
            .into_iter()
            .collect();

        let adjusted_score = |slug: &str| {
            let base = linucb_scores
                .get(slug)
                .copied()
                .filter(|score| score.is_finite())
                .unwrap_or(0.0);
            base + knowledge.score_for(slug)
        };

        let current_score = adjusted_score(&route.primary.slug);
        let best = candidates
            .iter()
            .map(|slug| (slug, adjusted_score(slug)))
            .max_by(|(left_slug, left_score), (right_slug, right_score)| {
                left_score
                    .total_cmp(right_score)
                    .then_with(|| right_slug.cmp(left_slug))
            });

        let mut swapped = false;
        let route = if let Some((best_slug, best_score)) = best {
            if !slugs_match(best_slug, &route.primary.slug) && best_score - current_score > 0.1 {
                swapped = true;
                self.retarget_route_primary(route, candidates, ModelSpec::from_slug(best_slug))
            } else {
                route
            }
        } else {
            route
        };

        tracing::debug!(
            primary = %route.primary.slug,
            knowledge_hints = ?knowledge.hints.len(),
            swapped = %swapped,
            "knowledge-aware routing applied"
        );

        route
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

    /// Route a context through the cascade and append a routing decision log.
    pub fn route_logged(
        &self,
        ctx: &RoutingContext,
        log: &RoutingLogger,
        meta: &RoutingDecisionMeta,
    ) -> std::io::Result<(CascadeModel, RoutingDecisionLog)> {
        let selected = self.route(ctx);
        let explanation = self.explain_route(ctx, None);
        let record =
            self.append_routing_log(log, meta, &selected.primary.slug, Some(&explanation))?;
        Ok((selected, record))
    }

    /// Route a context, overriding selection when a model experiment is active.
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

    /// Route a context, excluding models whose provider is currently unavailable.
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

    /// Remove candidates whose provider is currently unhealthy.
    #[must_use]
    pub fn filter_unhealthy(
        &self,
        candidates: &[String],
        health: &ProviderHealthRegistry,
        model_providers: &HashMap<String, String>,
    ) -> Vec<String> {
        if candidates.is_empty() {
            return Vec::new();
        }

        let available: Vec<String> = candidates
            .iter()
            .filter(|slug| {
                let provider = model_providers
                    .get(slug.as_str())
                    .map_or(slug.as_str(), String::as_str);
                health.is_healthy(provider)
            })
            .cloned()
            .collect();
        if !available.is_empty() {
            return available;
        }

        let snapshot = health.snapshot();
        let mut ranked: Vec<(String, ProviderHealthSnapshotKey)> = candidates
            .iter()
            .map(|slug| {
                let provider = model_providers
                    .get(slug.as_str())
                    .map_or(slug.as_str(), String::as_str);
                let health_record = snapshot
                    .get(provider)
                    .cloned()
                    .unwrap_or_else(|| health.get(provider));
                (
                    slug.clone(),
                    ProviderHealthSnapshotKey::from(&health_record),
                )
            })
            .collect();

        ranked.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
        ranked
            .first()
            .map(|(slug, _)| vec![slug.clone()])
            .unwrap_or_default()
    }

    /// Apply a routing bias to scored candidates.
    pub fn apply_bias(&self, candidates: &mut [(String, f64)], bias: &RoutingBias) {
        if bias.deprioritize.is_empty() && !bias.prefer_cheaper {
            return;
        }

        for (slug, score) in candidates.iter_mut() {
            if bias
                .deprioritize
                .iter()
                .any(|blocked| slugs_match(slug, blocked))
            {
                *score *= 0.5;
            }

            if bias.prefer_cheaper {
                *score *= routing_tier_bias_factor(slug_to_tier(slug, &self.tier_map));
            }
        }
    }

    /// Apply cost pressure to scored candidates.
    pub fn apply_cost_pressure(&self, candidates: &mut [(String, f64)], spike: bool) {
        if !spike {
            return;
        }

        for (slug, score) in candidates.iter_mut() {
            *score *= cost_pressure_factor(slug_to_tier(slug, &self.tier_map));
        }
    }

    /// Route a context, applying conductor routing bias directly.
    pub fn route_with_bias(&self, ctx: &RoutingContext, bias: &RoutingBias) -> CascadeModel {
        let candidates: Vec<String> = self
            .model_slugs
            .iter()
            .filter(|slug| !bias.deprioritize.iter().any(|d| slugs_match(slug, d)))
            .cloned()
            .collect();

        if candidates.is_empty() {
            return self.route(ctx);
        }

        self.route_with_cfactor_among(ctx, &candidates, None, None)
    }

    /// Load static routing overrides from a JSON map of role labels to model slugs.
    pub fn load_static_overrides(&self, path: &Path) -> Result<usize, crate::error::LearnError> {
        let contents = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(err) => {
                return Err(crate::error::LearnError::Io {
                    path: path.display().to_string(),
                    source: err,
                });
            }
        };
        let overrides =
            serde_json::from_str::<HashMap<String, String>>(&contents).map_err(|err| {
                crate::error::LearnError::Corrupt {
                    path: path.display().to_string(),
                    reason: err.to_string(),
                }
            })?;

        let mut applied = 0usize;
        for (parameter, winning_value) in overrides {
            if let Some(role) = parse_agent_role(&parameter) {
                if self.update_static_table(role, winning_value) {
                    applied += 1;
                }
            }
        }
        Ok(applied)
    }

    /// Compute a latency penalty using actual wall-clock latency.
    #[must_use]
    pub fn latency_penalty(actual_ms: f64, expected_ms: f64) -> f64 {
        if expected_ms <= 0.0 {
            return 0.0;
        }
        0.1 * (actual_ms / expected_ms - 1.0).max(0.0)
    }

    /// Convert a quality score into a latency-adjusted reward.
    #[must_use]
    pub fn reward_with_latency(
        quality_score: f64,
        actual_ms: Option<f64>,
        expected_ms: f64,
    ) -> f64 {
        let penalty = actual_ms
            .map(|actual_ms| Self::latency_penalty(actual_ms, expected_ms))
            .unwrap_or(0.0);
        (quality_score - penalty).max(0.0)
    }

    /// Convert a tracker-backed latency observation into a reward signal.
    #[must_use]
    pub fn reward_with_tracker_latency(
        &self,
        quality_score: f64,
        model: &str,
        tracker: &LatencyTracker,
        expected_ms: f64,
    ) -> f64 {
        let actual_ms = tracker
            .mean_latency(model)
            .or_else(|| tracker.p95_latency(model));
        Self::reward_with_latency(quality_score, actual_ms, expected_ms)
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

    /// Route a context through the cascade, optionally biasing by C-Factor and knowledge hints.
    pub fn route_with_knowledge(
        &self,
        ctx: &RoutingContext,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
        knowledge: Option<&KnowledgeRoutingAdvice>,
    ) -> CascadeModel {
        let route = self.route_with_cfactor(ctx, cfactor, agent_id);
        self.apply_knowledge_to_route(ctx, route, &self.model_slugs, knowledge)
    }

    /// Route a context through the cascade over a candidate subset.
    pub fn route_with_cfactor_among(
        &self,
        ctx: &RoutingContext,
        candidates: &[String],
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        let candidates = if candidates.is_empty() {
            &self.model_slugs
        } else {
            candidates
        };

        match self.current_stage() {
            CascadeStage::Static => self.route_static_among(ctx, candidates, cfactor, agent_id),
            CascadeStage::Confidence => {
                self.route_confidence_among(ctx, candidates, cfactor, agent_id)
            }
            CascadeStage::Ucb => self.route_ucb_among(ctx, candidates, cfactor, agent_id),
        }
    }

    /// Route a context through the cascade over a candidate subset with knowledge hints.
    pub fn route_with_knowledge_among(
        &self,
        ctx: &RoutingContext,
        candidates: &[String],
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
        knowledge: Option<&KnowledgeRoutingAdvice>,
    ) -> CascadeModel {
        let route = self.route_with_cfactor_among(ctx, candidates, cfactor, agent_id);
        let candidates = if candidates.is_empty() {
            &self.model_slugs
        } else {
            candidates
        };
        self.apply_knowledge_to_route(ctx, route, candidates, knowledge)
    }

    /// Explain a routing decision over the supplied candidate set.
    #[must_use]
    pub fn explain_routing(
        &self,
        ctx: &RoutingContext,
        candidates: &[String],
    ) -> CascadeRoutingExplanation {
        let candidates = if candidates.is_empty() {
            self.model_slugs.clone()
        } else {
            candidates.to_vec()
        };

        let route = match self.current_stage() {
            CascadeStage::Static => self.route_static_filtered(ctx, &candidates),
            CascadeStage::Confidence => self.route_confidence_filtered(ctx, &candidates),
            CascadeStage::Ucb => self.route_ucb_filtered(ctx, &candidates),
        };

        let frontier = self.current_pareto_frontier();
        let scores = self.stage_scores(ctx, &candidates, frontier.as_deref());
        let score_map: HashMap<_, _> = scores.into_iter().collect();

        let mut explained: Vec<_> = candidates
            .into_iter()
            .map(|slug| CascadeRoutingCandidate {
                cache_affinity: ctx.previous_model.as_deref() == Some(slug.as_str()),
                pareto_optimal: frontier
                    .as_ref()
                    .map(|frontier| frontier.iter().any(|frontier_slug| frontier_slug == &slug)),
                score: score_map.get(&slug).copied().unwrap_or(0.0),
                selected: slugs_match(&slug, &route.primary.slug),
                model: slug,
            })
            .collect();

        explained.sort_by(|a, b| {
            b.selected
                .cmp(&a.selected)
                .then_with(|| b.score.total_cmp(&a.score))
                .then_with(|| a.model.cmp(&b.model))
        });

        CascadeRoutingExplanation {
            stage: route.stage,
            selected_model: route.primary.slug,
            fallback_model: route.fallback_chain.first().map(|model| model.slug.clone()),
            latency_sla_ms: route.latency_sla_ms,
            candidates: explained,
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
        self.observe_internal(&ctx.to_features(), model_idx, reward, success, None, None);
    }

    /// Apply a WAL-replayed observation. Does NOT write a WAL entry.
    ///
    /// Validates that `model_idx` still maps to `model_slug`. If the config
    /// has changed since the WAL was written (e.g. model list reordered),
    /// the entry is silently skipped with a warning.
    pub fn replay_observation(
        &self,
        model_slug: &str,
        context_features: &[f64],
        model_idx: usize,
        reward: f64,
        success: bool,
    ) {
        // Validate the slug/index pair is still valid after potential config changes.
        let current_idx = self.model_index_for_slug(model_slug);
        let effective_idx = match current_idx {
            Some(idx) => idx,
            None => {
                tracing::warn!(
                    slug = %model_slug,
                    model_idx,
                    "[wal] replay: unknown model slug -- skipping observation"
                );
                return;
            }
        };
        if effective_idx != model_idx {
            tracing::warn!(
                slug = %model_slug,
                wal_idx = model_idx,
                current_idx = effective_idx,
                "[wal] replay: model index changed -- using current index"
            );
        }
        self.observe_internal(context_features, effective_idx, reward, success, None, None);
    }

    /// Record an observation enriched with Perplexity search metadata.
    pub fn record_perplexity_observation(
        &self,
        ctx: &RoutingContext,
        model_slug: &str,
        reward: f64,
        success: bool,
        observation: PerplexityObservation,
    ) -> bool {
        let Some(model_idx) = self.model_index_for_slug(model_slug) else {
            return false;
        };

        let perplexity = PerplexityObservationTotals {
            citation_count: observation.citation_count,
            search_latency_ms: observation.search_latency_ms,
            total_cost_usd: estimate_total_cost_usd(
                model_slug,
                observation.input_tokens,
                observation.output_tokens,
            ),
        };
        self.observe_internal(
            &ctx.to_features(),
            model_idx,
            reward,
            success,
            Some(perplexity),
            None,
        );
        true
    }

    /// Record an observation enriched with Gemini-native metadata.
    pub fn record_gemini_observation(
        &self,
        ctx: &RoutingContext,
        model_slug: &str,
        reward: f64,
        success: bool,
        observation: GeminiObservation,
    ) -> bool {
        let Some(model_idx) = self.model_index_for_slug(model_slug) else {
            return false;
        };

        let gemini = GeminiObservationTotals {
            thinking_tokens: observation.thinking_tokens.unwrap_or(0),
            cached_tokens: observation.cached_tokens.unwrap_or(0),
            grounding_query_count: observation.grounding_query_count,
            code_execution_success_count: observation.code_execution_success_count,
            code_execution_failure_count: observation.code_execution_failure_count,
            context_tier: observation.context_tier,
        };
        self.observe_internal(
            &ctx.to_features(),
            model_idx,
            reward,
            success,
            None,
            Some(gemini),
        );
        true
    }

    /// Record a binary confidence-only outcome for `model_slug`.
    ///
    /// This path intentionally does not update the contextual `LinUCB` bandit
    /// because the caller has not supplied a real [`RoutingContext`].
    pub fn record_confidence_outcome(&self, model_slug: &str, success: bool) -> bool {
        let Some(model_idx) = self.model_index_for_slug(model_slug) else {
            tracing::warn!(
                slug = %model_slug,
                success,
                "cascade router: unknown model slug -- outcome dropped. \
                 Add this model to [models] in roko.toml or provider config \
                 so the router can track it."
            );
            return false;
        };

        let Some(slug) = self.model_slugs.get(model_idx) else {
            return false;
        };

        let mut stats = self.confidence_stats.lock();
        let entry = stats.entry(slug.clone()).or_default();
        entry.trials += 1;
        if success {
            entry.successes += 1;
        }
        true
    }

    /// Apply a calibration correction to adjust confidence stats for a model.
    ///
    /// When the `CalibrationPolicy` detects systematic bias (overconfidence or
    /// underconfidence), it produces a `CalibrationCorrection`. This method
    /// adjusts the model's empirical pass rate by injecting synthetic
    /// observations that counteract the detected bias.
    ///
    /// For an overconfident model (positive `mean_bias`): inject virtual failures
    /// to lower the empirical pass rate.
    /// For an underconfident model (negative `mean_bias`): inject virtual successes
    /// to raise the empirical pass rate.
    ///
    /// The number of synthetic observations is proportional to both the magnitude
    /// of the correction and the existing sample count, capped to avoid
    /// overwhelming real data.
    pub fn apply_calibration_correction(
        &self,
        correction: &crate::calibration_policy::CalibrationCorrection,
    ) {
        let mut stats = self.confidence_stats.lock();
        let entry = stats.entry(correction.model.clone()).or_default();

        // Determine how many synthetic trials to inject. We use a fraction of
        // the existing trial count scaled by the correction magnitude. Cap at
        // 10% of existing trials to avoid overwhelming real observations.
        let existing_trials = entry.trials.max(1) as f64;
        let magnitude = correction.mean_bias.abs().clamp(0.0, 1.0);
        let synthetic_count = (existing_trials * magnitude * 0.1).ceil() as u64;

        if synthetic_count == 0 {
            return;
        }

        if correction.mean_bias > 0.0 {
            // Overconfident: inject synthetic failures (add trials but not successes).
            entry.trials += synthetic_count;
        } else {
            // Underconfident: inject synthetic successes (add both trials and successes).
            entry.trials += synthetic_count;
            entry.successes += synthetic_count;
        }

        tracing::debug!(
            model = %correction.model,
            bias = correction.mean_bias,
            synthetic_trials = synthetic_count,
            new_pass_rate = entry.pass_rate(),
            "calibration correction applied to confidence stats"
        );
    }

    /// Deprecated alias for callers that have not migrated to the
    /// confidence-only API name yet.
    ///
    /// Follow-up packet: migrate remaining non-`roko-learn` callers to
    /// `record_confidence_outcome` and remove this wrapper.
    #[deprecated(
        since = "0.1.0",
        note = "use record_confidence_outcome; this path does not update contextual LinUCB observations"
    )]
    pub fn record_outcome(&self, model_slug: &str, success: bool) -> bool {
        self.record_confidence_outcome(model_slug, success)
    }

    /// Record a force_backend override outcome for learning (UX34).
    pub fn record_override_outcome(
        &self,
        model_slug: &str,
        ctx: &RoutingContext,
        success: bool,
    ) -> bool {
        let Some(model_idx) = self.model_index_for_slug(model_slug) else {
            return false;
        };
        let raw_reward = if success { 1.0 } else { 0.0 };
        let dampened_reward = raw_reward * OVERRIDE_LEARNING_RATE;
        self.observe_internal(
            &ctx.to_features(),
            model_idx,
            dampened_reward,
            success,
            None,
            None,
        );
        true
    }

    /// Run a shadow evaluation against a free-tier Gemini model.
    pub async fn shadow_evaluate(
        &mut self,
        prompt: &str,
        primary_result: &AgentResult,
        free_model: &str,
    ) {
        if !is_free_tier_gemini_model(free_model) {
            return;
        }

        let Some(model_idx) = self.model_index_for_slug(free_model) else {
            return;
        };
        let Some(runner) = self.free_tier_shadow_runner.clone() else {
            return;
        };

        let prompt = prompt.trim();
        if prompt.is_empty() {
            return;
        }

        let shadow_result = runner.run_shadow(prompt, free_model).await;
        let quality = shadow_quality_score(prompt, primary_result, &shadow_result);
        let passed = quality >= 0.65;
        let ctx = infer_shadow_routing_context(prompt, primary_result);
        let reward = if passed {
            compute_routing_reward_v2(
                quality,
                0.0,
                shadow_result.usage.wall_ms as f64,
                default_latency_sla(slug_to_tier(free_model, &self.tier_map)) as f64,
            )
        } else {
            0.0
        };

        self.observe_internal(
            &ctx.to_features_for_model(Some(free_model)),
            model_idx,
            reward,
            passed,
            None,
            None,
        );
    }

    /// Record a successful observation from a raw 18-dim context vector.
    pub fn observe(&self, context_vec: Vec<f64>, model_idx: usize, reward: f64) {
        self.observe_internal(&context_vec, model_idx, reward, true, None, None);
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

    fn observe_internal(
        &self,
        context_vec: &[f64],
        model_idx: usize,
        reward: f64,
        success: bool,
        perplexity: Option<PerplexityObservationTotals>,
        gemini: Option<GeminiObservationTotals>,
    ) {
        let Some(slug) = self.model_slugs.get(model_idx) else {
            return;
        };

        // Phase 1: Update confidence stats (single lock, dropped before next).
        {
            let mut stats = self.confidence_stats.lock();
            let entry = stats.entry(slug.clone()).or_default();
            entry.trials += 1;
            if success {
                entry.successes += 1;
            }
            if let Some(perplexity) = perplexity {
                entry.total_citations += perplexity.citation_count;
                entry.total_search_latency_ms += perplexity.search_latency_ms;
                entry.total_cost_usd += perplexity.total_cost_usd;
                entry.perplexity_requests += 1;
            }
            if let Some(gemini) = gemini {
                entry.total_gemini_thinking_tokens += gemini.thinking_tokens;
                entry.total_gemini_cached_tokens += gemini.cached_tokens;
                entry.total_gemini_grounding_queries += gemini.grounding_query_count;
                entry.gemini_code_execution_successes += gemini.code_execution_success_count;
                entry.gemini_code_execution_failures += gemini.code_execution_failure_count;
                entry.gemini_requests += 1;
                match gemini.context_tier {
                    GeminiContextTier::UpTo200k => {
                        entry.gemini_context_window_le_200k_requests += 1;
                    }
                    GeminiContextTier::Over200k => {
                        entry.gemini_context_window_gt_200k_requests += 1;
                    }
                }
            }
        } // stats lock dropped

        // Phase 2: Update LinUCB (internal lock, not nested with ours).
        self.linucb.update_features(context_vec, model_idx, reward);

        // Refresh Pareto frontier if the observation count crossed a bucket boundary.
        self.refresh_pareto_frontier_if_needed();

        // Phase 3: Check stage transition (single lock, dropped before log).
        let obs = self.linucb.total_observations();
        let next = stage_for_observations(obs);
        let transition = {
            let mut stage_tracking = self.stage_tracking.lock();
            if next != stage_tracking.current {
                let t = StageTransition {
                    from: stage_tracking.current,
                    to: next,
                    observations: obs,
                    timestamp: Utc::now(),
                };
                stage_tracking.current = next;
                stage_tracking.transitions.push(t.clone());
                Some(t)
            } else {
                None
            }
        }; // stage_tracking lock dropped

        if let Some(transition) = transition {
            tracing::info!(
                from = %transition.from,
                to = %transition.to,
                observations = transition.observations,
                timestamp = %transition.timestamp.to_rfc3339(),
                "cascade router stage transition"
            );
        }
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

    /// Explain the current routing decision for `ctx`.
    pub fn explain_route(
        &self,
        ctx: &RoutingContext,
        candidates: Option<&[String]>,
    ) -> CascadeRouteExplanation {
        let candidates = candidates
            .filter(|candidates| !candidates.is_empty())
            .unwrap_or(&self.model_slugs);
        let stage = self.current_stage();
        let observations = self.total_observations();
        let pareto_frontier = self.recompute_pareto_frontier();

        match stage {
            CascadeStage::Static => {
                let selected = if std::ptr::eq(candidates, self.model_slugs.as_slice()) {
                    self.route_static(ctx, None, None).primary.slug
                } else {
                    self.route_static_filtered(ctx, candidates).primary.slug
                };
                let mut scored = candidates
                    .iter()
                    .map(|slug| CascadeCandidateScore {
                        slug: slug.clone(),
                        score: if slugs_match(slug, &selected) {
                            1.0
                        } else {
                            0.0
                        },
                        selected: slugs_match(slug, &selected),
                        on_pareto_frontier: pareto_frontier.iter().any(|entry| entry == slug),
                        exploitation: None,
                        exploration: None,
                    })
                    .collect::<Vec<_>>();
                scored.sort_by(|a, b| {
                    b.score
                        .total_cmp(&a.score)
                        .then_with(|| a.slug.cmp(&b.slug))
                });

                CascadeRouteExplanation {
                    stage,
                    observations,
                    alpha: None,
                    selected_slug: selected,
                    candidates: scored,
                    pareto_frontier,
                    knowledge_advice: None,
                }
            }
            CascadeStage::Confidence => {
                let scores = self.confidence_scores(candidates, ctx);
                let selected = scores
                    .iter()
                    .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
                    .map(|(slug, _)| slug.clone())
                    .unwrap_or_else(|| candidates[0].clone());
                let mut scored_candidates = scores
                    .into_iter()
                    .map(|(slug, score)| CascadeCandidateScore {
                        selected: slugs_match(&slug, &selected),
                        on_pareto_frontier: pareto_frontier.iter().any(|entry| entry == &slug),
                        slug,
                        score,
                        exploitation: None,
                        exploration: None,
                    })
                    .collect::<Vec<_>>();
                scored_candidates.sort_by(|a, b| {
                    b.score
                        .total_cmp(&a.score)
                        .then_with(|| a.slug.cmp(&b.slug))
                });

                CascadeRouteExplanation {
                    stage,
                    observations,
                    alpha: None,
                    selected_slug: selected,
                    candidates: scored_candidates,
                    pareto_frontier,
                    knowledge_advice: None,
                }
            }
            CascadeStage::Ucb => {
                self.refresh_pareto_frontier_if_needed();
                let frontier = {
                    let frontier_state = self.pareto_frontier.lock();
                    if frontier_state.bucket == 0 || frontier_state.frontier.is_empty() {
                        pareto_frontier.clone()
                    } else {
                        frontier_state.frontier.clone()
                    }
                };
                let base_alpha = self.linucb.current_alpha();
                let arm_scores: Vec<CandidateArmScore> = self
                    .linucb
                    .score_candidates_with_alpha_adjuster(ctx, candidates, |slug| {
                        pareto_adjusted_alpha(base_alpha, slug, &frontier)
                    });

                let selected = arm_scores
                    .iter()
                    .max_by(|left, right| {
                        left.score
                            .total_cmp(&right.score)
                            .then_with(|| right.slug.cmp(&left.slug))
                    })
                    .map(|score| score.slug.clone())
                    .unwrap_or_else(|| candidates[0].clone());

                let mut scored = arm_scores
                    .into_iter()
                    .map(|candidate| CascadeCandidateScore {
                        selected: slugs_match(&candidate.slug, &selected),
                        on_pareto_frontier: frontier.iter().any(|entry| entry == &candidate.slug),
                        slug: candidate.slug,
                        score: candidate.score,
                        exploitation: Some(candidate.exploitation),
                        exploration: Some(candidate.exploration),
                    })
                    .collect::<Vec<_>>();
                scored.sort_by(|a, b| {
                    b.score
                        .total_cmp(&a.score)
                        .then_with(|| a.slug.cmp(&b.slug))
                });

                CascadeRouteExplanation {
                    stage,
                    observations,
                    alpha: Some(base_alpha),
                    selected_slug: selected,
                    candidates: scored,
                    pareto_frontier: frontier,
                    knowledge_advice: None,
                }
            }
        }
    }

    /// Apply knowledge-informed routing advice to a routing explanation.
    pub fn apply_knowledge_advice(
        &self,
        explanation: &mut CascadeRouteExplanation,
        advice: KnowledgeRoutingAdvice,
    ) {
        if !advice.has_signal || advice.hints.is_empty() {
            explanation.knowledge_advice = Some(advice);
            return;
        }
        for candidate in &mut explanation.candidates {
            if let Some(hint) = advice.hint_for(&candidate.slug) {
                let adjustment = hint.score.clamp(-0.20, 0.20);
                candidate.score += adjustment;
            }
        }
        explanation.candidates.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.slug.cmp(&b.slug))
        });
        if let Some(new_top) = explanation.candidates.first() {
            if !slugs_match(&new_top.slug, &explanation.selected_slug) {
                tracing::info!(
                    previous = %explanation.selected_slug,
                    new = %new_top.slug,
                    "knowledge advice changed routing selection"
                );
            }
            let new_top_slug = new_top.slug.clone();
            for c in &mut explanation.candidates {
                c.selected = slugs_match(&c.slug, &new_top_slug);
            }
            explanation.selected_slug = new_top_slug;
        }
        explanation.knowledge_advice = Some(advice);
    }

    /// Append a routing decision log entry for a selected model.
    pub fn append_routing_log(
        &self,
        log: &RoutingLogger,
        meta: &RoutingDecisionMeta,
        selected_model: &str,
        explanation: Option<&CascadeRouteExplanation>,
    ) -> std::io::Result<RoutingDecisionLog> {
        let mut candidates = explanation
            .map(|explanation| {
                explanation
                    .candidates
                    .iter()
                    .map(|candidate| CandidateEntry {
                        model: candidate.slug.clone(),
                        provider: log.provider_for_model(&candidate.slug),
                        score: candidate.score,
                        disqualified: log.disqualified_reason(&candidate.slug),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if !candidates
            .iter()
            .any(|candidate| candidate.model == selected_model)
        {
            candidates.insert(
                0,
                CandidateEntry {
                    model: selected_model.to_string(),
                    provider: log.provider_for_model(selected_model),
                    score: 1.0,
                    disqualified: log.disqualified_reason(selected_model),
                },
            );
        }
        if candidates.is_empty() {
            candidates.push(CandidateEntry {
                model: selected_model.to_string(),
                provider: log.provider_for_model(selected_model),
                score: 1.0,
                disqualified: log.disqualified_reason(selected_model),
            });
        }

        let record = RoutingDecisionLog {
            timestamp: chrono::Utc::now().to_rfc3339(),
            trace_id: meta.trace_id.clone(),
            task_id: meta.task_id.clone(),
            requested_model: meta.requested_model.clone(),
            role: meta.role.clone(),
            task_complexity: meta.task_complexity.clone(),
            task_category: meta.task_category.clone(),
            selected_provider: log.provider_for_model(selected_model),
            selected_model: selected_model.to_string(),
            routing_stage: meta.routing_stage.clone(),
            routing_reason: meta.routing_reason.clone(),
            candidates,
            outcome_success: None,
            outcome_cost_usd: None,
            outcome_latency_ms: None,
        };
        log.append(&record)?;
        Ok(record)
    }

    /// Snapshot of richer per-model observations used by learning loops.
    pub fn observation_snapshot(&self) -> HashMap<String, CascadeObservationStats> {
        self.confidence_stats
            .lock()
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    CascadeObservationStats {
                        trials: v.trials,
                        successes: v.successes,
                        total_citations: v.total_citations,
                        avg_citations_per_response: v.avg_citations_per_response(),
                        total_search_latency_ms: v.total_search_latency_ms,
                        avg_search_latency_ms: v.avg_search_latency_ms(),
                        total_cost_usd: v.total_cost_usd,
                        avg_cost_usd: v.avg_cost_usd(),
                        perplexity_requests: v.perplexity_requests,
                        total_gemini_thinking_tokens: v.total_gemini_thinking_tokens,
                        avg_gemini_thinking_tokens_per_response: v
                            .avg_gemini_thinking_tokens_per_response(),
                        total_gemini_cached_tokens: v.total_gemini_cached_tokens,
                        avg_gemini_cached_tokens_per_response: v
                            .avg_gemini_cached_tokens_per_response(),
                        total_gemini_grounding_queries: v.total_gemini_grounding_queries,
                        avg_gemini_grounding_queries_per_response: v
                            .avg_gemini_grounding_queries_per_response(),
                        gemini_code_execution_successes: v.gemini_code_execution_successes,
                        gemini_code_execution_failures: v.gemini_code_execution_failures,
                        gemini_code_execution_success_rate: v.gemini_code_execution_success_rate(),
                        gemini_requests: v.gemini_requests,
                        gemini_context_window_le_200k_requests: v
                            .gemini_context_window_le_200k_requests,
                        gemini_context_window_gt_200k_requests: v
                            .gemini_context_window_gt_200k_requests,
                    },
                )
            })
            .collect()
    }

    /// Build a JSON snapshot of the current router state (same format as `save()`).
    pub fn snapshot_json(&self) -> String {
        let stage_transitions = self.stage_tracking.lock().transitions.clone();
        let snapshot = CascadeSnapshot {
            model_slugs: self.model_slugs.clone(),
            role_table: self.role_table.lock().clone(),
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
                            total_citations: v.total_citations,
                            total_search_latency_ms: v.total_search_latency_ms,
                            total_cost_usd: v.total_cost_usd,
                            perplexity_requests: v.perplexity_requests,
                            total_gemini_thinking_tokens: v.total_gemini_thinking_tokens,
                            total_gemini_cached_tokens: v.total_gemini_cached_tokens,
                            total_gemini_grounding_queries: v.total_gemini_grounding_queries,
                            gemini_code_execution_successes: v.gemini_code_execution_successes,
                            gemini_code_execution_failures: v.gemini_code_execution_failures,
                            gemini_context_window_le_200k_requests: v
                                .gemini_context_window_le_200k_requests,
                            gemini_context_window_gt_200k_requests: v
                                .gemini_context_window_gt_200k_requests,
                            gemini_requests: v.gemini_requests,
                        },
                    )
                })
                .collect(),
            total_observations: self.linucb.total_observations(),
            stage_transitions,
            // LinUCB export methods don't exist yet; populate None as
            // forward-compatible placeholder. Wire actual export when
            // LinUCBRouter exposes A/b matrices.
            linucb_state: None,
        };
        tracing::debug!(
            total_observations = snapshot.total_observations,
            linucb_persisted = snapshot.linucb_state.is_some(),
            "cascade router snapshot built"
        );
        serde_json::to_string_pretty(&snapshot).unwrap_or_default()
    }

    /// Save confidence stats, model slugs, and total observation count to a JSON file.
    pub fn save(&self, path: &Path) -> Result<(), crate::error::LearnError> {
        let json = self.snapshot_json();
        if json.is_empty() {
            return Err(crate::error::LearnError::Io {
                path: path.display().to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "failed to serialize cascade snapshot",
                ),
            });
        }
        roko_core::io::atomic_write_str(path, &json).map_err(|source| {
            crate::error::LearnError::Io {
                path: path.display().to_string(),
                source,
            }
        })
    }

    fn from_snapshot(snapshot: CascadeSnapshot, model_slugs: Vec<String>) -> Self {
        let CascadeSnapshot {
            model_slugs: persisted_model_slugs,
            confidence_stats,
            total_observations,
            role_table,
            stage_transitions,
            linucb_state: _linucb_state,
        } = snapshot;

        let slugs = if model_slugs.is_empty() {
            persisted_model_slugs.clone()
        } else {
            model_slugs
        };
        assert!(!slugs.is_empty(), "CascadeRouter: need at least one model");

        let version_changes = detect_version_changes(&persisted_model_slugs, &slugs);
        let migrated_stats = migrated_confidence_stats(&confidence_stats, &version_changes, &slugs);
        let router = Self::new(slugs);

        // Restore confidence stats.
        let mut stats = router.confidence_stats.lock();
        for (model, persisted) in &migrated_stats {
            stats.insert(
                model.clone(),
                ModelStats {
                    trials: persisted.trials,
                    successes: persisted.successes,
                    total_citations: persisted.total_citations,
                    total_search_latency_ms: persisted.total_search_latency_ms,
                    total_cost_usd: persisted.total_cost_usd,
                    perplexity_requests: persisted.perplexity_requests,
                    total_gemini_thinking_tokens: persisted.total_gemini_thinking_tokens,
                    total_gemini_cached_tokens: persisted.total_gemini_cached_tokens,
                    total_gemini_grounding_queries: persisted.total_gemini_grounding_queries,
                    gemini_code_execution_successes: persisted.gemini_code_execution_successes,
                    gemini_code_execution_failures: persisted.gemini_code_execution_failures,
                    gemini_context_window_le_200k_requests: persisted
                        .gemini_context_window_le_200k_requests,
                    gemini_context_window_gt_200k_requests: persisted
                        .gemini_context_window_gt_200k_requests,
                    gemini_requests: persisted.gemini_requests,
                },
            );
        }
        drop(stats);

        let total = if total_observations > 0 {
            total_observations
        } else {
            confidence_stats.values().map(|s| s.trials).sum()
        };
        router.linucb.set_total_observations(total);
        if !role_table.is_empty() {
            let mut rt = router.role_table.lock();
            for (role, slug) in role_table {
                rt.insert(role, remap_role_table_entry(slug, &version_changes));
            }
        }
        {
            let mut stage_tracking = router.stage_tracking.lock();
            stage_tracking.current = stage_for_observations(total);
            stage_tracking.transitions = stage_transitions;
        }

        router
    }

    /// Build a router from an in-memory snapshot JSON string.
    pub fn from_snapshot_json(
        json: &str,
        model_slugs: Vec<String>,
    ) -> Result<Self, crate::error::LearnError> {
        let snapshot: CascadeSnapshot = serde_json::from_str(json)?;
        Ok(Self::from_snapshot(snapshot, model_slugs))
    }

    /// Load a cascade router from a persisted JSON file, or create a new one.
    ///
    /// When the file exists but cannot be parsed, the corrupted file is
    /// backed up to `<path>.corrupted` and a warning is emitted before
    /// creating a fresh router.
    pub fn load_or_new(path: &Path, model_slugs: Vec<String>) -> Self {
        let raw = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => return Self::new(model_slugs),
        };

        match serde_json::from_str::<CascadeSnapshot>(&raw) {
            Ok(snapshot) => Self::from_snapshot(snapshot, model_slugs),
            Err(err) => {
                let backup = path.with_extension("json.corrupted");
                tracing::warn!(
                    path = %path.display(),
                    backup = %backup.display(),
                    %err,
                    "cascade router state corrupted — backing up and resetting"
                );
                if let Err(backup_err) = std::fs::copy(path, &backup) {
                    tracing::error!(
                        %backup_err,
                        "failed to backup corrupted cascade router file"
                    );
                }
                Self::new(model_slugs)
            }
        }
    }

    // ── Test-visible accessors for Pareto frontier ──────────────────

    /// Return the current Pareto frontier bucket (for tests).
    #[cfg(test)]
    pub(crate) fn pareto_frontier_bucket(&self) -> u64 {
        self.pareto_frontier.lock().bucket
    }

    /// Return the current Pareto frontier slugs (for tests).
    #[cfg(test)]
    pub(crate) fn pareto_frontier_slugs(&self) -> Vec<String> {
        self.pareto_frontier.lock().frontier.clone()
    }

    // ── Internal routing per stage ──────────────────────────────────────

    fn route_static(
        &self,
        ctx: &RoutingContext,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        if let Some(thinking_selected) = match thinking_preference(ctx) {
            ThinkingPreference::PreferThinking => pick_tier_extreme(
                &thinking_filtered_candidates(&self.model_slugs, ctx),
                true,
                &self.tier_map,
            ),
            ThinkingPreference::PreferNonThinking => pick_tier_extreme(
                &thinking_filtered_candidates(&self.model_slugs, ctx),
                false,
                &self.tier_map,
            ),
            ThinkingPreference::Neutral => None,
        } {
            let tier = slug_to_tier(&thinking_selected, &self.tier_map);
            let route = CascadeModel {
                primary: ModelSpec::from_slug(thinking_selected),
                fallback_chain: Vec::new(),
                context_overflow_fallback: None,
                latency_sla_ms: default_latency_sla(tier),
                stage: CascadeStage::Static,
            };
            return self.apply_context_biases_among(
                route,
                ctx,
                &self.model_slugs,
                cfactor,
                agent_id,
            );
        }

        let default_slug = self
            .model_slugs
            .first()
            .cloned()
            .unwrap_or_else(|| "claude-sonnet-4-5".to_string());
        let slug = if ctx.task_category == TaskCategory::Research {
            self.model_slugs
                .iter()
                .find(|s| s.as_str() == "sonar-pro" || s.as_str() == "sonar")
                .cloned()
                .unwrap_or_else(|| {
                    self.role_table
                        .lock()
                        .get(&ctx.role)
                        .cloned()
                        .unwrap_or_else(|| default_slug.clone())
                })
        } else {
            self.role_table
                .lock()
                .get(&ctx.role)
                .cloned()
                .unwrap_or(default_slug)
        };
        let tier = slug_to_tier(&slug, &self.tier_map);

        let route = CascadeModel {
            primary: ModelSpec::from_slug(&slug),
            fallback_chain: fallback_chain_for_model(&self.model_slugs, &slug, &self.tier_map),
            context_overflow_fallback: context_overflow_fallback_for_model(
                &self.model_slugs,
                &slug,
                &self.tier_map,
            ),
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Static,
        };
        self.apply_context_biases_among(route, ctx, &self.model_slugs, cfactor, agent_id)
    }

    fn route_static_filtered(&self, ctx: &RoutingContext, candidates: &[String]) -> CascadeModel {
        if let Some(thinking_selected) = match thinking_preference(ctx) {
            ThinkingPreference::PreferThinking => pick_tier_extreme(
                &thinking_filtered_candidates(candidates, ctx),
                true,
                &self.tier_map,
            ),
            ThinkingPreference::PreferNonThinking => pick_tier_extreme(
                &thinking_filtered_candidates(candidates, ctx),
                false,
                &self.tier_map,
            ),
            ThinkingPreference::Neutral => None,
        } {
            let selected = ModelSpec::from_slug(thinking_selected);
            let tier = slug_to_tier(&selected.slug, &self.tier_map);

            return CascadeModel {
                primary: selected,
                fallback_chain: Vec::new(),
                context_overflow_fallback: None,
                latency_sla_ms: default_latency_sla(tier),
                stage: CascadeStage::Static,
            };
        }

        let slug = self
            .role_table
            .lock()
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
                ModelTier::Fast => &["gemini-2.5-flash-lite", "claude-haiku-4-5"],
                ModelTier::Premium => &[
                    "claude-opus-4-6",
                    "gemini-3.1-pro-preview",
                    "gemini-2.5-pro",
                ],
                _ => &[
                    "gemini-2.5-flash",
                    "gemini-2.5-pro",
                    "kimi-k2.5",
                    "kimi-k2-thinking",
                    "claude-sonnet-4-6",
                    "claude-sonnet-4-5",
                ],
            };
            pick_available_static_slug(candidates, tier_candidates)
        };
        let selected = ModelSpec::from_slug(selected_slug);
        let tier = slug_to_tier(&selected.slug, &self.tier_map);
        let fallback_chain = fallback_chain_for_model(candidates, &selected.slug, &self.tier_map);
        let context_overflow_fallback =
            context_overflow_fallback_for_model(candidates, &selected.slug, &self.tier_map);

        CascadeModel {
            primary: selected,
            fallback_chain,
            context_overflow_fallback,
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Static,
        }
    }

    fn route_static_among(
        &self,
        ctx: &RoutingContext,
        candidates: &[String],
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        let route = self.route_static_filtered(ctx, candidates);
        self.apply_context_biases_among(route, ctx, candidates, cfactor, agent_id)
    }

    fn route_confidence(
        &self,
        ctx: &RoutingContext,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        let thinking_candidates = thinking_filtered_candidates(&self.model_slugs, ctx);
        let scores = self.confidence_scores(&thinking_candidates, ctx);
        let best_slug = select_with_hysteresis(&scores, ctx.previous_model.as_deref());
        let tier = slug_to_tier(&best_slug, &self.tier_map);

        let route = CascadeModel {
            primary: ModelSpec::from_slug(&best_slug),
            fallback_chain: fallback_chain_for_model(&self.model_slugs, &best_slug, &self.tier_map),
            context_overflow_fallback: context_overflow_fallback_for_model(
                &self.model_slugs,
                &best_slug,
                &self.tier_map,
            ),
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Confidence,
        };
        self.apply_context_biases_among(route, ctx, &self.model_slugs, cfactor, agent_id)
    }

    fn route_confidence_filtered(
        &self,
        ctx: &RoutingContext,
        candidates: &[String],
    ) -> CascadeModel {
        let thinking_candidates = thinking_filtered_candidates(candidates, ctx);
        let scores = self.confidence_scores(&thinking_candidates, ctx);
        let best_slug = select_with_hysteresis(&scores, ctx.previous_model.as_deref());

        let selected = ModelSpec::from_slug(best_slug);
        let tier = slug_to_tier(&selected.slug, &self.tier_map);
        let fallback_chain = fallback_chain_for_model(candidates, &selected.slug, &self.tier_map);
        let context_overflow_fallback =
            context_overflow_fallback_for_model(candidates, &selected.slug, &self.tier_map);

        CascadeModel {
            primary: selected,
            fallback_chain,
            context_overflow_fallback,
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Confidence,
        }
    }

    fn route_confidence_among(
        &self,
        ctx: &RoutingContext,
        candidates: &[String],
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        let route = self.route_confidence_filtered(ctx, candidates);
        self.apply_context_biases_among(route, ctx, candidates, cfactor, agent_id)
    }

    fn route_ucb(
        &self,
        ctx: &RoutingContext,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        let thinking_candidates = thinking_filtered_candidates(&self.model_slugs, ctx);
        let model = self.select_ucb_model(ctx, &thinking_candidates);
        let tier = slug_to_tier(&model.slug, &self.tier_map);
        let route = CascadeModel {
            primary: model.clone(),
            fallback_chain: fallback_chain_for_model(
                &self.model_slugs,
                &model.slug,
                &self.tier_map,
            ),
            context_overflow_fallback: context_overflow_fallback_for_model(
                &self.model_slugs,
                &model.slug,
                &self.tier_map,
            ),
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Ucb,
        };
        self.apply_context_biases_among(route, ctx, &self.model_slugs, cfactor, agent_id)
    }

    fn route_ucb_filtered(&self, ctx: &RoutingContext, candidates: &[String]) -> CascadeModel {
        let thinking_candidates = thinking_filtered_candidates(candidates, ctx);
        let model = self.select_ucb_model(ctx, &thinking_candidates);
        let selected = model;
        let tier = slug_to_tier(&selected.slug, &self.tier_map);
        let fallback_chain = fallback_chain_for_model(candidates, &selected.slug, &self.tier_map);
        let context_overflow_fallback =
            context_overflow_fallback_for_model(candidates, &selected.slug, &self.tier_map);

        CascadeModel {
            primary: selected,
            fallback_chain,
            context_overflow_fallback,
            latency_sla_ms: default_latency_sla(tier),
            stage: CascadeStage::Ucb,
        }
    }

    fn route_ucb_among(
        &self,
        ctx: &RoutingContext,
        candidates: &[String],
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
    ) -> CascadeModel {
        let route = self.route_ucb_filtered(ctx, candidates);
        self.apply_context_biases_among(route, ctx, candidates, cfactor, agent_id)
    }

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

    fn bias_model_for_cfactor_among(
        &self,
        model: ModelSpec,
        cfactor: Option<&CFactor>,
        agent_id: Option<&str>,
        candidates: &[String],
    ) -> ModelSpec {
        let Some(cfactor) = cfactor else {
            return model;
        };

        if let Some(agent_id) = agent_id {
            match cfactor.dispatch_bias_for_agent(agent_id) {
                AgentDispatchBias::PreferStronger => return self.strongest_model_among(candidates),
                AgentDispatchBias::PreferCheaper => return self.cheapest_model_among(candidates),
                AgentDispatchBias::Neutral => {}
            }
        }

        if cfactor.overall > HIGH_CFACTOR_THRESHOLD {
            self.cheapest_model_among(candidates)
        } else if cfactor.overall < LOW_CFACTOR_THRESHOLD {
            self.strongest_model_among(candidates)
        } else {
            model
        }
    }

    fn confidence_scores(&self, candidates: &[String], ctx: &RoutingContext) -> Vec<(String, f64)> {
        let stats = self.confidence_stats.lock();

        let prediction_error = 1.0 - ctx.daimon_policy.affect_confidence.clamp(0.0, 1.0);
        let low_confidence = ctx
            .tier_thresholds
            .as_ref()
            .map(|th| prediction_error > th.t0_ceiling)
            .unwrap_or(ctx.daimon_policy.affect_confidence < LOW_AFFECT_CONFIDENCE_THRESHOLD);

        let mut scores: Vec<(String, f64)> = candidates
            .iter()
            .map(|slug| {
                let s = stats.get(slug).cloned().unwrap_or_default();
                let base_score = if s.trials == 0 { 0.5 } else { s.upper_bound() };
                let tier_bonus = if low_confidence {
                    low_confidence_tier_bonus(slug_to_tier(slug, &self.tier_map))
                } else {
                    0.0
                };
                (slug.clone(), base_score + tier_bonus)
            })
            .collect();
        drop(stats);

        apply_cache_affinity(&mut scores, ctx.previous_model.as_deref());
        scores
    }

    fn select_ucb_model(&self, ctx: &RoutingContext, candidates: &[String]) -> ModelSpec {
        let frontier = self.current_pareto_frontier();
        let scores = self.ucb_scores(ctx, candidates, frontier.as_deref());
        let best_slug = select_with_hysteresis(&scores, ctx.previous_model.as_deref());
        ModelSpec::from_slug(best_slug)
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

    fn current_pareto_frontier(&self) -> Option<Vec<String>> {
        self.refresh_pareto_frontier_if_needed();
        let state = self.pareto_frontier.lock();
        if state.bucket == 0 || state.frontier.is_empty() {
            None
        } else {
            Some(state.frontier.clone())
        }
    }

    fn stage_scores(
        &self,
        ctx: &RoutingContext,
        candidates: &[String],
        frontier: Option<&[String]>,
    ) -> Vec<(String, f64)> {
        match self.current_stage() {
            CascadeStage::Static => {
                let selected = self.route_static_filtered(ctx, candidates).primary.slug;
                candidates
                    .iter()
                    .map(|slug| {
                        (
                            slug.clone(),
                            if slugs_match(slug, &selected) {
                                1.0
                            } else {
                                0.0
                            },
                        )
                    })
                    .collect()
            }
            CascadeStage::Confidence => {
                let thinking_candidates = thinking_filtered_candidates(candidates, ctx);
                let score_map: HashMap<_, _> = self
                    .confidence_scores(&thinking_candidates, ctx)
                    .into_iter()
                    .collect();
                candidates
                    .iter()
                    .map(|slug| (slug.clone(), score_map.get(slug).copied().unwrap_or(0.0)))
                    .collect()
            }
            CascadeStage::Ucb => {
                let thinking_candidates = thinking_filtered_candidates(candidates, ctx);
                let score_map: HashMap<_, _> = self
                    .ucb_scores(ctx, &thinking_candidates, frontier)
                    .into_iter()
                    .collect();
                candidates
                    .iter()
                    .map(|slug| (slug.clone(), score_map.get(slug).copied().unwrap_or(0.0)))
                    .collect()
            }
        }
    }

    fn ucb_scores(
        &self,
        ctx: &RoutingContext,
        candidates: &[String],
        frontier: Option<&[String]>,
    ) -> Vec<(String, f64)> {
        let base_alpha = self.linucb.current_alpha() * temperament_exploration_multiplier(ctx);

        self.linucb
            .score_features_from_candidates_with_alpha_adjuster(ctx, candidates, |slug| {
                frontier.map_or(base_alpha, |frontier| {
                    pareto_adjusted_alpha(base_alpha, slug, frontier)
                })
            })
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
                            cost_per_success: model_stats.cost_per_success().unwrap_or_else(|| {
                                pareto_cost_proxy(slug, &self.tier_map)
                                    / model_stats.pass_rate().max(0.01)
                            }),
                            avg_latency_ms: if model_stats.perplexity_requests > 0 {
                                model_stats.avg_search_latency_ms()
                            } else {
                                pareto_latency_proxy(slug, &self.tier_map)
                            },
                            reliability: if model_stats.trials > 0 {
                                model_stats.pass_rate().max(0.5)
                            } else {
                                0.5
                            },
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

    /// Feed prediction residuals back into the router after task completion.
    pub fn feedback(
        &self,
        model_slug: &str,
        _predicted_success: f64,
        actual_success: bool,
        residual: f64,
    ) {
        let Some(model_idx) = self.model_index_for_slug(model_slug) else {
            return;
        };

        let calibration_bonus = 1.0 - residual.abs().min(1.0);
        let base_reward = if actual_success { 1.0 } else { 0.0 };
        let adjusted_reward = (base_reward * 0.7 + calibration_bonus * 0.3).clamp(0.0, 1.0);

        let context_vec = vec![0.0; CONTEXT_DIM];
        self.observe_internal(
            &context_vec,
            model_idx,
            adjusted_reward,
            actual_success,
            None,
            None,
        );

        self.check_stage_transition();
    }

    /// Feed prediction residuals from a calibration tracker summary.
    pub fn feedback_from_prediction(
        &self,
        model_slug: &str,
        predicted_success: f64,
        actual_success: bool,
    ) {
        let actual_value = if actual_success { 1.0 } else { 0.0 };
        let residual = predicted_success - actual_value;
        self.feedback(model_slug, predicted_success, actual_success, residual);
    }
}

#[cfg(test)]
mod cascade_router_tests {
    use super::*;

    #[test]
    fn replay_observation_increments_confidence_stats() {
        let router =
            CascadeRouter::new(vec!["claude-sonnet-4-5".into(), "claude-haiku-4-5".into()]);
        let context_features = vec![0.0; crate::model_router::CONTEXT_DIM];

        // Replay 10 observations for the first model.
        for _ in 0..10 {
            router.replay_observation("claude-sonnet-4-5", &context_features, 0, 0.85, true);
        }

        let stats = router.confidence_stats.lock();
        let entry = stats.get("claude-sonnet-4-5").expect("stats should exist");
        assert_eq!(entry.trials, 10);
        assert_eq!(entry.successes, 10);
    }

    #[test]
    fn replay_observation_skips_unknown_slug() {
        let router = CascadeRouter::new(vec!["claude-sonnet-4-5".into()]);
        let context_features = vec![0.0; crate::model_router::CONTEXT_DIM];

        // This should log a warning and not panic.
        router.replay_observation("nonexistent-model", &context_features, 0, 1.0, true);

        let stats = router.confidence_stats.lock();
        assert!(stats.get("nonexistent-model").is_none());
    }

    #[test]
    fn replay_observation_uses_current_index_on_mismatch() {
        let router =
            CascadeRouter::new(vec!["claude-haiku-4-5".into(), "claude-sonnet-4-5".into()]);
        let context_features = vec![0.0; crate::model_router::CONTEXT_DIM];

        // WAL says model_idx=0 but the slug maps to current index 1.
        // replay_observation should use the current index (1) not the stale WAL index.
        router.replay_observation(
            "claude-sonnet-4-5",
            &context_features,
            0, // stale WAL index
            0.9,
            true,
        );

        let stats = router.confidence_stats.lock();
        let entry = stats.get("claude-sonnet-4-5").expect("stats should exist");
        assert_eq!(entry.trials, 1);
        assert_eq!(entry.successes, 1);
    }
}
