//! Shared application state for the HTTP server.
//!
//! [`AppState`] is wrapped in `Arc` and injected into every axum handler via
//! the [`axum::extract::State`] extractor. It holds the working directory,
//! layout, configuration, runtime services, and tracking maps for active
//! runs, plans, and operations.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use arc_swap::ArcSwap;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::{OnceCell, RwLock};
use tokio::task::JoinHandle;

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
use roko_core::obs::metrics::MetricRegistry;
use roko_fs::FileSubstrate;
use roko_fs::layout::RokoLayout;

use crate::events::ServerEvent;
use crate::templates::TemplateRegistry;

fn affect_state_path(layout_root: &Path) -> PathBuf {
    layout_root.join("daimon").join("affect.json")
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
    /// Lazily initialized `.roko/signals.jsonl` writer.
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
            event_bus: EventBus::new(1024),
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
        self.cancel.cancel();
        self.supervisor.shutdown_all().await;
        self.event_bus.publish(ServerEvent::ServerShutdown);
    }
}

/// Shared `.roko/signals.jsonl` persistence path.
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
