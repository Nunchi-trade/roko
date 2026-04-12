# Active Inference for Compute Allocation

> Expected Free Energy (EFE) determines how much cognitive resource an agent invests on each tick — zero hyperparameters, pure information-theoretic optimization.

**Topic**: [16-heartbeat](./INDEX.md)
**Prerequisites**: [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md), [09-16-t0-probes.md](./09-16-t0-probes.md)
**Key sources**: `refactoring-prd/01-synapse-architecture.md` §Dual-Process Cognition, `refactoring-prd/09-innovations.md` §XIX-A, Friston 2010 (Nature Reviews Neuroscience)

---

## Abstract

The T0/T1/T2 tier gating described in [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md) uses a heuristic threshold to decide how much compute to invest per tick. Active inference provides the theoretical foundation for this decision: the agent should invest compute that **minimizes expected free energy (EFE)** — balancing the pragmatic value of action with the epistemic value of information, minus the cost.

Friston's free energy principle (2010, "The free-energy principle: a unified brain theory?", Nature Reviews Neuroscience 11(2)) proposes that biological organisms minimize a quantity called "free energy" — the discrepancy between their internal model and the sensory evidence. Minimizing free energy is equivalent to maximizing model evidence, which is equivalent to minimizing surprise. An agent that minimizes free energy learns an accurate world model and acts to keep itself in preferred states.

The active inference extension (Friston et al. 2015, "Active inference and epistemic value", Cognitive Neuroscience 6(4)) goes further: the agent doesn't just minimize current free energy — it selects policies (sequences of actions) that minimize **expected** free energy into the future. This naturally balances exploitation (pragmatic value — doing things that achieve goals) with exploration (epistemic value — doing things that reduce uncertainty).

For Roko, active inference provides a principled, zero-hyperparameter answer to the question: "How much should I think about this?" The tier decision, the context budget, and the model selection all emerge from EFE minimization rather than hand-tuned thresholds.

---

## The EFE Formula

Expected Free Energy for a policy π at time step τ:

```
G(π, τ) = -E_Q[ln P(o_τ | C)]  +  E_Q[H[P(o_τ | s_τ)]]
           ─────────────────────     ─────────────────────
           pragmatic value            epistemic value
           (expected utility           (expected information
            of preferred outcomes)      gain from observations)
```

Where:
- `Q` is the agent's approximate posterior (its current beliefs)
- `o_τ` is the expected observation at time τ
- `s_τ` is the expected hidden state at time τ
- `C` is the agent's preferred outcomes (goals/objectives)
- `H` is entropy (uncertainty)

The first term (pragmatic value) captures how much the agent expects to achieve its goals under this policy. Policies that lead to preferred outcomes have low pragmatic free energy.

The second term (epistemic value) captures how much uncertainty the agent expects to resolve. Policies that expose the agent to informative observations have high epistemic value. This is what drives exploration — the agent seeks out information that reduces its uncertainty, even when the immediate pragmatic value is low.

### Applied to Tier Selection

For Roko's tier selection, the "policy" is the tier choice (T0, T1, T2) and the EFE formula becomes:

```
EFE(tier) = pragmatic_value(tier) + epistemic_value(tier) - cost(tier)

where:
  pragmatic_value(T0) = value of applying existing playbook rules (low if no matching rule)
  pragmatic_value(T1) = value of quick assessment + possible action (medium)
  pragmatic_value(T2) = value of deep analysis + comprehensive action (high)

  epistemic_value(T0) = 0 (no new information gained — only existing model used)
  epistemic_value(T1) = moderate (fast LLM provides some uncertainty reduction)
  epistemic_value(T2) = high (full LLM with complete context maximally reduces uncertainty)

  cost(T0) = 0
  cost(T1) = $0.001-0.003 + 200-500ms latency
  cost(T2) = $0.01-0.25 + 1-5s latency
```

The optimal tier is the one that maximizes `pragmatic_value + epistemic_value - cost`. When uncertainty is low (probes report nothing unusual), T0's zero cost dominates — there's no epistemic value to gain and no pragmatic value in redundant analysis. When uncertainty is high, T2's high epistemic value justifies its cost.

### Zero Hyperparameters

The elegance of the EFE formulation is that it requires **zero hyperparameters** for the exploration/exploitation tradeoff. Traditional approaches (epsilon-greedy, UCB, Thompson sampling) all require tuning:
- Epsilon-greedy: what ε?
- UCB: what exploration bonus coefficient?
- Thompson sampling: what prior distribution?

EFE naturally balances exploration and exploitation through the two terms of the formula. When epistemic value is high (uncertainty is large), the agent explores. When pragmatic value dominates (goals are clear, uncertainty is low), the agent exploits. The balance shifts automatically as the agent learns.

This is Friston's key insight: exploration and exploitation are not opposing objectives that require a tradeoff parameter — they are **two aspects of the same objective** (minimizing expected free energy).

---

## EFE for Context Selection

Active inference also applies to **context selection** — choosing which Engrams to include in the LLM's context window. Each potential context entry has an expected free energy:

```
EFE(entry) = -E[utility_gain | entry_included] + E[H_reduction | entry_included] - token_cost(entry)
```

Entries with high expected utility gain (they help the agent achieve its goal) and high expected uncertainty reduction (they resolve an open question) are prioritized. Entries that would consume many tokens without reducing uncertainty or improving outcomes are deprioritized.

This produces a principled alternative to top-k retrieval or manual priority ordering. The `PredictiveScorer` (target implementation in `roko-core`) computes this EFE approximation for each candidate Engram:

```rust
/// Scores Engrams using an active inference EFE approximation.
///
/// Each Engram is evaluated for:
/// 1. Pragmatic value: how useful is this for achieving current goals?
/// 2. Epistemic value: how much uncertainty does this resolve?
/// 3. Token cost: how many tokens does this consume?
///
/// The score is EFE = pragmatic + epistemic - cost_penalty.
pub struct PredictiveScorer {
    /// How much to weight pragmatic value relative to epistemic
    /// Note: this is NOT a tradeoff parameter — both are positive
    /// contributions. This weights domain-specific pragmatic metrics.
    pragmatic_weight: f32,
}

impl Scorer for PredictiveScorer {
    fn score(&self, engram: &Engram, ctx: &Context) -> Score {
        let pragmatic = self.compute_pragmatic_value(engram, ctx);
        let epistemic = self.compute_epistemic_value(engram, ctx);
        let cost_penalty = engram.estimated_tokens() as f32 / 1000.0 * 0.01;

        let effective = pragmatic * self.pragmatic_weight
            + epistemic
            - cost_penalty;

        Score {
            confidence: engram.score.confidence,
            novelty: epistemic,      // epistemic value IS novelty
            utility: pragmatic,      // pragmatic value IS utility
            reputation: engram.score.reputation,
            precision: engram.score.precision,
            salience: effective.max(0.0),  // effective EFE as salience
            coherence: engram.score.coherence,
        }
    }

    fn name(&self) -> &'static str { "predictive_scorer" }
}
```

---

## Connection to the CascadeRouter

The `CascadeRouter` in `roko-learn/src/cascade_router.rs` implements a three-stage cascade (Static → Confidence → UCB1) for model selection within a tier. Active inference provides the theoretical grounding for the UCB1 stage:

- **Static stage** (< 50 observations): Uses a fixed routing table because there isn't enough data for principled exploration. This is the "prior" phase.
- **Confidence stage** (50-200 observations): Routes by confidence score, which approximates pragmatic value. Exploration happens via a fixed probability (epsilon-like).
- **UCB1 stage** (> 200 observations): Contextual bandit with LinUCB. The UCB exploration bonus approximates epistemic value — models with fewer observations get higher exploration bonuses because there's more information to gain from trying them.

The target `ActiveInferenceRouter` (not yet implemented) replaces UCB1's heuristic exploration bonus with the full EFE computation, making the exploration/exploitation tradeoff principled rather than approximated.

---

## Relationship to Sims' Rational Inattention

Sims (2003, "Implications of rational inattention", Journal of Monetary Economics 50(3)) provides a complementary perspective: agents have finite information processing capacity and should allocate it optimally. The key result is that agents should pay more attention to channels (information sources) with higher signal-to-noise ratio relative to their processing cost.

For Roko, Sims' framework explains why T0 probes are valuable: they have extremely high signal-to-noise ratio (deterministic checks with no stochasticity) at zero processing cost. An agent should maximally exploit these cheap, reliable signals before investing in expensive, stochastic LLM analysis.

The rational inattention framework also explains the VCG Attention Auction (see [12-attention-auction-and-gating.md](./12-attention-auction-and-gating.md)): context sections with higher signal-to-noise ratio (measured by expected utility) should receive more token budget. The auction mechanism discovers the optimal allocation through truthful bidding.

---

## Implementation Path

The active inference compute allocation is implemented in stages:

### Stage 1: Heuristic Threshold (Current)

The current implementation uses prediction error vs. adaptive threshold — a heuristic approximation of EFE. This is what [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md) describes.

### Stage 2: PredictiveScorer (Near-term)

Add the `PredictiveScorer` to `roko-core` for EFE-based context selection. This doesn't change the tier gating — it improves context assembly within T1/T2.

### Stage 3: ActiveInferenceRouter (Target)

Replace the UCB1 stage of `CascadeRouter` with a full EFE computation for both tier selection and model routing. This requires:
- Maintaining a probabilistic world model (approximate posterior Q)
- Computing expected observations under each policy
- Estimating uncertainty reduction for each tier/model choice
- Tracking preferred outcomes (C matrix) from task specifications

See [11-active-inference-state-space.md](./11-active-inference-state-space.md) for the factorized discrete POMDP that makes Stage 3 tractable.

---

## Academic Foundations

- **Friston 2010** — "The free-energy principle: a unified brain theory?" (Nature Reviews Neuroscience 11(2)). The foundational paper on free energy minimization in biological cognition.
- **Friston et al. 2015** — "Active inference and epistemic value" (Cognitive Neuroscience 6(4)). Extension to expected free energy for action selection.
- **Parr & Friston 2017** — "Working memory, attention, and salience in active inference" (Scientific Reports 7). Attention as precision weighting in active inference.
- **Sims 2003** — "Implications of rational inattention" (Journal of Monetary Economics 50(3)). Optimal allocation of finite information processing capacity.
- **Kahneman 2011** — "Thinking, Fast and Slow" (Farrar, Straus and Giroux). System 1/System 2 as the cognitive basis.
- **Chen et al. 2023** — FrugalGPT (arXiv:2305.05176). Cascade routing as a practical implementation of compute-optimal allocation.
- **Koudahl et al. 2024** — (arXiv:2412.10425). Factorized discrete POMDP for tractable active inference.
- **VERSES AI** — Genius platform. Industrial deployment of active inference for agent cognition.

---

## Current Status and Gaps

**What exists:**
- `InferenceTier` and `TierRouter` in `bardo-primitives/src/tier.rs` — the heuristic threshold approach.
- `CascadeRouter` with UCB1 stage in `roko-learn/src/cascade_router.rs` — approximates exploration/exploitation.
- Prediction error concept in legacy heartbeat specification.

**What is missing:**
- `PredictiveScorer` implementing EFE approximation for context scoring.
- `ActiveInferenceRouter` replacing UCB1 with full EFE computation.
- Probabilistic world model (approximate posterior Q) for tier selection.
- Preferred outcome specification (C matrix) from task goals.
- Integration of Sims' rational inattention for attention budget allocation.

---

## Cross-References

- See [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md) for the heuristic tier gating this formalizes
- See [09-16-t0-probes.md](./09-16-t0-probes.md) for the probes that generate prediction error signals
- See [11-active-inference-state-space.md](./11-active-inference-state-space.md) for the factorized POMDP state space
- See [12-attention-auction-and-gating.md](./12-attention-auction-and-gating.md) for VCG-based context budget allocation
- See topic [05-learning](../05-learning/INDEX.md) for the CascadeRouter and bandit algorithms
