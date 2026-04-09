#![deny(unsafe_code)]
#![warn(missing_docs)]
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::doc_markdown,
    clippy::unreadable_literal,
    clippy::significant_drop_tightening,
    clippy::too_many_lines,
    clippy::missing_const_for_fn,
    clippy::option_if_let_else,
    clippy::manual_let_else,
    clippy::similar_names,
    clippy::struct_excessive_bools,
    clippy::trivially_copy_pass_by_ref,
    clippy::uninlined_format_args
)]

//! `roko-chain-watcher` binary entry point.
//!
//! Connects to a `mirage-rs` JSON-RPC endpoint, polls for pheromones and
//! insights, decides on reactions via [`reactions::decide`], and posts them
//! back to the chain. See the crate's README for runtime flags.

use std::sync::Arc;
use std::time::Duration;

use clap::Parser;

mod block_observer;
mod config;
mod known_addresses;
mod reactions;
mod rpc_client;
mod watcher;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ROKO_LOG").unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("info,roko_chain_watcher=debug")
            }),
        )
        .with_target(false)
        .init();

    let cli = config::WatcherCli::parse();
    tracing::info!(
        rpc_url = %cli.rpc_url,
        watcher_id = %cli.watcher_id,
        dry_run = cli.dry_run,
        "roko-chain-watcher starting"
    );

    // Probe mirage connectivity.
    let client = rpc_client::MirageRpcClient::new(cli.rpc_url.clone());
    match client.eth_block_number().await {
        Ok(n) => tracing::info!(block = n, "connected to mirage"),
        Err(e) => tracing::warn!(error=%e, "failed to probe mirage, continuing anyway"),
    }
    if let Ok(v) = client.chain_version().await {
        tracing::info!(version = %v, "chain surface version");
    } else {
        tracing::warn!("chain_version unavailable — subscriptions may not be wired");
    }

    // Launch block observer (real chain analysis).
    let observer_task = if cli.disable_block_observer {
        None
    } else {
        let eth_url = cli
            .eth_rpc_url
            .clone()
            .unwrap_or_else(|| cli.rpc_url.clone());
        let mirage_for_obs = rpc_client::MirageRpcClient::new(cli.rpc_url.clone());
        let watcher_id = cli.watcher_id.clone();
        let interval = Duration::from_millis(cli.block_poll_interval_ms);
        let backfill = cli.block_backfill;
        let fetch_full = cli.fetch_full_txs;
        let dry_run = cli.dry_run;
        tracing::info!(eth_url = %eth_url, backfill, fetch_full, "starting block observer");
        let observer = Arc::new(block_observer::BlockObserver::new(
            eth_url,
            mirage_for_obs,
            watcher_id,
            fetch_full,
            dry_run,
        ));
        Some(tokio::spawn(async move {
            if let Err(e) = observer.run(interval, backfill).await {
                tracing::warn!(error = %e, "block observer exited with error");
            }
        }))
    };

    // Launch pattern-matcher reaction loop.
    let reactions_task = if cli.disable_reactions {
        None
    } else {
        let watcher = watcher::Watcher::new(cli, client);
        Some(tokio::spawn(async move {
            if let Err(e) = watcher.run().await {
                tracing::warn!(error = %e, "reaction loop exited with error");
            }
        }))
    };

    // Wait for shutdown.
    let _ = tokio::signal::ctrl_c().await;
    tracing::warn!("SIGINT received — draining");

    if let Some(t) = observer_task {
        t.abort();
    }
    if let Some(t) = reactions_task {
        t.abort();
    }
    tracing::info!("roko-chain-watcher exited cleanly");
    Ok(())
}
