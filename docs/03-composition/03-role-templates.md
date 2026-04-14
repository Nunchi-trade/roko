# 03 — Role Templates: Per-Role Prompt Specialization

> Layer 2 Scaffold — Synapse Architecture
> Status: **Implemented** — `roko-compose::role_prompts` (462 lines) + `roko-compose::templates` (603 lines)
> Canonical source: `crates/roko-compose/src/role_prompts.rs`, `crates/roko-compose/src/templates/`


> **Implementation**: Shipping

---

## Abstract

Each agent role in Roko receives a specialized system prompt tailored to its task type. The role template system defines per-role identities, per-role token budgets, and per-role section emphasis. Twelve roles are currently defined: Strategist, Implementer, Architect, Auditor, QuickReviewer, Scribe, Critic, AutoFixer, IntegrationTester, Refactorer, Researcher, and Conductor. Each role receives a different allocation of the token budget, emphasizing the context types most critical to its function.

This document specifies the role catalog, the PromptBudget struct, the budget_for() allocation table, and the complexity-adaptive budget system.

---

## 1. The Role Catalog

### 1.1 Strategist

**Purpose:** Decomposes complex tasks into subtasks. Plans execution order and identifies dependencies.

**Identity emphasis:** Architectural thinking, decomposition, dependency analysis. The Strategist never writes code — it produces plans, decompositions, and task TOMLs.

**Budget emphasis:** Large workspace_map (20K) to see project structure. Large prd2 (12K) for specification context. Zero file_context — the Strategist plans but does not code.

### 1.2 Implementer

**Purpose:** Writes code to implement specified changes. The workhorse role.

**Identity emphasis:** Senior software engineer. Follows conventions. Writes tests. Handles edge cases. Checks compilation.

**Budget emphasis:** Largest file_context (8K) because it needs actual source code. Large workspace_map (20K) for navigation. Large brief (8K) for detailed task description.

### 1.3 Architect

**Purpose:** Reviews implementation for architectural quality, consistency with project patterns, and cross-crate impact.

**Identity emphasis:** Software architect. Evaluates design decisions. Checks interface contracts. Identifies coupling issues.

**Budget emphasis:** Moderate across all sections. Smaller workspace_map (6K) because reviews are focused. Moderate file_context (6K) for the code under review.

### 1.4 Auditor

**Purpose:** Security and correctness audit. Checks for OWASP top 10 vulnerabilities, unsafe code, resource leaks.

**Identity emphasis:** Security specialist. Paranoid by default. Checks every input boundary. Validates all assumptions.

**Budget emphasis:** Same as Architect. Smaller budgets because audits are narrowly scoped.

### 1.5 QuickReviewer

**Purpose:** Fast-turnaround code review for simple changes. Catches obvious issues.

**Identity emphasis:** Speed over depth. Focuses on obvious bugs, formatting, and convention violations. Does not evaluate architecture.

**Budget emphasis:** Minimal budgets across the board. Designed for low-token-cost reviews.

### 1.6 Scribe

**Purpose:** Technical documentation. Writes docstrings, README sections, architecture docs.

**Identity emphasis:** Technical writer. Accurate citations. Clear explanations. Follows documentation patterns.

**Budget emphasis:** Largest prd2 allocation (16K) because documentation must accurately cite specifications and academic references. Large file_context (6K) to see the code being documented.

### 1.7 Critic

**Purpose:** Devil's advocate. Challenges assumptions, finds edge cases, proposes failure scenarios.

**Identity emphasis:** Contrarian thinker. Questions every assumption. Looks for what could go wrong.

**Budget emphasis:** Moderate. Similar to Architect but with emphasis on anti-patterns.

### 1.8 AutoFixer

**Purpose:** Mechanical fix-up. Resolves compilation errors, lint warnings, and formatting issues.

**Identity emphasis:** Mechanical. Does not reason about design — applies fixes from error messages.

**Budget emphasis:** Minimal. Needs only the error output and the relevant file.

### 1.9 IntegrationTester

**Purpose:** Validates that changes work across system boundaries.

**Identity emphasis:** Tests cross-crate interactions. Checks public API contracts. Validates integration points.

**Budget emphasis:** Moderate workspace_map (to understand cross-crate relationships). Moderate file_context (for test files and interfaces).

### 1.10 Refactorer

**Purpose:** Restructures code without changing behavior.

**Identity emphasis:** Preserves behavior. Improves structure. Reduces duplication. Respects public API.

**Budget emphasis:** Large file_context (to see the code being refactored). Large workspace_map (to understand impact).

### 1.11 Researcher

**Purpose:** Conducts deep research on technical topics with citations.

**Identity emphasis:** Academic rigor. Finds and cites primary sources. Produces structured research artifacts.

**Budget emphasis:** Large prd2 (for existing research context). Moderate skills (for research methodologies).

### 1.12 Conductor

**Purpose:** Coordinates multi-agent plan execution. The meta-role.

**Identity emphasis:** Orchestration. Monitors progress. Resolves conflicts. Allocates tasks.

**Budget emphasis:** Large plan (to see the full execution plan). Moderate across other sections.

---

## 2. The PromptBudget Struct

Per-role token budgets are defined in the `PromptBudget` struct:

```rust
// crates/roko-compose/src/templates/common.rs

pub struct PromptBudget {
    /// Plan content (plan.md, tasks.toml).
    pub plan: usize,
    /// Workspace map (project structure overview).
    pub workspace_map: usize,
    /// PRD extract (specification sections relevant to this plan).
    pub prd2: usize,
    /// Cross-plan context (what other plans have done, shared registries).
    pub context: usize,
    /// Task brief (What/Why/How summary).
    pub brief: usize,
    /// Review feedback from prior reviews.
    pub reviews: usize,
    /// Role-specific instructions.
    pub instructions: usize,
    /// Relevant source file content.
    pub file_context: usize,
    /// Learned skills and playbook rules.
    pub skills: usize,
}
```

### 2.1 Budget Allocation Table

The `budget_for()` function returns the per-role budget:

```rust
// crates/roko-compose/src/templates/common.rs

pub const fn budget_for(role: AgentRole) -> PromptBudget {
    match role {
        AgentRole::Implementer => PromptBudget {
            plan: 50_000, workspace_map: 20_000, prd2: 12_000,
            context: 4_000, brief: 8_000, reviews: 3_000,
            instructions: 4_000, file_context: 8_000, skills: 8_000,
        },
        AgentRole::Strategist => PromptBudget {
            plan: 50_000, workspace_map: 20_000, prd2: 12_000,
            context: 4_000, brief: 6_000, reviews: 3_000,
            instructions: 4_000, file_context: 0, skills: 4_000,
        },
        AgentRole::Architect | AgentRole::Auditor => PromptBudget {
            plan: 50_000, workspace_map: 6_000, prd2: 6_000,
            context: 2_000, brief: 4_000, reviews: 3_000,
            instructions: 4_000, file_context: 6_000, skills: 4_000,
        },
        AgentRole::Scribe => PromptBudget {
            plan: 50_000, workspace_map: 6_000, prd2: 16_000,
            context: 4_000, brief: 6_000, reviews: 3_000,
            instructions: 4_000, file_context: 6_000, skills: 4_000,
        },
        _ => PromptBudget {
            plan: 50_000, workspace_map: 8_000, prd2: 6_000,
            context: 4_000, brief: 4_000, reviews: 2_000,
            instructions: 4_000, file_context: 6_000, skills: 4_000,
        },
    }
}
```

### 2.2 Key Budget Differences

| Section | Implementer | Strategist | Scribe | Default |
|---------|------------|------------|--------|---------|
| workspace_map | **20K** | **20K** | 6K | 8K |
| prd2 | 12K | 12K | **16K** | 6K |
| file_context | **8K** | **0** | 6K | 6K |
| brief | **8K** | 6K | 6K | 4K |
| skills | **8K** | 4K | 4K | 4K |

Key asymmetries:
- **Implementer gets the most file_context** (8K) because it writes code and needs to see existing signatures, patterns, and types.
- **Strategist gets zero file_context** because it plans but never writes code.
- **Scribe gets the most prd2** (16K) because documentation must accurately cite specifications and academic references.
- **Implementer gets the most skills** (8K) because learned playbook rules directly prevent repeated implementation mistakes.

---

## 3. Complexity-Adaptive Budgets

The base budgets are adjusted by task complexity through the `adjusted_budget_for()` function:

```rust
// crates/roko-compose/src/budget.rs

#[derive(Debug, Clone, Copy)]
pub enum Complexity {
    /// Two-line fix, rename, format change. ~4K total.
    Trivial,
    /// Standard implementation task. ~12K total.
    Standard,
    /// Cross-crate integration, architectural change. ~24K total.
    Complex,
}

pub struct AdjustedBudget {
    pub budget: PromptBudget,
    pub complexity: Complexity,
}

pub fn adjusted_budget_for(role: AgentRole, complexity: Complexity) -> AdjustedBudget {
    let base = budget_for(role);
    let adjusted = match complexity {
        Complexity::Trivial => PromptBudget {
            // Drop PRD, context, skills entirely.
            // Halve workspace_map and brief.
            prd2: 0,
            context: 0,
            skills: 0,
            workspace_map: base.workspace_map / 2,
            brief: base.brief / 2,
            ..base
        },
        Complexity::Standard => base,
        Complexity::Complex => PromptBudget {
            // Inflate workspace_map 50%, context 100%, file_context 50%.
            workspace_map: base.workspace_map * 3 / 2,
            context: base.context * 2,
            file_context: base.file_context * 3 / 2,
            ..base
        },
    };
    AdjustedBudget { budget: adjusted, complexity }
}
```

### 3.1 Trivial Tasks

For a two-line rename or format fix:
- **Drop** PRD extract (irrelevant to a mechanical fix)
- **Drop** cross-plan context (irrelevant)
- **Drop** skills/playbook (overkill)
- **Halve** workspace map (only need the target file's location)
- **Halve** brief (short description suffices)

Token savings: ~70% reduction vs. standard budget.

### 3.2 Complex Tasks

For cross-crate architectural changes:
- **50% more** workspace map (need to see broader project structure)
- **100% more** cross-plan context (need to know what other plans did)
- **50% more** file context (need to see more surrounding code)

### 3.3 Cache Break Points

The `adjusted_budget_for()` function also returns cache break positions for the complexity level:

| Complexity | Cache breaks at |
|-----------|----------------|
| Trivial | After conventions only (no workspace_map break) |
| Standard | After conventions, after workspace_map, after file_context |
| Complex | After conventions, after workspace_map, after file_context |

Fewer cache breaks for Trivial tasks means a shorter stable prefix, which is fine because Trivial tasks use cheap models (Haiku) where cache savings are less impactful.

---

## 4. Template Trait and Shared Stanzas

The `RolePromptTemplate` trait defines the contract for role templates:

```rust
// crates/roko-compose/src/templates/mod.rs

pub trait RolePromptTemplate {
    /// Return the structured sections for this role.
    fn sections(&self, context: &TemplateContext) -> Vec<PromptSection>;

    /// Return the role identity string.
    fn role_identity(&self) -> &str;
}
```

Shared stanzas used across multiple roles:

### CONTEXT_LAYOUT_STANZA

Instructs agents on where to find context files:

```
Read context/in/execution-pack.md for your main context.
Read context/in/brief.md for your task brief.
Read the narrowest artifacts first — only open broader context if needed.
```

### MCP_TOOLS_STANZA

MCP tool usage instructions:

```
You have access to MCP tools via the configured MCP servers.
Use tools as described in their schemas. Do not guess parameters.
Prefer MCP tools over shell commands when both are available.
```

### NITS_FORMAT

Output format specification for review roles:

```
Format your review as:
## Issues Found
- [severity] [file:line] Description
## Suggestions
- [priority] Description
```

---

## 5. Truncation Helpers

Two truncation helpers manage section content that exceeds its budget:

```rust
// crates/roko-compose/src/templates/mod.rs

/// Truncate from the end, preserving the beginning.
pub fn truncate(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars { return content.to_string(); }
    let truncated = &content[..max_chars.min(content.len())];
    format!("{}\n...(truncated)", truncated)
}

/// Truncate from the beginning, preserving the end.
pub fn truncate_tail(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars { return content.to_string(); }
    let start = content.len().saturating_sub(max_chars);
    format!("(truncated)...\n{}", &content[start..])
}
```

The choice of `truncate` vs. `truncate_tail` depends on the section:
- **workspace_map:** truncate from end (beginning has the most important crates)
- **gate_errors:** truncate from beginning / keep tail (most recent errors are most relevant)
- **file_context:** truncate from end (file headers and imports are most important)
- **prd_extract:** truncate from end (opening sections are most important)

---

## 6. The PlanSlice and TaskEnhancements Structs

Additional context structures passed to templates:

```rust
// crates/roko-compose/src/templates/mod.rs

pub struct PlanSlice {
    /// Plan content (plan.md).
    pub plan_content: String,
    /// Task TOML content.
    pub task_toml: String,
    /// Workspace map.
    pub workspace_map: String,
    /// PRD extract.
    pub prd_extract: String,
    /// Cross-plan context.
    pub cross_plan_context: String,
}

pub struct TaskEnhancements {
    /// Strategist brief.
    pub brief: Option<String>,
    /// Review feedback from prior attempts.
    pub reviews: Vec<String>,
    /// Iteration memory (what was tried before).
    pub iteration_memory: Option<String>,
    /// Research artifacts.
    pub research: Option<String>,
    /// Playbook rules matching this task.
    pub playbook_rules: Vec<String>,
    /// File content for target files.
    pub file_context: Vec<(String, String)>,
}
```

---

## 7. Role-to-Context-Tier Mapping

Roles map to context tiers that determine the default model and token budget:

| Role | Default Context Tier | Default Model | Rationale |
|------|---------------------|---------------|-----------|
| Strategist | Full | Opus | Strategic planning needs maximum context |
| Implementer | Focused | Sonnet | Implementation needs focused, not exhaustive context |
| Architect | Focused | Sonnet | Reviews are focused operations |
| Auditor | Focused | Sonnet | Audits are narrowly scoped |
| QuickReviewer | Surgical | Haiku | Fast, cheap reviews |
| Scribe | Focused | Sonnet | Documentation needs moderate context |
| Critic | Focused | Sonnet | Critiques need moderate context |
| AutoFixer | Surgical | Haiku | Mechanical fixes need minimal context |
| IntegrationTester | Focused | Sonnet | Integration testing needs moderate context |
| Refactorer | Focused | Sonnet | Refactoring needs focused context |
| Researcher | Full | Opus | Research needs maximum context |
| Conductor | Full | Opus | Coordination needs full plan visibility |

This mapping is the default; the CascadeRouter (from roko-learn) may override it based on historical performance data.

---

## 8. Empirical Budget Analysis

From prompt-logs analysis during Mori development:

| Section | Avg Tokens | % of Prompt | Pass Rate When Present |
|---------|-----------|-------------|----------------------|
| Learning Pack | 2,347 | 49% | 61% |
| PRD2 Context | 712 | 15% | 67% |
| Strategist Brief | 491 | 10% | **72%** |
| Workspace Map | 334 | 7% | 64% |
| Execution Strategy | 298 | 6% | 58% |
| Cross-Plan Context | 243 | 5% | 55% |
| Your Assignment | 189 | 4% | **71%** |
| MCP Tools | 78 | 2% | 65% |
| Self-Review | 78 | 2% | 63% |

Key findings:
- **Strategist Brief and Your Assignment** have the highest pass rates (72%, 71%) at the lowest token costs. These are the highest-value-per-token sections.
- **Learning Pack** dominates at 49% of tokens but has the lowest pass rate (61%). It may be adding noise.
- **Cross-Plan Context** has the lowest pass rate (55%) and may actively hurt simple tasks by injecting irrelevant information.

These findings motivated the complexity-adaptive budget system: Trivial tasks drop Learning Pack and Cross-Plan Context entirely, focusing on the high-value-per-token sections.

---

## 9. Academic Foundations

**The --bare Flag Experiment** (Mori development, 2025-2026). The 3-4× quality gap between bare and prompted agents (15-25% vs. 60-75% success) validates that role-specific system prompts are the highest-leverage scaffold investment.

**ETH Zurich AGENTS.md Study**. Unnecessary instructions decrease agent success by ~3% and increase token costs by 20%+. This finding motivates per-role budgets: only include the sections each role actually needs. The Strategist's zero file_context is a direct application of this principle.

**Meta-Harness** [Lee et al. 2026, arXiv:2603.28052]. Evaluating coding agents across scaffolds showed a 6× performance gap from scaffold changes alone, while using 4× fewer input tokens. Different roles benefit from different scaffold configurations, justifying the per-role template system.

---

## 10. Current Status and Gaps

| Aspect | Status |
|--------|--------|
| 12 role templates | **Implemented** |
| PromptBudget per role | **Implemented** |
| Complexity-adaptive budgets | **Implemented** |
| Truncation helpers | **Implemented** |
| Shared stanzas | **Implemented** |
| Wired into orchestrate.rs | **Implemented** |
| Learned budget optimization (DSPy) | **Not yet** |
| A/B testing framework for sections | **Scaffold** (ExperimentStore exists) |
| Per-role pass rate tracking | **Implemented** (via efficiency events) |

---

## Cross-References

- [02-system-prompt-builder-7-layer.md](02-system-prompt-builder-7-layer.md) — SystemPromptBuilder layers
- [04-enrichment-pipeline-13-step.md](04-enrichment-pipeline-13-step.md) — Enrichment artifacts that become sections
- [05-token-budget-management.md](05-token-budget-management.md) — Budget allocation details
- `crates/roko-compose/src/role_prompts.rs` — Role prompt spec
- `crates/roko-compose/src/templates/common.rs` — Budget table
- `crates/roko-compose/src/budget.rs` — Complexity-adaptive budgets
