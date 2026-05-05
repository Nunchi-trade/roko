# Drift and Stability

> Depth for [07-LEARNING.md](../../unified/07-LEARNING.md). Thompson Sampling drift detection as a Loop, stability mechanisms (hysteresis, frequency separation, EMA damping) that prevent eight concurrent feedback Loops from oscillating, and the compound stability condition that makes autocatalytic compounding possible.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse), [02-CELL](../../unified/02-CELL.md) (Cell, React, Observe, Verify protocols, predict-publish-correct), [04-EXECUTION](../../unified/04-EXECUTION.md) (Loop specialization, convergence), [07-LEARNING](../../unified/07-LEARNING.md) (L1 parameter tuning, four Loop taxonomy), [autocatalytic-compounding.md](autocatalytic-compounding.md) (seven compounding Loops, Kauffman condition)

**Source docs**: [11-thompson-sampling-drift.md](../../docs/05-learning/11-thompson-sampling-drift.md), [14-stability-mechanisms.md](../../docs/05-learning/14-stability-mechanisms.md)

---

## 1. The Problem: Non-Stationarity

The environment Roko learns in is not fixed. Provider model updates change quality overnight. The codebase evolves with each merged PR. Task distributions shift between development phases. Cache dynamics alter effective cost as patterns repeat. A learning system that treats all historical observations equally will be misled by stale data -- a model that was excellent three months ago may be mediocre today, but its strong historical record keeps the bandit selecting it.

This is the drift problem. It affects every Loop in the system, not just model routing. Gate thresholds tuned to last week's pass rates may be wrong for this week's task mix. Prompt section weights that worked for scaffolding tasks may waste tokens on refactoring tasks. The cascade router's confidence intervals, the playbook rule library, the skill effectiveness scores -- all of these can go stale.

The source docs identify two categories of drift:

| Drift type | Cause | Timescale | Example |
|---|---|---|---|
| **Gradual** | Codebase evolution, task mix shift, cache warming | Weeks to months | Crate familiarity increases, making sonnet sufficient where opus was needed |
| **Abrupt** | Provider model update, config change, dependency upgrade | Instantaneous | Claude 4.5 deployed with different tool-calling behavior |

---

## 2. Thompson Sampling with Discount as a Loop

Standard Thompson Sampling (Thompson 1933) maintains a Beta distribution per arm: `(alpha, beta)` where alpha counts successes and beta counts failures. Selection samples from each arm's distribution and picks the highest sample. The stochastic selection naturally balances exploration (wide distributions for uncertain arms) and exploitation (narrow distributions for well-known arms).

Discounted Thompson Sampling (Garivier & Moulines 2011) adds a forgetting mechanism. Before each update, existing evidence is decayed by a discount factor gamma:

```
On update for arm a:
    alpha_a <- gamma * alpha_a + reward
    beta_a  <- gamma * beta_a  + (1 - reward)
```

This creates an effective observation window of approximately `1 / (1 - gamma)`. Old observations fade exponentially, making the algorithm responsive to recent performance shifts.

### As a Loop Graph

In unified vocabulary, Thompson Sampling with drift is a **Loop** -- a Graph with a feedback edge from output back to input. The Loop has four Cells:

```toml
[graph]
id = "thompson-drift-loop"
kind = "Loop"

[[nodes]]
id = "observe"
cell = "roko:drift-observer"
protocol = "Observe"
# Reads posterior state, computes arm switching rate and prediction error trend

[[nodes]]
id = "sample"
cell = "roko:thompson-sampler"
protocol = "Route"
# Samples from discounted Beta posteriors, selects arm

[[nodes]]
id = "outcome"
cell = "roko:outcome-collector"
protocol = "Score"
# Records pass/fail outcome for the selected arm

[[nodes]]
id = "update"
cell = "roko:discount-updater"
protocol = "React"
# Applies gamma discount to all arms, incorporates new observation

[[edges]]
from = "observe"
to = "sample"

[[edges]]
from = "sample"
to = "outcome"

[[edges]]
from = "outcome"
to = "update"

[[edges]]
from = "update"
to = "observe"
condition = "always"
# Feedback edge: updated posteriors feed the next observation
```

### Per-Arm State

```rust
/// One arm in the Thompson Sampling bandit.
/// Each arm corresponds to a model slug in the CascadeRouter.
pub struct ThompsonArm {
    /// Model identifier.
    pub model: String,
    /// Discounted success count (Beta alpha parameter).
    pub alpha: f64,
    /// Discounted failure count (Beta beta parameter).
    pub beta: f64,
    /// Total undiscounted observations (for diagnostics only).
    pub total_observations: u64,
}

impl ThompsonArm {
    /// Sample from the arm's Beta posterior.
    /// Floor of 0.01 prevents degenerate distributions after heavy discounting.
    pub fn sample(&self, rng: &mut impl Rng) -> f64 {
        Beta::new(self.alpha.max(0.01), self.beta.max(0.01))
            .unwrap()
            .sample(rng)
    }

    /// Update with discounted evidence.
    pub fn update(&mut self, reward: f64, gamma: f64) {
        self.alpha = gamma * self.alpha + reward;
        self.beta = gamma * self.beta + (1.0 - reward);
        self.total_observations += 1;
    }
}
```

### Discount Factor Selection

The recommended default is gamma = 0.995 (effective window of ~200 observations). This balances responsiveness and stability:

| gamma | Effective window | Detects change within | Use when |
|---|---|---|---|
| 0.999 | ~1000 obs | ~250 obs of shift | Environment is nearly stationary |
| **0.995** | ~200 obs | ~50 obs of shift | **Default for model routing** |
| 0.99 | ~100 obs | ~25 obs of shift | Frequent provider updates |
| 0.95 | ~20 obs | ~5 obs of shift | Rapid prototyping, volatile env |

### Adaptive Discount

Rather than fixing gamma, the system can adjust it based on observed non-stationarity:

```rust
/// Adapt the discount factor based on arm switching rate.
/// High switching = environment is changing = increase forgetting.
/// Low switching = environment is stable = decrease forgetting.
pub fn adapt_gamma(
    current_gamma: f64,
    arm_switching_rate: f64,
) -> f64 {
    if arm_switching_rate > 0.20 {
        // Too much switching: increase forgetting to track change
        (current_gamma - 0.01).max(0.90)
    } else if arm_switching_rate < 0.05 {
        // Very stable: decrease forgetting to accumulate evidence
        (current_gamma + 0.01).min(0.999)
    } else {
        current_gamma
    }
}
```

---

## 3. Drift Detection for Abrupt Changes

The discount factor handles gradual drift automatically. For abrupt changes (e.g., a provider deploys a breaking update), a separate detection mechanism triggers a posterior reset:

```rust
/// Detect abrupt performance shifts by comparing recent and historical pass rates.
/// When the gap exceeds threshold, reset the arm to an uninformative prior.
pub fn detect_abrupt_drift(
    arm: &mut ThompsonArm,
    recent_window: &[bool],  // last 10 outcomes
    historical_window: &[bool],  // last 100 outcomes
    reset_threshold: f64,  // default: 0.25
) -> bool {
    let recent_rate = recent_window.iter().filter(|&&x| x).count() as f64
        / recent_window.len().max(1) as f64;
    let historical_rate = historical_window.iter().filter(|&&x| x).count() as f64
        / historical_window.len().max(1) as f64;

    if (historical_rate - recent_rate).abs() > reset_threshold {
        // Abrupt shift detected: reset to uninformative prior
        arm.alpha = 1.0;
        arm.beta = 1.0;
        true
    } else {
        false
    }
}
```

This connects to the provider health circuit breaker: when the circuit breaker detects degradation (Loop 1 from [missing-loops-and-calibration.md](missing-loops-and-calibration.md)), it can also trigger the Thompson arm reset for that provider's models.

---

## 4. Stability Mechanisms

A system with eight feedback Loops running simultaneously can oscillate. Loop 1 (Health->Routing) routes away from a provider; Loop 6 (Cost->Routing) routes back because the fallback is expensive; Loop 7 (Latency->Reward) routes away again because the fallback is slow. The system thrashes without settling.

Stability mechanisms are not an optimization -- they are a prerequisite for the autocatalytic compounding described in [autocatalytic-compounding.md](autocatalytic-compounding.md). Without stability, the system spends its energy oscillating rather than converging.

### 4.1 Hysteresis

Hysteresis introduces a switching threshold: the system only changes its decision when the new option is sufficiently better than the current one. Small improvements are ignored.

```rust
/// Apply hysteresis to a routing decision.
/// Only switch to challenger if its score exceeds current by at least the threshold.
pub fn apply_hysteresis(
    current_model: &str,
    current_score: f64,
    challenger_model: &str,
    challenger_score: f64,
    threshold: f64,  // default: 0.10
) -> &str {
    if challenger_score - current_score > threshold {
        challenger_model  // genuine improvement, switch
    } else {
        current_model  // within hysteresis band, keep current
    }
}
```

The 10% threshold was chosen because typical model performance differences are 5-15% in pass rate, and cost differences between tiers are 5-10x (much larger). A 10% improvement in composite score represents a genuine, actionable change.

Hysteresis applies beyond routing:

| Subsystem | Hysteresis mechanism |
|---|---|
| Playbook rules | Confidence must cross min_confidence to prune |
| Circuit breaker | Half-open requires a successful probe, not just cooldown expiry |
| Adaptive thresholds | EMA smoothing prevents batch-to-batch oscillation |
| Pattern discovery | min_support threshold prevents low-confidence patterns from promoting |

### 4.2 Frequency Separation

Different subsystems operate at different timescales. Assigning distinct update frequencies prevents fast Loops from reacting to signals that slow Loops have not yet confirmed.

```
Update Frequency Hierarchy
==========================

Every episode (immediate):
    Cascade router:      update bandit arms
    Episode logger:      append episode
    Cost log:            append cost record
    Provider health:     update circuit breaker

Every 5 episodes:
    Gate thresholds:     EMA update
    Regression check:    compare current vs baseline
    Section effectiveness: update pass-rate-when-included

Every 20 episodes:
    Pattern discovery:   trigram mining, pattern extraction
    Skill extraction:    Voyager-style skill accumulation
    HDC clustering:      codebook consolidation

Every 50 episodes:
    Pareto frontier:     recompute Pareto-optimal models
    C-factor:            recompute collective capability
```

This hierarchy creates a natural information cascade. Fast oscillations in per-episode routing are invisible to pattern discovery, which only sees the 20-episode trend. Each level receives data already filtered and stabilized by the level above.

### 4.3 EMA Damping

Exponential Moving Average smoothing damps oscillation in continuously-valued quantities:

```
ema_new = alpha * observation + (1 - alpha) * ema_old
```

| Subsystem | alpha | Effect |
|---|---|---|
| Gate thresholds | 0.1 | Heavy smoothing -- thresholds change slowly |
| Cost baseline | 0.2 | Moderate -- cost baseline adapts over ~5 observations |
| Latency baseline | 0.1 | Heavy -- latency baseline is conservative |
| LinUCB alpha decay | exp(-obs/60) | Exponential -- exploration decreases gradually |

EMA is preferred over simple moving average because SMA has a discontinuity problem: when an old value exits the window, the average jumps even without new data. EMA avoids this by weighting all past observations with exponentially decaying weights.

---

## 5. Compound Stability

The three mechanisms interact to create compound stability:

1. **Hysteresis** prevents switching on noise (discrete decisions).
2. **Frequency separation** prevents fast Loops from disrupting slow Loops (temporal isolation).
3. **EMA damping** prevents continuous quantities from oscillating (smoothing).

Together, they ensure the eight feedback Loops converge to a stable operating point rather than oscillating. The system "locks in" to good configurations and only moves when evidence is strong.

### Stability Budget

Each Loop has a "stability budget" -- the amount of perturbation it can absorb without oscillating. The hysteresis threshold, update frequency, and EMA alpha collectively determine this budget:

| Loop type | Stability budget | Rationale |
|---|---|---|
| Routing (per-episode) | Small | Low cost to change, fast feedback needed |
| Gate thresholds (per-5) | Medium | Moderate cost to change, EMA-smoothed |
| Pattern promotion (per-20) | Large | High cost to change (wrong rules degrade all future agents) |
| C-factor evolution (per-50) | Very large | Structural changes need high confidence |

The design principle: stability budgets increase with the severity of the action.

---

## 6. Anti-Patterns: Positive Feedback Traps

Without stability mechanisms, four positive feedback traps can drive the system to extremes:

| Anti-pattern | Mechanism | Prevention |
|---|---|---|
| **Model lock-in** | Bandit exploits one model so heavily that alternatives never get data | UCB exploration term, alpha decay |
| **Playbook explosion** | Rules accumulate without pruning, consuming prompt budget | Confidence decay, min_confidence threshold |
| **Cost death spiral** | Budget pressure forces cheap models -> failures -> more iterations -> higher cost | Per-task budget limit, hard stop |
| **Threshold collapse** | Adaptive thresholds relax until gates are meaningless | Absolute floor on threshold values (0.30) |

Each trap has a specific stability mechanism. The compound effect keeps the system within its "viable region" (Beer's Viable System Model).

---

## 7. Theoretical Foundations

| Principle | Application |
|---|---|
| **Ashby's Law of Requisite Variety** | A different damping mechanism for each oscillation type: hysteresis for binary switching, EMA for continuous drift, frequency separation for multi-rate interference |
| **Beer's Viable System Model** | System 2 (coordination) = frequency separation; System 3 (control) = regression detection, C-factor monitoring; System 5 (policy) = hysteresis thresholds, EMA parameters |
| **Good Regulator Theorem** | C-factor is the system's model of its own health. Regression detection uses this model to identify deviations. Self-regulation requires self-observation |
| **Garivier & Moulines 2011** | Discounted Thompson Sampling for non-stationary bandits. Regret bound: O(sqrt(T / (1-gamma))) |

---

## 8. Mori-Diffs Reality

Per `tmp/mori-diffs/04-LEARNING.md`, the current codebase has the following state:

- **CascadeRouter** is wired and records observations, but uses LinUCB in stage 3 rather than Thompson Sampling with drift. Thompson is a proposed alternative for non-stationary environments.
- **Stability mechanisms** exist partially: EMA on gate thresholds is wired, hysteresis on routing is wired, but frequency separation is implicit (different subsystems happen to run at different cadences) rather than explicitly enforced by a scheduler.
- **Drift detection** for abrupt changes is not yet wired into the circuit breaker -> Thompson arm reset pathway. The circuit breaker detects degradation independently; the arm reset is a separate manual step.
- **Adaptive discount** (adjusting gamma based on arm switching rate) is designed but not implemented.

---

## What This Enables

1. **Responsive routing in non-stationary environments**: Thompson Sampling with discount detects model quality shifts within ~50 observations, while LinUCB may take hundreds.
2. **Oscillation-free multi-loop learning**: the compound stability condition (hysteresis + frequency separation + EMA) ensures eight concurrent feedback Loops converge rather than thrash.
3. **Abrupt drift recovery**: posterior resets triggered by circuit breaker events allow the system to re-explore after a provider breaking change.
4. **Self-tuning forgetting**: adaptive discount tracks the actual rate of environmental change rather than relying on a fixed assumption.

## Feedback Loops

- **L1 (Parameter Tuning)**: EMA alpha on gate thresholds is itself an L1 parameter that can be tuned by observing threshold stability.
- **Drift -> Routing**: non-stationarity detection feeds the cascade router's choice of algorithm (LinUCB for stable regimes, Thompson+drift for volatile regimes).
- **Stability -> Compounding**: compound stability is a prerequisite for the autocatalytic cycle described in [autocatalytic-compounding.md](autocatalytic-compounding.md). Without it, the Loops oscillate instead of compounding.

## Open Questions

1. **Per-Loop discount factors**: Should each Loop have its own gamma, tuned to its characteristic timescale? The source docs use a single gamma for routing, but gate thresholds, section weights, and skill confidence each have their own drift rate.
2. **Multi-arm drift correlation**: When a provider update affects multiple models simultaneously, should the reset propagate to all arms of that provider, or should each arm be evaluated independently?
3. **Stability metric**: Is there a single scalar that captures "how stable is the system right now?" across all eight Loops, analogous to how C-factor captures collective quality? The source docs describe stability budgets per Loop but no aggregate measure.
4. **Frequency separation enforcement**: Should update frequencies be enforced by a scheduler (hard separation) or advisory (soft, with override for urgency)? Hard separation is more stable but slower to respond to genuine crises.
