# Digital Pheromones: Typed Engrams with Decay Profiles

> **Layer**: L0 Runtime (Substrate persistence and decay timers), L1 Framework (Bus
> publication and type system), L2 Scaffold (context assembly enrichment)
>
> **Synapse traits**: `Substrate` (store pheromone Engrams), `Bus` (announce Pulses),
> `Scorer` (rate pheromone intensity and relevance), `Router` (select highest-priority
> Pulse), `Policy` (react to pheromone streams)
>
> **Prerequisites**: `00-stigmergy-theory.md` (stigmergy theory),
> `01-stigmergy-beyond-termites.md` (generalized stigmergy)

> **See also**: `../../tmp/refinements/09-phase-2-implications.md`,
> `../00-architecture/01-naming-and-glossary.md`


> **Implementation**: Specified

---

## What Are Digital Pheromones?

Digital pheromones are software analogs of the chemical pheromones used by social insects for
indirect coordination. In Roko, a digital pheromone is a typed Engram that lives in a shared
Substrate and is announced as a Pulse on the Bus. The same coordination fact therefore has two
faces in the two-fabric model: durable storage and ephemeral announcement.

The concept was formalized by Parunak, Brueckner & Sauter (2005), who identified the key
properties that make biological pheromones effective as coordination mechanisms and showed how
to replicate these properties in software systems [Parunak, H.V.D., Brueckner, S.A. &
Sauter, J.A. "Digital Pheromones for Coordination of Unmanned Vehicles." *Environments for
Multi-Agent Systems*, LNCS 3374:246-263, Springer, 2005].

Roko extends Parunak's framework with three additions:

1. **Typed pheromones**: Each pheromone has a `PheromoneKind` that determines its semantic
   meaning and default decay profile (see `04-pheromone-kinds.md`).
2. **Scoped propagation**: Pheromones propagate through one of three scopes — Local, Mesh,
   or Global — controlling their audience and persistence. Mesh scope is expressed as Bus
   topics such as `mesh.pheromone.deposited` (see `05-pheromone-scope.md`).
3. **Confirmation reinforcement**: Multiple independent deposits of the same pheromone type
   extend its effective half-life, implementing a quorum-sensing mechanism analogous to
   bacterial autoinducer accumulation [Nealson, Platt & Hastings, *J. Bacteriology*, 1970].

---

## Engram-First Pheromone View

The primary durable object is still an `Engram`. The `Pheromone` struct below is an implementation-facing view over that Engram's tags and body when the system wants typed ergonomics. Storage stays Engram-first; live notification stays Pulse-first.

```rust
/// A digital pheromone — a typed Engram carrying coordination information.
///
/// Pheromones are the primary mechanism for indirect coordination between
/// agents. They are persisted as Engrams in a shared Substrate, announced
/// as Pulses on the Bus, decay over time, and influence the behavior of
/// agents that sense them.
///
/// # Stigmergic Properties
///
/// - **Deposition**: Created by an agent via `Substrate::store()`
/// - **Announcement**: Published as a Pulse on the Bus, often on
///   `mesh.pheromone.deposited`
/// - **Diffusion**: Propagates through MeshBus based on `scope`
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
    /// - `Mesh(CollectiveId)`: Visible to all agents via MeshBus topics
    /// - `Global`: Visible to all agents on the Korai chain
    pub scope: PheromoneScope,
}
```

### Relationship to the Engram Type

A `Pheromone` is a specialized view of the `Engram` type — Roko's universal unit of cognition.
The durable record is the Engram; the typed pheromone view is how coordination code interprets
that record, while `mesh.pheromone.deposited` on `MeshBus` is the live announcement that a new
deposit landed. The relationship is:

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
| `bus_topic` | `"mesh.pheromone.deposited"` | Topic used when announcing the deposit on the Bus |
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

Four fundamental operations on the pheromone field, adapted from the legacy collective-ecology
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
field and any fresh Pulses on the Bus that have not yet been folded into the Substrate:

### Context Enrichment Flow

```
Agent receives task assignment
    ↓
Composer queries Substrate for ambient pheromones
    ↓
Composer subscribes to `mesh.pheromone.deposited` Pulses for freshness
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
pheromones are part of the air that biological agents breathe. In two-fabric terms, the
Substrate holds the durable record and the Bus keeps the prompt current.

### Dynamic Context Assembly as Stigmergic Behavior

The context assembly process itself is stigmergic: the agent's context (which information it
receives) is determined by the pheromone field, which was shaped by previous agents' actions
and announced on the Bus. An agent that deposits a high-intensity `Threat` pheromone changes
the context that all subsequent agents in the same scope will receive, steering collective
attention toward the threat without any direct communication.

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

Based on the `scope`, the pheromone is announced on the Bus:

- `Local`: Stays in the agent's own NeuroStore. No propagation.
- `Mesh`: Published as `mesh.pheromone.deposited` and routed by `MeshBus` to the Collective while the durable Engram remains queryable in shared Substrate.
- `Global`: Replicated to the Korai chain for all agents worldwide, with durable state on `ChainSubstrate` and chain-side announcements available via `ChainBus`.

### 3. Sensing

Other agents query the Substrate and/or subscribe to the Bus and encounter the pheromone:

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
implementation. The Bus announcement remains ephemeral; only the Engram persists.

---

## Pheromone Interference and Crosstalk

In biological ant colonies, multiple pheromone types can interfere with each other — alarm pheromones disrupt trail-following, heterospecific trail pheromones create cross-species confusion, and environmental chemicals mask signals [Hölldobler, B. & Wilson, E.O. *The Superorganism*. W.W. Norton, 2008]. Digital pheromone systems face analogous interference challenges that must be explicitly modeled and mitigated.

### Types of Pheromone Interference

| Interference Type | Biological Analog | Digital Manifestation | Impact |
|------------------|------------------|----------------------|--------|
| **Intra-kind saturation** | Trail pheromone over-concentration causes confusion in ants | Too many Threat pheromones at once make prioritization impossible | Agents cannot distinguish high-priority from low-priority signals |
| **Cross-kind masking** | Alarm pheromone overrides foraging pheromone | High-intensity Threat suppresses all Opportunity sensing | Agents miss valuable opportunities during threat response |
| **Temporal aliasing** | Old pheromone trails mislead ants to depleted food sources | Stale confirmations extend signals past their useful life | Agents act on outdated information |
| **Spatial flooding** | Nest-vicinity pheromone saturation in dense colonies | Popular code modules accumulate excessive pheromone density | Hot modules become navigation hazards rather than coordination aids |
| **Sybil amplification** | N/A (biological colonies have physical identity) | Multiple low-reputation agents confirm a false signal | False signals persist with artificially extended half-lives |

### The Interference Model

Pheromone interference is modeled as a signal-to-interference-plus-noise ratio (SINR), adapted from wireless communications [Tse, D. & Viswanath, P. *Fundamentals of Wireless Communication*. Cambridge University Press, 2005]:

```
SINR_k = I_target_k / (Σ_{j≠k} α_{jk} × I_j + N₀)
```

Where:
- `I_target_k` = intensity of the target pheromone of kind k
- `I_j` = intensity of interfering pheromones of kind j
- `α_{jk}` = cross-kind interference coefficient (how much kind j interferes with sensing kind k)
- `N₀` = background noise floor (environmental noise, sensing imprecision)

```rust
/// Cross-kind interference coefficients.
///
/// Models how pheromone kinds interfere with each other's sensing.
/// A coefficient of 0.0 means no interference; 1.0 means full masking.
///
/// The default matrix encodes biological precedent:
/// - Threat has high cross-kind interference (alarm overrides foraging)
/// - Wisdom has low cross-kind interference (knowledge persists through noise)
/// - Alpha has near-zero interference (ephemeral, doesn't accumulate)
///
/// # References
/// Wilson, E.O. "The Insect Societies." 1971 — pheromone type interactions.
/// Tse & Viswanath 2005 — SINR framework from wireless communications.
pub struct InterferenceMatrix {
    /// NxN matrix where entry [j][k] = how much kind j interferes with sensing kind k.
    /// N = number of PheromoneKind variants (7 universal + custom).
    /// Range per entry: [0.0, 1.0].
    pub coefficients: Vec<Vec<f64>>,
}

impl InterferenceMatrix {
    /// Default interference matrix for the 7 universal kinds.
    ///
    /// Key design choices:
    /// - Threat → Opportunity: 0.6 (alarm suppresses foraging, per Wilson 1971)
    /// - Threat → Wisdom: 0.1 (knowledge is resistant to alarm)
    /// - Opportunity → Threat: 0.0 (opportunities don't mask threats)
    /// - Consensus → all: 0.05 (consensus is highly resistant to interference)
    pub fn default_universal() -> Self {
        //         Thr  Opp  Wis  Alp  Pat  Ano  Con
        let m = vec![
            vec![0.0, 0.6, 0.1, 0.3, 0.2, 0.1, 0.05], // Threat interferes with...
            vec![0.0, 0.0, 0.0, 0.1, 0.05, 0.0, 0.0],  // Opportunity
            vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],   // Wisdom (low interference)
            vec![0.1, 0.1, 0.0, 0.0, 0.05, 0.0, 0.0],  // Alpha
            vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],   // Pattern
            vec![0.2, 0.1, 0.0, 0.1, 0.1, 0.0, 0.0],   // Anomaly
            vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],   // Consensus
        ];
        Self { coefficients: m }
    }
}

/// Compute the effective sensed intensity of a target pheromone,
/// accounting for cross-kind interference from other active pheromones.
///
/// Returns the SINR-adjusted intensity. If the SINR falls below
/// `min_sinr`, returns 0.0 (signal is undetectable in the noise).
///
/// # Parameters
/// - `target_kind_idx`: Index of the target pheromone kind
/// - `target_intensity`: Raw intensity of the target pheromone
/// - `active_intensities`: Aggregate intensity per kind (vector of length N)
/// - `matrix`: Cross-kind interference coefficients
/// - `noise_floor`: Background noise level. Default: 0.01.
/// - `min_sinr`: Minimum SINR for detection. Default: 1.0 (0 dB).
pub fn sinr_adjusted_intensity(
    target_kind_idx: usize,
    target_intensity: f64,
    active_intensities: &[f64],
    matrix: &InterferenceMatrix,
    noise_floor: f64,
    min_sinr: f64,
) -> f64 {
    let interference: f64 = active_intensities.iter()
        .enumerate()
        .filter(|&(j, _)| j != target_kind_idx)
        .map(|(j, &intensity)| {
            matrix.coefficients[j][target_kind_idx] * intensity
        })
        .sum();

    let sinr = target_intensity / (interference + noise_floor);
    if sinr < min_sinr {
        0.0
    } else {
        target_intensity * (sinr / (1.0 + sinr)) // graceful degradation
    }
}

#[cfg(test)]
mod interference_tests {
    use super::*;

    #[test]
    fn threat_suppresses_opportunity() {
        let matrix = InterferenceMatrix::default_universal();
        let active = vec![0.9, 0.8, 0.0, 0.0, 0.0, 0.0, 0.0]; // Threat=0.9, Opp=0.8
        let opp_effective = sinr_adjusted_intensity(
            1, 0.8, &active, &matrix, 0.01, 1.0,
        );
        // Threat interferes with Opportunity at coefficient 0.6
        // Interference = 0.9 * 0.6 = 0.54; SINR = 0.8 / (0.54 + 0.01) ≈ 1.45
        assert!(opp_effective < 0.8, "opportunity suppressed by threat");
        assert!(opp_effective > 0.0, "opportunity not fully masked");
    }

    #[test]
    fn consensus_resists_interference() {
        let matrix = InterferenceMatrix::default_universal();
        let active = vec![0.9, 0.9, 0.9, 0.9, 0.9, 0.9, 0.9]; // all maxed
        let con_effective = sinr_adjusted_intensity(
            6, 0.9, &active, &matrix, 0.01, 1.0,
        );
        // All interference coefficients into Consensus are ≤ 0.05
        // Total interference ≤ 6 * 0.9 * 0.05 = 0.27; SINR = 0.9 / 0.28 ≈ 3.2
        assert!(con_effective > 0.5, "consensus resists interference");
    }

    #[test]
    fn zero_interference_passes_through() {
        let matrix = InterferenceMatrix::default_universal();
        let active = vec![0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0]; // only Pattern
        let pat_effective = sinr_adjusted_intensity(
            4, 0.5, &active, &matrix, 0.01, 1.0,
        );
        // No other kinds active → interference = 0
        // SINR = 0.5 / 0.01 = 50 → effective ≈ 0.5 * (50/51) ≈ 0.49
        assert!((pat_effective - 0.5).abs() < 0.02);
    }
}
```

### Anti-Saturation Mechanisms

When a pheromone field becomes saturated (too many active signals), the system applies anti-saturation countermeasures:

```rust
/// Anti-saturation configuration for the pheromone field.
///
/// Prevents field saturation from degrading coordination quality.
/// When the total active pheromone count exceeds thresholds, the
/// system applies progressively stronger countermeasures.
pub struct AntiSaturationConfig {
    /// Soft threshold: above this count, low-intensity pheromones
    /// decay 2× faster. Default: 500. Range: [100, 10000].
    pub soft_threshold: usize,

    /// Hard threshold: above this count, only pheromones with
    /// intensity > `hard_min_intensity` are retained.
    /// Default: 2000. Range: [500, 50000].
    pub hard_threshold: usize,

    /// Minimum intensity to survive hard threshold GC.
    /// Default: 0.1. Range: [0.01, 0.5].
    pub hard_min_intensity: f64,

    /// Maximum pheromones per kind per scope. Prevents any single
    /// kind from monopolizing the field.
    /// Default: 100. Range: [10, 1000].
    pub max_per_kind_per_scope: usize,
}

impl Default for AntiSaturationConfig {
    fn default() -> Self {
        Self {
            soft_threshold: 500,
            hard_threshold: 2000,
            hard_min_intensity: 0.1,
            max_per_kind_per_scope: 100,
        }
    }
}
```

### Interference Mitigation Strategies

| Strategy | Mechanism | When to Use |
|----------|-----------|-------------|
| **Kind isolation** | Pheromone kinds with zero mutual interference operate independently | Default for orthogonal signal types (Wisdom vs Alpha) |
| **Priority preemption** | High-priority kinds (Threat) temporarily mask lower-priority kinds | During active threat response |
| **Temporal separation** | Fast-decaying kinds (Alpha, 1h) naturally clear before slow kinds (Consensus, 48h) peak | Automatic via decay profile design |
| **Scope partitioning** | Signals at different scopes don't interfere (Local doesn't mask Mesh) | Inherent in the three-scope architecture |
| **Adaptive thresholds** | Raise sensing threshold when field is saturated | Via `AntiSaturationConfig` |

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

## Cross-References

- `04-pheromone-kinds.md` — Full taxonomy of `PheromoneKind`
- `05-pheromone-scope.md` — Local, Mesh, and Global scoping
- `06-agent-mesh-sync.md` — Transport layer for pheromone propagation
- `10-exponential-flywheel.md` — How pheromones enable compounding intelligence
- `../../tmp/refinements/09-phase-2-implications.md` — Phase 2+ Bus/Substrate framing for coordination
- `../00-architecture/01-naming-and-glossary.md` — Glossary for Bus, Pulse, MeshBus, and MeshSubstrate
