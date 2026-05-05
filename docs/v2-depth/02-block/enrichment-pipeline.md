# Enrichment Pipeline

> Depth for [02-CELL.md](../../unified/02-CELL.md). The 13-step enrichment pipeline as a Pipeline Graph of Compose Cells, VCG budget allocation across sections, and token budget management.

---

## 1. Enrichment as a Pipeline Graph

The enrichment pipeline pre-computes context artifacts before agent sessions begin. Instead of having agents spend tokens discovering what they need, the pipeline generates 13 typed artifacts using the cheapest appropriate model for each step. In unified terms, this is a **Pipeline Graph**: a linear chain of Cells, each producing one Signal kind.

```
[PrdExtractCell] -> [BriefCell] -> [TaskCell] -> [DecomposeCell]
    -> [ResearchCell] -> [DependencyCell] -> [FixtureCell]
    -> [IntegrationCell] -> [VerifyCell] -> [ReviewCell]
    -> [TestCell] -> [InvariantCell] -> [ScribeCell]
```

Each Cell in the pipeline:
- Conforms to the **Compose** protocol (it assembles a prompt for its own LLM call, producing an artifact Signal).
- Has a typed output schema (e.g., `PrdExtractCell` outputs a Signal of kind `PrdExtract`).
- Uses the cheapest sufficient model for its step (Haiku for mechanical extraction, Sonnet for reasoning, Opus for deep research).
- Writes its output to disk as a file for inspectability and staleness checking.

### The 13 Steps as Cells

| # | Cell | Output Signal Kind | Model | Token Cost | Purpose |
|---|---|---|---|---|---|
| 1 | PrdExtractCell | PrdExtract | Haiku | ~$0.005 | Extract plan-relevant PRD sections |
| 2 | BriefCell | Brief | Sonnet | ~$0.02 | Generate What/Why/How task summaries |
| 3 | TaskCell | TaskSpec | Sonnet | ~$0.02 | Generate task specifications (TOML) |
| 4 | DecomposeCell | Decomposition | Sonnet | ~$0.02 | Step-by-step subtask breakdown |
| 5 | ResearchCell | Research | Opus | ~$0.08 | Deep research with citations |
| 6 | DependencyCell | DependencyManifest | Haiku | ~$0.005 | External dependency list |
| 7 | FixtureCell | FixtureManifest | Haiku | ~$0.005 | Test fixture requirements |
| 8 | IntegrationCell | IntegrationNotes | Sonnet | ~$0.02 | Cross-crate integration notes |
| 9 | VerifyCell | VerificationScript | Haiku | ~$0.005 | Invariant verification script |
| 10 | ReviewCell | ReviewTasks | Haiku | ~$0.005 | Review task assignments |
| 11 | TestCell | TestTasks | Haiku | ~$0.005 | Test task assignments |
| 12 | InvariantCell | Invariants | Sonnet | ~$0.02 | Invariant specifications |
| 13 | ScribeCell | ScribeTasks | Haiku | ~$0.005 | Documentation task assignments |

Total cost for full enrichment: ~$0.15 per plan. This $0.15 investment produces a ~33% improvement in downstream agent success rate (from ~45% without enrichment to ~78% with it).

### LLM Client Abstraction

Each enrichment Cell delegates to an LLM client for its generation step:

```rust
pub trait LlmClient: Send + Sync {
    fn complete(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<String>;
}
```

Four backend implementations exist: Claude (API), Codex, Cursor, and Ollama. The pipeline uses two modes: batch (for multi-artifact steps) and direct (for single-artifact steps).

---

## 2. Pipeline Execution Semantics

### Staleness Checking

Before running a step, the pipeline checks whether the output Signal already exists and is fresh:

```rust
fn is_stale(&self, step: &EnrichStep) -> bool {
    let output_path = self.output_dir.join(step.output_filename());
    if !output_path.exists() { return true; }
    let age = time_since_modified(&output_path);
    age > self.config.max_staleness  // default: 24 hours
}
```

If the output exists and was generated within 24 hours, the Cell is skipped. This prevents re-running expensive LLM calls when the pipeline is invoked multiple times (e.g., after a plan run failure and restart).

### Continue-on-Failure

The pipeline uses **continue-on-failure semantics**: each Cell runs regardless of whether previous Cells failed. A failed enrichment step is logged as a warning, and the agent receives whatever artifacts were successfully generated. Missing artifacts are simply absent from the prompt -- the downstream Compose Cell's priority-based dropping handles this gracefully.

```
Step 1 (Prd):        ok  -> prd-extract.md
Step 2 (Briefs):     ok  -> brief.md
Step 3 (Tasks):      FAIL (TOML repair failed)
Step 4 (Decompose):  ok  -> decomposition.md
Step 5 (Research):   ok  -> research.md
...
```

This is a deliberate design choice. The cost of retrying a failed step is low (one more LLM call), but the cost of blocking the entire pipeline is high (delayed agent start). The agent can often succeed without every artifact.

### TOML Repair

Steps that produce TOML output include a validation and repair pass. If the initial generation produces invalid TOML, the pipeline sends the parse error back to the LLM with instructions to fix it. This is a single retry -- if the repair also fails, the step is marked as failed and the pipeline continues. The one-retry policy prevents infinite LLM loops on malformed output.

```rust
fn validate_and_repair_toml(&self, step: &EnrichStep, raw: &str) -> Result<String> {
    match toml::from_str::<toml::Value>(raw) {
        Ok(_) => Ok(raw.to_string()),
        Err(e) => {
            let repair_prompt = format!("Fix this TOML syntax error:\n{e}\n\n{raw}");
            let repaired = self.client.complete(
                step.default_model(),
                "You fix TOML syntax errors. Return only valid TOML.",
                &repair_prompt,
            )?;
            toml::from_str::<toml::Value>(&repaired)?;
            Ok(repaired)
        }
    }
}
```

This is a simple form of CRAG (Corrective RAG, Yan et al. 2024): when generation confidence is low (parse failure), retry with corrective feedback.

---

## 3. Step Selection: Adaptive Retrieval

Not every task needs all 13 steps. A `StepSelector` determines which Cells to activate based on task complexity and role:

| Task Complexity | Role | Steps Run | Steps Skipped |
|---|---|---|---|
| Trivial | Any | Prd, Briefs | 11 others |
| Standard | Implementer | Prd, Briefs, Tasks, Decompose, Research | 8 others |
| Standard | Scribe | Prd, Scribe, Research | 10 others |
| Complex | Any | All 13 | None |

```rust
pub fn steps_for(complexity: Complexity, role: AgentRole) -> Vec<EnrichStep> {
    match (complexity, role) {
        (Trivial, _) => vec![Prd, Briefs],
        (Standard, Scribe) => vec![Prd, Scribe, Research],
        (Complex, _) => EnrichStep::ALL_ORDERED.to_vec(),
        _ => vec![Prd, Briefs, Tasks, Decompose, Research],
    }
}
```

This is Self-RAG (Asai et al. 2023) applied at the task level: the system decides WHEN to retrieve based on task complexity. A simple rename (Trivial) is classified as "no retrieval needed" -- only the PRD extract and brief are generated. A cross-crate integration (Complex) triggers full enrichment ("retrieval strongly needed").

---

## 4. Budget Allocation via VCG Auction

Once enrichment artifacts are generated, they become bids in the Compose protocol's VCG auction (or greedy knapsack, depending on `CompositionStrategy`).

### Three-Tier Budget Architecture

Token budgets are resolved through three stacked constraints:

**Tier 1 -- Static per-role budgets.** `budget_for(role)` returns a `PromptBudget` with per-section allocations. Each role gets a different distribution reflecting what context it needs most.

**Tier 2 -- Complexity-adaptive scaling.** `adjusted_budget_for(role, complexity)` overlays Tier 1:
- Trivial: Drop PRD, context, skills. Halve workspace_map, brief. ~70% reduction.
- Standard: No change.
- Complex: +50% workspace_map, +100% context, +50% file_context. ~40% increase.

**Tier 3 -- Context-window constraint.** The model's context window sets the absolute ceiling:
- Surgical (Haiku, Ollama): 4,000 tokens max
- Focused (Sonnet): 12,000 tokens max
- Full (Opus): 24,000 tokens max

The tightest constraint across all three tiers wins.

### The Differential Budget Principle

Different content types have different information density and compression tolerance (LLMLingua Budget Controller, Jiang et al. 2023):

| Content Type | Compression Tolerance | Budget Priority |
|---|---|---|
| Task description | 0% (never compress) | Critical |
| Role identity | 0% | Critical |
| Safety constraints | 0% | Critical |
| Gate errors | 5% | High |
| File context (source code) | 10-20% | High |
| PRD extract | 20-30% | Medium |
| Workspace map | 30-50% | Medium |
| Cross-plan context | 50%+ | Low |
| Learning pack | 50%+ | Low |

### Priority-Ordered Allocation Algorithm

```rust
fn resolve_budget_conflicts(sections: &mut [Section], total_budget: usize) {
    sections.sort_by_key(|s| s.priority);  // Critical first
    let mut remaining = total_budget;

    // Phase 1: Critical sections -- always included, truncated if necessary
    for s in sections.iter_mut().filter(|s| s.priority == Critical) {
        s.allocated = s.actual_tokens;
        remaining = remaining.saturating_sub(s.allocated);
    }

    // Phase 2: Remaining sections in priority order
    for s in sections.iter_mut().filter(|s| s.priority != Critical) {
        if remaining < s.min_tokens {
            // Not enough for a useful version -- skip entirely
            // (Sufficient Context finding: bad context is 6x worse than none)
            s.allocated = 0;
            continue;
        }
        let alloc = s.actual_tokens.min(s.max_tokens).min(remaining);
        if alloc < s.actual_tokens {
            s.content = truncate(&s.content, alloc, s.truncation_strategy);
        }
        s.allocated = alloc;
        remaining = remaining.saturating_sub(alloc);
    }
}
```

### The Min-Tokens Guard

Each section has a minimum useful token threshold. If the remaining budget cannot accommodate at least `min_tokens`, the section is skipped entirely rather than included in a uselessly truncated form. This implements the Sufficient Context finding (Joren et al., ICLR 2025): Gemma went from 10.2% incorrect with no context to 66.1% incorrect with **insufficient** context. Bad context is 6x worse than no context.

### Per-Section Truncation Strategies

Each section kind has a specific truncation strategy that preserves its most valuable content:

| Section | Truncation Strategy |
|---|---|
| Gate errors | Keep N most recent (LIFO) |
| File context | Keep imports + signatures, drop function bodies |
| PRD extract | Keep requirements + criteria, drop background |
| Workspace map | Keep top 2 directory levels, drop deeper nodes |
| Task brief | Keep What/How, drop Why/Context |
| Learning pack | Drop entirely if below min_tokens (2,000) |

---

## 5. Cost Attribution and Section Learning

### Per-Section Cost Attribution

After an agent completes a turn, cost is proportionally attributed to the sections that were included:

```rust
pub struct CostAttribution {
    pub turn_id: String,
    pub total_input_tokens: u64,
    pub total_cost_usd: f64,
    pub sections: Vec<SectionCost>,
    pub strategy: CompositionStrategy,
    pub vcg_payments: Vec<(String, f64)>,
}

pub struct SectionCost {
    pub section_name: String,
    pub bidder: BidderId,
    pub estimated_tokens: usize,
    pub token_fraction: f64,           // estimated_tokens / total_estimated
    pub attributed_cost_usd: f64,      // total_cost * token_fraction
    pub gate_passed: Option<bool>,     // stamped after gate runs
}
```

This closes the feedback loop between composition and learning. The `SectionEffectivenessRegistry` (from `roko-learn`) adjusts section priorities based on gate pass/fail, and with cost attribution, it can also distinguish high-value-per-token sections from high-value-but-expensive ones.

### The Learning Flow

```
Compose Cell produces prompt with manifest of included sections
  -> Agent runs, returns usage (input_tokens, cost_usd)
    -> CostAttribution::from_turn() distributes cost proportionally
      -> Gate Cell runs, produces Verdict
        -> attribution.stamp_gate_result(passed)
          -> For each section: bidder.update_with_cost(
               section, was_included, gate_passed, cost, tokens)
            -> Updated posteriors feed next composition
```

**Mori-diffs reality**: The cost attribution types (`CostAttribution`, `SectionCost`) are designed but not implemented. The `SectionEffectivenessRegistry` tracks inclusion/outcome correlations but has no cost signal. The `LearningBidder` updates Beta posteriors on `(included, passed)` but ignores cost entirely. See [09-COMPOSITION-AUCTION.md](../../mori-diffs/09-COMPOSITION-AUCTION.md) for the full gap.

---

## 6. Prompt Prefix Stability for Caching

Budget allocation must respect prefix stability. The system maximizes the byte-identical prefix across requests for the same role:

```
+-------------------------------------------------+
| System Prompt (role identity, conventions, tools)| <- ALWAYS cached (90% discount)
| ~800 tokens                                      |
+-------------------------------------------------+
| Workspace Map (changes only when files change)   | <- Cached within wave
| ~334 tokens                                      |
+-------------------------------------------------+
| Learning Pack (changes on playbook refresh)      | <- Cached within batch
| ~2,000 tokens                                    |
+-------------------------------------------------+
| PRD Extract (changes per plan)                   | <- Cached within plan
| ~712 tokens                                      |
+-------------------------------------------------+
| Task Description (unique per task)               | <- CACHE MISS boundary
| ~189 tokens                                      |
+-------------------------------------------------+
| Iteration Context (unique per attempt)           | <- Always miss
+-------------------------------------------------+
```

Rules for budget-aware prefix stability:
1. Never randomize section ordering -- deterministic priority sort only.
2. Freeze workspace map within a plan execution -- generate once, reuse for all tasks.
3. Cap learning pack within a batch -- do not re-extract playbooks mid-batch.
4. Normalize whitespace -- strip trailing spaces, normalize newlines to `\n`.
5. Sort tool definitions alphabetically -- BTreeMap, not HashMap.

### Cost Impact

For a typical 20-plan run with 80 agent spawns:

| Without cache alignment | With cache alignment |
|---|---|
| ~$100 on Opus (20M tokens) | ~$19 on Opus |
| Every request pays full price | 90% discount on prefix layers |

---

## 7. Disk Layout: Signals as Files

Enrichment artifacts are persisted as files on disk, not in-memory Signals. This is a practical choice that maps cleanly to the Store protocol:

```
.roko/plans/<plan-slug>/
  prd-extract.md
  brief.md
  tasks.toml
  decomposition.md
  research.md
  dependency-manifest.toml
  fixture-manifest.toml
  integration.md
  verify.sh
  review-tasks.toml
  test-tasks.toml
  invariants.md
  scribe-tasks.toml
```

Every artifact is diffable (`git diff`), inspectable (human-readable files), cacheable (staleness via mtime), and debuggable (if an agent fails, the input artifacts are readable).

Role-specific context injection packs assemble subsets of these artifacts:

| Role | Primary Pack | Additional Files |
|---|---|---|
| Implementer | execution-pack.md | brief.md |
| Architect | architect-pack.md | review-tasks.toml, verify-tasks.toml |
| Scribe | scribe-pack.md | scribe-tasks.toml, research.md |

This prevents agents from reading the entire context directory. Each agent opens exactly what it needs.

---

## 8. What This Enables

1. **Compound AI cost reduction**: 13 Haiku/Sonnet calls at ~$0.15 total produce context that enables a single Sonnet call to achieve higher task success than Opus without enrichment. The central compound AI insight: clever engineering > model scaling.

2. **Adaptive retrieval depth**: The StepSelector adjusts retrieval depth per task. Trivial tasks get minimal enrichment (2 steps, ~$0.01). Complex tasks get full enrichment (13 steps, ~$0.15). This prevents context rot on simple tasks while ensuring sufficient context on hard ones.

3. **Cache-aligned prefix reuse**: Deterministic section ordering and byte-identical prefixes enable 90% cache hit rate, turning a $100 Opus session into a $19 one.

4. **Graceful degradation**: Continue-on-failure semantics mean that a failed enrichment step never blocks the agent. The priority-based budget allocator handles missing sections by giving remaining budget to higher-priority content.

5. **Offline optimization**: Because all composition metadata (which sections included, how many tokens, what the gate outcome was) is persisted, the system can learn optimal budget allocations offline -- no controlled experiments required.

---

## 9. Feedback Loops

**Short loop (per-task)**: Compose Cell includes/drops sections based on budget -> Gate Cell produces verdict -> Beta posteriors updated -> next composition adjusts bids.

**Medium loop (per-plan)**: Cost attribution distributes actual LLM costs to sections -> cost-effectiveness ratios inform VCG bids -> cheap-and-effective sections get higher bids.

**Long loop (daily)**: Leave-one-out influence analysis measures per-section value from natural variation -> budget allocations adjusted by +20% (valuable) or -50% (harmful) -> persisted to `.roko/learn/budget-allocations.json`.

---

## 10. Open Questions

1. **Parallel enrichment**: The pipeline currently runs sequentially. Steps 1-5 have dependencies (Briefs depends on PRD extract, Decompose depends on Briefs), but steps 6-13 are largely independent. A parallel executor could cut enrichment latency by 3-4x for Complex tasks.

2. **Learned step selection**: The `StepSelector` uses static rules. A learned selector that tracks which steps actually improve gate pass rates per task category could drop unnecessary steps and save LLM costs.

3. **Cost tracking per step**: No cost tracking per enrichment step exists. Without it, the system cannot identify which enrichment Cells are cost-effective and which waste money.

4. **Budget prediction**: The TALE framework (Hu et al. 2025) showed that models can predict needed token budget before generation, reducing usage by 68.9%. A `BudgetPredictor` that estimates context needs per task is designed but not implemented.

5. **Context anxiety**: Claude proactively summarizes when it perceives it is near context limits, even when it is not. The mitigation (always request maximum context window from the provider) is a workaround, not a fix. The root cause is the model's training to manage its own context, which conflicts with scaffold-managed composition.
