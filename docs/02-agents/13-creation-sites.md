# 13 — Eight Creation Sites Refactor

> Sub-doc 13 of **02-agents** · Roko Documentation
>
> This document identifies the eight places in the codebase where agents are
> constructed, explains why consolidation into `create_agent_for_model` matters,
> and tracks the migration status.


> **Implementation**: Shipping

---

## The Problem

Agent construction is still split between the shared factory and a handful of
specialized fallbacks. The primary runtime entry points now use
`create_agent_for_model`, and even no-routing subprocess fallbacks now flow
through that factory. The remaining direct paths are concentrated in
backend-specific adapters and a small set of intentional known-protocol
subprocess branches. They still mean:

1. **Inconsistent behavior** — Direct fallback paths can still miss shared
   defaults, options, or safety settings.
2. **Hard to add providers** — The remaining manual paths still require
   backend-aware handling outside the factory.
3. **No single point for routing** — The CascadeRouter can only intercept model
   selection on the factory path.

The refactoring PRD §07-implementation-priorities identifies "8 creation sites"
as a Tier 1 priority for consolidation.

---

## The Eight Sites

### 1. `orchestrate.rs::run_prepared_agent` (line 451)

The primary agent call site. Constructs `ClaudeCliAgent` or `ExecAgent`
based on the command string:

```rust
if cfg.command == "claude" {
    let mut agent = ClaudeCliAgent::new(&cfg.command, &cfg.exec_dir, &cfg.model)
        .with_timeout_ms(cfg.timeout_ms)
        .with_bare_mode(cfg.bare_mode)
        .with_effort(cfg.effort)
        .with_system_prompt(cfg.system_prompt)
        .with_tools(cfg.allowed_tools_csv)
        .with_mcp_config(mcp_path)
        .with_fallback_model(fallback)
        // ... 12 more config lines
        ;
    agent.run(&prompt_signal, &ctx).await
} else {
    let agent = ExecAgent::new(&cfg.command, cfg.extra_args.clone())
        .with_name(&cfg.model);
    agent.run(&prompt_signal, &ctx).await
}
```

**Problem:** This branch is now mostly consolidated, but known protocol
subprocess commands still stay manual when no routing config is present so their
current behavior does not change.

**Fix:** Replace with `create_agent_for_model(config, model_key, options)`.

### 2. `orchestrate.rs` — model selection (AgentRunConfig construction)

Before `run_prepared_agent` is called, the `AgentRunConfig` is constructed
from plan runner state. The model is selected from `roko.toml`
`[agent.roles.<role>].model` with a hardcoded fallback.

**Problem:** The model selection doesn't go through the CascadeRouter or
check the model registry for capabilities.

**Fix:** Route through `resolve_model` → CascadeRouter → provider adapter.

### 3. `run.rs` — single-prompt execution

The `roko run "<prompt>"` command constructs an agent directly for one-shot
execution.

**Problem:** Uses the same known-protocol subprocess fallback as
`orchestrate.rs` when routing config is unavailable.

**Fix:** Use `create_agent_for_model` with the default model from config.

### 4. `prd.rs` — PRD draft/plan generation

The `roko prd draft` and `roko prd plan` commands construct agents for
PRD-related tasks.

**Problem:** May use a different agent construction path than the main
orchestrator.

**Fix:** Standardize on `create_agent_for_model`.

### 5. `research.rs` — research agent

The `roko research` commands construct agents for deep research tasks.

**Problem:** Research agents may need different model profiles and search
options. The routed path now handles Gemini grounding and Perplexity
search-grounded research through the shared factory, but specialty endpoints
such as Perplexity deep research still diverge.

**Fix:** Configure research-specific model entries and use
`create_agent_for_model` with the research model key.

### 6. `agent_exec.rs` — agent execution helper

Internal helper that constructs agents for background tasks.

**Problem:** Still needs to stay aligned with the shared factory options, even
though the actual construction already routes through `create_agent_for_model`.

**Fix:** Consolidate into `create_agent_for_model`.

### 7. Test code — mock and integration tests

Tests construct agents directly (MockAgent, ExecAgent) for specific test
scenarios.

**Status:** This is acceptable. Tests should construct specific agent types
directly for determinism. No consolidation needed.

### 8. Examples and benchmarks

Example code and benchmark harnesses construct agents directly.

**Status:** Acceptable for examples. Should use `create_agent_for_model` in
benchmarks to exercise the full pipeline.

---

## The Target State

After consolidation, agent construction follows one path:

```
Call site (any of the 6 production sites)
    │
    ▼
create_agent_for_model(config, model_key, options)
    │
    ├── resolve_model(config, model_key) → ResolvedModel
    │   ├── Config registry lookup
    │   └── Fallback to slug heuristic
    │
    ├── CascadeRouter may override model_key → different tier
    │
    ├── adapter_for_kind(provider_kind) → &dyn ProviderAdapter
    │
    └── adapter.create_agent(provider, profile, options) → Box<dyn Agent>
```

Benefits:
1. **One place to add providers** — New providers are registered in the
   adapter dispatch table and config, not in call sites.
2. **CascadeRouter intercepts all model selection** — The router can
   override any model choice, enabling tier routing.
3. **Consistent configuration** — All agents get the same treatment:
   timeout, system prompt, tools, MCP, safety.
4. **Easy auditing** — One function to review for security and correctness.

---

## Migration Strategy

The migration is incremental — each call site can be migrated independently:

### Phase 1: Wire `create_agent_for_model` in `orchestrate.rs`

Replace the `run_prepared_agent` function's manual dispatch with:

```rust
async fn run_prepared_agent(cfg: AgentRunConfig, config: &RokoConfig) -> AgentResult {
    let options = AgentOptions {
        timeout_ms: Some(cfg.timeout_ms),
        system_prompt: Some(cfg.system_prompt),
        tools: Some(cfg.allowed_tools_csv),
        mcp_config: cfg.mcp_config,
        env: cfg.env_vars,
        extra_args: cfg.extra_args,
        effort: Some(cfg.effort),
        bare_mode: cfg.bare_mode,
        dangerously_skip_permissions: cfg.skip_permissions,
        name: cfg.model.clone(),
    };
    let agent = create_agent_for_model(config, &cfg.model, options)?;
    let ctx = Context::now();
    let prompt = Engram::builder(Kind::Task).body(Body::Text(cfg.prompt)).build();
    agent.run(&prompt, &ctx).await
}
```

### Phase 2: Migrate run.rs, prd.rs, research.rs

Each command constructs `AgentOptions` from its CLI arguments and calls
`create_agent_for_model`.

### Phase 3: Wire CascadeRouter into the factory path

Add a hook before `adapter_for_kind` that lets the CascadeRouter override
the model key based on task context.

---

## Current Status

| Site | Status | Notes |
|---|---|---|
| orchestrate.rs run_prepared_agent | **Migrated for routed and no-routing paths** | Routed path and generic no-routing subprocesses use `create_agent_for_model`; only known protocol subprocess commands stay manual |
| orchestrate.rs model selection | **Partially migrated** | Routing config now feeds the factory path; known-protocol no-config behavior still stays explicit |
| run.rs | **Migrated for routed and no-routing paths** | Routed path and generic no-routing subprocesses use the shared factory; only known protocol subprocess commands stay manual |
| prd.rs | **Not migrated** | Direct agent construction |
| research.rs | **Partially migrated** | Gemini grounding and Perplexity search-grounded paths now use the shared factory; specialty endpoints still diverge |
| agent_exec.rs | **Migrated** | Background task creation now goes through `create_agent_for_model` |
| Tests | **N/A** | Direct construction is correct |
| provider/mod.rs factory | **Implemented** | `create_agent_for_model` works |

The factory function exists and is tested. The migration is about wiring it
into the call sites, not about building new infrastructure.

---

## Citations

1. Refactoring PRD §07-implementation-priorities — Tier 1: 8 creation sites.
2. `crates/roko-cli/src/orchestrate.rs:451` — Primary agent construction.
3. `crates/roko-agent/src/provider/mod.rs:82` — `create_agent_for_model`.
4. Implementation plan `modelrouting/03-provider-adapters.md` — Unified
   factory design.
5. Implementation plan `modelrouting/19-implementation-guide.md` — 5
   integration points including agent creation.
