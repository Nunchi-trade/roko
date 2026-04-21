# Safety Enforcement & Learning-Loop Closure

> **New file** added 2026-04-16 during post-PR-13 audit. The two **P0** items
> here close the self-hosting loop that CLAUDE.md "What to work on" lines 10–11
> still flag as the last missing pieces. The remaining items consolidate
> safety / playbook / enrichment cross-references that previously lived
> across files 02, 05, 11.
>
> **Re-audit 2026-04-20**: All 7 items closed (89, 90, 91, 92, 93, 94, 95).
> 0 items still open.

## Summary

Seven items spanning safety contract enforcement and the closure of the
plan→execute→gate→learn→replan loop. Items 89 and 90 are the canonical
self-hosting blockers; everything else here is supporting infrastructure.

## Items

### 89. [DONE] **P0** — Gate feedback → plan generator prompt augmentation

**Resolved in**: Full `PlanRevision` pipeline wired in orchestrate.rs:
- `RokoEvent::PlanRevision` with `PlanRevisionReason::GateFailureLimit { attempts }` emitted
  after N consecutive gate failures (line ~5249-5252).
- `PlanRevisionClaim` enum (line ~3193) provides dedup + cap logic (lines ~5207-5228).
- `PlanRevisionOutcome` enum (line ~3200) tracks Disabled/NotEligible/Duplicate/CapReached/
  Regenerated/RegenerationFailed outcomes.
- The plan generator is re-invoked with failure context at line ~5369-5380.
- The orchestrator loop reloads the DAG on `Regenerated` (line ~9154).
- Unit test at line ~18658 verifies the flow.
Cross-ref item 06 now also DONE.

**Status**: DONE. Closes CLAUDE.md "What to work on" item 11.

---

### 90. [DONE] **P0** — PRD-publish event → orchestrator auto-trigger

**Resolved in**: Full orchestrator-side PRD-publish wiring:
- `RokoEvent::PrdPublished { slug, path, origin, published_at }` on the event bus
  (event_bus.rs line ~123) with round-trip test (line ~444).
- CLI promotes emit the event at prd.rs line ~672 via `global_event_bus().emit()`.
- `roko-serve/routes/prds.rs` has full handling:
  `append_prd_published_episode()` (line ~78),
  `handle_prd_published_event()` (line ~156) invokes `prd plan` + `plan run`,
  `follow_prd_published_audit()` (line ~186) watches the episode log for external promotes.
- `[serve] auto_orchestrate` config flag at schema.rs line ~3116 (default true).
- Integration test at `crates/roko-serve/tests/prd_publish.rs`.

**Status**: DONE. Closes CLAUDE.md "What to work on" item 10.

---

### 91. [DONE] Agent safety contracts defined but unenforced

**Resolved in**: `SafetyLayer` now loads and enforces `AgentContract` per dispatch:
`contract_for_role()` (safety/mod.rs line ~864) loads role-specific contracts,
`check_pre_execution()` calls `self.contract.check_pre_execution(call, ctx)` at step 8
(line ~407-409). Governance rules and invariants are checked declaratively.
Cross-ref item 35d now also DONE.

**Status**: DONE.

---

### 92. [DONE] Role-based tool whitelist not enforced

**Resolved in**: `SafetyLayer` now enforces per-role tool whitelists via
`role_tools: HashMap<String, ToolWhitelist>` (safety/mod.rs line ~207), built from
`config.agent.roles` via `build_role_tools()` (line ~893). `check_pre_execution()` checks
the whitelist at line ~327-331 and returns `ToolError::PermissionDenied` for unauthorized tools.
`roko.toml` schema supports `[agent.<role>]` overrides with tools, thresholds, budget, and
routing_overrides via `RoleOverride` (schema.rs line ~2022).
Cross-ref item 35e now also DONE.

**Status**: DONE.

---

### 93. [DONE] HDC fingerprint per-episode wiring

**Resolved in**: `Episode` now has `pub hdc_fingerprint: Option<String>` (episode_logger.rs
line ~244). `crates/roko-learn/src/hdc_fingerprint.rs` provides `fingerprint_episode(prompt,
outcome)` using `roko_primitives::hdc::fingerprint()`, plus base64 `encode()` / `decode()`.
Integration test at episode_logger.rs line ~1267 verifies the full round-trip through JSONL
append and read. Cross-ref items 11 and 30 now also DONE.

**Status**: DONE.

---

### 94. [DONE] Playbook store query integration

**Resolved in**: `crates/roko-cli/src/orchestrate.rs` now queries the playbook store at
pre-dispatch time:
- `playbook_query()` (line ~2778) builds the query from task metadata.
- `playbook_query_context()` (line ~2799) provides the full context.
- At line ~13280-13281: `let relevant_playbooks = match self.playbook.query(&playbook_query).await`.
- Relevant playbooks are passed through to prompt building (line ~13622, ~17041) and
  logged with count (line ~13668-13670).
Cross-ref item 35b now also DONE.

**Status**: DONE.

---

### 95. [DONE] Enrichment pipeline call-site decision

**Resolved in**: Wired at the orchestrator pre-dispatch phase. `orchestrate.rs` imports and
calls the enrichment pipeline extensively: `run_enrichment_pipeline()` (line ~8326) with
complexity estimation, step selection, and backend resolution. Enrichment artifacts are
injected into task context at line ~13739-13761. The enriching phase
(`build_enrichment_system_prompt`, line ~17627) runs before agent dispatch with full plan
context. Cross-ref item 29 now also DONE.

**Status**: DONE.
