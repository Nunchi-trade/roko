//! Agent process management endpoints.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use roko_runtime::process::ProcessId;

use crate::error::ApiError;
use crate::routes::run::spawn_background_run;
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/agents", get(list_agents))
        .route("/agents/{id}", get(get_agent))
        .route("/agents/{id}/stop", post(stop_agent))
        .route("/agents/{id}/episodes", get(agent_episodes))
        .route("/agents/{id}/message", post(send_message))
}

/// `GET /api/agents` — list all managed agent processes.
async fn list_agents(State(state): State<Arc<AppState>>) -> Json<Value> {
    let entries = state.supervisor.list().await;
    let items: Vec<Value> = entries
        .into_iter()
        .map(|(id, label)| {
            json!({
                "id": id.0,
                "label": label,
            })
        })
        .collect();
    Json(Value::Array(items))
}

/// `GET /api/agents/:id` — get info about a specific agent process.
async fn get_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u64>,
) -> Result<Json<Value>, ApiError> {
    let entries = state.supervisor.list().await;
    let found = entries.into_iter().find(|(pid, _)| pid.0 == id);

    match found {
        Some((pid, label)) => Ok(Json(json!({
            "id": pid.0,
            "label": label,
        }))),
        None => Err(ApiError::not_found(format!("agent {id} not found"))),
    }
}

/// `POST /api/agents/:id/stop` — shut down a specific agent process.
async fn stop_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u64>,
) -> Result<Json<Value>, ApiError> {
    state.supervisor.shutdown(ProcessId(id)).await.map_or_else(
        || Err(ApiError::not_found(format!("agent {id} not found"))),
        |o| {
            Ok(Json(json!({
                "id": id,
                "outcome": format!("{o:?}"),
            })))
        },
    )
}

/// `GET /api/agents/:id/episodes` — filter episodes for a specific agent.
async fn agent_episodes(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u64>,
) -> Result<Json<Value>, ApiError> {
    let path = state.layout.episodes_path();
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Json(json!([]))),
        Err(e) => {
            return Err(ApiError::internal(format!("read episodes: {e}")));
        }
    };

    let agent_id_str = id.to_string();
    let mut filtered: Vec<Value> = Vec::new();
    for (line_no, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str::<Value>(line)
            .map_err(|e| ApiError::internal(format!("parse episodes line {}: {e}", line_no + 1)))?;
        if value
            .get("agent_id")
            .is_some_and(|a| a.as_str() == Some(&agent_id_str) || a.as_u64() == Some(id))
        {
            filtered.push(value);
        }
    }

    Ok(Json(Value::Array(filtered)))
}

#[derive(Debug, Deserialize)]
struct SendMessageRequest {
    #[serde(alias = "content")]
    message: String,
    #[serde(default)]
    context: Option<Value>,
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    response_mode: Option<String>,
}

/// `POST /api/agents/{id}/message` — send a message to a logical agent.
async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.message.trim().is_empty() {
        return Err(ApiError::bad_request("message must not be empty"));
    }

    let prompt = build_agent_prompt(&agent_id, &req.message, req.context.as_ref());
    let run_id = spawn_background_run(&state, prompt, None, Some(agent_id.clone())).await;

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({
            "run_id": run_id,
            "agent_id": agent_id,
            "conversation_id": req.conversation_id,
            "response_mode": req.response_mode,
            "status": "running",
        })),
    ))
}

fn build_agent_prompt(agent_id: &str, message: &str, context: Option<&Value>) -> String {
    let mut prompt = format!("[agent:{agent_id}] {message}");
    if let Some(context) = context {
        prompt.push_str("\n\nContext:\n");
        prompt.push_str(&serde_json::to_string_pretty(context).unwrap_or_else(|_| context.to_string()));
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::to_bytes;
    use axum::response::IntoResponse;
    use roko_core::config::schema::RokoConfig;
    use tempfile::tempdir;

    use crate::deploy::manual::ManualBackend;
    use crate::events::ServerEvent;
    use crate::runtime::NoOpRuntime;
    use crate::state::AppState;

    #[test]
    fn build_agent_prompt_embeds_context() {
        let prompt = build_agent_prompt(
            "agent-7",
            "hello",
            Some(&json!({ "task": "research", "priority": "high" })),
        );

        assert!(prompt.starts_with("[agent:agent-7] hello"));
        assert!(prompt.contains("\"task\": \"research\""));
    }

    #[tokio::test]
    async fn send_message_creates_tracked_run_and_events() {
        let tempdir = tempdir().expect("tempdir");
        let state = Arc::new(AppState::new(
            tempdir.path().to_path_buf(),
            Arc::new(NoOpRuntime),
            RokoConfig::default(),
            Arc::new(ManualBackend::default()),
        ));

        let response = send_message(
            State(Arc::clone(&state)),
            Path("agent-1".to_string()),
            Json(SendMessageRequest {
                message: "hello".into(),
                context: Some(json!({ "source": "dashboard" })),
                conversation_id: Some("conv-1".into()),
                response_mode: Some("stream".into()),
            }),
        )
        .await
        .expect("send message")
        .into_response();

        assert_eq!(response.status(), axum::http::StatusCode::ACCEPTED);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let payload: Value = serde_json::from_slice(&body).expect("json body");
        let run_id = payload["run_id"].as_str().expect("run_id").to_string();

        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(state.active_runs.read().await.contains_key(&run_id));
        let events = state.event_bus.replay_from(0);
        assert!(events.iter().any(|event| matches!(
            &event.payload,
            ServerEvent::RunStarted { run_id: event_run_id, .. } if event_run_id == &run_id
        )));
        assert!(events.iter().any(|event| matches!(
            &event.payload,
            ServerEvent::AgentOutput {
                agent_id,
                run_id: Some(event_run_id),
                done: true,
                ..
            } if agent_id == "agent-1" && event_run_id == &run_id
        )));
    }
}
