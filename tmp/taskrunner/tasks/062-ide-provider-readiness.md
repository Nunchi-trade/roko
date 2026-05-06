# Task 062: IDE/ACP Provider Readiness Boolean

```toml
id = 62
title = "Add ready: bool to ConfigOptionValue for structured provider/model readiness"
track = "ide-acp"
wave = "wave-1"
priority = "medium"
blocked_by = [2]
touches = [
    "crates/roko-acp/src/types.rs",
    "crates/roko-acp/src/session.rs",
]
exclusive_files = []
estimated_minutes = 20
```

## Context

Provider readiness is reported as freeform description strings ("Ready" / "API key env X is
not set"). The IDE has to parse these strings to determine if a provider is usable. Adding a
structured `ready: bool` field to `ConfigOptionValue` lets clients filter and display provider
state without string parsing.

The readiness check already exists: `RokoConfig::is_provider_available()` (schema.rs:436)
returns `bool`. This task wires it into the JSON config options response.

Sources:
- `tmp/solutions/ide/CHECKLIST.md` — Agent 3J: Provider readiness (items 3.21-3.24)
- `tmp/solutions/ide/batches/W4-B-provider-readiness.md` — detailed FIND/REPLACE
- `tmp/solutions/ide/09-provider-readiness.md` — original issue analysis

## Background

Read these files before starting:
- `crates/roko-acp/src/types.rs` — `ConfigOptionValue` struct (lines 586-597)
- `crates/roko-acp/src/session.rs` — `build_config_options()`, provider options (lines 948-957),
  model options (lines 960-971)
- `crates/roko-core/src/config/schema.rs` — `is_provider_available()` method (line 436)

The batch file has EXACT FIND/REPLACE blocks: `tmp/solutions/ide/batches/W4-B-provider-readiness.md`

## What to Change

### 1. Add `ready: bool` field to `ConfigOptionValue` (types.rs)

Add a new field to the struct with appropriate serde attributes:

```rust
/// Whether this option is usable (e.g., provider has API key configured).
/// When `true`, serialized in JSON. When `false`, omitted from JSON (default).
#[serde(default, skip_serializing_if = "std::ops::Not::not")]
pub ready: bool,
```

Serde behavior: `ready: true` appears in JSON; `ready: false` is omitted entirely. Old clients
that do not know about this field see no change.

### 2. Set `ready` on provider options (session.rs)

In `build_config_options` provider option construction (lines 948-957), add:
```rust
ready: roko_config.is_provider_available(provider),
```

### 3. Set `ready` on model options (session.rs)

In `build_config_options` model option construction (lines 960-971), look up the model's
provider and check readiness:
```rust
let provider_ready = roko_config
    .providers
    .get(&profile.provider)
    .map(|p| roko_config.is_provider_available(p))
    .unwrap_or(false);
// ... then set `ready: provider_ready` in the ConfigOptionValue
```

### 4. Add `ready: true` to all other ConfigOptionValue construction sites

Search with: `grep -rn 'ConfigOptionValue {' crates/roko-acp/src/ --include='*.rs' | grep -v target/`

Non-provider/non-model options (effort, temperament, routing_mode, etc.) should use
`ready: true` since those options are always available.

## What NOT to Do

- Do NOT modify `ConfigOption` struct (the parent that holds `Vec<ConfigOptionValue>`).
- Do NOT modify `is_provider_available()` in schema.rs.
- Do NOT modify `provider_option_description()` — keep the text description alongside the bool.
- Do NOT remove the description field — it provides useful context to the IDE.

## Wire Target

```bash
# Provider options should show ready field for available providers
echo '{"jsonrpc":"2.0","method":"session/new","id":1,"params":{}}' \
  | cargo run -p roko-cli -- acp --quiet --no-serve --config ~/.nunchi/roko/roko.toml 2>/dev/null \
  | head -1 | python3 -c "
import sys,json; d=json.load(sys.stdin)
for o in d['result'].get('configOptions') or []:
  if o['id']=='provider':
    for opt in o.get('options', []):
      print(f\"{opt['value']}: ready={opt.get('ready', False)}\")
"
# EXPECTED: "openai: ready=True", providers without API keys show ready=False (field absent)
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `grep -rn 'ConfigOptionValue' crates/roko-acp/ --include='*.rs' | grep -v target/` — all construction sites set `ready`
- [ ] Providers with API keys show `"ready": true` in JSON
- [ ] Providers without API keys omit `ready` from JSON (not `"ready": false`)
- [ ] Non-provider options (effort, temperament) include `ready: true`

## Implementation Notes for Later Agent

Current branch facts to verify before editing:
- `ConfigOptionValue.ready` may already exist in `crates/roko-acp/src/types.rs`. The
  inspected branch used `#[serde(default = "default_true")]`, which serializes
  `ready: false`. The target contract for this task is different: missing/false should
  deserialize as `false` and `ready: false` should be omitted from JSON.
- `build_config_options` in `crates/roko-acp/src/session.rs` is the only intended
  runtime producer for ACP config option values. Search all struct literals before
  changing the type:
  `rg 'ConfigOptionValue \\{' crates/roko-acp/src -g '*.rs'`.
- Readiness source is `RokoConfig::is_provider_available()` in
  `crates/roko-core/src/config/schema.rs`. Do not duplicate its API-key logic.
- ACP wire flow is the same as task 061:
  `roko acp` -> `run_acp_server` -> `handler.rs` `session/new` ->
  `SessionManager::create_session` -> `AcpSession::new_with_config` ->
  `build_config_options` -> JSON `configOptions`.

Mechanical steps:
1. In `types.rs`, make the field contract exact:
   `#[serde(default, skip_serializing_if = "std::ops::Not::not")] pub ready: bool`.
   Remove any `default_true()` helper if it becomes unused.
2. In provider options, set `ready: roko_config.is_provider_available(provider)`.
   Keep `provider_option_description(...)`; clients need both the bool and the reason.
3. In model options, resolve `profile.provider` through `roko_config.providers`; set
   `ready` to that provider's availability, or `false` if the provider is missing.
   Preserve task 061's max-output description if present.
4. Add `ready: true` to every non-provider/model option literal because those choices
   are always selectable.
5. Run `cargo fmt` after changing struct literals; this file has many multiline options.

Tests to add or update:
- In `crates/roko-acp/src/types.rs` tests, serialize
  `ConfigOptionValue { ready: false, ... }` and assert the JSON has no `ready` key;
  serialize `ready: true` and assert `"ready": true`.
- In `crates/roko-acp/src/session.rs` tests, build one provider with empty
  `api_key_env` (ready) and one with an unset env var (not ready). Assert provider
  and model options mirror provider readiness.
- Add a regression test that all non-provider options (`effort`, `workflow`, `clippy`,
  `tests`, etc.) contain `ready: true` in the serialized `session/new` payload.

Additional verification commands:
```bash
rg 'ConfigOptionValue \\{' crates/roko-acp/src -g '*.rs'
# Expected: every literal has an explicit ready field.

echo '{"jsonrpc":"2.0","method":"session/new","id":1,"params":{}}' \
  | cargo run -p roko-cli -- acp --quiet --no-serve 2>/dev/null \
  | head -1 | python3 -c 'import json,sys; d=json.load(sys.stdin); print(json.dumps(d["result"]["configOptions"]))'
# Expected: ready providers/options have `"ready": true`; unavailable providers omit the key.
```

What not to do:
- Do not serialize `"ready": false` unless the task contract is explicitly changed.
- Do not treat a provider as fully healthy; this is only local credential/config
  readiness, not a live network/model/quota check.
- Do not remove or parse the human-readable description text.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
