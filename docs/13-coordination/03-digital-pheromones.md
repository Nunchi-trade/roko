# Digital Pheromones: Typed Engrams with Decay Profiles

> **Layer**: L0 Runtime (persistence and decay timers), L1 Framework (type system and
> transport), L2 Scaffold (context assembly enrichment)
>
> **Synapse traits**: `Substrate` (store pheromone Engrams), `Scorer` (rate pheromone
> intensity and relevance), `Router` (select highest-priority signal), `Policy` (react to
> pheromone streams)
>
> **Prerequisites**: `00-stigmergy-theory.md` (stigmergy theory),
> `01-stigmergy-beyond-termites.md` (generalized stigmergy)

---

## What Are Digital Pheromones?

Digital pheromones are software analogs of the chemical pheromones used by social insects for
indirect coordination. In Roko, a digital pheromone is a typed Engram — a content-addressed,
scored, decaying unit of cognition — that carries coordination information through the system.

The concept was formalized by Parunak, Brueckner & Sauter (2005), who identified the key
properties that make biological pheromones effective as coordination mechanisms and showed how
to replicate these properties in software systems [Parunak, H.V.D., Brueckner, S.A. &
Sauter, J.A. "Digital Pheromones for Coordination of Unmanned Vehicles." *Environments for
Multi-Agent Systems*, LNCS 3374:246-263, Springer, 2005].

Roko extends Parunak's framework with three additions:

1. **Typed pheromones**: Each pheromone has a `PheromoneKind` that determines its semantic
   meaning and default decay profile (see `04-pheromone-kinds.md`).
2. **Scoped propagation**: Pheromones propagate through one of three scopes — Local, Mesh,
   or Global — controlling their audience and persistence (see `05-pheromone-scope.md`).
3. **Confirmation reinforcement**: Multiple independent deposits of the same pheromone type
   extend its effective half-life, implementing a quorum-sensing mechanism analogous to
   bacterial autoinducer accumulation [Nealson, Platt & Hastings, *J. Bacteriology*, 1970].

---

## The Pheromone Struct

The core data structure for a digital pheromone in Roko:

```rust
/// A digital pheromone — a typed Engram carrying coordination information.
///
/// Pheromones are the primary mechanism for indirect coordination between
/// agents. They are deposited into a Substrate, propagate through a scope,
/// decay over time, and influence the behavior of agents that sense them.
///
/// # Stigmergic Properties
///
/// - **Deposition**: Created by an agent via `Substrate::store()`
/// - **Diffusion**: Propagates through the Agent Mesh based on `scope`
/// - **Evaporation**: Decays exponentially according to `decay_rate`
/// - **Sensing**: Queried by other agents via `Substrate::query()`
/// - **Reinforcement**: Confirmations extend effective half-life
pub struct Pheromone {
    /// The type of coordination signal this pheromone carries.
    /// Determines the default decay profile and the semantic meaning
    /// of the signal. See `PheromoneKind` for the full taxonomy.
    pub kind: PheromoneKind,

    /// The current intensity of the pheromone signal.
    /// Range: [0.0, 1.0]. Starts at `initial_intensity` (typically 1.0)
    /// and decays exponentially over time. When intensity drops below
    /// the sensing threshold (default: 0.01), the pheromone is eligible
    /// for garbage collection.
    pub intensity: f64,

    /// The half-life of the pheromone — the duration after which
    /// intensity drops to 50% of its initial value. Different pheromone
    /// kinds have different default half-lives:
    ///
    /// | Kind     | Default Half-Life |
    /// |----------|-------------------|
    /// | Threat   | 2 hours           |
    /// | Opportunity | 4 hours        |
    /// | Wisdom   | 24 hours          |
    /// | Alpha    | 1 hour            |
    /// | Pattern  | 12 hours          |
    /// | Anomaly  | 6 hours           |
    /// | Consensus | 48 hours         |
    /// | Custom   | Configurable      |
    pub decay_rate: Duration,

    /// The agent that deposited this pheromone.
    pub source: AgentId,

    /// The propagation scope of this pheromone.
    /// - `Local(SubstrateId)`: Visible only within the agent's own store
    /// - `Mesh(CollectiveId)`: Visible to all agents in the collective
    /// - `Global`: Visible to all agents on the Korai chain
    pub scope: PheromoneScope,
}
```

### Relationship to the Engram Type

A `Pheromone` is a specialized view of the `Engram` type — Roko's universal unit of cognition.
Every Engram has a `body` field that can contain a serialized `Pheromone` payload. The
relationship is:

```
Engram {
    hash: [u8; 32],          // Content-addressed identifier
    body: Vec<u8>,           // Contains serialized Pheromone
    score: f64,              // Set by Scorer based on intensity + relevance
    parents: Vec<[u8; 32]>,  // Lineage (what Engrams led to this deposit)
    created_at: Timestamp,   // When the pheromone was deposited
    tags: HashMap<String, String>,  // Includes "pheromone_kind", "scope", etc.
}
```

The Engram's `tags` field carries metadata that the `Substrate::query()` method can filter on:

| Tag Key | Example Value | Purpose |
|---------|--------------|---------|
| `pheromone_kind` | `"Threat"` | Filter by signal type |
| `pheromone_scope` | `"Mesh(collective-42)"` | Filter by propagation scope |
| `pheromone_intensity` | `"0.87"` | Current intensity (updated on read) |
| `pheromone_confirmations` | `"3"` | Number of independent confirmations |
| `pheromone_domain` | `"code-quality"` | Domain-specific context |

---

## Exponential Decay

The most important property of digital pheromones is their decay over time. Biological
pheromones evaporate through chemical degradation; digital pheromones decay through an explicit
exponential function.

### The Decay Formula

```rust
/// Compute the current intensity of a pheromone at time `now`.
///
/// Uses exponential decay with confirmation-extended half-life.
/// The formula is:
///
///   intensity(t) = base_intensity × e^(-0.693 × elapsed / τ_effective)
///
/// where:
///   τ_effective = τ_base × (1 + confirmations × 0.5)
///
/// This means:
///   - 0 confirmations: half-life = τ_base (e.g., 2 hours for Threat)
///   - 1 confirmation:  half-life = 1.5 × τ_base (3 hours for Threat)
///   - 2 confirmations: half-life = 2.0 × τ_base (4 hours for Threat)
///   - 4 confirmations: half-life = 3.0 × τ_base (6 hours for Threat)
///
/// The 0.693 constant is ln(2), which makes the formula produce
/// exactly 50% intensity at t = τ_effective.
pub fn pheromone_decay(
    base_intensity: f64,
    deposited_at: Instant,
    half_life: Duration,
    confirmations: u32,
) -> f64 {
    let effective_half_life = half_life.mul_f64(1.0 + confirmations as f64 * 0.5);
    let elapsed = deposited_at.elapsed();
    let decay_factor = (-0.693 * elapsed.as_secs_f64()
        / effective_half_life.as_secs_f64()).exp();
    base_intensity * decay_factor
}
```

### Why Exponential Decay?

Exponential decay is chosen over linear decay or step-function decay for several reasons:

| Property | Exponential | Linear | Step Function |
|----------|-----------|--------|---------------|
| **Smoothness** | Continuous, differentiable | Continuous, not differentiable at endpoint | Discontinuous |
| **Recency bias** | Strong initially, weakens over time | Constant rate | All-or-nothing |
| **Natural interpretation** | Half-life is intuitive | "Runs out in X seconds" less intuitive | "Valid for X seconds" is simplistic |
| **Biological fidelity** | Matches chemical degradation kinetics | No biological analog | No biological analog |
| **Composition** | Product of two exponentials = one exponential | Sum of linears = linear | Minimum of steps = step |

The exponential decay function has the memoryless property: at any point in time, the expected
remaining time until the pheromone reaches a given threshold depends only on the current
intensity, not on how long the pheromone has already existed. This simplifies reasoning about
pheromone interactions.

### Decay Profiles by Kind

Each `PheromoneKind` has a default half-life calibrated to its coordination role:

| Kind | Half-Life | Rationale |
|------|-----------|-----------|
| `Threat` | 2 hours | Threats need immediate response; stale threats should fade quickly to prevent permanent avoidance of healthy areas |
| `Opportunity` | 4 hours | Opportunities are moderately time-sensitive; agents should act within a few hours |
| `Wisdom` | 24 hours | Wisdom is durable but not permanent; insights should persist long enough for multiple agents to benefit |
| `Alpha` | 1 hour | Alpha signals (first-mover advantages) are the most ephemeral; by definition, alpha decays as more agents discover it |
| `Pattern` | 12 hours | Patterns in the codebase or data are moderately persistent; confirmed patterns should last through a development cycle |
| `Anomaly` | 6 hours | Anomalies need investigation within a working day; if no agent investigates, the anomaly fades |
| `Consensus` | 48 hours | Consensus signals are the most durable; collective agreement should persist across multiple development cycles |
| `Custom(String)` | User-defined | Domain-specific pheromones use whatever half-life is appropriate |

### Intensity Over Time: Worked Example

A `Threat` pheromone deposited with `base_intensity = 1.0`, `half_life = 2h`, and varying
confirmation counts:

| Time | 0 confirmations | 1 confirmation (τ=3h) | 3 confirmations (τ=5h) |
|------|-----------------|-----------------------|------------------------|
| T+0h | 1.000 | 1.000 | 1.000 |
| T+1h | 0.707 | 0.794 | 0.871 |
| T+2h | 0.500 | 0.630 | 0.758 |
| T+3h | 0.354 | 0.500 | 0.660 |
| T+4h | 0.250 | 0.397 | 0.574 |
| T+6h | 0.125 | 0.250 | 0.435 |
| T+8h | 0.063 | 0.157 | 0.330 |
| T+12h | 0.016 | 0.063 | 0.189 |
| T+24h | 0.000 | 0.004 | 0.036 |

With 3 confirmations, a Threat pheromone that would normally be negligible at T+12h still has
19% intensity — enough to influence agent behavior for a full working day.

---

## Confirmation Mechanics

Confirmation is the mechanism by which multiple agents reinforce a pheromone signal. When Agent
B independently deposits a pheromone of the same `kind` and `scope` as Agent A's existing
pheromone, the existing pheromone's `confirmations` count increments and its effective
half-life extends.

### Confirmation Rules

1. **Independence**: The confirming agent must not be the original depositor. Self-confirmation
   is not counted.
2. **Same kind and scope**: The confirming deposit must match both the `PheromoneKind` and the
   `PheromoneScope` of the existing pheromone.
3. **Proximity**: The confirming deposit must be "near" the original in the Substrate's address
   space. What "near" means depends on the Substrate implementation — for code, it might mean
   the same file or module; for knowledge, it might mean semantic similarity above a threshold.
4. **Temporal window**: The confirming deposit must occur while the original pheromone's
   intensity is above the sensing threshold (default: 0.01). Confirming a pheromone that has
   already fully decayed creates a new pheromone rather than extending an existing one.
5. **Anti-spoofing**: Confirmation is weighted by the confirming agent's reputation score.
   Agents with high reputation contribute full confirmation weight; agents with low reputation
   contribute partial weight. This prevents Sybil attacks where an adversary creates many
   low-reputation agents to artificially extend a pheromone's lifetime.

### Confirmation as Quorum Sensing

The confirmation mechanism is directly analogous to quorum sensing in bacteria [Nealson, Platt
& Hastings, "Cellular Control of the Synthesis and Activity of the Bacterial Luminescent
System." *J. Bacteriology*, 104(1):313-322, 1970]. In quorum sensing:

- Individual bacteria release autoinducer molecules into the environment.
- When the local concentration exceeds a threshold, coordinated behavior is triggered.
- The threshold ensures that the behavior only activates when enough bacteria are present to
  make the behavior effective.

In Roko's pheromone system:

- Individual agents deposit pheromone Engrams into the Substrate.
- When the confirmation count exceeds a threshold, the pheromone's effective half-life extends
  significantly, making it a durable coordination signal.
- The threshold ensures that only collectively validated signals persist long enough to
  influence the broader collective.

This creates a natural filter: noise (false signals from individual agents) decays quickly,
while genuine signals (confirmed by multiple independent agents) persist.

### Anti-Spoofing via Reputation Weighting

The confirmation weighting formula:

```rust
/// Compute the effective confirmation count, weighted by confirmer reputation.
///
/// Each confirmation is weighted by the confirming agent's reputation score
/// (range [0.0, 1.0]). This prevents Sybil attacks where many low-reputation
/// agents artificially extend a pheromone's lifetime.
///
/// A single confirmation from an Elite agent (reputation > 0.85) counts as
/// approximately one full confirmation. A confirmation from a Probation agent
/// (reputation < 0.50) counts as approximately half a confirmation.
pub fn effective_confirmations(
    confirmations: &[(AgentId, f64)],  // (confirmer, reputation)
) -> f64 {
    confirmations.iter()
        .map(|(_, rep)| rep.clamp(0.0, 1.0))
        .sum()
}
```

The effective half-life then uses `effective_confirmations` instead of raw count:

```
τ_effective = τ_base × (1 + effective_confirmations × 0.5)
```

---

## The Pheromone Field

The aggregate of all active pheromones in a scope constitutes a **pheromone field** — a
multi-dimensional signal landscape that agents navigate. The field has the following properties:

### Field Operations

Four fundamental operations on the pheromone field, adapted from the clade ecology
specification [Hölldobler, B. & Wilson, E.O. *The Superorganism*. W.W. Norton, 2008]:

| Operation | Description | Synapse Trait |
|-----------|-------------|---------------|
| **Deposition** | Agent deposits a new pheromone Engram | `Substrate::store()` |
| **Sensing** | Agent queries for pheromones above a threshold | `Substrate::query()` + `Scorer::score()` |
| **Reinforcement** | Agent confirms an existing pheromone | Specialized `Substrate::store()` with proximity matching |
| **Evaporation** | Pheromone intensity decreases over time | Computed on-read via `pheromone_decay()` |

### Field Composition

When multiple pheromones of different kinds coexist at the same scope, their combined effect is
computed by the `Composer` trait. The default composition strategy is weighted addition:

```
composite_signal = Σ (pheromone_i.intensity × kind_weight_i × relevance_i)
```

Where:
- `kind_weight_i` is the weight assigned to each `PheromoneKind` (configurable per agent role)
- `relevance_i` is the contextual relevance scored by the `Scorer`

For example, a coding agent might weight `Threat` pheromones heavily (bugs need fixing) and
`Pattern` pheromones moderately (code quality is important but not urgent), while a research
agent might weight `Wisdom` pheromones heavily and `Threat` pheromones lightly.

### Field Visualization

The pheromone field can be visualized as a heatmap over the Substrate's address space. In
Roko's text-mode dashboard (`roko dashboard`), the pheromone field is rendered as:

```
Pheromone Field (Mesh scope: collective-alpha)
───────────────────────────────────────────────
Threat   ██████░░░░░░░░░░  2 active (0.73 max)
Opportunity ███████████░░░  3 active (0.91 max)
Wisdom   ████████████████  5 active (0.95 max)
Pattern  █████████░░░░░░░  1 active (0.58 max)
Anomaly  ░░░░░░░░░░░░░░░░  0 active
Consensus ██████████████░  1 active (0.88 max)
```

---

## Pheromone-Enriched Context Assembly

Digital pheromones are integrated into Roko's context assembly pipeline at L2 Scaffold. When
the `Composer` assembles a prompt for an agent, it includes a summary of the ambient pheromone
field:

### Context Enrichment Flow

```
Agent receives task assignment
    ↓
Composer queries Substrate for ambient pheromones
    ↓
Scorer rates each pheromone by intensity × relevance
    ↓
Router selects top-K pheromones (default K=5)
    ↓
Composer formats pheromone summary into system prompt:
    "## Ambient Signals
     - [THREAT 0.73] Regression in gate pipeline (scorer NaN handling)
       — deposited 45min ago by agent-7, confirmed by agent-12
     - [OPPORTUNITY 0.91] New API endpoint ready for integration
       — deposited 2h ago by agent-3, 2 confirmations
     - [WISDOM 0.95] NaN scores should be clamped to 0.0 before comparison
       — deposited 6h ago by agent-7, confirmed by agents 8, 12, 15"
    ↓
Agent processes task with awareness of ambient signals
```

This enrichment happens automatically for every agent dispatch. The agent does not need to
explicitly request pheromone information — it is part of the environment, just as chemical
pheromones are part of the air that biological agents breathe.

### Dynamic Context Assembly as Stigmergic Behavior

The context assembly process itself is stigmergic: the agent's context (which information it
receives) is determined by the pheromone field, which was shaped by previous agents' actions.
An agent that deposits a high-intensity `Threat` pheromone changes the context that all
subsequent agents in the same scope will receive, steering collective attention toward the
threat without any direct communication.

This is the digital equivalent of an ant depositing alarm pheromone: the ant doesn't send a
message to specific other ants; it modifies the chemical environment, and all ants that pass
through the area are influenced.

---

## Pheromone Lifecycle

The complete lifecycle of a digital pheromone in Roko:

### 1. Creation

An agent detects a condition worth signaling (a bug, an opportunity, an insight) and deposits
a pheromone Engram:

```rust
let pheromone = Pheromone {
    kind: PheromoneKind::Threat,
    intensity: 1.0,  // Fresh deposit, full intensity
    decay_rate: Duration::from_secs(2 * 3600),  // 2-hour half-life
    source: self.agent_id.clone(),
    scope: PheromoneScope::Mesh(self.collective_id.clone()),
};

substrate.store(pheromone.into_engram())?;
```

### 2. Propagation

Based on the `scope`, the pheromone propagates:

- `Local`: Stays in the agent's own NeuroStore. No propagation.
- `Mesh`: Propagated to all agents in the Collective via Agent Mesh (WebSocket relay or Iroh
  P2P, see `06-agent-mesh-sync.md`).
- `Global`: Published to the Korai chain for all agents worldwide.

### 3. Sensing

Other agents query the Substrate and encounter the pheromone:

```rust
let threats = substrate.query(PheromoneFilter {
    kind: Some(PheromoneKind::Threat),
    scope: Some(PheromoneScope::Mesh(my_collective.clone())),
    min_intensity: 0.1,  // Sensing threshold
})?;
```

### 4. Scoring

The `Scorer` evaluates each sensed pheromone for relevance to the current agent's task:

```rust
for threat in &threats {
    let score = scorer.score(threat)?;
    // score combines intensity (how strong) with relevance (how related to current task)
}
```

### 5. Response

The `Router` selects the highest-scored pheromone, and the `Policy` determines the agent's
response:

- **Threat**: Prioritize fixing the issue (if the agent has the capability)
- **Opportunity**: Consider pursuing the opportunity (if it aligns with the agent's task)
- **Wisdom**: Incorporate the insight into the current work

### 6. Confirmation or Contradiction

After acting, the agent may deposit its own pheromone:

- **Confirm**: "I independently verified this threat — it's real" → confirmation increments,
  effective half-life extends
- **Contradict**: "I investigated this threat and it's a false positive" → deposit a counter-
  pheromone (same kind, negative tag) that weakens the original signal
- **Extend**: "I found additional context about this threat" → deposit a new pheromone with
  the original as a parent (lineage tracking via `parents` field in Engram)

### 7. Decay and Garbage Collection

Over time, the pheromone's intensity decays toward zero. When it drops below the sensing
threshold (default 0.01), it becomes invisible to other agents. When it drops below the GC
threshold (default 0.001), it is eligible for garbage collection by the `Substrate`
implementation.

---

## Comparison with Other Coordination Mechanisms

| Mechanism | Latency | Scalability | Robustness | Information Persistence | Roko Usage |
|-----------|---------|-------------|------------|------------------------|------------|
| Direct messaging | Low | O(N²) | Fragile (sender/receiver must be online) | None (fire-and-forget) | Not used for coordination |
| Blackboard | Medium | O(N) | Moderate (single point of failure) | Until cleared | Inspiration for Substrate |
| Publish-subscribe | Low | O(N × topics) | Moderate (broker dependency) | Until consumed | Transport layer (Agent Mesh) |
| **Digital pheromones** | Low–Medium | **O(N × M)** | **High** (decentralized, decay handles staleness) | **Configurable decay** | **Primary coordination mechanism** |
| Consensus protocols | High | O(N²) or O(N log N) | High (Byzantine fault tolerance) | Permanent (on-chain) | Global scope only (Korai) |

Digital pheromones occupy a unique design point: they provide the robustness and scalability of
decentralized coordination with the information persistence of a shared store, while the decay
mechanism automatically handles staleness — a problem that plagues blackboard and pub-sub
systems.

---

## References

- [Bonabeau, Dorigo & Theraulaz 1999] *Swarm Intelligence*, Oxford University Press
- [Deneubourg et al. 1990] Argentine ant self-organization, *J. Insect Behavior*
- [Dorigo, Maniezzo & Colorni 1996] Ant Colony Optimization, *IEEE SMC-B*
- [Grassé 1959] Termite mound stigmergy, *Insectes Sociaux*
- [Grossman & Stiglitz 1980] Informationally Efficient Markets, *AER*
- [Hölldobler & Wilson 2008] *The Superorganism*, W.W. Norton
- [Nealson, Platt & Hastings 1970] Quorum sensing, *J. Bacteriology*
- [Parunak 1997] Engineering from natural MAS, *Ann. Oper. Res.*
- [Parunak, Brueckner & Sauter 2005] Digital pheromones, *E4MAS*

---

## Related Sub-Docs

- `04-pheromone-kinds.md` — Full taxonomy of `PheromoneKind`
- `05-pheromone-scope.md` — Local, Mesh, and Global scoping
- `06-agent-mesh-sync.md` — Transport layer for pheromone propagation
- `10-exponential-flywheel.md` — How pheromones enable compounding intelligence
