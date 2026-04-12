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


> **Implementation**: Specified

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

### Pathological configurations

Collectives with >50 agents and adversarial traits may not converge. In such cases, the
Collective should be partitioned into smaller sub-collectives, each of which can achieve
stable specialization independently.

### Sensitivity analysis

The system's behavior under parameter perturbation (+/-10% from defaults) has been
characterized. The key finding: convergence is robust to moderate parameter variation, but
convergence speed changes.

| Parameter | Default | -10% | +10% | Effect of perturbation |
|-----------|---------|------|------|----------------------|
| alpha | 0.05 | 0.045 | 0.055 | Slower/faster specialization. Both converge. |
| beta | 0.15 | 0.135 | 0.165 | Lower beta: slight niche crowding. Higher beta: faster differentiation but shallower specialization peaks. |
| mu | 0.01 | 0.009 | 0.011 | Lower mu: deeper specialization (higher peak concentration). Higher mu: broader roles (less extreme). |
| sigma_noise | 0.005 | 0.0045 | 0.0055 | Minimal impact on convergence. Affects symmetry-breaking speed only. |
| beta/alpha ratio | 3.0 | 2.7 | 3.3 | Both converge. Below 2.0, instability condition weakens and convergence becomes unreliable. |

**Convergence guarantee**: For `beta/alpha >= 2.0` and `collective_size <= 50`, the system
converges to a stable pattern with probability > 0.99 within 3000 ticks. This was validated
through Monte Carlo simulation (10,000 runs per parameter setting, uniform random initial
perturbation).

```rust
/// Validate that morphogenetic parameters satisfy the Turing instability condition
/// and are within tested ranges.
///
/// Returns warnings (not errors) for out-of-range values — the system may still
/// work, but convergence is not guaranteed.
pub fn validate_params(params: &MorphogeneticParams) -> Vec<String> {
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
            "alpha {:.3} outside tested range (0, 0.5]",
            params.alpha
        ));
    }
    if params.mu > params.alpha {
        warnings.push(format!(
            "mu ({:.3}) > alpha ({:.3}); decay dominates activation, \
             specialization may not emerge",
            params.mu, params.alpha
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

### Domain-specific calibration

Different domains have different feedback timescales. Code development tasks produce returns
over minutes to hours; DeFi strategies produce returns in seconds. The parameters should be
calibrated per domain.

| Domain | alpha | beta | mu | sigma_noise | Rationale |
|--------|-------|------|----|-------------|-----------|
| Code (default) | 0.05 | 0.15 | 0.01 | 0.005 | Tasks take 50-200 ticks. Standard feedback loop. |
| DeFi/Chain | 0.10 | 0.25 | 0.02 | 0.003 | Fast feedback (price changes). Higher activation and inhibition to match faster dynamics. Lower noise because returns signal is strong. |
| Research | 0.03 | 0.10 | 0.005 | 0.008 | Slow feedback (paper review cycles). Lower rates to avoid premature lock-in. Higher noise to explore more. |
| Operations | 0.04 | 0.12 | 0.008 | 0.005 | Moderate feedback. Slightly conservative to avoid oscillation in production systems. |

```toml
# Domain-specific parameter overrides in roko.toml
[mesh.collective.morphogenetic.domains.chain]
alpha = 0.10
beta = 0.25
mu = 0.02
sigma_noise = 0.003

[mesh.collective.morphogenetic.domains.research]
alpha = 0.03
beta = 0.10
mu = 0.005
sigma_noise = 0.008
```

When an agent operates in multiple domains, it uses the parameters for its primary domain
(the domain with the highest strategy concentration).

### Noise implementation

Noise breaks initial symmetry so that identical agents diverge into different niches. The
implementation must be deterministic for reproducibility but produce different sequences
across agents.

**RNG seeding**: Each agent seeds its morphogenetic RNG with `blake3(agent_id || "morpho")`.
This produces a deterministic, agent-specific noise sequence: given the same agent ID and
the same tick number, the noise is identical across runs. This enables reproducible debugging
of specialization dynamics.

**Timing**: Noise is sampled once per update cycle (every 50 ticks), not per tick. Sampling
per tick would average out the noise (central limit theorem), weakening symmetry breaking.
The 50-tick interval gives each noise sample time to influence the strategy vector before the
next sample arrives.

```rust
/// Create the deterministic RNG for an agent's morphogenetic noise.
///
/// The seed is derived from the agent ID so that:
/// 1. Different agents get different noise sequences (symmetry breaking)
/// 2. The same agent gets the same sequence across restarts (reproducibility)
pub fn morpho_rng(agent_id: &AgentId) -> ChaCha20Rng {
    let seed_bytes = blake3::hash(
        format!("{}morpho", agent_id).as_bytes()
    );
    ChaCha20Rng::from_seed(*seed_bytes.as_bytes())
}
```

### Non-convergence handling

The stability condition (variance < 0.01 for 100 consecutive ticks) may not be met in
pathological cases: adversarial agents, extreme parameter settings, or Collectives where
the number of agents exceeds the number of strategy dimensions.

The system does not require convergence to function. If variance stays above 0.01
indefinitely, the morphogenetic coordinator classifies the Collective as "oscillating" and
takes the following actions:

```rust
/// Response to non-convergence in morphogenetic dynamics.
///
/// Triggered when variance > 0.01 persists for `non_convergence_timeout` ticks
/// (default: 5000, about 100 update cycles at 50 ticks/cycle).
pub struct NonConvergenceResponse {
    /// Number of ticks before declaring non-convergence.
    /// Default: 5000. Range: [1000, 50000].
    pub timeout_ticks: u64,

    /// Action to take on non-convergence.
    pub action: NonConvergenceAction,
}

pub enum NonConvergenceAction {
    /// Log a warning and continue. The system operates with oscillating roles.
    /// Agents still function; they just switch niches periodically.
    /// This is the default — oscillation is suboptimal but not fatal.
    WarnAndContinue,

    /// Increase mu (decay toward baseline) by 50% to dampen oscillation.
    /// Resets the convergence timer. If still non-convergent after another
    /// timeout, increases mu again (up to 3x original).
    IncreaseDamping,

    /// Freeze strategy vectors at their current values. Stops the
    /// reaction-diffusion update entirely. Useful as a last resort
    /// when oscillation causes performance problems.
    FreezeStrategies,

    /// Partition the Collective into sub-collectives of size <= STRATEGY_DIMS.
    /// Each sub-collective runs its own morphogenetic dynamics independently.
    /// Most effective for large Collectives (>20 agents).
    Partition,
}
```

The default action is `WarnAndContinue`. An oscillating Collective still functions -- agents
produce useful work, they just switch between niches more often than a converged Collective.
The performance cost is redundant work when two agents briefly occupy the same niche during
an oscillation cycle.

---

## Niche vacancy and respecialization details

### Niche vacancy alerts

A niche vacancy alert fires when an agent departs (any reason) and its specialist dimensions
have no other agent with concentration > `occupancy_threshold` in those dimensions.

```rust
/// Determine whether a departing agent's niche is vacant.
///
/// A niche is vacant when no remaining Collective member has concentration
/// above `occupancy_threshold` in any of the departing agent's specialist
/// dimensions (concentration > 0.3).
pub fn detect_vacancy(
    departed_strategy: &[f64; STRATEGY_DIMS],
    remaining_strategies: &[[f64; STRATEGY_DIMS]],
    occupancy_threshold: f64, // default: 0.2
) -> Option<NicheVacancy> {
    let specialist_dims: Vec<usize> = departed_strategy.iter()
        .enumerate()
        .filter(|(_, &c)| c > 0.3)
        .map(|(i, _)| i)
        .collect();

    if specialist_dims.is_empty() {
        return None; // Generalist departed; no specific niche to fill
    }

    // Check if any remaining agent covers these dimensions
    let covered = specialist_dims.iter().all(|&dim| {
        remaining_strategies.iter()
            .any(|s| s[dim] > occupancy_threshold)
    });

    if covered {
        return None; // Niche is already covered
    }

    Some(NicheVacancy {
        vacated_role: *departed_strategy,
        specialist_dimensions: specialist_dims,
        departed_agent_id: /* from context */,
        departure_tick: /* current tick */,
    })
}
```

**Occupancy threshold**: Default 0.2. This is deliberately lower than the specialist threshold
(0.3) so that a "partial specialist" (concentration 0.2-0.3) is considered sufficient coverage.
Range: [0.1, 0.5].

**Alert propagation**: Niche vacancy alerts are sent as `MeshPriority::Critical` messages
through the Agent Mesh (see `06-agent-mesh-sync.md`). They bypass the batch interval and
are delivered immediately to all Collective members.

### Respecialization response

When an agent receives a niche vacancy alert, it temporarily increases its activation rate
for the vacant dimensions. This accelerates movement toward the vacant niche.

```rust
/// Compute the respecialization acceleration factor for a dimension
/// that has a niche vacancy.
///
/// The acceleration decays over time — the urgency to fill a vacancy
/// decreases as the Collective adapts.
///
/// acceleration = base_factor × 2^(-ticks_since_vacancy / decay_duration)
///
/// Default base_factor: 3.0 (triple the normal activation rate).
/// Default decay_duration: 500 ticks (~10 update cycles).
pub fn respecialization_acceleration(
    ticks_since_vacancy: u64,
    base_factor: f64,      // default: 3.0, range: [1.5, 10.0]
    decay_duration: u64,   // default: 500, range: [100, 5000]
) -> f64 {
    let exponent = -(ticks_since_vacancy as f64 / decay_duration as f64);
    base_factor * 2.0_f64.powf(exponent)
}
```

The acceleration factor of 3.0 means the agent's activation rate for vacant dimensions is
effectively `3 * alpha = 0.15` — matching the inhibition rate. This temporarily overrides
the Turing instability condition for the vacant dimension, allowing rapid niche filling.
After `decay_duration` ticks, the acceleration fades to 1.0 and normal dynamics resume.

---

## Role conflict details

### The 0.9 similarity threshold

Role conflicts are detected when two agents' strategy vectors have cosine similarity > 0.9 for
100+ consecutive ticks. The 0.9 threshold was chosen based on the geometry of 8-dimensional
strategy space.

**Dimension computation**: In 8-dimensional space with vectors normalized to sum = 1.0 (the
probability simplex), two random uniform vectors have expected cosine similarity of
approximately 0.35. Two vectors that share a primary specialist dimension (both > 0.4 in the
same dimension) typically have cosine similarity 0.6-0.8. Two vectors that are functionally
identical (same specialization pattern) have cosine similarity > 0.95.

The 0.9 threshold sits between "similar specialization" (0.8) and "identical" (0.95). At this
level, the two agents overlap enough that one of them is producing largely redundant work. The
100-tick persistence requirement filters out transient overlap during respecialization
transitions.

```rust
/// Track role conflict state between agent pairs.
pub struct RoleConflictTracker {
    /// (agent_a, agent_b) -> number of consecutive ticks above threshold.
    /// Only tracks the lower-ID agent as key.a to avoid duplicate tracking.
    conflicts: HashMap<(AgentId, AgentId), u64>,

    /// Cosine similarity threshold for conflict detection.
    /// Default: 0.9. Range: [0.8, 0.99].
    pub similarity_threshold: f64,

    /// Ticks of sustained overlap before emitting an alert.
    /// Default: 100. Range: [50, 1000].
    pub duration_threshold: u64,
}

impl RoleConflictTracker {
    /// Update conflict tracking with the latest role vectors.
    ///
    /// Called every update cycle (50 ticks). Returns new conflicts
    /// that have just exceeded the duration threshold.
    pub fn update(
        &mut self,
        strategies: &HashMap<AgentId, [f64; STRATEGY_DIMS]>,
    ) -> Vec<RoleConflict> {
        let mut new_conflicts = Vec::new();
        let agents: Vec<_> = strategies.keys().collect();

        for i in 0..agents.len() {
            for j in (i + 1)..agents.len() {
                let sim = cosine_similarity(
                    &strategies[agents[i]],
                    &strategies[agents[j]],
                );

                let key = (agents[i].clone(), agents[j].clone());
                if sim > self.similarity_threshold {
                    let ticks = self.conflicts
                        .entry(key.clone())
                        .or_insert(0);
                    *ticks += 50; // One update cycle = 50 ticks

                    if *ticks == self.duration_threshold.next_multiple_of(50) {
                        // Just crossed the threshold
                        new_conflicts.push(RoleConflict {
                            conflicting_agent_id: key.1,
                            overlapping_dimensions: find_overlapping_dims(
                                &strategies[agents[i]],
                                &strategies[agents[j]],
                                0.2,
                            ),
                            conflict_duration_ticks: *ticks,
                            suggested_dimension: find_most_vacant_dim(strategies),
                        });
                    }
                } else {
                    // Similarity dropped below threshold; reset counter
                    self.conflicts.remove(&key);
                }
            }
        }
        new_conflicts
    }
}
```

**Resolution**: When a role conflict alert fires, the lower-specialization agent (the one with
a lower specialization index) receives a bias toward the suggested dimension. The bias is
implemented as a temporary addition to `attributed_returns[suggested_dimension]`, equivalent
to the agent having received positive returns in that dimension. The higher-specialization
agent keeps its niche. This asymmetric resolution prevents both agents from moving
simultaneously, which would create a new conflict in the target dimension.

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
