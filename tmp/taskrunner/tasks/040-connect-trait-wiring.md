# Task 040: Redesign Connect Trait + Wire ProviderConnection into Provider Health Check

```toml
id = 40
title = "Redesign Connect trait to async with ConnectionHealth + implement ProviderConnection + wire into roko config providers health"
track = "v2-core-abstractions"
wave = "wave-2"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-core/src/traits.rs",
    "crates/roko-core/src/lib.rs",
    "crates/roko-cli/src/provider_connection.rs",
    "crates/roko-cli/src/lib.rs",
    "crates/roko-cli/src/commands/config_cmd.rs",
]
exclusive_files = []
estimated_minutes = 180
```

## Context

The Connect trait exists in `roko-core/src/traits.rs` but it's a minimal stub:

```rust
pub trait Connect: crate::cell::Cell {
    fn connect(&self) -> Result<()>;
    fn health(&self) -> bool;
    fn disconnect(&self) -> Result<()>;
}
```

This has four problems:
1. It's synchronous — connections to APIs, databases, etc. require async I/O
2. `health()` returns a bare bool — no latency, error details, or status information
3. It has no `request()` method for actual I/O through the connection
4. It has zero implementations and zero callers

The v2 spec gives Connect lifecycle semantics (open/close) and structured health reporting.
This task redesigns Connect, implements `ProviderConnection` for LLM provider health checks,
and wires it into the existing `roko config providers health` command.

Checklist items: P1-11, P1-12.

## Background

Read these files before starting:

1. `crates/roko-core/src/traits.rs` — current Connect stub (lines 405-415)
2. `crates/roko-cli/src/commands/config_cmd.rs` — find `cmd_provider_health()` function
3. `crates/roko-core/src/connector.rs` — existing ConnectorHealth/ConnectorRegistry types (do NOT duplicate)
4. `tmp/v2-refactoring/06-NEW-PROTOCOLS.md` — the v2 spec for Connect

**Critical**: Check `connector.rs` for existing health/connection types:
```bash
grep -rn 'ConnectorHealth\|ConnectorStatus\|ConnectorKind' crates/roko-core/src/connector.rs | head -20
```
There may already be `ConnectorHealth` or similar types. If so, use them or extend them
rather than creating duplicates.

Also check the existing provider health infrastructure:
```bash
grep -rn 'ProviderHealth\|CircuitState' crates/roko-learn/src/ --include='*.rs' | head -20
```
There's a `ProviderHealth` struct in roko-learn with circuit breaker state. The Connect trait
should complement (not replace) this.

Current source notes that supersede the illustrative snippets below:

- The active command path is `crates/roko-cli/src/main.rs`
  `ConfigProviderCmd::Health` -> `crates/roko-cli/src/commands/config_cmd.rs`
  `dispatch_config` -> `cmd_provider_health`. `crates/roko-cli/src/config_cmd.rs`
  is not the primary dispatch path.
- `cmd_provider_health` currently loads config via
  `roko_core::config::loader::load_config_unified(workdir)`, prints credential
  coverage, then prints persisted circuit/latency rows from
  `provider-health.json` and `latency-stats.json`. Preserve that output and add
  live Connect health alongside it.
- `cmd_provider_list` already has bounded live probe helpers:
  `inspect_provider`, `inspect_http_provider`, and `probe_base_url`. Reuse or
  mirror that behavior instead of creating a divergent probing policy.
- `RokoConfig::is_provider_available` considers process env and `[agent.env]`.
  `ProviderConfig::resolve_api_key()` only checks process env, so do not use it
  alone to decide credential availability.
- `RokoError::Other` does not exist in the current error enum. Use an existing
  variant such as `RokoError::Invalid(...)` or `RokoError::Transport(...)`.
- Do not add a nested Tokio runtime. `dispatch_config` is already async, so make
  `cmd_provider_health` async and await connection health directly.

## What to Change

### 1. Add ConnectionHealth struct to roko-core

First check if a suitable struct already exists. If `ConnectorHealth` in `connector.rs` works,
use it. Otherwise, add:

```rust
/// Health status of a connection managed by the Connect trait.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionHealth {
    /// Whether the connection is currently established.
    pub connected: bool,
    /// Round-trip latency in milliseconds (if measured).
    pub latency_ms: Option<f64>,
    /// Human-readable error message (if unhealthy).
    pub error: Option<String>,
    /// Name of the connection target (e.g., provider name, endpoint URL).
    pub target: String,
}

impl ConnectionHealth {
    pub fn healthy(target: impl Into<String>, latency_ms: f64) -> Self {
        Self { connected: true, latency_ms: Some(latency_ms), error: None, target: target.into() }
    }
    pub fn unhealthy(target: impl Into<String>, error: impl Into<String>) -> Self {
        Self { connected: false, latency_ms: None, error: Some(error.into()), target: target.into() }
    }
}
```

### 2. Redesign Connect trait in `crates/roko-core/src/traits.rs`

Replace the existing Connect trait:

```rust
/// Manages a connection to an external system with lifecycle semantics.
///
/// Connect provides open/close/health lifecycle for durable connections
/// (LLM APIs, databases, WebSocket streams, MCP servers).
///
/// # Lifecycle
///
/// ```text
/// open() -> [healthy] -> request() -> ... -> close()
///                     -> health()
/// ```
#[async_trait]
pub trait Connect: Cell {
    /// Open/establish the connection.
    async fn open(&self, ctx: &Context) -> Result<()>;

    /// Close/tear down the connection.
    async fn close(&self, ctx: &Context) -> Result<()>;

    /// Check connection health. Must be cheap (no heavy I/O).
    async fn health(&self, ctx: &Context) -> Result<ConnectionHealth>;

    /// Send a request through the connection and receive a response.
    async fn request(&self, input: Engram, ctx: &Context) -> Result<Engram>;
}
```

### 3. Implement ProviderConnection

Create `crates/roko-cli/src/provider_connection.rs` since it needs reqwest and provider
config access, then export it from `crates/roko-cli/src/lib.rs` so later doctor/tests can
compile-check it:

```rust
use roko_core::{Cell, CellVersion, Context, Engram, ConnectionHealth, traits::Connect};
use async_trait::async_trait;

/// Connect implementation that checks LLM provider health by pinging their API endpoint.
pub struct ProviderConnection {
    provider_name: String,
    endpoint: String,
    api_key_env: Option<String>,
    credential_available: bool,
}

impl ProviderConnection {
    pub fn new(
        provider_name: String,
        endpoint: String,
        api_key_env: Option<String>,
        credential_available: bool,
    ) -> Self {
        Self { provider_name, endpoint, api_key_env, credential_available }
    }

    /// Create ProviderConnections from the config's provider list.
    pub fn from_config(config: &RokoConfig) -> Vec<Self> {
        // Iterate config.providers, create one ProviderConnection per entry
        // Use the provider's base_url as endpoint
    }
}

impl Cell for ProviderConnection {
    fn cell_id(&self) -> &str { &self.provider_name }
    fn cell_name(&self) -> &str { &self.provider_name }
    fn protocols(&self) -> &[&str] { &["Connect"] }
}

#[async_trait]
impl Connect for ProviderConnection {
    async fn open(&self, _ctx: &Context) -> roko_core::error::Result<()> {
        Ok(()) // HTTP is stateless; open is a no-op
    }

    async fn close(&self, _ctx: &Context) -> roko_core::error::Result<()> {
        Ok(())
    }

    async fn health(&self, _ctx: &Context) -> roko_core::error::Result<ConnectionHealth> {
        // Computed from RokoConfig::is_provider_available when constructed.
        if !self.credential_available {
            return Ok(ConnectionHealth::unhealthy(
                &self.provider_name,
                format!("API key env {} not set", self.api_key_env.as_deref().unwrap_or("?")),
            ));
        }

        // Ping the endpoint (lightweight HEAD request)
        let start = std::time::Instant::now();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| roko_core::error::RokoError::Transport(e.to_string()))?;

        match client.head(&self.endpoint).send().await {
            Ok(resp) => {
                let latency = start.elapsed().as_secs_f64() * 1000.0;
                if resp.status().is_success() || resp.status().as_u16() == 401 || resp.status().as_u16() == 403 {
                    // 401/403 means the endpoint is reachable (auth issue is separate)
                    Ok(ConnectionHealth::healthy(&self.provider_name, latency))
                } else {
                    Ok(ConnectionHealth::unhealthy(
                        &self.provider_name,
                        format!("HTTP {}", resp.status()),
                    ))
                }
            }
            Err(e) => Ok(ConnectionHealth::unhealthy(&self.provider_name, e.to_string())),
        }
    }

    async fn request(&self, _input: Engram, _ctx: &Context) -> roko_core::error::Result<Engram> {
        Err(roko_core::error::RokoError::Invalid(
            "ProviderConnection::request() not implemented — use agent dispatch".into(),
        ))
    }
}
```

### 4. Wire into `roko config providers health`

In `crates/roko-cli/src/commands/config_cmd.rs`, find `cmd_provider_health()`. Currently it
reads from the persisted `provider-health.json` file. Add a Connect-based live check:

```rust
use crate::provider_connection::ProviderConnection;
use roko_core::traits::Connect;

pub(crate) async fn cmd_provider_health(workdir: &Path) -> Result<()> {
    let config = load_config(workdir)?;

    // Create ProviderConnections from config
    let connections = ProviderConnection::from_config(&config);

    // Check health via Connect trait
    let ctx = Context::now();
    for conn in &connections {
        let health = conn.health(&ctx).await?;
        // Display health status (integrate with existing display logic)
    }

    // Also show persisted circuit-breaker data (existing logic)
    // ...
}
```

**Important**: Don't remove the existing persisted health display. Add the live Connect-based
check as additional information (e.g., "Live: connected, 45ms" alongside the circuit breaker
state). The existing provider health infrastructure records historical success/failure rates;
the Connect trait provides live reachability.

Also update the dispatch arm from `cmd_provider_health(&wd)?` to
`cmd_provider_health(&wd).await?`.

### 5. Export ConnectionHealth from lib.rs

Add to `crates/roko-core/src/lib.rs` exports:
```rust
pub use traits::ConnectionHealth; // or wherever it's defined
```

## Mechanical Implementation Plan

1. Add `ConnectionHealth` and update `Connect` in `roko-core/src/traits.rs`.
2. Export `ConnectionHealth` from `roko-core/src/lib.rs`.
3. Add `ProviderConnection` in `roko-cli/src/provider_connection.rs`.
4. Implement `Cell` for `ProviderConnection` with `protocols() == &["Connect"]`.
5. Implement `Connect`:
   - `open` and `close` are no-ops for stateless HTTP providers.
   - `health` returns deterministic unhealthy status when required credentials
     are unavailable, otherwise performs a bounded `HEAD`.
   - `request` returns a clear `RokoError::Invalid` because provider request
     dispatch is not part of this task.
6. Make `cmd_provider_health` async and update its dispatch arm to await it.
7. Print live connection health in a separate section/table or separate JSON
   field; avoid changing `ProviderHealthRow` unless all existing row literals
   and formatting tests are updated.
8. Add tests for no-network credential failure and CLI health output.

Expected runtime path:

`roko config providers health` -> `main.rs` `ConfigProviderCmd::Health` ->
`commands/config_cmd.rs::dispatch_config` -> `cmd_provider_health(...).await` ->
`ProviderConnection::from_config` -> `Connect::health(&connection, &ctx).await`.

## Provider Details

- Use configured `base_url` when present.
- Suggested default targets: Anthropic `https://api.anthropic.com`, OpenAI-compatible
  `https://api.openai.com/v1`, Gemini `https://generativelanguage.googleapis.com`,
  Perplexity `https://api.perplexity.ai`, Cerebras `https://api.cerebras.ai/v1`.
- Treat HTTP success and `401`/`403` as reachable; auth failures prove the endpoint
  responded.
- For CLI/local providers such as Claude CLI, check command availability rather
  than making a network request.
- Keep timeouts short and derived from provider timeout config where practical.

## What NOT to Do

- Do NOT implement Connect for every provider. Start with one generic ProviderConnection
  that works for any HTTP-based provider.
- Do NOT replace the existing circuit breaker / ProviderHealth system in roko-learn. Connect
  provides live checks; the circuit breaker provides historical tracking. They complement.
- Do NOT duplicate `ConnectorHealth` if it already exists in connector.rs. Use or extend it.
- Do NOT make the live health check blocking on startup. It should only run when
  `roko config providers health` is explicitly called.
- Do NOT remove existing provider health output. Add to it.
- Do NOT create a nested Tokio runtime or call `block_on` from the async command
  dispatch path.
- Do NOT decide credentials solely with `ProviderConfig::resolve_api_key()`; use
  the unified config availability semantics.
- Do NOT duplicate `ConnectorHealth` under a confusing name. If the existing
  connector type is not sufficient, add a distinct `ConnectionHealth` for this
  protocol.

## Wire Target

```bash
cargo run -p roko-cli -- config providers health
# Should show live connection status (via Connect::health()) alongside persisted circuit data
# Example output:
#   anthropic: Live: connected (45ms) | Circuit: CLOSED | Calls: 150 | Err: 2%
#   openai:    Live: connected (62ms) | Circuit: CLOSED | Calls: 80  | Err: 0%
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo run -p roko-cli -- config providers health` — shows live connection status
- [ ] `grep -rn 'Connect::health\|conn\.health' crates/roko-cli/ --include='*.rs' | grep -v target/ | grep -v test` — shows callsite in config_cmd.rs
- [ ] `grep -rn 'ConnectionHealth' crates/roko-core/src/ --include='*.rs' | grep -v target/` — struct exists and is exported
- [ ] `grep -rn 'async fn open\|async fn close\|async fn health\|async fn request' crates/roko-core/src/traits.rs` — 4 async methods on Connect
- [ ] Existing `roko config providers health` output still works (no regression)
- [ ] The live check has a timeout (does not hang if a provider is unreachable)
- [ ] `cargo test -p roko-cli provider_connection` — no-network connector tests pass

## Status Log

| Time | Agent | Action |
|------|-------|--------|
