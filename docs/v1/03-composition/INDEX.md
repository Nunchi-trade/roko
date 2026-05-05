# 03 — Composition: Scaffold Layer (L2) — Prompt Assembly & Context Engineering

> **Topic:** 03-composition
> **Layer:** 2 — Scaffold
> **Crate:** `roko-compose`
> **Sub-docs:** 14
> **Total citations:** 51

---

## Overview

The Scaffold layer (Layer 2) is where agent performance is won or lost. Given the same model at the same temperature, scaffold changes alone produce a 6× performance gap [Lee et al. 2026]. The Roko composition system implements a multi-stage context engineering pipeline that transforms raw project knowledge, episodic memory, task specifications, and affect state into cache-aligned, budget-fitted, attention-optimized prompts.

The core insight: the right 1,000 tokens of context outperform 100,000 tokens of wrong context. Context engineering is about selection and prioritization, not volume.

---

## Contents

| # | Sub-doc | Description | Status |
|---|---------|-------------|--------|
| [00](00-composer-trait.md) | **Composer Trait** | The Composer Synapse trait, Budget struct, why composer takes Scorer as parameter | Implemented |
| [01](01-prompt-composer.md) | **PromptComposer** | Priority dropping, greedy knapsack, cache-layer ordering, U-shape placement, token estimation | Implemented (18 tests) |
| [02](02-system-prompt-builder-7-layer.md) | **SystemPromptBuilder (7-Layer)** | 7-layer prompt assembly, cache alignment markers, affect guidance injection | Implemented (12 tests) |
| [03](03-role-templates.md) | **Role Templates** | 12 role templates, PromptBudget per role, complexity-adaptive budgets | Implemented |
| [04](04-enrichment-pipeline-13-step.md) | **Enrichment Pipeline (13-Step)** | 13 enrichment steps, LLM client abstraction, staleness checking, TOML repair | Implemented |
| [05](05-token-budget-management.md) | **Token Budget Management** | budget_for(), complexity-adaptive budgets, context tiers, differential allocation | Implemented |
| [06](06-lost-in-the-middle-u-shape.md) | **Lost in the Middle (U-Shape)** | Liu et al. 2023 attention curve, Placement enum, dual-position constraints | Implemented |
| [07](07-active-inference-context-selection.md) | **Active Inference Context Selection** | EFE formula, pragmatic + epistemic value, softmax selection, PAD modulation | Scaffold |
| [08](08-5-stage-assembly-pipeline.md) | **5-Stage Assembly Pipeline** | Query → Score → Deduplicate → Budget → Format, ContextAssembler | Partial |
| [09](09-predictive-foraging-mvt.md) | **Predictive Foraging (MVT)** | Charnov 1976 Marginal Value Theorem, exponential gain curve, stopping rule | Scaffold |
| [10](10-vcg-attention-auction.md) | **VCG Attention Auction** | Vickrey-Clarke-Groves mechanism, 8 bidding subsystems, second-price payments | Design |
| [11](11-distributed-context-engineering.md) | **Distributed Context Engineering** | Write/Select/Compress/Isolate strategies, 3 levels, Meta-Harness, RAGAS, CLEAR | Partial |
| [12](12-affect-modulated-retrieval.md) | **Affect-Modulated Retrieval** | PAD state biases retrieval, arousal/pleasure/dominance modulation, somatic markers | Scaffold |
| [13](13-current-status-and-gaps.md) | **Current Status & Gaps** | Full accounting: built vs. scaffold vs. pending, 51 citations, implementation priority | Report |

---

## Key Formulas

### Active Inference (EFE)
```
G(section) = pragmatic_value + epistemic_value - ambiguity
P(include section_i) = softmax(γ × G_i), γ = 8.0
```

### Context Scoring
```
score = track_record(entry) × belief_change(entry) / uncertainty
```

### MVT Stopping Rule
```
Stop when: relevance(last) / cost ≤ total_gain / total_cost
Gain curve: g(k) = G_max × (1 - exp(-λk))
```

### VCG Bid
```
bid(section) = expected_value × urgency × affect_weight
Payment: externality imposed on others
```

### Token Estimation
```
tokens ≈ bytes / 4
```

---

## Implementation Status Summary

| Category | Count | Items |
|----------|-------|-------|
| **Fully Implemented** | 7 | Composer trait, PromptComposer, SystemPromptBuilder, Role templates, Enrichment pipeline, Token budgets, Placement/U-shape |
| **Scaffold** | 4 | Active inference scoring, Affect persistence, Neuro injection, Predictive foraging |
| **Design Only** | 3 | VCG auction, RAGAS evaluation, Level 3 network context |
| **Total sub-docs** | 14 | |
| **Total tests** | 36+ | 18 (prompt) + 12 (builder) + 6 (scorer) + others |
| **Total citations** | 51 | See [13-current-status-and-gaps.md](13-current-status-and-gaps.md) §7 |

---

## Primary Crate Dependencies

```
roko-compose
├── roko-core         # Signal/Engram, Composer trait, Budget, Scorer trait
├── roko-neuro        # KnowledgeStore, EpisodeStore (for ContextAssembler)
├── roko-learn        # Episode logger (for track_record estimation)
├── bardo-primitives  # HdcVector (for HDC-based deduplication)
└── roko-index        # HDC similarity (for fingerprint search)
```

---

## Cross-Topic References

| Topic | Relationship |
|-------|-------------|
| `01-synapse-architecture` | Defines the Composer trait, Engram struct, 6 Synapse traits |
| `02-five-layers` | Defines Layer 2 Scaffold where composition operates |
| `04-knowledge-and-mesh` | Neuro knowledge store that feeds the assembly pipeline |
| `05-cognitive-subsystems` | Daimon (PAD state), Dreams (episode consolidation) |
| `06-interfaces` | ROSEDUST design language, Spectre visualization |

---

## Generation Notes

- **Generated:** 2026-04-11
- **Source reading:** 7 context-pack files, 5 refactoring-PRD canonical sources, 6 legacy PRD/research files, 12 roko-compose source files, 1 implementation plan
- **Naming map applied:** Bardo→Roko, Golem→Agent, Grimoire→Neuro, Signal→Engram (noted as pending Tier 0D), Mori→Roko Orchestrator
- **Reframe rules applied:** No mortality language, no death phases, budget/confidence/time pressure instead
