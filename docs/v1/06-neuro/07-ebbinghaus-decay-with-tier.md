# Demurrage with Tier Shaping

> Neuro keeps knowledge fresh through demurrage: every durable entry carries a balance, earns its keep through use, and cools when it stops being retrieved, cited, or reinforced. Ebbinghaus still matters, but as a rate-shaping component rather than the whole story.

> **Implementation**: Built

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [02-four-validation-tiers.md](./02-four-validation-tiers.md), [03-type-half-lives.md](./03-type-half-lives.md)
**Key sources**:
- historical archive: `bardo-backup/prd/04-memory/00-overview.md` (Ebbinghaus theory, forgetting curve equations)
- `refactoring-prd/03-cognitive-subsystems.md` §1 (effective_decay formula)
- `docs/00-architecture/04-decay-variants.md` (architecture-side decay model and Ebbinghaus shaping)
- `docs/00-architecture/18-decay-tier-matrix.md` (tier multipliers and promotion/demotion rules)
- `docs/00-architecture/01-naming-and-glossary.md` (canonical Neuro vocabulary)
- `tmp/refinements/12-knowledge-demurrage.md` (canonical demurrage refinement)
- `crates/roko-neuro/src/lib.rs` (half_life_days field, default_half_life_days)
- `crates/roko-neuro/src/knowledge_store.rs` (decay method, GC threshold)

---

## Abstract

Neuro no longer treats Ebbinghaus decay as the whole retention story. Durable knowledge now has an explicit freshness economy: each entry carries `balance`, that balance is charged by demurrage over time, and reinforcement replenishes it when the entry is actually useful. Ebbinghaus still shapes how quickly a kind of knowledge cools, but demurrage decides whether the knowledge keeps earning access to the hot path.

The validation tier still matters. Type-specific half-lives and tier multipliers shape the baseline drain rate, so a Transient Warning cools faster than a Persistent Fact. But the decisive factor is whether the entry keeps being retrieved, cited, surviving gates, or surfacing novel surprise. That makes Neuro self-trimming instead of merely time-decayed.

---

## The Forgetting Curve Formula

### Base Decay Equation

The retention curve is now best read as a balance update plus a time-shaping term:

```text
balance(t+Δt) = balance(t) - demurrage_tax(Δt) + reinforcement(kind, novelty)
freshness(t) = balance(t) × ebbinghaus_weight(age_days, type_half_life, tier_multiplier)
```

Where:
- `balance` is the durable-memory freshness reserve
- `demurrage_tax` is the holding cost paid per unit time
- `reinforcement` comes from retrieval, citation, gate survival, surprise, or agent quoting
- `ebbinghaus_weight` is the rate-shaping curve still provided by type and tier

At any point, the entry's practical usefulness is a product of current balance and the shaped time curve. If balance falls, the entry cools even if the curve would otherwise keep it warm.

### Effective Half-Life Composition

Type and tier remain the baseline knobs:

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

The important shift is interpretive: these values now shape how quickly balance is spent, not whether memory has an economy at all.

### Worked Examples

**Example 1: a Working Insight that keeps earning balance**

| Age | Balance / freshness | Interpretation |
|---|---|---|
| 0 days | 1.000 | Fresh at ingest |
| 7 days | 0.88 | Still being retrieved and cited |
| 15 days | 0.72 | Balance is being replenished by use |
| 30 days | 0.63 | Still warm because it keeps paying rent |

**Example 2: a Persistent Fact that stops being useful**

| Age | Balance / freshness | Interpretation |
|---|---|---|
| 0 days | 1.000 | Fresh |
| 30 days | 0.80 | Mild demurrage, no issue yet |
| 90 days | 0.41 | No longer reinforced, now cooling quickly |
| 180 days | 0.12 | Candidate for cold tier |

The point is not that older knowledge must disappear. The point is that knowledge should have to justify its storage cost.

---

## Reinforcement and Strengthening

### Balance-Earning Signals

Neuro treats these as balance-earning events:

- **Retrieved** - the entry was selected for active use
- **Cited** - another entry points to it as evidence or lineage
- **Gated** - it survived a verification gate
- **Surprised** - it explained an outcome that was novel or unexpected
- **AgentQuoted** - an agent explicitly reused it in an answer or plan

Each reinforcement path replenishes `balance` rather than merely bumping confidence. That matters because durable memory should stay warm only if it is still doing work.

### Novelty-Weighted Reinforcement

HDC similarity keeps reinforcement honest. A common entry that appears everywhere gets a small bump; a rare entry that is both correct and useful gets a larger bump. In Neuro terms, the bonus is novelty-weighted against the top-K HDC neighbors, so the memory system prefers knowledge that is both usable and distinctive.

This is the anti-hoarding rule: knowledge has to be earning its balance from uniquely useful contributions, not just from being repeated.

### Spacing Effect

Repeated reinforcement in distinct episodes matters more than the same turn being counted many times. That is the Neuro version of the spacing effect: balance rises when a rule survives across time, context, and gate outcomes, not when it is merely echoed inside one task.

---

## Garbage Collection

### GC Threshold

Balance has a floor. When an entry falls below that floor, it is no longer hot:

- it can be frozen into cold storage
- it can later be thawed if a future retrieval needs it
- it can be removed only if it is both cold and no longer worth keeping around

That means GC is no longer the only story. In Neuro terms, the first move is freeze, not forget.

### Cold-Tier Freeze/Thaw

Cold-tier graduation is the demurrage answer to archival bloat. The entry keeps its content address and lineage, but its body moves off the hot path. Thawing restores a starter balance so the entry can compete again, but not indefinitely. If it keeps failing after thaw, it cools back down.

This is the same basic rule as `18-decay-tier-matrix.md`: tier still shapes durability, but balance decides whether the entry deserves immediate access or cold storage.

### GC Schedule by Type × Tier

The old schedule still helps as a calibration reference, but read it as a baseline freshness envelope rather than as the full retention policy.

| Type \ Tier | Transient | Working | Consolidated | Persistent |
|---|---|---|---|---|
| **Warning** (7d) | 3 days | 15 days | 30 days | 151 days |
| **StrategyFragment** (14d) | 6 days | 30 days | 60 days | 301 days |
| **Insight** (30d) | 13 days | 65 days | 129 days | 646 days |
| **CausalLink** (60d) | 26 days | 129 days | 258 days | 1,293 days |
| **Heuristic** (90d) | 39 days | 194 days | 387 days | 1,939 days |
| **Fact** (365d) | 157 days | 787 days | 1,573 days | 7,865 days |

**AntiKnowledge** stays protected by its confidence floor and should still resist over-pruning.

---

## Demurrage vs. Neural Network Embedding Drift

Neuro's retention policy is explicit and auditable. Balance, tier, and age explain why an entry is warm, cold, or thawed. That is better than opaque embedding drift, where similarity changes because the model state changed under the hood.

The practical advantage is replayability: given an entry's type, tier, balance, and reinforcement history, Neuro can explain why that knowledge was available at the time of a decision. That aligns retention with forensic traceability instead of hidden vector behavior.

---

## Academic Foundations

- Ebbinghaus, H. (1885). *Über das Gedächtnis*. Leipzig: Duncker & Humblot.
- Murre, J. M. J., & Dros, J. (2015). "Replication and Analysis of Ebbinghaus' Forgetting Curve." *PLOS ONE*, 10(7), e0120644.
- Wixted, J. T., & Ebbesen, E. B. (1991). "On the Form of Forgetting." *Psychological Science*, 2(6), 409–415.
- Pimsleur, P. (1967). "A Memory Schedule." *Modern Language Journal*, 51(2), 73–75. (Graduated interval recall)
- McClelland, J. L., McNaughton, B. L., & O'Reilly, R. C. (1995). "Complementary learning systems." *Psychological Review*, 102(3). (Fast vs. slow learning)

---

## Current Status and Gaps

**Implemented or documented**:
- Type-specific half-life constants
- Tier multipliers for validation depth
- Explicit demurrage framing for durable-memory freshness
- HDC novelty-weighted reinforcement as the preferred reinforcement model
- Freeze/thaw semantics for cold-tier knowledge

**Still to wire through consistently**:
- Balance updates on every retrieval, citation, surprise, and gate-survival path
- Cold-tier freeze/thaw hooks in the storage backend
- Full replacement of old decay-only wording in downstream Neuro docs

---

## Cross-References

- See [04-decay-variants.md](../00-architecture/04-decay-variants.md) for the architecture-side decay model
- See [18-decay-tier-matrix.md](../00-architecture/18-decay-tier-matrix.md) for the tier matrix and promotion rules
- See [Naming and Glossary](../00-architecture/01-naming-and-glossary.md) for canonical Neuro terms
- See [tmp/refinements/12-knowledge-demurrage.md](../../tmp/refinements/12-knowledge-demurrage.md) for the full demurrage proposal
