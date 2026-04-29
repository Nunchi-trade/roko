# Compose Protocol and Builder

> Depth for [02-CELL.md](../../unified/02-CELL.md). How Cells implementing the Compose protocol assemble context under budget constraints, and how the 9-layer system prompt builder works as a Pipeline Graph of Compose Cells.

---

## 1. The Compose Protocol in the Cell Model

The Compose protocol is one of the 9 protocols a Cell can conform to. Its signature (defined in [02-CELL.md](../../unified/02-CELL.md) S2.5) accepts bids from multiple context sources, a budget constraint, and returns a composed Signal with VCG payments:

```rust
#[async_trait]
pub trait ComposeProtocol: Cell {
    async fn compose(
        &self,
        bids: Vec<ComposeBid>,
        budget: &ComposeBudget,
        ctx: &ComposeContext,
    ) -> Result<ComposeResult>;
}
```

This trait maps to the legacy `Composer` trait in `roko-core`, which accepted `(&[Engram], &Budget, &dyn Scorer, &Context)`. The key design shift: the new protocol replaces the `&dyn Scorer` parameter with pre-scored `ComposeBid` values. Scoring is no longer an input to composition -- it is upstream. A Score Cell produces bids; a Compose Cell assembles them. This separation lets the runtime compose any Score Cell with any Compose Cell without coupling.

### Why Compose Takes Bids, Not Raw Signals

The legacy `Composer` trait accepted raw Engrams plus a scorer reference, coupling scoring and composition into a single call. The Compose protocol separates these concerns:

1. **Score Cells** (upstream) evaluate each candidate Signal against the current context and produce `ComposeBid` values with a `value: f64` and `effect: BetaPosterior` (historical gate-pass correlation).
2. **Compose Cells** (this protocol) receive the bids and allocate budget.

This separation enables composability: different Score Cells can feed the same Compose Cell. A priority-based scorer produces deterministic compositions; an active-inference scorer (EFE-based) produces exploratory ones. The caller wires the Score -> Compose edge in the Graph; the Compose Cell does not know which scorer produced the bids.

### ComposeBid Structure

```rust
pub struct ComposeBid {
    pub bidder: BidderId,
    pub section: ComposeSection,
    pub value: f64,             // truthful bid under VCG
    pub token_cost: u32,        // estimated tokens
    pub effect: BetaPosterior,  // historical (included, gate_passed) correlation
}
```

Each bid carries:

- **bidder** -- which context source (Task, Code, Research, Episode, Heuristic, Tool, Safety, Neuro). See the 8+ built-in bidders in [02-CELL.md](../../unified/02-CELL.md) S2.5.
- **section** -- the content, its name, its kind (`ComposeSectionKind`), and references to the source Signals for lineage tracking.
- **value** -- the bid value. Under VCG, truthful bidding is a dominant strategy (Vickrey 1961), so bidders report their true marginal value without strategic inflation.
- **token_cost** -- estimated tokens, using the `len() / 4` byte heuristic (~4 bytes per token for English/code, empirically calibrated against cl100k_base).
- **effect** -- a `BetaPosterior(alpha, beta)` tracking how often this section's inclusion correlated with gate success. After each task, the runtime calls `alpha += 1` (if gate passed) or `beta += 1` (if gate failed), then the posterior mean `alpha / (alpha + beta)` is the learned inclusion-effectiveness.

### Novelty Attenuation

Boilerplate context that appears in every prompt gradually loses bid strength:

```
effective_value = stated_value * (1 / (1 + ln(freq)))
```

where `freq` is the number of recent compositions that included this exact section content. This creates room for novel context by penalizing repetitive inclusions -- a simple form of the demurrage principle applied to composition rather than storage.

---

## 2. The Compose Cell Implementations

### 2.1 PromptComposer: The Greedy Knapsack Cell

The primary Compose Cell implementation (`roko-compose/src/prompt.rs`, 772 lines, 18 tests). It implements a greedy knapsack with priority partitioning:

```rust
// Pseudocode for PromptComposer::compose()
fn compose(bids, budget, ctx) -> ComposeResult {
    // 1. Decode bids into PromptSections
    let sections = bids.map(|b| PromptSection {
        name: b.section.name,
        content: b.section.content,
        priority: priority_from_kind(b.section.kind),
        cache_layer: cache_layer_from_kind(b.section.kind),
        placement: placement_from_kind(b.section.kind),
        hard_cap: b.section.hard_cap,
    });

    // 2. Partition: Critical vs Optional
    let (critical, optional) = sections.partition(|s| s.priority == Critical);

    // 3. Sort optional by (cache_layer ASC, priority DESC, score DESC)
    optional.sort_by(|a, b| (a.cache_layer, Reverse(b.priority), Reverse(b.score)));

    // 4. Greedy include under budget
    let mut remaining = budget.max_tokens;
    let mut included = Vec::new();

    for s in critical {
        let tokens = estimate_tokens(&s.content);
        if tokens <= remaining {
            included.push(s);
            remaining -= tokens;
        } else {
            // Critical sections: truncate to fit, NEVER drop
            s.content = truncate_to_tokens(s.content, remaining);
            included.push(s);
            remaining = 0;
        }
    }

    for s in optional {
        let tokens = estimate_tokens(&s.content);
        if tokens <= remaining {
            included.push(s);
            remaining -= tokens;
        }
        // else: drop silently
    }

    // 5. U-shape reorder by Placement (Start, Middle, End)
    // 6. Concatenate with section headers and cache-layer markers
    // 7. Return composed Signal with lineage from included bids
}
```

Key invariants:
- **Critical sections survive.** `SystemInstruction`, `TaskDescription`, and `SafetyConstraint` kinds map to `SectionPriority::Critical` and are truncated before being dropped.
- **Deterministic output.** Same inputs always produce same output. This is critical for prefix caching -- non-deterministic composition defeats cache alignment.
- **Synchronous execution.** Composition is CPU-bound. No I/O, no LLM calls. All candidates are pre-gathered by upstream Cells.

### 2.2 VcgComposer: The Auction Cell

The VCG auction path (`roko-compose/src/auction.rs`). Built, exported, unit-tested (5 tests), but **not called from any runtime path** as of this writing.

The VCG (Vickrey-Clarke-Groves) mechanism has a key property: **truthful bidding is a dominant strategy**. Each bidder pays not their bid, but the externality they impose on other bidders. This means bidders have no incentive to inflate or deflate their stated values.

```rust
// VCG allocation (simplified)
fn vcg_allocate(
    bids: Vec<VcgBid>,
    budget: TokenBudget,
    affect: &AffectModulation,
) -> VcgAllocation {
    // 1. Sort by value-density = value / token_cost
    // 2. Greedy fill under budget (same as knapsack)
    // 3. For each winner w:
    //    payment(w) = welfare_without_w - welfare_without_w_but_with_next_best
    //    (the externality w imposes on the runners-up)
    // 4. Return winners, payments, displaced sections
}
```

**Current status (mori-diffs reality)**: The greedy path in `PromptComposer::compose()` dominates at runtime. It is structurally identical to VCG's greedy allocation but without payment computation or externality tracking. The planned activation path uses `CompositionStrategy::auto_select()`: when bidder observation counts exceed 10 (enough to have informative Beta posteriors), the runtime switches from greedy to full VCG. Below that threshold, VCG payments are meaningless because the posteriors are uninformative.

### 2.3 How They Compose as a Graph

Both implementations are Cells conforming to the Compose protocol. The runtime can wire either into a Graph:

```toml
# graph.toml -- composition subgraph
[[cells]]
id = "task-scorer"
kind = "ScoreCell"
config.strategy = "priority"

[[cells]]
id = "composer"
kind = "PromptComposer"           # or "VcgComposer"
config.budget_tokens = 12000

[[edges]]
from = "task-scorer"
to = "composer"
```

Because both are Cells with the same protocol, swapping between them is a TOML edit, not a code change.

---

## 3. The 9-Layer System Prompt Builder as a Pipeline Graph

The `SystemPromptBuilder` (`roko-compose/src/system_prompt_builder.rs`, 726 lines, 12 tests) constructs agent system prompts through 9 ordered layers. In unified terms, this builder is a **Pipeline Graph** -- a linear chain of Cells, each contributing one layer.

### The 9 Layers

| Layer | Name | Cache Tier | Compose Section Kind | Priority | Placement |
|---|---|---|---|---|---|
| 1 | Role Identity | System | SystemInstruction | Critical | Start |
| 2 | Conventions | System | SafetyConstraint | Critical | Start |
| 3a | Domain Context | Session | ResearchContext | Normal | Middle |
| 3b | Assembled Context | Session | CodeContext | Normal | Middle |
| 3c | Pheromone Signals | Session | Custom("pheromone") | Normal | Middle |
| 4 | Task Context | Task | TaskDescription | Critical | Start |
| 5 | Tool Instructions | System | ToolDocumentation | High | Start |
| 6a | Relevant Techniques | Task | HeuristicGuidance | Normal | End |
| 6b | Anti-Patterns | Task | SafetyConstraint | High | End |
| 8 | Affect Guidance | Dynamic | Custom("affect") | Low | End |

Each layer maps to a `ComposeSectionKind`, inherits a priority and placement, and belongs to a cache tier. The cache tier determines ordering within the assembled prompt for maximum prefix reuse.

### Cache Alignment: Why Layer Order Matters

The output is ordered by cache tier, not by layer number:

```
<!-- roko:layer:system -->
Layer 1: Role Identity        (identical across ALL tasks for this role)
Layer 2: Conventions           (identical across ALL tasks)
Layer 5: Tool Instructions     (identical across ALL tasks for this role)

<!-- roko:layer:session -->
Layer 3a: Domain Context       (stable within a plan execution)
Layer 3b: Assembled Context    (stable within a plan)
Layer 3c: Pheromone Signals    (stable within a plan, updated per-task)

<!-- roko:layer:task -->
Layer 4: Task Context          (unique per task)
Layer 6a: Techniques           (unique per task)
Layer 6b: Anti-Patterns        (unique per task, updated per-iteration)

<!-- roko:layer:dynamic -->
Layer 8: Affect Guidance       (unique per turn)
```

The System tier is byte-identical across all tasks for the same role. Anthropic's prompt caching gives a 90% token cost discount on cache hits. For a 20-plan run with 80 agent spawns, this prefix hits the cache on 79 of 80 requests -- saving approximately $80 on Opus.

**BTreeMap requirement**: All serialization in cacheable layers uses `BTreeMap` for deterministic key ordering. `HashMap` produces non-deterministic byte sequences, defeating prefix caching.

### The Builder API

```rust
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
    // Fluent builder methods
    pub fn new() -> Self;
    pub fn role_identity(mut self, content: &str) -> Self;
    pub fn conventions(mut self, content: &str) -> Self;
    // ... one method per layer

    /// Flat string with cache-layer markers.
    pub fn build(&self) -> String;

    /// Structured PromptSections for budget fitting by PromptComposer.
    pub fn build_sections(&self) -> Vec<PromptSection>;
}
```

The dual output is important: `build()` produces a ready-to-use string for simple dispatch paths; `build_sections()` produces structured sections that the PromptComposer can individually score, prioritize, and budget-fit.

### RoleSystemPromptSpec: The Orchestration Wrapper

The orchestrator (`roko-cli/src/orchestrate.rs`) does not call `SystemPromptBuilder` directly. It uses `RoleSystemPromptSpec`, which wraps the builder with role-specific budget constraints:

```rust
pub struct RoleSystemPromptSpec {
    pub role: AgentRole,
    pub builder: SystemPromptBuilder,
}

impl RoleSystemPromptSpec {
    pub fn build_with_context_window(&self, context_window: usize) -> String;
    pub fn compose_with_budget(&self, budget: &PromptBudget) -> String;
}
```

This is a Pipeline Graph in disguise: score (via role-specific budget weights) -> compose (via PromptComposer) -> format (via cache-layer markers).

---

## 4. Role Templates as Signal Kinds

Each of the 12 roles (Strategist, Implementer, Architect, Auditor, QuickReviewer, Scribe, Critic, AutoFixer, IntegrationTester, Refactorer, Researcher, Conductor) is defined by a template that specifies:

1. **Identity text** -- the role's persona (Layer 1 content).
2. **Budget allocation** -- how many tokens each section gets via `budget_for(role)`.
3. **Context tier** -- which model class to use by default.
4. **Section emphasis** -- which sections receive more or fewer tokens.

In unified terms, each role template is a **Signal of kind `RoleTemplate`** that configures the Compose pipeline:

```rust
pub const fn budget_for(role: AgentRole) -> PromptBudget {
    match role {
        Implementer => PromptBudget {
            plan: 50_000, workspace_map: 20_000, prd2: 12_000,
            context: 4_000, brief: 8_000, reviews: 3_000,
            instructions: 4_000, file_context: 8_000, skills: 8_000,
        },
        Strategist => PromptBudget {
            // Zero file_context -- Strategists plan but never code
            file_context: 0, ..similar
        },
        Scribe => PromptBudget {
            // Largest prd2 -- documentation must cite specs accurately
            prd2: 16_000, ..similar
        },
        // ... 9 other roles
    }
}
```

Key asymmetries:
- **Implementer** gets the most `file_context` (8K) and `skills` (8K) because it writes code and needs learned playbook rules.
- **Strategist** gets zero `file_context` because it plans but never touches code. ETH Zurich's AGENTS.md study showed that irrelevant instructions decrease success by ~3% per instruction, so excluding code context from the Strategist is a direct application.
- **Scribe** gets the most `prd2` (16K) because documentation must accurately cite specifications.

### Complexity-Adaptive Budgets

The base budgets are scaled by task complexity:

```rust
pub fn adjusted_budget_for(role: AgentRole, complexity: Complexity) -> AdjustedBudget {
    let base = budget_for(role);
    match complexity {
        Trivial   => /* Drop PRD, context, skills. Halve workspace_map, brief. ~70% reduction */,
        Standard  => base,
        Complex   => /* +50% workspace_map, +100% context, +50% file_context */,
    }
}
```

This is the three-tier budget architecture: static per-role (Tier 1), complexity-adaptive (Tier 2), context-window-constrained (Tier 3: Surgical 4K / Focused 12K / Full 24K). The tightest constraint wins.

---

## 5. The Empirical Basis

### The --bare Flag Experiment

During Mori development (2025-2026), an experiment compared agent success with and without system prompts:

| Condition | Task Success Rate |
|---|---|
| `claude --bare` (no system prompt) | 15-25% |
| `claude` (with full system prompt) | 60-75% |

A 3-4x quality gap from the system prompt alone. This is the single highest-leverage scaffold investment.

### Per-Section Token Analysis

From prompt-log analysis of successful and failed tasks:

| Section | Avg Tokens | % of Prompt | Pass Rate When Present |
|---|---|---|---|
| Task Brief ("Your Assignment") | 189 | 4% | **71%** |
| Strategist Brief | 491 | 10% | **72%** |
| PRD Extract | 712 | 15% | 67% |
| Workspace Map | 334 | 7% | 64% |
| Learning Pack | 2,347 | 49% | 61% |
| Cross-Plan Context | 243 | 5% | 55% |

Key insight: **Task Brief and Strategist Brief** have the highest pass rates at the lowest token costs. **Learning Pack** dominates at 49% of tokens but has the lowest pass rate -- it may be adding noise (context rot). **Cross-Plan Context** has the lowest pass rate and may actively hurt simple tasks. These findings directly motivated complexity-adaptive budgets: Trivial tasks drop Learning Pack and Cross-Plan Context entirely.

---

## 6. What This Enables

1. **Composable context assembly**: Any Score Cell can feed any Compose Cell. New context sources are wired by adding a bidder and an edge in the Graph, not by modifying the Composer.

2. **Automatic cost optimization**: VCG payments reveal the marginal value of each section. Sections that consistently displace higher-value content get lower bids over time (via `effect: BetaPosterior` updates). Sections that waste tokens without improving gate pass rates fade out.

3. **Role-specific prompt tailoring**: Each role receives exactly the context it needs. The ETH Zurich finding (~3% penalty per irrelevant instruction) is structurally addressed by per-role budget allocation.

4. **Cache-aligned cost reduction**: By ordering layers by cache tier, the system achieves ~90% prefix cache hit rate on repeated agent spawns, reducing a $100 Opus session to ~$19.

5. **Learned budget allocation**: The `BetaPosterior` on each bid tracks section effectiveness. Over time, the system learns which sections help and which hurt, without requiring controlled experiments.

---

## 7. Feedback Loops

```
Compose Cell assembles prompt
  -> Agent executes with composed context
    -> Gate Cell produces Verdict (pass/fail + reward)
      -> For each bid that was included:
           bid.effect.alpha += 1 if passed, bid.effect.beta += 1 if failed
        -> Updated posteriors feed next composition
          -> VCG auto-activates when posteriors become informative (>= 10 obs)
            -> Payments reveal marginal value
              -> Budget reallocation adjusts per-role allocations
```

This is a **predict-publish-correct Loop** applied to composition. The Compose Cell predicts which sections will help the agent succeed (by including them). The Verify Cell publishes the outcome. The `BetaPosterior` corrects the prediction. Learning is structural -- it emerges from the same Bus/Store fabric that carries all other Signals.

A second feedback loop operates at a longer timescale:

```
Budget learning (daily):
  influence(S) = pass_rate_with_S - pass_rate_without_S
  -> Valuable sections (influence > 0.05): +20% allocation
  -> Harmful sections (influence < -0.05): -50% allocation or drop
  -> Freed tokens redistributed to valuable sections
  -> Updated allocations persisted to .roko/learn/budget-allocations.json
```

This is leave-one-out influence measurement, enabled by natural variation: tasks where a section was dropped due to budget pressure serve as the "without S" condition.

---

## 8. Open Questions

1. **VCG activation threshold**: The 10-observation threshold for switching from greedy to VCG is a magic number. Should it be configurable per-bidder, or should the system use the posterior's variance (switch when variance < threshold, meaning the estimate is precise enough)?

2. **Inter-section interaction effects**: The current model treats sections as independent (each bid's value is independent of which other sections are included). In reality, two sections with overlapping information have diminishing marginal returns when both are included. The active-inference scoring approach (EFE-based) can theoretically capture these interactions, but it is not yet wired.

3. **Compression vs. dropping**: When a section does not fit the budget, the current system either truncates or drops. It never compresses (via LLMLingua-style token pruning). Adding a Compose Cell that compresses rather than drops could recover value from budget-constrained sections. The `CompressionBudgetController` is designed but not implemented.

4. **Dynamic layer ordering**: Research suggests that placing the task directive last (after grounding context) outperforms placing it first (Anthropic 2025). The builder uses a fixed canonical order. A `LayerOrderPolicy` that learns optimal ordering per task category is designed but not implemented.

5. **Mori-diffs gap: VCG never called at runtime**: The VCG auction is built, tested, and exported, but `PromptComposer::compose()` uses the greedy path exclusively. The `CompositionStrategy` design (auto-select between greedy and VCG based on observation count) is specified but not wired. See [09-COMPOSITION-AUCTION.md](../../mori-diffs/09-COMPOSITION-AUCTION.md) for the full gap analysis.
