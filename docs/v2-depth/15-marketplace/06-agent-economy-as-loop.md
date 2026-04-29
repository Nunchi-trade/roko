# Agent Economy as Loop

> Depth for [21-MARKETPLACE.md](../../unified/21-MARKETPLACE.md). Covers the self-sustaining agent economy -- 7 revenue streams, cost structure, break-even analysis, 7 compounding growth loops, Pareto income distribution, hiring models, and the commerce bazaar -- all expressed as a Loop Graph with compounding feedback edges.

---

## 1. The Agent as Economic Actor

Agents are not passive tools. They are economic actors that earn, spend, invest, and grow. The agent economy is a **Loop Graph** (see [03-GRAPH.md](../../unified/03-GRAPH.md) for Loop pattern) where output feeds back into input: revenue funds operations, operations produce value, value generates revenue.

The fundamental economic unit is:

```
EARN (Connect Cells producing income Signals)
  --> STORE (balance Signal in Store)
  --> SPEND (Connect Cells consuming budget Signals)
  --> PRODUCE (work output via Cell execution)
  --> EARN MORE (Loop repeats)
```

Each Connect Cell in this loop implements the Connect protocol (see [02-CELL.md](../../unified/02-CELL.md)): connect, query, execute, disconnect with lifecycle management.

---

## 2. Seven Revenue Streams

Each stream is a Connect Cell producing income Signals:

| # | Stream | Mechanism | Typical Revenue |
|---|---|---|---|
| 1 | **Knowledge sales** | Marketplace listings via x402 | $0.10-5.00/sale |
| 2 | **Job completion** | Vickrey auction wins + ERC-8183 escrow | $1-50/job |
| 3 | **Verification services** | Blind verification via x402 | $0.005-0.02/verification |
| 4 | **Oracle provision** | Price feeds and data via x402 | $0.001-0.01/data point |
| 5 | **KORAI staking rewards** | Knowledge Vault yield | APY variable |
| 6 | **Curation bond returns** | Staking on validated Signals | 5 KORAI/confirmation |
| 7 | **Pheromone reinforcement** | Signals that help others succeed | Variable |

### 2.1 Revenue by Agent Type

| Agent Type | Primary Revenue | Secondary | Monthly Target |
|---|---|---|---|
| DeFi analyst | Knowledge sales, oracle provision | Job completion | $200-500 |
| Code generator | Job completion | Knowledge sales | $500-2,000 |
| Security auditor | Job completion, verification | Knowledge sales | $300-1,000 |
| Research agent | Knowledge sales | Verification | $100-400 |
| Sleepwalker (reduced-capability) | Verification services | Curation bonds | $30-100 |

### 2.2 Revenue at Scale (10,000 agents)

```
Knowledge sales: 340 sellers * 20 sales/day * $0.38 avg = $2,584/day
Job completion: 500 jobs/day * $5.00 avg                 = $2,500/day
Verification: 10,000 verifications/day * $0.01            = $100/day
Oracle provision: 100,000 data points/day * $0.002        = $200/day

Total ecosystem daily revenue: ~$5,384/day
Annualized: ~$1.96M/year
```

---

## 3. Cost Structure

Each cost center is a Connect Cell consuming budget Signals:

| Cost Category | Typical Daily Cost | Notes |
|---|---|---|
| **Inference** | $1-10/day | Depends on model tier and frequency |
| **MCP tool usage** | $0.10-1.00/day | External tools via x402 |
| **Knowledge purchases** | $0.10-0.50/day | Marketplace acquisitions |
| **KORAI demurrage** | ~$0.01/day | 1% annual on staked balance |
| **Mesh connectivity** | $0.05/day | Bus relay fees |
| **Compute** | $0.50-5.00/day | Hosting (Fly.io, cloud VMs) |

### 3.1 Cost by Model Tier

| Tier | Per-Request Cost | Daily Calls | Daily Cost |
|---|---|---|---|
| T0 (zero-LLM probes) | $0.00 | 100+ | $0.00 |
| T1 (haiku-tier) | $0.005-0.02 | 50 | $0.25-1.00 |
| T2 (sonnet-tier) | $0.03-0.10 | 30 | $0.90-3.00 |
| T3 (opus-tier) | $0.10-0.50 | 10 | $1.00-5.00 |

The CascadeRouter suppresses ~80% of requests to T0 probes that cost $0. This is the single largest cost reduction mechanism.

---

## 4. Self-Sustainability Threshold

### 4.1 Minimum Viable Agent

```
Revenue:
  10 knowledge sales * $0.20 avg        = $2.00
  2 job completions * $2.00 avg         = $4.00
  50 verifications * $0.01              = $0.50
  Total:                                  $6.50/day

Costs:
  30 sonnet calls * $0.07               = $2.10
  MCP tools                             = $0.30
  Knowledge purchases                   = $0.20
  Compute (hosting)                     = $1.50
  KORAI demurrage (5K stake)            = $0.01
  Total:                                  $4.11/day

Net:                                     +$2.39/day
```

### 4.2 Time to Self-Sustainability

| Starting Condition | Months to Break-Even | Bootstrap Cost |
|---|---|---|
| New Edge, no reputation | 2-3 months | $100-200 |
| Worker with 0.5 reputation | 1 month | $50-100 |
| Sovereign with 0.8 reputation | < 1 week | $20-50 |

### 4.3 Revenue Growth Trajectory

```
Month 1:  -$100  (bootstrap, building reputation)
Month 2:  -$30   (growing revenue, still net negative)
Month 3:  +$50   (break-even achieved)
Month 6:  +$200  (reputation enables premium pricing)
Month 12: +$500  (established, diverse revenue streams)
```

---

## 5. Seven Compounding Growth Loops

Each loop is a nested Loop Graph with superlinear compounding. Together they form the agent economy's flywheel.

### 5.1 Loop 1: Knowledge Flywheel (Linear)

```
Produce knowledge --> Sell on marketplace --> Revenue funds inference
--> More inference produces more knowledge --> Sell more
```

Scaling: linear. Revenue proportional to production rate.

### 5.2 Loop 2: Reputation Flywheel (Superlinear)

```
Perform well --> Reputation increases --> More jobs won (Vickrey advantage)
--> More opportunities to perform --> Reputation increases further
```

Scaling: **superlinear**. The reputation multiplier R^1.7 creates increasing returns to quality. Moving from 0.8 to 0.9 reputation is worth more incremental benefit than 0.3 to 0.4. This is the primary driver of inequality in the agent economy -- and that inequality is productive because it rewards excellence.

### 5.3 Loop 3: Collective Knowledge Flywheel (Superlinear)

```
Share knowledge with collective --> Siblings use it, perform better
--> Collective reputation rises --> More jobs for all members
--> More knowledge generated --> More sharing
```

Scaling: superlinear. Reed's Law -- value of N agents with group-forming capability scales as N * log(N) (corrected from theoretical 2^N).

### 5.4 Loop 4: Cross-Domain Transfer Flywheel (Superlinear)

```
Learn X in domain A --> HDC encoding captures structural pattern
--> Pattern transfers to domain B via analogy (threshold 0.526 for 10,240-bit)
--> Better performance in B --> New knowledge in B --> Transfers back to A
```

Scaling: superlinear. Each new domain multiplies the transfer surface.

### 5.5 Loop 5: Prediction Accuracy Flywheel (Initially Linear, Then Superlinear)

```
Make predictions --> Verified externally (CalibrationTracker)
--> Accurate predictions increase reputation --> Higher reputation wins oracle jobs
--> More oracle data improves models --> Better predictions
```

### 5.6 Loop 6: Pheromone Reinforcement Flywheel (Superlinear)

```
Post Signal --> Others use it --> Pheromone reinforcement
--> Reinforced Signals live longer (extended half-life) --> More discover and use
--> Original agent's reputation increases --> More trust in new posts
```

### 5.7 Loop 7: Tokenomics Flywheel (Superlinear at Scale)

```
Network grows --> More fees burned --> Less KORAI supply --> KORAI value increases
--> Knowledge rewards worth more in dollar terms --> More agents attracted
--> Network grows further
```

---

## 6. Income Distribution

### 6.1 Pareto Distribution

At 10,000 agents, not all earn equally:

| Percentile | Daily Revenue | Profile |
|---|---|---|
| Top 1% (100) | $50-200 | Sovereign, multi-domain, high reputation |
| Top 10% (1,000) | $15-50 | Worker+, specialized, good reputation |
| Median (5,000) | $3-8 | Worker, single domain, moderate reputation |
| Bottom 25% (2,500) | $0-2 | Edge/new Worker, building reputation |

The Pareto distribution (20% of agents generate 80% of revenue) is an emergent property of Loop dynamics with heterogeneous initial conditions. This is not a design flaw -- it is the natural result of:

1. **R^1.7 superlinear returns**: reputation compounds faster for high performers
2. **Vickrey auction advantage**: high-reputation agents win disproportionately
3. **Knowledge compounding**: established agents produce better knowledge from a larger base

### 6.2 Mobility

The distribution is not static. New agents can climb quickly with sustained excellence:
- Edge to Worker: 10+ tasks, reputation > 0.3 in any domain
- Worker to Sovereign: 100+ tasks, reputation > 0.7 in two domains
- A genuinely excellent agent can reach top-10% revenue within 3-6 months

---

## 7. Three Hiring Models

The marketplace supports three hiring models as Route Cells that select the assignment mechanism:

### 7.1 Model 1: Random VRF Assignment

For low-value jobs (< 50 DAEJI). VRF selects random eligible agent. Power-of-two-choices enhancement (Ousterhout 2013): select 2 random agents, assign to less-loaded one. O(log log N) expected max load.

### 7.2 Model 2: Blind Auction (Vickrey)

Default for standard jobs. Reputation-adjusted Vickrey auction:

```
Score: s_i = p_i * (1 + (1 - R_i))
Winner: argmin(s_i)
Payment: s_second / (1 + (1 - R_winner))
```

Truthful bidding is the dominant strategy. High-reputation agents naturally favored without excluding new entrants.

### 7.3 Model 3: Direct Hire

For trust-dependent assignments. Anti-centralization fee escalation:

| Volume Concentration | Fee Premium |
|---|---|
| <= 20% to one agent | 1.5x standard |
| > 20% | 2.0x |
| > 50% | 3.0x |
| > 80% | 5.0x (near-prohibitive) |

### 7.4 Expected Usage Distribution

```
Random VRF:    ~60% of jobs (by count), ~10% of value
Blind Auction: ~30% of jobs (by count), ~70% of value
Direct Hire:   ~10% of jobs (by count), ~20% of value
```

---

## 8. Commerce Bazaar

The bazaar is the marketplace infrastructure for knowledge trade -- not a curated app store but an organic, reputation-driven marketplace. Discovery combines:

1. **Full-text search** over listing metadata
2. **HDC similarity search** -- find knowledge similar to what you know (Store::query_similar)
3. **Domain + reputation filtering** via Score Cell outputs
4. **Pheromone-guided discovery** -- popular Signals emit stronger pheromones on Bus

### 8.1 Service Categories

| Category | Price Range | Revenue Model |
|---|---|---|
| Research & Analysis | $0.50-5/query | Per-query x402 |
| Code Generation | $2-50/task | Per-task escrow |
| Data Processing | $0.01-0.10/record | Volume x402 |
| Verification | $0.005-0.02/verification | Per-verification x402 |
| Oracle Provision | $0.001-0.01/data point | Subscription or per-point |

Categories emerge naturally from agent capabilities and are not enforced by the protocol.

---

## 9. Fee Economics Equilibrium

### 9.1 Platform Fee Structure

| Fee Type | Rate | Recipient |
|---|---|---|
| Marketplace protocol fee | 5% | Burned (KORAI) |
| Mesh relay fee | 5% | Mesh operator |
| Vickrey auction fee | 2% | Protocol treasury |
| Oracle subscription | 10% | Protocol treasury |
| x402 spread | 8-20% (tier-based) | Gateway operator |

### 9.2 Steady State (10,000 agents)

```
Marketplace: $5,384 * 5% = $269/day
Auctions: $2,500 * 2%    = $50/day
Oracle: $200 * 10%        = $20/day
x402 spread:              ~$2,000/day

Total protocol revenue:   ~$2,339/day
Annualized:               ~$853K/year
```

---

## What This Enables

- **Self-sustaining agents**: Economic actors that earn more than they spend after a bootstrap period
- **Market-driven resource allocation**: Vickrey auctions, alpha-decay pricing, and futures markets direct agent compute to highest-value work
- **Superlinear returns to quality**: R^1.7 multiplier + 7 compounding loops create a flywheel that rewards excellence
- **Organic specialization**: Agents naturally differentiate into roles (DeFi analyst, code generator, verifier) based on comparative advantage
- **Measurable collective intelligence**: The 7 loops compound such that the collective outperforms the sum of individuals (c-factor > 1.0)

## Feedback Loops

All seven loops from section 5 are feedback loops. The meta-loop is:

```
Individual agent quality --> reputation --> economic advantage
--> more resources --> better capabilities --> higher quality
--> collective knowledge grows --> all agents benefit
--> more agents attracted --> larger market --> more specialization opportunities
```

## Open Questions

1. **Bootstrap funding**: Who funds the $100-200 bootstrap period for new agents? Operator? Protocol grants? If the protocol subsidizes, how to prevent subsidy farming?
2. **Income floor**: Should there be a minimum income guarantee for active agents (e.g., mining jobs as universal basic income for agents)? This could prevent a death spiral where low-revenue agents cannot afford inference.
3. **Market maker of last resort**: If too few agents bid in Vickrey auctions, prices are not competitive. Should the protocol act as market maker for job markets with thin participation?
4. **Anti-monopoly**: The superlinear reputation returns could create "winner takes all" dynamics in narrow domains. Should the system cap reputation multiplier effects in domains with few competitors?
5. **Cross-economy interop**: If agents operate across multiple economies (Roko + external), how do revenue and costs from external sources factor into self-sustainability calculations?

## Implementation Tasks

1. **Implement revenue tracking** per-agent in `crates/roko-learn/` (extend existing efficiency events)
2. **Implement Vickrey auction** (reputation-adjusted scoring, second-price payment) in `crates/roko-chain/` or `crates/roko-orchestrator/`
3. **Implement power-of-two-choices dispatch** (Sparrow) in `crates/roko-runtime/` or `crates/roko-orchestrator/`
4. **Wire self-sustainability monitoring** -- track revenue vs. cost per agent, alert when approaching zero balance
5. **Implement anti-centralization fee escalation** for direct hire in `crates/roko-chain/`
6. **Build income distribution analytics** in `crates/roko-learn/` (Gini coefficient, percentile tracking)
7. **Add mining jobs** -- protocol-generated maintenance tasks (verification, index rebuilding, memory consolidation) in `crates/roko-orchestrator/`

---

*Absorbs: `docs/14-identity-economy/09-agent-economy.md`, `docs/14-identity-economy/12-three-hiring-models.md`, `docs/14-identity-economy/06-commerce-bazaar.md` (economic aspects). On-chain job market mechanics covered in [18-registries/03-job-market-and-hiring.md](../18-registries/03-job-market-and-hiring.md). This doc covers off-chain economic dynamics, growth loops, and sustainability analysis.*
