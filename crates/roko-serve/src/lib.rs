//! HTTP server for the roko API.
//!
//! The [`ServerBuilder`] type is the main entrypoint for embedding the HTTP
//! server. [`run_server`] remains as a convenience wrapper for the current
//! CLI flow.
#![allow(
    dead_code,
    clippy::assigning_clones,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::default_trait_access,
    clippy::derivable_impls,
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::filter_map_bool_then,
    clippy::format_push_string,
    clippy::future_not_send,
    clippy::ignored_unit_patterns,
    clippy::items_after_statements,
    clippy::large_enum_variant,
    clippy::let_underscore_future,
    clippy::manual_let_else,
    clippy::manual_midpoint,
    clippy::map_unwrap_or,
    clippy::missing_const_for_fn,
    clippy::needless_continue,
    clippy::needless_lifetimes,
    clippy::needless_pass_by_ref_mut,
    clippy::needless_pass_by_value,
    clippy::option_if_let_else,
    clippy::or_fun_call,
    clippy::redundant_clone,
    clippy::redundant_closure_for_method_calls,
    clippy::redundant_pub_crate,
    clippy::semicolon_if_nothing_returned,
    clippy::significant_drop_tightening,
    clippy::similar_names,
    clippy::single_match_else,
    clippy::struct_field_names,
    clippy::suboptimal_flops,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::trait_duplication_in_bounds,
    clippy::uninlined_format_args,
    clippy::unnested_or_patterns,
    clippy::unused_async,
    clippy::unwrap_or_default,
    clippy::io_other_error
)]

pub mod config_watcher;
pub mod deploy;
pub mod dispatch;
pub mod dreams;
pub mod error;
pub mod event_bus;
pub mod events;
pub mod extract;
pub mod feedback;
pub mod fswatcher;
pub mod integrations;
pub mod job_runner;
pub mod openapi;
pub mod parity;
pub mod plan_types;
pub mod projection_contract;
pub mod relay;
pub mod retention;
pub mod routes;
pub mod runtime;
pub mod scheduler;
pub mod state;
pub mod templates;
pub mod truth_map;

pub use crate::routes::reload_config_from_disk;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use roko_core::Engram;
use roko_core::config::schema::RokoConfig;
use roko_core::dashboard_snapshot::DashboardEvent;
use roko_plugin::{CronEventSource, EventSource, FileWatchEventSource};

use crate::events::{ExecutionEvent, ServerEvent};
use runtime::CliRuntime;
use state::AppState;

/// Inputs required to start the HTTP server.
pub struct ServerBuildConfig {
    /// Project working directory.
    pub workdir: PathBuf,
    /// Runtime bridge to CLI operations (run_once, status, dashboard).
    pub runtime: Arc<dyn CliRuntime>,
    /// Full `roko.toml` schema configuration.
    pub roko_config: RokoConfig,
    /// Optional bind address override.
    pub bind: Option<String>,
    /// Optional port override.
    pub port: Option<u16>,
}

impl ServerBuildConfig {
    /// Create a new server build configuration.
    pub fn new(
        workdir: PathBuf,
        runtime: Arc<dyn CliRuntime>,
        roko_config: RokoConfig,
        bind: Option<String>,
        port: Option<u16>,
    ) -> Self {
        Self {
            workdir,
            runtime,
            roko_config,
            bind,
            port,
        }
    }

    fn effective_addr(&self) -> String {
        let bind = self
            .bind
            .as_deref()
            .unwrap_or(&self.roko_config.server.bind);
        let port = self.port.unwrap_or_else(|| {
            // Prefer [serve].port when [server].port is still the default.
            if self.roko_config.server.port == 6677 {
                self.roko_config
                    .serve
                    .port
                    .unwrap_or(self.roko_config.server.port)
            } else {
                self.roko_config.server.port
            }
        });
        format!("{bind}:{port}")
    }
}

/// Builder for the HTTP server.
///
/// The builder keeps the resolved bind address, runtime config, and lazily
/// constructed application state together so the same server implementation
/// can be reused by the CLI and future embedders.
pub struct ServerBuilder {
    addr: String,
    config: ServerBuildConfig,
    state: Option<Arc<AppState>>,
}

impl ServerBuilder {
    /// Start a new server builder from the resolved runtime config.
    #[must_use]
    pub fn new(config: ServerBuildConfig) -> Self {
        let addr = config.effective_addr();
        Self {
            addr,
            config,
            state: None,
        }
    }

    /// Enable API-key authentication with the provided key.
    #[must_use]
    pub fn with_auth(mut self, key: impl Into<String>) -> Self {
        self.config.roko_config.serve.auth.enabled = true;
        self.config.roko_config.serve.auth.api_key = key.into();
        self
    }

    /// Bind and run the HTTP server until shutdown.
    ///
    /// # Errors
    ///
    /// Returns an error if the `PORT` environment variable is not a valid
    /// `u16`, the listener cannot bind, or the Axum server exits with an
    /// error.
    pub async fn run(mut self) -> Result<()> {
        // -- PORT env var override (Railway / cloud platforms) -------------
        let addr = if let Ok(env_port) = std::env::var("PORT") {
            let p: u16 = env_port
                .parse()
                .context("PORT env var must be a valid u16")?;
            info!("PORT env var detected ({p}), binding to 0.0.0.0:{p}");
            format!("0.0.0.0:{p}")
        } else {
            self.addr.clone()
        };

        let workdir = self.config.workdir.clone();
        let runtime = Arc::clone(&self.config.runtime);
        let roko_config = self.config.roko_config.clone();
        let state = Arc::clone(
            self.state
                .get_or_insert_with(|| Arc::new(build_app_state(workdir, runtime, roko_config))),
        );
        if let Err(err) = state.restore_snapshot().await {
            warn!(error = %err, "failed to restore server state snapshot; starting fresh");
        }
        let dispatcher_roko_config = state.load_roko_config().as_ref().clone();
        let dispatcher = Arc::new(dispatch::TemplateAgentDispatcher::new(
            state.workdir.clone(),
            None,
            dispatcher_roko_config,
        ));
        tokio::spawn(dispatch::dispatch_loop(Arc::clone(&state), dispatcher));
        start_builtin_event_sources(Arc::clone(&state), self.config.roko_config.clone());
        let _config_watcher = config_watcher::start_config_watcher(Arc::clone(&state));
        let _prd_publish_subscriber = start_prd_publish_orchestrator(Arc::clone(&state));
        let _feedback_loop = feedback::start_feedback_loop(Arc::clone(&state));
        let _state_hub_bridge = start_state_hub_bridge(Arc::clone(&state));
        // NOTE: start_orchestrator_event_bridge is intentionally NOT started here.
        // It creates a feedback loop: EventBus → StateHub → EventBus → ∞.
        // The orchestrator publishes directly to the StateHub when running in-process.
        let _state_saver = start_state_snapshot_saver(Arc::clone(&state));
        let _job_runner = job_runner::start_job_runner(Arc::clone(&state));
        let router = routes::build_router(
            Arc::clone(&state),
            &self.config.roko_config.server.cors_origins,
            self.config.roko_config.serve.auth.clone(),
        );

        let listener = TcpListener::bind(&addr)
            .await
            .with_context(|| format!("bind to {addr}"))?;

        info!("roko server listening on http://{addr}");
        info!("workdir: {}", self.config.workdir.display());

        // Spawn chain-watcher if chain.rpc_url is configured (best-effort).
        if let Some(rpc_url) = self.config.roko_config.chain.rpc_url.as_deref() {
            let rpc = rpc_url.to_string();
            tokio::spawn(async move {
                let watcher = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.join("roko-chain-watcher")))
                    .unwrap_or_else(|| std::path::PathBuf::from("roko-chain-watcher"));
                match tokio::process::Command::new(&watcher)
                    .arg("--rpc-url")
                    .arg(&rpc)
                    .status()
                    .await
                {
                    Ok(s) => tracing::info!(exit = %s, "chain-watcher exited"),
                    Err(e) => {
                        tracing::debug!(error = %e, path = ?watcher, "chain-watcher not available")
                    }
                }
            });
        }

        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal(state))
            .await
            .context("axum server error")?;

        info!("server stopped");
        Ok(())
    }
}

/// Start the HTTP server.
///
/// # Errors
///
/// Returns an error if `roko.toml` cannot be read or parsed, if the resolved
/// listener cannot bind, or if serving the Axum router fails.
pub async fn run_server(
    workdir: PathBuf,
    runtime: Arc<dyn CliRuntime>,
    bind: Option<String>,
    port: Option<u16>,
) -> Result<()> {
    let roko_toml_path = workdir.join("roko.toml");

    let mut roko_config: RokoConfig = if roko_toml_path.exists() {
        let text = std::fs::read_to_string(&roko_toml_path)
            .with_context(|| format!("read {}", roko_toml_path.display()))?;
        toml::from_str(&text).with_context(|| format!("parse {}", roko_toml_path.display()))?
    } else {
        warn!(
            "no roko.toml found at {}; using defaults",
            roko_toml_path.display()
        );
        RokoConfig::default()
    };

    // Let `ROKO_DEPLOY_*` env vars override the TOML-declared deploy config so
    // secrets like `ROKO_DEPLOY_RAILWAY_API_TOKEN` never land in `roko.toml`.
    roko_config.deploy.apply_process_env();

    let config = ServerBuildConfig::new(workdir, runtime, roko_config, bind, port);
    ServerBuilder::new(config).run().await
}

/// Start the PRD-publish auto-orchestration background tasks for an existing state.
#[doc(hidden)]
pub fn start_prd_publish_orchestrator(state: Arc<AppState>) -> JoinHandle<()> {
    routes::start_prd_publish_subscriber(state)
}

/// Run the HTTP server against an already constructed [`AppState`].
///
/// # Errors
///
/// Returns an error if the listener cannot bind to `bind:port` or if the
/// Axum server exits with an error.
pub async fn run_server_with_state(state: Arc<AppState>, bind: &str, port: u16) -> Result<()> {
    if let Err(err) = state.restore_snapshot().await {
        warn!(error = %err, "failed to restore server state snapshot; starting fresh");
    }
    let roko_config = state.load_roko_config().as_ref().clone();
    let _config_watcher = config_watcher::start_config_watcher(Arc::clone(&state));
    let _prd_publish_subscriber = start_prd_publish_orchestrator(Arc::clone(&state));
    let _state_hub_bridge = start_state_hub_bridge(Arc::clone(&state));
    // NOTE: start_orchestrator_event_bridge is intentionally NOT started here.
    // It creates a feedback loop: EventBus → StateHub → EventBus → ∞.
    let _state_saver = start_state_snapshot_saver(Arc::clone(&state));
    let _job_runner = job_runner::start_job_runner(Arc::clone(&state));
    let router = routes::build_router(
        Arc::clone(&state),
        &roko_config.server.cors_origins,
        roko_config.serve.auth.clone(),
    );
    let addr = format!("{bind}:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("bind to {addr}"))?;

    info!("roko server listening on http://{addr}");
    info!("workdir: {}", state.workdir.display());

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_on_cancel(Arc::clone(&state)))
        .await
        .context("axum server error")?;

    info!("server stopped");
    Ok(())
}

fn build_app_state(
    workdir: PathBuf,
    runtime: Arc<dyn CliRuntime>,
    roko_config: RokoConfig,
) -> AppState {
    let deploy_backend = create_deploy_backend(&roko_config);
    let state = AppState::new(workdir, runtime, roko_config, deploy_backend);
    let _ = state.state_hub.bootstrap_from_workdir(&state.workdir);
    // Seed StateHub with persisted marketplace jobs so the TUI sees them on connect.
    let jobs = scan_marketplace_jobs(&state.workdir);
    if !jobs.is_empty() {
        info!(
            count = jobs.len(),
            "loaded existing marketplace jobs from disk"
        );
        state
            .state_hub
            .publish(DashboardEvent::MarketplaceJobsUpdated { jobs });
    }
    // Seed StateHub with persisted PRDs so the Atelier tab is populated on connect.
    let prds = scan_prd_summaries(&state.workdir);
    if !prds.is_empty() {
        info!(count = prds.len(), "loaded existing PRDs from disk");
        state.state_hub.publish(DashboardEvent::AtelierPrdsUpdated {
            prds,
            tasks: std::collections::HashMap::new(),
        });
    }
    state
}

/// Scan `.roko/jobs/*.json` and return a vec of `MarketplaceJob`.
fn scan_marketplace_jobs(workdir: &Path) -> Vec<roko_core::MarketplaceJob> {
    let dir = workdir.join(".roko").join("jobs");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let mut jobs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let data = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        match serde_json::from_str::<roko_core::MarketplaceJob>(&data) {
            Ok(job) => jobs.push(job),
            Err(err) => {
                warn!(
                    path = %path.display(),
                    error = %err,
                    "skipping malformed job file during startup scan"
                );
            }
        }
    }
    jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    jobs
}

/// Scan `.roko/prd/{drafts,published}/*.md` and return a vec of `PrdSummary`.
fn scan_prd_summaries(workdir: &Path) -> Vec<roko_core::PrdSummary> {
    let prd_dir = workdir.join(".roko").join("prd");
    let mut prds = Vec::new();
    for (status, subdir) in [("draft", "drafts"), ("published", "published")] {
        let dir = prd_dir.join(subdir);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let slug = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            prds.push(roko_core::PrdSummary {
                slug: slug.clone(),
                title: slug,
                status: status.to_string(),
                ..Default::default()
            });
        }
    }
    prds
}

fn start_state_hub_bridge(state: Arc<AppState>) -> JoinHandle<()> {
    let mut rx = state.event_bus.subscribe();
    let sender = state.state_hub.sender();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(envelope) => {
                    if let Some(event) = server_event_to_dashboard(&envelope.payload) {
                        sender.publish(event);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!(skipped, "state hub bridge lagged behind server event bus");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

fn server_event_to_dashboard(event: &ServerEvent) -> Option<roko_core::DashboardEvent> {
    use roko_core::DashboardEvent;

    match event {
        ServerEvent::PlanStarted { plan_id } => Some(DashboardEvent::PlanStarted {
            plan_id: plan_id.clone(),
        }),
        ServerEvent::PlanCompleted { plan_id, success } => Some(DashboardEvent::PlanCompleted {
            plan_id: plan_id.clone(),
            success: *success,
        }),
        ServerEvent::AgentSpawned { agent_id, role } => Some(DashboardEvent::AgentSpawned {
            agent_id: agent_id.clone(),
            role: role.clone(),
        }),
        ServerEvent::AgentOutput {
            agent_id, content, ..
        } => Some(DashboardEvent::AgentOutput {
            agent_id: agent_id.clone(),
            content: content.clone(),
        }),
        ServerEvent::GateResult {
            plan_id,
            task_id,
            gate,
            rung: _,
            passed,
        } => Some(DashboardEvent::GateResult {
            plan_id: plan_id.clone(),
            task_id: task_id.clone(),
            gate: gate.clone(),
            passed: *passed,
        }),
        ServerEvent::Execution { plan_id, event } => match event {
            ExecutionEvent::TaskStarted { task_id, phase } => Some(DashboardEvent::TaskStarted {
                plan_id: plan_id.clone(),
                task_id: task_id.clone(),
                phase: phase.clone(),
            }),
            ExecutionEvent::TaskCompleted { task_id, outcome } => {
                Some(DashboardEvent::TaskCompleted {
                    plan_id: plan_id.clone(),
                    task_id: task_id.clone(),
                    outcome: outcome.clone(),
                })
            }
            ExecutionEvent::TaskPhaseChanged {
                task_id,
                old_phase,
                new_phase,
            } => Some(DashboardEvent::TaskPhaseChanged {
                plan_id: plan_id.clone(),
                task_id: task_id.clone(),
                old_phase: old_phase.clone(),
                new_phase: new_phase.clone(),
            }),
            ExecutionEvent::GateResult {
                task_id,
                gate,
                passed,
                ..
            } => Some(DashboardEvent::GateResult {
                plan_id: plan_id.clone(),
                task_id: task_id.clone(),
                gate: gate.clone(),
                passed: *passed,
            }),
            _ => None,
        },
        ServerEvent::PhaseTransition { plan_id, from, to } => {
            Some(DashboardEvent::PhaseTransition {
                plan_id: plan_id.clone(),
                from: from.clone(),
                to: to.clone(),
            })
        }
        ServerEvent::EfficiencyEvent {
            plan_id,
            task_id,
            metric,
            value,
        } => Some(DashboardEvent::EfficiencyEvent {
            plan_id: plan_id.clone(),
            task_id: task_id.clone(),
            metric: metric.clone(),
            value: *value,
        }),
        ServerEvent::JobExecutionStarted {
            job_id,
            job_type,
            agent_id,
        } => Some(DashboardEvent::JobExecutionStarted {
            job_id: job_id.clone(),
            job_type: job_type.clone(),
            agent_id: agent_id.clone(),
        }),
        ServerEvent::JobProgress {
            job_id,
            percent,
            message,
        } => Some(DashboardEvent::JobProgress {
            job_id: job_id.clone(),
            percent: *percent,
            message: message.clone(),
        }),
        ServerEvent::Error { message } => Some(DashboardEvent::Error {
            message: message.clone(),
        }),
        _ => None,
    }
}

/// Bridge orchestrator events (`StateHub` → `EventBus`) so SSE/WS clients
/// see gate results, task completions, and other events from `roko plan run`.
///
/// This is the reverse direction of [`start_state_hub_bridge`] which pushes
/// REST-triggered `ServerEvent`s into the `StateHub` for the TUI.
pub fn start_orchestrator_event_bridge(state: Arc<AppState>) -> JoinHandle<()> {
    let mut rx = state.state_hub.subscribe_events();
    let bus = state.event_bus.clone();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(envelope) => {
                    if let Some(server_event) = dashboard_event_to_server(&envelope.payload) {
                        bus.publish(server_event);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!(n, "orchestrator bridge lagged behind state hub");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

/// Convert a [`DashboardEvent`] to a [`ServerEvent`] for SSE/WS delivery.
/// Inverse of [`server_event_to_dashboard`].
fn dashboard_event_to_server(event: &roko_core::DashboardEvent) -> Option<ServerEvent> {
    use roko_core::DashboardEvent;
    match event {
        DashboardEvent::PlanStarted { plan_id } => Some(ServerEvent::PlanStarted {
            plan_id: plan_id.clone(),
        }),
        DashboardEvent::PlanCompleted { plan_id, success } => Some(ServerEvent::PlanCompleted {
            plan_id: plan_id.clone(),
            success: *success,
        }),
        DashboardEvent::TaskStarted {
            plan_id,
            task_id,
            phase,
        } => Some(ServerEvent::Execution {
            plan_id: plan_id.clone(),
            event: ExecutionEvent::TaskStarted {
                task_id: task_id.clone(),
                phase: phase.clone(),
            },
        }),
        DashboardEvent::TaskCompleted {
            plan_id,
            task_id,
            outcome,
        } => Some(ServerEvent::Execution {
            plan_id: plan_id.clone(),
            event: ExecutionEvent::TaskCompleted {
                task_id: task_id.clone(),
                outcome: outcome.clone(),
            },
        }),
        DashboardEvent::TaskPhaseChanged {
            plan_id,
            task_id,
            old_phase,
            new_phase,
        } => Some(ServerEvent::Execution {
            plan_id: plan_id.clone(),
            event: ExecutionEvent::TaskPhaseChanged {
                task_id: task_id.clone(),
                old_phase: old_phase.clone(),
                new_phase: new_phase.clone(),
            },
        }),
        DashboardEvent::AgentSpawned { agent_id, role } => Some(ServerEvent::AgentSpawned {
            agent_id: agent_id.clone(),
            role: role.clone(),
        }),
        DashboardEvent::AgentOutput { agent_id, content } => Some(ServerEvent::AgentOutput {
            agent_id: agent_id.clone(),
            run_id: None,
            content: content.clone(),
            done: false,
            metadata: None,
        }),
        DashboardEvent::GateResult {
            plan_id,
            task_id,
            gate,
            passed,
        } => Some(ServerEvent::GateResult {
            plan_id: plan_id.clone(),
            task_id: task_id.clone(),
            gate: gate.clone(),
            rung: 0,
            passed: *passed,
        }),
        DashboardEvent::PhaseTransition { plan_id, from, to } => {
            Some(ServerEvent::PhaseTransition {
                plan_id: plan_id.clone(),
                from: from.clone(),
                to: to.clone(),
            })
        }
        DashboardEvent::EfficiencyEvent {
            plan_id,
            task_id,
            metric,
            value,
        } => Some(ServerEvent::EfficiencyEvent {
            plan_id: plan_id.clone(),
            task_id: task_id.clone(),
            metric: metric.clone(),
            value: *value,
        }),
        DashboardEvent::JobExecutionStarted {
            job_id,
            job_type,
            agent_id,
        } => Some(ServerEvent::JobExecutionStarted {
            job_id: job_id.clone(),
            job_type: job_type.clone(),
            agent_id: agent_id.clone(),
        }),
        DashboardEvent::JobProgress {
            job_id,
            percent,
            message,
        } => Some(ServerEvent::JobProgress {
            job_id: job_id.clone(),
            percent: *percent,
            message: message.clone(),
        }),
        DashboardEvent::Error { message } => Some(ServerEvent::Error {
            message: message.clone(),
        }),
        // Unmapped variants (Diagnosis, ExperimentWinnersUpdated, CFactorTrendUpdated,
        // CascadeRouterUpdated, GateThresholdsUpdated, etc.) are dropped.
        // FIXME: bridge loop — REST-originated events appear twice on EventBus
        // (once from REST, once via StateHub→EventBus). Accepted for now.
        _ => None,
    }
}

pub(crate) fn start_event_source_group(
    state: Arc<AppState>,
    sources: Vec<Box<dyn EventSource>>,
) -> JoinHandle<()> {
    if sources.is_empty() {
        return tokio::spawn(async {});
    }

    let cancel = CancellationToken::new();
    let cancel_for_shutdown = cancel.clone();
    let state_for_shutdown = Arc::clone(&state);
    tokio::spawn(async move {
        state_for_shutdown.cancel.cancelled().await;
        cancel_for_shutdown.cancel();
    });

    let (signal_tx, signal_rx) = mpsc::channel::<Engram>(256);
    tokio::spawn(signal_ingest_loop(
        Arc::clone(&state),
        signal_rx,
        cancel.clone(),
    ));

    for source in sources {
        let source_name = source.name().to_string();
        let source_kind = source.kind();
        let sender = signal_tx.clone();
        let cancel = cancel.clone();

        tokio::spawn(async move {
            if let Err(err) = source.start(sender, cancel).await {
                warn!(
                    source = %source_name,
                    kind = ?source_kind,
                    error = %err,
                    "event source stopped"
                );
            }
        });
    }

    tokio::spawn(async {})
}

fn start_state_snapshot_saver(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            tokio::select! {
                _ = state.cancel.cancelled() => break,
                _ = interval.tick() => {}
            }
            if let Err(err) = state.save_snapshot().await {
                warn!(error = %err, "periodic server state snapshot save failed");
            }
        }
    })
}

fn start_builtin_event_sources(state: Arc<AppState>, roko_config: RokoConfig) {
    let mut sources: Vec<Box<dyn EventSource>> = Vec::new();

    if !roko_config.scheduler.is_empty() && scheduler::claim_scheduler_guard() {
        sources.push(Box::new(CronEventSource::from_config(
            roko_config.scheduler.clone(),
        )));
    } else if !roko_config.scheduler.is_empty() {
        debug!("scheduler already started elsewhere; skipping cron in event sources");
    }

    if !roko_config.watcher.is_empty() {
        sources.push(Box::new(FileWatchEventSource::from_config(
            roko_config.watcher.clone(),
        )));
    }

    if sources.is_empty() {
        return;
    }

    let _ = start_event_source_group(state, sources);
}

async fn signal_ingest_loop(
    state: Arc<AppState>,
    mut receiver: mpsc::Receiver<Engram>,
    cancel: CancellationToken,
) {
    loop {
        let maybe_signal = tokio::select! {
            _ = cancel.cancelled() => None,
            signal = receiver.recv() => signal,
        };
        let Some(signal) = maybe_signal else {
            break;
        };

        if let Err(err) = state.signal_store.put(signal.clone()).await {
            warn!(
                kind = %signal.kind,
                error = %err,
                "failed to persist event-source signal"
            );
            continue;
        }

        state
            .event_bus
            .publish(ServerEvent::WebhookReceived { signal });
    }
}

async fn shutdown_on_cancel(state: Arc<AppState>) {
    state.cancel.cancelled().await;
    state.shutdown().await;
}

fn create_deploy_backend(roko_config: &RokoConfig) -> Arc<dyn deploy::DeployBackend> {
    let dc = &roko_config.deploy;
    match deploy::create_backend(
        &dc.backend,
        dc.railway_api_token.as_deref(),
        dc.project_id.as_deref(),
        dc.environment_id.as_deref(),
    ) {
        Ok(b) => Arc::from(b),
        Err(e) => {
            warn!(
                "failed to create deploy backend '{}': {e}; falling back to manual",
                dc.backend
            );
            Arc::from(
                deploy::create_backend("manual", None, None, None)
                    .expect("manual backend cannot fail"),
            )
        }
    }
}

/// Wait for ctrl-c then trigger graceful shutdown.
async fn shutdown_signal(state: Arc<AppState>) {
    let _ = tokio::signal::ctrl_c().await;
    info!("received ctrl-c, shutting down");
    state.shutdown().await;
}
