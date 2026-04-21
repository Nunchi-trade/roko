# T9–T19 Residuals — In-Flight Batch Work + Cleanup

> **Status (post-PR-13)**: items 21–26 all DONE; items 27, 28, 27a, 28a remain
> open as P1 runner-hardening follow-ups. Refreshed 2026-04-16.
>
> **Re-audit 2026-04-20**: 1 more item closed (28a). 3 items still open (27, 27a, 28).

## Summary

Per-batch cleanup items extracted from `tmp/tui-parity/logs/run-20260416-101433/status.tsv`
and the 11 prompt files in `tmp/tui-parity/prompts/`. Some batches committed
cleanly; others had retry attempts that left dirty state or skipped sub-scopes.

## Runner outcome matrix (run-20260416-101433)

| Batch | Attempts | Final commit | State (post-PR-13) |
|-------|----------|--------------|---------|
| T9  | 1 | `dcd06257` | ✅ merged into main via PR #13 |
| T10 | 1 | `fafbdbfd` | ✅ merged |
| T11 | 1 | `8d547899` | ✅ merged |
| T12 | 2 (spawn_failed ×2 then restart) | `fc1c1452` | ✅ merged + scope audited |
| T13 | 2 | `2940ede3` | ✅ merged + scope audited |
| T14 | 1 attempt_started; no result | (retry) | ✅ retry merged via PR #13 |
| T15 | 1 | `0dc01e21` | ✅ merged |
| T16 | 1 | `498d6ed6` | ✅ merged |
| T17 | (retry) | (retry) | ✅ retry merged via PR #13 |
| T18 | 2 (verify_failed on 1) | `552a7cd0` | ✅ rebased + clippy re-verified clean |
| T19 | (retry) | `c9029e20` | ✅ messaging integration tests landed |

## Items

### 21. [DONE] T12 scope audit — did all 6 items land?

**Resolved in**: PR #13 review pass; T12 diff (`fc1c1452`) audited against
`tmp/tui-parity/prompts/T12.prompt.md` checklist. All 6 inject/filter
input-line-visibility items confirmed present.

**Status**: ✅ DONE.

---

### 22. [DONE] T13 scope audit

**Resolved in**: PR #13 review pass; T13 diff (`2940ede3`) audited against
`tmp/tui-parity/prompts/T13.prompt.md` (modal data, PlanDetail, key intercepts).
All sub-items present.

**Status**: ✅ DONE.

---

### 23. [DONE] T14 modal-system consolidation

**Resolved in**: T14 retry batch landed via PR #13. See item 15 in
`03-non-batch-followups.md`.

**Status**: ✅ DONE.

---

### 24. [DONE] T17 nav/scroll

**Resolved in**: T17 retry batch landed via PR #13. See item 16 in
`03-non-batch-followups.md` and item 07 in `02-high-impact-quick-wins.md`.

**Status**: ✅ DONE.

---

### 25. [DONE] T19 integration tests

**Resolved in**: Commit `c9029e20`. See item 17 in `03-non-batch-followups.md`.

**Status**: ✅ DONE.

---

### 26. [DONE] T18 clean-rebase clippy re-verify

**Resolved in**: PR #13 integration verification. `cargo clippy -p roko-serve -p
roko-mcp-code --no-deps -- -D warnings` ran green on the rebased commit before
merge. No follow-up commit needed.

**Status**: ✅ DONE.

---

### 27. Consolidate `BATCHES.md` + `lib/common.sh` runner-wrapper drift

**Evidence**: At time of catalogue freeze, `git status` showed:
```
M tmp/tui-parity/BATCHES.md
M tmp/tui-parity/lib/common.sh
```
PR #13's afternoon merge cleaned the working tree (current `git status` is
clean), but neither file's runner-stop diagnosis was checked in as a code
change.

**Current state**: The runner wrapper that stopped at T14 was never
post-mortem'd. The dirty edits were either committed or discarded; either way,
the root cause is undocumented.

**Gap**: Open the run-20260416-101433 log + commit history, identify what
changed in `BATCHES.md` / `lib/common.sh`, and either land a defensive log
trailer or document the fix.

**Fix scope**: 30 minutes.

**Priority**: P1.

---

### 27a. Runner log retention policy missing

**Evidence**: `tmp/tui-parity/logs/` accumulates per-run subdirectories
(`run-YYYYMMDD-HHMMSS/`). No rotation, no cap, no `.gitignore`.

**Current state**: Logs grow unbounded; will eventually bloat the repo or local
clones.

**Gap**: Add a retention policy (e.g. "keep last 5 runs", or move to a
non-tracked location like `.tui-parity/logs/`).

**Fix scope**: 1 hour. Either gitignore + cleanup script or path move.

**Priority**: P1.

---

### 28. Runner max-batch / max-retry-per-batch environment knobs

**Evidence**: Runner stopped after T14 `attempt_started` with no retry or
failure log. Suggests either a hardcoded batch cap or a crashing step without
error propagation. `tmp/tui-parity/lib/common.sh` likely holds the logic.

**Gap**: Make max-batch and retry-cap first-class env vars
(`TUI_PARITY_MAX_BATCHES`, `TUI_PARITY_MAX_RETRIES`), log them at startup,
surface an error on reach.

**Fix scope**: 1 day. Runner-wrapper-only change.

**Priority**: P1.

---

### 28a. [DONE] Runner dry-run mode not exercised by CI

**Resolved in**: `.github/workflows/tui-parity-dry-run.yml` now runs both
`bash tmp/tui-parity/run-tui-parity.sh --dry-run` and
`bash tmp/ux-followup-runner/run-ux-followup.sh --dry-run` on PRs.

**Status**: DONE.
