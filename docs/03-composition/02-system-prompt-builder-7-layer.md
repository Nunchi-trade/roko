# 02 — SystemPromptBuilder: 7-Layer Prompt Assembly

> Layer 2 Scaffold — Synapse Architecture
> Status: **Implemented** — `roko-compose::system_prompt_builder` (726 lines, 12 tests)
> Canonical source: `crates/roko-compose/src/system_prompt_builder.rs`

---

## Abstract

The SystemPromptBuilder constructs agent system prompts through a 7-layer architecture that separates stable identity (role, conventions) from volatile context (task, affect). Each layer has a defined purpose, cache tier, and injection point. The builder produces both a flat string (`build()`) and structured sections (`build_sections()`) for use by the PromptComposer's budget-fitting algorithm. Cache alignment markers between tiers enable the inference gateway to place KV-cache breakpoints for maximum prefix reuse.

This document specifies the 7 layers, the builder API, cache alignment strategy, affect-guidance injection, and the wiring into the orchestration pipeline.

---

## 1. The 7 Layers

The SystemPromptBuilder assembles system prompts in seven ordered layers. Each layer has a defined scope, cache tier, and content source:

| Layer | Name | Cache Tier | Content Source | Purpose |
|-------|------|-----------|----------------|---------|
| 1 | Role Identity | System | `role_prompts.rs` | Who the agent is, what it specializes in |
| 2 | Conventions | System | CLAUDE.md / project config | Project patterns, style rules, safety constraints |
| 3a | Domain Context | Session | PRD extracts, workspace map | Domain-specific knowledge for this project |
| 3b | Relevant Context | Session | Knowledge store, enrichment artifacts | Task-relevant retrieved context |
| 4 | Task Context | Task | Task TOML, brief, gate errors | What the agent should do right now |
| 5 | Tool Instructions | Task | Tool definitions, MCP config | What tools are available and how to use them |
| 6 | Anti-Patterns | Dynamic | Playbook rules, failure history | What mistakes to avoid |
| 7 | Affect Guidance | Dynamic | Daimon PAD state | Emotional/motivational modulation |

### Layer 1: Role Identity

The foundation layer. Defines the agent's role, expertise, and behavioral style. Each role has a distinct identity:

```rust
// From role_prompts.rs — example identity fragments

Strategist:     "You are a technical strategist who decomposes complex tasks..."
Implementer:    "You are a senior software engineer implementing changes..."
Architect:      "You are a software architect reviewing implementation quality..."
Auditor:        "You are a security and correctness auditor..."
QuickReviewer:  "You are a fast-turnaround code reviewer..."
Scribe:         "You are a technical writer documenting implementations..."
Critic:         "You are a devil's advocate who challenges assumptions..."
AutoFixer:      "You fix compilation and lint errors mechanically..."
IntegrationTester: "You validate that changes work across system boundaries..."
Refactorer:     "You restructure code without changing behavior..."
Researcher:     "You conduct deep research on technical topics..."
Conductor:      "You coordinate multi-agent plan execution..."
```

Role identity is placed in the System cache tier because it is identical across all tasks for the same role. A 20-plan run with 40 Implementer spawns hits the cache on 39 of them.

### Layer 2: Conventions

Project-level rules and constraints. Loaded from `CLAUDE.md`, `roko.toml`, and project configuration. Contains:

- Coding style rules (naming conventions, error handling patterns)
- Safety constraints (never push to main, never delete without confirmation)
- Project-specific patterns (how imports are organized, how tests are structured)
- Architecture rules (which crates depend on which, public API surface)

Conventions are System-tier because they do not change between tasks.

### Layer 3a: Domain Context

Project-specific knowledge that changes across sessions but not across tasks within a session:

- PRD extracts relevant to the current plan
- Workspace map showing project structure
- Cross-plan context (what other plans have done, shared type registries)

### Layer 3b: Relevant Context

Task-relevant retrieved context from the knowledge store and enrichment pipeline:

- Knowledge entries matching the task description (via HDC similarity or keyword search)
- Episode summaries from similar past tasks
- Enrichment artifacts (research memos, dependency manifests)

This sub-layer is separate from 3a because its content is task-specific, while 3a is session-level. The split allows 3a to be cached at the Session tier while 3b is Task-tier.

### Layer 4: Task Context

The specific task the agent should perform:

- Task TOML (description, files to modify, acceptance criteria)
- Task brief (What/Why/How summary from the enrichment pipeline)
- Gate errors from previous attempts (for iteration 2+)
- Iteration memory (what was tried before and why it failed)

Task context is the most volatile layer that is still task-specific (as opposed to Dynamic, which changes per turn within a task).

### Layer 5: Tool Instructions

Available tools and how to use them:

- Tool definitions (sorted alphabetically for cache stability)
- MCP server configuration
- Tool-specific instructions (e.g., "prefer using Read over cat")
- Tool restrictions (e.g., "never use --force")

### Layer 6: Anti-Patterns

Known failure modes and explicit prohibitions:

- Playbook rules that match the current task's file paths and crates
- Common mistakes from the episode history
- Anti-knowledge entries (things that are explicitly wrong or dangerous)

Anti-patterns are Dynamic-tier because they may change as new failures are recorded, even within a single task's iterations.

### Layer 7: Affect Guidance

Motivational modulation based on the Daimon's PAD (Pleasure-Arousal-Dominance) state:

```rust
// From system_prompt_builder.rs

// High arousal (≥ 0.35): time pressure
"You are under time pressure. Focus on the most impactful changes first.
Avoid over-engineering. Prefer simple, correct solutions over elegant ones."

// Low arousal (≤ -0.35): exploration
"You have time to explore. Consider multiple approaches before committing.
Read surrounding code carefully. Look for patterns you can reuse."

// Low pleasure (≤ -0.35): caution after failures
"Recent attempts have had issues. Be extra careful with your changes.
Double-check your work against the acceptance criteria before finishing."
```

Affect guidance is the most volatile layer — it changes with every PAD state update.

---

## 2. Builder API

The SystemPromptBuilder uses a fluent builder pattern:

```rust
// crates/roko-compose/src/system_prompt_builder.rs

pub struct SystemPromptBuilder {
    role_identity: String,
    conventions: String,
    domain_context: String,
    relevant_context: String,
    task_context: String,
    tool_instructions: String,
    anti_patterns: String,
    affect_guidance: String,
}

impl SystemPromptBuilder {
    pub fn new() -> Self { ... }
    pub fn role_identity(mut self, content: &str) -> Self { ... }
    pub fn conventions(mut self, content: &str) -> Self { ... }
    pub fn domain_context(mut self, content: &str) -> Self { ... }
    pub fn relevant_context(mut self, content: &str) -> Self { ... }
    pub fn task_context(mut self, content: &str) -> Self { ... }
    pub fn tool_instructions(mut self, content: &str) -> Self { ... }
    pub fn anti_patterns(mut self, content: &str) -> Self { ... }
    pub fn affect_guidance(mut self, content: &str) -> Self { ... }

    /// Build as a single concatenated string with layer markers.
    pub fn build(&self) -> String { ... }

    /// Build as structured PromptSections for budget fitting.
    pub fn build_sections(&self) -> Vec<PromptSection> { ... }
}
```

The `build()` method produces a flat string with cache alignment markers:

```xml
<!-- roko:layer:system -->
{Layer 1: Role Identity}

{Layer 2: Conventions}

<!-- roko:layer:session -->
{Layer 3a: Domain Context}

{Layer 3b: Relevant Context}

<!-- roko:layer:task -->
{Layer 4: Task Context}

{Layer 5: Tool Instructions}

<!-- roko:layer:dynamic -->
{Layer 6: Anti-Patterns}

{Layer 7: Affect Guidance}
```

The `build_sections()` method produces structured `PromptSection` objects that the PromptComposer can individually score, prioritize, and budget-fit:

```rust
vec![
    PromptSection {
        name: "role_identity".into(),
        content: self.role_identity.clone(),
        priority: SectionPriority::Critical,
        cache_layer: CacheLayer::System,
        placement: Placement::Start,
        hard_cap: None,
    },
    PromptSection {
        name: "conventions".into(),
        content: self.conventions.clone(),
        priority: SectionPriority::Critical,
        cache_layer: CacheLayer::System,
        placement: Placement::Start,
        hard_cap: None,
    },
    // ... remaining layers
]
```

---

## 3. Cache Alignment Strategy

Cache alignment is the highest-leverage cost optimization in the entire scaffold. The goal: maximize the byte-identical prefix across requests.

### 3.1 Prefix Tiers

```
Tier 1 (System): Role Identity + Conventions
  → Identical across ALL tasks for this role
  → Cache hit on every request after the first
  → 90% discount (Anthropic), 50% (OpenAI)

Tier 2 (Session): Domain Context
  → Identical across all tasks in the same plan
  → Cache hit on all tasks within a plan run

Tier 3 (Task): Task Context + Tools
  → Identical across iterations of the same task
  → Cache hit on retry attempts

Tier 4 (Dynamic): Anti-Patterns + Affect
  → Unique per turn
  → No cache benefit
```

### 3.2 Rules for Cache Stability

1. **Never randomize section ordering.** Deterministic priority sort only.
2. **Normalize whitespace.** Strip trailing spaces, normalize newlines to `\n`.
3. **Sort tool definitions alphabetically.** Use BTreeMap, not HashMap.
4. **Freeze workspace map within a plan execution.** Generate once, reuse for all tasks.
5. **Emit explicit layer markers.** The inference gateway places `cache_control` breakpoints at these markers.

### 3.3 Cost Impact

For a typical 20-plan run with 80 agent spawns:

| Without cache alignment | With cache alignment |
|------------------------|---------------------|
| ~$100 on Opus (20M tokens) | ~$19 on Opus |
| Every request pays full price | 90% discount on prefix layers |
| Tool definition order varies | Deterministic ordering |

---

## 4. Wiring into Orchestration

The SystemPromptBuilder is wired into the orchestration pipeline through `RoleSystemPromptSpec`:

```rust
// crates/roko-compose/src/role_prompts.rs

pub struct RoleSystemPromptSpec {
    pub role: AgentRole,
    pub builder: SystemPromptBuilder,
}

impl RoleSystemPromptSpec {
    /// Build with context-window-aware budget fitting.
    pub fn build_with_context_window(
        &self,
        context_window: usize,
    ) -> String {
        // Apply soft and hard limits based on context window
        // Soft limit: target 60% of context window for system prompt
        // Hard limit: never exceed 80% of context window
        // Reserve 20% minimum for conversation turns
    }

    /// Compose with explicit budget constraint.
    pub fn compose_with_budget(
        &self,
        budget: &PromptBudget,
    ) -> String {
        // Apply per-section caps from PromptBudget
        // Truncate sections that exceed their allocation
        // Return assembled system prompt
    }
}
```

In `roko-cli/src/orchestrate.rs`, the orchestrator builds the system prompt for each agent spawn:

```rust
let spec = RoleSystemPromptSpec::for_role(task.role)
    .with_conventions(&conventions)
    .with_domain_context(&workspace_map, &prd_extract)
    .with_task_context(&task_toml, &brief, &gate_errors)
    .with_tools(&tool_defs)
    .with_anti_patterns(&playbook_rules)
    .with_affect(&daimon_state);

let system_prompt = spec.build_with_context_window(model_context_window);
```

---

## 5. Affect Guidance Details

The affect guidance layer translates the Daimon's PAD vector into natural language instructions that modulate agent behavior.

### 5.1 Arousal Dimension

| Arousal Level | Guidance | Behavioral Effect |
|--------------|----------|-------------------|
| High (≥ 0.35) | "You are under time pressure..." | Focus on impact, avoid over-engineering |
| Neutral | (no guidance) | Default behavior |
| Low (≤ -0.35) | "You have time to explore..." | Thorough investigation, multiple approaches |

### 5.2 Pleasure Dimension

| Pleasure Level | Guidance | Behavioral Effect |
|---------------|----------|-------------------|
| Low (≤ -0.35) | "Recent attempts have had issues..." | Extra caution, double-check work |
| Neutral/High | (no guidance) | Default confidence |

### 5.3 Dominance Dimension

Reserved for future use. Planned mapping: low dominance → seek confirmation before acting, high dominance → act autonomously.

### 5.4 Research Basis

The PAD (Pleasure-Arousal-Dominance) model was established by Mehrabian (1996) as a three-dimensional emotional space. Unlike basic sentiment (positive/negative), PAD captures motivational state: arousal determines urgency, dominance determines autonomy, pleasure determines risk tolerance. The Daimon's PAD vector is updated by appraisal triggers (gate success/failure, time pressure, task novelty) and decays toward neutral over time.

---

## 6. The --bare Flag Experiment

Empirical evidence for the value of system prompts comes from the `--bare` flag experiment conducted during Mori development (2025-2026):

| Condition | Task Success Rate |
|-----------|------------------|
| `claude --bare` (no system prompt) | 15-25% |
| `claude` (with system prompt) | 60-75% |

A 3-4× quality gap from the system prompt alone. The ETH Zurich AGENTS.md study quantified a complementary finding: unnecessary instructions in the system prompt decrease agent success by approximately 3% and increase token costs by 20% or more.

These findings combine into the scaffold's central design challenge: system prompts matter enormously (3-4× quality gap), AND they must be task-specific (3% penalty per irrelevant instruction). The SystemPromptBuilder addresses this by constructing minimal, maximally effective prompts for each specific task through the 7-layer architecture. Generic instructions go in Layer 2 (conventions, shared across all tasks). Task-specific instructions go in Layer 4 (unique per task). The builder includes only the layers that are relevant, avoiding the penalty for irrelevant content.

---

## 7. Layer Budget Allocation

Each layer has a default budget share, adjustable by role:

| Layer | Default Budget Share | Implementer | Strategist | Scribe |
|-------|---------------------|-------------|------------|--------|
| 1. Role Identity | 5% | 5% | 5% | 5% |
| 2. Conventions | 8% | 8% | 8% | 8% |
| 3a. Domain Context | 15% | 20% | 20% | 10% |
| 3b. Relevant Context | 10% | 15% | 5% | 10% |
| 4. Task Context | 30% | 35% | 25% | 25% |
| 5. Tool Instructions | 12% | 12% | 12% | 12% |
| 6. Anti-Patterns | 10% | 5% | 15% | 10% |
| 7. Affect Guidance | 2% | 2% | 2% | 2% |
| *Reserve (conversation turns)* | 8% | 8% | 8% | 18% |

The Implementer gets the largest Task Context share because it needs detailed code context. The Strategist gets the largest Anti-Patterns share because strategic errors are more costly. The Scribe gets the largest reserve because documentation tasks often require extensive back-and-forth.

---

## 8. Academic Foundations

**Plan-and-Solve Prompting** [Wang et al. 2023]. Improved zero-shot reasoning by splitting into two phases: devise a plan, then execute subtasks sequentially. The Strategist role's Layer 4 content embodies this: the task context includes the decomposition step that breaks down complex tasks before implementation.

**ReAct: Reasoning + Acting** [Yao et al. 2022]. Interleaving reasoning traces with task-specific actions produces better results than either alone. The 7-layer prompt structure supports ReAct by placing reasoning instructions (Layer 1 role identity, Layer 6 anti-patterns) alongside action instructions (Layer 4 task context, Layer 5 tools).

**Reflexion** [Shinn et al. 2023]. Verbal reinforcement learning: agents reflect on failures and use reflections to improve. Gate errors and iteration memory in Layer 4 are the Reflexion mechanism — they inject structured reflections from prior attempts into the next attempt's context.

**Step-Back Prompting** [Zheng et al. 2023]. Asking the model to abstract before solving improves reasoning by 7-27%. The Strategist's Layer 1 identity explicitly instructs abstraction before decomposition: "step back from the implementation details, reason about the architectural intent, then decompose."

---

## 9. Test Coverage

12 tests in `crates/roko-compose/src/system_prompt_builder.rs`:

- Layer ordering: layers appear in correct sequence
- Empty layers: skipped without placeholder text
- Cache markers: transition markers emitted at layer boundaries
- Affect guidance: arousal/pleasure thresholds produce correct guidance text
- Build vs build_sections: both methods produce consistent content
- Role identity: each role produces distinct identity text
- Budget fitting: sections are truncated to budget when build_with_context_window is used

---

## 10. Current Status and Gaps

| Aspect | Status |
|--------|--------|
| 7-layer builder | **Implemented** |
| Cache alignment markers | **Implemented** |
| Affect guidance (arousal, pleasure) | **Implemented** |
| Role-specific budgets | **Implemented** |
| Wired into orchestrate.rs | **Implemented** |
| 12 unit tests | **Passing** |
| Dominance affect guidance | **Not yet** |
| Dynamic anti-patterns from knowledge store | **Scaffold** |
| Learned budget allocation (DSPy-style) | **Not yet** |

---

## Cross-References

- [00-composer-trait.md](00-composer-trait.md) — Composer trait definition
- [03-role-templates.md](03-role-templates.md) — Role template details
- [05-token-budget-management.md](05-token-budget-management.md) — Budget allocation
- [12-affect-modulated-retrieval.md](12-affect-modulated-retrieval.md) — PAD-based modulation
- `crates/roko-compose/src/system_prompt_builder.rs` — Implementation source
- `crates/roko-compose/src/role_prompts.rs` — Role-specific wiring
