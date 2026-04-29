# VCG Attention Auction

> Depth for [02-CELL.md](../../unified/02-CELL.md). How the Compose protocol allocates scarce context budget across competing bidder subsystems using a VCG mechanism — truthful, efficient, diagnostic.

---

## Overview

The Compose protocol (see [02-CELL.md](../../unified/02-CELL.md) S2.5) defines budget-constrained assembly of Signals into a single output. The `ComposeProtocol` trait accepts `Vec<ComposeBid>` and a `ComposeBudget`, producing a `ComposeResult` with accepted bidders, VCG payments, and token/cost totals.

This doc specifies the mechanism design behind that assembly: how bids are formed, how winners are selected, how payments are computed, and what the payments reveal diagnostically. The mechanism is the Vickrey-Clarke-Groves (VCG) auction — a proven allocation mechanism from economics (Vickrey 1961, Clarke 1971, Groves 1973) applied to the problem of dividing limited context-window tokens among competing cognitive subsystems.

Herbert Simon (1971): "A wealth of information creates a poverty of attention." The context window is the attention constraint. VCG solves who gets it.

---

## 1. The Allocation Problem

A 128K-token model with 28K reserved for output has ~100K tokens of input budget. Multiple subsystems produce candidate context Signals:

| Bidder Cell | What It Bids | Source |
|---|---|---|
| `TaskBidder` | Task description, acceptance criteria | Current plan task |
| `CodeBidder` | Relevant code files, definitions | roko-index / tree-sitter |
| `ResearchBidder` | Research findings, citations | roko-research artifacts |
| `EpisodeBidder` | Prior episodes on similar tasks | roko-learn episodes via Store |
| `HeuristicBidder` | Relevant heuristics with calibration | roko-neuro |
| `ToolBidder` | Tool documentation for enabled tools | MCP tool manifests |
| `SafetyBidder` | Safety constraints, contract terms | Agent contract YAML |
| `NeuroBidder` | Distilled knowledge entries | roko-neuro knowledge Store |

Each bidder is a **Score Cell** that produces `ComposeBid` values. The **Compose Cell** is the auctioneer that accepts all bids and returns the allocation.

Without a coordination mechanism, the loudest subsystem (most content) dominates the prompt — not because it is most valuable, but because it is most prolific. The VCG mechanism prevents this.

---

## 2. The Compose Graph

The VCG auction is expressed as a Compose Graph with typed edges:

```
                    +--> [TaskBidder]  ---+
                    +--> [CodeBidder]  ---+
                    +--> [EpisodeBidder] -+
[TaskSignal] -------+--> [NeuroBidder]  --+--> [AuctioneerCell] --> [ComposeResult]
                    +--> [SafetyBidder] --+        ^
                    +--> [ResearchBidder]-+        |
                    +--> [ToolBidder]  ---+        |
                    +--> [HeuristicBidder]+    [ComposeBudget]
```

Each bidder Score Cell:
1. Receives the current task Signal as input
2. Queries its backing Store for relevant Signals
3. Scores each Signal using Thompson sampling (see S4)
4. Emits `Vec<ComposeBid>` — one per candidate section

The auctioneer Compose Cell:
1. Collects all bids from all bidders
2. Solves the budget-constrained allocation (greedy knapsack)
3. Computes VCG payments (externality pricing)
4. Emits `ComposeResult` with the composed Signal, accepted bidders, and diagnostics

---

## 3. The Bid Formula

Each bidder computes its bid as:

```
bid(section) = expected_value * urgency * affect_weight
```

| Component | Definition | Range |
|---|---|---|
| `expected_value` | `track_record(section) * relevance(section)` — conditional gate pass rate when this section was included, times task-specific relevance score | [0.0, 1.0] |
| `urgency` | `1.0 + max(0, (deadline - now) / total_budget)^(-1)` — amplifies under time pressure | [1.0, ~5.0] |
| `affect_weight` | Daimon PAD modulation (see S5) | [0.7, 1.5] |

**Novelty attenuation** (from [02-CELL.md](../../unified/02-CELL.md) S2.5): `effective_value = stated_value * (1 / (1 + ln(freq)))`. Common boilerplate gradually loses bid strength, making room for novel context.

---

## 4. Learning Bidders (Thompson Sampling)

Each bidder maintains per-section Beta posteriors updated by gate outcomes:

```rust
/// A bidder subsystem that learns section values from gate outcomes.
/// Each section tracks Beta(alpha, beta) where alpha = gate passes
/// when included, beta = gate failures when included.
struct LearningBidder {
    section_betas: HashMap<String, BetaPosterior>,
    section_costs: HashMap<String, SectionCostStats>,
}

impl LearningBidder {
    /// Compute bid via Thompson sampling.
    /// Samples from Beta posterior for exploration — sections with
    /// uncertain value occasionally get high bids, ensuring the
    /// system discovers their true contribution.
    fn bid(&self, section_name: &str, relevance: f64) -> f64 {
        let posterior = self.section_betas
            .get(section_name)
            .unwrap_or(&BetaPosterior { alpha: 1.0, beta: 1.0 });
        let sampled_track_record = beta_sample(posterior.alpha, posterior.beta);
        sampled_track_record * relevance
    }

    /// Cost-aware bid: scales by historical cost-effectiveness.
    /// Cheap AND effective sections get higher bids.
    fn bid_with_cost(&self, section_name: &str, relevance: f64) -> f64 {
        let base_bid = self.bid(section_name, relevance);
        let cost_factor = self.cost_effectiveness_factor(section_name);
        base_bid * cost_factor // clamped to [0.5, 2.0]
    }

    /// Update posterior after observing task outcome + cost.
    fn update_with_cost(
        &mut self,
        section_name: &str,
        was_included: bool,
        gate_passed: bool,
        attributed_cost_usd: f64,
        estimated_tokens: usize,
    ) {
        // Update Beta posterior
        if was_included {
            let entry = self.section_betas
                .entry(section_name.into())
                .or_insert(BetaPosterior { alpha: 1.0, beta: 1.0 });
            if gate_passed { entry.alpha += 1.0; }
            else { entry.beta += 1.0; }
        }
        // Update cost tracking
        if was_included && estimated_tokens > 0 {
            let stats = self.section_costs
                .entry(section_name.into())
                .or_default();
            stats.total_cost_usd += attributed_cost_usd;
            stats.total_tokens += estimated_tokens;
            stats.observation_count += 1;
            if gate_passed { stats.passes += 1; }
        }
    }
}
```

Thompson sampling provides natural exploration: sections with wide Beta distributions (high uncertainty) occasionally receive high sampled values, ensuring the system discovers their true contribution. As observations accumulate, posteriors narrow and bids converge to true expected values.

Convergence: ~50-100 tasks per subsystem to stabilize bids (from MARL auction literature, arXiv:2402.19420).

---

## 5. Affect Modulation of Bids

The Daimon's PAD state (see [distributed-and-affect-composition.md](distributed-and-affect-composition.md)) modulates bid weights:

| PAD State | Modulation |
|---|---|
| High arousal (>= 0.35) | x1.3 for action-oriented (task, file, gate errors), x0.7 for exploratory (research, cross-plan) |
| Low pleasure (<= -0.35) | x1.5 for anti-patterns and warnings, x0.8 for standard content |
| Low dominance (<= -0.35) | x1.2 for explanatory (architecture docs, overviews), x0.9 for directive |
| Neutral | x1.0 (no modulation) |

This is a **Functor** pattern: the Daimon affect state acts as an endofunctor `F: ComposeBid -> ComposeBid` that enriches bids before the auctioneer sees them, without changing the Graph's topology.

---

## 6. VCG Allocation (Greedy Knapsack)

The auctioneer solves a 0/1 knapsack: maximize total bid value subject to token budget.

```
optimal = maximize sum(value_i * x_i)
          subject to sum(tokens_i * x_i) <= budget
          x_i in {0, 1}
```

For prompt assembly with N < 50 candidates, the greedy approximation (sort by value density = value/tokens, include in order) provides >= 50% of optimal welfare (Dantzig 1957) and in practice > 90%.

---

## 7. VCG Payments (Externality Pricing)

Each winner pays the externality they impose on others:

```
payment(section_i) = sum_{j != i} value_j(optimal_without_i)
                   - sum_{j != i} value_j(optimal_with_i)
```

In words: section i pays the difference between the total value others would receive if i were absent versus present. This is the "damage" that i's inclusion causes by consuming budget.

### Why Payments Matter (Even Without Money)

In a single-agent system, payments are **diagnostic, not financial**. A section with high payment is consuming disproportionate budget relative to its value. This signals:
- The section should be **compressed** (same value in fewer tokens)
- The section should be **split** (share the allocation)
- The budget should be **increased** for this tier

Payments provide a principled measure of budget pressure that manual priority tuning cannot.

### Properties

| Property | Meaning for Context Allocation |
|---|---|
| **Truthful** | Each bidder's optimal strategy is to bid true expected value. No gaming. |
| **Efficient** | Allocation maximizes total expected value across all bidders. |
| **Individually rational** | No bidder is worse off by participating. |
| **Budget balanced** (weakly) | Total payments <= total welfare generated. |

---

## 8. Auction Diagnostics

```rust
/// Diagnostics computed after each Compose execution.
/// Stored as metadata on the output Signal and appended
/// to .roko/learn/cost-attributions.jsonl for the calibration Loop.
struct AuctionDiagnostics {
    total_welfare: f64,          // sum of winning bid values
    total_payments: f64,         // sum of VCG payments
    welfare_loss: f64,           // estimated gap vs exact knapsack
    pareto_optimal: bool,        // no profitable swaps possible?
    highest_payment_sections: Vec<(String, f64)>,
    displaced_sections: Vec<(String, f64)>,
    budget_utilization: f64,     // tokens_used / tokens_available
    strategy: CompositionStrategy, // WeightedSum or Vcg
}
```

### Pareto Optimality Check

An allocation is Pareto optimal if no excluded section can be added without removing a section of equal or greater value:

```rust
fn is_pareto_optimal(
    included: &[SectionAllocation],
    excluded: &[SectionAllocation],
    budget_remaining: usize,
) -> bool {
    // Any excluded section that fits in remaining budget?
    for exc in excluded {
        if exc.tokens <= budget_remaining { return false; }
    }
    // Any profitable swap?
    for exc in excluded {
        for inc in included {
            if inc.value < exc.value && inc.tokens >= exc.tokens {
                return false;
            }
        }
    }
    true
}
```

---

## 9. Strategy Auto-Selection

The system transitions between strategies based on observation count:

```rust
enum CompositionStrategy {
    /// Fast greedy allocation by value density. No payments.
    /// Used during cold start (< 10 observations per bidder).
    WeightedSum,
    /// Full VCG with payments, externality tracking, diagnostics.
    /// Used once posteriors are informative (>= 10 observations).
    Vcg,
}

impl CompositionStrategy {
    fn auto_select(bidder_observations: &HashMap<BidderId, u32>) -> Self {
        let min_obs = bidder_observations.values().copied().min().unwrap_or(0);
        if min_obs >= 10 { Self::Vcg } else { Self::WeightedSum }
    }
}
```

The WeightedSum path is identical to the current `PromptComposer::compose()` behavior — sort by value density, greedy fill. The VCG path adds payment computation and diagnostic reporting without changing the allocation algorithm.

---

## 10. Cost Attribution Feedback Loop

The VCG mechanism closes the feedback loop between composition and learning:

```
PromptComposer::compose() emits CompositionManifest
    (included sections, token estimates, strategy, VCG payments)
        |
        v
Agent dispatches with composed prompt
    |
    v
Agent completes turn -> usage.input_tokens, cost_usd
    |
    v
CostAttribution::from_turn() distributes actual cost
    proportionally across included sections
        |
        v
Gate pipeline runs -> Verdict (pass/fail)
    |
    v
attribution.stamp_gate_result(gate_passed)
    |
    v
For each section: bidder.update_with_cost(
    section, included, passed, attributed_cost, tokens)
        |
        v
Next composition uses updated posteriors + cost factors
```

This is a **Loop** pattern (see [00-INDEX.md](../../unified/00-INDEX.md)): the Compose Cell's output feeds back (via gate Verdicts and cost data) into the Score Cells that generate the next round of bids.

### Persistence

```
.roko/learn/
  section-costs.json          # LearningBidder state with cost stats
  cost-attributions.jsonl     # Append-only log of CostAttribution records
  composition-strategy.json   # Current strategy + observation counts
```

---

## 11. Fairness Alternatives

VCG maximizes aggregate welfare. Alternative criteria for specific scenarios:

| Criterion | Formula | When To Use |
|---|---|---|
| **VCG (alpha=0)** | maximize total welfare | Default — best total outcome |
| **Proportional (alpha=1)** | allocation proportional to bid | Cold-start when bid accuracy is low |
| **Max-min (alpha->inf)** | maximize minimum allocation | Safety floor — guarantee safety subsystem always gets representation |

The recommended policy: **VCG + safety floor**. Reserve `safety_floor_tokens` (default: 200) for the SafetyBidder (guaranteed minimum), then run VCG on remaining budget.

```rust
struct FairnessConfig {
    alpha: f64,               // 0.0 = VCG, 1.0 = proportional, 10.0 ~ max-min
    safety_floor_tokens: u32, // default: 200
}
```

---

## 12. Mori-Diffs Reality

Per [09-COMPOSITION-AUCTION.md](../../mori-diffs/09-COMPOSITION-AUCTION.md):

**`vcg_allocate()` in `crates/roko-compose/src/auction.rs` (lines 293-414) is complete** — greedy-VCG with affect modulation, value-density sorting, externality payments, 5 unit tests. It is exported from `roko-compose::lib.rs` (line 49). It is **never invoked from any runtime path**.

**`PromptComposer::compose()` uses a value-density greedy path** that is structurally identical to VCG's allocation without payments. This means the system pays the maintenance cost of VCG infrastructure but gets none of the benefits.

**`LearningBidder` tracks (included, gate_passed) but ignores token cost entirely.** The `SectionEffectivenessRegistry` adjusts section priorities based on pass/fail but has no cost signal. A 2,000-token section with 60% pass rate is treated identically to a 200-token section with the same rate.

**The fix** (from the mori-diffs implementation plan):
1. Add `SectionCostStats` to `LearningBidder`
2. Add `CompositionStrategy` auto-selection
3. Emit `CompositionManifest` as metadata on composed Signal
4. Wire VCG path into `PromptComposer::compose()` when strategy is `Vcg`
5. Wire cost attribution into `orchestrate.rs` post-turn
6. Close the feedback loop: cost -> posteriors -> bids -> next auction

---

## What This Enables

- **Truthful allocation** — bidders cannot benefit from inflating their values; the mechanism incentivizes honest reporting of expected contribution.
- **Diagnostic budget pressure** — VCG payments reveal which sections consume disproportionate budget, guiding compression and budget reallocation.
- **Cost-aware learning** — sections that are cheap AND effective get higher bids over time, reducing total inference cost while maintaining quality.
- **Principled cold-to-warm transition** — WeightedSum bootstraps safely; VCG activates once posteriors are informative.

## Feedback Loops

1. **Bid Calibration Loop**: `gate Verdict -> update BetaPosterior -> next bid uses updated track record` (Loop pattern via `LearningBidder.update_with_cost`)
2. **Cost Attribution Loop**: `actual token spend -> proportional attribution -> cost_effectiveness_factor -> next bid scaled` (Loop pattern via `CostAttribution.from_turn`)
3. **Strategy Transition**: `observation_count crosses threshold -> auto_select switches WeightedSum -> Vcg` (Trigger)
4. **Diagnostic Feedback**: `AuctionDiagnostics -> TUI dashboard F4 (Learning tab) -> human inspects budget pressure -> adjusts config` (Observe protocol via Lens)

## Open Questions

1. **VCG activation threshold**: Is 10 observations per bidder the right threshold? Should it be configurable via `roko.toml`?
2. **Greedy vs exact knapsack**: For N < 20 candidates, exhaustive search is feasible and produces exact VCG payments. Should the auctioneer switch to exact when N is small?
3. **Cost attribution fidelity**: Token-proportional attribution ignores the "lost in the middle" attention effect — middle sections may consume tokens but receive lower model attention. Should attribution weight by position?
4. **Bid correlation detection**: Two bidders with correlated inputs (e.g., NeuroBidder and HeuristicBidder sharing knowledge Store) may exhibit collusion-like bid patterns. The mori-diffs mention Pearson correlation detection at r > 0.85 — is this implemented?
5. **VCG revenue non-monotonicity**: Adding more candidates can decrease total payments. This is theoretically known but may confuse the diagnostic dashboard. Should payments be smoothed?

---

## References

- Vickrey (1961), Counterspeculation, Auctions, and Competitive Sealed Tenders
- Clarke (1971), Multipart Pricing of Public Goods
- Groves (1973), Incentives in Teams
- Simon (1971), Designing Organizations for an Information-Rich World
- Dantzig (1957), Greedy Knapsack Approximation
- Duetting et al. (2024), Mechanism Design for Large Language Models, WWW Best Paper, arXiv:2310.10826
- Zhu et al. (2025), Regularized Proportional Fairness, ICLR, arXiv:2501.01111
- MIT CEEPR (2023), Learning in Repeated Multi-Unit Auctions, WP 2023-18
- arXiv:2402.19420 (2024), Understanding Iterative Combinatorial Auction Designs via MARL
- Nisan & Ronen, Computationally Feasible VCG Mechanisms
