# 03-graph -- Depth Index

Depth for [03-GRAPH.md](../../unified/03-GRAPH.md)

---

## Source docs (14)

### Orchestration overview

| Source doc | Status |
|---|---|
| `docs/01-orchestration/00-layer-overview.md` | Absorbed -> `plan-discovery-and-dag.md`, `parallel-executor.md`, `plan-phases-and-actions.md` |
| `docs/01-orchestration/01-plan-discovery.md` | Absorbed -> `plan-discovery-and-dag.md` |
| `docs/01-orchestration/04-plan-phases.md` | Absorbed -> `plan-phases-and-actions.md` |

### DAG and execution

| Source doc | Status |
|---|---|
| `docs/01-orchestration/02-unified-task-dag.md` | Absorbed -> `plan-discovery-and-dag.md` |
| `docs/01-orchestration/03-parallel-executor.md` | Absorbed -> `parallel-executor.md` |
| `docs/01-orchestration/05-executor-actions.md` | Absorbed -> `plan-phases-and-actions.md` |
| `docs/01-orchestration/06-runtime-harness.md` | Absorbed -> `plan-phases-and-actions.md`, `parallel-executor.md` |

### Isolation and merge

| Source doc | Status |
|---|---|
| `docs/01-orchestration/07-worktree-isolation.md` | Absorbed -> `worktree-isolation-and-merge.md` |
| `docs/01-orchestration/08-merge-queue.md` | Absorbed -> `worktree-isolation-and-merge.md` |

### Recovery and events

| Source doc | Status |
|---|---|
| `docs/01-orchestration/09-snapshot-recovery.md` | Absorbed -> `snapshot-and-recovery.md` |
| `docs/01-orchestration/10-event-log.md` | Absorbed -> `snapshot-and-recovery.md`, `event-log-and-conductor.md` |

### Advanced orchestration

| Source doc | Status |
|---|---|
| `docs/01-orchestration/11-conductor-integration.md` | Absorbed -> `event-log-and-conductor.md` |
| `docs/01-orchestration/12-stigmergy-niche.md` | Absorbed -> `stigmergy-and-cross-domain.md` |
| `docs/01-orchestration/13-cross-domain-orchestration.md` | Absorbed -> `stigmergy-and-cross-domain.md` |

---

## Depth docs

| Doc | Absorbs | What it covers |
|---|---|---|
| [plan-discovery-and-dag.md](plan-discovery-and-dag.md) | 00, 01, 02 | Plan discovery (filesystem scan, frontmatter parsing, ranking), DAG construction (cross-plan edges, file-conflict inference, topological sort, wave computation, critical path), optimization passes (fusion, culling, speculation, partitioning), incremental recomputation |
| [parallel-executor.md](parallel-executor.md) | 00, 03, 06 | Pure state machine architecture, tick loop, action/event vocabulary, per-plan state, resource-aware scheduling (multi-dimensional budget), concurrency management, priority inversion prevention, Petri net formal model |
| [plan-phases-and-actions.md](plan-phases-and-actions.md) | 00, 04, 05, 06 | Phase lifecycle (12 phases, 2 retry loops, 3 terminal states), transition table, action vocabulary (10 actions), runtime harness integration, agent role dispatch, gate pipeline, learning/conductor integration, Mori mapping |
| [worktree-isolation-and-merge.md](worktree-isolation-and-merge.md) | 07, 08 | Worktree Space pattern (isolation boundary, lifecycle, budget enforcement, health checks), merge queue Pipeline (conflict detection, priority ordering, retry with backoff, file-level granularity), MergeCoordinator wiring, warm agent pool, mori-diffs reality (merge queue not yet wired) |
| [snapshot-and-recovery.md](snapshot-and-recovery.md) | 09, 10 | Executor snapshots (atomic writes, auto-save, legacy compat), event log (hash chain, BLAKE3, tamper detection), dual-source recovery engine (snapshot + event log merge), validation checks, incremental delta snapshots (design), CRDT executor state (future), torn write protection |
| [event-log-and-conductor.md](event-log-and-conductor.md) | 10, 11 | Event log as Bus, 10 Conductor Lens Cells (silence, ghost turn, compile loop, review loop, cost overrun, context pressure, gate failure rate, deadlock, resource pressure, progress stall), diagnosis engine, graduated interventions, Yerkes-Dodson dynamics, circuit breaker, WatcherRunner background task, Viable System Model mapping |
| [stigmergy-and-cross-domain.md](stigmergy-and-cross-domain.md) | 12, 13 | Stigmergic coordination (git as pheromone medium, Signal pheromone field, knowledge as persistent pheromone), niche construction, c-factor measurement, cross-domain DAG (single-DAG principle, domain types, gate differentiation), task routing and model selection, plan repair strategies (patch/replan/hierarchical/adaptive), saga pattern for irreversible steps |
