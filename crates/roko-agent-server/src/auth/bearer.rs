//! Bearer-token middleware for agent-server routes.

use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Bearer token validator used by protected routes.
#[derive(Clone, Debug)]
pub struct BearerAuth {
    token_hash: [u8; 32],
}

impl BearerAuth {
    /// Construct a new bearer-token validator from the shared secret.
    #[must_use]
    pub fn new(secret: impl AsRef<str>) -> Self {
        Self {
            token_hash: hash(secret.as_ref()),
        }
    }

    /// Return the SHA-256 digest encoded for transport/storage comparisons.
    #[must_use]
    pub fn token_hash_base64(&self) -> String {
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(self.token_hash)
    }

    /// Verify a candidate bearer token.
    #[must_use]
    pub fn verify(&self, token: &str) -> bool {
        hash(token) == self.token_hash
    }
}

/// Middleware for protected agent routes.
pub async fn require_bearer_auth(
    axum::extract::State(auth): axum::extract::State<BearerAuth>,
    request: Request,
    next: Next,
) -> Response {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| !token.trim().is_empty());

    if token.is_some_and(|token| auth.verify(token)) {
        return next.run(request).await;
    }

    let response = ErrorBody {
        error: "missing or invalid bearer token".to_string(),
    };
    (StatusCode::UNAUTHORIZED, axum::Json(response)).into_response()
}

fn hash(secret: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.finalize().into()
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_expected_secret() {
        let auth = BearerAuth::new("abc123");
        assert!(auth.verify("abc123"));
        assert!(!auth.verify("wrong"));
    }
}
