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

## 10. Budget conflict resolution

When multiple sections compete for a limited token budget, the system resolves conflicts through a strict priority ordering and a set of truncation strategies.

### 10.1 Priority ordering

Sections are allocated budget in this order. Higher-priority sections are guaranteed allocation before lower-priority sections are considered:

| Priority | Category | Sections | Drop policy |
|---|---|---|---|
| 0 (Critical) | Identity and safety | Role identity, safety constraints, task description | Never drop, never truncate |
| 1 (High) | Recent failures | Gate errors, iteration context | Truncate to last N errors if over budget |
| 2 (High) | Source code | File context | Truncate from bottom (keep imports + signatures) |
| 3 (Medium) | Task context | Task brief, PRD extract | Truncate from bottom (keep requirements section) |
| 4 (Medium) | Structure | Workspace map | Truncate deep nodes (keep top-level tree) |
| 5 (Low) | History | Cross-plan context, learning pack | Drop entirely before truncating higher sections |

When a priority tie occurs (two sections at the same level), the section with higher actual token count is allocated first. This prevents a small section from starving a large section that needs a minimum allocation to be useful.

### 10.2 Truncation strategies

Each section type has a truncation strategy that preserves the most valuable content:

```
Section truncation strategies:

  Gate errors:
    Keep the N most recent errors (LIFO).
    N = floor(budget / avg_error_tokens).
    Rationale: the latest error is the most relevant.

  File context:
    Keep: imports, struct/enum definitions, function signatures.
    Drop: function bodies (largest token consumer).
    If still over budget: keep only the file most recently modified by the agent.

  PRD extract:
    Keep: Requirements section, success criteria.
    Drop: Background, rationale, alternatives considered.
    Rationale: agents need to know WHAT, not WHY.

  Workspace map:
    Keep: top 2 levels of directory tree.
    Drop: deeper levels, file-level entries.
    If still over budget: keep only the crate(s) relevant to the task.

  Task brief:
    Keep: What/How sections.
    Drop: Why/Context sections.

  Learning pack:
    Drop: entire section if budget is < min_tokens (2,000).
    Rationale: partially truncated learning content is actively harmful
    (from the "Sufficient Context" finding: bad context makes the model 6x worse).
```

### 10.3 Conflict resolution algorithm

```
fn resolve_budget_conflicts(sections: &mut [AvailableSection], total_budget: usize) {
    // Phase 1: Sort by priority (lower number = higher priority)
    sections.sort_by_key(|s| s.priority);

    let mut remaining = total_budget;

    // Phase 2: Allocate critical sections (priority 0) unconditionally
    for section in sections.iter_mut().filter(|s| s.priority == 0) {
        let alloc = section.actual_tokens;
        section.allocated = alloc;
        remaining = remaining.saturating_sub(alloc);
    }

    // Phase 3: Allocate remaining sections in priority order
    for section in sections.iter_mut().filter(|s| s.priority > 0) {
        if remaining < section.min_tokens {
            // Not enough budget for a useful version of this section
            section.allocated = 0;
            section.dropped = true;
            continue;
        }

        let alloc = section.actual_tokens.min(section.max_tokens).min(remaining);
        if alloc < section.actual_tokens {
            // Budget is tight: apply section-specific truncation
            section.content = truncate(&section.content, alloc, section.strategy);
        }
        section.allocated = alloc;
        remaining = remaining.saturating_sub(alloc);
    }
}
```

### 10.4 Edge cases

| Scenario | Resolution |
|---|---|
| Critical sections exceed total budget | Truncate task description (never truncate role/safety). This should not happen with budgets >= 2,000 tokens. |
| All non-critical sections dropped and budget remains | Expand file context allocation (source code is the most useful non-critical content). |
| Two sections at same priority, both need full allocation | Allocate to the section with higher `max_tokens` first. The other gets whatever remains. |
| Section content is empty | Skip without deducting budget. |
| Total budget is 0 | Return only role identity (hardcoded ~200 tokens). |

---

## 11. Budget Prediction: Estimate Before Assembly

Before assembling context, predict how much budget a task will need. This avoids two failure modes: (a) over-fetching context for trivial tasks (wastes tokens, risks context rot), and (b) under-fetching for complex tasks (insufficient context, gate failure).

### 11.1 The TALE Approach

The TALE framework [Hu et al., ACL Findings 2025, arXiv:2412.18547] demonstrated that models can predict the token budget needed for a problem **before** generating the solution. TALE reduces token usage by 68.9% with <5% accuracy loss. The key insight: there is a consistent positive correlation between problem complexity and allocated budget — the model learns to quantify difficulty.

Applied to Roko: before running the 5-stage assembly pipeline, a lightweight predictor estimates the context budget needed for this specific task.

### 11.2 Budget Predictor

```rust
/// Predicts the token budget a task will need before assembly begins.
pub struct BudgetPredictor {
    /// Historical task outcomes: (features, budget_used, gate_passed).
    history: Vec<BudgetObservation>,
    /// Feature extractor for task descriptions.
    feature_extractor: TaskFeatureExtractor,
    /// Regression model: features → predicted optimal budget.
    model: BudgetRegressionModel,
}

pub struct BudgetObservation {
    pub task_category: String,
    pub complexity: Complexity,
    pub role: AgentRole,
    pub files_touched: usize,
    pub crates_involved: usize,
    pub has_prior_failures: bool,
    pub budget_allocated: usize,
    pub budget_used: usize,         // how much the agent actually consumed
    pub gate_passed: bool,
    pub iterations_needed: usize,
}

pub struct TaskFeatureExtractor;

impl TaskFeatureExtractor {
    /// Extract features from a task for budget prediction.
    pub fn extract(&self, task: &TaskInput) -> TaskFeatures {
        TaskFeatures {
            description_tokens: estimate_tokens(&task.description),
            file_count: task.read_files.len(),
            crate_count: task.target_crates.len(),
            has_gate_errors: !task.prior_gate_errors.is_empty(),
            iteration_number: task.iteration,
            complexity_band: task.complexity,
            role: task.role,
        }
    }
}

pub struct BudgetRegressionModel {
    /// Per-category linear regression coefficients.
    /// Predicts: optimal_budget = bias + Σ(w_i × feature_i)
    coefficients: HashMap<String, Vec<f64>>,
    /// Minimum observations before trusting predictions.
    min_observations: usize,  // default: 15
}

impl BudgetRegressionModel {
    /// Predict optimal budget for a task.
    pub fn predict(&self, category: &str, features: &TaskFeatures) -> Option<usize> {
        let coeffs = self.coefficients.get(category)?;
        if self.observation_count(category) < self.min_observations {
            return None;  // Not enough data — use static budget
        }
        let prediction = coeffs[0]  // bias
            + coeffs[1] * features.description_tokens as f64
            + coeffs[2] * features.file_count as f64
            + coeffs[3] * features.crate_count as f64
            + coeffs[4] * features.has_gate_errors as i32 as f64
            + coeffs[5] * features.iteration_number as f64;
        Some(prediction.max(2000.0) as usize)  // floor at 2000 tokens
    }
}
```

### 11.3 Prediction → Static Fallback Cascade

```
fn resolve_budget(task: &TaskInput, predictor: &BudgetPredictor) -> usize {
    // 1. Try learned prediction
    if let Some(predicted) = predictor.predict(task) {
        return predicted;
    }
    // 2. Fall back to complexity-adaptive static budget
    let adjusted = adjusted_budget_for(task.role, task.complexity);
    adjusted.total_tokens()
}
```

### 11.4 SelfBudgeter Pattern

The SelfBudgeter approach [Li et al., arXiv:2505.11274, May 2025] trains the model itself to predict its needed budget before reasoning. Applied to Roko: instead of an external predictor, the system prompt can include a preamble asking the agent to estimate its context needs:

```
Before starting, briefly assess: on a scale of 1-5, how much context
do you need for this task? (1 = trivial rename, 5 = cross-crate architectural change)
```

The agent's self-assessment is parsed and used to dynamically adjust which enrichment artifacts are loaded. This is lower accuracy than the statistical predictor but requires zero training data.

---

## 12. Budget Learning: Track and Optimize Allocations

### 12.1 Per-Section Value Tracking

Every task execution records which sections were included, their token counts, and the outcome:

```rust
/// Recorded after each task execution for budget optimization.
pub struct BudgetOutcome {
    pub task_id: String,
    pub task_category: String,
    pub role: AgentRole,
    pub complexity: Complexity,
    /// Per-section: (name, tokens_allocated, was_included, was_truncated).
    pub section_allocations: Vec<SectionAllocationRecord>,
    /// Task outcome.
    pub gate_passed: bool,
    pub iterations_needed: usize,
    pub total_input_tokens: usize,
    pub total_output_tokens: usize,
    pub total_cost_usd: f64,
    pub timestamp: Timestamp,
}

pub struct SectionAllocationRecord {
    pub section_name: String,
    pub tokens_allocated: usize,
    pub tokens_actual: usize,
    pub was_included: bool,
    pub was_truncated: bool,
    pub priority: SectionPriority,
}
```

### 12.2 Leave-One-Out Section Value

The Contextual Influence Value framework [Shanghai Jiao Tong University 2025] measures per-section impact through leave-one-out analysis. Applied to Roko's budget system:

```
For each section S in the context pack:
    influence(S) = pass_rate_with_S - pass_rate_without_S

If influence(S) > 0: S is valuable — increase its budget allocation.
If influence(S) ≈ 0: S is neutral — candidate for compression or dropping.
If influence(S) < 0: S is harmful — drop it (it's introducing context rot).
```

This doesn't require controlled experiments (which are expensive). Instead, it's computed from natural variation: tasks where a section was dropped due to budget constraints (the "without S" condition) versus tasks where it was included (the "with S" condition).

```rust
/// Compute per-section influence from historical outcomes.
pub fn compute_section_influence(
    outcomes: &[BudgetOutcome],
    section_name: &str,
    task_category: &str,
) -> SectionInfluence {
    let with_section: Vec<_> = outcomes.iter()
        .filter(|o| o.task_category == task_category)
        .filter(|o| o.section_allocations.iter().any(|s|
            s.section_name == section_name && s.was_included))
        .collect();

    let without_section: Vec<_> = outcomes.iter()
        .filter(|o| o.task_category == task_category)
        .filter(|o| o.section_allocations.iter().any(|s|
            s.section_name == section_name && !s.was_included))
        .collect();

    let pass_rate_with = with_section.iter()
        .filter(|o| o.gate_passed).count() as f64 / with_section.len().max(1) as f64;
    let pass_rate_without = without_section.iter()
        .filter(|o| o.gate_passed).count() as f64 / without_section.len().max(1) as f64;

    SectionInfluence {
        section_name: section_name.to_string(),
        influence: pass_rate_with - pass_rate_without,
        observations_with: with_section.len(),
        observations_without: without_section.len(),
        confidence: wilson_confidence_interval(
            with_section.len(), without_section.len(),
            pass_rate_with, pass_rate_without,
        ),
    }
}

pub struct SectionInfluence {
    pub section_name: String,
    /// Positive = section helps, negative = section hurts.
    pub influence: f64,
    pub observations_with: usize,
    pub observations_without: usize,
    /// 95% confidence interval width. Narrow = confident estimate.
    pub confidence: f64,
}
```

### 12.3 Adaptive Budget Reallocation

Once section influence values are computed, the budget allocations are updated:

```
Algorithm: Budget reallocation from influence values

1. Compute influence(S) for all sections S with sufficient observations (>= 20 each)
2. Classify sections:
   - Valuable (influence > 0.05): increase allocation by 20%
   - Neutral (-0.05 ≤ influence ≤ 0.05): no change
   - Harmful (influence < -0.05): reduce allocation by 50% or drop entirely
3. Redistribute freed tokens to valuable sections
4. Apply min_tokens floor (never allocate below minimum useful threshold)
5. Persist updated allocations to .roko/learn/budget-allocations.json
6. Log changes for audit trail

Constraints:
- Never modify Critical section allocations (role, safety, task)
- Reallocation bounded by ±50% of baseline per cycle
- Require >= 50 observations per category before any reallocation
- Run reallocation at most once per day (avoid over-fitting to recent data)
```

### 12.4 Information-Theoretic Budget Allocation

The Selective Context approach [Li et al., EMNLP 2023] uses self-information (surprisal) to identify valuable tokens. Applied to budget allocation: allocate more budget to sections with high average surprisal (they carry more information per token) and less to sections with low surprisal (they're predictable given other context).

```rust
/// Score a section's information density using token-level surprisal.
/// Higher surprisal = more informative content = deserves more budget.
pub fn section_information_density(
    section_content: &str,
    other_sections: &[&str],
) -> f64 {
    // Approximate: measure how much of section's content is predictable
    // given the other sections (cross-entropy proxy).
    //
    // Use n-gram overlap as a cheap proxy for mutual information:
    // - High overlap with other sections → low marginal information → low density
    // - Low overlap → high marginal information → high density
    let section_ngrams = extract_ngrams(section_content, 3);
    let other_ngrams: HashSet<_> = other_sections.iter()
        .flat_map(|s| extract_ngrams(s, 3))
        .collect();

    let novel_fraction = section_ngrams.iter()
        .filter(|ng| !other_ngrams.contains(*ng))
        .count() as f64 / section_ngrams.len().max(1) as f64;

    novel_fraction  // Range [0, 1]. Higher = more novel content.
}
```

### 12.5 Persistence

Budget learning state persists to `.roko/learn/budget-allocations.json`:

```json
{
  "version": 1,
  "updated_at": "2026-04-12T10:00:00Z",
  "section_influences": {
    "implement": {
      "workspace_map": { "influence": 0.08, "observations": 142, "confidence": 0.04 },
      "cross_plan_context": { "influence": -0.03, "observations": 89, "confidence": 0.06 },
      "learning_pack": { "influence": -0.07, "observations": 201, "confidence": 0.03 }
    }
  },
  "adjusted_allocations": {
    "implement": {
      "workspace_map": 24000,
      "cross_plan_context": 2000,
      "learning_pack": 0
    }
  }
}
```

---

## 13. Current Status and Gaps

| Aspect | Status |
|--------|--------|
| PromptBudget per role | **Implemented** |
| Complexity-adaptive budgets | **Implemented** |
| Context tier (Surgical/Focused/Full) | **Implemented** |
| Min-tokens guard | **Implemented** |
| Cache-aware allocation ordering | **Implemented** |
| History compaction | **Implemented** |
| A/B testing framework | **Scaffold** (ExperimentStore exists) |
| Budget prediction (§11) | **Designed** — BudgetPredictor + regression model specified |
| Budget learning / section influence (§12) | **Designed** — leave-one-out influence + adaptive reallocation specified |
| Information-theoretic density scoring (§12.4) | **Designed** — n-gram novelty proxy specified |
| Per-section value tracking | **Partially** (efficiency events exist, BudgetOutcome not yet) |

---

## Cross-References

- [00-composer-trait.md](00-composer-trait.md) — Budget struct in Composer trait
- [01-prompt-composer.md](01-prompt-composer.md) — Budget enforcement in assembly
- [02-system-prompt-builder-7-layer.md](02-system-prompt-builder-7-layer.md) — Compression integration (§9)
- [03-role-templates.md](03-role-templates.md) — Per-role allocation table
- [06-lost-in-the-middle-u-shape.md](06-lost-in-the-middle-u-shape.md) — Attention-aware placement
- [07-active-inference-context-selection.md](07-active-inference-context-selection.md) — Scoring that feeds budget decisions
- [09-predictive-foraging-mvt.md](09-predictive-foraging-mvt.md) — MVT stopping rule as budget control
- [11-distributed-context-engineering.md](11-distributed-context-engineering.md) — Contextual Influence Value framework
- `crates/roko-compose/src/budget.rs` — Complexity-adaptive budgets
- `crates/roko-compose/src/templates/common.rs` — budget_for() table
- `crates/roko-compose/src/context_provider.rs` — Context tier definitions
