//! `ToolLoop` (§36.f) — multi-turn tool-calling driver.
//!
//! Owns the iterative `prompt -> LLM -> tool_calls? -> dispatch ->
//! results -> LLM -> ...` loop for "raw" backends (Ollama, `OpenAI`,
//! `ReAct`) where Roko drives the full conversation.  Claude CLI
//! drives its own internal loop and bypasses this entirely.
//!
//! # Submodules
//!
//! - [`max_iter`] — iteration-cap configuration (§36.54).
//! - [`compaction`] — gentle tool-result truncation (§36.58).
//! - [`prune`] — context-growth guard (§36.55).
//! - [`result_msg`] — tool-result message construction (§36.56).
//! - [`checkpoint`] — resumable state (§36.57).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use roko_core::{
    config::schema::ModelProfile,
    tool::{ToolCall, ToolContext, ToolDef},
};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::dispatcher::ToolDispatcher;
use crate::introspection::{Intervention, MetacognitiveMonitor, Turn};
use crate::lifecycle::{BudgetStatus, BudgetTracker, CognitiveTier, TurnCostRecord};
use crate::provider::ProviderError;
use crate::retry::RetryPolicy;
use crate::streaming::StreamChunk;
use crate::translate::{BackendResponse, RenderedTools, SessionState, Translator};
use crate::usage::Usage;

/// Per-turn progress information emitted by [`ToolLoop`] via its `on_turn` callback.
#[derive(Debug, Clone)]
pub struct TurnProgress {
    /// Zero-based iteration number.
    pub iteration: usize,
    /// Tool calls dispatched this turn.
    pub tool_calls: Vec<ToolCall>,
    /// Brief text summaries of tool results (truncated to 120 chars each).
    pub tool_results: Vec<String>,
    /// Any text the LLM produced alongside tool calls (often empty).
    pub text_output: String,
    /// Optional reasoning/thinking content the backend exposed for this turn.
    pub reasoning: Option<String>,
    /// Token usage reported for this backend turn.
    pub usage: Usage,
}

/// Type-erased callback invoked after each tool-dispatch iteration.
pub type OnTurnCallback = Arc<dyn Fn(&TurnProgress) + Send + Sync>;

/// Metadata captured for one backend turn of the tool loop.
#[derive(Debug, Clone)]
pub struct ToolLoopTurnTrace {
    /// One-based turn number within this tool loop run.
    pub turn: u32,
    /// Tool calls dispatched during this turn.
    pub tool_calls: Vec<ToolCall>,
    /// Brief text summaries of tool results, index-aligned with `tool_calls`.
    pub tool_results: Vec<String>,
    /// Optional reasoning/thinking content the backend exposed for this turn.
    pub reasoning: Option<String>,
    /// Token usage reported for this backend turn.
    pub usage: Usage,
}

pub mod agent_wrapper;
pub mod backends;
pub mod checkpoint;
pub mod compaction;
pub mod max_iter;
pub mod prune;
pub mod result_msg;

pub use agent_wrapper::ToolLoopAgent;
pub use backends::OpenAiCompatBackend;
pub use checkpoint::Checkpoint;
pub use max_iter::DEFAULT_MAX_ITERATIONS;
pub use prune::DEFAULT_CONTEXT_TOKEN_LIMIT;

// ─── LlmBackend trait ────────────────────────────────────────────────

/// Raw LLM conversation interface for the tool loop.
///
/// The **primary** method is [`stream_turn`](Self::stream_turn), which
/// returns a stream of [`StreamEvent`] values as the backend processes
/// the turn. [`send_turn`](Self::send_turn) is the blocking convenience
/// wrapper that collects the stream into a [`BackendResponse`].
///
/// Backends that support native streaming should override `stream_turn`.
/// Backends that do not can implement only `send_turn`; the default
/// `stream_turn` wraps it in a synthetic event stream.
///
/// This is intentionally lower-level than [`Agent`](crate::agent::Agent):
/// it models a single request-response round, not a full agent run.
#[async_trait]
pub trait LlmBackend: Send + Sync {
    /// Send the current conversation state to the backend (blocking).
    ///
    /// `messages` is the accumulated message history (system, user,
    /// assistant, tool-result messages).  `tools` is the pre-rendered
    /// tool spec from [`Translator::render_tools`].
    ///
    /// The default implementation drives [`stream_turn`](Self::stream_turn)
    /// and collects the events via [`collect_stream_to_response`].
    async fn send_turn(
        &self,
        messages: &[serde_json::Value],
        tools: &RenderedTools,
        session: &SessionState,
    ) -> Result<BackendResponse, LlmError>;

    /// PRIMARY: Return a stream of events for one LLM turn.
    ///
    /// The stream MUST emit at minimum one `TextDelta` (even if empty)
    /// before `Done`. The FIRST `TextDelta` event is used to measure
    /// TTFT -- emit it as soon as the first bytes arrive from the
    /// provider, before accumulating a complete message.
    ///
    /// The stream MUST emit `Done` as the final event. After `Done`,
    /// the stream ends.
    ///
    /// The default implementation calls [`send_turn`](Self::send_turn)
    /// and wraps the result in a synthetic stream via
    /// [`response_to_synthetic_stream`].
    ///
    /// Backends that support native streaming should override this
    /// method and return a real incremental stream.
    async fn stream_turn(
        &self,
        messages: &[serde_json::Value],
        tools: &RenderedTools,
        session: &SessionState,
        _config: &TurnConfig,
    ) -> Result<futures::stream::BoxStream<'static, Result<StreamEvent, LlmError>>, LlmError> {
        let response = self.send_turn(messages, tools, session).await?;
        Ok(response_to_synthetic_stream(response))
    }

    /// Extract provider-issued session or conversation identifiers from a turn response.
    fn extract_session(&self, response: &BackendResponse) -> SessionState {
        let _ = response;
        SessionState::default()
    }

    /// Stable backend identifier for audit and episode logging.
    fn backend_id(&self) -> &'static str {
        "unknown"
    }

    /// Send the current conversation state to the backend in streaming mode.
    ///
    /// **Deprecated**: prefer [`stream_turn`](Self::stream_turn) for new code.
    /// This method exists for backward compatibility with callers that use
    /// the channel-based `StreamChunk` API. The default calls `send_turn`.
    ///
    /// Backends with native streaming (e.g. `OpenAiCompatLlmBackend`)
    /// override this to emit `StreamChunk` values into the channel
    /// as they arrive.
    // TODO(082): migrate callers to stream_turn, then remove this method.
    async fn send_turn_streaming(
        &self,
        messages: &[serde_json::Value],
        tools: &RenderedTools,
        session: &SessionState,
        event_tx: mpsc::Sender<StreamChunk>,
    ) -> Result<BackendResponse, LlmError> {
        let _ = event_tx;
        self.send_turn(messages, tools, session).await
    }
}

/// Errors from an [`LlmBackend`].
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// The backend returned a non-success status or an API error.
    #[error("backend error: {0}")]
    Backend(String),
    /// A network-level failure (DNS, timeout, connection reset).
    #[error("network error: {0}")]
    Network(String),
    /// The backend returned a provider-classified error suitable for retry logic.
    #[error("provider error: {0}")]
    Provider(ProviderError),
    /// Retry budget was exhausted before a successful backend response arrived.
    #[error("retries exhausted")]
    RetriesExhausted,
    /// The backend timed out (TTFT or total request).
    #[error("timeout: {0}")]
    Timeout(String),
}

// ─── Streaming-first types (task 082) ─────────────────────────────────

/// A single event emitted by a streaming LLM backend during one turn.
///
/// Events arrive in order. The sequence for a normal turn is:
/// `TextDelta*` -> `ToolCallStart?` -> `ToolCallDelta*` -> `ToolCallEnd?` -> `Usage` -> `Done`
///
/// A turn may have multiple interleaved `ToolCall*` sequences for parallel tool calls.
#[derive(Debug, Clone)]
pub struct StreamEvent {
    /// The kind of streaming event.
    pub kind: StreamEventKind,
    /// Monotonic timestamp -- used to measure TTFT from first `TextDelta`.
    pub timestamp: std::time::Instant,
}

impl StreamEvent {
    /// Convenience constructor: wrap a kind with the current instant.
    #[must_use]
    pub fn now(kind: StreamEventKind) -> Self {
        Self {
            kind,
            timestamp: std::time::Instant::now(),
        }
    }
}

/// The kind of a [`StreamEvent`].
#[derive(Debug, Clone)]
pub enum StreamEventKind {
    /// Incremental assistant-visible content text.
    ///
    /// Always emitted even if the text delta is empty, so callers can
    /// measure TTFT independently of text content.
    TextDelta(String),

    /// Incremental reasoning/thinking text from the model.
    ReasoningDelta(String),

    /// A tool call is starting. Emitted before `ToolCallDelta` events.
    ToolCallStart {
        /// Provider-assigned tool call identifier.
        id: String,
        /// Tool/function name.
        name: String,
    },

    /// Partial JSON arguments for an in-progress tool call.
    ToolCallDelta {
        /// Provider-assigned tool call identifier.
        id: String,
        /// Incremental JSON argument text.
        json_fragment: String,
    },

    /// A tool call is complete with fully assembled arguments.
    ToolCallEnd {
        /// Provider-assigned tool call identifier.
        id: String,
        /// Tool/function name.
        name: String,
        /// Fully assembled JSON arguments.
        args: serde_json::Value,
    },

    /// Final usage statistics for this turn.
    Usage(Usage),

    /// The turn is complete. No more events will follow.
    Done {
        /// Provider finish reason string.
        finish_reason: String,
    },
}

/// Convert a [`StreamChunk`] into a [`StreamEvent`] with the current timestamp.
impl From<StreamChunk> for StreamEvent {
    fn from(chunk: StreamChunk) -> Self {
        let kind = match chunk {
            StreamChunk::ContentDelta(text) => StreamEventKind::TextDelta(text),
            StreamChunk::ReasoningDelta(text) => StreamEventKind::ReasoningDelta(text),
            StreamChunk::ToolCallDelta {
                index: _,
                id_delta,
                name_delta,
                arguments_delta,
            } => {
                // When both id and name are present, this is a start + delta.
                // When only arguments are present, it's a delta.
                if id_delta.is_some() || name_delta.is_some() {
                    // For the combined start case, we emit a ToolCallStart.
                    // Callers who need the delta portion should use the raw
                    // streaming path; this conversion is for compatibility.
                    StreamEventKind::ToolCallStart {
                        id: id_delta.unwrap_or_default(),
                        name: name_delta.unwrap_or_default(),
                    }
                } else {
                    StreamEventKind::ToolCallDelta {
                        id: String::new(),
                        json_fragment: arguments_delta,
                    }
                }
            }
            StreamChunk::Usage(usage) => StreamEventKind::Usage(usage),
            StreamChunk::Done(finish_reason) => StreamEventKind::Done {
                finish_reason: format!("{finish_reason:?}"),
            },
            StreamChunk::Error(msg) => StreamEventKind::Done {
                finish_reason: format!("error: {msg}"),
            },
        };
        StreamEvent {
            kind,
            timestamp: std::time::Instant::now(),
        }
    }
}

/// Per-turn configuration that replaces scattered parameters.
///
/// Previously these were passed as individual arguments or threaded through
/// session state. `TurnConfig` consolidates them into one struct so backends
/// can be called with a consistent interface.
#[derive(Debug, Clone)]
pub struct TurnConfig {
    /// Maximum output tokens. Taken from model profile or `DEFAULT_MAX_OUTPUT_TOKENS`.
    pub max_tokens: u32,
    /// Optional sampling temperature. `None` = provider default.
    pub temperature: Option<f32>,
    /// Timeout for the first token to arrive. After this, the backend
    /// should return `LlmError::Timeout` rather than waiting indefinitely.
    pub ttft_timeout: Duration,
    /// Total request timeout including all streaming. The backend must
    /// complete the stream within this duration or return `LlmError::Timeout`.
    pub request_timeout: Duration,
    /// Stop sequences. Provider-specific; pass through verbatim.
    pub stop_sequences: Vec<String>,
}

impl Default for TurnConfig {
    fn default() -> Self {
        use roko_core::defaults::{DEFAULT_MAX_OUTPUT_TOKENS, DEFAULT_REQUEST_TIMEOUT_MS, DEFAULT_TTFT_TIMEOUT_MS};
        Self {
            max_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
            temperature: None,
            ttft_timeout: Duration::from_millis(DEFAULT_TTFT_TIMEOUT_MS),
            request_timeout: Duration::from_millis(DEFAULT_REQUEST_TIMEOUT_MS),
            stop_sequences: vec![],
        }
    }
}

/// Collect a `StreamEvent` stream into a [`BackendResponse`] and capture TTFT.
///
/// This is the default implementation used by [`LlmBackend::send_turn`]'s
/// default impl. It reassembles text deltas, tool calls, and usage from
/// the stream into a single JSON response.
pub async fn collect_stream_to_response(
    mut stream: futures::stream::BoxStream<'static, Result<StreamEvent, LlmError>>,
    request_start: std::time::Instant,
) -> Result<BackendResponse, LlmError> {
    use futures::StreamExt;

    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls: Vec<roko_core::tool::ToolCall> = vec![];
    let mut usage = Usage::default();
    let mut finish_reason = "stop".to_string();
    let mut ttft_ms: Option<u64> = None;
    // Track in-progress tool calls: id -> (name, accumulated_args)
    let mut in_progress_calls: std::collections::HashMap<String, (String, String)> =
        Default::default();

    while let Some(event) = stream.next().await {
        let event = event?;
        match event.kind {
            StreamEventKind::TextDelta(delta) => {
                if ttft_ms.is_none() {
                    ttft_ms = Some(request_start.elapsed().as_millis() as u64);
                }
                text.push_str(&delta);
            }
            StreamEventKind::ReasoningDelta(delta) => {
                reasoning.push_str(&delta);
            }
            StreamEventKind::ToolCallStart { id, name } => {
                in_progress_calls.insert(id, (name, String::new()));
            }
            StreamEventKind::ToolCallDelta { id, json_fragment } => {
                if let Some((_name, args)) = in_progress_calls.get_mut(&id) {
                    args.push_str(&json_fragment);
                }
            }
            StreamEventKind::ToolCallEnd { id, name, args } => {
                in_progress_calls.remove(&id);
                tool_calls.push(roko_core::tool::ToolCall::new(id, name, args));
            }
            StreamEventKind::Usage(u) => usage = u,
            StreamEventKind::Done { finish_reason: fr } => {
                finish_reason = fr;
            }
        }
    }

    // Flush any in-progress calls that got deltas but no ToolCallEnd event.
    // This handles the common case where providers send start+delta but
    // the stream ends before a formal end event.
    for (id, (name, args_str)) in in_progress_calls {
        let args = if args_str.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&args_str)
                .unwrap_or_else(|_| serde_json::Value::String(args_str))
        };
        tool_calls.push(roko_core::tool::ToolCall::new(id, name, args));
    }

    // Build an OpenAI-shaped JSON response so existing translators work.
    let tool_calls_json: Vec<serde_json::Value> = tool_calls
        .iter()
        .map(|tc| {
            serde_json::json!({
                "id": tc.id,
                "type": "function",
                "function": {
                    "name": tc.name,
                    "arguments": tc.arguments.to_string(),
                }
            })
        })
        .collect();

    let mut message = serde_json::json!({
        "role": "assistant",
        "content": text,
    });
    if !reasoning.is_empty() {
        message["reasoning_content"] = serde_json::Value::String(reasoning);
    }
    if !tool_calls_json.is_empty() {
        message["tool_calls"] = serde_json::Value::Array(tool_calls_json);
    }

    let mut json = serde_json::json!({
        "choices": [{
            "message": message,
            "finish_reason": finish_reason,
        }],
        "usage": {
            "prompt_tokens": usage.input_tokens,
            "completion_tokens": usage.output_tokens,
            "total_tokens": usage.input_tokens + usage.output_tokens,
            "prompt_tokens_details": {
                "cached_tokens": usage.cache_read_tokens,
            },
        },
    });

    // Stash TTFT in metadata for downstream consumers.
    if let Some(ttft) = ttft_ms {
        json["metadata"] = serde_json::json!({ "provider_ttft_ms": ttft });
    }

    Ok(BackendResponse::Json(json))
}

/// Convert a `BackendResponse` into a synthetic single-event stream.
///
/// Used by backends that do not implement native streaming: wraps the
/// complete response as a sequence of `TextDelta` + `Done` events.
pub fn response_to_synthetic_stream(
    response: BackendResponse,
) -> futures::stream::BoxStream<'static, Result<StreamEvent, LlmError>> {
    use futures::stream;

    let text = response.extract_text();
    let usage = response.extract_usage();
    let finish_reason = response
        .extract_finish_reason_raw()
        .unwrap_or_else(|| "stop".to_string());

    let events = vec![
        Ok(StreamEvent::now(StreamEventKind::TextDelta(text))),
        Ok(StreamEvent::now(StreamEventKind::Usage(usage))),
        Ok(StreamEvent::now(StreamEventKind::Done { finish_reason })),
    ];

    Box::pin(stream::iter(events))
}

// ─── StopReason + Output ─────────────────────────────────────────────

/// Why the tool loop stopped iterating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// The LLM returned a response with no tool calls — the final answer.
    Stop,
    /// The iteration budget was exhausted (default: 25).
    MaxIterations,
    /// The [`ToolContext`]'s cancel token was tripped between turns.
    Cancelled,
    /// The backend returned an error (API, parse, or network).
    BackendError(String),
    /// The cost budget was exhausted (daily or lifetime limit reached).
    BudgetExhausted,
}

/// Output from a completed [`ToolLoop::run`] or [`ToolLoop::resume`].
#[derive(Debug, Clone)]
pub struct ToolLoopOutput {
    /// The LLM's final text (empty when `stop_reason` is not `Stop`).
    pub final_text: String,
    /// Number of tool-call iterations that executed.
    pub iterations: usize,
    /// All tool calls dispatched across every iteration.
    pub tool_calls: Vec<ToolCall>,
    /// Aggregated usage across every backend turn in the loop.
    pub total_usage: Usage,
    /// Why the loop stopped.
    pub stop_reason: StopReason,
    /// Resumable snapshot — populated when `stop_reason != Stop`.
    pub checkpoint: Option<Checkpoint>,
    /// Per-turn trace metadata captured during the loop.
    pub turn_traces: Vec<ToolLoopTurnTrace>,
    /// MCP errors accumulated during the session (non-blocking, informational).
    ///
    /// Populated when an [`McpErrorAccumulator`](crate::mcp::McpErrorAccumulator)
    /// is attached to the MCP handler resolver. Empty if no MCP errors occurred
    /// or no accumulator was configured.
    pub mcp_errors: Vec<crate::mcp::McpErrorRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverflowAction {
    Ok,
    CompactRecommended,
    CompactRequired,
}

fn trace_turn(iterations: usize) -> u32 {
    let capped = iterations.saturating_add(1).min(u32::MAX as usize);
    capped as u32
}

fn tool_result_previews(results: &[(ToolCall, roko_core::tool::ToolResult)]) -> Vec<String> {
    results
        .iter()
        .map(|(_call, result)| match result {
            roko_core::tool::ToolResult::Ok { content, .. } => truncate_preview(content, 120),
            roko_core::tool::ToolResult::Err(err) => {
                truncate_preview(&format!("error: {err}"), 120)
            }
        })
        .collect()
}

fn truncate_preview(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let keep = max_chars.saturating_sub(3);
    let mut preview = text.chars().take(keep).collect::<String>();
    preview.push_str("...");
    preview
}

// ─── ToolLoop ────────────────────────────────────────────────────────

/// Multi-turn tool-calling loop (§36.f).
///
/// Drives the `prompt -> LLM -> tool_calls -> dispatch -> results -> LLM`
/// cycle until the LLM stops calling tools, the iteration cap is
/// reached, the cancel token fires, or the backend errors.
/// Shared budget guard that can be attached to a [`ToolLoop`] to enforce
/// budget constraints before each LLM invocation.
pub type SharedBudgetTracker = Arc<parking_lot::Mutex<BudgetTracker>>;

#[derive(Clone)]
pub struct ToolLoop {
    translator: Arc<dyn Translator>,
    dispatcher: Arc<ToolDispatcher>,
    backend: Arc<dyn LlmBackend>,
    max_iterations: usize,
    context_token_limit: usize,
    checkpoint_path: Option<PathBuf>,
    model_profile: Option<ModelProfile>,
    retry_policy: RetryPolicy,
    monitor: Option<MetacognitiveMonitor>,
    /// Optional budget tracker checked before each LLM call (LIFE-03).
    budget: Option<SharedBudgetTracker>,
    /// Optional callback fired after each tool-dispatch iteration.
    on_turn: Option<OnTurnCallback>,
    /// Few-shot example messages inserted between system and user prompt.
    /// Dramatically improves tool-call reliability for small models.
    few_shot_messages: Vec<Value>,
    /// Optional MCP error accumulator for IDE/ACP sessions.
    /// When attached, MCP tool failures are recorded here non-blockingly.
    mcp_error_accumulator: Option<crate::mcp::McpErrorAccumulator>,
}

impl ToolLoop {
    /// Construct a tool loop with default caps.
    #[must_use]
    pub fn new(
        translator: Arc<dyn Translator>,
        dispatcher: Arc<ToolDispatcher>,
        backend: Arc<dyn LlmBackend>,
    ) -> Self {
        Self {
            translator,
            dispatcher,
            backend,
            max_iterations: DEFAULT_MAX_ITERATIONS,
            context_token_limit: DEFAULT_CONTEXT_TOKEN_LIMIT,
            checkpoint_path: None,
            model_profile: None,
            retry_policy: RetryPolicy::default(),
            monitor: None,
            budget: None,
            on_turn: None,
            few_shot_messages: Vec::new(),
            mcp_error_accumulator: None,
        }
    }

    /// Stable identifier for the backing LLM implementation.
    #[must_use]
    pub fn backend_id(&self) -> &'static str {
        self.backend.backend_id()
    }

    /// Override the default iteration cap (25).
    #[must_use]
    pub const fn with_max_iterations(mut self, n: usize) -> Self {
        self.max_iterations = n;
        self
    }

    /// Override the default context-token limit.
    #[must_use]
    pub const fn with_context_token_limit(mut self, n: usize) -> Self {
        self.context_token_limit = n;
        self
    }

    /// Persist resumable checkpoints at the provided path.
    #[must_use]
    pub fn with_checkpoint_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.checkpoint_path = Some(path.into());
        self
    }

    /// Configure the model profile used for context-overflow detection.
    #[must_use]
    pub fn with_model_profile(mut self, model_profile: ModelProfile) -> Self {
        self.model_profile = Some(model_profile);
        self
    }

    /// Override the default retry policy used for provider-backed backend calls.
    #[must_use]
    pub fn with_retry_policy(mut self, retry_policy: RetryPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }

    /// Attach a metacognitive monitor.
    #[must_use]
    pub fn with_monitor(mut self, monitor: MetacognitiveMonitor) -> Self {
        self.monitor = Some(monitor);
        self
    }

    /// Attach a shared budget tracker checked before each LLM call.
    ///
    /// When the tracker reports [`BudgetStatus::Exhausted`], the loop
    /// terminates with [`StopReason::BudgetExhausted`]. After each
    /// successful LLM call, the turn cost is recorded into the tracker.
    #[must_use]
    pub fn with_budget(mut self, budget: SharedBudgetTracker) -> Self {
        self.budget = Some(budget);
        self
    }

    /// Attach a callback fired after each tool-dispatch iteration.
    ///
    /// The callback receives a [`TurnProgress`] snapshot of what happened
    /// in the iteration — which tools were called, brief result summaries,
    /// and any text the model emitted alongside tool calls.
    #[must_use]
    pub fn with_on_turn(mut self, cb: OnTurnCallback) -> Self {
        self.on_turn = Some(cb);
        self
    }

    /// Inject few-shot example messages between system and user prompts.
    ///
    /// These messages demonstrate correct tool-call behavior and
    /// dramatically improve reliability for small models (8B and below).
    #[must_use]
    pub fn with_few_shot_messages(mut self, messages: Vec<Value>) -> Self {
        self.few_shot_messages = messages;
        self
    }

    /// Attach an MCP error accumulator for IDE/ACP sessions.
    ///
    /// When set, MCP tool call failures during the session are recorded
    /// non-blockingly. Accumulated errors are surfaced in the
    /// [`ToolLoopOutput::mcp_errors`] field when the loop completes.
    #[must_use]
    pub fn with_mcp_error_accumulator(
        mut self,
        accumulator: crate::mcp::McpErrorAccumulator,
    ) -> Self {
        self.mcp_error_accumulator = Some(accumulator);
        self
    }

    /// Borrow the attached MCP error accumulator, if any.
    #[must_use]
    pub fn mcp_error_accumulator(&self) -> Option<&crate::mcp::McpErrorAccumulator> {
        self.mcp_error_accumulator.as_ref()
    }

    /// Build a [`TurnConfig`] from the current model profile and defaults.
    ///
    /// This consolidates the scattered parameters that previously had to be
    /// threaded individually. Backends receive this via `stream_turn`.
    #[must_use]
    pub fn turn_config(&self) -> TurnConfig {
        use roko_core::defaults::{DEFAULT_MAX_OUTPUT_TOKENS, DEFAULT_REQUEST_TIMEOUT_MS, DEFAULT_TTFT_TIMEOUT_MS};

        let max_tokens = self
            .model_profile
            .as_ref()
            .and_then(|p| p.max_output)
            .map(|v| v as u32)
            .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);

        TurnConfig {
            max_tokens,
            temperature: None,
            ttft_timeout: Duration::from_millis(DEFAULT_TTFT_TIMEOUT_MS),
            request_timeout: Duration::from_millis(DEFAULT_REQUEST_TIMEOUT_MS),
            stop_sequences: vec![],
        }
    }

    /// Run a fresh tool loop from an initial system + user prompt.
    pub async fn run(
        &self,
        system: &str,
        user: &str,
        tools: &[ToolDef],
        ctx: &ToolContext,
    ) -> ToolLoopOutput {
        if let Some(path) = self.checkpoint_path.as_deref() {
            match Checkpoint::load(path) {
                Ok(cp) => return self.resume(cp, tools, ctx).await,
                Err(err) => {
                    let is_not_found = matches!(
                        &err,
                        roko_core::RokoError::Io(e) if e.kind() == std::io::ErrorKind::NotFound
                    );
                    if !is_not_found {
                        return ToolLoopOutput {
                            final_text: String::new(),
                            iterations: 0,
                            tool_calls: Vec::new(),
                            total_usage: Usage::default(),
                            stop_reason: StopReason::BackendError(format!(
                                "checkpoint load {}: {err}",
                                path.display()
                            )),
                            checkpoint: None,
                            turn_traces: Vec::new(),
                            mcp_errors: Vec::new(),
                        };
                    }
                    // NotFound: no checkpoint yet, continue with fresh run
                }
            }
        }

        let messages = if self.few_shot_messages.is_empty() {
            result_msg::initial_messages(system, user)
        } else {
            result_msg::initial_messages_with_few_shot(system, user, &self.few_shot_messages)
        };
        self.run_inner_with_mcp_errors(
            messages,
            0,
            Vec::new(),
            Usage::default(),
            tools,
            ctx,
            None,
            SessionState::default(),
        )
        .await
    }

    /// Run a fresh tool loop and forward streaming chunks as each backend turn arrives.
    pub async fn run_streaming(
        &self,
        system: &str,
        user: &str,
        tools: &[ToolDef],
        ctx: &ToolContext,
        event_tx: mpsc::Sender<StreamChunk>,
    ) -> ToolLoopOutput {
        let messages = if self.few_shot_messages.is_empty() {
            result_msg::initial_messages(system, user)
        } else {
            result_msg::initial_messages_with_few_shot(system, user, &self.few_shot_messages)
        };
        self.run_inner_with_mcp_errors(
            messages,
            0,
            Vec::new(),
            Usage::default(),
            tools,
            ctx,
            Some(event_tx),
            SessionState::default(),
        )
        .await
    }

    /// Run a fresh streaming tool loop from an already-built message history.
    ///
    /// ACP and other chat surfaces may already have a normalized
    /// system/history/user message array. This keeps those callers on the
    /// shared tool-loop runtime without flattening history back into a single
    /// user prompt.
    pub async fn run_messages_streaming(
        &self,
        messages: Vec<Value>,
        tools: &[ToolDef],
        ctx: &ToolContext,
        event_tx: mpsc::Sender<StreamChunk>,
    ) -> ToolLoopOutput {
        self.run_inner_with_mcp_errors(
            messages,
            0,
            Vec::new(),
            Usage::default(),
            tools,
            ctx,
            Some(event_tx),
            SessionState::default(),
        )
        .await
    }

    /// Resume a tool loop from a previously saved [`Checkpoint`].
    pub async fn resume(
        &self,
        cp: Checkpoint,
        tools: &[ToolDef],
        ctx: &ToolContext,
    ) -> ToolLoopOutput {
        self.run_inner_with_mcp_errors(
            cp.messages,
            cp.iterations,
            cp.tool_calls,
            Usage::default(),
            tools,
            ctx,
            None,
            cp.session,
        )
        .await
    }

    /// Wrapper around `run_inner` that drains the MCP error accumulator
    /// into the output's `mcp_errors` field after the loop completes.
    #[allow(clippy::too_many_arguments)]
    async fn run_inner_with_mcp_errors(
        &self,
        messages: Vec<serde_json::Value>,
        iterations: usize,
        all_calls: Vec<ToolCall>,
        total_usage: Usage,
        tools: &[ToolDef],
        ctx: &ToolContext,
        event_tx: Option<mpsc::Sender<StreamChunk>>,
        session: SessionState,
    ) -> ToolLoopOutput {
        let mut output = self
            .run_inner(
                messages,
                iterations,
                all_calls,
                total_usage,
                tools,
                ctx,
                event_tx,
                session,
            )
            .await;

        // Drain accumulated MCP errors into the output for the caller to inspect.
        if let Some(ref accumulator) = self.mcp_error_accumulator {
            output.mcp_errors = accumulator.drain();
        }

        output
    }

    /// Core loop shared by [`run`](Self::run) and [`resume`](Self::resume).
    async fn run_inner(
        &self,
        mut messages: Vec<serde_json::Value>,
        mut iterations: usize,
        mut all_calls: Vec<ToolCall>,
        mut total_usage: Usage,
        tools: &[ToolDef],
        ctx: &ToolContext,
        event_tx: Option<mpsc::Sender<StreamChunk>>,
        initial_session: SessionState,
    ) -> ToolLoopOutput {
        let rendered_tools = self.translator.render_tools(tools);
        let mut session = initial_session;
        let mut turn_history: Vec<Turn> = Vec::new();
        let mut turn_traces: Vec<ToolLoopTurnTrace> = Vec::new();

        loop {
            self.prune_context_if_needed(&mut messages);

            // §36.54 — iteration cap.
            if max_iter::is_exhausted(iterations, self.max_iterations) {
                let cp = Checkpoint::new(iterations, all_calls.clone(), messages)
                    .with_session(session.clone());
                return ToolLoopOutput {
                    final_text: String::new(),
                    iterations,
                    tool_calls: all_calls,
                    total_usage,
                    stop_reason: StopReason::MaxIterations,
                    checkpoint: Some(cp),
                    turn_traces,
                    mcp_errors: Vec::new(),
                };
            }

            // §36.45 — cancellation between turns.
            if ctx.is_cancelled() {
                let cp = Checkpoint::new(iterations, all_calls.clone(), messages)
                    .with_session(session.clone());
                return ToolLoopOutput {
                    final_text: String::new(),
                    iterations,
                    tool_calls: all_calls,
                    total_usage,
                    stop_reason: StopReason::Cancelled,
                    checkpoint: Some(cp),
                    turn_traces,
                    mcp_errors: Vec::new(),
                };
            }

            // LIFE-03: Budget check before each LLM invocation.
            if let Some(ref budget) = self.budget {
                let guard = budget.lock();
                if guard.check() == BudgetStatus::Exhausted {
                    let cp = Checkpoint::new(iterations, all_calls.clone(), messages)
                        .with_session(session.clone());
                    return ToolLoopOutput {
                        final_text: String::new(),
                        iterations,
                        tool_calls: all_calls,
                        total_usage,
                        stop_reason: StopReason::BudgetExhausted,
                        checkpoint: Some(cp),
                        turn_traces,
                        mcp_errors: Vec::new(),
                    };
                }
            }

            // Send current conversation to the backend.
            let response = match match &event_tx {
                Some(event_tx) => {
                    self.backend
                        .send_turn_streaming(&messages, &rendered_tools, &session, event_tx.clone())
                        .await
                }
                None => {
                    self.send_turn_with_retry(&messages, &rendered_tools, &session)
                        .await
                }
            } {
                Ok(r) => r,
                Err(e) => {
                    let cp = Checkpoint::new(iterations, all_calls.clone(), messages)
                        .with_session(session.clone());
                    return ToolLoopOutput {
                        final_text: String::new(),
                        iterations,
                        tool_calls: all_calls,
                        total_usage,
                        stop_reason: StopReason::BackendError(e.to_string()),
                        checkpoint: Some(cp),
                        turn_traces,
                        mcp_errors: Vec::new(),
                    };
                }
            };
            merge_session_state(&mut session, self.backend.extract_session(&response));
            let turn_reasoning = response.extract_reasoning();
            let mut turn_usage = response.extract_usage();

            // Compute cost from model profile pricing when the provider did not
            // report a dollar amount (all OpenAI-compat backends).
            if let Some(profile) = self.model_profile.as_ref() {
                turn_usage.fill_cost_from_pricing(
                    profile.cost_input_per_m,
                    profile.cost_output_per_m,
                    profile.cost_cache_read_per_m,
                );
            }

            total_usage.add(&turn_usage);

            // LIFE-03: Record turn cost in budget tracker after LLM call.
            if let Some(ref budget) = self.budget {
                let mut guard = budget.lock();
                let model_name = self
                    .model_profile
                    .as_ref()
                    .map(|p| p.slug.clone())
                    .unwrap_or_default();
                let cost_record = TurnCostRecord {
                    turn_id: format!("turn-{iterations}"),
                    model: model_name,
                    input_tokens: u64::from(turn_usage.input_tokens),
                    output_tokens: u64::from(turn_usage.output_tokens),
                    cache_read_tokens: u64::from(turn_usage.cache_read_tokens),
                    estimated_cost_usd: f64::from(turn_usage.cost_usd),
                    cognitive_tier: CognitiveTier::Gamma,
                    t0_suppressed: false,
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                };
                guard.record_turn(&cost_record);
            }

            // Parse tool calls from the response.
            let calls = match self.translator.parse_calls(&response) {
                Ok(c) => c,
                Err(e) => {
                    let cp = Checkpoint::new(iterations, all_calls.clone(), messages)
                        .with_session(session.clone());
                    return ToolLoopOutput {
                        final_text: String::new(),
                        iterations,
                        tool_calls: all_calls,
                        total_usage,
                        stop_reason: StopReason::BackendError(format!("parse: {e}")),
                        checkpoint: Some(cp),
                        turn_traces,
                        mcp_errors: Vec::new(),
                    };
                }
            };

            // No tool calls -> final answer.
            if calls.is_empty() {
                self.clear_checkpoint_file();
                let final_text = response.extract_text();
                turn_traces.push(ToolLoopTurnTrace {
                    turn: trace_turn(iterations),
                    tool_calls: Vec::new(),
                    tool_results: Vec::new(),
                    reasoning: turn_reasoning,
                    usage: turn_usage,
                });
                let finish_reason_raw = response.extract_finish_reason_raw();
                let hit_length_limit = finish_reason_raw
                    .as_deref()
                    .is_some_and(|r| r == "length" || r == "max_tokens");
                tracing::info!(
                    iterations,
                    final_text_len = final_text.len(),
                    final_text_empty = final_text.trim().is_empty(),
                    finish_reason = ?finish_reason_raw,
                    input_tokens = turn_usage.input_tokens,
                    output_tokens = turn_usage.output_tokens,
                    "tool_loop: stop — no tool calls, returning final text"
                );
                if final_text.trim().is_empty() && hit_length_limit {
                    tracing::error!(
                        iterations,
                        output_tokens = turn_usage.output_tokens,
                        "tool_loop: model hit output token limit (finish_reason=length) \
                         and produced no final text — increase max_output for this model"
                    );
                } else if final_text.trim().is_empty() {
                    tracing::warn!(
                        iterations,
                        "tool_loop: final text is empty — model may have returned \
                         content in an unexpected format"
                    );
                }

                let stop_reason = if hit_length_limit {
                    StopReason::BackendError(
                        "model hit output token limit (finish_reason=length)".to_string(),
                    )
                } else {
                    StopReason::Stop
                };
                return ToolLoopOutput {
                    final_text,
                    iterations,
                    tool_calls: all_calls,
                    total_usage,
                    stop_reason,
                    checkpoint: None,
                    turn_traces,
                    mcp_errors: Vec::new(),
                };
            }

            // Inject the assistant's tool-call message into conversation history.
            if let Some(assistant_msg) = self.translator.render_assistant_message(&response) {
                messages.push(assistant_msg);
            }

            // Dispatch tool calls (§36.41 parallel/serial batching).
            let call_names: Vec<&str> = calls.iter().map(|c| c.name.as_str()).collect();
            tracing::info!(
                iteration = iterations,
                num_calls = calls.len(),
                tools = ?call_names,
                "tool_loop: dispatching tool calls"
            );
            let current_calls = calls.clone();
            let results = self.dispatcher.dispatch_batch(calls, ctx).await;
            all_calls.extend(current_calls.clone());
            let tool_results = tool_result_previews(&results);

            // §36.56 — shape results into messages for the next turn.
            let rendered_results = self.translator.render_results(&results);
            result_msg::append_results(&mut messages, rendered_results);

            turn_traces.push(ToolLoopTurnTrace {
                turn: trace_turn(iterations),
                tool_calls: current_calls.clone(),
                tool_results: tool_results.clone(),
                reasoning: turn_reasoning.clone(),
                usage: turn_usage,
            });

            // Fire on_turn callback with a snapshot of this iteration.
            if let Some(ref cb) = self.on_turn {
                let text_output = response.extract_text();
                cb(&TurnProgress {
                    iteration: iterations,
                    tool_calls: current_calls.clone(),
                    tool_results,
                    text_output,
                    reasoning: turn_reasoning,
                    usage: turn_usage,
                });
            }

            // Metacognitive intervention point: analyze the turn before the
            // conversation advances.
            if let Some(monitor) = self.monitor.as_ref() {
                let turn = Turn::from_response(iterations, &response, current_calls);
                turn_history.push(turn);
                if let Some(intervention) = monitor.check(&turn_history) {
                    match intervention {
                        Intervention::InjectReflection(message) => {
                            messages.push(serde_json::json!({
                                "role": "system",
                                "content": message,
                            }));
                        }
                        Intervention::EscalateModel
                        | Intervention::HumanHandoff
                        | Intervention::Abort => {
                            let cp = Checkpoint::new(iterations, all_calls.clone(), messages)
                                .with_session(session.clone());
                            return ToolLoopOutput {
                                final_text: String::new(),
                                iterations,
                                tool_calls: all_calls,
                                total_usage,
                                stop_reason: StopReason::BackendError(format!(
                                    "metacognitive intervention: {intervention:?}"
                                )),
                                checkpoint: Some(cp),
                                turn_traces,
                                mcp_errors: Vec::new(),
                            };
                        }
                    }
                }
            }

            // §36.55 — context-growth guard.
            self.prune_context_if_needed(&mut messages);

            iterations += 1;
            self.save_checkpoint_snapshot(iterations, &all_calls, &messages, &session);
        }
    }

    fn check_context_overflow(&self, messages: &[Value], model: &ModelProfile) -> OverflowAction {
        let estimated_tokens = prune::estimate_message_tokens(messages);
        let limit = Self::model_context_limit(model);

        if estimated_tokens > limit {
            return OverflowAction::CompactRequired;
        }
        if estimated_tokens > Self::compaction_target(limit) {
            return OverflowAction::CompactRecommended;
        }
        OverflowAction::Ok
    }

    fn prune_context_if_needed(&self, messages: &mut Vec<Value>) {
        match self.model_profile.as_ref() {
            Some(model) => match self.check_context_overflow(messages, model) {
                OverflowAction::Ok => {}
                OverflowAction::CompactRecommended | OverflowAction::CompactRequired => {
                    compaction::compact_tool_results(messages);
                    prune::prune_if_needed(
                        messages,
                        Self::compaction_target(Self::model_context_limit(model)),
                    );
                }
            },
            None => {
                if prune::estimate_message_tokens(messages) > self.context_token_limit {
                    compaction::compact_tool_results(messages);
                }
                prune::prune_if_needed(messages, self.context_token_limit);
            }
        }
    }

    fn model_context_limit(model: &ModelProfile) -> usize {
        usize::try_from(model.context_window)
            .ok()
            .filter(|limit| *limit > 0)
            .unwrap_or(128_000)
    }

    const fn compaction_target(limit: usize) -> usize {
        limit.saturating_mul(80) / 100
    }

    async fn send_turn_with_retry(
        &self,
        messages: &[serde_json::Value],
        tools: &RenderedTools,
        session: &SessionState,
    ) -> Result<BackendResponse, LlmError> {
        for attempt in 0..self.retry_policy.max_attempts {
            match self.backend.send_turn(messages, tools, session).await {
                Ok(response) => return Ok(response),
                Err(LlmError::Provider(ref error))
                    if self.retry_policy.should_retry(error, attempt) =>
                {
                    let delay = self
                        .retry_policy
                        .delay_with_retry_after(attempt, error.retry_after_ms());
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
                Err(error) => return Err(error),
            }
        }

        Err(LlmError::RetriesExhausted)
    }
}

impl ToolLoop {
    fn save_checkpoint_snapshot(
        &self,
        iterations: usize,
        all_calls: &[ToolCall],
        messages: &[serde_json::Value],
        session: &SessionState,
    ) {
        let Some(path) = self.checkpoint_path.as_deref() else {
            return;
        };

        let cp = Checkpoint::new(iterations, all_calls.to_vec(), messages.to_vec())
            .with_session(session.clone());
        if let Err(err) = cp.save(path) {
            tracing::warn!(path = %path.display(), error = %err, "failed to persist tool loop checkpoint");
        }
    }

    fn clear_checkpoint_file(&self) {
        let Some(path) = self.checkpoint_path.as_deref() else {
            return;
        };

        if let Err(err) = remove_checkpoint_file(path) {
            tracing::warn!(path = %path.display(), error = %err, "failed to clear tool loop checkpoint");
        }
    }
}

fn remove_checkpoint_file(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn merge_session_state(current: &mut SessionState, next: SessionState) {
    if next.session_id.is_some() {
        current.session_id = next.session_id;
    }
    if next.thread_id.is_some() {
        current.thread_id = next.thread_id;
    }
    if next.conversation_id.is_some() {
        current.conversation_id = next.conversation_id;
    }
}

impl std::fmt::Debug for ToolLoop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolLoop")
            .field("translator", &"Arc<dyn Translator>")
            .field("dispatcher", &format_args!("{:?}", self.dispatcher))
            .field("backend", &"Arc<dyn LlmBackend>")
            .field("max_iterations", &self.max_iterations)
            .field("context_token_limit", &self.context_token_limit)
            .field(
                "model_profile",
                &self
                    .model_profile
                    .as_ref()
                    .map(|model| (&model.provider, &model.slug, model.context_window)),
            )
            .field("retry_policy", &self.retry_policy)
            .field("monitor", &self.monitor.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatcher::{HandlerResolver, ToolDispatcher};
    use crate::translate::{
        BackendResponse, RenderedResults, RenderedTools, Translator, TranslatorError,
    };
    use roko_core::tool::{
        AtomicCancel, CancelToken, ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef,
        ToolFormat, ToolHandler, ToolPermission, ToolResult, VecToolRegistry,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    // ─── Mock translator ─────────────────────────────────────────────

    /// Simple translator for tests: parses `tool_calls` array from JSON,
    /// renders results as JSON messages with `tool_call_id`.
    struct MockTranslator;

    impl Translator for MockTranslator {
        fn format(&self) -> ToolFormat {
            ToolFormat::OpenAiJson
        }

        fn render_tools(&self, _tools: &[ToolDef]) -> RenderedTools {
            RenderedTools::JsonArray(serde_json::json!([]))
        }

        fn parse_calls(
            &self,
            response: &BackendResponse,
        ) -> Result<Vec<ToolCall>, TranslatorError> {
            let BackendResponse::Json(ref v) = *response else {
                return Ok(vec![]);
            };
            let Some(arr) = v.get("tool_calls").and_then(|tc| tc.as_array()) else {
                return Ok(vec![]);
            };
            let mut calls = Vec::new();
            for c in arr {
                let id = c
                    .get("id")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = c
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let args = c
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                calls.push(ToolCall::new(id, name, args));
            }
            Ok(calls)
        }

        fn render_results(&self, results: &[(ToolCall, ToolResult)]) -> RenderedResults {
            let msgs: Vec<serde_json::Value> = results
                .iter()
                .map(|(call, res)| {
                    let content = match res {
                        ToolResult::Ok { content, .. } => content.clone(),
                        ToolResult::Err(e) => format!("error: {e}"),
                    };
                    serde_json::json!({
                        "role": "tool",
                        "tool_call_id": call.id,
                        "content": content,
                    })
                })
                .collect();
            RenderedResults::JsonMessages(serde_json::json!(msgs))
        }

        fn render_assistant_message(
            &self,
            response: &BackendResponse,
        ) -> Option<serde_json::Value> {
            let BackendResponse::Json(ref v) = *response else {
                return None;
            };
            v.get("assistant_message").cloned()
        }
    }

    // ─── Mock handler ────────────────────────────────────────────────

    struct EchoHandler;

    #[async_trait]
    impl ToolHandler for EchoHandler {
        fn name(&self) -> &str {
            "echo"
        }
        async fn execute(&self, call: ToolCall, _ctx: &ToolContext) -> ToolResult {
            ToolResult::text(call.arguments.to_string())
        }
    }

    // ─── Mock backends ───────────────────────────────────────────────

    /// Always returns a response with no tool calls.
    struct FinalAnswerBackend {
        text: String,
    }

    #[async_trait]
    impl LlmBackend for FinalAnswerBackend {
        async fn send_turn(
            &self,
            _messages: &[serde_json::Value],
            _tools: &RenderedTools,
            _session: &SessionState,
        ) -> Result<BackendResponse, LlmError> {
            Ok(BackendResponse::Json(
                serde_json::json!({"message": {"content": self.text}}),
            ))
        }
    }

    /// Always returns a response with one tool call (infinite loop).
    struct AlwaysToolCallBackend;

    #[async_trait]
    impl LlmBackend for AlwaysToolCallBackend {
        async fn send_turn(
            &self,
            _messages: &[serde_json::Value],
            _tools: &RenderedTools,
            _session: &SessionState,
        ) -> Result<BackendResponse, LlmError> {
            Ok(BackendResponse::Json(serde_json::json!({
                "tool_calls": [{"id": "c1", "name": "echo", "arguments": {}}]
            })))
        }
    }

    /// First call: tool call.  Subsequent calls: final answer.
    struct TwoStepBackend {
        call_count: AtomicUsize,
    }

    impl TwoStepBackend {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl LlmBackend for TwoStepBackend {
        async fn send_turn(
            &self,
            _messages: &[serde_json::Value],
            _tools: &RenderedTools,
            _session: &SessionState,
        ) -> Result<BackendResponse, LlmError> {
            let n = self.call_count.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok(BackendResponse::Json(serde_json::json!({
                    "tool_calls": [{"id": "c1", "name": "echo", "arguments": {"x": 1}}]
                })))
            } else {
                Ok(BackendResponse::Json(
                    serde_json::json!({"message": {"content": "final answer"}}),
                ))
            }
        }
    }

    /// Always returns an error.
    struct ErrorBackend;

    #[async_trait]
    impl LlmBackend for ErrorBackend {
        async fn send_turn(
            &self,
            _messages: &[serde_json::Value],
            _tools: &RenderedTools,
            _session: &SessionState,
        ) -> Result<BackendResponse, LlmError> {
            Err(LlmError::Backend("server error".into()))
        }
    }

    /// First call: two parallel tool calls.  Second call: final answer.
    struct ParallelCallsBackend {
        call_count: AtomicUsize,
    }

    impl ParallelCallsBackend {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl LlmBackend for ParallelCallsBackend {
        async fn send_turn(
            &self,
            _messages: &[serde_json::Value],
            _tools: &RenderedTools,
            _session: &SessionState,
        ) -> Result<BackendResponse, LlmError> {
            let n = self.call_count.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok(BackendResponse::Json(serde_json::json!({
                    "tool_calls": [
                        {"id": "p1", "name": "echo", "arguments": {"a": 1}},
                        {"id": "p2", "name": "echo", "arguments": {"b": 2}},
                    ]
                })))
            } else {
                Ok(BackendResponse::Json(
                    serde_json::json!({"message": {"content": "done"}}),
                ))
            }
        }
    }

    /// Captures messages on each call.  First: tool call.  Second: final answer.
    struct CapturingBackend {
        call_count: AtomicUsize,
        captured: parking_lot::Mutex<Vec<Vec<serde_json::Value>>>,
    }

    impl CapturingBackend {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
                captured: parking_lot::Mutex::new(Vec::new()),
            }
        }
    }

    struct RetryingBackend {
        attempts: AtomicUsize,
        failures_before_success: usize,
        error: ProviderError,
    }

    impl RetryingBackend {
        fn new(failures_before_success: usize, error: ProviderError) -> Self {
            Self {
                attempts: AtomicUsize::new(0),
                failures_before_success,
                error,
            }
        }

        fn attempts(&self) -> usize {
            self.attempts.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl LlmBackend for RetryingBackend {
        async fn send_turn(
            &self,
            _messages: &[serde_json::Value],
            _tools: &RenderedTools,
            _session: &SessionState,
        ) -> Result<BackendResponse, LlmError> {
            let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
            if attempt < self.failures_before_success {
                Err(LlmError::Provider(self.error.clone()))
            } else {
                Ok(BackendResponse::Json(
                    serde_json::json!({"message": {"content": "final after retry"}}),
                ))
            }
        }
    }

    #[async_trait]
    impl LlmBackend for CapturingBackend {
        async fn send_turn(
            &self,
            messages: &[serde_json::Value],
            _tools: &RenderedTools,
            _session: &SessionState,
        ) -> Result<BackendResponse, LlmError> {
            self.captured.lock().push(messages.to_vec());
            let n = self.call_count.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok(BackendResponse::Json(serde_json::json!({
                    "tool_calls": [{"id": "call-42", "name": "echo", "arguments": {"key": "val"}}]
                })))
            } else {
                Ok(BackendResponse::Json(
                    serde_json::json!({"message": {"content": "final"}}),
                ))
            }
        }
    }

    /// Captures messages on each call.  First call: three tool calls with large
    /// arguments (enough to overflow a 1 000-token context).  Second call: final
    /// answer.  Used by [`context_overflow_detection_prunes_before_next_request`].
    struct OverflowCapturingBackend {
        call_count: AtomicUsize,
        captured: parking_lot::Mutex<Vec<Vec<serde_json::Value>>>,
    }

    impl OverflowCapturingBackend {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
                captured: parking_lot::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl LlmBackend for OverflowCapturingBackend {
        async fn send_turn(
            &self,
            messages: &[serde_json::Value],
            _tools: &RenderedTools,
            _session: &SessionState,
        ) -> Result<BackendResponse, LlmError> {
            self.captured.lock().push(messages.to_vec());
            let n = self.call_count.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                // Return 3 tool calls with large arguments so that the
                // accumulated messages exceed a 1 000-token context window.
                // Each tool result will echo back ~700 chars of padding,
                // producing ~700 * 3 / 4 ≈ 525 tokens of tool output alone.
                let pad = "x".repeat(700);
                Ok(BackendResponse::Json(serde_json::json!({
                    "assistant_message": {
                        "role": "assistant",
                        "tool_calls": [
                            {"id": "ov-1", "name": "echo"},
                            {"id": "ov-2", "name": "echo"},
                            {"id": "ov-3", "name": "echo"},
                        ]
                    },
                    "tool_calls": [
                        {"id": "ov-1", "name": "echo", "arguments": {"pad": pad}},
                        {"id": "ov-2", "name": "echo", "arguments": {"pad": pad}},
                        {"id": "ov-3", "name": "echo", "arguments": {"pad": pad}},
                    ]
                })))
            } else {
                Ok(BackendResponse::Json(
                    serde_json::json!({"message": {"content": "final after pruning"}}),
                ))
            }
        }
    }

    struct SessionTrackingBackend {
        call_count: AtomicUsize,
        extracted_sessions: AtomicUsize,
    }

    impl SessionTrackingBackend {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
                extracted_sessions: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl LlmBackend for SessionTrackingBackend {
        async fn send_turn(
            &self,
            _messages: &[serde_json::Value],
            _tools: &RenderedTools,
            _session: &SessionState,
        ) -> Result<BackendResponse, LlmError> {
            let turn = self.call_count.fetch_add(1, Ordering::SeqCst);
            if turn == 0 {
                Ok(BackendResponse::Json(serde_json::json!({
                    "id": "chatcmpl-turn-1",
                    "tool_calls": [{"id": "c1", "name": "echo", "arguments": {"x": 1}}]
                })))
            } else {
                Ok(BackendResponse::Json(serde_json::json!({
                    "id": "chatcmpl-turn-2",
                    "message": {"content": "final answer"}
                })))
            }
        }

        fn extract_session(&self, _response: &BackendResponse) -> SessionState {
            self.extracted_sessions.fetch_add(1, Ordering::SeqCst);
            SessionState {
                conversation_id: Some("conversation-1".to_string()),
                ..Default::default()
            }
        }
    }

    /// Captures each request and emits three thinking tool-call turns before
    /// returning a final answer.
    struct ReasoningCaptureBackend {
        call_count: AtomicUsize,
        captured: parking_lot::Mutex<Vec<Vec<serde_json::Value>>>,
    }

    impl ReasoningCaptureBackend {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
                captured: parking_lot::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl LlmBackend for ReasoningCaptureBackend {
        async fn send_turn(
            &self,
            messages: &[serde_json::Value],
            _tools: &RenderedTools,
            _session: &SessionState,
        ) -> Result<BackendResponse, LlmError> {
            self.captured.lock().push(messages.to_vec());

            let turn = self.call_count.fetch_add(1, Ordering::SeqCst) + 1;
            let response = match turn {
                1..=3 => serde_json::json!({
                    "assistant_message": {
                        "role": "assistant",
                        "content": "",
                        "reasoning_content": format!("reasoning turn {turn}"),
                        "tool_calls": [{
                            "id": format!("call-{turn}"),
                            "name": "echo",
                            "arguments": { "turn": turn }
                        }]
                    },
                    "tool_calls": [{
                        "id": format!("call-{turn}"),
                        "name": "echo",
                        "arguments": { "turn": turn }
                    }]
                }),
                _ => serde_json::json!({
                    "message": { "content": "final" }
                }),
            };

            Ok(BackendResponse::Json(response))
        }
    }

    // ─── Helpers ─────────────────────────────────────────────────────

    fn test_tools() -> Vec<ToolDef> {
        vec![
            ToolDef::new(
                "echo",
                "echo args",
                ToolCategory::Meta,
                ToolPermission::read_only(),
            )
            .with_concurrency(ToolConcurrency::Parallel),
        ]
    }

    fn make_tool_loop(backend: Arc<dyn LlmBackend>, max_iterations: usize) -> ToolLoop {
        let registry: Arc<dyn roko_core::tool::ToolRegistry> =
            Arc::new(VecToolRegistry::from_tools(test_tools()));
        let resolver: Arc<dyn HandlerResolver> =
            Arc::new(|name: &str| -> Option<Arc<dyn ToolHandler>> {
                if name == "echo" {
                    Some(Arc::new(EchoHandler) as Arc<dyn ToolHandler>)
                } else {
                    None
                }
            });
        let dispatcher = Arc::new(ToolDispatcher::new(registry, resolver));
        let translator: Arc<dyn Translator> = Arc::new(MockTranslator);
        ToolLoop::new(translator, dispatcher, backend).with_max_iterations(max_iterations)
    }

    struct SessionContinuityBackend {
        call_count: AtomicUsize,
        seen_sessions: parking_lot::Mutex<Vec<SessionState>>,
    }

    impl SessionContinuityBackend {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
                seen_sessions: parking_lot::Mutex::new(Vec::new()),
            }
        }
    }

    struct CheckpointPersistenceBackend;

    #[async_trait]
    impl LlmBackend for CheckpointPersistenceBackend {
        async fn send_turn(
            &self,
            messages: &[serde_json::Value],
            _tools: &RenderedTools,
            _session: &SessionState,
        ) -> Result<BackendResponse, LlmError> {
            let resumed = messages
                .iter()
                .any(|message| message["role"] == "tool" && message.get("tool_call_id").is_some());

            if resumed {
                Ok(BackendResponse::Json(
                    serde_json::json!({"message": {"content": "resumed final answer"}}),
                ))
            } else {
                Ok(BackendResponse::Json(serde_json::json!({
                    "tool_calls": [{
                        "id": "persist-1",
                        "name": "echo",
                        "arguments": {"step": 1}
                    }]
                })))
            }
        }
    }

    #[async_trait]
    impl LlmBackend for SessionContinuityBackend {
        async fn send_turn(
            &self,
            _messages: &[serde_json::Value],
            _tools: &RenderedTools,
            session: &SessionState,
        ) -> Result<BackendResponse, LlmError> {
            self.seen_sessions.lock().push(session.clone());
            let turn = self.call_count.fetch_add(1, Ordering::SeqCst);
            if turn == 0 {
                Ok(BackendResponse::Json(serde_json::json!({
                    "tool_calls": [{"id": "c1", "name": "echo", "arguments": {"x": 1}}]
                })))
            } else {
                Ok(BackendResponse::Json(
                    serde_json::json!({"message": {"content": "final answer"}}),
                ))
            }
        }

        fn extract_session(&self, response: &BackendResponse) -> SessionState {
            let turn = response
                .extract_text()
                .is_empty()
                .then_some("session-1")
                .map(str::to_string);
            SessionState {
                session_id: turn,
                ..Default::default()
            }
        }
    }

    fn test_retry_policy() -> RetryPolicy {
        RetryPolicy {
            max_attempts: 3,
            base_delay_ms: 0,
            max_delay_ms: 0,
            ..RetryPolicy::default()
        }
    }

    fn test_model_profile(context_window: u64) -> ModelProfile {
        ModelProfile {
            provider: "test-provider".to_string(),
            slug: "test-model".to_string(),
            context_window,
            max_output: None,
            supports_tools: true,
            supports_thinking: false,
            supports_vision: false,
            supports_web_search: false,
            supports_mcp_tools: false,
            supports_partial: false,
            supports_grounding: false,
            supports_code_execution: false,
            supports_caching: false,
            provider_routing: None,
            tool_format: "openai_json".to_string(),
            cost_input_per_m: None,
            cost_output_per_m: None,
            cost_input_per_m_high: None,
            cost_output_per_m_high: None,
            cost_cache_read_per_m: None,
            cost_cache_write_per_m: None,
            thinking_level: None,
            max_tools: None,
            tokenizer_ratio: None,
            ..Default::default()
        }
    }

    fn messages_for_estimated_tokens(target_tokens: usize) -> Vec<Value> {
        let mut messages = vec![
            serde_json::json!({"role": "system", "content": "sys"}),
            serde_json::json!({"role": "user", "content": "usr"}),
        ];
        while prune::estimate_message_tokens(&messages) < target_tokens {
            messages.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": format!("c{}", messages.len()),
                "content": "x".repeat(512),
            }));
        }
        messages
    }

    // ─── Tests ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn zero_tool_calls_returns_immediately() {
        let backend = Arc::new(FinalAnswerBackend {
            text: "done".into(),
        });
        let tl = make_tool_loop(backend, 25);
        let ctx = ToolContext::testing("/tmp");

        let out = tl.run("system", "user", &test_tools(), &ctx).await;

        assert_eq!(out.stop_reason, StopReason::Stop);
        assert_eq!(out.iterations, 0);
        assert!(out.tool_calls.is_empty());
        assert_eq!(out.final_text, "done");
        assert!(out.checkpoint.is_none());
    }

    #[tokio::test]
    async fn single_tool_call_runs_to_completion() {
        let backend = Arc::new(TwoStepBackend::new());
        let tl = make_tool_loop(backend, 25);
        let ctx = ToolContext::testing("/tmp");

        let out = tl.run("system", "user", &test_tools(), &ctx).await;

        assert_eq!(out.stop_reason, StopReason::Stop);
        assert_eq!(out.iterations, 1);
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].name, "echo");
        assert_eq!(out.final_text, "final answer");
        assert!(out.checkpoint.is_none());
    }

    #[tokio::test]
    async fn run_messages_streaming_preserves_prebuilt_history() {
        let backend = Arc::new(CapturingBackend::new());
        let tl = make_tool_loop(backend.clone(), 25);
        let ctx = ToolContext::testing("/tmp");
        let messages = vec![
            serde_json::json!({"role": "system", "content": "system"}),
            serde_json::json!({"role": "assistant", "content": "earlier answer"}),
            serde_json::json!({"role": "user", "content": "new prompt"}),
        ];
        let (stream_tx, _stream_rx) = mpsc::channel(roko_core::defaults::DEFAULT_CHANNEL_BUFFER);

        let out = tl
            .run_messages_streaming(messages.clone(), &test_tools(), &ctx, stream_tx)
            .await;

        assert_eq!(out.stop_reason, StopReason::Stop);
        assert_eq!(out.iterations, 1);
        let captured = backend.captured.lock();
        assert_eq!(captured[0], messages);
        assert!(
            captured[1]
                .iter()
                .any(|message| message["role"] == "tool" && message["tool_call_id"] == "call-42")
        );
    }

    #[tokio::test]
    async fn max_iterations_returns_max_iterations() {
        let backend = Arc::new(AlwaysToolCallBackend);
        let tl = make_tool_loop(backend, 25);
        let ctx = ToolContext::testing("/tmp");

        let out = tl.run("system", "user", &test_tools(), &ctx).await;

        assert_eq!(out.stop_reason, StopReason::MaxIterations);
        assert_eq!(out.iterations, 25);
        assert_eq!(out.tool_calls.len(), 25);
        assert!(out.final_text.is_empty());
        // Checkpoint should be present for resumability.
        let cp = out.checkpoint.expect("checkpoint should be present");
        assert_eq!(cp.iterations, 25);
        assert_eq!(cp.tool_calls.len(), 25);
    }

    #[tokio::test]
    async fn cancellation_halts_loop() {
        let cancel = Arc::new(AtomicCancel::new());

        // Handler that trips the cancel token when invoked.
        struct CancellingHandler {
            cancel: Arc<AtomicCancel>,
        }
        #[async_trait]
        impl ToolHandler for CancellingHandler {
            fn name(&self) -> &str {
                "echo"
            }
            async fn execute(&self, call: ToolCall, _ctx: &ToolContext) -> ToolResult {
                self.cancel.cancel();
                ToolResult::text(call.arguments.to_string())
            }
        }

        let backend: Arc<dyn LlmBackend> = Arc::new(AlwaysToolCallBackend);
        let registry: Arc<dyn roko_core::tool::ToolRegistry> =
            Arc::new(VecToolRegistry::from_tools(test_tools()));
        let cancel_for_handler = cancel.clone();
        let resolver: Arc<dyn HandlerResolver> =
            Arc::new(move |name: &str| -> Option<Arc<dyn ToolHandler>> {
                if name == "echo" {
                    Some(Arc::new(CancellingHandler {
                        cancel: cancel_for_handler.clone(),
                    }) as Arc<dyn ToolHandler>)
                } else {
                    None
                }
            });
        let dispatcher = Arc::new(ToolDispatcher::new(registry, resolver));
        let translator: Arc<dyn Translator> = Arc::new(MockTranslator);
        let tl = ToolLoop::new(translator, dispatcher, backend).with_max_iterations(100);

        let ctx = ToolContext::testing("/tmp").with_cancel_token(cancel as Arc<dyn CancelToken>);

        let out = tl.run("system", "user", &test_tools(), &ctx).await;

        assert_eq!(out.stop_reason, StopReason::Cancelled);
        // One iteration ran (dispatched the call, handler tripped cancel),
        // then the next iteration's cancel check fired.
        assert_eq!(out.iterations, 1);
        assert!(out.checkpoint.is_some());
    }

    #[tokio::test]
    async fn backend_error_returns_backend_error() {
        let backend = Arc::new(ErrorBackend);
        let tl = make_tool_loop(backend, 25);
        let ctx = ToolContext::testing("/tmp");

        let out = tl.run("system", "user", &test_tools(), &ctx).await;

        match &out.stop_reason {
            StopReason::BackendError(msg) => {
                assert!(msg.contains("server error"), "msg={msg}");
            }
            other => panic!("expected BackendError, got {other:?}"),
        }
        assert_eq!(out.iterations, 0);
        assert!(out.checkpoint.is_some());
    }

    #[tokio::test]
    async fn retry_with_jitter_retries_rate_limits_until_success() {
        let backend = Arc::new(RetryingBackend::new(
            2,
            ProviderError::RateLimit {
                retry_after_ms: None,
            },
        ));
        let tl = make_tool_loop(backend.clone(), 25).with_retry_policy(test_retry_policy());
        let ctx = ToolContext::testing("/tmp");

        let out = tl.run("system", "user", &test_tools(), &ctx).await;

        assert_eq!(out.stop_reason, StopReason::Stop);
        assert_eq!(out.final_text, "final after retry");
        assert_eq!(backend.attempts(), 3);
    }

    #[tokio::test]
    async fn retry_with_jitter_does_not_retry_auth_failures() {
        let backend = Arc::new(RetryingBackend::new(usize::MAX, ProviderError::AuthFailure));
        let tl = make_tool_loop(backend.clone(), 25).with_retry_policy(test_retry_policy());
        let ctx = ToolContext::testing("/tmp");

        let out = tl.run("system", "user", &test_tools(), &ctx).await;

        match &out.stop_reason {
            StopReason::BackendError(msg) => {
                assert!(msg.contains("authentication failed"), "msg={msg}");
            }
            other => panic!("expected BackendError, got {other:?}"),
        }
        assert_eq!(backend.attempts(), 1);
    }

    #[tokio::test]
    async fn parallel_tool_calls_dispatched_in_one_batch() {
        let backend = Arc::new(ParallelCallsBackend::new());
        let tl = make_tool_loop(backend, 25);
        let ctx = ToolContext::testing("/tmp");

        let out = tl.run("system", "user", &test_tools(), &ctx).await;

        assert_eq!(out.stop_reason, StopReason::Stop);
        assert_eq!(out.iterations, 1);
        assert_eq!(out.tool_calls.len(), 2);
        let ids: Vec<&str> = out.tool_calls.iter().map(|c| c.id.as_str()).collect();
        assert!(ids.contains(&"p1"), "missing tool call p1");
        assert!(ids.contains(&"p2"), "missing tool call p2");
    }

    #[tokio::test]
    async fn context_prune_drops_oldest_results_after_threshold() {
        let mut msgs = vec![
            serde_json::json!({"role": "system", "content": "sys"}),
            serde_json::json!({"role": "user", "content": "usr"}),
        ];
        for i in 0..20 {
            msgs.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": format!("c{i}"),
                "content": "x".repeat(500),
            }));
        }
        let before_len = msgs.len();

        prune::prune_if_needed(&mut msgs, 100);

        assert!(msgs.len() < before_len, "messages should be pruned");
        assert!(msgs.len() >= 5, "should keep at least head + tail");
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[1]["role"], "user");
    }

    #[test]
    fn context_overflow_detection_classifies_utilization_thresholds() {
        let backend = Arc::new(FinalAnswerBackend {
            text: "unused".into(),
        });
        let tl = make_tool_loop(backend, 25).with_model_profile(test_model_profile(2_000));

        let ok_messages = messages_for_estimated_tokens(1_400);
        assert_eq!(
            tl.check_context_overflow(&ok_messages, tl.model_profile.as_ref().expect("model")),
            OverflowAction::Ok
        );

        let recommended_messages = messages_for_estimated_tokens(1_700);
        assert_eq!(
            tl.check_context_overflow(
                &recommended_messages,
                tl.model_profile.as_ref().expect("model"),
            ),
            OverflowAction::CompactRecommended
        );

        let required_messages = messages_for_estimated_tokens(2_100);
        assert_eq!(
            tl.check_context_overflow(
                &required_messages,
                tl.model_profile.as_ref().expect("model"),
            ),
            OverflowAction::CompactRequired
        );
    }

    #[tokio::test]
    async fn context_overflow_detection_prunes_before_next_request() {
        let backend = Arc::new(OverflowCapturingBackend::new());
        // Use a context window small enough that the 3 x 700-char tool
        // results (~619 estimated tokens) exceed the 80% compaction
        // target (770 * 0.8 = 616), triggering a prune pass that removes
        // exactly one droppable message before the second backend call.
        let tl = make_tool_loop(backend.clone(), 25).with_model_profile(test_model_profile(770));
        let ctx = ToolContext::testing("/tmp");

        let out = tl.run("system", "user", &test_tools(), &ctx).await;

        assert_eq!(out.stop_reason, StopReason::Stop);
        assert_eq!(out.final_text, "final after pruning");

        let captured = backend.captured.lock();
        assert_eq!(captured.len(), 2, "backend should be called twice");
        let second_call_messages = &captured[1];
        assert_eq!(
            second_call_messages.len(),
            5,
            "one message should be pruned from the 6-message history"
        );
        assert_eq!(second_call_messages[0]["role"], "system");
        assert_eq!(second_call_messages[1]["role"], "user");
        assert!(
            prune::estimate_message_tokens(second_call_messages)
                <= ToolLoop::compaction_target(770),
            "expected second request to be compacted below the 80% target",
        );
    }

    #[test]
    fn tool_result_compaction_runs_before_pruning() {
        let backend = Arc::new(FinalAnswerBackend {
            text: "unused".into(),
        });
        let mut messages = vec![
            serde_json::json!({"role": "system", "content": "sys"}),
            serde_json::json!({"role": "user", "content": "usr"}),
            serde_json::json!({"role": "assistant", "tool_calls": [{"id": "old"}]}),
            serde_json::json!({
                "role": "tool",
                "tool_call_id": "old",
                "content": "a".repeat(900),
            }),
            serde_json::json!({"role": "assistant", "tool_calls": [{"id": "recent-1"}]}),
            serde_json::json!({
                "role": "tool",
                "tool_call_id": "recent-1",
                "content": "b".repeat(900),
            }),
            serde_json::json!({"role": "assistant", "tool_calls": [{"id": "recent-2"}]}),
            serde_json::json!({
                "role": "tool",
                "tool_call_id": "recent-2",
                "content": "c".repeat(900),
            }),
        ];
        let mut compacted = messages.clone();
        compaction::compact_tool_results(&mut compacted);

        let before_tokens = prune::estimate_message_tokens(&messages);
        let compacted_tokens = prune::estimate_message_tokens(&compacted);
        assert!(
            compacted_tokens < before_tokens,
            "compaction should shrink old results"
        );

        let target = compacted_tokens + ((before_tokens - compacted_tokens) / 2).max(1);
        let limit = (target * 100).div_ceil(80);
        let tl = make_tool_loop(backend, 25).with_model_profile(test_model_profile(limit as u64));

        tl.prune_context_if_needed(&mut messages);

        assert_eq!(
            messages.len(),
            8,
            "compaction should avoid dropping messages"
        );
        assert_eq!(messages[3]["tool_call_id"], "old");
        assert!(
            messages[3]["content"]
                .as_str()
                .expect("compacted old content")
                .contains("[truncated, 900 chars total]"),
        );
        assert_eq!(messages[5]["content"], "b".repeat(900));
        assert_eq!(messages[7]["content"], "c".repeat(900));
    }

    #[tokio::test]
    async fn tool_call_ids_flow_through_to_result_messages() {
        let capturing = Arc::new(CapturingBackend::new());
        let backend = capturing.clone() as Arc<dyn LlmBackend>;
        let tl = make_tool_loop(backend, 25);
        let ctx = ToolContext::testing("/tmp");

        let out = tl.run("system", "user", &test_tools(), &ctx).await;

        assert_eq!(out.stop_reason, StopReason::Stop);
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].id, "call-42");

        // The second backend call should have messages containing the
        // tool result with the correct tool_call_id.
        let captured = capturing.captured.lock();
        assert_eq!(captured.len(), 2, "backend should be called twice");
        let second_call_msgs = &captured[1];
        let tool_msg = second_call_msgs
            .iter()
            .find(|m| m.get("tool_call_id").is_some())
            .expect("should have a tool-result message");
        assert_eq!(tool_msg["tool_call_id"], "call-42");
    }

    #[tokio::test]
    async fn session_extraction_runs_after_each_turn() {
        let backend = Arc::new(SessionTrackingBackend::new());
        let tl = make_tool_loop(backend.clone(), 25);
        let ctx = ToolContext::testing("/tmp");

        let out = tl.run("system", "user", &test_tools(), &ctx).await;

        assert_eq!(out.stop_reason, StopReason::Stop);
        assert_eq!(backend.extracted_sessions.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn session_continuity_passes_previous_state_into_next_turn() {
        let backend = Arc::new(SessionContinuityBackend::new());
        let tl = make_tool_loop(backend.clone(), 25);
        let ctx = ToolContext::testing("/tmp");

        let out = tl.run("system", "user", &test_tools(), &ctx).await;

        assert_eq!(out.stop_reason, StopReason::Stop);

        let seen_sessions = backend.seen_sessions.lock();
        assert_eq!(seen_sessions.len(), 2);
        assert_eq!(seen_sessions[0].session_id, None);
        assert_eq!(seen_sessions[1].session_id.as_deref(), Some("session-1"));
    }

    #[tokio::test]
    async fn reasoning_preservation_across_loop_turns() {
        let capturing = Arc::new(ReasoningCaptureBackend::new());
        let backend = capturing.clone() as Arc<dyn LlmBackend>;
        let tl = make_tool_loop(backend, 25);
        let ctx = ToolContext::testing("/tmp");

        let out = tl.run("system", "user", &test_tools(), &ctx).await;

        assert_eq!(out.stop_reason, StopReason::Stop);
        assert_eq!(out.iterations, 3);
        assert_eq!(out.final_text, "final");

        let captured = capturing.captured.lock();
        assert_eq!(captured.len(), 4, "backend should be called four times");

        let fourth_turn_msgs = &captured[3];
        let reasoning_values: Vec<&str> = fourth_turn_msgs
            .iter()
            .filter(|message| message["role"] == "assistant")
            .filter_map(|message| message["reasoning_content"].as_str())
            .collect();
        assert_eq!(
            reasoning_values,
            vec!["reasoning turn 1", "reasoning turn 2", "reasoning turn 3"]
        );
    }

    #[tokio::test]
    async fn resume_continues_from_checkpoint() {
        // Run a loop that hits max iterations at 3.
        let backend = Arc::new(AlwaysToolCallBackend);
        let tl = make_tool_loop(backend, 3);
        let ctx = ToolContext::testing("/tmp");

        let out = tl.run("system", "user", &test_tools(), &ctx).await;
        assert_eq!(out.stop_reason, StopReason::MaxIterations);
        assert_eq!(out.iterations, 3);
        let cp = out.checkpoint.expect("checkpoint present");

        // Resume with a higher limit: backend still always emits tool
        // calls, so it should hit the new cap.  Start from iteration 3,
        // run 2 more (cap at 5).
        let backend2 = Arc::new(AlwaysToolCallBackend);
        let tl2 = make_tool_loop(backend2, 5);
        let out2 = tl2.resume(cp, &test_tools(), &ctx).await;
        assert_eq!(out2.stop_reason, StopReason::MaxIterations);
        assert_eq!(out2.iterations, 5);
        // 3 from the first run (in the checkpoint) + 2 new ones.
        assert_eq!(out2.tool_calls.len(), 5);
    }

    #[tokio::test]
    async fn checkpoint_persistence_survives_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let checkpoint_path = dir
            .path()
            .join(".roko")
            .join("state")
            .join("tool-loop-task-1.json");
        let ctx = ToolContext::testing(dir.path());

        let first = make_tool_loop(Arc::new(CheckpointPersistenceBackend), 1)
            .with_checkpoint_path(checkpoint_path.clone());
        let out1 = first.run("system", "user", &test_tools(), &ctx).await;

        assert_eq!(out1.stop_reason, StopReason::MaxIterations);
        assert!(checkpoint_path.exists(), "checkpoint should be persisted");

        let persisted = Checkpoint::load(&checkpoint_path).expect("load persisted checkpoint");
        assert_eq!(persisted.iterations, 1);
        assert_eq!(persisted.tool_calls.len(), 1);
        assert_eq!(persisted.tool_calls[0].id, "persist-1");

        let resumed = make_tool_loop(Arc::new(CheckpointPersistenceBackend), 5)
            .with_checkpoint_path(checkpoint_path.clone());
        let out2 = resumed.run("ignored", "ignored", &test_tools(), &ctx).await;

        assert_eq!(out2.stop_reason, StopReason::Stop);
        assert_eq!(out2.iterations, 1);
        assert_eq!(out2.tool_calls.len(), 1);
        assert_eq!(out2.final_text, "resumed final answer");
        assert!(
            !checkpoint_path.exists(),
            "successful completion should clear the persisted checkpoint"
        );
    }

    #[tokio::test]
    async fn debug_impl_does_not_panic() {
        let backend = Arc::new(FinalAnswerBackend { text: "x".into() });
        let tl = make_tool_loop(backend, 25);
        let s = format!("{tl:?}");
        assert!(s.contains("ToolLoop"));
        assert!(s.contains("max_iterations"));
    }

    // ─── StreamEvent / collect_stream_to_response tests ──────────────

    #[tokio::test]
    async fn collect_stream_text_deltas_concatenate() {
        use futures::stream;
        let events = vec![
            Ok(StreamEvent::now(StreamEventKind::TextDelta("Hello".to_string()))),
            Ok(StreamEvent::now(StreamEventKind::TextDelta(", world!".to_string()))),
            Ok(StreamEvent::now(StreamEventKind::Done {
                finish_reason: "stop".to_string(),
            })),
        ];
        let stream = Box::pin(stream::iter(events));
        let start = std::time::Instant::now();
        let response = collect_stream_to_response(stream, start).await.unwrap();
        let text = response.extract_text();
        assert_eq!(text, "Hello, world!");
    }

    #[tokio::test]
    async fn collect_stream_tool_calls_assemble() {
        use futures::stream;
        let events = vec![
            Ok(StreamEvent::now(StreamEventKind::ToolCallStart {
                id: "call-1".to_string(),
                name: "read_file".to_string(),
            })),
            Ok(StreamEvent::now(StreamEventKind::ToolCallDelta {
                id: "call-1".to_string(),
                json_fragment: r#"{"path":"#.to_string(),
            })),
            Ok(StreamEvent::now(StreamEventKind::ToolCallDelta {
                id: "call-1".to_string(),
                json_fragment: r#""foo.txt"}"#.to_string(),
            })),
            Ok(StreamEvent::now(StreamEventKind::ToolCallEnd {
                id: "call-1".to_string(),
                name: "read_file".to_string(),
                args: serde_json::json!({"path": "foo.txt"}),
            })),
            Ok(StreamEvent::now(StreamEventKind::Done {
                finish_reason: "tool_calls".to_string(),
            })),
        ];
        let stream = Box::pin(stream::iter(events));
        let start = std::time::Instant::now();
        let response = collect_stream_to_response(stream, start).await.unwrap();

        // The ToolCallEnd provides the final assembled args; the response
        // should contain one tool call with the right arguments.
        let BackendResponse::Json(ref json) = response else {
            panic!("expected Json response");
        };
        let tool_calls = json
            .pointer("/choices/0/message/tool_calls")
            .and_then(|tc| tc.as_array())
            .expect("expected tool_calls array");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(
            tool_calls[0].pointer("/function/name").and_then(|n| n.as_str()),
            Some("read_file")
        );
    }

    #[tokio::test]
    async fn collect_stream_usage_is_preserved() {
        use futures::stream;
        let usage = crate::usage::Usage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 10,
            ..Default::default()
        };
        let events = vec![
            Ok(StreamEvent::now(StreamEventKind::TextDelta("ok".to_string()))),
            Ok(StreamEvent::now(StreamEventKind::Usage(usage))),
            Ok(StreamEvent::now(StreamEventKind::Done {
                finish_reason: "stop".to_string(),
            })),
        ];
        let stream = Box::pin(stream::iter(events));
        let start = std::time::Instant::now();
        let response = collect_stream_to_response(stream, start).await.unwrap();
        let BackendResponse::Json(ref json) = response else {
            panic!("expected Json response");
        };
        assert_eq!(
            json.pointer("/usage/prompt_tokens").and_then(|v| v.as_u64()),
            Some(100)
        );
        assert_eq!(
            json.pointer("/usage/completion_tokens").and_then(|v| v.as_u64()),
            Some(50)
        );
    }

    #[tokio::test]
    async fn collect_stream_ttft_is_measured() {
        use futures::stream;
        let events = vec![
            Ok(StreamEvent::now(StreamEventKind::TextDelta("hi".to_string()))),
            Ok(StreamEvent::now(StreamEventKind::Done {
                finish_reason: "stop".to_string(),
            })),
        ];
        let stream = Box::pin(stream::iter(events));
        let start = std::time::Instant::now();
        let response = collect_stream_to_response(stream, start).await.unwrap();
        let BackendResponse::Json(ref json) = response else {
            panic!("expected Json response");
        };
        // TTFT should be present in metadata.
        let ttft = json.pointer("/metadata/provider_ttft_ms");
        assert!(ttft.is_some(), "expected provider_ttft_ms in metadata");
        // The TTFT value should be a non-negative number.
        let ttft_val = ttft.and_then(|v| v.as_u64()).unwrap_or(0);
        assert!(ttft_val < 5_000, "TTFT should be small in test: {ttft_val}ms");
    }

    #[tokio::test]
    async fn collect_stream_error_propagates() {
        use futures::stream;
        let events = vec![
            Ok(StreamEvent::now(StreamEventKind::TextDelta("partial".to_string()))),
            Err(LlmError::Network("connection reset".to_string())),
        ];
        let stream = Box::pin(stream::iter(events));
        let start = std::time::Instant::now();
        let result = collect_stream_to_response(stream, start).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("connection reset"));
    }

    #[tokio::test]
    async fn response_to_synthetic_stream_roundtrips() {
        use futures::StreamExt;

        let original = BackendResponse::Json(serde_json::json!({
            "choices": [{
                "message": {"role": "assistant", "content": "hello"},
                "finish_reason": "stop",
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5},
        }));
        let text_before = original.extract_text();

        let stream = response_to_synthetic_stream(original);
        let events: Vec<_> = stream.collect().await;

        // Should have 3 events: TextDelta, Usage, Done
        assert_eq!(events.len(), 3);
        assert!(matches!(
            &events[0].as_ref().unwrap().kind,
            StreamEventKind::TextDelta(text) if text == &text_before
        ));
        assert!(matches!(
            &events[1].as_ref().unwrap().kind,
            StreamEventKind::Usage(_)
        ));
        assert!(matches!(
            &events[2].as_ref().unwrap().kind,
            StreamEventKind::Done { .. }
        ));
    }

    #[test]
    fn turn_config_default_uses_roko_defaults() {
        let config = TurnConfig::default();
        assert_eq!(config.max_tokens, roko_core::defaults::DEFAULT_MAX_OUTPUT_TOKENS);
        assert!(config.temperature.is_none());
        assert_eq!(
            config.ttft_timeout.as_millis() as u64,
            roko_core::defaults::DEFAULT_TTFT_TIMEOUT_MS
        );
        assert_eq!(
            config.request_timeout.as_millis() as u64,
            roko_core::defaults::DEFAULT_REQUEST_TIMEOUT_MS
        );
        assert!(config.stop_sequences.is_empty());
    }

    #[test]
    fn stream_event_from_stream_chunk_content_delta() {
        let chunk = StreamChunk::ContentDelta("hello".to_string());
        let event: StreamEvent = chunk.into();
        assert!(matches!(event.kind, StreamEventKind::TextDelta(ref text) if text == "hello"));
    }

    #[test]
    fn stream_event_from_stream_chunk_reasoning_delta() {
        let chunk = StreamChunk::ReasoningDelta("thinking".to_string());
        let event: StreamEvent = chunk.into();
        assert!(matches!(
            event.kind,
            StreamEventKind::ReasoningDelta(ref text) if text == "thinking"
        ));
    }

    #[test]
    fn stream_event_from_stream_chunk_done() {
        let chunk = StreamChunk::Done(crate::translate::FinishReason::ToolCalls);
        let event: StreamEvent = chunk.into();
        assert!(matches!(event.kind, StreamEventKind::Done { ref finish_reason } if finish_reason.contains("ToolCalls")));
    }

    #[tokio::test]
    async fn stream_turn_default_wraps_send_turn() {
        let backend = FinalAnswerBackend {
            text: "from send_turn".to_string(),
        };
        let config = TurnConfig::default();
        let stream = backend
            .stream_turn(
                &[serde_json::json!({"role": "user", "content": "hi"})],
                &RenderedTools::JsonArray(serde_json::json!([])),
                &SessionState::default(),
                &config,
            )
            .await
            .expect("stream_turn should succeed");

        use futures::StreamExt;
        let events: Vec<_> = stream.collect::<Vec<_>>().await;
        assert!(!events.is_empty());
        // First event should contain the text
        let first = events[0].as_ref().unwrap();
        assert!(matches!(
            &first.kind,
            StreamEventKind::TextDelta(text) if text.contains("from send_turn")
        ));
    }

    #[tokio::test]
    async fn collect_stream_reasoning_delta_captured() {
        use futures::stream;
        let events = vec![
            Ok(StreamEvent::now(StreamEventKind::ReasoningDelta("step 1".to_string()))),
            Ok(StreamEvent::now(StreamEventKind::ReasoningDelta(" step 2".to_string()))),
            Ok(StreamEvent::now(StreamEventKind::TextDelta("answer".to_string()))),
            Ok(StreamEvent::now(StreamEventKind::Done {
                finish_reason: "stop".to_string(),
            })),
        ];
        let stream = Box::pin(stream::iter(events));
        let start = std::time::Instant::now();
        let response = collect_stream_to_response(stream, start).await.unwrap();
        let BackendResponse::Json(ref json) = response else {
            panic!("expected Json response");
        };
        // Reasoning should be in the message
        let reasoning = json
            .pointer("/choices/0/message/reasoning_content")
            .and_then(|v| v.as_str());
        assert_eq!(reasoning, Some("step 1 step 2"));
    }

    #[test]
    fn tool_loop_turn_config_uses_model_profile() {
        let profile = ModelProfile {
            provider: "test".to_string(),
            slug: "test-model".to_string(),
            context_window: 128_000,
            max_output: Some(8192),
            ..Default::default()
        };

        let registry: Arc<dyn roko_core::tool::ToolRegistry> =
            Arc::new(VecToolRegistry::from_tools(test_tools()));
        let resolver: Arc<dyn crate::dispatcher::HandlerResolver> =
            Arc::new(|name: &str| -> Option<Arc<dyn roko_core::tool::ToolHandler>> {
                if name == "echo" {
                    Some(Arc::new(EchoHandler) as Arc<dyn roko_core::tool::ToolHandler>)
                } else {
                    None
                }
            });
        let dispatcher = Arc::new(ToolDispatcher::new(registry, resolver));
        let translator: Arc<dyn Translator> = Arc::new(MockTranslator);
        let backend: Arc<dyn LlmBackend> = Arc::new(FinalAnswerBackend { text: "x".into() });

        let tl = ToolLoop::new(translator, dispatcher, backend)
            .with_model_profile(profile);

        let config = tl.turn_config();
        assert_eq!(config.max_tokens, 8192);
    }
}
