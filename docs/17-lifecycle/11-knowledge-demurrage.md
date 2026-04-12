# Knowledge Demurrage

> **Layer**: L1 Framework (Substrate decay mechanics) + L4 Orchestration (on-chain token economics, Korai chain)
>
> **Prerequisites**: `docs/17-lifecycle/10-ebbinghaus-for-knowledge-not-agents.md` (Ebbinghaus decay on Engrams), `docs/08-chain/INDEX.md` (Korai chain, KORAI/DAEJI tokens)
>
> **Synapse traits**: Substrate (knowledge store subject to demurrage), Scorer (confidence reduced by demurrage), Policy (demurrage cycles emitted as observability events)


> **Implementation**: Specified

---

## Overview

Knowledge demurrage applies Gesell's Freigeld principle (Gesell 1916) to both knowledge and tokens. Just as Ebbinghaus decay reduces confidence on unused Engrams (see `10-ebbinghaus-for-knowledge-not-agents.md`), KORAI token demurrage reduces the value of held tokens over time. Both mechanisms incentivize circulation over hoarding — use knowledge or lose it, use tokens or lose them.

This document specifies both levels of demurrage:

1. **Knowledge-level demurrage**: Periodic confidence reduction on Engrams that have not been re-validated
2. **Token-level demurrage**: 1% annual demurrage on held KORAI tokens (Korai chain, mainnet only)

---

## Knowledge-Level Demurrage

### The DemurrageConfig

```rust
/// Configuration for knowledge demurrage in the Neuro store.
/// Controls how quickly un-validated knowledge loses confidence.
///
/// Crate: `roko-core`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DemurrageConfig {
    /// Cognitive loop iterations between validation checks.
    /// The Neuro curator checks entries against recent outcomes at this interval.
    /// Default: 250 iterations (~2.9 hours at 1 iteration/40s)
    pub validation_interval: u64,

    /// Confidence loss per missed validation interval.
    /// An Engram not re-validated loses this much confidence each interval.
    /// Default: 0.03 (3% per interval)
    pub decay_per_interval: f64,

    /// Minimum confidence before automatic archiving.
    /// Engrams below this threshold are removed from active context
    /// and moved to cold storage. They persist for audit and provenance
    /// but do not influence ongoing decisions.
    /// Default: 0.1
    pub archive_threshold: f64,

    /// Domain-specific decay multipliers.
    /// Volatile domains decay faster than stable domains.
    /// The multiplier is applied to `decay_per_interval`.
    pub domain_multipliers: HashMap<String, f64>,
}

impl Default for DemurrageConfig {
    fn default() -> Self {
        let mut domain_multipliers = HashMap::new();
        domain_multipliers.insert("gas_patterns".into(), 2.0);      // 6% per interval
        domain_multipliers.insert("price_direction".into(), 1.5);    // 4.5% per interval
        domain_multipliers.insert("volatility_regime".into(), 1.0);  // 3% per interval
        domain_multipliers.insert("yield_trends".into(), 0.8);       // 2.4% per interval
        domain_multipliers.insert("protocol_behavior".into(), 0.5);  // 1.5% per interval

        Self {
            validation_interval: 250,
            decay_per_interval: 0.03,
            archive_threshold: 0.1,
            domain_multipliers,
        }
    }
}
```

### The Demurrage Cycle

Every `validation_interval` iterations, the Neuro curator runs a demurrage cycle:

```rust
/// Apply knowledge demurrage to an Engram.
///
/// Engrams lose confidence proportional to the time since their last validation,
/// scaled by domain-specific multipliers. Engrams that drop below the archive
/// threshold are moved to Archived status.
pub fn apply_demurrage(
    engram: &BackupEngram,
    config: &DemurrageConfig,
    current_iteration: u64,
) -> BackupEngram {
    // Already archived Engrams are not further decayed
    if engram.tier == KnowledgeTier::Archived {
        return engram.clone();
    }

    let iterations_since_validation = current_iteration
        .saturating_sub(engram.last_accessed_at);
    let intervals = iterations_since_validation / config.validation_interval;

    // No decay if within the current validation interval
    if intervals == 0 {
        return engram.clone();
    }

    // Apply domain-specific decay
    let domain = engram.tags.first().map(String::as_str).unwrap_or("default");
    let domain_multiplier = config
        .domain_multipliers
        .get(domain)
        .copied()
        .unwrap_or(1.0);
    let total_decay = config.decay_per_interval * domain_multiplier * intervals as f64;
    let new_confidence = (engram.score.confidence - total_decay).max(0.0);

    // Archive if below threshold
    let new_tier = if new_confidence < config.archive_threshold {
        KnowledgeTier::Archived
    } else {
        engram.tier
    };

    let mut updated = engram.clone();
    updated.score.confidence = new_confidence;
    updated.tier = new_tier;
    updated
}

/// Apply demurrage to all active Engrams in the Neuro store.
/// Called by the curator at each validation interval.
pub fn apply_demurrage_to_all(
    engrams: &[BackupEngram],
    config: &DemurrageConfig,
    current_iteration: u64,
) -> (Vec<BackupEngram>, DemurrageReport) {
    let mut archived_count = 0u32;
    let mut total_confidence_lost = 0.0f64;

    let updated: Vec<BackupEngram> = engrams
        .iter()
        .map(|engram| {
            let updated = apply_demurrage(engram, config, current_iteration);
            if updated.tier == KnowledgeTier::Archived
                && engram.tier != KnowledgeTier::Archived
            {
                archived_count += 1;
            }
            total_confidence_lost += engram.score.confidence - updated.score.confidence;
            updated
        })
        .collect();

    let report = DemurrageReport {
        entries_processed: engrams.len() as u32,
        entries_archived: archived_count,
        total_confidence_lost,
        average_confidence_after: updated
            .iter()
            .map(|e| e.score.confidence)
            .sum::<f64>()
            / updated.len().max(1) as f64,
    };

    (updated, report)
}
```

### What Demurrage Produces

Knowledge demurrage creates five beneficial dynamics, all of which were originally attributed to mortality pressure in the legacy system but are actually produced by knowledge-level decay:

1. **A lean, current Neuro store.** Stale Engrams naturally fade, keeping active context relevant. The agent's decision-making is not polluted by outdated patterns that confidently encode expired conditions.

2. **Natural knowledge turnover.** Old Engrams make room for new ones without explicit deletion. The agent does not need to decide what to forget — the forgetting happens automatically, and only actively validated knowledge persists.

3. **Incentive to explore.** Only fresh evidence maintains Engram confidence, rewarding active engagement. An agent that retreats to passive monitoring pays a knowledge tax that grows with every validation interval. Exploration is not optional — it is the cost of maintaining knowledge.

4. **Forced knowledge circulation.** Engrams approaching the archive threshold are prime candidates for Mesh sharing. The agent's incentive is to share marginal knowledge with peers before it depreciates entirely — better to contribute to the Collective than to let it evaporate. This implements Gesell's Freigeld principle for information (Gesell 1916): knowledge that is not actively used decays in value, forcing it into circulation.

5. **Domain-appropriate decay.** Gas price patterns decay in hours (domain multiplier 2.0×). Protocol behavior knowledge decays over months (domain multiplier 0.5×). The system tracks per-domain knowledge freshness, following Arbesman's domain-specific half-life research (Arbesman 2012).

---

## Token-Level Demurrage (KORAI)

For chain-domain agents operating on the Korai chain, the KORAI token (mainnet) has a 1% annual demurrage rate. DAEJI (testnet) has no demurrage.

### Mechanism

```
balance_effective(t) = balance_raw × (1 - demurrage_rate)^(years_since_last_update)
```

With demurrage_rate = 0.01:

| Time held | Effective balance (from 1000 KORAI) |
|-----------|-------------------------------------|
| 0 days | 1000.00 |
| 30 days | 999.17 |
| 90 days | 997.52 |
| 1 year | 990.00 |
| 5 years | 950.99 |
| 10 years | 904.38 |
| 50 years | 605.03 |

### Why Demurrage

Silvio Gesell (1916) argued that money should mirror the decay of physical goods — a bushel of wheat rots, a machine rusts, but gold endures forever. This asymmetry gives money holders an unfair advantage over goods holders. Demurrage equalizes by making money also "rot" slightly over time.

Applied to KORAI:
- **Incentivizes circulation**: Agents and operators are motivated to use KORAI (stake, trade, fund agents, pay for Mesh services) rather than hoard it
- **Mirrors knowledge decay**: Just as Engrams lose confidence without reinforcement, KORAI tokens lose value without use
- **Prevents wealth concentration**: Long-term holders face gradual dilution, while active participants maintain value
- **Funds ecosystem**: Demurrage proceeds fund protocol development, Mesh infrastructure, and agent subsidies

### Implementation

Demurrage is implemented at the smart contract level on the Korai chain:

```solidity
/// @dev Compute effective balance with demurrage applied.
/// Called on every balance read (ERC-20 balanceOf override).
function _effectiveBalance(address account) internal view returns (uint256) {
    uint256 raw = _rawBalances[account];
    uint256 lastUpdate = _lastUpdateTimestamp[account];
    uint256 elapsed = block.timestamp - lastUpdate;

    // Demurrage: 1% per year = 0.000000031709791983764586% per second
    // Using fixed-point arithmetic to avoid floating-point
    uint256 secondsPerYear = 365.25 days;
    uint256 decayNumerator = 99; // 99/100 = 0.99 per year
    uint256 decayDenominator = 100;

    // Apply compound demurrage for elapsed time
    // For gas efficiency, compute in yearly chunks + remainder
    uint256 fullYears = elapsed / secondsPerYear;
    uint256 remainder = elapsed % secondsPerYear;

    uint256 result = raw;
    for (uint256 i = 0; i < fullYears; i++) {
        result = (result * decayNumerator) / decayDenominator;
    }
    // Linear approximation for sub-year remainder
    result -= (result * remainder) / (secondsPerYear * decayDenominator);

    return result;
}
```

### Relationship to Neuro Demurrage

The two demurrage systems — knowledge-level (Ebbinghaus on Engrams) and token-level (KORAI) — are intentionally parallel:

| Property | Knowledge Demurrage | Token Demurrage |
|----------|-------------------|-----------------|
| **Rate** | 3% per validation interval (configurable) | 1% per year |
| **Counterforce** | Retrieval + validation (testing effect) | Usage (staking, trading, funding) |
| **Threshold** | Archive at confidence < 0.1 | No minimum (balance approaches 0 asymptotically) |
| **Domain sensitivity** | Yes (per-domain multipliers) | No (uniform rate) |
| **Reversibility** | Yes (re-validate to restore confidence) | No (demurrage is permanent) |

The parallel is not decorative — it reflects a deep design principle: **inactive resources decay, active resources persist**. This applies to both knowledge and capital.

---

## Philosophical Grounding

The demurrage design draws on several academic traditions:

### Gesell's Freigeld (1916)

Silvio Gesell proposed "stamp scrip" — money that depreciates at a fixed rate, requiring periodic payment to maintain face value. The goal: force money into circulation, preventing hoarding and deflation. Applied to knowledge: force Engrams into circulation (Mesh sharing) before they depreciate beyond usefulness.

### Ostrom's Commons Governance (1990)

Elinor Ostrom demonstrated that commons (shared resources) can be sustainably managed without privatization or centralized control, given appropriate institutional rules. Knowledge demurrage is an institutional rule for the Neuro commons: it prevents knowledge hoarding (where one agent accumulates Engrams without sharing) and ensures the Collective's knowledge pool remains fresh.

### Richards & Frankland's Forgetting as Optimization (2017)

Richards and Frankland argued that forgetting is not a failure of memory but an active optimization process. The brain forgets to generalize — removing specific details to extract patterns. Knowledge demurrage implements the same principle: by decaying low-confidence, domain-specific knowledge, the Neuro store naturally converges on generalizable patterns.

### Nietzsche's Active Forgetting (1874, 1887)

Nietzsche argued that the capacity to forget is essential for health and action. An organism that cannot forget is paralyzed by the accumulated weight of experience — "it is impossible to live at all without forgetting." Knowledge demurrage is Nietzsche's active forgetting, implemented computationally.

---

## Configuration

```toml
[neuro.demurrage]
# Validation interval (cognitive loop iterations)
validation_interval = 250

# Confidence loss per missed validation interval
decay_per_interval = 0.03

# Archive threshold
archive_threshold = 0.1

# Domain-specific multipliers
[neuro.demurrage.domains]
gas_patterns = 2.0
price_direction = 1.5
volatility_regime = 1.0
yield_trends = 0.8
protocol_behavior = 0.5
```

---

## Telemetry Events

| Event | Payload | Trigger |
|-------|---------|---------|
| `neuro.demurrage_applied` | Entries processed, entries archived, total confidence lost, average confidence | Each demurrage cycle |
| `neuro.engram_archived` | Engram hash, final confidence, domain, age | Individual Engram drops below archive threshold |
| `neuro.knowledge_erosion` | Active count, archived count, average confidence, tier distribution | Significant shift in Neuro health |
| `korai.demurrage_applied` | Account, raw balance, effective balance, elapsed time | Each KORAI balance read |

---

## Related Topics

- `docs/17-lifecycle/10-ebbinghaus-for-knowledge-not-agents.md` — Ebbinghaus decay mechanics
- `docs/08-chain/INDEX.md` — KORAI/DAEJI token economics
- `docs/03-neuro/INDEX.md` — Neuro store, Engram format
- `docs/17-lifecycle/12-academic-foundations.md` — Gesell, Ostrom, Richards & Frankland citations
