//! Webhook ingress endpoints.
//!
//! GitHub webhooks are verified with `X-Hub-Signature-256`, converted into
//! typed [`roko_core::Signal`]s, and published onto the shared event bus.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;
use hmac::{Hmac, Mac};
use roko_core::signal_kinds;
use roko_core::{Body, Kind, Provenance, Signal};
use serde_json::Value;
use sha2::Sha256;

use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::state::AppState;

type HmacSha256 = Hmac<Sha256>;

/// Build the webhook router.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/webhooks/github", post(github_webhook))
}

/// `POST /webhooks/github` — verify the GitHub signature, convert the payload
/// into a `Signal`, and publish it to the server event bus.
async fn github_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let secret = {
        let config = state.roko_config.read().await;
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

    let signal = Signal::builder(kind)
        .body(Body::Json(payload))
        .provenance(Provenance::external("github:webhook"))
        .build();

    state
        .event_bus
        .publish(ServerEvent::WebhookReceived { signal });

    Ok(StatusCode::OK)
}

fn github_signal_kind(event_type: &str, payload: &Value) -> Option<Kind> {
    match event_type {
        "push" => Some(Kind::Custom(signal_kinds::GITHUB_PUSH.into())),
        "pull_request" => payload
            .get("action")
            .and_then(Value::as_str)
            .filter(|action| *action == "opened")
            .map(|_| Kind::Custom(signal_kinds::GITHUB_PR_OPENED.into())),
        "pull_request_review" => {
            Some(Kind::Custom(signal_kinds::GITHUB_PR_REVIEW.into()))
        }
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
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type TestHmacSha256 = Hmac<Sha256>;

    #[test]
    fn maps_supported_github_events_to_signal_kinds() {
        let push = github_signal_kind("push", &serde_json::json!({}));
        assert!(matches!(push.as_ref().map(Kind::as_str), Some(kind) if kind == signal_kinds::GITHUB_PUSH));

        let pr_opened = github_signal_kind("pull_request", &serde_json::json!({ "action": "opened" }));
        assert!(matches!(pr_opened.as_ref().map(Kind::as_str), Some(kind) if kind == signal_kinds::GITHUB_PR_OPENED));

        let review = github_signal_kind("pull_request_review", &serde_json::json!({}));
        assert!(matches!(review.as_ref().map(Kind::as_str), Some(kind) if kind == signal_kinds::GITHUB_PR_REVIEW));

        let issue_opened = github_signal_kind("issues", &serde_json::json!({ "action": "opened" }));
        assert!(matches!(issue_opened.as_ref().map(Kind::as_str), Some(kind) if kind == signal_kinds::GITHUB_ISSUE_OPENED));
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
        let signature = format!("sha256={}", hex_encode(mac.finalize().into_bytes().as_ref()));

        assert!(verify_github_signature(secret, body, &signature));
        assert!(!verify_github_signature(secret, body, "sha256=deadbeef"));
    }

    #[test]
    fn parses_signature_with_or_without_prefix() {
        let sig = "sha256=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(parse_github_signature(sig).is_some());
        assert!(parse_github_signature(&sig[7..]).is_some());
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
