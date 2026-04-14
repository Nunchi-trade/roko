# Topic 18 — Tools, Plugins & Integrations

> The complete tool system: architecture, built-in tools, domain plugins, MCP integration,
> service integrations, agent templates, event sources, and plugin SDK.

---

## Sub-documents

| # | File | Title | Lines | Summary |
|---|---|---|---|---|
| 00 | [00-tool-architecture.md](00-tool-architecture.md) | Tool Architecture | ~350 | ToolDef pattern, ToolContext, ToolResult, three trust tiers (Read/Write/Privileged), Capability<T> compile-time safety, speculation engine, event bus integration, DecisionCycleRecord, relationship to Synapse Architecture |
| 01 | [01-builtin-tools.md](01-builtin-tools.md) | Built-in Tools (roko-std) | ~250 | 16 built-in tools: read_file, write_file, edit_file, multi_edit, glob, grep, bash, ls, web_fetch, web_search, notebook_edit, todo_write, task, exit_plan_mode, apply_patch, run_tests. StaticToolRegistry, role-based filtering, module structure |
| 02 | [02-tool-categories.md](02-tool-categories.md) | Tool Categories Taxonomy | ~250 | 17 chain domain categories, prefix conventions, chain support matrix, risk tier scale (Layer1/2/3), tool module breakdown (423+ tools), profile-to-category mapping, capability gating |
| 03 | [03-chain-domain-tools.md](03-chain-domain-tools.md) | Chain Domain Plugin | ~280 | 423+ DeFi tools as ONE domain plugin. Two-layer tool model, adapter pattern, profile-specific adapter sets, Alloy integration, TypeScript sidecar, Revm simulation, tool pruning, mirage-rs |
| 04 | [04-safety-hooks.md](04-safety-hooks.md) | Safety Hooks & Capability Tokens | ~280 | Capability<T> 8-step flow, compile-time safety, SafetyHook trait, 7-hook chain (PolicyCage → AllowlistGuard → SpendingLimiter → RateLimiter → RevmSimulator → HallucinationDetector → ResultFilter), WASM sandbox, TaintedString, audit trail |
| 05 | [05-tool-profiles.md](05-tool-profiles.md) | Tool Profiles & Configuration | ~250 | 13 chain domain profiles, profile filtering mechanism, configuration hierarchy (CLI > env > config > defaults), environment variables (ROKO_ prefix), three-tier API key model, data sources, caching, error taxonomy |
| 06 | [06-wallet-management.md](06-wallet-management.md) | Wallet Management | ~250 | Three custody modes (Delegation/Embedded/LocalKey), WalletHandle abstraction, 7 wallet providers, session keys (ERC-7715), wallet tools, identity NFT, credential lifecycle |
| 07 | [07-tool-testing.md](07-tool-testing.md) | Tool Testing Strategy | ~280 | Four-layer testing: SessionShim, unit tests, property-based (proptest), evaluation tests (66 LLM tool selection tests), red-team tests (OWASP Agentic Top 10 + DeFi attacks), CI pipeline |
| 08 | [08-service-integrations.md](08-service-integrations.md) | Service Integrations | ~230 | Three-layer integration architecture, chain domain services (MetaMask, Uniswap API, Venice, Bankr, AgentCash), operations adapters (Slack, GitHub, Linear), bounty program, structural vs decorative classification |
| 09 | [09-mcp-architecture.md](09-mcp-architecture.md) | MCP Integration Architecture | ~230 | JSON-RPC 2.0 over stdio, MCP client in roko-agent, tool discovery + conversion, dynamic tool registry (merged), config passthrough, security trust model |
| 10 | [10-mcp-github.md](10-mcp-github.md) | roko-mcp-github | ~250 | 17 tools: 6 PR tools (get/create/review/merge/update/list), 6 issue tools (get/create/update/list/comment/search), 4 repo tools (get_file/search_code/list_branches/get_tree), 1 CI tool (get_check_status). Full JSON Schema |
| 11 | [11-mcp-slack.md](11-mcp-slack.md) | roko-mcp-slack | ~230 | 8 tools: post_message, update_message, reply_thread, add_reaction, upload_file, get_channel_history, get_thread, lookup_user. Socket Mode + HTTP Mode, rate limits, Block Kit |
| 12 | [12-mcp-scripts.md](12-mcp-scripts.md) | roko-mcp-scripts | ~230 | Config-driven tool wrappers, scripts.toml format, executor with timeout/isolation, collaboration repo scripts (6), knowledge-base scripts (5), discovery mechanism |
| 13 | [13-mcp-stdio.md](13-mcp-stdio.md) | roko-mcp-stdio | ~210 | Scaffold crate: McpToolHandler trait, McpServerBuilder, JSON-RPC protocol handler, tool registry, error codes, middleware extension points |
| 14 | [14-plugin-sdk.md](14-plugin-sdk.md) | roko-plugin SDK | ~280 | EventSource, FeedbackCollector, Integration traits. 8-step domain plugin pattern (medical domain walkthrough), three plugin loading mechanisms, plugin lifecycle |
| 15 | [15-event-sources.md](15-event-sources.md) | Event Sources | ~280 | 5 event source types: Cron (tokio-cron-scheduler), FileWatch (notify), GitHub webhooks, Slack events, generic webhooks. Subscription configuration, dispatch loop, multi-repository support |
| 15-16 | [15-16-agent-templates.md](15-16-agent-templates.md) | Agent Templates | ~420 | 18 templates: 6 collaboration (doc-lifecycle, digest, meeting, sync, conflict-detector, freshness), 5 knowledge-base (pm-board, enrich, triage, pm-health, action-tracker), 7 roko (pr-review, slack-notify, auto-plan, code-implementer, gate-fixer, prd-ingestion, review-response). Full system prompts, triggers, subscription summary |
| 16 | [16-plugin-loading.md](16-plugin-loading.md) | Plugin Loading Mechanisms | ~250 | Three mechanisms: Cargo workspace (compile-time), config-declared (runtime dynamic linking), MCP discovery (runtime IPC). Comparison matrix, lifecycle, health monitoring, recommended strategy |

---

## Key Cross-References

| Concept | Primary Doc | Also Referenced In |
|---|---|---|
| Synapse Architecture (6 traits) | `docs/01-synapse-architecture/` | 00-tool-architecture |
| Engram (universal data type) | `docs/01-synapse-architecture/` | 00-tool-architecture, 04-safety-hooks |
| Universal Cognitive Loop | `docs/01-synapse-architecture/` | 00-tool-architecture, 07-tool-testing |
| Five Layers | `docs/01-synapse-architecture/` | 00-tool-architecture, 15-event-sources |
| Agent Types & Domains | `docs/05-agent-types/` | 03-chain-domain-tools, 14-plugin-sdk |
| Daimon (behavioral states) | `docs/07-daimon/` | 04-safety-hooks, 05-tool-profiles |
| Neuro (knowledge) | `docs/06-neuro/` | 00-tool-architecture, 03-chain-domain-tools |
| Dreams (offline learning) | `docs/08-dreams/` | 02-tool-categories, 05-tool-profiles |
| Innovations (16 T0 Probes, etc.) | `docs/09-innovations/` | 00-tool-architecture, 14-plugin-sdk |
| Developer Guide | `docs/10-developer-guide/` | 14-plugin-sdk, 16-plugin-loading |
| C-Factor | `docs/12-c-factor/` | 15-16-agent-templates |
| Interfaces (CLI, TUI) | `docs/06-interfaces/` | 00-tool-architecture |
| Korai/Daeji (chain) | `docs/15-chain/` | 06-wallet-management |

---

## Source Material

### Refactoring PRD Sources

- `refactoring-prd/05-agent-types.md` — Agent roles, chain agent heartbeat, multi-agent orchestration, extensibility
- `refactoring-prd/10-developer-guide.md` — Plugin system (§6), integration patterns (§7)
- `refactoring-prd/06-interfaces.md` — CLI scaffolders for new domains

### Legacy Sources

- `bardo-backup/prd/07-tools/00-overview.md` — 423+ DeFi tools, ToolDef pattern, personas, goals
- `bardo-backup/prd/07-tools/01-architecture.md` — Three trust tiers, Capability<T>, safety hooks, Revm, WASM sandbox, profiles, sidecar
- `bardo-backup/prd/07-tools/20-config.md` — Configuration hierarchy, env vars, custody, safety config
- `bardo-backup/prd/07-tools/21-profiles.md` — 13 profiles, 17 categories, profile-category matrix
- `bardo-backup/prd/07-tools/22-wallets.md` — Three custody modes, WalletHandle, session keys, providers
- `bardo-backup/prd/07-tools/24-testing.md` — Four-layer testing strategy
- `bardo-backup/prd/21-integrations/00-overview.md` — Service integration bounties, dependency graph
- `bardo-backup/tmp/mori-agents/14-service-integrations.md` — Slack integration details

### Implementation Plans

- `tmp/implementation-plans/07-mcp-tool-wiring.md` — MCP client wiring phases
- `tmp/implementation-plans/11-agent-dogfooding.md` — 9 phases, 16+ agent templates
- `tmp/implementation-plans/11-sections/phase-2.md` — MCP server specs (GitHub 17 tools, Slack 8 tools, scripts)
- `tmp/implementation-plans/11-sections/phase-3-4.md` — Agent templates, subscriptions, scheduler, file watcher

### Active Code

- `crates/roko-std/src/tool/builtin/mod.rs` — 16 built-in tools (confirmed count)
- `crates/roko-std/src/tool/registry.rs` — StaticToolRegistry, role-based filtering
- `crates/roko-std/src/tool/mod.rs` — Module structure and re-exports
- `crates/roko-agent/src/mcp/` — MCP client implementation (wired)

---

## Naming Map Applied

| Old Name | New Name | Notes |
|---|---|---|
| Golem / Golems | Agent / Agents | Throughout all docs |
| golem-tools | roko-domain-chain (target) | Chain domain plugin crate |
| Grimoire | Neuro / NeuroStore | Knowledge subsystem |
| Styx / Lethe | Agent Mesh / Collective | P2P knowledge sharing |
| GNOS | KORAI (mainnet) / DAEJI (testnet) | Token names |
| golem.toml | roko.toml | Configuration file |
| GOLEM_ env prefix | ROKO_ env prefix | Environment variables |
| Bardo Sanctum | Roko Portal | Web dashboard |
| Event Fabric | Event Bus | Event distribution |
| Pi extensions | Synapse traits | Framework layer |
| Clade | Collective / Mesh | Agent groups |
| Death/mortality phases | Behavioral states (Daimon) | Cyclical, no terminal state |

---

## Generation Notes

- **Built-in tool count**: Code confirms 16 tools in `TOOL_COUNT` constant and registry tests. The `sandbox` module exists but is not counted as a user-facing tool.
- **Agent template count**: 18 templates total (16 original from phase-3-4 plan + 2 additional: prd-ingestion-agent, review-response-agent).
- **Chain domain framing**: All 423+ DeFi tools are framed as ONE domain plugin. The core framework (roko-core, roko-std) is domain-agnostic.
- **MCP server status**: MCP client is built and wired in roko-agent. MCP server crates (roko-mcp-github, roko-mcp-slack, roko-mcp-scripts) are planned with complete specs but not yet implemented as code.
- **Mortality reframe**: All legacy mortality-phase references (Conservation, Declining, Terminal) have been translated to Daimon behavioral states (Struggling, Resting, etc.) per `refactoring-prd/08-translation-guide.md`.
