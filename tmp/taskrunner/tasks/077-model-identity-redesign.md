# Task 077: Model Identity Redesign — Slugs, Backend Heuristic, Tier Field, Per-Model Iteration Cap

```toml
id = 77
title = "Model identity redesign: current slugs, remove backend heuristic, tier field, per-model iteration cap"
track = "config-foundation"
wave = "wave-1"
priority = "high"
blocked_by = [2]
touches = [
    "crates/roko-core/src/config/schema.rs",
    "crates/roko-core/src/config/provider.rs",
    "crates/roko-core/src/config/mod.rs",
    "crates/roko-core/src/config/agent.rs",
    "crates/roko-core/src/config/presets.rs",
    "crates/roko-core/src/agent.rs",
    "crates/roko-core/src/dispatch_plan.rs",
    "crates/roko-learn/src/cascade/helpers.rs",
    "crates/roko-learn/src/cost_table.rs",
    "crates/roko-agent/src/provider/mod.rs",
    "crates/roko-agent/src/provider/",
    "crates/roko-agent/src/gemini/",
    "crates/roko-agent/src/perplexity/",
    "crates/roko-agent/src/tool_loop/",
    "crates/roko-agent/src/dispatch_resolver.rs",
    "crates/roko-agent/src/translate/capability.rs",
]
exclusive_files = [
    "crates/roko-core/src/agent.rs",
    "crates/roko-core/src/config/presets.rs",
]
estimated_minutes = 240
```

## Context

Model identity has four accumulated problems that interact:

1. Default model slug strings scattered across config and presets still reference old
   `-3-5` and `-4-5` names. Some files use current slugs; others do not. Any place that
   hardcodes a slug that is not also in `roko-core::defaults` becomes a divergence point
   the next time a model generation increments.

2. `AgentBackend::from_model()` and the private `is_cursor_slug()` function still exist
   as unconstrained heuristics. They are called from `ModelSpec::from_slug()`,
   `resolve_model()` (fallback path), and `resolved_from_profile()` (missing-provider
   fallback). All three callers have config-authoritative paths that should be taken
   first; the heuristic should only survive as an explicitly deprecated emergency
   fallback, not as a silent default.

3. `ModelProfile` in `crates/roko-core/src/config/provider.rs` already has a `tier:
   Option<ModelTier>` field (added in an earlier batch), but `CascadeRouter` construction
   reads it via `with_model_tiers()`. The `slug_to_tier_heuristic()` in
   `crates/roko-learn/src/cascade/helpers.rs` uses `contains("haiku")` and similar
   substring matches as the sole fallback. At code-inspection time the workspace
   `roko.toml` model entries already have explicit `tier` fields. The fix here is not
   config population and not heuristic removal; it is to keep configured tiers on the
   authoritative path and leave the heuristic as a backstop only.

4. `tool_loop_max_iterations()` in `crates/roko-agent/src/provider/mod.rs` is a
   process-wide global that reads `ACTIVE_TEMPERAMENT` (a thread-local). No per-model
   override exists. A fast tier model (Haiku/Flash-Lite) that rarely uses many tools
   gets the same 50-iteration cap as Opus running a complex multi-step plan. Adding
   `max_tool_iterations: Option<u32>` to `ModelProfile` and reading it in
   `tool_loop_max_iterations()` allows per-model tuning via config with the existing
   temperament adjustment preserved as the final multiplier.

These four items must land together because they all touch the same config schema and
provider dispatch code. Splitting them across batches caused the current divergence.

## Background

Read these files before starting:

1. `crates/roko-core/src/config/provider.rs` — `ModelProfile` struct (lines 356-462).
   `tier: Option<ModelTier>` already exists. `max_tool_iterations` does not yet exist.
2. `crates/roko-core/src/config/presets.rs` — `minimal_config()` and `power_config()`
   preset functions. Count the hardcoded model strings.
3. `crates/roko-core/src/config/agent.rs` — `default_data_llm_model()` returns
   `"claude-haiku-4-5"`. `AgentConfig` fields like `fallback_model`.
4. `crates/roko-core/src/agent.rs` — `AgentBackend::from_model()` (line 154),
   `is_cursor_slug()` (line 187), `ModelSpec::from_slug()` (line 243),
   `resolve_model()` (line 319), `resolved_from_profile()` (line 338).
5. `crates/roko-learn/src/cascade/helpers.rs` — `slug_to_tier_heuristic()` (line 143),
   `default_role_model_table()` (line 27). Check which candidate slugs are stale.
6. `crates/roko-agent/src/provider/mod.rs` — `tool_loop_max_iterations()` (line 378)
   and `DEFAULT_MAX_TOOL_ITERATIONS` import from `roko_core::defaults`.
7. `crates/roko-core/src/defaults.rs` — `MODEL_FAST`, `MODEL_FOCUSED`, `MODEL_DEEP`
   constants at lines 304-313. These are the single source of truth for default slugs.
8. `roko.toml` at workspace root — verify the inspected model entries already have
   `tier =` lines. This file is read-only for this task.
9. `tmp/model-provider-audit.md` — §1 (stale slugs), §2 (backend heuristic), §4 (tier
   field) for the full context that motivated this task.

Current-code inspection notes:
- `roko.toml` already has `tier =` on the inspected model entries. Do not churn it. If a
  fresh grep finds a missing model entry, record that as a Status Log ambiguity/blocker rather
  than editing `roko.toml`; config population is a follow-up task.
- `tool_loop_max_iterations()` is called from
  `crates/roko-agent/src/provider/anthropic_api/tool_loop.rs`,
  `crates/roko-agent/src/provider/cerebras.rs`,
  `crates/roko-agent/src/provider/openai_compat.rs`,
  `crates/roko-agent/src/gemini/adapter.rs`, and
  `crates/roko-agent/src/perplexity/adapter.rs`. All five have a `&ModelProfile` available
  at the call site and should pass `Some(model)`.
- Adding a non-optional Rust field to `ModelProfile` will break every existing
  `ModelProfile { ... }` literal. Before editing, run:
  `rg -n "ModelProfile \\{" crates/roko-core/src crates/roko-agent/src crates/roko-learn/src`.
  Add `max_tool_iterations: None` to each literal or use an existing `..Default::default()`
  pattern where the local code already uses one. Do not leave compile-driven fixes to the end.

## What to Change

### 1. Update stale default model slugs

All default slug strings must reference `roko_core::defaults::MODEL_*` constants or
current model IDs. Do not introduce new hardcoded strings.

In `crates/roko-core/src/config/agent.rs`:
- `default_data_llm_model()` currently hardcodes `"claude-haiku-4-5"` even though that value
  is current. Replace the hardcoded string with `crate::defaults::MODEL_FAST.to_string()`.
  Also replace `default_model()`'s `"claude-sonnet-4-6"` with
  `crate::defaults::MODEL_FOCUSED.to_string()` so the default agent model follows the same
  source of truth.

In `crates/roko-core/src/config/presets.rs`:
- `minimal_config()` hardcodes `"claude-haiku-4-5"` for default_model, fast_task_model,
  standard_task_model, and auto_fix_model. Replace all with `MODEL_FAST`.
- `minimal_config()` hardcodes `"claude-sonnet-4-6"` for complex_task_model. Replace
  with `MODEL_FOCUSED`.
- `power_config()` hardcodes `"claude-opus-4-6"`, `"claude-sonnet-4-6"` slugs.
  Replace with `MODEL_DEEP` and `MODEL_FOCUSED` respectively.
- Both presets should import `crate::defaults::{MODEL_DEEP, MODEL_FOCUSED, MODEL_FAST}`.

In `crates/roko-learn/src/cascade/helpers.rs`:
- `default_role_model_table()` lists `"claude-sonnet-4-5"` in the Researcher fallback
  chain and `"claude-sonnet-4-5"` in the Standard tier fallback. Check if these are
  candidates for removal or replacement with current slugs. Do not add slugs that are
  not in the configured model list; the function already guards with `pick_static_slug`
  which only returns slugs present in `model_slugs`. Update the comment block to note
  that stale slugs simply fall through to the next candidate.

Verify by running:
```bash
grep -rn 'claude-haiku-3-5\|claude-sonnet-3-5\|claude-opus-3-5' crates/ --include='*.rs' | grep -v target/ | grep -v '#\[deprecated\]' | grep -v 'test'
```
There should be zero matches in production code after this change.

### 2. Quarantine `AgentBackend::from_model()` behind `#[deprecated]`

The authoritative path is `ProviderKind::to_backend()` via configured provider. The
heuristic in `from_model()` silently routes unconfigured slugs — which should be an
error as of batch 13. Mark the function deprecated so new callers get a compiler
warning.

In `crates/roko-core/src/agent.rs`:

```rust
#[deprecated(
    since = "0.1.0",
    note = "Resolve model via config and use ProviderKind::to_backend() instead. \
            This heuristic fires only for slugs not present in [models.*] config, \
            which is an error in production. Use resolve_model() and check that \
            resolved.profile is Some before dispatching."
)]
#[must_use]
pub fn from_model(slug: &str) -> Self {
    // ... existing body unchanged ...
}
```

Do NOT mark `is_cursor_slug()` deprecated separately and do NOT remove it. It remains a private
helper for the deprecated heuristic. The existing tests in `agent.rs` exercise both paths and
must remain green. The intent is to make new accidental callers of `AgentBackend::from_model()`
visible at compile time.

Update the three call sites in the same file:

- `ModelSpec::from_slug()` (line 245): add `#[allow(deprecated)]` with a comment:
  `// Legacy fallback: slug not in config. Call sites that know the slug is configured
  // should call resolve_model() and use the provider-authoritative backend instead.`
- `resolve_model()` fallback branch (line 319): same `#[allow(deprecated)]` with
  comment: `// Unconfigured slug — heuristic fallback. This path fires when a model
  key is not in config and must not be silently accepted in dispatch paths.`
- `resolved_from_profile()` missing-provider fallback (line 343): same treatment.

If `cargo test` or `cargo clippy --all-targets` reports deprecation warnings in tests that
intentionally exercise the legacy heuristic, suppress those individual tests or the test
module narrowly with a comment naming the compatibility assertion. Do not add a crate-level
`#![allow(deprecated)]`.

### 3. Add `max_tool_iterations` to `ModelProfile`

In `crates/roko-core/src/config/provider.rs`, add the field to `ModelProfile` after
`max_tools: Option<u32>`:

```rust
/// Per-model tool-loop iteration cap.
///
/// When set, overrides the workspace default (`DEFAULT_MAX_TOOL_ITERATIONS`)
/// before the temperament adjustment is applied. Use this to raise the cap for
/// models known to need many sequential tool calls (e.g. complex Opus plans) or
/// lower it for fast-tier models where runaway loops are more costly.
///
/// `None` means use the workspace default. The final cap is:
///   `(max_tool_iterations.unwrap_or(DEFAULT_MAX_TOOL_ITERATIONS) adjusted by temperament)`
#[serde(default, skip_serializing_if = "Option::is_none")]
pub max_tool_iterations: Option<u32>,
```

Place it immediately after `max_tools: Option<u32>` so schema readers see the two
related fields together.

### 4. Wire `max_tool_iterations` into `tool_loop_max_iterations()`

`tool_loop_max_iterations()` in `crates/roko-agent/src/provider/mod.rs` currently reads
only `DEFAULT_MAX_TOOL_ITERATIONS` and the thread-local temperament. It has no access
to the `ModelProfile` for the currently-dispatching model.

The correct wiring is:
- Add `tool_loop_max_iterations_for_profile(profile: Option<&ModelProfile>) -> usize`
  that reads `profile.and_then(|p| p.max_tool_iterations).map(|n| n as usize).unwrap_or(DEFAULT_MAX_TOOL_ITERATIONS)`
  and then applies the same temperament adjustment that `tool_loop_max_iterations()` applies.
- Keep the existing `tool_loop_max_iterations()` (no argument) as a call to
  `tool_loop_max_iterations_for_profile(None)` so all existing callers remain unbroken.
- Replace the zero-arg calls in the five inspected call sites:
  `provider/anthropic_api/tool_loop.rs`, `provider/cerebras.rs`,
  `provider/openai_compat.rs`, `gemini/adapter.rs`, and `perplexity/adapter.rs`.
  Each should call `tool_loop_max_iterations_for_profile(Some(model))`.
  Leave legacy/simple paths with no `ModelProfile` on `tool_loop_max_iterations()`.

The temperament adjustment logic must be identical between the two variants. Extract the
current formula from `tool_loop_max_iterations()` into a private
`apply_temperament_to_iteration_cap(base: usize) -> usize` helper:
Balanced returns `base`, Conservative adds `10`, Aggressive returns
`base.saturating_sub(15).max(10)`, and Exploratory adds `20`.

### 5. Update regression tests

- The existing test `tool_loop_iterations_derive_from_workspace_default` in
  `crates/roko-agent/src/provider/mod.rs` must still pass.
- Add a new test:
  ```rust
  #[test]
  fn tool_loop_iterations_respect_per_model_override() {
      // A profile with max_tool_iterations = 20 should yield 20 under Balanced.
      // Under Conservative it should yield 30 (20 + 10).
      // Under Aggressive it should yield max(10, 20 - 15) = 10.
      // Under Exploratory it should yield 40 (20 + 20).
  }
  ```
  Adjust expected values to match the exact temperament formula in the implementation.

- Add a test in `crates/roko-core/src/config/provider.rs` (or the existing config
  test suite) that confirms `ModelProfile` roundtrips through TOML with
  `max_tool_iterations = 25` correctly deserialized.

## What NOT to Do

- Do NOT remove `AgentBackend::from_model()` or `is_cursor_slug()`. The
  `AgentBackend::from_model()` deprecation attribute is sufficient. Removal requires auditing
  every downstream crate.
- Do NOT add `max_tool_iterations` to `roko.toml` model entries as part of this task.
  The field addition makes it available; population of `roko.toml` is a follow-up.
- Do NOT change the temperament adjustment formula. It is tested and the existing
  behavior must be preserved exactly.
- Do NOT touch `slug_to_tier_heuristic()` itself. It is the correct fallback. The fix
  for heuristic routing is ensuring `tier` is set in config, not removing the fallback.
- Do NOT add `tier` fields to `roko.toml` model entries as part of this task. That is
  separate config population work.
- Do NOT modify the `default_role_model_table()` candidate lists unless a slug is
  verifiably retired (not just absent from the project `roko.toml`). The function
  picks the first match from the configured model set; stale candidates fall through
  harmlessly.

## Wire Target

This task has no new CLI surface. The wire target is the provider dispatch path:

```bash
# Compile the affected crates cleanly
cargo check -p roko-core -p roko-agent -p roko-learn

# Run existing iteration-cap tests
cargo test -p roko-agent tool_loop_iterations -- --nocapture

# Run new per-model override test
cargo test -p roko-agent tool_loop_iterations_respect_per_model_override -- --nocapture

# Confirm no stale 3-5 slugs remain in production code
grep -rn 'claude-haiku-3-5\|claude-sonnet-3-5\|claude-opus-3-5' crates/ --include='*.rs' | grep -v target/ | grep -v test
```

## Verification

- [ ] `cargo build --workspace` — clean build, no errors
- [ ] `cargo test --workspace` — no regressions
- [ ] `cargo clippy --workspace --no-deps -- -D warnings` — clean (deprecated
  attribute triggers `deprecated_member_use` at the three call sites; suppress each
  with `#[allow(deprecated)]` and a comment, not a blanket crate-level allow)
- [ ] `cargo test -p roko-agent tool_loop_iterations` — both old and new tests pass
- [ ] `cargo test -p roko-core model_profile_` — TOML roundtrip for
  `max_tool_iterations` passes
- [ ] `grep -rn 'claude-haiku-3-5\|claude-sonnet-3-5\|claude-opus-3-5' crates/ --include='*.rs' | grep -v target/ | grep -v test` —
  zero matches in production code
- [ ] `rg -n -B1 'AgentBackend::from_model' crates/roko-core/src/agent.rs` —
  the three intentional production fallback call sites have a nearby `#[allow(deprecated)]`
  and comment; no new call sites exist elsewhere
- [ ] `grep -rn 'max_tool_iterations' crates/roko-core/src/config/provider.rs` —
  field is present in `ModelProfile`
- [ ] `rg -n 'tool_loop_max_iterations_for_profile' crates/roko-agent/src/provider crates/roko-agent/src/gemini crates/roko-agent/src/perplexity` —
  helper exists and all five profile-aware provider adapters call it

## Status Log

| Time | Agent | Action |
|------|-------|--------|
