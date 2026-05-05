//! ModelCallService -- concrete implementation of `ModelCaller`.
//!
//! Wraps the existing provider dispatch (`create_agent_for_model`) with model
//! resolution, cost tracking, event emission, and feedback recording.

use crate::gateway_events::{GatewayEvent, GatewayEventWriter};
use crate::observer::InferenceObserver;
use crate::provider::{AgentOptions, create_agent_for_model};
use crate::task_runner::CostTable;
use async_trait::async_trait;
use chrono::Utc;
use roko_core::config::schema::RokoConfig;
use roko_core::foundation::{
    CachePolicy, ChatMessage, FeedbackEvent, FeedbackSink, GatewayError, MessageRole,
    ModelCallRequest, ModelCallResponse, ModelCaller, TokenBudget, TokenUsage,
};
use roko_core::{
    Body, Context, Signal, EventConsumer, Kind, Result, RokoError, RuntimeEvent, ToolCallSummary,
    Usage,
};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use parking_lot::{Mutex, RwLock};

type ModelRouter = dyn Fn(Option<&str>) -> String + Send + Sync;
type KnowledgeStoreQuery = dyn Fn(&str, usize) -> Result<Vec<serde_json::Value>> + Send + Sync;

/// Records explicit model override outcomes when no routing context is available.
pub trait ForceBackendOverrideRecorder: Send + Sync {
    /// Record a confidence-only outcome for a forced model slug.
    fn record_override_outcome(&self, model_slug: &str, success: bool) -> bool;
}

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
    /// Optional observer for backend inference lifecycle events.
    inference_observer: Option<Arc<dyn InferenceObserver>>,
    /// Optional sink for model-call feedback.
    feedback_sink: Option<Arc<dyn FeedbackSink>>,
    /// Optional durable gateway event writer.
    gateway_event_writer: Option<Arc<GatewayEventWriter>>,
    /// Optional knowledge store query adapter for knowledge-informed routing.
    ///
    /// TODO(converge): Replace this erased adapter with
    /// `Arc<dyn roko_neuro::NeuroStore + Send + Sync>` once `roko-agent` has a
    /// normal `roko-neuro` dependency and `NeuroStore` is object-safe. In this
    /// worktree `NeuroStore: Sized`, and this batch's scope forbids Cargo.toml
    /// changes, so direct trait-object storage cannot compile here.
    knowledge_store: Option<Arc<KnowledgeStoreQuery>>,
    /// Optional model router used when requests omit an explicit model.
    model_router: Option<Arc<ModelRouter>>,
    /// Optional cascade router callback for recording forced model observations.
    ///
    /// This is trait-typed to avoid adding a production `roko-agent` ->
    /// `roko-learn` dependency edge; `CascadeRouter` implements the trait in
    /// `roko-learn`.
    cascade_router: Option<Arc<dyn ForceBackendOverrideRecorder>>,
    /// Ordered fallback model slugs derived from workspace model config.
    fallback_models: Vec<String>,
    /// Service-scoped environment entries passed into provider construction.
    env: Vec<(String, String)>,
    /// Explicit MCP config path threaded into provider options.
    mcp_config: Option<PathBuf>,
    /// L1 exact-match response cache.
    cache: CacheCell,
    /// Service-lifetime cost budget tracker.
    budget: BudgetCell,
    /// Caps reasoning-token budgets for thinking-capable models.
    thinking_cap: ThinkingCapCell,
    /// Detects repeated near-identical agent outputs for a run/role.
    convergence: ConvergenceDetectionCell,
    /// Run id used for emitted events and feedback when the request has none.
    run_id: String,
    /// Per-service sequence for gateway request ids.
    request_seq: AtomicU64,
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
            inference_observer: None,
            feedback_sink: None,
            gateway_event_writer: None,
            knowledge_store: None,
            model_router: None,
            cascade_router: None,
            fallback_models: Vec::new(),
            env: Vec::new(),
            mcp_config: None,
            cache: CacheCell::new(128),
            budget: BudgetCell::new(None),
            thinking_cap: ThinkingCapCell::new(16_384),
            convergence: ConvergenceDetectionCell::new(5, 0.85, 3),
            run_id: "model-call-service".to_string(),
            request_seq: AtomicU64::new(1),
        }
    }

    /// Use an explicit Roko configuration for provider dispatch.
    #[must_use]
    pub fn with_config(mut self, config: RokoConfig) -> Self {
        self.fallback_models = configured_fallback_models(&config, &self.default_model);
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

    /// Attach an inference observer for backend call lifecycle events.
    #[must_use]
    pub fn with_inference_observer(mut self, observer: Arc<dyn InferenceObserver>) -> Self {
        self.inference_observer = Some(observer);
        self
    }

    /// Attach a feedback sink.
    #[must_use]
    pub fn with_feedback_sink(mut self, feedback_sink: Arc<dyn FeedbackSink>) -> Self {
        self.feedback_sink = Some(feedback_sink);
        self
    }

    /// Attach a durable gateway event writer.
    #[must_use]
    pub fn with_gateway_event_writer(mut self, writer: Arc<GatewayEventWriter>) -> Self {
        self.gateway_event_writer = Some(writer);
        self
    }

    /// Attach a knowledge store query adapter for knowledge-informed model routing.
    ///
    /// The adapter should return serialized neuro `KnowledgeEntry` values.
    #[must_use]
    #[allow(clippy::type_complexity)]
    pub fn with_knowledge_store(
        mut self,
        store: Arc<dyn Fn(&str, usize) -> Result<Vec<serde_json::Value>> + Send + Sync>,
    ) -> Self {
        self.knowledge_store = Some(store);
        self
    }

    /// Attach a model-selection callback used when requests omit an explicit model.
    #[must_use]
    pub fn with_model_router<F>(mut self, router_fn: F) -> Self
    where
        F: Fn(Option<&str>) -> String + Send + Sync + 'static,
    {
        self.model_router = Some(Arc::new(router_fn));
        self
    }

    /// Attach a CascadeRouter for force_backend learning (UX34).
    #[must_use]
    pub fn with_cascade_router<R>(mut self, router: Arc<R>) -> Self
    where
        R: ForceBackendOverrideRecorder + 'static,
    {
        self.cascade_router = Some(router);
        self
    }

    /// Provide an Anthropic API key for service-created agents.
    #[must_use]
    pub fn with_anthropic_api_key(mut self, key: String) -> Self {
        self.set_env("ANTHROPIC_API_KEY", key);
        self
    }

    /// Deprecated. Configure OpenAI-compatible providers in `RokoConfig` instead.
    #[must_use]
    #[deprecated(note = "configure OpenAI-compatible providers in RokoConfig")]
    pub fn with_openai_base_url(self, _url: String) -> Self {
        self
    }

    /// Use an explicit MCP config path for service-created agents.
    #[must_use]
    pub fn with_mcp_config(mut self, path: impl Into<PathBuf>) -> Self {
        self.mcp_config = Some(path.into());
        self
    }

    /// Set the maximum cache entries.
    #[must_use]
    pub fn with_cache_size(mut self, max_entries: usize) -> Self {
        self.cache = CacheCell::new(max_entries);
        self
    }

    /// Set a cumulative cost budget for the lifetime of this service.
    #[must_use]
    pub fn with_cost_budget(mut self, max_cost_usd: f64) -> Self {
        self.budget = BudgetCell::new(Some(max_cost_usd));
        self
    }

    /// Set the default thinking/reasoning token cap for capable models.
    #[must_use]
    pub fn with_thinking_budget(mut self, max_thinking_tokens: u32) -> Self {
        self.thinking_cap = ThinkingCapCell::new(max_thinking_tokens);
        self
    }

    /// Configure output convergence detection.
    #[must_use]
    pub fn with_convergence_detection(
        mut self,
        window_size: usize,
        similarity_threshold: f64,
        consecutive_trigger: usize,
    ) -> Self {
        self.convergence =
            ConvergenceDetectionCell::new(window_size, similarity_threshold, consecutive_trigger);
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
        let max_output_tokens = u64::from(
            req.max_tokens
                .unwrap_or(roko_core::defaults::DEFAULT_FALLBACK_MAX_OUTPUT_TOKENS),
        );
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

    fn config_for_model(&self, _model: &str) -> RokoConfig {
        self.config.clone()
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
            effort: Some(self.config.agent.default_effort.clone())
                .filter(|effort| !effort.trim().is_empty()),
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

    fn inference_started(&self, request_id: &str, model: &str, agent_id: &str, auto_routed: bool) {
        if let Some(observer) = &self.inference_observer {
            observer.on_start(&self.run_id, request_id, model, agent_id, auto_routed);
        }
    }

    fn inference_completed(
        &self,
        request_id: &str,
        model: &str,
        agent_id: &str,
        usage: &TokenUsage,
        duration_ms: u64,
    ) {
        if let Some(observer) = &self.inference_observer {
            observer.on_complete(
                &self.run_id,
                request_id,
                model,
                agent_id,
                usage.input_tokens,
                usage.output_tokens,
                usage.cost_usd,
                duration_ms,
            );
        }
    }

    fn inference_failed(&self, request_id: &str, model: &str, agent_id: &str, error: &str) {
        if let Some(observer) = &self.inference_observer {
            observer.on_error(&self.run_id, request_id, model, agent_id, error);
        }
    }

    fn emit_agent_trace_events(
        &self,
        agent_id: &str,
        traces: &[AgentTracePayload],
        fallback_usage: &TokenUsage,
    ) {
        if traces.is_empty() {
            self.emit(RuntimeEvent::AgentTrace {
                run_id: self.run_id.clone(),
                agent_id: agent_id.to_string(),
                turn: 1,
                tool_calls: Vec::new(),
                reasoning: None,
                usage: fallback_usage.clone(),
            });
            return;
        }

        for trace in traces {
            self.emit(RuntimeEvent::AgentTrace {
                run_id: self.run_id.clone(),
                agent_id: agent_id.to_string(),
                turn: trace.turn,
                tool_calls: trace.tool_calls.clone(),
                reasoning: trace.reasoning.clone(),
                usage: trace.usage.clone(),
            });
        }
    }

    async fn record_feedback(
        &self,
        req: &ModelCallRequest,
        request_id: &str,
        model: &str,
        provider: Option<&str>,
        usage: &TokenUsage,
        latency_ms: u64,
        success: bool,
    ) -> Result<()> {
        let Some(sink) = &self.feedback_sink else {
            tracing::debug!("feedback sink not configured for model call service; skipping");
            return Ok(());
        };

        sink.record(FeedbackEvent::ModelCall {
            run_id: req.run_id.clone().or_else(|| Some(self.run_id.clone())),
            request_id: Some(request_id.to_string()),
            prompt_section_ids: req.prompt_section_ids.clone(),
            knowledge_ids: req.knowledge_ids.clone(),
            model: Some(model.to_string()),
            provider: provider.map(ToOwned::to_owned),
            token_usage: Some(usage.total_tokens),
            cost: Some(usage.cost_usd),
            role: req.role.clone().unwrap_or_else(|| "model_call".to_string()),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cost_usd: usage.cost_usd,
            latency_ms,
            success,
        })
        .await
    }

    fn next_request_id(&self, cache_key: u64) -> String {
        let seq = self.request_seq.fetch_add(1, Ordering::Relaxed);
        format!("{}:{seq}:{cache_key:016x}", self.run_id)
    }

    fn provider_for_model(&self, model: &str) -> Option<String> {
        let models = self.config.effective_models();
        models
            .get(model)
            .or_else(|| models.values().find(|profile| profile.slug == model))
            .map(|profile| profile.provider.clone())
            .filter(|provider| !provider.trim().is_empty())
    }

    fn write_gateway_event(
        &self,
        req: &ModelCallRequest,
        request_id: &str,
        model: &str,
        usage: &TokenUsage,
        latency_ms: u64,
        cache_hit: bool,
        error: Option<String>,
    ) -> Result<()> {
        let Some(writer) = &self.gateway_event_writer else {
            return Ok(());
        };

        let caller = req
            .caller
            .clone()
            .or_else(|| req.role.clone())
            .unwrap_or_else(|| "model_call".to_string());
        let provider = self.provider_for_model(model);
        let success = error.is_none();
        writer
            .write(&GatewayEvent {
                request_id: request_id.to_string(),
                caller,
                model: model.to_string(),
                provider,
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cost_usd: usage.cost_usd,
                latency_ms,
                cache_hit,
                success,
                error,
                timestamp: Utc::now().to_rfc3339(),
            })
            .map_err(|err| RokoError::invalid(format!("write gateway event: {err}")))
    }

    fn record_force_backend_override(
        &self,
        requested_model: &str,
        model_used: &str,
        success: bool,
    ) {
        if requested_model.is_empty() {
            return;
        }

        let Some(router) = &self.cascade_router else {
            return;
        };

        if !router.record_override_outcome(model_used, success) {
            tracing::debug!(
                requested_model,
                model_used,
                success,
                "force_backend override outcome was not accepted by cascade router"
            );
        }
    }

    fn fallback_models_for_request(&self, model: &str) -> Vec<String> {
        self.fallback_models
            .iter()
            .filter(|fallback| fallback.as_str() != model)
            .cloned()
            .collect()
    }

    fn build_knowledge_advice(
        &self,
        candidate_slugs: &[String],
        role: Option<&str>,
        task_hint: Option<&str>,
    ) -> Option<KnowledgeRoutingAdvice> {
        let store = self.knowledge_store.as_ref()?;

        let query = format!(
            "{} {} routing model",
            role.unwrap_or("default"),
            task_hint.unwrap_or("general")
        );

        let entries = match store(&query, 10) {
            Ok(entries) => entries,
            Err(err) => {
                tracing::debug!(error = %err, "knowledge store query failed for routing");
                return None;
            }
        };

        if entries.is_empty() {
            return None;
        }

        let mut hints = Vec::new();
        let entry_ids = entries
            .iter()
            .filter_map(knowledge_id)
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        let prompt_facts = entries
            .iter()
            .filter(|entry| knowledge_confidence(entry) >= 0.5)
            .filter_map(knowledge_prompt_fact)
            .take(3)
            .collect::<Vec<_>>();

        for slug in candidate_slugs {
            let slug_lower = slug.to_lowercase();
            let mut score = 0.0_f64;
            let mut supporting = 0_u32;

            for entry in &entries {
                let content_matches = knowledge_content(entry)
                    .to_lowercase()
                    .contains(&slug_lower)
                    || knowledge_source_model(entry)
                        .is_some_and(|sm| sm.eq_ignore_ascii_case(slug))
                    || knowledge_tags(entry)
                        .iter()
                        .any(|tag| tag.eq_ignore_ascii_case(slug));
                if !content_matches {
                    continue;
                }
                supporting += 1;
                let weight = knowledge_confidence(entry).clamp(0.0, 1.0);
                if knowledge_is_anti(entry) {
                    score -= weight * 0.15;
                } else {
                    score += weight * 0.10;
                }
            }

            if supporting > 0 {
                hints.push(KnowledgeHint {
                    model_slug: slug.clone(),
                    score: score.clamp(-0.3, 0.3),
                    supporting_entries: supporting,
                    reason: format!("{supporting} knowledge entries for {slug}"),
                });
            }
        }

        let has_signal = !hints.is_empty();
        Some(KnowledgeRoutingAdvice {
            hints,
            entry_ids,
            prompt_facts,
            has_signal,
        })
    }
}

#[derive(Debug, Clone)]
struct KnowledgeHint {
    model_slug: String,
    score: f64,
    supporting_entries: u32,
    reason: String,
}

#[derive(Debug, Clone)]
struct KnowledgeRoutingAdvice {
    hints: Vec<KnowledgeHint>,
    entry_ids: Vec<String>,
    prompt_facts: Vec<String>,
    has_signal: bool,
}

fn apply_knowledge_advice(req: &mut ModelCallRequest, advice: &KnowledgeRoutingAdvice) {
    for id in &advice.entry_ids {
        if !id.trim().is_empty() && !req.knowledge_ids.contains(id) {
            req.knowledge_ids.push(id.clone());
        }
    }
    for hint in &advice.hints {
        let routing_hint = format!(
            "knowledge:{}:{:.3}:{}",
            hint.model_slug, hint.score, hint.supporting_entries
        );
        if !req.routing_hints.contains(&routing_hint) {
            req.routing_hints.push(routing_hint);
        }
    }
}

fn append_knowledge_to_system_prompt(
    system_prompt: Option<String>,
    advice: Option<&KnowledgeRoutingAdvice>,
) -> Option<String> {
    let Some(advice) = advice else {
        return system_prompt;
    };
    if advice.prompt_facts.is_empty() {
        return system_prompt;
    }

    let knowledge = format!(
        "## Relevant Knowledge\n\n{}",
        advice
            .prompt_facts
            .iter()
            .map(|fact| format!("- {fact}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    Some(match system_prompt {
        Some(existing) if !existing.trim().is_empty() => format!("{existing}\n\n{knowledge}"),
        _ => knowledge,
    })
}

fn request_prompt(messages: &[ChatMessage]) -> (Option<String>, String) {
    let mut system_prompt: Option<String> = None;
    let mut user_content = String::new();
    let conversational_turns = messages
        .iter()
        .filter(|msg| !matches!(msg.role, MessageRole::System))
        .count();
    let single_plain_user_turn = conversational_turns == 1
        && messages
            .iter()
            .any(|msg| matches!(msg.role, MessageRole::User));

    for msg in messages {
        match msg.role {
            MessageRole::System => {
                system_prompt = Some(match system_prompt {
                    Some(existing) if !existing.trim().is_empty() => {
                        format!("{existing}\n\n{}", msg.content)
                    }
                    _ => msg.content.clone(),
                });
            }
            MessageRole::User => {
                if !user_content.is_empty() {
                    user_content.push_str("\n\n");
                }
                if single_plain_user_turn {
                    user_content.push_str(&msg.content);
                } else {
                    user_content.push_str("User:\n");
                    user_content.push_str(&msg.content);
                }
            }
            MessageRole::Assistant => {
                if !user_content.is_empty() {
                    user_content.push_str("\n\n");
                }
                user_content.push_str("Assistant:\n");
                user_content.push_str(&msg.content);
            }
        }
    }

    (system_prompt, user_content)
}

fn request_task_hint(messages: &[ChatMessage]) -> Option<&str> {
    messages
        .iter()
        .find(|message| matches!(message.role, MessageRole::User) && !message.content.is_empty())
        .map(|message| message.content.as_str())
}

fn knowledge_content(entry: &serde_json::Value) -> &str {
    entry
        .get("content")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

fn knowledge_id(entry: &serde_json::Value) -> Option<&str> {
    entry.get("id").and_then(serde_json::Value::as_str)
}

fn knowledge_prompt_fact(entry: &serde_json::Value) -> Option<String> {
    let content = knowledge_content(entry).trim();
    if content.is_empty() {
        return None;
    }
    let kind = entry
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Knowledge");
    Some(format!("{kind}: {}", truncate_knowledge_fact(content, 240)))
}

fn truncate_knowledge_fact(value: &str, max_chars: usize) -> &str {
    value
        .char_indices()
        .nth(max_chars)
        .map_or(value, |(index, _)| &value[..index])
}

fn knowledge_source_model(entry: &serde_json::Value) -> Option<&str> {
    entry
        .get("source_model")
        .and_then(serde_json::Value::as_str)
}

fn knowledge_tags(entry: &serde_json::Value) -> Vec<&str> {
    entry
        .get("tags")
        .and_then(serde_json::Value::as_array)
        .map(|tags| {
            tags.iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn knowledge_confidence(entry: &serde_json::Value) -> f64 {
    entry
        .get("confidence")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or_default()
}

fn knowledge_is_anti(entry: &serde_json::Value) -> bool {
    entry
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| {
            kind.eq_ignore_ascii_case("AntiKnowledge")
                || kind.eq_ignore_ascii_case("anti_knowledge")
                || kind.eq_ignore_ascii_case("anti-knowledge")
        })
}

fn output_text(output: &Signal) -> String {
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

#[derive(Debug, Clone)]
struct AgentTracePayload {
    turn: u32,
    tool_calls: Vec<ToolCallSummary>,
    reasoning: Option<String>,
    usage: TokenUsage,
}

fn agent_trace_payloads(result: &crate::agent::AgentResult) -> Vec<AgentTracePayload> {
    result
        .trace
        .iter()
        .filter_map(agent_trace_payload)
        .collect()
}

fn agent_trace_payload(signal: &Signal) -> Option<AgentTracePayload> {
    if signal.kind.as_str() != "agent.trace" {
        return None;
    }

    let Body::Json(value) = &signal.body else {
        return None;
    };

    let turn = value
        .get("turn")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1)
        .min(u64::from(u32::MAX)) as u32;

    Some(AgentTracePayload {
        turn,
        tool_calls: agent_trace_tool_calls(value),
        reasoning: value
            .get("reasoning")
            .and_then(serde_json::Value::as_str)
            .filter(|reasoning| !reasoning.is_empty())
            .map(ToString::to_string),
        usage: agent_trace_usage(value),
    })
}

fn agent_trace_tool_calls(value: &serde_json::Value) -> Vec<ToolCallSummary> {
    value
        .get("tool_calls")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let name = item.get("name").and_then(serde_json::Value::as_str)?;
            let result_preview = item
                .get("result_preview")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            Some(ToolCallSummary {
                name: name.to_string(),
                result_preview: result_preview.to_string(),
            })
        })
        .collect()
}

fn agent_trace_usage(value: &serde_json::Value) -> TokenUsage {
    let usage = value.get("usage");
    let input_tokens = usage
        .and_then(|usage| usage.get("input_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();
    let output_tokens = usage
        .and_then(|usage| usage.get("output_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();
    let total_tokens = usage
        .and_then(|usage| usage.get("total_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_else(|| input_tokens.saturating_add(output_tokens));
    let cost_usd = usage
        .and_then(|usage| usage.get("cost_usd"))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or_default();

    TokenUsage {
        input_tokens,
        output_tokens,
        total_tokens,
        cost_usd,
    }
}

fn configured_fallback_models(config: &RokoConfig, default_model: &str) -> Vec<String> {
    let mut fallbacks = Vec::new();
    if let Some(fallback) = config.agent.fallback_model.as_deref() {
        push_unique_model_slug(config, &mut fallbacks, fallback);
    }

    let mut tier_models = config.agent.tier_models.values().collect::<Vec<_>>();
    tier_models.sort();
    for tier_model in tier_models {
        push_unique_model_slug(config, &mut fallbacks, tier_model);
    }

    let mut model_slugs = config
        .effective_models()
        .into_values()
        .map(|profile| profile.slug)
        .collect::<Vec<_>>();
    model_slugs.sort();
    for slug in model_slugs {
        push_unique_model_slug(config, &mut fallbacks, &slug);
    }

    fallbacks.retain(|fallback| fallback != default_model);
    fallbacks
}

fn push_unique_model_slug(config: &RokoConfig, fallbacks: &mut Vec<String>, model_key: &str) {
    let slug = roko_core::agent::resolve_model(config, model_key).slug;
    if !slug.trim().is_empty() && !fallbacks.contains(&slug) {
        fallbacks.push(slug);
    }
}

/// L1 in-memory response cache. Keyed by (model, messages, temperature).
///
/// Uses a single `RwLock` over the combined entries + LRU order to eliminate
/// the dual-mutex deadlock risk from the previous design (§15.2).
struct CacheCell {
    inner: RwLock<CacheCellInner>,
}

struct CacheCellInner {
    max_entries: usize,
    entries: HashMap<u64, CachedResponse>,
    /// Insertion-order keys for LRU eviction (oldest at front).
    order: Vec<u64>,
}

#[derive(Debug, Clone)]
struct CachedResponse {
    content: String,
    model: String,
    usage: TokenUsage,
    stop_reason: Option<String>,
}

impl CacheCell {
    fn new(max_entries: usize) -> Self {
        Self {
            inner: RwLock::new(CacheCellInner {
                max_entries,
                entries: HashMap::new(),
                order: Vec::new(),
            }),
        }
    }

    /// Compute a cache key from request fields.
    /// Hash of: model + system prompt + ordered messages + relevant generation parameters.
    fn cache_key(
        model: &str,
        system: Option<&str>,
        messages: &[ChatMessage],
        temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        model.hash(&mut hasher);
        system.hash(&mut hasher);
        messages.len().hash(&mut hasher);

        for message in messages {
            message_role_tag(&message.role).hash(&mut hasher);
            message.content.hash(&mut hasher);
        }

        temperature.map(f32::to_bits).hash(&mut hasher);
        max_tokens.hash(&mut hasher);
        hasher.finish()
    }

    /// Look up a cached response. Returns None on miss.
    /// On hit, promotes the key to most-recently-used.
    fn lookup(&self, key: u64) -> Option<CachedResponse> {
        // Fast path: read lock for miss check.
        {
            let inner = self.inner.read();
            if !inner.entries.contains_key(&key) {
                return None;
            }
        }

        // Slow path: write lock to clone + promote.
        let mut inner = self.inner.write();
        let response = inner.entries.get(&key).cloned();
        if response.is_some() {
            if let Some(index) = inner.order.iter().position(|existing| *existing == key) {
                inner.order.remove(index);
            }
            inner.order.push(key);
        }
        response
    }

    /// Store a successful response. Evicts the oldest entry if at capacity.
    fn store(&self, key: u64, response: CachedResponse) {
        let mut inner = self.inner.write();
        if inner.max_entries == 0 {
            return;
        }

        if inner.entries.contains_key(&key) {
            inner.order.retain(|existing| *existing != key);
        } else if inner.entries.len() >= inner.max_entries {
            if let Some(oldest) = inner.order.first().copied() {
                inner.entries.remove(&oldest);
                inner.order.remove(0);
            }
        }

        inner.entries.insert(key, response);
        inner.order.push(key);
    }

    fn evict(&self, key: u64) {
        let mut inner = self.inner.write();
        inner.entries.remove(&key);
        inner.order.retain(|existing| *existing != key);
    }
}

fn message_role_tag(role: &MessageRole) -> u8 {
    match role {
        MessageRole::System => 0,
        MessageRole::User => 1,
        MessageRole::Assistant => 2,
    }
}

/// Tracks process-local, best-effort cumulative cost across calls and enforces per-call budgets.
/// This is not durable, reserved, or cross-run budget accounting.
struct BudgetCell {
    /// Cumulative cost in micro-USD (1e-6 USD) for atomic tracking.
    cumulative_cost_micro_usd: AtomicU64,
    /// Maximum cumulative cost in micro-USD, if set.
    max_cumulative_cost_micro_usd: Option<u64>,
}

impl BudgetCell {
    fn new(max_cumulative_cost_usd: Option<f64>) -> Self {
        Self {
            cumulative_cost_micro_usd: AtomicU64::new(0),
            max_cumulative_cost_micro_usd: max_cumulative_cost_usd.map(usd_to_micro_usd),
        }
    }

    /// Check whether a request's budget allows it to proceed.
    /// Returns Err(GatewayError::BudgetExceeded) if the cumulative cost has been exceeded
    /// or if the request's own TokenBudget.max_cost_usd would be exceeded.
    fn check(&self, budget: &Option<TokenBudget>) -> std::result::Result<(), GatewayError> {
        let cumulative = self.cumulative_cost_micro_usd.load(Ordering::Relaxed);

        if let Some(limit) = self.max_cumulative_cost_micro_usd {
            if cumulative >= limit {
                return Err(GatewayError::BudgetExceeded {
                    detail: format!(
                        "cumulative cost {:.6} USD reached configured limit {:.6} USD",
                        micro_usd_to_usd(cumulative),
                        micro_usd_to_usd(limit)
                    ),
                });
            }
        }

        if let Some(budget) = budget {
            if let Some(max_cost_usd) = budget.max_cost_usd {
                if max_cost_usd <= 0.0 {
                    return Err(GatewayError::BudgetExceeded {
                        detail: format!(
                            "per-call max_cost_usd must be positive, got {max_cost_usd:.6}"
                        ),
                    });
                }
            }
        }

        Ok(())
    }

    /// Record the cost of a completed call. Adds to cumulative total.
    fn record_cost(&self, cost_usd: f64) {
        self.cumulative_cost_micro_usd
            .fetch_add(usd_to_micro_usd(cost_usd), Ordering::Relaxed);
    }

    /// Current cumulative cost in USD.
    fn cumulative_cost_usd(&self) -> f64 {
        micro_usd_to_usd(self.cumulative_cost_micro_usd.load(Ordering::Relaxed))
    }
}

fn usd_to_micro_usd(cost_usd: f64) -> u64 {
    if !cost_usd.is_finite() || cost_usd <= 0.0 {
        return 0;
    }

    (cost_usd * 1_000_000.0).round() as u64
}

fn micro_usd_to_usd(cost_micro_usd: u64) -> f64 {
    cost_micro_usd as f64 / 1_000_000.0
}

/// Caps thinking/reasoning tokens for models that support extended thinking.
struct ThinkingCapCell {
    /// Default maximum thinking tokens when thinking is enabled.
    default_thinking_budget: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ThinkingCapResult {
    /// Effective thinking budget (None = thinking not applicable).
    thinking_budget: Option<u32>,
    /// Whether the cap was applied (original was higher or unset).
    was_capped: bool,
    /// Original value before capping.
    original: Option<u32>,
}

impl ThinkingCapCell {
    fn new(default_thinking_budget: u32) -> Self {
        Self {
            default_thinking_budget,
        }
    }

    /// Apply thinking cap to a request, returning the effective thinking budget.
    fn apply(&self, model: &str, max_tokens: Option<u32>) -> ThinkingCapResult {
        if !supports_thinking(model) {
            return ThinkingCapResult {
                thinking_budget: None,
                was_capped: false,
                original: max_tokens,
            };
        }

        match max_tokens {
            Some(tokens) if tokens < self.default_thinking_budget => ThinkingCapResult {
                thinking_budget: Some(tokens),
                was_capped: false,
                original: max_tokens,
            },
            Some(tokens) if tokens == self.default_thinking_budget => ThinkingCapResult {
                thinking_budget: Some(tokens),
                was_capped: false,
                original: max_tokens,
            },
            _ => ThinkingCapResult {
                thinking_budget: Some(self.default_thinking_budget),
                was_capped: true,
                original: max_tokens,
            },
        }
    }
}

fn supports_thinking(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    lower.contains("opus")
        || lower.contains("o1")
        || lower.contains("o3")
        || lower.contains("o4")
        || lower.contains("deepseek-r1")
}

/// Detects when an agent produces near-identical outputs repeatedly.
struct ConvergenceDetectionCell {
    /// Maximum number of recent outputs to track per key.
    window_size: usize,
    /// Similarity threshold (0.0-1.0). Above this = "identical enough".
    similarity_threshold: f64,
    /// Number of consecutive similar outputs before triggering.
    consecutive_trigger: usize,
    /// Recent outputs keyed by (run_id, role).
    history: Mutex<HashMap<String, Vec<String>>>,
}

impl ConvergenceDetectionCell {
    fn new(window_size: usize, similarity_threshold: f64, consecutive_trigger: usize) -> Self {
        Self {
            window_size,
            similarity_threshold,
            consecutive_trigger,
            history: Mutex::new(HashMap::new()),
        }
    }

    /// Check a new output against the history for this key.
    fn check(&self, key: &str, new_output: &str) -> std::result::Result<(), GatewayError> {
        if self.consecutive_trigger <= 1 {
            return Err(GatewayError::ConvergenceDetected { consecutive: 1 });
        }

        let previous = {
            let history = self.history.lock();
            history.get(key).cloned().unwrap_or_default()
        };

        if previous.len() + 1 < self.consecutive_trigger {
            return Ok(());
        }

        let start = previous
            .len()
            .saturating_add(1)
            .saturating_sub(self.consecutive_trigger);
        let mut recent = previous.into_iter().skip(start).collect::<Vec<String>>();
        recent.push(new_output.to_string());

        let converged = recent
            .windows(2)
            .all(|pair| similarity(&pair[0], &pair[1]) >= self.similarity_threshold);

        if converged {
            Err(GatewayError::ConvergenceDetected {
                consecutive: self.consecutive_trigger as u32,
            })
        } else {
            Ok(())
        }
    }

    /// Record an output in the history for future checks.
    fn record(&self, key: &str, output: String) {
        if self.window_size == 0 {
            return;
        }

        let mut history = self.history.lock();
        let outputs = history.entry(key.to_string()).or_default();
        outputs.push(output);
        if outputs.len() > self.window_size {
            let drain_count = outputs.len() - self.window_size;
            outputs.drain(0..drain_count);
        }
    }
}

fn set_max_tokens_option(options: &mut AgentOptions, max_tokens: u32) {
    options
        .extra_args
        .retain(|arg| !arg.starts_with("max_tokens="));
    options.extra_args.push(format!("max_tokens={max_tokens}"));
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a = a.chars().take(500).collect::<Vec<_>>();
    let b = b.chars().take(500).collect::<Vec<_>>();

    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }

    let mut previous = (0..=b.len()).collect::<Vec<_>>();
    let mut current = vec![0; b.len() + 1];

    for (i, left) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, right) in b.iter().enumerate() {
            let substitution = usize::from(left != right);
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[b.len()]
}

fn similarity(a: &str, b: &str) -> f64 {
    let a = a.chars().take(500).collect::<String>();
    let b = b.chars().take(500).collect::<String>();

    if a.is_empty() && b.is_empty() {
        return 1.0;
    }

    let max_len = a.chars().count().max(b.chars().count());
    if max_len == 0 {
        return 1.0;
    }

    let distance = edit_distance(&a, &b);
    1.0 - (distance as f64 / max_len as f64)
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
        let prompt = Signal::builder(Kind::Prompt)
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
            let agent_traces = agent_trace_payloads(&result);

            if result.success {
                return Ok(CellOutput {
                    content: output_text(&result.output),
                    model_used: attempt_model.to_string(),
                    usage: result.usage,
                    cost_usd,
                    latency_ms: (total_start.elapsed().as_millis() as u64).max(1),
                    fallback_used: attempt_index > 0,
                    agent_traces,
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
    agent_traces: Vec<AgentTracePayload>,
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
    async fn call(&self, mut req: ModelCallRequest) -> Result<ModelCallResponse> {
        let mut knowledge_candidates = Vec::new();
        if !req.model.is_empty() {
            knowledge_candidates.push(req.model.clone());
        }
        if !self.default_model.is_empty() && !knowledge_candidates.contains(&self.default_model) {
            knowledge_candidates.push(self.default_model.clone());
        }
        for fallback in &self.fallback_models {
            if !knowledge_candidates.contains(fallback) {
                knowledge_candidates.push(fallback.clone());
            }
        }
        let knowledge_advice = self.build_knowledge_advice(
            &knowledge_candidates,
            req.role.as_deref(),
            request_task_hint(&req.messages),
        );
        if let Some(advice) = &knowledge_advice {
            apply_knowledge_advice(&mut req, advice);
        }
        if self.knowledge_store.is_some() {
            if let Some(advice) = &knowledge_advice {
                tracing::debug!(
                    has_signal = advice.has_signal,
                    hints = advice.hints.len(),
                    candidates = ?knowledge_candidates,
                    "knowledge routing advice produced"
                );
            } else {
                tracing::debug!(
                    candidates = ?knowledge_candidates,
                    "knowledge routing advice unavailable"
                );
            }
        }

        let model = self.resolve_model(&req);
        let start = Instant::now();
        let agent_id = format!("model-call:{model}");
        let auto_routed = req.model.trim().is_empty() || req.model != model;
        let cache_key = CacheCell::cache_key(
            &model,
            req.system.as_deref(),
            &req.messages,
            req.temperature,
            req.max_tokens,
        );
        let request_id = self.next_request_id(cache_key);
        let provider = self.provider_for_model(&model);

        match req.cache_policy {
            CachePolicy::Default => {
                if let Some(cached) = self.cache.lookup(cache_key) {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    let cached_provider = self.provider_for_model(&cached.model);
                    self.write_gateway_event(
                        &req,
                        &request_id,
                        &cached.model,
                        &cached.usage,
                        latency_ms,
                        true,
                        None,
                    )?;
                    self.record_feedback(
                        &req,
                        &request_id,
                        &cached.model,
                        cached_provider.as_deref(),
                        &cached.usage,
                        latency_ms,
                        true,
                    )
                    .await?;
                    return Ok(ModelCallResponse {
                        content: cached.content,
                        model: cached.model,
                        usage: cached.usage,
                        stop_reason: cached.stop_reason,
                        request_id: Some(request_id),
                    });
                }
            }
            CachePolicy::Bypass => {}
            CachePolicy::ForceRefresh => self.cache.evict(cache_key),
        }

        self.budget.check(&req.budget).map_err(RokoError::from)?;
        if let Some(budget) = &req.budget {
            let estimate = self.cost_predict(&req);
            if let Some(max_input) = budget.max_input {
                if estimate.estimated_input_tokens > max_input {
                    return Err(GatewayError::BudgetExceeded {
                        detail: format!(
                            "estimated input tokens {} exceed per-call limit {}",
                            estimate.estimated_input_tokens, max_input
                        ),
                    }
                    .into());
                }
            }
            if let Some(max_output) = budget.max_output {
                if estimate.max_output_tokens > max_output {
                    return Err(GatewayError::BudgetExceeded {
                        detail: format!(
                            "requested output tokens {} exceed per-call limit {}",
                            estimate.max_output_tokens, max_output
                        ),
                    }
                    .into());
                }
            }
            if let Some(max_cost_usd) = budget.max_cost_usd {
                if estimate.predicted_cost_usd > max_cost_usd {
                    return Err(GatewayError::BudgetExceeded {
                        detail: format!(
                            "predicted cost {:.6} USD exceeds per-call limit {:.6} USD",
                            estimate.predicted_cost_usd, max_cost_usd
                        ),
                    }
                    .into());
                }
            }
        }

        let (message_system, user_content) = request_prompt(&req.messages);
        let system_prompt = append_knowledge_to_system_prompt(
            req.system.clone().or(message_system),
            knowledge_advice.as_ref(),
        );
        let config = self.config_for_model(&model);

        let mut options = self.build_agent_options(&req, None);
        let thinking_cap = self.thinking_cap.apply(&model, req.max_tokens);
        if let Some(max_tokens) = thinking_cap.thinking_budget {
            set_max_tokens_option(&mut options, max_tokens);
        }
        // TODO(converge): Thread per-request generation settings through
        // AgentOptions/provider adapters. The Anthropic and OpenAI-compatible
        // adapters derive max tokens from ModelProfile::max_output and do not
        // parse "max_tokens=..." or "temperature=..." extra_args.
        // TODO(converge): Thread req-level MCP config here in S05 once
        // ModelCallRequest carries it.

        let fallback_models = self.fallback_models_for_request(&model);
        let cell = ProviderCallCell::new(config, self.cost_table.clone());
        self.inference_started(&request_id, &model, &agent_id, auto_routed);
        let inference_start = Instant::now();
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
                self.inference_failed(&request_id, &model, &agent_id, &message);
                self.emit(RuntimeEvent::AgentFailed {
                    run_id: self.run_id.clone(),
                    agent_id,
                    error: message.clone(),
                });
                self.write_gateway_event(
                    &req,
                    &request_id,
                    &model,
                    &usage,
                    latency_ms,
                    false,
                    Some(message.clone()),
                )?;
                self.record_force_backend_override(&req.model, &model, false);
                self.record_feedback(
                    &req,
                    &request_id,
                    &model,
                    provider.as_deref(),
                    &usage,
                    latency_ms,
                    false,
                )
                .await?;
                return Err(RokoError::Agent {
                    backend: model,
                    message,
                });
            }
        };

        let usage = token_usage(&output.usage, output.cost_usd);
        self.inference_completed(
            &request_id,
            &output.model_used,
            &agent_id,
            &usage,
            inference_start.elapsed().as_millis() as u64,
        );
        let role = req.role.as_deref().unwrap_or("default");
        let convergence_key = format!("{}:{role}", self.run_id);
        if let Err(error) = self.convergence.check(&convergence_key, &output.content) {
            let latency_ms = start.elapsed().as_millis() as u64;
            self.emit(RuntimeEvent::AgentFailed {
                run_id: self.run_id.clone(),
                agent_id: format!("model-call:{}", output.model_used),
                error: error.to_string(),
            });
            self.record_force_backend_override(&req.model, &output.model_used, false);
            let output_provider = self.provider_for_model(&output.model_used);
            self.write_gateway_event(
                &req,
                &request_id,
                &output.model_used,
                &usage,
                latency_ms,
                false,
                Some(error.to_string()),
            )?;
            self.record_feedback(
                &req,
                &request_id,
                &output.model_used,
                output_provider.as_deref(),
                &usage,
                latency_ms,
                false,
            )
            .await?;
            return Err(RokoError::from(error));
        }
        self.convergence
            .record(&convergence_key, output.content.clone());

        self.budget.record_cost(usage.cost_usd);
        self.cache.store(
            cache_key,
            CachedResponse {
                content: output.content.clone(),
                model: output.model_used.clone(),
                usage: usage.clone(),
                stop_reason: Some("end_turn".to_string()),
            },
        );

        let agent_id = format!("model-call:{}", output.model_used);
        // ToolLoopAgent attaches per-turn state as trace metadata; emit it
        // separately from AgentOutput before the completion event.
        self.emit_agent_trace_events(&agent_id, &output.agent_traces, &usage);
        self.emit(RuntimeEvent::AgentCompleted {
            run_id: self.run_id.clone(),
            agent_id,
            output: output.content.clone(),
            tokens_used: usage.total_tokens,
            cost_usd: usage.cost_usd,
        });
        self.record_force_backend_override(&req.model, &output.model_used, true);
        let output_provider = self.provider_for_model(&output.model_used);
        self.write_gateway_event(
            &req,
            &request_id,
            &output.model_used,
            &usage,
            output.latency_ms,
            false,
            None,
        )?;
        self.record_feedback(
            &req,
            &request_id,
            &output.model_used,
            output_provider.as_deref(),
            &usage,
            output.latency_ms,
            true,
        )
        .await?;

        Ok(ModelCallResponse {
            content: output.content,
            model: output.model_used,
            usage,
            stop_reason: Some("end_turn".to_string()),
            request_id: Some(request_id),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_runner::ModelPricing;
    use futures::StreamExt;
    use roko_core::{
        ModelStreamEvent, model_call_failure_to_stream, model_call_response_to_stream,
    };
    use roko_learn::cascade_router::CascadeRouter;
    use tempfile::tempdir;

    struct TestCascadeRecorder {
        router: Arc<CascadeRouter>,
    }

    impl ForceBackendOverrideRecorder for TestCascadeRecorder {
        fn record_override_outcome(&self, model_slug: &str, success: bool) -> bool {
            self.router.record_confidence_outcome(model_slug, success)
        }
    }

    fn user_request(model: impl Into<String>, content: impl Into<String>) -> ModelCallRequest {
        ModelCallRequest {
            model: model.into(),
            system: None,
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: content.into(),
            }],
            max_tokens: None,
            temperature: None,
            role: None,
            caller: None,
            run_id: None,
            prompt_section_ids: Vec::new(),
            knowledge_ids: Vec::new(),
            budget: None,
            budget_remaining: None,
            routing_hints: Vec::new(),
            cache_policy: roko_core::foundation::CachePolicy::Default,
        }
    }

    #[test]
    fn request_prompt_keeps_single_user_turn_plain() {
        let (system, prompt) = request_prompt(&[ChatMessage {
            role: MessageRole::User,
            content: "hello".to_string(),
        }]);

        assert_eq!(system, None);
        assert_eq!(prompt, "hello");
    }

    #[test]
    fn request_prompt_preserves_roles_for_history() {
        let (system, prompt) = request_prompt(&[
            ChatMessage {
                role: MessageRole::System,
                content: "system one".to_string(),
            },
            ChatMessage {
                role: MessageRole::System,
                content: "system two".to_string(),
            },
            ChatMessage {
                role: MessageRole::User,
                content: "first".to_string(),
            },
            ChatMessage {
                role: MessageRole::Assistant,
                content: "second".to_string(),
            },
            ChatMessage {
                role: MessageRole::User,
                content: "third".to_string(),
            },
        ]);

        assert_eq!(system.as_deref(), Some("system one\n\nsystem two"));
        assert_eq!(prompt, "User:\nfirst\n\nAssistant:\nsecond\n\nUser:\nthird");
    }

    #[tokio::test]
    async fn model_stream_adapter_maps_successful_response() {
        let usage = TokenUsage {
            input_tokens: 1,
            output_tokens: 2,
            total_tokens: 3,
            cost_usd: 0.01,
        };
        let response = ModelCallResponse {
            content: "ok".to_string(),
            model: "model-a".to_string(),
            usage: usage.clone(),
            stop_reason: Some("end_turn".to_string()),
            request_id: Some("req-1".to_string()),
        };

        let events = model_call_response_to_stream(response)
            .collect::<Vec<_>>()
            .await;

        assert_eq!(
            events,
            vec![
                ModelStreamEvent::Started {
                    model: "model-a".to_string()
                },
                ModelStreamEvent::ContentDelta {
                    text: "ok".to_string()
                },
                ModelStreamEvent::Usage { usage },
                ModelStreamEvent::Completed {
                    stop_reason: Some("end_turn".to_string())
                }
            ]
        );
    }

    #[tokio::test]
    async fn model_stream_adapter_maps_failure_response() {
        let events = model_call_failure_to_stream("provider failed")
            .collect::<Vec<_>>()
            .await;

        assert_eq!(
            events,
            vec![ModelStreamEvent::Failed {
                error: "provider failed".to_string()
            }]
        );
    }

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
            run_id: None,
            prompt_section_ids: Vec::new(),
            knowledge_ids: Vec::new(),
            budget: None,
            budget_remaining: None,
            routing_hints: Vec::new(),
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
            run_id: None,
            prompt_section_ids: Vec::new(),
            knowledge_ids: Vec::new(),
            budget: None,
            budget_remaining: None,
            routing_hints: Vec::new(),
            cache_policy: roko_core::foundation::CachePolicy::Default,
        };
        assert_eq!(svc.resolve_model(&req), "claude-opus-4-20250514");
    }

    #[tokio::test]
    async fn cascade_router_selects_model_when_request_is_empty() {
        let svc = ModelCallService::new("default".into()).with_model_router(|role| {
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
            run_id: None,
            prompt_section_ids: Vec::new(),
            knowledge_ids: Vec::new(),
            budget: None,
            budget_remaining: None,
            routing_hints: Vec::new(),
            cache_policy: roko_core::foundation::CachePolicy::Default,
        };

        assert_eq!(svc.resolve_model(&req), "router-selected-model");
    }

    #[tokio::test]
    async fn force_backend_records_when_router_present() {
        let dir = tempdir().expect("tempdir");
        let model = "ux34-model";
        let router = Arc::new(CascadeRouter::load_or_new(
            &dir.path().join("cascade.json"),
            vec![model.to_string()],
        ));
        let svc = ModelCallService::new("default".into()).with_cascade_router(Arc::new(
            TestCascadeRecorder {
                router: Arc::clone(&router),
            },
        ));

        let response = svc
            .call(user_request(model, "learn this override"))
            .await
            .expect("model call should succeed");

        assert_eq!(response.model, model);
        assert_eq!(router.confidence_snapshot().get(model), Some(&(1, 1)));
    }

    #[tokio::test]
    async fn force_backend_noop_when_no_router() {
        let svc = ModelCallService::new("default".into());

        let response = svc
            .call(user_request("ux34-model", "no router attached"))
            .await
            .expect("model call should succeed without router");

        assert_eq!(response.model, "ux34-model");
    }

    #[tokio::test]
    async fn force_backend_records_failure_when_router_present() {
        let dir = tempdir().expect("tempdir");
        let model = "ux34-failing-model";
        let router = Arc::new(CascadeRouter::load_or_new(
            &dir.path().join("cascade.json"),
            vec![model.to_string()],
        ));
        let mut config = RokoConfig::default();
        config.agent.command = Some("false".to_string());
        let svc = ModelCallService::new("default".into())
            .with_config(config)
            .with_cascade_router(Arc::new(TestCascadeRecorder {
                router: Arc::clone(&router),
            }));

        let result = svc.call(user_request(model, "this should fail")).await;

        assert!(result.is_err());
        assert_eq!(router.confidence_snapshot().get(model), Some(&(1, 0)));
    }

    #[test]
    fn config_for_model_does_not_synthesize_providers_from_runtime_inputs() {
        let mut config = RokoConfig::default();
        config.providers.clear();
        config.models.clear();

        #[allow(deprecated)]
        let svc = ModelCallService::new("default".into())
            .with_config(config)
            .with_anthropic_api_key("sk-test".into())
            .with_openai_base_url("https://example.invalid/v1".into());
        let req = ModelCallRequest {
            model: "claude-haiku-4".into(),
            system: None,
            messages: vec![],
            max_tokens: None,
            temperature: None,
            role: None,
            caller: None,
            run_id: None,
            prompt_section_ids: Vec::new(),
            knowledge_ids: Vec::new(),
            budget: None,
            budget_remaining: None,
            routing_hints: Vec::new(),
            cache_policy: roko_core::foundation::CachePolicy::Default,
        };
        let model = svc.resolve_model(&req);
        let config = svc.config_for_model(&model);

        assert!(config.providers.is_empty());
        assert!(config.models.is_empty());

        let options = svc.build_agent_options(&req, None);
        assert!(
            options
                .env
                .iter()
                .any(|(key, value)| key == "ANTHROPIC_API_KEY" && value == "sk-test")
        );
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
            run_id: None,
            prompt_section_ids: Vec::new(),
            knowledge_ids: Vec::new(),
            budget: None,
            budget_remaining: None,
            routing_hints: Vec::new(),
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
            model: "mystery-model".into(),
            system: None,
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "hello".into(),
            }],
            max_tokens: None,
            temperature: None,
            role: None,
            caller: None,
            run_id: None,
            prompt_section_ids: Vec::new(),
            knowledge_ids: Vec::new(),
            budget: None,
            budget_remaining: None,
            routing_hints: Vec::new(),
            cache_policy: roko_core::foundation::CachePolicy::Default,
        };

        let estimate = svc.cost_predict(&req);

        assert_eq!(estimate.model, "mystery-model");
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
            run_id: None,
            prompt_section_ids: Vec::new(),
            knowledge_ids: Vec::new(),
            budget: None,
            budget_remaining: None,
            routing_hints: Vec::new(),
            cache_policy: roko_core::foundation::CachePolicy::Default,
        };

        let estimate = svc.cost_predict(&req);

        assert_eq!(estimate.model, "known-model");
        assert_eq!(estimate.estimated_input_tokens, 250);
        assert_eq!(estimate.max_output_tokens, 2048);
        assert!(estimate.predicted_cost_usd > 0.0);
    }

    #[test]
    fn cache_key_deterministic() {
        let messages = vec![ChatMessage {
            role: MessageRole::User,
            content: "hello".into(),
        }];

        let first = CacheCell::cache_key("model-a", None, &messages, Some(0.2), Some(1024));
        let second = CacheCell::cache_key("model-a", None, &messages, Some(0.2), Some(1024));

        assert_eq!(first, second);
    }

    #[test]
    fn cache_key_differs_on_model() {
        let messages = vec![ChatMessage {
            role: MessageRole::User,
            content: "hello".into(),
        }];

        let first = CacheCell::cache_key("model-a", None, &messages, None, None);
        let second = CacheCell::cache_key("model-b", None, &messages, None, None);

        assert_ne!(first, second);
    }

    #[test]
    fn cache_key_differs_on_temperature() {
        let messages = vec![ChatMessage {
            role: MessageRole::User,
            content: "hello".into(),
        }];

        let first = CacheCell::cache_key("model-a", None, &messages, Some(0.1), None);
        let second = CacheCell::cache_key("model-a", None, &messages, Some(0.9), None);

        assert_ne!(first, second);
    }

    #[test]
    fn cache_key_differs_on_max_tokens() {
        let messages = vec![ChatMessage {
            role: MessageRole::User,
            content: "hello".into(),
        }];

        let first = CacheCell::cache_key("model-a", None, &messages, None, Some(1024));
        let second = CacheCell::cache_key("model-a", None, &messages, None, Some(2048));

        assert_ne!(first, second);
    }

    #[test]
    fn cache_key_preserves_message_order() {
        let first_messages = vec![
            ChatMessage {
                role: MessageRole::User,
                content: "first".into(),
            },
            ChatMessage {
                role: MessageRole::Assistant,
                content: "second".into(),
            },
        ];
        let swapped_messages = vec![
            ChatMessage {
                role: MessageRole::Assistant,
                content: "second".into(),
            },
            ChatMessage {
                role: MessageRole::User,
                content: "first".into(),
            },
        ];

        let first = CacheCell::cache_key("model-a", None, &first_messages, Some(0.2), Some(1024));
        let second =
            CacheCell::cache_key("model-a", None, &swapped_messages, Some(0.2), Some(1024));

        assert_ne!(first, second);
    }

    #[test]
    fn cache_evicts_oldest_when_full() {
        let cache = CacheCell::new(2);
        let response = |content: &str| CachedResponse {
            content: content.to_string(),
            model: "model-a".to_string(),
            usage: TokenUsage::default(),
            stop_reason: Some("end_turn".to_string()),
        };

        cache.store(1, response("one"));
        cache.store(2, response("two"));
        cache.store(3, response("three"));

        assert!(cache.lookup(1).is_none());
        assert_eq!(cache.lookup(2).expect("second entry").content, "two");
        assert_eq!(cache.lookup(3).expect("third entry").content, "three");
    }

    #[test]
    fn budget_check_passes_when_no_limit() {
        let budget = BudgetCell::new(None);

        budget.record_cost(1_000.0);

        assert!(budget.check(&None).is_ok());
        assert_eq!(budget.cumulative_cost_usd(), 1_000.0);
    }

    #[test]
    fn budget_check_fails_when_exceeded() {
        let budget = BudgetCell::new(Some(1.0));

        budget.record_cost(1.01);

        let err = budget.check(&None).expect_err("budget should be exceeded");
        assert!(matches!(err, GatewayError::BudgetExceeded { .. }));
    }

    #[test]
    fn thinking_cap_ignores_non_thinking_models() {
        let cap = ThinkingCapCell::new(16_384);

        let result = cap.apply("claude-sonnet-4-6", None);

        assert_eq!(result.thinking_budget, None);
        assert!(!result.was_capped);
    }

    #[test]
    fn thinking_cap_caps_opus() {
        let cap = ThinkingCapCell::new(16_384);

        let result = cap.apply("claude-opus-4-6", None);

        assert_eq!(result.thinking_budget, Some(16_384));
        assert!(result.was_capped);
        assert_eq!(result.original, None);
    }

    #[test]
    fn thinking_cap_respects_lower_explicit() {
        let cap = ThinkingCapCell::new(16_384);

        let result = cap.apply("claude-opus-4-6", Some(4096));

        assert_eq!(result.thinking_budget, Some(4096));
        assert!(!result.was_capped);
        assert_eq!(result.original, Some(4096));
    }

    #[test]
    fn convergence_allows_different_outputs() {
        let convergence = ConvergenceDetectionCell::new(5, 0.85, 3);

        for output in ["first output", "second response", "third answer"] {
            convergence
                .check("run:role", output)
                .expect("different output should not converge");
            convergence.record("run:role", output.to_string());
        }
    }

    #[test]
    fn convergence_detects_identical_outputs() {
        let convergence = ConvergenceDetectionCell::new(5, 0.85, 3);

        for _ in 0..2 {
            convergence
                .check("run:role", "same output")
                .expect("first two identical outputs should not trigger");
            convergence.record("run:role", "same output".to_string());
        }

        let err = convergence
            .check("run:role", "same output")
            .expect_err("third identical output should trigger");
        assert!(matches!(
            err,
            GatewayError::ConvergenceDetected { consecutive: 3 }
        ));
    }

    #[test]
    fn convergence_resets_on_different_output() {
        let convergence = ConvergenceDetectionCell::new(5, 0.85, 3);
        let outputs = [
            "repeat output",
            "repeat output",
            "a materially different response",
            "repeat output",
            "repeat output",
        ];

        for output in outputs {
            convergence
                .check("run:role", output)
                .expect("different output should reset the convergence sequence");
            convergence.record("run:role", output.to_string());
        }
    }

    #[test]
    fn edit_distance_basic() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn similarity_identical_strings() {
        assert_eq!(similarity("abc", "abc"), 1.0);
    }
}
