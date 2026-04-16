# roko-orchestrator

Plan discovery and unified task DAG for Roko. The scheduling layer — what runs next, in what order, across which plans.

## Install

```toml
[dependencies]
roko-orchestrator = { path = "../roko-orchestrator" }
roko-core = { path = "../roko-core" }
```

## What's shipped

- **`plan_discovery`** — scan a plans directory, parse YAML frontmatter, return ranked `PlanInfo` entries.
- **`dag`** — `UnifiedTaskDag` builds a cross-plan task DAG from intra-plan `depends_on`, cross-plan `depends_on`, plan-level `depends_on`, and optional file-overlap inference. It now also exposes CPM analysis, zero-slack critical-path extraction, linear-chain fusion, an incremental DAG wrapper, and live mutation support.
- **`executor`** — `ParallelExecutor` still drives plan phases, and now tracks speculative execution branches plus serializable DAG mutation requests for the runtime boundary.

## What's *not* yet built

- **Worktree manager** — no isolation between parallel tasks.
- **Merge queue** — no commit-and-integrate loop.

This crate exposes the data model downstream orchestration needs; the runtime that consumes it is future work.

## Plan layout

Two directory conventions, discovered automatically:

```
plans/01-auth-rewrite/plan.md         # new layout (preferred)
plans/02-cache-layer.md               # legacy layout
```

Frontmatter is optional YAML between `---` fences at the top of each plan:

```yaml
---
plan: 01-auth-rewrite
depends_on: []
parallel_with: [02-cache-layer]
priority: high
estimated_minutes: 240
---
```

## Example

```rust
use roko_orchestrator::{discover_plans, DagConfig, UnifiedTaskDag};

let plans = discover_plans("./plans")?;                   // ranked PlanInfo list
let tasks = collect_tasks_from_plans(&plans);             // user-supplied
let dag = UnifiedTaskDag::build(&tasks, &plan_deps, DagConfig::default())?;
for wave in dag.waves() {
    println!("wave {}: {:?} ({} min)", wave.index, wave.tasks, wave.estimated_minutes);
}
```

## Wave semantics

`UnifiedTaskDag::waves()` layers the DAG via BFS:
- **Wave 0** — tasks with no open dependencies.
- **Wave N** — tasks whose deps all live in waves `< N`.
- Within a wave, tasks sort by `GlobalTaskId` for deterministic output.
- `estimated_minutes` on each wave is the max over its tasks (wall-clock floor assuming full parallelism).

## File-overlap inference

Opt-in via `DagConfig::infer_file_overlap`: if two tasks both touch `src/lib.rs`, the DAG adds an edge serializing them (earlier `GlobalTaskId` runs first). Prevents worktree-level merge conflicts without requiring explicit `depends_on` edges.

## Downstream

`roko-cli` does *not* yet consume this crate — it runs one prompt at a time. A future executor + worktree manager will read `ExecutionWave`s from `UnifiedTaskDag` and dispatch tasks to agents, which is the jump from "CLI tool" to "orchestrator."
