//! Worker mode: thin HTTP server that executes agent templates.
//!
//! Started by `roko worker`, typically inside a deployed container.
//! Reads its configuration from environment variables:
//! - `ROKO_TEMPLATE_JSON`: Base64-encoded [`AgentTemplate`] JSON
//! - `ROKO_CONTROL_PLANE_URL`: Optional callback URL for result reporting
//! - `ROKO_DEPLOYMENT_ID`: Deployment identifier for callbacks
//! - `PORT`: Listen port (Railway injects this)

pub mod handler;

use std::sync::Arc;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use tokio::net::TcpListener;
use tracing::info;

use crate::serve::templates::AgentTemplate;

/// Shared state for the worker server.
pub struct WorkerState {
    /// The decoded agent template.
    pub template: AgentTemplate,
    /// Optional control plane URL for callbacks.
    pub control_plane_url: Option<String>,
    /// Deployment ID for callback identification.
    pub deployment_id: Option<String>,
    /// Server start time.
    pub started_at: std::time::Instant,
    /// Last task status (for /status endpoint).
    pub last_task: tokio::sync::RwLock<Option<handler::TaskResult>>,
}

/// Entry point for `roko worker`.
pub async fn run_worker(port: u16) -> Result<()> {
    // Decode template from env
    let template_b64 =
        std::env::var("ROKO_TEMPLATE_JSON").context("ROKO_TEMPLATE_JSON env var is required")?;
    let template_bytes = BASE64
        .decode(template_b64.as_bytes())
        .context("ROKO_TEMPLATE_JSON is not valid base64")?;
    let template: AgentTemplate = serde_json::from_slice(&template_bytes)
        .context("ROKO_TEMPLATE_JSON does not contain valid AgentTemplate JSON")?;

    info!(template = %template.name, "decoded agent template");

    let control_plane_url = std::env::var("ROKO_CONTROL_PLANE_URL").ok();
    let deployment_id = std::env::var("ROKO_DEPLOYMENT_ID").ok();

    let state = Arc::new(WorkerState {
        template,
        control_plane_url,
        deployment_id,
        started_at: std::time::Instant::now(),
        last_task: tokio::sync::RwLock::new(None),
    });

    // PORT env overrides the arg (Railway injects PORT)
    let effective_port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(port);

    let app = handler::build_router(state);

    let addr = format!("0.0.0.0:{effective_port}");
    let listener = TcpListener::bind(&addr)
        .await
        .context("bind worker address")?;
    info!("roko worker listening on http://{addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            info!("worker shutting down");
        })
        .await
        .context("worker server error")?;

    Ok(())
}
