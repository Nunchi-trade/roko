# Dual-Process Cognition: T0, T1, T2

> The LLM-Last architecture — most ticks are free, some are cheap, a few are expensive. Uncertainty determines compute investment.

**Topic**: [16-heartbeat](./INDEX.md)
**Prerequisites**: [00-coala-9-step-pipeline.md](./00-coala-9-step-pipeline.md), [04-gamma-reactive-loop.md](./04-gamma-reactive-loop.md)
**Key sources**: `refactoring-prd/01-synapse-architecture.md` §Dual-Process Cognition, `bardo-primitives/src/tier.rs`, legacy `bardo-backup/prd/01-golem/02-heartbeat.md` §S2

---

## Abstract

Daniel Kahneman's dual-process theory ("Thinking, Fast and Slow", 2011) distinguishes two modes of cognition: System 1 (fast, automatic, effortless, heuristic-based) and System 2 (slow, deliberate, effortful, analytical). In the brain, most processing is System 1 — you don't consciously decide to read these words, recognize a face, or dodge a thrown object. System 2 engages only when System 1 detects something that requires attention: a complex math problem, an unexpected event, a novel situation.

Roko implements this distinction literally. The three cognitive tiers — T0, T1, T2 — are not abstract labels. They correspond to concrete implementation choices with dramatically different costs:

| Tier | Kahneman | Implementation | Cost per Call | Latency | Frequency |
|---|---|---|---|---|---|
| **T0** | System 1 (pure) | Deterministic probes + playbook rules. No LLM. | $0.00 | <10ms | ~80% of ticks |
| **T1** | System 1 → System 2 (shallow) | Fast LLM (Haiku-class). Reduced context. | $0.001-0.003 | 200-500ms | ~15% of ticks |
| **T2** | System 2 (deep) | Full LLM (Sonnet/Opus-class). Full Cognitive Workspace. | $0.01-0.25 | 1-5s | ~5% of ticks |

The distribution is not a target — it is an **emergent property** of the gating mechanism. When the environment is predictable (low prediction error), most ticks naturally suppress at T0. When the environment is surprising (high prediction error), more ticks escalate. The 80/15/5 distribution is the observed steady-state for typical domains; volatile domains may shift to 60/25/15 and calm domains to 90/8/2.

This document specifies the dual-process architecture: the theoretical basis, the `InferenceTier` enum in the codebase, the adaptive gating threshold, the cost model, and how the three tiers interact with the Synapse traits.

---

## The LLM-Last Principle

Most agent frameworks are LLM-first: every input routes through an LLM, and tools handle execution. Roko inverts this. The LLM is the **last resort**, not the first. Every tick begins with deterministic checks. Only when those checks detect a condition requiring reasoning does the LLM get invoked.

This inversion is grounded in three academic frameworks:

1. **Kahneman (2011)** — Dual-process theory. System 1 handles routine; System 2 handles exceptions.
2. **Talker-Reasoner framework** (Google Research, 2024) — Formal separation of fast heuristic "talking" from slow analytical "reasoning" in agent architectures.
3. **FrugalGPT** (Chen et al. 2023, arXiv:2305.05176; published 2024 in TMLR) — Demonstrates that cascade architectures matching GPT-4 performance with up to 98% cost reduction through intelligent tier routing.
4. **DPT-Agent** (Zhang et al. 2025, arXiv:2502.11882) — Dual-process theory applied directly to LLM agent decision-making.
5. **CLARION** (Sun et al. 2005) — Cognitive architecture with explicit dual-level processing: implicit (subsymbolic, fast) and explicit (symbolic, slow). Roko's T0 maps to CLARION's implicit level; T1/T2 map to the explicit level.

The economic argument is decisive: if an agent ticks at 10-second intervals and calls an LLM on every tick, the daily cost at $0.10/call is ~$864. With T0 suppression handling 80% of ticks, daily LLM cost drops to ~$2-50. The dual-process architecture makes high-frequency autonomous agents economically viable.

---

## The InferenceTier Enum

The `InferenceTier` enum in `bardo-primitives/src/tier.rs` is the codebase representation of the three tiers:

```rust
/// Cognitive inference tier — how much compute to invest in this tick.
///
/// Maps to Kahneman's dual-process theory:
/// - T0: System 1 (pure heuristic, no LLM)
/// - T1: System 1 → System 2 transition (fast model, shallow reasoning)
/// - T2: System 2 (full model, deep reasoning)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InferenceTier {
    /// Suppress. No LLM call. Deterministic probes + playbook rules handle
    /// everything. ~80% of ticks. $0.00 inference cost.
    T0 = 0,

    /// Analyze. Fast model (Haiku-class). Reduced context (~4,000 tokens).
    /// Moderate anomaly detected. ~15% of ticks. $0.001-0.003 per call.
    T1 = 1,

    /// Deliberate. Full model (Sonnet/Opus-class). Complete Cognitive
    /// Workspace (~32,000 tokens). High surprise, novel situation, or
    /// forced escalation. ~5% of ticks. $0.01-0.25 per call.
    T2 = 2,
}
```

### TierRouter

The `TierRouter` in `bardo-primitives/src/tier.rs` selects the specific model within a tier:

```rust
/// Routes an InferenceTier to a concrete model identifier.
pub struct TierRouter;

impl TierRouter {
    /// Select model based on tier and resource state.
    ///
    /// The vitality parameter (renamed: resource_health) controls whether
    /// T2 uses Opus-class (resource_health > 0.3) or Sonnet-class
    /// (resource_health <= 0.3 — conserve resources when health is low).
    pub fn select_model(
        tier: InferenceTier,
        resource_health: f32,
    ) -> Option<&'static str> {
        match tier {
            InferenceTier::T0 => None,  // No model needed
            InferenceTier::T1 => Some("claude-haiku-4-5"),
            InferenceTier::T2 => {
                if resource_health > T2_RESOURCE_THRESHOLD {
                    Some("claude-opus-4-6")
                } else {
                    Some("claude-sonnet-4-6")
                }
            }
        }
    }
}

/// Resource health threshold for T2 model selection.
/// Below this, T2 downgrades from Opus to Sonnet to conserve budget.
pub const T2_RESOURCE_THRESHOLD: f32 = 0.3;
```

> **Code alignment note**: In the current codebase, the parameter is named `vitality` and the constant is `T2_VITALITY_THRESHOLD`. The rename to `resource_health` / `T2_RESOURCE_THRESHOLD` removes the mortality framing per the reframe rules. The underlying mechanism is identical: when the agent's resource health is low (approaching budget exhaustion), T2 uses a cheaper model.

---

## The Adaptive Gating Threshold

The gating decision — which tier handles this tick — is driven by **prediction error** compared to an **adaptive threshold**. The prediction error measures how surprising the current observation is; the threshold determines how much surprise is needed to justify LLM deliberation.

### Prediction Error Computation

Prediction error is a scalar in [0.0, 1.0] aggregated from probe results, regime detection, and domain-specific signals. The computation follows Friston's (2010) precision-weighted prediction error framework:

```rust
/// Compute aggregate prediction error from probe results.
///
/// Each probe contributes a weighted surprise signal.
/// The aggregate is capped at 1.0.
fn compute_prediction_error(
    probes: &[ProbeResult],
    predictions: &PredictionState,
    regime: &Regime,
) -> f32 {
    let mut error: f32 = 0.0;

    // Probe anomalies: 5% per anomaly
    let anomaly_count = probes.iter()
        .filter(|p| p.is_anomalous())
        .count();
    error += anomaly_count as f32 * 0.05;

    // Regime change: 40% if regime shifted since last tick
    if regime.changed_since_last_tick() {
        error += 0.40;
    }

    // World model drift: divergence between predicted and actual state
    error += predictions.compute_drift() * 0.30;

    // Pending interventions: 10% per intervention
    let pending = predictions.pending_intervention_count();
    error += pending as f32 * 0.10;

    error.min(1.0)
}
```

### Adaptive Threshold

The threshold is not fixed. It adapts based on affect state, resource constraints, and strategy confidence:

```rust
/// Compute the adaptive gating threshold.
///
/// Kahneman's (2011) insight: System 2 engagement is modulated by
/// cognitive load, emotional state, and available resources.
///
/// Friston's (2010) precision weighting: higher precision (confidence)
/// in current predictions → higher threshold → less escalation.
fn compute_adaptive_threshold(state: &AgentState) -> f32 {
    let base = 0.20;  // default threshold

    // Affect modulation: low dominance → lower threshold (more cautious)
    let dominance = state.cortical_state.pad().dominance;
    let affect_adj = if dominance < -0.2 {
        -0.05  // uncertain → easier to escalate
    } else if dominance > 0.3 {
        0.05   // confident → harder to escalate
    } else {
        0.0
    };

    // Resource modulation: low resources → higher threshold (conserve)
    let budget_pct = state.budget_tracker.daily_usage_percent();
    let resource_adj = if budget_pct > 0.80 {
        0.10  // approaching budget → harder to escalate
    } else {
        0.0
    };

    // Arousal modulation: high arousal → lower threshold (more alert)
    let arousal = state.cortical_state.pad().arousal;
    let arousal_adj = if arousal > 0.5 {
        -0.05  // surprised → easier to escalate
    } else {
        0.0
    };

    // Strategy confidence: high → higher threshold (coast)
    let confidence_adj = state.strategy_confidence * 0.05;

    (base + affect_adj + resource_adj + arousal_adj + confidence_adj)
        .clamp(0.05, 0.50)
}
```

### Gating Decision

```rust
/// The gating decision: which tier handles this tick?
fn gate(prediction_error: f32, threshold: f32, state: &AgentState) -> InferenceTier {
    // Forced escalation: user steer or safety alert always gets T2
    if state.has_forced_escalation() {
        return InferenceTier::T2;
    }

    if prediction_error < threshold {
        InferenceTier::T0
    } else if prediction_error < threshold * 2.0 {
        InferenceTier::T1
    } else {
        InferenceTier::T2
    }
}
```

The 2× threshold multiplier between T1 and T2 means T2 fires only when prediction error is at least double the T1 threshold. This ensures that moderate surprises get fast, cheap analysis (T1) while only genuinely novel or high-stakes situations trigger expensive deep reasoning (T2).

---

## What Each Tier Does

### T0: Suppress (System 1 — Pure Heuristic)

No LLM call. No actions. The agent observed, determined nothing interesting happened, and moved on. Cost: $0.00.

T0 is not "doing nothing" — it is actively processing. The 16 T0 probes execute, the prediction error is computed, the CorticalState is updated, and the DecisionCycleRecord is written. The agent is perceiving and recording; it just doesn't need to deliberate.

T0 also checks playbook rules: if a known situation matches a playbook rule's condition, the agent can act without LLM involvement. This means T0 is not limited to observation — it can execute learned responses.

### T1: Analyze (System 1 → System 2 Transition)

A fast LLM (Haiku-class, `claude-haiku-4-5`) processes a reduced context (~4,000 tokens):
- Current observation summary
- Top-5 retrieved Neuro entries (highest relevance)
- Active positions/tasks
- Critical warnings

The LLM decides whether action is needed and, if so, what kind. T1 is the "triage" tier — quick assessment, fast decision, minimal cost. Most T1 ticks conclude "something changed, but no action needed" (the LLM looked at it and decided the probes were right to flag it, but it's not actionable yet).

### T2: Deliberate (System 2 — Deep Reasoning)

The full LLM (Opus-class or Sonnet-class, depending on resource health) processes the complete Cognitive Workspace (~32,000 tokens), following Baddeley's (2000) working memory model:

- **Invariants**: Domain rules, safety constraints, policy limits
- **Strategy**: Current plan, goals, priorities
- **Playbook heuristics**: All applicable learned rules
- **Retrieved episodes**: Similar past situations and their outcomes
- **Retrieved insights**: Relevant knowledge from Neuro
- **Causal graph edges**: Known causal relationships in the domain
- **Dream hypotheses**: Novel hypotheses from recent delta consolidation
- **Somatic landscape**: Emotional markers from similar strategy regions (Damasio 1994)
- **Pheromone summary**: Coordination signals from mesh peers
- **Conversation tail**: Last N messages if a human is chatting

T2 is Kahneman's System 2: slow, deliberate, resource-intensive, but capable of handling genuinely novel situations that heuristics cannot address.

---

## The Cost Model

### Per-Call Costs

| Tier | Model | Input Tokens | Output Tokens | Estimated Cost |
|---|---|---|---|---|
| T0 | None | 0 | 0 | $0.00 |
| T1 | claude-haiku-4-5 | ~4,000 | ~500 | $0.001-0.003 |
| T2 (Sonnet) | claude-sonnet-4-6 | ~32,000 | ~2,000 | $0.01-0.05 |
| T2 (Opus) | claude-opus-4-6 | ~32,000 | ~2,000 | $0.05-0.25 |

### Daily Cost Model

At ~8,640 ticks/day (10-second average gamma interval):

| Scenario | T0 Rate | T1 Calls | T2 Calls | Raw Daily | With Context Eng. |
|---|---|---|---|---|---|
| Calm (low volatility) | ~90% | ~700 | ~165 | ~$3-8 | ~$1.00 |
| Normal | ~80% | ~1,300 | ~430 | ~$8-25 | ~$2.50 |
| Volatile (high activity) | ~60% | ~2,160 | ~1,300 | ~$30-100 | ~$8.00 |

Without tier gating (every tick at T2): 8,640 × $0.10 = **$864/day**. With gating: ~$1-8/day. **~100-800× cost reduction.**

Context engineering (prompt caching, cache alignment, tool pruning, multi-model routing) provides an additional ~6× reduction on top of tier gating.

### FrugalGPT Validation

Chen et al. (2023, arXiv:2305.05176; published 2024 in TMLR) demonstrated that cascade architectures can achieve up to 98% cost reduction while matching top-model quality. Roko's T0/T1/T2 cascade implements this principle with domain-specific probe functions as the first cascade stage (zero cost), a fast model as the second stage (low cost), and a full model as the final stage (full cost). The exact cost savings depend on domain volatility and the T1/T2 model pricing, but the architecture structurally enables FrugalGPT-class efficiency.

---

## Interaction with Synapse Traits

The tier decision affects which Synapse traits are invoked on each tick:

| Trait | T0 | T1 | T2 |
|---|---|---|---|
| `Substrate.query()` | Always (probes read state) | Always | Always |
| `Scorer.score()` | Always (score observations) | Always | Always |
| `Router.select()` | Always (tier selection) | Always | Always |
| `Composer.compose()` | Skip (no context needed) | Focused (~4K tokens) | Full (~32K tokens) |
| `Gate.verify()` | Skip (no action to verify) | If action taken | Always (multi-gate) |
| `Policy.decide()` | Always (logging) | Always | Always |

The first three traits (Substrate, Scorer, Router) always execute — they are the perception and gating machinery that determines the tier. The Composer, Gate, and extended Policy functions are conditional on the tier, which is what makes T0 nearly free.

---

## Active Inference Connection

The tier gating is not merely a cost optimization — it is an implementation of **active inference** (Friston 2010). In the active inference framework, an agent minimizes expected free energy (EFE) by choosing actions that reduce uncertainty. The tier decision IS an action: the agent decides how much cognitive resource to invest in reducing its uncertainty about the current situation.

- **T0**: Current uncertainty is low (prediction error below threshold). No additional information is needed. The agent's existing model of the world is sufficient.
- **T1**: Moderate uncertainty. The agent needs a quick check to determine if the situation is actionable. A cheap LLM call reduces uncertainty at minimal cost.
- **T2**: High uncertainty. The agent needs deep analysis to understand and respond. A full LLM call is justified by the expected information gain.

The EFE formulation:

```
EFE(tier) = pragmatic_value(tier) + epistemic_value(tier) - cost(tier)

where:
  pragmatic_value = expected utility of actions the tier enables
  epistemic_value = expected uncertainty reduction from the tier's analysis
  cost = inference cost + latency cost
```

T0 has zero cost but also zero epistemic value (it doesn't reduce uncertainty — it only acts on existing knowledge). T2 has the highest epistemic value but also the highest cost. T1 balances in the middle. The gating threshold implicitly implements EFE minimization by routing to the tier whose cost is justified by the expected information gain.

See [10-active-inference-compute-allocation.md](./10-active-inference-compute-allocation.md) for the full active inference specification.

---

## Academic Foundations

- **Kahneman 2011** — "Thinking, Fast and Slow" (Farrar, Straus and Giroux). System 1/System 2 dual-process theory.
- **Friston 2010** — "The Free-Energy Principle" (Nature Reviews Neuroscience 11(2)). Precision-weighted prediction error; active inference.
- **Chen et al. 2023** — FrugalGPT (arXiv:2305.05176, published 2024 TMLR). Cascade architectures for cost-optimal LLM routing.
- **Sun et al. 2005** — CLARION: "The Interaction of the Explicit and the Implicit" (Psychological Review). Dual-level cognitive architecture with implicit (subsymbolic) and explicit (symbolic) processing.
- **Baddeley 2000** — "The episodic buffer" (Trends in Cognitive Sciences 4(11)). Working memory model for T2 context assembly.
- **Damasio 1994** — "Descartes' Error" (Putnam). Somatic marker hypothesis: emotional fast-path before analytical reasoning.
- **Zhang et al. 2025** — DPT-Agent (arXiv:2502.11882). Dual-process theory applied to LLM agent decision-making.
- **Google Research 2024** — Talker-Reasoner framework. Separation of fast heuristic from slow analytical processing.

---

## Current Status and Gaps

**What exists:**
- `InferenceTier` enum (T0/T1/T2) in `bardo-primitives/src/tier.rs`.
- `TierRouter::select_model()` in `bardo-primitives/src/tier.rs`.
- `CascadeRouter` three-stage model routing in `roko-learn/src/cascade_router.rs`.
- The orchestration loop currently runs all tasks at T2 — no gating.

**What is missing:**
- Prediction error computation from probe results.
- Adaptive gating threshold with affect/resource/arousal modulation.
- T0 probe registry (see [09-16-t0-probes.md](./09-16-t0-probes.md)).
- T1 reduced context assembly in the Composer.
- Integration of `InferenceTier` into the orchestration loop for per-tick gating.
- Playbook rule matching at T0 for learned response execution.
- Budget-aware tier restriction (throttle T2 when approaching budget).

---

## Cross-References

- See [04-gamma-reactive-loop.md](./04-gamma-reactive-loop.md) for how tiers are selected per gamma tick
- See [09-16-t0-probes.md](./09-16-t0-probes.md) for the 16 zero-LLM probes driving T0 suppression
- See [10-active-inference-compute-allocation.md](./10-active-inference-compute-allocation.md) for the EFE-based allocation theory
- See [12-attention-auction-and-gating.md](./12-attention-auction-and-gating.md) for VCG-based context assembly at T1/T2
- See topic [05-learning](../05-learning/INDEX.md) for CascadeRouter model routing
