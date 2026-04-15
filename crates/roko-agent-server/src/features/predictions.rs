//! Prediction storage and retrieval routes.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};

use crate::state::{AgentPrediction, AgentState, PredictionCreateRequest};

/// Prediction routes.
pub fn router() -> Router<Arc<AgentState>> {
    Router::new()
        .route("/predictions", get(list_predictions).post(create_prediction))
        .route("/predictions/residuals", get(prediction_residuals))
        .route("/predictions/{id}", get(get_prediction))
}

async fn list_predictions(State(state): State<Arc<AgentState>>) -> Json<Vec<AgentPrediction>> {
    Json(state.list_predictions().await)
}

async fn create_prediction(
    State(state): State<Arc<AgentState>>,
    Json(request): Json<PredictionCreateRequest>,
) -> impl IntoResponse {
    let prediction = state.create_prediction(request).await;
    (StatusCode::OK, Json(prediction))
}

async fn get_prediction(
    State(state): State<Arc<AgentState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    state
        .get_prediction(&id)
        .await
        .map_or_else(|| StatusCode::NOT_FOUND.into_response(), |prediction| Json(prediction).into_response())
}

async fn prediction_residuals(State(state): State<Arc<AgentState>>) -> Json<serde_json::Value> {
    Json(state.prediction_residuals().await)
}
