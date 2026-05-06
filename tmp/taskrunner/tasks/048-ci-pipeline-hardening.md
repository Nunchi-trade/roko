# Task 048: Harden CI Pipeline and Test Infrastructure

```toml
id = 48
title = "Pin rustc in CI, add integration test job, ensure port 0 everywhere"
track = "infrastructure"
wave = "wave-1"
priority = "high"
blocked_by = []
touches = [
    ".github/workflows/ci.yml",
    "rust-toolchain.toml",
    "crates/roko-serve/src/lib.rs",
    "crates/roko-cli/tests/common/mod.rs",
    "crates/roko-cli/tests/smoke.rs",
]
exclusive_files = [
    ".github/workflows/ci.yml",
    "rust-toolchain.toml",
]
estimated_minutes = 90
```

## Context

The audit (S8, S10) identified CI reliability issues:
1. CI uses `@stable` which drifts — different lints on local vs CI.
2. No separate integration test job. Integration tests that touch ports/files mix
   with unit tests causing flakiness.
3. `rust-toolchain.toml` exists but only says `channel = "stable"` — no pinned version.
4. Some serve tests already use port 0 (good), but the pattern should be enforced.

## Background

Read:
- `.github/workflows/ci.yml` — current CI pipeline
- `rust-toolchain.toml` — current toolchain pin
- `crates/roko-serve/tests/` — existing test infrastructure (already uses port 0)
- `crates/roko-cli/tests/common/mod.rs` — test helpers (already uses port 0)
- `crates/roko-cli/tests/smoke.rs` — `item_08_agent_sidecar_starts_and_reports_health`
  currently gets a port from `pick_unused_port()` and binds later
- `crates/roko-serve/src/lib.rs` — hardcoded `6677` remains in
  `ServerBuildConfig::effective_port()` and relay registration fallback

Current local `rustc --version` output during enrichment:

```text
rustc 1.95.0 (59807616e 2026-04-14)
```

Current hardcoded-port grep under serve/cli tests shows no `:6677`-`:6680`
test binds, but `crates/roko-cli/tests/common/mod.rs` and
`crates/roko-cli/tests/smoke.rs` use a `pick_unused_port()` helper. That helper
binds `127.0.0.1:0` to discover a port, drops the listener, then starts the real
server later; this is still a race even though the port was discovered via
port 0.

## What to Change

### 1. Pin Rust version in `rust-toolchain.toml`

```toml
[toolchain]
channel = "1.95.0"
components = ["rustfmt", "clippy"]
```

Use the latest stable version available on this machine: `1.95.0`. It is
>= 1.87 per the alloy dependency requirement.

### 2. Enhance CI pipeline

Update `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

env:
  CARGO_TERM_COLOR: always
  RUSTFLAGS: "-D warnings"

jobs:
  fmt:
    name: Formatting
    runs-on: ubuntu-latest
    timeout-minutes: 10
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@nightly
        with:
          components: rustfmt
      - run: cargo fmt --all --check

  clippy:
    name: Clippy
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.95.0
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --workspace --no-deps -- -D warnings

  test:
    name: Unit Tests
    runs-on: ubuntu-latest
    timeout-minutes: 45
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.95.0
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace --lib --bins

  integration-test:
    name: Integration Tests
    runs-on: ubuntu-latest
    timeout-minutes: 60
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.95.0
      - uses: Swatinem/rust-cache@v2
      - run: cargo test -p roko-serve --tests -- --test-threads=1
      - run: cargo test -p roko-cli --tests -- --test-threads=1

  layer-check:
    name: Layer Check
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.95.0
      - uses: Swatinem/rust-cache@v2
      - name: Build roko-cli
        run: cargo build -p roko-cli
      - name: Run layer check
        run: cargo run -p roko-cli -- layer-check
```

Key changes:
- Split clippy into its own job (parallel with test)
- Keep `layer-check` as a separate job
- Do not leave `dtolnay/rust-toolchain@stable` in CI; use the pinned toolchain
  version for clippy/test/layer-check. Keep nightly only for `cargo fmt`.
- The unit-test job should avoid integration tests; the integration job runs
  port/filesystem-heavy test targets serially.

### 3. Verify port 0 usage in tests

Audit all test files under `crates/roko-serve/tests/` and `crates/roko-cli/tests/`
to confirm they use `TcpListener::bind("127.0.0.1:0")` for server tests. If any
test uses a hardcoded port (6677, 6678, etc.), change it to port 0.

```bash
grep -rn ':6677\|:6678\|:6679\|:6680' crates/roko-serve/tests/ crates/roko-cli/tests/ --include='*.rs' | grep -v target/
```

Also address the current port-picking race:
- In `crates/roko-cli/tests/smoke.rs`, bind the `AgentServer` to
  `"127.0.0.1:0"` and use the builder's `on_start` hook to send the bound
  `SocketAddr` through a oneshot channel before probing `/health`.
- In `crates/roko-cli/tests/common/mod.rs`, keep `pick_unused_port()` only if
  removing it would require new production support for subprocess bound-port
  discovery. If you leave it, document why in the task Status Log and ensure no
  fixed ports remain. Do not change subprocess helpers to `--port 0` unless the
  helper can reliably learn the selected port from the child process.

### 4. Remove production `6677` fallbacks in serve where already centralized

In `crates/roko-serve/src/lib.rs`:
- Replace `self.roko_config.server.port == 6677` in
  `ServerBuildConfig::effective_port()` with
  `roko_core_crate::defaults::DEFAULT_SERVE_PORT`.
- Replace `self.config.port.unwrap_or(6677)` in relay registration setup with
  `self.config.port.unwrap_or(roko_core_crate::defaults::DEFAULT_SERVE_PORT)`.

Do not change the user-facing default port; this is only centralizing the
literal.

## What NOT to Do

- Don't add Playwright/E2E tests — that is a separate task.
- Don't add `--features integration` job yet — no integration feature flag exists.
- Don't change existing test logic — only fix port binding.
- Don't remove the layer-check job.
- Don't use `cargo test --test '*'`; use `--tests` for all integration test
  targets.
- Don't overwrite unrelated in-progress changes in `crates/roko-serve/src/lib.rs`;
  inspect the dirty worktree first and edit around user changes.

## Wire Target

```bash
# Verify CI config parses
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"
# Verify tests pass with port 0
cargo test -p roko-serve --tests -- --test-threads=1
cargo test -p roko-cli --tests -- --test-threads=1
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `rust-toolchain.toml` has pinned version
- [ ] CI YAML is valid
- [ ] No hardcoded ports in serve/cli tests
  (`grep -rn ':6677' crates/roko-serve/tests/ crates/roko-cli/tests/` returns nothing)
- [ ] `rg -n 'dtolnay/rust-toolchain@stable|channel = "stable"' .github/workflows/ci.yml rust-toolchain.toml`
      returns no matches
- [ ] `rg -n 'unwrap_or\\(6677\\)|== 6677' crates/roko-serve/src/lib.rs`
      returns no matches

## Status Log

| Time | Agent | Action |
|------|-------|--------|
