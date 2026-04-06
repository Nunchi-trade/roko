//! Batch LLM client abstraction for the Anthropic Batch API.
//!
//! Batch mode submits enrichment requests at **50% cost** compared to real-time
//! requests. Requests are queued and processed asynchronously; callers poll for
//! completion.
//!
//! This module defines request/response types and a trait for the HTTP transport
//! layer. No real HTTP calls happen here -- implementations live in the app layer
//! (anti-pattern #8: I/O at boundary only).

use std::fmt;

use serde::{Deserialize, Serialize};

// ── Batch ID ────────────────────────────────────────────────────────────

/// Opaque identifier for a submitted batch.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BatchId(pub String);

impl fmt::Display for BatchId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl BatchId {
    /// Create a new batch ID from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

// ── Request / Response ──────────────────────────────────────────────────

/// A single request within a batch submission.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BatchRequest {
    /// Caller-assigned identifier so results can be correlated.
    pub custom_id: String,
    /// Model identifier (e.g. `"claude-sonnet-4-6"`).
    pub model: String,
    /// System prompt.
    pub system: String,
    /// User message content.
    pub user_message: String,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
}

impl BatchRequest {
    /// Build an Anthropic-style request body for this batch item.
    ///
    /// Returns the JSON-serializable structure matching the Anthropic
    /// `/v1/messages/batches` item format.
    pub fn to_api_body(&self) -> serde_json::Value {
        serde_json::json!({
            "custom_id": self.custom_id,
            "params": {
                "model": self.model,
                "max_tokens": self.max_tokens,
                "system": self.system,
                "messages": [
                    {"role": "user", "content": self.user_message}
                ]
            }
        })
    }
}

/// Processing status of a batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BatchStatus {
    /// The batch has been accepted but not yet started.
    Pending,
    /// The batch is currently being processed.
    Processing,
    /// All items in the batch have completed.
    Complete,
    /// The batch encountered a fatal error.
    Failed,
}

impl fmt::Display for BatchStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Pending => "pending",
            Self::Processing => "processing",
            Self::Complete => "complete",
            Self::Failed => "failed",
        };
        write!(f, "{label}")
    }
}

/// Token usage for a single batch response item.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BatchUsage {
    /// Number of input tokens consumed.
    pub input_tokens: u32,
    /// Number of output tokens generated.
    pub output_tokens: u32,
}

/// A single result from a completed batch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BatchResponse {
    /// The `custom_id` from the corresponding [`BatchRequest`].
    pub custom_id: String,
    /// Whether this individual item succeeded.
    pub succeeded: bool,
    /// Generated text content (empty on failure).
    pub content: String,
    /// Token usage for this item.
    pub usage: BatchUsage,
    /// Error message if this item failed.
    pub error: Option<String>,
}

// ── Transport trait ─────────────────────────────────────────────────────

/// Trait abstracting the HTTP transport for batch operations.
///
/// Implementations live outside this crate. Tests use mock implementations.
#[async_trait::async_trait]
pub trait BatchTransport: Send + Sync {
    /// Submit a list of requests as a batch. Returns the batch ID.
    async fn submit_batch(
        &self,
        requests: &[BatchRequest],
    ) -> Result<BatchId, Box<dyn std::error::Error + Send + Sync>>;

    /// Poll the status of a batch.
    async fn poll_batch(
        &self,
        id: &BatchId,
    ) -> Result<BatchStatus, Box<dyn std::error::Error + Send + Sync>>;

    /// Retrieve results for a completed batch.
    async fn get_results(
        &self,
        id: &BatchId,
    ) -> Result<Vec<BatchResponse>, Box<dyn std::error::Error + Send + Sync>>;
}

// ── BatchClient ─────────────────────────────────────────────────────────

/// High-level batch client that wraps a [`BatchTransport`] implementation.
///
/// Provides convenience methods for submitting enrichment requests via the
/// Anthropic Batch API. Batch processing yields **50% cost savings** compared
/// to real-time requests at the expense of higher latency.
pub struct BatchClient<T: BatchTransport> {
    transport: T,
}

impl<T: BatchTransport> BatchClient<T> {
    /// Create a new batch client with the given transport.
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Submit a batch of requests. Returns the batch identifier for polling.
    ///
    /// # Cost savings
    ///
    /// Batch API requests are billed at 50% of the standard per-token rate.
    /// Use batch mode for enrichment steps that are not latency-sensitive.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport fails to submit.
    pub async fn submit(
        &self,
        requests: Vec<BatchRequest>,
    ) -> Result<BatchId, Box<dyn std::error::Error + Send + Sync>> {
        if requests.is_empty() {
            return Err("cannot submit an empty batch".into());
        }
        self.transport.submit_batch(&requests).await
    }

    /// Poll the current status of a batch.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport fails to poll.
    pub async fn poll(
        &self,
        id: &BatchId,
    ) -> Result<BatchStatus, Box<dyn std::error::Error + Send + Sync>> {
        self.transport.poll_batch(id).await
    }

    /// Retrieve results for a completed batch.
    ///
    /// Should only be called when [`poll`](Self::poll) returns
    /// [`BatchStatus::Complete`].
    ///
    /// # Errors
    ///
    /// Returns an error if the batch is not complete or the transport fails.
    pub async fn results(
        &self,
        id: &BatchId,
    ) -> Result<Vec<BatchResponse>, Box<dyn std::error::Error + Send + Sync>> {
        self.transport.get_results(id).await
    }

    /// Estimate the cost savings from using batch mode.
    ///
    /// Returns the estimated savings as a fraction (0.5 = 50% savings).
    pub const fn cost_savings_fraction() -> f64 {
        0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // ── Mock transport ──────────────────────────────────────────────

    #[derive(Clone)]
    struct MockTransport {
        /// Submitted requests captured for assertions.
        submitted: Arc<Mutex<Vec<Vec<BatchRequest>>>>,
        /// Status to return from poll_batch.
        status: BatchStatus,
        /// Results to return from get_results.
        results: Vec<BatchResponse>,
        /// If set, submit_batch returns this error.
        submit_error: Option<String>,
    }

    impl MockTransport {
        fn new(status: BatchStatus, results: Vec<BatchResponse>) -> Self {
            Self {
                submitted: Arc::new(Mutex::new(Vec::new())),
                status,
                results,
                submit_error: None,
            }
        }

        fn with_submit_error(mut self, msg: &str) -> Self {
            self.submit_error = Some(msg.to_string());
            self
        }

        fn submitted_batches(&self) -> Vec<Vec<BatchRequest>> {
            self.submitted.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl BatchTransport for MockTransport {
        async fn submit_batch(
            &self,
            requests: &[BatchRequest],
        ) -> Result<BatchId, Box<dyn std::error::Error + Send + Sync>> {
            if let Some(ref err) = self.submit_error {
                return Err(err.clone().into());
            }
            self.submitted
                .lock()
                .unwrap()
                .push(requests.to_vec());
            Ok(BatchId::new("batch-001"))
        }

        async fn poll_batch(
            &self,
            _id: &BatchId,
        ) -> Result<BatchStatus, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.status)
        }

        async fn get_results(
            &self,
            _id: &BatchId,
        ) -> Result<Vec<BatchResponse>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.results.clone())
        }
    }

    // ── Helpers ─────────────────────────────────────────────────────

    fn sample_request(id: &str) -> BatchRequest {
        BatchRequest {
            custom_id: id.to_string(),
            model: "claude-sonnet-4-6".to_string(),
            system: "You are a helpful assistant.".to_string(),
            user_message: "Summarize this plan.".to_string(),
            max_tokens: 4096,
        }
    }

    fn sample_response(id: &str, content: &str) -> BatchResponse {
        BatchResponse {
            custom_id: id.to_string(),
            succeeded: true,
            content: content.to_string(),
            usage: BatchUsage {
                input_tokens: 100,
                output_tokens: 200,
            },
            error: None,
        }
    }

    // ── Tests ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn submit_batch_returns_batch_id() {
        let transport = MockTransport::new(BatchStatus::Pending, vec![]);
        let client = BatchClient::new(transport);
        let requests = vec![sample_request("req-1"), sample_request("req-2")];

        let batch_id = client.submit(requests).await.expect("submit should succeed");
        assert_eq!(batch_id.0, "batch-001");
    }

    #[tokio::test]
    async fn submit_captures_all_requests() {
        let transport = MockTransport::new(BatchStatus::Pending, vec![]);
        let client = BatchClient::new(transport.clone());
        let requests = vec![
            sample_request("a"),
            sample_request("b"),
            sample_request("c"),
        ];

        client.submit(requests).await.expect("submit");

        let batches = transport.submitted_batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 3);
        assert_eq!(batches[0][0].custom_id, "a");
        assert_eq!(batches[0][2].custom_id, "c");
    }

    #[tokio::test]
    async fn submit_empty_batch_returns_error() {
        let transport = MockTransport::new(BatchStatus::Pending, vec![]);
        let client = BatchClient::new(transport);

        let result = client.submit(vec![]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty batch"));
    }

    #[tokio::test]
    async fn poll_returns_current_status() {
        let transport = MockTransport::new(BatchStatus::Processing, vec![]);
        let client = BatchClient::new(transport);
        let id = BatchId::new("batch-001");

        let status = client.poll(&id).await.expect("poll");
        assert_eq!(status, BatchStatus::Processing);
    }

    #[tokio::test]
    async fn results_returns_all_responses() {
        let responses = vec![
            sample_response("req-1", "Result one"),
            sample_response("req-2", "Result two"),
        ];
        let transport = MockTransport::new(BatchStatus::Complete, responses);
        let client = BatchClient::new(transport);
        let id = BatchId::new("batch-001");

        let results = client.results(&id).await.expect("results");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].custom_id, "req-1");
        assert_eq!(results[0].content, "Result one");
        assert_eq!(results[1].custom_id, "req-2");
        assert_eq!(results[1].content, "Result two");
    }

    #[tokio::test]
    async fn submit_transport_error_propagates() {
        let transport =
            MockTransport::new(BatchStatus::Pending, vec![]).with_submit_error("network timeout");
        let client = BatchClient::new(transport);

        let result = client.submit(vec![sample_request("x")]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("network timeout"));
    }

    #[tokio::test]
    async fn batch_request_api_body_structure() {
        let req = sample_request("step-prd");
        let body = req.to_api_body();

        assert_eq!(body["custom_id"], "step-prd");
        assert_eq!(body["params"]["model"], "claude-sonnet-4-6");
        assert_eq!(body["params"]["max_tokens"], 4096);
        assert_eq!(body["params"]["messages"][0]["role"], "user");
        assert_eq!(
            body["params"]["messages"][0]["content"],
            "Summarize this plan."
        );
    }

    #[tokio::test]
    async fn batch_status_display() {
        assert_eq!(format!("{}", BatchStatus::Pending), "pending");
        assert_eq!(format!("{}", BatchStatus::Processing), "processing");
        assert_eq!(format!("{}", BatchStatus::Complete), "complete");
        assert_eq!(format!("{}", BatchStatus::Failed), "failed");
    }

    #[tokio::test]
    async fn cost_savings_is_fifty_percent() {
        let savings = BatchClient::<MockTransport>::cost_savings_fraction();
        assert!((savings - 0.5).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn batch_id_display_and_equality() {
        let id1 = BatchId::new("abc-123");
        let id2 = BatchId::new("abc-123");
        let id3 = BatchId::new("xyz-999");

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
        assert_eq!(format!("{id1}"), "abc-123");
    }

    #[tokio::test]
    async fn failed_response_has_error_field() {
        let resp = BatchResponse {
            custom_id: "req-fail".to_string(),
            succeeded: false,
            content: String::new(),
            usage: BatchUsage::default(),
            error: Some("rate limit exceeded".to_string()),
        };

        assert!(!resp.succeeded);
        assert!(resp.content.is_empty());
        assert_eq!(resp.error.as_deref(), Some("rate limit exceeded"));
    }
}
