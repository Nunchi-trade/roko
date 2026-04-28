//! Public dream runner facade.
//!
//! The runner wraps the existing dream-cycle implementation with a small API
//! that can be called from the CLI and from other workspace crates without
//! reimplementing the consolidation logic.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use cron::Schedule;
use roko_agent::provider::{AgentOptions, create_agent_for_model, is_known_protocol_command};
use roko_agent::{Agent, AgentResult};
use roko_core::config::schema::RokoConfig;
use roko_core::foundation::{
    CachePolicy, CallerIdentity, ChatMessage, MessageRole, ModelCallRequest, ModelCaller,
};
use roko_core::{Body, Context as RokoContext, Engram, Kind, Provenance};
use roko_learn::{episode_logger::EpisodeLogger, playbook::PlaybookStore};
use roko_neuro::{
    KnowledgeStore,
    tier_progression::{InsightRecord, TierProgression as NeuroTierProgression},
};
use serde::{Deserialize, Serialize};

use crate::cycle::{AgentDispatcher, DreamCycle, DreamCycleReport};
use crate::imagination::ImaginationMode;
use crate::phase2::advanced::{DreamJournal, DreamJournalEntry};
use crate::replay::{DreamReplayBatch, DreamReplayPolicy, select_replay_episodes};

/// Public alias for the replay input episodes.
pub type Episode = roko_learn::episode_logger::Episode;

/// Public alias for replay-generated insights.
pub type Insight = InsightRecord;

/// Public alias for the dream report written by consolidation.
pub type DreamReport = DreamCycleReport;

/// Configuration for the external agent used during consolidation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DreamAgentConfig {
    /// CLI command to invoke.
    pub command: String,
    /// Extra arguments for the CLI command.
    pub args: Vec<String>,
    /// Preferred model slug when the CLI supports one.
    pub model: Option<String>,
    /// Whether Claude should run in bare mode.
    pub bare_mode: bool,
    /// Claude reasoning effort.
    pub effort: String,
    /// Optional fallback model for Claude.
    pub fallback_model: Option<String>,
    /// Subprocess timeout in milliseconds.
    pub timeout_ms: u64,
    /// Environment variables passed through to the CLI.
    pub env: Vec<(String, String)>,
}

impl DreamAgentConfig {
    fn build_agent(&self, workdir: &Path) -> Result<DreamReviewAgent> {
        // TODO(gateway): remove direct construction once all callers provide a ModelCaller.
        let mut routing_config = roko_core::config::load_config(workdir)
            .with_context(|| format!("load routing config from {}", workdir.display()))?;
        routing_config.apply_process_env();
        let has_routing = !routing_config.providers.is_empty() || !routing_config.models.is_empty();

        if has_routing {
            let model = self
                .model
                .clone()
                .or_else(|| {
                    (!routing_config.agent.default_model.trim().is_empty())
                        .then(|| routing_config.agent.default_model.clone())
                })
                .ok_or_else(|| anyhow::anyhow!("dream review routing needs a configured model"))?;
            let agent = create_agent_for_model(
                &routing_config,
                &model,
                self.agent_options(workdir, self.args.clone(), format!("dream-review:{model}")),
            )
            .with_context(|| format!("create dream review agent for model {model}"))?;
            Ok(DreamReviewAgent::Agent { inner: agent })
        } else {
            let model = if self.command == "claude" {
                self.model
                    .clone()
                    .unwrap_or_else(|| "claude-opus-4-6".to_string())
            } else {
                self.model.clone().unwrap_or_else(|| self.command.clone())
            };
            let mut synthesized_config = RokoConfig::default();
            synthesized_config.agent.command = Some(self.command.clone());
            synthesized_config.agent.default_model = model.clone();
            if self.command == "claude" {
                synthesized_config.agent.default_backend = "claude".to_string();
            } else if is_known_protocol_command(&self.command) {
                synthesized_config.agent.default_backend = self.command.clone();
            }

            let mut extra_args = self.args.clone();
            if let Some(fallback_model) = &self.fallback_model {
                extra_args.push("--fallback-model".to_string());
                extra_args.push(fallback_model.clone());
            }

            let agent = create_agent_for_model(
                &synthesized_config,
                &model,
                self.agent_options(workdir, extra_args, format!("dream-review:{model}")),
            )
            .with_context(|| format!("create dream review subprocess agent for model {model}"))?;
            Ok(DreamReviewAgent::Agent { inner: agent })
        }
    }

    fn build_agent_via_gateway(
        &self,
        workdir: &Path,
        model_caller: Arc<dyn ModelCaller>,
    ) -> Result<DreamReviewAgent> {
        let model = self.gateway_model(workdir)?;
        Ok(DreamReviewAgent::Gateway {
            name: format!("dream-review:{model}"),
            model,
            model_caller,
        })
    }

    fn gateway_model(&self, workdir: &Path) -> Result<String> {
        if let Some(model) = self.model.clone().filter(|model| !model.trim().is_empty()) {
            return Ok(model);
        }

        let mut routing_config = roko_core::config::load_config(workdir)
            .with_context(|| format!("load routing config from {}", workdir.display()))?;
        routing_config.apply_process_env();
        if !routing_config.agent.default_model.trim().is_empty() {
            return Ok(routing_config.agent.default_model);
        }

        if self.command == "claude" {
            Ok("claude-opus-4-6".to_string())
        } else {
            Ok(self.command.clone())
        }
    }

    fn agent_options(&self, workdir: &Path, extra_args: Vec<String>, name: String) -> AgentOptions {
        AgentOptions {
            command: Some(self.command.clone()),
            timeout_ms: Some(self.timeout_ms),
            system_prompt: None,
            cached_content: None,
            tools: None,
            mcp_config: None,
            working_dir: Some(workdir.to_path_buf()),
            provider_semaphores: None,
            env: self.env.clone(),
            extra_args,
            effort: Some(self.effort.clone()),
            bare_mode: self.bare_mode,
            dangerously_skip_permissions: false,
            name,
        }
    }
}

/// Scheduler and agent settings used by the runner and daemon loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DreamLoopConfig {
    /// Whether automatic dreaming is enabled.
    pub auto_dream: bool,
    /// Idle threshold in minutes before a dream may run.
    pub idle_threshold_mins: u64,
    /// Minimum number of new episodes required before dreaming.
    pub min_episodes_for_dream: usize,
    /// Agent backend used to review the dream batch.
    pub agent: DreamAgentConfig,
}

/// Backwards-compatible config alias for callers that want a shorter name.
pub type DreamConfig = DreamLoopConfig;

/// Budget consumed by dream replay and consolidation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DreamBudget {
    /// Maximum replay tokens allowed.
    pub max_tokens: u64,
    /// Maximum dollar cost allowed.
    pub max_cost_usd: f64,
    /// Maximum wall-clock duration allowed, in seconds.
    pub max_duration_secs: u64,
    /// Tokens consumed so far.
    pub consumed_tokens: u64,
    /// Dollar cost consumed so far.
    pub consumed_cost_usd: f64,
    /// Wall-clock seconds consumed so far.
    pub consumed_duration_secs: u64,
}

impl Default for DreamBudget {
    fn default() -> Self {
        Self {
            max_tokens: u64::MAX,
            max_cost_usd: f64::MAX,
            max_duration_secs: u64::MAX,
            consumed_tokens: 0,
            consumed_cost_usd: 0.0,
            consumed_duration_secs: 0,
        }
    }
}

impl DreamBudget {
    /// Consume the accounting for a single episode.
    pub fn consume_episode(&mut self, episode: &Episode) {
        let usage_tokens = episode
            .usage
            .input_tokens
            .saturating_add(episode.usage.output_tokens);
        let tokens = episode.tokens_used.max(usage_tokens);
        self.consumed_tokens = self.consumed_tokens.saturating_add(tokens);
        self.consumed_cost_usd += episode.usage.cost_usd;
        self.consumed_duration_secs = self
            .consumed_duration_secs
            .saturating_add(episode.duration_secs.max(0.0).round() as u64);
    }

    /// Remaining budget fraction across all axes.
    #[must_use]
    pub fn remaining_fraction(self) -> f64 {
        let token_fraction = remaining_fraction(self.consumed_tokens, self.max_tokens);
        let cost_fraction = remaining_fraction_f64(self.consumed_cost_usd, self.max_cost_usd);
        let duration_fraction =
            remaining_fraction(self.consumed_duration_secs, self.max_duration_secs);
        token_fraction.min(cost_fraction).min(duration_fraction)
    }

    /// Whether the budget is exhausted on any axis.
    #[must_use]
    pub fn exhausted(self) -> bool {
        self.consumed_tokens >= self.max_tokens
            || self.consumed_cost_usd >= self.max_cost_usd
            || self.consumed_duration_secs >= self.max_duration_secs
    }
}

/// Dream scheduling trigger kinds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DreamTrigger {
    /// Idle gap between task dispatches.
    Idle,
    /// Cron-like schedule.
    Scheduled,
    /// Manual command invocation.
    Manual,
    /// Accumulated episode count since last dream.
    EpisodeCount,
    /// Bus-reactive trigger from a high-value engram (DREAM-09).
    BusPulse {
        /// Hash or ID of the engram that triggered the dream.
        #[serde(default)]
        engram_hash: String,
    },
    /// Coordination pattern trigger (INT-19): conductor compound patterns
    /// trigger dream consolidation so the system can process and learn from
    /// the detected coordination issues.
    CoordinationPattern {
        /// Name of the compound pattern that triggered the dream.
        #[serde(default)]
        pattern_name: String,
        /// Watchers that contributed to the pattern detection.
        #[serde(default)]
        contributing_watchers: Vec<String>,
    },
}

impl DreamTrigger {
    /// Stable lowercase label.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Scheduled => "scheduled",
            Self::Manual => "manual",
            Self::EpisodeCount => "episode_count",
            Self::BusPulse { .. } => "bus_pulse",
            Self::CoordinationPattern { .. } => "coordination_pattern",
        }
    }
}

/// Policy for checking dream eligibility after a plan completes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanCompletionTriggerPolicy {
    /// Minimum new episodes since the latest dream report.
    pub min_episodes: usize,
    /// Minimum elapsed time since the latest dream report, in seconds.
    pub min_elapsed_secs: u64,
    /// Whether callers should run consolidation synchronously.
    pub synchronous: bool,
}

impl Default for PlanCompletionTriggerPolicy {
    fn default() -> Self {
        Self {
            min_episodes: 5,
            min_elapsed_secs: 900,
            synchronous: false,
        }
    }
}

impl PlanCompletionTriggerPolicy {
    /// Return a dream trigger when the observed plan-completion state is
    /// eligible for consolidation.
    #[must_use]
    pub fn should_trigger(
        &self,
        episodes_since_last_dream: usize,
        seconds_since_last_dream: u64,
    ) -> Option<DreamTrigger> {
        if self.min_episodes > 0 && episodes_since_last_dream >= self.min_episodes {
            return Some(DreamTrigger::EpisodeCount);
        }
        if seconds_since_last_dream >= self.min_elapsed_secs && episodes_since_last_dream >= 2 {
            return Some(DreamTrigger::Idle);
        }
        None
    }
}

/// Criteria for determining whether an engram is "dream-worthy" (DREAM-09).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BusPulseTriggerConfig {
    /// Minimum engram score to trigger a dream (default 0.7).
    pub min_score: f64,
    /// Minimum interval between bus-triggered dreams (default 30 minutes).
    pub min_interval_secs: u64,
    /// Engram kinds that can trigger dreams.
    pub trigger_kinds: Vec<String>,
}

impl Default for BusPulseTriggerConfig {
    fn default() -> Self {
        Self {
            min_score: 0.7,
            min_interval_secs: 1800,
            trigger_kinds: vec![
                "gate_verdict".to_string(),
                "episode_complete".to_string(),
                "knowledge_ingested".to_string(),
            ],
        }
    }
}

impl BusPulseTriggerConfig {
    /// Check if an engram with the given score and kind qualifies as dream-worthy.
    #[must_use]
    pub fn is_dream_worthy(&self, score: f64, kind: &str) -> bool {
        score >= self.min_score
            && self
                .trigger_kinds
                .iter()
                .any(|k| k.eq_ignore_ascii_case(kind))
    }
}

/// Scheduling policy for when dream cycles may fire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamSchedulePolicy {
    /// Whether automatic dreaming is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Idle threshold in minutes.
    #[serde(default = "default_idle_threshold_mins")]
    pub idle_threshold_mins: u64,
    /// Optional cron expression for scheduled dreaming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_cron: Option<String>,
    /// Whether manual triggering is permitted.
    #[serde(default = "default_true")]
    pub manual_enabled: bool,
    /// Factor applied when dream output quality is high.
    #[serde(default = "default_quality_gain")]
    pub quality_gain: f64,
    /// Factor applied when dream output quality is low.
    #[serde(default = "default_quality_penalty")]
    pub quality_penalty: f64,
    /// Episode count threshold: trigger after N new episodes since last dream.
    /// Zero disables this trigger.
    #[serde(default)]
    pub episode_count_trigger: usize,
}

impl Default for DreamSchedulePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            idle_threshold_mins: 15,
            scheduled_cron: None,
            manual_enabled: true,
            quality_gain: 0.75,
            quality_penalty: 1.25,
            episode_count_trigger: 0,
        }
    }
}

impl DreamSchedulePolicy {
    /// Return the adapted idle delay for the current dream quality.
    #[must_use]
    pub fn idle_delay(
        &self,
        report: Option<&DreamReport>,
        budget: Option<&DreamBudget>,
    ) -> Duration {
        let mut multiplier = 1.0;
        if let Some(report) = report {
            let quality = dream_quality_score(report);
            if quality >= 1.5 {
                multiplier *= self.quality_gain;
            } else if quality <= 0.5 {
                multiplier *= self.quality_penalty;
            }
        }
        if let Some(budget) = budget
            && budget.remaining_fraction() < 0.20
        {
            multiplier *= self.quality_penalty;
        }
        let minutes = (self.idle_threshold_mins as f64 * multiplier)
            .max(1.0)
            .round() as u64;
        Duration::from_secs(minutes.saturating_mul(60))
    }

    /// Return the next cron fire delay, if a cron expression is configured.
    #[must_use]
    pub fn cron_delay(&self, now: DateTime<Utc>) -> Option<Duration> {
        let expression = self.scheduled_cron.as_ref()?.trim();
        if expression.is_empty() {
            return None;
        }
        let schedule = Schedule::from_str(expression).ok()?;
        let next = schedule.after(&now).next()?;
        (next - now).to_std().ok()
    }

    /// Determine whether a trigger kind is allowed under the current policy.
    #[must_use]
    pub fn allows(&self, trigger: &DreamTrigger) -> bool {
        match trigger {
            DreamTrigger::Idle | DreamTrigger::Scheduled => self.enabled,
            DreamTrigger::Manual => self.manual_enabled,
            DreamTrigger::EpisodeCount => self.enabled && self.episode_count_trigger > 0,
            DreamTrigger::BusPulse { .. } => self.enabled,
            // INT-19: coordination patterns always allowed when dreams are enabled.
            DreamTrigger::CoordinationPattern { .. } => self.enabled,
        }
    }

    /// Resolve the next delay for a specific trigger type.
    #[must_use]
    pub fn trigger_delay(
        &self,
        trigger: &DreamTrigger,
        report: Option<&DreamReport>,
        budget: Option<&DreamBudget>,
        now: DateTime<Utc>,
    ) -> Option<Duration> {
        if !self.allows(trigger) {
            return None;
        }
        match trigger {
            DreamTrigger::Idle => Some(self.idle_delay(report, budget)),
            DreamTrigger::Scheduled => self.cron_delay(now),
            DreamTrigger::Manual => Some(Duration::ZERO),
            DreamTrigger::EpisodeCount => Some(Duration::ZERO),
            DreamTrigger::BusPulse { .. } => Some(Duration::ZERO),
            // INT-19: coordination pattern triggers fire immediately.
            DreamTrigger::CoordinationPattern { .. } => Some(Duration::ZERO),
        }
    }
}

/// Heartbeat cadence and delta-loop policy for daemon-friendly polling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DreamHeartbeatPolicy {
    /// How often a daemon should poll for heartbeat updates, in seconds.
    #[serde(default = "default_heartbeat_tick_secs")]
    pub tick_interval_secs: u64,
    /// Minimum age of the last consolidated dream before a delta loop may fire.
    #[serde(default = "default_delta_interval_mins")]
    pub delta_interval_mins: u64,
    /// Minimum idle window before active plan work is considered quiescent.
    #[serde(default = "default_idle_grace_mins")]
    pub idle_grace_mins: u64,
}

impl Default for DreamHeartbeatPolicy {
    fn default() -> Self {
        Self {
            tick_interval_secs: default_heartbeat_tick_secs(),
            delta_interval_mins: default_delta_interval_mins(),
            idle_grace_mins: default_idle_grace_mins(),
        }
    }
}

/// Runtime snapshot describing whether the dreams-side heartbeat may fire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamHeartbeatReport {
    /// Cutoff used to decide which episodes are recent enough to matter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processed_through: Option<DateTime<Utc>>,
    /// Number of recent episodes seen after the cutoff.
    pub recent_episode_count: usize,
    /// Timestamp of the most recent episode in the active window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_episode_at: Option<DateTime<Utc>>,
    /// Timestamp of the most recent consolidated dream run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_dream_at: Option<DateTime<Utc>>,
    /// Heartbeat polling interval, in seconds.
    pub heartbeat_due_in_secs: u64,
    /// Delta-loop interval, in minutes.
    pub delta_interval_mins: u64,
    /// Idle grace period, in minutes.
    pub idle_grace_mins: u64,
    /// Whether enough fresh episodes exist to justify a delta pass.
    pub enough_recent_episodes: bool,
    /// Whether the plan still looks active enough to suppress delta.
    pub paused_for_active_plan: bool,
    /// Idle age of the most recent episode, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_for_secs: Option<u64>,
    /// Delay until the next delta loop is due, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta_due_in_secs: Option<u64>,
    /// Whether the delta loop is immediately runnable.
    pub delta_ready: bool,
}

/// Settings for intensive dream mode: longer, more thorough cycles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntensiveMode {
    /// Whether intensive mode is currently active.
    #[serde(default)]
    pub active: bool,
    /// Replay iteration multiplier (applied to default replay count).
    #[serde(default = "default_intensive_replay_multiplier")]
    pub replay_multiplier: u32,
    /// Maximum counterfactual exploration depth (deeper = more thorough).
    #[serde(default = "default_intensive_counterfactual_depth")]
    pub counterfactual_depth: usize,
    /// Threat rehearsal scenario limit (0 = use default).
    #[serde(default = "default_intensive_rehearsal_limit")]
    pub rehearsal_limit: usize,
    /// Episode backlog high-water mark: trigger intensive mode when
    /// unreplayed episodes exceed this count (default 50).
    #[serde(default = "default_backlog_high_water")]
    pub backlog_high_water: usize,
    /// Episode backlog low-water mark: exit intensive mode when
    /// unreplayed episodes drop below this count (default 20).
    #[serde(default = "default_backlog_low_water")]
    pub backlog_low_water: usize,
}

impl Default for IntensiveMode {
    fn default() -> Self {
        Self {
            active: false,
            replay_multiplier: default_intensive_replay_multiplier(),
            counterfactual_depth: default_intensive_counterfactual_depth(),
            rehearsal_limit: default_intensive_rehearsal_limit(),
            backlog_high_water: default_backlog_high_water(),
            backlog_low_water: default_backlog_low_water(),
        }
    }
}

impl IntensiveMode {
    /// Check if intensive mode should activate based on episode backlog.
    ///
    /// Returns the adjusted `max_episodes` if intensive, otherwise `None`.
    #[must_use]
    pub fn check_activation(&self, unreplayed_count: usize, default_max: usize) -> Option<usize> {
        if unreplayed_count >= self.backlog_high_water {
            Some(
                default_max
                    .saturating_mul(self.replay_multiplier as usize)
                    .max(default_max),
            )
        } else {
            None
        }
    }

    /// Check if intensive mode should deactivate.
    #[must_use]
    pub fn should_deactivate(&self, unreplayed_count: usize) -> bool {
        unreplayed_count < self.backlog_low_water
    }
}

/// Combined runtime controls for replay, budgeting, and scheduling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamRuntimeControls {
    /// Replay planner configuration.
    #[serde(default)]
    pub replay: DreamReplayPolicy,
    /// Optional offline consolidation budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<DreamBudget>,
    /// Scheduling policy.
    #[serde(default)]
    pub schedule: DreamSchedulePolicy,
    /// Heartbeat cadence and delta-loop policy.
    #[serde(default)]
    pub heartbeat: DreamHeartbeatPolicy,
    /// Creativity mode used for REM imagination.
    #[serde(default)]
    pub imagination_mode: ImaginationMode,
    /// Whether to emit threat warnings from repeated failures.
    #[serde(default = "default_true")]
    pub threat_simulation: bool,
    /// Minimum severity required before a threat becomes a warning entry.
    #[serde(default = "default_threat_floor")]
    pub threat_severity_floor: f64,
    /// Intensive mode settings for deeper dream cycles.
    #[serde(default)]
    pub intensive: IntensiveMode,
}

impl Default for DreamRuntimeControls {
    fn default() -> Self {
        Self {
            replay: DreamReplayPolicy::default(),
            budget: None,
            schedule: DreamSchedulePolicy::default(),
            heartbeat: DreamHeartbeatPolicy::default(),
            imagination_mode: ImaginationMode::default(),
            threat_simulation: true,
            threat_severity_floor: 0.20,
            intensive: IntensiveMode::default(),
        }
    }
}

impl DreamRuntimeControls {
    /// Summarize the current heartbeat and delta-loop readiness.
    #[must_use]
    pub fn heartbeat_report(
        &self,
        episodes: &[Episode],
        processed_through: Option<DateTime<Utc>>,
        last_dream_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
        min_recent_episodes: usize,
    ) -> DreamHeartbeatReport {
        let recent_episodes: Vec<&Episode> = episodes
            .iter()
            .filter(|episode| processed_through.is_none_or(|ts| episode.timestamp > ts))
            .collect();
        let recent_episode_count = recent_episodes.len();
        let latest_episode_at = recent_episodes
            .iter()
            .map(|episode| episode.timestamp.clone())
            .max();
        let idle_for_secs =
            latest_episode_at.map(|ts| now.signed_duration_since(ts).num_seconds().max(0) as u64);
        let enough_recent_episodes = recent_episode_count >= min_recent_episodes;
        let paused_for_active_plan = idle_for_secs
            .is_some_and(|secs| secs < self.heartbeat.idle_grace_mins.saturating_mul(60));
        let delta_due_in_secs = if enough_recent_episodes && !paused_for_active_plan {
            last_dream_at.map(|last_dream_at| {
                let delta_ready_at = last_dream_at
                    + chrono::Duration::minutes(self.heartbeat.delta_interval_mins as i64);
                if delta_ready_at <= now {
                    0
                } else {
                    delta_ready_at
                        .signed_duration_since(now)
                        .num_seconds()
                        .max(0) as u64
                }
            })
        } else {
            None
        };

        DreamHeartbeatReport {
            processed_through,
            recent_episode_count,
            latest_episode_at,
            last_dream_at,
            heartbeat_due_in_secs: self.heartbeat.tick_interval_secs.max(1),
            delta_interval_mins: self.heartbeat.delta_interval_mins.max(1),
            idle_grace_mins: self.heartbeat.idle_grace_mins.max(1),
            enough_recent_episodes,
            paused_for_active_plan,
            idle_for_secs,
            delta_due_in_secs,
            delta_ready: delta_due_in_secs == Some(0),
        }
    }
}

/// Public facade for dream replay, consolidation, and scheduling.
#[derive(Clone)]
pub struct DreamRunner {
    workdir: PathBuf,
    config: DreamLoopConfig,
    controls: DreamRuntimeControls,
    model_caller: Option<Arc<dyn ModelCaller>>,
}

impl std::fmt::Debug for DreamRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DreamRunner")
            .field("workdir", &self.workdir)
            .field("config", &self.config)
            .field("controls", &self.controls)
            .field("model_caller", &self.model_caller.is_some())
            .finish()
    }
}

impl DreamRunner {
    /// Construct a dream runner for a working directory.
    #[must_use]
    pub fn new(workdir: impl Into<PathBuf>, config: DreamLoopConfig) -> Self {
        Self {
            workdir: workdir.into(),
            config,
            controls: DreamRuntimeControls::default(),
            model_caller: None,
        }
    }

    /// Construct a dream runner with explicit runtime controls.
    #[must_use]
    pub fn with_controls(
        workdir: impl Into<PathBuf>,
        config: DreamLoopConfig,
        controls: DreamRuntimeControls,
    ) -> Self {
        Self {
            workdir: workdir.into(),
            config,
            controls,
            model_caller: None,
        }
    }

    /// Construct a dream runner that dispatches review inference through the gateway.
    #[must_use]
    pub fn with_model_caller(mut self, model_caller: Arc<dyn ModelCaller>) -> Self {
        self.model_caller = Some(model_caller);
        self
    }

    /// Replay the supplied episodes into durable insights without mutating the
    /// workspace.
    ///
    /// # Errors
    ///
    /// This method is currently infallible and always returns `Ok`.
    #[allow(clippy::needless_pass_by_value)]
    #[must_use]
    pub fn replay_insights(&self, episodes: &[Episode]) -> Result<Vec<Insight>> {
        let replay = self.plan_replay(episodes);
        let progression = NeuroTierProgression::default();
        let report = progression.analyze(&replay.episodes);
        Ok(report.insights)
    }

    /// Select a replay batch using the configured NREM mode.
    #[must_use]
    pub fn plan_replay(&self, episodes: &[Episode]) -> DreamReplayBatch {
        select_replay_episodes(episodes, &self.controls.replay, Utc::now())
    }

    /// Load the latest persisted dream report from `.roko/dreams/`.
    ///
    /// # Errors
    ///
    /// Returns an error if the report directory cannot be scanned, if the
    /// newest report cannot be read, or if the report contents are invalid
    /// JSON.
    pub fn latest_report(&self) -> Result<Option<DreamReport>> {
        load_latest_dream_report(&self.report_dir())
    }

    /// Summarize the current heartbeat and delta-loop status.
    ///
    /// # Errors
    ///
    /// Returns an error if the episode log cannot be read.
    pub fn heartbeat_report(&self) -> Result<DreamHeartbeatReport> {
        let episodes_path = self.workdir.join(".roko").join("episodes.jsonl");
        let episodes = block_on(EpisodeLogger::read_all_lossy(&episodes_path))
            .with_context(|| format!("read episode log from {}", episodes_path.display()))?;
        let latest_report = self.latest_report()?;
        Ok(self.heartbeat_snapshot(&episodes, latest_report.as_ref(), Utc::now()))
    }

    /// Run a full dream consolidation cycle against the workspace.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying dream cycle fails while reading the
    /// episode log, synthesizing insights, writing knowledge, or producing the
    /// consolidated report.
    pub fn consolidate_now(&mut self) -> Result<DreamReport> {
        block_on(self.consolidate_async())
    }

    /// Return the next time a dream should fire, if auto-dreaming is enabled
    /// and enough new episodes have accumulated.
    pub fn schedule_next(&self) -> Option<Duration> {
        self.schedule()
    }

    /// Resolve the delay for a specific trigger kind.
    #[must_use]
    pub fn trigger_delay(&self, trigger: &DreamTrigger) -> Option<Duration> {
        if !self.config.auto_dream {
            return None;
        }
        let latest_report = self.latest_report().ok().flatten();
        self.controls.schedule.trigger_delay(
            trigger,
            latest_report.as_ref(),
            self.controls.budget.as_ref(),
            Utc::now(),
        )
    }

    fn report_dir(&self) -> PathBuf {
        self.workdir.join(".roko").join("dreams")
    }

    fn heartbeat_snapshot(
        &self,
        episodes: &[Episode],
        latest_report: Option<&DreamReport>,
        now: DateTime<Utc>,
    ) -> DreamHeartbeatReport {
        let processed_through = latest_report.and_then(|report| {
            report
                .processed_through
                .clone()
                .or(Some(report.started_at.clone()))
        });
        let last_dream_at = latest_report.map(|report| report.completed_at.clone());
        self.controls.heartbeat_report(
            episodes,
            processed_through,
            last_dream_at,
            now,
            self.config.min_episodes_for_dream,
        )
    }

    async fn consolidate_async(&mut self) -> Result<DreamReport> {
        let episodes_path = self.workdir.join(".roko").join("episodes.jsonl");
        let episodes = Arc::new(EpisodeLogger::new(episodes_path.clone()));
        let knowledge = Arc::new(KnowledgeStore::for_workdir(&self.workdir));
        let playbooks_root = self.workdir.join(".roko").join("learn").join("playbooks");
        let playbooks = Arc::new(PlaybookStore::new(playbooks_root));
        let dispatcher = build_dream_review_dispatcher_with_model_caller(
            &self.workdir,
            &self.config.agent,
            self.model_caller.clone(),
        )?;

        // ── DREAM-06: Check episode backlog for intensive mode ──────
        let all_episodes = EpisodeLogger::read_all_lossy(&episodes_path)
            .await
            .unwrap_or_default();
        let latest_report = self.latest_report().ok().flatten();
        let unreplayed_count = match &latest_report {
            Some(report) => all_episodes
                .iter()
                .filter(|ep| {
                    report
                        .processed_through
                        .is_none_or(|cutoff| ep.timestamp > cutoff)
                })
                .count(),
            None => all_episodes.len(),
        };

        let intensive_active = self
            .controls
            .intensive
            .check_activation(unreplayed_count, self.controls.replay.max_episodes)
            .is_some();

        if intensive_active {
            self.controls.intensive.active = true;
            // Double max_episodes during intensive mode
            self.controls.replay.max_episodes = self
                .controls
                .replay
                .max_episodes
                .saturating_mul(self.controls.intensive.replay_multiplier as usize);
        }

        let mut cycle = DreamCycle::new(episodes, knowledge, playbooks, dispatcher);
        cycle.configure_threats(
            self.controls.threat_simulation,
            self.controls.threat_severity_floor,
        );
        let mut report = cycle.run_budgeted(&mut self.controls.budget).await?;

        // Mark intensive mode in the report.
        report.intensive_mode_active = intensive_active;

        // Check whether to deactivate intensive mode.
        if intensive_active
            && self
                .controls
                .intensive
                .should_deactivate(unreplayed_count.saturating_sub(report.processed_episodes))
        {
            self.controls.intensive.active = false;
        }

        // ── DREAM-14: Persist dream cycle report as journal entry ────
        self.persist_journal_entry(&report);

        Ok(report)
    }

    /// Append a journal entry for the completed dream cycle (DREAM-14).
    fn persist_journal_entry(&self, report: &DreamReport) {
        let mut journal = DreamJournal::standard(&self.workdir);
        let entry = DreamJournalEntry {
            cycle_id: format!("dream-{}", report.started_at.timestamp_millis()),
            agent_id: self.config.agent.command.clone(),
            cycle_start: report.started_at,
            cycle_end: report.completed_at,
            trigger: DreamTrigger::Manual,
            nrem_duration_secs: {
                let diff = report.completed_at - report.started_at;
                diff.num_seconds().max(0) as u64
            },
            rem_duration_secs: 0,
            consolidation_duration_secs: 0,
            hypotheses_generated: report.strategy_hypotheses.len(),
            hypotheses_staged: report
                .staging_buffer_stats
                .as_ref()
                .map(|s| s.total_entries)
                .unwrap_or(0),
            hypotheses_promoted: report
                .staging_buffer_stats
                .as_ref()
                .map(|s| s.promoted_this_cycle)
                .unwrap_or(0),
            hypotheses_refuted: 0,
            nightmares_detected: 0,
            human_review_required: false,
            hypothesis_diversity: 0.0,
            total_tokens: 0,
            early_termination: report
                .performance_notes
                .iter()
                .any(|n| n.contains("budget exhausted")),
            early_termination_reason: report
                .performance_notes
                .iter()
                .find(|n| n.contains("budget exhausted"))
                .cloned(),
        };
        let _ = journal.append(&entry);
    }
}

impl Default for DreamRunner {
    fn default() -> Self {
        Self::new(
            PathBuf::from("."),
            DreamLoopConfig {
                auto_dream: true,
                idle_threshold_mins: 15,
                min_episodes_for_dream: 5,
                agent: DreamAgentConfig {
                    command: "cat".to_string(),
                    args: Vec::new(),
                    model: None,
                    bare_mode: true,
                    effort: "medium".to_string(),
                    fallback_model: None,
                    timeout_ms: 120_000,
                    env: Vec::new(),
                },
            },
        )
    }
}

/// Trait for dream engines that can replay, consolidate, and schedule.
pub trait DreamEngine {
    /// Replay a batch of episodes into insights.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`DreamRunner::replay_insights`].
    fn replay(&mut self, episodes: &[Episode]) -> Result<Vec<Insight>>;
    /// Consolidate the workspace into a persisted dream report.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`DreamRunner::consolidate_now`].
    fn consolidate(&mut self) -> Result<DreamReport>;
    /// Schedule the next dream fire time.
    fn schedule(&self) -> Option<Duration>;
}

impl DreamEngine for DreamRunner {
    fn replay(&mut self, episodes: &[Episode]) -> Result<Vec<Insight>> {
        self.replay_insights(episodes)
    }

    fn consolidate(&mut self) -> Result<DreamReport> {
        self.consolidate_now()
    }

    fn schedule(&self) -> Option<Duration> {
        if !self.config.auto_dream || !self.controls.schedule.enabled {
            return None;
        }
        if self
            .controls
            .budget
            .is_some_and(|budget| budget.exhausted())
        {
            return None;
        }

        let episodes_path = self.workdir.join(".roko").join("episodes.jsonl");
        let episodes = block_on(EpisodeLogger::read_all_lossy(&episodes_path)).ok()?;
        let last_report = load_latest_dream_report(&self.report_dir()).ok().flatten();
        let now = Utc::now();
        let heartbeat = self.heartbeat_snapshot(&episodes, last_report.as_ref(), now);

        if !heartbeat.enough_recent_episodes {
            return None;
        }

        let latest_episode = heartbeat.latest_episode_at?;
        let idle_delay = self.controls.schedule.trigger_delay(
            &DreamTrigger::Idle,
            last_report.as_ref(),
            self.controls.budget.as_ref(),
            now,
        )?;
        let idle_fire_at = latest_episode + chrono::Duration::from_std(idle_delay).ok()?;
        let mut target = idle_fire_at;
        if let Some(cron_delay) = self.controls.schedule.trigger_delay(
            &DreamTrigger::Scheduled,
            last_report.as_ref(),
            self.controls.budget.as_ref(),
            now,
        ) {
            let cron_fire_at = now + chrono::Duration::from_std(cron_delay).ok()?;
            target = target.min(cron_fire_at);
        }
        if let Some(report) = last_report.as_ref() {
            if dream_quality_score(report) > 1.5 {
                let adjusted = self
                    .controls
                    .schedule
                    .idle_delay(Some(report), self.controls.budget.as_ref());
                target = latest_episode + chrono::Duration::from_std(adjusted).ok()?;
            }
        }
        if let Some(delta_delay) = heartbeat.delta_due_in_secs {
            let delta_fire_at = now + chrono::Duration::seconds(delta_delay as i64);
            target = target.min(delta_fire_at);
        }
        if target <= now {
            Some(Duration::ZERO)
        } else {
            (target - now).to_std().ok()
        }
    }
}

fn dream_quality_score(report: &DreamReport) -> f64 {
    let cluster_count = report.clusters.len().max(1) as f64;
    let knowledge = report.knowledge_entries_written as f64;
    let playbooks = report.playbooks_created as f64 * 0.5;
    let hypotheses = report.strategy_hypotheses.len() as f64 * 0.25;
    let regressions = report.regressions_detected.len() as f64 * 0.35;
    ((knowledge + playbooks + hypotheses) / cluster_count - regressions).clamp(0.0, 5.0)
}

fn remaining_fraction(current: u64, maximum: u64) -> f64 {
    if maximum == 0 {
        return 0.0;
    }
    let used = current.min(maximum) as f64 / maximum as f64;
    (1.0 - used).clamp(0.0, 1.0)
}

fn remaining_fraction_f64(current: f64, maximum: f64) -> f64 {
    if maximum <= 0.0 {
        return 0.0;
    }
    let used = (current / maximum).clamp(0.0, 1.0);
    (1.0 - used).clamp(0.0, 1.0)
}

fn default_true() -> bool {
    true
}

fn default_idle_threshold_mins() -> u64 {
    15
}

fn default_quality_gain() -> f64 {
    0.75
}

fn default_quality_penalty() -> f64 {
    1.25
}

fn default_threat_floor() -> f64 {
    0.20
}

fn default_heartbeat_tick_secs() -> u64 {
    75
}

fn default_delta_interval_mins() -> u64 {
    60
}

fn default_idle_grace_mins() -> u64 {
    15
}

fn default_intensive_replay_multiplier() -> u32 {
    3
}

fn default_intensive_counterfactual_depth() -> usize {
    8
}

fn default_intensive_rehearsal_limit() -> usize {
    50
}

fn default_backlog_high_water() -> usize {
    50
}

fn default_backlog_low_water() -> usize {
    20
}

/// Load the latest persisted dream report from a report directory.
///
/// Returns `Ok(None)` when the directory has no report yet.
///
/// # Errors
///
/// Returns an error if the report directory cannot be scanned, if the newest
/// report cannot be read, or if the report contents are invalid JSON.
pub fn load_latest_dream_report(report_dir: &Path) -> Result<Option<DreamReport>> {
    let Some(path) = latest_dream_report_path(report_dir)? else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("read dream report {}", path.display()))?;
    let report: DreamReport = serde_json::from_str(&text)
        .with_context(|| format!("parse dream report {}", path.display()))?;
    Ok(Some(report))
}

fn latest_dream_report_path(report_dir: &Path) -> Result<Option<PathBuf>> {
    let Ok(entries) = std::fs::read_dir(report_dir) else {
        return Ok(None);
    };

    let mut latest: Option<(i64, PathBuf)> = None;
    for entry in entries {
        let entry = entry.with_context(|| format!("scan {}", report_dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Some(ts) = stem.strip_prefix("dream-") else {
            continue;
        };
        let Ok(ts_ms) = ts.parse::<i64>() else {
            continue;
        };
        let should_replace = latest.as_ref().is_none_or(|(current, _)| ts_ms > *current);
        if should_replace {
            latest = Some((ts_ms, path));
        }
    }

    Ok(latest.map(|(_, path)| path))
}

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
    } else {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("create dream runtime");
        runtime.block_on(future)
    }
}

/// Build the dream review dispatcher from the configured agent backend.
///
/// # Errors
///
/// Returns an error if the routing config cannot be loaded, if a routed model
/// cannot be determined, or if the agent backend cannot be constructed.
pub fn build_dream_review_dispatcher(
    workdir: &Path,
    config: &DreamAgentConfig,
) -> Result<Arc<dyn AgentDispatcher>> {
    build_dream_review_dispatcher_with_model_caller(workdir, config, None)
}

/// Build the dream review dispatcher, preferring gateway-backed inference when supplied.
///
/// # Errors
///
/// Returns an error if the routing config cannot be loaded, if a routed model
/// cannot be determined, or if the agent backend cannot be constructed.
pub fn build_dream_review_dispatcher_with_model_caller(
    workdir: &Path,
    config: &DreamAgentConfig,
    model_caller: Option<Arc<dyn ModelCaller>>,
) -> Result<Arc<dyn AgentDispatcher>> {
    let agent = match model_caller {
        Some(model_caller) => config.build_agent_via_gateway(workdir, model_caller)?,
        None => config.build_agent(workdir)?,
    };
    Ok(Arc::new(agent))
}

/// Thin wrapper around the configured review agent used by dream consolidation.
enum DreamReviewAgent {
    Agent {
        inner: Box<dyn Agent>,
    },
    Gateway {
        model: String,
        model_caller: Arc<dyn ModelCaller>,
        name: String,
    },
}

#[async_trait]
impl Agent for DreamReviewAgent {
    async fn run(&self, input: &Engram, ctx: &RokoContext) -> AgentResult {
        match self {
            Self::Agent { inner } => inner.run(input, ctx).await,
            Self::Gateway {
                model,
                model_caller,
                name,
            } => {
                let prompt = match input.body.as_text() {
                    Ok(prompt) => prompt.to_string(),
                    Err(error) => {
                        let output = input
                            .derive(
                                Kind::AgentOutput,
                                Body::text(format!("dream review input was not text: {error}")),
                            )
                            .provenance(Provenance::agent(name))
                            .build();
                        return AgentResult::fail(output);
                    }
                };

                let request = ModelCallRequest {
                    model: model.clone(),
                    system: None,
                    messages: vec![ChatMessage {
                        role: MessageRole::User,
                        content: prompt,
                    }],
                    max_tokens: None,
                    temperature: None,
                    role: Some("dream-review".to_string()),
                    caller: Some(CallerIdentity::Dreams.into()),
                    run_id: None,
                    prompt_section_ids: Vec::new(),
                    knowledge_ids: Vec::new(),
                    budget: None,
                    budget_remaining: None,
                    routing_hints: Vec::new(),
                    cache_policy: CachePolicy::Default,
                };

                match model_caller.call(request).await {
                    Ok(response) => {
                        let output = input
                            .derive(Kind::AgentOutput, Body::text(response.content))
                            .provenance(Provenance::agent(name))
                            .build();
                        let usage = roko_agent::Usage {
                            input_tokens: response.usage.input_tokens.min(u32::MAX as u64) as u32,
                            output_tokens: response.usage.output_tokens.min(u32::MAX as u64) as u32,
                            cache_read_tokens: 0,
                            cache_create_tokens: 0,
                            cost_usd: response.usage.cost_usd as f32,
                            wall_ms: 0,
                        };
                        AgentResult::ok(output).with_usage(usage)
                    }
                    Err(error) => {
                        let output = input
                            .derive(Kind::AgentOutput, Body::text(error.to_string()))
                            .provenance(Provenance::agent(name))
                            .build();
                        AgentResult::fail(output)
                    }
                }
            }
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Agent { inner } => inner.name(),
            Self::Gateway { name, .. } => name,
        }
    }

    fn supports_streaming(&self) -> bool {
        match self {
            Self::Agent { inner } => inner.supports_streaming(),
            Self::Gateway { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_learn::episode_logger::Usage;
    use tempfile::TempDir;

    fn test_loop_config() -> DreamLoopConfig {
        DreamLoopConfig {
            auto_dream: true,
            idle_threshold_mins: 15,
            min_episodes_for_dream: 1,
            agent: DreamAgentConfig {
                command: "cat".to_string(),
                args: Vec::new(),
                model: None,
                bare_mode: true,
                effort: "medium".to_string(),
                fallback_model: None,
                timeout_ms: 120_000,
                env: Vec::new(),
            },
        }
    }

    fn episode_at(agent: &str, task: &str, timestamp: DateTime<Utc>) -> Episode {
        let mut episode = Episode::new(agent, task);
        episode.timestamp = timestamp.clone();
        episode.completed_at = timestamp.clone();
        episode.started_at = timestamp;
        episode
    }

    #[test]
    fn budget_tracks_consumption() {
        let mut budget = DreamBudget {
            max_tokens: 100,
            max_cost_usd: 10.0,
            max_duration_secs: 100,
            consumed_tokens: 0,
            consumed_cost_usd: 0.0,
            consumed_duration_secs: 0,
        };
        let mut episode = Episode::new("agent", "task-1");
        episode.tokens_used = 40;
        episode.usage = Usage::tokens(20, 10);
        episode.usage.cost_usd = 1.5;
        episode.duration_secs = 2.0;

        budget.consume_episode(&episode);

        assert!(budget.remaining_fraction() < 1.0);
        assert!(!budget.exhausted());
    }

    #[test]
    fn schedule_manual_trigger_is_allowed() {
        let policy = DreamSchedulePolicy::default();
        let delay = policy.trigger_delay(&DreamTrigger::Manual, None, None, Utc::now());
        assert_eq!(delay, Some(Duration::ZERO));
    }

    // INT-19: Coordination pattern trigger tests.

    #[test]
    fn coordination_pattern_trigger_label() {
        let trigger = DreamTrigger::CoordinationPattern {
            pattern_name: "resource_exhaustion".to_string(),
            contributing_watchers: vec!["cost-overrun".to_string(), "time-overrun".to_string()],
        };
        assert_eq!(trigger.label(), "coordination_pattern");
    }

    #[test]
    fn coordination_pattern_trigger_is_allowed() {
        let policy = DreamSchedulePolicy::default();
        let trigger = DreamTrigger::CoordinationPattern {
            pattern_name: "quality_degradation".to_string(),
            contributing_watchers: vec!["compile-fail-repeat".to_string()],
        };
        assert!(policy.allows(&trigger));
    }

    #[test]
    fn coordination_pattern_trigger_immediate_delay() {
        let policy = DreamSchedulePolicy::default();
        let trigger = DreamTrigger::CoordinationPattern {
            pattern_name: "progress_stall".to_string(),
            contributing_watchers: vec![],
        };
        let delay = policy.trigger_delay(&trigger, None, None, Utc::now());
        assert_eq!(delay, Some(Duration::ZERO));
    }

    #[test]
    fn schedule_scheduled_trigger_uses_cron_expression() {
        let policy = DreamSchedulePolicy {
            scheduled_cron: Some("0 * * * * * *".to_string()),
            ..DreamSchedulePolicy::default()
        };
        let now = DateTime::parse_from_rfc3339("2026-04-18T12:34:15Z")
            .expect("timestamp")
            .with_timezone(&Utc);

        let delay = policy.trigger_delay(&DreamTrigger::Scheduled, None, None, now);

        assert_eq!(delay, Some(Duration::from_secs(45)));
    }

    #[test]
    fn heartbeat_report_pauses_delta_during_recent_activity() {
        let mut controls = DreamRuntimeControls::default();
        controls.heartbeat = DreamHeartbeatPolicy {
            tick_interval_secs: 30,
            delta_interval_mins: 60,
            idle_grace_mins: 15,
        };
        let now = Utc::now();
        let episodes = vec![
            episode_at("agent", "task-1", now - chrono::Duration::minutes(6)),
            episode_at("agent", "task-1", now - chrono::Duration::minutes(5)),
        ];

        let report = controls.heartbeat_report(
            &episodes,
            Some(now - chrono::Duration::hours(2)),
            Some(now - chrono::Duration::hours(3)),
            now,
            1,
        );

        assert_eq!(report.recent_episode_count, 2);
        assert!(report.paused_for_active_plan);
        assert_eq!(report.delta_due_in_secs, None);
        assert!(!report.delta_ready);
        assert_eq!(report.heartbeat_due_in_secs, 30);
    }

    #[test]
    fn heartbeat_report_becomes_ready_after_interval() {
        let mut controls = DreamRuntimeControls::default();
        controls.heartbeat = DreamHeartbeatPolicy {
            tick_interval_secs: 75,
            delta_interval_mins: 60,
            idle_grace_mins: 15,
        };
        let now = Utc::now();
        let episodes = vec![episode_at(
            "agent",
            "task-1",
            now - chrono::Duration::hours(3),
        )];

        let report = controls.heartbeat_report(
            &episodes,
            Some(now - chrono::Duration::hours(4)),
            Some(now - chrono::Duration::hours(2)),
            now,
            1,
        );

        assert_eq!(report.recent_episode_count, 1);
        assert!(!report.paused_for_active_plan);
        assert_eq!(report.delta_due_in_secs, Some(0));
        assert!(report.delta_ready);
    }

    #[test]
    fn runner_heartbeat_report_handles_empty_workspace() {
        let tmp = TempDir::new().unwrap_or_else(|err| panic!("tempdir: {err}"));
        let runner = DreamRunner::with_controls(
            tmp.path(),
            test_loop_config(),
            DreamRuntimeControls::default(),
        );

        let report = runner
            .heartbeat_report()
            .unwrap_or_else(|err| panic!("heartbeat report: {err}"));

        assert_eq!(report.recent_episode_count, 0);
        assert!(!report.enough_recent_episodes);
        assert_eq!(report.delta_due_in_secs, None);
    }

    // ── DREAM-06 intensive mode tests ───────────────────────────────────

    #[test]
    fn intensive_mode_activates_at_high_water() {
        let mode = IntensiveMode::default();
        assert_eq!(mode.backlog_high_water, 50);
        assert_eq!(mode.backlog_low_water, 20);

        // Below high water: no activation.
        assert!(mode.check_activation(30, 24).is_none());

        // At high water: activates with doubled max.
        let adjusted = mode.check_activation(50, 24);
        assert!(adjusted.is_some());
        assert_eq!(adjusted.unwrap(), 24 * 3); // default multiplier is 3
    }

    #[test]
    fn intensive_mode_deactivates_at_low_water() {
        let mode = IntensiveMode::default();
        assert!(mode.should_deactivate(15));
        assert!(!mode.should_deactivate(25));
    }

    #[test]
    fn intensive_mode_serialization_roundtrip() {
        let mode = IntensiveMode {
            active: true,
            replay_multiplier: 4,
            counterfactual_depth: 10,
            rehearsal_limit: 100,
            backlog_high_water: 60,
            backlog_low_water: 25,
        };
        let json = serde_json::to_string(&mode).unwrap();
        let deserialized: IntensiveMode = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.backlog_high_water, 60);
        assert_eq!(deserialized.backlog_low_water, 25);
        assert!(deserialized.active);
    }

    // ── DREAM-14 journal persistence tests ──────────────────────────────

    #[test]
    fn journal_entry_written_to_standard_path() {
        let tmp = TempDir::new().unwrap();
        let mut journal = DreamJournal::standard(tmp.path());
        let entry = DreamJournalEntry {
            cycle_id: "test-cycle".to_string(),
            agent_id: "cat".to_string(),
            cycle_start: Utc::now(),
            cycle_end: Utc::now(),
            trigger: DreamTrigger::Manual,
            nrem_duration_secs: 10,
            rem_duration_secs: 5,
            consolidation_duration_secs: 2,
            hypotheses_generated: 3,
            hypotheses_staged: 2,
            hypotheses_promoted: 1,
            hypotheses_refuted: 0,
            nightmares_detected: 0,
            human_review_required: false,
            hypothesis_diversity: 0.5,
            total_tokens: 500,
            early_termination: false,
            early_termination_reason: None,
        };
        journal.append(&entry).unwrap();
        let entries = journal.read_all().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].cycle_id, "test-cycle");
    }

    #[test]
    fn bus_pulse_trigger_config_dream_worthiness() {
        let config = BusPulseTriggerConfig::default();
        assert!(config.is_dream_worthy(0.8, "gate_verdict"));
        assert!(!config.is_dream_worthy(0.5, "gate_verdict")); // below min_score
        assert!(!config.is_dream_worthy(0.8, "unknown_kind"));
    }
}
