//! Dream cycle endpoint.

use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::extract::{RequestPayload, ValidJson};
use crate::state::{AppState, OperationHandle, OperationStatus};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/dream/run", post(dream_run))
}

#[derive(Debug, Deserialize)]
struct DreamRunRequest {
    #[serde(default = "default_mode")]
    mode: String,
}

fn default_mode() -> String {
    "full".to_string()
}

impl RequestPayload for DreamRunRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        Ok(())
    }
}

/// `POST /api/dream/run` — trigger a dream consolidation cycle in the background.
async fn dream_run(
    State(state): State<Arc<AppState>>,
    ValidJson(_body): ValidJson<DreamRunRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let op_id = uuid::Uuid::new_v4().to_string();
    let bus = state.event_bus.clone();
    let workdir = state.workdir.clone();

    let handle = tokio::spawn({
        let op_id = op_id.clone();
        async move {
            bus.publish(ServerEvent::OperationStarted {
                op_id: op_id.clone(),
                kind: "dream_run".into(),
            });

            let result = tokio::task::spawn_blocking(move || {
                let config = roko_dreams::DreamLoopConfig {
                    auto_dream: true,
                    idle_threshold_mins: 0,
                    min_episodes_for_dream: 0,
                    agent: roko_dreams::DreamAgentConfig {
                        command: "cat".to_string(),
                        args: Vec::new(),
                        model: None,
                        bare_mode: true,
                        effort: "medium".to_string(),
                        fallback_model: None,
                        timeout_ms: 120_000,
                        env: Vec::new(),
                    },
                };
                let mut runner = roko_dreams::DreamRunner::new(workdir, config);
                runner.consolidate_now()
            })
            .await;

            let success = match result {
                Ok(Ok(_report)) => true,
                Ok(Err(err)) => {
                    bus.publish(ServerEvent::Error {
                        message: format!("dream run failed: {err}"),
                    });
                    false
                }
                Err(err) => {
                    bus.publish(ServerEvent::Error {
                        message: format!("dream run panicked: {err}"),
                    });
                    false
                }
            };

            bus.publish(ServerEvent::OperationCompleted {
                op_id,
                kind: "dream_run".into(),
                success,
            });
        }
    });

    let op = OperationHandle {
        id: op_id.clone(),
        kind: "dream_run".into(),
        status: OperationStatus::Running,
        handle,
    };
    state.operations.write().await.insert(op_id.clone(), op);

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "id": op_id })),
    ))
}
