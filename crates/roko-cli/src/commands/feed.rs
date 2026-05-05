//! `roko feed` -- inspect registered runtime feeds.
//!
//! The CLI commands are read-only introspection that query a running
//! `roko serve` instance via its REST API. When the server is not
//! reachable, a clear "not running" message is printed instead of
//! panicking.

use anyhow::Result;
use clap::Subcommand;
use roko_core::feed::FeedRuntimeStatus;

use crate::*;

/// Feed management subcommands.
#[derive(Debug, Subcommand)]
pub enum FeedCmd {
    /// List all runtime feeds with their topics and status.
    List,
    /// Show detailed status for a specific feed.
    Status {
        /// Feed identifier to inspect (e.g. `file-watch-roko-dir`).
        id: String,
    },
}

pub(crate) async fn cmd_feed(cli: &Cli, cmd: FeedCmd) -> Result<i32> {
    match cmd {
        FeedCmd::List => cmd_list(cli).await,
        FeedCmd::Status { id } => cmd_status(cli, &id).await,
    }
}

/// Resolve the roko serve base URL from the environment or default.
fn serve_base_url() -> String {
    std::env::var("ROKO_SERVE_URL")
        .unwrap_or_else(|_| "http://localhost:6677".to_string())
        .trim_end_matches('/')
        .to_string()
}

/// Lightweight summary for list output (mirrors the runtime status JSON).
#[derive(Debug, serde::Deserialize)]
struct FeedSummary {
    id: String,
    topic: String,
    kind: String,
    connected: bool,
    #[serde(default)]
    pulses_produced: u64,
}

async fn cmd_list(cli: &Cli) -> Result<i32> {
    let base = serve_base_url();
    let url = format!("{base}/api/feeds/runtime");

    match reqwest::get(&url).await {
        Ok(resp) if resp.status().is_success() => {
            let feeds: Vec<FeedSummary> = resp.json().await.unwrap_or_default();

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&serde_json::json!({
                    "feeds": feeds.iter().map(|f| serde_json::json!({
                        "id": f.id,
                        "topic": f.topic,
                        "kind": f.kind,
                        "connected": f.connected,
                        "pulses_produced": f.pulses_produced,
                    })).collect::<Vec<_>>(),
                    "total": feeds.len(),
                }))?);
            } else {
                println!(
                    "{:<24} {:<32} {:<10} {}",
                    "ID", "TOPIC", "KIND", "CONNECTED"
                );
                println!("{}", "-".repeat(80));

                for f in &feeds {
                    println!(
                        "{:<24} {:<32} {:<10} {}",
                        f.id,
                        f.topic,
                        f.kind,
                        if f.connected { "yes" } else { "no" }
                    );
                }
                if feeds.is_empty() {
                    println!("(no feeds registered)");
                }
            }
        }
        Ok(resp) => {
            let status = resp.status();
            if !cli.quiet {
                eprintln!("roko serve returned HTTP {status}");
            }
            return Ok(EXIT_FAILURE);
        }
        Err(_) => {
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({"error": "roko serve is not running"})
                );
            } else {
                println!("(roko serve is not running; no live feed data available)");
                println!("Start the server with: roko serve");
            }
        }
    }

    Ok(EXIT_SUCCESS)
}

async fn cmd_status(cli: &Cli, id: &str) -> Result<i32> {
    let base = serve_base_url();
    let url = format!("{base}/api/feeds/runtime/{id}");

    match reqwest::get(&url).await {
        Ok(resp) if resp.status().is_success() => {
            let status: FeedRuntimeStatus = resp.json().await?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!("Feed: {}", status.id);
                println!("  topic:            {}", status.topic);
                println!("  kind:             {}", status.kind);
                println!("  connected:        {}", status.connected);
                println!("  rate_hz:          {:.2}", status.rate_hz);
                println!("  pulses_produced:  {}", status.pulses_produced);
                if let Some(ms) = status.last_update_ms {
                    println!("  last_update_ms:   {ms}");
                }
                if let Some(err) = &status.error {
                    println!("  error:            {err}");
                }
            }
            Ok(EXIT_SUCCESS)
        }
        Ok(resp) if resp.status().as_u16() == 404 => {
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({"error": format!("feed '{id}' not found")})
                );
            } else {
                eprintln!("feed '{id}' not found");
            }
            Ok(EXIT_FAILURE)
        }
        Ok(resp) => {
            let status = resp.status();
            eprintln!("roko serve returned HTTP {status}");
            Ok(EXIT_FAILURE)
        }
        Err(_) => {
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({"error": "roko serve is not running"})
                );
            } else {
                eprintln!("roko serve is not running");
            }
            Ok(EXIT_FAILURE)
        }
    }
}
