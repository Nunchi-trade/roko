# Task 097: Feed Trait + FileWatchFeed + ProviderHealthFeed

```toml
id = 97
title = "Add Feed trait to roko-core, implement FileWatchFeed and ProviderHealthFeed"
track = "v2-core-abstractions"
wave = "wave-4"
priority = "high"
blocked_by = [35, 39, 40, 41]
touches = [
    "crates/roko-core/src/feed.rs",
    "crates/roko-core/src/traits.rs",
    "crates/roko-core/src/lib.rs",
    "crates/roko-cli/src/tui/fs_watch.rs",
    "crates/roko-cli/src/tui/mod.rs",
]
exclusive_files = ["crates/roko-core/src/feed.rs"]
estimated_minutes = 300
```

## Context

Phase 3A of the v2 refactoring introduces Feeds — Cells that connect to
external data sources and publish Pulses on the Bus. This task covers three
checklist items:

- **P3-1**: Add the `Feed` trait to `roko-core`
- **P3-2**: Implement `FileWatchFeed` — a Feed wrapping the existing notify
  debounce logic from `crates/roko-cli/src/tui/fs_watch.rs`
- **P3-3**: Implement `ProviderHealthFeed` — a Feed that polls providers on
  an interval and publishes health status Pulses

Feeds formalize how external data enters the system. Without them, each
consumer manages its own polling loop and error handling. With Feeds,
agents subscribe to Bus topics and receive data through a uniform
interface with built-in rate limiting and error handling.

Checklist items: P3-1, P3-2, P3-3.

## Background

Read these files before starting:

1. `crates/roko-core/src/feed.rs` — the current file is a `FeedRegistry`
   (data descriptor store for HTTP routes). The new `Feed` trait will be
   **added to this file**, not replace the existing registry. The registry
   and the trait serve different purposes and must coexist.
2. `crates/roko-core/src/cell.rs` — the `Cell` trait (Feed extends Cell)
3. `crates/roko-core/src/traits.rs` — `Connect` and `Trigger` protocol
   stubs already exist (sync, no-async yet). `Feed` builds on them.
4. `crates/roko-core/src/bus_backends.rs` — `BroadcastBus` and `MemoryBus`
   implementations. Use `BroadcastBus` for the Feed implementations.
5. `crates/roko-core/src/pulse.rs` — `Pulse`, `Topic`, `TopicFilter`
6. `crates/roko-cli/src/tui/fs_watch.rs` — the full `notify`-backed file
   watcher with poll fallback. FileWatchFeed wraps this logic.
7. `crates/roko-learn/src/provider_health.rs` — `ProviderHealthRegistry`
   with the circuit-breaker state machine. ProviderHealthFeed polls this.
8. `tmp/v2-refactoring/08-FEEDS.md` — the Feed design spec

## Current Checkout Corrections

These notes are authoritative for this checkout and override stale examples below:

- `FeedKind` already exists in `crates/roko-core/src/feed.rs` as
  `Raw/Derived/Composite/Meta`. Reuse it; do not define a second enum.
- `crates/roko-core/src/feed.rs` currently contains descriptor registry types
  (`FeedRegistry`, `FeedInfo`, `FeedAccess`). The runtime `Feed` trait must be added
  alongside them without changing registry semantics used by `roko-serve`.
- `CellContext` does not currently exist in `crates/roko-core/src/cell.rs`. This task is
  blocked by task 035, so first check whether it has landed. If it has not landed, add only
  a local placeholder in `feed.rs` and name the import sites accordingly; do not edit
  `cell.rs` in this task.
- `Connect` and `Trigger` in `traits.rs` are sync protocol traits. Do not make `Feed`
  inherit them unless the implementations can satisfy those methods. It is sufficient for
  `Cell::protocols()` to report `["Feed", "Connect", "Trigger"]`.
- `Bus` is not object-safe for storing directly in a context because of its associated
  receiver type. `bus_backends.rs` exposes `BusErased` for publish-only use. If the
  task-035 `CellContext` has no bus yet, `start()` should update runtime status only and
  task 098 will wire publish behavior.
- Feeds publish `Pulse`s, not `Signal`s. Prefer `pulses_produced` in
  `FeedRuntimeStatus`; if existing downstream code already expects `signals_produced`, add
  a serde alias rather than naming new code around signals.
- `roko-core` must not depend on `roko-cli` or `roko-learn`. `FileWatchFeed` may copy or
  extract the small fingerprint/debounce logic from `crates/roko-cli/src/tui/fs_watch.rs`,
  but it cannot call into the CLI crate. `ProviderHealthFeed` cannot import
  `ProviderHealthRegistry`; define a core-level health snapshot callback/source and bind it
  to serve's `ProviderHealthTracker` in task 098.
- `crates/roko-core/Cargo.toml` has `async-trait`, `tokio`, `parking_lot`, and `chrono`,
  but it does **not** have `notify` or `notify-debouncer-full`. Because this task's touch
  list does not include `crates/roko-core/Cargo.toml`, implement `FileWatchFeed` with a
  std-based polling fallback only, or expand the touch list before adding watcher
  dependencies.

## Recovery Worker 19 Checkout Notes

Use these details when turning the examples below into code:

- `Cell` lives in `crates/roko-core/src/cell.rs` and has only
  `cell_id()`, `cell_name()`, `cell_version()`, `protocols()`,
  `estimated_cost()`, and `estimated_duration()`. Do not add a
  `feed_kind_str()` method or any new required `Cell` method in this task.
- `Pulse` bodies should use `Body::from_json(&serde_json::json!({...}))?` or
  `Body::Json(value)`, not a signal-specific helper. `Topic` is a public tuple
  struct, so status/route code can read `topic.0` or use `topic.to_string()`.
- If `CellContext` still has to be local to `feed.rs`, make it useful:
  `#[derive(Clone, Default)] pub struct CellContext { pub bus:
  Option<Arc<dyn crate::bus_backends::BusErased>> }`. That gives task 098 a
  publish-only bridge without making the non-object-safe `Bus` trait part of
  the feed API.
- Store `latest_pulse: Arc<Mutex<Option<Pulse>>>` and `last_error:
  Arc<Mutex<Option<String>>>` on both concrete feeds. `poll()` should return
  the latest pulse when one exists; it must not always return `Ok(None)` once
  file/health events have been observed.
- Because no watcher dependency is available to `roko-core` under this touch
  list, copy only the recursive fingerprint behavior from
  `crates/roko-cli/src/tui/fs_watch.rs`: scan recursively, compare path/len/mtime
  fingerprints every 1 second, and apply the same 200 ms debounce before
  publishing. Leave the existing TUI watcher untouched.
- Avoid `tokio::spawn` examples that keep non-`Send` watcher state across
  awaits. For the polling implementation, a `std::thread::spawn` loop with an
  `AtomicBool` stop flag is enough and keeps `start()` nonblocking. If using
  Tokio tasks anyway, verify `cargo check -p roko-core` before relying on
  runtime/time features.
- `ProviderHealthFeed` must take an injected snapshot source such as
  `Arc<dyn Fn() -> serde_json::Value + Send + Sync>`. `default_interval()` can
  use an empty JSON object source. Do not import `roko_learn` or serve state.
- Tests in this task should be contained in `feed.rs`: start/stop idempotency,
  `poll()` returning a pulse after a temp file write, no pulse before a change,
  provider-health topic/kind/status, and `FeedRegistry` existing tests still
  passing.

## Mechanical Implementation Plan

1. In `feed.rs`, append `FeedRuntimeStatus`, optional local `CellContext` fallback, and
   `#[async_trait] pub trait Feed: Cell + Send + Sync`.
2. Use `fn topic(&self) -> &Topic`, `fn feed_kind(&self) -> FeedKind`, async
   `start/stop/poll/status`, and return `Result<Option<Pulse>>` from `poll()` unless the
   task-035 context explicitly requires `Signal`. The design goal is Pulse production.
3. Implement `FileWatchFeed` with fields: id, watched path, topic, running flag,
   `last_update_ms`, `pulses_produced`, last error, and latest pulse. The `.roko` convenience
   constructor should watch `workdir.join(".roko")` and publish/topic `fs.changed`.
4. Reuse the TUI watcher behavior mechanically: 200 ms debounce, bounded channel of 4, and
   1 s poll fallback using a recursive fingerprint of paths/mtime/len. Keep the existing TUI
   watcher behavior intact unless task 098 changes the call site.
5. On a file change, build a `Pulse` with topic `fs.changed`, kind appropriate for an
   event/update, body containing at least `{ "path": "...", "changed_at_ms": ... }`, update
   status, and publish only if the context exposes `BusErased`.
6. Implement `ProviderHealthFeed` as a core feed over an injected snapshot function:
   `Arc<dyn Fn() -> serde_json::Value + Send + Sync>`. `default_interval()` may use an empty
   snapshot until task 098 injects serve state. It should not import `roko_learn`.
7. Export `Feed`, `FeedRuntimeStatus`, `FileWatchFeed`, and `ProviderHealthFeed` from the
   existing `pub use feed::{...}` line in `lib.rs`.
8. Add unit tests in `feed.rs` for start/stop idempotency, status counters, file change
   detection via a temp `.roko` file write, and provider-health topic/kind. Tests must not
   require a live provider or serve process.

## What to Change

### 1. Add the `FeedStatus` struct and `Feed` trait to `crates/roko-core/src/feed.rs`

Append below the existing `FeedRegistry` impl. Do NOT modify the registry —
it has callers in `roko-serve` and must stay unchanged.

```rust
// ── Feed trait ────────────────────────────────────────────────────────────

use crate::error::Result;
use async_trait::async_trait;

/// Runtime status of a running Feed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FeedRuntimeStatus {
    /// Whether the feed is currently connected to its source.
    pub connected: bool,
    /// Approximate publication rate (events per second over last minute).
    pub rate_hz: f64,
    /// Unix milliseconds of the last published Pulse.
    pub last_update_ms: Option<i64>,
    /// Last error message, if any.
    pub error: Option<String>,
    /// Total Pulses published since start.
    pub pulses_produced: u64,
}

/// A continuous data stream that publishes Pulses to the Bus.
///
/// Feeds compose the Cell, Connect, and Trigger protocols into a
/// unified external-data ingestion interface.  They are activated by
/// the Engine and live for the duration of the runtime.
#[async_trait]
pub trait Feed: crate::cell::Cell {
    /// The Bus topic this feed publishes to.
    fn topic(&self) -> &crate::pulse::Topic;

    /// Classification of this feed's data lineage.
    fn feed_kind(&self) -> FeedKind;

    /// Start producing data.  Called by the Engine when the feed is
    /// activated.  Implementations should spawn a background task and
    /// return immediately.
    async fn start(&self, ctx: &CellContext) -> Result<()>;

    /// Stop producing data.  Must be idempotent.
    async fn stop(&self) -> Result<()>;

    /// Poll for the latest Pulse without subscribing.
    /// Returns `None` if no data has been produced yet.
    async fn poll(&self) -> Result<Option<crate::Pulse>>;

    /// Return the current runtime status of this feed.
    async fn status(&self) -> Result<FeedRuntimeStatus>;
}
```

**Note**: `CellContext` is expected from task 035. Check whether it exists
before adding `use crate::cell::CellContext;`. If it does not exist, define
a minimal local placeholder in `feed.rs` only:
`#[derive(Default)] pub struct CellContext;`. Do not edit `cell.rs` here.

### 2. Implement `FileWatchFeed` in `crates/roko-core/src/feed.rs`

`FileWatchFeed` publishes a `Pulse` with topic `"fs.changed"` whenever the
watched path tree changes. It wraps the same debounce logic as the TUI
watcher but exposes it through the `Feed` interface.

The code below is structural pseudocode from the original packet. In this
checkout, do not copy the `notify`, `setup_notify_watcher`, `tokio::spawn`, or
`tokio::time::sleep` parts verbatim unless `crates/roko-core/Cargo.toml` is
added to the touch list and the required features/dependencies are verified.
Under the current touch list, implement the background loop with std polling.

```rust
use std::path::PathBuf;
use std::sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}};
use parking_lot::Mutex;

/// Feed that watches a filesystem path and publishes change Pulses.
///
/// Wire target: replace direct use of `watch_roko_dir_with_fallback` in the
/// TUI with a `FileWatchFeed` (task 098 completes the wiring; this task
/// builds the implementation).
pub struct FileWatchFeed {
    /// Unique cell id.
    id: String,
    /// Directory to watch.
    path: PathBuf,
    /// Bus topic for published Pulses.
    topic: crate::pulse::Topic,
    /// Whether the feed has been started.
    running: Arc<AtomicBool>,
    /// Count of published Pulses.
    pulses_produced: Arc<AtomicU64>,
    /// Last publish timestamp (unix ms).
    last_update_ms: Arc<Mutex<Option<i64>>>,
}

impl FileWatchFeed {
    /// Create a new feed watching `path` and publishing on `topic`.
    pub fn new(id: impl Into<String>, path: PathBuf, topic: crate::pulse::Topic) -> Self {
        Self {
            id: id.into(),
            path,
            topic,
            running: Arc::new(AtomicBool::new(false)),
            pulses_produced: Arc::new(AtomicU64::new(0)),
            last_update_ms: Arc::new(Mutex::new(None)),
        }
    }

    /// Convenience constructor: watch `.roko/` in the given workdir.
    pub fn for_roko_dir(workdir: PathBuf) -> Self {
        Self::new(
            "file-watch-roko-dir",
            workdir.join(".roko"),
            crate::pulse::Topic::new("fs.changed"),
        )
    }
}

impl crate::cell::Cell for FileWatchFeed {
    fn cell_id(&self) -> &str { &self.id }
    fn cell_name(&self) -> &str { "FileWatchFeed" }
    fn protocols(&self) -> &[&str] { &["Feed", "Connect", "Trigger"] }
}

#[async_trait::async_trait]
impl Feed for FileWatchFeed {
    fn topic(&self) -> &crate::pulse::Topic { &self.topic }
    fn feed_kind(&self) -> FeedKind { FeedKind::Raw }

    async fn start(&self, ctx: &CellContext) -> Result<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Ok(()); // already running
        }

        // Clone everything needed by the background task.
        let path = self.path.clone();
        let topic = self.topic.clone();
        let running = Arc::clone(&self.running);
        let pulses_produced = Arc::clone(&self.pulses_produced);
        let last_update_ms = Arc::clone(&self.last_update_ms);

        // If CellContext exposes a publish-only bus handle (BusErased or
        // equivalent), publish the Pulse there. If it is still a local
        // placeholder, only update status/latest-pulse state; task 098 wires
        // serve-time publication.
        tokio::spawn(async move {
            use notify::{RecursiveMode, Watcher};
            use std::time::Duration;

            let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(8);

            // Best-effort watcher with a 200 ms debounce; fall back to the
            // recursive fingerprint poll from fs_watch.rs every 1 s if notify
            // setup fails. Do not use notify-debouncer-full unless the core
            // Cargo.toml touch list is updated to include that dependency.
            let debounce = Duration::from_millis(200);
            let result = setup_notify_watcher(&path, debounce, move |_| {
                let _ = tx.try_send(());
            });

            match result {
                Ok(mut debouncer) => {
                    let _ = debouncer.watcher().watch(&path, RecursiveMode::Recursive);
                    while running.load(Ordering::Relaxed) {
                        if rx.recv().await.is_some() {
                            let now_ms = chrono::Utc::now().timestamp_millis();
                            *last_update_ms.lock() = Some(now_ms);
                            pulses_produced.fetch_add(1, Ordering::Relaxed);
                            // Publish Pulse on ctx bus when available.
                        }
                    }
                }
                Err(_) => {
                    // Poll fallback: check path mtime every second.
                    while running.load(Ordering::Relaxed) {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        let now_ms = chrono::Utc::now().timestamp_millis();
                        *last_update_ms.lock() = Some(now_ms);
                        pulses_produced.fetch_add(1, Ordering::Relaxed);
                        // Publish Pulse on ctx bus when available.
                    }
                }
            }
        });

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn poll(&self) -> Result<Option<crate::Pulse>> {
        // FileWatchFeed publishes Pulses; return the latest pulse when wired.
        Ok(None)
    }

    async fn status(&self) -> Result<FeedRuntimeStatus> {
        Ok(FeedRuntimeStatus {
            connected: self.running.load(Ordering::Relaxed),
            rate_hz: 0.0, // populated by a rate tracker in a future pass
            last_update_ms: *self.last_update_ms.lock(),
            error: None,
            pulses_produced: self.pulses_produced.load(Ordering::Relaxed),
        })
    }
}
```

### 3. Implement `ProviderHealthFeed` in `crates/roko-core/src/feed.rs`

`ProviderHealthFeed` polls configured providers on an interval and publishes
a health summary Pulse on topic `"provider.health"`.

```rust
use std::time::Duration;

/// Feed that polls LLM providers and publishes health status Pulses.
///
/// Wire target: `roko serve` health endpoint subscribes to this feed's
/// topic so SSE clients receive provider health updates in real time
/// (task 098 completes the wiring).
pub struct ProviderHealthFeed {
    id: String,
    topic: crate::pulse::Topic,
    poll_interval: Duration,
    running: Arc<AtomicBool>,
    pulses_produced: Arc<AtomicU64>,
    last_update_ms: Arc<Mutex<Option<i64>>>,
}

impl ProviderHealthFeed {
    /// Create a new feed with the given poll interval.
    pub fn new(id: impl Into<String>, poll_interval: Duration) -> Self {
        Self {
            id: id.into(),
            topic: crate::pulse::Topic::new("provider.health"),
            poll_interval,
            running: Arc::new(AtomicBool::new(false)),
            pulses_produced: Arc::new(AtomicU64::new(0)),
            last_update_ms: Arc::new(Mutex::new(None)),
        }
    }

    /// Convenience constructor: poll every 30 seconds.
    pub fn default_interval() -> Self {
        Self::new("provider-health-feed", Duration::from_secs(30))
    }
}

impl crate::cell::Cell for ProviderHealthFeed {
    fn cell_id(&self) -> &str { &self.id }
    fn cell_name(&self) -> &str { "ProviderHealthFeed" }
    fn protocols(&self) -> &[&str] { &["Feed", "Connect"] }
}

#[async_trait::async_trait]
impl Feed for ProviderHealthFeed {
    fn topic(&self) -> &crate::pulse::Topic { &self.topic }
    fn feed_kind(&self) -> FeedKind { FeedKind::Meta }

    async fn start(&self, _ctx: &CellContext) -> Result<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        let running = Arc::clone(&self.running);
        let pulses_produced = Arc::clone(&self.pulses_produced);
        let last_update_ms = Arc::clone(&self.last_update_ms);
        let poll_interval = self.poll_interval;

        tokio::spawn(async move {
            while running.load(Ordering::Relaxed) {
                tokio::time::sleep(poll_interval).await;

                // Provider health check source is injected by task 098 from
                // serve state. Core must not import roko-learn here.
                let now_ms = chrono::Utc::now().timestamp_millis();
                *last_update_ms.lock() = Some(now_ms);
                pulses_produced.fetch_add(1, Ordering::Relaxed);
                // Publish Pulse on ctx bus when available with topic
                // "provider.health" and a JSON health snapshot body.
            }
        });

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn poll(&self) -> Result<Option<crate::Pulse>> {
        Ok(None)
    }

    async fn status(&self) -> Result<FeedRuntimeStatus> {
        Ok(FeedRuntimeStatus {
            connected: self.running.load(Ordering::Relaxed),
            rate_hz: 1.0 / self.poll_interval.as_secs_f64(),
            last_update_ms: *self.last_update_ms.lock(),
            error: None,
            pulses_produced: self.pulses_produced.load(Ordering::Relaxed),
        })
    }
}
```

### 4. Export new types from `crates/roko-core/src/lib.rs`

Add to the existing `pub use feed::` block:

```rust
pub use feed::{Feed, FeedKind, FeedAccess, FeedInfo, FeedRegistry, FeedRuntimeStatus,
               FileWatchFeed, ProviderHealthFeed};
```

Check the current `pub use feed::` line — extend it rather than replacing it.

### 5. Add integration test in `crates/roko-core/src/feed.rs`

```rust
#[cfg(test)]
mod feed_trait_tests {
    use super::*;

    #[tokio::test]
    async fn file_watch_feed_starts_and_stops() {
        let tempdir = tempfile::tempdir().unwrap();
        let feed = FileWatchFeed::for_roko_dir(tempdir.path().to_path_buf());

        let ctx = CellContext::default(); // placeholder until task 035 lands
        feed.start(&ctx).await.expect("start should succeed");

        let status = feed.status().await.expect("status OK");
        assert!(status.connected);

        feed.stop().await.expect("stop should succeed");

        // Give the background task a moment to observe the stop signal.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let status = feed.status().await.expect("status after stop");
        assert!(!status.connected);
    }

    #[tokio::test]
    async fn provider_health_feed_topic_is_correct() {
        let feed = ProviderHealthFeed::default_interval();
        assert_eq!(feed.topic().0, "provider.health");
        assert_eq!(feed.feed_kind(), FeedKind::Meta);
    }

    #[test]
    fn feed_kind_is_raw_for_file_watch() {
        let feed = FileWatchFeed::for_roko_dir(std::path::PathBuf::from("/tmp"));
        assert_eq!(feed.feed_kind(), FeedKind::Raw);
    }
}
```

## What NOT to Do

- Do NOT delete or modify the existing `FeedRegistry`, `FeedInfo`,
  `FeedKind`, or `FeedAccess` types. They have callers in `roko-serve`
  routes (`crates/roko-serve/src/routes/feeds.rs`). This task adds a new
  `Feed` trait alongside them.
- Do NOT fake Pulse publication if the context has no bus handle. Publish
  through the context when available; otherwise update feed runtime status
  and latest pulse only so task 098 can wire serve/SSE without hidden globals.
- Do NOT add the `Feed` trait as a supertrait to any existing types (gates,
  scorers). Feeds are a new protocol category.
- Do NOT build marketplace, paid feed, or blockchain feed logic. Phase 5+.
- Do NOT extract the TUI watcher into a separate crate in this task — that
  refactor is task 098's wire target.
- Do NOT add `CellContext` to `cell.rs` if task 035 has not landed — use
  the local placeholder defined in `feed.rs` and document it.
- Do NOT add `notify` or `notify-debouncer-full` to `roko-core` unless
  `crates/roko-core/Cargo.toml` is added to the task touch list. The current
  mechanical implementation should use std polling inside `feed.rs`.

## Wire Target

The wire target for this task is the integration test:

```bash
cargo test -p roko-core -- feed_trait_tests
# All three tests should pass
```

After this task, task 098 can wire the feeds into the Engine and SSE layer.

## Verification

- [ ] `cargo build --workspace` — no new compilation errors
- [ ] `cargo test --workspace` — no regressions in existing tests
- [ ] `cargo clippy --workspace --no-deps -- -D warnings` — clean
- [ ] `cargo test -p roko-core -- feed_trait_tests` — all 3 new tests pass
- [ ] `grep -n 'pub trait Feed' crates/roko-core/src/feed.rs` — trait exists
- [ ] `grep -n 'FeedRuntimeStatus' crates/roko-core/src/feed.rs` — struct exists
- [ ] `grep -n 'FileWatchFeed' crates/roko-core/src/feed.rs` — impl exists
- [ ] `grep -n 'ProviderHealthFeed' crates/roko-core/src/feed.rs` — impl exists
- [ ] `grep -rn 'FeedRegistry' crates/roko-serve/src/ --include='*.rs'` — existing callers still compile
- [ ] The existing `FeedRegistry` tests at the bottom of `feed.rs` still pass

## Status Log

| Time | Agent | Action |
|------|-------|--------|
