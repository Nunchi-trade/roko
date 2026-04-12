# 11 — Distributed Context Engineering

> Layer 2 Scaffold — Synapse Architecture
> Status: **Scaffold** — Framework specified, partial implementation
> Canonical sources: `refactoring-prd/09-innovations.md` §XV, Karpathy (2025)


> **Implementation**: Shipping

---

## Abstract

Distributed context engineering extends scaffold design beyond single-agent prompt assembly to multi-agent systems where context must be managed across parallel agents, shared knowledge stores, and coordinated execution plans. The four fundamental strategies — Write, Select, Compress, Isolate — form a complete basis for context management at any scale. This document specifies the four strategies, the three levels of context engineering, the Meta-Harness evaluation framework, and the integration with Roko's orchestration layer.

---

## 1. The Four Strategies

Andrej Karpathy (2025) articulated the context engineering framework: the real skill in building LLM applications is not prompt engineering (phrasing instructions well) but context engineering (managing the entire information environment the model sees). The framework defines four fundamental operations:

### 1.1 Write

**Definition:** Generating context that does not yet exist. Creating new information to inject into the prompt.

**In Roko:**
- The enrichment pipeline WRITES 13 artifact types (briefs, decompositions, research memos)
- The Strategist role WRITES plans and task breakdowns
- The knowledge store WRITES by distilling episodes into insights and heuristics
- The SystemPromptBuilder WRITES affect guidance from PAD state

Write is the most expensive strategy — it requires an LLM call to generate new content. The enrichment pipeline's model selection (Haiku for mechanical tasks, Opus for research) is a cost optimization for the Write strategy.

### 1.2 Select

**Definition:** Choosing which existing information to include. Filtering from a large candidate set to a small, high-value subset.

**In Roko:**
- Stage 2 (Score) of the 5-stage pipeline SELECTS candidates by composite score
- The ContextTier system SELECTS the appropriate amount of context per model class
- The role template system SELECTS which sections each role receives
- The MVT stopping rule SELECTS when to stop searching for more candidates

Select is the highest-leverage strategy because it determines the signal-to-noise ratio. The empirical evidence is unambiguous: including the wrong 1,000 tokens is worse than including no context at all [Joren et al., ICLR 2025]. Selection must be aggressive.

### 1.3 Compress

**Definition:** Reducing the size of existing information while preserving its semantic content.

**In Roko:**
- The ContextAssembler's compress() method COMPRESSES lower-ranked chunks to short summaries
- History compaction COMPRESSES old conversation turns to summaries
- The hard_cap mechanism COMPRESSES sections by truncation
- The PromptBudget system COMPRESSES by allocating smaller budgets to lower-priority sections

Compression exists on a fidelity spectrum:
- **Lossless:** Reformatting, whitespace removal, deterministic extraction → no information loss
- **Near-lossless:** LLMLingua-style token pruning → 20× compression, minimal quality drop
- **Lossy:** Haiku summarization → significant compression, some information loss
- **Extreme:** Gist tokens [Mu et al., NeurIPS 2023] → entire prompts → few special tokens

### 1.4 Isolate

**Definition:** Separating context into independent channels that do not interfere with each other.

**In Roko:**
- Each agent session is ISOLATED — no shared conversation history between agents
- The cache layer system ISOLATES stable prefix from volatile suffix
- The role template system ISOLATES different roles' context needs
- Git worktrees ISOLATE each agent's filesystem view

Isolation prevents context contamination — the phenomenon where one agent's irrelevant context pollutes another agent's prompt. The "write for amnesia" principle is an isolation strategy: each agent session starts cold, with no implicit context from other sessions.

---

## 2. Three Levels of Context Engineering

From the canonical specification (refactoring-prd/02-five-layers.md):

### 2.1 Level 1: Local Context Engineering

Optimizing the context for a single agent on a single task.

| Technique | Strategy | Example |
|-----------|----------|---------|
| Priority-based section dropping | Select | Drop workspace map for Trivial tasks |
| U-shape placement | Select | Place critical content at prompt edges |
| Cache-aligned prefix ordering | Compress | Stable prefix for KV cache hits |
| Complexity-adaptive budgets | Select | Trivial → 4K budget, Complex → 24K |
| Affect-modulated content | Write | Inject urgency guidance from PAD state |

Level 1 is where most scaffold work happens today. Roko's PromptComposer, SystemPromptBuilder, and ContextAssembler all operate at Level 1.

### 2.2 Level 2: Allocation Context Engineering

Optimizing context allocation across multiple agents working on the same plan.

| Technique | Strategy | Example |
|-----------|----------|---------|
| Shared plan context | Isolate | Byte-identical prefix across agents in same plan |
| Role-specific budgets | Select | Implementer gets 8K file_context, Strategist gets 0 |
| Cross-agent iteration memory | Write | Gate errors from Agent A inform Agent B's context |
| Differential compression by role | Compress | Architect gets full code, QuickReviewer gets summary |

Level 2 requires orchestration awareness — the scaffold must know about other agents and their needs. Roko's `SharedPlanContext` and `RoleSystemPromptSpec` operate at Level 2.

### 2.3 Level 3: Network Context Engineering

Optimizing context across agent collectives sharing a knowledge mesh.

| Technique | Strategy | Example |
|-----------|----------|---------|
| Stigmergic knowledge accumulation | Write | Agents deposit insights in shared Neuro store |
| Collective calibration | Select | Knowledge entries ranked by cross-agent track record |
| VCG attention auction | Select | Subsystems bid for context bandwidth |
| HDC-based retrieval | Select | Sub-50ns semantic search across collective knowledge |
| Knowledge distillation | Compress | Episodes → insights → heuristics → playbook rules |
| Agent mesh sync | Isolate | Permissioned knowledge sharing across agents |

Level 3 is the target architecture — a collective of agents that get smarter over time because every task outcome feeds back into the shared knowledge store. Roko's knowledge store and episode logging are the foundation; the full Level 3 implementation is the work specified in 12a-cognitive-layer.md.

---

## 3. The Meta-Harness Evaluation

Lee et al. (2026) [arXiv:2603.28052] evaluated coding agents across scaffolds and found:

| Finding | Measurement | Implication |
|---------|-------------|-------------|
| **6× performance gap** from scaffold changes alone | Same model, different scaffolds | Scaffold > model quality |
| **4× fewer input tokens** in the best scaffolds | Token usage comparison | Better context engineering = less input needed |
| **Scaffold diversity matters** | Performance across task types | No single scaffold dominates all tasks |

The Meta-Harness finding validates Roko's core premise: the scaffold IS the product. The 6× gap means that investing in better context engineering produces more improvement than upgrading to a more expensive model. The 4× token reduction means that better scaffolds are also cheaper.

---

## 4. The Write-for-Amnesia Principle

Every agent session starts cold. No conversation history. No shared memory. No implicit context.

The files on disk are the only truth.

This is an isolation strategy with profound implications for context engineering:

1. **All context must be explicit.** The agent cannot "remember" what a previous agent did. If the information is needed, it must be written to disk and injected into the prompt.

2. **Enrichment is pre-computation.** The enrichment pipeline creates artifacts BEFORE the agent session starts. The agent reads files, not memories.

3. **Iteration memory is structured.** When a task is retried after gate failure, the failure context (gate errors, prior attempt summary) is explicitly written to disk and injected. The agent does not "recall" the failure — it reads about it.

4. **Cross-agent communication is file-based.** Agent A's output is written to disk. Agent B's prompt includes Agent A's output as a file. There is no message passing, no shared state, no implicit knowledge transfer.

This principle makes the system fully inspectable: if an agent produces bad output, you can read its input files and see exactly what it saw. There is no hidden context, no conversation history, no mystery.

---

## 5. The CLEAR Framework Connection

The CLEAR framework [2025] defines five evaluation dimensions for AI systems: Cost, Latency, Efficacy, Assurance, Reliability. Distributed context engineering maps to CLEAR:

| CLEAR Dimension | Context Engineering Impact |
|----------------|--------------------------|
| **Cost** | Better selection = fewer tokens = lower API bills |
| **Latency** | Smaller prompts = faster inference |
| **Efficacy** | Better context = higher task success rate |
| **Assurance** | Explicit context = inspectable, auditable |
| **Reliability** | Deterministic assembly = reproducible prompts |

CLEAR's most important finding: optimizing for efficacy alone produces systems 4.4-10.8× more expensive than co-optimizing for cost and efficacy. The four context engineering strategies naturally co-optimize: Select reduces both cost and noise, Compress reduces cost while preserving quality, Isolate improves reliability, Write invests cost where it produces the highest return.

---

## 6. The RAGAS Evaluation Triad

RAGAS [Shahul Es et al., EACL 2024] defines three evaluation dimensions specifically for retrieval-augmented systems:

- **Faithfulness:** Does the agent's output match the provided context? (Measures hallucination)
- **Answer Relevance:** Does the output address the task? (Measures task completion)
- **Context Relevance:** Is the retrieved context actually useful? (Measures selection quality)

Most RAG systems optimize only for Answer Relevance and ignore Context Relevance. Roko explicitly optimizes for Context Relevance through:
- The 5-stage pipeline's deduplication stage (remove redundant context)
- The MVT stopping rule (stop when marginal relevance drops)
- The priority-based dropping (remove low-value sections first)
- The complexity-adaptive budgets (exclude sections irrelevant to simple tasks)

---

## 7. Contextual Influence Value

The Contextual Influence Value framework [Shanghai Jiao Tong University, 2025] provides per-section impact measurement through leave-one-out analysis:

```
For each section in the context pack:
    1. Remove the section
    2. Re-run the task
    3. Measure performance change
    4. The change is the section's influence value
```

If removing section A causes quality to drop 15%, section A is highly valuable. If removing section B causes quality to improve 3%, section B is actively harmful.

Three evaluation dimensions per section:
- **Query-aware relevance:** Does the section relate to the task?
- **List-aware uniqueness:** Does the section provide new information not covered by other sections?
- **Generator-aware utility:** Does the specific model benefit from this section?

This framework enables targeted pruning — removing sections that are redundant or harmful rather than globally reducing context.

---

## 8. Academic Foundations

**Karpathy, A. (2025).** Articulated the context engineering framework and the shift from "prompt engineering" to "context engineering" as the key skill for LLM application development.

**Lee et al. (2026), "Meta-Harness: Evaluating Coding Agents Across Scaffolds"** [arXiv:2603.28052]. The 6× performance gap finding. Scaffold diversity across task types.

**Zaharia et al. (2024), "The Shift to Compound AI Systems."** BAIR. State-of-the-art results from composing multiple components rather than scaling single models.

**RAGAS** [Shahul Es et al., EACL 2024]. Automated evaluation of retrieval-augmented generation systems via three metrics: Faithfulness, Answer Relevance, Context Relevance.

**ARES** [Saad-Falcon et al., NAACL 2024]. Statistical confidence intervals for RAG evaluation from minimal human labels via Prediction-Powered Inference.

**CLEAR Framework** [2025]. Five-dimensional evaluation: Cost, Latency, Efficacy, Assurance, Reliability. Accuracy-only optimization is 4.4-10.8× more expensive.

**AI Agents That Matter** [Kapoor et al., Princeton 2025]. Minimum evaluation bar: run each condition at least 5 times, report mean with confidence intervals. Use clustered standard errors.

**Contextual Influence Value** [Shanghai Jiao Tong University, 2025]. Leave-one-out per-section impact measurement for targeted context pruning.

---

## 9. Current Status and Gaps

| Aspect | Status |
|--------|--------|
| Write strategy (enrichment pipeline) | **Implemented** |
| Select strategy (priority dropping, tier budgets) | **Implemented** |
| Compress strategy (truncation, summary) | **Partially implemented** |
| Isolate strategy (session isolation, cache layers) | **Implemented** |
| Level 1 (local) context engineering | **Implemented** |
| Level 2 (allocation) context engineering | **Partially implemented** |
| Level 3 (network) context engineering | **Scaffold** |
| RAGAS-style evaluation | **Not yet** |
| Contextual influence value tracking | **Not yet** |
| Meta-Harness benchmarking | **Not yet** |

---

## Cross-References

- [04-enrichment-pipeline-13-step.md](04-enrichment-pipeline-13-step.md) — Write strategy implementation
- [05-token-budget-management.md](05-token-budget-management.md) — Select/Compress strategy
- [06-lost-in-the-middle-u-shape.md](06-lost-in-the-middle-u-shape.md) — Select strategy (placement)
- [08-5-stage-assembly-pipeline.md](08-5-stage-assembly-pipeline.md) — Full pipeline
- [10-vcg-attention-auction.md](10-vcg-attention-auction.md) — Level 3 allocation mechanism
- `refactoring-prd/09-innovations.md` §XV — Canonical specification
- `refactoring-prd/02-five-layers.md` — Three levels definition
