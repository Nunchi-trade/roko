# Checklist: Extend task completion with artifacts, summary, and metadata

**Priority**: P1 — enables job deliverables in dashboard
**Estimated LOC**: ~100 lines
**Source**: `workspace/sdb/job-deliverables-spec.md`, [GitHub #45](https://github.com/Nunchi-trade/collaboration/issues/45)

## Problem

`POST /api/tasks/{id}/complete` currently accepts only `{ result_insight_id: Option<String> }`. Agents produce files, reports, and transformed data. The dashboard My Jobs panel previously had rich mock deliverables (findings, attachments, provenance) that were stripped when going live. Needs `artifacts`, `summary`, and `completion_metadata` fields.

## Files to modify

### 1. `apps/mirage-rs/src/chain/task.rs`

- [ ] Add artifact and metadata types:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskArtifact {
    pub name: String,
    pub content_type: String,
    pub hash: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionMetadata {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub snapshot_block: Option<u64>,
    #[serde(default)]
    pub duration_s: u64,
    #[serde(default)]
    pub cost_usd: f64,
}
```

- [ ] Add fields to `TaskEntry`:
```rust
pub struct TaskEntry {
    // ...existing fields...
    #[serde(default)]
    pub artifacts: Vec<TaskArtifact>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub completion_metadata: Option<CompletionMetadata>,
}
```

- [ ] Update `TaskStore::complete()` to accept and store the new fields. Current signature takes `(id, result_insight_id, now)`. Extend to also accept `artifacts: Vec<TaskArtifact>`, `summary: Option<String>`, `completion_metadata: Option<CompletionMetadata>`.

### 2. `apps/mirage-rs/src/http_api/task.rs`

Current `CompleteTaskRequest` (line 274):
```rust
pub struct CompleteTaskRequest {
    #[serde(default)]
    pub result_insight_id: Option<String>,
}
```

- [ ] Extend to:
```rust
pub struct CompleteTaskRequest {
    #[serde(default)]
    pub result_insight_id: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<TaskArtifact>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub completion_metadata: Option<CompletionMetadata>,
}
```
(Import `TaskArtifact` and `CompletionMetadata` from `crate::chain::task`)

- [ ] Update `complete_task()` handler (line 282) to pass new fields to `task_store.complete()`

- [ ] Add new endpoint handler:
```rust
/// `GET /api/tasks/{id}/artifacts` — list artifacts for a completed task.
pub async fn get_task_artifacts(
    State(state): State<ApiState>,
    Path(id): Path<u64>,
) -> Result<Json<Value>, ApiError> {
    let chain = state.chain.read();
    match chain.task_store.get(id) {
        Some(task) => Ok(Json(json!({
            "task_id": id,
            "artifacts": task.artifacts,
            "summary": task.summary,
            "completion_metadata": task.completion_metadata,
        }))),
        None => Err(ApiError { error: format!("task not found: {id}"), code: 404 }),
    }
}
```

### 3. `apps/mirage-rs/src/http_api/mod.rs`

- [ ] Add route after existing task routes (after line 258):
```rust
.route("/tasks/{id}/artifacts", get(task::get_task_artifacts))
```

## Request/Response shapes

### `POST /api/tasks/{id}/complete` (extended)
```json
{
  "result_insight_id": "abc123",
  "artifacts": [
    { "name": "analysis.json", "content_type": "application/json", "hash": "sha256:...", "size": 2100 },
    { "name": "risk_report.md", "content_type": "text/markdown", "hash": "sha256:...", "size": 4800 }
  ],
  "summary": "Identified 3 reentrancy vectors in Vault.sol",
  "completion_metadata": {
    "model": "claude-sonnet-4",
    "method": "direct_observation",
    "snapshot_block": 45198,
    "duration_s": 862,
    "cost_usd": 0.45
  }
}
```

### `GET /api/tasks/{id}/artifacts`
```json
{
  "task_id": 42,
  "artifacts": [...],
  "summary": "...",
  "completion_metadata": {...}
}
```

## Backward compatibility

All new fields are `#[serde(default)]` / `Option` / `Vec` — existing clients sending `{ "result_insight_id": "..." }` continue to work without changes.

## Testing

- [ ] Complete task with old payload (only `result_insight_id`) → still works
- [ ] Complete task with artifacts + summary + metadata → all fields stored and retrievable
- [ ] `GET /api/tasks/{id}/artifacts` on completed task → returns artifacts
- [ ] `GET /api/tasks/{id}/artifacts` on incomplete task → returns empty artifacts list
- [ ] `GET /api/tasks/{id}` → includes artifacts in full task response
