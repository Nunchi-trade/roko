# 13 — Current Status and Gaps

> Layer 2 Scaffold — Synapse Architecture
> Status: Comprehensive status report as of 2026-04-11
> Canonical source: `crates/roko-compose/src/`


> **Implementation**: Shipping

---

## Abstract

This document provides a comprehensive accounting of what is built, what is scaffolded, and what remains to be implemented in the Roko composition layer. The `roko-compose` crate contains approximately 4,400 lines of Rust across 12 modules, with 47+ tests. The core prompt assembly pipeline is fully operational and wired into the orchestration loop. The advanced features — active inference scoring, VCG attention auction, predictive foraging, full HDC-based deduplication — remain as specified designs awaiting implementation.

---

## 1. Crate Structure

```
crates/roko-compose/src/
├── lib.rs                          # Crate root, exports
├── prompt.rs                       # PromptComposer, PromptSection (772 lines, 18 tests)
├── system_prompt_builder.rs        # 7-layer SystemPromptBuilder (726 lines, 12 tests)
├── scorer.rs                       # SectionScorer (167 lines, 6 tests)
├── role_prompts.rs                 # RoleSystemPromptSpec (462 lines)
├── budget.rs                       # Complexity-adaptive budgets (270 lines)
├── context_provider.rs             # ContextTier, ContextSource (62.6KB)
├── context_assembler.rs            # ContextAssembler, PadState (52.1KB)
├── symbol_resolver.rs              # Symbol resolution
├── task_brief.rs                   # TaskBriefGenerator
├── templates/
│   ├── mod.rs                      # RolePromptTemplate trait (256 lines)
│   └── common.rs                   # PromptBudget, budget_for() (347 lines)
└── enrichment/
    ├── mod.rs                      # EnrichmentPipeline exports (48 lines)
    ├── step.rs                     # 13 EnrichStep variants (365 lines)
    └── pipeline.rs                 # EnrichmentPipeline<C> (774 lines)
```

---

## 2. What Is Built (Operational)

### 2.1 Core Assembly

| Component | File | Lines | Tests | Status |
|-----------|------|-------|-------|--------|
| PromptComposer (Composer trait impl) | prompt.rs | 772 | 18 | **Wired** into orchestrate.rs |
| SystemPromptBuilder (7-layer) | system_prompt_builder.rs | 726 | 12 | **Wired** via RoleSystemPromptSpec |
| SectionScorer (static priorities) | scorer.rs | 167 | 6 | **Wired** into PromptComposer |
| RoleSystemPromptSpec (12 roles) | role_prompts.rs | 462 | — | **Wired** into orchestrate.rs |
| PromptBudget (per-role allocation) | templates/common.rs | 347 | — | **Wired** via budget_for() |
| Complexity-adaptive budgets | budget.rs | 270 | — | **Wired** via adjusted_budget_for() |
| ContextTier (Surgical/Focused/Full) | context_provider.rs | — | — | **Wired** via from_task_and_model() |
| ContextSource tracking | context_provider.rs | — | — | **Wired** into context assembly |
| ContextAssembler (gather + rank + compress) | context_assembler.rs | — | — | **Wired** into orchestrate.rs |
| PadState struct | context_assembler.rs | — | — | **Built**, wired as optional |
| Enrichment pipeline (13 steps) | enrichment/ | 1,187 | — | **Built**, staleness + TOML repair |
| Cache alignment markers | prompt.rs | — | — | **Built** (roko:layer:N) |
| Placement enum (Start/Middle/End) | prompt.rs | — | — | **Built**, U-shape ordering |

### 2.2 What "Wired" Means

Every component listed as "Wired" is called from `roko-cli/src/orchestrate.rs` during `roko plan run`. The data flow:

```
orchestrate.rs
  → RoleSystemPromptSpec::for_role(task.role)
    → SystemPromptBuilder (7 layers)
      → PromptComposer::compose()
        → SectionScorer::score()
        → Budget enforcement
        → U-shape Placement
      → Final system prompt string
  → ContextAssembler::gather()
    → Knowledge store query
    → Episode store query
    → File context read
    → Signal log read
    → Rank + Compress
  → Agent dispatch with assembled prompt
```

### 2.3 Test Summary

| Module | Test Count | Coverage Focus |
|--------|-----------|---------------|
| prompt.rs | 18 | Budget enforcement, priority dropping, cache ordering, U-shape, token estimation |
| system_prompt_builder.rs | 12 | Layer ordering, cache markers, affect guidance, empty layers |
| scorer.rs | 6 | Priority scoring, recency decay, novelty, reputation |
| **Total** | **36+** | |

Additional tests exist in context_provider.rs, context_assembler.rs, and enrichment modules.

---

## 3. What Is Scaffolded (Designed, Partially Built)

| Feature | Sub-doc | Code State | Blocker |
|---------|---------|-----------|---------|
| Active inference scoring (EFE) | [07](07-active-inference-context-selection.md) | PadState exists, scorer interface exists | Needs episode history query + belief change |
| HDC-based deduplication | [08](08-5-stage-assembly-pipeline.md) §4 | HDC exists in bardo-primitives, compress() exists | D16 in 12a plan: wire HDC into dedup |
| Affect persistence + decay | [12](12-affect-modulated-retrieval.md) §4 | PadState struct exists | F9 in 12a plan: persist to .roko/daimon/ |
| Neuro injection into context | [08](08-5-stage-assembly-pipeline.md) §2 | ContextAssembler queries KnowledgeStore | E6 in 12a plan: bridge roko-neuro |
| Dynamic token budget from outcomes | [05](05-token-budget-management.md) §5 | ExperimentStore exists | A/B test results needed |
| Dominance modulation | [12](12-affect-modulated-retrieval.md) §2.3 | PadState has dominance field | No appraisal triggers wired |

---

## 4. What Is Not Yet Built (Specified Only)

| Feature | Sub-doc | Specification | Blocker |
|---------|---------|-------------|---------|
| VCG attention auction | [10](10-vcg-attention-auction.md) | Full spec with 8 bidding subsystems | Requires all subsystems to implement bid() |
| Predictive foraging MVT | [09](09-predictive-foraging-mvt.md) | Stopping rule + calibration spec | Requires search iteration tracking |
| Contextual influence value | [11](11-distributed-context-engineering.md) §7 | Leave-one-out per-section measurement | Requires controlled evaluation framework |
| DSPy-style prompt optimization | [03](03-role-templates.md) §10 | Learnable prompt parameters | Requires evaluation metric + compiler |
| RAGAS evaluation | [11](11-distributed-context-engineering.md) §6 | Three-metric evaluation | Requires evaluation pipeline |
| Self-RAG adaptive retrieval | [04](04-enrichment-pipeline-13-step.md) §4 | Step selection by complexity/role | Step selector exists, learning not yet |
| Semantic density ranking | [06](06-lost-in-the-middle-u-shape.md) §7 | LongLLMLingua-style reordering | Requires per-chunk information density scoring |
| Level 3 network context engineering | [11](11-distributed-context-engineering.md) §2.3 | Agent mesh context sharing | Requires agent mesh infrastructure |

---

## 5. Implementation Priority (from 12a-cognitive-layer.md)

The 12a plan specifies the implementation order for composition features:

### Layer 1 (Core Cognitive — Current Priority)

| Item | What | Sub-doc |
|------|------|---------|
| E1 | 5-stage pipeline: Query → Score → Deduplicate → Budget → Format | [08](08-5-stage-assembly-pipeline.md) |
| E2 | Active inference scoring (EFE formula) | [07](07-active-inference-context-selection.md) |
| E3 | Attention-curve positioning (U-shape in retrieved context) | [06](06-lost-in-the-middle-u-shape.md) |
| E4 | Affect-modulated retrieval (PAD biases retrieval) | [12](12-affect-modulated-retrieval.md) |
| E5 | Dynamic token budget (fit within model context window) | [05](05-token-budget-management.md) |
| E6 | Neuro injection (bridge roko-neuro and roko-compose) | [08](08-5-stage-assembly-pipeline.md) |

### Depends On (From Other Cognitive Subsystems)

| Dependency | What It Enables |
|-----------|----------------|
| D7-D9 (Knowledge types + storage) | E1 Stage 1 queries, E6 Neuro injection |
| D12-D13 (HDC encoding + index) | E1 Stage 1 HDC search, E1 Stage 3 dedup |
| F1 (PadVector struct) | E4 Affect-modulated retrieval |
| F2-F5 (Daimon affect model) | Full PAD appraisal + decay |

---

## 6. Gap Analysis: What Would Make the Biggest Difference

Based on the empirical data from Mori development and the academic evidence:

### 6.1 Highest Impact Gaps

1. **Active inference scoring (E2).** Replaces hand-tuned priorities with learned, task-adaptive scoring. Expected improvement: 10-15% gate pass rate increase for novel task types where static priorities are wrong.

2. **HDC-based deduplication (D16 → E1 Stage 3).** Prevents cluster domination in knowledge retrieval. Expected improvement: 5-10% quality increase on tasks with many similar knowledge entries.

3. **Affect persistence and decay (F9).** Enables cross-session learning from emotional state. Expected improvement: reduced thrashing after failures (the agent "remembers" it is in a cautious state).

### 6.2 Medium Impact Gaps

4. **Neuro injection (E6).** Bridges the knowledge store into context assembly. Expected improvement: 8-12% gate pass rate increase for tasks with relevant knowledge entries.

5. **Dynamic token budget (E5).** Adapts budget allocation based on historical outcomes. Expected improvement: 15-30% token reduction with neutral or positive quality change.

6. **Predictive foraging MVT (stopping rule).** Optimizes search termination. Expected improvement: 10-20% reduction in retrieval latency with no quality loss.

### 6.3 Lower Impact (Long-Term)

7. **VCG attention auction.** Principled allocation for multi-subsystem competition. Impact depends on number of active subsystems.

8. **RAGAS evaluation pipeline.** Enables measurement-driven optimization. Impact is indirect but foundational.

9. **Level 3 network context engineering.** Requires agent mesh infrastructure. Impact at scale.

---

## 7. Academic Citation Index

All citations referenced across the 03-composition sub-docs:

| # | Citation | Used In |
|---|----------|---------|
| 1 | Lewis et al. (2020), RAG | [08](08-5-stage-assembly-pipeline.md) |
| 2 | Gao et al. (2023), Modular RAG Survey | [00](00-composer-trait.md), [04](04-enrichment-pipeline-13-step.md), [08](08-5-stage-assembly-pipeline.md) |
| 3 | Wei et al. (2022), Chain-of-Thought Prompting | [06](06-lost-in-the-middle-u-shape.md) (background) |
| 4 | Kojima et al. (2022), Zero-Shot CoT | [06](06-lost-in-the-middle-u-shape.md) (background) |
| 5 | Yao et al. (2022), ReAct | [02](02-system-prompt-builder-7-layer.md) |
| 6 | Shinn et al. (2023), Reflexion | [02](02-system-prompt-builder-7-layer.md) |
| 7 | Yao et al. (2023), Tree of Thoughts | [06](06-lost-in-the-middle-u-shape.md) (background) |
| 8 | Wang et al. (2023), Plan-and-Solve | [02](02-system-prompt-builder-7-layer.md) |
| 9 | Liu et al. (2023), "Lost in the Middle" [arXiv:2307.03172] | [06](06-lost-in-the-middle-u-shape.md), [01](01-prompt-composer.md), [08](08-5-stage-assembly-pipeline.md) |
| 10 | Jiang et al. (2023), LLMLingua [EMNLP] | [05](05-token-budget-management.md), [06](06-lost-in-the-middle-u-shape.md) |
| 11 | Li et al. (2023), Selective Context [EMNLP] | [01](01-prompt-composer.md), [05](05-token-budget-management.md), [06](06-lost-in-the-middle-u-shape.md) |
| 12 | Khattab et al. (2023), DSPy | [00](00-composer-trait.md), [04](04-enrichment-pipeline-13-step.md) |
| 13 | Gao et al. (2022), HyDE | [06](06-lost-in-the-middle-u-shape.md) (background) |
| 14 | Ma et al. (2023), Rewrite-Retrieve-Read | [06](06-lost-in-the-middle-u-shape.md) (background) |
| 15 | Zheng et al. (2023), Step-Back Prompting | [02](02-system-prompt-builder-7-layer.md) |
| 16 | Anthropic (2024), Prompt Caching | [01](01-prompt-composer.md), [05](05-token-budget-management.md) |
| 17 | Willard & Louf (2023), Structured Generation | [06](06-lost-in-the-middle-u-shape.md) (background) |
| 18 | Sumers et al. (2023), CoALA | [00](00-composer-trait.md), [07](07-active-inference-context-selection.md) |
| 19 | Asai et al. (2023), Self-RAG | [04](04-enrichment-pipeline-13-step.md) |
| 20 | Yan et al. (2024), CRAG | [04](04-enrichment-pipeline-13-step.md) |
| 21 | Friston (2006, 2010, 2022), Free Energy Principle | [07](07-active-inference-context-selection.md) |
| 22 | Friston et al. (2015), Active Inference & Epistemic Value | [07](07-active-inference-context-selection.md) |
| 23 | Zaharia et al. (2024), Compound AI Systems [BAIR] | [00](00-composer-trait.md), [04](04-enrichment-pipeline-13-step.md), [11](11-distributed-context-engineering.md) |
| 24 | Charnov (1976), Marginal Value Theorem | [09](09-predictive-foraging-mvt.md) |
| 25 | Pirolli & Card (1999), Information Foraging Theory | [09](09-predictive-foraging-mvt.md) |
| 26 | Hills et al. (2012), Cognitive Foraging | [09](09-predictive-foraging-mvt.md) |
| 27 | Vickrey (1961), Second-Price Auctions | [10](10-vcg-attention-auction.md) |
| 28 | Clarke (1971), Multipart Pricing | [10](10-vcg-attention-auction.md) |
| 29 | Groves (1973), Incentives in Teams | [10](10-vcg-attention-auction.md) |
| 30 | Simon (1971), Attention Economics | [10](10-vcg-attention-auction.md) |
| 31 | Karpathy (2025), Context Engineering | [11](11-distributed-context-engineering.md) |
| 32 | Lee et al. (2026), Meta-Harness [arXiv:2603.28052] | [11](11-distributed-context-engineering.md) |
| 33 | Shahul Es et al. (2024), RAGAS [EACL] | [11](11-distributed-context-engineering.md) |
| 34 | Saad-Falcon et al. (2024), ARES [NAACL] | [11](11-distributed-context-engineering.md) |
| 35 | Joren et al. (2025), Sufficient Context [ICLR] | [05](05-token-budget-management.md), [08](08-5-stage-assembly-pipeline.md) |
| 36 | Chroma (2025), Context Rot | [06](06-lost-in-the-middle-u-shape.md), [05](05-token-budget-management.md) |
| 37 | Shi et al. (2023), Irrelevant Context [ICML] | [06](06-lost-in-the-middle-u-shape.md) |
| 38 | Du et al. (2025), Whitespace Degradation [EMNLP] | [06](06-lost-in-the-middle-u-shape.md) |
| 39 | Mu et al. (2023), Gist Tokens [NeurIPS] | [06](06-lost-in-the-middle-u-shape.md) |
| 40 | Mehrabian (1996), PAD Model | [12](12-affect-modulated-retrieval.md) |
| 41 | Plutchik (1980), Emotion Wheel | [12](12-affect-modulated-retrieval.md) |
| 42 | Damasio (1994), Somatic Marker Hypothesis | [12](12-affect-modulated-retrieval.md) |
| 43 | Doya (2002), Neuromodulation | [12](12-affect-modulated-retrieval.md) |
| 44 | Itti & Baldi (2005), Bayesian Surprise [NeurIPS] | [07](07-active-inference-context-selection.md), [09](09-predictive-foraging-mvt.md) |
| 45 | Kapoor et al. (2025), AI Agents That Matter [Princeton] | [11](11-distributed-context-engineering.md) |
| 46 | Miller (2024), Clustered Standard Errors [Anthropic] | [11](11-distributed-context-engineering.md) |
| 47 | CLEAR Framework (2025) | [05](05-token-budget-management.md), [11](11-distributed-context-engineering.md) |
| 48 | Contextual Influence Value (2025), Shanghai Jiao Tong | [11](11-distributed-context-engineering.md) |
| 49 | McClelland, McNaughton, O'Reilly (1995), CLS Theory | (background, knowledge consolidation) |
| 50 | Grassé (1959), Stigmergy | (background, collective knowledge) |
| 51 | Dantzig (1957), Greedy Knapsack | [01](01-prompt-composer.md) |

---

## 8. Naming Map Compliance

| Old Term | New Term | Status in roko-compose |
|----------|----------|----------------------|
| Signal | Engram | **Pending** (Tier 0D). Code still uses `Signal`. |
| Golem | Agent | **Applied** |
| Bardo | Roko | **Applied** |
| Grimoire | Neuro / NeuroStore | **Applied** in context_assembler.rs (imports KnowledgeStore from roko-neuro) |
| Styx | Agent Mesh | **N/A** (no mesh code in roko-compose) |
| Mori | Roko Orchestrator | **Applied** (cache markers use `roko:layer:N` not `mori:layer:N`) |
| golem.toml | roko.toml | **Applied** |
| Clade | Collective / Mesh | **N/A** |
| GNOS | KORAI / DAEJI | **N/A** |
| Bardo Sanctum | Roko Portal | **N/A** |

---

## Cross-References

- All sub-docs in `docs/03-composition/` (00 through 12)
- `crates/roko-compose/src/` — Full implementation
- `crates/roko-cli/src/orchestrate.rs` — Orchestration wiring
- `refactoring-prd/02-five-layers.md` — Layer 2 Scaffold specification
- `refactoring-prd/09-innovations.md` — Innovation specifications
- `tmp/implementation-plans/12a-cognitive-layer.md` §E — Implementation items
