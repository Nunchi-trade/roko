//! Plan-driven orchestration loop: reads plans → builds executor → dispatches
//! agents → runs gates → persists results → advances phases.
//!
//! This is the runtime harness that connects the CLI to the orchestrator's
//! pure state machine. The orchestrator's [`ParallelExecutor`] never does I/O
//! — it returns [`ExecutorAction`]s. This module dispatches those actions to
//! real agents, gates, and git, then feeds results back as events.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use bardo_runtime::cancel::CancelToken;
use bardo_runtime::process::ProcessSupervisor;
use roko_agent::translate::{ClaudeTranslator, RenderedTools, Translator};
use roko_agent::{Agent, AgentResult, ClaudeCliAgent, ExecAgent};
use roko_compose::{
    ContextProvider, Placement, PlanArtifacts, PromptComposer, PromptSection, RoleSystemPromptSpec,
    SectionPriority, TaskContext,
};
use roko_conductor::{Conductor, ConductorDecision};
use roko_conductor::diagnosis::DiagnosisEngine;
use roko_core::metric::{ConfigHash, TaskMetric};
use roko_core::obs::health::{AlwaysUpProbe, ProbeRegistry};
use roko_core::obs::{LabelSet, MetricRegistry};
use roko_core::tool::TraceId;
use roko_core::tool::trace::{FailureKind, FailureTrace, TraceStep};
use roko_core::tool::{FormatBandit, ProfileBandit, ToolTraceEvent, TraceSink};
use roko_core::{
    AgentRole, Body, Budget, Composer, Context, Gate, Kind, PhaseKind, Provenance, Signal,
    Substrate, Verdict,
};
use roko_fs::FileSubstrate;
use roko_fs::observability::FsObservabilitySinks;
use roko_fs::RokoLayout;
use roko_gate::{
    adaptive_threshold::AdaptiveThresholds, clippy_gate::ClippyGate, compile::CompileGate,
    payload::GatePayload, test_gate::TestGate,
};
use roko_learn::costs_db::CostRecord;
use roko_learn::efficiency::AgentEfficiencyEvent;
use roko_learn::episode_logger::{Episode, GateVerdict, Usage};
use roko_learn::runtime_feedback::{CompletedRunInput, LearningRuntime, LearningUpdate};
use roko_orchestrator::worktree::{WorktreeConfig, WorktreeManager};
use roko_orchestrator::{
    EventKind, EventLog, EventLogSnapshot, ExecutorAction, ExecutorEvent, ExecutorSnapshot,
    GateResult, ParallelExecutor, PlanState, PostMergeRunner, discover_plans,
};
use roko_std::NoOpScorer;
use roko_std::StaticToolRegistry;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken as TokioCancellationToken;

use crate::config::Config;
use crate::task_parser::TasksFile;

/// Default number of actions between auto-saves.
const AUTOSAVE_INTERVAL: usize = 5;
const DEFAULT_WORKTREE_IDLE_TTL_SECS: u64 = 30 * 60;
const WATCHER_INTERVAL_SECS: u64 = 30;
const WATCHER_SIGNAL_TAIL: usize = 200;
const EFFICIENCY_SIGNAL_TAIL: usize = 256;
const GHOST_TURN_SIGNAL_KIND: &str = "conductor.ghost_turn";

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

    fn ref_rate(&self, tier: &str, source_type: &str) -> f64 {
        match self.rates.get(&(tier.to_string(), source_type.to_string())) {
            Some(&(referenced, total)) if total > 0 => referenced as f64 / total as f64,
            _ => 1.0,
        }
    }

    fn should_demote(&self, tier: &str, source_type: &str) -> bool {
        self.ref_rate(tier, source_type) < 0.10
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
    tools_csv: String,
    mcp_config: Option<PathBuf>,
    fallback_model: Option<String>,
    env_vars: Vec<(String, String)>,
    read_args: Vec<String>,
    extra_args: Vec<String>,
    resume_session: Option<String>,
    prompt: String,
    skip_permissions: bool,
}

/// Run a prepared agent configuration. No `PlanRunner` borrow required.
async fn run_prepared_agent(cfg: AgentRunConfig) -> AgentResult {
    let ctx = Context::now();
    let prompt_signal = Signal::builder(Kind::Task)
        .body(Body::Text(cfg.prompt.clone()))
        .build();

    if cfg.command == "claude" {
        let mut agent = ClaudeCliAgent::new(&cfg.command, &cfg.exec_dir, &cfg.model)
            .with_timeout_ms(cfg.timeout_ms)
            .with_bare_mode(cfg.bare_mode)
            .with_effort(cfg.effort)
            .with_system_prompt(cfg.system_prompt)
            .with_extra_args(cfg.read_args)
            .with_tools(cfg.tools_csv)
            .with_settings_json(roko_agent::claude_cli_agent::build_settings_json())
            .with_dangerously_skip_permissions(cfg.skip_permissions)
            .with_optional_resume(cfg.resume_session)
            .with_extra_args(cfg.extra_args);
        if let Some(mcp_path) = &cfg.mcp_config {
            agent = agent.with_mcp_config(mcp_path);
        }
        if let Some(fallback) = &cfg.fallback_model {
            agent = agent.with_fallback_model(fallback.clone());
        }
        for (k, v) in &cfg.env_vars {
            agent = agent.with_env_var(k, v);
        }
        agent.run(&prompt_signal, &ctx).await
    } else {
        let mut agent =
            ExecAgent::new(&cfg.command, cfg.extra_args).with_timeout_ms(cfg.timeout_ms);
        for (k, v) in &cfg.env_vars {
            agent = agent.with_env_var(k, v);
        }
        agent.run(&prompt_signal, &ctx).await
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
                            if !findings.is_empty() {
                                eprintln!(
                                    "[conductor] watcher runner observed {} intervention signal(s) from last {} signal(s)",
                                    findings.len(),
                                    signals.len()
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!(
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

async fn load_recent_signals(path: &Path, tail_len: usize) -> std::io::Result<Vec<Signal>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let text = tokio::fs::read_to_string(path).await?;
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(tail_len);
    let mut signals = Vec::with_capacity(lines.len().saturating_sub(start));
    for line in &lines[start..] {
        if let Ok(signal) = serde_json::from_str::<Signal>(line) {
            signals.push(signal);
        }
    }
    Ok(signals)
}

/// Load the latest efficiency entries and convert them into cost metric signals.
async fn load_efficiency_cost_signals(
    path: &Path,
    budget_usd: Option<f64>,
) -> std::io::Result<Vec<Signal>> {
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
) -> std::io::Result<Vec<Signal>> {
    let text = std::fs::read_to_string(path)?;
    Ok(build_efficiency_signals(&text, budget_usd))
}

/// Convert the latest efficiency entries into the signals expected by the conductor.
fn build_efficiency_signals(text: &str, budget_usd: Option<f64>) -> Vec<Signal> {
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

fn build_cost_overrun_signals(text: &str, budget_usd: f64) -> Vec<Signal> {
    let Some(cost_usd) = latest_efficiency_cost(text) else {
        return Vec::new();
    };

    vec![
        Signal::builder(Kind::Metric)
            .body(Body::text("plan cost"))
            .tag("name", "plan_cost")
            .tag("value", format!("{cost_usd:.6}"))
            .build(),
        Signal::builder(Kind::Metric)
            .body(Body::text("plan budget"))
            .tag("name", "plan_budget")
            .tag("value", format!("{budget_usd:.6}"))
            .build(),
    ]
}

fn build_context_window_pressure_signal(text: &str) -> Option<Signal> {
    let event = latest_efficiency_event(text)?;
    let body = Body::from_json(&event).unwrap_or_else(|_| {
        Body::text(format!(
            "{} tokens used on {}",
            event.total_prompt_tokens, event.model
        ))
    });

    Some(
        Signal::builder(Kind::TokenUsage)
            .body(body)
            .tag("plan_id", event.plan_id)
            .tag("task_id", event.task_id)
            .tag("role", event.role)
            .tag("model", event.model)
            .tag("tokens_used", event.total_prompt_tokens.to_string())
            .build(),
    )
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
    /// Process supervisor for tracking and cleaning up agent subprocesses.
    supervisor: ProcessSupervisor,
    /// Root cancellation token for coordinated shutdown.
    cancel: CancelToken,
    /// Per-plan task tracking for granular Implementing → Gating progression.
    task_trackers: HashMap<String, TaskTracker>,
    /// Conductor for anomaly detection between phases.
    conductor: Arc<Conductor>,
    /// Signals accumulated during the current plan run for conductor evaluation.
    conductor_signals: Vec<Signal>,
    /// Context attribution tracker for per-(tier, source_type) demotion decisions.
    attribution_tracker: ContextAttributionTracker,
    /// Cumulative USD cost per plan_id.
    plan_costs: HashMap<String, f64>,
    /// Cumulative USD cost per plan/task dispatch key.
    task_costs: HashMap<String, f64>,
    /// Metric registry for counters/histograms/gauges (prometheus-style).
    metrics: Arc<MetricRegistry>,
    /// Format-selection bandit for adaptive tool-call format per model/role.
    format_bandit: ProfileBandit,
    /// MCP server clients spawned at plan-run startup.
    /// MCP server clients (kept alive for lifecycle management).
    #[allow(dead_code)]
    mcp_clients: Vec<roko_agent::mcp::McpClient<roko_agent::mcp::StdioTransport>>,
    /// Dynamic tool registry combining static tools with MCP-discovered tools.
    tool_registry: Option<Arc<roko_agent::mcp::DynamicToolRegistry>>,
    /// Filesystem-backed observability sinks (traces + metrics).
    obs_sinks: FsObservabilitySinks,
    /// Health probe registry for readiness/liveness checks.
    health_probes: ProbeRegistry,
    /// Adaptive gate thresholds for retry budgeting.
    adaptive_thresholds: AdaptiveThresholds,
    /// In-memory efficiency events collected during this run.
    efficiency_events: Vec<AgentEfficiencyEvent>,
    /// Optional event bus sender for HTTP API event streaming.
    server_event_bus:
        Option<bardo_runtime::event_bus::BusSender<crate::serve::events::ServerEvent>>,
}

/// Tracks per-task completion within a plan. Lives in PlanRunner (CLI crate),
/// NOT in PlanState (orchestrator crate) — the state machine stays pure.
struct TaskTracker {
    tasks_file: TasksFile,
    completed: Vec<String>,
    failed: Vec<String>,
    current_group_index: usize,
    _plan_dir: PathBuf,
    last_gate_failure: Option<String>,
    /// Which gate phase failed (e.g. "compile", "test", "clippy").
    last_gate_failure_phase: Option<String>,
    /// The task id that was most recently dispatched for implementation.
    last_impl_task_id: Option<String>,
    review_feedback: Option<String>,
    impl_round: u32,
    /// Skill matched during the last dispatch (for confidence updates).
    last_matched_skill_id: Option<String>,
    /// Playbook rule matched during the last dispatch (for confidence updates).
    last_matched_rule_id: Option<String>,
    /// Prompt experiment variant assigned during the last dispatch.
    last_experiment_variant_id: Option<String>,
    /// Number of consecutive gate failures for this plan (for re-planning, §9).
    gate_failure_count: u32,
}

impl TaskTracker {
    fn new(tasks_file: TasksFile, plan_dir: PathBuf) -> Self {
        Self {
            tasks_file,
            completed: Vec::new(),
            failed: Vec::new(),
            current_group_index: 0,
            _plan_dir: plan_dir,
            last_gate_failure: None,
            last_gate_failure_phase: None,
            last_impl_task_id: None,
            review_feedback: None,
            impl_round: 0,
            last_matched_skill_id: None,
            last_matched_rule_id: None,
            last_experiment_variant_id: None,
            gate_failure_count: 0,
        }
    }

    /// Find the next unfinished task that has all deps satisfied.
    #[cfg(test)]
    #[allow(dead_code)]
    fn next_ready_task(
        &self,
        completed_plans: &[String],
    ) -> Option<&crate::task_parser::TaskDef> {
        self.ready_tasks(completed_plans).into_iter().next()
    }

    /// Return ALL ready tasks (deps satisfied, not completed, not failed).
    fn ready_tasks(&self, completed_plans: &[String]) -> Vec<&crate::task_parser::TaskDef> {
        self.tasks_file
            .tasks
            .iter()
            .filter(|t| {
                !self.completed.contains(&t.id)
                    && !self.failed.contains(&t.id)
                    && t.is_ready_with_plan_deps(&self.completed, completed_plans)
            })
            .collect()
    }

    /// Whether any unfinished task is currently blocked only by cross-plan deps.
    fn has_tasks_blocked_by_plans(&self, completed_plans: &[String]) -> bool {
        self.tasks_file.tasks.iter().any(|task| {
            !self.completed.contains(&task.id)
                && !self.failed.contains(&task.id)
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
            .all(|t| self.completed.contains(&t.id))
    }

    /// Mark a task as completed and advance group index if current group is fully done.
    fn mark_completed(&mut self, task_id: &str) {
        if !self.completed.contains(&task_id.to_string()) {
            self.completed.push(task_id.to_string());
        }
        // Advance group index if all tasks in current group are done
        let groups = self.tasks_file.parallel_groups();
        if self.current_group_index < groups.len() {
            let current_group_done = groups[self.current_group_index]
                .iter()
                .all(|t| self.completed.contains(&t.id));
            if current_group_done {
                self.current_group_index += 1;
            }
        }
    }

    /// Reset for re-implementation after review rejection.
    fn reset_for_reimpl(&mut self) {
        self.completed.clear();
        self.failed.clear();
        self.current_group_index = 0;
        self.impl_round += 1;
    }

    /// Return the most recently implemented task, if it still exists in the task file.
    fn last_impl_task(&self) -> Option<&crate::task_parser::TaskDef> {
        let task_id = self.last_impl_task_id.as_deref()?;
        self.tasks_file.tasks.iter().find(|task| task.id == task_id)
    }
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
        && groups[tracker.current_group_index]
            .iter()
            .all(|task| tracker.completed.iter().any(|completed| completed == &task.id))
    {
        tracker.current_group_index += 1;
    }
}

impl PlanRunner {
    /// Spawn MCP server processes and build a DynamicToolRegistry from their tools.
    ///
    /// Returns `(clients, registry)` where `registry` is `None` if no MCP config
    /// was found or no servers are configured.
    async fn setup_mcp(
        config: &Config,
        workdir: &Path,
    ) -> (
        Vec<roko_agent::mcp::McpClient<roko_agent::mcp::StdioTransport>>,
        Option<Arc<roko_agent::mcp::DynamicToolRegistry>>,
    ) {
        use roko_agent::mcp::{McpClient, StdioTransport, find_mcp_config, mcp_to_tool_def};
        use roko_core::tool::VecToolRegistry;

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
            _ => return (Vec::new(), None),
        };

        let mut clients = Vec::new();
        let mut all_server_tools = Vec::new();

        for server in &mcp_config.servers {
            match StdioTransport::spawn(&server.command, &server.args) {
                Ok(transport) => {
                    let client = McpClient::new(transport);
                    // Initialize the server
                    if let Err(e) = client.initialize().await {
                        tracing::warn!("MCP server '{}' initialize failed: {e}", server.name);
                        continue;
                    }
                    // List available tools
                    match client.list_tools().await {
                        Ok(tools) => {
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
                            clients.push(client);
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
            return (clients, None);
        }

        // Dedup across servers and build the dynamic registry.
        let deduped = roko_agent::mcp::dedup_tools(all_server_tools);
        let base = VecToolRegistry::new();
        let mut registry = roko_agent::mcp::DynamicToolRegistry::new(&base);
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

        (clients, Some(Arc::new(registry)))
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
                    eprintln!(
                        "[orchestrate] Plan {plan_id} depends on: {:?}",
                        fm.depends_on
                    );
                }
            }

            // Parse tasks.toml if it exists, log task count and parallel groups
            let tasks_path = plans_dir.join(&plan_info.base).join("tasks.toml");
            if tasks_path.exists() {
                if let Ok(tf) = crate::task_parser::TasksFile::parse(&tasks_path) {
                    let groups = tf.parallel_groups();
                    let model_tiers: Vec<String> = tf
                        .tasks
                        .iter()
                        .map(|t| format!("{}:{}", t.id, t.tier))
                        .collect();
                    eprintln!(
                        "[orchestrate] Plan {plan_id}: {} tasks, {} parallel groups, max_parallel={}, tiers=[{}]",
                        tf.tasks.len(),
                        groups.len(),
                        tf.meta.max_parallel,
                        model_tiers.join(", ")
                    );
                }
            }

            let state = PlanState::new(&plan_id);
            executor.add_plan(state);
        }

        // Wire cross-plan dependency ordering (§10).
        executor.set_plan_dependencies(plan_deps);

        // Pre-populate task trackers for plans with tasks.toml
        let mut task_trackers = HashMap::new();
        for plan_info in &plans {
            let plan_id = plan_info
                .frontmatter
                .as_ref()
                .and_then(|fm| fm.plan.clone())
                .unwrap_or_else(|| plan_info.base.clone());
            let tasks_path = plans_dir.join(&plan_info.base).join("tasks.toml");
            if tasks_path.exists() {
                if let Ok(tf) = TasksFile::parse(&tasks_path) {
                    let pdir = plans_dir.join(&plan_info.base);
                    task_trackers.insert(plan_id, TaskTracker::new(tf, pdir));
                }
            }
        }

        let cancel = CancelToken::new();
        let learning = LearningRuntime::open_under(workdir.join(".roko").join("learn"))
            .await
            .map_err(|e| anyhow!("init learning runtime: {e}"))?;
        let (mcp_clients, tool_registry) = Self::setup_mcp(&config, workdir).await;
        let obs_sinks = FsObservabilitySinks::for_workdir(workdir);
        obs_sinks
            .initialize()
            .context("initialize observability sinks")?;
        roko_core::obs::register_standard_metrics(&metrics);
        let health_probes = Self::build_health_probes(&config);
        Ok(Self {
            workdir: workdir.to_path_buf(),
            config,
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
            supervisor: ProcessSupervisor::new(cancel.clone()),
            cancel,
            task_trackers,
            conductor: Arc::new(Conductor::new()),
            conductor_signals: Vec::new(),
            attribution_tracker: ContextAttributionTracker::load(
                &workdir.join(".roko").join("context-attribution.jsonl"),
            ),
            plan_costs: HashMap::new(),
            task_costs: HashMap::new(),
            metrics,
            format_bandit: ProfileBandit::with_static_profiles(),
            mcp_clients,
            tool_registry,
            obs_sinks,
            health_probes,
            adaptive_thresholds: AdaptiveThresholds::load_or_new(
                &workdir
                    .join(".roko")
                    .join("learn")
                    .join("gate-thresholds.json"),
            ),
            efficiency_events: Vec::new(),
            server_event_bus: None,
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
    ) -> Result<Self> {
        let snapshot =
            ExecutorSnapshot::from_json(snapshot_json).map_err(|e| anyhow!("bad snapshot: {e}"))?;
        let executor = ParallelExecutor::from_snapshot(config.executor.clone(), snapshot);
        let legacy_completed = Self::legacy_completed_tasks_from_snapshot(snapshot_json);
        let task_trackers = Self::restore_task_trackers(workdir, &legacy_completed);
        let cancel = CancelToken::new();
        let learning = LearningRuntime::open_under(workdir.join(".roko").join("learn"))
            .await
            .map_err(|e| anyhow!("init learning runtime: {e}"))?;
        let (mcp_clients, tool_registry) = Self::setup_mcp(&config, workdir).await;
        let obs_sinks = FsObservabilitySinks::for_workdir(workdir);
        obs_sinks
            .initialize()
            .context("initialize observability sinks")?;
        roko_core::obs::register_standard_metrics(&metrics);
        let health_probes = Self::build_health_probes(&config);
        Ok(Self {
            workdir: workdir.to_path_buf(),
            config,
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
            supervisor: ProcessSupervisor::new(cancel.clone()),
            cancel,
            task_trackers,
            conductor: Arc::new(Conductor::new()),
            conductor_signals: Vec::new(),
            attribution_tracker: ContextAttributionTracker::load(
                &workdir.join(".roko").join("context-attribution.jsonl"),
            ),
            plan_costs: HashMap::new(),
            task_costs: HashMap::new(),
            metrics,
            format_bandit: ProfileBandit::with_static_profiles(),
            mcp_clients,
            tool_registry,
            obs_sinks,
            health_probes,
            adaptive_thresholds: AdaptiveThresholds::load_or_new(
                &workdir
                    .join(".roko")
                    .join("learn")
                    .join("gate-thresholds.json"),
            ),
            efficiency_events: Vec::new(),
            server_event_bus: None,
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
        let learning = LearningRuntime::open_under(workdir.join(".roko").join("learn"))
            .await
            .map_err(|e| anyhow!("init learning runtime: {e}"))?;
        let (mcp_clients, tool_registry) = Self::setup_mcp(&config, workdir).await;
        let obs_sinks = FsObservabilitySinks::for_workdir(workdir);
        obs_sinks
            .initialize()
            .context("initialize observability sinks")?;
        roko_core::obs::register_standard_metrics(&metrics);
        let health_probes = Self::build_health_probes(&config);
        Ok(Self {
            workdir: workdir.to_path_buf(),
            config,
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
            supervisor: ProcessSupervisor::new(cancel.clone()),
            cancel,
            task_trackers,
            conductor: Arc::new(Conductor::new()),
            conductor_signals: Vec::new(),
            attribution_tracker: ContextAttributionTracker::load(
                &workdir.join(".roko").join("context-attribution.jsonl"),
            ),
            plan_costs: HashMap::new(),
            task_costs: HashMap::new(),
            metrics,
            format_bandit: ProfileBandit::with_static_profiles(),
            mcp_clients,
            tool_registry,
            obs_sinks,
            health_probes,
            adaptive_thresholds: AdaptiveThresholds::load_or_new(
                &workdir
                    .join(".roko")
                    .join("learn")
                    .join("gate-thresholds.json"),
            ),
            efficiency_events: Vec::new(),
            server_event_bus: None,
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
        bus: bardo_runtime::event_bus::BusSender<crate::serve::events::ServerEvent>,
    ) {
        self.server_event_bus = Some(bus);
    }

    /// Emit a server event if a bus is attached.
    fn emit_server_event(&self, event: crate::serve::events::ServerEvent) {
        if let Some(bus) = &self.server_event_bus {
            bus.emit(event);
        }
    }

    /// Gracefully shut down all managed agent processes.
    pub async fn shutdown(&self) {
        let outcomes = self.supervisor.shutdown_all().await;
        if !outcomes.is_empty() {
            eprintln!("[orchestrate] shut down {} agent processes", outcomes.len());
        }
        // Dump prometheus metrics for post-mortem debugging.
        let metrics_dir = self.workdir.join(".roko").join("metrics");
        if let Err(e) = std::fs::create_dir_all(&metrics_dir) {
            eprintln!("[orchestrate] create metrics dir: {e}");
        } else {
            let prom = self.metrics.render_prometheus();
            if let Err(e) = std::fs::write(metrics_dir.join("prometheus.txt"), &prom) {
                eprintln!("[orchestrate] write prometheus.txt: {e}");
            }
        }
        // Persist adaptive gate thresholds.
        let thresholds_path = self
            .workdir
            .join(".roko")
            .join("learn")
            .join("gate-thresholds.json");
        if let Err(e) = self.adaptive_thresholds.save(&thresholds_path) {
            eprintln!("[orchestrate] save adaptive thresholds: {e}");
        }
        // Persist cascade router observations.
        if let Err(e) = self.learning.save_cascade_router() {
            eprintln!("[orchestrate] save cascade router: {e}");
        }
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

    /// Run conductor watchers against accumulated signals.
    /// Returns the decision and logs non-continue outcomes.
    fn run_conductor_check(&mut self, plan_id: &str) -> ConductorDecision {
        if self.conductor.circuit_breaker().is_broken(plan_id) {
            eprintln!("[conductor] pausing {plan_id}: circuit breaker tripped");
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
            self.event_log.append(
                EventKind::InterventionFired,
                payload.clone(),
            );
            self.emit_conductor_signal(
                Kind::Custom("conductor.circuit_breaker".into()),
                payload,
            );
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
                eprintln!("[conductor] {plan_id}: RESTART ({watcher}) — {reason}");
            }
            ConductorDecision::Fail { watcher, reason } => {
                eprintln!("[conductor] {plan_id}: FAIL ({watcher}) — {reason}");
            }
            _ => {}
        }
        decision
    }

    /// Push a conductor signal so watchers can detect anomalies (§7).
    fn emit_conductor_signal(&mut self, kind: Kind, body: serde_json::Value) {
        let sig = Signal::builder(kind).body(Body::Json(body)).build();
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
        let state_dir = self.workdir.join(".roko").join("state");
        std::fs::create_dir_all(&state_dir).map_err(|e| anyhow!("create state dir: {e}"))?;

        // Executor snapshot — atomic write.
        let exec_json = self.snapshot()?;
        let exec_path = state_dir.join("executor.json");
        let exec_tmp = state_dir.join("executor.json.tmp");
        std::fs::write(&exec_tmp, &exec_json).map_err(|e| anyhow!("write executor tmp: {e}"))?;
        std::fs::rename(&exec_tmp, &exec_path)
            .map_err(|e| anyhow!("rename executor snapshot: {e}"))?;

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

        Ok(())
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
                    "current_group_index": tracker.current_group_index,
                    "impl_round": tracker.impl_round,
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
            let plan_dir = workdir.join("plans").join(&plan_id);
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
            let current_group_index = entry["current_group_index"].as_u64().unwrap_or(0) as usize;
            let impl_round = entry["impl_round"].as_u64().unwrap_or(0) as u32;

            let mut tracker = TaskTracker::new(tf, plan_dir);
            tracker.completed = completed;
            tracker.failed = failed;
            tracker.current_group_index = current_group_index;
            tracker.impl_round = impl_round;
            if let Some(extra_completed) = completed_from_snapshot.get(&plan_id) {
                merge_completed_tasks(&mut tracker, extra_completed);
            }
            trackers.insert(plan_id, tracker);
        }

        for (plan_id, extra_completed) in completed_from_snapshot {
            if trackers.contains_key(plan_id) {
                continue;
            }
            let plan_dir = workdir.join("plans").join(plan_id);
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

    /// Extract completed task ids from legacy resume snapshots.
    ///
    /// Older `executor.json` files stored per-task records under `tasks`
    /// with a `status` field. Resume should preserve those completions so
    /// we do not rerun work that was already marked done/complete.
    fn legacy_completed_tasks_from_snapshot(
        snapshot_json: &str,
    ) -> HashMap<String, Vec<String>> {
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
        self.clear_stale_worktree_locks().await;
        // Clean up stale worktrees from previous runs (§6).
        if let Err(e) = self.worktrees.prune().await {
            eprintln!("[orchestrate] worktree prune failed: {e}");
        }
        if let Err(e) = self.worktrees.reclaim_idle().await {
            eprintln!("[orchestrate] worktree reclaim failed: {e}");
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
                let _ = self.executor.apply_event(plan_id, &ExecutorEvent::Start);
                self.emit_server_event(crate::serve::events::ServerEvent::PlanStarted {
                    plan_id: plan_id.clone(),
                });
            }
        }

        // Maximum iterations to prevent infinite loops.
        let max_iterations = 1000;
        let mut iteration = 0;

        loop {
            iteration += 1;
            if iteration > max_iterations {
                eprintln!("[orchestrate] hit max iterations ({max_iterations}), stopping");
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
                if self.all_terminal(&plan_ids) {
                    break;
                }
                // No actions but not all terminal — wait and retry.
                tokio::select! {
                    _ = watcher_cancel.cancelled() => break,
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {}
                }
                continue;
            }

            for action in actions {
                self.dispatch_action(action).await;
            }

            // Auto-save periodically.
            if self.actions_since_save >= AUTOSAVE_INTERVAL {
                if let Err(e) = self.save_state() {
                    eprintln!("[orchestrate] auto-save failed: {e}");
                }
                self.actions_since_save = 0;
            }
        }

        // Clean up worktrees after completion (§6).
        if let Err(e) = self.worktrees.reclaim_idle().await {
            eprintln!("[orchestrate] post-run worktree reclaim failed: {e}");
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

        // Emit plan-completed server events.
        for p in &plans {
            self.emit_server_event(crate::serve::events::ServerEvent::PlanCompleted {
                plan_id: p.plan_id.clone(),
                success: p.succeeded,
            });
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

        // Final save before returning.
        if let Err(e) = self.save_state() {
            eprintln!("[orchestrate] final save failed: {e}");
        }

        Ok(OrchestrationReport {
            total_agent_calls: self.agent_calls,
            total_gate_runs: self.gate_runs,
            plans,
        })
    }

    /// Run plans using tasks.toml files, routing through the full 14-phase
    /// executor state machine.
    ///
    /// Pre-loads [`TaskTracker`]s for plans that have `tasks.toml`, then
    /// delegates to [`run_all()`] which drives the state machine. The phase
    /// handlers (handle_enriching, handle_implementing, etc.) use the
    /// trackers for task-level granularity.
    pub async fn run_task_plans(&mut self, plans_dir: &Path) -> Result<OrchestrationReport> {
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
            let plan_dirs = Self::find_plan_dirs(plans_dir)?;
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
                eprintln!("[orchestrate] SpawnAgent plan={plan_id} role={role:?} task={task}");
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
                eprintln!("[orchestrate] RunGate plan={plan_id} rung={rung}");
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
                        ep.gate_verdicts
                            .push(GateVerdict::new(format!("rung-{rung}"), passed));
                        ep.input_signal_hash.clone_from(&plan_id);
                        let gate_input = self.enrich_completed_run(
                            ep,
                            &plan_id,
                            &format!("rung-{rung}"),
                            "gate",
                            "n/a",
                            Some(passed),
                            1,
                        );
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

                        // Conductor signal: gate verdict (§7).
                        self.emit_conductor_signal(
                            Kind::GateVerdict,
                            serde_json::json!({
                                "plan_id": &plan_id,
                                "rung": rung,
                                "passed": passed,
                                "duration_ms": wall_ms,
                            }),
                        );

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
                        }
                        let event = if passed {
                            if let Some(tracker) = self.task_trackers.get_mut(&plan_id) {
                                tracker.gate_failure_count = 0;
                            }
                            ExecutorEvent::GatePassed
                        } else {
                            if let Some(tracker) = self.task_trackers.get_mut(&plan_id) {
                                tracker.gate_failure_count += 1;
                            }
                            ExecutorEvent::GateFailed
                        };
                        let _ = self.executor.apply_event(&plan_id, &event);

                        // Failure-driven re-planning (§9): after 3 consecutive
                        // gate failures, attempt to re-plan.
                        if !passed {
                            let failure_count = self
                                .task_trackers
                                .get(&plan_id)
                                .map(|t| t.gate_failure_count)
                                .unwrap_or(0);
                            if failure_count >= 3 && self.executor.config().auto_replan {
                                self.attempt_replan(&plan_id).await;
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[orchestrate] gate failed for {plan_id}: {e}");
                        self.event_log.append(
                            EventKind::ErrorOccurred,
                            serde_json::json!({"plan_id": plan_id, "error": e.to_string()}),
                        );
                        let _ = self
                            .executor
                            .apply_event(&plan_id, &ExecutorEvent::GateFailed);
                    }
                }
                // Conductor check after gate results.
                match self.run_conductor_check(&plan_id) {
                    ConductorDecision::Continue => {}
                    ConductorDecision::Restart { reason, .. } => {
                        eprintln!("[conductor] restarting {plan_id}: {reason}");
                        let _ = self.executor.apply_event(&plan_id, &ExecutorEvent::Start);
                    }
                    ConductorDecision::Fail { reason, .. } => {
                        eprintln!("[conductor] failing {plan_id}: {reason}");
                        let _ = self.executor.apply_event(
                            &plan_id,
                            &ExecutorEvent::Fatal(format!("conductor: {reason}")),
                        );
                    }
                    _ => {}
                }
            }
            ExecutorAction::RunVerify { plan_id } => {
                eprintln!("[orchestrate] RunVerify plan={plan_id}");
                self.finish_verify_round(&plan_id).await;
            }
            ExecutorAction::MergeBranch { plan_id } => {
                eprintln!("[orchestrate] MergeBranch plan={plan_id}");
                self.event_log.append(
                    EventKind::MergeAttempted,
                    serde_json::json!({"plan_id": plan_id}),
                );
                match self.merge_branch(&plan_id).await {
                    Ok(()) => {
                        match self.run_post_merge_follow_up(&plan_id).await {
                            Ok(true) => {
                                let _ = self
                                    .executor
                                    .apply_event(&plan_id, &ExecutorEvent::MergeSucceeded);
                            }
                            Ok(false) => {
                                let _ = self
                                    .executor
                                    .apply_event(&plan_id, &ExecutorEvent::MergeFailed);
                            }
                            Err(e) => {
                                eprintln!(
                                    "[orchestrate] post-merge checks failed for {plan_id}: {e}"
                                );
                                self.event_log.append(
                                    EventKind::ErrorOccurred,
                                    serde_json::json!({"plan_id": plan_id, "error": format!("post-merge follow-up failed: {e}")}),
                                );
                                // Keep historical behavior on infrastructure errors:
                                // merge itself succeeded.
                                let _ = self
                                    .executor
                                    .apply_event(&plan_id, &ExecutorEvent::MergeSucceeded);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[orchestrate] merge failed for {plan_id}: {e}");
                        let _ = self
                            .executor
                            .apply_event(&plan_id, &ExecutorEvent::MergeFailed);
                    }
                }
                // Conductor check after merge results.
                match self.run_conductor_check(&plan_id) {
                    ConductorDecision::Continue => {}
                    ConductorDecision::Restart { reason, .. } => {
                        eprintln!("[conductor] restarting {plan_id}: {reason}");
                        let _ = self.executor.apply_event(&plan_id, &ExecutorEvent::Start);
                    }
                    ConductorDecision::Fail { reason, .. } => {
                        eprintln!("[conductor] failing {plan_id}: {reason}");
                        let _ = self.executor.apply_event(
                            &plan_id,
                            &ExecutorEvent::Fatal(format!("conductor: {reason}")),
                        );
                    }
                    _ => {}
                }
            }
            ExecutorAction::DispatchPlan { plan_id } => {
                eprintln!("[orchestrate] DispatchPlan {plan_id}");
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
                let _ = self.executor.apply_event(&plan_id, &ExecutorEvent::Start);
            }
            ExecutorAction::PausePlan { plan_id } => {
                eprintln!("[orchestrate] PausePlan {plan_id}");
                self.executor.pause_plan(&plan_id);
            }
            ExecutorAction::ResumePlan { plan_id } => {
                eprintln!("[orchestrate] ResumePlan {plan_id}");
                self.executor.resume_plan(&plan_id);
            }
            ExecutorAction::FailPlan { plan_id, reason } => {
                eprintln!("[orchestrate] FailPlan {plan_id}: {reason}");
                self.event_log.append(
                    EventKind::ErrorOccurred,
                    serde_json::json!({"plan_id": &plan_id, "error": reason.clone()}),
                );
                let _ = self
                    .executor
                    .apply_event(&plan_id, &ExecutorEvent::Fatal(reason));
            }
            ExecutorAction::CompletePlan { plan_id } => {
                eprintln!("[orchestrate] CompletePlan {plan_id}");
                if let Some(state) = self.executor.plan_state_mut(&plan_id) {
                    state.current_phase = roko_core::PlanPhase::Complete;
                    state.paused = false;
                }
                self.event_log.append(
                    EventKind::PhaseTransition,
                    serde_json::json!({"plan_id": &plan_id, "event": "CompletePlan"}),
                );
            }
            ExecutorAction::Reorder {
                plan_id,
                new_position,
            } => {
                eprintln!("[orchestrate] Reorder {plan_id} -> {new_position}");
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
                ep.input_signal_hash = plan_id.to_string();
                ep.output_signal_hash = result.output.id.to_string();
                let model = self.effective_model();
                let input = self.enrich_completed_run(ep, plan_id, "enrich", "Strategist", &model, None, 1);
                self.record_and_check_learning(input, plan_id).await;

                if let Some(tracker) = self.task_trackers.get(plan_id) {
                    let groups = tracker.tasks_file.parallel_groups();
                    eprintln!(
                        "[orchestrate] Enriching {plan_id}: {} tasks, {} parallel groups",
                        tracker.tasks_file.tasks.len(),
                        groups.len(),
                    );
                } else {
                    eprintln!(
                        "[orchestrate] Enriching {plan_id}: no tasks.toml, using generic strategist enrichment"
                    );
                }

                let event = ExecutorEvent::EnrichmentDone;
                self.log_transition(plan_id, &event);
                let _ = self.executor.apply_event(plan_id, &event);
            }
            Err(e) => {
                eprintln!("[orchestrate] Enrichment failed for {plan_id}: {e}");
                let _ = self.executor.apply_event(
                    plan_id,
                    &ExecutorEvent::Fatal(format!("enrichment failed: {e}")),
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

        if ready.is_empty() {
            // No ready tasks — check if all done or blocked
            let all_done = self
                .task_trackers
                .get(plan_id)
                .is_some_and(TaskTracker::all_tasks_done);
            if all_done {
                let event = ExecutorEvent::ImplementationDone;
                self.log_transition(plan_id, &event);
                let _ = self.executor.apply_event(plan_id, &event);
            } else if self
                .task_trackers
                .get(plan_id)
                .is_some_and(|tracker| tracker.has_tasks_blocked_by_plans(&completed_plans))
            {
                eprintln!(
                    "[orchestrate] {plan_id}: implementation blocked by dependent plan(s), pausing"
                );
                self.executor.pause_plan(plan_id);
            } else {
                eprintln!(
                    "[orchestrate] {plan_id}: no ready tasks but not all done — blocked or failed"
                );
                let _ = self.executor.apply_event(
                    plan_id,
                    &ExecutorEvent::Fatal("all remaining tasks blocked or failed".into()),
                );
            }
            return;
        }

        if ready.len() == 1 {
            // ── Single task: sequential dispatch with retry ──────────
            self.handle_implementing_single(plan_id, &ready[0]).await;
        } else {
            // ── Multiple ready tasks: parallel dispatch ──────────────
            let batch = ready;
            eprintln!(
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
            eprintln!("[orchestrate] {plan_id}: all tasks done, advancing to Gating");
            let event = ExecutorEvent::ImplementationDone;
            self.log_transition(plan_id, &event);
            let _ = self.executor.apply_event(plan_id, &event);
        }

        // Conductor check after agent dispatch completes.
        match self.run_conductor_check(plan_id) {
            ConductorDecision::Restart { reason, .. } => {
                eprintln!("[conductor] restarting {plan_id}: {reason}");
                let _ = self.executor.apply_event(plan_id, &ExecutorEvent::Start);
            }
            ConductorDecision::Fail { reason, .. } => {
                eprintln!("[conductor] failing {plan_id}: {reason}");
                let _ = self.executor.apply_event(
                    plan_id,
                    &ExecutorEvent::Fatal(format!("conductor: {reason}")),
                );
            }
            _ => {}
        }
        // else: plan stays in Implementing. Next tick() returns another SpawnAgent.
    }

    /// Dispatch a single task with retry logic (up to 2 retries).
    async fn handle_implementing_single(&mut self, plan_id: &str, task_id: &str) {
        eprintln!("[orchestrate] Implementing {plan_id}: dispatching task {task_id}");

        // Track which task is being worked on (used by autofix if gates fail).
        if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
            tracker.last_impl_task_id = Some(task_id.to_string());
        }

        let wt_id = format!("{plan_id}-{task_id}");
        let started = std::time::Instant::now();
        let max_retries = 2u32;
        let mut succeeded = false;
        let mut budget_aborted = false;
        let exec_dir = match self.task_exec_dir(plan_id, task_id).await {
            Ok(dir) => dir,
            Err(e) => {
                eprintln!(
                    "[orchestrate] task worktree acquisition failed for {plan_id}/{task_id}: {e}"
                );
                self.record_task_failure(plan_id, task_id, &e, &started, None)
                    .await;
                let _ = self.executor.apply_event(
                    plan_id,
                    &ExecutorEvent::Fatal(format!(
                        "failed to acquire worktree for task {task_id}: {e}"
                    )),
                );
                return;
            }
        };

        for attempt in 0..=max_retries {
            if attempt > 0 {
                eprintln!("[orchestrate] Retry {attempt}/{max_retries} for {plan_id}/{task_id}");
            }

            match self
                .dispatch_agent_with(
                    plan_id,
                    AgentRole::Implementer,
                    task_id,
                    None,
                    None,
                    Some(exec_dir.clone()),
                    None,
                )
                .await
            {
                Ok(result) => {
                    match self
                        .record_task_success(plan_id, task_id, &result, &started)
                        .await
                    {
                        Ok(()) => {
                            succeeded = true;
                        }
                        Err(e) => {
                            eprintln!("[orchestrate] task {task_id} aborted by plan budget: {e}");
                            let _ = self
                                .executor
                                .apply_event(plan_id, &ExecutorEvent::Fatal(e.to_string()));
                            budget_aborted = true;
                        }
                    }
                    break;
                }
                Err(e) => {
                    eprintln!(
                        "[orchestrate] task {task_id} failed (attempt {}): {e}",
                        attempt + 1
                    );
                    if attempt == max_retries {
                        self.record_task_failure(plan_id, task_id, &e, &started, None)
                            .await;
                    }
                }
            }
        }

        if let Err(e) = self.worktrees.remove(&wt_id).await {
            eprintln!("[orchestrate] worktree cleanup failed for {task_id}: {e}");
        }

        if !succeeded && !budget_aborted {
            eprintln!("[orchestrate] task {task_id} failed after {max_retries} retries");
            let _ = self.executor.apply_event(
                plan_id,
                &ExecutorEvent::Fatal(format!("task {task_id} failed after retries")),
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
                eprintln!(
                    "[orchestrate] task budget exhausted before dispatch for {plan_id}/{tid}: {e}"
                );
                self.record_task_failure(plan_id, tid, &e, &started, None).await;
                continue;
            }
            match self.task_exec_dir(plan_id, tid).await {
                Ok(dir) => task_dirs.push((tid.clone(), dir)),
                Err(e) => {
                    eprintln!(
                        "[orchestrate] task worktree acquisition failed for {plan_id}/{tid}: {e}"
                    );
                    self.record_task_failure(plan_id, tid, &e, &started, None)
                        .await;
                }
            }
        }

        // Track all tasks as in-progress.
        if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
            if let Some(first) = task_ids.first() {
                tracker.last_impl_task_id = Some(first.clone());
            }
        }

        // ── Build agent configs sequentially (needs &mut self) ───────
        let mut configs: Vec<(String, AgentRunConfig)> = Vec::with_capacity(task_dirs.len());

        let plan_dir = self.workdir.join("plans").join(plan_id);
        let tasks_toml = plan_dir.join("tasks.toml");
        let tasks_file = if tasks_toml.exists() {
            crate::task_parser::TasksFile::parse(&tasks_toml).ok()
        } else {
            None
        };

        let role = AgentRole::Implementer;
        let claude_tools_csv = claude_tool_allowlist(role);
        let skip_perms = role == AgentRole::Implementer || role == AgentRole::AutoFixer;

        for (tid, dir) in &task_dirs {
            let task_def = tasks_file
                .as_ref()
                .and_then(|tf| tf.tasks.iter().find(|t| t.id == *tid).cloned());

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

            let system_prompt = build_system_prompt(role, plan_id, tid, &claude_tools_csv);
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
                AgentRunConfig {
                    command: self.config.agent.command.clone(),
                    exec_dir: dir.clone(),
                    model,
                    timeout_ms: self.effective_task_timeout_ms(task_def.as_ref()),
                    bare_mode: self.config.agent.bare_mode,
                    effort: self.config.agent.effort.clone(),
                    system_prompt,
                    tools_csv: claude_tools_csv.clone(),
                    mcp_config: self.config.agent.mcp_config.clone(),
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

        let mut results: Vec<(String, AgentResult)> = Vec::with_capacity(task_ids.len());
        let mut pending = configs.into_iter();
        loop {
            // Run one dependency-level slice at a time, capping the number of
            // spawned tasks to the configured executor limit.
            let mut join_set = JoinSet::new();
            let mut launched = 0usize;
            while launched < concurrency_limit {
                let Some((tid, cfg)) = pending.next() else {
                    break;
                };
                launched += 1;
                join_set.spawn(async move {
                    let result = run_prepared_agent(cfg).await;
                    (tid, result)
                });
            }

            if launched == 0 {
                break;
            }

            while let Some(joined) = join_set.join_next().await {
                match joined {
                    Ok(pair) => results.push(pair),
                    Err(e) => {
                        eprintln!("[orchestrate] parallel task join failed: {e}");
                    }
                }
            }
        }

        // ── Process results sequentially ─────────────────────────────
        let mut any_fatal = false;
        for (tid, agent_result) in &results {
            self.add_task_spend(plan_id, tid, f64::from(agent_result.usage.cost_usd));
            if agent_result.success {
                if let Err(e) = self
                    .record_task_success(plan_id, tid, agent_result, &started)
                    .await
                {
                    eprintln!("[orchestrate] task {tid} aborted by plan budget: {e}");
                    let _ = self
                        .executor
                        .apply_event(plan_id, &ExecutorEvent::Fatal(e.to_string()));
                    any_fatal = true;
                    break;
                }
            } else {
                eprintln!("[orchestrate] parallel task {tid} failed");
                let err = anyhow!("agent returned non-success for task {tid}");
                self.record_task_failure(plan_id, tid, &err, &started, Some(agent_result))
                    .await;
                any_fatal = true;
            }
        }

        // ── Clean up per-task worktrees ──────────────────────────────
        for tid in task_ids {
            let wt_id = format!("{plan_id}-{tid}");
            if let Err(e) = self.worktrees.remove(&wt_id).await {
                eprintln!("[orchestrate] worktree cleanup failed for {tid}: {e}");
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
            let _ = self.executor.apply_event(
                plan_id,
                &ExecutorEvent::Fatal(
                    "parallel batch had failures; remaining tasks blocked".into(),
                ),
            );
        }
    }

    /// Build a [`CompletedRunInput`] enriched with cost record, provider, and
    /// task metric data derived from the agent result context.
    fn enrich_completed_run(
        &self,
        ep: Episode,
        plan_id: &str,
        task_id: &str,
        role: &str,
        model: &str,
        gate_passed: Option<bool>,
        iteration: u32,
    ) -> CompletedRunInput {
        let cost = CostRecord {
            timestamp: chrono::Utc::now().to_rfc3339(),
            model: model.to_string(),
            provider: "anthropic".to_string(),
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
        input.provider = Some(model.to_string());

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

    /// Build a learned-context string from skills, playbook rules, and patterns.
    ///
    /// Returns the context text plus any matched skill/rule IDs for flowing
    /// into `CompletedRunInput` so confidence gets updated.
    fn build_learned_context(&self, role: AgentRole, task_text: &str) -> LearnedContext {
        use roko_learn::playbook_rules::MatchContext;

        let mut parts: Vec<String> = Vec::new();
        let mut matched_skill_id: Option<String> = None;
        let mut matched_rule_id: Option<String> = None;

        // 1. Relevant skills from the skill library (search by role tag)
        let role_tag = format!("{role:?}").to_lowercase();
        let skills = self.learning.skill_library().search_by_tag(&role_tag);
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

        // 4. Prompt experiment variants — check if any active experiment applies.
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

        // 5. Crate familiarity score from cascade router observations (§9).
        let obs_count = self.learning.cascade_router().total_observations();
        if obs_count > 0 {
            let familiarity = (obs_count as f64 / 100.0).min(1.0);
            parts.push(format!(
                "## Crate Familiarity\nBased on {obs_count} prior observations, \
                 familiarity score: {familiarity:.2}/1.0."
            ));
        }

        // Ignore task_text for now — use it in future for semantic search
        let _ = task_text;

        LearnedContext {
            text: parts.join("\n"),
            matched_skill_id,
            matched_rule_id,
            experiment_variant_id,
        }
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

        // ── Observe cascade router for bandit learning (§9) ─────────
        {
            use roko_core::TaskComplexityBand;
            use roko_learn::model_router::RoutingContext;

            let task_def = self
                .task_trackers
                .get(plan_id)
                .and_then(|t| t.tasks_file.tasks.iter().find(|td| td.id == task_id));
            let complexity = task_def
                .map(|td| match td.tier.as_str() {
                    "mechanical" | "fast" => TaskComplexityBand::Fast,
                    "architectural" | "complex" | "premium" => TaskComplexityBand::Complex,
                    _ => TaskComplexityBand::Standard,
                })
                .unwrap_or(TaskComplexityBand::Standard);
            let routing_ctx = RoutingContext {
                task_category: roko_core::TaskCategory::Implementation,
                complexity,
                iteration: 0,
                role: AgentRole::Implementer,
                crate_familiarity: 0.5,
                has_prior_failure: false,
            };
            let model = self.effective_model();
            let reward = if result.success { 1.0 } else { 0.0 };
            self.learning.cascade_router().record_observation(
                &routing_ctx,
                &model,
                reward,
                result.success,
            );
        }

        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let mut ep = Episode::new("Implementer", task_id).succeeded();
        ep.usage = Usage {
            wall_ms,
            cost_usd: f64::from(result.usage.cost_usd),
            input_tokens: u64::from(result.usage.input_tokens),
            output_tokens: u64::from(result.usage.output_tokens),
            ..Usage::default()
        };
        ep.input_signal_hash = plan_id.to_string();
        ep.output_signal_hash = result.output.id.to_string();
        let model = self.effective_model();
        let input = self.enrich_completed_run(ep, plan_id, task_id, "Implementer", &model, None, 1);
        self.record_and_check_learning(input, plan_id).await;

        // Emit efficiency event for this agent turn.
        self.emit_efficiency_event(
            plan_id,
            task_id,
            "Implementer",
            &model,
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
        }

        // Emit observability trace event for the successful agent dispatch.
        self.emit_agent_trace(plan_id, task_id, true, wall_ms);

        eprintln!("[orchestrate] task {task_id} completed");
        Ok(())
    }

    /// Record a completed run and check the returned `LearningUpdate` for
    /// regression alerts.
    async fn record_and_check_learning(&mut self, input: CompletedRunInput, plan_id: &str) {
        match self.learning.record_completed_run(input).await {
            Ok(update) => self.handle_learning_update(&update, plan_id),
            Err(e) => eprintln!("[orchestrate] episode log failed: {e}"),
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

    /// Attempt to re-plan after repeated gate failures (§9).
    ///
    /// Spawns a Strategist agent with the failure context and asks it to
    /// update the remaining tasks. Resets the gate failure counter on success.
    async fn attempt_replan(&mut self, plan_id: &str) {
        let failure_context = self
            .task_trackers
            .get(plan_id)
            .and_then(|t| t.last_gate_failure.clone())
            .unwrap_or_default();
        let failure_phase = self
            .task_trackers
            .get(plan_id)
            .and_then(|t| t.last_gate_failure_phase.clone())
            .unwrap_or_default();

        let prompt = format!(
            "The plan '{plan_id}' has failed gates 3+ times consecutively.\n\n\
             Last failing phase: {failure_phase}\n\
             Failure details:\n```\n{failure_context}\n```\n\n\
             Analyze the failures and suggest concrete fixes. Focus on the root cause \
             and provide updated implementation steps."
        );

        eprintln!("[orchestrate] Attempting re-plan for {plan_id} after repeated gate failures");
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
            Ok(_result) => {
                eprintln!("[orchestrate] Re-plan completed for {plan_id}");
                if let Some(tracker) = self.task_trackers.get_mut(plan_id) {
                    tracker.gate_failure_count = 0;
                }
                // Reset to implementing phase so the updated plan gets executed.
                let _ = self
                    .executor
                    .apply_event(plan_id, &ExecutorEvent::EnrichmentDone);
            }
            Err(e) => {
                eprintln!("[orchestrate] Re-plan failed for {plan_id}: {e}");
            }
        }
    }

    fn primary_failed_gate_name_from_results<'a>(
        verdicts: &'a [&'a GateResult],
    ) -> Option<&'a str> {
        verdicts
            .iter()
            .find(|v| {
                !v.passed && matches!(v.gate_name.as_str(), "compile" | "test" | "clippy")
            })
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
            if let Some(digest) = verdict.error_digest.as_deref().map(str::trim).filter(|s| !s.is_empty())
            {
                section.push_str("\n\nerror_digest:\n");
                section.push_str(digest);
            }
            if let Some(detail) = verdict.detail.as_deref().map(str::trim).filter(|s| !s.is_empty())
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
        error: &anyhow::Error,
        started: &std::time::Instant,
        result: Option<&AgentResult>,
    ) {
        let wall_ms = result
            .map(|r| r.usage.wall_ms)
            .unwrap_or_else(|| u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
        let mut ep = Episode::new("Implementer", task_id).failed(error.to_string());
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
        let model = self.effective_model();
        let input = self.enrich_completed_run(ep, plan_id, task_id, "Implementer", &model, None, 1);
        self.record_and_check_learning(input, plan_id).await;
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
            eprintln!(
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
                let _ = self.executor.apply_event(plan_id, &event);
            }
            Err(e) => {
                eprintln!("[orchestrate] AutoFix failed for {plan_id}: {e}");
                let _ = self.executor.apply_event(
                    plan_id,
                    &ExecutorEvent::Fatal(format!("autofix failed: {e}")),
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
                    self.finish_verify_round(plan_id).await;
                }
            }
            Err(e) => {
                eprintln!("[orchestrate] RegenVerify failed for {plan_id}: {e}");
                let _ = self.executor.apply_event(
                    plan_id,
                    &ExecutorEvent::Fatal(format!("regen-verify failed: {e}")),
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
                let _ = self.executor.apply_event(plan_id, &ExecutorEvent::VerifyPassed);
            }
            Err((task_id, phase, command, error_output)) => {
                let msg = format!(
                    "verify failed for {plan_id}/{task_id} in phase {phase}: {command}"
                );
                eprintln!("[orchestrate] {msg}: {}", error_output.trim());
                self.event_log.append(
                    EventKind::ErrorOccurred,
                    serde_json::json!({
                        "plan_id": plan_id,
                        "task_id": task_id,
                        "phase": phase,
                        "command": command,
                        "error": error_output,
                    }),
                );
                if let Some(state) = self.executor.plan_state_mut(plan_id) {
                    state.last_error = Some(msg);
                }
                let _ = self.executor.apply_event(plan_id, &ExecutorEvent::VerifyFailed);
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
                let drift_report =
                    self.task_trackers.get(plan_id).and_then(|tracker| {
                        review_drift_report(&tracker.tasks_file, &output_text)
                    });
                if let Some(ref report) = drift_report {
                    if report.drifted() {
                        approved = false;
                    }
                }
                eprintln!(
                    "[orchestrate] Review {plan_id}: verdict={} drift={}",
                    if approved { "approved" } else { "revise" },
                    drift_report
                        .as_ref()
                        .map(|r: &ReviewDriftReport| {
                            format!("{:.1}% ({}/{})", r.coverage() * 100.0, r.matched, r.expected)
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
                ep.input_signal_hash = plan_id.to_string();
                ep.output_signal_hash = result.output.id.to_string();
                let model = self.effective_model();
                let input =
                    self.enrich_completed_run(ep, plan_id, "review", "Auditor", &model, None, 1);
                self.record_and_check_learning(input, plan_id).await;

                if approved {
                    let event = ExecutorEvent::ReviewApproved;
                    self.log_transition(plan_id, &event);
                    let _ = self.executor.apply_event(plan_id, &event);
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
                    let _ = self.executor.apply_event(plan_id, &event);
                }
            }
            Err(e) => {
                // On infrastructure error, auto-approve (don't block pipeline)
                eprintln!("[orchestrate] Review failed for {plan_id}: {e} — auto-approving");
                let event = ExecutorEvent::ReviewApproved;
                self.log_transition(plan_id, &event);
                let _ = self.executor.apply_event(plan_id, &event);
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
                ep.input_signal_hash = plan_id.to_string();
                ep.output_signal_hash = result.output.id.to_string();
                let model = self.effective_model();
                let input =
                    self.enrich_completed_run(ep, plan_id, "docs", "Scribe", &model, None, 1);
                self.record_and_check_learning(input, plan_id).await;
            }
            Err(e) => {
                eprintln!(
                    "[orchestrate] DocRevision failed for {plan_id}: {e} — continuing (non-blocking)"
                );
            }
        }
        // Always advance regardless of success/failure
        let event = ExecutorEvent::DocRevisionDone;
        self.log_transition(plan_id, &event);
        let _ = self.executor.apply_event(plan_id, &event);
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
                eprintln!(
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
                        eprintln!("[orchestrate] episode log failed: {e}");
                    }
                    let event = self.generic_completion_event(plan_id);
                    self.log_transition(plan_id, &event);
                    let _ = self.executor.apply_event(plan_id, &event);
                    succeeded = true;
                    break;
                }
                Err(e) => {
                    last_error = e.to_string();
                    if attempt == max_retries {
                        eprintln!(
                            "[orchestrate] agent failed for {plan_id} after {max_retries} retries: {e}"
                        );
                        let wall_ms =
                            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                        let mut ep = Episode::new(format!("{role:?}"), task).failed(e.to_string());
                        ep.usage = Usage {
                            wall_ms,
                            ..Usage::default()
                        };
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
                        let _ = self.executor.apply_event(
                            plan_id,
                            &ExecutorEvent::Fatal(format!(
                                "agent error after {attempt} retries: {e}"
                            )),
                        );
                    }
                }
            }
        }

        if !succeeded {
            eprintln!("[orchestrate] All retries exhausted for {plan_id}/{task}");
        }
    }

    /// Ensure a TaskTracker exists for the given plan (lazy loading).
    fn ensure_task_tracker(&mut self, plan_id: &str) {
        if self.task_trackers.contains_key(plan_id) {
            return;
        }
        let plan_dir = self.workdir.join("plans").join(plan_id);
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
        let ctx = Context::now();
        let exec_dir = match exec_dir_override {
            Some(dir) => dir,
            None => self.plan_exec_dir(plan_id).await,
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
        let plan_dir = self.workdir.join("plans").join(plan_id);
        let tasks_toml = plan_dir.join("tasks.toml");
        let tasks_file = if tasks_toml.exists() {
            crate::task_parser::TasksFile::parse(&tasks_toml).ok()
        } else {
            None
        };
        let task_def = tasks_file
            .as_ref()
            .and_then(|tf| tf.tasks.iter().find(|t| t.id == task).cloned());

        // ── Build prompt: surgical (from TaskDef) or generic ────────
        // Also collect attribution keys for context feedback after the agent runs.
        let mut attribution_keys: Vec<(String, String)> = Vec::new();
        let (task_text, mut selected_model) = if let Some(override_prompt) = prompt_override {
            let model = model_override.unwrap_or_else(|| {
                self.config
                    .agent
                    .model
                    .clone()
                    .unwrap_or_else(|| "claude-sonnet-4-6".into())
            });
            (override_prompt, model)
        } else if let Some(ref td) = task_def {
            let prompt = td.build_prompt(plan_id, &self.workdir);
            let model = td.effective_model(
                self.config
                    .agent
                    .model
                    .as_deref()
                    .unwrap_or("claude-sonnet-4-6"),
                Some(&self.config.agent.tier_models),
            );
            eprintln!(
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

        // ── Adaptive model selection via CascadeRouter ───────────────
        if let Some(td) = task_def.as_ref() {
            if td.model_hint.is_some() {
                eprintln!(
                    "[orchestrate] Task {} model_hint={} (skipping CascadeRouter)",
                    td.id, selected_model
                );
            } else {
                use roko_core::TaskComplexityBand;
                use roko_learn::model_router::RoutingContext;

                let cascade_router = self.learning.cascade_router();
                let complexity = match td.tier.as_str() {
                    "mechanical" | "fast" => TaskComplexityBand::Fast,
                    "architectural" | "complex" | "premium" => TaskComplexityBand::Complex,
                    _ => TaskComplexityBand::Standard,
                };
                let has_prior_failure = self
                    .task_trackers
                    .get(plan_id)
                    .is_some_and(|t| t.last_gate_failure.is_some());
                let routing_ctx = RoutingContext {
                    task_category: roko_core::TaskCategory::Implementation,
                    complexity,
                    iteration: 0,
                    role,
                    crate_familiarity: 0.5,
                    has_prior_failure,
                };
                let cascade = cascade_router.route(&routing_ctx);
                // Use cascade recommendation only if it has enough observations.
                // Otherwise stick with the statically selected model.
                if cascade.stage != roko_learn::cascade_router::CascadeStage::Static {
                    eprintln!(
                        "[orchestrate] CascadeRouter recommends model={} (stage={:?})",
                        cascade.primary.slug, cascade.stage
                    );
                    selected_model = cascade.primary.slug;
                }
            }
        } else {
            use roko_core::TaskComplexityBand;
            use roko_learn::model_router::RoutingContext;

            let cascade_router = self.learning.cascade_router();
            let routing_ctx = RoutingContext {
                task_category: roko_core::TaskCategory::Implementation,
                complexity: TaskComplexityBand::Standard,
                iteration: 0,
                role,
                crate_familiarity: 0.5,
                has_prior_failure: self
                    .task_trackers
                    .get(plan_id)
                    .is_some_and(|t| t.last_gate_failure.is_some()),
            };
            let cascade = cascade_router.route(&routing_ctx);
            if cascade.stage != roko_learn::cascade_router::CascadeStage::Static {
                eprintln!(
                    "[orchestrate] CascadeRouter recommends model={} (stage={:?})",
                    cascade.primary.slug, cascade.stage
                );
                selected_model = cascade.primary.slug;
            }
        }

        // ── Provider health check ────────────────────────────────────
        let selected_model = if !self.learning.provider_health().is_healthy(&selected_model) {
            let fallback = self
                .config
                .agent
                .fallback_model
                .clone()
                .unwrap_or_else(|| "claude-sonnet-4-6".into());
            tracing::warn!(
                unhealthy_model = %selected_model,
                fallback_model = %fallback,
                "model marked unhealthy by ProviderHealthTracker, falling back"
            );
            fallback
        } else {
            selected_model
        };

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

            let mut resolved = context_provider.resolve(
                &task_input,
                &selected_model,
                &plan_artifacts,
                &siblings,
                &prior_outputs,
            );

            eprintln!(
                "[orchestrate] Context tier={:?} sections={} tokens_est={} budget={}",
                resolved.tier,
                resolved.sections.len(),
                resolved.total_tokens_estimate,
                resolved.budget_tokens,
            );

            // ── Attribution-based demotion ───────────────────────────────
            let tier_str = format!("{:?}", resolved.tier);
            resolved.sections.retain(|cs| {
                use roko_compose::ContextSource;
                let source_type = match &cs.source {
                    ContextSource::InlineFile { .. } => "file",
                    ContextSource::SymbolSignature { .. } => "symbol",
                    ContextSource::AntiPattern => "anti_pattern",
                    ContextSource::Verification => "verification",
                    ContextSource::TaskBrief => "task_brief",
                    ContextSource::PriorTaskOutput { .. } => "prior_output",
                    ContextSource::PlanBrief => "plan_brief",
                    ContextSource::ResearchMemo => "research_memo",
                    ContextSource::Invariants => "invariants",
                    ContextSource::CrossPlanContext => "cross_plan",
                    ContextSource::PrdExtract => "prd_extract",
                    ContextSource::Decomposition => "decomposition",
                    ContextSource::SiblingTasks => "sibling_tasks",
                };
                let rate = self.attribution_tracker.ref_rate(&tier_str, source_type);
                let demoted = self
                    .attribution_tracker
                    .should_demote(&tier_str, source_type);
                if demoted {
                    eprintln!("[context] {source_type}: demoted (ref_rate={rate:.2})");
                } else {
                    eprintln!("[context] {source_type}: included (ref_rate={rate:.2})");
                }
                !demoted
            });

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

            resolved.into_prompt_sections()
        } else {
            Vec::new()
        };

        let claude_tools_csv = claude_tool_allowlist_with(role, self.tool_registry.as_deref());

        // ── Adaptive format selection via bandit ─────────────────────
        let tool_count = claude_tools_csv
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
        eprintln!(
            "[orchestrate] format_bandit: model={selected_model} role={role:?} tools={tool_count} → {selected_format:?}",
        );

        let role_instruction = system_prompt_override
            .unwrap_or_else(|| build_system_prompt(role, plan_id, task, &claude_tools_csv));
        let role_section = PromptSection::new("role", &role_instruction)
            .with_priority(SectionPriority::Critical)
            .with_placement(Placement::Start)
            .into_signal()
            .map_err(|e| anyhow!("role section: {e}"))?;
        let task_section = PromptSection::new("task", &task_text)
            .with_priority(SectionPriority::Critical)
            .with_placement(Placement::End)
            .into_signal()
            .map_err(|e| anyhow!("task section: {e}"))?;

        // Combine: role (Critical/Start) + context sections (tiered) + learned context + task (Critical/End)
        let mut sections = vec![role_section];
        for cs in context_sections {
            sections.push(
                cs.into_signal()
                    .map_err(|e| anyhow!("context section: {e}"))?,
            );
        }

        // ── Inject learned knowledge (skills, playbook rules, patterns) ──
        let learned = self.build_learned_context(role, &task_text);
        if !learned.text.is_empty() {
            let learned_section = PromptSection::new("learned-context", &learned.text)
                .with_priority(SectionPriority::Normal)
                .with_placement(Placement::Middle)
                .into_signal()
                .map_err(|e| anyhow!("learned-context section: {e}"))?;
            sections.push(learned_section);
            eprintln!(
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

        sections.push(task_section);

        // ── Tool manifest for non-CLI agents ──────────────────────────
        // Claude CLI gets tools via `--allowedTools` flag; for ExecAgent
        // and other backends, inject the role-filtered tool list into the
        // prompt so the agent knows which tools are available.
        let is_exec_agent = self.config.agent.command != "claude";
        if is_exec_agent {
            let tool_manifest = self.build_tool_manifest(role);
            if !tool_manifest.is_empty() {
                let tool_section = PromptSection::new("available-tools", &tool_manifest)
                    .with_priority(SectionPriority::Normal)
                    .with_placement(Placement::Middle)
                    .into_signal()
                    .map_err(|e| anyhow!("tool manifest section: {e}"))?;
                sections.push(tool_section);
            }
        }

        let composer = PromptComposer::new();
        let prompt = composer
            .compose(
                &sections,
                &Budget::tokens(self.config.prompt.token_budget),
                &NoOpScorer,
                &ctx,
            )
            .map_err(|e| anyhow!("compose: {e}"))?;

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
        let result: AgentResult = if self.config.agent.command == "claude" {
            let task_read_args = task_def
                .as_ref()
                .map(task_read_cli_args)
                .unwrap_or_default();
            let mut agent =
                ClaudeCliAgent::new(&self.config.agent.command, &exec_dir, &selected_model)
                    .with_timeout_ms(self.effective_task_timeout_ms(task_def.as_ref()))
                    .with_bare_mode(self.config.agent.bare_mode)
                    .with_effort(self.config.agent.effort.clone())
                    .with_system_prompt(role_instruction.clone())
                    .with_extra_args(task_read_args)
                    .with_tools(claude_tools_csv)
                    .with_settings_json(roko_agent::claude_cli_agent::build_settings_json())
                    .with_dangerously_skip_permissions(claude_skip_permissions_for_role(role))
                    .with_optional_resume(self.claude_resume_session.clone())
                    .with_extra_args(self.config.agent.args.clone());
            if let Some(mcp_path) = &self.config.agent.mcp_config {
                agent = agent.with_mcp_config(mcp_path);
            }
            if let Some(fallback_model) = &self.config.agent.fallback_model {
                agent = agent.with_fallback_model(fallback_model.clone());
            }
            for (k, v) in &self.config.agent.env {
                agent = agent.with_env_var(k, v);
            }
            agent.run(&prompt, &ctx).await
        } else {
            let mut agent =
                ExecAgent::new(&self.config.agent.command, self.config.agent.args.clone())
                    .with_timeout_ms(self.config.agent.timeout_ms);
            for (k, v) in &self.config.agent.env {
                agent = agent.with_env_var(k, v);
            }
            agent.run(&prompt, &ctx).await
        };

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

        if !result.success {
            return Err(anyhow!(
                "agent returned failure for plan={plan_id} task={task}"
            ));
        }

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
            }

            let ref_rate = if total > 0 {
                (referenced as f64) / (total as f64)
            } else {
                0.0
            };

            eprintln!(
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
        }

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

        // ── Run per-task verification pipeline ──────────────────────
        if let Some(ref td) = task_def {
            if let Err((task_id, _phase, command, _error_output)) =
                self.run_verify_steps(&td.id, &td.verify, &exec_dir).await
            {
                let msg = td
                    .verify
                    .iter()
                    .find(|s| s.command == command)
                    .and_then(|s| s.fail_msg.as_deref())
                    .unwrap_or("verification failed");
                return Err(anyhow!(
                    "verify failed for {}: {} — {}",
                    task_id,
                    command,
                    msg
                ));
            }
            self.verify_declared_write_files(plan_id, &td.id, &td.files, &exec_dir)
                .await?;
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

        eprintln!(
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
                    eprintln!("  ✅ [{}] {}", step.phase, step.command);
                }
                Ok(o) => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    let msg = step.fail_msg.as_deref().unwrap_or("verification failed");
                    eprintln!(
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
                    eprintln!("  ❌ [{}] {} — spawn error: {e}", step.phase, step.command);
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
        let exec_dir = self.plan_exec_dir(plan_id).await;
        let payload = GatePayload::in_dir(&exec_dir).with_label(format!("{plan_id}:rung-{rung}"));
        let payload_sig = Signal::builder(Kind::Task)
            .body(Body::from_json(&payload)?)
            .provenance(Provenance::trusted("orchestrate"))
            .tag("plan_id", plan_id)
            .tag("rung", rung.to_string())
            .build();

        let verdicts = Self::run_gate_rung(&payload_sig, rung).await;

        // Persist verdicts.
        let substrate_dir = self.workdir.join(".roko");
        if let Ok(substrate) = FileSubstrate::open(&substrate_dir).await {
            for verdict in &verdicts {
                let sig = payload_sig
                    .derive(
                        Kind::GateVerdict,
                        Body::from_json(verdict)
                            .unwrap_or_else(|_| Body::text(format!("{verdict:?}"))),
                    )
                    .provenance(Provenance::trusted("orchestrate"))
                    .tag("gate", &verdict.gate)
                    .tag("passed", verdict.passed.to_string())
                    .build();
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
            .map_or_else(|| format!("roko/{plan_id}"), |h| h.branch);
        let output = tokio::process::Command::new("git")
            .args(["merge", "--no-ff", &branch_name])
            .current_dir(&self.workdir)
            .output()
            .await
            .context("git merge")?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(anyhow!("merge failed: {stderr}"))
        }
    }

    async fn run_post_merge_follow_up(&self, plan_id: &str) -> Result<bool> {
        let payload =
            GatePayload::in_dir(&self.workdir).with_label(format!("{plan_id}:post-merge"));
        let payload_sig = Signal::builder(Kind::Task)
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

    async fn plan_exec_dir(&self, plan_id: &str) -> PathBuf {
        self.clear_stale_worktree_locks().await;
        match self.worktrees.ensure_for_plan(plan_id).await {
            Ok(handle) => handle.path,
            Err(err) => {
                eprintln!(
                    "[orchestrate] worktree unavailable for plan={plan_id}, using repo root: {err}"
                );
                self.workdir.clone()
            }
        }
    }

    /// Create (or fall back to plan-level) worktree for an individual task
    /// within a plan, so parallel tasks get isolated working directories.
    async fn task_exec_dir(&self, plan_id: &str, task_id: &str) -> Result<PathBuf> {
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

    async fn run_gate_rung(payload_sig: &Signal, rung: u32) -> Vec<Verdict> {
        let ctx = Context::now();
        // Rung 0 = compile, rung 1 = test, rung 2 = clippy, rung 3+ = all.
        match rung {
            0 => {
                let gate = CompileGate::cargo();
                vec![gate.verify(payload_sig, &ctx).await]
            }
            1 => {
                let gate = TestGate::cargo();
                vec![gate.verify(payload_sig, &ctx).await]
            }
            2 => {
                let gate = ClippyGate::cargo();
                vec![gate.verify(payload_sig, &ctx).await]
            }
            _ => {
                let c = CompileGate::cargo();
                let t = TestGate::cargo();
                let cl = ClippyGate::cargo();
                vec![
                    c.verify(payload_sig, &ctx).await,
                    t.verify(payload_sig, &ctx).await,
                    cl.verify(payload_sig, &ctx).await,
                ]
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
            eprintln!("[orchestrate] {plan_id}: no task verify steps declared");
            return Ok(());
        }

        let exec_dir = self.plan_exec_dir(plan_id).await;
        eprintln!(
            "[orchestrate] Running plan verify for {plan_id} across {} task(s)",
            steps_to_run.len()
        );

        for (task_id, verify_steps) in steps_to_run {
            if let Err(err) = self.run_verify_steps(&task_id, &verify_steps, &exec_dir).await {
                return Err(err);
            }
        }

        Ok(())
    }

    /// Remove stale git worktree locks before creating or using worktrees.
    async fn clear_stale_worktree_locks(&self) {
        match self.worktrees.clear_stale_locks() {
            Ok(cleared) if !cleared.is_empty() => {
                eprintln!(
                    "[orchestrate] cleared {} stale worktree lock(s)",
                    cleared.len()
                );
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("[orchestrate] stale lock cleanup failed: {e}");
            }
        }
    }

    /// Build a review prompt using the ReviewerTemplate with available context.
    async fn build_review_prompt(&self, plan_id: &str) -> String {
        use roko_compose::templates::reviewer::{Reviewer, ReviewerInput, ReviewerTemplate};
        use roko_compose::templates::{PlanSlice, RolePromptTemplate};

        let plan_dir = self.workdir.join("plans").join(plan_id);

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
        let exec_dir = self.plan_exec_dir(plan_id).await;
        let files_changed = tokio::process::Command::new("git")
            .args(["diff", "--name-only", "HEAD"])
            .current_dir(&exec_dir)
            .output()
            .await
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.lines().map(String::from).collect::<Vec<_>>())
            .unwrap_or_default();

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

        let plan_dir = self.workdir.join("plans").join(plan_id);
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
    ) -> (Vec<String>, Vec<roko_compose::templates::scribe::FileSnippet>) {
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
    fn emit_agent_turn_signal(&mut self, output: &Signal) {
        let body = match &output.body {
            Body::Text(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    return;
                }
                Body::text(trimmed)
            }
            Body::Json(value) => Body::Json(value.clone()),
            Body::Bytes(bytes) => {
                if bytes.is_empty() {
                    return;
                }
                Body::Bytes(bytes.clone())
            }
            Body::Empty => return,
        };

        self.conductor_signals
            .push(Signal::builder(output.kind.clone()).body(body).build());
    }

    /// Construct and persist an [`AgentEfficiencyEvent`] for one agent turn.
    async fn emit_efficiency_event(
        &mut self,
        plan_id: &str,
        task_id: &str,
        role: &str,
        model: &str,
        result: &AgentResult,
        wall_ms: u64,
        success: bool,
    ) {
        let event = AgentEfficiencyEvent {
            agent_id: result.output.id.to_string(),
            role: role.to_string(),
            backend: "claude".to_string(),
            model: model.to_string(),
            plan_id: plan_id.to_string(),
            task_id: task_id.to_string(),
            input_tokens: u64::from(result.usage.input_tokens),
            output_tokens: u64::from(result.usage.output_tokens),
            cache_read_tokens: u64::from(result.usage.cache_read_tokens),
            cache_write_tokens: u64::from(result.usage.cache_create_tokens),
            cost_usd: f64::from(result.usage.cost_usd),
            cost_usd_without_cache: f64::from(result.usage.cost_usd), // No cache discount info available.
            prompt_sections: Vec::new(),
            total_prompt_tokens: u64::from(result.usage.input_tokens),
            system_prompt_tokens: 0, // Not tracked at this level.
            tools_available: 0,
            tools_used: 0,
            tool_calls: Vec::new(),
            wall_time_ms: wall_ms,
            time_to_first_token_ms: 0,
            was_warm_start: false,
            iteration: 1,
            gate_passed: success,
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        tracing::info!(
            plan_id = %plan_id,
            task_id = %task_id,
            role = %role,
            model = %model,
            cost_usd = event.cost_usd,
            wall_ms = wall_ms,
            success = success,
            "agent efficiency event"
        );

        self.efficiency_events.push(event.clone());

        if let Err(e) = self.learning.append_efficiency_event(&event).await {
            tracing::warn!("failed to persist efficiency event: {e}");
        }
    }

    /// Build a tool manifest string for non-CLI agent backends.
    ///
    /// Uses `DynamicToolRegistry` (which includes MCP tools) if available,
    /// falling back to `StaticToolRegistry`. The result is a human-readable
    /// list of tool names and descriptions suitable for injection into a
    /// system prompt.
    fn build_tool_manifest(&self, role: AgentRole) -> String {
        use roko_core::tool::ToolRegistry;

        let tools: Vec<roko_core::tool::ToolDef> = if let Some(ref registry) = self.tool_registry {
            registry.for_role(role).into_iter().cloned().collect()
        } else {
            let static_reg = StaticToolRegistry::new();
            static_reg.for_role(role).into_iter().cloned().collect()
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

        Ok(parse_git_status_changed_files(&String::from_utf8_lossy(&output.stdout)))
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

fn is_meaningful_output(output: &Signal) -> bool {
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

fn now_unix_ms_i64() -> i64 {
    #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0_i64, |d| d.as_millis() as i64)
    }
}

fn build_system_prompt(role: AgentRole, plan_id: &str, task: &str, tools_csv: &str) -> String {
    RoleSystemPromptSpec::new(
        role,
        TaskContext::new(task)
            .with_plan_id(plan_id)
            .with_workspace("roko-cli orchestration"),
        tools_csv,
    )
    .build()
}

impl PlanRunner {
    /// Build the strategist system prompt for the Enriching phase.
    ///
    /// This assembles the same 6-layer system prompt as other agent dispatches,
    /// but injects the plan's task context and inline read_files content so the
    /// strategist sees the full enrichment surface before dispatch.
    fn build_enrichment_system_prompt(&self, plan_id: &str) -> String {
        let plan_dir = self.workdir.join("plans").join(plan_id);
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

        let tools_csv = claude_tool_allowlist_with(AgentRole::Strategist, self.tool_registry.as_deref());
        RoleSystemPromptSpec::new(
            AgentRole::Strategist,
            TaskContext::new(format!("Enrich plan {plan_id} before agent dispatch"))
                .with_plan_id(plan_id)
                .with_workspace(plan_dir.display().to_string())
                .with_domain_notes(context_summary),
            tools_csv,
        )
        .with_extra_conventions(
            "Treat enrichment as a pre-dispatch analysis step. Preserve task context, read_files, and dependency ordering so later agent turns receive accurate context.",
        )
        .add_anti_pattern(
            "Do not invent file contents, dependencies, or task requirements that are not present in the plan context.",
        )
        .add_anti_pattern(
            "Do not skip read_files: if a task declares context files, they must be reflected in the enrichment summary.",
        )
        .build()
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
                out.push_str(&format!(
                    "- [{}] {}\n",
                    step.phase,
                    step.command
                ));
            }
        }
    }

    out
}

fn significant_terms(text: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "the", "and", "for", "with", "from", "into", "that", "this", "task", "plan", "should",
        "must", "have", "has", "are", "was", "were", "will", "would", "could", "can", "done",
        "make", "build", "update", "implement", "review", "please", "then", "than", "when",
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
            if let Some(name) = std::path::Path::new(file).file_name().and_then(|n| n.to_str()) {
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

/// Persist a task's output summary so downstream tasks can reference it.
fn save_task_output(workdir: &Path, task_id: &str, output: &str) {
    let output_dir = workdir.join(".roko").join("task-outputs");
    let _ = std::fs::create_dir_all(&output_dir);
    let output_path = output_dir.join(format!("{task_id}.txt"));
    let summary = truncate_output(output);
    let _ = std::fs::write(output_path, summary);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
}
