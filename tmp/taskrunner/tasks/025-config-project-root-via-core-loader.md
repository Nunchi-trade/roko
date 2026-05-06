# Task 025: Runner Uses Core Config Loader (Not Ad-Hoc Project Root Walk)

```toml
id = 25
title = "Runner v2 uses roko-core config loader for project root resolution"
track = "runner-hardening"
wave = "wave-1"
priority = "high"
blocked_by = [1]
touches = [
    "crates/roko-cli/src/runner/event_loop.rs",
    "crates/roko-cli/src/runner/mod.rs",
]
exclusive_files = []
estimated_minutes = 60
```

## Context

The runner loads config from `cfg.exec_dir` which may be a worktree with no `roko.toml`.
The infrastructure audit documents 20 separate config loading functions — adding another
walk-up-to-find-config strategy would create a 21st.

**The fix is to have the runner use the core loader** (`roko_core::config::loader::load_config_unified()`
or `load_config_validated()`), which already handles ancestor walk, global merge, env vars, etc.

This task depends on Task 001 (config loader migration) because the core loader must be
the canonical path before the runner adopts it.

Sources:
- `tmp/infrastructure-audit.md` — Section 27: Config Loader Proliferation
- Audit finding: don't add a 21st config loader

## Background

Read these before editing:

1. `crates/roko-core/src/config/loader.rs`
   - canonical functions: `load_config_unified`, `load_config_with_options`,
     `load_config_validated`, `load_config_validated_with_options`
   - `load_config_unified(workdir)` performs ancestor discovery via `find_config_path`,
     optional global merge, named env overrides, interpolation, and file-secret resolution.
2. `crates/roko-cli/src/bootstrap.rs`
   - `RokoBootstrap::new` already calls `roko_core::config::loader::load_config_unified(&workdir)`.
3. `crates/roko-cli/src/commands/plan.rs` (read-only for this task unless the task metadata is expanded)
   - CLI chain: `roko plan run` -> resolve `wd` -> `RokoBootstrap::new(&wd, ...)` ->
     `early_roko_config` -> `RunConfig { roko_config: Some(Arc::new(roko_config.clone())), ... }`
     -> `runner::event_loop::run(...)`.
4. `crates/roko-cli/src/runner/types.rs`
   - `RunConfig::roko_config: Option<Arc<RokoConfig>>`
   - `RunConfig::from_roko_config(workdir, plan_dir, roko_config)` is the helper path for callers
     that already loaded effective config.
5. `crates/roko-cli/src/runner/event_loop.rs`
   - `run(plans, config, state_hub, cancel)` clones `RunConfig` and all later timeout/gate/model
     helpers should consume `config.roko_config`.
   - Key consumers: `agent_dispatch_timeout`, `plan_total_timeout`, `llm_call_timeout`,
     `gate_timeout`, and `gates_config_for_run`.
6. `crates/roko-cli/src/runner/mod.rs`
   - public runner entrypoint re-exports `run` and `RunConfig`.

Current-state checks:

```bash
grep -rn 'load_roko_config\|load_config\|load_layered' crates/roko-cli/src/runner/ --include='*.rs' | grep -v target/
grep -rn 'roko_config: Some' crates/roko-cli/src/commands/plan.rs crates/roko-cli/src/serve_runtime.rs crates/roko-cli/src/worker/cloud.rs --include='*.rs'
```

As of this spec pass, the active `roko plan run` path already loads config through
`RokoBootstrap`/`load_config_unified` in `commands/plan.rs` and passes it into
`RunConfig.roko_config`. This task should harden the runner boundary and verify there
is no runner-local project-root walk. The `cfg.exec_dir` wording in the original task
refers to legacy `orchestrate.rs`; do not use that as the active runner-v2 target.

## What to Change

1. In `crates/roko-cli/src/runner/event_loop.rs`, ensure `run(...)` has an effective
   `Arc<RokoConfig>` before any timeouts, gates, or agent dispatch are evaluated:
   - Prefer `config.roko_config.clone()` when present.
   - If absent, call `roko_core::config::loader::load_config_unified(&config.workdir)` once,
     wrap it in `Arc`, and assign it back onto the local cloned `RunConfig`.
   - Add `anyhow::Context` to errors so a bad config reports the workdir path.

   Mechanical shape:

   ```rust
   let mut config = config.clone();
   if config.roko_config.is_none() {
       let roko_config = roko_core::config::loader::load_config_unified(&config.workdir)
           .with_context(|| format!("load roko config for runner workdir {}", config.workdir.display()))?;
       config.roko_config = Some(Arc::new(roko_config));
   }
   ```

   Place this before helpers read `config.roko_config` or `config.timeout_secs`.

2. Keep existing callers that already pass `RunConfig.roko_config` unchanged. The fallback is
   for tests/secondary callers, not a replacement for the CLI bootstrap path.

3. Do not add config loading inside per-task dispatch, gate dispatch, or merge code. The loaded
   `Arc<RokoConfig>` must be shared through `RunConfig`.

4. In `crates/roko-cli/src/runner/mod.rs`, update the module docs only if needed to document
   that runner callers should pass a `RunConfig` built from the effective `RokoConfig`.

5. Add or update focused tests where feasible:
   - a runner test that builds `RunConfig` without `roko_config`, runs a dry/minimal path, and
     observes that no ad-hoc loader is needed
   - a `RunConfig::from_roko_config`/event-loop test proving `config.roko_config` wins over
     timeout/default fallbacks
   - if adding such a test requires editing `runner/types.rs` or `commands/plan.rs`, stop and
     note the task touch-list mismatch in the Status Log instead of editing outside `touches`.

## What NOT to Do

- Don't write a new config resolution function.
- Don't add `find_project_root()` to the runner — the core loader does this.
- Don't bypass the core loader for "convenience."
- Don't touch `crates/roko-cli/src/orchestrate.rs`; it is behind `legacy-orchestrate` and not the
  active runner-v2 path.
- Don't replace `commands/plan.rs` bootstrap/loading unless the task's `touches` metadata is
  explicitly expanded. It is currently read-only context for this task.
- Don't call `load_layered()` from runner internals. `load_layered()` is CLI provenance wrapping,
  not the core runtime loader.
- Don't silently fall back to `RokoConfig::default()` on loader errors in runner-v2 runtime paths.
  Propagate with context so config problems fail at startup.

## Wire Target

```bash
# Run from a worktree — should find config via core loader's ancestor walk
cargo run -p roko-cli -- plan run plans/
# Config should be found even without roko.toml in the immediate directory
```

Expected observable behavior:

- `roko plan run` loads one effective `RokoConfig` for the workspace root and carries it through
  `RunConfig.roko_config`.
- Gate config, timeout config, provider/model dispatch, budget limits, and runner concurrency
  use that same config.
- Running from a nested directory/worktree without a local `roko.toml` still finds an ancestor
  `roko.toml` through `roko_core::config::loader::load_config_unified`.
- Loader failures produce an actionable startup error instead of silently using defaults.

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] Runner uses core loader, not its own config resolution
- [ ] `grep -rn 'load_roko_config\|load_layered\|find_project_root' crates/roko-cli/src/runner/ --include='*.rs' | grep -v target/` shows no runner-local/ad-hoc loader
- [ ] `grep -rn 'load_config_unified' crates/roko-cli/src/runner crates/roko-cli/src/commands/plan.rs --include='*.rs' | grep -v target/` shows the active core-loader path or the event-loop fallback
- [ ] `cargo run -p roko-cli -- plan run plans/ --dry-run` still works from repo root
- [ ] From a nested temp directory under the repo, `cargo run -p roko-cli -- plan run <absolute-plans-dir> --dry-run` still resolves ancestor config

## Status Log

| Time | Agent | Action |
|------|-------|--------|
