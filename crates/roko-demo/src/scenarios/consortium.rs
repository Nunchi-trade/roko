//! Consortium validation scenario.
//!
//! Spine: poster posts → worker submits → assembleCommittee (picks 3 Trusted
//! validators) → each validator votes (LLM fills approve) → tally triggers
//! BountyMarket.resolve via delegated resolver.

use std::sync::Arc;

use alloy::primitives::{U256, keccak256};
use async_trait::async_trait;

use crate::bindings::{BountyMarket, ConsortiumValidator, MockERC20, WorkerRegistry};
use crate::chain_ctx::ChainCtx;
use crate::manifest::Scenario as ScenarioManifest;
use crate::scenarios::Scenario;
use crate::scenarios::ScenarioRuntime;
use crate::scenarios::llm::{LlmProvider, LlmRequest, VoteDecision};

/// Consortium validation scenario.
pub struct Consortium;

const WORKER_MINT: u128 = 10_000 * 10u128.pow(18);
const POSTER_MINT: u128 = 1_000_000 * 10u128.pow(18);
const STAKE: u128 = 1_000 * 10u128.pow(18);
const VALIDATOR_COUNT: usize = 3;

#[async_trait]
impl Scenario for Consortium {
    fn name(&self) -> &'static str {
        "consortium"
    }

    async fn spine(
        &self,
        ctx: Arc<ChainCtx>,
        _manifest: &ScenarioManifest,
        runtime: Arc<ScenarioRuntime>,
    ) -> anyhow::Result<()> {
        prepare(&ctx).await?;
        let job_id = post_and_submit_job(&ctx).await?;
        assemble_and_vote(&ctx, runtime.llm.clone(), job_id).await?;
        Ok(())
    }
}

async fn prepare(ctx: &ChainCtx) -> anyhow::Result<()> {
    let deployer_provider = ctx.wallet_provider("deployer")?;
    let token_addr = ctx.address_of("MockERC20")?;
    let registry_addr = ctx.address_of("WorkerRegistry")?;
    let token_via_deployer = MockERC20::new(token_addr, deployer_provider.clone());

    // Fund poster, worker, and 3 validators.
    for (name, amt) in [
        ("poster0", POSTER_MINT),
        ("worker0", WORKER_MINT),
        ("validator0", WORKER_MINT),
        ("validator1", WORKER_MINT),
        ("validator2", WORKER_MINT),
    ] {
        let addr = ctx.wallet_address(name)?;
        token_via_deployer
            .mint(addr, U256::from(amt))
            .send()
            .await?
            .watch()
            .await?;
    }
    // Poster approves market.
    let market_addr = ctx.address_of("BountyMarket")?;
    let poster_token = MockERC20::new(token_addr, ctx.wallet_provider("poster0")?);
    poster_token
        .approve(market_addr, U256::MAX)
        .send()
        .await?
        .watch()
        .await?;

    // Register worker at Standard tier.
    let worker_provider = ctx.wallet_provider("worker0")?;
    let token = MockERC20::new(token_addr, worker_provider.clone());
    token
        .approve(registry_addr, U256::MAX)
        .send()
        .await?
        .watch()
        .await?;
    let registry_as_worker = WorkerRegistry::new(registry_addr, worker_provider);
    let _ = registry_as_worker.register(U256::from(STAKE)).send().await;

    // Register validators and promote each to Trusted tier via 30 positive
    // reputation updates (EMA crosses the 0.80 Trusted threshold).
    let registry_as_deployer = WorkerRegistry::new(registry_addr, deployer_provider.clone());
    for i in 0..VALIDATOR_COUNT {
        let name = format!("validator{i}");
        let provider = ctx.wallet_provider(&name)?;
        let token = MockERC20::new(token_addr, provider.clone());
        token
            .approve(registry_addr, U256::MAX)
            .send()
            .await?
            .watch()
            .await?;
        let registry_as_validator = WorkerRegistry::new(registry_addr, provider);
        let _ = registry_as_validator
            .register(U256::from(STAKE))
            .send()
            .await;
        let validator_addr = ctx.wallet_address(&name)?;
        // Pump reputation via deployer's authorized `updateReputation` calls.
        // Deployer is authorized for both market + consortium via scenario wiring,
        // but *not* for direct updateReputation. Temporarily authorize deployer.
        registry_as_deployer
            .setAuthorized(ctx.wallet_address("deployer")?, true)
            .send()
            .await?
            .watch()
            .await?;
        for _ in 0..30 {
            registry_as_deployer
                .updateReputation(validator_addr, true)
                .send()
                .await?
                .watch()
                .await?;
        }
    }
    // Revoke deployer (leave only market + consortium authorized).
    registry_as_deployer
        .setAuthorized(ctx.wallet_address("deployer")?, false)
        .send()
        .await?
        .watch()
        .await?;
    Ok(())
}

async fn post_and_submit_job(ctx: &ChainCtx) -> anyhow::Result<U256> {
    let market_addr = ctx.address_of("BountyMarket")?;
    let poster_provider = ctx.wallet_provider("poster0")?;
    let market = BountyMarket::new(market_addr, poster_provider);
    let deadline = current_timestamp() + 3600;
    let spec = keccak256(b"consortium-job");
    market
        .postJob(
            spec.into(),
            U256::from(100u128 * 10u128.pow(18)),
            deadline,
            1,
        )
        .send()
        .await?
        .watch()
        .await?;
    let job_id: U256 = market.nextJobId().call().await? - U256::from(1);

    let worker_addr = ctx.wallet_address("worker0")?;
    let deployer_provider = ctx.wallet_provider("deployer")?;
    let market_as_deployer = BountyMarket::new(market_addr, deployer_provider);
    market_as_deployer
        .assign(job_id, worker_addr)
        .send()
        .await?
        .watch()
        .await?;

    let worker_provider = ctx.wallet_provider("worker0")?;
    let market_as_worker = BountyMarket::new(market_addr, worker_provider);
    let result = keccak256(b"result");
    market_as_worker
        .submit(job_id, result.into())
        .send()
        .await?
        .watch()
        .await?;
    Ok(job_id)
}

async fn assemble_and_vote(
    ctx: &ChainCtx,
    llm: Arc<dyn LlmProvider>,
    job_id: U256,
) -> anyhow::Result<()> {
    let consortium_addr = ctx.address_of("ConsortiumValidator")?;
    let deployer_provider = ctx.wallet_provider("deployer")?;
    let consortium = ConsortiumValidator::new(consortium_addr, deployer_provider);
    // Advance one block so blockhash(block.number - 1) != 0.
    mine_block(&ctx.rpc_url).await?;
    consortium
        .assembleCommittee(job_id)
        .send()
        .await?
        .watch()
        .await?;
    let members = consortium.getMembers(job_id).call().await?;
    tracing::info!("committee: {} {} {}", members[0], members[1], members[2]);

    // Each selected validator votes. Match the on-chain address back to the
    // wallet name by brute-force lookup (small set).
    for &member in members.iter() {
        let name = find_wallet_by_address(ctx, member)?;
        let provider = ctx.wallet_provider(&name)?;
        let c = ConsortiumValidator::new(consortium_addr, provider);
        let decision = llm
            .fill(LlmRequest {
                slot: "approve".into(),
                context: serde_json::json!({ "job_id": job_id.to_string() }),
            })
            .await?;
        let decision: VoteDecision = serde_json::from_value(decision).unwrap_or(VoteDecision {
            approve: true,
            reason: "fallback".into(),
        });
        tracing::info!(validator = %name, approve = decision.approve, "vote");
        c.vote(job_id, decision.approve)
            .send()
            .await?
            .watch()
            .await?;
    }
    Ok(())
}

fn find_wallet_by_address(
    ctx: &ChainCtx,
    addr: alloy::primitives::Address,
) -> anyhow::Result<String> {
    for w in &ctx.wallets.wallets {
        let derived = ctx.wallet_address(&w.name)?;
        if derived == addr {
            return Ok(w.name.clone());
        }
    }
    Err(anyhow::anyhow!("no wallet matches {addr:#x}"))
}

fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

async fn mine_block(rpc_url: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let _ = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc":"2.0","method":"evm_mine","params":[],"id":1
        }))
        .send()
        .await?;
    Ok(())
}
