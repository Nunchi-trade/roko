//! Cloud deployment CRUD, task proxy, and callback endpoints.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{error, info};

use crate::deploy::{DeploySpec, DeploymentStatus};
use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::state::AppState;
use crate::templates::TemplateRegistry;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/deployments",
            post(create_deployment).get(list_deployments),
        )
        .route(
            "/deployments/{id}",
            get(get_deployment).delete(teardown_deployment),
        )
        .route("/deployments/{id}/logs", get(get_logs))
        .route("/deployments/{id}/task", post(proxy_task))
        .route("/deployments/{id}/callback", post(receive_callback))
}

// ---- Request/Response types ------------------------------------------------

#[derive(Deserialize)]
struct CreateDeploymentRequest {
    /// Template name to deploy.
    template: String,
    /// Parameters to interpolate into the template.
    #[serde(default)]
    params: HashMap<String, String>,
    /// Override the deploy backend (e.g. "railway-api", "railway-cli", "manual").
    /// If omitted, uses the default from config.
    #[serde(default)]
    backend: Option<String>,
}

#[derive(Deserialize)]
struct LogsQuery {
    #[serde(default = "default_tail")]
    tail: usize,
}

const fn default_tail() -> usize {
    100
}

// ---- Handlers --------------------------------------------------------------

/// `POST /api/deployments` — create a new cloud deployment from a template.
#[allow(clippy::too_many_lines)]
async fn create_deployment(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateDeploymentRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // Look up template
    let templates = state.templates.read().await;
    let template = templates
        .get(&req.template)
        .ok_or_else(|| ApiError::not_found(format!("template '{}' not found", req.template)))?
        .clone();
    drop(templates);

    // Render prompt with params and encode template as base64 JSON
    let rendered_prompt = TemplateRegistry::render_prompt(&template, &req.params);
    let mut deploy_template = template.clone();
    deploy_template.system_prompt = rendered_prompt;

    let template_json = serde_json::to_vec(&deploy_template)
        .map_err(|e| ApiError::internal(format!("serialize template: {e}")))?;
    let template_b64 = BASE64.encode(&template_json);

    // Build env vars
    let mut env_vars = HashMap::new();
    env_vars.insert("ROKO_TEMPLATE_JSON".to_string(), template_b64);

    // Pass through ANTHROPIC_API_KEY if set
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        env_vars.insert("ANTHROPIC_API_KEY".to_string(), key);
    }

    // Read all config values we need from one config snapshot.
    let (control_url, image, region) = {
        let rc = state.load_roko_config();
        let url = format!("http://{}:{}", rc.server.bind, rc.server.port);
        let img = rc
            .deploy
            .worker_image
            .clone()
            .unwrap_or_else(|| "roko-worker:latest".to_string());
        let rgn = rc.deploy.default_region.clone();
        (url, img, rgn)
    };
    env_vars.insert("ROKO_CONTROL_PLANE_URL".to_string(), control_url);

    let service_name = format!("roko-worker-{}", req.template);

    let spec = DeploySpec {
        name: service_name.clone(),
        image,
        env_vars: env_vars.clone(),
        region,
    };

    // Create per-request backend if overridden, otherwise use the server default
    let backend: Arc<dyn crate::deploy::DeployBackend> = if let Some(ref name) = req.backend {
        let rc = state.load_roko_config();
        let b = crate::deploy::create_backend(
            name,
            rc.deploy.railway_api_token.as_deref(),
            rc.deploy.project_id.as_deref(),
            rc.deploy.environment_id.as_deref(),
        )
        .map_err(|e| ApiError::bad_request(format!("invalid backend '{name}': {e}")))?;
        Arc::from(b)
    } else {
        Arc::clone(&state.deploy_backend)
    };

    // Deploy
    let deployment = backend
        .deploy(&spec)
        .await
        .map_err(|e| ApiError::internal(format!("deploy failed: {e}")))?;

    // Set the deployment ID in env for callbacks
    env_vars.insert("ROKO_DEPLOYMENT_ID".to_string(), deployment.id.clone());

    let dep_id = deployment.id.clone();
    let dep_name = deployment.name.clone();

    // Store deployment
    state
        .deployments
        .write()
        .await
        .insert(dep_id.clone(), deployment.clone());

    // Emit event
    state.event_bus.publish(ServerEvent::DeploymentCreated {
        id: dep_id.clone(),
        name: dep_name,
    });

    // Spawn background task to poll status until terminal
    let backend = Arc::clone(&backend);
    let event_bus = state.event_bus.clone();
    let state_for_poll = Arc::clone(&state);
    let poll_id = dep_id.clone();

    tokio::spawn(async move {
        let mut interval_ms: u64 = 5_000;
        let max_interval_ms: u64 = 60_000;

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;

            match backend.status(&poll_id).await {
                Ok(status) => {
                    let is_terminal = status.is_terminal();

                    // Update stored deployment
                    let mut deps = state_for_poll.deployments.write().await;
                    if let Some(dep) = deps.get_mut(&poll_id) {
                        if let DeploymentStatus::Ready { ref url } = status {
                            dep.url = Some(url.clone());
                        }
                        dep.status = status.clone();
                    }
                    drop(deps);

                    match &status {
                        DeploymentStatus::Ready { url } => {
                            info!(%poll_id, %url, "deployment ready");
                            event_bus.publish(ServerEvent::DeploymentReady {
                                id: poll_id.clone(),
                                url: url.clone(),
                            });
                        }
                        DeploymentStatus::Failed { reason } => {
                            error!(%poll_id, %reason, "deployment failed");
                            event_bus.publish(ServerEvent::DeploymentFailed {
                                id: poll_id.clone(),
                                reason: reason.clone(),
                            });
                        }
                        _ => {}
                    }

                    if is_terminal {
                        break;
                    }
                }
                Err(e) => {
                    error!(%poll_id, error = %e, "status poll failed");
                }
            }

            // Exponential backoff
            interval_ms = (interval_ms * 2).min(max_interval_ms);
        }
    });

    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({
            "id": dep_id,
            "name": service_name,
            "status": "creating",
        })),
    ))
}

/// `GET /api/deployments` — list all deployments.
async fn list_deployments(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, ApiError> {
    let deps = state.deployments.read().await;
    let list: Vec<Value> = deps
        .values()
        .map(|d| {
            json!({
                "id": d.id,
                "name": d.name,
                "status": d.status,
                "url": d.url,
                "created_at": d.created_at.to_rfc3339(),
            })
        })
        .collect();
    drop(deps);
    Ok(Json(json!({ "deployments": list })))
}

/// `GET /api/deployments/:id` — get deployment details.
async fn get_deployment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let deps = state.deployments.read().await;
    let dep = deps
        .get(&id)
        .ok_or_else(|| ApiError::not_found("deployment not found"))?;
    let result = json!({
        "id": dep.id,
        "name": dep.name,
        "status": dep.status,
        "url": dep.url,
        "created_at": dep.created_at.to_rfc3339(),
    });
    drop(deps);
    Ok(Json(result))
}

/// `DELETE /api/deployments/:id` — tear down a deployment.
async fn teardown_deployment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    // Verify it exists
    {
        let deps = state.deployments.read().await;
        if !deps.contains_key(&id) {
            return Err(ApiError::not_found("deployment not found"));
        }
    }

    // Teardown via backend
    state
        .deploy_backend
        .teardown(&id)
        .await
        .map_err(|e| ApiError::internal(format!("teardown failed: {e}")))?;

    // Update status
    {
        let mut deps = state.deployments.write().await;
        if let Some(dep) = deps.get_mut(&id) {
            dep.status = DeploymentStatus::TornDown;
        }
    }

    state
        .event_bus
        .publish(ServerEvent::DeploymentTornDown { id: id.clone() });

    Ok(Json(json!({ "id": id, "status": "torn_down" })))
}

/// `GET /api/deployments/:id/logs` — fetch deployment logs.
async fn get_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    {
        let deps = state.deployments.read().await;
        if !deps.contains_key(&id) {
            return Err(ApiError::not_found("deployment not found"));
        }
    }

    let logs = state
        .deploy_backend
        .logs(&id, query.tail)
        .await
        .map_err(|e| ApiError::internal(format!("logs failed: {e}")))?;

    Ok(Json(json!({ "id": id, "logs": logs })))
}

/// `POST /api/deployments/:id/task` — proxy a task to the deployed worker.
async fn proxy_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    // Get the deployment URL
    let worker_url = {
        let deps = state.deployments.read().await;
        let dep = deps
            .get(&id)
            .ok_or_else(|| ApiError::not_found("deployment not found"))?;
        let url = dep
            .url
            .as_ref()
            .ok_or_else(|| ApiError::bad_request("deployment is not ready yet (no URL)"))?
            .clone();
        drop(deps);
        format!("{url}/task")
    };

    let task_id = uuid::Uuid::new_v4().to_string();

    state.event_bus.publish(ServerEvent::WorkerTaskStarted {
        deployment_id: id.clone(),
        task_id: task_id.clone(),
    });
    let client = reqwest::Client::new();
    let resp = client
        .post(&worker_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("proxy to worker failed: {e}")))?;

    let status_code = resp.status();
    let resp_body: Value = resp
        .json()
        .await
        .unwrap_or_else(|_| json!({"error": "failed to parse worker response"}));

    let success = status_code.is_success();
    state.event_bus.publish(ServerEvent::WorkerTaskCompleted {
        deployment_id: id,
        task_id,
        success,
    });

    Ok(Json(resp_body))
}

/// Infer the backing template name for a deployment, if it follows the worker naming convention.
async fn template_name_for_deployment(deployment_id: &str, state: &AppState) -> Option<String> {
    let deps = state.deployments.read().await;
    let deployment = deps.get(deployment_id)?;
    deployment
        .name
        .strip_prefix("roko-worker-")
        .map(|name| name.to_string())
        .or_else(|| Some(deployment.name.clone()))
}

/// `POST /api/deployments/:id/callback` — receive results from a worker callback.
async fn receive_callback(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let success = body
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    info!(%id, %success, "received worker callback");

    if let Some(template_name) = template_name_for_deployment(&id, &state).await {
        state
            .template_runs
            .write()
            .await
            .entry(template_name)
            .or_default()
            .push(crate::state::TemplateRunRecord {
                timestamp: chrono::Utc::now(),
                trigger_kind: "worker_callback".into(),
                success,
            });
    }

    state.event_bus.publish(ServerEvent::WorkerTaskCompleted {
        deployment_id: id,
        task_id: body
            .get("task_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        success,
    });

    Ok(Json(json!({ "received": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::error::Error;

    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use tempfile::{TempDir, tempdir};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;
    use tower::ServiceExt;

    use crate::deploy::{DeployBackend, Deployment};
    use crate::runtime::NoOpRuntime;
    use crate::templates::{AgentTemplate, TemplateOutputFormat};

    struct RecordingDeployBackend {
        next_url: Option<String>,
        deployed_specs: Mutex<Vec<DeploySpec>>,
    }

    impl RecordingDeployBackend {
        fn with_url(next_url: Option<String>) -> Self {
            Self {
                next_url,
                deployed_specs: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl DeployBackend for RecordingDeployBackend {
        async fn deploy(&self, spec: &DeploySpec) -> Result<Deployment> {
            self.deployed_specs.lock().await.push(spec.clone());

            Ok(Deployment {
                id: "dep-1".to_string(),
                name: spec.name.clone(),
                status: DeploymentStatus::Ready {
                    url: self
                        .next_url
                        .clone()
                        .unwrap_or_else(|| "http://worker.invalid".to_string()),
                },
                url: self.next_url.clone(),
                created_at: chrono::Utc::now(),
            })
        }

        async fn status(&self, deployment_id: &str) -> Result<DeploymentStatus> {
            Ok(DeploymentStatus::Ready {
                url: self
                    .next_url
                    .clone()
                    .unwrap_or_else(|| format!("http://worker.invalid/{deployment_id}")),
            })
        }

        async fn teardown(&self, _deployment_id: &str) -> Result<()> {
            Ok(())
        }

        async fn logs(&self, deployment_id: &str, tail: usize) -> Result<Vec<String>> {
            Ok(vec![format!("log line for {deployment_id} tail={tail}")])
        }
    }

    async fn record_task(
        State(recorded): State<Arc<Mutex<Vec<Value>>>>,
        Json(payload): Json<Value>,
    ) -> Json<Value> {
        recorded.lock().await.push(payload.clone());
        Json(json!({
            "forwarded": true,
            "received": payload,
        }))
    }

    async fn spawn_mock_worker_server()
    -> Result<(String, Arc<Mutex<Vec<Value>>>, tokio::task::JoinHandle<()>)> {
        let recorded = Arc::new(Mutex::new(Vec::<Value>::new()));
        let router = Router::new()
            .route("/task", post(record_task))
            .with_state(Arc::clone(&recorded));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|err| anyhow!("failed to bind mock worker listener: {err}"))?;
        let addr = listener
            .local_addr()
            .map_err(|err| anyhow!("failed to read mock worker address: {err}"))?;
        let handle = tokio::spawn(async move {
            if let Err(err) = axum::serve(listener, router).await {
                panic!("mock worker server stopped unexpectedly: {err}");
            }
        });
        Ok((format!("http://{addr}"), recorded, handle))
    }

    fn test_template(name: &str, prompt: &str) -> AgentTemplate {
        AgentTemplate {
            name: name.to_string(),
            description: format!("template for {name}"),
            model: "claude-sonnet-4-5".to_string(),
            role: "implementer".to_string(),
            system_prompt: prompt.to_string(),
            max_turns: 3,
            output_format: TemplateOutputFormat::Markdown,
            mcp_servers: Vec::new(),
            allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
            experiment: None,
        }
    }

    fn test_state(backend: Arc<dyn DeployBackend>) -> Result<(TempDir, Arc<AppState>)> {
        let dir = tempdir().map_err(|err| anyhow!("failed to create tempdir: {err}"))?;
        let state = Arc::new(AppState::new(
            dir.path().to_path_buf(),
            Arc::new(NoOpRuntime),
            roko_core::config::schema::RokoConfig::default(),
            backend,
        ));
        Ok((dir, state))
    }

    async fn insert_template(state: &Arc<AppState>, template: AgentTemplate) -> Result<()> {
        state
            .templates
            .write()
            .await
            .insert(template)
            .map_err(|err| anyhow!("failed to insert template into registry: {err}"))?;
        Ok(())
    }

    fn router(state: Arc<AppState>) -> Router {
        Router::new().nest("/api", routes()).with_state(state)
    }

    async fn json_body(response: axum::response::Response) -> Result<Value> {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .map_err(|err| anyhow!("failed to read response body bytes: {err}"))?;
        serde_json::from_slice(&bytes)
            .map_err(|err| anyhow!("failed to parse JSON response body: {err}"))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deployments_create_returns_id() -> std::result::Result<(), Box<dyn Error>> {
        let backend: Arc<dyn DeployBackend> = Arc::new(RecordingDeployBackend::with_url(Some(
            "http://worker.invalid".to_string(),
        )));
        let (_dir, state) = test_state(backend)?;
        insert_template(&state, test_template("reviewer", "Review {{subject}}")).await?;
        let app = router(Arc::clone(&state));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/deployments")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "template": "reviewer",
                            "params": { "subject": "batch T18" }
                        })
                        .to_string(),
                    ))
                    .map_err(|err| anyhow!("failed to build deployments create request: {err}"))?,
            )
            .await
            .map_err(|err| anyhow!("deployments create request failed: {err}"))?;

        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "POST /api/deployments should return 201 Created for a successful deployment"
        );

        let payload = json_body(response).await?;
        assert_eq!(
            payload["id"], "dep-1",
            "POST /api/deployments should return the created deployment id"
        );
        assert_eq!(
            payload["status"], "creating",
            "POST /api/deployments should report the new deployment as creating"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deployments_list_after_create() -> std::result::Result<(), Box<dyn Error>> {
        let backend: Arc<dyn DeployBackend> = Arc::new(RecordingDeployBackend::with_url(Some(
            "http://worker.invalid".to_string(),
        )));
        let (_dir, state) = test_state(backend)?;
        insert_template(&state, test_template("reviewer", "Review {{subject}}")).await?;
        let app = router(Arc::clone(&state));

        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/deployments")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "template": "reviewer",
                            "params": { "subject": "batch T18" }
                        })
                        .to_string(),
                    ))
                    .map_err(|err| anyhow!("failed to build deployment create request: {err}"))?,
            )
            .await
            .map_err(|err| anyhow!("deployment create request failed: {err}"))?;
        assert_eq!(
            create_response.status(),
            StatusCode::CREATED,
            "setup create request should succeed before listing deployments"
        );

        let list_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/deployments")
                    .body(Body::empty())
                    .map_err(|err| anyhow!("failed to build deployments list request: {err}"))?,
            )
            .await
            .map_err(|err| anyhow!("deployments list request failed: {err}"))?;
        assert_eq!(
            list_response.status(),
            StatusCode::OK,
            "GET /api/deployments should return 200 OK after a deployment is created"
        );

        let payload = json_body(list_response).await?;
        let deployments = payload["deployments"].as_array().ok_or_else(|| {
            anyhow!("deployments list response should contain a deployments array")
        })?;
        assert!(
            deployments
                .iter()
                .any(|deployment| deployment["id"] == "dep-1"),
            "GET /api/deployments should include the id of the deployment created earlier"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deployments_get_logs_missing_returns_404() -> std::result::Result<(), Box<dyn Error>> {
        let backend: Arc<dyn DeployBackend> = Arc::new(RecordingDeployBackend::with_url(Some(
            "http://worker.invalid".to_string(),
        )));
        let (_dir, state) = test_state(backend)?;
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/deployments/unknown/logs")
                    .body(Body::empty())
                    .map_err(|err| anyhow!("failed to build deployment logs request: {err}"))?,
            )
            .await
            .map_err(|err| anyhow!("deployment logs request failed: {err}"))?;

        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "GET /api/deployments/unknown/logs should return 404 for an unknown deployment id"
        );

        let payload = json_body(response).await?;
        assert_eq!(
            payload["error"]["code"], "not_found",
            "missing deployment logs should return the structured not_found error code"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deployments_task_proxy_forwards() -> std::result::Result<(), Box<dyn Error>> {
        let backend: Arc<dyn DeployBackend> = Arc::new(RecordingDeployBackend::with_url(Some(
            "http://worker.invalid".to_string(),
        )));
        let (_dir, state) = test_state(backend)?;
        let (worker_url, recorded, worker_handle) = spawn_mock_worker_server().await?;

        state.deployments.write().await.insert(
            "dep-1".to_string(),
            Deployment {
                id: "dep-1".to_string(),
                name: "roko-worker-reviewer".to_string(),
                status: DeploymentStatus::Ready {
                    url: worker_url.clone(),
                },
                url: Some(worker_url),
                created_at: chrono::Utc::now(),
            },
        );

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/deployments/dep-1/task")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({ "task": "ship tests" }).to_string()))
                    .map_err(|err| anyhow!("failed to build worker proxy request: {err}"))?,
            )
            .await
            .map_err(|err| anyhow!("worker proxy request failed: {err}"))?;

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "POST /api/deployments/:id/task should succeed when the worker endpoint accepts the payload"
        );

        let payload = json_body(response).await?;
        assert_eq!(
            payload["forwarded"], true,
            "POST /api/deployments/:id/task should return the worker response payload"
        );
        assert_eq!(
            payload["received"]["task"], "ship tests",
            "POST /api/deployments/:id/task should forward the original task payload to the worker"
        );

        let forwarded_payloads = recorded.lock().await;
        assert_eq!(
            forwarded_payloads.len(),
            1,
            "mock worker should record exactly one forwarded task request"
        );
        assert_eq!(
            forwarded_payloads[0]["task"], "ship tests",
            "mock worker should record the forwarded task body without mutation"
        );
        drop(forwarded_payloads);
        worker_handle.abort();
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deployments_callback_accepted() -> std::result::Result<(), Box<dyn Error>> {
        let backend: Arc<dyn DeployBackend> = Arc::new(RecordingDeployBackend::with_url(Some(
            "http://worker.invalid".to_string(),
        )));
        let (_dir, state) = test_state(backend)?;
        state.deployments.write().await.insert(
            "dep-1".to_string(),
            Deployment {
                id: "dep-1".to_string(),
                name: "roko-worker-reviewer".to_string(),
                status: DeploymentStatus::Ready {
                    url: "http://worker.invalid".to_string(),
                },
                url: Some("http://worker.invalid".to_string()),
                created_at: chrono::Utc::now(),
            },
        );
        let app = router(Arc::clone(&state));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/deployments/dep-1/callback")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "task_id": "task-123",
                            "success": true
                        })
                        .to_string(),
                    ))
                    .map_err(|err| anyhow!("failed to build deployment callback request: {err}"))?,
            )
            .await
            .map_err(|err| anyhow!("deployment callback request failed: {err}"))?;

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "POST /api/deployments/:id/callback should return 200 OK for a valid worker callback"
        );

        let payload = json_body(response).await?;
        assert_eq!(
            payload["received"], true,
            "POST /api/deployments/:id/callback should acknowledge that the callback was recorded"
        );

        let template_runs = state.template_runs.read().await;
        let reviewer_runs = template_runs.get("reviewer").ok_or_else(|| {
            anyhow!("worker callback should create a template run record for the backing template")
        })?;
        assert_eq!(
            reviewer_runs.len(),
            1,
            "worker callback should record exactly one template run for the deployment template"
        );
        assert_eq!(
            reviewer_runs[0].trigger_kind, "worker_callback",
            "worker callback should tag the recorded template run with trigger_kind=worker_callback"
        );
        assert!(
            reviewer_runs[0].success,
            "worker callback should store the success flag from the callback payload"
        );
        Ok(())
    }
}
