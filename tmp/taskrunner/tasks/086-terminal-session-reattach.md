# Task 086: Terminal Session Persistence + Reattach

```toml
id = 86
title = "PTY session grace period, scrollback persist, reconnect reattach, and exponential backoff WS retry"
track = "terminal-chat"
wave = "wave-2"
priority = "high"
blocked_by = [53]
touches = [
    "crates/roko-serve/src/terminal.rs",
    "crates/roko-serve/src/state.rs",
    "demo/demo-app/src/hooks/useTerminal.ts",
]
exclusive_files = ["crates/roko-serve/src/terminal.rs"]
estimated_minutes = 360
```

## Context

S3 (infrastructure-audit.md §3) + redesign-plan.md Phase 5.2.

Three compounding problems destroy terminal state on every page refresh or brief
network blip:

1. **No grace period** — `ws_terminal()` in `crates/roko-serve/src/terminal.rs:616`
   calls `destroy_session(&id)` before creating a new one on every WebSocket
   connection, even when the client is just reconnecting after a transient disconnect.
   The PTY child is killed instantly.

2. **Client generates a new ID every mount** — `useTerminal` in
   `demo/demo-app/src/hooks/useTerminal.ts:80` computes
   `` `t${Date.now().toString(36)}-${Math.random()...}` `` on every mount, so even
   if the server kept the session alive the client connects to a brand-new one.

3. **Fixed 500ms reconnect, no ceiling, no max retries** — `ws.onclose` schedules
   `setTimeout(connectWs, 500)` with no backoff and no limit. A downed server causes
   an indefinite reconnect storm (audit: "browser hammers it with reconnect attempts").

4. **Session generation counter resets** — `AtomicU64` in `SessionManager` starts at
   0 on every server restart. No server-lifetime discriminator exists.

5. **ZDOTDIR not cleaned up** — `PtySession.zdotdir` temp dir is removed in
   `finish_session()` only when the destroy path is taken cleanly. The grace-period
   reap path (not yet built) must also clean it up.

## Background

Read these files before starting:

1. `crates/roko-serve/src/terminal.rs` — `PtySession` struct (lines 37-48),
   `SessionManager` (lines 249-547), `ws_terminal()` route (lines 611-633),
   `handle_ws()` (lines 635-701). The destroy-on-connect pattern is at line 617.
   `finish_session()` at line 509 already removes ZDOTDIR.

2. `crates/roko-serve/src/state.rs` — `AppState` field `terminal_sessions:
   crate::terminal::SessionManager` at line 442. Check how state is initialized
   (lines 599-600) and what fields are available for workspace-path persistence.

3. `demo/demo-app/src/hooks/useTerminal.ts` — the full hook. Key points:
   - `id` generation at line 80 (fresh every mount)
   - `connectWs()` at line 346 (WS URL, open/close/error handlers)
   - Fixed 500ms reconnect at line 416
   - `ws.onopen` prompt detection async block at lines 373-387

4. Workspace persistence (task 053) — once task 53 lands, workspace IDs are stable
   across page loads. The terminal hook will receive `sessionId = workspace.id` from
   callers. That is the contract this task depends on.

## What to Change

### 1. Server — PTY grace period (terminal.rs)

Add `disconnected_at: Option<std::time::Instant>` to `PtySession`:

```rust
pub(crate) struct PtySession {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send>,
    sess_generation: u64,
    zdotdir: Option<std::path::PathBuf>,
    disconnected_at: Option<std::time::Instant>,   // NEW
    scrollback: std::collections::VecDeque<Vec<u8>>, // NEW
}
```

Change `ws_terminal()` so it does NOT destroy the existing session on connect.
Instead:

- If a session with `id` already exists and is in the grace period
  (`disconnected_at.is_some()` and elapsed < 60s), reattach: clear
  `disconnected_at`, drain `scrollback`, send it to the WS client as binary frames
  before the live stream, then resume.
- If a session with `id` already exists but the grace period has expired, destroy it
  and create a new one.
- If no session with `id` exists, create a new one (existing path).

Add `SessionManager::mark_disconnected(id)` — called when the WebSocket loop exits.
Sets `disconnected_at = Some(Instant::now())` instead of calling `destroy_session`.

Add `SessionManager::reap_expired(&self)` — removes sessions where
`disconnected_at.elapsed() > 60s`. Call it lazily at the start of
`create_session_inner` and from `list_sessions`. No background thread needed.

### 2. Server — scrollback ring buffer (terminal.rs)

In the PTY reader thread, before sending each chunk to `pty_tx`, push it to
`session.scrollback` (via a separate `Arc<Mutex<VecDeque<Vec<u8>>>>` or by locking
`sessions` briefly). Cap at 512 chunks. On reattach, drain and send the scrollback
as binary frames before the live reader starts.

Keep the scrollback append lightweight — the PTY reader thread must not block.
Use a separate `Arc<Mutex<VecDeque<Vec<u8>>>>` per session, not the global sessions
lock, to avoid contention.

### 3. Server — persist CWD + env snapshot (terminal.rs + state.rs)

On `mark_disconnected`, snapshot CWD and a minimal env subset to
`.roko/workspaces/{workspace_id}/terminal.state` (JSON). This supports full session
loss recovery after a server restart (tmux is not required; this is the fallback path
from the audit).

```json
{
  "session_id": "...",
  "workspace_id": "...",
  "cwd": "/home/user/project",
  "scrollback_lines": 42,
  "disconnected_at_unix": 1746600000
}
```

On server restart, if a WS connects with a known `id` but no live session, attempt to
restore CWD from the state file and replay scrollback (if still within 60s of the
`disconnected_at_unix` timestamp from the file). If the state file is stale (>60s),
create a fresh session but `cd` the new PTY into the saved CWD.

### 4. Client — stable session ID per workspace (useTerminal.ts)

The `sessionId` prop is already optional. When `sessionId` is provided, use it
directly instead of generating a fresh ID:

```ts
const id = sessionId ?? `t${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 5)}`;
```

This one-liner is already correct — the change is that callers (demo scenario slots,
workspace panels) must pass `sessionId={workspace.id}` so page-refresh reconnects to
the same server-side PTY. Do not change the default random-ID path.

After a successful reconnect where `sessionId` was caller-supplied and
`reconnectAttempts > 0`, write a brief status line so the user knows:

```ts
term.write('\r\n\x1b[38;5;132m[roko] Reconnected — replaying scrollback...\x1b[0m\r\n');
```

### 5. Client — exponential backoff with max retries (useTerminal.ts)

Replace the fixed 500ms reconnect with exponential backoff:

```ts
const RECONNECT = { base: 500, max: 30_000, factor: 2, maxAttempts: 20 };
let reconnectAttempts = 0;

function scheduleReconnect() {
    if (reconnectAttempts >= RECONNECT.maxAttempts) {
        // surface "Server unreachable" in UI — do not schedule further reconnects
        if (!disposed) setStatus('disconnected');
        return;
    }
    const delay = Math.min(RECONNECT.base * RECONNECT.factor ** reconnectAttempts, RECONNECT.max);
    reconnectAttempts++;
    reconnectTimer = setTimeout(connectWs, delay + Math.random() * 200);
}
```

On `ws.onopen`, reset `reconnectAttempts = 0`.

After `maxAttempts` failures, set status to `'disconnected'` and stop scheduling.
Add `retryNow(): void` to `TerminalHandle` — resets counter and calls `connectWs()`.
The UI can render a "Server unreachable — click to retry" button using this function.

### 6. Client — cleanup on unmount (useTerminal.ts)

Verify the existing cleanup block (line ~445) cancels `reconnectTimer` on unmount.
It already does — confirm and leave it unchanged.

## What NOT to Do

- Do NOT introduce tmux as a hard dependency. The redesign-plan tmux backend (5.2b)
  is an optional enhancement for later. This task implements the in-process fallback
  path (5.2a) only.
- Do NOT change the REST session API shape (`POST /api/terminal/sessions`).
- Do NOT change the WebSocket frame format (binary PTY data, JSON resize messages).
- Do NOT change `demo/demo-app/src/lib/terminal-session.ts` — it wraps `useTerminal`
  and does not need changes for reconnect.
- Do NOT add a new persistent database. State file in `.roko/workspaces/{id}/` is
  sufficient; it is JSON on disk.
- Do NOT change PTY spawn logic (shell command, ZDOTDIR setup, env).

## Wire Target

```bash
# Start serve
cargo run -p roko-cli -- serve &

# Open demo app
cd demo/demo-app && npm run dev
# Navigate to a scenario with a terminal. Pass workspace.id as sessionId.
# In browser devtools Network tab: close the WS connection manually.
# → useTerminal should reconnect with backoff, scrollback should replay.

# Page refresh:
# → terminal reconnects to same PTY (within 60s grace), scrollback visible.

# Kill serve, wait 65s, restart:
# → new PTY created, CWD restored from state file.

# Kill serve, hammer refresh 21 times:
# → after 20 attempts, UI shows "Server unreachable", no further reconnects.
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cd demo/demo-app && npx tsc --noEmit`
- [ ] Page refresh reconnects to existing PTY within 60s grace, scrollback replays
- [ ] After 60s+ of disconnect, session reaped; fresh PTY created, CWD restored
- [ ] ZDOTDIR temp dir removed on both immediate-destroy and grace-period-reap paths
- [ ] WS reconnect uses exponential backoff (500 → 1000 → 2000 → ... → 30000)
- [ ] After 20 failed reconnects, status is `'disconnected'`, no further attempts
- [ ] `retryNow()` on `TerminalHandle` resets counter and triggers fresh attempt
- [ ] `terminal.state` written to `.roko/workspaces/{id}/` on disconnect
- [ ] No `destroy_session` call on WebSocket connect when live session exists

## Worker 17 Mechanical Notes

### Current runtime call chain to preserve

- Server route: `crates/roko-serve/src/terminal.rs::routes()` registers
  `GET /ws/terminal/{id}` -> `ws_terminal()` -> `handle_ws()`.
- Current broken server behavior: `ws_terminal()` unconditionally calls
  `state.terminal_sessions.destroy_session(&id)` before
  `create_session_with_id(...)`; `handle_ws()` calls
  `destroy_session_if_sess_generation(...)` when the WebSocket loop exits.
- Session creation path: `SessionManager::new(workdir)` is constructed from
  `AppState::new_with_daimon_strategy_and_state_hub()` in
  `crates/roko-serve/src/state.rs`; `create_session_inner()` spawns the PTY,
  stores `PtySession`, and emits `CommandEvent::Started`.
- Client path: `TerminalPaneWithHandle` and `BottomTerminalPane` pass a
  `sessionId` prop into `useTerminal(sessionId)`. `useTerminal()` only
  generates a random ID when the prop is absent; it still reconnects with a
  fixed `setTimeout(connectWs, 500)` in `ws.onclose`.

### Mechanical server design

The current `handle_ws()` owns the PTY reader thread per WebSocket connection.
That is not enough for reattach: once the socket closes, the reader thread
stops and any output produced during the grace period is lost. Implement this
as a session-owned output pump instead:

1. Add constants in `terminal.rs` near the type definitions:
   `TERMINAL_GRACE_PERIOD: Duration = Duration::from_secs(60)` and
   `SCROLLBACK_CHUNKS: usize = 512`.
2. Extend `PtySession` with `disconnected_at`, `scrollback:
   Arc<Mutex<VecDeque<Vec<u8>>>>`, `subscribers:
   Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>>`, and enough spawn metadata to
   persist/restore state (`workdir`/cwd fallback, cols, rows, optional command).
3. Start the PTY reader thread in `create_session_inner()` immediately after
   cloning the reader. The thread should append every chunk to the per-session
   `scrollback` ring and `try_send`/`blocking_send` it to current subscribers;
   prune closed subscriber channels. Do not hold the global `sessions` lock in
   this thread.
4. Add a method such as `SessionManager::attach_session(id) ->
   AttachResult` that reaps expired sessions, decides whether to reuse or
   create, clears `disconnected_at` on reuse, returns the session generation,
   a receiver subscribed to live chunks, and a cloned scrollback snapshot to
   send before live data.
5. Change `ws_terminal()` to call `attach_session()` instead of
   `destroy_session()` + `create_session_with_id()`. Send scrollback binary
   frames before entering the live bridge loop.
6. Change `handle_ws()` so socket exit calls `mark_disconnected(id, gen)`, not
   `destroy_session_if_sess_generation()`. Keep `destroy_session*` for REST
   delete and expired reap only.
7. Implement `reap_expired()` by collecting expired IDs under the map lock,
   removing them, then calling `finish_session()` outside the map lock. This
   path must remove ZDOTDIR via the existing `finish_session()`.

### Persistence specifics

- Use `SessionManager.workdir.join(".roko/workspaces").join(id).join("terminal.state")`
  for the JSON state file unless task 053 introduced a stronger workspace API.
  Create parent dirs with `std::fs::create_dir_all`.
- `PtySession` cannot currently observe the shell's live CWD after arbitrary
  `cd` commands without protocol or shell instrumentation. Do not pretend it
  can. Persist the spawn workdir as the fallback CWD unless a separate task
  expands the terminal protocol to report CWD.
- On connect with no live session and an existing `terminal.state`, restore
  `cwd` when spawning the replacement PTY. Replay persisted scrollback only if
  the file's `disconnected_at_unix` is within the 60s grace window; otherwise
  create a new PTY in the saved CWD without replaying stale bytes.

### Mechanical client steps

- Import `RECONNECT_BACKOFF` and `TIMEOUTS` from `src/lib/serve-url.ts`; the
  constants already exist on the current branch.
- Add `retryNow(): void` to `TerminalHandle` and implement it on the handle
  object before assigning `handleRef.current`.
- Track `reconnectAttempts` and `reconnectTimer` inside the effect. On
  `ws.onopen`, write the reconnect status line only when `sessionId` was
  caller-supplied and attempts were nonzero, then reset the counter.
- Replace the fixed close handler with `scheduleReconnect()` using
  `RECONNECT_BACKOFF.baseMs`, `factor`, `maxMs`, `maxAttempts`, plus small
  jitter. After max attempts, set handle/status to `'disconnected'` and stop.
- Keep the existing unmount cleanup that clears `reconnectTimer`,
  `resizeTimer`, disposes xterm resources, and closes `handle.ws`.

### Tests to add or update

- In `terminal.rs` tests, add a unit test that creates a session with a stable
  ID, marks it disconnected, reattaches within the grace period, and verifies
  the generation/session is reused and scrollback is returned.
- Add a reap test with an artificially old `disconnected_at` that verifies the
  session is removed and ZDOTDIR is cleaned through `finish_session()`.
- Add a state-file test using `tempfile` that writes/reads
  `.roko/workspaces/<id>/terminal.state` and verifies stale vs fresh behavior.
- In the frontend, add or update hook tests if the demo test stack has them;
  otherwise rely on `npx tsc --noEmit` and manual browser verification.

### Known ambiguity for the implementation agent

Current demo scenario panes still create IDs like
`demo-${scenario.id}-${i}-${Date.now()}` in
`demo/demo-app/src/pages/Demo/ScenarioSlot.tsx`, so a full page refresh still
changes IDs. That caller file is not in this task's `touches` list. Do not edit
it silently; either get the touch list expanded or document that only callers
already passing stable IDs get page-refresh reattach.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
