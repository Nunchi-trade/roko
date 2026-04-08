//! Plan CRUD, execution, and generation endpoints.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::plan::{Plan, PlanTask};
use crate::serve::error::ApiError;
use crate::serve::events::ServerEvent;
use crate::serve::state::{AppState, OperationHandle, OperationStatus, PlanHandle};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/plans", get(list_plans).post(create_plan))
        .route("/plans/{id}", get(get_plan))
        .route("/plans/{id}/execute", post(execute_plan))
        .route("/plans/{id}/status", get(plan_status))
        .route("/plans/generate", post(generate_plan))
}

/// `GET /api/plans` — list plans from `.roko/plans/`.
async fn list_plans(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, ApiError> {
    let plans_dir = state.workdir.join(".roko").join("plans");
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
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Try to load plan; if it fails, still include a minimal summary.
        match load_plan_file(&path).await {
            Ok(plan) => summaries.push(json!({
                "id": plan.id,
                "title": plan.title,
                "task_count": plan.tasks.len(),
                "completed": plan.tasks.iter().all(|t| t.completed),
            })),
            Err(_) => summaries.push(json!({
                "id": stem,
                "title": stem,
                "task_count": 0,
                "completed": false,
            })),
        }
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

#[derive(Deserialize)]
struct CreatePlanRequest {
    title: String,
    description: String,
    #[serde(default)]
    tasks: Vec<CreateTaskEntry>,
}

#[derive(Deserialize)]
struct CreateTaskEntry {
    id: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    files: Vec<String>,
}

/// `POST /api/plans` — create a new plan from a JSON body.
async fn create_plan(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreatePlanRequest>,
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

    let plans_dir = state.workdir.join(".roko").join("plans");
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
    let _plan = find_plan(&state.workdir, &id).await?;

    // Check for duplicate execution.
    {
        let active = state.active_plans.read().await;
        if active.contains_key(&id) {
            return Err(ApiError::conflict(format!("plan {id} is already executing")));
        }
    }

    let run_id = uuid::Uuid::new_v4().to_string();
    let bus = state.event_bus.sender();
    let plan_id = id.clone();

    // Spawn a placeholder execution task. Full PlanRunner wiring is a
    // separate task — for now we mark the operation as completed immediately.
    let handle = tokio::spawn({
        let plan_id = plan_id.clone();
        async move {
            bus.emit(ServerEvent::PlanStarted {
                plan_id: plan_id.clone(),
            });
            // TODO: Wire PlanRunner execution here.
            bus.emit(ServerEvent::PlanCompleted {
                plan_id,
                success: true,
            });
        }
    });

    let plans_dir = state.workdir.join(".roko").join("plans");
    let plan_handle = PlanHandle {
        id: run_id.clone(),
        plan_dir: plans_dir,
        status: OperationStatus::Running,
        handle,
    };

    state.active_plans.write().await.insert(id, plan_handle);

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

#[derive(Deserialize)]
struct GenerateRequest {
    slug: String,
}

/// `POST /api/plans/generate` — spawn background plan generation from a PRD slug.
async fn generate_plan(
    State(state): State<Arc<AppState>>,
    Json(body): Json<GenerateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let op_id = uuid::Uuid::new_v4().to_string();
    let bus = state.event_bus.sender();
    let slug = body.slug.clone();

    let handle = tokio::spawn({
        let op_id = op_id.clone();
        async move {
            // TODO: Wire actual plan generation (`prd plan <slug>`).
            bus.emit(ServerEvent::OperationCompleted {
                op_id,
                kind: "plan_generate".into(),
                success: true,
            });
        }
    });

    let op = OperationHandle {
        id: op_id.clone(),
        kind: format!("plan_generate:{slug}"),
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
    let plans_dir = workdir.join(".roko").join("plans");
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

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("json");

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
