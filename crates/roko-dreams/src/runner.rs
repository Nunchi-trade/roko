//! Public dream runner facade.
//!
//! The runner wraps the existing dream-cycle implementation with a small API
//! that can be called from the CLI and from other workspace crates without
//! reimplementing the consolidation logic.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use roko_agent::{Agent, AgentResult, ClaudeCliAgent, ExecAgent};
use roko_core::{Context as RokoContext, Engram};
use roko_learn::{episode_logger::EpisodeLogger, playbook::PlaybookStore};
use roko_neuro::{
    KnowledgeStore,
    tier_progression::{InsightRecord, TierProgression as NeuroTierProgression},
};
use serde::{Deserialize, Serialize};

use crate::cycle::{AgentDispatcher, DreamCycle, DreamCycleReport};

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
    fn build_agent(&self, workdir: &Path) -> DreamReviewAgent {
        if self.command == "claude" {
            let model = self
                .model
                .clone()
                .unwrap_or_else(|| "claude-opus-4-6".to_string());
            let mut agent = ClaudeCliAgent::new(&self.command, workdir, model)
                .with_timeout_ms(self.timeout_ms)
                .with_bare_mode(self.bare_mode)
                .with_effort(self.effort.clone())
                .with_extra_args(self.args.clone());
            if let Some(fallback_model) = &self.fallback_model {
                agent = agent.with_fallback_model(fallback_model.clone());
            }
            for (key, value) in &self.env {
                agent = agent.with_env_var(key, value);
            }
            DreamReviewAgent::Claude(agent)
        } else {
            let mut agent =
                ExecAgent::new(&self.command, self.args.clone()).with_timeout_ms(self.timeout_ms);
            for (key, value) in &self.env {
                agent = agent.with_env_var(key, value);
            }
            DreamReviewAgent::Exec(agent)
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

/// Public facade for dream replay, consolidation, and scheduling.
#[derive(Debug, Clone)]
pub struct DreamRunner {
    workdir: PathBuf,
    config: DreamLoopConfig,
}

impl DreamRunner {
    /// Construct a dream runner for a working directory.
    #[must_use]
    pub fn new(workdir: impl Into<PathBuf>, config: DreamLoopConfig) -> Self {
        Self {
            workdir: workdir.into(),
            config,
        }
    }

    /// Replay the supplied episodes into durable insights without mutating the
    /// workspace.
    #[allow(clippy::needless_pass_by_value)]
    #[must_use]
    pub fn replay_insights(&self, episodes: &[Episode]) -> Result<Vec<Insight>> {
        let progression = NeuroTierProgression::default();
        let report = progression.analyze(episodes);
        Ok(report.insights)
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
        let dispatcher: Arc<dyn AgentDispatcher> =
            Arc::new(self.config.agent.build_agent(&self.workdir));
        let mut cycle = DreamCycle::new(episodes, knowledge, playbooks, dispatcher);
        cycle.run().await
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
        if !self.config.auto_dream {
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
        let idle_threshold =
            Duration::from_secs(self.config.idle_threshold_mins.saturating_mul(60));
        let target_fire_at = latest_episode + chrono::Duration::from_std(idle_threshold).ok()?;
        let now = Utc::now();
        if target_fire_at <= now {
            Some(Duration::ZERO)
        } else {
            (target_fire_at - now).to_std().ok()
        }
    }
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

enum DreamReviewAgent {
    Claude(ClaudeCliAgent),
    Exec(ExecAgent),
}

#[async_trait]
impl Agent for DreamReviewAgent {
    async fn run(&self, input: &Engram, ctx: &RokoContext) -> AgentResult {
        match self {
            Self::Claude(agent) => agent.run(input, ctx).await,
            Self::Exec(agent) => agent.run(input, ctx).await,
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Claude(agent) => agent.name(),
            Self::Exec(agent) => agent.name(),
        }
    }

    fn supports_streaming(&self) -> bool {
        match self {
            Self::Claude(agent) => agent.supports_streaming(),
            Self::Exec(agent) => agent.supports_streaming(),
        }
    }
}
