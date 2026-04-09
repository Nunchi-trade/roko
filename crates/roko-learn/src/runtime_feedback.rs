//! Runtime-facing learning orchestration helpers.
//!
//! This module provides a single integration point for CLI/orchestrator code:
//! pass one completed run, and the helper updates all configured learning
//! subsystems in a consistent order.

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex as AsyncMutex;

use roko_core::metric::TaskMetric;

use crate::cascade_router::CascadeRouter;
use crate::context_pack_cache::ContextPackCache;
use crate::costs_db::{CostRecord, CostsDb};
use crate::costs_log::CostsLog;
use crate::efficiency::AgentEfficiencyEvent;
use crate::episode_logger::{Episode, EpisodeLogger, LoggerError};
use crate::model_router::RoutingContext;
use crate::pattern_discovery::{EpisodeView, PatternMiner};
use crate::playbook::PlaybookStore;
use crate::playbook_rules::PlaybookRules;
use crate::prompt_experiment::ExperimentStore;
use crate::provider_health::ProviderHealthTracker;
use crate::regression::{RegressionReport, RegressionThresholds, detect_regressions};
use crate::skill_library::{SkillLibrary, SkillLibraryError, TemplatePatternGenerator};
use roko_core::agent::AgentRole;
use roko_core::task::{TaskCategory, TaskComplexityBand};

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
    /// Cascade router persisted observations JSON.
    pub cascade_router_json: PathBuf,
    /// Prompt experiment store JSON.
    pub experiments_json: PathBuf,
    /// Adaptive gate thresholds JSON.
    pub gate_thresholds_json: PathBuf,
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
            cascade_router_json: root.join("cascade-router.json"),
            experiments_json: root.join("experiments.json"),
            gate_thresholds_json: root.join("gate-thresholds.json"),
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
    costs_log: CostsLog,
    costs_db: CostsDb,
    provider_health: ProviderHealthTracker,
    skill_library: SkillLibrary,
    playbook_store: PlaybookStore,
    playbook_rules: PlaybookRules,
    regression: RegressionConfig,
    task_metrics: AsyncMutex<Vec<TaskMetric>>,
    pattern_miner: parking_lot::Mutex<PatternMiner>,
    cascade_router: CascadeRouter,
    context_pack_cache: ContextPackCache,
    experiment_store: parking_lot::Mutex<ExperimentStore>,
    /// Count of cascade router observations since last save.
    cascade_obs_since_save: std::sync::atomic::AtomicU32,
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

        let episode_logger = EpisodeLogger::new(&paths.episodes_jsonl);
        let costs_log = CostsLog::open_creating(&paths.costs_jsonl).await?;
        let costs_db = CostsDb::new();
        let existing_costs = costs_log.read_all().await?;
        costs_db.insert_batch(existing_costs);

        let skill_library = SkillLibrary::new(&paths.skills_json).await?;
        let playbook_store = PlaybookStore::new(&paths.playbooks_dir);
        let playbook_rules = PlaybookRules::open(&paths.playbook_rules_toml)?;
        let task_metrics = load_task_metrics(&paths.task_metrics_jsonl).await?;

        let pattern_miner = parking_lot::Mutex::new(PatternMiner::new(3, 0.5));
        let cascade_router = CascadeRouter::load_or_new(
            &paths.cascade_router_json,
            vec![
                "claude-sonnet-4-20250514".into(),
                "claude-haiku-4-5-20251001".into(),
            ],
        );
        let context_pack_cache = ContextPackCache::new(256, paths.root.join("context-cache.json"));
        let experiment_store = ExperimentStore::load_or_new(&paths.experiments_json);

        Ok(Self {
            paths,
            episode_logger,
            costs_log,
            costs_db,
            provider_health: ProviderHealthTracker::new(),
            skill_library,
            playbook_store,
            playbook_rules,
            regression,
            task_metrics: AsyncMutex::new(task_metrics),
            pattern_miner,
            cascade_router,
            context_pack_cache,
            experiment_store: parking_lot::Mutex::new(experiment_store),
            cascade_obs_since_save: std::sync::atomic::AtomicU32::new(0),
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

        let episode_logger = EpisodeLogger::new(&paths.episodes_jsonl);
        let costs_log = CostsLog::open_creating(&paths.costs_jsonl).await?;
        let costs_db = CostsDb::new();
        let existing_costs = costs_log.read_all().await?;
        costs_db.insert_batch(existing_costs);

        let skill_library = SkillLibrary::new(&paths.skills_json).await?;
        let playbook_store = PlaybookStore::new(&paths.playbooks_dir);
        let playbook_rules = PlaybookRules::open(&paths.playbook_rules_toml)?;
        let task_metrics = load_task_metrics(&paths.task_metrics_jsonl).await?;

        let pattern_miner = parking_lot::Mutex::new(PatternMiner::new(3, 0.5));
        let cascade_router = CascadeRouter::load_or_new(&paths.cascade_router_json, models);
        let context_pack_cache = ContextPackCache::new(256, paths.root.join("context-cache.json"));
        let experiment_store = ExperimentStore::load_or_new(&paths.experiments_json);

        Ok(Self {
            paths,
            episode_logger,
            costs_log,
            costs_db,
            provider_health: ProviderHealthTracker::new(),
            skill_library,
            playbook_store,
            playbook_rules,
            regression,
            task_metrics: AsyncMutex::new(task_metrics),
            pattern_miner,
            cascade_router,
            context_pack_cache,
            experiment_store: parking_lot::Mutex::new(experiment_store),
            cascade_obs_since_save: std::sync::atomic::AtomicU32::new(0),
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

    /// Borrow pattern miner (behind `parking_lot::Mutex` for `&mut` access).
    #[must_use]
    pub const fn pattern_miner(&self) -> &parking_lot::Mutex<PatternMiner> {
        &self.pattern_miner
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
        Ok(())
    }

    /// Read all persisted efficiency events from the JSONL log.
    ///
    /// Returns an empty vec if the file does not exist.
    pub async fn read_efficiency_events(
        &self,
    ) -> Result<Vec<AgentEfficiencyEvent>, LearningRuntimeError> {
        read_efficiency_events(&self.paths.efficiency_jsonl).await
    }

    /// Save cascade router observations to disk.
    pub fn save_cascade_router(&self) -> Result<(), LearningRuntimeError> {
        self.cascade_router.save(&self.paths.cascade_router_json)?;
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

        self.episode_logger.append(&input.episode).await?;
        update.episode_logged = ApplyStatus::Applied;

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
        if let Some(skill) = self.skill_library.extract(&input.episode, &generator).await {
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

        // ── Pattern mining ──────────────────────────────────────────────
        let actions = EpisodeActions::from_episode(&input.episode);
        if !actions.actions.is_empty() {
            self.pattern_miner.lock().ingest_episode(&actions);
            update.patterns_ingested = true;
        }

        // ── Cascade router observation ─────────────────────────────────
        update.router_updated = self.update_cascade_router(&input.episode);

        // Periodically save cascade router to disk (every 10 observations).
        if update.router_updated {
            let prev = self
                .cascade_obs_since_save
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if (prev + 1) % 10 == 0 {
                if let Err(e) = self.cascade_router.save(&self.paths.cascade_router_json) {
                    eprintln!("[learn] cascade router save failed: {e}");
                }
            }
        }

        // ── Prompt experiment outcome ────────────────────────────────────
        if let Some(ref variant_id) = input.experiment_variant_id {
            let mut store = self.experiment_store.lock();
            store.record_outcome(variant_id, input.episode.success);
            if let Err(e) = store.save(&self.paths.experiments_json) {
                eprintln!("[learn] experiment store save failed: {e}");
            }
        }

        Ok(update)
    }

    /// Update the cascade router from episode metadata if role + model are available.
    fn update_cascade_router(&self, episode: &Episode) -> bool {
        let role_str = extra_string(episode, "role");
        let model_slug = extra_string(episode, "model");
        let (Some(role_raw), Some(slug)) = (role_str, model_slug) else {
            return false;
        };
        let role_json = format!("\"{role_raw}\"");
        let Ok(role) = serde_json::from_str::<AgentRole>(&role_json) else {
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

        let ctx = RoutingContext {
            task_category,
            complexity,
            iteration: 0,
            role,
            crate_familiarity: 0.5,
            has_prior_failure: !episode.success,
        };
        let reward = if episode.success { 1.0 } else { 0.0 };
        self.cascade_router
            .record_observation(&ctx, &slug, reward, episode.success);
        true
    }
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

#[cfg(test)]
mod tests {
    use super::*;
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

    #[tokio::test]
    async fn completed_run_updates_episode_cost_provider_and_skill() {
        let tmp = TempDir::new().unwrap();
        let runtime = LearningRuntime::open_under(tmp.path()).await.unwrap();

        let input = CompletedRunInput::from_episode(sample_episode(true));
        let update = runtime.record_completed_run(input).await.unwrap();

        assert_eq!(update.episode_logged, ApplyStatus::Applied);
        assert_eq!(update.cost_logged, ApplyStatus::Applied);
        assert_eq!(update.provider_updated, ApplyStatus::Applied);
        assert!(update.extracted_skill_id.is_some());
        assert_eq!(runtime.costs_db().len(), 1);
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
}
