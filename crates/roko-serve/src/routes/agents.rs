//! Agent registration, token, and process management endpoints.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::Query;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use validator::Validate;

use roko_runtime::process::ProcessId;

use crate::error::ApiError;
use crate::extract::{RequestPayload, ValidJson, validate_with_validator};
use crate::routes::run::spawn_background_run;
use crate::state::{AgentRegistrationRecord, AppState};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/managed-agents", get(list_managed_agents))
        .route("/agents/register", post(register_agent))
        .route("/agents/{id}", get(get_agent))
        .route("/agents/{id}/stop", post(stop_agent))
        .route("/agents/{id}/episodes", get(agent_episodes))
        .route("/agents/{id}/logs", get(proxy_agent_logs))
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
    ValidJson(req): ValidJson<RegisterAgentRequest>,
) -> Result<Json<Value>, ApiError> {
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

#[derive(Debug, Deserialize, Default)]
struct LogsQuery {
    #[serde(default)]
    tail: Option<usize>,
}

/// `GET /api/agents/{id}/logs` — proxy agent-sidecar logs, preserving upstream status.
async fn proxy_agent_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<Response, ApiError> {
    let agent = state
        .discovered_agent(&id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("agent {id} not found")))?;
    let rest = agent
        .endpoints
        .rest
        .ok_or_else(|| ApiError::bad_request(format!("agent {id} has no rest endpoint")))?;

    let url = format!("{}/logs", rest.trim_end_matches('/'));
    let mut request = state.http_client.get(url);
    if let Some(tail) = query.tail {
        request = request.query(&[("tail", tail)]);
    }
    if let Some(token) = agent.proxy_token.as_ref() {
        request = request.bearer_auth(token);
    }

    let response = request
        .send()
        .await
        .map_err(|error| ApiError::internal(format!("proxy to agent logs failed: {error}")))?;
    let status = response.status();
    let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
    let body = response
        .bytes()
        .await
        .map_err(|error| ApiError::internal(format!("read proxied agent logs failed: {error}")))?;

    let mut builder = Response::builder().status(status);
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }

    builder.body(Body::from(body)).map_err(|error| {
        ApiError::internal(format!("build proxied agent logs response failed: {error}"))
    })
}

#[derive(Debug, Deserialize, Validate)]
struct SendMessageRequest {
    #[serde(alias = "content")]
    #[validate(
        length(min = 1),
        custom(function = "crate::extract::validate_non_blank")
    )]
    message: String,
    #[serde(default)]
    context: Option<Value>,
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    response_mode: Option<String>,
}

impl RequestPayload for SendMessageRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        validate_with_validator(self)
    }
}

/// `POST /api/agents/{id}/message` — send a message to a registered agent or fall back to a run.
async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    ValidJson(req): ValidJson<SendMessageRequest>,
) -> Result<impl IntoResponse, ApiError> {
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

#[derive(Debug, Deserialize, Validate)]
struct RegisterAgentRequest {
    #[validate(
        length(min = 1, max = 128),
        custom(function = "crate::extract::validate_non_blank")
    )]
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

impl RequestPayload for RegisterAgentRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        validate_with_validator(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::error::Error;
    use std::sync::Arc;
    use std::time::Duration;

    use anyhow::{Result, anyhow};
    use axum::body::Body;
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use roko_core::config::schema::RokoConfig;
    use tempfile::tempdir;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;
    use tower::ServiceExt;

    use crate::deploy::manual::ManualBackend;
    use crate::events::ServerEvent;
    use crate::runtime::NoOpRuntime;
    use crate::state::{AgentEndpoints, AgentRegistrationRecord, AppState};

    #[derive(Debug)]
    struct MockLogsServerState {
        response: Value,
        status: StatusCode,
        seen_tails: Mutex<Vec<Option<usize>>>,
    }

    async fn mock_logs_handler(
        State(state): State<Arc<MockLogsServerState>>,
        Query(query): Query<LogsQuery>,
    ) -> impl IntoResponse {
        state.seen_tails.lock().await.push(query.tail);
        (state.status, Json(state.response.clone()))
    }

    async fn spawn_mock_logs_server(
        status: StatusCode,
        response: Value,
    ) -> Result<(
        String,
        Arc<MockLogsServerState>,
        tokio::task::JoinHandle<()>,
    )> {
        let state = Arc::new(MockLogsServerState {
            response,
            status,
            seen_tails: Mutex::new(Vec::new()),
        });
        let router = Router::new()
            .route("/logs", get(mock_logs_handler))
            .with_state(Arc::clone(&state));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| anyhow!("failed to bind mock logs listener: {error}"))?;
        let addr = listener
            .local_addr()
            .map_err(|error| anyhow!("failed to read mock logs address: {error}"))?;
        let handle = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, router).await {
                panic!("mock logs server stopped unexpectedly: {error}");
            }
        });
        Ok((format!("http://{addr}"), state, handle))
    }

    fn router(state: Arc<AppState>) -> Router {
        Router::new().nest("/api", routes()).with_state(state)
    }

    async fn json_body(response: axum::response::Response) -> Result<Value> {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .map_err(|error| anyhow!("failed to read response body bytes: {error}"))?;
        serde_json::from_slice(&bytes)
            .map_err(|error| anyhow!("failed to parse JSON response body: {error}"))
    }

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
            ValidJson(SendMessageRequest {
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
            ValidJson(RegisterAgentRequest {
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

    #[tokio::test(flavor = "multi_thread")]
    async fn agent_logs_proxy_forwards_tail_and_body() -> std::result::Result<(), Box<dyn Error>> {
        let tempdir = tempdir().expect("tempdir");
        let state = Arc::new(AppState::new(
            tempdir.path().to_path_buf(),
            Arc::new(NoOpRuntime),
            RokoConfig::default(),
            Arc::new(ManualBackend::default()),
        ));
        let (logs_url, logs_state, _handle) = spawn_mock_logs_server(
            StatusCode::OK,
            json!({
                "lines": ["alpha", "bravo"],
                "path": "/tmp/agent.log",
            }),
        )
        .await?;
        state
            .upsert_discovered_agent(AgentRegistrationRecord {
                agent_id: "agent-logs".into(),
                label: Some("agent-logs".into()),
                process_id: None,
                owner: String::new(),
                endpoints: AgentEndpoints {
                    rest: Some(logs_url),
                    websocket: None,
                    a2a: None,
                    mcp: None,
                },
                card_uri: None,
                capabilities: Vec::new(),
                domain_tags: Vec::new(),
            })
            .await;

        let response = router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/agents/agent-logs/logs?tail=2")
                    .body(Body::empty())
                    .map_err(|error| anyhow!("failed to build agent logs request: {error}"))?,
            )
            .await
            .map_err(|error| anyhow!("agent logs request failed: {error}"))?;

        assert_eq!(response.status(), StatusCode::OK);
        let payload = json_body(response).await?;
        assert_eq!(payload["lines"], json!(["alpha", "bravo"]));
        assert_eq!(payload["path"], "/tmp/agent.log");

        let seen_tails = logs_state.seen_tails.lock().await;
        assert_eq!(seen_tails.as_slice(), &[Some(2)]);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn agent_logs_missing_agent_returns_404() -> std::result::Result<(), Box<dyn Error>> {
        let tempdir = tempdir().expect("tempdir");
        let state = Arc::new(AppState::new(
            tempdir.path().to_path_buf(),
            Arc::new(NoOpRuntime),
            RokoConfig::default(),
            Arc::new(ManualBackend::default()),
        ));

        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/agents/missing/logs")
                    .body(Body::empty())
                    .map_err(|error| {
                        anyhow!("failed to build missing-agent logs request: {error}")
                    })?,
            )
            .await
            .map_err(|error| anyhow!("missing-agent logs request failed: {error}"))?;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let payload = json_body(response).await?;
        assert_eq!(payload["code"], "not_found");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn agent_logs_sidecar_not_found_is_propagated() -> std::result::Result<(), Box<dyn Error>>
    {
        let tempdir = tempdir().expect("tempdir");
        let state = Arc::new(AppState::new(
            tempdir.path().to_path_buf(),
            Arc::new(NoOpRuntime),
            RokoConfig::default(),
            Arc::new(ManualBackend::default()),
        ));
        let (logs_url, logs_state, _handle) = spawn_mock_logs_server(
            StatusCode::NOT_FOUND,
            json!({ "error": "log file missing" }),
        )
        .await?;
        state
            .upsert_discovered_agent(AgentRegistrationRecord {
                agent_id: "agent-404".into(),
                label: Some("agent-404".into()),
                process_id: None,
                owner: String::new(),
                endpoints: AgentEndpoints {
                    rest: Some(logs_url),
                    websocket: None,
                    a2a: None,
                    mcp: None,
                },
                card_uri: None,
                capabilities: Vec::new(),
                domain_tags: Vec::new(),
            })
            .await;

        let response = router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/agents/agent-404/logs?tail=9")
                    .body(Body::empty())
                    .map_err(|error| {
                        anyhow!("failed to build sidecar-404 logs request: {error}")
                    })?,
            )
            .await
            .map_err(|error| anyhow!("sidecar-404 logs request failed: {error}"))?;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let payload = json_body(response).await?;
        assert_eq!(payload["error"], "log file missing");

        let seen_tails = logs_state.seen_tails.lock().await;
        assert_eq!(seen_tails.as_slice(), &[Some(9)]);
        Ok(())
    }
}
