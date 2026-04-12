# Ebbinghaus Decay with Tier Multipliers

> Knowledge entries decay exponentially following the Ebbinghaus forgetting curve, with the effective half-life determined by the multiplicative composition of type base half-life and tier multiplier.


> **Implementation**: Built

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [02-four-validation-tiers.md](./02-four-validation-tiers.md), [03-type-half-lives.md](./03-type-half-lives.md)
**Key sources**:
- `bardo-backup/prd/04-memory/00-overview.md` (Ebbinghaus theory, forgetting curve equations)
- `refactoring-prd/03-cognitive-subsystems.md` §1 (effective_decay formula)
- `crates/roko-neuro/src/lib.rs` (half_life_days field, default_half_life_days)
- `crates/roko-neuro/src/knowledge_store.rs` (decay method, GC threshold)

---

## Abstract

Every knowledge entry in Neuro decays over time unless it is reinforced through successful use. This decay follows the Ebbinghaus forgetting curve (1885) — an exponential model that has been replicated and validated across more than a century of memory research (Murre & Dros 2015; Wixted & Ebbesen 1991). The model captures a fundamental property of memory: most forgetting happens early, with the rate slowing as time progresses.

In Neuro, the decay rate is controlled by two independent dimensions — the knowledge **type** (Insight, Heuristic, Warning, etc.) determines a base half-life, and the validation **tier** (Transient, Working, Consolidated, Persistent) determines a multiplier. These compose multiplicatively into an effective half-life that governs the entry's decay trajectory. A Transient Warning decays with a 17-hour effective half-life, while a Persistent Fact decays with a 1,825-day (5-year) effective half-life.

---

## The Forgetting Curve Formula

### Base Decay Equation

```
weight(entry, t) = 2^(-age_days / effective_half_life)
```

Equivalently, using natural logarithm:

```
weight(entry, t) = exp(-age_days × ln(2) / effective_half_life)
```

Where:
- `age_days` = (now − entry.created_at) in days
- `effective_half_life` = tier_multiplier × type_base_half_life

At time `t = effective_half_life`, the weight equals exactly 0.5 (half strength).

### Effective Half-Life Composition

```
effective_half_life = tier_multiplier × type_base_half_life
```

**Type base half-lives** (from [03-type-half-lives.md](./03-type-half-lives.md)):

| Type | Base Half-Life |
|---|---|
| Warning | 7 days |
| StrategyFragment | 14 days |
| Insight | 30 days |
| CausalLink | 60 days |
| Heuristic | 90 days |
| Fact | 365 days |
| AntiKnowledge | ∞ (floor 0.3) |

**Tier multipliers** (from [02-four-validation-tiers.md](./02-four-validation-tiers.md)):

| Tier | Multiplier |
|---|---|
| Transient | 0.1× |
| Working | 0.5× |
| Consolidated | 1.0× |
| Persistent | 5.0× |

### Worked Examples

**Example 1: Transient Warning (effective half-life = 0.1 × 7 = 0.7 days = 16.8 hours)**

| Age | Weight | Interpretation |
|---|---|---|
| 0 hours | 1.000 | Fresh — full confidence |
| 4 hours | 0.841 | Still strong |
| 12 hours | 0.546 | Half-strength approaching |
| 16.8 hours | 0.500 | Half-life point |
| 24 hours | 0.370 | One day old — significantly degraded |
| 3 days | 0.050 | GC threshold reached |

An unvalidated warning is essentially worthless after 3 days unless reinforced.

**Example 2: Working Insight (effective half-life = 0.5 × 30 = 15 days)**

| Age | Weight | Interpretation |
|---|---|---|
| 0 days | 1.000 | Fresh |
| 7 days | 0.707 | One week — still useful |
| 15 days | 0.500 | Half-life |
| 30 days | 0.250 | One month — fading |
| 60 days | 0.063 | Two months — nearly gone |
| 65 days | 0.050 | GC threshold reached |

A Working-tier Insight lasts about two months before GC.

**Example 3: Consolidated Heuristic (effective half-life = 1.0 × 90 = 90 days)**

| Age | Weight | Interpretation |
|---|---|---|
| 0 days | 1.000 | Fresh |
| 30 days | 0.794 | One month |
| 90 days | 0.500 | Half-life (three months) |
| 180 days | 0.250 | Six months |
| 365 days | 0.063 | One year — approaching GC |
| 387 days | 0.050 | GC threshold |

A validated Heuristic persists for over a year.

**Example 4: Persistent Fact (effective half-life = 5.0 × 365 = 1,825 days)**

| Age | Weight | Interpretation |
|---|---|---|
| 1 year | 0.870 | Still strong |
| 2 years | 0.757 | Slowly declining |
| 5 years | 0.500 | Half-life |
| 10 years | 0.250 | |
| 21 years | 0.050 | GC threshold |

A Persistent Fact is essentially permanent for operational purposes.

---

## Reinforcement and Strengthening

### Confirmation Boost

When an agent retrieves a knowledge entry, uses it in a task, and the subsequent gate check passes, the entry receives a **confirmation boost**:

```rust
// From roko-neuro/src/knowledge_store.rs
pub const CONFIRMATION_BOOST: f64 = 1.5;

// Applied as: entry.confidence *= CONFIRMATION_BOOST;
// Clamped to [0.0, 1.0]
```

The boost is applied to the entry's **confidence** score, not to its half-life. Confidence and decay are separate dimensions:
- **Confidence** (0.0–1.0) determines retrieval priority — higher confidence entries are retrieved first
- **Weight** (from decay) determines temporal relevance — newer entries have higher weight

Both factors are combined during retrieval scoring:

```
retrieval_score = confidence × weight(age, effective_half_life)
```

### Spacing Effect

The Ebbinghaus spacing effect predicts that spaced retrievals strengthen memory more than massed retrievals. In Neuro, this is implemented through the tier promotion system: each successful use counts as one confirmation toward tier promotion, but the confirmations must come from **distinct episodes** (not the same task repeated). This naturally encourages spaced retrieval over massed repetition.

---

## Garbage Collection

### GC Threshold

Entries whose confidence-weighted decay score falls below `DEFAULT_GC_MIN_CONFIDENCE` are removed:

```rust
pub const DEFAULT_GC_MIN_CONFIDENCE: f64 = 0.05;
```

The number of half-lives to reach the GC threshold:

```
2^(-n) = 0.05
n = -log2(0.05) ≈ 4.32 half-lives
```

So an entry survives for approximately **4.3 effective half-lives** before GC removes it.

### GC Schedule by Type × Tier

| Type \ Tier | Transient | Working | Consolidated | Persistent |
|---|---|---|---|---|
| **Warning** (7d) | 3 days | 15 days | 30 days | 151 days |
| **StrategyFragment** (14d) | 6 days | 30 days | 60 days | 301 days |
| **Insight** (30d) | 13 days | 65 days | 129 days | 646 days |
| **CausalLink** (60d) | 26 days | 129 days | 258 days | 1,293 days |
| **Heuristic** (90d) | 39 days | 194 days | 387 days | 1,939 days |
| **Fact** (365d) | 157 days | 787 days | 1,573 days | 7,865 days |

**AntiKnowledge**: Exempt from GC; confidence floor of 0.3 prevents decay below GC threshold.

### GC Implementation

The current `KnowledgeStore.gc()` method scans all entries and removes those below the threshold:

```rust
// Simplified from roko-neuro/src/knowledge_store.rs
pub fn gc(&mut self, min_confidence: f64) -> Result<usize> {
    let before = self.entries.len();
    self.entries.retain(|entry| entry.confidence >= min_confidence);
    Ok(before - self.entries.len())
}
```

GC runs periodically (triggered by the orchestrator after each task completion) and is also available as a manual operation.

---

## Decay vs. Neural Network Embedding Drift

A notable advantage of Ebbinghaus decay over neural network embedding-based retrieval: embeddings from neural models can **silently drift** when the model is updated, changing similarity scores without any visible signal. Neuro's decay is **explicit and deterministic** — the decay rate is a known function of time, type, and tier. There is no hidden model state that can change the behavior of the knowledge base.

This property is important for auditability: given an entry's creation time, type, and tier, the decay weight at any past or future time can be exactly computed. This supports the forensic AI capability (see topic [00-architecture](../00-architecture/INDEX.md)) — replaying an agent's decision requires knowing exactly what knowledge was available and at what weight at the time of the decision.

---

## Academic Foundations

- Ebbinghaus, H. (1885). *Über das Gedächtnis*. Leipzig: Duncker & Humblot.
- Murre, J. M. J., & Dros, J. (2015). "Replication and Analysis of Ebbinghaus' Forgetting Curve." *PLOS ONE*, 10(7), e0120644.
- Wixted, J. T., & Ebbesen, E. B. (1991). "On the Form of Forgetting." *Psychological Science*, 2(6), 409–415.
- Pimsleur, P. (1967). "A Memory Schedule." *Modern Language Journal*, 51(2), 73–75. (Graduated interval recall)
- McClelland, J. L., McNaughton, B. L., & O'Reilly, R. C. (1995). "Complementary learning systems." *Psychological Review*, 102(3). (Fast vs. slow learning)

---

## Current Status and Gaps

**Implemented**:
- `half_life_days` field on `KnowledgeEntry` (set from type defaults)
- `FACT_HALF_LIFE_DAYS`, `INSIGHT_HALF_LIFE_DAYS`, `HEURISTIC_HALF_LIFE_DAYS` constants
- `KnowledgeStore.decay()` method
- `KnowledgeStore.gc()` with `DEFAULT_GC_MIN_CONFIDENCE = 0.05`
- `CONFIRMATION_BOOST = 1.5`

**Missing**:
- Tier multiplier on `KnowledgeEntry` (not yet a field)
- Effective half-life computation as `tier_multiplier × type_base_half_life`
- Combined retrieval score as `confidence × decay_weight`
- AntiKnowledge exemption from GC (confidence floor 0.3)
- Spacing effect enforcement (distinct episode requirement for confirmations)

---

## Cross-references

- See [02-four-validation-tiers.md](./02-four-validation-tiers.md) for tier multiplier details
- See [03-type-half-lives.md](./03-type-half-lives.md) for base half-life rationale
- See [10-knowledge-query-api.md](./10-knowledge-query-api.md) for how decay affects retrieval scoring
- See [12-4-tier-distillation-pipeline.md](./12-4-tier-distillation-pipeline.md) for how confirmation drives tier promotion
