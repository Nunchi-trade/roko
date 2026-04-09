//! Status, health, metrics, dashboard, episodes, signals, and operation endpoints.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::serve::error::ApiError;
use crate::serve::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health))
        .route("/status", get(session_status))
        .route("/metrics", get(metrics))
        .route("/dashboard", get(dashboard))
        .route("/episodes", get(episodes))
        .route("/signals", get(signals))
        .route("/operations/{id}", get(operation_status))
}

/// `GET /api/health` — liveness check.
async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// `GET /api/status` — session status overview.
async fn session_status(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let ss = crate::status::SessionStatus::offline(state.workdir.clone());
    Ok(Json(json!({
        "session_id": ss.session_id,
        "workdir": ss.workdir,
        "daemon_running": ss.daemon_running,
        "signal_count": ss.signal_count,
        "episode_count": ss.episode_count,
        "last_episode_passed": ss.last_episode_passed,
    })))
}

/// `GET /api/metrics` — metric snapshots as JSON.
async fn metrics(State(state): State<Arc<AppState>>) -> Json<Value> {
    let snapshots = state.metrics.snapshot();
    Json(serde_json::to_value(snapshots).unwrap_or(json!([])))
}

/// `GET /api/dashboard` — dashboard scaffold as JSON.
async fn dashboard(State(state): State<Arc<AppState>>) -> Json<Value> {
    let scaffold = crate::tui::DashboardScaffold::new_in(&state.workdir);
    let text = format!("{scaffold:?}");
    Json(json!({ "rendered": text }))
}

/// `GET /api/episodes` — read episodes JSONL as a JSON array.
async fn episodes(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.layout.episodes_path();
    read_jsonl_array(&path).await
}

#[derive(Deserialize)]
struct SignalQuery {
    limit: Option<usize>,
}

/// `GET /api/signals` — read signals JSONL as a JSON array, with optional `?limit=N`.
async fn signals(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SignalQuery>,
) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko").join("signals.jsonl");
    let entries = read_jsonl_entries(&path).await?;
    let limited = match q.limit {
        Some(n) => entries
            .into_iter()
            .rev()
            .take(n)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
        None => entries,
    };
    Ok(Json(Value::Array(limited)))
}

/// `GET /api/operations/:id` — look up a background operation by ID.
async fn operation_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let ops = state.operations.read().await;
    let handle = ops
        .get(&id)
        .ok_or_else(|| ApiError::not_found("operation not found"))?;
    let result = Json(json!({
        "id": id,
        "kind": handle.kind,
        "status": format!("{:?}", handle.status),
    }));
    drop(ops);
    Ok(result)
}

// ── helpers ──────────────────────────────────────────────────────────

/// Read a JSONL file and return each line as a parsed `serde_json::Value`.
async fn read_jsonl_entries(path: &std::path::Path) -> Result<Vec<Value>, ApiError> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(ApiError::internal(format!("read {}: {e}", path.display()))),
    };
    let entries: Vec<Value> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    Ok(entries)
}

/// Read a JSONL file and return the entries as a `Json<Value::Array>`.
async fn read_jsonl_array(path: &std::path::Path) -> Result<Json<Value>, ApiError> {
    let entries = read_jsonl_entries(path).await?;
    Ok(Json(Value::Array(entries)))
}
