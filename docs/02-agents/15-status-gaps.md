# 15 — Current Status and Gaps

> Sub-doc 15 of **02-agents** · Roko Documentation
>
> This document summarizes the current implementation status of the agent
> system, identifies the remaining gaps, and prioritizes the work needed
> to reach full integration.


> **Implementation**: Shipping

---

## Current Status Summary

### What Works

| Component | Status | Evidence |
|---|---|---|
| Agent trait + AgentResult | **Stable** | 6 implementations, all tested |
| ClaudeCliAgent | **Primary backend** | Used by orchestrate.rs for all plan execution |
| Provider registry (TOML) | **Implemented** | ProviderConfig, ModelProfile, resolve_model |
| Provider adapters (4) | **Implemented** | OpenAiCompat, ClaudeCli, AnthropicApi, CursorAcp |
| `create_agent_for_model` factory | **Implemented** | Integration test passes with mock HTTP server |
| ToolLoop (multi-turn) | **Implemented** | 263 lines production + 500 lines tests |
| ToolDispatcher (7-step) | **Implemented** | Full pipeline with audit signals |
| SafetyLayer (6 policies) | **Implemented** | Bash, git, network, path, scrub, rate_limit |
| Translator (4 formats) | **Implemented** | OpenAI, Claude, Ollama, ReAct |
| ChatResponse normalization | **Implemented** | FinishReason, reasoning extraction, metadata |
| MCP client + discovery | **Implemented** | stdio transport, .mcp.json, dedup, dynamic registry |
| MCP passthrough to Claude CLI | **Wired** | --mcp-config flag in orchestrate.rs |
| Agent pools (single + multi) | **Implemented** | AgentPool, MultiAgentPool, warm pool |
| OllamaLlmBackend | **Implemented** | Proves LlmBackend pattern works |
| RetryAction + should_retry | **Implemented** | Error classification → retry policy |
| OpenRouter metadata | **Implemented** | fetch_model_metadata for dynamic discovery |
| Error classification | **Implemented** | Per-adapter classify_error → ProviderError |
| AgentRole (28 roles) | **Implemented** | With backend, tier, budget, permission defaults |
| CascadeRouter | **Wired** | Persists to .roko/learn/cascade-router.json |
| Episode logging | **Wired** | Agent turns + gate results → .roko/episodes.jsonl |
| Efficiency tracking | **Wired** | Per-turn metrics → .roko/learn/efficiency.jsonl |
| Adaptive gate thresholds | **Wired** | EMA per rung → .roko/learn/gate-thresholds.json |

### What Is Built But Not Wired

| Component | Gap | Impact |
|---|---|---|
| `create_agent_for_model` | Not called from orchestrate.rs | 8 creation sites use manual dispatch |
| ToolDispatcher + SafetyLayer | Not called from orchestrate.rs | Safety bypassed for primary path |
| ToolLoop | No LlmBackend impls for HTTP providers | HTTP agents can't use tool loop |
| MultiAgentPool | Not used by orchestrate.rs | Agents created on-demand, not pooled |
| Temperament | Config field exists, not propagated | No behavioral dial connected |
| ChatResponse | Lives in roko-agent, not roko-core | roko-compose can't use typed responses |
| Role prompts | ~1 sentence each | Mori used ~2K tokens per role |

---

## Gap Analysis: Priority Order

### Gap #1: ToolDispatcher Never Called from Orchestrate.rs

**Severity:** Critical
**Component:** Safety pipeline
**Status:** SafetyLayer is wired into ToolDispatcher. ToolDispatcher is wired
into ToolLoop. ToolLoop exists and works. But none of this code is reached
from the primary execution path.

**Why:** `orchestrate.rs` constructs `ClaudeCliAgent` directly, and Claude
CLI drives its own internal tool loop. The `ToolDispatcher` + `SafetyLayer`
+ `ToolLoop` pipeline is only reachable for agents that go through the
ToolLoop (e.g., Ollama via `OllamaLlmBackend`).

**Fix:** Two complementary approaches:
1. Wire HTTP backends through `create_agent_for_model` → adapter →
   `LlmBackend` → `ToolLoop` → `ToolDispatcher` → `SafetyLayer`.
2. For Claude CLI: apply `SafetyLayer` policies at the orchestrator level
   (pre-prompt validation) rather than at the ToolDispatcher level.

**Reference:** Implementation plan `11-inconsistencies.md`, Gap #1.

### Gap #2: Eight Creation Sites Not Consolidated

**Severity:** High
**Component:** Agent construction
**Status:** `create_agent_for_model` exists and works. The 6 production call
sites still use manual dispatch.

**Fix:** Migrate each call site to `create_agent_for_model` (see sub-doc 13).

### Gap #3: LlmBackend Implementations for HTTP Providers

**Severity:** High
**Component:** ToolLoop integration
**Status:** `LlmBackend` trait defined. `OllamaLlmBackend` implemented.
No `OpenAiCompatBackend`, `AnthropicApiBackend`, or `GeminiBackend`.

**Fix:** Implement `LlmBackend` for each HTTP provider, following the
`OllamaLlmBackend` pattern. See sub-doc 07, "What Is Missing."

### Gap #4: ChatResponse Types in Wrong Crate

**Severity:** Medium
**Component:** Type system layering
**Status:** `ChatResponse`, `FinishReason`, `ResponseMetadata` live in
`roko-agent::translate`. `roko-compose` needs them but can't depend on
`roko-agent`.

**Fix:** Move to `roko-core`. See sub-doc 03.

### Gap #5: Role Prompts Are Minimal

**Severity:** Medium
**Component:** Prompt quality
**Status:** Role prompt templates are ~1 sentence each. Mori's role prompts
were ~2K tokens with detailed behavioral instructions.

**Fix:** Expand role templates in `roko-compose/src/templates/`. The
`SystemPromptBuilder` infrastructure exists; the content needs work.

### Gap #6: Temperament Not Propagated

**Severity:** Low (Tier 2)
**Component:** Configuration
**Status:** Config field exists. Not read by runtime.

**Fix:** Wire temperament into gate thresholds, tool selection, model
routing parameters, and review depth.

### Gap #7: MultiAgentPool Not Used

**Severity:** Low
**Component:** Agent lifecycle
**Status:** Pool infrastructure exists. Orchestrator creates agents on-demand.

**Fix:** Migrate orchestrator to use MultiAgentPool for warm-pool and
concurrency management.

---

## What's Next: The Integration Path

The gaps form a dependency chain:

```
Gap #3 (LlmBackend impls) → enables →
Gap #1 (ToolDispatcher + SafetyLayer) → enables →
Gap #2 (creation site consolidation) → enables →
Gap #7 (pool usage) → enables →
Gap #6 (temperament propagation)

Independently:
Gap #4 (ChatResponse to roko-core)
Gap #5 (role prompt expansion)
```

The critical path is: implement `LlmBackend` for HTTP providers → wire
ToolLoop into the factory path → consolidate creation sites. Once this is
done, every agent — regardless of backend — goes through the same safety
pipeline and routing logic.

---

## Metrics

| Metric | Current value | Target |
|---|---|---|
| Agent backends | 6 (Claude CLI, Claude API, OpenAI, Ollama, Cursor, Exec) | 6 (stable) |
| Provider adapters | 4 (OpenAiCompat, ClaudeCli, AnthropicApi, CursorAcp) | 4 (stable) |
| Translators | 4 (OpenAI, Claude, Ollama, ReAct) | 4 (stable) |
| LlmBackend impls | 1 (Ollama) | 4 (+ OpenAI, Anthropic, Gemini) |
| Creation sites consolidated | 0 / 6 production sites | 6 / 6 |
| Safety coverage | ~30% (only ToolLoop path) | 100% (all paths) |
| Role prompt tokens | ~20 per role | ~2000 per role |
| Provider integrations tested | 4 (Anthropic, Claude CLI, OpenAI, GLM) | 8+ |

---

## Test Coverage Summary

The agent system has substantial test coverage for implemented components:

| Component | Tests | Lines |
|---|---|---|
| ToolLoop | 9 async tests | ~200 lines |
| ToolDispatcher | 12 async tests | ~400 lines |
| SafetyLayer | 7 sync tests | ~100 lines |
| Provider factory | 2 tests (sync + async) | ~100 lines |
| Translate module | 16 tests | ~200 lines |
| Agent trait | 4 tests | ~50 lines |

Test patterns used:
- **Mock HTTP server** — `spawn_chat_server` in `provider/mod.rs` creates a
  local TCP listener that serves a predetermined response, enabling integration
  tests without real API keys.
- **Mock translator** — `MockTranslator` in `tool_loop/mod.rs` provides a
  minimal Translator implementation for testing the loop independently of
  wire format.
- **Mock backends** — `FinalAnswerBackend`, `TwoStepBackend`,
  `AlwaysToolCallBackend`, `ErrorBackend`, `ParallelCallsBackend`,
  `CapturingBackend` cover all stop conditions.
- **Mock handlers** — `EchoHandler`, `SleepHandler`, `HugeHandler`,
  `CancellingHandler` cover success, timeout, truncation, and cancellation.

### Missing test coverage

- **No integration tests for Perplexity, Gemini, Kimi** — Config entries are
  ready but no mock servers exercise the provider-specific response extensions.
- **No end-to-end test from orchestrate.rs through ToolLoop** — The #1 gap
  means this path is unreachable and therefore untested.
- **No temperament propagation tests** — Temperament is not wired so there
  is nothing to test.
- **No pool tests under concurrency** — MultiAgentPool tests exist but don't
  exercise concurrent agent execution.

---

## Relationship to Self-Hosting

The agent system is the execution engine for Roko's self-hosting workflow.
Every step in the self-hosting loop requires agents:

```
roko prd draft  → Agent (Researcher/Planner role)
roko prd plan   → Agent (Planner role)
roko plan run   → Agent (Implementer/Reviewer/Tester roles)
roko research   → Agent (Researcher role)
```

The gaps identified in this document directly impact self-hosting quality:

- **Gap #1 (safety)** — Self-hosting requires agents that can safely edit
  the roko codebase. Without SafetyLayer enforcement on the primary path,
  agents can make destructive changes.
- **Gap #2 (creation sites)** — Self-hosting requires model routing (use
  cheap models for easy tasks, expensive models for hard ones). Without
  consolidated creation sites, the CascadeRouter can't intercept.
- **Gap #5 (role prompts)** — Self-hosting quality depends on prompt quality.
  The current ~1-sentence role prompts don't carry enough context for
  agents to make good decisions about roko's own architecture.

Closing these gaps is the path from "roko can execute plans" (current state)
to "roko can execute plans well" (target state).

---

## Citations

1. Implementation plan `11-inconsistencies.md` — Gap #1 analysis.
2. Implementation plan `modelrouting/13-architectural-gaps.md` — 33 gaps.
3. Implementation plan `modelrouting/14-integration-refinements.md` —
   LlmBackend implementations needed.
4. Refactoring PRD §07-implementation-priorities — Tier 1 and Tier 2 tasks.
5. `crates/roko-cli/src/orchestrate.rs` — Primary execution path.
6. `crates/roko-agent/src/provider/mod.rs` — Factory function.
7. `crates/roko-agent/src/tool_loop/mod.rs` — ToolLoop + LlmBackend trait.
8. `crates/roko-agent/src/dispatcher/mod.rs` — ToolDispatcher pipeline.
9. `crates/roko-agent/src/safety/mod.rs` — SafetyLayer.
