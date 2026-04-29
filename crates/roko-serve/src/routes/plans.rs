//! Plan CRUD, execution, generation, control, and estimation endpoints.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use validator::Validate;

use crate::error::{ApiError, validate_path_segment};
use crate::events::ServerEvent;
use crate::extract::{RequestPayload, ValidJson, validate_with_validator};
use crate::plan_types::{Plan, PlanTask};
use crate::runtime::RunResult;
use crate::state::{AppState, OperationHandle, OperationStatus, PlanHandle};
use roko_runtime::cancel::CancelToken;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/plans", get(list_plans).post(create_plan))
        .route("/plans/{id}", get(get_plan))
        .route("/plans/{id}/tasks", get(plan_tasks))
        .route("/plans/{id}/execute", post(execute_plan))
        .route("/plans/{id}/status", get(plan_status))
        .route("/plans/{id}/pause", post(pause_plan))
        .route("/plans/{id}/resume", post(resume_plan))
        .route("/plans/{id}/gates", get(plan_gates))
        .route("/plans/{id}/reviews", get(list_reviews))
        .route("/plans/{id}/tasks/{task_id}/review", post(submit_review))
        .route("/plans/{id}/tasks/{task_id}/diff", get(task_diff))
        .route("/plans/{id}/chat", post(plan_chat))
        .route("/plans/{id}/estimate", post(plan_estimate))
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
        let completed_count = plan.tasks.iter().filter(|t| t.completed).count();
        summaries.push(json!({
            "id": plan.id,
            "title": plan.title,
            "task_count": plan.tasks.len(),
            "completed": plan.tasks.iter().all(|t| t.completed),
            "completed_task_count": completed_count,
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

/// `GET /api/plans/:id/tasks` — return the task list for a specific plan.
async fn plan_tasks(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let plan = find_plan(&state.workdir, &id).await?;
    let tasks: Vec<Value> = plan
        .tasks
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "description": t.description,
                "depends_on": t.depends_on,
                "files": t.files,
                "completed": t.completed,
                "status": task_status(t),
            })
        })
        .collect();
    Ok(Json(json!({
        "plan_id": plan.id,
        "task_count": tasks.len(),
        "tasks": tasks,
    })))
}

#[derive(Deserialize, Validate)]
struct CreatePlanRequest {
    #[validate(
        length(min = 1),
        custom(function = "crate::extract::validate_non_blank")
    )]
    title: String,
    #[validate(
        length(min = 1),
        custom(function = "crate::extract::validate_non_blank")
    )]
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
    #[validate(
        length(min = 1),
        custom(function = "crate::extract::validate_non_blank")
    )]
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

    let state_for_task = Arc::clone(&state);
    let handle = tokio::spawn({
        let plan_id = plan_id.clone();
        async move {
            bus.publish(ServerEvent::PlanStarted {
                plan_id: plan_id.clone(),
            });
            let success = match runtime.run_once(&workdir, &prompt).await {
                Ok(RunResult { success, .. }) => {
                    state_for_task.provider_health.record_success("default");
                    success
                }
                Err(err) => {
                    state_for_task.provider_health.record_failure("default");
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
        cancel: CancelToken::new(),
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

// ── Pause / Resume ───────────────────────────────────────────────────

/// `POST /api/plans/:id/pause` — pause a running plan execution.
///
/// Cancels the background task and saves a snapshot so the plan can be
/// resumed later.  Returns 200 with `{ "paused": true }` on success, 404
/// if the plan is not actively executing, or 409 if it already finished.
async fn pause_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let mut active = state.active_plans.write().await;
    let handle = active
        .get(&id)
        .ok_or_else(|| ApiError::not_found("no active execution for this plan"))?;

    if handle.handle.is_finished() {
        return Err(ApiError::conflict("plan execution already finished"));
    }

    // Signal cancellation then abort the tokio task.
    handle.cancel.cancel();
    handle.handle.abort();

    // Write a lightweight snapshot so the dashboard knows the plan is
    // paused and `POST /resume` can restart it.
    let snapshot_dir = state.workdir.join(".roko").join("state");
    let _ = tokio::fs::create_dir_all(&snapshot_dir).await;
    let snapshot_path = snapshot_dir.join(format!("{id}.paused.json"));
    let snapshot = json!({
        "plan_id": id,
        "paused": true,
        "paused_at": chrono::Utc::now().to_rfc3339(),
        "plan_dir": handle.plan_dir,
        "run_id": handle.id,
    });
    let _ = tokio::fs::write(
        &snapshot_path,
        serde_json::to_string_pretty(&snapshot).unwrap_or_default(),
    )
    .await;

    // Remove from active set.
    active.remove(&id);

    state.event_bus.publish(ServerEvent::PlanCompleted {
        plan_id: id.clone(),
        success: false,
    });

    Ok(Json(json!({ "paused": true, "snapshot": snapshot_path })))
}

/// `POST /api/plans/:id/resume` — resume a paused plan execution.
///
/// Re-starts the plan from where it left off.  If the plan was paused via
/// `/pause`, a `.paused.json` snapshot exists in `.roko/state/`.
async fn resume_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    // Check it's not already running.
    let active = state.active_plans.read().await;
    if active.contains_key(&id) {
        return Err(ApiError::conflict(format!(
            "plan {id} is already executing"
        )));
    }
    drop(active);

    // Load the plan to verify it exists.
    let plan = find_plan(&state.workdir, &id).await?;

    // Clean up the paused marker if it exists.
    let paused_path = state
        .workdir
        .join(".roko")
        .join("state")
        .join(format!("{id}.paused.json"));
    let _ = tokio::fs::remove_file(&paused_path).await;

    // Re-execute (same logic as execute, but marks it as a resume).
    let run_id = uuid::Uuid::new_v4().to_string();
    let bus = state.event_bus.clone();
    let runtime = state.runtime.clone();
    let workdir = state.workdir.clone();
    let prompt = build_plan_resume_prompt(&plan);
    let plan_id = id.clone();

    let mut active = state.active_plans.write().await;
    if active.contains_key(&id) {
        return Err(ApiError::conflict(format!(
            "plan {id} is already executing"
        )));
    }

    let cancel = CancelToken::new();
    let state_for_task = Arc::clone(&state);
    let handle = tokio::spawn({
        let plan_id = plan_id.clone();
        async move {
            bus.publish(ServerEvent::PlanStarted {
                plan_id: plan_id.clone(),
            });
            let success = match runtime.run_once(&workdir, &prompt).await {
                Ok(RunResult { success, .. }) => {
                    state_for_task.provider_health.record_success("default");
                    success
                }
                Err(err) => {
                    state_for_task.provider_health.record_failure("default");
                    bus.publish(ServerEvent::Error {
                        message: format!("plan resume failed for {plan_id}: {err}"),
                    });
                    false
                }
            };
            bus.publish(ServerEvent::PlanCompleted { plan_id, success });
        }
    });

    let plan_handle = PlanHandle {
        id: run_id.clone(),
        plan_dir: plans_dir(&state.workdir),
        status: OperationStatus::Running,
        handle,
        cancel,
    };

    active.insert(id, plan_handle);
    drop(active);

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "id": run_id, "resumed": true })),
    ))
}

// ── Verify results query ──────────────────────────────────────────────

/// `GET /api/plans/:id/gates` — query gate results for a specific plan.
///
/// Returns gate verdicts from the materialized dashboard snapshot, filtered
/// to the requested plan.
async fn plan_gates(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    validate_path_segment(&id, "plan id")?;

    // Verify the plan exists on disk.
    let _plan = find_plan(&state.workdir, &id).await?;

    // Pull gate results from the materialized snapshot.
    let snapshot = state.state_hub.current_snapshot();
    let gates: Vec<Value> = snapshot
        .gates
        .iter()
        .filter(|g| g.plan_id == id)
        .map(|g| {
            json!({
                "plan_id": g.plan_id,
                "task_id": g.task_id,
                "gate": g.gate,
                "passed": g.passed,
                "ts_millis": g.ts_millis,
            })
        })
        .collect();

    Ok(Json(json!({
        "plan_id": id,
        "gate_count": gates.len(),
        "passed": gates.iter().filter(|g| g["passed"] == true).count(),
        "failed": gates.iter().filter(|g| g["passed"] == false).count(),
        "gates": gates,
    })))
}

// ── Chat-based plan editing ─────────────────────────────────────────

#[derive(Deserialize, Validate)]
struct PlanChatRequest {
    #[validate(
        length(min = 1),
        custom(function = "crate::extract::validate_non_blank")
    )]
    message: String,
}

impl RequestPayload for PlanChatRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        validate_with_validator(self)
    }
}

/// `POST /api/plans/:id/chat` — LLM-powered plan mutation via natural language.
///
/// Sends the plan context + user message to the LLM, which returns structured
/// plan mutations (add/remove/update tasks, reorder, add dependencies).
async fn plan_chat(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ValidJson(body): ValidJson<PlanChatRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let plan = find_plan(&state.workdir, &id).await?;
    let plan_json = plan_to_json(&plan);

    let prompt = format!(
        "You are a plan editor. Given the plan below and the user's request, return a JSON \
         object with a `mutations` array. Each mutation is one of:\n\
         - {{\"op\":\"add_task\",\"task\":{{\"id\":\"T-new\",\"description\":\"...\",\"depends_on\":[],\"files\":[]}}}}\n\
         - {{\"op\":\"remove_task\",\"task_id\":\"T-old\"}}\n\
         - {{\"op\":\"update_task\",\"task_id\":\"T1\",\"patch\":{{\"description\":\"new desc\"}}}}\n\
         - {{\"op\":\"add_dependency\",\"task_id\":\"T2\",\"depends_on\":\"T1\"}}\n\
         - {{\"op\":\"reorder\",\"order\":[\"T1\",\"T3\",\"T2\"]}}\n\n\
         Return ONLY the JSON object, no markdown fences.\n\n\
         ## Current plan\n```json\n{plan}\n```\n\n## User request\n{msg}",
        plan = serde_json::to_string_pretty(&plan_json).unwrap_or_default(),
        msg = body.message,
    );

    let op_id = uuid::Uuid::new_v4().to_string();
    let bus = state.event_bus.clone();
    let runtime = state.runtime.clone();
    let workdir = state.workdir.clone();
    let plan_id = id.clone();

    let state_for_task = Arc::clone(&state);
    let op_id_inner = op_id.clone();
    let handle = tokio::spawn(async move {
        let success = match runtime.run_once(&workdir, &prompt).await {
            Ok(RunResult {
                success,
                output_text,
                ..
            }) => {
                // Try to parse mutations from the LLM output and apply them.
                if let Some(ref text) = output_text {
                    if let Ok(mutations) = serde_json::from_str::<Value>(text) {
                        // Write the mutations to a response file for the caller.
                        let mutations_path = workdir
                            .join(".roko")
                            .join("state")
                            .join(format!("{plan_id}.chat-response.json"));
                        let _ = tokio::fs::write(
                            &mutations_path,
                            serde_json::to_string_pretty(&mutations).unwrap_or_default(),
                        )
                        .await;
                    }
                }
                state_for_task.provider_health.record_success("default");
                success
            }
            Err(err) => {
                state_for_task.provider_health.record_failure("default");
                bus.publish(ServerEvent::Error {
                    message: format!("plan chat failed for {plan_id}: {err}"),
                });
                false
            }
        };
        bus.publish(ServerEvent::OperationCompleted {
            op_id: op_id_inner,
            kind: "plan_chat".into(),
            success,
        });
    });

    let op = OperationHandle {
        id: op_id.clone(),
        kind: format!("plan_chat:{id}"),
        status: OperationStatus::Running,
        handle,
    };
    state.operations.write().await.insert(op_id.clone(), op);

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "id": op_id })),
    ))
}

// ── Cost estimation ─────────────────────────────────────────────────

/// `POST /api/plans/:id/estimate` — estimate cost and time for plan execution.
///
/// Reads historical efficiency events to build per-task estimates based on
/// past performance for similar roles and models.
async fn plan_estimate(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let plan = find_plan(&state.workdir, &id).await?;

    // Load historical efficiency data.
    let efficiency_path = state
        .workdir
        .join(".roko")
        .join("learn")
        .join("efficiency.jsonl");
    let historical = load_efficiency_history(&efficiency_path).await;

    // Compute per-task estimates.
    let mut task_estimates = Vec::new();
    let mut total_input_tokens: u64 = 0;
    let mut total_output_tokens: u64 = 0;
    let mut total_cost_usd: f64 = 0.0;
    let mut total_duration_secs: f64 = 0.0;

    for task in &plan.tasks {
        if task.completed {
            continue;
        }
        // Find similar historical tasks (by matching plan role or averaging all).
        let (est_input, est_output, est_cost, est_duration) =
            estimate_task_from_history(&historical);

        total_input_tokens += est_input;
        total_output_tokens += est_output;
        total_cost_usd += est_cost;
        total_duration_secs += est_duration;

        task_estimates.push(json!({
            "task_id": task.id,
            "description": task.description,
            "estimated_input_tokens": est_input,
            "estimated_output_tokens": est_output,
            "estimated_cost_usd": format!("{:.4}", est_cost),
            "estimated_duration_secs": format!("{:.0}", est_duration),
        }));
    }

    let completed = plan.tasks.iter().filter(|t| t.completed).count();

    Ok(Json(json!({
        "plan_id": id,
        "total_tasks": plan.tasks.len(),
        "completed_tasks": completed,
        "remaining_tasks": plan.tasks.len() - completed,
        "estimate": {
            "total_input_tokens": total_input_tokens,
            "total_output_tokens": total_output_tokens,
            "total_cost_usd": format!("{:.4}", total_cost_usd),
            "total_duration_secs": format!("{:.0}", total_duration_secs),
        },
        "per_task": task_estimates,
        "confidence": if historical.is_empty() { "low" } else { "medium" },
        "note": if historical.is_empty() {
            "No historical data available; using default estimates"
        } else {
            "Based on historical efficiency events"
        },
    })))
}

/// A simplified efficiency record parsed from the JSONL log.
struct HistoricalEfficiency {
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: f64,
    duration_secs: f64,
}

/// Load efficiency history from the JSONL log file.
async fn load_efficiency_history(path: &std::path::Path) -> Vec<HistoricalEfficiency> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .filter_map(|line| {
            let v: Value = serde_json::from_str(line).ok()?;
            Some(HistoricalEfficiency {
                input_tokens: v.get("input_tokens")?.as_u64()?,
                output_tokens: v.get("output_tokens")?.as_u64()?,
                cost_usd: v.get("cost_usd").and_then(|c| c.as_f64()).unwrap_or(0.0),
                duration_secs: v
                    .get("wall_clock_ms")
                    .and_then(|d| d.as_f64())
                    .unwrap_or(30_000.0)
                    / 1000.0,
            })
        })
        .collect()
}

/// Estimate a single task from historical averages, with fallback defaults.
fn estimate_task_from_history(history: &[HistoricalEfficiency]) -> (u64, u64, f64, f64) {
    if history.is_empty() {
        // Default estimates for a single agent task.
        return (8_000, 4_000, 0.05, 60.0);
    }

    let n = history.len() as f64;
    let avg_input = (history.iter().map(|h| h.input_tokens).sum::<u64>() as f64 / n) as u64;
    let avg_output = (history.iter().map(|h| h.output_tokens).sum::<u64>() as f64 / n) as u64;
    let avg_cost = history.iter().map(|h| h.cost_usd).sum::<f64>() / n;
    let avg_duration = history.iter().map(|h| h.duration_secs).sum::<f64>() / n;

    (avg_input, avg_output, avg_cost, avg_duration)
}

fn build_plan_resume_prompt(plan: &Plan) -> String {
    let mut prompt = String::new();
    prompt.push_str(&format!(
        "Resume the implementation plan at `.roko/plans/{id}` from where it was paused.\n\
         Skip tasks that are already completed. Continue with the next pending task.\n\
         Use the plan as the source of truth for task order, file scope, and completion criteria.\n\n",
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

// ── Review workflow ──────────────────────────────────────────────────

/// `GET /api/plans/:id/reviews` — list tasks pending review.
///
/// Scans plan tasks that are completed (by an agent) and checks for
/// corresponding agent branches.  Returns gate results from the snapshot
/// and diff summaries from git.
async fn list_reviews(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let plan = find_plan(&state.workdir, &id).await?;

    // Pull gate results for this plan from the snapshot.
    let snapshot = state.state_hub.current_snapshot();
    let plan_gates: Vec<_> = snapshot.gates.iter().filter(|g| g.plan_id == id).collect();

    let mut reviews = Vec::new();

    for task in &plan.tasks {
        // Detect agent branches: convention is `agent/<name>/<task_id>`.
        let branch = find_agent_branch(&state.workdir, &task.id).await;

        // Gather gate results for this task.
        let task_gates: Vec<Value> = plan_gates
            .iter()
            .filter(|g| g.task_id == task.id)
            .map(|g| {
                json!({
                    "gate": g.gate,
                    "passed": g.passed,
                })
            })
            .collect();

        // Compute diff summary if branch exists.
        let (diff_summary, files_changed) = if let Some(ref branch_name) = branch {
            diff_summary(&state.workdir, branch_name).await
        } else {
            (String::new(), Vec::new())
        };

        // Determine review status.
        let status = if branch.is_some() && !files_changed.is_empty() {
            "pending_review"
        } else if task.completed {
            "completed"
        } else {
            "pending"
        };

        reviews.push(json!({
            "task_id": task.id,
            "description": task.description,
            "status": status,
            "branch": branch,
            "diff_summary": diff_summary,
            "gate_results": task_gates,
            "files_changed": files_changed,
        }));
    }

    Ok(Json(json!({
        "plan_id": id,
        "reviews": reviews,
    })))
}

#[derive(Deserialize, Validate)]
struct ReviewDecision {
    #[validate(custom(function = "validate_decision"))]
    decision: String,
    #[serde(default)]
    comment: String,
}

fn validate_decision(decision: &str) -> Result<(), validator::ValidationError> {
    match decision {
        "approve" | "reject" | "skip" => Ok(()),
        _ => {
            let mut err = validator::ValidationError::new("invalid_decision");
            err.message = Some("decision must be approve, reject, or skip".into());
            Err(err)
        }
    }
}

impl RequestPayload for ReviewDecision {
    fn validate_payload(&self) -> Result<(), ApiError> {
        validate_with_validator(self)
    }
}

/// `POST /api/plans/:id/tasks/:task_id/review` — approve, reject, or skip.
///
/// - **approve**: merge the agent branch into the base branch, mark approved.
/// - **reject**: record rejection with comment, keep branch for rework.
/// - **skip**: mark task as skipped, no merge.
async fn submit_review(
    State(state): State<Arc<AppState>>,
    Path((id, task_id)): Path<(String, String)>,
    ValidJson(body): ValidJson<ReviewDecision>,
) -> Result<Json<Value>, ApiError> {
    validate_path_segment(&id, "plan id")?;
    validate_path_segment(&task_id, "task id")?;

    // Verify plan and task exist.
    let plan = find_plan(&state.workdir, &id).await?;
    if !plan.tasks.iter().any(|t| t.id == task_id) {
        return Err(ApiError::not_found(format!(
            "task '{task_id}' not found in plan '{id}'"
        )));
    }

    let branch = find_agent_branch(&state.workdir, &task_id).await;

    let result = match body.decision.as_str() {
        "approve" => {
            let merged = if let Some(ref branch_name) = branch {
                merge_branch(&state.workdir, branch_name).await
            } else {
                false
            };

            // Record the review in the state directory.
            record_review(&state.workdir, &id, &task_id, "approved", &body.comment).await;

            json!({
                "task_id": task_id,
                "status": "approved",
                "merged": merged,
                "branch": branch,
            })
        }
        "reject" => {
            // Send feedback to the agent if possible.
            if let Some(ref branch_name) = branch {
                // Extract agent name from branch: `agent/<name>/<task_id>`
                let agent_name = branch_name
                    .strip_prefix("agent/")
                    .and_then(|s| s.split('/').next())
                    .unwrap_or("");

                if !agent_name.is_empty() {
                    let feedback = format!(
                        "Review rejected for task {task_id}: {}",
                        if body.comment.is_empty() {
                            "No comment provided"
                        } else {
                            &body.comment
                        }
                    );
                    // Publish as an event so agents can pick it up.
                    state.event_bus.publish(ServerEvent::Error {
                        message: format!(
                            "review:reject agent={agent_name} task={task_id}: {feedback}"
                        ),
                    });
                }
            }

            record_review(&state.workdir, &id, &task_id, "rejected", &body.comment).await;

            json!({
                "task_id": task_id,
                "status": "needs_rework",
                "branch": branch,
                "comment": body.comment,
            })
        }
        "skip" => {
            record_review(&state.workdir, &id, &task_id, "skipped", &body.comment).await;
            json!({ "task_id": task_id, "status": "skipped" })
        }
        _ => unreachable!("validated above"),
    };

    Ok(Json(result))
}

/// `GET /api/plans/:id/tasks/:task_id/diff` — structured diff for a task.
///
/// Finds the agent branch and runs `git diff` against main. Returns per-file
/// diff entries with path, status, additions, deletions, and unified patch.
async fn task_diff(
    State(state): State<Arc<AppState>>,
    Path((id, task_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    validate_path_segment(&id, "plan id")?;
    validate_path_segment(&task_id, "task id")?;

    // Verify plan and task exist.
    let plan = find_plan(&state.workdir, &id).await?;
    if !plan.tasks.iter().any(|t| t.id == task_id) {
        return Err(ApiError::not_found(format!(
            "task '{task_id}' not found in plan '{id}'"
        )));
    }

    let branch = find_agent_branch(&state.workdir, &task_id)
        .await
        .ok_or_else(|| {
            ApiError::not_found(format!("no agent branch found for task '{task_id}'"))
        })?;

    let files = parse_git_diff(&state.workdir, &branch).await?;

    Ok(Json(json!({
        "task_id": task_id,
        "branch": branch,
        "base": "main",
        "file_count": files.len(),
        "total_additions": files.iter().map(|f| f.additions).sum::<u32>(),
        "total_deletions": files.iter().map(|f| f.deletions).sum::<u32>(),
        "files": files.iter().map(|f| json!({
            "path": f.path,
            "status": f.status,
            "additions": f.additions,
            "deletions": f.deletions,
            "patch": f.patch,
        })).collect::<Vec<_>>(),
    })))
}

// ── Review helpers ──────────────────────────────────────────────────

/// Find an agent branch matching the task ID.
///
/// Convention: `agent/<agent-name>/<task_id>`.
async fn find_agent_branch(workdir: &std::path::Path, task_id: &str) -> Option<String> {
    let output = tokio::process::Command::new("git")
        .args(["branch", "--list", &format!("agent/*/{task_id}")])
        .current_dir(workdir)
        .output()
        .await
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    // git branch --list returns "  branch-name\n" or "* branch-name\n"
    stdout
        .lines()
        .map(|l| l.trim_start_matches(['*', ' '].as_ref()).trim().to_string())
        .find(|l| !l.is_empty())
}

/// Compute a short diff summary ("+N -M across K files") for a branch.
async fn diff_summary(workdir: &std::path::Path, branch: &str) -> (String, Vec<String>) {
    let output = tokio::process::Command::new("git")
        .args(["diff", "--stat", &format!("main...{branch}")])
        .current_dir(workdir)
        .output()
        .await;

    let output = match output {
        Ok(o) => o,
        Err(_) => return (String::new(), Vec::new()),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();

    // Last line is summary: " N files changed, M insertions(+), K deletions(-)"
    let summary = lines
        .last()
        .map(|l| l.trim().to_string())
        .unwrap_or_default();

    // Each preceding line is " path/to/file | N ++--"
    let files: Vec<String> = lines
        .iter()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.contains('|') {
                Some(trimmed.split('|').next().unwrap_or("").trim().to_string())
            } else {
                None
            }
        })
        .collect();

    (summary, files)
}

struct DiffFile {
    path: String,
    status: String,
    additions: u32,
    deletions: u32,
    patch: String,
}

/// Parse `git diff --numstat` + `git diff` into structured per-file entries.
async fn parse_git_diff(
    workdir: &std::path::Path,
    branch: &str,
) -> Result<Vec<DiffFile>, ApiError> {
    // Get numstat for additions/deletions counts.
    let numstat = tokio::process::Command::new("git")
        .args(["diff", "--numstat", &format!("main...{branch}")])
        .current_dir(workdir)
        .output()
        .await
        .map_err(|e| ApiError::internal(format!("git diff --numstat: {e}")))?;

    let numstat_str = String::from_utf8_lossy(&numstat.stdout);

    // Get the full diff for patches.
    let full_diff = tokio::process::Command::new("git")
        .args(["diff", &format!("main...{branch}")])
        .current_dir(workdir)
        .output()
        .await
        .map_err(|e| ApiError::internal(format!("git diff: {e}")))?;

    let full_diff_str = String::from_utf8_lossy(&full_diff.stdout);

    // Parse per-file patches from the full diff.
    let mut file_patches: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut current_file = String::new();
    let mut current_patch = String::new();

    for line in full_diff_str.lines() {
        if line.starts_with("diff --git") {
            if !current_file.is_empty() {
                file_patches.insert(current_file.clone(), current_patch.clone());
            }
            // Extract filename from "diff --git a/path b/path"
            current_file = line.split(" b/").nth(1).unwrap_or("").to_string();
            current_patch = String::new();
        }
        current_patch.push_str(line);
        current_patch.push('\n');
    }
    if !current_file.is_empty() {
        file_patches.insert(current_file, current_patch);
    }

    // Parse numstat lines: "additions\tdeletions\tpath"
    let mut files = Vec::new();
    for line in numstat_str.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 3 {
            continue;
        }
        let additions = parts[0].parse::<u32>().unwrap_or(0);
        let deletions = parts[1].parse::<u32>().unwrap_or(0);
        let path = parts[2].to_string();

        let status = if additions > 0 && deletions == 0 {
            "added"
        } else if additions == 0 && deletions > 0 {
            "deleted"
        } else {
            "modified"
        };

        let patch = file_patches.get(&path).cloned().unwrap_or_default();

        files.push(DiffFile {
            path,
            status: status.to_string(),
            additions,
            deletions,
            patch,
        });
    }

    Ok(files)
}

/// Merge an agent branch into the current branch.
async fn merge_branch(workdir: &std::path::Path, branch: &str) -> bool {
    let output = tokio::process::Command::new("git")
        .args([
            "merge",
            "--no-ff",
            "-m",
            &format!("Merge {branch} (approved via review)"),
            branch,
        ])
        .current_dir(workdir)
        .output()
        .await;

    matches!(output, Ok(o) if o.status.success())
}

/// Record a review decision to `.roko/state/reviews.jsonl`.
async fn record_review(
    workdir: &std::path::Path,
    plan_id: &str,
    task_id: &str,
    decision: &str,
    comment: &str,
) {
    let reviews_path = workdir.join(".roko").join("state").join("reviews.jsonl");
    let _ = tokio::fs::create_dir_all(reviews_path.parent().unwrap_or(workdir)).await;

    let entry = serde_json::json!({
        "plan_id": plan_id,
        "task_id": task_id,
        "decision": decision,
        "comment": comment,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });

    let mut line = serde_json::to_string(&entry).unwrap_or_default();
    line.push('\n');

    // Append atomically.
    if let Ok(mut f) = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&reviews_path)
        .await
    {
        use tokio::io::AsyncWriteExt;
        let _ = f.write_all(line.as_bytes()).await;
    }
}

#[derive(Deserialize, Validate)]
struct GenerateRequest {
    #[validate(
        length(min = 1),
        custom(function = "crate::extract::validate_non_blank")
    )]
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

    let state_for_task = Arc::clone(&state);
    let handle = tokio::spawn({
        let op_id = op_id.clone();
        async move {
            let success = match runtime.run_once(&workdir, &prompt).await {
                Ok(RunResult { success, .. }) => {
                    state_for_task.provider_health.record_success("default");
                    success
                }
                Err(err) => {
                    state_for_task.provider_health.record_failure("default");
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

/// Derive a status string from a task's completion state.
fn task_status(task: &PlanTask) -> &'static str {
    if task.completed {
        "completed"
    } else {
        "pending"
    }
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
            "status": task_status(t),
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
                output_text: None,
                usage: None,
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
        ).expect("AppState::new"));
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
        ).expect("AppState::new"));
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
