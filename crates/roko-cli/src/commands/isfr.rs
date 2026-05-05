//! `roko isfr` subcommand — manage the ISFR keeper.

use std::path::PathBuf;

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
    },
    /// List configured ISFR rate sources.
    Sources {
        /// Working directory (default: cwd or --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
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

            // Relay publish stub — full wiring happens in A6.
            if let Some(ref relay_url) = config.relay.url {
                println!("Relay: {relay_url} (publish stub — full wiring in task A6)");
                let publish_fn: roko_chain::isfr_keeper::PublishFn =
                    std::sync::Arc::new(|topic: &str, msg_type: &str, _payload| {
                        tracing::info!(topic, msg_type, "isfr: relay publish (stub)");
                    });
                keeper.set_publish_fn(publish_fn);
            }

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

        IsfrCmd::Status { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let config = load_roko_config(&wd);

            if !config.isfr.enabled {
                println!("ISFR: disabled");
                return Ok(EXIT_SUCCESS);
            }

            println!("ISFR: enabled");
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

            // TODO (A6): Query /api/isfr/current from serve if running.
            Ok(EXIT_SUCCESS)
        }

        IsfrCmd::Sources { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let config = load_roko_config(&wd);

            if config.isfr.sources.is_empty() {
                println!("No ISFR sources configured.");
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
            Ok(EXIT_SUCCESS)
        }
    }
}
