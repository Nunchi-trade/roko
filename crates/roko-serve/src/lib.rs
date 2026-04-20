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
    clippy::unwrap_or_default
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
pub mod openapi;
pub mod plan_types;
pub mod routes;
pub mod runtime;
pub mod scheduler;
pub mod state;
pub mod templates;

pub use crate::routes::reload_config_from_disk;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use roko_core::Engram;
use roko_core::config::schema::RokoConfig;
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
        let port = self.port.unwrap_or(self.roko_config.server.port);
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

    let roko_config: RokoConfig = if roko_toml_path.exists() {
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
    let roko_config = state.load_roko_config().as_ref().clone();
    let _config_watcher = config_watcher::start_config_watcher(Arc::clone(&state));
    let _prd_publish_subscriber = start_prd_publish_orchestrator(Arc::clone(&state));
    let _state_hub_bridge = start_state_hub_bridge(Arc::clone(&state));
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
    state
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
        ServerEvent::Error { message } => Some(DashboardEvent::Error {
            message: message.clone(),
        }),
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

fn start_builtin_event_sources(state: Arc<AppState>, roko_config: RokoConfig) {
    let mut sources: Vec<Box<dyn EventSource>> = Vec::new();

    if !roko_config.scheduler.is_empty() {
        sources.push(Box::new(CronEventSource::from_config(
            roko_config.scheduler.clone(),
        )));
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
