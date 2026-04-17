//! Plan CRUD, execution, and generation endpoints.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use validator::Validate;

use crate::error::{ApiError, validate_path_segment};
use crate::extract::{RequestPayload, ValidJson, validate_with_validator};
use crate::events::ServerEvent;
use crate::plan_types::{Plan, PlanTask};
use crate::runtime::RunResult;
use crate::state::{AppState, OperationHandle, OperationStatus, PlanHandle};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/plans", get(list_plans).post(create_plan))
        .route("/plans/{id}", get(get_plan))
        .route("/plans/{id}/execute", post(execute_plan))
        .route("/plans/{id}/status", get(plan_status))
        .route("/plans/generate", post(generate_plan))
}

/// `GET /api/plans` — list plans from `.roko/plans/`.
async fn list_plans(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let plans_dir = plans_dir(&state.workdir);
    if !plans_dir.is_dir() {
        return Ok(Json(json!([])));
    }

    let mut summaries = Vec::new();
    let mut entries = tokio::fs::read_dir(&plans_dir)
        .await
        .map_err(|e| ApiError::internal(format!("read plans dir: {e}")))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| ApiError::internal(format!("read plan entry: {e}")))?
    {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "toml" && ext != "json" {
            continue;
        }
        let plan = load_plan_file(&path).await?;
        summaries.push(json!({
            "id": plan.id,
            "title": plan.title,
            "task_count": plan.tasks.len(),
            "completed": plan.tasks.iter().all(|t| t.completed),
        }));
    }

    Ok(Json(Value::Array(summaries)))
}

/// `GET /api/plans/:id` — load a specific plan.
async fn get_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let plan = find_plan(&state.workdir, &id).await?;
    Ok(Json(plan_to_json(&plan)))
}

#[derive(Deserialize, Validate)]
struct CreatePlanRequest {
    #[validate(length(min = 1), custom(function = "crate::extract::validate_non_blank"))]
    title: String,
    #[validate(length(min = 1), custom(function = "crate::extract::validate_non_blank"))]
    description: String,
    #[serde(default)]
    #[validate(nested)]
    tasks: Vec<CreateTaskEntry>,
}

impl RequestPayload for CreatePlanRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        validate_with_validator(self)
    }
}

#[derive(Deserialize, Validate)]
struct CreateTaskEntry {
    #[validate(length(min = 1), custom(function = "crate::extract::validate_non_blank"))]
    id: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    #[validate(custom(function = "crate::extract::validate_string_items_non_blank"))]
    depends_on: Vec<String>,
    #[serde(default)]
    #[validate(custom(function = "crate::extract::validate_string_items_non_blank"))]
    files: Vec<String>,
}

/// `POST /api/plans` — create a new plan from a JSON body.
async fn create_plan(
    State(state): State<Arc<AppState>>,
    ValidJson(body): ValidJson<CreatePlanRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let plan_id = uuid::Uuid::new_v4().to_string();
    let mut plan = Plan::new(plan_id.clone(), body.title, body.description);

    for t in body.tasks {
        plan.add_task(PlanTask {
            id: t.id,
            description: t.description,
            depends_on: t.depends_on,
            files: t.files,
            completed: false,
        });
    }

    if let Err(errors) = plan.validate() {
        return Err(ApiError::bad_request(errors.join("; ")));
    }

    let plans_dir = plans_dir(&state.workdir);
    tokio::fs::create_dir_all(&plans_dir)
        .await
        .map_err(|e| ApiError::internal(format!("create plans dir: {e}")))?;

    let plan_json = plan_to_json(&plan);
    let path = plans_dir.join(format!("{plan_id}.json"));
    let content = serde_json::to_string_pretty(&plan_json)
        .map_err(|e| ApiError::internal(format!("serialize plan: {e}")))?;
    tokio::fs::write(&path, content)
        .await
        .map_err(|e| ApiError::internal(format!("write plan: {e}")))?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({ "id": plan_id })),
    ))
}

/// `POST /api/plans/:id/execute` — spawn a background plan execution task.
async fn execute_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    // Verify the plan exists.
    let plan = find_plan(&state.workdir, &id).await?;

    // Acquire write lock once to check-and-insert atomically (no TOCTOU race).
    let run_id = uuid::Uuid::new_v4().to_string();
    let bus = state.event_bus.clone();
    let runtime = state.runtime.clone();
    let workdir = state.workdir.clone();
    let prompt = build_plan_execution_prompt(&plan);
    let plan_id = id.clone();

    let mut active = state.active_plans.write().await;
    if active.contains_key(&id) {
        return Err(ApiError::conflict(format!(
            "plan {id} is already executing"
        )));
    }

    let handle = tokio::spawn({
        let plan_id = plan_id.clone();
        async move {
            bus.publish(ServerEvent::PlanStarted {
                plan_id: plan_id.clone(),
            });
            let success = match runtime.run_once(&workdir, &prompt).await {
                Ok(RunResult { success }) => success,
                Err(err) => {
                    bus.publish(ServerEvent::Error {
                        message: format!("plan execution failed for {plan_id}: {err}"),
                    });
                    false
                }
            };
            bus.publish(ServerEvent::PlanCompleted { plan_id, success });
        }
    });

    let plans_dir = plans_dir(&state.workdir);
    let plan_handle = PlanHandle {
        id: run_id.clone(),
        plan_dir: plans_dir,
        status: OperationStatus::Running,
        handle,
    };

    active.insert(id, plan_handle);
    drop(active);

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "id": run_id })),
    ))
}

/// `GET /api/plans/:id/status` — check execution status for a plan.
async fn plan_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let active = state.active_plans.read().await;
    active.get(&id).map_or_else(
        || Err(ApiError::not_found("no active execution for this plan")),
        |h| {
            Ok(Json(json!({
                "id": h.id,
                "plan_dir": h.plan_dir,
                "status": format!("{:?}", h.status),
                "finished": h.handle.is_finished(),
            })))
        },
    )
}

#[derive(Deserialize, Validate)]
struct GenerateRequest {
    #[validate(length(min = 1), custom(function = "crate::extract::validate_non_blank"))]
    slug: String,
}

impl RequestPayload for GenerateRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        validate_with_validator(self)
    }
}

/// `POST /api/plans/generate` — spawn background plan generation from a PRD slug.
async fn generate_plan(
    State(state): State<Arc<AppState>>,
    ValidJson(body): ValidJson<GenerateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let (prd_path, prd_content) = find_prd(&state.workdir, &body.slug).await?;
    let op_id = uuid::Uuid::new_v4().to_string();
    let bus = state.event_bus.clone();
    let runtime = state.runtime.clone();
    let workdir = state.workdir.clone();
    let prompt = build_plan_generation_prompt(&prd_path, &prd_content);
    let slug = body.slug.clone();
    let kind = format!("plan_generate:{slug}");
    let slug_for_task = slug.clone();

    let handle = tokio::spawn({
        let op_id = op_id.clone();
        async move {
            let success = match runtime.run_once(&workdir, &prompt).await {
                Ok(RunResult { success }) => success,
                Err(err) => {
                    bus.publish(ServerEvent::Error {
                        message: format!("plan generation failed for {slug_for_task}: {err}"),
                    });
                    false
                }
            };
            bus.publish(ServerEvent::OperationCompleted {
                op_id,
                kind: "plan_generate".into(),
                success,
            });
        }
    });

    let op = OperationHandle {
        id: op_id.clone(),
        kind,
        status: OperationStatus::Running,
        handle,
    };

    state.operations.write().await.insert(op_id.clone(), op);

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "id": op_id })),
    ))
}

// ── helpers ──────────────────────────────────────────────────────────

/// Locate and load a plan file by ID (checks `.json` then `.toml`).
async fn find_plan(workdir: &std::path::Path, id: &str) -> Result<Plan, ApiError> {
    validate_path_segment(id, "plan id")?;
    let plans_dir = plans_dir(workdir);
    for ext in &["json", "toml"] {
        let path = plans_dir.join(format!("{id}.{ext}"));
        if path.is_file() {
            return load_plan_file(&path).await;
        }
    }
    Err(ApiError::not_found(format!("plan '{id}' not found")))
}

/// Load a plan from a TOML or JSON file.
async fn load_plan_file(path: &std::path::Path) -> Result<Plan, ApiError> {
    #[derive(serde::Deserialize)]
    struct RawPlan {
        #[serde(default)]
        id: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        tasks: Vec<RawTask>,
    }

    #[derive(serde::Deserialize)]
    struct RawTask {
        #[serde(default)]
        id: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        depends_on: Vec<String>,
        #[serde(default)]
        files: Vec<String>,
        #[serde(default)]
        completed: bool,
    }

    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| ApiError::internal(format!("read plan file: {e}")))?;

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("json");

    let raw: RawPlan = match ext {
        "toml" => toml::from_str(&content)
            .map_err(|e| ApiError::internal(format!("parse plan TOML: {e}")))?,
        _ => serde_json::from_str(&content)
            .map_err(|e| ApiError::internal(format!("parse plan JSON: {e}")))?,
    };

    let mut plan = Plan::new(raw.id, raw.title, raw.description);
    for t in raw.tasks {
        plan.add_task(PlanTask {
            id: t.id,
            description: t.description,
            depends_on: t.depends_on,
            files: t.files,
            completed: t.completed,
        });
    }
    Ok(plan)
}

/// Serialize a `Plan` into a `serde_json::Value`.
fn plan_to_json(plan: &Plan) -> Value {
    json!({
        "id": plan.id,
        "title": plan.title,
        "description": plan.description,
        "tasks": plan.tasks.iter().map(|t| json!({
            "id": t.id,
            "description": t.description,
            "depends_on": t.depends_on,
            "files": t.files,
            "completed": t.completed,
        })).collect::<Vec<_>>(),
    })
}

async fn find_prd(
    workdir: &std::path::Path,
    slug: &str,
) -> Result<(std::path::PathBuf, String), ApiError> {
    validate_path_segment(slug, "PRD slug")?;

    let prds_dir = workdir.join(".roko").join("prd");
    for section in ["published", "drafts"] {
        let path = prds_dir.join(section).join(format!("{slug}.md"));
        if path.is_file() {
            let content = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| ApiError::internal(format!("read prd file: {e}")))?;
            return Ok((path, content));
        }
    }

    Err(ApiError::not_found(format!("PRD '{slug}' not found")))
}

fn build_plan_execution_prompt(plan: &Plan) -> String {
    let mut prompt = String::new();
    prompt.push_str(&format!(
        "Read the implementation plan at `.roko/plans/{id}` and execute it in the current workspace.\n\
         Use the plan as the source of truth for task order, file scope, and completion criteria.\n\
         Keep changes surgical and stop when the plan is complete.\n\n",
        id = plan.id,
    ));
    prompt.push_str("## Plan summary\n");
    prompt.push_str(&format!("- id: {}\n", plan.id));
    prompt.push_str(&format!("- title: {}\n", plan.title));
    prompt.push_str(&format!("- description: {}\n", plan.description));
    prompt.push_str("- tasks:\n");

    for task in &plan.tasks {
        prompt.push_str(&format!(
            "  - {}: {}\n    depends_on: {:?}\n    files: {:?}\n    completed: {}\n",
            task.id, task.description, task.depends_on, task.files, task.completed
        ));
    }

    prompt
}

fn build_plan_generation_prompt(prd_path: &std::path::Path, prd_content: &str) -> String {
    format!(
        "Read the PRD at {path} and generate implementation plan directories under .roko/plans.\n\
         Search the codebase first to understand what already exists.\n\
         Create or update plan.md and tasks.toml files directly, including per-task mcp_servers when a task needs a specific MCP server.\n\
         Each requirement and acceptance criterion should become one or more small, executable tasks.\n\n\
         PRD content:\n{content}\n",
        path = prd_path.display(),
        content = prd_content,
    )
}

fn plans_dir(workdir: &std::path::Path) -> std::path::PathBuf {
    workdir.join(".roko").join("plans")
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use roko_core::config::ServeAuthConfig;
    use tempfile::tempdir;
    use tokio::sync::Notify;
    use tower::ServiceExt;

    use crate::deploy::create_backend;
    use crate::routes::build_router;
    use crate::runtime::{CliRuntime, DashboardInfo, NoOpRuntime, RunResult, SessionStatusInfo};

    #[derive(Clone)]
    struct RecordingRuntime {
        calls: Arc<Mutex<Vec<(PathBuf, String)>>>,
        notify: Arc<Notify>,
        success: bool,
        call_count: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl CliRuntime for RecordingRuntime {
        async fn run_once(
            &self,
            workdir: &std::path::Path,
            prompt: &str,
        ) -> anyhow::Result<RunResult> {
            self.calls
                .lock()
                .expect("lock calls")
                .push((workdir.to_path_buf(), prompt.to_string()));
            self.call_count.fetch_add(1, Ordering::SeqCst);
            self.notify.notify_waiters();
            Ok(RunResult {
                success: self.success,
            })
        }

        fn session_status(&self, workdir: PathBuf) -> SessionStatusInfo {
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

    fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
        let dir = tempdir().expect("tempdir");
        let workdir = dir.path().to_path_buf();
        let deploy_backend =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        let state = Arc::new(AppState::new(
            workdir,
            Arc::new(NoOpRuntime),
            roko_core::config::schema::RokoConfig::default(),
            deploy_backend,
        ));
        (dir, state)
    }

    fn test_state_with_runtime(runtime: Arc<dyn CliRuntime>) -> (tempfile::TempDir, Arc<AppState>) {
        let dir = tempdir().expect("tempdir");
        let workdir = dir.path().to_path_buf();
        let deploy_backend =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        let state = Arc::new(AppState::new(
            workdir,
            runtime,
            roko_core::config::schema::RokoConfig::default(),
            deploy_backend,
        ));
        (dir, state)
    }

    #[tokio::test]
    async fn get_plan_returns_404_for_missing_plan() {
        let (_dir, state) = test_state();

        let err = get_plan(State(state), Path("missing-plan".into()))
            .await
            .expect_err("missing plan should error");

        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn execute_plan_returns_404_for_missing_plan() {
        let (_dir, state) = test_state();

        let err = match execute_plan(State(state), Path("missing-plan".into())).await {
            Ok(_) => panic!("missing plan should error"),
            Err(err) => err,
        };

        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn plan_status_returns_404_when_plan_is_not_active() {
        let (_dir, state) = test_state();

        let err = plan_status(State(state), Path("missing-plan".into()))
            .await
            .expect_err("missing active plan should error");

        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn create_plan_rejects_empty_fields() {
        let request = CreatePlanRequest {
            title: "   ".into(),
            description: "desc".into(),
            tasks: vec![CreateTaskEntry {
                id: " ".into(),
                description: "task".into(),
                depends_on: vec![],
                files: vec![],
            }],
        };

        assert!(request.validate().is_err());
    }

    #[tokio::test]
    async fn create_plan_route_returns_top_level_validation_error() {
        let (_dir, state) = test_state();
        let app = build_router(Arc::clone(&state), &[], ServeAuthConfig::default());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/plans")
                    .body(Body::from(r#"{"title":"   ","description":"desc"}"#))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: Value = serde_json::from_slice(&body).expect("parse response body");
        assert_eq!(payload["code"], "validation_error");
        assert_eq!(payload["message"], "request body validation failed");
        assert!(payload.get("error").is_none());
    }

    #[tokio::test]
    async fn generate_plan_rejects_empty_slug() {
        assert!(GenerateRequest { slug: "  ".into() }.validate().is_err());
    }

    #[tokio::test]
    async fn execute_plan_runs_runtime_with_plan_context() {
        let runtime = Arc::new(RecordingRuntime {
            calls: Arc::new(Mutex::new(Vec::new())),
            notify: Arc::new(Notify::new()),
            success: true,
            call_count: Arc::new(AtomicUsize::new(0)),
        });
        let notify = Arc::clone(&runtime.as_ref().notify);
        let calls = Arc::clone(&runtime.as_ref().calls);
        let (_dir, state) = test_state_with_runtime(runtime);

        let plans_dir = state.workdir.join(".roko").join("plans");
        tokio::fs::create_dir_all(&plans_dir)
            .await
            .expect("create plans dir");
        tokio::fs::write(
            plans_dir.join("demo.json"),
            serde_json::to_string_pretty(&json!({
                "id": "demo",
                "title": "Demo Plan",
                "description": "Implement the demo",
                "tasks": [
                    {
                        "id": "T1",
                        "description": "Update the widget",
                        "depends_on": [],
                        "files": ["src/widget.rs"],
                        "completed": false
                    }
                ]
            }))
            .expect("serialize plan"),
        )
        .await
        .expect("write plan");

        let response = execute_plan(State(Arc::clone(&state)), Path("demo".into()))
            .await
            .expect("execute plan");

        assert_eq!(
            response.into_response().status(),
            axum::http::StatusCode::ACCEPTED
        );

        tokio::time::timeout(std::time::Duration::from_secs(1), notify.notified())
            .await
            .expect("runtime should be called");

        let calls = calls.lock().expect("lock calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, state.workdir);
        assert!(calls[0].1.contains("Demo Plan"));
        assert!(calls[0].1.contains("Update the widget"));
    }

    #[tokio::test]
    async fn generate_plan_runs_runtime_with_prd_context() {
        let runtime = Arc::new(RecordingRuntime {
            calls: Arc::new(Mutex::new(Vec::new())),
            notify: Arc::new(Notify::new()),
            success: true,
            call_count: Arc::new(AtomicUsize::new(0)),
        });
        let notify = Arc::clone(&runtime.as_ref().notify);
        let calls = Arc::clone(&runtime.as_ref().calls);
        let (_dir, state) = test_state_with_runtime(runtime);

        let published_dir = state.workdir.join(".roko").join("prd").join("published");
        tokio::fs::create_dir_all(&published_dir)
            .await
            .expect("create published dir");
        tokio::fs::write(
            published_dir.join("demo.md"),
            "---\nstatus: published\n---\n# Demo PRD\nBuild the widget.\n",
        )
        .await
        .expect("write prd");

        let response = generate_plan(
            State(Arc::clone(&state)),
            ValidJson(GenerateRequest {
                slug: "demo".into(),
            }),
        )
        .await
        .expect("generate plan");

        assert_eq!(
            response.into_response().status(),
            axum::http::StatusCode::ACCEPTED
        );

        tokio::time::timeout(std::time::Duration::from_secs(1), notify.notified())
            .await
            .expect("runtime should be called");

        let calls = calls.lock().expect("lock calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, state.workdir);
        assert!(calls[0].1.contains(".roko/prd/published/demo.md"));
        assert!(calls[0].1.contains("Build the widget."));
    }

    #[tokio::test]
    async fn list_plans_returns_internal_error_for_corrupt_plan_file() {
        let (dir, state) = test_state();
        let plans_dir = state.workdir.join(".roko").join("plans");
        tokio::fs::create_dir_all(&plans_dir)
            .await
            .expect("create plans dir");
        tokio::fs::write(plans_dir.join("broken.json"), "{not-json}")
            .await
            .expect("write corrupt plan");

        let err = list_plans(State(state))
            .await
            .expect_err("corrupt plan should fail");

        assert_eq!(err.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        drop(dir);
    }
}
