# Thompson Sampling with Drift

> **Implementation plan:** `modelrouting/12-advanced-patterns.md` (tasks 2J.01–2J.03)
> **Academic basis:** Thompson 1933; Garivier & Moulines 2011 (discounted Thompson Sampling)
> **Cross-references:** [03-bandits-ucb-thompson-linucb](03-bandits-ucb-thompson-linucb.md), [04-cascade-router](04-cascade-router.md), [14-stability-mechanisms](14-stability-mechanisms.md)

---

## Purpose

Thompson Sampling with drift is a Bayesian bandit algorithm designed for non-stationary environments where the reward distribution of each arm changes over time. In the model routing context, non-stationarity arises from:

- **Provider updates**: A model's quality changes when the provider deploys a new version.
- **Codebase evolution**: As the codebase grows or is refactored, the relative performance of models shifts.
- **Task mix changes**: The distribution of task categories and complexities varies across development phases.
- **Cache dynamics**: Repeated access patterns improve cache hit rates, changing the effective cost of models.

Standard UCB1 and LinUCB are designed for stationary environments: they accumulate all historical observations equally. In a non-stationary world, old observations can mislead the algorithm — a model that was excellent three months ago may be mediocre today after a provider update, but its strong historical record keeps UCB1 selecting it.

Thompson Sampling with a discount factor addresses this by down-weighting old observations, making the algorithm responsive to recent performance changes.

---

## Algorithm

### Standard Thompson Sampling

For each arm `a` with binary reward (pass/fail):

1. Maintain Beta distribution parameters `(α_a, β_a)` where α = successes, β = failures.
2. To select: sample `θ_a ~ Beta(α_a, β_a)` for each arm. Select arm with highest sample.
3. To update: if reward = 1, α_a += 1. If reward = 0, β_a += 1.

The Beta distribution naturally encodes uncertainty: arms with few observations have wide distributions (high exploration), while well-observed arms have narrow distributions (high exploitation).

### Adding Drift (Discount Factor)

To handle non-stationarity, apply a discount factor γ ∈ (0, 1) to existing observations before updating:

```
On update for arm a:
    α_a ← γ · α_a + reward
    β_a ← γ · β_a + (1 − reward)
```

The discount factor γ controls the "effective window" of observations:

| γ | Effective window | Behavior |
|---|-----------------|----------|
| 0.999 | ~1000 observations | Very slow forgetting, near-stationary |
| 0.99 | ~100 observations | Moderate forgetting |
| 0.95 | ~20 observations | Fast forgetting, very responsive |
| 0.90 | ~10 observations | Aggressive forgetting |

### Effective Window Calculation

The effective window is approximately `1 / (1 − γ)`. After `n` observations, the weight of the oldest observation is `γ^n`. When `γ^n < 0.01` (i.e., the oldest observation contributes less than 1%), we consider it effectively forgotten:

```
n_effective = ln(0.01) / ln(γ) = −4.605 / ln(γ)
```

| γ | n_effective |
|---|-------------|
| 0.999 | 4603 |
| 0.99 | 460 |
| 0.95 | 90 |
| 0.90 | 44 |

---

## Design Considerations for Roko

### Recommended Discount Factor

For model routing in Roko, the recommended discount factor is **γ = 0.995** (effective window ~200 observations). This balances:

- **Responsiveness**: Detects model quality changes within ~50 observations of the change.
- **Stability**: Doesn't overreact to short-term noise from individual task outcomes.
- **Cold start**: After 200 observations, the system has effectively "forgotten" its cold-start period and responds only to recent performance.

### Comparison with UCB1 and LinUCB

| Property | UCB1 | LinUCB | Thompson + Drift |
|----------|------|--------|-----------------|
| Context-dependent | No | Yes (18-dim) | No (per-arm) |
| Non-stationary | No | No | Yes (γ discount) |
| Exploration | Deterministic (upper bound) | Deterministic (upper bound) | Stochastic (sampling) |
| Convergence | O(√(T ln T)) regret | O(d√(T ln T)) regret | O(√(T / (1−γ))) regret |
| Cold start | Infinite UCB for unpulled arms | Static fallback | Wide Beta prior |

### When to Use Thompson Sampling vs UCB1

- **Use UCB1** for stationary decisions (tool format selection, retry strategy) where the optimal choice doesn't change over time.
- **Use Thompson Sampling with drift** for non-stationary decisions (model routing, provider selection) where the optimal choice shifts with provider updates and codebase evolution.
- **Use LinUCB** when context features (task category, complexity, role) strongly influence the optimal choice, even in a stationary environment.

The cascade router currently uses LinUCB in stage 3. Thompson Sampling with drift is proposed as an alternative stage-3 algorithm for environments with frequent model updates, as specified in implementation plan 2J.01–2J.03.

---

## Implementation Design

### Per-Arm State

```rust
struct ThompsonArm {
    /// Model slug.
    model: String,
    /// Beta distribution α parameter (discounted successes).
    alpha: f64,
    /// Beta distribution β parameter (discounted failures).
    beta: f64,
    /// Total observations (not discounted, for diagnostics).
    total_observations: u64,
}
```

### Selection

```rust
fn select(arms: &[ThompsonArm], rng: &mut impl Rng) -> usize {
    arms.iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| {
            let sample_a = Beta::new(a.alpha.max(0.01), a.beta.max(0.01)).sample(rng);
            let sample_b = Beta::new(b.alpha.max(0.01), b.beta.max(0.01)).sample(rng);
            sample_a.partial_cmp(&sample_b).unwrap()
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}
```

The `max(0.01)` floor prevents degenerate Beta distributions when both parameters approach zero after heavy discounting.

### Update with Discount

```rust
fn update(arm: &mut ThompsonArm, reward: f64, gamma: f64) {
    arm.alpha = gamma * arm.alpha + reward;
    arm.beta = gamma * arm.beta + (1.0 - reward);
    arm.total_observations += 1;
}
```

---

## Interaction with Stability Mechanisms

Thompson Sampling's stochastic selection naturally provides exploration, but in combination with the cascade router's hysteresis mechanism, it can create oscillation between near-equal models. The hysteresis threshold (10% score delta to switch models) acts as a damper:

```
Current model: claude-sonnet-4 (sampled θ = 0.82)
Challenger: claude-opus-4 (sampled θ = 0.85)
Delta: 0.85 − 0.82 = 0.03 < 0.10 (hysteresis threshold)
→ Keep current model (no switch)
```

This prevents the stochastic nature of Thompson Sampling from causing rapid model switching when multiple models have similar performance. See [14-stability-mechanisms](14-stability-mechanisms.md) for the full hysteresis design.

---

## Drift Detection

Thompson Sampling with discount handles gradual drift automatically (the discount factor continuously down-weights old data). For sudden, abrupt changes (e.g., a provider deploys a breaking change), an additional drift detection mechanism can trigger a "reset":

```
If recent_pass_rate(last 10) << historical_pass_rate(last 100):
    Reset arm: α ← 1, β ← 1 (uninformative prior)
    → Full re-exploration for this arm
```

This combines the anomaly detection from [09-provider-health-circuit-breaker](09-provider-health-circuit-breaker.md) with the Thompson Sampling state: when the circuit breaker detects a provider degradation, the corresponding Thompson arm is reset to force re-evaluation.

---

## Contextual Thompson Sampling

Thompson Sampling can also be extended with context features, creating a Bayesian analogue to LinUCB. The contextual version maintains a posterior distribution over the weight vector `θ_a` for each arm:

```
Prior: θ_a ~ N(μ_0, Σ_0)
After observation (x, r): update posterior via Bayesian linear regression
Selection: sample θ_a from posterior, compute score = θ_a^T · x
```

This provides the exploration benefits of Thompson Sampling (stochastic selection based on posterior uncertainty) with the context-awareness of LinUCB (feature-dependent scoring).

### When to Use Contextual Thompson vs LinUCB

| Criterion | LinUCB | Contextual Thompson |
|-----------|--------|-------------------|
| Stationary environment | Preferred | Either |
| Non-stationary environment | Poor | Preferred (with discount) |
| Deterministic exploration | Yes | No (stochastic) |
| Posterior uncertainty | Point estimate + bound | Full distribution |
| Computational cost | Lower (matrix inverse) | Higher (sampling) |

For Roko's model routing, LinUCB is currently preferred because:
1. The 18-dimensional context space is well-suited to linear models.
2. Deterministic exploration (UCB bound) provides reproducible routing for debugging.
3. The stationary assumption holds over short periods (50-200 episodes).

Thompson Sampling with drift would be adopted when model provider updates create significant non-stationarity that LinUCB handles poorly.

---

## Empirical Guidance

### Monitoring Drift

The system can detect when Thompson Sampling with drift would outperform LinUCB by monitoring:

1. **Prediction error trend**: If the cascade router's predictions degrade steadily, the environment is non-stationary and Thompson with drift may help.
2. **Arm switching frequency**: If the bandit switches arms frequently (> 20% of decisions), the reward landscape is changing and a discount factor would help stabilize.
3. **Calibration drift**: If the CalibrationTracker (see [16-predictive-foraging](16-predictive-foraging.md)) shows systematic bias that increases over time, the model quality distribution is shifting.

### Adaptive Discount Factor

Rather than fixing γ, the system can adapt it based on observed non-stationarity:

```
If arm_switching_rate > 0.20:
    γ ← max(0.90, γ − 0.01)    // Increase forgetting
If arm_switching_rate < 0.05:
    γ ← min(0.999, γ + 0.01)   // Decrease forgetting
```

This ensures that the discount factor tracks the actual rate of change in the environment, rather than relying on a fixed prior assumption about non-stationarity.

---

## Relationship to Other Documents

- **[03-bandits-ucb-thompson-linucb](03-bandits-ucb-thompson-linucb.md)** — Foundational bandit algorithms that Thompson Sampling extends.
- **[04-cascade-router](04-cascade-router.md)** — Thompson Sampling is a proposed alternative to LinUCB for stage-3 routing.
- **[09-provider-health-circuit-breaker](09-provider-health-circuit-breaker.md)** — Circuit breaker events can trigger Thompson arm resets.
- **[10-pareto-frontier-pruning](10-pareto-frontier-pruning.md)** — Pareto pruning reduces the arm set before Thompson selection.
- **[14-stability-mechanisms](14-stability-mechanisms.md)** — Hysteresis prevents Thompson Sampling oscillation.
- **[12-self-improvement-frameworks](12-self-improvement-frameworks.md)** — Academic context for non-stationary bandit algorithms.
