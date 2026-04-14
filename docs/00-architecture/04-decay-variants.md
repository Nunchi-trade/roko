# Decay Variants

> **Abstract:** Every Engram in Roko decays. Pheromones fade over hours; episodes become
> less relevant over weeks; playbook rules age out over months. The Decay enum unifies
> four decay models — None (permanent), HalfLife (exponential), Ttl (hard cutoff), and
> Ebbinghaus (psychological forgetting curve) — into a single function that takes an age
> and returns a weight multiplier in [0, 1]. This document specifies each variant, provides
> the mathematical formulas, shows the implementation, and explains how decay interacts with
> Score to produce effective weight.


> **Implementation**: Shipping

---

## 1. Why Decay

Information has a shelf life. A pheromone warning about high gas prices is urgent now but
irrelevant in two hours. An episode from yesterday's debugging session is useful today but
decreasingly relevant next week. A playbook rule extracted from months of experience should
persist much longer.

Static systems treat all stored information equally regardless of age. This leads to
information overload — the system's "memory" grows without bound, relevant signals are
buried under stale data, and query performance degrades.

Roko solves this by making decay a first-class property of every Engram. The Decay enum
defines how an Engram's weight diminishes over time. When combined with the Score (see
[03-score-7-axis-appraisal.md](03-score-7-axis-appraisal.md)), decay produces a
time-varying effective weight:

```
weight(t) = score.effective() × decay.apply(age_ms)
```

Engrams whose weight falls below a threshold are pruned by `Substrate.prune()`, implementing
automatic memory management without explicit garbage collection.

---

## 2. The Decay Enum

```rust
/// How an Engram's weight diminishes over time.
///
/// `Decay::apply(age_ms)` returns a multiplier in [0.0, 1.0] that scales
/// the Engram's score. A fresh Engram has multiplier 1.0; a fully-decayed
/// Engram has multiplier 0.0.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Decay {
    /// No decay — Engram weight is permanent.
    None,

    /// Exponential half-life: weight = 0.5 ^ (age / half_life_ms).
    HalfLife { half_life_ms: u64 },

    /// Hard cutoff: full weight until ttl_ms, then zero.
    Ttl { ttl_ms: u64 },

    /// Ebbinghaus forgetting curve: weight = exp(-age / (strength * scale_ms)).
    Ebbinghaus { strength: f32, scale_ms: u64 },
}
```

---

## 3. Variant: None (Permanent)

**Formula**: `weight = 1.0` for all ages.

**Use cases**: Configuration signals, schema definitions, identity records — information
that should never expire.

```rust
Self::None => 1.0,
```

Examples:
- Agent configuration (`roko.toml` parsed as Engrams)
- Schema definitions (Kind → Body format mappings)
- Identity records (ERC-8004 Korai Passport)

---

## 4. Variant: HalfLife (Exponential Decay)

**Formula**: `weight = 0.5 ^ (age_ms / half_life_ms)`

This is the classic exponential decay used throughout biology, physics, and information
theory. The weight halves every `half_life_ms` milliseconds. It never reaches exactly zero
but approaches it asymptotically.

```rust
Self::HalfLife { half_life_ms } => {
    if *half_life_ms == 0 {
        return 0.0;
    }
    let hl = *half_life_ms as f32;
    (0.5_f32).powf(age_ms / hl)
}
```

### 4.1 Pheromone Half-Lives

The agent-chain design defines three standard pheromone half-lives, available as constants:

```rust
impl Decay {
    /// THREAT pheromone half-life (2 hours).
    pub const THREAT: Self = Self::HalfLife {
        half_life_ms: 7_200_000,
    };
    /// OPPORTUNITY pheromone half-life (4 hours).
    pub const OPPORTUNITY: Self = Self::HalfLife {
        half_life_ms: 14_400_000,
    };
    /// WISDOM pheromone half-life (24 hours).
    pub const WISDOM: Self = Self::HalfLife {
        half_life_ms: 86_400_000,
    };
}
```

These match the stigmergic coordination model (Grassé 1959; Parunak et al. 2007):

| Pheromone Type | Half-Life | After 1 Half-Life | After 4 Half-Lives | Purpose |
|---|---|---|---|---|
| **THREAT** | 2 hours | 50% weight | 6.25% weight | Urgent warnings that decay fast |
| **OPPORTUNITY** | 4 hours | 50% weight | 6.25% weight | Opportunities that remain relevant longer |
| **WISDOM** | 24 hours | 50% weight | 6.25% weight | Accumulated knowledge that persists |

### 4.2 Weight at Key Timepoints

For a THREAT pheromone (2h half-life):
- At creation: 1.0
- After 1 hour: 0.707
- After 2 hours (1 half-life): 0.5
- After 4 hours (2 half-lives): 0.25
- After 8 hours (4 half-lives): 0.0625
- After 20 half-lives (40 hours): < 0.000001

### 4.3 Edge Case: Zero Half-Life

If `half_life_ms = 0`, the Engram decays immediately to zero weight at any positive age.
This is useful for "fire-and-forget" Engrams that should be consumed once and then vanish.

---

## 5. Variant: Ttl (Hard Cutoff)

**Formula**: `weight = if age_ms < ttl_ms { 1.0 } else { 0.0 }`

A step function: full weight until the TTL expires, then zero. There is no gradual decline.

```rust
Self::Ttl { ttl_ms } => {
    if age_ms >= *ttl_ms as f32 {
        0.0
    } else {
        1.0
    }
}
```

### 5.1 Use Cases

| Context | TTL | Rationale |
|---|---|---|
| Bounty offers | 1 hour | Bounties expire at a fixed time |
| Session tokens | 24 hours | Session validity is binary |
| Transaction proposals | 10 minutes | Transaction proposals have a strict validity window |
| Rate limit windows | 60 seconds | Rate limits reset at fixed intervals |

TTL is appropriate when the information has a **defined validity window** — it is fully
valid until the window closes, then immediately invalid. There is no "partially expired
bounty."

---

## 6. Variant: Ebbinghaus (Psychological Forgetting Curve)

**Formula**: `weight = exp(-age_ms / (strength × scale_ms))`

Named after Hermann Ebbinghaus (1885), who discovered that human memory follows an
exponential decay curve where retention depends on the "strength" of the memory (how many
times it was rehearsed, how emotionally significant it was).

```rust
Self::Ebbinghaus { strength, scale_ms } => {
    if *scale_ms == 0 || *strength <= 0.0 {
        return 0.0;
    }
    let scale = (*strength) * (*scale_ms as f32);
    (-age_ms / scale).exp()
}
```

### 6.1 Parameters

- **`strength`** ∈ [0, ∞): How well-encoded this memory is. Higher strength = slower decay.
  Strength increases when a knowledge entry is rehearsed (accessed during Dreams NREM replay)
  or validated by experience.

- **`scale_ms`**: Base time unit. Controls the overall timescale of decay.

### 6.2 Comparison with HalfLife

At `age_ms = scale_ms` with `strength = 1.0`:
- Ebbinghaus: weight = exp(-1) ≈ 0.368
- HalfLife: weight = 0.5

Ebbinghaus decays faster initially but more slowly at longer timescales (the "long tail" of
memory). This matches psychological evidence: memories are most fragile shortly after
formation but become increasingly robust if they survive the initial decay period.

### 6.3 Knowledge Tier Half-Lives

The Neuro subsystem (see [13-cognitive-cross-cuts.md](13-cognitive-cross-cuts.md)) uses
Ebbinghaus decay with strength values corresponding to knowledge tiers:

| Knowledge Tier | Strength Multiplier | Effect |
|---|---|---|
| **Transient** | 0.1× | Decays very fast. Short-term working memory. |
| **Working** | 0.5× | Moderate decay. Active task context. |
| **Consolidated** | 1.0× | Standard decay. Proven knowledge. |
| **Persistent** | 5.0× | Very slow decay. Core heuristics and insights. |

Promotion between tiers happens during the Dreams consolidation cycle. A Transient knowledge
entry that proves useful in multiple episodes may be promoted to Working, then Consolidated,
then Persistent — each promotion increasing its strength multiplier and extending its
effective lifetime.

---

## 7. The apply() Method

All four variants are unified through the `apply()` method, which returns a weight multiplier
in [0.0, 1.0]:

```rust
impl Decay {
    /// Apply decay to get a weight multiplier at the given age.
    /// Clamped to [0.0, 1.0]. Negative ages (clock skew) return 1.0.
    pub fn apply(&self, age_ms: i64) -> f32 {
        if age_ms <= 0 {
            return 1.0;
        }
        let age_ms = age_ms as f32;
        match self {
            Self::None => 1.0,
            Self::HalfLife { half_life_ms } => {
                if *half_life_ms == 0 { return 0.0; }
                let hl = *half_life_ms as f32;
                (0.5_f32).powf(age_ms / hl)
            }
            Self::Ttl { ttl_ms } => {
                if age_ms >= *ttl_ms as f32 { 0.0 } else { 1.0 }
            }
            Self::Ebbinghaus { strength, scale_ms } => {
                if *scale_ms == 0 || *strength <= 0.0 { return 0.0; }
                let scale = (*strength) * (*scale_ms as f32);
                (-age_ms / scale).exp()
            }
        }
    }

    /// Is this Engram still meaningfully alive (weight > threshold)?
    pub fn is_alive(&self, age_ms: i64, threshold: f32) -> bool {
        self.apply(age_ms) > threshold
    }
}
```

### 7.1 Negative Age Handling

If `age_ms <= 0` (clock skew or future-dated Engram), `apply()` returns 1.0. The Engram is
treated as fresh. This prevents clock synchronization issues from producing negative weights.

---

## 8. Decay and Substrate Pruning

The Substrate trait provides a `prune()` method that removes Engrams whose effective weight
has fallen below a threshold:

```rust
async fn prune(&self, threshold: f32, ctx: &Context) -> Result<usize>;
```

Pruning uses `weight_at(ctx.now_ms)` which combines Score and Decay:

```
weight = score.effective() × decay.apply(now_ms - created_at_ms)
```

Engrams with `weight < threshold` are removed. This is how the system implements automatic
memory management — information that has decayed below the threshold of usefulness is
garbage-collected without explicit cleanup logic.

---

## 9. Decay Selection Guidelines

| Engram Type | Recommended Decay | Rationale |
|---|---|---|
| Configuration | `None` | Config is permanent until explicitly changed |
| Gate verdicts | `HalfLife { 86_400_000 }` (24h) | Verdicts are time-sensitive — code changes invalidate them |
| Pheromones | `HalfLife` (2-24h per type) | Stigmergic signals fade naturally |
| Episodes | `HalfLife { 604_800_000 }` (7d) | Recent episodes are more relevant |
| Knowledge (Transient) | `Ebbinghaus { 0.1, 3_600_000 }` | Unverified observations decay fast |
| Knowledge (Persistent) | `Ebbinghaus { 5.0, 86_400_000 }` | Proven insights persist for weeks |
| Bounties | `Ttl { 3_600_000 }` (1h) | Bounties have fixed expiry |
| Predictions | `HalfLife { 3_600_000 }` (1h) | Predictions are relevant near their resolution time |
| Tool traces | `HalfLife { 43_200_000 }` (12h) | Recent tool performance matters most |
| Metrics | `HalfLife { 86_400_000 }` (24h) | Rolling metrics window |

---

## 10. Modern Forgetting Curve Research

### 10.1 The Power-Law vs. Exponential Debate

Averell & Heathcote (2011, Journal of Mathematical Psychology 55(1)) provide the definitive
resolution: **individual traces decay exponentially, but aggregate population data follows a
power law**. The power law emerges as a mathematical artifact of mixing exponentials with
different decay constants across individuals.

This means Roko's `Decay::Ebbinghaus` (which is exponential) is correct at the individual
Engram level. Population-level statistics over many Engrams will show power-law patterns,
which is expected — not a modeling error.

### 10.2 The Permanent Asymptote

Averell & Heathcote also found above-chance asymptotic retention — some memories never fully
decay. This supports `Decay::None` for Persistent-tier knowledge and suggests that very
long-lived Ebbinghaus entries should have a configurable floor:

```rust
/// Ebbinghaus with permanent floor: weight never drops below `floor`.
/// Models the finding that some fraction of encoding is permanent.
pub fn apply_with_floor(&self, age_ms: i64, floor: f32) -> f32 {
    let raw = self.apply(age_ms);
    raw.max(floor)
}
```

| Parameter | Default | Range | Description |
|---|---|---|---|
| `persistent_floor` | 0.05 | 0.0 - 0.2 | Minimum weight for Persistent-tier Ebbinghaus entries |

### 10.3 Sleep Discontinuity

Murre & Dros (2015, PLOS ONE) replicated Ebbinghaus and found a **discontinuity at 24 hours**
— a memory boost after sleep consistent with consolidation research. This validates Roko's
Dreams consolidation cycle (Delta frequency) as a biologically grounded mechanism.

### 10.4 The Summed Exponential Model

Murre's Memory Chain Model (MCM) sums two exponential processes — a fast-decaying short-term
component and a slower long-term component — achieving better fit than any single function:

```
R(t) = a₁ × exp(-t/τ₁) + a₂ × exp(-t/τ₂)
```

A potential `Decay::SummedExponential` variant for Roko:

```rust
/// Summed exponential: models dual-process consolidation.
/// Fast component (working memory) + slow component (consolidated).
SummedExponential {
    fast_weight: f32,   // a₁ — proportion in fast store
    fast_tau_ms: u64,   // τ₁ — fast decay time constant
    slow_tau_ms: u64,   // τ₂ — slow decay time constant
},
```

---

## 11. Adaptive Decay: Learning Optimal Rates from Usage

### 11.1 The FSRS Model (Ye 2022)

The Free Spaced Repetition Scheduler (FSRS) models memory with three variables:

| Variable | Definition | Roko Analog |
|---|---|---|
| **Stability (S)** | How deeply encoded the memory is | Ebbinghaus `strength` parameter |
| **Retrievability (R)** | Probability of recall right now: `R = exp(ln(0.9) × t/S)` | `Decay::apply(age_ms)` output |
| **Difficulty (D)** | Intrinsic difficulty of the item | Knowledge type + content complexity |

FSRS learns S and D per item from retrieval history, producing 20-30% fewer reviews than SM-2
for the same retention level. The key mechanism: **stability increases after each successful
retrieval**, with larger increases for harder retrievals.

### 11.2 Desirable Difficulties (Bjork & Bjork 1992/2011)

The New Theory of Disuse defines two independent strength dimensions:

| Dimension | NTD Concept | Roko Analog |
|---|---|---|
| **Storage Strength (SS)** | How deeply embedded; only increases | Knowledge tier (Transient → Persistent) |
| **Retrieval Strength (RS)** | How accessible right now; decays | `Decay::apply()` output |

The critical interaction: **gains in SS are a decreasing function of RS**. The harder it is
to retrieve something (low RS), the bigger the boost to SS from a successful retrieval.

This directly justifies Roko's tier promotion rules: knowledge that is about to decay
(low RS) but is successfully retrieved (gate pass after knowledge retrieval) deserves a
larger promotion boost than knowledge that was easily accessible.

### 11.3 Adaptive Strength Algorithm

```rust
/// Adaptive decay: updates Ebbinghaus strength based on retrieval success.
/// Implements Bjork's desirable difficulty principle.
pub fn update_strength_on_retrieval(
    current_strength: f32,
    age_at_retrieval_ms: i64,
    scale_ms: u64,
    retrieval_success: bool,
) -> f32 {
    // Compute current retrievability at retrieval time
    let retrievability = Decay::Ebbinghaus { strength: current_strength, scale_ms }
        .apply(age_at_retrieval_ms);

    if retrieval_success {
        // Desirable difficulty: lower retrievability → bigger strength boost
        let difficulty_bonus = 1.0 - retrievability; // [0, 1]
        let boost = 0.1 + 0.3 * difficulty_bonus;    // [0.1, 0.4]
        (current_strength + boost).min(10.0)
    } else {
        // Failed retrieval: reduce strength (but not below minimum)
        (current_strength * 0.8).max(0.05)
    }
}
```

| Parameter | Default | Range | Description |
|---|---|---|---|
| `strength_boost_min` | 0.1 | 0.01 - 0.5 | Minimum strength increase on successful retrieval |
| `strength_boost_max` | 0.4 | 0.1 - 1.0 | Maximum strength increase (at lowest retrievability) |
| `strength_decay_factor` | 0.8 | 0.5 - 0.95 | Multiplicative strength reduction on failed retrieval |
| `strength_floor` | 0.05 | 0.01 - 0.5 | Minimum strength (prevents instant pruning) |

---

## 12. Decay Interaction Effects

### 12.1 Spreading Activation (Collins & Loftus 1975)

In associative memory networks, accessing one memory temporarily boosts activation of
associated memories. Accessing an Engram should slow the effective "forgetting" of its
associates — their activation stays elevated via lateral activation from the lineage DAG.

### 12.2 Retrieval-Induced Forgetting (Anderson et al. 1994)

Accessing one memory can **suppress** competing memories. When a knowledge entry is
retrieved via a query that also activated competitors, the competitors' retrieval strength
is temporarily inhibited. This means:

- Accessing a specific heuristic may make competing heuristics harder to find
- The effect is strongest for high-strength competitors (strong associations)
- The inhibition is temporary (~seconds in biological memory; configurable in Roko)

### 12.3 Implementation: Lineage-Based Decay Interaction

```rust
/// Adjust effective weight of related Engrams after a retrieval event.
/// Implements spreading activation (boost) + retrieval-induced forgetting (inhibit).
pub fn apply_retrieval_interaction(
    retrieved: &Signal,
    substrate: &dyn Substrate,
    ctx: &Context,
) -> Vec<(ContentHash, f32)> {
    let mut adjustments = Vec::new();

    // Boost: lineage parents get activation boost (spreading activation)
    for parent_id in &retrieved.lineage {
        adjustments.push((*parent_id, 0.05)); // +5% temporary boost
    }

    // Inhibit: same-Kind siblings get temporary suppression (RIF)
    // Only applies to siblings that share a parent with the retrieved Engram
    // and were activated (returned in the same query) but not selected.
    // Implementation: tag recently-activated non-selected Engrams for
    // temporary weight reduction in the next query cycle.

    adjustments
}
```

### 12.4 The Rational Analysis of Decay (Anderson & Schooler 1991)

The base-level activation equation from ACT-R:

```
B_i = ln( Σⱼ tⱼ^{-d} )
```

where t_j is the time since the j-th past use, d ≈ 0.5 is the decay parameter. This formula
treats decay as **rational discounting of evidence**: older evidence for future need is worth
less. Environmental statistics (word frequencies in NY Times, email reappearance rates) follow
the same power-law structure, meaning the memory system is adapted to its environment.

Roko's Ebbinghaus decay is a single-encounter approximation of this formula. The adaptive
strength algorithm (Section 11.3) makes it multi-encounter by boosting strength on each
successful retrieval.

---

## 13. Multi-Store Memory Models and the 4-Tier System

Roko's knowledge tiers (Transient → Working → Consolidated → Persistent) map onto
established multi-store memory models:

| Tier | Atkinson-Shiffrin (1968) | Baddeley (2000) | Cowan (1999) |
|---|---|---|---|
| **Transient** | Sensory registers | — | Activated LTM (outer layer) |
| **Working** | Short-term store | Phonological loop + visuospatial sketchpad | Focus of attention (~4 chunks) |
| **Consolidated** | Transfer to LTS | Episodic buffer | — |
| **Persistent** | Long-term store | — | Baseline LTM |

Cowan's embedded-processes model is most relevant: tiers are **not separate stores** but
**activation states within a single Substrate**. An Engram at the Transient tier has high
activation that decays rapidly. Promotion to Working means the Engram's strength has been
reinforced. This is a graduated activation model, not a transfer between containers.

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Ebbinghaus 1885, Über das Gedächtnis | Discovered the forgetting curve. |
| Grassé 1959 | Stigmergy: environmental signals that decay. |
| Parunak et al. 2007 | Digital pheromones with configurable evaporation rates. |
| Wixted 2004, Annual Review of Psychology 55 | Modern synthesis: power-law as consolidation-based interference. |
| Averell & Heathcote 2011, J. Math. Psych. 55(1) | Individual exponential, population power-law; permanent asymptote. |
| Murre & Dros 2015, PLOS ONE 10(7) | Ebbinghaus replication confirming 24-hour sleep discontinuity. |
| Anderson & Schooler 1991, Psych. Science 2(6) | Rational analysis of memory: environmental statistics match decay. |
| Collins & Loftus 1975, Psych. Review 82(6) | Spreading activation theory of semantic processing. |
| Anderson et al. 1994, J. Exp. Psych. LMC 20(5) | Retrieval-induced forgetting: accessing one memory suppresses competitors. |
| Bjork & Bjork 1992/2011 | New Theory of Disuse: storage strength vs. retrieval strength. |
| Ye 2022, KDD | FSRS: adaptive spaced repetition with stability/retrievability/difficulty. |
| Atkinson & Shiffrin 1968 | Modal model: sensory registers → STS → LTS. |
| Baddeley 2000, Trends in Cognitive Sciences 4(11) | Working memory: episodic buffer as integration zone. |
| Cowan 1999, in *Models of Working Memory*, CUP | Embedded-processes: tiers as activation states, not separate stores. |

---

## Current Status and Gaps

- **Implemented**: All four Decay variants (None, HalfLife, Ttl, Ebbinghaus) with `apply()`,
  `is_alive()`, and pheromone constants. Fully tested in `roko-core`.
- **Implemented**: Integration with Score via `weight_at()` on the Engram.
- **Implemented**: Substrate pruning via `prune()` threshold.
- **Gap**: No adaptive decay — strength parameters are static. Future: Dreams consolidation
  should promote knowledge tier, increasing Ebbinghaus strength.

---

## Cross-References

- [02-engram-data-type.md](02-engram-data-type.md) — Decay as a field on the Engram
- [03-score-7-axis-appraisal.md](03-score-7-axis-appraisal.md) — How Score and Decay combine
- [07-substrate-trait.md](07-substrate-trait.md) — Substrate.prune() uses decay
- [13-cognitive-cross-cuts.md](13-cognitive-cross-cuts.md) — Knowledge tier decay in Neuro
