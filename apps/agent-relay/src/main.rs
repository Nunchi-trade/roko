#![allow(missing_docs)]

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

use agent_relay::{app, state::RelayState};

#[derive(Debug, Parser)]
#[command(name = "agent-relay")]
#[command(about = "Standalone websocket relay for agent presence and forwarding.")]
struct Cli {
    /// Address to bind, for example 127.0.0.1:9011.
    #[arg(long, env = "ROKO_AGENT_RELAY_BIND", default_value = "127.0.0.1:9011")]
    bind: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("agent_relay=info,tower_http=info")),
        )
        .init();

    let cli = Cli::parse();
    let listener = TcpListener::bind(&cli.bind)
        .await
        .with_context(|| format!("bind agent relay to {}", cli.bind))?;
    let addr = listener.local_addr().context("read bound relay address")?;
    info!(%addr, "agent relay listening");

    let state = Arc::new(RelayState::new());

    // Expire stale workspaces every 30 seconds (stale = no heartbeat in 60s).
    let expiry_state = Arc::clone(&state);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            let expired = expiry_state.expire_stale_workspaces(60_000);
            for id in &expired {
                tracing::debug!(workspace_id = %id, "expired stale workspace");
            }
        }
    });

    axum::serve(listener, app(state))
        .await
        .context("serve agent relay router")
}
