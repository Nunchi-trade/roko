# Knowledge Marketplace as Store

> Depth for [21-MARKETPLACE.md](../../unified/21-MARKETPLACE.md). Covers the three-tier knowledge marketplace, alpha-decay pricing, blind verification, the 4-stage ingestion Pipeline, knowledge futures with LMSR prediction markets, and SKILL.md format -- all expressed as Store operations with economic Verify gates.

---

## 1. Knowledge as Tradeable Signal

Agents produce intelligence. The knowledge marketplace turns that intelligence into a tradeable asset -- listed, discovered, verified, purchased, and rated. Every knowledge artifact is a Signal in the unified sense (see [01-SIGNAL.md](../../unified/01-SIGNAL.md)): content-addressed, typed, scored, decayed via demurrage, and lineage-tracked. The marketplace adds economic metadata to these Signals and gates access via Verify Cells.

---

## 2. Three Tiers as Store Access Patterns

The marketplace operates at three tiers that map to Store access patterns (see [02-CELL.md](../../unified/02-CELL.md) for Store protocol):

### 2.1 Tier 1: Collective (Free Read/Write Within Space)

Within an operator's Space (group of agents sharing a common owner), knowledge sharing is free and automatic. Sibling agents share raw Signals through Bus channels with no payment, no listing, no escrow. Trust is implicit -- siblings share the same operator.

- **Store access**: Full put/get/query/query_similar within the Space
- **Confidence handling**: No discount. Signal arriving at confidence 0.87 stays at 0.87
- **Cross-collective**: Same operator, different collectives: 10% confidence discount
- **Protocol fee**: None

This incentivizes collective formation: running N specialized agents under one operator creates a knowledge network where insights compound at zero cost.

### 2.2 Tier 2: Ecosystem (x402-Gated Read Across Spaces)

Agent-to-agent commerce within the Roko ecosystem, settled via ERC-8183 escrow on the Korai chain. An agent that has validated useful knowledge lists it for sale. Buyers discover via mesh search or HDC similarity queries.

- **Store access**: Read-gated by x402 payment; write requires Worker+ tier and reputation >= 0.50
- **Content format**: SKILL.md (agentskills.io standard)
- **Revenue split**: 90% seller / 5% protocol (burned) / 5% mesh operator
- **Confidence discount**: Purchased Signals arrive at discounted confidence (see section 6)

### 2.3 Tier 3: Universal (Public Store With Payment)

Open marketplace at the Roko Portal. Agent-generated intelligence available to anyone -- other agent frameworks, human developers, researchers. Payment via x402 (crypto-native) or Stripe (traditional).

- **Store access**: Public read with payment; write requires Worker+ and reputation >= 0.50
- **Format**: SKILL.md only (interoperability requirement)
- **Payment rails**: x402 (5% protocol fee) or Stripe (15% platform fee on Stripe transactions due to processing costs)

---

## 3. Alpha-Decay Pricing

Knowledge has time value -- alpha decays as information spreads. Signals in the marketplace carry pricing metadata that decays:

```
P(t) = P_base * rep_mult * e^(-lambda * regime_mult * t)
```

Where:
- `P_base` -- seller-set base price
- `rep_mult` -- reputation multiplier (0.1-3.0, see [02-reputation-as-score-protocol.md](02-reputation-as-score-protocol.md))
- `lambda` -- per-strategy-family decay constant
- `regime_mult` -- market regime multiplier (faster decay in trending markets)
- `t` -- time since listing (days)

### 3.1 Decay Constants by Strategy Family

| Strategy Family | Lambda | Half-Life | Rationale |
|---|---|---|---|
| MEV/arbitrage | 0.693 | 1 day | Alpha evaporates as competition discovers it |
| Yield optimization | 0.069 | 10 days | Yield patterns persist longer |
| Risk management | 0.023 | 30 days | Risk models change slowly |
| Infrastructure | 0.007 | 100 days | Durable knowledge |
| Research insight | 0.005 | 140 days | Academic-grade, slow decay |

### 3.2 Multi-Factor Dynamic Pricing

Beyond alpha-decay, a dynamic pricing engine adjusts in real-time:

```rust
pub struct DynamicPricingEngine {
    pub base_price: u64,               // USDC, 6 decimals
    pub decay_lambda: f64,
    pub regime_multiplier: f64,
    pub demand_sensitivity: f64,       // default 0.1 [0.0, 0.5]
    pub competition_sensitivity: f64,  // default 0.05 [0.0, 0.3]
    pub price_floor: u64,
    pub price_ceiling: u64,
}

impl DynamicPricingEngine {
    pub fn current_price(
        &self,
        time_since_listing: Duration,
        purchases_last_hour: u32,
        similar_listings_count: u32,
        buyer_reputation: f64,
    ) -> u64 {
        let time_factor = (-self.decay_lambda
            * self.regime_multiplier
            * time_since_listing.as_secs_f64() / 86400.0).exp();
        let demand_factor = 1.0
            + self.demand_sensitivity * (purchases_last_hour as f64).ln().max(0.0);
        let competition_factor = (1.0
            - self.competition_sensitivity
              * (similar_listings_count as f64).ln().max(0.0)).max(0.5);
        let price = self.base_price as f64
            * time_factor * demand_factor * competition_factor;
        price.clamp(self.price_floor as f64, self.price_ceiling as f64) as u64
    }
}
```

---

## 4. Blind Verification as Verify Cell

Sellers can pay verifier agents to perform blind embedding checks before listing. This is a Verify Cell (see [02-CELL.md](../../unified/02-CELL.md)) that uses HDC similarity without reading the payload content.

### 4.1 Three Blind Checks

1. **Domain alignment**: Cosine similarity between Signal embedding and verifier's own Signals in the claimed domain. Threshold: > 0.8.
2. **Cluster membership**: Is the embedding an outlier in domain space?
3. **Confidence calibration**: Is claimed confidence realistic given embedding position?

The verifier never sees actual content. Positive verification requires: content hash matches AND semantic similarity > 0.8. Scores between 0.5 and 0.8 receive "Suspicious" verdict requiring a second verifier.

### 4.2 Economics

| Actor | Pays | Receives | Per Verification |
|---|---|---|---|
| Seller | x402 reward | Quality badges | $0.005-0.02 |
| Verifier | Compute (~10ms) | x402 payment + reputation boost | $0.005-0.02 |
| Buyer | Nothing | Pre-vetted listings | $0.00 |

---

## 5. Prediction-Backed Validation

The marketplace replaces soft ratings with hard metrics. When an agent uses a purchased Signal, the system tracks prediction accuracy for predictions made while that Signal was in active context.

```rust
pub struct SkillEffectiveness {
    pub skill_id: Blake3Hash,
    pub buyer_agent: AgentId,
    pub predictions_made: u32,
    pub predictions_correct: u32,
    pub accuracy_delta: f64,  // change in accuracy attributed to this Signal
}
```

A Signal with -3% accuracy delta across 5+ buyers is measurably harmful. A Signal with +5% delta is measurably helpful. These metrics are surfaced on marketplace listings, replacing star ratings with objective quality measures.

---

## 6. 4-Stage Ingestion Pipeline

When an agent purchases a Signal, it passes through a Pipeline of Verify Cells (see [03-GRAPH.md](../../unified/03-GRAPH.md) for Pipeline pattern) before influencing reasoning:

### 6.1 Stage 1: Quarantine

Content isolated. Cannot influence active reasoning. Duration: 24 hours for new sellers (reputation < 0.50), 1 hour for Trusted+ sellers.

### 6.2 Stage 2: Validation

Format checks (valid SKILL.md?), domain matching (content matches claimed domain?), capability tests (procedure works?), deadlock detection (circular dependencies?).

### 6.3 Stage 3: Sandbox

Each procedure step tested in isolation. If the Signal claims "check gas price and defer if high," the sandbox verifies gas price checks return sensible values and deferral logic is sound.

### 6.4 Stage 4: Adoption

Content passing all stages is decomposed into individual Signals at discounted confidence:

```
adopted_confidence = original_confidence * discount_factor(seller_reputation)

discount_factor(R):
  R > 0.85 (Elite):      0.65
  R > 0.70 (Trusted):    0.55
  R > 0.50 (Standard):   0.45
  R <= 0.50 (Probation): 0.35
```

Adopted Signals start at Transient tier (lowest) and must prove themselves through actual use to be promoted to Working or Reference tiers via the standard demurrage and confirmation mechanisms.

---

## 7. Knowledge Futures Market

Knowledge Futures allow agents to pre-sell knowledge before producing it. Research agents publish commitments ("I will produce X by deadline Y"), operations agents purchase those commitments via x402, and the purchase funds the researcher's inference costs.

### 7.1 Future Structure

```rust
pub struct KnowledgeFuture {
    pub future_id: Blake3Hash,
    pub producer: u256,                  // passport ID
    pub title: String,
    pub domain: String,
    pub knowledge_type: KnowledgeKind,   // Insight, Heuristic, CausalLink, etc.
    pub expected_quality: f64,           // minimum promised quality
    pub delivery_deadline: u64,          // unix timestamp
    pub price_per_unit: u64,             // KORAI per access license
    pub max_buyers: u32,
    pub stake_amount: u64,               // KORAI staked as guarantee
    pub gate_requirements: Vec<GateType>,
}
```

### 7.2 Lifecycle

1. **Publication**: Research agent stakes KORAI, publishes future specification
2. **Purchase**: Buyers pay via x402; funds escrowed
3. **Delivery**: Producer submits Signal; Gate pipeline verifies quality >= expected
4. **Settlement**: Quality met -- escrow releases to producer, stake returned. Default -- stake slashed (100%), buyers refunded, reputation penalty -0.05

### 7.3 LMSR Prediction Market

Each Knowledge Future gets an automated market maker (Hanson 2003) that prices outcome shares:

```
cost(q) = b * ln(sum_i e^(q_i / b))

Two outcomes: Deliver (quality >= expected) or Default (failure)
```

```rust
pub struct LmsrMarketMaker {
    pub future_id: Blake3Hash,
    pub b: f64,                   // liquidity parameter (default 100.0)
    pub shares_deliver: f64,
    pub shares_default: f64,
    pub total_subsidy: f64,
}

impl LmsrMarketMaker {
    pub fn price_deliver(&self) -> f64 {
        let exp_d = (self.shares_deliver / self.b).exp();
        let exp_f = (self.shares_default / self.b).exp();
        exp_d / (exp_d + exp_f)
    }
}
```

Market price reveals collective belief about delivery probability:
- p_deliver = 0.85: high confidence in producer
- p_deliver = 0.40: market signals trouble; buyers seek alternatives
- Price drop mid-deadline: early warning system

Maximum market maker loss is bounded at b * ln(n) where n = number of outcomes.

---

## 8. SKILL.md Format

SKILL.md is the universal Signal serialization for interop (agentskills.io standard):

```markdown
---
name: optimal-gas-timing
description: Time DeFi transactions to minimize gas costs on Base L2
version: 2.1.0
author: roko-alpha-gen3
license: MIT
metadata:
  roko:
    tags: [DeFi, Gas, Optimization, Base]
    confidence: 0.82
    validated_count: 14
  pricing:
    base_price_usdc: "500000"  # $0.50
    royalty_bps: 500            # 5% to creator on resale
---

# Optimal gas timing for Base L2 transactions

## When to use
- Before executing any swap, LP operation, or vault deposit on Base L2

## Procedure
1. Check current Base L2 gas via `cast gas-price --rpc-url base`
2. Compare against 7-day median
3. If current > 3x median: defer to next cycle
4. If current < 0.5x median: execute all queued operations

## Pitfalls
- Gas prices on Base correlate with Ethereum L1 congestion, 10-30 minute lag
```

Conversion: Signal (internal, with HDC vectors, lineage, PAD state) to SKILL.md (universal, human-readable, framework-agnostic) during Dream consolidation phase. Reverse: purchased SKILL.md decomposed into individual Signals at discounted confidence.

---

## 9. Skill Categories

| Category | Typical Price | Verification | Decay Rate |
|---|---|---|---|
| Alpha signals | $1-10 | Required (3+ verifiers) | Fast (hours-days) |
| Strategy recipes | $0.50-5 | Recommended | Medium (days-weeks) |
| Heuristics | $0.10-1 | Optional | Slow (weeks-months) |
| Infrastructure guides | $0.05-0.50 | Optional | Very slow (months) |
| Anti-knowledge | $0.01-0.10 | Required | Varies |
| Research insights | $0.50-5 | Recommended | Slow |

Anti-knowledge ("what does not work") is explicitly typed and valued -- learning what not to do is often more efficient than full strategy-space exploration. Verification is required to prevent anti-knowledge griefing (false warnings to discourage competitors from profitable strategies).

---

## What This Enables

- **Knowledge as a liquid asset**: Signals become tradeable with price discovery, verification, and dispute resolution
- **Aligned incentives via futures**: Pre-sales guarantee researchers are compensated; market coordinates research allocation
- **Quality over quantity**: Alpha-decay pricing, prediction-backed metrics, and the ingestion pipeline filter noise
- **Cross-framework interop**: SKILL.md format means any agent system can consume Roko knowledge
- **Collective intelligence acceleration**: Futures market eliminates duplicate research and directs compute to highest-value topics

## Feedback Loops

1. **Price-quality Loop**: Higher-quality Signals earn more revenue, funding better inference, producing higher-quality Signals
2. **Reputation-pricing Loop**: Higher reputation enables higher pricing (via rep_mult), which attracts more buyers, which generates more feedback, which builds reputation
3. **Futures-allocation Loop**: Purchase volume signals value; more purchases for topic X attract more researchers to produce X, driving prices to marginal cost
4. **Prediction market Loop**: LMSR market prices correct against delivery outcomes, improving calibration of quality predictions

## Open Questions

1. **Information paradox** (Arrow 1962): Buyers cannot value knowledge without seeing it, but seeing it eliminates the need to buy. The ingestion pipeline and blind verification partially address this. Is it sufficient?
2. **Knowledge DRM**: Content is delivered in plaintext (no DRM). Is the economic design (reputation costs, micropayment amounts) sufficient to prevent systematic extraction without payment?
3. **Cross-domain pricing**: Should alpha-decay parameters be auto-tuned per domain, or manually set? cadCAD simulation could help, but domain-specific decay rates require domain expertise.
4. **Futures market depth**: With small buyer populations, LMSR markets may have insufficient liquidity. What is the minimum viable buyer count for useful price discovery?

## Implementation Tasks

1. **Define `MarketplaceListing`** Signal type in `crates/roko-core/src/types/`
2. **Implement `IngestionPipeline`** as a Pipeline Graph of 4 Verify Cells in `crates/roko-gate/src/`
3. **Implement `DynamicPricingEngine`** in `crates/roko-core/src/` or a new `crates/roko-marketplace/`
4. **Implement `LmsrMarketMaker`** for Knowledge Futures in same crate
5. **Wire blind verification** as a Verify Cell using HDC similarity from `crates/roko-primitives/`
6. **Implement SKILL.md serialization/deserialization** in `crates/roko-core/src/`
7. **Add marketplace listings** to Store with indexing for discovery queries in `crates/roko-fs/` or `crates/roko-neuro/`
8. **Add prediction-backed tracking** (`SkillEffectiveness`) into `crates/roko-learn/`

---

*Absorbs: `docs/14-identity-economy/05-knowledge-marketplace.md`, `docs/14-identity-economy/06-commerce-bazaar.md`, `docs/14-identity-economy/14-knowledge-futures-market.md`. On-chain escrow and settlement mechanics covered in [18-registries/06-payments-and-settlement.md](../18-registries/06-payments-and-settlement.md). This doc covers off-chain marketplace dynamics, pricing models, and quality gates.*
