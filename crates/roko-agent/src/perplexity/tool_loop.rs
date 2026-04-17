//! Perplexity-native tool-loop agent and backend.
//!
//! This keeps Perplexity on the shared `ToolLoop` + `ToolDispatcher` path for
//! tool-capable models while preserving Perplexity metadata in the emitted
//! output signal.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::agent::{Agent, AgentResult, derived_output};
use crate::http::{HttpPoster, ReqwestPoster};
use crate::tool_loop::{LlmBackend, LlmError, StopReason, ToolLoop};
use crate::translate::{BackendResponse, RenderedTools, SessionState};
use async_trait::async_trait;
use roko_core::tool::{ToolContext, ToolDef};
use roko_core::{Body, Context, Engram, Kind, Provenance};
use roko_fs::RokoLayout;
use serde_json::Value;

use super::types::{PerplexityMetadata, SearchOptions};
use super::wire::{
    apply_search_options, base_chat_body, chat_endpoint, headers, metadata_is_empty,
    parse_pplx_meta,
};

pub trait PerplexityMetadataSource: Send + Sync {
    fn take_last_metadata(&self) -> Option<PerplexityMetadata>;
}

pub struct PerplexityToolLoopBackend {
    api_key: String,
    base_url: String,
    model_slug: String,
    search_options: SearchOptions,
    timeout_ms: u64,
    poster: Box<dyn HttpPoster>,
    last_metadata: Mutex<Option<PerplexityMetadata>>,
}

impl PerplexityToolLoopBackend {
    #[must_use]
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model_slug: impl Into<String>,
        search_options: SearchOptions,
        timeout_ms: u64,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into(),
            model_slug: model_slug.into(),
            search_options,
            timeout_ms,
            poster: Box::new(ReqwestPoster::new()),
            last_metadata: Mutex::new(None),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_poster(mut self, poster: Box<dyn HttpPoster>) -> Self {
        self.poster = poster;
        self
    }

    fn endpoint(&self) -> String {
        chat_endpoint(&self.base_url)
    }

    fn request_body(&self, messages: &[Value], tools: &RenderedTools) -> Result<Value, LlmError> {
        let mut body = base_chat_body(&self.model_slug, messages.to_vec());
        let Some(map) = body.as_object_mut() else {
            return Err(LlmError::Backend(
                "perplexity request body malformed".into(),
            ));
        };
        apply_search_options(map, &self.search_options);
        match tools {
            RenderedTools::JsonArray(value) => {
                map.insert("tools".to_string(), value.clone());
                Ok(body)
            }
            RenderedTools::CliFlag(_) | RenderedTools::SystemPromptBlock(_) => Err(
                LlmError::Backend("perplexity tool loop requires JSON tool rendering".into()),
            ),
        }
    }
}

impl PerplexityMetadataSource for PerplexityToolLoopBackend {
    fn take_last_metadata(&self) -> Option<PerplexityMetadata> {
        self.last_metadata.lock().expect("metadata lock").take()
    }
}

#[async_trait]
impl LlmBackend for PerplexityToolLoopBackend {
    async fn send_turn(
        &self,
        messages: &[Value],
        tools: &RenderedTools,
        _session: &SessionState,
    ) -> Result<BackendResponse, LlmError> {
        let body = self.request_body(messages, tools)?;
        let body_bytes = serde_json::to_vec(&body)
            .map_err(|e| LlmError::Backend(format!("serialize request failed: {e}")))?;

        let response_text = self
            .poster
            .post_json(
                &self.endpoint(),
                &headers(&self.api_key),
                &body_bytes,
                self.timeout_ms,
            )
            .await
            .map_err(|e| LlmError::Network(e.to_string()))?;

        let parsed: Value = serde_json::from_str(&response_text)
            .map_err(|e| LlmError::Backend(format!("malformed response json: {e}")))?;

        if let Some(err) = parsed.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown api error");
            return Err(LlmError::Backend(format!("api error: {msg}")));
        }

        let meta = parse_pplx_meta(&parsed);
        if !metadata_is_empty(&meta) {
            *self.last_metadata.lock().expect("metadata lock") = Some(meta);
        }

        Ok(BackendResponse::Json(parsed))
    }

    fn backend_id(&self) -> &'static str {
        "perplexity"
    }
}

pub struct PerplexityToolLoopAgent {
    tool_loop: ToolLoop,
    metadata_source: Arc<dyn PerplexityMetadataSource>,
    system_prompt: Option<String>,
    tools: Vec<ToolDef>,
    name: String,
    model_slug: String,
    worktree_path: PathBuf,
}

impl PerplexityToolLoopAgent {
    #[must_use]
    pub fn new(
        tool_loop: ToolLoop,
        metadata_source: Arc<dyn PerplexityMetadataSource>,
        model_slug: impl Into<String>,
    ) -> Self {
        Self {
            tool_loop,
            metadata_source,
            system_prompt: None,
            tools: Vec::new(),
            name: "perplexity-tool-loop".to_string(),
            model_slug: model_slug.into(),
            worktree_path: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    #[must_use]
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    #[must_use]
    pub fn with_tools(mut self, tools: Vec<ToolDef>) -> Self {
        self.tools = tools;
        self
    }

    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    #[must_use]
    pub fn with_worktree_path(mut self, worktree_path: impl Into<PathBuf>) -> Self {
        self.worktree_path = worktree_path.into();
        self
    }

    fn output_signal(
        &self,
        input: &Engram,
        text: &str,
        stop_reason: &str,
        iterations: usize,
        metadata: Option<PerplexityMetadata>,
    ) -> Engram {
        let mut builder = derived_output(input, Kind::AgentOutput, Body::text(text))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("model", &self.model_slug)
            .tag("stop_reason", stop_reason)
            .tag("iterations", iterations.to_string());
        if let Some(meta) = metadata {
            let meta_json = serde_json::to_string(&meta).unwrap_or_default();
            builder = builder.tag("pplx_meta", &meta_json);
        }
        builder.build()
    }

    fn checkpoint_path(&self, ctx: &Context) -> Option<PathBuf> {
        let task_id = crate::task_runner::task_id_from_context(ctx);
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
impl Agent for PerplexityToolLoopAgent {
    async fn run(&self, input: &Engram, ctx: &Context) -> AgentResult {
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

        let metadata = self.metadata_source.take_last_metadata();
        match output.stop_reason {
            StopReason::Stop => AgentResult::ok(self.output_signal(
                input,
                &output.final_text,
                "stop",
                output.iterations,
                metadata,
            ))
            .with_usage(output.total_usage),
            StopReason::MaxIterations => AgentResult::fail(self.output_signal(
                input,
                &format!("Max iterations ({}) reached", output.iterations),
                "max_iterations",
                output.iterations,
                metadata,
            ))
            .with_usage(output.total_usage),
            StopReason::Cancelled => AgentResult::fail(self.output_signal(
                input,
                "Tool loop cancelled",
                "cancelled",
                output.iterations,
                metadata,
            ))
            .with_usage(output.total_usage),
            StopReason::BackendError(err) => AgentResult::fail(self.output_signal(
                input,
                &err,
                "backend_error",
                output.iterations,
                metadata,
            ))
            .with_usage(output.total_usage),
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn backend_id(&self) -> &'static str {
        self.tool_loop.backend_id()
    }

    fn supports_streaming(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatcher::{HandlerResolver, ToolDispatcher};
    use crate::http::HttpPostError;
    use crate::tool_loop::{LlmBackend, LlmError, ToolLoop};
    use crate::translate::{
        BackendResponse, RenderedResults, RenderedTools, SessionState, Translator, TranslatorError,
    };
    use roko_core::tool::{
        ToolCall, ToolCategory, ToolContext, ToolDef, ToolFormat, ToolHandler, ToolPermission,
        ToolResult, ToolSchema, VecToolRegistry,
    };
    use roko_core::{Body, Context, Engram, Kind};
    use serde_json::{Value, json};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    fn echo_tool() -> ToolDef {
        ToolDef::new(
            "echo",
            "echo",
            ToolCategory::Meta,
            ToolPermission::read_only(),
        )
        .with_parameters(ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "value": { "type": "integer" }
            },
            "required": ["value"]
        })))
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

    struct MockTranslator;

    impl Translator for MockTranslator {
        fn format(&self) -> ToolFormat {
            ToolFormat::OpenAiJson
        }

        fn render_tools(&self, tools: &[ToolDef]) -> RenderedTools {
            let tools: Vec<Value> = tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters.as_value(),
                        }
                    })
                })
                .collect();
            RenderedTools::JsonArray(Value::Array(tools))
        }

        fn parse_calls(
            &self,
            response: &BackendResponse,
        ) -> Result<Vec<ToolCall>, TranslatorError> {
            let BackendResponse::Json(json) = response else {
                return Ok(Vec::new());
            };
            let Some(calls) = json
                .pointer("/choices/0/message/tool_calls")
                .and_then(Value::as_array)
            else {
                return Ok(Vec::new());
            };
            Ok(calls
                .iter()
                .map(|call| {
                    ToolCall::new(
                        call["id"].as_str().unwrap_or_default(),
                        call.pointer("/function/name")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        call.pointer("/function/arguments")
                            .cloned()
                            .unwrap_or_else(|| json!({})),
                    )
                })
                .collect())
        }

        fn render_results(&self, results: &[(ToolCall, ToolResult)]) -> RenderedResults {
            let messages: Vec<Value> = results
                .iter()
                .map(|(call, result)| {
                    let content = match result {
                        ToolResult::Ok { content, .. } => content.clone(),
                        ToolResult::Err(err) => format!("error: {err}"),
                    };
                    json!({
                        "role": "tool",
                        "tool_call_id": call.id,
                        "content": content,
                    })
                })
                .collect();
            RenderedResults::JsonMessages(Value::Array(messages))
        }
    }

    struct SequenceBackend {
        calls: AtomicUsize,
        metadata: Mutex<Option<PerplexityMetadata>>,
    }

    impl SequenceBackend {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                metadata: Mutex::new(None),
            }
        }
    }

    impl PerplexityMetadataSource for SequenceBackend {
        fn take_last_metadata(&self) -> Option<PerplexityMetadata> {
            self.metadata.lock().expect("metadata lock").take()
        }
    }

    #[async_trait]
    impl LlmBackend for SequenceBackend {
        async fn send_turn(
            &self,
            _messages: &[Value],
            _tools: &RenderedTools,
            _session: &SessionState,
        ) -> Result<BackendResponse, LlmError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                Ok(BackendResponse::Json(json!({
                    "choices": [{
                        "message": {
                            "tool_calls": [{
                                "id": "call-1",
                                "type": "function",
                                "function": {
                                    "name": "echo",
                                    "arguments": "{\"value\":1}"
                                }
                            }]
                        }
                    }]
                })))
            } else {
                let response = json!({
                    "choices": [{
                        "message": { "content": "final answer" }
                    }],
                    "citations": ["https://example.com/source"],
                    "search_results": [{
                        "url": "https://example.com/source",
                        "title": "Example",
                        "content": "source",
                        "date": null,
                        "last_updated": null
                    }]
                });
                *self.metadata.lock().expect("metadata lock") = Some(parse_pplx_meta(&response));
                Ok(BackendResponse::Json(response))
            }
        }
    }

    #[derive(Clone, Debug, Default)]
    struct Captured {
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    struct MockPoster {
        captured: Arc<Mutex<Option<Captured>>>,
        response: Result<String, HttpPostError>,
    }

    impl MockPoster {
        fn ok(body: impl Into<String>) -> (Self, Arc<Mutex<Option<Captured>>>) {
            let captured = Arc::new(Mutex::new(None));
            (
                Self {
                    captured: captured.clone(),
                    response: Ok(body.into()),
                },
                captured,
            )
        }
    }

    #[async_trait]
    impl HttpPoster for MockPoster {
        async fn post_json(
            &self,
            url: &str,
            headers: &[(String, String)],
            body: &[u8],
            _timeout_ms: u64,
        ) -> Result<String, HttpPostError> {
            *self.captured.lock().expect("lock") = Some(Captured {
                url: url.to_string(),
                headers: headers.to_vec(),
                body: body.to_vec(),
            });
            self.response.clone()
        }
    }

    fn build_tool_loop(backend: Arc<SequenceBackend>) -> ToolLoop {
        let registry: Arc<dyn roko_core::tool::ToolRegistry> =
            Arc::new(VecToolRegistry::from_tools(vec![echo_tool()]));
        let resolver: Arc<dyn HandlerResolver> =
            Arc::new(|name: &str| -> Option<Arc<dyn ToolHandler>> {
                (name == "echo").then(|| Arc::new(EchoHandler) as Arc<dyn ToolHandler>)
            });
        let dispatcher = Arc::new(ToolDispatcher::new(registry, resolver));
        let translator: Arc<dyn Translator> = Arc::new(MockTranslator);
        ToolLoop::new(translator, dispatcher, backend)
    }

    fn prompt(text: &str) -> Engram {
        Engram::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    #[tokio::test]
    async fn tool_loop_agent_preserves_pplx_meta() {
        let backend = Arc::new(SequenceBackend::new());
        let agent = PerplexityToolLoopAgent::new(
            build_tool_loop(backend.clone()),
            backend.clone(),
            "sonar-tools",
        )
        .with_tools(vec![echo_tool()])
        .with_name("perplexity-tool-loop:sonar-tools")
        .with_worktree_path("/tmp");

        let result = agent.run(&prompt("hi"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(
            result.output.body.as_text().expect("text body"),
            "final answer"
        );
        assert_eq!(result.output.tag("model"), Some("sonar-tools"));
        assert_eq!(result.output.tag("stop_reason"), Some("stop"));
        assert!(result.output.tag("pplx_meta").is_some());
    }

    #[tokio::test]
    async fn backend_injects_search_options_and_tools() {
        let response = json!({
            "choices": [{
                "message": { "content": "ok" }
            }]
        })
        .to_string();
        let (poster, captured) = MockPoster::ok(response);
        let backend = PerplexityToolLoopBackend::new(
            "pplx-key",
            "https://api.perplexity.ai",
            "sonar",
            SearchOptions {
                search_domain_filter: Some(vec!["arxiv.org".into()]),
                search_recency_filter: Some("week".into()),
                search_context_size: Some("high".into()),
                return_related_questions: Some(true),
                user_location: Some(super::super::types::UserLocation {
                    country: Some("US".into()),
                    city: Some("Berlin".into()),
                    region: None,
                    timezone: Some("Europe/Berlin".into()),
                }),
                ..Default::default()
            },
            60_000,
        )
        .with_poster(Box::new(poster));

        let tools = RenderedTools::JsonArray(json!([{
            "type": "function",
            "function": {
                "name": "echo",
                "description": "echo",
                "parameters": { "type": "object", "properties": {} }
            }
        }]));
        let response = backend
            .send_turn(
                &[json!({"role": "user", "content": "hi"})],
                &tools,
                &SessionState::default(),
            )
            .await
            .expect("backend response");
        assert!(matches!(response, BackendResponse::Json(_)));

        let captured = captured.lock().expect("lock").clone().expect("captured");
        assert_eq!(captured.url, "https://api.perplexity.ai/chat/completions");
        let body: Value = serde_json::from_slice(&captured.body).expect("body json");
        assert_eq!(body["model"], "sonar");
        assert_eq!(body["search_domain_filter"][0], "arxiv.org");
        assert_eq!(body["search_recency_filter"], "week");
        assert_eq!(body["web_search_options"]["search_context_size"], "high");
        assert_eq!(body["return_related_questions"], true);
        assert_eq!(body["user_location"]["country"], "US");
        assert_eq!(body["tools"][0]["function"]["name"], "echo");
        let headers = &captured.headers;
        assert!(
            headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && v == "Bearer pplx-key")
        );
        assert!(
            headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v == "application/json")
        );
    }
}
