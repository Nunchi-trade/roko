//! Diagnosis endpoint backed by the in-memory dashboard snapshot.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::state::AppState;
use roko_core::dashboard_snapshot::DiagnosisSummary;

/// Diagnosis routes backed by the in-memory dashboard snapshot.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/diagnosis/recent", get(recent))
}

/// `GET /api/diagnosis/recent` - recent conductor diagnoses from the state hub snapshot.
async fn recent(State(state): State<Arc<AppState>>) -> Json<Vec<DiagnosisSummary>> {
    let snapshot = state.state_hub.current_snapshot();
    Json(snapshot.diagnoses.iter().cloned().collect())
}
