# Non-Batch Follow-ups — Deferred from the T9–T19 Plan

> **Status (post-PR-13)**: 3 DONE / 3 open. Refreshed 2026-04-16.
>
> **Re-audit 2026-04-20**: 2 more items closed (18, 20). 1 item still open (19).

## Summary

Six items explicitly deferred from the original T9–T19 batch plan because they
either (a) couldn't fit the single-crate clippy gate, (b) required human review
of semantics, or (c) cross-cut multiple batches. Captured here so they don't
vanish when the next runner goes out.

## Items

### 15. [DONE] T14 modal-system consolidation

**Resolved in**: T14 retry batch landed with the rest of T9–T19 in PR #13.
Final integration commit: `1f40e3d6` (TUI parity batch series, see PR #13 body
parity table). Subsequent dual-modal cleanup at `e792e649` (merge of afternoon
runner).

**Original gap**: TUI had a dual modal system (`show_*: bool` and
`active_modal: Option<ModalState>`) that could desync.

**Status**: ✅ DONE (modal API consolidated). Cross-ref the surviving
`show_plan_detail` / `show_*` audit captured at item 70 / 73 in
`12-tui-event-parity.md` if any stragglers turn up during streaming work.

---

### 16. [DONE] T17 scroll / nav fixes

**Resolved in**: T17 retry landed in PR #13. Integration commit on the
parity-batch series at `0ac99938` (PgUp/Down + ScrollAccel + tab-aware nav +
Logs End/G + clamping + focus + Ctrl-C force-quit). See item 07.

**Original gap**: 7 distinct nav UX issues left over from the morning runner.

**Status**: ✅ DONE.

---

### 17. [DONE] T19 messaging integration tests

**Resolved in**: Commit `c9029e20`
(`tui-parity(T19): Agent-server messaging integration tests`). Adds end-to-end
axum-on-random-port tests covering the four scenarios from the original
`T19.prompt.md` (mock dispatcher, missing dispatcher → 503, dispatch error →
502, streaming chunks).

**Status**: ✅ DONE. Closes item 25 in `04-t9-t19-residuals.md` and item 57 in
`09-hygiene-and-test-coverage.md`.

---

### 18. [DONE] Restore `roko-runtime::CancellationToken` integration in graceful-shutdown path

**Resolved in**: `crates/roko-runtime/src/process.rs` now has:
- `CancellationToken` plumbed as the `cancel` field on `ProcessHandle` (line ~80), with
  `cancel_token()` accessor (line ~348).
- `shutdown()` (line ~329) calls `self.cancel.cancel()` then SIGTERM (line ~339), waits for
  graceful exit (line ~341), then escalates to `force_kill()` (line ~344).
- `impl Drop for ProcessSupervisor` (line ~708) calls `cancel()` and `force_kill_sync()` on
  all live children.
Cross-ref item 80 now also DONE.

**Status**: DONE.

---

### 19. `SystemPromptBuilder` 6-layer coverage audit

**Evidence**: CLAUDE.md claims "SystemPromptBuilder (6-layer prompts) — Wired". `crates/roko-compose/src/system_prompt_builder.rs` exists; workspace grep shows ~20 `unwrap()` calls (cross-ref item 55).

**Current state**: Wired but not audited layer-by-layer. Unknown whether all 6 layers (role/skills/tools/context/memory/policy) actually compose in live runs.

**Gap**: Write a snapshot test per role (implementer, reviewer, planner, researcher, etc.) that captures the rendered system prompt and diffs against a golden file.

**Fix scope**: 1 day. Snapshot tests only; no runtime changes.

**Priority**: P1.

---

### 20. [DONE] Prompt-experiment A/B registry visibility in dashboard

**Resolved in**: The Learning tab now renders concluded experiment winners:
- `crates/roko-cli/src/tui/dashboard.rs` computes `experiment_winners` from
  `experiment_store.winner_summaries()` (line ~459) and exposes them on `DashboardData`.
- `crates/roko-cli/src/tui/views/dashboard_view.rs` has a full
  `render_concluded_experiments_panel()` (line ~1208) showing experiment ID, winner variant,
  sample size, and confidence bars. Width scaling via `concluded_experiment_winner_width()`
  (line ~1349). Unit test at line ~2270 verifies rendering.
- `crates/roko-cli/src/tui/state.rs` carries `experiment_winners` (line ~786) populated from
  snapshot data. Cross-ref item 88 now also DONE.

**Status**: DONE.
