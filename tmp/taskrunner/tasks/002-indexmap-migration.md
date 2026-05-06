# Task 002: IndexMap Migration for Deterministic Iteration

```toml
id = 2
title = "Replace HashMap with IndexMap for providers/models config"
track = "config-foundation"
wave = "wave-0"
priority = "critical"
blocked_by = []
touches = [
    "Cargo.toml",
    "crates/roko-core/Cargo.toml",
    "crates/roko-core/src/config/registry.rs",
    "crates/roko-core/src/config/schema.rs",
    "crates/roko-agent/Cargo.toml",
    "crates/roko-agent/src/dispatch_resolver.rs",
    "crates/roko-agent/src/provider/mod.rs",
    "crates/roko-agent/src/task_runner.rs",
    "crates/roko-cli/Cargo.toml",
    "crates/roko-cli/src/config.rs",
    "crates/roko-cli/src/commands/config_cmd.rs",
    "crates/roko-cli/src/dispatch/model_routing.rs",
    "crates/roko-cli/src/model_selection.rs",
    "crates/roko-cli/src/plan_validate.rs",
    "crates/roko-cli/tests/plan_validation.rs",
    "crates/roko-learn/Cargo.toml",
    "crates/roko-learn/src/cost_table.rs",
    "crates/roko-serve/Cargo.toml",
    "crates/roko-serve/src/routes/providers.rs",
]
exclusive_files = ["crates/roko-core/src/config/schema.rs"]
estimated_minutes = 90
```

## Context

Original bug: `providers` and `models` fields in `RokoConfig` used `HashMap`. HashMap iteration
is non-deterministic, so the "first available provider" fallback could pick different providers
on different runs. This caused BUG#03 in the IDE audit (non-deterministic defaults).

Current branch note: most of the type migration already appears present. Treat this task as
"finish and verify deterministic config order", including remaining cleanup and CLI observability,
not as permission to redo unrelated maps.

Source: `tmp/solutions/ide/CHECKLIST.md` — Group 0 (IndexMap migration)

## Background

Read these files first:
1. `crates/roko-core/src/config/schema.rs` — `RokoConfig` struct, `providers` and `models` fields
2. `crates/roko-agent/src/dispatch_resolver.rs` — iterates over providers
3. `crates/roko-agent/src/provider/mod.rs` — iterates over providers
4. `crates/roko-cli/src/config.rs` — config helpers that iterate providers/models

The IDE solution docs have EXACT line numbers for every change: `tmp/solutions/ide/batches/W0-A-indexmap-migration.md`

Current branch facts to verify before editing:
- The dependency entries already exist in root `Cargo.toml` and in `roko-core`, `roko-agent`,
  `roko-cli`, `roko-learn`, and `roko-serve`.
- `RokoConfig.providers`, `RokoConfig.models`, CLI `Config.providers`, CLI `Config.models`,
  `effective_providers()`, `effective_models()`, and most consumer signatures already use
  `indexmap::IndexMap`.
- One likely cleanup left in the current branch: `crates/roko-core/src/config/registry.rs`
  has `ModelRegistry { profiles: IndexMap<...> }` but its `Default` impl still initializes
  `profiles` with `HashMap::new()`. Fix that to `IndexMap::new()`.
- The CLI wire target currently sorts provider names in
  `crates/roko-cli/src/commands/config_cmd.rs::cmd_provider_list()` via `sort_unstable()`.
  That makes TOML order unobservable even when the backing map is ordered. Remove sorting for
  the provider-list command; keep lookup-only maps as `HashMap`.

## What to Change

1. **Start with verification, not blind edits**:
   - Run `rg -n 'indexmap' Cargo.toml crates/*/Cargo.toml`.
   - Run `rg -n 'HashMap<String, ProviderConfig>|HashMap<String, ModelProfile>|HashMap.*providers|HashMap.*models' crates -g '*.rs'`.
2. **Dependency wiring**:
   - If missing, add `indexmap = { version = "2", features = ["serde"] }` to workspace
     `[workspace.dependencies]`.
   - If missing, add `indexmap = { workspace = true }` to `roko-core`, `roko-agent`,
     `roko-cli`, `roko-learn`, and `roko-serve`.
3. **Core schema and registry**:
   - Ensure `crates/roko-core/src/config/schema.rs` uses `IndexMap` only for config-order
     fields: `providers`, `models`, `effective_providers()`, `effective_models()`, and
     `interpolate_env_vars_with()`.
   - In `crates/roko-core/src/config/registry.rs`, make `ModelRegistry::default().profiles`
     use `IndexMap::new()`. Leave `slug_to_key` as `HashMap` because it is lookup-only.
4. **Consumer signatures**:
   - Keep provider/model registry parameters as `IndexMap` in
     `roko-agent/src/dispatch_resolver.rs`, `roko-agent/src/provider/mod.rs`,
     `roko-agent/src/task_runner.rs`, `roko-cli/src/model_selection.rs`,
     `roko-cli/src/plan_validate.rs`, `roko-cli/src/commands/config_cmd.rs`,
     `roko-learn/src/cost_table.rs`, and `roko-serve/src/routes/providers.rs`.
   - Leave local aggregation, health, alias, and lookup maps as `HashMap`.
5. **Make ordering observable at the CLI**:
   - In `cmd_provider_list()`, iterate `providers` directly in `IndexMap` order. Do not collect
     keys and `sort_unstable()` for the list command.
   - If a health/status command intentionally sorts for readability, document that it is not the
     deterministic fallback path and leave it alone unless a test requires TOML order there.
6. **Tests to add/update**:
   - Add or update a focused config test that parses TOML with providers declared in a non-alpha
     order and asserts `effective_providers().keys()` preserves that order.
   - Add or update a CLI formatting/unit test for provider list order if one exists near
     `format_provider_rows()`; otherwise document the manual wire verification.

## What NOT to Do

- Don't replace ALL HashMaps — only `providers` and `models` in config.
- Don't change the TOML file format.
- Don't change any HashMap that's used for lookup-only (not iterated in order).
- Don't sort provider/model keys in code paths whose purpose is "first configured/default"
  behavior or the `config providers list` wire target.
- Don't convert health snapshots, alias tables, slug lookups, or provider-health maps to
  `IndexMap` unless their order is user-visible and sourced from config.

## Wire Target

```bash
# Verify deterministic provider ordering
cargo run -p roko-cli -- config providers list
# Should show providers in TOML file order, not random
```

Expected observable behavior: with a temporary config declaring providers in the order `zai`,
`anthropic`, `ollama`, `config providers list` prints that same order. It must not alphabetize.

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `grep -rn 'HashMap.*providers\|HashMap.*models' crates/roko-core/src/config/ --include='*.rs'` returns nothing
- [ ] Provider listing shows deterministic order matching roko.toml file order
- [ ] `rg -n 'profiles: HashMap::new|HashMap<String, ProviderConfig>|HashMap<String, ModelProfile>' crates/roko-core crates/roko-agent crates/roko-cli crates/roko-learn crates/roko-serve -g '*.rs'` has no config-registry regressions
- [ ] Manual order check:
  ```bash
  tmpdir=$(mktemp -d)
  cat > "$tmpdir/roko.toml" <<'TOML'
  config_version = 2
  [providers.zai]
  kind = "openai_compat"
  api_key_env = ""
  base_url = "https://example.invalid/v1"
  [providers.anthropic]
  kind = "anthropic_api"
  api_key_env = ""
  [providers.ollama]
  kind = "openai_compat"
  api_key_env = ""
  base_url = "http://localhost:11434/v1"
  TOML
  cargo run -p roko-cli -- config providers list --workdir "$tmpdir"
  ```
  Expected first provider rows: `zai`, then `anthropic`, then `ollama`.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
