# Agent Lifecycle

The **Agent Lifecycle** topic documents how Roko agents are created, configured, funded, operated, backed up, deleted, and recreated — and how knowledge flows between agent generations through user-controlled backup/restore. This topic replaces the legacy Bardo "mortality" system entirely. Where the old architecture treated agents as mortal organisms with death clocks and succession protocols, the new architecture treats agents as user-directed software processes with explicit lifecycle commands. Every academic citation from the mortality research (130+ papers) is preserved and reframed: Ebbinghaus curves now govern knowledge freshness instead of agent lifespan, somatic markers drive strategy retrieval instead of mortality anxiety, and the Baldwin Effect describes backup/restore capacity transfer instead of death-triggered succession.

---

## Prerequisites

Before reading this topic, familiarity with these concepts is helpful (each is briefly redefined in the relevant sub-doc):

- **Engram** — content-addressed, scored, decaying unit of cognition (see `docs/01-architecture/`)
- **Synapse Architecture** — the 6-trait composition: Substrate, Scorer, Gate, Router, Composer, Policy (see `docs/01-architecture/`)
- **Neuro / NeuroStore** — semantic knowledge store wrapping Substrate (see `docs/03-neuro/`)
- **Daimon** — PAD affect engine driving behavioral states (see `docs/04-daimon/`)
- **Agent Mesh / Collective** — P2P knowledge sharing network (see `docs/09-mesh/`)
- **KORAI / DAEJI** — mainnet / testnet tokens (see `docs/08-chain/`)
- **5-Layer Taxonomy** — L0 Runtime, L1 Framework, L2 Scaffold, L3 Harness, L4 Orchestration (see `docs/01-architecture/`)

---

## Table of Contents

| # | Sub-doc | Lines | Summary |
|---|---------|-------|---------|
| 00 | [00-vision-and-mortality-replaced.md](00-vision-and-mortality-replaced.md) | 212 | Why the mortality thesis was a category error; complete catalog of removed, kept, and reframed concepts |
| 01 | [01-agent-creation.md](01-agent-creation.md) | 361 | Three-interaction creation flow (Describe/Review/Confirm), CLI and API flows, custody modes, strategy templates |
| 02 | [02-provisioning.md](02-provisioning.md) | 347 | Three deployment paths, type-state provisioning pipeline, warm pool, machine lifecycle states |
| 03 | [03-configuration-and-operator-model.md](03-configuration-and-operator-model.md) | 324 | Four config files, hot-reload, operator freedom hierarchy (5 levels), config validation |
| 04 | [04-funding-and-budgets.md](04-funding-and-budgets.md) | 310 | Budget allocation, cost tracking, multi-level guardrails, graceful degradation cascade, four funding sources |
| 05 | [05-knowledge-backup-export.md](05-knowledge-backup-export.md) | 345 | `roko neuro backup` command, archive format, genomic bottleneck compressed backups |
| 06 | [06-agent-deletion.md](06-agent-deletion.md) | 250 | `roko delete` command, 8-step clean shutdown, 30s per-step budget, force deletion |
| 07 | [07-new-agent-creation.md](07-new-agent-creation.md) | 215 | Three successor patterns (Clean/Same Strategy/Lineage), identity and continuity, anti-proletarianization |
| 08 | [08-selective-restore.md](08-selective-restore.md) | 338 | `roko neuro restore` command, 0.85^N confidence decay, quarantine/validate/adopt pipeline, cross-agent restore |
| 09 | [09-knowledge-transfer-via-mesh.md](09-knowledge-transfer-via-mesh.md) | 315 | Collective sharing, version-vector sync, Bloom filter discovery, four-tier gossip, stigmergy |
| 10 | [10-ebbinghaus-for-knowledge-not-agents.md](10-ebbinghaus-for-knowledge-not-agents.md) | 293 | Forgetting curve for knowledge freshness, tier-modulated decay, testing effect, tier promotion/demotion |
| 11 | [11-knowledge-demurrage.md](11-knowledge-demurrage.md) | 326 | Token-level knowledge decay, KORAI 1% annual demurrage, philosophical grounding |
| 12 | [12-academic-foundations.md](12-academic-foundations.md) | 441 | Complete citation catalog (85+ unique citations across 13 research domains), legacy vs new framing |

**Total**: 13 sub-docs, 4077 lines

---

## Core Lifecycle

The agent lifecycle is a user-directed sequence of explicit commands:

```
CREATE  →  CONFIGURE  →  FUND  →  RUN  →  BACKUP  →  DELETE  →  CREATE  →  RESTORE
  01          03          04      (runtime)   05        06        07         08
```

Each step is an independent CLI command. No step triggers automatically. The operator controls the entire lifecycle.

---

## Knowledge Transfer Across Generations

The four-step knowledge transfer process replaces legacy "succession":

```
1. roko neuro backup    →  Serialize Engrams, scores, tiers, provenance, decay state
2. roko delete          →  Clean shutdown, free resources
3. roko init            →  New agent with fresh ID, fresh Neuro, fresh Daimon
4. roko neuro restore   →  Selective import with 0.85^N generational confidence decay
```

Live agents can also share knowledge via the Agent Mesh (`09-knowledge-transfer-via-mesh.md`).

---

## Related Topics

| Topic | Path | Relationship |
|-------|------|-------------|
| Architecture | `docs/01-architecture/` | 5-layer taxonomy, Synapse Architecture — lifecycle features map to specific layers |
| Neuro | `docs/03-neuro/` | Engram storage, knowledge types, tier management — lifecycle manages Neuro state |
| Daimon | `docs/04-daimon/` | PAD affect engine, behavioral states — lifecycle creates/resets Daimon state |
| Dreams | `docs/05-dreams/` | Consolidation cycle — lifecycle backup captures post-dream knowledge |
| Mesh | `docs/09-mesh/` | Agent Mesh, collective intelligence — live knowledge transfer alternative |
| Chain | `docs/08-chain/` | KORAI/DAEJI tokens, ERC-8004 identity — chain domain lifecycle extensions |
| Compute | `docs/11-compute/` | Hosted VM provisioning — managed deployment path |
| Spectre | `docs/14-spectre/` | Visual display of cognitive state — reflects lifecycle state changes |

---

## Generation Notes

- **Sub-docs produced**: 13
- **Total line count**: 4077
- **Key legacy sources consulted**:
  - `bardo-backup/prd/02-mortality/` — 08-mortality-affect.md (628 lines), 02-epistemic-decay.md (~1000 lines), 07-succession.md (~840 lines), 15-references.md (162 citations), 14-research-foundations.md (cited but not directly read — citations extracted via 15-references.md)
  - `bardo-backup/prd/04-memory/03-mortal-memory.md` — (66KB, first 500 lines read for knowledge management context)
  - `bardo-backup/prd/01-golem/06-creation.md` — (first 500 lines read for creation flow)
  - `bardo-backup/prd/11-compute/` — 00-overview.md, 01-architecture.md, 02-provisioning.md (full reads for deployment and VM lifecycle)
  - `refactoring-prd/` — 00-architecture.md, 03-universal-loop.md, 09-innovations.md, 02-engram-spec.md (all consulted for new framing)
  - `context-pack/` — 01-naming-map.md, 02-reframe-rules.md, 03-concepts-lifecycle.md, 04-writing-rules.md
- **Decisions requiring judgment**:
  - The "Thriving → Terminal" language appears in `00-vision-and-mortality-replaced.md` and `12-academic-foundations.md` solely in the context of explaining what was removed. This is consistent with the writing rule permitting old names in quotes with parenthetical explanations.
  - `04-memory/03-mortal-memory.md` was only partially read (first 500 of ~1500 lines) due to file size. The thesis and key concepts were captured; deeper detail on mortal memory integration patterns may need a follow-up pass.
  - The citation catalog in `12-academic-foundations.md` contains 85+ unique citations across 13 domains. The full 162-entry legacy reference list in `15-references.md` uses an internal citation-key format; every substantive citation that appeared in the sub-docs was included, with full bibliographic detail where available from the source material. Some legacy citation keys (e.g., `NAKAMOTO-2008`, `SZABO-1997`) appear only in chain-specific contexts and were included in the chain/economics domains.
- **Unresolved tensions**:
  - The boundary between "knowledge demurrage" (doc 11) and "Ebbinghaus decay" (doc 10) overlaps conceptually. Both describe knowledge losing value over time. The distinction: Ebbinghaus is the in-memory decay model (how the agent forgets), while demurrage is the on-chain economic incentive (how the token system encourages knowledge refresh). This is an architectural tension that implementation will need to resolve clearly.
  - The "anti-proletarianization" measure (Stiegler 2010) — requiring restored knowledge to diverge by >= 0.15 to be considered healthy — needs careful calibration in implementation. Too high a threshold makes restore useless; too low makes it cargo-cult inheritance.
