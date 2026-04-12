# Plan Discovery

> **Module**: `roko-orchestrator/src/plan_discovery.rs`
> **Entry point**: `discover_plans(plans_dir: &Path) -> Result<Vec<PlanInfo>, DiscoveryError>`
> **CLI command**: `roko plan list` (lists discovered plans), `roko plan run <dir>` (discovers then executes)


> **Implementation**: Shipping

---

## Overview

Plan discovery is the first step of the orchestration pipeline. Before any agent
can be spawned, before any DAG can be constructed, the orchestrator must answer:
**what plans exist, what do they contain, and in what order should they run?**

The `discover_plans` function scans a directory for plan files, parses their
YAML frontmatter, validates the results, and returns a ranked list of `PlanInfo`
entries ready for the executor.

---

## Directory Layout

Two directory layouts are supported. The new layout takes precedence when both
exist for the same plan:

### New layout (preferred)

```
plans/
  01-workspace-scaffold/
    plan.md          ← plan description with YAML frontmatter
    tasks.toml       ← task definitions (parsed separately by TasksFile)
    CONTEXT.md       ← optional context document (skipped by discovery)
  02-core-traits/
    plan.md
    tasks.toml
```

Each plan lives in a numbered directory (`<num>-<slug>/`). The plan description
is always `plan.md`. The numeric prefix (`<num>`) may include alpha suffixes
(e.g., `08a-variant`), which sort after pure numerics (`08` < `08a` < `09`).

### Legacy layout (fallback)

```
plans/
  01-workspace-scaffold.md
  02-core-traits.md
```

Flat `.md` files at the top level. The base name (minus `.md`) becomes the plan
identifier. Legacy plans are discovered only if no new-layout directory exists
with the same base name.

### Conflict resolution

When both `plans/03-foo/plan.md` and `plans/03-foo.md` exist, the directory
layout wins. The legacy flat file is silently skipped. This ensures smooth
migration from flat files to structured plan directories.

---

## Frontmatter Contract

Frontmatter lives between two `---` fences at the top of `plan.md`. All fields
are optional — a plan without frontmatter discovers successfully with
`frontmatter = None`.

### Schema

```yaml
---
plan: "01-workspace-scaffold"          # Plan identifier
depends_on: ["00-init"]                # Plans that must complete first
parallel_with: ["02-core"]             # Plans safe to run in parallel with
crates_touched: ["roko-core", "roko-fs"]  # Crate directories modified
estimated_tasks: 8                     # Expected number of agent tasks
estimated_parallel_width: 4            # Max concurrent agents
estimated_minutes: 45                  # Expected wall-clock minutes
refactor_after: false                  # Run refactor pass on completion?
parallel_safe: true                    # Safe for parallel execution? (default: true)
priority: 10                           # Ranking priority (higher runs first)
tags: ["rust", "orchestrator"]         # Free-form tags
milestone: "v0.2"                      # Milestone label
---
```

### Field semantics

| Field | Type | Default | Purpose |
|-------|------|---------|---------|
| `plan` | `Option<String>` | `None` | Stable identifier. Used for cross-plan dependency references. |
| `depends_on` | `Vec<String>` | `[]` | Plans that must reach `Complete` before this plan starts. |
| `parallel_with` | `Vec<String>` | `[]` | Plans explicitly marked as safe for concurrent execution. |
| `crates_touched` | `Vec<String>` | `[]` | Crate directories this plan modifies. Used for file-conflict inference in the `UnifiedTaskDag`. |
| `estimated_tasks` | `Option<usize>` | `None` | Advisory: how many tasks to expect. |
| `estimated_parallel_width` | `Option<usize>` | `None` | Advisory: maximum concurrent agents for this plan. Must be > 0 if set. |
| `estimated_minutes` | `Option<u32>` | `None` | Advisory: expected duration. Must be > 0 if set. |
| `refactor_after` | `bool` | `false` | Whether to trigger a refactor pass after plan completion. |
| `parallel_safe` | `bool` | `true` | Whether this plan's tasks can run in parallel with other plans' tasks. |
| `priority` | `Option<u32>` | `None` (treated as 0) | Ranking priority. Higher values run first. Ties broken by `num`. |
| `tags` | `Vec<String>` | `[]` | Free-form metadata for filtering and reporting. |
| `milestone` | `Option<String>` | `None` | Associates the plan with a project milestone. |

### Parsing details

The frontmatter parser is BOM-tolerant (strips `U+FEFF` prefix) and handles
both LF and CRLF line endings. Parsing is done with `serde_yaml_ng`. If the
YAML is malformed, the discovery fails loudly with `DiscoveryError::BadFrontmatter`
rather than silently dropping the plan — this is a deliberate design choice to
catch errors early.

If a plan file starts with `---` but has no closing `---` fence, it is treated
as having no frontmatter (not an error).

---

## Validation

After parsing, frontmatter is validated by `validate_frontmatter()`:

1. **Plan ID must not be empty**: If `plan` is `Some("")` or `Some("   ")`, the
   plan is rejected with `ValidationError::MissingPlanId`. If `plan` is `None`,
   it passes — only an explicitly empty ID is an error.

2. **Estimated minutes must be > 0**: If set, `estimated_minutes: 0` is rejected
   with `ValidationError::InvalidMinutes`.

3. **Estimated parallel width must be > 0**: If set,
   `estimated_parallel_width: 0` is rejected with
   `ValidationError::InvalidParallelWidth`.

Validation is intentionally lax — only load-bearing invariants trigger errors.
Missing optional fields are fine. This allows plans to be written incrementally:
start with just the prose, add frontmatter later as the plan matures.

---

## Plan Ranking

After discovery, plans are sorted by `rank_plans()`:

1. **Primary sort**: Priority (descending). Plans with higher `priority` values
   run first.
2. **Secondary sort**: `num` prefix (ascending, lexicographic). Among plans with
   the same priority (or no priority), lower-numbered plans run first.

This means:

```
priority: 10, num: "12" → runs first
priority: 10, num: "13" → runs second (same priority, lower num wins)
priority:  1, num: "11" → runs third (lower priority)
priority:  0, num: "01" → runs fourth (default priority)
```

The ranking determines the initial execution queue order in the `ParallelExecutor`.
The queue can be dynamically reordered during execution via `Reorder` actions.

---

## PlanInfo Structure

```rust
pub struct PlanInfo {
    /// Full base name, e.g. "01-workspace-scaffold" or "08a-whatever".
    pub base: String,
    /// Numeric/alphanumeric prefix, e.g. "01" or "08a".
    pub num: String,
    /// Full path to the plan .md file.
    pub path: PathBuf,
    /// Parsed frontmatter. None when the file has no `---` fences.
    pub frontmatter: Option<PlanFrontmatter>,
}
```

The `base` field serves as the plan's stable identifier throughout the system.
It appears in:

- `PlanState.plan_id`
- Worktree branch names (`roko/plan/<base>`)
- Executor snapshots
- Event log payloads
- Episode logger records
- Cost tracking tables

---

## Error Handling

Discovery errors are typed and actionable:

| Error | Cause | Action |
|-------|-------|--------|
| `DirMissing(path)` | Plans directory doesn't exist | Create the directory or fix the path |
| `ReadFailed { path, source }` | I/O error reading a plan file | Check file permissions, disk space |
| `BadFrontmatter { path, reason }` | YAML parse error | Fix the YAML syntax |
| `Invalid { path, source }` | Validation failure | Fix the field value (empty plan ID, zero minutes, etc.) |

All errors include the offending file path, making them easy to locate and fix.

---

## Integration with the Orchestrator

After discovery, the ranked `Vec<PlanInfo>` flows into the orchestration
pipeline:

```
discover_plans()
    → Vec<PlanInfo>
    → PlanRunner::new() adds each plan to the ParallelExecutor
    → executor.add_plan(plan_id, PlanState::new(plan_id))
    → TaskTracker::new(TasksFile::parse(tasks_path), plan_dir)
```

The plan's `depends_on` frontmatter is used by the `UnifiedTaskDag` to create
cross-plan dependency edges. The `crates_touched` field enables file-conflict
inference between plans that modify the same crate directories.

The `parallel_safe` flag determines whether the plan's tasks can be scheduled
concurrently with tasks from other plans. Plans with `parallel_safe: false`
are serialized — they run alone.

---

## Test Coverage

The plan discovery module has comprehensive tests covering:

- Missing directory detection
- Empty directory returns empty vector
- New-layout plan discovery
- Legacy flat-file discovery
- New layout wins over legacy on conflict
- Plans without frontmatter discover with `None`
- Malformed YAML fails loudly
- Alpha-suffix prefix preservation (`08a`)
- Alpha-suffix sorting after numeric (`08` < `08a` < `09`)
- BOM prefix stripping
- Priority-based ordering with tie-breaking
- Directories without `plan.md` are skipped
- `CONTEXT.md` files are skipped
- Array fields parse correctly
- CRLF line endings are handled
- `parallel_safe` defaults to `true`
- Validation rejects zero minutes, zero width, empty plan ID
- Multiple plans sort deterministically

---

## References

- The plan discovery mechanism draws on the document hierarchy concept from the
  original Mori orchestrator's PRD-to-execution pipeline (Roko Orchestrator
  reference, `bardo-backup/prd/25-mori/mori-document-pipeline.md`), which
  defined a PRD → Plan → Task → Brief → Prompt hierarchy. In Roko, this
  hierarchy is simplified to Plan → Task, with the plan's `plan.md` serving as
  both description and configuration via frontmatter.

- The YAML frontmatter convention follows Hugo-style front matter widely used in
  static site generators and documentation systems. The choice of YAML over
  TOML for frontmatter (despite `tasks.toml` using TOML) reflects the
  prevalence of YAML frontmatter in the Markdown ecosystem.
