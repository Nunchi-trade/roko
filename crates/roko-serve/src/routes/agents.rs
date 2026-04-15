//! Agent registration, token, and process management endpoints.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use roko_runtime::process::ProcessId;

use crate::error::ApiError;
use crate::routes::run::spawn_background_run;
use crate::state::{AgentRegistrationRecord, AppState};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/managed-agents", get(list_managed_agents))
        .route("/agents/register", post(register_agent))
        .route("/agents/{id}", get(get_agent))
        .route("/agents/{id}/stop", post(stop_agent))
        .route("/agents/{id}/episodes", get(agent_episodes))
        .route("/agents/{id}/message", post(send_message))
        .route("/agents/{id}/token", get(token_status).post(issue_token))
}

/// `GET /api/managed-agents` — list all managed agent processes.
async fn list_managed_agents(State(state): State<Arc<AppState>>) -> Json<Value> {
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

/// `POST /api/agents/register` — upsert a discovery entry for an agent server.
async fn register_agent(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterAgentRequest>,
) -> Result<Json<Value>, ApiError> {
    if req.agent_id.trim().is_empty() {
        return Err(ApiError::bad_request("agent_id must not be empty"));
    }

    let agent = state
        .upsert_discovered_agent(AgentRegistrationRecord {
            agent_id: req.agent_id.clone(),
            label: req.label,
            process_id: req.process_id,
            owner: req.owner.unwrap_or_default(),
            endpoints: crate::state::AgentEndpoints {
                rest: req.rest_endpoint,
                websocket: req.websocket_endpoint,
                a2a: req.a2a_endpoint,
                mcp: req.mcp_endpoint,
            },
            card_uri: req.card_uri,
            capabilities: req.capabilities,
            domain_tags: req.domain_tags,
        })
        .await;

    let token = if req.issue_token.unwrap_or(false) {
        state.rotate_agent_token(&agent.agent_id).await
    } else {
        None
    };

    Ok(Json(json!({
        "agent": agent,
        "token": token,
    })))
}

/// `GET /api/agents/{id}` — get info about a discovered or supervised agent.
async fn get_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    if let Some(agent) = state.discovered_agent(&id).await {
        return Ok(Json(json!(agent)));
    }

    let parsed_id = id
        .parse::<u64>()
        .map_err(|_| ApiError::not_found(format!("agent {id} not found")))?;
    let entries = state.supervisor.list().await;
    let found = entries.into_iter().find(|(pid, _)| pid.0 == parsed_id);

    match found {
        Some((pid, label)) => Ok(Json(json!({
            "id": pid.0,
            "label": label,
        }))),
        None => Err(ApiError::not_found(format!("agent {id} not found"))),
    }
}

/// `POST /api/agents/{id}/stop` — shut down a specific supervised process.
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

/// `GET /api/agents/{id}/episodes` — filter episodes for a specific agent.
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

/// `POST /api/agents/{id}/message` — send a message to a registered agent or fall back to a run.
async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.message.trim().is_empty() {
        return Err(ApiError::bad_request("message must not be empty"));
    }

    if let Some(agent) = state.discovered_agent(&agent_id).await
        && let Some(rest) = agent.endpoints.rest
    {
        let url = format!("{}/message", rest.trim_end_matches('/'));
        let mut request = state.http_client.post(url).json(&json!({
            "prompt": req.message,
            "context": req.context,
        }));

        if let Some(token) = agent.proxy_token {
            request = request.bearer_auth(token);
        }

        match request.send().await {
            Ok(response) => {
                let status = response.status();
                let body = response
                    .json::<Value>()
                    .await
                    .unwrap_or_else(|_| json!({ "status": "proxy_error" }));
                return Ok((
                    StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                    Json(body),
                ));
            }
            Err(error) => {
                tracing::warn!(agent_id, %error, "direct agent message proxy failed, falling back to background run");
            }
        }
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

/// `POST /api/agents/{id}/token` — issue or rotate a bearer token.
async fn issue_token(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let issued = state
        .rotate_agent_token(&id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("agent {id} not found")))?;

    Ok(Json(json!(issued)))
}

/// `GET /api/agents/{id}/token` — check whether a token exists.
async fn token_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let status = state
        .agent_token_status(&id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("agent {id} not found")))?;
    Ok(Json(json!(status)))
}

fn build_agent_prompt(agent_id: &str, message: &str, context: Option<&Value>) -> String {
    let mut prompt = format!("[agent:{agent_id}] {message}");
    if let Some(context) = context {
        prompt.push_str("\n\nContext:\n");
        prompt.push_str(
            &serde_json::to_string_pretty(context).unwrap_or_else(|_| context.to_string()),
        );
    }
    prompt
}

#[derive(Debug, Deserialize)]
struct RegisterAgentRequest {
    agent_id: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    process_id: Option<u64>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    domain_tags: Vec<String>,
    #[serde(default)]
    card_uri: Option<String>,
    #[serde(default)]
    rest_endpoint: Option<String>,
    #[serde(default)]
    websocket_endpoint: Option<String>,
    #[serde(default)]
    a2a_endpoint: Option<String>,
    #[serde(default)]
    mcp_endpoint: Option<String>,
    #[serde(default)]
    issue_token: Option<bool>,
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

    #[tokio::test]
    async fn register_and_issue_token() {
        let tempdir = tempdir().expect("tempdir");
        let state = Arc::new(AppState::new(
            tempdir.path().to_path_buf(),
            Arc::new(NoOpRuntime),
            RokoConfig::default(),
            Arc::new(ManualBackend::default()),
        ));

        let _ = register_agent(
            State(Arc::clone(&state)),
            Json(RegisterAgentRequest {
                agent_id: "agent-2".into(),
                label: Some("agent-two".into()),
                process_id: None,
                owner: Some("owner".into()),
                capabilities: vec!["research".into()],
                domain_tags: vec!["roko".into()],
                card_uri: None,
                rest_endpoint: Some("http://127.0.0.1:9001".into()),
                websocket_endpoint: None,
                a2a_endpoint: None,
                mcp_endpoint: None,
                issue_token: Some(true),
            }),
        )
        .await
        .expect("register");

        let status = state
            .agent_token_status("agent-2")
            .await
            .expect("token status");
        assert!(status.exists);
    }
}
