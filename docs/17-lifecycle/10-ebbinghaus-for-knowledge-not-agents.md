# Ebbinghaus for Knowledge, Not Agents

> **Layer**: L1 Framework (Substrate decay mechanics) + Cross-cut (Neuro cognitive cross-cut)
>
> **Prerequisites**: `docs/03-neuro/INDEX.md` (Neuro store, Engram format, tier management), `docs/01-synapse/INDEX.md` (Synapse Architecture, Decay enum)
>
> **Synapse traits**: Substrate (implements Ebbinghaus decay on stored Engrams), Scorer (confidence as primary score axis, modified by decay), Composer (budget-aware context assembly must account for decayed confidence when selecting Engrams)


> **Implementation**: Specified

---

## Overview

The Ebbinghaus forgetting curve (Ebbinghaus 1885) is one of the most well-established results in experimental psychology. Hermann Ebbinghaus demonstrated that memory retention follows a negative exponential decay: recently learned information fades rapidly at first, then more slowly over time. The mathematical form is:

```
retention = e^(-t / (strength × scale))
```

where `t` is time since last access, `strength` is a measure of how well the memory was encoded, and `scale` is a time constant.

In the legacy Bardo architecture, Ebbinghaus decay was applied at two levels: (1) knowledge entries in the Grimoire, and (2) agent lifespan via the epistemic death clock. The epistemic death clock measured predictive fitness — when an agent's world-model decayed too far from reality, the agent died.

**In Roko, Ebbinghaus applies to knowledge only — never to agent lifespan.** Engrams in the Neuro store decay according to the Ebbinghaus curve, but the agent itself does not die from knowledge staleness. Knowledge staleness triggers tier demotion (Consolidated → Working → Transient), Daimon behavioral state transitions (Engaged → Struggling), and eventually knowledge archival — but the agent continues running. The user decides when to delete the agent.

This document specifies how Ebbinghaus decay works on Engrams, the tier system that modulates decay rates, the testing effect that counteracts decay, and why this approach is strictly superior to using Ebbinghaus for agent lifespan.

---

## The Decay Enum

The Synapse Architecture defines four decay variants on Engrams (see `refactoring-prd/01-synapse-architecture.md`):

```rust
/// Decay behavior for an Engram's confidence over time.
/// Attached to each Engram at creation. Determines how confidence
/// degrades without reinforcement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Decay {
    /// No decay. Confidence remains constant.
    /// Used for: architectural facts, mathematical constants, immutable rules.
    None,

    /// Exponential half-life decay.
    /// confidence(t) = initial × 0.5^(t / half_life_ms)
    /// Used for: time-sensitive observations with known shelf life.
    HalfLife { half_life_ms: u64 },

    /// Time-to-live. Binary: full confidence until TTL, then 0.
    /// Used for: ephemeral data (price quotes, gas estimates).
    Ttl { expires_at: u64 },

    /// Ebbinghaus forgetting curve.
    /// retention = e^(-t / (strength × scale_ms))
    /// Used for: most knowledge types (insights, heuristics, warnings).
    Ebbinghaus { strength: f64, scale_ms: u64 },
}
```

**`Decay::Ebbinghaus`** is the default for most knowledge types. The `strength` parameter encodes how well the Engram was encoded — higher strength means slower decay. The `scale_ms` parameter is the time constant in milliseconds.

---

## Tier-Modulated Decay

Engram decay rate is modulated by knowledge tier. Higher tiers decay more slowly because they represent more thoroughly validated knowledge:

| Tier | Multiplier | Effective Decay | What it means |
|------|-----------|----------------|---------------|
| **Transient** | 0.1× | Very fast | Recently created, unvalidated. Decays rapidly unless used. |
| **Working** | 0.5× | Moderate | Used but not consolidated. Decays at moderate rate. |
| **Consolidated** | 1.0× | Standard | Validated through experience. Standard Ebbinghaus rate. |
| **Persistent** | 5.0× | Very slow | Repeatedly validated, high confidence. Resists decay. |

The effective decay formula is:

```
effective_decay = tier_multiplier × type_base_half_life
```

Where `type_base_half_life` comes from the knowledge type configuration:

| Knowledge Type | Base Half-Life | Rationale |
|---------------|---------------|-----------|
| **Insight** | 168 hours (1 week) | Observations and interpretations. Moderate shelf life. |
| **Heuristic** | 336 hours (2 weeks) | Rules of thumb. Longer shelf life if validated. |
| **Warning** | 72 hours (3 days) | Safety-critical knowledge. Short shelf life to prevent stale warnings. |
| **CausalLink** | 504 hours (3 weeks) | Causal relationships. Longer shelf life, structural knowledge. |
| **StrategyFragment** | 168 hours (1 week) | Tactical knowledge. Regime-sensitive, moderate shelf life. |
| **AntiKnowledge** | 720 hours (30 days) | "What doesn't work." Long shelf life — negative knowledge is stable. |

### Example Decay Rates

A Warning Engram at Transient tier:
- Base half-life: 72 hours
- Tier multiplier: 0.1×
- Effective half-life: 7.2 hours
- This Warning will lose half its confidence in ~7 hours unless retrieved

A CausalLink Engram at Persistent tier:
- Base half-life: 504 hours
- Tier multiplier: 5.0×
- Effective half-life: 2,520 hours (~105 days)
- This CausalLink will retain confidence for months

---

## The Testing Effect: Retrieval Counteracts Decay

Roediger & Karpicke (2006) demonstrated that retrieving information from memory strengthens the memory trace more effectively than re-studying. In Roko, every time an Engram is retrieved from the Neuro store and used in a cognitive loop iteration, its `strength` parameter increases:

```rust
/// Update Engram strength after successful retrieval.
/// The testing effect: retrieval strengthens memory more than re-study.
pub fn apply_testing_effect(
    engram: &mut BackupEngram,
    retrieval_context: &RetrievalContext,
) {
    // Base strength increase from retrieval
    let base_increase = 0.05;

    // Bonus if the Engram was used in a turn that passed gates
    let gate_bonus = if retrieval_context.gate_passed { 0.03 } else { 0.0 };

    // Bonus if the Engram was retrieved under diverse conditions
    // (different Daimon states, different tasks)
    let diversity_bonus = retrieval_context.context_diversity * 0.02;

    let total_increase = base_increase + gate_bonus + diversity_bonus;

    // Update strength (used in Ebbinghaus formula)
    if let Decay::Ebbinghaus { ref mut strength, .. } = engram.decay.model {
        *strength = (*strength + total_increase).min(10.0);
    }

    // Reset ticks_since_access (restarts the decay clock)
    engram.decay.ticks_since_access = 0;

    // Increment retrieval count
    engram.retrieval_count += 1;

    // Update last_accessed_at
    engram.last_accessed_at = now();
}
```

This creates a natural selection pressure on knowledge: Engrams that are frequently retrieved and prove useful (gate passes) accumulate strength and resist decay. Engrams that sit unused decay rapidly. The Neuro store self-prunes without explicit deletion — the forgetting curve handles it.

---

## Why Ebbinghaus for Agent Lifespan Was Wrong

The legacy system used epistemic fitness (prediction accuracy) as an agent death clock. When an agent's predictions became too inaccurate (fitness < 0.35), it entered a senescence cascade ending in death. The research grounding was sound:

- **91% of ML models degrade temporally** (Vela et al. 2022)
- **Knowledge has measurable half-lives** (Arbesman 2012)
- **Concept drift formalizes decay** (Zliobaitė et al. 2014, Lu et al. 2020)
- **Expertise creates entrenchment** (Dane 2010)
- **Retraining from scratch outperforms continuous adaptation** (Van de Ven et al. 2024)
- **Optimal reset interval scales with volatility** (Besbes, Gur & Zeevi 2019)

All of these findings are valid. The error was applying them to agent lifespan instead of to knowledge management:

### The Category Error

When a biological organism's cells accumulate damage, the organism dies because repair mechanisms have finite fidelity. The damage is in the physical substrate and is irreversible at the organism level (though the species survives via reproduction).

When an agent's knowledge becomes stale, the knowledge can be refreshed, replaced, or restored — because knowledge is digital, not physical. The "damage" (staleness) is fully reversible. You don't need to kill the agent to fix stale knowledge. You can:

1. Let Ebbinghaus decay naturally prune stale Engrams
2. Run Dream consolidation to reorganize and refresh knowledge
3. Restore fresh knowledge from a backup or Mesh
4. Simply delete stale Engrams and let the agent re-learn

### What Ebbinghaus for Knowledge Achieves

All the benefits attributed to agent mortality are achieved by knowledge-level Ebbinghaus decay:

| Benefit attributed to agent death | How knowledge decay achieves it |
|----------------------------------|-------------------------------|
| **Stale knowledge purged** | Ebbinghaus decay reduces confidence of unused Engrams, eventually archiving them |
| **Active exploration incentivized** | Only fresh evidence maintains Engram confidence — the agent must explore to keep knowledge alive |
| **Knowledge sharing before loss** | Engrams approaching archive threshold are prime candidates for Mesh sharing (Gesell's Freigeld) |
| **Lean, current knowledge base** | Natural turnover: old Engrams make room for new ones without explicit deletion |
| **Domain-specific decay rates** | Gas knowledge decays fast (hours), protocol knowledge decays slow (months) — via per-type half-lives |

### What Knowledge Decay Avoids

| Problem with agent death | How knowledge decay avoids it |
|-------------------------|------------------------------|
| **Arbitrary termination** | No stochastic death clock — agent runs until user decides otherwise |
| **Terminal state behavioral distortion** | No "dying agent" behavior — agent always operates at full capability |
| **Succession overhead** | No death protocol, no testament generation — backup/restore when needed |
| **User frustration** | User controls lifecycle, not an opaque mortality algorithm |
| **Category error** | Decay applies to knowledge (which genuinely degrades) not to processes (which don't) |

---

## Domain-Specific Decay Rates

Different domains have different knowledge half-lives, following Arbesman's research (2012). The Neuro configuration supports domain-specific decay multipliers:

```toml
[neuro.domain_decay]
# Domain-specific decay multipliers (applied to type base half-life)
# Higher = faster decay; Lower = slower decay
gas_patterns = 2.0       # Gas knowledge decays 2x faster
price_direction = 1.5    # Price knowledge decays 1.5x faster
volatility_regime = 1.0  # Standard rate
yield_trends = 0.8       # Yield knowledge decays 0.8x rate
protocol_behavior = 0.5  # Protocol knowledge decays 0.5x rate (most stable)
```

These multipliers are configurable per agent. A chain-domain agent monitoring gas markets needs fast gas decay (gas patterns change hourly). A research agent tracking scientific literature needs slow decay (scientific findings are relatively stable).

---

## Knowledge Demurrage Connection

Ebbinghaus decay at the Engram level has a mirror at the token level for chain-domain agents. KORAI tokens have a 1% annual demurrage rate — held tokens lose value over time, just as held knowledge loses confidence. Both mechanisms incentivize circulation over hoarding: use your knowledge (retrieve Engrams, apply them) or lose it; use your tokens (stake, trade, fund agents) or lose them.

See `docs/17-lifecycle/11-knowledge-demurrage.md` for the full demurrage specification.

---

## Ebbinghaus Decay Implementation

```rust
/// Compute current retention for an Engram under Ebbinghaus decay.
///
/// retention = e^(-t / (strength × scale_ms))
///
/// where t = time since last access (in ms).
pub fn ebbinghaus_retention(
    time_since_access_ms: u64,
    strength: f64,
    scale_ms: u64,
) -> f64 {
    let t = time_since_access_ms as f64;
    let denominator = strength * scale_ms as f64;
    if denominator <= 0.0 {
        return 0.0;
    }
    (-t / denominator).exp()
}

/// Compute effective confidence for an Engram, accounting for
/// Ebbinghaus decay and tier multiplier.
pub fn effective_confidence(engram: &BackupEngram) -> f64 {
    match engram.decay.model {
        DecayModel::None => engram.score.confidence,
        DecayModel::HalfLife { half_life_ms } => {
            let t = engram.decay.ticks_since_access as f64 * TICK_DURATION_MS;
            engram.score.confidence * (0.5_f64).powf(t / half_life_ms as f64)
        }
        DecayModel::Ttl { expires_at } => {
            if now_ms() > expires_at { 0.0 } else { engram.score.confidence }
        }
        DecayModel::Ebbinghaus { strength, scale_ms } => {
            let t = engram.decay.ticks_since_access as f64 * TICK_DURATION_MS;
            let retention = ebbinghaus_retention(t as u64, strength, scale_ms);
            engram.score.confidence * retention * engram.decay.tier_multiplier
        }
    }
}
```

---

## Tier Promotion and Demotion

Engrams move between tiers based on validation:

```
Transient → Working: Engram retrieved and used in 3+ gate-passed turns
Working → Consolidated: Engram validated through 10+ independent experiences
Consolidated → Persistent: Engram confirmed across 3+ distinct contexts (different tasks, different time periods)

Persistent → Consolidated: Confidence drops below 0.6
Consolidated → Working: Confidence drops below 0.4
Working → Transient: Confidence drops below 0.2
Transient → Archived: Confidence drops below 0.05
```

Tier demotion happens automatically via Ebbinghaus decay. Tier promotion requires active validation — the agent must use the Engram and confirm it against ground truth (via Gate verification).

---

## Cross-References

- `docs/17-lifecycle/11-knowledge-demurrage.md` — Token-level knowledge decay
- `docs/17-lifecycle/12-academic-foundations.md` — Full Ebbinghaus and decay citations
- `docs/03-neuro/INDEX.md` — Neuro store architecture
- `docs/01-synapse/INDEX.md` — Decay enum specification
