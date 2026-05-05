//! Dashboard, session status, operation status, and truth map endpoints.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::state::AppState;
use roko_runtime::process::{ProcessSessionLedger, default_process_session_ledger_path};

/// `GET /api/dashboard` — dashboard scaffold as JSON.
pub async fn dashboard(State(state): State<Arc<AppState>>) -> Json<Value> {
    let info = state.runtime.dashboard_scaffold(&state.workdir);
    Json(json!({ "rendered": info.rendered }))
}

/// `GET /api/status` — session status overview.
pub async fn session_status(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let ss = state.runtime.session_status(state.workdir.clone());
    let supervised_processes = state.supervisor.snapshots().await;
    let process_ledger_path = default_process_session_ledger_path(&state.workdir);
    let process_sessions = {
        let ledger = ProcessSessionLedger::load(&process_ledger_path).map_err(|err| {
            ApiError::internal(format!(
                "load process session ledger {}: {err}",
                process_ledger_path.display()
            ))
        })?;
        // load() returns a default (empty) ledger when the file does not exist,
        // so we always have a valid summary — no separate existence check needed.
        Some(ledger.state_summary(Some(24 * 60 * 60 * 1_000), unix_ms()))
    };
    Ok(Json(json!({
        "session_id": ss.session_id,
        "workdir": ss.workdir,
        "daemon_running": ss.daemon_running,
        "signal_count": ss.signal_count,
        "episode_count": ss.episode_count,
        "last_episode_passed": ss.last_episode_passed,
        "supervised_processes": supervised_processes,
        "process_session_ledger": process_ledger_path,
        "process_sessions": process_sessions,
    })))
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis().min(u128::from(u64::MAX))).unwrap_or(u64::MAX)
        })
}

/// `GET /api/operations/:id` — look up a background operation by ID.
pub async fn operation_status(
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

/// `GET /api/truth_map` — return the entity truth-source registry.
pub async fn truth_map_handler() -> Json<Value> {
    Json(serde_json::to_value(crate::truth_map::truth_map()).unwrap_or_default())
}
