//! DeFi routing scenario.
//!
//! Simplified spine for the benchmark story: poster posts a single routing
//! job; N workers each race to submit a route proposal; first worker gets
//! paid. Real benchmark metrics are collected post-run via `verify`.

use std::sync::Arc;

use alloy::primitives::{U256, keccak256};
use async_trait::async_trait;

use crate::bindings::{BountyMarket, MockERC20, WorkerRegistry};
use crate::chain_ctx::ChainCtx;
use crate::events::EventEmitter;
use crate::manifest::Scenario as ScenarioManifest;
use crate::scenarios::Scenario;
use crate::scenarios::llm::{LlmProvider, LlmRequest};

/// DeFi routing benchmark scenario.
pub struct DefiRouting;

const WORKERS: usize = 5;
const WORKER_MINT: u128 = 10_000 * 10u128.pow(18);
const POSTER_MINT: u128 = 1_000_000 * 10u128.pow(18);
const STAKE: u128 = 1_000 * 10u128.pow(18);

#[async_trait]
impl Scenario for DefiRouting {
    fn name(&self) -> &'static str {
        "defi-routing"
    }

    async fn spine(
        &self,
        ctx: Arc<ChainCtx>,
        _manifest: &ScenarioManifest,
        llm: Arc<dyn LlmProvider>,
        _events: Arc<dyn EventEmitter>,
    ) -> anyhow::Result<()> {
        prepare(&ctx).await?;
        race(&ctx, llm).await
    }
}

async fn prepare(ctx: &ChainCtx) -> anyhow::Result<()> {
    let deployer = ctx.wallet_provider("deployer")?;
    let token_addr = ctx.address_of("MockERC20")?;
    let registry_addr = ctx.address_of("WorkerRegistry")?;
    let market_addr = ctx.address_of("BountyMarket")?;
    let token_d = MockERC20::new(token_addr, deployer.clone());
    token_d
        .mint(ctx.wallet_address("poster0")?, U256::from(POSTER_MINT))
        .send()
        .await?
        .watch()
        .await?;
    let poster_token = MockERC20::new(token_addr, ctx.wallet_provider("poster0")?);
    poster_token
        .approve(market_addr, U256::MAX)
        .send()
        .await?
        .watch()
        .await?;
    for i in 0..WORKERS {
        let name = format!("worker{i}");
        let addr = ctx.wallet_address(&name)?;
        token_d
            .mint(addr, U256::from(WORKER_MINT))
            .send()
            .await?
            .watch()
            .await?;
        let wprov = ctx.wallet_provider(&name)?;
        let token = MockERC20::new(token_addr, wprov.clone());
        token
            .approve(registry_addr, U256::MAX)
            .send()
            .await?
            .watch()
            .await?;
        let reg = WorkerRegistry::new(registry_addr, wprov);
        let _ = reg.register(U256::from(STAKE)).send().await;
    }
    Ok(())
}

async fn race(ctx: &ChainCtx, llm: Arc<dyn LlmProvider>) -> anyhow::Result<()> {
    let market_addr = ctx.address_of("BountyMarket")?;
    let poster = ctx.wallet_provider("poster0")?;
    let market = BountyMarket::new(market_addr, poster);
    let deadline = current_timestamp() + 3600;
    let spec = keccak256(b"defi-routing-benchmark");
    market
        .postJob(
            spec.into(),
            U256::from(200u128 * 10u128.pow(18)),
            deadline,
            1,
        )
        .send()
        .await?
        .watch()
        .await?;
    let job_id: U256 = market.nextJobId().call().await? - U256::from(1);

    // First worker wins the race (simplest deterministic outcome).
    let deployer_provider = ctx.wallet_provider("deployer")?;
    let market_d = BountyMarket::new(market_addr, deployer_provider);
    let winner_name = "worker0".to_string();
    market_d
        .assign(job_id, ctx.wallet_address(&winner_name)?)
        .send()
        .await?
        .watch()
        .await?;

    let proposal = llm
        .fill(LlmRequest {
            slot: "route_proposal".into(),
            context: serde_json::json!({ "job_id": job_id.to_string() }),
        })
        .await?;
    let submission = keccak256(proposal.to_string().as_bytes());
    let worker_prov = ctx.wallet_provider(&winner_name)?;
    let market_w = BountyMarket::new(market_addr, worker_prov);
    market_w
        .submit(job_id, submission.into())
        .send()
        .await?
        .watch()
        .await?;
    market_d.resolve(job_id, true).send().await?.watch().await?;
    Ok(())
}

fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
