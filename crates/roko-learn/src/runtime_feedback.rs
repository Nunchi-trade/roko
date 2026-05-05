//! Runtime-facing learning orchestration helpers.
//!
//! This module provides a single integration point for CLI/orchestrator code:
//! pass one completed run, and the helper updates all configured learning
//! subsystems in a consistent order.

use std::collections::HashMap;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
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
use crate::post_gate_reflection::{
    PostGateReflectionStore, ReflectionInput, ReflectionPromotionConfig,
};
use crate::prompt_experiment::{ExperimentStatus, ExperimentStore, PromptExperiment};
use crate::provider_health::ProviderHealthTracker;
use crate::provider_model_outcome::{
    ProviderModelOutcomeRecord, ProviderModelOutcomeStore, ProviderModelPassRateReport,
    read_provider_model_outcomes, summarize_provider_model_outcomes,
};
use crate::regression::{RegressionReport, RegressionThresholds, detect_regressions};
use crate::section_effect::SectionEffectivenessRegistry;
use crate::skill_library::{SkillLibrary, SkillLibraryError, TemplatePatternGenerator};
use roko_core::ConductorDecision;
use roko_core::DaimonPolicy;
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct GateCounts {
    passed: u64,
    failed: u64,
    skipped: u64,
}

impl GateCounts {
    fn executed(self) -> u64 {
        self.passed + self.failed
    }

    fn pass_rate(self) -> f64 {
        let executed = self.executed();
        if executed == 0 {
            0.0
        } else {
            self.passed as f64 / executed as f64
        }
    }

    fn summary(self) -> String {
        format!(
            "{} passed, {} failed, {} skipped",
            self.passed, self.failed, self.skipped
        )
    }

    fn has_only_skipped(self) -> bool {
        self.executed() == 0 && self.skipped > 0
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
    /// Append-only normalized efficiency summaries JSONL file.
    pub efficiency_summaries_jsonl: PathBuf,
    /// Append-only gate outcome JSONL file.
    pub gate_outcomes_jsonl: PathBuf,
    /// Append-only retry outcome JSONL file.
    pub retry_outcomes_jsonl: PathBuf,
    /// Append-only knowledge seed JSONL file for neuro ingestion.
    pub knowledge_seeds_jsonl: PathBuf,
    /// Persisted latency registry snapshot.
    pub latency_stats_json: PathBuf,
    /// Append-only C-Factor history JSONL file.
    pub cfactor_jsonl: PathBuf,
    /// Cascade router persisted observations JSON.
    pub cascade_router_json: PathBuf,
    /// Prompt experiment store JSON.
    pub experiments_json: PathBuf,
    /// Operator-facing summary of concluded experiment winners.
    pub experiment_winners_json: PathBuf,
    /// Adaptive gate thresholds JSON.
    pub gate_thresholds_json: PathBuf,
    /// Per-subsystem local reward functions JSON.
    pub local_rewards_json: PathBuf,
    /// Learned prompt section effectiveness snapshot.
    pub section_effects_json: PathBuf,
    /// Structured post-gate reflection records and candidates.
    pub post_gate_reflections_json: PathBuf,
    /// Append-only provider/model outcome telemetry for future bandits.
    pub provider_model_outcomes_jsonl: PathBuf,
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
            efficiency_summaries_jsonl: root.join("efficiency-summaries.jsonl"),
            gate_outcomes_jsonl: root.join("gate-outcomes.jsonl"),
            retry_outcomes_jsonl: root.join("retry-outcomes.jsonl"),
            knowledge_seeds_jsonl: root.join("knowledge-seeds.jsonl"),
            latency_stats_json: root.join("latency-stats.json"),
            cfactor_jsonl: root.join("c-factor.jsonl"),
            cascade_router_json: root.join("cascade-router.json"),
            experiments_json: root.join("experiments.json"),
            experiment_winners_json: root.join("experiment-winners.json"),
            gate_thresholds_json: root.join("gate-thresholds.json"),
            local_rewards_json: root.join("local-rewards.json"),
            section_effects_json: root.join("section-effects.json"),
            post_gate_reflections_json: root.join("post-gate-reflections.json"),
            provider_model_outcomes_jsonl: root.join("provider-model-outcomes.jsonl"),
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
    /// Whether a post-gate reflection record was persisted.
    pub reflection_recorded: ApplyStatus,
    /// Whether a reflection-derived playbook candidate was updated.
    pub reflection_candidate_updated: ApplyStatus,
    /// Whether a provider/model outcome record was persisted.
    pub provider_model_outcome_recorded: ApplyStatus,
    /// Whether a normalized efficiency summary was persisted.
    pub efficiency_summary_recorded: ApplyStatus,
    /// Number of gate outcome records persisted.
    pub gate_outcomes_recorded: usize,
    /// Whether a retry outcome record was persisted.
    pub retry_outcome_recorded: ApplyStatus,
    /// Whether a knowledge seed was persisted.
    pub knowledge_seed_recorded: ApplyStatus,
}

/// Current schema version for runtime feedback JSONL records.
pub const RUNTIME_FEEDBACK_SCHEMA_VERSION: u32 = 1;

/// Granularity represented by an efficiency summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EfficiencyScope {
    /// One provider/model turn.
    Turn,
    /// One task attempt, typically closed by a gate result.
    Task,
    /// One whole runner invocation or plan run.
    Run,
}

impl Default for EfficiencyScope {
    fn default() -> Self {
        Self::Task
    }
}

/// Normalized cost/token/latency summary for query surfaces.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EfficiencySummaryRecord {
    /// JSON schema version.
    pub schema_version: u32,
    /// ISO-8601 timestamp for the observation.
    pub timestamp: String,
    /// Summary granularity.
    pub scope: EfficiencyScope,
    /// Optional runner/session/run identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Stable episode identifier when this summary came from an episode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_id: Option<String>,
    /// Plan identifier.
    #[serde(default)]
    pub plan_id: String,
    /// Task identifier.
    #[serde(default)]
    pub task_id: String,
    /// Agent identifier.
    #[serde(default)]
    pub agent_id: String,
    /// Agent role/profile label.
    #[serde(default)]
    pub role: String,
    /// Provider/backend identifier.
    #[serde(default)]
    pub provider: String,
    /// Model slug.
    #[serde(default)]
    pub model: String,
    /// Retry/turn iteration number.
    #[serde(default)]
    pub iteration: u32,
    /// Input tokens.
    #[serde(default)]
    pub input_tokens: u64,
    /// Output tokens.
    #[serde(default)]
    pub output_tokens: u64,
    /// Reasoning/thinking tokens when available.
    #[serde(default)]
    pub reasoning_tokens: u64,
    /// Cache-read tokens.
    #[serde(default)]
    pub cache_read_tokens: u64,
    /// Cache-write tokens.
    #[serde(default)]
    pub cache_write_tokens: u64,
    /// Total input plus output tokens.
    #[serde(default)]
    pub total_tokens: u64,
    /// Observed cost in USD.
    #[serde(default)]
    pub cost_usd: f64,
    /// Estimated cost without cache discount.
    #[serde(default)]
    pub cost_usd_without_cache: f64,
    /// Cache hit rate in `[0.0, 1.0]`.
    #[serde(default)]
    pub cache_hit_rate: f64,
    /// Wall-clock duration in milliseconds.
    #[serde(default)]
    pub duration_ms: u64,
    /// Time to first token in milliseconds.
    #[serde(default)]
    pub time_to_first_token_ms: u64,
    /// Number of tools exposed to the agent.
    #[serde(default)]
    pub tools_available: u32,
    /// Number of tools used by the agent.
    #[serde(default)]
    pub tools_used: u32,
    /// Number of tool calls observed.
    #[serde(default)]
    pub tool_calls: u32,
    /// Whether the closing gate passed, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_passed: Option<bool>,
    /// Outcome label such as `success`, `failure`, or provider finish reason.
    #[serde(default)]
    pub outcome: String,
    /// Number of prompt sections represented in this summary.
    #[serde(default)]
    pub prompt_section_count: u32,
    /// Total prompt tokens represented in this summary.
    #[serde(default)]
    pub total_prompt_tokens: u64,
    /// Extra forward-compatible metadata.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub metadata: serde_json::Value,
}

/// One durable gate outcome emitted by the runner or derived from an episode.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GateOutcomeRecord {
    /// JSON schema version.
    pub schema_version: u32,
    /// ISO-8601 timestamp for the observation.
    pub timestamp: String,
    /// Optional runner/session/run identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Stable episode identifier when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_id: Option<String>,
    /// Plan identifier.
    #[serde(default)]
    pub plan_id: String,
    /// Task identifier.
    #[serde(default)]
    pub task_id: String,
    /// Gate identifier, such as `compile`, `clippy`, or `test`.
    #[serde(default)]
    pub gate_name: String,
    /// Gate family or effect kind, such as `gate` or `plan_verify`.
    #[serde(default)]
    pub gate_kind: String,
    /// Gate rung number.
    #[serde(default)]
    pub rung: u32,
    /// Whether the gate passed.
    #[serde(default)]
    pub passed: bool,
    /// Optional numeric score.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    /// Gate duration in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Retry attempt/iteration associated with this gate.
    #[serde(default)]
    pub attempt: u32,
    /// Runner-level failure classification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<String>,
    /// Short error digest/signature, never raw gate output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_digest: Option<String>,
    /// Provider/backend identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Model slug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Short human-readable summary.
    #[serde(default)]
    pub summary: String,
    /// Extra forward-compatible metadata.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub metadata: serde_json::Value,
}

/// Retry lifecycle status emitted by the runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryOutcomeStatus {
    /// A retry was scheduled but has not started yet.
    Scheduled,
    /// A retry attempt started.
    Started,
    /// A retry eventually passed its terminal gate.
    Succeeded,
    /// Retry budget was exhausted.
    Exhausted,
    /// Retry was skipped because the failure was non-retryable.
    NotRetryable,
    /// Retry was cancelled by operator/runtime shutdown.
    Cancelled,
}

impl Default for RetryOutcomeStatus {
    fn default() -> Self {
        Self::Scheduled
    }
}

/// One retry-policy outcome, append-only and queryable by plan/task.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryOutcomeRecord {
    /// JSON schema version.
    pub schema_version: u32,
    /// ISO-8601 timestamp for the observation.
    pub timestamp: String,
    /// Optional runner/session/run identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Stable episode identifier when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_id: Option<String>,
    /// Plan identifier.
    #[serde(default)]
    pub plan_id: String,
    /// Task identifier.
    #[serde(default)]
    pub task_id: String,
    /// Gate identifier that triggered the retry decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_name: Option<String>,
    /// Attempt number after the decision.
    #[serde(default)]
    pub attempt: u32,
    /// Configured maximum attempts when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<u32>,
    /// Retry status.
    pub status: RetryOutcomeStatus,
    /// Whether the triggering failure was retryable.
    #[serde(default)]
    pub retryable: bool,
    /// Runner-level failure classification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<String>,
    /// Cooldown before next retry, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_ms: Option<u64>,
    /// Provider/backend identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Model slug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Short reason for the decision.
    #[serde(default)]
    pub reason: String,
    /// Next runner action, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    /// Extra forward-compatible metadata.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub metadata: serde_json::Value,
}

/// Evidence item supporting a knowledge seed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeSeedEvidence {
    /// Evidence source type, such as `episode` or `gate`.
    pub source_type: String,
    /// Stable source identifier.
    pub source_id: String,
    /// Outcome label associated with the evidence.
    pub outcome: String,
    /// Evidence weight in `[0.0, 1.0]`.
    pub weight: f64,
}

/// Lightweight, dependency-free knowledge candidate emitted by runtime feedback.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeSeedRecord {
    /// JSON schema version.
    pub schema_version: u32,
    /// Stable deterministic seed id.
    pub seed_id: String,
    /// ISO-8601 creation timestamp.
    pub created_at: String,
    /// Knowledge kind label compatible with `roko-neuro` (`insight`, `warning`, etc.).
    pub kind: String,
    /// Candidate knowledge content.
    pub content: String,
    /// Starting confidence in `[0.0, 1.0]`.
    pub confidence: f64,
    /// Signed retrieval weight seed.
    pub confidence_weight: f64,
    /// Episode ids that support the seed.
    #[serde(default)]
    pub source_episodes: Vec<String>,
    /// Source model when the seed may be model-specific.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_model: Option<String>,
    /// Generality across model families (`1.0` = fully general).
    pub model_generality: f64,
    /// Topic tags for retrieval and admission filtering.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Plan identifier.
    #[serde(default)]
    pub plan_id: String,
    /// Task identifier.
    #[serde(default)]
    pub task_id: String,
    /// Evidence that caused this seed to be emitted.
    #[serde(default)]
    pub evidence: Vec<KnowledgeSeedEvidence>,
    /// Extra forward-compatible metadata.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub metadata: serde_json::Value,
}

/// Normalized runner event accepted by the runtime feedback facade.
#[derive(Debug, Clone)]
pub enum RunnerFeedbackEvent {
    /// A completed run should be persisted and fanned out to all derived feedback logs.
    CompletedRun {
        /// Completed-run input.
        input: Box<CompletedRunInput>,
    },
    /// A raw episode should be appended and projected into derived feedback logs.
    Episode {
        /// Episode record.
        episode: Box<Episode>,
    },
    /// A raw efficiency event should be appended and projected into summaries.
    EfficiencyEvent {
        /// Efficiency event.
        event: AgentEfficiencyEvent,
        /// Summary scope for the derived record.
        scope: EfficiencyScope,
    },
    /// A provider/model outcome should be appended directly.
    ProviderModelOutcome {
        /// Provider/model outcome record.
        outcome: ProviderModelOutcomeRecord,
    },
    /// A normalized efficiency summary should be appended directly.
    EfficiencySummary {
        /// Efficiency summary record.
        summary: EfficiencySummaryRecord,
    },
    /// A gate outcome should be appended directly.
    GateOutcome {
        /// Gate outcome record.
        outcome: GateOutcomeRecord,
    },
    /// A retry outcome should be appended directly.
    RetryOutcome {
        /// Retry outcome record.
        outcome: RetryOutcomeRecord,
    },
    /// A knowledge seed should be appended directly.
    KnowledgeSeed {
        /// Knowledge seed record.
        seed: KnowledgeSeedRecord,
    },
}

/// Opaque artifact validation payload carried alongside generation outcomes.
pub type ArtifactValidationReport = serde_json::Value;

/// Outcome of a generation operation, distinguishing process from artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationOutcome {
    /// Whether the agent process completed without error.
    pub process_success: bool,
    /// Whether the generated artifact passes grounding validation.
    pub artifact_valid: bool,
    /// Validation report (if validation ran).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_report: Option<ArtifactValidationReport>,
}

impl GenerationOutcome {
    /// True only when both process succeeded AND artifact is valid.
    #[must_use]
    pub fn fully_successful(&self) -> bool {
        self.process_success && self.artifact_valid
    }

    /// Status string for display and logging.
    #[must_use]
    pub fn status_label(&self) -> &'static str {
        match (self.process_success, self.artifact_valid) {
            (true, true) => "success",
            (true, false) => "partial_success",
            (false, _) => "failure",
        }
    }
}

/// Counts of append-only records written by a feedback facade call.
#[derive(Debug, Clone, Default)]
pub struct RuntimeFeedbackWrite {
    /// Completed-run update when a completed run was recorded.
    pub learning_update: Option<LearningUpdate>,
    /// Number of raw efficiency events appended.
    pub efficiency_events: usize,
    /// Number of provider/model outcomes appended directly by the facade.
    pub provider_model_outcomes: usize,
    /// Number of efficiency summaries appended.
    pub efficiency_summaries: usize,
    /// Number of gate outcomes appended.
    pub gate_outcomes: usize,
    /// Number of retry outcomes appended.
    pub retry_outcomes: usize,
    /// Number of knowledge seeds appended.
    pub knowledge_seeds: usize,
    /// Whether an episode was appended directly by the facade.
    pub episode_appended: bool,
}

impl RuntimeFeedbackWrite {
    fn merge(&mut self, other: Self) {
        self.efficiency_events += other.efficiency_events;
        self.provider_model_outcomes += other.provider_model_outcomes;
        self.efficiency_summaries += other.efficiency_summaries;
        self.gate_outcomes += other.gate_outcomes;
        self.retry_outcomes += other.retry_outcomes;
        self.knowledge_seeds += other.knowledge_seeds;
        self.episode_appended |= other.episode_appended;
        if self.learning_update.is_none() {
            self.learning_update = other.learning_update;
        }
    }
}

/// Query filters for append-only runtime feedback logs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeFeedbackQuery {
    /// Filter by plan id.
    pub plan_id: Option<String>,
    /// Filter by task id.
    pub task_id: Option<String>,
    /// Filter by episode id.
    pub episode_id: Option<String>,
    /// Filter by provider/backend.
    pub provider: Option<String>,
    /// Filter by model slug.
    pub model: Option<String>,
    /// Keep only the latest N records after filtering.
    pub limit: Option<usize>,
}

/// Query result spanning all canonical runtime feedback streams.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuntimeFeedbackSnapshot {
    /// Episode records.
    pub episodes: Vec<Episode>,
    /// Provider/model outcome records.
    pub provider_model_outcomes: Vec<ProviderModelOutcomeRecord>,
    /// Efficiency summary records.
    pub efficiency_summaries: Vec<EfficiencySummaryRecord>,
    /// Gate outcome records.
    pub gate_outcomes: Vec<GateOutcomeRecord>,
    /// Retry outcome records.
    pub retry_outcomes: Vec<RetryOutcomeRecord>,
    /// Knowledge seed records.
    pub knowledge_seeds: Vec<KnowledgeSeedRecord>,
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

impl EfficiencySummaryRecord {
    /// Build a task-level summary from a completed episode.
    #[must_use]
    pub fn from_episode(episode: &Episode) -> Self {
        let provider = episode_provider(episode);
        let model = episode_model(episode);
        let total_tokens = if episode.tokens_used > 0 {
            episode.tokens_used
        } else {
            episode
                .usage
                .input_tokens
                .saturating_add(episode.usage.output_tokens)
        };
        let prompt_section_count = prompt_section_count_from_episode(episode);
        let gate_counts = gate_counts_from_episode(episode);
        let has_only_skipped = gate_counts.is_some_and(GateCounts::has_only_skipped);

        Self {
            schema_version: RUNTIME_FEEDBACK_SCHEMA_VERSION,
            timestamp: episode.completed_at.to_rfc3339(),
            scope: EfficiencyScope::Task,
            run_id: episode_run_id(episode),
            episode_id: Some(episode_source_id(episode).to_string()),
            plan_id: extra_string(episode, "plan_id").unwrap_or_default(),
            task_id: episode.task_id.clone(),
            agent_id: episode.agent_id.clone(),
            role: episode_role(episode),
            provider,
            model,
            iteration: extra_u64(episode, "iteration")
                .or_else(|| extra_u64(episode, "retry_count"))
                .unwrap_or(0)
                .min(u64::from(u32::MAX)) as u32,
            input_tokens: episode.usage.input_tokens,
            output_tokens: episode.usage.output_tokens,
            reasoning_tokens: extra_u64(episode, "reasoning_tokens").unwrap_or(0),
            cache_read_tokens: episode.usage.cache_read_tokens,
            cache_write_tokens: episode.usage.cache_write_tokens,
            total_tokens,
            cost_usd: episode.usage.cost_usd,
            cost_usd_without_cache: episode.usage.cost_usd_without_cache,
            cache_hit_rate: ratio_u64(episode.usage.cache_read_tokens, episode.usage.input_tokens),
            duration_ms: episode.usage.wall_ms,
            time_to_first_token_ms: extra_u64(episode, "time_to_first_token_ms").unwrap_or(0),
            tools_available: extra_u64(episode, "tools_available")
                .unwrap_or(0)
                .min(u64::from(u32::MAX)) as u32,
            tools_used: extra_u64(episode, "tools_used")
                .unwrap_or(episode.external_actions.len() as u64)
                .min(u64::from(u32::MAX)) as u32,
            tool_calls: episode.external_actions.len().min(u32::MAX as usize) as u32,
            gate_passed: if has_only_skipped {
                None
            } else {
                Some(episode.success)
            },
            outcome: if has_only_skipped {
                extra_string(episode, "provider_model_outcome_status")
                    .unwrap_or_else(|| "blocked".to_string())
            } else if episode.success {
                "success".to_string()
            } else {
                episode
                    .failure_reason
                    .clone()
                    .unwrap_or_else(|| "failure".to_string())
            },
            prompt_section_count,
            total_prompt_tokens: extra_u64(episode, "total_prompt_tokens").unwrap_or(0),
            metadata: serde_json::json!({
                "source": "episode",
                "kind": episode.kind.clone(),
                "trigger_kind": episode.trigger_kind.clone(),
                "gate_count": episode.gate_verdicts.len(),
                "gate_summary": gate_counts.map(|counts| counts.summary()),
                "gate_pass_rate": gate_counts.map(|counts| counts.pass_rate()),
                "gate_counts": gate_counts.map(|counts| serde_json::json!({
                    "passed": counts.passed,
                    "failed": counts.failed,
                    "skipped": counts.skipped,
                    "executed": counts.executed(),
                    "summary": counts.summary(),
                    "pass_rate": counts.pass_rate(),
                })),
                "gates_passed": gate_counts.map(|counts| counts.passed),
                "gates_failed": gate_counts.map(|counts| counts.failed),
                "gates_skipped": gate_counts.map(|counts| counts.skipped),
                "gates_executed": gate_counts.map(|counts| counts.executed()),
                "provider_model_outcome_status": extra_string(episode, "provider_model_outcome_status"),
            }),
        }
    }

    /// Build a summary from an existing efficiency event.
    #[must_use]
    pub fn from_efficiency_event(event: &AgentEfficiencyEvent, scope: EfficiencyScope) -> Self {
        Self {
            schema_version: RUNTIME_FEEDBACK_SCHEMA_VERSION,
            timestamp: if event.timestamp.trim().is_empty() {
                Utc::now().to_rfc3339()
            } else {
                event.timestamp.clone()
            },
            scope,
            run_id: non_empty_string(event.plan_id.as_str()),
            episode_id: None,
            plan_id: event.plan_id.clone(),
            task_id: event.task_id.clone(),
            agent_id: event.agent_id.clone(),
            role: event.role.clone(),
            provider: event.backend.clone(),
            model: latency_model_slug(event).to_string(),
            iteration: event.iteration,
            input_tokens: event.input_tokens,
            output_tokens: event.output_tokens,
            reasoning_tokens: event.reasoning_tokens,
            cache_read_tokens: event.cache_read_tokens,
            cache_write_tokens: event.cache_write_tokens,
            total_tokens: event.total_tokens(),
            cost_usd: event.cost_usd,
            cost_usd_without_cache: event.cost_usd_without_cache,
            cache_hit_rate: event.cache_hit_rate(),
            duration_ms: event.duration_ms.max(event.wall_time_ms),
            time_to_first_token_ms: event.time_to_first_token_ms,
            tools_available: event.tools_available,
            tools_used: event.tools_used,
            tool_calls: event.tool_calls.len().min(u32::MAX as usize) as u32,
            gate_passed: Some(event.gate_passed),
            outcome: if event.outcome.trim().is_empty() {
                if event.gate_passed {
                    "success".to_string()
                } else {
                    "failure".to_string()
                }
            } else {
                event.outcome.clone()
            },
            prompt_section_count: event.prompt_sections.len().min(u32::MAX as usize) as u32,
            total_prompt_tokens: event.total_prompt_tokens,
            metadata: serde_json::json!({
                "source": "efficiency_event",
                "frequency": event.frequency,
                "strategy_attempted": event.strategy_attempted.clone(),
                "gate_errors": event.gate_errors.clone(),
            }),
        }
    }
}

impl GateOutcomeRecord {
    /// Build gate outcome records from an episode's gate verdicts.
    #[must_use]
    pub fn from_episode(episode: &Episode) -> Vec<Self> {
        let provider = non_empty_string(episode_provider(episode).as_str());
        let model = non_empty_string(episode_model(episode).as_str());
        let plan_id = extra_string(episode, "plan_id").unwrap_or_default();
        let run_id = episode_run_id(episode);
        let episode_id = Some(episode_source_id(episode).to_string());
        let attempt = extra_u64(episode, "iteration")
            .or_else(|| extra_u64(episode, "retry_count"))
            .unwrap_or(0)
            .min(u64::from(u32::MAX)) as u32;
        let duration_ms = nonzero_u64(extra_u64(episode, "gate_duration_ms").unwrap_or(0));
        let gate_counts = gate_counts_from_episode(episode);

        episode
            .gate_verdicts
            .iter()
            .enumerate()
            .map(|(idx, verdict)| Self {
                schema_version: RUNTIME_FEEDBACK_SCHEMA_VERSION,
                timestamp: episode.completed_at.to_rfc3339(),
                run_id: run_id.clone(),
                episode_id: episode_id.clone(),
                plan_id: plan_id.clone(),
                task_id: episode.task_id.clone(),
                gate_name: verdict.gate.clone(),
                gate_kind: extra_string(episode, "gate_kind").unwrap_or_else(|| "gate".into()),
                rung: extra_u64(episode, "rung")
                    .unwrap_or(idx as u64)
                    .min(u64::from(u32::MAX)) as u32,
                passed: verdict.passed,
                score: extra_f64(episode, "gate_score").map(|score| score as f32),
                duration_ms,
                attempt,
                failure_kind: extra_string(episode, "failure_kind"),
                error_digest: verdict.signature.clone(),
                provider: provider.clone(),
                model: model.clone(),
                summary: verdict.signature.clone().unwrap_or_default(),
                metadata: serde_json::json!({
                    "source": "episode",
                    "episode_kind": episode.kind.clone(),
                    "gate_counts": gate_counts.map(|counts| serde_json::json!({
                        "passed": counts.passed,
                        "failed": counts.failed,
                        "skipped": counts.skipped,
                        "executed": counts.executed(),
                        "summary": counts.summary(),
                        "pass_rate": counts.pass_rate(),
                    })),
                    "gates_passed": gate_counts.map(|counts| counts.passed),
                    "gates_failed": gate_counts.map(|counts| counts.failed),
                    "gates_skipped": gate_counts.map(|counts| counts.skipped),
                    "gates_executed": gate_counts.map(|counts| counts.executed()),
                    "gate_summary": gate_counts.map(|counts| counts.summary()),
                    "gate_pass_rate": gate_counts.map(|counts| counts.pass_rate()),
                }),
            })
            .collect()
    }
}

impl RetryOutcomeRecord {
    /// Build a retry outcome from episode metadata when the runner supplied retry fields.
    #[must_use]
    pub fn from_episode(episode: &Episode) -> Option<Self> {
        let status = retry_status_from_episode(episode)?;
        let provider = non_empty_string(episode_provider(episode).as_str());
        let model = non_empty_string(episode_model(episode).as_str());

        Some(Self {
            schema_version: RUNTIME_FEEDBACK_SCHEMA_VERSION,
            timestamp: episode.completed_at.to_rfc3339(),
            run_id: episode_run_id(episode),
            episode_id: Some(episode_source_id(episode).to_string()),
            plan_id: extra_string(episode, "plan_id").unwrap_or_default(),
            task_id: episode.task_id.clone(),
            gate_name: extra_string(episode, "gate_name").or_else(|| {
                episode
                    .gate_verdicts
                    .iter()
                    .find(|verdict| !verdict.passed)
                    .map(|verdict| verdict.gate.clone())
            }),
            attempt: extra_u64(episode, "retry_attempt")
                .or_else(|| extra_u64(episode, "iteration"))
                .unwrap_or(0)
                .min(u64::from(u32::MAX)) as u32,
            max_attempts: extra_u64(episode, "max_retries")
                .map(|value| value.min(u64::from(u32::MAX)) as u32),
            status,
            retryable: extra_bool(episode, "retryable").unwrap_or(matches!(
                status,
                RetryOutcomeStatus::Scheduled
                    | RetryOutcomeStatus::Started
                    | RetryOutcomeStatus::Succeeded
            )),
            failure_kind: extra_string(episode, "failure_kind"),
            cooldown_ms: extra_u64(episode, "retry_cooldown_ms"),
            provider,
            model,
            reason: extra_string(episode, "retry_reason")
                .or_else(|| episode.failure_reason.clone())
                .unwrap_or_default(),
            next_action: extra_string(episode, "retry_next_action"),
            metadata: serde_json::json!({
                "source": "episode",
                "episode_success": episode.success,
            }),
        })
    }
}

impl KnowledgeSeedRecord {
    /// Build a deterministic knowledge seed from a successful episode.
    #[must_use]
    pub fn from_successful_episode(episode: &Episode) -> Option<Self> {
        if !episode.success
            || gate_counts_from_episode(episode).is_some_and(GateCounts::has_only_skipped)
        {
            return None;
        }

        let source_id = episode_source_id(episode).to_string();
        let plan_id = extra_string(episode, "plan_id").unwrap_or_default();
        let task_category = extra_string(episode, "task_category")
            .unwrap_or_else(|| episode.trigger_kind.clone())
            .trim()
            .to_string();
        let role = episode_role(episode);
        let provider = episode_provider(episode);
        let model = episode_model(episode);
        let gate_names = episode
            .gate_verdicts
            .iter()
            .map(|verdict| verdict.gate.clone())
            .filter(|gate| !gate.trim().is_empty())
            .collect::<Vec<_>>();
        let files = extra_string_vec(episode, "files")
            .or_else(|| extra_string_vec(episode, "files_changed"))
            .unwrap_or_default();
        let gates_label = if gate_names.is_empty() {
            "terminal success".to_string()
        } else {
            gate_names.join(", ")
        };
        let files_label = if files.is_empty() {
            "no file scope recorded".to_string()
        } else {
            files.join(", ")
        };
        let task_label = if episode.task_id.trim().is_empty() {
            "unknown task"
        } else {
            episode.task_id.as_str()
        };
        let content = format!(
            "Successful {task_category} task {task_label} used role {role}, provider {provider}, model {model}, and passed {gates_label}. File scope: {files_label}."
        );
        let confidence = (0.70
            + (gate_names.len().min(5) as f64 * 0.025)
            + if model.is_empty() { 0.0 } else { 0.02 })
        .clamp(0.70, 0.85);
        let kind = if gate_names.len() >= 2 {
            "strategy_fragment"
        } else {
            "insight"
        }
        .to_string();
        let seed_id = format!(
            "ks-{}",
            stable_hash_hex(&[
                kind.as_str(),
                source_id.as_str(),
                episode.task_id.as_str(),
                model.as_str(),
                content.as_str(),
            ])
        );
        let mut tags = extra_string_vec(episode, "task_tags").unwrap_or_default();
        tags.extend([
            "runtime-success".to_string(),
            task_category.clone(),
            role.clone(),
            provider.clone(),
        ]);
        tags.retain(|tag| !tag.trim().is_empty());
        for tag in tags.iter_mut() {
            *tag = tag.trim().to_ascii_lowercase();
        }
        tags.sort();
        tags.dedup();

        Some(Self {
            schema_version: RUNTIME_FEEDBACK_SCHEMA_VERSION,
            seed_id,
            created_at: episode.completed_at.to_rfc3339(),
            kind,
            content,
            confidence,
            confidence_weight: confidence,
            source_episodes: vec![source_id.clone()],
            source_model: non_empty_string(model.as_str()),
            model_generality: if model.is_empty() { 0.75 } else { 0.35 },
            tags,
            plan_id,
            task_id: episode.task_id.clone(),
            evidence: vec![KnowledgeSeedEvidence {
                source_type: "episode".to_string(),
                source_id,
                outcome: "success".to_string(),
                weight: confidence,
            }],
            metadata: serde_json::json!({
                "provider": provider,
                "model": model,
                "role": role,
                "task_category": task_category,
                "gate_names": gate_names,
                "files": files,
                "tokens": {
                    "input": episode.usage.input_tokens,
                    "output": episode.usage.output_tokens,
                    "cache_read": episode.usage.cache_read_tokens,
                    "cache_write": episode.usage.cache_write_tokens,
                },
                "cost_usd": episode.usage.cost_usd,
            }),
        })
    }
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
    section_effectiveness: parking_lot::Mutex<SectionEffectivenessRegistry>,
    provider_model_outcomes: ProviderModelOutcomeStore,
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
        let section_effectiveness =
            SectionEffectivenessRegistry::load_or_new(&paths.section_effects_json);
        let provider_model_outcomes =
            ProviderModelOutcomeStore::open_creating(&paths.provider_model_outcomes_jsonl).await?;

        sync_experiment_winner_artifact(&paths.experiment_winners_json, &experiment_store)?;

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
            section_effectiveness: parking_lot::Mutex::new(section_effectiveness),
            provider_model_outcomes,
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
        let section_effectiveness =
            SectionEffectivenessRegistry::load_or_new(&paths.section_effects_json);
        let provider_model_outcomes =
            ProviderModelOutcomeStore::open_creating(&paths.provider_model_outcomes_jsonl).await?;

        sync_experiment_winner_artifact(&paths.experiment_winners_json, &experiment_store)?;

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
            section_effectiveness: parking_lot::Mutex::new(section_effectiveness),
            provider_model_outcomes,
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

    /// Open a runtime at `root` with a custom model list for the cascade router.
    ///
    /// # Errors
    ///
    /// Returns an error if persistence files cannot be read/initialized.
    pub async fn open_under_with_models(
        root: impl Into<PathBuf>,
        models: Vec<String>,
    ) -> Result<Self, LearningRuntimeError> {
        Self::open_with_models(
            LearningPaths::under(root),
            RegressionConfig::default(),
            models,
        )
        .await
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
            .filter_arms_or_best(all_model_slugs, provider_of);
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

    /// Inject config-sourced model tiers into the cascade router.
    ///
    /// Call this after construction when the `RokoConfig` is available,
    /// so the router uses explicit `tier` fields from `roko.toml` instead
    /// of substring heuristics.
    pub fn set_model_tiers(
        &mut self,
        models: &indexmap::IndexMap<String, roko_core::config::ModelProfile>,
    ) {
        self.cascade_router.set_model_tiers(models);
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

    /// Return a snapshot of the learned section-effectiveness registry.
    #[must_use]
    pub fn section_effectiveness_snapshot(&self) -> SectionEffectivenessRegistry {
        self.section_effectiveness.lock().clone()
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

    /// Consume one normalized runner event and append all canonical feedback records.
    ///
    /// Runner and serve code should prefer this facade over writing individual
    /// JSONL files directly. It keeps provider/model outcomes, summaries, gate
    /// outcomes, retry outcomes, and knowledge seeds in the same schema family.
    ///
    /// # Errors
    ///
    /// Returns an error when an enabled append fails.
    pub async fn record_runner_event(
        &self,
        event: RunnerFeedbackEvent,
    ) -> Result<RuntimeFeedbackWrite, LearningRuntimeError> {
        let mut write = RuntimeFeedbackWrite::default();

        match event {
            RunnerFeedbackEvent::CompletedRun { input } => {
                let update = self.record_completed_run(*input).await?;
                write.provider_model_outcomes +=
                    usize::from(update.provider_model_outcome_recorded == ApplyStatus::Applied);
                write.efficiency_summaries +=
                    usize::from(update.efficiency_summary_recorded == ApplyStatus::Applied);
                write.gate_outcomes += update.gate_outcomes_recorded;
                write.retry_outcomes +=
                    usize::from(update.retry_outcome_recorded == ApplyStatus::Applied);
                write.knowledge_seeds +=
                    usize::from(update.knowledge_seed_recorded == ApplyStatus::Applied);
                write.learning_update = Some(update);
            }
            RunnerFeedbackEvent::Episode { episode } => {
                self.append_episode(&episode).await?;
                write.episode_appended = true;
                write.merge(self.append_derived_episode_feedback(&episode, true).await?);
            }
            RunnerFeedbackEvent::EfficiencyEvent { event, scope } => {
                let provider_model_outcome =
                    ProviderModelOutcomeRecord::from_efficiency_event(&event).is_some();
                self.append_efficiency_event_with_scope(&event, scope)
                    .await?;
                write.efficiency_events = 1;
                write.efficiency_summaries = 1;
                write.provider_model_outcomes = usize::from(provider_model_outcome);
            }
            RunnerFeedbackEvent::ProviderModelOutcome { outcome } => {
                self.append_provider_model_outcome(&outcome).await?;
                write.provider_model_outcomes = 1;
            }
            RunnerFeedbackEvent::EfficiencySummary { summary } => {
                self.append_efficiency_summary(&summary).await?;
                write.efficiency_summaries = 1;
            }
            RunnerFeedbackEvent::GateOutcome { outcome } => {
                self.append_gate_outcome(&outcome).await?;
                write.gate_outcomes = 1;
            }
            RunnerFeedbackEvent::RetryOutcome { outcome } => {
                self.append_retry_outcome(&outcome).await?;
                write.retry_outcomes = 1;
            }
            RunnerFeedbackEvent::KnowledgeSeed { seed } => {
                self.append_knowledge_seed(&seed).await?;
                write.knowledge_seeds = 1;
            }
        }

        Ok(write)
    }

    /// Record a generation outcome while distinguishing process success from artifact validity.
    ///
    /// Episodes are always persisted for analysis. Positive learning signals are only
    /// generated when the process completed successfully and the artifact passed validation.
    ///
    /// # Errors
    ///
    /// Returns an error if persistence of the episode or derived feedback fails.
    pub async fn record_generation_outcome(
        &self,
        task_id: &str,
        model: &str,
        outcome: &GenerationOutcome,
    ) -> Result<(), LearningRuntimeError> {
        let mut episode = Episode::new("roko-cli", task_id);
        episode.kind = "generation".to_string();
        episode.agent_template = "generator".to_string();
        episode.model = model.to_string();
        episode.trigger_kind = "generation".to_string();
        episode.success = outcome.fully_successful();
        episode.failure_reason = if outcome.process_success && !outcome.artifact_valid {
            Some("artifact validation failed".to_string())
        } else if !outcome.process_success {
            Some("generation process failed".to_string())
        } else {
            None
        };
        episode.extra.insert(
            "process_success".to_string(),
            serde_json::json!(outcome.process_success),
        );
        episode.extra.insert(
            "artifact_valid".to_string(),
            serde_json::json!(outcome.artifact_valid),
        );
        episode.extra.insert(
            "generation_status".to_string(),
            serde_json::json!(outcome.status_label()),
        );
        if let Some(report) = &outcome.validation_report {
            episode
                .extra
                .insert("validation_report".to_string(), report.clone());
        }
        episode.attach_all_fingerprints();

        self.record_runner_event(RunnerFeedbackEvent::Episode {
            episode: Box::new(episode),
        })
        .await
        .map(|_| ())
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
        self.append_efficiency_event_with_scope(event, EfficiencyScope::Turn)
            .await
    }

    async fn append_efficiency_event_with_scope(
        &self,
        event: &AgentEfficiencyEvent,
        scope: EfficiencyScope,
    ) -> Result<(), LearningRuntimeError> {
        append_jsonl_record(&self.paths.efficiency_jsonl, event).await?;
        self.record_latency_from_efficiency_event(event)?;
        self.record_section_effectiveness_from_efficiency_event(event)?;
        let summary = EfficiencySummaryRecord::from_efficiency_event(event, scope);
        self.append_efficiency_summary(&summary).await?;
        if let Some(outcome) = ProviderModelOutcomeRecord::from_efficiency_event(event) {
            self.append_provider_model_outcome(&outcome).await?;
        }
        Ok(())
    }

    /// Append a normalized efficiency summary to the JSONL log.
    ///
    /// # Errors
    ///
    /// Returns an error on serialization or write failure.
    pub async fn append_efficiency_summary(
        &self,
        summary: &EfficiencySummaryRecord,
    ) -> Result<(), LearningRuntimeError> {
        append_jsonl_record(&self.paths.efficiency_summaries_jsonl, summary).await?;
        Ok(())
    }

    /// Append a provider/model outcome to the JSONL log.
    ///
    /// # Errors
    ///
    /// Returns an error on serialization or write failure.
    pub async fn append_provider_model_outcome(
        &self,
        outcome: &ProviderModelOutcomeRecord,
    ) -> Result<(), LearningRuntimeError> {
        self.provider_model_outcomes.append(outcome).await?;
        Ok(())
    }

    /// Append a gate outcome to the JSONL log.
    ///
    /// # Errors
    ///
    /// Returns an error on serialization or write failure.
    pub async fn append_gate_outcome(
        &self,
        outcome: &GateOutcomeRecord,
    ) -> Result<(), LearningRuntimeError> {
        append_jsonl_record(&self.paths.gate_outcomes_jsonl, outcome).await?;
        Ok(())
    }

    /// Append multiple gate outcomes to the JSONL log.
    ///
    /// # Errors
    ///
    /// Returns an error on the first serialization or write failure.
    pub async fn append_gate_outcomes(
        &self,
        outcomes: &[GateOutcomeRecord],
    ) -> Result<(), LearningRuntimeError> {
        for outcome in outcomes {
            self.append_gate_outcome(outcome).await?;
        }
        Ok(())
    }

    /// Append a retry outcome to the JSONL log.
    ///
    /// # Errors
    ///
    /// Returns an error on serialization or write failure.
    pub async fn append_retry_outcome(
        &self,
        outcome: &RetryOutcomeRecord,
    ) -> Result<(), LearningRuntimeError> {
        append_jsonl_record(&self.paths.retry_outcomes_jsonl, outcome).await?;
        Ok(())
    }

    /// Append a knowledge seed to the JSONL log.
    ///
    /// # Errors
    ///
    /// Returns an error on serialization or write failure.
    pub async fn append_knowledge_seed(
        &self,
        seed: &KnowledgeSeedRecord,
    ) -> Result<(), LearningRuntimeError> {
        append_jsonl_record(&self.paths.knowledge_seeds_jsonl, seed).await?;
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
    ///
    /// # Errors
    ///
    /// Returns an error if the log cannot be opened or read, or if a line
    /// read from the log fails unexpectedly.
    pub async fn read_efficiency_events(
        &self,
    ) -> Result<Vec<AgentEfficiencyEvent>, LearningRuntimeError> {
        read_efficiency_events(&self.paths.efficiency_jsonl).await
    }

    /// Read all persisted provider/model outcome telemetry records.
    ///
    /// Returns an empty vec if the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the log cannot be opened or read.
    pub async fn read_provider_model_outcomes(
        &self,
    ) -> Result<Vec<ProviderModelOutcomeRecord>, LearningRuntimeError> {
        read_provider_model_outcomes(&self.paths.provider_model_outcomes_jsonl)
            .await
            .map_err(LearningRuntimeError::Io)
    }

    /// Read all persisted efficiency summaries.
    ///
    /// Returns an empty vec if the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the log cannot be opened or read.
    pub async fn read_efficiency_summaries(
        &self,
    ) -> Result<Vec<EfficiencySummaryRecord>, LearningRuntimeError> {
        read_efficiency_summaries(&self.paths.efficiency_summaries_jsonl).await
    }

    /// Read all persisted gate outcomes.
    ///
    /// Returns an empty vec if the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the log cannot be opened or read.
    pub async fn read_gate_outcomes(&self) -> Result<Vec<GateOutcomeRecord>, LearningRuntimeError> {
        read_gate_outcomes(&self.paths.gate_outcomes_jsonl).await
    }

    /// Read all persisted retry outcomes.
    ///
    /// Returns an empty vec if the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the log cannot be opened or read.
    pub async fn read_retry_outcomes(
        &self,
    ) -> Result<Vec<RetryOutcomeRecord>, LearningRuntimeError> {
        read_retry_outcomes(&self.paths.retry_outcomes_jsonl).await
    }

    /// Read all persisted knowledge seeds.
    ///
    /// Returns an empty vec if the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the log cannot be opened or read.
    pub async fn read_knowledge_seeds(
        &self,
    ) -> Result<Vec<KnowledgeSeedRecord>, LearningRuntimeError> {
        read_knowledge_seeds(&self.paths.knowledge_seeds_jsonl).await
    }

    /// Query all canonical runtime feedback streams for this runtime.
    ///
    /// # Errors
    ///
    /// Returns an error if an existing log cannot be opened or read.
    pub async fn query_feedback(
        &self,
        query: &RuntimeFeedbackQuery,
    ) -> Result<RuntimeFeedbackSnapshot, LearningRuntimeError> {
        read_runtime_feedback_snapshot(&self.paths, query).await
    }

    /// Return rolling provider/model pass-rate summaries.
    ///
    /// # Errors
    ///
    /// Returns an error if the outcome log cannot be opened or read.
    pub async fn provider_model_pass_rates(
        &self,
        window_size: usize,
    ) -> Result<ProviderModelPassRateReport, LearningRuntimeError> {
        let records = self.read_provider_model_outcomes().await?;
        Ok(summarize_provider_model_outcomes(&records, window_size))
    }

    /// Read the latest persisted C-Factor snapshot, if one exists.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot file cannot be read.
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
    ///
    /// # Errors
    ///
    /// Returns an error if the cascade router snapshot cannot be written.
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
    ///
    /// # Errors
    ///
    /// Returns an error if the episode cannot be appended to the persisted
    /// log.
    pub async fn append_episode(&self, episode: &Episode) -> Result<(), LearningRuntimeError> {
        let mut episode = episode.clone();
        self.apply_affect_signature(&mut episode);
        self.episode_logger.append(&episode).await?;
        Ok(())
    }

    async fn append_derived_episode_feedback(
        &self,
        episode: &Episode,
        include_provider_model_outcome: bool,
    ) -> Result<RuntimeFeedbackWrite, LearningRuntimeError> {
        let mut write = RuntimeFeedbackWrite::default();

        if include_provider_model_outcome
            && let Some(outcome) = ProviderModelOutcomeRecord::from_episode(episode, None)
        {
            self.append_provider_model_outcome(&outcome).await?;
            write.provider_model_outcomes = 1;
        }

        let summary = EfficiencySummaryRecord::from_episode(episode);
        self.append_efficiency_summary(&summary).await?;
        write.efficiency_summaries = 1;

        let gate_outcomes = GateOutcomeRecord::from_episode(episode);
        self.append_gate_outcomes(&gate_outcomes).await?;
        write.gate_outcomes = gate_outcomes.len();

        if let Some(retry_outcome) = RetryOutcomeRecord::from_episode(episode) {
            self.append_retry_outcome(&retry_outcome).await?;
            write.retry_outcomes = 1;
        }

        // Gate knowledge seeds on artifact validity as well as process success.
        // `artifact_valid = false` means the process ran, but the produced artifact
        // failed grounding validation. Do not store positive learning from that case.
        let artifact_valid = extra_bool(episode, "artifact_valid").unwrap_or(true);
        if artifact_valid {
            if let Some(seed) = KnowledgeSeedRecord::from_successful_episode(episode) {
                self.append_knowledge_seed(&seed).await?;
                write.knowledge_seeds = 1;
            }
        } else {
            tracing::info!(
                task_id = %episode.task_id,
                episode_id = %episode.episode_id,
                "Withholding knowledge seed: artifact_valid=false in episode extra"
            );
        }

        Ok(write)
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

        let gate_counts = gate_counts_from_episode(&input.episode);
        let skip_only = gate_counts.is_some_and(GateCounts::has_only_skipped);
        if let Some(counts) = gate_counts {
            backfill_gate_counts(&mut input.episode, counts);
        }
        if skip_only {
            input.episode.success = false;
            if input
                .episode
                .failure_reason
                .as_ref()
                .is_none_or(|reason| reason.trim().is_empty())
            {
                input.episode.failure_reason = Some("all gates skipped".to_string());
            }
            input.episode.extra.insert(
                "provider_model_outcome_status".to_string(),
                serde_json::json!("blocked"),
            );
        }

        input.episode.attach_all_fingerprints();
        self.apply_affect_signature(&mut input.episode);
        self.episode_logger.append(&input.episode).await?;
        update.episode_logged = ApplyStatus::Applied;
        if let Some(hook) = &self.episode_completion_hook {
            hook(input.episode.clone());
        }
        let episode_count = self.episode_count.fetch_add(1, Ordering::Relaxed) + 1;

        if !skip_only && let Some(reflection_input) = ReflectionInput::from_episode(&input.episode)
        {
            let mut reflection_store =
                PostGateReflectionStore::load(&self.paths.post_gate_reflections_json);
            let observation =
                reflection_store.observe(reflection_input, ReflectionPromotionConfig::default());
            reflection_store.save(&self.paths.post_gate_reflections_json)?;
            update.reflection_recorded = ApplyStatus::Applied;
            if observation.candidate.is_some() {
                update.reflection_candidate_updated = ApplyStatus::Applied;
            }
        }

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

        let provider_for_outcome = input.provider.clone();

        if !skip_only && let Some(provider) = input.provider {
            if input.episode.success {
                self.provider_health.record_success(&provider);
            } else {
                self.provider_health.record_failure(&provider);
            }
            update.provider_updated = ApplyStatus::Applied;
        }

        if let Some(outcome) = ProviderModelOutcomeRecord::from_episode(
            &input.episode,
            provider_for_outcome.as_deref(),
        ) {
            self.provider_model_outcomes.append(&outcome).await?;
            update.provider_model_outcome_recorded = ApplyStatus::Applied;
        }

        let derived_feedback = self
            .append_derived_episode_feedback(&input.episode, false)
            .await?;
        if derived_feedback.efficiency_summaries > 0 {
            update.efficiency_summary_recorded = ApplyStatus::Applied;
        }
        update.gate_outcomes_recorded = derived_feedback.gate_outcomes;
        if derived_feedback.retry_outcomes > 0 {
            update.retry_outcome_recorded = ApplyStatus::Applied;
        }
        if derived_feedback.knowledge_seeds > 0 {
            update.knowledge_seed_recorded = ApplyStatus::Applied;
        }

        if !skip_only && let Some(playbook_id) = input.playbook_id {
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

        if !skip_only && let Some(rule_id) = input.playbook_rule_id {
            self.playbook_rules
                .record_outcome(&rule_id, input.episode.success);
            self.playbook_rules.save()?;
            update.playbook_rule_updated = ApplyStatus::Applied;
        }

        if !skip_only
            && let Some(skill_id) = input.matched_skill_id
            && self.skill_library.get(&skill_id).is_some()
        {
            self.skill_library
                .record_outcome(&skill_id, input.episode.success)
                .await?;
            update.matched_skill_updated = ApplyStatus::Applied;
        }

        let generator = TemplatePatternGenerator;
        if !skip_only
            && self.update_frequency.skill_mining_due(episode_count)
            && let Some(skill) = self.skill_library.extract(&input.episode, &generator).await
        {
            update.extracted_skill_id = Some(skill.name);
        }

        if !skip_only && let Some(metric) = input.task_metric {
            append_task_metric(&self.paths.task_metrics_jsonl, &metric).await?;
            let metrics_snapshot = {
                let mut guard = self.task_metrics.lock().await;
                guard.push(metric);
                guard.clone()
            };
            update.regression_report =
                compute_regression_report(&metrics_snapshot, &self.regression);
        }

        if !skip_only && self.update_frequency.distiller_due(episode_count) {
            self.append_cfactor_snapshot().await?;
        }

        // ── Pattern mining ──────────────────────────────────────────────
        let actions = EpisodeActions::from_episode(&input.episode);
        if !skip_only
            && self.update_frequency.pattern_discovery_due(episode_count)
            && !actions.actions.is_empty()
        {
            self.pattern_miner.lock().ingest_episode(&actions);
            update.patterns_ingested = true;
        }

        // ── Cascade router observation ─────────────────────────────────
        // Do not feed positive observations for artifact-invalid episodes.
        // Missing `artifact_valid` remains backward-compatible and counts as valid.
        let artifact_valid_for_router =
            extra_bool(&input.episode, "artifact_valid").unwrap_or(true);
        if !skip_only
            && self.update_frequency.router_due(episode_count)
            && artifact_valid_for_router
        {
            update.router_updated = self.update_cascade_router(&input.episode);
        } else if !artifact_valid_for_router {
            tracing::debug!(
                task_id = %input.episode.task_id,
                model = ?extra_string(&input.episode, "model"),
                "Cascade router: skipping positive observation -- artifact_valid=false"
            );
        }

        // Persist immediately so the router state file always reflects the
        // latest observation count and confidence stats.
        if update.router_updated {
            if let Err(e) = self.save_cascade_router() {
                eprintln!("[learn] cascade router save failed: {e}");
            }
        }

        // ── Prompt experiment outcome ────────────────────────────────────
        if !skip_only
            && self.update_frequency.experiments_due(episode_count)
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
            if let Err(e) =
                sync_experiment_winner_artifact(&self.paths.experiment_winners_json, &store)
            {
                eprintln!("[learn] experiment winner artifact save failed: {e}");
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
        if !skip_only && let Some(model) = extra_string(&input.episode, "model") {
            self.observe_local_reward("router", &model, success);
        }
        if !skip_only && let Some(ref skill_id) = local_reward_skill_id {
            self.observe_local_reward("skill", skill_id, success);
        }
        if !skip_only && let Some(ref rule_id) = local_reward_rule_id {
            self.observe_local_reward("playbook_rule", rule_id, success);
        }
        if !skip_only {
            self.save_local_rewards();
        }

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
        let skip_only = gate_counts_from_episode(episode).is_some_and(GateCounts::has_only_skipped);
        if !skip_only {
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

    /// Update the cascade router from episode metadata if model is available.
    fn update_cascade_router(&self, episode: &Episode) -> bool {
        let role_str = extra_string(episode, "role");
        let model_slug = extra_string(episode, "model");
        let Some(slug) = model_slug else {
            return false;
        };
        let role = role_str
            .as_deref()
            .and_then(parse_agent_role)
            .unwrap_or(AgentRole::Implementer);
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
            conductor_load: 0.0,
            active_agents: 0,
            ready_queue_depth: 0,
            max_queue_wait_hours: 0.0,
            daimon_policy: DaimonPolicy::new(
                extra_f64(episode, "affect_confidence").unwrap_or(0.5),
                roko_core::BehavioralState::Engaged,
            ),
            thinking_level: None,
            temperament: None,
            previous_model: None,
            plan_context_tokens: None,
            tier_thresholds: None,
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

fn sync_experiment_winner_artifact(path: &Path, store: &ExperimentStore) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let winners = store.winner_summaries();
    let json = serde_json::to_vec_pretty(&winners)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, json)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
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
    fn record_section_effectiveness_from_efficiency_event(
        &self,
        event: &AgentEfficiencyEvent,
    ) -> Result<(), LearningRuntimeError> {
        if event.role.trim().is_empty() || event.prompt_sections.is_empty() {
            return Ok(());
        }

        let mut registry = self.section_effectiveness.lock();
        for section in &event.prompt_sections {
            registry.record_outcome(
                section.name.clone(),
                event.role.trim(),
                !section.was_dropped,
                event.gate_passed,
            );
        }
        registry.save(&self.paths.section_effects_json)?;
        Ok(())
    }

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

fn extra_u64(episode: &Episode, key: &str) -> Option<u64> {
    episode.extra.get(key).and_then(serde_json::Value::as_u64)
}

fn extra_bool(episode: &Episode, key: &str) -> Option<bool> {
    episode.extra.get(key).and_then(serde_json::Value::as_bool)
}

fn gate_counts_from_episode(episode: &Episode) -> Option<GateCounts> {
    let gate_counts = episode
        .extra
        .get("gate_counts")
        .and_then(serde_json::Value::as_object);

    let passed = gate_counts
        .and_then(|counts| counts.get("passed"))
        .and_then(serde_json::Value::as_u64)
        .or_else(|| extra_u64(episode, "gates_passed"));
    let failed = gate_counts
        .and_then(|counts| counts.get("failed"))
        .and_then(serde_json::Value::as_u64)
        .or_else(|| extra_u64(episode, "gates_failed"));
    let skipped = gate_counts
        .and_then(|counts| counts.get("skipped"))
        .and_then(serde_json::Value::as_u64)
        .or_else(|| extra_u64(episode, "gates_skipped"));

    if passed.is_none() && failed.is_none() && skipped.is_none() && episode.gate_verdicts.is_empty()
    {
        return None;
    }

    Some(GateCounts {
        passed: passed.unwrap_or_else(|| {
            episode
                .gate_verdicts
                .iter()
                .filter(|verdict| verdict.passed)
                .count() as u64
        }),
        failed: failed.unwrap_or_else(|| {
            episode
                .gate_verdicts
                .iter()
                .filter(|verdict| !verdict.passed)
                .count() as u64
        }),
        skipped: skipped.unwrap_or(0),
    })
}

fn backfill_gate_counts(episode: &mut Episode, counts: GateCounts) {
    episode
        .extra
        .entry("gates_passed".to_string())
        .or_insert_with(|| serde_json::json!(counts.passed));
    episode
        .extra
        .entry("gates_failed".to_string())
        .or_insert_with(|| serde_json::json!(counts.failed));
    episode
        .extra
        .entry("gates_skipped".to_string())
        .or_insert_with(|| serde_json::json!(counts.skipped));
    episode
        .extra
        .entry("gates_executed".to_string())
        .or_insert_with(|| serde_json::json!(counts.executed()));
    episode
        .extra
        .entry("gate_summary".to_string())
        .or_insert_with(|| serde_json::json!(counts.summary()));
    episode
        .extra
        .entry("gate_pass_rate".to_string())
        .or_insert_with(|| serde_json::json!(counts.pass_rate()));
    episode
        .extra
        .entry("gate_counts".to_string())
        .or_insert_with(|| {
            serde_json::json!({
                "passed": counts.passed,
                "failed": counts.failed,
                "skipped": counts.skipped,
                "executed": counts.executed(),
                "summary": counts.summary(),
                "pass_rate": counts.pass_rate(),
            })
        });
}

fn extra_string_vec(episode: &Episode, key: &str) -> Option<Vec<String>> {
    let values = episode.extra.get(key)?.as_array()?;
    let out = values
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    (!out.is_empty()).then_some(out)
}

fn non_empty_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn nonzero_u64(value: u64) -> Option<u64> {
    (value > 0).then_some(value)
}

fn ratio_u64(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn episode_model(episode: &Episode) -> String {
    non_empty_string(&episode.model)
        .or_else(|| extra_string(episode, "model"))
        .or_else(|| extra_string(episode, "model_used"))
        .unwrap_or_default()
}

fn episode_provider(episode: &Episode) -> String {
    non_empty_string(&episode.backend)
        .or_else(|| extra_string(episode, "provider"))
        .or_else(|| extra_string(episode, "backend"))
        .unwrap_or_else(|| "unknown-provider".to_string())
}

fn episode_role(episode: &Episode) -> String {
    extra_string(episode, "role")
        .or_else(|| extra_string(episode, "role_id"))
        .or_else(|| non_empty_string(&episode.agent_template))
        .unwrap_or_else(|| "unknown-role".to_string())
}

fn episode_run_id(episode: &Episode) -> Option<String> {
    extra_string(episode, "run_id")
        .or_else(|| extra_string(episode, "session_id"))
        .or_else(|| non_empty_string(&episode.episode_id))
}

fn prompt_section_count_from_episode(episode: &Episode) -> u32 {
    episode
        .prompt_composition
        .as_ref()
        .and_then(|value| value.get("sections"))
        .and_then(serde_json::Value::as_array)
        .map_or(0, |sections| sections.len().min(u32::MAX as usize) as u32)
}

fn retry_status_from_episode(episode: &Episode) -> Option<RetryOutcomeStatus> {
    if let Some(raw) =
        extra_string(episode, "retry_status").or_else(|| extra_string(episode, "retry_outcome"))
    {
        return parse_retry_status(&raw);
    }
    if extra_bool(episode, "retry_scheduled") == Some(true) {
        return Some(RetryOutcomeStatus::Scheduled);
    }
    if extra_bool(episode, "retry_started") == Some(true) {
        return Some(RetryOutcomeStatus::Started);
    }
    if extra_bool(episode, "retry_exhausted") == Some(true) {
        return Some(RetryOutcomeStatus::Exhausted);
    }
    if extra_bool(episode, "retry_not_retryable") == Some(true) {
        return Some(RetryOutcomeStatus::NotRetryable);
    }
    None
}

fn parse_retry_status(raw: &str) -> Option<RetryOutcomeStatus> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "scheduled" => Some(RetryOutcomeStatus::Scheduled),
        "started" => Some(RetryOutcomeStatus::Started),
        "succeeded" | "success" | "passed" => Some(RetryOutcomeStatus::Succeeded),
        "exhausted" | "retries_exhausted" => Some(RetryOutcomeStatus::Exhausted),
        "not_retryable" | "non_retryable" => Some(RetryOutcomeStatus::NotRetryable),
        "cancelled" | "canceled" => Some(RetryOutcomeStatus::Cancelled),
        _ => None,
    }
}

fn stable_hash_hex(parts: &[&str]) -> String {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for part in parts {
        for byte in part.as_bytes().iter().copied().chain([0xff]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    format!("{hash:016x}")
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
        model: episode_model(episode),
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
///
/// # Errors
///
/// Returns an error if the file cannot be opened or if any read operation
/// fails unexpectedly.
pub async fn read_efficiency_events(
    path: &Path,
) -> Result<Vec<AgentEfficiencyEvent>, LearningRuntimeError> {
    read_jsonl_lossy(path).await
}

/// Read normalized efficiency summaries from a JSONL file.
///
/// Missing files produce an empty vector and malformed lines are skipped.
///
/// # Errors
///
/// Returns an error only for file open/read failures.
pub async fn read_efficiency_summaries(
    path: &Path,
) -> Result<Vec<EfficiencySummaryRecord>, LearningRuntimeError> {
    read_jsonl_lossy(path).await
}

/// Read gate outcomes from a JSONL file.
///
/// Missing files produce an empty vector and malformed lines are skipped.
///
/// # Errors
///
/// Returns an error only for file open/read failures.
pub async fn read_gate_outcomes(
    path: &Path,
) -> Result<Vec<GateOutcomeRecord>, LearningRuntimeError> {
    read_jsonl_lossy(path).await
}

/// Read retry outcomes from a JSONL file.
///
/// Missing files produce an empty vector and malformed lines are skipped.
///
/// # Errors
///
/// Returns an error only for file open/read failures.
pub async fn read_retry_outcomes(
    path: &Path,
) -> Result<Vec<RetryOutcomeRecord>, LearningRuntimeError> {
    read_jsonl_lossy(path).await
}

/// Read knowledge seeds from a JSONL file.
///
/// Missing files produce an empty vector and malformed lines are skipped.
///
/// # Errors
///
/// Returns an error only for file open/read failures.
pub async fn read_knowledge_seeds(
    path: &Path,
) -> Result<Vec<KnowledgeSeedRecord>, LearningRuntimeError> {
    read_jsonl_lossy(path).await
}

async fn append_jsonl_record<T: Serialize + Sync + ?Sized>(
    path: &Path,
    value: &T,
) -> Result<(), LearningRuntimeError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut line = serde_json::to_string(value)?;
    line.push('\n');
    // Size-based rotation: shared with EpisodeLogger so efficiency /
    // efficiency-summaries logs cannot grow unbounded over a long run.
    crate::jsonl_rotation::rotate_if_needed(
        path,
        crate::jsonl_rotation::DEFAULT_ROTATION_THRESHOLD_BYTES,
    )
    .await?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(line.as_bytes()).await?;
    file.sync_data().await?;
    Ok(())
}

async fn read_jsonl_lossy<T>(path: &Path) -> Result<Vec<T>, LearningRuntimeError>
where
    T: DeserializeOwned,
{
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
        if let Ok(record) = serde_json::from_str::<T>(trimmed) {
            out.push(record);
        }
    }
    Ok(out)
}

/// Learning artifacts discovered for a project workdir.
///
/// This is intentionally read-only and tolerant. It lets HTTP and CLI surfaces
/// show runner-produced durable feedback even while different runtimes still
/// write episodes to legacy and current locations.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectLearningSnapshot {
    /// Episode records found across known episode JSONL locations.
    pub episodes: Vec<Episode>,
    /// Efficiency events from `.roko/learn/efficiency.jsonl`.
    pub efficiency_events: Vec<AgentEfficiencyEvent>,
    /// Provider/model outcomes from `.roko/learn/provider-model-outcomes.jsonl`.
    pub provider_model_outcomes: Vec<ProviderModelOutcomeRecord>,
    /// Efficiency summaries from `.roko/learn/efficiency-summaries.jsonl`.
    pub efficiency_summaries: Vec<EfficiencySummaryRecord>,
    /// Gate outcomes from `.roko/learn/gate-outcomes.jsonl`.
    pub gate_outcomes: Vec<GateOutcomeRecord>,
    /// Retry outcomes from `.roko/learn/retry-outcomes.jsonl`.
    pub retry_outcomes: Vec<RetryOutcomeRecord>,
    /// Knowledge seeds from `.roko/learn/knowledge-seeds.jsonl`.
    pub knowledge_seeds: Vec<KnowledgeSeedRecord>,
    /// Parsed cascade router snapshot from `.roko/learn/cascade-router.json`.
    pub cascade_router: Option<serde_json::Value>,
    /// Number of durable knowledge entries in `.roko/neuro/knowledge.jsonl`.
    pub knowledge_entries: usize,
    /// Episode files that existed and were read.
    pub episode_paths: Vec<PathBuf>,
    /// Efficiency log path.
    pub efficiency_path: PathBuf,
    /// Provider/model outcome log path.
    pub provider_model_outcomes_path: PathBuf,
    /// Efficiency summary log path.
    pub efficiency_summaries_path: PathBuf,
    /// Gate outcome log path.
    pub gate_outcomes_path: PathBuf,
    /// Retry outcome log path.
    pub retry_outcomes_path: PathBuf,
    /// Knowledge seed log path.
    pub knowledge_seeds_path: PathBuf,
    /// Cascade router snapshot path.
    pub cascade_router_path: PathBuf,
    /// Durable knowledge JSONL path.
    pub knowledge_path: PathBuf,
}

/// Return known episode JSONL locations for `workdir`, canonical paths first,
/// followed by legacy locations.
///
/// Order: root episodes (canonical) -> learn dir -> memory dir (legacy fallback).
#[must_use]
pub fn project_episode_paths(workdir: impl AsRef<Path>) -> Vec<PathBuf> {
    let roko = workdir.as_ref().join(".roko");
    vec![
        // Canonical: root episodes.jsonl
        roko.join("episodes.jsonl"),
        // Canonical: learn directory
        roko.join("learn").join("episodes.jsonl"),
        // Legacy fallback: memory directory (migration surface only)
        roko.join("memory").join("episodes.jsonl"),
    ]
}

/// Read all valid project episodes from known JSONL locations, de-duplicating
/// records that appear in more than one location.
///
/// # Errors
///
/// Returns an error only for filesystem read failures from an existing file.
pub async fn read_project_episodes_lossy(
    workdir: impl AsRef<Path>,
) -> Result<Vec<Episode>, LearningRuntimeError> {
    let mut episodes = Vec::new();
    let mut seen = HashSet::new();
    for path in project_episode_paths(workdir) {
        if !path.exists() {
            continue;
        }
        for episode in EpisodeLogger::read_all_lossy(&path).await? {
            let key = episode_dedupe_key(&episode);
            if seen.insert(key) {
                episodes.push(episode);
            }
        }
    }
    Ok(episodes)
}

/// Read project efficiency events from `.roko/learn/efficiency.jsonl`.
///
/// # Errors
///
/// Returns an error if the efficiency log cannot be read.
pub async fn read_project_efficiency_events(
    workdir: impl AsRef<Path>,
) -> Result<Vec<AgentEfficiencyEvent>, LearningRuntimeError> {
    read_efficiency_events(&workdir.as_ref().join(".roko/learn/efficiency.jsonl")).await
}

/// Read the current project learning artifacts for CLI/API presentation.
///
/// # Errors
///
/// Returns an error if an existing artifact cannot be read.
pub async fn read_project_learning_snapshot(
    workdir: impl AsRef<Path>,
) -> Result<ProjectLearningSnapshot, LearningRuntimeError> {
    let workdir = workdir.as_ref();
    let roko = workdir.join(".roko");
    let efficiency_path = roko.join("learn").join("efficiency.jsonl");
    let provider_model_outcomes_path = roko.join("learn").join("provider-model-outcomes.jsonl");
    let efficiency_summaries_path = roko.join("learn").join("efficiency-summaries.jsonl");
    let gate_outcomes_path = roko.join("learn").join("gate-outcomes.jsonl");
    let retry_outcomes_path = roko.join("learn").join("retry-outcomes.jsonl");
    let knowledge_seeds_path = roko.join("learn").join("knowledge-seeds.jsonl");
    let cascade_router_path = roko.join("learn").join("cascade-router.json");
    let knowledge_path = roko.join("neuro").join("knowledge.jsonl");

    let episodes = read_project_episodes_lossy(workdir).await?;
    let episode_paths = project_episode_paths(workdir)
        .into_iter()
        .filter(|path| path.exists())
        .collect();
    let efficiency_events = read_efficiency_events(&efficiency_path).await?;
    let provider_model_outcomes = read_provider_model_outcomes(&provider_model_outcomes_path)
        .await
        .map_err(LearningRuntimeError::Io)?;
    let efficiency_summaries = read_efficiency_summaries(&efficiency_summaries_path).await?;
    let gate_outcomes = read_gate_outcomes(&gate_outcomes_path).await?;
    let retry_outcomes = read_retry_outcomes(&retry_outcomes_path).await?;
    let knowledge_seeds = read_knowledge_seeds(&knowledge_seeds_path).await?;
    let cascade_router = match tokio::fs::read_to_string(&cascade_router_path).await {
        Ok(contents) => serde_json::from_str(&contents).ok(),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => return Err(LearningRuntimeError::Io(err)),
    };
    let knowledge_entries = count_jsonl_records(&knowledge_path).await?;

    Ok(ProjectLearningSnapshot {
        episodes,
        efficiency_events,
        provider_model_outcomes,
        efficiency_summaries,
        gate_outcomes,
        retry_outcomes,
        knowledge_seeds,
        cascade_router,
        knowledge_entries,
        episode_paths,
        efficiency_path,
        provider_model_outcomes_path,
        efficiency_summaries_path,
        gate_outcomes_path,
        retry_outcomes_path,
        knowledge_seeds_path,
        cascade_router_path,
        knowledge_path,
    })
}

/// Query canonical feedback logs under `paths`.
///
/// # Errors
///
/// Returns an error if an existing log cannot be opened or read.
pub async fn read_runtime_feedback_snapshot(
    paths: &LearningPaths,
    query: &RuntimeFeedbackQuery,
) -> Result<RuntimeFeedbackSnapshot, LearningRuntimeError> {
    let mut episodes = EpisodeLogger::read_all_lossy(&paths.episodes_jsonl).await?;
    episodes.retain(|episode| episode_matches_query(episode, query));
    apply_latest_limit(&mut episodes, query.limit);

    let mut provider_model_outcomes =
        read_provider_model_outcomes(&paths.provider_model_outcomes_jsonl)
            .await
            .map_err(LearningRuntimeError::Io)?;
    provider_model_outcomes.retain(|record| provider_model_outcome_matches_query(record, query));
    apply_latest_limit(&mut provider_model_outcomes, query.limit);

    let mut efficiency_summaries =
        read_efficiency_summaries(&paths.efficiency_summaries_jsonl).await?;
    efficiency_summaries.retain(|record| efficiency_summary_matches_query(record, query));
    apply_latest_limit(&mut efficiency_summaries, query.limit);

    let mut gate_outcomes = read_gate_outcomes(&paths.gate_outcomes_jsonl).await?;
    gate_outcomes.retain(|record| gate_outcome_matches_query(record, query));
    apply_latest_limit(&mut gate_outcomes, query.limit);

    let mut retry_outcomes = read_retry_outcomes(&paths.retry_outcomes_jsonl).await?;
    retry_outcomes.retain(|record| retry_outcome_matches_query(record, query));
    apply_latest_limit(&mut retry_outcomes, query.limit);

    let mut knowledge_seeds = read_knowledge_seeds(&paths.knowledge_seeds_jsonl).await?;
    knowledge_seeds.retain(|record| knowledge_seed_matches_query(record, query));
    apply_latest_limit(&mut knowledge_seeds, query.limit);

    Ok(RuntimeFeedbackSnapshot {
        episodes,
        provider_model_outcomes,
        efficiency_summaries,
        gate_outcomes,
        retry_outcomes,
        knowledge_seeds,
    })
}

/// Query canonical project feedback logs using default `.roko/learn` paths.
///
/// This reads episodes from all known project episode locations, then reads the
/// canonical derived feedback streams from `.roko/learn`.
///
/// # Errors
///
/// Returns an error if an existing log cannot be opened or read.
pub async fn read_project_runtime_feedback_snapshot(
    workdir: impl AsRef<Path>,
    query: &RuntimeFeedbackQuery,
) -> Result<RuntimeFeedbackSnapshot, LearningRuntimeError> {
    let workdir = workdir.as_ref();
    let paths = LearningPaths::under(workdir.join(".roko").join("learn"));
    let mut snapshot = read_runtime_feedback_snapshot(&paths, query).await?;

    let mut project_episodes = read_project_episodes_lossy(workdir).await?;
    project_episodes.retain(|episode| episode_matches_query(episode, query));
    apply_latest_limit(&mut project_episodes, query.limit);
    snapshot.episodes = project_episodes;

    Ok(snapshot)
}

fn apply_latest_limit<T>(items: &mut Vec<T>, limit: Option<usize>) {
    let Some(limit) = limit else {
        return;
    };
    if items.len() > limit {
        let drop_count = items.len() - limit;
        items.drain(0..drop_count);
    }
}

fn query_matches(value: &str, expected: Option<&String>) -> bool {
    expected
        .map(String::as_str)
        .is_none_or(|expected| value.trim() == expected.trim())
}

fn query_matches_option(value: Option<&str>, expected: Option<&String>) -> bool {
    expected
        .map(String::as_str)
        .is_none_or(|expected| value.is_some_and(|value| value.trim() == expected.trim()))
}

fn episode_matches_query(episode: &Episode, query: &RuntimeFeedbackQuery) -> bool {
    query_matches(
        extra_string(episode, "plan_id")
            .unwrap_or_default()
            .as_str(),
        query.plan_id.as_ref(),
    ) && query_matches(&episode.task_id, query.task_id.as_ref())
        && query_matches(episode_source_id(episode), query.episode_id.as_ref())
        && query_matches(episode_provider(episode).as_str(), query.provider.as_ref())
        && query_matches(episode_model(episode).as_str(), query.model.as_ref())
}

fn provider_model_outcome_matches_query(
    record: &ProviderModelOutcomeRecord,
    query: &RuntimeFeedbackQuery,
) -> bool {
    query_matches_option(record.run_id.as_deref(), query.plan_id.as_ref())
        && query_matches(&record.task_id, query.task_id.as_ref())
        && query_matches_option(record.run_id.as_deref(), query.episode_id.as_ref())
        && query_matches(&record.provider, query.provider.as_ref())
        && query_matches(&record.model, query.model.as_ref())
}

fn efficiency_summary_matches_query(
    record: &EfficiencySummaryRecord,
    query: &RuntimeFeedbackQuery,
) -> bool {
    query_matches(&record.plan_id, query.plan_id.as_ref())
        && query_matches(&record.task_id, query.task_id.as_ref())
        && query_matches_option(record.episode_id.as_deref(), query.episode_id.as_ref())
        && query_matches(&record.provider, query.provider.as_ref())
        && query_matches(&record.model, query.model.as_ref())
}

fn gate_outcome_matches_query(record: &GateOutcomeRecord, query: &RuntimeFeedbackQuery) -> bool {
    query_matches(&record.plan_id, query.plan_id.as_ref())
        && query_matches(&record.task_id, query.task_id.as_ref())
        && query_matches_option(record.episode_id.as_deref(), query.episode_id.as_ref())
        && query_matches_option(record.provider.as_deref(), query.provider.as_ref())
        && query_matches_option(record.model.as_deref(), query.model.as_ref())
}

fn retry_outcome_matches_query(record: &RetryOutcomeRecord, query: &RuntimeFeedbackQuery) -> bool {
    query_matches(&record.plan_id, query.plan_id.as_ref())
        && query_matches(&record.task_id, query.task_id.as_ref())
        && query_matches_option(record.episode_id.as_deref(), query.episode_id.as_ref())
        && query_matches_option(record.provider.as_deref(), query.provider.as_ref())
        && query_matches_option(record.model.as_deref(), query.model.as_ref())
}

fn knowledge_seed_matches_query(
    record: &KnowledgeSeedRecord,
    query: &RuntimeFeedbackQuery,
) -> bool {
    let provider = record
        .metadata
        .get("provider")
        .and_then(serde_json::Value::as_str);
    let model = record.source_model.as_deref().or_else(|| {
        record
            .metadata
            .get("model")
            .and_then(serde_json::Value::as_str)
    });
    query_matches(&record.plan_id, query.plan_id.as_ref())
        && query_matches(&record.task_id, query.task_id.as_ref())
        && query
            .episode_id
            .as_deref()
            .is_none_or(|episode_id| record.source_episodes.iter().any(|id| id == episode_id))
        && query_matches_option(provider, query.provider.as_ref())
        && query_matches_option(model, query.model.as_ref())
}

fn episode_dedupe_key(episode: &Episode) -> String {
    if !episode.episode_id.is_empty() {
        return format!("episode_id:{}", episode.episode_id);
    }
    if !episode.id.is_empty() {
        return format!("id:{}", episode.id);
    }
    format!(
        "fallback:{}:{}:{}:{}",
        episode.agent_id,
        episode.task_id,
        episode.timestamp.to_rfc3339(),
        episode.model
    )
}

async fn count_jsonl_records(path: &Path) -> Result<usize, LearningRuntimeError> {
    let file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(LearningRuntimeError::Io(err)),
    };
    let mut lines = BufReader::new(file).lines();
    let mut count = 0;
    while let Some(line) = lines.next_line().await? {
        if !line.trim().is_empty() {
            count += 1;
        }
    }
    Ok(count)
}

/// Compute the current C-Factor snapshot for `learn_root` and append it to the
/// history log.
///
/// Returns the snapshot that was persisted.
///
/// # Errors
///
/// Returns an error if the snapshot cannot be computed or if the history log
/// cannot be updated.
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
    let social_perceptiveness = social_perceptiveness_from_attribution(
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
        social_perceptiveness,
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

fn social_perceptiveness_from_attribution(
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
    use serde::Serialize;
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

    fn skipped_only_episode() -> Episode {
        let mut ep = sample_episode(true);
        ep.gate_verdicts.clear();
        ep.extra
            .insert("gates_passed".to_string(), serde_json::json!(0_u64));
        ep.extra
            .insert("gates_failed".to_string(), serde_json::json!(0_u64));
        ep.extra
            .insert("gates_skipped".to_string(), serde_json::json!(3_u64));
        ep.extra
            .insert("gates_executed".to_string(), serde_json::json!(0_u64));
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

    fn write_jsonl<T: Serialize>(path: impl AsRef<Path>, values: &[T]) {
        let mut contents = String::new();
        for value in values {
            contents.push_str(&serde_json::to_string(value).unwrap());
            contents.push('\n');
        }
        std::fs::write(path, contents).unwrap();
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

        let mut episode = sample_episode(true);
        episode.backend = "claude_cli".to_string();
        let input = CompletedRunInput::from_episode(episode);
        let update = runtime.record_completed_run(input).await.unwrap();

        assert_eq!(update.episode_logged, ApplyStatus::Applied);
        assert_eq!(update.cost_logged, ApplyStatus::Applied);
        assert_eq!(update.provider_updated, ApplyStatus::Applied);
        assert_eq!(update.provider_model_outcome_recorded, ApplyStatus::Applied);
        assert!(update.extracted_skill_id.is_some());
        assert_eq!(runtime.costs_db().len(), 1);
        let pass_rates = runtime.provider_model_pass_rates(25).await.unwrap();
        assert_eq!(pass_rates.total_records, 1);
        assert_eq!(
            pass_rates.actions[0].action_id,
            "provider:anthropic|model:claude-opus-4-6"
        );
        assert_eq!(pass_rates.actions[0].successes, 1);
        assert_eq!(pass_rates.actions[0].task_types, vec!["bugfix"]);

        let episodes_jsonl = std::fs::read_to_string(&runtime.paths().episodes_jsonl).unwrap();
        let persisted: Episode = serde_json::from_str(episodes_jsonl.lines().next().unwrap())
            .expect("persisted episode");
        assert_eq!(persisted.backend, "claude_cli");
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
    async fn skipped_only_gate_runs_are_blocked_not_passed_and_do_not_update_learning() {
        let tmp = TempDir::new().unwrap();
        let mut runtime = LearningRuntime::open_with_models(
            LearningPaths::under(tmp.path()),
            RegressionConfig::default(),
            vec!["claude-opus-4-6".to_string()],
        )
        .await
        .unwrap();
        runtime.set_update_frequency(UpdateFrequency {
            router_every_n_episodes: 1,
            gate_thresholds_every_n: 1,
            experiments_every_n: 1,
            skill_mining_every_n: 1,
            pattern_discovery_every_n: 1,
            distiller_every_n: 1,
        });

        let mut experiment = PromptExperiment::new(
            "skip-only-exp",
            "model-routing",
            vec![PromptVariant {
                id: "blocked".to_string(),
                name: "Blocked".to_string(),
                section_name: "model-routing".to_string(),
                content: String::new(),
                slug: Some("claude-opus-4-6".to_string()),
                active: true,
            }],
        );
        experiment.min_trials_per_variant = 100;
        runtime.experiment_store().lock().register(experiment);

        let mut input = CompletedRunInput::from_episode(skipped_only_episode())
            .with_task_metric(sample_metric(1, true, 0.42));
        input.provider = Some("anthropic".to_string());
        input.playbook_id = Some("playbook-skip-only".to_string());
        input.playbook_rule_id = Some("rule-skip-only".to_string());
        input.matched_skill_id = Some("skill-skip-only".to_string());
        input.experiment_variant_id = Some("blocked".to_string());

        let update = runtime.record_completed_run(input).await.unwrap();

        assert_eq!(update.episode_logged, ApplyStatus::Applied);
        assert_eq!(update.cost_logged, ApplyStatus::Applied);
        assert_eq!(update.provider_model_outcome_recorded, ApplyStatus::Applied);
        assert_eq!(update.efficiency_summary_recorded, ApplyStatus::Applied);
        assert_eq!(update.gate_outcomes_recorded, 0);
        assert_eq!(update.provider_updated, ApplyStatus::Skipped);
        assert_eq!(update.playbook_updated, ApplyStatus::Skipped);
        assert_eq!(update.playbook_rule_updated, ApplyStatus::Skipped);
        assert_eq!(update.matched_skill_updated, ApplyStatus::Skipped);
        assert_eq!(update.reflection_recorded, ApplyStatus::Skipped);
        assert_eq!(update.reflection_candidate_updated, ApplyStatus::Skipped);
        assert_eq!(update.knowledge_seed_recorded, ApplyStatus::Skipped);
        assert_eq!(update.router_updated, false);
        assert!(update.extracted_skill_id.is_none());
        assert!(update.regression_report.is_none());
        assert!(!update.patterns_ingested);

        assert_eq!(runtime.local_reward_score("router", "claude-opus-4-6"), 0.5);
        assert_eq!(runtime.local_reward_score("skill", "skill-skip-only"), 0.5);
        assert_eq!(
            runtime.local_reward_score("playbook_rule", "rule-skip-only"),
            0.5
        );
        assert_eq!(runtime.cascade_router().total_observations(), 0);
        assert_eq!(runtime.skill_library().len(), 0);
        assert_eq!(runtime.pattern_miner().lock().total_episodes(), 0);
        assert!(!runtime.paths().cfactor_jsonl.exists());
        assert!(!runtime.paths().task_metrics_jsonl.exists());

        let persisted = std::fs::read_to_string(&runtime.paths().episodes_jsonl).unwrap();
        let episode: Episode = serde_json::from_str(persisted.lines().next().unwrap())
            .expect("persisted skip-only episode");
        assert!(!episode.success);
        assert_eq!(
            episode
                .failure_reason
                .as_deref()
                .expect("skip-only failure reason"),
            "all gates skipped"
        );
        assert_eq!(
            episode
                .extra
                .get("provider_model_outcome_status")
                .and_then(serde_json::Value::as_str),
            Some("blocked")
        );
        assert_eq!(
            episode
                .extra
                .get("gate_summary")
                .and_then(serde_json::Value::as_str),
            Some("0 passed, 0 failed, 3 skipped")
        );
        assert_eq!(
            episode
                .extra
                .get("gate_counts")
                .and_then(serde_json::Value::as_object)
                .and_then(|counts| counts.get("skipped"))
                .and_then(serde_json::Value::as_u64),
            Some(3)
        );
        assert_eq!(
            episode
                .extra
                .get("gate_pass_rate")
                .and_then(serde_json::Value::as_f64),
            Some(0.0)
        );
        assert_eq!(
            gate_counts_from_episode(&episode)
                .expect("gate counts")
                .summary(),
            "0 passed, 0 failed, 3 skipped"
        );
        assert!(
            EfficiencySummaryRecord::from_episode(&episode)
                .gate_passed
                .is_none()
        );
        assert!(GateOutcomeRecord::from_episode(&episode).is_empty());
        assert!(KnowledgeSeedRecord::from_successful_episode(&episode).is_none());

        let experiment_store = runtime.experiment_store().lock();
        let variant_trials = experiment_store
            .get("skip-only-exp")
            .and_then(|exp| exp.stats.get("blocked"))
            .map(|stats| stats.trials);
        assert_eq!(variant_trials, Some(0));
    }

    #[tokio::test]
    async fn append_efficiency_event_updates_section_effectiveness_registry() {
        let tmp = TempDir::new().unwrap();
        let runtime = LearningRuntime::open_under(tmp.path()).await.unwrap();
        let event = AgentEfficiencyEvent {
            role: "Implementer".to_string(),
            gate_passed: true,
            prompt_sections: vec![
                crate::efficiency::PromptSectionMeta {
                    name: "workspace_map".to_string(),
                    tokens: 120,
                    priority: 2,
                    was_truncated: false,
                    was_dropped: false,
                },
                crate::efficiency::PromptSectionMeta {
                    name: "playbook".to_string(),
                    tokens: 0,
                    priority: 0,
                    was_truncated: false,
                    was_dropped: true,
                },
            ],
            ..AgentEfficiencyEvent::default()
        };

        runtime.append_efficiency_event(&event).await.unwrap();

        let snapshot = runtime.section_effectiveness_snapshot();
        let included = snapshot
            .get("workspace_map", "Implementer")
            .expect("included section recorded");
        assert_eq!(included.included_trials, 1);
        assert_eq!(included.included_passes, 1);

        let excluded = snapshot
            .get("playbook", "Implementer")
            .expect("dropped section recorded");
        assert_eq!(excluded.excluded_trials, 1);
        assert_eq!(excluded.excluded_passes, 1);
        assert!(runtime.paths().section_effects_json.exists());
    }

    #[tokio::test]
    async fn project_learning_snapshot_reads_episode_efficiency_router_and_knowledge_artifacts() {
        let tmp = TempDir::new().unwrap();
        let workdir = tmp.path();
        let roko = workdir.join(".roko");
        let memory_dir = roko.join("memory");
        let learn_dir = roko.join("learn");
        let neuro_dir = roko.join("neuro");
        std::fs::create_dir_all(&memory_dir).unwrap();
        std::fs::create_dir_all(&learn_dir).unwrap();
        std::fs::create_dir_all(&neuro_dir).unwrap();

        let mut memory_episode = sample_episode(true);
        memory_episode.id = "episode-memory".to_string();
        memory_episode.episode_id = "episode-memory".to_string();
        memory_episode.task_id = "task-memory".to_string();
        let duplicate_episode = memory_episode.clone();
        let mut legacy_episode = sample_episode(false);
        legacy_episode.id = "episode-legacy".to_string();
        legacy_episode.episode_id = "episode-legacy".to_string();
        legacy_episode.task_id = "task-legacy".to_string();

        write_jsonl(memory_dir.join("episodes.jsonl"), &[memory_episode]);
        write_jsonl(
            roko.join("episodes.jsonl"),
            &[duplicate_episode, legacy_episode],
        );

        let efficiency_event = AgentEfficiencyEvent {
            agent_id: "agent-1".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            model_used: "claude-sonnet-4-5".to_string(),
            gate_passed: true,
            cost_usd: 0.25,
            ..AgentEfficiencyEvent::default()
        };
        write_jsonl(learn_dir.join("efficiency.jsonl"), &[efficiency_event]);
        std::fs::write(
            learn_dir.join("cascade-router.json"),
            serde_json::json!({"confidence_stats": {"claude-sonnet-4-5": {"trials": 1}}})
                .to_string(),
        )
        .unwrap();
        std::fs::write(
            neuro_dir.join("knowledge.jsonl"),
            "{\"id\":\"k1\"}\n{\"id\":\"k2\"}\n",
        )
        .unwrap();

        let snapshot = read_project_learning_snapshot(workdir).await.unwrap();

        assert_eq!(
            snapshot.episodes.len(),
            2,
            "duplicate episode should be skipped"
        );
        assert_eq!(snapshot.efficiency_events.len(), 1);
        assert_eq!(snapshot.knowledge_entries, 2);
        assert!(snapshot.cascade_router.is_some());
        assert_eq!(snapshot.episode_paths.len(), 2);
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
    fn social_perceptiveness_uses_prior_output_attributions() {
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

        let score = social_perceptiveness_from_attribution(&records, Duration::from_secs(60));
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

        let winner_artifact = std::fs::read_to_string(&runtime.paths().experiment_winners_json)
            .expect("experiment winners artifact");
        let winner_summaries: Vec<roko_core::ExperimentWinnerSummary> =
            serde_json::from_str(&winner_artifact).expect("winner summary json");
        assert_eq!(winner_summaries.len(), 1);
        assert_eq!(winner_summaries[0].experiment_id, "model-routing-exp");
        assert_eq!(winner_summaries[0].winner_variant_id, "haiku");

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
            balance: 1.0,
            demurrage_rate: 0.01,
            last_decay_at_ms: Utc::now().timestamp_millis(),
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
    async fn completed_gate_run_persists_post_gate_reflection() {
        let tmp = TempDir::new().unwrap();
        let runtime = LearningRuntime::open_under(tmp.path()).await.unwrap();
        let mut ep = sample_episode(false);
        ep.gate_verdicts.push(
            crate::episode_logger::GateVerdict::new("compile", false).with_signature("E0308"),
        );
        ep.reflection = Some(
            "Fix crates/roko-learn/src/lib.rs before retrying E0308 type_mismatch".to_string(),
        );

        let update = runtime
            .record_completed_run(CompletedRunInput::from_episode(ep))
            .await
            .unwrap();

        assert_eq!(update.reflection_recorded, ApplyStatus::Applied);
        assert_eq!(update.reflection_candidate_updated, ApplyStatus::Applied);
        let store = PostGateReflectionStore::load(&runtime.paths().post_gate_reflections_json);
        assert_eq!(store.records.len(), 1);
        assert_eq!(store.candidates.len(), 1);
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

        // Route subsystem should have observed the model decision.
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

    #[tokio::test]
    async fn generation_outcome_partial_success_does_not_seed_learning() {
        let tmp = TempDir::new().unwrap();
        let runtime = LearningRuntime::open_under(tmp.path()).await.unwrap();

        let outcome = GenerationOutcome {
            process_success: true,
            artifact_valid: false,
            validation_report: None,
        };

        runtime
            .record_generation_outcome("prd:plan:test", "claude-sonnet-4-6", &outcome)
            .await
            .unwrap();

        let snapshot = runtime
            .query_feedback(&RuntimeFeedbackQuery::default())
            .await
            .unwrap();

        assert_eq!(snapshot.episodes.len(), 1);
        assert!(!snapshot.episodes[0].success);
        assert!(snapshot.knowledge_seeds.is_empty());
        assert_eq!(snapshot.provider_model_outcomes.len(), 1);
    }
}
