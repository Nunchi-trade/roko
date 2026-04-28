//! RuntimeProjection-backed dashboard run routes.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use roko_runtime::projection::{RunSummary, RuntimeProjection};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/dashboard/runs", get(get_dashboard_runs))
}

/// `GET /api/dashboard/runs` — summarize runs from the runtime event log.
async fn get_dashboard_runs(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.layout.root().join("runtime-events.jsonl");
    let summaries = RuntimeProjection::from_file(&path).unwrap_or_default();
    let mut runs: Vec<RunSummary> = summaries.into_values().collect();
    runs.sort_by(|left, right| left.run_id.cmp(&right.run_id));

    let runs: Vec<Value> = runs
        .into_iter()
        .map(|run| {
            json!({
                "run_id": run.run_id,
                "template": run.template,
                "prompt": run.prompt,
                "current_phase": run.current_phase,
                "phases_visited": run.phases_visited,
                "gates_passed": run.gates_passed,
                "gates_failed": run.gates_failed,
                "agents_spawned": run.agents_spawned,
                "is_complete": run.is_complete,
                "outcome": run.outcome,
            })
        })
        .collect();

    Ok(Json(json!({ "runs": runs })))
}
