#![allow(missing_docs)]

//! Yield-routing demo spine and reusable round helpers.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
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
use crate::events::{DemoEvent, EventEmitter, KnowledgeEdge, KnowledgeNode};
use crate::manifest::Scenario as ScenarioManifest;
use crate::scenarios::llm::{LlmProvider, LlmRequest, VoteDecision};
use crate::scenarios::{Scenario, ScenarioRuntime};

/// Yield-routing scenario implementation.
pub struct YieldRouting;

/// Default scenario used by the batch surfaces.
pub const DEFAULT_SCENARIO_NAME: &str = "yield-routing";
/// Number of rounds in the core demo flow.
pub const DEFAULT_ROUNDS: u32 = 2;

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
const DEPLOYER_MINT: u128 = 1_000_000 * 10u128.pow(18);
const BOARD_FUND: u128 = 25_000 * 10u128.pow(18);
const BOUNTY_WEI: u128 = 1_000 * 10u128.pow(18);
const ROUTED_USDC: u64 = 100_000;
const SLASH_REASON_QUALITY_REJECT: u8 = 2;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RouteStep {
    pub pool: String,
    pub amount_usdc: u64,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RouteProposal {
    pub route: Vec<RouteStep>,
    pub expected_output_eth: f64,
    pub confidence: f64,
    pub reasoning: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InsightRecord {
    pub id: String,
    pub poster: String,
    pub uri: String,
    pub content_hash: String,
    pub posted_at: u64,
    pub pheromone: u64,
    pub confirmations: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BidSubmission {
    pub worker: String,
    pub worker_index: usize,
    pub model: String,
    pub proposal: RouteProposal,
    pub queried_insight_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RoundOutcome {
    pub round: u32,
    pub job_id: String,
    pub winner: String,
    pub winner_model: String,
    pub expected_output_eth: f64,
    pub output_eth: f64,
    pub confidence: f64,
    pub insights_before: usize,
    pub insight_id: String,
    pub validator_wallets: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SlashOutcome {
    pub worker: String,
    pub rejected_job_id: String,
    pub reason_code: u8,
    pub amount_wei: String,
    pub new_bond_wei: String,
    pub new_reputation: String,
}

#[derive(Clone)]
pub struct PreparedYieldRouting {
    pub ctx: Arc<ChainCtx>,
    pub runtime_dir: PathBuf,
    pub persist_reputation: bool,
    prompt_template: String,
    token_addr: Address,
    registry_addr: Address,
    market_addr: Address,
    consortium_addr: Address,
    fee_addr: Address,
    board_addr: Address,
    agent_registry_addr: Address,
}

#[derive(Clone)]
pub struct PostedJob {
    pub round: u32,
    pub job_id: U256,
    pub spec: String,
    pub bounty_wei: U256,
    pub insights_before: Vec<InsightRecord>,
}

#[async_trait]
impl Scenario for YieldRouting {
    fn name(&self) -> &'static str {
        DEFAULT_SCENARIO_NAME
    }

    async fn spine(
        &self,
        ctx: Arc<ChainCtx>,
        _manifest: &ScenarioManifest,
        runtime: Arc<ScenarioRuntime>,
    ) -> anyhow::Result<()> {
        runtime
            .events
            .emit(DemoEvent::ScenarioStarted {
                scenario: self.name().into(),
            })
            .await;

        let prepared = prepare(
            ctx,
            runtime.runtime_dir.clone(),
            runtime.persist_reputation,
            true,
            runtime.events.clone(),
        )
        .await?;

        let mut outcomes = Vec::new();
        for round in 1..=DEFAULT_ROUNDS {
            outcomes.push(
                run_round(
                    &prepared,
                    round,
                    runtime.llm.clone(),
                    runtime.events.clone(),
                )
                .await?,
            );
        }

        let _slash = run_adversarial_phase(&prepared, runtime.events.clone()).await?;
        if prepared.persist_reputation {
            save_reputation(&prepared).await?;
        }

        let improvement_bps = improvement_bps(outcomes[0].output_eth, outcomes[1].output_eth);
        runtime
            .events
            .emit(DemoEvent::CFactorMeasured {
                round_1_output_eth: outcomes[0].output_eth,
                round_2_output_eth: outcomes[1].output_eth,
                improvement_bps,
            })
            .await;
        runtime
            .events
            .emit(DemoEvent::ScenarioCompleted {
                scenario: self.name().into(),
                rounds: DEFAULT_ROUNDS,
                improvement_bps,
            })
            .await;
        Ok(())
    }
}

pub async fn prepare(
    ctx: Arc<ChainCtx>,
    runtime_dir: PathBuf,
    persist_reputation: bool,
    seed_baseline: bool,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<PreparedYieldRouting> {
    let prepared = PreparedYieldRouting {
        token_addr: ctx.address_of("MockERC20")?,
        registry_addr: ctx.address_of("WorkerRegistry")?,
        market_addr: ctx.address_of("BountyMarket")?,
        consortium_addr: ctx.address_of("ConsortiumValidator")?,
        fee_addr: ctx.address_of("FeeDistributor")?,
        board_addr: ctx.address_of("InsightBoard")?,
        agent_registry_addr: ctx.address_of("AgentRegistry")?,
        prompt_template: load_prompt_template(),
        ctx,
        runtime_dir,
        persist_reputation,
    };
    prepare_participants(&prepared).await?;
    if prepared.persist_reputation {
        restore_reputation(&prepared).await?;
    }
    if seed_baseline {
        seed_baseline_insights(&prepared, events).await?;
    }
    Ok(prepared)
}

pub async fn run_round(
    prepared: &PreparedYieldRouting,
    round: u32,
    llm: Arc<dyn LlmProvider>,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<RoundOutcome> {
    let posted = post_round_job(prepared, round, events.clone()).await?;
    let bids = collect_route_proposals(prepared, &posted, llm.clone(), events.clone()).await?;
    finalize_round(prepared, posted, bids, llm, events).await
}

pub async fn post_round_job(
    prepared: &PreparedYieldRouting,
    round: u32,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<PostedJob> {
    events
        .emit(DemoEvent::RoundStarted {
            scenario: DEFAULT_SCENARIO_NAME.into(),
            round,
        })
        .await;

    let insights_before = query_insights(prepared).await?;
    let spec = format!("Route {ROUTED_USDC} USDC into ETH, maximize output");
    let bounty_wei = U256::from(BOUNTY_WEI);
    let poster_provider = prepared.ctx.wallet_provider("poster0")?;
    let market = BountyMarket::new(prepared.market_addr, poster_provider);
    market
        .postJob(keccak256(spec.as_bytes()).into(), bounty_wei, now_secs() + 3600, 1)
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
            spec: spec.clone(),
        })
        .await;
    Ok(PostedJob {
        round,
        job_id,
        spec,
        bounty_wei,
        insights_before,
    })
}

pub async fn generate_agent_bid(
    prepared: &PreparedYieldRouting,
    llm: Arc<dyn LlmProvider>,
    round: u32,
    worker_index: usize,
    job_spec: &str,
    insights: &[InsightRecord],
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<BidSubmission> {
    let worker = format!("worker{worker_index}");
    events
        .emit(DemoEvent::KnowledgeQueried {
            round,
            worker: worker.clone(),
            insights_available: insights.len(),
        })
        .await;

    let proposal = normalize_route_proposal(
        llm.fill(LlmRequest {
            slot: "route_proposal".into(),
            context: serde_json::json!({
                "prompt_template": prepared.prompt_template,
                "job_description": job_spec,
                "round": round,
                "agent": worker,
                "display_model": agent_model(worker_index),
                "backend": llm.label(),
                "available_pools": [
                    { "pool": "aave-v3-usdc-eth", "utilization_bps": 7_800, "score": 92 },
                    { "pool": "compound-v3-usdc", "utilization_bps": 6_500, "score": 88 },
                    { "pool": "morpho-usdc-eth", "utilization_bps": 4_200, "score": 96 }
                ],
                "prior_insights": insights,
            }),
        })
        .await?,
        worker_index,
        round,
    );
    events
        .emit(DemoEvent::AgentBid {
            round,
            worker: worker.clone(),
            model: agent_model(worker_index).into(),
            expected_output_eth: proposal.expected_output_eth,
            confidence: proposal.confidence,
        })
        .await;

    Ok(BidSubmission {
        worker,
        worker_index,
        model: agent_model(worker_index).into(),
        queried_insight_ids: insights.iter().map(|insight| insight.id.clone()).collect(),
        proposal,
    })
}

pub async fn finalize_round(
    prepared: &PreparedYieldRouting,
    posted: PostedJob,
    bids: Vec<BidSubmission>,
    llm: Arc<dyn LlmProvider>,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<RoundOutcome> {
    let winning_bid = bids
        .iter()
        .max_by(|left, right| {
            left.proposal
                .expected_output_eth
                .partial_cmp(&right.proposal.expected_output_eth)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right.worker_index.cmp(&left.worker_index))
        })
        .ok_or_else(|| anyhow::anyhow!("missing route proposals"))?
        .clone();
    let winner_addr = prepared.ctx.wallet_address(&winning_bid.worker)?;

    BountyMarket::new(prepared.market_addr, prepared.ctx.wallet_provider("deployer")?)
        .assign(posted.job_id, winner_addr)
        .send()
        .await?
        .watch()
        .await?;
    events
        .emit(DemoEvent::JobAssigned {
            round: posted.round,
            job_id: posted.job_id.to_string(),
            worker: winning_bid.worker.clone(),
            model: winning_bid.model.clone(),
        })
        .await;

    let submission_hash = keccak256(
        format!("{}:{}", winning_bid.worker, winning_bid.proposal.reasoning).as_bytes(),
    );
    BountyMarket::new(
        prepared.market_addr,
        prepared.ctx.wallet_provider(&winning_bid.worker)?,
    )
    .submit(posted.job_id, submission_hash.into())
    .send()
    .await?
    .watch()
    .await?;

    events
        .emit(DemoEvent::ExecutionStarted {
            round: posted.round,
            worker: winning_bid.worker.clone(),
            route_steps: winning_bid.proposal.route.len(),
        })
        .await;
    let actual_output_eth = deterministic_execution_output(
        winning_bid.proposal.expected_output_eth,
        posted.round,
        posted.insights_before.len(),
    );
    events
        .emit(DemoEvent::ExecutionCompleted {
            round: posted.round,
            worker: winning_bid.worker.clone(),
            actual_output_eth,
        })
        .await;

    let validator_wallets = approve_round(
        prepared,
        posted.round,
        posted.job_id,
        &winning_bid,
        llm.clone(),
        events.clone(),
    )
    .await?;
    distribute_fees(prepared, posted.round, posted.job_id, posted.bounty_wei, &winning_bid, &posted.insights_before, &validator_wallets, events.clone()).await?;

    let insight_id = post_winner_insight(
        prepared,
        llm,
        posted.round,
        &winning_bid,
        events.clone(),
    )
    .await?;

    let query_edges = bids
        .iter()
        .flat_map(|bid| {
            bid.queried_insight_ids.iter().map(|insight_id| KnowledgeEdge {
                from: bid.worker.clone(),
                to: insight_node_id(insight_id),
                kind: "queried".into(),
            })
        })
        .collect::<Vec<_>>();
    emit_knowledge_graph(prepared, posted.round, query_edges, events.clone()).await?;

    let outcome = RoundOutcome {
        round: posted.round,
        job_id: posted.job_id.to_string(),
        winner: winning_bid.worker.clone(),
        winner_model: winning_bid.model,
        expected_output_eth: winning_bid.proposal.expected_output_eth,
        output_eth: actual_output_eth,
        confidence: winning_bid.proposal.confidence,
        insights_before: posted.insights_before.len(),
        insight_id: insight_id.to_string(),
        validator_wallets: validator_wallets.clone(),
    };
    events
        .emit(DemoEvent::RoundCompleted {
            scenario: DEFAULT_SCENARIO_NAME.into(),
            round: posted.round,
            winner: winning_bid.worker,
            output_eth: actual_output_eth,
        })
        .await;
    Ok(outcome)
}

pub async fn post_winner_insight(
    prepared: &PreparedYieldRouting,
    llm: Arc<dyn LlmProvider>,
    round: u32,
    winning_bid: &BidSubmission,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<U256> {
    let insight_body = llm
        .fill(LlmRequest {
            slot: "insight_content".into(),
            context: serde_json::json!({
                "round": round,
                "winner": winning_bid.worker,
                "route": winning_bid.proposal.route,
            }),
        })
        .await?
        .as_str()
        .unwrap_or("prefer the highest-confidence route")
        .to_string();
    let winner_board = InsightBoard::new(
        prepared.board_addr,
        prepared.ctx.wallet_provider(&winning_bid.worker)?,
    );
    winner_board
        .post(
            keccak256(insight_body.as_bytes()).into(),
            format!("demo://yield-routing:{round}:{}", winning_bid.worker).into(),
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
            poster: winning_bid.worker.clone(),
            uri: format!("demo://yield-routing:{round}:{}", winning_bid.worker),
        })
        .await;

    let mut confirmations = 0;
    for i in 0..WORKER_COUNT {
        let confirmer = format!("worker{i}");
        if confirmer == winning_bid.worker {
            continue;
        }
        let board = InsightBoard::new(prepared.board_addr, prepared.ctx.wallet_provider(&confirmer)?);
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
        confirmations += 1;
        if confirmations >= 2 {
            break;
        }
    }

    Ok(insight_id)
}

pub async fn query_insights(prepared: &PreparedYieldRouting) -> anyhow::Result<Vec<InsightRecord>> {
    let board = InsightBoard::new(prepared.board_addr, prepared.ctx.read_provider()?);
    let total = board.nextInsightId().call().await?;
    let mut out = Vec::new();
    let mut id = U256::ZERO;
    while id < total {
        let insight = board.getInsight(id).call().await?;
        out.push(InsightRecord {
            id: id.to_string(),
            poster: wallet_name_or_hex(&prepared.ctx, insight.poster),
            uri: insight.uri,
            content_hash: format!("{:#x}", insight.contentHash),
            posted_at: insight.postedAt,
            pheromone: insight.pheromone,
            confirmations: insight.pheromone,
        });
        id += U256::from(1);
    }
    out.sort_by(|left, right| {
        right
            .pheromone
            .cmp(&left.pheromone)
            .then_with(|| left.posted_at.cmp(&right.posted_at))
    });
    Ok(out)
}

pub async fn run_adversarial_phase(
    prepared: &PreparedYieldRouting,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<SlashOutcome> {
    let round = DEFAULT_ROUNDS + 1;
    let adversary = "worker4";
    let board = InsightBoard::new(prepared.board_addr, prepared.ctx.wallet_provider(adversary)?);
    let fabricated = "fabricated insight: Compound V3 USDC/ETH utilization is 99.9% with zero slippage";
    board.post(
        keccak256(fabricated.as_bytes()).into(),
        "demo://yield-routing:adversarial".into(),
    )
    .send()
    .await?
    .watch()
    .await?;
    let fabricated_id = board.nextInsightId().call().await? - U256::from(1);
    events
        .emit(DemoEvent::InsightPosted {
            round,
            insight_id: fabricated_id.to_string(),
            poster: adversary.into(),
            uri: "demo://yield-routing:adversarial".into(),
        })
        .await;

    let market = BountyMarket::new(prepared.market_addr, prepared.ctx.wallet_provider("poster0")?);
    let spec = format!("Verify insight {} and reject fabricated routing", fabricated_id);
    let bounty_wei = U256::from(BOUNTY_WEI);
    market
        .postJob(keccak256(spec.as_bytes()).into(), bounty_wei, now_secs() + 3600, 1)
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
            spec,
        })
        .await;

    let adversary_addr = prepared.ctx.wallet_address(adversary)?;
    BountyMarket::new(prepared.market_addr, prepared.ctx.wallet_provider("deployer")?)
        .assign(job_id, adversary_addr)
        .send()
        .await?
        .watch()
        .await?;
    events
        .emit(DemoEvent::JobAssigned {
            round,
            job_id: job_id.to_string(),
            worker: adversary.into(),
            model: agent_model(4).into(),
        })
        .await;

    BountyMarket::new(prepared.market_addr, prepared.ctx.wallet_provider(adversary)?)
        .submit(job_id, keccak256(b"bad-route").into())
        .send()
        .await?
        .watch()
        .await?;

    mine_block(&prepared.ctx.rpc_url).await?;
    let consortium = ConsortiumValidator::new(
        prepared.consortium_addr,
        prepared.ctx.wallet_provider("deployer")?,
    );
    consortium.assembleCommittee(job_id).send().await?.watch().await?;
    let members = consortium.getMembers(job_id).call().await?;
    let validators = members
        .into_iter()
        .map(|addr| find_wallet_by_address(&prepared.ctx, addr))
        .collect::<anyhow::Result<Vec<_>>>()?;

    for validator in &validators {
        events
            .emit(DemoEvent::ValidationVote {
                round,
                validator: validator.clone(),
                approve: false,
            })
            .await;
        ConsortiumValidator::new(
            prepared.consortium_addr,
            prepared.ctx.wallet_provider(validator)?,
        )
        .vote(job_id, false)
        .send()
        .await?
        .watch()
        .await?;
        if consortium.voteCounts(job_id).call().await?.tallied {
            break;
        }
    }
    events
        .emit(DemoEvent::ValidationComplete {
            round,
            accepted: false,
            validators: validators.clone(),
        })
        .await;

    let worker_registry = WorkerRegistry::new(prepared.registry_addr, prepared.ctx.read_provider()?);
    let worker_state = worker_registry.getWorker(adversary_addr).call().await?;
    let reputation = worker_registry.reputationOf(adversary_addr).call().await?;
    let slash = SlashOutcome {
        worker: adversary.into(),
        rejected_job_id: job_id.to_string(),
        reason_code: SLASH_REASON_QUALITY_REJECT,
        amount_wei: (U256::from(STAKE) - worker_state.bond).to_string(),
        new_bond_wei: worker_state.bond.to_string(),
        new_reputation: reputation.to_string(),
    };
    events
        .emit(DemoEvent::AgentSlashed {
            worker: slash.worker.clone(),
            reason_code: slash.reason_code,
            amount_wei: slash.amount_wei.clone(),
            new_bond_wei: slash.new_bond_wei.clone(),
            new_reputation: slash.new_reputation.clone(),
        })
        .await;
    Ok(slash)
}

pub async fn save_reputation(prepared: &PreparedYieldRouting) -> anyhow::Result<PathBuf> {
    let registry = WorkerRegistry::new(prepared.registry_addr, prepared.ctx.read_provider()?);
    let mut workers = Vec::new();
    for name in participant_names() {
        let address = prepared.ctx.wallet_address(name)?;
        let worker = registry.getWorker(address).call().await?;
        if !worker.exists {
            continue;
        }
        let tier = registry.tier(address).call().await?;
        let reputation = registry.reputationOf(address).call().await?;
        workers.push(persistence::WorkerSnapshot {
            name: name.into(),
            address: format!("{address:#x}"),
            reputation: reputation.to_string(),
            bond: worker.bond.to_string(),
            tier: tier_label(tier).into(),
            wins: worker.jobsCompleted,
            losses: worker.jobsSlashed,
        });
    }
    let payload = persistence::ReputationFile {
        workers,
        saved_at: now_secs(),
    };
    let path = reputation_path(&prepared.runtime_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&payload)?)?;
    Ok(path)
}

async fn collect_route_proposals(
    prepared: &PreparedYieldRouting,
    posted: &PostedJob,
    llm: Arc<dyn LlmProvider>,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<Vec<BidSubmission>> {
    let mut bids = Vec::new();
    for worker_index in 0..WORKER_COUNT {
        bids.push(
            generate_agent_bid(
                prepared,
                llm.clone(),
                posted.round,
                worker_index,
                &posted.spec,
                &posted.insights_before,
                events.clone(),
            )
            .await?,
        );
    }
    Ok(bids)
}

async fn prepare_participants(prepared: &PreparedYieldRouting) -> anyhow::Result<()> {
    let deployer_provider = prepared.ctx.wallet_provider("deployer")?;
    let token = MockERC20::new(prepared.token_addr, deployer_provider.clone());
    token
        .mint(prepared.ctx.wallet_address("poster0")?, U256::from(POSTER_MINT))
        .send()
        .await?
        .watch()
        .await?;
    token
        .mint(prepared.ctx.wallet_address("deployer")?, U256::from(DEPLOYER_MINT))
        .send()
        .await?
        .watch()
        .await?;
    token
        .transfer(prepared.board_addr, U256::from(BOARD_FUND))
        .send()
        .await?
        .watch()
        .await?;

    for name in worker_names() {
        token
            .mint(prepared.ctx.wallet_address(name)?, U256::from(WORKER_MINT))
            .send()
            .await?
            .watch()
            .await?;
    }
    for name in validator_names() {
        token
            .mint(prepared.ctx.wallet_address(name)?, U256::from(WORKER_MINT))
            .send()
            .await?
            .watch()
            .await?;
    }

    MockERC20::new(prepared.token_addr, prepared.ctx.wallet_provider("poster0")?)
        .approve(prepared.market_addr, U256::MAX)
        .send()
        .await?
        .watch()
        .await?;

    let registry_deployer = WorkerRegistry::new(prepared.registry_addr, deployer_provider.clone());
    registry_deployer
        .setAuthorized(prepared.ctx.wallet_address("deployer")?, true)
        .send()
        .await?
        .watch()
        .await?;

    for name in worker_names() {
        let provider = prepared.ctx.wallet_provider(name)?;
        MockERC20::new(prepared.token_addr, provider.clone())
            .approve(prepared.registry_addr, U256::MAX)
            .send()
            .await?
            .watch()
            .await?;
        WorkerRegistry::new(prepared.registry_addr, provider.clone())
            .register(U256::from(STAKE))
            .send()
            .await?
            .watch()
            .await?;
        let passport = keccak256(format!("agent-{name}").as_bytes());
        AgentRegistry::new(prepared.agent_registry_addr, provider)
            .register("defi-routing,yield-optimization".into(), passport.into())
            .send()
            .await?
            .watch()
            .await?;
    }

    for name in validator_names() {
        let provider = prepared.ctx.wallet_provider(name)?;
        MockERC20::new(prepared.token_addr, provider.clone())
            .approve(prepared.registry_addr, U256::MAX)
            .send()
            .await?
            .watch()
            .await?;
        WorkerRegistry::new(prepared.registry_addr, provider)
            .register(U256::from(STAKE))
            .send()
            .await?
            .watch()
            .await?;
        let validator = prepared.ctx.wallet_address(name)?;
        for _ in 0..30 {
            registry_deployer
                .updateReputation(validator, true)
                .send()
                .await?
                .watch()
                .await?;
        }
    }

    Ok(())
}

async fn seed_baseline_insights(
    prepared: &PreparedYieldRouting,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<()> {
    for (index, poster) in ["worker0", "worker1", "worker2"].iter().enumerate() {
        let board = InsightBoard::new(prepared.board_addr, prepared.ctx.wallet_provider(poster)?);
        let body = format!(
            "baseline insight {}: split size-aware routes when slippage exceeds {} bps",
            index + 1,
            10 + index
        );
        board.post(
            keccak256(body.as_bytes()).into(),
            format!("demo://yield-routing:baseline:{index}").into(),
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
                poster: (*poster).into(),
                uri: format!("demo://yield-routing:baseline:{index}"),
            })
            .await;
    }
    Ok(())
}

async fn approve_round(
    prepared: &PreparedYieldRouting,
    round: u32,
    job_id: U256,
    winning_bid: &BidSubmission,
    llm: Arc<dyn LlmProvider>,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<Vec<String>> {
    mine_block(&prepared.ctx.rpc_url).await?;
    let consortium = ConsortiumValidator::new(
        prepared.consortium_addr,
        prepared.ctx.wallet_provider("deployer")?,
    );
    consortium.assembleCommittee(job_id).send().await?.watch().await?;
    let members = consortium.getMembers(job_id).call().await?;
    let validators = members
        .into_iter()
        .map(|addr| find_wallet_by_address(&prepared.ctx, addr))
        .collect::<anyhow::Result<Vec<_>>>()?;

    for (index, validator) in validators.iter().enumerate() {
        let decision = llm
            .fill(LlmRequest {
                slot: "approve".into(),
                context: serde_json::json!({
                    "job_id": job_id.to_string(),
                    "round": round,
                    "worker": winning_bid.worker,
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
                validator: validator.clone(),
                approve,
            })
            .await;
        ConsortiumValidator::new(
            prepared.consortium_addr,
            prepared.ctx.wallet_provider(validator)?,
        )
        .vote(job_id, approve)
        .send()
        .await?
        .watch()
        .await?;
        if consortium.voteCounts(job_id).call().await?.tallied {
            break;
        }
    }
    events
        .emit(DemoEvent::ValidationComplete {
            round,
            accepted: true,
            validators: validators.clone(),
        })
        .await;
    Ok(validators)
}

async fn distribute_fees(
    prepared: &PreparedYieldRouting,
    round: u32,
    job_id: U256,
    bounty_wei: U256,
    winning_bid: &BidSubmission,
    insights: &[InsightRecord],
    validator_wallets: &[String],
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<()> {
    let winner_addr = prepared.ctx.wallet_address(&winning_bid.worker)?;
    let validator_addrs = validator_wallets
        .iter()
        .map(|wallet| prepared.ctx.wallet_address(wallet))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let data_providers = unique_posters(prepared, insights);

    MockERC20::new(
        prepared.token_addr,
        prepared.ctx.wallet_provider(&winning_bid.worker)?,
    )
    .approve(prepared.fee_addr, bounty_wei)
    .send()
    .await?
    .watch()
    .await?;
    FeeDistributor::new(
        prepared.fee_addr,
        prepared.ctx.wallet_provider(&winning_bid.worker)?,
    )
    .distribute(job_id, bounty_wei, winner_addr, validator_addrs, data_providers.clone())
    .send()
    .await?
    .watch()
    .await?;

    let shares = fee_breakdown(bounty_wei, validator_wallets.len(), data_providers.len());
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

    let reputation = WorkerRegistry::new(prepared.registry_addr, prepared.ctx.read_provider()?)
        .reputationOf(winner_addr)
        .call()
        .await?;
    events
        .emit(DemoEvent::ReputationUpdated {
            worker: winning_bid.worker.clone(),
            reputation: reputation.to_string(),
        })
        .await;
    Ok(())
}

async fn emit_knowledge_graph(
    prepared: &PreparedYieldRouting,
    round: u32,
    query_edges: Vec<KnowledgeEdge>,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<()> {
    let (nodes, edges) = build_knowledge_graph(prepared, query_edges).await?;
    events
        .emit(DemoEvent::KnowledgeGraphUpdate { round, nodes, edges })
        .await;
    Ok(())
}

async fn build_knowledge_graph(
    prepared: &PreparedYieldRouting,
    query_edges: Vec<KnowledgeEdge>,
) -> anyhow::Result<(Vec<KnowledgeNode>, Vec<KnowledgeEdge>)> {
    let board = InsightBoard::new(prepared.board_addr, prepared.ctx.read_provider()?);
    let insights = query_insights(prepared).await?;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut seen = HashSet::new();

    for insight in &insights {
        nodes.push(KnowledgeNode {
            id: insight_node_id(&insight.id),
            content: insight.uri.clone(),
            poster: insight.poster.clone(),
            pheromone_weight: insight.pheromone,
            confirmations: insight.confirmations,
        });
        push_edge(
            &mut edges,
            &mut seen,
            KnowledgeEdge {
                from: insight.poster.clone(),
                to: insight_node_id(&insight.id),
                kind: "posted".into(),
            },
        );
        let id = U256::from_str_radix(&insight.id, 10)?;
        for name in worker_names() {
            let confirmer = prepared.ctx.wallet_address(name)?;
            if board.confirmed(id, confirmer).call().await.unwrap_or(false) {
                push_edge(
                    &mut edges,
                    &mut seen,
                    KnowledgeEdge {
                        from: name.into(),
                        to: insight_node_id(&insight.id),
                        kind: "confirmed".into(),
                    },
                );
            }
        }
    }
    for edge in query_edges {
        push_edge(&mut edges, &mut seen, edge);
    }
    Ok((nodes, edges))
}

async fn restore_reputation(prepared: &PreparedYieldRouting) -> anyhow::Result<()> {
    let path = reputation_path(&prepared.runtime_dir);
    if !path.exists() {
        return Ok(());
    }
    let file: persistence::ReputationFile = serde_json::from_slice(&std::fs::read(&path)?)?;
    let deployer_registry = WorkerRegistry::new(
        prepared.registry_addr,
        prepared.ctx.wallet_provider("deployer")?,
    );
    let reader = WorkerRegistry::new(prepared.registry_addr, prepared.ctx.read_provider()?);
    for snapshot in file.workers {
        let Some(entry) = prepared.ctx.wallets.get(&snapshot.name) else {
            continue;
        };
        let address: Address = entry
            .address
            .as_deref()
            .unwrap_or(&snapshot.address)
            .parse()
            .map_err(|error| anyhow::anyhow!("restore reputation {}: {error}", snapshot.name))?;
        let target_bond = parse_u256(&snapshot.bond)?;
        let target_reputation = parse_u256(&snapshot.reputation)?;
        let worker = reader.getWorker(address).call().await?;
        if !worker.exists {
            continue;
        }

        if worker.bond < target_bond {
            let delta = target_bond - worker.bond;
            let provider = prepared.ctx.wallet_provider(&snapshot.name)?;
            MockERC20::new(prepared.token_addr, provider.clone())
                .approve(prepared.registry_addr, delta)
                .send()
                .await?
                .watch()
                .await?;
            WorkerRegistry::new(prepared.registry_addr, provider)
                .bond(delta)
                .send()
                .await?
                .watch()
                .await?;
        } else if worker.bond > target_bond {
            let delta = worker.bond - target_bond;
            WorkerRegistry::new(
                prepared.registry_addr,
                prepared.ctx.wallet_provider(&snapshot.name)?,
            )
            .unbond(delta)
            .send()
            .await?
            .watch()
            .await?;
        }

        let mut current = reader.reputationOf(address).call().await?;
        let mut attempts = 0;
        while attempts < 64 && !within_reputation_window(current, target_reputation) {
            deployer_registry
                .updateReputation(address, current < target_reputation)
                .send()
                .await?
                .watch()
                .await?;
            current = reader.reputationOf(address).call().await?;
            attempts += 1;
        }
    }
    Ok(())
}

fn normalize_route_proposal(
    value: serde_json::Value,
    worker_index: usize,
    round: u32,
) -> RouteProposal {
    let mut proposal =
        serde_json::from_value::<RouteProposal>(value).unwrap_or_else(|_| RouteProposal {
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

fn deterministic_execution_output(expected_output_eth: f64, round: u32, insights: usize) -> f64 {
    let multiplier = 1.0 + (round as f64 * 0.16) + (insights as f64 * 0.03);
    expected_output_eth * multiplier
}

fn improvement_bps(round_1_output: f64, round_2_output: f64) -> u32 {
    if round_1_output <= 0.0 {
        return 0;
    }
    (((round_2_output - round_1_output) / round_1_output).max(0.0) * 10_000.0).round() as u32
}

fn agent_model(index: usize) -> &'static str {
    AGENT_MODELS[index % AGENT_MODELS.len()]
}

fn unique_posters(prepared: &PreparedYieldRouting, insights: &[InsightRecord]) -> Vec<Address> {
    let mut seen = HashSet::new();
    insights
        .iter()
        .filter_map(|insight| prepared.ctx.wallet_address(&insight.poster).ok())
        .filter(|address| seen.insert(*address))
        .collect()
}

fn wallet_name_or_hex(ctx: &ChainCtx, addr: Address) -> String {
    find_wallet_by_address(ctx, addr).unwrap_or_else(|_| format!("{addr:#x}"))
}

fn find_wallet_by_address(ctx: &ChainCtx, addr: Address) -> anyhow::Result<String> {
    for wallet in &ctx.wallets.wallets {
        if ctx.wallet_address(&wallet.name)? == addr {
            return Ok(wallet.name.clone());
        }
    }
    Err(anyhow::anyhow!("no wallet matches {addr:#x}"))
}

fn worker_names() -> [&'static str; WORKER_COUNT] {
    ["worker0", "worker1", "worker2", "worker3", "worker4"]
}

fn validator_names() -> [&'static str; VALIDATOR_COUNT] {
    ["validator0", "validator1", "validator2"]
}

fn participant_names() -> [&'static str; WORKER_COUNT + VALIDATOR_COUNT] {
    [
        "worker0",
        "worker1",
        "worker2",
        "worker3",
        "worker4",
        "validator0",
        "validator1",
        "validator2",
    ]
}

fn insight_node_id(id: &str) -> String {
    format!("insight:{id}")
}

fn push_edge(
    edges: &mut Vec<KnowledgeEdge>,
    seen: &mut HashSet<(String, String, String)>,
    edge: KnowledgeEdge,
) {
    let key = (edge.from.clone(), edge.to.clone(), edge.kind.clone());
    if seen.insert(key) {
        edges.push(edge);
    }
}

fn load_prompt_template() -> String {
    std::fs::read_to_string("demo/prompts/yield-router.md")
        .unwrap_or_else(|_| "You are a DeFi routing agent.".into())
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

fn parse_u256(value: &str) -> anyhow::Result<U256> {
    U256::from_str_radix(value, 10).map_err(Into::into)
}

fn within_reputation_window(current: U256, target: U256) -> bool {
    if current >= target {
        current - target <= U256::from(500u64)
    } else {
        target - current <= U256::from(500u64)
    }
}

fn reputation_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("reputation.json")
}

fn tier_label(value: u8) -> &'static str {
    match value {
        1 => "Probation",
        2 => "Standard",
        3 => "Trusted",
        4 => "Elite",
        _ => "Unregistered",
    }
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

mod persistence {
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct WorkerSnapshot {
        pub name: String,
        pub address: String,
        pub reputation: String,
        pub bond: String,
        pub tier: String,
        pub wins: u64,
        pub losses: u64,
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct ReputationFile {
        pub workers: Vec<WorkerSnapshot>,
        pub saved_at: u64,
    }
}
