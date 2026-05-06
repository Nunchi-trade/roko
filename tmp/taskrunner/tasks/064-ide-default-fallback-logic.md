# Task 064: IDE/ACP Default Model/Provider Fallback Logic

```toml
id = 64
title = "Prefer first ready provider in default model fallback and add tracing::warn on miss"
track = "ide-acp"
wave = "wave-1"
priority = "high"
blocked_by = [2]
touches = [
    "crates/roko-acp/src/session.rs",
]
exclusive_files = []
estimated_minutes = 20
```

## Context

When `session/new` creates a session and the configured `agent.default_model` does not match
any key in `[models.*]`, the fallback picks `config.models.keys().next()`. After the IndexMap
migration (task 002), this is deterministic (first TOML-declared model), but it may pick a
model whose provider is not ready (no API key). This task makes the fallback smarter:

1. When `agent.default_model` is set but not found, emit `tracing::warn`
2. Prefer the first model (TOML declaration order) whose provider is actually ready
3. Fall back to first model regardless only if no provider is ready
4. Same pattern for provider fallback

Sources:
- `tmp/solutions/ide/CHECKLIST.md` — Agent 2E: Default fallback logic (items 2.14-2.15)
- `tmp/solutions/ide/batches/W1-B-default-fallback-logic.md` — detailed FIND/REPLACE
- `tmp/solutions/ide/03-deterministic-defaults.md` — original issue analysis

## Background

Read these files before starting:
- `crates/roko-acp/src/session.rs` — `SessionConfigState::from_roko_config()` (lines 175-204)
- `crates/roko-core/src/config/schema.rs` — `is_provider_available()` (line 436)

The batch file has the EXACT replacement: `tmp/solutions/ide/batches/W1-B-default-fallback-logic.md`

## What to Change

### 1. Rewrite `from_roko_config` fallback logic (session.rs, lines 175-204)

Replace the entire `SessionConfigState::from_roko_config` method. The key changes:

**Model fallback:**
```rust
let default_model =
    if !configured_default.is_empty() && config.models.contains_key(configured_default) {
        Some(configured_default)
    } else {
        if !configured_default.is_empty() {
            tracing::warn!(
                configured = configured_default,
                "agent.default_model not found in [models.*], falling back"
            );
        }
        // Prefer the first model (TOML declaration order) whose provider is ready.
        config
            .models
            .iter()
            .find(|(_, profile)| {
                config
                    .providers
                    .get(&profile.provider)
                    .map(|p| config.is_provider_available(p))
                    .unwrap_or(false)
            })
            .map(|(key, _)| key.as_str())
            // Fall back to the first model regardless of provider readiness.
            .or_else(|| config.models.keys().next().map(String::as_str))
    };
```

**Provider fallback:**
```rust
let default_provider = default_model
    .and_then(|model| config.models.get(model))
    .map(|profile| profile.provider.clone())
    .or_else(|| {
        // Prefer the first ready provider in declaration order.
        config
            .providers
            .iter()
            .find(|(_, p)| config.is_provider_available(p))
            .map(|(k, _)| k.clone())
            .or_else(|| config.providers.keys().next().cloned())
    })
    .unwrap_or_default();
```

The rest of the `Self { ... }` constructor remains unchanged.

## What NOT to Do

- Do NOT modify `update_config` (lines 560-652) — it uses `.min()` which is deterministic.
- Do NOT modify `revalidate_config_state` (lines 655-704) — it delegates to from_roko_config.
- Do NOT touch any other files.
- Do NOT add new fields to `SessionConfigState`.

## Wire Target

```bash
# With agent.default_model set to a nonexistent key, run 5 times — must always pick same default
for i in {1..5}; do
  echo '{"jsonrpc":"2.0","method":"session/new","id":1,"params":{}}' \
    | cargo run -p roko-cli -- acp --quiet --no-serve --config ~/.nunchi/roko/roko.toml 2>/dev/null \
    | head -1 | python3 -c "
import sys,json; d=json.load(sys.stdin)
for o in d['result'].get('configOptions') or []:
  if o['id']=='model': print(o['currentValue'])
" 2>/dev/null
done | sort -u | wc -l
# EXPECTED: prints "1" (same default every time, prefers ready provider)
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] Default model is deterministic across runs (always the same)
- [ ] Default model prefers a model whose provider has API keys configured
- [ ] `tracing::warn` emitted when `agent.default_model` is set but not found
- [ ] When all providers are ready, first TOML-declared model is selected

## Implementation Notes for Later Agent

Current branch facts to verify before editing:
- `SessionConfigState::from_roko_config` may delegate to
  `from_roko_config_with_warnings`. If that helper exists, update the helper rather
  than replacing only the wrapper.
- Current `SessionConfigState` may not contain every field shown in the older batch
  docs (for example no `routing_mode`). Preserve the live struct fields; do not add
  fields for this task.
- Readiness comes from `RokoConfig::is_provider_available()` in
  `crates/roko-core/src/config/schema.rs`. It is based on configured provider kind and
  local credential availability; it is not a network health check.
- ACP runtime chain:
  `roko acp` -> `run_acp_server` -> `handler.rs` `session/new` ->
  `SessionManager::create_session` -> `AcpSession::new_with_config` ->
  `SessionConfigState::from_roko_config(_with_warnings)` ->
  `build_config_options` current values.

Mechanical fallback rules:
1. Read `configured_default = config.agent.default_model.trim()`.
2. If `configured_default` names a model whose provider exists and is ready, select it.
3. If `configured_default` is non-empty but missing from `config.models`, emit
   `tracing::warn!` and, when the warnings helper exists, push a user-visible warning.
4. If `configured_default` names a model whose provider is missing or not ready, warn and
   prefer the first ready model in `config.models` declaration order.
5. If no ready model exists, fall back to the first declared model regardless of provider
   readiness.
6. Set `default_provider` from the selected model's profile when a model was selected.
   If no model exists at all, prefer the first ready provider in declaration order, then
   fall back to the first provider, then `""`.
7. Keep `revalidate_config_state` delegating to `from_roko_config`; do not duplicate this
   selection logic there.

Tests to add or update in `crates/roko-acp/src/session.rs`:
- Invalid `agent.default_model` with first provider not ready and second provider ready
  selects the first model whose provider is ready and returns/emits a warning.
- Valid configured default with ready provider is preserved even when another ready model
  appears earlier in TOML.
- Valid configured default with not-ready provider falls back to first ready model.
- No ready providers falls back to the first declared model deterministically.
- No models but multiple providers selects the first ready provider; if none are ready,
  selects the first declared provider.
- Run the same config several times and assert `SessionConfigState::from_roko_config`
  returns identical `provider` and `model`.

Additional verification commands:
```bash
rg 'from_roko_config_with_warnings|agent.default_model.*not.*declared|first ready model' \
  crates/roko-acp/src/session.rs

cargo test -p roko-acp session::tests -- --nocapture

for i in 1 2 3 4 5; do
  echo '{"jsonrpc":"2.0","method":"session/new","id":1,"params":{}}' \
    | cargo run -p roko-cli -- acp --quiet --no-serve --config /tmp/test_ordering.toml 2>/dev/null \
    | head -1 | python3 -c 'import json,sys; d=json.load(sys.stdin); print(next(o["currentValue"] for o in d["result"]["configOptions"] if o["id"]=="model"))'
done | sort -u
# Expected: exactly one unique model, and it belongs to the first ready provider.
```

What not to do:
- Do not use `.min()` or alphabetical sorting for initial defaults; TOML declaration
  order from `IndexMap` is the intended priority.
- Do not require `agent.default_model` to exist as a hard error in this task; warn and
  choose a deterministic fallback.
- Do not make provider fallback depend on the model option filter in
  `build_config_options`; selection belongs in `from_roko_config`.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
