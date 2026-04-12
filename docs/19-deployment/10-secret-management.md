# Secret Management

> Roko handles sensitive credentials (LLM provider API keys, webhook secrets, chain private
> keys) through a layered resolution strategy: CLI flags, environment variables, .env files
> (via dotenvy), OS keychain, and config files. This document covers the resolution order,
> scoped secrets per repository, secure storage patterns, Docker and Fly.io secret injection,
> and the `${VAR}` interpolation system in roko.toml.

---

## Secret Resolution Order

All Roko binaries (roko-cli, roko-serve, mirage-rs) resolve secrets using the same tiered
strategy. Higher-priority sources override lower ones:

```
1. CLI flag         roko run --api-key sk-ant-...           (highest priority)
       ↓
2. Environment      ANTHROPIC_API_KEY=sk-ant-...
       ↓
3. .env file        .env in the repo root (loaded by dotenvy)
       ↓
4. OS keychain      macOS Keychain / Linux Secret Service / Windows Credential Manager
       ↓
5. Config file      ~/.config/roko/config.toml → [credentials]
       ↓
6. Compiled default (none — fail with actionable error message)  (lowest priority)
```

### Why This Order

- **CLI flags** are for one-off overrides and debugging: `roko run --api-key sk-ant-test-...`
- **Environment variables** are for CI, Docker, and shell-level configuration
- **.env files** are for per-project secrets that the team shares (gitignored)
- **OS keychain** is the secure default for interactive use — keys never touch the filesystem
- **Config files** are the fallback for environments without keychain support

---

## Environment Variable Conventions

Each Roko product uses a consistent prefix:

| Product | Prefix | Key Examples |
|---|---|---|
| roko-cli | `ROKO_` | `ROKO_MODEL`, `ROKO_MAX_AGENTS` |
| roko-serve | `ROKO_SERVE_` | `ROKO_SERVE_PORT`, `ROKO_SERVE_BIND` |
| mirage-rs | `MIRAGE_` | `MIRAGE_RPC_URL`, `MIRAGE_PORT` |

Cross-product environment variables (read by multiple products):

| Variable | Used by | Purpose |
|---|---|---|
| `ANTHROPIC_API_KEY` | roko-cli, roko-serve | Anthropic Claude provider key |
| `OPENAI_API_KEY` | roko-cli, roko-serve | OpenAI GPT provider key |
| `OPENROUTER_API_KEY` | roko-cli, roko-serve | OpenRouter multi-model key |
| `RUST_LOG` | all | Log level filter (e.g., `info`, `roko=debug`) |

---

## .env File Loading

Roko uses the `dotenvy` crate to load `.env` files from the repository root. The loading
behavior:

1. Look for `.env` in the current working directory
2. If found, load its contents as environment variables
3. **Existing environment variables are NOT overwritten** — .env values only fill in gaps
4. If `.env` does not exist, silently continue (not an error)

```rust
// Early in main(), before config loading
dotenvy::dotenv().ok(); // Load .env if it exists, ignore if not
```

### .env File Format

```bash
# .env (gitignored — never commit this file)
#
# API keys for LLM providers
ANTHROPIC_API_KEY=sk-ant-abc123...
OPENAI_API_KEY=sk-def456...

# Webhook secrets
ROKO_WEBHOOK_SECRET_A=whsec_abc123
ROKO_WEBHOOK_SECRET_B=whsec_def456

# mirage-rs RPC endpoint
MIRAGE_RPC_URL=https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY
```

### .env in .gitignore

The `.env` file must be in `.gitignore` to prevent accidental commits of secrets:

```
# .gitignore
.env
.env.local
.env.*.local
```

The `roko init` command automatically adds `.env` to `.gitignore` if it is not already listed.

---

## ${VAR} Interpolation in roko.toml

Config files (`roko.toml`, `~/.config/roko/config.toml`) support `${VAR}` syntax for
referencing environment variables:

```toml
# roko.toml
[agent]
model = "claude-sonnet-4-6"

[agent.providers.anthropic]
api_key = "${ANTHROPIC_API_KEY}"

[agent.providers.openai]
api_key = "${OPENAI_API_KEY}"

[subscriptions.webhook]
secret = "${ROKO_WEBHOOK_SECRET}"
```

### Interpolation Rules

1. `${VAR}` — resolved from environment at config load time
2. `${VAR:-default}` — resolved from environment, falls back to `default` if not set
3. Literal `$` is escaped as `$$` (e.g., `$$HOME` produces the literal string `$HOME`)
4. Unresolved `${VAR}` (variable not set, no default) produces a warning log and keeps the
   literal string — this makes misconfiguration visible rather than silently failing

```rust
/// Resolve ${VAR} and ${VAR:-default} in a string.
fn interpolate_env(input: &str) -> String {
    let re = regex::Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-([^}]*))?\}").unwrap();

    re.replace_all(input, |caps: &regex::Captures| {
        let var_name = &caps[1];
        let default = caps.get(2).map(|m| m.as_str());

        match std::env::var(var_name) {
            Ok(val) => val,
            Err(_) => match default {
                Some(d) => d.to_string(),
                None => {
                    tracing::warn!(
                        var = var_name,
                        "Unresolved environment variable in config"
                    );
                    caps[0].to_string() // Keep literal ${VAR}
                }
            }
        }
    }).to_string()
}
```

---

## OS Keychain Integration

For interactive use (developer laptops), the `keyring` crate provides cross-platform access to
the OS credential store:

| Platform | Backend |
|---|---|
| macOS | Keychain (via Security.framework) |
| Linux | Secret Service (GNOME Keyring, KDE Wallet) |
| Windows | Credential Manager |

### Storing Keys

```rust
use keyring::Entry;

/// Save an API key to the OS keychain.
fn save_to_keychain(service: &str, key_name: &str, value: &str) -> Result<()> {
    let entry = Entry::new(service, key_name)?;
    entry.set_password(value)?;
    Ok(())
}

// Usage during `roko init --global`:
save_to_keychain("roko", "ANTHROPIC_API_KEY", "sk-ant-...")?;
```

### Retrieving Keys

```rust
/// Get an API key, checking env → keychain → config in order.
fn resolve_api_key(key_name: &str) -> Option<String> {
    // 1. Check environment variable
    if let Ok(val) = std::env::var(key_name) {
        return Some(val);
    }

    // 2. Check OS keychain
    if let Ok(entry) = Entry::new("roko", key_name) {
        if let Ok(val) = entry.get_password() {
            return Some(val);
        }
    }

    // 3. Check config file
    // (loaded separately during config merge)

    None
}
```

### Interactive Key Setup

The `roko init --global` command prompts for API keys and offers keychain storage:

```
$ roko init --global

Roko Global Setup

ANTHROPIC_API_KEY not found in environment or keychain.
Enter your Anthropic API key: sk-ant-...
Save to OS keychain? [Y/n] y
  Saved to macOS Keychain (service: roko, account: ANTHROPIC_API_KEY)

OPENAI_API_KEY not found (optional, for fallback routing).
Enter your OpenAI API key (or press Enter to skip):

Created: ~/.config/roko/config.toml
  API keys stored in OS keychain (not in config file).
```

### Key Rotation

To rotate a key:

```bash
# Update in keychain
roko config set-secret ANTHROPIC_API_KEY sk-ant-new-key-...
# This updates the keychain entry; no config file changes needed

# Verify
roko doctor
#   ANTHROPIC_API_KEY: set (sk-ant-...ew-k) ← shows last 4 chars
```

---

## Scoped Secrets

Different repositories may need different API keys (e.g., one project uses a team Anthropic
key, another uses a personal key). Scoped secrets are configured per-repo:

```toml
# .roko/config.toml (in repo root)
[credentials]
# Override the global Anthropic key for this repo
anthropic_api_key = "${ANTHROPIC_API_KEY_TEAM}"
```

The per-repo `.env` file can also scope secrets:

```bash
# project-a/.env
ANTHROPIC_API_KEY=sk-ant-team-key-...

# project-b/.env
ANTHROPIC_API_KEY=sk-ant-personal-key-...
```

Since `dotenvy` loads from the working directory, each project's `.env` takes effect when
roko is run from that project's root.

---

## Docker Secret Patterns

In Docker containers, secrets come from environment variables. The config file is optional.

### Docker Compose

```yaml
services:
  roko:
    image: ghcr.io/nunchi/roko-cli:latest
    environment:
      ANTHROPIC_API_KEY: ${ANTHROPIC_API_KEY:?Set ANTHROPIC_API_KEY in .env}
      OPENAI_API_KEY: ${OPENAI_API_KEY:-}
```

The `${VAR:?message}` syntax in docker-compose ensures the container fails fast if a required
secret is not set, rather than starting and failing on the first API call.

### Docker Secrets (Swarm)

For Docker Swarm deployments, use Docker secrets instead of environment variables:

```yaml
services:
  roko:
    image: ghcr.io/nunchi/roko-cli:latest
    secrets:
      - anthropic_api_key
    environment:
      ANTHROPIC_API_KEY_FILE: /run/secrets/anthropic_api_key

secrets:
  anthropic_api_key:
    external: true
```

Roko supports the `_FILE` suffix convention: if `ANTHROPIC_API_KEY_FILE` is set, the value is
read from the file at that path instead of from the `ANTHROPIC_API_KEY` environment variable.
This is the standard Docker pattern for secret injection.

```rust
/// Resolve an environment variable, supporting the _FILE suffix.
fn resolve_env_or_file(var_name: &str) -> Option<String> {
    // Check direct env var first
    if let Ok(val) = std::env::var(var_name) {
        return Some(val);
    }

    // Check _FILE variant
    let file_var = format!("{var_name}_FILE");
    if let Ok(path) = std::env::var(&file_var) {
        if let Ok(val) = std::fs::read_to_string(&path) {
            return Some(val.trim().to_string());
        }
    }

    None
}
```

---

## Fly.io Secret Management

On Fly.io, secrets are set via the `fly secrets` CLI and injected as environment variables into
the machine:

```bash
# Set secrets (encrypted at rest on Fly's infrastructure)
fly secrets set ANTHROPIC_API_KEY=sk-ant-... --app roko-cli

# List secrets (shows names only, not values)
fly secrets list --app roko-cli
# NAME                  DIGEST                 CREATED AT
# ANTHROPIC_API_KEY     abc123def456           2026-04-10T12:00:00Z

# Remove a secret
fly secrets unset ANTHROPIC_API_KEY --app roko-cli
```

Secrets never appear in `fly.toml`, Docker images, or logs. They exist only in Fly's encrypted
secret store and are injected as environment variables when the machine starts.

The deploy script (`deploy/scripts/fly-secrets.sh`) automates secret setup for all services.
See `06-cloud-fly-io.md` for the full Fly.io secret management flow.

---

## Secret Safety Rules

1. **Never log secrets.** All log formatters mask values that match API key patterns
   (`sk-ant-*`, `sk-*`, keys longer than 20 characters).
2. **Never include secrets in Engram bodies.** The `Provenance` field tracks which API key
   was used (by hash, not value) for audit purposes.
3. **Never write secrets to `.roko/` state files.** Config files may reference `${VAR}` but
   the resolved values are held in memory only.
4. **Always gitignore `.env` files.** The `roko init` command enforces this.
5. **Prefer OS keychain over config files** for interactive use. Config file storage is the
   last resort for environments without keychain support.
6. **Use `_FILE` suffix for container secrets.** This prevents secrets from appearing in
   `docker inspect` output or process environment listings.

### Audit Trail

The daemon logs when secrets are resolved (but not their values):

```
[INFO] Secret resolved: ANTHROPIC_API_KEY source=keychain
[INFO] Secret resolved: OPENAI_API_KEY source=env
[WARN] Secret not found: OPENROUTER_API_KEY (optional, skipped)
```

This makes it clear which secrets are active and where they came from, without exposing values.

---

## The `roko doctor` Secret Check

The `roko doctor` command validates secret availability:

```bash
$ roko doctor

Credentials:
    ANTHROPIC_API_KEY: set (sk-ant-...a8Qf) [source: keychain]
    OPENAI_API_KEY: set (sk-...ef12) [source: .env]
    OPENROUTER_API_KEY: not set (optional, for multi-provider routing)
```

The check:
1. Resolves each known secret using the full resolution chain
2. Prints the last 4 characters (masked) for identification
3. Shows the source (env, .env, keychain, config file)
4. Marks optional secrets as optional (not failures)

---

## Current Status

Secret management is partially implemented:

- **Environment variable resolution**: Fully wired in `roko-agent` for all LLM provider backends
- **dotenvy loading**: Not yet wired (the `dotenvy` crate is not yet a workspace dependency)
- **OS keychain**: Not yet wired (the `keyring` crate is not yet a workspace dependency)
- **${VAR} interpolation**: Not yet implemented in the config parser
- **_FILE suffix support**: Not yet implemented
- **roko doctor secret check**: Scaffold exists, not yet checking all sources

These are straightforward additions that can be wired incrementally as daemon mode development
progresses.
