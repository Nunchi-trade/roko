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

## 8. Strategic Bidding: Can Subsystems Learn to Bid Optimally?

In a repeated auction (context assembly runs for every task), subsystems can learn from past outcomes to improve their bids. This is desirable — learned bids reflect actual section value — but requires careful design to maintain truthfulness.

### 8.1 The Learning-Truthfulness Tension

VCG guarantees truthful bidding is a dominant strategy in a **single-shot** auction. In **repeated** auctions, strategic behavior can emerge even under VCG:

- A subsystem might learn that inflating bids for "safety" sections guarantees inclusion, even when those sections are marginally useful for the current task.
- A subsystem might learn that other subsystems always bid high, so it should bid even higher to capture budget.

Research on learning in repeated auctions [MIT CEEPR Working Paper 2023-18] shows that no-regret learning algorithms tend to converge to welfare-maximizing equilibria. Strategic bidding in first-price auctions is harder to stabilize [arXiv:2402.07363], but VCG's second-price rule dampens strategic incentives.

### 8.2 Bid Learning via Thompson Sampling

Each subsystem maintains a posterior distribution over its bid value, updated by task outcomes:

```rust
/// A subsystem that learns its bid value from historical outcomes.
pub struct LearningBidder {
    pub subsystem_id: SubsystemId,
    /// Per-section Beta distributions for Thompson sampling.
    /// Beta(alpha, beta) where alpha = successes when included, beta = failures.
    pub section_betas: HashMap<String, (f64, f64)>,
    /// Prior bid value (before learning).
    pub prior_bid: f64,
}

impl LearningBidder {
    /// Compute bid for a section using Thompson sampling.
    pub fn bid(&self, section_name: &str, relevance: f64) -> f64 {
        let (alpha, beta) = self.section_betas
            .get(section_name)
            .copied()
            .unwrap_or((1.0, 1.0));  // Uniform prior

        // Sample from Beta(alpha, beta) for exploration
        let sampled_track_record = beta_sample(alpha, beta);

        // Bid = sampled track record × relevance to current task
        sampled_track_record * relevance
    }

    /// Update after observing a task outcome.
    pub fn update(&mut self, section_name: &str, was_included: bool, gate_passed: bool) {
        if was_included {
            let entry = self.section_betas
                .entry(section_name.to_string())
                .or_insert((1.0, 1.0));
            if gate_passed {
                entry.0 += 1.0;  // alpha: success count
            } else {
                entry.1 += 1.0;  // beta: failure count
            }
        }
    }
}
```

Thompson sampling provides natural exploration: sections with uncertain value are occasionally bid higher (sampled from a wide Beta distribution), ensuring the system discovers their true value. As observations accumulate, the Beta distribution narrows and bids converge to the true expected value.

### 8.3 Convergence Properties

From the MARL auction literature [arXiv:2402.19420, 2024]:

| Property | Expected Behavior |
|---|---|
| Convergence time | ~50-100 tasks per subsystem to stabilize bids |
| Equilibrium type | Welfare-maximizing (under VCG payment rule) |
| Exploration rate | Decreasing: Beta distributions narrow over time |
| Sensitivity to environment change | Moderate: sudden shifts in task distribution require re-exploration |

### 8.4 Collusion Detection

Although subsystems under the same operator's control have no incentive to collude, structural coupling can create emergent collusion-like behavior (e.g., two subsystems always bidding high because they share a relevance signal). Detection:

```rust
/// Detect bid correlation that might indicate structural coupling.
pub fn detect_bid_correlation(
    bid_history: &[(SubsystemId, SubsystemId, Vec<(f64, f64)>)],
    threshold: f64,  // default: 0.85
) -> Vec<(SubsystemId, SubsystemId, f64)> {
    bid_history.iter()
        .filter_map(|(s1, s2, pairs)| {
            let correlation = pearson_correlation(pairs);
            if correlation > threshold {
                Some((*s1, *s2, correlation))
            } else {
                None
            }
        })
        .collect()
}
```

---

## 9. Auction Efficiency Metrics

### 9.1 Welfare Loss

The **welfare loss** (or **deadweight loss**) measures how much total value is lost compared to the optimal allocation:

```
welfare_loss = optimal_total_value - actual_total_value

Where:
  optimal_total_value = value of the allocation that maximizes Σ v_i × x_i
                        subject to Σ tokens_i × x_i ≤ budget
  actual_total_value  = value of the allocation produced by the auction
```

For the greedy knapsack used in Roko, the welfare loss is bounded:

```
greedy_welfare >= 0.5 × optimal_welfare  (Dantzig 1957)
```

In practice, with section values correlated to their token size, the greedy approximation is much tighter — typically >90% of optimal.

### 9.2 Pareto Optimality

An allocation is **Pareto optimal** if no section can be added without removing another section of equal or greater value. The VCG mechanism produces Pareto-optimal allocations when the welfare maximization is exact.

```rust
/// Check if a VCG allocation is Pareto optimal.
pub fn is_pareto_optimal(
    included: &[SectionAllocation],
    excluded: &[SectionAllocation],
    budget_remaining: usize,
) -> bool {
    // For each excluded section that fits in remaining budget:
    for exc in excluded {
        if exc.tokens <= budget_remaining {
            // Can we add it without removing anything?
            // If yes, the current allocation is NOT Pareto optimal.
            return false;
        }
    }
    // For each excluded section that doesn't fit:
    for exc in excluded {
        if exc.tokens > budget_remaining {
            // Can we swap it for any included section with lower value?
            for inc in included {
                if inc.value < exc.value && inc.tokens >= exc.tokens {
                    // Swap improves welfare → not Pareto optimal
                    return false;
                }
            }
        }
    }
    true
}
```

### 9.3 Price of Anarchy

The **Price of Anarchy** (PoA) measures welfare loss from strategic behavior:

```
PoA = welfare(socially optimal) / welfare(worst Nash equilibrium)
```

Under VCG with truthful bidding, PoA = 1 (no loss from strategic behavior). The concern is the greedy approximation: when welfare maximization is approximate (greedy knapsack), VCG payments no longer guarantee exact truthfulness [Nisan & Ronen]. The practical PoA for Roko's context allocation is estimated at <1.1 (less than 10% welfare loss) based on the small candidate set size (N < 50).

Research on strong and Pareto equilibria [Chien & Sinclair, UC Berkeley] shows that the PoA for Pareto-optimal Nash equilibria is significantly smaller than for arbitrary Nash equilibria in congestion games — a related allocation setting.

### 9.4 Diagnostic Dashboard Metrics

```rust
/// Auction diagnostics computed after each context assembly.
pub struct AuctionDiagnostics {
    /// Total bid value of winning sections.
    pub total_welfare: f64,
    /// Total VCG payments across all winners.
    pub total_payments: f64,
    /// Welfare loss vs. optimal (estimated by trying exhaustive search for N < 20).
    pub welfare_loss: f64,
    /// Is the allocation Pareto optimal?
    pub pareto_optimal: bool,
    /// Sections with highest payment (most budget pressure).
    pub highest_payment_sections: Vec<(String, f64)>,
    /// Sections that were displaced (excluded due to budget).
    pub displaced_sections: Vec<(String, f64)>,
    /// Budget utilization: tokens_used / tokens_available.
    pub budget_utilization: f64,
}
```

---

## 10. Alternative Fairness Criteria

VCG maximizes aggregate welfare, but there are scenarios where other fairness criteria are more appropriate.

### 10.1 Proportional Fairness

Each subsystem receives allocation proportional to its bid:

```
allocation_i = (bid_i / Σ bid_j) × total_budget
```

**Advantage:** Every subsystem gets some representation. No subsystem is completely starved.

**Disadvantage:** Low-value subsystems consume budget that higher-value subsystems need. Can produce worse outcomes than aggressive priority-based dropping.

**When to use:** When the system has no confidence in bid accuracy (early cold-start phase, or when all subsystems are poorly calibrated). Proportional fairness is the safe default.

Research: Regularized Proportional Fairness (RPF) [Zhu et al., ICLR 2025, arXiv:2501.01111] adds neural-network-learned regularization to standard PF, increasing robustness to misreported bids.

### 10.2 Max-Min Fairness

Maximize the minimum allocation across all subsystems:

```
max min_i allocation_i
subject to Σ allocation_i ≤ total_budget
```

**Advantage:** The worst-served subsystem is as well-served as possible. Prevents catastrophic context gaps.

**Disadvantage:** Very inefficient — gives equal weight to low-value and high-value subsystems. The safety system's 200-token constraint gets the same allocation as the file context module's 8000-token need.

**When to use:** Only for safety-critical subsystems. A max-min guarantee on the safety subsystem ensures that safety constraints always get minimum viable representation, regardless of how other subsystems bid.

### 10.3 Alpha-Fairness Spectrum

The three criteria are special cases of the alpha-fairness family [Bertsimas et al.]:

```
maximize Σ_i (allocation_i^(1-α)) / (1-α)

α = 0: Utilitarian (VCG) — maximize total welfare
α = 1: Proportional fairness — maximize geometric mean
α → ∞: Max-min fairness — maximize the minimum
```

Roko can implement a configurable α parameter:

```rust
/// Configurable fairness parameter for the attention auction.
pub struct FairnessConfig {
    /// Alpha parameter for the alpha-fairness family.
    /// 0.0 = pure efficiency (VCG-like)
    /// 1.0 = proportional fairness
    /// 10.0 = approximately max-min
    pub alpha: f64,  // default: 0.0 (pure efficiency)
    /// Minimum guaranteed allocation for safety subsystem (max-min floor).
    pub safety_floor_tokens: usize,  // default: 200
}
```

### 10.4 Hybrid Policy: VCG + Safety Floor

The recommended policy combines VCG efficiency with a max-min floor for safety:

```
1. Reserve safety_floor_tokens for the Safety subsystem (guaranteed minimum)
2. Run VCG auction on the remaining budget across all subsystems
3. Safety subsystem can bid for ADDITIONAL tokens beyond its floor
4. All other subsystems compete in the standard VCG auction
```

This ensures safety constraints always appear (max-min floor) while maximizing total value for the remaining budget (VCG efficiency).

---

## 11. Mechanism Design for LLMs: The Token Auction

A landmark paper directly connecting mechanism design to LLM systems:

**"Mechanism Design for Large Language Models"** [Duetting et al., WWW 2024 Best Paper, arXiv:2310.10826]. Proposes a **token auction** model where competing LLM agents bid for influence over the output, operating token-by-token. Key results:

- Desirable incentive properties (truthful bidding) are equivalent to a **monotonicity condition** on output aggregation.
- When valuations are KL-divergence-based, the welfare-maximizing rule is a **weighted log-space convex combination** of target distributions.
- This is the first clean extension of VCG to LLM content generation.

The connection to Roko's VCG attention auction: Duetting et al.'s token auction operates at the generation level (which tokens to produce), while Roko's operates at the context level (which tokens to include). Both use the same incentive-compatibility framework. The token auction validates that mechanism design is applicable to LLM systems in practice, not just in theory.

---

## 12. Academic Foundations

**Vickrey, W. (1961), "Counterspeculation, Auctions, and Competitive Sealed Tenders."** Journal of Finance, 16(1), 8-37. The foundational paper on second-price auctions.

**Clarke, E. H. (1971), "Multipart Pricing of Public Goods."** Public Choice, 11(1), 17-33.

**Groves, T. (1973), "Incentives in Teams."** Econometrica, 41(4), 617-631.

**Simon, H. A. (1971), "Designing Organizations for an Information-Rich World."**

**Friston, K. (2022), The Free Energy Principle.** Active inference as an alternative to mechanism design.

**Duetting, Mirrokni, Paes Leme, Xu, Zuo (2024), "Mechanism Design for Large Language Models."** WWW 2024 Best Paper, arXiv:2310.10826. Token auction model for aggregating competing LLM agents. First clean extension of VCG to LLM systems.

**Zhu et al. (2025), "Regularized Proportional Fairness Mechanism for Resource Allocation Without Money."** ICLR 2025, arXiv:2501.01111. RPF-Net adds neural regularization to proportional fairness for robustness against misreports.

**MIT CEEPR (2023), "Learning in Repeated Multi-Unit Auctions."** Working Paper 2023-18. No-regret learning converges to welfare-maximizing equilibria in repeated auctions.

**arXiv:2402.19420 (2024), "Understanding Iterative Combinatorial Auction Designs via Multi-Agent Reinforcement Learning."** Deep MARL computes equilibria in combinatorial auctions.

**arXiv:2402.07363 (2024), "Strategically-Robust Learning Algorithms for Bidding in First-Price Auctions."** Robustness guarantees against adversarial strategic behavior in learning-based bidding.

**Chien & Sinclair (UC Berkeley), "Strong and Pareto Price of Anarchy in Congestion Games."** The PoA for Pareto-optimal Nash equilibria is significantly smaller than for arbitrary Nash equilibria.

**Nisan & Ronen, "Computationally Feasible VCG Mechanisms."** When welfare maximization is approximate, VCG-based mechanisms lose truthfulness guarantees.

---

## 13. Implementation Plan

| # | Item | Status | Notes |
|---|------|--------|-------|
| 1 | Define 8 bidding subsystems as traits | **Not yet** | Each subsystem implements `fn bid(task) -> Vec<(Section, f64)>` |
| 2 | Implement VCG allocation (greedy knapsack) | **Not yet** | Reuse PromptComposer's greedy include |
| 3 | Implement VCG payment computation | **Not yet** | For diagnostic purposes |
| 4 | Wire bidding into context assembly | **Not yet** | Replace static priorities with bids |
| 5 | Payment-based budget pressure monitoring | **Not yet** | Dashboard for budget allocation analysis |
| 6 | Implement LearningBidder (Thompson sampling) | **Not yet** | Per-subsystem bid learning (§8.2) |
| 7 | Implement auction diagnostics (§9.4) | **Not yet** | Welfare loss, Pareto check, budget utilization |
| 8 | Implement alpha-fairness config (§10.3) | **Not yet** | Configurable VCG vs proportional vs max-min |
| 9 | Implement VCG + safety floor hybrid (§10.4) | **Not yet** | Guaranteed safety allocation + VCG for rest |

---

## 14. Current Status and Gaps

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
| Strategic bidding via Thompson sampling (§8) | **Designed** — LearningBidder specified |
| Auction efficiency metrics (§9) | **Designed** — welfare loss, Pareto, PoA specified |
| Alternative fairness criteria (§10) | **Designed** — proportional, max-min, alpha-fair specified |
| VCG + safety floor hybrid (§10.4) | **Designed** — recommended policy specified |
| Collusion detection (§8.4) | **Designed** — correlation-based detection specified |

---

## Cross-References

- [05-token-budget-management.md](05-token-budget-management.md) — Budget learning that feeds bid calibration
- [07-active-inference-context-selection.md](07-active-inference-context-selection.md) — Alternative scoring mechanism
- [08-5-stage-assembly-pipeline.md](08-5-stage-assembly-pipeline.md) — Pipeline where allocation occurs
- [09-predictive-foraging-mvt.md](09-predictive-foraging-mvt.md) — MVT as complementary stopping rule
- [12-affect-modulated-retrieval.md](12-affect-modulated-retrieval.md) — Affect modulation of bids
- `refactoring-prd/09-innovations.md` §II, §XIX.E — Canonical VCG specification
