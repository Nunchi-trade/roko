//! Task HTTP endpoints for agent work coordination.

use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::Deserialize;

use super::{ApiError, ApiState, MAX_LIMIT, PaginatedResponse, now_secs, with_cache_control};
use crate::chain::agent::AgentStats;
use crate::chain::task::{CompletionMetadata, TaskArtifact, TaskError, TaskPriority, TaskState};

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

fn default_limit() -> usize {
    20
}

/// Filter/pagination parameters for `GET /api/tasks`.
#[derive(Debug, Deserialize)]
pub struct TaskListQuery {
    /// Filter by task state (open, assigned, in_progress, completed, failed, cancelled).
    #[serde(default)]
    pub state: Option<TaskState>,
    /// Filter by task kind (research, validate, analyze, monitor, report, …).
    #[serde(default)]
    pub kind: Option<String>,
    /// Filter by assignee agent ID.
    #[serde(default)]
    pub assignee: Option<String>,
    /// Maximum number of tasks to return (default 20, max 200).
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Offset into the result set (default 0).
    #[serde(default)]
    pub offset: usize,
}

// ---------------------------------------------------------------------------
// GET /api/tasks
// ---------------------------------------------------------------------------

/// `GET /api/tasks` — list tasks with optional filters and pagination.
pub async fn list_tasks(
    State(state): State<ApiState>,
    Query(query): Query<TaskListQuery>,
) -> impl IntoResponse {
    let limit = query.limit.min(MAX_LIMIT);
    let chain = state.chain.read();
    let (tasks, total) = chain.task_store.list(
        query.state,
        query.kind.as_deref(),
        query.assignee.as_deref(),
        limit,
        query.offset,
    );
    let items: Vec<serde_json::Value> = tasks
        .into_iter()
        .map(|t| serde_json::to_value(t).unwrap_or_default())
        .collect();
    with_cache_control(PaginatedResponse::new(items, total, query.offset, limit), 2)
}

// ---------------------------------------------------------------------------
// GET /api/tasks/stats
// ---------------------------------------------------------------------------

/// `GET /api/tasks/stats` — aggregate task counts and totals.
pub async fn task_stats(State(state): State<ApiState>) -> impl IntoResponse {
    let chain = state.chain.read();
    let stats = chain.task_store.stats();
    with_cache_control(
        serde_json::json!({
            "open": stats.open,
            "assigned": stats.assigned,
            "in_progress": stats.in_progress,
            "completed": stats.completed,
            "failed": stats.failed,
            "cancelled": stats.cancelled,
            "total_stake_wei": stats.total_stake_wei,
            "total_reward_wei": stats.total_reward_wei,
        }),
        2,
    )
}

// ---------------------------------------------------------------------------
// GET /api/tasks/{id}
// ---------------------------------------------------------------------------

/// `GET /api/tasks/{id}` — get a single task by ID.
pub async fn get_task(
    State(state): State<ApiState>,
    Path(id): Path<u64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let chain = state.chain.read();
    match chain.task_store.get(id) {
        Some(task) => Ok(Json(serde_json::to_value(task).unwrap_or_default())),
        None => Err(ApiError {
            error: format!("task not found: {id}"),
            code: 404,
        }),
    }
}

// ---------------------------------------------------------------------------
// POST /api/tasks
// ---------------------------------------------------------------------------

/// Request body for `POST /api/tasks`.
#[derive(Debug, Deserialize)]
pub struct CreateTaskRequest {
    /// Short human-readable title (required).
    pub title: String,
    /// Detailed description (optional, defaults to empty).
    #[serde(default)]
    pub description: String,
    /// Task kind: "research", "validate", "analyze", "monitor", "report" (required).
    pub kind: String,
    /// Priority level (optional, defaults to medium).
    #[serde(default = "default_priority")]
    pub priority: TaskPriority,
    /// Agent ID creating the task (required).
    pub creator: String,
    /// Topic tags for matching (optional).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Stake to deposit in wei (optional, defaults to 0).
    #[serde(default)]
    pub stake_wei: u128,
}

fn default_priority() -> TaskPriority {
    TaskPriority::Medium
}

/// `POST /api/tasks` — create a new task.
pub async fn create_task(
    State(state): State<ApiState>,
    Json(req): Json<CreateTaskRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if req.title.is_empty() {
        return Err(ApiError {
            error: "title must not be empty".into(),
            code: 400,
        });
    }
    if req.kind.is_empty() {
        return Err(ApiError {
            error: "kind must not be empty".into(),
            code: 400,
        });
    }
    if req.creator.is_empty() {
        return Err(ApiError {
            error: "creator must not be empty".into(),
            code: 400,
        });
    }

    let now = now_secs();
    let mut chain = state.chain.write();
    let id = chain.task_store.create(
        req.title.clone(),
        req.description,
        req.kind.clone(),
        req.priority,
        req.creator.clone(),
        req.tags,
        req.stake_wei,
        now,
    );

    let _ = chain.task_bus.send(crate::chain::TaskEvent::Created {
        id,
        title: req.title.clone(),
        kind: req.kind.clone(),
        creator: req.creator.clone(),
    });

    Ok(Json(serde_json::json!({
        "id": id,
        "title": req.title,
        "kind": req.kind,
        "creator": req.creator,
        "created_at": now,
    })))
}

// ---------------------------------------------------------------------------
// POST /api/tasks/{id}/assign
// ---------------------------------------------------------------------------

/// Request body for `POST /api/tasks/{id}/assign`.
#[derive(Debug, Deserialize)]
pub struct AssignTaskRequest {
    /// Agent ID to assign the task to (required).
    pub assignee: String,
}

/// `POST /api/tasks/{id}/assign` — assign a task to an agent.
pub async fn assign_task(
    State(state): State<ApiState>,
    Path(id): Path<u64>,
    Json(req): Json<AssignTaskRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if req.assignee.is_empty() {
        return Err(ApiError {
            error: "assignee must not be empty".into(),
            code: 400,
        });
    }

    let now = now_secs();
    let mut chain = state.chain.write();
    chain
        .task_store
        .assign(id, req.assignee.clone(), now)
        .map_err(task_error_to_api)?;

    let _ = chain.task_bus.send(crate::chain::TaskEvent::Assigned {
        id,
        assignee: req.assignee.clone(),
    });

    Ok(Json(serde_json::json!({
        "ok": true,
        "id": id,
        "assignee": req.assignee,
        "assigned_at": now,
    })))
}

// ---------------------------------------------------------------------------
// POST /api/tasks/{id}/start
// ---------------------------------------------------------------------------

/// `POST /api/tasks/{id}/start` — mark a task as in-progress.
pub async fn start_task(
    State(state): State<ApiState>,
    Path(id): Path<u64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let now = now_secs();
    let mut chain = state.chain.write();
    chain.task_store.start(id, now).map_err(task_error_to_api)?;

    let assignee = chain
        .task_store
        .get(id)
        .and_then(|t| t.assignee.clone())
        .unwrap_or_default();

    let _ = chain.task_bus.send(crate::chain::TaskEvent::Started {
        id,
        assignee: assignee.clone(),
    });

    Ok(Json(serde_json::json!({
        "ok": true,
        "id": id,
        "assignee": assignee,
        "started_at": now,
    })))
}

// ---------------------------------------------------------------------------
// POST /api/tasks/{id}/complete
// ---------------------------------------------------------------------------

/// Request body for `POST /api/tasks/{id}/complete`.
#[derive(Debug, Deserialize)]
pub struct CompleteTaskRequest {
    /// Optional insight ID produced as a result of the task.
    #[serde(default)]
    pub result_insight_id: Option<String>,
    /// Task deliverables recorded on completion.
    #[serde(default)]
    pub artifacts: Vec<TaskArtifact>,
    /// Human-readable completion summary.
    #[serde(default)]
    pub summary: Option<String>,
    /// Runtime metadata recorded on completion.
    #[serde(default)]
    pub completion_metadata: Option<CompletionMetadata>,
}

/// `POST /api/tasks/{id}/complete` — complete a task with optional result insight.
pub async fn complete_task(
    State(state): State<ApiState>,
    Path(id): Path<u64>,
    Json(req): Json<CompleteTaskRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let now = now_secs();
    let mut chain = state.chain.write();
    let reward = chain
        .task_store
        .complete(
            id,
            req.result_insight_id.clone(),
            req.artifacts.clone(),
            req.summary.clone(),
            req.completion_metadata.clone(),
            now,
        )
        .map_err(task_error_to_api)?;

    let assignee = chain
        .task_store
        .get(id)
        .and_then(|t| t.assignee.clone())
        .unwrap_or_default();

    if !assignee.is_empty() {
        chain.agent_registry.add_stats_delta(
            &assignee,
            &AgentStats {
                tasks_completed: 1,
                ..AgentStats::default()
            },
        );
    }

    let _ = chain.task_bus.send(crate::chain::TaskEvent::Completed {
        id,
        assignee: assignee.clone(),
        result_insight_id: req.result_insight_id,
    });

    Ok(Json(serde_json::json!({
        "ok": true,
        "id": id,
        "assignee": assignee,
        "reward_wei": reward,
        "completed_at": now,
    })))
}

/// `GET /api/tasks/{id}/artifacts` — list completion artifacts for a task.
pub async fn get_task_artifacts(
    State(state): State<ApiState>,
    Path(id): Path<u64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let chain = state.chain.read();
    match chain.task_store.get(id) {
        Some(task) => Ok(Json(serde_json::json!({
            "task_id": id,
            "artifacts": task.artifacts,
            "summary": task.summary,
            "completion_metadata": task.completion_metadata,
        }))),
        None => Err(ApiError {
            error: format!("task not found: {id}"),
            code: 404,
        }),
    }
}

// ---------------------------------------------------------------------------
// POST /api/tasks/{id}/improve
// ---------------------------------------------------------------------------

/// Request body for `POST /api/tasks/{id}/improve`.
#[derive(Debug, Deserialize)]
pub struct ImproveTaskRequest {
    /// Requested revision or follow-up direction.
    pub feedback: String,
    /// Actor requesting the improvement loop.
    pub creator: String,
}

/// `POST /api/tasks/{id}/improve` — create a child improvement task.
pub async fn improve_task(
    State(state): State<ApiState>,
    Path(id): Path<u64>,
    Json(req): Json<ImproveTaskRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if req.feedback.trim().is_empty() {
        return Err(ApiError {
            error: "feedback must not be empty".into(),
            code: 400,
        });
    }
    if req.creator.trim().is_empty() {
        return Err(ApiError {
            error: "creator must not be empty".into(),
            code: 400,
        });
    }

    let now = now_secs();
    let mut chain = state.chain.write();
    let child_id = chain
        .task_store
        .create_improvement(id, req.feedback, req.creator.clone(), now)
        .map_err(task_error_to_api)?;

    let _ = chain.task_bus.send(crate::chain::TaskEvent::Created {
        id: child_id,
        title: format!("Improvement on task #{id}"),
        kind: "improvement".to_string(),
        creator: req.creator,
    });

    Ok(Json(serde_json::json!({
        "ok": true,
        "parent_task_id": id,
        "improvement_task_id": child_id,
        "created_at": now,
    })))
}

// ---------------------------------------------------------------------------
// POST /api/tasks/{id}/fail
// ---------------------------------------------------------------------------

/// Request body for `POST /api/tasks/{id}/fail`.
#[derive(Debug, Deserialize)]
pub struct FailTaskRequest {
    /// Reason for failure (required).
    pub reason: String,
}

/// `POST /api/tasks/{id}/fail` — fail a task with a reason.
pub async fn fail_task(
    State(state): State<ApiState>,
    Path(id): Path<u64>,
    Json(req): Json<FailTaskRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if req.reason.is_empty() {
        return Err(ApiError {
            error: "reason must not be empty".into(),
            code: 400,
        });
    }

    let now = now_secs();
    let assignee = {
        let chain = state.chain.read();
        chain
            .task_store
            .get(id)
            .and_then(|t| t.assignee.clone())
            .unwrap_or_default()
    };

    let mut chain = state.chain.write();
    chain
        .task_store
        .fail(id, req.reason.clone(), now)
        .map_err(task_error_to_api)?;

    if !assignee.is_empty() {
        chain.agent_registry.add_stats_delta(
            &assignee,
            &AgentStats {
                tasks_failed: 1,
                ..AgentStats::default()
            },
        );
    }

    let new_state = chain
        .task_store
        .get(id)
        .map(|t| t.state)
        .unwrap_or(crate::chain::TaskState::Failed);

    let _ = chain.task_bus.send(crate::chain::TaskEvent::Failed {
        id,
        assignee: assignee.clone(),
        reason: req.reason.clone(),
    });

    Ok(Json(serde_json::json!({
        "ok": true,
        "id": id,
        "assignee": assignee,
        "reason": req.reason,
        "new_state": new_state,
    })))
}

// ---------------------------------------------------------------------------
// POST /api/tasks/{id}/cancel
// ---------------------------------------------------------------------------

/// Request body for `POST /api/tasks/{id}/cancel`.
#[derive(Debug, Deserialize)]
pub struct CancelTaskRequest {
    /// Reason for cancellation (required).
    pub reason: String,
}

/// `POST /api/tasks/{id}/cancel` — cancel a task with a reason.
pub async fn cancel_task(
    State(state): State<ApiState>,
    Path(id): Path<u64>,
    Json(req): Json<CancelTaskRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if req.reason.is_empty() {
        return Err(ApiError {
            error: "reason must not be empty".into(),
            code: 400,
        });
    }

    let mut chain = state.chain.write();
    chain
        .task_store
        .cancel(id, req.reason.clone())
        .map_err(task_error_to_api)?;

    let _ = chain.task_bus.send(crate::chain::TaskEvent::Cancelled {
        id,
        reason: req.reason.clone(),
    });

    Ok(Json(serde_json::json!({
        "ok": true,
        "id": id,
        "reason": req.reason,
    })))
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

fn task_error_to_api(e: TaskError) -> ApiError {
    match &e {
        TaskError::NotFound(_) => ApiError {
            error: e.to_string(),
            code: 404,
        },
        TaskError::InvalidState { .. } => ApiError {
            error: e.to_string(),
            code: 409,
        },
        TaskError::AlreadyAssigned(_) => ApiError {
            error: e.to_string(),
            code: 409,
        },
        TaskError::MaxAttempts(_) => ApiError {
            error: e.to_string(),
            code: 409,
        },
        TaskError::ImprovementTargetUnassigned(_) => ApiError {
            error: e.to_string(),
            code: 409,
        },
    }
}
