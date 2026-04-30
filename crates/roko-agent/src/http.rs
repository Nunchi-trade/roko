//! Shared HTTP transport seam used by the LLM-backed agents.
//!
//! Each agent backend (`Claude`, `Ollama`, `OpenAI`, …) needs to POST a JSON body
//! to a remote endpoint and read back a string response. Rather than each
//! backend inventing its own `HttpPoster` trait and `ReqwestPoster` pair,
//! this module provides a single canonical trait and a production-grade
//! reqwest-backed implementation.
//!
//! # Why a trait (and not just `reqwest::Client`)?
//!
//! Tests inject a `MockPoster` that returns canned responses, so unit tests
//! never open sockets. The trait is the test seam.
//!
//! # Why `&[u8]` for the body?
//!
//! Most callers serialize JSON to `String` and call `.as_bytes()`; a few
//! serialize via `serde_json::to_vec`. Using `&[u8]` accepts both without
//! forcing an extra allocation.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

/// An error returned by the low-level HTTP transport.
#[derive(Clone, Debug)]
pub struct HttpPostError {
    /// HTTP status code, if any (e.g. 429, 500). `None` if the request
    /// failed before a response was received (DNS, connect, TLS…).
    pub status: Option<u16>,
    /// Human-readable message describing the failure.
    pub message: String,
}

impl HttpPostError {
    /// Build a pre-response transport error (no HTTP status).
    #[must_use]
    pub fn transport(message: impl Into<String>) -> Self {
        Self {
            status: None,
            message: message.into(),
        }
    }

    /// Build an error from an HTTP status + response body.
    #[must_use]
    pub fn http(status: u16, message: impl Into<String>) -> Self {
        Self {
            status: Some(status),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for HttpPostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.status {
            Some(code) => write!(f, "http {code}: {}", self.message),
            None => write!(f, "transport error: {}", self.message),
        }
    }
}

/// Transport abstraction so tests can inject a mock without hitting the network.
///
/// Implementors POST `body` as JSON to `url`, attaching the provided `headers`.
/// On 2xx, return the response body as a `String`. On any non-2xx status or
/// transport failure, return [`HttpPostError`].
///
/// Implementations MUST honour `timeout_ms`: if the remote does not respond
/// within the deadline the call should return an [`HttpPostError::transport`].
#[async_trait]
pub trait HttpPoster: Send + Sync {
    /// Post `body` to `url` with `headers`. See trait docs for semantics.
    async fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &[u8],
        timeout_ms: u64,
    ) -> Result<String, HttpPostError>;

    /// Perform a GET request to `url` with `headers`.
    ///
    /// The default implementation returns an error. Override in implementations
    /// that need GET support (e.g. polling endpoints).
    async fn get_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        timeout_ms: u64,
    ) -> Result<String, HttpPostError> {
        let _ = (url, headers, timeout_ms);
        Err(HttpPostError::transport(
            "get_json not supported by this poster",
        ))
    }

    /// Perform a DELETE request to `url` with `headers`.
    ///
    /// The default implementation returns an error. Override in implementations
    /// that need DELETE support.
    async fn delete_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        timeout_ms: u64,
    ) -> Result<String, HttpPostError> {
        let _ = (url, headers, timeout_ms);
        Err(HttpPostError::transport(
            "delete_json not supported by this poster",
        ))
    }
}

/// Production [`HttpPoster`] backed by `reqwest`.
#[derive(Debug, Default)]
pub struct ReqwestPoster {
    client: reqwest::Client,
}

impl ReqwestPoster {
    /// Build a new poster backed by the shared `reqwest::Client`.
    ///
    /// This reuses the process-wide connection pool so agent instances do not
    /// pay a fresh TCP+TLS handshake for every new backend.
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: crate::provider::shared_http_client(),
        }
    }

    /// Wrap an existing `reqwest::Client` (useful when pooling is desired).
    #[must_use]
    pub const fn with_client(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl HttpPoster for ReqwestPoster {
    async fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &[u8],
        timeout_ms: u64,
    ) -> Result<String, HttpPostError> {
        let mut req = self
            .client
            .post(url)
            .timeout(Duration::from_millis(timeout_ms));
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req
            .body(body.to_vec())
            .send()
            .await
            .map_err(|e| HttpPostError::transport(format!("request failed: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| HttpPostError::transport(format!("read body failed: {e}")))?;
        if status.is_success() {
            Ok(text)
        } else {
            Err(HttpPostError::http(status.as_u16(), text))
        }
    }

    async fn get_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        timeout_ms: u64,
    ) -> Result<String, HttpPostError> {
        let mut req = self
            .client
            .get(url)
            .timeout(Duration::from_millis(timeout_ms));
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req
            .send()
            .await
            .map_err(|e| HttpPostError::transport(format!("request failed: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| HttpPostError::transport(format!("read body failed: {e}")))?;
        if status.is_success() {
            Ok(text)
        } else {
            Err(HttpPostError::http(status.as_u16(), text))
        }
    }

    async fn delete_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        timeout_ms: u64,
    ) -> Result<String, HttpPostError> {
        let mut req = self
            .client
            .delete(url)
            .timeout(Duration::from_millis(timeout_ms));
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req
            .send()
            .await
            .map_err(|e| HttpPostError::transport(format!("request failed: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| HttpPostError::transport(format!("read body failed: {e}")))?;
        if status.is_success() {
            Ok(text)
        } else {
            Err(HttpPostError::http(status.as_u16(), text))
        }
    }
}

/// Create a shared HTTP poster for the process.
///
/// Call this once at startup and clone the returned [`Arc`] wherever HTTP
/// dispatch is needed.
#[must_use]
pub fn shared_http_client() -> Arc<ReqwestPoster> {
    Arc::new(ReqwestPoster::new())
}

/// Create a shared HTTP poster from an existing `reqwest::Client`.
#[must_use]
pub fn shared_http_client_from(client: reqwest::Client) -> Arc<ReqwestPoster> {
    Arc::new(ReqwestPoster::with_client(client))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_error_has_no_status() {
        let e = HttpPostError::transport("dns");
        assert!(e.status.is_none());
        assert_eq!(e.message, "dns");
        assert_eq!(e.to_string(), "transport error: dns");
    }

    #[test]
    fn http_error_carries_status() {
        let e = HttpPostError::http(429, "rate limited");
        assert_eq!(e.status, Some(429));
        assert_eq!(e.to_string(), "http 429: rate limited");
    }

    #[test]
    fn reqwest_poster_is_constructible() {
        let _p = ReqwestPoster::new();
        let _d = ReqwestPoster::default();
    }
}
