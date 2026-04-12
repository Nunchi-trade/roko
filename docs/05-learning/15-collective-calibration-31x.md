# Collective Calibration (31.6× Heuristic)

> **PRD source:** `refactoring-prd/09-innovations.md` §VI
> **Module:** `roko-learn/src/cfactor.rs`
> **Cross-references:** [04-cascade-router](04-cascade-router.md), [07-regression-detection](07-regression-detection.md), [13-8-missing-feedback-loops](13-8-missing-feedback-loops.md)

---

## Purpose

Collective Calibration is a heuristic framework for quantifying the aggregate performance improvement that emerges when multiple agents, feedback loops, and learning subsystems operate in concert. The core claim is that well-calibrated agent collectives can achieve up to 31.6× the throughput of individual agents — but this is a **heuristic upper bound with explicit caveats**, not a proven theorem.

The 31.6× figure comes from a simplified model inspired by the Central Limit Theorem. It provides a target and a measurement framework, not a guarantee.

---

## The 31.6× Heuristic

### Derivation

The heuristic models accuracy as:

```
accuracy(t) = 1 − 1/√(N × t)
```

where:
- N = number of agents in the collective
- t = number of calibration rounds (episodes)

For N = 10 agents and t = 100 rounds:

```
accuracy = 1 − 1/√(10 × 100) = 1 − 1/√1000 ≈ 1 − 0.0316 ≈ 0.968
```

The "31.6×" refers to the √1000 ≈ 31.6 factor in the denominator, which represents the effective sample size advantage of a calibrated collective over a single agent.

### CLT Inspiration

The formula is inspired by the Central Limit Theorem: the standard error of a sample mean decreases as 1/√n. If each agent provides an independent observation, and the collective aggregates these observations, the collective's error decreases as 1/√(N×t).

### Explicit Caveats

**This is NOT a theorem.** The following assumptions are required and frequently violated:

1. **Independence**: Agents' errors must be independent. In practice, agents using the same model and similar prompts make correlated errors. Correlation reduces the effective N.

2. **Stationarity**: The target distribution must not change during calibration. In practice, the codebase evolves, model providers update, and task distributions shift. Non-stationarity reduces the effective t.

3. **Aggregation mechanism**: The formula assumes optimal aggregation (e.g., majority voting or Bayesian averaging). In practice, Roko uses sequential execution with feedback, not parallel voting. The aggregation mechanism affects the constant factor.

4. **Finite-sample effects**: For small N and t, the 1/√(N×t) approximation is loose. The CLT is an asymptotic result; finite samples may be far from the limit.

5. **Heterogeneous quality**: The formula assumes equal-quality agents. If some agents are much worse than others, they add noise rather than signal, potentially reducing collective performance below individual performance.

**In practice, expect 3-10× improvement from collective calibration, not 31.6×.** The 31.6× is the idealized upper bound under perfect conditions.

---

## C-Factor: Composite Capability Metric

The C-Factor (Collective Capability Factor) is the practical implementation of collective calibration measurement. It combines multiple performance indicators into a single scalar:

```rust
pub struct CFactor {
    /// 0.0-1.0 composite score.
    pub overall: f64,
    /// Component breakdown.
    pub components: CFactorComponents,
    /// Per-agent leave-one-out contributions.
    pub agent_contributions: Vec<AgentCFactorContribution>,
    /// When the score was computed.
    pub computed_at: DateTime<Utc>,
    /// Number of episodes in the calculation.
    pub episode_count: usize,
}
```

### Components

```rust
pub struct CFactorComponents {
    /// % of tasks passing gates on first attempt.
    pub gate_pass_rate: f64,
    /// Inverse of cost per successful task, normalized.
    pub cost_efficiency: f64,
    /// Inverse of time per successful task, normalized.
    pub speed: f64,
    /// Normalized signal throughput.
    pub information_flow_rate: f64,
    /// % of tasks succeeding without re-plan.
    pub first_try_rate: f64,
    /// Rate of new knowledge entries per episode.
    pub knowledge_growth: f64,
    /// Speed of shared insight accumulation.
    pub knowledge_integration_rate: f64,
    /// How strongly templates specialize by category.
    pub task_diversity_coverage: f64,
    /// Speed of convergent conclusions.
    pub convergence_velocity: f64,
    /// Evenness of agent participation.
    pub turn_taking_equality: f64,
    /// Normalized dependency output rate.
    pub social_sensitivity: f64,
}
```

### Component Weights

The composite score is a weighted average of components. Default weights emphasize outcome metrics over process metrics:

| Component | Weight | Rationale |
|-----------|--------|-----------|
| gate_pass_rate | 0.20 | Primary success metric |
| cost_efficiency | 0.15 | Budget sustainability |
| speed | 0.10 | Throughput |
| first_try_rate | 0.15 | Efficiency of approach |
| knowledge_growth | 0.10 | Learning velocity |
| turn_taking_equality | 0.05 | Collaboration quality |
| Others | 0.25 (distributed) | Secondary indicators |

---

## Leave-One-Out Contributions

The C-Factor includes per-agent contribution scores computed via leave-one-out analysis:

```rust
pub struct AgentCFactorContribution {
    /// Agent identifier.
    pub agent_id: String,
    /// Episodes attributed to this agent.
    pub episode_count: usize,
    /// C-Factor without this agent's episodes.
    pub without_agent_overall: f64,
    /// Full score minus leave-one-out score.
    pub contribution_score: f64,
}
```

If `contribution_score > 0`, the agent raises the collective C-Factor (positive contributor). If `contribution_score < 0`, the agent drags it down (negative contributor).

### Dispatch Bias

Leave-one-out contributions inform routing decisions:

```rust
pub enum AgentDispatchBias {
    /// Agent has negative contribution → prefer stronger model.
    PreferStronger,
    /// Agent has strong positive contribution → prefer cheaper model.
    PreferCheaper,
    /// Neutral contribution → no bias.
    Neutral,
}
```

The cascade router uses this bias during the confidence stage: agents with consistently negative contributions are routed to stronger (more expensive) models, while agents with strong positive contributions can be routed to cheaper models without sacrificing quality.

---

## C-Factor Regression

The C-Factor tracks its own history for regression detection:

```rust
pub struct CFactorRegression {
    pub current_snapshot_at: DateTime<Utc>,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub sample_count: usize,
    // ... delta analysis
}
```

A C-Factor regression is triggered when the current C-Factor drops significantly below the trailing average. This catches systemic degradation that individual metrics might miss — a small drop in pass rate combined with a small increase in cost and a small decrease in speed may not trigger any individual threshold, but the C-Factor composite detects the overall decline.

---

## Computing the C-Factor

The C-Factor is computed every 50 episodes (the slowest learning frequency):

```
Every 50 episodes:
    │
    ├── 1. Load recent episodes (sliding window of last 200)
    │
    ├── 2. Compute component metrics:
    │       gate_pass_rate: successful episodes / total episodes
    │       cost_efficiency: 1 / (avg cost per success), normalized
    │       speed: 1 / (avg duration per success), normalized
    │       first_try_rate: iteration-0 successes / total tasks
    │       knowledge_growth: new skills + patterns per episode
    │       ...
    │
    ├── 3. Compute leave-one-out contributions per agent
    │
    ├── 4. Combine components with weights → overall score
    │
    └── 5. Persist to .roko/learn/c-factor.jsonl
```

### Normalization

Each component is normalized to [0.0, 1.0] before weighting. Normalization uses a baseline window: the component value from the first 10 plans serves as the reference point. Values below baseline map to [0.0, 0.5], values at baseline map to 0.5, and values above baseline map to [0.5, 1.0].

This relative normalization means the C-Factor measures improvement over the system's own baseline, not against an absolute standard. A C-Factor of 0.8 means the system is performing significantly better than its initial configuration, regardless of what that initial configuration was.

---

## Practical Interpretation

| C-Factor | Interpretation | Action |
|----------|---------------|--------|
| < 0.3 | System is performing poorly | Investigate regressions, consider manual intervention |
| 0.3 – 0.5 | Below baseline | Check feedback loops, review recent changes |
| 0.5 | At baseline | Normal operation |
| 0.5 – 0.7 | Above baseline, improving | Learning loops are working |
| 0.7 – 0.9 | Well above baseline | System has significantly improved through self-optimization |
| > 0.9 | Near-optimal | Consider lowering cost (cheaper models) while maintaining quality |

---

## Relationship to Other Documents

- **[04-cascade-router](04-cascade-router.md)** — C-Factor provides routing bias (PreferStronger/PreferCheaper/Neutral).
- **[07-regression-detection](07-regression-detection.md)** — C-Factor regression complements per-metric regression detection.
- **[13-8-missing-feedback-loops](13-8-missing-feedback-loops.md)** — The C-Factor measures the aggregate effect of all eight loops.
- **[14-stability-mechanisms](14-stability-mechanisms.md)** — C-Factor is computed at the slowest frequency (every 50 episodes), making it a stability anchor.
- **[17-adas-and-autocatalytic](17-adas-and-autocatalytic.md)** — The autocatalytic thesis predicts that C-Factor should increase over time as learning compounds.
- **[06-task-metrics-and-baselines](06-task-metrics-and-baselines.md)** — Individual metrics feed into C-Factor components.
