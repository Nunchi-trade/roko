# Selective Knowledge Restore

> **Layer**: L1 Framework (Substrate deserialization, knowledge ingestion)
>
> **Prerequisites**: `docs/17-lifecycle/05-knowledge-backup-export.md` (backup format), `docs/17-lifecycle/07-new-agent-creation.md` (new agent creation), `docs/03-neuro/INDEX.md` (Neuro store, Engram format, tier management)
>
> **Synapse traits**: Substrate (receives restored Engrams), Scorer (rescores restored Engrams with confidence decay), Gate (validates restored Engrams against current context), Policy (monitors restore process, emits ingestion events)


> **Implementation**: Specified

---

## Overview

Selective restore is the final step of the four-step knowledge transfer process:

```
BACKUP → DELETE → CREATE → RESTORE
```

It imports knowledge from a predecessor's backup into a new agent's Neuro store, with configurable filtering, confidence decay, and validation. Restore is always selective — the operator controls which knowledge types, confidence thresholds, and decay rates apply.

This replaces the legacy "succession inheritance" system. The mechanism is the same (compressed knowledge transfer with generational confidence decay), but the trigger is user-initiated rather than death-initiated, and the process is explicit rather than automatic.

---

## Restore Command

```bash
# Restore with default confidence decay (0.85× per generation)
roko neuro restore ./backups/agent-V1St-2026-04-12.neuro

# Restore with custom confidence decay
roko neuro restore ./backups/agent-V1St-2026-04-12.neuro --confidence-decay 0.7

# Restore only specific knowledge types
roko neuro restore ./backups/agent-V1St-2026-04-12.neuro --types insight,causal_link

# Restore only high-confidence entries
roko neuro restore ./backups/agent-V1St-2026-04-12.neuro --min-confidence 0.5

# Restore with maximum entry count (genomic bottleneck)
roko neuro restore ./backups/agent-V1St-2026-04-12.neuro --max-engrams 2048

# Restore from Mesh-synced backup
roko neuro restore --from-mesh --agent-id agent-V1StGXR8_Z5j

# Dry-run: show what would be restored
roko neuro restore ./backups/agent-V1St-2026-04-12.neuro --dry-run

# Restore with validation against current context
roko neuro restore ./backups/agent-V1St-2026-04-12.neuro --validate
```

---

## Generational Confidence Decay

The core mechanism that prevents blind inheritance. Every restored Engram's confidence is multiplied by a decay factor:

```
effective_confidence = original_confidence × decay_rate^generation
```

Default decay rate: **0.85 per generation** (configurable via `--confidence-decay`).

This rate was chosen based on the genomic bottleneck research (Shuvaev et al. 2024) and transgenerational epigenetic inheritance studies (Heard & Martienssen 2014):

| Generation | Confidence Multiplier | Effective Confidence (from 0.9) | Interpretation |
|-----------|----------------------|-------------------------------|----------------|
| G0 (original) | 1.000 | 0.900 | Full confidence in self-generated knowledge |
| G1 (first restore) | 0.850 | 0.765 | Slight skepticism of predecessor's knowledge |
| G2 | 0.723 | 0.650 | Moderate skepticism — must validate more |
| G3 | 0.614 | 0.553 | Significant skepticism — inherited knowledge is suggestions |
| G4 | 0.522 | 0.470 | ~Half confidence — validation strongly recommended |
| G5 | 0.444 | 0.399 | Most inherited knowledge below active threshold (0.4) |
| G10 | 0.197 | 0.177 | Only the most robust knowledge survives |
| G15 | 0.087 | 0.079 | Effectively zero — ancestral knowledge fully absorbed or lost |

This implements "survival of the flattest" (Bull et al. 2005) — over many generations, only robust, widely applicable knowledge persists. Fragile, regime-specific knowledge naturally fades. Knowledge that survives 10+ generations of restore cycles is, by construction, the most generalizable knowledge the lineage has ever produced.

### Implementation

```rust
/// Compute the confidence of a restored Engram after generational decay.
///
/// The decay is exponential: each generation multiplies by the decay rate,
/// producing geometric attenuation.
///
/// # Arguments
/// * `original_confidence` - The Engram's confidence at backup time [0.0, 1.0]
/// * `generation` - Generations between the Engram's origin and this restore
/// * `decay_rate` - Per-generation retention rate (default: 0.85)
///
/// # Returns
/// Decayed confidence, clamped to [0.01, original_confidence]
pub fn restore_confidence(
    original_confidence: f64,
    generation: u32,
    decay_rate: f64,
) -> f64 {
    assert!((0.0..=1.0).contains(&original_confidence));
    assert!((0.0..1.0).contains(&decay_rate));

    if generation == 0 {
        return original_confidence;
    }

    let decayed = original_confidence * decay_rate.powi(generation as i32);
    decayed.max(0.01) // Floor at 0.01 to preserve provenance tracking
}
```

---

## Restore Pipeline

The restore process follows a quarantine → validate → adopt pipeline:

### Stage 1: Quarantine

All Engrams from the backup are loaded into a quarantine buffer — they are not immediately added to the Neuro store.

```rust
/// Load backup into quarantine buffer for validation.
pub fn quarantine_backup(
    backup: &BackupArchive,
    config: &RestoreConfig,
) -> Vec<QuarantinedEngram> {
    backup.engrams
        .iter()
        .filter(|e| config.type_filter.accepts(&e.kind))
        .filter(|e| e.score.confidence >= config.min_confidence)
        .take(config.max_engrams.unwrap_or(usize::MAX))
        .map(|e| QuarantinedEngram {
            engram: e.clone(),
            original_confidence: e.score.confidence,
            decayed_confidence: restore_confidence(
                e.score.confidence,
                config.generation,
                config.confidence_decay,
            ),
            provenance_tag: ProvenanceTag::Restored {
                source_agent: backup.manifest.agent_id.clone(),
                source_generation: backup.manifest.generation.unwrap_or(0),
                restore_timestamp: now(),
            },
            validation_status: ValidationStatus::Pending,
        })
        .collect()
}
```

### Stage 2: Validate (Optional)

If `--validate` is specified, each quarantined Engram is checked against current context:

1. **Schema validation**: Engram format matches current Roko version
2. **Contradiction detection**: Restored Engram contradicts existing Engrams in the Neuro store (if any exist)
3. **Provenance verification**: BLAKE3 hash matches content
4. **Regime tagging** (domain-specific): For chain-domain agents, Engrams tagged with specific market regimes are compared against current market conditions. Regime-matched Engrams receive a +0.1 confidence bonus (capped). Non-matching Engrams are deprioritized but retained.

### Stage 3: Adopt

Validated Engrams are adopted into the Neuro store:

```rust
/// Adopt quarantined Engrams into the Neuro store.
pub fn adopt_engrams(
    neuro: &mut NeuroStore,
    quarantined: Vec<QuarantinedEngram>,
) -> RestoreReport {
    let mut report = RestoreReport::default();

    for qe in quarantined {
        if qe.validation_status == ValidationStatus::Rejected {
            report.rejected += 1;
            continue;
        }

        let mut engram = qe.engram;

        // Apply confidence decay
        engram.score.confidence = qe.decayed_confidence;

        // Set tier based on decayed confidence
        engram.tier = tier_from_confidence(qe.decayed_confidence);

        // Add provenance tag
        engram.provenance.push(ProvenanceEntry::Restored {
            source_agent: qe.provenance_tag.source_agent(),
            generation: qe.provenance_tag.generation(),
            timestamp: qe.provenance_tag.timestamp(),
        });

        // Reset decay state (fresh Ebbinghaus curve from restore time)
        engram.decay = DecayState {
            model: DecayModel::Ebbinghaus {
                strength: qe.decayed_confidence,
                scale_ms: default_scale_for_kind(&engram.kind),
            },
            effective_confidence: qe.decayed_confidence,
            ticks_since_access: 0,
            tier_multiplier: tier_multiplier(engram.tier),
        };

        neuro.insert(engram);
        report.adopted += 1;
    }

    report
}

/// Assign tier based on confidence level.
fn tier_from_confidence(confidence: f64) -> KnowledgeTier {
    if confidence >= 0.8 {
        KnowledgeTier::Consolidated
    } else if confidence >= 0.5 {
        KnowledgeTier::Working
    } else {
        KnowledgeTier::Transient
    }
}
```

Note that restored Engrams start at most at `Consolidated` tier — never `Persistent`. The `Persistent` tier (5.0× base half-life, slowest decay) is reserved for knowledge that has been repeatedly validated through the current agent's own experience. This prevents inherited knowledge from receiving the durability benefits that should only come from independent validation.

---

## PLAYBOOK.md Handling

The predecessor's `PLAYBOOK.md` (machine-evolved heuristics) receives special treatment during restore:

- **NOT automatically loaded** as active heuristics
- Stored as a reference document accessible to the agent
- Available for retrieval during planning (the agent can read it) but not automatically applied to decisions
- The new agent must develop its own `PLAYBOOK.md` through Dream integration

This is the anti-proletarianization measure (Stiegler 2010): a successor that blindly follows inherited heuristics without understanding them has been proletarianized. By making inherited heuristics available but not active, the system forces the new agent to independently validate and re-derive its own operational knowledge.

---

## Knowledge Type Priorities

Different knowledge types have different restore priorities:

| Knowledge Type | Priority | Rationale |
|---------------|----------|-----------|
| **Warning** | Highest | Safety-critical: mistakes to avoid. Never filter out warnings. |
| **AntiKnowledge** | High | "What doesn't work" is often more valuable than "what works." |
| **CausalLink** | High | Causal understanding transfers well across regimes. |
| **Insight** | Medium | Useful but may be regime-specific. |
| **Heuristic** | Medium | Practical but may be stale. Subject to highest confidence decay. |
| **StrategyFragment** | Low | Most regime-specific. Requires active validation. |

When `--max-engrams` is specified and the backup exceeds the limit, the compression algorithm prioritizes in the order above.

---

## Restore Report

The restore command produces a summary report:

```
$ roko neuro restore ./backups/agent-V1St-2026-04-12.neuro

Restore Report
==============
Source agent:     agent-V1StGXR8_Z5j
Source generation: 2
This generation:   3
Confidence decay:  0.85 (effective: 0.614 = 0.85^3)

Engrams processed:  12,847
  - Filtered (below threshold): 4,521
  - Quarantined:               8,326
  - Validated:                  8,201
  - Rejected (contradictions):    125
  - Adopted:                    8,201

By type:
  - Insight:           2,341 (avg confidence: 0.42)
  - Heuristic:         1,456 (avg confidence: 0.39)
  - Warning:           1,893 (avg confidence: 0.51)
  - CausalLink:        1,287 (avg confidence: 0.45)
  - StrategyFragment:    812 (avg confidence: 0.36)
  - AntiKnowledge:       412 (avg confidence: 0.47)

By tier (after decay):
  - Transient:  5,421
  - Working:    2,102
  - Consolidated: 678
  - Persistent: 0 (Persistent tier requires independent validation)

PLAYBOOK.md: Stored as reference (not active)

Next steps:
  1. The agent will begin validating restored knowledge through experience
  2. Engrams that prove accurate will be promoted to higher tiers
  3. Engrams that prove inaccurate will decay naturally via Ebbinghaus
  4. The agent's initial 100 iterations will run at elevated exploration (+0.2)
```

---

## Live Restore (Running Agent)

Knowledge can also be restored into a running agent without deletion:

```bash
# Restore into a running agent (merges with existing Neuro)
roko neuro restore ./backups/other-agent.neuro --merge

# Resolve conflicts: prefer existing knowledge
roko neuro restore ./backups/other-agent.neuro --merge --prefer-existing

# Resolve conflicts: prefer restored knowledge
roko neuro restore ./backups/other-agent.neuro --merge --prefer-restored
```

Live restore merges the backup Engrams with the existing Neuro store. Conflicts (same content hash, different scores) are resolved based on the `--prefer` flag. Default: prefer existing (the running agent's knowledge is assumed to be more current).

---

## Cross-Agent Restore

Backups can be restored into agents with different configurations, domains, and strategies. The Engram format is domain-agnostic — the knowledge content is the same regardless of which domain the agent operates in.

However, domain-specific Engrams (e.g., chain-domain Engrams about gas prices) may be less useful in a non-chain-domain agent. The operator should use type filters and confidence thresholds to select relevant knowledge.

Cross-domain knowledge transfer is also possible through HDC (Hyperdimensional Computing) structural analogy. Engrams with HDC vectors can be matched across domains based on structural similarity — a causal pattern discovered in financial markets might have an analogous pattern in code quality metrics. Cross-domain transfer threshold: Hamming similarity ≥ 0.526 for 10,240-bit BSC vectors (see `docs/03-neuro/` for HDC encoding details).

---

## Cross-References

- `docs/17-lifecycle/05-knowledge-backup-export.md` — Backup format specification
- `docs/17-lifecycle/09-knowledge-transfer-via-mesh.md` — Live agent-to-agent knowledge sharing
- `docs/17-lifecycle/10-ebbinghaus-for-knowledge-not-agents.md` — How Ebbinghaus decay works on restored knowledge
- `docs/03-neuro/INDEX.md` — Neuro store, Engram format, tier management
