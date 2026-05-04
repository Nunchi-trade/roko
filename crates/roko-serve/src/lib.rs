//! HTTP server for the roko API.
//!
//! The [`ServerBuilder`] type is the main entrypoint for embedding the HTTP
//! server. [`run_server`] remains as a convenience wrapper for the current
//! CLI flow.
#![allow(missing_docs)]
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
    clippy::needless_raw_string_hashes,
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
#![allow(hidden_glob_reexports)]

extern crate roko_core as roko_core_crate;
extern crate self as roko_core;

pub use roko_core_crate::*;

// TODO(converge): remove this compatibility layer once roko-core re-exports
// StateHub and SharedStateHub from its crate root again.
pub mod dashboard_snapshot {
    pub use crate::roko_core_crate::dashboard_snapshot::*;
}

#[path = "../../roko-core/src/state_hub.rs"]
pub mod state_hub_compat;

pub mod state_hub {
    pub use crate::state_hub_compat::*;
}

pub use state_hub_compat::{SharedStateHub, StateHub};

pub mod adapters;
pub mod bench;
pub mod command_events;
pub mod config_watcher;
pub mod deploy;
pub mod dispatch;
pub mod dreams;
pub mod embedded;
pub mod error;
pub mod event_bus;
pub mod events;
pub mod extract;
pub mod feedback;
pub mod fswatcher;
pub mod integrations;
pub mod job_runner;
pub mod jwks;
pub mod openapi;
pub mod parity;
pub mod plan_types;
pub mod projection_contract;
pub mod relay;
pub mod retention;
pub mod routes;
pub mod runtime;
pub mod sanitize;
pub mod scheduler;
pub mod state;
pub mod templates;
pub mod terminal;
pub mod truth_map;

pub use crate::routes::reload_config_from_disk;
pub use crate::sanitize::sanitize_agent_content;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as AnyhowContext, Result};
use axum::response::IntoResponse;
use roko_core_crate::config::ServeConfig;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{debug, info, warn};

use roko_core::Engram;
use roko_core::config::schema::RokoConfig;
use roko_core::connector::{ConnectorHealth, ConnectorInfo, ConnectorKind, ConnectorStatus};
use roko_core::dashboard_snapshot::DashboardEvent;
use roko_core::feed::{FeedAccess, FeedInfo, FeedKind};
use roko_core::foundation::EventConsumer;
use roko_core::{RuntimeEvent, WorkflowOutcome};
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

    fn effective_bind(&self) -> &str {
        self.bind
            .as_deref()
            .unwrap_or(&self.roko_config.server.bind)
    }

    fn effective_port(&self) -> u16 {
        self.port.unwrap_or_else(|| {
            // Prefer [serve].port when [server].port is still the default.
            if self.roko_config.server.port == 6677 {
                self.roko_config
                    .serve
                    .port
                    .unwrap_or(self.roko_config.server.port)
            } else {
                self.roko_config.server.port
            }
        })
    }

    fn effective_addr(&self) -> String {
        format!("{}:{}", self.effective_bind(), self.effective_port())
    }
}

/// Resolve the bind socket when the `PORT` environment variable is in play.
///
/// Cloud platforms (Railway, Fly, etc.) set `PORT` to tell the server which
/// port to listen on, but they intentionally do **not** dictate the bind
/// address. Earlier behaviour silently rebound to `0.0.0.0`, which exposed
/// the API surface of every local-dev workflow that happened to have `PORT`
/// set in its shell. From T3-25 onwards we honour `PORT` for the port only;
/// the bind comes from `[server].bind` in `roko.toml` (default
/// `127.0.0.1`). Operators who actually want a public bind opt in by
/// setting `bind = "0.0.0.0"` in their config (and clearing the existing
/// `serve.acknowledge_public_risk` / `serve.auth.enabled` checks in
/// [`validate_bind_safety`]).
pub(crate) fn resolve_bind_with_port_env(
    config_bind: &str,
    cli_bind_override: Option<&str>,
    config_port: u16,
    cli_port_override: Option<u16>,
    port_env: Option<&str>,
) -> Result<(String, u16)> {
    let resolved_port = match port_env {
        Some(value) => value
            .parse::<u16>()
            .with_context(|| format!("PORT env var must be a valid u16 (got {value:?})"))?,
        None => cli_port_override.unwrap_or(config_port),
    };
    let resolved_bind = cli_bind_override.unwrap_or(config_bind).to_string();
    Ok((resolved_bind, resolved_port))
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

    /// Start the server in the background and return the live state handle.
    ///
    /// The returned [`Arc<AppState>`] carries the [`SharedStateHub`] that the
    /// TUI or other in-process consumers can subscribe to.  The
    /// [`JoinHandle`] resolves when the server shuts down (e.g. because
    /// `state.cancel.cancel()` was called).
    #[allow(clippy::missing_panics_doc)]
    pub async fn start_background(mut self) -> Result<(Arc<AppState>, JoinHandle<Result<()>>)> {
        // -- PORT env var override (Railway / cloud platforms) -------------
        // The `PORT` env var lets the platform pick a port; it does NOT imply
        // the operator wants a public bind. Per T3-25 we override only the
        // port and keep the bind from config (default `127.0.0.1`). Setting
        // `[server].bind = "0.0.0.0"` in `roko.toml` is the explicit opt-in.
        let port_env = std::env::var("PORT").ok();
        let addr = if let Some(value) = port_env.as_deref() {
            let (bind, port) = resolve_bind_with_port_env(
                &self.config.roko_config.server.bind,
                self.config.bind.as_deref(),
                self.config.roko_config.server.port,
                self.config.port,
                Some(value),
            )?;
            info!(
                "PORT env var detected ({port}); binding to {bind}:{port} \
                 (set `[server].bind = \"0.0.0.0\"` for a public bind)"
            );
            format!("{bind}:{port}")
        } else {
            self.addr.clone()
        };

        let workdir = self.config.workdir.clone();
        let runtime = Arc::clone(&self.config.runtime);
        let roko_config = self.config.roko_config.clone();
        if self.state.is_none() {
            self.state = Some(Arc::new(build_app_state(workdir, runtime, roko_config)?));
        }
        let state = Arc::clone(self.state.as_ref().expect("state just set"));
        let roko_config = state.load_roko_config();
        validate_bind_safety(&addr, &roko_config.serve)?;
        if let Err(err) = state.restore_snapshot().await {
            warn!(error = %err, "failed to restore server state snapshot; starting fresh");
        }
        let dispatcher_roko_config = roko_config.as_ref().clone();
        let dispatcher = Arc::new(dispatch::TemplateAgentDispatcher::new(
            state.workdir.clone(),
            None,
            dispatcher_roko_config,
        ));
        tokio::spawn(dispatch::dispatch_loop(Arc::clone(&state), dispatcher));
        start_builtin_event_sources(Arc::clone(&state), roko_config.as_ref().clone());
        let _config_watcher = config_watcher::start_config_watcher(Arc::clone(&state));
        let _prd_publish_subscriber = start_prd_publish_orchestrator(Arc::clone(&state));
        let _feedback_loop = feedback::start_feedback_loop(Arc::clone(&state));
        let _state_hub_bridge = start_state_hub_bridge(Arc::clone(&state));
        let _state_saver = start_state_snapshot_saver(Arc::clone(&state));
        let _job_runner = job_runner::start_job_runner(Arc::clone(&state));
        let _cold_archival = start_cold_archival_timer(Arc::clone(&state));
        let _workspace_gc = start_workspace_gc(Arc::clone(&state));
        let _handle_gc = start_handle_gc(Arc::clone(&state));

        // Load persisted deployments from disk.
        routes::load_persisted_deployments(&state).await;

        // Eagerly prime the JWKS cache if Privy auth is configured.
        if roko_config.serve.auth.privy_app_id.is_some() {
            let jwks = Arc::clone(&state.jwks_cache);
            tokio::spawn(async move {
                jwks.prime().await;
            });
        }

        // Register workspace with relay if configured.
        let serve_port = self.config.port.unwrap_or(6677);
        let _relay_registration = relay::start_workspace_registration(
            self.config.roko_config.relay.clone(),
            serve_port,
            Arc::clone(&state.agent_count),
            Arc::clone(&state.relay_health),
        );

        let router = build_server_router(
            Arc::clone(&state),
            &roko_config.server.cors_origins,
            roko_config.server.unsafe_public_cors,
            roko_config.serve.auth.clone(),
        );

        let listener = TcpListener::bind(&addr)
            .await
            .with_context(|| format!("bind to {addr}"))?;

        info!("roko server listening on http://{addr}");
        info!("workdir: {}", self.config.workdir.display());

        // Spawn chain-watcher if chain.rpc_url is configured (best-effort).
        // Redirect all subprocess output to .roko/chain-watcher.log to prevent
        // flooding the terminal when serve runs in the background.
        if let Some(rpc_url) = self.config.roko_config.chain.rpc_url.as_deref() {
            let rpc = rpc_url.to_string();
            let log_path = self.config.workdir.join(".roko").join("chain-watcher.log");
            tokio::spawn(async move {
                let watcher = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.join("roko-chain-watcher")))
                    .unwrap_or_else(|| std::path::PathBuf::from("roko-chain-watcher"));

                // Open log file for subprocess output (fall back to /dev/null).
                let (stdout_target, stderr_target) = if let Ok(f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_path)
                {
                    let f2 = f
                        .try_clone()
                        .unwrap_or_else(|_| std::fs::File::open("/dev/null").expect("/dev/null"));
                    (std::process::Stdio::from(f), std::process::Stdio::from(f2))
                } else {
                    (std::process::Stdio::null(), std::process::Stdio::null())
                };

                match tokio::process::Command::new(&watcher)
                    .arg("--rpc-url")
                    .arg(&rpc)
                    .env("ROKO_LOG", "warn")
                    .stdout(stdout_target)
                    .stderr(stderr_target)
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

        let serve_state = Arc::clone(&state);
        let handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown_on_cancel(serve_state))
                .await
                .context("axum server error")?;
            info!("server stopped");
            Ok(())
        });

        Ok((state, handle))
    }

    /// Bind and run the HTTP server until shutdown.
    ///
    /// # Errors
    ///
    /// Returns an error if the `PORT` environment variable is not a valid
    /// `u16`, the listener cannot bind, or the Axum server exits with an
    /// error.
    pub async fn run(self) -> Result<()> {
        let (state, handle) = self.start_background().await?;
        // Block on Ctrl-C to shut down.
        let _ = tokio::signal::ctrl_c().await;
        info!("received ctrl-c, shutting down");
        state.shutdown().await;
        // Wait for the server task to finish.
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(anyhow::anyhow!("server task panicked: {e}")),
        }
        Ok(())
    }
}

/// Start the HTTP server.
///
/// # Errors
///
/// Returns an error if config cannot be read or parsed, if the resolved
/// listener cannot bind, or if serving the Axum router fails.
pub async fn run_server(
    workdir: PathBuf,
    runtime: Arc<dyn CliRuntime>,
    bind: Option<String>,
    port: Option<u16>,
) -> Result<()> {
    let roko_config = roko_core_crate::config::loader::load_config_unified(&workdir)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let config = ServerBuildConfig::new(workdir, runtime, roko_config, bind, port);
    ServerBuilder::new(config).run().await
}

/// Start the HTTP server in the background and return the live app state.
///
/// The returned [`Arc<AppState>`] carries the [`SharedStateHub`] that an
/// in-process TUI or other consumer can subscribe to.  The
/// [`JoinHandle`] resolves when the server shuts down.
///
/// Call `state.cancel.cancel()` or `state.shutdown().await` to stop the
/// server.
pub async fn start_server_background(
    workdir: PathBuf,
    runtime: Arc<dyn CliRuntime>,
    bind: Option<String>,
    port: Option<u16>,
) -> Result<(Arc<AppState>, JoinHandle<Result<()>>)> {
    let roko_config = roko_core_crate::config::loader::load_config_unified(&workdir)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let config = ServerBuildConfig::new(workdir, runtime, roko_config, bind, port);
    ServerBuilder::new(config).start_background().await
}

/// Start the PRD-publish auto-orchestration background tasks for an existing state.
#[doc(hidden)]
pub fn start_prd_publish_orchestrator(state: Arc<AppState>) -> JoinHandle<()> {
    routes::start_prd_publish_subscriber(state)
}

/// Bridges WorkflowEngine RuntimeEvents to SharedStateHub as DashboardEvents.
struct DashboardEventBridge {
    state_hub: SharedStateHub,
}

impl DashboardEventBridge {
    fn new(state_hub: SharedStateHub) -> Self {
        Self { state_hub }
    }
}

impl EventConsumer for DashboardEventBridge {
    fn consume(&self, event: &RuntimeEvent) {
        let events = match event {
            RuntimeEvent::WorkflowStarted { run_id, prompt, .. } => {
                let plan_id = workflow_plan_id(run_id);
                let task_id = workflow_task_id(run_id);
                vec![
                    DashboardEvent::PlanStarted {
                        plan_id: plan_id.clone(),
                    },
                    DashboardEvent::TaskStarted {
                        plan_id,
                        task_id,
                        title: prompt.clone(),
                        phase: "workflow".into(),
                    },
                ]
            }
            RuntimeEvent::AgentSpawned {
                agent_id,
                role,
                model,
                ..
            } => vec![DashboardEvent::AgentSpawned {
                agent_id: agent_id.clone(),
                role: role.clone(),
                model: dashboard_model_label(model, agent_id),
            }],
            RuntimeEvent::AgentOutput {
                agent_id, chunk, ..
            } => vec![DashboardEvent::AgentOutput {
                agent_id: agent_id.clone(),
                content: chunk.clone(),
            }],
            RuntimeEvent::AgentCompleted { agent_id, .. } => {
                vec![DashboardEvent::AgentCompleted {
                    agent_id: agent_id.clone(),
                }]
            }
            RuntimeEvent::PhaseTransition { run_id, from, to } => {
                vec![DashboardEvent::PhaseTransition {
                    plan_id: workflow_plan_id(run_id),
                    from: from.clone(),
                    to: to.clone(),
                }]
            }
            RuntimeEvent::GatePassed {
                run_id,
                gate_name,
                duration_ms,
            } => vec![
                DashboardEvent::GateResult {
                    plan_id: workflow_plan_id(run_id),
                    task_id: workflow_task_id(run_id),
                    gate: gate_name.clone(),
                    passed: true,
                },
                workflow_gate_log_entry(run_id, gate_name, *duration_ms, true),
            ],
            RuntimeEvent::GateFailed {
                run_id,
                gate_name,
                duration_ms,
                ..
            } => vec![
                DashboardEvent::GateResult {
                    plan_id: workflow_plan_id(run_id),
                    task_id: workflow_task_id(run_id),
                    gate: gate_name.clone(),
                    passed: false,
                },
                workflow_gate_log_entry(run_id, gate_name, *duration_ms, false),
            ],
            RuntimeEvent::WorkflowCompleted { run_id, outcome } => {
                let plan_id = workflow_plan_id(run_id);
                let success = matches!(outcome, WorkflowOutcome::Success { .. });
                vec![
                    DashboardEvent::TaskCompleted {
                        plan_id: plan_id.clone(),
                        task_id: workflow_task_id(run_id),
                        outcome: workflow_outcome_label(outcome),
                    },
                    DashboardEvent::PlanCompleted { plan_id, success },
                ]
            }
            _ => Vec::new(),
        };

        if !events.is_empty() {
            self.state_hub.publish_batch(events);
        }
    }
}

/// Create a DashboardEventBridge for attaching to WorkflowEngine instances.
#[must_use]
pub fn dashboard_event_bridge(state: &Arc<AppState>) -> Arc<dyn EventConsumer> {
    Arc::new(DashboardEventBridge::new(state.state_hub.clone()))
}

fn workflow_plan_id(run_id: &str) -> String {
    format!("wf-{}", run_id.chars().take(8).collect::<String>())
}

fn workflow_task_id(run_id: &str) -> String {
    format!("workflow-{}", run_id.chars().take(8).collect::<String>())
}

fn workflow_gate_log_entry(
    run_id: &str,
    gate_name: &str,
    duration_ms: u64,
    passed: bool,
) -> DashboardEvent {
    DashboardEvent::EventLogEntry {
        timestamp_ms: now_millis(),
        event_type: "gate_result".into(),
        plan_id: workflow_plan_id(run_id),
        task_id: gate_name.to_string(),
        message: format!(
            "{} {} ({}ms)",
            if passed { "PASS" } else { "FAIL" },
            gate_name,
            duration_ms
        ),
    }
}

fn workflow_outcome_label(outcome: &WorkflowOutcome) -> String {
    match outcome {
        WorkflowOutcome::Success { commit_hash } => commit_hash
            .as_ref()
            .map_or_else(|| "success".to_string(), |hash| format!("success ({hash})")),
        WorkflowOutcome::Halted { reason } => format!("halted: {reason}"),
        WorkflowOutcome::Cancelled => "cancelled".to_string(),
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

/// Returns `true` when the host portion of `addr` resolves to a loopback address.
///
/// Handles both `127.0.0.1`, `::1`, and hostnames like `localhost`.
/// Returns `false` on parse errors (conservative: unknown = non-loopback).
fn is_loopback_addr(addr: &str) -> bool {
    let host = if let Some(rest) = addr.strip_prefix('[') {
        if let Some(bracket_end) = rest.find(']') {
            &rest[..bracket_end]
        } else {
            addr
        }
    } else if let Some(colon) = addr.rfind(':') {
        &addr[..colon]
    } else {
        addr
    };

    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return ip.is_loopback();
    }

    host.eq_ignore_ascii_case("localhost")
}

/// Validate that a bind address is safe to expose.
///
/// Loopback addresses are always allowed. Public addresses require either
/// authentication or an explicit acknowledgement of the risk.
pub fn validate_bind_safety(addr: &str, serve: &ServeConfig) -> Result<()> {
    if is_loopback_addr(addr) || serve.auth.enabled {
        return Ok(());
    }

    if serve.acknowledge_public_risk {
        warn!(
            addr = %addr,
            "binding to a public address without authentication; all routes will be network-accessible"
        );
        return Ok(());
    }

    anyhow::bail!(
        "Public bind requires `serve.auth.enabled = true` or `serve.acknowledge_public_risk = true`.\n\
         Attempted to bind to: {addr}\n\
         Set `[serve] auth.enabled = true` for authenticated public access, or\n\
         set `[serve] acknowledge_public_risk = true` to proceed anyway."
    );
}

/// Run the HTTP server against an already constructed [`AppState`].
///
/// # Errors
///
/// Returns an error if the listener cannot bind to `bind:port` or if the
/// Axum server exits with an error.
pub async fn run_server_with_state(state: Arc<AppState>, bind: &str, port: u16) -> Result<()> {
    let addr = format!("{bind}:{port}");
    let roko_config = state.load_roko_config();
    validate_bind_safety(&addr, &roko_config.serve)?;
    if let Err(err) = state.restore_snapshot().await {
        warn!(error = %err, "failed to restore server state snapshot; starting fresh");
    }
    let roko_config = roko_config.as_ref().clone();
    let _config_watcher = config_watcher::start_config_watcher(Arc::clone(&state));
    let _prd_publish_subscriber = start_prd_publish_orchestrator(Arc::clone(&state));
    let _state_hub_bridge = start_state_hub_bridge(Arc::clone(&state));
    // NOTE: start_orchestrator_event_bridge is intentionally NOT started here.
    // It creates a feedback loop: EventBus → StateHub → EventBus → ∞.
    let _state_saver = start_state_snapshot_saver(Arc::clone(&state));
    let _job_runner = job_runner::start_job_runner(Arc::clone(&state));
    let router = build_server_router(
        Arc::clone(&state),
        &roko_config.server.cors_origins,
        roko_config.server.unsafe_public_cors,
        roko_config.serve.auth.clone(),
    );
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

fn build_server_router(
    state: Arc<AppState>,
    cors_origins: &[String],
    unsafe_public_cors: bool,
    api_auth: roko_core::config::ServeAuthConfig,
) -> axum::Router {
    // `routes::build_router` currently installs only the top-level SPA fallback.
    // Reset it here so the final fallback can distinguish API/WS typos from browser routes.
    let api_router =
        routes::build_router(Arc::clone(&state), cors_origins, api_auth).reset_fallback();
    let fallback_router = axum::Router::new()
        .fallback(serve_api_or_spa_fallback)
        .layer(TraceLayer::new_for_http())
        .layer(routes::cors_layer(cors_origins, unsafe_public_cors))
        .with_state(state);

    api_router.merge(fallback_router)
}

fn build_cors_layer(cors_origins: &[String]) -> CorsLayer {
    if cors_origins.is_empty() {
        CorsLayer::permissive()
    } else {
        let allowed: Vec<axum::http::HeaderValue> = cors_origins
            .iter()
            .filter_map(|origin| origin.parse().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(allowed)
            .allow_methods(Any)
            .allow_headers(Any)
    }
}

fn api_or_ws_path_requires_json_404(path: &str) -> bool {
    matches!(path, "/api" | "/ws" | "/roko-ws")
        || path.starts_with("/api/")
        || path.starts_with("/ws/")
        || path.starts_with("/roko-ws/")
}

async fn serve_api_or_spa_fallback(req: axum::extract::Request) -> axum::response::Response {
    let path = req.uri().path().to_string();
    if api_or_ws_path_requires_json_404(&path) {
        return (
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({
                "error": "not_found",
                "message": format!("No route matches {path}"),
            })),
        )
            .into_response();
    }

    crate::embedded::serve_embedded(req).await
}

fn log_provider_credential_status(config: &RokoConfig) {
    let available = config.available_provider_ids();
    let mut missing: Vec<String> = config
        .effective_providers()
        .iter()
        .filter(|(_, p)| !config.is_provider_available(p))
        .map(|(id, _)| id.clone())
        .collect();
    missing.sort();
    info!(providers = ?available, "providers with API credentials (or CLI backends)");
    if !missing.is_empty() {
        warn!(
            providers = ?missing,
            "providers missing credentials — models on these providers cannot dispatch until env vars or [agent.env] are set"
        );
    }
}

fn build_app_state(
    workdir: PathBuf,
    runtime: Arc<dyn CliRuntime>,
    mut roko_config: RokoConfig,
) -> anyhow::Result<AppState> {
    // Auto-configure Privy JWT auth: always set the app ID (it's a project
    // constant) and auto-enable auth when a stored Privy credential exists.
    if roko_config.serve.auth.privy_app_id.is_none() {
        roko_config.serve.auth.privy_app_id = Some(crate::jwks::NUNCHI_PRIVY_APP_ID.to_string());
    }
    if !roko_config.serve.auth.enabled {
        // Only auto-enable auth for non-loopback binds. Local dev (127.0.0.1 /
        // localhost) should respect the explicit `enabled = false` in roko.toml.
        let bind = &roko_config.server.bind;
        let is_loopback =
            bind == "127.0.0.1" || bind == "::1" || bind.eq_ignore_ascii_case("localhost");
        if !is_loopback {
            if let Ok(Some(cred)) = load_stored_credential() {
                if cred.get("method").and_then(|v| v.as_str()) == Some("privy") {
                    info!("Privy credential found — enabling auth for public bind");
                    roko_config.serve.auth.enabled = true;
                }
            }
        }
    }
    log_provider_credential_status(&roko_config);
    let deploy_backend = create_deploy_backend(&roko_config);
    let state = AppState::new(workdir, runtime, roko_config, deploy_backend)?;

    // Warm the cached cascade router once so gateway selection reuses the
    // persisted bandit state instead of rebuilding it on the first request.
    {
        let config = state.load_roko_config();
        let mut model_slugs: Vec<String> = config.model_slugs_for_cascade();
        model_slugs.sort();

        if !model_slugs.is_empty() {
            let router_path = state.layout.cascade_router_path();
            if !router_path.exists() {
                warn!(
                    path = %router_path.display(),
                    "persisted CascadeRouter not found; using fresh router"
                );
            }
            let router =
                roko_learn::cascade_router::CascadeRouter::load_or_new(&router_path, model_slugs);
            let observations = router.total_observations();

            tokio::task::block_in_place(|| {
                *state.cascade_router.blocking_write() = Some(router);
            });

            if observations > 0 {
                info!(
                    observations = observations,
                    path = %router_path.display(),
                    "loaded persisted CascadeRouter"
                );
            } else {
                debug!(path = %router_path.display(), "initialized fresh CascadeRouter");
            }
        }
    }

    let _ = state.state_hub.bootstrap_from_workdir(&state.workdir);
    if let Some(snapshot_json) = tokio::task::block_in_place(|| {
        state
            .cascade_router
            .blocking_read()
            .as_ref()
            .map(|router| router.snapshot_json())
    }) {
        let mut snapshot = state.state_hub.current_snapshot();
        snapshot.cascade_router_json = snapshot_json;
        state.state_hub.apply_snapshot(snapshot);
    }
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
    // Seed StateHub with knowledge entries from the neuro store.
    let knowledge = scan_knowledge_entries(&state.workdir);
    if !knowledge.is_empty() {
        info!(
            count = knowledge.len(),
            "loaded existing knowledge entries from neuro store"
        );
        state
            .state_hub
            .publish(DashboardEvent::KnowledgeEntriesUpdated { entries: knowledge });
    }

    // Seed connector and feed registries with default entries so routes
    // return real data instead of empty arrays (audit finding A3).
    seed_default_registries(&state);

    Ok(state)
}

/// Populate the connector and feed registries with default entries that
/// reflect the actual on-disk data files. Called once during server startup
/// before the `AppState` is shared behind an `Arc`.
///
/// Connectors registered:
/// - **filesystem**: the local `.roko/` data directory (`Database` kind)
/// - **neuro-store**: the durable knowledge store (`Database` kind)
///
/// Feeds registered:
/// - **engrams**: `.roko/engrams.jsonl` — raw signal/engram log (`Raw` kind)
/// - **episodes**: `.roko/memory/episodes.jsonl` — agent turn episodes (`Raw` kind)
/// - **efficiency**: `.roko/learn/efficiency.jsonl` — per-turn metrics (`Derived` kind)
/// - **knowledge**: neuro knowledge store entries (`Composite` kind)
fn seed_default_registries(state: &AppState) {
    // Use block_in_place so blocking_write doesn't panic inside a tokio runtime.
    // This is safe because build_app_state is called once at startup.
    tokio::task::block_in_place(|| seed_default_registries_inner(state));
}

fn seed_default_registries_inner(state: &AppState) {
    let now = chrono::Utc::now();
    let layout = &state.layout;

    // ── Connectors ────────────────────────────────────────────────────
    let mut connectors = state.connectors.blocking_write();

    let roko_root = layout.root().to_string_lossy().to_string();
    connectors.register(ConnectorInfo {
        name: "filesystem".to_string(),
        kind: ConnectorKind::Database,
        health: ConnectorHealth {
            status: ConnectorStatus::Connected,
            latency_ms: 0,
            last_check: now,
        },
        created_at: now,
        metadata: serde_json::json!({
            "description": "Local .roko/ data directory",
            "path": roko_root,
        }),
    });

    let neuro_path = layout.root().join("neuro");
    connectors.register(ConnectorInfo {
        name: "neuro-store".to_string(),
        kind: ConnectorKind::Database,
        health: ConnectorHealth {
            status: if neuro_path.exists() {
                ConnectorStatus::Connected
            } else {
                ConnectorStatus::Disconnected
            },
            latency_ms: 0,
            last_check: now,
        },
        created_at: now,
        metadata: serde_json::json!({
            "description": "Durable knowledge store (neuro)",
            "path": neuro_path.to_string_lossy(),
        }),
    });

    let connector_count = connectors.list().len();
    drop(connectors);

    // ── Feeds ─────────────────────────────────────────────────────────
    let mut feeds = state.feeds.blocking_write();

    let engrams_path = layout.engrams_path();
    feeds.register(FeedInfo {
        id: String::new(), // assigned by registry
        name: "engrams".to_string(),
        kind: FeedKind::Raw,
        access: FeedAccess::Public,
        agent_id: "system".to_string(),
        description: "Raw signal/engram log (.roko/engrams.jsonl)".to_string(),
        schema: None,
        created_at: now,
    });

    let episodes_path = layout.episodes_path();
    feeds.register(FeedInfo {
        id: String::new(),
        name: "episodes".to_string(),
        kind: FeedKind::Raw,
        access: FeedAccess::Public,
        agent_id: "system".to_string(),
        description: "Agent turn episode log (.roko/memory/episodes.jsonl)".to_string(),
        schema: None,
        created_at: now,
    });

    let efficiency_path = layout.efficiency_path();
    feeds.register(FeedInfo {
        id: String::new(),
        name: "efficiency".to_string(),
        kind: FeedKind::Derived,
        access: FeedAccess::Public,
        agent_id: "system".to_string(),
        description: "Per-turn efficiency metrics (.roko/learn/efficiency.jsonl)".to_string(),
        schema: None,
        created_at: now,
    });

    feeds.register(FeedInfo {
        id: String::new(),
        name: "knowledge".to_string(),
        kind: FeedKind::Composite,
        access: FeedAccess::Public,
        agent_id: "system".to_string(),
        description: "Durable knowledge entries from the neuro store".to_string(),
        schema: None,
        created_at: now,
    });

    let feed_count = feeds.list().len();
    drop(feeds);

    info!(
        connectors = connector_count,
        feeds = feed_count,
        engrams_path = %engrams_path.display(),
        episodes_path = %episodes_path.display(),
        efficiency_path = %efficiency_path.display(),
        "seeded default connector and feed registries"
    );
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
                debug!(
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

/// Load knowledge entries from the neuro JSONL store and project them into
/// lightweight `KnowledgeBrowseEntry` summaries for the dashboard snapshot.
fn scan_knowledge_entries(
    workdir: &Path,
) -> Vec<roko_core::dashboard_snapshot::KnowledgeBrowseEntry> {
    let store = roko_neuro::knowledge_store::KnowledgeStore::for_workdir(workdir);
    let entries = match store.query("*", 200) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    entries
        .into_iter()
        .map(|entry| {
            let preview = if entry.content.len() > 200 {
                format!("{}…", &entry.content[..200])
            } else {
                entry.content.clone()
            };
            let tier_str = match entry.tier {
                roko_neuro::KnowledgeTier::Transient => "transient",
                roko_neuro::KnowledgeTier::Working => "working",
                roko_neuro::KnowledgeTier::Consolidated => "consolidated",
                roko_neuro::KnowledgeTier::Persistent => "persistent",
            };
            roko_core::dashboard_snapshot::KnowledgeBrowseEntry {
                id: entry.id,
                kind: entry.kind.as_str().to_string(),
                content_preview: preview,
                confidence: entry.confidence,
                tier: tier_str.to_string(),
                tags: entry.tags,
                created_at: entry.created_at,
                frozen: false,
            }
        })
        .collect()
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
        ServerEvent::AgentSpawned {
            agent_id,
            role,
            model,
        } => Some(DashboardEvent::AgentSpawned {
            agent_id: agent_id.clone(),
            role: role.clone(),
            model: dashboard_model_label(model, agent_id),
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
            ExecutionEvent::TaskStarted {
                task_id,
                title,
                phase,
            } => Some(DashboardEvent::TaskStarted {
                plan_id: plan_id.clone(),
                task_id: task_id.clone(),
                title: title.clone(),
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
        // Map one-shot runs as ephemeral plans so the TUI's plan/task views show them.
        ServerEvent::RunStarted { run_id, .. } => Some(DashboardEvent::PlanStarted {
            plan_id: format!("run-{run_id}"),
        }),
        ServerEvent::RunCompleted { run_id, success } => Some(DashboardEvent::PlanCompleted {
            plan_id: format!("run-{run_id}"),
            success: *success,
        }),
        // Map agent lifecycle events from the supervisor.
        ServerEvent::AgentStarted { agent_id, .. } => Some(DashboardEvent::AgentSpawned {
            agent_id: agent_id.clone(),
            role: String::new(),
            model: dashboard_model_label("", agent_id),
        }),
        ServerEvent::AgentStopped { agent_id, .. } => Some(DashboardEvent::AgentCompleted {
            agent_id: agent_id.clone(),
        }),
        _ => None,
    }
}

fn dashboard_model_label(model: &str, fallback: &str) -> String {
    let model = model.trim();
    if model.is_empty() {
        fallback.to_string()
    } else {
        model.to_string()
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
            title,
            phase,
        } => Some(ServerEvent::Execution {
            plan_id: plan_id.clone(),
            event: ExecutionEvent::TaskStarted {
                task_id: task_id.clone(),
                title: title.clone(),
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
        DashboardEvent::AgentSpawned {
            agent_id,
            role,
            model,
        } => Some(ServerEvent::AgentSpawned {
            agent_id: agent_id.clone(),
            role: role.clone(),
            model: model.clone(),
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

/// Periodic garbage collection of ephemeral workspaces.
///
/// Runs every 5 minutes. Each tick removes entries from
/// `AppState.ephemeral_workspaces` whose `created_at` is older than 1 hour,
/// deleting the corresponding filesystem directories.
fn start_workspace_gc(state: Arc<AppState>) -> JoinHandle<()> {
    const INTERVAL_SECS: u64 = roko_core::defaults::DEFAULT_WORKSPACE_GC_INTERVAL_SECS;
    const MAX_AGE_SECS: u64 = 3600;

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(INTERVAL_SECS));
        // Skip the first immediate tick — let the server warm up.
        interval.tick().await;

        loop {
            tokio::select! {
                _ = state.cancel.cancelled() => break,
                _ = interval.tick() => {}
            }

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            let stale: Vec<crate::state::WorkspaceInfo> = {
                let map = state.ephemeral_workspaces.read().await;
                map.values()
                    .filter(|ws| now.saturating_sub(ws.created_at) > MAX_AGE_SECS)
                    .cloned()
                    .collect()
            };

            if stale.is_empty() {
                continue;
            }

            let mut removed = 0usize;
            for ws in &stale {
                if let Err(err) = tokio::fs::remove_dir_all(&ws.path).await {
                    warn!(
                        workspace_id = %ws.id,
                        path = %ws.path.display(),
                        error = %err,
                        "failed to remove stale ephemeral workspace directory"
                    );
                }
            }

            {
                let mut map = state.ephemeral_workspaces.write().await;
                for ws in &stale {
                    if map.remove(&ws.id).is_some() {
                        removed += 1;
                    }
                }
            }

            info!(
                count = removed,
                "workspace GC: removed {removed} stale ephemeral workspace(s)"
            );
        }
    })
}

/// Periodic GC for completed/failed operation handles (§15.6).
///
/// Without this, `active_runs`, `active_plans`, and `operations` HashMaps
/// grow unboundedly as completed JoinHandles accumulate.
fn start_handle_gc(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        interval.tick().await; // skip first immediate tick

        loop {
            tokio::select! {
                _ = state.cancel.cancelled() => break,
                _ = interval.tick() => {}
            }
            state.gc_completed_handles().await;
        }
    })
}

/// Periodic cold archival: migrates aged-out engrams from the hot substrate
/// (`.roko/engrams.jsonl` / `FileSubstrate`) to compressed monthly JSONL
/// archives in `.roko/cold/`.
///
/// Runs every hour (default) or at the interval specified in the
/// `archival_interval_secs` field. Each tick:
///  1. Opens the hot `FileSubstrate`.
///  2. Queries for engrams older than 7 days (default).
///  3. Batch-archives them to `ArchiveColdSubstrate`.
///  4. Applies retention compaction on observability artifacts.
///
/// Failures are logged but never crash the server.
fn start_cold_archival_timer(state: Arc<AppState>) -> JoinHandle<()> {
    // Default: run every hour.
    const DEFAULT_INTERVAL_SECS: u64 = 3600;
    // Default: archive engrams older than 7 days.
    const DEFAULT_MAX_AGE_MS: i64 = 7 * 24 * 3600 * 1000;
    // Default: archive up to 500 engrams per tick.
    const DEFAULT_BATCH_SIZE: usize = 500;

    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(DEFAULT_INTERVAL_SECS));

        // Skip the first immediate tick — let the server warm up.
        interval.tick().await;

        loop {
            tokio::select! {
                _ = state.cancel.cancelled() => break,
                _ = interval.tick() => {}
            }

            let workdir = &state.workdir;
            let roko_dir = workdir.join(".roko");
            if !roko_dir.exists() {
                continue;
            }

            // -- Phase 1: cold-archive aged-out engrams ----------------------
            match run_cold_archival_tick(&roko_dir, DEFAULT_MAX_AGE_MS, DEFAULT_BATCH_SIZE).await {
                Ok(0) => {
                    debug!("cold archival tick: no engrams to archive");
                }
                Ok(n) => {
                    info!(
                        "cold archival tick: archived {n} engram(s) to {}",
                        roko_dir.join("cold").display()
                    );
                }
                Err(err) => {
                    warn!(error = %err, "cold archival tick failed");
                }
            }

            // -- Phase 2: apply retention compaction -------------------------
            let actions = retention::apply_retention(workdir, false);
            for action in &actions {
                info!(
                    artifact = %action.artifact,
                    action = ?action.action,
                    "retention compaction applied"
                );
            }
        }
    })
}

/// Execute a single cold-archival tick: query old engrams from the hot
/// substrate and archive them to `.roko/cold/`.
///
/// Returns the number of engrams archived, or an error.
async fn run_cold_archival_tick(
    roko_dir: &std::path::Path,
    max_age_ms: i64,
    batch_size: usize,
) -> anyhow::Result<usize> {
    use roko_core::{ColdStore, Context, Query, Store};

    let hot = roko_fs::FileSubstrate::open(roko_dir).await?;
    let ctx = Context::now();
    let cutoff_ms = chrono::Utc::now().timestamp_millis() - max_age_ms;
    let query = Query::all().until(cutoff_ms).limit(batch_size);
    let candidates = hot.query(&query, &ctx).await?;

    if candidates.is_empty() {
        return Ok(0);
    }

    let cold_dir = roko_dir.join("cold");
    let cold = roko_fs::ArchiveColdSubstrate::open(&cold_dir).await?;
    let archived = cold.archive_batch(candidates).await?;
    Ok(archived)
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

/// Read `~/.roko/credentials.json` and return the "default" profile as a
/// raw JSON value. This avoids a dependency on roko-cli's `Credential` type.
fn load_stored_credential() -> Result<Option<serde_json::Value>> {
    let path = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".roko")
        .join("credentials.json");
    if !path.exists() {
        return Ok(None);
    }
    let data = std::fs::read_to_string(&path)?;
    let store: serde_json::Value = serde_json::from_str(&data)?;
    Ok(store.get("default").cloned())
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

#[cfg(test)]
mod tests {
    use super::{build_app_state, resolve_bind_with_port_env, serve_api_or_spa_fallback};

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header::CONTENT_TYPE};
    use roko_gate::AdaptiveThresholds;
    use roko_learn::cascade_router::CascadeRouter;
    use roko_learn::model_router::CONTEXT_DIM;
    use serde_json::Value;
    use std::sync::Arc;
    use tempfile::tempdir;

    use crate::runtime::NoOpRuntime;

    async fn fallback_response(path: &str) -> axum::response::Response {
        let request = Request::builder()
            .uri(path)
            .body(Body::empty())
            .expect("build request");
        serve_api_or_spa_fallback(request).await
    }

    #[tokio::test]
    async fn api_paths_return_json_404() {
        let response = fallback_response("/api/nonexistent").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .expect("content type");
        assert!(content_type.starts_with("application/json"));

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let json: Value = serde_json::from_slice(&body).expect("parse body");
        assert_eq!(json["error"].as_str(), Some("not_found"));
        assert_eq!(
            json["message"].as_str(),
            Some("No route matches /api/nonexistent")
        );
    }

    #[tokio::test]
    async fn ws_paths_return_json_404() {
        let response = fallback_response("/ws/nonexistent").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .expect("content type");
        assert!(content_type.starts_with("application/json"));
    }

    #[tokio::test]
    async fn non_api_paths_still_serve_spa_html() {
        let response = fallback_response("/nonexistent-page").await;
        assert_eq!(response.status(), StatusCode::OK);

        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .expect("content type");
        assert!(content_type.starts_with("text/html"));
    }

    #[tokio::test]
    async fn build_app_state_loads_persisted_learning_state_and_falls_back_cleanly() {
        let persisted_dir = tempdir().expect("tempdir");
        let persisted_workdir = persisted_dir.path().to_path_buf();
        let persisted_learn_dir = persisted_workdir.join(".roko").join("learn");
        std::fs::create_dir_all(&persisted_learn_dir).expect("create learn dir");

        let thresholds_path = persisted_learn_dir.join("gate-thresholds.json");
        let mut thresholds = AdaptiveThresholds::new();
        thresholds.update(1, true);
        thresholds.save(&thresholds_path).expect("seed thresholds");

        let router_path = persisted_learn_dir.join("cascade-router.json");
        let router = CascadeRouter::new(vec!["claude-sonnet-4-6".to_string()]);
        router.observe(vec![0.0; CONTEXT_DIM], 0, 1.0);
        router.save(&router_path).expect("seed router");

        let persisted_state = build_app_state(
            persisted_workdir.clone(),
            Arc::new(NoOpRuntime),
            roko_core::config::schema::RokoConfig::default(),
        )
        .expect("build_app_state");

        let persisted_router = persisted_state.cascade_router.read().await;
        let persisted_router = persisted_router.as_ref().expect("router loaded");
        assert_eq!(persisted_router.total_observations(), 1);

        let thresholds_snapshot = persisted_state.state_hub.current_snapshot();
        let expected_thresholds =
            std::fs::read_to_string(&thresholds_path).expect("read seeded thresholds");
        assert_eq!(
            thresholds_snapshot.gate_thresholds_json,
            expected_thresholds
        );

        let fresh_dir = tempdir().expect("tempdir");
        let fresh_state = build_app_state(
            fresh_dir.path().to_path_buf(),
            Arc::new(NoOpRuntime),
            roko_core::config::schema::RokoConfig::default(),
        )
        .expect("build_app_state");

        let fresh_router = fresh_state.cascade_router.read().await;
        let fresh_router = fresh_router.as_ref().expect("fresh router initialized");
        assert_eq!(fresh_router.total_observations(), 0);
    }

    #[tokio::test]
    async fn shutdown_persists_cascade_router_state() {
        let dir = tempdir().expect("tempdir");
        let workdir = dir.path().to_path_buf();
        let state = build_app_state(
            workdir.clone(),
            Arc::new(NoOpRuntime),
            roko_core::config::schema::RokoConfig::default(),
        )
        .expect("build_app_state");

        let router = CascadeRouter::new(vec!["claude-sonnet-4-6".to_string()]);
        router.observe(vec![0.0; CONTEXT_DIM], 0, 1.0);
        {
            let mut guard = state.cascade_router.write().await;
            *guard = Some(router);
        }

        state.shutdown().await;

        let reloaded = CascadeRouter::load_or_new(
            &state.layout.cascade_router_path(),
            vec!["claude-sonnet-4-6".to_string()],
        );
        assert_eq!(reloaded.total_observations(), 1);
    }

    /// T3-25: a `PORT` env override must replace **only** the port, leaving
    /// the configured bind (default `127.0.0.1`) intact. Cloud platforms set
    /// `PORT` to choose a port; they do not (and should not) imply a public
    /// bind.
    #[test]
    fn port_env_override_keeps_loopback_bind_by_default() {
        let (bind, port) = resolve_bind_with_port_env(
            "127.0.0.1", // server.bind default
            None,        // no CLI --bind
            6677,        // server.port default
            None,        // no CLI --port
            Some("8080"),
        )
        .expect("resolve");
        assert_eq!(bind, "127.0.0.1");
        assert_eq!(port, 8080);
    }

    /// Operators who explicitly set `[server].bind = "0.0.0.0"` get the
    /// public bind they asked for, with the `PORT`-supplied port.
    #[test]
    fn port_env_override_respects_explicit_public_bind() {
        let (bind, port) =
            resolve_bind_with_port_env("0.0.0.0", None, 6677, None, Some("8080")).expect("resolve");
        assert_eq!(bind, "0.0.0.0");
        assert_eq!(port, 8080);
    }

    /// CLI overrides (the `bind`/`port` arguments threaded through
    /// `ServerBuildConfig`) take precedence over both config and the `PORT`
    /// env var's bind half — but `PORT` still wins for the port number when
    /// it is set.
    #[test]
    fn port_env_override_respects_cli_bind_override() {
        let (bind, port) =
            resolve_bind_with_port_env("127.0.0.1", Some("10.0.0.5"), 6677, Some(7777), Some("80"))
                .expect("resolve");
        assert_eq!(bind, "10.0.0.5");
        // PORT env still wins over the CLI port override (matches existing
        // semantics — cloud platforms set PORT *because* they pick the port).
        assert_eq!(port, 80);
    }

    /// Without `PORT`, both bind and port come from the CLI override (or
    /// fall through to config defaults).
    #[test]
    fn no_port_env_falls_back_to_cli_or_config() {
        let (bind, port) = resolve_bind_with_port_env("127.0.0.1", None, 6677, None, None)
            .expect("resolve fallback");
        assert_eq!(bind, "127.0.0.1");
        assert_eq!(port, 6677);

        let (bind, port) =
            resolve_bind_with_port_env("127.0.0.1", Some("0.0.0.0"), 6677, Some(9999), None)
                .expect("resolve overrides");
        assert_eq!(bind, "0.0.0.0");
        assert_eq!(port, 9999);
    }

    #[test]
    fn invalid_port_env_returns_error() {
        let err = resolve_bind_with_port_env("127.0.0.1", None, 6677, None, Some("not-a-port"))
            .expect_err("non-numeric PORT must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("PORT env var must be a valid u16"),
            "unexpected error: {msg}"
        );
    }
}
