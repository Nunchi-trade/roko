//! Webhook ingress endpoints.
//!
//! GitHub and Slack webhooks are verified, converted into typed
//! [`roko_core::Engram`]s, persisted through `.roko/engrams.jsonl`, and
//! published onto the shared event bus.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use hmac::{Hmac, Mac};
use roko_core::signal_kinds;
use roko_core::{Body, Engram, Kind, Provenance};
use serde_json::{Value, json};
use sha2::Sha256;

use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::state::AppState;

type HmacSha256 = Hmac<Sha256>;

/// Public webhook ingress (providers verify signatures themselves).
pub fn public_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/webhooks/github", post(github_webhook))
        .route("/webhooks/slack", post(slack_webhook))
}

/// Authenticated webhook ingress — arbitrary JSON payloads must not be accepted anonymously.
pub fn authenticated_routes() -> Router<Arc<AppState>> {
    Router::new().route("/webhooks/generic", post(generic_webhook))
}

/// `POST /webhooks/github` — verify the GitHub signature, convert the payload
/// into a `Engram`, persist it, and publish it to the server event bus.
async fn github_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let secret = {
        let config = state.load_roko_config();
        config.webhooks.github.secret.clone()
    };

    if secret.trim().is_empty() {
        return Err(ApiError::internal(
            "github webhook secret is not configured",
        ));
    }

    let received_signature = headers
        .get("X-Hub-Signature-256")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::unauthorized("missing X-Hub-Signature-256 header"))?;

    if !verify_github_signature(&secret, &body, received_signature) {
        return Err(ApiError::unauthorized("invalid github webhook signature"));
    }

    let event_type = headers
        .get("X-GitHub-Event")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::bad_request("missing X-GitHub-Event header"))?;

    let payload: Value = serde_json::from_slice(&body)
        .map_err(|e| ApiError::bad_request(format!("invalid github webhook json: {e}")))?;

    let kind = github_signal_kind(event_type, &payload)
        .ok_or_else(|| ApiError::bad_request(format!("unsupported github event: {event_type}")))?;

    let signal = attach_hdc_fingerprint(
        Engram::builder(kind)
            .body(Body::Json(payload))
            .provenance(Provenance::external("github:webhook"))
            .build(),
    );

    persist_webhook_signal(&state, signal).await?;

    Ok(StatusCode::OK)
}

/// `POST /webhooks/slack` — verify the Slack signature, handle URL
/// verification challenges, convert supported events into a `Engram`, persist
/// them, and publish them to the server event bus.
async fn slack_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let payload: Value = serde_json::from_slice(&body)
        .map_err(|e| ApiError::bad_request(format!("invalid slack webhook json: {e}")))?;

    if payload.get("type").and_then(Value::as_str) == Some("url_verification") {
        let challenge = payload
            .get("challenge")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::bad_request("missing slack challenge field"))?;

        return Ok((
            StatusCode::OK,
            axum::Json(json!({ "challenge": challenge })),
        )
            .into_response());
    }

    let secret = std::env::var("SLACK_SIGNING_SECRET")
        .map_err(|_| ApiError::internal("slack webhook signing secret is not configured"))?;
    if secret.trim().is_empty() {
        return Err(ApiError::internal(
            "slack webhook signing secret is not configured",
        ));
    }

    let timestamp = headers
        .get("X-Slack-Request-Timestamp")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::unauthorized("missing X-Slack-Request-Timestamp header"))?;

    verify_slack_timestamp(timestamp)?;

    let received_signature = headers
        .get("X-Slack-Signature")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::unauthorized("missing X-Slack-Signature header"))?;

    if !verify_slack_signature(&secret, timestamp, &body, received_signature) {
        return Err(ApiError::unauthorized("invalid slack webhook signature"));
    }

    let event_type = payload
        .get("event")
        .and_then(|event| event.get("type"))
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad_request("missing slack event type"))?;

    let kind = slack_signal_kind(event_type)
        .ok_or_else(|| ApiError::bad_request(format!("unsupported slack event: {event_type}")))?;

    let signal = attach_hdc_fingerprint(
        Engram::builder(kind)
            .body(Body::Json(payload))
            .provenance(Provenance::external("slack:webhook"))
            .build(),
    );

    persist_webhook_signal(&state, signal).await?;

    Ok(StatusCode::OK.into_response())
}

/// `POST /api/webhooks/generic` — accept arbitrary JSON, convert it into a
/// `Engram`, persist it, and publish it to the server event bus. This endpoint skips
/// signature verification and requires API authentication when auth is enabled.
async fn generic_webhook(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let payload: Value = serde_json::from_slice(&body)
        .map_err(|e| ApiError::bad_request(format!("invalid generic webhook json: {e}")))?;

    let signal = attach_hdc_fingerprint(generic_webhook_signal(payload));
    persist_webhook_signal(&state, signal).await?;

    Ok(StatusCode::OK)
}

fn generic_webhook_signal(payload: Value) -> Engram {
    Engram::builder(Kind::Custom("webhook:generic".into()))
        .body(Body::Json(payload))
        .provenance(Provenance::external("webhook:generic"))
        .build()
}

#[cfg(feature = "hdc")]
fn attach_hdc_fingerprint(mut signal: Engram) -> Engram {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    let fingerprint = roko_primitives::hdc::fingerprint(&signal.body);
    signal.tags.insert(
        "hdc_fingerprint".into(),
        BASE64.encode(fingerprint.to_bytes()),
    );
    signal.id = signal.content_hash();
    signal
}

#[cfg(not(feature = "hdc"))]
fn attach_hdc_fingerprint(signal: Engram) -> Engram {
    signal
}

async fn persist_webhook_signal(state: &AppState, signal: Engram) -> Result<(), ApiError> {
    state
        .signal_store
        .put(signal.clone())
        .await
        .map_err(|e| ApiError::internal(format!("persist webhook signal: {e}")))?;

    state
        .event_bus
        .publish(ServerEvent::WebhookReceived { signal });

    Ok(())
}

fn github_signal_kind(event_type: &str, payload: &Value) -> Option<Kind> {
    match event_type {
        "push" => Some(Kind::Custom(signal_kinds::GITHUB_PUSH.into())),
        "pull_request" => payload
            .get("action")
            .and_then(Value::as_str)
            .and_then(|action| match action {
                "opened" => Some(Kind::Custom(signal_kinds::GITHUB_PR_OPENED.into())),
                "closed"
                    if payload
                        .get("pull_request")
                        .and_then(|pr| pr.get("merged"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                        && payload
                            .get("pull_request")
                            .and_then(|pr| pr.get("head"))
                            .and_then(|head| head.get("ref"))
                            .and_then(Value::as_str)
                            .is_some_and(|branch| branch.starts_with("plan/")) =>
                {
                    Some(Kind::Custom(signal_kinds::PRD_PLAN_APPROVED.into()))
                }
                _ => None,
            }),
        "pull_request_review" => Some(Kind::Custom(signal_kinds::GITHUB_PR_REVIEW.into())),
        "issues" => payload
            .get("action")
            .and_then(Value::as_str)
            .filter(|action| *action == "opened")
            .map(|_| Kind::Custom(signal_kinds::GITHUB_ISSUE_OPENED.into())),
        _ => None,
    }
}

fn verify_github_signature(secret: &str, body: &[u8], received_signature: &str) -> bool {
    let Some(received_bytes) = parse_github_signature(received_signature) else {
        return false;
    };

    let mut mac: HmacSha256 = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(mac) => mac,
        Err(_) => return false,
    };
    mac.update(body);
    let expected = mac.finalize().into_bytes();

    constant_time_eq(expected.as_ref(), &received_bytes)
}

fn slack_signal_kind(event_type: &str) -> Option<Kind> {
    match event_type {
        "message" => Some(Kind::Custom(signal_kinds::SLACK_MESSAGE.into())),
        "reaction_added" => Some(Kind::Custom(signal_kinds::SLACK_REACTION.into())),
        _ => None,
    }
}

fn verify_slack_signature(
    secret: &str,
    timestamp: &str,
    body: &[u8],
    received_signature: &str,
) -> bool {
    let Some(received_bytes) = parse_slack_signature(received_signature) else {
        return false;
    };

    let base = format!("v0:{timestamp}:");
    let mut mac: HmacSha256 = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(mac) => mac,
        Err(_) => return false,
    };
    mac.update(base.as_bytes());
    mac.update(body);
    let expected = mac.finalize().into_bytes();

    constant_time_eq(expected.as_ref(), &received_bytes)
}

fn parse_slack_signature(signature: &str) -> Option<[u8; 32]> {
    let hex = signature.strip_prefix("v0=").unwrap_or(signature);
    if hex.len() != 64 {
        return None;
    }

    let mut out = [0u8; 32];
    for (idx, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
        out[idx] = (hex_value(chunk[0])? << 4) | hex_value(chunk[1])?;
    }

    Some(out)
}

fn verify_slack_timestamp(timestamp: &str) -> Result<(), ApiError> {
    let timestamp = timestamp
        .parse::<i64>()
        .map_err(|_| ApiError::bad_request("invalid X-Slack-Request-Timestamp header"))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::internal("system clock is before unix epoch"))?
        .as_secs() as i64;

    if (now - timestamp).abs() > 300 {
        return Err(ApiError::unauthorized("stale slack webhook timestamp"));
    }

    Ok(())
}

fn parse_github_signature(signature: &str) -> Option<[u8; 32]> {
    let hex = signature.strip_prefix("sha256=").unwrap_or(signature);
    if hex.len() != 64 {
        return None;
    }

    let mut out = [0u8; 32];
    for (idx, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
        out[idx] = (hex_value(chunk[0])? << 4) | hex_value(chunk[1])?;
    }

    Some(out)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let mut diff = 0u8;
    for (lhs, rhs) in a.iter().zip(b.iter()) {
        diff |= lhs ^ rhs;
    }

    core::hint::black_box(diff) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "hdc")]
    use base64::Engine as _;
    #[cfg(feature = "hdc")]
    use base64::engine::general_purpose::STANDARD as BASE64;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type TestHmacSha256 = Hmac<Sha256>;

    #[test]
    fn maps_supported_github_events_to_signal_kinds() {
        let push = github_signal_kind("push", &serde_json::json!({}));
        assert!(
            matches!(push.as_ref().map(Kind::as_str), Some(kind) if kind == signal_kinds::GITHUB_PUSH)
        );

        let pr_opened =
            github_signal_kind("pull_request", &serde_json::json!({ "action": "opened" }));
        assert!(
            matches!(pr_opened.as_ref().map(Kind::as_str), Some(kind) if kind == signal_kinds::GITHUB_PR_OPENED)
        );

        let review = github_signal_kind("pull_request_review", &serde_json::json!({}));
        assert!(
            matches!(review.as_ref().map(Kind::as_str), Some(kind) if kind == signal_kinds::GITHUB_PR_REVIEW)
        );

        let issue_opened = github_signal_kind("issues", &serde_json::json!({ "action": "opened" }));
        assert!(
            matches!(issue_opened.as_ref().map(Kind::as_str), Some(kind) if kind == signal_kinds::GITHUB_ISSUE_OPENED)
        );

        let plan_approved = github_signal_kind(
            "pull_request",
            &serde_json::json!({
                "action": "closed",
                "pull_request": {
                    "merged": true,
                    "head": { "ref": "plan/test-feature" }
                }
            }),
        );
        assert!(
            matches!(plan_approved.as_ref().map(Kind::as_str), Some(kind) if kind == signal_kinds::PRD_PLAN_APPROVED)
        );
    }

    #[test]
    fn verifies_github_signature_in_constant_time() {
        let secret = "secret";
        let body = br#"{"hello":"world"}"#;

        let mut mac: TestHmacSha256 = match TestHmacSha256::new_from_slice(secret.as_bytes()) {
            Ok(mac) => mac,
            Err(_) => panic!("invalid test hmac key"),
        };
        mac.update(body);
        let signature = format!(
            "sha256={}",
            hex_encode(mac.finalize().into_bytes().as_ref())
        );

        assert!(verify_github_signature(secret, body, &signature));
        assert!(!verify_github_signature(secret, body, "sha256=deadbeef"));
    }

    #[test]
    fn maps_supported_slack_events_to_signal_kinds() {
        let message = slack_signal_kind("message");
        assert!(
            matches!(message.as_ref().map(Kind::as_str), Some(kind) if kind == signal_kinds::SLACK_MESSAGE)
        );

        let reaction = slack_signal_kind("reaction_added");
        assert!(
            matches!(reaction.as_ref().map(Kind::as_str), Some(kind) if kind == signal_kinds::SLACK_REACTION)
        );
    }

    #[test]
    fn verifies_slack_signature() {
        let secret = "secret";
        let timestamp = "1712668800";
        let body = br#"{"type":"event_callback","event":{"type":"message"}}"#;

        let mut mac: TestHmacSha256 = match TestHmacSha256::new_from_slice(secret.as_bytes()) {
            Ok(mac) => mac,
            Err(_) => panic!("invalid test hmac key"),
        };
        mac.update(format!("v0:{timestamp}:").as_bytes());
        mac.update(body);
        let signature = format!("v0={}", hex_encode(mac.finalize().into_bytes().as_ref()));

        assert!(verify_slack_signature(secret, timestamp, body, &signature));
        assert!(!verify_slack_signature(
            secret,
            timestamp,
            body,
            "v0=deadbeef"
        ));
    }

    #[test]
    fn parses_signature_with_or_without_prefix() {
        let sig = "sha256=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(parse_github_signature(sig).is_some());
        assert!(parse_github_signature(&sig[7..]).is_some());
    }

    #[test]
    fn builds_generic_webhook_signal_with_raw_json_payload() {
        let payload = serde_json::json!({
            "nested": { "value": 1 },
            "items": [1, 2, 3],
        });
        let signal = generic_webhook_signal(payload.clone());

        assert_eq!(signal.kind.as_str(), "webhook:generic");
        assert_eq!(signal.body, Body::Json(payload));
        assert_eq!(signal.provenance.author, "webhook:generic");
    }

    #[tokio::test]
    async fn generic_webhook_requires_auth_when_enabled() {
        use std::sync::Arc;

        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use roko_core::config::ServeAuthConfig;
        use roko_core::config::schema::RokoConfig;
        use tempfile::tempdir;
        use tower::ServiceExt;

        use crate::deploy::create_backend;
        use crate::routes::build_router;
        use crate::runtime::NoOpRuntime;
        use crate::state::AppState;

        let dir = tempdir().expect("tempdir");
        let mut cfg = RokoConfig::default();
        cfg.serve.auth.enabled = true;
        cfg.serve.auth.api_key = "correct-key".into();
        let deploy_backend =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        let state = Arc::new(
            AppState::new(
                dir.path().to_path_buf(),
                Arc::new(NoOpRuntime),
                cfg,
                deploy_backend,
            )
            .expect("AppState::new"),
        );
        let api_auth = ServeAuthConfig {
            enabled: true,
            api_key: "correct-key".into(),
            ..Default::default()
        };
        let app = build_router(Arc::clone(&state), &[], api_auth);

        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webhooks/generic")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let authorized = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webhooks/generic")
                    .header("X-Api-Key", "correct-key")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"ok":true}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(authorized.status(), StatusCode::OK);
    }

    #[cfg(feature = "hdc")]
    #[test]
    fn attach_hdc_fingerprint_populates_signal_metadata() {
        use roko_primitives::hdc::{HdcVector, fingerprint};

        let body = Body::Json(serde_json::json!({
            "event": "push",
            "repository": "roko",
            "changes": ["a.rs", "b.rs"],
        }));
        let signal = Engram::builder(Kind::Custom("github:push".into()))
            .body(body.clone())
            .provenance(Provenance::external("github:webhook"))
            .build();

        let signal = attach_hdc_fingerprint(signal);
        let encoded = signal
            .tag("hdc_fingerprint")
            .expect("expected hdc_fingerprint metadata");
        let decoded = BASE64.decode(encoded).expect("decode hdc fingerprint");
        let raw: [u8; 1280] = decoded
            .as_slice()
            .try_into()
            .expect("expected 1280-byte hdc fingerprint");
        let recovered = HdcVector::from_bytes(&raw);

        assert_eq!(recovered, fingerprint(&body));
    }

    fn hex_encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            use std::fmt::Write as _;
            let _ = write!(&mut out, "{byte:02x}");
        }
        out
    }
}
