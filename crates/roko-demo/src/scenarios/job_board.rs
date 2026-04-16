//! Job-board clade scenario.
//!
//! **Spine**:
//! - Deployer funds 1 poster + 5 workers with DAEJI, each worker registers in WorkerRegistry.
//! - Poster posts a job (bounty + deadline + min tier), LLM fills bounty_amount + job_spec.
//! - First-awake worker is assigned the job.
//! - Assigned worker submits a result (LLM fills submission_content).
//! - Deployer (acting as resolver) accepts, triggering payout + reputation update.
//! - Loop until we see N `JobResolved` events, then exit.

use std::sync::Arc;

use alloy::primitives::{Address, U256, keccak256};
use async_trait::async_trait;

use crate::bindings::{BountyMarket, MockERC20, WorkerRegistry};
use crate::chain_ctx::ChainCtx;
use crate::fixtures::FixtureRegistry;
use crate::manifest::Scenario as ScenarioManifest;
use crate::scenarios::llm::LlmRequest;
use crate::scenarios::{LlmProvider, Scenario, ScenarioRuntime};

/// Job-board scenario implementation.
pub struct JobBoard;

const JOBS_TO_COMPLETE: u32 = 3;
const WORKER_COUNT: usize = 5;
const STAKE: u128 = 1_000 * 10u128.pow(18);
const WORKER_MINT: u128 = 10_000 * 10u128.pow(18);
const POSTER_MINT: u128 = 1_000_000 * 10u128.pow(18);

#[async_trait]
impl Scenario for JobBoard {
    fn name(&self) -> &'static str {
        "job-board"
    }

    fn register_fixtures(&self, _registry: &mut FixtureRegistry) {}

    async fn spine(
        &self,
        ctx: Arc<ChainCtx>,
        _manifest: &ScenarioManifest,
        runtime: Arc<ScenarioRuntime>,
    ) -> anyhow::Result<()> {
        tracing::info!("job-board: preparing wallets + registrations");
        prepare_participants(&ctx).await?;

        tracing::info!("job-board: spine running, target={} jobs", JOBS_TO_COMPLETE);
        for round in 0..JOBS_TO_COMPLETE {
            run_single_job(&ctx, runtime.llm.clone(), round).await?;
            tracing::info!(round, "round complete");
        }
        tracing::info!("job-board: all {} rounds complete", JOBS_TO_COMPLETE);
        Ok(())
    }
}

async fn prepare_participants(ctx: &ChainCtx) -> anyhow::Result<()> {
    let deployer_provider = ctx.wallet_provider("deployer")?;
    let token_addr = ctx.address_of("MockERC20")?;
    let registry_addr = ctx.address_of("WorkerRegistry")?;
    let market_addr = ctx.address_of("BountyMarket")?;

    // Resolver = deployer for job-board (no consortium in this scenario).
    let market = BountyMarket::new(market_addr, deployer_provider.clone());
    // setResolver(deployer) is already the constructor default — nothing to do.
    let _ = market; // silence if unused later

    // Mint DAEJI to poster + workers.
    let token_via_deployer = MockERC20::new(token_addr, deployer_provider.clone());
    let poster_addr = ctx.wallet_address("poster0")?;
    token_via_deployer
        .mint(poster_addr, U256::from(POSTER_MINT))
        .send()
        .await?
        .watch()
        .await?;
    for i in 0..WORKER_COUNT {
        let w = ctx.wallet_address(&format!("worker{i}"))?;
        token_via_deployer
            .mint(w, U256::from(WORKER_MINT))
            .send()
            .await?
            .watch()
            .await?;
    }

    // Poster approves market, then workers approve registry + register.
    {
        let provider = ctx.wallet_provider("poster0")?;
        let token = MockERC20::new(token_addr, provider);
        token
            .approve(market_addr, U256::MAX)
            .send()
            .await?
            .watch()
            .await?;
    }
    for i in 0..WORKER_COUNT {
        let name = format!("worker{i}");
        let provider = ctx.wallet_provider(&name)?;
        let token = MockERC20::new(token_addr, provider.clone());
        token
            .approve(registry_addr, U256::MAX)
            .send()
            .await?
            .watch()
            .await?;
        let registry = WorkerRegistry::new(registry_addr, provider);
        let tx = registry.register(U256::from(STAKE)).send().await;
        match tx {
            Ok(pending) => {
                pending.watch().await?;
            }
            Err(e) => {
                // Already-registered if rerun — tolerate and move on.
                tracing::warn!(worker = %name, "register: {e}");
            }
        }
    }
    Ok(())
}

async fn run_single_job(
    ctx: &ChainCtx,
    llm: Arc<dyn LlmProvider>,
    round: u32,
) -> anyhow::Result<()> {
    let market_addr = ctx.address_of("BountyMarket")?;
    let poster_provider = ctx.wallet_provider("poster0")?;
    let market = BountyMarket::new(market_addr, poster_provider.clone());

    // --- LLM leaf: bounty amount + spec ----------------------------------
    let bounty_val = llm
        .fill(LlmRequest {
            slot: "bounty_amount".into(),
            context: serde_json::json!({ "round": round }),
        })
        .await?
        .as_u64()
        .unwrap_or(50);
    let spec_val = llm
        .fill(LlmRequest {
            slot: "job_spec".into(),
            context: serde_json::json!({ "round": round }),
        })
        .await?
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    // Rust validates + bounds the LLM output.
    let bounty_wei = U256::from(bounty_val.max(10).min(500)) * U256::from(10u128.pow(18));
    let spec_hash = keccak256(spec_val.as_bytes());
    let deadline: u64 = current_timestamp() + 3600;

    // --- Post job ---------------------------------------------------------
    let pending = market
        .postJob(spec_hash.into(), bounty_wei, deadline, 1) // Standard tier
        .send()
        .await?;
    let receipt = pending.get_receipt().await?;
    if !receipt.status() {
        return Err(anyhow::anyhow!("postJob reverted"));
    }
    let next_id = market.nextJobId().call().await?;
    let job_id: U256 = next_id - U256::from(1);
    tracing::info!(round, %job_id, bounty_wei = %bounty_wei, "posted");

    // --- Assign worker (round-robin) --------------------------------------
    let worker_index = (round as usize) % WORKER_COUNT;
    let worker_name = format!("worker{worker_index}");
    let worker_addr = ctx.wallet_address(&worker_name)?;
    market
        .assign(job_id, worker_addr)
        .send()
        .await?
        .watch()
        .await?;
    tracing::info!(round, worker = %worker_name, "assigned");

    // --- LLM leaf: submission content -------------------------------------
    let submission_content = llm
        .fill(LlmRequest {
            slot: "submission_content".into(),
            context: serde_json::json!({ "job_id": job_id.to_string() }),
        })
        .await?
        .as_str()
        .unwrap_or("default")
        .to_string();
    let result_hash = keccak256(submission_content.as_bytes());

    // --- Worker submits ---------------------------------------------------
    let worker_provider = ctx.wallet_provider(&worker_name)?;
    let market_as_worker = BountyMarket::new(market_addr, worker_provider);
    market_as_worker
        .submit(job_id, result_hash.into())
        .send()
        .await?
        .watch()
        .await?;

    // --- Resolver (deployer) accepts --------------------------------------
    let deployer_provider = ctx.wallet_provider("deployer")?;
    let market_as_resolver = BountyMarket::new(market_addr, deployer_provider);
    market_as_resolver
        .resolve(job_id, true)
        .send()
        .await?
        .watch()
        .await?;
    tracing::info!(round, %job_id, "resolved");
    Ok(())
}

fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// Silence unused-import warnings for the single-file fallback.
#[allow(dead_code)]
fn _unused(_: Address) {}
