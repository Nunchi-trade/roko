//! Shared in-memory state for per-agent routes.

use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use parking_lot::Mutex;
use roko_agent::chat_types::{
    ChatRequest, ChatResponse, FinishReason, RequestOptions, ResponseMetadata, SessionState,
    ToolChoice,
};
use roko_agent::dispatcher::ToolDispatcher;
use roko_agent::streaming::StreamChunk;
use roko_agent::tool_loop::LlmBackend;
use roko_agent::translate::{BackendResponse, RenderedTools, normalize_finish_reason};
use roko_chain::ChainClient;
use roko_core::obs::LogScrubber;
use roko_core::obs::metrics::{MetricSnapshot, MetricValue};
use roko_core::obs::schema::{self, CanonicalMetricSchema, MetricDescriptor, MetricSchema};
use roko_neuro::KnowledgeStore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::registration::{AgentCard, AgentCardEndpoints};

/// Opaque message context payload that round-trips caller JSON as-is.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MessageContext(serde_json::Value);

/// Errors returned by the message dispatch seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchError {
    /// No dispatcher was configured for this request.
    NotConfigured,
    /// Dispatch failed after reaching a configured backend.
    DispatchFailed(String),
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => f.write_str("no configured dispatcher"),
            Self::DispatchFailed(reason) => write!(f, "dispatch failed: {reason}"),
        }
    }
}

impl std::error::Error for DispatchError {}

/// Message dispatch abstraction used by messaging routes.
#[async_trait]
pub trait DispatchLike: Send + Sync {
    /// Dispatch a non-streaming message turn.
    async fn dispatch(&self, request: ChatRequest) -> Result<ChatResponse, DispatchError>;

    /// Dispatch a streaming message turn.
    async fn dispatch_streaming(
        &self,
        request: ChatRequest,
        event_tx: mpsc::UnboundedSender<StreamChunk>,
    ) -> Result<ChatResponse, DispatchError> {
        let _ = event_tx;
        self.dispatch(request).await
    }
}

struct BackendMessageDispatcher {
    backend: Arc<dyn LlmBackend>,
}

impl BackendMessageDispatcher {
    fn new(backend: Arc<dyn LlmBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl DispatchLike for BackendMessageDispatcher {
    async fn dispatch(&self, request: ChatRequest) -> Result<ChatResponse, DispatchError> {
        let messages = request
            .messages
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| DispatchError::DispatchFailed(error.to_string()))?;
        let response = self
            .backend
            .send_turn(
                &messages,
                &RenderedTools::JsonArray(serde_json::json!([])),
                &SessionState::default(),
            )
            .await
            .map_err(|error| DispatchError::DispatchFailed(error.to_string()))?;
        Ok(chat_response_from_backend(&*self.backend, &response))
    }

    async fn dispatch_streaming(
        &self,
        request: ChatRequest,
        event_tx: mpsc::UnboundedSender<StreamChunk>,
    ) -> Result<ChatResponse, DispatchError> {
        let messages = request
            .messages
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| DispatchError::DispatchFailed(error.to_string()))?;
        let response = self
            .backend
            .send_turn_streaming(
                &messages,
                &RenderedTools::JsonArray(serde_json::json!([])),
                &SessionState::default(),
                event_tx,
            )
            .await
            .map_err(|error| DispatchError::DispatchFailed(error.to_string()))?;
        Ok(chat_response_from_backend(&*self.backend, &response))
    }
}

fn chat_response_from_backend(
    backend: &dyn LlmBackend,
    response: &BackendResponse,
) -> ChatResponse {
    let finish_reason = response_finish_reason(response).unwrap_or(FinishReason::Stop);

    ChatResponse {
        content: response.extract_text(),
        reasoning: response.extract_reasoning(),
        tool_calls: Vec::new(),
        usage: response.extract_usage(),
        finish_reason,
        metadata: ResponseMetadata::default(),
        raw_assistant_message: None,
        session: backend.extract_session(response),
    }
}

fn response_finish_reason(response: &BackendResponse) -> Option<FinishReason> {
    match response {
        BackendResponse::Json(value) => value
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            .or_else(|| {
                value
                    .pointer("/candidates/0/finishReason")
                    .and_then(Value::as_str)
            })
            .map(normalize_finish_reason),
        BackendResponse::StreamJson(_) | BackendResponse::Text(_) => None,
    }
}

fn chat_request(prompt: &str, stream: bool) -> ChatRequest {
    ChatRequest {
        messages: vec![
            serde_json::from_value(serde_json::json!({
                "role": "user",
                "content": prompt,
            }))
            .unwrap_or_else(|error| panic!("valid message request: {error}")),
        ],
        model_slug: String::new(),
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop: None,
        stream,
        options: RequestOptions::default(),
    }
}

/// Internal and exported agent metrics.
#[derive(Debug, Default)]
pub struct AgentMetrics {
    request_count: AtomicU64,
    message_count: AtomicU64,
}

impl AgentMetrics {
    /// Record an inbound request.
    pub fn record_request(&self) {
        self.request_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a message request.
    pub fn record_message(&self) {
        self.record_request();
        self.message_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Export current counters.
    #[must_use]
    pub fn snapshot(&self) -> serde_json::Value {
        let request_count = self.request_count.load(Ordering::Relaxed);
        let message_count = self.message_count.load(Ordering::Relaxed);
        serde_json::json!({
            "schema_version": CanonicalMetricSchema::schema_version(),
            "families": [
                counter_snapshot(
                    &schema::ROKO_AGENT_SERVER_REQUESTS_TOTAL_DESCRIPTOR,
                    request_count,
                ),
                counter_snapshot(
                    &schema::ROKO_AGENT_SERVER_MESSAGE_REQUESTS_TOTAL_DESCRIPTOR,
                    message_count,
                ),
            ],
            "requests": request_count,
            "messages": message_count,
        })
    }
}

fn counter_snapshot(descriptor: &MetricDescriptor, value: u64) -> MetricSnapshot {
    debug_assert_eq!(descriptor.kind, roko_core::obs::MetricKind::Counter);
    debug_assert!(descriptor.labels.is_empty());
    MetricSnapshot {
        name: descriptor.name.to_string(),
        help: descriptor.help.to_string(),
        kind: descriptor.kind,
        labels: Vec::new(),
        value: MetricValue::Counter(value),
    }
}

/// Stats payload shaped to resemble the legacy mirage agent stats.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentRuntimeStats {
    /// Number of confirmations.
    pub confirmations_given: u64,
    /// Number of challenges.
    pub challenges_given: u64,
    /// Number of warnings.
    pub warnings_posted: u64,
    /// Number of insights posted.
    pub insights_posted: u64,
    /// Number of tasks completed.
    pub tasks_completed: u64,
    /// Number of failed tasks.
    pub tasks_failed: u64,
    /// Number of cognitive cycles.
    pub delta_cycles: u64,
    /// Total cost in USD.
    pub total_cost_usd: f64,
    /// Total token usage.
    pub total_tokens: u64,
}

/// Minimal prediction record exposed by the predictions feature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPrediction {
    /// Unique identifier.
    pub id: String,
    /// Source agent identifier.
    pub agent_id: String,
    /// Market or question label.
    pub market: String,
    /// Free-form category.
    #[serde(default)]
    pub category: String,
    /// Direction label.
    pub direction: String,
    /// Confidence score.
    pub confidence: f64,
    /// Predicted numeric value.
    #[serde(default)]
    pub predicted_value: f64,
    /// Optional interval width.
    #[serde(default)]
    pub interval_width: f64,
    /// Optional observed value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_value: Option<f64>,
    /// Unix timestamp.
    pub ts: u64,
}

/// Request payload for `POST /predictions`.
#[derive(Debug, Clone, Deserialize)]
pub struct PredictionCreateRequest {
    /// Market or question label.
    pub market: String,
    /// Direction label.
    pub direction: String,
    /// Confidence score.
    #[serde(default)]
    pub confidence: f64,
    /// Optional category.
    #[serde(default)]
    pub category: String,
    /// Optional numeric prediction.
    #[serde(default)]
    pub predicted_value: f64,
    /// Optional interval width.
    #[serde(default)]
    pub interval_width: f64,
    /// Optional observed value.
    #[serde(default)]
    pub actual_value: Option<f64>,
}

/// Per-prediction residual summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPredictionResidual {
    /// Prediction identifier.
    pub id: String,
    /// Residual value.
    pub residual: f64,
}

/// Request payload for the research route.
#[derive(Debug, Clone, Deserialize)]
pub struct ResearchRequest {
    /// Topic to investigate.
    pub topic: String,
    /// Depth hint.
    #[serde(default = "default_research_depth")]
    pub depth: String,
}

fn default_research_depth() -> String {
    "shallow".to_string()
}

/// Research response payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchResponse {
    /// Main findings.
    pub findings: Vec<String>,
    /// Source descriptors.
    pub sources: Vec<String>,
}

/// Task priority labels.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    /// Low priority.
    Low,
    /// Medium priority.
    #[default]
    Medium,
    /// High priority.
    High,
    /// Critical priority.
    Critical,
}

/// Task lifecycle labels.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// Open for work.
    #[default]
    Open,
    /// Accepted by the agent.
    Accepted,
    /// Completed successfully.
    Completed,
}

/// Task artifact payload.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskArtifact {
    /// Artifact kind.
    #[serde(default)]
    pub kind: String,
    /// Artifact label.
    #[serde(default)]
    pub label: String,
    /// Stable content hash.
    #[serde(default)]
    pub content_hash: String,
    /// Optional URI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// Condensed task summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskSummary {
    /// Human-readable completion summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Agent task entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEntry {
    /// Unique task identifier.
    pub id: u64,
    /// Task title.
    pub title: String,
    /// Task kind.
    #[serde(default)]
    pub kind: String,
    /// Priority.
    #[serde(default)]
    pub priority: TaskPriority,
    /// Lifecycle state.
    #[serde(default)]
    pub state: TaskState,
    /// Optional bounty amount.
    #[serde(default)]
    pub bounty: u64,
    /// Optional assigned agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// Creation timestamp.
    pub created_at: u64,
    /// Optional completion timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    /// Optional artifacts.
    #[serde(default)]
    pub artifacts: Vec<TaskArtifact>,
    /// Optional summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Completion payload for a task.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TaskCompletionRequest {
    /// Produced artifacts.
    #[serde(default)]
    pub artifacts: Vec<TaskArtifact>,
    /// Optional completion summary.
    #[serde(default, alias = "proof")]
    pub summary: Option<String>,
}

/// Shared state for agent-server routes.
pub struct AgentState {
    agent_id: String,
    owner: Option<String>,
    version: String,
    capabilities: Vec<String>,
    log_path: PathBuf,
    routes: Vec<String>,
    started_at: Instant,
    registered_at: u64,
    chain_client: Option<Arc<dyn ChainClient>>,
    #[allow(dead_code)]
    llm_backend: Option<Arc<dyn LlmBackend>>,
    message_dispatcher: Option<Arc<dyn DispatchLike>>,
    dispatcher: Option<Arc<ToolDispatcher>>,
    #[allow(dead_code)]
    knowledge_store: Option<Arc<KnowledgeStore>>,
    predictions: Mutex<Vec<AgentPrediction>>,
    tasks: Mutex<VecDeque<TaskEntry>>,
    stats: Mutex<AgentRuntimeStats>,
    metrics: AgentMetrics,
}

impl AgentState {
    /// Build a new shared state instance.
    #[must_use]
    pub fn new(
        agent_id: String,
        owner: Option<String>,
        version: String,
        capabilities: Vec<String>,
        chain_client: Option<Arc<dyn ChainClient>>,
        llm_backend: Option<Arc<dyn LlmBackend>>,
        knowledge_store: Option<Arc<KnowledgeStore>>,
    ) -> Self {
        let routes = build_routes(&capabilities);
        let message_dispatcher = llm_backend.as_ref().map(|backend| {
            Arc::new(BackendMessageDispatcher::new(Arc::clone(backend))) as Arc<dyn DispatchLike>
        });
        let log_path = default_log_path(&agent_id);
        Self {
            agent_id,
            owner,
            version,
            capabilities,
            log_path,
            routes,
            started_at: Instant::now(),
            registered_at: now_secs(),
            chain_client,
            llm_backend,
            message_dispatcher,
            dispatcher: None,
            knowledge_store,
            predictions: Mutex::new(Vec::new()),
            tasks: Mutex::new(VecDeque::new()),
            stats: Mutex::new(AgentRuntimeStats::default()),
            metrics: AgentMetrics::default(),
        }
    }

    /// Return the configured agent identifier.
    #[must_use]
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Return the start instant.
    #[must_use]
    pub const fn started_at(&self) -> Instant {
        self.started_at
    }

    /// Return the path used for the sidecar log file.
    #[must_use]
    pub fn log_path(&self) -> &Path {
        self.log_path.as_path()
    }

    /// Append one scrubbed line to the sidecar log file.
    pub async fn append_log_line(&self, line: impl Into<String>) {
        let log_path = self.log_path.clone();
        let display_path = self.log_path.display().to_string();
        let line = line.into();

        match tokio::task::spawn_blocking(move || append_log_line_sync(&log_path, &line)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(path = %display_path, %error, "failed to append sidecar log line");
            }
            Err(error) => {
                tracing::warn!(path = %display_path, %error, "sidecar log append task failed");
            }
        }
    }

    /// Borrow the metrics registry.
    #[must_use]
    pub const fn metrics(&self) -> &AgentMetrics {
        &self.metrics
    }

    /// Borrow the configured LLM backend, if one is attached.
    #[must_use]
    pub fn llm_backend(&self) -> Option<&Arc<dyn LlmBackend>> {
        self.llm_backend.as_ref()
    }

    /// Borrow the configured message dispatcher, if one is attached.
    #[must_use]
    pub fn message_dispatcher(&self) -> Option<Arc<dyn DispatchLike>> {
        self.message_dispatcher.as_ref().map(Arc::clone)
    }

    /// Dispatch one non-streaming prompt through the configured message seam.
    ///
    /// # Errors
    ///
    /// Returns an error when no dispatcher is configured or when the selected
    /// backend fails to complete the turn.
    pub async fn dispatch_prompt(&self, prompt: &str) -> Result<ChatResponse, DispatchError> {
        self.metrics.record_message();
        let dispatcher = self
            .message_dispatcher()
            .ok_or(DispatchError::NotConfigured)?;
        let response = dispatcher.dispatch(chat_request(prompt, false)).await;
        let status = if response.is_ok() { "ok" } else { "error" };
        self.append_log_line(format!("message prompt={prompt:?} status={status}"))
            .await;
        response
    }

    /// Attach a dispatcher used to service message routes.
    #[must_use]
    pub fn with_message_dispatcher(mut self, dispatcher: Arc<dyn DispatchLike>) -> Self {
        self.message_dispatcher = Some(dispatcher);
        self
    }

    /// Override the path used for the sidecar log file.
    #[must_use]
    pub fn with_log_path(mut self, log_path: impl Into<PathBuf>) -> Self {
        self.log_path = log_path.into();
        self
    }

    /// Attach a dispatcher used to service agent messages.
    #[must_use]
    pub fn with_dispatcher(mut self, dispatcher: Arc<ToolDispatcher>) -> Self {
        self.dispatcher = Some(dispatcher);
        self
    }

    /// Borrow the configured dispatcher, if one is attached.
    #[must_use]
    pub const fn dispatcher(&self) -> Option<&Arc<ToolDispatcher>> {
        self.dispatcher.as_ref()
    }

    /// Build the public capabilities manifest.
    #[must_use]
    pub fn capabilities_manifest(&self) -> serde_json::Value {
        let skills = self
            .capabilities
            .iter()
            .map(|capability| {
                (
                    capability.clone(),
                    serde_json::json!({
                        "enabled": true,
                        "config": {},
                    }),
                )
            })
            .collect::<serde_json::Map<String, serde_json::Value>>();

        serde_json::json!({
            "agent_id": self.agent_id,
            "features": self.capabilities,
            "routes": self.routes,
            "owner": self.owner,
            "registered_at": self.registered_at,
            "skills": skills,
            "stats": self.stats_payload(),
        })
    }

    /// Export the current stats payload.
    #[must_use]
    pub fn stats_payload(&self) -> serde_json::Value {
        let stats = self.stats.lock().clone();
        serde_json::json!({
            "agent_id": self.agent_id,
            "owner": self.owner,
            "confirmations_given": stats.confirmations_given,
            "challenges_given": stats.challenges_given,
            "warnings_posted": stats.warnings_posted,
            "insights_posted": stats.insights_posted,
            "tasks_completed": stats.tasks_completed,
            "tasks_failed": stats.tasks_failed,
            "delta_cycles": stats.delta_cycles,
            "total_cost_usd": stats.total_cost_usd,
            "total_tokens": stats.total_tokens,
            "registered_at": self.registered_at,
            "operating_frequency": operating_frequency(stats.tasks_completed + stats.tasks_failed),
            "metrics": self.metrics.snapshot(),
            "chain_backend": self.chain_client.as_ref().map(|client| client.name().to_string()),
        })
    }

    /// Build the agent card corresponding to this runtime.
    #[must_use]
    pub fn build_agent_card(&self, addr: std::net::SocketAddr) -> AgentCard {
        let host = if addr.ip().is_unspecified() {
            "127.0.0.1".to_string()
        } else {
            addr.ip().to_string()
        };
        let rest = format!("http://{host}:{}", addr.port());
        let websocket = format!("ws://{host}:{}/stream", addr.port());
        AgentCard {
            name: self.agent_id.clone(),
            capabilities: self.capabilities.clone(),
            endpoints: AgentCardEndpoints {
                rest: Some(rest),
                websocket: Some(websocket),
                a2a: None,
                mcp: None,
            },
            domain_tags: vec!["roko".to_string()],
            version: self.version.clone(),
        }
    }

    /// Create a prediction entry.
    #[allow(clippy::unused_async)]
    pub async fn create_prediction(&self, request: PredictionCreateRequest) -> AgentPrediction {
        self.metrics.record_request();
        let prediction = AgentPrediction {
            id: format!("pred-{}", Uuid::new_v4()),
            agent_id: self.agent_id.clone(),
            market: request.market,
            category: request.category,
            direction: request.direction,
            confidence: request.confidence,
            predicted_value: request.predicted_value,
            interval_width: request.interval_width,
            actual_value: request.actual_value,
            ts: now_secs(),
        };
        self.predictions.lock().push(prediction.clone());
        prediction
    }

    /// Return all stored predictions.
    #[allow(clippy::unused_async)]
    pub async fn list_predictions(&self) -> Vec<AgentPrediction> {
        self.metrics.record_request();
        self.predictions.lock().clone()
    }

    /// Fetch a prediction by identifier.
    #[allow(clippy::unused_async)]
    pub async fn get_prediction(&self, id: &str) -> Option<AgentPrediction> {
        self.metrics.record_request();
        self.predictions
            .lock()
            .iter()
            .find(|prediction| prediction.id == id)
            .cloned()
    }

    /// Summarize prediction residuals.
    #[allow(
        clippy::cast_precision_loss,
        clippy::significant_drop_tightening,
        clippy::unused_async
    )]
    pub async fn prediction_residuals(&self) -> serde_json::Value {
        self.metrics.record_request();
        let predictions = self.predictions.lock();
        let residuals: Vec<AgentPredictionResidual> = predictions
            .iter()
            .filter_map(|prediction| {
                prediction
                    .actual_value
                    .map(|actual| AgentPredictionResidual {
                        id: prediction.id.clone(),
                        residual: (prediction.predicted_value - actual).abs(),
                    })
            })
            .collect();
        let mse = if residuals.is_empty() {
            0.0
        } else {
            residuals
                .iter()
                .map(|residual| residual.residual.powi(2))
                .sum::<f64>()
                / residuals.len() as f64
        };
        let hit_rate = if residuals.is_empty() {
            0.0
        } else {
            residuals
                .iter()
                .filter(|residual| residual.residual <= 0.1)
                .count() as f64
                / residuals.len() as f64
        };
        serde_json::json!({
            "mse": mse,
            "hit_rate": hit_rate,
            "residuals": residuals,
        })
    }

    /// Execute a simple research request against the local state.
    #[allow(clippy::unused_async)]
    pub async fn research(&self, request: ResearchRequest) -> ResearchResponse {
        self.metrics.record_request();
        let depth = request.depth;
        ResearchResponse {
            findings: vec![
                format!("{} reviewed topic '{}'", self.agent_id, request.topic),
                format!("requested depth: {depth}"),
            ],
            sources: vec![
                format!("agent://{}/capabilities", self.agent_id),
                self.chain_client.as_ref().map_or_else(
                    || "chain://unconfigured".to_string(),
                    |client| format!("chain://{}", client.name()),
                ),
            ],
        }
    }

    /// Return the current task queue.
    #[allow(clippy::unused_async)]
    pub async fn list_tasks(&self) -> Vec<TaskEntry> {
        self.metrics.record_request();
        self.tasks.lock().iter().cloned().collect()
    }

    /// Accept a task by identifier.
    #[allow(clippy::significant_drop_tightening, clippy::unused_async)]
    pub async fn accept_task(&self, id: u64) -> Option<TaskEntry> {
        self.metrics.record_request();
        let mut tasks = self.tasks.lock();
        let task = tasks.iter_mut().find(|task| task.id == id)?;
        task.state = TaskState::Accepted;
        task.assignee = Some(self.agent_id.clone());
        Some(task.clone())
    }

    /// Complete a task by identifier.
    #[allow(clippy::significant_drop_tightening, clippy::unused_async)]
    pub async fn complete_task(
        &self,
        id: u64,
        request: TaskCompletionRequest,
    ) -> Option<TaskEntry> {
        self.metrics.record_request();
        let mut tasks = self.tasks.lock();
        let task = tasks.iter_mut().find(|task| task.id == id)?;
        task.state = TaskState::Completed;
        task.completed_at = Some(now_secs());
        task.artifacts = request.artifacts;
        task.summary = request.summary;
        self.stats.lock().tasks_completed += 1;
        Some(task.clone())
    }

    /// Seed a task for test or bootstrap use.
    pub fn push_task(&self, title: impl Into<String>, kind: impl Into<String>) -> TaskEntry {
        let mut tasks = self.tasks.lock();
        let id = tasks.len() as u64 + 1;
        let entry = TaskEntry {
            id,
            title: title.into(),
            kind: kind.into(),
            priority: TaskPriority::Medium,
            state: TaskState::Open,
            bounty: 0,
            assignee: None,
            created_at: now_secs(),
            completed_at: None,
            artifacts: Vec::new(),
            summary: None,
        };
        tasks.push_back(entry.clone());
        entry
    }
}

fn build_routes(capabilities: &[String]) -> Vec<String> {
    let mut routes = vec![
        "/health".to_string(),
        "/capabilities".to_string(),
        "/stats".to_string(),
        "/logs".to_string(),
    ];
    if capabilities
        .iter()
        .any(|capability| capability == "messaging")
    {
        routes.push("/message".to_string());
        routes.push("/stream".to_string());
    }
    if capabilities
        .iter()
        .any(|capability| capability == "predictions")
    {
        routes.push("/predictions".to_string());
        routes.push("/predictions/{id}".to_string());
        routes.push("/predictions/residuals".to_string());
    }
    if capabilities
        .iter()
        .any(|capability| capability == "research")
    {
        routes.push("/research".to_string());
    }
    if capabilities.iter().any(|capability| capability == "tasks") {
        routes.push("/tasks".to_string());
        routes.push("/tasks/{id}/accept".to_string());
        routes.push("/tasks/{id}/complete".to_string());
    }
    routes
}

const fn operating_frequency(task_count: u64) -> &'static str {
    match task_count {
        0 => "idle",
        1..=2 => "reactive",
        3..=5 => "active",
        _ => "intensive",
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn default_log_path(agent_id: &str) -> PathBuf {
    PathBuf::from(".roko")
        .join("agents")
        .join(agent_id)
        .join("log")
}

fn append_log_line_sync(path: &Path, line: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let scrubbed = LogScrubber::default().scrub(line);
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{scrubbed}")?;
    file.flush()
}
