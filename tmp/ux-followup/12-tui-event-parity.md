# TUI Event-Parity — Polling Is a Bug

> **New file** added 2026-04-16 during post-PR-13 audit. The user has confirmed
> that the TUI's file-polling refresh paths are bugs to fix, **not** intentional
> fallbacks. This file collects every polling site discovered during the sweep
> and flags them P0/P1/P2 according to user-visible impact.
>
> **Re-audit 2026-04-20**: 4 items closed (68, 69, 75, 77).
> 7 items still open (70, 71, 72, 73, 74, 76, 78).
> Note: item 68 (standalone hub) and 69 (polling thread) are resolved by
> `fs_watch.rs` (notify-based watcher) and in-process SharedStateHub. Item 75
> (git polling) replaced by `git_watch.rs`. Item 77 (unbounded channel) replaced
> by `watch::channel`. Verdict reading (items 83/87) done via `verdicts.rs`
> incremental substrate reader.

## Summary

The TUI mixes three refresh strategies:

1. **Push** via `StateHub` `snapshot_rx` (the intended design — sub-100 ms
   latency).
2. **Background-thread polling** via `std::thread::sleep(500ms)` over
   `.roko/`.
3. **Sync polling** at draw time when `snapshot_rx.is_none()`.

Anything other than (1) is a bug. The 11 items below catalogue every site we
found. They cluster naturally into three batches: **T27** (delete the polling
fallback), **T28** (sidecar WS for live agent state), **T29** (replace
background polling with `notify`-based watching).

## Items

### 68. [DONE] Standalone TUI must subscribe to StateHub unconditionally

**Resolved in**: `crates/roko-cli/src/tui/app.rs` now spawns a private in-process hub
via `roko_core::SharedStateHub::new_in_process()` (lines ~419, ~427) when no external hub
is provided. The `snapshot_rx` is always set (line ~522: `app.snapshot_rx = Some(snapshot_rx)`).
The old polling fallback is removed; the app now always receives push updates via
`tokio::sync::watch::Receiver<DashboardSnapshot>` (line ~114). Additionally,
`fs_watch::watch_roko_dir_with_fallback()` (line ~602) provides a debounced `notify::Watcher`
over `.roko/` with a polling fallback, replacing the old 500ms sleep loop.

**Status**: DONE.

---

### 69. [DONE] 500 ms file-polling thread is a bug

**Resolved in**: `crates/roko-cli/src/tui/fs_watch.rs` implements a debounced
`notify::Watcher` (200ms debounce window, line ~19) over `.roko/` with a bounded
`SyncSender<FsRefresh>` channel (bound=4, line ~21). The `FsWatchBackend` enum (line ~40)
supports both `Notify` (native fs events) and `Poll` (1-second fallback if notify fails).
`app.rs` line ~602 starts the watcher; line ~2484-2487 drains refresh events. The old
500ms sleep-loop polling thread has been removed.

**Status**: DONE.

---

### 70. Agent status read from `executor.json` instead of live `/stream` WS

**Evidence**: `crates/roko-cli/src/tui/views/agents_view.rs` builds agent rows
from `app.data.agents` which is populated by
`crates/roko-cli/src/tui/dashboard.rs:421` via `load_agents(&state)` — `state`
is the parsed `executor.json` snapshot, not a live agent feed.

**Current state**: Status fields lag the agent's real activity by the polling
interval (worst case 500 ms + write latency).

**Gap**: Subscribe to the per-agent `roko-agent-server` `/stream` WebSocket
when the Agents tab is active; render the live tail.

**Fix scope**: 2 days. Cross-ref T28.

**Priority**: P1.

---

### 71. Gate verdicts derived from signal file on every refresh

**Evidence**: `crates/roko-cli/src/tui/dashboard.rs:414-422`:
```rust
let (recent_signals, gate_signal_summaries, signal_gate_results, signals_state) =
    load_signal_state(&signals_path);
…
let gate_results = load_gate_results(&state, &signal_gate_results);
```

**Current state**: Every refresh re-parses `.roko/signals.jsonl` plus the
executor state to derive gate results.

**Gap**: Convert to incremental tail-reading using a stored offset (or move
to push-via-StateHub when the signal substrate emits a new gate verdict).

**Fix scope**: 1 day. Cross-ref T29.

**Priority**: P1.

---

### 72. Task output tails polled from `.roko/task-outputs/`

**Evidence**: `crates/roko-cli/src/tui/dashboard.rs:448-451`:
```rust
let task_outputs_dir = roko_dir.join("task-outputs");
let task_outputs = load_task_outputs(&task_outputs_dir);
let task_outputs_stamp = file_stamp(&task_outputs_dir);
```

**Current state**: The task-output directory is walked + per-file tailed on
each refresh.

**Gap**: Watch the directory with `notify`; only re-tail files that changed.

**Fix scope**: 1 day. Cross-ref T29.

**Priority**: P1.

---

### 73. Episode log polled from disk

**Evidence**: `crates/roko-cli/src/tui/dashboard.rs:559-563`:
```rust
let stamp = file_stamp(&episodes_path);
if stamp != self.episodes_state.stamp {
    self.refresh_episodes(&episodes_path, stamp);
    generation_changed = true;
}
```

**Current state**: Episode log re-loaded whenever the file mtime changes —
detected only when the polling refresh fires.

**Gap**: Watch `.roko/episodes.jsonl` directly; on append, parse only the new
lines using a stored offset.

**Fix scope**: 1 day. Cross-ref T29.

**Priority**: P1.

---

### 74. Event log polled from `.roko/state/events.json`

**Evidence**: `crates/roko-cli/src/tui/dashboard.rs:460-463`:
```rust
let events_path = roko_dir.join("state").join("events.json");
let event_log = load_event_log(&events_path);
let event_log_stamp = file_stamp(&events_path);
```

**Current state**: Same shape as item 73; whole file re-parsed on stamp change.

**Gap**: Either subscribe to `roko-runtime::event_bus` directly or watch +
incrementally tail.

**Fix scope**: 1 day.

**Priority**: P1.

---

### 75. [DONE] Git view polls git CLI every 3 s

**Resolved in**: `crates/roko-cli/src/tui/git_watch.rs` implements a debounced
`notify::Watcher` over git admin paths (.git/HEAD, .git/refs/) instead of polling
`git` every 3 seconds. Uses the same pattern as `fs_watch.rs`: `GitWatchHandle` with
`GitRefresh::Coalesced` signals, a `notify`-based backend with a metadata-poll fallback
(line ~176). Started at `app.rs` line ~608; drained at lines ~2500-2503.

**Status**: DONE.

---

### 76. Learning data (efficiency / cascade / experiments) polled from `.roko/learn/*`

**Evidence**: `crates/roko-cli/src/tui/dashboard.rs:565-586` — efficiency,
experiments, gate-thresholds, cascade-router, and c-factor files all re-read
when their stamp changes.

**Current state**: Same polling shape; same fix.

**Gap**: Single `notify::Watcher` over `.roko/learn/` covers all four files.

**Fix scope**: Folded into T29.

**Priority**: P1.

---

### 77. [DONE] Unbounded `std::sync::mpsc::Sender` on background thread has no backpressure

**Resolved in**: The old unbounded `std::sync::mpsc::channel` for dashboard data has been
replaced. The snapshot delivery now uses `tokio::sync::watch::channel` (app.rs line ~580:
`let (sys_tx, sys_rx) = watch::channel(SysSnapshot::default())`) which is single-slot and
always holds only the latest value, providing natural backpressure. The `fs_watch.rs` watcher
also uses a bounded `std::sync::mpsc::sync_channel` (bound=4, line ~21).

**Status**: DONE.

---

### 78. Generation counter in `OnceLock<HashMap<PathBuf, ...>>` not durable across restarts

**Evidence**: `crates/roko-cli/src/tui/dashboard.rs:104-110`:
```rust
struct DashboardGenerationState { fingerprint: u64, generation: u64 }
static DASHBOARD_DATA_GENERATIONS: OnceLock<Mutex<HashMap<PathBuf, DashboardGenerationState>>> =
    OnceLock::new();
```

**Current state**: The dashboard generation counter resets to 0 on every
process restart. Anything that uses generation as a "newer than" sentinel
breaks across restarts.

**Gap**: Persist `(fingerprint, generation)` to `.roko/state/dashboard-gen.json`
on update; load on startup.

**Fix scope**: 4 hours. Single struct + atomic write.

**Priority**: P2 (only matters if cross-process / multi-tab consumers exist).
