# Pheromone Mechanics and Interference

> Depth for [06-MEMORY.md](../../unified/06-MEMORY.md). The 7 universal Pulse kinds, their decay as ring-buffer position loss, the SINR interference model for detectability, anti-saturation mechanisms, Hill-function response thresholds, and the promotion cascade that graduates ephemeral Pulses into durable Signals.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal/Pulse duality, demurrage, Kind system), [02-CELL](../../unified/02-CELL.md) (Score, Verify, Route, React protocols), [03-GRAPH](../../unified/03-GRAPH.md) (Pipeline, Loop), [11-stigmergy-as-bus](11-stigmergy-as-bus.md) (Bus-native stigmergy, ring-buffer eviction as decay, dual-write insight)

**Builds on**: [11-stigmergy-as-bus.md](11-stigmergy-as-bus.md) established that pheromones are not separate infrastructure -- they are Pulses on the Bus with specific payloads, and evaporation is ring-buffer eviction. This document adds the kind taxonomy, the physics of interference between kinds, the mechanisms that prevent saturation, the adaptive thresholds that produce emergent division of labor, and the promotion pipeline that bridges Pulses into Signals.

---

## 1. The Kind System

Every pheromone Pulse carries a `kind` field that determines its semantic meaning, default decay profile, and the behavioral response it triggers. The kind system has three tiers:

1. **Universal kinds** (3): Present in every domain, every agent, every Space. These correspond to the three fundamental survival signals in biological pheromone systems -- alarm (Threat), recruitment (Opportunity), and trail (Wisdom).
2. **Domain-specific kinds** (4): Common across multiple domains but with domain-dependent interpretation. Alpha, Pattern, Anomaly, and Consensus.
3. **Custom kinds** (unbounded): User-defined via `Custom(String)` in TOML configuration. Domain plugins register their own without modifying the core type system.

### 1.1 The 7 Universal Kinds

| Kind | Half-Life | Default Intensity | Biological Analog | Confirmation Effect |
|------|-----------|-------------------|-------------------|---------------------|
| **Threat** | 2h | 1.0 | Alarm pheromone (formic acid) | Standard extension |
| **Opportunity** | 4h | 0.8 | Recruitment pheromone (trail to food) | Standard extension |
| **Wisdom** | 24h | 0.9 | Established trail pheromone | Extends; promotes to Consensus at 4+ |
| **Alpha** | 1h | 1.0 | Ephemeral scent mark | **Paradoxical**: confirmation *reduces* half-life |
| **Pattern** | 12h | 0.7 | Territorial marking | Extends; promotes to Wisdom at 3+ |
| **Anomaly** | 6h | 0.8 | Novel scent detection | Standard extension |
| **Consensus** | 48h | 0.9 | Colony odor | Extends; resists contradiction |

The half-lives are calibrated to the coordination role of each kind. Threats need immediate response and should not permanently poison a region -- 2 hours ensures decay within a work session. Consensus is the most durable because collective agreement is hard-won.

### 1.2 Kind as Pulse Payload

In the unified model, pheromone kinds are variants on the Pulse payload (see [01-SIGNAL.md](../../unified/01-SIGNAL.md) for Pulse definition). A pheromone Pulse has:

```rust
/// Pheromone payload carried by a Pulse on the Bus.
/// See [01-SIGNAL.md] for the Pulse container.
struct PheromonePulse {
    /// Which of the 7 universal kinds (or Custom).
    kind: PheromoneKind,
    /// HDC location fingerprint -- where in concept space.
    /// See [02-hdc-algebra-and-retrieval.md] for HDC encoding.
    location: HdcVector,
    /// Initial intensity in [0.0, 1.0].
    intensity: f64,
    /// The depositing agent's identity.
    source: AgentId,
    /// Bus topic scope (local/group/workspace/global).
    scope: Scope,
}

enum PheromoneKind {
    Threat,
    Opportunity,
    Wisdom,
    Alpha,
    Pattern,
    Anomaly,
    Consensus,
    Custom(String),  // validated: alphanumeric + underscores, 1..=64 chars
}
```

### 1.3 Custom Kind Registration

Custom kinds are registered in `roko.toml` and scoped to their domain namespace:

```toml
[pheromone.custom_kinds.code_coverage_gap]
half_life_secs = 28800  # 8 hours
description = "Code coverage below threshold in a module"

[pheromone.custom_kinds.model_drift]
half_life_secs = 7200   # 2 hours
description = "ML model predictions diverging from observed outcomes"
```

Custom kind identifiers must pass validation: ASCII alphanumeric + underscores, 1-64 characters, must not collide with built-in names (case-insensitive), must not start with `_` (reserved for internal kinds). Two different domains can register the same string identifier without conflict -- the scoping key is `(domain, kind_id)`.

---

## 2. Decay as Ring-Buffer Position Loss

[11-stigmergy-as-bus.md](11-stigmergy-as-bus.md) established the core insight: pheromone evaporation is not a separate timer infrastructure -- it is ring-buffer eviction. Bus ring buffers have finite capacity. Older Pulses are overwritten by newer ones. A pheromone that nobody reinforces gets pushed out of the buffer by the steady arrival of new Pulses.

This document adds precision: different kinds need different effective decay rates. The mechanism is the same (ring-buffer position), but the *interpretation* differs.

### 2.1 Position-Based Intensity

When an agent senses a pheromone Pulse, it computes the current intensity from the Pulse's position in the ring buffer and the kind's half-life:

```rust
/// Compute effective intensity from ring-buffer position.
///
/// The ring buffer has capacity C. A Pulse at position p (where 0 = newest,
/// C-1 = oldest) has decayed proportionally to how close it is to eviction.
///
/// For a kind with half-life H (expressed as ring-buffer positions rather
/// than wall-clock time), the intensity follows:
///
///   intensity(p) = initial_intensity * 2^(-p / H)
///
/// This is mathematically equivalent to exponential time-decay, but the
/// "clock" is ring-buffer throughput rather than wall time.
fn position_intensity(
    initial_intensity: f64,
    position: usize,      // 0 = newest
    kind_half_life: usize, // in ring-buffer positions
) -> f64 {
    let exponent = -(position as f64) / (kind_half_life as f64);
    initial_intensity * 2.0_f64.powf(exponent)
}
```

The half-life parameter is expressed in ring-buffer positions rather than seconds. This means decay rate is coupled to Bus throughput: in a busy Space, Pulses decay faster because the ring fills faster. In a quiet Space, they persist longer. This is the correct behavior -- a busy coordination environment should have a shorter attention span for individual signals.

### 2.2 Confirmation Extension

When a second agent independently deposits a Pulse of the same kind and proximate location, the original Pulse's effective half-life extends. For standard (non-Alpha) kinds:

```
half_life_extended = half_life_base * (1 + 0.15 * ln(1 + confirmations))
```

| Confirmations | Extension Multiplier | Effective Half-Life (base = 12h) |
|---------------|---------------------|----------------------------------|
| 0 | 1.00 | 12.0h |
| 1 | 1.10 | 13.2h |
| 3 | 1.21 | 14.5h |
| 5 | 1.27 | 15.2h |
| 10 | 1.36 | 16.3h |

The 0.15 coefficient was selected so that 10 confirmations extend half-life by ~36%. The logarithmic function `ln(1 + n)` grows without bound but slowly enough that no practical confirmation count produces an immortal Pulse.

### 2.3 The Alpha Paradox

Alpha Pulses have **paradoxical** confirmation: more confirmations mean *more agents know about the edge*, so the advantage erodes faster.

```
half_life_effective(Alpha) = half_life_base * max(0.5, 1 - confirmations * 0.2)
```

| Confirmations | Multiplier | Effective Half-Life (base = 1h) |
|---------------|------------|----------------------------------|
| 0 | 1.0 | 60 min |
| 1 | 0.8 | 48 min |
| 2 | 0.6 | 36 min |
| 3 | 0.4 | 24 min |
| 5+ | 0.5 (floor) | 30 min |

The floor at 0.5 prevents instant evaporation. An Alpha that many agents have confirmed still persists for at least half its base half-life, preserving a minimum window for agents that have already committed resources.

---

## 3. The SINR Interference Model

When multiple pheromone kinds coexist in the same scope, they interfere with each other's detectability. A high-intensity Threat suppresses Opportunity sensing, just as alarm pheromone overrides foraging pheromone in ant colonies (Wilson 1971).

The interference model adapts the Signal-to-Interference-plus-Noise Ratio (SINR) from wireless communications (Tse & Viswanath 2005):

```
SINR_k = I_target_k / (SUM_{j != k} alpha_{jk} * I_j + N_0)
```

Where:
- `I_target_k` = intensity of the target Pulse of kind k
- `I_j` = aggregate intensity of Pulses of kind j in the same scope
- `alpha_{jk}` = cross-kind interference coefficient (how much kind j interferes with sensing kind k)
- `N_0` = background noise floor (default: 0.01)

This is expressed as a **Score Cell** that rates pheromone detectability (see [02-CELL.md](../../unified/02-CELL.md) for the Score protocol):

```rust
/// Score Cell that computes SINR-adjusted pheromone intensity.
///
/// Inputs: PheromonePulse + ambient field intensities per kind.
/// Output: Signal with adjusted intensity score on the detectability axis.
///
/// This Cell implements the Score protocol. Its score represents
/// whether a given pheromone is detectable above the interference
/// of other active pheromones.
struct PheromoneDetectabilityScore {
    /// 7x7 matrix where entry [j][k] = how much kind j
    /// interferes with sensing kind k. Range per entry: [0.0, 1.0].
    interference_matrix: [[f64; 7]; 7],
    /// Background noise level. Default: 0.01.
    noise_floor: f64,
    /// Minimum SINR for detection. Default: 1.0 (0 dB).
    min_sinr: f64,
}
```

### 3.1 The Default Interference Matrix

The matrix encodes biological precedent from Wilson 1971:

```
           Thr   Opp   Wis   Alp   Pat   Ano   Con
Threat  [ 0.00  0.60  0.10  0.30  0.20  0.10  0.05 ]
Opp     [ 0.00  0.00  0.00  0.10  0.05  0.00  0.00 ]
Wisdom  [ 0.00  0.00  0.00  0.00  0.00  0.00  0.00 ]
Alpha   [ 0.10  0.10  0.00  0.00  0.05  0.00  0.00 ]
Pattern [ 0.00  0.00  0.00  0.00  0.00  0.00  0.00 ]
Anomaly [ 0.20  0.10  0.00  0.10  0.10  0.00  0.00 ]
Cons    [ 0.00  0.00  0.00  0.00  0.00  0.00  0.00 ]
```

Key design choices:
- **Threat -> Opportunity: 0.60** -- Alarm overrides foraging. Agents in threat-response mode should not be distracted by opportunities.
- **Threat -> Wisdom: 0.10** -- Knowledge is resistant to alarm. Validated insights persist through crisis.
- **Opportunity -> Threat: 0.00** -- Opportunities never mask threats. Safety is asymmetric.
- **Consensus -> all: 0.05 max** -- Consensus is highly resistant to interference. Collective agreement is durable.
- **Wisdom and Pattern rows: all zeros** -- These informational kinds do not interfere with other sensing. They inform without suppressing.

### 3.2 SINR-Adjusted Intensity Computation

```rust
/// Compute the effective sensed intensity of a target pheromone,
/// accounting for cross-kind interference.
///
/// Returns 0.0 if SINR falls below min_sinr (signal undetectable).
/// Otherwise returns gracefully degraded intensity.
fn sinr_adjusted_intensity(
    target_kind: usize,
    target_intensity: f64,
    active_intensities: &[f64; 7],  // aggregate intensity per kind
    matrix: &[[f64; 7]; 7],
    noise_floor: f64,
    min_sinr: f64,
) -> f64 {
    let interference: f64 = active_intensities.iter()
        .enumerate()
        .filter(|&(j, _)| j != target_kind)
        .map(|(j, &intensity)| matrix[j][target_kind] * intensity)
        .sum();

    let sinr = target_intensity / (interference + noise_floor);
    if sinr < min_sinr {
        0.0  // Below detection threshold
    } else {
        target_intensity * (sinr / (1.0 + sinr))  // Graceful degradation
    }
}
```

**Worked example**: Threat at 0.9 intensity, Opportunity at 0.8 intensity, all other kinds at 0.0:
- Threat interference on Opportunity: 0.9 * 0.60 = 0.54
- SINR for Opportunity: 0.8 / (0.54 + 0.01) = 1.45
- Effective Opportunity intensity: 0.8 * (1.45 / 2.45) = 0.47

The Opportunity is still detectable (SINR > 1.0) but at reduced effective intensity. During a genuine crisis, the Threat naturally dominates attention.

---

## 4. Anti-Saturation Mechanisms

When too many pheromone Pulses coexist in a scope, the field becomes noise rather than signal. Two threshold mechanisms prevent this, expressed as a **Verify Cell** pre-condition (see [02-CELL.md](../../unified/02-CELL.md) for the Verify protocol):

### 4.1 Soft Threshold (500 Active Pulses)

When the total active Pulse count in a scope exceeds 500, low-intensity Pulses decay at 2x speed. The ring buffer effectively halves in capacity for weak signals:

```rust
/// Anti-saturation Verify Cell.
///
/// Pre-condition check before a new pheromone Pulse is accepted.
/// At soft threshold: weak Pulses decay faster (ring-buffer shrinks for them).
/// At hard threshold: only high-intensity Pulses survive.
struct AntiSaturationVerify {
    /// Above this count, low-intensity Pulses decay 2x faster.
    soft_threshold: usize,   // default: 500
    /// Above this count, only Pulses with intensity > hard_min survive.
    hard_threshold: usize,   // default: 2000
    /// Minimum intensity to survive hard threshold.
    hard_min_intensity: f64, // default: 0.1
    /// Maximum Pulses per kind per scope. Prevents monopolization.
    max_per_kind_per_scope: usize, // default: 100
}
```

### 4.2 Hard Threshold (2000 Active Pulses)

Above 2000, only Pulses with intensity > 0.1 are retained. Everything below is immediately eligible for eviction. This is the "emergency brake" that prevents the field from becoming uniformly noisy.

### 4.3 Per-Kind Cap

No single kind can have more than 100 active Pulses per scope. This prevents a flood of Threat Pulses from consuming the entire field capacity. If a genuine crisis produces 100+ Threats, the oldest ones are evicted -- recent information is prioritized.

---

## 5. The Response Threshold Model

The response threshold model determines which agents respond to which pheromone Pulses. This is expressed as a **Route Cell** (see [02-CELL.md](../../unified/02-CELL.md) for the Route protocol) -- the agent chooses whether to act based on a per-kind threshold.

### 5.1 The Hill Function

The probability of responding follows a Hill function (Bonabeau, Theraulaz & Deneubourg 1998):

```
P(respond to kind k) = I_k^n / (I_k^n + theta_k^n)
```

Where:
- `I_k` = current SINR-adjusted intensity of Pulse kind k
- `theta_k` = agent's response threshold for kind k (range: [0.01, 1.0])
- `n` = Hill coefficient controlling curve steepness (default: 2)

```rust
/// Per-agent response thresholds for pheromone-driven task allocation.
///
/// Each agent maintains thresholds per kind. These thresholds determine
/// the probability of switching from the current task to respond to an
/// ambient Pulse.
///
/// This implements the Route protocol: given a set of candidate Pulses,
/// the agent routes its attention based on threshold-weighted probabilities.
struct ResponseThresholds {
    /// Per-kind response thresholds. Lower = more responsive.
    thresholds: [f64; 7],  // indexed by PheromoneKind
    /// Hill coefficient. n=1: linear. n=2: sigmoid. n=5: near-binary.
    hill_coefficient: f64,  // default: 2.0
    /// Learning rate for threshold adaptation. Default: 0.05.
    learning_rate: f64,
    /// Minimum threshold (maximum responsiveness). Default: 0.05.
    min_threshold: f64,
    /// Maximum threshold (minimum responsiveness). Default: 0.95.
    max_threshold: f64,
}

impl ResponseThresholds {
    /// Compute probability of responding to a Pulse of given kind.
    fn response_probability(&self, kind_idx: usize, intensity: f64) -> f64 {
        let theta = self.thresholds[kind_idx];
        let n = self.hill_coefficient;
        let i_n = intensity.powf(n);
        let theta_n = theta.powf(n);
        i_n / (i_n + theta_n)
    }

    /// Reinforce after successful response (lower threshold).
    fn reinforce(&mut self, kind_idx: usize) {
        self.thresholds[kind_idx] =
            (self.thresholds[kind_idx] - self.learning_rate).max(self.min_threshold);
    }

    /// Habituate after ignoring a signal (raise threshold, slower rate).
    fn habituate(&mut self, kind_idx: usize) {
        self.thresholds[kind_idx] =
            (self.thresholds[kind_idx] + self.learning_rate * 0.5).min(self.max_threshold);
    }
}
```

### 5.2 Emergent Division of Labor

The response threshold model produces emergent specialization through a positive feedback Loop (see [03-GRAPH.md](../../unified/03-GRAPH.md) for Loop pattern):

```
Agent succeeds at responding to Threat
    -> Threat threshold decreases (reinforce)
    -> Agent is more likely to respond to future Threats
    -> Agent becomes a "threat responder" without explicit assignment
    -> Other agents' Threat thresholds drift upward (habituation from non-response)
    -> Complementary specialization emerges
```

This mechanism operates at a **tactical timescale** (10-100 ticks) -- distinct from morphogenetic specialization which operates at a strategic timescale (500-2000 ticks). See [13-morphogenetic-specialization-as-loop.md](13-morphogenetic-specialization-as-loop.md) for the strategic layer.

| Mechanism | Timescale | Granularity | What It Decides |
|-----------|-----------|-------------|-----------------|
| Response thresholds | 10-100 ticks | Per-kind responsiveness | "Should I respond to this Pulse right now?" |
| Morphogenetic specialization | 500-2000 ticks | 8D strategy vector | "What kind of agent should I be?" |

The two reinforce each other: morphogenetic specialization sets the strategic role, while response thresholds handle tactical moment-to-moment attention allocation within that role.

---

## 6. The Promotion Cascade

The promotion cascade is the mechanism by which ephemeral Pulses graduate into durable Signals (see [01-SIGNAL.md](../../unified/01-SIGNAL.md) for the graduation bridge). It is expressed as a Pipeline of Verify Cells (see [02-CELL.md](../../unified/02-CELL.md)):

```
Pattern --[3+ confirmations, age > 50% half-life]--> Wisdom
Wisdom  --[4+ confirmations]-----------------------> Consensus
Consensus --[5+ confirmations]---------------------> Permanent Signal (Store)
```

### 6.1 Promotion Thresholds

```rust
/// Configuration for the promotion Pipeline.
/// Each stage is a Verify Cell that checks confirmation count and age.
struct PromotionConfig {
    /// Pattern -> Wisdom: minimum confirmations. Default: 3. Range: [2, 10].
    pattern_to_wisdom_confirmations: u32,
    /// Pattern -> Wisdom: minimum age as fraction of half-life.
    /// Default: 0.5 (must survive half its half-life). Range: [0.1, 1.0].
    pattern_to_wisdom_min_age_fraction: f64,
    /// Wisdom -> Consensus: minimum confirmations. Default: 4. Range: [3, 20].
    wisdom_to_consensus_confirmations: u32,
    /// Consensus -> permanent Signal: minimum confirmations. Default: 5. Range: [4, 50].
    consensus_to_signal_confirmations: u32,
    /// Whether to auto-promote or require explicit agent action. Default: true.
    auto_promote: bool,
}
```

### 6.2 The Promotion Pipeline Graph

```toml
# Promotion Pipeline: Verify Cells that graduate Pulses into Signals.
# Runs as a background Graph on the Curator cycle (every 50 ticks).

[[cells]]
id = "scan"
protocol = "observe"
description = "Scan active Pulses in the Bus ring buffer for promotion eligibility"

[[cells]]
id = "verify_pattern"
protocol = "verify"
description = "Check Pattern Pulses: confirmations >= 3 AND age > 50% half-life"

[[cells]]
id = "verify_wisdom"
protocol = "verify"
description = "Check Wisdom Pulses: confirmations >= 4"

[[cells]]
id = "verify_consensus"
protocol = "verify"
description = "Check Consensus Pulses: confirmations >= 5"

[[cells]]
id = "graduate"
protocol = "store"
description = "Graduate promoted Pulse into a durable Signal in Store with demurrage"

[[edges]]
from = "scan"
to = ["verify_pattern", "verify_wisdom", "verify_consensus"]

[[edges]]
from = ["verify_pattern", "verify_wisdom", "verify_consensus"]
to = "graduate"
```

### 6.3 Idempotency and Deduplication

The promotion check is idempotent: if a Signal with the same parent hash already exists in Store, the duplicate promotion is skipped. The parent hash provides deduplication -- two promoters cannot create two Wisdom Signals from the same Pattern Pulse.

### 6.4 Consensus Stability

Consensus Pulses and Signals resist contradiction. To contradict a Consensus, an agent must deposit a Threat Pulse of equal or greater intensity with explicit evidence. This prevents casual erosion of collective agreements while still allowing well-evidenced challenges.

---

## 7. Threat Intensity Scaling

Not all Threats are equal. The initial intensity encodes severity:

| Intensity Range | Severity | Example |
|-----------------|----------|---------|
| 0.1 - 0.3 | Low | Minor style violation, non-blocking warning |
| 0.4 - 0.6 | Medium | Test flakiness, moderate performance regression |
| 0.7 - 0.8 | High | Test failure, security vulnerability in development |
| 0.9 - 1.0 | Critical | Build failure, production security vulnerability |

**Gate interaction**: Threat Pulses at group scope influence adaptive gate thresholds (see [07-LEARNING.md](../../unified/07-LEARNING.md)). When ambient Threat intensity is high, gate thresholds tighten (more strict verification), implementing a collective immune response. This is a cross-system interaction: the pheromone field modulates the verification pipeline.

**Escalation**: If a Threat has intensity > 0.8 and confirmations > 3, it escalates to a broader scope. A workspace-level Threat that multiple agents confirm becomes visible at the global level.

---

## What This Enables

1. **Self-organizing attention**: Agents allocate attention based on ambient pheromone gradients rather than explicit assignment. The Hill function produces smooth, adaptive responses.
2. **Interference-aware sensing**: The SINR model prevents signal flooding. High-priority signals (Threats) naturally suppress low-priority ones without explicit priority queues.
3. **Knowledge graduation**: The promotion cascade is the bridge between ephemeral coordination (Bus) and durable knowledge (Store). Collectively validated patterns become permanent knowledge.
4. **Anti-fragile coordination**: Anti-saturation mechanisms prevent the pheromone field from degrading under load. The system gets more selective, not more confused, as signal density increases.
5. **Emergent specialization at tactical timescale**: Response thresholds produce division of labor without role assignment, complementing the strategic morphogenetic specialization.

## Feedback Loops

1. **Threshold calibration Loop**: Agent responds to Threat -> success -> threshold lowers -> more responsive to Threats -> more success -> specialization deepens. This is a predict-publish-correct Loop (see [02-CELL.md](../../unified/02-CELL.md)).
2. **Promotion calibration Loop**: Patterns that get confirmed promote to Wisdom. Wisdom that gets confirmed promotes to Consensus. Consensus that persists enters Store. Each stage filters noise, and the filtering thresholds can be adjusted based on downstream Signal quality.
3. **SINR adaptation Loop**: If the interference matrix is miscalibrated (e.g., Threat suppresses Opportunity too aggressively, causing missed opportunities), the matrix coefficients can be adjusted via the same predict-publish-correct mechanism -- predict the optimal coefficient, observe outcomes, adjust.

## Open Questions

1. **Should the interference matrix be learned or hardcoded?** The current matrix is derived from biological analogy. An adaptive matrix that learns cross-kind interactions from operational data might be more accurate but risks instability during the learning period.
2. **What is the right noise floor N_0?** The default 0.01 works for moderate field sizes, but at very large scales (1000+ agents), the noise floor may need to scale with field size.
3. **Should Alpha paradox apply to Custom kinds?** Some domain-specific kinds might also have paradoxical confirmation (e.g., a trading signal that loses value when widely known). The current design hard-codes the paradox for Alpha only.
4. **How should the Hill coefficient n adapt?** n=2 (sigmoid) is the default, but some agents might benefit from n=5 (near-binary: either respond or don't) or n=1 (linear: gradual response). Should n be per-agent, per-kind, or global?

## Implementation Tasks

1. **Add `PheromoneKind` enum to `roko-core`**: `crates/roko-core/src/types.rs` -- define the 7 universal kinds + `Custom(String)` variant with validation.
2. **Implement `InterferenceMatrix` in `roko-core`**: `crates/roko-core/src/scoring.rs` -- default 7x7 matrix, SINR computation function, configurable via `roko.toml`.
3. **Implement `AntiSaturationVerify` in `roko-gate`**: `crates/roko-gate/src/pheromone.rs` -- soft/hard thresholds, per-kind cap, integrate into gate pipeline.
4. **Implement `ResponseThresholds` in `roko-agent`**: `crates/roko-agent/src/thresholds.rs` -- Hill function, reinforce/habituate, persist thresholds to `.roko/learn/response-thresholds.json`.
5. **Implement promotion Pipeline in `roko-cli/src/orchestrate.rs`**: Wire the promotion Verify Cells into the Curator cycle (every 50 ticks). Ensure idempotent graduation into Store.
6. **Wire SINR scoring into context assembly**: `crates/roko-compose/src/system_prompt_builder.rs` -- when assembling ambient signals for the system prompt, use SINR-adjusted intensities rather than raw intensities.
7. **Add `alpha_effective_half_life` function**: `crates/roko-core/src/decay.rs` -- paradoxical confirmation for Alpha kind, floor at 0.5.

---

## References

- Bonabeau, Theraulaz & Deneubourg 1998, "Fixed Response Thresholds and the Regulation of Division of Labor in Insect Societies", *Bull. Math. Biol.*
- Tse, D. & Viswanath, P. 2005, *Fundamentals of Wireless Communication*, Cambridge University Press
- Wilson, E.O. 1971, *The Insect Societies*, Belknap Press
- Nealson, Platt & Hastings 1970, "Cellular Control of the Synthesis and Activity of the Bacterial Luminescent System", *J. Bacteriology*
- Grassé 1959, Termite mound stigmergy, *Insectes Sociaux*
