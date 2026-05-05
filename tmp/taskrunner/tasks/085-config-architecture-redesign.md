# Task 085: Config Architecture Redesign — Cache, ACP Watch, Slug Validation, Export

```toml
id = 85
title = "Config architecture redesign: ConfigCache (arc-swap + notify), ACP ConfigWatcher, validate_models warnings, provider merge validation, roko config export, delete railway.roko.toml"
track = "config-foundation"
wave = "wave-2"
priority = "high"
blocked_by = [1]
touches = [
    "crates/roko-core/src/config/loader.rs",
    "crates/roko-core/src/config/cache.rs",
    "crates/roko-core/src/config/validation.rs",
    "crates/roko-core/src/config/mod.rs",
    "crates/roko-core/Cargo.toml",
    "crates/roko-acp/src/config_watch.rs",
    "crates/roko-acp/src/handler.rs",
    "crates/roko-acp/src/session.rs",
    "crates/roko-cli/src/commands/config_cmd.rs",
    "crates/roko-cli/src/main.rs",
    "docker/railway.roko.toml",
]
exclusive_files = [
    "crates/roko-core/src/config/cache.rs",
    "crates/roko-core/src/config/validation.rs",
]
estimated_minutes = 360
```

## Context

Combines five concerns from the redesign plan and infrastructure audit:

- **Phase 4.2** (Config Caching): `load_roko_config` is called O(tasks × rungs) times per
  plan run — once per task, once per gate rung, once per dispatch. Each call reads and parses
  `roko.toml` from disk. The static `OnceLock` cache in `orchestrate.rs` covers only that
  process and has no invalidation. Replace with a proper `ConfigCache` backed by `arc-swap`
  and `notify::RecommendedWatcher` that lives in `roko-core` and is shareable across the
  whole process.

- **Phase 10.3** (Live ACP Config Watch): When a user edits `~/.roko/config.toml` or
  `roko.toml` while Zed is open, the ACP `SessionManager` keeps using the snapshot it loaded
  at startup. There is no mechanism to pick up changes short of restarting Zed. The redesign
  adds `ConfigWatcher` to `roko-acp` — a wrapper around `ConfigCache` that additionally
  rebuilds config options for active sessions when the underlying config swaps.

- **Phase 10.4 + S27.7** (Model Slug Validation at load time): `collect_diagnostics()` in
  `loader.rs` (lines 229-311) already detects duplicate slugs and orphaned model→provider
  references, but it is called only from `load_config_validated()`. The hot-path function
  `load_config_unified()` silently skips all diagnostics. Every duplicate slug (`kimi-k25`
  and `kimi-k2-5` both → `"kimi-k2.5"`) and every orphaned model are invisible to the
  plan runner, agent dispatch, and gate pipeline. Surface these as `tracing::warn!()` on
  every load.

- **S27.6** (Provider merge validation): `merge_global_into()` uses `entry.or_insert()`
  so global providers do not overwrite project providers. But no post-merge step checks
  that every `model.provider` key still names a real provider. A model defined in the
  project may reference a provider only available in global config, and vice versa; the merge
  can silently produce an inconsistent graph. Validation must run after merge, not before.

- **Phase 8.2** (Single config + `roko config export`): `docker/railway.roko.toml` is a
  hand-maintained fork of the main `roko.toml` that accumulates drift. Railway deployments
  should override via the named env vars that `RokoConfig::apply_process_env()` actually
  consumes. Do not assume hierarchical `ROKO__SECTION__FIELD` support unless it exists in
  the current loader. Delete the file and add `roko config export --env railway` that prints
  the minimal set of env vars needed to replicate the effective config at a deployment target.

## Background

Read these files before writing any code:

1. `crates/roko-core/src/config/loader.rs` — the unified loader. Key landmarks:
   - `load_config_unified()` line 92: the hot-path function. In older branches it skipped
     diagnostics; in the current branch verify that diagnostics flow through
     `load_from_resolved_path()` before adding anything.
   - `load_config_validated()` line 117: returns `ValidatedConfig` with diagnostics.
   - `collect_diagnostics()` line 229: already correct. Checks duplicate slugs (lines
     272-292) and orphaned model→provider references (lines 247-268). Do NOT rewrite it.
   - `merge_global_into()` line 406: uses `entry.or_insert()`, no post-merge validation.
   - `load_from_resolved_path()` line 210: internal helper shared by both public functions.

2. `crates/roko-core/src/config/hot_reload.rs` — `config_diff()` and `apply_hot_reload()`
   are already correct. Do not move cache code into this file; `ConfigCache` belongs in
   `config/cache.rs`, which already exists on the current branch.

3. `crates/roko-core/src/config/mod.rs` — lists all submodules. A new `cache` module
   needs to be added here. Check existing exports before adding new `pub use` lines.

4. `crates/roko-acp/src/session.rs` — `SessionManager` struct (line 877): holds
   `roko_config: RokoConfig` as a plain owned value (not behind `Arc` or `ArcSwap`).
   `create_session()` (line 897) passes `&self.roko_config` to `AcpSession::new_with_config()`.
   `revalidate_config_state()` (line 729) rebuilds config options for a session given a new
   `RokoConfig` — this is already the right shape; the gap is triggering it on file change.

5. `crates/roko-acp/src/runner.rs` line 453: loads config once at workflow-run time via
   `load_config_with_options(workdir, &LoadOptions::acp())`. This per-run load does not
   need to change — the `ConfigWatcher` target is the `SessionManager`, which lives for the
   lifetime of the ACP server process.

6. `crates/roko-acp/src/config.rs` — `AcpConfig` struct. Already has `config_path` and
   `global_config_path` fields. The `ConfigWatcher` should watch both paths.

7. `crates/roko-cli/src/commands/config_cmd.rs` — `dispatch_config()` (line 18). The current
   branch already has `ConfigCmd::Export`; verify behavior rather than adding a duplicate arm.

8. `docker/railway.roko.toml` — the file to delete. Contains providers, models, serve,
   server, chain, relay, runner sections. Before deleting, identify any settings NOT covered
   by env var overrides that would be lost — document them.

9. Cargo dependency check — the current branch has `arc-swap = "1"` and
   `notify = { workspace = true }` in `crates/roko-core/Cargo.toml`. Do not add `arc-swap`
   to the root workspace table unless it already exists there.

## Current Tree Notes and Remaining Mechanical Work

The current branch already contains partial implementations of this task. Before editing,
verify these facts and only change the missing or incorrect pieces:

- `crates/roko-core/src/config/cache.rs` exists and is exported from `config/mod.rs`.
- `crates/roko-core/Cargo.toml` already contains `arc-swap = "1"` and
  `notify = { workspace = true }`; the root workspace currently has `notify` but not
  `arc-swap`. Do not add a root workspace dependency unless the workspace table already owns it.
- `load_from_resolved_path()` already logs diagnostics from `collect_diagnostics()`.
- `merge_global_into()` already logs post-merge missing provider references.
- `crates/roko-acp/src/config_watch.rs` exists; `handler.rs` starts it, calls
  `config_watcher.changed()`, reloads config, calls `SessionManager::replace_roko_config()`,
  and sends config-option notifications.
- `SessionManager` still stores a plain `RokoConfig`, but it has `replace_roko_config()`,
  `revalidate_all_sessions()`, and `active_session_config_options()`. Do not rewrite it to
  hold `Arc<ConfigWatcher>` unless the handler-driven reload path cannot satisfy the tests.
- `roko config export --env railway` already exists in `commands/config_cmd.rs` and
  `ConfigCmd::Export` exists in `main.rs`.
- `docker/railway.roko.toml` is already deleted.

Remaining high-priority fix: `ConfigCache::new()` currently creates one `ArcSwap` captured by
the watcher closure and then stores a second independent `ArcSwap` in `Self`. That means watcher
reloads update the captured swap but `ConfigCache::get()` can keep returning the initial config.
Fix this before treating the cache as complete. The simplest safe shape is:

```rust
pub struct ConfigCache {
    config: Arc<ArcSwap<RokoConfig>>,
    _watcher: Option<RecommendedWatcher>,
}

// Closure captures Arc::clone(&config); get() calls self.config.load_full().
```

Do not use `MaybeUninit` or unsafe. A live-reload test must fail before this fix and pass after it.

Ordered implementation steps:
1. Fix `ConfigCache::new()` so the watcher and `get()` share the same `ArcSwap`.
2. Add a `ConfigCache::new()` integration/unit test that writes `roko.toml`, constructs a watched
   cache, edits the file, waits/polls for reload, and asserts `cache.get()` sees the new provider.
   Keep `new_static()` tests for deterministic no-watch behavior.
3. Keep the loader diagnostics and post-merge provider warnings in `loader.rs`; do not duplicate
   diagnostics in `load_config_unified()`.
4. For ACP, prefer the existing handler-driven reload model: `ConfigWatcher::changed()` ->
   `AcpConfig::load_roko_config()` -> `SessionManager::replace_roko_config()` -> notification.
   Add tests around this path instead of moving ownership into `SessionManager` if the current
   path works.
5. Verify `roko config export --env railway` prints only named overrides that
   `RokoConfig::apply_process_env()` actually consumes (`ROKO_MODEL`, `ROKO_BACKEND`,
   `ROKO_EFFORT`, `ROKO_BUDGET_USD`, `ROKO_MAX_AGENTS`, `ROKO_PARALLEL`, and provider key env
   comments). Do not claim hierarchical `ROKO__SECTION__FIELD` support in core.
6. Keep `docker/railway.roko.toml` deleted and remove any remaining references.

Runtime/CLI call chains to validate:
- Config load hot path: `roko-cli` command -> `resolve_workdir(cli)` ->
  `roko_core::config::loader::load_config_unified()` -> `load_from_resolved_path()` ->
  `merge_global_into()` -> env overrides -> diagnostics warnings.
- ACP live reload: `roko-acp::handler::handle_stdio()` loop -> `ConfigWatcher::changed()` ->
  `AcpConfig::load_roko_config()` -> `SessionManager::replace_roko_config()` ->
  `AcpSession::revalidate_config_state()` -> config-option notification.
- Export command: `ConfigCmd::Export` in `main.rs` -> `dispatch_config()` ->
  `commands::config_cmd::cmd_export()`.

Tests to add or update:
- `cargo test -p roko-core config::cache` must include a watched reload test, not only
  `new_static()` tests.
- `cargo test -p roko-core config::loader` should cover duplicate slug and missing-provider
  warnings through `load_config_unified()` where feasible.
- `cargo test -p roko-acp config_watch session` should cover reload/revalidation behavior or a
  direct `replace_roko_config()` regression if stdio handler testing is too heavy.
- `cargo test -p roko-cli config_export` should cover `railway` success and unknown target error.

## What to Change

### 1. Verify/Fix `ConfigCache` in `crates/roko-core/src/config/cache.rs`

The current branch already has `crates/roko-core/src/config/cache.rs`. Do NOT add cache code to
`hot_reload.rs` or `loader.rs` — keep/fix the cache as a separate module.

```rust
//! Config cache with file-watch invalidation (redesign Phase 4.2 / Phase 10.3).
//!
//! `ConfigCache` loads once at construction, then atomically swaps when the
//! underlying config file changes.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use arc_swap::ArcSwap;
use notify::{Event, EventKind, RecommendedWatcher, Watcher};

use super::loader::{
    LoadConfigError, LoadOptions, discover_project_config, global_config_path,
    load_config_with_options,
};
use super::schema::RokoConfig;

/// A config loaded once and atomically refreshed when the underlying
/// file changes on disk.
///
/// `ConfigCache` is `Send + Sync` and cheap to clone via `Arc`.
pub struct ConfigCache {
    config: Arc<ArcSwap<RokoConfig>>,
    /// Keeps the watcher alive for the lifetime of the cache.
    _watcher: Option<RecommendedWatcher>,
}

impl ConfigCache {
    /// Load config from `workdir` and start watching for changes.
    ///
    /// Both the project `roko.toml` and the global `~/.roko/config.toml` are
    /// watched. When either changes, the config is reloaded and atomically swapped.
    pub fn new(workdir: &Path) -> Result<Arc<Self>, LoadConfigError> {
        let opts = LoadOptions::default();
        let initial_config = load_config_with_options(workdir, &opts)?;
        let config = Arc::new(ArcSwap::from_pointee(initial_config));

        // Identify which files to watch.
        let project_path = discover_project_config(workdir);
        let global_path = {
            let p = global_config_path();
            p.exists().then_some(p)
        };

        let config_for_watcher = Arc::clone(&config);
        let workdir_owned = workdir.to_path_buf();

        let mut watcher = {
            let opts_clone = opts.clone();
            notify::recommended_watcher(move |res: notify::Result<Event>| {
                let Ok(event) = res else { return };
                if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                    match load_config_with_options(&workdir_owned, &opts_clone) {
                        Ok(new_config) => {
                            tracing::info!(
                                workdir = %workdir_owned.display(),
                                "roko.toml changed — config reloaded"
                            );
                            config_for_watcher.store(Arc::new(new_config));
                        }
                        Err(e) => {
                            tracing::warn!(
                                workdir = %workdir_owned.display(),
                                error = %e,
                                "roko.toml changed but reload failed — keeping previous config"
                            );
                        }
                    }
                }
            })
            .map_err(|e| LoadConfigError::Read {
                path: workdir.join("roko.toml"),
                source: std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
            })?
        };

        if let Some(ref p) = project_path {
            let _ = watcher.watch(p, notify::RecursiveMode::NonRecursive);
        }
        if let Some(ref p) = global_path {
            let _ = watcher.watch(p, notify::RecursiveMode::NonRecursive);
        }

        Ok(Arc::new(Self {
            config,
            _watcher: Some(watcher),
        }))
    }

    /// Read the current config. Zero-copy — returns an `Arc` into the swap slot.
    pub fn get(&self) -> Arc<RokoConfig> {
        self.config.load_full()
    }
}

impl std::fmt::Debug for ConfigCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigCache").finish_non_exhaustive()
    }
}
```

**Implementation note on `ArcSwap` duplication**: The construction above creates two
`ArcSwap` instances — one inside the watcher closure (`inner`) and one on `Self`. Do not
copy that shape. The implementation must store the same shared swap that the watcher closure
captures:

```rust
pub struct ConfigCache {
    config: Arc<ArcSwap<RokoConfig>>,
    _watcher: Option<RecommendedWatcher>,
}

let config = Arc::new(ArcSwap::from_pointee(initial_config));
let config_for_watcher = Arc::clone(&config);
// watcher closure calls config_for_watcher.store(...)
let cache = Self {
    config,
    _watcher: Some(watcher),
};
```

Do not use `MaybeUninit` or unsafe. If the watcher has to be constructed after the cache shell,
use `Option<RecommendedWatcher>` and assign it before returning.

Ensure `cache.rs` remains exported from `crates/roko-core/src/config/mod.rs`:

```rust
pub mod cache;
pub use cache::ConfigCache;
```

### 2. Verify diagnostics in `load_config_unified()` (log warnings on every load)

The current branch already routes diagnostics through `load_from_resolved_path()`. Verify it
matches this shape before changing it:

```rust
fn load_from_resolved_path(
    path: &Option<PathBuf>,
    opts: &LoadOptions,
) -> Result<RokoConfig, LoadConfigError> {
    let mut config = parse_from_resolved_path(path, opts)?;

    if opts.merge_global {
        merge_global_into(&mut config);
    }
    if opts.apply_env_overrides {
        config.apply_process_env();
    }
    config.interpolate_env_vars();
    config.resolve_file_secrets();

    // Emit diagnostics as warnings so callers don't need to opt into
    // load_config_validated() to see slug duplicates and orphaned models.
    for diag in collect_diagnostics(&config) {
        if diag.key.starts_with('_') {
            // Skip the env-override meta-note; it's noise on the hot path.
            continue;
        }
        tracing::warn!(
            config_key = %diag.key,
            "config warning: {}",
            diag.message
        );
    }

    Ok(config)
}
```

Do NOT add a `collect_diagnostics()` call in `load_config_unified()` itself — the helper
`load_from_resolved_path()` is the shared bottom layer, so the warning fires from both
`load_config_unified()` and `load_config_with_options()` automatically.

### 3. ALREADY EXISTS — wire/verify only: `global_config_path` in `AcpConfig`

The `global_config_path: Option<PathBuf>` field already exists in `crates/roko-acp/src/config.rs`
(line 18), along with `with_global_config()` builder (line 42) and merge logic (lines 56, 85, 91).
The `--global-config` CLI flag already exists in `crates/roko-cli/src/main.rs` (line 600) and is
forwarded at both ACP dispatch sites (lines 1892/1900 and 2413/2420). The `config_sources()` method
already exists in `crates/roko-acp/src/config.rs` (line 54) and is wired into `InitializeResult`
via `crates/roko-acp/src/handler.rs` (lines 97, 184) and `crates/roko-acp/src/types.rs` (line 185).

**Do not reimplement these.** Verification only: confirm the paths compile, the flag is accepted at
runtime, and `configSources` appears in the `initialize` JSON-RPC response.

### 4. Verify post-merge validation for provider references in `merge_global_into()`

The current branch already logs missing provider references after merge. Verify the check remains
at the bottom of `merge_global_into()` and emits a dedicated warning for any model whose provider
key is missing after project + global providers are merged:

```rust
pub fn merge_global_into(config: &mut RokoConfig) {
    // ... existing merge logic unchanged ...

    // Post-merge: validate model→provider references now that both layers are present.
    for (model_key, profile) in &config.models {
        if !config.providers.contains_key(&profile.provider) {
            tracing::warn!(
                model = %model_key,
                provider = %profile.provider,
                "model references provider '{}' which is missing after global+project merge",
                profile.provider,
            );
        }
    }
}
```

This is a targeted addition to the bottom of `merge_global_into()` — do not alter any of
the existing merge logic above it.

### 5. Verify `ConfigWatcher` reload wiring in `roko-acp`

The current branch already has `crates/roko-acp/src/config_watch.rs`, and
`handler.rs` already drives the watcher from the stdio loop. Do not rewrite
`SessionManager` to own `Arc<ConfigWatcher>` unless the existing handler-driven path cannot
be made correct.

Validate and, if needed, adjust this call chain:

```text
handler::handle_stdio()
  -> ConfigWatcher::changed()
  -> AcpConfig::load_roko_config()
  -> SessionManager::replace_roko_config()
  -> SessionManager::revalidate_all_sessions()
  -> AcpSession::revalidate_config_state()
  -> config-option notification
```

Mechanical checks:
- `SessionManager` may continue storing a plain `RokoConfig` if
  `replace_roko_config()` updates it and revalidates active sessions.
- `ConfigWatcher` must watch both project and global config paths from `AcpConfig`.
- The reload path must not run on every request; it should be event/change driven from the
  handler loop.
- Add a regression test around `replace_roko_config()`/`revalidate_all_sessions()` if an
  end-to-end stdio handler test is too heavy.

Check all callers of `SessionManager::new()` in `roko-acp` and keep their signatures aligned
with the current handler-driven design:

```bash
grep -rn 'SessionManager::new' crates/roko-acp/ --include='*.rs' | grep -v target/
```

### 6. Verify `roko config export --env <target>` subcommand

The current branch already has `ConfigCmd::Export` and `cmd_export()`. Verify the implementation
matches this behavior; do not add a second command or duplicate match arm:

```rust
ConfigCmd::Export { workdir, env } => {
    let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
    let config = roko_core::config::loader::load_config_unified(&wd)
        .context("failed to load config")?;

    match env.as_deref() {
        Some("railway") | Some("Railway") => {
            // Print env vars that Railway needs to replicate the effective config.
            // Use ROKO_* named vars for known fields; document that ROKO__SECTION__FIELD
            // hierarchical overrides are not yet implemented.
            println!("# Roko config export for Railway deployment");
            println!("# Set these as Railway environment variables.");
            println!();
            if !config.agent.default_model.is_empty() {
                println!("ROKO_MODEL={}", config.agent.default_model);
            }
            if !config.agent.default_backend.is_empty() {
                println!("ROKO_BACKEND={}", config.agent.default_backend);
            }
            if !config.agent.default_effort.is_empty() {
                println!("ROKO_EFFORT={}", config.agent.default_effort);
            }
            println!("ROKO_BUDGET_USD={:.2}", config.budget.max_plan_usd);
            println!("ROKO_MAX_AGENTS={}", config.conductor.max_agents);
            println!("ROKO_PARALLEL={}", config.conductor.parallel_enabled);
            println!();
            println!("# Provider API keys (set to actual values in Railway dashboard):");
            for (name, provider) in &config.providers {
                if let Some(ref key_env) = provider.api_key_env {
                    println!("# {name}: {key_env}=<your-key>");
                }
            }
        }
        Some(target) => {
            anyhow::bail!("unknown export target '{}'; supported: railway", target);
        }
        None => {
            anyhow::bail!("--env <target> is required; supported targets: railway");
        }
    }
    Ok(())
}
```

Ensure the `Export` variant remains present in `ConfigCmd` in `crates/roko-cli/src/main.rs`
(or wherever `ConfigCmd` is defined — check with
`grep -rn 'enum ConfigCmd' crates/ --include='*.rs'`):

```rust
/// Export config as environment variables for a deployment target.
Export {
    /// Working directory (default: current directory).
    #[arg(long)]
    workdir: Option<PathBuf>,
    /// Deployment target (currently only "railway" is supported).
    #[arg(long)]
    env: Option<String>,
},
```

### 7. Keep `docker/railway.roko.toml` deleted

The current branch already deleted the file. Check whether any CI scripts, Dockerfiles, or other
files still reference it:

```bash
grep -rn 'railway.roko.toml' . --include='*.yml' --include='*.yaml' \
    --include='Dockerfile*' --include='*.toml' --include='*.sh' | grep -v target/
```

If references exist, update them to use the env var approach instead. After deletion, no
`docker/railway.roko.toml` should exist anywhere in the repo.

Do NOT create a `docker/RAILWAY.md` or any other documentation file to replace it — the
`roko config export --env railway` command is the replacement documentation.

## What NOT to Do

- Do NOT rewrite `collect_diagnostics()`. It is already correct. Only change where it
  runs (add it to the load hot-path via `load_from_resolved_path()`).

- Do NOT add a new `ConfigWarning` enum — the existing `ConfigDiagnostic` struct in
  `provenance.rs` is the correct type. Use it as-is.

- Do NOT make `ConfigCache::new()` fallible with a hard error. The watcher setup failing
  is not fatal — log the failure and continue without watching. Only the initial load
  failure should propagate as `Err`.

- Do NOT change the `arc_swap` setup to use `ArcSwap::empty()` — `ArcSwap` requires
  the slot to always hold a valid `Arc`. Initialize with the initial config.

- Do NOT add an async channel from `notify` callback to `SessionManager` in this task.
  The poll-on-request model (calling `revalidate_all_sessions()` before each request) is
  sufficient for Phase 10.3. The channel-based push model is a follow-on.

- Do NOT add `notify` or `arc-swap` to the workspace-level `[dependencies]` table in the
  root `Cargo.toml`. Add them only to `crates/roko-core/Cargo.toml` where they are needed.
  `arc-swap` is already in `roko-serve`; use `arc-swap = { workspace = true }` if it is
  already in the workspace table, or `arc-swap = "1"` as a crate-level dep if not.

- Do NOT implement the full `ROKO__SECTION__FIELD` hierarchical override system. The export
  command uses only the named `ROKO_*` vars (model, backend, effort, budget, agents,
  parallel) that are already parsed by `apply_process_env()`.

- Do NOT change `SessionManager::new()` to be async. Keep it synchronous.

## Wire Target

```bash
# 1. Verify config export subcommand:
cargo run -p roko-cli -- config export --env railway
# Expected: prints ROKO_MODEL=..., ROKO_BACKEND=..., ROKO_BUDGET_USD=..., etc.

# 2. Verify railway.roko.toml is deleted:
ls /Users/will/dev/nunchi/roko/roko/docker/railway.roko.toml
# Expected: No such file or directory.

# 3. Verify duplicate-slug warning fires on the hot path:
RUST_LOG=roko_core=warn cargo run -p roko-cli -- status 2>&1 | grep "duplicate model slug"
# Expected: warn line fires if roko.toml has duplicate slugs (kimi-k25/kimi-k2-5).

# 4. Verify post-merge provider warning fires when a model references a
#    missing provider:
RUST_LOG=roko_core=warn cargo run -p roko-cli -- status 2>&1 | \
    grep "missing after global+project merge" || echo "no orphaned models (expected if config is clean)"

# 5. Verify ConfigCache is exported from roko-core:
grep -rn 'ConfigCache' crates/roko-core/src/config/ --include='*.rs' | grep -v target/
# Expected: config/cache.rs defines it; config/mod.rs re-exports it.

# 6. Verify ACP SessionManager uses ConfigWatcher:
grep -rn 'ConfigWatcher\|config_watcher' crates/roko-acp/src/session.rs | grep -v target/
# Expected: struct field and constructor parameter present.
```

## Verification

- [ ] `cargo build --workspace` — clean build with no new errors
- [ ] `cargo test --workspace` — no regressions
- [ ] `cargo clippy --workspace --no-deps -- -D warnings` — clean
- [ ] `cargo run -p roko-cli -- config export --env railway` — prints valid env vars
- [ ] `cargo run -p roko-cli -- config export --env unknown` — exits non-zero with
  "unknown export target" message
- [ ] `ls docker/railway.roko.toml` — exits non-zero (file does not exist)
- [ ] `grep -rn 'railway.roko.toml' . --include='*.yml' --include='*.yaml' --include='Dockerfile*' | grep -v target/` — no references remain
- [ ] `grep -rn 'ConfigCache' crates/roko-core/src/config/mod.rs` — exported
- [ ] `grep -rn 'config_watcher\|ConfigWatcher' crates/roko-acp/src/session.rs | grep -v target/` — wired
- [ ] `grep -rn 'arc-swap\|arc_swap' crates/roko-core/Cargo.toml` — dep is present
- [ ] `grep -rn 'notify' crates/roko-core/Cargo.toml` — dep is present
- [ ] `RUST_LOG=roko_core=warn cargo run -p roko-cli -- status 2>&1` — no panic, no new `[ERROR]` lines
- [ ] No `TODO`, `FIXME`, `unimplemented!()`, or `todo!()` in new files

## Status Log

| Time | Agent | Action |
|------|-------|--------|
