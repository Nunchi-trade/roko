# Stability Mechanisms

> **Implementation plan:** `modelrouting/17-meta-learning-and-corrections.md` (tasks 2O.04–2O.06)
> **Theoretical basis:** Control theory (hysteresis, frequency separation), Ashby's Law
> **Cross-references:** [04-cascade-router](04-cascade-router.md), [13-8-missing-feedback-loops](13-8-missing-feedback-loops.md), [07-regression-detection](07-regression-detection.md)

---

## Purpose

A system with eight feedback loops operating simultaneously can oscillate: loop 1 routes away from a provider, loop 6 routes back because the alternative is more expensive, loop 7 routes away again because the alternative is slower, and the system thrashes between options without settling. Stability mechanisms prevent this oscillation by introducing damping, hysteresis, and frequency separation.

These mechanisms are not an optimization — they are a prerequisite. Without them, the compound improvement described in [17-adas-and-autocatalytic](17-adas-and-autocatalytic.md) cannot occur because the system spends its energy oscillating rather than converging.

---

## Hysteresis

### Definition

Hysteresis introduces a switching threshold: the system only changes its decision when the new option is sufficiently better than the current option. Small improvements are ignored, preventing rapid oscillation between near-equal alternatives.

### Roko Implementation

The cascade router uses a 10% score delta threshold for model switching:

```
Current model: claude-sonnet-4 (score: 0.82)
Challenger: claude-opus-4 (score: 0.85)

Delta: 0.85 − 0.82 = 0.03
Hysteresis threshold: 0.10

0.03 < 0.10 → Keep current model (no switch)
```

The system only switches to a new model when the challenger's score exceeds the current model's score by at least 10%. This prevents oscillation between models with similar performance, which is common when:
- Two models have similar pass rates with different cost structures.
- Statistical noise makes one model appear slightly better on some batches.
- A new model's small advantage doesn't justify the disruption of switching.

### Why 10%?

The 10% threshold balances responsiveness and stability:

| Threshold | Behavior |
|-----------|----------|
| 1% | Near-zero hysteresis — switches on noise |
| 5% | Low hysteresis — switches on moderate improvements |
| **10%** | Moderate hysteresis — switches on meaningful improvements |
| 20% | High hysteresis — misses genuine improvements |
| 50% | Extreme — never switches except for dramatic changes |

The 10% value was chosen because:
- Typical model performance differences are 5-15% in pass rate.
- Cost differences between tiers are 5-10× (much larger than 10%).
- A 10% improvement in the composite score represents a genuine, actionable improvement.

### Hysteresis in Other Subsystems

The hysteresis principle applies beyond model routing:

| Subsystem | Hysteresis Mechanism |
|-----------|---------------------|
| Playbook rules | Confidence must cross min_confidence to prune (not oscillate near threshold) |
| Circuit breaker | Half-open requires a successful probe before closing (not just cooldown expiry) |
| Adaptive thresholds | EMA smoothing prevents threshold oscillation from batch-to-batch noise |
| Pattern discovery | min_support threshold prevents low-confidence patterns from being promoted |

---

## Frequency Separation

### Definition

Frequency separation assigns different update rates to subsystems based on their characteristic timescales. Fast subsystems (model routing) update every episode. Slow subsystems (pattern discovery) update every 20 episodes. This prevents fast loops from reacting to signals that haven't been confirmed by slow loops.

### Roko Implementation

```
Update Frequency Hierarchy:

    ┌─── Every episode ───────────────────────────────────────┐
    │  Cascade router:     update bandit arms                  │
    │  Episode logger:     append episode                      │
    │  Cost log:           append cost record                  │
    │  Provider health:    update circuit breaker               │
    │  Anomaly detector:   check prompt loop, cost spike        │
    └─────────────────────────────────────────────────────────┘
                            │
    ┌─── Every 5 episodes ────────────────────────────────────┐
    │  Gate thresholds:    EMA update of adaptive thresholds    │
    │  Regression check:   compare current vs baseline         │
    │  Efficiency grading: update section effectiveness         │
    └─────────────────────────────────────────────────────────┘
                            │
    ┌─── Every 20 episodes ───────────────────────────────────┐
    │  Pattern discovery:  trigram mining, pattern extraction   │
    │  Skill extraction:   Voyager-style skill mining           │
    │  Cross-episode:      HDC clustering consolidation         │
    └─────────────────────────────────────────────────────────┘
                            │
    ┌─── Every 50 episodes ───────────────────────────────────┐
    │  Pareto frontier:    recompute Pareto-optimal models     │
    │  C-Factor:           recompute collective capability     │
    └─────────────────────────────────────────────────────────┘
```

### Why These Frequencies?

| Subsystem | Frequency | Rationale |
|-----------|-----------|-----------|
| Cascade router | Every 1 | Routing decisions benefit from immediate feedback |
| Gate thresholds | Every 5 | Thresholds need multiple data points to avoid noise |
| Pattern discovery | Every 20 | Patterns need a statistically meaningful sample |
| Pareto frontier | Every 50 | Model statistics need many observations for stable estimates |

The frequencies are chosen so that each subsystem has enough observations to make a reliable update at its cadence. A subsystem that updates too frequently relative to its required sample size produces noisy decisions; one that updates too infrequently misses genuine changes.

### Interaction Between Frequencies

The frequency hierarchy creates a natural information cascade:

1. **Per-episode** data flows into fast subsystems (routing, health).
2. **Aggregated** data (5-episode windows) flows into medium subsystems (thresholds, regression).
3. **Consolidated** data (20-episode batches) flows into slow subsystems (patterns, skills).
4. **Summary** data (50-episode summaries) flows into the slowest subsystems (Pareto, C-Factor).

Each level receives data that has already been filtered and stabilized by the level above it. Fast oscillations in routing decisions are invisible to pattern discovery, which only sees the 20-episode trend.

---

## Damping

### EMA Smoothing

Exponential Moving Average (EMA) smoothing damps oscillation in continuously-valued quantities:

```
ema_new = α × observation + (1 − α) × ema_old
```

where α ∈ (0, 1) controls the smoothing rate. Small α = heavy smoothing (slow response). Large α = light smoothing (fast response).

| Subsystem | α | Behavior |
|-----------|---|----------|
| Gate thresholds | 0.1 | Heavy smoothing — thresholds change slowly |
| Cost EWMA | 0.2 | Moderate smoothing — cost baseline adapts over ~5 observations |
| Latency EMA | 0.1 | Heavy smoothing — latency baseline is conservative |
| LinUCB alpha decay | exp(-obs/60) | Exponential decay — exploration decreases gradually |

### Why Not Moving Average?

Simple moving averages (mean of last N values) have a discontinuity problem: when an old value exits the window, the average can jump even without new data. EMA avoids this by weighting all past observations, with exponentially decaying weights. The result is a smooth, continuous signal that responds proportionally to the magnitude of new observations.

---

## Compound Stability

The interaction of hysteresis, frequency separation, and EMA smoothing creates compound stability:

1. **Hysteresis** prevents switching on noise.
2. **Frequency separation** prevents fast loops from disrupting slow loops.
3. **EMA smoothing** prevents continuous quantities from oscillating.

Together, these mechanisms ensure that the eight feedback loops converge to a stable operating point rather than oscillating. The system "locks in" to good configurations and only moves when there is strong evidence for improvement.

### Stability Budget

Each feedback loop has a "stability budget": the amount of perturbation it can absorb without oscillating. The hysteresis threshold, update frequency, and EMA α collectively determine this budget. Loops with large stability budgets (pattern discovery: 20-episode frequency, high min_support threshold) are very stable but slow to respond. Loops with small stability budgets (routing: per-episode frequency, 10% hysteresis) are responsive but more prone to oscillation.

The system design ensures that stability budgets increase with the severity of the action: routing decisions (low cost to change) have small stability budgets, while pattern promotion (high cost to change — wrong rules degrade all future agents) has large stability budgets.

---

## Anti-Pattern: Positive Feedback Loops

Stability mechanisms are designed to prevent positive feedback loops — self-reinforcing cycles that drive the system to extremes:

| Anti-pattern | What happens | Prevention |
|-------------|-------------|------------|
| Model lock-in | Bandit exploits one model so heavily that alternatives never get enough data to compete | UCB exploration term, α decay |
| Playbook explosion | Rules accumulate without pruning, consuming entire prompt budget | Confidence decay, min_confidence threshold |
| Cost death spiral | Budget pressure forces cheap models → failures → more iterations → higher cost | Per-task budget limit, hard stop |
| Threshold collapse | Adaptive thresholds relax so far that gates are meaningless | Floor on threshold values |

Each anti-pattern has a specific stability mechanism that prevents it. The compound effect is that the system remains in its "viable region" (Beer's Viable System Model) — operating within the bounds where all feedback loops function correctly.

---

## Theoretical Foundation

### Ashby's Law of Requisite Variety

A control system must have at least as much variety (number of distinct states) as the system it controls. Roko's stability mechanisms implement this by providing a different damping mechanism for each type of oscillation:

| Oscillation Type | Required Variety | Mechanism |
|-----------------|-----------------|-----------|
| Binary switching (model A vs B) | Two states + threshold | Hysteresis |
| Continuous drift (parameter values) | Continuous damping | EMA smoothing |
| Multi-rate interference (fast loop disturbs slow loop) | Frequency isolation | Frequency separation |
| Degenerate convergence (all traffic to one arm) | Forced exploration | UCB exploration term |

### Beer's Viable System Model

Beer's VSM defines five systems required for organizational viability. Roko's stability mechanisms map to:

| VSM System | Function | Roko Implementation |
|-----------|----------|-------------------|
| System 1 | Operations | Individual learning subsystems (bandits, episode logger, etc.) |
| System 2 | Coordination | Frequency separation, LearningRuntime ordering |
| System 3 | Control | Regression detection, C-Factor monitoring |
| System 4 | Intelligence | Pattern discovery, predictive foraging |
| System 5 | Policy | Hysteresis thresholds, EMA parameters |

The stability mechanisms primarily implement Systems 2 and 3: coordination between subsystems and control over aggregate behavior.

### Good Regulator Theorem

A system that is a good regulator of another system must be a model of that system. Roko's C-Factor is a model of the system's overall health — it captures the key performance indicators in a single composite score. The regression detector uses this model to identify when the system deviates from expected behavior, triggering corrective actions.

---

## Relationship to Other Documents

- **[04-cascade-router](04-cascade-router.md)** — The primary subsystem where hysteresis is applied.
- **[13-8-missing-feedback-loops](13-8-missing-feedback-loops.md)** — The eight loops that stability mechanisms regulate.
- **[07-regression-detection](07-regression-detection.md)** — Regression detection is itself a stability mechanism (alerts on degradation).
- **[09-provider-health-circuit-breaker](09-provider-health-circuit-breaker.md)** — Circuit breaker is a stability mechanism for provider health.
- **[17-adas-and-autocatalytic](17-adas-and-autocatalytic.md)** — Stability is a prerequisite for the compound improvement described there.
