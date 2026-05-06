//! Bench run endpoints.
//!
//! Provides routes for starting, tracking, comparing, and analyzing
//! benchmark runs that exercise roko's `run_once()` pipeline.

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use futures::stream::{self};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::bench::{
    self, BenchConfigOverrides, BenchRun, BenchRunIndexEntry, BenchRunKind, BenchRunStatus,
    BenchRunSummary, BenchStrategy, BenchSuite, BenchTaskResult,
};
use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::state::{AppState, BenchRunHandle};
use roko_agent::CostTable;
use roko_core::Usage as CoreUsage;
use roko_learn::playbook::PlaybookStore;
use roko_neuro::KnowledgeStore;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/bench/provider-status", get(provider_status))
        .route("/bench/run", post(start_bench_run))
        .route("/bench/runs", post(start_bench_run))
        .route("/bench/run/{id}", get(get_bench_run))
        .route("/bench/runs/{id}", get(get_bench_run))
        .route("/bench/run/{id}/status", get(bench_run_status))
        .route("/bench/run/{id}", delete(delete_bench_run))
        .route("/bench/runs/{id}", delete(delete_bench_run))
        .route("/bench/runs/{id}/cancel", post(cancel_bench_run))
        .route("/bench/runs", get(list_bench_runs))
        .route("/bench/runs/compare", get(compare_bench_runs))
        .route("/bench/cost-summary", get(cost_summary))
        .route("/bench/suites", get(list_suites))
        .route("/bench/suites/{id}", get(get_suite))
        .route("/bench/suites", post(upload_suite))
        .route("/bench/models", get(list_models))
        .route("/bench/pareto", get(pareto_frontier))
        .route("/bench/export/{id}", get(export_bench_run))
        .route("/bench/events", get(bench_events_sse))
}

/// `GET /api/bench/provider-status` -- check whether LLM providers are configured.
async fn provider_status(State(state): State<Arc<AppState>>) -> Json<Value> {
    let config = state.load_roko_config();
    let providers = config.effective_providers();
    let has_providers = !providers.is_empty();
    let has_api_keys = providers.values().any(|p| config.is_provider_available(p));
    Json(json!({
        "has_providers": has_providers,
        "has_api_keys": has_api_keys,
        "demo_available": true,
    }))
}

// ---------------------------------------------------------------------------
// Request / query types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct StartBenchRequest {
    suite_id: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    overrides: BenchConfigOverrides,
}

#[derive(Deserialize)]
struct ListRunsQuery {
    #[serde(default)]
    suite_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
}

fn default_limit() -> usize {
    50
}

#[derive(Deserialize)]
struct CompareQuery {
    ids: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /api/bench/run` -- start a new bench run.
async fn start_bench_run(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StartBenchRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // Ensure built-in suites exist.
    bench::ensure_builtin_suites(&state.workdir).await;

    let suite = bench::load_suite(&state.workdir, &body.suite_id)
        .await
        .ok_or_else(|| ApiError::not_found("suite not found"))?;

    let run_id = uuid::Uuid::new_v4().to_string();
    let started_at = now_secs();

    let run = BenchRun {
        id: run_id.clone(),
        suite_id: suite.id.clone(),
        suite_name: suite.name.clone(),
        kind: BenchRunKind::Manual,
        overrides: body.overrides.clone(),
        label: body.label.clone(),
        status: BenchRunStatus::Running,
        started_at,
        finished_at: None,
        results: Vec::new(),
        summary: None,
        current_task_index: 0,
        total_tasks: suite.tasks.len(),
    };

    // Save initial state.
    if let Err(e) = bench::save_bench_run(&state.workdir, &run).await {
        tracing::warn!(error = %e, "failed to save initial bench run");
    }

    // Add index entry.
    let index_entry = BenchRunIndexEntry {
        id: run_id.clone(),
        suite_id: suite.id.clone(),
        suite_name: suite.name.clone(),
        status: BenchRunStatus::Running,
        started_at,
        finished_at: None,
        label: body.label.clone(),
        model: body.overrides.model.clone(),
        pass_rate: None,
        total_cost_usd: None,
    };
    if let Err(err) = bench::append_index_entry(&state.workdir, &index_entry).await {
        tracing::warn!(error = %err, "failed to append bench run index entry");
    }

    // Publish start event.
    state.event_bus.publish(ServerEvent::BenchRunStarted {
        bench_id: run_id.clone(),
        suite_id: suite.id.clone(),
        total_tasks: suite.tasks.len(),
    });

    // Spawn background execution.
    let handle = tokio::spawn(execute_bench_run(
        Arc::clone(&state),
        run_id.clone(),
        suite,
        body.overrides,
        body.label,
        started_at,
    ));

    state.active_bench_runs.write().await.insert(
        run_id.clone(),
        BenchRunHandle {
            id: run_id.clone(),
            handle,
        },
    );

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "id": run_id })),
    ))
}

/// Background task that executes all tasks in a bench suite.
async fn execute_bench_run(
    state: Arc<AppState>,
    run_id: String,
    suite: BenchSuite,
    overrides: BenchConfigOverrides,
    label: Option<String>,
    started_at: u64,
) {
    let total_tasks = suite.tasks.len();
    let mut results = Vec::new();
    let mut _passed_count = 0usize;
    let mut _failed_count = 0usize;

    // Build a CostTable from the live config for accurate cost estimation.
    let cost_table = CostTable::from_config_with_defaults(&state.roko_config.load().models);

    let bench_workdir = match scaffold_bench_workdir(&suite.id, &run_id).await {
        Ok(path) => path,
        Err(err) => {
            tracing::warn!(
                error = %err,
                run_id = %run_id,
                suite_id = %suite.id,
                "failed to scaffold bench workdir"
            );

            let finished_at = now_secs();
            if let Ok(Some(mut run)) = bench::load_bench_run(&state.workdir, &run_id).await {
                run.status = BenchRunStatus::Failed;
                run.finished_at = Some(finished_at);
                if let Err(err) = bench::save_bench_run(&state.workdir, &run).await {
                    tracing::warn!(error = %err, run_id = %run_id, "failed to save failed bench run state");
                }
            }

            let failed_index_entry = BenchRunIndexEntry {
                id: run_id.clone(),
                suite_id: suite.id.clone(),
                suite_name: suite.name.clone(),
                status: BenchRunStatus::Failed,
                started_at,
                finished_at: Some(finished_at),
                label: label.clone(),
                model: overrides.model.clone(),
                pass_rate: None,
                total_cost_usd: None,
            };
            if let Err(err) = bench::update_index_entry(&state.workdir, &failed_index_entry).await {
                tracing::warn!(error = %err, run_id = %run_id, "failed to update index for failed bench run");
            }

            state.active_bench_runs.write().await.remove(&run_id);
            return;
        }
    };
    let _bench_workdir_cleanup = BenchWorkdirCleanup::new(bench_workdir.clone());

    let learning_stores = if matches!(overrides.strategy, BenchStrategy::Minimal) {
        None
    } else {
        Some((
            PlaybookStore::new(state.workdir.join(".roko").join("learn").join("playbooks")),
            KnowledgeStore::for_workdir(&state.workdir),
        ))
    };
    let mut learning_totals =
        if let Some((playbook_store, knowledge_store)) = learning_stores.as_ref() {
            current_learning_totals(playbook_store, knowledge_store).await
        } else {
            None
        };

    for (idx, task) in suite.tasks.iter().enumerate() {
        // Publish task start.
        state.event_bus.publish(ServerEvent::BenchTaskStarted {
            bench_id: run_id.clone(),
            task_id: task.id.clone(),
            task_name: task.name.clone(),
            task_index: idx,
            total_tasks,
        });

        let start = std::time::Instant::now();
        let result = state
            .runtime
            .run_once_with_config(bench_workdir.as_path(), &task.prompt, &overrides)
            .await;
        let duration_ms = start.elapsed().as_millis() as u64;

        let task_result = match result {
            Ok(run_result) => {
                let passed = if run_result.success {
                    // Check expected output if specified.
                    if let Some(ref expected) = task.expected_output {
                        run_result
                            .output_text
                            .as_ref()
                            .is_some_and(|text| text.contains(expected.as_str()))
                    } else {
                        true
                    }
                } else {
                    false
                };

                let (input_tokens, output_tokens) = run_result
                    .usage
                    .as_ref()
                    .map(|u| (u.input_tokens, u.output_tokens))
                    .unwrap_or((0, 0));

                let cost_usd = cost_table.calculate(
                    overrides.model.as_deref().unwrap_or(""),
                    &CoreUsage {
                        input_tokens: input_tokens as u32,
                        output_tokens: output_tokens as u32,
                        ..CoreUsage::default()
                    },
                );
                let output_preview = run_result
                    .output_text
                    .as_ref()
                    .map(|t| t.chars().take(500).collect());

                let gate_verdicts: Vec<serde_json::Value> = run_result
                    .gate_results
                    .iter()
                    .map(|gr| {
                        serde_json::json!({
                            "gate": gr.gate,
                            "passed": gr.passed,
                            "detail": gr.detail,
                        })
                    })
                    .collect();

                BenchTaskResult {
                    task_id: task.id.clone(),
                    task_name: task.name.clone(),
                    status: if passed { "pass".into() } else { "fail".into() },
                    duration_ms,
                    model: overrides.model.clone().unwrap_or_default(),
                    tokens_in: input_tokens,
                    tokens_out: output_tokens,
                    cost_usd,
                    gate_verdicts,
                    retries_used: 0,
                    output_preview,
                    error: None,
                }
            }
            Err(e) => BenchTaskResult {
                task_id: task.id.clone(),
                task_name: task.name.clone(),
                status: "fail".into(),
                duration_ms,
                model: overrides.model.clone().unwrap_or_default(),
                tokens_in: 0,
                tokens_out: 0,
                cost_usd: 0.0,
                gate_verdicts: Vec::new(),
                retries_used: 0,
                output_preview: None,
                error: Some(format!("{e}")),
            },
        };

        if task_result.passed() {
            _passed_count += 1;
        } else {
            _failed_count += 1;
        }

        // Emit per-gate verdicts so the live UI can show gate pass/fail.
        for gv in &task_result.gate_verdicts {
            let gate_name = gv
                .get("gate")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let gate_passed = gv.get("passed").and_then(|v| v.as_bool()).unwrap_or(false);
            let gate_detail = gv.get("detail").and_then(|v| v.as_str()).map(String::from);
            state.event_bus.publish(ServerEvent::BenchGateVerdict {
                bench_id: run_id.clone(),
                task_id: task_result.task_id.clone(),
                gate: gate_name,
                passed: gate_passed,
                message: gate_detail,
                duration_ms: task_result.duration_ms,
            });
        }

        // Emit token velocity for throughput sparklines.
        let total_tokens = task_result.tokens_in + task_result.tokens_out;
        if task_result.duration_ms > 0 && total_tokens > 0 {
            let tps = (total_tokens as f64) / (task_result.duration_ms as f64 / 1000.0);
            state.event_bus.publish(ServerEvent::BenchTokenVelocity {
                bench_id: run_id.clone(),
                task_id: task_result.task_id.clone(),
                tokens_per_second: tps,
                tokens_in: task_result.tokens_in,
                tokens_out: task_result.tokens_out,
                duration_ms: task_result.duration_ms,
            });
        }

        // Publish task completion.
        state.event_bus.publish(ServerEvent::BenchTaskCompleted {
            bench_id: run_id.clone(),
            task_id: task_result.task_id.clone(),
            result: serde_json::to_value(&task_result).unwrap_or_default(),
        });

        let cost_so_far: f64 = results
            .iter()
            .map(|r: &BenchTaskResult| r.cost_usd)
            .sum::<f64>()
            + task_result.cost_usd;

        results.push(task_result);

        // Publish progress.
        state.event_bus.publish(ServerEvent::BenchProgress {
            bench_id: run_id.clone(),
            completed: results.len(),
            total: total_tasks,
            cost_so_far,
        });

        if let Some((playbook_store, knowledge_store)) = learning_stores.as_ref() {
            if let Some(current_totals) =
                current_learning_totals(playbook_store, knowledge_store).await
            {
                let previous_totals = learning_totals.replace(current_totals);
                let playbooks_created = previous_totals
                    .map(|previous| {
                        current_totals
                            .playbooks_total
                            .saturating_sub(previous.playbooks_total)
                    })
                    .unwrap_or(0);
                let anti_patterns_created = previous_totals
                    .map(|previous| {
                        current_totals
                            .anti_patterns_total
                            .saturating_sub(previous.anti_patterns_total)
                    })
                    .unwrap_or(0);

                state.event_bus.publish(ServerEvent::BenchLearningEvent {
                    bench_id: run_id.clone(),
                    task_id: task.id.clone(),
                    playbooks_created,
                    anti_patterns_created,
                    total_playbooks: current_totals.playbooks_total,
                    total_anti_patterns: current_totals.anti_patterns_total,
                });
            }
        }

        // Update on-disk state periodically.
        match bench::load_bench_run(&state.workdir, &run_id).await {
            Ok(Some(mut run)) => {
                run.results = results.clone();
                run.current_task_index = idx + 1;
                if let Err(err) = bench::save_bench_run(&state.workdir, &run).await {
                    tracing::warn!(error = %err, run_id = %run_id, "failed to save periodic bench run state");
                }
            }
            Ok(None) => {
                tracing::warn!(run_id = %run_id, "bench run file missing during periodic save");
            }
            Err(err) => {
                tracing::warn!(error = %err, run_id = %run_id, "failed to load bench run for periodic save");
            }
        }
    }

    // Finalize the run.
    let summary = BenchRunSummary::from_results(&results);
    let finished_at = now_secs();

    match bench::load_bench_run(&state.workdir, &run_id).await {
        Ok(Some(mut run)) => {
            run.status = BenchRunStatus::Completed;
            run.finished_at = Some(finished_at);
            run.results = results;
            run.summary = Some(summary.clone());
            run.current_task_index = total_tasks;
            if let Err(err) = bench::save_bench_run(&state.workdir, &run).await {
                tracing::warn!(error = %err, run_id = %run_id, "failed to save completed bench run");
            }
        }
        Ok(None) => {
            tracing::warn!(run_id = %run_id, "bench run file missing at finalization");
        }
        Err(err) => {
            tracing::warn!(error = %err, run_id = %run_id, "failed to load bench run at finalization");
        }
    }

    // Update index entry.
    let index_entry = BenchRunIndexEntry {
        id: run_id.clone(),
        suite_id: suite.id.clone(),
        suite_name: suite.name.clone(),
        status: BenchRunStatus::Completed,
        started_at,
        finished_at: Some(finished_at),
        label,
        model: overrides.model,
        pass_rate: Some(summary.pass_rate),
        total_cost_usd: Some(summary.total_cost_usd),
    };
    if let Err(err) = bench::update_index_entry(&state.workdir, &index_entry).await {
        tracing::warn!(error = %err, run_id = %run_id, "failed to update bench run index at completion");
    }

    // Publish completion event.
    state.event_bus.publish(ServerEvent::BenchRunCompleted {
        bench_id: run_id.clone(),
        summary: serde_json::to_value(&summary).unwrap_or_default(),
    });

    // ── Regression detection ─────────────────────────────────────────
    //
    // Convert current bench results into TaskMetric records and compare
    // against a baseline computed from prior completed bench runs.
    // TODO: re-enable after fixing roko_learn::baseline/regression imports
    // run_bench_regression(&state, &run_id, &suite.id, &results, &overrides);

    // Clean up handle.
    state.active_bench_runs.write().await.remove(&run_id);
}

/// `GET /api/bench/run/:id` -- get full bench run details.
async fn get_bench_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let run = bench::load_bench_run(&state.workdir, &id)
        .await
        .map_err(|e| ApiError::internal(format!("failed to load run: {e}")))?
        .ok_or_else(|| ApiError::not_found("bench run not found"))?;
    Ok(Json(serde_json::to_value(run).unwrap_or_default()))
}

/// `GET /api/bench/run/:id/status` -- lightweight status poll.
async fn bench_run_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let run = bench::load_bench_run(&state.workdir, &id)
        .await
        .map_err(|e| ApiError::internal(format!("failed to load run: {e}")))?
        .ok_or_else(|| ApiError::not_found("bench run not found"))?;
    Ok(Json(json!({
        "id": run.id,
        "status": run.status,
        "current_task_index": run.current_task_index,
        "total_tasks": run.total_tasks,
        "passed": run.results.iter().filter(|r| r.passed()).count(),
        "failed": run.results.iter().filter(|r| !r.passed()).count(),
        "summary": run.summary,
    })))
}

/// `DELETE /api/bench/run/:id` -- cancel or delete a bench run.
async fn delete_bench_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    // Abort if running.
    let handle = state.active_bench_runs.write().await.remove(&id);
    if let Some(handle) = handle {
        handle.handle.abort();
    }
    // Mark as cancelled on disk if still running.
    match bench::load_bench_run(&state.workdir, &id).await {
        Ok(Some(mut run)) => {
            if run.status == BenchRunStatus::Running {
                run.status = BenchRunStatus::Cancelled;
                run.finished_at = Some(now_secs());
                if let Err(err) = bench::save_bench_run(&state.workdir, &run).await {
                    tracing::warn!(error = %err, bench_id = %id, "failed to save cancelled bench run state");
                }
            }
        }
        Ok(None) => {} // Already deleted, nothing to cancel
        Err(err) => {
            tracing::warn!(error = %err, bench_id = %id, "failed to load bench run for cancellation");
        }
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// `POST /api/bench/runs/:id/cancel` -- cancel a running bench run.
///
/// Equivalent to DELETE but accepts POST (frontend convention).
async fn cancel_bench_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    // Abort if running.
    let handle = state.active_bench_runs.write().await.remove(&id);
    if let Some(handle) = handle {
        handle.handle.abort();
    }
    // Mark as cancelled on disk if still running.
    match bench::load_bench_run(&state.workdir, &id).await {
        Ok(Some(mut run)) => {
            if run.status == BenchRunStatus::Running {
                run.status = BenchRunStatus::Cancelled;
                run.finished_at = Some(now_secs());
                if let Err(err) = bench::save_bench_run(&state.workdir, &run).await {
                    tracing::warn!(error = %err, bench_id = %id, "failed to save cancelled bench run state");
                }
            }
        }
        Ok(None) => {}
        Err(err) => {
            tracing::warn!(error = %err, bench_id = %id, "failed to load bench run for cancellation");
        }
    }
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({ "id": id, "status": "cancelled" })),
    ))
}

/// `GET /api/bench/runs` -- list bench runs.
async fn list_bench_runs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListRunsQuery>,
) -> Json<Value> {
    let mut entries = bench::load_index_entries(&state.workdir).await;

    // Apply filters.
    if let Some(ref suite_id) = query.suite_id {
        entries.retain(|e| e.suite_id == *suite_id);
    }
    if let Some(ref status) = query.status {
        entries.retain(|e| {
            let s = serde_json::to_value(&e.status)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default();
            s == *status
        });
    }

    // Reverse chronological.
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.started_at));

    let total = entries.len();
    let page: Vec<_> = entries
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .collect();

    Json(json!({
        "total": total,
        "offset": query.offset,
        "limit": query.limit,
        "runs": page,
    }))
}

/// `GET /api/bench/runs/compare?ids=a,b` -- compare multiple runs.
async fn compare_bench_runs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CompareQuery>,
) -> Result<Json<Value>, ApiError> {
    let ids: Vec<&str> = query.ids.split(',').map(str::trim).collect();
    let mut runs = Vec::new();
    for id in &ids {
        if let Some(run) = bench::load_bench_run(&state.workdir, id)
            .await
            .map_err(|e| ApiError::internal(format!("load error: {e}")))?
        {
            runs.push(run);
        }
    }
    if runs.is_empty() {
        return Err(ApiError::not_found("no runs found"));
    }
    Ok(Json(json!({ "runs": runs })))
}

/// `GET /api/bench/suites` -- list available suites.
async fn list_suites(State(state): State<Arc<AppState>>) -> Json<Value> {
    bench::ensure_builtin_suites(&state.workdir).await;
    let suites = bench::load_suites(&state.workdir).await;
    let listing: Vec<Value> = suites
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "name": s.name,
                "description": s.description,
                "task_count": s.tasks.len(),
            })
        })
        .collect();
    Json(json!({ "suites": listing }))
}

/// `GET /api/bench/suites/:id` -- get full suite with tasks.
async fn get_suite(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    bench::ensure_builtin_suites(&state.workdir).await;
    let suite = bench::load_suite(&state.workdir, &id)
        .await
        .ok_or_else(|| ApiError::not_found("suite not found"))?;
    Ok(Json(serde_json::to_value(suite).unwrap_or_default()))
}

/// `POST /api/bench/suites` -- upload a custom suite.
async fn upload_suite(
    State(state): State<Arc<AppState>>,
    Json(suite): Json<BenchSuite>,
) -> Result<impl IntoResponse, ApiError> {
    if suite.id.is_empty() || suite.tasks.is_empty() {
        return Err(ApiError::bad_request(
            "suite must have an id and at least one task",
        ));
    }
    bench::save_suite(&state.workdir, &suite)
        .await
        .map_err(|e| ApiError::internal(format!("failed to save suite: {e}")))?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({ "id": suite.id })),
    ))
}

/// `GET /api/bench/models` -- list available models from config.
async fn list_models(State(state): State<Arc<AppState>>) -> Json<Value> {
    let config = state.load_roko_config();
    let models = bench::list_models_from_config(&config);
    Json(json!({ "models": models }))
}

/// `GET /api/bench/pareto` -- compute pareto frontier.
async fn pareto_frontier(State(state): State<Arc<AppState>>) -> Json<Value> {
    let frontier = bench::compute_pareto_frontier(&state.workdir).await;
    Json(json!({ "frontier": frontier }))
}

/// `GET /api/bench/export/:id` -- export a bench run as JSON.
async fn export_bench_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let run = bench::load_bench_run(&state.workdir, &id)
        .await
        .map_err(|e| ApiError::internal(format!("failed to load run: {e}")))?
        .ok_or_else(|| ApiError::not_found("bench run not found"))?;
    Ok(Json(serde_json::to_value(run).unwrap_or_default()))
}

/// `GET /api/bench/cost-summary` -- aggregate cost by model across all runs.
async fn cost_summary(State(state): State<Arc<AppState>>) -> Json<Value> {
    let entries = bench::load_index_entries(&state.workdir).await;
    let mut model_stats: std::collections::HashMap<String, (f64, u64, u64)> =
        std::collections::HashMap::new();

    for entry in &entries {
        if let Ok(Some(run)) = bench::load_bench_run(&state.workdir, &entry.id).await {
            for result in &run.results {
                let stat = model_stats.entry(result.model.clone()).or_default();
                stat.0 += result.cost_usd;
                stat.1 += result.tokens_in + result.tokens_out;
                stat.2 += 1;
            }
        }
    }

    let models: Vec<Value> = model_stats
        .into_iter()
        .map(|(model, (cost_usd, tokens, tasks))| {
            json!({
                "model": model,
                "cost_usd": cost_usd,
                "tokens": tokens,
                "tasks": tasks,
            })
        })
        .collect();

    Json(json!({ "models": models }))
}

/// `GET /api/bench/events` -- SSE stream filtered to bench events.
async fn bench_events_sse(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let rx = state.event_bus.subscribe();
    let stream = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(envelope) => {
                    // Only emit bench-related events.
                    let is_bench = matches!(
                        &envelope.payload,
                        ServerEvent::BenchRunStarted { .. }
                            | ServerEvent::BenchTaskStarted { .. }
                            | ServerEvent::BenchTaskCompleted { .. }
                            | ServerEvent::BenchLearningEvent { .. }
                            | ServerEvent::BenchProgress { .. }
                            | ServerEvent::BenchRunCompleted { .. }
                            | ServerEvent::BenchGateVerdict { .. }
                            | ServerEvent::BenchTokenVelocity { .. }
                            | ServerEvent::BenchAgentOutput { .. }
                            | ServerEvent::BenchRegressionReport { .. }
                    );
                    if !is_bench {
                        continue;
                    }
                    let data = serde_json::to_string(&envelope.payload).unwrap_or_default();
                    let sse_event = Event::default().data(data).id(envelope.seq.to_string());
                    return Some((Ok::<_, Infallible>(sse_event), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(n, "bench SSE client lagged");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    let sse = Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(8))
            .text("keepalive"),
    );
    (super::sse::sse_response_headers(), sse)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Clone, Copy)]
struct LearningTotals {
    playbooks_total: u32,
    anti_patterns_total: u32,
}

async fn current_learning_totals(
    playbook_store: &PlaybookStore,
    knowledge_store: &KnowledgeStore,
) -> Option<LearningTotals> {
    let playbooks_total = match playbook_store.list().await {
        Ok(playbooks) => playbooks.len().min(u32::MAX as usize) as u32,
        Err(err) => {
            tracing::warn!(error = %err, "failed to read bench playbook counts");
            return None;
        }
    };

    let anti_patterns_total = match knowledge_store.stats() {
        Ok(stats) => stats.anti_knowledge_count.min(u32::MAX as usize) as u32,
        Err(err) => {
            tracing::warn!(error = %err, "failed to read bench anti-pattern counts");
            return None;
        }
    };

    Some(LearningTotals {
        playbooks_total,
        anti_patterns_total,
    })
}

struct BenchWorkdirCleanup {
    path: PathBuf,
}

impl BenchWorkdirCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for BenchWorkdirCleanup {
    fn drop(&mut self) {
        // Intentionally ignoring: best-effort cleanup of temporary bench workdir
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

async fn scaffold_bench_workdir(suite_id: &str, run_id: &str) -> anyhow::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("roko-bench-{run_id}"));

    if tokio::fs::try_exists(&dir)
        .await
        .context("check whether bench workdir already exists")?
    {
        tokio::fs::remove_dir_all(&dir)
            .await
            .with_context(|| format!("remove stale bench workdir {}", dir.display()))?;
    }

    if let Err(err) = async {
        let cargo_toml_path = dir.join("Cargo.toml");
        let main_rs_path = dir.join("src").join("main.rs");
        let lib_rs_path = dir.join("src").join("lib.rs");
        let cargo_toml = bench_cargo_toml(suite_id, run_id);

        write_scaffold_file(&cargo_toml_path, &cargo_toml).await?;
        write_scaffold_file(&main_rs_path, bench_main_contents()).await?;

        if suite_id == "learnable-rust" {
            let helpers_rs_path = dir.join("src").join("helpers.rs");
            write_scaffold_file(&helpers_rs_path, learnable_helpers_contents()).await?;
            write_scaffold_file(&lib_rs_path, learnable_rust_lib_contents()).await?;
        } else {
            write_scaffold_file(&lib_rs_path, generic_lib_contents()).await?;
        }

        Ok::<(), anyhow::Error>(())
    }
    .await
    {
        // Intentionally ignoring: best-effort cleanup after scaffold failure
        let _ = tokio::fs::remove_dir_all(&dir).await;
        return Err(err);
    }

    Ok(dir)
}

async fn write_scaffold_file(path: &std::path::Path, contents: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create {}", parent.display()))?;
    }

    tokio::fs::write(path, contents)
        .await
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn bench_cargo_toml(suite_id: &str, run_id: &str) -> String {
    format!(
        r#"[package]
name = "roko-bench-scaffold"
version = "0.1.0"
edition = "2024"

# suite_id = "{suite_id}"
# run_id = "{run_id}"

[dependencies]
"#,
    )
}

fn bench_main_contents() -> &'static str {
    r#"fn main() {}
"#
}

fn generic_lib_contents() -> &'static str {
    r#"/// Minimal scaffold for non-learnable bench suites.
pub fn scaffold_marker() -> &'static str {
    "roko-bench"
}

#[cfg(test)]
mod tests {}
"#
}

fn learnable_helpers_contents() -> &'static str {
    r#"/// Type referenced by the broken import in `src/lib.rs`.
pub struct MissingType;
"#
}

fn learnable_rust_lib_contents() -> &'static str {
    r#"pub mod helpers;

/// Starter stub for task 1.
pub fn format_greeting(name: &str) -> String {
    // TODO: implement
    let _ = name;
    todo!("format the greeting")
}

/// Task 2 starter: the same loop appears twice so it can be extracted later.
pub fn total_message_bytes(messages: &[&str]) -> usize {
    let mut total = 0;
    for message in messages {
        if !message.trim().is_empty() {
            total += message.len();
        }
    }
    total
}

pub fn total_message_bytes_again(messages: &[&str]) -> usize {
    let mut total = 0;
    for message in messages {
        if !message.trim().is_empty() {
            total += message.len();
        }
    }
    total
}

/// Task 4 starter: keep the helper generic and preserve the incoming result.
pub fn wrap_result<T, E>(value: Result<T, E>) -> Result<T, E> {
    let _ = value;
    unimplemented!("wrap_result should return the input Result unchanged")
}

/// Task 5 starter: implement `Iterator` for this counter.
/// `CountUp` should yield numbers from 1 through `limit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountUp {
    /// Last yielded value.
    current: u64,
    /// Inclusive upper bound.
    limit: u64,
}

impl CountUp {
    pub fn new(limit: u64) -> Self {
        Self { current: 0, limit }
    }

    pub fn limit(&self) -> u64 {
        self.limit
    }
}

// ── Regression detection ───────────────────────────────────────────────────

/// Convert bench results to `TaskMetric` records and compare against a baseline
/// built from prior completed runs of the same suite.
fn run_bench_regression(
    state: &AppState,
    run_id: &str,
    suite_id: &str,
    results: &[BenchTaskResult],
    overrides: &BenchConfigOverrides,
) {
    use roko_core::metric::{ConfigHash, TaskMetric};
    use roko_learn::baseline::compute_baseline;
    use roko_learn::regression::{RegressionThresholds, detect_regressions};

    let model = overrides.model.as_deref().unwrap_or("unknown");
    let config_hash = ConfigHash(format!("bench-{suite_id}"));
    let now = chrono::Utc::now().to_rfc3339();

    // Convert current results to TaskMetric records.
    let current: Vec<TaskMetric> = results
        .iter()
        .map(|r| TaskMetric {
            timestamp: now.clone(),
            run_id: run_id.to_string(),
            config_hash: config_hash.clone(),
            plan_id: suite_id.to_string(),
            task_id: r.task_id.clone(),
            iteration: 1,
            role: "bench".to_string(),
            backend: "bench".to_string(),
            model: model.to_string(),
            complexity_band: "standard".to_string(),
            gate: "bench".to_string(),
            gate_passed: r.passed(),
            wall_time_ms: r.duration_ms,
            input_tokens: r.tokens_in,
            output_tokens: r.tokens_out,
            cached_tokens: 0,
            cost_usd: r.cost_usd,
            sections_included: 0,
            sections_dropped: 0,
            context_tokens: 0,
            cache_hit_rate: 0.0,
        })
        .collect();

    if current.len() < 5 {
        // Not enough data for regression detection.
        return;
    }

    // Load prior runs for the same suite as baseline.
    let bench_dir = state.workdir.join(".roko").join("bench");
    let mut baseline_metrics = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&bench_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.extension().is_some_and(|ext| ext == "json") {
                continue;
            }
            // Skip the current run.
            if path
                .file_stem()
                .is_some_and(|s| s.to_string_lossy().contains(run_id))
            {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let run: BenchRun = match serde_json::from_str(&content) {
                Ok(r) => r,
                Err(_) => continue,
            };
            if run.suite_id != suite_id
                || run.status != BenchRunStatus::Completed
                || run.results.is_empty()
            {
                continue;
            }
            let run_model = run
                .overrides
                .model
                .as_deref()
                .unwrap_or("unknown");
            for r in &run.results {
                baseline_metrics.push(TaskMetric {
                    timestamp: String::new(),
                    run_id: run.id.clone(),
                    config_hash: config_hash.clone(),
                    plan_id: suite_id.to_string(),
                    task_id: r.task_id.clone(),
                    iteration: 1,
                    role: "bench".to_string(),
                    backend: "bench".to_string(),
                    model: run_model.to_string(),
                    complexity_band: "standard".to_string(),
                    gate: "bench".to_string(),
                    gate_passed: r.passed(),
                    wall_time_ms: r.duration_ms,
                    input_tokens: r.tokens_in,
                    output_tokens: r.tokens_out,
                    cached_tokens: 0,
                    cost_usd: r.cost_usd,
                    sections_included: 0,
                    sections_dropped: 0,
                    context_tokens: 0,
                    cache_hit_rate: 0.0,
                });
            }
        }
    }

    let thresholds = RegressionThresholds::default();

    if baseline_metrics.len() < thresholds.min_records {
        tracing::debug!(
            run_id,
            baseline_count = baseline_metrics.len(),
            "not enough baseline data for bench regression detection"
        );
        return;
    }

    let baseline = compute_baseline(&baseline_metrics, thresholds.min_records);
    let report = detect_regressions(&baseline, &current, &thresholds);

    if report.has_regressions {
        tracing::warn!(
            run_id,
            alerts = report.alerts.len(),
            "bench regression detected"
        );
    } else {
        tracing::info!(run_id, "no bench regression detected");
    }

    state.event_bus.publish(ServerEvent::BenchRegressionReport {
        bench_id: run_id.to_string(),
        has_regressions: report.has_regressions,
        report: serde_json::to_value(&report).unwrap_or_default(),
    });
}

#[cfg(test)]
mod tests {}

#[cfg(test)]
mod import_checks {
    // Intentional wrong path for task 3: fix `helper` -> `helpers`.
    use crate::helper::MissingType;

    #[allow(dead_code)]
    fn _touch_missing_type() -> &'static str {
        core::any::type_name::<MissingType>()
    }
}
"#
}
