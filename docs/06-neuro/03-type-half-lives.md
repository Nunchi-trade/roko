# Type Half-Lives: Rationale and Design

> Each knowledge type has a base half-life calibrated to the expected staleness rate of that category of knowledge, drawn from memory research and practical agent operation.


> **Implementation**: Built

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [01-six-knowledge-types.md](./01-six-knowledge-types.md), [02-four-validation-tiers.md](./02-four-validation-tiers.md)
**Key sources**:
- `refactoring-prd/03-cognitive-subsystems.md` §1 (base half-life table)
- `bardo-backup/prd/04-memory/00-overview.md` (Ebbinghaus decay theory)
- `bardo-backup/prd/04-memory/01-grimoire.md` (original half-life assignments)
- `bardo-backup/prd/04-memory/01b-grimoire-memetic.md` (AntiKnowledge decay: floor 0.3, 0.5× demurrage)
- `crates/roko-neuro/src/lib.rs` (current half-life constants)

---

## Abstract

The half-life of a knowledge entry determines how quickly it decays when not reinforced. This is not an arbitrary design choice — it reflects the empirical observation (dating to Ebbinghaus 1885) that different kinds of information have different natural rates of obsolescence. A security warning about a specific vulnerability is far more time-sensitive than a fundamental fact about language semantics. A strategy fragment tailored to current market conditions becomes stale faster than a general heuristic about risk management.

This document provides the detailed rationale for each type's base half-life, the research basis for the Ebbinghaus decay model, and the interaction between type half-lives and tier multipliers. It also documents the current code constants and the gap between the implemented half-lives and the refactoring-prd design.

---

## The Ebbinghaus Decay Model

### Historical Foundation

Hermann Ebbinghaus (1885) conducted the first systematic experiments on memory retention. Using nonsense syllables to control for prior knowledge, he demonstrated that memory follows an approximately exponential decay curve:

```
retention(t) = e^(-t / S)
```

where `t` is time since encoding and `S` is a "stability" parameter that increases with each successful retrieval. Ebbinghaus found that:

1. **Most forgetting happens early** — retention drops steeply in the first hours and days
2. **Spacing effects strengthen memory** — items retrieved at increasing intervals are retained longer
3. **Meaningfulness matters** — meaningful material decays slower than arbitrary material

### Application to Agent Knowledge

Neuro applies the Ebbinghaus model to agent knowledge with the following adaptation:

```
weight(entry) = exp(-age_days / (half_life_days × ln(2)))
```

At time `t = half_life_days`, the weight equals 0.5 (half the original strength). This is equivalent to:

```
weight(entry) = 2^(-age_days / half_life_days)
```

The `half_life_days` is determined by two factors:
- **Type base half-life**: How quickly this *kind* of knowledge becomes stale
- **Tier multiplier**: How validated this *specific* entry is

The product `effective_half_life = tier_multiplier × type_base_half_life` gives the actual decay rate.

### Why Exponential Decay, Not Linear

Linear decay (weight decreases by a fixed amount per day) would mean that old knowledge and new knowledge decay at the same absolute rate. Exponential decay ensures that:
- **Fresh knowledge decays fastest** — a 1-day-old entry with a 7-day half-life has already lost 10% of its weight
- **Old knowledge decays slowly** — an entry that has survived for 3 half-lives has only 12.5% weight remaining, and each additional day reduces it by a tiny amount
- **The GC threshold creates a natural cutoff** — entries below 0.05 confidence are removed, which happens after approximately 4.3 half-lives (2^(-4.3) ≈ 0.05)

This matches the Ebbinghaus curve's characteristic shape: steep initial drop, then a long tail.

---

## Base Half-Lives by Type

### Warning: 7 Days

**Rationale**: Danger signals must be current. A warning about a specific vulnerability, a dangerous API behavior, or a market condition is only valuable if it reflects the present state. Warnings about software vulnerabilities may be patched within days. Warnings about network conditions change with every block. Warnings about API rate limits may be updated by the provider at any time.

**Effective half-lives by tier**:
| Tier | Multiplier | Effective Half-Life |
|---|---|---|
| Transient | 0.1× | 16.8 hours |
| Working | 0.5× | 3.5 days |
| Consolidated | 1.0× | 7 days |
| Persistent | 5.0× | 35 days |

**Time to GC (4.3 half-lives)**:
| Tier | Time to GC |
|---|---|
| Transient | ~3 days |
| Working | ~15 days |
| Consolidated | ~30 days |
| Persistent | ~150 days |

**Design note**: Even Persistent warnings have a relatively short effective half-life (35 days). This is intentional — no warning should persist for months without reconfirmation. If a warning is still valid, it will be reconfirmed by ongoing experience and stay above the GC threshold.

### StrategyFragment: 14 Days

**Rationale**: Strategies are context-dependent. A StrategyFragment encodes a multi-step procedure tailored to current conditions: current tool versions, API behaviors, market microstructure, team conventions. These conditions change frequently. A deployment strategy that worked two weeks ago may need adaptation after infrastructure changes. A trading strategy fragment may become unprofitable as market conditions shift.

**Effective half-lives by tier**:
| Tier | Multiplier | Effective Half-Life |
|---|---|---|
| Transient | 0.1× | 1.4 days |
| Working | 0.5× | 7 days |
| Consolidated | 1.0× | 14 days |
| Persistent | 5.0× | 70 days |

**Design note**: Transient StrategyFragments have only 1.4 days of effective half-life. This means an unvalidated strategy essentially evaporates within 6 days (4.3 half-lives). This aggressive decay prevents the agent from accumulating stale procedures that may cause harm when applied to changed conditions.

### Insight: 30 Days

**Rationale**: Observations need regular revalidation. An Insight records a pattern the agent has observed ("Rust's borrow checker errors often mean you need Arc here"). Patterns can shift as codebases evolve, APIs change, models update, and environments drift. A 30-day half-life ensures that unused Insights fade within a few months, preventing knowledge base bloat with obsolete observations.

**Effective half-lives by tier**:
| Tier | Multiplier | Effective Half-Life |
|---|---|---|
| Transient | 0.1× | 3 days |
| Working | 0.5× | 15 days |
| Consolidated | 1.0× | 30 days |
| Persistent | 5.0× | 150 days |

**Current code constant**:
```rust
// From roko-neuro/src/lib.rs
pub const INSIGHT_HALF_LIFE_DAYS: f64 = 30.0;
```

**Design note**: 30 days is a balance point. Shorter (7-14 days) would cause too much churn — agents would constantly re-discover the same patterns. Longer (60-90 days) would allow stale observations to persist too long and potentially mislead the agent.

### CausalLink: 60 Days

**Rationale**: Causal relationships need periodic confirmation but are more durable than simple observations. A CausalLink ("increasing thread pool size → reduced I/O latency") captures a structural relationship that tends to hold across a wider range of conditions than a single Insight. However, causal mechanisms can change — an API provider may change their backend, invalidating a learned cause-effect relationship.

**Effective half-lives by tier**:
| Tier | Multiplier | Effective Half-Life |
|---|---|---|
| Transient | 0.1× | 6 days |
| Working | 0.5× | 30 days |
| Consolidated | 1.0× | 60 days |
| Persistent | 5.0× | 300 days |

**Design note**: CausalLinks at the Working tier (30-day effective half-life) have the same durability as Consolidated Insights. This reflects the principle that a partially validated causal relationship is approximately as reliable as a fully validated observation — both describe real patterns, but causal relationships have more structural support.

### Heuristic: 90 Days

**Rationale**: Rules of thumb are the most durable category of practical knowledge (excluding Facts). A Heuristic represents a pattern that has been abstracted from multiple observations and validated across contexts. "Always run clippy before committing" does not depend on any specific codebase or API — it captures a general practice. Heuristics are the backbone of an experienced agent's operational knowledge.

**Effective half-lives by tier**:
| Tier | Multiplier | Effective Half-Life |
|---|---|---|
| Transient | 0.1× | 9 days |
| Working | 0.5× | 45 days |
| Consolidated | 1.0× | 90 days |
| Persistent | 5.0× | 450 days |

**Current code constant**:
```rust
// From roko-neuro/src/lib.rs
pub const HEURISTIC_HALF_LIFE_DAYS: f64 = 90.0;
```

**Design note**: A Persistent Heuristic (450 days ≈ 15 months) is essentially permanent for operational purposes. This is appropriate — a rule of thumb that has survived extensive validation and has never been contradicted represents genuine operational wisdom.

### Fact: 365 Days

**Rationale**: Established facts change slowly. A Fact records a declarative statement treated as true until contradicted: "Rust uses LLVM as its compilation backend," "Ethereum blocks are produced approximately every 12 seconds," "The TCP three-way handshake requires SYN, SYN-ACK, ACK." These statements are stable over long periods.

**Effective half-lives by tier**:
| Tier | Multiplier | Effective Half-Life |
|---|---|---|
| Transient | 0.1× | 36.5 days |
| Working | 0.5× | 182.5 days |
| Consolidated | 1.0× | 365 days |
| Persistent | 5.0× | 1,825 days (5 years) |

**Current code constant**:
```rust
// From roko-neuro/src/lib.rs
pub const FACT_HALF_LIFE_DAYS: f64 = 365.0;
```

**Design note**: Even facts eventually need revalidation. A 365-day half-life means that an unused Fact decays to GC threshold in about 4.3 years at Consolidated tier, or about 21 years at Persistent tier. This is appropriate — language specifications, protocol behaviors, and platform properties do change, just slowly.

### AntiKnowledge: Never (Floor 0.3)

**Rationale**: Known unknowns are always valuable. AntiKnowledge records things that seem true but are not: "Moving to async doesn't always improve throughput," "Higher APY doesn't mean higher risk-adjusted returns." An agent that forgets its AntiKnowledge will re-discover and re-try failed approaches, wasting resources and potentially causing harm.

**Decay behavior**: AntiKnowledge entries do not have a standard half-life. Instead:
- Their confidence decays at **0.5× the normal rate** (half-speed demurrage)
- Their confidence has a **floor of 0.3** — it can never drop below this
- They are **exempt from garbage collection** (the 0.05 GC threshold does not apply)

**The confidence floor of 0.3**: This specific value was chosen because it is:
- High enough to ensure the entry remains retrievable (0.3 is above the typical retrieval noise floor of 0.1-0.2)
- Low enough that newer positive evidence can outweigh it in retrieval ranking (a Consolidated entry at 0.7 confidence will rank above an AntiKnowledge entry at 0.3)
- The same as the initial confidence assigned to Dream-generated hypotheses (0.20-0.30), creating a natural equilibrium where old AntiKnowledge and new hypotheses have similar retrieval priority

**On-chain demurrage**: When AntiKnowledge entries are published to the Korai chain, they use 0.5× the standard demurrage rate. Standard entries decay at 1% per year; AntiKnowledge decays at 0.5% per year. This ensures that negative knowledge persists on-chain as a public good.

---

## Half-Life Ordering and Design Logic

The half-lives are ordered from shortest to longest as follows:

```
Warning (7d) < StrategyFragment (14d) < Insight (30d) < CausalLink (60d) < Heuristic (90d) < Fact (365d) < AntiKnowledge (∞)
```

This ordering follows a principle of **abstraction durability**: the more abstract and general a piece of knowledge is, the longer it persists. Concrete, context-dependent knowledge (Warnings, StrategyFragments) decays fastest. Abstract, validated patterns (Heuristics, Facts) persist longest. Knowledge about what is wrong (AntiKnowledge) never fully decays because the cost of re-discovering a known failure is always positive.

The ordering also reflects the typical lifecycle of knowledge in an agent's memory:
1. **Episodes** produce raw turns (not stored in Neuro)
2. **Distillation** extracts Insights (30d) and Warnings (7d) from episodes
3. **Pattern detection** promotes clusters of Insights to Heuristics (90d)
4. **Causal analysis** identifies CausalLinks (60d) between observed patterns
5. **Strategy formation** compiles Heuristics and CausalLinks into StrategyFragments (14d)
6. **Playbook compilation** compiles validated strategies into PLAYBOOK.md
7. **Failure analysis** generates AntiKnowledge (∞) from contradicted entries

---

## Default Half-Life in Code

The current implementation assigns a default half-life per `KnowledgeKind` via a match:

```rust
// From roko-neuro/src/lib.rs
const fn default_half_life_days() -> f64 {
    30.0  // fallback for types without specific constants
}

pub const FACT_HALF_LIFE_DAYS: f64 = 365.0;
pub const INSIGHT_HALF_LIFE_DAYS: f64 = 30.0;
pub const HEURISTIC_HALF_LIFE_DAYS: f64 = 90.0;

impl KnowledgeKind {
    pub const fn default_half_life_days(self) -> f64 {
        match self {
            Self::Fact => FACT_HALF_LIFE_DAYS,
            Self::Insight => INSIGHT_HALF_LIFE_DAYS,
            Self::Heuristic => HEURISTIC_HALF_LIFE_DAYS,
            Self::Procedure | Self::Playbook | Self::Constraint | Self::AntiKnowledge => {
                default_half_life_days()
            }
        }
    }
}
```

**Gap**: The current code assigns 30 days (the default) to Procedure, Playbook, Constraint, and AntiKnowledge. The refactoring-prd design specifies:
- Warning: 7 days (not yet a variant)
- CausalLink: 60 days (not yet a variant)
- StrategyFragment: 14 days (mapped to Procedure, currently at 30 days)
- AntiKnowledge: never-decay with floor 0.3 (currently at 30 days, which is incorrect)

Reconciliation requires adding the new variants and updating the `default_half_life_days()` method to return the correct constants for each type.

---

## Academic Foundations

- Ebbinghaus, H. (1885). *Über das Gedächtnis* (On Memory). Leipzig: Duncker & Humblot.
- Murre, J. M. J., & Dros, J. (2015). "Replication and Analysis of Ebbinghaus' Forgetting Curve." *PLOS ONE*, 10(7), e0120644. (Modern replication confirming the exponential model)
- Wixted, J. T., & Ebbesen, E. B. (1991). "On the Form of Forgetting." *Psychological Science*, 2(6), 409–415. (Power law vs. exponential debate; Ebbinghaus approximation holds for practical time scales)
- McClelland, J. L., McNaughton, B. L., & O'Reilly, R. C. (1995). "Why there are complementary learning systems in the hippocampus and neocortex." *Psychological Review*, 102(3), 419–457. (Fast vs. slow learning systems)

---

## Current Status and Gaps

**Implemented**:
- `FACT_HALF_LIFE_DAYS = 365.0`
- `INSIGHT_HALF_LIFE_DAYS = 30.0`
- `HEURISTIC_HALF_LIFE_DAYS = 90.0`
- `default_half_life_days()` fallback of 30.0 days
- `KnowledgeKind::default_half_life_days()` method

**Missing**:
- Warning half-life constant (7 days) — type not yet in enum
- CausalLink half-life constant (60 days) — type not yet in enum
- StrategyFragment half-life constant (14 days) — Procedure currently uses 30-day default
- AntiKnowledge special decay (never, floor 0.3) — currently uses 30-day default
- Tier multiplier application to half-lives
- Configurable half-lives in `roko.toml` (for domain-specific tuning)

---

## Cross-References

- See [01-six-knowledge-types.md](./01-six-knowledge-types.md) for the six types and their definitions
- See [02-four-validation-tiers.md](./02-four-validation-tiers.md) for the tier multiplier system
- See [07-ebbinghaus-decay-with-tier.md](./07-ebbinghaus-decay-with-tier.md) for the full decay formula with worked examples
- See [11-antiknowledge-challenge.md](./11-antiknowledge-challenge.md) for AntiKnowledge's special decay behavior
- See topic [05-learning](../05-learning/INDEX.md) for how decay interacts with the learning feedback loop
