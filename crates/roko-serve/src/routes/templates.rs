//! Template CRUD and deploy (run-from-template) endpoints.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::state::{AppState, OperationHandle, OperationStatus};
use crate::templates::AgentTemplate;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/templates", get(list_templates).post(create_template))
        .route(
            "/templates/{name}",
            get(get_template).delete(delete_template),
        )
        .route("/templates/{name}/deploy", post(deploy_template))
}

/// `GET /api/templates` — list all templates from the registry.
async fn list_templates(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let registry = state.templates.read().await;
    let items: Vec<Value> = registry
        .list()
        .into_iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "model": t.model,
                "role": t.role,
                "output_format": t.output_format,
            })
        })
        .collect();
    drop(registry);
    Ok(Json(Value::Array(items)))
}

/// `POST /api/templates` — create a new template.
async fn create_template(
    State(state): State<Arc<AppState>>,
    Json(template): Json<AgentTemplate>,
) -> Result<impl IntoResponse, ApiError> {
    let name = template.name.clone();

    {
        let registry = state.templates.read().await;
        if registry.get(&name).is_some() {
            return Err(ApiError::conflict(format!(
                "template '{name}' already exists"
            )));
        }
    }

    let mut registry = state.templates.write().await;
    registry
        .insert(template)
        .map_err(|e| ApiError::internal(format!("insert template: {e}")))?;
    drop(registry);

    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({ "name": name })),
    ))
}

/// `GET /api/templates/:name` — get a specific template.
async fn get_template(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let registry = state.templates.read().await;
    let template = registry
        .get(&name)
        .ok_or_else(|| ApiError::not_found(format!("template '{name}' not found")))?;
    let result = Json(
        serde_json::to_value(template)
            .map_err(|e| ApiError::internal(format!("serialize template: {e}")))?,
    );
    drop(registry);
    Ok(result)
}

/// `DELETE /api/templates/:name` — remove a template.
async fn delete_template(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let mut registry = state.templates.write().await;
    registry
        .remove(&name)
        .map_err(|e| ApiError::internal(format!("remove template: {e}")))?;
    drop(registry);
    Ok(Json(json!({ "removed": name })))
}

#[derive(Deserialize)]
struct DeployRequest {
    #[serde(default)]
    params: std::collections::HashMap<String, String>,
    /// When set, deploy as a cloud worker via the named backend
    /// ("railway-api", "railway-cli", "manual") instead of running in-process.
    #[serde(default)]
    backend: Option<String>,
}

/// `POST /api/templates/:name/deploy` — render a template and run it (or deploy it).
///
/// If `backend` is set in the request, this creates a cloud deployment via
/// `POST /api/deployments`. Otherwise it runs the template in-process as before.
async fn deploy_template(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<DeployRequest>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    // If a backend is specified, delegate to the deployments endpoint logic
    if body.backend.is_some() {
        return deploy_template_cloud(state, name, body).await;
    }

    // ---- In-process run (existing behavior) ----
    let prompt = {
        let registry = state.templates.read().await;
        let template = registry
            .get(&name)
            .ok_or_else(|| ApiError::not_found(format!("template '{name}' not found")))?;
        let rendered = crate::templates::TemplateRegistry::render_prompt(template, &body.params);
        drop(registry);
        rendered
    };

    let op_id = uuid::Uuid::new_v4().to_string();
    let workdir = state.workdir.clone();
    let bus = state.event_bus.clone();
    let runtime = state.runtime.clone();

    let handle = tokio::spawn({
        let op_id = op_id.clone();
        let template_name = name.clone();
        let state = Arc::clone(&state);
        async move {
            match runtime.run_once(&workdir, &prompt).await {
                Ok(result) => {
                    state
                        .template_runs
                        .write()
                        .await
                        .entry(template_name.clone())
                        .or_default()
                        .push(crate::state::TemplateRunRecord {
                            timestamp: chrono::Utc::now(),
                            trigger_kind: "template_deploy".into(),
                            success: result.success,
                        });
                    bus.publish(ServerEvent::OperationCompleted {
                        op_id,
                        kind: "template_deploy".into(),
                        success: result.success,
                    });
                }
                Err(e) => {
                    state
                        .template_runs
                        .write()
                        .await
                        .entry(template_name.clone())
                        .or_default()
                        .push(crate::state::TemplateRunRecord {
                            timestamp: chrono::Utc::now(),
                            trigger_kind: "template_deploy".into(),
                            success: false,
                        });
                    bus.publish(ServerEvent::Error {
                        message: format!("template deploy failed: {e}"),
                    });
                    bus.publish(ServerEvent::OperationCompleted {
                        op_id,
                        kind: "template_deploy".into(),
                        success: false,
                    });
                }
            }
        }
    });

    let op = OperationHandle {
        id: op_id.clone(),
        kind: format!("template_deploy:{name}"),
        status: OperationStatus::Running,
        handle,
    };

    state.operations.write().await.insert(op_id.clone(), op);

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "id": op_id })),
    ))
}

/// Cloud deployment path: create a Railway/manual deployment for this template.
async fn deploy_template_cloud(
    state: Arc<AppState>,
    name: String,
    body: DeployRequest,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    use crate::deploy::DeploySpec;
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as BASE64;

    // Look up and render template
    let templates = state.templates.read().await;
    let template = templates
        .get(&name)
        .ok_or_else(|| ApiError::not_found(format!("template '{name}' not found")))?
        .clone();
    drop(templates);

    let rendered_prompt =
        crate::templates::TemplateRegistry::render_prompt(&template, &body.params);
    let mut deploy_template = template.clone();
    deploy_template.system_prompt = rendered_prompt;

    let template_json = serde_json::to_vec(&deploy_template)
        .map_err(|e| ApiError::internal(format!("serialize template: {e}")))?;
    let template_b64 = BASE64.encode(&template_json);

    // Build env vars
    let mut env_vars = std::collections::HashMap::new();
    env_vars.insert("ROKO_TEMPLATE_JSON".to_string(), template_b64);
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        env_vars.insert("ANTHROPIC_API_KEY".to_string(), key);
    }

    let roko_config = state.roko_config.read().await;
    let image = roko_config
        .deploy
        .worker_image
        .clone()
        .unwrap_or_else(|| "roko-worker:latest".to_string());
    let region = roko_config.deploy.default_region.clone();
    drop(roko_config);

    let spec = DeploySpec {
        name: format!("roko-worker-{name}"),
        image,
        env_vars,
        region,
    };

    let deployment = state
        .deploy_backend
        .deploy(&spec)
        .await
        .map_err(|e| ApiError::internal(format!("cloud deploy failed: {e}")))?;

    let dep_id = deployment.id.clone();

    state
        .deployments
        .write()
        .await
        .insert(dep_id.clone(), deployment);

    state.event_bus.publish(ServerEvent::DeploymentCreated {
        id: dep_id.clone(),
        name: format!("roko-worker-{name}"),
    });

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "id": dep_id, "type": "cloud_deployment" })),
    ))
}
