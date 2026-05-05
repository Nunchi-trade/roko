# Task 081: Error Type Hierarchy

```toml
id = 81
title = "Define thiserror enums per crate; replace anyhow::Result in library crate public APIs"
track = "infrastructure"
wave = "wave-2"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-agent/src/error.rs",
    "crates/roko-agent/src/lib.rs",
    "crates/roko-agent/src/provider/mod.rs",
    "crates/roko-agent/src/tool_loop/mod.rs",
    "crates/roko-gate/src/error.rs",
    "crates/roko-gate/src/lib.rs",
    "crates/roko-gate/src/generated.rs",
    "crates/roko-learn/src/error.rs",
    "crates/roko-learn/src/lib.rs",
    "crates/roko-learn/src/cascade_router.rs",
    "crates/roko-learn/src/episode_logger.rs",
    "crates/roko-compose/Cargo.toml",
    "crates/roko-compose/src/error.rs",
    "crates/roko-compose/src/lib.rs",
]
exclusive_files = [
    "crates/roko-agent/src/error.rs",
    "crates/roko-gate/src/error.rs",
    "crates/roko-learn/src/error.rs",
    "crates/roko-compose/src/error.rs",
]
estimated_minutes = 360
```

## Context

S6.6 in the infrastructure audit identified that error handling is inconsistent across the
codebase: some functions return `anyhow::Result`, others return custom error enums, others
return `Box<dyn Error>`. There is no consistent error hierarchy for library crates.

The current state:
- `roko-agent` has `LlmError`, `ProviderError`, `AgentCreationError`; `LlmError` and
  `AgentCreationError` already derive `thiserror`, while `ProviderError` currently has a manual
  `Display`/`Error` impl. They are scattered across `tool_loop/mod.rs` and `provider/mod.rs`
  with no single crate-level error type.
- `roko-gate` exports `GateError = roko_core::RokoError` (a type alias, not a real enum).
- `roko-learn` has no crate-level error type; internal functions return `Result<_, std::io::Error>`
  or `Result<_, serde_json::Error>` directly.
- `roko-compose` uses `anyhow::Result` in `conventions.rs`.
- `roko-serve` has `ApiError` (an HTTP response type) but no server-side `ServeError` for
  internal logic.

This task defines a proper `thiserror` enum per crate. It does NOT convert every `anyhow`
callsite inside those crates — only public API functions (those with `pub fn` or `pub async fn`
signatures that are called from outside the crate) must return typed errors.

`anyhow::Result` remains correct at the CLI boundary (`roko-cli/src/`) and in internal
helper functions where the caller doesn't need to match on error variants.

Checklist item: S6.6.

## Background

Read these files before starting:

1. `crates/roko-agent/src/tool_loop/mod.rs` — existing `LlmError` at line 138, `LlmBackend`
   trait at line 80. `LlmError` is already well-designed; it just needs to be re-exported from
   a canonical `error.rs`.
2. `crates/roko-agent/src/provider/mod.rs` — existing `ProviderError` (line 558) and
   `AgentCreationError` (line 629). Re-export these from `error.rs`; do not move definitions
   unless you update every local import in the same patch.
3. `crates/roko-gate/src/generated.rs` — `GateError = roko_core::RokoError` (line 19). This
   is a type alias that hides gate-specific context. Replace with a real enum.
4. `crates/roko-gate/src/lib.rs` — existing exports; `GateError` re-exported from `generated`.
5. `crates/roko-compose/src/conventions.rs` — uses `anyhow` in one file; find the public fns.
6. `tmp/infrastructure-audit.md` section 6.6 — the full problem description and proposed fix.
7. `tmp/redesign-plan.md` Phase 1 — error types are a Phase 1 foundation change.

## What to Change

### 1. Create `crates/roko-agent/src/error.rs`

This file consolidates all public-facing agent error types. Do NOT move internal helper
errors (types that never appear in a `pub fn` signature).

```rust
//! Crate-level error types for roko-agent.
//!
//! Public API functions return these typed errors. Internal helpers
//! may use `anyhow` or local error types freely.

pub use crate::provider::{AgentCreationError, ProviderError};
pub use crate::tool_loop::LlmError;

/// Top-level error for any agent dispatch operation visible to callers
/// outside this crate.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// Agent could not be constructed from the given config.
    #[error("agent creation failed: {0}")]
    Creation(#[from] AgentCreationError),

    /// The LLM backend returned an error during a turn.
    #[error("llm backend error: {0}")]
    Backend(#[from] LlmError),

    /// A provider-level error (rate limit, auth failure, etc.)
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),

    /// Tool dispatch failed (bad args, permission denied, tool panicked).
    #[error("tool dispatch error: {0}")]
    ToolDispatch(String),

    /// Safety contract rejected the operation.
    #[error("safety contract violation: {0}")]
    SafetyViolation(String),

    /// Generic catch-all for errors that don't fit above categories.
    /// New callers should prefer adding a typed variant rather than using this.
    #[error("{0}")]
    Other(String),
}
```

Then add to `crates/roko-agent/src/lib.rs`:
```rust
pub mod error;
pub use error::AgentError;
```

### 2. Create `crates/roko-gate/src/error.rs`

The current `GateError = roko_core::RokoError` alias loses all gate-specific context. Replace it:

```rust
//! Crate-level error types for roko-gate.

/// Error from a gate or gate pipeline.
#[derive(Debug, thiserror::Error)]
pub enum GateError {
    /// The gate subprocess exited with a non-zero status.
    #[error("gate command failed (exit {code}): {stderr}")]
    CommandFailed { code: i32, stderr: String },

    /// Gate could not spawn the required subprocess.
    #[error("gate spawn failed: {0}")]
    SpawnFailed(String),

    /// Gate input payload was missing a required field.
    #[error("invalid gate payload: {0}")]
    InvalidPayload(String),

    /// Gate threshold was exceeded; verdict is Fail.
    #[error("gate threshold exceeded: {metric} = {value:.3} (threshold: {threshold:.3})")]
    ThresholdExceeded { metric: String, value: f64, threshold: f64 },

    /// I/O error while reading/writing gate artifacts.
    #[error("gate I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization/deserialization error for gate verdicts.
    #[error("gate serialize error: {0}")]
    Serialize(#[from] serde_json::Error),

    /// Generic catch-all. Prefer adding typed variants above.
    #[error("{0}")]
    Other(String),
}
```

In `crates/roko-gate/src/lib.rs`, replace the line:
```rust
// BEFORE
pub use generated::{GateError, GateGenerator, GeneratedCheck};
```
with:
```rust
// AFTER
pub use error::GateError;
pub use generated::{GateGenerator, GeneratedCheck};
```

And add `pub mod error;` to the module declarations.

**Important**: `GateGenerator::generate()` currently returns `GateError = roko_core::RokoError`.
Update the return type to `crate::error::GateError`. Check every `impl GateGenerator` block.

### 3. Create `crates/roko-learn/src/error.rs`

```rust
//! Crate-level error types for roko-learn.

/// Error from a learning subsystem operation.
#[derive(Debug, thiserror::Error)]
pub enum LearnError {
    /// Persistent state file could not be read or written.
    #[error("learn state I/O error at {path}: {source}")]
    Io { path: String, #[source] source: std::io::Error },

    /// JSON parsing or serialization failed for persisted learning data.
    #[error("learn state parse error: {0}")]
    Parse(#[from] serde_json::Error),

    /// A router or bandit state file was corrupt. The subsystem should
    /// reset to defaults rather than propagating this error upward.
    #[error("learn state corrupt at {path}: {reason}")]
    Corrupt { path: String, reason: String },

    /// Generic catch-all. Prefer adding typed variants above.
    #[error("{0}")]
    Other(String),
}
```

Add to `crates/roko-learn/src/lib.rs`:
```rust
pub mod error;
pub use error::LearnError;
```

Note: roko-learn's public API surface is small. The most important public functions to
migrate are in `cascade_router.rs` (`save`, `load`) and `episode_logger.rs` (`append`).
Those currently return `std::io::Error` directly. Change them to `LearnError`.

### 4. Create `crates/roko-compose/src/error.rs`

```rust
//! Crate-level error types for roko-compose.

/// Error from prompt assembly or composition operations.
#[derive(Debug, thiserror::Error)]
pub enum ComposeError {
    /// Template rendering failed.
    #[error("template render failed: {0}")]
    Template(String),

    /// Enrichment client returned an error.
    #[error("enrichment error: {0}")]
    Enrichment(String),

    /// Token counter exceeded a budget constraint.
    #[error("token budget exceeded: used {used}, budget {budget}")]
    TokenBudgetExceeded { used: usize, budget: usize },

    /// Generic catch-all.
    #[error("{0}")]
    Other(String),
}
```

Add to `crates/roko-compose/src/lib.rs`:
```rust
pub mod error;
pub use error::ComposeError;
```

`anyhow` is only used in `conventions.rs`. That file is internal analysis code with no
public functions called from outside the crate. Leave it as-is; do not replace it.

### 5. Update public API signatures

After creating the error types, update the return types of PUBLIC functions only:

For roko-agent — these are the key public functions:
```rust
// crates/roko-agent/src/provider/mod.rs
// BEFORE
pub fn create_agent_for_model(...) -> Result<Box<dyn Agent>, AgentCreationError>
// AFTER (AgentCreationError stays; AgentError wraps it at a higher call level)
// No change needed here — AgentCreationError is already a typed error.
```

For roko-gate — check `GateGenerator::generate()` return type:
```rust
// crates/roko-gate/src/generated.rs
// BEFORE
pub type GateError = roko_core::RokoError;
// AFTER: delete this line; use crate::error::GateError instead
```

For roko-learn — update `cascade_router.rs`:
```rust
// BEFORE
pub fn save(&self, path: &Path) -> Result<(), std::io::Error>
// AFTER
pub fn save(&self, path: &Path) -> Result<(), crate::error::LearnError>
```

### 6. Do NOT touch internal functions

Many internal helper functions return `anyhow::Result` or bare `std::io::Error`. These are
fine as-is. The goal is typed errors at the crate boundary, not throughout every internal
helper. Do not change:
- Private functions (`fn foo(...)`, not `pub fn`)
- Functions only called within the same crate
- Test helper functions

## Current Tree Notes and Mechanical Plan

The original touch list was incomplete for the described work. Use the `touches` list above,
not the older prose-only list: `generated.rs`, `cascade_router.rs`, `episode_logger.rs`,
`provider/mod.rs`, `tool_loop/mod.rs`, and `roko-compose/Cargo.toml` are required if the
public signatures or derives are changed.

Current facts to verify before editing:
- `crates/roko-agent/src/provider/mod.rs` defines `ProviderError` manually with `Display` +
  `std::error::Error`, and `AgentCreationError` already derives `thiserror::Error`.
- `crates/roko-agent/src/tool_loop/mod.rs` defines `LlmError` as a public `thiserror` enum.
- `crates/roko-gate/src/generated.rs` still exports `pub type GateError = roko_core::RokoError`.
- `crates/roko-learn/src/episode_logger.rs` already has public `LoggerError`; do not delete it.
  `LearnError` should wrap it with `Logger(#[from] crate::episode_logger::LoggerError)` unless
  the implementation deliberately migrates every external caller.
- `crates/roko-compose/Cargo.toml` currently has no direct `thiserror` dependency; add
  `thiserror = { workspace = true }` before deriving `ComposeError`.

Ordered implementation steps:
1. Add each crate `error.rs` and export it from that crate's `lib.rs`.
2. In `roko-agent`, re-export existing public error types from `error.rs`; do not move their
   definitions unless every local `use` is updated in the same patch.
3. In `roko-gate`, remove the `GateError` type alias from `generated.rs`, import
   `crate::error::GateError`, and keep `GateGenerator::generate()` returning
   `Result<Vec<GeneratedCheck>, GateError>`.
4. In `roko-learn`, add `LearnError` and migrate only public persistence boundaries whose
   callers can be updated in the same patch. At minimum cover `CascadeRouter::save()` and
   `CascadeRouter::from_snapshot_json()` with path-aware `Io`/`Parse` conversions; for
   `EpisodeLogger`, either keep `LoggerError` as the narrow public error and expose
   `impl From<LoggerError> for LearnError`, or update every external callsite that uses
   `EpisodeLogger::{append,read_all,read_all_lossy,compact}`.
5. In `roko-compose`, add `ComposeError` and export it; do not change
   `conventions::detect_conventions()` because it returns a concrete struct and does not use
   `anyhow` despite comments mentioning the pattern it detects.
6. Run greps before declaring the migration complete:
   `rg -n "pub (async )?fn .*anyhow::Result|pub (async )?fn .*Result<.*anyhow" crates/roko-{agent,gate,learn,compose}/src`
   and `rg -n "GateError = roko_core::RokoError" crates/roko-gate/src`.

CLI/runtime call chain for audit:
- Configured agents: `roko-cli` command -> `roko_agent::provider::create_agent_for_model()` ->
  provider adapter -> typed `AgentCreationError` / `AgentError`.
- Generated gate checks: runner gate dispatch -> `roko_gate::generated::GateGenerator::generate()`
  -> typed `GateError`.
- Learning persistence: runtime feedback / serve / ACP callsites -> `EpisodeLogger` or
  `CascadeRouter` public persistence methods -> `LearnError` or wrapped `LoggerError`.

Tests to add or update:
- `roko-agent`: compile-only/public API test imports `roko_agent::{AgentError, LlmError}` and
  converts `AgentCreationError` into `AgentError`.
- `roko-gate`: unit test calls a dummy `GateGenerator` and verifies the error type name is
  `roko_gate::GateError`, not `roko_core::RokoError`.
- `roko-learn`: tests for `LearnError::Io { path, .. }` and corrupt cascade snapshot parse
  mapping.
- `roko-compose`: compile test imports `roko_compose::ComposeError`.

## What NOT to Do

- Do NOT add `anyhow` as a dependency to crates that don't already have it (`roko-agent`,
  `roko-gate`, `roko-learn` currently have no `anyhow` in Cargo.toml — keep it that way).
- Do NOT create a single global `RokoError` mega-enum. The architecture calls for per-crate
  errors that compose naturally via `#[from]`.
- Do NOT convert internal helpers. Only change `pub fn` and `pub async fn` signatures.
- Do NOT change `roko-serve` in this task. `roko-serve` already has `ApiError` for HTTP
  responses. A `ServeError` for internal logic would require HTTP handler changes that are
  out of scope here.
- Do NOT add `ServeError` to the touches list; `roko-serve` is intentionally excluded.
- Do NOT remove the existing `LlmError` or `ProviderError` from their current locations —
  just re-export them from `error.rs`. Moving them causes churn across all internal callers.

## Wire Target

This task is library-level only; there is no CLI wire target. The wire target is compilation:

```bash
cargo build --workspace
# All crates that import roko-agent, roko-gate, roko-learn, roko-compose must still compile.
```

And the new types should be accessible:
```bash
grep -rn 'roko_agent::AgentError\|roko_gate::GateError\|roko_learn::LearnError' crates/ --include='*.rs' | grep -v target/
# Should show callers using the new canonical types
```

## Verification

- [ ] `cargo build --workspace` — all crates compile
- [ ] `cargo test --workspace` — no regressions
- [ ] `cargo clippy --workspace --no-deps -- -D warnings` — clean
- [ ] `crates/roko-agent/src/error.rs` exists and exports `AgentError`
- [ ] `crates/roko-gate/src/error.rs` exists and exports `GateError`
- [ ] `crates/roko-learn/src/error.rs` exists and exports `LearnError`
- [ ] `crates/roko-compose/src/error.rs` exists and exports `ComposeError`
- [ ] `grep -n 'GateError = roko_core' crates/roko-gate/src/generated.rs | wc -l` returns 0
  (the type alias is gone)
- [ ] `grep -rn 'use anyhow' crates/roko-gate/src/ --include='*.rs' | grep -v target/` returns
  empty (roko-gate has no anyhow dependency and must not start using it)
- [ ] `grep -rn 'use anyhow' crates/roko-learn/src/ --include='*.rs' | grep -v target/` returns
  empty (same constraint for roko-learn)
- [ ] `grep -rn 'use anyhow' crates/roko-agent/src/ --include='*.rs' | grep -v target/` returns
  empty (roko-agent has no anyhow in Cargo.toml)

## Status Log

| Time | Agent | Action |
|------|-------|--------|
