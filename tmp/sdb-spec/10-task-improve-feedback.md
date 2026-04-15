# Checklist: Add task improve/feedback endpoint

## Implementation note (2026-04-15)

- `TaskEntry` now stores `parent_task_id`, and `TaskStore::create_improvement` creates an auto-assigned child task from a completed parent
- The implementation requires the parent task to have an assignee and inherits that assignee, the parent priority, and tags
- Improvement tasks use kind `improvement`, title `Improve: {parent.title}`, description equal to the feedback text, and `stake_wei = 0`
- `POST /api/tasks/{id}/improve` currently requires both `feedback` and `creator`

**Priority**: P2 — enables iterative job deliverables
**Estimated LOC**: ~50 lines
**Source**: `workspace/sdb/prds/jobs-prd.md`, `workspace/sdb/prds/product-design-review.md`, [GitHub #45](https://github.com/Nunchi-trade/collaboration/issues/45)

## Problem

After a job is delivered, there's no way for the user to say "improve this." The prediction goes from completed to done with no iteration. Sam's design review flagged this as a critical gap: "Need an 'ask the agent to improve this' follow-up loop."

## Approach

`POST /api/tasks/{id}/improve` creates a child task linked to the parent. The agent receives the parent's context (deliverables, summary) plus the user's feedback as a new task.

## Files to modify

### 1. `apps/mirage-rs/src/chain/task.rs`

- [ ] Add `parent_task_id: Option<u64>` field to `TaskEntry` (with `#[serde(default)]`)
- [ ] Add `TaskStore::create_improvement(parent_id, feedback, creator, now) -> Result<u64, TaskError>`:
  - Verify parent exists and is in `Completed` state
  - Create new task with:
    - `kind: "improvement"`
    - `description`: feedback text
    - `parent_task_id: Some(parent_id)`
    - Same `assignee` as the parent (agent that delivered)
    - Same `tags` as parent
    - Auto-assign to the same agent (state = Assigned)

### 2. `apps/mirage-rs/src/http_api/task.rs`

- [ ] Add request struct:
```rust
#[derive(Debug, Deserialize)]
pub struct ImproveTaskRequest {
    pub feedback: String,
    pub creator: String,
}
```

- [ ] Add handler:
```rust
/// `POST /api/tasks/{id}/improve` — request improvement on a completed task.
pub async fn improve_task(
    State(state): State<ApiState>,
    Path(id): Path<u64>,
    Json(req): Json<ImproveTaskRequest>,
) -> Result<Json<Value>, ApiError> {
    if req.feedback.is_empty() {
        return Err(ApiError { error: "feedback must not be empty".into(), code: 400 });
    }
    let now = now_secs();
    let mut chain = state.chain.write();
    let child_id = chain.task_store
        .create_improvement(id, req.feedback.clone(), req.creator.clone(), now)
        .map_err(task_error_to_api)?;

    let _ = chain.task_bus.send(TaskEvent::Created {
        id: child_id,
        title: format!("Improvement on task #{id}"),
        kind: "improvement".to_string(),
        creator: req.creator.clone(),
    });

    Ok(Json(json!({
        "ok": true,
        "parent_task_id": id,
        "improvement_task_id": child_id,
        "created_at": now,
    })))
}
```

### 3. `apps/mirage-rs/src/http_api/mod.rs`

- [ ] Add route:
```rust
.route("/tasks/{id}/improve", post(task::improve_task))
```

## Response shape

### `POST /api/tasks/{id}/improve`
```json
{
  "ok": true,
  "parent_task_id": 42,
  "improvement_task_id": 43,
  "created_at": 1713100800
}
```

## Testing

- [ ] Improve a completed task → creates child task with same assignee
- [ ] Improve a non-completed task → returns 409
- [ ] Improve a nonexistent task → returns 404
- [ ] Child task has `parent_task_id` set to original
- [ ] Child task `kind` is "improvement"
