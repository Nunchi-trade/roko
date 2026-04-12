# 05 — Token Budget Management

> Layer 2 Scaffold — Synapse Architecture
> Status: **Implemented** — `roko-compose::budget` (270 lines) + `roko-compose::templates::common` (347 lines)
> Canonical source: `crates/roko-compose/src/budget.rs`, `crates/roko-compose/src/templates/common.rs`


> **Implementation**: Shipping

---

## Abstract

Token budget management determines how much of the LLM's context window is allocated to each prompt section. Roko implements a three-tier budget system: static per-role budgets (budget_for), complexity-adaptive budgets (adjusted_budget_for), and dynamic context-tier budgets (Surgical/Focused/Full). The system ensures that the most valuable context sections receive the most tokens, while low-value sections are dropped or truncated before they consume budget that higher-value sections need.

This document specifies the budget allocation tables, the complexity adjustment algorithm, the context tier system, the empirical basis for budget allocations, and the feedback loop that adapts budgets based on task outcomes.

---

## 1. Three-Tier Budget Architecture

### Tier 1: Static Per-Role Budgets

The foundation. Each role receives a fixed allocation across 9 section categories via `budget_for(role)` (see [03-role-templates.md](03-role-templates.md) §2.1). These budgets represent the baseline assumption about what each role needs.

### Tier 2: Complexity-Adaptive Budgets

Overlaid on the static budgets. The `adjusted_budget_for(role, complexity)` function scales allocations up or down based on task complexity:

| Complexity | Effect on Budget |
|-----------|-----------------|
| **Trivial** | Drop PRD, context, skills. Halve workspace_map and brief. ~70% reduction. |
| **Standard** | No change. Base budget applies. |
| **Complex** | +50% workspace_map, +100% context, +50% file_context. ~40% increase. |

### Tier 3: Context-Tier Budgets

The outermost constraint. The context tier (Surgical/Focused/Full) sets the absolute maximum token budget:

| Context Tier | Max Tokens | Model Class |
|-------------|-----------|-------------|
| **Surgical** | 4,000 | Haiku, Ollama, local models |
| **Focused** | 12,000 | Sonnet |
| **Full** | 24,000 | Opus |

The tightest constraint wins. A Complex Implementer task with a Full context tier gets up to 24K tokens with inflated allocations. A Trivial AutoFixer task with Surgical tier gets at most 4K tokens with deflated allocations.

---

## 2. The Differential Budget Principle

Different content types have different information density and different tolerance for compression. The budget system implements a differential allocation inspired by LLMLingua's Budget Controller [Jiang et al., EMNLP 2023]:

| Content Type | Compression Tolerance | Budget Priority | Rationale |
|-------------|---------------------|----------------|-----------|
| Task description | 0% (never compress) | Highest | Agent must know what to do |
| Role identity | 0% (never compress) | Highest | Agent must know what it is |
| Safety constraints | 0% (never compress) | Highest | Agent must know what not to do |
| Gate errors | 5% | High | Recent failures guide corrections |
| File context | 10-20% | High | Source code needs fidelity for correct implementation |
| Task brief | 10% | High | Summary of What/Why/How |
| PRD extract | 20-30% | Medium | Specification context |
| Workspace map | 30-50% | Medium | Project structure overview |
| Cross-plan context | 50%+ | Low | Often irrelevant to the current task |
| Learning pack | 50%+ | Low | High noise ratio (49% of tokens, 61% pass rate) |

The budget system encodes this differential: high-priority sections receive large allocations and are never dropped, while low-priority sections receive smaller allocations and are dropped first when the budget is tight.

---

## 3. Budget Allocation Algorithm

The allocation algorithm runs in two phases:

### Phase 1: Section Collection

All available sections are gathered with their content and metadata:

```rust
struct AvailableSection {
    name: String,
    content: String,
    actual_tokens: usize,
    priority: SectionPriority,
    cache_layer: CacheLayer,
}
```

### Phase 2: Priority-Ordered Allocation

```
1. Sort sections by priority (Critical first, then High, Normal, Low)
2. For each section in priority order:
   a. Look up its allocation in the PromptBudget
   b. If no allocation exists for this section, skip it
   c. If remaining budget < section's min_tokens, skip it
   d. Allocate min(actual_tokens, max_tokens, remaining_budget) tokens
   e. If actual_tokens > max_tokens, truncate section
   f. Deduct allocated tokens from remaining budget
3. Return allocated sections with their final content
```

This is a priority-first greedy allocation. Critical sections are guaranteed to be included (truncated if necessary). Lower-priority sections get whatever budget remains.

### The Min-Tokens Guard

Each section has a `min_tokens` threshold. If the remaining budget cannot accommodate at least `min_tokens` worth of content for a section, the section is skipped entirely rather than being included in a uselessly truncated form. A workspace map truncated to 100 tokens is worse than no workspace map — it provides structure without substance, confusing the model.

From the empirical budget analysis:

```rust
// Minimum useful content thresholds (from prompt-logs analysis)
SectionAllocation {
    section: "Workspace Map",
    max_tokens: 500,
    min_tokens: 100,  // Below 100 tokens, workspace map is useless
    priority: 3,
}
```

---

## 4. Prompt Prefix Stability for Caching

Budget allocation must respect prefix stability for prompt caching:

```
┌─────────────────────────────────────────────────────┐
│ System Prompt (role-specific, identical per role)    │ ← ALWAYS cached
│ Token cost: ~800                                    │
├─────────────────────────────────────────────────────┤
│ Workspace Map (changes only when files change)      │ ← Cached within wave
│ Token cost: ~334                                    │
├─────────────────────────────────────────────────────┤
│ Learning Pack (changes only on playbook refresh)    │ ← Cached within batch
│ Token cost: ~2,000 (after cap)                      │
├─────────────────────────────────────────────────────┤
│ PRD Extract (changes per plan)                      │ ← Cached within plan
│ Token cost: ~712                                    │
├─────────────────────────────────────────────────────┤
│ Task Description (unique per task)                  │ ← CACHE MISS boundary
│ Token cost: ~189                                    │
├─────────────────────────────────────────────────────┤
│ Iteration Context (unique per attempt)              │ ← Always miss
│ Token cost: varies                                  │
└─────────────────────────────────────────────────────┘
```

Rules for budget-aware prefix stability:
1. **Never randomize section ordering** — deterministic priority sort only
2. **Freeze workspace map within a plan execution** — generate once, reuse for all tasks
3. **Cap learning pack within a batch** — do not re-extract playbook mid-batch
4. **Normalize whitespace** — strip trailing spaces, normalize newlines
5. **Sort tool definitions alphabetically** — BTreeMap, not HashMap

---

## 5. Section A/B Testing Protocol

The budget system integrates with the ExperimentStore (from roko-learn) for A/B testing individual sections:

### Testing Template

For each section to test:
1. Configure experiment in roko.toml
2. Run 50+ plans (25 control, 25 variant)
3. Measure pass rate delta and cost delta
4. Apply decision matrix:

| Pass Rate Delta | Cost Delta | Decision |
|----------------|------------|----------|
| +3% or more | Lower | Keep variant (section hurts) |
| ±3% | Lower | Keep variant (section neutral, saves money) |
| ±3% | Same | Either (section does not matter) |
| −3% or more | — | Revert (section helps) |

### Recommended Test Priority

| Priority | Section | Hypothesis |
|----------|---------|-----------|
| 1 | Cross-Plan Context | Hurts simple tasks (55% pass rate) |
| 2 | Execution Strategy | Marginal value (58% pass rate) |
| 3 | Learning Pack cap (2800 tok) | Less noise improves outcomes |
| 4 | Workspace Map | May be redundant with MCP tools |
| 5 | Self-Review instructions | May be wasted tokens |

---

## 6. History Compaction

When conversation history exceeds the budget, lossy compaction is applied using a cheap model:

```
Non-system messages split into (older, recent) at split point.
Split point = total_messages - (recent_verbatim_turns × 2).
If older messages exceed the summary budget:
    Summarize older messages using Haiku → <conversation_summary>
    Prepend summary, then append recent messages verbatim.
```

Default parameters:
- `recent_verbatim_turns`: 10 (keep the last 10 turns verbatim)
- `older_summary_budget`: 2,000 tokens

Two compaction strategies:
1. **In-place compaction** (Claude Code pattern): Haiku summarizes older messages. After 2-3 compactions, information loss compounds. The system warns via header; agents should consider handoff after 2 compactions.
2. **Handoff** (Amp pattern): Sonnet produces a structured briefing from the full thread, then a new session starts with that briefing. Avoids the quality cliff of repeated compaction.

---

## 7. The "Context Anxiety" Mitigation

An empirical finding from Devin's development: Claude proactively summarizes when it perceives it is near context limits, even when it is not. The agent's own compaction interferes with managed compaction.

Mitigation: always request the maximum context window from the provider (1M tokens) regardless of actual usage. This prevents the model's own compaction from triggering, keeping context management entirely in the scaffold's control.

---

## 8. Impact Numbers

When all budget management layers compose:

| Metric | Without budget management | With budget management |
|--------|--------------------------|----------------------|
| Input tokens per task | ~12K average | ~2.4K average |
| Inference cost per task | ~$2.50 | ~$0.42 |
| Gate pass rate (first attempt) | 71% | 94% |
| Average iterations per plan | 3.4 | 1.8 |
| 20-plan run cost | ~$200 | ~$34 |

The 83% cost reduction comes from every layer stacking: extraction eliminates LLM calls, compression reduces token count, caching reduces per-token cost, better context reduces iteration count, fewer iterations reduce total calls. Each layer multiplies the effect of the others.

The general principle: replace every LLM call you can with a deterministic operation, and spend your LLM budget on work that only language models can do. Tree-sitter does symbol extraction in 6ms for $0.00. Asking an LLM to "extract the public API of this file" costs $0.02 and takes 8 seconds. At 847 files, that is $16.94 and 6,776 seconds versus 6 seconds and $0.00.

---

## 9. Academic Foundations

**LLMLingua Budget Controller** [Jiang et al., EMNLP 2023]. The differential budget principle (different content types have different compression tolerance) derives from LLMLingua's coarse-to-fine compression with budget awareness. LLMLingua achieves up to 20× compression with minimal performance loss on GSM8K, BBH, and ShareGPT.

**Selective Context** [Li et al., EMNLP 2023]. Information-theoretic approach to identifying and removing redundant content. 50% context reduction, 36% less memory usage, 32% faster inference with only 0.023 BERTscore drop. The principle — select context that maximizes mutual information with the task — is what Roko's priority-based dropping approximates.

**CLEAR Framework** [2025]. Five evaluation dimensions: Cost, Latency, Efficacy, Assurance, Reliability. CLEAR's most important finding: optimizing for accuracy alone produces systems 4.4-10.8× more expensive than cost-aware alternatives. The budget system explicitly co-optimizes cost and quality.

**Sufficient Context** [Joren et al., ICLR 2025]. The most striking RAG finding: Gemma went from 10.2% incorrect with no context to 66.1% incorrect with insufficient context. Adding bad context made the model 6× worse. The budget system's min_tokens guard implements this principle: if a section cannot be included with sufficient fidelity, skip it entirely rather than including a truncated version that might mislead.

**Context Rot** [Chroma 2025]. Performance degrades as context grows, even within capacity limits. Semantically close distractors are far more harmful than obviously irrelevant content. The budget system's aggressive pruning of low-value sections (Cross-Plan Context, Execution Strategy) directly mitigates context rot.

---

## 10. Current Status and Gaps

| Aspect | Status |
|--------|--------|
| PromptBudget per role | **Implemented** |
| Complexity-adaptive budgets | **Implemented** |
| Context tier (Surgical/Focused/Full) | **Implemented** |
| Min-tokens guard | **Implemented** |
| Cache-aware allocation ordering | **Implemented** |
| History compaction | **Implemented** |
| A/B testing framework | **Scaffold** (ExperimentStore exists) |
| Learned budget optimization | **Not yet** |
| Per-section value tracking | **Partially** (efficiency events) |

---

## Cross-References

- [00-composer-trait.md](00-composer-trait.md) — Budget struct in Composer trait
- [01-prompt-composer.md](01-prompt-composer.md) — Budget enforcement in assembly
- [03-role-templates.md](03-role-templates.md) — Per-role allocation table
- [06-lost-in-the-middle-u-shape.md](06-lost-in-the-middle-u-shape.md) — Attention-aware placement
- `crates/roko-compose/src/budget.rs` — Complexity-adaptive budgets
- `crates/roko-compose/src/templates/common.rs` — budget_for() table
- `crates/roko-compose/src/context_provider.rs` — Context tier definitions
