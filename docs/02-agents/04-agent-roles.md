# 04 — Agent Roles

> Sub-doc 04 of **02-agents** · Roko Documentation
>
> This document defines Roko's 28-role agent taxonomy, the per-role defaults
> (backend, model tier, budget, permissions), and how roles compose into
> agent types for different task categories.

---

## Overview

Every agent in Roko is assigned a **role** — a named persona that determines
what the agent can do, which model tier it defaults to, how much it can spend,
and what tools it can access. Roles are defined by the `AgentRole` enum in
`crates/roko-core/src/agent.rs`.

Roles serve three purposes:

1. **Capability scoping** — A `Reviewer` gets read-only tool permissions; an
   `Implementer` gets read + write + exec. This is enforced by the
   `ToolDispatcher`'s permission check.
2. **Model routing** — Each role has a default `ModelTier` (Fast/Standard/Premium)
   that the CascadeRouter uses as a starting point before learning adjusts it.
3. **Budget control** — Each role has a per-turn dollar ceiling (`TurnBudget`)
   that prevents runaway spending.

---

## The 28 Roles

The roles are organized by responsibility group:

### Planning roles

| Role | Tier | Budget | Permissions | Purpose |
|---|---|---|---|---|
| `Architect` | Premium | $3.00 | Read | System-level design decisions |
| `Planner` | Standard | $1.00 | Read | Task decomposition and DAG construction |
| `Researcher` | Standard | $1.50 | Read + Network | Deep research with citations |

### Implementation roles

| Role | Tier | Budget | Permissions | Purpose |
|---|---|---|---|---|
| `Implementer` | Standard | $1.50 | Read + Write + Exec | Primary coding agent |
| `Debugger` | Standard | $1.50 | Read + Write + Exec | Bug diagnosis and fix |
| `Optimizer` | Standard | $1.50 | Read + Write | Performance improvements |
| `Migrator` | Standard | $1.00 | Read + Write | Code migration / refactoring |

### Review roles

| Role | Tier | Budget | Permissions | Purpose |
|---|---|---|---|---|
| `Reviewer` | Standard | $0.75 | Read | Code review and feedback |
| `Auditor` | Premium | $2.00 | Read | Security and compliance audit |

### Validation roles

| Role | Tier | Budget | Permissions | Purpose |
|---|---|---|---|---|
| `Tester` | Standard | $1.00 | Read + Write + Exec | Test creation and execution |
| `Validator` | Fast | $0.50 | Read + Exec | Lightweight validation checks |
| `GateKeeper` | Fast | $0.30 | Read | Gate pipeline runner |

### Orchestration roles

| Role | Tier | Budget | Permissions | Purpose |
|---|---|---|---|---|
| `Conductor` | Fast | $0.10 | Read | Meta-orchestration (model routing) |
| `Coordinator` | Fast | $0.15 | Read | Task dependency management |
| `Monitor` | Fast | $0.10 | Read | Health monitoring and alerts |

### Specialized roles

| Role | Tier | Budget | Permissions | Purpose |
|---|---|---|---|---|
| `DocWriter` | Standard | $1.00 | Read + Write | Documentation generation |
| `Translator` | Standard | $0.75 | Read + Write | Format translation |
| `Analyst` | Standard | $1.00 | Read + Network | Data analysis |
| `Explorer` | Standard | $0.75 | Read + Network | Codebase exploration |

### Operations roles

| Role | Tier | Budget | Permissions | Purpose |
|---|---|---|---|---|
| `Deployer` | Standard | $0.50 | Read + Exec | Deployment automation |
| `Operator` | Fast | $0.30 | Read + Exec | Runtime operations |

### Additional roles complete the taxonomy to 28 total, with further roles
for chain operations, learning feedback, and cross-domain composition.

---

## Role Defaults

Each role carries four defaults defined via associated methods on `AgentRole`:

### Backend inference

```rust
impl AgentRole {
    pub fn backend(&self) -> AgentBackend {
        match self {
            Self::Conductor | Self::Monitor | Self::Validator
                => AgentBackend::Claude,
            Self::Implementer | Self::Debugger | Self::Tester
                => AgentBackend::Claude,
            _ => AgentBackend::Claude,
        }
    }
}
```

Currently all roles default to the Claude backend. The `AgentBackend::from_model`
heuristic overrides this when a specific model slug is configured for a role
in `roko.toml`:

```toml
[agent.roles.implementer]
model = "claude-opus-4-6"    # → Claude backend

[agent.roles.conductor]
model = "claude-haiku-4-5"   # → Claude backend (fast tier)

[agent.roles.researcher]
model = "sonar-pro"          # → OpenAI-compat backend (Perplexity)
```

### Model tier

```rust
pub enum ModelTier {
    Fast,      // Haiku-class: classification, watchers, orchestration
    Standard,  // Sonnet-class: implementation, review (the workhorse)
    Premium,   // Opus/GPT-5-class: architecture, hard debugging
}
```

The tier is a hint to the CascadeRouter's starting point. As the LinUCB
bandit learns which models succeed for which tasks, it may promote or demote
a role's effective tier. The defaults are chosen conservatively:
implementation tasks start at Standard to balance cost and quality, while
orchestration overhead (Conductor, Monitor) starts at Fast to minimize cost.

### Turn budget

The `TurnBudget` struct caps per-turn spending:

```rust
pub struct TurnBudget {
    pub base_usd: f32,
    pub multiplier: f32,
}
```

The `multiplier` adjusts for model escalation — when the CascadeRouter
escalates from Sonnet to Opus, the budget is multiplied by 2.0x to account
for the higher per-token cost. When it de-escalates to Haiku, the multiplier
drops to 0.6x.

The budget table is derived from the Mori agent roles specification
(`bardo-backup/tmp/mori-agents/03-agent-roles`), adjusted for current model
pricing.

### Tool permissions

Each role declares a `ToolPermission` that is checked by the `ToolDispatcher`
at step 4 (authorize). The permission flags are:

```rust
pub struct ToolPermissions {
    pub read: bool,     // File read, grep, glob
    pub write: bool,    // File write, edit, patch
    pub exec: bool,     // Bash, run_tests
    pub git: bool,      // Git operations
    pub network: bool,  // Web fetch, web search
}
```

The `ToolDispatcher` (at `crates/roko-agent/src/dispatcher/mod.rs:198`)
checks `def.permission.satisfied_by(&role_perms)` before allowing any tool
call to proceed. This means a `Reviewer` role cannot write files, even if
the model requests a `write_file` tool call — the dispatcher blocks it with
`ToolError::PermissionDenied`.

---

## Role Composition: Agent Types

The refactoring PRD §05-agent-types defines five composite agent types, each
built from a combination of roles:

### 1. Coding Agent

Roles: `Implementer` + `Reviewer` + `Tester`

The standard development cycle. The Implementer writes code, the Reviewer
checks it, and the Tester validates. In practice, a single agent run often
combines all three capabilities with different system prompt layers.

### 2. Research Agent

Roles: `Researcher` + `Analyst` + `Explorer`

Deep investigation with citations. The Research agent has network access for
web search and can explore the codebase with read-only tools.

### 3. Operations Agent

Roles: `Deployer` + `Operator` + `Monitor`

Deployment and runtime management. Has exec permissions but limited write
permissions — it can run commands but shouldn't be editing source code.

### 4. Cross-Domain Agent

Roles: `Architect` + `Coordinator` + multiple domain specialists

Multi-domain tasks that span crate boundaries. The Architect provides
system-level context, the Coordinator manages dependencies, and domain
specialists handle implementation in their respective areas.

### 5. Chain Agent

Roles: (Future) chain-specific roles for multi-agent collaboration.

Tracked as a Phase 2+ capability in `roko-golem`.

---

## Role-Specific System Prompts

The `SystemPromptBuilder` in `roko-compose` constructs 6-layer prompts
where the role determines Layer 2 (the role-specific context):

```
Layer 0: Global context (project name, codebase structure)
Layer 1: Task context (plan, task description, dependencies)
Layer 2: Role context (role-specific instructions and constraints)
Layer 3: Tool context (available tools and their descriptions)
Layer 4: History context (relevant previous outputs)
Layer 5: Meta context (budget remaining, time constraints)
```

The role layer is populated from templates in
`crates/roko-compose/src/templates/` — each role has a template that
describes its persona, constraints, and expected output format.

Reference: `RoleSystemPromptSpec` in `orchestrate.rs` uses the
6-layer builder with templates. See the Mori parity checklist for the
1,253-item comparison between Mori's ~2K-token role prompts and Roko's
current implementations.

---

## Configuration Override

Users can override any role default in `roko.toml`:

```toml
[agent.roles.implementer]
model = "claude-opus-4-6"
timeout_ms = 300000
budget_usd = 5.00

[agent.roles.conductor]
model = "claude-haiku-4-5"
budget_usd = 0.05
```

The override hierarchy is:
1. Per-task configuration in `tasks.toml` (highest priority)
2. Per-role configuration in `roko.toml`
3. Role defaults in `AgentRole` (lowest priority)

---

## Citations

1. `crates/roko-core/src/agent.rs` — AgentRole enum, AgentBackend, ModelTier,
   TurnBudget, ToolPermissions.
2. Refactoring PRD §05-agent-types — Agent role compositions, extensibility.
3. Refactoring PRD §02-five-layers — Dual-Process Tier Router, Temperament
   Profiling table.
4. `bardo-backup/tmp/mori-agents/03-agent-roles` — Original budget table.
5. `crates/roko-compose/src/templates/` — Role-specific prompt templates.
6. `crates/roko-agent/src/dispatcher/mod.rs:198` — Permission enforcement.
