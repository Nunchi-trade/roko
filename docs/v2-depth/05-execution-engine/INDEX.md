# 05-execution-engine — Depth Index

Depth for [05-EXECUTION-ENGINE.md](../../unified/05-EXECUTION-ENGINE.md)

---

## Source docs (3)

### Runtime loop

| Source doc | Status |
|---|---|
| `docs/00-architecture/09-universal-cognitive-loop.md` | Covered |

### Error handling and performance

| Source doc | Status |
|---|---|
| `docs/00-architecture/22-error-handling-recovery.md` | Covered |
| `docs/00-architecture/21-performance-numerical-stability.md` | Covered |

---

## Depth docs

| Doc | Covers | Source docs |
|---|---|---|
| [cognitive-loop-as-graph.md](cognitive-loop-as-graph.md) | 7-step loop as a concrete Hot Graph with typed Cells, Workflow/Activity split for resumability, composing nested loops, Byzantine Cell defenses | `09-universal-cognitive-loop.md` |
| [resilience-and-numerics.md](resilience-and-numerics.md) | Resilience algebra (4 error kinds with algebraic retry rules), circuit breakers as React-protocol state machines, graceful degradation ladder, f32/f64 precision decisions, hot-path budget table | `22-error-handling-recovery.md`, `21-performance-numerical-stability.md` |
