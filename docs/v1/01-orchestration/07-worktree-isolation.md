# Worktree Isolation

> **Module**: `roko-orchestrator/src/worktree.rs`
> **Key type**: `WorktreeManager`
> **Tests**: 20+ tests covering creation, removal, health checks, idle
> reclamation, budget enforcement


> **Implementation**: Shipping

---

## Overview

Git worktrees provide per-plan isolation for the Roko Orchestrator. Each active
plan gets its own worktree — a separate working directory backed by a branch
in the same repository. This allows multiple agents to work on different plans
simultaneously without conflicting on the filesystem.

The `WorktreeManager` handles the full worktree lifecycle: creation, branch
naming, health monitoring, idle reclamation, stale lock cleanup, and budget
enforcement. It ensures the number of live worktrees stays within configured
limits.

---

## Why Worktrees?

### The problem

When multiple agents modify a shared codebase simultaneously, they conflict:

1. **File conflicts**: Two agents editing the same file overwrite each other's
   changes
2. **Build conflicts**: Agent A's half-finished edit causes Agent B's
   compilation to fail
3. **Test contamination**: Test results reflect a mix of changes from different
   plans
4. **Merge hell**: Combining simultaneous changes requires complex merge
   resolution

### The solution

Git worktrees solve this at the filesystem level. Each worktree:

- Has its own working directory (separate files)
- Has its own branch (separate commit history)
- Shares the same `.git` repository (efficient — no full clone)
- Can run `cargo build`, `cargo test` independently

Agents working in different worktrees cannot conflict on files. They operate
on isolated branches and only interact at merge time, where conflicts are
handled explicitly by the `MergeQueue`.

---

## WorktreeConfig

```rust
pub struct WorktreeConfig {
    /// Path to the main repository root.
    pub repo_root: PathBuf,
    /// Base branch to create worktree branches from (e.g., "main").
    pub base_branch: String,
    /// Directory where worktrees are created.
    pub worktrees_root: PathBuf,
    /// Maximum number of live worktrees allowed.
    pub max_live: usize,
    /// Idle time (in seconds) after which a worktree can be reclaimed.
    pub idle_ttl: Duration,
}
```

### Configuration defaults

| Parameter | Default | Source |
|-----------|---------|--------|
| `repo_root` | Working directory | From CLI `--workdir` |
| `base_branch` | `"main"` | From config |
| `worktrees_root` | `.roko/worktrees/` | Convention |
| `max_live` | 8 | From `config.conductor.max_agents` |
| `idle_ttl` | 30 minutes | `DEFAULT_WORKTREE_IDLE_TTL_SECS` |

---

## WorktreeHandle

Each active worktree is tracked by a `WorktreeHandle`:

```rust
pub struct WorktreeHandle {
    /// Unique identifier for this worktree.
    pub id: String,
    /// Filesystem path to the worktree directory.
    pub path: PathBuf,
    /// Git branch name for this worktree.
    pub branch: String,
    /// Unix millisecond timestamp when the worktree was created.
    pub created_at_ms: u64,
    /// Unix millisecond timestamp of last activity.
    pub last_active_ms: u64,
}
```

The `last_active_ms` field is updated whenever an agent operates in the
worktree. It is used by the idle reclamation system to identify and remove
stale worktrees.

---

## Branch Naming Convention

Worktree branches follow the pattern:

```
roko/plan/<plan_id>
```

For example:

```
roko/plan/01-workspace-scaffold
roko/plan/02-core-traits
roko/plan/08a-chain-layer
```

This convention:

1. **Namespaces** branches under `roko/plan/` to avoid conflicts with
   user-created branches
2. **Includes the plan ID** for traceability — you can see which plan produced
   which branch
3. **Is deterministic** — the same plan always gets the same branch name,
   enabling `ensure_for_plan()` to reuse existing worktrees

---

## Lifecycle Operations

### create()

Creates a new worktree with a fresh branch:

```rust
pub async fn create(&self, id: &str) -> Result<WorktreeHandle, WorktreeError>
```

1. Checks if `max_live` would be exceeded; if so, returns `BudgetExceeded`
2. Creates the branch from `base_branch`:
   `git branch roko/plan/<id> <base_branch>`
3. Creates the worktree:
   `git worktree add <worktrees_root>/<id> roko/plan/<id>`
4. Records the `WorktreeHandle` in the internal HashMap

### create_for_plan()

Convenience method that uses the plan ID as the worktree ID:

```rust
pub async fn create_for_plan(&self, plan_id: &str) -> Result<WorktreeHandle, WorktreeError>
```

### ensure_for_plan()

Creates a worktree for a plan if one doesn't already exist, or returns the
existing handle:

```rust
pub async fn ensure_for_plan(&self, plan_id: &str) -> Result<WorktreeHandle, WorktreeError>
```

This is the preferred method for the runtime — it's idempotent and handles
resume scenarios where a worktree may already exist from a previous run.

### remove()

Removes a worktree and optionally deletes its branch:

```rust
pub async fn remove(&self, id: &str, delete_branch: bool) -> Result<(), WorktreeError>
```

1. Runs `git worktree remove <path>` (with `--force` if needed)
2. Optionally runs `git branch -D roko/plan/<id>`
3. Removes the handle from the internal HashMap

### check_health()

Checks the health of a worktree:

```rust
pub fn check_health(&self, id: &str) -> WorktreeHealth
```

Returns one of:

| Health | Meaning |
|--------|---------|
| `Ok` | Worktree exists and is functional |
| `Missing` | Worktree directory doesn't exist (deleted externally?) |
| `StaleLock` | A `*.lock` file exists (leftover from crashed git operation) |
| `Detached` | HEAD is detached (not on the expected branch) |

### reclaim_idle()

Removes worktrees that have been idle longer than `idle_ttl`:

```rust
pub async fn reclaim_idle(&self) -> Vec<String>
```

Iterates over all tracked worktrees, checks `last_active_ms`, and removes
any that exceed the TTL. Returns the IDs of reclaimed worktrees.

This prevents worktree accumulation when plans complete or stall. The default
30-minute TTL gives agents time to finish before reclamation.

### clear_stale_locks()

Removes leftover `*.lock` files from worktrees:

```rust
pub async fn clear_stale_locks(&self) -> Vec<String>
```

Lock files are created by git during operations like `git merge` and
`git rebase`. If an operation is interrupted (crash, kill), the lock file
persists and blocks future git operations. This method detects and removes
stale locks.

### prune()

Runs `git worktree prune` to clean up stale worktree metadata:

```rust
pub async fn prune(&self) -> Result<(), WorktreeError>
```

This removes worktree entries from `.git/worktrees/` that point to
non-existent directories.

---

## Budget Enforcement

The `max_live` parameter enforces a hard limit on concurrent worktrees. When
a `create()` call would exceed this limit:

1. The manager first tries `reclaim_idle()` to free idle worktrees
2. If still over budget, it returns `WorktreeError::BudgetExceeded`

This prevents disk space exhaustion and ensures the system operates within
configured resource bounds.

---

## Thread Safety

`WorktreeManager` uses `Arc<WorktreeConfig>` for shared configuration and
`Arc<Mutex<HashMap<String, WorktreeHandle>>>` for mutable state. The mutex
is from `parking_lot` for non-poisoning behavior and better performance.

Multiple async tasks can safely call `create()`, `remove()`, and
`check_health()` concurrently. The git operations themselves are serialized
by the filesystem — git uses lock files to prevent concurrent modifications.

---

## Integration with the Orchestrator

The `PlanRunner` creates a `WorktreeManager` during initialization:

```rust
let worktrees = WorktreeManager::new(WorktreeConfig {
    repo_root: workdir.clone(),
    base_branch: "main".to_string(),
    worktrees_root: workdir.join(".roko").join("worktrees"),
    max_live: config.conductor.max_agents,
    idle_ttl: Duration::from_secs(DEFAULT_WORKTREE_IDLE_TTL_SECS),
});
```

When a plan is dispatched (`DispatchPlan`):
1. `worktrees.ensure_for_plan(plan_id)` creates or reuses a worktree
2. The worktree path becomes the `exec_dir` for all agent processes in that plan
3. Gates run in the worktree directory
4. On merge, the worktree branch is merged into the batch branch

When a plan completes or fails, the worktree can be cleaned up. However,
per user preference, worktrees and branches are preserved for inspection
and history rather than automatically deleted.

---

## Relationship to Stigmergic Coordination

Worktrees are the physical manifestation of the stigmergic coordination model.
Each agent operates in its own environment (worktree), leaving traces (commits)
that other agents can observe through the shared repository. The merge queue
serializes the integration of these traces into the shared codebase.

This is analogous to how termites coordinate construction through pheromone
deposition on physical structures (Grassé 1959). In Roko, the "pheromones"
are git commits and the "structure" is the codebase. Agents don't communicate
directly — they communicate through the artifacts they produce.

---

## Error Types

```rust
pub enum WorktreeError {
    /// A git command failed.
    GitError(String),
    /// The worktree ID is already in use.
    AlreadyExists(String),
    /// The worktree ID was not found.
    NotFound(String),
    /// The max_live budget would be exceeded.
    BudgetExceeded { max_live: usize, current: usize },
}
```

---

## References

- Git worktrees: `git-worktree(1)` — official git documentation. Worktrees
  were introduced in git 2.5 (2015) specifically to enable parallel work
  within a single repository.
- Grassé, P.-P. (1959). La reconstruction du nid et les coordinations
  interindividuelles chez Bellicositermes natalensis et Cubitermes sp.
  *Insectes Sociaux*, 6(1), 41–80.
- Parunak, H. V. D. (2002). Digital pheromones for coordination of unmanned
  vehicles. *AAMAS 2002*. (Digital stigmergy in multi-agent systems)
