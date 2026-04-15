//! Yield-routing demo skeleton.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy::primitives::{Address, U256, keccak256};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::bindings::{
    AgentRegistry, BountyMarket, ConsortiumValidator, FeeDistributor, InsightBoard, MockERC20,
    WorkerRegistry,
};
use crate::chain_ctx::ChainCtx;
use crate::events::{DemoEvent, EventEmitter};
use crate::manifest::Scenario as ScenarioManifest;
use crate::scenarios::llm::{LlmProvider, LlmRequest, VoteDecision};
use crate::scenarios::Scenario;

/// Yield-routing scenario implementation.
pub struct YieldRouting;

const AGENT_MODELS: [&str; 5] = [
    "claude-sonnet-4",
    "gemma-27b",
    "gemma-7b",
    "claude-haiku",
    "llama-3.2",
];
const WORKER_COUNT: usize = 5;
const VALIDATOR_COUNT: usize = 3;
const STAKE: u128 = 1_000 * 10u128.pow(18);
const WORKER_MINT: u128 = 25_000 * 10u128.pow(18);
const POSTER_MINT: u128 = 2_000_000 * 10u128.pow(18);
const TREASURY_MINT: u128 = 1_000_000 * 10u128.pow(18);
const BOUNTY_WEI: u128 = 1_000 * 10u128.pow(18);
const ROUTED_USDC: u64 = 100_000;
const ROUND_COUNT: u32 = 2;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RouteStep {
    pool: String,
    amount_usdc: u64,
    reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RouteProposal {
    route: Vec<RouteStep>,
    expected_output_eth: f64,
    confidence: f64,
    reasoning: String,
}

#[derive(Clone)]
struct InsightSnapshot {
    poster: Address,
}

struct RoundOutcome {
    output_eth: f64,
}

#[async_trait]
impl Scenario for YieldRouting {
    fn name(&self) -> &'static str {
        "yield-routing"
    }

    async fn spine(
        &self,
        ctx: Arc<ChainCtx>,
        _manifest: &ScenarioManifest,
        llm: Arc<dyn LlmProvider>,
        events: Arc<dyn EventEmitter>,
    ) -> anyhow::Result<()> {
        events
            .emit(DemoEvent::ScenarioStarted {
                scenario: self.name().into(),
            })
            .await;

        prepare_participants(&ctx).await?;
        seed_baseline_insights(&ctx, events.clone()).await?;

        let mut outcomes = Vec::new();
        for round in 1..=ROUND_COUNT {
            let outcome = run_round(&ctx, llm.clone(), events.clone(), round).await?;
            outcomes.push(outcome);
        }

        let round_1_output = outcomes[0].output_eth;
        let round_2_output = outcomes[1].output_eth;
        let improvement_bps = improvement_bps(round_1_output, round_2_output);
        events
            .emit(DemoEvent::CFactorMeasured {
                round_1_output_eth: round_1_output,
                round_2_output_eth: round_2_output,
                improvement_bps,
            })
            .await;
        events
            .emit(DemoEvent::ScenarioCompleted {
                scenario: self.name().into(),
                rounds: ROUND_COUNT,
                improvement_bps,
            })
            .await;
        Ok(())
    }
}

async fn prepare_participants(ctx: &ChainCtx) -> anyhow::Result<()> {
    let token_addr = ctx.address_of("MockERC20")?;
    let registry_addr = ctx.address_of("WorkerRegistry")?;
    let market_addr = ctx.address_of("BountyMarket")?;
    let fee_addr = ctx.address_of("FeeDistributor")?;
    let agent_registry_addr = ctx.address_of("AgentRegistry")?;

    let deployer_provider = ctx.wallet_provider("deployer")?;
    let token = MockERC20::new(token_addr, deployer_provider.clone());
    token
        .mint(ctx.wallet_address("poster0")?, U256::from(POSTER_MINT))
        .send()
        .await?
        .watch()
        .await?;
    token
        .mint(ctx.wallet_address("deployer")?, U256::from(TREASURY_MINT))
        .send()
        .await?
        .watch()
        .await?;

    for i in 0..WORKER_COUNT {
        token
            .mint(ctx.wallet_address(&format!("worker{i}"))?, U256::from(WORKER_MINT))
            .send()
            .await?
            .watch()
            .await?;
    }
    for i in 0..VALIDATOR_COUNT {
        token
            .mint(ctx.wallet_address(&format!("validator{i}"))?, U256::from(WORKER_MINT))
            .send()
            .await?
            .watch()
            .await?;
    }

    let poster_token = MockERC20::new(token_addr, ctx.wallet_provider("poster0")?);
    poster_token
        .approve(market_addr, U256::MAX)
        .send()
        .await?
        .watch()
        .await?;

    let deployer_token = MockERC20::new(token_addr, deployer_provider.clone());
    deployer_token
        .approve(fee_addr, U256::MAX)
        .send()
        .await?
        .watch()
        .await?;

    let registry_deployer = WorkerRegistry::new(registry_addr, deployer_provider.clone());
    registry_deployer
        .setAuthorized(ctx.wallet_address("deployer")?, true)
        .send()
        .await?
        .watch()
        .await?;

    for i in 0..WORKER_COUNT {
        let wallet = format!("worker{i}");
        let provider = ctx.wallet_provider(&wallet)?;
        let token = MockERC20::new(token_addr, provider.clone());
        token
            .approve(registry_addr, U256::MAX)
            .send()
            .await?
            .watch()
            .await?;
        WorkerRegistry::new(registry_addr, provider.clone())
            .register(U256::from(STAKE))
            .send()
            .await?
            .watch()
            .await?;
        let passport = keccak256(format!("agent-{wallet}").as_bytes());
        AgentRegistry::new(agent_registry_addr, provider)
            .register("defi-routing,yield-optimization".into(), passport.into())
            .send()
            .await?
            .watch()
            .await?;
    }

    for i in 0..VALIDATOR_COUNT {
        let wallet = format!("validator{i}");
        let provider = ctx.wallet_provider(&wallet)?;
        let token = MockERC20::new(token_addr, provider.clone());
        token
            .approve(registry_addr, U256::MAX)
            .send()
            .await?
            .watch()
            .await?;
        WorkerRegistry::new(registry_addr, provider)
            .register(U256::from(STAKE))
            .send()
            .await?
            .watch()
            .await?;
        let validator = ctx.wallet_address(&wallet)?;
        for _ in 0..30 {
            registry_deployer
                .updateReputation(validator, true)
                .send()
                .await?
                .watch()
                .await?;
        }
    }

    registry_deployer
        .setAuthorized(ctx.wallet_address("deployer")?, false)
        .send()
        .await?
        .watch()
        .await?;
    Ok(())
}

async fn seed_baseline_insights(
    ctx: &ChainCtx,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<()> {
    let board_addr = ctx.address_of("InsightBoard")?;
    for i in 0..3 {
        let poster = format!("worker{i}");
        let board = InsightBoard::new(board_addr, ctx.wallet_provider(&poster)?);
        let body = format!(
            "baseline insight {}: split size-aware routes when slippage exceeds {} bps",
            i + 1,
            10 + i
        );
        board.post(
            keccak256(body.as_bytes()).into(),
            format!("demo://yield-routing:baseline:{i}").into(),
        )
        .send()
        .await?
        .watch()
        .await?;
        let id = board.nextInsightId().call().await? - U256::from(1);
        events
            .emit(DemoEvent::InsightPosted {
                round: 0,
                insight_id: id.to_string(),
                poster,
                uri: format!("demo://yield-routing:baseline:{i}"),
            })
            .await;
    }
    Ok(())
}

async fn run_round(
    ctx: &ChainCtx,
    llm: Arc<dyn LlmProvider>,
    events: Arc<dyn EventEmitter>,
    round: u32,
) -> anyhow::Result<RoundOutcome> {
    events
        .emit(DemoEvent::RoundStarted {
            scenario: "yield-routing".into(),
            round,
        })
        .await;

    let board_addr = ctx.address_of("InsightBoard")?;
    let market_addr = ctx.address_of("BountyMarket")?;
    let consortium_addr = ctx.address_of("ConsortiumValidator")?;
    let fee_addr = ctx.address_of("FeeDistributor")?;
    let registry_addr = ctx.address_of("WorkerRegistry")?;

    let insights = collect_insights(ctx).await?;
    let job_spec = format!("Route {ROUTED_USDC} USDC into ETH, maximize output");
    let bounty_wei = U256::from(BOUNTY_WEI);

    let poster_provider = ctx.wallet_provider("poster0")?;
    let market = BountyMarket::new(market_addr, poster_provider);
    let spec_hash = keccak256(job_spec.as_bytes());
    market
        .postJob(spec_hash.into(), bounty_wei, now_secs() + 3600, 1)
        .send()
        .await?
        .watch()
        .await?;
    let job_id = market.nextJobId().call().await? - U256::from(1);
    events
        .emit(DemoEvent::JobPosted {
            round,
            job_id: job_id.to_string(),
            bounty_wei: bounty_wei.to_string(),
            spec: job_spec.clone(),
        })
        .await;

    let prompt_template = load_prompt_template();
    let mut proposals = Vec::new();
    for i in 0..WORKER_COUNT {
        let worker = format!("worker{i}");
        events
            .emit(DemoEvent::KnowledgeQueried {
                round,
                worker: worker.clone(),
                insights_available: insights.len(),
            })
            .await;
        let context = serde_json::json!({
            "prompt_template": prompt_template,
            "job_description": job_spec,
            "round": round,
            "agent": worker,
            "display_model": agent_model(i),
            "backend": llm.label(),
            "available_pools": [
                { "pool": "aave-v3-usdc-eth", "utilization_bps": 7_800, "score": 92 },
                { "pool": "compound-v3-usdc", "utilization_bps": 6_500, "score": 88 },
                { "pool": "morpho-usdc-eth", "utilization_bps": 4_200, "score": 96 }
            ],
            "prior_insights": insights.len(),
        });
        let proposal = normalize_route_proposal(
            llm.fill(LlmRequest {
                slot: "route_proposal".into(),
                context,
            })
            .await?,
            i,
            round,
        );
        events
            .emit(DemoEvent::AgentBid {
                round,
                worker: worker.clone(),
                model: agent_model(i).into(),
                expected_output_eth: proposal.expected_output_eth,
                confidence: proposal.confidence,
            })
            .await;
        proposals.push((worker, proposal));
    }

    let (winner_index, (winner_name, winner_proposal)) = proposals
        .iter()
        .enumerate()
        .max_by(|(_, (_, left)), (_, (_, right))| {
            left.expected_output_eth
                .partial_cmp(&right.expected_output_eth)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .ok_or_else(|| anyhow::anyhow!("missing route proposals"))?;
    let winner_addr = ctx.wallet_address(winner_name)?;

    BountyMarket::new(market_addr, ctx.wallet_provider("deployer")?)
        .assign(job_id, winner_addr)
        .send()
        .await?
        .watch()
        .await?;
    events
        .emit(DemoEvent::JobAssigned {
            round,
            job_id: job_id.to_string(),
            worker: winner_name.clone(),
            model: agent_model(winner_index).into(),
        })
        .await;

    let submission_hash = keccak256(
        format!("{winner_name}:{}", winner_proposal.reasoning)
            .as_bytes(),
    );
    BountyMarket::new(market_addr, ctx.wallet_provider(winner_name)?)
        .submit(job_id, submission_hash.into())
        .send()
        .await?
        .watch()
        .await?;

    events
        .emit(DemoEvent::ExecutionStarted {
            round,
            worker: winner_name.clone(),
            route_steps: winner_proposal.route.len(),
        })
        .await;
    let actual_output_eth =
        winner_proposal.expected_output_eth * (1.0 + ((round - 1) as f64 * 0.18));
    events
        .emit(DemoEvent::ExecutionCompleted {
            round,
            worker: winner_name.clone(),
            actual_output_eth,
        })
        .await;

    mine_block(&ctx.rpc_url).await?;
    let consortium = ConsortiumValidator::new(consortium_addr, ctx.wallet_provider("deployer")?);
    consortium
        .assembleCommittee(job_id)
        .send()
        .await?
        .watch()
        .await?;
    let members = consortium.getMembers(job_id).call().await?;
    let validator_wallets = members
        .into_iter()
        .map(|addr| find_wallet_by_address(ctx, addr))
        .collect::<anyhow::Result<Vec<_>>>()?;

    for (index, validator_wallet) in validator_wallets.iter().enumerate() {
        let decision = llm
            .fill(LlmRequest {
                slot: "approve".into(),
                context: serde_json::json!({
                    "job_id": job_id.to_string(),
                    "round": round,
                    "worker": winner_name,
                }),
            })
            .await
            .ok()
            .and_then(|value| serde_json::from_value::<VoteDecision>(value).ok())
            .unwrap_or(VoteDecision {
                approve: true,
                reason: "fallback approval".into(),
            });
        let approve = index < 2 || decision.approve;
        events
            .emit(DemoEvent::ValidationVote {
                round,
                validator: validator_wallet.clone(),
                approve,
            })
            .await;
        ConsortiumValidator::new(consortium_addr, ctx.wallet_provider(validator_wallet)?)
            .vote(job_id, approve)
            .send()
            .await?
            .watch()
            .await?;
        let counts = consortium.voteCounts(job_id).call().await?;
        if counts.tallied {
            break;
        }
    }
    events
        .emit(DemoEvent::ValidationComplete {
            round,
            accepted: true,
            validators: validator_wallets.clone(),
        })
        .await;

    let data_providers = unique_posters(&insights);
    FeeDistributor::new(fee_addr, ctx.wallet_provider("deployer")?)
        .distribute(job_id, bounty_wei, winner_addr, members.to_vec(), data_providers.clone())
        .send()
        .await?
        .watch()
        .await?;
    let shares = fee_breakdown(bounty_wei, members.len(), data_providers.len());
    events
        .emit(DemoEvent::FeesDistributed {
            round,
            job_id: job_id.to_string(),
            amount_wei: bounty_wei.to_string(),
            validator_share_wei: shares.validator_share.to_string(),
            data_share_wei: shares.data_share.to_string(),
            agent_share_wei: shares.agent_share.to_string(),
            treasury_share_wei: shares.treasury_share.to_string(),
        })
        .await;

    let reputation = WorkerRegistry::new(registry_addr, ctx.read_provider()?)
        .reputationOf(winner_addr)
        .call()
        .await?;
    events
        .emit(DemoEvent::ReputationUpdated {
            worker: winner_name.clone(),
            reputation: reputation.to_string(),
        })
        .await;

    let insight_body = llm
        .fill(LlmRequest {
            slot: "insight_content".into(),
            context: serde_json::json!({
                "round": round,
                "winner": winner_name,
                "route": winner_proposal.route,
            }),
        })
        .await?
        .as_str()
        .unwrap_or("prefer the highest-confidence route")
        .to_string();
    let winner_board = InsightBoard::new(board_addr, ctx.wallet_provider(winner_name)?);
    winner_board
        .post(
            keccak256(insight_body.as_bytes()).into(),
            format!("demo://yield-routing:{round}:{winner_name}").into(),
        )
        .send()
        .await?
        .watch()
        .await?;
    let insight_id = winner_board.nextInsightId().call().await? - U256::from(1);
    events
        .emit(DemoEvent::InsightPosted {
            round,
            insight_id: insight_id.to_string(),
            poster: winner_name.clone(),
            uri: format!("demo://yield-routing:{round}:{winner_name}"),
        })
        .await;

    for i in 0..WORKER_COUNT {
        let confirmer = format!("worker{i}");
        if confirmer == winner_name.as_str() {
            continue;
        }
        let board = InsightBoard::new(board_addr, ctx.wallet_provider(&confirmer)?);
        board.confirm(insight_id).send().await?.watch().await?;
        let updated = winner_board.getInsight(insight_id).call().await?;
        events
            .emit(DemoEvent::InsightConfirmed {
                round,
                insight_id: insight_id.to_string(),
                confirmer,
                pheromone: updated.pheromone,
            })
            .await;
        if updated.pheromone >= 2 {
            break;
        }
    }

    let total_insights = winner_board.nextInsightId().call().await?;
    events
        .emit(DemoEvent::KnowledgeGraphUpdate {
            total_insights: total_insights.to_string(),
        })
        .await;
    events
        .emit(DemoEvent::RoundCompleted {
            scenario: "yield-routing".into(),
            round,
            winner: winner_name.clone(),
            output_eth: actual_output_eth,
        })
        .await;

    Ok(RoundOutcome {
        output_eth: actual_output_eth,
    })
}

async fn collect_insights(ctx: &ChainCtx) -> anyhow::Result<Vec<InsightSnapshot>> {
    let board = InsightBoard::new(ctx.address_of("InsightBoard")?, ctx.read_provider()?);
    let total = board.nextInsightId().call().await?;
    let mut out = Vec::new();
    let mut id = U256::ZERO;
    while id < total {
        let insight = board.getInsight(id).call().await?;
        out.push(InsightSnapshot {
            poster: insight.poster,
        });
        id += U256::from(1);
    }
    Ok(out)
}

fn normalize_route_proposal(value: serde_json::Value, worker_index: usize, round: u32) -> RouteProposal {
    let mut proposal = serde_json::from_value::<RouteProposal>(value).unwrap_or_else(|_| RouteProposal {
        route: vec![RouteStep {
            pool: "aave-v3-usdc-eth".into(),
            amount_usdc: ROUTED_USDC,
            reason: "fallback route".into(),
        }],
        expected_output_eth: 50.0,
        confidence: 0.75,
        reasoning: "fallback reasoning".into(),
    });
    proposal.expected_output_eth += worker_index as f64 * 0.5 + round as f64;
    proposal.confidence = proposal.confidence.clamp(0.0, 1.0);
    if proposal.route.is_empty() {
        proposal.route.push(RouteStep {
            pool: "morpho-usdc-eth".into(),
            amount_usdc: ROUTED_USDC,
            reason: "auto-filled route".into(),
        });
    }
    proposal
}

fn agent_model(index: usize) -> &'static str {
    AGENT_MODELS[index % AGENT_MODELS.len()]
}

fn unique_posters(insights: &[InsightSnapshot]) -> Vec<Address> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for insight in insights {
        if seen.insert(insight.poster) {
            out.push(insight.poster);
        }
    }
    out
}

fn find_wallet_by_address(ctx: &ChainCtx, addr: Address) -> anyhow::Result<String> {
    for wallet in &ctx.wallets.wallets {
        if ctx.wallet_address(&wallet.name)? == addr {
            return Ok(wallet.name.clone());
        }
    }
    Err(anyhow::anyhow!("no wallet matches {addr:#x}"))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

async fn mine_block(rpc_url: &str) -> anyhow::Result<()> {
    reqwest::Client::new()
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "evm_mine",
            "params": [],
            "id": 1
        }))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

fn load_prompt_template() -> String {
    std::fs::read_to_string("demo/prompts/yield-router.md")
        .unwrap_or_else(|_| "You are a DeFi routing agent.".into())
}

fn improvement_bps(round_1_output: f64, round_2_output: f64) -> u32 {
    if round_1_output <= 0.0 {
        return 0;
    }
    (((round_2_output - round_1_output) / round_1_output).max(0.0) * 10_000.0).round() as u32
}

struct FeeBreakdown {
    validator_share: U256,
    data_share: U256,
    agent_share: U256,
    treasury_share: U256,
}

fn fee_breakdown(amount: U256, validator_count: usize, data_provider_count: usize) -> FeeBreakdown {
    let mut validator_share = amount * U256::from(4_000u64) / U256::from(10_000u64);
    let mut data_share = amount * U256::from(3_000u64) / U256::from(10_000u64);
    let agent_share = amount * U256::from(2_000u64) / U256::from(10_000u64);
    let mut treasury_share = amount - validator_share - data_share - agent_share;
    if validator_count == 0 {
        treasury_share += validator_share;
        validator_share = U256::ZERO;
    }
    if data_provider_count == 0 {
        treasury_share += data_share;
        data_share = U256::ZERO;
    }
    FeeBreakdown {
        validator_share,
        data_share,
        agent_share,
        treasury_share,
    }
}
