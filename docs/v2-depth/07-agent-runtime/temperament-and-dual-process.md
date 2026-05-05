# Temperament Profiling and Dual-Process Routing

> Depth for [05-AGENT.md](../../unified/05-AGENT.md). Temperament as a Score protocol Cell that rates agent behavioral tendencies along PAD dimensions. Dual-process routing (T0/T1/T2) as a Route Cell using Expected Free Energy to select processing depth. Together they form a Loop that calibrates routing thresholds from experience.

## Temperament as a Score Cell

A **temperament** is a single configuration dial that controls multiple agent behaviors simultaneously. Rather than tuning 15 parameters (temperature, max_tokens, gate_threshold, review_passes, ...), the operator selects one temperament and all downstream behaviors adjust.

In unified terms, temperament is a **Score protocol Cell** that takes a task Signal and produces a behavioral rating Signal along multiple dimensions:

```rust
/// A Temperament Cell implements Score protocol.
/// Input: task Signal + config.
/// Output: behavioral dimension scores that downstream Cells consume.
pub struct TemperamentCell {
    level: TemperamentLevel,
}

pub enum TemperamentLevel {
    Conservative,  // Production, safety-critical
    Balanced,      // Default development
    Aggressive,    // Rapid prototyping
    Exploratory,   // Research, experimentation
}

/// The output of the Score protocol: 5 behavioral dimension scores.
pub struct TemperamentScores {
    pub model_precision: f64,       // temperature, top_p
    pub tool_openness: f64,         // tool count, dangerous tool access
    pub gate_strictness: f64,       // which gates are required vs. skipped
    pub review_depth: f64,          // review passes, reviewer tier
    pub cost_tolerance: f64,        // budget multiplier, escalation threshold
}
```

### Behavioral Dimension Tables

Each dimension maps to concrete parameter values:

**Model Parameters (precision)**

| Parameter | Conservative | Balanced | Aggressive | Exploratory |
|---|---|---|---|---|
| temperature | 0.1 | 0.3 | 0.7 | 1.0 |
| top_p | 0.9 | 0.95 | 0.98 | 1.0 |
| max_tokens | default | default | 1.5x | 2.0x |

**Tool Selection (openness)**

| Behavior | Conservative | Balanced | Aggressive | Exploratory |
|---|---|---|---|---|
| Tool count | Minimal | Standard | Expanded | All available |
| Dangerous tools | Blocked | Blocked | Allowed + confirm | Allowed |
| Network access | Denied | Per-request | Allowed | Allowed |
| File writes | Confirmed | Allowed | Allowed | Allowed |

**Gate Strictness**

| Gate | Conservative | Balanced | Aggressive | Exploratory |
|---|---|---|---|---|
| Compile | Required | Required | Required | Warning |
| Test | Required | Required | Warning | Skipped |
| Clippy | Required | Warning | Skipped | Skipped |
| Diff size | < 500 lines | < 2000 | < 5000 | Disabled |
| Review | Required | Optional | Skipped | Skipped |

**Review Depth**

| Behavior | Conservative | Balanced | Aggressive | Exploratory |
|---|---|---|---|---|
| Review passes | 2 (double) | 1 | 0 (self-review) | 0 |
| Review model | Premium tier | Standard tier | Same as implementer | None |
| Feedback loop | Required | Optional | Disabled | Disabled |

**Model Routing (cost tolerance)**

| Behavior | Conservative | Balanced | Aggressive | Exploratory |
|---|---|---|---|---|
| Starting tier | Standard | Standard | Fast | Fast |
| Escalation threshold | 0.9 confidence | 0.7 | 0.5 | 0.3 |
| Budget multiplier | 0.8x | 1.0x | 1.5x | 2.0x |
| Fallback on error | Always | Usually | Sometimes | Rarely |

### Configuration

```toml
[agent]
temperament = "balanced"

# Per-role overrides:
[agent.roles.implementer]
temperament = "balanced"

[agent.roles.researcher]
temperament = "exploratory"

[agent.roles.auditor]
temperament = "conservative"
```

## Active Inference Connection

Temperament maps directly to the **precision parameter** in the Free Energy Principle (Friston, 2010):

- **Conservative** = high precision on expected outcomes. The agent strongly expects correct code and requires strong evidence (gate passes) before accepting. Low free-energy tolerance.
- **Exploratory** = low precision on expected outcomes. The agent accepts more variance, allowing exploration of the state space. High free-energy tolerance (more surprise is acceptable).

This precision parameter flows into the CascadeRouter's confidence threshold: higher precision demands more confidence before committing to a model tier. The mapping is deliberate, not coincidental.

## Dual-Process Routing: The CascadeRouter

The dual-process model (Kahneman, 2011) maps to three processing tiers. See [dual-process-and-efe-routing.md](dual-process-and-efe-routing.md) for the full EFE treatment. This document focuses on how temperament modulates the routing:

### The Confidence Cascade

```
Task arrives
    |
    v
Stage 1: Try Fast model (System 1)
    |
    +-- Confidence >= threshold --> Accept, done
    |
    v
Stage 2: Try Standard model
    |
    +-- Confidence >= threshold --> Accept, done
    |
    v
Stage 3: Try Premium model (System 2)
    |
    +-- Accept regardless
```

The confidence threshold is where temperament enters the routing:

| Temperament | Threshold | Effect on cascade |
|---|---|---|
| Conservative | 0.9 | Almost always escalates to Standard or Premium |
| Balanced | 0.7 | Fast model handles ~40% of tasks |
| Aggressive | 0.5 | Fast model handles ~60% of tasks |
| Exploratory | 0.3 | Fast model handles ~80% of tasks |

### Confidence Signal Composition

The confidence score that drives escalation combines multiple signals:

```
confidence = w1 * gate_pass_rate
           + w2 * (1 - uncertainty_markers)
           + w3 * historical_success_rate
           + w4 * task_complexity_estimate
```

Weights w1-w4 are learned via the LinUCB bandit (Li et al., 2010). The bandit updates after every task, closing the predict-publish-correct Loop.

### LinUCB as a Route Cell with Learning

The CascadeRouter uses **LinUCB contextual bandits** to select models within each tier:

```
For each task:
  1. Observe context x (task type, complexity, role, budget, temperament)
  2. Select model a = argmax(theta_a . x + alpha * sqrt(x' A_a^(-1) x))
  3. Observe reward r (gate pass/fail, tokens, latency, cost)
  4. Update: A_a += x . x', b_a += r . x
```

- **alpha** (exploration parameter): controlled by temperament. Exploratory = high alpha (try more models). Conservative = low alpha (exploit known-good).
- **Context vector x**: includes temperament as a dimension, so the bandit learns different routing policies per temperament.

### Pareto Frontier Pruning

Before the bandit selects, a Pareto frontier computation prunes dominated candidates:

| Dimension | What |
|---|---|
| Quality | Historical gate pass rate for this task type |
| Cost | Price per million tokens |

Models dominated on both dimensions are removed. This prevents the bandit from exploring obviously bad options, reducing sample complexity.

### Thompson Sampling for Escalation

For the escalate-or-accept decision, the CascadeRouter uses Thompson sampling:

```
For each tier t:
    sample theta_t ~ Beta(successes_t, failures_t)
    adjusted_confidence = theta_t * raw_confidence
```

This introduces beneficial randomness: even when the fast model's average confidence is below threshold, it occasionally gets a chance (when sampled theta is high), allowing the system to discover that the fast model has improved.

## Affect-Modulated Routing

The Daimon affect engine (PAD vector + behavioral state) modulates routing decisions. Per the affect-routing reality analysis (12-AFFECT-ROUTING.md), this is **designed but not yet wired**:

```rust
/// Affect-derived routing bias.
/// Computed from PAD vector and behavioral state.
pub struct AffectRoutingBias {
    pub tier_shift: i32,                // [-2, +2]: positive = prefer higher tier
    pub exploration_multiplier: f64,    // [0.5, 2.0]: arousal-driven
    pub cost_sensitivity: f64,          // [0.5, 2.0]: stress amplifier
    pub latency_sensitivity: f64,       // [0.5, 2.0]: arousal = "hurry up"
}

impl AffectRoutingBias {
    /// Mapping follows Gebhard's ALMA model:
    /// - Pleasure axis: positive -> allow expensive models
    /// - Arousal axis: high -> increase exploration + latency sensitivity
    /// - Dominance axis: high -> maintain current strategy
    pub fn from_pad(pad: PadVector, state: BehavioralState) -> Self {
        let tier_shift = match state {
            Stressed | Cautious => -1 + (pad.pleasure * 0.5).round() as i32,
            Confident | Engaged => 1 + (pad.pleasure * 0.5).round() as i32,
            _ => (pad.pleasure * 1.5).round() as i32,
        };
        // ...
    }
}
```

The interaction: temperament sets the baseline routing policy, affect modulates it in response to runtime experience. After 3 consecutive gate failures, stress rises, shifting routing toward cheaper models to conserve budget. After a streak of successes, confidence rises, allowing more expensive models.

## Dual-Process Theory 2.0

The classical System 1/System 2 dichotomy has been refined (De Neys & Pennycook, 2019):

### Competing Intuitions Model

Multiple types of intuitions (some logical, some heuristic) can have different **activation strengths**. When activation is similar, deliberation (System 2) intervenes. Mapping to Roko: the CascadeRouter's confidence signal is the activation strength. High confidence = strong intuition = accept at Fast tier. Uncertain confidence = competing intuitions = escalate.

### Hybrid Two-Stage Model

The best architectural mapping for the CascadeRouter:
1. A "shallow analytic monitoring process" (confidence estimation) is always active -- this is T0.
2. An "optional deeper processing stage" (model escalation) activates only when conflict is detected -- this is T1/T2 escalation.

### Triple-Process Theory: Type 3 Metacognition

Evans (2019) proposes a metacognitive "Type 3" process that sits above both System 1 and System 2. Mapping to Roko: **meta-routing** (routing the router). The meta-router decides whether to use the learned CascadeRouter or a simple heuristic:

```rust
/// The meta-router selects routing strategy based on task characteristics.
/// This is the Type 3 metacognitive process.
pub struct MetaRouter {
    heuristic: HeuristicRouter,   // Instant, no overhead
    learned: CascadeRouter,       // LinUCB with online adaptation
    knn: KnnRouter,               // Fast similarity-based
    policy: MetaRoutingPolicy,
}

pub struct MetaRoutingPolicy {
    pub heuristic_threshold: f64,     // Use heuristic if >0.9 confidence
    pub knn_observation_min: u64,     // Use kNN if >100 observations
    pub budget_fallback_usd: f64,     // Fall back to heuristic if budget critical
}
```

Key finding: a well-tuned k-Nearest Neighbors approach often matches or outperforms complex learned routers (arXiv:2505.12601, 2025). The locality properties of model performance in embedding space enable simple methods to achieve strong routing.

## Mixture of Experts Analogy

MoE routing within a single model is architecturally analogous to model-level routing:

| MoE concept | Model routing equivalent |
|---|---|
| Gating network | CascadeRouter |
| Expert | Individual model (Haiku, Sonnet, Opus) |
| Top-K selection | Cascade stages |
| Load balancing | Rate limit awareness + cost budget |
| Expert collapse | Model monoculture (always routing to one model) |
| Sparse activation | Only invoking the cheapest sufficient model |

### Anti-Collapse Mechanisms

```rust
pub struct CollapseAvoidance {
    pub min_exploration_rate: f64,           // 5% of tasks to non-default models
    pub geometric_forgetting: f64,           // 0.95 -- forget faster, adapt sooner
    pub max_consecutive_same_model: usize,   // 20 -- then forced exploration
    pub diversity_bonus_per_100: f64,        // 0.1 bonus per 100 tasks since last use
}
```

## The Calibration Loop

The full routing system forms a **Loop** (Graph with feedback edge):

```
Task Signal
    |
    v
+----------+     +------------+     +-----------+
| Temper-  |---->| Cascade    |---->| Model     |
| ament    |     | Router     |     | Dispatch  |
| Score    |     | (Route)    |     | (Connect) |
+----------+     +-----+------+     +-----+-----+
                       ^                    |
                       |                    v
                +------+------+     +------+------+
                | Calibration |<----| Gate        |
                | Policy      |     | Pipeline    |
                | (React)     |     | (Verify)    |
                +--------------+     +-------------+
```

The CalibrationPolicy (a React Cell) subscribes to routing predictions and gate outcomes, pairs them, and computes calibration reward. This reward feeds back to the CascadeRouter, updating LinUCB weights. The Loop improves routing decisions over time.

### Predict-Publish-Correct for Routing

Before dispatch, the router emits a `RoutingPrediction` Pulse (predicted model, confidence, estimated cost). After gate completion, a `RoutingOutcome` Pulse (actual model, gate pass/fail, actual cost). The CalibrationPolicy pairs them and computes:

| Signal | Weight | Meaning |
|---|---|---|
| Gate pass | +0.6 / -0.4 | Primary signal |
| Cost accuracy | +0.2 / -0.2 | Predicted vs. actual cost |
| Model match | +0.2 | Router selected correctly |

The EMA of calibration reward tracks whether routing is improving. Model accuracy should increase over time; if it plateaus, the exploration rate needs adjustment.

## Research Context

### Latest Routing Research (2025-2026)

| Paper | Key Contribution | Relevance |
|---|---|---|
| Router-R1 (Chen et al., 2025, NeurIPS) | RL-trained LLM router with multi-round deliberation | Multi-round routing (think + route interleaved) |
| xRouter (Qian et al., 2025, Salesforce) | Cost-aware RL orchestration across 20+ LLMs | 80-90% of GPT-5 accuracy at <1/5 cost |
| IRT-Router (Song et al., 2025, ACL) | Psychometric routing via Item Response Theory | Superior cold-start across 20 LLMs |
| BEST-Route (Ding et al., 2025, ICML) | Allocates both model and number-of-responses | 60% cost reduction with <1% quality loss |
| Prefill routing (Varshney & Surla, 2026) | Uses prefill activations as predictive signal | Near-zero overhead routing |
| RADAR (2025) | IRT + multi-objective Pareto optimization | 90% of o4-mini at 10% cost |
| PILOT (2025) | Offline preference priors + online LinUCB | Warm-start bandits with human preferences |
| Cascade unification (Dekoninck et al., 2025, ICLR) | Dynamic cascade (skip, reorder, early exit) | +14% on SWE-Bench |

### Key Finding: kNN Competitive

arXiv:2505.12601 (2025) shows a well-tuned kNN approach matches or outperforms complex learned routers. This validates Roko's MetaRouter design: start with simple methods, escalate to bandits only when needed.

---

## What This Enables

1. **Single-dial operator control** -- Temperament replaces 15+ individual parameters with one choice, making agent behavior predictable without deep system knowledge.
2. **Self-improving routing** -- The predict-publish-correct Loop means routing gets better with every task dispatch, without manual tuning.
3. **Affect-responsive routing** -- When wired, the system naturally conserves budget under stress and explores under confidence, matching human decision-making patterns.

## Feedback Loops

1. **Temperament -> routing -> gate outcome -> threshold adjustment**: Conservative temperament sets high thresholds; if gate pass rate is already high at lower thresholds, the adaptive threshold Loop relaxes them, effectively making the system learn that it can be less conservative.
2. **Affect -> tier shift -> cost -> budget pressure -> affect**: Stressed routing prefers cheap models, reducing cost, which reduces budget pressure, which reduces stress. This is a stabilizing negative feedback loop.
3. **Exploration -> model discovery -> Pareto frontier update -> exploitation**: High exploration (Exploratory temperament) discovers new model-task matches. These update the Pareto frontier, improving exploitation for all temperaments.

## Open Questions

1. **Temperament wiring priority**: The config field exists but is not propagated to gates, tools, routing, or review. Should this be wired incrementally (one dimension at a time) or all at once?
2. **Automatic temperament selection**: Could the system learn the optimal temperament for a task type, rather than requiring operator selection? This would be a Route Cell that selects temperament as a candidate.
3. **Temperament and vitality interaction**: Should declining vitality automatically shift temperament toward Conservative (protect remaining budget), or should this be an independent mechanism? See [cognitive-energy-and-vitality.md](cognitive-energy-and-vitality.md).
4. **Calibration cold start**: The CalibrationPolicy needs paired prediction+outcome data to be useful. How many task dispatches are needed before calibration statistics are reliable? (Estimate: ~20-30 pairs for stable EMA.)

---

## Citations

1. Kahneman, D. (2011). "Thinking, Fast and Slow." -- Dual-process theory.
2. De Neys, W. & Pennycook, G. (2019). "Logic, Fast and Slow." Current Directions in Psychological Science. -- Competing intuitions.
3. Evans, J. (2019). Type 3 metacognitive process. -- Triple-process theory.
4. Li, L. et al. (2010). "A contextual-bandit approach to personalized news article recommendation." WWW. -- LinUCB.
5. Friston, K. (2010). "The free-energy principle: a unified brain theory?" Nature Reviews Neuroscience.
6. Chen, Z. et al. (2025). "Router-R1." NeurIPS. arXiv:2506.09033.
7. Qian, C. et al. (2025). "xRouter." Salesforce. arXiv:2510.08439.
8. Song, J. et al. (2025). "IRT-Router." ACL. arXiv:2506.01048.
9. Ding, Y. et al. (2025). "BEST-Route." ICML. arXiv:2506.22716.
10. Dekoninck, J. et al. (2025). "A Unified Approach to Routing and Cascading." ICLR. arXiv:2410.10347.
11. arXiv:2505.12601 (2025). kNN vs. learned routers.
12. See [02-CELL.md](../../unified/02-CELL.md) for Score, Route, React protocol definitions.
13. See [05-AGENT.md](../../unified/05-AGENT.md) S5 for EFE routing and behavioral phases.
