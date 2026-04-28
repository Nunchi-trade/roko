//! ModelCallService -- concrete implementation of `ModelCaller`.
//!
//! Wraps the existing provider dispatch (`create_agent_for_model`) with model
//! resolution, cost tracking, event emission, and feedback recording.

use crate::provider::{AgentOptions, create_agent_for_model};
use crate::task_runner::CostTable;
use async_trait::async_trait;
use roko_core::agent::ProviderKind;
use roko_core::config::schema::{ModelProfile, ProviderConfig, RokoConfig};
use roko_core::foundation::{
    ChatMessage, FeedbackEvent, FeedbackSink, MessageRole, ModelCallRequest, ModelCallResponse,
    ModelCaller, TokenUsage,
};
use roko_core::{
    Body, Context, Engram, EventConsumer, Kind, Result, RokoError, RuntimeEvent, Usage,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

type ModelRouter = dyn Fn(Option<&str>) -> String + Send + Sync;

/// Predicted cost for a model call before it is executed.
#[derive(Debug, Clone)]
pub struct CostEstimate {
    /// Model that would be used.
    pub model: String,
    /// Estimated input token count (prompt length heuristic: 4 chars/token).
    pub estimated_input_tokens: u64,
    /// Worst-case output tokens (from max_tokens, or a default of 2048).
    pub max_output_tokens: u64,
    /// Predicted cost in USD, or 0.0 if the model has no pricing entry.
    pub predicted_cost_usd: f64,
}

/// Service that calls LLM models via the existing provider infrastructure.
///
/// This is the canonical way to call models in the workflow engine. It:
/// - Resolves a request model, falling back to the configured default model
/// - Tracks token usage and cost
/// - Emits RuntimeEvents for observability
/// - Records feedback for learning when a feedback sink is configured
pub struct ModelCallService {
    /// Default model to use when request doesn't specify one.
    default_model: String,
    /// Provider/model configuration used by `create_agent_for_model`.
    config: RokoConfig,
    /// Optional pricing table for calculating cost from raw token usage.
    cost_table: CostTable,
    /// Optional event consumers for runtime observability.
    event_consumers: Vec<Arc<dyn EventConsumer>>,
    /// Optional sink for model-call feedback.
    feedback_sink: Option<Arc<dyn FeedbackSink>>,
    /// Optional model router used when requests omit an explicit model.
    model_router: Option<Arc<ModelRouter>>,
    /// Service-scoped environment entries passed into provider construction.
    env: Vec<(String, String)>,
    /// Optional base URL for OpenAI-compatible providers.
    openai_base_url: Option<String>,
    /// Explicit MCP config path threaded into provider options.
    mcp_config: Option<PathBuf>,
    /// Run id used for emitted events and feedback when the request has none.
    run_id: String,
}

impl ModelCallService {
    /// Create a new ModelCallService with the given default model.
    #[must_use]
    pub fn new(default_model: String) -> Self {
        Self {
            default_model,
            config: RokoConfig::default(),
            cost_table: CostTable::default(),
            event_consumers: Vec::new(),
            feedback_sink: None,
            model_router: None,
            env: Vec::new(),
            openai_base_url: None,
            mcp_config: None,
            run_id: "model-call-service".to_string(),
        }
    }

    /// Use an explicit Roko configuration for provider dispatch.
    #[must_use]
    pub fn with_config(mut self, config: RokoConfig) -> Self {
        self.config = config;
        self
    }

    /// Use an explicit pricing table for cost calculation.
    #[must_use]
    pub fn with_cost_table(mut self, cost_table: CostTable) -> Self {
        self.cost_table = cost_table;
        self
    }

    /// Attach a runtime event consumer.
    #[must_use]
    pub fn with_event_consumer(mut self, consumer: Arc<dyn EventConsumer>) -> Self {
        self.event_consumers.push(consumer);
        self
    }

    /// Attach a feedback sink.
    #[must_use]
    pub fn with_feedback_sink(mut self, feedback_sink: Arc<dyn FeedbackSink>) -> Self {
        self.feedback_sink = Some(feedback_sink);
        self
    }

    /// Attach a CascadeRouter-compatible callback for model selection.
    #[must_use]
    pub fn with_cascade_router<F>(mut self, router_fn: F) -> Self
    where
        F: Fn(Option<&str>) -> String + Send + Sync + 'static,
    {
        self.model_router = Some(Arc::new(router_fn));
        self
    }

    /// Provide an Anthropic API key for service-created agents.
    #[must_use]
    pub fn with_anthropic_api_key(mut self, key: String) -> Self {
        self.set_env("ANTHROPIC_API_KEY", key);
        self
    }

    /// Configure the base URL used by implicit OpenAI-compatible routes.
    #[must_use]
    pub fn with_openai_base_url(mut self, url: String) -> Self {
        self.openai_base_url = Some(url);
        self
    }

    /// Use an explicit MCP config path for service-created agents.
    #[must_use]
    pub fn with_mcp_config(mut self, path: impl Into<PathBuf>) -> Self {
        self.mcp_config = Some(path.into());
        self
    }

    /// Use a specific run id for emitted events and feedback.
    #[must_use]
    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = run_id.into();
        self
    }

    /// Resolve which model to use for a request.
    fn resolve_model(&self, req: &ModelCallRequest) -> String {
        if req.model.is_empty() {
            if let Some(router) = &self.model_router {
                return router(req.role.as_deref());
            }
            self.default_model.clone()
        } else {
            req.model.clone()
        }
    }

    /// Predict the cost of a model call before executing it.
    #[must_use]
    pub fn cost_predict(&self, req: &ModelCallRequest) -> CostEstimate {
        let model = self.resolve_model(req);
        let total_chars = req
            .system
            .as_deref()
            .map_or(0_u64, |system| system.chars().count() as u64)
            + req
                .messages
                .iter()
                .map(|message| message.content.chars().count() as u64)
                .sum::<u64>();
        let estimated_input_tokens = total_chars / 4;
        let max_output_tokens = u64::from(req.max_tokens.unwrap_or(2048));
        let usage_estimate = Usage {
            input_tokens: estimated_input_tokens.min(u64::from(u32::MAX)) as u32,
            output_tokens: max_output_tokens.min(u64::from(u32::MAX)) as u32,
            ..Usage::zero()
        };
        let predicted_cost_usd = self.cost_table.calculate(&model, &usage_estimate);

        CostEstimate {
            model,
            estimated_input_tokens,
            max_output_tokens,
            predicted_cost_usd,
        }
    }

    fn set_env(&mut self, key: &str, value: String) {
        if let Some((_, existing)) = self.env.iter_mut().find(|(name, _)| name == key) {
            *existing = value;
        } else {
            self.env.push((key.to_string(), value));
        }
    }

    fn has_env(&self, key: &str) -> bool {
        self.env
            .iter()
            .any(|(name, value)| name == key && !value.is_empty())
    }

    fn config_for_model(&self, model: &str) -> RokoConfig {
        let mut config = self.config.clone();

        if let Some(url) = &self.openai_base_url {
            config.providers.insert(
                "openai-compat".to_string(),
                ProviderConfig {
                    kind: ProviderKind::OpenAiCompat,
                    base_url: Some(url.clone()),
                    api_key_env: Some("OPENAI_API_KEY".to_string()),
                    command: None,
                    args: None,
                    timeout_ms: Some(120_000),
                    ttft_timeout_ms: Some(15_000),
                    connect_timeout_ms: Some(5_000),
                    extra_headers: None,
                    max_concurrent: None,
                },
            );
        }

        let has_explicit_model = config.models.contains_key(model)
            || config.models.values().any(|profile| profile.slug == model);
        let has_anthropic_provider = config
            .providers
            .values()
            .any(|provider| provider.kind == ProviderKind::AnthropicApi);

        if model.starts_with("claude-") && self.has_env("ANTHROPIC_API_KEY") {
            if !has_anthropic_provider {
                config.providers.insert(
                    "anthropic".to_string(),
                    ProviderConfig {
                        kind: ProviderKind::AnthropicApi,
                        base_url: Some("https://api.anthropic.com".to_string()),
                        api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
                        command: None,
                        args: None,
                        timeout_ms: Some(120_000),
                        ttft_timeout_ms: Some(15_000),
                        connect_timeout_ms: Some(5_000),
                        extra_headers: None,
                        max_concurrent: None,
                    },
                );
            }

            if !has_explicit_model {
                config.models.insert(
                    model.to_string(),
                    ModelProfile {
                        provider: "anthropic".to_string(),
                        slug: model.to_string(),
                        context_window: 200_000,
                        tool_format: "anthropic_blocks".to_string(),
                        ..Default::default()
                    },
                );
            }
        } else if self.openai_base_url.is_some()
            && !model.starts_with("claude-")
            && !has_explicit_model
        {
            config.models.insert(
                model.to_string(),
                ModelProfile {
                    provider: "openai-compat".to_string(),
                    slug: model.to_string(),
                    context_window: 128_000,
                    tool_format: "openai_json".to_string(),
                    ..Default::default()
                },
            );
        }

        config
    }

    fn build_agent_options(
        &self,
        req: &ModelCallRequest,
        system_prompt: Option<String>,
    ) -> AgentOptions {
        let mut options = AgentOptions {
            system_prompt,
            mcp_config: self.mcp_config.clone(),
            name: req.role.clone().unwrap_or_else(|| "model_call".to_string()),
            env: self.env.clone(),
            ..AgentOptions::default()
        };
        options.mcp_config = self
            .mcp_config
            .clone()
            .or_else(|| self.config_agent_mcp_config());
        options
    }

    fn config_agent_mcp_config(&self) -> Option<PathBuf> {
        // TODO(converge): Replace this with `self.config.agent.mcp_config.clone()`
        // once the roko-core `AgentConfig` in this worktree exposes that field.
        // This batch's write scope forbids adding the missing schema field here.
        let _ = &self.config;
        None
    }

    fn emit(&self, event: RuntimeEvent) {
        for consumer in &self.event_consumers {
            consumer.consume(&event);
        }
    }

    async fn record_feedback(
        &self,
        req: &ModelCallRequest,
        model: &str,
        usage: &TokenUsage,
        latency_ms: u64,
        success: bool,
    ) -> Result<()> {
        let Some(sink) = &self.feedback_sink else {
            // TODO(arch): Make a FeedbackSink mandatory at workflow construction
            // time so every model call is recorded without relying on optional
            // service wiring.
            return Ok(());
        };

        sink.record(FeedbackEvent::ModelCall {
            run_id: self.run_id.clone(),
            model: model.to_string(),
            role: req.role.clone().unwrap_or_else(|| "model_call".to_string()),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cost_usd: usage.cost_usd,
            latency_ms,
            success,
        })
        .await
    }
}

fn request_prompt(messages: &[ChatMessage]) -> (Option<String>, String) {
    let mut system_prompt = None;
    let mut user_content = String::new();

    for msg in messages {
        match msg.role {
            MessageRole::System => {
                system_prompt = Some(msg.content.clone());
            }
            MessageRole::User | MessageRole::Assistant => {
                if !user_content.is_empty() {
                    user_content.push_str("\n\n");
                }
                user_content.push_str(&msg.content);
            }
        }
    }

    (system_prompt, user_content)
}

fn output_text(output: &Engram) -> String {
    match &output.body {
        Body::Text(text) => text.clone(),
        Body::Json(value) => value.to_string(),
        Body::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        Body::Empty => String::new(),
    }
}

fn token_usage(usage: &Usage, cost_usd: f64) -> TokenUsage {
    TokenUsage {
        input_tokens: u64::from(usage.input_tokens),
        output_tokens: u64::from(usage.output_tokens),
        total_tokens: u64::from(usage.total_tokens()),
        cost_usd,
    }
}

/// Encapsulates a single provider execution attempt with fallback support.
struct ProviderCallCell {
    config: RokoConfig,
    cost_table: CostTable,
}

impl ProviderCallCell {
    fn new(config: RokoConfig, cost_table: CostTable) -> Self {
        Self { config, cost_table }
    }

    /// Execute a model call through the provider layer.
    ///
    /// 1. Calls `create_agent_for_model()` to get a `Box<dyn Agent>`.
    /// 2. Runs `agent.run()` with the prompt.
    /// 3. Classifies the result as success or failure.
    /// 4. On retryable failure, tries the next model in `fallback_models` (if any).
    /// 5. Returns the response or the final error.
    async fn execute(
        &self,
        model: &str,
        system_prompt: Option<&str>,
        user_content: &str,
        options: AgentOptions,
        fallback_models: &[String],
    ) -> std::result::Result<CellOutput, CellError> {
        let total_start = Instant::now();
        let prompt = Engram::builder(Kind::Prompt)
            .body(Body::text(user_content))
            .build();
        let mut last_error = None;

        for (attempt_index, attempt_model) in std::iter::once(model)
            .chain(fallback_models.iter().map(String::as_str))
            .enumerate()
        {
            let mut attempt_options = options.clone();
            attempt_options.system_prompt = system_prompt.map(ToOwned::to_owned);

            let agent = create_agent_for_model(&self.config, attempt_model, attempt_options)
                .map_err(|err| CellError::Construction {
                    message: err.to_string(),
                })?;

            let ctx = Context::now();
            let result = agent.run(&prompt, &ctx).await;
            let calculated_cost = self.cost_table.calculate(attempt_model, &result.usage);
            let cost_usd = if calculated_cost > 0.0 {
                calculated_cost
            } else {
                f64::from(result.usage.cost_usd)
            };

            if result.success {
                return Ok(CellOutput {
                    content: output_text(&result.output),
                    model_used: attempt_model.to_string(),
                    usage: result.usage,
                    cost_usd,
                    latency_ms: total_start.elapsed().as_millis() as u64,
                    fallback_used: attempt_index > 0,
                });
            }

            let message = output_text(&result.output);
            let error = if is_retryable_provider_message(&message) {
                CellError::Retryable { message }
            } else {
                CellError::Terminal { message }
            };

            if !matches!(error, CellError::Retryable { .. }) {
                return Err(error);
            }

            last_error = Some(error);
        }

        Err(last_error.unwrap_or_else(|| CellError::Retryable {
            message: "provider call failed with no fallback model available".to_string(),
        }))
    }
}

struct CellOutput {
    content: String,
    model_used: String,
    usage: Usage,
    cost_usd: f64,
    latency_ms: u64,
    fallback_used: bool,
}

/// Classified cell-level error.
#[derive(Debug)]
enum CellError {
    /// Provider returned an error but another provider may work.
    Retryable { message: String },
    /// Permanent failure, do not retry.
    Terminal { message: String },
    /// Agent construction failed (missing credentials, bad config).
    Construction { message: String },
}

impl CellError {
    fn message(&self) -> &str {
        match self {
            Self::Retryable { message }
            | Self::Terminal { message }
            | Self::Construction { message } => message,
        }
    }
}

fn is_retryable_provider_message(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("rate limit")
        || normalized.contains("timeout")
        || normalized.contains("timed out")
        || normalized.contains("503")
        || normalized.contains("temporarily unavailable")
}

#[async_trait]
impl ModelCaller for ModelCallService {
    async fn call(&self, req: ModelCallRequest) -> Result<ModelCallResponse> {
        let model = self.resolve_model(&req);
        let start = Instant::now();
        let agent_id = format!("model-call:{model}");
        let (message_system, user_content) = request_prompt(&req.messages);
        let system_prompt = req.system.clone().or(message_system);
        let config = self.config_for_model(&model);

        let options = self.build_agent_options(&req, None);
        // TODO(converge): Thread per-request generation settings through
        // AgentOptions/provider adapters. The Anthropic and OpenAI-compatible
        // adapters derive max tokens from ModelProfile::max_output and do not
        // parse "max_tokens=..." or "temperature=..." extra_args.
        // TODO(converge): Thread req-level MCP config here in S05 once
        // ModelCallRequest carries it.

        let fallback_models: Vec<String> = Vec::new();
        let cell = ProviderCallCell::new(config, self.cost_table.clone());
        let output = match cell
            .execute(
                &model,
                system_prompt.as_deref(),
                &user_content,
                options,
                &fallback_models,
            )
            .await
        {
            Ok(output) => output,
            Err(error) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                let usage = token_usage(&Usage::zero(), 0.0);
                let message = error.message().to_string();
                self.emit(RuntimeEvent::AgentFailed {
                    run_id: self.run_id.clone(),
                    agent_id,
                    error: message.clone(),
                });
                self.record_feedback(&req, &model, &usage, latency_ms, false)
                    .await?;
                return Err(RokoError::Agent {
                    backend: model,
                    message,
                });
            }
        };

        let usage = token_usage(&output.usage, output.cost_usd);

        self.emit(RuntimeEvent::AgentCompleted {
            run_id: self.run_id.clone(),
            agent_id: format!("model-call:{}", output.model_used),
            output: output.content.clone(),
            tokens_used: usage.total_tokens,
            cost_usd: usage.cost_usd,
        });
        self.record_feedback(&req, &output.model_used, &usage, output.latency_ms, true)
            .await?;

        Ok(ModelCallResponse {
            content: output.content,
            model: output.model_used,
            usage,
            stop_reason: Some("end_turn".to_string()),
            request_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_runner::ModelPricing;

    #[tokio::test]
    async fn default_model_resolution() {
        let svc = ModelCallService::new("claude-sonnet-4-20250514".into());
        let req = ModelCallRequest {
            model: String::new(),
            system: None,
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "hello".into(),
            }],
            max_tokens: None,
            temperature: None,
            role: None,
            caller: None,
            budget: None,
            cache_policy: roko_core::foundation::CachePolicy::Default,
        };
        assert_eq!(svc.resolve_model(&req), "claude-sonnet-4-20250514");
    }

    #[tokio::test]
    async fn explicit_model_resolution() {
        let svc = ModelCallService::new("default".into());
        let req = ModelCallRequest {
            model: "claude-opus-4-20250514".into(),
            system: None,
            messages: vec![],
            max_tokens: None,
            temperature: None,
            role: None,
            caller: None,
            budget: None,
            cache_policy: roko_core::foundation::CachePolicy::Default,
        };
        assert_eq!(svc.resolve_model(&req), "claude-opus-4-20250514");
    }

    #[tokio::test]
    async fn cascade_router_selects_model_when_request_is_empty() {
        let svc = ModelCallService::new("default".into()).with_cascade_router(|role| {
            assert_eq!(role, Some("reviewer"));
            "router-selected-model".to_string()
        });
        let req = ModelCallRequest {
            model: String::new(),
            system: None,
            messages: vec![],
            max_tokens: None,
            temperature: None,
            role: Some("reviewer".to_string()),
            caller: None,
            budget: None,
            cache_policy: roko_core::foundation::CachePolicy::Default,
        };

        assert_eq!(svc.resolve_model(&req), "router-selected-model");
    }

    #[tokio::test]
    async fn routes_claude_model_to_anthropic_api_when_key_set() {
        let svc = ModelCallService::new("default".into()).with_anthropic_api_key("sk-test".into());
        let req = ModelCallRequest {
            model: "claude-haiku-4".into(),
            system: None,
            messages: vec![],
            max_tokens: None,
            temperature: None,
            role: None,
            caller: None,
            budget: None,
            cache_policy: roko_core::foundation::CachePolicy::Default,
        };
        let model = svc.resolve_model(&req);
        let config = svc.config_for_model(&model);

        let provider = config
            .providers
            .get("anthropic")
            .expect("anthropic provider");
        assert_eq!(provider.kind, ProviderKind::AnthropicApi);
        assert_eq!(provider.api_key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
        assert_eq!(provider.timeout_ms, Some(120_000));

        let profile = config.models.get("claude-haiku-4").expect("model profile");
        assert_eq!(profile.provider, "anthropic");
        assert_eq!(profile.slug, "claude-haiku-4");
    }

    #[test]
    fn mcp_config_is_threaded_to_agent_options() {
        let svc = ModelCallService::new("claude".into()).with_mcp_config("/tmp/mcp.json");
        let req = ModelCallRequest {
            model: String::new(),
            system: None,
            messages: vec![],
            max_tokens: None,
            temperature: None,
            role: None,
            caller: None,
            budget: None,
            cache_policy: roko_core::foundation::CachePolicy::Default,
        };

        assert_eq!(svc.resolve_model(&req), "claude");

        let options = svc.build_agent_options(&req, None);
        assert_eq!(options.mcp_config, Some(PathBuf::from("/tmp/mcp.json")));
    }

    #[test]
    #[ignore = "blocked until roko_core::config::AgentConfig exposes mcp_config"]
    fn mcp_config_falls_back_to_roko_config() {
        // TODO(converge): Enable this once the roko-core AgentConfig schema has
        // `mcp_config: Option<PathBuf>`. The S05 write scope only allows edits
        // to this file, so the config-backed fallback cannot be compiled here.
    }

    #[test]
    fn cost_predict_returns_zero_for_unknown_model() {
        let svc = ModelCallService::new("default".into());
        let req = ModelCallRequest {
            model: "unknown-model".into(),
            system: None,
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "hello".into(),
            }],
            max_tokens: None,
            temperature: None,
            role: None,
            caller: None,
            budget: None,
            cache_policy: roko_core::foundation::CachePolicy::Default,
        };

        let estimate = svc.cost_predict(&req);

        assert_eq!(estimate.model, "unknown-model");
        assert_eq!(estimate.predicted_cost_usd, 0.0);
    }

    #[test]
    fn cost_predict_estimates_cost_for_known_model() {
        let mut cost_table = CostTable::default();
        cost_table.insert(
            "known-model",
            ModelPricing {
                input_per_m: 3.0,
                output_per_m: 15.0,
                cache_read_per_m: 0.0,
                cache_write_per_m: 0.0,
            },
        );
        let svc = ModelCallService::new("default".into()).with_cost_table(cost_table);
        let req = ModelCallRequest {
            model: "known-model".into(),
            system: None,
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "x".repeat(1000),
            }],
            max_tokens: Some(2048),
            temperature: None,
            role: None,
            caller: None,
            budget: None,
            cache_policy: roko_core::foundation::CachePolicy::Default,
        };

        let estimate = svc.cost_predict(&req);

        assert_eq!(estimate.model, "known-model");
        assert_eq!(estimate.estimated_input_tokens, 250);
        assert_eq!(estimate.max_output_tokens, 2048);
        assert!(estimate.predicted_cost_usd > 0.0);
    }
}
