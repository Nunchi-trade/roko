//! Bench run endpoints.
//!
//! Provides routes for starting, tracking, comparing, and analyzing
//! benchmark runs that exercise roko's `run_once()` pipeline.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use futures::stream::{self, Stream};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::bench::{
    self, BenchConfigOverrides, BenchRun, BenchRunIndexEntry, BenchRunKind, BenchRunStatus,
    BenchRunSummary, BenchSuite, BenchTaskResult,
};
use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::state::{AppState, BenchRunHandle};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/bench/run", post(start_bench_run))
        .route("/bench/run/{id}", get(get_bench_run))
        .route("/bench/run/{id}/status", get(bench_run_status))
        .route("/bench/run/{id}", delete(delete_bench_run))
        .route("/bench/runs", get(list_bench_runs))
        .route("/bench/runs/compare", get(compare_bench_runs))
        .route("/bench/suites", get(list_suites))
        .route("/bench/suites/{id}", get(get_suite))
        .route("/bench/suites", post(upload_suite))
        .route("/bench/models", get(list_models))
        .route("/bench/pareto", get(pareto_frontier))
        .route("/bench/export/{id}", get(export_bench_run))
        .route("/bench/events", get(bench_events_sse))
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
    let now = now_secs();

    let run = BenchRun {
        id: run_id.clone(),
        suite_id: suite.id.clone(),
        suite_name: suite.name.clone(),
        kind: BenchRunKind::Manual,
        overrides: body.overrides.clone(),
        label: body.label.clone(),
        status: BenchRunStatus::Running,
        started_at: now,
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
        started_at: now,
        finished_at: None,
        label: body.label.clone(),
        model: body.overrides.model.clone(),
        pass_rate: None,
        total_cost_usd: None,
    };
    let _ = bench::append_index_entry(&state.workdir, &index_entry).await;

    // Publish start event.
    state.event_bus.publish(ServerEvent::BenchRunStarted {
        run_id: run_id.clone(),
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
) {
    let total_tasks = suite.tasks.len();
    let mut results = Vec::new();
    let mut passed_count = 0usize;
    let mut failed_count = 0usize;

    for (idx, task) in suite.tasks.iter().enumerate() {
        // Publish task start.
        state.event_bus.publish(ServerEvent::BenchTaskStarted {
            run_id: run_id.clone(),
            task_id: task.id.clone(),
            task_index: idx,
            total_tasks,
        });

        let start = std::time::Instant::now();
        let result = state
            .runtime
            .run_once_with_config(state.workdir.as_path(), &task.prompt, &overrides)
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

                let output_preview = run_result
                    .output_text
                    .as_ref()
                    .map(|t| t.chars().take(500).collect());

                BenchTaskResult {
                    task_id: task.id.clone(),
                    task_name: task.name.clone(),
                    passed,
                    duration_ms,
                    model_used: overrides.model.clone(),
                    input_tokens,
                    output_tokens,
                    cost_usd: 0.0,
                    output_preview,
                    error: None,
                }
            }
            Err(e) => BenchTaskResult {
                task_id: task.id.clone(),
                task_name: task.name.clone(),
                passed: false,
                duration_ms,
                model_used: overrides.model.clone(),
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: 0.0,
                output_preview: None,
                error: Some(format!("{e}")),
            },
        };

        if task_result.passed {
            passed_count += 1;
        } else {
            failed_count += 1;
        }

        // Publish task completion.
        state.event_bus.publish(ServerEvent::BenchTaskCompleted {
            run_id: run_id.clone(),
            task_id: task_result.task_id.clone(),
            passed: task_result.passed,
            duration_ms: task_result.duration_ms,
            cost_usd: task_result.cost_usd,
        });

        results.push(task_result);

        // Publish progress.
        state.event_bus.publish(ServerEvent::BenchProgress {
            run_id: run_id.clone(),
            completed: results.len(),
            total: total_tasks,
            passed: passed_count,
            failed: failed_count,
        });

        // Update on-disk state periodically.
        if let Ok(Some(mut run)) = bench::load_bench_run(&state.workdir, &run_id).await {
            run.results = results.clone();
            run.current_task_index = idx + 1;
            let _ = bench::save_bench_run(&state.workdir, &run).await;
        }
    }

    // Finalize the run.
    let summary = BenchRunSummary::from_results(&results);
    let now = now_secs();

    if let Ok(Some(mut run)) = bench::load_bench_run(&state.workdir, &run_id).await {
        run.status = BenchRunStatus::Completed;
        run.finished_at = Some(now);
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
        started_at: 0, // will be preserved by update
        finished_at: Some(now),
        label,
        model: overrides.model,
        pass_rate: Some(summary.pass_rate),
        total_cost_usd: Some(summary.total_cost_usd),
    };
    let _ = bench::update_index_entry(&state.workdir, &index_entry).await;

    // Publish completion event.
    state.event_bus.publish(ServerEvent::BenchRunCompleted {
        run_id: run_id.clone(),
        suite_id: suite.id,
        pass_rate: summary.pass_rate,
        total_cost_usd: summary.total_cost_usd,
        total_duration_ms: summary.total_duration_ms,
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
        "passed": run.results.iter().filter(|r| r.passed).count(),
        "failed": run.results.iter().filter(|r| !r.passed).count(),
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
                            | ServerEvent::BenchProgress { .. }
                            | ServerEvent::BenchRunCompleted { .. }
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

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
