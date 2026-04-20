//! `roko trading ...` and `roko jobs ...` subcommands.
//!
//! Ports the CLI surface of `offchainservices-agent/cli/commands/` +
//! `cli/jobs/commands.py`. Maps every Python `hl ...` subcommand to a
//! `roko trading ...` subcommand.
//!
//! See `plans/P08-trading-surface.md` T12.

use anyhow::Result;
use clap::Subcommand;
use roko_apex::{apex_presets, ApexConfig, ApexEngine};
use roko_jobs::{list_jobs, list_jobs_by_category, JobCategory};
use roko_strategy::registry::StrategyRegistry;
use roko_venue::{Instrument, OrderSide, TimeInForce, VenueAdapter};

#[derive(Debug, Subcommand)]
pub enum TradingCmd {
    /// Place a single manual order on a venue.
    Trade {
        #[arg(long)]
        venue: String,
        #[arg(long)]
        instrument: String,
        #[arg(long)]
        side: String,
        #[arg(long)]
        size: f64,
        #[arg(long)]
        price: f64,
        #[arg(long, default_value = "ioc")]
        tif: String,
        #[arg(long)]
        account_id: Option<u64>,
        #[arg(long)]
        network: Option<String>,
        #[arg(long)]
        rpc_url: Option<String>,
    },
    /// Run an autonomous strategy against a venue.
    Run {
        #[arg(long)]
        strategy: String,
        #[arg(long, default_value = "hl")]
        venue: String,
        #[arg(long, default_value_t = 10.0)]
        tick_interval_s: f64,
        #[arg(long, default_value_t = 0)]
        max_ticks: u32,
        #[arg(long)]
        instrument: Option<String>,
        #[arg(long)]
        mock: bool,
    },
    /// APEX multi-slot orchestrator.
    Apex {
        #[command(subcommand)]
        cmd: ApexCmd,
    },
    /// Nightly REFLECT performance review.
    Reflect,
    /// Opportunity radar (scan + score).
    Radar,
    /// Pulse momentum detector.
    Pulse,
    /// Guard dynamic-stop-loss manager.
    Guard,
    /// Key management (add/list/rm).
    Keys {
        #[command(subcommand)]
        cmd: KeysCmd,
    },
    /// Wallet info + balances.
    Wallet,
    /// Account info (positions, equity, tradingAgent).
    Account,
    /// Builder-fee config (HL BuilderInfo).
    Builder,
    /// Engine + APEX status snapshot.
    Status,
    /// Trade journal viewer.
    Journal,
    /// Agent Skills — list/run.
    Skills {
        #[command(subcommand)]
        cmd: SkillsCmd,
    },
    /// Strategy registry — list/info.
    Strategies {
        #[command(subcommand)]
        cmd: StrategiesCmd,
    },
    /// Initial setup wizard.
    Setup,
    /// Telegram bot helpers.
    Telegram,
}

#[derive(Debug, Subcommand)]
pub enum ApexCmd {
    /// Start APEX multi-slot loop.
    Run {
        #[arg(long)]
        preset: Option<String>,
        #[arg(long, default_value = "hl")]
        venue: String,
        #[arg(long)]
        mock: bool,
    },
    /// Run a single APEX tick and exit.
    Once {
        #[arg(long)]
        preset: Option<String>,
    },
    /// List available presets.
    Presets,
    /// Current APEX state.
    Status,
}

#[derive(Debug, Subcommand)]
pub enum KeysCmd {
    Add {
        address: String,
    },
    List,
    Rm {
        address: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum SkillsCmd {
    List,
    Run {
        name: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum StrategiesCmd {
    List,
    Info {
        id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum JobsCmd {
    /// Show all registered job types.
    List,
    /// Show details for a single job.
    Info {
        job_id: String,
    },
    /// Register a local agent for a job.
    Register {
        #[arg(long)]
        config: String,
    },
    /// Run a registered job.
    Run {
        #[arg(long)]
        job_id: String,
        #[arg(long)]
        agent_id: String,
    },
    /// Print status snapshot from disk.
    Status {
        #[arg(long)]
        job_id: String,
        #[arg(long)]
        agent_id: String,
        #[arg(long, default_value = "data/jobs")]
        data_dir: String,
    },
    /// Stop a running job (best-effort via local signal file).
    Stop {
        #[arg(long)]
        job_id: String,
    },
}

pub async fn dispatch_trading(cmd: TradingCmd) -> Result<i32> {
    match cmd {
        TradingCmd::Trade {
            venue,
            instrument,
            side,
            size,
            price,
            tif,
            account_id: _account_id,
            network: _network,
            rpc_url: _rpc_url,
        } => {
            let mut adapter: Box<dyn VenueAdapter> = match venue.to_lowercase().as_str() {
                "mock" => Box::new(roko_venue::mock::MockVenue::with_mid(price)),
                other => anyhow::bail!(
                    "venue {other:?} not wired into the CLI yet (use `mock`). \
                     HL + Nunchi arrive once roko-chain's alloy submitter lands. \
                     Track: plans/P08-trading-surface.md T12 close-out."
                ),
            };
            adapter.connect("0x0", true).await.ok();
            let instrument = Instrument::new(instrument);
            let order_side = match side.to_lowercase().as_str() {
                "buy" | "long" => OrderSide::Buy,
                "sell" | "short" => OrderSide::Sell,
                other => anyhow::bail!("unknown side {other:?}"),
            };
            let order_tif = match tif.to_lowercase().as_str() {
                "gtc" => TimeInForce::Gtc,
                "ioc" => TimeInForce::Ioc,
                "alo" => TimeInForce::Alo,
                other => anyhow::bail!("unknown tif {other:?}"),
            };
            let fill = adapter
                .place_order(&instrument, order_side, size, price, order_tif, None)
                .await;
            match fill {
                Ok(Some(f)) => println!(
                    "Filled: {} {} {} @ {} (oid={})",
                    f.side.as_str().to_uppercase(),
                    f.quantity,
                    f.instrument,
                    f.price,
                    f.oid
                ),
                Ok(None) => println!("No fill"),
                Err(e) => println!("Error: {e}"),
            }
            Ok(0)
        }
        TradingCmd::Run { strategy, venue, tick_interval_s, max_ticks, instrument, mock } => {
            let _ = (tick_interval_s, max_ticks);
            let instrument = instrument.unwrap_or_else(|| "ETH-PERP".into());
            if !mock && venue.to_lowercase() != "mock" {
                println!(
                    "NOTE: live venues not yet wired in roko-cli — running against the MockVenue. \
                     HL + Nunchi arrive in plans/P08-trading-surface.md T12 close-out."
                );
            }
            let registry = StrategyRegistry::default();
            let _strategy = registry.load(&strategy, &Default::default())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("Would run strategy={strategy} on {instrument}@{venue} — full integration binds in T12 close-out.");
            Ok(0)
        }
        TradingCmd::Apex { cmd } => dispatch_apex(cmd).await,
        TradingCmd::Reflect | TradingCmd::Radar | TradingCmd::Pulse | TradingCmd::Guard
        | TradingCmd::Wallet | TradingCmd::Account | TradingCmd::Builder | TradingCmd::Status
        | TradingCmd::Journal | TradingCmd::Setup | TradingCmd::Telegram => {
            println!(
                "This subcommand is registered but not yet implemented. Tracked in plans/P08-trading-surface.md T12 close-out."
            );
            Ok(0)
        }
        TradingCmd::Keys { cmd } => {
            println!("roko trading keys — placeholder (wires via roko-chain wallet abstraction in T12 close-out): {:?}", cmd);
            Ok(0)
        }
        TradingCmd::Skills { cmd } => {
            println!("roko trading skills — placeholder: {:?}", cmd);
            Ok(0)
        }
        TradingCmd::Strategies { cmd } => {
            let registry = StrategyRegistry::default();
            match cmd {
                StrategiesCmd::List => {
                    for id in registry.ids() {
                        println!("{id}");
                    }
                }
                StrategiesCmd::Info { id } => match registry.load(&id, &Default::default()) {
                    Ok(s) => println!("{}", s.strategy_id()),
                    Err(e) => println!("Error: {e}"),
                },
            }
            Ok(0)
        }
    }
}

async fn dispatch_apex(cmd: ApexCmd) -> Result<i32> {
    match cmd {
        ApexCmd::Presets => {
            for (name, _preset) in apex_presets() {
                println!("{name}");
            }
            Ok(0)
        }
        ApexCmd::Once { preset } => {
            let config = preset
                .and_then(|p| apex_presets().get(&p).cloned())
                .map(|p| p.config)
                .unwrap_or_else(ApexConfig::default);
            let mut engine = ApexEngine::new(config);
            engine.tick();
            println!("tick={}  active_slots={}  daily_pnl={:+.2}",
                     engine.state.tick_count,
                     engine.state.active_slots().len(),
                     engine.state.daily_pnl);
            Ok(0)
        }
        ApexCmd::Run { .. } | ApexCmd::Status => {
            println!("APEX {:?} — placeholder; full loop lands in T8 close-out.", cmd);
            Ok(0)
        }
    }
}

pub async fn dispatch_jobs(cmd: JobsCmd) -> Result<i32> {
    match cmd {
        JobsCmd::List => {
            println!("{:<24} {:<26} {:<14} {:<14} {:<6} {:<10}",
                     "ID", "Name", "Category", "Trigger", "TEE", "Min Stake");
            println!("{}", "-".repeat(100));
            for job in list_jobs() {
                println!(
                    "{:<24} {:<26} {:<14} {:<14} {:<6} {:<10}",
                    job.job_id,
                    job.name,
                    job.category.as_str(),
                    format!("{:?}", job.trigger),
                    if job.requires_tee { "yes" } else { "no" },
                    job.min_stake_display()
                );
            }
            Ok(0)
        }
        JobsCmd::Info { job_id } => match roko_jobs::get_job(&job_id) {
            Ok(job) => {
                println!("Job: {}", job.job_id);
                println!("  Name: {}", job.name);
                println!("  Description: {}", job.description);
                println!("  Category: {}", job.category.as_str());
                println!("  Trigger: {:?}", job.trigger);
                println!("  Required role: {:?}", job.required_role);
                println!("  Min stake: {}", job.min_stake_display());
                println!("  TEE: {}", job.requires_tee);
                println!("  Custody destinations: {:?}", job.custody.destinations);
                println!("  Custody selectors: {:?}", job.custody.selectors);
                println!("  Rate limit/block: {}", job.custody.rate_limit_per_block);
                Ok(0)
            }
            Err(e) => {
                println!("{e}");
                Ok(1)
            }
        },
        JobsCmd::Register { config } => {
            println!("Register from {config}: placeholder — validate+write {config} to local job-registry in T12 close-out.");
            Ok(0)
        }
        JobsCmd::Run { job_id, agent_id } => {
            println!("Run job={job_id} agent={agent_id}: placeholder — wires KeeperEngine once roko-chain alloy submitter lands.");
            Ok(0)
        }
        JobsCmd::Status { job_id, agent_id, data_dir } => {
            if let Some(t) = roko_jobs::JobStatusTracker::load(&job_id, &agent_id, &data_dir) {
                println!("job_id:       {}", t.job_id);
                println!("agent_id:     {}", t.agent_id);
                println!("started_at:   {}", t.started_at);
                println!("heartbeats:   {}", t.heartbeats.len());
                println!("rewards:      {}  (total ETH: {:.6})", t.rewards.len(), t.total_reward_eth());
            } else {
                println!("No status found for {job_id}/{agent_id} in {data_dir}");
            }
            Ok(0)
        }
        JobsCmd::Stop { job_id } => {
            println!("Stop {job_id}: placeholder — SIGTERM to PID file in T12 close-out.");
            Ok(0)
        }
    }
}

/// `roko jobs list` filtered by category — exposed for tests/docs.
pub fn keeper_jobs() -> Vec<&'static roko_jobs::JobDefinition> {
    list_jobs_by_category(JobCategory::Keeper)
}
