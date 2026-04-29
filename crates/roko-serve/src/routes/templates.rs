//! Template CRUD and deploy (run-from-template) endpoints.

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use roko_agent::mcp::find_mcp_config;

use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::extract::{ApiJson, RequestPayload, ValidJson};
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
    ApiJson(template): ApiJson<AgentTemplate>,
) -> Result<impl IntoResponse, ApiError> {
    let name = template.name.clone();
    let configured_mcp_servers = configured_mcp_servers(&state.workdir);

    template
        .validate(None, configured_mcp_servers.as_ref())
        .map_err(|errors| ApiError::bad_request(errors.join("; ")))?;

    // Single write lock to check-and-insert atomically (no TOCTOU race).
    let mut registry = state.templates.write().await;
    if registry.get(&name).is_some() {
        return Err(ApiError::conflict(format!(
            "template '{name}' already exists"
        )));
    }
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
    ValidJson(body): ValidJson<DeployRequest>,
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

impl RequestPayload for DeployRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        if self
            .backend
            .as_deref()
            .is_some_and(|backend| backend.trim().is_empty())
        {
            return Err(ApiError::bad_request("backend must not be blank"));
        }
        Ok(())
    }
}

fn configured_mcp_servers(workdir: &std::path::Path) -> Option<HashSet<String>> {
    match find_mcp_config(workdir) {
        Some(Ok((_path, config))) => Some(
            config
                .servers
                .into_iter()
                .map(|server| server.name)
                .collect(),
        ),
        Some(Err(err)) => {
            tracing::warn!(error = %err, "failed to load MCP config while validating template");
            None
        }
        None => None,
    }
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

    let roko_config = state.load_roko_config();
    let image = roko_config
        .deploy
        .worker_image
        .clone()
        .unwrap_or_else(|| "roko-worker:latest".to_string());
    let region = roko_config.deploy.default_region.clone();

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

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header};
    use tempfile::tempdir;
    use tokio::sync::Notify;
    use tower::ServiceExt;

    use crate::deploy::manual::ManualBackend;
    use crate::runtime::{CliRuntime, DashboardInfo, NoOpRuntime, RunResult, SessionStatusInfo};

    #[tokio::test]
    async fn templates_create_and_get_roundtrip() {
        let state = test_state(Arc::new(NoOpRuntime));
        let router = test_router(state);
        let payload = json!({
            "name": "demo",
            "description": "Demo template",
            "model": "sonnet",
            "role": "implementer",
            "system_prompt": "Hello from demo",
            "max_turns": 3,
            "output_format": "markdown",
            "mcp_servers": [],
            "allowed_tools": [],
            "denied_tools": [],
        });

        let create_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/templates")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(payload.to_string()))
                    .expect("build create request"),
            )
            .await
            .expect("create template response");

        assert_eq!(
            create_response.status(),
            StatusCode::CREATED,
            "creating a valid template should return 201"
        );

        let create_body = to_bytes(create_response.into_body(), usize::MAX)
            .await
            .expect("read create response body");
        let create_json: Value =
            serde_json::from_slice(&create_body).expect("parse create response json");
        assert_eq!(
            create_json["name"], "demo",
            "create response should echo the inserted template name"
        );

        let get_response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/templates/demo")
                    .body(Body::empty())
                    .expect("build get request"),
            )
            .await
            .expect("get template response");

        assert_eq!(
            get_response.status(),
            StatusCode::OK,
            "fetching a created template should return 200"
        );

        let get_body = to_bytes(get_response.into_body(), usize::MAX)
            .await
            .expect("read get response body");
        let template_json: Value =
            serde_json::from_slice(&get_body).expect("parse get response json");
        assert_eq!(
            template_json["name"], "demo",
            "GET /api/templates/demo should return the created template"
        );
        assert_eq!(
            template_json["system_prompt"], "Hello from demo",
            "GET /api/templates/demo should preserve the system prompt"
        );
        assert_eq!(
            template_json["output_format"], "markdown",
            "GET /api/templates/demo should preserve the template output format"
        );
    }

    #[tokio::test]
    async fn templates_interpolate_replaces_placeholders() {
        let runtime = Arc::new(RecordingRuntime::new());
        let state = test_state(Arc::clone(&runtime) as Arc<dyn CliRuntime>);
        let router = test_router(state);

        let create_payload = json!({
            "name": "interpolate",
            "description": "Interpolation demo",
            "role": "implementer",
            "system_prompt": "Investigate {{subject}}",
            "max_turns": 2,
        });

        let create_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/templates")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(create_payload.to_string()))
                    .expect("build interpolation create request"),
            )
            .await
            .expect("create interpolation template response");
        assert_eq!(
            create_response.status(),
            StatusCode::CREATED,
            "template setup for interpolation test should succeed"
        );

        let deploy_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/templates/interpolate/deploy")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "params": {
                                "subject": "codebase"
                            }
                        })
                        .to_string(),
                    ))
                    .expect("build deploy request"),
            )
            .await
            .expect("deploy template response");

        assert_eq!(
            deploy_response.status(),
            StatusCode::ACCEPTED,
            "deploying a template should enqueue work"
        );

        let deploy_body = to_bytes(deploy_response.into_body(), usize::MAX)
            .await
            .expect("read deploy response body");
        let deploy_json: Value =
            serde_json::from_slice(&deploy_body).expect("parse deploy response json");
        assert!(
            deploy_json["id"].as_str().is_some(),
            "deploy response should include a run id"
        );

        tokio::time::timeout(Duration::from_secs(1), runtime.notified.notified())
            .await
            .expect("deploy task did not invoke the runtime");

        let prompt = runtime
            .prompt
            .lock()
            .expect("lock recorded prompt")
            .clone()
            .expect("runtime should capture the rendered prompt");
        assert_eq!(
            prompt, "Investigate codebase",
            "template deployment should interpolate request params into the prompt"
        );
    }

    #[tokio::test]
    async fn templates_bad_syntax_returns_400() {
        let state = test_state(Arc::new(NoOpRuntime));
        let router = test_router(state);

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/templates")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{"))
                    .expect("build malformed create request"),
            )
            .await
            .expect("malformed template response");

        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "malformed template JSON should be rejected with 400"
        );

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read malformed response body");
        let body_text = String::from_utf8(body.to_vec()).expect("decode malformed response body");
        assert!(
            body_text.contains("request body must be valid JSON"),
            "malformed template response should explain the JSON parse failure: {body_text}"
        );
    }

    fn test_state(runtime: Arc<dyn CliRuntime>) -> Arc<AppState> {
        let dir = tempdir().expect("create tempdir for template tests");
        Arc::new(
            AppState::new(
                dir.path().to_path_buf(),
                runtime,
                roko_core::config::schema::RokoConfig::default(),
                Arc::new(ManualBackend::default()),
            )
            .expect("AppState::new"),
        )
    }

    fn test_router(state: Arc<AppState>) -> Router {
        Router::new().nest("/api", routes()).with_state(state)
    }

    struct RecordingRuntime {
        prompt: Arc<Mutex<Option<String>>>,
        notified: Arc<Notify>,
    }

    impl RecordingRuntime {
        fn new() -> Self {
            Self {
                prompt: Arc::new(Mutex::new(None)),
                notified: Arc::new(Notify::new()),
            }
        }
    }

    #[async_trait]
    impl CliRuntime for RecordingRuntime {
        async fn run_once(
            &self,
            _workdir: &std::path::Path,
            prompt: &str,
        ) -> anyhow::Result<RunResult> {
            *self
                .prompt
                .lock()
                .expect("lock prompt recorder for template deployment") = Some(prompt.to_owned());
            self.notified.notify_one();
            Ok(RunResult {
                success: true,
                output_text: None,
                usage: None,
            })
        }

        fn session_status(&self, workdir: std::path::PathBuf) -> SessionStatusInfo {
            SessionStatusInfo {
                session_id: None,
                workdir,
                daemon_running: false,
                signal_count: None,
                episode_count: None,
                last_episode_passed: None,
            }
        }

        fn dashboard_scaffold(&self, _workdir: &std::path::Path) -> DashboardInfo {
            DashboardInfo {
                rendered: String::new(),
            }
        }
    }
}
