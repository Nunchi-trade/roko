//! HTTP server for the roko API.
//!
//! The [`run_server`] function is the entry point invoked by `roko serve`.
//! It loads configuration, constructs shared [`state::AppState`], builds the
//! [`axum::Router`] via [`routes::build_router`], and runs the server with
//! graceful shutdown on ctrl-c.

pub mod deploy;
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

use crate::config::Config;
use state::AppState;

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
    // -- Load configuration -------------------------------------------------

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
    let config: Config = if cli_config_path.exists() {
        Config::from_file(&cli_config_path)
            .with_context(|| format!("load CLI config {}", cli_config_path.display()))?
    } else {
        Config::default()
    };

    // -- Apply CLI overrides ------------------------------------------------

    let server_bind = bind.unwrap_or_else(|| roko_config.server.bind.clone());
    let server_port = port.unwrap_or(roko_config.server.port);

    // -- PORT env var override (Railway / cloud platforms) -------------------
    let (effective_bind, effective_port) = if let Ok(env_port) = std::env::var("PORT") {
        let p: u16 = env_port
            .parse()
            .context("PORT env var must be a valid u16")?;
        info!("PORT env var detected ({p}), binding to 0.0.0.0:{p}");
        ("0.0.0.0".to_string(), p)
    } else {
        (server_bind, server_port)
    };

    // -- Build shared state and router --------------------------------------

    let cors_origins = roko_config.server.cors_origins.clone();

    // -- Create deploy backend from config ------------------------------------
    let deploy_backend: Arc<dyn deploy::DeployBackend> = {
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
    };

    let state = Arc::new(AppState::new(
        workdir.clone(),
        config,
        roko_config,
        deploy_backend,
    ));
    let router = routes::build_router(Arc::clone(&state), &cors_origins);

    // -- Bind and serve -----------------------------------------------------

    let addr = format!("{effective_bind}:{effective_port}");
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("bind to {addr}"))?;

    info!("roko server listening on http://{addr}");
    info!("workdir: {}", workdir.display());

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal(Arc::clone(&state)))
        .await
        .context("axum server error")?;

    info!("server stopped");
    Ok(())
}

/// Wait for ctrl-c then trigger graceful shutdown.
async fn shutdown_signal(state: Arc<AppState>) {
    let _ = tokio::signal::ctrl_c().await;
    info!("received ctrl-c, shutting down");
    state.shutdown().await;
}
