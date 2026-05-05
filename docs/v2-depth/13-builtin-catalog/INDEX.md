# 13-builtin-catalog — Depth Index

Depth for [14-TOOLS.md](../../unified/14-TOOLS.md) (formerly 13-BUILTIN-BLOCK-CATALOG.md)

---

## Source docs (18) — All Absorbed

### Tool architecture and builtins

| Source doc | Status | Absorbed into |
|---|---|---|
| `docs/18-tools/00-tool-architecture.md` | Absorbed | → [01-tool-architecture-as-cell-protocol.md](01-tool-architecture-as-cell-protocol.md) |
| `docs/18-tools/01-builtin-tools.md` | Absorbed | → [01-tool-architecture-as-cell-protocol.md](01-tool-architecture-as-cell-protocol.md) |
| `docs/18-tools/02-tool-categories.md` | Absorbed | → [04-domain-tools-and-profiles.md](04-domain-tools-and-profiles.md) |
| `docs/18-tools/03-chain-domain-tools.md` | Absorbed | → [04-domain-tools-and-profiles.md](04-domain-tools-and-profiles.md) |
| `docs/18-tools/05-tool-profiles.md` | Absorbed | → [04-domain-tools-and-profiles.md](04-domain-tools-and-profiles.md) |
| `docs/18-tools/06-wallet-management.md` | Absorbed | → [04-domain-tools-and-profiles.md](04-domain-tools-and-profiles.md) |
| `docs/18-tools/07-tool-testing.md` | Absorbed | → [01-tool-architecture-as-cell-protocol.md](01-tool-architecture-as-cell-protocol.md) |
| `docs/18-tools/08-service-integrations.md` | Absorbed | → [02-mcp-as-connect-protocol.md](02-mcp-as-connect-protocol.md) |

### Safety hooks

| Source doc | Status | Absorbed into |
|---|---|---|
| `docs/18-tools/04-safety-hooks.md` | Absorbed | → [01-tool-architecture-as-cell-protocol.md](01-tool-architecture-as-cell-protocol.md) |

### MCP integrations

| Source doc | Status | Absorbed into |
|---|---|---|
| `docs/18-tools/09-mcp-architecture.md` | Absorbed | → [02-mcp-as-connect-protocol.md](02-mcp-as-connect-protocol.md) |
| `docs/18-tools/10-mcp-github.md` | Absorbed | → [02-mcp-as-connect-protocol.md](02-mcp-as-connect-protocol.md) |
| `docs/18-tools/11-mcp-slack.md` | Absorbed | → [02-mcp-as-connect-protocol.md](02-mcp-as-connect-protocol.md) |
| `docs/18-tools/12-mcp-scripts.md` | Absorbed | → [02-mcp-as-connect-protocol.md](02-mcp-as-connect-protocol.md) |
| `docs/18-tools/13-mcp-stdio.md` | Absorbed | → [02-mcp-as-connect-protocol.md](02-mcp-as-connect-protocol.md) |

### Plugins and event sources

| Source doc | Status | Absorbed into |
|---|---|---|
| `docs/18-tools/14-plugin-sdk.md` | Absorbed | → [03-plugin-spi-as-extension.md](03-plugin-spi-as-extension.md) |
| `docs/18-tools/15-event-sources.md` | Absorbed | → [05-event-sources-and-templates.md](05-event-sources-and-templates.md) |
| `docs/18-tools/15-16-agent-templates.md` | Absorbed | → [05-event-sources-and-templates.md](05-event-sources-and-templates.md) |
| `docs/18-tools/16-plugin-loading.md` | Absorbed | → [03-plugin-spi-as-extension.md](03-plugin-spi-as-extension.md) |

---

## Depth docs (5)

| # | Doc | Core thesis |
|---|---|---|
| **01** | [01-tool-architecture-as-cell-protocol.md](01-tool-architecture-as-cell-protocol.md) | Every tool is a Cell implementing Connect. ToolDef = Cell metadata. Three trust tiers = capability declarations. Safety hook chain = Pipeline of Verify Cells. Capability<T> provides compile-time safety via Rust ownership. |
| **02** | [02-mcp-as-connect-protocol.md](02-mcp-as-connect-protocol.md) | MCP is dynamic Cell discovery via Connect protocol. External processes expose tools as Cells. Namespaced registry merge. Trust hierarchy: in-process > domain plugin > MCP. Tool change notifications = Bus Pulses triggering registry refresh. |
| **03** | [03-plugin-spi-as-extension.md](03-plugin-spi-as-extension.md) | Five-tier plugin SPI as Extension specialization. Each tier has fixed power envelope and sandbox: data-only Signals (1-2), declarative Cells via manifests (3), native ABI implementations (4), WASM Cells with fuel metering (5). Discovery-first loading Pipeline. |
| **04** | [04-domain-tools-and-profiles.md](04-domain-tools-and-profiles.md) | Domain tools as Rack pattern. Two-layer model: 8 adapter-facing tools backed by 423+ protocol handlers = Route Cell to Connect Cell composition. Profiles are Rack macros creating structural absence (not policy filtering). 94% token savings. CascadeRouter prunes to 12 tools/tick. |
| **05** | [05-event-sources-and-templates.md](05-event-sources-and-templates.md) | Event sources as Trigger Cells. Five push-based types (cron, file watch, GitHub, Slack, generic). Agent templates as Graph templates (Rack pattern). Subscriptions bind triggers to templates. Dispatch loop is a React Cell. 18 shipped templates across 3 repo contexts. |

---

## Cross-references

| Related depth section | Connection |
|---|---|
| [17-security/](../17-security/) | Safety hook chain (7 Verify Cells), Capability<T> token, TaintedString, CaMeL IFC |
| [22-code-intelligence/](../22-code-intelligence/) | roko-mcp-code server (code-intelligence MCP), AST parsing, symbol lookup |
| [12-extensions/](../12-extensions/) | Extension trait (SPI surface), 8 layers / 22 hooks, CaMeL capability tags |
| [08-gateway/](../08-gateway/) | CascadeRouter (tool pruning), tier routing (T0/T1/T2), cost estimation |
| [07-learning/](../07-learning/) | Predict-publish-correct on tool Cells, adaptive gate thresholds, A/B experiments |
| [13-triggers/](../13-triggers/) (unified spec) | Trigger protocol, TriggerEngine, push-based design, Conductor watchers |

---

## Key implementation paths

| Component | Crate | Path |
|---|---|---|
| 16 built-in tool definitions | roko-std | `crates/roko-std/src/tool/builtin/mod.rs` |
| Tool registry + role filtering | roko-std | `crates/roko-std/src/tool/registry.rs` |
| Safety hook chain | roko-agent | `crates/roko-agent/src/safety/` |
| MCP client + tool converter | roko-agent | `crates/roko-agent/src/mcp/` |
| Extension trait (SPI) | roko-core | `crates/roko-core/src/extension.rs` |
| Code-intelligence MCP server | roko-mcp-code | `crates/roko-mcp-code/` |
| Event sources (cron, watcher) | roko-serve | `crates/roko-serve/src/scheduler.rs`, `fswatcher.rs` |
| Dispatch loop | roko-serve | `crates/roko-serve/src/state.rs` |
| CascadeRouter (tool pruning) | roko-learn | `crates/roko-learn/src/` |
| Daimon (profile-affect integration) | roko-daimon | `crates/roko-daimon/src/` |
