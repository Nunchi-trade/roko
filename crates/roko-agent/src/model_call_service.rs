//! ModelCallService -- concrete implementation of `ModelCaller`.
//!
//! Wraps the existing provider dispatch (`create_agent_for_model`) with model
//! resolution, cost tracking, event emission, and feedback recording.

use crate::provider::{AgentOptions, create_agent_for_model};
use crate::task_runner::CostTable;
use async_trait::async_trait;
use roko_core::config::schema::RokoConfig;
use roko_core::foundation::{
    ChatMessage, FeedbackEvent, FeedbackSink, MessageRole, ModelCallRequest, ModelCallResponse,
    ModelCaller, TokenUsage,
};
use roko_core::{
    Body, Context, Engram, EventConsumer, Kind, Result, RokoError, RuntimeEvent, Usage,
};
use std::sync::Arc;
use std::time::Instant;

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

    /// Use a specific run id for emitted events and feedback.
    #[must_use]
    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = run_id.into();
        self
    }

    /// Resolve which model to use for a request.
    fn resolve_model(&self, req: &ModelCallRequest) -> String {
        if req.model.is_empty() {
            // TODO(arch): Route through CascadeRouter once the service can receive
            // a router without making roko-agent depend on roko-learn.
            self.default_model.clone()
        } else {
            req.model.clone()
        }
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

#[async_trait]
impl ModelCaller for ModelCallService {
    async fn call(&self, req: ModelCallRequest) -> Result<ModelCallResponse> {
        let model = self.resolve_model(&req);
        let start = Instant::now();
        let agent_id = format!("model-call:{model}");
        let (message_system, user_content) = request_prompt(&req.messages);
        let system_prompt = req.system.clone().or(message_system);

        let options = AgentOptions {
            system_prompt,
            name: req.role.clone().unwrap_or_else(|| "model_call".to_string()),
            ..AgentOptions::default()
        };
        // TODO(converge): Thread per-request generation settings through
        // AgentOptions/provider adapters. The Anthropic and OpenAI-compatible
        // adapters derive max tokens from ModelProfile::max_output and do not
        // parse "max_tokens=..." or "temperature=..." extra_args.
        // TODO(converge): Thread req-level MCP config here in S05 once
        // ModelCallRequest carries it.

        let agent = create_agent_for_model(&self.config, &model, options).map_err(|err| {
            RokoError::Agent {
                backend: model.clone(),
                message: err.to_string(),
            }
        })?;

        let prompt = Engram::builder(Kind::Prompt)
            .body(Body::text(user_content))
            .build();
        let ctx = Context::now().with_session(self.run_id.clone());
        let result = agent.run(&prompt, &ctx).await;
        let latency_ms = start.elapsed().as_millis() as u64;
        let calculated_cost = self.cost_table.calculate(&model, &result.usage);
        let cost_usd = if calculated_cost > 0.0 {
            calculated_cost
        } else {
            f64::from(result.usage.cost_usd)
        };
        let usage = token_usage(&result.usage, cost_usd);

        if !result.success {
            let error = output_text(&result.output);
            self.emit(RuntimeEvent::AgentFailed {
                run_id: self.run_id.clone(),
                agent_id,
                error: error.clone(),
            });
            self.record_feedback(&req, &model, &usage, latency_ms, false)
                .await?;
            return Err(RokoError::Agent {
                backend: agent.backend_id().to_string(),
                message: error,
            });
        }

        let content = output_text(&result.output);
        self.emit(RuntimeEvent::AgentCompleted {
            run_id: self.run_id.clone(),
            agent_id,
            output: content.clone(),
            tokens_used: usage.total_tokens,
            cost_usd: usage.cost_usd,
        });
        self.record_feedback(&req, &model, &usage, latency_ms, true)
            .await?;

        Ok(ModelCallResponse {
            content,
            model,
            usage,
            stop_reason: Some("end_turn".to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        assert_eq!(svc.resolve_model(&req), "claude-opus-4-20250514");
    }
}
