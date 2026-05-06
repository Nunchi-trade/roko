# Task 090: Provider UX Redesign

```toml
id = 90
title = "Human-readable provider errors, startup pre-flight, provider discovery, config-merge validation"
track = "infrastructure"
wave = "wave-2"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-agent/src/provider/mod.rs",
    "crates/roko-cli/src/commands/config_cmd.rs",
    "crates/roko-core/src/config/loader.rs",
]
exclusive_files = []
estimated_minutes = 240
```

## Context

Four separate but related provider UX failures cause confusing first-run experiences and hard-to-debug
dispatch errors. This task fixes all four at the same architectural level: the provider adapter boundary
and the config loader boundary.

Sources:
- `tmp/model-provider-audit.md` §13 (raw errors), §14 (pre-flight), §15 (discovery)
- `tmp/infrastructure-audit.md` §27.6 (config-merge validation)

The four failures are:

1. **Raw HTTP errors reach users.** A 404 surfaces as `{"error":"model not found"}`. A 401 surfaces
   as `{"error":{"type":"authentication_error"}}`. `claude` binary missing surfaces as
   `spawn failed: No such file or directory`. None of these tell the user what to do.

2. **Provider availability is checked at dispatch time.** Context assembly (system prompt build,
   knowledge query, enrichment) runs for 30-120 seconds before the first provider call. If the
   `claude` binary is absent or `ANTHROPIC_API_KEY` is empty, the failure arrives after all that
   work has already been done. The check belongs at startup.

3. **No way to discover supported providers.** `roko config providers list` shows what is configured;
   it cannot show what is _possible_. `ProviderKind` has seven variants in `roko-core/src/agent.rs`
   (lines 35-51) but none are documented to users. There is no `roko config providers available`
   subcommand.

4. **Post-merge config may have dangling model.provider references.** After global (`~/.roko/config.toml`)
   and project (`roko.toml`) configs are merged, a `[models.*]` entry whose `provider` field names a
   key that no longer exists in the merged `[providers]` map is never caught. The failure surfaces at
   the first dispatch call with an opaque lookup error.

## Background

Read these files before writing any code:

1. `crates/roko-core/src/agent.rs` lines 35-84 — `ProviderKind` enum with `label()` and `to_backend()`.
   Seven variants: `AnthropicApi`, `ClaudeCli`, `OpenAiCompat`, `CursorAcp`, `PerplexityApi`,
   `GeminiApi`, `CerebrasApi`. These are the canonical names for the discovery table in §3.

2. `crates/roko-agent/src/provider/mod.rs` — `adapter_for_kind()` (line ~144) maps each `ProviderKind`
   to a static adapter. This is the correct location for the error-mapping function because every
   failed dispatch passes through here. Also read `shared_http_client()` (line ~133).

3. `crates/roko-agent/src/provider/anthropic_api.rs` — Inspect how HTTP errors from `reqwest` are
   propagated. Errors surface as generic `anyhow::Error`. Status 401 and 429 are raw JSON bodies
   without any human-readable wrapper.

4. `crates/roko-agent/src/provider/claude_cli.rs` — Inspect how `std::process::Command` spawn
   errors (ENOENT) propagate. Raw `std::io::Error` with no human-readable mapping.

5. `crates/roko-cli/src/commands/config_cmd.rs` — `ConfigProviderCmd` dispatch (lines ~113-146).
   `cmd_provider_list()` (line ~269) and `cmd_provider_health()` (line ~299) are the pattern for
   the new `Available` subcommand to follow.

6. `crates/roko-cli/src/main.rs` lines ~1722-1747 — `ConfigProviderCmd` enum with `List`, `Health`,
   `Test` variants. The new `Available` variant must be added here.

7. `crates/roko-core/src/config/loader.rs` — The unified config loader. Find where global and project
   configs are merged. Look for `merge_global_providers` or equivalent. Understand the point after
   merge where model profiles are available and cross-referencing can be done.

8. `crates/roko-core/src/config/schema.rs` — `ProviderConfig` struct, `ModelProfile` struct, and
   `is_provider_available()` on `RokoConfig`. The `api_key_env` field on `ProviderConfig` is
   the source of truth for which env var to display in error messages and discovery output.

## What to Change

### 1. Human-readable error mapping at the dispatch boundary

Add `pub fn map_provider_error(kind: ProviderKind, provider_name: &str, api_key_env: Option<&str>, base_url: Option<&str>, err: &anyhow::Error) -> String`
in `crates/roko-agent/src/provider/mod.rs`. The function inspects `err.to_string()` for known
patterns using simple string matching — do NOT depend on reqwest or std internal types.

Required mappings:

| Error pattern (case-insensitive string match) | Human message |
|---|---|
| "401" or "authentication_error" or "Unauthorized" | "API key invalid for provider '{name}'. Check ${env_var} or roko.toml [providers.{name}]." |
| "429" or "rate_limit" or "Too Many Requests" | "Rate limited by provider '{name}'. Wait and retry, or switch providers." |
| "404" or "model_not_found" or "model not found" | "Model not found on provider '{name}'. Verify the slug in roko.toml [models.*]." |
| "connection refused" or "ConnectError" or "tcp connect error" | "Cannot reach provider '{name}' at {base_url}. Is the server running?" |
| "No such file or directory" or "program not found" or "ENOENT" | "Provider binary not found on PATH for '{name}'. Install it or configure a different provider in roko.toml." |

The `env_var` comes from `api_key_env` (use `"(none)"` for CLI providers). The `base_url` is from
`ProviderConfig::base_url` or the adapter default (use `"(unknown)"` if not available).

Call `map_provider_error` at every call site in `provider/mod.rs` where a provider adapter error
is propagated upward. Wrap the original error with `.context(mapped_message)` — do NOT discard
the original error chain.

### 2. Pre-flight provider check at CLI startup

Add `pub fn check_provider_readiness(config: &RokoConfig) -> Vec<ProviderReadinessIssue>` in
`crates/roko-agent/src/provider/mod.rs` (or a new `pre_flight.rs` module in the same crate).

```rust
pub struct ProviderReadinessIssue {
    pub provider_name: String,
    pub message: String,
}
```

Per-provider checks:
- For `ClaudeCli` kind: run `std::process::Command::new(command).arg("--version").output()` where
  `command` comes from `ProviderConfig::command` or defaults to `"claude"`. On `Err(_)` or
  non-zero exit: add an issue: "claude CLI not found on PATH. Install: https://claude.ai/cli".
- For all API kinds: check that `api_key_env` is set and non-empty via `std::env::var()`. On
  `Err(_)` or empty string: add an issue: "Missing {env_var} for provider '{name}'. Export it
  in your shell or in a .env file."
- For all kinds: do NOT make network requests. PATH and env var checks only.

Only check providers referenced by at least one model profile in `config.models`. Providers that
are configured but unreferenced do not block startup.

The function returns an empty `Vec` when all referenced providers are ready. Callers decide
whether to exit.

Integrate into the CLI boot sequence by calling `check_provider_readiness` in the main
dispatch path before entering any long-running operation (plan run, chat, prd pipeline). Print
each issue to stderr with a `warning:` prefix. If all referenced providers have issues (none
are ready), print to stderr and return a non-zero exit code without starting the operation.

### 3. `roko config providers available` subcommand

Add `Available` to `ConfigProviderCmd` in `crates/roko-cli/src/main.rs`:

```rust
/// List all supported provider kinds with required credentials and setup instructions.
Available,
```

Add `cmd_provider_available()` in `crates/roko-cli/src/commands/config_cmd.rs`. The function
iterates all `ProviderKind` variants (hard-coded: 7 as of this task) and prints a plain-text
table. No config file is required — this command must work even with no `roko.toml`.

Required output format:

```
Available provider kinds:

  anthropic_api      Anthropic Messages API
                     Env: ANTHROPIC_API_KEY
                     Base: https://api.anthropic.com/v1
                     Add: edit roko.toml [providers.anthropic], kind = "anthropic_api"

  claude_cli         Claude CLI (local subprocess)
                     Install: npm install -g @anthropic-ai/claude-cli && claude login
                     Add: edit roko.toml [providers.claude], kind = "claude_cli"

  openai_compat      OpenAI-compatible HTTP API
                     Env: OPENAI_API_KEY
                     Base: https://api.openai.com/v1
                     Works with: OpenAI, Azure, Moonshot, Cerebras, Perplexity, local servers
                     Add: edit roko.toml [providers.openai], kind = "openai_compat"

  cursor_acp         Cursor IDE Agent Control Protocol
                     Auth: Cursor IDE handles authentication
                     Add: edit roko.toml [providers.cursor], kind = "cursor_acp"

  perplexity_api     Perplexity API
                     Env: PPLX_API_KEY
                     Base: https://api.perplexity.ai
                     Add: edit roko.toml [providers.perplexity], kind = "perplexity_api"

  gemini_api         Google Gemini native API (grounding, code execution)
                     Env: GEMINI_API_KEY
                     Base: https://generativelanguage.googleapis.com/v1beta
                     Add: edit roko.toml [providers.gemini], kind = "gemini_api"

  cerebras_api       Cerebras fast inference API
                     Env: CEREBRAS_API_KEY
                     Base: https://api.cerebras.ai/v1
                     Add: edit roko.toml [providers.cerebras], kind = "cerebras_api"

Run `roko config providers list` to see what is currently configured.
```

Wire the new variant in the `dispatch_config()` match in `crates/roko-cli/src/commands/config_cmd.rs`.

### 4. Post-merge provider reference validation in the config loader

In `crates/roko-core/src/config/loader.rs`, after the global and project configs are merged
(after the call to `merge_global_providers` or equivalent), add a validation pass:

```rust
for (model_key, model_profile) in &merged_config.models {
    if !merged_config.providers.contains_key(&model_profile.provider) {
        let msg = format!(
            "model '{}' references provider '{}' which does not exist in the merged config. \
             Check roko.toml [models.{}] and ensure [providers.{}] is defined.",
            model_key, model_profile.provider, model_key, model_profile.provider
        );
        if merged_config.validation.strict_validation {
            return Err(anyhow::anyhow!(msg));
        } else {
            tracing::warn!(
                model = %model_key,
                provider = %model_profile.provider,
                "{}",
                msg,
            );
        }
    }
}
```

**Severity behavior**: By default, missing provider references emit a `tracing::warn!()`
and config loading continues. This matches task 085's post-merge validation approach —
informational, non-blocking.

To elevate to a hard error (useful in CI or strict production environments), set
`strict_validation = true` in `roko.toml`:

```toml
[validation]
strict_validation = true
```

When `strict_validation = true`, any model referencing a nonexistent provider causes
`load_config` to return an error immediately with an actionable message.

The `[validation]` section and `strict_validation` field must be added to the config
schema in `crates/roko-core/src/config/schema.rs`:

```rust
/// Validation behavior configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationConfig {
    /// When true, missing provider references and other config issues
    /// become hard errors instead of warnings. Useful for CI and
    /// strict production environments.
    #[serde(default)]
    pub strict_validation: bool,
}
```

Add a `validation: ValidationConfig` field to `RokoConfig` with `#[serde(default)]`.

The validation runs on every config load, not just at init time. In lenient mode (default),
a dangling provider reference is logged as a warning but does not block startup — the
failure will surface later at dispatch time with a clear message (via the error mapping
in section 1). In strict mode, it fails fast at load time.

If the merged config has zero model profiles, skip the validation (empty config is valid; the
pre-flight check in §2 handles the "no usable provider" case separately).

## What NOT to Do

- Do NOT change `ProviderKind` variants or add new provider kinds. This task is UX wiring only.
- Do NOT make network requests in the pre-flight check. PATH and env var checks only.
- Do NOT change `cmd_provider_list`, `cmd_provider_health`, or `cmd_provider_test`. These
  existing commands are unchanged. Only add the new `Available` subcommand alongside them.
- Do NOT change the TUI or SSE layers. Provider errors surfaced via dashboard are out of scope.
- Do NOT hardcode env var strings anywhere outside of the `cmd_provider_available()` table and
  `map_provider_error()`. The canonical source is `ProviderConfig::api_key_env`.
- Do NOT make `roko config providers available` require a loaded config. It must work with no
  `roko.toml` present.

## Wire Target

```bash
# 1. Discovery: list all 7 provider kinds, no roko.toml required
cargo run -p roko-cli -- config providers available

# 2. Error mapping: verify 401 produces an actionable message
ANTHROPIC_API_KEY=invalid cargo run -p roko-cli -- chat
# Expected: "API key invalid for provider 'anthropic'..." — not raw JSON

# 3. Pre-flight: verify startup warns about missing binary
# (remove claude from PATH or set command to nonexistent binary in roko.toml)
cargo run -p roko-cli -- plan run plans/
# Expected: "warning: claude CLI not found on PATH..." printed before any work starts

# 4. Merge validation: verify dangling provider reference warns at load time (default)
# Add a model entry that points at a provider key not in roko.toml
RUST_LOG=roko_core=warn cargo run -p roko-cli -- status 2>&1
# Expected: warning log "model 'x' references provider 'y' which does not exist..."
# The command still succeeds (exit 0) — the warning is informational.

# 4b. Merge validation: verify strict mode elevates to hard error
# Add `[validation]\nstrict_validation = true` to roko.toml, plus a dangling provider ref
cargo run -p roko-cli -- status
# Expected: exits non-zero with "model 'x' references provider 'y' which does not exist..."
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo run -p roko-cli -- config providers available` lists all 7 `ProviderKind` variants
- [ ] `map_provider_error` returns non-empty human-readable strings for 401, 429, 404, ENOENT inputs
  (unit test with synthetic error strings)
- [ ] `check_provider_readiness` returns issues when the `claude` binary is absent (unit test by
  pointing `command` at a nonexistent path)
- [ ] A config with `models.x.provider = "nonexistent"` (no strict mode) emits a `tracing::warn!()`
  containing the model key and provider key, but config loading succeeds
- [ ] A config with `models.x.provider = "nonexistent"` AND `[validation] strict_validation = true`
  causes `load_config` to return an error whose message contains the model key and provider key
- [ ] `ValidationConfig` struct exists in `schema.rs` with `strict_validation: bool` defaulting to `false`
- [ ] `roko config providers available` runs without a `roko.toml` present
- [ ] Existing `cmd_provider_list`, `cmd_provider_health`, `cmd_provider_test` behavior is unchanged
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in any file touched by this task

## Worker 17 Mechanical Notes

### Current code facts to use

- `ProviderKind` variants and labels live in `crates/roko-core/src/agent.rs`;
  do not duplicate their spelling from memory.
- `create_agent_for_model()` in `crates/roko-agent/src/provider/mod.rs` is the
  construction boundary. It returns `AgentCreationError`, not `anyhow::Error`.
  Runtime model-call failures often surface later through `ModelCallService`
  and concrete agents, so the error-mapping helper should be reusable from both
  construction and execution paths.
- `ProviderAdapter::create_agent()` returns `AgentCreationError`; HTTP status
  classification already exists as `ProviderAdapter::classify_error()`.
- `crates/roko-cli/src/commands/util.rs` already has
  `preflight_provider_for_model()`, but it checks one model and bails on the
  first issue. This task needs an aggregate readiness check for all providers
  referenced by configured models.
- `loader.rs::collect_diagnostics()` and `merge_global_into()` already warn for
  dangling model-provider references in lenient mode. The missing piece is the
  user-configurable strict mode in schema plus a hard error path after the full
  merge/env/secret processing.

### Mechanical implementation order

1. Provider error mapping:
   - Add the helper in `provider/mod.rs`, but make it accept any displayable
     error string source:
     `pub fn map_provider_error(kind: ProviderKind, provider_name: &str,
     api_key_env: Option<&str>, base_url: Option<&str>, err: &(dyn
     std::error::Error + Send + Sync + 'static)) -> String` or provide a
     string-based internal helper. This avoids forcing non-anyhow call sites to
     allocate `anyhow::Error`.
   - Use it when converting `AgentCreationError` in `create_agent_for_model()`
     and in `ModelCallService::execute()` where `agent.run()` failures are
     turned into `CellError`. Preserve the original error text in the chain or
     message.
   - For HTTP streaming paths such as
     `openai_compat_backend.rs::send_turn_streaming()`, map the
     `HttpPostError::http(status, body)` string before sending the
     `StreamChunk::Error` if the provider identity is available there. If not,
     leave a small adapter-level helper rather than guessing provider names.

2. Provider readiness:
   - Implement `ProviderReadinessIssue` and `check_provider_readiness()` in
     `provider/mod.rs` or `provider/pre_flight.rs`, then re-export from
     `provider/mod.rs`.
   - Build the referenced provider set from `config.models.values().map(|m|
     &m.provider)`. Do not check unreferenced providers.
   - For `ProviderKind::ClaudeCli`, use `provider.command.as_deref().unwrap_or("claude")`.
     For `CursorAcp`, use `provider.command` if set; otherwise do not block
     startup because Cursor handles auth/process ownership differently.
   - For API providers, require `api_key_env` to be present and non-empty, then
     require the environment variable value to be non-empty. Do not make network
     requests.
   - In CLI integration, keep existing per-model preflight helpers for command
     paths that already use them, but route them through the new aggregate
     helper where practical so messages stay consistent.

3. CLI discovery command:
   - Add `Available` to `ConfigProviderCmd` in `main.rs`.
   - Add the dispatch arm in `commands/config_cmd.rs::dispatch_config()`.
   - Implement `cmd_provider_available()` near `cmd_provider_list()`.
   - Add parse tests beside the existing
     `cli_parses_config_providers_*` tests in `main.rs`.
   - This command must not call `load_config_unified()` and must work from an
     empty temp directory.

4. Strict post-merge validation:
   - `ValidationConfig` should live in `crates/roko-core/src/config/schema.rs`
     unless a local module already exists for schema sections; add
     `pub validation: ValidationConfig` to `RokoConfig`.
   - Run the strict missing-provider check in `loader.rs` after
     `merge_global_into()`, env overrides, interpolation, and file-secret
     resolution. In strict mode, return `LoadConfigError::Validation` or add an
     appropriate loader error variant with the actionable message.
   - Keep the existing lenient warnings from `collect_diagnostics()`; do not
     double-log if you can reuse one validation function with a severity flag.
   - Empty `config.models` remains valid.

### CLI integration targets for readiness

At minimum, wire startup readiness before these long-running paths:

- `crates/roko-cli/src/unified.rs::cmd_unified_chat()` before
  `chat_inline::run_unified_inline()`.
- `crates/roko-cli/src/commands/plan.rs::cmd_plan()` in the `PlanCmd::Run`
  branch, near the existing `preflight_provider_for_model()` call.
- `crates/roko-cli/src/commands/prd.rs` draft/plan branches, near existing
  preflight calls.
- `crates/roko-cli/src/commands/agent.rs::cmd_agent()` for agent chat.
- `crates/roko-cli/src/commands/do_cmd.rs` before dispatch.

Print each issue as `warning: ...`. Exit nonzero only when every provider
referenced by at least one model is not ready; otherwise allow fallback-capable
configurations to proceed.

### Tests to add

- `provider/mod.rs`: unit tests for `map_provider_error()` covering 401, 429,
  404, connection refused, and ENOENT strings.
- `provider/mod.rs`: readiness tests with temp provider configs: missing API
  env, empty API env, unreferenced provider ignored, nonexistent Claude command.
- `commands/config_cmd.rs` or `main.rs`: CLI parse and output smoke test for
  `roko config providers available` listing all seven labels.
- `loader.rs`: lenient dangling provider logs/diagnostics but succeeds; strict
  mode returns a hard error after global merge.

### Anti-patterns to avoid

- Do not add provider-kind discovery to `roko.toml` parsing. The available list
  is static UX output based on `ProviderKind`.
- Do not make readiness perform HTTP requests; that belongs in
  `config providers test` / health checks.
- Do not make `AgentCreationError::Display` itself read env vars or inspect
  config. Mapping needs provider name/env/base URL from the call site.
- Do not remove the existing lenient diagnostics. Strict mode is opt-in.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
