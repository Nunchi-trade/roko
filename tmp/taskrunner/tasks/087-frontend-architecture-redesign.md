# Task 087: Frontend Architecture Redesign

```toml
id = 87
title = "Replace polling loops, fix stale closures, add cleanup, centralize config, fix model slug matching"
track = "demo-ui"
wave = "wave-2"
priority = "high"
blocked_by = [13, 21]
touches = [
    "demo/demo-app/src/lib/serve-url.ts",
    "demo/demo-app/src/hooks/useWorkspace.ts",
    "demo/demo-app/src/components/ErrorBoundary.tsx",
    "demo/demo-app/src/lib/prd-pipeline-types.ts",
    "demo/demo-app/src/contexts/EventStreamContext.tsx",
    "demo/demo-app/src/hooks/useServerHealth.ts",
    "demo/demo-app/src/hooks/useLiveApi.ts",
    "demo/demo-app/src/hooks/useRokoConfig.ts",
    "demo/demo-app/src/lib/palette.ts",
]
exclusive_files = [
    "demo/demo-app/src/lib/serve-url.ts",
    "demo/demo-app/src/components/ErrorBoundary.tsx",
]
estimated_minutes = 360
```

## Context

S7.1-7.6 (infrastructure-audit.md §7) + redesign-plan.md Phase 7. This is the
architectural foundation that scenario redesign (task 021) builds on. Blocked by
task 013 (SSE keepalive + replay bound) because several of the SSE subscriptions
this task adds depend on server-side event delivery being reliable.

Six compounding anti-patterns across the demo frontend:

**S7.1 Polling loops** — `useServerHealth` calls `setInterval(check, 3000)` to poll
`/health`. `EventStreamContext.tsx` polls its own connected state with a 500ms
`setInterval`. `useLiveApi.ts` has a module-level probe singleton (`probeServer`,
`_healthListeners`) that duplicates health checking. All of these should use SSE
push or a single event-driven source.

**S7.2 Stale closures** — `useWorkspace.ts` references `serverWorkdir` inside
`createWorkspace`/`ensureWorkspace` callbacks without including it in deps or
wrapping it in a ref. `useTerminal.ts` captures `id` by closure in `connectWs`.
Scenario slot handlers call `setState` inside `useCallback` with empty dep arrays.

**S7.3 Missing cleanup** — `useServerHealth.ts` returns a `cancelledRef` guard but
the `setInterval` itself is cleared correctly (line 27: `clearInterval(id)`). The
`EventStreamContext.tsx` `setInterval` at line 31 IS cleaned up (line 36). The
pattern is mostly OK but needs audit of `useBench.ts`, `useBenchRuns.ts`, and
`hooks/useBlockStream.ts` to confirm every `setInterval`/`EventSource`/`WebSocket`
has a matching cleanup.

**S7.4 Hardcoded URLs outside config** — `demo/demo-app/src/lib/serve-url.ts` is
the canonical source for `SERVE_URL`, `WS_BASE`, `ABSOLUTE_SERVE_URL`,
`MIRAGE_WS_URL`, `MIRAGE_EVENTS_WS_URL`. Magic timeout numbers (5000, 2000, 500,
3000, 8000) are scattered across individual hooks and components with no single
source of truth.

**S7.5 No Error Boundaries around major sections** — `ErrorBoundary.tsx` exists
already at `demo/demo-app/src/components/ErrorBoundary.tsx` with a full
implementation. It is not yet applied to the terminal pane, scenario slot sidebar,
or dashboard route — a single failed fetch anywhere in those trees crashes the
entire page to a white screen.

**S7.6 Hardcoded model slug matching** — `prd-pipeline-types.ts:151-153` uses
`model.includes('opus')`, `model.includes('sonnet')` substring matching for
`inferRouteTier`. `palette.ts` `MODEL_COLORS` uses exact keys already but callers
may use `model.includes(k)` on the key string instead of on the model name. Remove
`includes`-based matching in favor of `startsWith` prefix matching on the model ID.

## Background

Read these files before starting:

1. `demo/demo-app/src/hooks/useServerHealth.ts` — 31 lines; `setInterval(check,
   intervalMs)` at line 26. The interval IS cleaned up at line 27. The fix is to
   replace the interval with an SSE subscription and keep `checkNow()` as a
   single one-shot fetch.

2. `demo/demo-app/src/contexts/EventStreamContext.tsx` — 60 lines; `setInterval`
   at line 31 polls `mgr.connected` state. Fix: subscribe to EventStreamManager's
   `onopen`/`onerror` callbacks directly and call `setConnected` from those instead
   of polling.

3. `demo/demo-app/src/hooks/useLiveApi.ts` — module-level `_serverLive` /
   `probeServer()` / `_healthListeners` singleton (lines 14-43). Already deprecated
   in a `@deprecated` comment at line 7. This polling singleton can be removed once
   all callers of `useServerLive()` / `useServerStatus()` from this file are migrated
   to `bootstrapTransport()` / `useServerConnected()` from `src/data/selectors.ts`.
   Do NOT remove the exported functions yet — audit callers first.

4. `demo/demo-app/src/hooks/useWorkspace.ts` — `serverWorkdir` state is referenced
   inside `createWorkspace` / `ensureWorkspace` callbacks. Wrap in a ref.

5. `demo/demo-app/src/lib/serve-url.ts` — the canonical URL source. Do NOT
   change its exports (`SERVE_URL`, `WS_BASE`, `ABSOLUTE_SERVE_URL`,
   `MIRAGE_WS_URL`, `MIRAGE_EVENTS_WS_URL`). This file stays as-is. Add a
   `RECONNECT_BACKOFF` and `TIMEOUTS` export here (see §1 below).

6. `demo/demo-app/src/lib/prd-pipeline-types.ts:149-153` — `inferRouteTier`
   function. The `model.includes('opus')` calls at lines 151-153.

7. `demo/demo-app/src/lib/palette.ts` — `MODEL_COLORS` record and `roleColor`
   function. Check callers of `MODEL_COLORS` in components for inline
   `model.includes(k)` lookups.

8. `demo/demo-app/src/components/ErrorBoundary.tsx` — already implemented with
   animated fallback UI. Check where it should be mounted.

9. `demo/demo-app/src/hooks/useBench.ts`, `useBenchRuns.ts`,
   `hooks/useBlockStream.ts` — audit for missing cleanup.

## What to Change

### 1. Central timeout + reconnect config (serve-url.ts)

Add two const exports to `demo/demo-app/src/lib/serve-url.ts`. Do NOT move or
rename existing exports. Add at the bottom of the file:

```ts
/** Canonical timeout values (ms) used across all hooks and session logic. */
export const TIMEOUTS = {
  health:       parseInt(viteEnv('VITE_TIMEOUT_HEALTH')    ?? '5000',  10),
  workspace:    parseInt(viteEnv('VITE_TIMEOUT_WORKSPACE') ?? '10000', 10),
  command:      parseInt(viteEnv('VITE_TIMEOUT_COMMAND')   ?? '180000',10),
  terminal:     parseInt(viteEnv('VITE_TIMEOUT_TERMINAL')  ?? '8000',  10),
  shellPrompt:  parseInt(viteEnv('VITE_TIMEOUT_PROMPT')    ?? '8000',  10),
} as const;

/** WebSocket reconnect backoff parameters used by useTerminal and WS transports. */
export const RECONNECT_BACKOFF = {
  baseMs:      parseInt(viteEnv('VITE_RECONNECT_BASE_MS')      ?? '500',   10),
  maxMs:       parseInt(viteEnv('VITE_RECONNECT_MAX_MS')       ?? '30000', 10),
  factor:      parseFloat(viteEnv('VITE_RECONNECT_FACTOR')     ?? '2'),
  maxAttempts: parseInt(viteEnv('VITE_RECONNECT_MAX_ATTEMPTS') ?? '20',    10),
} as const;
```

Replace all magic timeout literals in the hooks listed in §2-5 below with
`TIMEOUTS.health`, `TIMEOUTS.terminal`, etc. Do not change literals inside test
files or Storybook stories.

### 2. Replace setInterval polling in EventStreamContext (EventStreamContext.tsx)

The `setInterval` at line 31 polls `mgr.connected`. Replace with direct callback:

```ts
useEffect(() => {
    const mgr = createEventStreamManager(SERVE_URL);
    managerRef.current = mgr;
    mgr.onConnectedChange = (c: boolean) => setConnected(c);
    return () => {
        mgr.destroy();
        managerRef.current = null;
    };
}, []);
```

If `EventStreamManager` from `hooks/useEventStream.ts` does not expose an
`onConnectedChange` callback, add one. Alternatively, expose `onopen` and `onerror`
hooks that call `setConnected(true)` / `setConnected(false)` respectively. The
`setInterval` must be removed.

### 3. Replace setInterval in useServerHealth (useServerHealth.ts)

Rewrite to subscribe to the `EventStreamContext` for a `server:health` or
`health` event type. Fall back to a single fetch on mount for initial state.
Remove the `setInterval`. Keep the `checkNow()` function as a manual one-shot fetch:

```ts
export function useServerHealth() {
    const [status, setStatus] = useState<ServerStatus>('checking');
    const { subscribe } = useEventStreamContext();

    useEffect(() => {
        // Initial fetch
        const ac = new AbortController();
        fetch(`${SERVE_URL}/health`, { signal: AbortSignal.timeout(TIMEOUTS.health) })
            .then(r => setStatus(r.ok ? 'connected' : 'disconnected'))
            .catch(() => setStatus('disconnected'));

        // SSE subscription for push updates
        const unsub = subscribe(['server:health', 'health'], (event) => {
            const ok = (event as any)?.ok ?? (event as any)?.status === 'ok';
            setStatus(ok ? 'connected' : 'disconnected');
        });

        // Fallback: if no SSE health event within 10s, do a single re-check
        const fallback = setTimeout(() => {
            fetch(`${SERVE_URL}/health`, { signal: AbortSignal.timeout(TIMEOUTS.health) })
                .then(r => setStatus(r.ok ? 'connected' : 'disconnected'))
                .catch(() => setStatus('disconnected'));
        }, 10_000);

        return () => {
            ac.abort();
            unsub();
            clearTimeout(fallback);
        };
    }, [subscribe]);

    const checkNow = useCallback(async () => {
        try {
            const r = await fetch(`${SERVE_URL}/health`, { signal: AbortSignal.timeout(TIMEOUTS.health) });
            setStatus(r.ok ? 'connected' : 'disconnected');
        } catch {
            setStatus('disconnected');
        }
    }, []);

    return { status, checkNow };
}
```

The hook signature stays identical (`intervalMs` parameter is removed — it was the
only caller-facing surface). Check all callers of `useServerHealth` to confirm they
don't pass `intervalMs`; if any do, remove the argument.

### 4. Fix stale closures — useRef pattern (useWorkspace.ts)

In `useWorkspace.ts`, `serverWorkdir` state is referenced inside `createWorkspace`
and `ensureWorkspace` callbacks. Wrap it in a ref:

```ts
const serverWorkdirRef = useRef(serverWorkdir);
useEffect(() => { serverWorkdirRef.current = serverWorkdir; }, [serverWorkdir]);
```

Replace all reads of `serverWorkdir` inside callbacks with `serverWorkdirRef.current`.

Apply the same ref pattern to any other mutable state accessed inside `useCallback`
or `useEffect` closures in this file.

### 5. Ensure cleanup in every useEffect

Audit every `useEffect` in these files that creates any of: `setInterval`,
`setTimeout`, `EventSource`, `WebSocket`, `EventStreamContext` subscription,
`AbortController`. Every one must return a cleanup function.

Files to audit:
- `hooks/useBench.ts` — confirm any `setInterval` for bench polling is cleared
- `hooks/useBenchRuns.ts` — confirm `AbortController` / interval cleanup
- `hooks/useBlockStream.ts` — confirm `EventSource` / WS cleanup
- `contexts/EventStreamContext.tsx` — after §2 above, verify no remaining intervals

Do not restructure hooks that already have correct cleanup. Only add missing
`clearInterval` / `clearTimeout` / `.close()` / `.abort()` calls.

### 6. Fix model tier inference (prd-pipeline-types.ts)

Replace `model.includes(...)` substring matching in `inferRouteTier`
(lines 149-153) with a prefix-match map:

```ts
const MODEL_TIER_MAP: Record<string, PipelineRouteTier> = {
  'claude-opus':    'T3',
  'claude-sonnet':  'T2',
  'claude-haiku':   'T1',
  'gpt-5.4':        'T3',
  'gpt-5.4-mini':   'T2',
  'gemini-2.5-pro': 'T3',
  'gemini-2.5':     'T2',
  'codex':          'T2',
};

// Inside inferRouteTier, replace the three model.includes lines with:
const modelKey = Object.keys(MODEL_TIER_MAP).find(k => model.startsWith(k));
if (modelKey) return MODEL_TIER_MAP[modelKey];
```

Preserve the `roleText` and `maxLoc`/`verifyCount` fallbacks below. The explicit
`tier` check at the top of `inferRouteTier` stays as-is.

### 7. Add modelColor helper (palette.ts)

`MODEL_COLORS` uses exact key strings (`'claude-sonnet-4'`, etc.). Add a
`modelColor` export that does prefix matching on the model ID, not on the key:

```ts
export function modelColor(model: string): string {
    // Exact match first
    if (model in MODEL_COLORS) return MODEL_COLORS[model];
    // Prefix match: model 'claude-sonnet-4-6' should match key 'claude-sonnet-4'
    const key = Object.keys(MODEL_COLORS).find(k => model.startsWith(k));
    return MODEL_COLORS[key ?? ''] ?? '#706070';
}
```

Find all callsites that read `MODEL_COLORS` directly and do any form of
`model.includes(k)` substring logic. Replace them with `modelColor(model)`.
Check: `components/CostRace.tsx`, any bench/evaluate components, pages that import
`MODEL_COLORS`.

### 8. Apply ErrorBoundary to major sections

`ErrorBoundary.tsx` already exists at
`demo/demo-app/src/components/ErrorBoundary.tsx`. Apply it in the appropriate parent
layout files:

- Wrap the terminal pane wherever it is rendered (search for `useTerminal` or the
  terminal container component in `pages/Demo/` or `components/Terminal/`)
- Wrap the scenario slot sidebar
- Wrap the dashboard route (top-level `pages/Dashboard.tsx` or equivalent)

Import as:
```tsx
import ErrorBoundary from '../components/ErrorBoundary';
// or from wherever relative to the layout file

<ErrorBoundary>
    <TerminalPane ... />
</ErrorBoundary>
```

Do NOT wrap individual leaf components. Three boundaries maximum: terminal pane,
scenario sidebar, dashboard.

## What NOT to Do

- Do NOT remove `demo/demo-app/src/lib/serve-url.ts` or rename its existing exports.
  `SERVE_URL`, `WS_BASE`, `ABSOLUTE_SERVE_URL`, `MIRAGE_WS_URL`,
  `MIRAGE_EVENTS_WS_URL` stay exactly as-is.
- Do NOT add a separate `lib/config.ts` file. `serve-url.ts` IS the config file.
  Add `TIMEOUTS` and `RECONNECT_BACKOFF` there.
- Do NOT subscribe to SSE event types that do not yet exist on the server. Only
  subscribe to types that `roko-serve/src/events.rs` already emits. If a
  `server:health` type doesn't exist, the fallback timeout (§3) handles it.
- Do NOT remove the existing `EventStreamContext` — build on top of it.
- Do NOT add a new state management library (no Redux, Zustand, Jotai, etc.).
- Do NOT change `useTerminal.ts` — the reconnect backoff and session ID changes are
  covered by task 086.
- Do NOT rewrite working components — only fix the specific anti-patterns called out
  above.
- Do NOT remove deprecated hooks (`useLiveApi`, `useRokoConfig`) — audit callers
  first. Mark them deprecated if not already. Actual removal is a separate task.

## Wire Target

```bash
cd demo/demo-app

# TypeScript must pass clean
npx tsc --noEmit

# Build must succeed
npm run build

# Runtime checks (browser devtools):
# 1. Open Performance profiler → confirm no setInterval in useServerHealth,
#    EventStreamContext (polling interval replaced by callback)
# 2. Disconnect serve → useServerHealth transitions to 'disconnected' via SSE
#    fallback timeout within ~10s (not on a 3s polling cycle)
# 3. Crash a component tree artificially → ErrorBoundary renders instead of white screen

# Grep checks:
grep -rn 'model\.includes' demo/demo-app/src/lib/prd-pipeline-types.ts
# → no matches

grep -rn 'localhost:6677' demo/demo-app/src/ --include='*.ts' --include='*.tsx'
# → only serve-url.ts
```

## Verification

- [ ] `cd demo/demo-app && npx tsc --noEmit` — zero errors
- [ ] `cd demo/demo-app && npm run build` — succeeds
- [ ] `TIMEOUTS` and `RECONNECT_BACKOFF` exported from `serve-url.ts`
- [ ] No `setInterval` in `EventStreamContext.tsx` (replaced by `onConnectedChange`
      or `onopen`/`onerror` callbacks on `EventStreamManager`)
- [ ] No `setInterval` in `useServerHealth.ts` (replaced by SSE subscription +
      one-shot fallback timeout)
- [ ] Every `useEffect` in `useBench.ts`, `useBenchRuns.ts`, `useBlockStream.ts`
      that opens a connection or timer returns a cleanup function
- [ ] `serverWorkdirRef` pattern applied in `useWorkspace.ts`
- [ ] `inferRouteTier` uses `startsWith` prefix matching, not `model.includes()`
- [ ] `modelColor()` exported from `palette.ts`; no inline `model.includes(k)` on
      `MODEL_COLORS` keys in any caller
- [ ] `ErrorBoundary` wraps terminal pane, scenario sidebar, and dashboard route
- [ ] `grep -rn 'localhost:6677' demo/demo-app/src/ --include='*.ts'
      --include='*.tsx'` — only `serve-url.ts`

## Worker 17 Mechanical Notes

### Current branch snapshot

Several pieces from this task are already present in the current branch:

- `demo/demo-app/src/lib/serve-url.ts` already exports `TIMEOUTS` and
  `RECONNECT_BACKOFF`.
- `EventStreamContext.tsx` already uses `mgr.onConnectedChange`; the polling
  interval is gone. `hooks/useEventStream.ts` already defines
  `EventStreamManager.onConnectedChange`.
- `useServerHealth.ts` already uses initial `/health` fetch + SSE subscription
  + a single 10s fallback timeout. It no longer accepts an `intervalMs`
  argument.
- `useWorkspace.ts` already has `serverWorkdirRef`, but the ref is currently
  not read by the callbacks because `createWorkspace()` no longer uses
  `serverWorkdir`.
- `prd-pipeline-types.ts` already uses a `startsWith` tier map, and
  `palette.ts` already exports `modelColor()`.
- `main.tsx`, `pages/Terminal.tsx`, `pages/dashboard/Layout.tsx`, and several
  bench views already use `ErrorBoundary`/`ComponentErrorBoundary`.

Treat those as verification targets, not things to re-add.

### Remaining mechanical work

1. `useLiveApi.ts` still owns `_serverLive`, `_healthListeners`, `probeServer()`,
   and a 5s `setInterval`. Because many pages still call `useLiveApi()`, do not
   delete the exports. Replace the interval with state from the central data
   layer:
   - Prefer `useServerConnected()` / `useServerStatus()` from
     `src/data/selectors.ts` for `isLive`.
   - Keep `get/post/put` as thin wrappers around `useApi()`.
   - If a failed request should trigger a refresh, call the existing transport
     bootstrap action rather than starting another module-level poller.

2. `useRokoConfig.ts` still polls `/api/config` every 15s. Replace that with:
   - one initial `fetchConfig()` on mount, and
   - a subscription to the existing `config_reloaded` data event if this context
     remains mounted.
   The server already emits `config_reloaded` from
   `crates/roko-serve/src/routes/config.rs::reload_config_from_disk()`, and
   `demo/demo-app/src/app/DataHub.ts` already refreshes config on that event.
   If all consumers can be moved to `useConfigSlice()`, leave this provider as
   a compatibility shim with no polling interval.

3. `useBench.ts` and `useBenchRuns.ts` both keep a 3s active-run poll as an SSE
   safety net. They already clear `pollRef` on unmount and cancel. Do not remove
   this fallback unless there is a server-side replay guarantee for bench
   events; just verify the cleanup.

4. `useBlockStream.ts` has two modes:
   - direct Mirage WebSocket with reconnect timeout cleanup, and
   - `/api/chain/status` fallback using `setInterval`.
   The fallback interval is cleaned up, but its ref is typed as a timeout via a
   cast. If touching this file, split it into `pollIntervalRef` and
   `reconnectTimerRef` so `clearInterval`/`clearTimeout` are type-correct.

5. Error boundaries should be bounded to major surfaces. The current branch has
   route-level/dashboard/terminal boundaries; if adding more for the scenario
   sidebar, wrap the sidebar container in `ScenarioSlot.tsx`, not every command
   row or panel.

### Verification greps

Run these after changes in addition to the existing verification commands:

```bash
rg -n 'setInterval' demo/demo-app/src/contexts/EventStreamContext.tsx \
  demo/demo-app/src/hooks/useServerHealth.ts \
  demo/demo-app/src/hooks/useLiveApi.ts \
  demo/demo-app/src/hooks/useRokoConfig.ts
# Expected: no matches except comments if they explain removed legacy behavior.

rg -n 'model\\.includes|MODEL_COLORS.*includes|includes\\(k\\)' \
  demo/demo-app/src/lib demo/demo-app/src/components demo/demo-app/src/pages
# Expected: no MODEL_COLORS/model-tier substring matching. roleColor substring
# matching is allowed; it is role text, not model slug routing.

rg -n 'localhost:6677' demo/demo-app/src -g '*.ts' -g '*.tsx'
# Expected: only serve-url.ts.
```

### What not to rewrite

- Do not replace `EventStreamContext` with another state library.
- Do not remove `useLiveApi`, `useRokoConfig`, `WorkspaceProvider`, or
  `RokoConfigProvider` until every listed caller is migrated.
- Do not change `useTerminal.ts` in this task; terminal reconnect is task 086.
- Do not count animation timers, hover timers, or visual countdowns as S7.1
  polling bugs. This task is about server/API reachability and stream state.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
