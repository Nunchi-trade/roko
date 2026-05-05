# Task 089: Orchestration Cleanup + Model String Removal

```toml
id = 89
title = "Replace raw rung integers, fix worktree config loading, address skipped rungs, delete resolve_enrichment_backend, remove hardcoded model strings"
track = "cleanup"
wave = "wave-2"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-cli/src/orchestrate.rs",
    "crates/roko-cli/src/runner/event_loop.rs",
    "crates/roko-core/src/config/schema.rs",
]
exclusive_files = []
estimated_minutes = 300
```

## Context

This task is a REDESIGN sweep across orchestration code that has accumulated fragile patterns:
raw integer rung comparisons that silently break when the enum shifts, config loading from
worktree exec_dir (which has no roko.toml), three gate rungs permanently skipped with stale
batch references, a completely separate routing system with wrong substring matching, and
hardcoded model strings in active runner paths.

Sources:
- `tmp/infrastructure-audit.md` §13.7 (worktree config), §13.10 (skipped rungs), §13.12 (rung integers)
- `tmp/model-provider-audit.md` §5 (resolve_enrichment_backend)
- `tmp/redesign-plan.md` Phase 4.7 (hardcoded model strings in active runner)

**IMPORTANT**: `crates/roko-cli/src/orchestrate.rs` is behind the `legacy-orchestrate` feature
gate. For items that only touch `orchestrate.rs`, either delete clearly broken dead code or
annotate as legacy-only. The primary target for rung integers and model strings is the ACTIVE
runner at `crates/roko-cli/src/runner/`.

## Background

Read these files before making any changes:

1. `crates/roko-gate/src/rung_selector.rs`:
   - `Rung` enum (lines 108–117): `Compile=0, Lint=1, Test=2, Symbol=3, GeneratedTest=4,
     PropertyTest=5, Integration=6`. These indices are serialized to disk — do not reorder.
   - `Rung::as_index()` (lines 138–141): converts enum to `u32`.
   - `Rung::from_index()` (lines 150–157): converts `u32` back to `Option<Rung>`.
   - `CANONICAL_ORDER` slice (lines 120–128): the 7 rungs in execution order.
   - `GateCapabilities` and `allows()` (lines 199–219): capability-based rung filtering already
     exists — use this instead of the permanent "T1-11" skip.

2. `crates/roko-cli/src/orchestrate.rs` (legacy, `#[cfg(feature = "legacy-orchestrate")]`):
   - Lines 1998–2006: `fn resolve_enrichment_backend()` — maps provider kind strings using
     substring matching. Maps `"gemini_api"` → `EnrichmentLlmBackend::Codex` (wrong semantics:
     Codex is OpenAI's old name, has nothing to do with Gemini). Call site at line 9241.
     Test at lines 20850–20872 (`enrichment_backend_uses_provider_kind`).
   - Lines 17875–17894: Symbol (rung 3), PropertyTest (rung 5), Integration (rung 6) are
     permanently skipped with `tracing::debug!(rung = 3, "... pending (T1-11)")`.
   - Lines 18071, 18074, 18083, 18087, 18093, 18124, 18130: raw `u32` comparisons `if rung == 5`,
     `if rung > 6`, etc. in `gate_rung_config()` and `enrich_rung_config()`.
   - Line 1734 (approx): `load_roko_config(&cfg.exec_dir)` — exec_dir is a worktree path with
     no `roko.toml`. Config silently falls through to `unwrap_or_default()`.

3. `crates/roko-cli/src/runner/gate_dispatch.rs`:
   - Line 31: `rung: u32` parameter — the active runner passes rung as a raw `u32` through
     this function. Sentinel values `RUNG_PLAN_VERIFY = 1000` and `RUNG_MERGE = 1001` exist
     (lines 23–25) as raw constants.
   - No raw `if rung == X` comparisons exist here — the pipeline builder handles rung selection.

4. `crates/roko-cli/src/runner/event_loop.rs`:
   - Lines 133–135: `gate_timeout()` matches raw `u32` (0, 1, _) for compile/clippy/test — this
     IS a raw integer comparison but it is intentional (3 timeout tiers, not rung semantics).
     Check whether this should use `Rung::as_index()` or stay as-is.
   - Search for any other `rung ==` or `rung >` comparisons in this file.

5. `crates/roko-core/src/config/schema.rs`:
   - Check whether `[agent.defaults]` section exists with `generic_agent_model` or similar.
     This is where model key references should live (redesign-plan Phase 4.7).

6. `crates/roko-core/src/defaults.rs`:
   - `MODEL_DEEP`, `MODEL_FOCUSED`, `MODEL_FAST`, `MODEL_ESCALATION_LADDER` are already defined
     here (added in batch 5, 2026-05-03). Use these constants — do NOT add new ones that
     duplicate them.

## What to Change

### 1. Replace raw `u32` rung comparisons with named constants or `Rung` enum

**Why**: If the `Rung` enum indices shift, `if rung == 5` silently uses the wrong gate.
Named constants are the minimum fix; converting to enum is preferred (S13.12).

In `crates/roko-cli/src/orchestrate.rs` (legacy path), in `gate_rung_config()` (lines
18071–18093) and `enrich_rung_config()` (lines 18124–18130):

Replace all raw `u32` literals with `Rung` enum calls:

```rust
// Before (line 18071):
if rung == 5 {
    config.fact_check_min_confidence = Some(nominal);
}
if rung == 6 {
    config.llm_judge_min_score = Some(nominal as f32);
}
if rung == 3 || rung > 6 {
    config.source_roots = Some(vec![self.workdir.clone()]);
}

// After:
use roko_gate::rung_selector::Rung;
if rung == Rung::PropertyTest.as_index() {
    config.fact_check_min_confidence = Some(nominal);
}
if rung == Rung::Integration.as_index() {
    config.llm_judge_min_score = Some(nominal as f32);
}
if rung == Rung::Symbol.as_index() || rung > Rung::Integration.as_index() {
    config.source_roots = Some(vec![self.workdir.clone()]);
}
```

For the "run all rungs" sentinel (`if rung > 6`, line 18243), replace with:
```rust
if rung > Rung::Integration.as_index() {
```

In `crates/roko-cli/src/runner/event_loop.rs`, the `gate_timeout()` function (lines 130–135)
matches `0` (compile) and `1` (clippy). These are intentional tier groupings, not arbitrary
magic numbers, but they should still be named:

```rust
fn gate_timeout(config: &RunConfig, rung: u32) -> Duration {
    config.roko_config.as_deref().map_or_else(
        || Duration::from_secs(config.timeout_secs),
        |cfg| {
            if rung == Rung::Compile.as_index() {
                cfg.timeouts.gate_compile()
            } else if rung == Rung::Lint.as_index() {
                cfg.timeouts.gate_clippy()
            } else {
                cfg.timeouts.gate_test()
            }
        },
    )
}
```

### 2. Fix config loading from worktree `exec_dir`

**Why**: `load_roko_config(&cfg.exec_dir)` in `orchestrate.rs` loads config from the per-task
worktree directory, which has no `roko.toml`. All user-configured provider routing is silently
ignored for parallel tasks (S13.7).

In `crates/roko-cli/src/orchestrate.rs` (legacy), find all call sites of `load_roko_config`
that use `&cfg.exec_dir` or a task-scoped path. Replace with the project root:

```rust
// Before:
let routing_config = load_roko_config(&cfg.exec_dir).unwrap_or_default();

// After:
// exec_dir is the worktree checkout; roko.toml lives at project root.
let project_root = &self.workdir;
let routing_config = load_roko_config(project_root).unwrap_or_default();
```

For the active runner: verify that all config loading in `event_loop.rs` and `gate_dispatch.rs`
uses `config.workdir` (the project root passed in `RunConfig`), not any task-level path. Grep
for `load_roko_config` in `crates/roko-cli/src/runner/` and confirm all call sites use the
project root. If any use a task-scoped path, fix them.

### 3. Address permanently-skipped rungs

**Why**: Symbol (3), PropertyTest (5), and Integration (6) rungs are skipped with `tracing::debug!`
messages referencing "T1-11" — a closed sprint batch that no longer means anything (S13.10).
The rungs ARE implemented in `rung_dispatch.rs` and yield `stub_verdict` when their inputs are
not wired. This is acceptable behavior; the active runner already handles it correctly.

**Redesign for orchestrate.rs (legacy)**:

Replace the permanent-skip match arms with capability-gated dispatch using the existing
`GateCapabilities` mechanism:

```rust
// Before (orchestrate.rs lines 17875–17894):
Rung::Symbol => {
    tracing::debug!(rung = 3, "Symbol gate skipped: capability detection pending (T1-11)");
    skipped_count = skipped_count.saturating_add(1);
}
Rung::PropertyTest => {
    tracing::debug!(rung = 5, "PropertyTest gate skipped: ...");
    skipped_count = skipped_count.saturating_add(1);
}
Rung::Integration => {
    tracing::debug!(rung = 6, "Integration gate skipped: ...");
    skipped_count = skipped_count.saturating_add(1);
}

// After:
// These rungs use stub_verdict fallback when inputs are not wired.
// Wire them through run_gate_rung() like Compile/Lint/Test.
// If capability detection later determines the rung is unavailable, GateCapabilities
// will filter it before reaching this match arm.
Rung::Symbol | Rung::PropertyTest | Rung::Integration => {
    let verdicts = self.run_gate_rung(Some(plan_id), &payload_sig, rung.as_index()).await;
    steps.extend(/* convert verdicts to steps */);
}
```

If wiring these rungs through `run_gate_rung` in the legacy path would require significant
refactoring, the minimum acceptable fix is: delete the "T1-11" references and replace with a
config-gated skip:

```rust
Rung::Symbol | Rung::PropertyTest | Rung::Integration => {
    // These rungs require capability detection wiring.
    // They are skipped until [gates.enable_advanced_rungs = true] is set.
    if self.config.gates.enable_advanced_rungs.unwrap_or(false) {
        let verdicts = self.run_gate_rung(Some(plan_id), &payload_sig, rung.as_index()).await;
        // handle verdicts
    } else {
        tracing::debug!(?rung, "advanced rung skipped (gates.enable_advanced_rungs not set)");
        skipped_count = skipped_count.saturating_add(1);
    }
}
```

If adding `enable_advanced_rungs` to config, add it to `GatesConfig` in
`crates/roko-core/src/config/schema.rs`:

```rust
/// Enable advanced gate rungs (Symbol, PropertyTest, Integration).
/// These rungs use stub_verdict fallback when their input manifests are not wired.
#[serde(default)]
pub enable_advanced_rungs: bool,
```

### 4. Delete `resolve_enrichment_backend()` from `orchestrate.rs`

**Why**: This is a completely separate model-routing system with wrong semantics.
`"gemini_api"` maps to `EnrichmentLlmBackend::Codex` — Codex is an OpenAI product name,
has nothing to do with Gemini. The enrichment pipeline should route through
`create_agent_for_model()` like all other dispatch (model-audit §5).

In `crates/roko-cli/src/orchestrate.rs`:
- Delete `fn resolve_enrichment_backend()` (lines 1998–2006).
- Find its call site at `run_enrichment_pipeline()` (line 9241):
  ```rust
  // Before:
  let backend = resolve_enrichment_backend(&provider_kind);

  // After:
  // Drop `backend` and `provider_kind` variables entirely.
  // The enrichment pipeline dispatches through the standard agent factory.
  // If the enrichment pipeline takes a backend enum, replace with the model key
  // and let create_agent_for_model() resolve the provider.
  ```
- Delete the test at lines 20850–20872 (`enrichment_backend_uses_provider_kind`).
- If `EnrichmentLlmBackend` is only used by `resolve_enrichment_backend` and its call site,
  delete it too. If used elsewhere, leave it and just remove this mapping function.

The enrichment pipeline (`run_enrichment_pipeline`) should use `self.effective_model()` and
route through the standard agent factory. The `provider_kind` variable that was passed to
`resolve_enrichment_backend` is itself loaded via `resolve_model(&cfg, &model)` — use that
model key directly with `create_agent_for_model()` instead.

### 5. Remove hardcoded model strings from active runner paths

**Why**: Model string literals in Rust source become stale when providers release new versions.
The constants in `roko-core/src/defaults.rs` (`MODEL_FOCUSED`, `MODEL_FAST`, `MODEL_DEEP`,
`MODEL_ESCALATION_LADDER`) were added to fix this in orchestrate.rs — apply the same fix to
active runner paths (redesign-plan Phase 4.7).

First, search for literal model strings in the active runner:

```bash
grep -rn '"claude-\|"sonnet\|"haiku\|"opus' crates/roko-cli/src/runner/ --include='*.rs'
```

For each hit:
- If the string is in production dispatch code, replace with the appropriate constant from
  `roko_core::defaults` (`MODEL_FOCUSED`, `MODEL_FAST`, `MODEL_DEEP`).
- If the string is in a test or example, leave it — tests use specific strings intentionally.

Also check `crates/roko-core/src/config/schema.rs` for whether `[agent.defaults]` has a
`generic_agent_model` field. The redesign-plan Phase 4.7 target is:

```toml
# roko.toml
[agent.defaults]
generic_agent_model = "sonnet"   # references a [models.*] key, not a raw slug
```

```rust
// In active runner dispatch:
let model_key = config.roko_config
    .as_deref()
    .and_then(|c| c.agent.defaults.generic_agent_model.as_deref())
    .unwrap_or(roko_core::defaults::MODEL_FOCUSED);
```

If `agent.defaults` does not exist in the schema, add the `AgentDefaults` struct to
`crates/roko-core/src/config/schema.rs`:

```rust
/// Default agent behavior overrides. All fields are optional config-key references.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentDefaults {
    /// Default model key for generic agent dispatch (references [models.*]).
    #[serde(default)]
    pub generic_agent_model: Option<String>,
    /// Model key for gate judging.
    #[serde(default)]
    pub gate_judge_model: Option<String>,
}
```

And add `pub defaults: AgentDefaults` to `AgentConfig`.

Do NOT move all model references to config in this task — that is Phase 4.7 scope. The minimum
for this task is: no raw model string literals in the active runner production paths.

## What NOT to Do

- Do NOT change the `Rung` enum integer values (`Compile=0` through `Integration=6`). They are
  serialized to disk in gate attempt sentinel files and snapshot state.
- Do NOT remove the `legacy-orchestrate` feature gate from `orchestrate.rs`. Annotate legacy
  code as legacy, do not remove it.
- Do NOT implement the full Symbol/PropertyTest/Integration gate capability detection. The
  minimum is removing the stale "T1-11" references and replacing with config-gated or
  stub-tolerant dispatch.
- Do NOT change the active runner's gate pipeline selection logic in `gate_dispatch.rs` beyond
  the named-constant fix for rung comparisons.
- Do NOT add `EnrichmentLlmBackend` variants or extend the enrichment backend system. The goal
  is to DELETE it, not expand it.
- Do NOT update `roko.toml` model keys in this task. Config changes belong to a separate
  migration pass.

## Wire Target

```bash
# Verify no raw rung integer magic numbers remain in the active runner
grep -rn 'rung == [0-9]\|rung > [0-9]\|rung < [0-9]' \
  crates/roko-cli/src/runner/ --include='*.rs'
# Expect: zero results (gate_timeout's 0/1 should use enum constants)

# Verify resolve_enrichment_backend is gone
grep -n 'resolve_enrichment_backend' crates/roko-cli/src/orchestrate.rs
# Expect: zero results

# Verify no raw model strings in active runner production paths
grep -rn '"claude-\|"sonnet\|"haiku\|"opus' crates/roko-cli/src/runner/ --include='*.rs' \
  | grep -v '#\[cfg(test\|mod tests'
# Expect: zero results

# Run the plan runner to confirm it still works
cargo run -p roko-cli -- plan run plans/ --workdir . --dry-run 2>&1 | head -30
```

## Verification

- [ ] `cargo build --workspace` — clean build
- [ ] `cargo test --workspace` — no regressions
- [ ] `cargo clippy --workspace --no-deps -- -D warnings` — clean
- [ ] All `if rung == X` comparisons in `orchestrate.rs` use `Rung::Variant.as_index()`
- [ ] `gate_timeout()` in `event_loop.rs` uses `Rung::Compile.as_index()` and
  `Rung::Lint.as_index()` instead of bare `0` and `1`
- [ ] `fn resolve_enrichment_backend()` is deleted from `orchestrate.rs`
- [ ] `enrichment_backend_uses_provider_kind` test is deleted
- [ ] No `"T1-11"` references remain in `orchestrate.rs`
- [ ] Symbol/PropertyTest/Integration rungs in legacy path are either wired through
  `run_gate_rung()` or gated by `gates.enable_advanced_rungs` config
- [ ] No raw model string literals in `crates/roko-cli/src/runner/` production code
- [ ] `load_roko_config` in legacy path uses project root, not `exec_dir`

## Worker 17 Mechanical Notes

### Current code facts to use

- `crates/roko-gate/src/rung_selector.rs` defines `Rung`, `CANONICAL_ORDER`,
  `Rung::as_index()`, and `Rung::from_index()`. The capability struct is named
  `RungCaps`, not `GateCapabilities`.
- Active runner raw rung checks are limited to:
  `event_loop.rs::gate_timeout()` matching `0`/`1`, and comparisons against
  `config.max_gate_rung` that are threshold semantics and should stay numeric.
- Legacy `orchestrate.rs` still has:
  - `load_roko_config(&cfg.exec_dir)` in the enrichment worker path,
  - `resolve_enrichment_backend()` and the
    `enrichment_backend_uses_provider_kind` test,
  - stale `"T1-11"` skip messages for Symbol/PropertyTest/Integration,
  - raw rung numbers in `gate_rung_config()`, `enrich_rung_config()`,
    `run_gate_rung()`, and the `if rung == 0` selected-pipeline special case.
- Active runner production hardcoded model strings are currently in
  `crates/roko-cli/src/runner/types.rs::RunConfig::from_roko_config()` and
  `impl Default for RunConfig`. That file is not in this task's `touches`
  list, so the touch list is insufficient for the "no raw model strings in
  active runner" acceptance criterion.
- `AgentConfig` lives in `crates/roko-core/src/config/agent.rs`, and
  `GatesConfig` lives in `crates/roko-core/src/config/gates.rs`; both are
  re-exported through `schema.rs`. If an implementation needs to add
  `AgentDefaults` or `enable_advanced_rungs`, the current `touches` list is
  also insufficient.

### Mechanical implementation order

1. Start with the active runner:
   - In `runner/event_loop.rs`, import `roko_gate::rung_selector::Rung` and
     replace `0`/`1` in `gate_timeout()` with
     `Rung::Compile.as_index()` and `Rung::Lint.as_index()`.
   - Leave `completion.rung < config.max_gate_rung` and
     `rung >= ctx.config.max_gate_rung` alone; those compare against a user
     configured threshold, not a specific rung variant.
2. Fix legacy rung literals in `orchestrate.rs`:
   - Replace every `rung == 0/3/4/5/6` and `rung > 6` with
     `Rung::<Variant>.as_index()`.
   - Include the `if rung == 0` branch in `run_gate_rung()`; use
     `Rung::Compile.as_index()`.
   - For loops over all built-ins, prefer `for current_rung in
     CANONICAL_ORDER.map(Rung::as_index)` if import shape allows; otherwise use
     `0..=Rung::Integration.as_index()`.
3. Fix config loading in legacy enrichment:
   - The `WorkerConfig`/enrichment worker call at `load_roko_config(&cfg.exec_dir)`
     should use the project root carried by the runner (`self.workdir`) or an
     explicit `project_root` field in that config. Do not load provider routing
     from per-plan/per-task worktrees.
4. Remove `resolve_enrichment_backend()`:
   - Delete the function and its unit test.
   - In `run_enrichment_pipeline()`, stop resolving a provider kind just to map
     to `EnrichmentLlmBackend`.
   - If `EnrichmentRuntimeClient` still requires a `backend` argument through
     the enrichment crate API, pass the model key/slug through the standard
     `create_agent_for_model()` path instead of inventing a new enum mapping.
     Do not add Gemini/Perplexity/Cerebras variants to `EnrichmentLlmBackend`.
5. Address skipped advanced rungs:
   - Best fix: wire `Rung::Symbol`, `Rung::PropertyTest`, and
     `Rung::Integration` through the existing `run_gate_rung()` path, relying
     on `RungCaps` selection and stub verdicts where inputs are missing.
   - Minimum fix: remove all `"T1-11"` references and gate advanced-rung
     dispatch behind a config flag. If a config flag is needed, add it in
     `config/gates.rs`, not `schema.rs`.
6. Hardcoded model strings:
   - Replace production defaults in `runner/types.rs` with
     `roko_core::defaults::MODEL_FOCUSED`.
   - Test fixtures such as `runner/agent_stream.rs` may keep explicit strings.

### Verification greps

```bash
rg -n 'rung == [0-9]|rung > [0-9]|rung < [0-9]' \
  crates/roko-cli/src/runner crates/roko-cli/src/orchestrate.rs -g '*.rs'
# Expected: no variant-specific raw literals. max_gate_rung threshold
# comparisons are allowed if they are not against a literal.

rg -n 'T1-11|resolve_enrichment_backend|enrichment_backend_uses_provider_kind' \
  crates/roko-cli/src/orchestrate.rs
# Expected: no matches.

rg -n 'load_roko_config\\(&cfg\\.exec_dir\\)' crates/roko-cli/src/orchestrate.rs
# Expected: no matches.

rg -n '"claude-|\"sonnet|\"haiku|\"opus' crates/roko-cli/src/runner -g '*.rs' \
  | rg -v 'cfg\\(test\\)|mod tests|agent_stream.rs'
# Expected: no production matches after the touch list is expanded to include
# runner/types.rs.
```

### Touch-list ambiguity

The task cannot satisfy its own active-runner model-string acceptance criterion
without editing `crates/roko-cli/src/runner/types.rs`. It also cannot add the
optional config fields in the real files without touching
`crates/roko-core/src/config/agent.rs` or `crates/roko-core/src/config/gates.rs`.
An implementation agent should request a touch-list expansion before making
those changes; do not hide them in `schema.rs`.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
