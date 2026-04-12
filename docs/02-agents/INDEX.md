# 02 — Agents

> Topic index for the Roko agent system documentation.
>
> This topic covers the `Agent` trait, provider registry, provider adapters,
> chat types, agent roles, agent pools, MCP integration, tool loop, harness
> engineering, format translation, temperament profiling, dual-process tier
> routing, extensibility, creation site consolidation, provider integrations,
> and current status.

---

## Sub-documents

| # | Title | File | Summary |
|---|---|---|---|
| 00 | [Agent Trait](00-agent-trait.md) | `00-agent-trait.md` | The `Agent` trait, `AgentResult`, why agents are separate from the 6 Synapse traits, concrete implementations, orchestrator call sites |
| 01 | [Provider Registry](01-provider-registry.md) | `01-provider-registry.md` | Config-driven TOML schema for `[providers.*]` and `[models.*]`, `ProviderConfig`, `ModelProfile`, `ProviderKind`, model resolution |
| 02 | [Provider Adapters](02-provider-adapters.md) | `02-provider-adapters.md` | `ProviderAdapter` trait, 4 adapter implementations, `create_agent_for_model` factory, error classification, `RetryAction` |
| 03 | [Chat Types](03-chat-types.md) | `03-chat-types.md` | `ChatResponse`, `FinishReason`, `ResponseMetadata`, `BackendResponse`, why these types must live in roko-core |
| 04 | [Agent Roles](04-agent-roles.md) | `04-agent-roles.md` | 28-role taxonomy, per-role defaults (backend, tier, budget, permissions), role composition into agent types |
| 05 | [Agent Pools](05-agent-pools.md) | `05-agent-pools.md` | `AgentPool` (sequential), `MultiAgentPool` (parallel), warm-pool pre-spawning, lifecycle states, fallback retry |
| 06 | [MCP Integration](06-mcp-integration.md) | `06-mcp-integration.md` | JSON-RPC stdio client, tool conversion, config discovery, dedup, dynamic registry, Claude CLI passthrough |
| 07 | [Tool Loop](07-tool-loop.md) | `07-tool-loop.md` | `ToolLoop` multi-turn driver (**already exists**), `LlmBackend` trait, `ToolDispatcher` 7-step pipeline, `SafetyLayer`, integration gap |
| 08 | [Harness Engineering](08-harness-engineering.md) | `08-harness-engineering.md` | Meta-Harness research (Lee et al., 2026), 6 harness principles, +7.7/+4.7/4× evidence, mapping to Roko, remaining gaps |
| 09 | [Format Translation](09-format-translation.md) | `09-format-translation.md` | `Translator` trait, 4 translators (OpenAI/Claude/Ollama/ReAct), wire format types, model capabilities, reasoning extraction |
| 10 | [Temperament Profiling](10-temperament-profiling.md) | `10-temperament-profiling.md` | Conservative/Balanced/Aggressive/Exploratory dial, controls for model params, tool selection, gates, review, routing |
| 11 | [Dual-Process Routing](11-dual-process-routing.md) | `11-dual-process-routing.md` | System 1/System 2 model, `CascadeRouter`, `LinUCB` bandit, Pareto frontier, Thompson sampling, anomaly detection |
| 12 | [Extensibility](12-extensibility.md) | `12-extensibility.md` | Adding providers, adapters, translators, LlmBackends, 8-step domain plugin process |
| 13 | [Creation Sites](13-creation-sites.md) | `13-creation-sites.md` | 8 agent creation sites, consolidation into `create_agent_for_model`, migration strategy and status |
| 14 | [Provider Integrations](14-provider-integrations.md) | `14-provider-integrations.md` | Perplexity (Sonar), Gemini, ZhipuAI (GLM), Moonshot (Kimi), OpenRouter — API surfaces, config, extensions, status |
| 15 | [Status and Gaps](15-status-gaps.md) | `15-status-gaps.md` | What works, what's built but not wired, 7 prioritized gaps, integration path, metrics |

---

## Key Source Files

| File | What |
|---|---|
| `crates/roko-agent/src/agent.rs` | `Agent` trait, `AgentResult` |
| `crates/roko-agent/src/provider/mod.rs` | Provider adapters, `create_agent_for_model`, `ProviderAdapter`, `RetryAction` |
| `crates/roko-agent/src/provider/openai_compat.rs` | `OpenAiCompatAdapter` |
| `crates/roko-agent/src/provider/claude_cli.rs` | `ClaudeCliAdapter` |
| `crates/roko-agent/src/provider/anthropic_api.rs` | `AnthropicApiAdapter` |
| `crates/roko-agent/src/provider/cursor_acp.rs` | `CursorAcpAdapter` |
| `crates/roko-agent/src/tool_loop/mod.rs` | `ToolLoop`, `LlmBackend`, `StopReason` |
| `crates/roko-agent/src/dispatcher/mod.rs` | `ToolDispatcher`, 7-step pipeline |
| `crates/roko-agent/src/safety/mod.rs` | `SafetyLayer`, 6 policy families |
| `crates/roko-agent/src/translate/mod.rs` | `Translator`, `ChatResponse`, `BackendResponse` |
| `crates/roko-agent/src/mcp/` | MCP client, config, dedup, dynamic registry |
| `crates/roko-agent/src/pool.rs` | `AgentPool`, `AgentInstanceId` |
| `crates/roko-agent/src/multi_pool.rs` | `MultiAgentPool` |
| `crates/roko-core/src/agent.rs` | `AgentRole`, `ProviderKind`, `AgentBackend`, `ModelTier`, `resolve_model` |
| `crates/roko-core/src/config/schema.rs` | `RokoConfig`, `ProviderConfig`, `ModelProfile` |
| `crates/roko-cli/src/orchestrate.rs` | Primary agent call site, `run_prepared_agent` |

---

## Canonical Sources

| Source | What it covers |
|---|---|
| Refactoring PRD §01 | Synapse architecture, Engram, 6 traits, universal loop |
| Refactoring PRD §02 | Five layers, dual-process tier router, temperament |
| Refactoring PRD §05 | Agent types, role compositions, extensibility |
| Refactoring PRD §07 | Implementation priorities, tier 0/1/2 task list |
| Refactoring PRD §08 | Translation guide, naming map |
| Refactoring PRD §10 | Developer guide, plugin system |
| `modelrouting/00-INDEX.md` | 23-doc model routing architecture |
| `modelrouting/01-architecture.md` | Three-layer provider system |
| `modelrouting/02-provider-registry.md` | Registry schema and types |
| `modelrouting/03-provider-adapters.md` | Adapter trait and implementations |
| `modelrouting/04-translator-extensions.md` | ChatResponse, reasoning extraction |
| `modelrouting/11-research-context.md` | RouteLLM, FrugalGPT, AutoMix citations |
| `modelrouting/14-integration-refinements.md` | ToolLoop wiring, LlmBackend impls |
| `modelrouting/19-implementation-guide.md` | 5 integration points |
| `modelrouting/20-perplexity-integration.md` | Perplexity Sonar API surfaces |
| `modelrouting/21-gemini-integration.md` | Gemini 1M context, grounding |
| `11-inconsistencies.md` | Gap #1: SafetyLayer not reached |
| `01-agent-wiring.md` | ExecAgent → ClaudeCliAgent migration |

---

## Key Citations

1. Sumers, T. R. et al. (2023). "Cognitive Architectures for Language Agents."
   arXiv:2309.02427. — CoALA 9-step loop, theoretical basis for Agent trait
   separation.
2. Lee, S. Y. et al. (2026). "Meta-Harness: Harness Engineering for LLM
   Agents." arXiv:2603.28052. — +7.7 text classification, +4.7 IMO math,
   4× fewer tokens, 6× gap (ref [46], SWE-bench mobile).
3. Jimenez, C. E. et al. (2024). "SWE-bench: Can Language Models Resolve
   Real-World GitHub Issues?" — Benchmark context for harness variance.
4. Kahneman, D. (2011). "Thinking, Fast and Slow." — Dual-process theory
   for model tier routing.
5. Li, L. et al. (2010). "A contextual-bandit approach to personalized news
   article recommendation." WWW 2010. — LinUCB algorithm.
6. Chen, L. et al. (2023). "FrugalGPT: How to Use Large Language Models
   While Reducing Cost and Improving Performance." — Cascade routing.
7. Friston, K. (2010). "The free-energy principle: a unified brain theory?"
   Nature Reviews Neuroscience. — Active inference for model routing.
8. Woolley, A. W. et al. (2010). "Evidence for a Collective Intelligence
   Factor in the Performance of Human Groups." Science 330. — C-Factor for
   multi-agent coordination.
9. RouteLLM (2024). — Binary classifier for model routing.
10. MixLLM (2024). — Mixed model serving.
11. AutoMix (2024). — Automatic model mixing.
12. Router-R1 (2025). — RL-trained per-query router.
13. WildToolBench — Format-specific accuracy benchmarks.
14. Qwen3-coder — Documented format switching above 5 tools.
15. Mori (reference orchestrator) — `apps/mori/src/agent/connection.rs`,
    108K LOC reference implementation.

---

## Naming Map (applied throughout)

| Old name | New name | Context |
|---|---|---|
| Bardo | Roko | Project name |
| Golem | Agent | Agent subsystem |
| Mori | Roko Orchestrator | CLI/runtime |
| Grimoire | Neuro | Knowledge system |
| Signal | Engram | Content-addressed unit (rename Tier 0D) |
| Clade | Collective / Mesh | Multi-agent groups |
| GNOS | KORAI / DAEJI | Metrics systems |

---

## Critical Reminders

1. **ToolLoop already exists.** Do not rebuild. What's missing is `LlmBackend`
   implementations for HTTP providers.
2. **Chat types must live in roko-core.** `ChatResponse`, `FinishReason`,
   `ResponseMetadata` currently live in `roko-agent::translate` but are needed
   by `roko-compose`.
3. **ExecAgent is legacy fallback.** The primary backend is `ClaudeCliAgent`;
   `ExecAgent` remains for non-Claude backends pending migration.
4. **SafetyLayer is wired but unreachable.** The #1 integration gap:
   `SafetyLayer` → `ToolDispatcher` → `ToolLoop` pipeline is built but never
   called from `orchestrate.rs`.
5. **Meta-Harness "6× gap"** comes from ref [46] (SWE-bench mobile), not a
   general claim. +7.7 and +4.7 are more representative numbers.
