# Pheromone Kinds: The Taxonomy of Coordination Signals

> **Layer**: L1 Framework (type system definition), referenced by L2 Scaffold (context
> enrichment) and L4 Orchestration (multi-agent coordination)
>
> **Synapse traits**: `Substrate` (stores all kinds), `Scorer` (weights kinds differently per
> agent role), `Router` (selects among competing kinds), `Policy` (emits kind-specific
> responses)
>
> **Prerequisites**: `03-digital-pheromones.md` (pheromone fundamentals)

---

## Overview

Every digital pheromone in Roko carries a `PheromoneKind` that determines its semantic meaning,
default decay profile, and the behavioral response it triggers in agents that sense it. The
kind system is organized into three tiers:

1. **Universal kinds** (3): Present in every domain, every agent, every scope
2. **Domain-specific kinds** (4): Common across multiple domains but with domain-dependent
   interpretation
3. **Custom kinds** (∞): User-defined, domain-specific signals via `Custom(String)`

This design balances standardization (universal kinds ensure all agents share a common signal
vocabulary) with extensibility (custom kinds allow domain-specific coordination without
modifying the core type system).

---

## The PheromoneKind Enum

```rust
/// The type of coordination signal a pheromone carries.
///
/// # Design Rationale
///
/// The enum has three tiers:
///
/// 1. **Universal** (Threat, Opportunity, Wisdom): Every agent understands these.
///    They have hardcoded default decay profiles and behavioral responses.
///    Inspired by the three fundamental survival signals in biological
///    pheromone systems: alarm, recruitment, and trail pheromones.
///
/// 2. **Domain-specific** (Alpha, Pattern, Anomaly, Consensus): Common across
///    multiple domains but with domain-dependent interpretation. A "Pattern"
///    in code development means "code smell detected"; in data engineering it
///    means "recurring data quality issue".
///
/// 3. **Custom(String)**: Open-ended extension point. Domain plugins define
///    their own pheromone kinds without modifying this enum. The string must
///    be a valid identifier (alphanumeric + underscores, ≤64 chars).
///
/// # References
///
/// The three-tier structure is inspired by the hierarchy of pheromone types
/// observed in social insects [Wilson, E.O. "The Insect Societies." Belknap
/// Press, 1971]:
/// - Primer pheromones (long-term physiological changes) → Wisdom
/// - Releaser pheromones (immediate behavioral responses) → Threat, Opportunity
/// - Informational pheromones (contextual signals) → Alpha, Pattern, Anomaly
pub enum PheromoneKind {
    // ── Universal Kinds ────────────────────────────────────────────
    /// Something dangerous or harmful has been detected.
    /// Default half-life: 2 hours.
    ///
    /// Biological analog: Alarm pheromone (e.g., formic acid in ants).
    /// Triggers immediate defensive or remediation behavior.
    ///
    /// Examples across domains:
    /// - Code: Failing test suite, security vulnerability, broken build
    /// - Research: Retracted citation, contradicted hypothesis
    /// - Operations: Service outage, resource exhaustion, SLA breach
    /// - Blockchain: Smart contract vulnerability, oracle manipulation
    Threat,

    /// A favorable condition has been detected.
    /// Default half-life: 4 hours.
    ///
    /// Biological analog: Recruitment pheromone (e.g., trail pheromone
    /// leading to a food source).
    /// Triggers exploration or exploitation behavior.
    ///
    /// Examples across domains:
    /// - Code: Well-tested API ready for integration, refactoring opportunity
    /// - Research: Promising lead, high-impact citation discovered
    /// - Operations: Underutilized resource, optimization opportunity
    /// - Blockchain: Arbitrage opportunity, liquidity provision opening
    Opportunity,

    /// Validated knowledge or insight that should persist.
    /// Default half-life: 24 hours.
    ///
    /// Biological analog: Trail pheromone on an established foraging route
    /// (high persistence, well-confirmed).
    /// Triggers learning and knowledge integration behavior.
    ///
    /// Examples across domains:
    /// - Code: Design pattern that worked well, performance optimization insight
    /// - Research: Validated finding, cross-domain connection discovered
    /// - Operations: Operational runbook entry, incident postmortem lesson
    /// - Blockchain: Market regime characterization, protocol behavior insight
    Wisdom,

    // ── Domain-Specific Kinds ──────────────────────────────────────
    /// First-mover advantage or ephemeral edge.
    /// Default half-life: 1 hour.
    ///
    /// The most ephemeral pheromone kind — alpha signals lose value as more
    /// agents discover them. Named for the financial concept of "alpha"
    /// (excess return above a benchmark), but generalized to any domain.
    ///
    /// Examples across domains:
    /// - Code: Newly discovered approach that hasn't been adopted yet
    /// - Research: Breaking result not yet widely known
    /// - Operations: Transient optimization window (e.g., off-peak pricing)
    /// - Blockchain: MEV opportunity, temporary market inefficiency
    Alpha,

    /// Recurring structure or regularity detected.
    /// Default half-life: 12 hours.
    ///
    /// Signals that an agent has identified a pattern — a repeating structure
    /// in code, data, behavior, or environment that may be actionable.
    ///
    /// Examples across domains:
    /// - Code: Code smell, architectural pattern, dependency pattern
    /// - Research: Recurring theme across papers, methodology pattern
    /// - Operations: Traffic pattern, failure mode pattern, cost pattern
    /// - Blockchain: Trading pattern, gas price pattern, protocol usage pattern
    Pattern,

    /// Something unusual or unexpected detected.
    /// Default half-life: 6 hours.
    ///
    /// Signals deviation from expected behavior. Unlike Threat (which signals
    /// known danger), Anomaly signals the unknown — something that doesn't
    /// fit established patterns and warrants investigation.
    ///
    /// Examples across domains:
    /// - Code: Unexpected test behavior, performance regression, unusual dependency
    /// - Research: Contradictory evidence, outlier result, methodological concern
    /// - Operations: Unusual traffic spike, unexpected resource consumption
    /// - Blockchain: Unusual transaction pattern, protocol parameter anomaly
    Anomaly,

    /// Collective agreement on a fact or decision.
    /// Default half-life: 48 hours.
    ///
    /// The most persistent domain-specific kind. Signals that multiple agents
    /// have independently converged on the same conclusion. Consensus pheromones
    /// are typically created through the confirmation mechanism (see
    /// `03-digital-pheromones.md`) rather than direct deposit.
    ///
    /// Examples across domains:
    /// - Code: Agreed-upon architecture decision, validated design pattern
    /// - Research: Replicated finding, community-accepted methodology
    /// - Operations: Established operational procedure, validated runbook
    /// - Blockchain: Market consensus on fair price, accepted protocol parameters
    Consensus,

    // ── Custom Kinds ───────────────────────────────────────────────
    /// User-defined pheromone kind for domain-specific signals.
    ///
    /// The string identifier must be:
    /// - Alphanumeric + underscores only
    /// - Maximum 64 characters
    /// - Unique within a scope (two Custom pheromones with the same string
    ///   are considered the same kind for confirmation purposes)
    ///
    /// Custom kinds use user-specified decay profiles. If no decay profile
    /// is specified, the default is 6 hours (same as Anomaly).
    ///
    /// Examples:
    /// - `Custom("code_coverage_gap")` — specific code quality signal
    /// - `Custom("dependency_outdated")` — supply chain signal
    /// - `Custom("gas_price_surge")` — blockchain-specific signal
    /// - `Custom("model_drift")` — ML-specific signal
    Custom(String),
}
```

---

## Universal Kinds in Detail

### Threat

The Threat pheromone is the alarm signal of the Roko system. It triggers immediate attention
and prioritized response.

| Property | Value |
|----------|-------|
| Default half-life | 2 hours |
| Default initial intensity | 1.0 |
| Typical agent response | Stop current task, investigate threat, remediate if possible |
| Confirmation threshold | 2 (threats confirmed by 2+ agents become high-priority) |
| Escalation | If intensity > 0.8 and confirmations > 3, escalate to broader scope |

**Threat intensity scaling**: Not all threats are equal. The initial intensity encodes severity:

| Intensity | Severity | Example |
|-----------|----------|---------|
| 0.1–0.3 | Low | Minor code style violation, non-blocking warning |
| 0.4–0.6 | Medium | Test flakiness, moderate performance regression |
| 0.7–0.8 | High | Test failure, security vulnerability in development |
| 0.9–1.0 | Critical | Build failure, security vulnerability in production |

**Interaction with gates**: Threat pheromones at `Mesh` scope influence the adaptive gate
thresholds in `roko-gate`. When the ambient Threat intensity is high, gate thresholds tighten
(more strict verification), implementing a collective immune response.

### Opportunity

The Opportunity pheromone recruits agents toward productive work, analogous to the recruitment
pheromone that guides worker ants toward a rich food source.

| Property | Value |
|----------|-------|
| Default half-life | 4 hours |
| Default initial intensity | 0.8 (lower than Threat, reflecting lower urgency) |
| Typical agent response | Evaluate opportunity, add to task queue if aligned with role |
| Confirmation threshold | 1 (opportunities confirmed by 1+ agents are validated) |
| Competition | Multiple agents may respond to the same opportunity; first to act claims it |

**Opportunity types** (encoded in Engram tags, not in the enum):

| Tag Value | Description | Domain Example |
|-----------|-------------|----------------|
| `integration_ready` | API or component ready to be used | Code: new trait implementation available |
| `refactoring_target` | Code that would benefit from restructuring | Code: duplicated logic across modules |
| `knowledge_gap` | Information needed but not yet available | Research: unanswered question identified |
| `resource_available` | Compute, storage, or budget available | Ops: idle GPU cluster for training |
| `collaboration_possible` | Multiple agents could benefit from coordinating | Any: complementary capabilities detected |

### Wisdom

The Wisdom pheromone encodes validated, durable knowledge — insights that have been tested
and confirmed through operational experience.

| Property | Value |
|----------|-------|
| Default half-life | 24 hours |
| Default initial intensity | 0.9 |
| Typical agent response | Integrate into local knowledge base, apply to current work |
| Confirmation threshold | 3 (wisdom requires strong collective validation) |
| Promotion | Wisdom pheromones with 5+ confirmations may be promoted to permanent Engrams |

**Wisdom creation pathway**: Wisdom pheromones are typically not deposited directly. They emerge
through a pipeline:

```
Agent observes pattern → deposits Pattern pheromone
    ↓
Multiple agents confirm the Pattern
    ↓
Agent validates the pattern through operational testing
    ↓
Agent deposits Wisdom pheromone with the Pattern as parent
    ↓
Other agents confirm the Wisdom
    ↓
At 5+ confirmations, the Wisdom may be promoted to a permanent Engram
```

This pathway ensures that Wisdom pheromones represent collectively validated knowledge, not
individual speculation.

---

## Domain-Specific Kinds in Detail

### Alpha

Alpha signals are the most ephemeral coordination signals. In financial markets, alpha refers
to excess return above a benchmark — a temporary edge that disappears as more participants
discover it. The same concept applies to any domain where first-mover advantage exists.

| Property | Value |
|----------|-------|
| Default half-life | 1 hour |
| Default initial intensity | 1.0 |
| Typical agent response | Act immediately or discard; no value in delayed response |
| Confirmation effect | Paradoxical: confirmation of Alpha *reduces* its value (more agents know about it) |

**Alpha paradox**: Unlike other pheromone kinds where confirmation increases value, confirmation
of an Alpha signal indicates that the first-mover advantage is eroding. When 3+ agents confirm
an Alpha, the signal's effective intensity should decrease (the "alpha" has been discovered by
the crowd). Roko handles this by applying a negative confirmation weight for Alpha pheromones:

```
τ_effective(Alpha) = τ_base × max(0.5, 1 - confirmations × 0.2)
```

This means an Alpha with 3 confirmations has 40% of its original half-life — it fades faster
as more agents discover it, reflecting the real-world dynamics of first-mover advantages.

### Pattern

Pattern signals indicate that an agent has detected a recurring structure or regularity. In
software development, patterns are the primary coordination signal between agents — they encode
information about code quality, architectural decisions, and development conventions.

| Property | Value |
|----------|-------|
| Default half-life | 12 hours |
| Default initial intensity | 0.7 |
| Typical agent response | Incorporate pattern into decision-making; may confirm or contradict |
| Confirmation threshold | 2 (patterns confirmed by 2+ agents are considered reliable) |

**Pattern subtypes** (via Engram tags):

| Tag | Description | Agent Response |
|-----|-------------|----------------|
| `code_smell` | Fowler code smell detected | Refactoring agent investigates |
| `architecture_pattern` | Design pattern identified | Architecture agent evaluates |
| `performance_pattern` | Performance characteristic identified | Optimization agent acts |
| `dependency_pattern` | Dependency relationship identified | Dependency agent manages |
| `testing_pattern` | Testing convention identified | Testing agent follows |

### Anomaly

Anomaly signals flag deviations from expected behavior. Unlike Threats (which signal known
dangers), Anomalies signal the unknown — conditions that don't fit established patterns and
warrant investigation.

| Property | Value |
|----------|-------|
| Default half-life | 6 hours |
| Default initial intensity | 0.8 |
| Typical agent response | Investigate to determine if this is a Threat, Opportunity, or noise |
| Escalation | If investigation confirms danger → re-deposit as Threat with evidence |

**Anomaly triage**: When an agent senses an Anomaly pheromone, it should triage:

1. **Investigate**: Gather more information about the anomalous condition
2. **Classify**: Is this a Threat (danger), Opportunity (hidden value), or noise?
3. **Re-deposit**: Deposit a new pheromone of the appropriate kind with the Anomaly as parent
4. **Discard**: If the anomaly is noise, let it decay naturally

### Consensus

Consensus signals encode collective agreement. They are the most persistent domain-specific
kind because collective agreement is hard-won and should not be easily forgotten.

| Property | Value |
|----------|-------|
| Default half-life | 48 hours |
| Default initial intensity | 0.9 |
| Typical agent response | Treat as established fact; violating consensus requires strong evidence |
| Creation | Usually emerges from confirmation cascade, not direct deposit |

**Consensus formation**: Consensus pheromones typically form through a cascade:

```
Agent A deposits Wisdom: "NaN scores should be clamped to 0.0"
    ↓ (confirmed by Agent B)
    ↓ (confirmed by Agent C)
    ↓ (confirmed by Agent D)
    ↓
At 4 confirmations, the system auto-promotes to Consensus
    ↓
Consensus pheromone deposited with the Wisdom as parent
```

The auto-promotion threshold is configurable (default: 4 confirmations for Wisdom → Consensus
promotion). This ensures that Consensus pheromones represent genuine collective agreement, not
individual assertion.

---

## Custom Kinds

The `Custom(String)` variant enables domain-specific pheromone kinds without modifying the core
enum. Custom kinds participate in all the same mechanisms (decay, confirmation, scoping,
routing) as built-in kinds.

### Registration and Discovery

Custom kinds are registered in `roko.toml`:

```toml
[pheromone.custom_kinds]
# Define custom pheromone kinds with their decay profiles

[pheromone.custom_kinds.code_coverage_gap]
half_life_secs = 28800  # 8 hours
description = "Code coverage below threshold in a module"

[pheromone.custom_kinds.dependency_outdated]
half_life_secs = 86400  # 24 hours
description = "A dependency has a newer version available"

[pheromone.custom_kinds.model_drift]
half_life_secs = 7200   # 2 hours
description = "ML model predictions diverging from observed outcomes"
```

### Domain Plugin Pattern

Each domain plugin (`roko-chain` for blockchain, user-defined for other domains) can define its
own custom pheromone kinds. The domain-agnostic kernel doesn't need to know about these kinds —
it handles them generically through the `Custom(String)` variant.

Example domain-specific kinds:

| Domain | Custom Kind | Half-Life | Description |
|--------|-----------|-----------|-------------|
| Blockchain | `gas_price_surge` | 30 min | Gas prices above 3σ from 24h mean |
| Blockchain | `liquidity_shift` | 2 hours | Significant TVL change in a pool |
| ML/AI | `model_drift` | 2 hours | Prediction accuracy declining |
| ML/AI | `data_quality_issue` | 6 hours | Training data anomaly detected |
| Security | `cve_published` | 24 hours | New CVE affecting a dependency |
| Security | `unusual_access_pattern` | 4 hours | Access pattern deviation |
| Infrastructure | `capacity_threshold` | 1 hour | Resource approaching capacity |
| Infrastructure | `cost_anomaly` | 4 hours | Unexpected cost spike |

---

## Kind Interactions

Pheromone kinds interact in specific ways:

### Threat Suppression

A `Threat` pheromone at high intensity (> 0.7) suppresses `Opportunity` pheromones in the same
scope. Agents in threat-response mode should not be distracted by opportunities until the
threat is resolved. This is analogous to how alarm pheromone in ant colonies overrides
foraging pheromone [Wilson, E.O. "The Insect Societies." Belknap Press, 1971].

### Pattern → Wisdom Promotion

When a `Pattern` pheromone accumulates 3+ confirmations and persists for more than 50% of its
half-life, it becomes eligible for promotion to `Wisdom`. The promotion creates a new
pheromone with the original Pattern as parent, preserving lineage.

### Anomaly → Threat/Opportunity Resolution

`Anomaly` pheromones are inherently temporary. They should resolve into either `Threat`
(danger confirmed), `Opportunity` (hidden value discovered), or natural decay (noise). An
Anomaly that persists at high intensity without resolution may trigger escalation to a broader
scope.

### Consensus Stability

`Consensus` pheromones resist contradiction. To contradict a Consensus pheromone, an agent
must deposit a `Threat` pheromone of equal or greater intensity with explicit evidence. This
prevents casual erosion of collective agreements.

---

## Summary Table

| Kind | Half-Life | Intensity | Confirmation Effect | Biological Analog |
|------|-----------|-----------|--------------------|--------------------|
| Threat | 2h | 1.0 | Extends half-life (standard) | Alarm pheromone |
| Opportunity | 4h | 0.8 | Extends half-life (standard) | Recruitment pheromone |
| Wisdom | 24h | 0.9 | Extends half-life; promotes to Consensus at 4+ | Trail pheromone |
| Alpha | 1h | 1.0 | **Reduces** half-life (paradoxical) | Ephemeral scent mark |
| Pattern | 12h | 0.7 | Extends half-life; promotes to Wisdom at 3+ | Territorial marking |
| Anomaly | 6h | 0.8 | Extends half-life (standard) | Novel scent detection |
| Consensus | 48h | 0.9 | Extends half-life; resists contradiction | Colony odor |
| Custom(String) | User-defined | User-defined | Standard | Domain-specific |

---

## References

- [Bonabeau, Dorigo & Theraulaz 1999] *Swarm Intelligence*, Oxford University Press
- [Grassé 1959] Termite mound stigmergy, *Insectes Sociaux*
- [Nealson, Platt & Hastings 1970] Quorum sensing, *J. Bacteriology*
- [Parunak, Brueckner & Sauter 2005] Digital pheromones, *E4MAS*
- [Wilson 1971] *The Insect Societies*, Belknap Press

---

## Related Sub-Docs

- `03-digital-pheromones.md` — Pheromone struct, decay mechanics, confirmation
- `05-pheromone-scope.md` — How pheromone scope controls propagation
- `06-agent-mesh-sync.md` — Transport layer for pheromone propagation
