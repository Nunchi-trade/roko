# Pareto Frontier Pruning

> **Crate:** `roko-learn` · **Module:** `pareto.rs`
> **Wiring:** `CascadeRouter` calls `compute_pareto_frontier()` every 50 observations
> **Implementation plan:** `modelrouting/08-learning-loops.md` (task 2G.11)
> **Cross-references:** [03-bandits-ucb-thompson-linucb](03-bandits-ucb-thompson-linucb.md), [04-cascade-router](04-cascade-router.md), [08-cost-normalization](08-cost-normalization.md)

---

## Purpose

Pareto frontier pruning identifies which models are non-dominated with respect to two objectives: pass rate and cost per successful task. A model is Pareto-optimal if no other model has both a higher pass rate and a lower cost per successful task. Dominated models (worse on both metrics than some other model) are pruned from the candidate set before presenting arms to the bandit.

This serves two functions:
1. **Reduces exploration waste** — the bandit doesn't spend trials on clearly inferior models.
2. **Focuses the tradeoff** — the remaining Pareto-optimal models represent genuine cost-quality tradeoffs that the bandit must resolve.

---

## Dominance Definition

Model A dominates model B when:
- A has pass_rate ≥ B's pass_rate, AND
- A has cost_per_success ≤ B's cost_per_success, AND
- At least one inequality is strict.

```
Model A: pass_rate=0.90, cost/success=$10.00
Model B: pass_rate=0.70, cost/success=$12.00
Model C: pass_rate=0.80, cost/success=$9.00

A dominates B (higher pass rate AND lower cost).
Neither A nor C dominates the other:
  A has higher pass rate, but C has lower cost.
  → Both are Pareto-optimal.
```

---

## Algorithm

```rust
pub fn compute_pareto_frontier(
    stats: &HashMap<String, ModelObservation>
) -> Vec<String> {
    let mut frontier = Vec::new();

    for (slug_a, obs_a) in stats {
        let dominated = stats.iter().any(|(slug_b, obs_b)| {
            slug_b != slug_a
                && obs_b.pass_rate >= obs_a.pass_rate
                && obs_b.cost_per_success <= obs_a.cost_per_success
                && (obs_b.pass_rate > obs_a.pass_rate
                    || obs_b.cost_per_success < obs_a.cost_per_success)
        });

        if !dominated {
            frontier.push(slug_a.clone());
        }
    }

    frontier.sort();
    frontier
}
```

The algorithm is O(n²) where n is the number of models. With typical model counts (3-10), this is negligible.

### ModelObservation

```rust
pub struct ModelObservation {
    /// Fraction of tasks that passed.
    pub pass_rate: f64,
    /// Total cost divided by number of successful tasks.
    pub cost_per_success: f64,
    /// Average latency in milliseconds.
    pub avg_latency_ms: f64,
    /// Number of observations contributing to this summary.
    pub observations: u64,
}
```

Note that `avg_latency_ms` is tracked but not currently used in the dominance check. Future extensions may include latency as a third Pareto dimension, creating a three-objective frontier.

---

## Visualization

```
Pass Rate ↑
    1.0 │         ● A (Pareto-optimal)
        │
    0.8 │    ● C (Pareto-optimal)
        │
    0.7 │              ✗ B (dominated by A)
        │
    0.6 │
        │
    0.0 └────────────────────────────────► Cost/Success
        $0   $5    $9   $10   $12   $15
```

The Pareto frontier is the upper-left boundary of the point cloud. Points below and to the right of any frontier point are dominated.

---

## Integration with Cascade Router

The cascade router recomputes the Pareto frontier every `PARETO_RECOMPUTE_INTERVAL = 50` observations:

```
CascadeRouter::update(model, reward, cost)
    │
    ├── Update model stats (trials, successes, costs)
    │
    ├── if observations % 50 == 0:
    │       │
    │       ├── Collect ModelObservation for each model
    │       │     pass_rate = successes / trials
    │       │     cost_per_success = total_cost / successes
    │       │
    │       └── pareto_frontier = compute_pareto_frontier(observations)
    │
    └── Store frontier for use in next select() call
```

During `select()`, only models on the Pareto frontier are presented as candidates to the stage-2 or stage-3 routing algorithm. Models that fell off the frontier are excluded from consideration until the next recomputation (which may restore them if other models' statistics change).

---

## Multi-Objective Extension

The current implementation uses a two-objective Pareto frontier (pass_rate, cost_per_success). The implementation plan (modelrouting/12-advanced-patterns.md, task 2J.13) describes a multi-objective Pareto bandit that extends this to four dimensions:

| Objective | Direction | Weight |
|-----------|-----------|--------|
| Quality (pass rate) | Maximize | Configurable |
| Cost per success | Minimize | Configurable |
| Latency (p50) | Minimize | Configurable |
| Reliability (1 − error rate) | Maximize | Configurable |

The multi-objective extension uses scalarization: each objective is weighted and combined into a single score, and the Pareto frontier is computed over the scalarized scores. This preserves the O(n²) complexity while enabling richer tradeoff analysis.

---

## Edge Cases

### All Models Dominated

If the model set contains a single dominant model (highest pass rate AND lowest cost), all other models are dominated and the frontier contains only one model. In this case, the bandit has no choice to make — the dominant model is always selected. This is the expected steady-state for mature systems where one model clearly outperforms alternatives.

### Insufficient Observations

Models with very few observations have noisy statistics. A model that happened to succeed on its first 3 trials appears to have a 100% pass rate, potentially dominating models with hundreds of observations and a 90% pass rate. The cascade router mitigates this by requiring a minimum observation count before including a model in the Pareto computation. Models below this threshold are always included in the candidate set (exploration) regardless of dominance.

### New Models

When a new model is added to the system (e.g., a provider releases a new model version), it starts with zero observations and is excluded from Pareto computation. The bandit gives it maximum exploration priority (UCB1 selects unpulled arms first), ensuring it accumulates enough data for Pareto evaluation within the first 50 observations.

---

## Frontier Evolution Over Time

The Pareto frontier is not static — it evolves as the system accumulates observations and as models change.

### Cold Start

At system start with no observations, all models are on the Pareto frontier (no model has enough data to be dominated). The bandit explores uniformly.

### Convergence Phase (50-200 observations)

As statistics accumulate, dominated models begin to fall off the frontier. Typically, the model set converges to 2-3 Pareto-optimal models representing genuine tradeoffs (e.g., cheap-but-lower-quality vs. expensive-but-higher-quality).

### Steady State (200+ observations)

The frontier stabilizes. Changes occur when:
- A provider updates a model (changing its quality or cost characteristics).
- A new model is added to the system.
- The task mix changes (altering the observed pass rates).

### Provider Updates

When a provider deploys a new model version, the model's historical statistics may no longer reflect its current performance. The cascade router handles this by:
1. Detecting the model version change (via model slug comparison).
2. Discounting old observations (partial reset of the model's stats).
3. Re-including the model in the Pareto computation with reduced weight.

This ensures that a model that was previously dominated but has been improved by its provider gets a fair chance to re-enter the frontier.

---

## Practical Example

Consider a system with four models after 300 observations:

```
Model               Pass Rate   Cost/Success   On Frontier?
─────────────────────────────────────────────────────────────
claude-haiku-4.5     0.78        $0.12          YES (cheapest)
claude-sonnet-4      0.86        $0.95          YES (mid-range)
claude-opus-4        0.91        $2.40          YES (highest quality)
deepseek-chat        0.72        $0.45          NO (dominated by haiku)
```

Deepseek is dominated by haiku (haiku has both higher pass rate AND lower cost), so it's pruned from the candidate set. The bandit only considers haiku, sonnet, and opus — three models representing the genuine cost-quality tradeoff.

After a provider update where deepseek improves to 0.85 pass rate:

```
Model               Pass Rate   Cost/Success   On Frontier?
─────────────────────────────────────────────────────────────
claude-haiku-4.5     0.78        $0.12          YES (cheapest)
deepseek-chat        0.85        $0.45          YES (new: better than haiku, cheaper than sonnet)
claude-sonnet-4      0.86        $0.95          NO (dominated by deepseek!)
claude-opus-4        0.91        $2.40          YES (highest quality)
```

Now sonnet is dominated by deepseek (deepseek has nearly the same pass rate at half the cost), and deepseek enters the frontier. The bandit shifts exploration toward deepseek.

---

## Relationship to Other Documents

- **[03-bandits-ucb-thompson-linucb](03-bandits-ucb-thompson-linucb.md)** — Pareto pruning reduces the arm set presented to the bandit.
- **[04-cascade-router](04-cascade-router.md)** — The cascade router uses the Pareto frontier to filter candidates before scoring.
- **[08-cost-normalization](08-cost-normalization.md)** — Cost per success uses normalized costs from the cost normalization layer.
- **[11-thompson-sampling-drift](11-thompson-sampling-drift.md)** — Thompson Sampling with drift can be combined with Pareto pruning for non-stationary multi-objective optimization.
- **[14-stability-mechanisms](14-stability-mechanisms.md)** — Recomputation interval (every 50 observations) is a form of frequency separation.
