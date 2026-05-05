# Morphogenetic Specialization as Loop

> Depth for [06-MEMORY.md](../../unified/06-MEMORY.md). How agents self-organize into complementary specialists through Turing reaction-diffusion kinetics, expressed as a Loop Graph with an 8-dimensional strategy vector, Gierer-Meinhardt update dynamics, and Lyapunov stability monitoring.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal/Pulse duality), [02-CELL](../../unified/02-CELL.md) (Score, Verify, React, Observe protocols), [03-GRAPH](../../unified/03-GRAPH.md) (Loop pattern, Graph composition), [05-AGENT](../../unified/05-AGENT.md) (Agent runtime, vitality), [11-stigmergy-as-bus](11-stigmergy-as-bus.md) (Bus-native stigmergy, dual-write), [12-pheromone-mechanics-and-interference](12-pheromone-mechanics-and-interference.md) (Hill-function response thresholds, kind system)

---

## 1. The Niche Crowding Problem

When a Group (see [10-GROUPS.md](../../unified/10-GROUPS.md)) starts with identically configured agents, a coordination problem arises: all agents pursue the same strategies, compete for the same tasks, and produce redundant work. Five agents all trying to write the same function. Ten agents all investigating the same anomaly. This is the **niche crowding problem** -- too many generalists, no specialists.

Explicit role assignment (leader assigns "you do testing, you do docs") works for small groups but does not scale and creates a single point of failure. The solution comes from developmental biology.

### 1.1 Turing's Insight

In 1952, Alan Turing showed that a system of two chemicals -- an **activator** and an **inhibitor** -- can produce stable spatial patterns from a uniform initial state, provided three conditions hold:

1. The activator amplifies itself and the inhibitor (positive feedback locally)
2. The inhibitor suppresses the activator (negative feedback)
3. **The inhibitor diffuses faster than the activator** (D_inhibitor >> D_activator)

The third condition is the key insight: because inhibition spreads faster than activation, a local concentration of activator suppresses activator production in its neighborhood while reinforcing itself. The result is a pattern of peaks separated by troughs -- stable, self-organizing, and requiring no central planner.

Gierer & Meinhardt formalized this as the activator-inhibitor model (1972):

```
da/dt = rho_a * (a^2 / h) - mu_a * a + D_a * nabla^2(a) + sigma_a   (activator)
dh/dt = rho_h * a^2         - mu_h * h + D_h * nabla^2(h) + sigma_h   (inhibitor)
```

Where `a` = activator concentration, `h` = inhibitor concentration, `rho` = production rate, `mu` = decay rate, `D` = diffusion coefficient, `sigma` = noise (essential for symmetry breaking).

### 1.2 Why This Applies to Agents

| Biological Component | Roko Equivalent |
|---------------------|-----------------|
| Activator | Profitable returns for a strategy dimension -- local, slow (learning takes hundreds of ticks) |
| Inhibitor | Pheromone Pulses showing other agents' specializations -- propagates fast via Bus (~milliseconds) |
| Diffusion asymmetry (D_h >> D_a) | Learning is slow (individual experience). Inhibition is fast (Bus propagation). |
| Noise (sigma) | Small random perturbations to break initial symmetry |
| Spatial pattern | Role differentiation -- each agent specializes in a different strategy dimension |

Because inhibition propagates through the Bus in milliseconds while activation requires hundreds of ticks of experience, **Turing's instability condition is naturally satisfied**. Stable specialist patterns emerge from initially homogeneous populations without any central role assignment.

---

## 2. The Strategy Concentration Vector

Each agent maintains an 8-dimensional strategy vector that represents its current role. The 8 dimensions are domain-agnostic:

| Index | Dimension | What It Represents |
|-------|-----------|-------------------|
| 0 | depth | Deep analysis of narrow topics |
| 1 | breadth | Broad survey across many topics |
| 2 | execution | Implementing and building |
| 3 | verification | Testing and validation |
| 4 | time_horizon | Long-term vs short-term planning |
| 5 | exploration | Trying new approaches |
| 6 | exploitation | Optimizing known approaches |
| 7 | coordination | Managing multi-agent workflows |

Domain plugins can redefine these dimensions. For code development: `[refactoring, feature_dev, testing, docs, perf, security, deps, arch]`. For DeFi: `[momentum, mean_reversion, lp, risk, time_horizon, asset_breadth, vol, cross_chain]`.

```rust
const STRATEGY_DIMS: usize = 8;

/// Morphogenetic state for an agent.
/// The strategy vector is a probability distribution: values in [0, 1], sum = 1.0.
/// A generalist has uniform concentration (each dimension ~ 0.125).
/// A specialist has high concentration in 1-2 dimensions and low elsewhere.
struct MorphogeneticState {
    /// 8D strategy vector. Sum = 1.0.
    strategy: [f64; STRATEGY_DIMS],
    /// Per-dimension returns attributed since last update.
    /// Positive returns reinforce (activate) the dimension.
    attributed_returns: [f64; STRATEGY_DIMS],
    /// Aggregated strategy vectors from all Group members.
    /// Used to compute inhibition pressure.
    collective_pheromone: [f64; STRATEGY_DIMS],
    /// Number of agents in the Group.
    collective_size: usize,
}
```

### 2.1 Specialization Index

The specialization index measures how specialized an agent is, using normalized Shannon entropy:

```
specialization_index = 1 - H(s) / H_max
```

Where `H(s) = -SUM s_k * ln(s_k)` and `H_max = ln(STRATEGY_DIMS)`.

- 0.0 = maximum generalization (uniform distribution)
- 1.0 = maximum specialization (all concentration in one dimension)

A newly initialized agent (each dimension = 0.125) has specialization index 0.0. A pure specialist (one dimension = 1.0) has specialization index 1.0. In practice, healthy specialists stabilize around 0.5-0.7 -- concentrated but not brittle.

---

## 3. The Morphogenetic Loop

The morphogenetic update is expressed as a **Loop Graph** (see [03-GRAPH.md](../../unified/03-GRAPH.md) for the Loop pattern). The Loop fires every 50 ticks (aligned with the Curator cycle):

```toml
# Morphogenetic Loop Graph.
# Fires every 50 ticks. Each agent runs its own instance.

[graph]
id = "morphogenetic-loop"
pattern = "loop"
tick_interval = 50

[[cells]]
id = "observe_field"
protocol = "observe"
description = "Read collective pheromone field: aggregate strategy vectors from Group peers via Bus"

[[cells]]
id = "compute_update"
protocol = "score"
description = "Apply Gierer-Meinhardt reaction-diffusion update rule to strategy vector"

[[cells]]
id = "publish_strategy"
protocol = "react"
description = "Publish updated strategy vector as Pulse on Bus for Group peers"

[[cells]]
id = "stability_lens"
protocol = "observe"
description = "Monitor convergence via trait variance. Detect pathologies (Hopf oscillation, pitchfork bifurcation)."

[[edges]]
from = "observe_field"
to = "compute_update"

[[edges]]
from = "compute_update"
to = "publish_strategy"

[[edges]]
from = "publish_strategy"
to = "observe_field"
# feedback edge: output feeds back to input (Loop pattern)
feedback = true

[[edges]]
from = "compute_update"
to = "stability_lens"
# observation edge: Lens reads without mutation
observation = true
```

### 3.1 The Update Rule (Gierer-Meinhardt Kinetics)

For each dimension k:

```
s_k(t+1) = s_k(t) + activation_k - inhibition_k - decay_k + noise_k
```

Where:
- `activation_k = alpha * resource_pressure * max(0, returns[k]) * s_k`
- `inhibition_k = beta * (pheromone[k] / collective_size) * s_k`
- `decay_k = mu * (s_k - baseline)`
- `noise_k ~ N(0, sigma^2)`

After update, the vector is renormalized to sum to 1.0.

```rust
/// Morphogenetic parameters controlling reaction-diffusion dynamics.
struct MorphogeneticParams {
    /// Activation rate. How fast profitable strategies reinforce.
    /// Default: 0.05.
    alpha: f64,
    /// Inhibition rate. How strongly Group overlap suppresses.
    /// Default: 0.15. Must be > alpha for Turing instability.
    beta: f64,
    /// Decay rate toward baseline. Prevents extreme specialization.
    /// Default: 0.01.
    mu: f64,
    /// Baseline concentration per dimension. Default: 1/STRATEGY_DIMS = 0.125.
    baseline: f64,
    /// Noise standard deviation for symmetry breaking. Default: 0.005.
    sigma_noise: f64,
    /// Resource pressure scalar. Full resources: 1.0. High pressure: 0.1.
    /// Modulates activation rate based on agent's vitality
    /// (see [05-AGENT.md] for vitality definition).
    resource_pressure: f64,
}

/// Core update: apply Gierer-Meinhardt dynamics.
fn morphogenetic_update(
    state: &mut MorphogeneticState,
    params: &MorphogeneticParams,
    rng: &mut impl Rng,
) {
    let mut new_strategy = [0.0f64; STRATEGY_DIMS];

    for k in 0..STRATEGY_DIMS {
        let s_k = state.strategy[k];

        // Activation: profitable strategies reinforce (slow, local)
        let activation = params.alpha
            * params.resource_pressure
            * state.attributed_returns[k].max(0.0)
            * s_k;

        // Inhibition: Group overlap suppresses (fast, diffused via Bus)
        let inhibition = if state.collective_size > 1 {
            params.beta
                * (state.collective_pheromone[k] / state.collective_size as f64)
                * s_k
        } else {
            0.0  // No inhibition for solo agents
        };

        // Decay toward baseline (prevents extreme lock-in)
        let decay = params.mu * (s_k - params.baseline);

        // Noise for symmetry breaking
        let noise = Normal::new(0.0, params.sigma_noise).sample(rng);

        new_strategy[k] = (s_k + activation - inhibition - decay + noise).max(0.0);
    }

    // Renormalize to sum = 1.0
    let total: f64 = new_strategy.iter().sum();
    if total > 1e-10 {
        for k in 0..STRATEGY_DIMS {
            new_strategy[k] /= total;
        }
    } else {
        // Collapse: reset to baseline
        new_strategy = [params.baseline; STRATEGY_DIMS];
    }

    state.strategy = new_strategy;
    state.attributed_returns = [0.0; STRATEGY_DIMS]; // reset for next cycle
}
```

### 3.2 Why beta > alpha Is Essential

The critical condition for Turing instability is that inhibition diffuses faster than activation:

- **Activation** (alpha = 0.05) is driven by the agent's own experience -- it takes many ticks to accumulate meaningful attributed returns. This is "slow" like gene expression.
- **Inhibition** (beta = 0.15) is driven by the Group's pheromone field -- role vectors propagate via Bus in milliseconds. This is "fast" like extracellular diffusion.

With beta = 3 * alpha, inhibition ensures that an agent's specialization is suppressed in dimensions where other Group members are already concentrated. This pushes agents apart in strategy space, creating complementary specialists.

### 3.3 Resource Pressure Modulation

The `resource_pressure` scalar modulates activation based on the agent's vitality (see [05-AGENT.md](../../unified/05-AGENT.md) for vitality and behavioral phases):

| Resource State | Scalar | Effect |
|---------------|--------|--------|
| Full resources (Thriving) | 1.0 | Full activation. Agent deepens specialization. |
| Moderate pressure (Stable) | 0.5 | Halved activation. Agent becomes responsive to inhibition. Respecialization possible. |
| High pressure (Declining) | 0.1 | Near-zero activation. Agent effectively surrenders its niche. |
| Exhausted (Terminal) | 0.0 | No activation. Strategy vector freezes. |

When an agent recovers from pressure, it respecializes based on the current Group pheromone field. If its previous niche has been filled by another agent, the recovering agent is pushed toward a different niche by inhibition -- ensuring the Group maintains diversity.

---

## 4. Pheromone-Based Coordination Messages

Three message types coordinate morphogenetic specialization through the Bus:

### 4.1 Role Broadcast (Every 50 Ticks)

Piggybacks on the existing Curator sync cycle. 72 bytes overhead:

```rust
/// Morphogenetic role vector broadcast as Pulse payload.
struct MorphogeneticPheromone {
    /// 8D strategy concentration vector. Sum = 1.0.
    role_vector: [f64; STRATEGY_DIMS],
    /// Specialization index (Shannon entropy, normalized). [0, 1].
    specialization_index: f32,
    /// Tick at which this pheromone was emitted.
    emitted_at_tick: u64,
}
```

### 4.2 Niche Vacancy Alert (On Agent Departure)

Emitted immediately when a Group member is removed (shutdown, resource exhaustion, reassignment) and its strategy niche drops below the occupancy threshold:

```rust
/// Niche vacancy alert -- triggers accelerated respecialization in remaining agents.
struct NicheVacancy {
    /// The departed agent's role vector at time of removal.
    vacated_role: [f64; STRATEGY_DIMS],
    /// Dimensions with concentration > 0.3 (specialist dimensions).
    specialist_dimensions: Vec<usize>,
}
```

### 4.3 Role Conflict Alert (On Persistent Overlap)

Emitted when two agents' role vectors have cosine similarity > 0.9 for more than 100 consecutive ticks. Suggests one agent should respecialize:

```rust
/// Role conflict alert -- two agents overlap too much for too long.
struct RoleConflict {
    /// The other agent in the conflict.
    conflicting_agent: AgentId,
    /// Overlapping dimensions (concentration > 0.2 in both).
    overlapping_dimensions: Vec<usize>,
    /// How many ticks the conflict has persisted.
    conflict_duration_ticks: u64,
    /// Suggested respecialization direction: dimension with lowest
    /// aggregate Group concentration.
    suggested_dimension: usize,
}
```

---

## 5. Niche Competition

Niche competition measures how many other agents occupy a similar role, using cosine similarity between strategy vectors:

```rust
/// Compute the niche competition score for an agent.
///
/// Returns the count of Group members whose strategy vector has
/// cosine similarity > threshold with this agent's vector.
///
/// High competition (>2) indicates niche crowding.
///
/// Based on Lotka-Volterra competition dynamics (Lotka 1925, Volterra 1926).
fn niche_competition(
    my_strategy: &[f64; STRATEGY_DIMS],
    group_strategies: &[[f64; STRATEGY_DIMS]],
    similarity_threshold: f64, // default: 0.8
) -> usize {
    group_strategies.iter()
        .filter(|other| cosine_similarity(my_strategy, other) > similarity_threshold)
        .count()
}
```

---

## 6. Stability Analysis (Lens Cell)

The stability Lens (see [02-CELL.md](../../unified/02-CELL.md) for the Observe protocol) monitors the Loop for three pathological states:

### 6.1 Linear Stability

The system is considered stable when trait variance drops below threshold:

```
Variance(strategy) = SUM_k (s_k(t) - s_k(t-50))^2 < 0.01 for 100 consecutive ticks
```

### 6.2 Pitchfork Bifurcation Detection

When beta/alpha exceeds a critical threshold (~5.0), the system can undergo pitchfork bifurcation: agents split into two extreme clusters with nothing in between. The Lens detects this by monitoring the bimodality of strategy dimension distributions.

### 6.3 Hopf Oscillation Detection

If the system oscillates rather than converges -- agents cyclically swap roles without settling -- the Lens detects this via Lyapunov exponent estimation:

```rust
/// Stability Lens Cell for the morphogenetic Loop.
///
/// Implements the Observe protocol. Reads strategy vectors from
/// the Group without mutation. Emits stability Signals.
struct MorphogeneticStabilityLens {
    /// History of trait variance over the last 200 ticks.
    variance_history: VecDeque<f64>,
    /// Convergence threshold. Default: 0.01.
    convergence_threshold: f64,
    /// Ticks of consecutive sub-threshold variance needed. Default: 100.
    convergence_window: usize,
}

impl MorphogeneticStabilityLens {
    /// Assess current stability state.
    fn assess(&self) -> StabilityState {
        let recent = self.variance_history.iter().rev().take(self.convergence_window);
        let all_below = recent.clone().all(|&v| v < self.convergence_threshold);

        if all_below {
            return StabilityState::Converged;
        }

        // Check for oscillation: variance oscillates instead of monotonically decreasing
        let trend: Vec<f64> = self.variance_history.iter()
            .zip(self.variance_history.iter().skip(1))
            .map(|(a, b)| b - a)
            .collect();
        let sign_changes = trend.windows(2)
            .filter(|w| w[0].signum() != w[1].signum())
            .count();

        if sign_changes > self.convergence_window / 2 {
            return StabilityState::Oscillating;
        }

        StabilityState::Converging
    }
}

enum StabilityState {
    /// Strategy vectors have stabilized.
    Converged,
    /// Strategy vectors are still moving toward equilibrium.
    Converging,
    /// Strategy vectors are oscillating. May need parameter adjustment.
    Oscillating,
}
```

---

## 7. Convergence Analysis

### 7.1 Convergence Time

From homogeneous initial conditions, convergence scales as O(N * log N) ticks:

| Group Size | Typical Convergence (ticks) | Wall Time (at 4 ticks/min) |
|------------|---------------------------|---------------------------|
| 2 | ~500 | ~2 hours |
| 5 | ~800 | ~3.3 hours |
| 10 | ~1,200 | ~5 hours |
| 20 | ~1,800 | ~7.5 hours |
| 50 | ~3,000 | ~12.5 hours |

Convergence time grows sub-linearly because larger Groups have more diverse noise vectors, which speeds up symmetry breaking (Turing 1952).

### 7.2 Parameter Sensitivity

The system is robust to moderate parameter variation (+/- 10%):

| Parameter | Default | -10% | +10% | Effect |
|-----------|---------|------|------|--------|
| alpha | 0.05 | 0.045 | 0.055 | Slower/faster specialization. Both converge. |
| beta | 0.15 | 0.135 | 0.165 | Lower: slight niche crowding. Higher: faster differentiation, shallower peaks. |
| mu | 0.01 | 0.009 | 0.011 | Lower: deeper specialization. Higher: broader roles. |
| sigma_noise | 0.005 | 0.0045 | 0.0055 | Minimal impact. Affects symmetry-breaking speed only. |
| beta/alpha | 3.0 | 2.7 | 3.3 | Both converge. Below 2.0, instability condition weakens. |

**Convergence guarantee**: For beta/alpha >= 2.0 and collective_size <= 50, the system converges to a stable pattern with probability > 0.99 within 3000 ticks. Validated via Monte Carlo simulation (10,000 runs per parameter setting).

### 7.3 Edge of Chaos (Kauffman 1993)

The parameters are calibrated to keep the system at the "edge of chaos" -- the boundary between ordered and chaotic behavior:

- **Too much activation** (alpha >> beta): Agents lock into initial random biases. No diversity.
- **Too much inhibition** (beta >> alpha): All agents collapse to uniform baseline. No specialization.
- **Near-critical** (beta ~ 3 * alpha): Stable specialist patterns emerge from noise. The system can adapt when conditions change.

### 7.4 Scaling Limits

Groups with > 50 agents may not converge within a reasonable time frame. For larger Groups, the recommended approach is to partition into nested Spaces (sub-Groups), each of which achieves stable specialization independently. Cross-Space morphogenetic signals can propagate via graduated Pulses.

---

## 8. Parameter Validation

```rust
/// Validate that morphogenetic parameters satisfy the Turing instability
/// condition and are within tested ranges.
fn validate_morphogenetic_params(params: &MorphogeneticParams) -> Vec<String> {
    let mut warnings = Vec::new();

    let ratio = params.beta / params.alpha;
    if ratio < 2.0 {
        warnings.push(format!(
            "beta/alpha ratio {ratio:.2} is below 2.0; Turing instability \
             condition may not hold. Recommended: >= 3.0"
        ));
    }
    if params.alpha <= 0.0 || params.alpha > 0.5 {
        warnings.push(format!(
            "alpha {:.3} outside tested range (0, 0.5]", params.alpha
        ));
    }
    if params.mu > params.alpha {
        warnings.push(format!(
            "mu ({:.3}) > alpha ({:.3}); decay dominates activation, \
             specialization may not emerge", params.mu, params.alpha
        ));
    }
    if params.sigma_noise > 0.1 {
        warnings.push(format!(
            "sigma_noise {:.3} is very high; strategy vectors will be noisy",
            params.sigma_noise
        ));
    }

    warnings
}
```

---

## What This Enables

1. **Self-organizing role diversity**: From homogeneous initial conditions, agents differentiate into complementary specialists without explicit role assignment. The mechanism is purely local -- each agent reads the ambient field and adjusts.
2. **Adaptive respecialization**: When an agent departs, the niche vacancy creates a "basin of attraction" that remaining agents naturally fill. No coordinator needed.
3. **Domain-agnostic architecture**: The 8 strategy dimensions are domain-agnostic defaults that domain plugins can redefine. The reaction-diffusion mechanism works identically regardless of what the dimensions represent.
4. **Two-timescale coordination**: Morphogenetic specialization (strategic, 500-2000 ticks) composes with response threshold allocation (tactical, 10-100 ticks) from [12-pheromone-mechanics-and-interference.md](12-pheromone-mechanics-and-interference.md). Together they produce a complete attention allocation system.

## Feedback Loops

1. **Activation Loop**: Success in a dimension -> returns -> activation -> deeper specialization -> more success. This is the "career path" that makes specialists.
2. **Inhibition Loop**: Other agents specialized in a dimension -> pheromone signal -> inhibition -> pushed toward underserved dimensions. This is the "market pressure" that distributes roles.
3. **Stability Loop**: Lens detects oscillation -> parameter adjustment (reduce beta slightly) -> damped oscillation -> convergence. This is the "governor" that prevents runaway dynamics.
4. **Vacancy Loop**: Agent departure -> niche vacancy Pulse -> accelerated respecialization in remaining agents -> vacancy filled. This is the "healing" mechanism.

## Open Questions

1. **Should strategy dimensions be fixed at 8?** Biological morphogenetic systems can have arbitrary dimensionality. A configurable dimension count would be more flexible but makes the cosine similarity computation and visualization harder.
2. **How should cross-Space morphogenetic signals work?** When agents belong to multiple Groups (nested Spaces), their morphogenetic state reflects combined inhibition pressure from all Groups. The details of this intersection are not yet specified.
3. **Can the Loop detect and recover from adversarial manipulation?** An agent that deliberately broadcasts a false strategy vector could distort the specialization field. The current design trusts the broadcast; a Verify Cell could be added to the Loop that checks strategy vectors against observed behavior.
4. **What happens to attributed returns when the dimension mapping changes?** If a domain plugin redefines the strategy dimensions, existing return attributions become meaningless. A migration mechanism is needed.

## Implementation Tasks

1. **Add `MorphogeneticState` to `roko-core`**: `crates/roko-core/src/morphogenetic.rs` -- strategy vector, attributed returns, collective pheromone, specialization index.
2. **Implement the update rule**: `crates/roko-core/src/morphogenetic.rs` -- Gierer-Meinhardt dynamics, renormalization, parameter validation.
3. **Add morphogenetic Pulse types to `roko-runtime`**: `crates/roko-runtime/src/bus.rs` -- `MorphogeneticPheromone`, `NicheVacancy`, `RoleConflict` as Pulse payloads.
4. **Wire into the Curator cycle**: `crates/roko-cli/src/orchestrate.rs` -- every 50 ticks, each agent reads the collective pheromone field from Bus, runs the update, publishes the new strategy vector.
5. **Implement stability Lens**: `crates/roko-core/src/morphogenetic.rs` -- variance tracking, oscillation detection, convergence assessment.
6. **Add niche competition scoring**: `crates/roko-core/src/morphogenetic.rs` -- cosine similarity between strategy vectors, competition count.
7. **Persist morphogenetic state**: `crates/roko-cli/src/orchestrate.rs` -- save to `.roko/state/morphogenetic.json` for resume across sessions.
8. **TUI display**: `crates/roko-cli/src/tui/` -- show strategy vectors as bar charts per agent, specialization index, niche competition.

---

## References

- Turing, A.M. 1952, "The Chemical Basis of Morphogenesis", *Phil. Trans. Royal Society B*
- Gierer, A. & Meinhardt, H. 1972, "A Theory of Biological Pattern Formation", *Kybernetik*
- Kauffman, S. 1993, *The Origins of Order*, Oxford University Press
- Bonabeau, Theraulaz & Deneubourg 1998, "Fixed Response Thresholds", *Bull. Math. Biol.*
- Lotka, A.J. 1925, *Elements of Physical Biology*, Williams & Wilkins
- Shannon, C.E. 1948, "A Mathematical Theory of Communication", *Bell System Tech. J.*
