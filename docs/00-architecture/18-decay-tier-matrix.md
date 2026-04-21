# Decay x Knowledge Tier matrix

> Layer 0 Kernel -- Engram Lifecycle
> Status: **Specification** -- parameters ready for implementation
> Canonical source: `crates/roko-core/src/decay.rs`, `crates/roko-neuro/` (planned)
> Cross-references: [04-decay-variants.md](04-decay-variants.md), [13-cognitive-cross-cuts.md](13-cognitive-cross-cuts.md), [01-naming-and-glossary.md](01-naming-and-glossary.md), [tmp/refinements/12-knowledge-demurrage.md](../../tmp/refinements/12-knowledge-demurrage.md)

> **Implementation**: Shipping

---

## Purpose

Every Neuro knowledge entry still has a decay curve, but the tier policy now governs demurrage: how much balance it loses, how reinforcement sticks, when it freezes into cold storage, and what it takes to thaw it back. `04-decay-variants.md` defines the underlying decay curves; this chapter defines the tier policy layered on top of them.

Age still matters, but only as input to charge accumulation. Promotion and demotion are now driven by usage, contradiction, and balance rather than wall-clock time alone.

---

## 1. The 4x4 matrix

The matrix below maps each tier to its demurrage charge, reinforcement stickiness, cold-floor behavior, and thaw rule.

| Tier | Demurrage charge | Reinforcement stickiness | Cold-floor behavior | Thaw rule |
|---|---|---|---|---|
| **Transient** | Highest charge. Balance drops quickly unless the entry is actively used. | Reacts fast to `Retrieved` and `Surprised` reinforcement, but the gain is easy to lose. | Freezes early when balance falls below the floor or when contradictions stack up. | Easy to thaw if the entry is cited again or reused successfully. |
| **Working** | Baseline charge. Useful entries can stay warm, but neglect still costs them balance. | `Retrieved`, `Cited`, and `Gated` events keep it sticky across a few episodes. | Freezes when balance weakens or repeated failures show that the entry is no longer current. | Thaws on successful reuse and usually returns to the same tier policy. |
| **Consolidated** | Lower charge. Long-lived knowledge ages slowly unless contradicted. | Reinforcement is broader than in Working: cross-plan citations and successful gates matter more than repetition. | Freezes only after sustained contradiction or a long slide in balance. | Thaw requires fresh confirmation, not just a single read. |
| **Persistent** | Lowest charge. The entry should remain available unless the system has a strong reason to move it. | Reinforcement is mostly stickiness, not growth. It protects already-earned balance more than it adds new balance. | Usually pinned rather than deleted. Only extraordinary contradiction or policy says otherwise. | Thaw is explicit and policy-gated, especially for witness-backed material. |

Recommended balance bands:

| Tier | Suggested balance band |
|---|---|
| **Transient** | `< 0.35` |
| **Working** | `0.35 - 0.8` |
| **Consolidated** | `0.8 - 1.2` |
| **Persistent** | `> 1.2` |

These are defaults, not rigid law. The important point is that tier expresses a policy over balance, not just a half-life curve.

---

## 2. Tier definitions

```rust
/// Knowledge tier determines demurrage behavior, thaw behavior, and
/// how much reinforcement an entry can actually keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum KnowledgeTier {
    /// First observation. Highest charge, easiest to freeze.
    Transient,
    /// Recently useful knowledge. Baseline charge.
    Working,
    /// Cross-validated knowledge. Lower charge, broader stickiness.
    Consolidated,
    /// Heavily validated knowledge. Lowest charge, usually pinned.
    Persistent,
}

impl KnowledgeTier {
    /// Multiplier applied to the base demurrage charge.
    pub fn charge_multiplier(&self) -> f32 {
        match self {
            Self::Transient => 2.0,
            Self::Working => 1.0,
            Self::Consolidated => 0.5,
            Self::Persistent => 0.1,
        }
    }

    /// Multiplier applied to reinforcement gain.
    pub fn reinforcement_multiplier(&self) -> f32 {
        match self {
            Self::Transient => 1.5,
            Self::Working => 1.0,
            Self::Consolidated => 0.75,
            Self::Persistent => 0.5,
        }
    }

    /// Balance floor under which the entry should freeze.
    pub fn cold_floor(&self) -> f64 {
        match self {
            Self::Transient => 0.25,
            Self::Working => 0.15,
            Self::Consolidated => 0.05,
            Self::Persistent => 0.0,
        }
    }
}
```

The operative fields are `balance`, `demurrage_paid`, and `last_touched_at`. Balance is the live control surface; demurrage paid is observability; `last_touched_at` keeps charge and reinforcement monotonic.

---

## 3. Demurrage policy by knowledge type

Not every knowledge type should react the same way to demurrage. The tier policy stays the same, but the charge/reinforcement mix changes by kind.

| Knowledge type | Demurrage shape | Reinforcement bias | Cold-floor note |
|---|---|---|---|
| **Insight** | Moderate charge; should stay usable without becoming permanent by accident. | Strong on successful reuse and citation. | Freeze only after long non-use or contradiction. |
| **Heuristic** | Moderate charge, but more stickiness after validation. | `Gated` and `AgentQuoted` events matter more than raw retrieval. | Should thaw back into Working before it can be promoted again. |
| **Warning** | Higher charge unless it keeps proving current relevance. | `Surprised` events keep it alive when the system is still being tripped up by the same failure mode. | Stale warnings should not dominate routing. |
| **CausalLink** | Lower charge, because causal knowledge often survives longer than the episode that revealed it. | `Cited` and `Gated` events should be sticky. | Contradiction should demote it faster than age alone. |
| **StrategyFragment** | Low charge and broad utility. | Reuse across plans should reinforce it strongly. | Good candidate for cold storage and later thaw. |
| **AntiKnowledge** | Special case. Ordinary demurrage should not make falsified knowledge vanish. | Reinforce when it prevents re-exploration of a dead end. | Usually stays retrievable even when frozen. |

The `Demurrage` policy object is the thing that charges, reinforces, and reports effective weight:

```rust
pub trait Demurrage {
    fn charge(&mut self, entry: &mut KnowledgeEntry, now: Timestamp) -> f64;
    fn reinforce(&mut self, entry: &mut KnowledgeEntry, kind: ReinforceKind);
    fn effective_weight(&self, entry: &KnowledgeEntry) -> f64;
}

pub enum ReinforceKind {
    Cited,
    Retrieved,
    Gated,
    Surprised,
    AgentQuoted,
}
```

---

## 4. Promotion rules

Promotion is now based on usage plus balance. Age only matters indirectly, because demurrage charges accumulate over time.

```
Promotion criteria:
  Transient -> Working:
    - At least 2 successful uses in distinct episodes
    - Balance remains above the Transient floor
    - No unresolved contradiction in the recent window

  Working -> Consolidated:
    - At least 5 successful uses across 2+ plans
    - Balance remains in the Working/Consolidated band
    - Contradictions do not dominate recent evidence

  Consolidated -> Persistent:
    - At least 10 successful uses across 3+ sessions
    - Balance remains above the Persistent threshold
    - No unresolved contradictions in the recent window
```

### Promotion algorithm

```rust
fn try_promote(entry: &mut KnowledgeEntry, stats: &UsageStats) -> bool {
    let target = match entry.tier {
        KnowledgeTier::Transient
            if stats.successful_refs >= 2
            && stats.distinct_episodes >= 2
            && entry.balance >= 0.35
            && stats.unresolved_contradictions == 0 => KnowledgeTier::Working,
        KnowledgeTier::Working
            if stats.successful_refs >= 5
            && stats.distinct_plans >= 2
            && entry.balance >= 0.8
            && stats.recent_contradictions <= 1 => KnowledgeTier::Consolidated,
        KnowledgeTier::Consolidated
            if stats.successful_refs >= 10
            && stats.distinct_sessions >= 3
            && entry.balance >= 1.2
            && stats.unresolved_contradictions == 0 => KnowledgeTier::Persistent,
        _ => return false,
    };

    entry.tier = target;
    entry.last_touched_at = now();
    true
}
```

Promotion should prefer evidence of current utility over age-based survival. A stale but never-used entry should not climb tiers just because enough time passed.

---

## 5. Demotion rules

Demotion is the reverse path: balance falls, contradiction rises, or the entry stops paying for its place in memory.

```
Demotion criteria:
  Persistent -> Consolidated:
    - Balance drops below the Persistent band
    - OR a recent contradiction removes confidence

  Consolidated -> Working:
    - Balance drops below the Consolidated band
    - OR repeated contradictions appear in a short window

  Working -> Transient:
    - Balance drops below the Working band
    - OR several failed uses show that the entry is no longer current

  Transient -> Cold storage:
    - Balance falls to the floor
    - OR the entry is repeatedly contradicted and no longer worth hot-path cost
```

### Demotion algorithm

```rust
fn try_demote(entry: &mut KnowledgeEntry, stats: &UsageStats) -> bool {
    let target = match entry.tier {
        KnowledgeTier::Persistent
            if entry.balance < 1.2 || stats.recent_contradictions >= 1 => KnowledgeTier::Consolidated,
        KnowledgeTier::Consolidated
            if entry.balance < 0.8 || stats.recent_contradictions >= 2 => KnowledgeTier::Working,
        KnowledgeTier::Working
            if entry.balance < 0.35 || stats.failed_refs_total >= 3 => KnowledgeTier::Transient,
        _ => return false,
    };

    entry.tier = target;
    entry.last_touched_at = now();
    true
}
```

Transient entries do not necessarily disappear when they demote. They freeze when the floor is reached, then thaw later if the system has a reason to pull them back into the hot path.

---

## 6. Worked examples

### Example 1: Playbook freshness

A playbook is first stored as Transient with balance `1.0`.

```
t=0:   Playbook created. Tier = Transient. Balance = 1.0.

t=1d:  Agent reuses it twice in successful tasks. Reinforcement offsets the charge.
       Balance stays above 0.35. Promote to Working.

t=1w:  The same playbook keeps being cited across two plans.
       Balance stays in the Working/Consolidated band. Promote to Consolidated.

t=1m:  The playbook is still useful across multiple sessions.
       Balance stays above 1.2. Promote to Persistent.
```

The important part is not the age of the playbook. It is whether the playbook keeps earning balance.

### Example 2: Contradiction and thaw

A Consolidated heuristic says "prefer tool X for this workflow."

```
t=0:   Heuristic is Consolidated. Balance = 0.9.

t=2d:  Two tasks fail in the same way after following the heuristic.
       Recent contradiction pushes it below the Consolidated band.
       Demote to Working.

t=5d:  More failures drop balance to the floor.
       Freeze into cold storage.

t=20d: A new task reuses the same pattern and the frozen entry is thawed.
       The Bus publishes a thaw Pulse, and the entry returns with a conservative balance.
```

### Example 3: AntiKnowledge and chain-witnessed material

An AntiKnowledge entry disproves a bad hypothesis.

```
t=0:   AntiKnowledge is stored. It stays retrievable even if balance falls.

t=30d: The same false hypothesis appears again.
       The entry is still available and blocks re-exploration.
```

Current guidance for chain-witnessed material is similar but stricter: freeze it if it must leave the hot path, but do not casually delete it. Whether chain-witnessed entries are permanently pinned or only thaw-eligible is still an open policy question.

---

## 7. Configuration parameters

| Parameter | Default | Range | Meaning |
|---|---|---|---|
| `transient_charge_multiplier` | 2.0 | 1.0 - 4.0 | Highest demurrage charge. |
| `working_charge_multiplier` | 1.0 | 0.5 - 2.0 | Baseline charge. |
| `consolidated_charge_multiplier` | 0.5 | 0.1 - 1.0 | Lower charge for cross-validated knowledge. |
| `persistent_charge_multiplier` | 0.1 | 0.01 - 0.5 | Near-pinned charge. |
| `transient_reinforcement_multiplier` | 1.5 | 1.0 - 3.0 | Fast response to new evidence. |
| `working_reinforcement_multiplier` | 1.0 | 0.5 - 2.0 | Balanced reinforcement. |
| `consolidated_reinforcement_multiplier` | 0.75 | 0.25 - 1.5 | Broad but restrained stickiness. |
| `persistent_reinforcement_multiplier` | 0.5 | 0.1 - 1.0 | Mostly preserves earned balance. |
| `transient_floor_balance` | 0.25 | 0.05 - 0.5 | Freeze threshold for Transient. |
| `working_floor_balance` | 0.15 | 0.05 - 0.3 | Freeze threshold for Working. |
| `consolidated_floor_balance` | 0.05 | 0.0 - 0.2 | Freeze threshold for Consolidated. |
| `persistent_floor_balance` | 0.0 | 0.0 - 0.1 | Persistent is usually pinned, not pruned. |
| `thaw_start_balance` | 0.3 | 0.1 - 0.5 | Conservative balance after thaw. |

These knobs are the tier-facing policy surface. The underlying decay curve still lives in `04-decay-variants.md`, but the tier matrix decides how much the curve matters.

---

## 8. Integration wiring

The demurrage-tier matrix integrates at four points:

1. **Knowledge creation** (`NeuroStore::insert`): assigns the initial tier, balance, and floor policy.
2. **Knowledge use** (`Substrate.query`, gate checks, composer selection): charges demurrage and applies reinforcement based on the kind of use.
3. **Dreams consolidation** (`DreamsEngine::consolidate`): reads usage stats, contradictions, and balance, then applies promotion or demotion.
4. **Cold storage** (`Substrate.freeze` and `Substrate.thaw`): moves entries in and out of the cold tier and emits a Bus Pulse on thaw.

```
Balance flow:
  Agent uses knowledge -> usage stats recorded
      |
      v
  Demurrage charges balance over time
      |
      v
  Reinforcement adds sticky credit when the use is successful
      |
      v
  Dreams evaluates balance + contradiction + usage
      |
      v
  Promote, demote, freeze, or thaw
```

The integration point that matters most is the Scorer: it should read effective weight from balance-aware policy, not from age alone.

---

## 9. Error handling

| Condition | Response |
|---|---|
| Balance falls below zero | Clamp to the cold floor and freeze if needed. |
| Promotion and demotion both trigger | Demotion wins if contradiction or floor pressure is present. |
| Clock skew produces negative elapsed time | Do not charge demurrage for the negative interval. |
| A thaw request hits a witness-locked entry | Rehydrate only if policy allows it; otherwise keep it frozen. |
| Reinforcement and charge collide in the same tick | Apply charge first, then reinforcement, so successful use can offset the tax. |

---

## 10. Test criteria

1. Promotion from Transient to Persistent requires usage plus balance, not age alone.
2. Demotion occurs when contradiction or low balance crosses the configured floor.
3. Thaw restores a frozen entry with a conservative starting balance.
4. AntiKnowledge remains retrievable after freeze and does not vanish via ordinary demurrage.
5. Chain-witnessed entries obey their witness policy and do not get casually pruned.
6. Round-trip serialization of the tier policy preserves charge multipliers, floor balances, and thaw behavior.
7. Reinforcement by kind changes the balance outcome in the expected tier-specific direction.

---

## Cross-References

- [04-decay-variants.md](04-decay-variants.md) -- underlying decay curves and `Decay` enum details
- [13-cognitive-cross-cuts.md](13-cognitive-cross-cuts.md) -- Neuro tier promotion, Dreams consolidation, and cross-cut injection
- [01-naming-and-glossary.md](01-naming-and-glossary.md) -- current kernel vocabulary
- [tmp/refinements/12-knowledge-demurrage.md](../../tmp/refinements/12-knowledge-demurrage.md) -- canonical demurrage refinement source
- `crates/roko-core/src/decay.rs` -- lower-level decay implementation
- `crates/roko-neuro/` -- NeuroStore integration target
