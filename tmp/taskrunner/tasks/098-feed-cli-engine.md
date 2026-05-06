# Task 098: Feed CLI + Engine Integration

```toml
id = 98
title = "Add roko feed CLI commands and wire feeds into Engine and SSE layer"
track = "v2-core-abstractions"
wave = "wave-4"
priority = "high"
blocked_by = [97, 67]
touches = [
    "crates/roko-cli/src/commands/mod.rs",
    "crates/roko-cli/src/commands/feed.rs",
    "crates/roko-cli/src/main.rs",
    "crates/roko-serve/src/routes/feeds.rs",
    "crates/roko-serve/src/routes/sse.rs",
    "crates/roko-serve/src/state.rs",
]
exclusive_files = ["crates/roko-cli/src/commands/feed.rs"]
estimated_minutes = 180
```

## Context

This task wires the `Feed` trait implementations from task 097 into the
runtime, adds CLI commands for introspection, and routes feed events
through the existing SSE layer.

Checklist item: P3-4.

After task 097, `FileWatchFeed` and `ProviderHealthFeed` exist as structs
but nothing starts them, nothing reads their status, and no Pulses flow.
This task changes that: feeds are started at serve-time, their status is
queryable via CLI, and their Pulses appear in the SSE stream.

## Background

Read these files before starting:

1. `crates/roko-core/src/feed.rs` — `Feed` trait + impls from task 097
2. `crates/roko-cli/src/commands/mod.rs` — how commands are registered
   (follow the existing `learn`, `knowledge`, or `status` patterns)
3. `crates/roko-cli/src/main.rs` — top-level subcommand dispatch
4. `crates/roko-serve/src/routes/sse.rs` — SSE handler (`/api/events`)
   subscribes to `state.state_hub.subscribe_events()`. Feed Pulses should
   arrive here as `DashboardEvent` variants.
5. `crates/roko-serve/src/state.rs` — `AppState` struct. Feed registry
   lives here.
6. `crates/roko-cli/src/tui/fs_watch.rs` — the TUI's existing file
   watcher. After this task, the TUI should use `FileWatchFeed::start()`
   instead of `watch_roko_dir_with_fallback()`.
7. StateHub projection layer — currently path-included by `roko-serve` from
   `crates/roko-core/src/state_hub.rs`. Task 104 tracks moving this to a real crate
   boundary before new feed code depends on it directly.

## Current Checkout Corrections

These notes are authoritative for this checkout and override stale examples below:

- `crates/roko-cli/src/main.rs` uses a private `Command` enum and
  `dispatch_subcommand(command, cli)`, not a public `Commands` enum or
  `CommandContext`. Add `Command::Feed { cmd: FeedCmd }` or `Command::Feed(FeedArgs)` and
  dispatch it in `dispatch_subcommand()` following `Learn`/`Bench` patterns.
- `crates/roko-cli/src/commands/mod.rs` is just module registration. Command argument
  enums are currently defined in `main.rs`, while handler functions live under
  `commands/*`. Keep that local pattern unless a neighboring command has already moved its
  enum into the command module.
- `AppState` already has `feeds: RwLock<FeedRegistry>` for descriptor CRUD. Do not replace
  it with runtime feed handles. Add a separate field such as `runtime_feeds: ServeFeeds`.
- `AppState::new()` is synchronous. If feed startup remains in `state.rs`, use
  `tokio::runtime::Handle::try_current().spawn(async move { ... })` and log startup errors.
  An async startup helper is cleaner, but adding a call site in `roko-serve/src/lib.rs`
  requires adding that file to the touch list.
- Runtime feed routes belong in the existing `crates/roko-serve/src/routes/feeds.rs`
  router. It is already merged under `/api`, so route paths in that file are
  `/feeds/runtime` and `/feeds/runtime/{id}`.
- `DashboardEvent::FsChanged` does not exist. Either add a new event variant in
  `crates/roko-core/src/dashboard_snapshot.rs` and update the touch list, or use the
  existing `DashboardEvent::EventLogEntry { event_type: "feed_pulse", ... }` bridge. With
  the current touch list, use `EventLogEntry`.
- The TUI watcher is started from `crates/roko-cli/src/tui/app.rs`, not from
  `fs_watch.rs`. This task should not migrate the TUI watcher unless `app.rs` is added to
  the touch list; keep the existing direct watcher path for now.
- If task 097 renamed status to `pulses_produced`, use that field in CLI output and JSON.
  Do not print or deserialize `signals_produced` unless 097 intentionally kept a serde alias.

## Recovery Worker 19 Checkout Notes

Use these details to avoid crossing crate and touch-list boundaries:

- Existing feed routes in `crates/roko-serve/src/routes/feeds.rs` are descriptor CRUD over
  `AppState.feeds: RwLock<FeedRegistry>`. Add runtime route handlers beside them and use a
  new `AppState.runtime_feeds` or `AppState.serve_feeds` field; do not rename the existing
  registry field because `/api/feeds` tests and callers depend on it.
- Runtime route tests in this file should call `/feeds/runtime` and
  `/feeds/runtime/{id}` because the top-level server router applies the `/api` prefix
  elsewhere. Keep existing descriptor route tests for `/feeds` and `/feeds/{id}` green.
- `AppState::new()` currently builds `model_call_service`, `metrics`, `provider_health`,
  `state_hub`, and the descriptor `FeedRegistry` synchronously. Construct runtime feeds after
  `state_hub`/`provider_health` are available, then spawn startup with
  `Handle::try_current()`; if no runtime is present in a unit test, leave feeds disconnected
  but queryable.
- If task 097 exposes `CellContext { bus: Option<Arc<dyn BusErased>> }`, create a small
  serve-local bridge type in `state.rs` that implements `BusErased::publish_erased(Pulse)` by
  publishing `DashboardEvent::EventLogEntry { event_type: "feed_pulse", message:
  serde_json::to_string(&pulse).unwrap_or_default(), ... }` to `state.state_hub`. This avoids
  changing `dashboard_snapshot.rs` under the current touch list.
- `ProviderHealthFeed` should get its snapshot from serve state, not from `roko_learn`.
  Prefer a closure over `state.provider_health`/current provider status that returns a stable
  JSON object. If the exact tracker snapshot API is awkward, publish at least provider ids and
  current health summary already exposed by `ProviderHealthTracker`; do not fabricate provider
  health values.
- `commands/feed.rs` can use `reqwest` because neighboring CLI commands already use it.
  Resolve the base URL as `ROKO_SERVE_URL` or `http://localhost:6677`, trim a trailing slash,
  then call `/api/feeds/runtime`. The command is read-only and should print a clear "roko serve
  is not running" message on connection errors.
- `crate::Cli` in `main.rs` is private to the binary module. Command handlers that accept it
  should be `pub(crate)` and follow `commands::learn::dispatch_learn(cli, cmd).await` or
  `commands::bench::cmd_bench(cli, cmd).await`; do not try to make a public library API.
- Add CLI parse tests near the existing `Cli::try_parse_from` tests in `main.rs` for
  `roko feed list` and `roko feed status file-watch-roko-dir`. These tests should not require
  a live server.

## Mechanical Implementation Plan

1. Add `commands/feed.rs` with read-only handlers:
   `cmd_feed(cli: &Cli, cmd: FeedCmd) -> Result<i32>`, `list`, and `status`. Use
   `reqwest` if already available to `roko-cli`; otherwise use the existing HTTP helper
   pattern from neighboring commands.
2. Add `FeedCmd`/`FeedStatusArgs` to `main.rs` near `LearnCmd`, add `Command::Feed`, and
   route it in `dispatch_subcommand()`. Include parser tests next to existing CLI parse tests
   for `roko feed list` and `roko feed status file-watch-roko-dir`.
3. In `state.rs`, define `ServeFeeds` with `Arc<FileWatchFeed>` and
   `Arc<ProviderHealthFeed>`. Keep it separate from `FeedRegistry`.
4. Create feeds from `state.workdir` and `state.provider_health` snapshots. Start them from
   the sync constructor by spawning one async task per feed, or add a documented async
   helper if the touch list is expanded.
5. In `routes/feeds.rs`, extend `routes()` with
   `.route("/feeds/runtime", get(list_runtime_feeds))` and
   `.route("/feeds/runtime/{id}", get(get_runtime_feed_status))`. Return stable JSON with
   `id`, `topic`, `kind`, `connected`, `last_update_ms`, `rate_hz`, `pulses_produced`, and
   `error`.
6. Bridge feed publication to SSE through `state.state_hub.publish(DashboardEvent::EventLogEntry
   { event_type: "feed_pulse", plan_id: "", task_id: "", message: ... })` until a dedicated
   dashboard event variant is added. Do not create a second SSE route.
7. Add route tests in `routes/feeds.rs` for list/status/404 using `AppState::new()` test
   setup. Add command tests in `main.rs` for CLI parsing; avoid requiring a live server in
   unit tests.

## What to Change

### 1. Create `crates/roko-cli/src/commands/feed.rs`

```rust
//! `roko feed` — inspect and manage registered feeds.

use anyhow::Result;
use clap::{Args, Subcommand};
use roko_core::feed::FeedRuntimeStatus;

#[derive(Debug, Args)]
pub struct FeedArgs {
    #[command(subcommand)]
    pub command: FeedCommand,
}

#[derive(Debug, Subcommand)]
pub enum FeedCommand {
    /// List all registered feeds with their topics and status.
    List,
    /// Show detailed status for a specific feed.
    Status {
        /// Feed cell ID to inspect.
        id: String,
    },
}

pub async fn cmd_feed(cli: &crate::Cli, args: FeedArgs) -> Result<i32> {
    match args.command {
        FeedCommand::List => cmd_list(cli).await?,
        FeedCommand::Status { id } => cmd_status(cli, &id).await?,
    };
    Ok(crate::EXIT_SUCCESS)
}

async fn cmd_list(cli: &crate::Cli) -> Result<()> {
    // Query the serve endpoint if a server URL is configured; otherwise
    // show what's registered locally (empty if serve is not running).
    //
    // For the initial implementation: print a table showing known feeds
    // from the static registry in the serve state.  A running `roko serve`
    // instance exposes GET /api/feeds which returns FeedInfo entries.
    println!("{:<24} {:<32} {:<10} {}", "ID", "TOPIC", "KIND", "CONNECTED");
    println!("{}", "-".repeat(80));

    // Attempt to query serve at the configured base URL.
    let base = std::env::var("ROKO_SERVE_URL")
        .unwrap_or_else(|_| "http://localhost:6677".to_string());
    let url = format!("{base}/api/feeds/runtime");

    match reqwest::get(&url).await {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(feeds) = resp.json::<Vec<FeedSummary>>().await {
                for f in &feeds {
                    println!(
                        "{:<24} {:<32} {:<10} {}",
                        f.id, f.topic, f.kind,
                        if f.connected { "yes" } else { "no" }
                    );
                }
                if feeds.is_empty() {
                    println!("(no feeds registered — start roko serve to activate feeds)");
                }
            }
        }
        _ => {
            println!("(roko serve is not running; no live feed data available)");
            println!("Start the server with: roko serve");
        }
    }

    Ok(())
}

async fn cmd_status(cli: &crate::Cli, id: &str) -> Result<()> {
    let base = std::env::var("ROKO_SERVE_URL")
        .unwrap_or_else(|_| "http://localhost:6677".to_string());
    let url = format!("{base}/api/feeds/runtime/{id}");

    match reqwest::get(&url).await {
        Ok(resp) if resp.status().is_success() => {
            let status: FeedRuntimeStatus = resp.json().await?;
            println!("Feed: {id}");
            println!("  connected:        {}", status.connected);
            println!("  rate_hz:          {:.2}", status.rate_hz);
            println!("  pulses_produced:  {}", status.pulses_produced);
            if let Some(ms) = status.last_update_ms {
                println!("  last_update_ms:   {}", ms);
            }
            if let Some(err) = &status.error {
                println!("  error:            {}", err);
            }
        }
        Ok(resp) if resp.status().as_u16() == 404 => {
            anyhow::bail!("feed '{}' not found", id);
        }
        _ => {
            anyhow::bail!("roko serve is not running");
        }
    }

    Ok(())
}

/// Lightweight summary for list output (mirrors the runtime status).
#[derive(Debug, serde::Deserialize)]
struct FeedSummary {
    id: String,
    topic: String,
    kind: String,
    connected: bool,
}
```

### 2. Register `feed` in `crates/roko-cli/src/commands/mod.rs`

Follow the exact pattern used by `learn` or `status`:

```rust
pub mod feed;
```

### 3. Add `feed` to the top-level `Commands` enum in `crates/roko-cli/src/main.rs`

In the `Command` enum:

```rust
/// Inspect and manage registered data feeds.
Feed(crate::commands::feed::FeedArgs),
```

In the dispatch `match`:

```rust
Command::Feed(args) => crate::commands::feed::cmd_feed(cli, args).await,
```

### 4. Start feeds in `crates/roko-serve/src/state.rs`

Add a `FeedSet` to `AppState` that holds the active feeds and their status:

```rust
use roko_core::feed::{Feed, FileWatchFeed, ProviderHealthFeed};
use std::sync::Arc;

/// Active feeds registered with the serve runtime.
pub struct ServeFeeds {
    pub file_watch: Arc<FileWatchFeed>,
    pub provider_health: Arc<ProviderHealthFeed>,
}
```

In `AppState::new()` (or wherever AppState is constructed), start the feeds:

```rust
let cwd = std::env::current_dir().unwrap_or_default();
let workspace = roko_core::Workspace::open_or_create(&cwd)?;

let file_watch = Arc::new(FileWatchFeed::for_roko_dir(workspace.roko_dir()));
let provider_health = Arc::new(ProviderHealthFeed::default_interval());

// AppState::new() is sync. Spawn startup work on the current runtime.
if let Ok(handle) = tokio::runtime::Handle::try_current() {
    let file_watch_for_task = Arc::clone(&file_watch);
    let provider_health_for_task = Arc::clone(&provider_health);
    handle.spawn(async move {
        let ctx = roko_core::feed::CellContext::default();
        if let Err(e) = file_watch_for_task.start(&ctx).await {
            tracing::warn!(error = %e, "FileWatchFeed failed to start");
        }
        if let Err(e) = provider_health_for_task.start(&ctx).await {
            tracing::warn!(error = %e, "ProviderHealthFeed failed to start");
        }
    });
}
```

**Important**: Check whether `AppState::new()` is sync or async. If sync,
wrap the feed startup in `tokio::spawn` instead of awaiting directly.

### 5. Add `GET /api/feeds/runtime` and `GET /api/feeds/runtime/{id}` routes

Add to `crates/roko-serve/src/routes/feeds.rs` (file already exists — add
to the existing router):

Use the brace path syntax used by the current file (`/feeds/{id}`), so the
runtime route is `/feeds/runtime/{id}` rather than `:id`.

```rust
use axum::extract::Path;

pub fn runtime_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/feeds/runtime", get(list_runtime_feeds))
        .route("/feeds/runtime/{id}", get(get_runtime_feed_status))
}

async fn list_runtime_feeds(State(state): State<Arc<AppState>>) -> Json<Vec<serde_json::Value>> {
    let mut result = Vec::new();

    // File watch feed
    if let Ok(s) = state.runtime_feeds.file_watch.status().await {
        result.push(serde_json::json!({
            "id": state.runtime_feeds.file_watch.cell_id(),
            "topic": state.runtime_feeds.file_watch.topic().0,
            "kind": "Raw",
            "connected": s.connected,
            "pulses_produced": s.pulses_produced,
        }));
    }

    // Provider health feed
    if let Ok(s) = state.runtime_feeds.provider_health.status().await {
        result.push(serde_json::json!({
            "id": state.runtime_feeds.provider_health.cell_id(),
            "topic": state.runtime_feeds.provider_health.topic().0,
            "kind": "Meta",
            "connected": s.connected,
            "pulses_produced": s.pulses_produced,
        }));
    }

    Json(result)
}

async fn get_runtime_feed_status(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<FeedRuntimeStatus>, StatusCode> {
    let feeds: Vec<(&dyn Feed, &str)> = vec![
        (&*state.runtime_feeds.file_watch, state.runtime_feeds.file_watch.cell_id()),
        (&*state.runtime_feeds.provider_health, state.runtime_feeds.provider_health.cell_id()),
    ];

    for (feed, fid) in feeds {
        if fid == id {
            return feed.status().await
                .map(Json)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    Err(StatusCode::NOT_FOUND)
}
```

### 6. Publish `FileWatchFeed` Pulses into SSE stream

Publish through the existing `StateHub`, not by changing the SSE handler.
With the current touch list, use `DashboardEvent::EventLogEntry` as a
temporary bridge:

```rust
state.state_hub.publish(roko_core::DashboardEvent::EventLogEntry {
    timestamp_ms: now_ms as u64,
    event_type: "feed_pulse".to_string(),
    plan_id: String::new(),
    task_id: String::new(),
    message: serde_json::to_string(&pulse).unwrap_or_default(),
});
```

If a dedicated `DashboardEvent::FeedPulse` is preferred, add
`crates/roko-core/src/dashboard_snapshot.rs` to the touch list and update
snapshot application and event type filtering in the same task.

## What NOT to Do

- Do NOT redesign the SSE route (`/api/events`). Feed events piggyback on
  the existing `DashboardEvent` broadcast via `StateHub`. Do not add a new
  SSE endpoint.
- Do NOT require `roko serve` to be running for `roko feed list` to work.
  When the server is not reachable, print a clear message — do not panic.
- Do NOT change the TUI watcher in this task. Keep the TUI using
  `watch_roko_dir_with_fallback` until `crates/roko-cli/src/tui/app.rs` is
  explicitly in the touch list.
- Do NOT call `Feed::start()` from the CLI commands. Feeds are started only
  by the serve runtime. The CLI commands are read-only introspection.

## Wire Target

```bash
# With roko serve running in one terminal:
cargo run -p roko-cli -- serve &

# In another terminal:
cargo run -p roko-cli -- feed list
# Expected output:
# ID                       TOPIC                            KIND       CONNECTED
# --------------------------------------------------------------------------------
# file-watch-roko-dir      fs.changed                       Raw        yes
# provider-health-feed     provider.health                  Meta       yes

cargo run -p roko-cli -- feed status file-watch-roko-dir
# Expected: connected: true, pulses_produced: N, rate_hz: ...
```

## Verification

- [ ] `cargo build --workspace` — clean
- [ ] `cargo test --workspace` — no regressions
- [ ] `cargo clippy --workspace --no-deps -- -D warnings` — clean
- [ ] `cargo run -p roko-cli -- feed --help` — subcommand exists
- [ ] `cargo run -p roko-cli -- feed list` — prints table (or "not running" message)
- [ ] `cargo run -p roko-cli -- feed status file-watch-roko-dir` — prints status or "not running"
- [ ] `GET http://localhost:6677/api/feeds/runtime` returns JSON (when serve running)
- [ ] `GET http://localhost:6677/api/feeds/runtime/file-watch-roko-dir` returns status JSON
- [ ] `GET http://localhost:6677/api/feeds/runtime/nonexistent` returns 404
- [ ] `grep -n 'FeedCommand' crates/roko-cli/src/commands/feed.rs` — enum exists
- [ ] `grep -n 'Feed(' crates/roko-cli/src/main.rs` — dispatch arm exists

## Status Log

| Time | Agent | Action |
|------|-------|--------|
