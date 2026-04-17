#![deny(unsafe_code)]
#![warn(missing_docs)]
#![allow(
    clippy::doc_markdown,
    clippy::ignored_unit_patterns,
    clippy::needless_borrows_for_generic_args,
    clippy::ptr_arg
)]

//! The `roko-demo` binary.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use alloy::primitives::{U256, keccak256};
use clap::{Parser, Subcommand};
use roko_demo::autonomous::{prepare_autonomous, run_autonomous};
use roko_demo::benchmark::{prepare_benchmark, run_benchmark};
use roko_demo::bindings::{AgentRegistry, MockERC20, WorkerRegistry};
use roko_demo::chain_ctx::ChainCtx;
use roko_demo::deploy::deploy_suite;
use roko_demo::events::create_emitter;
use roko_demo::fixtures::{FixtureRegistry, run_fixtures};
use roko_demo::manifest::{LoadedManifest, write_deployments};
use roko_demo::scenarios::{self, ScenarioRuntime, create_provider};
use roko_demo::tournament::{prepare_tournament, run_tournament};
use roko_demo::tui;
use roko_demo::verify;

#[derive(Parser)]
#[command(name = "roko-demo", about = "Roko demo-environment orchestrator")]
struct Cli {
    /// Path to the demo dir (containing manifest.toml).
    #[arg(long, default_value = "demo")]
    demo_dir: PathBuf,

    /// Runtime artifacts dir (deployments.json lives here).
    #[arg(long, default_value = "demo/.runtime")]
    runtime_dir: PathBuf,

    /// Override JSON-RPC URL (else uses ROKO_MIRAGE_URL env / manifest default).
    #[arg(long)]
    rpc_url: Option<String>,

    /// LLM backend to use for scenario slots.
    #[arg(long, default_value = "stub")]
    llm_backend: String,

    /// Demo event output mode.
    #[arg(long, default_value = "none")]
    events: String,

    /// WebSocket port for event streaming when `--events ws|both`.
    #[arg(long, default_value_t = 9090)]
    ws_port: u16,

    /// Persist worker reputation snapshots to the runtime directory.
    #[arg(long)]
    persist_reputation: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run end-to-end: deploy + fixtures + scripted spine.
    #[command(visible_alias = "run")]
    Up {
        /// Scenario name.
        scenario: String,
        /// Skip the agent-spawn step.
        #[arg(long)]
        no_agents: bool,
    },
    /// Deploy only (writes deployments.json).
    Deploy {
        /// Scenario name.
        scenario: String,
    },
    /// Run fixtures only (requires prior deploy).
    Seed {
        /// Scenario name.
        scenario: String,
    },
    /// Assert post-run invariants.
    Verify {
        /// Scenario name.
        scenario: String,
    },
    /// Run benchmark suites.
    Benchmark {
        #[command(subcommand)]
        cmd: BenchmarkCmd,
    },
    /// Run a multi-round tournament.
    Tournament {
        #[arg(long, default_value_t = 5)]
        rounds: usize,
        #[arg(default_value = "yield-routing")]
        scenario: String,
    },
    /// Run the autonomous poster/agent loop.
    Autonomous {
        #[arg(long, default_value_t = 5)]
        agents: usize,
        #[arg(long, default_value_t = 3)]
        jobs: usize,
        #[arg(long, default_value_t = 10)]
        interval: u64,
        #[arg(long, default_value_t = 300)]
        timeout: u64,
        #[arg(default_value = "yield-routing")]
        scenario: String,
    },
    /// Launch a terminal UI for a live demo run.
    Tui {
        #[arg(long, default_value = "yield-routing")]
        scenario: String,
    },
    /// Register one agent against an existing deployment.
    RegisterAgent {
        #[arg(long, default_value = "yield-routing")]
        scenario: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        wallet: String,
        #[arg(long, default_value_t = 1_000)]
        stake: u128,
    },
    /// List scenarios in the manifest.
    List,
}

#[derive(Subcommand)]
enum BenchmarkCmd {
    /// Measure the C-factor from a cold and warm run.
    CFactor {
        #[arg(default_value = "yield-routing")]
        scenario: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[allow(clippy::too_many_lines)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,roko_demo=debug")),
        )
        .init();

    let cli = Cli::parse();
    let loaded = LoadedManifest::load(&cli.demo_dir)?;

    match &cli.cmd {
        Cmd::List => {
            for s in &loaded.manifest.scenarios {
                println!("{:20}  {}", s.name, s.description.as_deref().unwrap_or(""));
            }
        }
        Cmd::Deploy { scenario } => deploy_cmd(&cli, &loaded, scenario).await?,
        Cmd::Seed { scenario } => seed_cmd(&cli, &loaded, scenario).await?,
        Cmd::Verify { scenario } => verify_cmd(&cli, &loaded, scenario).await?,
        Cmd::Up {
            scenario,
            no_agents,
        } => {
            deploy_cmd(&cli, &loaded, scenario).await?;
            seed_cmd(&cli, &loaded, scenario).await?;
            if !no_agents {
                agents_cmd(&cli, &loaded, scenario).await?;
            }
        }
        Cmd::Benchmark { cmd } => match cmd {
            BenchmarkCmd::CFactor { scenario, output } => {
                let ctx = fresh_chain_ctx(&cli, &loaded, scenario).await?;
                let llm = create_provider(&cli.llm_backend)?;
                let events = create_emitter(&cli.events, cli.ws_port).await?;
                let prepared = prepare_benchmark(
                    ctx,
                    cli.runtime_dir.clone(),
                    cli.persist_reputation,
                    events.clone(),
                )
                .await?;
                let report = run_benchmark(&prepared, llm, events).await?;
                write_json_output(output.as_ref(), &report)?;
                eprintln!(
                    "c-factor: {:.2}% warm-over-cold improvement",
                    report.c_factor.output_improvement_pct
                );
            }
        },
        Cmd::Tournament { rounds, scenario } => {
            let ctx = fresh_chain_ctx(&cli, &loaded, scenario).await?;
            let llm = create_provider(&cli.llm_backend)?;
            let events = create_emitter(&cli.events, cli.ws_port).await?;
            let prepared = prepare_tournament(
                ctx,
                cli.runtime_dir.clone(),
                cli.persist_reputation,
                events.clone(),
            )
            .await?;
            let report = run_tournament(&prepared, llm, *rounds, events).await?;
            write_json_output(None, &report)?;
        }
        Cmd::Autonomous {
            agents,
            jobs,
            interval,
            timeout,
            scenario,
        } => {
            let ctx = fresh_chain_ctx(&cli, &loaded, scenario).await?;
            let llm = create_provider(&cli.llm_backend)?;
            let events = create_emitter(&cli.events, cli.ws_port).await?;
            let prepared = prepare_autonomous(
                ctx,
                cli.runtime_dir.clone(),
                cli.persist_reputation,
                events.clone(),
            )
            .await?;
            let report =
                run_autonomous(&prepared, llm, *agents, *jobs, *interval, *timeout, events).await?;
            write_json_output(None, &report)?;
        }
        Cmd::Tui { scenario } => {
            let ctx = fresh_chain_ctx(&cli, &loaded, scenario).await?;
            let scenario_manifest = loaded.load_scenario(scenario)?;
            let llm = create_provider(&cli.llm_backend)?;
            let runtime_dir = cli.runtime_dir.clone();
            let persist = cli.persist_reputation;
            let scenario_impl = scenarios::find(scenario)
                .ok_or_else(|| anyhow::anyhow!("no Rust impl for scenario {scenario}"))?;
            tui::run_tui("Roko Yield Routing Demo", move |emitter| {
                let ctx = ctx.clone();
                let llm = llm.clone();
                let runtime_dir = runtime_dir.clone();
                let scenario_manifest = scenario_manifest.clone();
                async move {
                    let runtime = Arc::new(ScenarioRuntime {
                        llm,
                        events: emitter,
                        runtime_dir,
                        persist_reputation: persist,
                    });
                    scenario_impl.spine(ctx, &scenario_manifest, runtime).await
                }
            })
            .await?;
        }
        Cmd::RegisterAgent {
            scenario,
            name,
            model,
            wallet,
            stake,
        } => {
            register_agent_cmd(&cli, &loaded, scenario, name, model, wallet, *stake).await?;
        }
    }
    Ok(())
}

async fn deploy_cmd(cli: &Cli, loaded: &LoadedManifest, scenario_name: &str) -> anyhow::Result<()> {
    let scenario = loaded.load_scenario(scenario_name)?;
    let ctx = loaded.build_deploy_ctx(cli.rpc_url.clone())?;
    tracing::info!(
        rpc = %ctx.rpc_url,
        chain_id = ctx.chain_id,
        contracts_dir = %ctx.contracts_dir.display(),
        "deploying {} contracts",
        scenario.deploy.contracts.len()
    );
    let suite = deploy_suite(&ctx, &scenario.deploy).await?;
    let path = write_deployments(
        &cli.runtime_dir,
        scenario_name,
        &suite.addresses,
        ctx.chain_id,
        suite.last_block,
    )?;
    tracing::info!(deployments = %path.display(), "deploy complete");
    for (name, addr) in &suite.addresses {
        println!("{name:20} {addr}");
    }
    Ok(())
}

async fn seed_cmd(cli: &Cli, loaded: &LoadedManifest, scenario_name: &str) -> anyhow::Result<()> {
    let scenario = loaded.load_scenario(scenario_name)?;
    let deploy_ctx = loaded.build_deploy_ctx(cli.rpc_url.clone())?;
    let deployments = load_deployments(&cli.runtime_dir, scenario_name)?;
    let ctx = ChainCtx {
        rpc_url: deploy_ctx.rpc_url.clone(),
        chain_id: deploy_ctx.chain_id,
        wallets: deploy_ctx.wallets.clone(),
        addresses: deployments.contracts,
        deployed_at_block: deployments.deployed_at_block,
    };
    let mut registry = FixtureRegistry::new();
    if let Some(impls) = scenarios::find(scenario_name) {
        impls.register_fixtures(&mut registry);
    }
    run_fixtures(
        &ctx,
        &registry,
        &scenario.fixtures,
        &deploy_ctx.contracts_dir,
    )
    .await?;
    tracing::info!("fixtures complete: {} step(s)", scenario.fixtures.len());
    Ok(())
}

async fn agents_cmd(cli: &Cli, loaded: &LoadedManifest, scenario_name: &str) -> anyhow::Result<()> {
    let scenario_manifest = loaded.load_scenario(scenario_name)?;
    let ctx = load_chain_ctx(cli, loaded, scenario_name)?;
    let scenario = scenarios::find(scenario_name)
        .ok_or_else(|| anyhow::anyhow!("no Rust impl for scenario {scenario_name}"))?;
    let llm = create_provider(&cli.llm_backend)?;
    let events = create_emitter(&cli.events, cli.ws_port).await?;
    let runtime = Arc::new(ScenarioRuntime {
        llm: llm.clone(),
        events,
        runtime_dir: cli.runtime_dir.clone(),
        persist_reputation: cli.persist_reputation,
    });
    let timeout = std::time::Duration::from_secs(scenario_manifest.success.max_duration_secs);
    tracing::info!(
        scenario = scenario_name,
        llm_backend = llm.label(),
        events = %cli.events,
        timeout_s = timeout.as_secs(),
        "running scripted spine"
    );
    tokio::select! {
        result = scenario.spine(ctx, &scenario_manifest, runtime) => result?,
        _ = tokio::time::sleep(timeout) => {
            return Err(anyhow::anyhow!(
                "scenario {scenario_name} timed out after {}s",
                timeout.as_secs()
            ));
        }
    }
    tracing::info!("spine complete");
    Ok(())
}

async fn verify_cmd(cli: &Cli, loaded: &LoadedManifest, scenario_name: &str) -> anyhow::Result<()> {
    let scenario = loaded.load_scenario(scenario_name)?;
    let deploy_ctx = loaded.build_deploy_ctx(cli.rpc_url.clone())?;
    let deployments = load_deployments(&cli.runtime_dir, scenario_name)?;
    let ctx = ChainCtx {
        rpc_url: deploy_ctx.rpc_url.clone(),
        chain_id: deploy_ctx.chain_id,
        wallets: deploy_ctx.wallets.clone(),
        addresses: deployments.contracts,
        deployed_at_block: deployments.deployed_at_block,
    };
    let report = verify::verify(&ctx, &scenario, &deploy_ctx.contracts_dir).await?;
    for finding in &report.findings {
        println!("{finding}");
    }
    if report.ok {
        println!("\nverify: OK");
        Ok(())
    } else {
        Err(anyhow::anyhow!("verify: one or more invariants failed"))
    }
}

async fn register_agent_cmd(
    cli: &Cli,
    loaded: &LoadedManifest,
    scenario_name: &str,
    name: &str,
    model: &str,
    wallet: &str,
    stake_tokens: u128,
) -> anyhow::Result<()> {
    if !deployments_path(&cli.runtime_dir, scenario_name).exists() {
        deploy_cmd(cli, loaded, scenario_name).await?;
        seed_cmd(cli, loaded, scenario_name).await?;
    }
    let ctx = load_chain_ctx(cli, loaded, scenario_name)?;
    let started = Instant::now();
    let stake = U256::from(stake_tokens * 10u128.pow(18));
    let token_addr = ctx.address_of("MockERC20")?;
    let registry_addr = ctx.address_of("WorkerRegistry")?;
    let agent_registry_addr = ctx.address_of("AgentRegistry")?;
    let worker_addr = ctx.wallet_address(wallet)?;

    MockERC20::new(token_addr, ctx.wallet_provider("deployer")?)
        .mint(worker_addr, stake)
        .send()
        .await?
        .watch()
        .await?;
    MockERC20::new(token_addr, ctx.wallet_provider(wallet)?)
        .approve(registry_addr, stake)
        .send()
        .await?
        .watch()
        .await?;
    WorkerRegistry::new(registry_addr, ctx.wallet_provider(wallet)?)
        .register(stake)
        .send()
        .await?
        .watch()
        .await?;
    let passport = keccak256(format!("{name}:{model}:{wallet}").as_bytes());
    AgentRegistry::new(agent_registry_addr, ctx.wallet_provider(wallet)?)
        .register(format!("yield-routing,{model}"), passport)
        .send()
        .await?
        .watch()
        .await?;

    let registry = WorkerRegistry::new(registry_addr, ctx.read_provider()?);
    let reputation = registry.reputationOf(worker_addr).call().await?;
    let tier = registry.tier(worker_addr).call().await?;

    println!("Agent registered!");
    println!("Name: {name}");
    println!("Model: {model}");
    println!("Wallet: {wallet}");
    println!("Stake: {stake_tokens} DAEJI");
    println!("Tier: {}", tier_label(tier));
    println!("Time: {:.2}s", started.elapsed().as_secs_f64());
    println!("Ready to bid on jobs.");
    println!("Reputation: {reputation}");
    Ok(())
}

async fn fresh_chain_ctx(
    cli: &Cli,
    loaded: &LoadedManifest,
    scenario_name: &str,
) -> anyhow::Result<Arc<ChainCtx>> {
    deploy_cmd(cli, loaded, scenario_name).await?;
    seed_cmd(cli, loaded, scenario_name).await?;
    load_chain_ctx(cli, loaded, scenario_name)
}

fn load_chain_ctx(
    cli: &Cli,
    loaded: &LoadedManifest,
    scenario_name: &str,
) -> anyhow::Result<Arc<ChainCtx>> {
    let deploy_ctx = loaded.build_deploy_ctx(cli.rpc_url.clone())?;
    let deployments = load_deployments(&cli.runtime_dir, scenario_name)?;
    Ok(Arc::new(ChainCtx {
        rpc_url: deploy_ctx.rpc_url.clone(),
        chain_id: deploy_ctx.chain_id,
        wallets: deploy_ctx.wallets,
        addresses: deployments.contracts,
        deployed_at_block: deployments.deployed_at_block,
    }))
}

fn write_json_output<T: serde::Serialize>(path: Option<&PathBuf>, value: &T) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    println!("{json}");
    if let Some(path) = path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, json)?;
    }
    Ok(())
}

fn load_deployments(runtime_dir: &PathBuf, scenario: &str) -> anyhow::Result<verify::Deployments> {
    let path = deployments_path(runtime_dir, scenario);
    verify::Deployments::load(&path)
}

fn deployments_path(runtime_dir: &PathBuf, scenario: &str) -> PathBuf {
    runtime_dir.join(scenario).join("deployments.json")
}

const fn tier_label(value: u8) -> &'static str {
    match value {
        1 => "Probation",
        2 => "Standard",
        3 => "Trusted",
        4 => "Elite",
        _ => "Unregistered",
    }
}
