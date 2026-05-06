# Task 051: Add Core Integration Test Suite

```toml
id = 51
title = "Add integration tests for config roundtrip, PRD pipeline, serve lifecycle"
track = "infrastructure"
wave = "wave-2"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-core/tests/",
    "crates/roko-cli/tests/",
    "crates/roko-serve/tests/",
]
exclusive_files = []
estimated_minutes = 240
```

## Context

The audit (S8) estimated overall test coverage at ~35%. The most critical gap is
integration tests that verify end-to-end behavior across crate boundaries. Unit tests
exist but don't catch "works in isolation, broken when wired" bugs.

The audit identified these as the highest-priority integration tests:
1. Config roundtrip (parse → serialize → reparse → assert equal)
2. PRD pipeline (idea → draft scaffold → verify artifacts exist)
3. Serve lifecycle (start → health check → shutdown → port released)

## Background

Read:
- `crates/roko-core/tests/config_loader_integration.rs` — existing config tests
- `crates/roko-cli/tests/smoke.rs` — existing smoke tests
- `crates/roko-serve/tests/api_integration.rs` — existing API tests
- `crates/roko-cli/tests/common/mod.rs` — shared test utilities

Current helper/call-path facts:
- `RokoConfig` derives `PartialEq` in `crates/roko-core/src/config/schema.rs`
  and exposes `from_toml`, `to_toml`, and `to_toml_pretty`.
- `crates/roko-core/tests/config_loader_integration.rs` already contains config
  roundtrip tests and a workspace-root pattern using
  `PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize()`.
- `roko-cli` exports `roko_cli::prd`. Public helpers available for tests include
  `ensure_dirs`, `cmd_idea`, `new_draft_frontmatter`, `slugify`, and
  `materialize_agent_markdown_output`. There is no exported `write_scaffold`
  helper in `roko_cli::prd`.
- Agent-assisted PRD draft creation is implemented in
  `crates/roko-cli/src/commands/prd.rs` under `PrdDraftCmd::New`; it pre-writes
  the scaffold, then calls `run_agent_capture_silent`. Do not use that path in a
  default integration test unless the agent execution is mocked or suppressed.
- `roko-serve` integration tests already define minimal `CliRuntime`
  implementations in `tests/api_integration.rs`, `tests/job_lifecycle.rs`, and
  `tests/prd_publish.rs`.
- The health endpoint mounted by serve is `/api/health`, not `/health`.

## What to Change

### 1. Config Roundtrip Test (`roko-core/tests/`)

Prefer extending `crates/roko-core/tests/config_loader_integration.rs` unless a
new file keeps the assertions clearly separated. Avoid duplicating existing
small roundtrip tests; add coverage for the repository's actual root
`roko.toml`.

```rust
#[test]
fn root_roko_toml_roundtrip_preserves_model_registry() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let raw = std::fs::read_to_string(workspace_root.join("roko.toml")).unwrap();
    let config = RokoConfig::from_toml(&raw).unwrap();

    let serialized = config.to_toml_pretty().unwrap();
    let reparsed = RokoConfig::from_toml(&serialized).unwrap();

    assert_eq!(config.providers, reparsed.providers);
    assert_eq!(config.models, reparsed.models);
    assert_eq!(config.server, reparsed.server);
    assert_eq!(config.serve, reparsed.serve);
}
```

If a new `config_roundtrip.rs` file is created, import
`roko_core::config::schema::RokoConfig` and `std::path::PathBuf`; do not use a
relative `"../../roko.toml"` string because cargo test working directories vary.

### 2. PRD Pipeline Test (`roko-cli/tests/`)

Add `crates/roko-cli/tests/prd_pipeline.rs`:

```rust
#[test]
fn prd_idea_creates_ideas_file() {
    let tmp = tempfile::tempdir().unwrap();
    // Initialize .roko/ structure
    roko_cli::prd::ensure_dirs(tmp.path()).unwrap();

    // Capture idea
    roko_cli::prd::cmd_idea(tmp.path(), "test integration idea").unwrap();

    // Verify file exists and contains the idea
    let ideas = tmp.path().join(".roko/prd/ideas.md");
    assert!(ideas.exists());
    let content = std::fs::read_to_string(&ideas).unwrap();
    assert!(content.contains("test integration idea"));
}

#[test]
fn prd_draft_scaffold_creates_markdown() {
    let tmp = tempfile::tempdir().unwrap();
    roko_cli::prd::ensure_dirs(tmp.path()).unwrap();

    let title = "Test Title";
    let slug = roko_cli::prd::slugify(title);
    let frontmatter = roko_cli::prd::new_draft_frontmatter(&slug, title);
    let scaffold = format!(
        "{frontmatter}# {title}\n\n\
         ## Overview\n\n## Requirements\n\n## Acceptance criteria\n\n\
         ## Design\n\n## References\n"
    );
    let draft_path = tmp.path().join(format!(".roko/prd/drafts/{slug}.md"));
    std::fs::write(&draft_path, scaffold).unwrap();

    assert!(draft_path.exists());
    let content = std::fs::read_to_string(&draft_path).unwrap();
    assert!(content.contains(&slug));
    assert!(content.contains("status: draft"));
}
```

Also add one recovery-oriented assertion that uses
`materialize_agent_markdown_output` with the scaffold and verifies frontmatter is
present when agent output lacks it. This covers the non-agent materialization
path without requiring a live CLI agent.

Do not call `roko prd draft new` through the binary unless you add a deterministic
way to prevent `run_agent_capture_silent` from invoking a real provider. A test
that depends on `claude`, API keys, or model configuration does not belong in
the default suite.

### 3. Serve Lifecycle Test (`roko-serve/tests/`)

Add `crates/roko-serve/tests/lifecycle.rs`:

```rust
#[tokio::test]
async fn serve_start_health_shutdown() {
    // Bind to random port
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    // Build app state with temp workdir
    let tmp = tempfile::tempdir().unwrap();
    let state = build_test_app_state(tmp.path()).await;
    let cancel = state.cancel.clone();

    // Start server in background
    let handle = tokio::spawn(async move {
        roko_serve::run_server_with_state(state, "127.0.0.1", port).await
    });

    // Poll readiness instead of sleeping once.
    let health_url = format!("http://127.0.0.1:{port}/api/health");
    let client = reqwest::Client::new();
    let ready = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(resp) = client.get(&health_url).send().await {
                if resp.status().is_success() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(ready.is_ok(), "server did not become healthy at {health_url}");

    // Trigger shutdown
    cancel.cancel();

    // Server should exit cleanly
    let result = tokio::time::timeout(Duration::from_secs(5), handle).await;
    assert!(result.is_ok(), "server did not shut down in 5s");
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
}
```

Build `AppState` the same way existing serve tests do: create a temp workdir,
use `RokoConfig::default()`, create a manual deploy backend with
`roko_serve::deploy::create_backend`, and implement a no-op `CliRuntime` that
returns a successful `RunResult`. Disable auth in the test config or router only
if the endpoint being exercised requires unauthenticated access; `/api/health`
should remain reachable with the normal serve router.

### 4. Shared Test Utilities

If not already present, add test helpers for:
- Creating temp workdirs with `.roko/` structure
- Building `AppState` with test config (no real LLM keys needed)

Keep helper changes local to the test crate that uses them unless at least two
new tests share the helper. Do not move existing helpers out of
`tests/api_integration.rs` as part of this task.

## What NOT to Do

- Don't add tests that require real LLM API keys — these are structural integration
  tests, not live provider tests.
- Don't add Playwright E2E tests — that is a separate task.
- Don't modify existing test infrastructure — only add new tests.
- Don't add a `--features integration` gate yet — keep tests in the default test suite
  unless they are slow (> 5 seconds).
- Don't use hard-coded port `6677` in tests; bind `127.0.0.1:0`, capture the
  assigned port, and release it before starting serve.
- Don't use fixed sleeps as readiness checks when a polling loop can observe the
  endpoint.
- Don't introduce `#[ignore]` or tests that only pass on a developer machine with
  local provider credentials.

## Wire Target

```bash
cargo test -p roko-core --test config_loader_integration root_roko_toml_roundtrip
cargo test -p roko-cli --test prd_pipeline
cargo test -p roko-serve --test lifecycle
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] Config roundtrip test passes with actual `roko.toml`
- [ ] PRD pipeline test creates artifacts in temp dir
- [ ] Serve lifecycle test starts/stops cleanly on an OS-assigned port and proves
      the port can be rebound after cancellation

## Status Log

| Time | Agent | Action |
|------|-------|--------|
