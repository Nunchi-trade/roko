# Decay x Knowledge Tier matrix

> Layer 0 Kernel -- Engram Lifecycle
> Status: **Specification** -- parameters ready for implementation
> Canonical source: `crates/roko-core/src/decay.rs`, `crates/roko-neuro/` (planned)
> Cross-references: [04-decay-variants.md](04-decay-variants.md), [13-cognitive-cross-cuts.md](13-cognitive-cross-cuts.md)


> **Implementation**: Shipping

---

## Purpose

Every knowledge entry in the Neuro subsystem carries two orthogonal properties:

1. **Decay variant** -- how the weight diminishes over time (None, HalfLife, Ttl, Ebbinghaus)
2. **Knowledge tier** -- how well-established the knowledge is (Transient, Working, Consolidated, Persistent)

This document specifies the full 4x4 matrix of decay variant x knowledge tier, defines the parameters for each cell, and describes promotion/demotion rules that move knowledge between tiers.

---

## 1. The 4x4 matrix

Each cell shows the `Decay` configuration and effective half-life (time to reach 50% weight).

| | **Transient** (0.1x) | **Working** (0.5x) | **Consolidated** (1.0x) | **Persistent** (5.0x) |
|---|---|---|---|---|
| **None** | weight = 1.0 forever | weight = 1.0 forever | weight = 1.0 forever | weight = 1.0 forever |
| **HalfLife** | hl = 7,200,000 (2h) | hl = 43,200,000 (12h) | hl = 86,400,000 (24h) | hl = 604,800,000 (7d) |
| **Ttl** | ttl = 3,600,000 (1h) | ttl = 14,400,000 (4h) | ttl = 86,400,000 (24h) | ttl = 604,800,000 (7d) |
| **Ebbinghaus** | s=0.1, sc=3,600,000 | s=0.5, sc=3,600,000 | s=1.0, sc=86,400,000 | s=5.0, sc=86,400,000 |

Where `s` = strength, `sc` = scale_ms, `hl` = half_life_ms, `ttl` = ttl_ms.

### Ebbinghaus effective half-lives

The Ebbinghaus curve `weight = exp(-age / (strength * scale_ms))` reaches 50% at `age = strength * scale_ms * ln(2)`:

| Tier | Strength | Scale | Effective half-life |
|---|---|---|---|
| Transient | 0.1 | 3,600,000 (1h) | ~4.2 minutes |
| Working | 0.5 | 3,600,000 (1h) | ~20.8 minutes |
| Consolidated | 1.0 | 86,400,000 (24h) | ~16.6 hours |
| Persistent | 5.0 | 86,400,000 (24h) | ~3.5 days |

---

## 2. Tier definitions

```rust
/// Knowledge tier determines retention characteristics.
/// Higher tiers have stronger decay resistance and wider retrieval reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum KnowledgeTier {
    /// First observation. Fast decay. Not yet validated.
    Transient,
    /// Referenced in 2+ successful episodes. Moderate decay.
    Working,
    /// Validated by gate verdicts or prediction outcomes. Standard decay.
    Consolidated,
    /// Repeatedly validated across multiple sessions. Very slow decay.
    Persistent,
}

impl KnowledgeTier {
    /// Strength multiplier for Ebbinghaus decay.
    pub fn strength_multiplier(&self) -> f32 {
        match self {
            Self::Transient => 0.1,
            Self::Working => 0.5,
            Self::Consolidated => 1.0,
            Self::Persistent => 5.0,
        }
    }

    /// Default prune threshold for this tier.
    pub fn prune_threshold(&self) -> f32 {
        match self {
            Self::Transient => 0.05,
            Self::Working => 0.02,
            Self::Consolidated => 0.01,
            Self::Persistent => 0.005,
        }
    }
}
```

---

## 3. Decay variant selection by knowledge type

Not every knowledge type uses every decay variant. The recommended pairing:

| Knowledge type | Decay variant | Rationale |
|---|---|---|
| **Insight** | Ebbinghaus | Insights strengthen with validation, weaken without rehearsal |
| **Heuristic** | Ebbinghaus | Procedural rules follow the forgetting curve |
| **Warning** | HalfLife | Warnings need urgency; exponential decay matches threat pheromones |
| **CausalLink** | Ebbinghaus | Causal relationships are memory-like |
| **StrategyFragment** | Ebbinghaus | Strategies strengthen with reuse |
| **AntiKnowledge** | None | Falsified knowledge must persist to prevent re-exploration |

AntiKnowledge always uses `Decay::None` regardless of tier. The system must remember what failed.

---

## 4. Promotion rules

Promotion moves knowledge up one tier. It happens during the Dreams consolidation cycle (Delta frequency) or inline when a gate verdict confirms a knowledge entry's prediction.

```
Promotion criteria:
  Transient -> Working:
    - Referenced in >= 2 successful episodes (gate pass after knowledge retrieval)
    - OR explicitly endorsed by a human review
    - Time in tier: >= 5 minutes (prevent instant promotion of noise)

  Working -> Consolidated:
    - Referenced in >= 5 successful episodes across >= 2 distinct plans
    - OR validated by a gate verdict that confirms the knowledge's prediction
    - Time in tier: >= 1 hour

  Consolidated -> Persistent:
    - Referenced in >= 10 successful episodes across >= 3 distinct sessions
    - AND zero contradicting episodes in the last 24 hours
    - Time in tier: >= 24 hours
```

### Promotion algorithm

```
fn try_promote(entry: &mut KnowledgeEntry, stats: &UsageStats) -> bool {
    let current = entry.tier;
    let target = match current {
        Transient if stats.successful_refs >= 2
            && entry.age() >= Duration::minutes(5) => Working,
        Working if stats.successful_refs >= 5
            && stats.distinct_plans >= 2
            && entry.age_in_tier() >= Duration::hours(1) => Consolidated,
        Consolidated if stats.successful_refs >= 10
            && stats.distinct_sessions >= 3
            && stats.contradictions_last_24h == 0
            && entry.age_in_tier() >= Duration::hours(24) => Persistent,
        _ => return false,
    };

    entry.tier = target;
    entry.decay = entry.decay.with_strength(target.strength_multiplier());
    entry.promoted_at = now();
    true
}
```

---

## 5. Demotion rules

Demotion moves knowledge down one tier. It triggers when knowledge fails validation or ages without use.

```
Demotion criteria:
  Persistent -> Consolidated:
    - Referenced in a failed episode (gate fail after knowledge retrieval)
    - OR no references in the last 7 days

  Consolidated -> Working:
    - Referenced in >= 2 failed episodes within 24 hours
    - OR no references in the last 72 hours

  Working -> Transient:
    - Referenced in >= 3 failed episodes
    - OR no references in the last 24 hours

  Transient -> [pruned]:
    - Weight falls below prune threshold (0.05)
    - Handled by Substrate.prune(), not explicit demotion
```

### Demotion algorithm

```
fn try_demote(entry: &mut KnowledgeEntry, stats: &UsageStats) -> bool {
    let current = entry.tier;
    let target = match current {
        Persistent if stats.failed_refs_last_24h >= 1
            || stats.time_since_last_ref >= Duration::days(7) => Consolidated,
        Consolidated if stats.failed_refs_last_24h >= 2
            || stats.time_since_last_ref >= Duration::hours(72) => Working,
        Working if stats.total_failed_refs >= 3
            || stats.time_since_last_ref >= Duration::hours(24) => Transient,
        _ => return false,
    };

    entry.tier = target;
    entry.decay = entry.decay.with_strength(target.strength_multiplier());
    entry.demoted_at = now();
    true
}
```

---

## 6. Worked examples

### Example 1: Insight promotion chain

An agent discovers that "this codebase uses builder pattern extensively."

```
t=0:   Insight created at Transient tier.
       Decay: Ebbinghaus { strength: 0.1, scale_ms: 3_600_000 }
       Effective half-life: ~4.2 minutes.

t=8m:  Agent retrieves this insight while working on task #2. Task passes gate.
       successful_refs = 1. Not enough for promotion.

t=15m: Agent retrieves insight during task #5. Task passes gate.
       successful_refs = 2. Age >= 5 minutes. PROMOTE to Working.
       Decay: Ebbinghaus { strength: 0.5, scale_ms: 3_600_000 }
       Effective half-life: ~20.8 minutes.

t=3h:  Across plans A and B, insight referenced in 5 successful episodes.
       distinct_plans = 2. Age in tier >= 1 hour. PROMOTE to Consolidated.
       Decay: Ebbinghaus { strength: 1.0, scale_ms: 86_400_000 }
       Effective half-life: ~16.6 hours.

t=48h: Across 3 sessions, insight referenced in 12 successful episodes.
       No contradictions in last 24h. PROMOTE to Persistent.
       Decay: Ebbinghaus { strength: 5.0, scale_ms: 86_400_000 }
       Effective half-life: ~3.5 days.
```

### Example 2: Warning demotion

A warning states "never run tests without --release flag."

```
t=0:   Warning created at Consolidated tier (human-endorsed).
       Decay: HalfLife { half_life_ms: 86_400_000 } (24h)

t=26h: A task runs tests without --release and passes anyway.
       failed_refs_last_24h = 1. Not enough for demotion (need 2).

t=27h: Another task runs tests without --release and passes.
       failed_refs_last_24h = 2. DEMOTE to Working.
       Decay: HalfLife { half_life_ms: 43_200_000 } (12h)

t=28h: Third failure. total_failed_refs = 3. DEMOTE to Transient.
       Decay: HalfLife { half_life_ms: 7_200_000 } (2h)

t=30h: Weight drops below 0.05 threshold. Pruned by Substrate.prune().
```

### Example 3: AntiKnowledge persistence

A hypothesis "alloy requires nightly Rust" is tested and disproved.

```
t=0:   AntiKnowledge created at Persistent tier.
       Decay: None (permanent).
       Weight = 1.0 at all times.

t=30d: Still at Persistent, weight = 1.0.
       Future agents querying "alloy Rust requirements" retrieve
       this entry and skip the nightly hypothesis.
```

---

## 7. Configuration parameters

| Parameter | Default | Range | Where |
|---|---|---|---|
| `transient_strength` | 0.1 | 0.01 - 0.5 | `KnowledgeTier::strength_multiplier()` |
| `working_strength` | 0.5 | 0.1 - 2.0 | `KnowledgeTier::strength_multiplier()` |
| `consolidated_strength` | 1.0 | 0.5 - 5.0 | `KnowledgeTier::strength_multiplier()` |
| `persistent_strength` | 5.0 | 2.0 - 50.0 | `KnowledgeTier::strength_multiplier()` |
| `transient_prune_threshold` | 0.05 | 0.01 - 0.2 | `KnowledgeTier::prune_threshold()` |
| `promote_min_refs` | [2, 5, 10] | 1 - 50 each | Promotion rules per tier |
| `demote_fail_threshold` | [3, 2, 1] | 1 - 10 each | Demotion rules per tier |
| `demote_idle_hours` | [24, 72, 168] | 1 - 720 each | Idle demotion per tier |
| `ebbinghaus_base_scale_ms` | 3,600,000 | 60,000 - 86,400,000 | Low-tier scale |
| `ebbinghaus_high_scale_ms` | 86,400,000 | 3,600,000 - 604,800,000 | High-tier scale |

---

## 8. Integration wiring

The decay-tier matrix integrates at three points:

1. **Knowledge creation** (`NeuroStore::insert`): assigns initial tier (Transient) and corresponding decay parameters.
2. **Dreams consolidation** (`DreamsEngine::consolidate`): runs promotion/demotion checks during Delta cycle. Reads usage stats from episode log.
3. **Substrate pruning** (`Substrate::prune`): removes entries whose weight falls below the tier's prune threshold.

```
Signal flow:
  Agent retrieves knowledge -> episode logged with knowledge IDs
      |
      v
  Gate verdict (pass/fail) -> episode updated with outcome
      |
      v
  Dreams cycle (Delta frequency):
      - Scan episodes since last consolidation
      - For each referenced knowledge entry:
          - Compute usage stats (successful_refs, failed_refs, distinct_plans)
          - Run try_promote() and try_demote()
          - Update decay parameters if tier changed
      - Run Substrate.prune() with tier-specific thresholds
```

---

## 9. Error handling

| Condition | Response |
|---|---|
| Strength multiplier is zero or negative | Clamp to 0.01; log warning |
| Scale_ms is zero | Treat as instant decay (weight = 0.0 at any positive age) |
| Promotion and demotion both triggered | Demotion wins (failure evidence is stronger than success count) |
| Clock skew produces negative age | Return weight = 1.0 (see `Decay::apply`) |
| NaN from exp() overflow | Clamp to 0.0; entry eligible for pruning |

---

## 10. Test criteria

1. Promotion from Transient to Persistent with exact reference counts at each boundary.
2. Demotion from Persistent to pruned with exact failure counts.
3. AntiKnowledge at Persistent tier with `Decay::None` maintains weight = 1.0 after 30 days.
4. Ebbinghaus effective half-life matches the formula `strength * scale_ms * ln(2)` within 1% tolerance.
5. Concurrent promotion + demotion resolves to demotion.
6. Zero half-life produces weight = 0.0 at any positive age.
7. Round-trip serialization of all 16 matrix cells preserves parameters.

---

## Cross-references

- [04-decay-variants.md](04-decay-variants.md) -- Decay enum specification and formulas
- [13-cognitive-cross-cuts.md](13-cognitive-cross-cuts.md) -- Neuro knowledge tiers
- [02-engram-data-type.md](02-engram-data-type.md) -- Decay as a field on Signal
- [03-score-7-axis-appraisal.md](03-score-7-axis-appraisal.md) -- How Score and Decay combine
- `crates/roko-core/src/decay.rs` -- Decay enum implementation
- `crates/roko-neuro/` -- NeuroStore (planned integration target)
