# Task 093: Dead Subsystem Triage

```toml
id = 93
title = "Wire or delete three dead subsystems: context pressure watcher, Anthropic API tool loop, shell-prompt warning"
track = "cleanup"
wave = "wave-2"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-conductor/src/context_window_pressure.rs",
    "crates/roko-agent/src/provider/anthropic_api.rs",
    "demo/demo-app/src/hooks/useTerminal.ts",
]
exclusive_files = []
estimated_minutes = 180
```

## Context

Three subsystems compile and pass unit tests but have zero runtime effect. Each one requires a
deliberate decision — wire it, delete it, or gate-flag it — and that decision must be documented
in the code so future contributors do not re-investigate the same question.

The outcome of this task is not "all three subsystems are wired." The outcome is "all three have
a documented status with no ambiguity about why they exist or do not exist."

Sources:
- `tmp/infrastructure-audit.md` §18 (context window pressure: S18.1-S18.3), §22.6 (shell-prompt warning), §23.8 (Anthropic API tool loop)

## Background

Read these files before making any decision:

1. `crates/roko-conductor/src/context_window_pressure.rs` (full file) — The watcher fires on
   `Kind::TokenUsage` signals. `context_window_tokens()` (lines ~22-124) hardcodes only three
   Anthropic models (opus, sonnet, haiku) and returns `None` for all others. `decide()` (lines
   ~53-86) reads only the most recent signal, not a sliding window. The watcher emits
   `conductor.intervention` signals. Understand exactly what payload it sends.

2. `crates/roko-conductor/src/watchers/mod.rs` — Which watchers are registered in the conductor
   runtime. Check if `ContextWindowPressureWatcher` appears in the registration list.

3. `crates/roko-cli/src/orchestrate.rs` or `crates/roko-cli/src/runner/event_loop.rs` —
   Search for `conductor.intervention` subscribers. Confirm that nothing in the runtime reacts
   to intervention signals from the context window watcher.

4. `crates/roko-agent/src/provider/mod.rs` — `adapter_for_kind()`. Confirm that `"anthropic_api"`
   kind is in the dispatch table. Then check `roko.toml` (repo root and `docker/railway.roko.toml`)
   to confirm zero model entries point at `kind = "anthropic_api"`. The adapter exists in the
   dispatch table but is never reached because no model profile selects it.

5. `crates/roko-agent/src/provider/anthropic_api.rs` and (if it exists)
   `crates/roko-agent/src/provider/anthropic_api/tool_loop.rs` — Understand what
   `AnthropicApiAdapter` provides over `claude_cli`: streaming via Messages API, no subprocess
   overhead, concurrent request support, proper token usage reporting (unlike `claude_cli` which
   reports zero tokens — see §23.6).

6. `demo/demo-app/src/hooks/useTerminal.ts` line ~365 — The 8-second shell-prompt detection
   timeout. Confirm the warning currently goes to `console.warn` only and does not appear in the
   UI. Understand what the detection does (reads terminal output for a shell prompt pattern) and
   what user-visible surface would be appropriate for a failure.

7. `demo/demo-app/src/` — Find the toast notification system (if one exists) or the inline
   terminal panel component. This is where the shell-prompt warning should be routed.

## What to Change

For each subsystem, apply exactly one decision: Wire, Delete, or Gate-flag.

---

### Subsystem 1: Context Window Pressure Watcher

**Files**: `crates/roko-conductor/src/context_window_pressure.rs`,
`crates/roko-conductor/src/watchers/mod.rs`

**Problem summary**:
- `context_window_tokens()` only covers Anthropic model slugs — Gemini, Perplexity, GLM, and all
  other models return `None` and the watcher never fires for them.
- Even when it fires, nothing subscribes to `conductor.intervention` in the runner. The watcher
  emits a signal that no consumer ever reads.
- `decide()` checks only the most recent signal, making it noisy for alternating utilization.

**Decision: Gate-flag it.**

The implementation is structurally sound and worth keeping. The bugs make it wrong to enable by
default. Proceed as follows:

1. Extend `context_window_tokens()` to read from `ModelProfile.context_window` from the loaded
   config. When a `model_id` matches a configured model profile that has a `context_window` field
   set, return that value instead of the hardcoded table. The hardcoded Anthropic fallback table
   remains as a last resort for models not in config:

   ```rust
   pub fn context_window_tokens(model_id: &str, config: Option<&RokoConfig>) -> Option<u64> {
       // First: check config model profiles
       if let Some(cfg) = config {
           for (_, profile) in &cfg.models {
               if profile.slug.as_deref() == Some(model_id) {
                   if let Some(ctx) = profile.context_window {
                       return Some(ctx);
                   }
               }
           }
       }
       // Fallback: hardcoded Anthropic models
       // ... existing match ...
   }
   ```

2. Replace the single-signal check in `decide()` with a trailing minimum over the last N signals
   (use N=3 as a constant). This prevents alternating 85%/30% utilization from firing repeatedly:

   ```rust
   // Take the maximum utilization over the last N token-usage signals
   // to reduce noise from alternating utilization.
   const PRESSURE_LOOKBACK: usize = 3;
   ```

3. Do NOT wire the `conductor.intervention` subscriber in this task — that requires changes to
   the orchestrator which are out of scope. Instead, ensure the watcher is registered in
   `watchers/mod.rs` only when a config flag is set. Add a `conductor.context_pressure_enabled`
   boolean to the `[conductor]` section of `roko.toml` (default `false`). Register the watcher
   conditionally:

   ```rust
   if config.conductor.context_pressure_enabled.unwrap_or(false) {
       watchers.push(Box::new(ContextWindowPressureWatcher::new(/* ... */)));
   }
   ```

4. Add a `// STATUS: GATED` comment at the top of `context_window_pressure.rs`:

   ```rust
   // STATUS: GATED — only active when conductor.context_pressure_enabled = true in roko.toml.
   // Extended in 2026-05 to read context_window from ModelProfile config for non-Anthropic models.
   // Emits conductor.intervention signals; orchestrate.rs must subscribe to react.
   // Enable only after wiring a subscriber in the runner event loop.
   ```

---

### Subsystem 2: Anthropic API Tool Loop (Dead Code)

**Files**: `crates/roko-agent/src/provider/anthropic_api.rs` and
`crates/roko-agent/src/provider/anthropic_api/tool_loop.rs` (if it exists as a submodule)

**Problem summary**: All Claude models in `roko.toml` use `kind = "claude_cli"`. The
`AnthropicApiAdapter` is in the dispatch table in `adapter_for_kind()` but is never selected
because no model profile has `kind = "anthropic_api"`. The Messages API tool-loop code is
untested against real workloads.

Note that `claude_cli` has two significant bugs that `anthropic_api` would fix: it reports zero
tokens for all calls (§23.6), and `finish_reason` is always `None` (§23.7). The `anthropic_api`
adapter is the correct long-term path for Claude models.

**Decision: Add one wired test model entry and gate-flag the adapter.**

Do NOT delete the implementation. Proceed:

1. Add a commented-out example model profile to the inline default config in
   `crates/roko-core/src/config/` (or to `roko.toml` directly under a `# Anthropic Messages API`
   comment block). The example shows `kind = "anthropic_api"` with its advantages noted:

   ```toml
   # [providers.anthropic-api]
   # kind = "anthropic_api"
   # api_key_env = "ANTHROPIC_API_KEY"
   # # Advantages over claude_cli: streaming via Messages API, no subprocess overhead,
   # # proper token usage reporting, concurrent requests.
   #
   # [models.claude-sonnet-api]
   # provider = "anthropic-api"
   # slug = "claude-sonnet-4-6"
   ```

2. Add a unit integration test in `crates/roko-agent/tests/` (or `src/provider/anthropic_api.rs`)
   that constructs an `AnthropicApiAdapter` with a mock HTTP server and verifies that a basic
   tool-loop round trip parses correctly. The test must not require a live API key — use
   `wiremock` or a hand-rolled `tokio::spawn` HTTP listener returning a canned response.
   Gate the test behind `#[cfg(feature = "integration-tests")]` or a `#[ignore]` attribute.

3. Add `// STATUS: GATED` comments at the top of both files:

   ```rust
   // STATUS: GATED — AnthropicApiAdapter is reachable via kind = "anthropic_api" in roko.toml
   // but no default model profile uses this kind. To activate: add [providers.anthropic-api]
   // with kind = "anthropic_api" and ANTHROPIC_API_KEY. Advantages: proper token accounting
   // (claude_cli always reports zero tokens — §23.6), finish_reason detection (§23.7),
   // streaming, concurrent requests.
   ```

No other code changes. The adapter remains in the dispatch table exactly as it is.

---

### Subsystem 3: Shell Prompt Detection Warning

**File**: `demo/demo-app/src/hooks/useTerminal.ts` line ~365

**Problem summary**: An 8-second timeout for shell prompt detection fires `console.warn` only.
The user sees nothing in the UI. They cannot tell if their terminal session failed to start or
is still initializing.

**Decision: Wire it.**

Surface the warning as a visible UI element. Proceed:

1. In `useTerminal.ts`, when the 8-second timeout fires without detecting a shell prompt, instead
   of (or in addition to) `console.warn`, emit a state update that the terminal panel can display:

   ```typescript
   // Instead of only:
   console.warn('Shell prompt not detected after 8s — terminal may not be ready');

   // Also set a visible warning:
   setShellWarning('Shell prompt not detected. The terminal may still be starting, or the shell may have exited.');
   ```

   Add `shellWarning: string | null` to the hook's return value. Initialize to `null`; clear it
   when a prompt is successfully detected after the timeout fires.

2. In the terminal panel component that consumes `useTerminal` (find it by searching for
   `useTerminal` in `demo/demo-app/src/`), render the warning inline when `shellWarning` is
   non-null. Use a yellow/amber banner above or below the terminal output area:

   ```tsx
   {shellWarning && (
     <div className="terminal-warning">
       {shellWarning}
     </div>
   )}
   ```

   If the app has a toast notification system, use that instead of an inline banner. Match
   the existing notification style — do not introduce a new design pattern.

3. The `console.warn` can remain alongside the UI warning (useful for debugging). Do not remove it.

4. Clear the warning when the terminal session reconnects or the prompt is detected (call
   `setShellWarning(null)` in the prompt-detected branch).

---

## What NOT to Do

- Do NOT delete `AnthropicApiAdapter` or its tool-loop code. Gate-flag and document it.
- Do NOT wire the `conductor.intervention` subscriber in this task. The context window watcher
  gains gating and config-based extension; wiring the subscriber is a separate task.
- Do NOT introduce new Rust feature flags for the watcher. Use the `roko.toml` config flag.
- Do NOT change `roko.toml` model entries to point at `kind = "anthropic_api"` — that is an
  operational decision, not a cleanup task. The example config is commented out.
- Do NOT change any gate tests in `crates/roko-gate/`. Only the three files listed in `touches`.
- Do NOT leave files with `// TODO: investigate` or `// DEAD CODE` comments. After this task,
  all three subsystems have `// STATUS: GATED` (with enable instructions) or are wired.
- Do NOT add a toast library or new frontend dependency. Use the existing notification system
  or an inline `<div>`. Match the existing pattern.

## Wire Target

```bash
# Subsystem 1: status comment present in context_window_pressure.rs
grep -n 'STATUS: GATED' crates/roko-conductor/src/context_window_pressure.rs
# Expected: line with the comment

# Subsystem 1: watcher is NOT registered when flag is false (default)
grep -n 'context_pressure_enabled' crates/roko-conductor/src/watchers/mod.rs
# Expected: conditional registration guard

# Subsystem 1: ModelProfile-based context window lookup compiles
cargo check -p roko-conductor --jobs 1

# Subsystem 2: status comment present in anthropic_api.rs
grep -n 'STATUS: GATED' crates/roko-agent/src/provider/anthropic_api.rs
# Expected: line with the comment

# Subsystem 2: AnthropicApiAdapter still in dispatch table
grep -n 'anthropic_api' crates/roko-agent/src/provider/mod.rs
# Expected: entry in adapter_for_kind()

# Subsystem 3: shellWarning is exported from useTerminal
grep -n 'shellWarning' demo/demo-app/src/hooks/useTerminal.ts
# Expected: state declaration + returned in hook object

# Full build passes
cargo build --workspace
cargo test --workspace

# Frontend builds (if demo-app has a build step)
cd demo/demo-app && npm run build
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `context_window_pressure.rs` has `// STATUS: GATED` comment explaining how to enable
- [ ] `context_window_tokens()` reads from `ModelProfile.context_window` when config is provided
- [ ] `decide()` uses a lookback window (constant `PRESSURE_LOOKBACK = 3`) rather than most-recent-only
- [ ] `ContextWindowPressureWatcher` is registered in `watchers/mod.rs` only when
  `conductor.context_pressure_enabled = true` in config
- [ ] `anthropic_api.rs` has `// STATUS: GATED` comment with `kind = "anthropic_api"` enable instructions
- [ ] `anthropic_api/tool_loop.rs` (if separate file) has `// STATUS: GATED` comment
- [ ] `AnthropicApiAdapter` remains in `adapter_for_kind()` dispatch table — not removed
- [ ] Example commented-out `[providers.anthropic-api]` block exists in config (roko.toml or inline default)
- [ ] `useTerminal.ts` exports `shellWarning: string | null` from the hook
- [ ] The terminal panel component renders a visible warning element when `shellWarning` is non-null
- [ ] Shell warning clears when prompt is detected after the timeout fired
- [ ] `grep -rn 'STATUS: DEAD' crates/ --include='*.rs' | grep -v target/` — zero results
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in any file touched by this task

## Implementation Ground Truth (Worker 18 Enrichment)

Current file paths and call chains differ from the draft above:

- Context pressure watcher file is `crates/roko-conductor/src/watchers/context_window_pressure.rs`, not `crates/roko-conductor/src/context_window_pressure.rs`.
- Watcher re-exports are in `crates/roko-conductor/src/watchers/mod.rs`, but registration happens in `crates/roko-conductor/src/conductor.rs` in both `default_watchers()` and `configured_watchers(config: &ConductorConfig)`.
- `ConductorConfig` and `WatcherThresholds` are in `crates/roko-core/src/config/schema.rs`; there is already `WatcherThresholds.context_window_pressure: Option<ContextWindowPressureConfig>` with warn/critical thresholds, but no boolean `context_pressure_enabled`.
- `ContextWindowPressureWatcher::decide(&self, stream, ctx)` currently has no access to `RokoConfig` or `ModelProfile`. `extract_usage()` only receives an `Engram`. To use configured `ModelProfile.context_window`, either the watcher must carry a precomputed `HashMap<String, u64>` built when constructing the conductor, or the producer of `Kind::TokenUsage` signals must reliably tag `tokens_total`. Do not pretend `context_window_tokens(model_id, config)` can be called from the current `React` signature without plumbing that config in.
- Anthropic API adapter is real and currently reachable only when a provider has `kind = "anthropic_api"`: `provider/mod.rs::adapter_for_kind(ProviderKind::AnthropicApi)` returns the static adapter. Root `roko.toml` currently has Claude providers as `kind = "claude_cli"`; no active default model profile uses `anthropic_api`.
- Existing Anthropic API tests already cover adapter creation and tool-loop parsing in `anthropic_api.rs` and `anthropic_api/tool_loop.rs` without live API access. Add the requested status comments and only add another ignored/mock test if it covers a missing behavior.
- Shell prompt timeout is in `demo/demo-app/src/hooks/useTerminal.ts` inside the WebSocket `onopen` async block. `useTerminal()` currently returns `{ attach, status, handle }`; no `shellWarning` state exists. Consumers include `pages/Demo/TerminalPaneWithHandle.tsx`, `pages/Demo/BottomTerminalPane.tsx`, `pages/Terminal.tsx`, `components/Terminal/TerminalPane.tsx`, and `pages/Builder.tsx`.

## Mechanical Decisions and Steps (Worker 18 Enrichment)

### Context Window Pressure Watcher

1. Add `// STATUS: GATED` at the top of `watchers/context_window_pressure.rs` with the enable conditions and the note that runner reaction to `conductor.intervention` is still separate.
2. Add the config boolean to `ConductorConfig`:
   - `#[serde(default)] pub context_pressure_enabled: bool`
   - default `false`
   - include it in `Default`, example config rendering (`write_example_conductor`), and config tests that assert defaults.
3. Change `conductor.rs` registration:
   - Remove `ContextWindowPressureWatcher` from `default_watchers()` or keep `default_watchers()` only for tests that intentionally want all built-ins; production `Conductor::from_config()` must not include it when the flag is false.
   - In `configured_watchers()`, push the watcher only when `config.context_pressure_enabled` is true.
4. Fix context-window lookup mechanically:
   - If you can expand constructor scope, add `ContextWindowPressureWatcher::with_context_windows(max_ratio, map)` and have conductor construction pass model slugs/windows from the full `RokoConfig`. This requires changing the caller that creates `Conductor::from_config` to pass the whole config, not just `[conductor]`.
   - If that scope is not available, do not change the `React` trait. Instead, document that configured model windows are consumed only when token-usage signals contain `tokens_total`, and leave hardcoded Anthropic fallback as-is.
5. Replace most-recent-only detection with a lookback over the last `PRESSURE_LOOKBACK = 3` `Kind::TokenUsage` signals. Use max utilization over the window so any recent high-pressure state triggers once, and add tests for alternating high/low usage.

### Anthropic API Tool Loop

1. Add `// STATUS: GATED` comments to `anthropic_api.rs` and `anthropic_api/tool_loop.rs`.
2. Keep `ProviderKind::AnthropicApi` in `adapter_for_kind()` unchanged.
3. Do not change root `roko.toml` active model/provider kinds. If adding an example config, make it commented-out and put it in a file that is actually in scope for the implementation task. The current `touches` list does not include `roko.toml` or init-template files.
4. If adding a mock test, prefer the existing `HttpPoster` seam in `tool_loop.rs` over adding `wiremock`. A hand-rolled poster or local `tokio::net::TcpListener` keeps dependencies stable.

### Shell Prompt Warning

1. In `useTerminal.ts`, add `const [shellWarning, setShellWarning] = useState<string | null>(null);`.
2. On prompt success, call `setShellWarning(null)` before setting connected status.
3. On the 8-second timeout, keep `console.warn` and also set:
   `Shell prompt not detected. The terminal may still be starting, or the shell may have exited.`
4. Clear the warning on reconnect start (`ws.onopen`) and on `ws.onclose`.
5. Return `shellWarning` from the hook. Update every direct consumer destructuring `useTerminal(...)` so TypeScript builds.
6. Render the warning in terminal pane components. For the demo terminal, `TerminalPaneWithHandle.tsx` is the primary visible pane; add a compact amber inline banner between the header/cmd echo and `.demo-term-body`. For global terminal/components, either render the same warning or explicitly document why demo-only coverage satisfies this task.

## Scope Notes (Worker 18 Enrichment)

The task's `touches` metadata is stale. A complete implementation needs at least `crates/roko-conductor/src/watchers/context_window_pressure.rs`, `crates/roko-conductor/src/conductor.rs`, `crates/roko-core/src/config/schema.rs`, `crates/roko-agent/src/provider/anthropic_api.rs`, `crates/roko-agent/src/provider/anthropic_api/tool_loop.rs`, `demo/demo-app/src/hooks/useTerminal.ts`, and the terminal pane consumer(s). The listed path `crates/roko-conductor/src/context_window_pressure.rs` does not exist. Do not start code work until the implementation worker updates the task scope or records owner approval for these paths.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
