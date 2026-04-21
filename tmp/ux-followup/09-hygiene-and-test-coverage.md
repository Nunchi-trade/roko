# Hygiene & Test Coverage

> **Status (post-PR-13)**: item 57 closed; 5 originals open; 5 new items
> appended. Refreshed 2026-04-16.
>
> **Re-audit 2026-04-20**: 7 more items closed (55, 59, 60, 60a, 60b, 60d, 60e).
> 3 items still open (56, 58, 60c).

## Summary

Hygiene items — unchecked `unwrap()` density, missing unit tests, CI flakes,
clippy warnings that are silenced rather than fixed, schema gaps. Not urgent
individually but a long-term drag on velocity. Grouped here so a "hardening
batch" (file 11 recommends T26 + T32) can pick them up in one run.

## Items

### 55. [DONE] Unwrap density — top-10 hottest files

**Resolved in**: The two worst offenders have been cleaned up:
- `system_prompt_builder.rs`: grep now shows 0 unwrap() calls (was 20).
- `routes/middleware.rs`: grep now shows 0 unwrap() calls (was 22).
The remaining files (payload.rs, event_bus.rs, metrics.rs, adaptive_threshold.rs,
artifact_store.rs, learning.rs, symbol_resolver.rs, status.rs) still carry their original
small counts but the top-10 total has dropped from 70 to ~28.

**Status**: DONE (primary hotspots cleaned; residual low-count files remain as minor hygiene).

---

### 56. `clippy::missing_errors_doc` / `missing_panics_doc` — suppressed, not fixed

**Evidence**: Several crates enable `#[allow(clippy::missing_errors_doc)]` at
the crate level. Search with `grep -rn 'clippy::missing_' crates/`.

**Current state**: Workspace clippy is `-D warnings` at gate-time, but
per-crate allow lists mask docs debt.

**Gap**: Add the missing `# Errors` / `# Panics` sections, then remove the allow.

**Fix scope**: 2–3 days grind.

**Priority**: P1.

---

### 57. [DONE] Missing integration tests for `roko-agent-server` messaging

**Resolved in**: Commit `c9029e20`
(`tui-parity(T19): Agent-server messaging integration tests`). Adds
`crates/roko-agent-server/tests/messaging.rs` with the four required scenarios:
mock dispatcher, missing dispatcher (503), dispatch error (502), streaming
chunks.

**Status**: ✅ DONE.

---

### 58. Flaky tests — timeout-based assertions

**Evidence**: Several backends have `with_timeout_ms(100)` in tests
(e.g. `crates/roko-agent/src/exec.rs:504`). On slow CI these can spuriously
pass or fail depending on scheduler noise.

**Current state**: No quarantine list; flakes show up in runner retries.

**Gap**: Either mock the clock (tokio `time::pause`) or bump the timeout 10×
for CI runs via a `CI=true` env guard.

**Fix scope**: 1 day audit + fix.

**Priority**: P1.

---

### 59. [DONE] Coverage measurement not in CI

**Resolved in**: `.github/workflows/coverage.yml` now runs `cargo llvm-cov --html` with
`--ignore-filename-regex` for tests/target/testdata/benches, and produces both an HTML report
(`target/llvm-cov`) and a JSON summary (`coverage-summary.txt`). Both are uploaded as CI
artifacts (`coverage-html`, `coverage-summary`). Uses `taiki-e/install-action@cargo-llvm-cov`.

**Status**: DONE.

---

### 60. [DONE] No end-to-end smoke test of the self-hosting loop

**Resolved in**: `crates/roko-cli/tests/e2e_self_host.rs` implements a full
`self_hosting_workflow_with_mock_dispatcher()` test that exercises:
`roko init` -> `prd idea` -> `prd draft new` -> `research enhance-prd` ->
`prd draft promote` -> verifies `.roko/prd/published/` materialization,
`prd_published` episode entries in `.roko/episodes.jsonl` (line ~93), and
auto-plan generation. Uses `assert_cmd::Command` with a tempdir.

**Status**: DONE.

---

### 60a. [DONE] HTTP routes lack request validation / OpenAPI

**Resolved in**: `crates/roko-serve/src/openapi.rs` now generates a full OpenAPI document
via `utoipa::OpenApi` (line ~28: `#[derive(OpenApi)]`) with 14 named tag groups covering
all route categories (status, plans, run, templates, deployments, agents, research, config,
subscriptions, prds, webhooks, providers, learning, aggregator, diagnosis). Served at
`GET /api/openapi.json` (line ~21). A unified `ApiError` type lives at
`crates/roko-serve/src/error.rs`. Integration tests at
`crates/roko-serve/tests/api_integration.rs` verify the surface.

**Status**: DONE (OpenAPI + error shaping landed; per-route validator crate integration
is a follow-up refinement).

---

### 60b. [DONE] `Episode` struct missing explicit backend / dispatcher field

**Resolved in**: `crates/roko-learn/src/episode_logger.rs` now has
`pub backend: String` on the `Episode` struct (line ~202). This field preserves which
dispatch backend (claude_cli, claude_api, codex, cursor, openai_compat, ollama, etc.)
was used for each episode, enabling cascade analysis to distinguish between backends
even when the model name is the same.

**Status**: DONE.

---

### 60c. Cascade router selection untested end-to-end

**Cross-ref**: see item 40a in `06-advanced-agent-backends.md`. Listed here
to keep the hygiene-batch worklist self-contained.

**Priority**: P1.

---

### 60d. [DONE] Resume snapshot schema lacks version field

**Resolved in**: `ExecutorSnapshot` now has a `schema_version` field (verified at
orchestrate.rs line ~734-738 with `debug_assert_eq!(snapshot.schema_version, CURRENT_SCHEMA_VERSION)`).
`crates/roko-cli/src/snapshot_migrate.rs` implements the full migration framework: reads
`schema_version` from raw JSON (line ~12), applies per-version upgrades in a loop (line ~44-67),
and handles v0 (missing field, default to zero) through v2. Tests at lines ~118, ~137, ~155
verify v1->v2 upgrade, v2 passthrough, and v0 default handling. Cross-ref items 79 and 81
now also DONE.

**Status**: DONE.

---

### 60e. [DONE] ProcessSupervisor zombie cleanup incomplete

**Resolved in**: `crates/roko-runtime/src/process.rs` now has:
(a) SIGTERM-then-SIGKILL escalation in `shutdown()` (lines ~326-344): sends `Signal::SIGTERM`,
    waits for graceful exit, then escalates to `force_kill()`.
(b) `impl Drop for ProcessSupervisor` (line ~708-729) that logs a warning and calls
    `force_kill_sync()` on all live children.
(c) `CancellationToken` plumbed through the supervisor (line ~80, accessor at line ~348).
Cross-ref item 80 now also DONE.

**Status**: DONE.
