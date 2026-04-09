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

use clap::{Parser, Subcommand};
use roko_demo::chain_ctx::ChainCtx;
use roko_demo::deploy::deploy_suite;
use roko_demo::fixtures::{FixtureRegistry, run_fixtures};
use roko_demo::manifest::{LoadedManifest, write_deployments};
use roko_demo::scenarios::{self, StubLlm};
use roko_demo::verify;

#[derive(Parser)]
#[command(name = "roko-demo", about = "Roko demo-environment orchestrator")]
struct Cli {
    /// Path to the demo dir (containing manifest.toml).
    #[arg(long, default_value = "roko/demo")]
    demo_dir: PathBuf,

    /// Runtime artifacts dir (deployments.json lives here).
    #[arg(long, default_value = "roko/demo/.runtime")]
    runtime_dir: PathBuf,

    /// Override JSON-RPC URL (else uses ROKO_MIRAGE_URL env / manifest default).
    #[arg(long)]
    rpc_url: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run end-to-end: deploy + fixtures (+ agents later).
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
    /// List scenarios in the manifest.
    List,
}

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
        Cmd::Deploy { scenario } => {
            deploy_cmd(&cli, &loaded, scenario).await?;
        }
        Cmd::Seed { scenario } => {
            seed_cmd(&cli, &loaded, scenario).await?;
        }
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
        Cmd::Verify { scenario } => {
            verify_cmd(&cli, &loaded, scenario).await?;
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
    let registry = FixtureRegistry::new();
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
    let deploy_ctx = loaded.build_deploy_ctx(cli.rpc_url.clone())?;
    let deployments = load_deployments(&cli.runtime_dir, scenario_name)?;
    let ctx = Arc::new(ChainCtx {
        rpc_url: deploy_ctx.rpc_url.clone(),
        chain_id: deploy_ctx.chain_id,
        wallets: deploy_ctx.wallets.clone(),
        addresses: deployments.contracts,
        deployed_at_block: deployments.deployed_at_block,
    });
    let scenario = scenarios::find(scenario_name)
        .ok_or_else(|| anyhow::anyhow!("no Rust impl for scenario {scenario_name}"))?;
    let llm = Arc::new(StubLlm::new());
    // Honour the declared timeout so CI doesn't hang on a stuck spine.
    let timeout = std::time::Duration::from_secs(scenario_manifest.success.max_duration_secs);
    tracing::info!(
        scenario = scenario_name,
        timeout_s = timeout.as_secs(),
        "running scripted spine"
    );
    tokio::select! {
        result = scenario.spine(ctx, &scenario_manifest, llm) => result?,
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
    for f in &report.findings {
        println!("{f}");
    }
    if report.ok {
        println!("\nverify: OK");
        Ok(())
    } else {
        Err(anyhow::anyhow!("verify: one or more invariants failed"))
    }
}

fn load_deployments(runtime_dir: &PathBuf, scenario: &str) -> anyhow::Result<verify::Deployments> {
    let path = runtime_dir.join(scenario).join("deployments.json");
    verify::Deployments::load(&path)
}
