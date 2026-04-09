//! Shared application state for the HTTP server.
//!
//! [`AppState`] is wrapped in `Arc` and injected into every axum handler via
//! the [`axum::extract::State`] extractor. It holds the working directory,
//! layout, configuration, runtime services, and tracking maps for active
//! runs, plans, and operations.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use bardo_runtime::cancel::CancelToken;
use bardo_runtime::process::ProcessSupervisor;
use roko_core::config::schema::RokoConfig;

use crate::deploy::{DeployBackend, Deployment};
use crate::event_bus::EventBus;
use roko_core::obs::metrics::MetricRegistry;
use roko_fs::layout::RokoLayout;

use crate::events::ServerEvent;
use crate::templates::TemplateRegistry;

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
    /// Cancellation token for graceful shutdown.
    pub cancel: CancelToken,
    /// Monotonic timestamp when the server state was created.
    pub started_at: Instant,
    /// Prometheus-style metric registry.
    pub metrics: Arc<MetricRegistry>,
    /// Process lifecycle manager.
    pub supervisor: Arc<ProcessSupervisor>,
    /// Event bus for streaming server events to clients.
    pub event_bus: EventBus<ServerEvent>,
    /// CLI-level configuration (`roko.toml`).
    pub config: RwLock<roko_cli::Config>,
    /// Full `roko.toml` schema configuration.
    pub roko_config: RwLock<RokoConfig>,
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
}

impl AppState {
    /// Construct a new `AppState` from the working directory and loaded configs.
    pub fn new(
        workdir: PathBuf,
        config: roko_cli::Config,
        roko_config: RokoConfig,
        deploy_backend: Arc<dyn DeployBackend>,
    ) -> Self {
        let layout = RokoLayout::for_project(&workdir);
        let cancel = CancelToken::new();
        let supervisor = Arc::new(ProcessSupervisor::new(cancel.child()));
        let templates_dir = workdir.join(".roko").join("templates");

        let mut template_registry = TemplateRegistry::new(templates_dir);
        if let Err(e) = template_registry.scan() {
            tracing::warn!("failed to scan templates: {e}");
        }

        Self {
            workdir,
            layout,
            cancel,
            started_at: Instant::now(),
            metrics: Arc::new(MetricRegistry::new()),
            supervisor,
            event_bus: EventBus::new(1024),
            config: RwLock::new(config),
            roko_config: RwLock::new(roko_config),
            active_runs: RwLock::new(HashMap::new()),
            active_plans: RwLock::new(HashMap::new()),
            operations: RwLock::new(HashMap::new()),
            templates: RwLock::new(template_registry),
            deploy_backend,
            deployments: RwLock::new(HashMap::new()),
            template_runs: RwLock::new(HashMap::new()),
        }
    }

    /// Initiate graceful shutdown: cancel all work and stop supervised processes.
    pub async fn shutdown(&self) {
        tracing::info!("server shutdown initiated");
        self.cancel.cancel();
        self.supervisor.shutdown_all().await;
        self.event_bus.publish(ServerEvent::ServerShutdown);
    }
}
