//! Bench run endpoints.
//!
//! Provides routes for starting, tracking, comparing, and analyzing
//! benchmark runs that exercise roko's `run_once()` pipeline.

use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream::{self, Stream};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::bench::{
    self, BenchConfigOverrides, BenchRun, BenchRunIndexEntry, BenchRunKind, BenchRunStatus,
    BenchRunSummary, BenchStrategy, BenchSuite, BenchTaskResult, MatrixLaneConfig, MatrixRun,
    MatrixRunStatus,
};
use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::state::{AppState, BenchRunHandle, MatrixRunHandle};
use roko_learn::episode_logger::{Episode, GateVerdict, Usage};
use roko_learn::playbook::PlaybookStore;
use roko_learn::runtime_feedback::{CompletedRunInput, LearningRuntime};
use roko_neuro::KnowledgeStore;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/bench/run", post(start_bench_run))
        .route(
            "/bench/run/{id}",
            get(get_bench_run).delete(delete_bench_run),
        )
        .route("/bench/run/{id}/status", get(bench_run_status))
        // Frontend uses /bench/runs (plural) for both listing and starting.
        .route("/bench/runs", get(list_bench_runs).post(start_bench_run))
        // Frontend polls /bench/runs/:id for run status.
        .route("/bench/runs/{id}", get(get_bench_run))
        // Frontend subscribes to /bench/runs/:id/events for SSE.
        .route("/bench/runs/{id}/events", get(bench_events_sse))
        // Frontend cancels via /bench/runs/:id/cancel.
        .route("/bench/runs/{id}/cancel", post(delete_bench_run))
        .route("/bench/runs/compare", get(compare_bench_runs))
        .route("/bench/suites", get(list_suites))
        .route("/bench/suites/{id}", get(get_suite))
        .route("/bench/suites", post(upload_suite))
        .route("/bench/models", get(list_models))
        .route("/bench/cost-summary", get(bench_cost_summary))
        .route("/bench/pareto", get(pareto_frontier))
        .route("/bench/export/{id}", get(export_bench_run))
        .route("/bench/events", get(bench_events_sse))
        // Matrix (multi-lane) bench runs.
        .route(
            "/bench/matrix",
            get(list_matrix_runs).post(start_matrix_run),
        )
        .route("/bench/matrix/{id}", get(get_matrix_run))
        .route("/bench/matrix/{id}/cancel", post(cancel_matrix_run))
}

// ---------------------------------------------------------------------------
// Request / query types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct StartBenchRequest {
    suite_id: String,
    #[serde(default)]
    label: Option<String>,
    /// Accept both `overrides` (backend canonical) and `config` (frontend sends).
    #[serde(default, alias = "config")]
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
    let _ = bench::append_index_entry(&state.workdir, &index_entry).await;

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

    let bench_workdir = match scaffold_bench_workdir(&suite.id, &run_id, &state.workdir).await {
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
                let _ = bench::save_bench_run(&state.workdir, &run).await;
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
            let _ = bench::update_index_entry(&state.workdir, &failed_index_entry).await;

            state.active_bench_runs.write().await.remove(&run_id);
            return;
        }
    };
    let _bench_workdir_cleanup = BenchWorkdirCleanup::new(bench_workdir.clone());

    let learn_root = state.workdir.join(".roko").join("learn");
    let learning_rt = if matches!(overrides.strategy, BenchStrategy::Minimal) {
        None
    } else {
        match LearningRuntime::open_under(&learn_root).await {
            Ok(rt) => Some(rt),
            Err(err) => {
                tracing::warn!(error = %err, "failed to open LearningRuntime for bench run");
                None
            }
        }
    };
    let learning_stores = if learning_rt.is_some() {
        Some((
            PlaybookStore::new(learn_root.join("playbooks")),
            KnowledgeStore::for_workdir(&state.workdir),
        ))
    } else {
        None
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

                let cost_usd = bench::estimate_cost_usd(
                    overrides.model.as_deref(),
                    input_tokens,
                    output_tokens,
                );
                let output_preview = run_result
                    .output_text
                    .as_ref()
                    .map(|t| t.chars().take(500).collect());

                let gate_verdicts: Vec<serde_json::Value> = run_result
                    .gate_results
                    .iter()
                    .map(|g| {
                        json!({
                            "gate": g.gate,
                            "passed": g.passed,
                            "message": g.detail,
                        })
                    })
                    .collect();

                BenchTaskResult {
                    task_id: task.id.clone(),
                    task_name: task.name.clone(),
                    status: if passed { "pass" } else { "fail" }.to_string(),
                    duration_ms,
                    model: overrides
                        .model
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string()),
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
                status: "fail".to_string(),
                duration_ms,
                model: overrides
                    .model
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                tokens_in: 0,
                tokens_out: 0,
                cost_usd: bench::estimate_cost_usd(overrides.model.as_deref(), 0, 0),
                gate_verdicts: Vec::new(),
                retries_used: 0,
                output_preview: None,
                error: Some(format!("{e}")),
            },
        };

        // Publish task completion with the full result object.
        let result_value = serde_json::to_value(&task_result).unwrap_or_default();
        state.event_bus.publish(ServerEvent::BenchTaskCompleted {
            bench_id: run_id.clone(),
            task_id: task_result.task_id.clone(),
            result: result_value,
        });

        // Emit per-gate verdicts.
        for v in &task_result.gate_verdicts {
            let gate = v
                .get("gate")
                .and_then(|g| g.as_str())
                .unwrap_or("unknown")
                .to_string();
            let passed = v.get("passed").and_then(|p| p.as_bool()).unwrap_or(false);
            let message = v
                .get("message")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string());
            state.event_bus.publish(ServerEvent::BenchGateVerdict {
                bench_id: run_id.clone(),
                task_id: task_result.task_id.clone(),
                gate,
                passed,
                message,
                duration_ms: task_result.duration_ms,
            });
        }

        // Emit token velocity.
        let duration_secs = task_result.duration_ms as f64 / 1000.0;
        let tokens_per_second = if duration_secs > 0.0 {
            (task_result.tokens_in + task_result.tokens_out) as f64 / duration_secs
        } else {
            0.0
        };
        state.event_bus.publish(ServerEvent::BenchTokenVelocity {
            bench_id: run_id.clone(),
            task_id: task_result.task_id.clone(),
            tokens_per_second,
            tokens_in: task_result.tokens_in,
            tokens_out: task_result.tokens_out,
            duration_ms: task_result.duration_ms,
        });

        // Emit agent output preview.
        if let Some(ref preview) = task_result.output_preview {
            state.event_bus.publish(ServerEvent::BenchAgentOutput {
                bench_id: run_id.clone(),
                task_id: task_result.task_id.clone(),
                agent_id: task_result.model.clone(),
                content: preview.clone(),
                done: true,
                tool_calls: None,
                reasoning: None,
            });
        }

        results.push(task_result);

        // Record episode via LearningRuntime when available.
        if let Some(ref rt) = learning_rt {
            let last = results.last().expect("just pushed");
            let episode = bench_task_to_episode(last, task, &overrides, &run_id, &suite.id);
            if let Err(err) = rt
                .record_completed_run(CompletedRunInput::from_episode(episode))
                .await
            {
                tracing::warn!(error = %err, task_id = %task.id, "failed to record bench episode");
            }
        }

        // Compute cumulative cost so far.
        let cost_so_far: f64 = results.iter().map(|r| r.cost_usd).sum();

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
        if let Ok(Some(mut run)) = bench::load_bench_run(&state.workdir, &run_id).await {
            run.results = results.clone();
            run.current_task_index = idx + 1;
            let _ = bench::save_bench_run(&state.workdir, &run).await;
        }
    }

    // Finalize the run.
    let summary = BenchRunSummary::from_results(&results);
    let finished_at = now_secs();

    if let Ok(Some(mut run)) = bench::load_bench_run(&state.workdir, &run_id).await {
        run.status = BenchRunStatus::Completed;
        run.finished_at = Some(finished_at);
        run.results = results;
        run.summary = Some(summary.clone());
        run.current_task_index = total_tasks;
        let _ = bench::save_bench_run(&state.workdir, &run).await;
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
    let _ = bench::update_index_entry(&state.workdir, &index_entry).await;

    // Publish completion event with the full summary object.
    let summary_value = serde_json::to_value(&summary).unwrap_or_default();
    state.event_bus.publish(ServerEvent::BenchRunCompleted {
        bench_id: run_id.clone(),
        summary: summary_value,
    });

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
    if let Ok(Some(mut run)) = bench::load_bench_run(&state.workdir, &id).await {
        if run.status == BenchRunStatus::Running {
            run.status = BenchRunStatus::Cancelled;
            run.finished_at = Some(now_secs());
            let _ = bench::save_bench_run(&state.workdir, &run).await;
        }
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// `GET /api/bench/runs` -- list bench runs.
///
/// Returns `BenchRun[]` (flat array) to match the frontend expectation.
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

    let page: Vec<_> = entries
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .collect();

    // Load full BenchRun objects for each index entry.
    let mut runs = Vec::with_capacity(page.len());
    for entry in &page {
        if let Ok(Some(run)) = bench::load_bench_run(&state.workdir, &entry.id).await {
            runs.push(serde_json::to_value(run).unwrap_or_default());
        }
    }

    Json(json!(runs))
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
///
/// Returns `BenchSuite[]` (flat array) to match the frontend expectation.
async fn list_suites(State(state): State<Arc<AppState>>) -> Json<Value> {
    bench::ensure_builtin_suites(&state.workdir).await;
    let suites = bench::load_suites(&state.workdir).await;
    Json(serde_json::to_value(suites).unwrap_or_default())
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

/// `GET /api/bench/models` -- list available models with full pricing info.
///
/// Returns `BenchModel[]` matching the frontend type:
/// `{ id, name, provider, cost_per_1k_input, cost_per_1k_output, max_tokens, context_window }`
async fn list_models(State(state): State<Arc<AppState>>) -> Json<Value> {
    let config = state.load_roko_config();
    let slugs = bench::list_models_from_config(&config);

    let models: Vec<Value> = slugs
        .into_iter()
        .map(|slug| {
            let (cost_in, cost_out) = model_pricing(&slug);
            let provider = infer_provider(&slug);
            let context_window = infer_context_window(&slug);
            json!({
                "id": slug,
                "name": slug,
                "provider": provider,
                "cost_per_1k_input": cost_in,
                "cost_per_1k_output": cost_out,
                "max_tokens": 8192,
                "context_window": context_window,
            })
        })
        .collect();

    Json(json!(models))
}

#[derive(Default)]
struct CostSummaryModel {
    cost_usd: f64,
    tokens: u64,
    tasks: u64,
}

/// `GET /api/bench/cost-summary` -- aggregate real bench result costs by model.
async fn bench_cost_summary(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let entries = bench::load_index_entries(&state.workdir).await;
    let mut by_model: HashMap<String, CostSummaryModel> = HashMap::new();

    for entry in entries {
        let Some(run) = bench::load_bench_run(&state.workdir, &entry.id)
            .await
            .map_err(|e| {
                ApiError::internal(format!("failed to load bench run {}: {e}", entry.id))
            })?
        else {
            continue;
        };

        for result in run.results {
            let model = if result.model.trim().is_empty() || result.model == "unknown" {
                run.overrides
                    .model
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string())
            } else {
                result.model
            };
            let summary = by_model.entry(model).or_default();
            summary.cost_usd += result.cost_usd;
            summary.tokens += result.tokens_in + result.tokens_out;
            summary.tasks += 1;
        }
    }

    let mut models: Vec<Value> = by_model
        .into_iter()
        .map(|(model, summary)| {
            json!({
                "model": model,
                "cost_usd": summary.cost_usd,
                "tokens": summary.tokens,
                "tasks": summary.tasks,
            })
        })
        .collect();

    models.sort_by(|a, b| {
        let a_cost = a.get("cost_usd").and_then(Value::as_f64).unwrap_or(0.0);
        let b_cost = b.get("cost_usd").and_then(Value::as_f64).unwrap_or(0.0);
        b_cost.total_cmp(&a_cost)
    });

    Ok(Json(json!({ "models": models })))
}

/// Return (input_cost_per_1k, output_cost_per_1k) for a model slug.
fn model_pricing(slug: &str) -> (f64, f64) {
    let s = slug.to_lowercase();
    if s.contains("haiku") {
        (0.00025, 0.00125)
    } else if s.contains("opus") {
        (0.015, 0.075)
    } else if s.contains("sonnet") {
        (0.003, 0.015)
    } else if s.contains("gpt-5.4-mini") || s.contains("gpt-4o-mini") {
        (0.00015, 0.0006)
    } else if s.contains("gpt-5") || s.contains("gpt-4") {
        (0.005, 0.015)
    } else if s.contains("o3-mini") || s.contains("o1-mini") {
        (0.0011, 0.0044)
    } else if s.contains("gemini") {
        (0.00125, 0.01)
    } else if s.contains("llama") || s.contains("cerebras") {
        (0.0001, 0.0001)
    } else {
        (0.003, 0.015)
    }
}

fn infer_provider(slug: &str) -> &'static str {
    let s = slug.to_lowercase();
    if s.contains("claude") || s.contains("haiku") || s.contains("sonnet") || s.contains("opus") {
        "Anthropic"
    } else if s.contains("gpt") || s.contains("o3") || s.contains("o1") {
        "OpenAI"
    } else if s.contains("gemini") {
        "Google"
    } else if s.contains("llama") || s.contains("cerebras") {
        "Cerebras"
    } else if s.contains("codex") {
        "OpenAI"
    } else {
        "Unknown"
    }
}

fn infer_context_window(slug: &str) -> u64 {
    let s = slug.to_lowercase();
    if s.contains("haiku") || s.contains("sonnet") || s.contains("opus") {
        200_000
    } else if s.contains("gpt-5") || s.contains("gpt-4") {
        128_000
    } else if s.contains("gemini") {
        1_000_000
    } else {
        // llama and other models default to 128k
        128_000
    }
}

/// `GET /api/bench/pareto` -- compute pareto frontier.
async fn pareto_frontier(State(state): State<Arc<AppState>>) -> Json<Value> {
    let frontier = bench::compute_pareto_frontier(&state.workdir).await;
    let points: Vec<Value> = frontier
        .iter()
        .map(|point| {
            json!({
                "run_id": point.run_id,
                "suite_id": point.suite_id,
                "model": point.model,
                "label": point.label,
                "pass_rate": point.pass_rate,
                "cost_usd": point.total_cost_usd,
                "total_cost_usd": point.total_cost_usd,
            })
        })
        .collect();
    Json(json!({ "frontier": frontier, "points": points }))
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

// ---------------------------------------------------------------------------
// Matrix (multi-lane) bench run handlers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct StartMatrixRequest {
    suite_id: String,
    lanes: Vec<MatrixLaneConfigInput>,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Deserialize)]
struct MatrixLaneConfigInput {
    model: String,
    #[serde(default)]
    backend: Option<String>,
    #[serde(default)]
    strategy: BenchStrategy,
    #[serde(default)]
    label: Option<String>,
}

/// `POST /api/bench/matrix` -- start a matrix (multi-lane) bench run.
async fn start_matrix_run(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StartMatrixRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if body.lanes.is_empty() {
        return Err(ApiError::bad_request("at least one lane is required"));
    }

    bench::ensure_builtin_suites(&state.workdir).await;
    let suite = bench::load_suite(&state.workdir, &body.suite_id)
        .await
        .ok_or_else(|| ApiError::not_found("suite not found"))?;

    let matrix_id = uuid::Uuid::new_v4().to_string();
    let started_at = now_secs();

    let mut lane_ids = Vec::with_capacity(body.lanes.len());
    let mut lane_configs = Vec::with_capacity(body.lanes.len());
    let mut lane_handles = Vec::with_capacity(body.lanes.len());

    for (i, lane_input) in body.lanes.iter().enumerate() {
        let lane_id = uuid::Uuid::new_v4().to_string();
        let lane_label = lane_input
            .label
            .clone()
            .unwrap_or_else(|| format!("lane-{i}"));

        let overrides = BenchConfigOverrides {
            model: Some(lane_input.model.clone()),
            backend: lane_input.backend.clone(),
            strategy: lane_input.strategy,
            ..BenchConfigOverrides::default()
        };

        let run_label = format!("matrix:{matrix_id}:{lane_label}");

        // Create the per-lane BenchRun.
        let run = BenchRun {
            id: lane_id.clone(),
            suite_id: suite.id.clone(),
            suite_name: suite.name.clone(),
            kind: BenchRunKind::Comparison,
            overrides: overrides.clone(),
            label: Some(run_label.clone()),
            status: BenchRunStatus::Running,
            started_at,
            finished_at: None,
            results: Vec::new(),
            summary: None,
            current_task_index: 0,
            total_tasks: suite.tasks.len(),
        };
        if let Err(e) = bench::save_bench_run(&state.workdir, &run).await {
            tracing::warn!(error = %e, "failed to save matrix lane run");
        }

        let index_entry = BenchRunIndexEntry {
            id: lane_id.clone(),
            suite_id: suite.id.clone(),
            suite_name: suite.name.clone(),
            status: BenchRunStatus::Running,
            started_at,
            finished_at: None,
            label: Some(run_label),
            model: Some(lane_input.model.clone()),
            pass_rate: None,
            total_cost_usd: None,
        };
        let _ = bench::append_index_entry(&state.workdir, &index_entry).await;

        // Spawn the bench run for this lane (reuses existing execute_bench_run).
        let handle = tokio::spawn(execute_bench_run(
            Arc::clone(&state),
            lane_id.clone(),
            suite.clone(),
            overrides.clone(),
            Some(format!("matrix:{matrix_id}:{lane_label}")),
            started_at,
        ));

        // Also register in active_bench_runs so per-lane cancel works.
        state.active_bench_runs.write().await.insert(
            lane_id.clone(),
            BenchRunHandle {
                id: lane_id.clone(),
                handle: tokio::spawn(async {}), // placeholder; real handle in matrix
            },
        );

        lane_ids.push(lane_id);
        lane_configs.push(MatrixLaneConfig {
            model: lane_input.model.clone(),
            backend: lane_input.backend.clone(),
            strategy: lane_input.strategy,
            label: Some(lane_label),
            overrides,
        });
        lane_handles.push(handle);
    }

    // Save the matrix record.
    let matrix_run = MatrixRun {
        id: matrix_id.clone(),
        suite_id: suite.id.clone(),
        suite_name: suite.name.clone(),
        lane_ids: lane_ids.clone(),
        lanes: lane_configs,
        status: MatrixRunStatus::Running,
        started_at,
        finished_at: None,
        label: body.label.clone(),
    };
    if let Err(e) = bench::save_matrix_run(&state.workdir, &matrix_run).await {
        tracing::warn!(error = %e, "failed to save matrix run");
    }

    // Publish start event.
    state.event_bus.publish(ServerEvent::MatrixRunStarted {
        matrix_id: matrix_id.clone(),
        suite_id: suite.id.clone(),
        lane_ids: lane_ids.clone(),
        total_lanes: lane_ids.len(),
    });

    // Store the matrix handle.
    state.active_matrix_runs.write().await.insert(
        matrix_id.clone(),
        MatrixRunHandle {
            id: matrix_id.clone(),
            lane_handles,
        },
    );

    // Spawn a monitor task that watches lane completion.
    let monitor_state = Arc::clone(&state);
    let monitor_matrix_id = matrix_id.clone();
    let monitor_lane_ids = lane_ids.clone();
    tokio::spawn(async move {
        monitor_matrix_completion(monitor_state, monitor_matrix_id, monitor_lane_ids).await;
    });

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "id": matrix_id, "matrix_id": matrix_id, "lane_ids": lane_ids })),
    ))
}

/// Background monitor that polls lane statuses and publishes matrix events.
async fn monitor_matrix_completion(state: Arc<AppState>, matrix_id: String, lane_ids: Vec<String>) {
    let mut completed_lanes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut lane_summaries: Vec<Value> = Vec::new();

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let mut all_done = true;
        for lane_id in &lane_ids {
            if completed_lanes.contains(lane_id) {
                continue;
            }
            if let Ok(Some(run)) = bench::load_bench_run(&state.workdir, lane_id).await {
                if run.status == BenchRunStatus::Running {
                    all_done = false;
                } else {
                    completed_lanes.insert(lane_id.clone());
                    let pass_rate = run.summary.as_ref().map(|s| s.pass_rate).unwrap_or(0.0);
                    let cost_usd = run
                        .summary
                        .as_ref()
                        .map(|s| s.total_cost_usd)
                        .unwrap_or(0.0);

                    state.event_bus.publish(ServerEvent::MatrixLaneCompleted {
                        matrix_id: matrix_id.clone(),
                        lane_id: lane_id.clone(),
                        pass_rate,
                        cost_usd,
                    });

                    lane_summaries.push(json!({
                        "lane_id": lane_id,
                        "model": run.overrides.model,
                        "strategy": run.overrides.strategy,
                        "status": run.status,
                        "pass_rate": pass_rate,
                        "cost_usd": cost_usd,
                        "duration_ms": run.summary.as_ref().map(|s| s.total_duration_ms).unwrap_or(0),
                        "total_tasks": run.summary.as_ref().map(|s| s.total_tasks).unwrap_or(run.total_tasks),
                        "passed": run.summary.as_ref().map(|s| s.passed).unwrap_or(0),
                        "failed": run.summary.as_ref().map(|s| s.failed).unwrap_or(0),
                        "summary": run.summary,
                    }));
                }
            } else {
                all_done = false;
            }
        }

        if all_done {
            break;
        }
    }

    // Determine overall status.
    let any_failed = lane_summaries.iter().any(|s| {
        s.get("status")
            .and_then(|v| serde_json::from_value::<BenchRunStatus>(v.clone()).ok())
            .is_some_and(|status| {
                matches!(status, BenchRunStatus::Failed | BenchRunStatus::Cancelled)
            })
    });

    let finished_at = now_secs();
    let overall_status = if any_failed {
        MatrixRunStatus::PartialFailure
    } else {
        MatrixRunStatus::Completed
    };

    // Update and save matrix run.
    if let Ok(Some(mut matrix)) = bench::load_matrix_run(&state.workdir, &matrix_id).await {
        matrix.status = overall_status;
        matrix.finished_at = Some(finished_at);
        let _ = bench::save_matrix_run(&state.workdir, &matrix).await;
    }

    // Publish completion event.
    state.event_bus.publish(ServerEvent::MatrixRunCompleted {
        matrix_id: matrix_id.clone(),
        summary: lane_summaries,
    });

    // Clean up handle.
    state.active_matrix_runs.write().await.remove(&matrix_id);
}

/// `GET /api/bench/matrix/:id` -- get full matrix run status.
async fn get_matrix_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let matrix = bench::load_matrix_run(&state.workdir, &id)
        .await
        .map_err(|e| ApiError::internal(format!("failed to load matrix run: {e}")))?
        .ok_or_else(|| ApiError::not_found("matrix run not found"))?;

    // Load per-lane run details.
    let mut lane_details = Vec::with_capacity(matrix.lane_ids.len());
    for (i, lane_id) in matrix.lane_ids.iter().enumerate() {
        let lane_config = matrix.lanes.get(i);
        let run = bench::load_bench_run(&state.workdir, lane_id)
            .await
            .ok()
            .flatten();
        lane_details.push(json!({
            "lane_id": lane_id,
            "config": lane_config,
            "run": run.as_ref().map(|r| serde_json::to_value(r).unwrap_or_default()),
        }));
    }

    Ok(Json(json!({
        "id": matrix.id,
        "suite_id": matrix.suite_id,
        "suite_name": matrix.suite_name,
        "status": matrix.status,
        "started_at": matrix.started_at,
        "finished_at": matrix.finished_at,
        "label": matrix.label,
        "lanes": lane_details,
    })))
}

/// `POST /api/bench/matrix/:id/cancel` -- cancel all lanes of a matrix run.
async fn cancel_matrix_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    // Abort all lane handles.
    let removed = state.active_matrix_runs.write().await.remove(&id);
    if let Some(handle) = removed {
        for lane_handle in handle.lane_handles {
            lane_handle.abort();
        }
    }

    // Also try to cancel individual lane bench runs.
    if let Ok(Some(matrix)) = bench::load_matrix_run(&state.workdir, &id).await {
        for lane_id in &matrix.lane_ids {
            let removed_bench = state.active_bench_runs.write().await.remove(lane_id);
            if let Some(bench_handle) = removed_bench {
                bench_handle.handle.abort();
            }
            if let Ok(Some(mut run)) = bench::load_bench_run(&state.workdir, lane_id).await {
                if run.status == BenchRunStatus::Running {
                    run.status = BenchRunStatus::Cancelled;
                    run.finished_at = Some(now_secs());
                    let _ = bench::save_bench_run(&state.workdir, &run).await;
                }
            }
        }

        // Update matrix status.
        if let Ok(Some(mut m)) = bench::load_matrix_run(&state.workdir, &id).await {
            m.status = MatrixRunStatus::Cancelled;
            m.finished_at = Some(now_secs());
            let _ = bench::save_matrix_run(&state.workdir, &m).await;
        }
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// `GET /api/bench/matrix` -- list all matrix runs.
async fn list_matrix_runs(State(state): State<Arc<AppState>>) -> Json<Value> {
    let runs = bench::list_matrix_runs(&state.workdir).await;
    Json(serde_json::to_value(runs).unwrap_or_default())
}

/// `GET /api/bench/events` -- SSE stream filtered to bench events.
async fn bench_events_sse(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
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
                            | ServerEvent::BenchGateVerdict { .. }
                            | ServerEvent::BenchTokenVelocity { .. }
                            | ServerEvent::BenchAgentOutput { .. }
                            | ServerEvent::BenchLearningEvent { .. }
                            | ServerEvent::BenchProgress { .. }
                            | ServerEvent::BenchRunCompleted { .. }
                            | ServerEvent::MatrixRunStarted { .. }
                            | ServerEvent::MatrixLaneCompleted { .. }
                            | ServerEvent::MatrixRunCompleted { .. }
                    );
                    if !is_bench {
                        continue;
                    }
                    let data = serde_json::to_string(&envelope.payload).unwrap_or_default();
                    let sse_event = Event::default().data(data).id(envelope.seq.to_string());
                    return Some((Ok(sse_event), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(n, "bench SSE client lagged");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build an [`Episode`] from a completed bench task result.
fn bench_task_to_episode(
    result: &BenchTaskResult,
    task: &bench::BenchTask,
    overrides: &BenchConfigOverrides,
    run_id: &str,
    suite_id: &str,
) -> Episode {
    let task_id = format!("{run_id}/{}", task.id);
    let mut episode = Episode::new(format!("bench-{suite_id}"), task_id);
    episode.kind = "bench_task".to_string();
    episode.episode_id = format!("bench-{run_id}-{}", task.id);
    episode.agent_template = format!("bench/{suite_id}");
    episode.model = overrides
        .model
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    episode.backend = overrides
        .backend
        .clone()
        .unwrap_or_else(|| "roko-bench".to_string());
    episode.trigger_kind = "bench".to_string();
    episode.completed_at = chrono::Utc::now();
    episode.duration_secs = result.duration_ms as f64 / 1000.0;
    episode.success = result.passed();
    episode.turns = 1;
    episode.tokens_used = result.tokens_in.saturating_add(result.tokens_out);
    episode.usage = Usage {
        input_tokens: result.tokens_in,
        output_tokens: result.tokens_out,
        cost_usd: result.cost_usd,
        wall_ms: result.duration_ms,
        ..Usage::default()
    };
    episode.gate_verdicts = result
        .gate_verdicts
        .iter()
        .filter_map(|v| {
            let gate = v.get("gate")?.as_str()?;
            let passed = v.get("passed")?.as_bool()?;
            Some(GateVerdict::new(gate, passed))
        })
        .collect();
    if episode.gate_verdicts.is_empty() {
        // Synthesize a basic verdict from the pass/fail status.
        episode
            .gate_verdicts
            .push(GateVerdict::new("bench:outcome", result.passed()));
    }
    episode.failure_reason = result.error.clone();
    episode
}

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
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

async fn scaffold_bench_workdir(
    suite_id: &str,
    run_id: &str,
    server_workdir: &std::path::Path,
) -> anyhow::Result<PathBuf> {
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

        // Copy roko.toml from the server workspace so the agent dispatch can
        // find provider/model configuration (API keys, endpoints, etc.).
        let server_roko_toml = server_workdir.join("roko.toml");
        if tokio::fs::try_exists(&server_roko_toml)
            .await
            .unwrap_or(false)
        {
            let _ = tokio::fs::copy(&server_roko_toml, dir.join("roko.toml")).await;
        }

        // Initialize a git repo so agent tooling (Claude CLI, etc.) can work.
        let dir_clone = dir.clone();
        tokio::task::spawn_blocking(move || {
            for args in [
                &["init"][..],
                &["add", "-A"][..],
                &["commit", "-m", "bench scaffold", "--allow-empty"][..],
            ] {
                let _ = std::process::Command::new("git")
                    .args(args)
                    .current_dir(&dir_clone)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
        })
        .await
        .ok();

        Ok::<(), anyhow::Error>(())
    }
    .await
    {
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
    r"fn main() {}
"
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
    r"/// Type referenced by the broken import in `src/lib.rs`.
pub struct MissingType;
"
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
