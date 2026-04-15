//! Shared in-memory state for per-agent routes.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use roko_agent::tool_loop::LlmBackend;
use roko_chain::ChainClient;
use roko_neuro::KnowledgeStore;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::registration::{AgentCard, AgentCardEndpoints};

/// Opaque message context payload.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageContext {
    /// Raw extra JSON carried by the caller.
    #[serde(default)]
    pub extra: serde_json::Value,
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
        serde_json::json!({
            "requests": self.request_count.load(Ordering::Relaxed),
            "messages": self.message_count.load(Ordering::Relaxed),
        })
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    /// Low priority.
    Low,
    /// Medium priority.
    Medium,
    /// High priority.
    High,
    /// Critical priority.
    Critical,
}

impl Default for TaskPriority {
    fn default() -> Self {
        Self::Medium
    }
}

/// Task lifecycle labels.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// Open for work.
    Open,
    /// Accepted by the agent.
    Accepted,
    /// Completed successfully.
    Completed,
}

impl Default for TaskState {
    fn default() -> Self {
        Self::Open
    }
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
    routes: Vec<String>,
    started_at: Instant,
    registered_at: u64,
    chain_client: Option<Arc<dyn ChainClient>>,
    #[allow(dead_code)]
    llm_backend: Option<Arc<dyn LlmBackend>>,
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
        Self {
            agent_id,
            owner,
            version,
            capabilities,
            routes,
            started_at: Instant::now(),
            registered_at: now_secs(),
            chain_client,
            llm_backend,
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

    /// Borrow the metrics registry.
    #[must_use]
    pub const fn metrics(&self) -> &AgentMetrics {
        &self.metrics
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
    pub async fn list_predictions(&self) -> Vec<AgentPrediction> {
        self.metrics.record_request();
        self.predictions.lock().clone()
    }

    /// Fetch a prediction by identifier.
    pub async fn get_prediction(&self, id: &str) -> Option<AgentPrediction> {
        self.metrics.record_request();
        self.predictions
            .lock()
            .iter()
            .find(|prediction| prediction.id == id)
            .cloned()
    }

    /// Summarize prediction residuals.
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
    pub async fn list_tasks(&self) -> Vec<TaskEntry> {
        self.metrics.record_request();
        self.tasks.lock().iter().cloned().collect()
    }

    /// Accept a task by identifier.
    pub async fn accept_task(&self, id: u64) -> Option<TaskEntry> {
        self.metrics.record_request();
        let mut tasks = self.tasks.lock();
        let task = tasks.iter_mut().find(|task| task.id == id)?;
        task.state = TaskState::Accepted;
        task.assignee = Some(self.agent_id.clone());
        Some(task.clone())
    }

    /// Complete a task by identifier.
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

fn operating_frequency(task_count: u64) -> &'static str {
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
