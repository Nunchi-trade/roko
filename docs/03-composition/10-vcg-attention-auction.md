# 10 — VCG Attention Auction: Mechanism Design for Context Allocation

> Layer 2 Scaffold — Synapse Architecture
> Status: **Design** — Specified in PRD, not yet implemented
> Canonical sources: `refactoring-prd/09-innovations.md` §II, §XIX.E


> **Implementation**: Shipping

---

## Abstract

The VCG (Vickrey-Clarke-Groves) attention auction applies mechanism design to the problem of allocating the scarce context window among competing cognitive subsystems. Each subsystem (e.g., episodic memory, knowledge store, task context, safety constraints) bids for attention bandwidth based on its expected contribution to task success. The VCG mechanism ensures truthful bidding (no subsystem benefits from misrepresenting its value) and efficient allocation (the combination that maximizes total value wins). Winners pay the externality they impose on others (second-price), not their own bid.

This document specifies the VCG mechanism, the bid formula, the eight bidding subsystems, the payment rule, and the relationship to active inference.

---

## 1. The Attention Allocation Problem

Herbert Simon (1971): "A wealth of information creates a poverty of attention."

The context window is scarce. A 128K-token model with 28K reserved for output has ~100K tokens of input budget. Multiple cognitive subsystems compete for this budget:

- The knowledge store wants to inject relevant insights and heuristics
- The episode store wants to inject past task outcomes
- The file context module wants to inject source code
- The safety system wants to inject constraints and anti-patterns
- The enrichment pipeline wants to inject briefs, research, and decompositions
- The Daimon wants to inject affect-modulated guidance

Each subsystem believes its content is the most important. Without a coordination mechanism, the subsystem that produces the most content dominates the prompt — not because its content is the most valuable, but because it is the loudest.

---

## 2. The VCG Mechanism

### 2.1 Origins

The VCG mechanism combines three foundational results in mechanism design:

- **Vickrey (1961):** In a second-price auction, the winner pays the second-highest bid. This incentivizes truthful bidding — your optimal strategy is to bid your true value, regardless of what others bid.
- **Clarke (1971):** Extended second-price auctions to multiple items. Each winner pays the externality their allocation imposes on others — the reduction in total welfare caused by their presence.
- **Groves (1973):** Proved that the VCG payment rule is the unique mechanism that simultaneously achieves truthful bidding and efficient allocation for quasi-linear utility functions.

### 2.2 Properties

| Property | Meaning for Context Allocation |
|----------|------------------------------|
| **Truthful** | Each subsystem's optimal strategy is to bid its true expected value. No gaming. |
| **Efficient** | The allocation maximizes total expected value across all subsystems. |
| **Individual rationality** | No subsystem is made worse off by participating. |
| **Budget balanced** (weakly) | Total payments ≤ total welfare generated. |

---

## 3. The Bid Formula

From the canonical specification (refactoring-prd/09-innovations.md §XIX.E):

```
bid(section) = expected_value × urgency × affect_weight

Where:
  expected_value = track_record(section) × relevance(section)
  urgency        = 1.0 + time_pressure_factor
  affect_weight  = daimon_modulation(section.type, current_pad_state)
```

### 3.1 Expected Value

```
expected_value = E[task_success | section_included] × relevance_to_current_task
```

This is the same `track_record` used in the active inference scorer (see [07-active-inference-context-selection.md](07-active-inference-context-selection.md)), multiplied by a task-specific relevance score. The expected value measures: "if I include this section, how much does it improve the probability of task success?"

### 3.2 Urgency

```
urgency = 1.0 + max(0, (deadline - now) / total_time_budget)^(-1)
```

When the agent is under time pressure (approaching a deadline or budget limit), urgency increases. High urgency amplifies bids for action-oriented content (task description, file context, gate errors) and dampens bids for exploratory content (research memos, cross-plan context).

### 3.3 Affect Weight

The Daimon's PAD state modulates bids:

| PAD State | Modulation |
|-----------|-----------|
| High arousal | ×1.3 for action-oriented, ×0.7 for exploratory |
| Low pleasure | ×1.5 for anti-patterns and warnings, ×0.8 for standard content |
| Low dominance | ×1.2 for explanatory content, ×0.9 for directive content |
| Neutral | ×1.0 (no modulation) |

---

## 4. The Eight Bidding Subsystems

From the canonical specification:

| # | Subsystem | Bids For | Typical Bid Range |
|---|-----------|----------|-------------------|
| 1 | **Episodic Memory** | Past task outcomes, iteration memory | 0.3-0.8 |
| 2 | **Knowledge Store (Neuro)** | Insights, heuristics, warnings | 0.4-0.9 |
| 3 | **Task Context** | Task description, acceptance criteria | 0.7-1.0 |
| 4 | **File Context** | Source code, type signatures | 0.5-0.9 |
| 5 | **Safety System** | Constraints, anti-patterns, prohibitions | 0.6-1.0 |
| 6 | **Enrichment** | Briefs, research, decompositions | 0.3-0.7 |
| 7 | **Daimon (Affect)** | Affect guidance, motivational modulation | 0.1-0.4 |
| 8 | **Collective** | Mesh knowledge, cross-agent context | 0.2-0.6 |

Each subsystem produces a set of candidate sections with associated bids. The VCG mechanism selects the combination that maximizes total bid value, subject to the token budget constraint.

---

## 5. The Auction Algorithm

### 5.1 Combinatorial Allocation

The attention auction is a **combinatorial auction**: the auctioneer (Composer) must allocate multiple items (context window slots) to multiple bidders (subsystems) with complementarities. Two knowledge entries from the same domain may be worth more together than separately (complementary). Two overlapping entries may be worth less than either alone (substitutes).

### 5.2 VCG Allocation Rule

```
1. Collect bids from all 8 subsystems
   bids = {(section_i, value_i, tokens_i)} for i in 1..N

2. Find the allocation that maximizes total value within budget
   optimal = maximize Σ value_i × x_i
             subject to Σ tokens_i × x_i ≤ budget
             x_i ∈ {0, 1}

3. This is a 0/1 knapsack problem (NP-hard in general)
   For prompt assembly, N is small (20-50 candidates) → solvable by greedy

4. Winner determination: x* = greedy solution
```

### 5.3 VCG Payment Rule

Each winning section pays the externality it imposes:

```
payment(section_i) = Σ_{j ≠ i} value_j(optimal without i) - Σ_{j ≠ i} value_j(optimal with i)
```

In words: section i pays the difference between the total value others would get without i and the total value others get with i. This is the "damage" that i's inclusion causes to others by consuming budget.

### 5.4 Why Payments Matter

In a single-agent system, payments are accounting constructs — no actual money changes hands. Their purpose is **diagnostic**: a section with a high payment is consuming disproportionate budget relative to its value. This signals:

- The section should be compressed (same value in fewer tokens)
- The section should be split (share the budget allocation)
- The budget should be increased for this tier

Payments provide a principled measure of "budget pressure" that manual priority tuning cannot.

---

## 6. Relationship to Active Inference

The VCG auction and active inference (see [07-active-inference-context-selection.md](07-active-inference-context-selection.md)) solve the same allocation problem through different mechanisms:

| Aspect | Active Inference | VCG Auction |
|--------|-----------------|-------------|
| **Setting** | Single agent, centralized | Multi-subsystem, decentralized |
| **Scoring** | EFE: pragmatic + epistemic | Bid: expected_value × urgency × affect |
| **Selection** | Softmax over scores | Combinatorial optimization |
| **Exploration** | Emerges from epistemic value | Emerges from bid uncertainty |
| **Optimality** | Maximizes expected free energy | Maximizes total welfare |
| **Truthfulness** | N/A (single scorer) | Guaranteed (VCG property) |

Both converge on the same allocation under certain conditions:
- When all subsystems bid truthfully (VCG guarantees this), the VCG allocation maximizes total value
- When the EFE scorer has accurate track_record estimates, the softmax selection approximates the value-maximizing allocation

The practical difference: active inference is simpler to implement and sufficient for single-agent prompt assembly. The VCG auction is designed for the multi-agent case — when autonomous agents on the knowledge chain compete for shared context bandwidth, truthful bidding prevents gaming.

---

## 7. Game-Theoretic Properties

### 7.1 Incentive Compatibility

The VCG mechanism is **dominant-strategy incentive compatible**: each subsystem's optimal strategy is to bid its true expected value, regardless of what other subsystems bid.

If the safety system inflates its bids to capture more context window:
- Its sections win the auction at higher payments
- The payments represent the value that displaced sections would have provided
- If the safety sections are less valuable than what they displaced, the total outcome worsens
- The mechanism detects this through outcome tracking

### 7.2 No Useful Deviation

No subsystem can improve its allocation by deviating from truthful bidding:

```
For all subsystems i:
  bid_truthful(i) = argmax utility(i)

This holds because:
  utility(i) = value(i) - payment(i)
  payment(i) depends only on OTHER subsystems' bids
  So i's utility is maximized by maximizing value(i)
  Which means bidding true value
```

### 7.3 Limitations

VCG has known limitations:
- **Computational complexity:** Combinatorial knapsack is NP-hard. For prompt assembly with <50 candidates, the greedy approximation is sufficient.
- **Revenue non-monotonicity:** Adding more candidates can decrease total payments. Not relevant for context allocation.
- **Collusion vulnerability:** Multiple subsystems could collude to lower their payments. Not relevant when subsystems are software modules under the same operator's control.

---

## 8. Academic Foundations

**Vickrey, W. (1961), "Counterspeculation, Auctions, and Competitive Sealed Tenders."** Journal of Finance, 16(1), 8-37. The foundational paper on second-price auctions. Proved that truthful bidding is a dominant strategy in sealed-bid second-price auctions.

**Clarke, E. H. (1971), "Multipart Pricing of Public Goods."** Public Choice, 11(1), 17-33. Extended Vickrey's result to multi-item allocations.

**Groves, T. (1973), "Incentives in Teams."** Econometrica, 41(4), 617-631. Proved the uniqueness of the VCG payment rule for achieving truthfulness and efficiency simultaneously.

**Simon, H. A. (1971), "Designing Organizations for an Information-Rich World."** The observation that information abundance creates attention scarcity — the foundational framing for attention allocation.

**Friston, K. (2022), The Free Energy Principle.** Active inference as an alternative to mechanism design for attention allocation. Both approaches converge on efficient allocation under different assumptions.

---

## 9. Implementation Plan

| # | Item | Status | Notes |
|---|------|--------|-------|
| 1 | Define 8 bidding subsystems as traits | **Not yet** | Each subsystem implements `fn bid(task) -> Vec<(Section, f64)>` |
| 2 | Implement VCG allocation (greedy knapsack) | **Not yet** | Reuse PromptComposer's greedy include |
| 3 | Implement VCG payment computation | **Not yet** | For diagnostic purposes |
| 4 | Wire bidding into context assembly | **Not yet** | Replace static priorities with bids |
| 5 | Payment-based budget pressure monitoring | **Not yet** | Dashboard for budget allocation analysis |

---

## 10. Current Status and Gaps

| Aspect | Status |
|--------|--------|
| VCG mechanism specified | **Specified** |
| Bid formula specified | **Specified** |
| 8 bidding subsystems defined | **Specified** |
| Priority-based allocation (fallback) | **Implemented** |
| VCG allocation | **Not yet** |
| VCG payments | **Not yet** |
| Truthfulness verification | **Not yet** |
| Budget pressure monitoring | **Not yet** |

---

## Cross-References

- [07-active-inference-context-selection.md](07-active-inference-context-selection.md) — Alternative scoring mechanism
- [08-5-stage-assembly-pipeline.md](08-5-stage-assembly-pipeline.md) — Pipeline where allocation occurs
- [12-affect-modulated-retrieval.md](12-affect-modulated-retrieval.md) — Affect modulation of bids
- `refactoring-prd/09-innovations.md` §II, §XIX.E — Canonical VCG specification
