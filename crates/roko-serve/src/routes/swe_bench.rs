//! SWE-bench HTTP endpoints.
//!
//! Provides routes for starting, listing, and inspecting SWE-bench
//! evaluation runs through the HTTP control plane.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::runtime::SweBenchRunOptions;
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/bench/swe/run", post(start_swe_run))
        .route("/bench/swe/runs", get(list_swe_runs))
        .route("/bench/swe/runs/{id}", get(get_swe_run))
        .route("/bench/swe/datasets", get(list_swe_datasets))
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct StartSweRunRequest {
    #[serde(default)]
    dataset_path: Option<String>,
    #[serde(default = "default_agent_mode")]
    agent_mode: String,
    #[serde(default = "default_batch_size")]
    batch_size: usize,
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    record_learning: bool,
}

fn default_agent_mode() -> String {
    "gold".to_string()
}

fn default_batch_size() -> usize {
    10
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn swe_dir(workdir: &std::path::Path) -> std::path::PathBuf {
    workdir.join(".roko").join("bench").join("swe")
}

fn swe_run_path(workdir: &std::path::Path, run_id: &str) -> std::path::PathBuf {
    swe_dir(workdir).join(format!("{run_id}.json"))
}

fn swe_datasets_dir(workdir: &std::path::Path) -> std::path::PathBuf {
    swe_dir(workdir).join("datasets")
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /api/bench/swe/run` -- start a SWE-bench evaluation run.
async fn start_swe_run(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StartSweRunRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let options = SweBenchRunOptions {
        dataset_path: body.dataset_path.map(std::path::PathBuf::from),
        agent_mode: body.agent_mode,
        batch_size: body.batch_size,
        offset: body.offset,
        record_learning: body.record_learning,
    };

    let run_id = uuid::Uuid::new_v4().to_string();
    let dataset_label = options
        .dataset_path
        .as_ref()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("built-in")
        .to_string();

    // Publish start event.
    state.event_bus.publish(ServerEvent::SweRunStarted {
        run_id: run_id.clone(),
        dataset: dataset_label.clone(),
        total_instances: options.batch_size,
    });

    // Spawn background execution.
    let workdir = state.workdir.clone();
    let runtime = Arc::clone(&state.runtime);
    let event_bus = state.event_bus.clone();
    let swe_workdir = workdir.clone();
    let spawned_run_id = run_id.clone();

    tokio::spawn(async move {
        match runtime.run_swe_bench(&workdir, options).await {
            Ok(result) => {
                // Save result to disk.
                let path = swe_run_path(&swe_workdir, &result.run_id);
                if let Some(parent) = path.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                if let Ok(json) = serde_json::to_string_pretty(&result) {
                    let _ = tokio::fs::write(&path, json).await;
                }

                // Publish per-instance and completion events.
                for inst in &result.instances {
                    event_bus.publish(ServerEvent::SweInstanceCompleted {
                        run_id: spawned_run_id.clone(),
                        instance_id: inst.instance_id.clone(),
                        resolved: inst.resolved,
                        duration_ms: inst.duration_ms,
                    });
                }

                event_bus.publish(ServerEvent::SweRunCompleted {
                    run_id: spawned_run_id,
                    resolved: result.resolved as u32,
                    total: result.total as u32,
                    pass_rate: result.pass_rate,
                });
            }
            Err(err) => {
                tracing::warn!(error = %err, run_id = %spawned_run_id, "SWE-bench run failed");
                event_bus.publish(ServerEvent::SweRunCompleted {
                    run_id: spawned_run_id,
                    resolved: 0,
                    total: 0,
                    pass_rate: 0.0,
                });
            }
        }
    });

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "run_id": run_id, "dataset": dataset_label })),
    ))
}

/// `GET /api/bench/swe/runs` -- list SWE-bench runs.
async fn list_swe_runs(State(state): State<Arc<AppState>>) -> Json<Value> {
    let dir = swe_dir(&state.workdir);
    let mut runs = Vec::new();

    if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Ok(data) = tokio::fs::read_to_string(&path).await {
                    if let Ok(run) = serde_json::from_str::<Value>(&data) {
                        runs.push(run);
                    }
                }
            }
        }
    }

    // Sort by run_id descending (UUIDs have a time component).
    runs.sort_by(|a, b| {
        let a_id = a.get("run_id").and_then(|v| v.as_str()).unwrap_or("");
        let b_id = b.get("run_id").and_then(|v| v.as_str()).unwrap_or("");
        b_id.cmp(a_id)
    });

    Json(json!(runs))
}

/// `GET /api/bench/swe/runs/:id` -- get a full SWE-bench run report.
async fn get_swe_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let path = swe_run_path(&state.workdir, &id);
    if !path.exists() {
        return Err(ApiError::not_found("SWE-bench run not found"));
    }
    let data = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| ApiError::internal(format!("failed to read run: {e}")))?;
    let run: Value =
        serde_json::from_str(&data).map_err(|e| ApiError::internal(format!("parse error: {e}")))?;
    Ok(Json(run))
}

/// `GET /api/bench/swe/datasets` -- list available SWE-bench dataset files.
async fn list_swe_datasets(State(state): State<Arc<AppState>>) -> Json<Value> {
    let dir = swe_datasets_dir(&state.workdir);
    let mut datasets = Vec::new();

    if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|ext| ext == "jsonl" || ext == "json")
            {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    let size = tokio::fs::metadata(&path)
                        .await
                        .map(|m| m.len())
                        .unwrap_or(0);
                    datasets.push(json!({
                        "name": name,
                        "path": path,
                        "size_bytes": size,
                    }));
                }
            }
        }
    }

    datasets.sort_by(|a, b| {
        let a_name = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let b_name = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
        a_name.cmp(b_name)
    });

    Json(json!({ "datasets": datasets }))
}
