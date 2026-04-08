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
//! - [`prune`] — context-growth guard (§36.55).
//! - [`result_msg`] — tool-result message construction (§36.56).
//! - [`checkpoint`] — resumable state (§36.57).

use std::sync::Arc;

use async_trait::async_trait;
use roko_core::tool::{ToolCall, ToolContext, ToolDef};

use crate::dispatcher::ToolDispatcher;
use crate::translate::{BackendResponse, RenderedTools, Translator};

pub mod checkpoint;
pub mod max_iter;
pub mod prune;
pub mod result_msg;

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
    ) -> Result<BackendResponse, LlmError>;
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
    /// Why the loop stopped.
    pub stop_reason: StopReason,
    /// Resumable snapshot — populated when `stop_reason != Stop`.
    pub checkpoint: Option<Checkpoint>,
}

// ─── ToolLoop ────────────────────────────────────────────────────────

/// Multi-turn tool-calling loop (§36.f).
///
/// Drives the `prompt -> LLM -> tool_calls -> dispatch -> results -> LLM`
/// cycle until the LLM stops calling tools, the iteration cap is
/// reached, the cancel token fires, or the backend errors.
pub struct ToolLoop {
    translator: Arc<dyn Translator>,
    dispatcher: Arc<ToolDispatcher>,
    backend: Arc<dyn LlmBackend>,
    max_iterations: usize,
    context_token_limit: usize,
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

    /// Run a fresh tool loop from an initial system + user prompt.
    pub async fn run(
        &self,
        system: &str,
        user: &str,
        tools: &[ToolDef],
        ctx: &ToolContext,
    ) -> ToolLoopOutput {
        let messages = result_msg::initial_messages(system, user);
        self.run_inner(messages, 0, Vec::new(), tools, ctx).await
    }

    /// Resume a tool loop from a previously saved [`Checkpoint`].
    pub async fn resume(
        &self,
        cp: Checkpoint,
        tools: &[ToolDef],
        ctx: &ToolContext,
    ) -> ToolLoopOutput {
        self.run_inner(cp.messages, cp.iterations, cp.tool_calls, tools, ctx)
            .await
    }

    /// Core loop shared by [`run`](Self::run) and [`resume`](Self::resume).
    async fn run_inner(
        &self,
        mut messages: Vec<serde_json::Value>,
        mut iterations: usize,
        mut all_calls: Vec<ToolCall>,
        tools: &[ToolDef],
        ctx: &ToolContext,
    ) -> ToolLoopOutput {
        let rendered_tools = self.translator.render_tools(tools);

        loop {
            // §36.54 — iteration cap.
            if max_iter::is_exhausted(iterations, self.max_iterations) {
                let cp = Checkpoint::new(iterations, all_calls.clone(), messages);
                return ToolLoopOutput {
                    final_text: String::new(),
                    iterations,
                    tool_calls: all_calls,
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
                    stop_reason: StopReason::Cancelled,
                    checkpoint: Some(cp),
                };
            }

            // Send current conversation to the backend.
            let response = match self.backend.send_turn(&messages, &rendered_tools).await {
                Ok(r) => r,
                Err(e) => {
                    let cp = Checkpoint::new(iterations, all_calls.clone(), messages);
                    return ToolLoopOutput {
                        final_text: String::new(),
                        iterations,
                        tool_calls: all_calls,
                        stop_reason: StopReason::BackendError(e.to_string()),
                        checkpoint: Some(cp),
                    };
                }
            };

            // Parse tool calls from the response.
            let calls = match self.translator.parse_calls(&response) {
                Ok(c) => c,
                Err(e) => {
                    let cp = Checkpoint::new(iterations, all_calls.clone(), messages);
                    return ToolLoopOutput {
                        final_text: String::new(),
                        iterations,
                        tool_calls: all_calls,
                        stop_reason: StopReason::BackendError(format!("parse: {e}")),
                        checkpoint: Some(cp),
                    };
                }
            };

            // No tool calls -> final answer.
            if calls.is_empty() {
                let final_text = response.extract_text();
                return ToolLoopOutput {
                    final_text,
                    iterations,
                    tool_calls: all_calls,
                    stop_reason: StopReason::Stop,
                    checkpoint: None,
                };
            }

            // Dispatch tool calls (§36.41 parallel/serial batching).
            let results = self.dispatcher.dispatch_batch(calls.clone(), ctx).await;
            all_calls.extend(calls);

            // §36.56 — shape results into messages for the next turn.
            let rendered_results = self.translator.render_results(&results);
            result_msg::append_results(&mut messages, rendered_results);

            // §36.55 — context-growth guard.
            prune::prune_if_needed(&mut messages, self.context_token_limit);

            iterations += 1;
        }
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

    #[async_trait]
    impl LlmBackend for CapturingBackend {
        async fn send_turn(
            &self,
            messages: &[serde_json::Value],
            _tools: &RenderedTools,
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
    async fn debug_impl_does_not_panic() {
        let backend = Arc::new(FinalAnswerBackend { text: "x".into() });
        let tl = make_tool_loop(backend, 25);
        let s = format!("{tl:?}");
        assert!(s.contains("ToolLoop"));
        assert!(s.contains("max_iterations"));
    }
}
