# Attention as Universal Cognitive Currency

> **Abstract:** Attention is Roko's scarce resource — every perception, composition, inference,
> and verification step costs attention tokens drawn from a finite pool. This document unifies
> three previously disjoint mechanisms (VCG attention auction, CascadeRouter model selection,
> and budget management) into a single coherent economy where attention tokens are the universal
> unit of account. It also treats durable memory as a separate ledger: attention tokens spend in
> the loop, while demurrage taxes idle Engram balance between loops so stale knowledge does not
> keep its seat for free. The result is an attention market that enables principled,
> incentive-compatible allocation of cognitive resources across competing goals, agents, and
> timescales.

> **Implementation**: Specified

**Topic**: [00-architecture](./INDEX.md)
**Prerequisites**: [08-scorer-gate-router-composer-policy](./08-scorer-gate-router-composer-policy.md), [09-universal-cognitive-loop](./09-universal-cognitive-loop.md), [10-three-cognitive-speeds](./10-three-cognitive-speeds.md)
**Key sources**:
- Duetting et al. 2024, WWW — Mechanism Design for Large Language Models
- arXiv:2504.14824 (2025) — Enhanced Dual-Currency VCG Auction for Multi-Agent Resource Allocation
- arXiv:2407.01548 (2024) — From Cognition to Computation: Human Attention vs. Transformer Architectures
- arXiv:2310.05746 (2024) — AucArena: Strategic Planning and Execution of LLM Agents in Auctions
- Chen et al. 2023, arXiv:2305.05176 — FrugalGPT: LLM cascade optimization
- Kahneman 1973, "Attention and Effort" — Attention as limited cognitive resource

---

## 1. The Problem: Three Disconnected Resource Systems

Roko currently manages loop-time cognitive resources through three independent mechanisms, plus
a separate memory-side ledger:

1. **CascadeRouter** (`roko-learn`): Selects among T0/T1/T2 inference tiers using LinUCB bandits.
   Each tier has a different cost (T0 ≈ 0 tokens, T1 ≈ 2K tokens, T2 ≈ 32K tokens). The router
   optimizes for cost-adjusted quality but has no awareness of global budget constraints.

2. **Composer budget** (`roko-compose`): The `Budget` struct caps context window size per
   composition. Each `compose()` call operates independently — there is no cross-tick budget
   accounting.

3. **VCG Attention Auction** (specified in [17-design-principles-and-frontier-summary](./17-design-principles-and-frontier-summary.md)):
   Designed to allocate "attention slots" among competing Engrams, but never wired to the
   actual Router or Composer.

4. **Neuro demurrage** (see [04-decay-variants](./04-decay-variants.md),
   [18-decay-tier-matrix](./18-decay-tier-matrix.md), [Topic 06: Neuro](../06-neuro/INDEX.md),
   and [tmp/refinements/12-knowledge-demurrage.md](../../tmp/refinements/12-knowledge-demurrage.md)):
   Durable Engrams carry `balance`, a holding-cost ledger that decays when memory sits idle and
   is reinforced by use, citation, retrieval, or surprise.

These mechanisms make locally rational decisions that are globally incoherent. An agent
might cascade to T2 for every tick (CascadeRouter thinks it's optimal), while the budget is
exhausted by low-priority context (Composer has no notion of priority), and the VCG auction
sits disconnected from both.

That is only half the economy, though. **Attention tokens** govern in-loop spending; **balance**
governs whether durable knowledge remains economically justified between loops.

---

## 2. The Attention Token Model

### 2.1 Core Abstraction

```rust
/// A single unit of cognitive attention. Dimensionless, fungible.
/// 1 attention token ≈ cost of 1 T0 probe tick.
///
/// Exchange rates:
///   T0 probe   = 1 AT
///   T1 fast    = 200 AT
///   T2 full    = 3200 AT
///   Context KB = 10 AT per KB
///   Gate eval  = 50 AT per gate
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct AttentionToken(f64);

impl AttentionToken {
    pub const ZERO: Self = Self(0.0);
    pub const T0_COST: Self = Self(1.0);
    pub const T1_COST: Self = Self(200.0);
    pub const T2_COST: Self = Self(3200.0);
    pub const CONTEXT_PER_KB: Self = Self(10.0);
    pub const GATE_COST: Self = Self(50.0);

    pub fn new(amount: f64) -> Self {
        Self(amount.max(0.0))
    }

    pub fn value(&self) -> f64 {
        self.0
    }

    /// Spend tokens from a pool; returns None if insufficient.
    pub fn spend(pool: &mut Self, cost: Self) -> Option<Self> {
        if pool.0 >= cost.0 {
            pool.0 -= cost.0;
            Some(cost)
        } else {
            None
        }
    }
}
```

### 2.2 Budget Pools

Attention tokens are allocated in hierarchical pools that mirror Roko's three cognitive speeds:

```rust
/// Hierarchical attention budget for a single agent session.
pub struct AttentionBudget {
    /// Total session budget (replenished per Delta cycle).
    pub session_total: AttentionToken,
    /// Remaining session tokens.
    pub session_remaining: AttentionToken,

    /// Gamma-speed budget (per-tick cap).
    pub gamma_cap: AttentionToken,
    /// Theta-speed budget (per-reflection cap).
    pub theta_cap: AttentionToken,
    /// Delta-speed budget (per-consolidation cap).
    pub delta_cap: AttentionToken,

    /// Rollover fraction: what % of unspent Gamma tokens carry to next tick.
    pub rollover_fraction: f64,  // default 0.1 (10%)

    /// Emergency reserve: fraction of session budget held back for critical operations.
    pub emergency_reserve: f64,  // default 0.15 (15%)
}

impl Default for AttentionBudget {
    fn default() -> Self {
        Self {
            session_total: AttentionToken::new(100_000.0),
            session_remaining: AttentionToken::new(100_000.0),
            gamma_cap: AttentionToken::new(500.0),
            theta_cap: AttentionToken::new(5_000.0),
            delta_cap: AttentionToken::new(30_000.0),
            rollover_fraction: 0.1,
            emergency_reserve: 0.15,
        }
    }
}
```

### 2.3 The Memory Ledger

Attention tokens buy compute inside a tick. Demurrage taxes the right to keep a durable Engram
warm after the tick ends.

```text
balance(t + Δt) = balance(t) - r·Δt - β·balance(t)·Δt + reinforcement
```

The `reinforcement` term comes from reads, citations, successful gates, and surprise. In the
memory layer, `balance` is not interchangeable with the live attention pool: a session can be
well-budgeted and still be fed by a petrified memory base if stale Engrams never pay a holding
cost.

| Ledger | Charged when | Governs | Example |
|---|---|---|---|
| Attention budget | During the loop | Router, Composer, Gate, Policy spend | A T2 inference burns session tokens |
| Demurrage balance | Between loops | Durable memory residency | A stale playbook slowly loses balance |

See also [04-decay-variants](./04-decay-variants.md), [18-decay-tier-matrix](./18-decay-tier-matrix.md), [Topic 06: Neuro](../06-neuro/INDEX.md), and [tmp/refinements/12-knowledge-demurrage.md](../../tmp/refinements/12-knowledge-demurrage.md).

### 2.4 Token Flow Per Cognitive Loop Tick

```
┌──────────────────────────────────────────────┐
│              Session Budget Pool             │
│            (100,000 AT default)              │
└──────────┬───────────────────────────────────┘
           │ allocate per-speed caps
    ┌──────┴──────┬───────────────┐
    ▼             ▼               ▼
┌────────┐  ┌──────────┐  ┌───────────┐
│ Gamma  │  │  Theta   │  │   Delta   │
│ 500 AT │  │ 5000 AT  │  │ 30000 AT  │
└───┬────┘  └────┬─────┘  └─────┬─────┘
    │            │              │
    ▼            ▼              ▼
 ┌─────────────────────────────────────────┐
 │       VCG Attention Auction             │
 │  Engrams bid for attention slots        │
 │  Winners consume AT from speed pool     │
 └─────────────────────────────────────────┘
    │            │              │
    ▼            ▼              ▼
 ┌────────┐ ┌────────┐  ┌───────────┐
 │Compose │ │Cascade │  │   Gate    │
 │Context │ │ Route  │  │  Verify   │
 │ 10/KB  │ │T0-T2   │  │  50/gate  │
 └────────┘ └────────┘  └───────────┘
```

---

## 3. The VCG Attention Auction

### 3.1 Mechanism Design

The VCG (Vickrey-Clarke-Groves) auction allocates attention slots to Engrams competing for
inclusion in the cognitive loop. Each Engram "bids" based on its Score; the auction selects
winners and charges them the externality they impose on others — ensuring truthful bidding
is the dominant strategy.

```rust
/// An attention auction that allocates K slots among N competing Engrams.
///
/// Properties (Duetting et al. 2024):
///   - DSIC: dominant-strategy incentive compatible
///   - Individually rational: no Engram is worse off for participating
///   - Allocatively efficient: maximizes total attention value
pub struct AttentionAuction {
    /// Number of attention slots available this tick.
    pub slots: usize,
    /// Minimum bid (Score.effective) to participate.
    pub reserve_price: f64,
    /// Maximum fraction of budget any single Engram can consume.
    pub max_bid_fraction: f64,  // default 0.3
}

/// A bid in the attention auction.
pub struct AttentionBid {
    /// The Engram competing for attention.
    pub engram_hash: ContentHash,
    /// Bid value derived from Score.effective() × urgency_multiplier.
    pub bid_value: f64,
    /// Estimated attention cost if this Engram wins (context size + processing).
    pub estimated_cost: AttentionToken,
    /// Priority class: Critical > High > Normal > Background.
    pub priority: AttentionPriority,
}

/// Auction result.
pub struct AuctionOutcome {
    /// Winning Engrams in priority order.
    pub winners: Vec<AuctionWinner>,
    /// Total attention tokens committed.
    pub total_cost: AttentionToken,
    /// Engrams that bid but lost.
    pub rejected: Vec<ContentHash>,
    /// Revenue (VCG payments) — recycled into emergency reserve.
    pub vcg_revenue: AttentionToken,
}

pub struct AuctionWinner {
    pub engram_hash: ContentHash,
    pub slot: usize,
    pub bid_value: f64,
    /// VCG payment = externality imposed on others.
    /// Always ≤ bid_value (individual rationality).
    pub vcg_payment: AttentionToken,
}
```

### 3.2 VCG Payment Computation

```rust
impl AttentionAuction {
    /// Run the VCG auction.
    ///
    /// Algorithm:
    /// 1. Sort bids by bid_value descending
    /// 2. Select top-K as winners (subject to budget and reserve price)
    /// 3. For each winner i, compute VCG payment:
    ///    payment_i = (sum of top-K bids without i) - (sum of top-K bids with i, excluding i's bid)
    ///    Equivalently: payment_i = bid_{K+1} (the first excluded bid)
    ///    when slots are homogeneous.
    /// 4. Charge each winner their VCG payment in attention tokens
    pub fn run(
        &self,
        bids: &mut [AttentionBid],
        budget: &mut AttentionToken,
    ) -> AuctionOutcome {
        // Sort by bid value descending
        bids.sort_by(|a, b| b.bid_value.partial_cmp(&a.bid_value).unwrap());

        let mut winners = Vec::with_capacity(self.slots);
        let mut total_cost = AttentionToken::ZERO;
        let mut rejected = Vec::new();

        for (i, bid) in bids.iter().enumerate() {
            if winners.len() >= self.slots {
                rejected.push(bid.engram_hash);
                continue;
            }
            if bid.bid_value < self.reserve_price {
                rejected.push(bid.engram_hash);
                continue;
            }
            if bid.estimated_cost.value() > budget.value() * self.max_bid_fraction {
                rejected.push(bid.engram_hash);
                continue;
            }

            // VCG payment: the (K+1)th highest bid, or reserve price if fewer bids
            let vcg_payment_value = bids
                .get(self.slots)
                .map(|b| b.bid_value)
                .unwrap_or(self.reserve_price);
            let vcg_payment = AttentionToken::new(
                vcg_payment_value * bid.estimated_cost.value()
                    / bid.bid_value.max(f64::EPSILON),
            );

            if AttentionToken::spend(budget, bid.estimated_cost).is_some() {
                winners.push(AuctionWinner {
                    engram_hash: bid.engram_hash,
                    slot: i,
                    bid_value: bid.bid_value,
                    vcg_payment,
                });
                total_cost = AttentionToken::new(
                    total_cost.value() + bid.estimated_cost.value(),
                );
            } else {
                rejected.push(bid.engram_hash);
            }
        }

        let vcg_revenue = AttentionToken::new(
            winners.iter().map(|w| w.vcg_payment.value()).sum(),
        );

        AuctionOutcome { winners, total_cost, rejected, vcg_revenue }
    }
}
```

### 3.3 Priority Classes

```rust
/// Priority classes modulate the auction reserve price.
/// Critical items get reduced reserve prices (easier to win).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AttentionPriority {
    /// Background: speculative, non-urgent. Reserve price × 2.0.
    Background = 0,
    /// Normal: standard cognitive loop operations. Reserve price × 1.0.
    Normal = 1,
    /// High: time-sensitive or high-value. Reserve price × 0.5.
    High = 2,
    /// Critical: safety-relevant or deadline-bound. Reserve price × 0.1.
    Critical = 3,
}
```

---

## 4. Unifying CascadeRouter with Attention Tokens

### 4.1 The CascadeRouter as Attention Spender

The CascadeRouter currently selects T0/T1/T2 based on quality estimates from LinUCB bandits.
Under the attention economy, the router becomes a **budget-aware spender**: it must purchase
inference from the attention pool, and the pool constrains its choices.

```rust
/// Attention-aware cascade routing.
///
/// The router observes the remaining attention budget and adjusts
/// tier selection accordingly. When budget is plentiful, it routes
/// aggressively to T2 for maximum quality. When budget is scarce,
/// it conserves by staying at T0/T1.
pub struct AttentionCascadeRouter {
    /// Inner LinUCB bandit for quality estimation.
    pub bandit: LinUCBRouter,
    /// Tier costs in attention tokens.
    pub tier_costs: [AttentionToken; 3],  // [T0, T1, T2]
    /// Budget pressure threshold: below this fraction, prefer cheaper tiers.
    pub pressure_threshold: f64,  // default 0.3 (30% remaining)
    /// Quality discount factor under pressure: how much quality to sacrifice.
    pub pressure_discount: f64,   // default 0.6
}

impl AttentionCascadeRouter {
    /// Select tier given current budget state.
    ///
    /// Returns (selected_tier, cost) or None if budget is exhausted.
    pub fn select_tier(
        &self,
        context: &RouterContext,
        budget: &AttentionBudget,
    ) -> Option<(InferenceTier, AttentionToken)> {
        let remaining_fraction = budget.session_remaining.value()
            / budget.session_total.value().max(f64::EPSILON);

        // Under pressure: discount quality estimates for expensive tiers
        let quality_estimates = self.bandit.estimate_all(context);
        let adjusted: Vec<(InferenceTier, f64, AttentionToken)> = quality_estimates
            .iter()
            .enumerate()
            .map(|(i, &quality)| {
                let tier = InferenceTier::from_index(i);
                let cost = self.tier_costs[i];
                let adj_quality = if remaining_fraction < self.pressure_threshold {
                    // Under budget pressure: penalize expensive tiers
                    let cost_penalty = cost.value() / self.tier_costs[2].value();
                    quality * (1.0 - cost_penalty * (1.0 - self.pressure_discount))
                } else {
                    quality
                };
                (tier, adj_quality, cost)
            })
            .collect();

        // Select highest adjusted quality that fits budget
        adjusted
            .into_iter()
            .filter(|(_, _, cost)| cost.value() <= budget.session_remaining.value())
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .map(|(tier, _, cost)| (tier, cost))
    }
}
```

### 4.2 Budget Pressure Curve

Budget pressure follows a sigmoid to avoid sharp transitions:

```
Pressure(r) = 1 / (1 + exp(10 * (r - threshold)))

where r = remaining_fraction, threshold = 0.3

Pressure = 0.0  when budget is plentiful (r >> 0.3)
Pressure = 0.5  when budget hits threshold (r = 0.3)
Pressure = 1.0  when budget is near-exhausted (r << 0.3)
```

---

## 5. Composer Budget Integration

### 5.1 Context Window as Attention Consumer

Under the attention economy, every KB of context assembled by the Composer costs attention
tokens. This creates a natural pressure toward concise, relevant prompts.

```rust
/// Attention-aware composition budget.
pub struct AttentionComposerBudget {
    /// Maximum context window in tokens (model-dependent).
    pub max_context_tokens: usize,
    /// Attention cost per KB of assembled context.
    pub cost_per_kb: AttentionToken,  // default: 10 AT/KB
    /// Premium multiplier for low-relevance context (discourages padding).
    pub low_relevance_premium: f64,   // default: 2.0 (Score < 0.3 costs 2× AT)
    /// Discount for cached/reused context (encourages knowledge reuse).
    pub cache_discount: f64,          // default: 0.5 (cached context costs 0.5×)
}
```

### 5.2 Incentive Alignment

The pricing structure creates aligned incentives:

| Behavior | AT Cost | Incentive |
|---|---|---|
| Include high-relevance Engram (Score > 0.7) | 10 AT/KB | Neutral — pay base rate |
| Include low-relevance Engram (Score < 0.3) | 20 AT/KB | Discouraged — pay premium |
| Reuse cached context from previous tick | 5 AT/KB | Encouraged — pay discount |
| Include AntiKnowledge (known-false) | 0 AT | Free — always include to prevent re-exploration |
| Exceed 80% of context window | 1.5× multiplier | Discouraged — marginal cost increases |

---

## 6. Cross-Speed Token Economics

### 6.1 The Delta Dividend

During Delta (consolidation) cycles, the system performs knowledge compression and strategy
distillation. Effective consolidation reduces future attention costs by producing higher-quality
heuristics and more relevant knowledge.

```rust
/// Delta consolidation produces an "attention dividend" —
/// future ticks cost less because knowledge is better organized.
pub struct DeltaDividend {
    /// Knowledge compression ratio achieved (e.g., 0.7 = 30% reduction).
    pub compression_ratio: f64,
    /// Number of Transient→Working promotions (reduce future search cost).
    pub promotions: usize,
    /// Number of AntiKnowledge entries created (prevent future wasted attention).
    pub anti_knowledge_created: usize,
    /// Estimated future AT savings per Gamma tick.
    pub projected_savings_per_tick: AttentionToken,
}
```

### 6.2 Theta Budget Arbitrage

Theta (reflective) ticks are more expensive per-tick but can save total attention by catching
strategic errors early. The system tracks the **arbitrage ratio**:

```
theta_arbitrage = (gamma_AT_saved_by_replan) / (theta_AT_spent)

If theta_arbitrage > 1.0: Theta reflection is net-positive. Increase Theta frequency.
If theta_arbitrage < 0.5: Theta reflection is wasteful. Decrease Theta frequency.
```

### 6.3 Delta Dividend vs Demurrage

Delta consolidation and demurrage act on different sides of the ledger. The Delta dividend
reduces future loop spend by compressing and distilling knowledge; demurrage reduces the amount
of stale durable memory that can claim future attention at all.

That distinction matters operationally. A cheaper Router does not fix a bloated memory base, and
more aggressive Composer budgeting does not prevent old Engrams from ossifying. Only the memory
ledger can do that, which is why the demurrage rules live with Neuro and the decay/tier docs
rather than inside the attention budget itself.

---

## 7. Daimon Modulation of Attention Allocation

The Daimon (affect subsystem) modulates attention allocation based on the agent's affective
state, implementing Kahneman's (1973) resource theory of attention:

```rust
/// Affect-driven attention modulation.
///
/// PAD dimensions (Pleasure-Arousal-Dominance) modulate attention:
///   - High Arousal → broader attention (lower reserve price, more slots)
///   - High Dominance → more aggressive tier selection (prefer T2)
///   - Low Pleasure → risk-averse spending (increase emergency reserve)
pub struct DaimonAttentionModulator {
    /// Arousal coefficient: how much arousal widens attention.
    pub arousal_slot_bonus: f64,      // default: 0.3 (30% more slots at max arousal)
    /// Dominance coefficient: how much dominance shifts tier preference.
    pub dominance_tier_shift: f64,    // default: 0.2 (20% more T2 preference at max dominance)
    /// Displeasure reserve coefficient: how much displeasure increases reserve.
    pub displeasure_reserve: f64,     // default: 0.1 (10% more emergency reserve at min pleasure)
}

impl DaimonAttentionModulator {
    pub fn modulate(&self, budget: &mut AttentionBudget, pad: &PadVector) {
        // Arousal widens attention: more auction slots
        let arousal_factor = 1.0 + self.arousal_slot_bonus * pad.arousal.clamp(-1.0, 1.0);

        // Low pleasure → conserve resources
        let pleasure_reserve = if pad.pleasure < 0.0 {
            self.displeasure_reserve * pad.pleasure.abs()
        } else {
            0.0
        };
        budget.emergency_reserve = (budget.emergency_reserve + pleasure_reserve).min(0.5);

        // Dominance → aggressive routing (handled by CascadeRouter pressure_discount)
        // Passed through context, not mutated here.
    }
}
```

---

## 8. Configuration

```toml
[attention]
# Total session budget in attention tokens.
session_budget = 100_000

# Per-speed caps.
gamma_cap = 500
theta_cap = 5_000
delta_cap = 30_000

# Rollover fraction (unspent Gamma tokens carried to next tick).
rollover_fraction = 0.1

# Emergency reserve fraction.
emergency_reserve = 0.15

[attention.auction]
# Number of attention slots per Gamma tick.
gamma_slots = 8
# Number of attention slots per Theta tick.
theta_slots = 16
# Reserve price (minimum Score.effective to compete).
reserve_price = 0.05
# Max fraction of budget any single Engram can consume.
max_bid_fraction = 0.3

[attention.cascade]
# Budget pressure threshold (fraction remaining triggers conservation).
pressure_threshold = 0.3
# Quality discount under pressure.
pressure_discount = 0.6

[attention.composer]
# Attention cost per KB of context.
cost_per_kb = 10.0
# Premium for low-relevance context.
low_relevance_premium = 2.0
# Discount for cached context.
cache_discount = 0.5
```

---

## 9. Integration Wiring

### 9.1 Into the Universal Cognitive Loop

The attention economy wires into every step of the 9-step loop:

| Loop Step | Attention Integration |
|---|---|
| 1. PERCEIVE | Query returns candidates → they become auction bidders |
| 2. EVALUATE | Score.effective() → bid value |
| 3. ATTEND | **VCG Auction** selects winners, charges AT |
| 4. INTEGRATE | Composer draws AT per KB assembled |
| 5. ACT | **CascadeRouter** draws AT per tier selected |
| 6. VERIFY | Gate draws AT per gate evaluation |
| 7. PERSIST | No AT cost in-loop; durable memory is taxed separately by demurrage |
| 8. ADAPT | Policy observes AT expenditure, adjusts future budgets |
| 9. META-COGNIZE | Daimon modulates next tick's AT allocation |

This table covers loop-time attention spend only. Demurrage lives on the durable-memory ledger
and is charged between loops by Neuro, so it does not appear as a Router, Composer, or Gate line
item.

### 9.2 Into Existing Crates

| Crate | Integration Point | Change |
|---|---|---|
| `roko-core` | `Context` struct | Add `attention_budget: AttentionBudget` field |
| `roko-learn` | `CascadeRouter` | Wrap in `AttentionCascadeRouter` |
| `roko-compose` | `Budget` struct | Add `AttentionComposerBudget` alongside token budget |
| `roko-orchestrator` | `loop_tick()` | Insert auction before `Router.select()`, deduct AT at each step |
| `roko-daimon` | `PadVector` consumers | Add `DaimonAttentionModulator` called in step 9 |
| `roko-learn` | `EpisodeLogger` | Log AT expenditure per tick for learning |
| `roko-conductor` | Circuit breaker | Trigger if AT burn rate exceeds 3× expected |

---

## 10. Observability and Dashboard Implications

If the dashboard only shows attention burn, it can miss the real failure mode: a healthy-looking
tick budget with an unhealthy, over-retained memory base. The operator needs both ledgers on the
same surface.

```rust
/// Per-tick attention telemetry, logged to .roko/learn/attention.jsonl.
#[derive(Serialize, Deserialize)]
pub struct AttentionTelemetry {
    pub tick_id: u64,
    pub speed: CognitiveSpeed,
    pub budget_before: f64,
    pub budget_after: f64,
    pub auction_bids: usize,
    pub auction_winners: usize,
    pub vcg_revenue: f64,
    pub cascade_tier: InferenceTier,
    pub cascade_cost: f64,
    pub composer_context_kb: f64,
    pub composer_cost: f64,
    pub gate_cost: f64,
    pub total_spent: f64,
    pub pressure: f64,
    pub pad_modulation: [f64; 3],  // [pleasure, arousal, dominance]
    pub demurrage_balance_total: f64,
    pub demurrage_paid_total: f64,
    pub reinforcement_events: usize,
    pub thaw_events: usize,
}
```

### 10.1 Dashboard Surfaces

- **Spend burn**: how quickly the live attention pool is being consumed per tick.
- **Balance distribution**: how much durable memory is sitting warm versus drifting cold.
- **Reinforcement-by-kind**: whether citation, retrieval, gate success, or surprise is keeping
  knowledge alive.
- **Thaw rate**: how often cold Engrams return to the warm path, which indicates whether the
  demurrage curve is too steep.
- **Attention leaderboard**: the highest-balance Engrams, useful for spotting hoarding or
  over-consolidation.

These tiles should sit next to the Router and Composer spend charts, not behind a separate
Neuro-only view. Otherwise the system can look budget-disciplined while still accumulating stale
knowledge that never pays its holding cost.

---

## 11. Test Criteria

| Test | What It Validates | Type |
|---|---|---|
| `test_vcg_payment_truthful` | VCG payments make truthful bidding dominant | Unit |
| `test_vcg_individual_rationality` | No winner pays more than their bid | Unit |
| `test_budget_exhaustion_graceful` | When AT = 0, tick completes with T0 only | Integration |
| `test_pressure_shifts_to_cheap_tier` | Below threshold, router prefers T0/T1 | Unit |
| `test_rollover_fraction` | Unspent Gamma AT partially carries to next tick | Unit |
| `test_emergency_reserve_locked` | Emergency reserve not spent by normal operations | Unit |
| `test_delta_dividend_reduces_future_cost` | After consolidation, average tick AT decreases | Integration |
| `test_daimon_high_arousal_more_slots` | High arousal increases auction slots | Unit |
| `test_displeasure_increases_reserve` | Negative pleasure increases emergency reserve | Unit |
| `test_low_relevance_premium_applied` | Score < 0.3 Engrams cost 2× AT in composer | Unit |
| `test_cache_discount_applied` | Cached context costs 0.5× AT | Unit |
| `test_telemetry_logged_per_tick` | AttentionTelemetry written every tick | Integration |
| `test_circuit_breaker_on_overspend` | AT burn > 3× triggers conductor circuit breaker | Integration |
| `test_demurrage_reduces_idle_balance` | Idle durable Engrams lose balance between loops | Unit |
| `test_reinforcement_refunds_balance` | Citation, retrieval, or surprise increases balance | Unit |
| `test_zero_balance_thaws_to_cold_tier` | Balance floor moves memory into cold storage | Integration |

---

## 12. Theoretical Foundations

### 12.1 VCG Auction Theory

The Vickrey-Clarke-Groves mechanism (Vickrey 1961, Clarke 1971, Groves 1973) is the unique
mechanism that is simultaneously dominant-strategy incentive compatible (DSIC), allocatively
efficient, and individually rational. Applied to cognitive attention:

- **DSIC**: Each Engram's optimal strategy is to bid its true Score. No Engram benefits from
  inflating or deflating its bid.
- **Allocative efficiency**: The winners are the K highest-value Engrams. Total attention
  value is maximized.
- **Individual rationality**: No winner pays more than their bid. Participating in the auction
  is never worse than not participating.

Duetting et al. (2024) extended VCG to multi-agent LLM settings, proving that token auctions
maintain DSIC even when agents are LLMs with strategic capabilities. arXiv:2504.14824 (2025)
further showed dual-currency VCG with MFMARL reduces collusion risk in distributed settings.

### 12.2 Resource Theory of Attention (Kahneman 1973)

Kahneman's resource theory models attention as a finite pool shared among concurrent processes.
Key insights mapped to Roko:

| Kahneman Insight | Roko Mapping |
|---|---|
| Attention pool replenishes slowly | Session budget replenished per Delta cycle |
| Arousal increases pool size | Daimon arousal → more auction slots |
| Difficult tasks require more attention | T2 costs 16× T1, reflecting cognitive difficulty |
| Automaticity reduces attention cost | T0 probes cost 1 AT (habitual processing) |

### 12.3 FrugalGPT Connection

Chen et al. (2023) introduced FrugalGPT — cascading from cheap to expensive models based on
confidence. Roko's `AttentionCascadeRouter` extends FrugalGPT with three innovations:

1. **Budget pressure** — FrugalGPT routes based on quality alone; Roko routes based on
   quality × budget state.
2. **Affect modulation** — FrugalGPT is affectless; Roko's Daimon shifts routing under stress.
3. **VCG pricing** — FrugalGPT uses ad hoc cascading; Roko uses mechanism design to ensure
   incentive compatibility when multiple goals compete for the same budget.

---

## 13. Open Questions

1. **Inter-agent AT markets**: When multiple agents share a budget, should they trade AT among
   themselves via a continuous double auction? Or should a central allocator distribute AT?
2. **AT inflation/deflation**: As the system learns and becomes more efficient, should AT
   values be reindexed to maintain stable prices?
3. **Credit assignment**: When a Gamma tick's output is only valuable because of a prior Theta
   reflection, how should the AT "profit" be attributed?
4. **Fairness constraints**: Should the VCG auction enforce a minimum AT allocation per
   knowledge type (e.g., AntiKnowledge always gets at least 1 slot)?

---

## Cross-References

- [08-scorer-gate-router-composer-policy](./08-scorer-gate-router-composer-policy.md) — Trait specs that the attention economy wraps
- [09-universal-cognitive-loop](./09-universal-cognitive-loop.md) — The 9-step loop that attention tokens flow through
- [10-three-cognitive-speeds](./10-three-cognitive-speeds.md) — Gamma/Theta/Delta speed pools
- [04-decay-variants](./04-decay-variants.md) — Demurrage as the memory-side holding cost over decay
- [18-decay-tier-matrix](./18-decay-tier-matrix.md) — Tier promotion, cold storage, and thaw rules
- [Topic 06: Neuro](../06-neuro/INDEX.md) — Durable knowledge, tiering, and reinforcement
- [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md) — Daimon affect modulation
- [29-cognitive-energy-model](./29-cognitive-energy-model.md) — Energy pools that replenish AT budgets
- [Topic 05: Learning](../05-learning/INDEX.md) — CascadeRouter and bandit optimization
- [Topic 16: Heartbeat](../16-heartbeat/INDEX.md) — CoALA 9-step pipeline integration
- [tmp/refinements/12-knowledge-demurrage.md](../../tmp/refinements/12-knowledge-demurrage.md) — Full demurrage proposal behind this chapter update
