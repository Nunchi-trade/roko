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

use anyhow::{Context, Result};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use roko_core::Engram;
use roko_core::config::schema::RokoConfig;
use roko_core::connector::{ConnectorHealth, ConnectorInfo, ConnectorKind, ConnectorStatus};
use roko_core::dashboard_snapshot::DashboardEvent;
use roko_core::feed::{FeedAccess, FeedInfo, FeedKind};
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

    /// Start the server in the background and return the live state handle.
    ///
    /// The returned [`Arc<AppState>`] carries the [`SharedStateHub`] that the
    /// TUI or other in-process consumers can subscribe to.  The
    /// [`JoinHandle`] resolves when the server shuts down (e.g. because
    /// `state.cancel.cancel()` was called).
    pub async fn start_background(mut self) -> Result<(Arc<AppState>, JoinHandle<Result<()>>)> {
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
        let _state_saver = start_state_snapshot_saver(Arc::clone(&state));
        let _job_runner = job_runner::start_job_runner(Arc::clone(&state));
        let _cold_archival = start_cold_archival_timer(Arc::clone(&state));

        // Load persisted deployments from disk.
        routes::load_persisted_deployments(&state).await;

        // Eagerly prime the JWKS cache if Privy auth is configured.
        if state.load_roko_config().serve.auth.privy_app_id.is_some() {
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
                let (stdout_target, stderr_target) = if let Ok(f) =
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&log_path)
                {
                    let f2 = f.try_clone().unwrap_or_else(|_| {
                        std::fs::File::open("/dev/null").expect("/dev/null")
                    });
                    (std::process::Stdio::from(f), std::process::Stdio::from(f2))
                } else {
                    (
                        std::process::Stdio::null(),
                        std::process::Stdio::null(),
                    )
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

/// Load [`RokoConfig`] from the workdir's `roko.toml`, falling back to the
/// global `~/.roko/config.toml` if no project config exists.
fn load_roko_config(workdir: &Path) -> Result<RokoConfig> {
    let project_path = workdir.join("roko.toml");
    if project_path.exists() {
        let text = std::fs::read_to_string(&project_path)
            .with_context(|| format!("read {}", project_path.display()))?;
        return toml::from_str(&text).with_context(|| format!("parse {}", project_path.display()));
    }

    // No project config — try global ~/.roko/config.toml.
    if let Ok(home) = std::env::var("HOME") {
        let global_path = PathBuf::from(&home).join(".roko").join("config.toml");
        if global_path.exists() {
            let text = std::fs::read_to_string(&global_path)
                .with_context(|| format!("read {}", global_path.display()))?;
            info!("using global config: {}", global_path.display());
            return toml::from_str(&text)
                .with_context(|| format!("parse {}", global_path.display()));
        }
    }

    warn!(
        "no roko.toml found at {} and no global config; using defaults",
        project_path.display()
    );
    Ok(RokoConfig::default())
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
    let roko_config = load_roko_config(&workdir)?;
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
    let roko_config = load_roko_config(&workdir)?;
    let config = ServerBuildConfig::new(workdir, runtime, roko_config, bind, port);
    ServerBuilder::new(config).start_background().await
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
    mut roko_config: RokoConfig,
) -> AppState {
    // Auto-configure Privy JWT auth: always set the app ID (it's a project
    // constant) and auto-enable auth when a stored Privy credential exists.
    if roko_config.serve.auth.privy_app_id.is_none() {
        roko_config.serve.auth.privy_app_id = Some(crate::jwks::NUNCHI_PRIVY_APP_ID.to_string());
    }
    if !roko_config.serve.auth.enabled {
        if let Ok(Some(cred)) = load_stored_credential() {
            if cred.get("method").and_then(|v| v.as_str()) == Some("privy") {
                info!("Privy credential found — enabling auth");
                roko_config.serve.auth.enabled = true;
            }
        }
    }
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

    state
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
