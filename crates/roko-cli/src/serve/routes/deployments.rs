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

use crate::serve::deploy::{DeploySpec, DeploymentStatus};
use crate::serve::error::ApiError;
use crate::serve::events::ServerEvent;
use crate::serve::state::AppState;
use crate::serve::templates::TemplateRegistry;

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
    deploy_template.prompt.system = rendered_prompt;

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

    // Read all config values we need in one lock acquisition
    let (control_url, image, region) = {
        let rc = state.roko_config.read().await;
        let url = format!("http://{}:{}", rc.server.bind, rc.server.port);
        let img = rc
            .deploy
            .worker_image
            .clone()
            .unwrap_or_else(|| "roko-worker:latest".to_string());
        let rgn = rc.deploy.default_region.clone();
        drop(rc);
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
    let backend: Arc<dyn crate::serve::deploy::DeployBackend> = if let Some(ref name) = req.backend
    {
        let rc = state.roko_config.read().await;
        let b = crate::serve::deploy::create_backend(
            name,
            rc.deploy.railway_api_token.as_deref(),
            rc.deploy.project_id.as_deref(),
            rc.deploy.environment_id.as_deref(),
        )
        .map_err(|e| ApiError::bad_request(format!("invalid backend '{name}': {e}")))?;
        drop(rc);
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
    state.event_bus.emit(ServerEvent::DeploymentCreated {
        id: dep_id.clone(),
        name: dep_name,
    });

    // Spawn background task to poll status until terminal
    let backend = Arc::clone(&backend);
    let event_bus = state.event_bus.sender();
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
                            event_bus.emit(ServerEvent::DeploymentReady {
                                id: poll_id.clone(),
                                url: url.clone(),
                            });
                        }
                        DeploymentStatus::Failed { reason } => {
                            error!(%poll_id, %reason, "deployment failed");
                            event_bus.emit(ServerEvent::DeploymentFailed {
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
        .emit(ServerEvent::DeploymentTornDown { id: id.clone() });

    Ok(Json(json!({ "id": id, "status": "torn_down" })))
}

/// `GET /api/deployments/:id/logs` — fetch deployment logs.
async fn get_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<impl IntoResponse, ApiError> {
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

    state.event_bus.emit(ServerEvent::WorkerTaskStarted {
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
    state.event_bus.emit(ServerEvent::WorkerTaskCompleted {
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
            .push(crate::serve::state::TemplateRunRecord {
                timestamp: chrono::Utc::now(),
                trigger_kind: "worker_callback".into(),
                success,
            });
    }

    state.event_bus.emit(ServerEvent::WorkerTaskCompleted {
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
