//! `roko isfr` subcommand — manage the ISFR keeper.

use std::path::PathBuf;
use std::time::Duration;

use clap::Subcommand;
use roko_core::config::schema::RokoConfig;

use crate::*;

/// ISFR keeper management.
#[derive(Debug, Subcommand)]
pub enum IsfrCmd {
    /// Start the ISFR keeper (foreground, Ctrl-C to stop).
    Start {
        /// Override poll interval in seconds.
        #[arg(long)]
        poll_interval: Option<u64>,
        /// Working directory (default: cwd or --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Show ISFR configuration and status.
    Status {
        /// Working directory (default: cwd or --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Output raw JSON (machine-readable).
        #[arg(long)]
        json: bool,
    },
    /// List configured ISFR rate sources.
    Sources {
        /// Working directory (default: cwd or --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Output raw JSON (machine-readable).
        #[arg(long)]
        json: bool,
    },
}

/// Load `RokoConfig` from the workdir's `roko.toml`, falling back to defaults.
fn load_roko_config(workdir: &std::path::Path) -> RokoConfig {
    std::fs::read_to_string(workdir.join("roko.toml"))
        .ok()
        .and_then(|s| RokoConfig::from_toml(&s).ok())
        .unwrap_or_default()
}

pub(crate) async fn cmd_isfr(cli: &Cli, cmd: IsfrCmd) -> Result<i32> {
    match cmd {
        IsfrCmd::Start {
            poll_interval,
            workdir,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let config = load_roko_config(&wd);

            if !config.isfr.enabled {
                println!("ISFR is not enabled. Set [isfr] enabled = true in roko.toml.");
                return Ok(EXIT_FAILURE);
            }

            // Build ISFRKeeperConfig from the [isfr] and [chain] sections.
            let keeper_config = roko_chain::isfr_keeper::ISFRKeeperConfig {
                poll_interval_secs: poll_interval.unwrap_or(config.isfr.poll_interval_secs),
                epoch_duration_secs: config.isfr.epoch_duration_secs,
                min_submissions: config.isfr.min_submissions,
                outlier_sigma: config.isfr.outlier_sigma,
                relay_url: config.relay.url.clone(),
                chain_id: config
                    .chain
                    .chain_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "31337".to_string()),
            };

            // Convert [[isfr.sources]] entries to SourceConfig.
            let source_configs: Vec<roko_chain::isfr_keeper::SourceConfig> = config
                .isfr
                .sources
                .iter()
                .map(|s| roko_chain::isfr_keeper::SourceConfig {
                    name: s.name.clone(),
                    kind: s.kind.clone(),
                    weight: s.weight,
                    class: s.class.clone(),
                    rate_bps: s.rate_bps,
                    jitter_bps: s.jitter_bps,
                    rpc_url: s.rpc_url.clone(),
                    pool_address: s.pool_address.clone(),
                })
                .collect();

            let keeper_id = format!("isfr-keeper-{}", &uuid::Uuid::new_v4().to_string()[..8]);

            let keeper = if source_configs.is_empty() {
                println!("No sources configured, using 4 default mock sources.");
                roko_chain::isfr_keeper::ISFRKeeper::mock_keeper(&keeper_id, keeper_config)
            } else {
                roko_chain::isfr_keeper::ISFRKeeper::from_config(
                    &keeper_id,
                    keeper_config,
                    &source_configs,
                )
            };

            // Wire publish callback — prints computed rate to stdout and logs via tracing.
            // When a relay URL is configured, note that relay transport is deferred (Phase 2).
            if let Some(ref relay_url) = config.relay.url {
                println!("Relay: {relay_url} (WebSocket relay transport is Phase 2)");
            }

            let publish_fn: roko_chain::isfr_keeper::PublishFn = std::sync::Arc::new(
                move |topic: &str, msg_type: &str, payload: serde_json::Value| {
                    if let Some(composite_bps) =
                        payload.get("composite_bps").and_then(|v| v.as_u64())
                    {
                        let pct = composite_bps as f64 / 100.0;
                        println!("ISFR rate: {pct:.2}% (composite) -> topic={topic}");
                    }
                    tracing::info!(topic, msg_type, %payload, "isfr: rate published");
                },
            );
            keeper.set_publish_fn(publish_fn);

            println!(
                "ISFR keeper '{}' starting (poll every {}s)...",
                keeper_id, config.isfr.poll_interval_secs
            );
            println!("Press Ctrl-C to stop.\n");

            let cancel = tokio_util::sync::CancellationToken::new();
            let cancel_clone = cancel.clone();
            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.ok();
                cancel_clone.cancel();
            });

            keeper.run(cancel).await;
            println!("\nISFR keeper stopped.");
            Ok(EXIT_SUCCESS)
        }

        IsfrCmd::Status { workdir, json } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let config = load_roko_config(&wd);

            // Try to query a running roko serve instance first.
            let client = match reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
            {
                Ok(c) => c,
                Err(_) => reqwest::Client::new(),
            };

            match client
                .get("http://localhost:6677/api/isfr/status")
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    let body: serde_json::Value = resp.json().await.unwrap_or_default();

                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&body).unwrap_or_default()
                        );
                        return Ok(EXIT_SUCCESS);
                    }

                    // Human-readable live status from serve.
                    let enabled = body
                        .get("enabled")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let running = body
                        .get("keeper_running")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let sources_count = body
                        .get("sources_count")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let poll_secs = body
                        .get("poll_interval_secs")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let epoch_secs = body
                        .get("epoch_duration_secs")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let rate_bps = body.get("current_rate_bps").and_then(|v| v.as_u64());
                    let confidence = body.get("current_confidence").and_then(|v| v.as_f64());

                    println!("ISFR status (live from roko serve)");
                    println!("  enabled:    {enabled}");
                    println!(
                        "  keeper:     {}",
                        if running { "running" } else { "stopped" }
                    );
                    println!("  sources:    {sources_count}");
                    println!("  poll:       {poll_secs}s");
                    println!("  epoch:      {epoch_secs}s");

                    match rate_bps {
                        Some(bps) => {
                            let pct = bps as f64 / 100.0;
                            let conf_pct = confidence.unwrap_or(0.0) * 100.0;
                            println!("  rate:       {pct:.2}% ({bps} bps)");
                            println!("  confidence: {conf_pct:.1}%");
                        }
                        None => {
                            println!("  rate:       (no rate computed yet)");
                        }
                    }
                }
                _ => {
                    // Fall back to config-only display when serve is not running.
                    if json {
                        let out = serde_json::json!({
                            "source": "config",
                            "enabled": config.isfr.enabled,
                            "chain": config.chain.profile,
                            "relay": config.relay.url,
                            "sources_count": config.isfr.sources.len(),
                            "poll_interval_secs": config.isfr.poll_interval_secs,
                            "epoch_duration_secs": config.isfr.epoch_duration_secs,
                            "sources": config.isfr.sources.iter().map(|s| serde_json::json!({
                                "name": s.name,
                                "kind": s.kind,
                                "class": s.class,
                                "weight": s.weight,
                                "rate_bps": s.rate_bps,
                            })).collect::<Vec<_>>(),
                        });
                        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
                        return Ok(EXIT_SUCCESS);
                    }

                    if !config.isfr.enabled {
                        println!("ISFR: disabled");
                        return Ok(EXIT_SUCCESS);
                    }

                    println!("ISFR status (config only — roko serve not reachable)");
                    println!("  chain:    {}", config.chain.profile);
                    println!(
                        "  relay:    {}",
                        config.relay.url.as_deref().unwrap_or("not configured")
                    );
                    println!("  sources:  {}", config.isfr.sources.len());
                    println!("  poll:     {}s", config.isfr.poll_interval_secs);
                    println!("  epoch:    {}s", config.isfr.epoch_duration_secs);

                    if !config.isfr.sources.is_empty() {
                        println!("\n  Sources:");
                        for src in &config.isfr.sources {
                            println!(
                                "    {} ({}, {}, weight={:.2})",
                                src.name, src.kind, src.class, src.weight
                            );
                        }
                    }
                }
            }

            Ok(EXIT_SUCCESS)
        }

        IsfrCmd::Sources { workdir, json } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let config = load_roko_config(&wd);

            // Try to get live source health from serve first.
            let client = match reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
            {
                Ok(c) => c,
                Err(_) => reqwest::Client::new(),
            };

            match client
                .get("http://localhost:6677/api/isfr/sources")
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    let body: serde_json::Value = resp.json().await.unwrap_or_default();

                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&body).unwrap_or_default()
                        );
                        return Ok(EXIT_SUCCESS);
                    }

                    // Human-readable live source health from serve.
                    println!("ISFR sources (live from roko serve)");
                    if let Some(sources) = body.as_array() {
                        if sources.is_empty() {
                            println!("  (no sources tracked yet)");
                        } else {
                            println!(
                                "  {:<20} {:<10} {:<8} {:<8} {}",
                                "NAME", "STATUS", "WEIGHT", "RATE_BPS", "FAILURES"
                            );
                            for src in sources {
                                let name = src.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                                let status =
                                    src.get("health").and_then(|v| v.as_str()).unwrap_or("?");
                                let weight =
                                    src.get("weight").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                // No consecutive_failures field; show 0 when healthy
                                let failures: u64 = if status == "live" { 0 } else { 1 };
                                let rate_bps = src.get("last_rate_bps").and_then(|v| v.as_u64());
                                let rate_str = rate_bps
                                    .map(|b| b.to_string())
                                    .unwrap_or_else(|| "-".to_string());
                                println!(
                                    "  {:<20} {:<10} {:<8.2} {:<8} {}",
                                    name, status, weight, rate_str, failures
                                );
                            }
                        }
                    }
                }
                _ => {
                    // Fall back to config-only display.
                    if config.isfr.sources.is_empty() {
                        if json {
                            println!("[]");
                        } else {
                            println!("No ISFR sources configured.");
                        }
                        return Ok(EXIT_SUCCESS);
                    }

                    if json {
                        let out: Vec<_> = config
                            .isfr
                            .sources
                            .iter()
                            .map(|s| {
                                serde_json::json!({
                                    "name": s.name,
                                    "kind": s.kind,
                                    "class": s.class,
                                    "weight": s.weight,
                                    "rate_bps": s.rate_bps,
                                    "jitter_bps": s.jitter_bps,
                                })
                            })
                            .collect();
                        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
                        return Ok(EXIT_SUCCESS);
                    }

                    println!(
                        "{:<20} {:<10} {:<12} {:>6} {:>8}",
                        "NAME", "KIND", "CLASS", "WEIGHT", "RATE_BPS"
                    );
                    for src in &config.isfr.sources {
                        println!(
                            "{:<20} {:<10} {:<12} {:>6.2} {:>8}",
                            src.name, src.kind, src.class, src.weight, src.rate_bps
                        );
                    }
                }
            }

            Ok(EXIT_SUCCESS)
        }
    }
}
