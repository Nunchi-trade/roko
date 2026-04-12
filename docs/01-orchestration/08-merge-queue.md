# Merge Queue

> **Module**: `roko-orchestrator/src/merge_queue.rs`
> **Key type**: `MergeQueue`
> **Tests**: 20 tests covering priority ordering, conflict detection, parallel
> non-conflicting merges, retry logic


> **Implementation**: Shipping

---

## Overview

The `MergeQueue` serializes plan merges to prevent file conflicts. When multiple
plans complete simultaneously, they cannot all merge at once — if Plan A and
Plan B both modified `crates/roko-core/src/lib.rs`, merging both simultaneously
would create a conflict.

The merge queue solves this by:

1. Tracking which files each plan modified
2. Detecting file overlaps between pending merges
3. Allowing non-conflicting merges to proceed in parallel
4. Serializing conflicting merges and retrying failed ones

---

## Architecture

```rust
pub struct MergeQueue {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    /// Pending merge requests, ordered by priority.
    pending: Vec<MergeRequest>,
    /// Currently merging requests (files locked).
    merging: HashMap<String, MergeRequest>,
    /// Files currently locked by in-progress merges.
    locked_files: HashSet<String>,
    /// Completed merge results.
    completed: Vec<MergeResult>,
}
```

The queue uses `parking_lot::Mutex` for thread-safe access without poisoning.
The `Arc` wrapper allows cloning the queue handle across async tasks.

---

## MergeRequest

```rust
pub struct MergeRequest {
    /// Plan identifier.
    pub plan_id: String,
    /// Git branch to merge.
    pub branch_name: String,
    /// Files modified by this plan (for conflict detection).
    pub files_changed: Vec<String>,
    /// Priority (higher merges first).
    pub priority: u32,
    /// Number of merge attempts so far.
    pub retry_count: u32,
}
```

The `files_changed` list is populated from the plan's `PlanState`, which
accumulates file paths as agents complete tasks. This list is the key input
for conflict detection.

---

## Operations

### enqueue()

```rust
pub fn enqueue(&self, request: MergeRequest)
```

Adds a merge request to the pending queue. The queue maintains priority
ordering: higher-priority requests are processed first. Among equal-priority
requests, the order of enqueue determines precedence.

### next_mergeable()

```rust
pub fn next_mergeable(&self) -> Option<MergeRequest>
```

Returns the highest-priority pending request that does not conflict with any
currently merging request. A conflict exists when:

```rust
request.files_changed.iter().any(|f| locked_files.contains(f))
```

If all pending requests conflict with in-progress merges, returns `None`. The
caller should wait for current merges to complete before retrying.

This algorithm is the critical safety mechanism. It guarantees that no two
concurrent merges touch the same files, preventing git merge conflicts at the
filesystem level.

### mark_merging()

```rust
pub fn mark_merging(&self, plan_id: &str)
```

Moves a request from `pending` to `merging` and adds its files to
`locked_files`. This reserves the files for the duration of the merge.

### mark_complete()

```rust
pub fn mark_complete(&self, plan_id: &str, success: bool)
```

Removes a request from `merging`, releases its files from `locked_files`, and
records the result. If `success` is false, the request may be re-enqueued for
retry (see below).

### mark_failed()

```rust
pub fn mark_failed(&self, plan_id: &str)
```

Handles merge failure with retry logic:

1. Increment `retry_count`
2. If `retry_count < MAX_RETRIES` (5), re-enqueue with reduced priority
   (the request goes to the back of its priority group)
3. If `retry_count >= MAX_RETRIES`, move to completed with failure status

The retry mechanism handles transient conflicts that resolve when other merges
complete first. For example, if Plan A and Plan B both modified
`Cargo.lock`, merging Plan A first and rebuilding Plan B's branch may resolve
the conflict automatically.

---

## Conflict Detection Algorithm

The conflict detection is straightforward but effective:

```
for each pending request R:
    for each file F in R.files_changed:
        if F in locked_files:
            R is conflicting → skip
    if R is not conflicting:
        return R  ← next mergeable
```

This is an O(P × F) algorithm where P is the number of pending requests and F
is the average number of files per request. In practice, P is small (< 10
concurrent plans) and F is manageable (< 100 files per plan), so performance
is not a concern.

### File-level granularity

Conflicts are tracked at the individual file level, not the plan or crate
level. This means:

- Plan A modifies `crates/roko-core/src/lib.rs` and `crates/roko-core/src/config.rs`
- Plan B modifies `crates/roko-core/src/types.rs` and `crates/roko-agent/src/pool.rs`

These two plans do NOT conflict (despite touching the same crate). They can
merge in parallel. Only plans that modify the *exact same files* are
serialized.

This granularity maximizes parallelism — serialization only occurs when
strictly necessary.

---

## Priority Ordering

Merges are processed in priority order:

1. Higher `priority` value → processes first
2. Equal priority → first-enqueued processes first (FIFO within priority class)

Priority comes from the plan's `PlanState.priority`, which is initialized from
the plan's frontmatter `priority` field and can be dynamically adjusted by the
conductor or operator.

---

## Retry with Backoff

When a merge fails, the request is re-enqueued with the same priority but
positioned after other requests at the same priority level. This implements
a simple form of backoff — the failed merge waits for other merges to complete,
which may resolve the conflict.

The maximum retry count (`MAX_RETRIES = 5`) prevents infinite retry loops.
After 5 failures, the plan transitions to `PlanPhase::Failed { reason: Deadlock }`.

### Why retry helps

Consider this scenario:

1. Plan A merges first, modifying `Cargo.lock`
2. Plan B tries to merge, but its `Cargo.lock` changes conflict with Plan A's
3. Plan B's merge is re-enqueued
4. Before Plan B retries, the batch branch is updated with Plan A's changes
5. Plan B rebases onto the updated batch branch, resolving the `Cargo.lock`
   conflict
6. Plan B's retry succeeds

This pattern is common with auto-generated files like `Cargo.lock`,
`Cargo.toml`, and aggregate exports.

---

## Integration with the Orchestrator

The merge queue is used by `PlanRunner` when processing `MergeBranch` actions:

```
executor.tick()
  → MergeBranch { plan_id: "01-workspace" }
    → merge_queue.enqueue(MergeRequest { ... })
    → merge_queue.next_mergeable()
      → if Some(request):
          merge_queue.mark_merging(plan_id)
          git merge roko/plan/01-workspace → batch-branch
          if success:
            merge_queue.mark_complete(plan_id, true)
            executor.apply_event(MergeSucceeded)
          else:
            merge_queue.mark_failed(plan_id)
            executor.apply_event(MergeFailed)
```

### Post-merge actions

After a successful merge, the `PostMergeRunner` runs regression detection:

1. Compile the merged result
2. Run tests
3. If regressions detected, flag for follow-up

This ensures that merges don't introduce cross-plan regressions — even though
individual plans passed their gates in isolation, the combination may fail.

---

## Thread Safety

The merge queue is designed for concurrent access:

- `Arc<Mutex<Inner>>` allows multiple async tasks to enqueue, query, and
  complete merges simultaneously
- `parking_lot::Mutex` is non-poisoning — a panic in one task doesn't
  permanently lock the queue
- File locks are tracked in a `HashSet<String>` for O(1) conflict checks

---

## Relationship to the DAG

The merge queue complements the `UnifiedTaskDag`:

- The **DAG** prevents conflicting tasks from *executing* simultaneously
  (via file-overlap inference edges)
- The **merge queue** prevents conflicting plans from *merging* simultaneously
  (via file-level lock tracking)

Both use file overlap as the conflict signal, but at different stages of the
pipeline. The DAG operates during implementation; the merge queue operates
during integration.

---

## References

- The merge queue pattern is common in CI/CD systems. GitHub's Merge Queue,
  GitLab's Merge Train, and Bors-NG all implement similar serialization
  for concurrent PRs.
- The file-level conflict detection is analogous to fine-grained locking in
  database systems (Gray, J. & Reuter, A. (1992). *Transaction Processing:
  Concepts and Techniques*. Morgan Kaufmann), where locks are held on
  individual records rather than entire tables.
- Retry with backoff follows the exponential backoff pattern from distributed
  systems, adapted here as positional backoff within the priority queue.
