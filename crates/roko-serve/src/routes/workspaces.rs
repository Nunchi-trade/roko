//! Workspace CRUD endpoints.
//!
//! Provides routes for creating, querying, and deleting ephemeral workspace
//! directories used by demo scenarios and bench runs.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, State};
use axum::routing::{delete, get};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::state::{AppState, WorkspaceInfo, WorkspaceStatus};
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

fn run_git(dir: &std::path::Path, args: &[&str]) -> Result<(), String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|err| format!("spawn git {}: {err}", args.join(" ")))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(format!("git {} failed: {stderr}", args.join(" ")))
    }
}

fn init_git_repo(dir: &std::path::Path) -> Result<(), String> {
    run_git(dir, &["init"])?;
    run_git(dir, &["config", "user.email", "roko-demo@example.local"])?;
    run_git(dir, &["config", "user.name", "Roko Demo"])?;
    run_git(dir, &["add", "-A"])?;
    run_git(dir, &["commit", "-m", "workspace init", "--allow-empty"])?;
    Ok(())
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
        .route(
            "/workspaces/{id}",
            delete(delete_workspace).get(get_workspace_state),
        )
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
        // Intentionally ignoring: best-effort cleanup of partially-created workspace
        let _ = tokio::fs::remove_dir_all(&dir).await;
        return Err((
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("ensure dirs: {e}") })),
        ));
    }

    // Write the server's resolved config to the workspace so provider config,
    // env-var overrides, and secret interpolation are all captured.
    {
        let config = state.load_roko_config();
        match toml::to_string_pretty(&*config) {
            Ok(text) => {
                if let Err(e) = tokio::fs::write(dir.join("roko.toml"), text).await {
                    let _ = tokio::fs::remove_dir_all(&dir).await;
                    return Err((
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": format!("write roko.toml: {e}") })),
                    ));
                }
            }
            Err(e) => {
                let _ = tokio::fs::remove_dir_all(&dir).await;
                return Err((
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": format!("serialize config: {e}") })),
                ));
            }
        }
    }

    // Optionally initialise a git repo (same pattern as scaffold_bench_workdir).
    if body.git_init {
        let dir_clone = dir.clone();
        let git_result = tokio::task::spawn_blocking(move || init_git_repo(&dir_clone))
            .await
            .map_err(|err| format!("join git init task: {err}"))
            .and_then(|result| result);

        if let Err(e) = git_result {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            return Err((
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("init git repo: {e}") })),
            ));
        }
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let info = WorkspaceInfo {
        id: id.clone(),
        path: dir.clone(),
        created_at: now,
        last_accessed_at: now,
        status: WorkspaceStatus::Active,
    };
    if let Err(e) = state.insert_workspace(info).await {
        // Registry persistence failed — clean up directory best-effort.
        let _ = tokio::fs::remove_dir_all(&dir).await;
        return Err((
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("persist workspace registry: {e}") })),
        ));
    }

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
    let removed = match state.remove_workspace(&id).await {
        Ok(r) => r,
        Err(e) => {
            return Err((
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("persist workspace registry: {e}"), "id": id })),
            ));
        }
    };

    match removed {
        Some(ws) => {
            // Best-effort directory removal after registry persistence succeeded.
            if let Err(err) = tokio::fs::remove_dir_all(&ws.path).await {
                tracing::warn!(
                    workspace_id = %id,
                    path = %ws.path.display(),
                    error = %err,
                    "failed to remove ephemeral workspace directory"
                );
            }
            Ok(Json(json!({ "deleted": true, "id": id })))
        }
        None => Err((
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({ "error": "workspace not found", "id": id })),
        )),
    }
}

/// `GET /api/workspaces/:id` -- return a state dump for debugging failed plan runs.
///
/// Reads files from the workspace `.roko/` directory and returns whatever is
/// available. Missing files are silently skipped; read errors are collected in
/// the `errors` array so the caller always gets a 200 with partial data.
///
/// If the workspace path no longer exists, attempts to re-create it. If
/// re-creation fails, returns HTTP 410 Gone.
async fn get_workspace_state(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (axum::http::StatusCode, Json<Value>)> {
    let ws = state.get_workspace_info(&id).await;

    let ws = match ws {
        Some(ws) => ws,
        None => {
            return Err((
                axum::http::StatusCode::NOT_FOUND,
                Json(json!({ "error": "workspace not found", "id": id })),
            ));
        }
    };

    // Re-validate: if the path is missing, attempt to re-create it.
    let ws = if !ws.path.exists() {
        // Try to re-create the workspace directory and .roko layout.
        let recreated = tokio::fs::create_dir_all(&ws.path).await.is_ok();
        if recreated {
            let layout = RokoLayout::for_project(&ws.path);
            if layout.ensure_dirs().await.is_ok() {
                // Mark as Active, update last_accessed_at, persist.
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                {
                    let mut map = state.ephemeral_workspaces.write().await;
                    if let Some(entry) = map.get_mut(&id) {
                        entry.status = WorkspaceStatus::Active;
                        entry.last_accessed_at = now;
                    }
                }
                let _ = state.persist_workspace_registry().await;
                // Re-read the updated entry.
                state.get_workspace_info(&id).await.unwrap_or(ws)
            } else {
                // Re-creation of .roko layout failed — mark Stale and return 410.
                {
                    let mut map = state.ephemeral_workspaces.write().await;
                    if let Some(entry) = map.get_mut(&id) {
                        entry.status = WorkspaceStatus::Stale;
                    }
                }
                let _ = state.persist_workspace_registry().await;
                return Err((
                    axum::http::StatusCode::GONE,
                    Json(json!({
                        "error": "workspace path could not be recreated; create a new workspace",
                        "id": id,
                        "path": ws.path.display().to_string(),
                    })),
                ));
            }
        } else {
            // Directory creation failed — mark Stale and return 410.
            {
                let mut map = state.ephemeral_workspaces.write().await;
                if let Some(entry) = map.get_mut(&id) {
                    entry.status = WorkspaceStatus::Stale;
                }
            }
            let _ = state.persist_workspace_registry().await;
            return Err((
                axum::http::StatusCode::GONE,
                Json(json!({
                    "error": "workspace path could not be recreated; create a new workspace",
                    "id": id,
                    "path": ws.path.display().to_string(),
                })),
            ));
        }
    } else {
        // Path exists — just touch last_accessed_at.
        let _ = state.touch_workspace(&id).await;
        ws
    };

    let roko_dir = ws.path.join(".roko");
    let mut errors: Vec<String> = Vec::new();

    // 1. Executor state: .roko/state/executor.json
    let executor_state = match read_json_file(roko_dir.join("state").join("executor.json")).await {
        Ok(v) => v,
        Err(ReadFileError::NotFound) => Value::Null,
        Err(ReadFileError::Io(e)) => {
            errors.push(format!("executor.json: {e}"));
            Value::Null
        }
        Err(ReadFileError::Parse(e)) => {
            errors.push(format!("executor.json parse: {e}"));
            Value::Null
        }
    };

    // 2. Episodes: try canonical paths first, then legacy memory fallback.
    //    Order: .roko/episodes.jsonl -> .roko/learn/episodes.jsonl -> .roko/memory/episodes.jsonl
    let episodes = {
        let candidate_paths = [
            roko_dir.join("episodes.jsonl"),
            roko_dir.join("learn").join("episodes.jsonl"),
            // Legacy fallback — only used when canonical paths are missing.
            roko_dir.join("memory").join("episodes.jsonl"),
        ];
        let mut result = Value::Array(Vec::new());
        for path in candidate_paths {
            match read_jsonl_tail(path, 10).await {
                Ok(v) if !v.is_empty() => {
                    result = Value::Array(v);
                    break;
                }
                Ok(_) => continue,
                Err(ReadFileError::NotFound) => continue,
                Err(ReadFileError::Io(e)) => {
                    errors.push(format!("episodes.jsonl: {e}"));
                    break;
                }
                Err(ReadFileError::Parse(e)) => {
                    errors.push(format!("episodes.jsonl parse: {e}"));
                    break;
                }
            }
        }
        result
    };

    // 3. Plans: scan .roko/plans/ for subdirectories containing tasks.toml.
    //    Also check the workspace root for a plans/ directory (common layout
    //    when `roko plan run plans/` writes tasks.toml at workspace top-level).
    let plans = collect_plans(&roko_dir.join("plans"), &ws.path.join("plans"), &mut errors).await;

    // 4. Log tail: last 50 lines of .roko/roko.log
    let roko_log_tail = match read_text_tail(roko_dir.join("roko.log"), 50).await {
        Ok(text) => Value::String(text),
        Err(ReadFileError::NotFound) | Err(ReadFileError::Parse(_)) => Value::Null,
        Err(ReadFileError::Io(e)) => {
            errors.push(format!("roko.log: {e}"));
            Value::Null
        }
    };

    Ok(Json(json!({
        "workspace_id": ws.id,
        "workspace_path": ws.path.display().to_string(),
        "executor_state": executor_state,
        "episodes": episodes,
        "plans": plans,
        "roko_log_tail": roko_log_tail,
        "errors": errors,
    })))
}

// ---------------------------------------------------------------------------
// Internal file-reading helpers
// ---------------------------------------------------------------------------

/// Categorized file-read error.
enum ReadFileError {
    NotFound,
    Io(String),
    Parse(String),
}

/// Read a JSON file and return its parsed value.
async fn read_json_file(path: PathBuf) -> Result<Value, ReadFileError> {
    let data = match tokio::fs::read_to_string(&path).await {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ReadFileError::NotFound),
        Err(e) => return Err(ReadFileError::Io(e.to_string())),
    };
    serde_json::from_str(&data).map_err(|e| ReadFileError::Parse(e.to_string()))
}

/// Read the last `n` lines of a JSONL file, parsing each line as a JSON value.
async fn read_jsonl_tail(path: PathBuf, n: usize) -> Result<Vec<Value>, ReadFileError> {
    let data = match tokio::fs::read_to_string(&path).await {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ReadFileError::NotFound),
        Err(e) => return Err(ReadFileError::Io(e.to_string())),
    };

    let lines: Vec<&str> = data.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(n);
    let mut values = Vec::with_capacity(n);
    for line in &lines[start..] {
        match serde_json::from_str(line) {
            Ok(v) => values.push(v),
            Err(e) => return Err(ReadFileError::Parse(e.to_string())),
        }
    }
    Ok(values)
}

/// Read the last `n` lines of a text file.
async fn read_text_tail(path: PathBuf, n: usize) -> Result<String, ReadFileError> {
    let data = match tokio::fs::read_to_string(&path).await {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ReadFileError::NotFound),
        Err(e) => return Err(ReadFileError::Io(e.to_string())),
    };

    let lines: Vec<&str> = data.lines().collect();
    let start = lines.len().saturating_sub(n);
    Ok(lines[start..].join("\n"))
}

/// Scan one or more plan directories for `tasks.toml` files and return a JSON
/// object keyed by plan name.
async fn collect_plans(
    roko_plans: &std::path::Path,
    workspace_plans: &std::path::Path,
    errors: &mut Vec<String>,
) -> Value {
    let mut plans = serde_json::Map::new();

    for dir in [roko_plans, workspace_plans] {
        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(e) => e,
            Err(_) => continue, // directory doesn't exist — skip silently
        };

        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(e)) => e,
                Ok(None) => break,
                Err(e) => {
                    errors.push(format!("reading plans dir {}: {e}", dir.display()));
                    break;
                }
            };

            let entry_path = entry.path();
            if !entry_path.is_dir() {
                continue;
            }

            let tasks_path = entry_path.join("tasks.toml");
            let plan_name = entry.file_name().to_string_lossy().to_string();

            // Skip if we've already seen this plan name from the other directory.
            if plans.contains_key(&plan_name) {
                continue;
            }

            match tokio::fs::read_to_string(&tasks_path).await {
                Ok(contents) => {
                    let task_count = contents
                        .lines()
                        .filter(|l| l.trim().starts_with("[[task]]"))
                        .count();
                    plans.insert(
                        plan_name,
                        json!({
                            "tasks_toml": contents,
                            "task_count": task_count,
                        }),
                    );
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // Plan directory exists but no tasks.toml — skip.
                }
                Err(e) => {
                    errors.push(format!("reading {}: {e}", tasks_path.display()));
                }
            }
        }
    }

    Value::Object(plans)
}
