# 01-orchestration — L4 Orchestration Layer

> **Layer**: L4 Orchestration
> **Primary crate**: `roko-orchestrator` (`crates/roko-orchestrator/`)
> **Runtime harness**: `roko-cli/src/orchestrate.rs`
> **Status**: Wired end-to-end

---

## Summary

L4 Orchestration is the topmost layer of the Roko five-layer architecture. It
coordinates multiple agents working on multiple plans simultaneously through a
pure state machine (`ParallelExecutor`) that emits actions and consumes events,
connected to a runtime harness (`PlanRunner`) that dispatches those actions to
real subsystems — agent processes, compilation gates, git merges.

The orchestrator maintains plan lifecycle (Queued → Enriching → Implementing →
Gating → Verifying → Reviewing → DocRevision → Merging → Complete), serializes
merges via a file-conflict-aware queue, isolates plans in git worktrees, and
recovers from crashes using dual-source recovery (executor snapshots + hash-
chained event-log replay).

Multi-agent coordination follows a stigmergic model (Grassé 1959): agents
communicate indirectly through the shared codebase they modify, with git
commits serving as digital pheromones. The conductor monitors system health
via 10 watchers and applies Yerkes-Dodson pressure dynamics to maintain
optimal performance.

---

## Sub-documents

| # | File | Topic | Lines |
|---|------|-------|-------|
| 00 | [00-layer-overview.md](./00-layer-overview.md) | L4 layer position, five-layer architecture, key concepts, sub-doc map | ~220 |
| 01 | [01-plan-discovery.md](./01-plan-discovery.md) | Plan scanning, frontmatter parsing, validation, ranking | ~210 |
| 02 | [02-unified-task-dag.md](./02-unified-task-dag.md) | Cross-plan DAG, file-conflict inference, topological sort, wave scheduling, critical path | ~230 |
| 03 | [03-parallel-executor.md](./03-parallel-executor.md) | Pure state machine, tick/event loop, concurrency management, design rationale | ~200 |
| 04 | [04-plan-phases.md](./04-plan-phases.md) | Phase lifecycle, state transition diagram, transition rules, retry bounds, failure types | ~250 |
| 05 | [05-executor-actions.md](./05-executor-actions.md) | Action vocabulary, dispatch semantics, serialization, action flow | ~220 |
| 06 | [06-runtime-harness.md](./06-runtime-harness.md) | PlanRunner structure, dispatch loop, agent dispatch, task tracking, learning integration | ~260 |
| 07 | [07-worktree-isolation.md](./07-worktree-isolation.md) | Per-plan worktrees, branch naming, health checks, idle reclamation, budget enforcement | ~230 |
| 08 | [08-merge-queue.md](./08-merge-queue.md) | File-conflict-aware merge serialization, priority ordering, retry with backoff | ~220 |
| 09 | [09-snapshot-recovery.md](./09-snapshot-recovery.md) | Executor snapshots, event-log replay, merged recovery, validation warnings | ~250 |
| 10 | [10-event-log.md](./10-event-log.md) | Hash-chained event sourcing, BLAKE3 integrity, tamper detection, forensic replay | ~240 |
| 11 | [11-conductor-integration.md](./11-conductor-integration.md) | 10 watchers, Yerkes-Dodson dynamics, cost monitoring, diagnosis engine | ~220 |
| 12 | [12-stigmergy-niche.md](./12-stigmergy-niche.md) | Stigmergic coordination, niche construction, C-Factor, pheromone typology | ~260 |
| 13 | [13-cross-domain-orchestration.md](./13-cross-domain-orchestration.md) | Multi-domain DAGs, domain-specific gates, HEFT scheduling, Spore/Sparrow | ~250 |

---

## Key types

| Type | Crate | Purpose |
|------|-------|---------|
| `ParallelExecutor` | `roko-orchestrator` | Pure state machine driving plan lifecycle |
| `PlanStateMachine` | `roko-orchestrator` | Phase transition logic |
| `PlanState` | `roko-orchestrator` | Per-plan mutable state |
| `ExecutorAction` | `roko-orchestrator` | Side-effect requests (10 variants) |
| `ExecutorEvent` | `roko-orchestrator` | State transition triggers (13 variants) |
| `UnifiedTaskDag` | `roko-orchestrator` | Cross-plan task graph with file-conflict edges |
| `MergeQueue` | `roko-orchestrator` | File-conflict-aware merge serialization |
| `WorktreeManager` | `roko-orchestrator` | Per-plan git worktree lifecycle |
| `EventLog` | `roko-orchestrator` | Hash-chained append-only event log |
| `ExecutorSnapshot` | `roko-orchestrator` | Serializable crash-recovery state |
| `RecoveryEngine` | `roko-orchestrator` | Dual-source crash recovery |
| `PlanRunner` | `roko-cli` | Effectful runtime harness |
| `TaskTracker` | `roko-cli` | Per-plan task progress tracking |
| `Conductor` | `roko-conductor` | Meta-cognitive anomaly detection |

---

## Key files

| Path | Description |
|------|-------------|
| `crates/roko-orchestrator/src/dag.rs` | UnifiedTaskDag implementation (760 lines) |
| `crates/roko-orchestrator/src/executor/mod.rs` | ParallelExecutor (719 lines) |
| `crates/roko-orchestrator/src/executor/action.rs` | ExecutorAction enum (203 lines) |
| `crates/roko-orchestrator/src/executor/plan_state.rs` | PlanState struct (271 lines) |
| `crates/roko-orchestrator/src/executor/state_machine.rs` | PlanStateMachine (633 lines) |
| `crates/roko-orchestrator/src/executor/snapshot.rs` | ExecutorSnapshot (300 lines) |
| `crates/roko-orchestrator/src/executor/recovery.rs` | RecoveryEngine (1075 lines) |
| `crates/roko-orchestrator/src/merge_queue.rs` | MergeQueue (627 lines) |
| `crates/roko-orchestrator/src/worktree.rs` | WorktreeManager (999 lines) |
| `crates/roko-orchestrator/src/event_log.rs` | EventLog (526 lines) |
| `crates/roko-orchestrator/src/plan_discovery.rs` | discover_plans() (594 lines) |
| `crates/roko-cli/src/orchestrate.rs` | PlanRunner runtime harness |

---

## Architecture diagram

```
                    ┌─────────────────────────────────────────────┐
                    │              roko plan run                   │
                    │                                             │
                    │  ┌───────────────────────────────────────┐  │
                    │  │            PlanRunner                  │  │
                    │  │  ┌─────────────┐  ┌───────────────┐   │  │
                    │  │  │ Parallel    │  │ WorktreeManager│   │  │
                    │  │  │ Executor    │  └───────────────┘   │  │
                    │  │  │ (pure SM)   │  ┌───────────────┐   │  │
                    │  │  │             │  │  MergeQueue    │   │  │
                    │  │  │ tick()      │  └───────────────┘   │  │
                    │  │  │   ↓         │  ┌───────────────┐   │  │
                    │  │  │ actions     │  │  EventLog      │   │  │
                    │  │  │   ↓         │  │ (hash-chain)   │   │  │
                    │  │  │ dispatch    │  └───────────────┘   │  │
                    │  │  │   ↓         │  ┌───────────────┐   │  │
                    │  │  │ events      │  │  Conductor     │   │  │
                    │  │  │   ↓         │  │ (10 watchers)  │   │  │
                    │  │  │ apply_event │  └───────────────┘   │  │
                    │  │  └─────────────┘                       │  │
                    │  │                                        │  │
                    │  │  ┌─────────┐ ┌────────┐ ┌──────────┐  │  │
                    │  │  │Learning │ │Daimon  │ │Skills    │  │  │
                    │  │  │Runtime  │ │State   │ │Library   │  │  │
                    │  │  └─────────┘ └────────┘ └──────────┘  │  │
                    │  └───────────────────────────────────────┘  │
                    │                    │                         │
                    │         ┌──────────┴──────────┐              │
                    │         ▼                     ▼              │
                    │  ┌──────────────┐    ┌──────────────────┐   │
                    │  │ ClaudeCliAgent│    │   Gate Pipeline  │   │
                    │  │ (in worktree) │    │ compile→test→    │   │
                    │  └──────────────┘    │ clippy→verify    │   │
                    │                      └──────────────────┘   │
                    └─────────────────────────────────────────────┘
```

---

## CLI commands

| Command | Orchestration role |
|---------|-------------------|
| `roko plan list` | Discovers and lists plans |
| `roko plan show <id>` | Shows plan details |
| `roko plan create` | Creates a new plan |
| `roko plan run <dir>` | Full orchestration loop |
| `roko plan run <dir> --resume <snapshot>` | Resume from crash |
| `roko dashboard` | Monitor orchestration progress |
| `roko status` | Query signals and episodes |

---

## Citations referenced

| Author(s) | Year | Work | Used in |
|-----------|------|------|---------|
| Grassé, P.-P. | 1959 | La reconstruction du nid (*Insectes Sociaux*) | 00, 02, 07, 12 |
| Parunak, H. V. D. | 2002 | Digital pheromones (*AAMAS*) | 00, 07, 12 |
| Dorigo, M. & Gambardella, L. M. | 1997 | Ant colony system (*IEEE Trans. EC*) | 00, 12 |
| Woolley, A. W. et al. | 2010 | Collective intelligence factor (*Science* 330) | 00, 06, 12 |
| Yerkes, R. M. & Dodson, J. D. | 1908 | Stimulus-habit formation (*JCNP*) | 00, 06, 11 |
| Odling-Smee, F. J. et al. | 2003 | *Niche Construction* (Princeton) | 00, 12 |
| Tomasello, M. | 2014 | *A Natural History of Human Thinking* (Harvard) | 00, 12 |
| Topcuoglu, H. et al. | 2002 | HEFT scheduling (*IEEE TPDS*) | 02, 13 |
| Damasio, A. | 1994 | *Descartes' Error* (Putnam) | 06 |
| Sumers, T. R. et al. | 2023 | CoALA cognitive architectures (*arXiv:2309.02427*) | 03 |
| Charnov, E. L. | 1976 | Marginal value theorem (*TPB*) | 12 |
| Derrida, J. | 1993 | *Specters of Marx* (Routledge) | 12 |
| Beer, S. | 1972 | *Brain of the Firm* (Allen Lane) | 11 |
| Conant, R. C. & Ashby, W. R. | 1970 | Good regulator theorem (*IJSS*) | 11 |
| Nygard, M. T. | 2007 | *Release It!* (Pragmatic Bookshelf) | 04, 11 |
| Fowler, M. | 2005 | Event Sourcing | 03, 09, 10 |
| Mohan, C. et al. | 1992 | ARIES recovery (*ACM TODS*) | 09 |
| Gray, J. & Reuter, A. | 1992 | *Transaction Processing* (Morgan Kaufmann) | 08 |
| Vickrey, W. | 1961 | Sealed-bid auctions (*J. Finance*) | 13 |
| Ousterhout, J. | 2013 | Sparrow scheduling (*SOSP*) | 13 |
| Hu, S. et al. | 2025 | ADAS (*ICLR*) | 13 |
| Lee, J. et al. | 2026 | FrugalGPT (*arXiv:2603.28052*) | 13 |
| Wooldridge, M. | 2009 | *Introduction to MultiAgent Systems* (Wiley) | 05 |
| Kahn, A. B. | 1962 | Topological sorting | 02 |
| Nakamoto, S. | 2008 | Bitcoin whitepaper | 10 |
| van der Aalst, W. M. P. | 1998 | Petri nets for workflow (*JCSC*) | 04 |

---

## Naming conventions applied

| Old name | New name | Notes |
|----------|----------|-------|
| Mori | Roko Orchestrator | The orchestration subsystem |
| Golem | Agent | Domain-agnostic agent |
| Grimoire | Neuro | Knowledge store |
| Styx | Agent Mesh | P2P communication |
| Clade | Collective / Mesh | Agent group |
| Signal | Engram (in architecture docs) | Content-addressed cognition unit |
| GNOS | KORAI / DAEJI | Token names |
| Fleet | Collective | Agent group (corrected from earlier error) |

> Note: In the active codebase, `Signal` is still the Rust type name. `Engram`
> is the architectural concept name used in design documents. The rename will
> occur in a future crate-wide refactoring pass.
