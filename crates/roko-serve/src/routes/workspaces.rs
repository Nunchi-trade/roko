//! Workspace CRUD endpoints.
//!
//! Provides routes for creating, querying, and deleting ephemeral workspace
//! directories used by demo scenarios and bench runs.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, State};
use axum::routing::{delete, get};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::state::{AppState, WorkspaceInfo};
use roko_fs::layout::RokoLayout;

/// Request body for `POST /api/workspaces`.
#[derive(Debug, Deserialize)]
pub struct CreateWorkspaceRequest {
    /// Directory name prefix (e.g. `"roko-demo"`).
    #[serde(default = "default_prefix")]
    pub prefix: String,
    /// Whether to initialise a git repository in the workspace.
    #[serde(default)]
    pub git_init: bool,
}

fn default_prefix() -> String {
    "roko-ws".to_string()
}

/// Response body for `POST /api/workspaces`.
#[derive(Debug, Serialize)]
pub struct CreateWorkspaceResponse {
    pub id: String,
    pub path: String,
    pub ready: bool,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/workspaces", get(list_workspaces).post(create_workspace))
        .route("/workspaces/default", get(get_default_workspace))
        .route("/workspaces/{id}", delete(delete_workspace))
}

/// `GET /api/workspaces` -- list all tracked ephemeral workspaces.
async fn list_workspaces(State(state): State<Arc<AppState>>) -> Json<Value> {
    let map = state.ephemeral_workspaces.read().await;
    let workspaces: Vec<Value> = map
        .values()
        .map(|ws| {
            json!({
                "id": ws.id,
                "path": ws.path.display().to_string(),
                "created_at": ws.created_at,
            })
        })
        .collect();
    Json(json!({ "workspaces": workspaces }))
}

/// `POST /api/workspaces` -- create an ephemeral workspace directory.
async fn create_workspace(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateWorkspaceRequest>,
) -> Result<Json<Value>, (axum::http::StatusCode, Json<Value>)> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let id = format!("{}-{millis}", body.prefix);
    let dir = std::env::temp_dir().join(&id);

    // Create the directory tree and .roko/ layout.
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        return Err((
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("create dir: {e}") })),
        ));
    }

    let layout = RokoLayout::for_project(&dir);
    if let Err(e) = layout.ensure_dirs().await {
        let _ = tokio::fs::remove_dir_all(&dir).await;
        return Err((
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("ensure dirs: {e}") })),
        ));
    }

    // Copy roko.toml from the server workspace so provider config is available.
    let server_toml = state.workdir.join("roko.toml");
    if tokio::fs::try_exists(&server_toml)
        .await
        .unwrap_or(false)
    {
        let _ = tokio::fs::copy(&server_toml, dir.join("roko.toml")).await;
    }

    // Optionally initialise a git repo (same pattern as scaffold_bench_workdir).
    if body.git_init {
        let dir_clone = dir.clone();
        tokio::task::spawn_blocking(move || {
            for args in [
                &["init"][..],
                &["add", "-A"][..],
                &["commit", "-m", "workspace init", "--allow-empty"][..],
            ] {
                let _ = std::process::Command::new("git")
                    .args(args)
                    .current_dir(&dir_clone)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
        })
        .await
        .ok();
    }

    let info = WorkspaceInfo {
        id: id.clone(),
        path: dir.clone(),
        created_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    state
        .ephemeral_workspaces
        .write()
        .await
        .insert(id.clone(), info);

    Ok(Json(json!({
        "id": id,
        "path": dir.display().to_string(),
        "ready": true,
    })))
}

/// `GET /api/workspaces/default` -- return the server's working directory.
async fn get_default_workspace(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({ "path": state.workdir.display().to_string() }))
}

/// `DELETE /api/workspaces/:id` -- remove an ephemeral workspace.
async fn delete_workspace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (axum::http::StatusCode, Json<Value>)> {
    let info = {
        let mut map = state.ephemeral_workspaces.write().await;
        map.remove(&id)
    };

    match info {
        Some(ws) => {
            let _ = tokio::fs::remove_dir_all(&ws.path).await;
            Ok(Json(json!({ "deleted": true, "id": id })))
        }
        None => Err((
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({ "error": "workspace not found", "id": id })),
        )),
    }
}
