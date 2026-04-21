# roko-neuro

Durable knowledge store and memory subsystems for Roko.

## What it does

Manages long-lived knowledge entries (insights, heuristics, warnings, causal links, strategy
fragments) with temporal decay, confidence tracking, emotional provenance, and tier-based
retention. Provides distillation from raw episodes into durable knowledge and HDC-based
clustering for worldview preservation during garbage collection.

## Key types and modules

- `KnowledgeEntry` -- core struct: content + kind + confidence + decay + emotional provenance
- `KnowledgeKind` -- enum: `Insight`, `Heuristic`, `AntiKnowledge`, `Warning`, `CausalLink`, `StrategyFragment`
- `KnowledgeTier` -- retention tiers: `Transient`, `Working`, `Consolidated`, `Persistent`
- `NeuroStore` trait -- init/query/ingest interface for knowledge backends
- `knowledge_store` -- file-backed JSONL store with `MemoryIndex` for fast retrieval
- `distiller` -- converts raw episodes into compact knowledge entries
- `episode_completion` -- async distillation spawner (`spawn_episode_distillation`)
- `context` -- context-aware knowledge retrieval helpers
- `temporal` -- time-aware decay, demurrage, and refresh logic
- `tier_progression` -- automatic tier promotion based on validation history
- `EmotionalProvenance` -- PAD-based emotional reliability metadata
- `ValidationArc` -- narrative arc (Redemptive/Contaminating/Stable/Progressive)
- `WorldviewCluster` / `HdcCluster` -- HDC-based clustering for GC preservation
- `apply_demurrage` / `freeze_entry` / `thaw_entry` -- entry lifecycle management

## Half-life defaults

| Kind | Off-chain | On-chain (blocks) |
|------|-----------|-------------------|
| Insight | 30 days | 7 days |
| Heuristic | 90 days | 15 days |
| Warning | 1 hour | ~3 min |
| CausalLink | 60 days | 15 days |
| StrategyFragment | 14 days | 15 days |

## Usage

```rust
use roko_neuro::{KnowledgeEntry, KnowledgeKind, NeuroStore};
use roko_neuro::knowledge_store::KnowledgeStore;

let store = KnowledgeStore::init(".roko/neuro")?;
let results = store.query("how to handle flaky tests", 5)?;
```

## Architecture

Sits downstream of `roko-learn` (which produces episodes) and upstream of `roko-compose`
(which injects relevant knowledge into system prompts). The distiller bridges the two:
episodes come in, durable knowledge goes out. Tier progression automatically promotes
entries that accumulate validation evidence over time.
