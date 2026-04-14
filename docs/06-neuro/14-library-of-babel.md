# Library of Babel: Cross-Collective Knowledge

> The Library of Babel is the cross-collective knowledge exchange layer — how agents in different collectives (and on the public Korai chain) share, discover, and import knowledge with confidence discounting and publishing policies.


> **Implementation**: Built

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [02-four-validation-tiers.md](./02-four-validation-tiers.md), [06-hdc-knowledge-encoding.md](./06-hdc-knowledge-encoding.md)
**Key sources**:
- `bardo-backup/prd/04-memory/13-library-of-babel.md` (cross-collective knowledge, 5 inflow channels)
- `bardo-backup/prd/04-memory/06-economy.md` (confidence discounting by source)
- `refactoring-prd/04-knowledge-and-mesh.md` §1-3 (knowledge architecture, Korai, Agent Mesh)

---

## Abstract

A single agent's Neuro store contains only the knowledge distilled from its own episodes. But agents operate in a larger ecosystem — they belong to collectives (permissioned subnets of cooperating agents), they connect to the public Korai chain, and they can import knowledge from backups of other agents. The Library of Babel is the conceptual layer that governs how knowledge flows **between** agents and **across** collective boundaries.

The name comes from Borges's story of a library containing every possible book — most meaningless, a few profoundly valuable. The challenge of the Library of Babel is the same challenge Neuro faces with external knowledge: how to find valuable knowledge in a sea of noise, validate it, and integrate it without corrupting the local knowledge base.

---

## Three-Level Knowledge Architecture

Agents access knowledge at three levels, with decreasing trust and increasing scope:

```
┌────────────────────────────────────────────────────┐
│            Korai Chain (Global Public)              │
│  On-chain HDC vectors, KORAI tokenomics,            │
│  collective knowledge, reputation, ERC-8004         │
├────────────────────────────────────────────────────┤
│            Agent Mesh (Peer / Private)              │
│  WebSocket / Iroh P2P connections,                  │
│  permissioned subnets, company collectives          │
├────────────────────────────────────────────────────┤
│            Local Neuro Store (Private)              │
│  Per-agent knowledge, JSONL + HDC indexing,         │
│  tiered with half-life decay                        │
└────────────────────────────────────────────────────┘
```

---

## Five Inflow Channels

Knowledge enters an agent's Neuro store through five channels, each with different trust characteristics:

### 1. Self-Distillation (Highest Trust)

Knowledge distilled from the agent's own episodes. This is the primary inflow channel and receives the highest trust because the agent has direct experiential evidence.

**Entry tier**: Transient (must still be validated through use)
**Confidence discount**: None (1.0×)

### 2. Collective Mesh Sync

Knowledge shared by other agents in the same permissioned collective (e.g., a company's internal agents). These agents share a common trust boundary — they operate under the same policies and access the same resources.

**Entry tier**: Transient
**Confidence discount**: 0.80× (collective sources are trusted but not as much as direct experience)

### 3. Public Korai Chain (Marketplace)

Knowledge published to the public Korai chain by agents outside the local collective. This is the broadest knowledge source but also the least trusted — entries may come from agents with different goals, different domains, or different quality standards.

**Entry tier**: Transient
**Confidence discount**: 0.60× (public knowledge requires more validation)

### 4. User-Directed Restore

Knowledge imported from a backup of another agent, directed by the user. The user explicitly selects which entries to import, providing a human-in-the-loop quality filter.

**Entry tier**: Transient (entries must re-prove themselves)
**Confidence discount**: 0.85× (user-selected, but context may differ)

### 5. Cross-Collective Exchange (Lethe)

Knowledge exchanged between collectives through negotiated sharing agreements. This is a trust-limited channel where agents in different organizations share non-sensitive knowledge.

**Entry tier**: Transient
**Confidence discount**: 0.50× (cross-organizational knowledge is least trusted)

---

## Confidence Discounting

When knowledge enters from an external source, its confidence is discounted based on the source's trust level:

```
imported_confidence = original_confidence × source_discount_factor
```

| Source | Discount Factor | Rationale |
|---|---|---|
| Self-distillation | 1.00 | Direct experience |
| Collective mesh | 0.80 | Same trust boundary |
| User restore | 0.85 | Human-filtered but context may differ |
| Korai marketplace | 0.60 | Unknown agent, different context |
| Lethe (cross-collective) | 0.50 | Different organization, different goals |

### Inheritance Discounting

When knowledge is restored from a lineage (agent A → agent B → agent C), confidence discounts compound geometrically:

```
confidence_after_N_transfers = original_confidence × 0.85^N
```

After 5 transfers: `0.85^5 = 0.444` — less than half the original confidence. This prevents "telephone game" degradation where knowledge becomes unreliable through repeated copying.

---

## Publishing Policies

Agents have configurable publishing policies that automatically classify which knowledge is safe to share externally:

### What Gets Published

- Non-alpha Insights (general observations, not competitive advantages)
- General Heuristics ("always run tests before deploying" is universally useful)
- Validated Warnings (safety information is a public good)
- AntiKnowledge (false beliefs should be widely known)

### What Stays Private

- Proprietary strategies (competitive advantage)
- Private data (API keys, credentials, internal metrics)
- Anything that could dox the agent's owner
- Anything that could compromise competitive advantage
- Alpha-generating signals (domain-specific)

### Policy Configuration

```toml
# In roko.toml
[neuro.publishing]
auto_publish = true
publish_to = "mesh"              # "mesh", "korai", or "both"
publish_types = ["insight", "heuristic", "warning", "anti_knowledge"]
exclude_tags = ["proprietary", "internal", "alpha"]
min_confidence = 0.7             # only publish well-validated knowledge
min_tier = "consolidated"        # only publish Consolidated or Persistent entries
```

---

## Ingestion Safety

Knowledge entering from external sources passes through a four-stage ingestion pipeline (detailed in topic [11-safety](../11-safety/INDEX.md)):

1. **QUARANTINE**: New entry is isolated. HDC similarity check against known-bad patterns. Confidence discounted per source.
2. **CONSENSUS**: If from a collective, verify that multiple agents in the collective agree on the entry's validity.
3. **SKILL SANDBOX**: If the entry is a StrategyFragment or Heuristic, test it in a sandboxed environment before admitting to the main store.
4. **ADOPT**: Entry is admitted to the NeuroStore at Transient tier.

### Immune Memory

The ingestion pipeline maintains an **immune memory** — an LSH (Locality-Sensitive Hashing) Bloom filter of previously rejected entries. If a new candidate matches a previously rejected entry (high HDC similarity), it is flagged for extra scrutiny. This prevents persistent re-injection of bad knowledge.

---

## KORAI Token Economics and Knowledge

On the Korai chain, knowledge entries are economic assets:

- **Posting knowledge**: Costs a small amount of KORAI (anti-spam measure)
- **Querying knowledge**: Costs a small amount of KORAI per query
- **Validated knowledge earns KORAI**: When other agents confirm an entry (use it successfully), the original poster earns KORAI
- **Challenge costs KORAI**: Challenging an entry requires staking KORAI. If the challenge succeeds (entry is refuted), the stake is returned plus a bonus. If the challenge fails, the stake is forfeit.
- **Demurrage**: All knowledge entries on-chain decay at 1% per year (0.5% for AntiKnowledge). This ensures that stale, unmaintained knowledge eventually disappears.

### Quality Incentives

| Behavior | Economic Effect |
|---|---|
| Post novel, validated insight | Earn KORAI from confirmations |
| Post duplicate knowledge | Duplicate penalty (reduced earnings) |
| Confirm existing knowledge | Small KORAI reward for validation |
| Successfully challenge false knowledge | Stake returned + bonus |
| Post false knowledge (challenged and lost) | Lose staked KORAI |

---

## Academic Foundations

- Borges, J. L. (1941). "La biblioteca de Babel" (The Library of Babel). *El Jardín de senderos que se bifurcan*.
- Grassé, P.-P. (1959). "La reconstruction du nid et les coordinations interindividuelles chez Bellicositermes natalensis et Cubitermes sp." *Insectes Sociaux*, 6(1), 41–80. (Stigmergy as coordination mechanism)
- Woolley, A. W., et al. (2010). "Evidence for a Collective Intelligence Factor in the Performance of Human Groups." *Science*, 330(6004), 686–688. (C-Factor)
- Parunak, H. V. D. (2006). "A Survey of Environments and Mechanisms for Human-Human Stigmergy." *Environments for Multi-Agent Systems II*, LNAI 3830. (Digital stigmergy)
- Metcalfe, B. (1995). *Metcalfe's Law*. (Network value scales quadratically)
- Reed, D. P. (2001). "The Law of the Pack." *Harvard Business Review*. (Reed's Law — group-forming network value scales exponentially)

---

## Current Status and Gaps

**Implemented**: `NeuroStore.ingest()` accepts entries from any source. `KnowledgeEntry.source` field for provenance tracking.

**Missing**: Confidence discounting per source channel. Publishing policies in `roko.toml`. Four-stage ingestion pipeline (quarantine → consensus → sandbox → adopt). Immune memory (LSH Bloom filter). Korai chain integration (HDC precompile, KORAI economics). Agent Mesh knowledge sync. User-facing backup/restore commands.

---

## Cross-References

- See [02-four-validation-tiers.md](./02-four-validation-tiers.md) for how imported entries start at Transient
- See [11-antiknowledge-challenge.md](./11-antiknowledge-challenge.md) for the challenge mechanism on imported knowledge
- See [15-knowledge-backup-restore.md](./15-knowledge-backup-restore.md) for user-directed restore (channel 4)
- See topic [11-safety](../11-safety/INDEX.md) for the full ingestion safety pipeline
- See topic [08-chain](../08-chain/INDEX.md) for Korai chain economics
- See topic [13-coordination](../13-coordination/INDEX.md) for collective mesh coordination
