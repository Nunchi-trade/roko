//! Gemini native `generateContent` agent.

use std::sync::Arc;
use std::time::Instant;

use crate::agent::derived_output;
#[cfg(test)]
use crate::http::HttpPostError;
use crate::http::{HttpPoster, ReqwestPoster};
use crate::provider::AgentOptions;
use crate::provider::current_safety_layer;
use crate::safety::SafetyLayer;
use crate::translate::{ChatResponse, FinishReason, ResponseMetadata, normalize_finish_reason};
use crate::usage::{UsageObservation, UsageSource};
use crate::{Agent, AgentResult};
use async_trait::async_trait;
use roko_core::config::schema::ModelProfile;
use roko_core::tool::{ToolCall, ToolDef};
use roko_core::{Body, Context, Engram, Kind, Provenance};
use serde_json::{Value, json};

use super::types::{
    Content, FunctionDeclaration, FunctionResponsePart, GeminiMetadata, GeminiTool,
    GenerateContentRequest, GenerateContentResponse, GenerationConfig, Part, SafetySettingRequest,
    ThinkingConfig,
};
use super::wire::{
    generate_content_endpoint, generate_content_headers, send_generate_content_request,
    serialize_generate_content_request,
};

use roko_core::defaults::DEFAULT_REQUEST_TIMEOUT_MS;

const DEFAULT_TIMEOUT_MS: u64 = DEFAULT_REQUEST_TIMEOUT_MS;

pub(crate) fn system_instruction_from_segments(segments: Vec<String>) -> Option<Content> {
    let segments: Vec<String> = segments
        .into_iter()
        .map(|segment| segment.trim().to_string())
        .filter(|segment| !segment.is_empty())
        .collect();

    (!segments.is_empty()).then(|| Content {
        role: "system".to_string(),
        parts: vec![Part::Text {
            text: segments.join("\n\n"),
        }],
    })
}

pub(crate) fn build_generation_config(
    model: &ModelProfile,
    thinking_level: Option<&str>,
) -> Option<GenerationConfig> {
    let thinking_config = thinking_level
        .map(str::trim)
        .filter(|level| !level.is_empty())
        .map(|thinking_level| ThinkingConfig {
            thinking_level: thinking_level.to_string(),
        });

    if model.max_output.is_none() && thinking_config.is_none() {
        return None;
    }

    Some(GenerationConfig {
        temperature: None,
        top_p: None,
        max_output_tokens: model.max_output.and_then(|value| u32::try_from(value).ok()),
        stop_sequences: None,
        response_mime_type: None,
        response_schema: None,
        thinking_config,
    })
}

pub(crate) fn build_generate_content_request(
    contents: Vec<Content>,
    system_instruction: Option<Content>,
    tools: Option<Vec<GeminiTool>>,
    generation_config: Option<GenerationConfig>,
    safety_settings: Option<Vec<SafetySettingRequest>>,
    cached_content: Option<String>,
) -> GenerateContentRequest {
    GenerateContentRequest {
        contents,
        system_instruction,
        tools,
        tool_config: None,
        generation_config,
        safety_settings,
        cached_content,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq)]
enum ChatMessage {
    System(String),
    User(String),
    Assistant(String),
    ToolResult {
        name: String,
        tool_call_id: Option<String>,
        response: Value,
    },
}

/// Native Gemini chat agent backed by `models/*:generateContent`.
pub struct GeminiNativeAgent {
    api_key: String,
    base_url: String,
    model: ModelProfile,
    thinking_level: Option<String>,
    cached_content: Option<String>,
    enable_grounding: bool,
    enable_code_execution: bool,
    safety_settings: Vec<SafetySettingRequest>,
    safety: Option<SafetyLayer>,
    system_prompt: Option<String>,
    timeout_ms: u64,
    name: String,
    poster: Arc<dyn HttpPoster>,
}

impl GeminiNativeAgent {
    /// Construct a native Gemini agent from provider/model options.
    #[must_use]
    pub fn new(
        api_key: String,
        base_url: String,
        model: ModelProfile,
        options: &AgentOptions,
    ) -> Self {
        let name = if options.name.is_empty() {
            format!("gemini-native:{}", model.slug)
        } else {
            options.name.clone()
        };

        Self {
            api_key,
            base_url,
            thinking_level: options
                .effort
                .clone()
                .or_else(|| model.thinking_level.clone()),
            cached_content: options.cached_content.clone(),
            enable_grounding: model.supports_grounding,
            enable_code_execution: model.supports_code_execution,
            safety_settings: Vec::new(),
            safety: current_safety_layer(),
            system_prompt: options.system_prompt.clone(),
            timeout_ms: options.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
            name,
            model,
            poster: Arc::new(ReqwestPoster::new()),
        }
    }

    #[must_use]
    pub fn with_safety_layer(mut self, safety: Option<SafetyLayer>) -> Self {
        self.safety = safety;
        self
    }

    #[cfg(test)]
    #[must_use]
    fn with_http_poster(mut self, poster: Arc<dyn HttpPoster>) -> Self {
        self.poster = poster;
        self
    }

    fn failure(&self, input: &Engram, reason: String, started: &Instant) -> AgentResult {
        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let output = derived_output(input, Kind::AgentOutput, Body::text(reason))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("failed", "true")
            .build();
        AgentResult::fail(output).with_usage_obs(UsageObservation {
            input_tokens: None,
            output_tokens: None,
            cache_creation_tokens: None,
            cache_read_tokens: None,
            cost_usd: None,
            source: UsageSource::Unknown,
            model: Some(self.model.slug.clone()),
            wall_ms,
        })
    }

    fn build_request(&self, messages: &[ChatMessage], tools: &[ToolDef]) -> GenerateContentRequest {
        let system_instruction = self.system_instruction(messages);
        let contents = self.translate_messages(messages);
        let tools = self.translate_tools(tools);
        let generation_config = self.generation_config();

        build_generate_content_request(
            contents,
            system_instruction,
            (!tools.is_empty()).then_some(tools),
            generation_config,
            (!self.safety_settings.is_empty()).then_some(self.safety_settings.clone()),
            self.cached_content.clone(),
        )
    }

    fn system_instruction(&self, messages: &[ChatMessage]) -> Option<Content> {
        let mut segments = Vec::new();

        if let Some(system_prompt) = self
            .system_prompt
            .as_deref()
            .map(str::trim)
            .filter(|prompt| !prompt.is_empty())
        {
            segments.push(system_prompt.to_string());
        }

        for message in messages {
            if let ChatMessage::System(text) = message {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    segments.push(trimmed.to_string());
                }
            }
        }

        system_instruction_from_segments(segments)
    }

    fn translate_messages(&self, messages: &[ChatMessage]) -> Vec<Content> {
        messages
            .iter()
            .filter_map(|message| match message {
                ChatMessage::System(_) => None,
                ChatMessage::User(text) => Some(Content {
                    role: "user".to_string(),
                    parts: vec![Part::Text { text: text.clone() }],
                }),
                ChatMessage::Assistant(text) => Some(Content {
                    role: "model".to_string(),
                    parts: vec![Part::Text { text: text.clone() }],
                }),
                ChatMessage::ToolResult {
                    name,
                    tool_call_id,
                    response,
                } => Some(Content {
                    role: "function".to_string(),
                    parts: vec![Part::FunctionResponse {
                        function_response: FunctionResponsePart {
                            name: name.clone(),
                            response: response.clone(),
                            id: tool_call_id.clone(),
                        },
                    }],
                }),
            })
            .collect()
    }

    fn translate_tools(&self, tools: &[ToolDef]) -> Vec<GeminiTool> {
        let mut gemini_tools = Vec::new();

        if !tools.is_empty() {
            let declarations = tools
                .iter()
                .map(|tool| FunctionDeclaration {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    parameters: tool.parameters.as_value().clone(),
                })
                .collect();
            gemini_tools.push(GeminiTool::FunctionDeclarations {
                function_declarations: declarations,
            });
        }

        if self.enable_grounding {
            gemini_tools.push(GeminiTool::GoogleSearch {
                google_search: json!({}),
            });
        }

        if self.enable_code_execution {
            gemini_tools.push(GeminiTool::CodeExecution {
                code_execution: json!({}),
            });
        }

        gemini_tools
    }

    fn generation_config(&self) -> Option<GenerationConfig> {
        build_generation_config(&self.model, self.thinking_level.as_deref())
    }

    fn parse_response(&self, response: &GenerateContentResponse) -> Result<ChatResponse, String> {
        let candidate = response
            .candidates
            .first()
            .ok_or_else(|| "response missing candidates[0]".to_string())?;

        let mut text_parts = Vec::new();
        let mut tool_calls = Vec::new();
        let mut code_results = Vec::new();

        for part in &candidate.content.parts {
            match part {
                Part::Text { text } => text_parts.push(text.clone()),
                Part::FunctionCall { function_call } => tool_calls.push(ToolCall::new(
                    function_call
                        .id
                        .clone()
                        .unwrap_or_else(|| format!("call_{}", tool_calls.len())),
                    function_call.name.clone(),
                    function_call.args.clone(),
                )),
                Part::CodeExecutionResult {
                    code_execution_result,
                } => code_results.push(code_execution_result.clone()),
                Part::ExecutableCode { .. }
                | Part::FunctionResponse { .. }
                | Part::InlineData { .. } => {}
            }
        }

        let raw_finish_reason = candidate.finish_reason.clone();
        let finish_reason = raw_finish_reason
            .as_deref()
            .map(|reason| normalize_finish_reason(&reason.to_ascii_lowercase()))
            .unwrap_or(FinishReason::Stop);

        let usage_metadata = response.usage_metadata.as_ref();
        let cached_tokens = usage_metadata.and_then(|usage| usage.cached_content_token_count);
        let metadata = GeminiMetadata {
            grounding_metadata: candidate.grounding_metadata.clone(),
            code_execution_results: code_results.clone(),
            thinking_tokens: usage_metadata.and_then(|usage| usage.thinking_token_count),
            cached_tokens,
            safety_ratings: candidate.safety_ratings.clone(),
        };

        Ok(ChatResponse {
            content: text_parts.join(""),
            reasoning: None,
            tool_calls,
            usage: gemini_observation(usage_metadata, 0, Some(self.model.slug.clone())).into(),
            finish_reason,
            metadata: ResponseMetadata {
                model_used: Some(self.model.slug.clone()),
                cached_tokens,
                extra: serde_json::to_value(metadata).ok(),
                raw_finish_reason,
                ..Default::default()
            },
            ..Default::default()
        })
    }
}

#[async_trait]
impl Agent for GeminiNativeAgent {
    async fn run(&self, input: &Engram, _ctx: &Context) -> AgentResult {
        let started = Instant::now();

        let prompt_text = match input.body.as_text() {
            Ok(text) => text.to_string(),
            Err(_) => match serde_json::to_string(&input.body) {
                Ok(text) => text,
                Err(error) => {
                    return self.failure(
                        input,
                        format!("input body not readable as text or json: {error}"),
                        &started,
                    );
                }
            },
        };

        let request = self.build_request(&[ChatMessage::User(prompt_text)], &[]);
        let body = match serialize_generate_content_request(&request) {
            Ok(body) => body,
            Err(error) => {
                return self.failure(
                    input,
                    format!("request serialize failed: {error}"),
                    &started,
                );
            }
        };
        let response_text = match send_generate_content_request(
            &*self.poster,
            &generate_content_endpoint(&self.base_url, &self.model.slug),
            &generate_content_headers(&self.api_key),
            &body,
            self.timeout_ms,
        )
        .await
        {
            Ok(response) => response,
            Err(error) => {
                return self.failure(input, format!("http error: {error}"), &started);
            }
        };

        let response = match serde_json::from_str::<GenerateContentResponse>(&response_text) {
            Ok(response) => response,
            Err(error) => {
                return self.failure(input, format!("malformed response json: {error}"), &started);
            }
        };

        let parsed = match self.parse_response(&response) {
            Ok(parsed) => parsed,
            Err(error) => return self.failure(input, error, &started),
        };

        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let observation = gemini_observation(
            response.usage_metadata.as_ref(),
            wall_ms,
            Some(self.model.slug.clone()),
        );

        let content = self
            .safety
            .as_ref()
            .map(|safety| safety.scrub_text(&parsed.content))
            .unwrap_or_else(|| parsed.content.clone());
        let mut builder = derived_output(input, Kind::AgentOutput, Body::text(content))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("model", &self.model.slug);
        if let Some(meta_json) = parsed
            .metadata
            .extra
            .as_ref()
            .and_then(|meta| serde_json::to_string(meta).ok())
        {
            builder = builder.tag("gemini_meta", &meta_json);
        }
        let output = builder.build();

        AgentResult::ok(output).with_usage_obs(observation)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn backend_id(&self) -> &'static str {
        "gemini"
    }

    fn supports_streaming(&self) -> bool {
        false
    }
}

fn saturating_u64_to_u32(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Build a canonical [`UsageObservation`] from Gemini's `usageMetadata`.
///
/// `None` for `usage_metadata` means the API did not report usage at all
/// — every token field is left as `None` instead of collapsing to `0`.
/// When `usage_metadata` is present, `prompt_token_count` and
/// `candidates_token_count` are surfaced as `Some(n)` (preserving an
/// explicit `0`); `cached_content_token_count` is already optional in
/// the wire shape and flows through unchanged.
fn gemini_observation(
    usage_metadata: Option<&super::types::UsageMetadata>,
    wall_ms: u64,
    model: Option<String>,
) -> UsageObservation {
    let (input_tokens, output_tokens, cache_read_tokens, source) = match usage_metadata {
        Some(usage) => (
            Some(usage.prompt_token_count),
            usage.candidates_token_count,
            usage.cached_content_token_count,
            UsageSource::ProviderReported,
        ),
        None => (None, None, None, UsageSource::Unknown),
    };

    UsageObservation {
        input_tokens,
        output_tokens,
        cache_creation_tokens: None,
        cache_read_tokens,
        cost_usd: None,
        source,
        model,
        wall_ms,
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_types)] // tests use std::sync::Mutex for simplicity
mod tests {
    use super::*;
    use std::sync::Mutex;

    use roko_core::config::schema::ModelProfile;
    use roko_core::tool::{ToolCategory, ToolPermission, ToolSchema};

    #[derive(Clone, Debug, Default)]
    struct Captured {
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        timeout_ms: u64,
    }

    #[derive(Debug)]
    struct MockPoster {
        captured: Arc<Mutex<Captured>>,
        response: Result<String, HttpPostError>,
    }

    impl MockPoster {
        fn ok(captured: Arc<Mutex<Captured>>, response: serde_json::Value) -> Self {
            Self {
                captured,
                response: Ok(response.to_string()),
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
            let mut captured = self.captured.lock().expect("capture lock");
            captured.url = url.to_string();
            captured.headers = headers.to_vec();
            captured.body = body.to_vec();
            captured.timeout_ms = timeout_ms;
            self.response.clone()
        }
    }

    fn base_model() -> ModelProfile {
        ModelProfile {
            provider: "gemini".to_string(),
            slug: "gemini-2.5-pro".to_string(),
            context_window: 1_048_576,
            max_output: Some(65_536),
            supports_tools: true,
            supports_thinking: true,
            supports_vision: false,
            supports_web_search: false,
            supports_mcp_tools: false,
            supports_partial: false,
            supports_grounding: true,
            supports_code_execution: true,
            supports_caching: false,
            provider_routing: None,
            tool_format: "gemini_native".to_string(),
            cost_input_per_m: None,
            cost_output_per_m: None,
            cost_input_per_m_high: None,
            cost_output_per_m_high: None,
            cost_cache_read_per_m: None,
            cost_cache_write_per_m: None,
            thinking_level: Some("high".to_string()),
            max_tools: None,
            tokenizer_ratio: None,
            supports_search: false,
            supports_citations: false,
            supports_async: false,
            is_embedding_model: false,
            search_context_size: None,
            cost_per_request: None,
        }
    }

    fn prompt(text: &str) -> Engram {
        Engram::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    #[test]
    fn gemini_native_agent_translate_messages_maps_roles() {
        let agent = GeminiNativeAgent::new(
            "test-key".to_string(),
            "https://generativelanguage.googleapis.com".to_string(),
            base_model(),
            &AgentOptions {
                system_prompt: Some("system seed".to_string()),
                ..Default::default()
            },
        );

        let messages = vec![
            ChatMessage::System("system from history".to_string()),
            ChatMessage::User("question".to_string()),
            ChatMessage::Assistant("answer".to_string()),
            ChatMessage::ToolResult {
                name: "read_file".to_string(),
                tool_call_id: Some("call-1".to_string()),
                response: json!({ "content": "fn main() {}" }),
            },
        ];

        let contents = agent.translate_messages(&messages);
        let system_instruction = agent.system_instruction(&messages);

        assert_eq!(contents.len(), 3);
        assert_eq!(contents[0].role, "user");
        assert_eq!(contents[1].role, "model");
        assert_eq!(contents[2].role, "function");
        assert!(matches!(
            &contents[2].parts[0],
            Part::FunctionResponse { function_response }
                if function_response.name == "read_file"
                    && function_response.id.as_deref() == Some("call-1")
        ));
        assert!(matches!(
            system_instruction,
            Some(Content { role, parts })
                if role == "system"
                    && matches!(&parts[0], Part::Text { text }
                        if text == "system seed\n\nsystem from history")
        ));
    }

    #[test]
    fn gemini_native_agent_translate_tools_includes_builtins_and_custom_tools() {
        let agent = GeminiNativeAgent::new(
            "test-key".to_string(),
            "https://generativelanguage.googleapis.com".to_string(),
            base_model(),
            &AgentOptions::default(),
        );
        let tools = vec![
            ToolDef::new(
                "read_file",
                "Read a file from disk",
                ToolCategory::Read,
                ToolPermission::read_only(),
            )
            .with_parameters(ToolSchema::from_value(json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"]
            }))),
        ];

        let translated = agent.translate_tools(&tools);

        assert_eq!(translated.len(), 3);
        assert!(matches!(
            &translated[0],
            GeminiTool::FunctionDeclarations { function_declarations }
                if function_declarations.len() == 1
                    && function_declarations[0].name == "read_file"
                    && function_declarations[0].parameters["required"] == json!(["path"])
        ));
        assert!(matches!(&translated[1], GeminiTool::GoogleSearch { .. }));
        assert!(matches!(&translated[2], GeminiTool::CodeExecution { .. }));
    }

    #[test]
    fn gemini_native_agent_parse_response_preserves_metadata() {
        let agent = GeminiNativeAgent::new(
            "test-key".to_string(),
            "https://generativelanguage.googleapis.com".to_string(),
            base_model(),
            &AgentOptions::default(),
        );
        let response = serde_json::from_value::<GenerateContentResponse>(json!({
            "candidates": [
                {
                    "content": {
                        "role": "model",
                        "parts": [
                            { "text": "Grounded answer. " },
                            {
                                "functionCall": {
                                    "name": "read_file",
                                    "args": { "path": "src/lib.rs" },
                                    "id": "call-7"
                                }
                            },
                            {
                                "codeExecutionResult": {
                                    "outcome": "OUTCOME_OK",
                                    "output": "tests passed"
                                }
                            },
                            { "text": "Done." }
                        ]
                    },
                    "finishReason": "STOP",
                    "safetyRatings": [
                        {
                            "category": "HARM_CATEGORY_HARASSMENT",
                            "probability": "NEGLIGIBLE"
                        }
                    ],
                    "groundingMetadata": {
                        "webSearchQueries": ["rust edition 2024 let chains"],
                        "groundingChunks": [
                            {
                                "web": {
                                    "uri": "https://doc.rust-lang.org/edition-guide/rust-2024/",
                                    "title": "Rust Edition Guide"
                                }
                            }
                        ]
                    }
                }
            ],
            "usageMetadata": {
                "promptTokenCount": 21,
                "candidatesTokenCount": 8,
                "totalTokenCount": 29,
                "cachedContentTokenCount": 5,
                "thinkingTokenCount": 3
            }
        }))
        .expect("parse response");

        let parsed = agent
            .parse_response(&response)
            .expect("parse chat response");

        assert_eq!(parsed.content, "Grounded answer. Done.");
        assert_eq!(parsed.usage.input_tokens, 21);
        assert_eq!(parsed.usage.output_tokens, 8);
        assert_eq!(parsed.usage.cache_read_tokens, 5);
        assert_eq!(parsed.finish_reason, FinishReason::Stop);
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].id, "call-7");
        assert_eq!(parsed.tool_calls[0].name, "read_file");
        assert_eq!(
            parsed.tool_calls[0].arguments,
            json!({ "path": "src/lib.rs" })
        );
        assert_eq!(
            parsed.metadata.model_used.as_deref(),
            Some("gemini-2.5-pro")
        );
        assert_eq!(parsed.metadata.cached_tokens, Some(5));
        assert_eq!(parsed.metadata.raw_finish_reason.as_deref(), Some("STOP"));

        let extra = parsed.metadata.extra.expect("gemini metadata");
        let metadata: GeminiMetadata =
            serde_json::from_value(extra).expect("deserialize gemini metadata");
        assert_eq!(metadata.thinking_tokens, Some(3));
        assert_eq!(metadata.cached_tokens, Some(5));
        assert_eq!(metadata.code_execution_results.len(), 1);
        assert_eq!(metadata.code_execution_results[0].outcome, "OUTCOME_OK");
        assert_eq!(metadata.code_execution_results[0].output, "tests passed");
        assert_eq!(
            metadata
                .grounding_metadata
                .and_then(|grounding| grounding.web_search_queries)
                .expect("grounding queries"),
            vec!["rust edition 2024 let chains".to_string()]
        );
    }

    #[tokio::test]
    async fn gemini_native_agent_run_sends_generate_content_request() {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let poster = Arc::new(MockPoster::ok(
            Arc::clone(&captured),
            json!({
                "candidates": [
                    {
                        "content": {
                            "role": "model",
                            "parts": [{ "text": "native response" }]
                        },
                        "finishReason": "STOP",
                        "groundingMetadata": {
                            "webSearchQueries": ["inspect this crate"],
                            "groundingChunks": [
                                {
                                    "web": {
                                        "uri": "https://example.com/source",
                                        "title": "Example Source"
                                    }
                                }
                            ]
                        }
                    }
                ],
                "usageMetadata": {
                    "promptTokenCount": 11,
                    "candidatesTokenCount": 4,
                    "totalTokenCount": 15
                }
            }),
        ));

        let agent = GeminiNativeAgent::new(
            "test-key".to_string(),
            "https://generativelanguage.googleapis.com".to_string(),
            base_model(),
            &AgentOptions {
                cached_content: Some("cachedContents/cache-123".to_string()),
                system_prompt: Some("system seed".to_string()),
                timeout_ms: Some(9_999),
                effort: Some("low".to_string()),
                ..Default::default()
            },
        )
        .with_http_poster(poster);

        let result = agent
            .run(&prompt("inspect this crate"), &Context::now())
            .await;

        assert!(result.success);
        assert_eq!(
            result.output.body.as_text().expect("output text"),
            "native response"
        );
        assert_eq!(result.usage.input_tokens, 11);
        assert_eq!(result.usage.output_tokens, 4);
        let metadata: GeminiMetadata =
            serde_json::from_str(result.output.tag("gemini_meta").expect("gemini_meta tag"))
                .expect("deserialize gemini_meta");
        assert_eq!(
            metadata
                .grounding_metadata
                .and_then(|grounding| grounding.web_search_queries)
                .expect("grounding queries"),
            vec!["inspect this crate".to_string()]
        );

        let captured = captured.lock().expect("capture lock");
        assert_eq!(
            captured.url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:generateContent"
        );
        assert_eq!(captured.timeout_ms, 9_999);
        assert!(
            captured
                .headers
                .iter()
                .any(|(key, value)| { key == "x-goog-api-key" && value == "test-key" })
        );

        let body: Value = serde_json::from_slice(&captured.body).expect("request body json");
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "system seed");
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(
            body["contents"][0]["parts"][0]["text"],
            "inspect this crate"
        );
        assert_eq!(body["cachedContent"], "cachedContents/cache-123");
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "low"
        );
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 65_536);
        let tools = body["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 2);
        assert!(tools.iter().any(|tool| tool.get("google_search").is_some()));
        assert!(
            tools
                .iter()
                .any(|tool| tool.get("code_execution").is_some())
        );
    }

    #[tokio::test]
    async fn gemini_native_agent_preserves_lineage_and_scrubs_output_when_safety_is_attached() {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let poster = Arc::new(MockPoster::ok(
            Arc::clone(&captured),
            json!({
                "candidates": [
                    {
                        "content": {
                            "role": "model",
                            "parts": [{ "text": "PASSWORD=hunter2" }]
                        },
                        "finishReason": "STOP"
                    }
                ]
            }),
        ));
        let ancestor = Engram::builder(Kind::Prompt)
            .body(Body::text("ancestor"))
            .build();
        let input = Engram::builder(Kind::Prompt)
            .body(Body::text("show the secret"))
            .lineage([ancestor.id])
            .build();

        let agent = GeminiNativeAgent::new(
            "test-key".to_string(),
            "https://generativelanguage.googleapis.com".to_string(),
            base_model(),
            &AgentOptions::default(),
        )
        .with_safety_layer(Some(SafetyLayer::with_defaults()))
        .with_http_poster(poster);

        let result = agent.run(&input, &Context::now()).await;

        assert!(result.success);
        assert_eq!(
            result.output.body.as_text().expect("output text"),
            "PASSWORD=[REDACTED]"
        );
        assert_eq!(result.output.lineage, vec![ancestor.id, input.id]);
    }

    #[tokio::test]
    async fn gemini_usage_distinguishes_absent_from_zero() {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let poster_zero = Arc::new(MockPoster::ok(
            Arc::clone(&captured),
            json!({
                "candidates": [
                    {
                        "content": {
                            "role": "model",
                            "parts": [{ "text": "ok" }]
                        },
                        "finishReason": "STOP"
                    }
                ],
                "usageMetadata": {
                    "promptTokenCount": 0,
                    "candidatesTokenCount": 0,
                    "totalTokenCount": 0,
                    "cachedContentTokenCount": 0
                }
            }),
        ));
        let agent_zero = GeminiNativeAgent::new(
            "test-key".to_string(),
            "https://generativelanguage.googleapis.com".to_string(),
            base_model(),
            &AgentOptions::default(),
        )
        .with_http_poster(poster_zero);
        let result_zero = agent_zero.run(&prompt("hi"), &Context::now()).await;
        let obs_zero = result_zero.usage_obs.expect("usage_obs populated");
        assert_eq!(obs_zero.input_tokens, Some(0));
        assert_eq!(obs_zero.output_tokens, Some(0));
        assert_eq!(obs_zero.cache_read_tokens, Some(0));
        assert_eq!(obs_zero.source, UsageSource::ProviderReported);

        let captured_absent = Arc::new(Mutex::new(Captured::default()));
        let poster_absent = Arc::new(MockPoster::ok(
            Arc::clone(&captured_absent),
            json!({
                "candidates": [
                    {
                        "content": {
                            "role": "model",
                            "parts": [{ "text": "ok" }]
                        },
                        "finishReason": "STOP"
                    }
                ]
            }),
        ));
        let agent_absent = GeminiNativeAgent::new(
            "test-key".to_string(),
            "https://generativelanguage.googleapis.com".to_string(),
            base_model(),
            &AgentOptions::default(),
        )
        .with_http_poster(poster_absent);
        let result_absent = agent_absent.run(&prompt("hi"), &Context::now()).await;
        let obs_absent = result_absent.usage_obs.expect("usage_obs populated");
        assert_eq!(obs_absent.input_tokens, None);
        assert_eq!(obs_absent.output_tokens, None);
        assert_eq!(obs_absent.cache_read_tokens, None);
        assert_eq!(obs_absent.source, UsageSource::Unknown);
    }
}
