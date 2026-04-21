//! Shared API auth and scrubbing middleware for `/api/*` routes.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::Request;
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use roko_core::config::ServeAuthConfig;
use roko_core::obs::LogScrubber;
use tower_http::cors::{Any, CorsLayer};

use crate::error::ApiError;

enum ApiCredential<'a> {
    Missing,
    XApiKey(&'a str),
    InvalidXApiKey,
    Bearer(&'a str),
    InvalidAuthorization,
}

fn api_credential(headers: &HeaderMap) -> ApiCredential<'_> {
    if let Some(value) = headers.get("X-Api-Key") {
        return match value.to_str() {
            Ok(value) => ApiCredential::XApiKey(value),
            Err(_) => ApiCredential::InvalidXApiKey,
        };
    }

    if let Some(value) = headers.get(AUTHORIZATION) {
        return match value.to_str() {
            Ok(value) => match value.strip_prefix("Bearer ") {
                Some(token) => ApiCredential::Bearer(token),
                None => ApiCredential::InvalidAuthorization,
            },
            Err(_) => ApiCredential::InvalidAuthorization,
        };
    }

    ApiCredential::Missing
}

/// Require a matching API credential for the request to continue.
///
/// `X-Api-Key` retains precedence when both auth headers are present so
/// credential resolution stays deterministic for existing clients.
pub async fn require_api_key(
    State(auth): State<ServeAuthConfig>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    match api_credential(req.headers()) {
        ApiCredential::XApiKey(supplied) if supplied == auth.api_key => {}
        ApiCredential::Bearer(supplied) if supplied == auth.api_key => {}
        ApiCredential::XApiKey(_) | ApiCredential::InvalidXApiKey => {
            return Err(ApiError::unauthorized(
                "invalid or missing X-Api-Key header",
            ));
        }
        ApiCredential::Bearer(_) | ApiCredential::InvalidAuthorization => {
            return Err(ApiError::unauthorized(
                "invalid or missing Authorization bearer token",
            ));
        }
        ApiCredential::Missing => {
            return Err(ApiError::unauthorized(
                "missing X-Api-Key header or Authorization bearer token",
            ));
        }
    }

    Ok(next.run(req).await)
}

/// Build the CORS layer from configured origins.
pub fn cors_layer(cors_origins: &[String]) -> CorsLayer {
    if cors_origins.is_empty() {
        CorsLayer::permissive()
    } else {
        let allowed: Vec<axum::http::HeaderValue> =
            cors_origins.iter().filter_map(|o| o.parse().ok()).collect();
        CorsLayer::new()
            .allow_origin(allowed)
            .allow_methods(Any)
            .allow_headers(Any)
    }
}

/// Returns `true` when the response content-type indicates a text-like body
/// that should be scrubbed for secrets.
fn is_scrubbable_content_type(response: &Response) -> bool {
    let Some(ct) = response.headers().get(axum::http::header::CONTENT_TYPE) else {
        // No content-type — assume JSON (axum default for Json responses).
        return true;
    };
    let Ok(ct_str) = ct.to_str() else {
        return false;
    };
    let ct_lower = ct_str.to_ascii_lowercase();
    ct_lower.contains("json")
        || ct_lower.contains("text/")
        || ct_lower.contains("javascript")
        || ct_lower.contains("xml")
}

/// Axum middleware that scrubs secret patterns from text/JSON response bodies.
///
/// Binary or image responses are passed through unchanged.
/// Uses the shared [`LogScrubber`] stored in `AppState.scrubber`.
pub async fn scrub_secrets(
    State(scrubber): State<Arc<LogScrubber>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let response = next.run(req).await;

    if !is_scrubbable_content_type(&response) {
        return response;
    }

    let (parts, body) = response.into_parts();

    // Collect the full body. On error (e.g. connection drop) return an
    // empty 500 rather than leaking unscrubbed partial data.
    // Cap at 16 MiB to avoid unbounded memory growth.
    let Ok(bytes) = axum::body::to_bytes(body, 16 * 1024 * 1024).await else {
        return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };

    // Fast path: if the body is empty or not valid UTF-8, pass through.
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Response::from_parts(parts, Body::from(bytes));
    };

    if text.is_empty() {
        return Response::from_parts(parts, Body::from(bytes));
    }

    let redacted = scrubber.scrub(text);

    // Avoid an allocation when nothing was redacted.
    if redacted.len() == text.len() && redacted == text {
        return Response::from_parts(parts, Body::from(bytes));
    }

    Response::from_parts(parts, Body::from(redacted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::http::StatusCode;
    use axum::http::header::AUTHORIZATION;
    use axum::http::header::CONTENT_TYPE;
    use axum::routing::get;
    use roko_core::config::ServeAuthConfig;
    use serde_json::Value;
    use tower::ServiceExt;

    /// Build a test router that echoes the provided body, with the scrub
    /// middleware wired in.
    fn test_app(scrubber: Arc<LogScrubber>, body: &'static str) -> Router {
        let handler = move || async move { body.to_string() };
        Router::new()
            .route("/test", get(handler))
            .layer(axum::middleware::from_fn_with_state(
                scrubber,
                scrub_secrets,
            ))
    }

    fn test_app_json(scrubber: Arc<LogScrubber>, body: &'static str) -> Router {
        let handler =
            move || async move { axum::Json(serde_json::Value::String(body.to_string())) };
        Router::new()
            .route("/test", get(handler))
            .layer(axum::middleware::from_fn_with_state(
                scrubber,
                scrub_secrets,
            ))
    }

    fn auth_test_app(auth: ServeAuthConfig) -> Router {
        Router::new()
            .route("/test", get(|| async { StatusCode::NO_CONTENT }))
            .layer(axum::middleware::from_fn_with_state(auth, require_api_key))
    }

    async fn auth_response(
        app: Router,
        build: impl FnOnce(axum::http::request::Builder) -> axum::http::request::Builder,
    ) -> Response {
        let req = build(Request::builder().uri("/test"))
            .body(Body::empty())
            .expect("invariant: auth test request builds");
        app.oneshot(req)
            .await
            .expect("invariant: auth test router responds")
    }

    async fn auth_error_body(response: Response) -> Value {
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .expect("invariant: auth test response body buffers");
        serde_json::from_slice(&body).expect("invariant: auth error payload is valid json")
    }

    #[tokio::test]
    async fn require_api_key_accepts_matching_x_api_key_header() {
        let app = auth_test_app(ServeAuthConfig {
            enabled: true,
            api_key: "secret-key-123".into(),
        });

        let response = auth_response(app, |req| req.header("X-Api-Key", "secret-key-123")).await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn require_api_key_accepts_matching_bearer_token() {
        let app = auth_test_app(ServeAuthConfig {
            enabled: true,
            api_key: "secret-key-123".into(),
        });

        let response = auth_response(app, |req| {
            req.header(AUTHORIZATION, "Bearer secret-key-123")
        })
        .await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn require_api_key_rejects_missing_credentials() {
        let app = auth_test_app(ServeAuthConfig {
            enabled: true,
            api_key: "secret-key-123".into(),
        });

        let response = auth_response(app, |req| req).await;
        let status = response.status();
        let body = auth_error_body(response).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["code"], "unauthorized");
        assert_eq!(
            body["message"],
            "missing X-Api-Key header or Authorization bearer token"
        );
    }

    #[tokio::test]
    async fn require_api_key_rejects_invalid_bearer_token() {
        let app = auth_test_app(ServeAuthConfig {
            enabled: true,
            api_key: "secret-key-123".into(),
        });

        let response =
            auth_response(app, |req| req.header(AUTHORIZATION, "Bearer wrong-key")).await;
        let status = response.status();
        let body = auth_error_body(response).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            body["message"],
            "invalid or missing Authorization bearer token"
        );
    }

    #[tokio::test]
    async fn require_api_key_prefers_x_api_key_when_both_headers_are_present() {
        let app = auth_test_app(ServeAuthConfig {
            enabled: true,
            api_key: "secret-key-123".into(),
        });

        let response = auth_response(app, |req| {
            req.header("X-Api-Key", "wrong-key")
                .header(AUTHORIZATION, "Bearer secret-key-123")
        })
        .await;
        let status = response.status();
        let body = auth_error_body(response).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["message"], "invalid or missing X-Api-Key header");
    }

    #[tokio::test]
    async fn scrubs_api_key_from_json_response() {
        let scrubber = Arc::new(LogScrubber::new());
        let leaked = "your key is sk-ant-api03-abcdefghijklmnopqrstuvwxyz1234567890 ok";
        let app = test_app(scrubber, leaked);
        let req = Request::builder()
            .uri("/test")
            .body(Body::empty())
            .expect("invariant: building request body for test");
        let resp = app
            .oneshot(req)
            .await
            .expect("invariant: middleware test router responds");
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096)
            .await
            .expect("invariant: test response body buffers");
        let text =
            String::from_utf8(body.to_vec()).expect("invariant: middleware test body is utf-8");
        assert!(!text.contains("sk-ant-api03"));
        assert!(text.contains("[REDACTED"));
    }

    #[tokio::test]
    async fn clean_response_passes_through_unchanged() {
        let scrubber = Arc::new(LogScrubber::new());
        let clean = "all good, no secrets here";
        let app = test_app(scrubber, clean);
        let req = Request::builder()
            .uri("/test")
            .body(Body::empty())
            .expect("invariant: building request body for test");
        let resp = app
            .oneshot(req)
            .await
            .expect("invariant: middleware test router responds");
        let body = axum::body::to_bytes(resp.into_body(), 4096)
            .await
            .expect("invariant: test response body buffers");
        assert_eq!(
            std::str::from_utf8(&body).expect("invariant: clean response remains utf-8"),
            clean
        );
    }

    #[tokio::test]
    async fn binary_content_type_passes_through() {
        let scrubber = Arc::new(LogScrubber::new());
        let leaked = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz1234567890";
        let handler = move || async move {
            Response::builder()
                .header(CONTENT_TYPE, "image/png")
                .body(Body::from(leaked))
                .expect("invariant: image response body builds")
        };
        let app =
            Router::new()
                .route("/test", get(handler))
                .layer(axum::middleware::from_fn_with_state(
                    scrubber,
                    scrub_secrets,
                ));
        let req = Request::builder()
            .uri("/test")
            .body(Body::empty())
            .expect("invariant: building request body for test");
        let resp = app
            .oneshot(req)
            .await
            .expect("invariant: middleware test router responds");
        let body = axum::body::to_bytes(resp.into_body(), 4096)
            .await
            .expect("invariant: test response body buffers");
        // Binary/image content should NOT be scrubbed.
        assert_eq!(
            std::str::from_utf8(&body).expect("invariant: binary test payload is utf-8"),
            leaked
        );
    }

    #[tokio::test]
    async fn scrubs_github_pat_from_json_response() {
        let scrubber = Arc::new(LogScrubber::new());
        let leaked = "token: ghp_ABCDEFGHIJKLMNOPqrstuvwxyz1234567890";
        let app = test_app_json(scrubber, leaked);
        let req = Request::builder()
            .uri("/test")
            .body(Body::empty())
            .expect("invariant: building request body for test");
        let resp = app
            .oneshot(req)
            .await
            .expect("invariant: middleware test router responds");
        let body = axum::body::to_bytes(resp.into_body(), 4096)
            .await
            .expect("invariant: test response body buffers");
        let text =
            String::from_utf8(body.to_vec()).expect("invariant: middleware test body is utf-8");
        assert!(!text.contains("ghp_"));
        assert!(text.contains("[REDACTED"));
    }

    #[test]
    fn is_scrubbable_detects_json() {
        let resp = Response::builder()
            .header(CONTENT_TYPE, "application/json")
            .body(Body::empty())
            .expect("invariant: response builder constructs json response");
        assert!(is_scrubbable_content_type(&resp));
    }

    #[test]
    fn is_scrubbable_detects_text() {
        let resp = Response::builder()
            .header(CONTENT_TYPE, "text/plain")
            .body(Body::empty())
            .expect("invariant: response builder constructs text response");
        assert!(is_scrubbable_content_type(&resp));
    }

    #[test]
    fn is_scrubbable_rejects_image() {
        let resp = Response::builder()
            .header(CONTENT_TYPE, "image/png")
            .body(Body::empty())
            .expect("invariant: response builder constructs image response");
        assert!(!is_scrubbable_content_type(&resp));
    }

    #[test]
    fn is_scrubbable_rejects_octet_stream() {
        let resp = Response::builder()
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(Body::empty())
            .expect("invariant: response builder constructs octet-stream response");
        assert!(!is_scrubbable_content_type(&resp));
    }

    #[test]
    fn is_scrubbable_assumes_json_when_no_content_type() {
        let resp = Response::builder()
            .body(Body::empty())
            .expect("invariant: response builder constructs empty response");
        assert!(is_scrubbable_content_type(&resp));
    }
}
