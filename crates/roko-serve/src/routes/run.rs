//! Single-prompt run endpoints.

use std::path::PathBuf;
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
    let run_id = spawn_background_run(
        &state,
        body.prompt.clone(),
        body.workdir.map(PathBuf::from),
        None,
    )
    .await;

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

pub(crate) async fn spawn_background_run(
    state: &Arc<AppState>,
    prompt: String,
    workdir: Option<PathBuf>,
    agent_target: Option<String>,
) -> String {
    let run_id = uuid::Uuid::new_v4().to_string();
    let workdir = workdir.unwrap_or_else(|| state.workdir.clone());
    let bus = state.event_bus.clone();
    let runtime = state.runtime.clone();

    let handle = tokio::spawn({
        let run_id = run_id.clone();
        let prompt_for_handle = prompt.clone();
        async move {
            publish_run_started(&bus, &run_id, &prompt_for_handle, agent_target.as_deref());

            match runtime
                .run_once(workdir.as_path(), &prompt_for_handle)
                .await
            {
                Ok(result) => {
                    publish_run_completed(
                        &bus,
                        &run_id,
                        agent_target.as_deref(),
                        result.success,
                        None,
                    );
                }
                Err(e) => {
                    let error_message = format!("run failed: {e}");
                    bus.publish(ServerEvent::Error {
                        message: error_message.clone(),
                    });
                    publish_run_completed(
                        &bus,
                        &run_id,
                        agent_target.as_deref(),
                        false,
                        Some(serde_json::json!({ "error": error_message })),
                    );
                }
            }
        }
    });

    let run_handle = RunHandle {
        id: run_id.clone(),
        prompt,
        status: OperationStatus::Running,
        handle,
    };

    state
        .active_runs
        .write()
        .await
        .insert(run_id.clone(), run_handle);
    run_id
}

fn publish_run_started(
    bus: &crate::event_bus::EventBus<ServerEvent>,
    run_id: &str,
    prompt: &str,
    agent_target: Option<&str>,
) {
    bus.publish(ServerEvent::RunStarted {
        run_id: run_id.to_owned(),
        prompt: prompt.to_owned(),
    });
    if let Some(agent_id) = agent_target {
        bus.publish(ServerEvent::AgentOutput {
            agent_id: agent_id.to_owned(),
            run_id: Some(run_id.to_owned()),
            content: String::new(),
            done: false,
            metadata: Some(serde_json::json!({ "status": "started" })),
        });
    }
}

fn publish_run_completed(
    bus: &crate::event_bus::EventBus<ServerEvent>,
    run_id: &str,
    agent_target: Option<&str>,
    success: bool,
    metadata: Option<Value>,
) {
    if let Some(agent_id) = agent_target {
        bus.publish(ServerEvent::AgentOutput {
            agent_id: agent_id.to_owned(),
            run_id: Some(run_id.to_owned()),
            content: String::new(),
            done: true,
            metadata: Some(serde_json::json!({
                "status": if success { "completed" } else { "failed" },
                "success": success,
                "details": metadata.unwrap_or(Value::Null),
            })),
        });
    }

    bus.publish(ServerEvent::RunCompleted {
        run_id: run_id.to_owned(),
        success,
    });
}
