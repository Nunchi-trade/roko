use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::agent::Agent;
use crate::claude_agent::{AnthropicTool, DEFAULT_ANTHROPIC_VERSION, DEFAULT_BASE_URL};
use roko_core::defaults::DEFAULT_MAX_OUTPUT_TOKENS;
use crate::dispatcher::HandlerResolver;
use crate::http::{HttpPoster, ReqwestPoster};
use crate::provider::openai_compat::tool_registry_for_options;
use crate::provider::{
    AgentCreationError, AgentOptions, ProviderSemaphores, build_tool_dispatcher,
    tool_loop_max_iterations,
};
use crate::tool_loop::{LlmBackend, LlmError, ToolLoop, ToolLoopAgent};
use crate::translate::{
    BackendResponse, RenderedResults, RenderedTools, SessionState, Translator, TranslatorError,
};
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use roko_core::tool::{ToolCall, ToolDef, ToolFormat, ToolResult};

pub(super) fn create_tool_loop_agent(
    api_key: String,
    provider: &ProviderConfig,
    model: &ModelProfile,
    options: &AgentOptions,
) -> Result<Box<dyn Agent>, AgentCreationError> {
    let (registry, tools) = tool_registry_for_options(model, options)?;
    let resolver: Arc<dyn HandlerResolver> =
        Arc::new(|name: &str| roko_std::tool::handlers::handler_for(name));
    let dispatcher = build_tool_dispatcher(registry, resolver);
    let translator: Arc<dyn Translator> = Arc::new(AnthropicTranslator);
    let backend = create_tool_loop_backend_with_api_key(
        api_key,
        provider,
        model,
        options,
        Box::new(ReqwestPoster::new()),
    )?;

    let tool_loop = ToolLoop::new(translator, dispatcher, backend)
        .with_max_iterations(tool_loop_max_iterations(50))
        .with_context_token_limit(usize::try_from(model.context_window).unwrap_or(usize::MAX))
        .with_model_profile(model.clone());

    let name = if options.name.is_empty() {
        format!("anthropic-tool-loop:{}", model.slug)
    } else {
        options.name.clone()
    };

    let mut agent = ToolLoopAgent::new(tool_loop)
        .with_tools(tools)
        .with_name(name);
    if let Some(prompt) = &options.system_prompt {
        agent = agent.with_system_prompt(prompt.clone());
    }
    if let Some(ref dir) = options.working_dir {
        agent = agent.with_worktree_path(dir.clone());
    }

    Ok(Box::new(agent))
}

pub(crate) fn create_tool_loop_backend(
    provider: &ProviderConfig,
    model: &ModelProfile,
    options: &AgentOptions,
    poster: Box<dyn HttpPoster>,
) -> Result<Arc<dyn LlmBackend>, AgentCreationError> {
    let api_key = provider.resolve_api_key().ok_or_else(|| {
        AgentCreationError::MissingApiKey(provider.api_key_env.clone().unwrap_or_default())
    })?;
    create_tool_loop_backend_with_api_key(api_key, provider, model, options, poster)
}

/// Create an Anthropic Messages API backend + translator pair from a raw API key.
///
/// This is the lightweight entry point for callers that have an API key but
/// no `ProviderConfig` / `ModelProfile` (e.g. `roko run`).
pub fn create_anthropic_backend_simple(
    api_key: String,
    model: &str,
    timeout_ms: u64,
) -> (Arc<dyn LlmBackend>, Arc<dyn Translator>) {
    let backend = AnthropicMessagesBackend::new(api_key, model).with_timeout_ms(timeout_ms);
    let translator: Arc<dyn Translator> = Arc::new(AnthropicTranslator);
    (Arc::new(backend), translator)
}

fn create_tool_loop_backend_with_api_key(
    api_key: String,
    provider: &ProviderConfig,
    model: &ModelProfile,
    options: &AgentOptions,
    poster: Box<dyn HttpPoster>,
) -> Result<Arc<dyn LlmBackend>, AgentCreationError> {
    let timeout_ms = options
        .timeout_ms
        .or(provider.timeout_ms)
        .unwrap_or(120_000);

    let mut backend = AnthropicMessagesBackend::new(api_key, model.slug.clone())
        .with_provider_id(model.provider.clone())
        .with_base_url(super::AnthropicApiAdapter::base_url(provider))
        .with_timeout_ms(timeout_ms)
        .with_max_tokens(
            model
                .max_output
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS),
        )
        .with_extra_headers(provider.extra_headers.clone().unwrap_or_default())
        .with_poster(poster);

    if let Some(provider_semaphores) = options.provider_semaphores.clone() {
        backend = backend.with_provider_semaphores(provider_semaphores);
    }

    Ok(Arc::new(backend))
}

#[derive(Debug, Default, Clone, Copy)]
struct AnthropicTranslator;

impl Translator for AnthropicTranslator {
    fn format(&self) -> ToolFormat {
        ToolFormat::AnthropicBlocks
    }

    fn render_tools(&self, tools: &[ToolDef]) -> RenderedTools {
        let definitions: Vec<AnthropicTool> = tools
            .iter()
            .map(|tool| {
                AnthropicTool::new(
                    tool.name.clone(),
                    tool.description.clone(),
                    tool.parameters.as_value().clone(),
                )
            })
            .collect();

        RenderedTools::JsonArray(json!(definitions))
    }

    fn parse_calls(&self, response: &BackendResponse) -> Result<Vec<ToolCall>, TranslatorError> {
        let BackendResponse::Json(json) = response else {
            return Err(TranslatorError::Malformed("expected json".into()));
        };

        let Some(blocks) = json.get("content").and_then(Value::as_array) else {
            return Ok(Vec::new());
        };

        let mut calls = Vec::new();
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }

            let id = block.get("id").and_then(Value::as_str).unwrap_or_default();
            let name = block
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| TranslatorError::Malformed("missing tool_use.name".into()))?
                .to_string();
            let input = block.get("input").cloned().unwrap_or_else(|| json!({}));

            calls.push(ToolCall::new(id, name, input));
        }

        Ok(calls)
    }

    fn render_results(&self, results: &[(ToolCall, ToolResult)]) -> RenderedResults {
        let messages: Vec<Value> = results
            .iter()
            .map(|(call, result)| {
                json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": call.id.clone(),
                        "content": tool_result_content(result),
                        "is_error": matches!(result, ToolResult::Err(_)),
                    }]
                })
            })
            .collect();

        RenderedResults::JsonMessages(json!(messages))
    }

    fn render_assistant_message(&self, response: &BackendResponse) -> Option<Value> {
        let BackendResponse::Json(json) = response else {
            return None;
        };

        let content = json.get("content")?.clone();
        Some(json!({
            "role": "assistant",
            "content": content,
        }))
    }
}

fn tool_result_content(result: &ToolResult) -> Value {
    match result {
        ToolResult::Ok {
            content,
            is_structured,
            ..
        } if *is_structured => {
            serde_json::from_str(content).unwrap_or_else(|_| Value::String(content.clone()))
        }
        ToolResult::Ok { content, .. } => Value::String(content.clone()),
        ToolResult::Err(err) => Value::String(err.to_string()),
    }
}

struct AnthropicMessagesBackend {
    api_key: String,
    model: String,
    provider_id: String,
    base_url: String,
    timeout_ms: u64,
    max_tokens: u32,
    extra_headers: Vec<(String, String)>,
    provider_semaphores: Option<Arc<ProviderSemaphores>>,
    poster: Box<dyn HttpPoster>,
}

impl AnthropicMessagesBackend {
    fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let model = model.into();
        Self {
            api_key: api_key.into(),
            provider_id: model.clone(),
            model,
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout_ms: 120_000,
            max_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
            extra_headers: Vec::new(),
            provider_semaphores: None,
            poster: Box::new(ReqwestPoster::new()),
        }
    }

    fn with_provider_id(mut self, provider_id: impl Into<String>) -> Self {
        self.provider_id = provider_id.into();
        self
    }

    fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    fn with_extra_headers(mut self, extra_headers: HashMap<String, String>) -> Self {
        let mut extra_headers: Vec<(String, String)> = extra_headers.into_iter().collect();
        extra_headers.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        self.extra_headers = extra_headers;
        self
    }

    fn with_provider_semaphores(mut self, provider_semaphores: Arc<ProviderSemaphores>) -> Self {
        self.provider_semaphores = Some(provider_semaphores);
        self
    }

    fn with_poster(mut self, poster: Box<dyn HttpPoster>) -> Self {
        self.poster = poster;
        self
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/messages", self.base_url.trim_end_matches('/'))
    }

    fn headers(&self) -> Vec<(String, String)> {
        let mut headers = vec![
            ("content-type".to_owned(), "application/json".to_owned()),
            ("x-api-key".to_owned(), self.api_key.clone()),
            (
                "anthropic-version".to_owned(),
                DEFAULT_ANTHROPIC_VERSION.to_owned(),
            ),
        ];
        headers.extend(self.extra_headers.iter().cloned());
        headers
    }

    fn build_body(&self, messages: &[Value], tools: &RenderedTools) -> Result<Vec<u8>, LlmError> {
        let RenderedTools::JsonArray(tools) = tools else {
            return Err(LlmError::Backend("expected json tool array".into()));
        };

        let mut system_prompt = Vec::new();
        let mut anthropic_messages = Vec::with_capacity(messages.len());

        for message in messages {
            let Some(role) = message.get("role").and_then(Value::as_str) else {
                anthropic_messages.push(message.clone());
                continue;
            };

            if role == "system" {
                if let Some(content) = message.get("content").and_then(Value::as_str) {
                    system_prompt.push(content.to_string());
                }
                continue;
            }

            anthropic_messages.push(message.clone());
        }

        crate::translate::claude::inject_cache_markers(&mut anthropic_messages);

        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": anthropic_messages,
            "tools": tools,
        });

        if !system_prompt.is_empty() {
            let mut system = Value::String(system_prompt.join("\n"));
            let _ = crate::translate::claude::inject_cache_markers_into_content(&mut system);
            body["system"] = system;
        }

        serde_json::to_vec(&body).map_err(|err| LlmError::Backend(format!("serialize: {err}")))
    }

    fn normalize_response(raw: Value) -> Value {
        let content = raw.get("content").cloned().unwrap_or_else(|| json!([]));
        let text = content_as_text(&content);
        let usage = raw
            .get("usage")
            .map(normalize_usage)
            .unwrap_or_else(|| json!({}));

        json!({
            "id": raw.get("id").cloned().unwrap_or(Value::Null),
            "model": raw.get("model").cloned().unwrap_or(Value::Null),
            "stop_reason": raw.get("stop_reason").cloned().unwrap_or(Value::Null),
            "content": content,
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": text,
                }
            }],
            "usage": usage,
        })
    }
}

#[async_trait]
impl LlmBackend for AnthropicMessagesBackend {
    async fn send_turn(
        &self,
        messages: &[Value],
        tools: &RenderedTools,
        _session: &SessionState,
    ) -> Result<BackendResponse, LlmError> {
        let _permit = match (&self.provider_id, &self.provider_semaphores) {
            (provider_id, Some(provider_semaphores)) => {
                Some(provider_semaphores.acquire(provider_id).await)
            }
            _ => None,
        };

        let body_bytes = self.build_body(messages, tools)?;
        let raw = self
            .poster
            .post_json(
                &self.endpoint(),
                &self.headers(),
                &body_bytes,
                self.timeout_ms,
            )
            .await
            .map_err(|err| LlmError::Network(err.to_string()))?;

        let json: Value = serde_json::from_str(&raw)
            .map_err(|err| LlmError::Backend(format!("parse response: {err}")))?;

        Ok(BackendResponse::Json(Self::normalize_response(json)))
    }

    fn extract_session(&self, response: &BackendResponse) -> SessionState {
        match response {
            BackendResponse::Json(json) => SessionState {
                session_id: None,
                thread_id: None,
                conversation_id: json.get("id").and_then(Value::as_str).map(str::to_string),
            },
            BackendResponse::StreamJson(_) | BackendResponse::Text(_) => SessionState::default(),
        }
    }

    fn backend_id(&self) -> &'static str {
        "claude_api"
    }
}

fn content_as_text(content: &Value) -> String {
    let Some(blocks) = content.as_array() else {
        return content.as_str().unwrap_or_default().to_string();
    };

    let mut text = String::new();
    for block in blocks {
        let Some(block_type) = block.get("type").and_then(Value::as_str) else {
            continue;
        };

        if block_type == "text"
            && let Some(block_text) = block.get("text").and_then(Value::as_str)
        {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(block_text);
        }
    }
    text
}

fn normalize_usage(usage: &Value) -> Value {
    let input_tokens = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cached_tokens = usage
        .get("cache_read_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    json!({
        "prompt_tokens": input_tokens,
        "completion_tokens": output_tokens,
        "total_tokens": input_tokens + output_tokens,
        "prompt_tokens_details": {
            "cached_tokens": cached_tokens,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatcher::HandlerResolver;
    use crate::http::HttpPostError;
    use crate::provider::AgentOptions;
    use crate::provider::openai_compat::tool_registry_for_options;
    use crate::tool_loop::{LlmBackend, ToolLoop};
    use crate::translate::Translator;
    use roko_core::tool::{
        ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolHandler, ToolPermission,
    };
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[derive(Clone, Debug)]
    struct RecordedRequest {
        url: String,
        headers: Vec<(String, String)>,
        body: Value,
        timeout_ms: u64,
    }

    struct MockPoster {
        responses: Mutex<VecDeque<Result<String, HttpPostError>>>,
        requests: Mutex<Vec<RecordedRequest>>,
    }

    impl MockPoster {
        fn new(responses: Vec<Result<String, HttpPostError>>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().collect()),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl HttpPoster for MockPoster {
        async fn post_json(
            &self,
            url: &str,
            headers: &[(String, String)],
            body: &[u8],
            timeout_ms: u64,
        ) -> Result<String, HttpPostError> {
            let body: Value = serde_json::from_slice(body).expect("request body must be json");
            self.requests
                .lock()
                .expect("requests lock")
                .push(RecordedRequest {
                    url: url.to_string(),
                    headers: headers.to_vec(),
                    body,
                    timeout_ms,
                });
            self.responses
                .lock()
                .expect("responses lock")
                .pop_front()
                .expect("queued response")
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

    fn tool(name: &str) -> ToolDef {
        ToolDef::new(
            name,
            "test tool",
            ToolCategory::Meta,
            ToolPermission::read_only(),
        )
        .with_concurrency(ToolConcurrency::Parallel)
    }

    #[test]
    fn translator_renders_native_tools_and_results() {
        let tools = [tool("read_file")];
        let RenderedTools::JsonArray(rendered) = AnthropicTranslator.render_tools(&tools) else {
            panic!("expected JsonArray");
        };
        assert_eq!(rendered[0]["name"], "read_file");

        let call = ToolCall::new("call-1", "read_file", json!({"path": "x"}));
        let rendered = AnthropicTranslator.render_results(&[(call, ToolResult::text("ok"))]);
        let RenderedResults::JsonMessages(msgs) = rendered else {
            panic!("expected JsonMessages");
        };
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"][0]["type"], "tool_result");
        assert_eq!(msgs[0]["content"][0]["tool_use_id"], "call-1");
    }

    #[tokio::test]
    async fn backend_normalizes_anthropic_responses_for_tool_loop() {
        let poster = MockPoster::new(vec![Ok(json!({
            "id": "msg_1",
            "model": "claude-sonnet-4-6",
            "stop_reason": "tool_use",
            "content": [
                { "type": "text", "text": "working" },
                { "type": "tool_use", "id": "t1", "name": "echo", "input": { "value": 1 } }
            ],
            "usage": {
                "input_tokens": 11,
                "output_tokens": 22,
                "cache_read_input_tokens": 3,
                "cache_creation_input_tokens": 4
            }
        })
        .to_string())]);
        let backend = AnthropicMessagesBackend::new("test-key", "claude-sonnet-4-6")
            .with_base_url("https://example.test")
            .with_poster(Box::new(poster));

        let response = backend
            .send_turn(
                &[json!({"role": "user", "content": "hi"})],
                &RenderedTools::JsonArray(json!([])),
                &SessionState::default(),
            )
            .await
            .expect("send turn");

        let text = response.extract_text();
        assert_eq!(text, "working");
        assert_eq!(response.extract_usage().input_tokens, 11);
        assert_eq!(response.extract_usage().cache_read_tokens, 3);
        let calls = AnthropicTranslator
            .parse_calls(&response)
            .expect("parse calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "echo");
    }

    #[tokio::test]
    async fn tool_loop_agent_executes_anthropic_tool_calls() {
        let first_response = json!({
            "id": "msg_1",
            "model": "claude-sonnet-4-6",
            "stop_reason": "tool_use",
            "content": [
                { "type": "tool_use", "id": "t1", "name": "ls", "input": {} }
            ],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 1,
                "cache_read_input_tokens": 0,
                "cache_creation_input_tokens": 0
            }
        })
        .to_string();
        let second_response = json!({
            "id": "msg_2",
            "model": "claude-sonnet-4-6",
            "stop_reason": "end_turn",
            "content": [
                { "type": "text", "text": "anthropic-final" }
            ],
            "usage": {
                "input_tokens": 12,
                "output_tokens": 2,
                "cache_read_input_tokens": 1,
                "cache_creation_input_tokens": 0
            }
        })
        .to_string();
        let poster = MockPoster::new(vec![Ok(first_response), Ok(second_response)]);

        let (registry, tools) = tool_registry_for_options(
            &ModelProfile {
                provider: "anthropic".to_string(),
                slug: "claude-sonnet-4-6".to_string(),
                context_window: 200_000,
                supports_tools: true,
                tool_format: "anthropic_blocks".to_string(),
                ..Default::default()
            },
            &AgentOptions {
                tools: Some("ls".to_string()),
                ..Default::default()
            },
        )
        .expect("tools");
        let resolver: Arc<dyn HandlerResolver> =
            Arc::new(|name: &str| roko_std::tool::handlers::handler_for(name));
        let dispatcher = build_tool_dispatcher(registry, resolver);
        let backend = AnthropicMessagesBackend::new("test-key", "claude-sonnet-4-6")
            .with_base_url("https://example.test")
            .with_poster(Box::new(poster));
        let tool_loop = ToolLoop::new(Arc::new(AnthropicTranslator), dispatcher, Arc::new(backend));
        let ctx = ToolContext::testing(".");

        let output = tool_loop.run("system", "please use ls", &tools, &ctx).await;

        assert_eq!(output.final_text, "anthropic-final");
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].name, "ls");
    }

    #[test]
    fn content_as_text_joins_text_blocks_only() {
        let content = json!([
            { "type": "text", "text": "hello" },
            { "type": "tool_use", "id": "x", "name": "echo", "input": {} },
            { "type": "text", "text": "world" }
        ]);
        assert_eq!(content_as_text(&content), "hello\nworld");
    }
}
