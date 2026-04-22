//! Shared application state for the HTTP server.
//!
//! [`AppState`] is wrapped in `Arc` and injected into every axum handler via
//! the [`axum::extract::State`] extractor. It holds the working directory,
//! layout, configuration, runtime services, and tracking maps for active
//! runs, plans, and operations.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use arc_swap::ArcSwap;
use base64::Engine;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{OnceCell, RwLock};
use tokio::task::JoinHandle;
use uuid::Uuid;

use roko_core::config::schema::RokoConfig;
use roko_core::obs::LogScrubber;
use roko_core::{Engram, Substrate};
use roko_daimon::{DaimonState, StrategySpaceDefinition};
use roko_learn::latency::LatencyRegistry;
use roko_learn::provider_health::ProviderHealthTracker;
use roko_runtime::cancel::CancelToken;
use roko_runtime::process::ProcessSupervisor;

use crate::deploy::{DeployBackend, Deployment};
use crate::dispatch::SubscriptionRegistry;
use crate::event_bus::EventBus;
use crate::runtime::CliRuntime;
use crate::runtime::RunResult;
use roko_core::obs::metrics::MetricRegistry;
use roko_fs::FileSubstrate;
use roko_fs::layout::RokoLayout;

use crate::events::ServerEvent;
use crate::templates::TemplateRegistry;

fn affect_state_path(layout_root: &Path) -> PathBuf {
    layout_root.join("daimon").join("affect.json")
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Re-export the shared endpoint type from roko-core.
pub use roko_core::AgentEndpoints;

/// Agent discovery entry used by the serve-side aggregator.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscoveredAgent {
    /// Stable agent identifier.
    pub agent_id: String,
    /// Optional process label from the supervisor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Optional local process id when the agent is supervised here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u64>,
    /// Optional owner identity.
    #[serde(default)]
    pub owner: String,
    /// Registration timestamp.
    #[serde(default)]
    pub registered_at: u64,
    /// Last refresh timestamp.
    #[serde(default)]
    pub last_seen_at: u64,
    /// Known endpoint set.
    #[serde(default)]
    pub endpoints: AgentEndpoints,
    /// Optional ERC-8004 card URI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_uri: Option<String>,
    /// Advertised capabilities.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Domain tags.
    #[serde(default)]
    pub domain_tags: Vec<String>,
    /// Agent tier (e.g. "Unverified", "Verified", "Trusted", "Expert", "Pioneer").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    /// Agent reputation score (0–100).
    #[serde(default)]
    pub reputation: u32,
    /// Skill tags for matchmaking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    /// Number of previously completed jobs.
    #[serde(default)]
    pub past_jobs_completed: u32,
    /// Maximum concurrent jobs this agent accepts.
    #[serde(default)]
    pub max_concurrent_jobs: u32,
    /// Token hash used by the agent server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_hash: Option<String>,
    /// Token expiry timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Coarse liveness label.
    #[serde(default = "default_agent_status")]
    pub status: String,
    /// Ephemeral plaintext token retained only for active proxy fan-out.
    #[serde(skip)]
    pub proxy_token: Option<String>,
}

fn default_agent_status() -> String {
    "discovered".to_string()
}

/// Upsert payload for the discovery registry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentRegistrationRecord {
    /// Stable agent identifier.
    pub agent_id: String,
    /// Optional process label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Optional local process id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u64>,
    /// Optional owner.
    #[serde(default)]
    pub owner: String,
    /// Endpoints advertised by the agent.
    #[serde(default)]
    pub endpoints: AgentEndpoints,
    /// Optional ERC-8004 card URI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_uri: Option<String>,
    /// Capability list.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Domain tags.
    #[serde(default)]
    pub domain_tags: Vec<String>,
    /// Agent tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    /// Agent reputation score.
    #[serde(default)]
    pub reputation: u32,
    /// Skill tags.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Number of previously completed jobs.
    #[serde(default)]
    pub past_jobs_completed: u32,
    /// Maximum concurrent jobs.
    #[serde(default)]
    pub max_concurrent_jobs: u32,
}

/// Returned when a new token is issued or rotated.
#[derive(Debug, Clone, Serialize)]
pub struct IssuedAgentToken {
    /// Agent identifier.
    pub agent_id: String,
    /// Plaintext token returned once to the caller.
    pub token: String,
    /// Token expiry timestamp.
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// Public status payload for an agent token.
#[derive(Debug, Clone, Serialize)]
pub struct AgentTokenStatus {
    /// Agent identifier.
    pub agent_id: String,
    /// Whether a token exists.
    pub exists: bool,
    /// Token expiry timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Cached JSON payload stored for short-lived aggregator responses.
#[derive(Debug, Clone)]
pub struct CachedJsonValue {
    expires_at: Instant,
    value: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Handle types for tracked async work
// ---------------------------------------------------------------------------

/// A tracked one-shot `roko run` invocation.
pub struct RunHandle {
    /// Unique identifier for this run.
    pub id: String,
    /// The prompt that was submitted.
    pub prompt: String,
    /// Current execution status.
    pub status: OperationStatus,
    /// Final result payload once the run has completed.
    pub result: Option<RunResult>,
    /// Background task driving the run.
    pub handle: JoinHandle<()>,
}

/// A tracked plan execution.
pub struct PlanHandle {
    /// Unique identifier for this plan run.
    pub id: String,
    /// Directory containing the plan files.
    pub plan_dir: PathBuf,
    /// Current execution status.
    pub status: OperationStatus,
    /// Background task driving the plan runner.
    pub handle: JoinHandle<()>,
}

/// A tracked generic operation (PRD draft, research, etc.).
pub struct OperationHandle {
    /// Unique identifier for this operation.
    pub id: String,
    /// Operation kind (e.g. `"prd_draft"`, `"research"`).
    pub kind: String,
    /// Current execution status.
    pub status: OperationStatus,
    /// Background task driving the operation.
    pub handle: JoinHandle<()>,
}

/// Status of an async operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum OperationStatus {
    /// The operation is still executing.
    Running,
    /// The operation finished successfully.
    Completed {
        /// Optional result payload.
        result: Option<String>,
    },
    /// The operation failed.
    Failed {
        /// Error description.
        error: String,
    },
}

/// A recorded template run outcome used by the metrics summary endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateRunRecord {
    /// When the run completed.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// What triggered the run (e.g. `template_deploy`, `worker_callback`).
    #[serde(default)]
    pub trigger_kind: String,
    /// Whether the run succeeded.
    pub success: bool,
}

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

/// Shared server state, wrapped in `Arc` for handler access.
pub struct AppState {
    /// Project working directory.
    pub workdir: PathBuf,
    /// `.roko/` directory layout helper.
    pub layout: RokoLayout,
    /// Lazily initialized `.roko/engrams.jsonl` writer.
    pub signal_store: SignalStore,
    /// Cancellation token for graceful shutdown.
    pub cancel: CancelToken,
    /// Monotonic timestamp when the server state was created.
    pub started_at: Instant,
    /// Prometheus-style metric registry.
    pub metrics: Arc<MetricRegistry>,
    /// Process lifecycle manager.
    pub supervisor: Arc<ProcessSupervisor>,
    /// Affect engine used to stamp PAD vectors onto persisted episodes.
    pub affect_engine: Mutex<DaimonState>,
    /// Event bus for streaming server events to clients.
    pub event_bus: EventBus<ServerEvent>,
    /// Unified state hub for dashboard snapshot + event streaming.
    pub state_hub: roko_core::SharedStateHub,
    /// Event subscriptions loaded at startup.
    pub subscriptions: SubscriptionRegistry,
    /// Runtime bridge to CLI operations (run_once, status, dashboard).
    pub runtime: Arc<dyn CliRuntime>,
    /// Full `roko.toml` schema configuration with lock-free reads.
    pub roko_config: ArcSwap<RokoConfig>,
    /// In-memory provider health tracker exposed via serve APIs.
    pub provider_health: ProviderHealthTracker,
    /// In-memory provider latency stats exposed via serve APIs.
    pub latency_registry: LatencyRegistry,
    /// Active one-shot runs.
    pub active_runs: RwLock<HashMap<String, RunHandle>>,
    /// Active plan executions.
    pub active_plans: RwLock<HashMap<String, PlanHandle>>,
    /// Active generic operations.
    pub operations: RwLock<HashMap<String, OperationHandle>>,
    /// Agent template registry.
    pub templates: RwLock<TemplateRegistry>,
    /// Cloud deploy backend (Railway, CLI, manual).
    pub deploy_backend: Arc<dyn DeployBackend>,
    /// Active cloud deployments.
    pub deployments: RwLock<HashMap<String, Deployment>>,
    /// Recent template run outcomes keyed by template name.
    pub template_runs: RwLock<HashMap<String, Vec<TemplateRunRecord>>>,
    /// Secret scrubber for redacting API-key / token patterns from responses.
    pub scrubber: Arc<LogScrubber>,
    /// Shared HTTP client for aggregator fan-out.
    pub http_client: reqwest::Client,
    /// Discovery registry for local and chain-discovered agents.
    pub discovered_agents: RwLock<HashMap<String, DiscoveredAgent>>,
    /// Short-lived aggregator cache keyed by route + query signature.
    pub aggregator_cache: RwLock<HashMap<String, CachedJsonValue>>,
    /// Ring buffer of recent heartbeat payloads.
    pub heartbeats: RwLock<VecDeque<roko_core::HeartbeatPayload>>,
}

impl AppState {
    /// Construct a new `AppState` from the working directory and loaded configs.
    pub fn new(
        workdir: PathBuf,
        runtime: Arc<dyn CliRuntime>,
        roko_config: RokoConfig,
        deploy_backend: Arc<dyn DeployBackend>,
    ) -> Self {
        Self::new_with_daimon_strategy(
            workdir,
            runtime,
            roko_config,
            deploy_backend,
            StrategySpaceDefinition::default(),
        )
    }

    /// Construct a new `AppState` with an explicit Daimon strategy-space definition.
    pub fn new_with_daimon_strategy(
        workdir: PathBuf,
        runtime: Arc<dyn CliRuntime>,
        roko_config: RokoConfig,
        deploy_backend: Arc<dyn DeployBackend>,
        strategy_space: StrategySpaceDefinition,
    ) -> Self {
        let layout = RokoLayout::for_project(&workdir);
        let signal_root = layout.root().to_path_buf();
        let affect_path = affect_state_path(layout.root());
        let cancel = CancelToken::new();
        let supervisor = Arc::new(ProcessSupervisor::new(cancel.child()));
        let subscriptions = SubscriptionRegistry::load_from_project(&workdir, &roko_config);
        let mut affect_engine = DaimonState::load_or_new(&affect_path);
        affect_engine.configure_strategy_space(strategy_space);

        let mut template_registry = TemplateRegistry::new(workdir.clone());
        template_registry.scan();

        Self {
            workdir,
            layout,
            signal_store: SignalStore::new(signal_root),
            cancel,
            started_at: Instant::now(),
            metrics: Arc::new(MetricRegistry::new()),
            supervisor,
            affect_engine: Mutex::new(affect_engine),
            event_bus: EventBus::new(16_384),
            state_hub: roko_core::shared_state_hub(),
            subscriptions,
            runtime,
            roko_config: ArcSwap::from_pointee(roko_config),
            provider_health: ProviderHealthTracker::new(),
            latency_registry: LatencyRegistry::new(),
            active_runs: RwLock::new(HashMap::new()),
            active_plans: RwLock::new(HashMap::new()),
            operations: RwLock::new(HashMap::new()),
            templates: RwLock::new(template_registry),
            deploy_backend,
            deployments: RwLock::new(HashMap::new()),
            template_runs: RwLock::new(HashMap::new()),
            scrubber: Arc::new(LogScrubber::new()),
            http_client: reqwest::Client::new(),
            discovered_agents: RwLock::new(HashMap::new()),
            aggregator_cache: RwLock::new(HashMap::new()),
            heartbeats: RwLock::new(VecDeque::new()),
        }
    }

    /// Load the current config snapshot.
    #[must_use]
    pub fn load_roko_config(&self) -> Arc<RokoConfig> {
        self.roko_config.load_full()
    }

    /// Atomically swap in a new config snapshot.
    pub fn store_roko_config(&self, roko_config: RokoConfig) {
        self.roko_config.store(Arc::new(roko_config));
    }

    /// Initiate graceful shutdown: cancel all work and stop supervised processes.
    pub async fn shutdown(&self) {
        tracing::info!("server shutdown initiated");
        if let Err(err) = self.save_snapshot().await {
            tracing::warn!(error = %err, "failed to save server state on shutdown");
        }
        self.cancel.cancel();
        self.supervisor.shutdown_all().await;
        self.event_bus.publish(ServerEvent::ServerShutdown);
    }

    fn snapshot_path(&self) -> PathBuf {
        self.workdir
            .join(".roko")
            .join("state")
            .join("server-state.json")
    }

    /// Persist discovered agents and template run records to disk (atomic write).
    pub async fn save_snapshot(&self) -> anyhow::Result<()> {
        let agents: HashMap<String, DiscoveredAgent> = self.discovered_agents.read().await.clone();
        let runs: HashMap<String, Vec<TemplateRunRecord>> = self.template_runs.read().await.clone();
        let snapshot = ServerStateSnapshot {
            discovered_agents: agents,
            template_runs: runs,
        };
        let json = serde_json::to_string_pretty(&snapshot)?;
        let target = self.snapshot_path();
        let parent = target
            .parent()
            .ok_or_else(|| anyhow::anyhow!("invalid snapshot path"))?;
        tokio::fs::create_dir_all(parent).await?;
        let tmp = target.with_extension("json.tmp");
        tokio::fs::write(&tmp, json).await?;
        tokio::fs::rename(&tmp, &target).await?;
        tracing::debug!(path = %target.display(), "server state snapshot saved");
        Ok(())
    }

    /// Restore discovered agents and template run records from disk.
    pub async fn restore_snapshot(&self) -> anyhow::Result<()> {
        let path = self.snapshot_path();
        if !path.exists() {
            tracing::debug!("no server state snapshot found; starting fresh");
            return Ok(());
        }
        let data = tokio::fs::read_to_string(&path).await?;
        let snapshot: ServerStateSnapshot = serde_json::from_str(&data)?;
        {
            let mut agents = self.discovered_agents.write().await;
            for (id, agent) in snapshot.discovered_agents {
                agents.entry(id).or_insert(agent);
            }
        }
        {
            let mut runs = self.template_runs.write().await;
            for (name, records) in snapshot.template_runs {
                runs.entry(name).or_insert(records);
            }
        }
        tracing::info!(path = %path.display(), "restored server state from snapshot");
        Ok(())
    }

    /// Insert or update a discovery entry and return the stored snapshot.
    pub async fn upsert_discovered_agent(
        &self,
        registration: AgentRegistrationRecord,
    ) -> DiscoveredAgent {
        let now = now_unix_secs();
        let mut agents = self.discovered_agents.write().await;
        let entry = agents
            .entry(registration.agent_id.clone())
            .or_insert_with(|| DiscoveredAgent {
                agent_id: registration.agent_id.clone(),
                label: registration.label.clone(),
                process_id: registration.process_id,
                owner: registration.owner.clone(),
                registered_at: now,
                last_seen_at: now,
                endpoints: registration.endpoints.clone(),
                card_uri: registration.card_uri.clone(),
                capabilities: registration.capabilities.clone(),
                domain_tags: registration.domain_tags.clone(),
                tier: registration.tier.clone(),
                reputation: registration.reputation,
                skills: registration.skills.clone(),
                past_jobs_completed: registration.past_jobs_completed,
                max_concurrent_jobs: registration.max_concurrent_jobs,
                token_hash: None,
                token_expires_at: None,
                status: "registered".to_string(),
                proxy_token: None,
            });

        entry.label = registration.label.or_else(|| entry.label.clone());
        entry.process_id = registration.process_id.or(entry.process_id);
        if !registration.owner.is_empty() {
            entry.owner = registration.owner;
        }
        if registration.endpoints.rest.is_some() {
            entry.endpoints.rest = registration.endpoints.rest;
        }
        if registration.endpoints.websocket.is_some() {
            entry.endpoints.websocket = registration.endpoints.websocket;
        }
        if registration.endpoints.a2a.is_some() {
            entry.endpoints.a2a = registration.endpoints.a2a;
        }
        if registration.endpoints.mcp.is_some() {
            entry.endpoints.mcp = registration.endpoints.mcp;
        }
        if registration.card_uri.is_some() {
            entry.card_uri = registration.card_uri;
        }
        if !registration.capabilities.is_empty() {
            entry.capabilities = registration.capabilities;
        }
        if !registration.domain_tags.is_empty() {
            entry.domain_tags = registration.domain_tags;
        }
        if registration.tier.is_some() {
            entry.tier = registration.tier;
        }
        if registration.reputation > 0 {
            entry.reputation = registration.reputation;
        }
        if !registration.skills.is_empty() {
            entry.skills = registration.skills;
        }
        if registration.past_jobs_completed > 0 {
            entry.past_jobs_completed = registration.past_jobs_completed;
        }
        if registration.max_concurrent_jobs > 0 {
            entry.max_concurrent_jobs = registration.max_concurrent_jobs;
        }
        entry.last_seen_at = now;
        entry.status = "registered".to_string();
        entry.clone()
    }

    /// List all known discovery entries.
    pub async fn list_discovered_agents(&self) -> Vec<DiscoveredAgent> {
        self.discovered_agents
            .read()
            .await
            .values()
            .cloned()
            .collect()
    }

    /// Fetch one discovery entry.
    pub async fn discovered_agent(&self, agent_id: &str) -> Option<DiscoveredAgent> {
        self.discovered_agents.read().await.get(agent_id).cloned()
    }

    /// Store a refreshed discovery entry.
    pub async fn store_discovered_agent(&self, agent: DiscoveredAgent) {
        self.discovered_agents
            .write()
            .await
            .insert(agent.agent_id.clone(), agent);
    }

    /// Return a public token status snapshot for an agent.
    pub async fn agent_token_status(&self, agent_id: &str) -> Option<AgentTokenStatus> {
        self.discovered_agent(agent_id)
            .await
            .map(|agent| AgentTokenStatus {
                agent_id: agent.agent_id,
                exists: agent.token_hash.is_some(),
                expires_at: agent.token_expires_at,
            })
    }

    /// Issue or rotate the bearer token for an agent.
    pub async fn rotate_agent_token(&self, agent_id: &str) -> Option<IssuedAgentToken> {
        let mut agents = self.discovered_agents.write().await;
        let agent = agents.get_mut(agent_id)?;

        let raw = format!("{}{}", Uuid::new_v4(), Uuid::new_v4());
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw.as_bytes());
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        let digest = hasher.finalize();
        let expires_at = chrono::Utc::now() + chrono::Duration::hours(24);

        agent.token_hash = Some(base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest));
        agent.token_expires_at = Some(expires_at);
        agent.proxy_token = Some(token.clone());

        Some(IssuedAgentToken {
            agent_id: agent_id.to_string(),
            token,
            expires_at,
        })
    }

    /// Fetch a cached JSON payload when it is still fresh.
    pub async fn cached_json(&self, key: &str) -> Option<serde_json::Value> {
        let cache = self.aggregator_cache.read().await;
        let entry = cache.get(key)?;
        if entry.expires_at > Instant::now() {
            Some(entry.value.clone())
        } else {
            None
        }
    }

    /// Store a cached JSON payload with a time-to-live.
    pub async fn put_cached_json(
        &self,
        key: impl Into<String>,
        ttl: Duration,
        value: serde_json::Value,
    ) {
        self.aggregator_cache.write().await.insert(
            key.into(),
            CachedJsonValue {
                expires_at: Instant::now() + ttl,
                value,
            },
        );
    }
}

/// Serializable snapshot of server state that survives restarts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerStateSnapshot {
    /// Discovery registry entries.
    #[serde(default)]
    pub discovered_agents: HashMap<String, DiscoveredAgent>,
    /// Template run outcome records.
    #[serde(default)]
    pub template_runs: HashMap<String, Vec<TemplateRunRecord>>,
}

/// Shared `.roko/engrams.jsonl` persistence path.
pub struct SignalStore {
    root: PathBuf,
    substrate: OnceCell<Arc<FileSubstrate>>,
}

impl SignalStore {
    /// Create a new store rooted at the `.roko/` directory.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            substrate: OnceCell::new(),
        }
    }

    async fn substrate(&self) -> anyhow::Result<Arc<FileSubstrate>> {
        let substrate = self
            .substrate
            .get_or_try_init(|| async {
                FileSubstrate::open(self.root.clone()).await.map(Arc::new)
            })
            .await?;
        Ok(Arc::clone(substrate))
    }

    /// Persist a signal through the normal file-backed substrate path.
    ///
    /// # Errors
    ///
    /// Returns an error if the backing [`FileSubstrate`] cannot be opened or
    /// if the signal cannot be appended to the `.roko/engrams.jsonl` store.
    pub async fn put(&self, signal: Engram) -> anyhow::Result<()> {
        let substrate = self.substrate().await?;
        substrate.put(signal).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;
    use tokio::task::JoinSet;

    use crate::deploy::manual::ManualBackend;
    use crate::runtime::NoOpRuntime;

    #[tokio::test]
    async fn arcswap_config_supports_concurrent_reads_during_swap() {
        let tempdir = tempdir().expect("tempdir");
        let mut initial = RokoConfig::default();
        initial.server.port = 4000;

        let state = Arc::new(AppState::new(
            tempdir.path().to_path_buf(),
            Arc::new(NoOpRuntime),
            initial,
            Arc::new(ManualBackend::default()),
        ));

        let mut readers = JoinSet::new();
        for _ in 0..8 {
            let state = Arc::clone(&state);
            readers.spawn(async move {
                for _ in 0..256 {
                    let config = state.load_roko_config();
                    let port = config.server.port;
                    assert!(port == 4000 || port == 5000);
                    tokio::task::yield_now().await;
                }
            });
        }

        let mut updated = state.load_roko_config().as_ref().clone();
        updated.server.port = 5000;
        state.store_roko_config(updated);

        while let Some(result) = readers.join_next().await {
            result.expect("reader task");
        }

        assert_eq!(state.load_roko_config().server.port, 5000);
    }
}
