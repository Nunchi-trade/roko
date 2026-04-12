# Sandboxing: Worktree Isolation and Process Containment

> **Layer**: L0 Runtime (process management), L1 Framework (path policy), L4 Orchestration (worktree management)
>
> **Crate**: `roko-agent` (safety/path.rs), `roko-orchestrator` (worktree), `bardo-runtime` (ProcessSupervisor)
>
> **Synapse traits**: `Substrate` (isolated storage per worktree), `Gate` (verify sandbox boundaries)
>
> **Prerequisites**: [00-defense-in-depth.md](00-defense-in-depth.md), [04-permits-allowlists.md](04-permits-allowlists.md)

---

## Overview

Sandboxing in Roko operates at three levels:

1. **Filesystem sandboxing**: Every agent operates within a git worktree. The `PathPolicy` in `roko-agent/src/safety/path.rs` ensures all file operations stay within the worktree boundary.
2. **Process sandboxing**: The `ProcessSupervisor` in `bardo-runtime` manages agent process lifecycles, enforcing timeouts, resource limits, and cooperative shutdown.
3. **Worktree isolation**: The `WorktreeManager` in `roko-orchestrator` creates isolated git worktrees for parallel task execution, ensuring agents cannot interfere with each other's work.

---

## Filesystem Sandboxing via PathPolicy

The `PathPolicy` is the single authority on whether a path argument is safe to hand to a filesystem tool handler. Every filesystem-touching built-in tool runs its path through `canonicalize_with_policy()` before any I/O.

### The Canonicalization Algorithm

1. **Build the joined path**: If the argument is absolute, use it as-is. If relative, join it to the worktree root.

2. **Canonicalize both paths**: Resolve symlinks and normalize `.` and `..` components. For non-existent leaves (e.g., `write_file` creating a new file), canonicalize the deepest existing ancestor and re-attach the missing tail. This avoids platform-specific behavior differences when canonicalizing non-existent paths.

3. **Escape check**: When `prevent_escapes` is true (default), the canonical joined path must `starts_with()` the canonical worktree root. If not, `ToolError::PathOutsideWorktree` is returned. This prevents:
   - `../../etc/passwd` — parent directory traversal
   - `/etc/passwd` — absolute path escape
   - Symlink-based escapes (when `deny_symlinks` is also set)

4. **Symlink check**: When `deny_symlinks` is true, walk the on-disk components and reject any symlink. This prevents an attacker from creating a symlink inside the worktree that points outside it.

5. **Compute relative form**: Strip the worktree prefix to produce a clean relative path with no leading `/` or `./` and no `..` components.

### CanonicalPath Return Type

```rust
pub struct CanonicalPath {
    /// Absolute, canonicalized path. Guaranteed inside worktree
    /// when prevent_escapes is true.
    pub absolute: PathBuf,
    /// Path relative to worktree root. No leading "/" or "./",
    /// no ".." components.
    pub relative: PathBuf,
}
```

### Non-Existent Path Handling

A critical detail: `write_file` needs to create files that don't exist yet. The standard `Path::canonicalize()` fails on non-existent paths. The implementation handles this by walking up ancestors until one canonicalizes, then re-attaching the missing tail:

```rust
fn canonicalize_existing_or_parent(path: &Path) -> PathBuf {
    if let Ok(p) = path.canonicalize() {
        return p;
    }
    let mut tail: Vec<&std::ffi::OsStr> = Vec::new();
    let mut cursor: &Path = path;
    loop {
        if let Ok(p) = cursor.canonicalize() {
            let mut out = p;
            for segment in tail.iter().rev() {
                out.push(segment);
            }
            return normalize(&out);
        }
        match (cursor.parent(), cursor.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name);
                cursor = parent;
            }
            _ => break,
        }
    }
    normalize(path)
}
```

### Post-Hoc Audit

The `is_within_worktree()` function provides a post-hoc check: "did this tool invocation produce an artifact inside the sandbox?" This is used for audit purposes — even if the path policy is misconfigured, a post-hoc check can detect escapes.

---

## Process Sandboxing via ProcessSupervisor

The `ProcessSupervisor` in `bardo-runtime` manages agent process lifecycles:

### Process Lifecycle

```rust
pub struct ProcessSupervisor {
    processes: Mutex<HashMap<ProcessId, ProcessHandle>>,
}

pub struct ProcessHandle {
    child: tokio::process::Child,
    id: ProcessId,
    started_at: Instant,
}
```

### Key Operations

- **Spawn with limits**: Create agent processes with configurable timeouts and resource limits
- **Cooperative shutdown**: Send SIGTERM, wait for grace period, then SIGKILL if unresponsive
- **Bulk kill/reap**: `shutdown_all()` terminates all supervised processes (used during plan abort)
- **Stdout/stderr capture**: Stream capture for audit logging and failure diagnosis

### Integration with Orchestrator

The `PlanRunner` in `orchestrate.rs` tracks active agents via the `ProcessSupervisor`:

1. When a task is dispatched, the agent process is spawned under supervision
2. The supervisor monitors the process for timeout or abnormal termination
3. On plan completion or abort, all supervised processes are shut down cooperatively
4. Process lifecycle events are recorded as Engrams for audit

---

## Worktree Isolation

The `WorktreeManager` in `roko-orchestrator` creates isolated git worktrees for parallel task execution:

### Why Worktrees?

When multiple agents work on tasks in parallel, they cannot share the same git working directory:
- File modifications by one agent would conflict with another's
- Build artifacts from one agent pollute another's environment
- Git operations (staging, committing) would race

Git worktrees solve this: each agent gets its own filesystem view of the repository, with its own working directory and index, sharing the same git objects database.

### Worktree Configuration

```rust
pub struct WorktreeConfig {
    /// Base directory for worktrees (default: `.roko/worktrees/`)
    pub base_dir: PathBuf,
    /// Idle TTL before a worktree is eligible for cleanup (default: 30 min)
    pub idle_ttl: Duration,
    /// Maximum number of concurrent worktrees (default: 8)
    pub max_worktrees: usize,
}
```

### Worktree Lifecycle

1. **Creation**: A new branch is created from the current HEAD, and a git worktree is checked out at `base_dir/<task-id>/`
2. **Assignment**: The worktree path is passed to the agent as its working directory. The `PathPolicy` sandboxes all file operations to this worktree.
3. **Execution**: The agent works within its isolated worktree. Changes are committed to the task branch.
4. **Merge**: On task success, the task branch is merged back into the main branch. The `PostMergeRunner` handles conflict resolution.
5. **Cleanup**: After merge (or after idle TTL expires for failed tasks), the worktree is removed.

### Safety Properties

- **Isolation**: Agents in different worktrees cannot see or modify each other's files
- **Rollback**: A failed task's worktree can be discarded without affecting the main branch
- **Audit**: Each worktree has its own git history, providing a clear record of what each agent did
- **Resource limits**: `max_worktrees` prevents disk exhaustion from too many concurrent agents

---

## Future: Container-Level Sandboxing

The current sandboxing is filesystem-level (PathPolicy) and process-level (ProcessSupervisor). Future iterations will add container-level sandboxing:

- **Namespace isolation**: Each agent in its own Linux namespace (PID, network, mount)
- **Seccomp filtering**: Restrict system calls available to agent processes
- **Cgroup limits**: CPU, memory, and I/O bandwidth limits per agent
- **Network isolation**: Each agent with its own network namespace, filtered by NetworkPolicy

This is a Tier 3 implementation target. The current filesystem and process sandboxing provides sufficient isolation for single-machine deployments.

---

## Academic References

| Paper | Contribution |
|-------|-------------|
| Saltzer & Schroeder (1975) | Principle of least privilege — minimum necessary access |
| Bershad et al. (1995) | SPIN — extensible operating system with safety guarantees |
| Engler et al. (2003) | Exokernel — application-level resource management |
| Watson et al. (2015) | CHERI — hardware-enforced memory capabilities |

---

## Related Topics

- [00-defense-in-depth.md](00-defense-in-depth.md) — PathPolicy as Guard 5
- [01-capability-tokens.md](01-capability-tokens.md) — Tool-level capability enforcement
- [04-permits-allowlists.md](04-permits-allowlists.md) — Permission model
- [14-cognitive-kernel-safety.md](14-cognitive-kernel-safety.md) — Cognitive Namespaces extend sandboxing to knowledge isolation
- [16-critical-integration-gap.md](16-critical-integration-gap.md) — PathPolicy is built but not invoked from CLI without ToolDispatcher
