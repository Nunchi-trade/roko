# Task 028: Verify orchestrate.rs Feature Gate is Clean

```toml
id = 28
title = "Verify and clean up orchestrate.rs feature gate -- no non-gated references"
track = "cleanup"
wave = "wave-1"
priority = "low"
blocked_by = []
touches = [
    "crates/roko-cli/src/lib.rs",
]
exclusive_files = []
estimated_minutes = 30
```

## Context

`orchestrate.rs` (23K+ lines) is behind `#[cfg(feature = "legacy-orchestrate")]` and should
not be compiled by default. The module declaration in `lib.rs` is correctly gated, and the
`pub use` re-export is also gated. But there may be stale references elsewhere in roko-cli
that mention types from orchestrate.rs without the feature gate, which would cause compile
errors if those types are ever removed.

This is a hygiene task: verify the gate is airtight, document what you find, and fix any
leaks.

Sources:
- `tmp/v2-refactoring/CHECKLIST.md` -- QW-4
- `tmp/v2-refactoring/03-QUICK-WINS.md` -- QW-4

## Background

Read these files first:
1. `crates/roko-cli/src/lib.rs` -- lines 88-89 (mod declaration) and 141-142 (pub use)
2. `crates/roko-cli/Cargo.toml` -- feature definition (line 16)
3. `crates/roko-cli/src/run.rs` -- uses `#[cfg(feature = "legacy-orchestrate")]` in multiple places
4. `tmp/v2-refactoring/00-INDEX.md` -- active paths are `roko run`, `roko plan run`, and `roko serve`
5. `tmp/v2-refactoring/10-DEAD-CODE-AUDIT.md` -- orchestrate is legacy/deletion candidate

Current-state details:

- `crates/roko-cli/Cargo.toml` has `default = []` and
  `legacy-orchestrate = ["legacy-direct-dispatch"]`.
- `crates/roko-cli/src/lib.rs` gates both `pub mod orchestrate;` and the
  `pub use orchestrate::{OrchestrationReport, PlanRunReport, PlanRunner};` re-export.
- `crates/roko-cli/src/run.rs` has many `#[cfg(feature = "legacy-orchestrate")]` sections and
  a default-feature stub for `run_once` that returns an error.
- `crates/roko-cli/tests/snapshot.rs` imports `roko_cli::orchestrate`, but `Cargo.toml` marks
  that test with `required-features = ["legacy-orchestrate"]`.
- Many grep hits are comments, string literals, tracing labels, or names from the separate
  `roko_orchestrator` crate; those are not leaks.

## What to Change

1. **Grep for all references to `orchestrate`** in `crates/roko-cli/src/`:
   ```bash
   grep -rn 'orchestrate' crates/roko-cli/src/ --include='*.rs' | grep -v target/ | grep -v '//.*orchestrate'
   ```

2. **For each reference found**:
   - If it's a `mod orchestrate` or `use orchestrate::` -- verify it's behind `#[cfg(feature = "legacy-orchestrate")]`
   - If it's a doc comment or string mentioning orchestrate.rs -- that's fine, leave it
   - If it's code that imports types from orchestrate without a feature gate -- add the gate
   Use a second, narrower grep to avoid false positives from `roko_orchestrator`:

   ```bash
   rg -n 'use crate::orchestrate|pub use orchestrate|pub mod orchestrate|roko_cli::orchestrate|PlanRunner|PlanRunReport|OrchestrationReport' crates/roko-cli/src crates/roko-cli/tests --glob '*.rs'
   ```

3. **Verify the build compiles without the feature**:
   ```bash
   cargo build -p roko-cli
   ```
   This should succeed (it's the default build). If it fails due to missing types from
   orchestrate, you've found a leak.

4. **Document findings** in the Status Log, including the grep output showing all
   references and whether each is properly gated.

5. Only edit `crates/roko-cli/src/lib.rs` for this task unless the task metadata is expanded.
   If a true leak appears in `run.rs`, tests, or another source file, record it as a blocker
   instead of editing outside `touches`.

Mechanical decision table:

| Reference kind | Action |
|----------------|--------|
| `#[cfg(feature = "legacy-orchestrate")] pub mod orchestrate;` | leave as-is |
| `#[cfg(feature = "legacy-orchestrate")] pub use orchestrate::{...};` | leave as-is |
| `use crate::orchestrate::...` outside orchestrate.rs | must be gated or reported as blocker |
| `roko_cli::orchestrate::...` in tests | allowed only when Cargo test target has `required-features = ["legacy-orchestrate"]` |
| comments/docstrings/string literals mentioning orchestrate | leave as-is |
| `roko_orchestrator::...` imports/types | not related to this feature gate |

## What NOT to Do

- Don't delete orchestrate.rs -- it's legacy but still compilable for comparison.
- Don't refactor orchestrate.rs internals.
- Don't change any feature flag names.
- Don't replace Runner v2 references with orchestrate references.
- Don't gate unrelated `roko_orchestrator` crate imports.
- Don't edit `Cargo.toml` unless the snapshot test's `required-features` entry is missing.
- Don't treat comments like `extracted from orchestrate.rs` as leaks.

## Wire Target

```bash
# This is cleanup -- verify by building without the feature:
cargo build -p roko-cli
# And verify WITH the feature still compiles:
cargo build -p roko-cli --features legacy-orchestrate
```

Expected observable behavior:

- Default `cargo build -p roko-cli` does not compile `src/orchestrate.rs`.
- Feature-enabled build still compiles legacy code for comparison/testing.
- Non-comment references to the `orchestrate` module outside `src/orchestrate.rs` are either
  `#[cfg(feature = "legacy-orchestrate")]` gated or live only in a required-feature test target.

## Verification

- [ ] `cargo build -p roko-cli` (without legacy-orchestrate feature)
- [ ] `cargo build -p roko-cli --features legacy-orchestrate`
- [ ] `rg -n 'use crate::orchestrate|pub use orchestrate|pub mod orchestrate|roko_cli::orchestrate|PlanRunner|PlanRunReport|OrchestrationReport' crates/roko-cli/src crates/roko-cli/tests --glob '*.rs'`
      shows only gated code or required-feature tests
- [ ] All `orchestrate` references in non-comment code are behind `#[cfg(feature = "legacy-orchestrate")]`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
