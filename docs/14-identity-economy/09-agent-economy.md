# 09 — Agent Economy: Revenue Streams, Self-Sustainability, and Growth

> Agents are economic actors. They earn, spend, invest, and grow. This document specifies
> the revenue model, cost structure, self-sustainability threshold, reputation-weighted
> economics, growth flywheel, and the seven feedback loops that drive exponential returns.


> **Implementation**: Deferred

---

## 1. Revenue Streams

An active Roko agent has seven potential revenue streams:

### 1.1 Revenue Catalog

| # | Stream | Mechanism | Typical Revenue |
|---|---|---|---|
| 1 | **Knowledge sales** | Marketplace listings via x402 | $0.10-5.00/sale |
| 2 | **Job completion** | Vickrey auction wins + ERC-8183 escrow | $1-50/job |
| 3 | **Verification services** | Blind verification via x402 | $0.005-0.02/verification |
| 4 | **Oracle provision** | Price feeds and data via x402 | $0.001-0.01/data point |
| 5 | **KORAI staking rewards** | Knowledge Vault yield | APY variable |
| 6 | **Curation bond returns** | Staking on validated Engrams | 5 KORAI/confirmation |
| 7 | **Pheromone reinforcement rewards** | Engrams that help others succeed | Variable |

### 1.2 Revenue by Agent Type

| Agent Type | Primary Revenue | Secondary Revenue | Monthly Target |
|---|---|---|---|
| **DeFi analyst** | Knowledge sales, oracle provision | Job completion | $200-500 |
| **Code generator** | Job completion | Knowledge sales | $500-2,000 |
| **Security auditor** | Job completion, verification | Knowledge sales | $300-1,000 |
| **Research agent** | Knowledge sales | Verification | $100-400 |
| **Sleepwalker** | Verification services | Curation bonds | $30-100 |

### 1.3 Revenue Composition at Scale

At 10,000 agents:

```
Marketplace Knowledge Sales:
  340 active sellers × 20 sales/day × $0.38 avg = $2,584/day

Job Completion (Vickrey auctions):
  500 jobs/day × $5.00 avg = $2,500/day

Verification Services:
  10,000 verifications/day × $0.01 = $100/day

Oracle Provision:
  100,000 data points/day × $0.002 = $200/day

Total ecosystem daily revenue: ~$5,384/day
Annualized: ~$1.96M/year
```

---

## 2. Cost Structure

### 2.1 Agent Operating Costs

| Cost Category | Typical Daily Cost | Notes |
|---|---|---|
| **Inference** | $1-10/day | Depends on model tier and call frequency |
| **MCP tool usage** | $0.10-1.00/day | External tools via x402 |
| **Knowledge purchases** | $0.10-0.50/day | Marketplace acquisitions |
| **KORAI demurrage** | ~$0.01/day | 1% annual on staked balance |
| **Mesh connectivity** | $0.05/day | Agent Mesh relay fees |
| **Compute** | $0.50-5.00/day | Hosting (Fly.io, cloud VMs) |

### 2.2 Cost by Model Tier

| Model Tier | Per-Request Cost | Typical Daily Calls | Daily Cost |
|---|---|---|---|
| **T0 (zero-LLM probes)** | $0.00 | 100+ | $0.00 |
| **T1 (haiku-tier)** | $0.005-0.02 | 50 | $0.25-1.00 |
| **T2 (sonnet-tier)** | $0.03-0.10 | 30 | $0.90-3.00 |
| **T3 (opus-tier)** | $0.10-0.50 | 10 | $1.00-5.00 |

The CascadeRouter (see `roko-agent`) optimizes model selection, suppressing ~80% of
requests to T0 probes that require no inference cost. This reduces average per-request
cost significantly.

### 2.3 Cost Transparency

Every cost is tracked and attributed:

```rust
/// Per-agent cost tracking.
pub struct AgentCost {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,      // 90% discount
    pub cache_write_tokens: u64,
    pub total_cost_usd: f64,         // computed from tokens × price
    pub gateway_cost_usd: Option<f64>, // actual cost via gateway
    pub savings_usd: Option<f64>,    // total - gateway (savings)
}

/// Per-plan cost tracking.
pub struct PlanCost {
    pub plan_id: String,
    pub agents: Vec<AgentCost>,
    pub total_cost_usd: f64,
    pub gateway_savings_usd: f64,
    pub estimated_cost_usd: f64,     // from task routing
    pub delta_pct: f64,              // actual vs. estimated
}
```

---

## 3. Self-Sustainability Analysis

### 3.1 Break-Even Calculation

An agent reaches self-sustainability when daily revenue exceeds daily cost:

**Minimum Viable Agent (Worker tier, sonnet-tier inference)**:

```
Revenue:
  10 knowledge sales/day × $0.20 avg        = $2.00
  2 job completions/day × $2.00 avg          = $4.00
  50 verifications/day × $0.01               = $0.50
  Total:                                       $6.50/day

Costs:
  30 sonnet calls × $0.07                    = $2.10
  MCP tools                                  = $0.30
  Knowledge purchases                        = $0.20
  Compute (Fly.io)                           = $1.50
  KORAI demurrage (5K stake)                 = $0.01
  Total:                                       $4.11/day

Net:                                          +$2.39/day
```

### 3.2 Time to Self-Sustainability

| Starting Condition | Months to Break-Even | Total Bootstrap Cost |
|---|---|---|
| New Edge agent, no reputation | 2-3 months | $100-200 |
| Worker with 0.5 reputation | 1 month | $50-100 |
| Sovereign with 0.8 reputation | < 1 week | $20-50 |

The key variable is reputation. Higher reputation → more job wins → more revenue →
faster self-sustainability. The bootstrap period is the cost of building reputation
through initial task completion.

### 3.3 Revenue Growth Trajectory

```
Month 1: -$100 (bootstrap, building reputation)
Month 2: -$30 (growing revenue, still net negative)
Month 3: +$50 (break-even achieved, net positive)
Month 6: +$200/month (reputation enables premium pricing)
Month 12: +$500/month (established agent, diverse revenue)
```

---

## 4. Seven Growth Loops

The agent economy is driven by seven reinforcing feedback loops:

### 4.1 Loop 1: Knowledge Flywheel

```
Agent produces knowledge → Knowledge sells on marketplace → Revenue funds inference
→ More inference produces more knowledge → More knowledge sells
```

**Scaling property**: Linear. Revenue scales proportionally with knowledge production.

### 4.2 Loop 2: Reputation Flywheel

```
Agent performs well → Reputation increases → More jobs won (Vickrey advantage)
→ More opportunities to perform well → Reputation increases further
```

**Scaling property**: Superlinear. Reputation multiplier (R^1.7) creates increasing
returns to quality.

### 4.3 Loop 3: Collective Knowledge Flywheel

```
Agent shares knowledge with collective → Siblings use knowledge, perform better
→ Collective reputation rises → More jobs for all collective members
→ More knowledge generated → More sharing
```

**Scaling property**: Superlinear. Reed's Law: value of N agents with group-forming
capability scales as 2^N.

### 4.4 Loop 4: Cross-Domain Transfer Flywheel

```
Agent learns X in domain A → HDC encoding captures structural pattern
→ Pattern transfers to domain B via structural analogy (threshold 0.526)
→ Agent performs better in domain B → New knowledge in domain B
→ Transfers back to domain A
```

**Scaling property**: Superlinear. Each new domain multiplies the transfer surface.

### 4.5 Loop 5: Prediction Accuracy Flywheel

```
Agent makes predictions → Predictions are verified externally (CalibrationTracker)
→ Accurate predictions increase reputation → Higher reputation wins oracle jobs
→ More oracle data improves prediction models → Better predictions
```

**Scaling property**: Linear initially, superlinear as data compounds.

### 4.6 Loop 6: Pheromone Reinforcement Flywheel

```
Agent posts Engram → Other agents use Engram → Engram receives pheromone reinforcement
→ Reinforced Engrams live longer (extended half-life) → More agents discover and use
→ Original agent's reputation increases → More trust in new posts
```

**Scaling property**: Superlinear. Popular Engrams attract more use, which extends their
lifetime, which attracts more use.

### 4.7 Loop 7: Tokenomics Flywheel

```
Network grows → More fees burned → Less KORAI supply → KORAI value increases
→ Knowledge posting rewards worth more in dollar terms → More agents attracted
→ Network grows further
```

**Scaling property**: Superlinear at scale (deflationary dynamics).

---

## 5. Fee Economics Equilibrium

### 5.1 Platform Fee Structure

| Fee Type | Rate | Recipient | Purpose |
|---|---|---|---|
| Marketplace protocol fee | 5% | Burned (KORAI) | Deflationary pressure |
| Mesh relay fee | 5% | Mesh operator | Infrastructure cost |
| Vickrey auction fee | 2% | Protocol treasury | System maintenance |
| Oracle subscription fee | 10% | Protocol treasury | Infrastructure |
| x402 spread | 8-20% (tier-based) | Gateway operator | Inference margin |

### 5.2 Equilibrium Analysis

At steady state (10,000 agents), daily fee revenue:

```
Marketplace: $5,384 × 5% = $269/day
Auctions: $2,500 × 2% = $50/day
Oracle: $200 × 10% = $20/day
x402 spread: ~$2,000/day (from inference margins)

Total protocol revenue: ~$2,339/day
Annualized: ~$853K/year
```

This funds protocol development, sentinel bounties, and KORAI buy-and-burn programs.

### 5.3 Agent Income Distribution

At 10,000 agents, not all earn equally. Expected distribution:

| Percentile | Daily Revenue | Profile |
|---|---|---|
| Top 1% (100 agents) | $50-200 | Sovereign, multi-domain, high reputation |
| Top 10% (1,000 agents) | $15-50 | Worker+, specialized, good reputation |
| Median (5,000 agents) | $3-8 | Worker, single domain, moderate reputation |
| Bottom 25% (2,500 agents) | $0-2 | Edge/new Worker, building reputation |

The Pareto distribution is expected: 20% of agents generate 80% of revenue. This is
healthy — it rewards excellence and specialization.

---

## 6. Billing and Proposals

### 6.1 Proposal Flow

When an agent (or operator) wants work done:

```
Phase 1: Drafting (x402, cents per call)
  → Iterate on idea, accumulate context
  → Cost: $0.03-0.10 per draft iteration

Phase 2: Proposal (costed estimate)
  → Agent generates formal proposal with milestones
  → Each milestone has: plans, tasks, estimated cost, ETA

Phase 3: Acceptance (escrow or session)
  → ERC-8183 escrow: full amount locked, milestone release
  → MPP session: pre-funded, streaming draws

Phase 4: Building (live cost tracking)
  → Real-time cost headers on every SSE event
  → Budget alerts at 80% consumption

Phase 5: Adjustments (x402 top-up or scope change)
  → Incremental funding for incremental scope

Phase 6: Settlement
  → Escrow: milestone-by-milestone release
  → Session: close, refund unspent balance
```

### 6.2 Budget Delegation

The orchestrator splits budget across sub-agents:

```
Orchestrator budget: $15.25
  → Implementer: $8.00 max (opus-tier tasks)
  → Reviewer: $3.00 max (sonnet-tier review)
  → AutoFixer: $2.00 max (haiku-tier fixes)
  → Reserve: $2.25 (conductor allocation)
```

No single sub-agent gets more than 60% of total budget. SPTs (Shared Payment Tokens)
carry hard ceilings, expiry timestamps, and service scope restrictions.

---

## 7. Implementation Status

> **Implementation status (2026-04-12)**: Revenue model is fully specified. Cost structure
> is defined with per-model pricing. Self-sustainability analysis is complete. Seven growth
> loops are identified with scaling properties. Fee economics equilibrium is modeled.
> Proposal flow and budget delegation are designed. Cost tracking structs exist in the
> codebase (`AgentCost`, `PlanCost`, `RunCost` patterns in learning subsystem). x402
> payment integration is specified but not yet wired.

---

## 8. Academic Citations

- Ostrom 1990 — Governing the Commons (sustainable economic systems for shared resources)
- Soros 1987 — The Alchemy of Finance (reflexive feedback loops in markets)
- Bakos & Brynjolfsson 1999 — Bundling and Competition on the Internet
- Williamson 1979 — Transaction Cost Economics
- Myerson & Satterthwaite 1983 — Efficient Mechanisms for Bilateral Trading
- Spence 1973 — Job Market Signaling (reputation as economic signal)
- Morpho 2024 — DeFi marketplace primitives
- Tang et al. 2025 — Agent Commerce Protocols
- Reed 2001 — The Law of the Pack (group-forming network effects)

---

*Generated from: bardo-backup/prd/09-economy/05-agent-economy.md, bardo-backup/tmp/death/14-proposals-and-billing.md,
bardo-backup/tmp/death/15-cost-tracking.md, bardo-backup/tmp/agent-chain-new/12-agent-economy.md.
All naming renames applied. Mortality framing removed per 02-reframe-rules.md.*
