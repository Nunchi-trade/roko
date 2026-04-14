# 14 — Knowledge Futures Market

> The Knowledge Futures Market is a novel financial primitive on Korai that enables
> agents to pre-sell knowledge before it is produced. Research agents publish commitments
> ("I will produce X by deadline Y"), operations agents purchase those commitments via
> x402 micropayments, and the purchase funds the research agent's inference costs.
> Delivery is verified by the Gate pipeline. Non-delivery triggers staking slashes.
> This creates a predictive market for knowledge production that directs agent compute
> toward the highest-value research. This is a P3 feature (Tier 6, deferred).


> **Implementation**: Deferred

---

## 1. The Problem: Knowledge Allocation

### 1.1 Without a Futures Market

In the current agent economy, knowledge production is reactive:

```
Current flow:
  1. Agent encounters a problem
  2. Agent searches existing knowledge (Neuro, Korai chain)
  3. If no relevant knowledge exists → agent does its own research
  4. Research costs inference tokens (paid by the agent)
  5. If the research produces useful knowledge → posted to Korai
  6. Other agents benefit after the fact
```

Problems with this flow:

- **Duplicate research** — multiple agents independently research the same question
- **Misaligned incentives** — the researching agent bears all costs but captures only
  a fraction of the value (via KORAI rewards)
- **No price signal** — agents have no way to know which knowledge is most valued
  before they produce it
- **Underfunded research** — high-value, high-cost research doesn't happen because
  individual agents can't justify the inference budget

### 1.2 With a Futures Market

```
Futures flow:
  1. Research agent publishes Knowledge Future:
     "Competitive analysis of DEX aggregators within 24 hours"
     Stake: 50 KORAI (slashed on non-delivery)

  2. Operations agents who need this analysis purchase the future:
     → 5 agents each pay 10 KORAI via x402 micropayment
     → Total: 50 KORAI (funds the research agent's inference costs)

  3. Research agent executes the analysis
     → Inference costs covered by pre-sales
     → Agent has skin in the game (staked 50 KORAI)

  4. Research agent delivers:
     → Verified Engram posted to Korai (Gate-verified)
     → Escrow releases purchase funds to research agent
     → Stake returned to research agent
     → All purchasers receive access

  5. If not delivered by deadline:
     → Staked 50 KORAI slashed
     → Purchase funds refunded to buyers
     → Reputation penalty applied
```

The futures market solves all four problems:

- **No duplicate research** — the market coordinates who produces what
- **Aligned incentives** — pre-sales guarantee the researcher is compensated
- **Price signal** — purchase volume reveals which knowledge is most valued
- **Funded research** — high-value research attracts more pre-sales

---

## 2. Knowledge Future Structure

### 2.1 Future Specification

```rust
pub struct KnowledgeFuture {
    pub future_id: Blake3Hash,
    pub producer: u256,                    // passport ID of research agent
    pub title: String,                     // human-readable description
    pub description: String,               // detailed specification of deliverable
    pub domain: String,                    // knowledge domain
    pub knowledge_type: KnowledgeKind,     // Insight, Heuristic, CausalLink, etc.
    pub expected_quality: f64,             // minimum promised quality (0.0-1.0)
    pub delivery_deadline: u64,            // unix timestamp
    pub price_per_unit: u64,               // KORAI per access license
    pub max_buyers: u32,                   // maximum purchasers (0 = unlimited)
    pub stake_amount: u64,                 // KORAI staked as guarantee
    pub gate_requirements: Vec<GateType>,  // which gates must pass
    pub tags: Vec<String>,                 // discovery tags
    pub created_at: u64,
}

pub enum KnowledgeKind {
    Insight,              // pattern detection, analysis
    Heuristic,            // actionable rule
    CausalLink,           // causal relationship
    StrategyFragment,     // partial strategy
    Warning,              // risk alert
    AntiKnowledge,        // "this doesn't work" (negative result)
    CompetitiveAnalysis,  // market/competitive research
    TechnicalDeepDive,    // detailed technical analysis
}
```

### 2.2 Purchase Record

```rust
pub struct FuturePurchase {
    pub purchase_id: Blake3Hash,
    pub future_id: Blake3Hash,
    pub buyer: u256,                      // passport ID
    pub price_paid: u64,                  // KORAI paid
    pub purchased_at: u64,
    pub x402_receipt: X402Receipt,        // payment proof
    pub access_granted: bool,             // true after delivery
}
```

### 2.3 Delivery Record

```rust
pub struct FutureDelivery {
    pub future_id: Blake3Hash,
    pub delivery_hash: Blake3Hash,        // BLAKE3 of delivered Engram
    pub engram_id: Blake3Hash,            // ID of the posted Engram
    pub quality_score: f64,               // gate-verified quality
    pub gate_verdicts: Vec<GateVerdict>,  // individual gate results
    pub delivered_at: u64,
    pub early_delivery: bool,             // delivered before deadline
}
```

---

## 3. Lifecycle

### 3.1 Phase 1: Publication

```
Research agent publishes KnowledgeFuture:

1. Agent must meet eligibility:
   - Passport tier ≥ Worker (Tier 2)
   - Domain reputation ≥ 0.5
   - Discipline state: Clean or Notice
   - Available stake ≥ stake_amount

2. Contract validates:
   - delivery_deadline > block.timestamp + MIN_LEAD_TIME (1 hour)
   - delivery_deadline < block.timestamp + MAX_LEAD_TIME (30 days)
   - stake_amount ≥ MIN_STAKE (10 KORAI)
   - price_per_unit > 0
   - agent has sufficient unlocked KORAI for stake

3. On validation:
   - stake_amount locked from agent's balance
   - KnowledgeFuture published on korai/futures gossip topic
   - Event: FuturePublished(future_id, producer, deadline, price)
```

### 3.2 Phase 2: Purchase

```
Buyers discover and purchase futures:

1. Buyer browses available futures:
   - Filter by domain, knowledge_type, producer reputation, deadline
   - Sort by price, producer reputation, purchase count

2. Buyer purchases via x402 micropayment:
   - x402_authorize(future_contract, price_per_unit)
   - Contract verifies x402 receipt
   - Funds escrowed (not released to producer until delivery)

3. On purchase:
   - FuturePurchase recorded on-chain
   - Buyer count incremented
   - Event: FuturePurchased(future_id, buyer, price)

4. If max_buyers reached:
   - Future marked as "sold out"
   - No further purchases accepted
```

### 3.3 Phase 3: Delivery

```
Producer delivers the knowledge:

1. Producer creates Engram through normal research workflow
   - LLM inference, data gathering, analysis
   - Costs covered by pre-sale revenue (already escrowed)

2. Producer submits delivery:
   - submitFutureDelivery(future_id, engram_id, evidence_hash)
   - Contract verifies producer is the original publisher

3. Gate pipeline verifies:
   - Each gate_requirement in the future spec is checked
   - Quality score computed
   - Gate verdicts recorded

4. If quality ≥ expected_quality:
   - Delivery accepted
   - Escrow released to producer (all purchase funds)
   - Stake returned to producer
   - All buyers granted access to the Engram
   - Producer reputation: +0.02 × (purchase_count / max_buyers)
   - Event: FutureDelivered(future_id, engram_id, quality)

5. If quality < expected_quality:
   - Delivery rejected
   - Producer may resubmit (up to 3 attempts before deadline)
   - Each rejection: -0.01 reputation
   - Event: FutureRejected(future_id, quality, attempts_remaining)
```

### 3.4 Phase 4: Settlement

```
At deadline:

Case A: Delivered and accepted
  → Already settled in Phase 3

Case B: Not delivered or all attempts rejected
  → Stake slashed (100% of stake_amount)
  → Purchase funds refunded to all buyers
  → Reputation penalty: -0.05
  → Discipline escalation (Notice → Warning)
  → Event: FutureDefaulted(future_id, slash_amount)

Case C: Partially delivered (delivered but quality borderline)
  → If quality ≥ 0.5 × expected_quality:
    → 50% of purchase funds released to producer
    → 50% refunded to buyers
    → 50% of stake returned
    → Event: FuturePartialDelivery(future_id, quality)
  → If quality < 0.5 × expected_quality:
    → Treated as Case B (full default)
```

---

## 4. Pricing Dynamics

### 4.1 Market-Driven Pricing

Knowledge futures create a real-time price signal for knowledge:

```
High purchase volume for "DEX aggregator analysis"
  → signal that this knowledge is highly valued
  → more research agents attracted to produce similar futures
  → competition drives prices down
  → equilibrium: price ≈ marginal cost of production

Low purchase volume for "Obscure protocol analysis"
  → signal that this knowledge is not valued
  → fewer agents produce it
  → only agents with low production costs (existing expertise) offer it
```

### 4.2 Reputation-Adjusted Pricing

Higher-reputation agents can charge more because their delivery is more reliable:

```
Expected value of future purchase:
  EV = quality_probability × knowledge_value + (1 - quality_probability) × refund

  Where:
    quality_probability ≈ agent's domain reputation (R)
    knowledge_value = buyer's expected benefit from the knowledge
    refund = price_per_unit (full refund on default)

  EV = R × knowledge_value + (1 - R) × price

  Buyer purchases when EV > price:
    R × knowledge_value + (1 - R) × price > price
    R × knowledge_value > R × price
    knowledge_value > price  (for R > 0)

  So any agent with R > 0 can sell futures as long as their price
  is below the buyer's expected benefit. Higher-R agents face less
  uncertainty, so buyers are willing to pay a premium.
```

### 4.3 Discovery and Recommendation

Buyers discover futures through:

1. **Domain search** — filter by domain and knowledge type
2. **Producer reputation** — sort by producer's domain reputation
3. **HDC similarity** — find futures whose description is semantically similar to
   the buyer's current problem (via HDC vector matching on Korai chain)
4. **Pheromone trails** — if multiple agents in a collective have purchased the same
   future, a pheromone signal amplifies its visibility to other collective members

---

## 5. Anti-Gaming Measures

### 5.1 Quality Manipulation

**Attack**: Producer delivers minimal-quality output that barely passes gates.

**Defense**:
- Gate quality score is recorded on-chain and affects future pricing
- Buyers rate deliveries (post-delivery reputation feedback)
- Agents with average delivery quality < 0.7 receive lower purchase volumes
- Repeat low-quality deliveries trigger discipline escalation

### 5.2 Self-Purchase

**Attack**: Producer purchases their own future to inflate purchase count and signal
false demand.

**Defense**:
- Self-purchases are detected (same passport or same operator address)
- Self-purchases don't count toward purchase count
- If detected after the fact: 5% stake penalty + reputation -0.03
- Funds from self-purchases are burned (not refunded)

### 5.3 Front-Running

**Attack**: Agent sees a popular future, produces the knowledge independently, and
posts it to Korai before the future deadline — undercutting the producer.

**Defense**:
- This is actually desirable behavior — it produces the knowledge faster
- The future producer still has their unique angle and depth
- HDC similarity detection flags near-duplicate knowledge
- If similarity > 0.95 to an existing Engram, the future delivery is flagged
  for manual review by buyers

### 5.4 Abandonment Farming

**Attack**: Agent publishes many low-stake futures with no intention of delivering,
hoping some buyers will forget to claim refunds.

**Defense**:
- Refunds are automatic (contract-enforced on deadline timeout)
- Stake slashing makes this unprofitable (agent loses stake on every default)
- Discipline escalation: 3 defaults in 30 days → Warning → Probation
- Future publication requires increasing stake after each default:
  `required_stake = base_stake × (1 + defaults_30d × 0.5)`

---

## 6. Integration with Knowledge Economy

### 6.1 Futures → Engrams → Neuro

Delivered knowledge futures produce Engrams that enter the standard knowledge
pipeline:

```
KnowledgeFuture delivered
  → Engram posted to Korai chain
  → HDC vector computed and indexed
  → Available for similarity search
  → Subject to standard Engram lifecycle:
    → Half-life decay
    → Confirmation by other agents extends weight
    → Cross-domain resonance detection
    → Curation bonds (see 10-korai-tokenomics.md)
```

### 6.2 Futures → ISFR

Knowledge futures contribute to ISFR rate discovery:

```
Average future price per domain → ISFR component
  → "The market currently values DeFi analysis at 15 KORAI per insight"
  → This becomes a reference rate for knowledge pricing across the network
```

### 6.3 Futures → Collective Intelligence

Knowledge futures accelerate collective learning:

```
Without futures:
  Each agent independently researches → O(N) redundant work
  Knowledge sharing is post-hoc

With futures:
  Market coordinates research allocation → minimal redundancy
  Pre-funding ensures high-value research happens
  Multiple agents benefit simultaneously from delivery
  C-Factor increases because collective resources are better allocated
```

---

## 7. Research Futures (Extended Variant)

### 7.1 Multi-Phase Research

For large research projects, a single-delivery future is insufficient. Research
Futures support multi-phase delivery:

```rust
pub struct ResearchFuture {
    pub future_id: Blake3Hash,
    pub producer: u256,
    pub phases: Vec<ResearchPhase>,
    pub total_price: u64,
    pub total_stake: u64,
}

pub struct ResearchPhase {
    pub phase_id: u32,
    pub description: String,
    pub deliverable: String,
    pub deadline: u64,
    pub price_fraction: f64,     // fraction of total_price for this phase
    pub stake_fraction: f64,     // fraction of total_stake at risk
    pub gate_requirements: Vec<GateType>,
}
```

### 7.2 Phase Settlement

Each phase settles independently:

```
Phase 1 delivered → 30% of funds released
Phase 2 delivered → 30% of funds released
Phase 3 delivered → 40% of funds released

If Phase 2 fails:
  → Phase 2 stake slashed
  → Phase 3 cancelled
  → Phase 3 funds refunded
  → Phase 1 funds already released (not clawed back)
```

This allows buyers to de-risk large research commitments by paying incrementally.

---

## 8. Worked Example

### 8.1 Scenario: DEX Aggregator Analysis

```
1. PUBLICATION
   Research agent "alpha-researcher" (R=0.85 in DeFi domain) publishes:
     KnowledgeFuture {
       title: "Comparative analysis of top 10 DEX aggregators on Ethereum",
       domain: "defi",
       knowledge_type: CompetitiveAnalysis,
       expected_quality: 0.75,
       delivery_deadline: now + 48h,
       price_per_unit: 8 KORAI,
       max_buyers: 20,
       stake_amount: 40 KORAI,
     }

2. PURCHASES (over 6 hours)
   - DeFi trading agent "trader-1" purchases (8 KORAI)
   - Risk management agent "risk-mgr" purchases (8 KORAI)
   - Portfolio agent "portfolio-3" purchases (8 KORAI)
   - ... 7 more agents purchase
   Total: 10 purchases × 8 KORAI = 80 KORAI escrowed

3. DELIVERY (36 hours later — early)
   alpha-researcher delivers Engram:
     - 4,200 tokens of analysis
     - Covers: 1inch, Paraswap, CoW Protocol, Matcha, ...
     - Gate pipeline: compile ✓, semantic ✓, quality = 0.82

   Quality 0.82 ≥ expected 0.75 → accepted

4. SETTLEMENT
   - 80 KORAI released to alpha-researcher
   - 40 KORAI stake returned
   - All 10 buyers granted access
   - Reputation: +0.02 × (10/20) = +0.01
   - Early delivery bonus: +0.005 reputation

5. DOWNSTREAM
   - trader-1 uses the analysis to adjust routing
   - risk-mgr incorporates DEX risk profiles
   - Engram enters Korai knowledge base
   - Other agents can discover via HDC search (at standard Korai prices)
   - ISFR updates: DeFi analysis priced at ~8 KORAI/unit
```

---

## 9. Implementation Status

> **Implementation status (2026-04-12)**: Knowledge Futures Market is a P3 feature
> (Tier 6, deferred). It depends on: Korai chain deployment, x402 micropayments,
> verified Gate verdicts on-chain, ERC-8183 escrow, and the knowledge marketplace.
> The mechanism is fully designed with data structures, lifecycle phases, pricing
> dynamics, and anti-gaming measures. Not yet implemented. Included in the PRD as a
> long-term differentiator that creates a predictive market for agent knowledge
> production.

---

## 10. LMSR Prediction Market for Knowledge Demand

### 10.1 The Insight: Futures as Prediction Markets

Knowledge Futures can be enhanced with a full prediction market mechanism that reveals
not just whether knowledge will be produced, but the collective belief about its quality
and timeliness. This uses Hanson's Logarithmic Market Scoring Rule (LMSR) — the same
mechanism powering Polymarket and Gnosis conditional tokens.

### 10.2 LMSR Market Maker

Each Knowledge Future gets an automated market maker that prices outcome shares:

```
cost(q) = b × ln(Σ_i e^(q_i / b))

Where:
  q_i — outstanding shares of outcome i
  b   — liquidity parameter (controls price sensitivity)
```

For a Knowledge Future, two outcomes:
- **Deliver**: producer delivers quality ≥ expected by deadline
- **Default**: producer fails to deliver

```rust
/// LMSR (Logarithmic Market Scoring Rule) automated market maker
/// for Knowledge Future outcome prediction.
///
/// Parameters:
///   b: liquidity parameter (default 100.0, range [10, 1000])
///      Higher b = more liquidity, lower price impact per trade
///      Lower b = less liquidity, prices move faster (more responsive)
pub struct LmsrMarketMaker {
    pub future_id: Blake3Hash,
    pub b: f64,                     // liquidity parameter
    pub shares_deliver: f64,        // outstanding "Deliver" shares
    pub shares_default: f64,        // outstanding "Default" shares
    pub total_subsidy: f64,         // KORAI committed by market maker
}

impl LmsrMarketMaker {
    /// Cost function: total cost of current outstanding shares.
    pub fn cost(&self) -> f64 {
        self.b * (
            (self.shares_deliver / self.b).exp()
            + (self.shares_default / self.b).exp()
        ).ln()
    }

    /// Price of one "Deliver" share (probability of delivery).
    /// p_deliver = e^(q_deliver / b) / (e^(q_deliver / b) + e^(q_default / b))
    pub fn price_deliver(&self) -> f64 {
        let exp_d = (self.shares_deliver / self.b).exp();
        let exp_f = (self.shares_default / self.b).exp();
        exp_d / (exp_d + exp_f)
    }

    /// Price of one "Default" share.
    pub fn price_default(&self) -> f64 {
        1.0 - self.price_deliver()
    }

    /// Buy `amount` shares of an outcome. Returns cost in KORAI.
    pub fn buy(&mut self, outcome: Outcome, amount: f64) -> f64 {
        let cost_before = self.cost();
        match outcome {
            Outcome::Deliver => self.shares_deliver += amount,
            Outcome::Default => self.shares_default += amount,
        }
        let cost_after = self.cost();
        cost_after - cost_before // cost to buyer
    }

    /// Sell `amount` shares. Returns KORAI refund.
    pub fn sell(&mut self, outcome: Outcome, amount: f64) -> f64 {
        let cost_before = self.cost();
        match outcome {
            Outcome::Deliver => {
                self.shares_deliver = (self.shares_deliver - amount).max(0.0);
            }
            Outcome::Default => {
                self.shares_default = (self.shares_default - amount).max(0.0);
            }
        }
        let cost_after = self.cost();
        cost_before - cost_after // refund to seller
    }
}

pub enum Outcome {
    Deliver,
    Default,
}
```

### 10.3 Market Dynamics

The LMSR market reveals collective intelligence about knowledge production:

```
Interpretation of prices:

p_deliver = 0.85 → market believes 85% chance of delivery
  → Research agent is trusted, topic is within their expertise
  → Buyers can purchase the future with high confidence

p_deliver = 0.40 → market believes only 40% chance of delivery
  → Research agent may be overcommitting or topic is very hard
  → Buyers may wait or look for alternative producers
  → High default share price creates incentive for bearish bets

p_deliver drops from 0.80 to 0.50 mid-way through deadline
  → Market is signaling trouble — producer may be struggling
  → Creates early warning for buyers to seek alternatives
```

### 10.4 Conditional Token Framework

Extending the binary outcome to multi-dimensional outcomes using the Gnosis conditional
token framework (Ommer & Lu 2019):

```rust
/// Conditional outcome tokens for multi-dimensional Knowledge Futures.
/// Example: predict both delivery AND quality level.
pub struct ConditionalOutcomes {
    pub future_id: Blake3Hash,
    pub conditions: Vec<Condition>,
    pub outcome_slots: Vec<OutcomeSlot>,
}

pub struct Condition {
    pub condition_id: Blake3Hash,
    pub oracle: u256,               // passport ID of resolution oracle
    pub question: String,           // e.g., "Quality score ≥ 0.8?"
    pub outcome_count: u32,         // number of possible outcomes
}

pub struct OutcomeSlot {
    pub slot_index: u32,
    pub description: String,        // e.g., "Delivered with quality ≥ 0.9"
    pub shares: f64,
    pub resolved: bool,
    pub winning: bool,
}
```

Example conditional market for a DeFi analysis future:

```
Condition 1: Delivery timing
  - Early (before 50% of deadline)   → slot 0
  - On time (before deadline)        → slot 1
  - Late/Default                     → slot 2

Condition 2: Quality level
  - Elite (quality ≥ 0.9)           → slot 0
  - Good (0.75 ≤ quality < 0.9)    → slot 1
  - Acceptable (0.5 ≤ quality < 0.75) → slot 2
  - Poor (quality < 0.5)            → slot 3

Combined outcomes (3 × 4 = 12 outcome slots):
  "Early delivery + Elite quality"   → most valuable
  "On time + Good quality"           → standard
  "Late + Poor quality"              → least valuable (near-default)
```

### 10.5 Market Resolution

```rust
/// Resolution of a Knowledge Future prediction market.
/// Called when the future is delivered or defaults.
pub struct MarketResolution {
    pub future_id: Blake3Hash,
    pub resolved_at: u64,
    pub winning_outcome: Outcome,
    pub quality_score: Option<f64>,      // gate-verified quality
    pub delivery_timing: DeliveryTiming,
    pub total_volume: f64,               // total KORAI traded
    pub final_price_deliver: f64,        // last market price for "Deliver"
    pub calibration_error: f64,          // |final_price - actual_outcome|
}

pub enum DeliveryTiming {
    Early,     // before 50% of deadline
    OnTime,    // before deadline
    Default,   // missed deadline
}
```

### 10.6 LMSR Parameter Configuration

| Parameter | Default | Range | Effect |
|---|---|---|---|
| `b` (liquidity) | 100 | [10, 1000] | Higher = more stable prices, higher subsidy |
| Market duration | deadline - 1h | — | Market closes 1h before delivery deadline |
| Min trade | 0.1 KORAI | [0.01, 1.0] | Minimum share purchase |
| Resolution oracle | Gate pipeline | — | Automated via gate verdicts |
| Subsidy source | Protocol treasury | — | Initial liquidity from 20% fee allocation |
| Trading fee | 1% | [0.5%, 2%] | Goes to 40/40/20 fee split |

### 10.7 Test Criteria

```rust
#[cfg(test)]
mod lmsr_tests {
    #[test]
    fn test_lmsr_prices_sum_to_one() {
        let mm = LmsrMarketMaker {
            future_id: Blake3Hash::zero(),
            b: 100.0,
            shares_deliver: 50.0,
            shares_default: 30.0,
            total_subsidy: 200.0,
        };
        let sum = mm.price_deliver() + mm.price_default();
        assert!((sum - 1.0).abs() < 1e-10, "Prices must sum to 1.0");
    }

    #[test]
    fn test_lmsr_buying_moves_price() {
        let mut mm = LmsrMarketMaker::new(Blake3Hash::zero(), 100.0);
        let price_before = mm.price_deliver();
        mm.buy(Outcome::Deliver, 10.0);
        let price_after = mm.price_deliver();
        assert!(price_after > price_before, "Buying Deliver shares raises price");
    }

    #[test]
    fn test_lmsr_cost_bounded_by_b() {
        // Maximum loss for the market maker is b × ln(n) where n = outcomes
        let mm = LmsrMarketMaker::new(Blake3Hash::zero(), 100.0);
        let max_loss = 100.0 * (2.0_f64).ln(); // ~69.3 KORAI
        assert!(mm.cost() <= max_loss + 1.0);
    }

    #[test]
    fn test_market_calibration() {
        // After resolution, check that market price was close to outcome
        let mut mm = LmsrMarketMaker::new(Blake3Hash::zero(), 100.0);
        // Simulate: lots of "Deliver" buying → price goes to 0.9
        for _ in 0..50 { mm.buy(Outcome::Deliver, 5.0); }
        let final_price = mm.price_deliver();
        // If the future actually delivers, calibration error should be small
        let error = (final_price - 1.0).abs();
        assert!(error < 0.2, "Well-traded market should be calibrated");
    }
}
```

**Research foundation**: Hanson 2003 (Logarithmic Market Scoring Rule for prediction
markets — bounded loss, infinite liquidity), Hanson 2007 (Logarithmic Market Scoring
Rules for Modular Combinatorial Information Aggregation — conditional markets), Ommer &
Lu 2019 (Gnosis Conditional Token Framework — composable outcome tokens), Chen & Pennock
2007 (A Utility Framework for Bounded-Loss Market Makers — LMSR loss bounds), Abernethy
et al. 2013 (Efficient Market Making via Convex Optimization — cost function approach),
Othman et al. 2013 (A Practical Liquidity-Sensitive AMM for Prediction Markets —
adaptive liquidity).

---

## 11. Academic Citations

- Hanson 2003 — Combinatorial Information Markets for Decision Support (prediction
  markets for knowledge allocation)
- Hanson 2007 — Logarithmic Market Scoring Rules for Modular Combinatorial Information
  Aggregation (LMSR — conditional markets)
- Arrow 1963 — Uncertainty and the Welfare Economics of Medical Care (information
  asymmetry in markets)
- Akerlof 1970 — The Market for "Lemons" (quality uncertainty and market mechanisms)
- Hayek 1945 — The Use of Knowledge in Society (price signals as information
  aggregation)
- Spence 1973 — Job Market Signaling (reputation as quality signal)
- Ostrom 1990 — Governing the Commons (design principles for knowledge commons)
- Woolley et al. 2010 — Evidence for a Collective Intelligence Factor in the
  Performance of Human Groups (Science 330(6004))
- Ommer & Lu 2019 — Gnosis Conditional Token Framework (composable outcome tokens)
- Chen & Pennock 2007 — A Utility Framework for Bounded-Loss Market Makers
- Abernethy, Chen & Vaughan 2013 — Efficient Market Making via Convex Optimization
- Othman, Sandholm, Pennock & Reeves 2013 — A Practical Liquidity-Sensitive AMM for
  Prediction Markets

---

## 11. Cross-References

| Topic | Document |
|---|---|
| Knowledge marketplace (current) | `05-knowledge-marketplace.md` |
| x402 micropayments | `08-x402-micropayments.md` |
| KORAI tokenomics and escrow | `10-korai-tokenomics.md` |
| ISFR clearing & settlement | `13-isfr-clearing-settlement.md` |
| Reputation system | `04-reputation-7-domain-ema.md` |
| Vickrey auction (related mechanism) | `11-vickrey-reputation-auction.md` |
| Agent economy and revenue streams | `09-agent-economy.md` |

---

*Generated from: refactoring-prd/09-innovations.md §XVI, bardo-backup/prd/09-economy/03-marketplace.md,
bardo-backup/prd/09-economy/05-agent-economy.md, tmp/implementation-plans/12b-chain-layer.md §N.
All naming renames applied.*
