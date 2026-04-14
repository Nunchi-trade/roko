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
use crate::provider::ProviderError;
use crate::retry::RetryPolicy;
use crate::streaming::StreamChunk;
use crate::translate::{BackendResponse, RenderedTools, SessionState, Translator};
use crate::usage::Usage;

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
/// Sends a conversation turn (messages + tool specs) and returns the
/// backend's response.  The [`ToolLoop`] calls this once per iteration
/// and inspects the response for tool calls via the [`Translator`].
///
/// This is intentionally lower-level than [`Agent`](crate::agent::Agent):
/// it models a single request-response round, not a full agent run.
#[async_trait]
pub trait LlmBackend: Send + Sync {
    /// Send the current conversation state to the backend.
    ///
    /// `messages` is the accumulated message history (system, user,
    /// assistant, tool-result messages).  `tools` is the pre-rendered
    /// tool spec from [`Translator::render_tools`].
    async fn send_turn(
        &self,
        messages: &[serde_json::Value],
        tools: &RenderedTools,
        session: &SessionState,
    ) -> Result<BackendResponse, LlmError>;

    /// Extract provider-issued session or conversation identifiers from a turn response.
    fn extract_session(&self, response: &BackendResponse) -> SessionState {
        let _ = response;
        SessionState::default()
    }

    /// Send the current conversation state to the backend in streaming mode.
    ///
    /// Backends that do not implement streaming fall back to [`send_turn`](Self::send_turn).
    async fn send_turn_streaming(
        &self,
        messages: &[serde_json::Value],
        tools: &RenderedTools,
        session: &SessionState,
        event_tx: mpsc::UnboundedSender<StreamChunk>,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverflowAction {
    Ok,
    CompactRecommended,
    CompactRequired,
}

// ─── ToolLoop ────────────────────────────────────────────────────────

/// Multi-turn tool-calling loop (§36.f).
///
/// Drives the `prompt -> LLM -> tool_calls -> dispatch -> results -> LLM`
/// cycle until the LLM stops calling tools, the iteration cap is
/// reached, the cancel token fires, or the backend errors.
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
        }
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

    /// Run a fresh tool loop from an initial system + user prompt.
    pub async fn run(
        &self,
        system: &str,
        user: &str,
        tools: &[ToolDef],
        ctx: &ToolContext,
    ) -> ToolLoopOutput {
        if let Some(path) = self.checkpoint_path.as_deref().filter(|path| path.exists()) {
            match Checkpoint::load(path) {
                Ok(cp) => return self.resume(cp, tools, ctx).await,
                Err(err) => {
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
                    };
                }
            }
        }

        let messages = result_msg::initial_messages(system, user);
        self.run_inner(messages, 0, Vec::new(), Usage::default(), tools, ctx, None)
            .await
    }

    /// Run a fresh tool loop and forward streaming chunks as each backend turn arrives.
    pub async fn run_streaming(
        &self,
        system: &str,
        user: &str,
        tools: &[ToolDef],
        ctx: &ToolContext,
        event_tx: mpsc::UnboundedSender<StreamChunk>,
    ) -> ToolLoopOutput {
        let messages = result_msg::initial_messages(system, user);
        self.run_inner(
            messages,
            0,
            Vec::new(),
            Usage::default(),
            tools,
            ctx,
            Some(event_tx),
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
        self.run_inner(
            cp.messages,
            cp.iterations,
            cp.tool_calls,
            Usage::default(),
            tools,
            ctx,
            None,
        )
        .await
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
        event_tx: Option<mpsc::UnboundedSender<StreamChunk>>,
    ) -> ToolLoopOutput {
        let rendered_tools = self.translator.render_tools(tools);
        let mut session = SessionState::default();

        loop {
            self.prune_context_if_needed(&mut messages);

            // §36.54 — iteration cap.
            if max_iter::is_exhausted(iterations, self.max_iterations) {
                let cp = Checkpoint::new(iterations, all_calls.clone(), messages);
                return ToolLoopOutput {
                    final_text: String::new(),
                    iterations,
                    tool_calls: all_calls,
                    total_usage,
                    stop_reason: StopReason::MaxIterations,
                    checkpoint: Some(cp),
                };
            }

            // §36.45 — cancellation between turns.
            if ctx.is_cancelled() {
                let cp = Checkpoint::new(iterations, all_calls.clone(), messages);
                return ToolLoopOutput {
                    final_text: String::new(),
                    iterations,
                    tool_calls: all_calls,
                    total_usage,
                    stop_reason: StopReason::Cancelled,
                    checkpoint: Some(cp),
                };
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
                    let cp = Checkpoint::new(iterations, all_calls.clone(), messages);
                    return ToolLoopOutput {
                        final_text: String::new(),
                        iterations,
                        tool_calls: all_calls,
                        total_usage,
                        stop_reason: StopReason::BackendError(e.to_string()),
                        checkpoint: Some(cp),
                    };
                }
            };
            merge_session_state(&mut session, self.backend.extract_session(&response));
            total_usage.add(&response.extract_usage());

            // Parse tool calls from the response.
            let calls = match self.translator.parse_calls(&response) {
                Ok(c) => c,
                Err(e) => {
                    let cp = Checkpoint::new(iterations, all_calls.clone(), messages);
                    return ToolLoopOutput {
                        final_text: String::new(),
                        iterations,
                        tool_calls: all_calls,
                        total_usage,
                        stop_reason: StopReason::BackendError(format!("parse: {e}")),
                        checkpoint: Some(cp),
                    };
                }
            };

            // No tool calls -> final answer.
            if calls.is_empty() {
                self.clear_checkpoint_file();
                let final_text = response.extract_text();
                return ToolLoopOutput {
                    final_text,
                    iterations,
                    tool_calls: all_calls,
                    total_usage,
                    stop_reason: StopReason::Stop,
                    checkpoint: None,
                };
            }

            // Inject the assistant's tool-call message into conversation history.
            if let Some(assistant_msg) = self.translator.render_assistant_message(&response) {
                messages.push(assistant_msg);
            }

            // Dispatch tool calls (§36.41 parallel/serial batching).
            let results = self.dispatcher.dispatch_batch(calls.clone(), ctx).await;
            all_calls.extend(calls);

            // §36.56 — shape results into messages for the next turn.
            let rendered_results = self.translator.render_results(&results);
            result_msg::append_results(&mut messages, rendered_results);

            // §36.55 — context-growth guard.
            self.prune_context_if_needed(&mut messages);

            iterations += 1;
            self.save_checkpoint_snapshot(iterations, &all_calls, &messages);
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
    ) {
        let Some(path) = self.checkpoint_path.as_deref() else {
            return;
        };

        let cp = Checkpoint::new(iterations, all_calls.to_vec(), messages.to_vec());
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
}
