# 11 — Dual-Process Tier Routing

> Sub-doc 11 of **02-agents** · Roko Documentation
>
> This document describes the dual-process cognitive model (System 1 / System 2)
> as applied to Roko's model tier routing, the CascadeRouter, the LinUCB bandit,
> Pareto frontier computation, and anomaly detection.


> **Implementation**: Shipping

---

## The Cognitive Model

Roko's model routing is inspired by dual-process theory from cognitive science
(Kahneman, 2011):

- **System 1** (fast, automatic) — Quick, pattern-matching responses. Low cost,
  low latency. Maps to the Fast model tier (Haiku-class).
- **System 2** (slow, deliberate) — Careful reasoning, multi-step analysis.
  Higher cost, higher quality. Maps to the Premium model tier (Opus-class).

The insight: most agent tasks don't need System 2. Classification, validation,
orchestration overhead, and simple code changes can be handled by fast models.
Only hard debugging, architectural decisions, and complex reasoning need premium
models. Routing everything through premium models wastes money without improving
outcomes.

---

## Three Model Tiers

```rust
pub enum ModelTier {
    Fast,      // Haiku-class: classification, watchers, orchestration
    Standard,  // Sonnet-class: implementation, review (the workhorse)
    Premium,   // Opus/GPT-5-class: architecture, hard debugging
}
```

| Tier | Examples | Typical cost | Use cases |
|---|---|---|---|
| Fast | Claude Haiku, GPT-4o-mini | ~$0.25/M input | Watchers, validators, conductors |
| Standard | Claude Sonnet, GPT-4o | ~$3/M input | Implementation, review, testing |
| Premium | Claude Opus, GPT-5 | ~$15/M input | Architecture, hard debug, audit |

Each role has a default tier (sub-doc 04), but the CascadeRouter can override
it dynamically based on learned performance data.

---

## CascadeRouter

The CascadeRouter implements the dual-process model as a multi-stage
confidence cascade:

```
Task arrives
    │
    ▼
Stage 1: Try Fast model (System 1)
    │
    ├── Confidence ≥ threshold → Accept result, done
    │
    ▼
Stage 2: Try Standard model
    │
    ├── Confidence ≥ threshold → Accept result, done
    │
    ▼
Stage 3: Try Premium model (System 2)
    │
    └── Accept result regardless
```

The "confidence" signal comes from multiple sources:
1. **Gate results** — Did the output pass compile/test/clippy gates?
2. **Self-assessment** — Did the model express uncertainty in its output?
3. **Historical performance** — For similar tasks, which tier succeeded?
4. **Cost-quality tradeoff** — Given the budget, is escalation worth it?

### Confidence computation

The confidence score is a weighted combination of signals:

```
confidence = w1 × gate_pass_rate
           + w2 × (1 - uncertainty_markers)
           + w3 × historical_success_rate
           + w4 × task_complexity_estimate
```

The weights are learned via the LinUCB bandit (see below). The threshold
for accepting a fast-model result depends on the temperament setting
(sub-doc 10): Conservative requires 0.9 confidence, Balanced requires 0.7.

### Persistence

The CascadeRouter persists its state to `.roko/learn/cascade-router.json`.
This means routing decisions improve across sessions — a model that
consistently fails for a task type will be avoided in future runs.

---

## LinUCB Bandit

The CascadeRouter uses a **LinUCB contextual bandit** (Li et al., 2010,
"A contextual-bandit approach to personalized news article recommendation")
to select models within each tier:

### How it works

1. **Context vector** — For each task, compute features: task type, estimated
   complexity, historical performance, role, current budget.
2. **Arm selection** — Each model is an "arm". LinUCB computes an upper
   confidence bound for each arm given the context.
3. **Reward** — After the task completes, the reward signal combines: gate
   pass/fail, token efficiency, wall-clock time, cost.
4. **Update** — The bandit updates its weight matrix for the selected arm.

LinUCB balances exploration (trying new models to learn their performance)
with exploitation (using the model that's historically best for this context).
The exploration parameter is controlled by temperament: Exploratory temperament
sets a high exploration parameter, causing the bandit to try more models.

### Pareto frontier pruning

Before the bandit selects a model, a Pareto frontier computation prunes the
candidate set. Models are evaluated on two dimensions:

1. **Quality** — Historical gate pass rate for the task type
2. **Cost** — Price per million tokens

Models that are dominated (worse on both dimensions than another model) are
removed from consideration. This prevents the bandit from exploring obviously
bad options.

Implementation plan `modelrouting/2G.10` describes the Pareto computation.
Implementation plan `modelrouting/2G.11` describes applying Pareto pruning
to the LinUCB exploration set.

---

## Thompson Sampling

For the confidence-threshold decision (escalate or accept?), the CascadeRouter
uses Thompson sampling over the weighted confidence signals:

```
For each tier t:
    sample θ_t ~ Beta(successes_t, failures_t)
    adjusted_confidence = θ_t × raw_confidence
```

This introduces beneficial randomness: even when the fast model's average
confidence is below threshold, it occasionally gets a chance (when the sampled
θ is high), allowing the system to discover that the fast model has improved
for certain task types.

---

## Anomaly Detection

Implementation plan `modelrouting/2G.12` introduces an `AnomalyDetector`
that monitors model performance for unusual patterns:

```rust
pub struct AnomalyDetector {
    // Tracks per-model running statistics
    // Flags when a model's recent performance deviates significantly
    // from its historical baseline
}
```

The detector watches for:
- **Sudden quality drops** — A model that was passing gates 90% of the time
  drops to 50% (provider degradation, model update).
- **Latency spikes** — Response times exceed 2× the rolling average.
- **Cost anomalies** — Token usage significantly higher than expected for
  the task type.

When an anomaly is detected, the router temporarily de-prioritizes the
affected model and fires an alert to the Monitor role.

Implementation plan `modelrouting/2G.16` wires the AnomalyDetector into
the dispatch pipeline.

---

## Three Cognitive Speeds

The dual-process model maps to three cognitive speeds in Roko's execution:

| Speed | Latency | Description | Example |
|---|---|---|---|
| **Gamma** | ~5-15s | Fast reflexive response | File read, simple classification |
| **Theta** | ~75s | Standard deliberation | Implementation, code review |
| **Delta** | Hours | Deep reasoning | Architecture, research, complex debug |

The CascadeRouter uses these speeds as a prior: Gamma tasks start at Fast
tier, Theta tasks start at Standard, Delta tasks start at Premium. The
bandit can override these starting points based on learned performance.

---

## Active Inference Connection

The model routing system is theoretically grounded in the Free Energy
Principle (Friston, 2010). The CascadeRouter's behavior can be interpreted
as minimizing expected free energy:

- **Epistemic value** — Exploration (trying new models) reduces uncertainty
  about model performance, lowering expected free energy.
- **Pragmatic value** — Exploitation (using known-good models) directly
  achieves task objectives.
- **Confidence threshold** — The threshold acts as a precision parameter:
  high precision (Conservative) demands more evidence before accepting,
  low precision (Exploratory) accepts with less evidence.

This connection is documented in refactoring PRD §01-synapse-architecture
and provides the theoretical basis for why the bandit approach works:
it naturally balances the explore-exploit tradeoff in a principled way.

---

## Research Context

The model routing approach draws on several research directions:

1. **RouteLLM** (2024) — Binary classifier for cheap/expensive model routing.
   Roko extends this to a multi-tier cascade with contextual bandits.
2. **MixLLM** (2024) — Mixed model serving with learned routing policies.
3. **FrugalGPT** (Chen et al., 2023) — Cost-efficient LLM serving via
   cascading and caching. The cascade structure is similar.
4. **AutoMix** (2024) — Automatic model mixing based on query difficulty.
5. **Router-R1** (2025) — RL-trained router that learns per-query routing.

Implementation plan `modelrouting/11-research-context.md` provides full
citations and comparative analysis for each approach.

Roko's contribution is combining these approaches into a unified system:
Pareto pruning (from multi-objective optimization) → LinUCB selection (from
contextual bandits) → Thompson sampling for confidence (from Bayesian
decision theory) → anomaly detection for robustness.

---

## Citations

1. Kahneman, D. (2011). "Thinking, Fast and Slow." — Dual-process theory.
2. Li, L. et al. (2010). "A contextual-bandit approach to personalized news
   article recommendation." WWW 2010. — LinUCB algorithm.
3. Chen, L. et al. (2023). "FrugalGPT: How to Use Large Language Models
   While Reducing Cost and Improving Performance." — Cascade routing.
4. Friston, K. (2010). "The free-energy principle: a unified brain theory?"
   Nature Reviews Neuroscience. — Active inference basis.
5. Implementation plans `modelrouting/2G.10` through `modelrouting/2G.17` —
   Pareto frontier, LinUCB pruning, anomaly detection, retry actions.
6. Implementation plan `modelrouting/11-research-context.md` — RouteLLM,
   MixLLM, FrugalGPT, AutoMix, Router-R1 analysis.
7. Refactoring PRD §02-five-layers — Dual-Process Tier Router specification.
8. `.roko/learn/cascade-router.json` — Persisted routing state.
