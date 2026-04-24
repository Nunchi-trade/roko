//! Shared API auth and scrubbing middleware for `/api/*` routes.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::Request;
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use roko_core::config::{ApiKeyEntry, ServeAuthConfig};
use roko_core::obs::LogScrubber;
use sha2::{Digest, Sha256};
use tower_http::cors::{Any, CorsLayer};

use crate::error::ApiError;

/// Extract a bearer token from an `Authorization` header value.
///
/// Performs case-insensitive prefix matching on "bearer", trims whitespace,
/// and returns `None` if the token portion is empty.
pub fn extract_bearer_token(header_value: &str) -> Option<&str> {
    let lower = header_value.as_bytes();
    if lower.len() < 7 {
        return None;
    }
    if !lower[..6].eq_ignore_ascii_case(b"bearer") {
        return None;
    }
    let rest = &header_value[6..];
    // Must be followed by whitespace (or be exactly "bearer" + space).
    let token = rest.trim();
    if token.is_empty() {
        return None;
    }
    Some(token)
}

/// Returns `true` when `token` looks structurally like a JWT (three
/// non-empty dot-separated segments of valid base64url characters).
///
/// No signature verification is performed.
pub fn is_structurally_valid_jwt(token: &str) -> bool {
    let segments: Vec<&str> = token.split('.').collect();
    if segments.len() != 3 {
        return false;
    }
    segments
        .iter()
        .all(|s| !s.is_empty() && s.bytes().all(is_base64url_byte))
}

fn is_base64url_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'='
}

/// Which authentication method was used for a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    /// Authenticated via `X-Api-Key` header.
    ApiKey,
    /// Authenticated via a structurally valid JWT in `Authorization: Bearer`.
    Jwt,
    /// Authenticated via a non-JWT bearer token.
    Bearer,
}

impl AuthMethod {
    /// Machine-readable label set in the `X-Auth-Method` response header.
    pub fn header_value(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::Jwt => "jwt",
            Self::Bearer => "bearer",
        }
    }
}

/// Authenticated caller context injected into request extensions.
///
/// Routes can extract this via `req.extensions().get::<AuthContext>()` or
/// the axum `Extension<AuthContext>` extractor.
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// How the caller authenticated.
    pub method: AuthMethod,
    /// Permission scope (e.g. "admin", "agent:write", "read").
    pub scope: String,
    /// Optional user/key identifier.
    pub user_id: Option<String>,
}

/// Compute the hex-encoded SHA-256 hash of a plaintext API key.
pub fn hash_api_key(plaintext: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(plaintext.as_bytes());
    let digest = hasher.finalize();
    // Inline hex encoding to avoid adding a `hex` dependency.
    digest.iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Check an API key against the list of named key entries.
///
/// Returns the matching entry if the hash matches and the key has not expired.
fn match_api_key_entry<'a>(token: &str, entries: &'a [ApiKeyEntry]) -> Option<&'a ApiKeyEntry> {
    let token_hash = hash_api_key(token);
    let now = Utc::now().to_rfc3339();
    entries.iter().find(|entry| {
        if entry.key_hash != token_hash {
            return false;
        }
        // Reject expired keys.
        if let Some(ref expires) = entry.expires_at {
            if *expires < now {
                return false;
            }
        }
        true
    })
}

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
            Ok(value) => match extract_bearer_token(value) {
                Some(token) => ApiCredential::Bearer(token),
                None => ApiCredential::InvalidAuthorization,
            },
            Err(_) => ApiCredential::InvalidAuthorization,
        };
    }

    ApiCredential::Missing
}

/// Authenticate the supplied token against the legacy single key and the
/// named `api_keys` list. Returns `(AuthMethod, scope, user_id)` on success.
fn authenticate_token(
    token: &str,
    auth: &ServeAuthConfig,
    via_header: bool,
) -> Option<(AuthMethod, String, Option<String>)> {
    // 1. Try named API keys first.
    if let Some(entry) = match_api_key_entry(token, &auth.api_keys) {
        let method = if via_header {
            AuthMethod::ApiKey
        } else if is_structurally_valid_jwt(token) {
            AuthMethod::Jwt
        } else {
            AuthMethod::Bearer
        };
        return Some((method, entry.scope.clone(), Some(entry.name.clone())));
    }

    // 2. Fall back to legacy single api_key for backwards compatibility.
    if !auth.api_key.is_empty() && token == auth.api_key {
        let method = if via_header {
            AuthMethod::ApiKey
        } else if is_structurally_valid_jwt(token) {
            AuthMethod::Jwt
        } else {
            AuthMethod::Bearer
        };
        return Some((method, "admin".to_string(), None));
    }

    // 3. Bearer-only: check if it looks like a Privy JWT (stub).
    if !via_header && is_structurally_valid_jwt(token) && auth.privy_app_id.is_some() {
        // TODO(phase-1b): real Privy JWT signature verification.
        // For now, accept any structurally-valid JWT when privy_app_id is configured.
        return Some((AuthMethod::Jwt, "read".to_string(), None));
    }

    None
}

/// Require a matching API credential for the request to continue.
///
/// Supports three credential sources:
/// - `X-Api-Key` header (checked first for deterministic precedence)
/// - `Authorization: Bearer <token>` (API key or JWT)
/// - Named API keys from `api_keys` list (SHA-256 hash comparison)
///
/// On success, injects [`AuthContext`] into request extensions so downstream
/// routes can inspect the caller's scope and identity.
pub async fn require_api_key(
    State(auth): State<ServeAuthConfig>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let (auth_method, ctx) = match api_credential(req.headers()) {
        ApiCredential::XApiKey(supplied) => match authenticate_token(supplied, &auth, true) {
            Some((method, scope, user_id)) => (
                method,
                AuthContext {
                    method,
                    scope,
                    user_id,
                },
            ),
            None => {
                return Err(ApiError::unauthorized(
                    "invalid or missing X-Api-Key header",
                ));
            }
        },
        ApiCredential::Bearer(supplied) => match authenticate_token(supplied, &auth, false) {
            Some((method, scope, user_id)) => (
                method,
                AuthContext {
                    method,
                    scope,
                    user_id,
                },
            ),
            None => {
                return Err(ApiError::unauthorized(
                    "invalid or missing Authorization bearer token",
                ));
            }
        },
        ApiCredential::InvalidXApiKey => {
            return Err(ApiError::unauthorized(
                "invalid or missing X-Api-Key header",
            ));
        }
        ApiCredential::InvalidAuthorization => {
            return Err(ApiError::unauthorized(
                "invalid or missing Authorization bearer token",
            ));
        }
        ApiCredential::Missing => {
            return Err(ApiError::unauthorized(
                "missing X-Api-Key header or Authorization bearer token",
            ));
        }
    };

    // Inject AuthContext for downstream handlers.
    req.extensions_mut().insert(ctx);

    let mut response = next.run(req).await;
    response.headers_mut().insert(
        "X-Auth-Method",
        axum::http::HeaderValue::from_static(auth_method.header_value()),
    );
    Ok(response)
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

    fn legacy_auth(api_key: &str) -> ServeAuthConfig {
        ServeAuthConfig {
            enabled: true,
            api_key: api_key.into(),
            api_keys: Vec::new(),
            privy_app_id: None,
        }
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
        let app = auth_test_app(legacy_auth("secret-key-123"));

        let response = auth_response(app, |req| req.header("X-Api-Key", "secret-key-123")).await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn require_api_key_accepts_matching_bearer_token() {
        let app = auth_test_app(legacy_auth("secret-key-123"));

        let response = auth_response(app, |req| {
            req.header(AUTHORIZATION, "Bearer secret-key-123")
        })
        .await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn require_api_key_rejects_missing_credentials() {
        let app = auth_test_app(legacy_auth("secret-key-123"));

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
        let app = auth_test_app(legacy_auth("secret-key-123"));

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
        let app = auth_test_app(legacy_auth("secret-key-123"));

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

    // --- extract_bearer_token tests ---

    #[test]
    fn extract_bearer_token_standard_case() {
        assert_eq!(extract_bearer_token("Bearer mytoken"), Some("mytoken"));
    }

    #[test]
    fn extract_bearer_token_lowercase() {
        assert_eq!(extract_bearer_token("bearer mytoken"), Some("mytoken"));
    }

    #[test]
    fn extract_bearer_token_uppercase() {
        assert_eq!(extract_bearer_token("BEARER mytoken"), Some("mytoken"));
    }

    #[test]
    fn extract_bearer_token_no_prefix() {
        assert_eq!(extract_bearer_token("mytoken"), None);
    }

    #[test]
    fn extract_bearer_token_empty_string() {
        assert_eq!(extract_bearer_token(""), None);
    }

    #[test]
    fn extract_bearer_token_empty_after_strip() {
        assert_eq!(extract_bearer_token("Bearer "), None);
    }

    // --- is_structurally_valid_jwt tests ---

    #[test]
    fn jwt_valid_three_segments() {
        assert!(is_structurally_valid_jwt("abc.def.ghi"));
    }

    #[test]
    fn jwt_rejects_two_segments() {
        assert!(!is_structurally_valid_jwt("abc.def"));
    }

    #[test]
    fn jwt_rejects_four_segments() {
        assert!(!is_structurally_valid_jwt("a.b.c.d"));
    }

    #[test]
    fn jwt_rejects_empty_segment() {
        assert!(!is_structurally_valid_jwt("a..c"));
    }

    #[test]
    fn jwt_accepts_base64url_chars() {
        assert!(is_structurally_valid_jwt(
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abc_DEF-123="
        ));
    }

    // --- X-Auth-Method response header tests ---

    #[tokio::test]
    async fn auth_method_header_set_to_api_key() {
        let app = auth_test_app(legacy_auth("secret-key-123"));
        let response = auth_response(app, |req| req.header("X-Api-Key", "secret-key-123")).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response
                .headers()
                .get("X-Auth-Method")
                .unwrap()
                .to_str()
                .unwrap(),
            "api_key"
        );
    }

    #[tokio::test]
    async fn auth_method_header_set_to_bearer() {
        let app = auth_test_app(legacy_auth("secret-key-123"));
        let response = auth_response(app, |req| {
            req.header(AUTHORIZATION, "Bearer secret-key-123")
        })
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response
                .headers()
                .get("X-Auth-Method")
                .unwrap()
                .to_str()
                .unwrap(),
            "bearer"
        );
    }

    #[tokio::test]
    async fn auth_method_header_set_to_jwt() {
        // Use a JWT-shaped token (3 dot-separated base64url segments) as the api_key
        let jwt_key = "eyJhbGci.eyJzdWIi.abc123";
        let app = auth_test_app(legacy_auth(jwt_key));
        let response = auth_response(app, |req| {
            req.header(AUTHORIZATION, format!("Bearer {jwt_key}"))
        })
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response
                .headers()
                .get("X-Auth-Method")
                .unwrap()
                .to_str()
                .unwrap(),
            "jwt"
        );
    }
}
