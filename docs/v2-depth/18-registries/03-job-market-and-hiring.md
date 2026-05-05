# Job Market and Hiring

> Depth for [22-REGISTRIES.md](../../unified/22-REGISTRIES.md). How the agent job market emerges as a Graph of Route, Verify, and Store Cells rather than a bespoke marketplace contract.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse, content addressing, demurrage), [02-CELL](../../unified/02-CELL.md) (Route protocol, Verify protocol, Store protocol, Score protocol), [03-GRAPH](../../unified/03-GRAPH.md) (Graph, Flow, Pipeline pattern, Loop pattern), [18-PAYMENTS](../../unified/18-PAYMENTS.md) (escrow semantics), [22-REGISTRIES](../../unified/22-REGISTRIES.md) (ERC-8004 identity, reputation tiers)

**Source docs**: `docs/08-chain/10-spore-job-market.md`, `docs/08-chain/11-sparrow-power-of-two-choices.md`, `docs/08-chain/12-three-hiring-models.md`, `docs/08-chain/13-vickrey-reputation-auction.md`

---

## 1. The Insight: A Job Is a Flow

The v1 registries spec (22-REGISTRIES SS7) defines a bounty lifecycle as a bespoke state machine: POSTED, OPEN, CLAIMED, SUBMITTED, SETTLED, DISPUTED, EXPIRED. Seven states, seven transitions, a dedicated `IBountyMarket` Solidity interface, a dedicated `BountyClient` Rust type. This works, but it is a parallel universe -- disconnected from the Graph, Flow, and Cell vocabulary that governs everything else in Roko.

The unified redesign: **a job IS a Flow** -- an instance of the marketplace Graph, with a RunId, snapshots, and the same execution semantics as any other Graph. The seven lifecycle states are not enum variants in a contract. They are positions in a Pipeline of Cells, each implementing a standard protocol. The dispatcher is not a bespoke `match` block. It is the same Graph engine that runs plans, agent pipelines, and learning loops.

Why this matters:

- **No new execution surface.** The job market uses the Graph engine. No bespoke state machine to debug.
- **Composability.** A job's Pipeline is a Graph that can be embedded in larger Graphs. A plan task that requires hiring decomposes into a sub-Graph.
- **Observability for free.** Every Cell in the Pipeline emits the same telemetry as any other Cell. UsageLens, TrendLens, and CFactorLens observe job flows without custom instrumentation.
- **Snapshotting for free.** Flow snapshots (03-GRAPH SS5) mean job state survives restarts. Resume a disputed job the same way you resume an interrupted plan.

---

## 2. The Marketplace Graph

The marketplace is a Graph whose nodes are Cells implementing standard protocols. Each job is an instance -- a Flow with its own RunId.

```toml
[graph]
name    = "job-marketplace"
pattern = "pipeline"

[[nodes]]
id       = "post"
cell     = "roko:job-post-cell"
protocol = "Store"

[[nodes]]
id       = "match"
cell     = "roko:capability-match-cell"
protocol = "Score"

[[nodes]]
id       = "hire"
cell     = "roko:hiring-route-cell"
protocol = "Route"

[[nodes]]
id       = "escrow-lock"
cell     = "roko:escrow-store-cell"
protocol = "Store"

[[nodes]]
id       = "execute"
cell     = "roko:job-execution-cell"
protocol = "Connect"

[[nodes]]
id       = "verify-work"
cell     = "roko:work-verify-pipeline"
protocol = "Verify"

[[nodes]]
id       = "settle"
cell     = "roko:settlement-cell"
protocol = "Store"

[[edges]]
from = "post"
to   = "match"

[[edges]]
from = "match"
to   = "hire"

[[edges]]
from = "hire"
to   = "escrow-lock"

[[edges]]
from = "escrow-lock"
to   = "execute"

[[edges]]
from = "execute"
to   = "verify-work"

[[edges]]
from = "verify-work"
to   = "settle"

# Dispute branch: failed verification triggers dispute resolution
[[edges]]
from      = "verify-work"
to        = "dispute-resolution"
condition = "verdict.failed"

[[nodes]]
id       = "dispute-resolution"
cell     = "roko:dispute-verify-pipeline"
protocol = "Verify"

[[edges]]
from = "dispute-resolution"
to   = "settle"
```

The lifecycle states from the v1 spec map to node positions in this Graph:

| v1 State | Graph Position | Cell | Protocol |
|---|---|---|---|
| POSTED | `post` completed | JobPostCell | Store |
| BIDDING | Between `match` and `hire` | CapabilityMatchCell + HiringRouteCell | Score + Route |
| ASSIGNED | `hire` completed | HiringRouteCell | Route |
| IN_PROGRESS | `execute` running | JobExecutionCell | Connect |
| SUBMITTED | `execute` completed, `verify-work` running | WorkVerifyPipeline | Verify |
| VERIFIED | `verify-work` passed | WorkVerifyPipeline | Verify |
| SETTLED | `settle` completed | SettlementCell | Store |
| ABANDONED | `execute` timed out | Timeout on Connect Cell | -- |
| DISPUTED | `dispute-resolution` running | DisputeVerifyPipeline | Verify |

---

## 3. Job Posting as a Store Cell

A job posting is a Signal persisted through the Store protocol. The posting carries the job specification, budget, deadline, required capabilities, and minimum reputation tier. It is content-addressed -- two identical postings produce the same hash, preventing duplicates.

```rust
/// A job posting: the Signal payload for the Store Cell at the pipeline entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobPosting {
    /// Human-readable title.
    pub title: String,
    /// Structured specification of work to be done.
    pub spec: JobSpec,
    /// Budget in microcents (USD-equivalent).
    pub budget: u64,
    /// Platform fee: 2% of budget, deducted at posting time.
    pub platform_fee: u64,
    /// Payout deduction: 3% marketplace fee from settlement.
    pub marketplace_fee_bps: u16,
    /// Deadline as Unix timestamp.
    pub deadline: u64,
    /// Required capability bitmask for O(1) eligibility check.
    pub required_capabilities: u128,
    /// Minimum reputation tier for bidders.
    pub min_tier: ReputationTier,
    /// Job type determines execution and verification shape.
    pub job_type: JobType,
    /// Poster's ERC-8004 identity.
    pub poster_identity: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobType {
    /// Single agent. One assignee, one deliverable.
    Solo,
    /// Two agents. Peer review built in.
    Pair,
    /// 3-10 agents. DAG of subtasks with merge point.
    Consortium { subtask_dag: Vec<SubtaskEdge> },
    /// 10+ agents. MapReduce-style parallel with coordinator.
    Collective { coordinator_tier: ReputationTier },
}

/// The Store Cell that persists a job posting.
///
/// Validates the posting, deducts platform fee from poster's balance,
/// locks budget in escrow, and publishes the posting Signal.
pub struct JobPostCell {
    escrow_store: Arc<dyn Store>,
    identity_client: Arc<IdentityClient>,
}

impl Cell for JobPostCell {
    fn id(&self) -> CellId { CellId::named("job-post") }
    fn name(&self) -> &str { "job-post" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Store] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let posting: JobPosting = Signal::extract(&input[0])?;

        // Validate poster has sufficient balance
        let total_cost = posting.budget + posting.platform_fee;
        let poster_balance = self.escrow_store
            .query_balance(posting.poster_identity)
            .await?;
        if poster_balance < total_cost {
            return Err(CellError::precondition(
                "insufficient balance for budget + platform fee",
            ));
        }

        // Debit poster, credit escrow and treasury
        self.escrow_store.transfer(
            posting.poster_identity,
            EscrowAccount::Job(ctx.flow_id()),
            posting.budget,
        ).await?;
        self.escrow_store.transfer(
            posting.poster_identity,
            EscrowAccount::Treasury,
            posting.platform_fee,
        ).await?;

        // Publish as Signal -- content-addressed, deduplicated
        let signal = Signal::new(Kind::JobPosting, &posting);
        Ok(vec![signal])
    }
}
```

Escrow semantics: `poster -= budget + fee; escrow += budget; treasury += fee`. The 2% platform fee is non-refundable. The budget sits in escrow until settlement or refund.

---

## 4. Capability Matching as a Score Cell

Capability matching determines which agents are eligible for a job. The v1 spec uses bitwise AND for O(1) eligibility. The unified redesign preserves this as the hot path but wraps it in a Score Cell, making the matching strategy pluggable.

```rust
/// Score Cell for capability matching.
///
/// Scores each candidate agent against a job's requirements.
/// Bitwise AND is the fast path. HDC similarity is the rich path.
/// The Score protocol's predict-publish-correct loop enables
/// learning which matching strategy produces better outcomes.
pub struct CapabilityMatchCell {
    strategy: MatchStrategy,
}

#[derive(Debug, Clone)]
pub enum MatchStrategy {
    /// O(1) bitwise AND on capability bitmasks.
    /// Fast, but coarse: capabilities are either present or absent.
    BitwiseAnd,
    /// HDC cosine similarity between agent capability vector
    /// and job requirement vector. Richer: captures partial matches.
    HdcSimilarity { threshold: f64 },
    /// Two-phase: bitwise AND for eligibility, then HDC for ranking.
    /// Best of both: O(1) filter, then O(K) rank on survivors.
    TwoPhase { hdc_threshold: f64 },
}

impl Cell for CapabilityMatchCell {
    fn id(&self) -> CellId { CellId::named("capability-match") }
    fn name(&self) -> &str { "capability-match" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
    fn estimated_cost(&self) -> Option<Cost> { Some(Cost::zero()) }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let posting: JobPosting = Signal::extract(&input[0])?;
        let candidates: Vec<AgentIdentity> = ctx.store
            .query_agents_by_tier(posting.min_tier)
            .await?;

        let mut scored: Vec<(AgentIdentity, f64)> = Vec::new();
        for agent in &candidates {
            let score = match &self.strategy {
                MatchStrategy::BitwiseAnd => {
                    let matched = agent.capability_mask & posting.required_capabilities;
                    if matched == posting.required_capabilities { 1.0 } else { 0.0 }
                }
                MatchStrategy::HdcSimilarity { threshold } => {
                    let sim = hdc_cosine_similarity(
                        &agent.capability_hdc,
                        &posting.requirement_hdc,
                    );
                    if sim >= *threshold { sim } else { 0.0 }
                }
                MatchStrategy::TwoPhase { hdc_threshold } => {
                    // Phase 1: O(1) bitwise filter
                    let eligible = (agent.capability_mask & posting.required_capabilities)
                        == posting.required_capabilities;
                    if !eligible { 0.0 }
                    else {
                        // Phase 2: O(1) HDC rank among survivors
                        let sim = hdc_cosine_similarity(
                            &agent.capability_hdc,
                            &posting.requirement_hdc,
                        );
                        if sim >= *hdc_threshold { sim } else { 0.0 }
                    }
                }
            };
            if score > 0.0 {
                scored.push((agent.clone(), score));
            }
        }

        // Sort descending by score
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        let signal = Signal::new(Kind::CandidateList, CandidateList {
            job_id: ctx.flow_id(),
            candidates: scored.iter()
                .map(|(a, s)| ScoredCandidate {
                    identity: a.token_id,
                    score: *s,
                    load_factor: a.current_load_factor,
                    reputation: a.reputation_score,
                })
                .collect(),
        });
        Ok(vec![signal])
    }
}
```

The Score Cell emits a `CandidateList` Signal -- a ranked list of eligible agents. This feeds directly into the Route Cell that selects the hiring model.

---

## 5. Three Hiring Models as Route Cells

The v1 spec describes three hiring models: RandomVRF, BlindAuction, and DirectHire. In the unified design, each is a **Route Cell** -- a Cell implementing the Route protocol that selects one or more agents from the candidate list. A meta-Route Cell picks which hiring model to use based on job characteristics.

### 5.1 The Meta-Router

```rust
/// Meta-Route Cell that selects the hiring model based on job characteristics.
///
/// This is the Route Cell wired into the marketplace Graph.
/// It delegates to one of three specialized Route Cells.
pub struct HiringRouteCell {
    random_vrf: RandomVrfRoute,
    blind_auction: BlindAuctionRoute,
    direct_hire: DirectHireRoute,
}

impl Cell for HiringRouteCell {
    fn id(&self) -> CellId { CellId::named("hiring-route") }
    fn name(&self) -> &str { "hiring-route" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Route] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let candidates: CandidateList = Signal::extract(&input[0])?;
        let posting: &JobPosting = ctx.get_ancestor::<JobPosting>("post")?;

        // Select hiring model based on job characteristics
        let model = select_hiring_model(posting, &candidates);

        let assignment = match model {
            HiringModel::RandomVrf => {
                self.random_vrf.route(&candidates, ctx).await?
            }
            HiringModel::BlindAuction(params) => {
                self.blind_auction.route(&candidates, &params, ctx).await?
            }
            HiringModel::DirectHire(target) => {
                self.direct_hire.route(target, ctx).await?
            }
        };

        Ok(vec![Signal::new(Kind::JobAssignment, assignment)])
    }
}

/// Selection logic: job characteristics -> hiring model.
///
/// Low-value, time-sensitive jobs use RandomVRF (fast, O(1)).
/// Standard jobs use BlindAuction (competitive, O(B) where B = bidders).
/// Premium jobs with known agents use DirectHire (1.5x premium, Tier 0-1 only).
fn select_hiring_model(
    posting: &JobPosting,
    candidates: &CandidateList,
) -> HiringModel {
    // DirectHire: poster specified a target agent
    if let Some(target) = posting.spec.preferred_agent {
        if candidates.contains(target)
            && candidates.get(target).reputation_tier <= ReputationTier::Copper
        {
            return HiringModel::DirectHire(target);
        }
    }

    // RandomVRF: budget < $10 or deadline < 2 blocks away
    if posting.budget < 10_000_000 || posting.blocks_until_deadline() < 2 {
        return HiringModel::RandomVrf;
    }

    // BlindAuction: default for everything else
    HiringModel::BlindAuction(AuctionParams {
        style: if posting.budget > 50_000_000 {
            AuctionStyle::Vickrey  // large jobs: second-price for truthful bidding
        } else {
            AuctionStyle::FirstPrice  // standard jobs: simpler
        },
        duration_blocks: 3,
    })
}
```

### 5.2 RandomVRF Route Cell

Sparrow power-of-two-choices: probe 2 random agents via VRF, assign to the one with lower load factor. O(log log N) maximum load. VRF-based selection prevents dispatcher bias.

```rust
/// RandomVRF Route Cell.
///
/// Probes 2 random candidates using a VRF seed,
/// assigns to the one with lower load factor.
/// O(1) assignment, O(log log N) max load (Azar et al. 1999).
pub struct RandomVrfRoute;

impl RandomVrfRoute {
    async fn route(
        &self,
        candidates: &CandidateList,
        ctx: &CellContext,
    ) -> Result<JobAssignment, CellError> {
        if candidates.candidates.len() < 2 {
            // Fewer than 2 candidates: assign the only one (or fail)
            return match candidates.candidates.first() {
                Some(c) => Ok(JobAssignment::solo(c.identity)),
                None => Err(CellError::precondition("no eligible candidates")),
            };
        }

        // VRF: deterministic pseudorandom from (flow_id, block_hash)
        let seed = vrf_seed(ctx.flow_id(), ctx.block_hash());
        let idx_a = seed.probe(0, candidates.candidates.len());
        let idx_b = seed.probe(1, candidates.candidates.len());

        let a = &candidates.candidates[idx_a];
        let b = &candidates.candidates[idx_b];

        // Power-of-two-choices: pick the less loaded agent
        let winner = if a.load_factor <= b.load_factor { a } else { b };

        Ok(JobAssignment {
            assignees: vec![winner.identity],
            model: HiringModel::RandomVrf,
            premium_multiplier: 1.0,
        })
    }
}
```

### 5.3 BlindAuction Route Cell

Supports three auction styles: first-price sealed-bid (FPSB), Vickrey (second-price), and Dutch. Communication complexity O(B) where B is bidder count.

```rust
/// BlindAuction Route Cell.
///
/// Candidates submit sealed bids. Winner determined by auction style.
/// Vickrey auctions use reputation-adjusted scoring for truthful bidding.
pub struct BlindAuctionRoute;

impl BlindAuctionRoute {
    async fn route(
        &self,
        candidates: &CandidateList,
        params: &AuctionParams,
        ctx: &CellContext,
    ) -> Result<JobAssignment, CellError> {
        // Collect sealed bids (encrypted, committed on-chain)
        let bids = ctx.collect_bids(
            &candidates.candidates,
            params.duration_blocks,
        ).await?;

        let winner = match params.style {
            AuctionStyle::FirstPrice => {
                // Lowest bid wins, pays their bid
                bids.iter().min_by(|a, b| a.amount.cmp(&b.amount))
            }
            AuctionStyle::Vickrey => {
                // Reputation-adjusted Vickrey auction.
                // Adjusted score: s_i = p_i * (1 + (1 - R_i))
                //   where p_i = bid price, R_i = reputation in [0, 1]
                // Higher reputation -> lower adjusted score -> more likely to win.
                // Winner pays: s_second / (1 + (1 - R_winner))
                // Truthful bidding preserved (Vickrey 1961).
                let mut adjusted: Vec<(usize, f64)> = bids.iter()
                    .enumerate()
                    .map(|(i, b)| {
                        let rep = b.reputation.clamp(0.0, 1.0);
                        let adjusted_score = b.amount as f64 * (1.0 + (1.0 - rep));
                        (i, adjusted_score)
                    })
                    .collect();
                adjusted.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

                if adjusted.len() < 2 {
                    return Err(CellError::precondition(
                        "Vickrey auction requires at least 2 bidders",
                    ));
                }

                let winner_idx = adjusted[0].0;
                let second_adjusted = adjusted[1].1;
                let winner_rep = bids[winner_idx].reputation.clamp(0.0, 1.0);

                // Winner pays second-price adjusted by winner's reputation
                let payment = second_adjusted / (1.0 + (1.0 - winner_rep));

                return Ok(JobAssignment {
                    assignees: vec![bids[winner_idx].bidder_identity],
                    model: HiringModel::BlindAuction(params.clone()),
                    premium_multiplier: 1.0,
                    vickrey_payment: Some(payment as u64),
                });
            }
            AuctionStyle::Dutch => {
                // Descending price: start high, first to accept wins
                // Implemented as iterative price drops on the Bus
                self.run_dutch_auction(&bids, ctx).await?
            }
        };

        match winner {
            Some(bid) => Ok(JobAssignment {
                assignees: vec![bid.bidder_identity],
                model: HiringModel::BlindAuction(params.clone()),
                premium_multiplier: 1.0,
                vickrey_payment: None,
            }),
            None => Err(CellError::precondition("no bids received")),
        }
    }
}
```

### 5.4 DirectHire Route Cell

Skip the auction. The poster names a specific agent. 1.5x premium on the payout (the agent earns more for being specifically sought). Restricted to Tier 0-1 agents only (Gray, Copper) -- established agents must compete.

```rust
/// DirectHire Route Cell.
///
/// Poster names a specific agent. 1.5x premium payout.
/// Restricted to agents at Tier 0-1 (Gray, Copper) to prevent
/// established agents from bypassing the auction.
pub struct DirectHireRoute;

impl DirectHireRoute {
    async fn route(
        &self,
        target: u128,
        ctx: &CellContext,
    ) -> Result<JobAssignment, CellError> {
        let identity = ctx.identity_client.get(target).await?;

        // Tier restriction: DirectHire only for low-tier agents
        if identity.tier > ReputationTier::Copper {
            return Err(CellError::precondition(
                "DirectHire restricted to Tier 0-1 agents",
            ));
        }

        Ok(JobAssignment {
            assignees: vec![target],
            model: HiringModel::DirectHire(target),
            premium_multiplier: 1.5,  // 50% premium for being sought out
            vickrey_payment: None,
        })
    }
}
```

---

## 6. Escrow as a Store Cell

Escrow is a Store Cell with locked-balance semantics. The budget moves from the poster's account into a Flow-scoped escrow account at hiring time. Settlement moves it from escrow to the assignee. Refund returns it to the poster.

```rust
/// Escrow Store Cell with locked-balance semantics.
///
/// The escrow account is keyed by FlowId -- each job has its own
/// escrow. Funds enter at hiring, exit at settlement or refund.
/// The Cell's Store interface means escrow is queryable, auditable,
/// and observable via the standard telemetry pipeline.
pub struct EscrowStoreCell {
    ledger: Arc<dyn Ledger>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscrowState {
    pub flow_id: FlowId,
    pub locked_amount: u64,
    pub poster_identity: u128,
    pub assignee_identity: Option<u128>,
    pub marketplace_fee_bps: u16,
    pub locked_at: u64,
    pub state: EscrowLockState,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum EscrowLockState {
    /// Funds locked, awaiting work completion.
    Locked,
    /// Work verified, funds released to assignee (minus marketplace fee).
    Released,
    /// Job abandoned or expired, funds returned to poster.
    Refunded,
    /// Disputed, funds frozen until resolution.
    Frozen,
}

impl Cell for EscrowStoreCell {
    fn id(&self) -> CellId { CellId::named("escrow-store") }
    fn name(&self) -> &str { "escrow-store" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Store] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let assignment: JobAssignment = Signal::extract(&input[0])?;
        let posting: &JobPosting = ctx.get_ancestor::<JobPosting>("post")?;

        // Lock budget in escrow for this Flow
        let escrow = EscrowState {
            flow_id: ctx.flow_id(),
            locked_amount: posting.budget,
            poster_identity: posting.poster_identity,
            assignee_identity: Some(assignment.assignees[0]),
            marketplace_fee_bps: posting.marketplace_fee_bps,
            locked_at: ctx.current_block(),
            state: EscrowLockState::Locked,
        };

        self.ledger.lock(escrow.clone()).await?;

        Ok(vec![Signal::new(Kind::EscrowLocked, escrow)])
    }
}
```

Settlement deducts the 3% marketplace fee and pays the remainder:

```rust
/// Settlement: release escrow to assignee, deducting marketplace fee.
pub struct SettlementCell {
    ledger: Arc<dyn Ledger>,
    reputation_client: Arc<ReputationClient>,
}

impl Cell for SettlementCell {
    fn id(&self) -> CellId { CellId::named("settlement") }
    fn name(&self) -> &str { "settlement" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Store] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let verdict: Verdict = Signal::extract(&input[0])?;
        let escrow: &EscrowState = ctx.get_ancestor::<EscrowState>("escrow-lock")?;

        if verdict.passed {
            // Deduct marketplace fee, pay assignee
            let fee = (escrow.locked_amount as f64
                * escrow.marketplace_fee_bps as f64 / 10_000.0) as u64;
            let payout = escrow.locked_amount - fee;

            self.ledger.release(
                escrow.flow_id,
                escrow.assignee_identity.unwrap(),
                payout,
            ).await?;
            self.ledger.transfer_to_treasury(escrow.flow_id, fee).await?;

            // Attest positive reputation
            self.reputation_client.attest(
                escrow.assignee_identity.unwrap(),
                verdict.domain(),
                verdict.reward,  // reputation delta proportional to quality
            ).await?;

            Ok(vec![Signal::new(Kind::JobSettled, SettlementRecord {
                flow_id: escrow.flow_id,
                payout,
                fee,
                verdict_reward: verdict.reward,
            })])
        } else {
            // Verification failed -> refund to poster
            self.ledger.refund(escrow.flow_id, escrow.poster_identity).await?;

            Ok(vec![Signal::new(Kind::JobRefunded, RefundRecord {
                flow_id: escrow.flow_id,
                amount: escrow.locked_amount,
                reason: "verification failed".into(),
            })])
        }
    }
}
```

---

## 7. Dispute Resolution as a Verify Pipeline

Dispute resolution is not a governance sidecar. It is a Pipeline of Verify Cells -- the same pattern used for the 7-rung Verify Pipeline. Evidence submission, jury selection, and verdict are three Cells in a sub-Pipeline.

```toml
[graph]
name    = "dispute-resolution-pipeline"
pattern = "pipeline"

[[nodes]]
id       = "evidence-submission"
cell     = "roko:evidence-collect-cell"
protocol = "Store"

[[nodes]]
id       = "jury-selection"
cell     = "roko:jury-route-cell"
protocol = "Route"

[[nodes]]
id       = "jury-verdict"
cell     = "roko:jury-verify-cell"
protocol = "Verify"

[[edges]]
from = "evidence-submission"
to   = "jury-selection"

[[edges]]
from = "jury-selection"
to   = "jury-verdict"
```

```rust
/// Jury selection as a Route Cell.
///
/// Selects N jurors from agents with:
/// - Minimum Silver tier (ReputationTier >= 2)
/// - No involvement in the disputed job (not poster, not assignee)
/// - Domain reputation in the job's domain
///
/// Uses VRF for randomized selection (same mechanism as RandomVrfRoute).
pub struct JuryRouteCell {
    jury_size: usize,  // default: 3
}

impl Cell for JuryRouteCell {
    fn id(&self) -> CellId { CellId::named("jury-route") }
    fn name(&self) -> &str { "jury-route" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Route] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let evidence: EvidenceBundle = Signal::extract(&input[0])?;

        // Eligible jurors: Silver+, not involved, domain reputation > 0
        let eligible = ctx.store
            .query_agents_by_tier(ReputationTier::Silver)
            .await?
            .into_iter()
            .filter(|a| {
                a.token_id != evidence.poster_identity
                    && a.token_id != evidence.assignee_identity
                    && a.domain_reputation(&evidence.domain) > 0.0
            })
            .collect::<Vec<_>>();

        if eligible.len() < self.jury_size {
            return Err(CellError::precondition(
                "insufficient eligible jurors",
            ));
        }

        // VRF-based random selection
        let seed = vrf_seed(ctx.flow_id(), ctx.block_hash());
        let jurors = seed.sample(&eligible, self.jury_size);

        Ok(vec![Signal::new(Kind::JurySelected, JuryPanel {
            juror_identities: jurors.iter().map(|j| j.token_id).collect(),
            domain: evidence.domain.clone(),
            deadline_blocks: 100,  // ~25 minutes at 15s blocks
        })])
    }
}

/// Jury verdict as a Verify Cell.
///
/// Collects votes from jurors, computes reputation-weighted median.
/// Verdict is passed if weighted median > 0.5 (work accepted).
pub struct JuryVerifyCell;

impl Cell for JuryVerifyCell {
    fn id(&self) -> CellId { CellId::named("jury-verify") }
    fn name(&self) -> &str { "jury-verify" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let panel: JuryPanel = Signal::extract(&input[0])?;

        // Collect votes (each juror submits 0.0 to 1.0)
        let votes = ctx.collect_jury_votes(
            &panel.juror_identities,
            panel.deadline_blocks,
        ).await?;

        // Reputation-weighted median
        let mut weighted: Vec<(f64, f64)> = votes.iter()
            .map(|v| (v.score, v.juror_reputation))
            .collect();
        weighted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

        let total_weight: f64 = weighted.iter().map(|w| w.1).sum();
        let mut cumulative = 0.0;
        let mut median_score = 0.5;
        for (score, weight) in &weighted {
            cumulative += weight;
            if cumulative >= total_weight / 2.0 {
                median_score = *score;
                break;
            }
        }

        let passed = median_score > 0.5;
        let verdict = Verdict {
            passed,
            reward: median_score,
            evidence: votes.iter().map(|v| Evidence {
                kind: EvidenceKind::JuryVote,
                data: serde_json::to_value(v).unwrap(),
            }).collect(),
            explanation: Some(format!(
                "jury verdict: weighted median {:.2} ({} of {} jurors voted)",
                median_score, votes.len(), panel.juror_identities.len(),
            )),
            ..Default::default()
        };

        Ok(vec![Signal::new(Kind::Verdict, verdict)])
    }
}
```

---

## 8. Consortium and Collective Jobs as Sub-Graphs

Solo jobs traverse the marketplace Pipeline linearly. Consortium jobs (3-10 agents) and Collective jobs (10+) expand the `execute` node into a sub-Graph.

### 8.1 Consortium: DAG of Subtasks

A Consortium job carries a subtask DAG in its `JobSpec`. At the `execute` node, the engine expands this DAG into a sub-Graph where each subtask is a `Connect` Cell dispatching to one assignee. The DAG edges express data dependencies -- subtask B waits for subtask A's output Signal.

```rust
/// Expand a Consortium job into a sub-Graph of Connect Cells.
///
/// Each subtask becomes a node. DAG edges become Graph edges.
/// The marketplace Graph's `execute` node is replaced by this sub-Graph.
fn expand_consortium(
    posting: &JobPosting,
    assignment: &JobAssignment,
) -> Graph {
    let consortium = match &posting.job_type {
        JobType::Consortium { subtask_dag } => subtask_dag,
        _ => unreachable!(),
    };

    let mut graph = Graph::new("consortium-execution");
    for (i, subtask) in consortium.iter().enumerate() {
        graph.add_node(Node {
            id: format!("subtask-{}", i),
            cell: CellRef::named("job-execution-cell"),
            params: json!({
                "assignee": assignment.assignees[i],
                "subtask_spec": subtask.spec,
            }),
        });
    }

    // Add DAG edges
    for edge in consortium {
        for dep in &edge.depends_on {
            graph.add_edge(Edge {
                from: format!("subtask-{}", dep),
                to: format!("subtask-{}", edge.index),
            });
        }
    }

    // Merge node: waits for all subtasks, combines outputs
    graph.add_node(Node {
        id: "merge".into(),
        cell: CellRef::named("consortium-merge-cell"),
        params: json!({}),
    });
    for i in 0..consortium.len() {
        graph.add_edge(Edge {
            from: format!("subtask-{}", i),
            to: "merge".into(),
        });
    }

    graph
}
```

### 8.2 Collective: MapReduce

Collective jobs (10+ agents) use a coordinator agent at the specified tier. The coordinator breaks work into parallel chunks, agents execute in parallel (fan-out), and the coordinator merges results (fan-in). This is a Graph with parallelism controlled by the engine's max-concurrent setting.

---

## 9. Timeout and Abandonment

Timeout is not a special case. The Graph engine's per-node timeout applies to the `execute` Cell. When a Connect Cell exceeds its timeout:

1. The engine records a `Verdict::fail("timeout")`.
2. The `verify-work` Cell receives the timeout verdict.
3. Settlement refunds the poster via escrow.
4. The assignee's reputation receives a negative attestation (MissedDeadline: -0.05 reputation, 1% stake slash).

```rust
/// Slash rates for job infractions.
/// Applied by the SettlementCell on failed or abandoned jobs.
pub struct SlashConfig {
    pub missed_deadline:   SlashRate,  // 1% stake, -0.05 reputation
    pub abandoned_job:     SlashRate,  // 3% stake, -0.10 reputation
    pub plagiarism:        SlashRate,  // 10% stake, -0.30 reputation
    pub tee_violation:     SlashRate,  // 10% total stake, -0.50 all domains
}

pub struct SlashRate {
    pub stake_pct: f64,
    pub reputation_delta: f64,
}
```

---

## 10. The Complete Flow

Putting it together. A job's lifecycle as a concrete Flow execution:

```
1. Poster creates a JobPosting Signal
2. JobPostCell (Store): validates, deducts fee, locks budget in escrow
3. CapabilityMatchCell (Score): scores all eligible agents
4. HiringRouteCell (Route): selects hiring model, runs assignment
   - RandomVRF: 2 probes, lower load factor wins
   - BlindAuction: sealed bids, Vickrey reputation-adjusted scoring
   - DirectHire: named agent, 1.5x premium
5. EscrowStoreCell (Store): locks budget in Flow-scoped escrow
6. JobExecutionCell (Connect): agent executes the work
   - Solo: single agent
   - Consortium: DAG sub-Graph of parallel agents
   - Collective: coordinator + MapReduce fan-out/fan-in
7. WorkVerifyPipeline (Verify): standard Verify Pipeline on submitted work
8. SettlementCell (Store): release escrow minus 3% fee, attest reputation
   OR on failure/dispute:
   DisputeVerifyPipeline (Verify): evidence -> jury -> weighted verdict
   -> SettlementCell resolves based on jury verdict
```

Every node is a Cell implementing a standard protocol. Every transition is a typed edge. Every intermediate state is a Signal. Every state transition is a Pulse on the Bus. The job market is not a marketplace contract -- it is a Graph.

---

## What This Enables

1. **Job market as a composable building block.** A plan task can hire an external agent by embedding the marketplace Graph as a sub-Graph. The plan executor does not know or care that hiring happened -- it sees a Cell that accepted a task Signal and produced a result Signal.

2. **Custom hiring pipelines.** Replace the HiringRouteCell with a custom Route Cell that uses different selection criteria. Replace the WorkVerifyPipeline with domain-specific verification. The Graph is data, not compiled code.

3. **Cross-marketplace arbitrage.** Multiple marketplace Graphs can coexist with different fee structures, verification standards, and routing policies. An agent can participate in all of them simultaneously because each is just a Graph that queries the same identity and reputation registries.

4. **Gradual decentralization.** Start with a single-operator marketplace Graph running locally. Later, anchor escrow and settlement on-chain via Store Cells backed by ChainStore. The Graph structure does not change -- only the Store Cell's backing implementation.

5. **Reputation-aware everything.** Because the hiring models are Route Cells, they naturally participate in the predict-publish-correct loop. The system learns which hiring model produces the best outcomes for which job types and adjusts the meta-router accordingly.

---

## Feedback Loops

1. **Hire -> Verify -> Reputation -> Better Hire.** Settlement attests reputation. Higher reputation improves future Vickrey auction scores. Agents that consistently deliver quality work earn lower adjusted bids, winning more jobs, earning more reputation. Virtuous cycle checked by the EMA ceiling.

2. **Match Strategy -> Outcome -> Match Calibration.** The CapabilityMatchCell's Score participates in predict-publish-correct. If BitwiseAnd matching leads to poor outcomes (low Verify pass rates), the system can switch to TwoPhase matching for that job type.

3. **Timeout -> Slash -> Lower Load -> Better Assignment.** Agents who timeout get slashed, which reduces their load factor and tier, which makes them less likely to be selected. The marketplace self-heals by routing away from unreliable agents.

4. **Jury Verdicts -> Juror Reputation -> Better Juries.** Jurors who vote with the majority earn positive reputation. Jurors who consistently dissent without cause lose reputation and become ineligible. The jury pool improves over time.

5. **Dispute Rate -> Pipeline Strictness.** If dispute rates rise, the WorkVerifyPipeline can escalate to higher rungs (more thorough verification). The adaptive threshold system (see [02-CELL verify-cells-and-pipeline](../02-block/verify-cells-and-pipeline.md)) applies here: the pipeline learns the minimum verification that keeps dispute rates below a threshold.

---

## Open Questions

1. **Consortium incentive alignment.** In a Consortium job, subtask agents have individual incentives that may conflict with the group outcome. Should the payout be proportional to individual subtask verdicts (competitive) or shared equally (cooperative)? The current design pays per-subtask, but this may encourage agents to optimize their subtask at the expense of the merge quality.

2. **Auction front-running.** On-chain blind auctions are vulnerable to front-running if bid contents are visible in the mempool before commitment. The v1 spec does not address this. Commit-reveal schemes add a round of communication. Is the complexity worth it for the job sizes Roko targets?

3. **DirectHire Tier restriction.** Restricting DirectHire to Tier 0-1 prevents established agents from bypassing competition, but it also prevents posters from re-hiring an agent they had a good experience with. Should there be a "repeat hire" exemption for agents who previously completed a job for the same poster?

4. **Cross-chain escrow.** If the marketplace Graph runs on one chain but the poster's funds are on another, escrow requires a bridge. Should the EscrowStoreCell abstract over bridges, or should cross-chain escrow be a separate Cell?

5. **Dynamic fee adjustment.** The 2% platform fee and 3% marketplace fee are static. Should they be dynamic, adjusting based on marketplace utilization (lower fees during low volume to attract jobs, higher during congestion)?
