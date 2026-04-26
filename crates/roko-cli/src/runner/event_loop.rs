//! Core event loop — drives the executor via `tokio::select!` over agent
//! events, gate completions, executor ticks, periodic flushes, and
//! cancellation.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use roko_core::PlanPhase;
use roko_core::state_hub::StateHub;
use roko_learn::efficiency::AgentEfficiencyEvent;
use roko_learn::episode_logger::{Episode, GateVerdict, Usage};
use roko_orchestrator::{
    ExecutorAction, ExecutorConfig, ExecutorEvent, ExecutorSnapshot, ParallelExecutor,
    PlanState as OrcPlanState,
};
use tokio::sync::mpsc;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::task_parser::TaskDef;

use super::agent_events::handle_agent_event;
use super::agent_stream::{self, AgentHandle, AgentSpawnConfig};
use super::gate_dispatch;
use super::persist::{self, PersistPaths};
use super::plan_loader::Plan;
use super::state::RunState;
use super::tui_bridge::TuiBridge;
use super::types::{AgentEvent, GateCompletion, RunConfig};

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
}

/// Per-plan report.
#[derive(Debug, Clone)]
pub struct PlanReport {
    pub plan_id: String,
    pub completed: bool,
    pub tasks_total: usize,
    pub tasks_completed: usize,
    pub tasks_failed: usize,
}

impl RunReport {
    pub fn all_succeeded(&self) -> bool {
        self.tasks_failed == 0 && self.plans.iter().all(|p| p.completed)
    }
}

// ─── RunContext ──────────────────────────────────────────────────────────

/// Shared context for the dispatch loop, replacing 11 loose parameters.
struct RunContext<'a> {
    executor: &'a mut ParallelExecutor,
    task_index: &'a HashMap<String, HashMap<String, TaskDef>>,
    skip_enrichment: &'a HashMap<String, bool>,
    config: &'a RunConfig,
    tui: &'a TuiBridge,
    state: &'a mut RunState,
    agent_handle: &'a mut Option<AgentHandle>,
    agent_tx: &'a mpsc::Sender<AgentEvent>,
    gate_tx: &'a mpsc::Sender<GateCompletion>,
    paths: &'a PersistPaths,
}

// ─── Main Entry Point ───────────────────────────────────────────────────

/// Run all plans to completion (or cancellation).
pub async fn run(
    plans: Vec<Plan>,
    config: &RunConfig,
    state_hub: &StateHub,
    cancel: CancellationToken,
) -> Result<RunReport> {
    let exec_config = ExecutorConfig {
        max_concurrent_plans: 4,
        max_concurrent_tasks: 1,
        max_auto_fix_iterations: config.max_retries,
        task_timeout_secs: config.timeout_secs,
        ..Default::default()
    };

    let paths = PersistPaths::from_workdir(&config.workdir)?;
    persist::cleanup_orphaned_agents(&paths);

    // Build plan ID set for resume validation.
    let plan_ids: Vec<String> = plans.iter().map(|p| p.id.clone()).collect();

    // Only resume if snapshot exists AND its plans match the current run.
    let mut executor = try_resume(&paths, &exec_config, &plan_ids)
        .unwrap_or_else(|| ParallelExecutor::new(exec_config));

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
    let (gate_tx, mut gate_rx) = mpsc::channel::<GateCompletion>(16);

    // State and TUI bridge.
    let tui = TuiBridge::new(state_hub.sender());
    let mut state = RunState::new(total_tasks);
    let mut agent_handle: Option<AgentHandle> = None;

    let skip_enrichment: HashMap<String, bool> = plans
        .iter()
        .map(|p| (p.id.clone(), p.tasks.meta.skip_enrichment))
        .collect();

    let mut tick_interval = interval(Duration::from_millis(100));
    let mut flush_interval = interval(Duration::from_secs(2));

    info!(plan_count = plans.len(), total_tasks, "starting runner v2 event loop");

    loop {
        // Cancel-safety analysis:
        //   Branch 1 (agent_rx.recv): cancel-safe — mpsc::Receiver::recv drops no data.
        //   Branch 2 (gate_rx.recv):  cancel-safe — mpsc::Receiver::recv drops no data.
        //   Branch 3 (tick_interval): cancel-safe — Interval::tick is restartable.
        //   Branch 4 (flush_interval): cancel-safe — Interval::tick is restartable.
        //   Branch 5 (cancel.cancelled): cancel-safe — CancellationToken is idempotent.
        tokio::select! {
            // ─── Branch 1: Agent events ─────────────────────────────
            Some(event) = agent_rx.recv() => {
                let is_turn_done = matches!(&event, AgentEvent::TurnCompleted { .. });
                let is_exited = matches!(&event, AgentEvent::Exited { .. });

                handle_agent_event(&event, &mut state, &tui);

                // Per-turn budget check.
                if is_turn_done {
                    let max_turn = config.max_turn_usd;
                    if max_turn > 0.0 && state.cost_usd > max_turn {
                        warn!(
                            task = %state.current_task,
                            turn_cost = state.cost_usd,
                            limit = max_turn,
                            "single turn exceeded per-turn budget limit"
                        );
                    }
                }

                if is_turn_done || is_exited {
                    let plan_id = state.plan_id.clone();
                    if !plan_id.is_empty() {
                        match executor.apply_event(&plan_id, &ExecutorEvent::ImplementationDone) {
                            Ok(phase) => {
                                tui.phase_transition(&plan_id, "implementing", &format!("{phase:?}"));
                                info!(plan_id = %plan_id, phase = ?phase, "implementation done");
                            }
                            Err(e) => {
                                warn!(plan_id = %plan_id, err = %e, "transition error after implementation");
                            }
                        }
                    }

                    if is_exited {
                        if let Some(handle) = agent_handle.take() {
                            roko_agent::process::unregister_pid(handle.pid);
                        }
                    }

                    save_snapshot(&executor, &paths, &mut state);
                }
            }

            // ─── Branch 2: Verify completions ─────────────────────────
            Some(completion) = gate_rx.recv() => {
                let event = if completion.passed {
                    ExecutorEvent::GatePassed
                } else {
                    ExecutorEvent::GateFailed
                };

                state.gate_output = completion.output.clone();

                for v in &completion.verdicts {
                    tui.gate_result(
                        &completion.plan_id,
                        &completion.task_id,
                        &v.gate_name,
                        v.passed,
                    );
                }

                // Emit learning events for the completed agent+gate cycle.
                emit_episode(&completion, &state, &paths, &tui);
                emit_efficiency_event(&completion, &state, &config.model, &paths);

                match executor.apply_event(&completion.plan_id, &event) {
                    Ok(phase) => {
                        tui.phase_transition(&completion.plan_id, "gating", &format!("{phase:?}"));
                        info!(
                            plan_id = %completion.plan_id,
                            passed = completion.passed,
                            phase = ?phase,
                            "gate result applied"
                        );
                    }
                    Err(e) => {
                        warn!(plan_id = %completion.plan_id, err = %e, "transition error after gate");
                    }
                }

                save_snapshot(&executor, &paths, &mut state);
            }

            // ─── Branch 3: Executor tick ────────────────────────────
            _ = tick_interval.tick() => {
                let actions = executor.tick();
                for action in actions {
                    let mut ctx = RunContext {
                        executor: &mut executor,
                        task_index: &task_index,
                        skip_enrichment: &skip_enrichment,
                        config,
                        tui: &tui,
                        state: &mut state,
                        agent_handle: &mut agent_handle,
                        agent_tx: &agent_tx,
                        gate_tx: &gate_tx,
                        paths: &paths,
                    };
                    dispatch_action(&action, &mut ctx).await;
                }
            }

            // ─── Branch 4: Periodic flush ───────────────────────────
            _ = flush_interval.tick() => {
                save_snapshot(&executor, &paths, &mut state);
                if let Some(ref handle) = agent_handle {
                    let _ = persist::save_agent_pids(&paths, &[handle.pid]);
                }
            }

            // ─── Branch 5: Cancellation ─────────────────────────────
            _ = cancel.cancelled() => {
                warn!("cancellation requested — shutting down");
                if let Some(handle) = agent_handle.take() {
                    handle.kill(Duration::from_secs(3)).await;
                }
                save_snapshot(&executor, &paths, &mut state);
                break;
            }
        }

        if all_plans_terminal(&executor, &plans) {
            info!("all plans terminal — exiting event loop");
            break;
        }
    }

    Ok(build_report(&executor, &plans, &state))
}

// ─── Snapshot Helper ────────────────────────────────────────────────────

/// Save executor snapshot and track consecutive failures.
fn save_snapshot(
    executor: &ParallelExecutor,
    paths: &PersistPaths,
    state: &mut RunState,
) {
    let timestamp_ms = chrono::Utc::now().timestamp_millis() as u64;
    let snapshot = executor.snapshot(timestamp_ms);
    match persist::save_executor_snapshot(paths, &snapshot) {
        Ok(()) => state.snapshot_succeeded(),
        Err(e) => {
            error!(err = %e, "failed to save executor snapshot");
            state.snapshot_failed();
        }
    }
}

// ─── Resume ─────────────────────────────────────────────────────────────

/// Try to resume from an executor snapshot. Returns `None` if the snapshot
/// doesn't exist, is corrupt, or its plan set doesn't match `plan_ids`.
fn try_resume(
    paths: &PersistPaths,
    config: &ExecutorConfig,
    plan_ids: &[String],
) -> Option<ParallelExecutor> {
    if !paths.executor_json.exists() {
        return None;
    }

    let json = match std::fs::read_to_string(&paths.executor_json) {
        Ok(j) => j,
        Err(e) => {
            warn!(err = %e, "failed to read executor snapshot");
            return None;
        }
    };

    let snapshot: ExecutorSnapshot = match serde_json::from_str(&json) {
        Ok(s) => s,
        Err(e) => {
            warn!(err = %e, "corrupt executor snapshot — starting fresh");
            return None;
        }
    };

    // Validate: snapshot must contain at least one of the current plan IDs.
    let snap_plan_ids: Vec<&String> = snapshot.plan_states.keys().collect();
    let has_overlap = plan_ids.iter().any(|id| snapshot.plan_states.contains_key(id));

    if snap_plan_ids.is_empty() || !has_overlap {
        info!(
            snapshot_plans = ?snap_plan_ids,
            current_plans = ?plan_ids,
            "stale executor snapshot (no plan overlap) — starting fresh"
        );
        return None;
    }

    info!(
        path = %paths.executor_json.display(),
        plans = ?snap_plan_ids,
        "resuming from executor snapshot"
    );
    Some(ParallelExecutor::from_snapshot(config.clone(), snapshot))
}

// ─── Action Dispatcher ──────────────────────────────────────────────────

async fn dispatch_action(action: &ExecutorAction, ctx: &mut RunContext<'_>) {
    match action {
        ExecutorAction::DispatchPlan { plan_id } => {
            info!(plan_id = %plan_id, "dispatching plan");
            ctx.tui.plan_started(plan_id);

            if let Err(e) = ctx.executor.apply_event(plan_id, &ExecutorEvent::Start) {
                error!(plan_id = %plan_id, err = %e, "failed to start plan");
                return;
            }

            if ctx.skip_enrichment.get(plan_id.as_str()).copied().unwrap_or(false) {
                if let Err(e) = ctx.executor.apply_event(plan_id, &ExecutorEvent::EnrichmentDone) {
                    error!(plan_id = %plan_id, err = %e, "failed to skip enrichment");
                }
                ctx.tui.phase_transition(plan_id, "enriching", "implementing");
            }
        }

        ExecutorAction::SpawnAgent { plan_id, task, .. } => {
            info!(plan_id = %plan_id, task = %task, "spawning agent");

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
                ctx.tui.error(&format!("budget exceeded: ${plan_spent:.2} >= ${max_plan_usd:.2}"));
                let _ = ctx.executor.apply_event(
                    plan_id,
                    &ExecutorEvent::Fatal(format!("budget exceeded: ${plan_spent:.2} >= ${max_plan_usd:.2}")),
                );
                return;
            }

            let task_def = match ctx.task_index.get(plan_id.as_str()).and_then(|m| m.get(task.as_str())) {
                Some(t) => t,
                None => {
                    error!(plan_id = %plan_id, task = %task, "task not found in index");
                    let _ = ctx.executor.apply_event(
                        plan_id,
                        &ExecutorEvent::Fatal(format!("task {task} not found")),
                    );
                    return;
                }
            };

            ctx.state.reset_for_task(plan_id, task);
            ctx.state.total_agent_calls += 1;
            ctx.state.task_agent_calls += 1;

            let prompt = agent_stream::build_task_prompt(task_def, plan_id, &ctx.config.workdir);
            let system_prompt = agent_stream::build_minimal_system_prompt(task_def, plan_id);

            let model = task_def
                .model_hint
                .as_deref()
                .unwrap_or(&ctx.config.model)
                .to_string();

            let agent_id = format!("{plan_id}/{task}");

            // Prepend gate feedback if this is a retry.
            let final_prompt = if !ctx.state.gate_output.is_empty() {
                format!(
                    "## Previous Verify Failure\n\n{}\n\n---\n\n{prompt}",
                    ctx.state.gate_output
                )
            } else {
                prompt
            };

            let spawn_config = AgentSpawnConfig::from_run_config(
                ctx.config,
                final_prompt,
                system_prompt,
                model,
                agent_id.clone(),
            );

            // Kill any existing agent.
            if let Some(old) = ctx.agent_handle.take() {
                old.kill(Duration::from_secs(3)).await;
            }

            match agent_stream::spawn_agent(&spawn_config, ctx.agent_tx.clone()).await {
                Ok(handle) => {
                    ctx.state.agent_active = true;
                    ctx.state.agent_pid = Some(handle.pid);
                    ctx.tui.agent_spawned(&agent_id, task_def.role.as_deref().unwrap_or("implementer"));
                    ctx.tui.task_started(plan_id, task, &task_def.title, "implementing");
                    *ctx.agent_handle = Some(handle);
                }
                Err(e) => {
                    error!(err = %e, "failed to spawn agent");
                    ctx.tui.error(&format!("agent spawn failed: {e}"));
                    let _ = ctx.executor.apply_event(
                        plan_id,
                        &ExecutorEvent::Fatal(format!("spawn failed: {e}")),
                    );
                }
            }
        }

        ExecutorAction::RunGate { plan_id, rung } => {
            // Honor gates config: skip clippy rung (1) if disabled, skip test rung (2) if skip_tests.
            if *rung == 1 && !ctx.config.clippy_enabled {
                info!(plan_id = %plan_id, rung = rung, "skipping clippy gate (disabled in config)");
                let _ = ctx.executor.apply_event(plan_id, &ExecutorEvent::GatePassed);
                return;
            }
            if *rung == 2 && ctx.config.skip_tests {
                info!(plan_id = %plan_id, rung = rung, "skipping test gate (skip_tests in config)");
                let _ = ctx.executor.apply_event(plan_id, &ExecutorEvent::GatePassed);
                return;
            }

            info!(plan_id = %plan_id, rung = rung, "dispatching gate");
            gate_dispatch::spawn_gate(
                plan_id.clone(),
                ctx.state.current_task.clone(),
                *rung,
                ctx.config.workdir.clone(),
                ctx.gate_tx.clone(),
            );
        }

        ExecutorAction::RunVerify { plan_id } => {
            // STUB: auto-pass verification. Wire real verify phase in Phase 5.
            info!(plan_id = %plan_id, "auto-passing verification (stub)");
            let _ = ctx.executor.apply_event(plan_id, &ExecutorEvent::VerifyPassed);
        }

        ExecutorAction::CompletePlan { plan_id } => {
            info!(plan_id = %plan_id, "plan completed");
            ctx.state.task_completed();
            ctx.tui.task_completed(&ctx.state.plan_id, &ctx.state.current_task, "succeeded");
            ctx.tui.plan_completed(plan_id, true);
            save_snapshot(ctx.executor, ctx.paths, ctx.state);
        }

        ExecutorAction::FailPlan { plan_id, reason } => {
            warn!(plan_id = %plan_id, reason = %reason, "plan failed");
            ctx.state.task_failed();
            ctx.tui.task_completed(&ctx.state.plan_id, &ctx.state.current_task, "failed");
            ctx.tui.plan_completed(plan_id, false);
        }

        ExecutorAction::MergeBranch { plan_id } => {
            info!(plan_id = %plan_id, "auto-advancing merge");
            let _ = ctx.executor.apply_event(plan_id, &ExecutorEvent::MergeSucceeded);
        }

        _ => {
            info!(action = ?action, "auto-advancing action");
        }
    }
}

// ─── Learning Emission ──────────────────────────────────────────────────

/// Emit an episode after a gate cycle completes (pass or fail).
fn emit_episode(
    completion: &GateCompletion,
    state: &RunState,
    paths: &PersistPaths,
    tui: &TuiBridge,
) {
    let agent_id = format!("{}/{}", completion.plan_id, completion.task_id);
    let gate_verdicts: Vec<GateVerdict> = completion
        .verdicts
        .iter()
        .map(|v| {
            let mut gv = GateVerdict::new(&v.gate_name, v.passed);
            if !v.summary.is_empty() {
                gv = gv.with_signature(&v.summary);
            }
            gv
        })
        .collect();

    let task_wall_ms = state.task_elapsed_ms();
    let mut ep = Episode::new(&agent_id, &completion.task_id);
    ep.model = state.agent_model.clone();
    ep.backend = "claude".to_string();
    ep.usage = Usage {
        input_tokens: state.tokens_in,
        output_tokens: state.tokens_out,
        cache_read_tokens: state.cache_read_tokens,
        cache_write_tokens: state.cache_write_tokens,
        cost_usd: state.cost_usd,
        wall_ms: task_wall_ms,
        ..Usage::default()
    };
    ep.gate_verdicts = gate_verdicts;
    ep.turns = u64::from(state.task_agent_calls);
    ep.tokens_used = state.tokens_in + state.tokens_out;
    ep.extra.insert(
        "plan_id".to_string(),
        serde_json::Value::String(completion.plan_id.clone()),
    );

    if completion.passed {
        ep = ep.succeeded();
    } else {
        ep = ep.failed(&completion.output);
    }

    ep.attach_all_fingerprints();

    if let Err(e) = persist::append_jsonl(&paths.episodes_jsonl, &ep) {
        error!(err = %e, "failed to append episode");
    } else {
        tui.efficiency_event(
            &completion.plan_id,
            &completion.task_id,
            "episode_logged",
            1.0,
        );
    }
}

/// Emit an efficiency event after a gate cycle completes.
fn emit_efficiency_event(
    completion: &GateCompletion,
    state: &RunState,
    default_model: &str,
    paths: &PersistPaths,
) {
    let model = if state.agent_model.is_empty() {
        default_model.to_string()
    } else {
        state.agent_model.clone()
    };
    let task_wall_ms = state.task_elapsed_ms();

    let gate_errors: Vec<String> = completion
        .verdicts
        .iter()
        .filter(|v| !v.passed)
        .map(|v| format!("{}: {}", v.gate_name, v.summary))
        .collect();

    let event = AgentEfficiencyEvent {
        agent_id: format!("{}/{}", completion.plan_id, completion.task_id),
        role: "implementer".to_string(),
        backend: "claude".to_string(),
        model: model.clone(),
        plan_id: completion.plan_id.clone(),
        task_id: completion.task_id.clone(),
        input_tokens: state.tokens_in,
        output_tokens: state.tokens_out,
        cache_read_tokens: state.cache_read_tokens,
        cache_write_tokens: state.cache_write_tokens,
        cost_usd: state.cost_usd,
        wall_time_ms: task_wall_ms,
        duration_ms: task_wall_ms,
        iteration: state.iteration,
        gate_passed: completion.passed,
        outcome: if completion.passed { "success" } else { "failure" }.to_string(),
        gate_errors,
        model_used: model,
        timestamp: chrono::Utc::now().to_rfc3339(),
        ..AgentEfficiencyEvent::default()
    };

    if let Err(e) = persist::append_jsonl(&paths.efficiency_jsonl, &event) {
        error!(err = %e, "failed to append efficiency event");
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────

fn all_plans_terminal(executor: &ParallelExecutor, plans: &[Plan]) -> bool {
    plans.iter().all(|p| {
        executor
            .plan_state(&p.id)
            .map_or(true, |s| s.is_terminal())
    })
}

fn build_report(
    executor: &ParallelExecutor,
    plans: &[Plan],
    state: &RunState,
) -> RunReport {
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
    }
}
