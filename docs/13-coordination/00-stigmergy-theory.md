# Stigmergy Theory: Indirect Coordination Through Environmental Modification

> **Layer**: L4 Orchestration (coordination mechanisms), with cross-cuts into L0 Runtime (event
> persistence) and L1 Framework (transport)
>
> **Synapse traits**: `Substrate` (store pheromone Engrams), `Scorer` (rate pheromone intensity),
> `Policy` (observe pheromone streams, emit reactive Engrams)
>
> **Prerequisites**: `13-coordination/INDEX.md` (overview), familiarity with the Engram struct
> (Roko's content-addressed, scored, decaying unit of cognition)


> **Implementation**: Specified

---

## What Is Stigmergy?

Stigmergy is a mechanism of indirect coordination between agents, where the trace left in the
environment by one agent's action stimulates the performance of a subsequent action by the same
or a different agent. The term was coined by French entomologist Pierre-Paul Grassé in 1959 to
describe how termites coordinate the construction of elaborate mound structures without any
centralized plan, blueprint, or direct communication between individuals [Grassé, P.-P. "La
Reconstruction du Nid et les Coordinations Inter-Individuelles chez Bellicositermes Natalensis
et Cubitermes sp." *Insectes Sociaux*, 6(1):41-80, 1959].

The core insight is deceptively simple: **agents do not need to communicate with each other
directly. They only need to read from and write to a shared environment.** The environment
itself becomes the coordination medium.

### The Termite Example

Grassé observed that termites building a mound follow no central plan. Instead:

1. A termite picks up a mud pellet and deposits it at a location.
2. The deposited pellet contains a chemical pheromone that attracts other termites.
3. Attracted termites deposit their own pellets nearby, reinforcing the pheromone signal.
4. The growing pile of pellets, through its physical shape and pheromone concentration,
   guides further construction — arches form, chambers emerge, ventilation shafts develop.
5. No termite has a model of the whole structure. Each termite responds to local stimuli.

The structure that emerges is far more complex than any individual termite could plan. This is
the hallmark of stigmergy: **simple local rules + persistent environmental modification =
complex global behavior**.

### Formal Definition

Stigmergy requires three conditions [Theraulaz, G. & Bonabeau, E. "A Brief History of
Stigmergy." *Artificial Life*, 5(2):97-116, 1999]:

| Condition | Description | Roko Equivalent |
|-----------|-------------|-----------------|
| **Shared environment** | All agents can read from and write to a common medium | NeuroStore (local Substrate), Agent Mesh (peer network), Korai chain (global ledger) |
| **Persistent modifications** | Agent actions leave traces that outlast the agent's presence | Engrams with configurable decay rates (2h–∞) |
| **Stimulus-response coupling** | Traces in the environment trigger specific behaviors in agents that encounter them | `Policy` trait implementations that react to scored Engrams |

When all three conditions are met, coordination emerges without any agent needing a global view,
without any central coordinator, and without agents needing to know about each other's existence.

---

## Two Forms of Stigmergy

Grassé and subsequent researchers identified two distinct forms of stigmergy, both of which
appear in Roko's architecture [Holland, O. & Melhuish, C. "Stigmergy, Self-Organization, and
Sorting in Collective Robotics." *Artificial Life*, 5(2):173-202, 1999]:

### Sematectonic Stigmergy (Structure-Based)

In sematectonic stigmergy, the **physical structure** created by agents guides subsequent work.
The product itself is the signal. Grassé observed this in termite mound construction: the shape
of a partially built arch tells the next termite where to place its mud pellet. No chemical
signal is needed — the geometry of the structure is the coordination mechanism.

**In Roko**: The codebase itself is sematectonic stigmergy. When a coding agent writes a
function, the function's signature, its location in the module hierarchy, its documentation, and
its test coverage all constitute structural signals that guide subsequent agents. A well-typed
function with clear documentation "invites" usage; a function with poor error handling "invites"
improvement. The code structure guides agent behavior without any explicit communication.

Examples of sematectonic stigmergy in software development:

| Structural Feature | Signal Conveyed | Agent Response |
|-------------------|----------------|----------------|
| Empty test file | "Tests needed here" | Testing agent writes tests |
| `TODO` comment | "Incomplete implementation" | Coding agent completes it |
| Unused import | "Stale code" | Refactoring agent cleans up |
| Well-documented API | "Ready for integration" | Integration agent uses it |
| Failing CI badge | "Broken build" | Debugging agent investigates |
| Missing error handling | "Fragile code path" | Hardening agent adds handling |

### Marker-Based Stigmergy (Signal-Based)

In marker-based stigmergy, agents deposit **explicit signals** (markers or pheromones) in the
environment. These signals carry information beyond the physical structure — they encode
urgency, type, confidence, and decay over time. Ant trail pheromones are the canonical example:
a foraging ant that finds food deposits a trail pheromone on its return path, and the
pheromone's concentration encodes the quality and proximity of the food source.

**In Roko**: Digital pheromones — typed Engrams with explicit `kind`, `intensity`, `scope`, and
exponential decay profiles — implement marker-based stigmergy. When an agent detects a threat
(e.g., a failing test suite, an anomalous metric, a security vulnerability), it deposits a
`Threat` pheromone that other agents can sense and react to. The pheromone decays over time
(threat signals have a 2-hour half-life), so stale threats don't permanently distort behavior.

Key properties of marker-based stigmergy in Roko:

1. **Typed signals**: Different pheromone kinds trigger different agent responses (see
   `04-pheromone-kinds.md`).
2. **Exponential decay**: Signals lose intensity over time, preventing stale information from
   accumulating (see `03-digital-pheromones.md`).
3. **Confirmation reinforcement**: When multiple agents deposit the same pheromone type at the
   same location, the effective half-life extends, making well-confirmed signals persist longer.
4. **Scope control**: Pheromones can be local (one Substrate), mesh-wide (one Collective), or
   global (public Korai chain) — see `05-pheromone-scope.md`.

---

## Why Stigmergy, Not Direct Communication?

Direct agent-to-agent communication (message passing, shared blackboards, leader election) has
been the dominant paradigm in multi-agent systems since the 1980s [Hewitt, C. "Viewing Control
Structures as Patterns of Passing Messages." *Artificial Intelligence*, 8(3):323-364, 1977].
Roko uses stigmergy instead for several fundamental reasons, each grounded in empirical research.

### Scalability

Direct communication scales as O(N²) for N agents — every agent must potentially communicate
with every other agent. Stigmergy scales as O(N × M) where M is the number of distinct signal
types (pheromone kinds), which is bounded and small. In Roko, M = 7 universal kinds + domain
extensions, so coordination cost grows linearly with the number of agents.

| Agents (N) | Direct Comm (O(N²)) | Stigmergy (O(N × M), M=10) |
|-----------|--------------------|-----------------------------|
| 5 | 25 channels | 50 read/writes |
| 50 | 2,500 channels | 500 read/writes |
| 500 | 250,000 channels | 5,000 read/writes |
| 5,000 | 25,000,000 channels | 50,000 read/writes |

> "Stigmergy provides a clear separation between the coordination mechanism and the individual
> agents, allowing the system to scale without modifying agent behavior." — [Parunak, H.V.D.
> "Go to the Ant: Engineering Principles from Natural Multi-Agent Systems." *Annals of
> Operations Research*, 75:69-101, 1997]

### Robustness

In a direct communication system, the failure of a key node (coordinator, leader, message
broker) can paralyze the entire collective. Stigmergy is inherently robust because the
coordination state is distributed across the environment, not concentrated in any single agent.
If an agent fails:

- Its previously deposited pheromones persist and continue to guide other agents.
- No other agent needs to be notified of the failure.
- The pheromone field naturally adapts as the failed agent's signals decay.
- New agents joining the collective can immediately sense the current state.

This property is formalized in the concept of **graceful degradation**: the collective's
performance degrades smoothly with agent loss, rather than failing catastrophically
[Bonabeau, E., Dorigo, M. & Theraulaz, G. *Swarm Intelligence: From Natural to Artificial
Systems*. Oxford University Press, 1999].

### Asynchrony

Stigmergy is inherently asynchronous. The depositing agent and the sensing agent do not need
to be active at the same time. A pheromone deposited at time T can influence an agent at time
T + Δt, where Δt can be seconds, minutes, or hours. This is essential for Roko's multi-speed
cognitive architecture:

| Cognitive Speed | Tick Duration | Stigmergy Role |
|----------------|---------------|----------------|
| T0 (System-1, fast) | ~15 seconds | Sense ambient pheromones, react to high-intensity signals |
| T1 (System-2, deliberate) | ~60 seconds | Analyze pheromone patterns, deposit new observations |
| T2 (Reflective) | ~5 minutes | Consolidate pheromone history, emit Wisdom Engrams |

An agent running at T0 speed can sense pheromones deposited by a T2 agent hours earlier. The
decoupling of production and consumption in time is a fundamental advantage over synchronous
communication protocols.

### Minimal Agent Complexity

Each agent only needs to implement two operations:

1. **Deposit**: Write an Engram to the Substrate with a pheromone kind and intensity.
2. **Sense**: Query the Substrate for nearby Engrams above a threshold intensity.

The agent does not need to know how many other agents exist, what strategies they follow, or
whether they are online. This dramatically reduces the complexity of individual agents while
enabling sophisticated collective behavior.

In terms of the Synapse Architecture, deposit maps to the `Substrate::store()` trait method,
and sense maps to `Substrate::query()` followed by `Scorer::score()` to rank the sensed
pheromones by relevance. The `Policy` trait then observes the scored pheromone stream and
decides whether to emit a reactive Engram (closing the stigmergic loop).

---

## Stigmergy in Computer Science

While Grassé studied biological systems, the principles of stigmergy have been applied
extensively in computer science, particularly in swarm intelligence and distributed optimization.

### Ant Colony Optimization (ACO)

Dorigo's Ant Colony Optimization (ACO) is the most prominent computational application of
stigmergy [Dorigo, M., Maniezzo, V. & Colorni, A. "Ant System: Optimization by a Colony of
Cooperating Agents." *IEEE Transactions on Systems, Man, and Cybernetics B*, 26(1):29-41,
1996]. In ACO:

1. Artificial ants traverse a graph (e.g., the Travelling Salesman Problem).
2. Each ant deposits pheromone on the edges it traverses, proportional to the quality of
   its solution.
3. Subsequent ants probabilistically prefer edges with higher pheromone concentration.
4. Pheromone decays over time (evaporation), preventing premature convergence.
5. The collective converges on high-quality solutions without any ant having a global view.

ACO has been applied to routing, scheduling, assignment problems, and protein folding. Its
success demonstrates that stigmergy is not just a biological curiosity — it is a **general
coordination principle** applicable to any domain where agents must collectively optimize in a
large search space.

### Digital Pheromone Systems

Parunak extended stigmergy to general software agent systems with the concept of digital
pheromones — software analogs of chemical pheromones that are deposited, sensed, and decay in
a shared computational environment [Parunak, H.V.D., Brueckner, S.A. & Sauter, J.A. "Digital
Pheromones for Coordination of Unmanned Vehicles." *Environments for Multi-Agent Systems*,
LNCS 3374:246-263, Springer, 2005].

Key properties identified by Parunak:

| Property | Biological | Digital (Roko) |
|----------|-----------|----------------|
| Deposition | Chemical secretion | `Substrate::store(Engram { kind: PheromoneKind::Threat, ... })` |
| Diffusion | Brownian motion through medium | Mesh gossip propagation (see `06-agent-mesh-sync.md`) |
| Evaporation | Chemical degradation | Exponential decay: `intensity(t) = base × e^(-0.693 × elapsed / τ)` |
| Sensing | Chemoreceptors | `Substrate::query()` with pheromone kind filter |
| Reinforcement | Multiple depositions | Confirmation count extends effective half-life |

### Swarm Intelligence in Distributed Systems

Beyond ACO, stigmergy has been applied to:

- **Load balancing**: Agents leave "workload" pheromones; overloaded nodes repel new tasks,
  underloaded nodes attract them [Gupta, D. et al. "Online Load Balancing via Swarm
  Intelligence." *Autonomous Agents and Multi-Agent Systems*, 8(2):209-229, 2004].
- **Network routing**: Ant-based routing protocols (AntNet) achieve adaptive routing in
  telecommunications networks [Di Caro, G. & Dorigo, M. "AntNet: Distributed Stigmergic
  Control for Communications Networks." *JAIR*, 9:317-365, 1998].
- **Collaborative filtering**: User actions (purchases, ratings) serve as implicit pheromones
  that guide other users' discovery [Leskovec, J. et al. "The Dynamics of Viral Marketing."
  *ACM TWEB*, 1(1):5, 2007].
- **Version control**: Git repositories are stigmergic environments — commits, branches, and
  merge patterns guide future development (see `02-git-as-stigmergy.md`).

---

## Stigmergy in Roko's Architecture

Roko implements stigmergy at multiple architectural layers, making it a first-class
coordination primitive rather than an afterthought.

### Layer Mapping

| Layer | Stigmergic Component | Implementation |
|-------|---------------------|----------------|
| L0 Runtime | Engram persistence, decay timers, event emission | `roko-fs` (FileSubstrate JSONL), adaptive clock |
| L1 Framework | Pheromone type system, transport backends | `PheromoneKind` enum, `PheromoneScope` enum, WebSocket/Iroh/ERC-8004 |
| L2 Scaffold | Pheromone-enriched context assembly | Context composer includes ambient pheromone summary |
| L3 Harness | Pheromone-based gate thresholds | Threat pheromone concentration adjusts gate strictness |
| L4 Orchestration | Multi-agent pheromone coordination, morphogenetic specialization | Collective-level pheromone field, reaction-diffusion dynamics |

### The Stigmergic Loop

The complete stigmergic loop in Roko follows this sequence:

```
Agent A acts → deposits pheromone Engram to Substrate
    ↓
Engram propagates (local → Mesh → Global based on scope)
    ↓
Agent B queries Substrate → senses pheromone
    ↓
Scorer rates pheromone intensity and relevance
    ↓
Router selects highest-priority pheromone signal
    ↓
Agent B acts in response → deposits its own pheromone Engram
    ↓
(cycle continues — emergent coordination without direct communication)
```

This loop is isomorphic to Roko's universal cognitive loop (query → score → route → compose →
act → verify → write → react), with pheromone Engrams serving as the coordination substrate.

### Three Knowledge Scopes as Stigmergic Layers

Roko's three-level knowledge architecture maps directly to three stigmergic scopes (see
`05-pheromone-scope.md` for full details):

| Scope | Environment | Persistence | Audience | Example |
|-------|-------------|-------------|----------|---------|
| `Local(SubstrateId)` | Agent's own NeuroStore | Infinite (until GC) | Self only | "I found a bug in module X" |
| `Mesh(CollectiveId)` | Collective's Agent Mesh | Configurable (hours–days) | Collective members | "Module X has a regression" |
| `Global` | Korai chain (public) | Permanent (on-chain) | All agents | "CVE-2026-XXXX affects dependency Y" |

Each scope represents a different stigmergic environment with different persistence
characteristics, audience sizes, and trust levels. Information flows upward through promotion
gates: a local observation can be promoted to Mesh scope after confidence validation, and from
Mesh to Global after collective confirmation.

---

## The Grossman-Stiglitz Paradox and Pheromone Economics

A fundamental challenge in any information-sharing system is the Grossman-Stiglitz paradox:
if information is freely available, no agent has an incentive to incur the cost of producing
it [Grossman, S.J. & Stiglitz, J.E. "On the Impossibility of Informationally Efficient
Markets." *American Economic Review*, 70(3):393-408, 1980].

Roko resolves this paradox through the pheromone system's natural properties:

1. **Decay creates scarcity**: Pheromones decay exponentially. Information that was freely
   sensed yesterday may no longer be available today. Agents that produce fresh signals provide
   genuine value to the collective.

2. **Confirmation extends value**: An agent that confirms another agent's pheromone (by
   depositing the same kind at the same scope) extends the effective half-life. This is a
   cooperative act that benefits the confirming agent (stronger signal for its own decisions)
   and the collective (more persistent shared knowledge).

3. **Reputation tracks contribution**: Agents that consistently produce high-quality pheromones
   (signals that are later confirmed rather than contradicted) accumulate reputation, which
   affects their routing priority and resource allocation.

4. **Domain scoping prevents free-riding**: Pheromones are scoped to specific domains. An
   agent must be active in a domain to sense its pheromones, creating natural communities of
   practice where contributors and consumers overlap.

---

## Research Foundations

The theoretical foundations of stigmergy draw from multiple disciplines:

### Entomology and Ethology

- [Grassé 1959] — Original observation of stigmergy in termite mound construction
- [Wilson, E.O. "The Insect Societies." Belknap Press, 1971] — Comprehensive treatment of
  social insect coordination mechanisms
- [Hölldobler, B. & Wilson, E.O. "The Superorganism." W.W. Norton, 2008] — Superorganism
  theory and multi-level coordination in ant colonies
- [Camazine, S. et al. "Self-Organization in Biological Systems." Princeton University Press,
  2001] — General principles of biological self-organization

### Swarm Intelligence

- [Bonabeau, Dorigo & Theraulaz 1999] — *Swarm Intelligence: From Natural to Artificial
  Systems*, the definitive bridge between biological stigmergy and computational applications
- [Dorigo, Maniezzo & Colorni 1996] — Ant Colony Optimization, the first major computational
  application of stigmergy
- [Kennedy, J. & Eberhart, R. "Particle Swarm Optimization." *IEEE ICNN*, 1995] — Related
  swarm intelligence paradigm

### Multi-Agent Systems

- [Parunak 1997] — "Go to the Ant" — engineering principles from biological multi-agent systems
- [Parunak, Brueckner & Sauter 2005] — Digital pheromones for coordination
- [Theraulaz & Bonabeau 1999] — History and formalization of stigmergy

### Distributed Systems

- [Lamport, L. "Time, Clocks, and the Ordering of Events in a Distributed System." *CACM*,
  21(7), 1978] — Foundation for version vectors used in pheromone deduplication across transports
- [Fidge, C.J. "Timestamps in Message-Passing Systems." *ACSC*, 10(1), 1988] — Vector clock
  formalization applied to Engram sequence tracking

---

## Information-Theoretic Analysis of Stigmergy

Understanding stigmergy through the lens of information theory reveals fundamental limits and design principles for digital pheromone systems. Shannon's framework [Shannon, C.E. "A Mathematical Theory of Communication." *Bell System Technical Journal*, 27(3):379-423, 1948] provides the tools to quantify how much coordination information can flow through a stigmergic medium and how to maximize that throughput.

### The Stigmergic Channel

A stigmergic system can be modeled as a noisy communication channel where agents are both transmitters and receivers, and the shared environment is the channel medium. The channel has properties fundamentally different from point-to-point communication:

| Property | Point-to-Point Channel | Stigmergic Channel |
|----------|----------------------|-------------------|
| Transmitter | One sender | N concurrent senders |
| Receiver | One receiver | N concurrent receivers |
| Medium | Dedicated wire/spectrum | Shared environment (Substrate) |
| Interference | External noise | Other agents' deposits (crosstalk) |
| Memory | Memoryless (typically) | Persistent with decay (exponential) |
| Capacity | Shannon limit: C = B log₂(1 + SNR) | Bounded by field saturation and decay rate |

### Channel Capacity of a Pheromone Field

The effective channel capacity of a pheromone field — the maximum rate at which coordination information can be reliably transmitted — is bounded by three factors:

1. **Field saturation**: When too many pheromones are active simultaneously, the signal-to-noise ratio (SNR) degrades. The "noise floor" is the aggregate intensity of pheromones irrelevant to the sensing agent's current task.

2. **Decay rate**: Faster decay clears the channel faster (higher throughput) but reduces persistence (less time for agents to sense signals). This is analogous to bandwidth allocation in radio systems.

3. **Confirmation overhead**: Each confirmation extends half-life, which consumes channel capacity by keeping old signals alive longer.

The capacity can be approximated as:

```
C_stigmergy ≈ M × R_gc × log₂(1 + I_signal / I_noise)
```

Where:
- `M` = number of distinguishable pheromone kinds (channel multiplexing)
- `R_gc` = effective garbage collection rate (channel clearing rate)
- `I_signal` = intensity of the target pheromone
- `I_noise` = aggregate intensity of non-target pheromones at the same scope

```rust
/// Estimate the effective information throughput of a pheromone field.
///
/// Models the field as a multi-channel communication medium where each
/// PheromoneKind is an independent sub-channel (frequency-division analogy).
///
/// Returns bits per tick of coordination information capacity.
///
/// # Parameters
/// - `kind_count`: Number of active pheromone kinds (channel count)
/// - `avg_signal_intensity`: Mean intensity of target pheromones
/// - `avg_noise_intensity`: Mean intensity of non-target pheromones
/// - `gc_rate_per_tick`: Fraction of field cleared per tick by decay
///
/// # References
/// Shannon, C.E. "A Mathematical Theory of Communication." 1948.
pub fn stigmergic_channel_capacity(
    kind_count: usize,
    avg_signal_intensity: f64,
    avg_noise_intensity: f64,
    gc_rate_per_tick: f64,
) -> f64 {
    if avg_noise_intensity <= 0.0 || gc_rate_per_tick <= 0.0 {
        return 0.0;
    }
    let snr = avg_signal_intensity / avg_noise_intensity;
    kind_count as f64 * gc_rate_per_tick * (1.0 + snr).log2()
}

#[cfg(test)]
mod channel_tests {
    use super::*;

    #[test]
    fn capacity_scales_with_kinds() {
        let c1 = stigmergic_channel_capacity(4, 0.8, 0.2, 0.1);
        let c2 = stigmergic_channel_capacity(8, 0.8, 0.2, 0.1);
        assert!((c2 / c1 - 2.0).abs() < 0.01, "doubling kinds doubles capacity");
    }

    #[test]
    fn capacity_zero_when_all_noise() {
        let c = stigmergic_channel_capacity(4, 0.0, 0.5, 0.1);
        // SNR = 0 → log₂(1) = 0
        assert!(c.abs() < 1e-10);
    }

    #[test]
    fn capacity_increases_with_gc_rate() {
        let c_slow = stigmergic_channel_capacity(4, 0.8, 0.2, 0.05);
        let c_fast = stigmergic_channel_capacity(4, 0.8, 0.2, 0.20);
        assert!(c_fast > c_slow);
    }
}
```

### Entropy Rate of the Pheromone Field

The **entropy rate** of the pheromone field measures the information content generated per unit time. A healthy stigmergic system should have a moderate entropy rate:

- **Too low**: The field is static — either no agents are depositing, or all agents are depositing the same signals (echo chamber). No new coordination information is being generated.
- **Too high**: The field is chaotic — signals change too rapidly for agents to sense and respond. Coordination breaks down because the environment is unstable.
- **Optimal**: The field evolves at a rate matched to the agents' sensing and response timescales.

```rust
/// Compute the entropy rate of pheromone field state transitions.
///
/// Measures how much new information appears in the pheromone field
/// per tick. Uses the conditional entropy H(X_t | X_{t-1}) where X_t
/// is the discretized field state at tick t.
///
/// A sliding window of `window_size` ticks is used to estimate
/// transition probabilities.
///
/// # Returns
/// Entropy rate in bits per tick. Typical healthy range: [0.3, 2.0].
///
/// # Parameters
/// - `field_snapshots`: Sequence of discretized field states (kind → intensity bucket)
/// - `window_size`: Number of ticks for transition probability estimation
///   Default: 100. Range: [20, 1000].
pub struct FieldEntropyEstimator {
    /// Number of intensity buckets for discretization.
    /// Default: 10 (deciles). Range: [4, 100].
    pub intensity_buckets: usize,

    /// Sliding window size for transition probability estimation.
    /// Default: 100 ticks. Range: [20, 1000].
    pub window_size: usize,
}

impl Default for FieldEntropyEstimator {
    fn default() -> Self {
        Self {
            intensity_buckets: 10,
            window_size: 100,
        }
    }
}
```

The entropy rate connects to the edge-of-chaos operating point described by Kauffman (1993): systems at the boundary between order and chaos have maximal computational capacity, which corresponds to an entropy rate that is neither minimal (frozen) nor maximal (random) [Langton, C.G. "Computation at the Edge of Chaos: Phase Transitions and Emergent Computation." *Physica D*, 42(1-3):12-37, 1990].

### Transfer Entropy: Measuring Causal Information Flow

**Transfer entropy** [Schreiber, T. "Measuring Information Transfer." *Physical Review Letters*, 85(2):461-464, 2000] quantifies the directed information flow between agents through the stigmergic medium. Unlike mutual information (which is symmetric), transfer entropy captures the *causal* direction of influence:

```
T_{A→B} = Σ p(b_{t+1}, b_t, a_t) × log₂( p(b_{t+1} | b_t, a_t) / p(b_{t+1} | b_t) )
```

Where:
- `a_t` = Agent A's pheromone deposit at time t
- `b_t` = Agent B's state at time t
- `b_{t+1}` = Agent B's state at time t+1

High `T_{A→B}` indicates that Agent A's pheromone deposits causally influence Agent B's behavior — the stigmergic loop is working. Low transfer entropy in both directions indicates that agents are operating independently despite sharing an environment.

```rust
/// Configuration for transfer entropy estimation between agent pairs.
///
/// Transfer entropy T_{A→B} measures the reduction in uncertainty about
/// Agent B's next state given knowledge of Agent A's current pheromone
/// deposits, beyond what B's own history provides.
///
/// # Interpretation
/// - T ≈ 0: No causal influence (agents independent)
/// - T > 0.1: Moderate influence (stigmergic coordination active)
/// - T > 0.5: Strong influence (tight coupling via pheromones)
///
/// # References
/// Schreiber, T. "Measuring Information Transfer." PRL 85(2), 2000.
pub struct TransferEntropyConfig {
    /// History length for conditioning (number of past ticks).
    /// Default: 5. Range: [1, 20].
    pub history_length: usize,

    /// Minimum number of samples before producing an estimate.
    /// Default: 200. Range: [50, 10000].
    pub min_samples: usize,

    /// Significance threshold (shuffle test p-value).
    /// Default: 0.05. Range: [0.001, 0.1].
    pub significance_threshold: f64,
}

impl Default for TransferEntropyConfig {
    fn default() -> Self {
        Self {
            history_length: 5,
            min_samples: 200,
            significance_threshold: 0.05,
        }
    }
}
```

### Design Implications

The information-theoretic analysis yields concrete design principles for Roko's pheromone system:

| Principle | Rationale | Implementation |
|-----------|-----------|----------------|
| **Kind multiplexing** | Each kind is an independent sub-channel; more kinds = more capacity | 7 universal + Custom(String) kinds |
| **Decay = bandwidth** | Faster decay clears the channel for new signals | Kind-specific half-lives tuned to coordination tempo |
| **Confirmation = coding gain** | Confirmation extends signals, acting like error-correcting codes | Reputation-weighted confirmation mechanics |
| **Scope = frequency reuse** | Local/Mesh/Global scopes reuse the same kind namespace without interference | Three-level scope hierarchy |
| **Sensing threshold = noise floor** | The 0.01 intensity threshold filters noise | Configurable per agent role |
| **Field entropy monitoring** | Tracks whether the system is frozen, healthy, or chaotic | `FieldEntropyEstimator` in the dashboard |

---

## Summary

Stigmergy is Roko's primary coordination mechanism because it provides:

1. **Linear scalability** — O(N × M) vs O(N²) for direct communication
2. **Inherent robustness** — no single point of failure, graceful degradation
3. **Temporal decoupling** — producers and consumers operate asynchronously
4. **Minimal agent complexity** — deposit + sense, no knowledge of other agents needed
5. **Natural information economics** — decay, confirmation, and reputation resolve the
   Grossman-Stiglitz paradox

The next sub-docs detail how these principles are instantiated in Roko's digital pheromone
system (`03-digital-pheromones.md`), the specific pheromone types (`04-pheromone-kinds.md`),
scope model (`05-pheromone-scope.md`), and transport layer (`06-agent-mesh-sync.md`).

---

## Cross-References

- `01-stigmergy-beyond-termites.md` — Stigmergy in non-biological systems
- `02-git-as-stigmergy.md` — Version control as a stigmergic environment
- `03-digital-pheromones.md` — Roko's typed pheromone Engram system
- `07-morphogenetic-specialization.md` — Turing reaction-diffusion for role emergence
- `10-exponential-flywheel.md` — How stigmergy enables superlinear scaling
