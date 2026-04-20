# Spec-Code Drift — Mismatches, Pick a Direction

> **Status (post-PR-13)**: items 41, 42, 43, 44, 48 closed; 45, 46, 47 still
> open + 3 new drift items. Refreshed 2026-04-16.
>
> **Re-audit 2026-04-20**: 3 more items closed (48a, 48b, 48c). 3 items still open (45, 46, 47).

## Summary

Places where documentation (PR body, CLAUDE.md, prompt specs,
MORI-PARITY-CHECKLIST.md) says one thing and the code says another. For each,
pick: update doc to match code, or change code to match spec. Left unresolved,
these become tech debt as newcomers follow the wrong source.

## Items

### 41. [DONE] PR #13 body says "T1–T8 landed"; worktree has T9–T18 too

**Resolved in**: PR body refresh + merge of `codex/tui-parity-run-20260416-101433`
(`5ff264c9`). The merge commit narrative now covers T1–T19.

**Status**: ✅ DONE.

---

### 42. [DONE] CLAUDE.md "TUI is text-only" vs. actual ratatui wiring

**Resolved in**: CLAUDE.md updates that landed alongside PR #13 (`b1bba746`).
Current "Status table" lists "Interactive TUI (ratatui) — Wired" with the
`F1–F7 tabs` label and `roko dashboard` entry-point.

**Status**: ✅ DONE.

---

### 43. [DONE] CLAUDE.md "Text dashboard | Scaffold" row

**Resolved in**: Same CLAUDE.md refresh as item 42. The row now reads
"Interactive TUI (ratatui) — Wired" pointing at `crates/roko-cli/src/tui/`.

**Status**: ✅ DONE.

---

### 44. [DONE] Spec #05 (agent messaging) marked ✅ on PR; code on main was stub

**Resolved in**: T9 commit `dcd06257` merged via PR #13. Spec #05 is now
genuinely satisfied by `crates/roko-agent-server/src/features/messaging.rs`'s
real `backend.send_turn(...)` path. T19 integration tests (`c9029e20`) lock in
the contract.

**Status**: ✅ DONE.

---

### 45. CLAUDE.md "What to work on" items 1–9 marked done; code paths differ

**Evidence**: CLAUDE.md "Self-hosting workflow" + "What to work on" lists
items 1–9 (Rust toolchain, SystemPromptBuilder, EpisodeLogger, ProcessSupervisor,
MCP, Learning & feedback, TUI, Sidecar, HTTP) as ~~struck through~~ = "Done".
Audit shows most are wired but no integration tests confirm end-to-end
semantics.

**Direction**: Either add smoke tests per item (cross-ref item 60), or
qualify the "Done" markers in CLAUDE.md to "Wired, smoke test pending".

**Fix scope**: 1–2 days per item for smoke tests.

**Priority**: P1.

---

### 46. `MORI-PARITY-CHECKLIST.md` claims 33% done; no mechanical check ever ran

**Evidence**: `bardo-backup/tmp/roko-progress/MORI-PARITY-CHECKLIST.md` —
1 253 items, "~33% done". The markdown is hand-maintained and has not been
regenerated post-PR-13.

**Direction**: Write a script that walks the checklist and for each item greps
the current codebase for the relevant call-site to produce a freshness-verified
percentage.

**Fix scope**: 2–3 days for a robust walker. 1 day for a fuzzy one.

**Priority**: P1.

---

### 47. `bardo-backup/tmp/roko-progress/` is a snapshot — reviewers think it's current

**Evidence**: Path contains "bardo-backup", modification dates pre-date most
roko commits. But the files contain phrases like "CURRENT state" which read as
live.

**Direction**: Add a banner at the top of each
`bardo-backup/tmp/roko-progress/*.md`:
```
> ⚠ This document is a snapshot from <date>. It is **not** updated as roko evolves.
> For current state, see `CLAUDE.md` in the repo root.
```

**Fix scope**: 30 minutes for a mass sed insertion.

**Priority**: P1.

---

### 48. [DONE] `roko-primitives` vs `bardo-primitives` naming confusion

**Resolved in**: CLAUDE.md updated to list `roko-primitives` (with the
"HDC vectors, tier routing" descriptor and "Tier wired in
orchestrate/neuro/learn; HDC fingerprint-per-episode pending" note).

**Status**: ✅ DONE.

---

### 48a. [DONE] Adaptive gate thresholds — write path verified, load path not

**Resolved in**: `AdaptiveThresholds::load_or_new()` is now called at orchestrator
initialization (orchestrate.rs lines ~4667, ~4825, ~4985) and the loaded thresholds are
consumed at dispatch: `threshold_for(rung)` (line ~15357), `should_skip_rung()` (line ~15215),
`override_for_role()` (line ~15349), `suggested_max_retries()` (line ~7965). Save path at
line ~5547. Both load and save paths fully verified. Cross-ref item 08 now also DONE.

**Status**: DONE.

---

### 48b. [DONE] `roko.toml` keys defined in schema but not consumed at runtime

**Resolved in**: `crates/roko-core/src/config/schema.rs` defines `RoleOverride` (line ~2022)
with all the keys: `routing_overrides` (line ~2052), `thresholds` including
`gate_pass_rate_floor` (line ~1924), and budget fields. These are parsed and consumed at
runtime: `SafetyLayer::build_role_tools()` reads `config.agent.roles` (safety/mod.rs line
~266), the adaptive thresholds honor `gate_pass_rate_floor` (adaptive_threshold.rs line ~307),
and routing overrides are plumbed through the cascade router. Tests at
`crates/roko-cli/tests/agent_config.rs` (line ~18) verify `thresholds` parsing, and schema.rs
(line ~4487) tests `routing_overrides` parsing.

**Status**: DONE.

---

### 48c. [DONE] CLAUDE.md "What to work on" items 10–11 still open — explicit acknowledgement

**Resolved in**: Both items are now fully wired:
- **Item 10** (auto plan generation): In addition to the CLI-side
  `maybe_generate_plan_after_promote` (prd.rs line ~628), the orchestrator side is now fully
  wired. `RokoEvent::PrdPublished` exists on the event bus (event_bus.rs line ~123) with tests
  (lines ~444, ~461). `crates/roko-serve/src/routes/prds.rs` has full PRD-publish handling:
  `append_prd_published_episode()` (line ~78), `handle_prd_published_event()` (line ~156),
  `follow_prd_published_audit()` (line ~186), and a `[serve] auto_orchestrate` config flag
  (schema.rs line ~3116, default true). The promote route emits the event (line ~525) and
  triggers auto-plan + auto-orchestrate (line ~499). Integration test in
  `crates/roko-serve/tests/prd_publish.rs`.
- **Item 11** (feedback loop): Full `PlanRevision` pipeline in orchestrate.rs (see item 06
  and 89 DONE notes). `PlanRevisionReason::GateFailureLimit`, dedup, and DAG re-injection
  all wired.

Cross-ref items 89 and 90 now also DONE.

**Status**: DONE.
