# Task 073: ACP Startup Resilience

```toml
id = 73
title = "Harden ACP startup: provider readiness warnings, configWarnings in InitializeResult, smoke test"
track = "ide-acp"
wave = "wave-1"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-acp/src/handler.rs",
    "crates/roko-acp/src/config.rs",
    "crates/roko-acp/src/types.rs",
    "crates/roko-core/src/config/loader.rs",
]
exclusive_files = ["crates/roko-acp/src/config.rs"]
estimated_minutes = 90
```

## Context

This task hardens the ACP (Agent Client Protocol) server startup against common failure modes that
cause Zed and other editors to show opaque "server shut down unexpectedly" errors. **Read the code
before writing anything.** Several gaps mentioned in the redesign plan are already implemented.

Sources:
- `tmp/redesign-plan.md` lines 1033-1085 — Phase 0.4 and 0.5
- `tmp/infrastructure-audit.md` section on ACP/Zed integration

## Background

Read these files before writing any code:

1. `crates/roko-acp/src/handler.rs` — The main ACP dispatch loop. Already implements:
   - `.roko/` auto-creation (lines 48-57): `std::fs::create_dir_all(&roko_dir)` before logging setup,
     with a warning printed to stderr on failure (non-fatal).
   - `/tmp` log fallback (lines 59-67): `setup_file_logging().or_else(|e| ...)` tries
     `roko-acp-{pid}.log` in the OS temp dir if the primary path fails.
   - JSON-RPC error on startup failure (lines 27-45): `run_acp_server()` wraps `run_acp_server_inner()`
     and emits `{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"..."}}` to stdout
     before returning `Err`. **Phases 0.4 and 0.5 are fully implemented. Do not touch these.**

2. `crates/roko-acp/src/config.rs` — `AcpConfig` and `load_roko_config()` (line 80):
   Returns `RokoConfig` directly using `.unwrap_or_default()` (line 90). If `roko.toml` exists but
   fails to parse (malformed TOML, unknown field), the error is silently dropped and an empty-default
   config is used. Every subsequent agent dispatch attempt will fail with a cryptic error because no
   providers are configured. The caller at `handler.rs:83` sees no indication of this.

   **Important**: `load_roko_config()` is called from many places (handler.rs lines 83 and 127,
   plus tests in config.rs). Do NOT change its return type — that would cascade to all callers.
   Instead, add a new method.

3. `crates/roko-acp/src/handler.rs` lines 83-94 — `run_acp_server_with_transport()` calls
   `config.load_roko_config()` and logs `info!` with provider/model counts. It does not distinguish
   between "no roko.toml found" (normal, use defaults) and "roko.toml exists but failed to parse"
   (misconfiguration, warn loudly). It also does not check whether any configured provider has
   resolvable credentials.

4. `crates/roko-acp/src/types.rs` — `InitializeResult` (line 171):
   ```rust
   pub struct InitializeResult {
       pub protocol_version: u32,
       pub agent_capabilities: AgentCapabilities,
       pub auth_methods: Vec<serde_json::Value>,
       pub agent_info: Option<AgentInfo>,
       pub config_sources: Vec<String>,
   }
   ```
   No field for surfacing configuration warnings to the editor at `initialize` time. Editors receive
   no hint that something is misconfigured until dispatch fails several seconds later.

5. `crates/roko-acp/src/handler.rs` lines 163-185 — The `initialize` handler builds `InitializeResult`
   and calls `send_success()`. `config_sources: sessions.config_sources.clone()` is already populated
   (line 96: `sessions.config_sources = config.config_sources()`). Adding `config_warnings` follows
   the same pattern.

6. `crates/roko-acp/src/transport.rs` — `send_notification(method, params)` at line 149: sends a
   JSON-RPC notification to the client. This is how to emit a warning before the first request.

## What to Change

### 1. Add `load_roko_config_with_warning()` to `AcpConfig`

In `crates/roko-acp/src/config.rs`, add a new method that wraps `load_roko_config()` but also
returns a diagnostic string when the config file exists but cannot be loaded:

```rust
/// Load the workspace `RokoConfig` and return a warning string if the config
/// file exists but failed to parse or contains unknown fields.
///
/// Returns `(RokoConfig, None)` when no config file exists (normal, use defaults).
/// Returns `(RokoConfig, Some(reason))` when a file exists but loading failed.
/// Returns `(RokoConfig, None)` on success.
pub fn load_roko_config_with_warning(&self) -> (roko_core::config::schema::RokoConfig, Option<String>) {
    let config_file = match &self.config_path {
        Some(path) => Some(path.as_path()),
        None => {
            let candidate = self.workdir.join("roko.toml");
            if candidate.is_file() { None } else { return (self.load_roko_config(), None); }
        }
    };
    // If there's a config file (implicit or explicit), try loading it directly
    // to capture the error, then fall back to the safe loader.
    let warning = if let Some(path) = config_file {
        let opts = roko_core::config::loader::LoadOptions::acp();
        match roko_core::config::loader::load_config_file(path, &opts) {
            Ok(_) => None,
            Err(e) => Some(format!("roko.toml parse error: {e:#}")),
        }
    } else {
        // Implicit workspace file — check if it exists and fails
        let candidate = self.workdir.join("roko.toml");
        if candidate.is_file() {
            let opts = roko_core::config::loader::LoadOptions::acp();
            match roko_core::config::loader::load_config_file(&candidate, &opts) {
                Ok(_) => None,
                Err(e) => Some(format!("roko.toml parse error: {e:#}")),
            }
        } else {
            None
        }
    };
    (self.load_roko_config(), warning)
}
```

Keep the existing `load_roko_config()` unchanged — only add this new method.

### 2. Add `check_provider_readiness()` to `handler.rs`

In `crates/roko-acp/src/handler.rs`, add a free function that checks whether the loaded config has
at least one provider with resolvable credentials:

```rust
/// Returns a human-readable warning string if no configured provider has credentials,
/// or `None` if at least one provider appears ready for dispatch.
fn check_provider_readiness(config: &roko_core::config::schema::RokoConfig) -> Option<String> {
    use roko_core::agent::ProviderKind;
    if config.providers.is_empty() {
        return Some(
            "no providers configured in roko.toml — agent dispatch will fail; \
             set ANTHROPIC_API_KEY or install the claude CLI"
                .to_string(),
        );
    }
    for (_name, provider) in &config.providers {
        if provider.kind == ProviderKind::ClaudeCli {
            // Claude CLI providers don't need an API key env var
            return None;
        }
        if let Some(env_var) = &provider.api_key_env {
            if std::env::var(env_var)
                .ok()
                .filter(|k| !k.is_empty())
                .is_some()
            {
                return None;
            }
        }
    }
    Some(
        "no provider has resolvable credentials — check api_key_env vars in roko.toml \
         or set ANTHROPIC_API_KEY"
            .to_string(),
    )
}
```

### 3. Update `run_acp_server_with_transport()` to use both helpers

In `handler.rs`, replace the call to `config.load_roko_config()` (line 83) with the new
`load_roko_config_with_warning()`:

```rust
// Before (line 83):
let roko_config = config.load_roko_config();

// After:
let (roko_config, config_load_warning) = config.load_roko_config_with_warning();
if let Some(ref warning) = config_load_warning {
    warn!("{warning}");
}
```

Then add the provider readiness check after the info log (line 90):

```rust
// After the existing info! log:
let provider_warning = check_provider_readiness(&roko_config);
if let Some(ref warning) = provider_warning {
    warn!("{warning}");
}

// Collect all startup warnings
let startup_warnings: Vec<String> = [config_load_warning, provider_warning]
    .into_iter()
    .flatten()
    .collect();
```

Pass `startup_warnings` into `SessionManager` so the `initialize` handler can return them.

Add `startup_warnings: Vec<String>` to `SessionManager` (or pass it through to `config_sources`):

```rust
sessions.config_sources = config.config_sources();
sessions.startup_warnings = startup_warnings;
```

If modifying `SessionManager` is too invasive, store the warnings in a local variable and close over
them in the `initialize` handler — the simplest approach is to add a `startup_warnings` field to
`SessionManager` alongside the existing `config_sources` field.

Also update the config-hot-reload path (line 127) to use `load_roko_config_with_warning()`:

```rust
// Before (line 127):
let refreshed = config.load_roko_config();

// After:
let (refreshed, reload_warning) = config.load_roko_config_with_warning();
if let Some(warning) = reload_warning {
    warn!("config reload warning: {warning}");
}
```

### 4. Add `config_warnings` to `InitializeResult`

In `crates/roko-acp/src/types.rs`, add an optional warnings field to `InitializeResult`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: u32,
    #[serde(default)]
    pub agent_capabilities: AgentCapabilities,
    #[serde(default)]
    pub auth_methods: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_info: Option<AgentInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_sources: Vec<String>,
    /// Human-readable warnings about the server configuration, surfaced at init time.
    /// Editors should display these to the user immediately after connection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_warnings: Vec<String>,
}
```

In the `initialize` handler in `handler.rs` (line ~163), populate the new field:

```rust
let result = InitializeResult {
    // ... existing fields ...
    config_sources: sessions.config_sources.clone(),
    config_warnings: sessions.startup_warnings.clone(),
};
```

### 5. Write a startup smoke test

Add a test in `crates/roko-acp/tests/` (or in `crates/roko-acp/src/handler.rs` under
`#[cfg(test)]`) that verifies the new behavior end-to-end using the existing
`run_acp_server_with_transport()` with a mock transport:

```rust
#[tokio::test]
async fn initialize_with_missing_provider_credentials_returns_warning() {
    let dir = tempfile::tempdir().unwrap();
    // Write a roko.toml with a provider that references a missing env var
    std::fs::write(
        dir.path().join("roko.toml"),
        r#"
config_version = 2
schema_version = 2

[providers.test-provider]
kind = "openai_compat"
base_url = "https://api.example.com/v1"
api_key_env = "ROKO_TEST_MISSING_KEY_XYZ_DOES_NOT_EXIST"
"#,
    ).unwrap();

    let config = AcpConfig::new(
        dir.path(),
        "default",
        None,
        dir.path().join(".roko/acp.log"),
    );

    // Feed: initialize request, then EOF (empty body closes stdin)
    let input = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"0.1","workdir":".","profile":"default"}}"#;
    let mut output = Vec::new();
    let mut transport = StdioTransport::from_bytes(input, &mut output);
    run_acp_server_with_transport(config, &mut transport).await.unwrap();

    // Parse the response
    let response_str = String::from_utf8(output).unwrap();
    let response: serde_json::Value = serde_json::from_str(&response_str.lines().next().unwrap()).unwrap();
    let warnings = response["result"]["configWarnings"].as_array().unwrap();
    assert!(!warnings.is_empty(), "expected a provider readiness warning");
}

#[tokio::test]
async fn initialize_with_no_roko_toml_returns_empty_warnings() {
    let dir = tempfile::tempdir().unwrap();
    // No roko.toml in dir — use defaults
    let config = AcpConfig::new(
        dir.path(),
        "default",
        None,
        dir.path().join(".roko/acp.log"),
    );

    let input = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"0.1","workdir":".","profile":"default"}}"#;
    let mut output = Vec::new();
    let mut transport = StdioTransport::from_bytes(input, &mut output);
    run_acp_server_with_transport(config, &mut transport).await.unwrap();

    let response_str = String::from_utf8(output).unwrap();
    let response: serde_json::Value = serde_json::from_str(&response_str.lines().next().unwrap()).unwrap();
    // No roko.toml = no warnings about config problems, but may warn about no providers
    // The key check: the field exists and is a JSON array
    let warnings = &response["result"]["configWarnings"];
    assert!(warnings.is_null() || warnings.is_array());
}
```

Check whether `StdioTransport` has a constructor that accepts byte slices. If not, use
`tokio_test::io::Builder` or `tokio::io::duplex()` to create the mock transport.

## What NOT to Do

- Do NOT change the `.roko/` auto-creation logic (lines 48-57 of `handler.rs`) — already correct.
- Do NOT change the JSON-RPC error-on-fatal-startup-failure (lines 27-45 of `handler.rs`) — done.
- Do NOT change `load_roko_config()` return type — it has dozens of callers across the workspace.
  Only add the new `load_roko_config_with_warning()` method.
- Do NOT make provider readiness a hard failure. It must be a warning. The server must start
  even if no provider is configured — config may be passed at session time or set up later.
- Do NOT add new external dependencies to `roko-acp`. Use what is already available.
- Do NOT change the transport protocol or ACP spec.
- Do NOT add a panic hook — ACP server is not a terminal app; panic output goes to stderr which
  editors monitor separately.

## Wire Target

```bash
# Verify ACP starts correctly with no roko.toml (no crash, no warning flood)
cd /tmp && cargo run -p roko-acp 2>&1 &
# Should block on stdin. Kill after verifying "loaded roko.toml configuration" log line.

# Verify warning appears with RUST_LOG when no provider credentials
RUST_LOG=roko_acp=warn cargo run -p roko-acp --manifest-path /path/to/roko/Cargo.toml 2>&1 | head -5

# Send initialize and verify configWarnings in the response
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"0.1","workdir":"/tmp","profile":"default"}}' \
  | cargo run -p roko-acp 2>/dev/null
# Expected: JSON with "configWarnings": [...] (empty array OK if no provider gap detected)

# Unit tests
cargo test -p roko-acp
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo test -p roko-acp` — all existing tests pass
- [ ] `load_roko_config()` return type is unchanged (no breakage to existing callers)
- [ ] New `load_roko_config_with_warning()` method exists in `AcpConfig`
- [ ] New `check_provider_readiness()` function exists in `handler.rs`
- [ ] `InitializeResult` has `config_warnings: Vec<String>` field with `serde` skip-if-empty
- [ ] `InitializeResult` deserialization is backward-compatible (new field has `#[serde(default)]`)
- [ ] `SessionManager` has `startup_warnings: Vec<String>` field (or equivalent) populated at startup
- [ ] `initialize` handler populates `config_warnings` from `startup_warnings`
- [ ] Smoke test: no `roko.toml` → empty `config_warnings` array in `InitializeResult`
- [ ] Smoke test: provider with missing env var → non-empty `config_warnings` in `InitializeResult`
- [ ] `RUST_LOG=roko_acp=warn` shows warning log line when no provider is ready
- [ ] Config hot-reload path (line 127 of `handler.rs`) also uses `load_roko_config_with_warning()`
- [ ] Existing `.roko/` auto-creation behavior (lines 48-57 of `handler.rs`) is unchanged
- [ ] Existing JSON-RPC error-on-fatal-failure (lines 27-45 of `handler.rs`) is unchanged
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in changed files

## Implementation Detail

### Current Code Facts to Account For

- `crates/roko-acp/src/handler.rs::run_acp_server` already catches startup errors and writes a JSON-RPC error to stdout.
- `run_acp_server_inner` already creates `.roko/` and falls back ACP logs to a temp dir if workspace log creation fails.
- `AcpConfig::load_roko_config` in `crates/roko-acp/src/config.rs` currently returns `RokoConfig` and swallows load errors with defaults. Keep that compatibility, but add a warning-producing helper for ACP initialization.
- `InitializeResult` in `crates/roko-acp/src/types.rs` currently has `config_sources` only. Add `config_warnings` with serde rename/default/skip-empty so old clients are not forced to handle it.

### Mechanical Implementation Steps

1. Add `AcpConfig::load_roko_config_with_warning(&self) -> (RokoConfig, Option<String>)` or an equivalent helper. Reuse `LoadOptions::acp()` and the same explicit/global merge rules as `load_roko_config`; do not create a second config-loading policy.
2. For explicit `--config`, warn when the file cannot be parsed or read, then fall back to the existing default load behavior. For implicit `workdir/roko.toml`, warn only if the file exists and fails to load. Missing project config is not a client-visible warning.
3. If an explicit or implicit global config file is present and fails to load, include a warning for that source as well. Preserve the existing behavior of continuing startup with defaults/project config where possible.
4. Add provider readiness checking after config load. Use effective provider data if available, and treat CLI providers such as `claude_cli` as ready without an API key when their command can be resolved or when the existing provider/auth helper says they are usable.
5. Store startup warnings on `SessionManager` (for example `startup_warnings: Vec<String>`) and set them during server startup. On hot reload, recompute the same warnings and replace the stored list so later `initialize` calls report current config state.
6. In the `initialize` handler, copy `sessions.startup_warnings` into `InitializeResult.config_warnings`.

### Tests to Add or Update

- Extend the existing ACP protocol harness in `crates/roko-acp/tests/protocol_conformance.rs`; it already uses `tokio::io::duplex` and `StdioTransport::from_io`, so do not invent a separate byte transport.
- `initialize_with_no_roko_toml_returns_empty_warnings`: use a temp workdir and isolate global config by passing an explicit missing `global_config_path` or equivalent so the developer machine's home config cannot affect the test.
- `initialize_with_malformed_roko_toml_returns_config_warning`: write invalid TOML in the temp workdir, initialize successfully, assert `result.configWarnings` is non-empty and mentions the parse/read failure.
- `initialize_with_unavailable_provider_credentials_returns_warning`: write a valid config with an API-key provider whose env var is intentionally absent, initialize successfully, and assert a provider readiness warning is returned.
- If testing `claude_cli`, make the command path deterministic with a temp executable or skip that provider-readiness branch in unit tests; do not depend on the developer machine having Claude installed.

### Expected Observable Behavior

- Empty directory startup succeeds and `initialize` either omits `configWarnings` or returns an empty array.
- Malformed project/global config no longer kills ACP startup; clients receive a warning and can still create sessions against defaults where possible.
- Missing credentials for API-key providers are surfaced as warnings, while CLI-backed providers do not produce false API-key warnings.

### Additional Verification Commands

- `cargo test -p roko-acp --test protocol_conformance initialize`
- `cargo test -p roko-acp`
- Manual: `mkdir -p /tmp/roko-acp-empty && (cd /tmp/roko-acp-empty && cargo run -p roko-cli -- acp)` then send an `initialize` request and confirm success.

### Additional What NOT To Do

- Do not make config warnings fatal.
- Do not surface "missing roko.toml" as a client-visible warning; that is already logged and is an allowed startup mode.
- Do not check only `api_key_env` strings; use the provider's resolved credential behavior so direct keys, env vars, and CLI providers are handled consistently.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
