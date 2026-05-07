//! Core event loop — drives the executor via `tokio::select!` over agent
//! events, gate completions, executor ticks, periodic flushes, and
//! cancellation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::state_hub::StateHub;
use anyhow::{Context, Result};
use roko_core::RuntimeEvent;
use roko_core::agent::ModelSpec;
use roko_core::config::GatesConfig;
use roko_core::defaults::{DEFAULT_AGENT_TURN_LIMIT, DEFAULT_RUNNER_MAX_CONCURRENT_PLANS};
// TimeoutConfig-derived helpers: agent_dispatch_timeout, plan_total_timeout,
// llm_call_timeout, gate_timeout — see below.
use roko_core::runtime_event::WorkflowOutcome as RuntimeWorkflowOutcome;
use roko_core::{AgentRole, ContentHash, PhaseKind, PlanPhase};
use roko_daimon::{
    AffectEngine as _, AffectEvent, DispatchParams, SomaticSignal, StrategyCoordinates,
    TaskStrategyObservation,
};
use roko_fs::RokoLayout;
use roko_gate::{PlanComplexity, classify_gate_failure, render_failure_classification};
use roko_orchestrator::{
    ExecutorAction, ExecutorConfig, ExecutorEvent, ExecutorSnapshot, GateResult, MergeQueue,
    MergeRequest, OrchestratorSnapshot, ParallelExecutor, PlanState as OrcPlanState,
    RecoveryEngine, TransitionError,
};
use roko_runtime::{HttpEventSink, RunLedger, WorkflowConfig};
use tokio::sync::mpsc;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::dispatch::model_routing::tier_to_complexity;
use crate::dispatch::{
    AgentDispatchRequest, DispatchContext, GateFeedback as DispatchGateFeedback, PromptCache,
    PromptDiagnostics, ResolvedAgentRuntime, SharedAgentFactory,
};
use crate::inline::DiffEntry;
use crate::knowledge_helpers::{build_knowledge_routing_advice, neuro_prompt_task_category};
use crate::task_helpers::task_target_crates;
use crate::task_parser::TaskDef;
use roko_learn::playbook::PlaybookStore;
use roko_learn::post_gate_reflection::{PostGateReflectionStore, ReflectionGateOutcome};
use roko_learn::section_outcome::{
    SECTION_OUTCOME_SCHEMA_VERSION, SectionOutcomeRecord, SectionOutcomeStatus, SectionOutcomeStore,
};
use roko_neuro::KnowledgeStore;

use super::agent_events::handle_agent_event;
use super::agent_stream::{AgentHandle, AgentSpawnConfig};
use super::gate_dispatch;
use super::merge::{MergeDispatch, PlanMerger, PlanMergerConfig};
use super::output_sink::RunOutputSink;
use super::persist::{self, GateThresholds, PersistPaths};
use super::plan_loader::Plan;
use super::snapshot_writer::{SnapshotPayload, SnapshotWriter};
use super::state::RunState;
use super::tui_bridge::TuiBridge;
use super::types::{
    AgentCompletionSummary, AgentDispatchOutcome, AgentEvent, GateCompletion, GateCompletionKind,
    GateVerdictSummary, PlanOutcome, PlanRunSummary, PromptAssemblyDiagnostics, ResumeMarker,
    ResumeOutcome, RetryAction, RunConfig, RunOutcome, RunTotals, RunnerEvent, RunnerFailureKind,
    TaskAttemptOutcome, TaskAttemptRef, TaskAttemptStatus, effective_plan_timeout_secs,
};

// ─── RunReport ──────────────────────────────────────────────────────────

/// Summary of a completed run.
#[derive(Debug, Clone)]
pub struct RunReport {
    pub plans: Vec<PlanReport>,
    pub total_tasks: usize,
    pub tasks_completed: usize,
    pub tasks_failed: usize,
    pub total_cost_usd: f64,
    pub total_tokens_in: u64,
    pub total_tokens_out: u64,
    pub total_agent_calls: usize,
    pub duration: Duration,
    /// Per-task failure reasons keyed by "plan_id:task_id".
    pub failure_reasons: HashMap<String, String>,
    /// Per-task cost breakdown.
    pub task_costs: Vec<TaskCostReport>,
}

/// Per-task cost report for the RunLedger.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskCostReport {
    pub plan_id: String,
    pub task_id: String,
    pub model: String,
    pub provider: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    pub agent_calls: u32,
    pub outcome: String,
}

/// Per-plan report.
#[derive(Debug, Clone)]
pub struct PlanReport {
    pub plan_id: String,
    pub completed: bool,
    pub tasks_total: usize,
    pub tasks_completed: usize,
    pub tasks_failed: usize,
    pub gate_results: Vec<GateResult>,
}

impl RunReport {
    pub fn all_succeeded(&self) -> bool {
        self.tasks_failed == 0 && self.plans.iter().all(|p| p.completed)
    }
}

fn duration_secs(duration: Duration) -> u64 {
    duration.as_secs().max(1)
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis())
        .unwrap_or(u64::MAX)
        .max(1)
}

/// Resolve agent dispatch wall-clock timeout from `TimeoutConfig` (preferred)
/// or the legacy `RunConfig.timeout_secs` scalar.
pub(crate) fn agent_dispatch_timeout(config: &RunConfig) -> Duration {
    config.roko_config.as_deref().map_or_else(
        || Duration::from_secs(config.timeout_secs),
        |cfg| cfg.timeouts.agent_dispatch(),
    )
}

/// Resolve plan-level wall-clock timeout from `TimeoutConfig` (preferred)
/// or the legacy `RunConfig.plan_timeout_secs` scalar.
pub(crate) fn plan_total_timeout(config: &RunConfig) -> Duration {
    config.roko_config.as_deref().map_or_else(
        || Duration::from_secs(config.plan_timeout_secs),
        |cfg| Duration::from_secs(effective_plan_timeout_secs(cfg)),
    )
}

/// Resolve LLM call timeout from `TimeoutConfig`.
pub(crate) fn llm_call_timeout(config: &RunConfig) -> Duration {
    config.roko_config.as_deref().map_or_else(
        || roko_core::config::TimeoutConfig::default().llm_call(),
        |cfg| cfg.timeouts.llm_call(),
    )
}

/// Resolve gate rung timeout from `TimeoutConfig`:
/// - Compile rung -> `gate_compile()`
/// - Lint rung -> `gate_clippy()`
/// - all other rungs -> `gate_test()`
pub(crate) fn gate_timeout(config: &RunConfig, rung: u32) -> Duration {
    use roko_gate::rung_selector::Rung;
    config.roko_config.as_deref().map_or_else(
        || Duration::from_secs(config.timeout_secs),
        |cfg| {
            if rung == Rung::Compile.as_index() {
                cfg.timeouts.gate_compile()
            } else if rung == Rung::Lint.as_index() {
                cfg.timeouts.gate_clippy()
            } else {
                cfg.timeouts.gate_test()
            }
        },
    )
}

/// Resolve HTTP request timeout from `TimeoutConfig`.
pub(crate) fn http_request_timeout(config: &RunConfig) -> Duration {
    config.roko_config.as_deref().map_or_else(
        || roko_core::config::TimeoutConfig::default().http_request(),
        |cfg| cfg.timeouts.http_request(),
    )
}

/// Resolve health check timeout from `TimeoutConfig`.
pub(crate) fn health_check_timeout(config: &RunConfig) -> Duration {
    config.roko_config.as_deref().map_or_else(
        || roko_core::config::TimeoutConfig::default().health_check(),
        |cfg| cfg.timeouts.health_check(),
    )
}

// ─── RunContext ──────────────────────────────────────────────────────────

/// Shared context for the dispatch loop, replacing 11 loose parameters.
struct RunContext<'a> {
    executor: &'a mut ParallelExecutor,
    task_index: &'a HashMap<String, HashMap<String, TaskDef>>,
    skip_enrichment: &'a HashMap<String, bool>,
    config: &'a RunConfig,
    sink: &'a dyn RunOutputSink,
    tui: &'a TuiBridge,
    state: &'a mut RunState,
    active_agent_tasks: &'a mut HashMap<String, String>,
    agent_handles: &'a mut HashMap<String, AgentHandle>,
    agent_tx: &'a mpsc::Sender<AgentEvent>,
    gate_tx: &'a mpsc::Sender<GateCompletion>,
    fatal_tx: mpsc::Sender<AgentEvent>,
    paths: &'a PersistPaths,
    merge_queue: &'a MergeQueue,
    gate_thresholds: &'a GateThresholds,
    snapshot_writer: &'a SnapshotWriter,
    prompt_cache: &'a Arc<PromptCache>,
    factory: &'a SharedAgentFactory,
    gate_sem: Arc<tokio::sync::Semaphore>,
    /// Prompt section diagnostics per attempt key — populated at dispatch,
    /// consumed on gate completion to build SectionOutcomeRecords.
    section_diagnostics: &'a mut HashMap<String, PromptDiagnostics>,
    /// Playbook IDs per attempt key — populated at dispatch, consumed on gate
    /// terminal to call `PlaybookStore::record_outcome`.
    task_playbook_ids: &'a mut HashMap<String, Vec<String>>,
}

// Result of dispatching one executor action. Most actions are internal phase
// advances; the runner ledger only records an agent start when a runtime was
// actually launched for a concrete task.
enum ActionDispatchOutcome {
    Noop,
    Handled,
    AgentStarted { plan_id: String, task_id: String },
}

// ─── Main Entry Point ───────────────────────────────────────────────────

/// Run all plans to completion (or cancellation).
pub async fn run(
    plans: Vec<Plan>,
    config: &RunConfig,
    state_hub: &StateHub,
    cancel: CancellationToken,
) -> Result<RunReport> {
    // ── Ensure effective RokoConfig is available ─────────────────────────
    //
    // The CLI bootstrap path (`commands/plan.rs`) already loads config via
    // `RokoBootstrap` / `load_config_unified` and passes it through
    // `RunConfig.roko_config`. This fallback covers secondary callers
    // (tests, integration shims) that construct `RunConfig` without a
    // pre-loaded config: we call the canonical core loader which performs
    // ancestor walk, global merge, and env var overrides — no ad-hoc
    // project-root resolution in the runner.
    let mut config = config.clone();
    if config.roko_config.is_none() {
        let loaded =
            roko_core::config::loader::load_config_unified(&config.workdir).with_context(|| {
                format!(
                    "load roko config for runner workdir {}",
                    config.workdir.display()
                )
            })?;
        config.roko_config = Some(Arc::new(loaded));
    }

    if config.http_event_sink.is_none() {
        config.http_event_sink = HttpEventSink::from_env();
    }

    let max_concurrent_tasks = config.max_concurrent_tasks.max(1);
    let task_timeout_secs = duration_secs(agent_dispatch_timeout(&config));

    let exec_config = ExecutorConfig {
        max_concurrent_plans: DEFAULT_RUNNER_MAX_CONCURRENT_PLANS,
        max_concurrent_tasks,
        max_auto_fix_iterations: config.max_retries,
        task_timeout_secs,
        ..Default::default()
    };
    let paths = PersistPaths::from_workdir(&config.workdir)?;
    let snapshot_writer = SnapshotWriter::new(4);
    persist::cleanup_orphaned_agents(&paths);
    let mut gate_thresholds = persist::load_gate_thresholds(&paths).unwrap_or_default();

    // Ensure knowledge store directory exists for episode ingestion.
    let neuro_dir = config.layout.neuro_dir();
    if let Err(err) = std::fs::create_dir_all(&neuro_dir) {
        warn!(error = %err, "failed to create neuro directory");
    }

    // ── Strict resume validation + JSONL recovery ─────────────────────────
    //
    // Run before any state file is reopened. The validator:
    // 1. Loads `.roko/state/run-state.json` if present.
    // 2. Verifies current task fingerprints against the prior snapshot
    //    unless `--force-resume` is set.
    // 3. Reports drifted completed tasks so the caller can re-queue
    //    them instead of aborting the resume.
    // 4. Truncates `episodes.jsonl`, `events.jsonl`, and
    //    `efficiency.jsonl` after their last validated line (recovers
    //    from partial-append corruption left by a prior crash).
    //
    // On `PlanMissing` / `UnsupportedSchema` the validator still
    // returns Err. We surface the failure and abort the run so the
    // operator can either edit the plan back into a known state or
    // discard the snapshot.
    // Try the unified state snapshot first; fall back to legacy run-state.json.
    let (prior_snapshot, loaded_gate_thresholds) = match persist::load_state_snapshot(&paths) {
        Ok(Some(unified)) => {
            info!(
                timestamp_ms = unified.timestamp_ms,
                "loaded state snapshot -- checksum valid"
            );
            let run_state: Option<persist::RunStateSnapshot> =
                match serde_json::from_str(&unified.run_state_json) {
                    Ok(rs) => Some(rs),
                    Err(err) => {
                        warn!(
                            error = %err,
                            "failed to parse run_state_json from unified snapshot; ignoring"
                        );
                        None
                    }
                };
            let loaded_gt: Option<GateThresholds> =
                match serde_json::from_str(&unified.gate_thresholds_json) {
                    Ok(gt) => Some(gt),
                    Err(err) => {
                        warn!(
                            error = %err,
                            "failed to parse gate_thresholds_json from unified snapshot; using file"
                        );
                        None
                    }
                };
            (run_state, loaded_gt)
        }
        Ok(None) => {
            // No unified snapshot -- try legacy run-state.json.
            match persist::load_run_state(&paths) {
                Ok(Some(snapshot)) => {
                    warn!("no state-snapshot.json found; falling back to legacy run-state.json");
                    (Some(snapshot), None)
                }
                Ok(None) => (None, None),
                Err(err) => {
                    warn!(
                        error = %err,
                        "failed to read prior run-state.json; continuing without seeded resume state"
                    );
                    (None, None)
                }
            }
        }
        Err(err) => {
            warn!(
                error = %err,
                "failed to load state snapshot; falling back to legacy run-state.json"
            );
            match persist::load_run_state(&paths) {
                Ok(snap) => (snap, None),
                Err(err2) => {
                    warn!(
                        error = %err2,
                        "failed to read legacy run-state.json; continuing without seeded resume state"
                    );
                    (None, None)
                }
            }
        }
    };
    // Override gate thresholds if we loaded them from the unified snapshot.
    if let Some(gt) = loaded_gate_thresholds {
        gate_thresholds = gt;
    }
    let resume_report = {
        let mut plan_map: HashMap<String, Vec<TaskDef>> = HashMap::new();
        for plan in &plans {
            plan_map.insert(plan.id.clone(), plan.tasks.tasks.clone());
        }
        let prior_fingerprints = prior_snapshot
            .as_ref()
            .map(|snapshot| snapshot.fingerprints.as_slice())
            .unwrap_or(&[]);
        match super::resume::prepare_resume_with_force(
            &paths,
            &plan_map,
            prior_fingerprints,
            config.force_resume,
        ) {
            Ok(report) => {
                if report.resumed && !config.force_resume {
                    info!(
                        prior_run_id = ?report.prior_run_id,
                        validated_tasks = report.validated_tasks,
                        "resume validated"
                    );
                }
                for f in &report.recovered_files {
                    use super::resume::JsonlRecoveryReport;
                    match &f.recovery {
                        JsonlRecoveryReport::Clean { .. } => {}
                        JsonlRecoveryReport::TruncatedTrailing {
                            truncated_bytes, ..
                        } => {
                            warn!(file = %f.path, truncated_bytes, "recovered partial JSONL");
                        }
                        JsonlRecoveryReport::DroppedInvalid { dropped_lines, .. } => {
                            warn!(file = %f.path, dropped_lines, "recovered malformed JSONL");
                        }
                    }
                }
                report
            }
            Err(err) => {
                return Err(anyhow::anyhow!("resume validation failed: {err}"));
            }
        }
    };

    // Verify checkpoint integrity when resuming an existing run.
    // A mismatch means the state files were modified outside of a clean
    // atomic write (e.g. partial crash, manual edit, cross-plan leakage).
    // This is non-fatal: we warn and continue so the run is not blocked,
    // but the operator is alerted to potential state inconsistency.
    if prior_snapshot.is_some() {
        let state_dir = paths.executor_json.parent().unwrap_or(&paths.executor_json);
        match persist::verify_checkpoint(state_dir) {
            Ok(true) => {
                debug!("state checkpoint verified — all files consistent");
            }
            Ok(false) => {
                warn!(
                    state_dir = %state_dir.display(),
                    "state checkpoint mismatch: one or more state files changed since last write \
                     (possible cross-plan leakage or crash mid-write)"
                );
            }
            Err(err) => {
                warn!(
                    error = %err,
                    "failed to verify state checkpoint; continuing without verification"
                );
            }
        }
    }

    // Prefer the embedded router snapshot over the file-backed router on resume.
    if let Some(router_json) = resume_report.cascade_router_json.as_deref() {
        if let Some(existing_router) = config.cascade_router.as_ref() {
            let model_slugs = existing_router.model_slugs().to_vec();
            match roko_learn::cascade_router::CascadeRouter::from_snapshot_json(
                router_json,
                model_slugs,
            ) {
                Ok(router) => {
                    info!("restored cascade router from run-state snapshot");
                    config.cascade_router = Some(Arc::new(router));
                }
                Err(err) => {
                    warn!(
                        error = %err,
                        "failed to restore cascade router from run-state snapshot; using file-based state"
                    );
                }
            }
        }
    }

    // All mutations to `config` are done; reborrow as shared reference so
    // downstream helpers that expect `&RunConfig` work without extra `&`.
    let config = &config;

    // Per-run gate semaphore — limits how many gate rungs execute concurrently.
    let gate_sem = Arc::new(tokio::sync::Semaphore::new(config.gate_concurrency.max(1)));

    // Build plan ID set for resume validation.
    let plan_ids: Vec<String> = plans.iter().map(|p| p.id.clone()).collect();

    // Only resume if snapshot exists AND its plans match the current run.
    let resume = load_executor(&paths, &exec_config, &plan_ids);
    let mut executor = resume.executor;
    let merge_queue = resume.merge_queue;

    // Index tasks by plan_id/task_id for lookup.
    let mut task_index: HashMap<String, HashMap<String, TaskDef>> = HashMap::new();
    let mut total_tasks = 0usize;

    for plan in &plans {
        // add_plan is a no-op if plan already exists (from snapshot).
        let orc_state = OrcPlanState::new(&plan.id);
        executor.add_plan(orc_state);

        let mut tasks_map = HashMap::new();
        for task in &plan.tasks.tasks {
            tasks_map.insert(task.id.clone(), task.clone());
            total_tasks += 1;
        }
        task_index.insert(plan.id.clone(), tasks_map);
    }

    // Channels.
    let (agent_tx, mut agent_rx) = mpsc::channel::<AgentEvent>(256);
    // Dynamic gate channel buffer: max_concurrent_tasks * 7 rungs, clamped to [32, 256].
    let gate_buffer = (config.max_concurrent_tasks * 7).max(32).min(256);
    let (gate_tx, mut gate_rx) = mpsc::channel::<GateCompletion>(gate_buffer);
    let sink = config.output_sink.as_ref();

    // -- Warm cargo cache -------------------------------------------------------
    // Run `cargo check --workspace` once before the main loop so that
    // subsequent per-task compile gates are incremental (2-5s vs 30-120s).
    if config.warm_cache {
        sink.warm_cache_started();
        let warm_start = std::time::Instant::now();
        let warm_result = tokio::process::Command::new("cargo")
            .args(["check", "--workspace", "--message-format=short"])
            .current_dir(&config.workdir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        let warm_ms = warm_start.elapsed().as_millis() as u64;
        match warm_result {
            Ok(status) if status.success() => {
                info!(warm_ms, "cargo cache warmed successfully");
                sink.warm_cache_completed(warm_ms);
            }
            Ok(status) => {
                warn!(
                    warm_ms,
                    exit_code = status.code().unwrap_or(-1),
                    "cargo cache warm failed (non-fatal)"
                );
            }
            Err(e) => {
                warn!(warm_ms, error = %e, "cargo cache warm failed (non-fatal)");
            }
        }
    }

    // Seed playbooks if the store is empty (bootstrap chicken-and-egg).
    seed_playbooks_if_empty(&config.layout).await;

    // Build prompt cache once — reused across all task dispatches.
    // Refreshed when stale (default 5 min) or after gate failures.
    let mut prompt_cache = Arc::new(PromptCache::load(&config.workdir));

    // Shared agent factory — expensive components (semaphores, MCP tools,
    // dispatcher, resolver) created once and reused for every task dispatch.
    let t_factory = Instant::now();
    let factory = SharedAgentFactory::new(
        config.roko_config.clone().unwrap_or_default(),
        config.mcp_config.as_ref(),
        config.cascade_router.clone(),
        Some(Arc::clone(&prompt_cache)),
    )
    .await;
    info!(
        factory_init_ms = t_factory.elapsed().as_millis() as u64,
        "agent factory initialized"
    );

    // State and TUI bridge.
    let tui = TuiBridge::new(state_hub.sender());
    let mut state = RunState::new(total_tasks);
    let mut dream_completion_pending = false;

    // Run ledger — optional enhancement for tracking task starts, completions,
    // and gate outcomes in a typed JSONL file at `.roko/state/run-ledger.jsonl`.
    // Initialization is infallible; we keep it as Option so downstream code
    // gracefully no-ops if we ever make init fallible.
    let mut run_ledger: Option<RunLedger> = {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let prompt_summary = plans
            .first()
            .map(|p| p.id.clone())
            .unwrap_or_else(|| "multi-plan".to_string());
        let ledger = RunLedger::new(
            state.run_id(),
            prompt_summary,
            WorkflowConfig::default(),
            now_ms,
        );
        debug!("run ledger initialized for run {}", state.run_id());
        Some(ledger)
    };

    // Compute task fingerprints once at startup so every subsequent
    // `save_snapshot` writes them into `run-state.json` for the strict
    // resume validator to consume on the next run.
    state.task_fingerprints = plans
        .iter()
        .flat_map(|plan| {
            plan.tasks
                .tasks
                .iter()
                .map(move |task| persist::TaskDefFingerprint::from_task(task, &plan.id))
        })
        .collect();

    if matches!(resume.marker.outcome, ResumeOutcome::Resumed) {
        if let Some(snapshot) = prior_snapshot.as_ref() {
            restore_state_from_resume_snapshot(
                &mut state,
                snapshot,
                &task_index,
                &resume_report.drifted_tasks,
            );
        }
    }

    let mut agent_handles: HashMap<String, AgentHandle> = HashMap::new();
    let mut active_agent_tasks: HashMap<String, String> = HashMap::new();
    let mut feedback_tasks: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();

    // Track prompt section diagnostics per attempt so gate completions can
    // build SectionOutcomeRecords joining section presence to pass/fail.
    let mut section_diagnostics: HashMap<String, PromptDiagnostics> = HashMap::new();

    // PlaybookStore for recording gate outcomes back to playbook success/failure
    // counters. This closes the feedback loop so adaptive playbook selection can
    // learn which playbooks correlate with task success.
    let playbook_store = PlaybookStore::new(config.layout.playbooks_dir());

    // Playbook IDs injected per task attempt (keyed by attempt key
    // "{plan_id}:{task_id}:{attempt}"). Populated at dispatch time from prompt
    // diagnostics so the gate terminal handler can call record_outcome.
    let mut task_playbook_ids: HashMap<String, Vec<String>> = HashMap::new();

    // skip_enrichment is a plan-level DAG phase control: when true, the plan
    // transitions directly from "started" to "implementing", bypassing the
    // "enriching" executor phase. This is NOT an LLM pre-processing pipeline
    // -- the legacy orchestrate.rs enrichment pipeline (multi-step LLM
    // pre-dispatch enrichment) was not ported to v2 because the v2 prompt
    // builder already handles context assembly via PromptContext sections
    // (workspace_context, cfactor_context, knowledge, playbooks, etc.).
    let skip_enrichment: HashMap<String, bool> = plans
        .iter()
        .map(|p| (p.id.clone(), p.tasks.meta.skip_enrichment))
        .collect();

    let mut tick_interval = interval(Duration::from_millis(100));
    let mut flush_interval = interval(Duration::from_secs(2));
    let plan_timeout_duration = plan_total_timeout(&config);
    let agent_timeout_duration = agent_dispatch_timeout(&config);
    let plan_deadline = tokio::time::Instant::now() + plan_timeout_duration;
    let plan_timeout = tokio::time::sleep_until(plan_deadline);
    tokio::pin!(plan_timeout);

    info!(
        plan_count = plans.len(),
        total_tasks,
        model = %config.model,
        max_concurrent = config.max_concurrent_tasks,
        max_retries = config.max_retries,
        max_gate_rung = config.max_gate_rung,
        max_plan_usd = config.max_plan_usd,
        max_turn_usd = config.max_turn_usd,
        timeout_secs = duration_secs(agent_timeout_duration),
        plan_timeout_secs = duration_secs(plan_timeout_duration),
        clippy_enabled = config.clippy_enabled,
        skip_tests = config.skip_tests,
        output_sink = ?config.output_sink,
        has_mcp_config = config.mcp_config.is_some(),
        has_cascade_router = config.cascade_router.is_some(),
        "starting runner v2 event loop"
    );
    let run_id = state.run_id().to_string();
    emit_runner_event(
        &paths,
        &mut state,
        &tui,
        config,
        RunnerEvent::resume_marker(&run_id, resume.marker.clone()),
    );
    emit_runner_event(
        &paths,
        &mut state,
        &tui,
        config,
        RunnerEvent::run_started(
            &run_id,
            plan_ids.clone(),
            total_tasks,
            matches!(resume.marker.outcome, ResumeOutcome::Resumed),
            config.resume_session.clone(),
        ),
    );

    // ─── Phase 0: Initialize subsystems ─────────────────────────────
    // Extension chain init (no-op with empty chain).
    if let Some(ext_chain) = &config.extension_chain {
        let mut chain = ext_chain.lock().await;
        let errors = chain.init_all().await;
        for (name, err) in &errors {
            warn!(extension = %name, error = %err, "extension init failed");
        }
        if !errors.is_empty() {
            info!(
                failed = errors.len(),
                "extension chain init completed with errors"
            );
        }
    }

    // Register MCP connectors in the connector registry.
    if let Some(registry) = &config.connector_registry {
        if let Some(mcp_path) = &config.mcp_config {
            if let Ok(contents) = std::fs::read_to_string(mcp_path) {
                if let Ok(mcp_json) = serde_json::from_str::<serde_json::Value>(&contents) {
                    if let Some(servers) = mcp_json.get("mcpServers").and_then(|s| s.as_object()) {
                        if let Ok(mut reg) = registry.lock() {
                            for name in servers.keys() {
                                reg.register(roko_core::ConnectorInfo {
                                    name: name.clone(),
                                    kind: roko_core::ConnectorKind::Mcp,
                                    health: roko_core::ConnectorHealth {
                                        status: roko_core::ConnectorStatus::Connected,
                                        latency_ms: 0,
                                        last_check: chrono::Utc::now(),
                                    },
                                    created_at: chrono::Utc::now(),
                                    metadata: serde_json::Value::Null,
                                });
                            }
                            info!(count = servers.len(), "registered MCP connectors");
                        }
                    }
                }
            }
        }
    }

    // ── Spawn the learning event subscriber ──────────────────────────
    // Background task that consumes gate/turn events and feeds them into
    // CalibrationPolicy, VerdictScorer, ProviderHealth, CascadeRouter,
    // CostsDb, and the efficiency JSONL log.
    let learning_event_bus = roko_learn::events::EventBus::new(256);
    let learning_subscriber_rx = learning_event_bus.subscribe();
    let subscriber_model_slugs: Vec<String> = config
        .cascade_router
        .as_ref()
        .map(|r| r.model_slugs().to_vec())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| vec![config.model.clone()]);
    let learning_subscriber_handle = {
        use std::sync::Mutex;
        let health = Arc::new(roko_learn::provider_health::ProviderHealthRegistry::new());
        let latency = Arc::new(roko_learn::latency::LatencyRegistry::new());
        let router = Arc::new(roko_learn::cascade_router::CascadeRouter::new(
            subscriber_model_slugs,
        ));
        let anomaly_start_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let anomaly = Arc::new(Mutex::new(roko_learn::anomaly::AnomalyDetector::new(
            anomaly_start_ms,
        )));
        let costs = Arc::new(roko_learn::costs_db::CostsDb::new());
        let efficiency_path = config.layout.learn_dir().join("efficiency.jsonl");
        let router_persist_path = Some(config.layout.learn_dir().join("cascade-router.json"));
        tokio::spawn(roko_learn::event_subscriber::run_learning_subscriber(
            learning_subscriber_rx,
            health,
            latency,
            router,
            anomaly,
            costs,
            efficiency_path,
            router_persist_path,
        ))
    };

    let mut timed_out = false;

    loop {
        // Cancel-safety analysis:
        //   Branch 1 (agent_rx.recv): cancel-safe — mpsc::Receiver::recv drops no data.
        //   Branch 2 (gate_rx.recv):  cancel-safe — mpsc::Receiver::recv drops no data.
        //   Branch 3 (tick_interval): cancel-safe — Interval::tick is restartable.
        //   Branch 4 (flush_interval): cancel-safe — Interval::tick is restartable.
        //   Branch 5 (plan_timeout): cancel-safe — fixed deadline, no state lost.
        //   Branch 6 (cancel.cancelled): cancel-safe — CancellationToken is idempotent.
        tokio::select! {
            // ─── Branch 1: Agent events ─────────────────────────────
            Some(event) = agent_rx.recv() => {
                let is_turn_done = matches!(&event, AgentEvent::TurnCompleted { .. });
                let is_exited = matches!(&event, AgentEvent::Exited { .. });
                let turn_completed_before_event = state.agent_turn_completed;
                let turn_error = matches!(&event, AgentEvent::TurnCompleted { is_error: true, .. });

                handle_agent_event(&event, &mut state, &tui, sink);
                append_agent_event(&paths, &event, &state);
                publish_learning_agent_event(&learning_event_bus, &event, &state);
                if is_turn_done || is_exited {
                    active_agent_tasks.remove(&state.plan_id);
                }

                // ── Forward progress-relevant agent events to RuntimeEvent ──
                if let Some(http_sink) = config.http_event_sink.as_ref() {
                    match &event {
                        AgentEvent::ToolCall { name, .. } => {
                            http_sink.emit(RuntimeEvent::ToolCallStarted {
                                run_id: run_id.clone(),
                                agent_id: format!("{}/{}", state.plan_id, state.current_task),
                                tool: name.clone(),
                                iteration: 0,
                            });
                        }
                        AgentEvent::MessageDelta { text } => {
                            http_sink.emit(RuntimeEvent::AgentOutput {
                                run_id: run_id.clone(),
                                agent_id: format!("{}/{}", state.plan_id, state.current_task),
                                chunk: text.clone(),
                            });
                        }
                        _ => {}
                    }
                }

                // Per-turn budget observation. This event arrives after the
                // provider turn has already completed, so a successful turn
                // should still move on to gating; otherwise we throw away a
                // potentially valid patch after paying for it.
                if is_turn_done {
                    let max_turn = config.max_turn_usd;
                    if let AgentEvent::TurnCompleted { total_cost_usd, .. } = &event
                        && turn_exceeds_budget(*total_cost_usd, max_turn)
                    {
                        warn!(
                            task = %state.current_task,
                            turn_cost = total_cost_usd.unwrap_or_default(),
                            limit = max_turn,
                            "single turn exceeded per-turn budget limit -- continuing to gate completed work"
                        );
                    }
                }

                if is_turn_done {
                    if let AgentEvent::TurnCompleted {
                        session_id,
                        total_cost_usd,
                        num_turns,
                        is_error,
                    } = &event
                    {
                        let agent_id = format!("{}/{}", state.plan_id, state.current_task);
                        let outcome = if *is_error {
                            AgentDispatchOutcome::Failed
                        } else {
                            AgentDispatchOutcome::Completed
                        };
                        let attempt = state.current_attempt_ref();
                        let run_id = state.run_id().to_string();
                        emit_runner_event(
                            &paths,
                            &mut state,
                            &tui,
                            config,
                            RunnerEvent::agent_completed(
                                &run_id,
                                attempt,
                                &agent_id,
                                outcome,
                                AgentCompletionSummary {
                                    session_id: session_id.clone(),
                                    total_cost_usd: *total_cost_usd,
                                    turns: *num_turns,
                                    exit_code: None,
                                    message: (*is_error)
                                        .then(|| "agent reported an error result".to_string()),
                                },
                            ),
                        );
                    }

                    // Extension: post_inference hook.
                    let task_role = task_index
                        .get(state.plan_id.as_str())
                        .and_then(|tasks| tasks.get(state.current_task.as_str()))
                        .and_then(|t| t.role.as_deref())
                        .unwrap_or("implementer");
                    fire_post_inference_hook(
                        config,
                        &state.plan_id,
                        &state.current_task,
                        &state.agent_model,
                        task_role,
                        !turn_error,
                        state.cost_usd,
                        state.task_elapsed_ms(),
                        &tui,
                    )
                    .await;

                    let plan_id = state.plan_id.clone();
                    if !plan_id.is_empty() {
                        if turn_error {
                            let message = agent_failure_message(&state.agent_output)
                                .unwrap_or_else(|| "agent reported an error result".to_string());
                            fire_on_error_hook(config, &message, "agent_turn", &tui, &state.plan_id, &state.current_task).await;
                            handle_agent_failure(
                                &mut executor,
                                &task_index,
                                &mut state,
                                &paths,
                                &tui,
                                sink,
                                config,
                                message,
                            );
                        } else {
                            apply_agent_completion(&mut executor, &plan_id, &tui);
                        }
                        save_snapshot(config, &executor, &paths, &mut state, &merge_queue, &gate_thresholds, &snapshot_writer);
                    }
                }

                if is_exited {
                    let exit_code = if let Some(handle) = agent_handles.remove(&state.plan_id) {
                        let pid = handle.pid;
                        let code = handle.wait().await;
                        roko_agent::process::unregister_pid(pid);
                        code
                    } else if let AgentEvent::Exited { exit_code } = event {
                        exit_code
                    } else {
                        None
                    };

                    let plan_id = state.plan_id.clone();
                    if !turn_completed_before_event && !plan_id.is_empty() {
                        let agent_id = format!("{}/{}", state.plan_id, state.current_task);
                        if exit_code.unwrap_or(0) == 0 {
                            let attempt = state.current_attempt_ref();
                            let run_id = state.run_id().to_string();
                            emit_runner_event(
                                &paths,
                                &mut state,
                                &tui,
                                config,
                                RunnerEvent::agent_completed(
                                    &run_id,
                                    attempt,
                                    &agent_id,
                                    AgentDispatchOutcome::Exited,
                                    AgentCompletionSummary {
                                        exit_code,
                                        ..AgentCompletionSummary::default()
                                    },
                                ),
                            );
                            apply_agent_completion(&mut executor, &plan_id, &tui);
                        } else {
                            let message = format!(
                                "agent process exited unsuccessfully: exit_code={}",
                                exit_code.map_or_else(|| "unknown".into(), |code| code.to_string())
                            );
                            let attempt = state.current_attempt_ref();
                            let run_id = state.run_id().to_string();
                            emit_runner_event(
                                &paths,
                                &mut state,
                                &tui,
                                config,
                                RunnerEvent::agent_completed(
                                    &run_id,
                                    attempt,
                                    &agent_id,
                                    AgentDispatchOutcome::Failed,
                                    AgentCompletionSummary {
                                        exit_code,
                                        message: Some(message.clone()),
                                        ..AgentCompletionSummary::default()
                                    },
                                ),
                            );
                            handle_agent_failure(
                                &mut executor,
                                &task_index,
                                &mut state,
                                &paths,
                                &tui,
                                sink,
                                config,
                                message,
                            );
                        }
                    }

                    save_snapshot(config, &executor, &paths, &mut state, &merge_queue, &gate_thresholds, &snapshot_writer);
                }
            }

            // ─── Branch 2: Verify completions ─────────────────────────
            Some(completion) = gate_rx.recv() => {
                let effect_key = gate_effect_key(
                    &completion.plan_id,
                    &completion.task_id,
                    completion.rung,
                    completion.kind,
                );
                state.clear_gate_active(&effect_key);
                state.gate_output = completion.output.clone();
                let completion_attempt = TaskAttemptRef::new(
                    completion.plan_id.clone(),
                    completion.task_id.clone(),
                    state.iteration_for(&completion.plan_id, &completion.task_id),
                );

                for v in &completion.verdicts {
                    tui.gate_result(
                        &completion.plan_id,
                        &completion.task_id,
                        &v.gate_name,
                        v.passed,
                    );

                    // Emit gate verdict metric.
                    if let Some(ref metrics) = config.metrics {
                        let verdict_str = if v.passed { "pass" } else { "fail" };
                        let labels = roko_core::obs::metrics::LabelSet::from_pairs(&[
                            (roko_core::obs::schema::LABEL_GATE, &v.gate_name),
                            (roko_core::obs::schema::LABEL_VERDICT, verdict_str),
                        ]);
                        metrics
                            .register_counter(
                                roko_core::obs::schema::ROKO_GATE_VERDICTS_TOTAL,
                                "Verify verdicts, by gate and verdict",
                                labels,
                            )
                            .inc();
                    }
                }

                // Render gate verdicts through the output sink.
                if completion.kind == GateCompletionKind::Gate {
                    for v in &completion.verdicts {
                        sink.gate_result(
                            &completion.plan_id,
                            &completion.task_id,
                            &super::output_sink::GateResultSummary {
                                rung: completion.rung,
                                passed: v.passed,
                                gate_name: v.gate_name.clone(),
                                summary: v.summary.clone(),
                                duration_ms: completion.duration_ms,
                            },
                        );
                    }
                }
                if completion.kind == GateCompletionKind::Gate {
                    if let Some(plan_state) = executor.plan_state_mut(&completion.plan_id) {
                        for verdict in &completion.verdicts {
                            plan_state.gate_results.push(GateResult {
                                gate_name: verdict.gate_name.clone(),
                                rung: completion.rung,
                                passed: verdict.passed,
                                summary: verdict.summary.clone(),
                                duration_ms: completion.duration_ms,
                                test_count: None,
                            });
                        }
                    }
                }
                let run_id = state.run_id().to_string();
                emit_runner_event(
                    &paths,
                    &mut state,
                    &tui,
                    config,
                    RunnerEvent::gate_completed(
                        &run_id,
                        completion_attempt.clone(),
                        &completion,
                    ),
                );
                record_daimon_gate_result(config, &completion);

                // Record gate outcome in the run ledger.
                if let Some(ref mut ledger) = run_ledger {
                    for verdict in &completion.verdicts {
                        ledger.record_gate_run(
                            &verdict.gate_name,
                            verdict.passed,
                            Some(verdict.summary.clone()),
                            completion.duration_ms,
                        );
                    }
                    append_ledger_entry(
                        &paths.run_ledger_jsonl,
                        "gate_outcome",
                        &serde_json::json!({
                            "plan_id": completion.plan_id,
                            "task_id": completion.task_id,
                            "rung": completion.rung,
                            "passed": completion.passed,
                            "duration_ms": completion.duration_ms,
                            "gate_kind": format!("{:?}", completion.kind),
                        }),
                    );
                }

                if completion.kind == GateCompletionKind::Merge {
                    emit_runner_event(
                        &paths,
                        &mut state,
                        &tui,
                        config,
                        RunnerEvent::merge_backend_completed(
                            &run_id,
                            completion_attempt.clone(),
                            &completion,
                            merge_branch_from_task_id(&completion.task_id),
                            conflict_paths_from_merge_output(&completion.output),
                        ),
                    );
                }

                let retry_budget = config
                    .max_retries
                    .min(gate_thresholds.suggested_max_retries(completion.rung));

                update_gate_thresholds(
                    &mut gate_thresholds,
                    completion.rung,
                    completion.passed,
                );
                emit_gate_thresholds_event(&gate_thresholds, &tui);

                // Publish gate result to the learning event bus so the
                // background subscriber can update VerdictHistory and
                // CalibrationPolicy.
                learning_event_bus.publish(
                    roko_learn::events::AgentEvent::GateResult {
                        gate_name: format!("rung-{}", completion.rung),
                        passed: completion.passed,
                        score: if completion.passed { 1.0 } else { 0.0 },
                        duration_ms: completion.duration_ms,
                    },
                );

                // Append gate verdict to signals.jsonl for audit / replay.
                {
                    let verdict_json = serde_json::json!({
                        "kind": "GateVerdict",
                        "plan_id": completion.plan_id,
                        "task_id": completion.task_id,
                        "rung": completion.rung,
                        "passed": completion.passed,
                        "gate_kind": format!("{:?}", completion.kind),
                        "duration_ms": completion.duration_ms,
                        "timestamp": chrono::Utc::now().to_rfc3339(),
                    });
                    let signals_path = config.layout.signals_path();
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&signals_path)
                    {
                        use std::io::Write;
                        let _ = writeln!(f, "{}", verdict_json);
                    }
                }

                // Extension: on_gate hook.
                fire_on_gate_hook(config, &completion, &tui).await;

                if completion.kind == GateCompletionKind::Merge {
                    handle_merge_completion(
                        &completion,
                        &mut executor,
                        &mut state,
                        &paths,
                        &merge_queue,
                        &gate_tx,
                        &config.workdir,
                        gate_timeout(&config, 0),
                        &tui,
                        config,
                        &gate_thresholds,
                        &snapshot_writer,
                    );
                    continue;
                }

                if completion.kind == GateCompletionKind::PlanVerify {
                    handle_plan_verify_completion(
                        &completion,
                        &mut executor,
                        &mut state,
                        &paths,
                        &merge_queue,
                        &tui,
                        config,
                        &gate_thresholds,
                        &snapshot_writer,
                    );
                    continue;
                }

                if completion.passed && completion.rung < config.max_gate_rung {
                    state.clear_retry_backoff(&completion.plan_id);
                    info!(
                        plan_id = %completion.plan_id,
                        task_id = %completion.task_id,
                        rung = completion.rung,
                        max_gate_rung = config.max_gate_rung,
                        "gate rung passed — advancing to next configured rung"
                    );
                    continue;
                }

                // ── SectionOutcome recording ────────────────────────────
                // Terminal gate result (final rung pass or any fail): join
                // the prompt section diagnostics captured at dispatch time
                // to the gate outcome so adaptive policy can learn which
                // sections correlate with success/failure.
                {
                    let attempt_key = completion_attempt.key();
                    if let Some(diag) = section_diagnostics.remove(&attempt_key) {
                        let status = if completion.passed {
                            SectionOutcomeStatus::Passed
                        } else {
                            SectionOutcomeStatus::Failed
                        };
                        let records = build_section_outcome_records(
                            &completion.plan_id,
                            &completion.task_id,
                            &state.agent_model,
                            &state.agent_provider,
                            status,
                            &diag,
                            &completion.verdicts,
                        );
                        let outcomes_path =
                            persist::section_outcomes_path(&config.workdir);
                        feedback_tasks.spawn(append_section_outcomes(
                            outcomes_path,
                            records,
                        ));
                    }
                }

                // ── Playbook outcome recording ───────────────────────────
                // Record gate pass/fail against every playbook that was
                // injected into this task's prompt. This closes the feedback
                // loop so PlaybookStore can learn which playbooks correlate
                // with task success and adaptive selection improves over time.
                {
                    let attempt_key = completion_attempt.key();
                    if let Some(pb_ids) = task_playbook_ids.remove(&attempt_key) {
                        let store = playbook_store.clone();
                        let gate_passed = completion.passed;
                        feedback_tasks.spawn(async move {
                            for pb_id in &pb_ids {
                                match store.record_outcome(pb_id, gate_passed).await {
                                    Ok(true) => {
                                        debug!(
                                            playbook_id = %pb_id,
                                            passed = gate_passed,
                                            "playbook outcome recorded"
                                        );
                                    }
                                    Ok(false) => {
                                        debug!(
                                            playbook_id = %pb_id,
                                            "playbook not found for outcome recording"
                                        );
                                    }
                                    Err(err) => {
                                        warn!(
                                            playbook_id = %pb_id,
                                            error = %err,
                                            "failed to record playbook outcome"
                                        );
                                    }
                                }
                            }
                        });
                    }
                }

                if completion.passed {
                    // Mark this task completed in the DAG and check for more.
                    state.clear_retry_backoff(&completion.plan_id);
                    state.mark_task_completed(&completion.plan_id, &completion.task_id);
                    let task_declared_files = task_index
                        .get(completion.plan_id.as_str())
                        .and_then(|tasks| tasks.get(completion.task_id.as_str()))
                        .map(|task| task.files.clone())
                        .unwrap_or_default();
                    // Snapshot which files this task produced so downstream
                    // tasks can be told what their dependencies already created.
                    let output_diffs = git_diff_entries_since_task_start(&config.workdir)
                        .into_iter()
                        .filter(|entry| {
                            task_path_allowed_by_declared_files(&entry.path, &task_declared_files)
                        })
                        .collect::<Vec<_>>();
                    let output_files = output_diffs
                        .iter()
                        .map(|entry| entry.path.clone())
                        .collect();
                    state.record_task_outputs(
                        &completion.plan_id,
                        &completion.task_id,
                        output_files,
                    );
                    state.task_completed();
                    // Record task completion in the run ledger.
                    if let Some(ref mut ledger) = run_ledger {
                        let now_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;
                        ledger.record_phase_transition(
                            roko_runtime::Phase::Implementing,
                            roko_runtime::Phase::Complete,
                            now_ms,
                        );
                        append_ledger_entry(
                            &paths.run_ledger_jsonl,
                            "task_completed",
                            &serde_json::json!({
                                "plan_id": completion.plan_id,
                                "task_id": completion.task_id,
                                "passed": true,
                                "duration_ms": completion.duration_ms,
                                "timestamp_ms": now_ms,
                            }),
                        );
                    }
                    let run_id = state.run_id().to_string();
                    let agent_model = state.agent_model.clone();
                    let agent_provider = state.agent_provider.clone();
                    emit_runner_event(
                        &paths,
                        &mut state,
                        &tui,
                        config,
                        RunnerEvent::task_attempt_completed(
                            &run_id,
                            completion_attempt.clone(),
                            TaskAttemptOutcome::Passed,
                            None,
                            completion.duration_ms,
                            agent_model,
                            agent_provider,
                        ),
                    );
                    record_daimon_task_outcome(
                        config,
                        state.current_daimon_strategy,
                        &completion.plan_id,
                        &completion.task_id,
                        true,
                        &format!("gate-rung-{}", completion.rung),
                    );
                    tui.task_completed(&completion.plan_id, &completion.task_id, "passed");

                    // Commit generated code to git so subsequent tasks can diff.
                    commit_task_changes(
                        &config.workdir,
                        &completion.plan_id,
                        &completion.task_id,
                        &task_declared_files,
                    );

                    let total_task_ms = state.task_elapsed_ms();
                    let dispatch_ms = state.last_dispatch_ms;
                    let gate_ms = completion.duration_ms;
                    let agent_ms = if state.task_agent_calls == 0 {
                        0
                    } else {
                        total_task_ms.saturating_sub(dispatch_ms + gate_ms)
                    };

                    sink.diff_block(&completion.plan_id, &completion.task_id, &output_diffs);
                    sink.task_completed(
                        &completion.plan_id,
                        &completion.task_id,
                        state.tasks_completed,
                        state.tasks_total,
                        total_task_ms,
                    );
                    info!(
                        task = %completion.task_id,
                        dispatch_ms,
                        agent_ms,
                        gate_ms,
                        "task timing"
                    );

                    let completed = state.plan_completed_tasks(&completion.plan_id);
                    let failed = state.plan_failed_tasks(&completion.plan_id);
                    let completed_plans = completed_plan_ids(&executor, &task_index);
                    let has_more = task_index
                        .get(completion.plan_id.as_str())
                        .map(|tasks| {
                            tasks
                                .values()
                                .any(|t| {
                                    !completed.contains(&t.id)
                                        && !failed.contains(&t.id)
                                        && t.is_ready_with_plan_deps(completed, &completed_plans)
                                })
                        })
                        .unwrap_or(false);

                    if has_more {
                        // More tasks remain — force plan back to Implementing so
                        // the next tick resolves the next ready task.
                        if let Some(ps) = executor.plan_state_mut(&completion.plan_id) {
                            ps.gate_results.clear();
                            ps.current_phase = PlanPhase::Implementing;
                        }
                        let remaining = task_index.get(completion.plan_id.as_str())
                            .map(|t| t.len().saturating_sub(completed.len() + failed.len())).unwrap_or(0);
                        info!(
                            plan_id = %completion.plan_id,
                            remaining,
                            "task passed — advancing to next task"
                        );
                    } else {
                        // All tasks done — run the plan-level verify chain.
                        let _ = executor.apply_event(&completion.plan_id, &ExecutorEvent::GatePassed);
                        info!(plan_id = %completion.plan_id, "all tasks passed — running plan verify");

                        // Queue dream consolidation only when this run actually
                        // spawned agents. Verification-only runs do not create
                        // new agent episodes, and blocking after run.completed
                        // on old episodes makes no-op reruns look stuck.
                        if state.total_agent_calls > 0 {
                            dream_completion_pending = true;
                            debug!("dream consolidation queued after plan completion");
                        } else {
                            debug!(
                                "dream consolidation skipped after verification-only plan completion"
                            );
                        }
                    }
                } else {
                    let failure_kind = completion
                        .failure_kind
                        .unwrap_or_else(|| RunnerFailureKind::from_output(&completion.output));
                    let can_retry = executor
                        .plan_state(&completion.plan_id)
                        .map(|ps| ps.iteration <= retry_budget && failure_kind.is_retryable())
                        .unwrap_or(false);
                    if can_retry {
                        match executor.apply_event(&completion.plan_id, &ExecutorEvent::GateFailed) {
                            Ok(phase) => {
                                let mut next_attempt = None;
                                let mut cooldown_ms = 0;
                                if let Some(ps) = executor.plan_state_mut(&completion.plan_id) {
                                    let attempt = ps.iteration;
                                    ps.reset_for_retry();
                                    state.set_retry_backoff(
                                        &completion.plan_id,
                                        failure_kind,
                                        attempt,
                                    );
                                    state.set_iteration(&completion.plan_id, &completion.task_id, ps.iteration);
                                    next_attempt = Some(ps.iteration);
                                    cooldown_ms = state
                                        .retry_cooldown_remaining(&completion.plan_id)
                                        .map(|duration| duration.as_millis() as u64)
                                        .unwrap_or_default();
                                }
                                let run_id = state.run_id().to_string();
                                emit_runner_event(
                                    &paths,
                                    &mut state,
                                    &tui,
                                    config,
                                    RunnerEvent::retry_decision(
                                        &run_id,
                                        completion_attempt.clone(),
                                        RetryAction::RetryAfterBackoff,
                                        failure_kind,
                                        next_attempt,
                                        cooldown_ms,
                                        "gate failed and retry policy allows auto-fix".to_string(),
                                    ),
                                );
                                tui.phase_transition(&completion.plan_id, "gating", &format!("{phase:?}"));

                                sink.gate_retry(
                                    &completion.plan_id,
                                    &completion.task_id,
                                    next_attempt.unwrap_or(
                                        state.iteration_for(&completion.plan_id, &completion.task_id) + 1,
                                    ),
                                    cooldown_ms,
                                );

                                info!(
                                    plan_id = %completion.plan_id,
                                    phase = ?phase,
                                    failure_kind = ?failure_kind,
                                    "gate failed — entering auto-fix"
                                );

                                // Enrich every retry prompt with failure context so the
                                // agent understands what went wrong and can adjust.
                                {
                                    let attempt_num = state.iteration_for(&completion.plan_id, &completion.task_id) + 1;
                                    let mut replan_context = build_gate_retry_context(
                                        &completion.output,
                                        &state.agent_output,
                                        attempt_num,
                                    );

                                    // Append lessons from past post-gate reflections.
                                    let gate_name = completion
                                        .verdicts
                                        .iter()
                                        .find(|v| !v.passed)
                                        .map(|v| v.gate_name.as_str())
                                        .unwrap_or("unknown");
                                    let lessons = lessons_from_post_gate_reflections(
                                        &config.layout.learn_dir(),
                                        gate_name,
                                        &completion.task_id,
                                    );
                                    if !lessons.is_empty() {
                                        replan_context.push_str(
                                            "\n\n### Lessons from past failures on this gate\n",
                                        );
                                        for lesson in &lessons {
                                            replan_context
                                                .push_str(&format!("- {lesson}\n"));
                                        }
                                        debug!(
                                            gate = %gate_name,
                                            lessons = lessons.len(),
                                            "post-gate reflection lessons added to retry prompt"
                                        );
                                    }

                                    state.set_replan_context(
                                        &completion.plan_id,
                                        &completion.task_id,
                                        replan_context,
                                    );
                                }

                                // Refresh prompt cache after gate failure — the
                                // agent may have written new episodes / knowledge
                                // that should inform the retry prompt.
                                prompt_cache = Arc::new(PromptCache::load(&config.workdir));
                                debug!("prompt cache refreshed after gate failure");
                            }
                            Err(e) => {
                                warn!(plan_id = %completion.plan_id, err = %e, "transition error after gate failure");
                            }
                        }
                    } else {
                        state.task_failed();
                        tui.task_completed(&completion.plan_id, &completion.task_id, "failed");
                        let reason = if failure_kind.is_retryable() {
                            format!("gate failed and retries exhausted: {}", completion.output)
                        } else {
                            format!(
                                "gate failed with non-retryable {failure_kind:?} failure: {}",
                                completion.output
                            )
                        };
                        state.record_task_failure(&completion.plan_id, &completion.task_id, &reason);
                        // Record task failure in the run ledger.
                        if let Some(ref mut ledger) = run_ledger {
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64;
                            ledger.record_agent_failed(
                                "implementer",
                                roko_runtime::EffectErrorKind::Unknown,
                                &reason,
                            );
                            append_ledger_entry(
                                &paths.run_ledger_jsonl,
                                "task_failed",
                                &serde_json::json!({
                                    "plan_id": completion.plan_id,
                                    "task_id": completion.task_id,
                                    "passed": false,
                                    "reason": reason,
                                    "duration_ms": completion.duration_ms,
                                    "timestamp_ms": now_ms,
                                }),
                            );
                        }

                        sink.task_failed(&completion.plan_id, &completion.task_id, &reason);
                        let run_id = state.run_id().to_string();
                        emit_runner_event(
                            &paths,
                            &mut state,
                            &tui,
                            config,
                            RunnerEvent::retry_decision(
                                &run_id,
                                completion_attempt.clone(),
                                if failure_kind.is_retryable() {
                                    RetryAction::Exhausted
                                } else {
                                    RetryAction::NotRetryable
                                },
                                failure_kind,
                                None,
                                0,
                                reason.clone(),
                            ),
                        );
                        let run_id = state.run_id().to_string();
                        let agent_model = state.agent_model.clone();
                        let agent_provider = state.agent_provider.clone();
                        emit_runner_event(
                            &paths,
                            &mut state,
                            &tui,
                            config,
                            RunnerEvent::task_attempt_completed(
                                &run_id,
                                completion_attempt.clone(),
                                if failure_kind.is_retryable() {
                                    TaskAttemptOutcome::Exhausted
                                } else {
                                    TaskAttemptOutcome::Failed
                                },
                                Some(failure_kind),
                                completion.duration_ms,
                                agent_model,
                                agent_provider,
                            ),
                        );
                        record_daimon_task_outcome(
                            config,
                            state.current_daimon_strategy,
                            &completion.plan_id,
                            &completion.task_id,
                            false,
                            &reason,
                        );

                        // Track this task as failed so dependents are skipped.
                        state.mark_task_failed(&completion.plan_id, &completion.task_id);

                        // Check if there are still runnable tasks in this plan
                        // (tasks whose deps don't include the failed task).
                        let completed = state.plan_completed_tasks(&completion.plan_id);
                        let failed = state.plan_failed_tasks(&completion.plan_id);
                        let completed_plans = completed_plan_ids(&executor, &task_index);
                        let has_runnable = task_index
                            .get(completion.plan_id.as_str())
                            .map(|tasks| {
                                tasks.values().any(|t| {
                                    !completed.contains(&t.id)
                                        && !failed.contains(&t.id)
                                        && t.is_ready_with_plan_deps(completed, &completed_plans)
                                })
                            })
                            .unwrap_or(false);

                        if has_runnable {
                            // Keep the plan in Implementing so the next tick
                            // picks up the next independent task.
                            if let Some(ps) = executor.plan_state_mut(&completion.plan_id) {
                                ps.gate_results.clear();
                                ps.current_phase = PlanPhase::Implementing;
                            }
                            warn!(
                                plan_id = %completion.plan_id,
                                task_id = %completion.task_id,
                                "task failed — skipping, other tasks remain"
                            );
                            tui.error(&format!(
                                "task {} failed (skipped) — continuing with remaining tasks",
                                completion.task_id
                            ));
                        } else {
                            // No more runnable tasks — fail the plan.
                            let _ = executor.apply_event(
                                &completion.plan_id,
                                &ExecutorEvent::Fatal(reason.clone()),
                            );
                            tui.error(&reason);
                        }
                    }
                }

                save_snapshot(config, &executor, &paths, &mut state, &merge_queue, &gate_thresholds, &snapshot_writer);
            }

            // ─── Branch 3: Executor tick ────────────────────────────
            _ = tick_interval.tick() => {
                // Refresh prompt cache if stale (default 5 min).
                if prompt_cache.is_stale() {
                    prompt_cache = Arc::new(PromptCache::load(&config.workdir));
                    debug!("prompt cache refreshed (stale)");
                }
                let actions = executor.tick();
                for action in actions {
                    let t_dispatch = Instant::now();
                    let action_label = match &action {
                        ExecutorAction::SpawnAgent { plan_id, task, .. } => {
                            format!("{plan_id}/{task}")
                        }
                        ExecutorAction::DispatchPlan { plan_id } => {
                            format!("{plan_id}/plan")
                        }
                        ExecutorAction::RunGate { plan_id, rung } => {
                            format!("{plan_id}/gate-{rung}")
                        }
                        _ => "other".to_string(),
                    };
                    let mut ctx = RunContext {
                        executor: &mut executor,
                        task_index: &task_index,
                        skip_enrichment: &skip_enrichment,
                        config,
                        sink,
                        tui: &tui,
                        state: &mut state,
                        active_agent_tasks: &mut active_agent_tasks,
                        agent_handles: &mut agent_handles,
                        agent_tx: &agent_tx,
                        gate_tx: &gate_tx,
                        fatal_tx: agent_tx.clone(),
                        paths: &paths,
                        merge_queue: &merge_queue,
                        gate_thresholds: &gate_thresholds,
                        snapshot_writer: &snapshot_writer,
                        prompt_cache: &prompt_cache,
                        factory: &factory,
                        gate_sem: gate_sem.clone(),
                        section_diagnostics: &mut section_diagnostics,
                        task_playbook_ids: &mut task_playbook_ids,
                    };
                    let dispatch_outcome = dispatch_action(&action, &mut ctx).await;
                    let dispatch_ms = t_dispatch.elapsed().as_millis() as u64;
                    if let ActionDispatchOutcome::AgentStarted { plan_id, task_id } = dispatch_outcome {
                        ctx.state.last_dispatch_ms = dispatch_ms;
                        let action_label = format!("{plan_id}/{task_id}");
                        info!(action = %action_label, dispatch_ms, "agent action dispatched");
                        // Record task start in the run ledger.
                        if let Some(ref mut ledger) = run_ledger {
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64;
                            ledger.record_phase_transition(
                                roko_runtime::Phase::Pending,
                                roko_runtime::Phase::Implementing,
                                now_ms,
                            );
                            append_ledger_entry(
                                &paths.run_ledger_jsonl,
                                "task_started",
                                &serde_json::json!({
                                    "plan_id": plan_id,
                                    "task_id": task_id,
                                    "timestamp_ms": now_ms,
                                }),
                            );
                        }
                    } else if matches!(&action, ExecutorAction::SpawnAgent { .. }) {
                        debug!(action = %action_label, dispatch_ms, "agent action suppressed or delayed");
                    } else if dispatch_ms > 50 {
                        info!(action = %action_label, dispatch_ms, "action dispatched (slow)");
                    } else {
                        debug!(action = %action_label, dispatch_ms, "action dispatched");
                    }
                }
            }

            // ─── Branch 4: Periodic flush ───────────────────────────
            _ = flush_interval.tick() => {
                save_snapshot(config, &executor, &paths, &mut state, &merge_queue, &gate_thresholds, &snapshot_writer);
                {
                    let pids: Vec<u32> = agent_handles.values().map(|h| h.pid).collect();
                    if !pids.is_empty() {
                        let _ = persist::save_agent_pids(&paths, &pids);
                    }
                }
            }

            // ─── Branch 5: Plan timeout ──────────────────────────────
            _ = &mut plan_timeout, if !timed_out => {
                handle_plan_timeout(
                    &executor,
                    &plans,
                    &mut state,
                    &mut agent_handles,
                    &paths,
                    &merge_queue,
                    &tui,
                    config,
                    &gate_thresholds,
                    &snapshot_writer,
                )
                .await?;
                timed_out = true;
            }

            // ─── Branch 6: Cancellation ─────────────────────────────
            _ = cancel.cancelled() => {
                warn!("cancellation requested — shutting down");
                stop_all_agents(&mut agent_handles, &mut state, Duration::from_secs(3)).await;
                save_snapshot(config, &executor, &paths, &mut state, &merge_queue, &gate_thresholds, &snapshot_writer);
                snapshot_writer.flush();
                shutdown_subsystems(config, &tui).await;
                let event =
                    build_run_completed_event(&executor, &plans, &state, RunOutcome::Cancelled);
                emit_runner_event(&paths, &mut state, &tui, config, event);
                break;
            }
        }

        if !timed_out && tokio::time::Instant::now() >= plan_deadline {
            handle_plan_timeout(
                &executor,
                &plans,
                &mut state,
                &mut agent_handles,
                &paths,
                &merge_queue,
                &tui,
                config,
                &gate_thresholds,
                &snapshot_writer,
            )
            .await?;
            timed_out = true;
        }

        if all_plans_terminal(&executor, &plans) {
            save_snapshot(
                config,
                &executor,
                &paths,
                &mut state,
                &merge_queue,
                &gate_thresholds,
                &snapshot_writer,
            );
            let final_report = build_report(&executor, &plans, &state);
            let outcome = if final_report.all_succeeded() {
                RunOutcome::Succeeded
            } else {
                RunOutcome::Failed
            };
            let event = build_run_completed_event(&executor, &plans, &state, outcome);
            emit_runner_event(&paths, &mut state, &tui, config, event);
            let cost_display = format!("{:.4}", final_report.total_cost_usd);
            info!(
                outcome = ?outcome,
                total_tasks = final_report.total_tasks,
                completed = final_report.tasks_completed,
                failed = final_report.tasks_failed,
                cost_usd = %cost_display,
                tokens_in = final_report.total_tokens_in,
                tokens_out = final_report.total_tokens_out,
                agent_calls = final_report.total_agent_calls,
                duration_secs = final_report.duration.as_secs(),
                "run complete — exiting event loop"
            );
            for plan_report in &final_report.plans {
                info!(
                    plan_id = %plan_report.plan_id,
                    completed = plan_report.completed,
                    tasks_done = plan_report.tasks_completed,
                    tasks_total = plan_report.tasks_total,
                    tasks_failed = plan_report.tasks_failed,
                    "plan summary"
                );
            }
            // Persist the run ledger at run completion.
            persist_run_ledger(&run_ledger, &paths.run_ledger_jsonl);
            break;
        }
    }

    // Drain any pending feedback tasks.
    while feedback_tasks.try_join_next().is_some() {}

    // Ensure all pending snapshots land on disk before returning.
    snapshot_writer.flush();

    // Persist the run ledger (final write on the general exit path).
    persist_run_ledger(&run_ledger, &paths.run_ledger_jsonl);

    let report = build_report(&executor, &plans, &state);

    // Shut down the learning subscriber after the event bus is closed so
    // pending turn events are flushed to `.roko/learn/efficiency.jsonl`.
    drop(learning_event_bus);
    if let Err(err) = learning_subscriber_handle.await {
        warn!(error = %err, "learning subscriber task failed during shutdown");
    }

    // Shutdown Phase 0 subsystems and persist learned state.
    shutdown_subsystems(config, &tui).await;

    if dream_completion_pending && !cancel.is_cancelled() {
        run_dream_consolidation_if_enabled(config).await;
    }

    // ── Post-run episode compaction ──────────────────────────────────
    //
    // Compact the episode log using the default retention policy.  This
    // runs after the main loop so it does not contend with the episode
    // sink appending new entries.
    compact_episodes_if_needed(&paths.episodes_jsonl).await;

    Ok(report)
}

fn apply_agent_completion(executor: &mut ParallelExecutor, plan_id: &str, tui: &TuiBridge) {
    let Some(phase_kind) = executor
        .plan_state(plan_id)
        .map(|state| state.current_phase.kind())
    else {
        warn!(plan_id = %plan_id, "agent completed for unknown plan");
        return;
    };

    let event = match phase_kind {
        PhaseKind::Enriching => ExecutorEvent::EnrichmentDone,
        PhaseKind::Implementing => ExecutorEvent::ImplementationDone,
        PhaseKind::AutoFixing => ExecutorEvent::AutoFixDone,
        PhaseKind::RegeneratingVerify => ExecutorEvent::VerifyRegenDone,
        PhaseKind::Reviewing => ExecutorEvent::ReviewApproved,
        PhaseKind::DocRevision => ExecutorEvent::DocRevisionDone,
        _ => {
            info!(
                plan_id = %plan_id,
                phase = ?phase_kind,
                "agent completion ignored for phase"
            );
            return;
        }
    };

    match executor.apply_event(plan_id, &event) {
        Ok(phase) => {
            tui.phase_transition(plan_id, &format!("{phase_kind:?}"), &format!("{phase:?}"));
            info!(plan_id = %plan_id, from = ?phase_kind, phase = ?phase, "agent phase completed");
        }
        Err(e) => {
            warn!(plan_id = %plan_id, err = %e, "transition error after agent completion");
        }
    }
}

fn turn_exceeds_budget(total_cost_usd: Option<f64>, max_turn_usd: f64) -> bool {
    max_turn_usd > 0.0 && total_cost_usd.is_some_and(|cost| cost > max_turn_usd)
}

fn agent_failure_message(agent_output: &str) -> Option<String> {
    agent_output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && (line.contains("API Error")
                    || line.to_ascii_lowercase().contains("error")
                    || line.to_ascii_lowercase().contains("failed"))
        })
        .map(ToOwned::to_owned)
}

fn build_agent_retry_context(message: &str, agent_output: &str, attempt_num: u32) -> String {
    let agent_excerpt = if agent_output.len() > 3000 {
        &agent_output[..3000]
    } else {
        agent_output
    };
    format!(
        "## IMPORTANT: Your previous agent turn failed\n\n\
         Attempt {attempt_num} ended with an agent/runtime error.\n\n\
         ### Error\n```\n{message}\n```\n\n\
         ### Previous agent output\n```\n{agent_excerpt}\n```"
    )
}

fn handle_agent_failure(
    executor: &mut ParallelExecutor,
    task_index: &HashMap<String, HashMap<String, TaskDef>>,
    state: &mut RunState,
    paths: &PersistPaths,
    tui: &TuiBridge,
    sink: &dyn RunOutputSink,
    config: &RunConfig,
    message: String,
) {
    let plan_id = state.plan_id.clone();
    let task_id = state.current_task.clone();
    if plan_id.is_empty() || task_id.is_empty() {
        tui.error(&message);
        return;
    }

    let failure_text = format!("{message}\n{}", state.agent_output);
    let failure_kind = RunnerFailureKind::from_output(&failure_text);
    let retry_budget = config.max_retries;
    let can_retry = executor
        .plan_state(&plan_id)
        .map(|ps| ps.iteration <= retry_budget && failure_kind.is_retryable())
        .unwrap_or(false);
    let attempt = state.current_attempt_ref();
    let run_id = state.run_id().to_string();

    if can_retry {
        let mut next_attempt = None;
        let mut cooldown_ms = 0;
        if let Some(ps) = executor.plan_state_mut(&plan_id) {
            let failed_attempt = ps.iteration;
            ps.reset_for_retry();
            ps.current_phase = PlanPhase::Implementing;
            state.set_retry_backoff(&plan_id, failure_kind, failed_attempt);
            state.set_iteration(&plan_id, &task_id, ps.iteration);
            next_attempt = Some(ps.iteration);
            cooldown_ms = state
                .retry_cooldown_remaining(&plan_id)
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or_default();
        }

        let retry_attempt = next_attempt.unwrap_or_else(|| {
            state
                .iteration_for(&plan_id, &task_id)
                .saturating_add(1)
                .max(1)
        });
        state.set_replan_context(
            &plan_id,
            &task_id,
            build_agent_retry_context(&message, &state.agent_output, retry_attempt),
        );
        emit_runner_event(
            paths,
            state,
            tui,
            config,
            RunnerEvent::retry_decision(
                &run_id,
                attempt,
                RetryAction::RetryAfterBackoff,
                failure_kind,
                next_attempt,
                cooldown_ms,
                "agent turn failed and retry policy allows another attempt".to_string(),
            ),
        );
        warn!(
            plan_id = %plan_id,
            task_id = %task_id,
            failure_kind = ?failure_kind,
            cooldown_ms,
            "agent turn failed — retrying task after backoff"
        );
        tui.error(&format!(
            "agent turn failed for {task_id}; retrying after {}s",
            cooldown_ms / 1000
        ));
        return;
    }

    state.task_failed();
    let reason = if failure_kind.is_retryable() {
        format!("agent turn failed and retries exhausted: {message}")
    } else {
        format!("agent turn failed with non-retryable {failure_kind:?} failure: {message}")
    };
    state.record_task_failure(&plan_id, &task_id, &reason);
    state.mark_task_failed(&plan_id, &task_id);
    sink.task_failed(&plan_id, &task_id, &reason);
    tui.task_completed(&plan_id, &task_id, "failed");

    append_ledger_entry(
        &paths.run_ledger_jsonl,
        "task_failed",
        &serde_json::json!({
            "plan_id": &plan_id,
            "task_id": &task_id,
            "passed": false,
            "reason": &reason,
            "timestamp_ms": chrono::Utc::now().timestamp_millis().max(0) as u64,
        }),
    );

    emit_runner_event(
        paths,
        state,
        tui,
        config,
        RunnerEvent::retry_decision(
            &run_id,
            attempt.clone(),
            if failure_kind.is_retryable() {
                RetryAction::Exhausted
            } else {
                RetryAction::NotRetryable
            },
            failure_kind,
            None,
            0,
            reason.clone(),
        ),
    );
    let agent_model = state.agent_model.clone();
    let agent_provider = state.agent_provider.clone();
    emit_runner_event(
        paths,
        state,
        tui,
        config,
        RunnerEvent::task_attempt_completed(
            &run_id,
            attempt,
            if failure_kind.is_retryable() {
                TaskAttemptOutcome::Exhausted
            } else {
                TaskAttemptOutcome::Failed
            },
            Some(failure_kind),
            state.task_elapsed_ms(),
            agent_model,
            agent_provider,
        ),
    );
    record_daimon_task_outcome(
        config,
        state.current_daimon_strategy,
        &plan_id,
        &task_id,
        false,
        &reason,
    );

    let completed = state.plan_completed_tasks(&plan_id);
    let failed = state.plan_failed_tasks(&plan_id);
    let completed_plans = completed_plan_ids(executor, task_index);
    let has_runnable = task_index
        .get(plan_id.as_str())
        .map(|tasks| {
            tasks.values().any(|t| {
                !completed.contains(&t.id)
                    && !failed.contains(&t.id)
                    && t.is_ready_with_plan_deps(completed, &completed_plans)
            })
        })
        .unwrap_or(false);

    if has_runnable {
        if let Some(ps) = executor.plan_state_mut(&plan_id) {
            ps.gate_results.clear();
            ps.current_phase = PlanPhase::Implementing;
        }
        warn!(
            plan_id = %plan_id,
            task_id = %task_id,
            "agent failed task — continuing with remaining independent tasks"
        );
        tui.error(&format!(
            "task {task_id} failed after agent error — continuing with remaining tasks"
        ));
    } else if let Err(err) = executor.apply_event(&plan_id, &ExecutorEvent::Fatal(reason.clone())) {
        error!(plan_id = %plan_id, error = %err, "failed to apply Fatal event after agent failure");
        state.force_plan_terminal(&plan_id);
        tui.error(&reason);
    } else {
        tui.error(&reason);
    }
}

fn complete_plan_after_successful_verify(
    plan_id: &str,
    executor: &mut ParallelExecutor,
) -> Result<PlanPhase, TransitionError> {
    let mut phase = match executor.plan_state(plan_id) {
        Some(state) if matches!(state.current_phase, PlanPhase::Complete) => {
            return Ok(PlanPhase::Complete);
        }
        Some(state) if state.current_phase.kind() == PhaseKind::Verifying => {
            executor.apply_event(plan_id, &ExecutorEvent::VerifyPassed)?
        }
        Some(state) if state.current_phase.kind() == PhaseKind::Reviewing => {
            state.current_phase.clone()
        }
        Some(state) if state.current_phase.kind() == PhaseKind::DocRevision => {
            state.current_phase.clone()
        }
        Some(state) if state.current_phase.kind() == PhaseKind::Merging => {
            state.current_phase.clone()
        }
        Some(state) => {
            let from = state.current_phase.kind();
            return Err(TransitionError {
                from,
                to: PhaseKind::Complete,
                reason: format!("cannot complete verified plan from unexpected phase {from:?}"),
            });
        }
        None => {
            return Err(TransitionError {
                from: PhaseKind::Queued,
                to: PhaseKind::Complete,
                reason: format!("plan '{plan_id}' not found"),
            });
        }
    };

    if phase.kind() == PhaseKind::Reviewing {
        phase = executor.apply_event(plan_id, &ExecutorEvent::ReviewApproved)?;
    }
    if phase.kind() == PhaseKind::DocRevision {
        phase = executor.apply_event(plan_id, &ExecutorEvent::DocRevisionDone)?;
    }
    if phase.kind() == PhaseKind::Merging {
        phase = executor.apply_event(plan_id, &ExecutorEvent::MergeSucceeded)?;
    }

    Ok(phase)
}

fn complete_verified_plan_success(
    plan_id: &str,
    executor: &mut ParallelExecutor,
    state: &mut RunState,
    paths: &PersistPaths,
    tui: &TuiBridge,
    config: &RunConfig,
) -> Result<PlanPhase, TransitionError> {
    let was_complete = executor
        .plan_state(plan_id)
        .is_some_and(|state| matches!(state.current_phase, PlanPhase::Complete));
    let phase = complete_plan_after_successful_verify(plan_id, executor)?;
    if !was_complete {
        tui.plan_completed(plan_id, true);
        let run_id = state.run_id().to_string();
        emit_runner_event(
            paths,
            state,
            tui,
            config,
            RunnerEvent::plan_completed(&run_id, plan_id, PlanOutcome::Succeeded, None),
        );
    }
    Ok(phase)
}

fn handle_plan_verify_completion(
    completion: &GateCompletion,
    executor: &mut ParallelExecutor,
    state: &mut RunState,
    paths: &PersistPaths,
    merge_queue: &MergeQueue,
    tui: &TuiBridge,
    config: &RunConfig,
    gate_thresholds: &GateThresholds,
    writer: &SnapshotWriter,
) {
    if completion.passed {
        state.clear_retry_backoff(&completion.plan_id);
        match complete_verified_plan_success(
            &completion.plan_id,
            executor,
            state,
            paths,
            tui,
            config,
        ) {
            Ok(phase) => {
                tui.phase_transition(&completion.plan_id, "verifying", &format!("{phase:?}"));
                info!(plan_id = %completion.plan_id, phase = ?phase, "plan verify passed — plan complete");
            }
            Err(e) => {
                warn!(
                    plan_id = %completion.plan_id,
                    err = %e,
                    "transition error while completing plan after verify pass"
                );
                let _ = executor.apply_event(
                    &completion.plan_id,
                    &ExecutorEvent::Fatal(format!("plan verify transition failed: {e}")),
                );
            }
        }
    } else {
        let failure_kind = completion
            .failure_kind
            .unwrap_or_else(|| RunnerFailureKind::from_output(&completion.output));
        let iter = state.iteration_for(&completion.plan_id, &completion.task_id);
        state.set_retry_backoff(&completion.plan_id, failure_kind, iter);
        let cooldown_ms = state
            .retry_cooldown_remaining(&completion.plan_id)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or_default();
        let run_id = state.run_id().to_string();
        let attempt = TaskAttemptRef::new(
            completion.plan_id.clone(),
            completion.task_id.clone(),
            state.iteration_for(&completion.plan_id, &completion.task_id),
        );
        let cur_iter = state.iteration_for(&completion.plan_id, &completion.task_id);
        let next_attempt = Some(cur_iter.saturating_add(1).max(1));
        emit_runner_event(
            paths,
            state,
            tui,
            config,
            RunnerEvent::retry_decision(
                &run_id,
                attempt,
                RetryAction::RetryAfterBackoff,
                failure_kind,
                next_attempt,
                cooldown_ms,
                "plan verify failed and verify regeneration is available".to_string(),
            ),
        );
        match executor.apply_event(&completion.plan_id, &ExecutorEvent::VerifyFailed) {
            Ok(phase) => {
                tui.phase_transition(&completion.plan_id, "verifying", &format!("{phase:?}"));
                warn!(
                    plan_id = %completion.plan_id,
                    failure_kind = ?failure_kind,
                    phase = ?phase,
                    "plan verify failed"
                );
            }
            Err(e) => {
                let reason = format!("plan verify failed: {}", completion.output);
                warn!(
                    plan_id = %completion.plan_id,
                    err = %e,
                    "transition error after plan verify failure"
                );
                let _ = executor.apply_event(&completion.plan_id, &ExecutorEvent::Fatal(reason));
            }
        }
    }

    save_snapshot(
        config,
        executor,
        paths,
        state,
        merge_queue,
        gate_thresholds,
        writer,
    );
}

fn merge_branch_from_task_id(task_id: &str) -> Option<String> {
    task_id
        .strip_prefix("merge:")
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
        .map(ToOwned::to_owned)
}

fn conflict_paths_from_merge_output(output: &str) -> Vec<String> {
    output
        .lines()
        .find_map(|line| {
            line.split_once("conflicted paths:")
                .map(|(_, paths)| paths.to_string())
        })
        .map(|paths| {
            paths
                .split([',', ' ', '\t'])
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn handle_merge_completion(
    completion: &GateCompletion,
    executor: &mut ParallelExecutor,
    state: &mut RunState,
    paths: &PersistPaths,
    merge_queue: &MergeQueue,
    gate_tx: &mpsc::Sender<GateCompletion>,
    workdir: &Path,
    regression_timeout: Duration,
    tui: &TuiBridge,
    config: &RunConfig,
    gate_thresholds: &GateThresholds,
    writer: &SnapshotWriter,
) {
    let run_id = state.run_id().to_string();
    if completion.passed {
        match executor.apply_event(&completion.plan_id, &ExecutorEvent::MergeSucceeded) {
            Ok(phase) => {
                tui.phase_transition(&completion.plan_id, "merging", &format!("{phase:?}"));
                tui.plan_completed(&completion.plan_id, true);
                emit_runner_event(
                    paths,
                    state,
                    tui,
                    config,
                    RunnerEvent::plan_completed(
                        &run_id,
                        &completion.plan_id,
                        PlanOutcome::Succeeded,
                        None,
                    ),
                );
                info!(
                    plan_id = %completion.plan_id,
                    output = %completion.output,
                    "merge finalized and regression passed"
                );
            }
            Err(err) => {
                let reason = format!("executor rejected successful merge: {err}");
                let _ = executor.apply_event(&completion.plan_id, &ExecutorEvent::Fatal(reason));
            }
        }
    } else {
        let reason = format!("merge failed: {}", completion.output);
        match executor.apply_event(&completion.plan_id, &ExecutorEvent::MergeFailed) {
            Ok(phase) => {
                tui.phase_transition(&completion.plan_id, "merging", &format!("{phase:?}"));
                tui.plan_completed(&completion.plan_id, false);
            }
            Err(err) => {
                warn!(
                    plan_id = %completion.plan_id,
                    error = %err,
                    "transition error after merge failure"
                );
                let _ = executor
                    .apply_event(&completion.plan_id, &ExecutorEvent::Fatal(reason.clone()));
            }
        }
        emit_runner_event(
            paths,
            state,
            tui,
            config,
            RunnerEvent::plan_completed(
                &run_id,
                &completion.plan_id,
                PlanOutcome::Failed,
                Some(reason.clone()),
            ),
        );
        tui.error(&reason);
    }

    if let Some(next_plan_id) = PlanMerger::new(
        merge_queue.clone(),
        PlanMergerConfig::new(workdir.to_path_buf(), regression_timeout),
    )
    .drain_next(gate_tx.clone())
    {
        info!(plan_id = %next_plan_id, "started next queued merge");
    }
    save_snapshot(
        config,
        executor,
        paths,
        state,
        merge_queue,
        gate_thresholds,
        writer,
    );
}

fn append_agent_event(paths: &PersistPaths, event: &AgentEvent, state: &RunState) {
    let event_type = event.event_type();

    let payload = serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "timestamp_ms": chrono::Utc::now().timestamp_millis().max(0) as u64,
        "type": event_type,
        "run_id": state.run_id(),
        "plan_id": state.plan_id.clone(),
        "task_id": state.current_task.clone(),
        "attempt": state.iteration_for(&state.plan_id, &state.current_task),
        "agent_pid": state.agent_pid,
        "event": agent_event_json(event),
    });

    if let Err(err) = persist::append_jsonl(&paths.events_jsonl, &payload) {
        warn!(error = %err, "failed to append runner event");
    }
}

fn publish_learning_agent_event(
    bus: &roko_learn::events::EventBus,
    event: &AgentEvent,
    state: &RunState,
) {
    match event {
        AgentEvent::Started {
            provider, model, ..
        } => {
            bus.publish(roko_learn::events::AgentEvent::TurnStarted {
                task_id: learning_task_id(state),
                model: model.clone(),
                provider: provider.clone(),
                timestamp_ms: chrono::Utc::now().timestamp_millis(),
            });
        }
        AgentEvent::TurnCompleted {
            total_cost_usd,
            num_turns,
            is_error,
            ..
        } => {
            let finish_reason = if *is_error {
                roko_agent::chat_types::FinishReason::Error("agent reported an error".to_string())
            } else {
                roko_agent::chat_types::FinishReason::Stop
            };
            bus.publish(roko_learn::events::AgentEvent::TurnCompleted {
                turn: num_turns.unwrap_or(1),
                usage: roko_agent::Usage {
                    input_tokens: saturating_u32(state.tokens_in),
                    output_tokens: saturating_u32(state.tokens_out),
                    cache_read_tokens: saturating_u32(state.cache_read_tokens),
                    cache_create_tokens: saturating_u32(state.cache_write_tokens),
                    cost_usd: total_cost_usd.unwrap_or(state.cost_usd) as f32,
                    wall_ms: state.task_elapsed_ms(),
                },
                tool_call_count: 0,
                gate_passed: None,
                finish_reason,
            });
        }
        _ => {}
    }
}

fn learning_task_id(state: &RunState) -> String {
    if state.plan_id.is_empty() {
        state.current_task.clone()
    } else {
        let agent_call = state.total_agent_calls.max(1);
        format!("{}:{}:{}", state.plan_id, state.current_task, agent_call)
    }
}

fn saturating_u32(value: u64) -> u32 {
    value.min(u64::from(u32::MAX)) as u32
}

fn agent_event_json(event: &AgentEvent) -> serde_json::Value {
    match event {
        AgentEvent::Started {
            agent_id,
            provider,
            model,
            pid,
        } => serde_json::json!({
            "agent_id": agent_id,
            "provider": provider,
            "model": model,
            "pid": pid,
        }),
        AgentEvent::SystemInit { session_id, model } => {
            serde_json::json!({"session_id": session_id, "model": model})
        }
        AgentEvent::MessageDelta { text } => serde_json::json!({"text": text}),
        AgentEvent::ToolCall { id, name } => serde_json::json!({"id": id, "name": name}),
        AgentEvent::ToolOutput { id, output } => serde_json::json!({"id": id, "output": output}),
        AgentEvent::TokenUsage {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        } => serde_json::json!({
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "cache_read_tokens": cache_read_tokens,
            "cache_write_tokens": cache_write_tokens,
        }),
        AgentEvent::TurnCompleted {
            session_id,
            total_cost_usd,
            num_turns,
            is_error,
        } => serde_json::json!({
            "session_id": session_id,
            "total_cost_usd": total_cost_usd,
            "num_turns": num_turns,
            "is_error": is_error,
        }),
        AgentEvent::Error { message } => serde_json::json!({"message": message}),
        AgentEvent::Exited { exit_code } => serde_json::json!({"exit_code": exit_code}),
    }
}

/// Single emit path for runner lifecycle events.
///
/// Owns:
/// - state apply (`RunState::apply_runner_event`)
/// - TUI dashboard publish (`TuiBridge::runner_event`)
/// - durable JSONL append (`persist::append_runner_event`)
/// - **projection broadcast** (`config.projection`)
/// - **feedback fan-out** (`config.feedback_facade`, fire-and-forget)
///
/// Helpers that do not have `&RunConfig` in scope use
/// [`emit_runner_event_facadeless`] which is equivalent to passing
/// `None`/`None` for projection + feedback.
fn emit_runner_event(
    paths: &PersistPaths,
    state: &mut RunState,
    tui: &TuiBridge,
    config: &RunConfig,
    event: RunnerEvent,
) {
    emit_runner_event_with_facades(
        paths,
        state,
        tui,
        config.projection.as_ref(),
        config.feedback_facade.as_ref(),
        config.http_event_sink.as_ref(),
        event,
        None,
    );
}

/// Drop-in for emit sites that do not hold a `&RunConfig` (helpers
/// invoked outside `run()`). Skips projection + feedback fan-out; the
/// runner-level emits still cover the lifecycle events these helpers
/// produce because the helpers themselves only emit on their plan's
/// completion which is also republished from `run()`.
fn emit_runner_event_facadeless(
    paths: &PersistPaths,
    state: &mut RunState,
    tui: &TuiBridge,
    event: RunnerEvent,
) {
    emit_runner_event_with_facades(paths, state, tui, None, None, None, event, None);
}

/// Internal variant accepting the optional projection + feedback facades.
fn emit_runner_event_with_facades(
    paths: &PersistPaths,
    state: &mut RunState,
    tui: &TuiBridge,
    projection: Option<&Arc<super::projection::Projection>>,
    feedback_facade: Option<&Arc<crate::runtime_feedback::FeedbackFacade>>,
    http_event_sink: Option<&HttpEventSink>,
    event: RunnerEvent,
    feedback_tasks: Option<&mut tokio::task::JoinSet<()>>,
) {
    state.apply_runner_event(&event);
    tui.runner_event(&event);
    if let Err(err) = persist::append_runner_event(paths, &event) {
        warn!(
            event_type = event.event_type(),
            error = %err,
            "failed to append runner lifecycle event"
        );
    }

    // ── Mirror to projection facade ─────────────────────────────────────
    if let Some(proj) = projection {
        let raw = super::projection::RawRuntimeEvent::Runner(event.clone());
        match proj.publish(raw) {
            Ok(()) => {}
            Err(super::projection::ProjectionError::NoSubscribers) => {
                // Publishing without live subscribers is normal during smoke
                // runs — the projection facade tracks the dropped-event
                // counter for diagnostics.
            }
        }
    }

    // ── Forward canonical RuntimeEvent over HTTP, if configured ─────────
    if let Some(sink) = http_event_sink {
        let runtime_event = runner_event_to_runtime_event(&event);
        sink.emit(runtime_event.clone());
    }

    // ── Translate to FeedbackEvent and fan out ──────────────────────────
    if let Some(facade) = feedback_facade {
        let usage = TaskUsageSnapshot {
            tokens_in: state.tokens_in,
            tokens_out: state.tokens_out,
            cost_usd: state.cost_usd,
            duration_ms: state.task_elapsed_ms(),
            prompt_text: (!state.current_prompt_text.trim().is_empty())
                .then(|| state.current_prompt_text.clone()),
            agent_output: state.agent_output.clone(),
        };
        if let Some(feedback) = runner_event_to_feedback(&event, &state.routing_context, &usage) {
            if let Some(tasks) = feedback_tasks {
                // Reap completed tasks (non-blocking) to prevent unbounded growth.
                while tasks.try_join_next().is_some() {}

                if tasks.len() >= 32 {
                    debug!(
                        "feedback task backlog full ({} tasks), dropping event",
                        tasks.len()
                    );
                } else {
                    let facade = Arc::clone(facade);
                    tasks.spawn(async move {
                        if let Err(err) = facade.on_event(&feedback).await {
                            warn!(
                                event_type = feedback.label(),
                                %err,
                                "feedback facade returned terminal error",
                            );
                        }
                    });
                }
            } else {
                // Fallback for callers that don't provide a JoinSet.
                let facade = Arc::clone(facade);
                tokio::spawn(async move {
                    if let Err(err) = facade.on_event(&feedback).await {
                        warn!(
                            event_type = feedback.label(),
                            %err,
                            "feedback facade returned terminal error",
                        );
                    }
                });
            }
        }
    }
}

fn runner_event_to_runtime_event(event: &RunnerEvent) -> RuntimeEvent {
    match event {
        RunnerEvent::RunStarted {
            run_id,
            plan_ids,
            total_tasks,
            resumed,
            ..
        } => RuntimeEvent::WorkflowStarted {
            run_id: run_id.clone(),
            template: "runner-v2".to_string(),
            prompt: format!(
                "plans={} total_tasks={} resumed={}",
                plan_ids.join(","),
                total_tasks,
                resumed
            ),
        },
        RunnerEvent::RunCompleted {
            run_id, outcome, ..
        } => RuntimeEvent::WorkflowCompleted {
            run_id: run_id.clone(),
            outcome: runtime_workflow_outcome(*outcome),
        },
        RunnerEvent::AgentDispatchStarted {
            run_id,
            agent_id,
            role,
            requested_model,
            ..
        } => RuntimeEvent::AgentSpawned {
            run_id: run_id.clone(),
            agent_id: agent_id.clone(),
            role: role.clone(),
            model: requested_model.clone(),
        },
        RunnerEvent::AgentDispatchCompleted {
            run_id,
            agent_id,
            outcome:
                AgentDispatchOutcome::SpawnFailed
                | AgentDispatchOutcome::Failed
                | AgentDispatchOutcome::Exited,
            message,
            ..
        } => RuntimeEvent::AgentFailed {
            run_id: run_id.clone(),
            agent_id: agent_id.clone(),
            error: message.clone().unwrap_or_else(|| event.message()),
        },
        RunnerEvent::AgentCompleted {
            run_id,
            agent_id,
            outcome,
            total_cost_usd,
            exit_code,
            message,
            ..
        } if agent_completion_succeeded(*outcome, *exit_code) => RuntimeEvent::AgentCompleted {
            run_id: run_id.clone(),
            agent_id: agent_id.clone(),
            output: message.clone().unwrap_or_else(|| event.message()),
            tokens_used: 0,
            cost_usd: total_cost_usd.unwrap_or(0.0),
        },
        RunnerEvent::AgentCompleted {
            run_id,
            agent_id,
            message,
            ..
        } => RuntimeEvent::AgentFailed {
            run_id: run_id.clone(),
            agent_id: agent_id.clone(),
            error: message.clone().unwrap_or_else(|| event.message()),
        },
        RunnerEvent::GateDispatchStarted {
            run_id,
            attempt,
            kind,
            rung,
            ..
        } => RuntimeEvent::GateStarted {
            run_id: run_id.clone(),
            gate_name: runtime_gate_name(*kind, attempt),
            rung: (*rung).min(u32::from(u8::MAX)) as u8,
        },
        RunnerEvent::GateCompleted {
            run_id,
            attempt,
            kind,
            passed: true,
            duration_ms,
            ..
        } => RuntimeEvent::GatePassed {
            run_id: run_id.clone(),
            gate_name: runtime_gate_name(*kind, attempt),
            duration_ms: *duration_ms,
        },
        RunnerEvent::GateCompleted {
            run_id,
            attempt,
            kind,
            output,
            duration_ms,
            ..
        } => RuntimeEvent::GateFailed {
            run_id: run_id.clone(),
            gate_name: runtime_gate_name(*kind, attempt),
            output: output.clone(),
            duration_ms: *duration_ms,
        },
        // ── Progress variants ────────────────────────────────────────────
        RunnerEvent::TaskAttemptStarted {
            run_id,
            attempt,
            title,
            ..
        } => RuntimeEvent::TaskStarted {
            run_id: run_id.clone(),
            plan_id: attempt.plan_id.clone(),
            task_id: attempt.task_id.clone(),
            task_title: title.clone(),
            role: String::new(),
        },
        RunnerEvent::TaskAttemptCompleted {
            run_id,
            attempt,
            outcome,
            duration_ms,
            ..
        } => RuntimeEvent::TaskCompleted {
            run_id: run_id.clone(),
            plan_id: attempt.plan_id.clone(),
            task_id: attempt.task_id.clone(),
            passed: matches!(outcome, TaskAttemptOutcome::Passed),
            duration_ms: *duration_ms,
        },
        RunnerEvent::PlanStarted {
            run_id, plan_id, ..
        } => RuntimeEvent::PipelinePhase {
            run_id: run_id.clone(),
            phase: plan_id.clone(),
            status: "started".to_string(),
        },
        RunnerEvent::PlanCompleted {
            run_id,
            plan_id,
            outcome,
            ..
        } => {
            let status = match outcome {
                PlanOutcome::Succeeded => "complete",
                PlanOutcome::Failed => "failed",
                PlanOutcome::Skipped => "skipped",
            };
            RuntimeEvent::PipelinePhase {
                run_id: run_id.clone(),
                phase: plan_id.clone(),
                status: status.to_string(),
            }
        }
        _ => RuntimeEvent::FeedbackRecorded {
            run_id: runner_event_run_id(event).to_string(),
            kind: event.event_type().to_string(),
            summary: event.message(),
        },
    }
}

fn runtime_workflow_outcome(outcome: RunOutcome) -> RuntimeWorkflowOutcome {
    match outcome {
        RunOutcome::Succeeded => RuntimeWorkflowOutcome::Success { commit_hash: None },
        RunOutcome::Failed => RuntimeWorkflowOutcome::Halted {
            reason: "runner failed".to_string(),
        },
        RunOutcome::Cancelled => RuntimeWorkflowOutcome::Cancelled,
    }
}

fn agent_completion_succeeded(outcome: AgentDispatchOutcome, exit_code: Option<i32>) -> bool {
    matches!(outcome, AgentDispatchOutcome::Completed)
        || (matches!(outcome, AgentDispatchOutcome::Exited) && exit_code.unwrap_or(0) == 0)
}

fn runtime_gate_name(kind: GateCompletionKind, attempt: &TaskAttemptRef) -> String {
    let kind = match kind {
        GateCompletionKind::Gate => "gate",
        GateCompletionKind::PlanVerify => "plan_verify",
        GateCompletionKind::Merge => "merge",
    };
    format!(
        "{kind}:{}:{}",
        attempt.plan_id.as_str(),
        attempt.task_id.as_str()
    )
}

fn runner_event_run_id(event: &RunnerEvent) -> &str {
    match event {
        RunnerEvent::ResumeMarker { run_id, .. }
        | RunnerEvent::RunStarted { run_id, .. }
        | RunnerEvent::RunCompleted { run_id, .. }
        | RunnerEvent::PlanStarted { run_id, .. }
        | RunnerEvent::PlanCompleted { run_id, .. }
        | RunnerEvent::TaskAttemptStarted { run_id, .. }
        | RunnerEvent::TaskAttemptCompleted { run_id, .. }
        | RunnerEvent::AgentDispatchStarted { run_id, .. }
        | RunnerEvent::AgentDispatchCompleted { run_id, .. }
        | RunnerEvent::AgentCompleted { run_id, .. }
        | RunnerEvent::GateDispatchStarted { run_id, .. }
        | RunnerEvent::GateCompleted { run_id, .. }
        | RunnerEvent::PromptAssembled { run_id, .. }
        | RunnerEvent::MergeBackendCompleted { run_id, .. }
        | RunnerEvent::RetryDecision { run_id, .. } => run_id,
    }
}

/// Per-task usage snapshot captured just before emitting feedback.
/// Carries the accumulated token / cost / timing data from [`RunState`]
/// so that `runner_event_to_feedback` does not have to zero-fill those
/// fields.
#[derive(Debug, Clone, Default)]
struct TaskUsageSnapshot {
    tokens_in: u64,
    tokens_out: u64,
    cost_usd: f64,
    duration_ms: u64,
    prompt_text: Option<String>,
    agent_output: String,
}

#[derive(Debug, Clone)]
struct DaimonTaskHook {
    strategy: StrategyCoordinates,
    signal: SomaticSignal,
    affect_confidence: f64,
    behavioral_state: roko_core::BehavioralState,
    pleasure: f64,
    arousal: f64,
    dominance: f64,
}

#[derive(Debug, Clone)]
struct DaimonDispatchModulation {
    model: String,
    turn_limit: u32,
    effort: String,
}

fn with_daimon_state<T>(
    config: &RunConfig,
    f: impl FnOnce(&mut roko_daimon::DaimonState) -> T,
) -> Option<T> {
    let daimon_state = config.daimon_state.as_ref()?;
    match daimon_state.lock() {
        Ok(mut guard) => Some(f(&mut guard)),
        Err(err) => {
            warn!(error = %err, "daimon state lock poisoned; skipping affect hook");
            None
        }
    }
}

fn daimon_task_hook(
    config: &RunConfig,
    task_def: &TaskDef,
    attempt_num: u32,
) -> Option<DaimonTaskHook> {
    with_daimon_state(config, |daimon| {
        let affect = daimon.query();
        let observation = TaskStrategyObservation {
            task_tier: task_def.tier.clone(),
            file_count: task_def.files.len(),
            verification_count: task_def.verify.len(),
            dependency_count: task_def.depends_on.len(),
            max_loc: task_def.max_loc.unwrap_or(50),
            familiarity: 0.5,
            confidence: affect.confidence,
            failure_pressure: f64::from(attempt_num.saturating_sub(1).min(5)) / 5.0,
            urgency_pressure: if attempt_num > 1 { 1.0 } else { 0.0 },
        };
        let strategy = daimon.strategy_space().computer().task_coords(&observation);
        let signal = daimon.query_somatic(strategy);
        if signal.should_emit_event() {
            info!(
                task_id = %task_def.id,
                valence = signal.valence,
                intensity = signal.intensity,
                source_episodes = signal.source_episodes.len(),
                "daimon somatic marker fired"
            );
        }
        DaimonTaskHook {
            strategy,
            signal,
            affect_confidence: affect.confidence,
            behavioral_state: affect.behavioral_state,
            pleasure: affect.pad.pleasure,
            arousal: affect.pad.arousal,
            dominance: affect.pad.dominance,
        }
    })
}

fn daimon_policy_for_hook(hook: Option<&DaimonTaskHook>) -> roko_core::DaimonPolicy {
    hook.map(|hook| roko_core::DaimonPolicy::new(hook.affect_confidence, hook.behavioral_state))
        .unwrap_or_default()
}

fn daimon_dispatch_modulation(
    config: &RunConfig,
    hook: &DaimonTaskHook,
    selected_model: &str,
    allow_model_modulation: bool,
) -> Option<DaimonDispatchModulation> {
    with_daimon_state(config, |daimon| {
        let mut params = DispatchParams::new(selected_model.to_string(), DEFAULT_AGENT_TURN_LIMIT);
        params.effort = default_effort_label(config);
        daimon.modulate_with_strategy(&mut params, hook.strategy);
        if !allow_model_modulation {
            params.model = selected_model.to_string();
        }
        DaimonDispatchModulation {
            model: params.model,
            turn_limit: params.turn_limit.max(1),
            effort: params.effort,
        }
    })
}

fn default_effort_label(config: &RunConfig) -> String {
    config
        .roko_config
        .as_ref()
        .map(|config| config.agent.default_effort.trim())
        .filter(|effort| !effort.is_empty())
        .unwrap_or("medium")
        .to_string()
}

fn render_daimon_prompt_context(hook: &DaimonTaskHook) -> Option<String> {
    let pad_magnitude =
        hook.pleasure.abs() + hook.arousal.abs() + hook.dominance.abs() + hook.signal.intensity;
    if pad_magnitude < 0.35 {
        return None;
    }

    let mut content = format!(
        "# Daimon state\nBehavioral state: {:?}\nPAD: pleasure={:.2}, arousal={:.2}, dominance={:.2}",
        hook.behavioral_state, hook.pleasure, hook.arousal, hook.dominance
    );
    if hook.signal.intensity >= 0.15 {
        content.push_str(&format!(
            "\nSomatic hint: valence={:.2}, intensity={:.2}",
            hook.signal.valence, hook.signal.intensity
        ));
        if hook.signal.valence <= -0.2 {
            content
                .push_str("\nInterpretation: slow down, prefer caution, and verify risky moves.");
        } else if hook.signal.valence >= 0.2 {
            content.push_str(
                "\nInterpretation: this strategy region has positive prior outcomes; keep momentum without skipping checks.",
            );
        }
    }
    Some(content)
}

fn record_daimon_gate_result(config: &RunConfig, completion: &GateCompletion) {
    with_daimon_state(config, |daimon| {
        daimon.appraise(AffectEvent::GateResult {
            plan_id: completion.plan_id.clone(),
            task_id: completion.task_id.clone(),
            passed: completion.passed,
            rung: completion.rung,
        });
    });
}

fn record_daimon_task_outcome(
    config: &RunConfig,
    strategy: Option<StrategyCoordinates>,
    plan_id: &str,
    task_id: &str,
    succeeded: bool,
    discriminator: &str,
) {
    with_daimon_state(config, |daimon| {
        daimon.appraise(AffectEvent::TaskOutcome {
            task_id: task_id.to_string(),
            succeeded,
        });
        if let Some(strategy) = strategy {
            daimon.record_somatic_outcome(
                strategy,
                somatic_episode_hash(
                    plan_id,
                    task_id,
                    if succeeded { "success" } else { "failure" },
                    discriminator,
                ),
            );
        }
    });
}

fn somatic_episode_hash(
    plan_id: &str,
    task_id: &str,
    outcome: &str,
    discriminator: &str,
) -> ContentHash {
    ContentHash::of(format!("somatic:{plan_id}:{task_id}:{outcome}:{discriminator}").as_bytes())
}

/// Translate a [`RunnerEvent`] into a [`FeedbackEvent`] when the runner
/// has enough information for one. Returns `None` for variants that do
/// not map to the feedback layer (e.g. `RunStarted`, `ResumeMarker`).
///
/// `routing_ctx` is the dispatch-time routing context stored on
/// [`RunState`] — threaded here so `TaskCompleted` events carry the
/// real feature vector for the CascadeRouter's bandit.
fn runner_event_to_feedback(
    event: &RunnerEvent,
    routing_ctx: &Option<roko_learn::model_router::RoutingContext>,
    usage: &TaskUsageSnapshot,
) -> Option<crate::runtime_feedback::FeedbackEvent> {
    use crate::dispatch::{AgentOutcome, ModelChoiceSource};
    use crate::runtime_feedback::FeedbackEvent;

    match event {
        RunnerEvent::TaskAttemptCompleted {
            attempt,
            outcome,
            model,
            provider,
            ..
        } => {
            if model.trim().is_empty() {
                return None;
            }

            let succeeded = matches!(outcome, TaskAttemptOutcome::Passed);
            let agent_outcome = AgentOutcome {
                task_id: attempt.task_id.clone(),
                plan_id: attempt.plan_id.clone(),
                model: model.clone(),
                provider: provider.clone(),
                output: usage.agent_output.clone(),
                tokens_in: usage.tokens_in,
                tokens_out: usage.tokens_out,
                cost_usd: usage.cost_usd,
                duration_ms: usage.duration_ms,
                exit_code: None,
                is_error: !succeeded,
            };
            Some(FeedbackEvent::TaskCompleted {
                plan_id: attempt.plan_id.clone(),
                task_id: attempt.task_id.clone(),
                outcome: agent_outcome,
                model_source: ModelChoiceSource::Default,
                succeeded,
                routing_context: routing_ctx.clone(),
                prompt_text: usage.prompt_text.clone(),
            })
        }
        RunnerEvent::GateCompleted {
            attempt,
            rung,
            passed,
            duration_ms,
            ..
        } => Some(FeedbackEvent::GateOutcome {
            plan_id: attempt.plan_id.clone(),
            task_id: attempt.task_id.clone(),
            rung: *rung,
            passed: *passed,
            duration_ms: *duration_ms,
        }),
        RunnerEvent::RetryDecision {
            attempt,
            cooldown_ms,
            current_attempt,
            ..
        } => Some(FeedbackEvent::RetryDecision {
            plan_id: attempt.plan_id.clone(),
            task_id: attempt.task_id.clone(),
            attempt: *current_attempt,
            backoff_secs: cooldown_ms / 1000,
        }),
        RunnerEvent::PlanCompleted {
            plan_id, outcome, ..
        } => {
            let succeeded = matches!(outcome, PlanOutcome::Succeeded);
            Some(FeedbackEvent::PlanCompleted {
                plan_id: plan_id.clone(),
                succeeded,
                tasks_completed: 0,
                tasks_failed: 0,
                total_cost_usd: 0.0,
            })
        }
        _ => None,
    }
}

fn build_run_completed_event(
    executor: &ParallelExecutor,
    plans: &[Plan],
    state: &RunState,
    outcome: RunOutcome,
) -> RunnerEvent {
    let report = build_report(executor, plans, state);
    RunnerEvent::run_completed(
        state.run_id(),
        outcome,
        RunTotals {
            total_tasks: report.total_tasks,
            tasks_completed: report.tasks_completed,
            tasks_failed: report.tasks_failed,
            total_agent_calls: report.total_agent_calls,
            total_cost_usd: report.total_cost_usd,
            duration_ms: report.duration.as_millis() as u64,
        },
        report
            .plans
            .into_iter()
            .map(|plan| PlanRunSummary {
                plan_id: plan.plan_id,
                completed: plan.completed,
                tasks_total: plan.tasks_total,
                tasks_completed: plan.tasks_completed,
                tasks_failed: plan.tasks_failed,
            })
            .collect(),
    )
}

// ─── Snapshot Helper ────────────────────────────────────────────────────

/// Build a unified [`StateSnapshot`] from all four state groups (executor,
/// orchestrator, run counters, gate thresholds) with a SHA-256 checksum,
/// then enqueue the serialized blob on the async writer. Serialisation
/// (<1ms) happens on the caller's thread; the single atomic disk write
/// runs on the dedicated writer thread.
fn save_snapshot(
    config: &RunConfig,
    executor: &ParallelExecutor,
    paths: &PersistPaths,
    state: &mut RunState,
    merge_queue: &MergeQueue,
    gate_thresholds: &GateThresholds,
    writer: &SnapshotWriter,
) {
    let timestamp_ms = chrono::Utc::now().timestamp_millis() as u64;
    let snapshot = executor.snapshot(timestamp_ms);
    let orchestrator_snapshot = OrchestratorSnapshot::new(snapshot.clone(), timestamp_ms)
        .with_merge_queue(merge_queue.snapshot());

    let orchestrator_json = match orchestrator_snapshot.to_json() {
        Ok(json) => json,
        Err(e) => {
            error!(error = %e, "failed to serialize orchestrator snapshot");
            state.snapshot_failed();
            return;
        }
    };
    let executor_json = match serde_json::to_string_pretty(&snapshot) {
        Ok(json) => json,
        Err(e) => {
            error!(error = %e, "failed to serialize executor snapshot");
            state.snapshot_failed();
            return;
        }
    };

    let run_state = persist::RunStateSnapshot {
        schema_version: persist::RUN_STATE_SCHEMA_VERSION,
        run_id: state.run_id().to_string(),
        started_at_ms: state.start_epoch_ms,
        timestamp_ms,
        tasks_total: state.tasks_total,
        tasks_completed: state.tasks_completed,
        tasks_failed: state.tasks_failed,
        total_tokens_in: state.total_tokens_in,
        total_tokens_out: state.total_tokens_out,
        total_cost_usd: state.total_cost_usd,
        total_agent_calls: state.total_agent_calls,
        plan_costs: state.plan_costs.clone(),
        completed_tasks: state.completed_tasks.clone(),
        snapshot_fail_streak: state.snapshot_fail_streak,
        fingerprints: state.task_fingerprints.clone(),
        cascade_router_json: config
            .cascade_router
            .as_ref()
            .map(|router| router.snapshot_json()),
    };
    let run_state_json = match serde_json::to_string_pretty(&run_state) {
        Ok(json) => json,
        Err(e) => {
            error!(error = %e, "failed to serialize run-state snapshot");
            state.snapshot_failed();
            return;
        }
    };
    let gate_thresholds_json = match serde_json::to_string_pretty(gate_thresholds) {
        Ok(json) => json,
        Err(e) => {
            error!(error = %e, "failed to serialize gate thresholds");
            state.snapshot_failed();
            return;
        }
    };

    let unified = roko_runtime::StateSnapshot::new(
        timestamp_ms,
        executor_json,
        orchestrator_json,
        run_state_json,
        gate_thresholds_json,
    );

    let snapshot_json = match serde_json::to_vec_pretty(&unified) {
        Ok(json) => json,
        Err(e) => {
            error!(error = %e, "failed to serialize unified state snapshot");
            state.snapshot_failed();
            return;
        }
    };

    writer.write(SnapshotPayload {
        snapshot_json,
        snapshot_path: paths.state_snapshot_json.clone(),
    });
}

fn restore_state_from_resume_snapshot(
    state: &mut RunState,
    snapshot: &persist::RunStateSnapshot,
    task_index: &HashMap<String, HashMap<String, TaskDef>>,
    drifted_tasks: &[super::resume::DriftedTask],
) {
    state.tasks_failed = snapshot.tasks_failed;
    state.total_tokens_in = snapshot.total_tokens_in;
    state.total_tokens_out = snapshot.total_tokens_out;
    state.total_cost_usd = snapshot.total_cost_usd;
    state.total_agent_calls = snapshot.total_agent_calls;
    state.plan_costs = snapshot.plan_costs.clone();
    state.snapshot_fail_streak = snapshot.snapshot_fail_streak;
    state.completed_tasks = snapshot.completed_tasks.clone();
    state.completed_tasks.retain(|plan_id, completed| {
        let Some(tasks) = task_index.get(plan_id) else {
            return false;
        };
        completed.retain(|task_id| tasks.contains_key(task_id));
        !completed.is_empty()
    });

    let mut requeued_count = 0usize;
    for drifted in drifted_tasks {
        if let Some(completed) = state.completed_tasks.get_mut(&drifted.plan_id) {
            let before = completed.len();
            completed.retain(|task_id| task_id != &drifted.task_id);
            if completed.len() != before {
                requeued_count += 1;
                warn!(
                    plan = %drifted.plan_id,
                    task = %drifted.task_id,
                    "task definition drifted since snapshot — re-queuing"
                );
                info!(
                    plan = %drifted.plan_id,
                    task = %drifted.task_id,
                    "re-queued (definition changed)"
                );
            }
        }
    }

    if requeued_count > 0 {
        warn!(
            drifted_count = requeued_count,
            "detected drifted tasks — completed ones were re-queued"
        );
    }

    state.tasks_completed = state.completed_tasks.values().map(Vec::len).sum::<usize>();
}

// ─── Resume ─────────────────────────────────────────────────────────────

struct ResumeLoad {
    executor: ParallelExecutor,
    merge_queue: MergeQueue,
    marker: ResumeMarker,
}

/// Load a resumable executor snapshot when compatible, otherwise start fresh
/// and emit a structured resume marker explaining the decision.
fn load_executor(paths: &PersistPaths, config: &ExecutorConfig, plan_ids: &[String]) -> ResumeLoad {
    let (snapshot, merge_queue, snapshot_path) = match load_orchestrator_checkpoint(paths) {
        Ok(Some((snapshot, merge_queue))) => (
            snapshot,
            merge_queue,
            paths.orchestrator_json.display().to_string(),
        ),
        Ok(None) => match load_unified_state_checkpoint(paths) {
            Ok(Some((snapshot, merge_queue))) => (
                snapshot,
                merge_queue,
                paths.state_snapshot_json.display().to_string(),
            ),
            Ok(None) => match load_legacy_executor_checkpoint(paths) {
                Ok(Some(snapshot)) => (
                    snapshot,
                    MergeQueue::new(),
                    paths.executor_json.display().to_string(),
                ),
                Ok(None) => {
                    return ResumeLoad {
                        executor: ParallelExecutor::new(config.clone()),
                        merge_queue: MergeQueue::new(),
                        marker: ResumeMarker {
                            outcome: ResumeOutcome::Fresh,
                            snapshot_path: paths.state_snapshot_json.display().to_string(),
                            snapshot_plan_ids: Vec::new(),
                            current_plan_ids: plan_ids.to_vec(),
                            message: Some("no executor snapshot found".to_string()),
                        },
                    };
                }
                Err(e) => {
                    let snapshot_path = paths.executor_json.display().to_string();
                    warn!(err = %e, "failed to load legacy executor snapshot");
                    return fresh_after_snapshot_error(
                        Some((snapshot_path, ResumeOutcome::Corrupt, e)),
                        config,
                        plan_ids,
                    );
                }
            },
            Err(e) => {
                let snapshot_path = paths.state_snapshot_json.display().to_string();
                warn!(err = %e, "failed to load unified state snapshot");
                let first_error = (snapshot_path, ResumeOutcome::Corrupt, e);
                match load_legacy_executor_checkpoint(paths) {
                    Ok(Some(snapshot)) => (
                        snapshot,
                        MergeQueue::new(),
                        paths.executor_json.display().to_string(),
                    ),
                    Ok(None) => {
                        return fresh_after_snapshot_error(Some(first_error), config, plan_ids);
                    }
                    Err(e) => {
                        warn!(err = %e, "failed to load legacy executor snapshot");
                        return fresh_after_snapshot_error(Some(first_error), config, plan_ids);
                    }
                }
            }
        },
        Err(e) => {
            let snapshot_path = paths.orchestrator_json.display().to_string();
            warn!(err = %e, "failed to load orchestrator snapshot");
            let first_error = (snapshot_path, ResumeOutcome::Corrupt, e);
            match load_unified_state_checkpoint(paths) {
                Ok(Some((snapshot, merge_queue))) => (
                    snapshot,
                    merge_queue,
                    paths.state_snapshot_json.display().to_string(),
                ),
                Ok(None) => match load_legacy_executor_checkpoint(paths) {
                    Ok(Some(snapshot)) => (
                        snapshot,
                        MergeQueue::new(),
                        paths.executor_json.display().to_string(),
                    ),
                    Ok(None) => {
                        return fresh_after_snapshot_error(Some(first_error), config, plan_ids);
                    }
                    Err(e) => {
                        warn!(err = %e, "failed to load legacy executor snapshot");
                        return fresh_after_snapshot_error(Some(first_error), config, plan_ids);
                    }
                },
                Err(e) => {
                    warn!(err = %e, "failed to load unified state snapshot");
                    match load_legacy_executor_checkpoint(paths) {
                        Ok(Some(snapshot)) => (
                            snapshot,
                            MergeQueue::new(),
                            paths.executor_json.display().to_string(),
                        ),
                        Ok(None) => {
                            return fresh_after_snapshot_error(Some(first_error), config, plan_ids);
                        }
                        Err(e) => {
                            warn!(err = %e, "failed to load legacy executor snapshot");
                            return fresh_after_snapshot_error(Some(first_error), config, plan_ids);
                        }
                    }
                }
            }
        }
    };

    // Validate: snapshot must contain at least one of the current plan IDs.
    let snap_plan_ids: Vec<String> = snapshot.plan_states.keys().cloned().collect();
    let has_overlap = plan_ids
        .iter()
        .any(|id| snapshot.plan_states.contains_key(id));

    if snap_plan_ids.is_empty() || !has_overlap {
        info!(
            snapshot_plans = ?snap_plan_ids,
            current_plans = ?plan_ids,
            "stale executor snapshot (no plan overlap) — starting fresh"
        );
        return ResumeLoad {
            executor: ParallelExecutor::new(config.clone()),
            merge_queue: MergeQueue::new(),
            marker: ResumeMarker {
                outcome: ResumeOutcome::IgnoredStale,
                snapshot_path,
                snapshot_plan_ids: snap_plan_ids,
                current_plan_ids: plan_ids.to_vec(),
                message: Some("snapshot has no overlap with current plans".to_string()),
            },
        };
    }

    info!(
        path = %snapshot_path,
        plans = ?snap_plan_ids,
        "resuming from executor snapshot"
    );
    let mut executor = ParallelExecutor::from_snapshot(config.clone(), snapshot.clone());
    let recovery = RecoveryEngine::new().recover_from_executor_snapshot(snapshot);
    let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let resume_plan = recovery.resume_plan(now_ms);
    for warning in &resume_plan.warnings {
        warn!(
            plan_id = %warning.plan_id,
            message = %warning.message,
            severity = ?warning.severity,
            "orchestrator recovery warning"
        );
    }
    for plan in &resume_plan.retryable_terminal {
        if executor.requeue_retryable_terminal(&plan.plan_id).is_some() {
            info!(
                plan_id = %plan.plan_id,
                retry_after_ms = ?plan.retry_after_ms,
                "requeued retryable terminal plan after recovery"
            );
        }
    }
    ResumeLoad {
        executor,
        merge_queue,
        marker: ResumeMarker {
            outcome: ResumeOutcome::Resumed,
            snapshot_path,
            snapshot_plan_ids: snap_plan_ids,
            current_plan_ids: plan_ids.to_vec(),
            message: Some("resumed from compatible executor snapshot".to_string()),
        },
    }
}

fn fresh_after_snapshot_error(
    first_error: Option<(String, ResumeOutcome, String)>,
    config: &ExecutorConfig,
    plan_ids: &[String],
) -> ResumeLoad {
    let (snapshot_path, outcome, message) = first_error.unwrap_or_else(|| {
        (
            String::new(),
            ResumeOutcome::ReadFailed,
            "failed to load executor snapshot".to_string(),
        )
    });
    ResumeLoad {
        executor: ParallelExecutor::new(config.clone()),
        merge_queue: MergeQueue::new(),
        marker: ResumeMarker {
            outcome,
            snapshot_path,
            snapshot_plan_ids: Vec::new(),
            current_plan_ids: plan_ids.to_vec(),
            message: Some(message),
        },
    }
}

fn load_orchestrator_checkpoint(
    paths: &PersistPaths,
) -> Result<Option<(ExecutorSnapshot, MergeQueue)>, String> {
    let json = match std::fs::read_to_string(&paths.orchestrator_json) {
        Ok(j) => j,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("failed to read aggregate snapshot: {e}")),
    };
    let snapshot = OrchestratorSnapshot::from_json(&json)
        .map_err(|err| format!("failed to parse aggregate snapshot: {err}"))?;
    let merge_queue = snapshot
        .merge_queue
        .map(MergeQueue::from_snapshot)
        .unwrap_or_else(MergeQueue::new);
    Ok(Some((snapshot.executor, merge_queue)))
}

fn load_unified_state_checkpoint(
    paths: &PersistPaths,
) -> Result<Option<(ExecutorSnapshot, MergeQueue)>, String> {
    let unified = persist::load_state_snapshot(paths)
        .map_err(|err| format!("failed to read unified state snapshot: {err}"))?;
    let Some(unified) = unified else {
        return Ok(None);
    };

    if !unified.orchestrator_json.trim().is_empty() {
        match OrchestratorSnapshot::from_json(&unified.orchestrator_json) {
            Ok(snapshot) => {
                let merge_queue = snapshot
                    .merge_queue
                    .map(MergeQueue::from_snapshot)
                    .unwrap_or_else(MergeQueue::new);
                return Ok(Some((snapshot.executor, merge_queue)));
            }
            Err(err) => {
                warn!(
                    error = %err,
                    "failed to parse orchestrator_json from unified state snapshot; trying executor_json"
                );
            }
        }
    }

    let snapshot = ExecutorSnapshot::from_json(&unified.executor_json).map_err(|err| {
        format!("failed to parse executor_json from unified state snapshot: {err}")
    })?;
    Ok(Some((snapshot, MergeQueue::new())))
}

fn load_legacy_executor_checkpoint(
    paths: &PersistPaths,
) -> Result<Option<ExecutorSnapshot>, String> {
    let json = match std::fs::read_to_string(&paths.executor_json) {
        Ok(json) => json,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("failed to read executor snapshot: {e}")),
    };
    ExecutorSnapshot::from_json(&json)
        .map(Some)
        .map_err(|err| format!("corrupt executor snapshot: {err}"))
}

// ─── Action Dispatcher ──────────────────────────────────────────────────

fn record_skipped_gate_rung(
    ctx: &mut RunContext<'_>,
    plan_id: &str,
    task_id: &str,
    rung: u32,
    gate_name: &str,
    summary: &str,
) {
    if let Some(plan_state) = ctx.executor.plan_state_mut(plan_id) {
        plan_state.gate_results.push(GateResult {
            gate_name: gate_name.to_string(),
            rung,
            passed: true,
            summary: summary.to_string(),
            duration_ms: 0,
            test_count: None,
        });
    }
    ctx.tui.gate_result(plan_id, task_id, gate_name, true);

    if rung >= ctx.config.max_gate_rung {
        if let Err(err) = ctx
            .executor
            .apply_event(plan_id, &ExecutorEvent::GatePassed)
        {
            warn!(plan_id = %plan_id, rung, error = %err, "failed to advance after skipped final gate");
        }
    } else {
        debug!(
            plan_id = %plan_id,
            task_id = %task_id,
            rung,
            max_gate_rung = ctx.config.max_gate_rung,
            "skipped gate rung recorded; advancing to next rung"
        );
    }
}

fn gates_config_for_run(config: &RunConfig) -> GatesConfig {
    let mut gates = config
        .roko_config
        .as_ref()
        .map(|roko_config| roko_config.gates.clone())
        .unwrap_or_default();
    if !gates.has_custom_rungs() {
        gates.clippy_enabled = config.clippy_enabled;
        gates.skip_tests = config.skip_tests;
    }
    gates
}

fn gate_plan_complexity_for_task(task_def: Option<&TaskDef>) -> PlanComplexity {
    match task_def.map(|task| task.tier.as_str()).unwrap_or("focused") {
        "mechanical" | "fast" => PlanComplexity::Trivial,
        "focused" => PlanComplexity::Simple,
        "integrative" => PlanComplexity::Standard,
        "architectural" | "complex" | "premium" => PlanComplexity::Complex,
        _ => PlanComplexity::Simple,
    }
}

fn task_role_is_read_only(task_def: Option<&TaskDef>) -> bool {
    task_def
        .and_then(|task| task.role.as_deref())
        .map_or(false, |role| {
            matches!(role, "researcher" | "strategist" | "quick-reviewer")
        })
}

fn task_should_preflight_verify(task_def: &TaskDef, attempt_num: u32) -> bool {
    attempt_num == 1 && !task_def.verify.is_empty() && !task_role_is_read_only(Some(task_def))
}

async fn dispatch_action(
    action: &ExecutorAction,
    ctx: &mut RunContext<'_>,
) -> ActionDispatchOutcome {
    match action {
        ExecutorAction::DispatchPlan { plan_id } => {
            info!(plan_id = %plan_id, "dispatching plan");
            ctx.tui.plan_started(plan_id);

            if let Err(e) = ctx.executor.apply_event(plan_id, &ExecutorEvent::Start) {
                error!(plan_id = %plan_id, err = %e, "failed to start plan");
                return ActionDispatchOutcome::Noop;
            }
            let run_id = ctx.state.run_id().to_string();
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                ctx.config,
                RunnerEvent::plan_started(&run_id, plan_id),
            );

            if ctx
                .skip_enrichment
                .get(plan_id.as_str())
                .copied()
                .unwrap_or(false)
            {
                if let Err(e) = ctx
                    .executor
                    .apply_event(plan_id, &ExecutorEvent::EnrichmentDone)
                {
                    error!(plan_id = %plan_id, err = %e, "failed to skip enrichment");
                }
                ctx.tui
                    .phase_transition(plan_id, "enriching", "implementing");
            }
            ActionDispatchOutcome::Handled
        }

        ExecutorAction::SpawnAgent { plan_id, task, .. } => {
            // Resolve sentinel task names ("next", "fix", etc.) to actual task IDs
            // by walking the plan's DAG and finding the first ready task.
            let resolved_task = if task == "next" || task == "fix" || task == "regen-verify" {
                let completed = ctx.state.plan_completed_tasks(plan_id);
                let failed = ctx.state.plan_failed_tasks(plan_id);
                let completed_plans = completed_plan_ids(ctx.executor, ctx.task_index);
                let plan_tasks = ctx.task_index.get(plan_id.as_str());
                plan_tasks.and_then(|tasks| {
                    // Collect all TaskDefs, then find the first ready one in definition order.
                    let mut all_tasks: Vec<&TaskDef> = tasks.values().collect();
                    all_tasks.sort_by_key(|t| t.sequence);
                    all_tasks
                        .iter()
                        .find(|t| {
                            !completed.contains(&t.id)
                                && !failed.contains(&t.id)
                                && t.is_ready_with_plan_deps(completed, &completed_plans)
                        })
                        .map(|t| t.id.clone())
                })
            } else if matches!(task.as_str(), "review" | "doc-revision" | "docs" | "enrich") {
                if ctx.state.current_task.is_empty() {
                    ctx.task_index
                        .get(plan_id.as_str())
                        .and_then(|tasks| tasks.values().min_by_key(|t| t.sequence))
                        .map(|t| t.id.clone())
                } else {
                    Some(ctx.state.current_task.clone())
                }
            } else {
                Some(task.clone())
            };

            let task_id = match resolved_task {
                Some(id) => id,
                None => {
                    // No more ready tasks — all done for this plan.
                    info!(plan_id = %plan_id, "no more ready tasks — implementation complete");
                    let _ = ctx
                        .executor
                        .apply_event(plan_id, &ExecutorEvent::ImplementationDone);
                    return ActionDispatchOutcome::Noop;
                }
            };

            if let Some(active_task) = ctx.active_agent_tasks.get(plan_id.as_str()) {
                debug!(
                    plan_id = %plan_id,
                    task = %task_id,
                    active_task = %active_task,
                    "agent already active for this plan — suppressing duplicate spawn"
                );
                return ActionDispatchOutcome::Noop;
            }

            if ctx.agent_handles.contains_key(plan_id.as_str()) {
                debug!(
                    plan_id = %plan_id,
                    task = %task_id,
                    "agent already active for this plan — suppressing duplicate spawn"
                );
                return ActionDispatchOutcome::Noop;
            }

            if let Some(remaining) = ctx.state.retry_cooldown_remaining(plan_id) {
                debug!(
                    plan_id = %plan_id,
                    task = %task_id,
                    cooldown_ms = remaining.as_millis(),
                    "retry backoff active — delaying spawn"
                );
                return ActionDispatchOutcome::Noop;
            }

            info!(plan_id = %plan_id, task = %task_id, "spawning agent");

            // Per-plan budget check.
            let max_plan_usd = ctx.config.max_plan_usd;
            let plan_spent = ctx.state.plan_cost(plan_id);
            if max_plan_usd > 0.0 && plan_spent >= max_plan_usd {
                warn!(
                    plan_id = %plan_id,
                    spent = plan_spent,
                    limit = max_plan_usd,
                    "plan budget exceeded — aborting"
                );
                ctx.tui.error(&format!(
                    "budget exceeded: ${plan_spent:.2} >= ${max_plan_usd:.2}"
                ));
                if let Err(e) = ctx.executor.apply_event(
                    plan_id,
                    &ExecutorEvent::Fatal(format!(
                        "budget exceeded: ${plan_spent:.2} >= ${max_plan_usd:.2}"
                    )),
                ) {
                    error!(plan_id = %plan_id, error = %e,
                        "failed to apply Fatal event -- forcing plan terminal");
                    ctx.state.force_plan_terminal(plan_id);
                }
                return ActionDispatchOutcome::Noop;
            }

            let task_def = match ctx
                .task_index
                .get(plan_id.as_str())
                .and_then(|m| m.get(task_id.as_str()))
            {
                Some(t) => t,
                None => {
                    error!(plan_id = %plan_id, task = %task_id, "task not found in index");
                    if let Err(e) = ctx.executor.apply_event(
                        plan_id,
                        &ExecutorEvent::Fatal(format!("task {task_id} not found")),
                    ) {
                        error!(plan_id = %plan_id, error = %e,
                            "failed to apply Fatal event -- forcing plan terminal");
                        ctx.state.force_plan_terminal(plan_id);
                    }
                    return ActionDispatchOutcome::Noop;
                }
            };

            let previous_gate_output = ctx.state.gate_output.clone();
            let attempt_num = ctx
                .executor
                .plan_state(plan_id)
                .map(|state| state.iteration)
                .unwrap_or(1);
            ctx.state.reset_for_task(plan_id, &task_id);
            ctx.state.set_iteration(plan_id, &task_id, attempt_num);
            let role = task_def.role.as_deref().unwrap_or("implementer");

            if task_should_preflight_verify(task_def, attempt_num) {
                let gates_config = gates_config_for_run(ctx.config);
                let has_cargo_toml =
                    std::fs::metadata(ctx.config.workdir.join("Cargo.toml")).is_ok();
                if gates_config.has_custom_rungs() || has_cargo_toml {
                    let pipeline_rung = ctx.config.max_gate_rung;
                    info!(
                        plan_id = %plan_id,
                        task = %task_id,
                        rung = pipeline_rung,
                        "running task verification preflight before agent dispatch"
                    );
                    let preflight = gate_dispatch::run_gate_once(
                        plan_id.clone(),
                        task_id.clone(),
                        pipeline_rung,
                        ctx.config.workdir.clone(),
                        gates_config,
                        gate_plan_complexity_for_task(Some(task_def)),
                        task_def.verify.clone(),
                        duration_secs(gate_timeout(ctx.config, pipeline_rung)),
                        task_target_crates(Some(task_def)),
                    )
                    .await;

                    if preflight.passed {
                        info!(
                            plan_id = %plan_id,
                            task = %task_id,
                            duration_ms = preflight.duration_ms,
                            "task verification already passes -- skipping agent"
                        );
                        ctx.sink.task_started(
                            plan_id,
                            &task_id,
                            role,
                            &task_def.title,
                            attempt_num,
                        );
                        ctx.tui
                            .task_started(plan_id, &task_id, &task_def.title, "verifying");
                        let attempt_ref =
                            TaskAttemptRef::new(plan_id.clone(), task_id.clone(), attempt_num);
                        let run_id = ctx.state.run_id().to_string();
                        emit_runner_event(
                            ctx.paths,
                            ctx.state,
                            ctx.tui,
                            ctx.config,
                            RunnerEvent::task_attempt_started(
                                &run_id,
                                attempt_ref.clone(),
                                &task_def.title,
                            ),
                        );

                        let effect_key = gate_effect_key(
                            plan_id,
                            &task_id,
                            pipeline_rung,
                            GateCompletionKind::Gate,
                        );
                        if !ctx.state.mark_gate_active(effect_key.clone()) {
                            debug!(
                                plan_id = %plan_id,
                                task = %task_id,
                                rung = pipeline_rung,
                                "preflight gate result found an already active gate effect"
                            );
                        }
                        emit_runner_event(
                            ctx.paths,
                            ctx.state,
                            ctx.tui,
                            ctx.config,
                            RunnerEvent::gate_dispatch_started(
                                &run_id,
                                attempt_ref.clone(),
                                GateCompletionKind::Gate,
                                pipeline_rung,
                            ),
                        );

                        match ctx
                            .executor
                            .apply_event(plan_id, &ExecutorEvent::ImplementationDone)
                        {
                            Ok(phase) => {
                                ctx.tui.phase_transition(plan_id, "implementing", "gating");
                                info!(
                                    plan_id = %plan_id,
                                    task = %task_id,
                                    phase = ?phase,
                                    "preflight verification advanced task to gate"
                                );
                            }
                            Err(e) => {
                                warn!(plan_id = %plan_id, task = %task_id, err = %e,
                                    "transition error after preflight verification");
                            }
                        }

                        let gate_tx = ctx.gate_tx.clone();
                        let fatal_tx = ctx.fatal_tx.clone();
                        let plan_id_fatal = plan_id.clone();
                        tokio::spawn(async move {
                            if let Err(e) = gate_tx.send(preflight).await {
                                error!(plan_id = %plan_id_fatal, err = %e,
                                    "CRITICAL: failed to send preflight gate completion -- sending fatal");
                                let _ = fatal_tx
                                    .send(AgentEvent::Error {
                                        message: format!(
                                            "gate channel closed for plan {plan_id_fatal}: {e}"
                                        ),
                                    })
                                    .await;
                            }
                        });
                        return ActionDispatchOutcome::Handled;
                    }

                    debug!(
                        plan_id = %plan_id,
                        task = %task_id,
                        duration_ms = preflight.duration_ms,
                        output = %preflight.output,
                        "task verification preflight failed -- dispatching agent"
                    );
                }
            }

            ctx.state.total_agent_calls += 1;
            ctx.state.task_agent_calls += 1;

            let role_enum = parse_dispatch_role(role);
            let task_category = neuro_prompt_task_category(role_enum);

            ctx.sink
                .task_started(plan_id, &task_id, role, &task_def.title, attempt_num);
            let bias_weight = knowledge_bias_weight(ctx.config);
            let knowledge_candidates = candidate_model_slugs(ctx.config);
            let knowledge_store = KnowledgeStore::for_workdir(&ctx.config.workdir);
            let knowledge_advice = build_knowledge_routing_advice(
                &knowledge_store,
                &knowledge_candidates,
                role_enum,
                task_category.label(),
            );
            debug!(
                plan_id = %plan_id,
                task = %task_id,
                role = %role_enum,
                task_category = %task_category.label(),
                hints = knowledge_advice.hints.len(),
                bias_weight = bias_weight,
                "knowledge store consulted for routing"
            );
            let gate_feedback = DispatchGateFeedback::from_raw(&previous_gate_output);
            let daimon_hook = daimon_task_hook(ctx.config, task_def, attempt_num);
            ctx.state.current_daimon_strategy = daimon_hook.as_ref().map(|hook| hook.strategy);
            let routing_context = {
                use roko_learn::model_router::RoutingContext;
                RoutingContext {
                    task_category,
                    complexity: tier_to_complexity(&task_def.tier),
                    iteration: attempt_num.saturating_sub(1),
                    role: role_enum,
                    crate_familiarity: 0.5,
                    has_prior_failure: attempt_num > 1,
                    conductor_load: 0.0,
                    active_agents: 0,
                    ready_queue_depth: 0,
                    max_queue_wait_hours: 0.0,
                    daimon_policy: daimon_policy_for_hook(daimon_hook.as_ref()),
                    thinking_level: daimon_hook
                        .as_ref()
                        .map(|_| default_effort_label(ctx.config)),
                    temperament: None,
                    previous_model: None,
                    plan_context_tokens: None,
                    tier_thresholds: daimon_hook
                        .as_ref()
                        .map(|hook| roko_daimon::adjusted_thresholds(&hook.behavioral_state)),
                }
            };
            let dispatch_ctx = DispatchContext {
                plan_id: plan_id.clone(),
                role: role.to_string(),
                workdir: ctx.config.workdir.clone(),
                model_hint: None,
                force_backend: ctx.config.cli_model_override.clone(),
                budget_remaining_usd: if ctx.config.max_plan_usd > 0.0 {
                    (ctx.config.max_plan_usd - ctx.state.plan_cost(plan_id)).max(0.0)
                } else {
                    f64::INFINITY
                },
                attempt: attempt_num.saturating_sub(1),
                gate_feedback,
                routing_context: Some(routing_context),
                dependency_outputs: ctx.state.dependency_outputs(plan_id, &task_def.depends_on),
            };
            ctx.state.task_model_hint = task_def.model_hint.clone();
            ctx.state.routing_context = dispatch_ctx.routing_context.clone();
            let dispatcher = ctx.factory.dispatcher();
            let mut dispatch_plan = match dispatcher.plan(task_def, &dispatch_ctx) {
                Ok(plan) => plan,
                Err(err) => {
                    let message = format!("dispatch planning failed: {err}");
                    error!(plan_id = %plan_id, task = %task_id, error = %message);
                    if let Err(e) = ctx
                        .executor
                        .apply_event(plan_id, &ExecutorEvent::Fatal(message.clone()))
                    {
                        error!(plan_id = %plan_id, error = %e,
                            "failed to apply Fatal event -- forcing plan terminal");
                        ctx.state.force_plan_terminal(plan_id);
                    }
                    ctx.tui.error(&message);
                    return ActionDispatchOutcome::Noop;
                }
            };
            let baseline_model = dispatch_plan.model.slug.clone();
            let baseline_score = knowledge_advice.score_for(&baseline_model);
            let mut selected_source = "dispatcher".to_string();
            let allow_learned_model_modulation =
                task_def.model_hint.is_none() && !dispatch_plan.forced;
            if allow_learned_model_modulation {
                if let Some(best_hint) = knowledge_advice
                    .hints
                    .iter()
                    .filter(|hint| hint.model_slug != baseline_model)
                    .max_by(|left, right| {
                        left.score
                            .total_cmp(&right.score)
                            .then_with(|| left.model_slug.cmp(&right.model_slug))
                    })
                {
                    if best_hint.score + bias_weight > baseline_score {
                        debug!(
                            from = %baseline_model,
                            to = %best_hint.model_slug,
                            baseline_score,
                            hint_score = best_hint.score,
                            bias_weight = bias_weight,
                            reason = %best_hint.reason,
                            supporting_entries = best_hint.supporting_entries,
                            "knowledge store nudged model selection"
                        );
                        dispatch_plan.model = ModelSpec::from_slug(best_hint.model_slug.clone());
                        selected_source = "dispatcher+knowledge".to_string();
                    }
                }
            }
            let mut dispatch_turn_limit = DEFAULT_AGENT_TURN_LIMIT;
            let mut dispatch_effort = None;
            let daimon_modulation = daimon_hook.as_ref().and_then(|hook| {
                daimon_dispatch_modulation(
                    ctx.config,
                    hook,
                    &dispatch_plan.model.slug,
                    allow_learned_model_modulation,
                )
            });
            if let Some(modulation) = &daimon_modulation {
                dispatch_turn_limit = modulation.turn_limit;
                dispatch_effort = Some(modulation.effort.clone());
                if modulation.model != dispatch_plan.model.slug {
                    debug!(
                        from = %dispatch_plan.model.slug,
                        to = %modulation.model,
                        effort = %modulation.effort,
                        turn_limit = modulation.turn_limit,
                        "daimon modulated dispatch"
                    );
                    dispatch_plan.model = ModelSpec::from_slug(modulation.model.clone());
                    selected_source = if selected_source == "dispatcher+knowledge" {
                        "dispatcher+knowledge+daimon".to_string()
                    } else {
                        "dispatcher+daimon".to_string()
                    };
                }
            }
            let requested_model = dispatch_plan.model.slug.clone();
            let prompt_diagnostics = dispatch_plan.prompt.diagnostics.clone();
            // Stash section diagnostics keyed by attempt for SectionOutcome
            // recording when the gate completes (pass or fail).
            {
                let attempt_key = format!("{plan_id}:{task_id}:{attempt_num}");
                // Store playbook IDs so gate terminal can call record_outcome.
                if !prompt_diagnostics.playbook_ids.is_empty() {
                    ctx.task_playbook_ids
                        .insert(attempt_key.clone(), prompt_diagnostics.playbook_ids.clone());
                }
                ctx.section_diagnostics
                    .insert(attempt_key, prompt_diagnostics.clone());
            }
            ctx.tui
                .model_selected(plan_id, &task_id, &requested_model, &selected_source);
            let mut system_prompt = dispatch_plan.prompt.system_prompt;
            if let Some(section) = daimon_hook.as_ref().and_then(render_daimon_prompt_context) {
                system_prompt.push_str("\n\n");
                system_prompt.push_str(&section);
            }
            let mut final_prompt = dispatch_plan.prompt.user_prompt;
            info!(
                plan_id = %plan_id,
                task = %task_id,
                model = %requested_model,
                source = %selected_source,
                system_prompt_len = system_prompt.len(),
                user_prompt_len = final_prompt.len(),
                estimated_tokens = prompt_diagnostics.estimated_tokens,
                included_sections = prompt_diagnostics.included_sections.len(),
                dropped_sections = prompt_diagnostics.dropped_sections.len(),
                "dispatch: model selected, prompt assembled"
            );
            debug!(
                plan_id = %plan_id,
                task = %task_id,
                included_sections = ?dispatch_plan.prompt.diagnostics.included_sections,
                dropped_sections = ?dispatch_plan.prompt.diagnostics.dropped_sections,
                knowledge_ids = ?dispatch_plan.prompt.diagnostics.knowledge_ids,
                playbook_ids = ?dispatch_plan.prompt.diagnostics.playbook_ids,
                "dispatch prompt detail"
            );

            // Append replan context before prompt diagnostics so the durable
            // event captures the actual prompt shape sent to the runtime.
            if let Some(replan) = ctx.state.take_replan_context(plan_id, &task_id) {
                final_prompt.push_str(&replan);
            }
            ctx.state.current_prompt_text = format!("{system_prompt}\n\n{final_prompt}");

            // Extension: pre-inference hook.
            let task_role = task_def.role.as_deref().unwrap_or("implementer");
            fire_pre_inference_hook(
                ctx.config,
                plan_id,
                &task_id,
                &requested_model,
                task_role,
                ctx.tui,
            )
            .await;

            let dispatch = match ctx.factory.resolve_runtime(&requested_model) {
                Ok(selection) => selection,
                Err(hint_err) => {
                    // Fall back to default model when model_hint can't be resolved
                    let default_model = &ctx.config.model;
                    warn!(
                        plan_id = %plan_id,
                        task = %task_id,
                        hint = %requested_model,
                        fallback = %default_model,
                        "model_hint resolution failed, falling back to default model"
                    );
                    match ctx.factory.resolve_runtime(default_model) {
                        Ok(selection) => selection,
                        Err(default_err) => {
                            let message = format!(
                                "model resolution failed: hint '{}': {}; default '{}': {}",
                                requested_model, hint_err, default_model, default_err
                            );
                            error!(plan_id = %plan_id, task = %task_id, error = %message);
                            if let Err(e) = ctx
                                .executor
                                .apply_event(plan_id, &ExecutorEvent::Fatal(message.clone()))
                            {
                                error!(plan_id = %plan_id, error = %e,
                                    "failed to apply Fatal event -- forcing plan terminal");
                                ctx.state.force_plan_terminal(plan_id);
                            }
                            ctx.tui.error(&message);
                            return ActionDispatchOutcome::Noop;
                        }
                    }
                }
            };

            let agent_id = format!("{plan_id}/{task_id}");
            let attempt_ref = TaskAttemptRef::new(plan_id.clone(), task_id.clone(), attempt_num);
            let run_id = ctx.state.run_id().to_string();
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                ctx.config,
                RunnerEvent::task_attempt_started(&run_id, attempt_ref.clone(), &task_def.title),
            );
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                ctx.config,
                RunnerEvent::prompt_assembled(
                    &run_id,
                    attempt_ref.clone(),
                    role,
                    &requested_model,
                    system_prompt.len(),
                    final_prompt.len(),
                    PromptAssemblyDiagnostics {
                        included_sections: prompt_diagnostics.included_sections,
                        dropped_sections: prompt_diagnostics.dropped_sections,
                        estimated_tokens: prompt_diagnostics.estimated_tokens,
                        knowledge_ids: prompt_diagnostics.knowledge_ids,
                        playbook_ids: prompt_diagnostics.playbook_ids,
                    },
                ),
            );
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                ctx.config,
                RunnerEvent::agent_dispatch_started(
                    &run_id,
                    attempt_ref.clone(),
                    &agent_id,
                    role,
                    &requested_model,
                ),
            );

            match dispatch {
                ResolvedAgentRuntime::Cli {
                    model,
                    cli_provider,
                } => {
                    let model_display = model.clone();
                    let mut spawn_config = AgentSpawnConfig::from_run_config(
                        ctx.config,
                        final_prompt,
                        system_prompt,
                        model,
                        agent_id.clone(),
                    );
                    spawn_config.max_turns = dispatch_turn_limit;
                    spawn_config.effort = dispatch_effort.clone();
                    if let Some(provider) = cli_provider {
                        spawn_config = spawn_config.with_cli_provider(provider);
                    }

                    match ctx
                        .factory
                        .dispatcher()
                        .spawn_streaming_cli_agent(&spawn_config, ctx.agent_tx.clone())
                        .await
                    {
                        Ok(handle) => {
                            ctx.state.agent_active = true;
                            ctx.state.agent_pid = Some(handle.pid);
                            emit_runner_event(
                                ctx.paths,
                                ctx.state,
                                ctx.tui,
                                ctx.config,
                                RunnerEvent::agent_dispatch_completed(
                                    &run_id,
                                    attempt_ref,
                                    &agent_id,
                                    AgentDispatchOutcome::Spawned,
                                    Some(model_display.clone()),
                                    Some(handle.pid),
                                    None,
                                ),
                            );
                            ctx.tui.agent_spawned(&agent_id, role, &model_display);
                            ctx.tui.task_started(
                                plan_id,
                                &task_id,
                                &task_def.title,
                                "implementing",
                            );
                            ctx.agent_handles.insert(plan_id.to_string(), handle);
                            ctx.active_agent_tasks
                                .insert(plan_id.to_string(), task_id.clone());
                            register_agent_feed(ctx.config, plan_id, &task_id, &agent_id, ctx.tui);
                            return ActionDispatchOutcome::AgentStarted {
                                plan_id: plan_id.clone(),
                                task_id,
                            };
                        }
                        Err(e) => {
                            error!(err = %e, "failed to spawn agent");
                            let message = format!("agent spawn failed: {e}");
                            let agent_provider = ctx.state.agent_provider.clone();
                            emit_runner_event(
                                ctx.paths,
                                ctx.state,
                                ctx.tui,
                                ctx.config,
                                RunnerEvent::agent_dispatch_completed(
                                    &run_id,
                                    attempt_ref.clone(),
                                    &agent_id,
                                    AgentDispatchOutcome::SpawnFailed,
                                    Some(model_display.clone()),
                                    None,
                                    Some(message.clone()),
                                ),
                            );
                            emit_runner_event(
                                ctx.paths,
                                ctx.state,
                                ctx.tui,
                                ctx.config,
                                RunnerEvent::task_attempt_completed(
                                    &run_id,
                                    attempt_ref,
                                    TaskAttemptOutcome::Failed,
                                    Some(RunnerFailureKind::Resource),
                                    0,
                                    model_display,
                                    agent_provider,
                                ),
                            );
                            record_daimon_task_outcome(
                                ctx.config,
                                ctx.state.current_daimon_strategy,
                                plan_id,
                                &task_id,
                                false,
                                &message,
                            );
                            ctx.tui.error(&message);
                            if let Err(e2) = ctx.executor.apply_event(
                                plan_id,
                                &ExecutorEvent::Fatal(format!("spawn failed: {e}")),
                            ) {
                                error!(plan_id = %plan_id, error = %e2,
                                    "failed to apply Fatal event -- forcing plan terminal");
                                ctx.state.force_plan_terminal(plan_id);
                            }
                            return ActionDispatchOutcome::Noop;
                        }
                    }
                }
                ResolvedAgentRuntime::Bridge {
                    model,
                    provider_id,
                    roko_config,
                } => {
                    ctx.state.agent_active = true;
                    ctx.state.agent_pid = None;
                    let request = AgentDispatchRequest {
                        model_key: requested_model.clone(),
                        prompt: final_prompt,
                        system_prompt,
                        workdir: ctx.config.workdir.clone(),
                        agent_id: agent_id.clone(),
                        command: None,
                        timeout_ms: Some(duration_millis(agent_dispatch_timeout(ctx.config))),
                        mcp_config: ctx.config.mcp_config.clone(),
                        env: vec![
                            ("CARGO_INCREMENTAL".to_string(), "0".to_string()),
                            ("CARGO_BUILD_JOBS".to_string(), "2".to_string()),
                        ],
                        extra_args: Vec::new(),
                        effort: dispatch_effort.clone(),
                        tools: None,
                        bare_mode: false,
                        dangerously_skip_permissions: ctx.config.dangerously_skip_permissions,
                    };
                    ctx.factory
                        .spawn_shared_agent_bridge(request, ctx.agent_tx.clone());
                    emit_runner_event(
                        ctx.paths,
                        ctx.state,
                        ctx.tui,
                        ctx.config,
                        RunnerEvent::agent_dispatch_completed(
                            &run_id,
                            attempt_ref,
                            &agent_id,
                            AgentDispatchOutcome::Spawned,
                            Some(model.clone()),
                            None,
                            None,
                        ),
                    );
                    ctx.tui
                        .agent_spawned(&agent_id, role, &format!("{provider_id}:{model}"));
                    ctx.tui
                        .task_started(plan_id, &task_id, &task_def.title, "implementing");
                    ctx.active_agent_tasks
                        .insert(plan_id.to_string(), task_id.clone());
                    register_agent_feed(ctx.config, plan_id, &task_id, &agent_id, ctx.tui);
                    return ActionDispatchOutcome::AgentStarted {
                        plan_id: plan_id.clone(),
                        task_id,
                    };
                }
            }
        }

        ExecutorAction::RunGate { plan_id, rung } => {
            let task_id = ctx.state.current_task.clone();
            let gates_config = gates_config_for_run(ctx.config);
            let pipeline_rung = ctx.config.max_gate_rung;
            // Default selected rungs are Cargo-oriented; custom rungs own their command semantics.
            let has_cargo_toml = std::fs::metadata(ctx.config.workdir.join("Cargo.toml")).is_ok();
            if !gates_config.has_custom_rungs() && !has_cargo_toml {
                info!(plan_id = %plan_id, rung = pipeline_rung, "skipping default gate pipeline (no Cargo.toml in workspace)");
                record_skipped_gate_rung(
                    ctx,
                    plan_id,
                    &task_id,
                    pipeline_rung,
                    "gate-pipeline:default",
                    "skipped: no Cargo.toml in workspace",
                );
                return ActionDispatchOutcome::Handled;
            }

            let effect_key =
                gate_effect_key(plan_id, &task_id, pipeline_rung, GateCompletionKind::Gate);
            if !ctx.state.mark_gate_active(effect_key.clone()) {
                debug!(
                    plan_id = %plan_id,
                    task_id = %task_id,
                    rung = pipeline_rung,
                    "gate pipeline already active - suppressing duplicate dispatch"
                );
                return ActionDispatchOutcome::Noop;
            }
            info!(
                plan_id = %plan_id,
                requested_rung = *rung,
                rung = pipeline_rung,
                custom_rungs = gates_config.has_custom_rungs(),
                "dispatching gate pipeline"
            );
            let run_id = ctx.state.run_id().to_string();
            let attempt_ref = TaskAttemptRef::new(
                plan_id.clone(),
                task_id.clone(),
                ctx.state.iteration_for(plan_id, &task_id),
            );
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                ctx.config,
                RunnerEvent::gate_dispatch_started(
                    &run_id,
                    attempt_ref,
                    GateCompletionKind::Gate,
                    pipeline_rung,
                ),
            );
            let task_def = ctx
                .task_index
                .get(plan_id.as_str())
                .and_then(|tasks| tasks.get(task_id.as_str()));
            let is_read_only_role = task_role_is_read_only(task_def);

            if is_read_only_role {
                // Read-only tasks don't produce artifacts — auto-pass the gate.
                // Running cargo check / structural verify on a researcher task
                // wastes time and fails on files not yet created.
                //
                // IMPORTANT: Send via spawned task, NOT inline. Sending on
                // gate_tx from inside the select loop that reads gate_rx
                // would deadlock if the channel buffer is full.
                info!(
                    plan_id = %plan_id,
                    task_id = %task_id,
                    rung = pipeline_rung,
                    "skipping gate for read-only role"
                );
                let completion = GateCompletion {
                    plan_id: plan_id.clone(),
                    task_id: task_id.clone(),
                    rung: pipeline_rung,
                    passed: true,
                    output: "skipped: read-only role".to_string(),
                    failure_kind: None,
                    duration_ms: 0,
                    kind: GateCompletionKind::Gate,
                    verdicts: Vec::new(),
                };
                let gate_tx = ctx.gate_tx.clone();
                let fatal_tx = ctx.fatal_tx.clone();
                let plan_id_fatal = plan_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = gate_tx.send(completion).await {
                        error!(plan_id = %plan_id_fatal, err = %e,
                            "CRITICAL: failed to send auto-pass gate -- sending fatal");
                        let _ = fatal_tx
                            .send(AgentEvent::Error {
                                message: format!(
                                    "gate channel closed for plan {plan_id_fatal}: {e}"
                                ),
                            })
                            .await;
                    }
                });
            } else {
                let verify_steps = task_def.map(|task| task.verify.clone()).unwrap_or_default();
                let complexity = gate_plan_complexity_for_task(task_def);
                let target_crates = task_target_crates(task_def);
                gate_dispatch::spawn_gate(
                    plan_id.clone(),
                    task_id,
                    pipeline_rung,
                    ctx.config.workdir.clone(),
                    gates_config,
                    complexity,
                    verify_steps,
                    duration_secs(gate_timeout(ctx.config, pipeline_rung)),
                    ctx.gate_tx.clone(),
                    ctx.gate_sem.clone(),
                    target_crates,
                );
            }
            ActionDispatchOutcome::Handled
        }

        ExecutorAction::RunVerify { plan_id } => {
            let verify_steps = ctx
                .task_index
                .get(plan_id.as_str())
                .map(|tasks| {
                    let mut tasks: Vec<_> = tasks.values().collect();
                    tasks.sort_by_key(|t| t.sequence);
                    tasks
                        .into_iter()
                        .filter(|task| !task.verify.is_empty())
                        .map(|task| (task.id.clone(), task.verify.clone()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            if verify_steps.is_empty() {
                info!(plan_id = %plan_id, "no declared plan verify steps — passing verify phase");
                match complete_verified_plan_success(
                    plan_id,
                    ctx.executor,
                    ctx.state,
                    ctx.paths,
                    ctx.tui,
                    ctx.config,
                ) {
                    Ok(phase) => {
                        ctx.tui
                            .phase_transition(plan_id, "verifying", &format!("{phase:?}"));
                        info!(plan_id = %plan_id, phase = ?phase, "plan verify passed — plan complete");
                    }
                    Err(err) => {
                        warn!(
                            plan_id = %plan_id,
                            error = %err,
                            "transition error while completing plan with no verify steps"
                        );
                        let _ = ctx.executor.apply_event(
                            plan_id,
                            &ExecutorEvent::Fatal(format!("plan verify transition failed: {err}")),
                        );
                    }
                }
                save_snapshot(
                    ctx.config,
                    ctx.executor,
                    ctx.paths,
                    ctx.state,
                    ctx.merge_queue,
                    ctx.gate_thresholds,
                    ctx.snapshot_writer,
                );
                return ActionDispatchOutcome::Handled;
            }

            let effect_key = gate_effect_key(
                plan_id,
                "plan-verify",
                u32::MAX,
                GateCompletionKind::PlanVerify,
            );
            if !ctx.state.mark_gate_active(effect_key.clone()) {
                debug!(
                    plan_id = %plan_id,
                    "plan verify already active — suppressing duplicate dispatch"
                );
                return ActionDispatchOutcome::Noop;
            }
            let run_id = ctx.state.run_id().to_string();
            let attempt_ref = TaskAttemptRef::new(
                plan_id.clone(),
                "plan-verify",
                ctx.state.iteration_for(plan_id, "plan-verify"),
            );
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                ctx.config,
                RunnerEvent::gate_dispatch_started(
                    &run_id,
                    attempt_ref,
                    GateCompletionKind::PlanVerify,
                    u32::MAX,
                ),
            );

            info!(
                plan_id = %plan_id,
                task_count = verify_steps.len(),
                "dispatching plan verify"
            );
            gate_dispatch::spawn_plan_verify(
                plan_id.clone(),
                ctx.config.workdir.clone(),
                verify_steps,
                duration_secs(gate_timeout(ctx.config, 2)),
                ctx.gate_tx.clone(),
                ctx.gate_sem.clone(),
            );
            ActionDispatchOutcome::Handled
        }

        ExecutorAction::CompletePlan { plan_id } => {
            info!(plan_id = %plan_id, "plan completed");
            ctx.tui.plan_completed(plan_id, true);
            let run_id = ctx.state.run_id().to_string();
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                ctx.config,
                RunnerEvent::plan_completed(&run_id, plan_id, PlanOutcome::Succeeded, None),
            );
            save_snapshot(
                ctx.config,
                ctx.executor,
                ctx.paths,
                ctx.state,
                ctx.merge_queue,
                ctx.gate_thresholds,
                ctx.snapshot_writer,
            );
            ActionDispatchOutcome::Handled
        }

        ExecutorAction::FailPlan { plan_id, reason } => {
            warn!(plan_id = %plan_id, reason = %reason, "plan failed");
            ctx.state.tasks_failed += 1;
            ctx.state.roll_into_totals();
            ctx.tui
                .task_completed(plan_id, &ctx.state.current_task, "failed");
            ctx.tui.plan_completed(plan_id, false);
            let run_id = ctx.state.run_id().to_string();
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                ctx.config,
                RunnerEvent::plan_completed(
                    &run_id,
                    plan_id,
                    PlanOutcome::Failed,
                    Some(reason.clone()),
                ),
            );
            ActionDispatchOutcome::Handled
        }

        ExecutorAction::MergeBranch { plan_id } => {
            let files_changed = ctx
                .executor
                .plan_state(plan_id)
                .map(|state| state.files_changed.clone())
                .unwrap_or_default();
            let request = MergeRequest::new(
                plan_id.clone(),
                format!("roko/plan/{plan_id}"),
                files_changed,
                0,
            );
            let merger = PlanMerger::new(
                ctx.merge_queue.clone(),
                PlanMergerConfig::new(ctx.config.workdir.clone(), gate_timeout(ctx.config, 0)),
            );
            match merger.submit(request, ctx.gate_tx.clone()) {
                MergeDispatch::Reserved {
                    plan_id,
                    branch_name,
                } => {
                    info!(
                        plan_id = %plan_id,
                        branch = %branch_name,
                        "reserved merge queue request"
                    );
                    save_snapshot(
                        ctx.config,
                        ctx.executor,
                        ctx.paths,
                        ctx.state,
                        ctx.merge_queue,
                        ctx.gate_thresholds,
                        ctx.snapshot_writer,
                    );
                }
                MergeDispatch::Blocked { plan_id } => {
                    info!(
                        plan_id = %plan_id,
                        blocked_conflicts = ?ctx.merge_queue.blocked_conflicts(),
                        "merge queued but currently blocked by file locks"
                    );
                    save_snapshot(
                        ctx.config,
                        ctx.executor,
                        ctx.paths,
                        ctx.state,
                        ctx.merge_queue,
                        ctx.gate_thresholds,
                        ctx.snapshot_writer,
                    );
                }
            }
            ActionDispatchOutcome::Handled
        }

        _ => {
            info!(action = ?action, "auto-advancing action");
            ActionDispatchOutcome::Handled
        }
    }
}

// ─── Adaptive gate thresholds ────────────────────────────────────────────

/// Update EMA-based adaptive gate thresholds for a given rung.
fn update_gate_thresholds(thresholds: &mut GateThresholds, rung: u32, passed: bool) {
    thresholds.observe(rung, passed);
    // Gate thresholds are now persisted as part of the unified StateSnapshot
    // in save_snapshot(), not via a separate disk write.
}

/// Emit gate thresholds into the TUI push pipeline after updating in memory.
fn emit_gate_thresholds_event(thresholds: &GateThresholds, tui: &TuiBridge) {
    if let Ok(json) = serde_json::to_string(thresholds) {
        tui.gate_thresholds_updated(&json);
    }
}

// ─── Section outcome helpers ─────────────────────────────────────────────

/// Build lightweight `SectionOutcomeRecord` entries from prompt diagnostics
/// and a terminal gate result. Each included/dropped section becomes one
/// record so the downstream bandit can attribute section presence to
/// pass/fail.
fn build_section_outcome_records(
    plan_id: &str,
    task_id: &str,
    model: &str,
    provider: &str,
    status: SectionOutcomeStatus,
    diag: &PromptDiagnostics,
    verdicts: &[GateVerdictSummary],
) -> Vec<SectionOutcomeRecord> {
    let timestamp = chrono::Utc::now().to_rfc3339();
    let gate_outcomes = verdicts
        .iter()
        .map(|v| roko_learn::section_outcome::SectionGateOutcome {
            gate_id: v.gate_name.clone(),
            outcome: if v.passed {
                "passed".to_string()
            } else {
                "failed".to_string()
            },
            required: true,
        })
        .collect::<Vec<_>>();

    let mut records =
        Vec::with_capacity(diag.included_sections.len() + diag.dropped_sections.len());

    for section_name in &diag.included_sections {
        records.push(SectionOutcomeRecord {
            schema_version: SECTION_OUTCOME_SCHEMA_VERSION,
            timestamp: timestamp.clone(),
            workspace_id: String::new(),
            invocation_id: format!("{plan_id}:{task_id}"),
            task_id: task_id.to_string(),
            task_type: "plan_task".to_string(),
            role_id: String::new(),
            provider: provider.to_string(),
            model: model.to_string(),
            section_id: section_name.clone(),
            section_name: section_name.clone(),
            action_id: format!("prompt_section:{section_name}"),
            section_kind: roko_learn::section_outcome::SectionKind::Prompt,
            included: true,
            estimated_tokens: 0,
            tokens_used: 0,
            token_budget: None,
            source_type: None,
            source_id: None,
            experiment_id: None,
            status,
            gate_outcomes: gate_outcomes.clone(),
            review_verdicts: Vec::new(),
        });
    }

    for section_name in &diag.dropped_sections {
        records.push(SectionOutcomeRecord {
            schema_version: SECTION_OUTCOME_SCHEMA_VERSION,
            timestamp: timestamp.clone(),
            workspace_id: String::new(),
            invocation_id: format!("{plan_id}:{task_id}"),
            task_id: task_id.to_string(),
            task_type: "plan_task".to_string(),
            role_id: String::new(),
            provider: provider.to_string(),
            model: model.to_string(),
            section_id: section_name.clone(),
            section_name: section_name.clone(),
            action_id: format!("prompt_section:{section_name}"),
            section_kind: roko_learn::section_outcome::SectionKind::Prompt,
            included: false,
            estimated_tokens: 0,
            tokens_used: 0,
            token_budget: None,
            source_type: None,
            source_id: None,
            experiment_id: None,
            status,
            gate_outcomes: gate_outcomes.clone(),
            review_verdicts: Vec::new(),
        });
    }

    records
}

/// Append section outcome records to the JSONL store. Failures are logged
/// but do not abort the run.
async fn append_section_outcomes(path: PathBuf, records: Vec<SectionOutcomeRecord>) {
    if records.is_empty() {
        return;
    }
    match SectionOutcomeStore::open_creating(path).await {
        Ok(store) => {
            if let Err(err) = store.append_many(&records).await {
                warn!(err = %err, "failed to append section outcome records");
            }
        }
        Err(err) => warn!(err = %err, "failed to open section outcome store"),
    }
}

fn parse_dispatch_role(role: &str) -> AgentRole {
    match role.trim().to_ascii_lowercase().as_str() {
        "conductor" => AgentRole::Conductor,
        "strategist" => AgentRole::Strategist,
        "implementer" => AgentRole::Implementer,
        "architect" => AgentRole::Architect,
        "researcher" => AgentRole::Researcher,
        "auditor" | "reviewer" => AgentRole::Auditor,
        "quick-reviewer" | "quick_reviewer" => AgentRole::QuickReviewer,
        "scribe" => AgentRole::Scribe,
        "critic" => AgentRole::Critic,
        "auto-fixer" => AgentRole::AutoFixer,
        "refactorer" => AgentRole::Refactorer,
        "pre-planner" => AgentRole::PrePlanner,
        "doc-verifier" => AgentRole::DocVerifier,
        "integration-tester" => AgentRole::IntegrationTester,
        "merge-resolver" => AgentRole::MergeResolver,
        "terminal-validator" => AgentRole::TerminalValidator,
        "golem-lifecycle-tester" => AgentRole::GolemLifecycleTester,
        "spec-drift-detector" => AgentRole::SpecDriftDetector,
        "regression-detector" => AgentRole::RegressionDetector,
        "performance-sentinel" => AgentRole::PerformanceSentinel,
        "coverage-tracker" => AgentRole::CoverageTracker,
        "plan-lifecycle-manager" | "plan-lifecycle-mgr" => AgentRole::PlanLifecycleManager,
        "cross-system-tester" => AgentRole::CrossSystemTester,
        "error-diagnoser" => AgentRole::ErrorDiagnoser,
        "dep-validator" | "dependency-validator" => AgentRole::DependencyValidator,
        "pattern-extractor" => AgentRole::PatternExtractor,
        "snapshot-comparator" => AgentRole::SnapshotComparator,
        "full-loop-validator" => AgentRole::FullLoopValidator,
        _ => AgentRole::Implementer,
    }
}

fn candidate_model_slugs(config: &RunConfig) -> Vec<String> {
    let mut slugs = if let Some(router) = &config.cascade_router {
        router.model_slugs().to_vec()
    } else if let Some(roko_config) = &config.roko_config {
        roko_config.effective_models().keys().cloned().collect()
    } else {
        Vec::new()
    };
    slugs.sort();
    slugs.dedup();
    slugs
}

fn knowledge_bias_weight(config: &RunConfig) -> f64 {
    config
        .roko_config
        .as_ref()
        .map(|cfg| {
            // Prefer the dedicated knowledge_bias weight; fall back to latency.
            cfg.routing
                .weights
                .default
                .knowledge_bias
                .unwrap_or(cfg.routing.weights.default.latency)
        })
        .unwrap_or(0.2)
        .clamp(0.0, 1.0)
}

// ─── Extension Chain Hooks ───────────────────────────────────────────────

/// Fire pre_inference extension hook (non-blocking try_lock to avoid stalling select).
async fn fire_pre_inference_hook(
    config: &RunConfig,
    plan_id: &str,
    task_id: &str,
    model: &str,
    role: &str,
    tui: &TuiBridge,
) {
    let Some(ext_chain) = &config.extension_chain else {
        return;
    };
    let Ok(chain) = ext_chain.try_lock() else {
        warn!("extension chain lock contended, skipping pre_inference hook");
        return;
    };
    let mut req = roko_core::extension::InferenceRequest {
        plan_id: plan_id.to_string(),
        task: task_id.to_string(),
        role: role.to_string(),
        model: model.to_string(),
        prompt_tokens: 0,
        extra: serde_json::Value::Null,
    };
    let success = chain.run_pre_inference(&mut req).await.is_ok();
    if !success {
        warn!("extension pre_inference hook failed");
    }
    tui.extension_hook(plan_id, task_id, "pre_inference", success);
}

/// Fire post_inference extension hook.
async fn fire_post_inference_hook(
    config: &RunConfig,
    plan_id: &str,
    task_id: &str,
    model: &str,
    role: &str,
    success: bool,
    cost_usd: f64,
    wall_ms: u64,
    tui: &TuiBridge,
) {
    let Some(ext_chain) = &config.extension_chain else {
        return;
    };
    let Ok(chain) = ext_chain.try_lock() else {
        warn!("extension chain lock contended, skipping post_inference hook");
        return;
    };
    let mut resp = roko_core::extension::InferenceResponse {
        plan_id: plan_id.to_string(),
        task: task_id.to_string(),
        role: role.to_string(),
        model: model.to_string(),
        success,
        cost_usd,
        wall_ms,
        extra: serde_json::Value::Null,
    };
    let hook_ok = chain.run_post_inference(&mut resp).await.is_ok();
    if !hook_ok {
        warn!("extension post_inference hook failed");
    }
    tui.extension_hook(plan_id, task_id, "post_inference", hook_ok);
}

/// Fire on_gate extension hook.
async fn fire_on_gate_hook(config: &RunConfig, completion: &GateCompletion, tui: &TuiBridge) {
    let Some(ext_chain) = &config.extension_chain else {
        return;
    };
    let Ok(chain) = ext_chain.try_lock() else {
        warn!("extension chain lock contended, skipping on_gate hook");
        return;
    };
    for verdict in &completion.verdicts {
        let mut event = roko_core::extension::GateEvent {
            plan_id: completion.plan_id.clone(),
            gate_name: verdict.gate_name.clone(),
            passed: verdict.passed,
            rung: format!("rung-{}", completion.rung),
            duration_ms: completion.duration_ms,
            details: serde_json::Value::Null,
        };
        let hook_ok = chain.run_on_gate(&mut event).await.is_ok();
        if !hook_ok {
            warn!(gate = %verdict.gate_name, "extension on_gate hook failed");
        }
        tui.extension_hook(
            &completion.plan_id,
            &completion.task_id,
            &format!("on_gate:{}", verdict.gate_name),
            hook_ok,
        );
    }
}

/// Fire on_error extension hook.
async fn fire_on_error_hook(
    config: &RunConfig,
    message: &str,
    source: &str,
    tui: &TuiBridge,
    plan_id: &str,
    task_id: &str,
) {
    let Some(ext_chain) = &config.extension_chain else {
        return;
    };
    let Ok(chain) = ext_chain.try_lock() else {
        warn!("extension chain lock contended, skipping on_error hook");
        return;
    };
    let event = roko_core::extension::ErrorEvent {
        error_message: message.to_string(),
        source: source.to_string(),
        extra: serde_json::Value::Null,
    };
    let hook_ok = chain.run_on_error(&event).await.is_ok();
    tui.extension_hook(plan_id, task_id, "on_error", hook_ok);
}

/// Shutdown extension chain + persist cascade router.
async fn shutdown_subsystems(config: &RunConfig, tui: &TuiBridge) {
    // Extension chain shutdown.
    if let Some(ext_chain) = &config.extension_chain {
        let mut chain = ext_chain.lock().await;
        let errors = chain.shutdown_all().await;
        for (name, err) in &errors {
            warn!(extension = %name, error = %err, "extension shutdown failed");
        }
    }

    // Persist cascade router learned state.
    if let Some(router) = &config.cascade_router {
        let router_path = config.layout.cascade_router_path();
        if let Err(err) = router.save(&router_path) {
            warn!(error = %err, "failed to persist cascade router");
        } else {
            info!("cascade router state persisted");
            tui.cascade_router_updated(&router.snapshot_json());
        }
    }
}

/// Compact the episode log if it exceeds the retention threshold.
///
/// Uses the default [`RetentionPolicy`] (200 episodes, 90 days).
/// Errors are logged but never propagated — compaction is best-effort.
async fn compact_episodes_if_needed(episodes_path: &std::path::Path) {
    use roko_learn::episode_logger::{EpisodeLogger, RetentionPolicy};

    // Use metadata probe instead of .exists() to avoid TOCTOU.
    // If the file doesn't exist, there's nothing to compact.
    match std::fs::metadata(episodes_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            warn!(error = %e, "failed to stat episodes file");
            return;
        }
        Ok(_) => {}
    }

    let logger = EpisodeLogger::new(episodes_path.to_path_buf());
    let policy = RetentionPolicy::default();
    let now = chrono::Utc::now();

    match logger.compact(now, &policy).await {
        Ok(stats) if stats.removed > 0 => {
            info!(
                before = stats.before,
                after = stats.after,
                removed = stats.removed,
                bytes_reclaimed = stats.bytes_reclaimed,
                "episode log compacted"
            );
        }
        Ok(_) => {} // nothing to compact
        Err(err) => {
            warn!(error = %err, "episode compaction failed (best-effort)");
        }
    }
}

async fn handle_plan_timeout(
    executor: &ParallelExecutor,
    plans: &[Plan],
    state: &mut RunState,
    agent_handles: &mut HashMap<String, AgentHandle>,
    paths: &PersistPaths,
    merge_queue: &MergeQueue,
    tui: &TuiBridge,
    config: &RunConfig,
    gate_thresholds: &GateThresholds,
    writer: &SnapshotWriter,
) -> Result<()> {
    let in_flight = collect_in_flight_attempts(state);
    let timeout_secs = duration_secs(plan_total_timeout(config));
    error!(
        timeout_secs,
        current_plan = %state.plan_id,
        current_task = %state.current_task,
        active_plans = ?executor.active_plans(),
        in_flight_attempts = ?in_flight,
        "plan execution exceeded wall-clock timeout"
    );
    stop_all_agents(agent_handles, state, Duration::from_secs(3)).await;
    save_snapshot(
        config,
        executor,
        paths,
        state,
        merge_queue,
        gate_thresholds,
        writer,
    );
    writer.flush();
    shutdown_subsystems(config, tui).await;
    let event = build_run_completed_event(executor, plans, state, RunOutcome::Failed);
    emit_runner_event(paths, state, tui, config, event);
    Err(anyhow::anyhow!(
        "plan execution exceeded wall-clock timeout after {} seconds",
        timeout_secs
    ))
}

fn collect_in_flight_attempts(state: &RunState) -> Vec<String> {
    let mut attempts = state
        .lifecycle
        .task_attempts
        .values()
        .filter(|attempt| {
            !matches!(
                attempt.status,
                TaskAttemptStatus::Passed
                    | TaskAttemptStatus::Failed
                    | TaskAttemptStatus::Exhausted
                    | TaskAttemptStatus::Cancelled
            )
        })
        .map(|attempt| format!("{}:{:?}", attempt.attempt.key(), attempt.status))
        .collect::<Vec<_>>();
    attempts.sort();
    attempts
}

async fn stop_all_agents(
    agent_handles: &mut HashMap<String, AgentHandle>,
    state: &mut RunState,
    grace: Duration,
) {
    for (_plan_id, handle) in agent_handles.drain() {
        let pid = handle.pid;
        handle.kill(grace).await;
        roko_agent::process::unregister_pid(pid);
    }
    if let Some(pid) = state.agent_pid.take() {
        roko_agent::process::unregister_pid(pid);
    }
    state.agent_active = false;
    state.agent_pid = None;
    state.agent_turn_completed = false;
}

async fn run_dream_consolidation_if_enabled(config: &RunConfig) {
    let Some(roko_config) = config.roko_config.as_ref() else {
        debug!("no roko config -- skipping dream consolidation");
        return;
    };

    if !roko_config.learning.dream_on_completion {
        debug!("dream consolidation after plan completion disabled");
        return;
    }

    debug!("running dream consolidation after plan completion");
    run_dream_consolidation(config).await;
}

async fn run_dream_consolidation(config: &RunConfig) {
    let workdir = config.workdir.clone();
    let timeout = llm_call_timeout(config);
    let dream_config = roko_dreams::DreamLoopConfig {
        auto_dream: true,
        idle_threshold_mins: 0,
        min_episodes_for_dream: 1,
        agent: roko_dreams::DreamAgentConfig {
            command: "claude".to_string(),
            args: Vec::new(),
            model: None,
            bare_mode: true,
            effort: "low".to_string(),
            fallback_model: None,
            timeout_ms: duration_millis(timeout),
            env: Vec::new(),
        },
    };
    let join = tokio::task::spawn_blocking(move || {
        let mut dream_runner = roko_dreams::DreamRunner::new(workdir.clone(), dream_config);
        dream_runner.consolidate_now()
    });
    match tokio::time::timeout(timeout, join).await {
        Ok(Ok(Ok(report))) => info!(
            processed_episodes = report.processed_episodes,
            knowledge_entries = report.knowledge_entries_written,
            playbooks = report.playbooks_created,
            "dream consolidation completed"
        ),
        Ok(Ok(Err(err))) => {
            warn!(error = %err, "dream consolidation failed — plan results unaffected")
        }
        Ok(Err(join_err)) => warn!(error = %join_err, "dream consolidation worker aborted"),
        Err(_) => warn!(
            timeout_secs = duration_secs(timeout),
            "dream consolidation timed out — skipping"
        ),
    }
}

/// Register an agent feed entry after successful spawn.
fn register_agent_feed(
    config: &RunConfig,
    plan_id: &str,
    task_id: &str,
    agent_id: &str,
    tui: &TuiBridge,
) {
    let Some(registry) = &config.feed_registry else {
        return;
    };
    if let Ok(mut reg) = registry.try_lock() {
        reg.register(roko_core::FeedInfo {
            id: String::new(), // Auto-assigned by registry
            name: format!("{plan_id}/{task_id}"),
            agent_id: agent_id.to_string(),
            kind: roko_core::FeedKind::Raw,
            access: roko_core::FeedAccess::Private,
            description: String::new(),
            schema: None,
            created_at: chrono::Utc::now(),
        });
        tui.extension_hook(plan_id, task_id, "feed_registered", true);
    }
}

// ─── Playbook Seeding ────────────────────────────────────────────────────

/// Seed the playbook store with starter templates when empty.
///
/// This solves the chicken-and-egg problem: playbooks are normally only
/// saved on task SUCCESS, but without playbooks the system has no guidance
/// from the start. These seeds give the first few runs structured advice.
async fn seed_playbooks_if_empty(layout: &RokoLayout) {
    use roko_learn::playbook::{Playbook, PlaybookStep, PlaybookStore};

    let pb_dir = layout.playbooks_dir();

    // Quick check: if the directory has any .json files, skip seeding.
    // Use read_dir directly instead of exists() to avoid TOCTOU race.
    match tokio::fs::read_dir(&pb_dir).await {
        Ok(mut entries) => {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry.path().extension().and_then(|e| e.to_str()) == Some("json") {
                    debug!("playbook store already has entries, skipping seed");
                    return;
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Directory doesn't exist yet — will be created by PlaybookStore
        }
        Err(e) => {
            warn!(error = %e, dir = %pb_dir.display(), "failed to read playbook dir");
            return;
        }
    }

    info!("playbook store empty — seeding with starter templates");

    let store = PlaybookStore::new(&pb_dir);

    let seeds: Vec<Playbook> = vec![
        {
            let mut pb = Playbook::new(
                "minimal-edit",
                "Make targeted edits to existing code. Keep diffs under 30 lines. Do not create new files unless explicitly required.",
            );
            pb.name = "Minimal Edit".to_string();
            pb.steps = vec![
                PlaybookStep::new(
                    0,
                    "Search codebase for the relevant function/type",
                    "search",
                    vec!["file_found".into()],
                ),
                PlaybookStep::new(
                    1,
                    "Read the target file to understand context",
                    "read_file",
                    vec!["context_loaded".into()],
                ),
                PlaybookStep::new(
                    2,
                    "Make the minimal edit that satisfies the requirement",
                    "edit_file",
                    vec!["file_modified".into()],
                ),
                PlaybookStep::new(
                    3,
                    "Verify the change compiles",
                    "run_command",
                    vec!["compile_success".into()],
                ),
            ];
            pb
        },
        {
            let mut pb = Playbook::new(
                "test-first",
                "Write or update tests first, then implement. Verify tests pass before finishing.",
            );
            pb.name = "Test First".to_string();
            pb.steps = vec![
                PlaybookStep::new(
                    0,
                    "Identify the test file for the target module",
                    "search",
                    vec!["test_file_found".into()],
                ),
                PlaybookStep::new(
                    1,
                    "Write a failing test that captures the requirement",
                    "edit_file",
                    vec!["test_added".into()],
                ),
                PlaybookStep::new(
                    2,
                    "Implement the code to make the test pass",
                    "edit_file",
                    vec!["implementation_done".into()],
                ),
                PlaybookStep::new(
                    3,
                    "Run the test suite and verify all tests pass",
                    "run_command",
                    vec!["tests_pass".into()],
                ),
            ];
            pb
        },
        {
            let mut pb = Playbook::new(
                "grep-before-write",
                "Search the codebase before writing new code. Check if the function/type already exists.",
            );
            pb.name = "Grep Before Write".to_string();
            pb.steps = vec![
                PlaybookStep::new(
                    0,
                    "Search for existing implementations of the target",
                    "search",
                    vec!["search_complete".into()],
                ),
                PlaybookStep::new(
                    1,
                    "If found, extend or modify rather than duplicate",
                    "read_file",
                    vec!["existing_found".into()],
                ),
                PlaybookStep::new(
                    2,
                    "Implement changes in the existing location",
                    "edit_file",
                    vec!["change_applied".into()],
                ),
                PlaybookStep::new(
                    3,
                    "Verify no duplicate definitions introduced",
                    "search",
                    vec!["no_duplicates".into()],
                ),
            ];
            pb
        },
        {
            let mut pb = Playbook::new(
                "wire-not-build",
                "Connect existing code rather than reimplementing. Check what already exists before creating anything new.",
            );
            pb.name = "Wire Not Build".to_string();
            pb.steps = vec![
                PlaybookStep::new(
                    0,
                    "Search for the target struct/function in the codebase",
                    "search",
                    vec!["target_found".into()],
                ),
                PlaybookStep::new(
                    1,
                    "Trace the call chain to find where it should be wired",
                    "read_file",
                    vec!["call_site_found".into()],
                ),
                PlaybookStep::new(
                    2,
                    "Add the function call or import at the correct call site",
                    "edit_file",
                    vec!["wired".into()],
                ),
                PlaybookStep::new(
                    3,
                    "Verify the feature is accessible via CLI or API",
                    "run_command",
                    vec!["feature_reachable".into()],
                ),
            ];
            pb
        },
        {
            let mut pb = Playbook::new(
                "compile-check-loop",
                "After every edit, run cargo check. Fix errors immediately before proceeding to the next change.",
            );
            pb.name = "Compile Check Loop".to_string();
            pb.steps = vec![
                PlaybookStep::new(
                    0,
                    "Make a single logical change",
                    "edit_file",
                    vec!["change_made".into()],
                ),
                PlaybookStep::new(
                    1,
                    "Run cargo check to verify compilation",
                    "run_command",
                    vec!["compile_success".into()],
                ),
                PlaybookStep::new(
                    2,
                    "If errors, fix them before proceeding",
                    "edit_file",
                    vec!["errors_fixed".into()],
                ),
                PlaybookStep::new(
                    3,
                    "Repeat until all changes are applied and compiling",
                    "run_command",
                    vec!["all_clean".into()],
                ),
            ];
            pb
        },
    ];

    for pb in &seeds {
        if let Err(err) = store.save(pb).await {
            warn!(playbook = %pb.id, error = %err, "failed to seed playbook");
        } else {
            debug!(playbook = %pb.id, "seeded playbook");
        }
    }

    info!(
        count = seeds.len(),
        "playbook store seeded with starter templates"
    );
}

// ─── Run Ledger Helpers ──────────────────────────────────────────────────

/// Append a single typed entry to the run ledger JSONL file.
/// Failures are logged but never propagated — the ledger is best-effort.
fn append_ledger_entry(path: &std::path::Path, kind: &str, data: &serde_json::Value) {
    let entry = serde_json::json!({
        "kind": kind,
        "ts": chrono::Utc::now().to_rfc3339(),
        "data": data,
    });
    let line = match serde_json::to_string(&entry) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "failed to serialize ledger entry");
            return;
        }
    };
    if let Err(e) = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(f, "{}", line)?;
        Ok(())
    })() {
        warn!(error = %e, path = %path.display(), "failed to append to run ledger");
    }
}

/// Persist a final summary entry for the run ledger at run completion.
/// This is a no-op if the ledger was never initialized.
fn persist_run_ledger(ledger: &Option<RunLedger>, path: &std::path::Path) {
    let Some(ledger) = ledger else { return };
    let summary = serde_json::json!({
        "kind": "run_summary",
        "ts": chrono::Utc::now().to_rfc3339(),
        "data": {
            "run_id": ledger.run_id,
            "started_at_ms": ledger.started_at_ms,
            "phase_transitions": ledger.phase_history.len(),
            "agent_outcomes": ledger.agent_outcomes.len(),
            "gate_runs": ledger.gate_runs.len(),
        },
    });
    let line = match serde_json::to_string(&summary) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "failed to serialize run ledger summary");
            return;
        }
    };
    if let Err(e) = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(f, "{}", line)?;
        Ok(())
    })() {
        warn!(error = %e, path = %path.display(), "failed to persist run ledger summary");
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────

/// Collect files modified or created since the last commit with diff stats.
///
/// Uses two git queries:
/// - `git diff --numstat HEAD` — tracked files with unstaged/staged changes
/// - `git status --porcelain` — includes untracked (`??`) files
///
/// The combined list is deduped and capped at 50 entries.
fn git_diff_entries_since_task_start(workdir: &Path) -> Vec<DiffEntry> {
    let mut entries: Vec<DiffEntry> = Vec::new();

    // Modified tracked files.
    if let Ok(output) = std::process::Command::new("git")
        .args(["diff", "--numstat", "HEAD"])
        .current_dir(workdir)
        .output()
    {
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let mut parts = line.splitn(3, '\t');
                let additions = parts
                    .next()
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or(0);
                let deletions = parts
                    .next()
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or(0);
                if let Some(path) = parts.next().map(str::trim).filter(|path| !path.is_empty()) {
                    entries.push(DiffEntry {
                        path: path.to_string(),
                        additions,
                        deletions,
                        summary: None,
                    });
                }
            }
        }
    }

    // Untracked files are not present in `git diff --numstat HEAD`.
    if let Ok(output) = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(workdir)
        .output()
    {
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("?? ") {
                    let path = trimmed.splitn(2, ' ').nth(1).unwrap_or("").trim();
                    if !path.is_empty() {
                        entries.push(DiffEntry {
                            path: path.to_string(),
                            additions: 0,
                            deletions: 0,
                            summary: Some("untracked".to_string()),
                        });
                    }
                }
            }
        }
    }

    // Dedup while preserving order, cap at 50.
    let mut seen = std::collections::HashSet::new();
    entries.retain(|entry| seen.insert(entry.path.clone()));
    entries.truncate(50);
    entries
}

fn normalized_task_git_path(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return None;
    }
    let trimmed = trimmed.strip_prefix("./").unwrap_or(trimmed);
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed.starts_with('/')
        || trimmed.starts_with("../")
        || trimmed.contains("/../")
    {
        return None;
    }
    Some(trimmed.trim_end_matches('/').to_string())
}

fn task_declared_git_paths(declared_files: &[String]) -> Vec<String> {
    let mut paths = declared_files
        .iter()
        .filter_map(|path| normalized_task_git_path(path))
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

fn task_path_allowed_by_declared_files(path: &str, declared_files: &[String]) -> bool {
    let Some(path) = normalized_task_git_path(path) else {
        return false;
    };
    task_declared_git_paths(declared_files)
        .iter()
        .any(|declared| path == *declared || path.starts_with(&format!("{declared}/")))
}

fn all_plans_terminal(executor: &ParallelExecutor, plans: &[Plan]) -> bool {
    plans
        .iter()
        .all(|p| executor.plan_state(&p.id).map_or(true, |s| s.is_terminal()))
}

fn completed_plan_ids(
    executor: &ParallelExecutor,
    task_index: &HashMap<String, HashMap<String, TaskDef>>,
) -> Vec<String> {
    task_index
        .keys()
        .filter(|plan_id| {
            executor
                .plan_state(plan_id)
                .is_some_and(|state| matches!(state.current_phase, PlanPhase::Complete))
        })
        .cloned()
        .collect()
}

fn gate_effect_key(plan_id: &str, task_id: &str, rung: u32, kind: GateCompletionKind) -> String {
    format!("{kind:?}:{plan_id}:{task_id}:{rung}")
}

/// Build enriched retry context for the agent after a gate failure.
///
/// Uses structured error classification from `roko_gate` to provide the agent
/// with a machine-readable analysis of what went wrong, alongside truncated
/// excerpts of the raw gate output and previous agent output.
fn build_gate_retry_context(
    gate_output: &str,
    prev_agent_output: &str,
    attempt_num: u32,
) -> String {
    let lower_gate_output = gate_output.to_ascii_lowercase();
    let classification_gate = if lower_gate_output.contains("test result: failed")
        || lower_gate_output.contains("assertion failed")
        || lower_gate_output.contains("panicked at")
    {
        "test"
    } else {
        "gate"
    };
    let classification = classify_gate_failure(classification_gate, gate_output);
    let analysis = render_failure_classification(&classification);

    let gate_excerpt = if gate_output.len() > 3000 {
        &gate_output[..3000]
    } else {
        gate_output
    };
    let agent_excerpt = if prev_agent_output.len() > 2000 {
        &prev_agent_output[..2000]
    } else {
        prev_agent_output
    };

    format!(
        "## IMPORTANT: Your previous attempt failed\n\n\
         Attempt {attempt_num} failed.\n\n\
         ### Error analysis\n{analysis}\n\n\
         ### Gate error output\n```\n{gate_excerpt}\n```\n\n\
         ### What you did last time\n```\n{agent_excerpt}\n```"
    )
}

/// Commit working tree changes for a completed task.
///
/// Only acts if there are uncommitted changes in the task's declared files.
/// Silently succeeds if git is not available or the workdir is not a git repo.
/// Uses `--no-verify` to avoid triggering hooks in generated workspaces.
fn commit_task_changes(
    workdir: &std::path::Path,
    plan_id: &str,
    task_id: &str,
    declared_files: &[String],
) {
    use std::process::Command;

    let paths = task_declared_git_paths(declared_files);
    if paths.is_empty() {
        debug!(
            %plan_id,
            %task_id,
            "task has no declared files -- skipping auto-commit"
        );
        return;
    }

    // Check if there are changes to commit in this task's declared write set.
    let mut status_cmd = Command::new("git");
    status_cmd
        .args(["status", "--porcelain", "--"])
        .args(&paths)
        .current_dir(workdir);
    let status = status_cmd.output();
    let has_changes = status
        .as_ref()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    if !has_changes {
        debug!(%plan_id, %task_id, "no uncommitted changes to commit");
        return;
    }

    let msg = format!("[roko] {plan_id}: {task_id} completed");
    let mut add_cmd = Command::new("git");
    add_cmd
        .args(["add", "--"])
        .args(&paths)
        .current_dir(workdir);
    let add = add_cmd.status();
    if add.is_err() || !add.as_ref().map(|s| s.success()).unwrap_or(false) {
        debug!(%plan_id, %task_id, "git add failed -- skipping commit");
        return;
    }
    let mut commit_cmd = Command::new("git");
    commit_cmd
        .args(["commit", "-m", &msg, "--no-verify", "--only", "--"])
        .args(&paths)
        .current_dir(workdir);
    let commit = commit_cmd.status();
    match commit {
        Ok(s) if s.success() => {
            info!(%plan_id, %task_id, "committed task changes to git");
        }
        _ => {
            debug!(%plan_id, %task_id, "git commit failed -- non-fatal");
        }
    }
}

/// Extract lessons from past post-gate reflections for a specific gate.
///
/// Returns up to 3 de-duplicated lessons from failed reflections with confidence
/// above 0.3. If the store file is missing or malformed, returns an empty vec.
fn lessons_from_post_gate_reflections(
    learn_dir: &Path,
    gate_name: &str,
    _task_id: &str,
) -> Vec<String> {
    let path = learn_dir.join("post-gate-reflections.json");
    let store = PostGateReflectionStore::load(&path);

    let mut lessons: Vec<String> = store
        .records
        .iter()
        .filter(|r| r.trigger_gate == gate_name)
        .filter(|r| matches!(r.outcome, ReflectionGateOutcome::Failed))
        .filter(|r| r.confidence > 0.3)
        .filter(|r| !r.proposed_lesson.is_empty())
        .map(|r| r.proposed_lesson.clone())
        .collect();

    lessons.dedup();
    lessons.truncate(3);
    lessons
}

fn build_report(executor: &ParallelExecutor, plans: &[Plan], state: &RunState) -> RunReport {
    let plan_reports: Vec<PlanReport> = plans
        .iter()
        .map(|p| {
            let orc_state = executor.plan_state(&p.id);
            let completed = orc_state
                .map(|s| matches!(s.current_phase, PlanPhase::Complete))
                .unwrap_or(false);
            PlanReport {
                plan_id: p.id.clone(),
                completed,
                tasks_total: p.tasks.tasks.len(),
                tasks_completed: if completed { p.tasks.tasks.len() } else { 0 },
                tasks_failed: if !completed && orc_state.map_or(false, |s| s.is_terminal()) {
                    1
                } else {
                    0
                },
                gate_results: orc_state
                    .map(|state| state.gate_results.clone())
                    .unwrap_or_default(),
            }
        })
        .collect();

    RunReport {
        plans: plan_reports,
        total_tasks: state.tasks_total,
        tasks_completed: state.tasks_completed,
        tasks_failed: state.tasks_failed,
        total_cost_usd: state.total_cost_usd,
        total_tokens_in: state.total_tokens_in,
        total_tokens_out: state.total_tokens_out,
        total_agent_calls: state.total_agent_calls,
        duration: state.elapsed(),
        failure_reasons: state.failure_reasons.clone(),
        task_costs: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_plan_verify_finalizes_runner_plan() {
        let mut executor = ParallelExecutor::new(ExecutorConfig::default());
        executor.add_plan(OrcPlanState::new("plan-verify"));

        executor
            .apply_event("plan-verify", &ExecutorEvent::Start)
            .unwrap();
        executor
            .apply_event("plan-verify", &ExecutorEvent::EnrichmentDone)
            .unwrap();
        executor
            .apply_event("plan-verify", &ExecutorEvent::ImplementationDone)
            .unwrap();
        executor
            .apply_event("plan-verify", &ExecutorEvent::GatePassed)
            .unwrap();

        let phase = complete_plan_after_successful_verify("plan-verify", &mut executor).unwrap();

        assert_eq!(phase, PlanPhase::Complete);
        assert!(executor.plan_state("plan-verify").unwrap().is_terminal());
        assert!(
            executor.tick().is_empty(),
            "completed plan must not request review/doc agents or rerun tasks"
        );
    }

    #[test]
    fn load_executor_resumes_from_unified_state_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let paths = persist::PersistPaths::from_workdir(dir.path()).unwrap();
        let mut snapshot = ExecutorSnapshot::new(123);
        snapshot
            .plan_states
            .insert("self-dev-ux".to_string(), OrcPlanState::new("self-dev-ux"));
        snapshot.queue_order.push("self-dev-ux".to_string());
        let orchestrator_json = OrchestratorSnapshot::new(snapshot.clone(), 123)
            .to_json()
            .unwrap();
        let unified = roko_runtime::StateSnapshot::new(
            123,
            snapshot.to_json().unwrap(),
            orchestrator_json,
            "{}".to_string(),
            "{}".to_string(),
        );
        persist::save_state_snapshot(&paths, &unified).unwrap();

        let resume = load_executor(
            &paths,
            &ExecutorConfig::default(),
            &["self-dev-ux".to_string()],
        );

        assert_eq!(resume.marker.outcome, ResumeOutcome::Resumed);
        assert_eq!(
            resume.marker.snapshot_path,
            paths.state_snapshot_json.display().to_string()
        );
        assert!(resume.executor.plan_state("self-dev-ux").is_some());
    }

    #[test]
    fn task_path_filter_only_allows_declared_files() {
        let declared = vec![
            "crates/roko-core/src/lib.rs".to_string(),
            "crates/roko-cli/src/runner".to_string(),
        ];

        assert!(task_path_allowed_by_declared_files(
            "crates/roko-core/src/lib.rs",
            &declared
        ));
        assert!(task_path_allowed_by_declared_files(
            "crates/roko-cli/src/runner/event_loop.rs",
            &declared
        ));
        assert!(!task_path_allowed_by_declared_files(
            "crates/roko-core/src/lib.rs.bak",
            &declared
        ));
        assert!(!task_path_allowed_by_declared_files(
            "../outside.rs",
            &declared
        ));
    }

    #[test]
    fn turn_budget_check_is_disabled_when_limit_is_zero() {
        assert!(!turn_exceeds_budget(Some(100.0), 0.0));
        assert!(!turn_exceeds_budget(None, 3.0));
        assert!(!turn_exceeds_budget(Some(3.0), 3.0));
        assert!(turn_exceeds_budget(Some(3.01), 3.0));
    }

    #[test]
    fn verification_only_task_does_not_emit_model_feedback() {
        let event = RunnerEvent::task_attempt_completed(
            "run-1",
            TaskAttemptRef::new("plan".to_string(), "task".to_string(), 1),
            TaskAttemptOutcome::Passed,
            None,
            123,
            "",
            "",
        );

        let feedback = runner_event_to_feedback(&event, &None, &TaskUsageSnapshot::default());

        assert!(
            feedback.is_none(),
            "verification-only tasks have no model to reward in the cascade router"
        );
    }

    #[test]
    fn commit_task_changes_only_commits_declared_files() {
        fn git(workdir: &std::path::Path, args: &[&str]) {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(workdir)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
                args,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init"]);
        git(
            dir.path(),
            &["config", "user.email", "roko@example.invalid"],
        );
        git(dir.path(), &["config", "user.name", "Roko Test"]);

        std::fs::write(dir.path().join("declared.txt"), "before\n").unwrap();
        std::fs::write(dir.path().join("unrelated.txt"), "before\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "initial"]);

        std::fs::write(dir.path().join("declared.txt"), "after\n").unwrap();
        std::fs::write(dir.path().join("unrelated.txt"), "after\n").unwrap();

        commit_task_changes(dir.path(), "plan", "task", &["declared.txt".to_string()]);

        let show = std::process::Command::new("git")
            .args(["show", "--name-only", "--format=", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(show.status.success());
        let committed_files = String::from_utf8_lossy(&show.stdout);
        assert!(committed_files.contains("declared.txt"));
        assert!(!committed_files.contains("unrelated.txt"));

        let status = std::process::Command::new("git")
            .args(["status", "--short"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(status.status.success());
        let status = String::from_utf8_lossy(&status.stdout);
        assert!(status.contains("unrelated.txt"));
        assert!(!status.contains("declared.txt"));
    }

    #[test]
    fn build_gate_retry_context_compile_error_produces_analysis() {
        let gate_output = "error[E0433]: failed to resolve: use of undeclared crate or module `foo`\n\
                           --> src/lib.rs:3:5\n  |\n3 | use foo::bar;\n  |     ^^^ use of undeclared crate or module `foo`";
        let agent_output = "I added `use foo::bar;` to import the module.";
        let result = build_gate_retry_context(gate_output, agent_output, 2);

        assert!(result.contains("## IMPORTANT: Your previous attempt failed"));
        assert!(result.contains("Attempt 2 failed."));
        assert!(result.contains("### Error analysis"));
        // The analysis should contain structured JSON with the classification
        assert!(result.contains("\"primary\""));
        assert!(result.contains("### Gate error output"));
        assert!(result.contains("error[E0433]"));
        assert!(result.contains("### What you did last time"));
        assert!(result.contains("use foo::bar"));
    }

    #[test]
    fn build_gate_retry_context_truncates_long_output() {
        let gate_output = "x".repeat(5000);
        let agent_output = "y".repeat(4000);
        let result = build_gate_retry_context(&gate_output, &agent_output, 1);

        // Gate output truncated to 3000 chars
        let gate_section_start = result.find("### Gate error output").unwrap();
        let agent_section_start = result.find("### What you did last time").unwrap();
        let gate_section = &result[gate_section_start..agent_section_start];
        // Count the 'x' chars in the gate section — should be 3000, not 5000
        let x_count = gate_section.chars().filter(|c| *c == 'x').count();
        assert_eq!(x_count, 3000);

        // Agent output truncated to 2000 chars
        let agent_section = &result[agent_section_start..];
        let agent_block_start = agent_section.find("```\n").unwrap() + "```\n".len();
        let agent_block = &agent_section[agent_block_start..];
        let agent_block_end = agent_block.find("\n```").unwrap();
        let y_count = agent_block[..agent_block_end]
            .chars()
            .filter(|c| *c == 'y')
            .count();
        assert_eq!(y_count, 2000);
    }

    #[test]
    fn build_gate_retry_context_test_output_preserves_test_names() {
        let gate_output = "test result: FAILED. 9 passed; 1 failed; 0 ignored\n\
                           failures:\n    tests::my_important_test\n\
                           thread 'tests::my_important_test' panicked at assertion failed: expected 42, got 0";
        let agent_output = "I implemented the function but forgot to handle the edge case.";
        let result = build_gate_retry_context(gate_output, agent_output, 3);

        assert!(result.contains("Attempt 3 failed."));
        assert!(result.contains("tests::my_important_test"));
        assert!(result.contains("assertion failed"));
        // Classification should identify this as a test failure
        assert!(result.contains("test_expectation_failure"));
    }
}

#[cfg(test)]
mod tests_post_gate_reflection_lessons {
    use super::*;
    use roko_learn::post_gate_reflection::{
        PostGateReflectionRecord, PostGateReflectionStore, ReflectionAdmissionStatus,
        ReflectionGateOutcome,
    };
    use tempfile::TempDir;

    fn make_record(
        gate: &str,
        outcome: ReflectionGateOutcome,
        confidence: f64,
        lesson: &str,
    ) -> PostGateReflectionRecord {
        PostGateReflectionRecord {
            reflection_id: format!("test-{}", lesson.len()),
            plan_id: None,
            task_id: Some("task-1".to_string()),
            episode_id: None,
            trigger_gate: gate.to_string(),
            outcome,
            failure_pattern_ids: vec![],
            pass_evidence: vec![],
            proposed_lesson: lesson.to_string(),
            confidence,
            evidence_count: 1,
            admission_status: ReflectionAdmissionStatus::Candidate,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn missing_store_path_returns_empty_vec() {
        let dir = TempDir::new().unwrap();
        let learn_dir = dir.path().join("nonexistent");
        let result = lessons_from_post_gate_reflections(&learn_dir, "compile", "task-1");
        assert!(result.is_empty());
    }

    #[test]
    fn matching_failed_records_are_included() {
        let dir = TempDir::new().unwrap();
        let learn_dir = dir.path();
        let store = PostGateReflectionStore {
            records: vec![
                make_record(
                    "compile",
                    ReflectionGateOutcome::Failed,
                    0.5,
                    "Fix type mismatch in handler",
                ),
                make_record(
                    "compile",
                    ReflectionGateOutcome::Failed,
                    0.6,
                    "Check import paths",
                ),
            ],
            candidates: vec![],
        };
        store
            .save(&learn_dir.join("post-gate-reflections.json"))
            .unwrap();

        let result = lessons_from_post_gate_reflections(learn_dir, "compile", "task-1");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "Fix type mismatch in handler");
        assert_eq!(result[1], "Check import paths");
    }

    #[test]
    fn only_top_3_lessons_returned() {
        let dir = TempDir::new().unwrap();
        let learn_dir = dir.path();
        let store = PostGateReflectionStore {
            records: vec![
                make_record("test", ReflectionGateOutcome::Failed, 0.5, "Lesson A"),
                make_record("test", ReflectionGateOutcome::Failed, 0.6, "Lesson B"),
                make_record("test", ReflectionGateOutcome::Failed, 0.7, "Lesson C"),
                make_record("test", ReflectionGateOutcome::Failed, 0.8, "Lesson D"),
                make_record("test", ReflectionGateOutcome::Failed, 0.9, "Lesson E"),
            ],
            candidates: vec![],
        };
        store
            .save(&learn_dir.join("post-gate-reflections.json"))
            .unwrap();

        let result = lessons_from_post_gate_reflections(learn_dir, "test", "task-1");
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn passed_outcomes_excluded() {
        let dir = TempDir::new().unwrap();
        let learn_dir = dir.path();
        let store = PostGateReflectionStore {
            records: vec![
                make_record(
                    "compile",
                    ReflectionGateOutcome::Passed,
                    0.8,
                    "Good approach",
                ),
                make_record("compile", ReflectionGateOutcome::Failed, 0.5, "Fix the bug"),
            ],
            candidates: vec![],
        };
        store
            .save(&learn_dir.join("post-gate-reflections.json"))
            .unwrap();

        let result = lessons_from_post_gate_reflections(learn_dir, "compile", "task-1");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "Fix the bug");
    }

    #[test]
    fn low_confidence_records_excluded() {
        let dir = TempDir::new().unwrap();
        let learn_dir = dir.path();
        let store = PostGateReflectionStore {
            records: vec![
                make_record(
                    "compile",
                    ReflectionGateOutcome::Failed,
                    0.2,
                    "Low confidence",
                ),
                make_record(
                    "compile",
                    ReflectionGateOutcome::Failed,
                    0.5,
                    "High confidence",
                ),
            ],
            candidates: vec![],
        };
        store
            .save(&learn_dir.join("post-gate-reflections.json"))
            .unwrap();

        let result = lessons_from_post_gate_reflections(learn_dir, "compile", "task-1");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "High confidence");
    }

    #[test]
    fn different_gate_records_excluded() {
        let dir = TempDir::new().unwrap();
        let learn_dir = dir.path();
        let store = PostGateReflectionStore {
            records: vec![
                make_record("test", ReflectionGateOutcome::Failed, 0.5, "Test lesson"),
                make_record(
                    "compile",
                    ReflectionGateOutcome::Failed,
                    0.5,
                    "Compile lesson",
                ),
            ],
            candidates: vec![],
        };
        store
            .save(&learn_dir.join("post-gate-reflections.json"))
            .unwrap();

        let result = lessons_from_post_gate_reflections(learn_dir, "compile", "task-1");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "Compile lesson");
    }
}
