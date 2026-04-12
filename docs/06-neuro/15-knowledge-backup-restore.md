# Knowledge Backup and Restore

> Users control knowledge lifecycle through a four-step BACKUP→DELETE→CREATE→RESTORE process, replacing the legacy succession model with explicit, auditable data management.

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [02-four-validation-tiers.md](./02-four-validation-tiers.md), [14-library-of-babel.md](./14-library-of-babel.md)
**Key sources**:
- `refactoring-prd/04-knowledge-and-mesh.md` §5 (Knowledge Backup & Restore)
- `context-pack/02-reframe-rules.md` (Succession → Backup/Restore)
- `context-pack/03-concepts-lifecycle.md` (Removed: succession, generational transfer)

---

## Abstract

In the legacy Bardo (now Roko) architecture, knowledge transfer between agents was framed as "succession" — a dying agent ran a "thanatopsis" phase where it selected knowledge to pass to a new agent. This model has been completely replaced. Agents do not die, do not choose successors, and do not automatically transfer knowledge.

Knowledge transfer in Roko is **user-directed**. Users explicitly export, select, and import knowledge through a four-step process: BACKUP the source agent's NeuroStore, DELETE the source agent (optional), CREATE a new agent, and RESTORE selected knowledge from the backup. Restored entries start at Transient tier — they must re-prove themselves in the new context. Provenance tracks the origin: "restored from agent X on date Y."

This design reflects three principles:
1. **User agency**: Users decide what knowledge moves where, not the agent
2. **Auditability**: Every knowledge transfer is explicitly logged and traceable
3. **Context awareness**: Knowledge that worked in one context may not work in another — starting at Transient tier forces revalidation

---

## The Four-Step Process

### Step 1: BACKUP

Export the agent's full NeuroStore to a portable format.

```bash
roko neuro backup --agent <agent-id> --output <path>
```

**What gets exported**:
- All `KnowledgeEntry` objects (JSONL format)
- HDC vectors for each entry (binary, 1,280 bytes each)
- Tier metadata (current tier, confirmation count, promotion history)
- Provenance chain (source episodes, original creation dates)
- KnowledgeStats snapshot (aggregate statistics at time of backup)
- Somatic markers (if SomaticLandscape is implemented)

**What does NOT get exported**:
- Episode logs (too large, too specific to the original agent's context)
- Daimon PAD state (internal to the original agent)
- Active task state (transient runtime information)
- Credentials, API keys, or configuration secrets

**Backup format**:

```
backup_2026-04-10T22:00:00Z/
├── manifest.json           # Backup metadata: agent ID, date, entry count, stats
├── knowledge.jsonl         # All knowledge entries (one per line)
├── hdc_vectors.bin         # Binary HDC vectors (1,280 bytes × N entries)
├── tier_metadata.jsonl     # Tier status for each entry
├── provenance.jsonl        # Source episodes and lineage for each entry
└── somatic_markers.jsonl   # Somatic landscape markers (if available)
```

### Step 2: DELETE (Optional)

User explicitly deletes the source agent if it is no longer needed.

```bash
roko agent delete <agent-id>
```

This step is optional — the user may keep the source agent running while also restoring its knowledge into a new agent. Deletion removes the agent and its local store but does not affect the backup.

### Step 3: CREATE

User creates a new agent with a fresh NeuroStore.

```bash
roko agent create --name <new-agent-name> --config roko.toml
```

The new agent starts with an empty knowledge base, no somatic markers, and a neutral Daimon PAD state.

### Step 4: RESTORE

User selectively imports knowledge from the backup into the new agent.

```bash
roko neuro restore --from <backup-path> --agent <new-agent-id> [--filter <options>]
```

**Filter options**:
- `--types insight,heuristic` — only restore specific knowledge types
- `--min-confidence 0.5` — only restore entries above a confidence threshold
- `--tags rust,async` — only restore entries matching specific tags
- `--exclude-tags proprietary` — exclude entries with specific tags
- `--max-entries 1000` — limit the number of restored entries
- `--all` — restore everything (with confidence discount applied)

**Restore behavior**:
1. Each selected entry is imported into the new agent's NeuroStore
2. Confidence is discounted by 0.85× (user-restore discount factor)
3. Tier is reset to **Transient** — entries must re-prove themselves
4. Provenance is updated: `source: "restored from agent {id} on {date}"`
5. `created_at` is preserved (original creation date, for decay computation)
6. HDC vectors are preserved (same encoding, no recomputation needed)

### Why Entries Start at Transient

Restored entries start at Transient tier (0.1× decay multiplier) for three reasons:

1. **Context mismatch**: Knowledge that worked for agent A may not work for agent B. Different agents may have different roles, different tools, different domains. Starting at Transient forces the entry to be validated in the new context before it gets the durability of a higher tier.

2. **Staleness risk**: The backup may be days, weeks, or months old. Starting at Transient ensures that time-sensitive knowledge (Warnings, StrategyFragments) decays appropriately if not quickly confirmed.

3. **Safety**: Prevents imported knowledge from immediately influencing critical decisions at full confidence. The agent must have at least one positive experience with the entry before it is promoted to Working tier.

---

## Comparison: Legacy Succession vs. New Backup/Restore

| Aspect | Legacy Succession | New Backup/Restore |
|---|---|---|
| **Trigger** | Agent approaching death | User command at any time |
| **Decision maker** | The dying agent | The user |
| **Knowledge selection** | Agent selects automatically | User selects with filters |
| **Transfer direction** | Parent → child (one-way, one-time) | Any agent → any agent (repeatable) |
| **Tier on arrival** | Inherited (parent's tier × 0.85) | Transient (must re-prove) |
| **Provenance** | Lineage tracking (generational) | Source tracking (flat) |
| **Timing** | During thanatopsis phase | Anytime |
| **Mortality requirement** | Agent must be dying | No mortality concept |
| **Reversibility** | Irreversible (agent is dead) | Reversible (backup persists) |

---

## Mesh-Based Knowledge Sharing

In addition to explicit backup/restore, agents share knowledge through the Agent Mesh:

### Collective Sync

Agents in the same collective automatically synchronize certain knowledge categories:

```toml
# In roko.toml
[neuro.mesh_sync]
enabled = true
sync_types = ["warning", "anti_knowledge"]  # share dangers collectively
sync_interval = "1h"                         # sync every hour
min_confidence = 0.7                         # only share well-validated entries
```

Mesh-synced entries enter with the collective confidence discount (0.80×) and at Transient tier.

### Publishing to Korai

Agents can publish validated knowledge to the public Korai chain:

```bash
roko neuro publish --entry <entry-id>
roko neuro publish --type heuristic --min-confidence 0.8 --min-tier consolidated
```

Published entries are encoded as HDC vectors on-chain and can be discovered by any agent using the on-chain HDC precompile.

---

## Academic Foundations

- Tulving, E. (1972). "Episodic and semantic memory." In *Organization of Memory*. Academic Press. (Distinction between episodic and semantic transfer)
- McClelland, J. L., et al. (1995). "Complementary learning systems." *Psychological Review*, 102(3). (Why transferred knowledge needs reconsolidation)
- Nader, K., Schafe, G. E., & Le Doux, J. E. (2000). "Fear memories require protein synthesis in the amygdala for reconsolidation after retrieval." *Nature*, 406, 722–726. (Reconsolidation — retrieved memories become labile and must be re-stabilized)

---

## Current Status and Gaps

**Implemented**: `NeuroStore.ingest()` supports bulk entry ingestion. `KnowledgeEntry.source` field for provenance. All data structures support serialization/deserialization.

**Missing**:
- `roko neuro backup` CLI command (export NeuroStore to portable format)
- `roko neuro restore` CLI command with filter options (type, confidence, tags, max-entries)
- Confidence discounting per source (0.85× for user restore, 0.80× for collective, 0.60× for marketplace)
- Tier reset to Transient on import (ensuring revalidation in new context)
- Provenance update on restore (source agent ID and date recorded)
- Mesh sync implementation (automatic collective knowledge sharing)
- `roko neuro publish` CLI command (push validated entries to Korai chain)
- Backup format specification (manifest.json + knowledge.jsonl + hdc_vectors.bin)
- Selective restore tooling (interactive entry selection UI)
- Lineage tracking with geometric confidence discount (0.85^N per transfer)
- Backup integrity verification (BLAKE3 checksums on backup files)

---

## Cross-references

- See [02-four-validation-tiers.md](./02-four-validation-tiers.md) for why Transient tier is the entry point
- See [14-library-of-babel.md](./14-library-of-babel.md) for the five inflow channels and confidence discounting
- See [10-knowledge-query-api.md](./10-knowledge-query-api.md) for the NeuroStore ingest API
- See topic [13-coordination](../13-coordination/INDEX.md) for Agent Mesh connectivity
- See topic [08-chain](../08-chain/INDEX.md) for Korai chain publishing
