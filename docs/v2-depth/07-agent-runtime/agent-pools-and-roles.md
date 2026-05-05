# Agent Pools and Roles

> Depth for [05-AGENT.md](../../unified/05-AGENT.md). How agent roles constrain protocol conformance and resource budgets, how pools act as warm Cell caches for fast spawn, and how role composition enables domain profiles.

---

## 1. Roles as Capability Profiles

Every agent in Roko is assigned a **role** -- a named capability profile that determines four things:

1. **Tool permissions** -- what the agent can do (read, write, exec, git, network)
2. **Model tier** -- which class of model it defaults to (Fast, Standard, Premium)
3. **Budget ceiling** -- how much it can spend per turn
4. **System prompt layer** -- which role-specific instructions it receives

In unified terms, a role is a set of **protocol conformance constraints** applied to an Agent Cell. A `Reviewer` role restricts the Agent to the Observe protocol (read-only); an `Implementer` role enables Store (write) + Connect (exec) + Observe (read).

### The 28-role taxonomy

Roles are organized by responsibility group:

| Group | Roles | Tier | Key constraint |
|---|---|---|---|
| **Planning** | Architect, Planner, Researcher | Premium/Standard | Read-only (+ Network for Researcher) |
| **Implementation** | Implementer, Debugger, Optimizer, Migrator | Standard | Full read + write + exec |
| **Review** | Reviewer, Auditor | Standard/Premium | Read-only |
| **Validation** | Tester, Validator, GateKeeper | Standard/Fast | Read + exec (write for Tester) |
| **Orchestration** | Conductor, Coordinator, Monitor | Fast | Read-only, low budget |
| **Specialized** | DocWriter, Translator, Analyst, Explorer | Standard | Domain-specific permissions |
| **Operations** | Deployer, Operator | Standard/Fast | Read + exec (no source write) |

### Tool permissions

Each role declares a `ToolPermissions` struct checked by the ToolDispatcher at step 4 (authorize):

```rust
pub struct ToolPermissions {
    pub read: bool,     // File read, grep, glob
    pub write: bool,    // File write, edit, patch
    pub exec: bool,     // Bash, run_tests
    pub git: bool,      // Git operations
    pub network: bool,  // Web fetch, web search
}
```

The enforcement point is `ToolDispatcher::dispatch()` at step 4:

```rust
// crates/roko-agent/src/dispatcher/mod.rs:198
if !def.permission.satisfied_by(&role_perms) {
    return Err(ToolError::PermissionDenied);
}
```

A `Reviewer` role cannot write files even if the model requests `write_file` -- the dispatcher blocks it with `PermissionDenied`. This is the Verify protocol applied to tool access.

### Model tier

```rust
pub enum ModelTier {
    Fast,      // Haiku-class: classification, watchers, orchestration ($0.01-0.05/run)
    Standard,  // Sonnet-class: implementation, review ($0.10-0.50/run)
    Premium,   // Opus/GPT-5-class: architecture, hard debugging ($1.00-5.00/run)
}
```

The tier is a hint to the CascadeRouter's starting point. As the LinUCB bandit learns which models succeed for which tasks, it may promote or demote a role's effective tier. Defaults are conservative: implementation starts at Standard (cost/quality balance), orchestration overhead starts at Fast (minimize cost).

### Turn budget

```rust
pub struct TurnBudget {
    pub base_usd: f32,      // Per-turn dollar ceiling
    pub multiplier: f32,    // Adjusts for model escalation
}
```

The `multiplier` adjusts for model tier changes:
- Escalation to Premium: multiplier = 2.0x (higher per-token cost)
- De-escalation to Fast: multiplier = 0.6x (lower cost)

Budget table (derived from legacy Mori agent roles, adjusted for 2026 pricing):

| Role | Base USD | Multiplier range |
|---|---|---|
| Architect | $3.00 | 1.0-2.0x |
| Implementer | $1.50 | 0.6-2.0x |
| Researcher | $1.50 | 0.6-2.0x |
| Reviewer | $0.75 | 0.6-1.5x |
| Conductor | $0.10 | 0.6-1.0x |
| Monitor | $0.10 | 0.6-1.0x |
| GateKeeper | $0.30 | 0.6-1.0x |

---

## 2. Role Defaults and Override Hierarchy

### Backend inference

All roles currently default to the Claude backend:

```rust
impl AgentRole {
    pub fn backend(&self) -> AgentBackend {
        // All roles default to Claude
        AgentBackend::Claude
    }
}
```

The `AgentBackend::from_model` heuristic overrides this when a specific model slug is configured:

```toml
[agent.roles.implementer]
model = "claude-opus-4-6"      # -> Claude backend (Premium)

[agent.roles.conductor]
model = "claude-haiku-4-5"     # -> Claude backend (Fast)

[agent.roles.researcher]
model = "sonar-pro"            # -> OpenAI-compat backend (Perplexity)
```

### Override hierarchy

Priority from highest to lowest:

1. **Per-task** configuration in `tasks.toml` (task-level `model_hint`)
2. **Per-role** configuration in `roko.toml` (`[agent.roles.*]`)
3. **Role defaults** in `AgentRole` associated methods

```toml
# roko.toml -- per-role override
[agent.roles.implementer]
role = "code_implementer"
model = "claude-opus-4-6"
tools = ["read_file", "edit_file", "git-*"]
budget = { max_tokens_per_turn = 12000, max_cost_usd_cents_per_turn = 500 }
thresholds = { gate_pass_rate_floor = 0.70 }
routing_overrides = { force_backend = "claude", force_tier = "focused" }
```

---

## 3. Role-Specific System Prompts

The `SystemPromptBuilder` in `roko-compose` constructs multi-layer prompts where the role determines Layer 2:

```
Layer 0: Global context (project name, codebase structure)
Layer 1: Task context (plan, task description, dependencies)
Layer 2: Role context (role-specific instructions and constraints)
Layer 3: Tool context (available tools and their descriptions)
Layer 4: History context (relevant previous outputs)
Layer 5: Meta context (budget remaining, time constraints)
```

Layer 2 is populated from templates in `crates/roko-compose/src/templates/`. Each role has a template describing its persona, constraints, and expected output format.

The actual wiring uses `RoleSystemPromptSpec` in `orchestrate.rs`, which feeds the 9-layer builder with the role template plus task-specific enrichment.

---

## 4. Role Composition: Agent Types

Roles compose into **agent types** for different task categories. Each type assembles multiple roles into a coherent capability set:

### 1. Coding Agent (Implementer + Reviewer + Tester)

The standard development cycle. In practice, a single agent run often combines all three role layers in its system prompt with different weighting per phase.

### 2. Research Agent (Researcher + Analyst + Explorer)

Deep investigation with citations. Has network access for web search. Read-only filesystem access via Explorer role.

### 3. Operations Agent (Deployer + Operator + Monitor)

Has exec permissions for running commands but limited write permissions. Should not edit source code.

### 4. Cross-Domain Agent (Architect + Coordinator + specialists)

Multi-domain tasks spanning crate boundaries. Architect provides system-level context, Coordinator manages dependencies, specialists handle domain-specific implementation.

---

## 5. From Roles to Domain Profiles

Roles answer "what persona should this turn use?" Domain profiles answer "what bundle should a deployment ship for this domain?"

The relationship:

- **Roles** provide per-turn defaults: model tier, budget, permissions
- **Domain profiles** select the default role set and add tools, gates, heuristics, templates, memory shapes
- A deployment can load multiple profiles

Profile composition rules:

1. Merge tools by union unless a profile overrides an identical tool ID
2. Stack gates unless a gate is explicitly scoped to a profile
3. Keep heuristics available to all installed profiles, route by fit
4. Warn on role-name collisions for explicit operator resolution
5. Carry domain context as typed fields rather than free-form prose

---

## 6. Agent Pools as Warm Cell Caches

Roko provides two pool implementations for agent lifecycle management. In unified terms, pools are **Spaces** containing pre-instantiated Agent Cells that can be promoted to active status without cold-start construction.

### AgentPool (single-role, sequential)

Manages a queue of tasks for one agent role. Tasks execute sequentially. Failed tasks retry with a fallback agent (different model tier):

```
Primary (Opus) fails -> Fallback (Sonnet) retries -> Final result
```

### MultiAgentPool (multi-role, concurrent)

Manages multiple `AgentPool` instances for concurrent execution across roles:

```rust
pub struct MultiAgentPool {
    active: HashMap<AgentInstanceId, ActiveEntry>,
    warm: HashMap<(AgentRole, String), WarmEntry>,
    fallbacks: HashMap<AgentRole, Arc<dyn Agent>>,
    concurrency_limits: HashMap<AgentRole, usize>,
    default_concurrency: usize,  // Default: 4
}
```

### AgentInstanceId

Every agent instance gets a unique identifier:

```rust
pub struct AgentInstanceId {
    pub role: AgentRole,
    pub instance: String,  // e.g., "plan42-task3"
}

impl AgentInstanceId {
    pub fn key(&self) -> String {
        format!("{}-{}", self.role.label(), self.instance)
        // e.g., "implementer-plan42-task3"
    }
}
```

### Instance lifecycle

```rust
pub enum InstanceStatus {
    Warm,       // Pre-spawned, waiting for work
    Pending,    // Queued, waiting its turn
    Running,    // Currently executing
    Completed,  // Finished successfully
    Failed,     // Finished with error
    Killed,     // Terminated externally
}
```

State transitions:

```
Warm --work-arrives--> Pending --turn-comes--> Running
                                                  |
                                          +-------+-------+
                                          |               |
                                      Completed        Failed
                                                         |
                                                   +-----+-----+
                                                   |           |
                                             TryFallback    Killed
```

### Warm pool pre-spawning

Agents are constructed and held in memory before work arrives:

```rust
struct WarmEntry {
    agent: Arc<dyn Agent>,
    spawned_at: Instant,
}
```

Warm entries have a time-to-live. `evict_stale_warm` removes entries idle longer than a configurable timeout (default: 5 minutes).

When a task arrives for a role with a warm agent, the pool promotes it to active status instead of constructing a new one -- eliminating cold-start latency.

### Concurrency control

Each role has its own concurrency limit:

```rust
pool.set_concurrency_limit(AgentRole::Architect, 1);    // Serial (expensive)
pool.set_concurrency_limit(AgentRole::Implementer, 4);  // Parallel
pool.set_concurrency_limit(AgentRole::Validator, 8);     // High parallelism (cheap)
```

When a role hits its limit, new tasks queue in `Pending` status until a running instance completes.

### Bulk operations

- **`kill_all()`** -- Terminate all active instances (plan completion or Ctrl-C)
- **`kill_by_plan(plan_id)`** -- Terminate instances matching a plan (plan failure)
- **`kill_by_role(role)`** -- Terminate all instances of a specific role

These work through the `ProcessSupervisor` in `roko-runtime`: SIGTERM -> wait -> SIGKILL for graceful shutdown.

### Pool hierarchy

```
MultiAgentPool
    |
    +-- AgentPool (per role)
    |   +-- AgentInstanceId + status tracking
    |   +-- fallback retry logic
    |
    v
create_agent_for_model() -> Box<dyn Agent>
    |
    v
Agent::run() -> AgentResult
    |
    +-- ClaudeCliAgent -> ProcessSupervisor (subprocess)
    +-- OpenAiAgent    -> HTTP client (no supervisor)
    +-- OllamaAgent    -> HTTP client (no supervisor)
```

---

## 7. Agent Composition

Agents compose in two fundamentally different ways:

### Compilation: Multi-Agent -> Single-Agent

Merging a multi-agent team into a single agent with a skill library reduces token consumption by 53.7% and latency by 50% (arXiv:2601.04748, 2025). The savings come from eliminating redundant context repetition across agent calls.

**Phase transition at 50-100 skills**: Semantic confusability causes selection failures as libraries grow. SkillReducer (arXiv:2603.29919) achieves 86% pass rate across 600 skills via taxonomy-driven progressive disclosure.

### Coordination: Agent Pipelines and Meshes

Five coordination patterns (2025 consensus):

| Pattern | Topology | Roko mapping |
|---|---|---|
| **Orchestrator-Worker** | Central coordinator fans out | PlanRunner (current model) |
| **Pipeline** | Sequential stages | proposer -> coder -> reviewer -> gater |
| **Hierarchical** | Tree-structured delegation | Erlang supervision tree pattern |
| **Swarm** | Decentralized emergent | OpenAI Swarm SDK concept |
| **Mesh** | Direct peer-to-peer | Agent-to-agent Bus messaging |

### Supervision strategies (from Erlang/OTP)

```rust
pub enum SupervisionStrategy {
    /// Restart only the failed task with a fallback model.
    OneForOne { max_restarts: u32, within_ms: u64, fallback_tier: Option<ModelTier> },
    /// Re-run all tasks in the plan group.
    OneForAll { max_restarts: u32 },
    /// Re-run the failed task and all downstream dependents in the DAG.
    RestForOne { max_restarts: u32 },
}
```

---

## 8. Agent Introspection

Agents have five engineering introspection capabilities:

| Capability | Implementation | Unified primitive |
|---|---|---|
| State inspection | EpisodeLogger queries | Observe protocol (Lens) |
| Capability assessment | `ToolRegistry::all()` | Compose protocol |
| Confidence estimation | CascadeRouter signals | Score protocol |
| History review | `.roko/episodes.jsonl` | Store protocol (query) |
| Failure detection | `roko-conductor` watchers | React protocol |

### Metacognitive monitoring

A secondary monitoring layer watches the primary agent for failure signals (arXiv:2509.19783, 2025). Achieves +7.78pp success rate improvement. Maps to Roko's `roko-conductor` watcher/circuit-breaker pattern:

```rust
pub struct MetacognitiveConfig {
    pub max_stalled_turns: usize,       // Default: 5
    pub max_idle_ms: u64,               // Default: 120s
    pub repetition_threshold: usize,    // Default: 3 (same tool, similar args)
    pub confidence_floor: f64,          // Default: 0.3
}
```

Intervention options when failure is detected:

```rust
pub enum Intervention {
    EscalateModel(ModelTier),     // Try a higher-tier model
    HumanHandoff(String),         // Request human review
    Abort(String),                // Abort with failure reason
    InjectReflection(String),     // Self-reflection prompt injection
}
```

---

## 9. Mori-Diffs Reality

The current `PlanRunner` in `orchestrate.rs` does **not** use `MultiAgentPool` directly. It constructs agents on-demand and tracks them via the `ProcessSupervisor`. The pool types exist for the future state where all agent lifecycle is delegated to the pool layer:

```
Current:
  orchestrate.rs -> AgentRunConfig -> run_prepared_agent() -> ClaudeCliAgent

Future:
  orchestrate.rs -> MultiAgentPool.submit(role, task) -> pool handles:
    -> warm-pool promotion or cold-start construction
    -> create_agent_for_model() via provider adapter
    -> execution with timeout + cancellation
    -> fallback retry on failure
    -> lifecycle state tracking
    -> bulk kill on plan completion
```

The mori-diff notes the runner has no warm pool: every agent spawn is cold. For gate-then-retry cycles, pre-spawning a reviewer during gate execution would reduce latency significantly.

---

## What This Enables

- **Fine-grained capability control**: Each role constrains exactly what an agent can do, enforced at the ToolDispatcher level
- **Adaptive model routing**: Role tiers provide starting points that the CascadeRouter refines through learning
- **Cost control**: Per-role budgets with tier multipliers prevent runaway spending
- **Fast spawn**: Warm pools eliminate cold-start latency for frequently-used roles
- **Automatic fallback**: Failed agents retry with cheaper models before marking tasks as failed
- **Concurrent execution**: Per-role concurrency limits balance parallelism with cost

## Feedback Loops

1. **Role tier -> CascadeRouter -> Effective tier**: The role's default tier seeds the router. The router learns from outcomes and adjusts the effective tier. Over time, the router may promote cheap roles (if tasks succeed with Fast) or demote expensive ones (if Premium models don't outperform Standard).

2. **Budget -> Vitality -> Behavioral phase**: Budget consumption flows through vitality. As vitality drops (budget consumed), the agent enters Conservation and then Declining phases, which constrain available tools and model tier.

3. **Pool metrics -> Concurrency tuning**: Active/warm/queued counts per role feed into concurrency limit adjustments. Roles with consistently full queues get higher limits; roles with idle warm agents get lower limits.

4. **Fallback outcomes -> Router learning**: When a fallback model succeeds after the primary fails, the router records both the failure (primary) and success (fallback), updating its model preferences.

5. **Gate pass rate -> Role threshold tuning**: Each role has a `gate_pass_rate_floor` that triggers escalation when pass rates fall below it. The adaptive gate threshold system (`.roko/learn/gate-thresholds.json`) adjusts these floors based on historical performance.

## Open Questions

1. **Pool integration timeline**: The MultiAgentPool exists as a library but is not used by the orchestrator or runner. When should the migration happen, and should it be incremental (one role at a time) or wholesale?

2. **Warm pool TTL**: Pre-spawned Claude CLI agents consume a process slot. If the gate takes 30+ seconds, the warm agent is idle. Should TTL be adaptive (shorter when system load is high)?

3. **Role taxonomy completeness**: The 28-role taxonomy covers current needs but may need expansion for new domains (DeFi, data science, hardware). Should roles be extensible via config, or is the compile-time enum the right constraint?

4. **Capability-based security (OCaps)**: Research shows OCaps models (cryptographic warrants with attenuation) are strictly more expressive than RBAC for dynamic agent tasks. Should Roko migrate from role-based permissions to capability warrants?

---

## Citations

1. `crates/roko-core/src/agent.rs` -- AgentRole enum, AgentBackend, ModelTier, TurnBudget, ToolPermissions.
2. `crates/roko-agent/src/pool.rs` -- AgentPool, AgentInstanceId, InstanceStatus, TaskOutcome.
3. `crates/roko-agent/src/multi_pool.rs` -- MultiAgentPool, WarmEntry, concurrency control.
4. `crates/roko-agent/src/dispatcher/mod.rs:198` -- Permission enforcement in ToolDispatcher.
5. `crates/roko-compose/src/templates/` -- Role-specific prompt templates.
6. arXiv:2601.04748 (2025). "When Single-Agent with Skills Replace Multi-Agent Systems."
7. arXiv:2509.19783 (2025). "Agentic Metacognition: Self-Aware Agent for Failure Prediction."
8. arXiv:2410.15048 (2024). "MorphAgent: Self-Evolving Profiles and Decentralized Collaboration."
9. Hewitt, C., Bishop, P., & Steiger, R. (1973). "A Universal Modular ACTOR Formalism."
10. `tmp/mori-diffs/01-AGENT-DISPATCH.md` -- Warm pool and runner dispatch gaps.
