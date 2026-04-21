# High-Impact Quick Wins — 1–3 Days Each

> **Status (post-PR-13)**: 2 items closed (05 auto-plan, 07 ScrollAccel).
> 8 items still open. Refreshed 2026-04-16.
>
> **Re-audit 2026-04-20**: All 8 remaining items closed (06, 08, 09, 10, 11, 12, 13, 14).
> 0 items still open.

## Summary

10 items surfaced by cross-referencing `bardo-backup/tmp/roko-progress/MORI-PARITY-CHECKLIST.md`
against current `crates/` state. Each is a 1–3 day unit of work with high
leverage — either an already-built subsystem that just needs a call-site, or a
tiny missing piece whose absence blocks a whole feature.

None are P0 for the PR. Each unlocks follow-on work or measurable UX
improvement. Ranked by ROI descending.

## Items

### 05. [DONE] Auto-trigger `prd plan` when a PRD is promoted to `published`

**Resolved in**: `crates/roko-cli/src/prd.rs:628` — `maybe_generate_plan_after_promote`
is now invoked from the promote handler, gated by the `[prd] auto_plan` toml key
(`prd.rs:690-708`). Helper `auto_plan_enabled` short-circuits when disabled.

**Original gap**: Two-step promote → plan workflow added unnecessary friction.

**Status**: ✅ DONE. Closes the first half of CLAUDE.md "What to work on" item 10
(auto-plan on promote). Item 90 in `15-safety-and-learning-closure.md` tracks the
remaining half (orchestrator-side subscription to PRD-publish events).

---

### 06. [DONE] Feed failed-gate results back into plan generator for re-planning

**Resolved in**: `crates/roko-cli/src/orchestrate.rs` now has a full `PlanRevision` pipeline:
`PlanRevisionReason::GateFailureLimit` is emitted after N consecutive gate failures (line ~5249),
`PlanRevisionClaim` provides dedup logic (lines 5193-5228), and the plan generator is re-invoked
with failure context to produce a replacement `tasks.toml` (`PlanRevisionOutcome::Regenerated`,
line ~5378). The orchestrator loop at line ~9154 reloads the DAG on successful regeneration.
Unit test `plan_revision_outcome` at line ~18658 confirms the flow. `RokoEvent::PlanRevision`
variant is wired in `roko-runtime/src/event_bus.rs`. Cross-ref item 89 now also DONE.

**Status**: DONE. Closes CLAUDE.md "What to work on" item 11.

---

### 07. [DONE] Wire `ScrollAccel` (exists but never instantiated)

**Resolved in**: `crates/roko-cli/src/tui/app.rs:396` — `App` now owns
`scroll_accel: super::scroll::ScrollAccel`. The accelerator is consumed at
`crates/roko-cli/src/tui/scroll.rs:72,78,89,99` via the standard PgUp/PgDn/G key
handlers.

**Original gap**: Implementation existed; zero call sites outside its own tests.

**Status**: ✅ DONE in T17 (PR #13).

---

### 08. [DONE] Persist adaptive gate thresholds across sessions (verify)

**Resolved in**: `AdaptiveThresholds::load_or_new(...)` is called at orchestrator init
(orchestrate.rs lines ~4667, ~4825, ~4985). The thresholds are saved after each gate run
(line ~5547) and consulted at dispatch time: `adaptive_thresholds.threshold_for(rung)` (line
~15357), `should_skip_rung()` (line ~15215), `suggested_max_retries()` (line ~7965),
`override_for_role()` (line ~15349). Neuro gate hints are applied via
`apply_neuro_gate_hints()` (line ~7634). Load and save paths both verified.

**Status**: DONE.

---

### 09. [DONE] Expose `GET /api/c-factor/trend` with historical rollup

**Resolved in**: `crates/roko-serve/src/routes/learning.rs` now has a `GET /c-factor/trend`
endpoint (line ~26) backed by `roko_learn::aggregate::cfactor_trend` (line ~65). Supports
`?window=24h` (default) and `?window=7d` via `parse_cfactor_trend_window()` (line ~150).
The `roko-core::dashboard_snapshot::DashboardSnapshot` also carries a `cfactor_trend` field
exposed via the projections route (line ~131). Unit tests cover default, 7d, and missing-file
scenarios (lines ~1447-1495).

**Status**: DONE.

---

### 10. [DONE] Call `roko_conductor::diagnosis` output into dashboard Diagnosis page

**Resolved in**: Diagnosis data is now visible in both TUI and HTTP:
- **TUI**: `dashboard_view.rs` renders a "Diagnosis" panel (line ~1020) with severity-colored
  rows via `diagnosis_rows()` (line ~2075). State carries `diagnoses: Vec<DiagnosisSummary>`
  (state.rs line ~784), populated from `DashboardSnapshot`.
- **HTTP**: `crates/roko-serve/src/routes/diagnosis.rs` exposes `GET /api/diagnosis/recent`.
Cross-ref item 84 now also DONE.

**Status**: DONE.

---

### 11. [DONE] Wire `roko-primitives::HdcFingerprint` into episode logger

**Resolved in**: `Episode` now has `pub hdc_fingerprint: Option<String>` (episode_logger.rs
line ~244). `crates/roko-learn/src/hdc_fingerprint.rs` provides `fingerprint_episode(prompt,
outcome)` with full base64 encode/decode. Integration test at episode_logger.rs line ~1267
verifies the round-trip. Cross-ref item 93 now also DONE.

**Status**: DONE.

---

### 12. [DONE] Add `roko plan validate` command (lint a plan without running it)

**Resolved in**: `crates/roko-cli/src/plan_validate.rs` implements a full `ValidationReport`
with `Diagnostic` entries (severity Error/Warning), per-plan `PlanDiagnostics`, and totals.
`crates/roko-cli/src/main.rs` registers the `PlanCmd::Validate { dir, strict, json }`
subcommand (line ~670), handled by `cmd_plan_validate()` (line ~3951) which calls
`plan_validate::validate_plans_dir()` with optional `--strict` and `--json` flags. Checks
DAG cycles via `roko_orchestrator::detect_cycle_nodes`, gate rung references, role templates,
and model profiles.

**Status**: DONE.

---

### 13. [DONE] Expose `/api/agents/{id}/logs` to stream sidecar logs via aggregator

**Resolved in**: `crates/roko-agent-server/src/features/logs.rs` implements a full
`GET /logs?tail=N` endpoint (default 200, max 2000). The handler reads the agent's log file
via `tail_file()`, scrubs sensitive content through `roko_core::obs::LogScrubber`, and returns
a `LogsResponse { lines, path }` JSON payload. The router is registered in
`crates/roko-agent-server/src/lib.rs`.

**Status**: DONE.

---

### 14. [DONE] Compact "agent topology" map widget in TUI

**Resolved in**: Full agent topology panel implemented across multiple files:
- `crates/roko-cli/src/tui/views/agents_view.rs` has `render_agent_topology_panel()` (line
  ~693), `build_agent_topology_lines()` (line ~790), and `topology_status_text()` (line ~757).
- `crates/roko-cli/src/tui/state.rs` carries `agent_topology: AgentTopology`,
  `agent_topology_status`, `agent_topology_visible`, and scroll state (lines ~795-851).
- `crates/roko-cli/src/tui/app.rs` has `request_agent_topology_refresh()` (line ~2547) which
  fetches from `roko-serve`'s `/api/agents/topology` endpoint via a background thread.
- Ctrl+T toggles the panel (input.rs line ~576). Status bar shows the keybinding.
- Tests at state.rs line ~3009 verify toggle/clamp behavior.

**Status**: DONE.
