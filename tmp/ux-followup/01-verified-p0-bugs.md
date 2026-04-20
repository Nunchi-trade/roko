# Verified P0 Bugs — Pre-Merge Hotfixes

> **All P0 merge-blockers were addressed in PR #13 (`5ff264c9`).** This file is
> retained as evidence of what landed; no remaining open items in this category.
> See `00-INDEX.md` "Post-PR-13 delta" for the full closeout list.

## Summary

Four issues found while comparing the committed T9–T19 worktree
(`codex/tui-parity-run-20260416-101433`) against `main`. Every item here blocked
PR #13 from shipping in its original form. All four were resolved by the PR #13
merge and follow-up batches.

## Items

### 01. [DONE] Agent-server `POST /message` still returns echo-back on main

**Resolved in**: T9 commit `dcd06257` merged via PR #13 (`5ff264c9`).
Live dispatch confirmed at
`crates/roko-agent-server/src/features/messaging.rs` (real `backend.send_turn(...)` with
503/502 error paths).

**Original evidence**:
- Pre-merge main: `messaging.rs` returned `format!("{}: {}", state.agent_id(), request.prompt.trim())`.
- Worktree path: `.roko/worktrees/tui-parity-run-20260416-101433/crates/roko-agent-server/src/features/messaging.rs:44-68` — real dispatch via `backend.send_turn(...)`.

**Original gap**: Issue #45 spec #5 was effectively FALSE on `main`; aggregator-proxied
calls returned the echo string instead of model output.

**Fix scope (delivered)**: Single-commit cherry-pick into PR #13 + workspace clippy
+ T19 integration tests (item 25).

**Status**: ✅ DONE (`dcd06257` on main as of `5ff264c9`).

---

### 02. [DONE] PR #13 body lists only T1–T8 but worktree has T9–T18 committed

**Resolved in**: PR #13 body refresh shipped at `b1bba746`
(`docs: refresh READMEs for PR #13 feature surface`) and the merge commit `5ff264c9`
collapses T1–T19 into one umbrella diff. `git log --oneline main` now shows
`Merge TUI parity batches T9-T19 from afternoon Codex runner` (`e792e649`).

**Original evidence**: `gh pr view 13 --json body` originally stopped the parity
table at T8 (`5fd6956a`); 8 more tui-parity commits were ahead on
`codex/tui-parity-run-20260416-101433`.

**Original gap**: Reviewers could not tell which batches were in / failed /
pending. The TL;DR claim "T1–T8" was stale.

**Fix scope (delivered)**: PR body rewrite + merge of the worktree branch.

**Status**: ✅ DONE.

---

### 03. [DONE] T18 verification failed on attempt 1; retry committed with possibly-different scope

**Resolved in**: T18 retry-commit `552a7cd0` was rebased and re-verified during
the PR #13 integration; the merge to main passed `cargo clippy -p roko-serve -p
roko-mcp-code --no-deps -- -D warnings` cleanly (no follow-up commit was needed).

**Original evidence**: `tmp/tui-parity/logs/run-20260416-101433/status.tsv` shows
the verify_failed → preserved_retry → succeeded sequence on T18.

**Original gap**: Attempt 1's dirty state was preserved into attempt 2; no
confirmed clean clippy on a clean re-run.

**Fix scope (delivered)**: 10-min verification on the integration branch.

**Status**: ✅ DONE.

---

### 04. [DONE] Runner auto-stopped at T14; T14, T17, T19 have no result files

**Resolved in**: All three batches re-queued and landed on main:
- T14 modal consolidation → see PR #13 body / `e792e649`.
- T17 scroll/nav → see PR #13 body / `e792e649`.
- T19 messaging integration tests → commit `c9029e20`
  (`tui-parity(T19): Agent-server messaging integration tests`).

**Original evidence**:
- `ls tmp/tui-parity/logs/run-20260416-101433/*.result` was missing T14/T17/T19.
- `status.tsv` last entry for T14 was `attempt_started` with no follow-on event.

**Original gap**: Three batches in the original T9–T19 plan were not executed;
runner wrapper stopped silently.

**Fix scope (delivered)**: Re-queue via the afternoon runner pass. Items 27/28
(runner hardening) remain open in `04-t9-t19-residuals.md` as P1 follow-ups.

**Status**: ✅ DONE.

---

## Post-mortem note

All four P0s closed within the PR #13 cycle. The runner-wrapper drift that
caused the original T14 stop is still being tracked under file 04
(items 27, 28, 27a, 28a) so the next runner pass doesn't repeat the silent stop.
