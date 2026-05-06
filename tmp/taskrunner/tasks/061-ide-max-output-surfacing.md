# Task 061: IDE/ACP max_output Surfacing

```toml
id = 61
title = "Add effective_max_output() method and surface max_output in config options + diagnostics"
track = "ide-acp"
wave = "wave-1"
priority = "medium"
blocked_by = [2]
touches = [
    "crates/roko-core/src/config/provider.rs",
    "crates/roko-core/src/config/loader.rs",
    "crates/roko-acp/src/session.rs",
]
exclusive_files = ["crates/roko-core/src/config/provider.rs"]
estimated_minutes = 20
```

## Context

The effective max_output tokens value is invisible to the IDE. `ModelProfile::max_output` is
`Option<u64>` defaulting to `None`, which falls back to `DEFAULT_MAX_OUTPUT_TOKENS = 16_384`
deep in agent dispatch. Users cannot tell what value is in effect, and setting a suspiciously
low value (e.g. 500) truncates responses silently with no warning.

This task adds:
1. An `effective_max_output()` method on `ModelProfile` so callers get the resolved value
2. The effective max_output shown in model option descriptions sent to the IDE
3. A config diagnostic warning when `max_output < 1000`

Sources:
- `tmp/solutions/ide/CHECKLIST.md` — Agent 3I: max_output surfacing (items 3.18-3.20)
- `tmp/solutions/ide/batches/W4-A-max-output-surfacing.md` — detailed FIND/REPLACE
- `tmp/solutions/ide/11-max-output-default.md` — original issue analysis

## Background

Read these files before starting:
- `crates/roko-core/src/config/provider.rs` — `ModelProfile` struct, currently has no `impl` block
- `crates/roko-core/src/defaults.rs` — `DEFAULT_MAX_OUTPUT_TOKENS` constant (line 32, value 16,384)
- `crates/roko-acp/src/session.rs` — `build_config_options()`, model options (lines 960-971)
- `crates/roko-core/src/config/loader.rs` — `collect_diagnostics()` (lines 213-262)

The batch file has EXACT FIND/REPLACE blocks: `tmp/solutions/ide/batches/W4-A-max-output-surfacing.md`

## What to Change

### 1. Add `effective_max_output()` method to `ModelProfile` (provider.rs)

There is no existing `impl ModelProfile` block. Add one after the struct closing brace (line 461),
before the GeminiConfig section:

```rust
impl ModelProfile {
    /// Returns the effective max output tokens, falling back to the system default.
    ///
    /// When `self.max_output` is `None`, returns `DEFAULT_MAX_OUTPUT_TOKENS` (16,384).
    /// This mirrors the runtime fallback in agent dispatch.
    pub fn effective_max_output(&self) -> u64 {
        self.max_output
            .unwrap_or(crate::defaults::DEFAULT_MAX_OUTPUT_TOKENS as u64)
    }
}
```

### 2. Show effective max_output in model option descriptions (session.rs)

In `build_config_options` (lines 960-971), change the model option map closure to include
max_output in the description. Change the description from `profile.slug.clone()` to
`format!("{} (max output: {})", profile.slug, profile.effective_max_output())`.

### 3. Add config diagnostic for low max_output (loader.rs)

In `collect_diagnostics()`, before the final `diagnostics` return (lines 259-262), add a loop
over `config.models` that pushes a `ConfigDiagnostic` when `max_output` is explicitly set and
less than 1000.

## What NOT to Do

- Do NOT change `DEFAULT_MAX_OUTPUT_TOKENS` value (16,384 is correct).
- Do NOT change `ModelProfile::max_output` type from `Option<u64>`.
- Do NOT change agent dispatch code — the runtime fallback is already correct.
- Do NOT modify `build_config_options` beyond the model option descriptions.
- Do NOT add a new `impl ModelProfile` block if one already exists — extend it.

## Wire Target

```bash
# Model options should show max output in description
echo '{"jsonrpc":"2.0","method":"session/new","id":1,"params":{}}' \
  | cargo run -p roko-cli -- acp --quiet --no-serve --config ~/.nunchi/roko/roko.toml 2>/dev/null \
  | head -1 | python3 -c "
import sys,json; d=json.load(sys.stdin)
for o in d['result'].get('configOptions') or []:
  if o['id']=='model':
    for opt in o.get('options', []):
      print(f\"{opt['value']}: {opt.get('description', 'no desc')}\")
"
# EXPECTED: "sonnet: claude-sonnet-4-20250514 (max output: 16384)" or similar
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `grep -rn 'effective_max_output' crates/ --include='*.rs' | grep -v target/ | grep -v test` shows callsites
- [ ] Model option descriptions include "(max output: N)" in session/new response
- [ ] Config with `max_output = 500` produces a diagnostic warning

## Implementation Notes for Later Agent

Current branch facts to verify before editing:
- `ModelProfile::effective_max_output()` may already exist in
  `crates/roko-core/src/config/provider.rs`; if so, extend or test it rather than
  adding a second impl.
- The runtime fallback constant is `roko_core::defaults::DEFAULT_MAX_OUTPUT_TOKENS`
  (`16_384`, `u32`). The helper should return `u64` and use `u64::from(...)`.
- ACP session option flow:
  `crates/roko-cli/src/main.rs` `Command::Acp` ->
  `roko_acp::run_acp_server` -> `crates/roko-acp/src/handler.rs`
  `session/new` -> `SessionManager::create_session` ->
  `AcpSession::new_with_config` -> `build_config_options`.
- Config diagnostic flow:
  `roko_core::config::loader::load_config_validated` ->
  internal `collect_diagnostics(&RokoConfig)` -> `ValidatedConfig::diagnostics()`.

Mechanical steps:
1. In `provider.rs`, add or verify exactly one `impl ModelProfile` containing
   `pub fn effective_max_output(&self) -> u64`.
2. In `session.rs::build_config_options`, update only the model option
   `description`; keep filtering by `profile.provider == state.provider`, sorting by
   value, and all existing `ConfigOptionValue` fields. If task 062 has added `ready`,
   preserve that field in the same literal.
3. In `loader.rs::collect_diagnostics`, warn only when `profile.max_output` is
   explicitly `Some(n)` and `n < 1000`. Do not warn for `None`, because `None` resolves
   to the 16K default.
4. Keep the diagnostic key stable: `models.{model_key}.max_output`.

Tests to add or update:
- `roko-core/src/config/provider.rs` or an existing config test: `None` resolves to
  `16_384`; `Some(500)` resolves to `500`.
- `roko-core/src/config/loader.rs` tests: a temp `roko.toml` with
  `max_output = 500` appears in `load_config_validated(...).diagnostics()`, while a
  model with no `max_output` does not produce that warning.
- `roko-acp/src/session.rs` tests: `session.config_options()` model option
  descriptions contain `(max output: 16384)` for omitted values and the explicit number
  for configured values.

Additional verification commands:
```bash
rg 'effective_max_output' crates/roko-core/src crates/roko-acp/src -g '*.rs'
# Expected: one definition on ModelProfile and a non-test call from build_config_options.

cargo test -p roko-core config::loader::tests -- --nocapture
cargo test -p roko-acp session::tests -- --nocapture
```

What not to do:
- Do not change provider dispatch code or any `max_tokens` request construction.
- Do not change `max_output: Option<u64>` or reinterpret `None` as unlimited.
- Do not make the ACP description depend on provider availability; that belongs to task 062.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
