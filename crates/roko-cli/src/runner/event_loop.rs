//! Core event loop — drives the executor via `tokio::select!` over agent
//! events, gate completions, executor ticks, periodic flushes, and
//! cancellation.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::state_hub::StateHub;
use anyhow::{Context, Result};
use roko_core::agent::ModelSpec;
use roko_core::{AgentRole, PhaseKind, PlanPhase};
use roko_orchestrator::{
    ExecutorAction, ExecutorConfig, ExecutorEvent, ExecutorSnapshot, GateResult, MergeQueue,
    MergeRequest, OrchestratorSnapshot, ParallelExecutor, PlanState as OrcPlanState,
    RecoveryEngine,
};
use tokio::sync::mpsc;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::dispatch::model_routing::tier_to_complexity;
use crate::dispatch::{
    AgentDispatchRequest, DispatchContext, GateFeedback as DispatchGateFeedback, PromptCache,
    ResolvedAgentRuntime, SharedAgentFactory,
};
use crate::inline::DiffEntry;
use crate::knowledge_helpers::{build_knowledge_routing_advice, neuro_prompt_task_category};
use crate::task_parser::TaskDef;
use roko_neuro::KnowledgeStore;

use super::agent_events::{AgentStreamBuffer, handle_agent_event};
use super::agent_stream::{AgentHandle, AgentSpawnConfig};
use super::gate_dispatch;
use super::inline_output::RunnerInlineTerminal;
use super::merge::{MergeDispatch, PlanMerger, PlanMergerConfig};
use super::persist::{self, GateThresholds, PersistPaths};
use super::plan_loader::Plan;
use super::snapshot_writer::{SnapshotPayload, SnapshotWriter};
use super::state::RunState;
use super::tui_bridge::TuiBridge;
use super::types::{
    AgentCompletionSummary, AgentDispatchOutcome, AgentEvent, GateCompletion, GateCompletionKind,
    PlanOutcome, PlanRunSummary, PromptAssemblyDiagnostics, ResumeMarker, ResumeOutcome,
    RetryAction, RunConfig, RunOutcome, RunTotals, RunnerEvent, RunnerFailureKind,
    TaskAttemptOutcome, TaskAttemptRef, TaskAttemptStatus,
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
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX).max(1)
}

fn agent_dispatch_timeout(config: &RunConfig) -> Duration {
    config
        .roko_config
        .as_deref()
        .map_or_else(|| Duration::from_secs(config.timeout_secs), |cfg| {
            cfg.timeouts.agent_dispatch()
        })
}

fn plan_total_timeout(config: &RunConfig) -> Duration {
    config
        .roko_config
        .as_deref()
        .map_or_else(|| Duration::from_secs(config.plan_timeout_secs), |cfg| {
            cfg.timeouts.plan_total()
        })
}

fn llm_call_timeout(config: &RunConfig) -> Duration {
    config
        .roko_config
        .as_deref()
        .map_or_else(|| roko_core::config::TimeoutConfig::default().llm_call(), |cfg| {
            cfg.timeouts.llm_call()
        })
}

fn gate_timeout(config: &RunConfig, rung: u32) -> Duration {
    config
        .roko_config
        .as_deref()
        .map_or_else(|| Duration::from_secs(config.timeout_secs), |cfg| match rung {
            0 => cfg.timeouts.gate_compile(),
            1 => cfg.timeouts.gate_clippy(),
            _ => cfg.timeouts.gate_test(),
        })
}

// ─── RunContext ──────────────────────────────────────────────────────────

/// Shared context for the dispatch loop, replacing 11 loose parameters.
struct RunContext<'a> {
    executor: &'a mut ParallelExecutor,
    task_index: &'a HashMap<String, HashMap<String, TaskDef>>,
    skip_enrichment: &'a HashMap<String, bool>,
    config: &'a RunConfig,
    inline: &'a mut RunnerInlineTerminal,
    tui: &'a TuiBridge,
    state: &'a mut RunState,
    agent_handles: &'a mut HashMap<String, AgentHandle>,
    agent_tx: &'a mpsc::Sender<AgentEvent>,
    gate_tx: &'a mpsc::Sender<GateCompletion>,
    fatal_tx: mpsc::Sender<AgentEvent>,
    paths: &'a PersistPaths,
    merge_queue: &'a MergeQueue,
    snapshot_writer: &'a SnapshotWriter,
    prompt_cache: &'a Arc<PromptCache>,
    factory: &'a SharedAgentFactory,
    gate_sem: Arc<tokio::sync::Semaphore>,
}

// ─── Main Entry Point ───────────────────────────────────────────────────

/// Run all plans to completion (or cancellation).
pub async fn run(
    plans: Vec<Plan>,
    config: &RunConfig,
    state_hub: &StateHub,
    cancel: CancellationToken,
) -> Result<RunReport> {
    let max_concurrent_tasks = config.max_concurrent_tasks.max(1);
    let task_timeout_secs = duration_secs(agent_dispatch_timeout(config));

    let exec_config = ExecutorConfig {
        max_concurrent_plans: 4,
        max_concurrent_tasks,
        max_auto_fix_iterations: config.max_retries,
        task_timeout_secs,
        ..Default::default()
    };

    let mut config = config.clone();
    let paths = PersistPaths::from_workdir(&config.workdir)?;
    let snapshot_writer = SnapshotWriter::new(4);
    persist::cleanup_orphaned_agents(&paths);
    let mut gate_thresholds = persist::load_gate_thresholds(&paths).unwrap_or_default();

    // Ensure knowledge store directory exists for episode ingestion.
    let neuro_dir = config.workdir.join(".roko").join("neuro");
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
    let prior_snapshot = match persist::load_run_state(&paths) {
        Ok(Some(snapshot)) => Some(snapshot),
        Ok(None) => None,
        Err(err) => {
            warn!(
                error = %err,
                "failed to read prior run-state.json; continuing without seeded resume state"
            );
            None
        }
    };
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
    let mut inline = RunnerInlineTerminal::new(config.stream_to_stderr);

    // -- Warm cargo cache -------------------------------------------------------
    // Run `cargo check --workspace` once before the main loop so that
    // subsequent per-task compile gates are incremental (2-5s vs 30-120s).
    if config.warm_cache {
        inline.warm_cache_started();
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
                inline.warm_cache_completed(warm_ms);
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
    seed_playbooks_if_empty(&config.workdir).await;

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
    let mut stream_buf = AgentStreamBuffer::new();
    let mut dream_completion_pending = false;

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
    let mut feedback_tasks: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();

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
        stream_to_stderr = config.stream_to_stderr,
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

                handle_agent_event(&event, &mut state, &tui, &mut inline, &mut stream_buf);
                append_agent_event(&paths, &event, &state);

                // Per-turn budget enforcement.
                if is_turn_done {
                    let max_turn = config.max_turn_usd;
                    if max_turn > 0.0 && state.cost_usd > max_turn {
                        warn!(
                            task = %state.current_task,
                            turn_cost = state.cost_usd,
                            limit = max_turn,
                            "single turn exceeded per-turn budget limit -- stopping agent"
                        );
                        stop_all_agents(&mut agent_handles, &mut state, Duration::from_secs(3)).await;
                        let plan_id = state.plan_id.clone();
                        if !plan_id.is_empty() {
                            let _ = executor.apply_event(
                                &plan_id,
                                &ExecutorEvent::Fatal(format!(
                                    "turn cost ${:.2} exceeded per-turn limit ${:.2}",
                                    state.cost_usd, max_turn,
                                )),
                            );
                        }
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
                            let message = "agent reported an error result".to_string();
                            fire_on_error_hook(config, &message, "agent_turn", &tui, &state.plan_id, &state.current_task).await;
                            tui.error(&message);
                            let _ = executor.apply_event(&plan_id, &ExecutorEvent::Fatal(message));
                        } else {
                            apply_agent_completion(&mut executor, &plan_id, &tui);
                        }
                        save_snapshot(config, &executor, &paths, &mut state, &merge_queue, &snapshot_writer);
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
                            tui.error(&message);
                            let _ = executor.apply_event(&plan_id, &ExecutorEvent::Fatal(message));
                        }
                    }

                    save_snapshot(config, &executor, &paths, &mut state, &merge_queue, &snapshot_writer);
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
                }

                inline.gate_completed(&completion);
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
                    &paths.gate_thresholds_json,
                    completion.rung,
                    completion.passed,
                );
                emit_gate_thresholds_event(&gate_thresholds, &tui);

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
                    let signals_path = config.workdir.join(".roko/signals.jsonl");
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

                if completion.passed {
                    // Mark this task completed in the DAG and check for more.
                    state.clear_retry_backoff(&completion.plan_id);
                    state.mark_task_completed(&completion.plan_id, &completion.task_id);
                    // Snapshot which files this task produced so downstream
                    // tasks can be told what their dependencies already created.
                    let output_diffs = git_diff_entries_since_task_start(&config.workdir);
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
                    tui.task_completed(&completion.plan_id, &completion.task_id, "passed");

                    // Commit generated code to git so subsequent tasks can diff.
                    commit_task_changes(
                        &config.workdir,
                        &completion.plan_id,
                        &completion.task_id,
                    );

                    let total_task_ms = state.task_elapsed_ms();
                    let dispatch_ms = state.last_dispatch_ms;
                    let gate_ms = completion.duration_ms;
                    let agent_ms = total_task_ms.saturating_sub(dispatch_ms + gate_ms);

                    inline.diff_block(&output_diffs);
                    inline.task_done(state.tasks_completed, state.tasks_total, total_task_ms);
                    info!(
                        task = %completion.task_id,
                        dispatch_ms,
                        agent_ms,
                        gate_ms,
                        "task timing"
                    );

                    let completed = state.plan_completed_tasks(&completion.plan_id);
                    let completed_plans = completed_plan_ids(&executor, &task_index);
                    let has_more = task_index
                        .get(completion.plan_id.as_str())
                        .map(|tasks| {
                            tasks
                                .values()
                                .any(|t| {
                                    !completed.contains(&t.id)
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
                            .map(|t| t.len().saturating_sub(completed.len())).unwrap_or(0);
                        info!(
                            plan_id = %completion.plan_id,
                            remaining,
                            "task passed — advancing to next task"
                        );
                    } else {
                        // All tasks done — let the plan proceed to completion.
                        let _ = executor.apply_event(&completion.plan_id, &ExecutorEvent::GatePassed);
                        info!(plan_id = %completion.plan_id, "all tasks passed — plan completing");

                        // Queue dream consolidation for the post-run drain.
                        dream_completion_pending = true;
                        debug!("dream consolidation queued after plan completion");
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

                                inline.gate_retry(
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
                                    let gate_output: String = completion.output.chars().take(3000).collect();
                                    let agent_prev: String = state.agent_output.chars().take(2000).collect();
                                    let strategy_hint = if state.iteration_for(&completion.plan_id, &completion.task_id) >= 3 {
                                        "Your previous approaches have failed multiple times. \
                                         You MUST try a fundamentally different strategy."
                                    } else {
                                        "Try a different approach than your previous attempt."
                                    };
                                    let replan_context = format!(
                                        "\n\n## IMPORTANT: Your previous attempt failed\n\n\
                                         This is attempt {attempt_num}.\n\n\
                                         ### Gate error output\n```\n{gate_output}\n```\n\n\
                                         ### What you did last time\n```\n{agent_prev}\n```\n\n\
                                         {strategy_hint}",
                                    );
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

                        inline.task_failed(&reason);
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
                        let _ = executor.apply_event(
                            &completion.plan_id,
                            &ExecutorEvent::Fatal(reason.clone()),
                        );
                        tui.error(&reason);
                    }
                }

                save_snapshot(config, &executor, &paths, &mut state, &merge_queue, &snapshot_writer);
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
                        inline: &mut inline,
                        tui: &tui,
                        state: &mut state,
                        agent_handles: &mut agent_handles,
                        agent_tx: &agent_tx,
                        gate_tx: &gate_tx,
                        fatal_tx: agent_tx.clone(),
                        paths: &paths,
                        merge_queue: &merge_queue,
                        snapshot_writer: &snapshot_writer,
                        prompt_cache: &prompt_cache,
                        factory: &factory,
                        gate_sem: gate_sem.clone(),
                    };
                    dispatch_action(&action, &mut ctx).await;
                    let dispatch_ms = t_dispatch.elapsed().as_millis() as u64;
                    if matches!(&action, ExecutorAction::SpawnAgent { .. }) {
                        ctx.state.last_dispatch_ms = dispatch_ms;
                        info!(action = %action_label, dispatch_ms, "agent action dispatched");
                    } else if dispatch_ms > 50 {
                        info!(action = %action_label, dispatch_ms, "action dispatched (slow)");
                    } else {
                        debug!(action = %action_label, dispatch_ms, "action dispatched");
                    }
                }
            }

            // ─── Branch 4: Periodic flush ───────────────────────────
            _ = flush_interval.tick() => {
                save_snapshot(config, &executor, &paths, &mut state, &merge_queue, &snapshot_writer);
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
                    &snapshot_writer,
                )
                .await?;
                timed_out = true;
            }

            // ─── Branch 6: Cancellation ─────────────────────────────
            _ = cancel.cancelled() => {
                warn!("cancellation requested — shutting down");
                stop_all_agents(&mut agent_handles, &mut state, Duration::from_secs(3)).await;
                save_snapshot(config, &executor, &paths, &mut state, &merge_queue, &snapshot_writer);
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
            break;
        }
    }

    // Drain any pending feedback tasks.
    while feedback_tasks.try_join_next().is_some() {}

    // Ensure all pending snapshots land on disk before returning.
    snapshot_writer.flush();

    let report = build_report(&executor, &plans, &state);

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

fn handle_plan_verify_completion(
    completion: &GateCompletion,
    executor: &mut ParallelExecutor,
    state: &mut RunState,
    paths: &PersistPaths,
    merge_queue: &MergeQueue,
    tui: &TuiBridge,
    config: &RunConfig,
    writer: &SnapshotWriter,
) {
    if completion.passed {
        state.clear_retry_backoff(&completion.plan_id);
        match executor.apply_event(&completion.plan_id, &ExecutorEvent::VerifyPassed) {
            Ok(phase) => {
                tui.phase_transition(&completion.plan_id, "verifying", &format!("{phase:?}"));
                info!(plan_id = %completion.plan_id, phase = ?phase, "plan verify passed");
            }
            Err(e) => warn!(
                plan_id = %completion.plan_id,
                err = %e,
                "transition error after plan verify pass"
            ),
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

    save_snapshot(config, executor, paths, state, merge_queue, writer);
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
    save_snapshot(config, executor, paths, state, merge_queue, writer);
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
    emit_runner_event_with_facades(paths, state, tui, None, None, event, None);
}

/// Internal variant accepting the optional projection + feedback facades.
fn emit_runner_event_with_facades(
    paths: &PersistPaths,
    state: &mut RunState,
    tui: &TuiBridge,
    projection: Option<&Arc<super::projection::Projection>>,
    feedback_facade: Option<&Arc<crate::runtime_feedback::FeedbackFacade>>,
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

    // ── Translate to FeedbackEvent and fan out ──────────────────────────
    if let Some(facade) = feedback_facade {
        let usage = TaskUsageSnapshot {
            tokens_in: state.tokens_in,
            tokens_out: state.tokens_out,
            cost_usd: state.cost_usd,
            duration_ms: state.task_elapsed_ms(),
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
            let succeeded = matches!(outcome, TaskAttemptOutcome::Passed);
            let agent_outcome = AgentOutcome {
                task_id: attempt.task_id.clone(),
                plan_id: attempt.plan_id.clone(),
                model: model.clone(),
                provider: provider.clone(),
                output: String::new(),
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

/// Build a [`SnapshotPayload`] from current state and enqueue it on the
/// async writer. Serialisation (<1ms) happens on the caller's thread;
/// the actual disk I/O runs on the dedicated writer thread.
fn save_snapshot(
    config: &RunConfig,
    executor: &ParallelExecutor,
    paths: &PersistPaths,
    state: &mut RunState,
    merge_queue: &MergeQueue,
    writer: &SnapshotWriter,
) {
    let timestamp_ms = chrono::Utc::now().timestamp_millis() as u64;
    let snapshot = executor.snapshot(timestamp_ms);
    let orchestrator_snapshot = OrchestratorSnapshot::new(snapshot.clone(), timestamp_ms)
        .with_merge_queue(merge_queue.snapshot());

    let orchestrator_json = match orchestrator_snapshot.to_json() {
        Ok(json) => json.into_bytes(),
        Err(e) => {
            error!(error = %e, "failed to serialize orchestrator snapshot");
            state.snapshot_failed();
            return;
        }
    };
    let executor_json = match serde_json::to_string_pretty(&snapshot) {
        Ok(json) => json.into_bytes(),
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
        Ok(json) => json.into_bytes(),
        Err(e) => {
            error!(error = %e, "failed to serialize run-state snapshot");
            state.snapshot_failed();
            return;
        }
    };

    writer.write(SnapshotPayload {
        orchestrator_json,
        orchestrator_path: paths.orchestrator_json.clone(),
        executor_json,
        executor_path: paths.executor_json.clone(),
        run_state_json,
        run_state_path: paths.run_state_json.clone(),
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
    let snapshot_path = if paths.orchestrator_json.exists() {
        paths.orchestrator_json.display().to_string()
    } else {
        paths.executor_json.display().to_string()
    };
    if !paths.orchestrator_json.exists() && !paths.executor_json.exists() {
        return ResumeLoad {
            executor: ParallelExecutor::new(config.clone()),
            merge_queue: MergeQueue::new(),
            marker: ResumeMarker {
                outcome: ResumeOutcome::Fresh,
                snapshot_path,
                snapshot_plan_ids: Vec::new(),
                current_plan_ids: plan_ids.to_vec(),
                message: Some("no executor snapshot found".to_string()),
            },
        };
    }

    let (snapshot, merge_queue) = match load_orchestrator_checkpoint(paths) {
        Ok(Some((snapshot, merge_queue))) => (snapshot, merge_queue),
        Ok(None) => {
            let json = match std::fs::read_to_string(&paths.executor_json) {
                Ok(j) => j,
                Err(e) => {
                    warn!(err = %e, "failed to read executor snapshot");
                    return ResumeLoad {
                        executor: ParallelExecutor::new(config.clone()),
                        merge_queue: MergeQueue::new(),
                        marker: ResumeMarker {
                            outcome: ResumeOutcome::ReadFailed,
                            snapshot_path,
                            snapshot_plan_ids: Vec::new(),
                            current_plan_ids: plan_ids.to_vec(),
                            message: Some(format!("failed to read executor snapshot: {e}")),
                        },
                    };
                }
            };
            match ExecutorSnapshot::from_json(&json) {
                Ok(snapshot) => (snapshot, MergeQueue::new()),
                Err(e) => {
                    warn!(err = %e, "corrupt executor snapshot — starting fresh");
                    return ResumeLoad {
                        executor: ParallelExecutor::new(config.clone()),
                        merge_queue: MergeQueue::new(),
                        marker: ResumeMarker {
                            outcome: ResumeOutcome::Corrupt,
                            snapshot_path,
                            snapshot_plan_ids: Vec::new(),
                            current_plan_ids: plan_ids.to_vec(),
                            message: Some(format!("corrupt executor snapshot: {e}")),
                        },
                    };
                }
            }
        }
        Err(e) => {
            warn!(err = %e, "corrupt orchestrator snapshot — starting fresh");
            return ResumeLoad {
                executor: ParallelExecutor::new(config.clone()),
                merge_queue: MergeQueue::new(),
                marker: ResumeMarker {
                    outcome: ResumeOutcome::Corrupt,
                    snapshot_path,
                    snapshot_plan_ids: Vec::new(),
                    current_plan_ids: plan_ids.to_vec(),
                    message: Some(format!("corrupt orchestrator snapshot: {e}")),
                },
            };
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
        "resuming from orchestrator snapshot"
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

fn load_orchestrator_checkpoint(
    paths: &PersistPaths,
) -> Result<Option<(ExecutorSnapshot, MergeQueue)>, String> {
    if !paths.orchestrator_json.exists() {
        return Ok(None);
    }
    let json = std::fs::read_to_string(&paths.orchestrator_json)
        .map_err(|err| format!("failed to read aggregate snapshot: {err}"))?;
    let snapshot = OrchestratorSnapshot::from_json(&json)
        .map_err(|err| format!("failed to parse aggregate snapshot: {err}"))?;
    let merge_queue = snapshot
        .merge_queue
        .map(MergeQueue::from_snapshot)
        .unwrap_or_else(MergeQueue::new);
    Ok(Some((snapshot.executor, merge_queue)))
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

async fn dispatch_action(action: &ExecutorAction, ctx: &mut RunContext<'_>) {
    match action {
        ExecutorAction::DispatchPlan { plan_id } => {
            info!(plan_id = %plan_id, "dispatching plan");
            ctx.tui.plan_started(plan_id);

            if let Err(e) = ctx.executor.apply_event(plan_id, &ExecutorEvent::Start) {
                error!(plan_id = %plan_id, err = %e, "failed to start plan");
                return;
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
        }

        ExecutorAction::SpawnAgent { plan_id, task, .. } => {
            // Resolve sentinel task names ("next", "fix", etc.) to actual task IDs
            // by walking the plan's DAG and finding the first ready task.
            let resolved_task = if task == "next" || task == "fix" || task == "regen-verify" {
                let completed = ctx.state.plan_completed_tasks(plan_id);
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
                    return;
                }
            };

            if ctx.agent_handles.contains_key(plan_id.as_str()) {
                debug!(
                    plan_id = %plan_id,
                    task = %task_id,
                    "agent already active for this plan — suppressing duplicate spawn"
                );
                return;
            }

            if let Some(remaining) = ctx.state.retry_cooldown_remaining(plan_id) {
                debug!(
                    plan_id = %plan_id,
                    task = %task_id,
                    cooldown_ms = remaining.as_millis(),
                    "retry backoff active — delaying spawn"
                );
                return;
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
                return;
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
                    return;
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
            ctx.state.total_agent_calls += 1;
            ctx.state.task_agent_calls += 1;

            let role = task_def.role.as_deref().unwrap_or("implementer");
            let role_enum = parse_dispatch_role(role);
            let task_category = neuro_prompt_task_category(role_enum);

            ctx.inline
                .task_started(&task_id, role, &task_def.title, attempt_num);
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
            let routing_context = {
                use roko_core::{BehavioralState, DaimonPolicy};
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
                    daimon_policy: DaimonPolicy::new(0.5, BehavioralState::Engaged),
                    thinking_level: None,
                    temperament: None,
                    previous_model: None,
                    plan_context_tokens: None,
                    tier_thresholds: None,
                }
            };
            let dispatch_ctx = DispatchContext {
                plan_id: plan_id.clone(),
                role: role.to_string(),
                workdir: ctx.config.workdir.clone(),
                model_hint: Some(ctx.config.model.clone()),
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
                    return;
                }
            };
            let baseline_model = dispatch_plan.model.slug.clone();
            let baseline_score = knowledge_advice.score_for(&baseline_model);
            let mut selected_source = "dispatcher";
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
                    selected_source = "dispatcher+knowledge";
                }
            }
            let requested_model = dispatch_plan.model.slug.clone();
            let prompt_diagnostics = dispatch_plan.prompt.diagnostics.clone();
            ctx.tui
                .model_selected(plan_id, &task_id, &requested_model, selected_source);
            let system_prompt = dispatch_plan.prompt.system_prompt;
            let mut final_prompt = dispatch_plan.prompt.user_prompt;
            info!(
                plan_id = %plan_id,
                task = %task_id,
                model = %requested_model,
                source = selected_source,
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
                            return;
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
                            register_agent_feed(ctx.config, plan_id, &task_id, &agent_id, ctx.tui);
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
                            ctx.tui.error(&message);
                            if let Err(e2) = ctx.executor.apply_event(
                                plan_id,
                                &ExecutorEvent::Fatal(format!("spawn failed: {e}")),
                            ) {
                                error!(plan_id = %plan_id, error = %e2,
                                    "failed to apply Fatal event -- forcing plan terminal");
                                ctx.state.force_plan_terminal(plan_id);
                            }
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
                        effort: None,
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
                    register_agent_feed(ctx.config, plan_id, &task_id, &agent_id, ctx.tui);
                }
            }
        }

        ExecutorAction::RunGate { plan_id, rung } => {
            let task_id = ctx.state.current_task.clone();
            // Skip compile/clippy/test gates when no Cargo.toml exists (greenfield workspace).
            if *rung <= 2 && !ctx.config.workdir.join("Cargo.toml").exists() {
                info!(plan_id = %plan_id, rung = rung, "skipping cargo gate (no Cargo.toml in workspace)");
                record_skipped_gate_rung(
                    ctx,
                    plan_id,
                    &task_id,
                    *rung,
                    "cargo",
                    "skipped: no Cargo.toml in workspace",
                );
                return;
            }
            // Honor gates config: skip clippy rung (1) if disabled, skip test rung (2) if skip_tests.
            if *rung == 1 && !ctx.config.clippy_enabled {
                info!(plan_id = %plan_id, rung = rung, "skipping clippy gate (disabled in config)");
                record_skipped_gate_rung(
                    ctx,
                    plan_id,
                    &task_id,
                    *rung,
                    "clippy",
                    "skipped: clippy disabled in config",
                );
                return;
            }
            if *rung == 2 && ctx.config.skip_tests {
                info!(plan_id = %plan_id, rung = rung, "skipping test gate (skip_tests in config)");
                record_skipped_gate_rung(
                    ctx,
                    plan_id,
                    &task_id,
                    *rung,
                    "test",
                    "skipped: tests disabled in config",
                );
                return;
            }

            info!(plan_id = %plan_id, rung = rung, "dispatching gate");
            let effect_key = gate_effect_key(plan_id, &task_id, *rung, GateCompletionKind::Gate);
            if !ctx.state.mark_gate_active(effect_key.clone()) {
                debug!(
                    plan_id = %plan_id,
                    task_id = %task_id,
                    rung = rung,
                    "gate already active — suppressing duplicate dispatch"
                );
                return;
            }
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
                    *rung,
                ),
            );
            let task_def = ctx
                .task_index
                .get(plan_id.as_str())
                .and_then(|tasks| tasks.get(task_id.as_str()));
            let is_read_only_role = task_def.and_then(|t| t.role.as_deref()).map_or(false, |r| {
                matches!(r, "researcher" | "strategist" | "quick-reviewer")
            });

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
                    rung = rung,
                    "skipping gate for read-only role"
                );
                let completion = GateCompletion {
                    plan_id: plan_id.clone(),
                    task_id: task_id.clone(),
                    rung: *rung,
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
                gate_dispatch::spawn_gate(
                    plan_id.clone(),
                    task_id,
                    *rung,
                    ctx.config.workdir.clone(),
                    verify_steps,
                    duration_secs(gate_timeout(ctx.config, *rung)),
                    ctx.gate_tx.clone(),
                    ctx.gate_sem.clone(),
                );
            }
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
                let _ = ctx
                    .executor
                    .apply_event(plan_id, &ExecutorEvent::VerifyPassed);
                return;
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
                return;
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
                ctx.snapshot_writer,
            );
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
                PlanMergerConfig::new(
                    ctx.config.workdir.clone(),
                    gate_timeout(ctx.config, 0),
                ),
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
                        ctx.snapshot_writer,
                    );
                }
            }
        }

        _ => {
            info!(action = ?action, "auto-advancing action");
        }
    }
}

// ─── Adaptive gate thresholds ────────────────────────────────────────────

/// Update EMA-based adaptive gate thresholds for a given rung.
fn update_gate_thresholds(thresholds: &mut GateThresholds, path: &Path, rung: u32, passed: bool) {
    thresholds.observe(rung, passed);
    if let Err(err) = thresholds.save(path) {
        warn!(
            error = %err,
            path = %path.display(),
            rung,
            passed,
            "failed to persist adaptive gate thresholds"
        );
    }
}

/// Emit gate thresholds into the TUI push pipeline after persisting to disk.
fn emit_gate_thresholds_event(thresholds: &GateThresholds, tui: &TuiBridge) {
    if let Ok(json) = serde_json::to_string(thresholds) {
        tui.gate_thresholds_updated(&json);
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
        let router_path = config
            .workdir
            .join(".roko")
            .join("learn")
            .join("cascade-router.json");
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

    if !episodes_path.exists() {
        return;
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
    save_snapshot(config, executor, paths, state, merge_queue, writer);
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
async fn seed_playbooks_if_empty(workdir: &Path) {
    use roko_learn::playbook::{Playbook, PlaybookStep, PlaybookStore};

    let pb_dir = workdir.join(".roko").join("learn").join("playbooks");

    // Quick check: if the directory exists and has any .json files, skip.
    if pb_dir.exists() {
        if let Ok(mut entries) = tokio::fs::read_dir(&pb_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry.path().extension().and_then(|e| e.to_str()) == Some("json") {
                    debug!("playbook store already has entries, skipping seed");
                    return;
                }
            }
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

/// Commit working tree changes for a completed task.
///
/// Only acts if there are uncommitted changes. Silently succeeds if git is
/// not available or the workdir is not a git repo. Uses `--no-verify` to
/// avoid triggering hooks in generated workspaces.
fn commit_task_changes(workdir: &std::path::Path, plan_id: &str, task_id: &str) {
    use std::process::Command;

    // Check if there are changes to commit
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(workdir)
        .output();
    let has_changes = status
        .as_ref()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    if !has_changes {
        debug!(%plan_id, %task_id, "no uncommitted changes to commit");
        return;
    }

    let msg = format!("[roko] {plan_id}: {task_id} completed");
    let add = Command::new("git")
        .args(["add", "-A"])
        .current_dir(workdir)
        .status();
    if add.is_err() || !add.as_ref().map(|s| s.success()).unwrap_or(false) {
        debug!(%plan_id, %task_id, "git add failed -- skipping commit");
        return;
    }
    let commit = Command::new("git")
        .args(["commit", "-m", &msg, "--no-verify"])
        .current_dir(workdir)
        .status();
    match commit {
        Ok(s) if s.success() => {
            info!(%plan_id, %task_id, "committed task changes to git");
        }
        _ => {
            debug!(%plan_id, %task_id, "git commit failed -- non-fatal");
        }
    }
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
    }
}
