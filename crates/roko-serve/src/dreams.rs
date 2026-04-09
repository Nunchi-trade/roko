//! Background dream-cycle bootstrap for daemon mode.
//!
//! The loop watches for idle periods in daemon mode, then runs the existing
//! `roko-dreams` batch processor when enough new episodes have accumulated.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use roko_agent::{Agent, AgentResult, ClaudeCliAgent, ExecAgent};
use roko_core::{Context as RokoContext, Signal};
use roko_dreams::DreamCycle;
use roko_dreams::cycle::{DreamCycleReport, DreamOutcome};
use roko_learn::{episode_logger::EpisodeLogger, playbook::PlaybookStore};
use roko_neuro::KnowledgeStore;
use tokio::task::JoinHandle;
use tokio::time::{Instant as TokioInstant, interval_at};
use tracing::{info, warn};

use crate::state::AppState;

const DREAM_CHECK_INTERVAL: Duration = Duration::from_secs(60);

/// Configuration for the dream review agent.
#[derive(Clone, Debug)]
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
    /// Build a dream review agent around the configured CLI backend.
    #[must_use]
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

/// Combined scheduler + agent configuration for the dream loop.
#[derive(Clone, Debug)]
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

/// Start the dream cycle in the background.
#[must_use]
pub fn start_dream_loop(state: Arc<AppState>, config: DreamLoopConfig) -> JoinHandle<()> {
    tokio::spawn(async move {
        if !config.auto_dream {
            return;
        }

        let mut cycle = match build_dream_cycle(&state, &config).await {
            Ok(cycle) => cycle,
            Err(err) => {
                warn!(error = %err, "dream cycle bootstrap failed");
                return;
            }
        };

        if let Err(err) = restore_last_dream_at(&state, &mut cycle) {
            warn!(error = %err, "failed to restore last dream checkpoint");
        }

        let idle_threshold = Duration::from_secs(config.idle_threshold_mins.saturating_mul(60));
        let mut idle_since: Option<TokioInstant> = None;
        let mut interval = interval_at(
            TokioInstant::now() + DREAM_CHECK_INTERVAL,
            DREAM_CHECK_INTERVAL,
        );

        loop {
            interval.tick().await;

            if state.cancel.is_cancelled() {
                break;
            }

            let active_agents = state.supervisor.count().await;
            if active_agents > 0 {
                idle_since = None;
                continue;
            }

            let now = TokioInstant::now();
            let started_idle = idle_since.get_or_insert(now);
            if now.duration_since(*started_idle) < idle_threshold {
                continue;
            }

            if let Err(err) =
                maybe_run_dream_cycle(&state, &mut cycle, config.min_episodes_for_dream).await
            {
                warn!(error = %err, "dream cycle failed");
            }
        }
    })
}

/// Run one dream cycle immediately using the existing stores and agent config.
///
/// This mirrors the daemon bootstrap path, but executes the batch once instead
/// of waiting for the idle scheduler.
pub async fn run_dream_cycle_now(
    state: Arc<AppState>,
    config: DreamLoopConfig,
) -> Result<DreamCycleReport> {
    let mut cycle = build_dream_cycle(&state, &config).await?;
    restore_last_dream_at(&state, &mut cycle)?;
    let report = cycle.run().await.context("run dream cycle")?;
    apply_dream_affect_feedback(&state, &report);
    Ok(report)
}

/// Load the latest persisted dream report from the report directory.
///
/// Returns `Ok(None)` when no report exists yet.
pub fn load_latest_dream_report(report_dir: &Path) -> Result<Option<DreamCycleReport>> {
    let Some(path) = latest_dream_report_path(report_dir)? else {
        return Ok(None);
    };
    let text = fs::read_to_string(&path)
        .with_context(|| format!("read dream report {}", path.display()))?;
    let report: DreamCycleReport = serde_json::from_str(&text)
        .with_context(|| format!("parse dream report {}", path.display()))?;
    Ok(Some(report))
}

async fn build_dream_cycle(state: &AppState, config: &DreamLoopConfig) -> Result<DreamCycle> {
    let episodes = Arc::new(EpisodeLogger::new(state.layout.episodes_path()));
    let knowledge = Arc::new(KnowledgeStore::for_layout(&state.layout));
    let playbooks_root = state.layout.root().join("learn").join("playbooks");
    let playbooks = Arc::new(PlaybookStore::new(playbooks_root));
    let dispatcher = Arc::new(config.agent.build_agent(&state.workdir));
    Ok(DreamCycle::new(episodes, knowledge, playbooks, dispatcher))
}

fn restore_last_dream_at(state: &AppState, cycle: &mut DreamCycle) -> Result<()> {
    let report_dir = state.layout.root().join("dreams");
    let Some(path) = latest_dream_report_path(&report_dir)? else {
        return Ok(());
    };
    let text = fs::read_to_string(&path)
        .with_context(|| format!("read dream report {}", path.display()))?;
    let report: DreamCycleReport = serde_json::from_str(&text)
        .with_context(|| format!("parse dream report {}", path.display()))?;
    cycle.set_last_dream_at(report.processed_through.or(Some(report.started_at)));
    Ok(())
}

fn latest_dream_report_path(report_dir: &Path) -> Result<Option<PathBuf>> {
    let Ok(entries) = fs::read_dir(report_dir) else {
        return Ok(None);
    };

    let mut latest: Option<(DateTime<Utc>, PathBuf)> = None;
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
        let Some(dt) = Utc.timestamp_millis_opt(ts_ms).single() else {
            continue;
        };

        let should_replace = latest.as_ref().is_none_or(|(current, _)| dt > *current);
        if should_replace {
            latest = Some((dt, path));
        }
    }

    Ok(latest.map(|(_, path)| path))
}

async fn maybe_run_dream_cycle(
    state: &AppState,
    cycle: &mut DreamCycle,
    min_episodes_for_dream: usize,
) -> Result<()> {
    let episodes_path = state.layout.episodes_path();
    let episodes = EpisodeLogger::read_all_lossy(&episodes_path)
        .await
        .with_context(|| format!("load episodes from {}", episodes_path.display()))?;
    let last_dream_at = cycle.last_dream_at();
    let new_episode_count = episodes
        .iter()
        .filter(|episode| {
            last_dream_at
                .map(|cutoff| episode.timestamp > cutoff)
                .unwrap_or(true)
        })
        .count();

    if new_episode_count < min_episodes_for_dream {
        return Ok(());
    }

    info!(
        new_episodes = new_episode_count,
        min_episodes_for_dream, "running dream cycle"
    );
    let report = cycle.run().await.context("run dream cycle")?;
    info!(
        processed_episodes = report.processed_episodes,
        knowledge_entries_written = report.knowledge_entries_written,
        playbooks_created = report.playbooks_created,
        "dream cycle completed"
    );
    apply_dream_affect_feedback(state, &report);
    Ok(())
}

enum DreamReviewAgent {
    Claude(ClaudeCliAgent),
    Exec(ExecAgent),
}

fn apply_dream_affect_feedback(state: &AppState, report: &DreamCycleReport) {
    let mut engine = state.affect_engine.lock();
    apply_dream_affect_feedback_to_engine(&mut engine, report);
}

fn apply_dream_affect_feedback_to_engine(
    engine: &mut roko_golem::AffectEngine,
    report: &DreamCycleReport,
) {
    let mut failing_task_types: BTreeMap<String, usize> = BTreeMap::new();
    for cluster in &report.clusters {
        if cluster.key.outcome != DreamOutcome::Failure || cluster.failure_count <= 2 {
            continue;
        }
        *failing_task_types
            .entry(cluster.key.task_type.clone())
            .or_insert(0) += cluster.failure_count;
    }

    for (task_type, failure_count) in failing_task_types {
        let _ = engine.on_dream_failure(task_type, failure_count);
    }
}

#[async_trait]
impl Agent for DreamReviewAgent {
    async fn run(&self, input: &Signal, ctx: &RokoContext) -> AgentResult {
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

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Duration as ChronoDuration;
    use roko_dreams::cycle::{DreamClusterKey, DreamClusterReport, DreamOutcome};
    use roko_golem::AffectEngine;

    #[test]
    fn dream_failures_reduce_confidence_by_task_type() {
        let mut engine = AffectEngine::new();
        let report = DreamCycleReport {
            started_at: Utc::now() - ChronoDuration::minutes(10),
            completed_at: Utc::now(),
            total_episodes: 3,
            processed_episodes: 3,
            processed_through: None,
            analysis: roko_neuro::tier_progression::TierProgression::default().analyze(&[]),
            clusters: vec![DreamClusterReport {
                key: DreamClusterKey {
                    plan_id: "plan-a".to_string(),
                    task_type: "implementation".to_string(),
                    outcome: DreamOutcome::Failure,
                    model: "claude-haiku-4-5".to_string(),
                },
                episode_count: 3,
                success_count: 0,
                failure_count: 3,
                first_seen_at: Utc::now() - ChronoDuration::minutes(20),
                last_seen_at: Utc::now() - ChronoDuration::minutes(5),
                episode_ids: vec!["ep-1".to_string(), "ep-2".to_string(), "ep-3".to_string()],
                knowledge_entries: Vec::new(),
                playbook: None,
                regression_entries: Vec::new(),
                agent_review: None,
                warnings: Vec::new(),
            }],
            knowledge_entries_written: 0,
            playbooks_created: 0,
            regressions_detected: Vec::new(),
            strategy_hypotheses: Vec::new(),
        };

        apply_dream_affect_feedback_to_engine(&mut engine, &report);

        let state = engine.get_state("implementation");
        assert!(state.confidence < 0.5);
        assert!(state.confidence > 0.25);
    }
}
