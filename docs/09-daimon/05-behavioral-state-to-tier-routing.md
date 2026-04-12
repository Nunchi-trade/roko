# Behavioral State to Tier Routing

> How the Daimon's behavioral state modulates the CascadeRouter's prediction error thresholds, creating a closed loop between emotional state and compute allocation.


> **Implementation**: Built

**Topic**: [Daimon](./INDEX.md)
**Prerequisites**: [04-six-behavioral-states.md](./04-six-behavioral-states.md), [01-pad-vector.md](./01-pad-vector.md)
**Key sources**: `roko-daimon/src/lib.rs`, `refactoring-prd/03-cognitive-subsystems.md` §2, `refactoring-prd/09-innovations.md` §I

---

## Abstract

The tier routing system determines how much compute the agent spends on each cognitive operation. The CascadeRouter (implemented in `roko-learn`) uses a prediction error scalar (0.0–1.0) from zero-cost probes to classify each operation into one of three tiers: T0 (free, deterministic probes only), T1 (fast model, shallow reasoning), or T2 (full model, deep reasoning). The Daimon's behavioral state modulates the thresholds between these tiers, creating a feedback loop: emotional state influences compute allocation, which influences task outcomes, which influence emotional state.

This feedback loop is the concrete mechanism by which affect controls resource efficiency. A Struggling agent spends more compute (lower T2 threshold → routes to deep reasoning sooner). A Coasting agent spends less compute (higher T2 threshold → stays on cheap models longer). Neither mode is permanently "correct" — the behavioral state adapts to the agent's ongoing experience.

---

## The Three-Tier Cognitive Architecture

### Tier Definitions

The three-tier system is inspired by dual-process theory (Kahneman 2011) extended with a zero-cost probe layer:

| Tier | Description | Cost | Latency | When Used |
|---|---|---|---|---|
| **T0** | 16 deterministic probes, no LLM | Zero | ~1ms | 80% of cognitive cycles — routine monitoring |
| **T1** | Fast model (haiku-class), shallow context | Low | ~2-5s | 15% of cycles — standard tasks, familiar territory |
| **T2** | Full model (opus-class), deep context | High | ~10-30s | 5% of cycles — novel situations, complex reasoning |

The tier selection is driven by a **prediction error scalar** (0.0–1.0), computed by the 16 zero-cost probes at every gamma tick (~5–15 seconds). Low prediction error means the world is behaving as expected — no need for deep reasoning. High prediction error means something unexpected happened — escalate to a stronger model.

### Default Thresholds

Without Daimon modulation, the CascadeRouter uses fixed thresholds:

```
prediction_error < 0.2  →  T0 (suppress, no LLM)
prediction_error < 0.6  →  T1 (fast model)
prediction_error ≥ 0.6  →  T2 (full model)
```

These thresholds produce the expected distribution: ~80% T0, ~15% T1, ~5% T2. The exact distribution depends on the task domain and current conditions — routine coding tasks with stable tests produce mostly T0, while novel feature development produces more T1/T2.

### Cost Implications

The cost differential between tiers is substantial. Using Claude model pricing as a reference:

| Tier | Model Class | Cost per 1M Input Tokens | Relative Cost |
|---|---|---|---|
| T0 | None | $0.00 | 0× |
| T1 | Haiku | ~$0.25 | 1× |
| T2 | Opus | ~$15.00 | 60× |

A 5% shift from T1 to T2 can increase total compute cost by approximately 3×. This makes the tier routing thresholds economically significant — the Daimon's modulation isn't cosmetic, it directly affects the agent's burn rate.

**Citation**: FrugalGPT (Chen et al. 2023, arXiv:2305.05176) demonstrated that cascade architectures can achieve substantial cost reduction while matching top-model quality through intelligent routing.

---

## Daimon Modulation Mechanism

### How Behavioral State Shifts Thresholds

The Daimon modulates tier routing by adjusting the prediction error thresholds:

```rust
/// Compute adjusted tier thresholds based on current behavioral state.
fn adjusted_thresholds(state: &BehavioralState) -> TierThresholds {
    match state {
        // Struggling: escalate sooner
        // Lower both thresholds → more T1 and T2, less T0
        BehavioralState::Struggling => TierThresholds {
            t0_ceiling: 0.10,  // was 0.2 — fewer cycles stay free
            t1_ceiling: 0.40,  // was 0.6 — escalate to T2 sooner
        },

        // Coasting: stay cheap longer
        // Raise both thresholds → more T0 and T1, less T2
        BehavioralState::Coasting => TierThresholds {
            t0_ceiling: 0.30,  // was 0.2 — more cycles stay free
            t1_ceiling: 0.80,  // was 0.6 — only escalate T2 for major anomalies
        },

        // Focused: exploit — similar to Coasting but less extreme
        BehavioralState::Focused => TierThresholds {
            t0_ceiling: 0.25,
            t1_ceiling: 0.70,
        },

        // Exploring: more T1 for breadth, keep T2 for research
        BehavioralState::Exploring => TierThresholds {
            t0_ceiling: 0.15,  // slightly lower — more goes to models
            t1_ceiling: 0.55,  // slightly lower — research benefits from T2
        },

        // Resting: T1 for dreams, T2 not warranted
        BehavioralState::Resting => TierThresholds {
            t0_ceiling: 0.20,
            t1_ceiling: 0.90,  // almost never T2 during maintenance
        },

        // Engaged: default thresholds
        BehavioralState::Engaged => TierThresholds {
            t0_ceiling: 0.20,
            t1_ceiling: 0.60,
        },
    }
}
```

### Expected Tier Distributions Under Each State

| State | T0 % | T1 % | T2 % | Compute Cost (Relative) |
|---|---|---|---|---|
| Engaged | 80% | 15% | 5% | 1.0× (baseline) |
| Struggling | 60% | 25% | 15% | ~3.5× |
| Coasting | 90% | 8% | 2% | ~0.4× |
| Focused | 85% | 12% | 3% | ~0.6× |
| Exploring | 70% | 20% | 10% | ~2.2× |
| Resting | 80% | 19% | 1% | ~0.5× |

These are approximate distributions based on a typical task mix. The actual distribution depends on the prediction error distribution of the current workload.

**Key insight**: Over a typical work session, an agent that transitions through all states (Engaged → Struggling → Resting → Exploring → Focused → Coasting → Engaged) averages roughly 1.0× cost because the expensive Struggling phases are balanced by cheap Coasting and Resting phases. The affect system provides **automatic cost regulation** — the agent spends more when it needs to and less when it doesn't.

---

## The Feedback Loop

### Closed-Loop Dynamics

The Daimon and tier routing form a closed feedback loop:

```
Affect state → Threshold modulation → Tier selection → Model quality → Task outcome
      ↑                                                                      │
      └──────────────────── Appraisal ───────────────────────────────────────┘
```

**Positive feedback (self-correcting)**: A Struggling agent routes more operations to T2 (opus-class models). Stronger models produce better task outcomes. Better outcomes increase pleasure and confidence. Higher pleasure and confidence transition the agent from Struggling to Engaged or Focused. The agent then reduces T2 usage, saving compute.

**Negative feedback (self-regulating)**: A Coasting agent routes almost everything to T0/T1. Cheaper models may produce lower-quality results on harder tasks. Lower-quality results decrease pleasure. Lower pleasure transitions from Coasting to Engaged, restoring the default thresholds.

### Stability Analysis

The feedback loop is stable because:

1. **Decay provides a restoring force**: PAD values decay toward zero with a 4-hour half-life. Without reinforcing events, every state eventually returns to Engaged (near-origin PAD).

2. **Model quality has diminishing returns**: Promoting from haiku to sonnet produces a larger quality improvement than promoting from sonnet to opus for most tasks. This means the Struggling → T2 escalation has diminishing marginal benefit, preventing runaway compute escalation.

3. **Tier thresholds are bounded**: The adjustments don't eliminate tiers entirely. Even a Struggling agent still routes 60% of cycles to T0 (free). Even a Coasting agent still uses T2 for 2% of cycles (genuine anomalies still escalate).

4. **Asymmetric appraisal prevents oscillation**: Gate failures produce 2× the pleasure impact of gate passes (prospect theory asymmetry). This means the agent can't rapidly oscillate between Struggling and Coasting — it takes more positive outcomes to recover from a failure streak than it took to enter the failure streak.

---

## Integration with CascadeRouter

### Current Architecture

The CascadeRouter (in `roko-learn`) maintains per-model performance statistics and uses a LinUCB bandit algorithm to select models. The Daimon modulation is applied as a **bias term** on the CascadeRouter's tier selection, not as a replacement for it:

```
final_tier = cascade_router.select_tier(
    prediction_error,
    adjusted_thresholds(behavioral_state),
    task_features,
)
```

The CascadeRouter considers:
1. Prediction error from probes (primary signal)
2. Daimon-adjusted thresholds (behavioral bias)
3. Task features (domain, complexity estimate)
4. Historical model performance (LinUCB posterior)

The Daimon bias shifts thresholds, but the CascadeRouter can override if its bandit model strongly suggests a different tier. This prevents the affect system from forcing T0 on a task that the router's learned model knows requires T2.

### Model Promotion/Demotion

Within a tier, the Daimon also affects which specific model is selected. The `modulate()` method in `roko-daimon/src/lib.rs` implements string-based model promotion and demotion:

```rust
fn promote_model(model: &str) -> String {
    if model.contains("haiku") {
        model.replacen("haiku", "sonnet", 1)
    } else if model.contains("sonnet") {
        model.replacen("sonnet", "opus", 1)
    } else {
        model.to_string()
    }
}

fn demote_model(model: &str) -> String {
    if model.contains("opus") {
        model.replacen("opus", "sonnet", 1)
    } else if model.contains("sonnet") {
        model.replacen("sonnet", "haiku", 1)
    } else {
        model.to_string()
    }
}
```

This is a coarse heuristic — it operates on model name strings rather than a structured model registry. The CascadeRouter's bandit model provides the fine-grained selection within the promoted/demoted tier.

---

## Turn Limit Modulation

In addition to tier routing, the Daimon modulates the turn limit (maximum number of agent turns before timeout):

| State | Turn Limit Adjustment | Rationale |
|---|---|---|
| Struggling (Escalating) | +10 turns | Give the agent more attempts with the stronger model |
| Struggling (Conservative) | -3 turns | Fail fast — don't waste turns on an approach that isn't working |
| Coasting | -5 turns | Tasks are easier — complete faster |
| Resting | +5 turns | Maintenance tasks can take longer without urgency |
| Engaged | 0 (default) | Standard turn budget |

The turn limit affects both cost (more turns = more API calls) and latency (more turns = longer wall-clock time per task). The Struggling/Escalating combination (+10 turns with a promoted model) is the most expensive configuration — it's reserved for situations where the agent is genuinely stuck and needs to try harder.

---

## Current Status and Gaps

**Implemented**: Model promotion/demotion in `roko-daimon/src/lib.rs`. Turn limit adjustments in `modulate()`. Dispatch strategy selection based on PAD thresholds. Effort labeling for cost tracking.

**Gap**: The Daimon's threshold adjustments are not yet wired to the CascadeRouter's `select_tier()` method. The CascadeRouter uses its own fixed thresholds. Wiring this requires the CascadeRouter to accept a `TierThresholds` parameter from the Daimon.

**Gap**: The 16 zero-cost probes (Section I of `09-innovations.md`) are specified but not all implemented. The prediction error scalar that drives tier selection is not yet computed from the full probe set.

**Gap**: The interaction between Daimon tier bias and CascadeRouter's LinUCB bandit is not yet defined. The bandit may learn to override the Daimon bias if it consistently produces suboptimal results, but this interaction needs explicit specification.

---

## Academic Foundations

- Kahneman, D. (2011). *Thinking, Fast and Slow*. Farrar, Straus and Giroux.
- Chen, L. et al. (2023). "FrugalGPT: How to Use Large Language Models While Reducing Cost and Improving Performance." arXiv:2305.05176.
- Li, L., Chu, W., Langford, J., & Schapire, R.E. (2010). "A contextual-bandit approach to personalized news article recommendation." In *Proceedings of the 19th International Conference on World Wide Web*, pp. 661–670.
- Mehrabian, A. (1996). "Pleasure-arousal-dominance: A general framework for describing and measuring individual differences in temperament." *Current Psychology*, 14(4), 261–292.

---

## Cross-references

- See [04-six-behavioral-states.md](./04-six-behavioral-states.md) for behavioral state definitions and PAD thresholds
- See [10-integration-points.md](./10-integration-points.md) for how tier routing connects with VCG bidding and dispatch
- See topic [05-learning](../05-learning/INDEX.md) for CascadeRouter and LinUCB algorithm details
- See [13-current-status-and-gaps.md](./13-current-status-and-gaps.md) for wiring gaps
