# Task 001: Migrate CLI Config Loading to Core Loader

```toml
id = 1
title = "Migrate CLI's load_layered() callsites to roko-core's unified config loader"
track = "config-foundation"
wave = "wave-0"
priority = "critical"
blocked_by = []
touches = [
    "crates/roko-core/src/config/loader.rs",
    "crates/roko-core/src/config/provenance.rs",
    "crates/roko-core/src/config/mod.rs",
    "crates/roko-cli/src/config.rs",
    "crates/roko-cli/src/config_cmd.rs",
    "crates/roko-cli/src/lib.rs",
]
exclusive_files = ["crates/roko-cli/src/config.rs"]
estimated_minutes = 240
```

## Context

The unified config loader ALREADY EXISTS in `roko-core/src/config/loader.rs` — it has
`load_config_unified()`, `load_config_validated()`, `load_config_with_options()`, diagnostics
collection, global merge, env var overrides, and ancestor walk. This was built in redesign
batches 5-29.

However, `roko-cli/src/config.rs` still has its OWN 2,871-line config system with
`load_layered()`, `ConfigLayer`, `ResolvedConfig`, and provenance tracking. There are 30+
callsites using `load_layered()`. This means CLI and serve use different config loading paths.

**The task is NOT "build a unified loader" — it's "migrate CLI to the one that already exists."**

Sources:
- `tmp/infrastructure-audit.md` — B4, Section 27: 20 config loading functions
- `tmp/redesign-plan.md` — Batches 5-29 (core loader consolidation, partially done)

## Background

Read these files first:
1. `crates/roko-core/src/config/loader.rs` — the EXISTING unified loader (this is the target)
2. `crates/roko-cli/src/config.rs` — the CLI's legacy loader (this is what gets migrated)
3. `grep -rn 'load_layered\|ResolvedConfig' crates/roko-cli/ --include='*.rs' | grep -v target/` — all callsites

Current branch facts to verify before editing:
- Core loader entry points are `load_config_unified()`, `load_config_with_options()`,
  `load_config_file()`, `load_config_validated()`, and
  `load_config_validated_with_options()` in `crates/roko-core/src/config/loader.rs`.
- `load_layered()` still lives in `crates/roko-cli/src/config.rs` and still manually reads
  global/project `ConfigLayer`s, applies `ROKO__...` env overrides, computes `ConfigSources`,
  and builds `RepoRegistry`.
- Core supports named env vars through `RokoConfig::apply_process_env()` (`ROKO_MODEL`,
  `ROKO_CONTEXT_LIMIT_K`, etc.), but the current loader comments explicitly say hierarchical
  `ROKO__SECTION__FIELD` overrides are not implemented in core. CLI still implements that at
  `collect_env_override_layer_from()` / `env_override_path()` / `apply_layer_value()`.
- CLI call chain examples:
  - `roko config show` -> `crates/roko-cli/src/config_cmd.rs::cmd_show()` -> `load_layered()`.
  - `roko plan run` -> `crates/roko-cli/src/commands/plan.rs::cmd_plan()` -> `load_layered()`
    -> `RunConfig` construction -> `runner::event_loop::run()`.
  - `roko run` -> `crates/roko-cli/src/run.rs::run_once()` -> `load_layered()`.
- `ResolvedConfig` currently carries four surfaces later code uses: `config`, `repo_registry`,
  `sources`, and `paths`. Do not drop these without replacing every consumer.

## What to Change

1. **Make core handle all effective config inputs first**:
   - In `roko-core/src/config/loader.rs`, add support for hierarchical `ROKO__...` overrides
     before returning the migrated/effective `RokoConfig`. Port the existing path parsing idea
     from CLI (`ROKO__AGENT__MODEL` -> `agent.model`) but apply it to `RokoConfig` through
     structured TOML/serde, not ad hoc string edits.
   - Preserve existing named env vars (`ROKO_MODEL`, `ROKO_CONTEXT_LIMIT_K`, etc.). Add tests
     proving named and hierarchical overrides both flow through `load_config_validated*()`.
2. **Keep CLI provenance as a compatibility wrapper, not a second effective loader**:
   - Replace the body of `crates/roko-cli/src/config.rs::load_layered()` with a wrapper around
     `roko_core::config::loader::load_config_validated_with_options(workdir, &LoadOptions::default())`.
   - Keep `ResolvedConfig`, `ConfigSources`, `ConfigPaths`, and `RepoRegistry` until all
     consumers are migrated. Build `paths` with the existing wrappers around core helpers.
   - Use the core-loaded `ValidatedConfig.migrated` as the authoritative source for providers,
     models, agent defaults, timeouts, gates, and env-overridden values. If any CLI-only fields
     still require `ConfigLayer` parsing (`auto_plan`, `repos`, legacy `[[gate]]`, etc.), document
     each remaining compatibility parse in the function comment and do not let it override core
     provider/model/env behavior.
3. **Rename away from the legacy API once the wrapper delegates to core**:
   - Add a clearly named CLI adapter such as `load_resolved_config(workdir) -> Result<ResolvedConfig>`.
   - Update every non-test callsite returned by
     `rg 'load_layered\(' crates/roko-cli/src -g '*.rs'` to call the new adapter.
   - Remove the public `load_layered` export from `crates/roko-cli/src/lib.rs`. Keeping a private
     test-only shim is acceptable only if grep proves no runtime callsite remains.
4. **Callsite checklist**: update all current hits in `prd.rs`, `unified.rs`,
   `commands/agent.rs`, `doctor.rs`, `commands/plan.rs`, `commands/job.rs`, `bench_demo.rs`,
   `chat_inline.rs`, `commands/server.rs`, `dispatch_v2.rs`, `config_cmd.rs`, `main.rs`,
   `commands/util.rs`, `daemon.rs`, and `run.rs`.
5. **Tests to add/update**:
   - Core loader test: global + project + named env + `ROKO__...` env precedence.
   - CLI config tests in `config.rs`: `load_resolved_config()` still returns source tags used by
     `config show`, and `RepoRegistry::load()` behavior is unchanged.
   - CLI command test or assertable snapshot for `config show` showing env-tagged values.

## What NOT to Do

- Don't write a NEW config loader (there are already too many).
- Don't change the config schema.
- Don't change what fields are in roko.toml.
- Don't skip callsites — migrate ALL of them or the dual-loader problem persists.
- Don't touch ACP config loading (that was already fixed in batches 25-30).
- Don't serialize `RokoConfig` to a string and parse it back into CLI `Config` as the only
  bridge; that hides schema mismatches and loses provenance. Use explicit structured conversion
  or narrowly documented compatibility parsing.
- Don't leave hierarchical `ROKO__...` support only in `roko-cli`; after this task, core owns env
  override behavior for effective config.

## Wire Target

Both paths must produce identical config:
```bash
cargo run -p roko-cli -- config show
```

```bash
# Verify no CLI-side config loading remains:
grep -rn 'load_layered' crates/roko-cli/ --include='*.rs' | grep -v target/
# Should return nothing (or only a deprecated re-export)
```

Expected observable behavior:
- A core loader unit test proves `ROKO_CONTEXT_LIMIT_K=32` reaches
  `RokoConfig.agent.context_limit_k`.
- `ROKO_MODEL=test-model cargo run -p roko-cli -- config show` and
  `ROKO__AGENT__MODEL=test-model cargo run -p roko-cli -- config show` mark the agent model as
  env-sourced and the same override is visible to `roko plan run` setup.
- `rg 'load_layered\(' crates/roko-cli/src -g '*.rs'` has no runtime hits after the rename.

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `grep -rn 'load_layered' crates/roko-cli/ --include='*.rs' | grep -v target/` — zero callsites remain
- [ ] `cargo run -p roko-cli -- config show`
- [ ] `ROKO_MODEL=test-model cargo run -p roko-cli -- config show`
- [ ] `ROKO__AGENT__MODEL=test-model cargo run -p roko-cli -- config show`
- [ ] `rg 'load_config_validated_with_options|load_config_unified|load_config_with_options' crates/roko-cli/src/config.rs crates/roko-cli/src/config_cmd.rs`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
