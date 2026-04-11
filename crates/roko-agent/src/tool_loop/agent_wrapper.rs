//! `ToolLoopAgent` — wrap [`ToolLoop`](super::ToolLoop) in the runtime-facing
//! [`Agent`](crate::agent::Agent) trait.

use std::path::PathBuf;

use async_trait::async_trait;
use roko_core::tool::{ToolContext, ToolDef};
use roko_core::{Body, Context, Kind, Signal};
use roko_fs::RokoLayout;

use crate::agent::{Agent, AgentResult};
use crate::task_runner::task_id_from_context;

use super::{StopReason, ToolLoop};

/// Runtime-facing wrapper that lets the orchestrator drive [`ToolLoop`] via
/// the existing [`Agent`] trait.
pub struct ToolLoopAgent {
    tool_loop: ToolLoop,
    system_prompt: Option<String>,
    tools: Vec<ToolDef>,
    name: String,
    worktree_path: PathBuf,
}

impl ToolLoopAgent {
    /// Construct a wrapper around an existing tool loop.
    #[must_use]
    pub fn new(tool_loop: ToolLoop) -> Self {
        Self {
            tool_loop,
            system_prompt: None,
            tools: Vec::new(),
            name: "tool-loop".to_string(),
            worktree_path: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    /// Attach a system prompt that is prepended on the first turn.
    #[must_use]
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Attach the tool definitions exposed to the model.
    #[must_use]
    pub fn with_tools(mut self, tools: Vec<ToolDef>) -> Self {
        self.tools = tools;
        self
    }

    /// Override the display name used by logs and the orchestrator.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Override the worktree root used for tool execution.
    #[must_use]
    pub fn with_worktree_path(mut self, worktree_path: impl Into<PathBuf>) -> Self {
        self.worktree_path = worktree_path.into();
        self
    }

    fn output_signal(text: &str, stop_reason: &str, iterations: usize) -> Signal {
        Signal::builder(Kind::AgentOutput)
            .body(Body::text(text))
            .tag("stop_reason", stop_reason)
            .tag("iterations", iterations.to_string())
            .build()
    }

    fn checkpoint_path(&self, ctx: &Context) -> Option<PathBuf> {
        let task_id = task_id_from_context(ctx);
        if task_id.is_empty() {
            return None;
        }

        let safe_task_id = task_id.replace(['/', '\\'], "_");
        Some(
            RokoLayout::for_project(&self.worktree_path)
                .state_dir()
                .join(format!("tool-loop-{safe_task_id}.json")),
        )
    }
}

#[async_trait]
impl Agent for ToolLoopAgent {
    async fn run(&self, input: &Signal, ctx: &Context) -> AgentResult {
        let prompt = input.body.as_text().unwrap_or_default();
        let tool_ctx = ToolContext::testing(&self.worktree_path);
        let tool_loop = match self.checkpoint_path(ctx) {
            Some(path) => self.tool_loop.clone().with_checkpoint_path(path),
            None => self.tool_loop.clone(),
        };
        let output = tool_loop
            .run(
                self.system_prompt.as_deref().unwrap_or(""),
                prompt,
                &self.tools,
                &tool_ctx,
            )
            .await;

        match output.stop_reason {
            StopReason::Stop => AgentResult::ok(Self::output_signal(
                &output.final_text,
                "stop",
                output.iterations,
            ))
            .with_usage(output.total_usage),
            StopReason::MaxIterations => AgentResult::fail(Self::output_signal(
                &format!("Max iterations ({}) reached", output.iterations),
                "max_iterations",
                output.iterations,
            ))
            .with_usage(output.total_usage),
            StopReason::Cancelled => AgentResult::fail(Self::output_signal(
                "Tool loop cancelled",
                "cancelled",
                output.iterations,
            ))
            .with_usage(output.total_usage),
            StopReason::BackendError(err) => AgentResult::fail(Self::output_signal(
                &err,
                "backend_error",
                output.iterations,
            ))
            .with_usage(output.total_usage),
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn supports_streaming(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::dispatcher::{HandlerResolver, ToolDispatcher};
    use crate::tool_loop::{LlmBackend, LlmError, ToolLoop};
    use crate::translate::{
        BackendResponse, RenderedResults, RenderedTools, Translator, TranslatorError,
    };
    use roko_core::tool::{
        ToolCall, ToolCategory, ToolConcurrency, ToolFormat, ToolHandler, ToolPermission,
        ToolResult, VecToolRegistry,
    };

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
            let BackendResponse::Json(ref value) = *response else {
                return Ok(Vec::new());
            };
            let Some(calls) = value.get("tool_calls").and_then(|value| value.as_array()) else {
                return Ok(Vec::new());
            };

            Ok(calls
                .iter()
                .map(|call| {
                    ToolCall::new(
                        call["id"].as_str().unwrap_or_default(),
                        call["name"].as_str().unwrap_or_default(),
                        call.get("arguments")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({})),
                    )
                })
                .collect())
        }

        fn render_results(&self, results: &[(ToolCall, ToolResult)]) -> RenderedResults {
            let messages: Vec<serde_json::Value> = results
                .iter()
                .map(|(call, result)| {
                    let content = match result {
                        ToolResult::Ok { content, .. } => content.clone(),
                        ToolResult::Err(err) => format!("error: {err}"),
                    };
                    serde_json::json!({
                        "role": "tool",
                        "tool_call_id": call.id,
                        "content": content,
                    })
                })
                .collect();
            RenderedResults::JsonMessages(serde_json::json!(messages))
        }
    }

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
            _session: &crate::translate::SessionState,
        ) -> Result<BackendResponse, LlmError> {
            let call = self.call_count.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                Ok(BackendResponse::Json(serde_json::json!({
                    "tool_calls": [{
                        "id": "call-1",
                        "name": "echo",
                        "arguments": { "value": 1 }
                    }]
                })))
            } else {
                Ok(BackendResponse::Json(
                    serde_json::json!({"message": {"content": "final answer"}}),
                ))
            }
        }
    }

    struct ErrorBackend;

    #[async_trait]
    impl LlmBackend for ErrorBackend {
        async fn send_turn(
            &self,
            _messages: &[serde_json::Value],
            _tools: &RenderedTools,
            _session: &crate::translate::SessionState,
        ) -> Result<BackendResponse, LlmError> {
            Err(LlmError::Backend("server error".into()))
        }
    }

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

    fn make_tool_loop(backend: Arc<dyn LlmBackend>) -> ToolLoop {
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
        ToolLoop::new(translator, dispatcher, backend)
    }

    #[tokio::test]
    async fn tool_loop_agent_wrapper_runs_tool_loop() {
        let agent = ToolLoopAgent::new(make_tool_loop(Arc::new(TwoStepBackend::new())))
            .with_name("glm-tool-loop")
            .with_system_prompt("system prompt")
            .with_tools(test_tools())
            .with_worktree_path("/tmp");
        let input = Signal::builder(Kind::Prompt)
            .body(Body::text("call the tool"))
            .build();

        let result = agent.run(&input, &Context::now()).await;

        assert!(result.success);
        assert_eq!(
            result.output.body.as_text().expect("text output"),
            "final answer"
        );
        assert_eq!(result.output.tag("stop_reason"), Some("stop"));
        assert_eq!(result.output.tag("iterations"), Some("1"));
        assert_eq!(agent.name(), "glm-tool-loop");
        assert!(!agent.supports_streaming());
    }

    #[tokio::test]
    async fn tool_loop_agent_wrapper_maps_backend_errors_to_failures() {
        let agent = ToolLoopAgent::new(make_tool_loop(Arc::new(ErrorBackend)))
            .with_tools(test_tools())
            .with_worktree_path("/tmp");
        let input = Signal::builder(Kind::Prompt)
            .body(Body::text("fail"))
            .build();

        let result = agent.run(&input, &Context::now()).await;

        assert!(!result.success);
        assert_eq!(
            result.output.body.as_text().expect("text output"),
            "backend error: server error"
        );
        assert_eq!(result.output.tag("stop_reason"), Some("backend_error"));
    }
}
