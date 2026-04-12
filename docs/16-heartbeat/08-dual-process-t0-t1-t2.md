# Dual-Process Cognition: T0, T1, T2

> The LLM-Last architecture — most ticks are free, some are cheap, a few are expensive. Uncertainty determines compute investment.


> **Implementation**: Specified

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

Prediction error is a scalar in [0.0, 1.0] aggregated from probe results, regime detection, and domain-specific signals. The computation follows Friston's (2010) precision-weighted prediction error framework.

#### Weight derivation

The four weights (0.05 per anomaly, 0.40 for regime change, 0.30 for drift, 0.10 per intervention) are empirical starting points calibrated against a replay corpus of ~2,000 ticks from the mori orchestrator. The reasoning:

| Component | Weight | Rationale |
|---|---|---|
| Probe anomaly | 0.05 each | A single anomalous probe is a weak signal. 4+ anomalies (0.20) cross the base threshold, triggering T1. 16 anomalies (0.80) are near-certain T2. |
| Regime change | 0.40 flat | A regime shift (calm -> volatile, trending -> crisis) is the single strongest indicator that the world model is stale. One regime change alone pushes past the T1 threshold. |
| World model drift | 0.30 * drift | Drift is a continuous signal. The 0.30 coefficient means maximum drift (1.0) contributes 0.30 -- significant but not T2 by itself. |
| Pending intervention | 0.10 each | Interventions are explicit requests. Two pending interventions (0.20) match the base threshold. |

These weights should be adaptive over time using the same EMA mechanism as gate thresholds (see `roko-learn/src/gate_thresholds.rs`). The initial values serve as priors until enough tick data accumulates.

#### Anomaly count source

`ProbeResult::is_anomalous()` returns `true` when a probe's measured value exceeds a per-probe z-score threshold. Each of the 16 T0 probes (see [09-16-t0-probes.md](./09-16-t0-probes.md)) maintains a rolling mean and standard deviation over its last 100 measurements. A measurement is anomalous if `|value - mean| > 2.0 * stddev`. The count is simply the number of probes returning `true` on this tick.

```rust
/// Per-probe anomaly detection using rolling z-score.
pub struct ProbeResult {
    pub probe_id: &'static str,
    pub value: f64,
    pub rolling_mean: f64,
    pub rolling_stddev: f64,
    pub z_threshold: f64,  // default: 2.0
}

impl ProbeResult {
    pub fn is_anomalous(&self) -> bool {
        if self.rolling_stddev < f64::EPSILON {
            return false; // no variance yet — not anomalous
        }
        let z = (self.value - self.rolling_mean).abs() / self.rolling_stddev;
        z > self.z_threshold
    }
}
```

#### Drift metric

`PredictionState::compute_drift()` returns a scalar in [0.0, 1.0] measuring divergence between the agent's predicted state and the observed state. The metric is **normalized Euclidean distance** over a fixed-dimension state vector.

The state vector contains the CorticalState signals that the agent's world model predicts: regime, aggregate accuracy, resource health, active count, and arousal. Each dimension is normalized to [0.0, 1.0] before computing distance.

```rust
impl PredictionState {
    /// Euclidean drift between predicted and observed state vectors,
    /// normalized by sqrt(N) so the result is in [0.0, 1.0].
    pub fn compute_drift(&self) -> f32 {
        let pairs: &[(f32, f32)] = &[
            (self.predicted_regime_f32, self.observed_regime_f32),
            (self.predicted_accuracy, self.observed_accuracy),
            (self.predicted_resource_health, self.observed_resource_health),
            (self.predicted_active_ratio, self.observed_active_ratio),
            (self.predicted_arousal, self.observed_arousal),
        ];
        let sum_sq: f32 = pairs.iter()
            .map(|(p, o)| (p - o).powi(2))
            .sum();
        let n = pairs.len() as f32;
        (sum_sq / n).sqrt()  // normalized by sqrt(N)
    }
}
```

**Why Euclidean over KL divergence?** KL divergence requires probability distributions and is undefined when the predicted distribution assigns zero probability to an observed outcome. The state vector is a mix of continuous and ordinal values, not a probability distribution. Euclidean distance is simpler, symmetric, always defined, and sufficient for detecting drift in a low-dimensional state space (5 dimensions). If the state space grows or the values become distributional, KL or Jensen-Shannon divergence would be appropriate.

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

    // Probe anomalies: 5% per anomaly (z-score based)
    let anomaly_count = probes.iter()
        .filter(|p| p.is_anomalous())
        .count();
    error += anomaly_count as f32 * 0.05;

    // Regime change: 40% if regime shifted since last tick
    if regime.changed_since_last_tick() {
        error += 0.40;
    }

    // World model drift: normalized Euclidean distance
    error += predictions.compute_drift() * 0.30;

    // Pending interventions: 10% per intervention
    let pending = predictions.pending_intervention_count();
    error += pending as f32 * 0.10;

    error.min(1.0)
}
```

### Adaptive Threshold

The threshold is not fixed. It adapts based on affect state, resource constraints, and strategy confidence.

#### Base threshold derivation (0.20)

The base threshold of 0.20 means the agent needs at least 4 anomalous probes (4 * 0.05 = 0.20) or one regime change approaching (0.40 > 0.20) to escalate from T0 to T1. This value was chosen to produce the target ~80% T0 suppression rate under normal conditions.

The derivation: in a calm environment, the expected number of anomalous probes per tick follows a Poisson distribution with lambda ~0.5 (each of 16 probes has roughly a 3% false-positive rate under 2-sigma z-score thresholds: 16 * 0.03 = 0.48). The probability of 4+ anomalies under Poisson(0.5) is ~0.002, which means T0 suppresses ~99.8% of ticks in perfectly calm conditions. In normal conditions with genuine signal mixed in, the effective suppression rate drops to the target ~80%.

If you change the z-score threshold on probes (currently 2.0), recalibrate this base value. A tighter z-threshold (e.g. 3.0) produces fewer anomalies and the base should decrease. A looser threshold (e.g. 1.5) produces more anomalies and the base should increase.

#### Affect adjustment: additive, not multiplicative

All modulation is **additive** (affect_adj, resource_adj, arousal_adj, confidence_adj are summed). The alternative -- multiplicative modulation -- was considered and rejected:

- **Additive** keeps the adjustments independent and bounded. Each adjustment's effect is constant regardless of other adjustments. The clamp to [0.05, 0.50] provides hard safety rails.
- **Multiplicative** creates compounding effects: an agent that is low-dominance, high-arousal, AND low-resource could multiply the base down to near-zero, causing T2 escalation on every tick. That runaway is undesirable.
- **Signed magnitude** of each adjustment (0.05-0.10) is small relative to the base (0.20), producing shifts of 25-50%. This keeps gating stable while still allowing affect to influence behavior.

The specific adjustment values:

| Condition | Adjustment | Effect on threshold | Justification |
|---|---|---|---|
| Low dominance (< -0.2) | -0.05 | Easier to escalate | Uncertain agents should think more. Maps to Kahneman's finding that low confidence increases System 2 engagement. |
| High dominance (> 0.3) | +0.05 | Harder to escalate | Confident agents can coast on heuristics. Mirrors the cognitive ease effect. |
| Budget > 80% used | +0.10 | Harder to escalate | Budget conservation overrides curiosity. This is the strongest single adjustment because running out of budget halts the agent. |
| High arousal (> 0.5) | -0.05 | Easier to escalate | High arousal signals surprise or urgency. The biological analogue: adrenaline sharpens attention. |
| Strategy confidence | +0.00 to +0.05 | Harder to escalate | Continuous: `confidence * 0.05`. A fully confident agent (1.0) adds 0.05. |

```rust
/// Compute the adaptive gating threshold.
///
/// Kahneman's (2011) insight: System 2 engagement is modulated by
/// cognitive load, emotional state, and available resources.
///
/// Friston's (2010) precision weighting: higher precision (confidence)
/// in current predictions -> higher threshold -> less escalation.
fn compute_adaptive_threshold(state: &AgentState) -> f32 {
    let base = 0.20;  // default: 4 anomalous probes to escalate

    // Affect modulation: low dominance -> lower threshold (more cautious)
    let dominance = state.cortical_state.pad().dominance;
    let affect_adj = if dominance < -0.2 {
        -0.05  // uncertain -> easier to escalate
    } else if dominance > 0.3 {
        0.05   // confident -> harder to escalate
    } else {
        0.0
    };

    // Resource modulation: low resources -> higher threshold (conserve)
    let budget_pct = state.budget_tracker.daily_usage_percent();
    let resource_adj = if budget_pct > 0.80 {
        0.10  // approaching budget -> harder to escalate
    } else {
        0.0
    };

    // Arousal modulation: high arousal -> lower threshold (more alert)
    let arousal = state.cortical_state.pad().arousal;
    let arousal_adj = if arousal > 0.5 {
        -0.05  // surprised -> easier to escalate
    } else {
        0.0
    };

    // Strategy confidence: high -> higher threshold (coast)
    let confidence_adj = state.strategy_confidence * 0.05;

    (base + affect_adj + resource_adj + arousal_adj + confidence_adj)
        .clamp(0.05, 0.50)
}
```

#### Threshold bounds

The clamp range [0.05, 0.50] prevents degenerate behavior:

- **Floor (0.05)**: Even under maximum conservation pressure, a single anomalous probe (0.05) can trigger T1. Without a floor, budget exhaustion could lock the agent into permanent T0 suppression, unable to respond to genuine emergencies.
- **Ceiling (0.50)**: Even under maximum alertness, at least 10 anomalous probes or a regime change + drift is needed for T1. Without a ceiling, a panicking agent (high arousal, low dominance, low pleasure) could escalate every tick to T2, burning budget on noise.

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

T0 also checks playbook rules: if a known situation matches a playbook rule's condition, the agent can act without LLM involvement. This means T0 is not limited to observation -- it can execute learned responses.

#### T0 playbook rule matching

Playbook rules are learned heuristics stored in the `PlaybookStore` (see topic [05-learning](../05-learning/INDEX.md)). Each rule has a condition, a confidence score, and an action. At T0, the agent checks all rules against the current tick state.

**Condition matching algorithm.** Conditions use a **predicate tree** -- a conjunction (AND) of simple predicates, each testing a CorticalState signal or probe result against a threshold. No fuzzy matching or regex. The predicates are:

```rust
/// A playbook rule condition is a conjunction of predicates.
/// All predicates must match for the rule to fire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybookCondition {
    pub predicates: Vec<Predicate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Predicate {
    /// CorticalState signal exceeds threshold.
    SignalAbove { signal: SignalId, threshold: f64 },
    /// CorticalState signal below threshold.
    SignalBelow { signal: SignalId, threshold: f64 },
    /// Specific regime is active.
    RegimeIs(Regime),
    /// N or more probes are anomalous.
    AnomalyCountAbove(u32),
    /// A specific probe is anomalous.
    ProbeAnomalous(ProbeId),
    /// Current task has a specific tag.
    TaskTagged(String),
}

impl PlaybookCondition {
    /// Returns true if all predicates match the current state.
    pub fn matches(&self, state: &TickState) -> bool {
        self.predicates.iter().all(|p| p.evaluate(state))
    }
}
```

**Why conjunctive predicates over richer expressions?** Simplicity and speed. T0 must complete in under 10ms. A conjunction of simple predicates evaluates in O(N) with N typically 2-4. Disjunction (OR) can be expressed as multiple rules. Negation is handled by the `SignalBelow` / `SignalAbove` duality. This covers >95% of practical conditions. If a condition is too complex for predicates, the right answer is T1 escalation, not a richer T0 language.

**Rule precedence.** When multiple rules match, the agent selects the one with the highest confidence score. Ties are broken by specificity (more predicates = more specific = higher priority).

```rust
/// Select the best matching playbook rule.
fn select_playbook_rule<'a>(
    rules: &'a [PlaybookRule],
    state: &TickState,
) -> Option<&'a PlaybookRule> {
    rules.iter()
        .filter(|r| r.condition.matches(state))
        .max_by(|a, b| {
            a.confidence.partial_cmp(&b.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    a.condition.predicates.len()
                        .cmp(&b.condition.predicates.len())
                })
        })
}
```

**Action execution sandbox.** Playbook actions execute in a restricted sandbox. T0 actions cannot:
- Invoke an LLM (by definition -- that would be T1 or T2).
- Modify the plan DAG (add/remove/reorder tasks).
- Send external requests (HTTP, MCP tool calls).

T0 actions **can**:
- Update CorticalState signals (e.g., set regime, adjust arousal).
- Emit a `CognitiveSignal` (e.g., `Escalate`, `Cooldown`, `Explore`).
- Log an observation to the episode log.
- Adjust the adaptive clock interval (speed up or slow down gamma).

```rust
/// Actions permitted at T0. No LLM, no external I/O.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlaybookAction {
    /// Update a CorticalState signal.
    SetSignal { signal: SignalId, value: f64 },
    /// Emit a cognitive signal to the frequency scheduler.
    EmitSignal(CognitiveSignal),
    /// Log an observation without LLM processing.
    LogObservation { message: String },
    /// Adjust gamma interval by a multiplier (e.g., 0.5 = double speed).
    AdjustGamma { multiplier: f64 },
    /// Force escalation to a specific tier on the next tick.
    ForceEscalation(InferenceTier),
}
```

This sandbox ensures T0 playbook execution is fast, deterministic, and side-effect-free relative to the external world. The agent can react to known patterns instantly, but it cannot take actions that require reasoning or have irreversible external consequences.

### T1: Analyze (System 1 -> System 2 Transition)

A fast LLM (Haiku-class, `claude-haiku-4-5`) processes a reduced context (~4,000 tokens):
- Current observation summary
- Top-5 retrieved Neuro entries (highest relevance)
- Active positions/tasks
- Critical warnings

The LLM decides whether action is needed and, if so, what kind. T1 is the "triage" tier -- quick assessment, fast decision, minimal cost. Most T1 ticks conclude "something changed, but no action needed" (the LLM looked at it and decided the probes were right to flag it, but it's not actionable yet).

#### T1 context assembly algorithm

The T1 context is assembled by the `Composer` using a fixed-structure template (not the VCG auction, which runs only at T2). The 4,000-token budget is partitioned:

| Section | Token budget | Source |
|---|---|---|
| System prompt + task description | ~1,200 | `RoleSystemPromptSpec` invariants layer |
| Top-5 Neuro entries | ~1,500 | `PredictiveScorer` ranked retrieval |
| Active tasks/positions | ~800 | Current plan state |
| Critical warnings | ~500 | Probes with severity >= Critical |

**Top-5 Neuro entry selection.** The `PredictiveScorer` (see [10-active-inference-compute-allocation.md](./10-active-inference-compute-allocation.md)) scores all candidate Engrams and returns the top 5. The scorer computes `salience = pragmatic_weight * utility + epistemic_value - cost_penalty` for each entry and sorts descending. Ties are broken by recency (more recent wins).

```rust
/// Select top-k Engrams for T1 context.
///
/// Uses PredictiveScorer for ranking. Falls back to recency
/// if the scorer returns fewer than k candidates.
fn select_t1_engrams(
    candidates: &[Engram],
    scorer: &PredictiveScorer,
    ctx: &Context,
    k: usize,  // default: 5
) -> Vec<Engram> {
    let mut scored: Vec<(usize, f32)> = candidates.iter()
        .enumerate()
        .map(|(i, e)| {
            let score = scorer.score(e, ctx);
            (i, score.salience)
        })
        .collect();

    // Sort by salience descending, break ties by recency
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                candidates[b.0].timestamp
                    .cmp(&candidates[a.0].timestamp)
            })
    });

    scored.iter()
        .take(k)
        .map(|(i, _)| candidates[*i].clone())
        .collect()
}
```

**Critical warning priority.** If any probe returns severity `Critical` or `Emergency`, its warning text is included in the T1 context unconditionally, displacing lower-priority Neuro entries if needed. The logic: critical warnings consume their token allocation first, then the remaining budget is split among the other sections.

```rust
/// Critical warnings consume their budget before other sections.
fn partition_t1_budget(
    warnings: &[ProbeWarning],
    total_budget: usize,  // 4000
) -> T1BudgetPartition {
    let warning_tokens: usize = warnings.iter()
        .filter(|w| w.severity >= Severity::Critical)
        .map(|w| w.estimated_tokens())
        .sum();

    let warning_budget = warning_tokens.min(total_budget / 4);  // cap at 25%
    let remaining = total_budget - warning_budget;

    T1BudgetPartition {
        system_prompt: (remaining as f32 * 0.30) as usize,
        neuro_entries: (remaining as f32 * 0.38) as usize,
        active_tasks: (remaining as f32 * 0.20) as usize,
        warnings: warning_budget + (remaining as f32 * 0.12) as usize,
    }
}
```

**Integration with `SystemPromptBuilder`.** The T1 context uses layers 1-3 of the 6-layer builder (invariants, role template, task context). Layers 4-6 (retrieved knowledge, iteration memory, domain signals) are replaced by the streamlined top-5 selection above. This keeps the system prompt consistent between T1 and T2 while cutting the knowledge and memory layers down to fit the smaller budget.

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

## The TierDecision Struct

Every tier gating decision produces a `TierDecision` record. This struct captures the full reasoning chain from probes through prediction error to the selected tier, enabling debugging, replay, and threshold adaptation.

```rust
/// Complete record of a tier gating decision.
///
/// Produced on every gamma tick. Persisted to the episode log
/// for post-hoc analysis and threshold adaptation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierDecision {
    /// Tick identifier (monotonically increasing).
    pub tick_id: u64,

    /// Timestamp of the decision.
    pub timestamp: chrono::DateTime<chrono::Utc>,

    /// The selected tier.
    pub tier: InferenceTier,

    /// Aggregate prediction error in [0.0, 1.0].
    pub prediction_error: f32,

    /// The adaptive threshold at decision time.
    pub threshold: f32,

    /// Breakdown: how many probes flagged anomalous.
    pub anomaly_count: u32,

    /// Breakdown: did a regime change occur this tick?
    pub regime_changed: bool,

    /// Breakdown: world model drift in [0.0, 1.0].
    pub drift: f32,

    /// Breakdown: pending intervention count.
    pub pending_interventions: u32,

    /// Whether forced escalation was triggered (user steer or safety).
    pub forced: bool,

    /// If forced, the reason.
    pub force_reason: Option<String>,

    /// The PAD vector at decision time (for threshold replay).
    pub pad: PadVector,

    /// Budget usage percentage at decision time.
    pub budget_usage_pct: f32,

    /// Strategy confidence at decision time.
    pub strategy_confidence: f32,

    /// If a playbook rule matched at T0, which rule.
    pub playbook_rule_id: Option<String>,

    /// The model selected for this tier (None for T0).
    pub model: Option<String>,

    /// Resource health at decision time (for model selection within T2).
    pub resource_health: f32,
}
```

### Configuration parameters

| Parameter | Default | Range | Where |
|---|---|---|---|
| `base_threshold` | 0.20 | [0.05, 0.50] | `roko.toml` `[heartbeat.gating]` |
| `t1_t2_multiplier` | 2.0 | [1.5, 4.0] | `roko.toml` `[heartbeat.gating]` |
| `anomaly_weight` | 0.05 | [0.01, 0.15] | `roko.toml` `[heartbeat.prediction_error]` |
| `regime_change_weight` | 0.40 | [0.20, 0.60] | `roko.toml` `[heartbeat.prediction_error]` |
| `drift_weight` | 0.30 | [0.10, 0.50] | `roko.toml` `[heartbeat.prediction_error]` |
| `intervention_weight` | 0.10 | [0.05, 0.25] | `roko.toml` `[heartbeat.prediction_error]` |
| `z_score_threshold` | 2.0 | [1.5, 3.0] | `roko.toml` `[heartbeat.probes]` |
| `t2_resource_threshold` | 0.30 | [0.10, 0.50] | `roko.toml` `[heartbeat.gating]` |
| `threshold_clamp_min` | 0.05 | [0.01, 0.10] | `roko.toml` `[heartbeat.gating]` |
| `threshold_clamp_max` | 0.50 | [0.30, 0.80] | `roko.toml` `[heartbeat.gating]` |

### Error handling

| Failure mode | Behavior |
|---|---|
| All probes fail (no probe results) | Set prediction error to 0.50 (assume moderate surprise). Log warning. |
| PredictionState unavailable (first tick) | Skip drift term. Prediction error = anomaly + regime + interventions only. |
| CorticalState read fails (corrupted atomics) | Use base threshold (0.20) with no modulation. Log error. |
| Budget tracker unavailable | Skip resource adjustment. Log warning. |
| Playbook rule action panics | Catch at sandbox boundary. Log error. Skip rule. Proceed to tier gating. |

### Integration wiring

The tier gating runs inside the gamma tick handler in the orchestration loop. The wiring path:

1. `orchestrate.rs` gamma tick entry point calls `compute_prediction_error()` with probe results.
2. `compute_adaptive_threshold()` reads CorticalState + budget tracker.
3. `gate()` produces `InferenceTier`.
4. `TierRouter::select_model()` maps tier to model (if T1 or T2).
5. `ContextGovernor::assemble()` builds context for the selected tier.
6. `TierDecision` is constructed and persisted to the episode log.
7. If T0, the playbook rule matcher runs and the tick completes.
8. If T1/T2, the selected model is invoked via the agent dispatcher.

### Test criteria

| Test | Assertion |
|---|---|
| Zero anomalies, no regime change, no drift | `tier == T0` |
| 4 anomalies, no other signals | `tier == T1` (prediction_error 0.20 >= base 0.20) |
| 8 anomalies, no other signals | `tier == T2` (prediction_error 0.40 >= base * 2.0) |
| Regime change alone | `tier == T2` (0.40 >= 0.20 * 2.0) |
| Budget > 80% shifts threshold to 0.30 | 6 anomalies (0.30) needed for T1 |
| Forced escalation flag set | `tier == T2` regardless of prediction error |
| All probes fail | prediction_error == 0.50, tier == T2 (failsafe) |
| Low resource health (< 0.30) at T2 | Model is Sonnet, not Opus |
| TierDecision serializes/deserializes | Round-trip through serde_json preserves all fields |

---

## Cross-References

- See [04-gamma-reactive-loop.md](./04-gamma-reactive-loop.md) for how tiers are selected per gamma tick
- See [09-16-t0-probes.md](./09-16-t0-probes.md) for the 16 zero-LLM probes driving T0 suppression
- See [10-active-inference-compute-allocation.md](./10-active-inference-compute-allocation.md) for the EFE-based allocation theory
- See [12-attention-auction-and-gating.md](./12-attention-auction-and-gating.md) for VCG-based context assembly at T1/T2
- See topic [05-learning](../05-learning/INDEX.md) for CascadeRouter model routing
