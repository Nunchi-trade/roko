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
use roko_core::{Context as RokoContext, Engram};
use roko_learn::{episode_logger::EpisodeLogger, playbook::PlaybookStore};
use roko_neuro::{
    KnowledgeStore,
    tier_progression::{InsightRecord, TierProgression as NeuroTierProgression},
};
use serde::{Deserialize, Serialize};

use crate::cycle::{AgentDispatcher, DreamCycle, DreamCycleReport};
use crate::imagination::ImaginationMode;
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
            Ok(DreamReviewAgent { inner: agent })
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
            Ok(DreamReviewAgent { inner: agent })
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DreamTrigger {
    /// Idle gap between task dispatches.
    Idle,
    /// Cron-like schedule.
    Scheduled,
    /// Manual command invocation.
    Manual,
}

impl DreamTrigger {
    /// Stable lowercase label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Scheduled => "scheduled",
            Self::Manual => "manual",
        }
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
    pub fn allows(&self, trigger: DreamTrigger) -> bool {
        match trigger {
            DreamTrigger::Idle | DreamTrigger::Scheduled => self.enabled,
            DreamTrigger::Manual => self.manual_enabled,
        }
    }

    /// Resolve the next delay for a specific trigger type.
    #[must_use]
    pub fn trigger_delay(
        &self,
        trigger: DreamTrigger,
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
        }
    }
}

/// Combined runtime controls for replay, budgeting, and scheduling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamRuntimeControls {
    /// Replay planner configuration.
    #[serde(default)]
    pub replay: DreamReplayPolicy,
    /// Optional sleep-time compute budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<DreamBudget>,
    /// Scheduling policy.
    #[serde(default)]
    pub schedule: DreamSchedulePolicy,
    /// Creativity mode used for REM imagination.
    #[serde(default)]
    pub imagination_mode: ImaginationMode,
    /// Whether to emit threat warnings from repeated failures.
    #[serde(default = "default_true")]
    pub threat_simulation: bool,
    /// Minimum severity required before a threat becomes a warning entry.
    #[serde(default = "default_threat_floor")]
    pub threat_severity_floor: f64,
}

impl Default for DreamRuntimeControls {
    fn default() -> Self {
        Self {
            replay: DreamReplayPolicy::default(),
            budget: None,
            schedule: DreamSchedulePolicy::default(),
            imagination_mode: ImaginationMode::default(),
            threat_simulation: true,
            threat_severity_floor: 0.20,
        }
    }
}

/// Public facade for dream replay, consolidation, and scheduling.
#[derive(Debug, Clone)]
pub struct DreamRunner {
    workdir: PathBuf,
    config: DreamLoopConfig,
    controls: DreamRuntimeControls,
}

impl DreamRunner {
    /// Construct a dream runner for a working directory.
    #[must_use]
    pub fn new(workdir: impl Into<PathBuf>, config: DreamLoopConfig) -> Self {
        Self {
            workdir: workdir.into(),
            config,
            controls: DreamRuntimeControls::default(),
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
        }
    }

    /// Replay the supplied episodes into durable insights without mutating the
    /// workspace.
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
    pub fn latest_report(&self) -> Result<Option<DreamReport>> {
        load_latest_dream_report(&self.report_dir())
    }

    /// Run a full dream consolidation cycle against the workspace.
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
    pub fn trigger_delay(&self, trigger: DreamTrigger) -> Option<Duration> {
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

    async fn consolidate_async(&mut self) -> Result<DreamReport> {
        let episodes = Arc::new(EpisodeLogger::new(
            self.workdir.join(".roko").join("episodes.jsonl"),
        ));
        let knowledge = Arc::new(KnowledgeStore::for_workdir(&self.workdir));
        let playbooks_root = self.workdir.join(".roko").join("learn").join("playbooks");
        let playbooks = Arc::new(PlaybookStore::new(playbooks_root));
        let dispatcher = build_dream_review_dispatcher(&self.workdir, &self.config.agent)?;
        let mut cycle = DreamCycle::new(episodes, knowledge, playbooks, dispatcher);
        cycle.run_budgeted(&mut self.controls.budget).await
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
    fn replay(&mut self, episodes: &[Episode]) -> Result<Vec<Insight>>;
    /// Consolidate the workspace into a persisted dream report.
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
        let cutoff = last_report
            .as_ref()
            .and_then(|report| report.processed_through.or(Some(report.started_at)));

        let recent: Vec<&Episode> = episodes
            .iter()
            .filter(|episode| cutoff.is_none_or(|ts| episode.timestamp > ts))
            .collect();

        if recent.len() < self.config.min_episodes_for_dream {
            return None;
        }

        let latest_episode = recent.iter().map(|episode| episode.timestamp).max()?;
        let now = Utc::now();
        let idle_delay = self.controls.schedule.trigger_delay(
            DreamTrigger::Idle,
            last_report.as_ref(),
            self.controls.budget.as_ref(),
            now,
        )?;
        let idle_fire_at = latest_episode + chrono::Duration::from_std(idle_delay).ok()?;
        let mut target = idle_fire_at;
        if let Some(cron_delay) = self.controls.schedule.trigger_delay(
            DreamTrigger::Scheduled,
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

/// Load the latest persisted dream report from a report directory.
///
/// Returns `Ok(None)` when the directory has no report yet.
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
pub fn build_dream_review_dispatcher(
    workdir: &Path,
    config: &DreamAgentConfig,
) -> Result<Arc<dyn AgentDispatcher>> {
    Ok(Arc::new(config.build_agent(workdir)?))
}

/// Thin wrapper around the configured review agent used by dream consolidation.
struct DreamReviewAgent {
    inner: Box<dyn Agent>,
}

#[async_trait]
impl Agent for DreamReviewAgent {
    async fn run(&self, input: &Engram, ctx: &RokoContext) -> AgentResult {
        self.inner.run(input, ctx).await
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn supports_streaming(&self) -> bool {
        self.inner.supports_streaming()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_learn::episode_logger::Usage;

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
        let delay = policy.trigger_delay(DreamTrigger::Manual, None, None, Utc::now());
        assert_eq!(delay, Some(Duration::ZERO));
    }
}
