//! HTTP server for the roko API.
//!
//! The [`ServerBuilder`] type is the main entrypoint for embedding the HTTP
//! server. [`run_server`] remains as a convenience wrapper for the current
//! CLI flow.

pub mod deploy;
pub mod dispatch;
pub mod feedback;
pub mod event_bus;
pub mod error;
pub mod events;
pub mod routes;
pub mod state;
pub mod templates;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::net::TcpListener;
use tracing::{info, warn};

use roko_core::config::schema::RokoConfig;

use roko_cli::Config;
use state::AppState;

/// Inputs required to start the HTTP server.
#[derive(Clone)]
pub struct ServerBuildConfig {
    /// Project working directory.
    pub workdir: PathBuf,
    /// CLI-level configuration (`.roko/config.toml`).
    pub cli_config: Config,
    /// Full `roko.toml` schema configuration.
    pub roko_config: RokoConfig,
    /// Optional bind address override.
    pub bind: Option<String>,
    /// Optional port override.
    pub port: Option<u16>,
}

impl ServerBuildConfig {
    /// Create a new server build configuration.
    #[must_use]
    pub fn new(
        workdir: PathBuf,
        cli_config: Config,
        roko_config: RokoConfig,
        bind: Option<String>,
        port: Option<u16>,
    ) -> Self {
        Self {
            workdir,
            cli_config,
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
        let cli_config = self.config.cli_config.clone();
        let roko_config = self.config.roko_config.clone();
        let state = Arc::clone(self.state.get_or_insert_with(|| {
            Arc::new(build_app_state(workdir, cli_config, roko_config))
        }));
        let dispatcher = Arc::new(dispatch::TemplateAgentDispatcher::new(
            state.workdir.clone(),
        ));
        tokio::spawn(dispatch::dispatch_loop(Arc::clone(&state), dispatcher));
        let _feedback_loop = feedback::start_feedback_loop(Arc::clone(&state));
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
/// # Arguments
///
/// * `workdir`  – Project working directory (must contain `roko.toml` or
///   defaults will be used).
/// * `bind`     – Optional bind address override (takes precedence over
///   `roko.toml` `[server].bind`).
/// * `port`     – Optional port override (takes precedence over
///   `roko.toml` `[server].port`).
pub async fn run_server(workdir: PathBuf, bind: Option<String>, port: Option<u16>) -> Result<()> {
    let config = load_server_build_config(workdir, bind, port)?;
    ServerBuilder::new(config).run().await
}

fn load_server_build_config(
    workdir: PathBuf,
    bind: Option<String>,
    port: Option<u16>,
) -> Result<ServerBuildConfig> {
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

    let cli_config_path = workdir.join(".roko").join("config.toml");
    let cli_config: Config = if cli_config_path.exists() {
        Config::from_file(&cli_config_path)
            .with_context(|| format!("load CLI config {}", cli_config_path.display()))?
    } else {
        Config::default()
    };

    Ok(ServerBuildConfig::new(
        workdir,
        cli_config,
        roko_config,
        bind,
        port,
    ))
}

fn build_app_state(
    workdir: PathBuf,
    cli_config: Config,
    roko_config: RokoConfig,
) -> AppState {
    let deploy_backend = create_deploy_backend(&roko_config);
    AppState::new(workdir, cli_config, roko_config, deploy_backend)
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
