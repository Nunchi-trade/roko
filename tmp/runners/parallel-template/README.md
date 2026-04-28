# Parallel Runner Template

Dependency-aware parallel batch runner. Uses codex to execute batches on separate git worktrees, merges results back, and validates with gates.

## How it works

1. You define **batches** in `batches.toml` — each batch is a unit of work with an ID, scope files, dependencies, and a prompt
2. The DAG scheduler finds batches whose dependencies are satisfied and dispatches them in parallel
3. Each batch gets its own git worktree forked from a main run branch
4. Codex runs the batch prompt in that worktree
5. Anti-pattern checks run on the output
6. The batch worktree is merged back to the main branch (serialized via flock)
7. When all batches in a group complete, a wave gate runs (cargo check + clippy)
8. After everything, a test gate runs (cargo test)

## Directory structure of a runner

```
my-runner/
├── run.sh              # Thin wrapper that sets env and calls parallel-template
├── batches.toml        # Batch definitions (required)
├── prompts/            # One .prompt.md per batch ID (required)
│   ├── X01.prompt.md
│   └── X02.prompt.md
├── context-pack/       # Shared context injected into every prompt (optional but recommended)
│   ├── 00-RULES.md
│   ├── 01-ARCHITECTURE.md
│   └── 02-ANTI-PATTERNS.md
└── logs/               # Created at runtime — run artifacts, status, events
```

## Creating a new runner

### 1. Create the directory

```bash
mkdir -p tmp/runners/my-runner/{prompts,context-pack}
```

### 2. Create `run.sh`

This is a thin wrapper. The only thing it does is set `RUNNER_NAME` (must be unique across all runners) and point to the template.

```bash
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export RUNNER_NAME="my-runner"
export RUNNER_ROOT="$SCRIPT_DIR"
export LOG_ROOT="$SCRIPT_DIR/logs"
export PROMPTS_DIR="$SCRIPT_DIR/prompts"
export CONTEXT_DIR="$SCRIPT_DIR/context-pack"
exec bash "$SCRIPT_DIR/../parallel-template/run-parallel.sh" "$@"
```

`chmod +x tmp/runners/my-runner/run.sh`

### 3. Create `batches.toml`

Each batch is a `[[batch]]` entry:

```toml
[[batch]]
id = "X01"
title = "Short description of what this batch does"
group = "X"
deps = []
scope = ["crates/some-crate/src/file.rs"]
also_read = ["crates/some-crate/src/related.rs"]
verify = "quick"

[[batch]]
id = "X02"
title = "Second thing that depends on the first"
group = "X"
deps = ["X01"]
scope = ["crates/some-crate/src/other.rs"]
also_read = []
verify = "quick"

[[batch]]
id = "Y01"
title = "Independent thing in a different group"
group = "Y"
deps = []
scope = ["crates/another/src/thing.rs"]
also_read = []
verify = "quick"
```

**Field reference:**

| Field | Required | Description |
|---|---|---|
| `id` | yes | Unique batch ID. Convention: group letter + zero-padded number (A01, B03, etc.) |
| `title` | yes | Human-readable title. Used in commit messages and logs |
| `group` | yes | Group/wave letter. All batches in a group share a wave gate (cargo check + clippy runs once after the entire group completes) |
| `deps` | yes | Array of batch IDs that must succeed before this batch can start. Use `[]` for no dependencies |
| `scope` | yes | Array of file paths (relative to repo root) that this batch will modify. Used for anti-pattern checks and cumulative context |
| `also_read` | yes | Array of additional file paths to include in the prompt for context. The batch won't modify these but needs to see them |
| `verify` | yes | Verification mode. Use `"quick"` (anti-pattern grep only) for most batches |

**Dependency rules:**
- A batch won't start until ALL its deps have succeeded
- If a dep fails terminally, the batch is marked `blocked`
- Batches with no deps (or all deps satisfied) run in parallel up to `--parallel N`
- Cross-group deps are fine — the DAG handles arbitrary dependency graphs

### 4. Create prompt files

One file per batch, named `{id}.prompt.md` in the `prompts/` directory.

```markdown
## Task

Concise description of what to do.

## Problem

What's wrong currently and why it needs to change.

## Changes Required

1. In `crates/some-crate/src/file.rs`:
   - Add field `foo: Bar` to struct `Thing` (around line 42)
   - Update `impl Thing` to initialize `foo` in `new()`

2. In `crates/some-crate/src/other.rs`:
   - Call `thing.foo()` in the main loop (around line 100)

## Acceptance Criteria

- [ ] `Thing::new()` initializes `foo`
- [ ] The main loop calls `thing.foo()`
- [ ] No new warnings from clippy
- [ ] Existing tests still pass

## Do NOT

- Do not add new dependencies to Cargo.toml
- Do not modify files outside the scope list
- Do not add stub implementations that silently pass
```

**Prompt writing tips:**
- Be specific about file paths and line numbers — codex works better with precise locations
- Include "Do NOT" constraints to prevent common mistakes
- The prompt will automatically be prefixed with: context-pack files, cumulative context from prior batches, and live file contents from the worktree
- Keep prompts focused on one coherent change — don't pack unrelated work into one batch

### 5. Create context-pack files (optional but recommended)

These markdown files are injected into every batch prompt, sorted by filename. Use them for:

- **00-RULES.md** — coding rules, style guidelines, constraints
- **01-ARCHITECTURE.md** — crate structure, key types, how things connect
- **02-ANTI-PATTERNS.md** — known bad patterns with examples of what NOT to do

Keep these concise. Every byte here is multiplied by the number of batches.

## Running

```bash
# List all batches with deps
bash tmp/runners/my-runner/run.sh --list

# Show the DAG schedule without executing
bash tmp/runners/my-runner/run.sh --dry-run

# Run everything (3 parallel by default)
bash tmp/runners/my-runner/run.sh

# Run with more parallelism
bash tmp/runners/my-runner/run.sh --parallel 5

# Run only group X
bash tmp/runners/my-runner/run.sh --group X

# Run specific batches
bash tmp/runners/my-runner/run.sh --only X01,X02,Y01

# Skip cargo gates (faster, less safe)
bash tmp/runners/my-runner/run.sh --no-gate --no-test

# Pause between waves for manual inspection
bash tmp/runners/my-runner/run.sh --pause

# Resume after interrupt or failure
bash tmp/runners/my-runner/run.sh --continue

# Check status from another terminal while running
bash tmp/runners/my-runner/run.sh --status

# Or watch the status file directly
watch -n5 cat tmp/runners/my-runner/logs/run-*/status.json

# Follow a specific batch's codex session
# (the runner prints tail -f commands for each batch as it starts)
tail -f tmp/runners/my-runner/logs/run-*/X01.log

# Disk usage
bash tmp/runners/my-runner/run.sh --disk

# Clean old runs (keeps 2 most recent by default)
bash tmp/runners/my-runner/run.sh --cleanup
bash tmp/runners/my-runner/run.sh --cleanup-keep 1
```

## CLI reference

| Flag | Description |
|---|---|
| `--parallel N` | Max concurrent batches (default: 3) |
| `--only A,B,C` | Comma-separated batch IDs to run |
| `--group X` | Run only batches in this group |
| `--continue` | Resume the latest run (skip completed batches) |
| `--dry-run` | Show wave schedule without executing |
| `--list` | List batches and exit |
| `--status` | Print status.json from latest run |
| `--disk` | Show disk usage |
| `--cleanup` | Clean old runs and exit |
| `--cleanup-keep N` | Keep N most recent runs (default: 2) |
| `--no-gate` | Skip wave gates (cargo check + clippy) |
| `--no-test` | Skip end-of-run test gate |
| `--pause` | Pause between waves for manual inspection |

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `RUNNER_NAME` | `default` | Unique name for this runner. Namespaces worktrees, target dirs, logs |
| `PARALLEL` | `3` | Same as `--parallel` |
| `MAX_RETRIES` | `2` | Attempts per batch before marking as failed |
| `CONV_MODEL` | `gpt-5.5` | Model passed to `codex exec --model` |
| `CONV_REASONING` | `high` | Reasoning effort passed to codex |
| `CONV_TIMEOUT` | `5400` | Seconds before killing a codex session (90 min) |
| `CONV_MIN_FREE_MB` | `5000` | Minimum free disk space in MB before aborting |
| `CUMULATIVE_BUDGET_KB` | `50` | Max size of cumulative context section in prompts |

## How the DAG works

The scheduler loops:

1. Find all batches whose deps are satisfied and aren't done yet → "ready" set
2. Take up to `PARALLEL` from the ready set
3. For each: create a sub-worktree → run codex → AP check → merge back (flock-serialized)
4. After a group's batches all complete, run wave gate (cargo check + clippy)
5. Repeat until no batches are ready
6. Run test gate (cargo test --workspace)

Example with `--parallel 3`:

```
Wave 1: [A01 A02 B03]     — 3 batches, all independent, run together
Wave 2: [A03 A04 B02 B05] — 4 ready, dispatched as [A03 A04 B02] then [B05]
Wave 3: [A05 B01 C01]     — A05 needed A01-A04; B01 needed A01+A04; C01 needed A02+A03
...
```

## Batch statuses

| Status | Meaning |
|---|---|
| `success` | Codex produced changes, AP checks passed, merged |
| `success_noop` | Codex ran but produced no file changes |
| `spawn_failed` | Codex exited non-zero |
| `timeout` | Codex exceeded CONV_TIMEOUT |
| `antipattern_failed` | AP checks found violations |
| `merge_failed` | Changes conflicted with another batch's merge |
| `blocked` | A dependency failed terminally |
| `in_progress` | Currently running |

## Anti-pattern checks

These run on every batch's scope files after codex completes (instant, no cargo):

| ID | Check |
|---|---|
| AP-1 | Stubs that silently pass (`Verdict::pass` with stub/noop/todo/placeholder) |
| AP-2 | `block_on` in async code |
| AP-3 | Duplicate trait definitions vs `foundation.rs` |
| AP-5 | Shelling out to `claude` or `codex` CLI |
| AP-6 | Inline prompt strings (`format!("You are a..."`) |
| AP-7 | `std::sync::Mutex` held across `.await` |

## Multiple runners on the same repo

Each runner uses `RUNNER_NAME` to namespace everything:
- Worktrees: `.roko/worktrees/{RUNNER_NAME}-{run_id}-{batch}`
- Target dirs: `$TMPDIR/roko-par-{RUNNER_NAME}/{run_id}/`
- Logs: `{RUNNER_ROOT}/logs/`

You can run `converge-followup` and `my-runner` simultaneously — they won't collide.

## Disk management

The biggest disk consumer is `CARGO_TARGET_DIR` (5-10GB per run). The runner:
- Checks free space before each wave (aborts at 2GB)
- Auto-cleans old runs on startup (keeps 2 most recent)
- Namespaces target dirs so different runners don't share (but batches within a run DO share for incremental compilation)
- Cleans orphaned target dirs and worktrees on `--cleanup`

## After a run completes

```bash
# Inspect the merged result
cd .roko/worktrees/my-runner-run-YYYYMMDD-HHMMSS-main
git log --oneline

# If satisfied, merge to your working branch
git -C /path/to/repo merge codex/my-runner-run-YYYYMMDD-HHMMSS

# Clean up
bash tmp/runners/my-runner/run.sh --cleanup
```
