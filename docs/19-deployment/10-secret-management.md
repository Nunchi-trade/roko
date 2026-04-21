# Secret Management

> Roko handles sensitive credentials through a layered resolution strategy that works across
> laptop-local, single-server, container, clustered, and edge profiles. The goal is the same
> everywhere: secrets should be easy to inject, hard to leak, and portable across shapes.
> See also `../../tmp/refinements/24-deployment-ux.md`.

> **Implementation**: Specified

---

## Shape-Aware Secret Policy

Secret handling follows the deployment profile. The same binary uses different defaults based
on the selected shape:

| Shape | Default secret source | Notes |
|---|---|---|
| laptop-local | OS keychain | Best interactive experience; secrets stay off disk |
| single-server | OS keychain or host secret store | Shared machine, scoped by user or role |
| container | Env vars or `_FILE` mounts | Fits Docker, Compose, and orchestrators |
| clustered | External secret store | Vault, cloud secret manager, or sealed secrets |
| edge | Provider-native secret injection | Minimal surface; avoid local persistence |

Profiles can override these defaults, but they should not change the resolution semantics.

---

## Secret Resolution Order

Higher-priority sources override lower ones:

```text
1. CLI flags         roko run --api-key sk-ant-...
       ↓
2. Environment       ANTHROPIC_API_KEY=sk-ant-...
       ↓
3. Config files      roko.toml / ~/.config/roko/config.toml
       ↓
4. OS keychain       macOS Keychain / Linux Secret Service / Windows Credential Manager
       ↓
5. Secret store      Vault / AWS Secrets Manager / 1Password CLI / K8s Secret / Swarm secret
       ↓
6. Compiled default  fail with an actionable error message
```

This order keeps fast local overrides at the top, then falls back to durable profile-specific
stores for shared and production deployments.

### Why This Order

- CLI flags are for one-off debugging and scripted overrides.
- Environment variables are the portable default for containers and CI.
- Config files hold declarative references, including `${VAR}` interpolation.
- OS keychains are the default for interactive laptop-local use.
- Secret stores are the default for single-server and clustered production deployments.

---

## Environment Variable Conventions

Each product reads a consistent prefix:

| Product | Prefix | Key Examples |
|---|---|---|
| roko-cli | `ROKO_` | `ROKO_MODEL`, `ROKO_MAX_AGENTS` |
| roko-serve | `ROKO_SERVE_` | `ROKO_SERVE_PORT`, `ROKO_SERVE_BIND` |
| mirage-rs | `MIRAGE_` | `MIRAGE_RPC_URL`, `MIRAGE_PORT` |

Cross-product environment variables:

| Variable | Used by | Purpose |
|---|---|---|
| `ANTHROPIC_API_KEY` | roko-cli, roko-serve | Anthropic provider key |
| `OPENAI_API_KEY` | roko-cli, roko-serve | OpenAI provider key |
| `OPENROUTER_API_KEY` | roko-cli, roko-serve | OpenRouter multi-model key |
| `RUST_LOG` | all | Log level filter |

---

## .env File Loading

`dotenvy` can load `.env` files from the working directory for laptop-local and single-server
projects. This is a convenience layer, not the primary secret store.

```rust
dotenvy::dotenv().ok();
```

Rules:

1. Look for `.env` in the current working directory.
2. Load it if present.
3. Do not overwrite existing environment variables.
4. Continue silently if the file is missing.

---

### .env File Format

```bash
# .env (gitignored)
ANTHROPIC_API_KEY=sk-ant-abc123...
OPENAI_API_KEY=sk-def456...
ROKO_WEBHOOK_SECRET=whsec_abc123
MIRAGE_RPC_URL=https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY
```

### .env in .gitignore

Keep `.env` files out of version control:

```gitignore
.env
.env.local
.env.*.local
```

---

## ${VAR} Interpolation in roko.toml

Config files support `${VAR}` syntax so profile configuration can stay declarative:

```toml
[agent.providers.anthropic]
api_key = "${ANTHROPIC_API_KEY}"

[agent.providers.openai]
api_key = "${OPENAI_API_KEY}"

[subscription.webhook]
secret = "${ROKO_WEBHOOK_SECRET}"
```

### Interpolation Rules

1. `${VAR}` resolves from the environment.
2. `${VAR:-default}` falls back to a default.
3. `$$` escapes a literal dollar sign.
4. An unresolved variable should log a warning and remain visible in the config path.

---

## OS Keychain Integration

For interactive use, the OS keychain is the preferred secret store.

| Platform | Backend |
|---|---|
| macOS | Keychain |
| Linux | Secret Service |
| Windows | Credential Manager |

Use it for laptop-local and single-server profiles when a user is at the keyboard.

### Interactive Key Setup

`roko init` should be able to prompt for a key and offer to store it in the keychain instead of
writing the value into a config file.

### Key Rotation

Rotation should update the backing store without forcing a restart when the deployment profile
supports live secret refresh.

---

## Scoped Secrets

Secrets can be scoped per repository, per tenant, or per role.

```toml
[credentials]
anthropic_api_key = "${ANTHROPIC_API_KEY_TEAM}"
```

In a shared deployment, the same `roko.toml` can point to different sources for different
projects or teams without changing the binary.

### Secret CLI

The operator-facing CLI should make secret lifecycle actions explicit:

```bash
roko secret set anthropic.api_key
roko secret get anthropic.api_key
roko secret list
roko secret rotate anthropic.api_key
```

The command surface can route to the active profile's backing store, whether that is a
keychain, secret manager, container mount, or provider-native secret system.

### Role-Based Secrets

Some roles need additional keys. The secret layer should support role-scoped injection so a
reviewer, implementer, or operator only receives the credentials it needs.

---

## Docker Secret Patterns

Containers should prefer environment variables or `_FILE` indirection.

```yaml
services:
  roko:
    image: ghcr.io/nunchi/roko-cli:latest
    environment:
      ANTHROPIC_API_KEY: ${ANTHROPIC_API_KEY:?Set ANTHROPIC_API_KEY}
      OPENAI_API_KEY: ${OPENAI_API_KEY:-}
```

For Docker secrets and similar orchestrator mounts, support the `_FILE` suffix:

```rust
fn resolve_env_or_file(var_name: &str) -> Option<String> {
    if let Ok(val) = std::env::var(var_name) {
        return Some(val);
    }

    let file_var = format!("{var_name}_FILE");
    if let Ok(path) = std::env::var(&file_var) {
        if let Ok(val) = std::fs::read_to_string(&path) {
            return Some(val.trim().to_string());
        }
    }

    None
}
```

This keeps secrets out of process listings and image layers.

---

## Fly.io Secret Management

Fly.io and similar platforms inject secrets as environment variables at runtime.

```bash
fly secrets set ANTHROPIC_API_KEY=sk-ant-... --app roko-cli
fly secrets list --app roko-cli
fly secrets unset ANTHROPIC_API_KEY --app roko-cli
```

Secrets should never land in `fly.toml`, image layers, or logs.

---

## Secret Safety Rules

1. Never log secret values.
2. Never write secret values into Engram bodies or long-lived state.
3. Never persist resolved secret values into `.roko/` archives.
4. Keep `.env` files gitignored.
5. Prefer the OS keychain for laptop-local use.
6. Prefer `_FILE` and secret stores for container and clustered deployments.

### Audit Trail

The daemon may log which source resolved a secret without exposing the value:

```text
[INFO] Secret resolved: ANTHROPIC_API_KEY source=keychain
[INFO] Secret resolved: OPENAI_API_KEY source=env
[WARN] Secret not found: OPENROUTER_API_KEY (optional)
```

---

## The `roko doctor` Secret Check

`roko doctor` should validate secret availability across the active profile:

```text
Credentials:
  ANTHROPIC_API_KEY: set [source: keychain]
  OPENAI_API_KEY: set [source: .env]
  OPENROUTER_API_KEY: not set (optional)
```

The check should resolve sources in order, mask values, and show the source that won.

---

## Current Status

The target design is profile-aware and shape-aware. The important point for the deployment
chapter is not the exact backend implementation, but that the same secret model works for
laptop-local, single-server, container, clustered, and edge deployments without special-case
code.
