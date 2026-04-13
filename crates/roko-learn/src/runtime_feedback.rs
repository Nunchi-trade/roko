//! Runtime-facing learning orchestration helpers.
//!
//! This module provides a single integration point for CLI/orchestrator code:
//! pass one completed run, and the helper updates all configured learning
//! subsystems in a consistent order.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use thiserror::Error;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex as AsyncMutex;

use crate::cfactor::{CFactor, compute_cfactor};
use roko_core::metric::TaskMetric;

use crate::cascade_router::CascadeRouter;
use crate::context_pack_cache::ContextPackCache;
use crate::costs_db::{CostRecord, CostsDb};
use crate::costs_log::CostsLog;
use crate::efficiency::AgentEfficiencyEvent;
use crate::episode_logger::{Episode, EpisodeLogger, LoggerError};
use crate::latency::LatencyRegistry;
use crate::local_reward::LocalRewardFunction;
use crate::model_router::{RoutingContext, compute_routing_reward_v2};
use crate::pattern_discovery::{
    CrossEpisodeConsolidationReport, CrossEpisodeConsolidator, EpisodeView, PatternMiner,
};
use crate::playbook::PlaybookStore;
use crate::playbook_rules::PlaybookRules;
use crate::prompt_experiment::{ExperimentStatus, ExperimentStore, PromptExperiment};
use crate::provider_health::ProviderHealthTracker;
use crate::regression::{RegressionReport, RegressionThresholds, detect_regressions};
use crate::skill_library::{SkillLibrary, SkillLibraryError, TemplatePatternGenerator};
use roko_core::ConductorDecision;
use roko_core::agent::AgentRole;
use roko_core::task::{TaskCategory, TaskComplexityBand};
use roko_daimon::{AffectEngine as _, AffectEvent, DaimonState, queue_wait_arousal};

type EpisodeCompletionHook = Arc<dyn Fn(Episode) + Send + Sync>;

/// Filesystem locations used by [`LearningRuntime`].
/// Thin wrapper that materializes the action slice required by [`EpisodeView`]
/// from an [`Episode`]'s gate verdicts.
struct EpisodeActions {
    actions: Vec<String>,
    success: bool,
}

impl EpisodeActions {
    fn from_episode(ep: &Episode) -> Self {
        Self {
            actions: ep.gate_verdicts.iter().map(|v| v.gate.clone()).collect(),
            success: ep.success,
        }
    }
}

fn affect_state_path(learn_root: &Path) -> PathBuf {
    let root = learn_root.parent().unwrap_or(learn_root);
    root.join("daimon").join("affect.json")
}

impl EpisodeView for EpisodeActions {
    fn actions(&self) -> &[String] {
        &self.actions
    }
    fn succeeded(&self) -> bool {
        self.success
    }
}

/// Well-known paths used by the learning runtime for persistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearningPaths {
    /// Root directory for runtime-managed learning artifacts.
    pub root: PathBuf,
    /// Append-only episode log.
    pub episodes_jsonl: PathBuf,
    /// Append-only cost log.
    pub costs_jsonl: PathBuf,
    /// JSON map of extracted skills.
    pub skills_json: PathBuf,
    /// Playbook JSON directory.
    pub playbooks_dir: PathBuf,
    /// TOML rules file for playbook rule confidence tracking.
    pub playbook_rules_toml: PathBuf,
    /// Append-only `TaskMetric` JSONL file used for regression checks.
    pub task_metrics_jsonl: PathBuf,
    /// Append-only efficiency events JSONL file.
    pub efficiency_jsonl: PathBuf,
    /// Persisted latency registry snapshot.
    pub latency_stats_json: PathBuf,
    /// Append-only C-Factor history JSONL file.
    pub cfactor_jsonl: PathBuf,
    /// Cascade router persisted observations JSON.
    pub cascade_router_json: PathBuf,
    /// Prompt experiment store JSON.
    pub experiments_json: PathBuf,
    /// Adaptive gate thresholds JSON.
    pub gate_thresholds_json: PathBuf,
    /// Per-subsystem local reward functions JSON.
    pub local_rewards_json: PathBuf,
}

impl LearningPaths {
    /// Build the default path layout under `root`.
    #[must_use]
    pub fn under(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            episodes_jsonl: root.join("episodes.jsonl"),
            costs_jsonl: root.join("costs.jsonl"),
            skills_json: root.join("skills.json"),
            playbooks_dir: root.join("playbooks"),
            playbook_rules_toml: root.join("playbook-rules.toml"),
            task_metrics_jsonl: root.join("task-metrics.jsonl"),
            efficiency_jsonl: root.join("efficiency.jsonl"),
            latency_stats_json: root.join("latency-stats.json"),
            cfactor_jsonl: root.join("c-factor.jsonl"),
            cascade_router_json: root.join("cascade-router.json"),
            experiments_json: root.join("experiments.json"),
            gate_thresholds_json: root.join("gate-thresholds.json"),
            local_rewards_json: root.join("local-rewards.json"),
            root,
        }
    }
}

/// Optional knobs for regression detection in [`LearningRuntime`].
#[derive(Debug, Clone)]
pub struct RegressionConfig {
    /// Thresholds used by [`detect_regressions`].
    pub thresholds: RegressionThresholds,
    /// Number of latest metrics used as the "current" sample.
    pub current_window: usize,
}

impl Default for RegressionConfig {
    fn default() -> Self {
        Self {
            thresholds: RegressionThresholds::default(),
            current_window: 20,
        }
    }
}

/// Cadence controls for learning subsystems that should not all react on the
/// same episode boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateFrequency {
    /// Cascade router observation cadence.
    pub router_every_n_episodes: u32,
    /// Reserved for orchestrator-managed adaptive gate threshold batching.
    pub gate_thresholds_every_n: u32,
    /// Prompt experiment outcome cadence.
    pub experiments_every_n: u32,
    /// Skill extraction cadence.
    pub skill_mining_every_n: u32,
    /// Pattern miner ingestion cadence.
    pub pattern_discovery_every_n: u32,
    /// Cross-episode consolidation cadence.
    pub distiller_every_n: u32,
}

impl UpdateFrequency {
    fn due(episode_count: u64, every_n: u32) -> bool {
        let cadence = u64::from(every_n.max(1));
        episode_count % cadence == 0
    }

    fn router_due(self, episode_count: u64) -> bool {
        Self::due(episode_count, self.router_every_n_episodes)
    }

    fn experiments_due(self, episode_count: u64) -> bool {
        Self::due(episode_count, self.experiments_every_n)
    }

    fn skill_mining_due(self, episode_count: u64) -> bool {
        Self::due(episode_count, self.skill_mining_every_n)
    }

    fn pattern_discovery_due(self, episode_count: u64) -> bool {
        Self::due(episode_count, self.pattern_discovery_every_n)
    }

    fn distiller_due(self, episode_count: u64) -> bool {
        Self::due(episode_count, self.distiller_every_n)
    }
}

impl Default for UpdateFrequency {
    fn default() -> Self {
        Self {
            router_every_n_episodes: 1,
            gate_thresholds_every_n: 5,
            experiments_every_n: 1,
            skill_mining_every_n: 10,
            pattern_discovery_every_n: 20,
            distiller_every_n: 50,
        }
    }
}

/// Input payload for one completed runtime run.
#[derive(Debug, Clone)]
pub struct CompletedRunInput {
    /// Canonical episode for this run.
    pub episode: Episode,
    /// Optional explicit cost record.
    pub cost_record: Option<CostRecord>,
    /// Optional provider name when no explicit cost record is supplied.
    pub provider: Option<String>,
    /// Optional playbook id to update outcome counters.
    pub playbook_id: Option<String>,
    /// Optional playbook rule id to update confidence.
    pub playbook_rule_id: Option<String>,
    /// Optional skill id injected into prompt; updates validation counters.
    pub matched_skill_id: Option<String>,
    /// Optional metric for regression history.
    pub task_metric: Option<TaskMetric>,
    /// Optional prompt experiment variant id for A/B outcome recording.
    pub experiment_variant_id: Option<String>,
}

impl CompletedRunInput {
    /// Construct an input from an episode.
    #[must_use]
    pub const fn from_episode(episode: Episode) -> Self {
        Self {
            episode,
            cost_record: None,
            provider: None,
            playbook_id: None,
            playbook_rule_id: None,
            matched_skill_id: None,
            task_metric: None,
            experiment_variant_id: None,
        }
    }

    /// Attach an explicit cost record.
    #[must_use]
    pub fn with_cost_record(mut self, record: CostRecord) -> Self {
        self.cost_record = Some(record);
        self
    }

    /// Attach a task metric to update regression history.
    #[must_use]
    pub fn with_task_metric(mut self, metric: TaskMetric) -> Self {
        self.task_metric = Some(metric);
        self
    }
}

/// Status of a specific learning side effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApplyStatus {
    /// The subsystem was not updated for this run.
    #[default]
    Skipped,
    /// The subsystem was updated.
    Applied,
}

/// Summary of side effects produced by [`LearningRuntime::record_completed_run`].
#[derive(Debug, Clone, Default)]
pub struct LearningUpdate {
    /// Whether the episode was persisted.
    pub episode_logged: ApplyStatus,
    /// Whether a cost record was persisted.
    pub cost_logged: ApplyStatus,
    /// Whether provider health state was updated.
    pub provider_updated: ApplyStatus,
    /// Whether a playbook outcome was updated.
    pub playbook_updated: ApplyStatus,
    /// Whether a playbook rule outcome was updated.
    pub playbook_rule_updated: ApplyStatus,
    /// Newly extracted skill id, if extraction succeeded.
    pub extracted_skill_id: Option<String>,
    /// Whether an existing matched skill outcome was recorded.
    pub matched_skill_updated: ApplyStatus,
    /// Regression report when a task metric was provided and sufficient data exists.
    pub regression_report: Option<RegressionReport>,
    /// Whether pattern mining ingested this episode.
    pub patterns_ingested: bool,
    /// Whether the cascade router was updated with an observation.
    pub router_updated: bool,
}

/// Errors produced by [`LearningRuntime`].
#[derive(Debug, Error)]
pub enum LearningRuntimeError {
    /// Filesystem errors.
    #[error("learning runtime io error: {0}")]
    Io(#[from] io::Error),
    /// Episode logger errors.
    #[error("learning runtime episode error: {0}")]
    Episode(#[from] LoggerError),
    /// Skill library errors.
    #[error("learning runtime skill error: {0}")]
    Skill(#[from] SkillLibraryError),
    /// JSON serialization/parsing errors.
    #[error("learning runtime serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Runtime orchestrator for `roko-learn` subsystems.
pub struct LearningRuntime {
    paths: LearningPaths,
    episode_logger: EpisodeLogger,
    update_frequency: UpdateFrequency,
    episode_count: AtomicU64,
    affect_engine: parking_lot::Mutex<DaimonState>,
    costs_log: CostsLog,
    costs_db: CostsDb,
    provider_health: ProviderHealthTracker,
    skill_library: SkillLibrary,
    playbook_store: PlaybookStore,
    playbook_rules: PlaybookRules,
    regression: RegressionConfig,
    task_metrics: AsyncMutex<Vec<TaskMetric>>,
    pattern_miner: parking_lot::Mutex<PatternMiner>,
    latency_registry: LatencyRegistry,
    cascade_router: CascadeRouter,
    context_pack_cache: ContextPackCache,
    experiment_store: parking_lot::Mutex<ExperimentStore>,
    local_rewards: parking_lot::Mutex<HashMap<String, LocalRewardFunction>>,
    episode_completion_hook: Option<EpisodeCompletionHook>,
}

impl LearningRuntime {
    /// Open a runtime at `paths` and preload persisted state.
    ///
    /// # Errors
    ///
    /// Returns an error if persistence files cannot be read/initialized.
    pub async fn open(
        paths: LearningPaths,
        regression: RegressionConfig,
    ) -> Result<Self, LearningRuntimeError> {
        tokio::fs::create_dir_all(&paths.root).await?;
        tokio::fs::create_dir_all(&paths.playbooks_dir).await?;
        let affect_path = affect_state_path(&paths.root);
        if let Some(parent) = affect_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let episode_logger = EpisodeLogger::new(&paths.episodes_jsonl);
        let costs_log = CostsLog::open_creating(&paths.costs_jsonl).await?;
        let costs_db = CostsDb::new();
        let existing_costs = costs_log.read_all().await?;
        costs_db.insert_batch(existing_costs);
        let episode_count = count_episode_records(&paths.episodes_jsonl).await?;

        let skill_library = SkillLibrary::new(&paths.skills_json).await?;
        let playbook_store = PlaybookStore::new(&paths.playbooks_dir);
        let playbook_rules = PlaybookRules::open(&paths.playbook_rules_toml)?;
        let task_metrics = load_task_metrics(&paths.task_metrics_jsonl).await?;

        let pattern_miner = parking_lot::Mutex::new(PatternMiner::new(3, 0.5));
        let latency_registry = LatencyRegistry::load_or_new(&paths.latency_stats_json);
        let cascade_router = CascadeRouter::load_or_new(
            &paths.cascade_router_json,
            vec!["claude-sonnet-4-5".into(), "claude-haiku-4-5".into()],
        );
        let context_pack_cache = ContextPackCache::new(256, paths.root.join("context-cache.json"));
        let experiment_store = ExperimentStore::load_or_new(&paths.experiments_json);
        let local_rewards = load_local_rewards(&paths.local_rewards_json);

        Ok(Self {
            paths,
            episode_logger,
            update_frequency: UpdateFrequency::default(),
            episode_count: AtomicU64::new(episode_count),
            affect_engine: parking_lot::Mutex::new(DaimonState::load_or_new(&affect_path)),
            costs_log,
            costs_db,
            provider_health: ProviderHealthTracker::new(),
            skill_library,
            playbook_store,
            playbook_rules,
            regression,
            task_metrics: AsyncMutex::new(task_metrics),
            pattern_miner,
            latency_registry,
            cascade_router,
            context_pack_cache,
            experiment_store: parking_lot::Mutex::new(experiment_store),
            local_rewards: parking_lot::Mutex::new(local_rewards),
            episode_completion_hook: None,
        })
    }

    /// Open a runtime with a custom model list for the cascade router.
    ///
    /// # Errors
    ///
    /// Returns an error if persistence files cannot be read/initialized.
    pub async fn open_with_models(
        paths: LearningPaths,
        regression: RegressionConfig,
        models: Vec<String>,
    ) -> Result<Self, LearningRuntimeError> {
        tokio::fs::create_dir_all(&paths.root).await?;
        tokio::fs::create_dir_all(&paths.playbooks_dir).await?;
        let affect_path = affect_state_path(&paths.root);
        if let Some(parent) = affect_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let episode_logger = EpisodeLogger::new(&paths.episodes_jsonl);
        let costs_log = CostsLog::open_creating(&paths.costs_jsonl).await?;
        let costs_db = CostsDb::new();
        let existing_costs = costs_log.read_all().await?;
        costs_db.insert_batch(existing_costs);
        let episode_count = count_episode_records(&paths.episodes_jsonl).await?;

        let skill_library = SkillLibrary::new(&paths.skills_json).await?;
        let playbook_store = PlaybookStore::new(&paths.playbooks_dir);
        let playbook_rules = PlaybookRules::open(&paths.playbook_rules_toml)?;
        let task_metrics = load_task_metrics(&paths.task_metrics_jsonl).await?;

        let pattern_miner = parking_lot::Mutex::new(PatternMiner::new(3, 0.5));
        let latency_registry = LatencyRegistry::load_or_new(&paths.latency_stats_json);
        let cascade_router = CascadeRouter::load_or_new(&paths.cascade_router_json, models);
        let context_pack_cache = ContextPackCache::new(256, paths.root.join("context-cache.json"));
        let experiment_store = ExperimentStore::load_or_new(&paths.experiments_json);
        let local_rewards = load_local_rewards(&paths.local_rewards_json);

        Ok(Self {
            paths,
            episode_logger,
            update_frequency: UpdateFrequency::default(),
            episode_count: AtomicU64::new(episode_count),
            affect_engine: parking_lot::Mutex::new(DaimonState::load_or_new(&affect_path)),
            costs_log,
            costs_db,
            provider_health: ProviderHealthTracker::new(),
            skill_library,
            playbook_store,
            playbook_rules,
            regression,
            task_metrics: AsyncMutex::new(task_metrics),
            pattern_miner,
            latency_registry,
            cascade_router,
            context_pack_cache,
            experiment_store: parking_lot::Mutex::new(experiment_store),
            local_rewards: parking_lot::Mutex::new(local_rewards),
            episode_completion_hook: None,
        })
    }

    /// Convenience constructor using default paths under `root` and default regression config.
    ///
    /// # Errors
    ///
    /// Returns an error if persistence files cannot be read/initialized.
    pub async fn open_under(root: impl Into<PathBuf>) -> Result<Self, LearningRuntimeError> {
        Self::open(LearningPaths::under(root), RegressionConfig::default()).await
    }

    /// Borrow configured paths.
    #[must_use]
    pub const fn paths(&self) -> &LearningPaths {
        &self.paths
    }

    /// Borrow the configured subsystem update cadences.
    #[must_use]
    pub const fn update_frequency(&self) -> &UpdateFrequency {
        &self.update_frequency
    }

    /// Override the subsystem update cadences for this runtime.
    pub fn set_update_frequency(&mut self, update_frequency: UpdateFrequency) {
        self.update_frequency = update_frequency;
    }

    /// Borrow in-memory costs DB.
    #[must_use]
    pub const fn costs_db(&self) -> &CostsDb {
        &self.costs_db
    }

    /// Borrow provider health tracker.
    #[must_use]
    pub const fn provider_health(&self) -> &ProviderHealthTracker {
        &self.provider_health
    }

    /// Filter model slugs down to those whose providers are currently healthy.
    ///
    /// When every candidate resolves to an unhealthy provider, the original
    /// `all_model_slugs` set is returned so routing can still make progress.
    pub fn healthy_model_slugs<F>(&self, all_model_slugs: &[String], provider_of: F) -> Vec<String>
    where
        F: Fn(&str) -> String,
    {
        let healthy_models = self
            .provider_health
            .filter_arms(all_model_slugs, provider_of);
        if healthy_models.is_empty() {
            all_model_slugs.to_vec()
        } else {
            healthy_models
        }
    }

    /// Borrow skill library.
    #[must_use]
    pub const fn skill_library(&self) -> &SkillLibrary {
        &self.skill_library
    }

    /// Mutably borrow the skill library (e.g. for recording outcomes).
    pub const fn skill_library_mut(&mut self) -> &mut SkillLibrary {
        &mut self.skill_library
    }

    /// Borrow playbook rules.
    #[must_use]
    pub const fn playbook_rules(&self) -> &PlaybookRules {
        &self.playbook_rules
    }

    /// Borrow the latency registry used for routing feedback.
    #[must_use]
    pub const fn latency_registry(&self) -> &LatencyRegistry {
        &self.latency_registry
    }

    /// Borrow pattern miner (behind `parking_lot::Mutex` for `&mut` access).
    #[must_use]
    pub const fn pattern_miner(&self) -> &parking_lot::Mutex<PatternMiner> {
        &self.pattern_miner
    }

    /// Run the offline cross-episode consolidation pass over the persisted log.
    ///
    /// This loads the current `.roko/episodes.jsonl` batch, vectorizes each
    /// episode, and returns structural meta-patterns discovered through
    /// HDC bundling plus k-medoids clustering.
    ///
    /// # Errors
    ///
    /// Returns an error if the episode log cannot be read.
    pub async fn discover_cross_episode_patterns(
        &self,
    ) -> Result<CrossEpisodeConsolidationReport, LearningRuntimeError> {
        let episodes = EpisodeLogger::read_all(&self.paths.episodes_jsonl).await?;
        Ok(CrossEpisodeConsolidator::default().discover(&episodes))
    }

    /// Borrow cascade router.
    #[must_use]
    pub const fn cascade_router(&self) -> &CascadeRouter {
        &self.cascade_router
    }

    /// Borrow context pack cache.
    #[must_use]
    pub const fn context_pack_cache(&self) -> &ContextPackCache {
        &self.context_pack_cache
    }

    /// Borrow experiment store (behind `parking_lot::Mutex`).
    #[must_use]
    pub const fn experiment_store(&self) -> &parking_lot::Mutex<ExperimentStore> {
        &self.experiment_store
    }

    /// Query the local reward score for a subsystem decision.
    ///
    /// Returns a value in `[0.0, 1.0]` estimating how strongly the given
    /// local decision correlates with global task success.  Unknown
    /// decisions return a neutral `0.5`.
    pub fn local_reward_score(&self, subsystem: &str, decision_key: &str) -> f64 {
        self.local_rewards
            .lock()
            .get(subsystem)
            .map_or(0.5, |reward| reward.score(decision_key))
    }

    /// Record a local decision outcome against global task success for the
    /// named subsystem.
    fn observe_local_reward(&self, subsystem: &str, decision_key: &str, global_success: bool) {
        self.local_rewards
            .lock()
            .entry(subsystem.to_owned())
            .or_default()
            .observe(decision_key, global_success);
    }

    /// Persist local reward functions to disk.
    fn save_local_rewards(&self) {
        let rewards = self.local_rewards.lock();
        if let Ok(json) = serde_json::to_string_pretty(&*rewards) {
            let _ = std::fs::write(&self.paths.local_rewards_json, json);
        }
    }

    /// Install a callback that runs after a completed episode is
    /// persisted.
    ///
    /// The callback is synchronous so it can enqueue background work
    /// without holding up the learning runtime.
    pub fn set_episode_completion_hook<F>(&mut self, hook: F)
    where
        F: Fn(Episode) + Send + Sync + 'static,
    {
        self.episode_completion_hook = Some(Arc::new(hook));
    }

    /// Append an efficiency event to the JSONL log.
    ///
    /// # Errors
    ///
    /// Returns an error on write failure.
    pub async fn append_efficiency_event(
        &self,
        event: &AgentEfficiencyEvent,
    ) -> Result<(), LearningRuntimeError> {
        let mut line = serde_json::to_string(event)?;
        line.push('\n');
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.paths.efficiency_jsonl)
            .await?;
        f.write_all(line.as_bytes()).await?;
        self.record_latency_from_efficiency_event(event)?;
        Ok(())
    }

    /// Compute a latency-aware routing reward for a model/provider observation.
    ///
    /// The current wall-clock latency comes from the efficiency event emitted
    /// for this turn. When that timing is unavailable, the historical p50 for
    /// the same `(model, provider)` pair is used as a fallback.
    #[must_use]
    pub fn compute_routing_reward_with_latency(
        &self,
        gate_passed: bool,
        cost_usd: f64,
        wall_time_ms: u64,
        model: &str,
        provider: &str,
    ) -> f64 {
        compute_reward_with_latency(
            gate_passed,
            cost_usd,
            wall_time_ms,
            &self.latency_registry,
            model,
            provider,
        )
    }

    /// Read all persisted efficiency events from the JSONL log.
    ///
    /// Returns an empty vec if the file does not exist.
    pub async fn read_efficiency_events(
        &self,
    ) -> Result<Vec<AgentEfficiencyEvent>, LearningRuntimeError> {
        read_efficiency_events(&self.paths.efficiency_jsonl).await
    }

    /// Read the latest persisted C-Factor snapshot, if one exists.
    pub async fn latest_cfactor(&self) -> Result<Option<CFactor>, LearningRuntimeError> {
        let contents = match tokio::fs::read_to_string(&self.paths.cfactor_jsonl).await {
            Ok(contents) => contents,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(LearningRuntimeError::Io(err)),
        };

        let snapshot = contents
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .and_then(|line| serde_json::from_str::<CFactor>(line).ok());

        Ok(snapshot)
    }

    /// Save cascade router observations to disk.
    pub fn save_cascade_router(&self) -> Result<(), LearningRuntimeError> {
        self.cascade_router.save(&self.paths.cascade_router_json)?;
        Ok(())
    }

    /// Record conductor-driven negative feedback for the routed model.
    ///
    /// Restart/fail interventions indicate the selected model failed to make
    /// acceptable progress for the current routing context, so they are fed
    /// back into the cascade router as a zero-reward failure.
    pub fn record_conductor_intervention(
        &self,
        routing_context: &RoutingContext,
        model_slug: &str,
        intervention: &ConductorDecision,
    ) -> bool {
        if !matches!(
            intervention,
            ConductorDecision::Restart { .. } | ConductorDecision::Fail { .. }
        ) {
            return false;
        }

        self.cascade_router
            .record_observation(routing_context, model_slug, 0.0, false);
        if let Err(err) = self.save_cascade_router() {
            eprintln!("[learn] cascade router save failed after conductor intervention: {err}");
        }
        true
    }

    /// Append one raw episode record without triggering any learning updates.
    pub async fn append_episode(&self, episode: &Episode) -> Result<(), LearningRuntimeError> {
        let mut episode = episode.clone();
        self.apply_affect_signature(&mut episode);
        self.episode_logger.append(&episode).await?;
        Ok(())
    }

    /// Persist one completed run and update all available learning subsystems.
    ///
    /// The function is intentionally tolerant of missing optional fields:
    /// it performs whichever updates are possible from the provided input.
    ///
    /// # Errors
    ///
    /// Returns an error on persistence failures for enabled subsystems.
    pub async fn record_completed_run(
        &self,
        mut input: CompletedRunInput,
    ) -> Result<LearningUpdate, LearningRuntimeError> {
        let mut update = LearningUpdate::default();

        input.episode.attach_all_fingerprints();
        self.apply_affect_signature(&mut input.episode);
        self.episode_logger.append(&input.episode).await?;
        update.episode_logged = ApplyStatus::Applied;
        if let Some(hook) = &self.episode_completion_hook {
            hook(input.episode.clone());
        }
        let episode_count = self.episode_count.fetch_add(1, Ordering::Relaxed) + 1;

        if input.playbook_id.is_none() {
            input.playbook_id = extra_string(&input.episode, "playbook_id");
        }
        if input.playbook_rule_id.is_none() {
            input.playbook_rule_id = extra_string(&input.episode, "playbook_rule_id");
        }
        if input.matched_skill_id.is_none() {
            input.matched_skill_id = extra_string(&input.episode, "skill_name")
                .or_else(|| extra_string(&input.episode, "matched_skill_id"));
        }

        let cost_record = match input.cost_record {
            Some(record) => Some(record),
            None => derive_cost_record(&input.episode, input.provider.as_deref()),
        };

        if let Some(record) = cost_record {
            self.costs_db.insert(record.clone());
            self.costs_log.append(&record).await?;
            update.cost_logged = ApplyStatus::Applied;

            if input.provider.is_none() {
                input.provider = Some(record.provider.clone());
            }
        }

        if let Some(provider) = input.provider {
            if input.episode.success {
                self.provider_health.record_success(&provider);
            } else {
                self.provider_health.record_failure(&provider);
            }
            update.provider_updated = ApplyStatus::Applied;
        }

        if let Some(playbook_id) = input.playbook_id {
            if self
                .playbook_store
                .record_outcome(&playbook_id, input.episode.success)
                .await?
            {
                update.playbook_updated = ApplyStatus::Applied;
            }
        }

        // Stash decision keys before they're consumed for local reward tracking.
        let local_reward_rule_id = input.playbook_rule_id.clone();
        let local_reward_skill_id = input.matched_skill_id.clone();

        if let Some(rule_id) = input.playbook_rule_id {
            self.playbook_rules
                .record_outcome(&rule_id, input.episode.success);
            self.playbook_rules.save()?;
            update.playbook_rule_updated = ApplyStatus::Applied;
        }

        if let Some(skill_id) = input.matched_skill_id
            && self.skill_library.get(&skill_id).is_some()
        {
            self.skill_library
                .record_outcome(&skill_id, input.episode.success)
                .await?;
            update.matched_skill_updated = ApplyStatus::Applied;
        }

        let generator = TemplatePatternGenerator;
        if self.update_frequency.skill_mining_due(episode_count)
            && let Some(skill) = self.skill_library.extract(&input.episode, &generator).await
        {
            update.extracted_skill_id = Some(skill.name);
        }

        if let Some(metric) = input.task_metric {
            append_task_metric(&self.paths.task_metrics_jsonl, &metric).await?;
            let metrics_snapshot = {
                let mut guard = self.task_metrics.lock().await;
                guard.push(metric);
                guard.clone()
            };
            update.regression_report =
                compute_regression_report(&metrics_snapshot, &self.regression);
        }

        if self.update_frequency.distiller_due(episode_count) {
            self.append_cfactor_snapshot().await?;
        }

        // ── Pattern mining ──────────────────────────────────────────────
        let actions = EpisodeActions::from_episode(&input.episode);
        if self.update_frequency.pattern_discovery_due(episode_count) && !actions.actions.is_empty()
        {
            self.pattern_miner.lock().ingest_episode(&actions);
            update.patterns_ingested = true;
        }

        // ── Cascade router observation ─────────────────────────────────
        if self.update_frequency.router_due(episode_count) {
            update.router_updated = self.update_cascade_router(&input.episode);
        }

        // Persist immediately so the router state file always reflects the
        // latest observation count and confidence stats.
        if update.router_updated {
            if let Err(e) = self.save_cascade_router() {
                eprintln!("[learn] cascade router save failed: {e}");
            }
        }

        // ── Prompt experiment outcome ────────────────────────────────────
        if self.update_frequency.experiments_due(episode_count)
            && let Some(ref variant_id) = input.experiment_variant_id
        {
            let mut store = self.experiment_store.lock();
            let was_running = store
                .iter()
                .find(|experiment| experiment.stats.contains_key(variant_id))
                .is_some_and(|experiment| experiment.status == ExperimentStatus::Running);
            store.record_outcome(variant_id, input.episode.success);
            let static_table_updated = was_running
                && store
                    .iter()
                    .find(|experiment| experiment.stats.contains_key(variant_id))
                    .is_some_and(|experiment| self.on_experiment_concluded(experiment));
            if let Err(e) = store.save(&self.paths.experiments_json) {
                eprintln!("[learn] experiment store save failed: {e}");
            }
            drop(store);
            if static_table_updated && let Err(e) = self.save_cascade_router() {
                eprintln!("[learn] cascade router save failed after experiment conclusion: {e}");
            }
        }

        // ── Local reward observations ─────────────────────────────────────
        // Record (local_decision, global_outcome) for each subsystem that
        // participated in this run so the Optimas-style reward functions
        // learn which local choices correlate with global task success.
        let success = input.episode.success;
        if let Some(model) = extra_string(&input.episode, "model") {
            self.observe_local_reward("router", &model, success);
        }
        if let Some(ref skill_id) = local_reward_skill_id {
            self.observe_local_reward("skill", skill_id, success);
        }
        if let Some(ref rule_id) = local_reward_rule_id {
            self.observe_local_reward("playbook_rule", rule_id, success);
        }
        self.save_local_rewards();

        Ok(update)
    }

    /// Attach the current PAD snapshot to an episode before it is persisted.
    fn apply_affect_signature(&self, episode: &mut Episode) {
        let task_key = if episode.task_id.trim().is_empty() {
            episode.agent_id.clone()
        } else {
            episode.task_id.clone()
        };

        let mut engine = self.affect_engine.lock();
        for (rung, verdict) in episode.gate_verdicts.iter().enumerate() {
            let _ = engine.appraise(AffectEvent::GateResult {
                plan_id: String::new(),
                task_id: task_key.clone(),
                passed: verdict.passed,
                rung: rung as u32,
            });
        }
        if episode.success {
            let _ = engine.appraise(AffectEvent::TaskOutcome {
                task_id: task_key.clone(),
                succeeded: true,
            });
        } else {
            let _ = engine.appraise(AffectEvent::TaskOutcome {
                task_id: task_key.clone(),
                succeeded: false,
            });
        }

        let state = engine.query();
        episode.extra.insert(
            "pad".to_string(),
            serde_json::json!({
                "pleasure": state.pad.pleasure,
                "arousal": state.pad.arousal,
                "dominance": state.pad.dominance,
            }),
        );
        episode.extra.insert(
            "affect_confidence".to_string(),
            serde_json::json!(state.confidence),
        );
    }

    /// Update the cascade router from episode metadata if role + model are available.
    fn update_cascade_router(&self, episode: &Episode) -> bool {
        let role_str = extra_string(episode, "role");
        let model_slug = extra_string(episode, "model");
        let (Some(role_raw), Some(slug)) = (role_str, model_slug) else {
            return false;
        };
        let Some(role) = parse_agent_role(&role_raw) else {
            return false;
        };
        let category_str =
            extra_string(episode, "task_category").unwrap_or_else(|| "implementation".to_string());
        let cat_json = format!("\"{category_str}\"");
        let task_category =
            serde_json::from_str::<TaskCategory>(&cat_json).unwrap_or(TaskCategory::Implementation);
        let complexity_str =
            extra_string(episode, "complexity_band").unwrap_or_else(|| "standard".to_string());
        let cplx_json = format!("\"{complexity_str}\"");
        let complexity = serde_json::from_str::<TaskComplexityBand>(&cplx_json)
            .unwrap_or(TaskComplexityBand::Standard);
        let crate_familiarity = extra_f64(episode, "crate_familiarity").unwrap_or(0.5);

        let ctx = RoutingContext {
            task_category,
            complexity,
            iteration: 0,
            role,
            crate_familiarity,
            has_prior_failure: !episode.success,
            affect_confidence: extra_f64(episode, "affect_confidence").unwrap_or(0.5),
            behavioral_state: roko_core::BehavioralState::Engaged,
            thinking_level: None,
            previous_model: None,
            plan_context_tokens: None,
        };
        if episode
            .extra
            .get("cascade_router_observed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            return false;
        }
        let provider = extra_string(episode, "provider")
            .or_else(|| extra_string(episode, "backend"))
            .unwrap_or_else(|| "unknown-provider".to_string());
        let reward = self.compute_routing_reward_with_latency(
            episode.success,
            episode.usage.cost_usd,
            episode.usage.wall_ms,
            &slug,
            &provider,
        );
        self.cascade_router
            .record_observation(&ctx, &slug, reward, episode.success);
        true
    }

    /// Promote a concluded model experiment winner into the router's static table.
    fn on_experiment_concluded(&self, experiment: &PromptExperiment) -> bool {
        let (Some(winner_id), Some(role_raw)) =
            (experiment.winner_id.as_deref(), experiment.role.as_deref())
        else {
            return false;
        };
        let Some(role) = parse_agent_role(role_raw) else {
            return false;
        };
        let Some(winner_slug) = experiment
            .variants
            .iter()
            .find(|variant| variant.id == winner_id)
            .and_then(|variant| variant.slug.as_deref())
        else {
            return false;
        };
        if !self.cascade_router.update_static_table(role, winner_slug) {
            return false;
        }
        eprintln!(
            "[learn] experiment concluded — updated static routing table: experiment={} winner={} role={}",
            experiment.experiment_id, winner_slug, role_raw
        );
        true
    }

    /// Return the current arousal value tracked for a task key.
    pub fn task_arousal(&self, task_id: impl AsRef<str>) -> f64 {
        let _ = task_id.as_ref();
        self.affect_engine.lock().query().pad.arousal
    }

    /// Return the current task confidence tracked for a task key.
    pub fn task_confidence(&self, task_id: impl AsRef<str>) -> f64 {
        let _ = task_id.as_ref();
        self.affect_engine.lock().query().confidence
    }

    /// Return the current task arousal with queue-wait motivation applied.
    pub fn task_arousal_with_queue_wait(&self, task_id: impl AsRef<str>, queued_hours: f64) -> f64 {
        let base = self.task_arousal(task_id);
        let bump = queue_wait_arousal(queued_hours);
        (base + bump).clamp(-1.0, 1.0)
    }

    /// Compute the current C-Factor snapshot and append it to the history log.
    async fn append_cfactor_snapshot(&self) -> Result<(), LearningRuntimeError> {
        let snapshot = compute_cfactor_snapshot(&self.paths.root).await?;
        append_cfactor_snapshot(&self.paths.cfactor_jsonl, &snapshot).await?;
        Ok(())
    }
}

fn compute_reward_with_latency(
    gate_passed: bool,
    cost_usd: f64,
    wall_time_ms: u64,
    latency_stats: &LatencyRegistry,
    model: &str,
    provider: &str,
) -> f64 {
    let pass_rate = if gate_passed { 1.0 } else { 0.0 };
    let max_cost = 5.0;
    let normalized_cost = (cost_usd / max_cost).min(1.0);
    let historical_p50_ms = latency_stats
        .get(model, provider)
        .map(|stats| stats.p50_ms());
    let observed_latency_ms = if wall_time_ms > 0 {
        wall_time_ms as f64
    } else {
        historical_p50_ms.unwrap_or(30_000.0)
    };
    let sla_ms = 120_000.0;
    compute_routing_reward_v2(pass_rate, normalized_cost, observed_latency_ms, sla_ms)
}

fn latency_model_slug(event: &AgentEfficiencyEvent) -> &str {
    let model = event.model_used.trim();
    if model.is_empty() {
        event.model.trim()
    } else {
        model
    }
}

fn latency_provider_id(event: &AgentEfficiencyEvent) -> &str {
    event.backend.trim()
}

fn latency_total_ms(event: &AgentEfficiencyEvent) -> f64 {
    if event.wall_time_ms > 0 {
        event.wall_time_ms as f64
    } else {
        event.duration_ms as f64
    }
}

impl LearningRuntime {
    fn record_latency_from_efficiency_event(
        &self,
        event: &AgentEfficiencyEvent,
    ) -> Result<(), LearningRuntimeError> {
        let model = latency_model_slug(event);
        let provider = latency_provider_id(event);
        if model.is_empty() || provider.is_empty() {
            return Ok(());
        }

        let total_ms = latency_total_ms(event);
        self.latency_registry.record(
            model,
            provider,
            event.time_to_first_token_ms as f64,
            total_ms,
            event.output_tokens,
        );
        self.latency_registry.save(&self.paths.latency_stats_json)?;
        Ok(())
    }
}

/// Load persisted local reward functions, or return an empty map.
fn load_local_rewards(path: &Path) -> HashMap<String, LocalRewardFunction> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

/// Read optional string value from `episode.extra`.
fn extra_string(episode: &Episode, key: &str) -> Option<String> {
    episode
        .extra
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
}

/// Read optional floating-point value from `episode.extra`.
fn extra_f64(episode: &Episode, key: &str) -> Option<f64> {
    episode.extra.get(key).and_then(serde_json::Value::as_f64)
}

fn episode_source_id(episode: &Episode) -> &str {
    if episode.episode_id.trim().is_empty() {
        &episode.id
    } else {
        &episode.episode_id
    }
}

fn episode_agent_label(episode: &Episode) -> String {
    let agent_id = episode.agent_id.trim();
    if !agent_id.is_empty() {
        return agent_id.to_string();
    }

    let template = episode.agent_template.trim();
    if !template.is_empty() {
        return template.to_string();
    }

    episode.id.clone()
}

/// Parse an [`AgentRole`] from either the persisted kebab-case label or the
/// debug-style variant name used by `format!("{role:?}")` in orchestration.
fn parse_agent_role(raw: &str) -> Option<AgentRole> {
    if let Ok(role) = serde_json::from_str::<AgentRole>(&format!("\"{raw}\"")) {
        return Some(role);
    }

    std::iter::once(AgentRole::Conductor)
        .chain(AgentRole::ALL_AGENTS.iter().copied())
        .find(|role| raw == format!("{role:?}"))
}

/// Build a [`CostRecord`] from an [`Episode`] and optional provider override.
fn derive_cost_record(episode: &Episode, provider_override: Option<&str>) -> Option<CostRecord> {
    if episode.agent_id.is_empty() && episode.task_id.is_empty() {
        return None;
    }

    let provider = provider_override
        .map(ToOwned::to_owned)
        .or_else(|| extra_string(episode, "provider"))
        .unwrap_or_else(|| "unknown-provider".to_string());

    Some(CostRecord {
        timestamp: episode.timestamp.to_rfc3339(),
        model: extra_string(episode, "model").unwrap_or_else(|| "unknown-model".to_string()),
        provider,
        role: extra_string(episode, "role").unwrap_or_else(|| "unknown-role".to_string()),
        plan_id: extra_string(episode, "plan_id").unwrap_or_default(),
        task_id: if episode.task_id.is_empty() {
            extra_string(episode, "task_id").unwrap_or_default()
        } else {
            episode.task_id.clone()
        },
        complexity_band: extra_string(episode, "complexity_band")
            .unwrap_or_else(|| "standard".to_string()),
        input_tokens: episode.usage.input_tokens,
        output_tokens: episode.usage.output_tokens,
        cached_tokens: episode.usage.cache_read_tokens,
        cost_usd: episode.usage.cost_usd,
        duration_ms: episode.usage.wall_ms,
        success: episode.success,
        session_id: extra_string(episode, "session_id").unwrap_or_default(),
    })
}

/// Load `TaskMetric` records from a JSONL path, skipping malformed lines.
async fn load_task_metrics(path: &Path) -> io::Result<Vec<TaskMetric>> {
    let file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut lines = BufReader::new(file).lines();
    let mut out = Vec::new();
    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(metric) = serde_json::from_str::<TaskMetric>(trimmed) {
            out.push(metric);
        }
    }
    Ok(out)
}

async fn count_episode_records(path: &Path) -> io::Result<u64> {
    let file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };
    let mut lines = BufReader::new(file).lines();
    let mut count = 0_u64;
    while let Some(line) = lines.next_line().await? {
        if !line.trim().is_empty() {
            count = count.saturating_add(1);
        }
    }
    Ok(count)
}

/// Append one `TaskMetric` line to `path`.
async fn append_task_metric(path: &Path, metric: &TaskMetric) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut line =
        serde_json::to_string(metric).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    line.push('\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(line.as_bytes()).await?;
    file.sync_data().await?;
    Ok(())
}

/// Append one `CFactor` snapshot to `path`.
async fn append_cfactor_snapshot(
    path: &Path,
    snapshot: &crate::cfactor::CFactor,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut line = serde_json::to_string(snapshot)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    line.push('\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(line.as_bytes()).await?;
    file.sync_data().await?;
    Ok(())
}

/// Compute a regression report using historical records.
///
/// Uses all-but-last-`current_window` records as baseline and the latest
/// window as current. Returns `None` when there is insufficient history.
fn compute_regression_report(
    metrics: &[TaskMetric],
    cfg: &RegressionConfig,
) -> Option<RegressionReport> {
    let min = cfg.thresholds.min_records;
    if metrics.len() < min.saturating_mul(2) {
        return None;
    }

    let window = cfg
        .current_window
        .max(min)
        .min(metrics.len().saturating_sub(min));
    if window == 0 || metrics.len() <= window {
        return None;
    }

    let split = metrics.len() - window;
    let baseline_records = &metrics[..split];
    let current_records = &metrics[split..];
    let baseline = crate::baseline::compute_baseline(baseline_records, min);
    Some(detect_regressions(
        &baseline,
        current_records,
        &cfg.thresholds,
    ))
}

/// Read efficiency events from a JSONL file. Returns empty vec if file missing.
pub async fn read_efficiency_events(
    path: &Path,
) -> Result<Vec<AgentEfficiencyEvent>, LearningRuntimeError> {
    let file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(LearningRuntimeError::Io(err)),
    };
    let mut lines = BufReader::new(file).lines();
    let mut out = Vec::new();
    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<AgentEfficiencyEvent>(trimmed) {
            out.push(event);
        }
    }
    Ok(out)
}

/// Compute the current C-Factor snapshot for `learn_root` and append it to the
/// history log.
///
/// Returns the snapshot that was persisted.
pub async fn refresh_cfactor_snapshot(
    learn_root: impl AsRef<Path>,
) -> Result<CFactor, LearningRuntimeError> {
    let learn_root = learn_root.as_ref();
    let paths = LearningPaths::under(learn_root.to_path_buf());
    let snapshot = compute_cfactor_snapshot(learn_root).await?;
    append_cfactor_snapshot(&paths.cfactor_jsonl, &snapshot).await?;
    Ok(snapshot)
}

#[derive(Debug, Deserialize)]
struct ContextAttributionRecord {
    #[serde(default = "default_now")]
    ts: DateTime<Utc>,
    #[serde(default)]
    source_type: String,
    #[serde(default)]
    referenced: bool,
}

async fn compute_cfactor_snapshot(learn_root: &Path) -> Result<CFactor, LearningRuntimeError> {
    let paths = LearningPaths::under(learn_root.to_path_buf());
    let episodes = EpisodeLogger::read_all_lossy(&paths.episodes_jsonl).await?;
    let attribution_path = learn_root
        .parent()
        .unwrap_or(learn_root)
        .join("context-attribution.jsonl");
    let knowledge_path = learn_root
        .parent()
        .unwrap_or(learn_root)
        .join("neuro")
        .join("knowledge.jsonl");
    // Dedicated confirmation records emitted by KnowledgeStore on ingest.
    let confirmations_path = learn_root
        .parent()
        .unwrap_or(learn_root)
        .join("neuro")
        .join("knowledge-confirmations.jsonl");
    let attribution_records = read_context_attribution_records(&attribution_path).await?;
    // Read from both legacy knowledge entries and the dedicated
    // confirmation records file, then merge.
    let mut knowledge_records = read_knowledge_records(&knowledge_path).await?;
    let confirmation_records = read_knowledge_records(&confirmations_path).await?;
    knowledge_records.extend(confirmation_records);
    let social_sensitivity = social_sensitivity_from_attribution(
        &attribution_records,
        Duration::from_secs(7 * 24 * 60 * 60),
    );
    let knowledge_integration_rate = knowledge_integration_rate(
        &knowledge_records,
        &episodes,
        Duration::from_secs(7 * 24 * 60 * 60),
    );
    let convergence_velocity = convergence_velocity_from_agreement(
        &knowledge_records,
        &episodes,
        Duration::from_secs(7 * 24 * 60 * 60),
    );
    Ok(compute_cfactor(
        &episodes,
        Duration::from_secs(7 * 24 * 60 * 60),
        social_sensitivity,
        knowledge_integration_rate,
        convergence_velocity,
    ))
}

async fn read_context_attribution_records(
    path: &Path,
) -> Result<Vec<ContextAttributionRecord>, LearningRuntimeError> {
    let file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(LearningRuntimeError::Io(err)),
    };

    let mut lines = BufReader::new(file).lines();
    let mut out = Vec::new();
    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<ContextAttributionRecord>(trimmed) {
            out.push(record);
        }
    }
    Ok(out)
}

fn social_sensitivity_from_attribution(
    records: &[ContextAttributionRecord],
    window: Duration,
) -> f64 {
    let cutoff = match chrono::Duration::from_std(window) {
        Ok(delta) => Utc::now() - delta,
        Err(_) => DateTime::<Utc>::MIN_UTC,
    };

    let mut referenced = 0usize;
    let mut total = 0usize;
    for record in records.iter().filter(|record| record.ts >= cutoff) {
        if record.source_type != "prior_output" {
            continue;
        }
        total += 1;
        if record.referenced {
            referenced += 1;
        }
    }

    if total == 0 {
        0.0
    } else {
        referenced as f64 / total as f64
    }
}

fn default_now() -> DateTime<Utc> {
    Utc::now()
}

#[derive(Debug, Deserialize)]
struct KnowledgeConfirmationRecord {
    #[serde(default = "default_now")]
    created_at: DateTime<Utc>,
    #[serde(default)]
    source_episodes: Vec<String>,
}

async fn read_knowledge_records(
    path: &Path,
) -> Result<Vec<KnowledgeConfirmationRecord>, LearningRuntimeError> {
    let file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(LearningRuntimeError::Io(err)),
    };

    let mut lines = BufReader::new(file).lines();
    let mut out = Vec::new();
    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<KnowledgeConfirmationRecord>(trimmed) {
            out.push(record);
        }
    }
    Ok(out)
}

fn knowledge_integration_rate(
    records: &[KnowledgeConfirmationRecord],
    episodes: &[Episode],
    window: Duration,
) -> f64 {
    let cutoff = match chrono::Duration::from_std(window) {
        Ok(delta) => Utc::now() - delta,
        Err(_) => DateTime::<Utc>::MIN_UTC,
    };

    let mut episode_timestamps: HashMap<String, DateTime<Utc>> = HashMap::new();
    for episode in episodes {
        let source_id = episode_source_id(episode).to_string();
        episode_timestamps
            .entry(source_id)
            .and_modify(|current| {
                if episode.timestamp < *current {
                    *current = episode.timestamp;
                }
            })
            .or_insert(episode.timestamp);
    }

    let mut weighted_speed_sum = 0.0;
    let mut total_weight = 0.0;

    for record in records.iter().filter(|record| record.created_at >= cutoff) {
        let mut source_ids = record.source_episodes.iter().cloned().collect::<Vec<_>>();
        source_ids.sort();
        source_ids.dedup();

        let mut timestamps: Vec<DateTime<Utc>> = source_ids
            .iter()
            .filter_map(|source| episode_timestamps.get(source).copied())
            .collect();
        timestamps.sort();
        if timestamps.len() < 2 {
            continue;
        }

        let confirmations = source_ids.len().saturating_sub(1);
        let span = timestamps
            .last()
            .copied()
            .unwrap_or(record.created_at)
            .signed_duration_since(timestamps.first().copied().unwrap_or(record.created_at));
        let span_hours = span
            .to_std()
            .map(|duration| duration.as_secs_f64() / 3_600.0)
            .unwrap_or(0.0);
        let normalized_speed =
            ((confirmations as f64) / span_hours.max(1.0 / 60.0) / 4.0).clamp(0.0, 1.0);
        let weight = confirmations as f64;
        weighted_speed_sum += normalized_speed * weight;
        total_weight += weight;
    }

    if total_weight == 0.0 {
        0.0
    } else {
        (weighted_speed_sum / total_weight).clamp(0.0, 1.0)
    }
}

fn convergence_velocity_from_agreement(
    records: &[KnowledgeConfirmationRecord],
    episodes: &[Episode],
    window: Duration,
) -> f64 {
    let cutoff = match chrono::Duration::from_std(window) {
        Ok(delta) => Utc::now() - delta,
        Err(_) => DateTime::<Utc>::MIN_UTC,
    };

    let mut episode_agents: HashMap<String, (DateTime<Utc>, String)> = HashMap::new();
    for episode in episodes {
        let source_id = episode_source_id(episode).to_string();
        let agent_id = episode_agent_label(episode);
        episode_agents
            .entry(source_id)
            .and_modify(|current| {
                if episode.timestamp < current.0 {
                    current.0 = episode.timestamp;
                    current.1 = agent_id.clone();
                }
            })
            .or_insert((episode.timestamp, agent_id));
    }

    let mut weighted_speed_sum = 0.0;
    let mut total_weight = 0.0;

    for record in records.iter().filter(|record| record.created_at >= cutoff) {
        let mut source_ids = record.source_episodes.iter().cloned().collect::<Vec<_>>();
        source_ids.sort();
        source_ids.dedup();

        let mut agent_timestamps: HashMap<String, DateTime<Utc>> = HashMap::new();
        for source_id in source_ids {
            let Some((timestamp, agent_id)) = episode_agents.get(&source_id).cloned() else {
                continue;
            };
            agent_timestamps
                .entry(agent_id)
                .and_modify(|current| {
                    if timestamp < *current {
                        *current = timestamp;
                    }
                })
                .or_insert(timestamp);
        }

        if agent_timestamps.len() < 2 {
            continue;
        }

        let mut timestamps: Vec<DateTime<Utc>> = agent_timestamps.values().copied().collect();
        timestamps.sort();
        let span = timestamps
            .last()
            .copied()
            .unwrap_or(record.created_at)
            .signed_duration_since(timestamps.first().copied().unwrap_or(record.created_at));
        let span_hours = span
            .to_std()
            .map(|duration| duration.as_secs_f64() / 3_600.0)
            .unwrap_or(0.0);
        let agreements = agent_timestamps.len().saturating_sub(1);
        let normalized_velocity =
            ((agreements as f64) / span_hours.max(1.0 / 60.0) / 4.0).clamp(0.0, 1.0);
        let weight = agreements as f64;
        weighted_speed_sum += normalized_velocity * weight;
        total_weight += weight;
    }

    if total_weight == 0.0 {
        0.0
    } else {
        (weighted_speed_sum / total_weight).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_experiment::{PromptExperiment, PromptVariant};
    use chrono::Utc;
    use roko_core::metric::{ConfigHash, TaskMetric};
    use tempfile::TempDir;

    fn sample_episode(success: bool) -> Episode {
        let mut ep = Episode::new("claude", "task-1");
        ep.success = success;
        ep.timestamp = Utc::now();
        ep.usage.input_tokens = 123;
        ep.usage.output_tokens = 45;
        ep.usage.cache_read_tokens = 7;
        ep.usage.cost_usd = 0.42;
        ep.usage.wall_ms = 900;
        ep.extra
            .insert("provider".to_string(), serde_json::json!("anthropic"));
        ep.extra
            .insert("model".to_string(), serde_json::json!("claude-opus-4-6"));
        ep.extra
            .insert("role".to_string(), serde_json::json!("Implementer"));
        ep.extra
            .insert("plan_id".to_string(), serde_json::json!("plan-1"));
        ep.extra
            .insert("complexity_band".to_string(), serde_json::json!("standard"));
        ep.extra
            .insert("iteration".to_string(), serde_json::json!(1_u64));
        ep.extra
            .insert("task_tags".to_string(), serde_json::json!(["rust", "fix"]));
        ep.extra.insert(
            "files".to_string(),
            serde_json::json!(["crates/roko-cli/src/run.rs"]),
        );
        ep.extra
            .insert("task_category".to_string(), serde_json::json!("bugfix"));
        ep
    }

    fn episode_at(task_id: &str, minutes_ago: i64, success: bool) -> Episode {
        let mut ep = sample_episode(success);
        ep.id = format!("{task_id}-id");
        ep.episode_id = task_id.to_string();
        ep.task_id = task_id.to_string();
        ep.timestamp = Utc::now() - chrono::Duration::minutes(minutes_ago);
        ep
    }

    fn episode_with_agent(
        task_id: &str,
        minutes_ago: i64,
        success: bool,
        agent_id: &str,
    ) -> Episode {
        let mut ep = episode_at(task_id, minutes_ago, success);
        ep.agent_id = agent_id.to_string();
        ep
    }

    fn sample_metric(i: u32, passed: bool, cost: f64) -> TaskMetric {
        let mut m = TaskMetric::new(ConfigHash::from("cfg-1".to_string()), "plan-1", "task-1");
        m.timestamp = "2026-04-08T00:00:00Z".to_string();
        m.run_id = format!("run-{i}");
        m.iteration = i;
        m.role = "Implementer".to_string();
        m.backend = "claude".to_string();
        m.model = "claude-opus-4-6".to_string();
        m.complexity_band = "standard".to_string();
        m.gate = "compile".to_string();
        m.gate_passed = passed;
        m.wall_time_ms = 1000 + u64::from(i);
        m.input_tokens = 100;
        m.output_tokens = 20;
        m.cached_tokens = 0;
        m.cost_usd = cost;
        m.sections_included = 3;
        m.sections_dropped = 0;
        m.context_tokens = 400;
        m.cache_hit_rate = 0.0;
        m
    }

    fn sample_pattern_episode(success: bool, suffix: &str) -> Episode {
        let mut ep = sample_episode(success);
        ep.id = format!("episode-{suffix}");
        ep.episode_id = format!("episode-{suffix}");
        ep.task_id = format!("task-{suffix}");
        ep.gate_verdicts = vec![
            crate::episode_logger::GateVerdict::new("read", true),
            crate::episode_logger::GateVerdict::new("edit", true),
            crate::episode_logger::GateVerdict::new("test", true),
        ];
        ep.extra.insert(
            "task_tags".to_string(),
            serde_json::json!(["rust", format!("tag-{suffix}")]),
        );
        ep.extra.insert(
            "files".to_string(),
            serde_json::json!([format!("crates/roko-cli/src/{suffix}.rs")]),
        );
        ep
    }

    #[tokio::test]
    async fn completed_run_updates_episode_cost_provider_and_skill() {
        let tmp = TempDir::new().unwrap();
        let mut runtime = LearningRuntime::open_under(tmp.path()).await.unwrap();
        let mut freq = *runtime.update_frequency();
        freq.skill_mining_every_n = 1;
        runtime.set_update_frequency(freq);

        let input = CompletedRunInput::from_episode(sample_episode(true));
        let update = runtime.record_completed_run(input).await.unwrap();

        assert_eq!(update.episode_logged, ApplyStatus::Applied);
        assert_eq!(update.cost_logged, ApplyStatus::Applied);
        assert_eq!(update.provider_updated, ApplyStatus::Applied);
        assert!(update.extracted_skill_id.is_some());
        assert_eq!(runtime.costs_db().len(), 1);

        let episodes_jsonl = std::fs::read_to_string(&runtime.paths().episodes_jsonl).unwrap();
        let persisted: Episode = serde_json::from_str(episodes_jsonl.lines().next().unwrap())
            .expect("persisted episode");
        let pad = persisted
            .extra
            .get("pad")
            .and_then(serde_json::Value::as_object)
            .expect("pad signature");
        assert!(pad.contains_key("pleasure"));
        assert!(pad.contains_key("arousal"));
        assert!(pad.contains_key("dominance"));
    }

    #[tokio::test]
    async fn completed_runs_append_cfactor_history() {
        let tmp = TempDir::new().unwrap();
        let mut runtime = LearningRuntime::open_under(tmp.path()).await.unwrap();
        let mut freq = *runtime.update_frequency();
        freq.distiller_every_n = 1;
        runtime.set_update_frequency(freq);

        runtime
            .record_completed_run(CompletedRunInput::from_episode(sample_episode(true)))
            .await
            .unwrap();
        runtime
            .record_completed_run(CompletedRunInput::from_episode(sample_episode(true)))
            .await
            .unwrap();

        let cfactor_jsonl = std::fs::read_to_string(&runtime.paths().cfactor_jsonl).unwrap();
        let snapshots: Vec<crate::cfactor::CFactor> = cfactor_jsonl
            .lines()
            .map(|line| serde_json::from_str(line).expect("valid c-factor snapshot"))
            .collect();

        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].episode_count, 1);
        assert_eq!(snapshots[1].episode_count, 2);
    }

    #[tokio::test]
    async fn update_frequency_separation() {
        let tmp = TempDir::new().unwrap();
        let mut runtime = LearningRuntime::open_with_models(
            LearningPaths::under(tmp.path()),
            RegressionConfig::default(),
            vec!["claude-opus-4-6".to_string()],
        )
        .await
        .unwrap();
        runtime.set_update_frequency(UpdateFrequency {
            router_every_n_episodes: 2,
            gate_thresholds_every_n: 5,
            experiments_every_n: 3,
            skill_mining_every_n: 2,
            pattern_discovery_every_n: 3,
            distiller_every_n: 4,
        });

        let mut experiment = PromptExperiment::new(
            "cadence-exp",
            "model-routing",
            vec![PromptVariant {
                id: "cadence".to_string(),
                name: "Cadence".to_string(),
                section_name: "model-routing".to_string(),
                content: String::new(),
                slug: Some("claude-opus-4-6".to_string()),
                active: true,
            }],
        );
        experiment.min_trials_per_variant = 100;
        runtime.experiment_store().lock().register(experiment);

        let update = runtime
            .record_completed_run(CompletedRunInput {
                experiment_variant_id: Some("cadence".to_string()),
                ..CompletedRunInput::from_episode(sample_pattern_episode(true, "one"))
            })
            .await
            .unwrap();
        assert!(!update.router_updated);
        assert!(update.extracted_skill_id.is_none());
        assert!(!update.patterns_ingested);
        assert_eq!(runtime.cascade_router().total_observations(), 0);
        assert_eq!(runtime.skill_library().len(), 0);
        assert_eq!(runtime.pattern_miner().lock().total_episodes(), 0);
        assert!(!runtime.paths().cfactor_jsonl.exists());
        assert_eq!(
            runtime
                .experiment_store()
                .lock()
                .get("cadence-exp")
                .and_then(|exp| exp.stats.get("cadence"))
                .map(|stats| stats.trials),
            Some(0)
        );

        let update = runtime
            .record_completed_run(CompletedRunInput {
                experiment_variant_id: Some("cadence".to_string()),
                ..CompletedRunInput::from_episode(sample_pattern_episode(true, "two"))
            })
            .await
            .unwrap();
        assert!(update.router_updated);
        assert!(update.extracted_skill_id.is_some());
        assert!(!update.patterns_ingested);
        assert_eq!(runtime.cascade_router().total_observations(), 1);
        assert_eq!(runtime.skill_library().len(), 1);
        assert_eq!(runtime.pattern_miner().lock().total_episodes(), 0);
        assert!(!runtime.paths().cfactor_jsonl.exists());
        assert_eq!(
            runtime
                .experiment_store()
                .lock()
                .get("cadence-exp")
                .and_then(|exp| exp.stats.get("cadence"))
                .map(|stats| stats.trials),
            Some(0)
        );

        let update = runtime
            .record_completed_run(CompletedRunInput {
                experiment_variant_id: Some("cadence".to_string()),
                ..CompletedRunInput::from_episode(sample_pattern_episode(true, "three"))
            })
            .await
            .unwrap();
        assert!(!update.router_updated);
        assert!(update.extracted_skill_id.is_none());
        assert!(update.patterns_ingested);
        assert_eq!(runtime.cascade_router().total_observations(), 1);
        assert_eq!(runtime.skill_library().len(), 1);
        assert_eq!(runtime.pattern_miner().lock().total_episodes(), 1);
        assert!(!runtime.paths().cfactor_jsonl.exists());
        assert_eq!(
            runtime
                .experiment_store()
                .lock()
                .get("cadence-exp")
                .and_then(|exp| exp.stats.get("cadence"))
                .map(|stats| stats.trials),
            Some(1)
        );

        let update = runtime
            .record_completed_run(CompletedRunInput {
                experiment_variant_id: Some("cadence".to_string()),
                ..CompletedRunInput::from_episode(sample_pattern_episode(true, "four"))
            })
            .await
            .unwrap();
        assert!(update.router_updated);
        assert!(update.extracted_skill_id.is_some());
        assert!(!update.patterns_ingested);
        assert_eq!(runtime.cascade_router().total_observations(), 2);
        assert_eq!(runtime.skill_library().len(), 2);
        assert_eq!(runtime.pattern_miner().lock().total_episodes(), 1);
        let cfactor_jsonl = std::fs::read_to_string(&runtime.paths().cfactor_jsonl).unwrap();
        let snapshots: Vec<crate::cfactor::CFactor> = cfactor_jsonl
            .lines()
            .map(|line| serde_json::from_str(line).expect("valid c-factor snapshot"))
            .collect();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].episode_count, 4);
        assert_eq!(
            runtime
                .experiment_store()
                .lock()
                .get("cadence-exp")
                .and_then(|exp| exp.stats.get("cadence"))
                .map(|stats| stats.trials),
            Some(1)
        );
    }

    #[test]
    fn social_sensitivity_uses_prior_output_attributions() {
        let now = Utc::now();
        let records = vec![
            ContextAttributionRecord {
                ts: now,
                source_type: "prior_output".to_string(),
                referenced: true,
            },
            ContextAttributionRecord {
                ts: now,
                source_type: "prior_output".to_string(),
                referenced: false,
            },
            ContextAttributionRecord {
                ts: now,
                source_type: "file".to_string(),
                referenced: true,
            },
        ];

        let score = social_sensitivity_from_attribution(&records, Duration::from_secs(60));
        assert!((score - 0.5).abs() < 1e-9);
    }

    #[test]
    fn knowledge_integration_rate_uses_confirmation_chains() {
        let episodes = vec![
            episode_at("task-1", 5, true),
            episode_at("task-2", 4, true),
            episode_at("task-3", 3, true),
        ];
        let records = vec![
            KnowledgeConfirmationRecord {
                created_at: Utc::now(),
                source_episodes: vec!["task-1".to_string(), "task-2".to_string()],
            },
            KnowledgeConfirmationRecord {
                created_at: Utc::now(),
                source_episodes: vec![
                    "task-1".to_string(),
                    "task-2".to_string(),
                    "task-3".to_string(),
                ],
            },
            KnowledgeConfirmationRecord {
                created_at: Utc::now(),
                source_episodes: vec!["task-1".to_string()],
            },
        ];

        let score = knowledge_integration_rate(&records, &episodes, Duration::from_secs(60));
        assert!(score > 0.0);
        assert!(score <= 1.0);
    }

    #[test]
    fn convergence_velocity_uses_agreement_across_agents() {
        let episodes = vec![
            episode_with_agent("task-1", 6, true, "agent-a"),
            episode_with_agent("task-2", 4, true, "agent-b"),
            episode_with_agent("task-3", 2, true, "agent-c"),
        ];
        let records = vec![
            KnowledgeConfirmationRecord {
                created_at: Utc::now(),
                source_episodes: vec!["task-1".to_string(), "task-2".to_string()],
            },
            KnowledgeConfirmationRecord {
                created_at: Utc::now(),
                source_episodes: vec![
                    "task-1".to_string(),
                    "task-2".to_string(),
                    "task-3".to_string(),
                ],
            },
        ];

        let score =
            convergence_velocity_from_agreement(&records, &episodes, Duration::from_secs(60));
        assert!(score > 0.0);
        assert!(score <= 1.0);
    }

    #[tokio::test]
    async fn open_under_loads_persisted_cascade_router_state() {
        let tmp = TempDir::new().unwrap();
        let learn_root = tmp.path().join(".roko").join("learn");
        let paths = LearningPaths::under(&learn_root);

        let router = CascadeRouter::new(vec![
            "claude-sonnet-4-20250514".to_string(),
            "claude-haiku-4-5-20251001".to_string(),
        ]);
        let ctx = RoutingContext {
            task_category: TaskCategory::Implementation,
            complexity: TaskComplexityBand::Standard,
            iteration: 0,
            role: roko_core::agent::AgentRole::Implementer,
            crate_familiarity: 0.5,
            has_prior_failure: false,
            affect_confidence: 0.5,
            behavioral_state: roko_core::BehavioralState::Engaged,
            thinking_level: None,
            previous_model: None,
            plan_context_tokens: None,
        };
        for _ in 0..60 {
            router.record_observation(&ctx, "claude-sonnet-4-20250514", 0.9, true);
        }
        router.save(&paths.cascade_router_json).unwrap();

        let runtime = LearningRuntime::open_under(&learn_root).await.unwrap();
        let loaded_router = runtime.cascade_router();

        assert_eq!(loaded_router.total_observations(), 60);
        assert_eq!(
            loaded_router.current_stage(),
            crate::cascade_router::CascadeStage::Confidence
        );
        let routed = loaded_router.route(&ctx);
        assert_eq!(
            routed.stage,
            crate::cascade_router::CascadeStage::Confidence
        );
    }

    #[tokio::test]
    async fn record_completed_run_persists_cascade_router_immediately() {
        let tmp = TempDir::new().unwrap();
        let learn_root = tmp.path().join(".roko").join("learn");
        let runtime = LearningRuntime::open_under(&learn_root).await.unwrap();
        let router_path = learn_root.join("cascade-router.json");
        assert!(
            !router_path.exists(),
            "router file should not exist before observation"
        );

        let mut ep = sample_episode(true);
        ep.extra
            .insert("model".to_string(), serde_json::json!("claude-sonnet-4-5"));

        let update = runtime
            .record_completed_run(CompletedRunInput::from_episode(ep))
            .await
            .unwrap();

        assert!(
            update.router_updated,
            "completed run should update cascade router"
        );
        assert!(
            router_path.exists(),
            "router file should be written after observation"
        );

        let contents = std::fs::read_to_string(&router_path).unwrap();
        let snapshot: serde_json::Value = serde_json::from_str(&contents).unwrap();
        let stats = snapshot
            .get("confidence_stats")
            .and_then(serde_json::Value::as_object)
            .expect("confidence stats should be persisted");
        let sonnet = stats
            .get("claude-sonnet-4-5")
            .and_then(serde_json::Value::as_object)
            .expect("sonnet observation should be persisted");
        assert_eq!(
            sonnet.get("trials").and_then(serde_json::Value::as_u64),
            Some(1),
            "persisted router should reflect the new observation"
        );
        assert_eq!(
            sonnet.get("successes").and_then(serde_json::Value::as_u64),
            Some(1),
            "persisted router should reflect the successful observation"
        );
    }

    #[tokio::test]
    async fn experiment_updates_static_table() {
        let tmp = TempDir::new().unwrap();
        let learn_root = tmp.path().join(".roko").join("learn");
        let paths = LearningPaths::under(&learn_root);
        let runtime = LearningRuntime::open_with_models(
            paths.clone(),
            RegressionConfig::default(),
            vec![
                "claude-sonnet-4-20250514".to_string(),
                "claude-haiku-4-5-20251001".to_string(),
            ],
        )
        .await
        .unwrap();

        let mut experiment = PromptExperiment::new(
            "model-routing-exp",
            "model-routing",
            vec![
                PromptVariant {
                    id: "sonnet".to_string(),
                    name: "Sonnet".to_string(),
                    section_name: "model-routing".to_string(),
                    content: String::new(),
                    slug: Some("claude-sonnet-4-20250514".to_string()),
                    active: true,
                },
                PromptVariant {
                    id: "haiku".to_string(),
                    name: "Haiku".to_string(),
                    section_name: "model-routing".to_string(),
                    content: String::new(),
                    slug: Some("claude-haiku-4-5-20251001".to_string()),
                    active: true,
                },
            ],
        );
        experiment.role = Some("implementer".to_string());
        experiment.min_trials_per_variant = 1;
        experiment.min_effect_size = 0.5;
        runtime.experiment_store().lock().register(experiment);

        let mut before_ctx = RoutingContext {
            task_category: TaskCategory::Implementation,
            complexity: TaskComplexityBand::Standard,
            iteration: 0,
            role: AgentRole::Implementer,
            crate_familiarity: 0.5,
            has_prior_failure: false,
            affect_confidence: 0.5,
            behavioral_state: roko_core::BehavioralState::Engaged,
            thinking_level: None,
            previous_model: None,
            plan_context_tokens: None,
        };
        assert_eq!(
            runtime.cascade_router().route(&before_ctx).primary.slug,
            "claude-sonnet-4-20250514"
        );

        let mut losing_episode = sample_episode(false);
        losing_episode.extra.insert(
            "model".to_string(),
            serde_json::json!("claude-sonnet-4-20250514"),
        );
        runtime
            .record_completed_run(CompletedRunInput {
                experiment_variant_id: Some("sonnet".to_string()),
                ..CompletedRunInput::from_episode(losing_episode)
            })
            .await
            .unwrap();

        let mut winning_episode = sample_episode(true);
        winning_episode.extra.insert(
            "model".to_string(),
            serde_json::json!("claude-haiku-4-5-20251001"),
        );
        runtime
            .record_completed_run(CompletedRunInput {
                experiment_variant_id: Some("haiku".to_string()),
                ..CompletedRunInput::from_episode(winning_episode)
            })
            .await
            .unwrap();

        before_ctx.iteration = 1;
        assert_eq!(
            runtime.cascade_router().route(&before_ctx).primary.slug,
            "claude-haiku-4-5-20251001"
        );

        let reloaded = LearningRuntime::open_with_models(
            paths,
            RegressionConfig::default(),
            vec![
                "claude-sonnet-4-20250514".to_string(),
                "claude-haiku-4-5-20251001".to_string(),
            ],
        )
        .await
        .unwrap();
        assert_eq!(
            reloaded.cascade_router().route(&before_ctx).primary.slug,
            "claude-haiku-4-5-20251001"
        );
    }

    #[tokio::test]
    async fn completed_run_updates_playbook_and_rule_outcomes() {
        let tmp = TempDir::new().unwrap();
        let paths = LearningPaths::under(tmp.path());
        let runtime = LearningRuntime::open(paths.clone(), RegressionConfig::default())
            .await
            .unwrap();

        let mut pb = crate::playbook::Playbook::new("pb-1", "goal");
        pb.steps.push(crate::playbook::PlaybookStep::new(
            0,
            "step",
            "edit_file",
            vec!["signal".to_string()],
        ));
        runtime.playbook_store.save(&pb).await.unwrap();

        let mut rule = crate::playbook_rules::Rule {
            rule_id: "r-1".to_string(),
            title: "title".to_string(),
            body: "body".to_string(),
            triggers: crate::playbook_rules::Triggers {
                tags: vec!["rust".to_string()],
                ..Default::default()
            },
            confidence: 0.5,
            validations: 0,
            contradictions: 0,
            last_applied: None,
            created_at: Utc::now(),
            source_episodes: vec![],
        };
        runtime.playbook_rules.upsert(rule.clone()).unwrap();
        runtime.playbook_rules.save().unwrap();

        let mut ep = sample_episode(false);
        ep.extra
            .insert("playbook_id".to_string(), serde_json::json!("pb-1"));
        ep.extra
            .insert("playbook_rule_id".to_string(), serde_json::json!("r-1"));
        let update = runtime
            .record_completed_run(CompletedRunInput::from_episode(ep))
            .await
            .unwrap();

        assert_eq!(update.playbook_updated, ApplyStatus::Applied);
        assert_eq!(update.playbook_rule_updated, ApplyStatus::Applied);

        let loaded_pb = runtime.playbook_store.load("pb-1").await.unwrap().unwrap();
        assert_eq!(loaded_pb.failure_count, 1);

        let rules = runtime.playbook_rules.snapshot();
        rule = rules.into_iter().find(|r| r.rule_id == "r-1").unwrap();
        assert_eq!(rule.contradictions, 1);
    }

    #[tokio::test]
    async fn completed_run_emits_regression_report_when_enough_metrics() {
        let tmp = TempDir::new().unwrap();
        let cfg = RegressionConfig {
            thresholds: RegressionThresholds {
                min_records: 2,
                pass_rate_drop: 0.1,
                cost_increase: 0.1,
                duration_increase: 0.1,
                iterations_increase: 0.1,
            },
            current_window: 2,
        };
        let runtime = LearningRuntime::open(LearningPaths::under(tmp.path()), cfg)
            .await
            .unwrap();

        // Baseline: good + cheap.
        for i in 1..=2_u32 {
            let input = CompletedRunInput::from_episode(sample_episode(true))
                .with_task_metric(sample_metric(i, true, 0.1));
            let update = runtime.record_completed_run(input).await.unwrap();
            assert!(update.regression_report.is_none());
        }

        // Current window: worse + expensive.
        let update = runtime
            .record_completed_run(
                CompletedRunInput::from_episode(sample_episode(false))
                    .with_task_metric(sample_metric(3, false, 1.0)),
            )
            .await
            .unwrap();
        assert!(update.regression_report.is_none());

        let update = runtime
            .record_completed_run(
                CompletedRunInput::from_episode(sample_episode(false))
                    .with_task_metric(sample_metric(4, false, 1.1)),
            )
            .await
            .unwrap();
        let report = update.regression_report.expect("regression report");
        assert!(report.sufficient_data);
        assert!(!report.alerts.is_empty());
    }

    #[tokio::test]
    async fn health_filters_routing() {
        let tmp = TempDir::new().unwrap();
        let runtime = LearningRuntime::open_with_models(
            LearningPaths::under(tmp.path()),
            RegressionConfig::default(),
            vec![
                "moonshot-fast".to_string(),
                "moonshot-premium".to_string(),
                "anthropic-safe".to_string(),
            ],
        )
        .await
        .unwrap();

        for _ in 0..3 {
            runtime.provider_health().record_failure("moonshot");
        }

        let all_model_slugs = runtime
            .cascade_router()
            .linucb()
            .arm_stats()
            .into_iter()
            .map(|arm| arm.slug)
            .collect::<Vec<_>>();
        let healthy_models = runtime.healthy_model_slugs(&all_model_slugs, |model_slug| {
            if model_slug.starts_with("moonshot") {
                "moonshot".to_string()
            } else {
                "anthropic".to_string()
            }
        });

        assert_eq!(healthy_models, vec!["anthropic-safe".to_string()]);

        let ctx = RoutingContext {
            task_category: TaskCategory::Implementation,
            complexity: TaskComplexityBand::Standard,
            iteration: 0,
            role: AgentRole::Implementer,
            crate_familiarity: 0.5,
            has_prior_failure: false,
            affect_confidence: 0.5,
            behavioral_state: roko_core::BehavioralState::Engaged,
            thinking_level: None,
            previous_model: None,
            plan_context_tokens: None,
        };
        let selected = runtime
            .cascade_router()
            .select_for_frequency_among(
                roko_core::OperatingFrequency::Theta,
                Some(&ctx),
                None,
                Some("Implementer"),
                &healthy_models,
            )
            .expect("theta should route to a healthy model");

        assert_eq!(selected.slug, "anthropic-safe");
    }

    #[tokio::test]
    async fn conductor_negative_feedback_records_failed_router_observation() {
        let tmp = TempDir::new().unwrap();
        let runtime = LearningRuntime::open_with_models(
            LearningPaths::under(tmp.path()),
            RegressionConfig::default(),
            vec!["claude-opus-4-6".to_string()],
        )
        .await
        .unwrap();

        let routing_context = RoutingContext {
            task_category: TaskCategory::Implementation,
            complexity: TaskComplexityBand::Standard,
            iteration: 1,
            role: AgentRole::Implementer,
            crate_familiarity: 0.5,
            has_prior_failure: false,
            affect_confidence: 0.5,
            behavioral_state: roko_core::BehavioralState::Engaged,
            thinking_level: None,
            previous_model: None,
            plan_context_tokens: None,
        };

        let recorded = runtime.record_conductor_intervention(
            &routing_context,
            "claude-opus-4-6",
            &ConductorDecision::restart("stuck-pattern", "repeated output"),
        );
        assert!(recorded);

        let stats = runtime.cascade_router().observation_snapshot();
        let opus = stats.get("claude-opus-4-6").expect("router stats");
        assert_eq!(opus.trials, 1);
        assert_eq!(opus.successes, 0);

        let reloaded = LearningRuntime::open_with_models(
            LearningPaths::under(tmp.path()),
            RegressionConfig::default(),
            vec!["claude-opus-4-6".to_string()],
        )
        .await
        .unwrap();
        let persisted = reloaded.cascade_router().observation_snapshot();
        let opus = persisted
            .get("claude-opus-4-6")
            .expect("persisted router stats");
        assert_eq!(opus.trials, 1);
        assert_eq!(opus.successes, 0);
    }

    #[tokio::test]
    async fn latency_aware_reward_prefers_faster_models() {
        let tmp = TempDir::new().unwrap();
        let runtime = LearningRuntime::open_under(tmp.path()).await.unwrap();

        let event = AgentEfficiencyEvent {
            backend: "anthropic".to_string(),
            model: "claude-opus-4-6".to_string(),
            model_used: "claude-opus-4-6".to_string(),
            wall_time_ms: 1_000,
            duration_ms: 1_000,
            output_tokens: 128,
            ..AgentEfficiencyEvent::default()
        };
        runtime.append_efficiency_event(&event).await.unwrap();

        let faster = runtime.compute_routing_reward_with_latency(
            true,
            0.25,
            1_000,
            "claude-opus-4-6",
            "anthropic",
        );
        let slower = runtime.compute_routing_reward_with_latency(
            true,
            0.25,
            60_000,
            "claude-opus-4-6",
            "anthropic",
        );

        assert!(faster > slower, "faster={faster}, slower={slower}");
    }

    #[tokio::test]
    async fn latency_aware_reward_uses_latency_registry_fallback() {
        let tmp = TempDir::new().unwrap();
        let runtime = LearningRuntime::open_under(tmp.path()).await.unwrap();

        for wall_time_ms in [10_000_u64, 20_000, 30_000] {
            let event = AgentEfficiencyEvent {
                backend: "anthropic".to_string(),
                model: "claude-opus-4-6".to_string(),
                model_used: "claude-opus-4-6".to_string(),
                wall_time_ms,
                duration_ms: wall_time_ms,
                output_tokens: 64,
                ..AgentEfficiencyEvent::default()
            };
            runtime.append_efficiency_event(&event).await.unwrap();
        }

        let reloaded = LearningRuntime::open_under(tmp.path()).await.unwrap();
        let stats = reloaded
            .latency_registry()
            .get("claude-opus-4-6", "anthropic")
            .expect("latency stats");
        assert_eq!(stats.p50_ms(), 20_000.0);

        let reward = reloaded.compute_routing_reward_with_latency(
            true,
            0.25,
            0,
            "claude-opus-4-6",
            "anthropic",
        );
        let expected = compute_routing_reward_v2(1.0, 0.25_f64 / 5.0, 20_000.0, 120_000.0);
        assert!((reward - expected).abs() < 1e-9);
    }

    #[tokio::test]
    async fn local_reward_functions_observe_and_persist_across_runs() {
        let tmp = TempDir::new().unwrap();
        let runtime = LearningRuntime::open_with_models(
            LearningPaths::under(tmp.path()),
            RegressionConfig::default(),
            vec!["claude-opus-4-6".to_string()],
        )
        .await
        .unwrap();

        // Before any observations, unknown decisions return the 0.5 prior.
        assert_eq!(runtime.local_reward_score("router", "claude-opus-4-6"), 0.5);

        // Record a successful run with model + skill metadata.
        let mut ep = sample_pattern_episode(true, "local-reward-test");
        ep.extra
            .insert("model".to_string(), serde_json::json!("claude-opus-4-6"));
        let mut input = CompletedRunInput::from_episode(ep);
        input.matched_skill_id = Some("rust-impl".to_string());
        input.playbook_rule_id = Some("rule-001".to_string());
        runtime.record_completed_run(input).await.unwrap();

        // Router subsystem should have observed the model decision.
        assert_eq!(
            runtime.local_reward_score("router", "claude-opus-4-6"),
            1.0,
            "single success should give 1.0"
        );
        // Skill subsystem should track the matched skill.
        assert_eq!(runtime.local_reward_score("skill", "rust-impl"), 1.0);
        // Playbook rule subsystem.
        assert_eq!(runtime.local_reward_score("playbook_rule", "rule-001"), 1.0);

        // Record a failed run for the same model.
        let mut ep2 = sample_pattern_episode(false, "local-reward-fail");
        ep2.extra
            .insert("model".to_string(), serde_json::json!("claude-opus-4-6"));
        ep2.success = false;
        runtime
            .record_completed_run(CompletedRunInput::from_episode(ep2))
            .await
            .unwrap();
        assert!(
            (runtime.local_reward_score("router", "claude-opus-4-6") - 0.5).abs() < 1e-9,
            "1 success + 1 failure = 0.5"
        );

        // Verify persistence: reload from disk and check scores survive.
        let reloaded = LearningRuntime::open_under(tmp.path()).await.unwrap();
        assert!(
            (reloaded.local_reward_score("router", "claude-opus-4-6") - 0.5).abs() < 1e-9,
            "persisted score should survive reload"
        );
        assert_eq!(reloaded.local_reward_score("skill", "rust-impl"), 1.0);
    }
}
