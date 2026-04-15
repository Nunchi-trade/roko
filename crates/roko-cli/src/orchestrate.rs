//! Plan-driven orchestration loop: reads plans → builds executor → dispatches
//! agents → runs gates → persists results → advances phases.
//!
//! This is the runtime harness that connects the CLI to the orchestrator's
//! pure state machine. The orchestrator's [`ParallelExecutor`] never does I/O
//! — it returns [`ExecutorAction`]s. This module dispatches those actions to
//! real agents, gates, and git, then feeds results back as events.

use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow};
use roko_agent::chat_types::FinishReason;
use roko_agent::gemini::{Content, GeminiCacheClient, Part};
use roko_agent::mcp::{McpConfig, McpServerConfig};
use roko_agent::perplexity::PerplexitySearchClient;
use roko_agent::provider::is_known_protocol_command;
use roko_agent::safety::scrub::{ScrubPolicy, scrub_secrets};
use roko_agent::task_runner::{
    AnomalyDetector as RunnerAnomalyDetector, BudgetGuardrail as RunnerBudgetGuardrail,
    ConductorBandit as RunnerConductorBandit, CostTable as RunnerCostTable,
    EventBus as RunnerEventBus, ModelPricing as RunnerModelPricing, TaskRunner, TaskRunnerError,
};
use roko_agent::translate::{ClaudeTranslator, RenderedTools, Translator};
use roko_agent::{Agent, AgentResult, SafetyLayer};
use roko_compose::{
    AttentionBidder, ContextProvider, PadState, Placement, PlanArtifacts, PromptComposer,
    PromptSection, SectionPriority, SectionScorer, TaskContext,
};
use roko_conductor::diagnosis::{DiagnosisEngine, ErrorCategory};
use roko_conductor::{Conductor, ConductorDecision};
use roko_core::DaimonPolicy;
use roko_core::Policy;
use roko_core::agent::{ProviderKind, resolve_model};
use roko_core::attestation::{self, SigningKey};
use roko_core::config::schema::RokoConfig;
use roko_core::metric::{ConfigHash, TaskMetric};
use roko_core::obs::health::{AlwaysUpProbe, ProbeRegistry};
use roko_core::obs::{LabelSet, MetricRegistry};
use roko_core::tool::TraceId;
use roko_core::tool::trace::{FailureKind, FailureTrace, TraceStep};
use roko_core::tool::{FormatBandit, ProfileBandit, ToolTraceEvent, TraceSink};
use roko_core::{
    AgentRole, Body, Budget, Composer, ContentHash, Context, Engram, Gate, Kind,
    OperatingFrequency, OperatingFrequencyScheduleContext, PhaseKind, Provenance, Substrate,
    TaskCategory, TaskRequirements, ToolRegistry, Verdict, score_model_for_task,
};
use roko_core::{
    CFactorPolicy, CFactorSource, CFactorSummary, CatalystImpactSummary, CatalystScorer,
    CatalystSignalSource, PredictionPolicy, PredictiveScorer,
};
use roko_daimon::{
    AffectEngine as _, AffectEvent, DaimonState, DispatchParams, SomaticSignal,
    StrategyCoordinates, TaskStrategyObservation,
};
use roko_dreams::{DreamAgentConfig, DreamLoopConfig, DreamRunner};
use roko_fs::FileSubstrate;
use roko_fs::RokoLayout;
use roko_fs::observability::FsObservabilitySinks;
use roko_gate::{
    adaptive_threshold::AdaptiveThresholds, clippy_gate::ClippyGate, compile::CompileGate,
    payload::GatePayload, test_gate::TestGate,
};
use roko_learn::anomaly::{Anomaly, AnomalyDetector};
use roko_learn::budget::{BudgetAction, BudgetGuardrail};
use roko_learn::cfactor::{CFactor, detect_cfactor_regression};
use roko_learn::conductor::{
    ConductorAction as RetryConductorAction, ConductorBandit,
    ConductorState as RetryConductorState, ErrorPattern as RetryErrorPattern, HintType,
};
use roko_learn::costs_db::CostRecord;
use roko_learn::costs_log::CostsLog;
use roko_learn::efficiency::{
    AgentEfficiencyEvent, FleetCFactor, PromptSectionMeta, compute_fleet_cfactor,
};
use roko_learn::episode_logger::{Episode, GateVerdict, Usage};
use roko_learn::events::{AgentEvent, EventBus as LearningEventBus};
use roko_learn::latency::LatencyRegistry;
use roko_learn::model_experiment::ModelExperimentStore;
use roko_learn::playbook::{Playbook, PlaybookStore};
use roko_learn::prediction::CalibrationTracker;
use roko_learn::prompt_experiment::DEFAULT_STATIC_OVERRIDES_PATH;
use roko_learn::routing_log::{
    RoutingDecisionLog, RoutingDecisionLogStore, RoutingDecisionMeta, RoutingLogger,
};
use roko_learn::runtime_feedback::{
    CompletedRunInput, LearningRuntime, LearningUpdate, read_efficiency_events,
    refresh_cfactor_snapshot,
};
use roko_learn::section_effect::{PriorityChange, SectionEffectivenessRegistry};
use roko_learn::skill_library::Skill;
use roko_learn::skill_library::{
    SkillExtractionRequest, SkillGateResult, SkillLibrary, SkillQuery,
};
use roko_neuro::tier_progression::{TierProgression, TierProgressionDecision};
use roko_neuro::{
    EmotionalProvenance, KnowledgeEntry, KnowledgeKind, KnowledgeStore, KnowledgeTier, NeuroStore,
};
use roko_orchestrator::worktree::{WorktreeConfig, WorktreeManager, format_branch_name};
use roko_orchestrator::{
    EventKind, EventLog, EventLogSnapshot, ExecutorAction, ExecutorEvent, ExecutorSnapshot,
    GateResult, ParallelExecutor, PlanState, PostMergeRunner, ReplanResult, ReplanStrategy,
    discover_plans,
};
use roko_runtime::cancel::CancelToken;
use roko_runtime::process::ProcessSupervisor;
use roko_std::StaticToolRegistry;
use roko_std::SumScorer;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::signal;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken as TokioCancellationToken;
use tracing::{Instrument, info_span, instrument};

use crate::agent_config::{
    synthesize_claude_cli_config, synthesize_known_protocol_config, synthesize_subprocess_config,
};
use crate::agent_spawn::{SpawnAgentSpec, spawn_agent_with_layer};
use crate::config::Config;
use crate::heartbeat::{
    HeartbeatClock, HeartbeatProbeKind, HeartbeatProbeResult, HeartbeatSnapshot,
    persist_heartbeat_snapshot,
};
use crate::plan::plans_dir;
use crate::prompting::{
    PromptBuildOptions, build_role_system_prompt, build_role_system_prompt_validated,
};
use crate::task_parser::{TaskValidationIssue, TasksFile};
use crate::worker::cloud::CloudExecution;
use crate::workspace_paths::find_prd_path;

/// Default number of actions between auto-saves.
const AUTOSAVE_INTERVAL: usize = 5;
const DEFAULT_WORKTREE_IDLE_TTL_SECS: u64 = 30 * 60;
const WATCHER_INTERVAL_SECS: u64 = 30;
const WATCHER_SIGNAL_TAIL: usize = 200;
const EFFICIENCY_SIGNAL_TAIL: usize = 256;
const GHOST_TURN_SIGNAL_KIND: &str = "conductor.ghost_turn";
const SHUTDOWN_DRAIN_GRACE_SECS: u64 = 30;

fn model_experiments_path(workdir: &Path) -> PathBuf {
    workdir
        .join(".roko")
        .join("learn")
        .join("model-experiments.json")
}

fn daimon_state_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("daimon").join("affect.json")
}

fn latency_registry_path(workdir: &Path) -> PathBuf {
    workdir
        .join(".roko")
        .join("learn")
        .join("latency-stats.json")
}

fn static_overrides_path(workdir: &Path) -> PathBuf {
    workdir.join(DEFAULT_STATIC_OVERRIDES_PATH)
}

fn routing_log_path(workdir: &Path) -> PathBuf {
    RokoLayout::for_project(workdir)
        .learn_dir()
        .join("routing.jsonl")
}

fn cfactor_history_path(workdir: &Path) -> PathBuf {
    RokoLayout::for_project(workdir)
        .learn_dir()
        .join("c-factor.jsonl")
}

#[derive(Debug, Clone, Copy)]
struct HeartbeatCounts {
    active_tasks: usize,
    ready_tasks: usize,
    completed_tasks: usize,
    failed_tasks: usize,
    completion_rate: f64,
    max_queue_wait_hours: f64,
    cross_plan_blocked: bool,
}

#[derive(Clone)]
struct SectionEffectCatalystSource {
    registry: SectionEffectivenessRegistry,
    role: String,
}

impl CatalystSignalSource for SectionEffectCatalystSource {
    fn impact(&self, signal: &Engram, _ctx: &Context) -> CatalystImpactSummary {
        let mut summary = CatalystImpactSummary {
            reuse_count: parse_count_tag(signal, "reuse_count"),
            confirmation_count: parse_count_tag(signal, "confirmation_count"),
            confidence: if signal.lineage.is_empty() { 0.0 } else { 0.2 },
            ..CatalystImpactSummary::default()
        };

        if let Ok(section) = PromptSection::from_signal(signal) {
            if let Some(effect) = self.registry.get(&section.name, &self.role) {
                let compared_trials = effect.included_trials.min(effect.excluded_trials) as f32;
                summary.downstream_impact = effect.lift().clamp(0.0, 1.0) as f32;
                summary.reuse_count = summary.reuse_count.max(effect.included_trials as usize);
                summary.confirmation_count = summary
                    .confirmation_count
                    .max(effect.included_passes as usize);
                summary.confidence = (compared_trials / 20.0).clamp(0.0, 1.0);
            }
        }

        summary
    }
}

#[derive(Clone)]
struct StaticCFactorSource {
    summary: Option<CFactorSummary>,
}

impl CFactorSource for StaticCFactorSource {
    fn summary(&self) -> Option<CFactorSummary> {
        self.summary.clone()
    }
}

async fn load_predictive_calibration(workdir: &Path) -> Option<Arc<CalibrationTracker>> {
    match CalibrationTracker::load_from_routing_log(routing_log_path(workdir)).await {
        Ok(tracker) if !tracker.is_empty() => Some(Arc::new(tracker)),
        Ok(_) => None,
        Err(err) => {
            tracing::debug!(error = %err, "failed to load predictive calibration");
            None
        }
    }
}

async fn load_cfactor_source(workdir: &Path) -> Option<Arc<StaticCFactorSource>> {
    let contents = match tokio::fs::read_to_string(cfactor_history_path(workdir)).await {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            tracing::debug!(error = %err, "failed to load c-factor history");
            return None;
        }
    };

    let mut history = contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str::<CFactor>(line).ok())
        .collect::<Vec<_>>();
    history.sort_by(|left, right| left.computed_at.cmp(&right.computed_at));
    let current = history.last()?.clone();
    let historical_average = if history.len() > 1 {
        history[..history.len() - 1]
            .iter()
            .map(|snapshot| snapshot.overall)
            .sum::<f64>()
            / (history.len() - 1) as f64
    } else {
        current.overall
    };
    let trend = current.overall - historical_average;
    let regression =
        detect_cfactor_regression(&history, Duration::from_secs(7 * 24 * 60 * 60), 0.08);
    let (top_positive_contributors, top_negative_contributors) = top_cfactor_contributors(&current);

    Some(Arc::new(StaticCFactorSource {
        summary: Some(CFactorSummary {
            overall: current.overall,
            trend,
            regression_drop: regression.map_or(0.0, |entry| entry.drop_fraction),
            gate_pass_rate: current.components.gate_pass_rate,
            turn_taking_equality: current.components.turn_taking_equality,
            social_sensitivity: current.components.social_sensitivity,
            task_diversity_coverage: current.components.task_diversity_coverage,
            episode_count: current.episode_count,
            top_positive_contributors,
            top_negative_contributors,
        }),
    }))
}

fn predictive_policy_sections(
    calibration: Arc<CalibrationTracker>,
    model_slug: &str,
    task_category: &str,
) -> Vec<PromptSection> {
    let policy = PredictionPolicy::new(calibration).with_min_samples(6);
    let seed = Engram::builder(Kind::Prediction)
        .body(Body::text(format!(
            "Predictive calibration seed for {model_slug}/{task_category}"
        )))
        .tag("model_slug", model_slug)
        .tag("task_category", task_category)
        .build();
    policy
        .decide(&[seed], &Context::now())
        .into_iter()
        .filter_map(|engram| {
            let text = engram.body.as_text().ok()?.trim().to_string();
            if text.is_empty() {
                return None;
            }
            Some(
                PromptSection::new("predictive-calibration", text)
                    .with_priority(SectionPriority::Normal)
                    .with_placement(Placement::Middle)
                    .with_hard_cap(256)
                    .with_bidder(AttentionBidder::Oracles),
            )
        })
        .collect()
}

fn cfactor_policy_sections(source: Arc<dyn CFactorSource>) -> Vec<PromptSection> {
    let policy = CFactorPolicy::new(source).with_min_episode_count(6);
    policy
        .decide(&[], &Context::now())
        .into_iter()
        .filter_map(|engram| {
            let text = engram.body.as_text().ok()?.trim().to_string();
            if text.is_empty() {
                return None;
            }
            Some(
                PromptSection::new("collective-calibration", text)
                    .with_priority(SectionPriority::Normal)
                    .with_placement(Placement::Middle)
                    .with_hard_cap(256)
                    .with_bidder(AttentionBidder::Oracles),
            )
        })
        .collect()
}

fn parse_count_tag(signal: &Engram, key: &str) -> usize {
    signal
        .tag(key)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0)
}

fn top_cfactor_contributors(snapshot: &CFactor) -> (Vec<String>, Vec<String>) {
    let mut positive = snapshot
        .agent_contributions
        .iter()
        .filter(|contribution| contribution.contribution_score > 0.0)
        .cloned()
        .collect::<Vec<_>>();
    positive.sort_by(|left, right| {
        right
            .contribution_score
            .total_cmp(&left.contribution_score)
            .then(left.agent_id.cmp(&right.agent_id))
    });

    let mut negative = snapshot
        .agent_contributions
        .iter()
        .filter(|contribution| contribution.contribution_score < 0.0)
        .cloned()
        .collect::<Vec<_>>();
    negative.sort_by(|left, right| {
        left.contribution_score
            .total_cmp(&right.contribution_score)
            .then(left.agent_id.cmp(&right.agent_id))
    });

    (
        positive
            .into_iter()
            .take(3)
            .map(|contribution| {
                format!(
                    "{}={:+.3}",
                    contribution.agent_id, contribution.contribution_score
                )
            })
            .collect(),
        negative
            .into_iter()
            .take(3)
            .map(|contribution| {
                format!(
                    "{}={:+.3}",
                    contribution.agent_id, contribution.contribution_score
                )
            })
            .collect(),
    )
}

fn task_requirements_for_routing(
    task_def: Option<&crate::task_parser::TaskDef>,
    role: AgentRole,
    task_text: &str,
    allowed_tools_csv: &str,
    context_window_tokens: u64,
) -> TaskRequirements {
    let task_lower = task_text.to_ascii_lowercase();
    let tools_lower = allowed_tools_csv.to_ascii_lowercase();
    let tier = task_def.map(|task| task.tier.as_str()).unwrap_or("focused");
    let min_context_window = match tier {
        "mechanical" => 32_000,
        "focused" => 64_000,
        "integrative" => 128_000,
        "architectural" => 200_000,
        _ => 64_000,
    }
    .max(context_window_tokens.min(256_000));

    let needs_web_search = task_def
        .is_some_and(crate::task_parser::TaskDef::needs_external_context)
        || matches!(role, AgentRole::Researcher | AgentRole::PatternExtractor)
        || task_lower.contains("research")
        || task_lower.contains("look up")
        || tools_lower.contains("web");
    let needs_code_execution =
        tools_lower.contains("python") || tools_lower.contains("code_execution");
    let needs_thinking = matches!(
        role,
        AgentRole::Strategist | AgentRole::Architect | AgentRole::Auditor
    ) || matches!(tier, "integrative" | "architectural");
    let needs_vision = task_lower.contains("image")
        || task_lower.contains("screenshot")
        || task_lower.contains("vision");
    let needs_structured_output = matches!(
        role,
        AgentRole::Strategist | AgentRole::Auditor | AgentRole::Scribe
    ) || task_lower.contains("json")
        || task_lower.contains("yaml")
        || task_lower.contains("toml")
        || task_lower.contains("schema");
    let max_cost_output_per_m = match tier {
        "mechanical" => Some(10.0),
        "focused" => Some(18.0),
        _ => None,
    };
    let max_latency_ms = match tier {
        "mechanical" => Some(3_000),
        "focused" => Some(6_000),
        _ => None,
    };

    TaskRequirements {
        needs_web_search,
        needs_code_execution,
        needs_thinking,
        needs_vision,
        needs_structured_output,
        min_context_window,
        max_cost_output_per_m,
        max_latency_ms,
    }
}

fn conductor_policy_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("learn").join("conductor.json")
}

fn scrub_json_value(value: &serde_json::Value, policy: &ScrubPolicy) -> serde_json::Value {
    match value {
        serde_json::Value::String(text) => serde_json::Value::String(scrub_secrets(text, policy)),
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|item| scrub_json_value(item, policy))
                .collect(),
        ),
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), scrub_json_value(value, policy)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn scrub_body(body: &Body, policy: &ScrubPolicy) -> Body {
    match body {
        Body::Text(text) => Body::text(scrub_secrets(text, policy)),
        Body::Json(value) => Body::Json(scrub_json_value(value, policy)),
        other => other.clone(),
    }
}

fn scrub_signal(signal: &Engram, policy: &ScrubPolicy) -> Engram {
    let scrubbed_body = scrub_body(&signal.body, policy);
    if scrubbed_body == signal.body {
        return signal.clone();
    }

    let mut scrubbed = signal.clone();
    scrubbed.body = scrubbed_body;
    scrubbed.id = scrubbed.content_hash();
    scrubbed
}

fn scrub_agent_result(result: &AgentResult, policy: &ScrubPolicy) -> AgentResult {
    AgentResult {
        output: scrub_signal(&result.output, policy),
        trace: result
            .trace
            .iter()
            .map(|signal| scrub_signal(signal, policy))
            .collect(),
        usage: result.usage,
        success: result.success,
    }
}

#[derive(Debug, Clone)]
struct TurnLearningFeedback {
    task_id: String,
    model: String,
    provider: String,
    timestamp_ms: i64,
    prompt_hash: u64,
    ttft_ms: u64,
    total_ms: u64,
    output_tokens: u64,
    cost_usd: f64,
    usage: roko_agent::Usage,
    success: bool,
}

fn publish_turn_learning_feedback(
    event_bus: &LearningEventBus,
    latency_registry: &LatencyRegistry,
    anomaly_detector: &mut AnomalyDetector,
    feedback: TurnLearningFeedback,
) {
    let mut rx = event_bus.subscribe();
    event_bus.publish(AgentEvent::TurnStarted {
        task_id: feedback.task_id.clone(),
        model: feedback.model.clone(),
        provider: feedback.provider.clone(),
        timestamp_ms: feedback.timestamp_ms,
    });
    event_bus.publish(AgentEvent::TurnCompleted {
        turn: 1,
        usage: feedback.usage,
        tool_call_count: 0,
        gate_passed: Some(feedback.success),
        finish_reason: if feedback.success {
            FinishReason::Stop
        } else {
            FinishReason::Error("agent failed".to_string())
        },
    });
    event_bus.publish(AgentEvent::CostRecorded {
        model: feedback.model.clone(),
        provider: feedback.provider.clone(),
        cost_usd: feedback.cost_usd,
        tokens: u64::from(feedback.usage.total_tokens()),
    });

    drain_turn_learning_events(&mut rx, latency_registry, anomaly_detector, &feedback);
}

fn drain_turn_learning_events(
    rx: &mut tokio::sync::broadcast::Receiver<AgentEvent>,
    latency_registry: &LatencyRegistry,
    anomaly_detector: &mut AnomalyDetector,
    feedback: &TurnLearningFeedback,
) {
    loop {
        match rx.try_recv() {
            Ok(AgentEvent::TurnStarted { .. }) => {
                if let Some(anomaly) = anomaly_detector.check_prompt(feedback.prompt_hash) {
                    tracing::warn!(
                        model = %feedback.model,
                        provider = %feedback.provider,
                        ?anomaly,
                        "learning anomaly detected from prompt"
                    );
                }
            }
            Ok(AgentEvent::TurnCompleted { .. }) => {
                latency_registry.record(
                    &feedback.model,
                    &feedback.provider,
                    feedback.ttft_ms as f64,
                    feedback.total_ms as f64,
                    feedback.output_tokens,
                );
                tracing::info!(
                    model = %feedback.model,
                    provider = %feedback.provider,
                    ttft_ms = feedback.ttft_ms,
                    total_ms = feedback.total_ms,
                    output_tokens = feedback.output_tokens,
                    "learning latency recorded"
                );
            }
            Ok(AgentEvent::CostRecorded { .. }) => {
                if let Some(anomaly) = anomaly_detector.check_cost(feedback.cost_usd) {
                    tracing::warn!(
                        model = %feedback.model,
                        provider = %feedback.provider,
                        ?anomaly,
                        "learning anomaly detected from cost"
                    );
                } else {
                    tracing::info!(
                        model = %feedback.model,
                        provider = %feedback.provider,
                        "learning anomaly scan complete"
                    );
                }
            }
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            | Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(skipped)) => {
                tracing::warn!(skipped, "learning feedback lagged behind event stream");
            }
        }
    }
}

fn state_dir(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("state")
}

fn executor_snapshot_path(workdir: &Path) -> PathBuf {
    state_dir(workdir).join("executor.json")
}

fn save_snapshot_atomic(snapshot: &ExecutorSnapshot, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create snapshot dir {}", parent.display()))?;
    }
    let tmp_path = path.with_extension("json.tmp");
    let json = snapshot.to_json().map_err(|e| anyhow!("snapshot: {e}"))?;
    std::fs::write(&tmp_path, &json)
        .with_context(|| format!("write snapshot tmp {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path)
        .with_context(|| format!("rename snapshot {}", path.display()))?;
    Ok(())
}

async fn wait_for_shutdown_signal() -> Result<&'static str> {
    #[cfg(unix)]
    {
        let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
            .map_err(|e| anyhow!("install SIGTERM handler: {e}"))?;
        tokio::select! {
            result = signal::ctrl_c() => {
                result.map_err(|e| anyhow!("wait for SIGINT: {e}"))?;
                Ok("SIGINT")
            }
            _ = sigterm.recv() => Ok("SIGTERM"),
        }
    }

    #[cfg(not(unix))]
    {
        signal::ctrl_c()
            .await
            .map_err(|e| anyhow!("wait for Ctrl+C: {e}"))?;
        Ok("SIGINT")
    }
}

fn sync_file_if_present(path: &Path) -> Result<()> {
    match std::fs::File::open(path) {
        Ok(file) => file
            .sync_all()
            .with_context(|| format!("sync {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("open {}", path.display())),
    }
}

fn install_episode_distillation_hook(learning: &mut LearningRuntime, workdir: &Path) {
    let distillation_workdir = workdir.to_path_buf();
    learning.set_episode_completion_hook(move |episode| {
        roko_neuro::spawn_episode_distillation(distillation_workdir.clone(), episode);
    });
}

fn load_roko_config(workdir: &Path) -> Result<RokoConfig> {
    let path = workdir.join("roko.toml");
    if !path.exists() {
        return Ok(RokoConfig::default());
    }

    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    RokoConfig::from_toml(&text).with_context(|| format!("parse {}", path.display()))
}

fn frequency_label(frequency: OperatingFrequency) -> &'static str {
    match frequency {
        OperatingFrequency::Gamma => "gamma",
        OperatingFrequency::Theta => "theta",
        OperatingFrequency::Delta => "delta",
    }
}

fn task_runner_cost_table(resolved: &roko_core::agent::ResolvedModel) -> RunnerCostTable {
    let mut cost_table = RunnerCostTable::default();

    if let Some(profile) = resolved.profile.as_ref() {
        cost_table.insert(
            resolved.slug.clone(),
            RunnerModelPricing {
                input_per_m: profile.cost_input_per_m.unwrap_or(0.0),
                output_per_m: profile.cost_output_per_m.unwrap_or(0.0),
                cache_read_per_m: profile.cost_cache_read_per_m.unwrap_or(0.0),
                cache_write_per_m: profile.cost_cache_write_per_m.unwrap_or(0.0),
            },
        );
    }

    cost_table
}

fn routing_model_provider_map(config: &RokoConfig) -> HashMap<String, String> {
    let mut providers = HashMap::new();
    for (model_key, profile) in config.effective_models() {
        providers.insert(model_key, profile.provider.clone());
        providers.entry(profile.slug).or_insert(profile.provider);
    }
    providers
}

fn provider_id_for_routing_model(
    config: &RokoConfig,
    model_providers: &HashMap<String, String>,
    model: &str,
) -> String {
    model_providers.get(model).cloned().unwrap_or_else(|| {
        let resolved = resolve_model(config, model);
        resolved
            .profile
            .map(|profile| profile.provider)
            .unwrap_or_else(|| resolved.provider_kind.label().to_owned())
    })
}

// ─── ContextAttributionTracker ────────────────────────────────────────────

/// Tracks per-(tier, source_type) context attribution rates.
/// Loaded from `.roko/context-attribution.jsonl` at startup.
struct ContextAttributionTracker {
    /// (tier, source_type) -> (referenced_count, total_count)
    rates: HashMap<(String, String), (usize, usize)>,
}

impl ContextAttributionTracker {
    fn load(path: &Path) -> Self {
        let mut rates: HashMap<(String, String), (usize, usize)> = HashMap::new();
        if let Ok(contents) = std::fs::read_to_string(path) {
            for line in contents.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                    if let (Some(tier), Some(source_type)) = (
                        v.get("tier").and_then(|t| t.as_str()),
                        v.get("source_type").and_then(|s| s.as_str()),
                    ) {
                        let referenced = v
                            .get("referenced")
                            .and_then(|r| r.as_bool())
                            .unwrap_or(false);
                        let entry = rates
                            .entry((tier.to_string(), source_type.to_string()))
                            .or_insert((0, 0));
                        if referenced {
                            entry.0 += 1;
                        }
                        entry.1 += 1;
                    }
                }
            }
        }
        Self { rates }
    }

    fn record(&mut self, tier: &str, source_type: &str, referenced: bool) {
        let entry = self
            .rates
            .entry((tier.to_string(), source_type.to_string()))
            .or_insert((0, 0));
        if referenced {
            entry.0 += 1;
        }
        entry.1 += 1;
    }
}

/// Tracks rolling EMA per-(task_tier, context_source_type).
struct ContextAverageTracker {
    path: PathBuf,
    averages: HashMap<String, HashMap<String, ContextAverageStats>>,
}

/// Per-(task_tier, context_source_type) rolling average state.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
struct ContextAverageStats {
    /// Exponential moving average of reference rate.
    ema_reference_rate: f64,
    /// Total observations seen for this pair.
    total_observations: u64,
}

impl ContextAverageTracker {
    fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let averages = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| {
                serde_json::from_str::<HashMap<String, HashMap<String, ContextAverageStats>>>(&s)
                    .ok()
            })
            .unwrap_or_default();
        Self { path, averages }
    }

    fn record(&mut self, tier: &str, source_type: &str, referenced: bool) {
        const EMA_ALPHA: f64 = 0.1;
        let tier_entry = self.averages.entry(tier.to_string()).or_default();
        let stats = tier_entry.entry(source_type.to_string()).or_default();
        let value = if referenced { 1.0 } else { 0.0 };

        if stats.total_observations == 0 {
            stats.ema_reference_rate = value;
        } else {
            stats.ema_reference_rate =
                EMA_ALPHA.mul_add(value, (1.0 - EMA_ALPHA) * stats.ema_reference_rate);
        }
        stats.total_observations += 1;
    }

    fn save(&self) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(&self.averages)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

// ─── CrateFamiliarityTracker ──────────────────────────────────────────────

/// Tracks per-crate task outcomes so we can derive a familiarity score.
///
/// The score is `success_count / total_count` for the crate touched by the
/// task's changed files. This is persisted as JSONL under `.roko/learn/` and
/// reloaded on startup so routing has history across runs.
struct CrateFamiliarityTracker {
    /// Append-only JSONL file with crate outcome observations.
    path: PathBuf,
    /// crate_name -> (success_count, total_count)
    stats: HashMap<String, (u64, u64)>,
}

impl CrateFamiliarityTracker {
    fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut stats: HashMap<String, (u64, u64)> = HashMap::new();
        if let Ok(contents) = std::fs::read_to_string(&path) {
            for line in contents.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(line)
                    && let Some(crate_name) = v.get("crate_name").and_then(|c| c.as_str())
                {
                    let success = v.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
                    let entry = stats.entry(crate_name.to_string()).or_insert((0, 0));
                    if success {
                        entry.0 += 1;
                    }
                    entry.1 += 1;
                }
            }
        }
        Self { path, stats }
    }

    /// Return the familiarity score for the crate touched by `task_def`.
    #[must_use]
    fn score_for_task(&self, task_def: Option<&crate::task_parser::TaskDef>) -> f64 {
        task_crate_name(task_def)
            .as_deref()
            .map(|crate_name| self.score_for_crate(crate_name))
            .unwrap_or(0.5)
    }

    /// Record one task outcome for every crate touched by the task files.
    fn record_task_outcome(
        &mut self,
        task_def: Option<&crate::task_parser::TaskDef>,
        success: bool,
        plan_id: &str,
        task_id: &str,
    ) -> std::io::Result<()> {
        let Some(crate_name) = task_crate_name(task_def) else {
            return Ok(());
        };

        let entry = self.stats.entry(crate_name.clone()).or_insert((0, 0));
        if success {
            entry.0 += 1;
        }
        entry.1 += 1;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let record = serde_json::json!({
            "plan_id": plan_id,
            "task_id": task_id,
            "crate_name": crate_name,
            "success": success,
            "ts": chrono::Utc::now().to_rfc3339(),
        });
        writeln!(file, "{}", record)?;
        Ok(())
    }

    fn score_for_crate(&self, crate_name: &str) -> f64 {
        match self.stats.get(crate_name) {
            Some(&(success_count, total_count)) if total_count > 0 => {
                success_count as f64 / total_count as f64
            }
            _ => 0.5,
        }
    }
}

/// Derive a crate name from the task's modified files.
fn task_crate_name(task_def: Option<&crate::task_parser::TaskDef>) -> Option<String> {
    let mut seen = HashSet::new();
    task_def
        .into_iter()
        .flat_map(|task| task.files.iter())
        .filter_map(|file| crate_name_for_path(file))
        .find(|crate_name| seen.insert(crate_name.clone()))
}

/// Best-effort crate key derivation from a repository-relative file path.
fn crate_name_for_path(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let parts: Vec<&str> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    match parts.as_slice() {
        [first, second, ..] if *first == "crates" || *first == "apps" => {
            Some((*second).to_string())
        }
        [first, second, ..] if matches!(*second, "src" | "tests" | "benches") => {
            Some((*first).to_string())
        }
        [first, ..] if matches!(*first, "src" | "tests" | "benches") => {
            Some("workspace".to_string())
        }
        [first, ..] if *first == "Cargo.toml" => Some("workspace".to_string()),
        _ => None,
    }
}

fn crate_root_for_path(path: &str) -> Option<PathBuf> {
    let normalized = path.replace('\\', "/");
    let parts: Vec<&str> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    match parts.as_slice() {
        [first, second, ..] if *first == "crates" || *first == "apps" => {
            Some(PathBuf::from(first).join(second))
        }
        [first, ..] if matches!(*first, "src" | "tests" | "benches" | "examples") => {
            Some(PathBuf::new())
        }
        [first] if matches!(*first, "Cargo.toml" | "build.rs") => Some(PathBuf::new()),
        _ => None,
    }
}

fn collect_crate_source_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry.with_context(|| format!("read entry in {}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_crate_source_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }

    Ok(())
}

fn read_full_crate_source(crate_root: &Path) -> Result<String> {
    let mut files = Vec::new();

    for path in [crate_root.join("Cargo.toml"), crate_root.join("build.rs")] {
        if path.is_file() {
            files.push(path);
        }
    }
    for dir in ["src", "tests", "benches", "examples"] {
        collect_crate_source_files(&crate_root.join(dir), &mut files)?;
    }

    files.sort();
    files.dedup();

    let mut combined = String::new();
    for path in files {
        let contents =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let relative = path.strip_prefix(crate_root).unwrap_or(path.as_path());
        combined.push_str(&format!(
            "// FILE: {}\n{}\n\n",
            relative.display(),
            contents
        ));
    }

    Ok(combined)
}

fn log_tasks_validation_issue(
    plan_id: &str,
    plan_base: &str,
    tasks_path: &Path,
    issue: &TaskValidationIssue,
) {
    match issue {
        TaskValidationIssue::MissingRequiredField { task_id, field } => {
            tracing::error!(
                target: "plan_validation",
                plan_id = %plan_id,
                plan_base = %plan_base,
                tasks_path = %tasks_path.display(),
                issue = "missing_required_field",
                task_id = %task_id,
                field = field,
                "tasks.toml validation failed"
            );
        }
        TaskValidationIssue::UnknownDependency {
            task_id,
            dependency,
        } => {
            tracing::error!(
                target: "plan_validation",
                plan_id = %plan_id,
                plan_base = %plan_base,
                tasks_path = %tasks_path.display(),
                issue = "unknown_dependency",
                task_id = %task_id,
                dependency = %dependency,
                "tasks.toml validation failed"
            );
        }
        TaskValidationIssue::CircularDependency { cycle } => {
            tracing::error!(
                target: "plan_validation",
                plan_id = %plan_id,
                plan_base = %plan_base,
                tasks_path = %tasks_path.display(),
                issue = "circular_dependency",
                cycle = ?cycle,
                "tasks.toml validation failed"
            );
        }
        TaskValidationIssue::NoStartNode => {
            tracing::error!(
                target: "plan_validation",
                plan_id = %plan_id,
                plan_base = %plan_base,
                tasks_path = %tasks_path.display(),
                issue = "no_start_node",
                "tasks.toml validation failed"
            );
        }
    }
}

fn validate_tasks_file_for_execution(
    plan_id: &str,
    plan_base: &str,
    tasks_path: &Path,
    tasks_file: &TasksFile,
) -> Result<()> {
    let issues = tasks_file.validate_structure();
    if issues.is_empty() {
        return Ok(());
    }

    for issue in &issues {
        log_tasks_validation_issue(plan_id, plan_base, tasks_path, issue);
    }

    Err(anyhow!(
        "tasks.toml validation failed for {}",
        tasks_path.display()
    ))
}

// ─── Parallel agent execution ────────────────────────────────────────────

/// Owned data needed to run a single agent subprocess in isolation.
/// Constructed from `PlanRunner` state, then run in parallel without
/// borrowing the runner.
struct AgentRunConfig {
    command: String,
    exec_dir: PathBuf,
    model: String,
    timeout_ms: u64,
    bare_mode: bool,
    effort: String,
    system_prompt: String,
    allowed_tools_csv: String,
    mcp_config: Option<PathBuf>,
    fallback_model: Option<String>,
    env_vars: Vec<(String, String)>,
    read_args: Vec<String>,
    extra_args: Vec<String>,
    resume_session: Option<String>,
    prompt: String,
    skip_permissions: bool,
}

/// Result bundle returned from a parallel task execution.
struct ParallelTaskResult {
    task_id: String,
    exec_dir: PathBuf,
    prompt_text: String,
    system_prompt: String,
    model: String,
    result: AgentResult,
}

/// Run a prepared agent configuration. No `PlanRunner` borrow required.
async fn run_prepared_agent(cfg: AgentRunConfig) -> AgentResult {
    let ctx = Context::now();
    let prompt_signal = Engram::builder(Kind::Task)
        .body(Body::Text(cfg.prompt.clone()))
        .build();
    let mut routing_config = load_roko_config(&cfg.exec_dir).unwrap_or_default();
    routing_config.apply_process_env();
    let has_routing = !routing_config.providers.is_empty() || !routing_config.models.is_empty();

    if has_routing {
        let mut extra_args = cfg.read_args;
        extra_args.extend(cfg.extra_args);
        if let Some(fallback_model) = cfg.fallback_model.clone() {
            extra_args.push("--fallback-model".to_string());
            extra_args.push(fallback_model);
        }
        if let Some(resume_session) = cfg.resume_session.clone() {
            extra_args.push("--resume".to_string());
            extra_args.push(resume_session);
        }

        match spawn_agent_with_layer(
            &routing_config,
            None,
            SpawnAgentSpec {
                model: cfg.model.clone(),
                command: Some(cfg.command.clone()),
                timeout_ms: Some(cfg.timeout_ms),
                system_prompt: Some(cfg.system_prompt.clone()),
                cached_content: None,
                tools: Some(cfg.allowed_tools_csv.clone()),
                mcp_config: cfg.mcp_config.clone(),
                working_dir: Some(cfg.exec_dir.clone()),
                env: cfg.env_vars.clone(),
                extra_args,
                effort: Some(cfg.effort.clone()),
                bare_mode: cfg.bare_mode,
                dangerously_skip_permissions: cfg.skip_permissions,
                name: String::new(),
            },
            format!("create prepared agent for {}", cfg.model),
        ) {
            Ok(agent) => agent.run(&prompt_signal, &ctx).await,
            Err(err) => AgentResult::fail(
                prompt_signal
                    .derive(
                        Kind::AgentOutput,
                        Body::text(format!(
                            "create prepared agent for {} failed: {err}",
                            cfg.model
                        )),
                    )
                    .build(),
            ),
        }
    } else if cfg.command == "claude" {
        let synthesized_config = synthesize_claude_cli_config(&cfg.command, &cfg.model);

        let mut extra_args = cfg.read_args;
        extra_args.extend(cfg.extra_args);
        if let Some(fallback_model) = cfg.fallback_model.clone() {
            extra_args.push("--fallback-model".to_string());
            extra_args.push(fallback_model);
        }
        if let Some(resume_session) = cfg.resume_session.clone() {
            extra_args.push("--resume".to_string());
            extra_args.push(resume_session);
        }

        match spawn_agent_with_layer(
            &synthesized_config,
            None,
            SpawnAgentSpec {
                model: cfg.model.clone(),
                command: Some(cfg.command.clone()),
                timeout_ms: Some(cfg.timeout_ms),
                system_prompt: Some(cfg.system_prompt),
                cached_content: None,
                tools: Some(cfg.allowed_tools_csv),
                mcp_config: cfg.mcp_config,
                working_dir: Some(cfg.exec_dir.clone()),
                env: cfg.env_vars,
                extra_args,
                effort: Some(cfg.effort),
                bare_mode: cfg.bare_mode,
                dangerously_skip_permissions: cfg.skip_permissions,
                name: String::new(),
            },
            format!("create synthesized claude agent for {}", cfg.model),
        ) {
            Ok(agent) => agent.run(&prompt_signal, &ctx).await,
            Err(err) => AgentResult::fail(
                prompt_signal
                    .derive(
                        Kind::AgentOutput,
                        Body::text(format!(
                            "create synthesized claude agent for {} failed: {err}",
                            cfg.model
                        )),
                    )
                    .build(),
            ),
        }
    } else if is_known_protocol_command(&cfg.command) {
        let fallback_config = synthesize_known_protocol_config(&cfg.command, &cfg.model);

        match spawn_agent_with_layer(
            &fallback_config,
            None,
            SpawnAgentSpec {
                model: cfg.model.clone(),
                command: Some(cfg.command.clone()),
                timeout_ms: Some(cfg.timeout_ms),
                system_prompt: None,
                cached_content: None,
                tools: None,
                mcp_config: None,
                working_dir: Some(cfg.exec_dir.clone()),
                env: cfg.env_vars,
                extra_args: cfg.extra_args,
                effort: None,
                bare_mode: cfg.bare_mode,
                dangerously_skip_permissions: false,
                name: String::new(),
            },
            format!("create known-protocol subprocess agent for {}", cfg.command),
        ) {
            Ok(agent) => agent.run(&prompt_signal, &ctx).await,
            Err(err) => AgentResult::fail(
                prompt_signal
                    .derive(
                        Kind::AgentOutput,
                        Body::text(format!(
                            "create known-protocol subprocess agent for {} failed: {err}",
                            cfg.command
                        )),
                    )
                    .build(),
            ),
        }
    } else {
        let model = cfg.model.clone();
        let fallback_config = synthesize_subprocess_config(&cfg.command);
        match spawn_agent_with_layer(
            &fallback_config,
            None,
            SpawnAgentSpec {
                model: model.clone(),
                command: Some(cfg.command.clone()),
                timeout_ms: Some(cfg.timeout_ms),
                system_prompt: Some(cfg.system_prompt),
                cached_content: None,
                tools: Some(cfg.allowed_tools_csv),
                mcp_config: cfg.mcp_config,
                working_dir: Some(cfg.exec_dir.clone()),
                env: cfg.env_vars,
                extra_args: cfg.extra_args,
                effort: Some(cfg.effort),
                bare_mode: cfg.bare_mode,
                dangerously_skip_permissions: cfg.skip_permissions,
                name: String::new(),
            },
            format!("create generic subprocess agent for {}", cfg.command),
        ) {
            Ok(agent) => agent.run(&prompt_signal, &ctx).await,
            Err(err) => AgentResult::fail(
                prompt_signal
                    .derive(
                        Kind::AgentOutput,
                        Body::text(format!(
                            "create generic subprocess agent for {} failed: {err}",
                            cfg.command
                        )),
                    )
                    .build(),
            ),
        }
    }
}

// ─── Report types ─────────────────────────────────────────────────────────

/// Report returned after a single plan's execution completes.
#[derive(Debug, Clone)]
pub struct PlanRunReport {
    /// Plan ID.
    pub plan_id: String,
    /// Whether the plan reached a success terminal phase.
    pub succeeded: bool,
    /// Number of agent invocations for this plan.
    pub agent_calls: usize,
    /// Gate results collected during execution.
    pub gate_results: Vec<(String, bool)>,
}

/// Summary of the entire orchestration run across all plans.
#[derive(Debug, Clone)]
pub struct OrchestrationReport {
    /// Per-plan results.
    pub plans: Vec<PlanRunReport>,
    /// Total agent invocations across all plans.
    pub total_agent_calls: usize,
    /// Total gate runs across all plans.
    pub total_gate_runs: usize,
    /// Fleet-level C-Factor for the current `roko plan run` session.
    pub fleet_cfactor: Option<FleetCFactor>,
}

impl OrchestrationReport {
    /// True if every plan reached a success terminal state.
    #[must_use]
    pub fn all_succeeded(&self) -> bool {
        self.plans.iter().all(|p| p.succeeded)
    }
}

/// Health probe that checks if a CLI command is findable on PATH.
struct CliProbe {
    command: String,
}

impl roko_core::obs::health::Probe for CliProbe {
    fn name(&self) -> &str {
        &self.command
    }
    fn check(&self) -> Result<(), roko_core::obs::health::DegradedReason> {
        // Use `command -v` to check PATH availability without adding a dep.
        let ok = std::process::Command::new("sh")
            .args(["-c", &format!("command -v {}", self.command)])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            Ok(())
        } else {
            Err(roko_core::obs::health::DegradedReason::new(
                &self.command,
                format!("command '{}' not found on PATH", self.command),
            ))
        }
    }
}

/// Context gathered from the learning subsystem for a given task dispatch.
///
/// Includes the prompt text plus IDs of matched skills/rules so confidence
/// can be updated after the task completes.
struct LearnedContext {
    /// Assembled context text to inject into the agent prompt.
    text: String,
    /// The best-match skill ID (if any) for confidence updates.
    matched_skill_id: Option<String>,
    /// The best-match playbook rule ID (if any) for confidence updates.
    matched_rule_id: Option<String>,
    /// The assigned prompt experiment variant ID (if any) for outcome tracking.
    experiment_variant_id: Option<String>,
}

struct SelectedModelExperiment {
    experiment_id: String,
    variant_id: String,
    model_slug: String,
}

/// Background checker that tails `.roko/signals.jsonl` and periodically
/// runs the conductor against the most recent signals.
struct WatcherRunner {
    conductor: Arc<Conductor>,
    signals_path: PathBuf,
    efficiency_path: PathBuf,
    budget_usd: Option<f64>,
    cancel: TokioCancellationToken,
}

impl WatcherRunner {
    fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move { self.run().await })
    }

    async fn run(self) {
        let mut interval = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(WATCHER_INTERVAL_SECS),
            Duration::from_secs(WATCHER_INTERVAL_SECS),
        );
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => break,
                _ = interval.tick() => {
                    match load_recent_signals(&self.signals_path, WATCHER_SIGNAL_TAIL).await {
                        Ok(recent_signals) => {
                            let mut signals = recent_signals;
                            if let Ok(cost_signals) = load_efficiency_cost_signals(
                                &self.efficiency_path,
                                self.budget_usd,
                            )
                            .await
                            {
                                signals.extend(cost_signals);
                            }
                            let findings = self.conductor.check_all(&signals);
                            let alert_signals: Vec<Engram> = findings
                                .into_iter()
                                .filter(|signal| {
                                    matches!(
                                        &signal.kind,
                                        Kind::Custom(kind) if kind.starts_with("conductor:alert:")
                                    )
                                })
                                .collect();
                            if !alert_signals.is_empty() {
                                if let Some(root) = self.signals_path.parent() {
                                    match FileSubstrate::open(root).await {
                                        Ok(substrate) => {
                                            for signal in alert_signals {
                                                if let Err(e) = substrate.put(signal).await {
                                                    tracing::error!(
                                                        "[conductor] watcher runner failed to persist alert to {}: {e}",
                                                        self.signals_path.display()
                                                    );
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "[conductor] watcher runner failed to open {}: {e}",
                                                root.display()
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                "[conductor] watcher runner failed to read {}: {e}",
                                self.signals_path.display()
                            );
                        }
                    }
                }
            }
        }
    }
}

async fn load_recent_signals(path: &Path, tail_len: usize) -> std::io::Result<Vec<Engram>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let text = tokio::fs::read_to_string(path).await?;
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(tail_len);
    let mut signals = Vec::with_capacity(lines.len().saturating_sub(start));
    for line in &lines[start..] {
        if let Ok(signal) = serde_json::from_str::<Engram>(line) {
            signals.push(signal);
        }
    }
    Ok(signals)
}

/// Load the latest efficiency entries and convert them into cost metric signals.
async fn load_efficiency_cost_signals(
    path: &Path,
    budget_usd: Option<f64>,
) -> std::io::Result<Vec<Engram>> {
    let Some(budget_usd) = budget_usd.filter(|budget| *budget > 0.0) else {
        return Ok(Vec::new());
    };

    let text = tokio::fs::read_to_string(path).await?;
    Ok(build_cost_overrun_signals(&text, budget_usd))
}

/// Synchronous variant used by the main conductor check path.
fn load_efficiency_signals_sync(
    path: &Path,
    budget_usd: Option<f64>,
) -> std::io::Result<Vec<Engram>> {
    let text = std::fs::read_to_string(path)?;
    Ok(build_efficiency_signals(&text, budget_usd))
}

async fn load_or_create_skill_library(path: &Path) -> Result<SkillLibrary> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
    {
        Ok(mut file) => {
            file.write_all(b"[]").await?;
            file.flush().await?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(err.into()),
    }

    Ok(SkillLibrary::new(path).await?)
}

async fn load_or_create_playbook_store(path: &Path) -> Result<PlaybookStore> {
    tokio::fs::create_dir_all(path).await?;
    Ok(PlaybookStore::new(path))
}

fn apply_concluded_experiment_overrides(learning: &LearningRuntime, workdir: &Path) {
    let overrides_path = static_overrides_path(workdir);
    let winners = {
        let store = learning.experiment_store().lock();
        let winners = store.concluded_winners();
        if let Err(err) = store.apply_winners_to(&winners, &overrides_path) {
            tracing::warn!(error = %err, path = %overrides_path.display(), "failed to persist experiment winners");
        }
        winners
    };

    if winners.is_empty() {
        return;
    }

    match learning
        .cascade_router()
        .load_static_overrides(&overrides_path)
    {
        Ok(applied) => {
            tracing::info!(applied, path = %overrides_path.display(), "applied static routing overrides")
        }
        Err(err) => {
            tracing::warn!(error = %err, path = %overrides_path.display(), "failed to load static routing overrides")
        }
    }
}

/// Convert the latest efficiency entries into the signals expected by the conductor.
fn build_efficiency_signals(text: &str, budget_usd: Option<f64>) -> Vec<Engram> {
    let mut signals = Vec::new();

    if let Some(budget_usd) = budget_usd.filter(|budget| *budget > 0.0) {
        signals.extend(build_cost_overrun_signals(text, budget_usd));
    }

    if let Some(signal) = build_context_window_pressure_signal(text) {
        signals.push(signal);
    }

    signals
}

/// Sum the cost from the latest valid efficiency events in the JSONL log.
fn latest_efficiency_cost(text: &str) -> Option<f64> {
    let mut total = 0.0;
    let mut seen = 0usize;

    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<AgentEfficiencyEvent>(trimmed) {
            total += event.cost_usd;
            seen += 1;
            if seen >= EFFICIENCY_SIGNAL_TAIL {
                break;
            }
        }
    }

    (seen > 0).then_some(total)
}

fn build_cost_overrun_signals(text: &str, budget_usd: f64) -> Vec<Engram> {
    let Some(cost_usd) = latest_efficiency_cost(text) else {
        return Vec::new();
    };

    vec![
        Engram::builder(Kind::Metric)
            .body(Body::text("plan cost"))
            .tag("name", "plan_cost")
            .tag("value", format!("{cost_usd:.6}"))
            .build(),
        Engram::builder(Kind::Metric)
            .body(Body::text("plan budget"))
            .tag("name", "plan_budget")
            .tag("value", format!("{budget_usd:.6}"))
            .build(),
    ]
}

fn build_context_window_pressure_signal(text: &str) -> Option<Engram> {
    let event = latest_efficiency_event(text)?;
    let body = Body::from_json(&event).unwrap_or_else(|_| {
        Body::text(format!(
            "{} tokens used on {}",
            event.total_prompt_tokens, event.model
        ))
    });

    Some(
        Engram::builder(Kind::TokenUsage)
            .body(body)
            .tag("plan_id", event.plan_id)
            .tag("task_id", event.task_id)
            .tag("role", event.role)
            .tag("model", event.model)
            .tag("tokens_used", event.total_prompt_tokens.to_string())
            .build(),
    )
}

/// Render up to 3 prior skills into a "## Prior Experience" context section.
///
/// Follows the same pattern as `render_knowledge_context` — a markdown heading
/// followed by numbered entries with key metadata so the agent can leverage
/// successful approaches from prior tasks.
fn render_prior_experience(skills: &[Skill]) -> String {
    use std::fmt::Write as _;

    let mut content = String::from(
        "## Relevant Skills\n\nThe following skills were high-confidence matches for this task:\n",
    );
    for (idx, skill) in skills.iter().enumerate() {
        let _ = write!(
            content,
            "\n### {}. {} (confidence: {:.0}%)\n",
            idx + 1,
            skill.name,
            (skill.score.clamp(0.0, 1.0) * 100.0).round(),
        );
        let _ = writeln!(content, "Summary: {}", skill.summary);
        if skill.usage_count > 0 {
            let _ = writeln!(
                content,
                "Telemetry: {:.0}% success over {} uses",
                (skill.success_rate.clamp(0.0, 1.0) * 100.0).round(),
                skill.usage_count
            );
        }
        if skill.validated_count > 0 {
            let _ = writeln!(content, "Validated matches: {}", skill.validated_count);
        }
        if !skill.description.is_empty() {
            let _ = writeln!(content, "Description: {}", skill.description);
        }
        if !skill.files.is_empty() {
            let _ = writeln!(content, "Files: {}", skill.files.join(", "));
        }
        if !skill.pattern.is_empty() {
            let _ = writeln!(content, "Pattern:\n{}", skill.pattern);
        }
        if !skill.prompt_template.is_empty() {
            let _ = writeln!(content, "Template:\n{}", skill.prompt_template);
        }
    }
    content
}

fn select_prompt_skills(
    skill_library: &SkillLibrary,
    task_def: Option<&crate::task_parser::TaskDef>,
    task_text: &str,
    limit: usize,
) -> Vec<Skill> {
    let task_files = task_def.map(|task| task.files.clone()).unwrap_or_default();
    let mut tags = extract_task_symbols(task_text);
    if let Some(crate_name) = task_crate_name(task_def) {
        tags.push(crate_name);
    }

    let query = SkillQuery {
        tags,
        category: Some(TaskCategory::Implementation.label().to_string()),
        files_hint: task_files,
    };

    skill_library
        .select(&query, limit)
        .into_iter()
        .filter(|skill| skill.score >= 0.5)
        .filter(|skill| !skill.tags.iter().any(|tag| tag == "outcome:failure"))
        .collect()
}

fn slug_matches(lhs: &str, rhs: &str) -> bool {
    lhs == rhs
        || lhs
            .split(['/', '-'])
            .next()
            .is_some_and(|family| rhs.starts_with(family))
        || rhs
            .split(['/', '-'])
            .next()
            .is_some_and(|family| lhs.starts_with(family))
}

fn is_premium_model(slug: &str) -> bool {
    let normalized = slug.to_ascii_lowercase();
    normalized.contains("opus")
        || normalized.contains("gpt-5")
        || normalized.contains("o3")
        || normalized.contains("sonnet-max")
}

fn cascade_routing_bias_from_conductor(
    bias: &roko_conductor::RoutingBias,
) -> roko_learn::cascade_router::RoutingBias {
    roko_learn::cascade_router::RoutingBias {
        deprioritize: bias.deprioritize.clone(),
        prefer_cheaper: bias.prefer_cheaper,
        reason: bias.reason.clone(),
    }
}

fn neuro_prompt_task_category(role: AgentRole) -> TaskCategory {
    match role {
        AgentRole::Researcher | AgentRole::PrePlanner | AgentRole::Strategist => {
            TaskCategory::Research
        }
        AgentRole::Refactorer => TaskCategory::Refactor,
        AgentRole::Scribe => TaskCategory::Docs,
        AgentRole::DocVerifier
        | AgentRole::IntegrationTester
        | AgentRole::TerminalValidator
        | AgentRole::GolemLifecycleTester
        | AgentRole::RegressionDetector
        | AgentRole::CoverageTracker
        | AgentRole::CrossSystemTester
        | AgentRole::DependencyValidator
        | AgentRole::FullLoopValidator => TaskCategory::Verification,
        _ => TaskCategory::Implementation,
    }
}

fn strategy_fragment_query(
    role: AgentRole,
    task_def: Option<&crate::task_parser::TaskDef>,
    task_text: &str,
) -> String {
    let mut query_parts = vec![
        neuro_prompt_task_category(role).label().to_string(),
        task_text.trim().to_string(),
    ];
    if let Some(crate_name) = task_crate_name(task_def) {
        query_parts.push(crate_name);
    }
    query_parts.retain(|part| !part.trim().is_empty());
    query_parts.join(" ")
}

fn select_strategy_fragments(
    knowledge_store: &KnowledgeStore,
    role: AgentRole,
    task_def: Option<&crate::task_parser::TaskDef>,
    task_text: &str,
    current_model: &str,
    limit: usize,
) -> Vec<KnowledgeEntry> {
    let query = strategy_fragment_query(role, task_def, task_text);
    knowledge_store
        .query_kind(
            &query,
            KnowledgeKind::StrategyFragment,
            limit.saturating_mul(3).max(limit),
        )
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| entry.confidence > 0.0)
        .filter(|entry| entry.applies_to_model(current_model))
        .take(limit)
        .collect()
}

fn render_strategy_fragments(entries: &[KnowledgeEntry]) -> String {
    use std::fmt::Write as _;

    let mut content = String::from(
        "## Strategy Fragments\n\nThe following reusable approach fragments were distilled from repeated successful runs:\n",
    );
    for (idx, entry) in entries.iter().enumerate() {
        let confidence = entry.confidence.clamp(0.0, 1.0);
        let tags = if entry.tags.is_empty() {
            String::from("-")
        } else {
            entry.tags.join(", ")
        };
        let _ = write!(
            content,
            "\n### {}. Strategy fragment ({:.0}%)\nTags: {}\n\n{}\n",
            idx + 1,
            confidence * 100.0,
            tags,
            entry.content.trim()
        );
    }
    content
}

fn build_strategy_fragment_context(
    knowledge_store: &KnowledgeStore,
    role: AgentRole,
    task_def: Option<&crate::task_parser::TaskDef>,
    task_text: &str,
    current_model: &str,
) -> Option<String> {
    let fragments =
        select_strategy_fragments(knowledge_store, role, task_def, task_text, current_model, 3);
    if fragments.is_empty() {
        None
    } else {
        Some(render_strategy_fragments(&fragments))
    }
}

fn render_playbook_context(playbook: &Playbook) -> String {
    let mut parts = vec![
        format!("Name: {}", playbook.name),
        format!("Goal: {}", playbook.goal),
        format!(
            "Success rate: {:.0}%",
            (playbook.success_rate().unwrap_or(0.0).clamp(0.0, 1.0) * 100.0).round()
        ),
    ];

    if !playbook.steps.is_empty() {
        let mut steps = String::from("Steps:\n");
        for step in &playbook.steps {
            steps.push_str(&format!(
                "- {}. [{}] {}\n",
                step.index, step.action_kind, step.description
            ));
            if !step.expected_signals.is_empty() {
                steps.push_str(&format!(
                    "  Signals: {}\n",
                    step.expected_signals.join(", ")
                ));
            }
        }
        parts.push(steps.trim_end().to_string());
    }

    parts.join("\n\n")
}

fn latest_efficiency_event(text: &str) -> Option<AgentEfficiencyEvent> {
    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<AgentEfficiencyEvent>(trimmed) {
            return Some(event);
        }
    }

    None
}

/// Enrich a task with search context from Perplexity Sonar before dispatch.
///
/// Runs a single `sonar` search query for tasks whose tier indicates they
/// benefit from external best-practice documentation (`architectural` or
/// `integrative`). Returns `None` for simple tasks or when the search fails.
///
/// The returned string is a markdown-formatted context block with the top 3
/// search results, suitable for injection as a `PromptSection`.
async fn enrich_task_context_with_search(
    task: &crate::task_parser::TaskDef,
    pplx_client: &PerplexitySearchClient,
) -> Option<String> {
    if !task.needs_external_context() {
        return None;
    }

    let query = search_query_for_task(task);

    let results = pplx_client.search(&query).await.ok()?;

    if results.results.is_empty() {
        return None;
    }

    Some(render_search_context(
        &query,
        &results.results,
        task.files.as_slice(),
    ))
}

fn search_query_for_task(task: &crate::task_parser::TaskDef) -> String {
    let description = task.description.as_deref().unwrap_or(&task.title).trim();
    let mut query_parts = vec![
        "Rust".to_string(),
        task.tier.clone(),
        description.to_string(),
        "best practices".to_string(),
    ];

    if let Some(crate_name) = task_crate_name(Some(task)) {
        query_parts.push(crate_name);
    }

    let file_hints = task
        .files
        .iter()
        .take(2)
        .filter_map(|path| {
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(ToOwned::to_owned)
        })
        .collect::<Vec<_>>();
    if !file_hints.is_empty() {
        query_parts.push(file_hints.join(" "));
    }

    query_parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_search_context(
    query: &str,
    results: &[roko_agent::perplexity::SearchResult],
    files: &[String],
) -> String {
    use std::fmt::Write as _;

    let mut context = String::from(
        "## External Research\n\nPre-dispatch search grounding for architecture/integration work.\n",
    );
    let _ = writeln!(context, "\nQuery: {query}");
    if !files.is_empty() {
        let _ = writeln!(context, "Files: {}", files.join(", "));
    }

    for (idx, result) in results.iter().take(3).enumerate() {
        let snippet = truncate_doc_snippet(result.content.trim(), 420);
        let _ = write!(
            context,
            "\n### {}. {}\n{}\nSource: {}",
            idx + 1,
            result.title.trim(),
            snippet,
            result.url
        );
        if let Some(date) = result
            .last_updated
            .as_deref()
            .or(result.date.as_deref())
            .filter(|date| !date.trim().is_empty())
        {
            let _ = write!(context, "\nDate: {date}");
        }
        context.push('\n');
    }

    context
}

// ─── PlanRunner ───────────────────────────────────────────────────────────

/// The runtime harness that drives plan execution end-to-end.
///
/// Connects the CLI to the orchestrator, agents, and gates. Maintains
/// an event log for crash recovery and periodically auto-saves state.
pub struct PlanRunner {
    /// Working directory (repo root).
    workdir: PathBuf,
    /// CLI config for agent/gate settings.
    config: Config,
    /// CLI override to disable all re-planning.
    no_replan: bool,
    /// The executor state machine.
    executor: ParallelExecutor,
    /// Append-only event log for crash recovery.
    event_log: EventLog,
    /// Counters for reporting.
    agent_calls: usize,
    gate_runs: usize,
    /// Per-plan worktree manager.
    worktrees: WorktreeManager,
    /// Post-merge regression history and follow-up decisions.
    post_merge: PostMergeRunner,
    /// Optional Claude session resume id from upper layers.
    claude_resume_session: Option<String>,
    /// Actions dispatched since last auto-save.
    actions_since_save: usize,
    /// Per-plan tracking.
    per_plan_agents: HashMap<String, usize>,
    per_plan_gates: HashMap<String, Vec<(String, bool)>>,
    /// Episode logger for recording agent turns to `.roko/episodes.jsonl`.
    learning: LearningRuntime,
    /// Daimon affect state used to modulate future dispatches.
    daimon: DaimonState,
    /// Skill library for reusable prompt patterns and successful task recipes.
    skill_library: SkillLibrary,
    /// Playbook store for reusable successful task sequences.
    #[allow(dead_code)]
    playbook: PlaybookStore,
    /// Durable knowledge store queried per task for task-scoped context.
    knowledge_store: KnowledgeStore,
    /// Process supervisor for tracking and cleaning up agent subprocesses.
    supervisor: ProcessSupervisor,
    /// Root cancellation token for coordinated shutdown.
    cancel: CancelToken,
    /// Per-plan task tracking for granular Implementing → Gating progression.
    task_trackers: HashMap<String, TaskTracker>,
    /// Explicit Gemini context caches keyed by plan id.
    gemini_plan_caches: HashMap<String, GeminiPlanCache>,
    /// Conductor for anomaly detection between phases.
    conductor: Arc<Conductor>,
    /// Default safety policies applied around agent dispatch.
    safety_layer: SafetyLayer,
    /// Signals accumulated during the current plan run for conductor evaluation.
    conductor_signals: Vec<Engram>,
    /// Learned intervention policy for failed task dispatch / verification retries.
    retry_conductor: ConductorBandit,
    /// Context attribution tracker for per-(tier, source_type) demotion decisions.
    attribution_tracker: ContextAttributionTracker,
    /// Rolling EMA of reference rates per (task_tier, context_source_type).
    context_average_tracker: ContextAverageTracker,
    /// Per-crate familiarity tracker for LinUCB context features.
    crate_familiarity_tracker: CrateFamiliarityTracker,
    /// Cumulative USD cost per plan_id.
    plan_costs: HashMap<String, f64>,
    /// Cumulative USD cost per plan/task dispatch key.
    task_costs: HashMap<String, f64>,
    /// Session-local detector for prompt loops, cost spikes, and quality drift.
    anomaly_detector: AnomalyDetector,
    /// Pending one-shot model override applied to the next routed task dispatch.
    force_model_override: Option<String>,
    /// Metric registry for counters/histograms/gauges (prometheus-style).
    metrics: Arc<MetricRegistry>,
    /// Format-selection bandit for adaptive tool-call format per model/role.
    format_bandit: ProfileBandit,
    /// MCP server names discovered for this run.
    mcp_server_names: Vec<String>,
    /// MCP server clients plus lease counts.
    mcp_state: tokio::sync::Mutex<McpServerState>,
    /// Dynamic tool registry combining static tools with MCP-discovered tools.
    tool_registry: Option<Arc<roko_agent::mcp::DynamicToolRegistry>>,
    /// Filesystem-backed observability sinks (traces + metrics).
    obs_sinks: FsObservabilitySinks,
    /// Health probe registry for readiness/liveness checks.
    health_probes: ProbeRegistry,
    /// Adaptive gate thresholds for retry budgeting.
    adaptive_thresholds: AdaptiveThresholds,
    /// Rolling latency registry for routed model/provider pairs.
    latency_registry: LatencyRegistry,
    /// Event bus used to publish post-turn learning signals.
    learning_event_bus: LearningEventBus,
    /// In-memory efficiency events collected during this run.
    efficiency_events: Vec<AgentEfficiencyEvent>,
    /// Optional event bus sender for HTTP API event streaming.
    server_event_bus: Option<roko_runtime::event_bus::BusSender<crate::serve::events::ServerEvent>>,
    /// Optional state hub sender for unified dashboard snapshot updates.
    state_hub_sender: Option<roko_core::StateHubSender>,
    /// Optional cloud execution state for code-implementer runs.
    cloud_execution: Option<CloudExecution>,
    /// Optional Perplexity search client for pre-dispatch context enrichment.
    ///
    /// Present when `PERPLEXITY_API_KEY` is set at startup. Used to run a
    /// single `sonar` search before dispatching complex tasks so the agent
    /// receives grounded best-practice context.
    search_client: Option<PerplexitySearchClient>,
}

/// Tracks per-task completion within a plan. Lives in PlanRunner (CLI crate),
/// NOT in PlanState (orchestrator crate) — the state machine stays pure.
struct TaskTracker {
    tasks_file: TasksFile,
    completed: Vec<String>,
    failed: Vec<String>,
    skipped: Vec<String>,
    current_group_index: usize,
    /// When each ready task first entered the queue, in Unix ms.
    ready_since_ms: HashMap<String, u64>,
    _plan_dir: PathBuf,
    last_gate_failure: Option<String>,
    /// Which gate phase failed (e.g. "compile", "test", "clippy").
    last_gate_failure_phase: Option<String>,
    /// The task id that was most recently dispatched for implementation.
    last_impl_task_id: Option<String>,
    /// Model slug used by the most recently dispatched implementation task.
    last_impl_model_slug: Option<String>,
    /// Output hash from the most recent implementation dispatch.
    last_impl_output_hash: Option<ContentHash>,
    /// Knowledge entry ids surfaced in the most recent task context.
    last_context_knowledge_ids: Vec<String>,
    /// Last detailed gate verdicts emitted for this plan.
    last_gate_verdicts: Vec<GateVerdict>,
    review_feedback: Option<String>,
    impl_round: u32,
    /// Skill matched during the last dispatch (for confidence updates).
    last_matched_skill_id: Option<String>,
    /// Playbook rule matched during the last dispatch (for confidence updates).
    last_matched_rule_id: Option<String>,
    /// Prompt experiment variant assigned during the last dispatch.
    last_experiment_variant_id: Option<String>,
    /// Prompt-section composition metadata for the most recent dispatch.
    last_prompt_sections: Vec<PromptSectionMeta>,
    /// Pending skill extraction request for the most recent successful task.
    last_skill_request: Option<SkillExtractionRequest>,
    /// Number of consecutive gate failures for this plan (for re-planning, §9).
    gate_failure_count: u32,
}

#[derive(Debug, Clone)]
struct GeminiPlanCache {
    model_slug: String,
    cache_id: String,
}

/// Shared MCP server runtime state for a plan run.
#[derive(Default)]
struct McpServerState {
    /// Live client handles keyed by server name.
    clients: HashMap<String, Arc<roko_agent::mcp::McpClient<roko_agent::mcp::StdioTransport>>>,
    /// Live server configs keyed by server name.
    server_configs: HashMap<String, McpServerConfig>,
    /// Active leases keyed by server name.
    ref_counts: HashMap<String, usize>,
}

fn role_hash_features(role: &str) -> [f64; 4] {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in role.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    [
        (h & 0xFFFF) as f64 / 65535.0,
        ((h >> 16) & 0xFFFF) as f64 / 65535.0,
        ((h >> 32) & 0xFFFF) as f64 / 65535.0,
        ((h >> 48) & 0xFFFF) as f64 / 65535.0,
    ]
}

fn cascade_routing_context(
    runner: &PlanRunner,
    plan_id: &str,
    _task_id: &str,
    role: AgentRole,
    task_def: Option<&crate::task_parser::TaskDef>,
) -> roko_learn::model_router::RoutingContext {
    use roko_core::TaskCategory;
    use roko_core::TaskComplexityBand;
    use roko_learn::model_router::RoutingContext;

    let completed_plans = runner.executor.completed_plans();
    let complexity = match task_def.map(|td| td.tier.as_str()).unwrap_or("focused") {
        "mechanical" | "fast" => TaskComplexityBand::Fast,
        "architectural" | "complex" | "premium" => TaskComplexityBand::Complex,
        _ => TaskComplexityBand::Standard,
    };
    let iteration = runner
        .task_trackers
        .get(plan_id)
        .map(|tracker| tracker.impl_round.saturating_add(1))
        .unwrap_or(1);
    let has_prior_failure = runner
        .task_trackers
        .get(plan_id)
        .is_some_and(|tracker| tracker.last_gate_failure.is_some());

    let crate_familiarity = runner.crate_familiarity_tracker.score_for_task(task_def);
    let affect = runner.daimon.query();
    let (ready_queue_depth, max_queue_wait_hours) = runner
        .task_trackers
        .get(plan_id)
        .map(|tracker| {
            let ready_ids = tracker.ready_task_ids(&completed_plans);
            let max_wait = ready_ids
                .iter()
                .filter_map(|task_id| tracker.queue_wait_hours(task_id))
                .fold(0.0, f64::max);
            (u32::try_from(ready_ids.len()).unwrap_or(u32::MAX), max_wait)
        })
        .unwrap_or((0, 0.0));
    let conductor_load = routing_load_pressure(0, ready_queue_depth, max_queue_wait_hours);

    RoutingContext {
        task_category: TaskCategory::Implementation,
        complexity,
        iteration,
        role,
        crate_familiarity,
        has_prior_failure,
        conductor_load,
        active_agents: 0,
        ready_queue_depth,
        max_queue_wait_hours,
        daimon_policy: DaimonPolicy::new(affect.confidence, affect.behavioral_state),
        thinking_level: Some(runner.config.agent.effort.clone()),
        previous_model: None,
        plan_context_tokens: None,
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct RoutingLoadSnapshot {
    pressure: f64,
    active_agents: u32,
    ready_queue_depth: u32,
    max_queue_wait_hours: f64,
}

fn routing_load_pressure(
    active_agents: u32,
    ready_queue_depth: u32,
    max_queue_wait_hours: f64,
) -> f64 {
    let active_pressure = (f64::from(active_agents) / 6.0).clamp(0.0, 1.0);
    let queue_pressure = (f64::from(ready_queue_depth) / 6.0).clamp(0.0, 1.0);
    let wait_pressure = (max_queue_wait_hours / 8.0).clamp(0.0, 1.0);
    active_pressure.max(queue_pressure).max(wait_pressure)
}

fn coding_strategy_coordinates(
    runner: &PlanRunner,
    plan_id: &str,
    task_id: &str,
    task_def: Option<&crate::task_parser::TaskDef>,
) -> StrategyCoordinates {
    let affect = runner.daimon.query();
    let familiarity = runner.crate_familiarity_tracker.score_for_task(task_def);
    let tracker = runner.task_trackers.get(plan_id);
    let gate_failure_pressure = tracker
        .map(|tracker| f64::from(tracker.gate_failure_count.min(5)) / 5.0)
        .unwrap_or(0.0);
    let is_current_impl_task =
        tracker.and_then(|tracker| tracker.last_impl_task_id.as_deref()) == Some(task_id);
    let observation = TaskStrategyObservation {
        task_tier: task_def
            .map(|task| task.tier.clone())
            .unwrap_or_else(|| "focused".to_string()),
        file_count: task_def.map_or(0, |task| task.files.len()),
        verification_count: task_def.map_or(0, |task| task.verify.len()),
        dependency_count: task_def.map_or(0, |task| task.depends_on.len()),
        max_loc: task_def.and_then(|task| task.max_loc).unwrap_or(50),
        familiarity,
        confidence: affect.confidence,
        failure_pressure: gate_failure_pressure,
        urgency_pressure: f64::from(is_current_impl_task),
    };

    runner
        .daimon
        .strategy_space()
        .computer()
        .task_coords(&observation)
}

fn somatic_episode_hash(
    plan_id: &str,
    task_id: &str,
    outcome: &str,
    discriminator: &str,
) -> ContentHash {
    ContentHash::of(format!("somatic:{plan_id}:{task_id}:{outcome}:{discriminator}").as_bytes())
}

fn cascade_context_vec(
    runner: &PlanRunner,
    plan_id: &str,
    task_id: &str,
    role: AgentRole,
    task_def: Option<&crate::task_parser::TaskDef>,
) -> Vec<f64> {
    cascade_routing_context(runner, plan_id, task_id, role, task_def).to_features()
}

impl TaskTracker {
    fn new(tasks_file: TasksFile, plan_dir: PathBuf) -> Self {
        let skipped = tasks_file
            .tasks
            .iter()
            .filter(|task| task.status.eq_ignore_ascii_case("skipped"))
            .map(|task| task.id.clone())
            .collect();
        let mut tracker = Self {
            tasks_file,
            completed: Vec::new(),
            failed: Vec::new(),
            skipped,
            current_group_index: 0,
            ready_since_ms: HashMap::new(),
            _plan_dir: plan_dir,
            last_gate_failure: None,
            last_gate_failure_phase: None,
            last_impl_task_id: None,
            last_impl_model_slug: None,
            last_impl_output_hash: None,
            last_context_knowledge_ids: Vec::new(),
            last_gate_verdicts: Vec::new(),
            review_feedback: None,
            impl_round: 0,
            last_matched_skill_id: None,
            last_matched_rule_id: None,
            last_experiment_variant_id: None,
            last_prompt_sections: Vec::new(),
            last_skill_request: None,
            gate_failure_count: 0,
        };
        tracker.advance_group_index();
        tracker
    }

    /// Find the next unfinished task that has all deps satisfied.
    #[cfg(test)]
    #[allow(dead_code)]
    fn next_ready_task(&self, completed_plans: &[String]) -> Option<&crate::task_parser::TaskDef> {
        self.ready_tasks(completed_plans).into_iter().next()
    }

    /// Return the task IDs that are currently ready to execute.
    fn ready_task_ids(&self, completed_plans: &[String]) -> Vec<String> {
        self.tasks_file
            .tasks
            .iter()
            .filter(|t| {
                !self.completed.contains(&t.id)
                    && !self.failed.contains(&t.id)
                    && !self.skipped.contains(&t.id)
                    && t.is_ready_with_plan_deps(&self.completed, completed_plans)
            })
            .map(|t| t.id.clone())
            .collect()
    }

    /// Return ALL ready tasks (deps satisfied, not completed, not failed).
    fn ready_tasks(&self, completed_plans: &[String]) -> Vec<&crate::task_parser::TaskDef> {
        self.tasks_file
            .tasks
            .iter()
            .filter(|t| {
                !self.completed.contains(&t.id)
                    && !self.failed.contains(&t.id)
                    && !self.skipped.contains(&t.id)
                    && t.is_ready_with_plan_deps(&self.completed, completed_plans)
            })
            .collect()
    }

    /// Refresh ready-since timestamps for tasks that are currently queued.
    fn sync_ready_queue(&mut self, completed_plans: &[String]) {
        let now = now_ms();
        let ready_ids = self.ready_task_ids(completed_plans);
        let ready_set: HashSet<String> = ready_ids.into_iter().collect();

        self.ready_since_ms
            .retain(|task_id, _| ready_set.contains(task_id));
        for task_id in ready_set {
            self.ready_since_ms.entry(task_id).or_insert(now);
        }
    }

    /// Return how long a ready task has been waiting in the queue.
    fn queue_wait_hours(&self, task_id: &str) -> Option<f64> {
        let started_ms = self.ready_since_ms.get(task_id)?;
        let elapsed_ms = now_ms().saturating_sub(*started_ms);
        Some(elapsed_ms as f64 / 3_600_000.0)
    }

    /// Whether any unfinished task is currently blocked only by cross-plan deps.
    fn has_tasks_blocked_by_plans(&self, completed_plans: &[String]) -> bool {
        self.tasks_file.tasks.iter().any(|task| {
            !self.completed.contains(&task.id)
                && !self.failed.contains(&task.id)
                && !self.skipped.contains(&task.id)
                && task.is_ready(&self.completed)
                && !task
                    .depends_on_plan
                    .iter()
                    .all(|dep| completed_plans.contains(dep))
        })
    }

    /// Whether all tasks in the plan are completed.
    fn all_tasks_done(&self) -> bool {
        self.tasks_file
            .tasks
            .iter()
            .all(|t| self.completed.contains(&t.id) || self.skipped.contains(&t.id))
    }

    /// Mark a task as completed and advance group index if current group is fully done.
    fn mark_completed(&mut self, task_id: &str) {
        if !self.completed.contains(&task_id.to_string()) {
            self.completed.push(task_id.to_string());
        }
        self.failed.retain(|id| id != task_id);
        self.skipped.retain(|id| id != task_id);
        self.ready_since_ms.remove(task_id);
        self.advance_group_index();
    }

    /// Mark a task as skipped and advance group index if current group is fully done.
    fn mark_skipped(&mut self, task_id: &str) {
        if !self.skipped.contains(&task_id.to_string()) {
            self.skipped.push(task_id.to_string());
        }
        self.failed.retain(|id| id != task_id);
        self.ready_since_ms.remove(task_id);
        self.advance_group_index();
    }

    /// Re-load `tasks.toml` after the agent rewrites the plan in place.
    ///
    /// Completed tasks are preserved if they still exist. Failed/skipped
    /// state is cleared because the plan structure has changed.
    fn reload_tasks_file(&mut self) -> Result<()> {
        let tasks_path = self._plan_dir.join("tasks.toml");
        self.tasks_file = TasksFile::parse(&tasks_path)?;
        let task_ids: std::collections::HashSet<String> = self
            .tasks_file
            .tasks
            .iter()
            .map(|task| task.id.clone())
            .collect();
        self.failed.clear();
        self.skipped = self
            .tasks_file
            .tasks
            .iter()
            .filter(|task| task.status.eq_ignore_ascii_case("skipped"))
            .map(|task| task.id.clone())
            .collect();
        self.completed.retain(|task_id| task_ids.contains(task_id));
        self.current_group_index = 0;
        self.ready_since_ms.clear();
        self.impl_round += 1;
        self.advance_group_index();
        Ok(())
    }

    /// Count distinct tasks that are terminal due to failure or skip.
    fn terminal_task_count(&self) -> usize {
        use std::collections::HashSet;

        self.failed
            .iter()
            .chain(self.skipped.iter())
            .collect::<HashSet<_>>()
            .len()
    }

    /// Whether failed/skipped tasks exceed half of the current plan.
    fn terminal_task_ratio_exceeds_half(&self) -> bool {
        let total = self.tasks_file.tasks.len();
        total > 0 && self.terminal_task_count() * 2 > total
    }

    /// Advance the current parallel group index while the active group is terminal.
    fn advance_group_index(&mut self) {
        // Advance group index if all tasks in current group are done
        let completed = self.completed.clone();
        let skipped = self.skipped.clone();
        let groups = self.tasks_file.parallel_groups();
        while self.current_group_index < groups.len() {
            let current_group_done = groups[self.current_group_index]
                .iter()
                .all(|t| completed.contains(&t.id) || skipped.contains(&t.id));
            if current_group_done {
                self.current_group_index += 1;
            } else {
                break;
            }
        }
    }

    /// Reset for re-implementation after review rejection.
    fn reset_for_reimpl(&mut self) {
        self.completed.clear();
        self.failed.clear();
        self.skipped.clear();
        self.current_group_index = 0;
        self.ready_since_ms.clear();
        self.impl_round += 1;
    }

    /// Update a task's `model_hint` and persist the rewritten `tasks.toml`.
    fn set_task_model_hint(&mut self, task_id: &str, model_hint: Option<String>) -> Result<()> {
        let Some(task) = self
            .tasks_file
            .tasks
            .iter_mut()
            .find(|task| task.id == task_id)
        else {
            return Err(anyhow!(
                "task {task_id} not found in plan {}",
                self._plan_dir.display()
            ));
        };

        task.model_hint = model_hint;

        let tasks_path = self._plan_dir.join("tasks.toml");
        let rendered = toml::to_string_pretty(&self.tasks_file)
            .context("serialize tasks.toml after model escalation")?;
        std::fs::write(&tasks_path, rendered)
            .with_context(|| format!("write {}", tasks_path.display()))?;
        Ok(())
    }

    /// Return the most recently implemented task, if it still exists in the task file.
    fn last_impl_task(&self) -> Option<&crate::task_parser::TaskDef> {
        let task_id = self.last_impl_task_id.as_deref()?;
        self.tasks_file.tasks.iter().find(|task| task.id == task_id)
    }

    /// Return the completed task definitions still present in the tracker.
    fn completed_task_defs(&self) -> Vec<crate::task_parser::TaskDef> {
        self.tasks_file
            .tasks
            .iter()
            .filter(|task| self.completed.iter().any(|task_id| task_id == &task.id))
            .cloned()
            .collect()
    }
}

fn prioritize_ready_tasks<F>(ready: Vec<String>, mut arousal_for_task: F) -> Vec<String>
where
    F: FnMut(&str) -> f64,
{
    let ready_count = ready.len();
    let mut scored: Vec<(usize, f64, String)> = ready
        .into_iter()
        .enumerate()
        .map(|(idx, task_id)| {
            let base_priority = (ready_count.saturating_sub(idx)) as f64;
            let arousal = arousal_for_task(&task_id).clamp(-1.0, 1.0);
            let effective_priority = base_priority * (1.0 + arousal * 0.5);
            (idx, effective_priority, task_id)
        })
        .collect();

    scored.sort_by(|(idx_a, score_a, _), (idx_b, score_b, _)| {
        score_b
            .partial_cmp(score_a)
            .unwrap_or(Ordering::Equal)
            .then(idx_a.cmp(idx_b))
    });

    scored.into_iter().map(|(_, _, task_id)| task_id).collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

fn merge_completed_tasks(tracker: &mut TaskTracker, completed_tasks: &[String]) {
    for task_id in completed_tasks {
        if !tracker.completed.iter().any(|existing| existing == task_id) {
            tracker.completed.push(task_id.clone());
        }
    }

    let groups = tracker.tasks_file.parallel_groups();
    tracker.current_group_index = tracker.current_group_index.min(groups.len());
    while tracker.current_group_index < groups.len()
        && groups[tracker.current_group_index].iter().all(|task| {
            tracker
                .completed
                .iter()
                .any(|completed| completed == &task.id)
        })
    {
        tracker.current_group_index += 1;
    }
}

fn normalize_task_title(title: &str) -> String {
    title
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn task_title_similarity(a: &str, b: &str) -> f64 {
    let normalized_a = normalize_task_title(a);
    let normalized_b = normalize_task_title(b);
    if normalized_a == normalized_b {
        return 1.0;
    }
    if normalized_a.contains(&normalized_b) || normalized_b.contains(&normalized_a) {
        return 0.95;
    }

    let terms_a = significant_terms(a);
    let terms_b = significant_terms(b);
    if terms_a.is_empty() || terms_b.is_empty() {
        return 0.0;
    }

    let set_a: HashSet<&str> = terms_a.iter().map(String::as_str).collect();
    let set_b: HashSet<&str> = terms_b.iter().map(String::as_str).collect();
    let common = set_a.intersection(&set_b).count() as f64;
    let len_sum = (set_a.len() + set_b.len()) as f64;
    let dice = if len_sum == 0.0 {
        0.0
    } else {
        (2.0 * common) / len_sum
    };
    let containment = common / set_a.len().min(set_b.len()) as f64;
    dice.max(containment)
}

fn best_completed_task_match<'a>(
    title: &str,
    completed_tasks: &'a [crate::task_parser::TaskDef],
) -> Option<(&'a str, f64)> {
    let mut best: Option<(&str, f64)> = None;
    for task in completed_tasks {
        let score = task_title_similarity(title, &task.title);
        if score < 0.6 {
            continue;
        }
        match best {
            Some((_, best_score)) if score <= best_score => {}
            _ => best = Some((task.id.as_str(), score)),
        }
    }
    best
}

fn merge_regenerated_plan(
    plan_id: &str,
    old_tasks: &TasksFile,
    regenerated_tasks: TasksFile,
    completed_tasks: &[crate::task_parser::TaskDef],
) -> TasksFile {
    let mut merged_tasks = Vec::new();
    let mut preserved_completed_ids = HashSet::new();
    for task in &old_tasks.tasks {
        if completed_tasks
            .iter()
            .any(|completed| completed.id == task.id)
        {
            let mut preserved = task.clone();
            preserved.status = "done".to_string();
            preserved_completed_ids.insert(preserved.id.clone());
            merged_tasks.push(preserved);
        }
    }

    let mut skipped_task_replacements: HashMap<String, String> = HashMap::new();
    let mut seen_new_ids: HashSet<String> = HashSet::new();
    for mut task in regenerated_tasks.tasks {
        if preserved_completed_ids.contains(&task.id) {
            continue;
        }
        if !seen_new_ids.insert(task.id.clone()) {
            tracing::warn!(
                "[orchestrate] regeneration for {plan_id} emitted duplicate task id {}",
                task.id
            );
            continue;
        }

        if let Some((completed_id, score)) = best_completed_task_match(&task.title, completed_tasks)
        {
            tracing::info!(
                "[orchestrate] regeneration for {plan_id} skipped task {} because it matches completed task {} (score={score:.2})",
                task.id,
                completed_id
            );
            skipped_task_replacements.insert(task.id.clone(), completed_id.to_string());
            continue;
        }

        task.status = "ready".to_string();
        merged_tasks.push(task);
    }

    for task in &mut merged_tasks {
        let mut deduped_deps = Vec::with_capacity(task.depends_on.len());
        for dep in &task.depends_on {
            let rewritten = skipped_task_replacements
                .get(dep)
                .cloned()
                .unwrap_or_else(|| dep.clone());
            if !deduped_deps.contains(&rewritten) {
                deduped_deps.push(rewritten);
            }
        }
        task.depends_on = deduped_deps;
    }

    let mut meta = regenerated_tasks.meta;
    if meta.plan.trim().is_empty() {
        meta.plan = old_tasks.meta.plan.clone();
    }
    if meta.plan.trim().is_empty() {
        meta.plan = plan_id.to_string();
    }
    meta.iteration = old_tasks.meta.iteration.saturating_add(1);
    meta.total = merged_tasks.len() as u32;
    meta.done = merged_tasks
        .iter()
        .filter(|task| task.status.eq_ignore_ascii_case("done"))
        .count() as u32;
    meta.status = if meta.total > 0 && meta.done == meta.total {
        "done".to_string()
    } else {
        "ready".to_string()
    };

    TasksFile {
        meta,
        tasks: merged_tasks,
    }
}

impl PlanRunner {
    /// Spawn MCP server processes and build a DynamicToolRegistry from their tools.
    ///
    /// Returns `(clients, registry, server_names)` where `registry` is `None`
    /// if no MCP config was found or no servers are configured.
    async fn setup_mcp(
        config: &Config,
        workdir: &Path,
        selected_servers: Option<&HashSet<String>>,
    ) -> (
        HashMap<String, Arc<roko_agent::mcp::McpClient<roko_agent::mcp::StdioTransport>>>,
        Option<Arc<roko_agent::mcp::DynamicToolRegistry>>,
        Vec<String>,
        HashMap<String, McpServerConfig>,
    ) {
        use roko_agent::mcp::{McpClient, StdioTransport, find_mcp_config, mcp_to_tool_def};
        use roko_std::tool::StaticToolRegistry;

        // Resolve MCP config: explicit path in config, or walk-up discovery.
        let mcp_config = if let Some(ref explicit) = config.agent.mcp_config {
            match roko_agent::mcp::McpConfig::load(explicit) {
                Ok(cfg) => Some(cfg),
                Err(e) => {
                    tracing::warn!("failed to load MCP config from {}: {e}", explicit.display());
                    None
                }
            }
        } else {
            find_mcp_config(workdir).and_then(|res| match res {
                Ok((_path, cfg)) => Some(cfg),
                Err(e) => {
                    tracing::warn!("MCP config discovery error: {e}");
                    None
                }
            })
        };

        let mcp_config = match mcp_config {
            Some(cfg) if !cfg.servers.is_empty() => cfg,
            _ => return (HashMap::new(), None, Vec::new(), HashMap::new()),
        };

        let selected_servers = selected_servers.filter(|names| !names.is_empty());
        let mut clients = HashMap::new();
        let mut all_server_tools = Vec::new();
        let mut server_names = Vec::new();
        let mut running_server_configs = HashMap::new();

        for server in &mcp_config.servers {
            if selected_servers.is_some_and(|names| !names.contains(&server.name)) {
                continue;
            }
            match StdioTransport::spawn(&server.command, &server.args) {
                Ok(transport) => {
                    let client = McpClient::new(transport);
                    match tokio::time::timeout(Duration::from_secs(5), client.initialize()).await {
                        Ok(Ok(_)) => {}
                        Ok(Err(e)) => {
                            tracing::warn!("MCP server '{}' initialize failed: {e}", server.name);
                            continue;
                        }
                        Err(_) => {
                            tracing::error!(
                                "MCP server '{}' initialize timed out after 5s",
                                server.name
                            );
                            continue;
                        }
                    }
                    // List available tools
                    match client.list_tools().await {
                        Ok(tools) => {
                            server_names.push(server.name.clone());
                            tracing::info!(
                                "MCP server '{}': discovered {} tools",
                                server.name,
                                tools.len()
                            );
                            let defs: Vec<_> = tools
                                .iter()
                                .map(|t| mcp_to_tool_def(t, &server.name))
                                .collect();
                            all_server_tools.push((server.name.clone(), defs));
                            clients.insert(server.name.clone(), Arc::new(client));
                            running_server_configs.insert(server.name.clone(), server.clone());
                        }
                        Err(e) => {
                            tracing::warn!("MCP server '{}' list_tools failed: {e}", server.name);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("failed to spawn MCP server '{}': {e}", server.name);
                }
            }
        }

        if all_server_tools.is_empty() {
            return (clients, None, server_names, running_server_configs);
        }

        // Dedup across servers and build the dynamic registry.
        let deduped = roko_agent::mcp::dedup_tools(all_server_tools);
        let base = StaticToolRegistry::new();
        let mut registry =
            roko_agent::mcp::DynamicToolRegistry::with_preference(&base, config.tools.prefer_mcp);
        // Group deduped tools by their server prefix (everything before `__`).
        let mut by_server: HashMap<String, Vec<roko_core::tool::ToolDef>> = HashMap::new();
        for tool in deduped {
            let server_name = tool
                .name
                .split("__")
                .next()
                .unwrap_or("unknown")
                .to_string();
            by_server.entry(server_name).or_default().push(tool);
        }
        for (server_name, tools) in by_server {
            registry.add_mcp_tools(&server_name, tools);
        }

        (
            clients,
            Some(Arc::new(registry)),
            server_names,
            running_server_configs,
        )
    }

    /// Resolve the MCP config path for Claude.
    ///
    /// If runtime MCP servers are active, writes `.roko/mcp-config.json`
    /// from the live server list and returns that generated file. Otherwise
    /// falls back to any explicit config configured in `roko.toml`.
    async fn resolve_mcp_config_path(&self) -> Option<PathBuf> {
        let mut servers = {
            let state = self.mcp_state.lock().await;
            state.server_configs.values().cloned().collect::<Vec<_>>()
        };

        if servers.is_empty() {
            return self.config.agent.mcp_config.clone();
        }

        servers.sort_by(|a, b| a.name.cmp(&b.name));
        let config = McpConfig { servers };
        let dir = self.workdir.join(".roko");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!(
                "failed to create MCP config directory {}: {e}",
                dir.display()
            );
            return self.config.agent.mcp_config.clone();
        }

        let path = dir.join("mcp-config.json");
        let tmp_path = path.with_extension("json.tmp");
        let json = match serde_json::to_string_pretty(&config) {
            Ok(json) => json,
            Err(e) => {
                tracing::warn!("failed to serialize runtime MCP config: {e}");
                return self.config.agent.mcp_config.clone();
            }
        };

        if let Err(e) = std::fs::write(&tmp_path, json) {
            tracing::warn!(
                "failed to write runtime MCP config {}: {e}",
                tmp_path.display()
            );
            return self.config.agent.mcp_config.clone();
        }
        if let Err(e) = std::fs::rename(&tmp_path, &path) {
            let _ = std::fs::remove_file(&tmp_path);
            tracing::warn!(
                "failed to publish runtime MCP config {}: {e}",
                path.display()
            );
            return self.config.agent.mcp_config.clone();
        }

        Some(path)
    }

    /// Discover plans from a directory and build the executor.
    ///
    /// # Errors
    ///
    /// Returns an error if the plans directory doesn't exist, contains no
    /// plans, or plan discovery fails.
    pub async fn from_plans_dir(
        plans_dir: &Path,
        workdir: &Path,
        config: Config,
        metrics: Arc<MetricRegistry>,
        no_replan: bool,
    ) -> Result<Self> {
        if !plans_dir.exists() {
            return Err(anyhow!(
                "plans directory does not exist: {}",
                plans_dir.display()
            ));
        }

        let plans = discover_plans(plans_dir).map_err(|e| anyhow!("plan discovery failed: {e}"))?;

        if plans.is_empty() {
            return Err(anyhow!("no plans found in {}", plans_dir.display()));
        }

        let mut executor = ParallelExecutor::new(config.executor.clone());

        // Track cross-plan dependencies from frontmatter
        let mut plan_deps: HashMap<String, Vec<String>> = HashMap::new();

        for plan_info in &plans {
            let plan_id = plan_info
                .frontmatter
                .as_ref()
                .and_then(|fm| fm.plan.clone())
                .unwrap_or_else(|| plan_info.base.clone());

            // Read cross-plan dependencies from frontmatter
            if let Some(ref fm) = plan_info.frontmatter {
                if !fm.depends_on.is_empty() {
                    plan_deps.insert(plan_id.clone(), fm.depends_on.clone());
                    tracing::info!(
                        "[orchestrate] Plan {plan_id} depends on: {:?}",
                        fm.depends_on
                    );
                }
            }

            // Parse tasks.toml if it exists, log task count and parallel groups
            let tasks_path = plans_dir.join(&plan_info.base).join("tasks.toml");
            if tasks_path.exists() {
                let tf = TasksFile::parse(&tasks_path).map_err(|e| {
                    tracing::error!(
                        target: "plan_validation",
                        plan_id = %plan_id,
                        plan_base = %plan_info.base,
                        tasks_path = %tasks_path.display(),
                        issue = "parse_error",
                        error = %e,
                        "tasks.toml validation failed"
                    );
                    anyhow!("tasks.toml parse failed for {}: {e}", tasks_path.display())
                })?;
                validate_tasks_file_for_execution(&plan_id, &plan_info.base, &tasks_path, &tf)?;
                let groups = tf.parallel_groups();
                let model_tiers: Vec<String> = tf
                    .tasks
                    .iter()
                    .map(|t| format!("{}:{}", t.id, t.tier))
                    .collect();
                tracing::info!(
                    "[orchestrate] Plan {plan_id}: {} tasks, {} parallel groups, max_parallel={}, tiers=[{}]",
                    tf.tasks.len(),
                    groups.len(),
                    tf.meta.max_parallel,
                    model_tiers.join(", ")
                );
            }

            let state = PlanState::new(&plan_id);
            executor.add_plan(state);
        }

        // Wire cross-plan dependency ordering (§10).
        executor.set_plan_dependencies(plan_deps);

        // Pre-populate task trackers for plans with tasks.toml
        let mut task_trackers = HashMap::new();
        let mut requested_mcp_servers: HashSet<String> = HashSet::new();
        let mut any_task_without_mcp_list = false;
        for plan_info in &plans {
            let plan_id = plan_info
                .frontmatter
                .as_ref()
                .and_then(|fm| fm.plan.clone())
                .unwrap_or_else(|| plan_info.base.clone());
            let tasks_path = plans_dir.join(&plan_info.base).join("tasks.toml");
            if tasks_path.exists() {
                let tf = TasksFile::parse(&tasks_path).map_err(|e| {
                    tracing::error!(
                        target: "plan_validation",
                        plan_id = %plan_id,
                        plan_base = %plan_info.base,
                        tasks_path = %tasks_path.display(),
                        issue = "parse_error",
                        error = %e,
                        "tasks.toml validation failed"
                    );
                    anyhow!("tasks.toml parse failed for {}: {e}", tasks_path.display())
                })?;
                validate_tasks_file_for_execution(&plan_id, &plan_info.base, &tasks_path, &tf)?;
                for task in &tf.tasks {
                    match task.mcp_servers.as_ref() {
                        Some(servers) if !servers.is_empty() => {
                            requested_mcp_servers.extend(servers.iter().cloned());
                        }
                        _ => {
                            any_task_without_mcp_list = true;
                        }
                    }
                }
                let pdir = plans_dir.join(&plan_info.base);
                task_trackers.insert(plan_id, TaskTracker::new(tf, pdir));
            }
        }

        let cancel = CancelToken::new();
        let mut learning = LearningRuntime::open_under(workdir.join(".roko").join("learn"))
            .await
            .map_err(|e| anyhow!("init learning runtime: {e}"))?;
        install_episode_distillation_hook(&mut learning, workdir);
        apply_concluded_experiment_overrides(&learning, workdir);
        let mut daimon = DaimonState::load_or_new(daimon_state_path(workdir));
        daimon.configure_strategy_space(config.daimon.strategy_space.clone());
        let skill_library =
            load_or_create_skill_library(&workdir.join(".roko").join("learn").join("skills.json"))
                .await
                .context("init skill library")?;
        let playbook = load_or_create_playbook_store(
            &workdir.join(".roko").join("learn").join("playbooks.json"),
        )
        .await
        .context("init playbook store")?;
        let knowledge_store =
            KnowledgeStore::init(&workdir.join(".roko").join("neuro").join("knowledge.jsonl"))
                .context("init knowledge store")?;
        let selected_mcp_servers = if any_task_without_mcp_list || requested_mcp_servers.is_empty()
        {
            None
        } else {
            Some(requested_mcp_servers)
        };
        let (mcp_clients, tool_registry, mcp_server_names, mcp_server_configs) =
            Self::setup_mcp(&config, workdir, selected_mcp_servers.as_ref()).await;
        let obs_sinks = FsObservabilitySinks::for_workdir(workdir);
        obs_sinks
            .initialize()
            .context("initialize observability sinks")?;
        roko_core::obs::register_standard_metrics(&metrics);
        let health_probes = Self::build_health_probes(&config);
        Ok(Self {
            workdir: workdir.to_path_buf(),
            config,
            no_replan,
            executor,
            event_log: EventLog::default(),
            agent_calls: 0,
            gate_runs: 0,
            worktrees: default_worktree_manager(workdir),
            post_merge: PostMergeRunner::new(),
            claude_resume_session: None,
            actions_since_save: 0,
            per_plan_agents: HashMap::new(),
            per_plan_gates: HashMap::new(),
            learning,
            daimon,
            skill_library,
            supervisor: ProcessSupervisor::new(cancel.clone()),
            cancel,
            task_trackers,
            gemini_plan_caches: HashMap::new(),
            conductor: Arc::new(Conductor::new()),
            safety_layer: SafetyLayer::with_defaults(),
            conductor_signals: Vec::new(),
            retry_conductor: ConductorBandit::load_or_new(&conductor_policy_path(workdir)),
            attribution_tracker: ContextAttributionTracker::load(
                &workdir.join(".roko").join("context-attribution.jsonl"),
            ),
            context_average_tracker: ContextAverageTracker::load(
                workdir
                    .join(".roko")
                    .join("learn")
                    .join("context-averages.json"),
            ),
            crate_familiarity_tracker: CrateFamiliarityTracker::load(
                workdir
                    .join(".roko")
                    .join("learn")
                    .join("crate-familiarity.jsonl"),
            ),
            plan_costs: HashMap::new(),
            task_costs: HashMap::new(),
            anomaly_detector: AnomalyDetector::new(now_unix_ms_i64()),
            force_model_override: None,
            metrics,
            format_bandit: ProfileBandit::with_static_profiles(),
            mcp_server_names,
            mcp_state: tokio::sync::Mutex::new(McpServerState {
                clients: mcp_clients,
                server_configs: mcp_server_configs,
                ref_counts: HashMap::new(),
            }),
            tool_registry,
            obs_sinks,
            health_probes,
            adaptive_thresholds: AdaptiveThresholds::load_or_new(
                &workdir
                    .join(".roko")
                    .join("learn")
                    .join("gate-thresholds.json"),
            ),
            latency_registry: LatencyRegistry::load_or_new(&latency_registry_path(workdir)),
            learning_event_bus: LearningEventBus::new(256),
            efficiency_events: Vec::new(),
            server_event_bus: None,
            state_hub_sender: None,
            cloud_execution: None,
            playbook,
            knowledge_store,
            search_client: std::env::var("PERPLEXITY_API_KEY")
                .ok()
                .map(PerplexitySearchClient::new),
        })
    }

    /// Restore a runner from a snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if snapshot parsing fails.
    pub async fn from_snapshot(
        snapshot_json: &str,
        workdir: &Path,
        config: Config,
        metrics: Arc<MetricRegistry>,
        no_replan: bool,
    ) -> Result<Self> {
        let snapshot =
            ExecutorSnapshot::from_json(snapshot_json).map_err(|e| anyhow!("bad snapshot: {e}"))?;
        let executor = ParallelExecutor::from_snapshot(config.executor.clone(), snapshot);
        let legacy_completed = Self::legacy_completed_tasks_from_snapshot(snapshot_json);
        let task_trackers = Self::restore_task_trackers(workdir, &legacy_completed);
        let cancel = CancelToken::new();
        let mut learning = LearningRuntime::open_under(workdir.join(".roko").join("learn"))
            .await
            .map_err(|e| anyhow!("init learning runtime: {e}"))?;
        install_episode_distillation_hook(&mut learning, workdir);
        apply_concluded_experiment_overrides(&learning, workdir);
        let mut daimon = DaimonState::load_or_new(daimon_state_path(workdir));
        daimon.configure_strategy_space(config.daimon.strategy_space.clone());
        let skill_library =
            load_or_create_skill_library(&workdir.join(".roko").join("learn").join("skills.json"))
                .await
                .context("init skill library")?;
        let playbook = load_or_create_playbook_store(
            &workdir.join(".roko").join("learn").join("playbooks.json"),
        )
        .await
        .context("init playbook store")?;
        let knowledge_store =
            KnowledgeStore::init(&workdir.join(".roko").join("neuro").join("knowledge.jsonl"))
                .context("init knowledge store")?;
        let (mcp_clients, tool_registry, mcp_server_names, mcp_server_configs) =
            Self::setup_mcp(&config, workdir, None).await;
        let obs_sinks = FsObservabilitySinks::for_workdir(workdir);
        obs_sinks
            .initialize()
            .context("initialize observability sinks")?;
        roko_core::obs::register_standard_metrics(&metrics);
        let health_probes = Self::build_health_probes(&config);
        Ok(Self {
            workdir: workdir.to_path_buf(),
            config,
            no_replan,
            executor,
            event_log: EventLog::default(),
            agent_calls: 0,
            gate_runs: 0,
            worktrees: default_worktree_manager(workdir),
            post_merge: PostMergeRunner::new(),
            claude_resume_session: None,
            actions_since_save: 0,
            per_plan_agents: HashMap::new(),
            per_plan_gates: HashMap::new(),
            learning,
            daimon,
            skill_library,
            supervisor: ProcessSupervisor::new(cancel.clone()),
            cancel,
            task_trackers,
            gemini_plan_caches: HashMap::new(),
            conductor: Arc::new(Conductor::new()),
            safety_layer: SafetyLayer::with_defaults(),
            conductor_signals: Vec::new(),
            retry_conductor: ConductorBandit::load_or_new(&conductor_policy_path(workdir)),
            attribution_tracker: ContextAttributionTracker::load(
                &workdir.join(".roko").join("context-attribution.jsonl"),
            ),
            context_average_tracker: ContextAverageTracker::load(
                workdir
                    .join(".roko")
                    .join("learn")
                    .join("context-averages.json"),
            ),
            crate_familiarity_tracker: CrateFamiliarityTracker::load(
                workdir
                    .join(".roko")
                    .join("learn")
                    .join("crate-familiarity.jsonl"),
            ),
            plan_costs: HashMap::new(),
            task_costs: HashMap::new(),
            anomaly_detector: AnomalyDetector::new(now_unix_ms_i64()),
            force_model_override: None,
            metrics,
            format_bandit: ProfileBandit::with_static_profiles(),
            mcp_server_names,
            mcp_state: tokio::sync::Mutex::new(McpServerState {
                clients: mcp_clients,
                server_configs: mcp_server_configs,
                ref_counts: HashMap::new(),
            }),
            tool_registry,
            obs_sinks,
            health_probes,
            adaptive_thresholds: AdaptiveThresholds::load_or_new(
                &workdir
                    .join(".roko")
                    .join("learn")
                    .join("gate-thresholds.json"),
            ),
            latency_registry: LatencyRegistry::load_or_new(&latency_registry_path(workdir)),
            learning_event_bus: LearningEventBus::new(256),
            efficiency_events: Vec::new(),
            server_event_bus: None,
            state_hub_sender: None,
            cloud_execution: None,
            playbook,
            knowledge_store,
            search_client: std::env::var("PERPLEXITY_API_KEY")
                .ok()
                .map(PerplexitySearchClient::new),
        })
    }

    /// Restore a runner from both an executor snapshot and an event log snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if parsing fails.
    pub async fn from_snapshots(
        executor_json: &str,
        event_log_json: &str,
        workdir: &Path,
        config: Config,
        metrics: Arc<MetricRegistry>,
        no_replan: bool,
    ) -> Result<Self> {
        let exec_snap = ExecutorSnapshot::from_json(executor_json)
            .map_err(|e| anyhow!("bad executor snapshot: {e}"))?;
        let log_snap: EventLogSnapshot = serde_json::from_str(event_log_json)
            .map_err(|e| anyhow!("bad event log snapshot: {e}"))?;
        let executor = ParallelExecutor::from_snapshot(config.executor.clone(), exec_snap);
        let event_log = EventLog::restore(log_snap);
        let legacy_completed = Self::legacy_completed_tasks_from_snapshot(executor_json);
        let task_trackers = Self::restore_task_trackers(workdir, &legacy_completed);
        let cancel = CancelToken::new();
        let mut learning = LearningRuntime::open_under(workdir.join(".roko").join("learn"))
            .await
            .map_err(|e| anyhow!("init learning runtime: {e}"))?;
        install_episode_distillation_hook(&mut learning, workdir);
        apply_concluded_experiment_overrides(&learning, workdir);
        let mut daimon = DaimonState::load_or_new(daimon_state_path(workdir));
        daimon.configure_strategy_space(config.daimon.strategy_space.clone());
        let skill_library =
            load_or_create_skill_library(&workdir.join(".roko").join("learn").join("skills.json"))
                .await
                .context("init skill library")?;
        let playbook = load_or_create_playbook_store(
            &workdir.join(".roko").join("learn").join("playbooks.json"),
        )
        .await
        .context("init playbook store")?;
        let knowledge_store =
            KnowledgeStore::init(&workdir.join(".roko").join("neuro").join("knowledge.jsonl"))
                .context("init knowledge store")?;
        let (mcp_clients, tool_registry, mcp_server_names, mcp_server_configs) =
            Self::setup_mcp(&config, workdir, None).await;
        let obs_sinks = FsObservabilitySinks::for_workdir(workdir);
        obs_sinks
            .initialize()
            .context("initialize observability sinks")?;
        roko_core::obs::register_standard_metrics(&metrics);
        let health_probes = Self::build_health_probes(&config);
        Ok(Self {
            workdir: workdir.to_path_buf(),
            config,
            no_replan,
            executor,
            event_log,
            agent_calls: 0,
            gate_runs: 0,
            worktrees: default_worktree_manager(workdir),
            post_merge: PostMergeRunner::new(),
            claude_resume_session: None,
            actions_since_save: 0,
            per_plan_agents: HashMap::new(),
            per_plan_gates: HashMap::new(),
            learning,
            daimon,
            skill_library,
            supervisor: ProcessSupervisor::new(cancel.clone()),
            cancel,
            task_trackers,
            gemini_plan_caches: HashMap::new(),
            conductor: Arc::new(Conductor::new()),
            safety_layer: SafetyLayer::with_defaults(),
            conductor_signals: Vec::new(),
            retry_conductor: ConductorBandit::load_or_new(&conductor_policy_path(workdir)),
            attribution_tracker: ContextAttributionTracker::load(
                &workdir.join(".roko").join("context-attribution.jsonl"),
            ),
            context_average_tracker: ContextAverageTracker::load(
                workdir
                    .join(".roko")
                    .join("learn")
                    .join("context-averages.json"),
            ),
            crate_familiarity_tracker: CrateFamiliarityTracker::load(
                workdir
                    .join(".roko")
                    .join("learn")
                    .join("crate-familiarity.jsonl"),
            ),
            plan_costs: HashMap::new(),
            task_costs: HashMap::new(),
            anomaly_detector: AnomalyDetector::new(now_unix_ms_i64()),
            force_model_override: None,
            metrics,
            format_bandit: ProfileBandit::with_static_profiles(),
            mcp_server_names,
            mcp_state: tokio::sync::Mutex::new(McpServerState {
                clients: mcp_clients,
                server_configs: mcp_server_configs,
                ref_counts: HashMap::new(),
            }),
            tool_registry,
            obs_sinks,
            health_probes,
            adaptive_thresholds: AdaptiveThresholds::load_or_new(
                &workdir
                    .join(".roko")
                    .join("learn")
                    .join("gate-thresholds.json"),
            ),
            latency_registry: LatencyRegistry::load_or_new(&latency_registry_path(workdir)),
            learning_event_bus: LearningEventBus::new(256),
            efficiency_events: Vec::new(),
            server_event_bus: None,
            state_hub_sender: None,
            cloud_execution: None,
            playbook,
            knowledge_store,
            search_client: std::env::var("PERPLEXITY_API_KEY")
                .ok()
                .map(PerplexitySearchClient::new),
        })
    }

    /// Thread an optional Claude resume id from upper-layer orchestration
    /// context into per-agent launches.
    pub fn set_claude_resume_session(&mut self, session_id: Option<String>) {
        self.claude_resume_session = normalize_resume_session(session_id);
    }

    /// Attach a server event bus sender for HTTP API event streaming.
    pub fn set_server_event_bus(
        &mut self,
        bus: roko_runtime::event_bus::BusSender<crate::serve::events::ServerEvent>,
    ) {
        self.server_event_bus = Some(bus);
    }

    /// Attach a state hub sender for unified dashboard snapshot updates.
    pub fn set_state_hub(&mut self, sender: roko_core::StateHubSender) {
        self.state_hub_sender = Some(sender);
    }

    /// Enable cloud execution behavior for the current plan run.
    pub fn enable_cloud_execution(&mut self, cloud_execution: CloudExecution) {
        self.cloud_execution = Some(cloud_execution);
    }

    /// Emit a server event if a bus is attached, and publish to the state hub.
    fn emit_server_event(&self, event: crate::serve::events::ServerEvent) {
        // Publish to the unified state hub (for TUI, web, API).
        if let Some(hub) = &self.state_hub_sender {
            if let Some(dashboard_event) = server_event_to_dashboard(&event) {
                hub.publish(dashboard_event);
            }
        }
        // Also publish to the HTTP server's event bus (for WebSocket).
        if let Some(bus) = &self.server_event_bus {
            bus.emit(event);
        }
    }

    fn phase_label(kind: PhaseKind) -> &'static str {
        match kind {
            PhaseKind::Queued => "queued",
            PhaseKind::Enriching => "enriching",
            PhaseKind::Implementing => "implementing",
            PhaseKind::Gating => "gating",
            PhaseKind::Verifying => "verifying",
            PhaseKind::Reviewing => "reviewing",
            PhaseKind::DocRevision => "doc-revision",
            PhaseKind::AutoFixing => "auto-fixing",
            PhaseKind::RegeneratingVerify => "regenerating-verify",
            PhaseKind::Merging => "merging",
            PhaseKind::Complete => "complete",
            PhaseKind::Done => "done",
            PhaseKind::Failed => "failed",
            PhaseKind::Skipped => "skipped",
            _ => "unknown",
        }
    }

    fn emit_execution_event(&self, plan_id: &str, event: crate::serve::events::ExecutionEvent) {
        self.emit_server_event(crate::serve::events::ServerEvent::Execution {
            plan_id: plan_id.to_string(),
            event,
        });
    }

    fn apply_event_and_emit(
        &mut self,
        plan_id: &str,
        task_id: &str,
        event: &ExecutorEvent,
        status: &str,
    ) {
        let old_phase = self
            .executor
            .plan_state(plan_id)
            .map(|state| Self::phase_label(state.current_phase.kind()).to_string())
            .unwrap_or_else(|| "unknown".to_string());
        if let Ok(new_phase) = self.executor.apply_event(plan_id, event) {
            let new_phase_label = Self::phase_label(new_phase.kind()).to_string();
            let exec_event = if old_phase == "queued" {
                crate::serve::events::ExecutionEvent::TaskStarted {
                    task_id: task_id.to_string(),
                    phase: new_phase_label,
                }
            } else if status == "completed" {
                crate::serve::events::ExecutionEvent::TaskCompleted {
                    task_id: task_id.to_string(),
                    outcome: "completed".to_string(),
                }
            } else {
                crate::serve::events::ExecutionEvent::TaskPhaseChanged {
                    task_id: task_id.to_string(),
                    old_phase,
                    new_phase: new_phase_label,
                }
            };
            self.emit_execution_event(plan_id, exec_event);
        }
    }

    /// Apply a structured re-plan result to the live executor state.
    fn apply_replan_result(&mut self, result: &ReplanResult) {
        if !result.requires_restart() {
            return;
        }

        let plan_id = result.plan_id().to_string();
        let task_id = result.task_id().to_string();
        let old_phase = self
            .executor
            .plan_state(&plan_id)
            .map(|state| Self::phase_label(state.current_phase.kind()).to_string())
            .unwrap_or_else(|| "unknown".to_string());

        if self.executor.restart_plan(&plan_id).is_some() {
            self.emit_execution_event(
                &plan_id,
                crate::serve::events::ExecutionEvent::TaskPhaseChanged {
                    task_id,
                    old_phase,
                    new_phase: "queued".to_string(),
                },
            );
        }
    }

    /// Gracefully shut down all managed agent processes.
    pub async fn shutdown(&self) {
        let outcomes = self.supervisor.shutdown_all().await;
        if !outcomes.is_empty() {
            tracing::info!("[orchestrate] shut down {} agent processes", outcomes.len());
        }
        // Dump prometheus metrics for post-mortem debugging.
        let metrics_dir = self.workdir.join(".roko").join("metrics");
        if let Err(e) = std::fs::create_dir_all(&metrics_dir) {
            tracing::error!("[orchestrate] create metrics dir: {e}");
        } else {
            let prom = self.metrics.render_prometheus();
            if let Err(e) = std::fs::write(metrics_dir.join("prometheus.txt"), &prom) {
                tracing::error!("[orchestrate] write prometheus.txt: {e}");
            }
        }
        // Persist adaptive gate thresholds.
        let thresholds_path = self
            .workdir
            .join(".roko")
            .join("learn")
            .join("gate-thresholds.json");
        if let Err(e) = self.adaptive_thresholds.save(&thresholds_path) {
            tracing::error!("[orchestrate] save adaptive thresholds: {e}");
        }
        // Persist cascade router observations.
        if let Err(e) = self.learning.save_cascade_router() {
            tracing::error!("[orchestrate] save cascade router: {e}");
        }
        let mut mcp_state = self.mcp_state.lock().await;
        mcp_state.clients.clear();
        mcp_state.ref_counts.clear();
    }

    /// Force-kill all managed agent processes, then persist runtime artifacts.
    async fn force_shutdown(&self) {
        let killed = self.supervisor.kill_all().await;
        if !killed.is_empty() {
            tracing::warn!(
                "[orchestrate] killed {} agent processes after shutdown timeout",
                killed.len()
            );
        }
        self.shutdown().await;
    }

    /// Flush log-like runtime artifacts so resume sees the latest state.
    async fn flush_logs(&self) -> Result<()> {
        sync_file_if_present(&self.workdir.join(".roko").join("signals.jsonl"))?;
        sync_file_if_present(&self.workdir.join(".roko").join("episodes.jsonl"))?;
        sync_file_if_present(&self.workdir.join(".roko").join("logs").join("daemon.log"))?;
        sync_file_if_present(&self.workdir.join(".roko").join("logs").join("daemon.err"))?;
        std::io::stdout().flush().context("flush stdout")?;
        std::io::stderr().flush().context("flush stderr")?;
        Ok(())
    }

    /// The root cancellation token — callers can cancel to trigger shutdown.
    #[must_use]
    pub const fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }

    /// The learning runtime — exposed for status queries.
    #[must_use]
    pub const fn learning(&self) -> &LearningRuntime {
        &self.learning
    }

    /// The adaptive gate thresholds — exposed for status queries.
    #[must_use]
    pub const fn adaptive_thresholds(&self) -> &AdaptiveThresholds {
        &self.adaptive_thresholds
    }

    /// In-memory efficiency events collected during this run.
    #[must_use]
    pub fn efficiency_events(&self) -> &[AgentEfficiencyEvent] {
        &self.efficiency_events
    }

    /// The metric registry — exposed for status queries and external instrumentation.
    #[must_use]
    pub fn metrics(&self) -> &Arc<MetricRegistry> {
        &self.metrics
    }

    /// The process supervisor — exposed for status queries.
    #[must_use]
    pub const fn supervisor(&self) -> &ProcessSupervisor {
        &self.supervisor
    }

    /// The filesystem-backed observability sinks — exposed for status queries.
    #[must_use]
    pub fn obs_sinks(&self) -> &FsObservabilitySinks {
        &self.obs_sinks
    }

    /// The health probe registry — exposed for status queries and dashboard.
    #[must_use]
    pub fn health_probes(&self) -> &ProbeRegistry {
        &self.health_probes
    }

    /// Build the probe registry with real probes for configured backends.
    fn build_health_probes(config: &Config) -> ProbeRegistry {
        let registry = ProbeRegistry::new();
        registry.register(Arc::new(AlwaysUpProbe::new("orchestrator")));

        // Register a probe for the Claude CLI backend — checks if the binary exists.
        let command = config.agent.command.clone();
        registry.register(Arc::new(CliProbe { command }));

        registry
    }

    /// Pause a plan whose circuit breaker has tripped and emit the standard
    /// intervention signals.
    fn handle_tripped_circuit_breaker(&mut self, plan_id: &str) {
        tracing::error!("[conductor] pausing {plan_id}: circuit breaker tripped");
        let _ = self.executor.pause_plan(plan_id);
        let error_output = self
            .conductor
            .circuit_breaker()
            .get_record(plan_id)
            .map(|record| {
                if record.reasons.is_empty() {
                    "failure budget exhausted".to_owned()
                } else {
                    record.reasons.join("\n")
                }
            })
            .unwrap_or_else(|| "failure budget exhausted".to_owned());
        let diagnosis_engine = DiagnosisEngine::default();
        let diagnosis_results = diagnosis_engine.diagnose(&error_output);
        let primary_diagnosis = diagnosis_results.first().cloned();
        let payload = serde_json::json!({
            "plan_id": plan_id,
            "action": "pause",
            "watcher": "circuit-breaker",
            "reason": "failure budget exhausted",
            "error_output": error_output,
            "primary_diagnosis": primary_diagnosis,
            "diagnosis_results": diagnosis_results,
        });
        self.event_log
            .append(EventKind::InterventionFired, payload.clone());
        self.emit_conductor_signal(Kind::Custom("conductor.circuit_breaker".into()), payload);
        self.emit_execution_event(
            plan_id,
            crate::serve::events::ExecutionEvent::WatcherAlert {
                watcher: "circuit-breaker".to_string(),
                message: "failure budget exhausted".to_string(),
            },
        );
    }

    /// Refuse to dispatch work for a plan whose circuit breaker has tripped.
    fn ensure_dispatch_allowed(&mut self, plan_id: &str) -> Result<()> {
        if self.conductor.circuit_breaker().is_broken(plan_id) {
            self.handle_tripped_circuit_breaker(plan_id);
            return Err(anyhow!(
                "plan {plan_id} refused before dispatch: circuit breaker tripped"
            ));
        }
        Ok(())
    }

    /// Run conductor watchers against accumulated signals.
    /// Returns the decision and logs non-continue outcomes.
    fn run_conductor_check(&mut self, plan_id: &str) -> ConductorDecision {
        if self.conductor.circuit_breaker().is_broken(plan_id) {
            self.handle_tripped_circuit_breaker(plan_id);
            return ConductorDecision::Continue;
        }

        let ctx = Context::now();
        let mut signals = self.conductor_signals.clone();
        if let Ok(efficiency_signals) = load_efficiency_signals_sync(
            &self.learning.paths().efficiency_jsonl,
            self.executor.config().budget_usd,
        ) {
            signals.extend(efficiency_signals);
        }
        let decision = self.conductor.evaluate(&signals, &ctx);
        match &decision {
            ConductorDecision::Continue => {}
            ConductorDecision::Restart { watcher, reason } => {
                tracing::info!("[conductor] {plan_id}: RESTART ({watcher}) — {reason}");
                self.record_conductor_negative_feedback(plan_id, &decision);
                self.emit_execution_event(
                    plan_id,
                    crate::serve::events::ExecutionEvent::WatcherAlert {
                        watcher: watcher.clone(),
                        message: reason.to_string(),
                    },
                );
            }
            ConductorDecision::Fail { watcher, reason } => {
                tracing::error!("[conductor] {plan_id}: FAIL ({watcher}) — {reason}");
                self.record_conductor_negative_feedback(plan_id, &decision);
                self.emit_execution_event(
                    plan_id,
                    crate::serve::events::ExecutionEvent::WatcherAlert {
                        watcher: watcher.clone(),
                        message: reason.to_string(),
                    },
                );
            }
            _ => {}
        }
        decision
    }

    fn worktrees_enabled(&self) -> bool {
        self.executor.config().use_worktrees && self.cloud_execution.is_none()
    }

    async fn routing_load_snapshot(&self) -> RoutingLoadSnapshot {
        let active_agents = u32::try_from(self.supervisor.count().await).unwrap_or(u32::MAX);
        let completed_plans = self.executor.completed_plans();
        let mut ready_queue_depth = 0_u32;
        let mut max_queue_wait_hours = 0.0;

        for tracker in self.task_trackers.values() {
            let ready_ids = tracker.ready_task_ids(&completed_plans);
            ready_queue_depth = ready_queue_depth
                .saturating_add(u32::try_from(ready_ids.len()).unwrap_or(u32::MAX));
            let plan_max_wait = ready_ids
                .iter()
                .filter_map(|task_id| tracker.queue_wait_hours(task_id))
                .fold(0.0, f64::max);
            if plan_max_wait > max_queue_wait_hours {
                max_queue_wait_hours = plan_max_wait;
            }
        }

        RoutingLoadSnapshot {
            pressure: routing_load_pressure(active_agents, ready_queue_depth, max_queue_wait_hours),
            active_agents,
            ready_queue_depth,
            max_queue_wait_hours,
        }
    }

    fn heartbeat_counts(&self, completed_plans: &[String]) -> HeartbeatCounts {
        let mut total_tasks = 0usize;
        let mut completed_tasks = 0usize;
        let mut failed_tasks = 0usize;
        let mut ready_tasks = 0usize;
        let mut max_queue_wait_hours = 0.0f64;
        let mut cross_plan_blocked = false;

        for tracker in self.task_trackers.values() {
            total_tasks += tracker.tasks_file.tasks.len();
            completed_tasks += tracker.completed.len();
            failed_tasks += tracker.failed.len();
            let ready = tracker.ready_tasks(completed_plans);
            ready_tasks += ready.len();
            for task in ready {
                if let Some(wait_hours) = tracker.queue_wait_hours(&task.id) {
                    max_queue_wait_hours = max_queue_wait_hours.max(wait_hours);
                }
            }
            cross_plan_blocked |= tracker.has_tasks_blocked_by_plans(completed_plans);
        }

        let active_tasks = total_tasks.saturating_sub(completed_tasks + failed_tasks);
        let completion_rate = if total_tasks == 0 {
            0.0
        } else {
            completed_tasks as f64 / total_tasks as f64
        };

        HeartbeatCounts {
            active_tasks,
            ready_tasks,
            completed_tasks,
            failed_tasks,
            completion_rate,
            max_queue_wait_hours,
            cross_plan_blocked,
        }
    }

    fn heartbeat_probe_results(
        &self,
        completed_plans: &[String],
        counts: HeartbeatCounts,
        active_agents: usize,
        watcher_cancel: &TokioCancellationToken,
        theta_due: bool,
        delta_due: bool,
    ) -> Vec<HeartbeatProbeResult> {
        let (readiness, degraded_reasons) = self.health_probes.readiness();
        let health_degraded =
            matches!(readiness, roko_core::obs::health::ReadinessStatus::NotReady);
        let recent_gate_failure = self
            .task_trackers
            .values()
            .any(|tracker| tracker.last_gate_failure.is_some());
        let repeated_gate_failures = self
            .task_trackers
            .values()
            .any(|tracker| tracker.gate_failure_count >= 2);
        let total_spend = self.plan_costs.values().sum::<f64>();
        let affect_confidence = self.daimon.query().confidence;
        let low_affect_confidence = affect_confidence < 0.35;
        let mcp_unavailable = !self.mcp_server_names.is_empty()
            && self
                .tool_registry
                .as_ref()
                .is_none_or(|registry| registry.all().is_empty());

        vec![
            HeartbeatProbeResult::new(
                HeartbeatProbeKind::ShutdownRequested,
                self.cancel.is_cancelled(),
                self.cancel
                    .is_cancelled()
                    .then(|| "root cancel token tripped".to_string()),
            ),
            HeartbeatProbeResult::new(
                HeartbeatProbeKind::WatcherCancelled,
                watcher_cancel.is_cancelled(),
                watcher_cancel
                    .is_cancelled()
                    .then(|| "watcher task cancellation observed".to_string()),
            ),
            HeartbeatProbeResult::new(
                HeartbeatProbeKind::NoReadyTasks,
                counts.ready_tasks == 0,
                (counts.ready_tasks == 0).then(|| "no tasks are ready to dispatch".to_string()),
            ),
            HeartbeatProbeResult::new(
                HeartbeatProbeKind::ReadyQueueStalled,
                counts.max_queue_wait_hours >= 0.25,
                (counts.max_queue_wait_hours >= 0.25)
                    .then(|| format!("max queued task wait {:.2}h", counts.max_queue_wait_hours)),
            ),
            HeartbeatProbeResult::new(
                HeartbeatProbeKind::CrossPlanBlocked,
                counts.cross_plan_blocked,
                counts.cross_plan_blocked.then(|| {
                    format!(
                        "{} plan(s) waiting on cross-plan dependencies",
                        self.task_trackers
                            .values()
                            .filter(|tracker| tracker.has_tasks_blocked_by_plans(completed_plans))
                            .count()
                    )
                }),
            ),
            HeartbeatProbeResult::new(
                HeartbeatProbeKind::GateFailurePresent,
                recent_gate_failure,
                recent_gate_failure.then(|| "recent gate failure recorded".to_string()),
            ),
            HeartbeatProbeResult::new(
                HeartbeatProbeKind::RepeatedGateFailures,
                repeated_gate_failures,
                repeated_gate_failures.then(|| "repeated gate failures detected".to_string()),
            ),
            HeartbeatProbeResult::new(
                HeartbeatProbeKind::ForceModelOverrideArmed,
                self.force_model_override.is_some(),
                self.force_model_override
                    .as_ref()
                    .map(|model| format!("pending override: {model}")),
            ),
            HeartbeatProbeResult::new(
                HeartbeatProbeKind::LowAffectConfidence,
                low_affect_confidence,
                low_affect_confidence
                    .then(|| format!("daimon confidence {:.2}", affect_confidence)),
            ),
            HeartbeatProbeResult::new(
                HeartbeatProbeKind::ActiveAgentsPresent,
                active_agents > 0,
                (active_agents > 0).then(|| format!("{active_agents} active agent(s)")),
            ),
            HeartbeatProbeResult::new(
                HeartbeatProbeKind::HealthDegraded,
                health_degraded,
                health_degraded.then(|| {
                    degraded_reasons
                        .iter()
                        .map(std::string::ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; ")
                }),
            ),
            HeartbeatProbeResult::new(
                HeartbeatProbeKind::SearchUnavailable,
                self.search_client.is_none(),
                self.search_client
                    .is_none()
                    .then(|| "PERPLEXITY_API_KEY not configured".to_string()),
            ),
            HeartbeatProbeResult::new(
                HeartbeatProbeKind::McpUnavailable,
                mcp_unavailable,
                mcp_unavailable
                    .then(|| "MCP servers requested but no tools are active".to_string()),
            ),
            HeartbeatProbeResult::new(
                HeartbeatProbeKind::SessionSpendElevated,
                total_spend >= 1.0,
                (total_spend >= 1.0).then(|| format!("session spend ${total_spend:.2}")),
            ),
            HeartbeatProbeResult::new(
                HeartbeatProbeKind::ThetaDue,
                theta_due,
                theta_due.then(|| "adaptive theta cadence elapsed".to_string()),
            ),
            HeartbeatProbeResult::new(
                HeartbeatProbeKind::DeltaDue,
                delta_due,
                delta_due.then(|| "delta consolidation cadence elapsed while idle".to_string()),
            ),
        ]
    }

    async fn maybe_run_heartbeat(
        &mut self,
        heartbeat_clock: &mut HeartbeatClock,
        watcher_cancel: &TokioCancellationToken,
    ) {
        let completed_plans = self.executor.completed_plans();
        let counts = self.heartbeat_counts(&completed_plans);
        let now = std::time::Instant::now();
        let active_agents = self.supervisor.count().await;
        let affect = self.daimon.query();
        let context = OperatingFrequencyScheduleContext::from_affect(
            Duration::from_secs(heartbeat_clock.seconds_since_last_theta(now)),
            counts.active_tasks,
            counts.completion_rate,
            &affect,
        );
        let theta_due = heartbeat_clock.theta_due(now, context);
        let delta_due = heartbeat_clock.delta_due(now, context);
        let Some(frequency) = heartbeat_clock.next_due(now, context) else {
            return;
        };

        let snapshot = HeartbeatSnapshot {
            timestamp: chrono::Utc::now(),
            frequency,
            active_tasks: counts.active_tasks,
            ready_tasks: counts.ready_tasks,
            completed_tasks: counts.completed_tasks,
            failed_tasks: counts.failed_tasks,
            completion_rate: counts.completion_rate,
            active_agents,
            seconds_since_last_theta: heartbeat_clock.seconds_since_last_theta(now),
            delta_interval_secs: heartbeat_clock.delta_interval_secs(),
            probes: self.heartbeat_probe_results(
                &completed_plans,
                counts,
                active_agents,
                watcher_cancel,
                theta_due,
                delta_due,
            ),
        };

        if let Err(err) = persist_heartbeat_snapshot(&self.workdir, &snapshot) {
            tracing::warn!(error = %err, "failed to persist heartbeat snapshot");
        }

        let triggered = snapshot.triggered_probe_labels();
        self.emit_conductor_signal(
            Kind::Custom(format!("heartbeat.{}", frequency_label(frequency)).into()),
            serde_json::to_value(&snapshot)
                .unwrap_or_else(|_| serde_json::json!({"frequency": frequency_label(frequency)})),
        );
        tracing::info!(
            frequency = frequency_label(frequency),
            active_tasks = snapshot.active_tasks,
            ready_tasks = snapshot.ready_tasks,
            completion_rate = snapshot.completion_rate,
            triggered = ?triggered,
            "heartbeat snapshot recorded"
        );
        heartbeat_clock.record(now, frequency);

        if frequency == OperatingFrequency::Delta {
            self.maybe_auto_dream().await;
        }
    }

    fn record_conductor_negative_feedback(&self, plan_id: &str, intervention: &ConductorDecision) {
        let Some((task_id, model_slug, task_def)) =
            self.task_trackers.get(plan_id).and_then(|tracker| {
                Some((
                    tracker.last_impl_task_id.clone()?,
                    tracker.last_impl_model_slug.clone()?,
                    tracker.last_impl_task().cloned(),
                ))
            })
        else {
            return;
        };

        let routing_context = cascade_routing_context(
            self,
            plan_id,
            &task_id,
            AgentRole::Implementer,
            task_def.as_ref(),
        );
        if self
            .learning
            .record_conductor_intervention(&routing_context, &model_slug, intervention)
        {
            tracing::info!(
                plan_id = %plan_id,
                task_id = %task_id,
                model = %model_slug,
                decision = intervention.label(),
                "recorded conductor intervention as negative routing feedback"
            );
        }
    }

    /// Push a conductor signal so watchers can detect anomalies (§7).
    fn emit_conductor_signal(&mut self, kind: Kind, body: serde_json::Value) {
        let sig = maybe_attest_engram(
            Engram::builder(kind)
                .body(Body::Json(body))
                .emotional_tag(self.daimon.emotional_tag("conductor"))
                .build(),
        );
        self.conductor_signals.push(sig);
    }

    /// Take a snapshot of the current executor state.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn snapshot(&self) -> Result<String> {
        #[allow(clippy::cast_possible_truncation)]
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        let snap = self.executor.snapshot(ts);
        snap.to_json().map_err(|e| anyhow!("snapshot: {e}"))
    }

    /// Take a snapshot of the event log.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn event_log_snapshot(&self) -> Result<String> {
        let snap = self.event_log.snapshot();
        serde_json::to_string_pretty(&snap).map_err(|e| anyhow!("event log: {e}"))
    }

    /// Persist both executor and event log snapshots to `.roko/state/`.
    ///
    /// Uses atomic write (write to temp + rename) for safety.
    ///
    /// # Errors
    ///
    /// Returns an error if the state directory cannot be created or the
    /// files cannot be written.
    pub fn save_state(&self) -> Result<()> {
        let state_dir = state_dir(&self.workdir);
        std::fs::create_dir_all(&state_dir).map_err(|e| anyhow!("create state dir: {e}"))?;

        // Executor snapshot — atomic write.
        let exec_path = state_dir.join("executor.json");
        #[allow(clippy::cast_possible_truncation)]
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        let exec_snapshot = self.executor.snapshot(ts);
        save_snapshot_atomic(&exec_snapshot, &exec_path)?;

        // Event log snapshot — atomic write.
        let log_json = self.event_log_snapshot()?;
        let log_path = state_dir.join("events.json");
        let log_tmp = state_dir.join("events.json.tmp");
        std::fs::write(&log_tmp, &log_json).map_err(|e| anyhow!("write events tmp: {e}"))?;
        std::fs::rename(&log_tmp, &log_path).map_err(|e| anyhow!("rename events snapshot: {e}"))?;

        // Task tracker snapshot — atomic write.
        let tracker_json = self.task_tracker_snapshot()?;
        let tracker_path = state_dir.join("task-trackers.json");
        let tracker_tmp = state_dir.join("task-trackers.json.tmp");
        std::fs::write(&tracker_tmp, &tracker_json)
            .map_err(|e| anyhow!("write tracker tmp: {e}"))?;
        std::fs::rename(&tracker_tmp, &tracker_path)
            .map_err(|e| anyhow!("rename tracker snapshot: {e}"))?;

        let daimon_path = daimon_state_path(&self.workdir);
        if let Err(e) = self.daimon.persist(&daimon_path) {
            tracing::warn!(
                "[orchestrate] failed to persist daimon state to {}: {e}",
                daimon_path.display()
            );
        }

        Ok(())
    }

    /// Persist the current state and ensure the executor snapshot is present at
    /// `snapshot_path`, even when the caller wants a non-default location.
    fn save_state_to(&self, snapshot_path: &Path) -> Result<()> {
        self.save_state()?;

        let default_snapshot = executor_snapshot_path(&self.workdir);
        if snapshot_path == default_snapshot {
            return Ok(());
        }

        #[allow(clippy::cast_possible_truncation)]
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        let snapshot = self.executor.snapshot(ts);
        save_snapshot_atomic(&snapshot, snapshot_path)?;
        Ok(())
    }

    fn current_report(&self) -> OrchestrationReport {
        let snapshot = self.executor.snapshot(0);
        let mut plan_ids = snapshot.queue_order;
        for plan_id in snapshot.plan_states.keys() {
            if !plan_ids.iter().any(|queued| queued == plan_id) {
                plan_ids.push(plan_id.clone());
            }
        }

        let plans = plan_ids
            .iter()
            .map(|id| {
                let state = self.executor.plan_state(id);
                let succeeded = state.is_some_and(|s| {
                    matches!(
                        s.current_phase.kind(),
                        PhaseKind::Complete | PhaseKind::Done
                    )
                });
                PlanRunReport {
                    plan_id: id.clone(),
                    succeeded,
                    agent_calls: self.per_plan_agents.get(id).copied().unwrap_or(0),
                    gate_results: self.per_plan_gates.get(id).cloned().unwrap_or_default(),
                }
            })
            .collect();

        let fleet_cfactor = compute_fleet_cfactor(&self.efficiency_events);
        let fleet_cfactor = if fleet_cfactor.plan_count > 0 {
            Some(fleet_cfactor)
        } else {
            None
        };

        OrchestrationReport {
            total_agent_calls: self.agent_calls,
            total_gate_runs: self.gate_runs,
            plans,
            fleet_cfactor,
        }
    }

    /// Returns a reference to the inner executor (for status queries).
    #[must_use]
    pub const fn executor(&self) -> &ParallelExecutor {
        &self.executor
    }

    /// Serialize task tracker state for persistence.
    fn task_tracker_snapshot(&self) -> Result<String> {
        let entries: Vec<serde_json::Value> = self
            .task_trackers
            .iter()
            .map(|(plan_id, tracker)| {
                serde_json::json!({
                    "plan_id": plan_id,
                    "completed": tracker.completed,
                    "failed": tracker.failed,
                    "skipped": tracker.skipped,
                    "current_group_index": tracker.current_group_index,
                    "impl_round": tracker.impl_round,
                    "ready_since_ms": tracker.ready_since_ms,
                })
            })
            .collect();
        serde_json::to_string_pretty(&entries).map_err(|e| anyhow!("tracker snapshot: {e}"))
    }

    /// Restore task trackers from `.roko/state/task-trackers.json` + plan dirs.
    fn restore_task_trackers(
        workdir: &Path,
        completed_from_snapshot: &HashMap<String, Vec<String>>,
    ) -> HashMap<String, TaskTracker> {
        let tracker_path = workdir
            .join(".roko")
            .join("state")
            .join("task-trackers.json");
        let snap: Vec<serde_json::Value> = std::fs::read_to_string(&tracker_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        let mut trackers = HashMap::new();
        for entry in snap {
            let plan_id = entry["plan_id"].as_str().unwrap_or_default().to_string();
            if plan_id.is_empty() {
                continue;
            }
            let plan_dir = plans_dir(workdir).join(&plan_id);
            let tasks_path = plan_dir.join("tasks.toml");
            let Ok(tf) = TasksFile::parse(&tasks_path) else {
                continue;
            };

            let completed: Vec<String> = entry["completed"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let failed: Vec<String> = entry["failed"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let skipped: Vec<String> = entry["skipped"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let current_group_index = entry["current_group_index"].as_u64().unwrap_or(0) as usize;
            let impl_round = entry["impl_round"].as_u64().unwrap_or(0) as u32;
            let ready_since_ms = entry
                .get("ready_since_ms")
                .and_then(|value| value.as_object())
                .map(|map| {
                    map.iter()
                        .filter_map(|(task_id, started_ms)| {
                            started_ms.as_u64().map(|ms| (task_id.clone(), ms))
                        })
                        .collect::<HashMap<String, u64>>()
                })
                .unwrap_or_default();

            let mut tracker = TaskTracker::new(tf, plan_dir);
            tracker.completed = completed;
            tracker.failed = failed;
            tracker.skipped = skipped;
            tracker.current_group_index = current_group_index;
            tracker.impl_round = impl_round;
            tracker.ready_since_ms = ready_since_ms;
            if let Some(extra_completed) = completed_from_snapshot.get(&plan_id) {
                merge_completed_tasks(&mut tracker, extra_completed);
            }
            trackers.insert(plan_id, tracker);
        }

        for (plan_id, extra_completed) in completed_from_snapshot {
            if trackers.contains_key(plan_id) {
                continue;
            }
            let plan_dir = plans_dir(workdir).join(plan_id);
            let tasks_path = plan_dir.join("tasks.toml");
            let Ok(tf) = TasksFile::parse(&tasks_path) else {
                continue;
            };
            let mut tracker = TaskTracker::new(tf, plan_dir);
            merge_completed_tasks(&mut tracker, extra_completed);
            trackers.insert(plan_id.clone(), tracker);
        }

        trackers
    }

    fn plan_crate_root(&self, plan_id: &str) -> Option<PathBuf> {
        let tracker = self.task_trackers.get(plan_id)?;
        let mut roots = tracker
            .tasks_file
            .tasks
            .iter()
            .flat_map(|task| task.files.iter())
            .filter_map(|path| crate_root_for_path(path))
            .collect::<Vec<_>>();
        roots.sort();
        roots.dedup();
        (roots.len() == 1).then(|| self.workdir.join(&roots[0]))
    }

    async fn ensure_plan_gemini_cache(
        &mut self,
        plan_id: &str,
        roko_config: &RokoConfig,
        model_key: &str,
    ) -> Result<Option<String>> {
        if !roko_config.gemini.enable_context_caching {
            return Ok(None);
        }

        let resolved = resolve_model(roko_config, model_key);
        if resolved.provider_kind != ProviderKind::GeminiApi {
            return Ok(None);
        }

        let Some(model) = resolved
            .profile
            .or_else(|| roko_config.effective_models().get(model_key).cloned())
        else {
            return Ok(None);
        };

        if !model.supports_caching {
            return Ok(None);
        }

        if let Some(cache) = self.gemini_plan_caches.get(plan_id) {
            if cache.model_slug == model.slug {
                return Ok(Some(cache.cache_id.clone()));
            }
            return Ok(None);
        }

        let Some(provider) = resolved.provider_config.or_else(|| {
            roko_config
                .effective_providers()
                .get(&model.provider)
                .cloned()
        }) else {
            return Ok(None);
        };

        let Some(crate_root) = self.plan_crate_root(plan_id) else {
            tracing::debug!(
                "[orchestrate] skipped Gemini context caching for {plan_id}: no single crate root"
            );
            return Ok(None);
        };

        let crate_source = match read_full_crate_source(&crate_root) {
            Ok(source) if !source.trim().is_empty() => source,
            Ok(_) => return Ok(None),
            Err(error) => {
                tracing::warn!(
                    "[orchestrate] failed to read crate source for Gemini cache {}: {error}",
                    crate_root.display()
                );
                return Ok(None);
            }
        };

        let Some(api_key) = provider.resolve_api_key() else {
            return Ok(None);
        };
        let base_url = provider
            .base_url
            .unwrap_or_else(|| "https://generativelanguage.googleapis.com".to_string());
        let timeout_ms = provider.timeout_ms.unwrap_or(120_000);
        let cache_client = GeminiCacheClient::new(api_key, base_url).with_timeout_ms(timeout_ms);

        match cache_client
            .create_cache(
                &model.slug,
                &[Content {
                    role: "user".to_string(),
                    parts: vec![Part::Text { text: crate_source }],
                }],
                3_600,
            )
            .await
        {
            Ok(cache_id) => {
                tracing::info!(
                    "[orchestrate] created Gemini context cache for {plan_id} using {}",
                    model.slug
                );
                self.gemini_plan_caches.insert(
                    plan_id.to_string(),
                    GeminiPlanCache {
                        model_slug: model.slug,
                        cache_id: cache_id.clone(),
                    },
                );
                Ok(Some(cache_id))
            }
            Err(error) => {
                tracing::warn!(
                    "[orchestrate] failed to create Gemini context cache for {plan_id}: {error}"
                );
                Ok(None)
            }
        }
    }

    /// Extract completed task ids from legacy resume snapshots.
    ///
    /// Older `executor.json` files stored per-task records under `tasks`
    /// with a `status` field. Resume should preserve those completions so
    /// we do not rerun work that was already marked done/complete.
    fn legacy_completed_tasks_from_snapshot(snapshot_json: &str) -> HashMap<String, Vec<String>> {
        let mut completed: HashMap<String, Vec<String>> = HashMap::new();
        let Ok(value) = serde_json::from_str::<serde_json::Value>(snapshot_json) else {
            return completed;
        };

        let Some(tasks) = value.get("tasks").and_then(|tasks| tasks.as_array()) else {
            return completed;
        };

        for task in tasks {
            let status = task
                .get("status")
                .and_then(|status| status.as_str())
                .map(|status| status.to_ascii_lowercase())
                .unwrap_or_default();
            if !matches!(status.as_str(), "done" | "complete" | "completed") {
                continue;
            }

            let plan_id = task
                .get("plan")
                .or_else(|| task.get("plan_id"))
                .and_then(|plan| plan.as_str())
                .unwrap_or_default();
            let task_id = task
                .get("id")
                .or_else(|| task.get("task_id"))
                .and_then(|id| id.as_str())
                .unwrap_or_default();

            if plan_id.is_empty() || task_id.is_empty() {
                continue;
            }

            let entry = completed.entry(plan_id.to_string()).or_default();
            if !entry.iter().any(|existing| existing == task_id) {
                entry.push(task_id.to_string());
            }
        }

        completed
    }

    /// Run all plans to completion (or failure).
    ///
    /// This is the main orchestration loop. It calls `tick()` on the
    /// executor, dispatches the returned actions, feeds results back as
    /// events, and repeats until all plans are terminal.
    ///
    /// # Errors
    ///
    /// Returns an error if agent dispatch, gate execution, or substrate
    /// I/O fails fatally (per-plan failures are recorded in the report).
    pub async fn run_all(
        &mut self,
        watcher_cancel: &TokioCancellationToken,
    ) -> Result<OrchestrationReport> {
        if self.worktrees_enabled() {
            self.clear_stale_worktree_locks().await;
            // Clean up stale worktrees from previous runs (§6).
            if let Err(e) = self.worktrees.prune().await {
                tracing::error!("[orchestrate] worktree prune failed: {e}");
            }
            if let Err(e) = self.worktrees.reclaim_idle().await {
                tracing::error!("[orchestrate] worktree reclaim failed: {e}");
            }
        }

        // Start plans whose cross-plan dependencies are already satisfied (§10).
        // Plans with unsatisfied deps will be started once their deps complete.
        let plan_ids: Vec<String> = self
            .executor
            .snapshot(0)
            .plan_states
            .keys()
            .cloned()
            .collect();
        for plan_id in &plan_ids {
            let Some(state) = self.executor.plan_state(plan_id) else {
                continue;
            };
            if state.current_phase.kind() == PhaseKind::Queued
                && self.executor.can_dispatch(plan_id)
            {
                self.apply_event_and_emit(plan_id, "plan", &ExecutorEvent::Start, "transitioned");
                self.emit_execution_event(
                    plan_id,
                    crate::serve::events::ExecutionEvent::PlanStarted,
                );
                self.emit_server_event(crate::serve::events::ServerEvent::PlanStarted {
                    plan_id: plan_id.clone(),
                });
            }
        }

        // Maximum iterations to prevent infinite loops.
        let max_iterations = 1000;
        let mut iteration = 0;
        let mut heartbeat_clock = HeartbeatClock::new();

        loop {
            iteration += 1;
            if iteration > max_iterations {
                tracing::error!("[orchestrate] hit max iterations ({max_iterations}), stopping");
                break;
            }

            if self.cancel.is_cancelled() {
                tracing::warn!("[orchestrate] shutdown requested; stopping new dispatches");
                break;
            }

            let completed_plans = self.executor.completed_plans();
            for plan_id in &plan_ids {
                let Some(state) = self.executor.plan_state(plan_id) else {
                    continue;
                };
                if state.paused && state.current_phase.kind() == PhaseKind::Implementing {
                    if self
                        .task_trackers
                        .get(plan_id)
                        .is_some_and(|tracker| !tracker.ready_tasks(&completed_plans).is_empty())
                    {
                        self.executor.resume_plan(plan_id);
                    }
                }
            }

            let actions = self.executor.tick();

            if actions.is_empty() {
                self.maybe_run_heartbeat(&mut heartbeat_clock, watcher_cancel)
                    .await;
                if self.all_terminal(&plan_ids) {
                    break;
                }
                // No actions but not all terminal — wait and retry.
                tokio::select! {
                    _ = watcher_cancel.cancelled() => break,
                    _ = self.cancel.cancelled() => break,
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {}
                }
                continue;
            }

            for action in actions {
                if self.cancel.is_cancelled() {
                    tracing::warn!(
                        "[orchestrate] shutdown requested; leaving remaining actions queued"
                    );
                    break;
                }
                self.dispatch_action(action).await;
            }

            self.maybe_run_heartbeat(&mut heartbeat_clock, watcher_cancel)
                .await;

            // Auto-save periodically.
            if self.actions_since_save >= AUTOSAVE_INTERVAL {
                if let Err(e) = self.save_state() {
                    tracing::error!("[orchestrate] auto-save failed: {e}");
                }
                self.actions_since_save = 0;
            }
        }

        if self.cancel.is_cancelled() || watcher_cancel.is_cancelled() {
            self.cleanup_tracked_plan_worktrees().await;
        }

        // Clean up worktrees after completion (§6).
        if self.worktrees_enabled()
            && let Err(e) = self.worktrees.reclaim_idle().await
        {
            tracing::error!("[orchestrate] post-run worktree reclaim failed: {e}");
        }

        // Build the report.
        let plans: Vec<PlanRunReport> = plan_ids
            .iter()
            .map(|id| {
                let state = self.executor.plan_state(id);
                let succeeded = state.is_some_and(|s| {
                    matches!(
                        s.current_phase.kind(),
                        PhaseKind::Complete | PhaseKind::Done
                    )
                });
                PlanRunReport {
                    plan_id: id.clone(),
                    succeeded,
                    agent_calls: self.per_plan_agents.get(id).copied().unwrap_or(0),
                    gate_results: self.per_plan_gates.get(id).cloned().unwrap_or_default(),
                }
            })
            .collect();

        if self.cancel.is_cancelled() {
            tracing::warn!(
                "[orchestrate] run interrupted by shutdown signal; preserving partial state"
            );
        } else {
            // Emit plan-completed server events.
            for p in &plans {
                self.emit_server_event(crate::serve::events::ServerEvent::PlanCompleted {
                    plan_id: p.plan_id.clone(),
                    success: p.succeeded,
                });
                self.emit_execution_event(
                    &p.plan_id,
                    crate::serve::events::ExecutionEvent::PlanCompleted {
                        outcome: if p.succeeded {
                            "succeeded".to_string()
                        } else {
                            "failed".to_string()
                        },
                        stats: serde_json::json!({
                            "plan_id": &p.plan_id,
                            "succeeded": p.succeeded,
                            "agent_calls": p.agent_calls,
                            "gate_results": &p.gate_results,
                        }),
                    },
                );
            }

            // Increment plan completion metrics and log cost summaries.
            for p in &plans {
                let status = if p.succeeded { "succeeded" } else { "failed" };
                self.metrics
                    .register_counter(
                        "roko_plans_total",
                        "",
                        LabelSet::from_pairs(&[("status", status)]),
                    )
                    .inc();

                // Log cost summary from plan_costs HashMap.
                let plan_cost = self.plan_costs.get(&p.plan_id).copied().unwrap_or(0.0);
                if plan_cost > 0.0 {
                    tracing::info!(
                        plan_id = %p.plan_id,
                        cost_usd = plan_cost,
                        agent_calls = p.agent_calls,
                        succeeded = p.succeeded,
                        "plan completed"
                    );
                }
            }
        }

        // Log aggregate cost from CostsDb.
        let total_cost = self.learning.costs_db().total_cost();
        if total_cost > 0.0 {
            tracing::info!(
                total_cost_usd = total_cost,
                total_agent_calls = self.agent_calls,
                total_gate_runs = self.gate_runs,
                "orchestration cost summary"
            );
        }

        // Shut down any lingering agent processes.
        self.shutdown().await;

        // Best-effort aggregate of efficiency telemetry for the whole run.
        let efficiency_path = self.learning.paths().efficiency_jsonl.clone();
        let efficiency_events = match read_efficiency_events(&efficiency_path).await {
            Ok(events) => events,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %efficiency_path.display(),
                    "failed to read efficiency events"
                );
                self.efficiency_events.clone()
            }
        };
        let total_input_tokens: u64 = efficiency_events
            .iter()
            .map(|event| event.input_tokens)
            .sum();
        let total_output_tokens: u64 = efficiency_events
            .iter()
            .map(|event| event.output_tokens)
            .sum();
        let total_cost_usd: f64 = efficiency_events.iter().map(|event| event.cost_usd).sum();
        let duration_secs: f64 = efficiency_events
            .iter()
            .map(|event| event.wall_time_ms)
            .sum::<u64>() as f64
            / 1000.0;
        let tasks_completed: usize = self
            .task_trackers
            .values()
            .map(|tracker| tracker.completed.len())
            .sum();
        let tasks_failed: usize = self
            .task_trackers
            .values()
            .map(|tracker| tracker.failed.len())
            .sum();
        let fleet_cfactor = compute_fleet_cfactor(&self.efficiency_events);
        let fleet_cfactor = if fleet_cfactor.plan_count > 0 {
            Some(fleet_cfactor)
        } else {
            None
        };

        tracing::info!(
            total_cost_usd = total_cost_usd,
            total_input_tokens = total_input_tokens,
            total_output_tokens = total_output_tokens,
            duration_secs = duration_secs,
            tasks_completed = tasks_completed,
            tasks_failed = tasks_failed,
            "plan run complete"
        );

        if !self.cancel.is_cancelled() {
            // Auto-dream consolidation at plan completion (§5D.05).
            self.maybe_auto_dream().await;
        }

        // Final save before returning.
        if let Err(e) = self.save_state() {
            tracing::error!("[orchestrate] final save failed: {e}");
        }

        Ok(OrchestrationReport {
            total_agent_calls: self.agent_calls,
            total_gate_runs: self.gate_runs,
            plans,
            fleet_cfactor,
        })
    }

    /// Trigger an automatic dream consolidation if the config enables it and
    /// enough new episodes have accumulated since the last dream report.
    ///
    /// This is called at plan completion — not on a background loop. Failures
    /// are logged as warnings but never propagate to the caller.
    async fn maybe_auto_dream(&self) {
        if !self.config.dreams.auto_dream {
            tracing::debug!("[orchestrate] auto-dream disabled, skipping");
            return;
        }

        let episodes_path = self.workdir.join(".roko").join("episodes.jsonl");
        let episodes =
            match roko_learn::episode_logger::EpisodeLogger::read_all_lossy(&episodes_path).await {
                Ok(eps) => eps,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "[orchestrate] auto-dream: failed to read episodes, skipping"
                    );
                    return;
                }
            };

        // Determine a cutoff from the latest dream report so we only count
        // episodes that arrived after the previous consolidation.
        let dreams_dir = self.workdir.join(".roko").join("dreams");
        let last_report = roko_dreams::runner::load_latest_dream_report(&dreams_dir)
            .ok()
            .flatten();
        let cutoff = last_report
            .as_ref()
            .and_then(|r| r.processed_through.or(Some(r.started_at)));

        let new_episode_count = episodes
            .iter()
            .filter(|ep| cutoff.is_none_or(|ts| ep.timestamp > ts))
            .count();

        let min_required = self.config.dreams.min_episodes_for_dream;
        if new_episode_count < min_required {
            tracing::debug!(
                new_episode_count,
                min_required,
                "[orchestrate] auto-dream: not enough new episodes ({new_episode_count}/{min_required}), skipping"
            );
            return;
        }

        tracing::info!(
            new_episode_count,
            "[orchestrate] auto-dream: triggering consolidation ({new_episode_count} new episodes)"
        );

        let dream_config = DreamLoopConfig {
            auto_dream: self.config.dreams.auto_dream,
            idle_threshold_mins: self.config.dreams.idle_threshold_mins,
            min_episodes_for_dream: self.config.dreams.min_episodes_for_dream,
            agent: DreamAgentConfig {
                command: self.config.agent.command.clone(),
                args: self.config.agent.args.clone(),
                model: self.config.agent.model.clone(),
                bare_mode: self.config.agent.bare_mode,
                effort: self.config.agent.effort.clone(),
                fallback_model: self.config.agent.fallback_model.clone(),
                timeout_ms: self.config.agent.timeout_ms,
                env: self.config.agent.env.clone(),
            },
        };
        let mut runner = DreamRunner::new(self.workdir.clone(), dream_config);

        match runner.consolidate_now() {
            Ok(report) => {
                tracing::info!(
                    processed = report.processed_episodes,
                    clusters = report.clusters.len(),
                    knowledge = report.knowledge_entries_written,
                    playbooks = report.playbooks_created,
                    "[orchestrate] auto-dream consolidation complete"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "[orchestrate] auto-dream consolidation failed (non-fatal)"
                );
            }
        }
    }

    /// Run plans using tasks.toml files, routing through the full 14-phase
    /// executor state machine.
    ///
    /// Pre-loads [`TaskTracker`]s for plans that have `tasks.toml`, then
    /// delegates to [`run_all()`] which drives the state machine. The phase
    /// handlers (handle_enriching, handle_implementing, etc.) use the
    /// trackers for task-level granularity.
    #[instrument(skip_all, fields(plan_dir = %path.display()))]
    pub async fn run_task_plans(&mut self, path: &Path) -> Result<OrchestrationReport> {
        enum RunExit {
            Completed(Result<OrchestrationReport>),
            Signaled(Result<OrchestrationReport>),
            SignalTimedOut,
        }

        let snapshot_path = executor_snapshot_path(&self.workdir);
        let cancel = self.cancel.clone();
        let outcome = {
            let run = self.run_task_plans_inner(path);
            tokio::pin!(run);

            tokio::select! {
                result = &mut run => RunExit::Completed(result),
                signal = wait_for_shutdown_signal() => {
                    let signal = match signal {
                        Ok(signal) => signal,
                        Err(err) => return Err(err),
                    };
                    tracing::warn!(
                        signal,
                        "[orchestrate] shutdown signal received, draining in-flight tasks"
                    );

                    // Stop accepting new work. The run loop observes this and exits
                    // after the current in-flight task finishes or gets aborted.
                    cancel.cancel();

                    match tokio::time::timeout(
                        Duration::from_secs(SHUTDOWN_DRAIN_GRACE_SECS),
                        &mut run,
                    )
                    .await
                    {
                        Ok(result) => RunExit::Signaled(result),
                        Err(_) => {
                            tracing::warn!(
                                "[orchestrate] shutdown drain timed out after {}s, killing remaining agents",
                                SHUTDOWN_DRAIN_GRACE_SECS
                            );
                            RunExit::SignalTimedOut
                        }
                    }
                }
            }
        };

        match outcome {
            RunExit::Completed(result) => result,
            RunExit::Signaled(result) => {
                self.cleanup_tracked_plan_worktrees().await;
                self.save_state_to(&snapshot_path)?;
                tracing::info!(
                    "[orchestrate] checkpoint saved to {}",
                    snapshot_path.display()
                );
                self.flush_logs().await?;
                result
            }
            RunExit::SignalTimedOut => {
                self.force_shutdown().await;
                self.cleanup_tracked_plan_worktrees().await;
                self.save_state_to(&snapshot_path)?;
                tracing::info!(
                    "[orchestrate] checkpoint saved to {}",
                    snapshot_path.display()
                );
                self.flush_logs().await?;
                Ok(self.current_report())
            }
        }
    }

    #[instrument(skip_all, fields(plan_dir = %path.display()))]
    async fn run_task_plans_inner(&mut self, path: &Path) -> Result<OrchestrationReport> {
        let watcher_cancel = TokioCancellationToken::new();
        let watcher_task = WatcherRunner {
            conductor: Arc::clone(&self.conductor),
            signals_path: self.workdir.join(".roko").join("signals.jsonl"),
            efficiency_path: self.learning.paths().efficiency_jsonl.clone(),
            budget_usd: self.executor.config().budget_usd,
            cancel: watcher_cancel.clone(),
        }
        .spawn();

        let result = async {
            // Pre-load task trackers for any plans not already tracked
            let plan_dirs = Self::find_plan_dirs(path)?;
            for plan_dir in &plan_dirs {
                let name = plan_dir
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let tasks_path = plan_dir.join("tasks.toml");
                if tasks_path.exists() {
                    if let Ok(tf) = TasksFile::parse(&tasks_path) {
                        self.task_trackers
                            .entry(name)
                            .or_insert_with(|| TaskTracker::new(tf, plan_dir.clone()));
                    }
                }
            }

            self.run_all(&watcher_cancel).await
        }
        .await;

        watcher_cancel.cancel();
        let _ = watcher_task.await;

        if let Err(e) = refresh_cfactor_snapshot(self.learning.paths().root.clone()).await {
            tracing::warn!(error = %e, "failed to refresh c-factor snapshot after plan run");
        }

        result
    }

    /// Find plan directories (containing plan.md or tasks.toml).
    fn find_plan_dirs(plans_dir: &Path) -> Result<Vec<PathBuf>> {
        let mut dirs = Vec::new();
        if !plans_dir.is_dir() {
            return Ok(dirs);
        }

        // If plans_dir itself IS a plan (has tasks.toml or plan.md), use it directly.
        if plans_dir.join("tasks.toml").exists() || plans_dir.join("plan.md").exists() {
            dirs.push(plans_dir.to_path_buf());
            return Ok(dirs);
        }

        // Otherwise, look for plan subdirectories.
        for entry in
            std::fs::read_dir(plans_dir).with_context(|| format!("read {}", plans_dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() && (path.join("tasks.toml").exists() || path.join("plan.md").exists())
            {
                dirs.push(path);
            }
        }
        dirs.sort();
        Ok(dirs)
    }

    // ── Internal dispatch ─────────────────────────────────────────────────

    #[allow(clippy::too_many_lines)]
    async fn dispatch_action(&mut self, action: ExecutorAction) {
        self.actions_since_save += 1;

        match action {
            ExecutorAction::SpawnAgent {
                plan_id,
                role,
                task,
            } => {
                tracing::info!("[orchestrate] SpawnAgent plan={plan_id} role={role:?} task={task}");
                self.event_log.append(
                    EventKind::AgentSpawned,
                    serde_json::json!({"plan_id": plan_id, "role": format!("{role:?}"), "task": task}),
                );
                // Conductor signal: agent spawned (§7).
                self.emit_conductor_signal(
                    Kind::Custom("conductor.agent_spawn".into()),
                    serde_json::json!({
                        "plan_id": &plan_id,
                        "role": format!("{role:?}"),
                        "task": &task,
                        "event": "spawned",
                    }),
                );

                self.emit_server_event(crate::serve::events::ServerEvent::AgentSpawned {
                    agent_id: format!("{plan_id}:{task}"),
                    role: format!("{role:?}"),
                });

                match (role, task.as_str()) {
                    (AgentRole::Strategist, "enrich") => self.handle_enriching(&plan_id).await,
                    (AgentRole::Implementer, _) => self.handle_implementing(&plan_id).await,
                    (AgentRole::AutoFixer, "fix") => self.handle_autofix(&plan_id).await,
                    (AgentRole::AutoFixer, "regen-verify") => {
                        self.handle_regen_verify(&plan_id).await
                    }
                    (AgentRole::Auditor, "review") => self.handle_reviewing(&plan_id).await,
                    (AgentRole::Scribe, "docs") => self.handle_doc_revision(&plan_id).await,
                    _ => self.handle_generic_agent(&plan_id, role, &task).await,
                }
            }
            ExecutorAction::RunGate { plan_id, rung } => {
                tracing::info!("[orchestrate] RunGate plan={plan_id} rung={rung}");
                let gate_started = std::time::Instant::now();
                match self.run_gate_pipeline(&plan_id, rung).await {
                    Ok(passed) => {
                        self.gate_runs += 1;
                        self.per_plan_gates
                            .entry(plan_id.clone())
                            .or_default()
                            .push((format!("rung-{rung}"), passed));
                        self.event_log.append(
                            EventKind::GateResult,
                            serde_json::json!({"plan_id": plan_id, "rung": rung, "passed": passed}),
                        );
                        // Record gate episode.
                        let wall_ms =
                            u64::try_from(gate_started.elapsed().as_millis()).unwrap_or(u64::MAX);
                        // Gate runs are local process work, so the episode records zero USD cost
                        // while still carrying the latency field alongside it.
                        let gate_cost_usd = 0.0;
                        let mut ep = Episode::new("gate", format!("{plan_id}:rung-{rung}"));
                        ep.success = passed;
                        ep.usage = Usage {
                            wall_ms,
                            cost_usd: gate_cost_usd,
                            cost_usd_without_cache: gate_cost_usd,
                            ..Usage::default()
                        };
                        self.stamp_episode_affect(&mut ep, "gate", None);
                        ep.gate_verdicts = self
                            .task_trackers
                            .get(&plan_id)
                            .map(|tracker| tracker.last_gate_verdicts.clone())
                            .filter(|verdicts| !verdicts.is_empty())
                            .unwrap_or_else(|| {
                                vec![GateVerdict::new(format!("rung-{rung}"), passed)]
                            });
                        ep.input_signal_hash = self
                            .task_trackers
                            .get(&plan_id)
                            .and_then(|tracker| tracker.last_impl_output_hash)
                            .map(|hash| hash.to_string())
                            .unwrap_or_else(|| plan_id.clone());
                        let gate_input = self.enrich_completed_run(
                            ep,
                            &plan_id,
                            &format!("rung-{rung}"),
                            "gate",
                            "n/a",
                            Some(passed),
                            1,
                        );
                        self.apply_knowledge_tier_feedback(&plan_id);
                        self.record_and_check_learning(gate_input, &plan_id).await;

                        // Emit observability metric for gate result.
                        self.emit_gate_metric(&plan_id, rung, passed, wall_ms);

                        // Update adaptive gate thresholds.
                        self.adaptive_thresholds.update(rung, passed);

                        self.emit_server_event(crate::serve::events::ServerEvent::GateResult {
                            plan_id: plan_id.clone(),
                            task_id: format!("rung-{rung}"),
                            gate: format!("rung-{rung}"),
                            passed,
                        });
                        self.emit_execution_event(
                            &plan_id,
                            crate::serve::events::ExecutionEvent::GateResult {
                                task_id: format!("rung-{rung}"),
                                gate: format!("rung-{rung}"),
                                passed,
                                message: if passed {
                                    format!("rung-{rung} passed")
                                } else {
                                    format!("rung-{rung} failed")
                                },
                            },
                        );

                        let _ = self.daimon.appraise(AffectEvent::GateResult {
                            plan_id: plan_id.clone(),
                            task_id: format!("rung-{rung}"),
                            passed,
                            rung,
                        });

                        // Store gate failure context for AutoFix phase
                        if !passed {
                            let failed_gates: Vec<&GateResult> = self
                                .executor
                                .plan_state(&plan_id)
                                .map(|s| s.gate_results.iter().filter(|g| !g.passed).collect())
                                .unwrap_or_default();
                            let failure_context = self
                                .executor
                                .plan_state(&plan_id)
                                .and_then(|state| state.last_error.clone())
                                .unwrap_or_default();
                            let phase = Self::primary_failed_gate_name_from_results(&failed_gates)
                                .unwrap_or("unknown");

                            if let Some(tracker) = self.task_trackers.get_mut(&plan_id) {
                                tracker.last_gate_failure = Some(failure_context.clone());
                                tracker.last_gate_failure_phase = Some(phase.to_string());
                            }

                            // Emit a FailureTrace for observability.
                            let trace_id =
                                Self::trace_id_for(&plan_id, &format!("gate-fail-{rung}"));
                            let evidence = if failure_context.is_empty() {
                                failed_gates
                                    .iter()
                                    .map(|g| format!("{}: {}", g.gate_name, g.summary))
                                    .collect::<Vec<_>>()
                                    .join("; ")
                            } else {
                                failure_context.clone()
                            };
                            let ft = FailureTrace::new(
                                trace_id,
                                TraceStep::Execute,
                                FailureKind::ToolHandlerError,
                                evidence,
                            );
                            let event = ToolTraceEvent::Custom {
                                name: "failure_trace".to_string(),
                                data: serde_json::to_value(&ft).unwrap_or_default(),
                                at_ms: now_unix_ms_i64(),
                            };
                            self.obs_sinks.trace_sink.append(trace_id, event);

                            // 5F.23: Auto-generate AntiKnowledge from gate failures.
                            let anti_task_id = self
                                .task_trackers
                                .get(&plan_id)
                                .and_then(|t| t.last_impl_task_id.clone())
                                .unwrap_or_else(|| format!("rung-{rung}"));
                            let anti_content = if failure_context.is_empty() {
                                format!(
                                    "Gate '{phase}' failed for task {anti_task_id} in plan {plan_id}: {}",
                                    failed_gates
                                        .iter()
                                        .map(|g| format!("{}: {}", g.gate_name, g.summary))
                                        .collect::<Vec<_>>()
                                        .join("; ")
                                )
                            } else {
                                let snippet = if failure_context.len() > 500 {
                                    &failure_context[..500]
                                } else {
                                    &failure_context
                                };
                                format!(
                                    "Gate '{phase}' failed for task {anti_task_id} in plan {plan_id}: {snippet}"
                                )
                            };
                            let gate_failure_tag = self.daimon.emotional_tag("gate_failure");
                            let anti_entry = KnowledgeEntry {
                                id: format!("anti-gate-{plan_id}-{anti_task_id}-{rung}"),
                                kind: KnowledgeKind::AntiKnowledge,
                                source: Some(format!("gate-failure:{phase}")),
                                content: anti_content,
                                confidence: 0.9,
                                confidence_weight: -0.9,
                                refuted_insight_id: None,
                                refutation_evidence: None,
                                source_episodes: vec![format!("{plan_id}:rung-{rung}")],
                                tags: vec![
                                    "anti-knowledge".to_string(),
                                    "gate-failure".to_string(),
                                    phase.to_string(),
                                ],
                                source_model: None,
                                model_generality: 1.0,
                                created_at: chrono::Utc::now(),
                                half_life_days: KnowledgeKind::AntiKnowledge
                                    .default_half_life_days(),
                                tier: KnowledgeTier::Working,
                                emotional_tag: Some(gate_failure_tag.clone()),
                                emotional_provenance: Some(EmotionalProvenance::from_tag(
                                    &gate_failure_tag,
                                )),
                                hdc_vector: None,
                            };
                            if let Err(e) = self.knowledge_store.add(anti_entry) {
                                tracing::warn!(
                                    plan_id = %plan_id,
                                    phase = %phase,
                                    error = %e,
                                    "failed to record AntiKnowledge for gate failure"
                                );
                            }
                        }
                        let failure_reason = self
                            .task_trackers
                            .get(&plan_id)
                            .and_then(|tracker| tracker.last_gate_failure.clone())
                            .unwrap_or_else(|| "gate failure".to_string());
                        let event = if passed {
                            if let Some(tracker) = self.task_trackers.get_mut(&plan_id) {
                                tracker.gate_failure_count = 0;
                            }
                            ExecutorEvent::GatePassed
                        } else if self.no_replan {
                            if let Some(tracker) = self.task_trackers.get_mut(&plan_id) {
                                tracker.gate_failure_count += 1;
                            }
                            ExecutorEvent::Fatal(failure_reason)
                        } else {
                            if let Some(tracker) = self.task_trackers.get_mut(&plan_id) {
                                tracker.gate_failure_count += 1;
                            }
                            ExecutorEvent::GateFailed
                        };
                        self.apply_event_and_emit(
                            &plan_id,
                            &format!("rung-{rung}"),
                            &event,
                            "transitioned",
                        );

                        // Failure-driven re-planning (§9): retry after every
                        // consecutive gate failure, escalating the strategy as
                        // the failure count grows.
                        if !passed && !self.no_replan {
                            let failure_count = self
                                .task_trackers
                                .get(&plan_id)
                                .map(|t| t.gate_failure_count)
                                .unwrap_or(0);
                            if failure_count >= 1 && self.executor.config().auto_replan {
                                self.emit_execution_event(
                                    &plan_id,
                                    crate::serve::events::ExecutionEvent::ReplanTriggered {
                                        task_id: format!("rung-{rung}"),
                                        strategy: format!("gate_fail_count_{failure_count}"),
                                    },
                                );
                                self.attempt_replan(&plan_id).await;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("[orchestrate] gate failed for {plan_id}: {e}");
                        self.event_log.append(
                            EventKind::ErrorOccurred,
                            serde_json::json!({"plan_id": plan_id, "error": e.to_string()}),
                        );
                        self.apply_event_and_emit(
                            &plan_id,
                            &format!("rung-{rung}"),
                            &ExecutorEvent::GateFailed,
                            "transitioned",
                        );
                    }
                }
                // Conductor check after gate results.
                match self.run_conductor_check(&plan_id) {
                    ConductorDecision::Continue => {}
                    ConductorDecision::Restart { reason, .. } => {
                        tracing::info!("[conductor] restarting {plan_id}: {reason}");
                        self.apply_event_and_emit(
                            &plan_id,
                            "plan",
                            &ExecutorEvent::Start,
                            "transitioned",
                        );
                    }
                    ConductorDecision::Fail { reason, .. } => {
                        tracing::error!("[conductor] failing {plan_id}: {reason}");
                        self.apply_event_and_emit(
                            &plan_id,
                            "plan",
                            &ExecutorEvent::Fatal(format!("conductor: {reason}")),
                            "failed",
                        );
                    }
                    _ => {}
                }
            }
            ExecutorAction::RunVerify { plan_id } => {
                tracing::info!("[orchestrate] RunVerify plan={plan_id}");
                self.finish_verify_round(&plan_id).await;
            }
            ExecutorAction::MergeBranch { plan_id } => {
                tracing::info!("[orchestrate] MergeBranch plan={plan_id}");
                self.event_log.append(
                    EventKind::MergeAttempted,
                    serde_json::json!({"plan_id": plan_id}),
                );
                match self.merge_branch(&plan_id).await {
                    Ok(()) => {
                        match self.run_post_merge_follow_up(&plan_id).await {
                            Ok(true) => {
                                self.extract_pending_skill(&plan_id).await;
                                self.apply_event_and_emit(
                                    &plan_id,
                                    "merge",
                                    &ExecutorEvent::MergeSucceeded,
                                    "transitioned",
                                );
                                self.cleanup_plan_worktree(&plan_id).await;
                            }
                            Ok(false) => {
                                self.apply_event_and_emit(
                                    &plan_id,
                                    "merge",
                                    &ExecutorEvent::MergeFailed,
                                    "failed",
                                );
                            }
                            Err(e) => {
                                tracing::error!(
                                    "[orchestrate] post-merge checks failed for {plan_id}: {e}"
                                );
                                self.event_log.append(
                                    EventKind::ErrorOccurred,
                                    serde_json::json!({"plan_id": plan_id, "error": format!("post-merge follow-up failed: {e}")}),
                                );
                                // Keep historical behavior on infrastructure errors:
                                // merge itself succeeded.
                                self.apply_event_and_emit(
                                    &plan_id,
                                    "merge",
                                    &ExecutorEvent::MergeSucceeded,
                                    "transitioned",
                                );
                                self.cleanup_plan_worktree(&plan_id).await;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("[orchestrate] merge failed for {plan_id}: {e}");
                        self.apply_event_and_emit(
                            &plan_id,
                            "merge",
                            &ExecutorEvent::MergeFailed,
                            "failed",
                        );
                    }
                }
                // Conductor check after merge results.
                match self.run_conductor_check(&plan_id) {
                    ConductorDecision::Continue => {}
                    ConductorDecision::Restart { reason, .. } => {
                        tracing::info!("[conductor] restarting {plan_id}: {reason}");
                        self.apply_event_and_emit(
                            &plan_id,
                            "plan",
                            &ExecutorEvent::Start,
                            "transitioned",
                        );
                    }
                    ConductorDecision::Fail { reason, .. } => {
                        tracing::error!("[conductor] failing {plan_id}: {reason}");
                        self.apply_event_and_emit(
                            &plan_id,
                            "plan",
                            &ExecutorEvent::Fatal(format!("conductor: {reason}")),
                            "failed",
                        );
                    }
                    _ => {}
                }
            }
            ExecutorAction::DispatchPlan { plan_id } => {
                tracing::info!("[orchestrate] DispatchPlan {plan_id}");
                self.metrics
                    .register_counter(
                        "roko_plans_total",
                        "",
                        LabelSet::from_pairs(&[("status", "started")]),
                    )
                    .inc();
                self.event_log.append(
                    EventKind::PlanStarted,
                    serde_json::json!({"plan_id": plan_id}),
                );
                // Ensure TaskTracker exists for resume-from-snapshot case
                self.ensure_task_tracker(&plan_id);
                self.apply_event_and_emit(&plan_id, "plan", &ExecutorEvent::Start, "transitioned");
            }
            ExecutorAction::PausePlan { plan_id } => {
                tracing::info!("[orchestrate] PausePlan {plan_id}");
                self.executor.pause_plan(&plan_id);
            }
            ExecutorAction::ResumePlan { plan_id } => {
                tracing::info!("[orchestrate] ResumePlan {plan_id}");
                self.executor.resume_plan(&plan_id);
            }
            ExecutorAction::FailPlan { plan_id, reason } => {
                tracing::error!("[orchestrate] FailPlan {plan_id}: {reason}");
                self.event_log.append(
                    EventKind::ErrorOccurred,
                    serde_json::json!({"plan_id": &plan_id, "error": reason.clone()}),
                );
                self.apply_event_and_emit(
                    &plan_id,
                    "plan",
                    &ExecutorEvent::Fatal(reason),
                    "failed",
                );
                self.cleanup_plan_worktree(&plan_id).await;
            }
            ExecutorAction::CompletePlan { plan_id } => {
                tracing::info!("[orchestrate] CompletePlan {plan_id}");
                if let Some(state) = self.executor.plan_state_mut(&plan_id) {
                    state.current_phase = roko_core::PlanPhase::Complete;
                    state.paused = false;
                }
                self.event_log.append(
                    EventKind::PhaseTransition,
                    serde_json::json!({"plan_id": &plan_id, "event": "CompletePlan"}),
                );
                self.emit_execution_event(
                    &plan_id,
                    crate::serve::events::ExecutionEvent::PlanCompleted {
                        outcome: "succeeded".to_string(),
                        stats: serde_json::json!({
                            "plan_id": &plan_id,
                            "phase": "complete",
                        }),
                    },
                );
                self.cleanup_plan_worktree(&plan_id).await;
            }
            ExecutorAction::Reorder {
                plan_id,
                new_position,
            } => {
                tracing::info!("[orchestrate] Reorder {plan_id} -> {new_position}");
                self.executor.reorder_plan(&plan_id, new_position);
                self.event_log.append(
                    EventKind::PhaseTransition,
                    serde_json::json!({"plan_id": &plan_id, "event": "Reorder", "new_position": new_position}),
                );
            }
            _ => unreachable!("non-exhaustive ExecutorAction variant"),
        }
    }

    // ── Phase handlers ─────────────────────────────────────────────────

    /// Enriching phase: build the strategist enrichment prompt, dispatch the agent,
    /// and advance only after enrichment completes successfully.
    async fn handle_enriching(&mut self, plan_id: &str) {
        // Ensure tracker is loaded
        self.ensure_task_tracker(plan_id);

        let started = std::time::Instant::now();
        let enrichment_user_prompt = format!(
            "Enrich plan {plan_id}: analyze the supplied plan context, read_files, and task constraints. \
            Return execution-ready notes that preserve task dependencies, blockers, and role constraints."
        );
        let enrichment_system_prompt = self.build_enrichment_system_prompt(plan_id);
        let role = AgentRole::Strategist;

        match self
            .dispatch_agent_with(
                plan_id,
                role,
                "enrich",
                Some(enrichment_user_prompt),
                None,
                None,
                Some(enrichment_system_prompt),
            )
            .await
        {
            Ok(result) => {
                *self.per_plan_agents.entry(plan_id.to_string()).or_default() += 1;
                self.agent_calls += 1;

                let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                let mut ep = Episode::new("Strategist", "enrich").succeeded();
                ep.usage = Usage {
                    wall_ms,
                    cost_usd: f64::from(result.usage.cost_usd),
                    input_tokens: u64::from(result.usage.input_tokens),
                    output_tokens: u64::from(result.usage.output_tokens),
                    ..Usage::default()
                };
                self.stamp_episode_affect(&mut ep, "enrich", Some(&result.output));
                ep.input_signal_hash = plan_id.to_string();
                ep.output_signal_hash = result.output.id.to_string();
                let model = self.effective_model();
                let input =
                    self.enrich_completed_run(ep, plan_id, "enrich", "Strategist", &model, None, 1);
                self.record_and_check_learning(input, plan_id).await;

                if let Some(tracker) = self.task_trackers.get(plan_id) {
                    let groups = tracker.tasks_file.parallel_groups();
                    tracing::info!(
                        "[orchestrate] Enriching {plan_id}: {} tasks, {} parallel groups",
                        tracker.tasks_file.tasks.len(),
                        groups.len(),
                    );
                } else {
                    tracing::info!(
                        "[orchestrate] Enriching {plan_id}: no tasks.toml, using generic strategist enrichment"
                    );
                }

                let event = ExecutorEvent::EnrichmentDone;
                self.log_transition(plan_id, &event);
                self.apply_event_and_emit(plan_id, "enrich", &event, "transitioned");
            }
            Err(e) => {
                tracing::error!("[orchestrate] Enrichment failed for {plan_id}: {e}");
                self.apply_event_and_emit(
                    plan_id,
                    "enrich",
                    &ExecutorEvent::Fatal(format!("enrichment failed: {e}")),
                    "failed",
                );
            }
        }
    }

    /// Implementing phase: dispatch ready tasks, parallelising when multiple are
    /// unblocked. Single-task dispatch includes retry logic; parallel batches
    /// fail individual tasks without retries (the next tick re-evaluates).
    async fn handle_implementing(&mut self, plan_id: &str) {
        // If no tracker, fall through to generic agent
        if !self.task_trackers.contains_key(plan_id) {
            self.handle_generic_agent(plan_id, AgentRole::Implementer, "next")
                .await;
            return;
        }

        let completed_plans = self.executor.completed_plans();

        // Collect ALL ready tasks (deps satisfied, not completed/failed).
        {
            let Some(tracker) = self.task_trackers.get_mut(plan_id) else {
                return; // unreachable: checked above
            };
            tracker.sync_ready_queue(&completed_plans);
        }
        let ready: Vec<String> = {
            let Some(tracker) = self.task_trackers.get(plan_id) else {
                return; // unreachable: checked above
            };
            let groups = tracker.tasks_file.parallel_groups();
            groups
                .get(tracker.current_group_index)
                .map(|group| {
                    group
                        .iter()
                        .filter(|t| {
                            !tracker.completed.contains(&t.id)
                                && !tracker.failed.contains(&t.id)
                                && t.is_ready_with_plan_deps(&tracker.completed, &completed_plans)
                        })
                        .map(|t| t.id.clone())
                        .collect()
                })
                .unwrap_or_default()
        };
        let ready = prioritize_ready_tasks(ready, |task_id| {
            let queue_wait_hours = self
                .task_trackers
                .get(plan_id)
                .and_then(|tracker| tracker.queue_wait_hours(task_id))
                .unwrap_or(0.0);
            self.learning
                .task_arousal_with_queue_wait(task_id, queue_wait_hours)
        });

        // Appraise QueueWait for any ready task that has been waiting > 24h.
        if let Some(tracker) = self.task_trackers.get(plan_id) {
            for tid in &ready {
                if let Some(wait_hours) = tracker.queue_wait_hours(tid) {
                    if wait_hours > 24.0 {
                        let _ = self.daimon.appraise(AffectEvent::QueueWait {
                            task_id: tid.clone(),
                            wait_hours,
                        });
                    }
                }
            }
        }

        if ready.is_empty() {
            // No ready tasks — check if all done or blocked
            let all_done = self
                .task_trackers
                .get(plan_id)
                .is_some_and(TaskTracker::all_tasks_done);
            if all_done {
                let event = ExecutorEvent::ImplementationDone;
                self.log_transition(plan_id, &event);
                let task_id = self
                    .task_trackers
                    .get(plan_id)
                    .and_then(|tracker| tracker.last_impl_task_id.clone())
                    .unwrap_or_else(|| "implementation".into());
                self.apply_event_and_emit(plan_id, &task_id, &event, "transitioned");
            } else if self
                .task_trackers
                .get(plan_id)
                .is_some_and(|tracker| tracker.has_tasks_blocked_by_plans(&completed_plans))
            {
                // Count how many tasks are blocked by cross-plan deps.
                let blocker_count = self
                    .task_trackers
                    .get(plan_id)
                    .map(|tracker| {
                        tracker
                            .tasks_file
                            .tasks
                            .iter()
                            .filter(|t| {
                                !tracker.completed.contains(&t.id)
                                    && !tracker.failed.contains(&t.id)
                                    && !t.is_ready_with_plan_deps(
                                        &tracker.completed,
                                        &completed_plans,
                                    )
                            })
                            .count()
                    })
                    .unwrap_or(1);
                let _ = self.daimon.appraise(AffectEvent::Blocked {
                    task_id: plan_id.to_string(),
                    blocker_count,
                });
                tracing::info!(
                    "[orchestrate] {plan_id}: implementation blocked by dependent plan(s), pausing"
                );
                self.executor.pause_plan(plan_id);
            } else {
                // Count remaining non-completed, non-failed tasks as blocked.
                let blocker_count = self
                    .task_trackers
                    .get(plan_id)
                    .map(|tracker| {
                        tracker
                            .tasks_file
                            .tasks
                            .iter()
                            .filter(|t| {
                                !tracker.completed.contains(&t.id)
                                    && !tracker.failed.contains(&t.id)
                            })
                            .count()
                    })
                    .unwrap_or(1);
                let _ = self.daimon.appraise(AffectEvent::Blocked {
                    task_id: plan_id.to_string(),
                    blocker_count,
                });
                tracing::error!(
                    "[orchestrate] {plan_id}: no ready tasks but not all done — blocked or failed"
                );
                self.apply_event_and_emit(
                    plan_id,
                    "implementation",
                    &ExecutorEvent::Fatal("all remaining tasks blocked or failed".into()),
                    "failed",
                );
            }
            return;
        }

        if self.cloud_execution.is_some() && ready.len() > 1 {
            for task_id in &ready {
                self.handle_implementing_single(plan_id, task_id).await;
            }
        } else if ready.len() == 1 {
            // ── Single task: sequential dispatch with retry ──────────
            self.handle_implementing_single(plan_id, &ready[0]).await;
        } else {
            // ── Multiple ready tasks: parallel dispatch ──────────────
            let batch = ready;
            tracing::info!(
                "[orchestrate] Implementing {plan_id}: dispatching {} tasks in parallel: {}",
                batch.len(),
                batch.join(", "),
            );
            self.handle_implementing_parallel(plan_id, &batch).await;
        }

        // Check if all tasks are now done
        let all_done = self
            .task_trackers
            .get(plan_id)
            .is_some_and(TaskTracker::all_tasks_done);
        if all_done {
            tracing::info!("[orchestrate] {plan_id}: all tasks done, advancing to Gating");
            let event = ExecutorEvent::ImplementationDone;
            self.log_transition(plan_id, &event);
            let task_id = self
                .task_trackers
                .get(plan_id)
                .and_then(|tracker| tracker.last_impl_task_id.clone())
                .unwrap_or_else(|| "implementation".into());
            self.apply_event_and_emit(plan_id, &task_id, &event, "transitioned");
        }

        // Conductor check after agent dispatch completes.
        match self.run_conductor_check(plan_id) {
            ConductorDecision::Restart { reason, .. } => {
                tracing::info!("[conductor] restarting {plan_id}: {reason}");
                self.apply_event_and_emit(plan_id, "plan", &ExecutorEvent::Start, "transitioned");
            }
            ConductorDecision::Fail { reason, .. } => {
                tracing::error!("[conductor] failing {plan_id}: {reason}");
                self.apply_event_and_emit(
                    plan_id,
                    "plan",
                    &ExecutorEvent::Fatal(format!("conductor: {reason}")),
                    "failed",
                );
            }
            _ => {}
        }
        // else: plan stays in Implementing. Next tick() returns another SpawnAgent.
    }

    /// Dispatch a single task with retry logic (up to 2 retries).
    async fn handle_implementing_single(&mut self, plan_id: &str, task_id: &str) {
        tracing::info!("[orchestrate] Implementing {plan_id}: dispatching task {task_id}");

        let task_def = self
            .task_trackers
            .get(plan_id)
            .and_then(|tracker| {
                tracker
                    .tasks_file
                    .tasks
                    .iter()
                    .find(|task| task.id == task_id)
            })
            .cloned();
        let task_phase = task_def
            .as_ref()
            .map(|task| task.status.as_str())
            .unwrap_or("unknown");
        let _span = info_span!("task", plan_id = %plan_id, task_id = %task_id, phase = %task_phase)
            .entered();

        // Track which task is being worked on (used by autofix if gates fail).
        if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
            tracker.last_impl_task_id = Some(task_id.to_string());
            tracker.last_impl_model_slug = None;
            tracker.last_impl_output_hash = None;
            tracker.last_context_knowledge_ids.clear();
            tracker.last_gate_verdicts.clear();
        }

        let wt_id = format!("{plan_id}-{task_id}");
        let started = std::time::Instant::now();
        let max_retries = 2u32;
        let max_dispatches = max_retries + 1;
        let mut succeeded = false;
        let mut budget_aborted = false;
        let mut total_dispatches = 0u32;
        let mut retry_iteration = 0u32;
        let mut consecutive_failures = 0u32;
        let task_complexity = task_def
            .as_ref()
            .map(|task| task.tier.clone())
            .unwrap_or_else(|| "focused".to_string());
        let base_retry_model = self.task_retry_model(plan_id, task_id);
        let mut retry_model = base_retry_model.clone();
        let mut retry_prompt_override: Option<String> = None;
        let mut pending_feedback: Option<(RetryConductorState, RetryConductorAction)> = None;
        let mut terminal_error: Option<anyhow::Error> = None;
        let mut terminal_failure_handled = false;
        let exec_dir = match self.task_exec_dir(plan_id, task_id).await {
            Ok(dir) => dir,
            Err(e) => {
                tracing::error!(
                    "[orchestrate] task worktree acquisition failed for {plan_id}/{task_id}: {e}"
                );
                self.record_task_failure(plan_id, task_id, None, None, &e, &started, None)
                    .await;
                self.apply_event_and_emit(
                    plan_id,
                    task_id,
                    &ExecutorEvent::Fatal(format!(
                        "failed to acquire worktree for task {task_id}: {e}"
                    )),
                    "failed",
                );
                return;
            }
        };

        while total_dispatches < max_dispatches {
            if total_dispatches > 0 {
                tracing::info!(
                    "[orchestrate] Retry {}/{} for {plan_id}/{task_id} (model={retry_model})",
                    total_dispatches,
                    max_retries
                );
            }

            let prompt_override = retry_prompt_override.take();
            let model_override = prompt_override.as_ref().map(|_| retry_model.clone());
            total_dispatches += 1;

            match self
                .dispatch_agent_with(
                    plan_id,
                    AgentRole::Implementer,
                    task_id,
                    prompt_override,
                    model_override,
                    Some(exec_dir.clone()),
                    None,
                )
                .await
            {
                Ok(result) => {
                    if let Some((state, action)) = pending_feedback.take() {
                        self.retry_conductor.record_outcome(&state, action, true);
                        self.persist_retry_conductor();
                    }
                    if let Err(e) = self
                        .finalize_successful_task_worktree(plan_id, task_id, &exec_dir)
                        .await
                    {
                        tracing::error!(
                            "[orchestrate] task worktree finalization failed for {plan_id}/{task_id}: {e}"
                        );
                        self.record_task_failure(
                            plan_id,
                            task_id,
                            None,
                            Some(retry_model.as_str()),
                            &e,
                            &started,
                            Some(&result),
                        )
                        .await;
                        self.apply_event_and_emit(
                            plan_id,
                            task_id,
                            &ExecutorEvent::Fatal(format!(
                                "task worktree finalization failed: {e}"
                            )),
                            "failed",
                        );
                        terminal_failure_handled = true;
                        break;
                    }
                    match self
                        .record_task_success(plan_id, task_id, &result, &started)
                        .await
                    {
                        Ok(()) => {
                            succeeded = true;
                            if let Some(cloud) = self.cloud_execution.clone() {
                                let task_title = self
                                    .task_trackers
                                    .get(plan_id)
                                    .and_then(|tracker| {
                                        tracker
                                            .tasks_file
                                            .tasks
                                            .iter()
                                            .find(|task| task.id == task_id)
                                            .map(|task| task.title.clone())
                                    })
                                    .unwrap_or_else(|| task_id.to_string());
                                let commit_message = format!("task: {task_title}");
                                if let Err(e) =
                                    crate::worker::cloud::git_commit(&exec_dir, &commit_message)
                                        .await
                                {
                                    tracing::error!(
                                        "[orchestrate] cloud commit failed for {plan_id}/{task_id}: {e}"
                                    );
                                    self.apply_event_and_emit(
                                        plan_id,
                                        task_id,
                                        &ExecutorEvent::Fatal(format!("cloud commit failed: {e}")),
                                        "failed",
                                    );
                                    return;
                                }

                                if let Err(e) = crate::worker::cloud::git_push(
                                    &exec_dir,
                                    &cloud.branch_name(),
                                    &cloud.github_token,
                                )
                                .await
                                {
                                    tracing::error!(
                                        "[orchestrate] cloud push failed for {plan_id}/{task_id}: {e}"
                                    );
                                    self.apply_event_and_emit(
                                        plan_id,
                                        task_id,
                                        &ExecutorEvent::Fatal(format!("cloud push failed: {e}")),
                                        "failed",
                                    );
                                    return;
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                "[orchestrate] task {task_id} aborted by plan budget: {e}"
                            );
                            self.apply_event_and_emit(
                                plan_id,
                                task_id,
                                &ExecutorEvent::Fatal(e.to_string()),
                                "failed",
                            );
                            budget_aborted = true;
                        }
                    }
                    break;
                }
                Err(e) => {
                    if let Some((state, action)) = pending_feedback.take() {
                        self.retry_conductor.record_outcome(&state, action, false);
                        self.persist_retry_conductor();
                    }

                    consecutive_failures = consecutive_failures.saturating_add(1);
                    let failure_gate = Self::retry_failure_gate(&e);
                    let failure_context = format!("{e:#}");
                    let state = RetryConductorState {
                        iteration: retry_iteration.saturating_add(1),
                        consecutive_failures,
                        error_pattern: Self::retry_error_pattern(&e),
                        elapsed_ms: u64::try_from(started.elapsed().as_millis())
                            .unwrap_or(u64::MAX),
                        cost_so_far_usd: self.task_spent(plan_id, task_id),
                        model_tier: Self::retry_model_tier_label(&retry_model),
                        task_complexity: task_complexity.clone(),
                    };
                    let action = self.retry_conductor.select_action(&state);
                    tracing::error!(
                        "[orchestrate] task {task_id} failed (attempt {}): {e}",
                        total_dispatches
                    );

                    tracing::info!(
                        plan_id = %plan_id,
                        task_id = %task_id,
                        ?action,
                        iteration = state.iteration,
                        consecutive_failures = state.consecutive_failures,
                        error_pattern = ?state.error_pattern,
                        "[orchestrate] conductor selected retry action"
                    );

                    if matches!(action, RetryConductorAction::Abort)
                        || total_dispatches >= max_dispatches
                    {
                        self.retry_conductor.record_outcome(&state, action, false);
                        self.persist_retry_conductor();
                        terminal_error = Some(e);
                        break;
                    }

                    match action {
                        RetryConductorAction::Continue => {
                            retry_prompt_override = (retry_model != base_retry_model).then(|| {
                                self.build_conductor_retry_prompt(
                                    plan_id,
                                    task_id,
                                    task_def.as_ref(),
                                    &failure_context,
                                    failure_gate.as_deref(),
                                    None,
                                )
                            });
                            retry_iteration = retry_iteration.saturating_add(1);
                            pending_feedback = Some((state, action));
                        }
                        RetryConductorAction::InjectHint(hint) => {
                            retry_prompt_override = Some(self.build_conductor_retry_prompt(
                                plan_id,
                                task_id,
                                task_def.as_ref(),
                                &failure_context,
                                failure_gate.as_deref(),
                                Some(hint),
                            ));
                            retry_iteration = retry_iteration.saturating_add(1);
                            pending_feedback = Some((state, action));
                        }
                        RetryConductorAction::SwitchModel => {
                            retry_model = self.next_tier_model_slug(&retry_model);
                            retry_prompt_override = Some(self.build_conductor_retry_prompt(
                                plan_id,
                                task_id,
                                task_def.as_ref(),
                                &failure_context,
                                failure_gate.as_deref(),
                                None,
                            ));
                            retry_iteration = retry_iteration.saturating_add(1);
                            pending_feedback = Some((state, action));
                        }
                        RetryConductorAction::Restart => {
                            retry_model = base_retry_model.clone();
                            retry_prompt_override = None;
                            retry_iteration = 0;
                            consecutive_failures = 0;
                            pending_feedback = Some((state, action));
                        }
                        RetryConductorAction::Abort => {
                            unreachable!("abort handled above");
                        }
                    }
                }
            }
        }

        if let Some(error) = terminal_error.as_ref() {
            self.record_task_failure(
                plan_id,
                task_id,
                None,
                Some(retry_model.as_str()),
                error,
                &started,
                None,
            )
            .await;
            if self.should_replan_after_task_failure()
                && !self.no_replan
                && self.executor.config().auto_replan
            {
                tracing::info!(
                    plan_id = %plan_id,
                    task_id = %task_id,
                    "[orchestrate] Daimon requested replan after task failure"
                );
                self.attempt_replan(plan_id).await;
                terminal_failure_handled = true;
            }
        }

        if self.worktrees_enabled() {
            if let Err(e) = self.worktrees.remove(&wt_id).await {
                tracing::error!("[orchestrate] worktree cleanup failed for {task_id}: {e}");
            }
        }

        if !succeeded && !budget_aborted && !terminal_failure_handled {
            tracing::error!("[orchestrate] task {task_id} failed after {max_retries} retries");
            self.apply_event_and_emit(
                plan_id,
                task_id,
                &ExecutorEvent::Fatal(format!("task {task_id} failed after retries")),
                "failed",
            );
        }
    }

    /// Dispatch multiple tasks in parallel using per-task worktrees.
    /// Each task gets its own worktree so agents don't step on each other.
    /// Failures are recorded individually; the batch does not abort on error.
    async fn handle_implementing_parallel(&mut self, plan_id: &str, task_ids: &[String]) {
        let concurrency_limit = self.executor.config().max_concurrent_tasks.max(1);

        // Create per-task worktrees and record exec dirs.
        let shared_target = RokoLayout::for_project(&self.workdir).cargo_target_dir();
        let mut task_dirs: Vec<(String, PathBuf)> = Vec::with_capacity(task_ids.len());
        let started = std::time::Instant::now();
        for tid in task_ids {
            if let Err(e) = self.ensure_task_budget_available(plan_id, tid) {
                tracing::error!(
                    "[orchestrate] task budget exhausted before dispatch for {plan_id}/{tid}: {e}"
                );
                self.record_task_failure(plan_id, tid, None, None, &e, &started, None)
                    .await;
                continue;
            }
            match self.task_exec_dir(plan_id, tid).await {
                Ok(dir) => task_dirs.push((tid.clone(), dir)),
                Err(e) => {
                    tracing::error!(
                        "[orchestrate] task worktree acquisition failed for {plan_id}/{tid}: {e}"
                    );
                    self.record_task_failure(plan_id, tid, None, None, &e, &started, None)
                        .await;
                }
            }
        }

        // Track all tasks as in-progress.
        if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
            if let Some(first) = task_ids.first() {
                tracker.last_impl_task_id = Some(first.clone());
                tracker.last_impl_model_slug = None;
            }
        }

        // ── Build agent configs sequentially (needs &mut self) ───────
        let mut configs: Vec<(String, String, AgentRunConfig)> =
            Vec::with_capacity(task_dirs.len());

        let plan_dir = plans_dir(&self.workdir).join(plan_id);
        let tasks_toml = plan_dir.join("tasks.toml");
        let tasks_file = if tasks_toml.exists() {
            crate::task_parser::TasksFile::parse(&tasks_toml).ok()
        } else {
            None
        };

        let role = AgentRole::Implementer;
        let claude_tools_csv = claude_tool_allowlist(role);
        let skip_perms = role == AgentRole::Implementer || role == AgentRole::AutoFixer;
        let mcp_config_path = self.resolve_mcp_config_path().await;

        for (tid, dir) in &task_dirs {
            let task_def = tasks_file
                .as_ref()
                .and_then(|tf| tf.tasks.iter().find(|t| t.id == *tid).cloned());
            let task_phase = task_def
                .as_ref()
                .map(|task| task.status.clone())
                .unwrap_or_else(|| "unknown".into());

            let (prompt_text, model) = if let Some(ref td) = task_def {
                let p = td.build_prompt(plan_id, &self.workdir);
                let m = td.effective_model(
                    self.config
                        .agent
                        .model
                        .as_deref()
                        .unwrap_or("claude-sonnet-4-6"),
                    Some(&self.config.agent.tier_models),
                );
                (p, m)
            } else {
                let p =
                    format!("Plan: {plan_id}\nTask: {tid}\n\nImplement the task described above.");
                let m = self
                    .config
                    .agent
                    .model
                    .clone()
                    .unwrap_or_else(|| "claude-opus-4-6".into());
                (p, m)
            };

            let task_allowed_tools_csv = if let Some(task) = task_def.as_ref() {
                claude_task_tool_allowlist_with(
                    role,
                    task.allowed_tools.as_deref(),
                    task.denied_tools.as_deref(),
                    self.tool_registry.as_deref(),
                )
            } else {
                claude_tools_csv.clone()
            };
            let system_prompt = build_system_prompt(
                role,
                plan_id,
                tid,
                &task_allowed_tools_csv,
                task_def.as_ref(),
            );
            let env_vars: Vec<(String, String)> = self
                .config
                .agent
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .chain(std::iter::once((
                    "CARGO_TARGET_DIR".into(),
                    shared_target.display().to_string(),
                )))
                .collect();

            configs.push((
                tid.clone(),
                task_phase,
                AgentRunConfig {
                    command: self.config.agent.command.clone(),
                    exec_dir: dir.clone(),
                    model,
                    timeout_ms: self.effective_task_timeout_ms(task_def.as_ref()),
                    bare_mode: self.config.agent.bare_mode,
                    effort: self.config.agent.effort.clone(),
                    system_prompt,
                    allowed_tools_csv: task_allowed_tools_csv.clone(),
                    mcp_config: mcp_config_path.clone(),
                    fallback_model: self.config.agent.fallback_model.clone(),
                    env_vars,
                    read_args: task_def
                        .as_ref()
                        .map(task_read_cli_args)
                        .unwrap_or_default(),
                    extra_args: self.config.agent.args.clone(),
                    resume_session: self.claude_resume_session.clone(),
                    prompt: prompt_text,
                    skip_permissions: skip_perms,
                },
            ));
        }

        let mut results: Vec<ParallelTaskResult> = Vec::with_capacity(task_ids.len());
        let plan_id_owned = plan_id.to_owned();
        let mut pending = configs.into_iter();
        loop {
            // Run one dependency-level slice at a time, capping the number of
            // spawned tasks to the configured executor limit.
            let mut join_set = JoinSet::new();
            let mut launched = 0usize;
            while launched < concurrency_limit {
                let Some((tid, task_phase, cfg)) = pending.next() else {
                    break;
                };
                launched += 1;
                let plan_id = plan_id_owned.clone();
                join_set.spawn(async move {
                    let span = info_span!(
                        "task",
                        plan_id = %plan_id,
                        task_id = %tid,
                        agent_model = %cfg.model,
                        task_role = %role,
                        phase = %task_phase
                    );
                    let task_id = tid;
                    let exec_dir = cfg.exec_dir.clone();
                    let prompt_text = cfg.prompt.clone();
                    let system_prompt = cfg.system_prompt.clone();
                    let model = cfg.model.clone();
                    let result = run_prepared_agent(cfg).instrument(span).await;
                    ParallelTaskResult {
                        task_id,
                        exec_dir,
                        prompt_text,
                        system_prompt,
                        model,
                        result,
                    }
                });
            }

            if launched == 0 {
                break;
            }

            while let Some(joined) = join_set.join_next().await {
                match joined {
                    Ok(pair) => results.push(pair),
                    Err(e) => {
                        tracing::error!("[orchestrate] parallel task join failed: {e}");
                    }
                }
            }
        }

        // ── Process results sequentially ─────────────────────────────
        let mut any_fatal = false;
        for task_result in &results {
            let tid = &task_result.task_id;
            self.add_task_spend(plan_id, tid, f64::from(task_result.result.usage.cost_usd));
            if task_result.result.success {
                let task_def = tasks_file
                    .as_ref()
                    .and_then(|tf| tf.tasks.iter().find(|task| task.id == *tid).cloned());
                if let Err(e) = self
                    .finish_task_post_processing(
                        plan_id,
                        tid,
                        task_def.as_ref(),
                        &task_result.prompt_text,
                        &task_result.system_prompt,
                        &task_result.model,
                        &task_result.result,
                        &task_result.exec_dir,
                    )
                    .await
                {
                    tracing::error!(
                        "[orchestrate] parallel task {tid} post-processing failed: {e}"
                    );
                    self.record_task_failure(
                        plan_id,
                        tid,
                        Some(task_result.prompt_text.as_str()),
                        Some(task_result.model.as_str()),
                        &e,
                        &started,
                        Some(&task_result.result),
                    )
                    .await;
                    any_fatal = true;
                    continue;
                }
                if let Err(e) = self
                    .finalize_successful_task_worktree(plan_id, tid, &task_result.exec_dir)
                    .await
                {
                    tracing::error!(
                        "[orchestrate] parallel task worktree finalization failed for {plan_id}/{tid}: {e}"
                    );
                    self.record_task_failure(
                        plan_id,
                        tid,
                        Some(task_result.prompt_text.as_str()),
                        Some(task_result.model.as_str()),
                        &e,
                        &started,
                        Some(&task_result.result),
                    )
                    .await;
                    any_fatal = true;
                    continue;
                }
                if let Err(e) = self
                    .record_task_success(plan_id, tid, &task_result.result, &started)
                    .await
                {
                    tracing::error!("[orchestrate] task {tid} aborted by plan budget: {e}");
                    self.apply_event_and_emit(
                        plan_id,
                        tid,
                        &ExecutorEvent::Fatal(e.to_string()),
                        "failed",
                    );
                    any_fatal = true;
                    break;
                }
            } else {
                tracing::error!("[orchestrate] parallel task {tid} failed");
                let err = anyhow!("agent returned non-success for task {tid}");
                self.record_task_failure(
                    plan_id,
                    tid,
                    None,
                    Some(task_result.model.as_str()),
                    &err,
                    &started,
                    Some(&task_result.result),
                )
                .await;
                any_fatal = true;
            }
        }

        // ── Clean up per-task worktrees ──────────────────────────────
        if self.worktrees_enabled() {
            for tid in task_ids {
                let wt_id = format!("{plan_id}-{tid}");
                if let Err(e) = self.worktrees.remove(&wt_id).await {
                    tracing::error!("[orchestrate] worktree cleanup failed for {tid}: {e}");
                }
            }
        }

        let completed_plans = self.executor.completed_plans();
        if any_fatal
            && self
                .task_trackers
                .get(plan_id)
                .is_some_and(|t| t.ready_tasks(&completed_plans).is_empty())
        {
            // All remaining tasks are blocked by failures.
            self.apply_event_and_emit(
                plan_id,
                "implementation",
                &ExecutorEvent::Fatal(
                    "parallel batch had failures; remaining tasks blocked".into(),
                ),
                "failed",
            );
        }
    }

    /// Build a [`CompletedRunInput`] enriched with cost record, provider, and
    /// task metric data derived from the agent result context.
    fn enrich_completed_run(
        &self,
        mut ep: Episode,
        plan_id: &str,
        task_id: &str,
        role: &str,
        model: &str,
        gate_passed: Option<bool>,
        iteration: u32,
    ) -> CompletedRunInput {
        if ep.agent_template.trim().is_empty() {
            ep.agent_template = role.to_string();
        }
        if ep.model.trim().is_empty() {
            ep.model = model.to_string();
        }
        ep.extra
            .entry("plan_id".to_string())
            .or_insert_with(|| serde_json::json!(plan_id));
        ep.extra
            .entry("role".to_string())
            .or_insert_with(|| serde_json::json!(role));
        ep.extra
            .entry("model".to_string())
            .or_insert_with(|| serde_json::json!(model));
        ep.extra
            .entry("task_category".to_string())
            .or_insert_with(|| serde_json::json!(default_task_category(role)));

        let provider = self.provider_id_for_model(model);
        let cost = CostRecord {
            timestamp: chrono::Utc::now().to_rfc3339(),
            model: model.to_string(),
            provider: provider.clone(),
            role: role.to_string(),
            plan_id: plan_id.to_string(),
            task_id: task_id.to_string(),
            complexity_band: "standard".to_string(),
            input_tokens: ep.usage.input_tokens,
            output_tokens: ep.usage.output_tokens,
            cached_tokens: 0,
            cost_usd: ep.usage.cost_usd,
            duration_ms: ep.usage.wall_ms,
            success: ep.success,
            session_id: plan_id.to_string(),
        };

        let mut input = CompletedRunInput::from_episode(ep).with_cost_record(cost);
        input.provider = Some(provider);

        // Flow matched skill/rule/experiment IDs from the task tracker so
        // record_completed_run can update confidence scores and experiment outcomes.
        if let Some(tracker) = self.task_trackers.get(plan_id) {
            if input.matched_skill_id.is_none() {
                input.matched_skill_id = tracker.last_matched_skill_id.clone();
            }
            if input.playbook_rule_id.is_none() {
                input.playbook_rule_id = tracker.last_matched_rule_id.clone();
            }
            if input.experiment_variant_id.is_none() {
                input.experiment_variant_id = tracker.last_experiment_variant_id.clone();
            }
        }

        if let Some(passed) = gate_passed {
            let metric = TaskMetric {
                timestamp: chrono::Utc::now().to_rfc3339(),
                plan_id: plan_id.to_string(),
                task_id: task_id.to_string(),
                iteration,
                role: role.to_string(),
                backend: "claude".to_string(),
                model: model.to_string(),
                gate_passed: passed,
                wall_time_ms: input.episode.usage.wall_ms,
                input_tokens: input.episode.usage.input_tokens,
                output_tokens: input.episode.usage.output_tokens,
                cost_usd: input.episode.usage.cost_usd,
                ..TaskMetric::new(ConfigHash("roko".to_string()), plan_id, task_id)
            };
            input = input.with_task_metric(metric);
        }

        input
    }

    /// Resolve the effective model name from config.
    fn effective_model(&self) -> String {
        self.config
            .agent
            .model
            .clone()
            .unwrap_or_else(|| "claude-sonnet-4-6".into())
    }

    fn task_retry_model(&self, plan_id: &str, task_id: &str) -> String {
        self.task_trackers
            .get(plan_id)
            .and_then(|tracker| {
                tracker
                    .tasks_file
                    .tasks
                    .iter()
                    .find(|task| task.id == task_id)
            })
            .map(|task| {
                task.effective_model(
                    self.config
                        .agent
                        .model
                        .as_deref()
                        .unwrap_or("claude-sonnet-4-6"),
                    Some(&self.config.agent.tier_models),
                )
            })
            .unwrap_or_else(|| self.effective_model())
    }

    fn retry_model_tier_label(model_slug: &str) -> String {
        if model_slug.contains("haiku") || model_slug.contains("mini") {
            "fast".to_string()
        } else if model_slug.contains("opus") || model_slug.contains("gpt-5") {
            "premium".to_string()
        } else {
            "standard".to_string()
        }
    }

    fn retry_failure_gate(error: &anyhow::Error) -> Option<String> {
        error
            .chain()
            .find_map(|cause| cause.to_string().strip_prefix("gate=").map(str::to_owned))
            .or_else(|| {
                let chain = format!("{error:#}");
                chain
                    .lines()
                    .find_map(|line| line.trim().strip_prefix("gate="))
                    .map(str::trim)
                    .filter(|gate| !gate.is_empty())
                    .map(ToOwned::to_owned)
            })
    }

    fn retry_error_pattern(error: &anyhow::Error) -> RetryErrorPattern {
        let gate = Self::retry_failure_gate(error);
        if gate
            .as_deref()
            .is_some_and(|gate| matches!(gate, "compile" | "clippy"))
        {
            return RetryErrorPattern::Compile;
        }
        if gate
            .as_deref()
            .is_some_and(|gate| matches!(gate, "test" | "integration"))
        {
            return RetryErrorPattern::Test;
        }

        let chain = format!("{error:#}");
        if chain.contains(GHOST_TURN_SIGNAL_KIND) {
            return RetryErrorPattern::LoopDetected;
        }
        if chain.to_ascii_lowercase().contains("tool") {
            return RetryErrorPattern::ToolCall;
        }

        let diagnosis = DiagnosisEngine::default().diagnose(&chain);
        diagnosis
            .first()
            .map(|match_| match match_.category {
                ErrorCategory::CompileError
                | ErrorCategory::ClippyWarning
                | ErrorCategory::DependencyError
                | ErrorCategory::TypeMismatch
                | ErrorCategory::BorrowCheckerError
                | ErrorCategory::LifetimeError
                | ErrorCategory::ImportError => RetryErrorPattern::Compile,
                ErrorCategory::TestFailure => RetryErrorPattern::Test,
                ErrorCategory::TimeoutError => RetryErrorPattern::Timeout,
                ErrorCategory::LlmRateLimit => RetryErrorPattern::RateLimit,
                ErrorCategory::LlmContextOverflow => RetryErrorPattern::ContextOverflow,
                ErrorCategory::LlmRefusal => RetryErrorPattern::Refusal,
                ErrorCategory::LoopDetected => RetryErrorPattern::LoopDetected,
                ErrorCategory::GitConflict
                | ErrorCategory::MissingFile
                | ErrorCategory::PermissionDenied
                | ErrorCategory::NetworkError
                | ErrorCategory::OomError
                | ErrorCategory::DiskFull
                | ErrorCategory::ProcessCrash => RetryErrorPattern::Infrastructure,
                _ => RetryErrorPattern::Unknown,
            })
            .unwrap_or(RetryErrorPattern::Unknown)
    }

    fn build_conductor_retry_hint(
        &self,
        plan_id: &str,
        task_id: &str,
        task_def: Option<&crate::task_parser::TaskDef>,
        hint: HintType,
        error_output: &str,
    ) -> String {
        match hint {
            HintType::ErrorDigest => format!(
                "## Retry Guidance\n\nUse the failing gate output below as the primary constraint and fix the concrete issue before making broader changes.\n\n```text\n{}\n```",
                truncate_output(error_output)
            ),
            HintType::SkillSuggestion => {
                let task_files = task_def.map(|task| task.files.clone()).unwrap_or_default();
                let task_tier = task_def.map(|task| task.tier.as_str()).unwrap_or("");
                let task_text = task_def
                    .map(|task| task.build_prompt(plan_id, &self.workdir))
                    .unwrap_or_else(|| format!("Plan: {plan_id}\nTask ID: {task_id}\n"));
                let symbols = extract_task_symbols(&task_text);
                let skills: Vec<_> = self
                    .skill_library
                    .query(&task_files, task_tier, &symbols)
                    .into_iter()
                    .filter(|skill| skill.success_rate > 0.5)
                    .take(1)
                    .collect();

                if skills.is_empty() {
                    "## Retry Guidance\n\nReuse the simplest prior-success pattern available in this area instead of inventing a new approach.".to_string()
                } else {
                    format!(
                        "## Retry Guidance\n\nApply the strongest matching prior skill before changing other parts of the codebase.\n\n{}",
                        render_prior_experience(&skills)
                    )
                }
            }
            HintType::SimplifyApproach => "## Retry Guidance\n\nTake the smallest path that makes the failing verification pass. Avoid refactors, new abstractions, and unrelated cleanup on this retry.".to_string(),
        }
    }

    fn build_conductor_retry_prompt(
        &self,
        plan_id: &str,
        task_id: &str,
        task_def: Option<&crate::task_parser::TaskDef>,
        error_output: &str,
        gate: Option<&str>,
        hint: Option<HintType>,
    ) -> String {
        let gate = gate.unwrap_or("verify");
        let base_prompt = task_def
            .map(|task| task.build_prompt(plan_id, &self.workdir))
            .unwrap_or_else(|| {
                format!("Plan: {plan_id}\nTask: {task_id}\n\nImplement the task described above.")
            });
        let prompt = task_def
            .map(|task| task.build_fix_prompt(&base_prompt, gate, error_output))
            .unwrap_or_else(|| {
                format!(
                    "{base_prompt}\n\n---\n\n## Verification Failed\n\nPhase: {gate}\n\nError output:\n```text\n{}\n```",
                    truncate_output(error_output)
                )
            });

        match hint {
            Some(hint) => format!(
                "{prompt}\n\n---\n\n{}",
                self.build_conductor_retry_hint(plan_id, task_id, task_def, hint, error_output)
            ),
            None => prompt,
        }
    }

    fn persist_retry_conductor(&self) {
        if let Err(err) = self
            .retry_conductor
            .save(&conductor_policy_path(&self.workdir))
        {
            tracing::warn!("[orchestrate] failed to persist conductor policy: {err}");
        }
    }

    /// Resolve the runtime provider id for a model slug using `roko.toml`.
    fn provider_id_for_model(&self, model_slug: &str) -> String {
        let Ok(routing_config) = load_roko_config(&self.workdir) else {
            return model_slug.to_string();
        };
        let resolved = resolve_model(&routing_config, model_slug);
        resolved
            .profile
            .as_ref()
            .map(|profile| profile.provider.clone())
            .or_else(|| {
                resolved
                    .provider_config
                    .as_ref()
                    .map(|provider| provider.kind.label().to_string())
            })
            .unwrap_or_else(|| resolved.slug.clone())
    }

    fn record_model_experiment_outcome(
        &self,
        selected_experiment: Option<&SelectedModelExperiment>,
        gate_passed: bool,
        result: &AgentResult,
    ) -> Result<()> {
        let Some(selected_experiment) = selected_experiment else {
            return Ok(());
        };

        let experiment_path = model_experiments_path(&self.workdir);
        let mut experiment_store = ModelExperimentStore::load_or_new(&experiment_path);
        experiment_store.record_outcome(
            &selected_experiment.experiment_id,
            &selected_experiment.variant_id,
            gate_passed,
            f64::from(result.usage.cost_usd),
            u64::from(result.usage.input_tokens) + u64::from(result.usage.output_tokens),
            result.usage.wall_ms,
        );
        experiment_store
            .save(&experiment_path)
            .with_context(|| format!("save model experiments to {}", experiment_path.display()))
            .map_err(Into::into)
    }

    /// Record post-turn latency and anomaly feedback before any later early return.
    fn record_turn_learning_feedback(
        &mut self,
        prompt: &Engram,
        model: &str,
        result: &AgentResult,
    ) {
        let provider_id = self.provider_id_for_model(model);

        let prompt_hash = {
            let mut hasher = DefaultHasher::new();
            prompt.id.to_hex().hash(&mut hasher);
            hasher.finish()
        };

        let ttft_source_ms = result
            .trace
            .first()
            .map(|signal| signal.created_at_ms)
            .unwrap_or(result.output.created_at_ms);
        let ttft_ms = ttft_source_ms.saturating_sub(prompt.created_at_ms).max(0) as u64;
        let total_ms = result.usage.wall_ms;
        let output_tokens = u64::from(result.usage.output_tokens);
        publish_turn_learning_feedback(
            &self.learning_event_bus,
            &self.latency_registry,
            &mut self.anomaly_detector,
            TurnLearningFeedback {
                task_id: prompt.id.to_hex(),
                model: model.to_string(),
                provider: provider_id,
                timestamp_ms: prompt.created_at_ms,
                prompt_hash,
                ttft_ms,
                total_ms,
                output_tokens,
                cost_usd: f64::from(result.usage.cost_usd),
                usage: result.usage,
                success: result.success,
            },
        );
    }

    /// Record a LinUCB observation for the implementer task route.
    fn observe_cascade_router(
        &self,
        plan_id: &str,
        task_id: &str,
        task_def: Option<&crate::task_parser::TaskDef>,
        model_slug: &str,
        reward: f64,
    ) {
        if let Some(model_idx) = self
            .learning
            .cascade_router()
            .model_index_for_slug(model_slug)
        {
            let context_vec =
                cascade_context_vec(self, plan_id, task_id, AgentRole::Implementer, task_def);
            self.learning
                .cascade_router()
                .observe(context_vec, model_idx, reward);
        } else {
            tracing::debug!(
                plan_id = %plan_id,
                model = %model_slug,
            "skipping cascade observation: model not found in router arms"
            );
        }
    }

    /// Persist one crate-familiarity observation for the current task.
    fn record_crate_familiarity(
        &mut self,
        plan_id: &str,
        task_id: &str,
        task_def: Option<&crate::task_parser::TaskDef>,
        success: bool,
    ) {
        if let Err(err) = self
            .crate_familiarity_tracker
            .record_task_outcome(task_def, success, plan_id, task_id)
        {
            tracing::warn!(
                plan_id = %plan_id,
                task_id = %task_id,
                error = %err,
                "failed to record crate familiarity observation"
            );
        }
    }

    /// Build a learned-context string from skills, playbook rules, and patterns.
    ///
    /// Returns the context text plus any matched skill/rule IDs for flowing
    /// into `CompletedRunInput` so confidence gets updated.
    fn build_learned_context(
        &self,
        role: AgentRole,
        task_def: Option<&crate::task_parser::TaskDef>,
        task_text: &str,
        task_tier: Option<&str>,
        current_model: &str,
    ) -> LearnedContext {
        use roko_learn::playbook_rules::MatchContext;

        let mut parts: Vec<String> = Vec::new();
        let mut matched_skill_id: Option<String> = None;
        let mut matched_rule_id: Option<String> = None;

        // 1. Relevant skills from the skill library (search by role tag)
        let role_tag = format!("{role:?}").to_lowercase();
        let skills = self.skill_library.search_by_tag(&role_tag);
        if !skills.is_empty() {
            // Track the top skill as the matched one for confidence updates.
            matched_skill_id = skills.first().map(|s| s.name.clone());
            let mut skill_section = String::from("## Relevant Skills from Past Successes\n");
            for skill in skills.iter().take(3) {
                skill_section.push_str(&format!("- **{}**: {}\n", skill.name, skill.summary));
            }
            parts.push(skill_section);
        }

        // 2. Applicable playbook rules
        let match_ctx = MatchContext {
            files: Vec::new(),
            tags: Vec::new(),
            category: None,
            error_signature: None,
            role: role_tag.clone(),
        };
        let rules = self.learning.playbook_rules().select(&match_ctx, 5);
        if !rules.is_empty() {
            // Track the top rule for confidence updates.
            matched_rule_id = rules.first().map(|r| r.rule_id.clone());
            let mut rule_section = String::from("## Playbook Rules (do/don\'t heuristics)\n");
            for rule in &rules {
                rule_section.push_str(&format!(
                    "- [confidence={:.0}%] {}\n",
                    rule.confidence * 100.0,
                    rule.body
                ));
            }
            parts.push(rule_section);
        }

        // 3. Discovered patterns from the pattern miner
        let patterns = self.learning.pattern_miner().lock().discover();
        if !patterns.is_empty() {
            let mut pat_section = String::from("## Discovered Action Patterns\n");
            for pat in patterns.iter().take(3) {
                pat_section.push_str(&format!(
                    "- Pattern (support={}, confidence={:.0}%): {}\n",
                    pat.support_count,
                    pat.confidence * 100.0,
                    pat.description
                ));
            }
            parts.push(pat_section);
        }

        // 4. Persistent strategy fragments distilled by roko-neuro.
        if let Some(strategy_fragments) = build_strategy_fragment_context(
            &self.knowledge_store,
            role,
            task_def,
            task_text,
            current_model,
        ) {
            parts.push(strategy_fragments);
        }

        // 5. Durable knowledge queried from NeuroStore for this task.
        if let Some(knowledge_context) = self.build_knowledge_context(task_text, task_tier) {
            parts.push(knowledge_context);
        }

        // 6. Prompt experiment variants — check if any active experiment applies.
        let mut experiment_variant_id = None;
        // Check standard prompt section names for active experiments.
        {
            let store = self.learning.experiment_store().lock();
            for section in &["constraints", "style", "guidelines", "context"] {
                if let Some((vid, content)) = store.assign_variant_for_section(section) {
                    parts.push(format!("## Experiment ({section})\n{content}"));
                    experiment_variant_id = Some(vid);
                    break; // Only one experiment at a time.
                }
            }
        }

        // 7. Crate familiarity score from cascade router observations (§9).
        let obs_count = self.learning.cascade_router().total_observations();
        if obs_count > 0 {
            let familiarity = (obs_count as f64 / 100.0).min(1.0);
            parts.push(format!(
                "## Crate Familiarity\nBased on {obs_count} prior observations, \
                 familiarity score: {familiarity:.2}/1.0."
            ));
        }

        LearnedContext {
            text: parts.join("\n"),
            matched_skill_id,
            matched_rule_id,
            experiment_variant_id,
        }
    }

    fn build_knowledge_context(&self, task_text: &str, task_tier: Option<&str>) -> Option<String> {
        let query_limit = match task_tier.unwrap_or("focused") {
            "mechanical" => 3,
            "focused" => 5,
            "integrative" => 5,
            "architectural" => 6,
            _ => 4,
        };

        let Ok(entries) = NeuroStore::query(&self.knowledge_store, task_text, query_limit) else {
            tracing::debug!("[orchestrate] knowledge query failed for task context");
            return None;
        };
        let entries: Vec<_> = entries
            .into_iter()
            .filter(|entry| entry.kind != KnowledgeKind::StrategyFragment)
            .collect();
        if entries.is_empty() {
            return None;
        }

        Some(render_knowledge_context(entries))
    }

    /// Record a successful task result: persist output, episode, mark completed.
    async fn record_task_success(
        &mut self,
        plan_id: &str,
        task_id: &str,
        result: &AgentResult,
        started: &std::time::Instant,
    ) -> Result<()> {
        *self.per_plan_agents.entry(plan_id.to_string()).or_default() += 1;
        self.agent_calls += 1;

        if let Ok(text) = result.output.body.as_text() {
            save_task_output(&self.workdir, task_id, text);
        }

        let task_def = self
            .task_trackers
            .get(plan_id)
            .and_then(|t| t.tasks_file.tasks.iter().find(|td| td.id == task_id))
            .cloned();
        let frequency = task_def
            .as_ref()
            .map_or(OperatingFrequency::Theta, |td| td.operating_frequency());
        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        // Appraise time pressure when elapsed > 80% of timeout.
        {
            let timeout_ms = self.effective_task_timeout_ms(task_def.as_ref());
            if timeout_ms > 0 {
                let proximity = (wall_ms as f64) / (timeout_ms as f64);
                if proximity > 0.8 {
                    let _ = self.daimon.appraise(AffectEvent::TimePressure {
                        task_id: task_id.to_string(),
                        deadline_proximity: proximity.min(1.0),
                    });
                }
            }
        }

        let mut cascade_router_observed = false;

        // ── Observe cascade router for bandit learning (§9) ─────────
        if result.success {
            use roko_core::TaskComplexityBand;
            use roko_core::config::schema::RewardWeights;
            use roko_learn::model_router::CONTEXT_DIM;

            let model = self.effective_model();
            if let Some(model_idx) = self.learning.cascade_router().model_index_for_slug(&model) {
                let task_tier = task_def
                    .as_ref()
                    .map(|td| td.tier.as_str())
                    .unwrap_or("focused");
                // Keep the 17-dim LinUCB shape and reserve the trailing slots
                // so the raw success-path observation matches the router schema.
                let mut context_vec = vec![0.0; CONTEXT_DIM];
                let tier_idx = match task_tier {
                    "mechanical" => 0,
                    "focused" => 1,
                    "integrative" => 2,
                    "architectural" => 3,
                    _ => 1,
                };
                context_vec[tier_idx] = 1.0;

                let complexity = match task_tier {
                    "mechanical" => TaskComplexityBand::Fast,
                    "architectural" => TaskComplexityBand::Complex,
                    _ => TaskComplexityBand::Standard,
                };
                context_vec[4] = match complexity {
                    TaskComplexityBand::Fast => 0.0,
                    TaskComplexityBand::Complex => 1.0,
                    TaskComplexityBand::Standard => 0.5,
                    _ => 0.5,
                };

                let iteration = self
                    .task_trackers
                    .get(plan_id)
                    .map(|tracker| f64::from(tracker.impl_round.saturating_add(1)))
                    .unwrap_or(1.0);
                context_vec[5] = (iteration / 10.0).min(1.0);
                context_vec[6..10].copy_from_slice(&role_hash_features("Implementer"));
                context_vec[10] = 0.5;
                context_vec[11] = if self
                    .task_trackers
                    .get(plan_id)
                    .is_some_and(|tracker| tracker.gate_failure_count > 0)
                {
                    1.0
                } else {
                    0.0
                };
                context_vec[16] = 1.0;

                let normalized_cost = if self.config.budget.max_task_usd > 0.0 {
                    (f64::from(result.usage.cost_usd) / self.config.budget.max_task_usd).min(1.0)
                } else {
                    0.0
                };
                let normalized_duration = {
                    let timeout_ms = self.effective_task_timeout_ms(task_def.as_ref());
                    if timeout_ms > 0 {
                        (wall_ms as f64 / timeout_ms as f64).min(1.0)
                    } else {
                        0.0
                    }
                };
                let reward_weights = load_roko_config(&self.workdir)
                    .map(|cfg| cfg.routing.weights.for_tier(task_tier))
                    .unwrap_or_else(|_| RewardWeights::default());

                self.learning.cascade_router().observe_multi_objective(
                    context_vec,
                    model_idx,
                    1.0,
                    normalized_cost,
                    normalized_duration,
                    &reward_weights,
                );
                cascade_router_observed = true;
            } else {
                tracing::debug!(
                    plan_id = %plan_id,
                    task_id = %task_id,
                    model = %model,
                    "skipping cascade observation: model not found in router arms"
                );
            }
        }

        if let Some(task_def) = task_def.as_ref()
            && let Err(err) = self.playbook.record(&task_def.id, result.success).await
        {
            tracing::warn!(
                plan_id = %plan_id,
                task_id = %task_id,
                error = %err,
                "failed to record playbook outcome"
            );
        }

        let mut ep = Episode::new("Implementer", task_id).succeeded();
        if ep.episode_id.is_empty() {
            ep.episode_id = ep.id.clone();
        }
        let success_episode_id = ep.episode_id.clone();
        let task_strategy =
            self.stamp_task_strategy_metadata(&mut ep, plan_id, task_id, task_def.as_ref());
        ep.usage = Usage {
            wall_ms,
            cost_usd: f64::from(result.usage.cost_usd),
            input_tokens: u64::from(result.usage.input_tokens),
            output_tokens: u64::from(result.usage.output_tokens),
            ..Usage::default()
        };
        self.stamp_episode_affect(&mut ep, "task_success", Some(&result.output));
        ep.input_signal_hash = plan_id.to_string();
        ep.output_signal_hash = result.output.id.to_string();
        if cascade_router_observed {
            ep.extra.insert(
                "cascade_router_observed".to_string(),
                serde_json::json!(true),
            );
        }
        let model = self.effective_model();
        let input = self.enrich_completed_run(ep, plan_id, task_id, "Implementer", &model, None, 1);
        self.record_and_check_learning(input, plan_id).await;
        self.record_crate_familiarity(plan_id, task_id, task_def.as_ref(), true);
        let success_entry = build_success_knowledge_entry(
            plan_id,
            task_id,
            task_def.as_ref(),
            result,
            &model,
            &success_episode_id,
        );
        if let Err(err) = self.knowledge_store.ingest(vec![success_entry]) {
            tracing::warn!(
                plan_id = %plan_id,
                task_id = %task_id,
                error = %err,
                "failed to ingest success knowledge entry"
            );
        }

        // Emit efficiency event for this agent turn.
        self.emit_efficiency_event(
            plan_id,
            task_id,
            "Implementer",
            &model,
            frequency,
            result,
            wall_ms,
            true,
        )
        .await;

        let plan_spent = self.plan_costs.get(plan_id).copied().unwrap_or(0.0);
        if plan_spent >= self.config.budget.max_plan_usd {
            return Err(anyhow!(
                "plan {plan_id} budget exhausted: ${plan_spent:.2} >= ${:.2} max",
                self.config.budget.max_plan_usd
            ));
        }

        if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
            tracker.mark_completed(task_id);
            tracker.last_impl_output_hash = Some(result.output.id);
        }

        self.emit_execution_event(
            plan_id,
            crate::serve::events::ExecutionEvent::TaskCompleted {
                task_id: task_id.to_string(),
                outcome: if result.success {
                    "succeeded".to_string()
                } else {
                    "failed".to_string()
                },
            },
        );

        // Appraise task outcome for affect modulation.
        let _ = self.daimon.appraise(AffectEvent::TaskOutcome {
            task_id: task_id.to_string(),
            succeeded: true,
        });
        self.daimon.record_somatic_outcome(
            task_strategy,
            somatic_episode_hash(plan_id, task_id, "success", &success_episode_id),
        );

        // Emit observability trace event for the successful agent dispatch.
        self.emit_agent_trace(plan_id, task_id, true, wall_ms);

        tracing::info!(
            plan_id = %plan_id,
            task_id = %task_id,
            duration_ms = wall_ms,
            "task completed"
        );
        Ok(())
    }

    /// Record a completed run and check the returned `LearningUpdate` for
    /// regression alerts.
    async fn record_and_check_learning(&mut self, input: CompletedRunInput, plan_id: &str) {
        match self.learning.record_completed_run(input).await {
            Ok(update) => self.handle_learning_update(&update, plan_id),
            Err(e) => tracing::error!("[orchestrate] episode log failed: {e}"),
        }
    }

    /// Inspect a `LearningUpdate` for regression alerts and extracted skills,
    /// logging them via tracing.
    fn handle_learning_update(&self, update: &LearningUpdate, plan_id: &str) {
        if let Some(ref report) = update.regression_report {
            if report.has_regressions {
                for alert in report.regressions() {
                    tracing::warn!(
                        plan_id = %plan_id,
                        metric = %alert.metric_name,
                        severity = ?alert.severity,
                        description = %alert.description,
                        "regression detected"
                    );
                }
            }
        }
        if let Some(ref skill_id) = update.extracted_skill_id {
            tracing::info!(plan_id = %plan_id, skill = %skill_id, "skill extracted from agent output");
        }
    }

    /// Summarize all known gate failures for a plan.
    fn gate_failure_report(&self, plan_id: &str) -> String {
        let Some(state) = self.executor.plan_state(plan_id) else {
            return String::new();
        };

        let mut sections = Vec::new();
        for verdict in state.gate_results.iter().filter(|verdict| !verdict.passed) {
            let mut section = format!("{}: {}", verdict.gate_name, verdict.summary.trim());
            if !verdict.summary.trim().is_empty() && verdict.summary.trim() != verdict.gate_name {
                section.push_str("\nsummary:\n");
                section.push_str(verdict.summary.trim());
            }
            sections.push(section);
        }

        if let Some(last_error) = state
            .last_error
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            sections.push(format!("executor.last_error:\n{last_error}"));
        }

        sections.join("\n\n---\n\n")
    }

    /// Build the prompt used for decomposition re-planning.
    fn build_decompose_prompt(
        &self,
        plan_id: &str,
        tasks_file: &TasksFile,
        original_task: &crate::task_parser::TaskDef,
        failure_summary: &str,
        gate_report: &str,
    ) -> String {
        let mut prompt = String::new();
        prompt.push_str(failure_summary);
        prompt.push_str("\n\n## Original task spec\n");
        prompt.push_str(&task_spec_summary(tasks_file));
        prompt.push_str("\n\n## Original task prompt\n");
        prompt.push_str(&original_task.build_prompt(plan_id, &self.workdir));

        if !gate_report.trim().is_empty() {
            prompt.push_str("\n\n## Gate failure outputs\n");
            prompt.push_str(gate_report);
        }

        prompt.push_str(
            "\n\n## Output requirements\n\
             Return ONLY a valid tasks.toml file.\n\
             Produce 2-3 subtasks that replace the failed task.\n\
             Keep task ids unique, preserve the plan's executable order, and do not add prose or markdown fences.",
        );
        prompt
    }

    /// Append a re-plan episode to the learning log.
    async fn record_replan_episode(
        &mut self,
        plan_id: &str,
        original_task_id: &str,
        original_task: Option<&crate::task_parser::TaskDef>,
        strategy: ReplanStrategy,
        attempt_number: u32,
        resulting_subtasks: &[crate::task_parser::TaskDef],
        success: bool,
        failure_reason: Option<String>,
    ) {
        let mut ep = Episode::new("Strategist", original_task_id);
        ep.kind = "replan".to_string();
        ep.success = success;
        ep.failure_reason = failure_reason;
        self.stamp_episode_affect(&mut ep, "replan", None);
        ep.input_signal_hash = plan_id.to_string();
        ep.output_signal_hash = format!("{plan_id}:{original_task_id}:replan:{strategy}");
        ep.extra
            .insert("strategy".to_string(), serde_json::json!(strategy));
        ep.extra.insert(
            "attempt_number".to_string(),
            serde_json::json!(attempt_number),
        );
        ep.extra.insert(
            "original_task_id".to_string(),
            serde_json::json!(original_task_id),
        );
        if let Some(task) = original_task {
            ep.extra.insert(
                "original_task".to_string(),
                serde_json::json!({
                    "id": &task.id,
                    "title": &task.title,
                    "status": &task.status,
                    "tier": &task.tier,
                    "role": &task.role,
                    "depends_on": &task.depends_on,
                    "files": &task.files,
                    "replan_strategy": task.replan_strategy,
                }),
            );
        }
        ep.extra.insert(
            "resulting_subtasks".to_string(),
            serde_json::json!(
                resulting_subtasks
                    .iter()
                    .map(|task| {
                        serde_json::json!({
                            "id": &task.id,
                            "title": &task.title,
                        })
                    })
                    .collect::<Vec<_>>()
            ),
        );

        if let Err(e) = self.learning.append_episode(&ep).await {
            tracing::error!(
                plan_id = %plan_id,
                task_id = %original_task_id,
                strategy = %strategy,
                error = %e,
                "failed to append re-plan episode"
            );
        }
    }

    /// Attempt to re-plan after gate failures (§9).
    async fn attempt_replan(&mut self, plan_id: &str) {
        let Some(tracker) = self.task_trackers.get(plan_id) else {
            tracing::warn!("[orchestrate] replan requested for unknown plan {plan_id}");
            return;
        };

        let failure_context = tracker.last_gate_failure.clone().unwrap_or_default();
        let failure_phase = tracker.last_gate_failure_phase.clone().unwrap_or_default();
        let failure_count = tracker.gate_failure_count;
        let task_id = tracker
            .last_impl_task_id
            .clone()
            .or_else(|| tracker.tasks_file.tasks.first().map(|task| task.id.clone()))
            .unwrap_or_else(|| "replan".to_string());
        let task_def = tracker
            .tasks_file
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .cloned();
        let frequency = task_def
            .as_ref()
            .map_or(OperatingFrequency::Theta, |task| task.operating_frequency());
        let terminal_count = tracker.terminal_task_count();
        let total_tasks = tracker.tasks_file.tasks.len();
        let plan_dir = tracker._plan_dir.clone();
        let current_model = self.effective_model();
        let task_model = task_def
            .as_ref()
            .map(|task| task.effective_model(&current_model, Some(&self.config.agent.tier_models)))
            .unwrap_or_else(|| current_model.clone());
        let escalate_model = self.next_tier_model_slug(&task_model);
        let architectural_model = self
            .config
            .agent
            .tier_models
            .get("architectural")
            .cloned()
            .unwrap_or_else(|| "claude-opus-4-6".into());

        let strategy = task_def
            .as_ref()
            .and_then(|task| task.replan_strategy)
            .unwrap_or_else(|| {
                if total_tasks > 0 && terminal_count * 2 > total_tasks {
                    ReplanStrategy::RegeneratePlan
                } else {
                    match failure_count {
                        0 | 1 => ReplanStrategy::RetrySame,
                        2 if task_model.contains("opus") => ReplanStrategy::Decompose,
                        2 => ReplanStrategy::RetryWithEscalation,
                        3 => ReplanStrategy::Decompose,
                        _ => ReplanStrategy::Skip,
                    }
                }
            });

        let gate_errors = self
            .gate_failure_report(plan_id)
            .split("\n\n---\n\n")
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();

        self.emit_failure_efficiency_event(
            plan_id,
            &task_id,
            "Implementer",
            &task_model,
            frequency,
            0,
            gate_errors,
            &strategy.to_string(),
            failure_count,
        )
        .await;

        tracing::info!(
            plan_id = %plan_id,
            task_id = %task_id,
            failure_count,
            terminal_count,
            total_tasks,
            strategy = %strategy,
            "attempting replan"
        );

        let failure_summary = format!(
            "Plan '{plan_id}' failed gate checks.\n\n\
             Failure count: {failure_count}\n\
             Last failing phase: {failure_phase}\n\
             Failure details:\n```\n{failure_context}\n```"
        );

        match strategy {
            ReplanStrategy::RetrySame => {
                let prompt = format!(
                    "{failure_summary}\n\n\
                     Retry the same task with the error context above. \
                     Focus on the smallest fix that makes the gate pass."
                );
                match self
                    .dispatch_agent_with(
                        plan_id,
                        AgentRole::Strategist,
                        "replan",
                        Some(prompt),
                        None,
                        None,
                        None,
                    )
                    .await
                {
                    Ok(_) => {
                        tracing::info!("[orchestrate] replan retry-same completed for {plan_id}");
                        self.record_replan_episode(
                            plan_id,
                            &task_id,
                            task_def.as_ref(),
                            ReplanStrategy::RetrySame,
                            failure_count,
                            &[],
                            true,
                            None,
                        )
                        .await;
                        if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
                            tracker.gate_failure_count = 0;
                        }
                        let result = ReplanResult::RetrySame {
                            plan_id: plan_id.to_string(),
                            task_id: task_id.clone(),
                        };
                        self.apply_replan_result(&result);
                    }
                    Err(e) => {
                        tracing::error!(
                            "[orchestrate] retry-same replan failed for {plan_id}: {e}"
                        );
                        self.record_replan_episode(
                            plan_id,
                            &task_id,
                            task_def.as_ref(),
                            ReplanStrategy::RetrySame,
                            failure_count,
                            &[],
                            false,
                            Some(e.to_string()),
                        )
                        .await;
                    }
                }
            }
            ReplanStrategy::RetryWithEscalation => {
                if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
                    if let Err(e) =
                        tracker.set_task_model_hint(&task_id, Some(escalate_model.clone()))
                    {
                        tracing::error!(
                            "[orchestrate] failed to persist escalated model hint for {plan_id}:{task_id}: {e}"
                        );
                    }
                }
                let prompt = format!(
                    "{failure_summary}\n\n\
                     Retry the same task with the error context above using a stronger model. \
                     Preserve the task boundary and fix the root cause."
                );
                match self
                    .dispatch_agent_with(
                        plan_id,
                        AgentRole::Strategist,
                        "replan",
                        Some(prompt),
                        Some(escalate_model.clone()),
                        None,
                        None,
                    )
                    .await
                {
                    Ok(_) => {
                        tracing::info!(
                            "[orchestrate] replan with escalation completed for {plan_id}"
                        );
                        self.record_replan_episode(
                            plan_id,
                            &task_id,
                            task_def.as_ref(),
                            ReplanStrategy::RetryWithEscalation,
                            failure_count,
                            &[],
                            true,
                            None,
                        )
                        .await;
                        if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
                            tracker.gate_failure_count = 0;
                        }
                        let result = ReplanResult::RetryWithEscalation {
                            plan_id: plan_id.to_string(),
                            task_id: task_id.clone(),
                            escalated_model: escalate_model.clone(),
                        };
                        self.apply_replan_result(&result);
                    }
                    Err(e) => {
                        tracing::error!("[orchestrate] escalated replan failed for {plan_id}: {e}");
                        self.record_replan_episode(
                            plan_id,
                            &task_id,
                            task_def.as_ref(),
                            ReplanStrategy::RetryWithEscalation,
                            failure_count,
                            &[],
                            false,
                            Some(e.to_string()),
                        )
                        .await;
                    }
                }
            }
            ReplanStrategy::Decompose => {
                let tasks_path = plan_dir.join("tasks.toml");
                let Some(tasks_snapshot) = self
                    .task_trackers
                    .get(plan_id)
                    .map(|tracker| tracker.tasks_file.clone())
                else {
                    tracing::warn!(
                        "[orchestrate] decomposition requested for unknown plan {plan_id}"
                    );
                    self.record_replan_episode(
                        plan_id,
                        &task_id,
                        task_def.as_ref(),
                        ReplanStrategy::Decompose,
                        failure_count,
                        &[],
                        false,
                        Some("decomposition requested for unknown plan".to_string()),
                    )
                    .await;
                    return;
                };
                let Some(original_task) = tasks_snapshot
                    .tasks
                    .iter()
                    .find(|task| task.id == task_id)
                    .cloned()
                else {
                    tracing::warn!(
                        "[orchestrate] decomposition requested for missing task {task_id} in {plan_id}"
                    );
                    self.record_replan_episode(
                        plan_id,
                        &task_id,
                        task_def.as_ref(),
                        ReplanStrategy::Decompose,
                        failure_count,
                        &[],
                        false,
                        Some(format!(
                            "decomposition requested for missing task {task_id} in {plan_id}"
                        )),
                    )
                    .await;
                    return;
                };
                let gate_report = self.gate_failure_report(plan_id);
                let system_prompt =
                    crate::plan_generate::build_generator_system_prompt(&self.workdir);
                let prompt = self.build_decompose_prompt(
                    plan_id,
                    &tasks_snapshot,
                    &original_task,
                    &failure_summary,
                    &gate_report,
                );
                match self
                    .dispatch_agent_with(
                        plan_id,
                        AgentRole::Strategist,
                        &task_id,
                        Some(prompt),
                        Some(architectural_model),
                        None,
                        Some(system_prompt),
                    )
                    .await
                {
                    Ok(result) => {
                        let response_text = match result.output.body.as_text() {
                            Ok(text) => text,
                            Err(e) => {
                                tracing::error!(
                                    "[orchestrate] decomposition agent returned non-text output for {plan_id}: {e}"
                                );
                                self.record_replan_episode(
                                    plan_id,
                                    &task_id,
                                    Some(&original_task),
                                    ReplanStrategy::Decompose,
                                    failure_count,
                                    &[],
                                    false,
                                    Some(e.to_string()),
                                )
                                .await;
                                return;
                            }
                        };

                        let parsed = match crate::task_parser::TasksFile::parse_agent_output(
                            response_text,
                        ) {
                            Ok(tf) => tf,
                            Err(e) => {
                                tracing::error!(
                                    "[orchestrate] failed to parse decomposition output for {plan_id}: {e}"
                                );
                                self.record_replan_episode(
                                    plan_id,
                                    &task_id,
                                    Some(&original_task),
                                    ReplanStrategy::Decompose,
                                    failure_count,
                                    &[],
                                    false,
                                    Some(e.to_string()),
                                )
                                .await;
                                return;
                            }
                        };

                        if !(2..=3).contains(&parsed.tasks.len()) {
                            tracing::error!(
                                "[orchestrate] decomposition output for {plan_id} produced {} tasks, expected 2-3",
                                parsed.tasks.len()
                            );
                            self.record_replan_episode(
                                plan_id,
                                &task_id,
                                Some(&original_task),
                                ReplanStrategy::Decompose,
                                failure_count,
                                &[],
                                false,
                                Some(format!(
                                    "decomposition produced {} tasks, expected 2-3",
                                    parsed.tasks.len()
                                )),
                            )
                            .await;
                            return;
                        }

                        let existing_ids: HashSet<String> = tasks_snapshot
                            .tasks
                            .iter()
                            .map(|task| task.id.clone())
                            .collect();
                        let mut new_tasks = parsed.tasks;
                        let mut new_ids = HashSet::new();
                        for task in &mut new_tasks {
                            if task.id == task_id || existing_ids.contains(&task.id) {
                                tracing::error!(
                                    "[orchestrate] decomposition output reused existing task id {} for {plan_id}",
                                    task.id
                                );
                                self.record_replan_episode(
                                    plan_id,
                                    &task_id,
                                    Some(&original_task),
                                    ReplanStrategy::Decompose,
                                    failure_count,
                                    &[],
                                    false,
                                    Some(format!(
                                        "decomposition reused existing task id {}",
                                        task.id
                                    )),
                                )
                                .await;
                                return;
                            }
                            if !new_ids.insert(task.id.clone()) {
                                tracing::error!(
                                    "[orchestrate] decomposition output duplicated task id {} for {plan_id}",
                                    task.id
                                );
                                self.record_replan_episode(
                                    plan_id,
                                    &task_id,
                                    Some(&original_task),
                                    ReplanStrategy::Decompose,
                                    failure_count,
                                    &[],
                                    false,
                                    Some(format!("decomposition duplicated task id {}", task.id)),
                                )
                                .await;
                                return;
                            }
                            if task.depends_on.iter().any(|dep| dep == &task_id) {
                                tracing::error!(
                                    "[orchestrate] decomposition task {} still depends on failed task {task_id}",
                                    task.id
                                );
                                self.record_replan_episode(
                                    plan_id,
                                    &task_id,
                                    Some(&original_task),
                                    ReplanStrategy::Decompose,
                                    failure_count,
                                    &[],
                                    false,
                                    Some(format!(
                                        "decomposition task {} still depends on failed task {task_id}",
                                        task.id
                                    )),
                                )
                                .await;
                                return;
                            }
                            task.status = "ready".to_string();
                            task.split_into = None;
                        }
                        let resulting_subtasks = new_tasks.clone();

                        let terminal_ids: Vec<String> = new_tasks
                            .iter()
                            .filter(|task| {
                                !new_tasks
                                    .iter()
                                    .any(|other| other.depends_on.iter().any(|dep| dep == &task.id))
                            })
                            .map(|task| task.id.clone())
                            .collect();
                        if terminal_ids.is_empty() {
                            tracing::error!(
                                "[orchestrate] decomposition output for {plan_id} produced no terminal subtasks"
                            );
                            self.record_replan_episode(
                                plan_id,
                                &task_id,
                                Some(&original_task),
                                ReplanStrategy::Decompose,
                                failure_count,
                                &resulting_subtasks,
                                false,
                                Some("decomposition produced no terminal subtasks".to_string()),
                            )
                            .await;
                            return;
                        }

                        let mut rewritten_tasks = tasks_snapshot.clone();
                        let Some(original_index) = rewritten_tasks
                            .tasks
                            .iter()
                            .position(|task| task.id == task_id)
                        else {
                            tracing::error!(
                                "[orchestrate] original task {task_id} disappeared before decomposition rewrite"
                            );
                            self.record_replan_episode(
                                plan_id,
                                &task_id,
                                Some(&original_task),
                                ReplanStrategy::Decompose,
                                failure_count,
                                &resulting_subtasks,
                                false,
                                Some(
                                    "original task disappeared before decomposition rewrite"
                                        .to_string(),
                                ),
                            )
                            .await;
                            return;
                        };

                        let mut original = original_task.clone();
                        original.status = "skipped".to_string();
                        original.split_into =
                            Some(new_tasks.iter().map(|task| task.id.clone()).collect());

                        rewritten_tasks.tasks.remove(original_index);
                        rewritten_tasks.tasks.insert(original_index, original);
                        for (offset, task) in new_tasks.into_iter().enumerate() {
                            rewritten_tasks
                                .tasks
                                .insert(original_index + 1 + offset, task);
                        }

                        for task in &mut rewritten_tasks.tasks {
                            if task.id == task_id || new_ids.contains(&task.id) {
                                continue;
                            }
                            let mut rewritten_deps = Vec::with_capacity(task.depends_on.len());
                            for dep in &task.depends_on {
                                if dep == &task_id {
                                    for terminal_id in &terminal_ids {
                                        if !rewritten_deps.contains(terminal_id) {
                                            rewritten_deps.push(terminal_id.clone());
                                        }
                                    }
                                } else if !rewritten_deps.contains(dep) {
                                    rewritten_deps.push(dep.clone());
                                }
                            }
                            task.depends_on = rewritten_deps;
                        }

                        rewritten_tasks.meta.plan = plan_id.to_string();
                        rewritten_tasks.meta.total = rewritten_tasks.tasks.len() as u32;
                        rewritten_tasks.meta.done = (rewritten_tasks
                            .tasks
                            .iter()
                            .filter(|task| {
                                task.status.eq_ignore_ascii_case("skipped")
                                    || task.status.eq_ignore_ascii_case("done")
                            })
                            .count()) as u32;
                        rewritten_tasks.meta.status = "ready".to_string();

                        let rendered = match toml::to_string_pretty(&rewritten_tasks) {
                            Ok(text) => text,
                            Err(e) => {
                                tracing::error!(
                                    "[orchestrate] failed to serialize decomposed tasks for {plan_id}: {e}"
                                );
                                self.record_replan_episode(
                                    plan_id,
                                    &task_id,
                                    Some(&original_task),
                                    ReplanStrategy::Decompose,
                                    failure_count,
                                    &resulting_subtasks,
                                    false,
                                    Some(e.to_string()),
                                )
                                .await;
                                return;
                            }
                        };

                        if let Err(e) = std::fs::write(&tasks_path, rendered) {
                            tracing::error!(
                                "[orchestrate] failed to write decomposed tasks for {plan_id}: {e}"
                            );
                            self.record_replan_episode(
                                plan_id,
                                &task_id,
                                Some(&original_task),
                                ReplanStrategy::Decompose,
                                failure_count,
                                &resulting_subtasks,
                                false,
                                Some(e.to_string()),
                            )
                            .await;
                            return;
                        }

                        tracing::info!(
                            "[orchestrate] decomposition replan completed for {plan_id} with subtasks {:?}",
                            rewritten_tasks
                                .tasks
                                .iter()
                                .filter(|task| new_ids.contains(&task.id))
                                .map(|task| task.id.clone())
                                .collect::<Vec<_>>()
                        );

                        if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
                            if let Err(e) = tracker.reload_tasks_file() {
                                tracing::error!(
                                    "[orchestrate] failed to reload decomposed tasks for {plan_id}: {e}"
                                );
                            }
                            tracker.gate_failure_count = 0;
                            tracker.last_gate_failure = None;
                            tracker.last_gate_failure_phase = None;
                            tracker.last_impl_task_id = None;
                            tracker.last_impl_model_slug = None;
                        }
                        let result = ReplanResult::Decompose {
                            plan_id: plan_id.to_string(),
                            task_id: task_id.clone(),
                            new_task_ids: resulting_subtasks
                                .iter()
                                .map(|task| task.id.clone())
                                .collect(),
                        };
                        self.apply_replan_result(&result);
                        self.record_replan_episode(
                            plan_id,
                            &task_id,
                            Some(&original_task),
                            ReplanStrategy::Decompose,
                            failure_count,
                            &resulting_subtasks,
                            true,
                            None,
                        )
                        .await;
                    }
                    Err(e) => {
                        tracing::error!(
                            "[orchestrate] decomposition replan failed for {plan_id}: {e}"
                        );
                        self.record_replan_episode(
                            plan_id,
                            &task_id,
                            Some(&original_task),
                            ReplanStrategy::Decompose,
                            failure_count,
                            &[],
                            false,
                            Some(e.to_string()),
                        )
                        .await;
                    }
                }
            }
            ReplanStrategy::Skip => {
                let skip_reason =
                    format!("skipped after {failure_count} consecutive gate failures");
                tracing::warn!(
                    plan_id = %plan_id,
                    task_id = %task_id,
                    skip_reason = %skip_reason,
                    "marking task skipped"
                );
                if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
                    tracker.mark_skipped(&task_id);
                    tracker.gate_failure_count = 0;
                    tracker.last_gate_failure = None;
                    tracker.last_gate_failure_phase = None;
                }
                self.emit_execution_event(
                    plan_id,
                    crate::serve::events::ExecutionEvent::TaskCompleted {
                        task_id: task_id.clone(),
                        outcome: "skipped".to_string(),
                    },
                );

                let mut ep = Episode::new("Strategist", &task_id).failed(skip_reason);
                ep.kind = "replan".to_string();
                self.stamp_episode_affect(&mut ep, "task_skipped", None);
                ep.input_signal_hash = plan_id.to_string();
                ep.output_signal_hash = format!("{plan_id}:{task_id}:skipped");
                ep.extra.insert(
                    "replan_strategy".to_string(),
                    serde_json::json!(ReplanStrategy::Skip),
                );
                ep.extra.insert(
                    "failure_count".to_string(),
                    serde_json::json!(failure_count),
                );
                ep.extra.insert(
                    "failure_phase".to_string(),
                    serde_json::json!(failure_phase),
                );
                ep.extra.insert(
                    "failure_context".to_string(),
                    serde_json::json!(failure_context),
                );
                if let Some(task) = task_def.as_ref() {
                    ep.extra.insert(
                        "original_task".to_string(),
                        serde_json::json!({
                            "id": &task.id,
                            "title": &task.title,
                            "status": &task.status,
                            "tier": &task.tier,
                            "role": &task.role,
                            "depends_on": &task.depends_on,
                            "files": &task.files,
                            "replan_strategy": task.replan_strategy,
                        }),
                    );
                }
                ep.extra.insert(
                    "attempt_number".to_string(),
                    serde_json::json!(failure_count),
                );
                ep.extra.insert(
                    "resulting_subtasks".to_string(),
                    serde_json::Value::Array(Vec::new()),
                );
                let input = self.enrich_completed_run(
                    ep,
                    plan_id,
                    &task_id,
                    "Strategist",
                    &current_model,
                    None,
                    failure_count,
                );
                self.record_and_check_learning(input, plan_id).await;

                let result = ReplanResult::Skip {
                    plan_id: plan_id.to_string(),
                    task_id: task_id.clone(),
                };
                self.apply_replan_result(&result);

                if self
                    .task_trackers
                    .get(plan_id)
                    .is_some_and(TaskTracker::terminal_task_ratio_exceeds_half)
                {
                    tracing::warn!(
                        "[orchestrate] {plan_id}: skipped/failed tasks exceeded 50%, regenerating plan"
                    );
                    self.replan_plan(plan_id, &task_id, &failure_summary, &architectural_model)
                        .await;
                }
            }
            ReplanStrategy::RegeneratePlan => {
                self.replan_plan(plan_id, &task_id, &failure_summary, &architectural_model)
                    .await;
            }
        }
    }

    async fn replan_plan(
        &mut self,
        plan_id: &str,
        task_id: &str,
        failure_summary: &str,
        model: &str,
    ) {
        let Some(tracker_snapshot) = self.task_trackers.get(plan_id) else {
            tracing::warn!("[orchestrate] regenerate requested for unknown plan {plan_id}");
            return;
        };

        let plan_dir = tracker_snapshot._plan_dir.clone();
        let old_tasks = tracker_snapshot.tasks_file.clone();
        let completed_tasks = tracker_snapshot.completed_task_defs();
        let completed_task_ids = tracker_snapshot.completed.clone();
        let replan_attempt_number = tracker_snapshot.gate_failure_count;
        let original_task = old_tasks
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .cloned();
        let plan_slug_candidates = [
            old_tasks.meta.plan.trim().to_string(),
            plan_id.to_string(),
            plan_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string(),
        ];
        let Some(prd_path) = plan_slug_candidates
            .iter()
            .find_map(|slug| (!slug.trim().is_empty()).then(|| find_prd_path(&self.workdir, slug)))
            .flatten()
        else {
            tracing::error!(
                "[orchestrate] plan regeneration failed for {plan_id}: could not find matching PRD"
            );
            self.record_replan_episode(
                plan_id,
                task_id,
                original_task.as_ref(),
                ReplanStrategy::RegeneratePlan,
                replan_attempt_number,
                &[],
                false,
                Some("could not find matching PRD".to_string()),
            )
            .await;
            return;
        };

        let prd_slug = prd_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(plan_id);
        let prd_content = match std::fs::read_to_string(&prd_path) {
            Ok(content) => content,
            Err(e) => {
                tracing::error!(
                    "[orchestrate] failed to read PRD for {plan_id} at {}: {e}",
                    prd_path.display()
                );
                self.record_replan_episode(
                    plan_id,
                    task_id,
                    original_task.as_ref(),
                    ReplanStrategy::RegeneratePlan,
                    replan_attempt_number,
                    &[],
                    false,
                    Some(e.to_string()),
                )
                .await;
                return;
            }
        };

        let tasks_path = plan_dir.join("tasks.toml");
        let existing_tasks = std::fs::read_to_string(&tasks_path).unwrap_or_default();
        let system_prompt =
            crate::plan_generate::build_generation_prompt(&self.workdir, &prd_content, "prd");
        let prompt = format!(
            "{failure_summary}\n\n\
             Regenerate the implementation plan from the PRD at {}.\
             This is the same work as `roko prd plan {prd_slug}`.\
             Preserve these completed task IDs when you rebuild the plan: {completed_task_ids:?}\n\n\
             Regenerate the plan at .roko/plans/{plan_id}/tasks.toml so it can continue \
             from the failed task '{task_id}'. Keep only genuinely new tasks, skip \
             any task whose title matches a completed task, and rewrite the file in place.\n\n\
             ## PRD source\n\
             {prd_content}",
            prd_path.display()
        );

        match self
            .dispatch_agent_with(
                plan_id,
                AgentRole::Strategist,
                task_id,
                Some(prompt),
                Some(model.to_string()),
                None,
                Some(system_prompt),
            )
            .await
        {
            Ok(_) => {
                let regenerated_tasks = match crate::task_parser::TasksFile::parse(&tasks_path) {
                    Ok(tasks) => tasks,
                    Err(e) => {
                        tracing::error!(
                            "[orchestrate] failed to parse regenerated tasks for {plan_id}: {e}"
                        );
                        self.record_replan_episode(
                            plan_id,
                            task_id,
                            original_task.as_ref(),
                            ReplanStrategy::RegeneratePlan,
                            replan_attempt_number,
                            &[],
                            false,
                            Some(e.to_string()),
                        )
                        .await;
                        if let Err(write_err) = std::fs::write(&tasks_path, existing_tasks) {
                            tracing::error!(
                                "[orchestrate] failed to restore original tasks for {plan_id}: {write_err}"
                            );
                        }
                        return;
                    }
                };
                let regenerated_subtasks = regenerated_tasks.tasks.clone();

                let merged_tasks = merge_regenerated_plan(
                    plan_id,
                    &old_tasks,
                    regenerated_tasks,
                    &completed_tasks,
                );
                let rendered = match toml::to_string_pretty(&merged_tasks) {
                    Ok(text) => text,
                    Err(e) => {
                        tracing::error!(
                            "[orchestrate] failed to serialize merged regenerated plan for {plan_id}: {e}"
                        );
                        self.record_replan_episode(
                            plan_id,
                            task_id,
                            original_task.as_ref(),
                            ReplanStrategy::RegeneratePlan,
                            replan_attempt_number,
                            &regenerated_subtasks,
                            false,
                            Some(e.to_string()),
                        )
                        .await;
                        if let Err(write_err) = std::fs::write(&tasks_path, existing_tasks) {
                            tracing::error!(
                                "[orchestrate] failed to restore original tasks for {plan_id}: {write_err}"
                            );
                        }
                        return;
                    }
                };

                if let Err(e) = std::fs::write(&tasks_path, rendered) {
                    tracing::error!(
                        "[orchestrate] failed to write merged regenerated tasks for {plan_id}: {e}"
                    );
                    self.record_replan_episode(
                        plan_id,
                        task_id,
                        original_task.as_ref(),
                        ReplanStrategy::RegeneratePlan,
                        replan_attempt_number,
                        &regenerated_subtasks,
                        false,
                        Some(e.to_string()),
                    )
                    .await;
                    if let Err(write_err) = std::fs::write(&tasks_path, existing_tasks) {
                        tracing::error!(
                            "[orchestrate] failed to restore original tasks for {plan_id}: {write_err}"
                        );
                    }
                    return;
                }

                tracing::info!("[orchestrate] plan regeneration completed for {plan_id}");
                self.record_replan_episode(
                    plan_id,
                    task_id,
                    original_task.as_ref(),
                    ReplanStrategy::RegeneratePlan,
                    replan_attempt_number,
                    &regenerated_subtasks,
                    true,
                    None,
                )
                .await;
                if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
                    if let Err(e) = tracker.reload_tasks_file() {
                        tracing::error!(
                            "[orchestrate] failed to reload regenerated tasks for {plan_id}: {e}"
                        );
                    }
                    tracker.gate_failure_count = 0;
                    tracker.last_gate_failure = None;
                    tracker.last_gate_failure_phase = None;
                    tracker.last_impl_task_id = None;
                    tracker.last_impl_model_slug = None;
                }
                let result = ReplanResult::RegeneratePlan {
                    plan_id: plan_id.to_string(),
                    task_id: task_id.to_string(),
                    new_task_ids: regenerated_subtasks
                        .iter()
                        .map(|task| task.id.clone())
                        .collect(),
                };
                self.apply_replan_result(&result);
            }
            Err(e) => {
                tracing::error!("[orchestrate] plan regeneration failed for {plan_id}: {e}");
                self.record_replan_episode(
                    plan_id,
                    task_id,
                    original_task.as_ref(),
                    ReplanStrategy::RegeneratePlan,
                    replan_attempt_number,
                    &[],
                    false,
                    Some(e.to_string()),
                )
                .await;
            }
        }
    }

    /// Select the next tier up in the haiku → sonnet → opus chain.
    fn next_tier_model_slug(&self, current_model: &str) -> String {
        if current_model.contains("haiku") {
            self.config
                .agent
                .tier_models
                .get("focused")
                .cloned()
                .unwrap_or_else(|| "claude-sonnet-4-6".into())
        } else if current_model.contains("sonnet") {
            self.config
                .agent
                .tier_models
                .get("architectural")
                .cloned()
                .unwrap_or_else(|| "claude-opus-4-6".into())
        } else {
            self.config
                .agent
                .tier_models
                .get("architectural")
                .cloned()
                .unwrap_or_else(|| current_model.to_string())
        }
    }

    fn primary_failed_gate_name_from_results<'a>(
        verdicts: &'a [&'a GateResult],
    ) -> Option<&'a str> {
        verdicts
            .iter()
            .find(|v| !v.passed && matches!(v.gate_name.as_str(), "compile" | "test" | "clippy"))
            .map(|v| v.gate_name.as_str())
            .or_else(|| {
                verdicts
                    .iter()
                    .find(|v| !v.passed)
                    .map(|v| v.gate_name.as_str())
            })
    }

    fn format_gate_failure_context(verdicts: &[Verdict]) -> String {
        let mut sections = Vec::new();
        for verdict in verdicts.iter().filter(|v| !v.passed) {
            let mut section = format!("{}: {}", verdict.gate, verdict.reason.trim());
            if let Some(digest) = verdict
                .error_digest
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                section.push_str("\n\nerror_digest:\n");
                section.push_str(digest);
            }
            if let Some(detail) = verdict
                .detail
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                section.push_str("\n\nstderr/stdout:\n");
                section.push_str(&detail.chars().take(4000).collect::<String>());
            }
            sections.push(section);
        }

        if sections.is_empty() {
            String::new()
        } else {
            sections.join("\n\n---\n\n")
        }
    }

    /// Extract the most relevant compile failure summary from a gate run.
    ///
    /// The `compile_fail_repeat` watcher keys off `Kind::CompileDiagnostic`
    /// signals, so we emit a normalized message whenever the compile gate
    /// fails. The watcher then compares the message across consecutive
    /// agent turns.
    fn compile_failure_message(verdicts: &[Verdict]) -> Option<String> {
        verdicts.iter().find_map(|verdict| {
            if verdict.passed || !verdict.gate.starts_with("compile") {
                return None;
            }

            let message = verdict
                .error_digest
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| verdict.reason.trim());

            (!message.is_empty()).then_some(message.to_owned())
        })
    }

    /// Record a failed task: episode log + mark failed in tracker.
    async fn record_task_failure(
        &mut self,
        plan_id: &str,
        task_id: &str,
        task_text: Option<&str>,
        selected_model: Option<&str>,
        error: &anyhow::Error,
        started: &std::time::Instant,
        result: Option<&AgentResult>,
    ) {
        let wall_ms = result
            .map(|r| r.usage.wall_ms)
            .unwrap_or_else(|| u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
        let task_def = self
            .task_trackers
            .get(plan_id)
            .and_then(|t| t.tasks_file.tasks.iter().find(|td| td.id == task_id))
            .cloned();
        let frequency = task_def
            .as_ref()
            .map_or(OperatingFrequency::Theta, |td| td.operating_frequency());
        if let Some(model) = selected_model {
            self.observe_cascade_router(plan_id, task_id, task_def.as_ref(), model, 0.0);
        }
        let mut ep = Episode::new("Implementer", task_id).failed(error.to_string());
        self.stamp_episode_affect(
            &mut ep,
            "task_failure",
            result.as_ref().map(|agent_result| &agent_result.output),
        );
        let task_strategy =
            self.stamp_task_strategy_metadata(&mut ep, plan_id, task_id, task_def.as_ref());
        ep.usage = match result {
            Some(result) => Usage {
                wall_ms,
                cost_usd: f64::from(result.usage.cost_usd),
                cost_usd_without_cache: f64::from(result.usage.cost_usd),
                input_tokens: u64::from(result.usage.input_tokens),
                output_tokens: u64::from(result.usage.output_tokens),
                cache_read_tokens: u64::from(result.usage.cache_read_tokens),
                cache_write_tokens: u64::from(result.usage.cache_create_tokens),
            },
            None => Usage {
                wall_ms,
                ..Usage::default()
            },
        };
        ep.input_signal_hash = plan_id.to_string();
        let model = selected_model
            .map(str::to_owned)
            .unwrap_or_else(|| self.effective_model());
        let input = self.enrich_completed_run(ep, plan_id, task_id, "Implementer", &model, None, 1);
        self.record_and_check_learning(input, plan_id).await;
        self.emit_failure_efficiency_event(
            plan_id,
            task_id,
            "Implementer",
            &model,
            frequency,
            wall_ms,
            Vec::new(),
            "retry_same",
            1,
        )
        .await;
        self.record_crate_familiarity(plan_id, task_id, task_def.as_ref(), false);
        if let Some(request) = self
            .build_failed_skill_request(plan_id, task_id, task_text, selected_model)
            .await
            && let Some(skill) = self.skill_library.record_failure(request).await
        {
            tracing::info!(
                "[orchestrate] recorded failure pattern {} from plan {}",
                skill.name,
                plan_id
            );
        }
        if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
            tracker.failed.push(task_id.to_string());
        }

        // Emit observability trace event for the failed agent dispatch.
        self.emit_agent_trace(plan_id, task_id, false, wall_ms);

        // Emit a FailureTrace for agent dispatch errors.
        let trace_id = Self::trace_id_for(plan_id, task_id);
        let kind = if error.to_string().to_lowercase().contains("timeout") {
            FailureKind::Timeout
        } else {
            FailureKind::ToolHandlerError
        };
        let ft = FailureTrace::new(trace_id, TraceStep::Execute, kind, error.to_string());
        let event = ToolTraceEvent::Custom {
            name: "failure_trace".to_string(),
            data: serde_json::to_value(&ft).unwrap_or_default(),
            at_ms: now_unix_ms_i64(),
        };
        self.obs_sinks.trace_sink.append(trace_id, event);

        // Appraise task outcome for affect modulation.
        let _ = self.daimon.appraise(AffectEvent::TaskOutcome {
            task_id: task_id.to_string(),
            succeeded: false,
        });
        self.daimon.record_somatic_outcome(
            task_strategy,
            somatic_episode_hash(plan_id, task_id, "failure", &error.to_string()),
        );

        tracing::error!(
            plan_id = %plan_id,
            task_id = %task_id,
            duration_ms = wall_ms,
            error = ?error,
            "task failed"
        );
    }

    async fn build_failed_skill_request(
        &self,
        plan_id: &str,
        task_id: &str,
        task_text: Option<&str>,
        selected_model: Option<&str>,
    ) -> Option<SkillExtractionRequest> {
        let tracker = self.task_trackers.get(plan_id)?;
        let task_def = tracker
            .tasks_file
            .tasks
            .iter()
            .find(|task| task.id == task_id)?;
        let role = AgentRole::Implementer;
        let task_allowed_tools_csv = claude_task_tool_allowlist_with(
            role,
            task_def.allowed_tools.as_deref(),
            task_def.denied_tools.as_deref(),
            self.tool_registry.as_deref(),
        );
        let model = selected_model.map(str::to_owned).unwrap_or_else(|| {
            task_def.effective_model(
                self.config
                    .agent
                    .model
                    .as_deref()
                    .unwrap_or("claude-sonnet-4-6"),
                Some(&self.config.agent.tier_models),
            )
        });
        let prompt_hash = roko_core::ContentHash::of(
            build_system_prompt(
                role,
                plan_id,
                task_id,
                &task_allowed_tools_csv,
                Some(task_def),
            )
            .as_bytes(),
        )
        .to_hex();
        let task_text = task_text
            .map(str::to_owned)
            .unwrap_or_else(|| task_def.build_prompt(plan_id, &self.workdir));
        let symbols = extract_task_symbols(&task_text);

        let mut task_files = Vec::new();
        let mut seen_files = HashSet::new();
        if let Some(exec_dir) = self
            .worktrees
            .get(&format!("{plan_id}-{task_id}"))
            .map(|handle| handle.path)
            && let Ok(changed_files) = self.git_changed_files(&exec_dir).await
        {
            for file in changed_files {
                if seen_files.insert(file.clone()) {
                    task_files.push(file);
                }
            }
        }
        for file in &task_def.files {
            if seen_files.insert(file.clone()) {
                task_files.push(file.clone());
            }
        }

        let gate_results = vec![SkillGateResult::new("task_failure", false, 0.0)];

        Some(SkillExtractionRequest::new(
            task_files,
            task_def.tier.clone(),
            symbols,
            model,
            prompt_hash,
            gate_results,
        ))
    }
    ///
    /// Uses `TaskDef::build_fix_prompt` to produce a targeted prompt that includes
    /// the original task, the failing phase, and the error output. Selects the model
    /// based on error type: Haiku for compile errors (fast iteration), Sonnet for
    /// test/clippy failures (needs reasoning).
    async fn handle_autofix(&mut self, plan_id: &str) {
        let gate_context = self
            .task_trackers
            .get(plan_id)
            .and_then(|t| t.last_gate_failure.clone())
            .or_else(|| {
                self.executor
                    .plan_state(plan_id)
                    .and_then(|state| state.last_error.clone())
            })
            .unwrap_or_default();

        let gate_phase = self
            .task_trackers
            .get(plan_id)
            .and_then(|t| t.last_gate_failure_phase.clone())
            .unwrap_or_else(|| "unknown".into());

        let tracker = self.task_trackers.get(plan_id);
        let last_task_id = tracker.and_then(|t| t.last_impl_task_id.as_deref());
        let task_def = tracker.and_then(|t| {
            last_task_id.and_then(|tid| t.tasks_file.tasks.iter().find(|td| td.id == tid))
        });

        let fix_tier = if gate_phase == "compile" {
            "mechanical"
        } else {
            "focused"
        };
        let fix_model = self
            .config
            .agent
            .tier_models
            .get(fix_tier)
            .cloned()
            .unwrap_or_else(|| match fix_tier {
                "mechanical" => "claude-haiku-4-5".into(),
                _ => "claude-sonnet-4-6".into(),
            });

        let fix_prompt = if let Some(td) = task_def {
            let original_prompt = td.build_prompt(plan_id, &self.workdir);
            td.build_fix_prompt(&original_prompt, &gate_phase, &gate_context)
        } else {
            let truncated = gate_context.chars().take(4000).collect::<String>();
            format!(
                "Plan: {plan_id}\nTask: fix\n\n## Verification Failed\n\n\
                 Phase: {gate_phase}\n\n\
                 Error output:\n```\n{truncated}\n```\n\n\
                 Fix the issue and ensure all verification steps pass."
            )
        };

        if !gate_context.is_empty() {
            tracing::info!(
                "[orchestrate] AutoFix {plan_id}: gate failure phase={gate_phase} context ({} chars)",
                gate_context.len()
            );
        }

        let started = std::time::Instant::now();
        match self
            .dispatch_agent_with(
                plan_id,
                AgentRole::AutoFixer,
                "fix",
                Some(fix_prompt),
                Some(fix_model),
                None,
                None,
            )
            .await
        {
            Ok(result) => {
                *self.per_plan_agents.entry(plan_id.to_string()).or_default() += 1;
                self.agent_calls += 1;

                let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                let mut ep = Episode::new("AutoFixer", "fix").succeeded();
                ep.usage = Usage {
                    wall_ms,
                    cost_usd: f64::from(result.usage.cost_usd),
                    input_tokens: u64::from(result.usage.input_tokens),
                    output_tokens: u64::from(result.usage.output_tokens),
                    ..Usage::default()
                };
                self.stamp_episode_affect(&mut ep, "autofix", Some(&result.output));
                ep.input_signal_hash = plan_id.to_string();
                ep.output_signal_hash = result.output.id.to_string();
                let model = self.effective_model();
                let input =
                    self.enrich_completed_run(ep, plan_id, "fix", "AutoFixer", &model, None, 1);
                self.record_and_check_learning(input, plan_id).await;

                // Reset for retry: increment iteration, clear gate results
                if let Some(state) = self.executor.plan_state_mut(plan_id) {
                    state.reset_for_retry();
                }

                let event = ExecutorEvent::AutoFixDone;
                self.log_transition(plan_id, &event);
                self.apply_event_and_emit(plan_id, "fix", &event, "transitioned");
            }
            Err(e) => {
                tracing::error!("[orchestrate] AutoFix failed for {plan_id}: {e}");
                self.apply_event_and_emit(
                    plan_id,
                    "fix",
                    &ExecutorEvent::Fatal(format!("autofix failed: {e}")),
                    "failed",
                );
            }
        }
    }

    /// RegeneratingVerify phase: dispatch fixer with verify-specific context.
    async fn handle_regen_verify(&mut self, plan_id: &str) {
        let started = std::time::Instant::now();
        match self
            .dispatch_agent(plan_id, AgentRole::AutoFixer, "regen-verify")
            .await
        {
            Ok(result) => {
                *self.per_plan_agents.entry(plan_id.to_string()).or_default() += 1;
                self.agent_calls += 1;

                let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                let mut ep = Episode::new("AutoFixer", "regen-verify").succeeded();
                ep.usage = Usage {
                    wall_ms,
                    cost_usd: f64::from(result.usage.cost_usd),
                    input_tokens: u64::from(result.usage.input_tokens),
                    output_tokens: u64::from(result.usage.output_tokens),
                    ..Usage::default()
                };
                self.stamp_episode_affect(&mut ep, "regen_verify", Some(&result.output));
                ep.input_signal_hash = plan_id.to_string();
                ep.output_signal_hash = result.output.id.to_string();
                let model = self.effective_model();
                let input = self.enrich_completed_run(
                    ep,
                    plan_id,
                    "regen-verify",
                    "AutoFixer",
                    &model,
                    None,
                    1,
                );
                self.record_and_check_learning(input, plan_id).await;

                let event = ExecutorEvent::VerifyRegenDone;
                self.log_transition(plan_id, &event);
                if self.executor.apply_event(plan_id, &event).is_ok() {
                    self.emit_execution_event(
                        plan_id,
                        crate::serve::events::ExecutionEvent::TaskPhaseChanged {
                            task_id: "regen-verify".to_string(),
                            old_phase: "auto-fixing".to_string(),
                            new_phase: "verifying".to_string(),
                        },
                    );
                    self.finish_verify_round(plan_id).await;
                }
            }
            Err(e) => {
                tracing::error!("[orchestrate] RegenVerify failed for {plan_id}: {e}");
                self.apply_event_and_emit(
                    plan_id,
                    "regen-verify",
                    &ExecutorEvent::Fatal(format!("regen-verify failed: {e}")),
                    "failed",
                );
            }
        }
    }

    /// Run the task verification pipeline and advance the phase based on the result.
    async fn finish_verify_round(&mut self, plan_id: &str) {
        match self.run_plan_verify_steps(plan_id).await {
            Ok(()) => {
                if let Some(state) = self.executor.plan_state_mut(plan_id) {
                    state.last_error = None;
                }
                self.apply_event_and_emit(
                    plan_id,
                    "verify",
                    &ExecutorEvent::VerifyPassed,
                    "transitioned",
                );
            }
            Err((task_id, phase, command, error_output)) => {
                let msg =
                    format!("verify failed for {plan_id}/{task_id} in phase {phase}: {command}");
                let task_output_path = self
                    .workdir
                    .join(".roko")
                    .join("task-outputs")
                    .join(format!("{task_id}.txt"));
                let output_tail = std::fs::read_to_string(&task_output_path)
                    .ok()
                    .map(|text| tail_output_lines(&text, TASK_FAILURE_OUTPUT_TAIL_LINES));
                let error = with_task_failure_context(
                    anyhow!("{msg}; stderr/stdout:\n{error_output}"),
                    &task_id,
                    &phase,
                    &command,
                    output_tail.as_deref(),
                );
                tracing::error!(
                    plan_id = %plan_id,
                    task_id = %task_id,
                    phase = %phase,
                    gate = %command,
                    error = ?error,
                    "task verification failed"
                );
                self.event_log.append(
                    EventKind::ErrorOccurred,
                    serde_json::json!({
                        "plan_id": plan_id,
                        "task_id": task_id,
                        "phase": phase,
                        "command": command,
                        "error": format!("{error:#}"),
                    }),
                );
                if let Some(state) = self.executor.plan_state_mut(plan_id) {
                    state.last_error = Some(format!("{error:#}"));
                }
                self.apply_event_and_emit(
                    plan_id,
                    "verify",
                    &ExecutorEvent::VerifyFailed,
                    "transitioned",
                );
            }
        }
    }

    /// Reviewing phase: dispatch auditor using ReviewerTemplate, parse verdict.
    async fn handle_reviewing(&mut self, plan_id: &str) {
        let started = std::time::Instant::now();

        // Build review prompt from ReviewerTemplate with available context.
        let review_prompt = self.build_review_prompt(plan_id).await;

        match self
            .dispatch_agent_with(
                plan_id,
                AgentRole::Auditor,
                "review",
                Some(review_prompt),
                None,
                None,
                None,
            )
            .await
        {
            Ok(result) => {
                *self.per_plan_agents.entry(plan_id.to_string()).or_default() += 1;
                self.agent_calls += 1;

                let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                let output_text = result.output.body.as_text().unwrap_or_default().to_string();

                let mut approved = parse_review_verdict(&output_text);
                let drift_report = self
                    .task_trackers
                    .get(plan_id)
                    .and_then(|tracker| review_drift_report(&tracker.tasks_file, &output_text));
                if let Some(ref report) = drift_report {
                    if report.drifted() {
                        approved = false;
                    }
                }
                tracing::info!(
                    "[orchestrate] Review {plan_id}: verdict={} drift={}",
                    if approved { "approved" } else { "revise" },
                    drift_report
                        .as_ref()
                        .map(|r: &ReviewDriftReport| {
                            format!(
                                "{:.1}% ({}/{})",
                                r.coverage() * 100.0,
                                r.matched,
                                r.expected
                            )
                        })
                        .unwrap_or_else(|| "n/a".into())
                );

                let mut ep = Episode::new("Auditor", "review").succeeded();
                ep.usage = Usage {
                    wall_ms,
                    cost_usd: f64::from(result.usage.cost_usd),
                    input_tokens: u64::from(result.usage.input_tokens),
                    output_tokens: u64::from(result.usage.output_tokens),
                    ..Usage::default()
                };
                self.stamp_episode_affect(&mut ep, "review", Some(&result.output));
                ep.input_signal_hash = plan_id.to_string();
                ep.output_signal_hash = result.output.id.to_string();
                let model = self.effective_model();
                let input =
                    self.enrich_completed_run(ep, plan_id, "review", "Auditor", &model, None, 1);
                self.record_and_check_learning(input, plan_id).await;

                if approved {
                    let event = ExecutorEvent::ReviewApproved;
                    self.log_transition(plan_id, &event);
                    self.apply_event_and_emit(plan_id, "review", &event, "transitioned");
                } else {
                    // Store feedback and reset tracker for reimplementation
                    if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
                        tracker.review_feedback = Some(match drift_report {
                            Some(report) if report.drifted() => format!(
                                "Spec drift detected while reviewing task output.\n\
                                 Coverage: {:.1}% ({}/{})\n\
                                 Missing anchors: {}\n\n\
                                 Reviewer output:\n{}",
                                report.coverage() * 100.0,
                                report.matched,
                                report.expected,
                                report.missing.join(", "),
                                output_text
                            ),
                            _ => output_text.clone(),
                        });
                        tracker.reset_for_reimpl();
                    }
                    let event = ExecutorEvent::ReviewRejected;
                    self.log_transition(plan_id, &event);
                    self.apply_event_and_emit(plan_id, "review", &event, "transitioned");
                }
            }
            Err(e) => {
                // On infrastructure error, auto-approve (don't block pipeline)
                tracing::error!("[orchestrate] Review failed for {plan_id}: {e} — auto-approving");
                let event = ExecutorEvent::ReviewApproved;
                self.log_transition(plan_id, &event);
                self.apply_event_and_emit(plan_id, "review", &event, "transitioned");
            }
        }
    }

    /// DocRevision phase: dispatch scribe. Non-blocking — always advances.
    async fn handle_doc_revision(&mut self, plan_id: &str) {
        let started = std::time::Instant::now();

        // Build doc-revision prompt from ScribeTemplate with available context.
        let doc_prompt = self.build_doc_revision_prompt(plan_id).await;

        match self
            .dispatch_agent_with(
                plan_id,
                AgentRole::Scribe,
                "docs",
                Some(doc_prompt),
                None,
                None,
                None,
            )
            .await
        {
            Ok(result) => {
                *self.per_plan_agents.entry(plan_id.to_string()).or_default() += 1;
                self.agent_calls += 1;

                let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                let mut ep = Episode::new("Scribe", "docs").succeeded();
                ep.usage = Usage {
                    wall_ms,
                    cost_usd: f64::from(result.usage.cost_usd),
                    input_tokens: u64::from(result.usage.input_tokens),
                    output_tokens: u64::from(result.usage.output_tokens),
                    ..Usage::default()
                };
                self.stamp_episode_affect(&mut ep, "docs", Some(&result.output));
                ep.input_signal_hash = plan_id.to_string();
                ep.output_signal_hash = result.output.id.to_string();
                let model = self.effective_model();
                let input =
                    self.enrich_completed_run(ep, plan_id, "docs", "Scribe", &model, None, 1);
                self.record_and_check_learning(input, plan_id).await;
            }
            Err(e) => {
                tracing::error!(
                    "[orchestrate] DocRevision failed for {plan_id}: {e} — continuing (non-blocking)"
                );
            }
        }
        // Always advance regardless of success/failure
        let event = ExecutorEvent::DocRevisionDone;
        self.log_transition(plan_id, &event);
        self.apply_event_and_emit(plan_id, "docs", &event, "transitioned");
    }

    /// Generic fallback agent handler with retry loop + model escalation.
    /// Used for any role not handled by a dedicated phase handler.
    async fn handle_generic_agent(&mut self, plan_id: &str, role: AgentRole, task: &str) {
        let max_retries = 3u32;
        let escalation_models = ["claude-haiku-4-5", "claude-sonnet-4-6", "claude-opus-4-6"];
        let mut last_error = String::new();
        let mut succeeded = false;
        let started = std::time::Instant::now();

        for attempt in 0..=max_retries {
            if attempt > 0 {
                let current = self
                    .config
                    .agent
                    .model
                    .as_deref()
                    .unwrap_or("claude-sonnet-4-6");
                let current_idx = escalation_models
                    .iter()
                    .position(|m| *m == current)
                    .unwrap_or(1);
                let next_idx = (current_idx + attempt as usize).min(escalation_models.len() - 1);
                let escalated = escalation_models[next_idx];
                tracing::info!(
                    "[orchestrate] Retry {attempt}/{max_retries} for {plan_id}/{task} — escalating to {escalated} (error: {last_error})"
                );
            }

            match self.dispatch_agent(plan_id, role, task).await {
                Ok(ref result) => {
                    *self.per_plan_agents.entry(plan_id.to_string()).or_default() += 1;
                    self.agent_calls += 1;
                    let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    let mut ep = Episode::new(format!("{role:?}"), task).succeeded();
                    ep.usage = Usage {
                        wall_ms,
                        cost_usd: f64::from(result.usage.cost_usd),
                        input_tokens: u64::from(result.usage.input_tokens),
                        output_tokens: u64::from(result.usage.output_tokens),
                        ..Usage::default()
                    };
                    self.stamp_episode_affect(&mut ep, task, Some(&result.output));
                    ep.input_signal_hash = plan_id.to_string();
                    ep.output_signal_hash = result.output.id.to_string();
                    let model = self.effective_model();
                    let role_str = format!("{role:?}");
                    let input = self.enrich_completed_run(
                        ep,
                        plan_id,
                        task,
                        &role_str,
                        &model,
                        None,
                        attempt + 1,
                    );
                    if let Err(e) = self.learning.record_completed_run(input).await {
                        tracing::error!("[orchestrate] episode log failed: {e}");
                    }
                    let event = self.generic_completion_event(plan_id);
                    self.log_transition(plan_id, &event);
                    self.apply_event_and_emit(plan_id, task, &event, "transitioned");
                    succeeded = true;
                    break;
                }
                Err(e) => {
                    last_error = e.to_string();
                    if attempt == max_retries {
                        tracing::error!(
                            "[orchestrate] agent failed for {plan_id} after {max_retries} retries: {e}"
                        );
                        let wall_ms =
                            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                        let mut ep = Episode::new(format!("{role:?}"), task).failed(e.to_string());
                        ep.usage = Usage {
                            wall_ms,
                            ..Usage::default()
                        };
                        self.stamp_episode_affect(&mut ep, task, None);
                        ep.input_signal_hash = plan_id.to_string();
                        let model = self.effective_model();
                        let role_str = format!("{role:?}");
                        let input = self.enrich_completed_run(
                            ep,
                            plan_id,
                            task,
                            &role_str,
                            &model,
                            None,
                            attempt + 1,
                        );
                        self.record_and_check_learning(input, plan_id).await;
                        self.event_log.append(
                            EventKind::ErrorOccurred,
                            serde_json::json!({"plan_id": plan_id, "error": e.to_string(), "attempts": attempt + 1}),
                        );
                        self.apply_event_and_emit(
                            plan_id,
                            task,
                            &ExecutorEvent::Fatal(format!(
                                "agent error after {attempt} retries: {e}"
                            )),
                            "failed",
                        );
                    }
                }
            }
        }

        if !succeeded {
            tracing::error!("[orchestrate] All retries exhausted for {plan_id}/{task}");
        }
    }

    /// Ensure a TaskTracker exists for the given plan (lazy loading).
    fn ensure_task_tracker(&mut self, plan_id: &str) {
        if self.task_trackers.contains_key(plan_id) {
            return;
        }
        let plan_dir = plans_dir(&self.workdir).join(plan_id);
        let tasks_path = plan_dir.join("tasks.toml");
        if tasks_path.exists() {
            if let Ok(tf) = TasksFile::parse(&tasks_path) {
                self.task_trackers
                    .insert(plan_id.to_string(), TaskTracker::new(tf, plan_dir));
            }
        }
    }

    /// Log a phase transition event and emit a conductor signal (§7).
    fn log_transition(&mut self, plan_id: &str, event: &ExecutorEvent) {
        self.emit_server_event(crate::serve::events::ServerEvent::PhaseTransition {
            plan_id: plan_id.to_string(),
            from: String::new(),
            to: format!("{event:?}"),
        });
        self.event_log.append(
            EventKind::PhaseTransition,
            serde_json::json!({"plan_id": plan_id, "event": format!("{event:?}")}),
        );
        self.emit_conductor_signal(
            Kind::PlanPhase,
            serde_json::json!({
                "plan_id": plan_id,
                "event": format!("{event:?}"),
            }),
        );
    }

    fn all_terminal(&self, plan_ids: &[String]) -> bool {
        plan_ids.iter().all(|id| {
            self.executor.plan_state(id).is_none_or(|state| {
                state.is_terminal() || state.current_phase.kind() == PhaseKind::Done
            })
        })
    }

    /// Determine which completion event to fire for the generic agent handler.
    /// Only used by `handle_generic_agent` for non-standard roles.
    #[allow(clippy::match_same_arms)]
    fn generic_completion_event(&self, plan_id: &str) -> ExecutorEvent {
        let Some(state) = self.executor.plan_state(plan_id) else {
            return ExecutorEvent::Fatal("unknown plan".into());
        };
        match state.current_phase.kind() {
            PhaseKind::Enriching => ExecutorEvent::EnrichmentDone,
            PhaseKind::Implementing => ExecutorEvent::ImplementationDone,
            PhaseKind::AutoFixing => ExecutorEvent::AutoFixDone,
            PhaseKind::Verifying => ExecutorEvent::VerifyPassed,
            PhaseKind::Reviewing => ExecutorEvent::ReviewApproved,
            PhaseKind::DocRevision => ExecutorEvent::DocRevisionDone,
            PhaseKind::RegeneratingVerify => ExecutorEvent::VerifyRegenDone,
            _ => ExecutorEvent::ImplementationDone,
        }
    }

    /// Compose a prompt for the given task/role and run the agent.
    ///
    /// If a `tasks.toml` exists for this plan, the task is looked up by ID
    /// to get tier-based model selection, surgical context, and per-task
    /// verification. Falls back to the generic prompt if no tasks.toml exists
    /// or the task ID isn't found.
    async fn dispatch_agent(
        &mut self,
        plan_id: &str,
        role: AgentRole,
        task: &str,
    ) -> Result<AgentResult> {
        self.dispatch_agent_with(plan_id, role, task, None, None, None, None)
            .await
    }

    /// Build the per-task budget ledger key used for cumulative spend tracking.
    fn task_budget_key(plan_id: &str, task: &str) -> String {
        format!("{plan_id}::{task}")
    }

    /// Return the cumulative spend recorded for a plan/task dispatch key.
    fn task_spent(&self, plan_id: &str, task: &str) -> f64 {
        self.task_costs
            .get(&Self::task_budget_key(plan_id, task))
            .copied()
            .unwrap_or(0.0)
    }

    /// Record spend against a plan/task dispatch key.
    fn add_task_spend(&mut self, plan_id: &str, task: &str, cost: f64) {
        *self
            .task_costs
            .entry(Self::task_budget_key(plan_id, task))
            .or_insert(0.0) += cost;
    }

    /// Emit a warning once a plan crosses the configured budget threshold.
    fn warn_plan_budget_pressure(&mut self, plan_id: &str, plan_spent: f64) {
        let budget = &self.config.budget;
        let warn_threshold = budget.warn_threshold_usd();
        if budget.max_plan_usd > 0.0 && plan_spent >= warn_threshold {
            let max_plan_usd = budget.max_plan_usd;
            let warn_at_percent = budget.warn_at_percent;
            let percent_used = (plan_spent / budget.max_plan_usd) * 100.0;
            tracing::warn!(
                plan_id,
                plan_spent,
                max_plan_usd,
                warn_at_percent,
                "[budget] plan {plan_id} has consumed {:.0}% of budget (${plan_spent:.2}/${max_plan_usd:.2})",
                percent_used,
            );
            self.emit_conductor_signal(
                Kind::Custom("cost-pressure".into()),
                serde_json::json!({
                    "plan_id": plan_id,
                    "plan_spent": plan_spent,
                    "max_plan_usd": max_plan_usd,
                    "warn_at_percent": warn_at_percent,
                    "percent_used": percent_used,
                }),
            );
        }
    }

    /// Abort before dispatch if the cumulative task budget is already exhausted.
    fn ensure_task_budget_available(&self, plan_id: &str, task: &str) -> Result<()> {
        let task_spent = self.task_spent(plan_id, task);
        let max_task_usd = self.config.budget.max_task_usd;
        if task_spent >= max_task_usd {
            return Err(anyhow!(
                "task {plan_id}/{task} budget exhausted: ${task_spent:.2} >= max_task_usd ${max_task_usd:.2}"
            ));
        }
        Ok(())
    }

    /// Acquire the MCP servers needed for the current task.
    ///
    /// Returns the leased server names so the caller can release them after the
    /// agent completes.
    async fn acquire_task_mcp_servers(
        &mut self,
        task_def: Option<&crate::task_parser::TaskDef>,
    ) -> Vec<String> {
        let requested: Vec<String> = match task_def.and_then(|task| task.mcp_servers.as_ref()) {
            Some(servers) if !servers.is_empty() => servers.clone(),
            _ => self.mcp_server_names.clone(),
        };
        if requested.is_empty() {
            return Vec::new();
        }

        let missing: HashSet<String> = {
            let state = self.mcp_state.lock().await;
            requested
                .iter()
                .filter(|name| !state.clients.contains_key(*name))
                .cloned()
                .collect()
        };

        if !missing.is_empty() {
            let (clients, registry, discovered_names, discovered_configs) =
                Self::setup_mcp(&self.config, &self.workdir, Some(&missing)).await;
            if self.tool_registry.is_none() {
                self.tool_registry = registry;
            }

            let mut state = self.mcp_state.lock().await;
            for (name, client) in clients {
                state.clients.insert(name, client);
            }
            for (name, config) in discovered_configs {
                state.server_configs.insert(name, config);
            }
            for name in discovered_names {
                if !self.mcp_server_names.contains(&name) {
                    self.mcp_server_names.push(name);
                }
            }
        }

        let mut state = self.mcp_state.lock().await;
        for name in &requested {
            *state.ref_counts.entry(name.clone()).or_insert(0) += 1;
        }
        requested
    }

    /// Release MCP servers after the current task completes.
    async fn release_task_mcp_servers(&self, servers: &[String]) {
        if servers.is_empty() {
            return;
        }

        let mut state = self.mcp_state.lock().await;
        for name in servers {
            let should_remove = if let Some(count) = state.ref_counts.get_mut(name) {
                if *count > 0 {
                    *count -= 1;
                }
                *count == 0
            } else {
                false
            };

            if should_remove {
                state.ref_counts.remove(name);
                state.clients.remove(name);
            }
        }
    }

    /// Core agent dispatch with optional prompt, model, and system-prompt overrides.
    async fn dispatch_agent_with(
        &mut self,
        plan_id: &str,
        role: AgentRole,
        task: &str,
        prompt_override: Option<String>,
        model_override: Option<String>,
        exec_dir_override: Option<PathBuf>,
        system_prompt_override: Option<String>,
    ) -> Result<AgentResult> {
        self.ensure_dispatch_allowed(plan_id)?;
        let ctx = Context::now();
        let exec_dir = match exec_dir_override {
            Some(dir) => dir,
            None => self.ensure_plan_exec_dir(plan_id).await?,
        };
        let preexisting_changed_files = self.git_changed_files(&exec_dir).await.ok();

        // ── Budget check before dispatch ─────────────────────────────
        self.ensure_task_budget_available(plan_id, task)?;
        let plan_spent = self.plan_costs.get(plan_id).copied().unwrap_or(0.0);
        let budget = &self.config.budget;
        if plan_spent >= budget.max_plan_usd {
            return Err(anyhow!(
                "plan {plan_id} budget exhausted: ${plan_spent:.2} >= ${:.2} max",
                budget.max_plan_usd
            ));
        }
        self.warn_plan_budget_pressure(plan_id, plan_spent);

        // ── Try to load structured task definition ──────────────────
        let plan_dir = plans_dir(&self.workdir).join(plan_id);
        let tasks_toml = plan_dir.join("tasks.toml");
        let tasks_file = if tasks_toml.exists() {
            crate::task_parser::TasksFile::parse(&tasks_toml).ok()
        } else {
            None
        };
        let task_def = tasks_file
            .as_ref()
            .and_then(|tf| tf.tasks.iter().find(|t| t.id == task).cloned());
        let frequency = task_def
            .as_ref()
            .map_or(OperatingFrequency::Theta, |td| td.operating_frequency());
        let explicit_model_override = model_override;

        let mcp_lease = self.acquire_task_mcp_servers(task_def.as_ref()).await;

        // ── Build prompt: surgical (from TaskDef) or generic ────────
        // Also collect attribution keys for context feedback after the agent runs.
        let mut attribution_keys: Vec<(String, String)> = Vec::new();
        let (task_text, mut selected_model) = if let Some(override_prompt) = prompt_override {
            let model = explicit_model_override.clone().unwrap_or_else(|| {
                self.config
                    .agent
                    .model
                    .clone()
                    .unwrap_or_else(|| "claude-sonnet-4-6".into())
            });
            (override_prompt, model)
        } else if let Some(ref td) = task_def {
            let prompt = td.build_prompt(plan_id, &self.workdir);
            let model = explicit_model_override.clone().unwrap_or_else(|| {
                td.effective_model(
                    self.config
                        .agent
                        .model
                        .as_deref()
                        .unwrap_or("claude-sonnet-4-6"),
                    Some(&self.config.agent.tier_models),
                )
            });
            tracing::info!(
                "[orchestrate] Task {} tier={} model={} max_loc={:?} context={} verify={}",
                td.id,
                td.tier,
                model,
                td.max_loc,
                td.context.is_some(),
                td.verify.len(),
            );
            (prompt, model)
        } else {
            let text =
                format!("Plan: {plan_id}\nTask: {task}\n\nImplement the task described above.");
            let model = self
                .config
                .agent
                .model
                .clone()
                .unwrap_or_else(|| "claude-opus-4-6".into());
            (text, model)
        };
        let roko_config = match load_roko_config(&self.workdir) {
            Ok(config) => config,
            Err(err) => {
                tracing::warn!(
                    "[orchestrate] failed to load roko.toml for provider routing metadata: {err}"
                );
                RokoConfig::default()
            }
        };
        let model_providers = routing_model_provider_map(&roko_config);
        let pending_force_model_override =
            if task_def.is_some() && explicit_model_override.is_none() {
                self.force_model_override.take()
            } else {
                None
            };

        let requested_model = selected_model.clone();
        let mut routing_stage = "static".to_string();
        let mut routing_reason = "configured_default".to_string();
        let mut routing_explanation: Option<roko_learn::cascade_router::CascadeRouteExplanation> =
            None;
        let mut routing_log_store: Option<RoutingDecisionLogStore> = None;
        let mut routing_log_record: Option<RoutingDecisionLog> = None;

        // ── Adaptive model selection via CascadeRouter ───────────────
        let mut selected_model_experiment = None;
        if let Some(forced_model) = pending_force_model_override {
            tracing::warn!(
                forced_model = %forced_model,
                "applying pending cost-anomaly model override before routing"
            );
            selected_model = forced_model;
        } else if let Some(td) = task_def.as_ref() {
            let cascade_router = self.learning.cascade_router();
            let mut routing_ctx = cascade_routing_context(self, plan_id, task, role, Some(td));
            let load_snapshot = self.routing_load_snapshot().await;
            routing_ctx.conductor_load = load_snapshot.pressure;
            routing_ctx.active_agents = load_snapshot.active_agents;
            routing_ctx.ready_queue_depth = load_snapshot.ready_queue_depth;
            routing_ctx.max_queue_wait_hours = load_snapshot.max_queue_wait_hours;
            let routing_bias = {
                let mut signals = self.conductor_signals.clone();
                if let Ok(efficiency_signals) = load_efficiency_signals_sync(
                    &self.learning.paths().efficiency_jsonl,
                    self.executor.config().budget_usd,
                ) {
                    signals.extend(efficiency_signals);
                }
                let _ = self.conductor.decide(&signals, &Context::now());
                self.conductor.routing_bias()
            };
            if routing_bias.prefer_cheaper {
                routing_ctx.conductor_load = routing_ctx.conductor_load.max(0.85);
            }
            let cost_spike = CostsLog::at(self.learning.paths().costs_jsonl.clone())
                .is_cost_spike(0.50)
                .await
                .unwrap_or(false);
            let agent_id = format!("{role:?}");
            let all_model_slugs = cascade_router
                .linucb()
                .arm_stats()
                .into_iter()
                .map(|arm| arm.slug)
                .collect::<Vec<_>>();
            let healthy_models =
                self.learning
                    .healthy_model_slugs(&all_model_slugs, |model_slug| {
                        provider_id_for_routing_model(&roko_config, &model_providers, model_slug)
                    });
            let effective_models = roko_config.effective_models();
            let task_requirements = task_requirements_for_routing(
                Some(td),
                role,
                &task_text,
                &td.allowed_tools.clone().unwrap_or_default().join(","),
                effective_context_window_tokens(&self.config) as u64,
            );
            let healthy_models = {
                let fallback_models = healthy_models.clone();
                let mut ranked = healthy_models
                    .iter()
                    .filter_map(|slug| {
                        let reward = self.learning.local_reward_score("router", slug);
                        match effective_models.get(slug) {
                            Some(profile) => score_model_for_task(profile, &task_requirements).map(
                                |capability_score| (slug.clone(), capability_score + reward * 0.5),
                            ),
                            None => Some((slug.clone(), reward)),
                        }
                    })
                    .collect::<Vec<_>>();
                if ranked.is_empty() {
                    healthy_models
                } else {
                    let cascade_bias = cascade_routing_bias_from_conductor(&routing_bias);
                    cascade_router.apply_bias(&mut ranked, &cascade_bias);
                    cascade_router.apply_cost_pressure(&mut ranked, cost_spike);
                    let candidate_count = ranked.len();
                    ranked.retain(|(slug, score)| {
                        *score > 0.0
                            && (!cost_spike || !is_premium_model(slug))
                            && (!routing_bias
                                .deprioritize
                                .iter()
                                .any(|blocked| slug_matches(slug, blocked))
                                || candidate_count == 1)
                    });
                    if ranked.is_empty() {
                        fallback_models
                    } else {
                        ranked.sort_by(|a, b| {
                            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
                        });
                        ranked.into_iter().map(|(slug, _)| slug).collect()
                    }
                }
            };

            routing_explanation =
                Some(cascade_router.explain_route(&routing_ctx, Some(&healthy_models)));
            if let Some(explanation) = routing_explanation.as_ref() {
                routing_stage = explanation.stage.label().to_string();
                routing_reason = if cost_spike {
                    "cost_spike"
                } else if !routing_bias.deprioritize.is_empty() {
                    "conductor_deprioritize"
                } else if routing_bias.prefer_cheaper {
                    "conductor_prefer_cheaper"
                } else {
                    match explanation.stage {
                        roko_learn::cascade_router::CascadeStage::Static => "role_default",
                        roko_learn::cascade_router::CascadeStage::Confidence => {
                            "highest_confidence_score"
                        }
                        roko_learn::cascade_router::CascadeStage::Ucb => "highest_ucb_score",
                    }
                }
                .to_string();
            }
            let cfactor_snapshot = match self.learning.latest_cfactor().await {
                Ok(snapshot) => snapshot,
                Err(err) => {
                    tracing::debug!("[orchestrate] failed to read latest C-Factor: {err}");
                    None
                }
            };

            let experiment_store =
                ModelExperimentStore::load_or_new(&model_experiments_path(&self.workdir));
            if let Some((experiment_id, variant)) = experiment_store.assign_model_with_experiment(
                routing_ctx.role.label(),
                routing_ctx.task_category.label(),
            ) {
                tracing::info!(
                    experiment_id = %experiment_id,
                    variant_id = %variant.id,
                    model = %variant.slug,
                    "[orchestrate] model experiment override selected variant"
                );
                selected_model_experiment = Some(SelectedModelExperiment {
                    experiment_id,
                    variant_id: variant.id.clone(),
                    model_slug: variant.slug.clone(),
                });
                selected_model = variant.slug;
                routing_reason = "experiment_override".to_string();
            } else {
                match cascade_router.select_for_frequency_among(
                    frequency,
                    Some(&routing_ctx),
                    cfactor_snapshot.as_ref(),
                    Some(agent_id.as_str()),
                    &healthy_models,
                ) {
                    Some(model) => {
                        tracing::info!(
                            "[orchestrate] frequency={} model={} healthy_candidates={} (selected via cascade)",
                            frequency_label(frequency),
                            model.slug,
                            healthy_models.len()
                        );
                        selected_model = model.slug;
                    }
                    None => {
                        tracing::info!(
                            "[orchestrate] frequency={} (reactive; bypassing model selection)",
                            frequency_label(frequency)
                        );
                        routing_reason = "reactive_bypass".to_string();
                    }
                }
            }
        }

        // ── Budget guardrail before dispatch ───────────────────────
        let mut budget = BudgetGuardrail::new(
            self.config.budget.max_task_usd,
            self.config.budget.max_session_usd,
            self.config.budget.max_plan_usd,
            f64::from(self.config.budget.warn_at_percent) / 100.0,
        );
        let last_cost_usd = self.task_spent(plan_id, task);
        match budget.record_cost(last_cost_usd, "task") {
            BudgetAction::Block => {
                return Err(anyhow!(
                    "task {plan_id}/{task} budget exhausted: ${last_cost_usd:.2} >= max_task_usd ${:.2}",
                    self.config.budget.max_task_usd
                ));
            }
            BudgetAction::RouteToCheaper => {
                selected_model = mechanical_tier_model(&self.config)
                    .unwrap_or_else(|| "claude-haiku-4-5".into());
                routing_reason = "budget_guardrail".to_string();
            }
            BudgetAction::Warn { percent_used, .. } => {
                tracing::warn!(pct = percent_used, "budget warning");
            }
            _ => {}
        }

        // ── Dispatch-time skill hint from successful prior tasks ──────
        let prior_skills =
            select_prompt_skills(&self.skill_library, task_def.as_ref(), &task_text, 5);
        let playbook_context_section = match self.playbook.lookup(task).await {
            Ok(Some(playbook)) => {
                let playbook_text = render_playbook_context(&playbook);
                let playbook_text_len = playbook_text.len();
                Some((
                    PromptSection::new("playbook", playbook_text)
                        .with_priority(SectionPriority::Low)
                        .with_placement(Placement::Middle)
                        .with_hard_cap(1024),
                    playbook_text_len,
                ))
            }
            Ok(None) => None,
            Err(err) => {
                tracing::warn!("[orchestrate] failed to lookup playbook for task {task}: {err}");
                None
            }
        };

        let search_context_section = if let (Some(task), Some(search_client)) =
            (task_def.as_ref(), self.search_client.as_ref())
        {
            match enrich_task_context_with_search(task, search_client).await {
                Some(search_text) => {
                    let search_text_len = search_text.len();
                    Some((
                        PromptSection::new("external-research", search_text)
                            .with_priority(SectionPriority::Low)
                            .with_placement(Placement::Middle)
                            .with_hard_cap(2_048),
                        search_text_len,
                    ))
                }
                None => None,
            }
        } else {
            None
        };

        // ── Provider health check ────────────────────────────────────
        let selected_provider =
            provider_id_for_routing_model(&roko_config, &model_providers, &selected_model);
        let selected_model = if !self
            .learning
            .provider_health()
            .is_healthy(&selected_provider)
        {
            let fallback = self
                .config
                .agent
                .fallback_model
                .clone()
                .unwrap_or_else(|| "claude-sonnet-4-6".into());
            tracing::warn!(
                unhealthy_model = %selected_model,
                unhealthy_provider = %selected_provider,
                fallback_model = %fallback,
                "model marked unhealthy by ProviderHealthTracker, falling back"
            );
            routing_reason = "provider_unhealthy".to_string();
            fallback
        } else {
            selected_model
        };

        let task_strategy = self.current_task_strategy(plan_id, task, task_def.as_ref());
        let somatic_signal = self.daimon.query_somatic(task_strategy);
        self.emit_somatic_marker_fired_event(plan_id, task, &somatic_signal, "dispatch");

        let pre_daimon_model = selected_model.clone();
        let mut dispatch_params =
            DispatchParams::new(selected_model.clone(), frequency.turn_limit());
        dispatch_params.effort = self.config.agent.effort.clone();
        self.daimon
            .modulate_with_strategy(&mut dispatch_params, task_strategy);
        let selected_model = dispatch_params.model;
        let dispatch_turn_limit = dispatch_params.turn_limit;
        let dispatch_effort = dispatch_params.effort.clone();
        if selected_model != pre_daimon_model {
            routing_reason = "fallback".to_string();
        }
        if selected_model_experiment
            .as_ref()
            .is_some_and(|selection| selection.model_slug != selected_model)
        {
            tracing::info!(
                actual_model = %selected_model,
                "[orchestrate] clearing model experiment assignment after downstream model override"
            );
            selected_model_experiment = None;
        }

        let candidate_models = routing_explanation
            .as_ref()
            .map(|explanation| {
                explanation
                    .candidates
                    .iter()
                    .map(|candidate| candidate.slug.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut routing_model_providers = candidate_models
            .iter()
            .map(|model| {
                (
                    model.clone(),
                    provider_id_for_routing_model(&roko_config, &model_providers, model),
                )
            })
            .collect::<HashMap<_, _>>();
        routing_model_providers
            .entry(selected_model.clone())
            .or_insert_with(|| {
                provider_id_for_routing_model(&roko_config, &model_providers, &selected_model)
            });
        let disqualifications = routing_model_providers
            .iter()
            .filter_map(|(model, provider)| {
                (!self.learning.provider_health().is_healthy(provider))
                    .then_some((model.clone(), "provider_unhealthy".to_string()))
            })
            .collect::<HashMap<_, _>>();
        let task_category_label = TaskCategory::Implementation.label().to_string();
        let routing_meta = RoutingDecisionMeta {
            trace_id: Self::trace_id_for(plan_id, task).to_hex(),
            task_id: task.to_string(),
            requested_model: requested_model.clone(),
            role: role.label().to_string(),
            task_complexity: task_def
                .as_ref()
                .map(|td| td.tier.clone())
                .unwrap_or_else(|| "unknown".to_string()),
            task_category: task_category_label.clone(),
            routing_stage: routing_stage.clone(),
            routing_reason: routing_reason.clone(),
        };
        match RoutingLogger::open_creating(routing_log_path(&self.workdir)) {
            Ok(logger) => {
                let logger = logger
                    .with_model_providers(routing_model_providers)
                    .with_disqualifications(disqualifications);
                match self.learning.cascade_router().append_routing_log(
                    &logger,
                    &routing_meta,
                    &selected_model,
                    routing_explanation.as_ref(),
                ) {
                    Ok(routing_log) => {
                        routing_log_record = Some(routing_log);
                        routing_log_store = Some(logger.store());
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "failed to append routing decision log");
                    }
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "failed to open routing decision log");
            }
        }
        if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
            tracker.last_impl_task_id = Some(task.to_string());
            tracker.last_impl_model_slug = Some(selected_model.clone());
            tracker.last_context_knowledge_ids.clear();
        }

        // ── Build context via tiered ContextProvider ───────────────
        let context_sections = if let Some(ref td) = task_def {
            let context_provider = ContextProvider::new(self.workdir.clone())
                .with_budgets(self.config.prompt.context_budgets.to_context_budgets());

            let task_input = task_def_to_input(td);
            let plan_artifacts = PlanArtifacts::new(plan_dir.clone(), plan_id.to_string());

            // Build sibling list from the tasks file
            let siblings: Vec<roko_compose::SiblingTask> = tasks_file
                .as_ref()
                .map(|tf| {
                    tf.tasks
                        .iter()
                        .filter(|t| t.id != td.id)
                        .map(|t| roko_compose::SiblingTask {
                            id: t.id.clone(),
                            title: t.title.clone(),
                            status: t.status.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default();

            // Prior task outputs: read from .roko/task-outputs/ if available
            let prior_outputs = load_prior_task_outputs(&self.workdir, &td.depends_on);

            let resolved = context_provider.resolve(
                frequency,
                &task_input,
                &selected_model,
                &plan_artifacts,
                &siblings,
                &prior_outputs,
            );

            tracing::info!(
                "[orchestrate] Context tier={:?} sections={} tokens_est={} budget={}",
                resolved.tier,
                resolved.sections.len(),
                resolved.total_tokens_estimate,
                resolved.budget_tokens,
            );

            // Extract attribution keys before consuming into prompt sections.
            // Each key is a searchable token (file path, symbol name) that we'll
            // look for in the agent's output to measure context utilization.
            attribution_keys = resolved
                .sections
                .iter()
                .filter_map(|cs| {
                    use roko_compose::ContextSource;
                    match &cs.source {
                        ContextSource::InlineFile { path, .. } => {
                            Some(("file".into(), path.clone()))
                        }
                        ContextSource::SymbolSignature { symbol, .. } => {
                            Some(("symbol".into(), symbol.clone()))
                        }
                        _ => None,
                    }
                })
                .collect();
            let context_knowledge_ids = resolved
                .sections
                .iter()
                .filter_map(|cs| {
                    use roko_compose::ContextSource;
                    match &cs.source {
                        ContextSource::KnowledgeEntry { entry_id, .. } => Some(entry_id.clone()),
                        _ => None,
                    }
                })
                .collect::<Vec<_>>();
            if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
                tracker.last_context_knowledge_ids = context_knowledge_ids;
            }

            resolved.into_prompt_sections()
        } else {
            Vec::new()
        };

        let claude_tools_csv = claude_tool_allowlist_with(role, self.tool_registry.as_deref());
        let task_allowed_tools_csv = if let Some(task) = task_def.as_ref() {
            claude_task_tool_allowlist_with(
                role,
                task.allowed_tools.as_deref(),
                task.denied_tools.as_deref(),
                self.tool_registry.as_deref(),
            )
        } else {
            claude_tools_csv.clone()
        };
        let task_allowed_tools_csv = self.apply_daimon_tool_policy_csv(&task_allowed_tools_csv);

        // ── Adaptive format selection via bandit ─────────────────────
        let tool_count = task_allowed_tools_csv
            .split(',')
            .filter(|s| !s.is_empty())
            .count();
        let complexity = task_def
            .as_ref()
            .map(|td| match td.tier.as_str() {
                "fast" => roko_core::TaskComplexityBand::Fast,
                "complex" | "premium" => roko_core::TaskComplexityBand::Complex,
                _ => roko_core::TaskComplexityBand::Standard,
            })
            .unwrap_or(roko_core::TaskComplexityBand::Standard);
        #[allow(clippy::cast_possible_truncation)]
        let bandit_key = roko_core::tool::BanditKey::new(
            &selected_model,
            role,
            tool_count.min(255) as u8,
            complexity,
        );
        let selected_format = self.format_bandit.select(&bandit_key);
        tracing::info!(
            "[orchestrate] format_bandit: model={selected_model} role={role:?} tools={tool_count} → {selected_format:?}",
        );

        let task_affect_state = self
            .current_pad_state()
            .with_somatic_hint(somatic_signal.valence, somatic_signal.intensity);
        let role_key = format!("{role:?}");
        let section_effectiveness = self.learning.section_effectiveness_snapshot();
        let role_instruction = if let Some(system_prompt) = system_prompt_override {
            system_prompt
        } else {
            let relevant_context = build_relevant_context_layer(&context_sections);
            let context_window_tokens = effective_context_window_tokens(&self.config);
            build_system_prompt_with_context_validated(
                role,
                plan_id,
                task,
                &task_allowed_tools_csv,
                relevant_context.as_deref(),
                Some(task_affect_state),
                task_def.as_ref(),
                &prior_skills,
                context_window_tokens,
                Some(&section_effectiveness),
            )?
        };
        let role_section = PromptSection::new("role", &role_instruction)
            .with_priority(SectionPriority::Critical)
            .with_placement(Placement::Start)
            .with_bidder(AttentionBidder::TaskContext)
            .into_signal()
            .map_err(|e| anyhow!("role section: {e}"))?;
        let task_section = PromptSection::new("task", &task_text)
            .with_priority(SectionPriority::Critical)
            .with_placement(Placement::End)
            .with_bidder(AttentionBidder::TaskContext)
            .into_signal()
            .map_err(|e| anyhow!("task section: {e}"))?;

        // Combine: role (Critical/Start) + context sections (tiered) + learned context + task (Critical/End)
        let mut sections = vec![role_section];
        for cs in context_sections {
            sections.push(
                apply_section_effectiveness_to_prompt_section(
                    cs,
                    &role_key,
                    &section_effectiveness,
                )
                .into_signal()
                .map_err(|e| anyhow!("context section: {e}"))?,
            );
        }

        if let Some((playbook_section, playbook_text_len)) = playbook_context_section {
            sections.push(
                apply_section_effectiveness_to_prompt_section(
                    playbook_section,
                    &role_key,
                    &section_effectiveness,
                )
                .with_bidder(AttentionBidder::PlaybookRules)
                .into_signal()
                .map_err(|e| anyhow!("playbook section: {e}"))?,
            );
            tracing::info!(
                "[orchestrate] injected playbook context ({} chars)",
                playbook_text_len
            );
        }

        if let Some((search_section, search_text_len)) = search_context_section {
            sections.push(
                apply_section_effectiveness_to_prompt_section(
                    search_section,
                    &role_key,
                    &section_effectiveness,
                )
                .with_bidder(AttentionBidder::Research)
                .into_signal()
                .map_err(|e| anyhow!("external-research section: {e}"))?,
            );
            tracing::info!(
                "[orchestrate] injected external research context ({} chars)",
                search_text_len
            );
        }

        // ── Inject learned knowledge (skills, playbook rules, patterns) ──
        let learned = self.build_learned_context(
            role,
            task_def.as_ref(),
            &task_text,
            task_def.as_ref().map(|td| td.tier.as_str()),
            &selected_model,
        );
        if !learned.text.is_empty() {
            let learned_section = apply_section_effectiveness_to_prompt_section(
                PromptSection::new("learned-context", &learned.text)
                    .with_priority(SectionPriority::Normal)
                    .with_placement(Placement::Middle)
                    .with_bidder(AttentionBidder::Neuro),
                &role_key,
                &section_effectiveness,
            )
            .into_signal()
            .map_err(|e| anyhow!("learned-context section: {e}"))?;
            sections.push(learned_section);
            tracing::info!(
                "[orchestrate] injected learned context ({} chars)",
                learned.text.len()
            );
        }
        // Store matched skill/rule IDs for flowing into CompletedRunInput.
        if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
            tracker.last_matched_skill_id = learned.matched_skill_id;
            tracker.last_matched_rule_id = learned.matched_rule_id;
            tracker.last_experiment_variant_id = learned.experiment_variant_id;
        }

        if let Some(daimon_section) =
            build_daimon_context_section(task_affect_state, self.daimon.query().behavioral_state)
        {
            sections.push(
                apply_section_effectiveness_to_prompt_section(
                    daimon_section,
                    &role_key,
                    &section_effectiveness,
                )
                .into_signal()
                .map_err(|e| anyhow!("daimon-state section: {e}"))?,
            );
        }

        sections.push(task_section);

        // ── Tool manifest for non-CLI agents ──────────────────────────
        // Claude CLI gets tools via `--allowedTools` flag; for ExecAgent
        // and other backends, inject the role-filtered tool list into the
        // prompt so the agent knows which tools are available.
        let is_exec_agent = self.config.agent.command != "claude";
        if is_exec_agent {
            let tool_manifest = self.build_tool_manifest(
                role,
                task_def
                    .as_ref()
                    .and_then(|task| task.denied_tools.as_deref()),
            );
            if !tool_manifest.is_empty() {
                let tool_section = apply_section_effectiveness_to_prompt_section(
                    PromptSection::new("available-tools", &tool_manifest)
                        .with_priority(SectionPriority::Normal)
                        .with_placement(Placement::Middle)
                        .with_bidder(AttentionBidder::TaskContext),
                    &role_key,
                    &section_effectiveness,
                )
                .into_signal()
                .map_err(|e| anyhow!("tool manifest section: {e}"))?;
                sections.push(tool_section);
            }
        }

        let predictive_calibration = load_predictive_calibration(&self.workdir).await;
        let cfactor_source = load_cfactor_source(&self.workdir).await;
        if let Some(calibration) = predictive_calibration.as_ref() {
            for policy_section in predictive_policy_sections(
                calibration.clone(),
                &selected_model,
                &task_category_label,
            ) {
                sections.push(
                    apply_section_effectiveness_to_prompt_section(
                        policy_section,
                        &role_key,
                        &section_effectiveness,
                    )
                    .into_signal()
                    .map_err(|e| anyhow!("predictive-calibration section: {e}"))?,
                );
            }
        }
        if let Some(source) = cfactor_source.as_ref() {
            for policy_section in cfactor_policy_sections(source.clone()) {
                sections.push(
                    apply_section_effectiveness_to_prompt_section(
                        policy_section,
                        &role_key,
                        &section_effectiveness,
                    )
                    .into_signal()
                    .map_err(|e| anyhow!("collective-calibration section: {e}"))?,
                );
            }
        }

        let composer = PromptComposer::new();
        let catalyst_source = Arc::new(SectionEffectCatalystSource {
            registry: section_effectiveness.clone(),
            role: role_key.clone(),
        });
        let mut scorers: Vec<Box<dyn roko_core::Scorer>> = vec![
            Box::new(SectionScorer::new()),
            Box::new(CatalystScorer::new(catalyst_source)),
        ];
        if let Some(calibration) = predictive_calibration.clone() {
            scorers.push(Box::new(
                PredictiveScorer::new(calibration).with_pragmatic_weight(match role {
                    AgentRole::Strategist | AgentRole::Auditor => 0.8,
                    AgentRole::Implementer | AgentRole::AutoFixer => 1.5,
                    _ => 1.0,
                }),
            ));
        }
        let section_scorer: Box<dyn roko_core::Scorer> = Box::new(SumScorer::new(scorers));
        let prompt_ctx = ctx
            .clone()
            .with_attr("roko.model_slug", &selected_model)
            .with_attr("roko.role", &role_key)
            .with_attr("roko.task_category", &task_category_label)
            .with_attr("roko.task_text", &task_text)
            .with_attr(
                "roko.daimon.pleasure",
                format!("{:.4}", task_affect_state.pleasure),
            )
            .with_attr(
                "roko.daimon.arousal",
                format!("{:.4}", task_affect_state.arousal),
            )
            .with_attr(
                "roko.daimon.dominance",
                format!("{:.4}", task_affect_state.dominance),
            )
            .with_attr(
                "roko.daimon.behavioral_state",
                format!("{:?}", self.daimon.query().behavioral_state),
            );
        let prompt = composer
            .compose(
                &sections,
                &Budget::tokens(self.config.prompt.token_budget),
                section_scorer.as_ref(),
                &prompt_ctx,
            )
            .map_err(|e| anyhow!("compose: {e}"))?;
        let prompt_sections = prompt_section_meta_from_sections(&sections, &prompt);
        let prompt = scrub_signal(&prompt, &self.safety_layer.scrub_policy);
        if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
            tracker.last_prompt_sections = prompt_sections;
        }

        // Persist the prompt.
        let substrate_dir = self.workdir.join(".roko");
        let substrate = FileSubstrate::open(&substrate_dir)
            .await
            .map_err(|e| anyhow!("open substrate: {e}"))?;
        substrate
            .put(prompt.clone())
            .await
            .map_err(|e| anyhow!("persist prompt: {e}"))?;

        // ── Run the agent with per-task model selection ─────────────
        let ctx = ctx
            .with_attr("task_id", task)
            .with_attr(
                "task_complexity",
                task_def
                    .as_ref()
                    .map(|td| td.tier.as_str())
                    .unwrap_or("focused"),
            )
            .with_attr("model_tier", Self::retry_model_tier_label(&selected_model));
        let result: AgentResult = if self.config.agent.command == "claude" {
            let task_role = task_def
                .as_ref()
                .and_then(|task| task.role.clone())
                .unwrap_or_else(|| format!("{role:?}"));
            let _span = info_span!(
                "agent",
                plan_id = %plan_id,
                task_id = %task,
                agent_model = %selected_model,
                task_role = %task_role
            )
            .entered();
            let roko_config = load_roko_config(&self.workdir)?;
            let cached_content = self
                .ensure_plan_gemini_cache(plan_id, &roko_config, &selected_model)
                .await?;
            let mcp_config = self.resolve_mcp_config_path().await;
            let task_read_args = task_def
                .as_ref()
                .map(task_read_cli_args)
                .unwrap_or_default();
            let mut extra_args = task_read_args;
            extra_args.extend(self.config.agent.args.clone());
            extra_args.push("--max-turns".to_string());
            extra_args.push(dispatch_turn_limit.to_string());
            if let Some(fallback_model) = &self.config.agent.fallback_model {
                extra_args.push("--fallback-model".to_string());
                extra_args.push(fallback_model.clone());
            }
            if let Some(resume) = &self.claude_resume_session {
                extra_args.push("--resume".to_string());
                extra_args.push(resume.clone());
            }

            let agent = spawn_agent_with_layer(
                &roko_config,
                Some(self.safety_layer.clone()),
                SpawnAgentSpec {
                    model: selected_model.clone(),
                    command: Some(self.config.agent.command.clone()),
                    timeout_ms: Some(self.effective_task_timeout_ms(task_def.as_ref())),
                    system_prompt: Some(role_instruction.clone()),
                    cached_content,
                    tools: Some(task_allowed_tools_csv.clone()),
                    mcp_config,
                    working_dir: Some(exec_dir.clone()),
                    env: self.config.agent.env.clone(),
                    extra_args,
                    effort: Some(dispatch_effort.clone()),
                    bare_mode: self.config.agent.bare_mode,
                    dangerously_skip_permissions: claude_skip_permissions_for_role(role),
                    name: String::new(),
                },
                format!("create agent for model {selected_model}"),
            )?;
            let resolved = resolve_model(&roko_config, &selected_model);
            let cost_table = task_runner_cost_table(&resolved);
            let mut runner_budget = RunnerBudgetGuardrail::new(
                self.config.budget.max_task_usd,
                self.config.budget.max_session_usd,
                self.config.budget.max_plan_usd,
                f64::from(self.config.budget.warn_at_percent) / 100.0,
            );
            let task_spend = self.task_spent(plan_id, task);
            let _ = runner_budget.record_cost(task_spend, "task");
            let _ = runner_budget.record_cost(self.plan_costs.values().sum::<f64>(), "session");

            let mut runner = TaskRunner {
                agent,
                event_bus: RunnerEventBus::new(16),
                anomaly: RunnerAnomalyDetector::new(self.anomaly_detector.session_start_ms()),
                budget: runner_budget,
                conductor: RunnerConductorBandit::new(),
                cost_table,
                model_slug: resolved.slug.clone(),
                provider_id: resolved.provider_kind.label().to_string(),
                // Orchestrate still owns cross-attempt retry/escalation logic.
                max_iterations: 1,
            };
            let task_result = runner.run_task(&prompt, &ctx).await.map_err(|err| match err {
                TaskRunnerError::BudgetExhausted => anyhow!(
                    "task {plan_id}/{task} budget exhausted while running {selected_model}"
                ),
                TaskRunnerError::Anomaly(anomaly) => anyhow!(
                    "task {plan_id}/{task} anomaly detected while running {selected_model}: {anomaly:?}"
                ),
                TaskRunnerError::ModelEscalation => anyhow!(
                    "task {plan_id}/{task} requested model escalation while running {selected_model}"
                ),
            })?;

            let mut usage = task_result.total_usage;
            usage.cost_usd = task_result.total_cost_usd as f32;
            AgentResult {
                output: task_result.output,
                trace: Vec::new(),
                usage,
                success: task_result.gate_passed,
            }
        } else {
            let task_role = task_def
                .as_ref()
                .and_then(|task| task.role.clone())
                .unwrap_or_else(|| format!("{role:?}"));
            let _span = info_span!(
                "agent",
                plan_id = %plan_id,
                task_id = %task,
                agent_model = %selected_model,
                task_role = %task_role
            )
            .entered();
            let agent: Box<dyn Agent> = if is_known_protocol_command(&self.config.agent.command) {
                let fallback_config =
                    synthesize_known_protocol_config(&self.config.agent.command, &selected_model);
                let agent = spawn_agent_with_layer(
                    &fallback_config,
                    Some(self.safety_layer.clone()),
                    SpawnAgentSpec {
                        model: selected_model.clone(),
                        command: Some(self.config.agent.command.clone()),
                        timeout_ms: Some(self.config.agent.timeout_ms),
                        system_prompt: None,
                        cached_content: None,
                        tools: None,
                        mcp_config: None,
                        working_dir: Some(exec_dir.clone()),
                        env: self.config.agent.env.clone(),
                        extra_args: self.config.agent.args.clone(),
                        effort: None,
                        bare_mode: self.config.agent.bare_mode,
                        dangerously_skip_permissions: false,
                        name: String::new(),
                    },
                    format!(
                        "create known-protocol subprocess agent for {}",
                        self.config.agent.command
                    ),
                )?;
                agent
            } else {
                let fallback_config = synthesize_subprocess_config(&self.config.agent.command);
                let agent = spawn_agent_with_layer(
                    &fallback_config,
                    Some(self.safety_layer.clone()),
                    SpawnAgentSpec {
                        model: selected_model.clone(),
                        command: Some(self.config.agent.command.clone()),
                        timeout_ms: Some(self.config.agent.timeout_ms),
                        system_prompt: None,
                        cached_content: None,
                        tools: None,
                        mcp_config: None,
                        working_dir: Some(exec_dir.clone()),
                        env: self.config.agent.env.clone(),
                        extra_args: self.config.agent.args.clone(),
                        effort: None,
                        bare_mode: self.config.agent.bare_mode,
                        dangerously_skip_permissions: false,
                        name: String::new(),
                    },
                    format!(
                        "create generic subprocess agent for {}",
                        self.config.agent.command
                    ),
                )?;
                agent
            };
            let mut runner_budget = RunnerBudgetGuardrail::new(
                self.config.budget.max_task_usd,
                self.config.budget.max_session_usd,
                self.config.budget.max_plan_usd,
                f64::from(self.config.budget.warn_at_percent) / 100.0,
            );
            let task_spend = self.task_spent(plan_id, task);
            let _ = runner_budget.record_cost(task_spend, "task");
            let _ = runner_budget.record_cost(self.plan_costs.values().sum::<f64>(), "session");

            let mut runner = TaskRunner {
                agent,
                event_bus: RunnerEventBus::new(16),
                anomaly: RunnerAnomalyDetector::new(self.anomaly_detector.session_start_ms()),
                budget: runner_budget,
                conductor: RunnerConductorBandit::new(),
                cost_table: RunnerCostTable::default(),
                model_slug: selected_model.clone(),
                provider_id: self.provider_id_for_model(&selected_model),
                // Orchestrate still owns cross-attempt retry/escalation logic.
                max_iterations: 1,
            };
            let task_result = runner
                .run_task(&prompt, &ctx)
                .await
                .map_err(|err| match err {
                    TaskRunnerError::BudgetExhausted => anyhow!(
                        "task {plan_id}/{task} budget exhausted while running {}",
                        self.config.agent.command
                    ),
                    TaskRunnerError::Anomaly(anomaly) => anyhow!(
                        "task {plan_id}/{task} anomaly detected while running {}: {anomaly:?}",
                        self.config.agent.command
                    ),
                    TaskRunnerError::ModelEscalation => anyhow!(
                        "task {plan_id}/{task} requested model escalation while running {}",
                        self.config.agent.command
                    ),
                })?;

            let mut usage = task_result.total_usage;
            usage.cost_usd = task_result.total_cost_usd as f32;
            AgentResult {
                output: task_result.output,
                trace: Vec::new(),
                usage,
                success: task_result.gate_passed,
            }
        };
        let result = scrub_agent_result(&result, &self.safety_layer.scrub_policy);

        self.record_turn_learning_feedback(&prompt, &selected_model, &result);
        if let (Some(store), Some(record)) = (&routing_log_store, routing_log_record.as_ref()) {
            let completed = record.clone().with_outcome(
                result.success,
                f64::from(result.usage.cost_usd),
                result.usage.wall_ms,
            );
            if let Err(err) = store.append(&completed).await {
                tracing::warn!(error = %err, "failed to append completed routing decision log");
            }
        }

        let task_cost = f64::from(result.usage.cost_usd);
        self.add_task_spend(plan_id, task, task_cost);
        let plan_spent = self.plan_costs.get(plan_id).copied().unwrap_or(0.0);
        self.warn_plan_budget_pressure(plan_id, plan_spent);
        if plan_spent >= self.config.budget.max_plan_usd {
            return Err(anyhow!(
                "plan {plan_id} budget exhausted: ${plan_spent:.2} >= ${:.2} max",
                self.config.budget.max_plan_usd
            ));
        }

        // Persist the output.
        substrate
            .put(result.output.clone())
            .await
            .map_err(|e| anyhow!("persist agent output: {e}"))?;

        if !is_meaningful_output(&result.output) {
            if let (Some(before_changed_files), Some(after_changed_files)) = (
                preexisting_changed_files.as_ref(),
                self.git_changed_files(&exec_dir).await.ok(),
            ) {
                if before_changed_files == &after_changed_files {
                    self.emit_conductor_signal(
                        Kind::Custom(GHOST_TURN_SIGNAL_KIND.into()),
                        serde_json::json!({
                            "plan_id": plan_id,
                            "task": task,
                            "role": format!("{role:?}"),
                            "model": &selected_model,
                            "cost_usd": task_cost,
                            "duration_ms": result.usage.wall_ms,
                            "changed_files_before": before_changed_files,
                            "changed_files_after": after_changed_files,
                            "net_new_changes": 0usize,
                            "output_meaningful": false,
                            "wasted_cost": true,
                        }),
                    );
                }
            }
        }

        // Feed the raw agent turn into the conductor stream so the stuck-pattern
        // watcher can compare consecutive outputs across turns.
        self.emit_agent_turn_signal(&result.output);

        // ── Context attribution feedback ──────────────────────────────
        // Scan agent output for references to injected context sections.
        // This measures which context was actually useful, enabling the
        // ContextProvider to demote low-utility sources over time.
        if !attribution_keys.is_empty() {
            let output_text = result.output.body.as_text().unwrap_or_default();
            let mut referenced = 0usize;
            let total = attribution_keys.len();

            for (kind, key) in &attribution_keys {
                // Check if the agent's output references this context section.
                // For files: look for the file path. For symbols: look for the symbol name.
                let was_referenced = match kind.as_str() {
                    "file" => {
                        // Match full path or just filename
                        let filename = std::path::Path::new(key)
                            .file_name()
                            .and_then(|f| f.to_str())
                            .unwrap_or(key);
                        output_text.contains(key) || output_text.contains(filename)
                    }
                    "symbol" => output_text.contains(key.as_str()),
                    _ => false,
                };
                if was_referenced {
                    referenced += 1;
                }
                // Update rolling attribution tracker per (tier, source_type).
                let tier_str = task_def
                    .as_ref()
                    .map(|td| td.tier.as_str())
                    .unwrap_or("unknown");
                self.attribution_tracker
                    .record(tier_str, kind, was_referenced);
                self.context_average_tracker
                    .record(tier_str, kind, was_referenced);
            }

            let ref_rate = if total > 0 {
                (referenced as f64) / (total as f64)
            } else {
                0.0
            };

            tracing::info!(
                "[orchestrate] Context attribution: {referenced}/{total} sections referenced (ref_rate={ref_rate:.2})"
            );

            // Persist attribution to .roko/context-attribution.jsonl
            let attribution_path = self.workdir.join(".roko").join("context-attribution.jsonl");
            let record = serde_json::json!({
                "plan_id": plan_id,
                "task": task,
                "tier": task_def.as_ref().map(|td| td.tier.as_str()).unwrap_or("unknown"),
                "total_sections": total,
                "referenced_sections": referenced,
                "ref_rate": ref_rate,
                "ts": chrono::Utc::now().to_rfc3339(),
            });
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&attribution_path)
            {
                use std::io::Write;
                let _ = writeln!(file, "{}", record);
                // Write per-source records so the tracker can distinguish source types.
                for (kind, key) in &attribution_keys {
                    let was_referenced = match kind.as_str() {
                        "file" => {
                            let filename = std::path::Path::new(key)
                                .file_name()
                                .and_then(|f| f.to_str())
                                .unwrap_or(key);
                            output_text.contains(key) || output_text.contains(filename)
                        }
                        "symbol" => output_text.contains(key.as_str()),
                        _ => false,
                    };
                    let per_source = serde_json::json!({
                        "plan_id": plan_id,
                        "task": task,
                        "tier": task_def.as_ref().map(|td| td.tier.as_str()).unwrap_or("unknown"),
                        "source_type": kind,
                        "source_key": key,
                        "referenced": was_referenced,
                        "ts": chrono::Utc::now().to_rfc3339(),
                    });
                    let _ = writeln!(file, "{}", per_source);
                }
            }

            if let Err(e) = self.context_average_tracker.save() {
                tracing::warn!(
                    "[context] failed to persist context averages to {}: {e}",
                    self.workdir
                        .join(".roko")
                        .join("learn")
                        .join("context-averages.json")
                        .display()
                );
            }
        }

        if let Some(forced_model) =
            detect_cost_anomaly_override(&mut self.anomaly_detector, task_cost, &self.config)
        {
            self.force_model_override = Some(forced_model);
        }

        if !result.success {
            self.record_model_experiment_outcome(
                selected_model_experiment.as_ref(),
                false,
                &result,
            )?;
            self.observe_cascade_router(plan_id, task, task_def.as_ref(), &selected_model, 0.0);
            let task_phase = task_def
                .as_ref()
                .map(|task| task.status.as_str())
                .unwrap_or("unknown");
            let output_tail = result
                .output
                .body
                .as_text()
                .ok()
                .map(|text| tail_output_lines(text, TASK_FAILURE_OUTPUT_TAIL_LINES));
            let error = anyhow!("agent returned failure for plan={plan_id} task={task}");
            self.release_task_mcp_servers(&mcp_lease).await;
            return Err(with_task_failure_context(
                error,
                task,
                task_phase,
                "agent",
                output_tail.as_deref(),
            ));
        }

        let post_result = async {
            // ── Cost recording ────────────────────────────────────────────
            if task_cost > self.config.budget.max_task_usd {
                return Err(anyhow!(
                    "task {task} cost ${task_cost:.2} exceeds max_task_usd ${:.2}",
                    self.config.budget.max_task_usd
                ));
            }
            *self.plan_costs.entry(plan_id.to_string()).or_insert(0.0) += task_cost;
            let plan_spent = self.plan_costs.get(plan_id).copied().unwrap_or(0.0);
            self.warn_plan_budget_pressure(plan_id, plan_spent);

            // ── Session budget check (§8) ───────────────────────────────
            let max_session_usd = self.config.budget.max_session_usd;
            let session_total: f64 = self.plan_costs.values().sum();
            if max_session_usd > 0.0 && session_total > max_session_usd {
                return Err(anyhow!(
                    "session budget exceeded: ${session_total:.2} > max_session_usd ${max_session_usd:.2}"
                ));
            }

            self.learning.costs_db().insert(CostRecord {
                timestamp: chrono::Utc::now().to_rfc3339(),
                model: selected_model.clone(),
                provider: self.config.agent.command.clone(),
                role: format!("{role:?}"),
                plan_id: plan_id.to_string(),
                task_id: task.to_string(),
                complexity_band: task_def
                    .as_ref()
                    .map(|td| td.tier.clone())
                    .unwrap_or_default(),
                input_tokens: u64::from(result.usage.input_tokens),
                output_tokens: u64::from(result.usage.output_tokens),
                cached_tokens: u64::from(result.usage.cache_read_tokens),
                cost_usd: task_cost,
                duration_ms: result.usage.wall_ms,
                success: result.success,
                session_id: self.claude_resume_session.clone().unwrap_or_default(),
            });

            // ── Metric instrumentation ──────────────────────────────────────
            #[allow(clippy::cast_precision_loss)]
            {
                let status = if result.success {
                    "succeeded"
                } else {
                    "failed"
                };
                let role_str = format!("{role:?}");
                self.metrics
                    .register_counter(
                        "roko_tasks_total",
                        "",
                        LabelSet::from_pairs(&[("status", status), ("role", &role_str)]),
                    )
                    .inc();
                self.metrics
                    .register_histogram(
                        "roko_agent_duration_seconds",
                        "",
                        LabelSet::from_pairs(&[("role", &role_str)]),
                        roko_core::obs::LLM_LATENCY_BUCKETS.to_vec(),
                    )
                    .observe(result.usage.wall_ms as f64 / 1000.0);
                let total_tokens =
                    u64::from(result.usage.input_tokens) + u64::from(result.usage.output_tokens);
                self.metrics
                    .register_counter(
                        "roko_llm_tokens_total",
                        "",
                        LabelSet::from_pairs(&[("role", &role_str)]),
                    )
                    .inc_by(total_tokens);
                // Cost metric — scale to millionths to use integer counter.
                #[allow(clippy::cast_sign_loss)]
                let cost_micro = (task_cost * 1_000_000.0) as u64;
                self.metrics
                    .register_counter(
                        "roko_llm_cost_usd_total",
                        "",
                        LabelSet::from_pairs(&[("role", &role_str), ("model", &selected_model)]),
                    )
                    .inc_by(cost_micro);
            }

            // ── Conductor signal: agent output (§7) ──────────────────────
            let timeout_secs = task_def
                .as_ref()
                .map(|td| td.timeout_secs)
                .unwrap_or(self.executor.config().task_timeout_secs);
            self.emit_conductor_signal(
                Kind::Custom("conductor.agent_output".into()),
                serde_json::json!({
                    "plan_id": plan_id,
                    "task": task,
                    "role": format!("{role:?}"),
                    "model": &selected_model,
                    "cost_usd": task_cost,
                    "duration_ms": result.usage.wall_ms,
                    "timeout_secs": timeout_secs,
                    "tokens": u64::from(result.usage.input_tokens) + u64::from(result.usage.output_tokens),
                    "success": result.success,
                }),
            );

            self.finish_task_post_processing(
                plan_id,
                task,
                task_def.as_ref(),
                &task_text,
                &role_instruction,
                &selected_model,
                &result,
                &exec_dir,
            )
            .await?;

            Ok::<(), anyhow::Error>(())
        }
        .await;

        self.release_task_mcp_servers(&mcp_lease).await;
        match post_result {
            Ok(()) => {
                self.record_model_experiment_outcome(
                    selected_model_experiment.as_ref(),
                    true,
                    &result,
                )?;
            }
            Err(err) => {
                self.record_model_experiment_outcome(
                    selected_model_experiment.as_ref(),
                    false,
                    &result,
                )?;
                return Err(err);
            }
        }

        Ok(result)
    }

    /// Run per-task verification steps.
    ///
    /// Returns `Ok(())` if all steps succeed. If a step fails, returns
    /// `Err((task_id, phase, command, error_output))`.
    async fn run_verify_steps(
        &self,
        task_id: &str,
        verify_steps: &[crate::task_parser::VerifyStep],
        exec_dir: &Path,
    ) -> Result<(), (String, String, String, String)> {
        if verify_steps.is_empty() {
            return Ok(());
        }

        tracing::info!(
            "[orchestrate] Running {} verify steps for {}",
            verify_steps.len(),
            task_id
        );
        for step in verify_steps {
            let output = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&step.command)
                .current_dir(exec_dir)
                .output()
                .await;

            match output {
                Ok(o) if o.status.success() => {
                    tracing::info!("  ✅ [{}] {}", step.phase, step.command);
                }
                Ok(o) => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    let msg = step.fail_msg.as_deref().unwrap_or("verification failed");
                    tracing::error!(
                        "  ❌ [{}] {} — {}: {}",
                        step.phase,
                        step.command,
                        msg,
                        stderr.trim()
                    );
                    return Err((
                        task_id.to_string(),
                        step.phase.clone(),
                        step.command.clone(),
                        stderr.to_string(),
                    ));
                }
                Err(e) => {
                    tracing::error!("  ❌ [{}] {} — spawn error: {e}", step.phase, step.command);
                    return Err((
                        task_id.to_string(),
                        step.phase.clone(),
                        step.command.clone(),
                        format!("spawn error: {e}"),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Run gates at the specified rung level and return whether all passed.
    async fn run_gate_pipeline(&mut self, plan_id: &str, rung: u32) -> Result<bool> {
        let exec_dir = self.ensure_plan_exec_dir(plan_id).await?;
        let payload = GatePayload::in_dir(&exec_dir).with_label(format!("{plan_id}:rung-{rung}"));
        let started = std::time::Instant::now();
        let mut payload_builder = Engram::builder(Kind::Task)
            .body(Body::from_json(&payload)?)
            .provenance(Provenance::trusted("orchestrate"))
            .tag("plan_id", plan_id)
            .tag("rung", rung.to_string());
        if let Some(parent_hash) = self
            .task_trackers
            .get(plan_id)
            .and_then(|tracker| tracker.last_impl_output_hash)
        {
            payload_builder = payload_builder.lineage([parent_hash]);
        }
        let payload_sig = maybe_attest_engram(payload_builder.build());

        let verdicts = Self::run_gate_rung(&payload_sig, rung).await;
        if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
            tracker.last_gate_verdicts = verdicts
                .iter()
                .map(|verdict| GateVerdict::new(verdict.gate.clone(), verdict.passed))
                .collect();
        }

        // Persist verdicts.
        let substrate_dir = self.workdir.join(".roko");
        if let Ok(substrate) = FileSubstrate::open(&substrate_dir).await {
            for verdict in &verdicts {
                let sig = maybe_attest_engram(
                    payload_sig
                        .derive(
                            Kind::GateVerdict,
                            Body::from_json(verdict)
                                .unwrap_or_else(|_| Body::text(format!("{verdict:?}"))),
                        )
                        .provenance(Provenance::trusted("orchestrate"))
                        .tag("gate", &verdict.gate)
                        .tag("passed", verdict.passed.to_string())
                        .build(),
                );
                let _ = substrate.put(sig).await;
            }
        }

        // Record gate results on the plan state.
        if let Some(state) = self.executor.plan_state_mut(plan_id) {
            for verdict in &verdicts {
                state
                    .gate_results
                    .push(GateResult::from_verdict(verdict, rung));
            }
        }

        let all_passed = verdicts.iter().all(|v| v.passed);

        if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
            if all_passed {
                if let Some(mut request) = tracker.last_skill_request.clone() {
                    request.gate_results = verdicts
                        .iter()
                        .map(|verdict| {
                            SkillGateResult::new(
                                verdict.gate.clone(),
                                verdict.passed,
                                f64::from(verdict.score),
                            )
                        })
                        .collect();
                    tracker.last_skill_request = Some(request);
                }
            } else {
                tracker.last_skill_request = None;
            }
        }

        // Increment gate verdict metrics.
        for v in &verdicts {
            let verdict_str = if v.passed { "pass" } else { "fail" };
            self.metrics
                .register_counter(
                    "roko_gate_verdicts_total",
                    "",
                    LabelSet::from_pairs(&[("gate", &v.gate), ("verdict", verdict_str)]),
                )
                .inc();
        }

        // Conductor signal: gate verdict (§7).
        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let test_count = verdicts.iter().find_map(|verdict| {
            verdict.test_count.map(|test_count| {
                serde_json::json!({
                    "passed": test_count.passed,
                    "failed": test_count.failed,
                    "ignored": test_count.ignored,
                    "total": test_count.total(),
                })
            })
        });
        self.emit_conductor_signal(
            Kind::GateVerdict,
            serde_json::json!({
                "plan_id": plan_id,
                "rung": rung,
                "passed": all_passed,
                "duration_ms": wall_ms,
                "test_count": test_count,
            }),
        );

        if !all_passed {
            if let Some(state) = self.executor.plan_state_mut(plan_id) {
                state.last_error = Some(Self::format_gate_failure_context(&verdicts));
            }

            if let Some(message) = Self::compile_failure_message(&verdicts) {
                self.emit_conductor_signal(
                    Kind::CompileDiagnostic,
                    serde_json::json!({
                        "plan_id": plan_id,
                        "message": message,
                    }),
                );
            }
        }
        Ok(all_passed)
    }

    /// Attempt a git merge for a plan's branch.
    async fn merge_branch(&self, plan_id: &str) -> Result<()> {
        let branch_name = self
            .worktrees
            .get(plan_id)
            .map_or_else(|| format_branch_name(plan_id), |h| h.branch);
        git_merge_branch_into(&self.workdir, &branch_name).await
    }

    async fn finalize_successful_task_worktree(
        &self,
        plan_id: &str,
        task_id: &str,
        exec_dir: &Path,
    ) -> Result<()> {
        if !self.worktrees_enabled() {
            return Ok(());
        }

        self.clear_stale_worktree_locks().await;
        let plan_handle = self
            .worktrees
            .ensure_for_plan(plan_id)
            .await
            .map_err(|e| anyhow!("ensure plan worktree {plan_id}: {e}"))?;
        let commit_message = format!("task: {task_id}");
        git_commit_all_if_needed(exec_dir, &commit_message).await?;
        let task_branch = format!("roko/task/{plan_id}/{task_id}");
        git_merge_branch_into(&plan_handle.path, &task_branch)
            .await
            .with_context(|| format!("merge task branch {task_branch} into plan {plan_id}"))?;
        self.worktrees.touch(plan_id);
        Ok(())
    }

    async fn cleanup_plan_worktree(&self, plan_id: &str) {
        if !self.worktrees_enabled() || self.worktrees.get(plan_id).is_none() {
            return;
        }

        if let Err(e) = self.worktrees.remove(plan_id).await {
            tracing::error!("[orchestrate] plan worktree cleanup failed for {plan_id}: {e}");
        }
    }

    async fn cleanup_tracked_plan_worktrees(&self) {
        if !self.worktrees_enabled() {
            return;
        }

        if let Err(e) = self.worktrees.remove_all().await {
            tracing::error!("[orchestrate] tracked plan worktree cleanup failed: {e}");
        }
    }

    async fn run_post_merge_follow_up(&self, plan_id: &str) -> Result<bool> {
        let payload =
            GatePayload::in_dir(&self.workdir).with_label(format!("{plan_id}:post-merge"));
        let payload_sig = Engram::builder(Kind::Task)
            .body(Body::from_json(&payload)?)
            .provenance(Provenance::trusted("orchestrate"))
            .tag("plan_id", plan_id)
            .tag("rung", "post-merge")
            .build();

        let verdicts = Self::run_gate_rung(&payload_sig, 3).await;
        let merged_at_ms = now_unix_ms_i64();
        let (_check, follow_up) =
            self.post_merge
                .run_record_and_follow_up(plan_id, merged_at_ms, &verdicts);

        if follow_up.needs_revert() {
            self.event_log.append(
                EventKind::ErrorOccurred,
                serde_json::json!({
                    "plan_id": plan_id,
                    "error": "post-merge regression detected",
                    "failing_tests": follow_up.failing_tests,
                }),
            );
            return Ok(false);
        }

        Ok(true)
    }

    /// Persist a successful task into the skill library after merge succeeds.
    async fn extract_pending_skill(&mut self, plan_id: &str) {
        let Some(request) = self
            .task_trackers
            .get(plan_id)
            .and_then(|tracker| tracker.last_skill_request.clone())
        else {
            return;
        };

        if let Some(skill) = self.skill_library.extract_skill(request).await {
            tracing::info!(
                "[orchestrate] extracted skill {} from plan {}",
                skill.name,
                plan_id
            );
        }

        if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
            tracker.last_skill_request = None;
        }
    }

    async fn ensure_plan_exec_dir(&self, plan_id: &str) -> Result<PathBuf> {
        if !self.worktrees_enabled() {
            return Ok(self.workdir.clone());
        }
        self.clear_stale_worktree_locks().await;
        self.worktrees
            .ensure_for_plan(plan_id)
            .await
            .map(|handle| handle.path)
            .map_err(|err| anyhow!("worktree unavailable for plan={plan_id}: {err}"))
    }

    #[cfg(test)]
    async fn plan_exec_dir(&self, plan_id: &str) -> PathBuf {
        match self.ensure_plan_exec_dir(plan_id).await {
            Ok(path) => path,
            Err(err) => {
                tracing::error!("[orchestrate] {err}; using repo root");
                self.workdir.clone()
            }
        }
    }

    /// Create (or fall back to plan-level) worktree for an individual task
    /// within a plan, so parallel tasks get isolated working directories.
    async fn task_exec_dir(&self, plan_id: &str, task_id: &str) -> Result<PathBuf> {
        if !self.worktrees_enabled() {
            return Ok(self.workdir.clone());
        }
        self.clear_stale_worktree_locks().await;
        let wt_id = format!("{plan_id}-{task_id}");
        let branch = format!("roko/task/{plan_id}/{task_id}");
        let handle = self
            .worktrees
            .create(&wt_id, &branch)
            .await
            .map_err(|e| anyhow!("create task worktree {wt_id}: {e}"))?;
        Ok(handle.path)
    }

    async fn run_gate_rung(payload_sig: &Engram, rung: u32) -> Vec<Verdict> {
        let ctx = Context::now();
        // Rung 0 = compile, rung 1 = test, rung 2 = clippy, rung 3+ = all.
        match rung {
            0 => {
                let gate = CompileGate::cargo();
                let _span = info_span!("gate", gate = %gate.name(), rung = %rung).entered();
                vec![gate.verify(payload_sig, &ctx).await]
            }
            1 => {
                let gate = TestGate::cargo();
                let _span = info_span!("gate", gate = %gate.name(), rung = %rung).entered();
                vec![gate.verify(payload_sig, &ctx).await]
            }
            2 => {
                let gate = ClippyGate::cargo();
                let _span = info_span!("gate", gate = %gate.name(), rung = %rung).entered();
                vec![gate.verify(payload_sig, &ctx).await]
            }
            _ => {
                let c = CompileGate::cargo();
                let t = TestGate::cargo();
                let cl = ClippyGate::cargo();
                let compile = {
                    let _span = info_span!("gate", gate = %c.name(), rung = %rung).entered();
                    c.verify(payload_sig, &ctx).await
                };
                let test = {
                    let _span = info_span!("gate", gate = %t.name(), rung = %rung).entered();
                    t.verify(payload_sig, &ctx).await
                };
                let clippy = {
                    let _span = info_span!("gate", gate = %cl.name(), rung = %rung).entered();
                    cl.verify(payload_sig, &ctx).await
                };
                vec![compile, test, clippy]
            }
        }
    }

    /// Run task-level verification commands declared in `tasks.toml` for a plan.
    async fn run_plan_verify_steps(
        &self,
        plan_id: &str,
    ) -> Result<(), (String, String, String, String)> {
        let Some(tracker) = self.task_trackers.get(plan_id) else {
            return Ok(());
        };

        let steps_to_run: Vec<(String, Vec<crate::task_parser::VerifyStep>)> = tracker
            .tasks_file
            .tasks
            .iter()
            .filter(|task| tracker.completed.contains(&task.id))
            .filter(|task| !task.verify.is_empty())
            .map(|task| (task.id.clone(), task.verify.clone()))
            .collect();

        if steps_to_run.is_empty() {
            tracing::info!("[orchestrate] {plan_id}: no task verify steps declared");
            return Ok(());
        }

        let exec_dir = self.ensure_plan_exec_dir(plan_id).await.map_err(|err| {
            (
                plan_id.to_string(),
                "worktree".to_string(),
                "ensure_plan_exec_dir".to_string(),
                err.to_string(),
            )
        })?;
        tracing::info!(
            "[orchestrate] Running plan verify for {plan_id} across {} task(s)",
            steps_to_run.len()
        );

        for (task_id, verify_steps) in steps_to_run {
            if let Err(err) = self
                .run_verify_steps(&task_id, &verify_steps, &exec_dir)
                .await
            {
                return Err(err);
            }
        }

        Ok(())
    }

    /// Remove stale git worktree locks before creating or using worktrees.
    async fn clear_stale_worktree_locks(&self) {
        if !self.worktrees_enabled() {
            return;
        }
        match self.worktrees.clear_stale_locks() {
            Ok(cleared) if !cleared.is_empty() => {
                tracing::info!(
                    "[orchestrate] cleared {} stale worktree lock(s)",
                    cleared.len()
                );
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!("[orchestrate] stale lock cleanup failed: {e}");
            }
        }
    }

    /// Build a review prompt using the ReviewerTemplate with available context.
    async fn build_review_prompt(&self, plan_id: &str) -> String {
        use roko_compose::templates::reviewer::{Reviewer, ReviewerInput, ReviewerTemplate};
        use roko_compose::templates::{PlanSlice, RolePromptTemplate};

        let plan_dir = plans_dir(&self.workdir).join(plan_id);

        // Load plan.md content
        let plan_md_path = plan_dir.join("plan.md");
        let mut plan_content = tokio::fs::read_to_string(&plan_md_path)
            .await
            .unwrap_or_default();

        if let Some(tracker) = self.task_trackers.get(plan_id) {
            let task_spec = task_spec_summary(&tracker.tasks_file);
            if !task_spec.is_empty() {
                plan_content.push_str("\n\n---\n\n## Task spec\n");
                plan_content.push_str(&task_spec);
            }
        }

        // Load AGENTS.md if it exists
        let agents_md_path = self.workdir.join("AGENTS.md");
        let agents_md = tokio::fs::read_to_string(&agents_md_path)
            .await
            .unwrap_or_default();

        // Get files changed via git diff
        let files_changed = match self.ensure_plan_exec_dir(plan_id).await {
            Ok(exec_dir) => tokio::process::Command::new("git")
                .args(["diff", "--name-only", "HEAD"])
                .current_dir(&exec_dir)
                .output()
                .await
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.lines().map(String::from).collect::<Vec<_>>())
                .unwrap_or_default(),
            Err(err) => {
                tracing::warn!(
                    "[orchestrate] review prompt skipped worktree diff for {plan_id}: {err}"
                );
                Vec::new()
            }
        };

        // Prior review findings from tracker
        let prior_findings = self
            .task_trackers
            .get(plan_id)
            .and_then(|t| t.review_feedback.clone());

        let input = ReviewerInput {
            agents_md,
            plan: PlanSlice {
                num: String::new(),
                base: plan_id.to_string(),
                title: plan_id.to_string(),
                content: plan_content,
            },
            filtered_workspace_map: String::new(),
            prd2_extract: String::new(),
            brief: String::new(),
            files_changed,
            prior_findings,
        };

        let template = ReviewerTemplate::new(Reviewer::Combined);
        let sections = template.sections(&input);

        // Join sections into a single prompt string
        sections
            .iter()
            .map(|s| s.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    }

    /// Build a doc-revision prompt using the ScribeTemplate with available context.
    async fn build_doc_revision_prompt(&self, plan_id: &str) -> String {
        use roko_compose::templates::scribe::{ScribeInput, ScribeTemplate, ScribeVariant};
        use roko_compose::templates::{PlanSlice, RolePromptTemplate};

        let plan_dir = plans_dir(&self.workdir).join(plan_id);
        let mut public_api_files = Vec::new();
        let mut source_snippets = Vec::new();

        let last_task = self
            .task_trackers
            .get(plan_id)
            .and_then(TaskTracker::last_impl_task)
            .cloned();

        if let Some(task) = last_task {
            let (files, snippets) = self.collect_public_api_snippets(&task).await;
            public_api_files = files;
            source_snippets = snippets;
        }

        // Load plan.md content
        let plan_md_path = plan_dir.join("plan.md");
        let plan_content = tokio::fs::read_to_string(&plan_md_path)
            .await
            .unwrap_or_default();

        // Load AGENTS.md if it exists
        let agents_md_path = self.workdir.join("AGENTS.md");
        let agents_md = tokio::fs::read_to_string(&agents_md_path)
            .await
            .unwrap_or_default();

        let brief = if public_api_files.is_empty() {
            String::new()
        } else {
            format!(
                "This task changed public API surface. Generate or update documentation for the exported items in the touched files:\n{}\n\n\
                 Update module docs, inline docs, and user-facing references so the public API remains accurate.",
                public_api_files
                    .iter()
                    .map(|file| format!("- {file}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };

        let input = ScribeInput {
            agents_md,
            plan: PlanSlice {
                num: String::new(),
                base: plan_id.to_string(),
                title: plan_id.to_string(),
                content: plan_content,
            },
            prd2_extract: String::new(),
            brief,
            source_snippets,
            variant: ScribeVariant::Initial,
            critic_feedback: None,
            prior_docs: None,
        };

        let template = ScribeTemplate;
        let sections = template.sections(&input);

        sections
            .iter()
            .map(|s| s.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    }

    /// Collect source snippets for touched files that appear to expose public API.
    async fn collect_public_api_snippets(
        &self,
        task: &crate::task_parser::TaskDef,
    ) -> (
        Vec<String>,
        Vec<roko_compose::templates::scribe::FileSnippet>,
    ) {
        let mut public_api_files = Vec::new();
        let mut snippets = Vec::new();

        for file in &task.files {
            let path = self.workdir.join(file);
            let Ok(content) = tokio::fs::read_to_string(&path).await else {
                continue;
            };

            if !file_contains_public_api(file, &content) {
                continue;
            }

            public_api_files.push(file.clone());
            snippets.push(roko_compose::templates::scribe::FileSnippet {
                path: file.clone(),
                content: truncate_doc_snippet(&content, 12_000),
            });
        }

        (public_api_files, snippets)
    }

    // ── Observability helpers ────────────────────────────────────────────

    /// Derive a deterministic `TraceId` from plan + task identifiers.
    fn trace_id_for(plan_id: &str, task_id: &str) -> TraceId {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        plan_id.hash(&mut hasher);
        task_id.hash(&mut hasher);
        let h = hasher.finish();
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&h.to_le_bytes());
        // Second half: hash again with a salt for uniqueness.
        "roko-trace".hash(&mut hasher);
        let h2 = hasher.finish();
        bytes[8..].copy_from_slice(&h2.to_le_bytes());
        TraceId::from_bytes(bytes)
    }

    /// Emit a trace event after an agent dispatch (success or failure).
    fn emit_agent_trace(&self, plan_id: &str, task_id: &str, success: bool, wall_ms: u64) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let trace_id = Self::trace_id_for(plan_id, task_id);
        let event = ToolTraceEvent::Custom {
            name: "agent_dispatch".to_string(),
            data: serde_json::json!({
                "plan_id": plan_id,
                "task_id": task_id,
                "success": success,
                "wall_ms": wall_ms,
            }),
            at_ms: now_ms,
        };
        self.obs_sinks.trace_sink.append(trace_id, event);
    }

    /// Emit a trace event after a gate pipeline run.
    fn emit_gate_metric(&self, plan_id: &str, rung: u32, passed: bool, wall_ms: u64) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let trace_id = Self::trace_id_for(plan_id, &format!("gate-rung-{rung}"));
        let event = ToolTraceEvent::Custom {
            name: "gate_result".to_string(),
            data: serde_json::json!({
                "plan_id": plan_id,
                "rung": rung,
                "passed": passed,
                "wall_ms": wall_ms,
            }),
            at_ms: now_ms,
        };
        self.obs_sinks.trace_sink.append(trace_id, event);

        // Increment the well-known gate metric.
        let rung_str = format!("rung-{rung}");
        let verdict = if passed { "pass" } else { "fail" };
        self.metrics
            .register_counter(
                "roko_gate_verdicts_total",
                "",
                LabelSet::from_pairs(&[("gate", &rung_str), ("verdict", verdict)]),
            )
            .inc();
    }

    /// Feed the raw agent turn output into the conductor stream.
    ///
    /// The stuck-pattern watcher only counts consecutive action bodies, so we
    /// emit one action signal per completed turn and keep the metadata signals
    /// on non-action kinds.
    fn emit_agent_turn_signal(&mut self, output: &Engram) {
        if let Some(signal) = conductor_signal_from_output(output) {
            self.conductor_signals.push(signal);
        }
    }

    /// Construct and persist an [`AgentEfficiencyEvent`] for one agent turn.
    async fn emit_efficiency_event(
        &mut self,
        plan_id: &str,
        task_id: &str,
        role: &str,
        model: &str,
        frequency: OperatingFrequency,
        result: &AgentResult,
        wall_ms: u64,
        success: bool,
    ) {
        let prompt_sections = self
            .task_trackers
            .get(plan_id)
            .filter(|tracker| tracker.last_impl_task_id.as_deref() == Some(task_id))
            .map(|tracker| tracker.last_prompt_sections.clone())
            .unwrap_or_default();
        let event = AgentEfficiencyEvent {
            agent_id: result.output.id.to_string(),
            role: role.to_string(),
            backend: "claude".to_string(),
            model: model.to_string(),
            plan_id: plan_id.to_string(),
            task_id: task_id.to_string(),
            input_tokens: u64::from(result.usage.input_tokens),
            output_tokens: u64::from(result.usage.output_tokens),
            reasoning_tokens: 0,
            cache_read_tokens: u64::from(result.usage.cache_read_tokens),
            cache_write_tokens: u64::from(result.usage.cache_create_tokens),
            cost_usd: f64::from(result.usage.cost_usd),
            cost_usd_without_cache: f64::from(result.usage.cost_usd), // No cache discount info available.
            total_prompt_tokens: prompt_sections.iter().map(|section| section.tokens).sum(),
            system_prompt_tokens: prompt_sections
                .iter()
                .filter(|section| {
                    matches!(
                        section.name.as_str(),
                        "role"
                            | "role_identity"
                            | "conventions"
                            | "tool_instructions"
                            | "anti_patterns"
                            | "affect_guidance"
                            | "domain_context"
                            | "context_layer"
                            | "task_context"
                    )
                })
                .map(|section| section.tokens)
                .sum(),
            prompt_sections,
            tools_available: 0,
            tools_used: 0,
            tool_calls: Vec::new(),
            wall_time_ms: wall_ms,
            duration_ms: wall_ms,
            time_to_first_token_ms: 0,
            was_warm_start: false,
            iteration: 1,
            gate_passed: success,
            outcome: if success {
                "success".to_string()
            } else {
                "failure".to_string()
            },
            gate_errors: Vec::new(),
            model_used: model.to_string(),
            frequency,
            strategy_attempted: "none".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        tracing::info!(
            plan_id = %plan_id,
            task_id = %task_id,
            role = %role,
            model = %model,
            cost_usd = event.cost_usd,
            duration_ms = wall_ms,
            success = success,
            "agent efficiency event"
        );

        self.efficiency_events.push(event.clone());

        if let Err(e) = self.learning.append_efficiency_event(&event).await {
            tracing::warn!("failed to persist efficiency event: {e}");
        }
    }

    /// Construct and persist a failure efficiency event for a task that did not succeed.
    async fn emit_failure_efficiency_event(
        &mut self,
        plan_id: &str,
        task_id: &str,
        role: &str,
        model: &str,
        frequency: OperatingFrequency,
        wall_ms: u64,
        gate_errors: Vec<String>,
        strategy_attempted: &str,
        iteration: u32,
    ) {
        let prompt_sections = self
            .task_trackers
            .get(plan_id)
            .filter(|tracker| tracker.last_impl_task_id.as_deref() == Some(task_id))
            .map(|tracker| tracker.last_prompt_sections.clone())
            .unwrap_or_default();
        let event = AgentEfficiencyEvent {
            agent_id: format!("{plan_id}:{task_id}:failure"),
            role: role.to_string(),
            backend: "claude".to_string(),
            model: model.to_string(),
            plan_id: plan_id.to_string(),
            task_id: task_id.to_string(),
            input_tokens: 0,
            output_tokens: 0,
            reasoning_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: 0.0,
            cost_usd_without_cache: 0.0,
            total_prompt_tokens: prompt_sections.iter().map(|section| section.tokens).sum(),
            system_prompt_tokens: prompt_sections
                .iter()
                .filter(|section| {
                    matches!(
                        section.name.as_str(),
                        "role"
                            | "role_identity"
                            | "conventions"
                            | "tool_instructions"
                            | "anti_patterns"
                            | "affect_guidance"
                            | "domain_context"
                            | "context_layer"
                            | "task_context"
                    )
                })
                .map(|section| section.tokens)
                .sum(),
            prompt_sections,
            tools_available: 0,
            tools_used: 0,
            tool_calls: Vec::new(),
            wall_time_ms: wall_ms,
            duration_ms: wall_ms,
            time_to_first_token_ms: 0,
            was_warm_start: false,
            iteration,
            gate_passed: false,
            outcome: "failure".to_string(),
            gate_errors,
            model_used: model.to_string(),
            frequency,
            strategy_attempted: strategy_attempted.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        tracing::info!(
            plan_id = %plan_id,
            task_id = %task_id,
            role = %role,
            model = %model,
            strategy = %strategy_attempted,
            "failed-task efficiency event"
        );

        self.efficiency_events.push(event.clone());
        if let Err(e) = self.learning.append_efficiency_event(&event).await {
            tracing::warn!("failed to persist failure efficiency event: {e}");
        }
    }

    /// Build a tool manifest string for non-CLI agent backends.
    ///
    /// Uses `DynamicToolRegistry` (which includes MCP tools) if available,
    /// falling back to `StaticToolRegistry`. The result is a human-readable
    /// list of tool names and descriptions suitable for injection into a
    /// system prompt.
    fn build_tool_manifest(&self, role: AgentRole, denied_tools: Option<&[String]>) -> String {
        use roko_core::tool::ToolRegistry;
        let denied: Option<HashSet<&str>> =
            denied_tools.map(|tools| tools.iter().map(String::as_str).collect());

        let tools: Vec<roko_core::tool::ToolDef> = if let Some(ref registry) = self.tool_registry {
            registry
                .for_role(role)
                .into_iter()
                .filter(|tool| {
                    denied
                        .as_ref()
                        .is_none_or(|set| !set.contains(tool.name.as_str()))
                })
                .cloned()
                .collect()
        } else {
            let static_reg = StaticToolRegistry::new();
            static_reg
                .for_role(role)
                .into_iter()
                .filter(|tool| {
                    denied
                        .as_ref()
                        .is_none_or(|set| !set.contains(tool.name.as_str()))
                })
                .cloned()
                .collect()
        };

        if tools.is_empty() {
            return String::new();
        }

        let mut manifest = String::from("## Available Tools\n\n");
        manifest.push_str("You may call the following tools during this task:\n\n");
        for tool in &tools {
            manifest.push_str(&format!("- **{}**", tool.name));
            if !tool.description.is_empty() {
                manifest.push_str(&format!(": {}", tool.description));
            }
            manifest.push('\n');
        }
        manifest
    }

    /// Effective per-task timeout, taking the task TOML override when present.
    fn effective_task_timeout_ms(&self, task_def: Option<&crate::task_parser::TaskDef>) -> u64 {
        let secs = task_def
            .map(|td| td.timeout_secs)
            .unwrap_or(self.executor.config().task_timeout_secs);
        secs.saturating_mul(1000)
    }

    /// Load the current worktree diff as a list of changed paths.
    async fn git_changed_files(&self, exec_dir: &Path) -> Result<Vec<String>> {
        let output = tokio::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(exec_dir)
            .output()
            .await
            .with_context(|| format!("git status for {}", exec_dir.display()))?;

        if !output.status.success() {
            return Err(anyhow!(
                "git status failed for {}: {}",
                exec_dir.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        Ok(parse_git_status_changed_files(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }

    /// Enforce the task's declared write-file scope after successful execution.
    async fn verify_declared_write_files(
        &mut self,
        plan_id: &str,
        task_id: &str,
        allowed_files: &[String],
        exec_dir: &Path,
    ) -> Result<()> {
        if allowed_files.is_empty() {
            return Ok(());
        }

        let allowed: Vec<&str> = allowed_files.iter().map(String::as_str).collect();
        let changed = self.git_changed_files(exec_dir).await?;

        let mut unexpected = Vec::new();
        for path in &changed {
            let permitted = allowed.iter().any(|declared| {
                path == declared
                    || path.starts_with(&format!("{declared}/"))
                    || path.starts_with(&format!("{declared}\\"))
            });
            if !permitted {
                unexpected.push(path.clone());
            }
        }

        if !unexpected.is_empty() {
            let unexpected_list = unexpected.join(", ");
            let drift_ratio = if changed.is_empty() {
                0.0
            } else {
                unexpected.len() as f64 / changed.len() as f64
            };
            self.emit_conductor_signal(
                Kind::Metric,
                serde_json::json!({
                    "plan_id": plan_id,
                    "task_id": task_id,
                    "write_files": allowed_files,
                    "changed_files": changed,
                    "unexpected_files": unexpected,
                    "drift_ratio": drift_ratio,
                }),
            );
            return Err(anyhow!(
                "task {task_id} modified files outside write_files scope: {}",
                unexpected_list
            ));
        }

        Ok(())
    }

    async fn finish_task_post_processing(
        &mut self,
        plan_id: &str,
        task_id: &str,
        task_def: Option<&crate::task_parser::TaskDef>,
        task_text: &str,
        role_instruction: &str,
        selected_model: &str,
        result: &AgentResult,
        exec_dir: &Path,
    ) -> Result<()> {
        let Some(td) = task_def else {
            return Ok(());
        };

        if let Err((failed_task_id, phase, command, error_output)) =
            self.run_verify_steps(&td.id, &td.verify, exec_dir).await
        {
            let msg = td
                .verify
                .iter()
                .find(|step| step.command == command)
                .and_then(|step| step.fail_msg.as_deref())
                .unwrap_or("verification failed");
            let output_tail = result
                .output
                .body
                .as_text()
                .ok()
                .map(|text| tail_output_lines(text, TASK_FAILURE_OUTPUT_TAIL_LINES));
            self.observe_cascade_router(plan_id, task_id, task_def, selected_model, 0.0);
            let error = anyhow!(
                "verify failed for {failed_task_id}: {command} — {msg}; stderr/stdout:\n{error_output}"
            );
            return Err(with_task_failure_context(
                error,
                &failed_task_id,
                td.status.as_str(),
                &phase,
                output_tail.as_deref(),
            ));
        }

        self.verify_declared_write_files(plan_id, &td.id, &td.files, exec_dir)
            .await?;

        let mut task_files = Vec::new();
        let mut seen_files = HashSet::new();
        if let Ok(changed_files) = self.git_changed_files(exec_dir).await {
            for file in changed_files {
                if seen_files.insert(file.clone()) {
                    task_files.push(file);
                }
            }
        }
        for file in &td.files {
            if seen_files.insert(file.clone()) {
                task_files.push(file.clone());
            }
        }

        let symbols = extract_task_symbols(task_text);
        let prompt_hash = roko_core::ContentHash::of(role_instruction.as_bytes()).to_hex();
        let request = SkillExtractionRequest::new(
            task_files,
            td.tier.clone(),
            symbols,
            selected_model.to_string(),
            prompt_hash,
            Vec::new(),
        );
        if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
            tracker.last_skill_request = Some(request);
        }

        Ok(())
    }
}

fn render_knowledge_context(entries: Vec<KnowledgeEntry>) -> String {
    use std::fmt::Write as _;

    let mut content = String::from("## Durable Knowledge\n");
    for (idx, entry) in entries.into_iter().enumerate() {
        let confidence = entry.confidence.clamp(0.0, 1.0);
        let source = entry.source.as_deref().unwrap_or("-");
        let tags = if entry.tags.is_empty() {
            String::from("-")
        } else {
            entry.tags.join(", ")
        };
        let episodes = if entry.source_episodes.is_empty() {
            String::from("-")
        } else {
            entry.source_episodes.join(", ")
        };

        if let Some(warning) = entry.refutation_warning() {
            let _ = write!(
                content,
                "\n### {}. Warning {:?} ({confidence:.2})\nSource: {source}\nWeight: {:.2}\nTags: {tags}\nEpisodes: {episodes}\n{warning}\n```\n{}\n```\n",
                idx + 1,
                entry.kind,
                entry.confidence_weight,
                entry.content.trim(),
            );
        } else {
            let _ = write!(
                content,
                "\n### {}. {:?} ({confidence:.2})\nSource: {source}\nTags: {tags}\nEpisodes: {episodes}\n```\n{}\n```\n",
                idx + 1,
                entry.kind,
                entry.content.trim(),
            );
        }
    }

    content
}

fn build_success_knowledge_entry(
    plan_id: &str,
    task_id: &str,
    task_def: Option<&crate::task_parser::TaskDef>,
    result: &AgentResult,
    model: &str,
    episode_id: &str,
) -> KnowledgeEntry {
    let kind = infer_success_knowledge_kind(task_def, result);
    let title = task_def
        .map(|task| task.title.trim())
        .filter(|title| !title.is_empty())
        .unwrap_or(task_id);
    let description = task_def
        .and_then(|task| task.description.as_deref())
        .map(str::trim)
        .filter(|description| !description.is_empty());
    let output_summary = result
        .output
        .body
        .as_text()
        .ok()
        .map(|text| truncate_doc_snippet(text, 600))
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| {
            format!(
                "Task completed with {} output.",
                result.output.kind.as_str()
            )
        });

    let mut content = format!("Task `{task_id}` succeeded: {title}.");
    if let Some(description) = description {
        content.push_str("\n\nWhy it worked:\n");
        content.push_str(description);
    }
    content.push_str("\n\nSuccessful outcome:\n");
    content.push_str(&output_summary);

    let mut tags = vec![
        "task-success".to_string(),
        format!("plan:{plan_id}"),
        format!("task:{task_id}"),
        kind.as_str().to_string(),
    ];
    if let Some(task) = task_def {
        tags.push(format!("tier:{}", task.tier));
        if task.files.len() > 1 {
            tags.push("multi-file".to_string());
        } else if let Some(path) = task.files.first() {
            tags.push(format!("file:{path}"));
        }
    }
    if let Some(emotional_tag) = result.output.emotional_tag.as_ref() {
        tags.push(format!(
            "emotion_trigger:{}",
            emotional_tag.trigger.trim().to_ascii_lowercase()
        ));
        tags.push(emotion_valence_tag(emotional_tag).to_string());
        tags.push(emotion_arousal_tag(emotional_tag).to_string());
    }
    tags.sort();
    tags.dedup();

    KnowledgeEntry {
        id: format!("task-success:{plan_id}:{task_id}:{}", result.output.id),
        kind,
        source: Some("task-success".to_string()),
        content,
        confidence: 0.75,
        confidence_weight: 0.75,
        refuted_insight_id: None,
        refutation_evidence: None,
        source_episodes: vec![episode_id.to_string()],
        tags,
        source_model: Some(model.to_string()),
        model_generality: 0.9,
        created_at: chrono::Utc::now(),
        half_life_days: kind.default_half_life_days(),
        tier: KnowledgeTier::Transient,
        emotional_tag: result.output.emotional_tag.clone(),
        emotional_provenance: result
            .output
            .emotional_tag
            .as_ref()
            .map(EmotionalProvenance::from_tag),
        hdc_vector: None,
    }
}

fn emotion_valence_tag(tag: &roko_core::EmotionalTag) -> &'static str {
    if tag.pad.pleasure >= 0.2 {
        "emotion_valence:positive"
    } else if tag.pad.pleasure <= -0.2 {
        "emotion_valence:negative"
    } else {
        "emotion_valence:neutral"
    }
}

fn emotion_arousal_tag(tag: &roko_core::EmotionalTag) -> &'static str {
    if tag.pad.arousal >= 0.35 {
        "emotion_arousal:high"
    } else if tag.pad.arousal <= -0.35 {
        "emotion_arousal:low"
    } else {
        "emotion_arousal:mid"
    }
}

fn infer_success_knowledge_kind(
    task_def: Option<&crate::task_parser::TaskDef>,
    result: &AgentResult,
) -> KnowledgeKind {
    let mut hint_text = task_def
        .map(|task| {
            format!(
                "{} {}",
                task.title,
                task.description.as_deref().unwrap_or_default()
            )
        })
        .unwrap_or_default();
    if let Ok(output_text) = result.output.body.as_text() {
        hint_text.push(' ');
        hint_text.push_str(output_text);
    }
    let hint_text = hint_text.to_ascii_lowercase();
    let looks_reusable = task_def.is_some_and(|task| task.files.len() > 1)
        || [
            "refactor",
            "standardize",
            "reuse",
            "reusable",
            "pattern",
            "workflow",
            "pipeline",
            "template",
            "guardrail",
            "strategy",
        ]
        .iter()
        .any(|needle| hint_text.contains(needle));

    if looks_reusable {
        KnowledgeKind::Heuristic
    } else {
        KnowledgeKind::Insight
    }
}

fn parse_git_status_changed_files(status: &str) -> Vec<String> {
    let mut changed: Vec<String> = status
        .lines()
        .filter_map(|line| {
            if line.len() < 4 {
                return None;
            }
            let path = line[3..].trim();
            if path.is_empty() {
                None
            } else if let Some((_, new_path)) = path.rsplit_once(" -> ") {
                Some(new_path.trim().to_string())
            } else {
                Some(path.to_string())
            }
        })
        .collect();
    changed.sort();
    changed.dedup();
    changed
}

async fn git_commit_all_if_needed(workspace: &Path, message: &str) -> Result<bool> {
    let add_output = tokio::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(workspace)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .context("spawn git add -A")?;

    if !add_output.status.success() {
        let stderr = String::from_utf8_lossy(&add_output.stderr);
        return Err(anyhow!("git add -A failed: {stderr}"));
    }

    let diff_output = tokio::process::Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(workspace)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .context("spawn git diff --cached")?;

    if diff_output.status.success() {
        return Ok(false);
    }

    if diff_output.status.code() != Some(1) {
        let stderr = String::from_utf8_lossy(&diff_output.stderr);
        return Err(anyhow!("git diff --cached failed: {stderr}"));
    }

    let commit_output = tokio::process::Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(workspace)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "roko")
        .env("GIT_AUTHOR_EMAIL", "roko@nunchi.dev")
        .env("GIT_COMMITTER_NAME", "roko")
        .env("GIT_COMMITTER_EMAIL", "roko@nunchi.dev")
        .output()
        .await
        .context("spawn git commit")?;

    if !commit_output.status.success() {
        let stderr = String::from_utf8_lossy(&commit_output.stderr);
        return Err(anyhow!("git commit failed: {stderr}"));
    }

    Ok(true)
}

async fn git_merge_branch_into(workspace: &Path, branch: &str) -> Result<()> {
    let output = tokio::process::Command::new("git")
        .args(["merge", "--no-ff", "--no-edit", branch])
        .current_dir(workspace)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .context("spawn git merge")?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let details = if stderr.is_empty() { stdout } else { stderr };
    Err(anyhow!("git merge failed: {details}"))
}

fn is_meaningful_output(output: &Engram) -> bool {
    match &output.body {
        Body::Empty => false,
        Body::Text(text) => !text.trim().is_empty(),
        Body::Json(value) => value.as_str().is_none_or(|s| !s.trim().is_empty()),
        Body::Bytes(bytes) => !bytes.is_empty(),
    }
}

// ─── Role-specific system prompts ────────────────────────────────────────

fn default_worktree_manager(workdir: &Path) -> WorktreeManager {
    let config = WorktreeConfig {
        repo_root: workdir.to_path_buf(),
        base_branch: "HEAD".to_string(),
        worktrees_root: workdir.join(".roko").join("worktrees"),
        max_live: None,
        idle_ttl: Duration::from_secs(DEFAULT_WORKTREE_IDLE_TTL_SECS),
    };
    WorktreeManager::new(config)
}

const fn claude_skip_permissions_for_role(role: AgentRole) -> bool {
    let perms = role.tool_permissions();
    perms.exec || perms.write || perms.git
}

fn normalize_resume_session(session_id: Option<String>) -> Option<String> {
    session_id.and_then(|id| {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

/// Convert a `ServerEvent` into a `DashboardEvent` for the state hub.
///
/// Returns `None` for event types that don't affect the dashboard snapshot
/// (e.g. deployment events, webhook signals).
fn server_event_to_dashboard(
    event: &crate::serve::events::ServerEvent,
) -> Option<roko_core::DashboardEvent> {
    use crate::serve::events::ServerEvent;
    use roko_core::DashboardEvent;

    match event {
        ServerEvent::PlanStarted { plan_id } => Some(DashboardEvent::PlanStarted {
            plan_id: plan_id.clone(),
        }),
        ServerEvent::PlanCompleted { plan_id, success } => Some(DashboardEvent::PlanCompleted {
            plan_id: plan_id.clone(),
            success: *success,
        }),
        ServerEvent::AgentSpawned { agent_id, role } => Some(DashboardEvent::AgentSpawned {
            agent_id: agent_id.clone(),
            role: role.clone(),
        }),
        ServerEvent::AgentOutput {
            agent_id, content, ..
        } => Some(DashboardEvent::AgentOutput {
            agent_id: agent_id.clone(),
            content: content.clone(),
        }),
        ServerEvent::GateResult {
            plan_id,
            task_id,
            gate,
            passed,
        } => Some(DashboardEvent::GateResult {
            plan_id: plan_id.clone(),
            task_id: task_id.clone(),
            gate: gate.clone(),
            passed: *passed,
        }),
        ServerEvent::Execution { plan_id, event } => {
            use crate::serve::events::ExecutionEvent;
            match event {
                ExecutionEvent::TaskStarted { task_id, phase } => {
                    Some(DashboardEvent::TaskStarted {
                        plan_id: plan_id.clone(),
                        task_id: task_id.clone(),
                        phase: phase.clone(),
                    })
                }
                ExecutionEvent::TaskCompleted { task_id, outcome } => {
                    Some(DashboardEvent::TaskCompleted {
                        plan_id: plan_id.clone(),
                        task_id: task_id.clone(),
                        outcome: outcome.clone(),
                    })
                }
                ExecutionEvent::TaskPhaseChanged {
                    task_id,
                    old_phase,
                    new_phase,
                } => Some(DashboardEvent::TaskPhaseChanged {
                    plan_id: plan_id.clone(),
                    task_id: task_id.clone(),
                    old_phase: old_phase.clone(),
                    new_phase: new_phase.clone(),
                }),
                ExecutionEvent::GateResult {
                    task_id,
                    gate,
                    passed,
                    ..
                } => Some(DashboardEvent::GateResult {
                    plan_id: plan_id.clone(),
                    task_id: task_id.clone(),
                    gate: gate.clone(),
                    passed: *passed,
                }),
                _ => None,
            }
        }
        ServerEvent::PhaseTransition { plan_id, from, to } => {
            Some(DashboardEvent::PhaseTransition {
                plan_id: plan_id.clone(),
                from: from.clone(),
                to: to.clone(),
            })
        }
        ServerEvent::EfficiencyEvent {
            plan_id,
            task_id,
            metric,
            value,
        } => Some(DashboardEvent::EfficiencyEvent {
            plan_id: plan_id.clone(),
            task_id: task_id.clone(),
            metric: metric.clone(),
            value: *value,
        }),
        ServerEvent::Error { message } => Some(DashboardEvent::Error {
            message: message.clone(),
        }),
        // Deployment, webhook, run, and operation events don't affect the dashboard snapshot.
        _ => None,
    }
}

fn now_unix_ms_i64() -> i64 {
    #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0_i64, |d| d.as_millis() as i64)
    }
}

fn mechanical_tier_model(config: &Config) -> Option<String> {
    config.agent.tier_models.get("mechanical").cloned()
}

fn detect_cost_anomaly_override(
    detector: &mut AnomalyDetector,
    turn_cost: f64,
    config: &Config,
) -> Option<String> {
    match detector.check_cost(turn_cost) {
        Some(Anomaly::CostSpike { z_score }) => {
            let mechanical_model = mechanical_tier_model(config);
            match mechanical_model.as_deref() {
                Some(model) => {
                    tracing::warn!(
                        z_score,
                        forced_model = %model,
                        "cost anomaly detected; forcing cheaper model on next routed turn"
                    );
                }
                None => {
                    tracing::warn!(
                        z_score,
                        "cost anomaly detected but no mechanical tier model is configured"
                    );
                }
            }
            mechanical_model
        }
        _ => None,
    }
}

fn task_dispatch_conventions(task_def: Option<&crate::task_parser::TaskDef>) -> Option<String> {
    let task_def = task_def?;
    let mut sections = Vec::new();

    if !task_def.files.is_empty() {
        let mut write_scope = String::from(
            "Honor the declared write scope strictly. Only create, edit, move, or delete files in this allowlist unless the user explicitly expands it:",
        );
        for path in &task_def.files {
            write_scope.push_str("\n- ");
            write_scope.push_str(path);
        }
        sections.push(write_scope);
    }

    if let Some(max_loc) = task_def.max_loc {
        sections.push(format!(
            "Keep the total code delta within roughly {max_loc} lines of change unless verification requires a tightly scoped follow-up."
        ));
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

fn build_system_prompt(
    role: AgentRole,
    plan_id: &str,
    task: &str,
    tools_csv: &str,
    task_def: Option<&crate::task_parser::TaskDef>,
) -> String {
    build_system_prompt_with_context(role, plan_id, task, tools_csv, None, None, task_def)
}

fn build_system_prompt_with_context(
    role: AgentRole,
    plan_id: &str,
    task: &str,
    tools_csv: &str,
    context_layer: Option<&str>,
    affect_state: Option<PadState>,
    task_def: Option<&crate::task_parser::TaskDef>,
) -> String {
    let mut task_context = TaskContext::new(task)
        .with_plan_id(plan_id)
        .with_workspace("roko-cli orchestration");
    if let Some(context) = context_layer.filter(|context| !context.trim().is_empty()) {
        task_context = task_context.with_context(context);
    }
    build_role_system_prompt(
        role,
        task_context,
        tools_csv,
        PromptBuildOptions {
            affect_state,
            extra_conventions: task_dispatch_conventions(task_def),
            ..PromptBuildOptions::default()
        },
    )
}

fn build_system_prompt_with_context_validated(
    role: AgentRole,
    plan_id: &str,
    task: &str,
    tools_csv: &str,
    context_layer: Option<&str>,
    affect_state: Option<PadState>,
    task_def: Option<&crate::task_parser::TaskDef>,
    relevant_skills: &[Skill],
    context_window_tokens: usize,
    section_effectiveness: Option<&SectionEffectivenessRegistry>,
) -> Result<String> {
    let mut task_context = TaskContext::new(task)
        .with_plan_id(plan_id)
        .with_workspace("roko-cli orchestration");
    if let Some(context) = context_layer.filter(|context| !context.trim().is_empty()) {
        task_context = task_context.with_context(context);
    }
    build_role_system_prompt_validated(
        role,
        task_context,
        tools_csv,
        PromptBuildOptions {
            affect_state,
            extra_conventions: task_dispatch_conventions(task_def),
            relevant_skills: relevant_skills.to_vec(),
            ..PromptBuildOptions::default()
        },
        context_window_tokens,
        section_effectiveness,
    )
}

fn effective_context_window_tokens(config: &Config) -> usize {
    config.prompt.token_budget
}

fn build_relevant_context_layer(context_sections: &[PromptSection]) -> Option<String> {
    let content = context_sections
        .iter()
        .map(|section| section.content.trim())
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    if content.is_empty() {
        None
    } else {
        Some(format!("## Relevant Context\n\n{content}"))
    }
}

fn adjust_priority_from_section_learning(
    priority: SectionPriority,
    section_name: &str,
    role: &str,
    registry: &SectionEffectivenessRegistry,
) -> SectionPriority {
    let next = match registry.recommend_priority_change(section_name, role) {
        PriorityChange::Increase => (priority as u8).saturating_add(1),
        PriorityChange::Decrease => (priority as u8).saturating_sub(1),
        PriorityChange::NoChange | PriorityChange::InsufficientData => priority as u8,
    };
    match next {
        0 => SectionPriority::Low,
        1 => SectionPriority::Normal,
        2 => SectionPriority::High,
        _ => SectionPriority::Critical,
    }
}

fn apply_section_effectiveness_to_prompt_section(
    mut section: PromptSection,
    role: &str,
    registry: &SectionEffectivenessRegistry,
) -> PromptSection {
    section.priority =
        adjust_priority_from_section_learning(section.priority, &section.name, role, registry);
    section
}

fn prompt_section_meta_from_sections(
    sections: &[Engram],
    prompt: &Engram,
) -> Vec<PromptSectionMeta> {
    let included = prompt.lineage.iter().copied().collect::<HashSet<_>>();
    sections
        .iter()
        .filter_map(|signal| {
            PromptSection::from_signal(signal)
                .ok()
                .map(|section| (signal, section))
        })
        .map(|(signal, original)| {
            let rendered = original.clone().enforce_hard_cap();
            let is_included = included.contains(&signal.id);
            let was_truncated = rendered.content != original.content;
            let tokens = if is_included {
                rendered.estimated_tokens() as u64
            } else {
                0
            };
            PromptSectionMeta {
                name: rendered.name,
                tokens,
                priority: rendered.priority as u8,
                was_truncated: is_included && was_truncated,
                was_dropped: !is_included,
            }
        })
        .collect()
}

fn build_daimon_context_section(
    affect_state: PadState,
    behavioral_state: roko_core::BehavioralState,
) -> Option<PromptSection> {
    let pad_magnitude = affect_state.pleasure.abs()
        + affect_state.arousal.abs()
        + affect_state.dominance.abs()
        + affect_state.somatic_intensity;
    if pad_magnitude < 0.35 {
        return None;
    }

    let mut content = format!(
        "## Daimon state\nBehavioral state: {behavioral_state:?}\nPAD: pleasure={:.2}, arousal={:.2}, dominance={:.2}\n",
        affect_state.pleasure, affect_state.arousal, affect_state.dominance
    );
    if affect_state.somatic_intensity >= 0.15 {
        content.push_str(&format!(
            "Somatic hint: valence={:.2}, intensity={:.2}\n",
            affect_state.somatic_valence, affect_state.somatic_intensity
        ));
        if affect_state.somatic_valence <= -0.2 {
            content
                .push_str("Interpretation: slow down, prefer caution, and verify risky moves.\n");
        } else if affect_state.somatic_valence >= 0.2 {
            content.push_str(
                "Interpretation: this strategy region has positive prior outcomes; keep momentum without skipping checks.\n",
            );
        }
    }

    Some(
        PromptSection::new("daimon-state", content)
            .with_priority(SectionPriority::Normal)
            .with_placement(Placement::Middle)
            .with_bidder(AttentionBidder::Daimon)
            .with_hard_cap(256),
    )
}

impl PlanRunner {
    fn current_pad_state(&self) -> PadState {
        PadState::from(self.daimon.query().pad)
    }

    fn current_daimon_policy(&self) -> DaimonPolicy {
        let affect = self.daimon.query();
        DaimonPolicy::new(affect.confidence, affect.behavioral_state)
    }

    fn apply_daimon_tool_policy_csv(&self, tools_csv: &str) -> String {
        let policy = self.current_daimon_policy();
        let Some(registry) = self.tool_registry.as_deref() else {
            return tools_csv.to_string();
        };
        if !matches!(
            policy.behavioral_state,
            roko_core::BehavioralState::Struggling | roko_core::BehavioralState::Resting
        ) {
            return tools_csv.to_string();
        }

        tools_csv
            .split(',')
            .filter(|tool| !tool.is_empty())
            .filter(|tool| {
                registry
                    .get(tool)
                    .is_none_or(|def| !(def.permission.network || def.permission.git))
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    fn should_replan_after_task_failure(&self) -> bool {
        let policy = self.current_daimon_policy();
        matches!(
            policy.behavioral_state,
            roko_core::BehavioralState::Struggling
        ) && policy.affect_confidence < 0.45
    }

    fn apply_knowledge_tier_feedback(&mut self, plan_id: &str) {
        let Some(tracker) = self.task_trackers.get(plan_id) else {
            return;
        };
        if tracker.last_context_knowledge_ids.is_empty() || tracker.last_gate_verdicts.is_empty() {
            return;
        }

        let touched_ids = tracker
            .last_context_knowledge_ids
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let verdicts = tracker.last_gate_verdicts.clone();

        if let Err(err) = self.knowledge_store.update_entries(|entry| {
            if !touched_ids.contains(&entry.id) {
                return false;
            }

            match TierProgression::evaluate_tier_progression(entry, &verdicts) {
                TierProgressionDecision::Promote(tier) | TierProgressionDecision::Demote(tier)
                    if entry.tier != tier =>
                {
                    entry.tier = tier;
                    true
                }
                TierProgressionDecision::ReviewExpiry => {
                    if entry.tags.iter().any(|tag| tag == "expiry-review") {
                        false
                    } else {
                        entry.tags.push("expiry-review".to_string());
                        true
                    }
                }
                _ => false,
            }
        }) {
            tracing::warn!(
                plan_id = %plan_id,
                error = %err,
                "failed to apply knowledge tier feedback"
            );
        }
    }

    fn current_task_strategy(
        &self,
        plan_id: &str,
        task_id: &str,
        task_def: Option<&crate::task_parser::TaskDef>,
    ) -> StrategyCoordinates {
        coding_strategy_coordinates(self, plan_id, task_id, task_def)
    }

    fn stamp_task_strategy_metadata(
        &self,
        episode: &mut Episode,
        plan_id: &str,
        task_id: &str,
        task_def: Option<&crate::task_parser::TaskDef>,
    ) -> StrategyCoordinates {
        let strategy = self.current_task_strategy(plan_id, task_id, task_def);
        episode.extra.insert(
            "strategy_coordinates".to_string(),
            serde_json::to_value(strategy)
                .expect("strategy coordinates serialization should not fail"),
        );
        episode.extra.insert(
            "strategy_space_domain".to_string(),
            serde_json::json!(self.daimon.strategy_space().domain.clone()),
        );
        episode.extra.insert(
            "strategy_space_dimensions".to_string(),
            serde_json::to_value(self.daimon.strategy_space().labels())
                .expect("strategy space labels serialization should not fail"),
        );
        episode.extra.insert(
            "crate_familiarity".to_string(),
            serde_json::json!(self.crate_familiarity_tracker.score_for_task(task_def)),
        );
        if let Some(task_def) = task_def {
            episode.extra.insert(
                "task_tier".to_string(),
                serde_json::json!(task_def.tier.clone()),
            );
            episode.extra.insert(
                "file_count".to_string(),
                serde_json::json!(task_def.files.len()),
            );
            episode.extra.insert(
                "verify_count".to_string(),
                serde_json::json!(task_def.verify.len()),
            );
            episode.extra.insert(
                "dependency_count".to_string(),
                serde_json::json!(task_def.depends_on.len()),
            );
            if let Some(max_loc) = task_def.max_loc {
                episode
                    .extra
                    .insert("max_loc".to_string(), serde_json::json!(max_loc));
            }
            episode.extra.insert(
                "files".to_string(),
                serde_json::json!(task_def.files.clone()),
            );
        }
        strategy
    }

    fn emit_somatic_marker_fired_event(
        &self,
        plan_id: &str,
        task_id: &str,
        signal: &SomaticSignal,
        strategy_param: &str,
    ) {
        if !signal.should_emit_event() {
            return;
        }

        self.learning_event_bus
            .publish(AgentEvent::SomaticMarkerFired {
                task_id: task_id.to_string(),
                valence: signal.valence,
                intensity: signal.intensity,
                source_episode_count: signal.source_episodes.len(),
            });
        self.emit_server_event(crate::serve::events::ServerEvent::SomaticMarkerFired {
            plan_id: plan_id.to_string(),
            task_id: task_id.to_string(),
            valence: signal.valence,
            intensity: signal.intensity,
            source_episodes: signal.source_episodes.clone(),
            strategy_param: strategy_param.to_string(),
        });
    }

    fn stamp_episode_affect(&self, episode: &mut Episode, trigger: &str, output: Option<&Engram>) {
        if episode.emotional_tag.is_some() {
            return;
        }
        if let Some(tag) = output.and_then(|engram| engram.emotional_tag.clone()) {
            episode.emotional_tag = Some(tag);
            return;
        }
        episode.emotional_tag = Some(self.daimon.emotional_tag(trigger));
    }

    /// Build the strategist system prompt for the Enriching phase.
    ///
    /// This assembles the same 6-layer system prompt as other agent dispatches,
    /// but injects the plan's task context and inline read_files content so the
    /// strategist sees the full enrichment surface before dispatch.
    fn build_enrichment_system_prompt(&self, plan_id: &str) -> String {
        let plan_dir = plans_dir(&self.workdir).join(plan_id);
        let tasks_file = self
            .task_trackers
            .get(plan_id)
            .map(|tracker| &tracker.tasks_file);

        let mut context_summary = String::new();
        if let Some(tasks_file) = tasks_file {
            context_summary.push_str(&format!(
                "Plan {plan_id} enrichment context\n\n\
                 Use this task inventory and inline file context to prepare execution-ready notes.\n"
            ));
            for task in &tasks_file.tasks {
                context_summary.push_str(&format!(
                    "\n## Task {} - {}\n\
                     Status: {}\n\
                     Tier: {}\n",
                    task.id, task.title, task.status, task.tier
                ));
                if !task.files.is_empty() {
                    context_summary.push_str("Files to modify:\n");
                    for file in &task.files {
                        context_summary.push_str(&format!("- {file}\n"));
                    }
                }
                context_summary.push_str(&task.build_prompt(plan_id, &self.workdir));
                context_summary.push('\n');
            }
        } else {
            context_summary.push_str(&format!(
                "Plan {plan_id} has no tasks.toml. Enrich the plan from the available plan.md and repository context."
            ));
        }

        let tools_csv =
            claude_tool_allowlist_with(AgentRole::Strategist, self.tool_registry.as_deref());
        build_role_system_prompt(
            AgentRole::Strategist,
            TaskContext::new(format!("Enrich plan {plan_id} before agent dispatch"))
                .with_plan_id(plan_id)
                .with_workspace(plan_dir.display().to_string())
                .with_domain_notes(context_summary),
            tools_csv,
            PromptBuildOptions {
                affect_state: Some(self.current_pad_state()),
                extra_conventions: Some(
                    "Treat enrichment as a pre-dispatch analysis step. Preserve task context, read_files, and dependency ordering so later agent turns receive accurate context.".to_string(),
                ),
                extra_anti_patterns: vec![
                    "Do not invent file contents, dependencies, or task requirements that are not present in the plan context.".to_string(),
                    "Do not skip read_files: if a task declares context files, they must be reflected in the enrichment summary.".to_string(),
                ],
                relevant_skills: Vec::new(),
            },
        )
    }
}

fn default_task_category(role: &str) -> &'static str {
    if role.eq_ignore_ascii_case("Implementer") || role.eq_ignore_ascii_case("AutoFixer") {
        "implementation"
    } else if role.eq_ignore_ascii_case("Strategist") {
        "planning"
    } else if role.eq_ignore_ascii_case("Auditor") {
        "review"
    } else if role.eq_ignore_ascii_case("Scribe") {
        "documentation"
    } else {
        "implementation"
    }
}

fn claude_tool_allowlist(role: AgentRole) -> String {
    claude_tool_allowlist_with(role, None)
}

fn claude_tool_allowlist_with(
    role: AgentRole,
    dynamic_registry: Option<&roko_agent::mcp::DynamicToolRegistry>,
) -> String {
    use roko_core::tool::ToolRegistry;
    let tools: Vec<roko_core::tool::ToolDef> = if let Some(registry) = dynamic_registry {
        registry.for_role(role).into_iter().cloned().collect()
    } else {
        let registry = StaticToolRegistry::new();
        registry.for_role(role).into_iter().cloned().collect()
    };
    match ClaudeTranslator.render_tools(&tools) {
        RenderedTools::CliFlag(csv) => csv,
        _ => String::new(),
    }
}

fn claude_task_tool_allowlist_with(
    role: AgentRole,
    allowed_tools: Option<&[String]>,
    denied_tools: Option<&[String]>,
    dynamic_registry: Option<&roko_agent::mcp::DynamicToolRegistry>,
) -> String {
    use roko_core::tool::ToolRegistry;

    let allowed: Option<HashSet<&str>> =
        allowed_tools.map(|tools| tools.iter().map(String::as_str).collect());
    let denied: Option<HashSet<&str>> =
        denied_tools.map(|tools| tools.iter().map(String::as_str).collect());
    let tools: Vec<roko_core::tool::ToolDef> = if let Some(registry) = dynamic_registry {
        registry
            .for_role(role)
            .into_iter()
            .filter(|tool| {
                allowed
                    .as_ref()
                    .is_none_or(|set| set.contains(tool.name.as_str()))
            })
            .filter(|tool| {
                denied
                    .as_ref()
                    .is_none_or(|set| !set.contains(tool.name.as_str()))
            })
            .cloned()
            .collect()
    } else {
        let registry = StaticToolRegistry::new();
        registry
            .for_role(role)
            .into_iter()
            .filter(|tool| {
                allowed
                    .as_ref()
                    .is_none_or(|set| set.contains(tool.name.as_str()))
            })
            .filter(|tool| {
                denied
                    .as_ref()
                    .is_none_or(|set| !set.contains(tool.name.as_str()))
            })
            .cloned()
            .collect()
    };

    match ClaudeTranslator.render_tools(&tools) {
        RenderedTools::CliFlag(csv) => csv,
        _ => String::new(),
    }
}

/// Summary of how tightly a review output stays anchored to the task spec.
#[derive(Debug, Clone, PartialEq)]
struct ReviewDriftReport {
    matched: usize,
    expected: usize,
    missing: Vec<String>,
}

impl ReviewDriftReport {
    fn coverage(&self) -> f64 {
        if self.expected == 0 {
            1.0
        } else {
            self.matched as f64 / self.expected as f64
        }
    }

    fn drifted(&self) -> bool {
        self.expected > 0 && self.coverage() < 0.35
    }
}

/// Render the task spec into a reviewable summary block.
fn task_spec_summary(tasks_file: &TasksFile) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "[meta]\nplan = {}\niteration = {}\ntotal = {}\ndone = {}\nstatus = {}\nmax_parallel = {}\nestimated_total_minutes = {}\n",
        tasks_file.meta.plan,
        tasks_file.meta.iteration,
        tasks_file.meta.total,
        tasks_file.meta.done,
        tasks_file.meta.status,
        tasks_file.meta.max_parallel,
        tasks_file.meta.estimated_total_minutes,
    ));

    for task in &tasks_file.tasks {
        out.push_str(&format!("\n### {} - {}\n", task.id, task.title));
        out.push_str(&format!("tier = {}\n", task.tier));
        if !task.files.is_empty() {
            out.push_str("files:\n");
            for file in &task.files {
                out.push_str(&format!("- {file}\n"));
            }
        }
        if !task.depends_on.is_empty() {
            out.push_str(&format!("depends_on = {}\n", task.depends_on.join(", ")));
        }
        if !task.depends_on_plan.is_empty() {
            out.push_str(&format!(
                "depends_on_plan = {}\n",
                task.depends_on_plan.join(", ")
            ));
        }
        if !task.acceptance.is_empty() {
            out.push_str("acceptance:\n");
            for item in &task.acceptance {
                out.push_str(&format!("- {item}\n"));
            }
        }
        if !task.verify.is_empty() {
            out.push_str("verify:\n");
            for step in &task.verify {
                out.push_str(&format!("- [{}] {}\n", step.phase, step.command));
            }
        }
    }

    out
}

fn significant_terms(text: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "the",
        "and",
        "for",
        "with",
        "from",
        "into",
        "that",
        "this",
        "task",
        "plan",
        "should",
        "must",
        "have",
        "has",
        "are",
        "was",
        "were",
        "will",
        "would",
        "could",
        "can",
        "done",
        "make",
        "build",
        "update",
        "implement",
        "review",
        "please",
        "then",
        "than",
        "when",
    ];

    let mut seen = HashSet::new();
    let mut terms = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '/') {
        let term = raw.trim().to_lowercase();
        if term.len() < 4 || STOP_WORDS.contains(&term.as_str()) {
            continue;
        }
        if seen.insert(term.clone()) {
            terms.push(term);
        }
    }
    terms
}

fn review_drift_report(tasks_file: &TasksFile, output: &str) -> Option<ReviewDriftReport> {
    let lower = output.to_lowercase();
    let mut expected = Vec::new();
    let mut seen = HashSet::new();

    let mut push_expected = |value: String| {
        let value = value.trim().to_lowercase();
        if value.is_empty() {
            return;
        }
        if seen.insert(value.clone()) {
            expected.push(value);
        }
    };

    for task in &tasks_file.tasks {
        push_expected(task.id.clone());
        push_expected(task.title.clone());

        for term in significant_terms(&task.title) {
            push_expected(term);
        }

        for file in &task.files {
            push_expected(file.clone());
            if let Some(name) = std::path::Path::new(file)
                .file_name()
                .and_then(|n| n.to_str())
            {
                push_expected(name.to_string());
            }
        }

        for verify in &task.verify {
            push_expected(verify.phase.clone());
        }

        for acceptance in &task.acceptance {
            push_expected(acceptance.clone());
            for term in significant_terms(acceptance) {
                push_expected(term);
            }
        }

        for anti_pattern in task
            .context
            .as_ref()
            .map(|ctx| ctx.anti_patterns.iter())
            .into_iter()
            .flatten()
        {
            push_expected(anti_pattern.clone());
            for term in significant_terms(anti_pattern) {
                push_expected(term);
            }
        }
    }

    if expected.is_empty() {
        return None;
    }

    let mut matched = 0usize;
    let mut missing = Vec::new();
    for anchor in &expected {
        if lower.contains(anchor) {
            matched += 1;
        } else {
            missing.push(anchor.clone());
        }
    }

    Some(ReviewDriftReport {
        matched,
        expected: expected.len(),
        missing,
    })
}

/// Parse a review verdict from agent output text.
///
/// Looks for `verdict = "approve"` / `verdict = "revise"` patterns,
/// falls back to keyword matching. Returns `true` for approve.
fn parse_review_verdict(output: &str) -> bool {
    let lower = output.to_lowercase();
    // Structured verdict
    if lower.contains("verdict = \"approve\"") || lower.contains("verdict: approve") {
        return true;
    }
    if lower.contains("verdict = \"revise\"")
        || lower.contains("verdict: revise")
        || lower.contains("verdict = \"reject\"")
        || lower.contains("verdict: reject")
    {
        return false;
    }
    // Keyword fallback
    if lower.contains("approved") || lower.contains("lgtm") || lower.contains("looks good") {
        return true;
    }
    if lower.contains("revise") || lower.contains("reject") || lower.contains("rework") {
        return false;
    }
    // Default: approve (don't block pipeline on ambiguous output)
    true
}

/// Convert a `TaskDef` (from the CLI's task_parser) into a `TaskInput`
/// (from roko-compose's `context_provider`). This bridges the two crate
/// boundaries without creating a dependency.
fn task_def_to_input(td: &crate::task_parser::TaskDef) -> roko_compose::TaskInput {
    let (read_files, symbols, anti_patterns, prior_failures) = match &td.context {
        Some(ctx) => (
            ctx.read_files
                .iter()
                .map(|rf| roko_compose::ReadFileSpec {
                    path: rf.path.clone(),
                    lines: rf.lines.clone(),
                    why: rf.why.clone(),
                })
                .collect(),
            ctx.symbols.clone(),
            ctx.anti_patterns.clone(),
            ctx.prior_failures.clone(),
        ),
        None => (Vec::new(), Vec::new(), Vec::new(), Vec::new()),
    };

    roko_compose::TaskInput {
        id: td.id.clone(),
        title: td.title.clone(),
        description: td.description.clone(),
        tier: td.tier.clone(),
        files: td.files.clone(),
        read_files,
        symbols,
        anti_patterns,
        prior_failures,
        verify_commands: td
            .verify
            .iter()
            .map(|v| roko_compose::VerifySpec {
                phase: v.phase.clone(),
                command: v.command.clone(),
                fail_msg: v.fail_msg.clone(),
            })
            .collect(),
        acceptance: td.acceptance.clone(),
        depends_on: td.depends_on.clone(),
        max_loc: td.max_loc,
    }
}

/// Convert declared task context files into Claude CLI `--read` args.
fn task_read_cli_args(task_def: &crate::task_parser::TaskDef) -> Vec<String> {
    task_def
        .context
        .as_ref()
        .map(|ctx| {
            ctx.read_files
                .iter()
                .flat_map(|rf| ["--read".to_string(), rf.path.clone()])
                .collect()
        })
        .unwrap_or_default()
}

fn file_contains_public_api(path: &str, content: &str) -> bool {
    let normalized = path.replace('\\', "/");
    if normalized.ends_with("/src/lib.rs") || normalized.ends_with("/src/mod.rs") {
        return true;
    }

    content.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("pub fn ")
            || trimmed.starts_with("pub struct ")
            || trimmed.starts_with("pub enum ")
            || trimmed.starts_with("pub trait ")
            || trimmed.starts_with("pub type ")
            || trimmed.starts_with("pub use ")
            || trimmed.starts_with("pub mod ")
    })
}

fn truncate_doc_snippet(content: &str, max_chars: usize) -> String {
    let mut chars = content.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_none() {
        content.to_string()
    } else {
        format!("{truncated}\n\n[... truncated]")
    }
}

/// Load prior task outputs from `.roko/task-outputs/{task_id}.txt`.
///
/// When a task completes successfully, we persist a summary of its output
/// so that downstream tasks can reference it. If no outputs exist on disk,
/// returns an empty vec.
fn load_prior_task_outputs(
    workdir: &Path,
    depends_on: &[String],
) -> Vec<roko_compose::PriorTaskOutput> {
    let output_dir = workdir.join(".roko").join("task-outputs");
    let mut outputs = Vec::new();

    for dep_id in depends_on {
        let output_path = output_dir.join(format!("{dep_id}.txt"));
        if let Ok(summary) = std::fs::read_to_string(&output_path) {
            if !summary.trim().is_empty() {
                outputs.push(roko_compose::PriorTaskOutput {
                    task_id: dep_id.clone(),
                    summary,
                });
            }
        }
    }

    outputs
}

/// Maximum output size stored in task outputs and episode context (32 KB).
const MAX_OUTPUT_BYTES: usize = 32_768;
/// Number of output lines to include in task failure logs.
const TASK_FAILURE_OUTPUT_TAIL_LINES: usize = 20;

/// Truncate an agent output string, keeping the last N lines if it exceeds
/// `MAX_OUTPUT_BYTES` and prepending a truncation header.
fn truncate_output(output: &str) -> String {
    if output.len() <= MAX_OUTPUT_BYTES {
        return output.to_string();
    }
    // Keep the tail — the most recent output is usually most relevant.
    let tail = &output[output.len() - MAX_OUTPUT_BYTES..];
    // Find the first newline to avoid a partial first line.
    let start = tail.find('\n').map_or(0, |i| i + 1);
    format!(
        "[truncated: original {} bytes, showing last {} bytes]\n{}",
        output.len(),
        MAX_OUTPUT_BYTES,
        &tail[start..]
    )
}

/// Return the last `line_count` lines from `output`, preserving order.
fn tail_output_lines(output: &str, line_count: usize) -> String {
    if output.is_empty() || line_count == 0 {
        return String::new();
    }

    let mut lines: Vec<&str> = output.lines().rev().take(line_count).collect();
    lines.reverse();
    lines.join("\n")
}

/// Pull likely Rust symbol names out of task text for skill extraction.
fn extract_task_symbols(text: &str) -> Vec<String> {
    let mut symbols = Vec::new();
    let mut seen = HashSet::new();

    for raw in text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == ':')) {
        if raw.is_empty() {
            continue;
        }

        for candidate in raw.split("::") {
            let candidate =
                candidate.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_');
            if candidate.len() < 3 {
                continue;
            }
            let has_underscore = candidate.contains('_');
            let has_upper = candidate.chars().any(|ch| ch.is_ascii_uppercase());
            if !has_underscore && !has_upper {
                continue;
            }

            let candidate = candidate.to_string();
            if seen.insert(candidate.clone()) {
                symbols.push(candidate);
            }
        }
    }

    symbols
}

/// Add task failure context to an error chain.
fn with_task_failure_context(
    error: anyhow::Error,
    task_id: &str,
    phase: &str,
    gate: &str,
    output_tail: Option<&str>,
) -> anyhow::Error {
    let error = error
        .context(format!("task_id={task_id}"))
        .context(format!("phase={phase}"))
        .context(format!("gate={gate}"));

    match output_tail {
        Some(tail) if !tail.trim().is_empty() => error.context(format!(
            "agent_output_tail_last_{}_lines:\n{}",
            TASK_FAILURE_OUTPUT_TAIL_LINES, tail
        )),
        _ => error.context(format!(
            "agent_output_tail_last_{}_lines: <unavailable>",
            TASK_FAILURE_OUTPUT_TAIL_LINES
        )),
    }
}

/// Persist a task's output summary so downstream tasks can reference it.
fn save_task_output(workdir: &Path, task_id: &str, output: &str) {
    let output_dir = workdir.join(".roko").join("task-outputs");
    let _ = std::fs::create_dir_all(&output_dir);
    let output_path = output_dir.join(format!("{task_id}.txt"));
    let summary = truncate_output(output);
    let _ = std::fs::write(output_path, summary);
}

fn attestation_signing_key_from_env() -> Option<SigningKey> {
    let seed = std::env::var("ROKO_ATTEST_SIGNING_KEY_HEX").ok()?;
    let seed = seed.trim().trim_start_matches("0x");
    let hash = ContentHash::from_hex(seed)?;
    Some(SigningKey::from_bytes(&hash.0))
}

fn maybe_attest_engram(mut signal: Engram) -> Engram {
    if signal.attestation.is_none()
        && let Some(key) = attestation_signing_key_from_env()
    {
        signal.attestation = Some(attestation::sign(&signal, &key));
    }
    signal
}

fn conductor_signal_from_output(output: &Engram) -> Option<Engram> {
    let body = match &output.body {
        Body::Text(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            Body::text(trimmed)
        }
        Body::Json(value) => Body::Json(value.clone()),
        Body::Bytes(bytes) => {
            if bytes.is_empty() {
                return None;
            }
            Body::Bytes(bytes.clone())
        }
        Body::Empty => return None,
    };

    let mut builder = Engram::builder(output.kind.clone())
        .body(body)
        .provenance(output.provenance.clone())
        .lineage(
            output
                .lineage
                .iter()
                .copied()
                .chain(std::iter::once(output.id)),
        );
    for (key, value) in &output.tags {
        builder = builder.tag(key.clone(), value.clone());
    }
    if let Some(attestation) = output.attestation.clone() {
        builder = builder.attestation(attestation);
    }
    if let Some(emotional_tag) = output.emotional_tag.clone() {
        builder = builder.emotional_tag(emotional_tag);
    }
    Some(maybe_attest_engram(builder.build()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    }

    fn run_git(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("spawn git");
        assert!(
            output.status.success(),
            "git {:?} failed: stdout={} stderr={}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn init_git_repo() -> Option<TempDir> {
        if !git_available() {
            return None;
        }

        let tmp = TempDir::new().expect("tempdir");
        run_git(tmp.path(), &["init"]);
        run_git(tmp.path(), &["config", "user.name", "Test User"]);
        run_git(tmp.path(), &["config", "user.email", "test@example.com"]);
        std::fs::write(tmp.path().join("README.md"), "seed\n").expect("write seed");
        run_git(tmp.path(), &["add", "README.md"]);
        run_git(tmp.path(), &["commit", "-m", "seed"]);
        Some(tmp)
    }

    async fn runner_for_repo(workdir: &Path, use_worktrees: bool) -> PlanRunner {
        let snapshot_json = ExecutorSnapshot::new(0).to_json().expect("snapshot json");
        let mut config = Config::default();
        config.executor.use_worktrees = use_worktrees;
        PlanRunner::from_snapshot(
            &snapshot_json,
            workdir,
            config,
            Arc::new(MetricRegistry::new()),
            false,
        )
        .await
        .expect("plan runner")
    }

    #[test]
    fn scrub_signal_redacts_text_and_rehashes_identity() {
        let policy = ScrubPolicy::default();
        let signal = Engram::builder(Kind::Task)
            .body(Body::text("token=sk-proj-abcdefghijklmnopqrstuvwxyz123456"))
            .build();

        let scrubbed = scrub_signal(&signal, &policy);

        assert_eq!(
            scrubbed.body.as_text().expect("text body"),
            "token=[REDACTED]"
        );
        assert_ne!(scrubbed.id, signal.id);
    }

    #[test]
    fn scrub_agent_result_redacts_trace_and_output() {
        let policy = ScrubPolicy::default();
        let trace = Engram::builder(Kind::AgentMessage)
            .body(Body::text("sk-proj-abcdefghijklmnopqrstuvwxyz123456"))
            .build();
        let output = Engram::builder(Kind::AgentOutput)
            .body(Body::text("AKIAABCDEFGHIJKLMNOP"))
            .build();
        let result = AgentResult {
            output,
            trace: vec![trace],
            usage: roko_agent::Usage::zero(),
            success: true,
        };

        let scrubbed = scrub_agent_result(&result, &policy);

        assert_eq!(
            scrubbed.output.body.as_text().expect("output text"),
            "[REDACTED]"
        );
        assert_eq!(
            scrubbed.trace[0].body.as_text().expect("trace text"),
            "[REDACTED]"
        );
    }

    #[test]
    fn event_bus_wiring_records_turn_feedback() {
        let tmp = TempDir::new().expect("tempdir");
        let latency_path = tmp.path().join("latency-stats.json");
        let latency_registry = LatencyRegistry::load_or_new(&latency_path);
        let mut anomaly_detector = AnomalyDetector::new(1_700_000_000_000);
        let event_bus = LearningEventBus::new(16);

        for _ in 0..5 {
            publish_turn_learning_feedback(
                &event_bus,
                &latency_registry,
                &mut anomaly_detector,
                TurnLearningFeedback {
                    task_id: "task-2k23".to_string(),
                    model: "glm-5.1".to_string(),
                    provider: "zai".to_string(),
                    timestamp_ms: 1_700_000_000_000,
                    prompt_hash: 42,
                    ttft_ms: 120,
                    total_ms: 900,
                    output_tokens: 64,
                    cost_usd: 0.05,
                    usage: roko_agent::Usage {
                        input_tokens: 128,
                        output_tokens: 64,
                        cost_usd: 0.05,
                        wall_ms: 900,
                        ..Default::default()
                    },
                    success: true,
                },
            );
        }

        let stats = latency_registry
            .get("glm-5.1", "zai")
            .expect("latency stats should be recorded");
        assert_eq!(stats.observations, 5);
        assert_eq!(stats.recent_latencies, vec![900.0; 5]);

        assert!(matches!(
            anomaly_detector.check_prompt(42),
            Some(roko_learn::anomaly::Anomaly::PromptLoop { repeated_count })
                if repeated_count >= 5
        ));
    }

    #[test]
    fn conductor_wiring_persists_learned_switch_model_policy() {
        let tmp = TempDir::new().expect("tempdir");
        let path = conductor_policy_path(tmp.path());
        let state = RetryConductorState {
            iteration: 2,
            consecutive_failures: 2,
            error_pattern: RetryErrorPattern::Compile,
            elapsed_ms: 45_000,
            cost_so_far_usd: 0.08,
            model_tier: "fast".to_string(),
            task_complexity: "architectural".to_string(),
        };
        let mut bandit = ConductorBandit::new();

        for _ in 0..64 {
            bandit.record_outcome(&state, RetryConductorAction::Continue, false);
            bandit.record_outcome(&state, RetryConductorAction::SwitchModel, true);
        }

        bandit.save(&path).expect("save conductor policy");
        let reloaded = ConductorBandit::load_or_new(&path);

        let mut switch_model_count = 0;
        for _ in 0..32 {
            if reloaded.select_action(&state) == RetryConductorAction::SwitchModel {
                switch_model_count += 1;
            }
        }

        assert!(
            switch_model_count >= 24,
            "expected persisted policy to prefer switch_model, got {switch_model_count}/32"
        );
    }

    #[test]
    fn conductor_wiring_maps_compile_gate_failures_to_compile_pattern() {
        let error = with_task_failure_context(
            anyhow!("verify failed for T1: cargo check -p roko-cli"),
            "T1",
            "ready",
            "compile",
            None,
        );

        assert_eq!(
            PlanRunner::retry_failure_gate(&error).as_deref(),
            Some("compile")
        );
        assert_eq!(
            PlanRunner::retry_error_pattern(&error),
            RetryErrorPattern::Compile
        );
    }

    #[test]
    fn budget_enforcement_routes_to_mechanical_model() {
        let mut guardrail = BudgetGuardrail::new(1.0, 50.0, 10.0, 0.8);
        assert_eq!(
            guardrail.record_cost(0.81, "task"),
            BudgetAction::RouteToCheaper
        );

        let mut config = Config::default();
        config
            .agent
            .tier_models
            .insert("mechanical".to_string(), "claude-haiku-4-5".to_string());
        assert_eq!(
            mechanical_tier_model(&config),
            Some("claude-haiku-4-5".to_string())
        );
    }

    #[test]
    fn orchestration_report_all_succeeded() {
        let report = OrchestrationReport {
            plans: vec![
                PlanRunReport {
                    plan_id: "p1".into(),
                    succeeded: true,
                    agent_calls: 2,
                    gate_results: vec![("compile".into(), true)],
                },
                PlanRunReport {
                    plan_id: "p2".into(),
                    succeeded: true,
                    agent_calls: 1,
                    gate_results: vec![("test".into(), true)],
                },
            ],
            total_agent_calls: 3,
            total_gate_runs: 2,
            fleet_cfactor: None,
        };
        assert!(report.all_succeeded());
    }

    #[test]
    fn orchestration_report_partial_failure() {
        let report = OrchestrationReport {
            plans: vec![
                PlanRunReport {
                    plan_id: "p1".into(),
                    succeeded: true,
                    agent_calls: 1,
                    gate_results: vec![],
                },
                PlanRunReport {
                    plan_id: "p2".into(),
                    succeeded: false,
                    agent_calls: 1,
                    gate_results: vec![],
                },
            ],
            total_agent_calls: 2,
            total_gate_runs: 1,
            fleet_cfactor: None,
        };
        assert!(!report.all_succeeded());
    }

    #[test]
    fn role_prompt_coverage() {
        let roles = [
            AgentRole::Implementer,
            AgentRole::Auditor,
            AgentRole::Scribe,
            AgentRole::AutoFixer,
            AgentRole::Strategist,
            AgentRole::Researcher,
            AgentRole::Conductor,
        ];
        for role in roles {
            let prompt = roko_compose::role_identity_for(role);
            assert!(!prompt.is_empty(), "empty prompt for {role:?}");
        }
    }

    #[test]
    fn claude_skip_permissions_tracks_role_permissions() {
        assert!(claude_skip_permissions_for_role(AgentRole::Implementer));
        assert!(claude_skip_permissions_for_role(
            AgentRole::IntegrationTester
        ));
        assert!(!claude_skip_permissions_for_role(AgentRole::Auditor));
        assert!(!claude_skip_permissions_for_role(AgentRole::Strategist));
    }

    #[test]
    fn claude_task_tool_allowlist_filters_to_task_subset() {
        let csv = claude_task_tool_allowlist_with(
            AgentRole::Implementer,
            Some(&["read_file".to_string(), "write_file".to_string()]),
            None,
            None,
        );
        assert_eq!(csv, "Read,Write");
    }

    #[test]
    fn claude_task_tool_allowlist_drops_unlisted_tools() {
        let csv = claude_task_tool_allowlist_with(
            AgentRole::Implementer,
            Some(&["definitely_not_a_real_tool".to_string()]),
            None,
            None,
        );
        assert!(csv.is_empty());
    }

    #[test]
    fn claude_task_tool_allowlist_respects_denied_tools() {
        let csv = claude_task_tool_allowlist_with(
            AgentRole::Implementer,
            None,
            Some(&["write_file".to_string(), "edit_file".to_string()]),
            None,
        );
        assert!(csv.contains("Read"));
        assert!(!csv.contains(",Write,") && !csv.starts_with("Write,") && !csv.ends_with(",Write"));
    }

    #[test]
    fn task_dispatch_conventions_include_write_scope_and_loc() {
        let task: crate::task_parser::TaskDef = toml::from_str(
            r#"
id = "T1"
title = "Scope-limited refactor"
tier = "focused"
max_loc = 40
files = ["crates/roko-cli/src/orchestrate.rs", "crates/roko-cli/src/task_parser.rs"]
depends_on = []
"#,
        )
        .unwrap();

        let conventions =
            task_dispatch_conventions(Some(&task)).expect("dispatch conventions should exist");

        assert!(conventions.contains("Honor the declared write scope strictly."));
        assert!(conventions.contains("crates/roko-cli/src/orchestrate.rs"));
        assert!(conventions.contains("crates/roko-cli/src/task_parser.rs"));
        assert!(conventions.contains("40 lines of change"));
    }

    #[test]
    fn build_system_prompt_includes_declared_write_scope() {
        let task: crate::task_parser::TaskDef = toml::from_str(
            r#"
id = "T1"
title = "Scope-limited refactor"
tier = "focused"
files = ["crates/roko-cli/src/orchestrate.rs"]
depends_on = []
"#,
        )
        .unwrap();

        let prompt = build_system_prompt(
            AgentRole::Implementer,
            "plan-1",
            "T1",
            "Read,Edit",
            Some(&task),
        );

        assert!(prompt.contains("Honor the declared write scope strictly."));
        assert!(prompt.contains("crates/roko-cli/src/orchestrate.rs"));
    }

    #[tokio::test]
    async fn dispatch_refuses_tripped_circuit_breaker_before_launch() {
        let tmp = TempDir::new().unwrap();
        let mut plan_states = HashMap::new();
        plan_states.insert("plan-1".to_string(), PlanState::new("plan-1"));
        let snapshot = ExecutorSnapshot {
            plan_states,
            queue_order: vec!["plan-1".to_string()],
            timestamp_ms: 0,
            speculative_executions: HashMap::new(),
        };
        let snapshot_json = snapshot.to_json().unwrap();
        let mut runner = PlanRunner::from_snapshot(
            &snapshot_json,
            tmp.path(),
            Config::default(),
            Arc::new(MetricRegistry::new()),
            false,
        )
        .await
        .unwrap();

        runner
            .conductor
            .circuit_breaker()
            .record_failure("plan-1", "err1", 1);
        runner
            .conductor
            .circuit_breaker()
            .record_failure("plan-1", "err2", 2);

        let err = runner
            .dispatch_agent_with(
                "plan-1",
                AgentRole::Implementer,
                "task-1",
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("circuit breaker tripped"));
        assert!(
            runner
                .executor
                .plan_state("plan-1")
                .is_some_and(|state| state.paused)
        );
    }

    #[tokio::test]
    async fn build_review_prompt_skips_repo_root_when_worktree_unavailable() {
        let tmp = TempDir::new().unwrap();
        let mut plan_states = HashMap::new();
        plan_states.insert("plan-1".to_string(), PlanState::new("plan-1"));
        let snapshot = ExecutorSnapshot {
            plan_states,
            queue_order: vec!["plan-1".to_string()],
            timestamp_ms: 0,
            speculative_executions: HashMap::new(),
        };
        let snapshot_json = snapshot.to_json().unwrap();
        let runner = PlanRunner::from_snapshot(
            &snapshot_json,
            tmp.path(),
            {
                let mut config = Config::default();
                config.executor.use_worktrees = true;
                config
            },
            Arc::new(MetricRegistry::new()),
            false,
        )
        .await
        .unwrap();

        let prompt = runner.build_review_prompt("plan-1").await;
        assert!(!prompt.contains("crates/roko-cli/src/orchestrate.rs"));
    }

    #[test]
    fn normalize_resume_session_trims_and_drops_blank_values() {
        assert_eq!(normalize_resume_session(None), None);
        assert_eq!(normalize_resume_session(Some(String::new())), None);
        assert_eq!(normalize_resume_session(Some("   ".to_string())), None);
        assert_eq!(
            normalize_resume_session(Some("  sess-42  ".to_string())),
            Some("sess-42".to_string())
        );
    }

    #[test]
    fn default_worktree_manager_paths_under_roko_directory() {
        let workdir = PathBuf::from("/tmp/roko-test");
        let manager = default_worktree_manager(&workdir);
        assert_eq!(
            manager.path_for("plan-1"),
            workdir.join(".roko").join("worktrees").join("plan-1")
        );
    }

    #[tokio::test]
    async fn finalize_successful_task_worktree_merges_into_plan_branch() {
        let Some(tmp) = init_git_repo() else {
            return;
        };
        let runner = runner_for_repo(tmp.path(), true).await;
        let task_dir = runner
            .task_exec_dir("plan-1", "T1")
            .await
            .expect("task worktree");

        std::fs::write(task_dir.join("feature.txt"), "task change\n").expect("write change");

        runner
            .finalize_successful_task_worktree("plan-1", "T1", &task_dir)
            .await
            .expect("finalize task worktree");

        let plan_handle = runner.worktrees.get("plan-1").expect("plan worktree");
        assert_eq!(
            std::fs::read_to_string(plan_handle.path.join("feature.txt")).expect("plan file"),
            "task change\n"
        );

        runner
            .merge_branch("plan-1")
            .await
            .expect("merge canonical plan branch");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("feature.txt")).expect("repo file"),
            "task change\n"
        );
    }

    #[tokio::test]
    async fn merge_branch_falls_back_to_canonical_plan_branch_name() {
        let Some(tmp) = init_git_repo() else {
            return;
        };
        let main_branch = run_git(tmp.path(), &["branch", "--show-current"]);
        run_git(tmp.path(), &["checkout", "-b", "roko/plan/plan-1"]);
        std::fs::write(tmp.path().join("branch.txt"), "plan branch\n").expect("write branch");
        run_git(tmp.path(), &["add", "branch.txt"]);
        run_git(tmp.path(), &["commit", "-m", "plan branch"]);
        run_git(tmp.path(), &["checkout", &main_branch]);

        let runner = runner_for_repo(tmp.path(), false).await;
        runner
            .merge_branch("plan-1")
            .await
            .expect("merge canonical fallback branch");

        assert_eq!(
            std::fs::read_to_string(tmp.path().join("branch.txt")).expect("merged branch file"),
            "plan branch\n"
        );
    }

    #[tokio::test]
    async fn cleanup_plan_worktree_removes_tracked_checkout() {
        let Some(tmp) = init_git_repo() else {
            return;
        };
        let runner = runner_for_repo(tmp.path(), true).await;
        let plan_dir = runner.plan_exec_dir("plan-1").await;
        assert!(plan_dir.exists(), "plan worktree should exist");

        runner.cleanup_plan_worktree("plan-1").await;

        assert!(runner.worktrees.get("plan-1").is_none());
        assert!(!plan_dir.exists(), "plan worktree should be removed");
    }

    #[tokio::test]
    async fn ensure_plan_exec_dir_errors_when_worktree_creation_fails() {
        let tmp = TempDir::new().expect("tempdir");
        let runner = runner_for_repo(tmp.path(), true).await;

        let err = runner
            .ensure_plan_exec_dir("plan-1")
            .await
            .expect_err("non-git worktree acquisition should fail");

        assert!(err.to_string().contains("worktree unavailable"));
    }

    #[tokio::test]
    async fn complete_plan_dispatch_cleans_up_plan_worktree() {
        let Some(tmp) = init_git_repo() else {
            return;
        };
        let mut runner = runner_for_repo(tmp.path(), true).await;
        let plan_dir = runner.plan_exec_dir("plan-1").await;
        assert!(plan_dir.exists(), "plan worktree should exist");

        runner
            .dispatch_action(ExecutorAction::CompletePlan {
                plan_id: "plan-1".to_string(),
            })
            .await;

        assert!(runner.worktrees.get("plan-1").is_none());
        assert!(!plan_dir.exists(), "plan worktree should be removed");
    }

    #[tokio::test]
    async fn run_all_cleans_up_tracked_plan_worktree_on_cancel() {
        let Some(tmp) = init_git_repo() else {
            return;
        };
        let mut runner = runner_for_repo(tmp.path(), true).await;
        let plan_dir = runner.plan_exec_dir("plan-1").await;
        assert!(plan_dir.exists(), "plan worktree should exist");

        runner.cancel.cancel();
        let watcher_cancel = TokioCancellationToken::new();
        let _ = runner
            .run_all(&watcher_cancel)
            .await
            .expect("run_all should return after cancellation");

        assert!(runner.worktrees.get("plan-1").is_none());
        assert!(!plan_dir.exists(), "plan worktree should be removed");
    }

    #[test]
    fn post_merge_follow_up_reports_unresolved_regression() {
        let runner = PostMergeRunner::new();
        let (_check, follow_up) =
            runner.run_record_and_follow_up("plan-a", 100, &[Verdict::fail("test", "boom")]);
        assert!(follow_up.needs_revert());
        assert_eq!(runner.unresolved_regressions(), vec!["plan-a".to_string()]);
    }

    #[test]
    fn parse_review_verdict_structured() {
        assert!(parse_review_verdict("verdict = \"approve\""));
        assert!(!parse_review_verdict("verdict = \"revise\""));
        assert!(parse_review_verdict("verdict: approve"));
        assert!(!parse_review_verdict("verdict: reject"));
    }

    #[test]
    fn parse_review_verdict_keyword_fallback() {
        assert!(parse_review_verdict("The code looks good, LGTM!"));
        assert!(parse_review_verdict("Changes approved."));
        assert!(!parse_review_verdict("Please revise the implementation."));
        assert!(!parse_review_verdict("I reject this change due to bugs."));
        // Ambiguous → default approve
        assert!(parse_review_verdict("I have some minor comments."));
    }

    #[test]
    fn task_tracker_next_ready_and_completion() {
        let toml_str = r#"
[meta]
plan = "test"
total = 3

[[task]]
id = "T1"
title = "first"
depends_on = []

[[task]]
id = "T2"
title = "second"
depends_on = ["T1"]

[[task]]
id = "T3"
title = "independent"
depends_on = []
"#;
        let tf: TasksFile = toml::from_str(toml_str).unwrap();
        let mut tracker = TaskTracker::new(tf, PathBuf::from("/tmp"));

        assert!(!tracker.all_tasks_done());

        // T1 and T3 should be ready (no deps)
        let ready = tracker.next_ready_task(&[]).unwrap();
        assert!(ready.id == "T1" || ready.id == "T3");

        tracker.mark_completed("T1");
        tracker.mark_completed("T3");

        // Now T2 should be ready
        let ready = tracker.next_ready_task(&[]).unwrap();
        assert_eq!(ready.id, "T2");

        tracker.mark_completed("T2");
        assert!(tracker.all_tasks_done());
        assert!(tracker.next_ready_task(&[]).is_none());
    }

    #[test]
    fn task_tracker_blocks_on_completed_plan_deps() {
        let toml_str = r#"
[meta]
plan = "test"
total = 2

[[task]]
id = "T1"
title = "first"
depends_on = []

[[task]]
id = "T2"
title = "waits for external plan"
depends_on = []
depends_on_plan = ["other-plan"]
"#;
        let tf: TasksFile = toml::from_str(toml_str).unwrap();
        let tracker = TaskTracker::new(tf, PathBuf::from("/tmp"));

        let ready = tracker.ready_tasks(&[]);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "T1");
        assert!(tracker.has_tasks_blocked_by_plans(&[]));

        let completed_plans = vec!["other-plan".to_string()];
        let ready_with_dep = tracker.ready_tasks(&completed_plans);
        assert_eq!(ready_with_dep.len(), 2);
        assert!(!tracker.has_tasks_blocked_by_plans(&completed_plans));
    }

    #[test]
    fn task_tracker_reset_for_reimpl() {
        let toml_str = r#"
[meta]
plan = "test"
total = 1

[[task]]
id = "T1"
title = "first"
depends_on = []
"#;
        let tf: TasksFile = toml::from_str(toml_str).unwrap();
        let mut tracker = TaskTracker::new(tf, PathBuf::from("/tmp"));

        tracker.mark_completed("T1");
        assert!(tracker.all_tasks_done());
        assert_eq!(tracker.impl_round, 0);

        tracker.reset_for_reimpl();
        assert!(!tracker.all_tasks_done());
        assert_eq!(tracker.impl_round, 1);
        assert!(tracker.completed.is_empty());
    }

    #[test]
    fn task_tracker_persists_escalated_model_hint() {
        let dir = TempDir::new().unwrap();
        let tasks_path = dir.path().join("tasks.toml");
        std::fs::write(
            &tasks_path,
            r#"
[meta]
plan = "test"
total = 1

[[task]]
id = "T1"
title = "first"
tier = "focused"
model_hint = "claude-sonnet-4-5"
depends_on = []
"#,
        )
        .unwrap();

        let tf = TasksFile::parse(&tasks_path).unwrap();
        let mut tracker = TaskTracker::new(tf, dir.path().to_path_buf());

        tracker
            .set_task_model_hint("T1", Some("claude-opus-4".to_string()))
            .unwrap();

        let rendered = std::fs::read_to_string(&tasks_path).unwrap();
        assert!(rendered.contains(r#"model_hint = "claude-opus-4""#));
        assert_eq!(
            tracker
                .tasks_file
                .tasks
                .iter()
                .find(|task| task.id == "T1")
                .and_then(|task| task.model_hint.as_deref()),
            Some("claude-opus-4")
        );
    }

    #[test]
    fn review_drift_report_flags_unanchored_output() {
        let tasks: TasksFile = toml::from_str(
            r#"
[meta]
plan = "demo"
total = 1

[[task]]
id = "T1"
title = "Wire reviewing drift guard"
tier = "focused"
files = ["src/orchestrate.rs"]
depends_on = []

[task.context]
anti_patterns = ["Do not skip the drift check"]

[[task.verify]]
phase = "compile"
command = "cargo check -p roko-cli"
"#,
        )
        .unwrap();

        let report = review_drift_report(&tasks, "Looks good, approve.");
        assert!(report.is_some());
        let report = report.unwrap();
        assert!(report.drifted());
        assert!(report.coverage() < 0.35);
    }

    #[test]
    fn review_drift_report_accepts_anchored_output() {
        let tasks: TasksFile = toml::from_str(
            r#"
[meta]
plan = "demo"
total = 1

[[task]]
id = "T1"
title = "Wire reviewing drift guard"
tier = "focused"
files = ["src/orchestrate.rs"]
depends_on = []

[task.context]
anti_patterns = ["Do not skip the drift check"]

[[task.verify]]
phase = "compile"
command = "cargo check -p roko-cli"
"#,
        )
        .unwrap();

        let report = review_drift_report(
            &tasks,
            "T1 review: src/orchestrate.rs implements the drift guard and cargo check stays green.",
        );
        assert!(report.is_some());
        let report = report.unwrap();
        assert!(!report.drifted());
        assert!(report.coverage() >= 0.35);
    }

    #[test]
    fn file_contains_public_api_detects_exports() {
        assert!(file_contains_public_api(
            "crates/demo/src/lib.rs",
            "pub fn exported() {}\n"
        ));
        assert!(file_contains_public_api(
            "crates/demo/src/foo.rs",
            "pub struct Thing;\n"
        ));
        assert!(!file_contains_public_api(
            "crates/demo/src/foo.rs",
            "fn helper() {}\n"
        ));
    }

    #[test]
    fn truncate_doc_snippet_limits_length() {
        let content = "a".repeat(20);
        let truncated = truncate_doc_snippet(&content, 8);
        assert!(truncated.starts_with("aaaaaaaa"));
        assert!(truncated.contains("[... truncated]"));
    }

    #[test]
    fn search_query_for_task_includes_tier_context_and_crate_hints() {
        let task: crate::task_parser::TaskDef = toml::from_str(
            r#"
id = "T1"
title = "Integrate provider routing"
description = "Wire adaptive provider selection into orchestration."
status = "pending"
tier = "integrative"
files = ["crates/roko-cli/src/orchestrate.rs", "crates/roko-agent/src/provider/mod.rs"]
depends_on = []
depends_on_plan = []
verify = []
"#,
        )
        .unwrap();

        let query = search_query_for_task(&task);
        assert!(query.contains("Rust"));
        assert!(query.contains("integrative"));
        assert!(query.contains("adaptive provider selection"));
        assert!(query.contains("roko-cli"));
        assert!(query.contains("orchestrate.rs"));
    }

    #[test]
    fn render_search_context_includes_sources_and_truncates_large_snippets() {
        let results = vec![roko_agent::perplexity::SearchResult {
            url: "https://example.com/routing".to_string(),
            title: "Routing patterns".to_string(),
            content: "a".repeat(600),
            date: Some("2026-04-10".to_string()),
            last_updated: None,
        }];

        let rendered = render_search_context(
            "Rust integrative provider routing best practices",
            &results,
            &["crates/roko-cli/src/orchestrate.rs".to_string()],
        );

        assert!(rendered.contains("## External Research"));
        assert!(rendered.contains("Query: Rust integrative provider routing best practices"));
        assert!(rendered.contains("Routing patterns"));
        assert!(rendered.contains("Source: https://example.com/routing"));
        assert!(rendered.contains("Date: 2026-04-10"));
        assert!(rendered.contains("[... truncated]"));
    }

    #[test]
    fn success_knowledge_entry_prefers_heuristic_for_refactors() {
        let task: crate::task_parser::TaskDef = toml::from_str(
            r#"
id = "T1"
title = "Refactor gate pipeline"
description = "Standardize the reusable verification flow across modules."
status = "pending"
tier = "focused"
files = ["crates/roko-gate/src/a.rs", "crates/roko-gate/src/b.rs"]
depends_on = []
depends_on_plan = []
verify = []
"#,
        )
        .unwrap();
        let result = AgentResult::ok(
            Engram::builder(Kind::AgentOutput)
                .body(Body::text(
                    "Shared the verification helper and removed duplication.",
                ))
                .build(),
        );

        let entry =
            build_success_knowledge_entry("plan-1", "T1", Some(&task), &result, "gpt-5", "ep-1");

        assert_eq!(entry.kind, KnowledgeKind::Heuristic);
        assert_eq!(entry.tier, KnowledgeTier::Transient);
        assert_eq!(entry.source_episodes, vec!["ep-1".to_string()]);
        assert!(entry.tags.contains(&"task-success".to_string()));
        assert!(entry.tags.contains(&"multi-file".to_string()));
    }

    #[test]
    fn success_knowledge_entry_defaults_to_insight_for_single_fix() {
        let task: crate::task_parser::TaskDef = toml::from_str(
            r#"
id = "T2"
title = "Fix import mismatch"
description = "Align a single module import with the renamed type."
status = "pending"
tier = "mechanical"
files = ["crates/roko-core/src/lib.rs"]
depends_on = []
depends_on_plan = []
verify = []
"#,
        )
        .unwrap();
        let result = AgentResult::ok(
            Engram::builder(Kind::AgentOutput)
                .body(Body::text("Updated the import and cargo check passed."))
                .build(),
        );

        let entry =
            build_success_knowledge_entry("plan-2", "T2", Some(&task), &result, "gpt-5", "ep-2");

        assert_eq!(entry.kind, KnowledgeKind::Insight);
        assert_eq!(entry.source_model.as_deref(), Some("gpt-5"));
        assert!(entry.content.contains("Successful outcome"));
        assert!(
            entry
                .tags
                .contains(&"file:crates/roko-core/src/lib.rs".to_string())
        );
    }

    #[test]
    fn conductor_signal_preserves_lineage_and_provenance() {
        let output = Engram::builder(Kind::AgentOutput)
            .body(Body::text("  patched the failing import  "))
            .provenance(Provenance::trusted("claude"))
            .tag("task_id", "T1")
            .build();

        let signal = conductor_signal_from_output(&output).expect("signal");

        assert_eq!(
            signal.body.as_text().expect("text body"),
            "patched the failing import"
        );
        assert_eq!(signal.lineage, vec![output.id]);
        assert_eq!(signal.provenance, output.provenance);
        assert_eq!(signal.tag("task_id"), Some("T1"));
    }

    #[test]
    fn conductor_signal_skips_empty_text_output() {
        let output = Engram::builder(Kind::AgentOutput)
            .body(Body::text("   \n\t  "))
            .build();

        assert!(conductor_signal_from_output(&output).is_none());
    }

    #[test]
    fn task_read_cli_args_emits_claude_read_flags() {
        let task: crate::task_parser::TaskDef = toml::from_str(
            r#"
id = "T1"
title = "Read context"
depends_on = []

[context]
read_files = [
    { path = "src/lib.rs" },
    { path = "src/mod.rs" },
]
"#,
        )
        .unwrap();

        assert_eq!(
            task_read_cli_args(&task),
            vec![
                "--read".to_string(),
                "src/lib.rs".to_string(),
                "--read".to_string(),
                "src/mod.rs".to_string(),
            ]
        );
    }

    #[test]
    fn crate_name_for_path_derives_member_and_workspace_crates() {
        assert_eq!(
            crate_name_for_path("crates/roko-cli/src/orchestrate.rs"),
            Some("roko-cli".to_string())
        );
        assert_eq!(
            crate_name_for_path("apps/demo/src/main.rs"),
            Some("demo".to_string())
        );
        assert_eq!(
            crate_name_for_path("src/lib.rs"),
            Some("workspace".to_string())
        );
        assert_eq!(crate_name_for_path("README.md"), None);
    }

    #[test]
    fn crate_familiarity_tracker_persists_ratio() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("crate-familiarity.jsonl");
        let task: crate::task_parser::TaskDef = toml::from_str(
            r#"
id = "T1"
title = "Touch orchestrator"
files = ["crates/roko-cli/src/orchestrate.rs"]
"#,
        )
        .unwrap();

        let mut tracker = CrateFamiliarityTracker::load(&path);
        assert!((tracker.score_for_task(Some(&task)) - 0.5).abs() < f64::EPSILON);

        tracker
            .record_task_outcome(Some(&task), true, "plan-a", "task-a")
            .unwrap();
        tracker
            .record_task_outcome(Some(&task), false, "plan-b", "task-b")
            .unwrap();

        assert!((tracker.score_for_task(Some(&task)) - 0.5).abs() < f64::EPSILON);

        let reloaded = CrateFamiliarityTracker::load(&path);
        assert!((reloaded.score_for_task(Some(&task)) - 0.5).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn skill_injection_selects_high_confidence_matching_skills() {
        let tmp = TempDir::new().unwrap();
        let library = SkillLibrary::new(tmp.path().join("skills.json"))
            .await
            .unwrap();

        let mut matching = Skill::new(
            "matching_skill",
            "Wire prompt assembly through the skill library",
            "Inject the skill section before the task body.",
        );
        matching.task_category = "implementation".to_string();
        matching.files = vec!["crates/roko-cli/src/orchestrate.rs".to_string()];
        matching.tags = vec!["PromptComposer".to_string(), "roko-cli".to_string()];
        matching.score = 0.91;
        matching.success_rate = 0.75;
        matching.usage_count = 4;
        matching.validated_count = 2;
        library.register(&matching).await.unwrap();

        let mut low_confidence = Skill::new(
            "low_confidence_skill",
            "Weakly related prompt tweak",
            "This should not be injected.",
        );
        low_confidence.task_category = "implementation".to_string();
        low_confidence.files = vec!["crates/roko-cli/src/orchestrate.rs".to_string()];
        low_confidence.score = 0.2;
        library.register(&low_confidence).await.unwrap();

        let mut failed = Skill::new(
            "failed_skill",
            "Failure pattern that should stay out of the prompt",
            "This should not be injected.",
        );
        failed.task_category = "implementation".to_string();
        failed.files = vec!["crates/roko-cli/src/orchestrate.rs".to_string()];
        failed.tags = vec!["outcome:failure".to_string()];
        failed.score = 0.95;
        library.register(&failed).await.unwrap();

        let task: crate::task_parser::TaskDef = toml::from_str(
            r#"
id = "T1"
title = "Wire Skill Library into PromptComposer"
files = ["crates/roko-cli/src/orchestrate.rs"]
"#,
        )
        .unwrap();

        let selected = select_prompt_skills(
            &library,
            Some(&task),
            "Wire Skill Library into PromptComposer and inject relevant skills",
            5,
        );

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "matching_skill");

        let rendered = render_prior_experience(&selected);
        assert!(rendered.contains("matching_skill"));
        assert!(rendered.contains("confidence: 91%"));
        assert!(rendered.contains("Telemetry: 75% success over 4 uses"));
    }

    #[test]
    fn neuro_strategy_fragment_injection_prefers_persistent_matches() {
        let tmp = TempDir::new().unwrap();
        let store = KnowledgeStore::new(tmp.path().join("knowledge.jsonl"));

        store
            .add(KnowledgeEntry {
                id: "playbook-match".to_string(),
                kind: KnowledgeKind::StrategyFragment,
                source: Some("roko-neuro".to_string()),
                content: "# STRATEGY FRAGMENT\n\n## Action Rules\n\nPrefer injecting reusable strategy fragments before the task body when wiring prompt assembly in crates/roko-cli/src/orchestrate.rs.\n".to_string(),
                confidence: 0.94,
                confidence_weight: 0.94,
                refuted_insight_id: None,
                refutation_evidence: None,
                source_episodes: vec!["ep-1".to_string(), "ep-2".to_string()],
                tags: vec![
                    "tier:strategy_fragment".to_string(),
                    "implementation".to_string(),
                    "roko-cli".to_string(),
                ],
                source_model: None,
                model_generality: 1.0,
                created_at: chrono::Utc::now(),
                half_life_days: 30.0,
                tier: KnowledgeTier::Persistent,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,
            })
            .unwrap();

        store
            .add(KnowledgeEntry {
                id: "heuristic-match".to_string(),
                kind: KnowledgeKind::Heuristic,
                source: Some("roko-neuro".to_string()),
                content: "This lower-tier heuristic should not be injected when requesting strategy-fragment guidance.".to_string(),
                confidence: 0.99,
                confidence_weight: 0.99,
                refuted_insight_id: None,
                refutation_evidence: None,
                source_episodes: vec!["ep-3".to_string()],
                tags: vec![
                    "tier:heuristic".to_string(),
                    "implementation".to_string(),
                    "roko-cli".to_string(),
                ],
                source_model: None,
                model_generality: 1.0,
                created_at: chrono::Utc::now(),
                half_life_days: 90.0,
                tier: KnowledgeTier::Consolidated,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,
            })
            .unwrap();

        let task: crate::task_parser::TaskDef = toml::from_str(
            r#"
id = "T1"
title = "Wire prompt assembly to inject neuro strategy fragments"
files = ["crates/roko-cli/src/orchestrate.rs"]
"#,
        )
        .unwrap();

        let context = build_strategy_fragment_context(
            &store,
            AgentRole::Implementer,
            Some(&task),
            "Wire prompt assembly to inject neuro strategy fragments in roko-cli orchestrate",
            "claude-sonnet-4-5",
        )
        .expect("strategy fragment context");

        assert!(context.contains("## Strategy Fragments"));
        assert!(
            context.contains("Prefer injecting reusable strategy fragments before the task body")
        );
        assert!(!context.contains("lower-tier heuristic"));
    }

    #[test]
    fn model_specific_strategy_fragments_require_matching_model() {
        let tmp = TempDir::new().unwrap();
        let store = KnowledgeStore::new(tmp.path().join("knowledge.jsonl"));

        store
            .add(KnowledgeEntry {
                id: "playbook-model-specific".to_string(),
                kind: KnowledgeKind::StrategyFragment,
                source: Some("roko-neuro".to_string()),
                content: "# STRATEGY FRAGMENT\n\nUse XML tool-call tags for this model.\n"
                    .to_string(),
                confidence: 0.92,
                confidence_weight: 0.92,
                refuted_insight_id: None,
                refutation_evidence: None,
                source_episodes: vec!["ep-1".to_string()],
                tags: vec![
                    "tier:strategy_fragment".to_string(),
                    "implementation".to_string(),
                ],
                source_model: Some("claude-sonnet-4-5".to_string()),
                model_generality: 0.1,
                created_at: chrono::Utc::now(),
                half_life_days: 30.0,
                tier: KnowledgeTier::Persistent,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,
            })
            .unwrap();

        let task: crate::task_parser::TaskDef = toml::from_str(
            r#"
id = "T1"
title = "Inject only matching model-specific strategy fragments"
files = ["crates/roko-cli/src/orchestrate.rs"]
"#,
        )
        .unwrap();

        let mismatch = build_strategy_fragment_context(
            &store,
            AgentRole::Implementer,
            Some(&task),
            "Inject only matching model-specific strategy fragments",
            "gpt-5.4",
        );
        assert!(mismatch.is_none());

        let matched = build_strategy_fragment_context(
            &store,
            AgentRole::Implementer,
            Some(&task),
            "Inject only matching model-specific strategy fragments",
            "claude-sonnet-4-5",
        )
        .expect("matched strategy fragment context");
        assert!(matched.contains("XML tool-call tags"));
    }

    #[test]
    fn cost_anomaly_downgrade() {
        let mut config = Config::default();
        config
            .agent
            .tier_models
            .insert("mechanical".to_string(), "claude-haiku-4-5".to_string());

        let mut detector = AnomalyDetector::new(1_700_000_000_000);
        for cost in [1.0, 1.2, 0.9, 1.1, 1.05, 0.95, 1.15, 1.0] {
            assert!(detect_cost_anomaly_override(&mut detector, cost, &config).is_none());
        }

        assert_eq!(
            detect_cost_anomaly_override(&mut detector, 10.0, &config),
            Some("claude-haiku-4-5".to_string())
        );
    }

    #[test]
    fn atomic_checkpoint_preserves_existing_snapshot_on_tmp_write_failure() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("executor.json");

        let original = ExecutorSnapshot::new(1);
        save_snapshot_atomic(&original, &path).unwrap();
        let original_json = std::fs::read_to_string(&path).unwrap();

        let tmp_path = path.with_extension("json.tmp");
        std::fs::create_dir(&tmp_path).unwrap();

        let replacement = ExecutorSnapshot::new(2);
        let err = save_snapshot_atomic(&replacement, &path).unwrap_err();
        assert!(err.to_string().contains("write snapshot tmp"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original_json);
    }

    #[test]
    fn prioritize_ready_tasks_prefers_high_arousal() {
        let ready = vec!["calm".to_string(), "urgent".to_string()];
        let result = prioritize_ready_tasks(ready, |task_id| match task_id {
            "calm" => -1.0,
            "urgent" => 1.0,
            _ => 0.0,
        });

        assert_eq!(result, vec!["urgent", "calm"]);
    }

    #[test]
    fn prioritize_ready_tasks_keeps_original_order_for_ties() {
        let ready = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let result = prioritize_ready_tasks(ready, |_| 0.0);

        assert_eq!(result, vec!["a", "b", "c"]);
    }

    #[test]
    fn regenerate_plan_preserves_completed_tasks_and_rewrites_dependencies() {
        let old_tasks: TasksFile = toml::from_str(
            r#"
[meta]
plan = "demo"
iteration = 2
total = 2
done = 1
status = "ready"
max_parallel = 2

[[task]]
id = "T1"
title = "Implement plan regeneration"
status = "done"
tier = "focused"
depends_on = []
files = ["src/a.rs"]

[[task.verify]]
phase = "compile"
command = "cargo check -p roko-cli"

[[task]]
id = "T2"
title = "Wire dashboard"
status = "ready"
tier = "focused"
depends_on = ["T1"]
files = ["src/b.rs"]

[[task.verify]]
phase = "compile"
command = "cargo check -p roko-cli"
"#,
        )
        .unwrap();
        let regenerated_tasks: TasksFile = toml::from_str(
            r#"
[meta]
plan = "demo"
iteration = 1
total = 3
done = 0
status = "ready"
max_parallel = 2

[[task]]
id = "N1"
title = "Implement plan regen"
status = "ready"
tier = "focused"
depends_on = []
files = ["src/a.rs"]

[[task.verify]]
phase = "compile"
command = "cargo check -p roko-cli"

[[task]]
id = "N2"
title = "Wire dashboard"
status = "ready"
tier = "focused"
depends_on = ["N1"]
files = ["src/b.rs"]

[[task.verify]]
phase = "compile"
command = "cargo check -p roko-cli"

[[task]]
id = "N3"
title = "Add metrics"
status = "ready"
tier = "focused"
depends_on = ["N2"]
files = ["src/c.rs"]

[[task.verify]]
phase = "compile"
command = "cargo check -p roko-cli"
"#,
        )
        .unwrap();
        let completed = vec![old_tasks.tasks[0].clone()];

        let merged = merge_regenerated_plan("demo", &old_tasks, regenerated_tasks, &completed);

        assert_eq!(merged.meta.plan, "demo");
        assert_eq!(merged.meta.iteration, 3);
        assert_eq!(merged.meta.total, 3);
        assert_eq!(merged.meta.done, 1);
        assert_eq!(merged.meta.status, "ready");
        assert_eq!(merged.tasks.len(), 3);
        assert_eq!(merged.tasks[0].id, "T1");
        assert_eq!(merged.tasks[0].status, "done");
        assert_eq!(merged.tasks[1].id, "N2");
        assert_eq!(merged.tasks[1].depends_on, vec!["T1"]);
        assert_eq!(merged.tasks[2].id, "N3");
        assert_eq!(merged.tasks[2].depends_on, vec!["N2"]);
        assert!(merged.tasks.iter().all(|task| task.id != "N1"));
    }
}
