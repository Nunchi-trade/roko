//! Single-prompt run endpoints.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::state::{AppState, OperationStatus, RunHandle};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/run", post(start_run))
        .route("/run/{id}/status", get(run_status))
}

#[derive(Deserialize)]
struct RunRequest {
    prompt: String,
    #[serde(default)]
    workdir: Option<String>,
}

/// `POST /api/run` — spawn a background `run_once()` invocation.
async fn start_run(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RunRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let run_id = uuid::Uuid::new_v4().to_string();
    let workdir = body
        .workdir
        .map_or_else(|| state.workdir.clone(), std::path::PathBuf::from);
    let config = state.config.read().await.clone();
    let prompt = body.prompt.clone();
    let bus = state.event_bus.clone();

    let handle = tokio::spawn({
        let run_id = run_id.clone();
        async move {
            bus.publish(ServerEvent::RunStarted {
                run_id: run_id.clone(),
                prompt: prompt.clone(),
            });

            match roko_cli::run_once(&workdir, &config, &prompt).await {
                Ok(report) => {
                    let success = report.overall_success();
                    bus.publish(ServerEvent::RunCompleted { run_id, success });
                }
                Err(e) => {
                    bus.publish(ServerEvent::Error {
                        message: format!("run failed: {e}"),
                    });
                    bus.publish(ServerEvent::RunCompleted {
                        run_id,
                        success: false,
                    });
                }
            }
        }
    });

    let run_handle = RunHandle {
        id: run_id.clone(),
        prompt: body.prompt,
        status: OperationStatus::Running,
        handle,
    };

    state
        .active_runs
        .write()
        .await
        .insert(run_id.clone(), run_handle);

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "id": run_id })),
    ))
}

/// `GET /api/run/:id/status` — check the status of a background run.
async fn run_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let runs = state.active_runs.read().await;
    let handle = runs
        .get(&id)
        .ok_or_else(|| ApiError::not_found("run not found"))?;

    let result = Json(json!({
        "id": handle.id,
        "prompt": handle.prompt,
        "status": format!("{:?}", handle.status),
        "finished": handle.handle.is_finished(),
    }));
    drop(runs);

    Ok(result)
}
