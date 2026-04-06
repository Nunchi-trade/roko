//! Direct (real-time) LLM client abstraction.
//!
//! Provides request/response types and a trait for real-time LLM calls.
//! Unlike the batch client, direct calls return results immediately and
//! support streaming via an async chunk iterator.
//!
//! No HTTP logic lives here -- implementations live in the app layer.
//! (Anti-pattern #8: I/O at boundary only.)

use serde::{Deserialize, Serialize};

// ── Request / Response ──────────────────────────────────────────────────

/// A single message in a conversation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    /// Role of the message sender (`"user"` or `"assistant"`).
    pub role: String,
    /// Text content of the message.
    pub content: String,
}

impl Message {
    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
        }
    }

    /// Create an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
        }
    }
}

/// A request for a real-time LLM completion.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirectRequest {
    /// Model identifier (e.g. `"claude-sonnet-4-6"`).
    pub model: String,
    /// System prompt.
    pub system: Option<String>,
    /// Conversation messages.
    pub messages: Vec<Message>,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
    /// Sampling temperature (0.0 = deterministic, 1.0 = creative).
    pub temperature: f32,
}

impl DirectRequest {
    /// Create a simple single-turn request with system + user message.
    pub fn simple(
        model: impl Into<String>,
        system: impl Into<String>,
        user_message: impl Into<String>,
        max_tokens: u32,
    ) -> Self {
        Self {
            model: model.into(),
            system: Some(system.into()),
            messages: vec![Message::user(user_message)],
            max_tokens,
            temperature: 0.0,
        }
    }

    /// Set the temperature for this request.
    #[must_use]
    pub const fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature;
        self
    }

    /// Build the Anthropic Messages API request body.
    pub fn to_api_body(&self) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "temperature": self.temperature,
            "messages": self.messages.iter().map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                })
            }).collect::<Vec<_>>(),
        });

        if let Some(ref sys) = self.system {
            body["system"] = serde_json::json!(sys);
        }

        body
    }
}

/// Token usage information from a completion.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DirectUsage {
    /// Number of input tokens consumed.
    pub input_tokens: u32,
    /// Number of output tokens generated.
    pub output_tokens: u32,
}

impl DirectUsage {
    /// Total tokens (input + output).
    pub const fn total(&self) -> u32 {
        self.input_tokens + self.output_tokens
    }
}

/// Response from a real-time LLM completion.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirectResponse {
    /// Generated text content.
    pub content: String,
    /// Token usage statistics.
    pub usage: DirectUsage,
    /// Model that served the request (may differ from requested model
    /// if the provider substituted).
    pub model: String,
    /// Stop reason: `"end_turn"`, `"max_tokens"`, `"stop_sequence"`, etc.
    pub stop_reason: Option<String>,
}

/// A chunk from a streaming response.
#[derive(Clone, Debug)]
pub struct StreamChunk {
    /// Delta text content in this chunk.
    pub delta: String,
    /// Whether this is the final chunk.
    pub is_final: bool,
}

// ── Transport trait ─────────────────────────────────────────────────────

/// Trait abstracting the HTTP transport for real-time LLM calls.
///
/// Implementations handle the actual HTTP communication and live outside
/// this crate.
#[async_trait::async_trait]
pub trait DirectTransport: Send + Sync {
    /// Send a completion request and return the full response.
    async fn complete(
        &self,
        request: &DirectRequest,
    ) -> Result<DirectResponse, Box<dyn std::error::Error + Send + Sync>>;

    /// Send a streaming completion request and return chunks.
    ///
    /// The default implementation falls back to [`complete`](Self::complete)
    /// and returns the full response as a single chunk.
    async fn stream(
        &self,
        request: &DirectRequest,
    ) -> Result<Vec<StreamChunk>, Box<dyn std::error::Error + Send + Sync>> {
        let response = self.complete(request).await?;
        Ok(vec![StreamChunk {
            delta: response.content,
            is_final: true,
        }])
    }
}

// ── DirectClient ────────────────────────────────────────────────────────

/// High-level client for real-time LLM completions.
///
/// Wraps a [`DirectTransport`] implementation and provides convenience
/// methods for the enrichment pipeline.
pub struct DirectClient<T: DirectTransport> {
    transport: T,
}

impl<T: DirectTransport> DirectClient<T> {
    /// Create a new direct client with the given transport.
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Send a completion request and return the full response.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport fails.
    pub async fn complete(
        &self,
        request: DirectRequest,
    ) -> Result<DirectResponse, Box<dyn std::error::Error + Send + Sync>> {
        self.transport.complete(&request).await
    }

    /// Send a streaming request and return chunks.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport fails.
    pub async fn stream(
        &self,
        request: DirectRequest,
    ) -> Result<Vec<StreamChunk>, Box<dyn std::error::Error + Send + Sync>> {
        self.transport.stream(&request).await
    }

    /// Convenience: single-turn completion with system + user message.
    ///
    /// Returns just the generated text content.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport fails.
    pub async fn simple_complete(
        &self,
        model: &str,
        system: &str,
        user_message: &str,
        max_tokens: u32,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let request = DirectRequest::simple(model, system, user_message, max_tokens);
        let response = self.transport.complete(&request).await?;
        Ok(response.content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Mock transport ──────────────────────────────────────────────

    struct MockDirectTransport {
        response: Result<DirectResponse, String>,
        stream_chunks: Option<Vec<StreamChunk>>,
    }

    impl MockDirectTransport {
        fn ok(content: &str, input_tokens: u32, output_tokens: u32) -> Self {
            Self {
                response: Ok(DirectResponse {
                    content: content.to_string(),
                    usage: DirectUsage {
                        input_tokens,
                        output_tokens,
                    },
                    model: "claude-sonnet-4-6".to_string(),
                    stop_reason: Some("end_turn".to_string()),
                }),
                stream_chunks: None,
            }
        }

        fn err(msg: &str) -> Self {
            Self {
                response: Err(msg.to_string()),
                stream_chunks: None,
            }
        }

        fn with_stream_chunks(mut self, chunks: Vec<StreamChunk>) -> Self {
            self.stream_chunks = Some(chunks);
            self
        }
    }

    #[async_trait::async_trait]
    impl DirectTransport for MockDirectTransport {
        async fn complete(
            &self,
            _request: &DirectRequest,
        ) -> Result<DirectResponse, Box<dyn std::error::Error + Send + Sync>> {
            match &self.response {
                Ok(r) => Ok(r.clone()),
                Err(e) => Err(e.clone().into()),
            }
        }

        async fn stream(
            &self,
            request: &DirectRequest,
        ) -> Result<Vec<StreamChunk>, Box<dyn std::error::Error + Send + Sync>> {
            if let Some(ref chunks) = self.stream_chunks {
                return Ok(chunks.clone());
            }
            // Default: fall back to complete.
            let response = self.complete(request).await?;
            Ok(vec![StreamChunk {
                delta: response.content,
                is_final: true,
            }])
        }
    }

    // ── Tests ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn complete_returns_content_and_usage() {
        let transport = MockDirectTransport::ok("Hello, world!", 50, 10);
        let client = DirectClient::new(transport);

        let request = DirectRequest::simple(
            "claude-sonnet-4-6",
            "Be helpful.",
            "Say hello.",
            1024,
        );
        let resp = client.complete(request).await.expect("complete");

        assert_eq!(resp.content, "Hello, world!");
        assert_eq!(resp.usage.input_tokens, 50);
        assert_eq!(resp.usage.output_tokens, 10);
        assert_eq!(resp.usage.total(), 60);
        assert_eq!(resp.model, "claude-sonnet-4-6");
        assert_eq!(resp.stop_reason.as_deref(), Some("end_turn"));
    }

    #[tokio::test]
    async fn simple_complete_returns_content_string() {
        let transport = MockDirectTransport::ok("Generated output", 100, 50);
        let client = DirectClient::new(transport);

        let content = client
            .simple_complete("claude-sonnet-4-6", "System", "User msg", 2048)
            .await
            .expect("simple_complete");

        assert_eq!(content, "Generated output");
    }

    #[tokio::test]
    async fn complete_error_propagates() {
        let transport = MockDirectTransport::err("connection refused");
        let client = DirectClient::new(transport);

        let request = DirectRequest::simple("m", "s", "u", 100);
        let result = client.complete(request).await;

        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("connection refused")
        );
    }

    #[tokio::test]
    async fn stream_returns_multiple_chunks() {
        let chunks = vec![
            StreamChunk {
                delta: "Hello".to_string(),
                is_final: false,
            },
            StreamChunk {
                delta: ", world!".to_string(),
                is_final: true,
            },
        ];
        let transport = MockDirectTransport::ok("Hello, world!", 50, 10)
            .with_stream_chunks(chunks);
        let client = DirectClient::new(transport);

        let request = DirectRequest::simple("m", "s", "u", 100);
        let result = client.stream(request).await.expect("stream");

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].delta, "Hello");
        assert!(!result[0].is_final);
        assert_eq!(result[1].delta, ", world!");
        assert!(result[1].is_final);
    }

    #[tokio::test]
    async fn stream_fallback_returns_single_chunk() {
        // No explicit stream_chunks set, so falls back to complete().
        let transport = MockDirectTransport::ok("Full response", 30, 20);
        let client = DirectClient::new(transport);

        let request = DirectRequest::simple("m", "s", "u", 100);
        let result = client.stream(request).await.expect("stream fallback");

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].delta, "Full response");
        assert!(result[0].is_final);
    }

    #[tokio::test]
    async fn request_api_body_structure() {
        let req = DirectRequest::simple(
            "claude-sonnet-4-6",
            "You are helpful.",
            "What is 2+2?",
            512,
        )
        .with_temperature(0.7);

        let body = req.to_api_body();

        assert_eq!(body["model"], "claude-sonnet-4-6");
        assert_eq!(body["max_tokens"], 512);
        assert_eq!(body["system"], "You are helpful.");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "What is 2+2?");
        // Temperature should be close to 0.7 (floating point).
        let temp = body["temperature"].as_f64().unwrap();
        assert!((temp - 0.7).abs() < 0.01);
    }

    #[tokio::test]
    async fn request_without_system_omits_system_field() {
        let req = DirectRequest {
            model: "claude-haiku-4-5-20251001".to_string(),
            system: None,
            messages: vec![Message::user("hi")],
            max_tokens: 100,
            temperature: 0.0,
        };

        let body = req.to_api_body();
        assert!(body.get("system").is_none());
    }

    #[tokio::test]
    async fn message_constructors() {
        let user = Message::user("hello");
        assert_eq!(user.role, "user");
        assert_eq!(user.content, "hello");

        let asst = Message::assistant("hi back");
        assert_eq!(asst.role, "assistant");
        assert_eq!(asst.content, "hi back");
    }

    #[tokio::test]
    async fn usage_total() {
        let usage = DirectUsage {
            input_tokens: 100,
            output_tokens: 50,
        };
        assert_eq!(usage.total(), 150);
    }

    #[tokio::test]
    async fn with_temperature_builder() {
        let req = DirectRequest::simple("m", "s", "u", 100).with_temperature(0.5);
        assert!((req.temperature - 0.5).abs() < f32::EPSILON);
    }
}
