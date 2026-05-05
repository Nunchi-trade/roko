# 02 — SystemPromptBuilder: 9-Layer Prompt Assembly

> Layer 2 Scaffold — Synapse Architecture
> Status: **Implemented** — `roko-compose::system_prompt_builder` (726 lines, 12 tests)
> Canonical source: `crates/roko-compose/src/system_prompt_builder.rs`


> **Implementation**: Shipping

---

## Abstract

The SystemPromptBuilder constructs agent system prompts through a 9-layer architecture that separates stable identity (role, conventions) from volatile context (task, affect). Each layer has a defined purpose, cache tier, and injection point. The builder produces both a flat string (`build()`) and structured sections (`build_sections()`) for use by the PromptComposer's budget-fitting algorithm. Cache alignment markers between tiers enable the inference gateway to place KV-cache breakpoints for maximum prefix reuse.

This document specifies the 9 layers, the builder API, cache alignment strategy, affect-guidance injection, and the wiring into the orchestration pipeline.

---

## 1. The 9 Layers

The SystemPromptBuilder assembles system prompts in nine ordered layers. Each layer has a defined scope, cache tier, and content source:

| Layer | Name | Cache Tier | Content Source | Purpose |
|-------|------|-----------|----------------|---------|
| 1 | Role Identity | System | `role_prompts.rs` | Who the agent is, what it specializes in |
| 2 | Conventions | System | CLAUDE.md / project config | Project patterns, style rules, safety constraints |
| 3a | Domain Context | Session | PRD extracts, workspace map | Domain-specific knowledge for this project |
| 3b | Assembled Context | Session | Knowledge store, enrichment artifacts | Task-relevant retrieved context |
| 3c | Pheromone Signals | Session | Stigmergic signals, active context | Active environmental signals guiding behavior |
| 4 | Task Context | Task | Task TOML, brief, gate errors | What the agent should do right now |
| 5 | Tool Instructions | System | Tool definitions, MCP config | What tools are available and how to use them |
| 6a | Relevant Techniques | Task | Playbook rules, learned skills | Learned techniques to prefer for this task |
| 6b | Anti-Patterns | Task | Failure history, anti-knowledge | What mistakes to avoid |
| 7 | (reserved) | -- | -- | -- |
| 8 | Affect Guidance | Dynamic | Daimon PAD state | Emotional/motivational modulation |

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

### Layer 3b: Assembled Context

Task-relevant retrieved context from the knowledge store and enrichment pipeline:

- Knowledge entries matching the task description (via HDC similarity or keyword search)
- Episode summaries from similar past tasks
- Enrichment artifacts (research memos, dependency manifests)

This sub-layer is separate from 3a because its content is task-specific, while 3a is session-level.

### Layer 3c: Pheromone Signals

Active environmental signals that guide agent behavior through stigmergy:

- Recent engrams from the current plan that signal progress or blockers
- Inter-agent coordination signals (e.g., "crate X was just modified")
- Environment state indicators (build status, test results, resource usage)

Pheromone signals enable indirect coordination between agents without explicit messaging.

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

### Layer 6a: Relevant Techniques

Learned skills and playbook sequences that match the current task:

- Playbook rules that match the current task's file paths and crates
- Skill library entries relevant to the task type
- Reusable task sequences from prior successful plans

### Layer 6b: Anti-Patterns

Known failure modes and explicit prohibitions:

- Common mistakes from the episode history
- Anti-knowledge entries (things that are explicitly wrong or dangerous)
- Gate failure patterns from similar tasks

Anti-patterns are Task-tier because they may change as new failures are recorded across task iterations.

### Layer 8: Affect Guidance

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

{Layer 5: Tool Instructions}

<!-- roko:layer:session -->
{Layer 3a: Domain Context}

{Layer 3b: Assembled Context}

{Layer 3c: Pheromone Signals}

<!-- roko:layer:task -->
{Layer 4: Task Context}

{Layer 6a: Relevant Techniques}

{Layer 6b: Anti-Patterns}

<!-- roko:layer:dynamic -->
{Layer 8: Affect Guidance}
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
| 3a. Domain Context | 12% | 15% | 15% | 8% |
| 3b. Assembled Context | 8% | 12% | 5% | 8% |
| 3c. Pheromone Signals | 3% | 3% | 5% | 2% |
| 4. Task Context | 28% | 33% | 23% | 23% |
| 5. Tool Instructions | 12% | 12% | 12% | 12% |
| 6a. Relevant Techniques | 5% | 3% | 7% | 5% |
| 6b. Anti-Patterns | 7% | 4% | 10% | 7% |
| 8. Affect Guidance | 2% | 2% | 2% | 2% |
| *Reserve (conversation turns)* | 10% | 3% | 8% | 20% |

The Implementer gets the largest Task Context share because it needs detailed code context. The Strategist gets the largest Anti-Patterns share because strategic errors are more costly. The Scribe gets the largest reserve because documentation tasks often require extensive back-and-forth.

---

## 8. Dynamic Layer Ordering

The 9 layers are assembled in a fixed canonical order by default. But the optimal ordering may vary by task type. Research from 2025 strongly supports this hypothesis.

### 8.1 The Layer Ordering Hypothesis

**Directive-last principle.** Anthropic's context engineering guidance [2025] and systematic prompt surveys [arXiv:2402.07927] both confirm: placing the core task directive at the END of the prompt outperforms placing it first. When instructions come first, the LLM tends to generate additional context before following the task. When instructions come last, the model integrates all preceding grounding before acting.

This suggests the canonical order (role → conventions → knowledge → task → tools → anti-patterns → affect) is nearly optimal: grounding layers (1-3) precede the directive (4), which precedes constraints and modulation (5-7). The model reads grounding, receives the task, then sees tool availability and warnings before generating.

**But the optimal ordering is task-dependent.** For a simple rename task, the task directive should appear early (the agent needs minimal grounding). For a cross-crate integration, extensive grounding should precede the directive. This suggests a learned ordering policy.

### 8.2 Learned Layer Ordering

```rust
/// Represents a learned optimal layer ordering for a task category.
pub struct LayerOrderPolicy {
    /// Task category → ordered list of layer indices.
    /// Layer indices correspond to the 9 layers (0..9).
    pub orderings: HashMap<String, Vec<usize>>,
    /// Observation counts per category for confidence estimation.
    pub observation_counts: HashMap<String, usize>,
    /// Default ordering used when category has < min_observations.
    pub default_order: Vec<usize>,
    /// Minimum observations before using learned ordering.
    pub min_observations: usize,  // default: 20
}

impl LayerOrderPolicy {
    /// Select the layer ordering for a task.
    pub fn order_for(&self, task_category: &str) -> &[usize] {
        match self.orderings.get(task_category) {
            Some(order) if self.observation_counts[task_category] >= self.min_observations => order,
            _ => &self.default_order,
        }
    }

    /// Update the policy after observing a task outcome.
    /// Uses Thompson sampling: maintain Beta distributions per ordering variant.
    pub fn update(&mut self, task_category: &str, ordering_used: &[usize], gate_passed: bool) {
        // Increment observation count
        // Update success/failure counts for this ordering variant
        // Periodically re-solve for the best ordering using accumulated statistics
    }
}
```

### 8.3 Layer Interaction Effects

Do layers interact? Does putting knowledge before task context produce different outcomes than the reverse? Empirical evidence suggests yes:

**Grounding-before-directive.** Knowledge context (Layer 3a/3b) placed before the task directive (Layer 4) allows the model to integrate domain knowledge into its task understanding. The reverse — task first, then knowledge — risks the model forming a plan before seeing the relevant context, leading to plans that ignore available information.

**Anti-patterns-near-output.** Anti-patterns (Layer 6) placed at the end (near the generation boundary) exploit the recency attention effect [Liu et al. 2023]. The model's last impression before generating is "don't make these mistakes." Moving anti-patterns to Layer 3 position would bury them in the middle, reducing their effectiveness by ~30% (the attention degradation factor).

**Interaction matrix** (hypothesized, to be validated empirically):

| Layer A before B | Effect | Confidence |
|---|---|---|
| Knowledge → Task | Model grounds before planning | High (Anthropic 2025) |
| Task → Anti-patterns | Avoidance instructions near output | High (Liu et al. 2023) |
| Role → Conventions | Identity before rules | High (standard practice) |
| Tools → Task | Model plans with tool awareness | Medium (untested) |
| Affect → Anti-patterns | Mood context before warnings | Low (interaction unclear) |

### 8.4 Empirical Validation Plan

```
Protocol for measuring layer ordering effects:

1. Define 4 candidate orderings:
   a. Canonical: [1,2,3a,3b,4,5,6,7]
   b. Task-first: [1,4,2,3a,3b,5,6,7]
   c. Knowledge-heavy: [1,2,3a,3b,5,4,6,7] (tools before task)
   d. Safety-sandwiched: [6,1,2,3a,3b,4,5,7,6] (anti-patterns at start AND end)

2. Run each ordering on 50+ tasks per category (rename, implement, integrate)
3. Measure: gate pass rate, token usage, iteration count
4. Statistical test: paired t-test with Bonferroni correction for multiple comparisons

Expected result: ordering (a) or (d) dominates for complex tasks;
ordering (b) dominates for trivial tasks.
```

---

## 9. Prompt Compression Integration

The SystemPromptBuilder can optionally compress layers before assembly, enabling larger effective context in smaller windows.

### 9.1 Compression Strategies by Layer

Research from the LLMLingua family [Jiang et al., EMNLP 2023; LLMLingua-2, ACL Findings 2024] and RECOMP [Xu et al., ICLR 2024] demonstrates that different content types tolerate different compression methods:

| Layer | Compression Method | Compression Ratio | Rationale |
|---|---|---|---|
| 1. Role Identity | **None** | 1:1 | Identity is hand-crafted; compression risks altering persona |
| 2. Conventions | **None** | 1:1 | Safety rules must be verbatim |
| 3a. Domain Context | **RECOMP extractive** | 3:1 - 6:1 | Select most relevant sentences from PRD/workspace |
| 3b. Relevant Context | **LLMLingua-2 token pruning** | 2:1 - 5:1 | Remove low-information tokens while preserving semantics |
| 4. Task Context | **Light pruning only** | 1.5:1 | Task description needs high fidelity |
| 5. Tool Instructions | **Deduplication** | 1.2:1 | Remove redundant tool descriptions |
| 6. Anti-Patterns | **None** | 1:1 | Warnings must be exact |
| 7. Affect Guidance | **None** | 1:1 | Already minimal (~50 tokens) |

### 9.2 The Size-Fidelity Paradox

A critical finding from the NAACL 2025 prompt compression survey [Li et al., arXiv:2410.12388]: **larger compressor models produce less faithful compressions.** This occurs because larger models substitute their own parametric knowledge for source facts ("knowledge overwriting"). For Roko's composition layer, this means:

- Use **small** models (BERT-class, Haiku) for compression, not Opus
- Validate compressed output against source with exact-match checks on critical terms
- Never compress safety constraints or role identity (Layers 1, 2, 6)

### 9.3 Compression Budget Controller

```rust
/// Controls per-layer compression to fit an aggressive token budget.
pub struct CompressionBudgetController {
    /// Target total tokens after compression.
    pub target_tokens: usize,
    /// Per-layer compression configs.
    pub layer_configs: [LayerCompressionConfig; 8],
}

pub struct LayerCompressionConfig {
    /// Whether this layer can be compressed at all.
    pub compressible: bool,
    /// Maximum compression ratio (e.g., 5.0 means 5:1 max).
    pub max_ratio: f64,
    /// Compression method to use.
    pub method: CompressionMethod,
    /// Minimum tokens to retain (never compress below this).
    pub floor_tokens: usize,
}

pub enum CompressionMethod {
    /// No compression. Used for identity, safety, affect.
    None,
    /// Extractive: select most relevant sentences. For domain context.
    Extractive,
    /// Token pruning: remove low-surprisal tokens. For retrieved context.
    TokenPruning,
    /// Deduplication: remove redundant segments. For tool instructions.
    Dedup,
    /// Abstractive summarization: generate summary. For episode history.
    Abstractive,
}
```

### 9.4 Chain of Draft Integration

Chain of Draft [Zoom Research, arXiv:2502.18600] demonstrated that instructing models to produce 5-word-maximum intermediate reasoning steps matches CoT accuracy while using only **7.6% of tokens**. This is directly applicable to Layer 1 (Role Identity) instructions:

```rust
// Instead of verbose CoT instructions in role identity:
// OLD: "Think step by step. For each step, explain your reasoning in detail."
// NEW: "Think step by step, but write each reasoning step as a brief 5-word note."
```

This reduces token overhead in the agent's response without sacrificing reasoning quality.

---

## 10. Academic Foundations

**Plan-and-Solve Prompting** [Wang et al. 2023]. Improved zero-shot reasoning by splitting into two phases: devise a plan, then execute subtasks sequentially. The Strategist role's Layer 4 content embodies this: the task context includes the decomposition step that breaks down complex tasks before implementation.

**ReAct: Reasoning + Acting** [Yao et al. 2022]. Interleaving reasoning traces with task-specific actions produces better results than either alone. The 9-layer prompt structure supports ReAct by placing reasoning instructions (Layer 1 role identity, Layer 6 anti-patterns) alongside action instructions (Layer 4 task context, Layer 5 tools).

**Reflexion** [Shinn et al. 2023]. Verbal reinforcement learning: agents reflect on failures and use reflections to improve. Gate errors and iteration memory in Layer 4 are the Reflexion mechanism — they inject structured reflections from prior attempts into the next attempt's context.

**Step-Back Prompting** [Zheng et al. 2023]. Asking the model to abstract before solving improves reasoning by 7-27%. The Strategist's Layer 1 identity explicitly instructs abstraction before decomposition: "step back from the implementation details, reason about the architectural intent, then decompose."

**Chain of Draft** [Zoom Research, arXiv:2502.18600, February 2025]. Concise 5-word intermediate reasoning steps match CoT accuracy at 7.6% of the token cost. No model fine-tuning required — purely a prompt modification. Directly applicable to role identity instructions for token-constrained contexts.

**The Decreasing Value of CoT** [Meincke et al., Wharton GAIL 2025]. For reasoning models (o3-mini, o4-mini), explicit CoT prompts add only 2.9-3.1% improvement at 20-80% higher latency. For non-reasoning models, CoT increases variance. Implication: CoT instructions in Layer 1 should be conditional on the model class — skip for reasoning models, include for standard models.

**Anthropic Context Engineering** [Anthropic 2025]. Reframed "prompt engineering" as "context engineering." Key guidance: write instructions at the right altitude (not too high-level, not too low-level). Place grounding knowledge before task directives. The SystemPromptBuilder's layered architecture implements this principle directly.

**LLMLingua-2** [Pan et al., ACL Findings 2024, arXiv:2403.12968]. Token classification-based prompt compression. 3-6× faster than LLMLingua-1 with superior out-of-domain generalization. The BERT-level classifier makes it cheap enough for per-request compression of retrieved context layers.

**RECOMP** [Xu et al., ICLR 2024, arXiv:2310.04408]. Two-compressor architecture (extractive + abstractive) achieves 94% token reduction with minimal performance loss. Selective augmentation: returns empty string when retrieved content is irrelevant. Directly applicable to Layer 3b compression.

**Promptomatix** [arXiv:2507.14241, July 2025]. Automated prompt optimization framework that transforms natural language task descriptions into optimized prompts. Supports DSPy-powered compilation. Validates the concept of learning optimal prompt structure automatically — the same goal as Roko's learned layer ordering policy.

**IPEM: Inclusive Prompt Engineering Model** [Springer AI Review 2025]. Modular layered framework integrating Memory-of-Thought, Enhanced Chain-of-Thought, and feedback loops. Validates multi-layer prompt construction as superior to monolithic prompts.

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

## 11. Test Criteria for New Features

### Dynamic Layer Ordering Tests

```
test_canonical_ordering_is_default:
    Given no learned policy
    When building for any task category
    Then layers appear in order [1,2,3a,3b,4,5,6,7]

test_learned_ordering_applied:
    Given a policy with category "rename" → [1,4,2,5,6,7]
    When building for a "rename" task
    Then layers appear in learned order

test_cold_start_fallback:
    Given a policy with < min_observations for category "integrate"
    When building for "integrate"
    Then canonical ordering is used

test_ordering_preserves_cache_tiers:
    Given any layer ordering
    When building
    Then all System-tier layers appear before the first Session-tier layer
    (Cache alignment overrides learned ordering within tiers)
```

### Compression Integration Tests

```
test_identity_never_compressed:
    Given CompressionBudgetController with any settings
    When compressing Layer 1 (Role Identity)
    Then content is unchanged

test_domain_context_extractive_compression:
    Given a 2000-token domain context and 500-token budget
    When applying extractive compression
    Then output is <= 500 tokens
    And output contains the highest-relevance sentences from input

test_compression_floor_respected:
    Given floor_tokens = 100 for a layer
    When compressing that layer
    Then output is >= 100 tokens (or original if already under)
```

---

## 12. Current Status and Gaps

| Aspect | Status |
|--------|--------|
| 9-layer builder | **Implemented** |
| Cache alignment markers | **Implemented** |
| Affect guidance (arousal, pleasure) | **Implemented** |
| Role-specific budgets | **Implemented** |
| Wired into orchestrate.rs | **Implemented** |
| 12 unit tests | **Passing** |
| Dominance affect guidance | **Not yet** |
| Dynamic anti-patterns from knowledge store | **Scaffold** |
| Learned budget allocation (DSPy-style) | **Not yet** |
| Dynamic layer ordering (§8) | **Designed** — LayerOrderPolicy specified |
| Layer interaction measurement (§8.4) | **Not yet** — validation protocol specified |
| Prompt compression integration (§9) | **Designed** — CompressionBudgetController specified |
| Chain of Draft integration (§9.4) | **Not yet** — applicable to role identity |
| Conditional CoT by model class | **Not yet** — skip CoT for reasoning models |

---

## Cross-References

- [00-composer-trait.md](00-composer-trait.md) — Composer trait definition
- [03-role-templates.md](03-role-templates.md) — Role template details
- [05-token-budget-management.md](05-token-budget-management.md) — Budget allocation
- [06-lost-in-the-middle-u-shape.md](06-lost-in-the-middle-u-shape.md) — Attention curve that motivates layer positioning
- [12-affect-modulated-retrieval.md](12-affect-modulated-retrieval.md) — PAD-based modulation
- `crates/roko-compose/src/system_prompt_builder.rs` — Implementation source
- `crates/roko-compose/src/role_prompts.rs` — Role-specific wiring
