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
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream::{self, Stream};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::bench::{
    self, BenchConfigOverrides, BenchRun, BenchRunIndexEntry, BenchRunKind, BenchRunStatus,
    BenchRunSummary, BenchStrategy, BenchSuite, BenchTaskResult,
};
use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::state::{AppState, BenchRunHandle};
use roko_learn::playbook::PlaybookStore;
use roko_neuro::KnowledgeStore;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/bench/run", post(start_bench_run))
        .route("/bench/run/{id}", get(get_bench_run).delete(delete_bench_run))
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

    let learning_stores = if matches!(overrides.strategy, BenchStrategy::Minimal) {
        None
    } else {
        Some((
            PlaybookStore::new(state.workdir.join(".roko").join("learn").join("playbooks")),
            KnowledgeStore::for_workdir(&state.workdir),
        ))
    };
    let mut learning_totals = if let Some((playbook_store, knowledge_store)) = learning_stores.as_ref()
    {
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

                BenchTaskResult {
                    task_id: task.id.clone(),
                    task_name: task.name.clone(),
                    passed,
                    duration_ms,
                    model_used: overrides.model.clone(),
                    input_tokens,
                    output_tokens,
                    cost_usd,
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
                cost_usd: bench::estimate_cost_usd(overrides.model.as_deref(), 0, 0),
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

        results.push(task_result);

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
            if let Some(current_totals) = current_learning_totals(playbook_store, knowledge_store).await
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

/// Return (input_cost_per_1k, output_cost_per_1k) for a model slug.
fn model_pricing(slug: &str) -> (f64, f64) {
    let s = slug.to_lowercase();
    if s.contains("haiku") {
        (0.00025, 0.00125)
    } else if s.contains("opus") {
        (0.015, 0.075)
    } else if s.contains("sonnet") {
        (0.003, 0.015)
    } else if s.contains("gpt-4o-mini") {
        (0.00015, 0.0006)
    } else if s.contains("gpt-4o") || s.contains("gpt-4") {
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
    } else if s.contains("gpt-4o") {
        128_000
    } else if s.contains("gemini") {
        1_000_000
    } else if s.contains("llama") {
        128_000
    } else {
        128_000
    }
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
                            | ServerEvent::BenchLearningEvent { .. }
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
