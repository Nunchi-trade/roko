# Phase B: Feature Build Sequence from PRDs

After Phase A refactoring is complete, these are the features to build from the PRD docs. Ordered by dependency and impact on self-hosting capability.

## Tier 1: Close the Self-Hosting Loop

These three items complete autonomous operation.

### B1: Interactive TUI (ratatui)
- **PRD source**: `docs/07-interfaces/` (29 screens documented)
- **Current state**: ratatui is in deps, `roko dashboard` renders text, no interactive UI
- **What to build**: Wire ratatui into dashboard scaffold, implement key screens (plan progress, agent output, gate status, efficiency metrics)
- **Effort**: Medium
- **Impact**: High — operator interface for monitoring self-development

### B2: Automatic Plan Generation
- **PRD source**: `docs/01-orchestration/`
- **Current state**: the promote-to-plan handoff is wired in `roko-serve`; PRD promotion can queue plan generation automatically, and direct PRD/research/plan-generation runs now emit episodes via `crates/roko-cli/src/agent_exec.rs`
- **What to build**: this item is mostly closed; follow-on work is around higher-order failure feedback and broader safety unification, not the promote/plan trigger itself
- **Effort**: Mostly complete
- **Impact**: High — removes a manual step from the loop

### B3: Failure Feedback Loop
- **PRD source**: `docs/05-learning/`
- **Current state**: Gate failures are logged but don't feed back into plan generation
- **What to build**: When a task fails gates, automatically analyze the failure, enrich the task context, and retry or re-plan
- **Effort**: Medium
- **Impact**: High — closes learn-from-failure, enables true self-correction

---

## Tier 2: Wire Built-But-Disconnected Subsystems

These are all "built but never connected" — the code exists, just needs wiring into the runtime.

### B4: Wire SafetyLayer into ToolDispatcher
- **PRD source**: `docs/06-safety/`
- **Current state**: SafetyLayer is built in roko-agent and now reached on orchestrate provider paths, direct PRD/plan/research agent-exec paths, and `roko run` provider-backed construction
- **What to build**: remaining work is the true lowest-level fallback: plain `ExecAgent` creation when no provider is resolved still sits outside the shared dispatcher/safety pipeline
- **Effort**: Small

### B5: Wire Conductor (Watchers) into Executor
- **PRD source**: `docs/11-observability/`
- **Current state**: 10 watchers + circuit breaker built in roko-conductor, not called
- **What to build**: Call conductor from the executor loop to detect anomalies (cost spikes, latency outliers, stuck tasks)
- **Effort**: Small

### B6: Wire Neuro (Knowledge) into Orchestrator
- **PRD source**: `docs/06-neuro/`
- **Current state**: KnowledgeStore + tiers + HDC built in roko-neuro, not queried during task execution
- **What to build**: Query neuro for relevant knowledge when assembling task context. Write gate results + successful patterns back as knowledge entries.
- **Effort**: Medium

### B7: Wire Daimon (Affect) into Dispatch
- **PRD source**: `docs/09-daimon/`
- **Current state**: PAD vectors + behavioral states built in roko-daimon, not modulating dispatch
- **What to build**: Appraise gate results and task outcomes via daimon. Use affect state to modulate model selection (low confidence → escalate model, high confidence → use cheaper model).
- **Effort**: Medium

---

## Tier 3: New Capabilities

### B8: Code Intelligence MCP Server
- **PRD source**: `docs/08-tools/`
- **Current state**: roko-index has parser + graph + HDC, no MCP server
- **What to build**: MCP server that exposes code intelligence (symbol lookup, dependency graph, semantic search) to agents during task execution
- **Effort**: Medium

### B9: Dream Runner (Offline Consolidation)
- **PRD source**: `docs/10-dreams/`
- **Current state**: Scaffold only in roko-dreams
- **What to build**: NREM replay (prioritized episode replay), integration staging (knowledge tier promotion), basic consolidation loop triggered by idle time
- **Effort**: Large

### B10: Heartbeat (Gamma/Theta/Delta Speeds)
- **PRD source**: `docs/16-heartbeat/`
- **Current state**: Specified in docs, no code
- **What to build**: Three-speed cognitive loop — Gamma (reactive tool calls), Theta (reflective planning), Delta (consolidation)
- **Effort**: Large

---

## Tier 4: Platform Features

### B11: Agent Mesh / Coordination
- **PRD source**: `docs/13-coordination/`
- **What to build**: Multi-agent stigmergy, shared knowledge, collective calibration

### B12: HTTP API (roko-serve)
- **PRD source**: `docs/15-deployment/`
- **What to build**: Production REST + WebSocket API for remote orchestration

### B13: Lifecycle Management
- **PRD source**: `docs/14-lifecycle/`
- **What to build**: Agent creation, configuration, backup, restore, deletion

### B14: Identity & Economy (Korai Chain)
- **PRD source**: `docs/04-chain/`
- **What to build**: ERC-8004 agent identity, on-chain attestation
- **Status**: Deferred until Korai chain launch

---

## Self-Development Strategy

For each item above, the workflow is:

```bash
# 1. Create PRD from the relevant docs section
roko prd idea "Wire Neuro into orchestrator for knowledge-enriched task context"
roko prd draft new "wire-neuro-orchestrator"

# 2. Enrich with codebase context
roko research enhance-prd wire-neuro-orchestrator

# 3. Generate implementation plan
roko prd plan wire-neuro-orchestrator

# 4. Review generated tasks.toml, verify context is sufficient

# 5. Execute
roko plan run plans/

# 6. Monitor and iterate
roko dashboard
```

### Critical: Pre-enrich PRDs for Phase B

Unlike Phase A (mechanical renames), Phase B features require architectural understanding. Before running `roko prd plan`, ensure each PRD includes:

1. **Exact integration points**: which function in which file to modify
2. **Existing API surfaces**: what traits/structs the new code must implement
3. **Test patterns**: how similar features are tested in this codebase
4. **The naming glossary**: so generated code uses correct names

This can be done via `roko research enhance-prd` or manually.
