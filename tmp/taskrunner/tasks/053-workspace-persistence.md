# Task 053: Add Workspace Persistence for Server Restarts

```toml
id = 53
title = "Persist workspace registry to disk so workspaces survive server restart"
track = "infrastructure"
wave = "wave-2"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-serve/src/state.rs",
    "crates/roko-serve/src/routes/workspaces.rs",
    "crates/roko-serve/src/lib.rs",
    "crates/roko-serve/tests/workspaces_persistence.rs",
    "crates/roko-core/src/config/serve.rs",
    "crates/roko-core/src/config/schema.rs",
]
exclusive_files = [
    "crates/roko-serve/src/state.rs",
    "crates/roko-serve/src/routes/workspaces.rs",
]
estimated_minutes = 180
```

## Context

The audit (S2) identified that `ephemeral_workspaces: RwLock<HashMap<...>>` in
`AppState` is in-memory only. On server restart, all workspace state is lost. The demo
UI creates workspaces that vanish on page refresh after a restart.

Additional issues:
- macOS `std::env::temp_dir()` returns `/var/folders/...` not `/tmp/` — paths mismatch
- No workspace recovery when paths become invalid
- 1-hour GC interval is too long for dev (workspaces accumulate)

## Background

Read:
- `crates/roko-serve/src/state.rs` — `ephemeral_workspaces` field, `WorkspaceGc`
- `crates/roko-serve/src/routes/workspaces.rs` — workspace CRUD routes
- `crates/roko-serve/src/lib.rs` — `start_workspace_gc` is started by
  `ServerBuilder::start_background`; current interval uses
  `roko_core::defaults::DEFAULT_WORKSPACE_GC_INTERVAL_SECS`
- `crates/roko-core/src/config/serve.rs` — `ServerConfig` currently has
  `bind`, `port`, `cors_origins`, `auth_token`, and `unsafe_public_cors`
- `crates/roko-core/src/config/schema.rs` — example config rendering for
  `[server]`

Grep for workspace management:
```bash
rg -n "ephemeral_workspaces|WorkspaceGc|workspace_gc|start_workspace_gc|WorkspaceInfo" crates/roko-serve/src crates/roko-core/src/config --glob '*.rs'
```

Current code facts:
- `AppState::new_with_daimon_strategy_and_state_hub` initializes
  `ephemeral_workspaces: RwLock::new(HashMap::new())` in `state.rs`.
- `WorkspaceInfo` currently contains only `id`, `path`, and `created_at` as Unix
  seconds.
- `routes/workspaces.rs` creates workspace directories under
  `std::env::temp_dir()`, writes a resolved `roko.toml` with `tokio::fs::write`,
  inserts into `state.ephemeral_workspaces`, and does not persist the map.
- `start_workspace_gc` in `lib.rs` already runs every
  `DEFAULT_WORKSPACE_GC_INTERVAL_SECS`, which is currently 300 seconds. The
  missing piece is the requested `[server].workspace_gc_interval_secs` override,
  not the default value.

## What to Change

### 1. Add workspace registry file

Create a workspace registry at `.roko/workspaces.json`:

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct WorkspaceRegistry {
    pub workspaces: HashMap<String, WorkspaceEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub id: String,
    pub path: PathBuf,
    pub created_at: u64,
    pub last_accessed_at: u64,
    pub status: WorkspaceStatus,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum WorkspaceStatus {
    Active,
    Stale,
}
```

Use Unix seconds (`u64`) to match the existing `WorkspaceInfo::created_at`
representation. Add `last_accessed_at` and `status` to `WorkspaceInfo` or add a
separate `WorkspaceEntry` conversion type; keep API response JSON stable unless
there is a test proving UI compatibility.

Add helpers in `crates/roko-serve/src/state.rs`:
- `fn workspace_registry_path_for(workdir: &Path) -> PathBuf`
- `fn load_workspace_registry(workdir: &Path) -> HashMap<String, WorkspaceInfo>`
- `async fn persist_workspace_registry(&self) -> anyhow::Result<()>`
- `async fn insert_workspace(&self, info: WorkspaceInfo) -> anyhow::Result<()>`
- `async fn remove_workspace(&self, id: &str) -> anyhow::Result<Option<WorkspaceInfo>>`
- `async fn get_workspace_info(&self, id: &str) -> Option<WorkspaceInfo>`
- `async fn touch_workspace(&self, id: &str) -> anyhow::Result<Option<WorkspaceInfo>>`

Persistence should serialize the map in deterministic order before writing, or
use a `BTreeMap` in the persisted registry type, then write with
`roko_core::io::atomic_write_async` or `roko_fs::atomic_write_json`.

### 2. Load on startup

In `AppState` construction, load the workspace registry from
`.roko/workspaces.json` if it exists. Do this inside `state.rs` before building
the final `AppState` so all direct `AppState::new` tests get the behavior.
Validate each entry:
- If the workspace path still exists on disk → mark `Active`
- If the path is gone → mark `Stale` and keep it in the map so `GET
  /api/workspaces/:id` can attempt recovery

If the registry file is corrupt, log `tracing::warn!` with the registry path and
start with an empty map. Do not panic during server startup because one
workspace registry file is malformed.

### 3. Save on workspace create/delete

After creating or deleting a workspace, persist the registry to disk using
`roko_core::io::atomic_write` or `roko_fs::atomic_write_json`.

Route behavior:
- `create_workspace`: insert into the map only after directory creation,
  `.roko` layout creation, config write, and optional git init all succeed. If
  registry persistence fails after insertion, remove the map entry, clean up the
  directory best-effort, and return HTTP 500.
- `delete_workspace`: remove from the map, persist, then remove the directory
  best-effort. If persistence fails, reinsert the removed info and return HTTP
  500.
- Keep config copy behavior intact, but prefer `roko_core::io::atomic_write_async`
  for the workspace `roko.toml` copy so a partial config is not left behind.

### 4. Update `last_accessed` on use

When a workspace is accessed via API (`GET /api/workspaces/:id`), update
`last_accessed` and persist.

`GET /api/workspaces` may list stale entries, but each object should still have
the existing `id`, `path`, and `created_at` fields. If adding `status` to the
response, keep it additive and update tests accordingly.

### 5. Shorten GC interval

Change workspace GC from 60 minutes to 5 minutes for dev. Make it configurable via:
```toml
[server]
workspace_gc_interval_secs = 300
```

The default is already 300 seconds in `roko_core::defaults`. Implement the
configurability:
1. Add `workspace_gc_interval_secs: u64` to
   `crates/roko-core/src/config/serve.rs::ServerConfig` with serde default
   `crate::defaults::DEFAULT_WORKSPACE_GC_INTERVAL_SECS`.
2. Include the field in `Default` and in the `[server]` example rendering in
   `crates/roko-core/src/config/schema.rs`.
3. In `crates/roko-serve/src/lib.rs::start_workspace_gc`, read
   `state.load_roko_config().server.workspace_gc_interval_secs` once before
   creating the interval. Clamp zero to the default or to 1 second; do not allow
   a zero-duration busy loop.

### 6. Re-validate on access

When a workspace is requested by ID:
- Check if the path still exists
- If not, attempt to re-create at the same path
- If re-creation succeeds, run `RokoLayout::for_project(&path).ensure_dirs().await`,
  mark the workspace `Active`, update `last_accessed_at`, persist, and continue
  returning the normal state dump.
- If re-creation fails, mark the workspace `Stale`, persist best-effort, and
  return HTTP `410 Gone` with JSON containing `error`, `id`, and `path`, plus a
  message telling the caller to create a new workspace.

Do not compare paths by string prefix such as `/tmp`. On macOS, temp paths under
`/var/folders/...` are normal. Persist and return the actual absolute path
created by `std::env::temp_dir()`.

## What NOT to Do

- Don't add tmux/session reattach — that is a separate concern (terminal sessions).
- Don't change the workspace directory structure on disk.
- Don't add workspace-to-workspace migration.
- Don't change the API response format — just make workspaces survive restarts.
- Don't make this a database — a JSON file is fine for the expected scale (< 100
  workspaces).
- Don't remove stale registry entries during startup; startup should be
  read-only recovery plus status marking.
- Don't hold the `ephemeral_workspaces` lock while doing filesystem IO except for
  the brief serialization snapshot needed by `persist_workspace_registry`.
- Don't make GC delete `Stale` entries immediately on every tick without honoring
  the existing max-age behavior.

## Tests to Add

Add `crates/roko-serve/tests/workspaces_persistence.rs` using the same test
runtime pattern as `api_integration.rs`:
- `create_workspace_persists_registry`: POST `/api/workspaces`, assert
  `.roko/workspaces.json` exists under the server workdir and contains the new
  workspace id/path.
- `workspace_survives_app_state_restart`: create a workspace through the router,
  build a second `AppState` using the same server workdir, call
  `GET /api/workspaces`, and assert the workspace id is present.
- `get_workspace_recreates_missing_path`: pre-seed or create a workspace, remove
  the workspace directory, call `GET /api/workspaces/:id`, and assert the path is
  recreated and the response is 200.
- `get_workspace_returns_gone_when_recreate_fails`: seed a registry entry whose
  parent path cannot be created (for example a file where a parent directory
  should be), then assert `GET /api/workspaces/:id` returns `410`.
- `delete_workspace_removes_registry_entry`: DELETE a workspace and assert a
  fresh `AppState` no longer loads the id.

## Wire Target

```bash
# Start serve, create workspace, restart, verify workspace still exists
cargo run -p roko-cli -- serve > /tmp/roko-serve-workspaces.log 2>&1 &
serve_pid=$!
for i in {1..40}; do
  curl -sf http://127.0.0.1:6677/api/health && break
  sleep 0.25
done
# Create workspace
curl -X POST http://localhost:6677/api/workspaces -H 'Content-Type: application/json' -d '{"prefix":"test"}'
# Kill serve
kill -TERM "$serve_pid"
wait "$serve_pid" || true
# Restart serve
cargo run -p roko-cli -- serve > /tmp/roko-serve-workspaces-2.log 2>&1 &
serve_pid=$!
for i in {1..40}; do
  curl -sf http://127.0.0.1:6677/api/health && break
  sleep 0.25
done
# Workspace should still exist
curl http://localhost:6677/api/workspaces
kill -TERM "$serve_pid"
wait "$serve_pid" || true
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test -p roko-serve --test workspaces_persistence`
- [ ] `cargo test -p roko-core config::serve`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `.roko/workspaces.json` is created after workspace creation
- [ ] Workspace survives server restart
- [ ] Stale workspace paths are detected and reported
- [ ] `[server].workspace_gc_interval_secs` roundtrips through `RokoConfig`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
