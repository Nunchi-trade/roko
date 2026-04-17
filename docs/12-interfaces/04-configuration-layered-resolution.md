# Configuration — Layered Resolution

> `roko.toml` format and the layered resolution system: CLI flags → environment variables → config file → defaults. Minimal config, override only what you need.


> **Implementation**: Scaffold

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [00-cli-overview.md](./00-cli-overview.md)
**Key sources**: `refactoring-prd/06-interfaces.md` §1, `refactoring-prd/10-developer-guide.md` §1, `roko-cli/src/config.rs`, `bardo-backup/prd/shared/config-reference.md`

---

## Abstract

Roko configuration follows the **convention over configuration** principle: sensible defaults for everything, with a layered override system that lets users change only what they need. The configuration system resolves values from four layers, highest priority first: CLI flags, environment variables (`ROKO_*` prefix), the `roko.toml` file, and compiled-in defaults.

The `roko.toml` file is the primary configuration surface for runtime behavior, but REF17 makes
plugin discovery separate from configuration. Standard plugin installs land under `plugins/**`
and are discovered automatically; `roko.toml` provides overrides and site policy, not the
default install path. See [14-plugin-sdk.md](../18-tools/14-plugin-sdk.md),
[16-plugin-loading.md](../18-tools/16-plugin-loading.md),
[01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md), and
[tmp/refinements/17-plugin-extension-architecture.md](../../tmp/refinements/17-plugin-extension-architecture.md).

---

## Resolution Order

Values are resolved from four layers, highest priority first:

| Priority | Source | Example |
|---|---|---|
| 1 (highest) | **CLI flags** | `roko run --model claude-opus-4-6 "prompt"` |
| 2 | **Environment variables** | `ROKO_MODEL=claude-opus-4-6` |
| 3 | **`roko.toml`** | `[agent] model = "claude-sonnet-4-6"` |
| 4 (lowest) | **Compiled defaults** | `claude-sonnet-4-6` |

This layering means:
- A developer can run `roko run "prompt"` with zero configuration (defaults handle everything)
- A project can standardize settings in `roko.toml` (checked into version control)
- A CI system can override per-run with environment variables
- A specific invocation can override everything with CLI flags

For plugins, this layering applies after discovery. The loader first walks `plugins/**`, reads
manifests, validates permissions, and classifies the plugin tier. Config layers then decide
which plugins are preferred, constrained, or disabled for a given deployment.

### Environment Variable Convention

Every `roko.toml` key maps to an environment variable with the `ROKO_` prefix and underscore-separated path:

| Config key | Environment variable |
|---|---|
| `agent.model` | `ROKO_AGENT_MODEL` |
| `agent.command` | `ROKO_AGENT_COMMAND` |
| `gates.pipeline` | `ROKO_GATES_PIPELINE` |
| `router.type` | `ROKO_ROUTER_TYPE` |
| `server.port` | `ROKO_SERVER_PORT` |
| `daimon.enabled` | `ROKO_DAIMON_ENABLED` |
| `neuro.enabled` | `ROKO_NEURO_ENABLED` |

---

## `roko.toml` Schema

### Minimal Configuration

For most Rust projects, this is all you need:

```toml
[agent]
model = "claude-sonnet-4-6"
```

Everything else is auto-detected or uses defaults.

For plugins, the minimal path is often no config at all:

```bash
roko plugin install cargo.udeps
roko plugin audit
```

That flow installs a manifest-backed plugin into `./plugins`, after which the loader discovers
it automatically on the next run.

### Full Configuration Reference

```toml
# ════════��════════════════════════════════���═════════════════════
# Agent — LLM backend configuration
# ═══════════════════════════════════════��═══════════════════════

[agent]
command = "claude"                      # Agent backend command
model = "claude-sonnet-4-6"             # Default model
args = ["--print"]                      # Additional arguments
mcp_config = ".roko/mcp-servers.json"   # MCP server configuration path

# ════════════════════════════════════════��══════════════════════
# Substrate — Engram persistence
# ═════════════════════════════════════════════════��═════════════

[substrate]
type = "file"                           # Persistence backend: "file" (JSONL)
path = ".roko/signals.jsonl"            # Storage path

# ═══════════════════════════════════════════════════════════════
# Gates — Verification pipeline (L3 Harness)
# ═══════════════════════════════════════════════════════════��═══

[gates]
pipeline = ["compile", "test", "clippy"]  # Gate sequence
max_retries = 3                           # Retries on gate failure

[gates.compile]
command = "cargo build"
timeout_ms = 60000

[gates.test]
command = "cargo test"
timeout_ms = 120000

[gates.clippy]
command = "cargo clippy --no-deps -- -D warnings"
timeout_ms = 60000

# ═══════════════════════════════════════════════════════════════
# Router — Model routing (L1 Framework)
# ═══════════════════════════════════════���═══════════════════════

[router]
type = "cascade"                        # Routing strategy: "cascade", "fixed", "random"

[[router.tiers]]
model = "claude-haiku-4-5"
min_confidence = 0.9

[[router.tiers]]
model = "claude-sonnet-4-6"
min_confidence = 0.7

[[router.tiers]]
model = "claude-opus-4-6"
min_confidence = 0.0

# ═══════════════════════════════════════════════════════════════
# Composer — Context engineering (L2 Scaffold)
# ═════════════════════════════════════��═════════════════════════

[composer]
budget_tokens = 50000                   # Maximum context window tokens
role = "implementer"                    # Agent role for system prompt

# ══════════════════════════════════════════════════���════════════
# Daimon — Affect engine (cognitive cross-cut)
# ═══════════════════════════════════════════════════════════════

[daimon]
enabled = true
half_life_hours = 4                     # PAD vector decay half-life

# ══════════════════════��════════════════════════════��═══════════
# Neuro — Knowledge persistence (cognitive cross-cut)
# ═══════════════════════════════════════════════════════════════

[neuro]
enabled = true
knowledge_path = ".roko/neuro/"
gc_min_confidence = 0.1                 # Minimum confidence for GC retention

# ════════════════════════════════════════════���══════════════════
# Dreams — Offline consolidation (cognitive cross-cut)
# ═══════════════════════════════════════════════════���═══════════

[dreams]
enabled = false
schedule = "idle"                       # "idle", "nightly", "every_6h"

# ══════════════════════════════════════════════��════════════════
# Providers — LLM provider configuration
# ═══════════════════════════════════════��═══════════════════════

[providers.anthropic]
api_key_env = "ANTHROPIC_API_KEY"
base_url = "https://api.anthropic.com"

[providers.openrouter]
api_key_env = "OPENROUTER_API_KEY"
base_url = "https://openrouter.ai/api/v1"

# ═══════════════════��═══════════════════════════════════════════
# Server — HTTP API configuration (for `roko serve`)
# ══════════════════════════════════════════��════════════════════

[server]
bind = "127.0.0.1"
port = 9090
cors_origins = ["http://localhost:3000"]

[serve.auth]
enabled = false
api_key = ""

# ══════════════════════════════════════════════���════════════════
# Scheduler — Cron-based event sources
# ═════════════════════════════════════════════���═════════════════

[[scheduler]]
name = "nightly-review"
cron = "0 2 * * *"
template = "code-reviewer"

# ════════════════════════════════════════════════════���══════════
# Watcher — File-based event sources
# ══════════════════════════��═══════════════════════════════════��

[[watcher]]
name = "prd-watcher"
paths = [".roko/prd/"]
pattern = "*.md"
template = "prd-ingestion"

# ═══════════════════════════════════════════════════════════════
# Deploy — Cloud deployment configuration
# ══════════════════════════════════════���═══════════════════��════

[deploy]
backend = "manual"                      # "manual", "railway"
```

---

## Auto-Detection

When `roko init` runs without a `--template` flag, it scans the project directory to detect the development environment:

| Detection target | Files checked | Result |
|---|---|---|
| Rust | `Cargo.toml` | Gates: `compile` → `cargo build`, `test` → `cargo test`, `clippy` → `cargo clippy` |
| Node.js | `package.json` | Gates: `compile` ��� `npm run build`, `test` → `npm test` |
| Go | `go.mod` | Gates: `compile` → `go build ./...`, `test` → `go test ./...` |
| Python | `pyproject.toml`, `setup.py` | Gates: `test` → `pytest` |

Auto-detection results populate the `[gates]` section of `roko.toml`. Users can override any auto-detected value.

---

## Current Status and Gaps

The layered configuration system is **fully implemented** in `roko-cli/src/config.rs`. The `load_layered` function implements the four-layer resolution. Auto-detection is implemented for Rust, Node.js, Go, and Python.

**Gaps:**
- `[providers.*]` section validation and migration not yet implemented (Tier 1L)
- Hot-reload of configuration during TUI mode not yet implemented
- Interactive config wizard not yet implemented (Tier 4)

---

## Cross-References

- See [00-cli-overview.md](./00-cli-overview.md) for CLI modes
- See [03-progressive-help-and-explain.md](./03-progressive-help-and-explain.md) for `roko config wizard`
- See topic [02-agents](../02-agents/INDEX.md) for provider configuration
- See topic [05-learning](../05-learning/INDEX.md) for cascade router config
