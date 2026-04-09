//! Typed API error response for the HTTP server.
//!
//! Every handler returns `Result<T, ApiError>` so that error responses are
//! consistent JSON envelopes: `{ "error": { "code": "…", "message": "…" } }`.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// A structured error returned from any API endpoint.
#[derive(Debug, Serialize)]
pub struct ApiError {
    /// HTTP status code (not serialized — used only for the response status).
    #[serde(skip)]
    pub status: StatusCode,
    /// Machine-readable error code (e.g. `"not_found"`, `"bad_request"`).
    pub code: String,
    /// Human-readable description.
    pub message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "error": {
                "code": self.code,
                "message": self.message,
            }
        });
        (self.status, axum::Json(body)).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        Self::internal(err.to_string())
    }
}

impl ApiError {
    /// 404 Not Found.
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found".into(),
            message: msg.into(),
        }
    }

    /// 400 Bad Request.
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad_request".into(),
            message: msg.into(),
        }
    }

    /// 500 Internal Server Error.
    pub fn internal(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error".into(),
            message: msg.into(),
        }
    }

    /// 409 Conflict.
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "conflict".into(),
            message: msg.into(),
        }
    }

    /// 401 Unauthorized.
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized".into(),
            message: msg.into(),
        }
    }
}
