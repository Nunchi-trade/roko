//! Single-prompt run endpoints.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use validator::Validate;

use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::extract::{RequestPayload, ValidJson, validate_with_validator};
use crate::runtime::RunResult;
use crate::state::{AppState, OperationStatus, RunHandle};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/run", post(start_run))
        .route("/run/{id}/status", get(run_status))
}

#[derive(Deserialize, Validate)]
struct RunRequest {
    #[validate(
        length(min = 1),
        custom(function = "crate::extract::validate_non_blank")
    )]
    prompt: String,
    #[serde(default)]
    workdir: Option<String>,
}

impl RequestPayload for RunRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        validate_with_validator(self)
    }
}

/// `POST /api/run` — spawn a background `run_once()` invocation.
async fn start_run(
    State(state): State<Arc<AppState>>,
    ValidJson(body): ValidJson<RunRequest>,
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

    let (status, error) = operation_status_parts(&handle.status);
    let result = Json(json!({
        "id": handle.id,
        "prompt": handle.prompt,
        "status": status,
        "success": handle.result.as_ref().map(|result| result.success),
        "output_text": handle.result.as_ref().and_then(|result| result.output_text.clone()),
        "error": error,
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
    let state_for_task = Arc::clone(state);

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
                    state_for_task.provider_health.record_success("default");
                    record_run_result(&state_for_task, &run_id, result.clone()).await;
                    publish_run_completed(
                        &bus,
                        &run_id,
                        agent_target.as_deref(),
                        result.success,
                        result.output_text.as_ref().map(|output| json!({
                            "output_text": output,
                        })),
                    );
                }
                Err(e) => {
                    state_for_task.provider_health.record_failure("default");
                    let error_message = format!("run failed: {e}");
                    record_run_failure(&state_for_task, &run_id, &error_message).await;
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
        result: None,
        handle,
    };

    state
        .active_runs
        .write()
        .await
        .insert(run_id.clone(), run_handle);
    run_id
}

async fn record_run_result(state: &AppState, run_id: &str, result: RunResult) {
    if let Some(handle) = state.active_runs.write().await.get_mut(run_id) {
        handle.status = OperationStatus::Completed {
            result: result.output_text.clone(),
        };
        handle.result = Some(result);
    }
}

async fn record_run_failure(state: &AppState, run_id: &str, error_message: &str) {
    if let Some(handle) = state.active_runs.write().await.get_mut(run_id) {
        handle.status = OperationStatus::Failed {
            error: error_message.to_string(),
        };
        handle.result = Some(RunResult {
            success: false,
            output_text: None,
        });
    }
}

fn operation_status_parts(status: &OperationStatus) -> (&'static str, Option<&str>) {
    match status {
        OperationStatus::Running => ("running", None),
        OperationStatus::Completed { .. } => ("completed", None),
        OperationStatus::Failed { error } => ("failed", Some(error.as_str())),
    }
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
