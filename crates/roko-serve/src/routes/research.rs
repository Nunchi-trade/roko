//! Research endpoints — topic research, PRD/plan/task enhancement, analysis.

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

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/research", get(list_research))
        .route("/research/topic", post(research_topic))
        .route("/research/enhance-prd/{slug}", post(enhance_prd))
        .route("/research/enhance-plan/{plan}", post(enhance_plan))
        .route("/research/enhance-tasks/{plan}", post(enhance_tasks))
        .route("/research/analyze", post(analyze))
}

/// `GET /api/research` — list research artifacts from `.roko/research/`.
async fn list_research(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let dir = state.workdir.join(".roko").join("research");
    if !dir.is_dir() {
        return Ok(Json(json!([])));
    }

    let mut artifacts = Vec::new();
    let mut rd = tokio::fs::read_dir(&dir)
        .await
        .map_err(|e| ApiError::internal(format!("read research dir: {e}")))?;

    while let Some(entry) = rd
        .next_entry()
        .await
        .map_err(|e| ApiError::internal(format!("read entry: {e}")))?
    {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let meta = tokio::fs::metadata(&path).await.ok();
            artifacts.push(json!({
                "name": name,
                "size": meta.as_ref().map(std::fs::Metadata::len),
                "is_file": path.is_file(),
            }));
        }
    }

    Ok(Json(Value::Array(artifacts)))
}

#[derive(Deserialize)]
struct TopicRequest {
    topic: String,
}

/// `POST /api/research/topic` — spawn background topic research.
async fn research_topic(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TopicRequest>,
) -> Result<impl IntoResponse, ApiError> {
    spawn_research_op(&state, "topic", &body.topic).await
}

/// `POST /api/research/enhance-prd/:slug` — enhance a PRD with research.
async fn enhance_prd(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    spawn_research_op(&state, "enhance_prd", &slug).await
}

/// `POST /api/research/enhance-plan/:plan` — optimize a plan with research.
async fn enhance_plan(
    State(state): State<Arc<AppState>>,
    Path(plan): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    spawn_research_op(&state, "enhance_plan", &plan).await
}

/// `POST /api/research/enhance-tasks/:plan` — split/optimize tasks.
async fn enhance_tasks(
    State(state): State<Arc<AppState>>,
    Path(plan): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    spawn_research_op(&state, "enhance_tasks", &plan).await
}

/// `POST /api/research/analyze` — analyze execution data.
async fn analyze(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, ApiError> {
    spawn_research_op(&state, "analyze", "execution_data").await
}

// ── helpers ──────────────────────────────────────────────────────────

/// Spawn a generic background research operation.
async fn spawn_research_op(
    state: &AppState,
    kind: &str,
    target: &str,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let op_id = uuid::Uuid::new_v4().to_string();
    let bus = state.event_bus.clone();
    let kind_str = kind.to_string();
    let target_str = target.to_string();

    let handle = tokio::spawn({
        let op_id = op_id.clone();
        let kind_str = kind_str.clone();
        async move {
            // TODO: Wire actual research agent execution.
            bus.publish(ServerEvent::OperationCompleted {
                op_id,
                kind: format!("research_{kind_str}"),
                success: true,
            });
        }
    });

    let op = OperationHandle {
        id: op_id.clone(),
        kind: format!("research_{kind_str}:{target_str}"),
        status: OperationStatus::Running,
        handle,
    };

    state.operations.write().await.insert(op_id.clone(), op);

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "id": op_id })),
    ))
}
