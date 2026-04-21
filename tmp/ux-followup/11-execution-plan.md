# Execution Plan — Phases, Batches, Prompt Skeletons

> **Status (post-PR-13)**: Phase A and Phase B closed (T20–T23 landed via
> PR #13 merge). Phase C partially advanced — T24 half done; T25 + T26 still
> open. Phases D + E + F (T27–T32) appended below for the next runner pass.
> Refreshed 2026-04-16.

## Summary

Phased execution order for the 100+ items now catalogued in files 01–15, plus
ready-to-paste batch-prompt skeletons for the Codex/Claude runner.

## Phase ordering

### Phase A — [DONE] Unblock PR #13

Goal: ship PR #13 with accurate claims. **All four items closed.**

1. **Hotfix merge** — items 01, 02, 44 → DONE (PR #13 merged via `5ff264c9`).
2. **Audit T12, T13 scope** — items 21, 22 → DONE.
3. **Re-verify T18 clippy on clean base** — item 26 → DONE.
4. **Re-queue T14, T17, T19** → DONE (T14/T17 in PR #13; T19 via `c9029e20`).

### Phase B — [DONE] Finish parity batches

5. **T20** (T14 modal consolidation) — DONE.
6. **T21** (T17 scroll/nav) — DONE.
7. **T22** (T19 messaging integration tests) — DONE (`c9029e20`).
8. **T23** (Runner hardening — items 27, 28, 27a, 28a) — **still open**;
   move to Phase D.

### Phase C — Close the self-hosting loop (in progress)

9. **T24** (Auto-plan-on-promote — item 05) — **half done**. CLI side wired
   at `crates/roko-cli/src/prd.rs:628` (`maybe_generate_plan_after_promote`).
   Orchestrator-side PRD-publish event subscription (T31 below) still missing.
10. **T25** (Gate-failure feedback loop — item 06 / item 89) — **not started**.
11. **T26** (End-to-end self-hosting smoke test — item 60) — **not started**.

### Phase D — Hardening & UX polish (next 2 weeks)

12. **T23 (carry-forward)**: runner hardening — items 27, 28, 27a, 28a.
13. Unwrap cleanup — item 55.
14. Coverage in CI — item 59.
15. Doc sweep — items 61–67, 67a.
16. HTTP route validation — item 60a.
17. Enrichment call-site decision — items 29 / 95.
18. Diagnosis / topology / experiments widgets — items 10, 14, 20, 84.

### Phase E — Streaming TUI (T27–T29)

Replace polling with subscriptions. See file 12 (TUI event-parity) for items
68–78 that drive these batches.

19. **T27**: Wire TUI `StateHub` subscription unconditionally.
20. **T28**: Sidecar `/stream` WebSocket consumer in TUI.
21. **T29**: File-watcher (notify crate) replaces 500 ms polling thread.

### Phase F — Self-hosting closure (T30–T32)

22. **T30**: Gate-feedback → plan-regeneration loop (CLAUDE.md item 11).
23. **T31**: PRD-publish event → auto-orchestrator (CLAUDE.md item 10).
24. **T32**: Snapshot schema versioning + migration shim.

### Phase G — Phase 2 vision (unscheduled)

25. Items 49–54 (file 08). Parked until Phase F completes.

## Batch prompt skeletons

Each skeleton follows the existing T-batch format (see
`tmp/tui-parity/prompts/T*.prompt.md` for shape). Paste into
`tmp/tui-parity/prompts/T2N.prompt.md` to queue.

### T20 skeleton — TUI modal consolidation [DONE]

(Landed via PR #13. Skeleton retained for reference.)

```markdown
# Batch T20: TUI modal system consolidation (retry of T14)

Read these files first:
- tmp/tui-parity/context-pack/00-TUI-PARITY-RULES.md
- crates/roko-cli/src/tui/state.rs
- crates/roko-cli/src/tui/input.rs
- crates/roko-cli/src/tui/app.rs
- crates/roko-cli/src/tui/modals/

## Task
Retire every `show_*: bool` on TuiState in favour of `active_modal: Option<ModalState>`.
Grep, rewrite, verify no desync possible.
```

### T21 skeleton — TUI scroll & navigation [DONE]

(Landed via PR #13. ScrollAccel wired; PgUp/Down + tab-aware nav + Logs G/End +
clamping + focus border + Ctrl-C force-quit all in.)

### T22 skeleton — messaging integration tests [DONE]

(Landed via `c9029e20`.)

### T23 skeleton — runner hardening [STILL OPEN]

```markdown
# Batch T23: tui-parity runner hardening

Read these files first:
- tmp/tui-parity/run-tui-parity.sh
- tmp/tui-parity/lib/common.sh
- tmp/tui-parity/BATCHES.md
- tmp/tui-parity/logs/run-20260416-101433/status.tsv

## Task
1. Document the runner-stop root cause from 2026-04-16.
2. Add explicit TUI_PARITY_MAX_BATCHES and TUI_PARITY_MAX_RETRIES env vars.
3. Log these at runner start.
4. On unexpected spawn_failed + no result file, emit a trailer to status.tsv.
5. Add a CI step that runs `bash tmp/tui-parity/run-tui-parity.sh --dry-run`
   on PRs touching `tmp/tui-parity/`.
6. Add a log retention policy (e.g. keep last 5 runs).

## Write scope
- tmp/tui-parity/lib/common.sh
- tmp/tui-parity/BATCHES.md
- tmp/tui-parity/run-tui-parity.sh (if needed)
- .github/workflows/tui-parity-runner.yml (new)

## Rules
1. Bash-only; no runtime Rust changes.
2. Script changes must still allow resumption via existing status.tsv.
```

### T24 skeleton — auto-plan on PRD promote [HALF DONE]

CLI half wired at `crates/roko-cli/src/prd.rs:628`. Remaining work covered by
T31 (orchestrator side).

### T25 skeleton — failed-gate feedback loop [STILL OPEN]

```markdown
# Batch T25: Gate-failure → plan-revision feedback loop

Read these files first:
- CLAUDE.md (What to work on, item 11)
- crates/roko-gate/src/lib.rs
- crates/roko-cli/src/orchestrate.rs
- crates/roko-orchestrator/src/plan_runner.rs (or equivalent)
- crates/roko-learn/src/episode_logger.rs

## Task
After N consecutive gate failures on a task, emit a PlanRevision event that
triggers a re-plan. Dedupe via task hash to avoid infinite loops.

## Write scope
- crates/roko-orchestrator/src/plan_runner.rs (event emit)
- crates/roko-cli/src/orchestrate.rs (subscribe + dispatch to plan-gen)
- new: crates/roko-orchestrator/src/replanning.rs

## Rules
1. Default N = 3, configurable via roko.toml.
2. Replan requests recorded in .roko/episodes.jsonl.
3. Max replans per plan is 2 (hardcoded for Phase F).

## Done when
Setting `ROKO_FORCE_FAIL=1` in a test task triggers a replan after 3 attempts
AND stops after 2 replans, both observed in the episode log.
```

### T26 skeleton — end-to-end self-hosting smoke [STILL OPEN]

```markdown
# Batch T26: Self-hosting smoke test (E2E)

Read these files first:
- CLAUDE.md (Self-hosting workflow)
- crates/roko-cli/src/main.rs (all subcommands)
- tests/ at workspace root (may need to create)

## Task
Create an integration test (or shell script) that drives the full CLAUDE.md
workflow with a MockDispatcher that returns canned responses. Assert the
expected `.roko/*.jsonl` + plans/tasks.toml artifacts land.

## Write scope
- tests/e2e_self_host.rs OR tmp/smoke/e2e.sh
- new: crates/roko-agent/src/mock.rs adjustments if needed for scripted replies

## Rules
1. Runs under `cargo test --workspace` (or a specific test filter).
2. Completes in under 60 s on developer laptop.
3. Cleans up `.roko-test/` scratch dir on exit.

## Done when
CI green on a workflow that runs only the e2e test.
```

### T27 skeleton — TUI subscribe to StateHub unconditionally

```markdown
# Batch T27: Eliminate TUI polling fallback; subscribe to StateHub always

Read these files first:
- tmp/ux-followup/12-tui-event-parity.md (item 68 + 69)
- crates/roko-cli/src/tui/app.rs (especially lines 357-361, 412, 432-447, 533-539)
- crates/roko-cli/src/tui/dashboard.rs (refresh_sync at lines 532+)
- crates/roko-core/src/state_hub.rs (or equivalent)
- crates/roko-cli/src/tui/state.rs

## Task
1. Standalone TUI must always subscribe to a StateHub instance. If no
   external hub is supplied, spawn a private hub in-process.
2. Delete the `if app.snapshot_rx.is_none() { ... refresh ... }` polling fallback.
3. Confirm `TuiState::update_from_snapshot` is the only path that mutates
   render state.
4. Add a unit test asserting the polling branch is unreachable.

## Write scope
- crates/roko-cli/src/tui/app.rs
- crates/roko-cli/src/tui/dashboard.rs (refresh callers may need slimming)
- new: crates/roko-cli/src/tui/state_hub_local.rs (in-process hub)

## Rules
1. No `last_refresh.elapsed() > Duration::from_secs(_)` patterns post-batch.
2. cargo clippy -p roko-cli --no-deps -- -D warnings green.
3. Manually verify `roko dashboard` still renders updates within 200 ms of an
   event.

## Done when
Grep `if .*snapshot_rx.is_none` in `crates/roko-cli/src/tui/` returns 0.
```

### T28 skeleton — Sidecar /stream WebSocket consumer in TUI

```markdown
# Batch T28: Wire roko-agent-server `/stream` WS into the Agents-tab view

Read these files first:
- tmp/ux-followup/12-tui-event-parity.md (item 70)
- crates/roko-agent-server/src/features/messaging.rs (post-T9 stream handler)
- crates/roko-cli/src/tui/views/agents_view.rs
- crates/roko-cli/src/tui/state.rs (agent status fields)

## Task
1. On Agents-tab activation, open a WS to `ws://<host>/agents/{id}/stream`
   for each visible agent.
2. Push incoming chunks into `TuiState::agent_streams` (new ring buffer).
3. Render the live tail in the agent detail panel.
4. Tear down the WS on tab change to avoid leaks.

## Write scope
- crates/roko-cli/src/tui/views/agents_view.rs
- crates/roko-cli/src/tui/state.rs (new agent_streams field)
- new: crates/roko-cli/src/tui/ws_client.rs

## Rules
1. Use `tokio-tungstenite`; reuse existing dependency if present.
2. Bounded ring buffer (e.g. last 200 chunks per agent) — no unbounded growth.
3. Reconnect on socket close with exponential back-off.
```

### T29 skeleton — File-watcher replaces 500 ms polling thread

```markdown
# Batch T29: Replace .roko/* polling with `notify` file watcher

Read these files first:
- tmp/ux-followup/12-tui-event-parity.md (items 69, 71-76)
- crates/roko-cli/src/tui/app.rs (lines 526-549)
- crates/roko-cli/src/tui/dashboard.rs (refresh_sync, file_stamp callers)

## Task
1. Spawn a single `notify::Watcher` over `.roko/` (recursive).
2. On any change, send a debounced (200 ms) refresh signal.
3. Delete the 500 ms `std::thread::sleep` polling loop.
4. Compatibility: when run on a filesystem without inotify (some macOS bind
   mounts), fall back to a 1 s poll explicitly logged at startup.

## Write scope
- crates/roko-cli/src/tui/app.rs
- new: crates/roko-cli/src/tui/fs_watch.rs

## Rules
1. Add `notify = "6"` to roko-cli/Cargo.toml.
2. Ring-buffer the events; coalesce bursts.
3. cargo test -p roko-cli green.
```

### T30 skeleton — Gate-feedback → plan-regeneration loop

(Same body as T25 above. T30 is the renumbered, post-PR-13 successor of T25
once Phase D wraps.)

### T31 skeleton — PRD-publish event → auto-orchestrator

```markdown
# Batch T31: Auto-trigger orchestrator when a PRD is published

Read these files first:
- CLAUDE.md (What to work on, item 10)
- crates/roko-cli/src/prd.rs:628-708 (existing CLI half)
- crates/roko-cli/src/main.rs (orchestrator entrypoints)
- crates/roko-runtime/src/event_bus.rs

## Task
1. Define a `RokoEvent::PrdPublished { slug, path }` on the event bus.
2. Emit it from `prd draft promote` whenever the promote succeeds (whether
   or not the in-process auto-plan ran).
3. Add an orchestrator-side subscriber in `roko serve` that, on receipt,
   runs `prd plan <slug>` then `plan run <plans-root>` (idempotent).
4. Provide a roko.toml flag `[serve] auto_orchestrate = true` (default true).

## Write scope
- crates/roko-runtime/src/event_bus.rs (event variant)
- crates/roko-cli/src/prd.rs (publish-emit)
- crates/roko-serve/src/lib.rs (subscriber wiring)
- crates/roko-cli/src/serve.rs (boot the subscriber)

## Rules
1. Event must include enough context for the subscriber to act without
   re-reading the PRD draft (slug + canonical path is enough).
2. Subscriber must dedupe: if the same slug is published twice within 60 s,
   only run once.
3. Integration test in `crates/roko-serve/tests/`.

## Done when
`cargo run -p roko-cli -- prd draft promote demo` (with `roko serve` running
in another terminal) leaves a `plans/demo/tasks.toml` AND triggers a plan
run, observable in `.roko/episodes.jsonl`.
```

### T32 skeleton — Snapshot schema versioning + migration shim

```markdown
# Batch T32: Version the executor.json snapshot

Read these files first:
- tmp/ux-followup/13-session-state-mgmt.md (items 79, 81, 82)
- crates/roko-cli/src/orchestrate.rs:646-658 (save_snapshot_atomic)
- ExecutorSnapshot definition (grep `pub struct ExecutorSnapshot`)

## Task
1. Add `pub schema_version: u32` to `ExecutorSnapshot`, default 1.
2. On load, if `schema_version != CURRENT`, run a `SnapshotMigrate::upgrade`
   shim that walks v1 → v2 → … → CURRENT in order.
3. If a snapshot has no version field at all, treat as v0 and migrate.
4. Validate the plan-discovery list against the snapshot at resume time —
   if a plan was renamed between save and resume, surface a clear error.
5. Snapshot rotated atomically as today.

## Write scope
- crates/roko-cli/src/orchestrate.rs (struct + migrate fn)
- new: crates/roko-cli/src/snapshot_migrate.rs

## Rules
1. Migrations forward-only; no reverse path.
2. Tests covering v0 → v1, v1 → v2 (synthetic).
3. Resume against a missing-plan snapshot must error before any side effect.

## Done when
`roko plan run plans/ --resume <old-snapshot.json>` from a v0 fixture loads
without panic.
```

## Dependency graph (text form)

```
Phase A (DONE)
Phase B (DONE)
Phase C (in progress)
  └── T24 (half done) — orchestrator side via T31
  └── T25 → T30 (carry forward)
  └── T26 (e2e smoke)

Phase D (hardening)
  └── T23 carry-forward (runner)
  └── parallel: doc sweep, unwrap, coverage, validation

Phase E (streaming TUI)
  └── T27 (StateHub unconditional) → T28 (sidecar WS) → T29 (file watcher)

Phase F (self-hosting closure)
  └── T30 (gate feedback) — depends on T25 unfinished bits
  └── T31 (PRD publish event) — depends on T24 CLI side (DONE)
  └── T32 (snapshot version) — independent
```

## Recommended next action

1. Review the post-PR-13 catalogue refresh (`00-INDEX.md` + this file) with the user.
2. Decide whether to land Phase D (hardening) or Phase E (streaming TUI) first.
3. Write `T27.prompt.md` (or `T23.prompt.md` for runner hardening) into
   `tmp/tui-parity/prompts/` and kick off the runner.

No further code changes are produced by this catalogue refresh — this document
is the handoff.
