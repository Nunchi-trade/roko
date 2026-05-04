//! HTTP routes for vision-loop: start, status, cancel.
//!
//! The server spawns `roko vision-loop` as a subprocess (avoiding a circular
//! dependency on `roko-cli`). Status and cancellation are tracked in-process.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use roko_core::defaults::{
    DEFAULT_VISION_LOOP_MAX_ITERATIONS, DEFAULT_VISION_LOOP_TARGET_SCORE,
    DEFAULT_VISION_LOOP_VIEWPORT_HEIGHT, DEFAULT_VISION_LOOP_VIEWPORT_WIDTH,
    DEFAULT_VISION_LOOP_WAIT_MS,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::RwLock;

use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::extract::{RequestPayload, ValidJson};
use crate::state::AppState;

/// In-flight vision loop runs tracked by the server.
static VISION_LOOPS: std::sync::LazyLock<RwLock<HashMap<String, VisionLoopHandle>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

struct VisionLoopHandle {
    run_id: String,
    status: VisionLoopStatus,
    child: Option<tokio::process::Child>,
    result: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum VisionLoopStatus {
    Running,
    Completed,
    Cancelled,
    Failed,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/vision-loop", post(start_vision_loop))
        .route("/vision-loop/{run_id}/status", get(vision_loop_status))
        .route("/vision-loop/{run_id}/cancel", post(cancel_vision_loop))
}

#[derive(Debug, Deserialize)]
struct StartRequest {
    target_file: String,
    goal: String,
    url: String,
    #[serde(default = "default_max_iterations")]
    max_iterations: u32,
    #[serde(default = "default_target_score")]
    target_score: f64,
    #[serde(default)]
    model_key: Option<String>,
    #[serde(default = "default_viewport_width")]
    viewport_width: u32,
    #[serde(default = "default_viewport_height")]
    viewport_height: u32,
    #[serde(default = "default_wait_ms")]
    wait_ms: u64,
}

fn default_max_iterations() -> u32 {
    DEFAULT_VISION_LOOP_MAX_ITERATIONS
}
fn default_target_score() -> f64 {
    DEFAULT_VISION_LOOP_TARGET_SCORE
}
fn default_viewport_width() -> u32 {
    DEFAULT_VISION_LOOP_VIEWPORT_WIDTH
}
fn default_viewport_height() -> u32 {
    DEFAULT_VISION_LOOP_VIEWPORT_HEIGHT
}
fn default_wait_ms() -> u64 {
    DEFAULT_VISION_LOOP_WAIT_MS
}

impl RequestPayload for StartRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        if self.target_file.trim().is_empty() {
            return Err(ApiError::bad_request("target_file must not be blank"));
        }
        if self.goal.trim().is_empty() {
            return Err(ApiError::bad_request("goal must not be blank"));
        }
        if self.url.trim().is_empty() {
            return Err(ApiError::bad_request("url must not be blank"));
        }
        Ok(())
    }
}

/// `POST /api/vision-loop` — start a vision loop run in the background.
///
/// Spawns `roko vision-loop <args>` as a subprocess and tracks it.
async fn start_vision_loop(
    State(state): State<Arc<AppState>>,
    ValidJson(body): ValidJson<StartRequest>,
) -> Result<Json<Value>, ApiError> {
    let run_id = uuid::Uuid::new_v4().to_string();
    let bus = state.event_bus.clone();

    // Build CLI args.
    let mut args = vec![
        "vision-loop".to_string(),
        body.target_file.clone(),
        "--goal".to_string(),
        body.goal.clone(),
        "--url".to_string(),
        body.url.clone(),
        "--max-iter".to_string(),
        body.max_iterations.to_string(),
        "--target-score".to_string(),
        body.target_score.to_string(),
        "--viewport-width".to_string(),
        body.viewport_width.to_string(),
        "--viewport-height".to_string(),
        body.viewport_height.to_string(),
        "--wait-ms".to_string(),
        body.wait_ms.to_string(),
    ];
    if let Some(ref model) = body.model_key {
        args.push("--model".to_string());
        args.push(model.clone());
    }

    // Spawn the CLI subprocess.
    let child = tokio::process::Command::new("roko")
        .args(&args)
        .current_dir(&state.workdir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| ApiError::internal(format!("failed to spawn roko vision-loop: {e}")))?;

    // Register the handle.
    {
        let mut loops = VISION_LOOPS.write().await;
        loops.insert(
            run_id.clone(),
            VisionLoopHandle {
                run_id: run_id.clone(),
                status: VisionLoopStatus::Running,
                child: Some(child),
                result: None,
            },
        );
    }

    // Monitor the subprocess in the background.
    let spawned_run_id = run_id.clone();
    tokio::spawn(async move {
        // Take ownership of the child.
        let mut child = {
            let mut loops = VISION_LOOPS.write().await;
            loops.get_mut(&spawned_run_id).and_then(|h| h.child.take())
        };

        if let Some(ref mut c) = child {
            match c.wait().await {
                Ok(exit) if exit.success() => {
                    bus.publish(ServerEvent::VisionLoopCompleted {
                        run_id: spawned_run_id.clone(),
                        iterations: 0, // Not available from subprocess
                        best_score: 0.0,
                        stop_reason: "completed".to_string(),
                    });
                    let mut loops = VISION_LOOPS.write().await;
                    if let Some(handle) = loops.get_mut(&spawned_run_id) {
                        handle.status = VisionLoopStatus::Completed;
                        handle.result = Some(json!({ "status": "completed" }));
                    }
                }
                Ok(exit) => {
                    let code = exit.code().unwrap_or(-1);
                    bus.publish(ServerEvent::Error {
                        message: format!("vision loop exited with code {code}"),
                    });
                    let mut loops = VISION_LOOPS.write().await;
                    if let Some(handle) = loops.get_mut(&spawned_run_id) {
                        handle.status = VisionLoopStatus::Failed;
                        handle.result = Some(json!({ "error": format!("exit code {code}") }));
                    }
                }
                Err(e) => {
                    bus.publish(ServerEvent::Error {
                        message: format!("vision loop process error: {e}"),
                    });
                    let mut loops = VISION_LOOPS.write().await;
                    if let Some(handle) = loops.get_mut(&spawned_run_id) {
                        handle.status = VisionLoopStatus::Failed;
                        handle.result = Some(json!({ "error": e.to_string() }));
                    }
                }
            }
        }
    });

    Ok(Json(json!({
        "run_id": run_id,
        "status": "running",
    })))
}

/// `GET /api/vision-loop/:run_id/status` — check vision loop status.
async fn vision_loop_status(Path(run_id): Path<String>) -> Result<Json<Value>, ApiError> {
    let loops = VISION_LOOPS.read().await;
    let handle = loops
        .get(&run_id)
        .ok_or_else(|| ApiError::not_found(format!("vision loop {run_id} not found")))?;

    Ok(Json(json!({
        "run_id": handle.run_id,
        "status": handle.status,
        "result": handle.result,
    })))
}

/// `POST /api/vision-loop/:run_id/cancel` — cancel a running vision loop.
async fn cancel_vision_loop(Path(run_id): Path<String>) -> Result<Json<Value>, ApiError> {
    let mut loops = VISION_LOOPS.write().await;
    let handle = loops
        .get_mut(&run_id)
        .ok_or_else(|| ApiError::not_found(format!("vision loop {run_id} not found")))?;

    if handle.status != VisionLoopStatus::Running {
        return Err(ApiError::bad_request("vision loop is not running"));
    }

    // Kill the subprocess if still running.
    if let Some(ref mut child) = handle.child {
        let _ = child.kill().await;
    }
    handle.status = VisionLoopStatus::Cancelled;

    Ok(Json(json!({
        "run_id": run_id,
        "status": "cancelled",
    })))
}
