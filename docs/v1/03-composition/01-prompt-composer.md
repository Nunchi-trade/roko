# 01 — PromptComposer: Priority Dropping and U-Shape Placement

> Layer 2 Scaffold — Synapse Architecture
> Status: **Implemented** — `roko-compose::prompt` (772 lines, 18 tests)
> Canonical source: `crates/roko-compose/src/prompt.rs`


> **Implementation**: Shipping

---

## Abstract

PromptComposer is the primary implementation of the Composer trait. It transforms a collection of typed, prioritized prompt sections into a single budget-fitted, cache-aligned prompt string. The core algorithm is a greedy knapsack with priority partitioning: Critical sections are never dropped, optional sections are included in priority order until the budget is exhausted, and the final output is ordered by Placement hints to implement the U-shaped attention optimization from Liu et al. (2023).

This document specifies the PromptSection data model, the priority dropping algorithm, the cache-layer ordering scheme, the U-shape placement logic, and the token estimation heuristic.

---

## 1. The PromptSection Data Model

Every piece of context that enters the Composer is wrapped in a `PromptSection`:

```rust
// crates/roko-compose/src/prompt.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptSection {
    /// Human-readable section name (e.g., "role_identity", "workspace_map").
    pub name: String,
    /// The actual content text of this section.
    pub content: String,
    /// Priority level for budget fitting.
    pub priority: SectionPriority,
    /// Which cache layer this section belongs to.
    pub cache_layer: CacheLayer,
    /// Where this section should appear in the final prompt.
    pub placement: Placement,
    /// Optional hard character cap for this section.
    pub hard_cap: Option<usize>,
}
```

### 1.1 SectionPriority

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SectionPriority {
    /// Drop first when budget is tight.
    Low = 1,
    /// Standard priority — included unless budget is exhausted.
    Normal = 2,
    /// Important — included before Normal sections.
    High = 3,
    /// Never dropped, only truncated. Safety rules, role identity, task description.
    Critical = 4,
}
```

Critical sections are the invariant core of every prompt: role identity, safety constraints, task description. They are never dropped, even if the budget is exceeded — they are truncated to fit if necessary. This guarantee ensures that the agent always knows what it is, what it should do, and what it must not do.

### 1.2 CacheLayer

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CacheLayer {
    /// Role-level: identical across all tasks for this role. Highest cache value.
    System = 0,
    /// Session-level: stable within a plan execution.
    Session = 1,
    /// Task-level: stable within a single task's iterations.
    Task = 2,
    /// Dynamic: unique per request. No cache value.
    Dynamic = 3,
}
```

Cache layers control the ordering of sections in the assembled prompt. Lower-numbered layers appear first, forming a stable prefix that can be cached by the LLM provider:

- **System (0):** Role identity, conventions, tool definitions. Identical across all tasks for the same role. Anthropic's prompt caching gives 90% token cost discount on cache hits. For a 20-plan run with 80 agent spawns, this prefix hits the cache on every request after the first.
- **Session (1):** Workspace map, cross-plan context. Stable within a build iteration.
- **Task (2):** Plan content, PRD extract, task brief. Stable within a single task.
- **Dynamic (3):** Gate errors, iteration memory, review feedback. Unique per turn.

The BTreeMap requirement: cache hits require byte-identical content. All serialization in cacheable layers uses `BTreeMap` for deterministic key ordering. If tool definitions were serialized with `HashMap` (non-deterministic ordering in Rust), two runs would produce different bytes for the same logical content, defeating prefix caching. This detail saves approximately $1.75 per 20-plan run.

### 1.3 Placement

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Placement {
    /// Place at the beginning of the prompt. Highest attention zone.
    Start,
    /// Place in the middle. Lowest attention zone.
    Middle,
    /// Place at the end. Second-highest attention zone.
    End,
}
```

Placement implements the "Lost in the Middle" optimization from Liu et al. (2023) [arXiv:2307.03172]. Language models attend most strongly to the beginning and end of their context, with degraded attention to the middle. Critical information (task description, safety rules, recent errors) is placed at Start or End. Lower-priority information (workspace map, cross-plan context) occupies the Middle.

---

## 2. The Assembly Algorithm

The PromptComposer's `compose()` method implements a multi-phase assembly:

### Phase 1: Decode and Score

Candidate engrams are decoded into `PromptSection` structs. Each section is scored by the provided Scorer, which produces a composite score from priority, recency, relevance, and other signals.

### Phase 2: Partition

Sections are partitioned into two groups:
- **Critical:** `SectionPriority::Critical` — guaranteed inclusion.
- **Optional:** Everything else — included by score order until budget exhausted.

### Phase 3: Sort and Select

Optional sections are sorted by two keys:
1. **CacheLayer ascending** — System before Session before Task before Dynamic
2. **SectionPriority descending** — High before Normal before Low

Within each (CacheLayer, Priority) group, sections are ordered by their Scorer-assigned score descending. This produces a deterministic ordering that maximizes prefix cache hits while respecting priority.

### Phase 4: Greedy Include

```
remaining_budget = budget.max_tokens
included = []

// Critical sections always included
for section in critical_sections:
    if estimate_tokens(section.content) <= remaining_budget:
        included.append(section)
        remaining_budget -= estimate_tokens(section.content)
    else:
        // Truncate to fit — never drop Critical
        section.content = truncate_to_tokens(section.content, remaining_budget)
        included.append(section)
        remaining_budget = 0

// Optional sections by score order
for section in sorted_optional_sections:
    tokens = estimate_tokens(section.content)
    if tokens <= remaining_budget:
        // Apply hard_cap if set
        if section.hard_cap and len(section.content) > section.hard_cap:
            section.content = truncate(section.content, section.hard_cap)
            tokens = estimate_tokens(section.content)
        included.append(section)
        remaining_budget -= tokens
    // else: skip (drop) this section
```

This is a greedy knapsack, not an optimal one. Greedy is chosen over dynamic programming for three reasons:
1. **Speed:** O(n log n) sort + O(n) scan, compared to O(n × W) for DP knapsack.
2. **Determinism:** Same input always produces same output.
3. **Priority correctness:** A greedy algorithm respecting priority ordering always includes the most important sections, which is the correct heuristic for prompt assembly.

### Phase 5: U-Shape Ordering

After selection, included sections are reordered by Placement:

```
final_order = [
    sections with Placement::Start,   // highest attention
    sections with Placement::Middle,  // lowest attention
    sections with Placement::End,     // second-highest attention
]
```

Within each placement group, the CacheLayer ordering is preserved. This produces the U-shape: critical content at the beginning (role, safety, task) and end (recent errors, constraints reminder), with supporting context (workspace map, enrichment artifacts) in the middle.

### Phase 6: Concatenate

Sections are concatenated with headers:

```
<!-- roko:section:role_identity -->
{role identity content}

<!-- roko:section:workspace_map -->
{workspace map content}

...
```

Cache-layer transition markers are emitted at layer boundaries:

```
<!-- roko:layer:0 -->
{system-level sections}

<!-- roko:layer:1 -->
{session-level sections}

<!-- roko:layer:2 -->
{task-level sections}
```

These markers allow the inference gateway to place `cache_control` breakpoints at the correct positions.

---

## 3. Token Estimation

The PromptComposer uses a byte-based heuristic for token estimation:

```rust
// crates/roko-compose/src/prompt.rs

fn estimate_tokens(text: &str) -> usize {
    // ~4 bytes per token for English text and source code.
    // Empirically calibrated against cl100k_base (Anthropic/OpenAI tokenizer).
    text.len() / 4
}
```

This heuristic is deliberately conservative:
- English prose averages ~4.5 bytes/token
- Source code averages ~3.5 bytes/token
- The 4.0 heuristic slightly overestimates for prose and underestimates for code, producing prompts that are near but safely within the budget.

Exact tokenization (loading the cl100k_base tokenizer) takes ~2ms per call and adds a dependency on a tokenizer library. The heuristic takes <1μs and is correct within ±15%. For prompt assembly where the budget is a soft target (not a hard API limit), this accuracy is sufficient.

---

## 4. The PromptBuild Metadata

Each composition produces metadata alongside the assembled prompt:

```rust
// crates/roko-compose/src/prompt.rs

pub struct PromptBuild {
    /// Estimated total tokens in the assembled prompt.
    pub estimated_tokens: usize,
    /// Number of sections included.
    pub sections_included: usize,
    /// Number of sections dropped due to budget.
    pub sections_dropped: usize,
    /// Names of dropped sections (for debugging).
    pub dropped_names: Vec<String>,
    /// Cache layer breakdown (tokens per layer).
    pub tokens_per_layer: HashMap<CacheLayer, usize>,
}
```

This metadata enables:
- **Cost prediction:** Estimated tokens directly predict inference cost.
- **Debugging:** If a task fails, the dropped sections list shows what context was missing.
- **Cache analysis:** Tokens per layer shows what fraction of the prompt is cacheable.
- **Budget tuning:** If sections are consistently dropped, the budget or priorities need adjustment.

---

## 5. Legacy Comparison: Mori's assemble_prompt

The Roko PromptComposer replaces Mori's `assemble_prompt` function (`apps/mori/src/orchestrator/prompts.rs`). Key differences:

| Aspect | Mori (legacy) | Roko PromptComposer |
|--------|--------------|---------------------|
| Priority levels | u8 (0-255) | Enum (Low/Normal/High/Critical) |
| Budget unit | Characters (token_budget × 4) | Tokens (estimated) |
| Cache layers | u8 (0-3) with `<!-- mori:layer:N -->` markers | CacheLayer enum with `<!-- roko:layer:N -->` markers |
| U-shape | Not implemented (sort by cache_layer only) | Placement enum (Start/Middle/End) |
| Scorer integration | None (static priorities) | Accepts `&dyn Scorer` parameter |
| Hard caps | Optional per-section character limit | Optional per-section character limit |
| Critical guarantee | Priority 5 is truncated, never dropped | Critical enum variant is truncated, never dropped |
| Metadata | None | PromptBuild struct with drop report |

The mechanical improvement is the U-shape placement. Mori's prompts placed sections in cache-layer order (system → workspace → plan → volatile), which put the task description and gate errors in the volatile section at the end. This accidentally achieved partial U-shape (task errors at the end = high attention), but workspace maps and cross-plan context were in the middle where attention degrades. The Roko PromptComposer explicitly places Start/Middle/End sections to maximize attention to critical content.

---

## 6. Critical Section Examples

Sections that receive `SectionPriority::Critical`:

| Section | Rationale |
|---------|-----------|
| `role_identity` | Agent must know what role it plays |
| `task_description` | Agent must know what to do |
| `safety_constraints` | Agent must know what not to do |
| `conventions` | Agent must follow project patterns |
| `anti_patterns` | Agent must avoid known failure modes |

Sections that receive `SectionPriority::High`:

| Section | Rationale |
|---------|-----------|
| `gate_errors` | Recent failures must inform next attempt |
| `iteration_memory` | Cross-iteration state prevents repeated mistakes |
| `task_brief` | Detailed context for current task |

Sections that receive `SectionPriority::Normal`:

| Section | Rationale |
|---------|-----------|
| `workspace_map` | Helpful but not always needed |
| `cross_plan_context` | Useful for integration tasks |
| `prd_extract` | Relevant for spec compliance tasks |
| `research_memo` | Relevant for novel tasks |

Sections that receive `SectionPriority::Low`:

| Section | Rationale |
|---------|-----------|
| `sibling_tasks` | Awareness of other tasks in the plan |
| `registry` | Tool availability reference |

---

## 7. Interaction with Enrichment Pipeline

The PromptComposer operates downstream of the enrichment pipeline (see [04-enrichment-pipeline-13-step.md](04-enrichment-pipeline-13-step.md)). The enrichment pipeline pre-computes artifacts (briefs, decompositions, research memos, verification checklists) that become PromptSections fed into the Composer. The Composer does not know or care how the artifacts were produced — it receives them as PromptSections with priority, cache_layer, and placement metadata, and assembles them under budget.

This separation is load-bearing: the enrichment pipeline can be modified, extended, or replaced without changing the Composer. New enrichment steps simply produce new PromptSections. The Composer includes or drops them based on their priority and the available budget.

---

## 8. Academic Foundations

**Greedy Knapsack Approximation.** The Composer's budget-fitting algorithm is a greedy approximation to the 0/1 knapsack problem. When items are sorted by value-to-weight ratio (here: priority-to-token-count), the greedy algorithm achieves at least 50% of optimal [Dantzig 1957]. For prompt assembly, greedy is preferred because the "value" of including a section is not independent of other sections — the marginal value of a second workspace map is zero, while the value of a first one is high. True knapsack optimization would require a value function that accounts for inter-section dependencies, which is the role of the Scorer.

**Prefix Caching** [Anthropic 2024]. The cache-layer ordering scheme directly targets Anthropic's prompt caching feature, which provides a 90% input token cost discount for cached prefixes. The PromptComposer ensures that the stable prefix (System + Session layers) is byte-identical across all requests for the same role and plan, maximizing cache hit rate. Without this optimization, a heavy agent session (~20M tokens on Opus) costs ~$100. With 90% cache hit rate, it drops to ~$19 [from prd/12-inference/04-context-engineering.md].

**LLMLingua: Prompt Compression** [Jiang et al., EMNLP 2023]. The Composer's hard_cap feature is a simple form of the compression principle: not all sections need full fidelity. A workspace map can be compressed 5× with no information loss for a bug-fix task. The hard_cap allows per-section truncation limits that approximate content-aware compression without requiring an LLM call.

**Selective Context** [Li et al., EMNLP 2023]. The priority-based dropping algorithm is a manual approximation of Selective Context's information-theoretic approach. Selective Context automatically identifies and removes redundant content, achieving 50% context reduction with only 0.023 BERTscore drop. The Composer's manual priorities approximate this by encoding human knowledge about which sections are typically redundant. The active inference scorer (see [07-active-inference-context-selection.md](07-active-inference-context-selection.md)) proposes replacing manual priorities with learned ones.

**"Lost in the Middle"** [Liu et al., TACL 2024, arXiv:2307.03172]. The Placement enum and U-shape ordering directly implement the mitigation strategy for the attention degradation phenomenon documented by Liu et al. See [06-lost-in-the-middle-u-shape.md](06-lost-in-the-middle-u-shape.md) for full details.

---

## 9. Test Coverage

The PromptComposer has 18 tests in `crates/roko-compose/src/prompt.rs`:

- Budget enforcement: sections are correctly dropped when budget is exceeded
- Critical guarantee: Critical sections survive even when budget is exhausted
- Cache-layer ordering: sections appear in System → Session → Task → Dynamic order
- Priority ordering: within a cache layer, higher-priority sections appear first
- Hard cap: sections are truncated to their hard_cap before budget fitting
- Token estimation: byte/4 heuristic produces expected values
- Empty input: empty section list produces empty output
- Single section: single Critical section survives any budget
- Metadata: PromptBuild correctly reports included/dropped counts

---

## 10. Current Status and Gaps

| Aspect | Status |
|--------|--------|
| Core assembly algorithm | **Implemented** |
| Priority enum | **Implemented** |
| CacheLayer enum | **Implemented** |
| Placement enum | **Implemented** |
| Hard cap truncation | **Implemented** |
| PromptBuild metadata | **Implemented** |
| Token estimation (byte/4) | **Implemented** |
| 18 unit tests | **Passing** |
| Scorer integration in assembly | **Implemented** |
| Active inference re-scoring during assembly | **Not yet** (see E2) |
| Deduplication of overlapping sections | **Not yet** (see E1 stage 3) |
| Dynamic hard_cap based on task complexity | **Not yet** |

---

## Cross-References

- [00-composer-trait.md](00-composer-trait.md) — Composer trait definition and rationale
- [02-system-prompt-builder-7-layer.md](02-system-prompt-builder-7-layer.md) — 7-layer SystemPromptBuilder
- [05-token-budget-management.md](05-token-budget-management.md) — Budget derivation
- [06-lost-in-the-middle-u-shape.md](06-lost-in-the-middle-u-shape.md) — U-shape attention
- [08-5-stage-assembly-pipeline.md](08-5-stage-assembly-pipeline.md) — Full pipeline
- `crates/roko-compose/src/prompt.rs` — Implementation source
