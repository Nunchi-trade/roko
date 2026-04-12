# Morphogenetic Specialization: Turing Reaction-Diffusion for Role Emergence

> **Layer**: L4 Orchestration (multi-agent role coordination), with cross-cuts into L1
> Framework (agent type system) and L3 Harness (monitoring specialization health)
>
> **Synapse traits**: `Policy` (morphogenetic update rule), `Scorer` (evaluate role vector
> fitness), `Router` (select specialization direction), `Substrate` (store role vectors)
>
> **Prerequisites**: `00-stigmergy-theory.md` (stigmergy fundamentals),
> `03-digital-pheromones.md` (pheromone mechanics),
> `06-agent-mesh-sync.md` (how role vectors propagate)

---

## The Problem: Identical Agents, Redundant Work

When a Collective (group of agents sharing a common purpose) starts with identically configured
agents, a coordination problem arises: all agents pursue the same strategies, compete for the
same tasks, and produce redundant work. This is the **niche crowding problem** — too many
generalists, no specialists.

The solution comes from developmental biology. Alan Turing's reaction-diffusion mechanism
[Turing, A.M. "The Chemical Basis of Morphogenesis." *Philosophical Transactions of the Royal
Society B*, 237(641):37-72, 1952] explains how initially identical cells differentiate into
specialized tissues during embryonic development. The same mechanism, applied to agents,
produces spontaneous role differentiation from homogeneous starting conditions.

---

## Turing's Reaction-Diffusion Mechanism

### The Biological Inspiration

In 1952, Turing showed that a system of two chemicals — an **activator** and an **inhibitor** —
can produce stable spatial patterns from a uniform initial state, provided:

1. The activator amplifies itself and the inhibitor (positive feedback locally)
2. The inhibitor suppresses the activator (negative feedback)
3. **The inhibitor diffuses faster than the activator** (D_B >> D_A)

The third condition is the key insight: because inhibition spreads faster than activation, a
local concentration of activator suppresses activator production in its neighborhood while
reinforcing itself. The result is a pattern of peaks (high activator concentration) separated
by troughs (low activator, high inhibitor) — stable, self-organizing, and requiring no central
planner.

Gierer & Meinhardt formalized this as the activator-inhibitor model [Gierer, A. & Meinhardt,
H. "A Theory of Biological Pattern Formation." *Kybernetik*, 12(1):30-39, 1972]:

```
∂a/∂t = ρ_a × (a² / h) - μ_a × a + D_a × ∇²a + σ_a    (activator)
∂h/∂t = ρ_h × a²       - μ_h × h + D_h × ∇²h + σ_h    (inhibitor)
```

Where:
- `a` = activator concentration, `h` = inhibitor concentration
- `ρ` = production rate, `μ` = decay rate, `D` = diffusion coefficient
- `σ` = noise (essential for symmetry breaking)

The instability condition (Turing instability) requires: `D_h / D_a >> 1`

### Why This Applies to Agent Collectives

In a Roko Collective:

| Biological Component | Roko Equivalent |
|---------------------|-----------------|
| Activator | Profitable returns for a strategy dimension — local, slow (learning takes hundreds of ticks) |
| Inhibitor | Collective-wide pheromone signals showing other agents' specializations — propagates fast via Agent Mesh (milliseconds) |
| Diffusion asymmetry (D_B >> D_A) | Learning is slow (individual experience) but inhibition is fast (pheromone sync via Mesh) |
| Noise (σ) | Small random perturbations to break initial symmetry |
| Spatial pattern | Role differentiation — each agent specializes in a different strategy dimension |

Because inhibition (awareness of others' roles via pheromone sync) propagates through the
Agent Mesh in milliseconds while activation (learning that a strategy is profitable) requires
hundreds of ticks of experience, **Turing's instability condition is naturally satisfied.**
Stable specialist patterns emerge from initially homogeneous populations without any central
role assignment.

---

## The Strategy Concentration Vector

Each agent maintains an 8-dimensional strategy concentration vector that represents its current
role:

```rust
/// The number of strategy dimensions.
/// Each dimension represents a broad strategy type.
///
/// The 8 default dimensions are domain-agnostic:
///   0: depth       — deep analysis of narrow topics
///   1: breadth     — broad survey across many topics
///   2: execution   — implementing and building
///   3: verification — testing and validation
///   4: time_horizon — long-term vs short-term planning
///   5: exploration  — trying new approaches
///   6: exploitation — optimizing known approaches
///   7: coordination — managing multi-agent workflows
///
/// Domain plugins can redefine these dimensions:
///   Code:  [refactoring, feature_dev, testing, docs, perf, security, deps, arch]
///   DeFi:  [momentum, mean_reversion, lp, risk, time_horizon, asset_breadth, vol, cross_chain]
pub const STRATEGY_DIMS: usize = 8;

/// Morphogenetic state for an agent.
///
/// Tracks the agent's current specialization and the signals
/// needed to update it via the reaction-diffusion mechanism.
pub struct MorphogeneticState {
    /// 8-dimensional strategy concentration vector.
    /// Values are in [0, 1] and sum to 1.0.
    /// A generalist has uniform concentration (each dimension ≈ 0.125).
    /// A specialist has high concentration in 1-2 dimensions and low elsewhere.
    pub strategy: [f64; STRATEGY_DIMS],

    /// Per-dimension returns attributed since last update.
    /// Positive returns reinforce (activate) the dimension.
    /// Negative returns suppress (additional inhibition) the dimension.
    pub attributed_returns: [f64; STRATEGY_DIMS],

    /// Aggregated strategy vectors from all Collective members.
    /// Each entry is the sum of all received role_vector[k] values.
    /// Used to compute inhibition pressure.
    pub collective_pheromone: [f64; STRATEGY_DIMS],

    /// Number of agents in the Collective.
    /// Used to normalize inhibition pressure.
    pub collective_size: usize,
}
```

### Specialization Index

The specialization index measures how specialized an agent is, using normalized Shannon entropy:

```rust
/// Compute the specialization index of a strategy vector.
///
/// Returns a value in [0, 1]:
/// - 0.0 = maximum specialization (all concentration in one dimension)
/// - 1.0 = maximum generalization (uniform distribution)
///
/// Formula: 1 - H(s) / H_max
/// where H(s) = -Σ s_k × ln(s_k) and H_max = ln(STRATEGY_DIMS)
///
/// Shannon, C.E. "A Mathematical Theory of Communication."
/// Bell System Technical Journal, 27(3):379-423, 1948.
pub fn specialization_index(strategy: &[f64; STRATEGY_DIMS]) -> f64 {
    let h: f64 = strategy.iter()
        .filter(|&&s| s > 1e-10)
        .map(|&s| -s * s.ln())
        .sum();
    let h_max = (STRATEGY_DIMS as f64).ln();
    1.0 - h / h_max
}
```

A newly initialized agent (uniform distribution, each dimension = 0.125) has specialization
index = 0.0. A pure specialist (one dimension = 1.0, others = 0.0) has specialization index
= 1.0.

---

## The Reaction-Diffusion Update Rule

Every `update_interval` ticks (default: 50, aligned with Curator cycle), each agent updates
its strategy vector according to the Gierer-Meinhardt-inspired reaction-diffusion rule:

```rust
/// Morphogenetic parameters controlling the reaction-diffusion dynamics.
pub struct MorphogeneticParams {
    /// Activation rate. How fast profitable strategies reinforce.
    /// Default: 0.05.
    pub alpha: f64,

    /// Inhibition rate. How strongly Collective overlap suppresses.
    /// Default: 0.15. Must be > alpha for Turing instability.
    pub beta: f64,

    /// Decay rate toward baseline. Prevents extreme specialization.
    /// Default: 0.01.
    pub mu: f64,

    /// Baseline concentration per dimension. Default: 1/STRATEGY_DIMS = 0.125.
    pub baseline: f64,

    /// Noise standard deviation for symmetry breaking.
    /// Default: 0.005.
    pub sigma_noise: f64,

    /// Resource pressure scalar. Modulates activation rate based on
    /// agent's current resource situation.
    /// Full resources: 1.0. Resource pressure: 0.5-0.1.
    /// This replaces the legacy "mortality_activation_scalar" concept.
    pub resource_pressure_scalar: f64,
}

/// Update the strategy vector using reaction-diffusion dynamics.
///
/// For each dimension k:
///   s_k(t+1) = s_k(t) + activation_k - inhibition_k - decay_k + noise_k
///
/// Where:
///   activation_k = alpha × resource_pressure_scalar × max(0, returns[k]) × s_k
///   inhibition_k = beta × (pheromone[k] / collective_size) × s_k
///   decay_k      = mu × (s_k - baseline)
///   noise_k      ~ N(0, sigma_noise²)
///
/// After update, the vector is renormalized to sum to 1.0.
///
/// References:
/// - Turing 1952, "The Chemical Basis of Morphogenesis"
/// - Gierer & Meinhardt 1972, "A Theory of Biological Pattern Formation"
pub fn update(
    state: &mut MorphogeneticState,
    params: &MorphogeneticParams,
    rng: &mut impl Rng,
) {
    let mut new_strategy = [0.0f64; STRATEGY_DIMS];

    for k in 0..STRATEGY_DIMS {
        let s_k = state.strategy[k];

        // Activation: profitable strategies reinforce (slow, local)
        let activation = params.alpha
            * params.resource_pressure_scalar
            * state.attributed_returns[k].max(0.0)
            * s_k;

        // Inhibition: Collective overlap suppresses (fast, diffused)
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
        let noise = Normal::new(0.0, params.sigma_noise)
            .unwrap()
            .sample(rng);

        // Update
        new_strategy[k] = (s_k + activation - inhibition - decay + noise).max(0.0);
    }

    // Renormalize to sum = 1.0
    let total: f64 = new_strategy.iter().sum();
    if total > 1e-10 {
        for k in 0..STRATEGY_DIMS {
            new_strategy[k] /= total;
        }
    } else {
        // Edge case: all dimensions collapsed. Reset to baseline.
        new_strategy = [params.baseline; STRATEGY_DIMS];
    }

    state.strategy = new_strategy;
    // Reset returns for next cycle
    state.attributed_returns = [0.0; STRATEGY_DIMS];
}
```

### Why beta > alpha Is Essential

The critical condition for Turing instability is that inhibition diffuses faster than
activation. In Roko's implementation:

- **Activation** (`alpha = 0.05`) is driven by the agent's own experience — it takes many
  ticks to accumulate meaningful attributed returns. This is "slow" in the same way that
  cellular activation is slow (gene expression, protein synthesis).
- **Inhibition** (`beta = 0.15`) is driven by the Collective's pheromone field — role vectors
  propagate via Agent Mesh in milliseconds. This is "fast" in the same way that diffusion
  through extracellular medium is fast.

With `beta = 3 × alpha`, the inhibition rate ensures that an agent's specialization is
suppressed in dimensions where other Collective members are already concentrated. This pushes
agents apart in strategy space, creating complementary specialists.

---

## Niche Competition

Niche competition measures how many other agents in the Collective occupy a similar role:

```rust
/// Compute the niche competition score for an agent.
///
/// Returns the number of effective competitors: Collective members
/// whose strategy vector has cosine similarity > 0.8 with this agent's
/// strategy vector.
///
/// High niche competition (>2) indicates niche crowding — the agent
/// should consider respecializing toward an underserved dimension.
///
/// Based on Lotka-Volterra competition dynamics:
///   dN_i/dt = r_i × N_i × (1 - Σ α_ij × N_j / K_i)
/// where α_ij measures competitive overlap between species i and j.
///
/// References:
/// - Lotka, A.J. "Elements of Physical Biology." Williams & Wilkins, 1925.
/// - Volterra, V. "Fluctuations in the Abundance of a Species." Nature, 1926.
pub fn niche_competition(
    my_strategy: &[f64; STRATEGY_DIMS],
    collective_strategies: &[[f64; STRATEGY_DIMS]],
    similarity_threshold: f64,  // default: 0.8
) -> f32 {
    collective_strategies.iter()
        .filter(|other| cosine_similarity(my_strategy, other) > similarity_threshold)
        .count() as f32
}

fn cosine_similarity(a: &[f64; STRATEGY_DIMS], b: &[f64; STRATEGY_DIMS]) -> f64 {
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let mag_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if mag_a < 1e-10 || mag_b < 1e-10 { return 0.0; }
    dot / (mag_a * mag_b)
}
```

---

## Pheromone-Based Role Coordination

Three message types coordinate morphogenetic specialization through the Agent Mesh (see
`06-agent-mesh-sync.md`):

### Role Broadcast

Included in every Curator-aligned batch sync (every 50 ticks). Carries the agent's current
role vector, specialization index, and resource state:

```rust
/// Morphogenetic role vector broadcast in collective sync.
/// Piggybacks on the existing batch sync message — 72 bytes overhead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MorphogeneticPheromone {
    /// 8-dimensional strategy concentration vector. Sum = 1.0.
    pub role_vector: [f64; STRATEGY_DIMS],
    /// Shannon entropy of the role vector, normalized to [0, 1].
    /// Low entropy = specialist. High entropy = generalist.
    pub specialization_index: f32,
    /// Tick at which this pheromone was emitted.
    pub emitted_at_tick: u64,
    /// Agent's current resource state (affects activation rate).
    pub resource_state: String,
}
```

### Niche Vacancy Alert

Emitted when a Collective member is removed (shutdown, resource exhaustion, reassignment) and
its strategy niche drops below the occupancy threshold. This triggers accelerated
respecialization in remaining agents — they can sense the newly vacant niche and adjust their
strategy vectors to fill it.

```rust
/// Niche vacancy alert — pushed immediately when a niche opens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NicheVacancy {
    /// The departed agent's role vector at time of removal.
    pub vacated_role: [f64; STRATEGY_DIMS],
    /// Dimensions with concentration > 0.3 (specialist dimensions).
    pub specialist_dimensions: Vec<usize>,
    /// The departed agent's ID (for lineage tracking).
    pub departed_agent_id: AgentId,
    /// Tick of departure.
    pub departure_tick: u64,
}
```

### Role Conflict Alert

Emitted when two agents' role vectors have cosine similarity > 0.9 for more than 100
consecutive ticks, indicating redundant specialization. Suggests that one agent should
respecialize toward an underserved dimension.

```rust
/// Role conflict alert — pushed when two agents overlap too much.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleConflict {
    /// The other agent in the conflict.
    pub conflicting_agent_id: AgentId,
    /// Overlapping dimensions (concentration > 0.2 in both).
    pub overlapping_dimensions: Vec<usize>,
    /// How many ticks the conflict has persisted.
    pub conflict_duration_ticks: u64,
    /// Suggested respecialization direction: the dimension with lowest
    /// aggregate Collective concentration.
    pub suggested_dimension: usize,
}
```

---

## Resource Pressure and Specialization Dynamics

The `resource_pressure_scalar` modulates morphogenetic dynamics based on the agent's resource
situation:

| Resource State | Scalar | Effect |
|---------------|--------|--------|
| Full resources | 1.0 | Full activation rate. Agent deepens specialization based on returns. |
| Moderate pressure | 0.5 | Halved activation. Agent becomes more responsive to inhibition. Allows potential respecialization. |
| High pressure | 0.1 | Near-zero activation. Agent effectively surrenders its niche. If resources recover, it respecializes based on current Collective structure. |
| Exhausted | 0.0 | No activation. Role vector freezes. Agent's pheromone signal persists until processed. |

This mechanism replaces the legacy "mortality phase" modulation. Instead of mapping agent
lifecycle phases (Thriving → Terminal) to activation scalars, Roko uses a continuous resource
pressure measure that reflects the agent's actual operational situation.

When an agent recovers from resource pressure, it starts respecializing based on the current
Collective pheromone field. If its previous niche has been filled by another agent during the
pressure period, the recovering agent will be pushed toward a different niche by inhibition
pressure — ensuring that the Collective maintains diversity.

---

## Convergence Analysis

### Timeline

From homogeneous initial conditions (all agents at baseline = 0.125 per dimension), the
reaction-diffusion dynamics typically converge in 500–2000 ticks, depending on Collective
size and parameter settings.

| Collective Size | Typical Convergence | Specialist Patterns |
|----------------|--------------------|--------------------|
| 2 agents | ~500 ticks | 2 complementary specialists |
| 5 agents | ~800 ticks | 3-5 specialists (some overlap) |
| 10 agents | ~1200 ticks | 5-8 specialists |
| 20 agents | ~1800 ticks | 8 specialists (all dimensions covered) |

### Stability Condition

The system is considered stable when the trait variance drops below a threshold:

```
Variance(strategy) = Σ_k (s_k(t) - s_k(t-50))² < 0.01 for 100 consecutive ticks
```

### Edge of Chaos (Kauffman 1993)

The parameter settings are calibrated to keep the system at the "edge of chaos" — the boundary
between ordered and chaotic behavior where computational capacity is maximized [Kauffman, S.
*The Origins of Order*. Oxford University Press, 1993]:

- **Too much activation** (alpha >> beta): Agents lock into their initial random biases;
  no diversity.
- **Too much inhibition** (beta >> alpha): All agents collapse to uniform baseline; no
  specialization.
- **Near-critical** (beta ≈ 3 × alpha): Stable specialist patterns emerge from noise;
  the system can adapt when conditions change.

The decay parameter (`mu = 0.01`) provides a gentle restoring force toward the baseline,
preventing extreme specialization that would make agents brittle. The noise parameter
(`sigma_noise = 0.005`) provides the symmetry-breaking perturbations that initiate
pattern formation.

### Pathological Configurations

Collectives with >50 agents and adversarial traits may not converge. In such cases, the
Collective should be partitioned into smaller sub-collectives, each of which can achieve
stable specialization independently.

---

## DeLanda's Assemblage Theory

Manuel DeLanda's assemblage theory provides a philosophical framework for understanding
morphogenetic specialization in agent Collectives [DeLanda, M. *A New Philosophy of Society:
Assemblage Theory and Social Complexity*. Continuum, 2006]:

- Agents are **components** with **capacities** (what they can do) that are not fully
  determined by their properties alone.
- The Collective is an **assemblage** — a heterogeneous whole whose properties emerge from
  the interactions of its components.
- Specialization is a process of **territorialization** — agents settle into defined roles
  that stabilize the assemblage.
- Respecialization (triggered by niche vacancies or role conflicts) is a process of
  **deterritorialization** — agents leave their established roles and find new ones.

This framework helps explain why morphogenetic specialization is not a one-time event but an
ongoing dynamic process: the assemblage continuously adjusts as agents join, leave, improve,
and encounter new challenges.

---

## Configuration

```toml
[mesh.collective.morphogenetic]
# Enable reaction-diffusion specialization.
enabled = true           # default: true (requires mesh enabled)
# Activation rate: how fast profitable strategies reinforce.
alpha = 0.05             # default: 0.05
# Inhibition rate: how strongly Collective overlap suppresses.
beta = 0.15              # default: 0.15
# Decay rate toward baseline (prevents extreme specialization).
mu = 0.01                # default: 0.01
# Noise standard deviation for symmetry breaking.
sigma_noise = 0.005      # default: 0.005
# Emit NicheVacancy alerts on agent departure.
vacancy_alerts = true    # default: true
# Emit RoleConflict alerts after this many ticks of high similarity.
conflict_threshold_ticks = 100  # default: 100
# Strategy dimensions (can be overridden by domain plugins).
# strategy_dims = 8     # default: 8
```

---

## Summary

Morphogenetic specialization transforms a group of identical agents into a diverse ecology of
complementary specialists through:

1. **Activation** (slow, local): Profitable strategies reinforce via experience
2. **Inhibition** (fast, global): Collective pheromone signals suppress overlap
3. **Decay** (constant): Gentle restoring force prevents extreme lock-in
4. **Noise** (random): Breaks initial symmetry to initiate pattern formation

The mechanism requires no central role assignment, no negotiation between agents, and no
explicit task allocation. Roles emerge from the same stigmergic pheromone infrastructure that
handles all other coordination — making specialization a natural consequence of collective
operation rather than an engineered feature.

---

## References

- [DeLanda 2006] *A New Philosophy of Society*, Continuum
- [Gierer & Meinhardt 1972] Biological Pattern Formation, *Kybernetik*
- [Kauffman 1993] *The Origins of Order*, Oxford University Press
- [Lotka 1925] *Elements of Physical Biology*, Williams & Wilkins
- [Shannon 1948] Mathematical Theory of Communication, *Bell System Technical Journal*
- [Turing 1952] Chemical Basis of Morphogenesis, *Phil. Trans. Royal Society B*
- [Volterra 1926] Fluctuations in Species Abundance, *Nature*

---

## Related Sub-Docs

- `00-stigmergy-theory.md` — Stigmergy foundations
- `03-digital-pheromones.md` — Pheromone mechanics underlying role coordination
- `06-agent-mesh-sync.md` — Transport for morphogenetic signals
- `09-stigmergy-scaling.md` — Scaling properties of the morphogenetic system
- `11-collective-intelligence-metrics.md` — Measuring specialization effectiveness
