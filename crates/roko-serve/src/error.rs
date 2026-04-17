//! Typed API error response for the HTTP server.
//!
//! Every handler returns `Result<T, ApiError>` so that error responses are
//! consistent JSON objects: `{ "code": "…", "message": "…", "details": … }`.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use serde::Serialize;
use utoipa::ToSchema;
use validator::{ValidationErrors, ValidationErrorsKind};

/// A structured error returned from any API endpoint.
#[derive(Debug, Serialize, ToSchema)]
pub struct ApiError {
    /// HTTP status code (not serialized — used only for the response status).
    #[serde(skip)]
    pub status: StatusCode,
    /// Machine-readable error code (e.g. `"not_found"`, `"bad_request"`).
    pub code: String,
    /// Human-readable description.
    pub message: String,
    /// Optional structured debugging context for the client.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status;
        let body = serde_json::to_value(&self).unwrap_or_else(|error| {
            json!({
                "code": "internal_error",
                "message": format!("failed to serialize API error: {error}"),
            })
        });
        (status, axum::Json(body)).into_response()
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
            details: None,
        }
    }

    /// 400 Bad Request.
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad_request".into(),
            message: msg.into(),
            details: None,
        }
    }

    /// 500 Internal Server Error.
    pub fn internal(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error".into(),
            message: msg.into(),
            details: None,
        }
    }

    /// 409 Conflict.
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "conflict".into(),
            message: msg.into(),
            details: None,
        }
    }

    /// 401 Unauthorized.
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized".into(),
            message: msg.into(),
            details: None,
        }
    }

    /// JSON parse failure.
    pub fn parse(err: serde_json::Error) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_json".into(),
            message: "request body must be valid JSON".into(),
            details: Some(json!({ "reason": err.to_string() })),
        }
    }

    /// Validation failure for a decoded request body.
    pub fn validation(err: ValidationErrors) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "validation_error".into(),
            message: "request body validation failed".into(),
            details: Some(validation_errors_to_value(&err)),
        }
    }
}

/// Reject path segments that could escape their parent directory.
pub fn validate_path_segment(segment: &str, label: &str) -> Result<(), ApiError> {
    if segment.contains("..") || segment.contains('/') || segment.contains('\\') {
        return Err(ApiError::bad_request(format!(
            "invalid {label}: must not contain path separators or '..'"
        )));
    }
    Ok(())
}

fn validation_errors_to_value(errors: &ValidationErrors) -> Value {
    Value::Object(
        errors
            .errors()
            .iter()
            .map(|(field, kind)| ((*field).to_string(), validation_kind_to_value(kind)))
            .collect(),
    )
}

fn validation_kind_to_value(kind: &ValidationErrorsKind) -> Value {
    match kind {
        ValidationErrorsKind::Field(errors) => Value::Array(
            errors
                .iter()
                .map(|error| {
                    let mut body = serde_json::Map::new();
                    body.insert("code".into(), Value::String(error.code.to_string()));
                    if let Some(message) = &error.message {
                        body.insert("message".into(), Value::String(message.to_string()));
                    }
                    Value::Object(body)
                })
                .collect(),
        ),
        ValidationErrorsKind::Struct(errors) => validation_errors_to_value(errors),
        ValidationErrorsKind::List(errors) => Value::Object(
            errors
                .iter()
                .map(|(index, nested)| (index.to_string(), validation_errors_to_value(nested)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::to_bytes;

    #[tokio::test]
    async fn api_error_serializes_top_level_shape() {
        let response = ApiError::bad_request("nope").into_response();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let json: Value = serde_json::from_slice(&body).expect("parse body");

        assert_eq!(json["code"], "bad_request");
        assert_eq!(json["message"], "nope");
        assert!(json.get("details").is_none());
    }
}
