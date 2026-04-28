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
    CachePolicy, ChatMessage, FeedbackEvent, FeedbackSink, GatewayError, MessageRole,
    ModelCallRequest, ModelCallResponse, ModelCaller, TokenBudget, TokenUsage,
};
use roko_core::{
    Body, Context, Engram, EventConsumer, Kind, Result, RokoError, RuntimeEvent, Usage,
};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

type ModelRouter = dyn Fn(Option<&str>) -> String + Send + Sync;

/// Records explicit model override outcomes for cascade-router learning.
pub trait ForceBackendOverrideRecorder: Send + Sync {
    /// Record the outcome for a forced model slug.
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
    /// Optional sink for model-call feedback.
    feedback_sink: Option<Arc<dyn FeedbackSink>>,
    /// Optional model router used when requests omit an explicit model.
    model_router: Option<Arc<ModelRouter>>,
    /// Optional cascade router callback for recording forced model observations.
    ///
    /// This is trait-typed to avoid adding a production `roko-agent` ->
    /// `roko-learn` dependency edge; `CascadeRouter` implements the trait in
    /// `roko-learn`.
    cascade_router: Option<Arc<dyn ForceBackendOverrideRecorder>>,
    /// Service-scoped environment entries passed into provider construction.
    env: Vec<(String, String)>,
    /// Optional base URL for OpenAI-compatible providers.
    openai_base_url: Option<String>,
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
            cascade_router: None,
            env: Vec::new(),
            openai_base_url: None,
            mcp_config: None,
            cache: CacheCell::new(128),
            budget: BudgetCell::new(None),
            thinking_cap: ThinkingCapCell::new(16_384),
            convergence: ConvergenceDetectionCell::new(5, 0.85, 3),
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

/// L1 in-memory response cache. Keyed by (model, messages, temperature).
///
/// TODO(gateway): L2 semantic cache.
struct CacheCell {
    /// Maximum number of cached entries.
    max_entries: usize,
    /// The cache store. Uses a simple HashMap with LRU eviction.
    entries: Mutex<HashMap<u64, CachedResponse>>,
    /// Insertion-order keys for LRU eviction.
    order: Mutex<Vec<u64>>,
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
            max_entries,
            entries: Mutex::new(HashMap::new()),
            order: Mutex::new(Vec::new()),
        }
    }

    /// Compute a cache key from request fields.
    /// Hash of: model + sorted message contents + temperature (if set).
    fn cache_key(model: &str, messages: &[ChatMessage], temperature: Option<f32>) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        model.hash(&mut hasher);

        let mut messages = messages
            .iter()
            .map(|message| (message_role_tag(&message.role), message.content.as_str()))
            .collect::<Vec<_>>();
        messages.sort_unstable();

        for (role, content) in messages {
            role.hash(&mut hasher);
            content.hash(&mut hasher);
        }

        temperature.map(f32::to_bits).hash(&mut hasher);
        hasher.finish()
    }

    /// Look up a cached response. Returns None on miss.
    fn lookup(&self, key: u64) -> Option<CachedResponse> {
        let entries = self.entries.lock().expect("cache entries mutex poisoned");
        let response = entries.get(&key).cloned();
        drop(entries);

        if response.is_some() {
            let mut order = self.order.lock().expect("cache order mutex poisoned");
            if let Some(index) = order.iter().position(|existing| *existing == key) {
                order.remove(index);
            }
            order.push(key);
        }

        response
    }

    /// Store a successful response. Evicts the oldest entry if at capacity.
    fn store(&self, key: u64, response: CachedResponse) {
        if self.max_entries == 0 {
            return;
        }

        let mut entries = self.entries.lock().expect("cache entries mutex poisoned");
        let mut order = self.order.lock().expect("cache order mutex poisoned");

        if entries.contains_key(&key) {
            order.retain(|existing| *existing != key);
        } else if entries.len() >= self.max_entries {
            if let Some(oldest) = order.first().copied() {
                entries.remove(&oldest);
                order.remove(0);
            }
        }

        entries.insert(key, response);
        order.push(key);
    }

    fn evict(&self, key: u64) {
        let mut entries = self.entries.lock().expect("cache entries mutex poisoned");
        let mut order = self.order.lock().expect("cache order mutex poisoned");
        entries.remove(&key);
        order.retain(|existing| *existing != key);
    }
}

fn message_role_tag(role: &MessageRole) -> u8 {
    match role {
        MessageRole::System => 0,
        MessageRole::User => 1,
        MessageRole::Assistant => 2,
    }
}

/// Tracks cumulative cost across calls within a workflow run and enforces per-call budgets.
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
            let history = self
                .history
                .lock()
                .expect("convergence history mutex poisoned");
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

        let mut history = self
            .history
            .lock()
            .expect("convergence history mutex poisoned");
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
        let cache_key = CacheCell::cache_key(&model, &req.messages, req.temperature);

        match req.cache_policy {
            CachePolicy::Default => {
                if let Some(cached) = self.cache.lookup(cache_key) {
                    return Ok(ModelCallResponse {
                        content: cached.content,
                        model: cached.model,
                        usage: cached.usage,
                        stop_reason: cached.stop_reason,
                        request_id: Some(format!("cache-hit:{cache_key:016x}")),
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
        let system_prompt = req.system.clone().or(message_system);
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
                self.record_force_backend_override(&req.model, &model, false);
                self.record_feedback(&req, &model, &usage, latency_ms, false)
                    .await?;
                return Err(RokoError::Agent {
                    backend: model,
                    message,
                });
            }
        };

        let usage = token_usage(&output.usage, output.cost_usd);
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
            self.record_feedback(&req, &output.model_used, &usage, latency_ms, false)
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

        self.emit(RuntimeEvent::AgentCompleted {
            run_id: self.run_id.clone(),
            agent_id: format!("model-call:{}", output.model_used),
            output: output.content.clone(),
            tokens_used: usage.total_tokens,
            cost_usd: usage.cost_usd,
        });
        self.record_force_backend_override(&req.model, &output.model_used, true);
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
    use roko_learn::cascade_router::CascadeRouter;
    use roko_learn::model_router::RoutingContext;
    use tempfile::tempdir;

    struct TestCascadeRecorder {
        router: Arc<CascadeRouter>,
    }

    impl ForceBackendOverrideRecorder for TestCascadeRecorder {
        fn record_override_outcome(&self, model_slug: &str, success: bool) -> bool {
            self.router
                .record_override_outcome(model_slug, &RoutingContext::default(), success)
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
            budget: None,
            cache_policy: roko_core::foundation::CachePolicy::Default,
        }
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
            budget: None,
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

    #[test]
    fn cache_key_deterministic() {
        let messages = vec![ChatMessage {
            role: MessageRole::User,
            content: "hello".into(),
        }];

        let first = CacheCell::cache_key("model-a", &messages, Some(0.2));
        let second = CacheCell::cache_key("model-a", &messages, Some(0.2));

        assert_eq!(first, second);
    }

    #[test]
    fn cache_key_differs_on_model() {
        let messages = vec![ChatMessage {
            role: MessageRole::User,
            content: "hello".into(),
        }];

        let first = CacheCell::cache_key("model-a", &messages, None);
        let second = CacheCell::cache_key("model-b", &messages, None);

        assert_ne!(first, second);
    }

    #[test]
    fn cache_key_differs_on_temperature() {
        let messages = vec![ChatMessage {
            role: MessageRole::User,
            content: "hello".into(),
        }];

        let first = CacheCell::cache_key("model-a", &messages, Some(0.1));
        let second = CacheCell::cache_key("model-a", &messages, Some(0.9));

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

        let result = cap.apply("claude-sonnet-4", None);

        assert_eq!(result.thinking_budget, None);
        assert!(!result.was_capped);
    }

    #[test]
    fn thinking_cap_caps_opus() {
        let cap = ThinkingCapCell::new(16_384);

        let result = cap.apply("claude-opus-4", None);

        assert_eq!(result.thinking_budget, Some(16_384));
        assert!(result.was_capped);
        assert_eq!(result.original, None);
    }

    #[test]
    fn thinking_cap_respects_lower_explicit() {
        let cap = ThinkingCapCell::new(16_384);

        let result = cap.apply("claude-opus-4", Some(4096));

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
