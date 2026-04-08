//! Request handlers for the worker HTTP server.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, info};

use crate::config::Config;
use crate::serve::templates::TemplateRegistry;

use super::WorkerState;

/// Convert elapsed milliseconds to u64, saturating at `u64::MAX`.
fn elapsed_ms(start: std::time::Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Result of a task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    /// Whether the agent and all gates passed.
    pub success: bool,
    /// Episode ID from the run report, if available.
    pub episode_id: Option<String>,
    /// Per-gate verdicts: (gate name, passed).
    pub gate_verdicts: Vec<(String, bool)>,
    /// Error message, if the run failed outright.
    pub error: Option<String>,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
}

/// Payload for `POST /task`.
#[derive(Deserialize)]
struct TaskRequest {
    /// Parameter values to interpolate into the template prompt.
    #[serde(default)]
    params: HashMap<String, String>,
}

/// Build the axum router for the worker server.
pub fn build_router(state: Arc<WorkerState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/task", post(run_task))
        .route("/status", get(status))
        .with_state(state)
}

/// `GET /health` — liveness check.
async fn health(State(state): State<Arc<WorkerState>>) -> impl IntoResponse {
    let uptime = state.started_at.elapsed().as_secs();
    Json(json!({
        "status": "ok",
        "template": state.template.name,
        "uptime_secs": uptime,
    }))
}

/// `GET /status` — worker status including last task result.
async fn status(State(state): State<Arc<WorkerState>>) -> impl IntoResponse {
    let last_task = state.last_task.read().await.clone();
    Json(json!({
        "template": state.template.name,
        "model": state.template.agent.model,
        "last_task": last_task,
        "uptime_secs": state.started_at.elapsed().as_secs(),
    }))
}

/// `POST /task` — execute the template with the given parameters.
async fn run_task(
    State(state): State<Arc<WorkerState>>,
    Json(req): Json<TaskRequest>,
) -> impl IntoResponse {
    info!(template = %state.template.name, params = ?req.params, "executing task");

    let start = std::time::Instant::now();

    // Interpolate params into template prompt
    let prompt = TemplateRegistry::render_prompt(&state.template, &req.params);

    // Create temp workdir
    let work_id = uuid::Uuid::new_v4();
    let workdir = std::env::temp_dir().join(format!("roko-worker-{work_id}"));
    if let Err(e) = std::fs::create_dir_all(&workdir) {
        let result = TaskResult {
            success: false,
            episode_id: None,
            gate_verdicts: Vec::new(),
            error: Some(format!("failed to create workdir: {e}")),
            duration_ms: elapsed_ms(start),
        };
        *state.last_task.write().await = Some(result.clone());
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::to_value(result).unwrap_or_default()),
        );
    }

    // Build a Config from the template's agent settings
    let mut config = Config::default();
    config.agent.command = state.template.agent.command.clone();
    config.agent.model = Some(state.template.agent.model.clone());

    // Run the universal loop
    let result = match crate::run::run_once(&workdir, &config, &prompt).await {
        Ok(report) => TaskResult {
            success: report.overall_success(),
            episode_id: Some(report.episode_id),
            gate_verdicts: report.gate_verdicts,
            error: None,
            duration_ms: elapsed_ms(start),
        },
        Err(e) => TaskResult {
            success: false,
            episode_id: None,
            gate_verdicts: Vec::new(),
            error: Some(format!("{e:#}")),
            duration_ms: elapsed_ms(start),
        },
    };

    *state.last_task.write().await = Some(result.clone());

    // Callback to control plane if configured
    if let (Some(url), Some(dep_id)) =
        (&state.control_plane_url, &state.deployment_id)
    {
        let callback_url = format!("{url}/api/deployments/{dep_id}/callback");
        let client = reqwest::Client::new();
        if let Err(e) = client.post(&callback_url).json(&result).send().await {
            error!(%callback_url, error = %e, "callback to control plane failed");
        }
    }

    // Clean up temp workdir
    let _ = std::fs::remove_dir_all(&workdir);

    let status_code = if result.success {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    };

    (
        status_code,
        Json(serde_json::to_value(result).unwrap_or_default()),
    )
}
