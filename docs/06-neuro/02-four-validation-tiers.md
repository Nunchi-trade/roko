# Four Validation Tiers

> Knowledge reliability is tracked through four validation tiers — Transient, Working, Consolidated, Persistent — each with a multiplicative effect on the base half-life of the knowledge type.

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [01-six-knowledge-types.md](./01-six-knowledge-types.md) for knowledge type definitions
**Key sources**:
- `refactoring-prd/03-cognitive-subsystems.md` §1 (Tier Progression table)
- `bardo-backup/prd/04-memory/00-overview.md` (CLS theory, tiered memory)
- `bardo-backup/prd/04-memory/01-grimoire.md` (A-MAC admission gate)
- `crates/roko-neuro/src/tier_progression.rs` (current implementation)
- `crates/roko-neuro/src/knowledge_store.rs` (CONFIRMATION_BOOST, GC threshold)

---

## Abstract

Not all knowledge is equally reliable. A freshly extracted observation is less trustworthy than a heuristic that has been validated across dozens of episodes. Neuro tracks this reliability through a four-tier validation system that is orthogonal to the knowledge type system. Every knowledge entry has both a **type** (what kind of knowledge it is) and a **tier** (how validated that knowledge is).

The four tiers — Transient, Working, Consolidated, Persistent — form a progression from unvalidated to core knowledge. Each tier carries a **multiplier** that scales the type's base half-life: a Transient entry decays 10× faster than its base rate, while a Persistent entry decays 5× slower. This two-dimensional decay model (type × tier) ensures that unreliable knowledge disappears quickly while proven knowledge persists.

Tier transitions are driven by **outcome feedback**: successful use of a knowledge entry promotes its tier (increasing the multiplier), while unsuccessful use demotes it. This creates a natural selection pressure where useful knowledge rises to Persistent and unreliable knowledge decays to nothing. The system implements a computational analogue of Complementary Learning Systems theory (McClelland et al. 1995), where fast-learning episodic memory consolidates into slow, durable semantic memory through repeated successful retrieval.

---

## The Four Tiers

### Transient (Multiplier: 0.1×)

**Definition**: Just extracted, unvalidated. The entry has been distilled from a single episode or small cluster of episodes but has not yet been used or cross-validated.

**Effective half-life examples**:
| Type | Base Half-Life | × Transient (0.1×) | Effective |
|---|---|---|---|
| Insight | 30 days | × 0.1 | **3 days** |
| Heuristic | 90 days | × 0.1 | **9 days** |
| Warning | 7 days | × 0.1 | **0.7 days (17 hours)** |
| CausalLink | 60 days | × 0.1 | **6 days** |
| StrategyFragment | 14 days | × 0.1 | **1.4 days** |
| Fact | 365 days | × 0.1 | **36.5 days** |

**Entry criteria**: All newly distilled knowledge entries start at Transient tier. Entries restored from backup also start at Transient (they must re-prove themselves in the new context).

**Promotion criteria**: Used once successfully — the agent retrieved this entry, acted on it, and the subsequent gate check passed. One positive outcome is enough to promote from Transient to Working.

**Demotion criteria**: None (Transient is the lowest tier). Entries that are never used simply decay and are eventually garbage-collected when their confidence falls below `DEFAULT_GC_MIN_CONFIDENCE` (0.05 in the current implementation).

**Rationale**: The 0.1× multiplier creates a strong filter. A Transient Insight has only 3 days of effective half-life — if it is not used and confirmed within that window, it will decay rapidly. This prevents the knowledge base from filling with speculative observations that were never validated. The harsh decay rate mirrors the hippocampus's rapid forgetting of unconsolidated episodic memories (McClelland et al. 1995).

### Working (Multiplier: 0.5×)

**Definition**: Used once and confirmed. The entry has been retrieved during a task, the agent acted on it, and the outcome was positive (gate check passed). It has demonstrated utility in at least one context.

**Effective half-life examples**:
| Type | Base Half-Life | × Working (0.5×) | Effective |
|---|---|---|---|
| Insight | 30 days | × 0.5 | **15 days** |
| Heuristic | 90 days | × 0.5 | **45 days** |
| Warning | 7 days | × 0.5 | **3.5 days** |
| CausalLink | 60 days | × 0.5 | **30 days** |
| StrategyFragment | 14 days | × 0.5 | **7 days** |
| Fact | 365 days | × 0.5 | **182.5 days** |

**Entry criteria**: Promoted from Transient after one successful use.

**Promotion criteria**: Used 3+ times successfully and cross-validated (either by multiple independent uses or by confirmation from another agent in the mesh). Three is the minimum support threshold for the transition from Working to Consolidated.

**Demotion criteria**: Led to a bad outcome. If the agent retrieved this entry, acted on it, and the gate check failed, the entry is demoted back to Transient. A single negative outcome is enough for demotion — the asymmetry between promotion (requires 3+ successes) and demotion (requires 1 failure) reflects the principle that negative evidence should be weighted more heavily than positive evidence for knowledge reliability.

**Rationale**: Working tier is the standard operating tier for most knowledge. With a 0.5× multiplier, entries have moderate durability — enough to be useful across multiple episodes but not so persistent that stale entries linger. The 3+ use threshold for promotion ensures that only repeatedly useful knowledge advances to Consolidated.

### Consolidated (Multiplier: 1.0×)

**Definition**: Validated through repeated use and cross-validation. The entry has been used 3+ times with positive outcomes and has been cross-validated (either by multiple independent uses in different contexts or by confirmation from another agent).

**Effective half-life examples**:
| Type | Base Half-Life | × Consolidated (1.0×) | Effective |
|---|---|---|---|
| Insight | 30 days | × 1.0 | **30 days** |
| Heuristic | 90 days | × 1.0 | **90 days** |
| Warning | 7 days | × 1.0 | **7 days** |
| CausalLink | 60 days | × 1.0 | **60 days** |
| StrategyFragment | 14 days | × 1.0 | **14 days** |
| Fact | 365 days | × 1.0 | **365 days** |

**Entry criteria**: Promoted from Working after 3+ successful uses with cross-validation.

**Promotion criteria**: Core knowledge with high reputation — used extensively, never contradicted, potentially confirmed by multiple agents. The specific threshold for Consolidated → Persistent is high: the entry must have been used successfully in 10+ episodes without a single negative outcome, or must have been independently confirmed by 3+ agents in the mesh.

**Demotion criteria**: Contradicted by newer evidence. If a newer entry (with higher recency) provides evidence against this Consolidated entry, it is demoted to Working for re-evaluation. The contradiction can come from:
- A gate failure where this entry was in the retrieved context
- An AntiKnowledge entry that specifically refutes this entry
- A conflicting entry confirmed by multiple agents

**Rationale**: Consolidated is the "normal" tier where the base half-life applies without modification (multiplier 1.0×). This is the target state for knowledge that has proven reliable — it decays at the rate appropriate for its type. A Consolidated Insight (30-day half-life) will naturally need revalidation monthly, while a Consolidated Fact (365-day half-life) persists for roughly a year.

### Persistent (Multiplier: 5.0×)

**Definition**: Core knowledge with high reputation. The entry has been used extensively, confirmed by multiple sources, and represents foundational understanding that the agent relies on regularly.

**Effective half-life examples**:
| Type | Base Half-Life | × Persistent (5.0×) | Effective |
|---|---|---|---|
| Insight | 30 days | × 5.0 | **150 days** |
| Heuristic | 90 days | × 5.0 | **450 days** |
| Warning | 7 days | × 5.0 | **35 days** |
| CausalLink | 60 days | × 5.0 | **300 days** |
| StrategyFragment | 14 days | × 5.0 | **70 days** |
| Fact | 365 days | × 5.0 | **1,825 days (5 years)** |

**Entry criteria**: Promoted from Consolidated after extensive successful use (10+ episodes) with no negative outcomes and/or multi-agent confirmation (3+ agents).

**Promotion criteria**: None — Persistent is the highest tier. Entries remain here as long as they are not explicitly deprecated.

**Demotion criteria**: Explicitly deprecated. Persistent entries are not automatically demoted by a single negative outcome (unlike lower tiers). They require explicit deprecation — either by a user command, by an overwhelming weight of AntiKnowledge evidence, or by a manual review process. The high bar for demotion reflects the fact that Persistent entries represent the agent's most reliable knowledge and should not be discarded lightly.

**Rationale**: The 5.0× multiplier creates extremely durable knowledge. A Persistent Fact has an effective half-life of 5 years — it will remain in the knowledge base essentially forever unless actively deprecated. A Persistent Heuristic (450-day half-life) persists for over a year. This durability is appropriate for core knowledge that the agent has validated extensively, but it also means that incorrect entries at this tier are dangerous — they resist correction. The explicit deprecation requirement is a safety mechanism.

---

## Tier Transition Mechanics

### Promotion Flow

```
Transient ──(1 success)──→ Working ──(3+ successes)──→ Consolidated ──(10+ successes)──→ Persistent
```

Each promotion requires that the agent:
1. **Retrieved** the knowledge entry as part of context assembly for a task
2. **Used** the entry (it was in the LLM's context when generating the response)
3. **Succeeded** (the subsequent gate check passed)

The promotion is tracked through the `KnowledgeConfirmationRecord` in the current implementation:

```rust
// From roko-neuro/src/knowledge_store.rs
pub struct KnowledgeConfirmationRecord {
    pub entry_id: String,
    pub confirmed: bool,
    pub episode_id: String,
    pub timestamp: DateTime<Utc>,
}
```

Each successful use creates a confirmation record. The tier progression system counts confirmations to determine promotion eligibility.

### Demotion Flow

```
Persistent ──(explicit deprecation)──→ Consolidated ──(contradicted)──→ Working ──(bad outcome)──→ Transient
```

Demotion rules differ by tier:
- **Working → Transient**: One negative outcome (gate failure where the entry was in context)
- **Consolidated → Working**: Contradicted by newer evidence or AntiKnowledge
- **Persistent → Consolidated**: Explicit deprecation only

The asymmetry is deliberate: promotion requires accumulating positive evidence, while demotion can happen from a single negative signal (except at Persistent tier). This mirrors the precautionary principle — it is safer to demote knowledge that may be wrong than to maintain confidence in unreliable entries.

### Confidence Boost on Confirmation

In the current implementation, each confirmation applies a `CONFIRMATION_BOOST` of 1.5× to the entry's confidence:

```rust
// From roko-neuro/src/knowledge_store.rs
pub const CONFIRMATION_BOOST: f64 = 1.5;
```

This boost is multiplicative: an entry with confidence 0.4 that is confirmed once becomes `0.4 × 1.5 = 0.6`. A second confirmation raises it to `0.6 × 1.5 = 0.9`. The confidence is clamped to the range [0.0, 1.0].

### Garbage Collection

Entries whose confidence falls below `DEFAULT_GC_MIN_CONFIDENCE` (0.05 in the current implementation) are removed by the garbage collector:

```rust
// From roko-neuro/src/knowledge_store.rs
pub const DEFAULT_GC_MIN_CONFIDENCE: f64 = 0.05;
```

This threshold ensures that entries which have decayed to near-zero confidence are cleaned up, preventing unbounded growth of the knowledge store. AntiKnowledge entries are exempt from GC below their confidence floor of 0.3.

---

## Two-Dimensional Decay: Type × Tier

The effective decay rate of a knowledge entry is determined by two independent dimensions:

1. **Type** determines the **base half-life** (what kind of knowledge)
2. **Tier** determines the **multiplier** (how validated is the knowledge)

The composition is multiplicative:

```
effective_half_life = tier_multiplier × type_base_half_life
```

This produces a 6×4 matrix of effective half-lives (24 combinations for the six primary types, excluding Playbook and Constraint which have special decay behavior):

| | Transient (0.1×) | Working (0.5×) | Consolidated (1.0×) | Persistent (5.0×) |
|---|---|---|---|---|
| **Insight** (30d) | 3 days | 15 days | 30 days | 150 days |
| **Heuristic** (90d) | 9 days | 45 days | 90 days | 450 days |
| **Warning** (7d) | 17 hours | 3.5 days | 7 days | 35 days |
| **CausalLink** (60d) | 6 days | 30 days | 60 days | 300 days |
| **StrategyFragment** (14d) | 1.4 days | 7 days | 14 days | 70 days |
| **Fact** (365d) | 36.5 days | 182.5 days | 365 days | 1,825 days |

Key observations:
- A Transient Warning (17 hours) decays extremely fast — if it is not confirmed within a day, it is essentially gone. This is appropriate: an unvalidated danger signal should not persist.
- A Persistent Fact (1,825 days ≈ 5 years) is the most durable knowledge type. Core facts that have been extensively validated are essentially permanent.
- A Working Insight (15 days) is the "default" state for most knowledge — useful for a couple of weeks, then needs reconfirmation or it fades.

---

## CLS Theory Mapping

The four-tier system directly implements the Complementary Learning Systems (CLS) theory of McClelland, McNaughton, and O'Reilly (1995):

| CLS Concept | Neuro Implementation |
|---|---|
| **Hippocampal fast learning** | Transient tier — entries are created quickly from episodes but decay rapidly |
| **Neocortical slow learning** | Persistent tier — entries are stable, representing generalized patterns |
| **Consolidation during sleep** | Dreams subsystem replays episodes and promotes tiers during idle time |
| **Interference protection** | Tier separation — Transient entries cannot corrupt Persistent entries |
| **Gradual transfer** | Working → Consolidated → Persistent — knowledge moves slowly through tiers as evidence accumulates |

The CLS model predicts that memories stored quickly in the hippocampus are initially fragile and must be gradually consolidated into the neocortex through repeated replay (especially during sleep). Neuro mirrors this: fast-extracted Transient entries are fragile (0.1× multiplier) and must be promoted through use and replay to reach the stable Persistent tier (5.0× multiplier).

The tier progression pipeline (see [12-4-tier-distillation-pipeline.md](./12-4-tier-distillation-pipeline.md)) serves as the consolidation mechanism — it analyzes episodes, extracts patterns, and promotes knowledge entries through tiers based on accumulated evidence. The Dreams subsystem drives this consolidation during idle time, mirroring hippocampal replay during sleep.

---

## Interaction with Confidence

Tiers and confidence are related but distinct:

- **Confidence** (0.0–1.0) is a continuous score reflecting accumulated evidence for/against the entry
- **Tier** (Transient/Working/Consolidated/Persistent) is a discrete level reflecting validation state

Confidence changes on every use:
- Positive outcome → confidence × `CONFIRMATION_BOOST` (1.5×)
- Negative outcome → confidence × 0.5 (halved)

Tier changes on accumulation:
- Promotion requires threshold count of positive outcomes
- Demotion requires specific trigger (negative outcome or contradiction)

An entry can have high confidence at a low tier (e.g., a Transient entry with 0.8 confidence that was just extracted from a high-quality source) or low confidence at a high tier (e.g., a Persistent entry with 0.4 confidence that has not been used recently and has decayed). Tier determines the decay rate; confidence determines the retrieval priority.

---

## Academic Foundations

- McClelland, J. L., McNaughton, B. L., & O'Reilly, R. C. (1995). "Why there are complementary learning systems in the hippocampus and neocortex." *Psychological Review*, 102(3), 419–457.
- Ebbinghaus, H. (1885). *Über das Gedächtnis* (On Memory).
- Walker, M. P., & van der Helm, E. (2009). "Overnight therapy? The role of sleep in emotional brain processing." *Psychological Bulletin*, 135(5), 731–748.
- Nader, K., Schafe, G. E., & Le Doux, J. E. (2000). "Fear memories require protein synthesis in the amygdala for reconsolidation after retrieval." *Nature*, 406, 722–726. (Memory reconsolidation — knowledge re-entering lower tiers during review)

---

## Current Status and Gaps

**Implemented**:
- Half-life constants per knowledge kind in `roko-neuro/src/lib.rs` (`FACT_HALF_LIFE_DAYS`, `INSIGHT_HALF_LIFE_DAYS`, `HEURISTIC_HALF_LIFE_DAYS`)
- `KnowledgeConfirmationRecord` for tracking confirmations
- `KnowledgeStore.decay()` method for applying time-based decay
- `KnowledgeStore.gc()` method for removing low-confidence entries
- `CONFIRMATION_BOOST = 1.5` for confidence boosting on confirmation
- `DEFAULT_GC_MIN_CONFIDENCE = 0.05` threshold

**Missing**:
- The tier enum itself (Transient/Working/Consolidated/Persistent) — not yet as a field on `KnowledgeEntry`
- Tier multiplier logic (`effective_half_life = tier_multiplier × type_base_half_life`)
- Tier promotion/demotion logic based on confirmation counts
- Special demotion rules per tier (Persistent requires explicit deprecation)
- AntiKnowledge confidence floor enforcement (0.3 minimum during GC)
- Integration with gate results for automatic tier updates

---

## Cross-references

- See [01-six-knowledge-types.md](./01-six-knowledge-types.md) for the base half-lives per type
- See [03-type-half-lives.md](./03-type-half-lives.md) for half-life rationale in detail
- See [07-ebbinghaus-decay-with-tier.md](./07-ebbinghaus-decay-with-tier.md) for the full decay formula
- See [12-4-tier-distillation-pipeline.md](./12-4-tier-distillation-pipeline.md) for how distillation drives tier progression
- See topic [10-dreams](../10-dreams/INDEX.md) for how Dreams drives offline consolidation/promotion
- See topic [04-verification](../04-verification/INDEX.md) for how gate results feed back to tier updates
