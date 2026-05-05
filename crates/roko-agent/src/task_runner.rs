//! Task-runner composition types for the model-routing pipeline.
//!
//! This module defines the top-level runtime shape that coordinates agent
//! execution, events, anomaly detection, budget enforcement, conductor actions,
//! and cost accounting across task iterations.

use crate::{Agent, Usage, chat_types::FinishReason};
use indexmap::IndexMap;
use roko_core::config::schema::ModelProfile;
use roko_core::{Context, Signal};
use std::collections::{HashMap, VecDeque};
use thiserror::Error;
use tokio::sync::broadcast;

const DEFAULT_EVENT_BUS_CAPACITY: usize = roko_core::defaults::DEFAULT_EVENT_BUS_CAPACITY;
const PROMPT_LOOP_WINDOW: usize = 20;
const PROMPT_LOOP_THRESHOLD: usize = 5;

/// Composition point for the task execution pipeline.
pub struct TaskRunner {
    /// The task-facing agent implementation, such as Claude CLI or the tool loop.
    pub agent: Box<dyn Agent>,
    /// Event stream publisher for runtime feedback.
    pub event_bus: EventBus,
    /// Session-local anomaly detector.
    pub anomaly: AnomalyDetector,
    /// Budget guardrail applied across task iterations.
    pub budget: BudgetGuardrail,
    /// Learned conductor policy for intervention decisions.
    pub conductor: ConductorBandit,
    /// Pricing table used to compute aggregate cost.
    pub cost_table: CostTable,
    /// Requested model slug for this task.
    pub model_slug: String,
    /// Provider identifier for this task.
    pub provider_id: String,
    /// Maximum task-loop iterations before aborting.
    pub max_iterations: u32,
}

/// Aggregated outcome from a task runner execution.
#[derive(Debug, Clone)]
pub struct TaskResult {
    /// Final output signal emitted by the agent pipeline.
    pub output: Signal,
    /// Total usage accumulated across all iterations.
    pub total_usage: Usage,
    /// Total cost accumulated across all iterations.
    pub total_cost_usd: f64,
    /// Number of iterations consumed by the task run.
    pub iterations: u32,
    /// Whether the downstream gate accepted the result.
    pub gate_passed: bool,
    /// Conductor actions taken while processing the task.
    pub conductor_actions: Vec<ConductorAction>,
}

/// Errors that terminate the task loop immediately.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum TaskRunnerError {
    /// Budget guardrail blocked additional task execution.
    #[error("task budget exhausted")]
    BudgetExhausted,
    /// Prompt-loop anomaly was detected before dispatching the next turn.
    #[error("task anomaly detected: {0:?}")]
    Anomaly(Anomaly),
    /// The conductor requested a model escalation.
    #[error("task requires model escalation")]
    ModelEscalation,
}

/// Canonical runtime events emitted by the task runner.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    TurnStarted {
        task_id: String,
        model: String,
        provider: String,
        timestamp_ms: i64,
    },
    TurnCompleted {
        turn: u32,
        usage: Usage,
        tool_call_count: usize,
        gate_passed: Option<bool>,
        finish_reason: FinishReason,
    },
    CostRecorded {
        model: String,
        provider: String,
        cost_usd: f64,
        tokens: u64,
    },
    AnomalyDetected {
        anomaly: Anomaly,
    },
}

/// Pub/sub bus for broadcasting task-runner events.
pub struct EventBus {
    tx: broadcast::Sender<AgentEvent>,
}

impl EventBus {
    /// Create a new event bus with the given broadcast channel capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity.max(1));
        Self { tx }
    }

    /// Publish an event to all live subscribers.
    pub fn publish(&self, event: AgentEvent) {
        let _ = self.tx.send(event);
    }

    /// Subscribe to future events from this bus.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.tx.subscribe()
    }
}

impl Clone for EventBus {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(DEFAULT_EVENT_BUS_CAPACITY)
    }
}

/// Session-local anomalies surfaced by the task runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anomaly {
    /// The same prompt hash repeated too many times in the sliding window.
    PromptLoop {
        /// Number of matching prompt hashes inside the recent window.
        repeated_count: usize,
    },
}

/// Session-local anomaly detector owned by the task runner.
#[derive(Debug, Clone)]
pub struct AnomalyDetector {
    prompt_hash_window: VecDeque<u64>,
    session_start_ms: i64,
}

impl AnomalyDetector {
    /// Create a detector for a new session.
    #[must_use]
    pub fn new(session_start_ms: i64) -> Self {
        Self {
            prompt_hash_window: VecDeque::with_capacity(PROMPT_LOOP_WINDOW),
            session_start_ms,
        }
    }

    /// Return the session start timestamp in Unix milliseconds.
    #[must_use]
    pub const fn session_start_ms(&self) -> i64 {
        self.session_start_ms
    }

    /// Check a prompt hash for repeated-loop behavior.
    #[must_use]
    pub fn check_prompt(&mut self, prompt_hash: u64) -> Option<Anomaly> {
        self.prompt_hash_window.push_back(prompt_hash);
        if self.prompt_hash_window.len() > PROMPT_LOOP_WINDOW {
            self.prompt_hash_window.pop_front();
        }

        let repeated_count = self
            .prompt_hash_window
            .iter()
            .filter(|&&hash| hash == prompt_hash)
            .count();

        (repeated_count >= PROMPT_LOOP_THRESHOLD).then_some(Anomaly::PromptLoop { repeated_count })
    }
}

impl Default for AnomalyDetector {
    fn default() -> Self {
        Self::new(chrono::Utc::now().timestamp_millis())
    }
}

/// Budget enforcement state across multiple scopes.
#[derive(Debug, Clone)]
pub struct BudgetGuardrail {
    /// Maximum spend allowed per task, in USD.
    pub per_task_limit_usd: f64,
    /// Maximum spend allowed per session, in USD.
    pub per_session_limit_usd: f64,
    /// Maximum spend allowed per day, in USD.
    pub per_day_limit_usd: f64,
    /// Warning threshold expressed as a fraction in the range `0.0..=1.0`.
    pub warn_at_percent: f64,
    task_spent: f64,
    session_spent: f64,
    day_spent: f64,
}

/// Action to take once a budget threshold is crossed.
#[derive(Debug, Clone, PartialEq)]
pub enum BudgetAction {
    /// No budget issue detected.
    Ok,
    /// Warn while returning the percent of budget consumed.
    Warn {
        /// Fraction of the budget consumed.
        percent_used: f64,
        /// Level used for the warning.
        level: &'static str,
    },
    /// Spend is high enough that routing should favor cheaper models.
    RouteToCheaper,
    /// New sessions should be blocked to limit further exposure.
    BlockNewSessions,
    /// The budget has been exhausted.
    Block,
}

impl BudgetGuardrail {
    /// Create a new guardrail with explicit limits.
    #[must_use]
    pub const fn new(
        per_task_limit_usd: f64,
        per_session_limit_usd: f64,
        per_day_limit_usd: f64,
        warn_at_percent: f64,
    ) -> Self {
        Self {
            per_task_limit_usd,
            per_session_limit_usd,
            per_day_limit_usd,
            warn_at_percent,
            task_spent: 0.0,
            session_spent: 0.0,
            day_spent: 0.0,
        }
    }

    /// Record a cost against a budget level and return the resulting action.
    pub fn record_cost(&mut self, cost_usd: f64, level: &str) -> BudgetAction {
        match level {
            "task" => {
                self.task_spent += cost_usd;
                self.check_budget(self.task_spent, self.per_task_limit_usd)
            }
            "session" => {
                self.session_spent += cost_usd;
                self.check_budget(self.session_spent, self.per_session_limit_usd)
            }
            "day" => {
                self.day_spent += cost_usd;
                self.check_budget(self.day_spent, self.per_day_limit_usd)
            }
            _ => BudgetAction::Ok,
        }
    }

    fn check_budget(&self, spent: f64, limit: f64) -> BudgetAction {
        if limit <= 0.0 {
            return BudgetAction::Ok;
        }

        let pct = spent / limit;
        if pct >= 1.0 {
            BudgetAction::Block
        } else if pct >= 0.95 {
            BudgetAction::BlockNewSessions
        } else if pct >= 0.80 {
            BudgetAction::RouteToCheaper
        } else if pct >= self.warn_at_percent {
            BudgetAction::Warn {
                percent_used: pct,
                level: "budget",
            }
        } else {
            BudgetAction::Ok
        }
    }
}

impl Default for BudgetGuardrail {
    fn default() -> Self {
        Self::new(0.0, 0.0, 0.0, 0.75)
    }
}

/// Coarse error taxonomy for conductor features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorPattern {
    /// No reliable diagnosis is available yet.
    Unknown,
    /// Compiler or static-analysis failure.
    Compile,
    /// Tests or assertions failed.
    Test,
    /// Tool invocation or tool-result mismatch failed.
    ToolCall,
    /// A timeout or long-running stall was observed.
    Timeout,
    /// Filesystem, process, or environment failure.
    Infrastructure,
}

/// Current conductor decision context.
#[derive(Debug, Clone, PartialEq)]
pub struct ConductorState {
    /// Current attempt number for the task.
    pub iteration: u32,
    /// Number of failed attempts in a row.
    pub consecutive_failures: u32,
    /// Coarse classification of the failure shape.
    pub error_pattern: ErrorPattern,
    /// Wall-clock time already spent on this task.
    pub elapsed_ms: u64,
    /// Accumulated spend for the task so far.
    pub cost_so_far_usd: f64,
    /// Active model tier label such as `fast`, `standard`, or `premium`.
    pub model_tier: String,
    /// Task tier label such as `mechanical` or `architectural`.
    pub task_complexity: String,
}

/// Learned conductor policy placeholder owned by the task runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HintType {
    /// Inject an enriched error digest into the next retry.
    #[default]
    ErrorDigest,
    /// Suggest a relevant reusable skill or recipe.
    SkillSuggestion,
    /// Request a simpler implementation path.
    SimplifyApproach,
}

/// Intervention selected by the conductor while a task is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConductorAction {
    /// Continue without extra intervention.
    #[default]
    Continue,
    /// Inject a targeted hint before the next retry.
    InjectHint(HintType),
    /// Switch to a different model tier.
    SwitchModel,
    /// Restart the task from a cleaner state.
    Restart,
    /// Abort the task.
    Abort,
}

/// Lightweight conductor policy shim for the task runner.
#[derive(Debug, Clone)]
pub struct ConductorBandit {
    default_action: ConductorAction,
}

impl ConductorBandit {
    /// Create a conductor that defaults to `Continue`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            default_action: ConductorAction::Continue,
        }
    }

    /// Create a conductor that always selects the provided action.
    #[must_use]
    pub const fn with_default_action(default_action: ConductorAction) -> Self {
        Self { default_action }
    }

    /// Choose an action for the current task state.
    #[must_use]
    pub const fn select_action(&self, _state: &ConductorState) -> ConductorAction {
        self.default_action
    }
}

impl Default for ConductorBandit {
    fn default() -> Self {
        Self::new()
    }
}

/// Pricing for a single model slug.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelPricing {
    /// Cost in USD per million input tokens.
    pub input_per_m: f64,
    /// Cost in USD per million output tokens.
    pub output_per_m: f64,
    /// Cost in USD per million cache-read tokens.
    pub cache_read_per_m: f64,
    /// Cost in USD per million cache-write tokens.
    pub cache_write_per_m: f64,
}

/// Per-model pricing table owned by the task runner.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CostTable {
    /// Pricing entries keyed by model slug.
    pub models: HashMap<String, ModelPricing>,
}

/// Sonnet-rate fallback used when a model slug is unknown but tokens > 0.
const SONNET_FALLBACK: ModelPricing = ModelPricing {
    input_per_m: 3.00,
    output_per_m: 15.00,
    cache_read_per_m: 0.30,
    cache_write_per_m: 3.75,
};

/// Hardcoded pricing for well-known models: (slug, input, output, cache_read, cache_write).
const KNOWN_MODEL_PRICING: &[(&str, f64, f64, f64, f64)] = &[
    ("claude-opus-4-6", 15.00, 75.00, 3.75, 18.75),
    ("claude-sonnet-4-6", 3.00, 15.00, 0.30, 3.75),
    ("claude-haiku-4-5", 0.80, 4.00, 0.08, 1.00),
    ("glm-5.1", 1.40, 4.40, 0.26, 1.75),
    ("glm-5", 1.00, 3.20, 0.50, 1.25),
    ("kimi-k2.5", 0.60, 3.00, 0.10, 0.75),
    ("gpt-5.2", 2.00, 8.00, 0.50, 2.50),
    ("gpt-5.4", 2.50, 10.00, 0.63, 3.13),
    ("gpt-5.4-mini", 0.40, 1.60, 0.10, 0.50),
];

impl CostTable {
    /// Insert or replace pricing for a model.
    pub fn insert(&mut self, model_slug: impl Into<String>, pricing: ModelPricing) {
        self.models.insert(model_slug.into(), pricing);
    }

    /// Build a cost table from config model profiles, then merge hardcoded defaults
    /// for known models (without overriding config-supplied pricing).
    #[must_use]
    pub fn from_config_with_defaults(models: &IndexMap<String, ModelProfile>) -> Self {
        let mut table = Self::default();

        // Populate from config profiles.
        for profile in models.values() {
            if let (Some(input), Some(output)) =
                (profile.cost_input_per_m, profile.cost_output_per_m)
            {
                table.insert(
                    profile.slug.clone(),
                    ModelPricing {
                        input_per_m: input,
                        output_per_m: output,
                        cache_read_per_m: profile.cost_cache_read_per_m.unwrap_or(input * 0.5),
                        cache_write_per_m: profile.cost_cache_write_per_m.unwrap_or(input * 1.25),
                    },
                );
            }
        }

        // Merge hardcoded defaults for known models (won't override config).
        for &(slug, input, output, cache_r, cache_w) in KNOWN_MODEL_PRICING {
            table
                .models
                .entry(slug.to_string())
                .or_insert(ModelPricing {
                    input_per_m: input,
                    output_per_m: output,
                    cache_read_per_m: cache_r,
                    cache_write_per_m: cache_w,
                });
        }

        table
    }

    /// Calculate request cost from raw token counts.
    ///
    /// Falls back to Sonnet rates when the model is unknown but tokens > 0.
    #[must_use]
    pub fn calculate(&self, model_slug: &str, usage: &Usage) -> f64 {
        let total_tokens = usage.input_tokens
            + usage.output_tokens
            + usage.cache_read_tokens
            + usage.cache_create_tokens;

        let pricing = match self.models.get(model_slug) {
            Some(p) => p,
            None if total_tokens > 0 => &SONNET_FALLBACK,
            None => return 0.0,
        };

        (usage.input_tokens as f64 * pricing.input_per_m / 1_000_000.0)
            + (usage.output_tokens as f64 * pricing.output_per_m / 1_000_000.0)
            + (usage.cache_read_tokens as f64 * pricing.cache_read_per_m / 1_000_000.0)
            + (usage.cache_create_tokens as f64 * pricing.cache_write_per_m / 1_000_000.0)
    }
}

impl TaskRunner {
    /// Run the full task pipeline until success, abort, escalation, or a hard stop.
    pub async fn run_task(
        &mut self,
        task_signal: &Signal,
        ctx: &Context,
    ) -> Result<TaskResult, TaskRunnerError> {
        let mut iterations = 0;
        let mut total_usage = Usage::default();
        let mut total_cost_usd = 0.0;
        let mut last_iteration_cost_usd = 0.0;
        let mut consecutive_failures: u32 = 0;
        let mut conductor_actions = Vec::new();

        loop {
            iterations += 1;

            // BudgetGuardrail already accumulates internally, so only feed it
            // the previous turn's incremental cost before starting the next turn.
            let budget_action = self.budget.record_cost(last_iteration_cost_usd, "task");
            if matches!(budget_action, BudgetAction::Block) {
                return Err(TaskRunnerError::BudgetExhausted);
            }

            let prompt_hash = prompt_hash_u64(task_signal);
            if let Some(anomaly) = self.anomaly.check_prompt(prompt_hash) {
                self.event_bus.publish(AgentEvent::AnomalyDetected {
                    anomaly: anomaly.clone(),
                });
                return Err(TaskRunnerError::Anomaly(anomaly));
            }

            self.event_bus.publish(AgentEvent::TurnStarted {
                task_id: task_id_from_context(ctx),
                model: self.model_slug.clone(),
                provider: self.provider_id.clone(),
                timestamp_ms: event_timestamp_ms(ctx),
            });

            let result = self.agent.run(task_signal, ctx).await;
            total_usage.add(&result.usage);

            let iteration_cost_usd = self.cost_table.calculate(&self.model_slug, &result.usage);
            total_cost_usd += iteration_cost_usd;
            last_iteration_cost_usd = iteration_cost_usd;

            let finish_reason = if result.success {
                FinishReason::Stop
            } else {
                FinishReason::Error("agent failed".to_string())
            };

            self.event_bus.publish(AgentEvent::TurnCompleted {
                turn: iterations,
                usage: result.usage,
                tool_call_count: 0,
                gate_passed: None,
                finish_reason: finish_reason.clone(),
            });
            self.event_bus.publish(AgentEvent::CostRecorded {
                model: self.model_slug.clone(),
                provider: self.provider_id.clone(),
                cost_usd: iteration_cost_usd,
                tokens: u64::from(result.usage.total_tokens()),
            });

            if result.success {
                return Ok(TaskResult {
                    output: result.output,
                    total_usage,
                    total_cost_usd,
                    iterations,
                    gate_passed: true,
                    conductor_actions,
                });
            }

            consecutive_failures = consecutive_failures.saturating_add(1);
            let conductor_state = ConductorState {
                iteration: iterations,
                consecutive_failures,
                error_pattern: classify_error_pattern(&result.output),
                elapsed_ms: total_usage.wall_ms,
                cost_so_far_usd: total_cost_usd,
                model_tier: ctx.attr("model_tier").unwrap_or_default().to_string(),
                task_complexity: ctx.attr("task_complexity").unwrap_or_default().to_string(),
            };
            let action = self.conductor.select_action(&conductor_state);
            conductor_actions.push(action);

            match action {
                ConductorAction::Abort => {
                    return Ok(TaskResult {
                        output: result.output,
                        total_usage,
                        total_cost_usd,
                        iterations,
                        gate_passed: false,
                        conductor_actions,
                    });
                }
                ConductorAction::SwitchModel => {
                    return Err(TaskRunnerError::ModelEscalation);
                }
                ConductorAction::Restart => {
                    consecutive_failures = 0;
                }
                ConductorAction::Continue | ConductorAction::InjectHint(_) => {}
            }

            if iterations >= self.max_iterations {
                return Ok(TaskResult {
                    output: result.output,
                    total_usage,
                    total_cost_usd,
                    iterations,
                    gate_passed: false,
                    conductor_actions,
                });
            }
        }
    }
}

fn prompt_hash_u64(signal: &Signal) -> u64 {
    let hash = signal.content_hash();
    let bytes: [u8; 8] = hash.0[..8].try_into().expect("content hash prefix");
    u64::from_be_bytes(bytes)
}

pub(crate) fn task_id_from_context(ctx: &Context) -> String {
    ctx.attr("task_id")
        .or_else(|| ctx.attr("task"))
        .or(ctx.session.as_deref())
        .unwrap_or_default()
        .to_string()
}

fn event_timestamp_ms(ctx: &Context) -> i64 {
    if ctx.now_ms != 0 {
        ctx.now_ms
    } else {
        chrono::Utc::now().timestamp_millis()
    }
}

fn classify_error_pattern(output: &Signal) -> ErrorPattern {
    let Ok(text) = output.body.as_text() else {
        return ErrorPattern::Unknown;
    };
    let text = text.to_ascii_lowercase();

    // Priority-ordered matching: more specific patterns first to avoid
    // false positives (e.g. "this test compiles fine" should match Test,
    // not Compile).

    // Timeout is the most unambiguous signal.
    if text.contains("timed out") || text.contains("timeout") || text.contains("deadline exceeded")
    {
        return ErrorPattern::Timeout;
    }

    // Infrastructure: network/IO errors are unambiguous when they include
    // specific keywords.
    if text.contains("io error")
        || text.contains("permission denied")
        || text.contains("connection refused")
        || text.contains("dns resolution")
        || text.contains("network error")
        || text.contains("econnreset")
        || text.contains("broken pipe")
    {
        return ErrorPattern::Infrastructure;
    }

    // Tool call failures -- look for specific tool error patterns.
    if text.contains("tool call failed")
        || text.contains("tool execution error")
        || text.contains("tool_use_error")
    {
        return ErrorPattern::ToolCall;
    }

    // Compile: look for compiler-specific indicators, not just "compile".
    if text.contains("error[e") // rustc error codes like error[E0308]
        || text.contains("borrow checker")
        || text.contains("cannot find")
        || text.contains("mismatched types")
        || text.contains("unresolved import")
        || (text.contains("rustc") && text.contains("error"))
        || (text.contains("cargo build") && text.contains("failed"))
    {
        return ErrorPattern::Compile;
    }

    // Test: look for test runner output patterns.
    if text.contains("test result: failed")
        || text.contains("assertion failed")
        || text.contains("panicked at")
        || (text.contains("cargo test") && text.contains("failed"))
    {
        return ErrorPattern::Test;
    }

    // Fallback: broad filesystem/network patterns (lower priority to avoid
    // false positives).
    if text.contains("filesystem") || text.contains("no such file") {
        return ErrorPattern::Infrastructure;
    }

    ErrorPattern::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Agent, AgentResult};
    use async_trait::async_trait;
    use roko_core::{Body, Kind, Provenance};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    struct SequenceAgent {
        name: String,
        results: Arc<Mutex<VecDeque<AgentResult>>>,
        run_count: Arc<AtomicUsize>,
    }

    impl SequenceAgent {
        fn new(results: Vec<AgentResult>) -> Self {
            Self {
                name: "sequence".to_string(),
                results: Arc::new(Mutex::new(results.into())),
                run_count: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn run_count_handle(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.run_count)
        }
    }

    #[async_trait]
    impl Agent for SequenceAgent {
        async fn run(&self, _input: &Signal, _ctx: &Context) -> AgentResult {
            self.run_count.fetch_add(1, Ordering::SeqCst);
            self.results
                .lock()
                .expect("sequence agent lock")
                .pop_front()
                .expect("sequence agent result")
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    fn prompt(text: &str) -> Signal {
        Signal::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    fn agent_result(text: &str, success: bool, usage: Usage) -> AgentResult {
        let output = Signal::builder(Kind::AgentOutput)
            .body(Body::text(text))
            .provenance(Provenance::agent("sequence"))
            .build();

        if success {
            AgentResult::ok(output).with_usage(usage)
        } else {
            AgentResult::fail(output).with_usage(usage)
        }
    }

    fn runner(agent: SequenceAgent) -> TaskRunner {
        let mut cost_table = CostTable::default();
        cost_table.insert(
            "glm-5.1",
            ModelPricing {
                input_per_m: 1.0,
                output_per_m: 2.0,
                cache_read_per_m: 0.0,
                cache_write_per_m: 0.0,
            },
        );

        TaskRunner {
            agent: Box::new(agent),
            event_bus: EventBus::new(16),
            anomaly: AnomalyDetector::new(1_700_000_000_000),
            budget: BudgetGuardrail::new(10.0, 10.0, 10.0, 0.75),
            conductor: ConductorBandit::new(),
            cost_table,
            model_slug: "glm-5.1".to_string(),
            provider_id: "zai".to_string(),
            max_iterations: 3,
        }
    }

    fn drain_events(rx: &mut broadcast::Receiver<AgentEvent>) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    #[tokio::test]
    async fn task_runner_pipeline_success_publishes_events_and_returns_result() {
        let agent = SequenceAgent::new(vec![agent_result(
            "done",
            true,
            Usage {
                input_tokens: 1_000,
                output_tokens: 500,
                wall_ms: 900,
                ..Usage::default()
            },
        )]);
        let mut runner = runner(agent);
        let mut rx = runner.event_bus.subscribe();
        let task = prompt("ship it");
        let ctx = Context::at(1234)
            .with_attr("task_id", "task-2k29")
            .with_attr("task_complexity", "architectural")
            .with_attr("model_tier", "standard");

        let result = runner.run_task(&task, &ctx).await.expect("task result");
        let events = drain_events(&mut rx);

        assert!(result.gate_passed);
        assert_eq!(result.iterations, 1);
        assert!(result.conductor_actions.is_empty());
        assert_eq!(result.total_usage.input_tokens, 1_000);
        assert_eq!(result.total_usage.output_tokens, 500);
        assert_eq!(result.total_usage.wall_ms, 900);
        assert!((result.total_cost_usd - 0.002).abs() < 1e-12);
        assert_eq!(result.output.body.as_text().ok(), Some("done"));

        assert_eq!(events.len(), 3);
        assert!(matches!(
            &events[0],
            AgentEvent::TurnStarted {
                task_id,
                model,
                provider,
                timestamp_ms,
            } if task_id == "task-2k29"
                && model == "glm-5.1"
                && provider == "zai"
                && *timestamp_ms == 1234
        ));
        assert!(matches!(
            &events[1],
            AgentEvent::TurnCompleted {
                turn,
                usage,
                gate_passed,
                finish_reason,
                ..
            } if *turn == 1
                && usage.input_tokens == 1_000
                && usage.output_tokens == 500
                && gate_passed.is_none()
                && *finish_reason == FinishReason::Stop
        ));
        assert!(matches!(
            &events[2],
            AgentEvent::CostRecorded {
                model,
                provider,
                cost_usd,
                tokens,
            } if model == "glm-5.1"
                && provider == "zai"
                && (*cost_usd - 0.002).abs() < 1e-12
                && *tokens == 1_500
        ));
    }

    #[tokio::test]
    async fn task_runner_pipeline_aborts_when_conductor_requests_it() {
        let agent = SequenceAgent::new(vec![agent_result(
            "compile failed",
            false,
            Usage {
                input_tokens: 200,
                output_tokens: 50,
                wall_ms: 100,
                ..Usage::default()
            },
        )]);
        let mut runner = runner(agent);
        runner.conductor = ConductorBandit::with_default_action(ConductorAction::Abort);

        let result = runner
            .run_task(&prompt("fix build"), &Context::at(55))
            .await
            .expect("abort result");

        assert!(!result.gate_passed);
        assert_eq!(result.iterations, 1);
        assert_eq!(result.conductor_actions, vec![ConductorAction::Abort]);
        assert_eq!(result.output.body.as_text().ok(), Some("compile failed"));
    }

    #[tokio::test]
    async fn task_runner_pipeline_returns_model_escalation_when_requested() {
        let agent = SequenceAgent::new(vec![agent_result(
            "still broken",
            false,
            Usage {
                input_tokens: 100,
                output_tokens: 20,
                ..Usage::default()
            },
        )]);
        let mut runner = runner(agent);
        runner.conductor = ConductorBandit::with_default_action(ConductorAction::SwitchModel);

        let err = runner
            .run_task(&prompt("retry"), &Context::at(77))
            .await
            .expect_err("model escalation");

        assert_eq!(err, TaskRunnerError::ModelEscalation);
    }

    #[tokio::test]
    async fn task_runner_pipeline_stops_on_prompt_loop_before_running_agent() {
        let agent =
            SequenceAgent::new(vec![agent_result("should not run", true, Usage::default())]);
        let run_count = agent.run_count_handle();
        let task = prompt("loop forever");
        let prompt_hash = prompt_hash_u64(&task);
        let mut anomaly = AnomalyDetector::new(1_700_000_000_000);
        for _ in 0..4 {
            assert!(anomaly.check_prompt(prompt_hash).is_none());
        }

        let mut runner = runner(agent);
        runner.anomaly = anomaly;
        let mut rx = runner.event_bus.subscribe();

        let err = runner
            .run_task(&task, &Context::at(88))
            .await
            .expect_err("prompt loop anomaly");
        let events = drain_events(&mut rx);

        assert_eq!(run_count.load(Ordering::SeqCst), 0);
        assert_eq!(
            err,
            TaskRunnerError::Anomaly(Anomaly::PromptLoop { repeated_count: 5 })
        );
        assert_eq!(
            events,
            vec![AgentEvent::AnomalyDetected {
                anomaly: Anomaly::PromptLoop { repeated_count: 5 }
            }]
        );
    }

    #[tokio::test]
    async fn task_runner_pipeline_blocks_budget_before_running_agent() {
        let agent =
            SequenceAgent::new(vec![agent_result("should not run", true, Usage::default())]);
        let run_count = agent.run_count_handle();
        let mut budget = BudgetGuardrail::new(0.5, 1.0, 1.0, 0.75);
        assert_eq!(budget.record_cost(0.5, "task"), BudgetAction::Block);

        let mut runner = runner(agent);
        runner.budget = budget;

        let err = runner
            .run_task(&prompt("budgeted"), &Context::at(99))
            .await
            .expect_err("budget exhausted");

        assert_eq!(run_count.load(Ordering::SeqCst), 0);
        assert_eq!(err, TaskRunnerError::BudgetExhausted);
    }

    #[tokio::test]
    async fn task_runner_pipeline_continues_until_max_iterations() {
        let agent = SequenceAgent::new(vec![
            agent_result(
                "test failed",
                false,
                Usage {
                    input_tokens: 100,
                    output_tokens: 25,
                    wall_ms: 40,
                    ..Usage::default()
                },
            ),
            agent_result(
                "test failed again",
                false,
                Usage {
                    input_tokens: 120,
                    output_tokens: 30,
                    wall_ms: 50,
                    ..Usage::default()
                },
            ),
        ]);
        let mut runner = runner(agent);
        runner.max_iterations = 2;
        let mut rx = runner.event_bus.subscribe();

        let result = runner
            .run_task(
                &prompt("keep trying"),
                &Context::at(111).with_attr("task_complexity", "mechanical"),
            )
            .await
            .expect("max-iteration result");
        let events = drain_events(&mut rx);

        assert!(!result.gate_passed);
        assert_eq!(result.iterations, 2);
        assert_eq!(
            result.conductor_actions,
            vec![ConductorAction::Continue, ConductorAction::Continue]
        );
        assert_eq!(result.total_usage.input_tokens, 220);
        assert_eq!(result.total_usage.output_tokens, 55);
        assert_eq!(result.total_usage.wall_ms, 90);
        assert!((result.total_cost_usd - 0.00033).abs() < 1e-12);
        assert_eq!(result.output.body.as_text().ok(), Some("test failed again"));
        assert_eq!(events.len(), 6);
    }
}
