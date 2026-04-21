# Partially-Wired Subsystems — "Compiles but Doesn't Run in Prod"

> **Status (post-PR-13)**: 7 original items still open + 5 new items appended
> from the audit sweep. Refreshed 2026-04-16.
>
> **Re-audit 2026-04-20**: 8 more items closed (29, 30, 31, 35, 35b, 35c, 35d, 35e).
> 4 items still open (32, 33, 34, 35a).

## Summary

Cases where the crate builds, has unit tests, and is listed as "Wired" or
"Built" in CLAUDE.md — but a grep for the exported types shows either zero
external callers, or only test-module callers, or callers that never fire in
the default CLI-driven workflow. Each is a chance to delete dead code *or* to
add the one missing call-site that activates the feature.

## Items

### 29. [DONE] `roko-compose::enrichment::*` — self-contained, no external call-sites

**Resolved in**: `crates/roko-cli/src/orchestrate.rs` now imports and calls the enrichment
pipeline extensively: `use roko_compose::enrichment::{ ... StepOutcome, StepSelector,
estimate_enrichment }` (line ~32-35). A full `run_enrichment_pipeline()` method (line ~8326)
orchestrates the pipeline per plan, with `selected_enrichment_steps()` (line ~1805),
`resolve_enrichment_backend()` (line ~1809), and `render_enrichment_artifact_context()` (line
~1870). Enrichment artifacts are injected into task context at line ~13739-13761 via
`apply_section_effectiveness_to_prompt_section()`. Cross-ref item 95 now also DONE.

**Status**: DONE.

---

### 30. [DONE] `roko-primitives::HdcFingerprint` — built, no per-episode call-sites

**Resolved in**: `crates/roko-learn/src/episode_logger.rs` now has
`pub hdc_fingerprint: Option<String>` on the `Episode` struct (line ~244).
`crates/roko-learn/src/hdc_fingerprint.rs` provides `fingerprint_episode(prompt, outcome)`
which uses `roko_primitives::hdc::fingerprint()`, plus `encode()` / `decode()` for base64
round-tripping. Integration test `hdc_fingerprint_round_trips_through_jsonl_append_and_read()`
at line ~1267 confirms the full flow. Cross-ref items 11 and 93 now also DONE.

**Status**: DONE.

---

### 31. [DONE] `roko-conductor::diagnosis` — called, output invisible

**Resolved in**: Diagnoses are now visible in both the TUI and HTTP API:
- **TUI**: `crates/roko-cli/src/tui/views/dashboard_view.rs` renders a "Diagnosis" panel
  (line ~1020) with `diagnosis_rows()` (line ~2075) showing severity, message, and timestamp
  per `DiagnosisSummary`. State carried in `tui/state.rs` (line ~784).
- **HTTP**: `crates/roko-serve/src/routes/diagnosis.rs` exposes `GET /api/diagnosis/recent`
  (line ~17). OpenAPI tags include "diagnosis" (openapi.rs line ~51).
Cross-ref items 10 and 84 now also DONE.

**Status**: DONE.

---

### 32. `roko-dreams::imagination` — Phase-2 module, but tests run in CI

**Evidence**: `crates/roko-dreams/src/` contains `imagination.rs`,
`hypnagogia.rs`, `cycle.rs`. `roko-dreams` is listed as Phase 2+ in CLAUDE.md,
so tests run but no production call-site exists.

**Current state**: Module compiles, has tests, runs in CI — wastes ~30 s of CI
time per commit for code that's by policy untouched until Phase 2.

**Gap**: Either (a) move `roko-dreams` behind a workspace feature gate that
defaults off, or (b) accept the CI cost as intentional.

**Fix scope**: 2 hours to feature-gate.

**Priority**: P1.

---

### 33. `roko-daimon` / `roko-chain` — not in default build output

**Evidence**: `ls crates/` shows these dirs but CLAUDE.md's "Key crates" table
marks them "Phase 2+". The default `cargo build --workspace` builds them
anyway (workspace members). Note: `roko-golem` is NOT in the current crate
tree — naming was retired; renamed/folded as part of the bardo→roko migration.

**Current state**: Same shape as #32. Build cost without runtime benefit.

**Gap**: Move to `tools/` or an opt-in workspace slice.

**Fix scope**: 1 day. Cargo.toml workspace reshuffle.

**Priority**: P2 (cosmetic CI cost only).

---

### 34. MCP server crates — coverage / ship-gate audit

**Evidence**: Four `roko-mcp-*` crates in addition to `roko-mcp-code`:

| Crate | LOC (`src/main.rs` or `lib.rs`) | Wired in CLAUDE.md "Key crates"? |
|-------|--------------------------------|---------------------------------|
| `roko-mcp-github`  | 2 643 | partial (`roko-mcp-*` row) |
| `roko-mcp-slack`   |   920 | partial |
| `roko-mcp-scripts` |   767 | partial |
| `roko-mcp-stdio`   |   246 (`lib.rs`) | partial |

PR #13 advertises `roko-mcp-code` as "wired"; the others have no
ship-gate evidence (no integration test, no documented dispatch path, no
default `mcp_config` entry).

**Current state**: Crates compile; runtime status unknown per crate.

**Gap**: Audit each for "minimum viable MCP server" semantics. Decide which to
ship by default, which to gate behind opt-in `roko.toml` keys, and which to
deprecate.

**Fix scope**: 1 day audit + 1–3 days per crate to finish or document.

**Priority**: P1 (ship-gate for any release that advertises broad MCP coverage).

---

### 35. [DONE] `roko-core::obs::health` + `obs::metrics` — limited exposure

**Resolved in**: A canonical `MetricSchema` trait now lives at
`crates/roko-core/src/obs/schema.rs` (line ~63) with `CanonicalMetricSchema` (line ~72) as
the shared implementation. `crates/roko-agent-server/src/state.rs` imports and uses the same
canonical schema (line ~24, referencing `CanonicalMetricSchema::schema_version()`). A dedicated
test at `crates/roko-core/tests/metric_schema.rs` (line ~78:
`agent_server_metrics_use_canonical_schema_constants`) explicitly verifies that the agent-server
metrics use the canonical schema constants, preventing drift. Cross-ref item 86 now also DONE.

**Status**: DONE.

---

### 35a. Gate pipeline — 4 of 7 rungs unwired in `run_gate_rung`

**Evidence**: `crates/roko-cli/src/orchestrate.rs:11423-11461`
(`async fn run_gate_rung`). Only Compile / Test / Clippy gates are dispatched.
The other gates documented in CLAUDE.md (FactCheck, Symbol, GeneratedTest,
PropertyTest, VerifyChain, LlmJudge, Integration) exist as crates / structs
under `crates/roko-gate/src/` but `run_gate_rung` never calls them; rung values
≥ 3 fall into the catch-all `_ => { compile + test + clippy }` arm.

**Current state**: 11-gate / 7-rung pipeline advertised; only the first 3
gates fire in production.

**Gap**: Either (a) extend `run_gate_rung` to dispatch the rest of the gates
per-rung, or (b) downgrade the CLAUDE.md / PRD claims to "3 gates wired,
others Phase 2+".

**Fix scope**: 2 days for full wiring; 5 minutes for the doc downgrade.

**Priority**: **P1** (silent feature gap — users believe they have stronger
verification than they do).

---

### 35b. [DONE] Playbook store loaded but never queried

**Resolved in**: `orchestrate.rs` now queries the playbook store at pre-dispatch time:
`playbook_query()` (line ~2778), `playbook_query_context()` (line ~2799), and the
query call at line ~13281: `self.playbook.query(&playbook_query).await`. Results fed to
prompt building (line ~13622) and logged (line ~13668-13670). Cross-ref item 94 now DONE.

**Status**: DONE.

---

### 35c. [DONE] Verdicts persisted via `FileSubstrate` but no readers

**Resolved in**: Multiple consumers now read `Kind::GateVerdict`:
- TUI `verdicts.rs` has a `VerdictsAggregator` with incremental substrate queries.
- `roko-serve/routes/status.rs` (line ~1828) reads gate verdicts.
- `roko-conductor` watchers (stuck_pattern.rs, test_failure_budget.rs, ghost_turn.rs)
  all consume `Kind::GateVerdict` engrams.
- `roko-learn/verdict_scorer.rs` scores verdict engrams.
- Dashboard renders per-gate trend grids from the aggregated data.
Cross-ref item 83 now also DONE.

**Status**: DONE.

---

### 35d. [DONE] Agent safety contracts defined but unenforced

**Resolved in**: `AgentContract` is now fully wired into the `SafetyLayer`:
- `SafetyLayer` owns a `pub contract: AgentContract` field (safety/mod.rs line ~203).
- `contract_for_role()` (line ~864) loads role-specific contracts via
  `AgentContract::load_for_role(role)`, falling back to `AgentContract::permissive()`.
- `check_pre_execution()` (line ~324) enforces the contract at step 8 (line ~407-409):
  `self.contract.check_pre_execution(call, ctx).map_err(|violation| violation.into_tool_error())`
- `AgentContract` itself supports `load_for_role()` with per-role invariants and governance
  rules (contract.rs lines ~528-530 test implementer/reviewer/researcher).
Cross-ref item 91 now also DONE.

**Status**: DONE.

---

### 35e. [DONE] Role-based tool access control not wired

**Resolved in**: `SafetyLayer` now enforces per-role tool whitelists:
- `role_tools: HashMap<String, ToolWhitelist>` field (safety/mod.rs line ~207).
- `build_role_tools()` (line ~893) constructs the whitelist map from
  `config.agent.roles` (`RoleOverride` structs in `roko.toml`).
- `check_pre_execution()` (line ~327-331) checks `self.role_tools.get(&self.role)`
  and returns `ToolError::PermissionDenied` if the tool is not in the whitelist.
- `roko-core/src/config/schema.rs` defines the `RoleOverride` struct (line ~2022) with
  `routing_overrides`, thresholds, budget, and tool lists consumed at runtime.
Cross-ref item 92 now also DONE.

**Status**: DONE.
