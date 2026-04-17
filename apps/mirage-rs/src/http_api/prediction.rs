//! Prediction HTTP endpoints.

use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::Deserialize;

use super::{ApiError, ApiState, MAX_LIMIT, PaginatedResponse, now_secs, with_cache_control};
use crate::chain::{PredictionError, PredictionEvent, SessionState};

fn default_limit() -> usize {
    20
}

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub question: String,
    pub creator: String,
    #[serde(default)]
    pub staked_points: u64,
    #[serde(default)]
    pub target_block: u64,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default)]
    pub context: String,
    #[serde(default = "default_metric")]
    pub metric: String,
}

#[derive(Debug, Deserialize)]
pub struct SessionListQuery {
    #[serde(default)]
    pub state: Option<SessionState>,
    #[serde(default)]
    pub creator: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

#[derive(Debug, Deserialize)]
pub struct ResolveSessionRequest {
    pub actual_value: f64,
}

#[derive(Debug, Deserialize)]
pub struct SubmitClaimRequest {
    pub session_id: String,
    pub agent_id: String,
    pub predicted_value: f64,
    pub interval_width: f64,
    pub confidence: f64,
    #[serde(default)]
    pub entries_in_context: Vec<String>,
    #[serde(default)]
    pub registered_block: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct ClaimListQuery {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_category() -> String {
    "general".to_string()
}

fn default_metric() -> String {
    "value".to_string()
}

/// `POST /api/predictions/sessions` — create a prediction session.
///
/// # Errors
///
/// Returns `400` if the question or creator is empty.
pub async fn create_session(
    State(state): State<ApiState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if req.question.trim().is_empty() {
        return Err(ApiError {
            error: "question must not be empty".into(),
            code: 400,
        });
    }
    if req.creator.trim().is_empty() {
        return Err(ApiError {
            error: "creator must not be empty".into(),
            code: 400,
        });
    }

    let now = now_secs();
    let mut chain = state.chain.write();
    let session_id = chain.prediction_store.create_session(
        req.question.clone(),
        req.creator.clone(),
        req.staked_points,
        req.target_block,
        req.category,
        req.context,
        req.metric,
        now,
    );
    let session = chain
        .prediction_store
        .get_session(&session_id)
        .cloned()
        .expect("new session exists");
    let _ = chain.prediction_bus.send(PredictionEvent::SessionCreated {
        session_id: session_id.clone(),
        question: req.question,
    });
    Ok(Json(serde_json::json!({
        "session": session,
    })))
}

/// `GET /api/predictions/sessions` — list prediction sessions.
pub async fn list_sessions(
    State(state): State<ApiState>,
    Query(query): Query<SessionListQuery>,
) -> impl IntoResponse {
    let limit = query.limit.min(MAX_LIMIT);
    let chain = state.chain.read();
    let (sessions, total) = chain.prediction_store.list_sessions(
        query.state,
        query.creator.as_deref(),
        limit,
        query.offset,
    );
    let items = sessions
        .into_iter()
        .map(|session| serde_json::to_value(session).unwrap_or_default())
        .collect();
    with_cache_control(PaginatedResponse::new(items, total, query.offset, limit), 2)
}

/// `GET /api/predictions/sessions/{id}` — fetch a session and its claims.
///
/// # Errors
///
/// Returns `404` if the session does not exist.
pub async fn get_session(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let chain = state.chain.read();
    let session = chain
        .prediction_store
        .get_session(&id)
        .ok_or_else(|| prediction_error_to_api(PredictionError::SessionNotFound(id.clone())))?;
    let claims: Vec<_> = session
        .claims
        .iter()
        .filter_map(|claim_id| chain.prediction_store.get_claim(claim_id))
        .cloned()
        .collect();
    Ok(Json(serde_json::json!({
        "session": session,
        "claims": claims,
    })))
}

/// `POST /api/predictions/sessions/{id}/resolve` — resolve a prediction session.
///
/// # Errors
///
/// Returns `400` if the actual value is not finite, `404` if the session does
/// not exist, or `409` if the session is already resolved.
pub async fn resolve_session(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<ResolveSessionRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let now = now_secs();
    let mut chain = state.chain.write();
    let result = chain
        .prediction_store
        .resolve_session(&id, req.actual_value, now)
        .map_err(prediction_error_to_api)?;
    let _ = chain.prediction_bus.send(PredictionEvent::SessionResolved {
        session_id: id.clone(),
        consensus_residual: result.mean_residual,
    });
    let session = chain
        .prediction_store
        .get_session(&id)
        .cloned()
        .expect("resolved session exists");
    Ok(Json(serde_json::json!({
        "ok": true,
        "result": result,
        "session": session,
    })))
}

/// `POST /api/predictions/claims` — submit a prediction claim.
///
/// # Errors
///
/// Returns `400` if `session_id` or `agent_id` is empty, `404` if the session
/// does not exist, `409` if the claim is duplicated or the session state is
/// incompatible, or `400` if the numeric fields are invalid.
pub async fn submit_claim(
    State(state): State<ApiState>,
    Json(req): Json<SubmitClaimRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if req.session_id.trim().is_empty() {
        return Err(ApiError {
            error: "session_id must not be empty".into(),
            code: 400,
        });
    }
    if req.agent_id.trim().is_empty() {
        return Err(ApiError {
            error: "agent_id must not be empty".into(),
            code: 400,
        });
    }

    let now = now_secs();
    let current_block = (state.current_block)();
    let mut chain = state.chain.write();
    let claim_id = chain
        .prediction_store
        .submit_claim(
            &req.session_id,
            req.agent_id.clone(),
            req.predicted_value,
            req.interval_width,
            req.confidence,
            req.entries_in_context,
            req.registered_block.unwrap_or(current_block),
            now,
        )
        .map_err(prediction_error_to_api)?;
    let claim = chain
        .prediction_store
        .get_claim(&claim_id)
        .cloned()
        .expect("new claim exists");
    let claim_count = chain
        .prediction_store
        .get_session(&req.session_id)
        .map_or(0, |session| session.claims.len());
    let session_state = chain
        .prediction_store
        .get_session(&req.session_id)
        .map(|session| session.state);

    let _ = chain.prediction_bus.send(PredictionEvent::ClaimSubmitted {
        session_id: req.session_id.clone(),
        agent_id: req.agent_id,
        confidence: claim.confidence,
    });
    if session_state == Some(SessionState::Registered) {
        let _ = chain
            .prediction_bus
            .send(PredictionEvent::SessionRegistered {
                session_id: req.session_id,
                claim_count,
            });
    }

    Ok(Json(serde_json::json!({
        "claim": claim,
    })))
}

/// `GET /api/predictions/claims` — list prediction claims.
pub async fn list_claims(
    State(state): State<ApiState>,
    Query(query): Query<ClaimListQuery>,
) -> impl IntoResponse {
    let limit = query.limit.min(MAX_LIMIT);
    let chain = state.chain.read();
    let (claims, total) = chain.prediction_store.list_claims(
        query.session_id.as_deref(),
        query.agent_id.as_deref(),
        limit,
        query.offset,
    );
    let items = claims
        .into_iter()
        .map(|claim| serde_json::to_value(claim).unwrap_or_default())
        .collect();
    with_cache_control(PaginatedResponse::new(items, total, query.offset, limit), 2)
}

/// `GET /api/predictions/calibration/{agent_id}` — summarize an agent's calibration.
pub async fn get_calibration(
    State(state): State<ApiState>,
    Path(agent_id): Path<String>,
) -> Json<serde_json::Value> {
    let chain = state.chain.read();
    let summary = chain.prediction_store.calibration_summary(&agent_id);
    Json(serde_json::to_value(summary).unwrap_or_default())
}

fn prediction_error_to_api(error: PredictionError) -> ApiError {
    match error {
        PredictionError::SessionNotFound(_) | PredictionError::ClaimNotFound(_) => ApiError {
            error: error.to_string(),
            code: 404,
        },
        PredictionError::InvalidSessionState { .. } | PredictionError::DuplicateClaim { .. } => {
            ApiError {
                error: error.to_string(),
                code: 409,
            }
        }
        PredictionError::Validation(_) => ApiError {
            error: error.to_string(),
            code: 400,
        },
    }
}
