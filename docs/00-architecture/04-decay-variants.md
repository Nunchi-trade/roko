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

## Academic Foundations

| Citation | Contribution |
|---|---|
| Ebbinghaus 1885, Über das Gedächtnis | Discovered the forgetting curve: memory decays exponentially with rehearsal-dependent strength. |
| Grassé 1959 | Stigmergy: indirect coordination via environmental signals that decay. Foundation for pheromone half-lives. |
| Parunak et al. 2007, Mechanisms and Methods in Multi-Agent Systems | Digital pheromones with configurable evaporation rates for multi-agent coordination. |
| Wixted & Ebbesen 1991, Journal of Experimental Psychology | Empirical validation of the exponential forgetting curve across multiple memory tasks. |
| Murre & Dros 2015, PLOS ONE 10(7) | Replicated Ebbinghaus' original experiments, confirming the exponential decay model. |

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
