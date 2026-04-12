# 08 — The 5-Stage Assembly Pipeline: Query → Score → Deduplicate → Budget → Format

> Layer 2 Scaffold — Synapse Architecture
> Status: **Partially Implemented** — Stages 1-2 in ContextAssembler, Stage 3 (compress), Stages 4-5 in PromptComposer
> Canonical sources: `refactoring-prd/02-five-layers.md`, `12a-cognitive-layer.md` §E


> **Implementation**: Shipping

---

## Abstract

The 5-stage assembly pipeline transforms a task description into a cache-aligned, budget-fitted, U-shaped prompt. The five stages — Query, Score, Deduplicate, Budget, Format — are executed in order for every agent spawn. The pipeline bridges the gap between raw context sources (knowledge store, episodes, file content, signals) and the final assembled prompt. Each stage is independently testable and replaceable. This document specifies each stage, the data flow between them, the scoring formula, the deduplication threshold, and the integration points.

---

## 1. Pipeline Overview

```
Task description + metadata
         │
         ▼
┌─────────────────────────┐
│ Stage 1: QUERY          │  HDC fingerprint search + keyword search
│ Candidate retrieval     │  Returns top-50 candidates with similarity scores
└────────────┬────────────┘
             │ Vec<ContextChunk>
             ▼
┌─────────────────────────┐
│ Stage 2: SCORE          │  Composite score per candidate
│ Rank by relevance       │  track_record × belief_change / uncertainty
└────────────┬────────────┘
             │ sorted Vec<ContextChunk>
             ▼
┌─────────────────────────┐
│ Stage 3: DEDUPLICATE    │  Remove near-duplicates
│ Diversity enforcement   │  Hamming distance < 0.15 → duplicate
└────────────┬────────────┘
             │ pruned Vec<ContextChunk>
             ▼
┌─────────────────────────┐
│ Stage 4: BUDGET         │  Fit to token budget
│ Priority-based dropping │  800-1,200 tokens for knowledge context
└────────────┬────────────┘
             │ budget-fitted Vec<ContextChunk>
             ▼
┌─────────────────────────┐
│ Stage 5: FORMAT         │  U-shaped placement
│ Cache-aligned output    │  Most relevant at start + end
└────────────┬────────────┘
             │ final assembled prompt
             ▼
       Agent execution
```

---

## 2. Stage 1: Query (Candidate Retrieval)

The query stage retrieves candidate context chunks from four sources:

### 2.1 Sources

| Source | Content | Query Method | Typical Candidates |
|--------|---------|-------------|-------------------|
| Knowledge Store | Insights, heuristics, warnings, anti-knowledge | HDC fingerprint similarity + keyword search | 5-15 entries |
| Episode Store | Past task execution records | Task category + crate + file overlap | 3-5 episodes |
| File Context | Source code from target files | Direct file read (from task TOML `read_files`) | 2-8 files |
| Signal Log | Recent plan signals (gate results, outputs) | Plan ID filter + recency | 2-5 signals |

### 2.2 Hybrid Search

Knowledge retrieval uses hybrid search — both HDC fingerprint similarity and keyword matching — fused using Reciprocal Rank Fusion (RRF):

```
RRF_score = Σ_{search_mode} 1 / (K + rank_in_mode)

where K = 60 (standard RRF constant)
```

A result ranked first in both keyword and HDC search scores `1/61 + 1/61 = 0.033`. A result only in one list at rank 5 scores `1/66 = 0.015`. Results near the top in both lists naturally win.

### 2.3 Implementation

```rust
// crates/roko-compose/src/context_assembler.rs

impl ContextAssembler {
    pub fn gather(
        &self,
        workdir: impl AsRef<Path>,
        task: &TaskInput,
        plan_id: &str,
        signals_path: impl AsRef<Path>,
    ) -> Vec<ContextChunk> {
        let task_text = task_query_text(task);

        let mut chunks = Vec::new();
        chunks.extend(self.gather_knowledge(&task_text));
        chunks.extend(self.gather_episodes(task, plan_id, &task_text));
        chunks.extend(self.gather_read_files(workdir, task));
        chunks.extend(self.gather_recent_signals(plan_id, signals_path));

        self.rank(&task_text, &mut chunks);
        self.compress(chunks)
    }
}
```

The `ContextChunk` struct carries metadata for downstream scoring:

```rust
pub struct ContextChunk {
    pub content: String,
    pub source: ContextSource,
    pub relevance: f64,
    pub track_record: Option<f64>,
    pub confidence: Option<f64>,
    pub recency: Option<f64>,
}
```

---

## 3. Stage 2: Score (Ranking)

### 3.1 Current Scoring (Static)

The current implementation scores chunks using a composite formula:

```rust
fn score_chunk(task_text: &str, chunk: &ContextChunk, affect: Option<&PadState>) -> f64 {
    let base = source_priority(&chunk.source)        // source type weight
        + chunk.relevance * 0.4                       // relevance from retrieval
        + chunk.track_record.unwrap_or(0.0) * 0.3    // historical success
        + chunk.confidence.unwrap_or(0.5) * 0.2      // confidence level
        + chunk.recency.unwrap_or(0.5) * 0.1;        // recency bonus

    // Affect modulation
    let affect_modifier = match affect {
        Some(pad) if pad.arousal >= 0.35 => {
            // High arousal: boost recent and action-oriented
            chunk.recency.unwrap_or(0.0) * 0.2
        }
        Some(pad) if pad.pleasure <= -0.35 => {
            // Low pleasure: boost anti-knowledge and warnings
            if matches!(chunk.source, ContextSource::AntiPattern) { 0.3 } else { 0.0 }
        }
        _ => 0.0,
    };

    base + affect_modifier
}
```

### 3.2 Target Scoring (Active Inference)

The target scoring replaces the ad hoc weights with the active inference EFE formula:

```
score = track_record(entry) × belief_change(entry) / uncertainty
```

See [07-active-inference-context-selection.md](07-active-inference-context-selection.md) for the full specification.

### 3.3 Source Priority

Different context sources receive baseline priority weights:

| Source Type | Priority Weight | Rationale |
|------------|----------------|-----------|
| AntiPattern | 1.0 | Critical safety information |
| Verification | 0.9 | Verification commands for this task |
| TaskBrief | 0.8 | Direct task context |
| InlineFile | 0.7 | Source code for target files |
| KnowledgeEntry | 0.6 | Relevant knowledge from store |
| Episode | 0.5 | Past experience |
| SymbolSignature | 0.4 | Type signatures |
| RecentSignal | 0.3 | Plan signals |
| SiblingTasks | 0.2 | Awareness of other tasks |

---

## 4. Stage 3: Deduplicate (Diversity Enforcement)

### 4.1 Near-Duplicate Detection

Candidates that are too similar to each other are deduplicated using HDC fingerprint comparison:

```
For each candidate (in score order, highest first):
    If Hamming_distance(candidate.fingerprint, any_selected.fingerprint) < 0.15:
        Skip candidate (near-duplicate)
    Else:
        Select candidate
```

The 0.15 Hamming threshold removes entries that are functionally identical while preserving genuinely distinct perspectives on the same topic.

### 4.2 Why Deduplication Matters

Without deduplication, a query for "proxy deployment" might return 15 near-identical entries about UUPS initializer patterns, leaving no room for the chain-specific gas warning that prevents the most common failure. Cluster domination wastes budget on redundant information while starving diverse perspectives.

### 4.3 Current Implementation

The current ContextAssembler implements a simpler form of deduplication: the `compress` method summarizes lower-ranked chunks to short heads:

```rust
fn compress(&self, mut chunks: Vec<ContextChunk>) -> Vec<ContextChunk> {
    let split_at = chunks.len() / 2;
    for (idx, chunk) in chunks.iter_mut().enumerate() {
        if idx >= split_at {
            continue;  // top half stays verbatim
        }
        chunk.content = summarize_content(&chunk.content);  // bottom half summarized
    }
    // Drop lowest-ranked until budget fits
    while total_tokens > self.max_context_tokens {
        chunks.pop();
    }
    chunks
}
```

HDC-based deduplication (D16 in 12a-cognitive-layer.md) is the planned replacement for this simpler compression.

---

## 5. Stage 4: Budget (Token Fitting)

### 5.1 Budget Targets

| Context Category | Token Budget |
|-----------------|-------------|
| Knowledge context (from Neuro) | 800-1,200 tokens |
| File context (source code) | Up to 8,000 tokens |
| Episode summaries | 500-1,000 tokens |
| Signal context | 200-500 tokens |
| **Total assembled context** | Per context tier: 4K / 12K / 24K |

### 5.2 Budget Enforcement

The budget stage is greedy: candidates are included in score order until the budget is exhausted. Unlike the PromptComposer's approach (which truncates critical sections to fit), the context assembler drops candidates entirely — a context chunk either fits whole or is skipped. This preserves semantic coherence within each chunk.

From the canonical spec:

> Entries are never truncated; an entry either fits whole or is skipped entirely. This preserves semantic coherence within each entry.

### 5.3 Interaction with PromptComposer Budget

The 5-stage pipeline produces context sections that are then fed into the PromptComposer along with other sections (role identity, conventions, task description). The PromptComposer applies its own budget fitting across all sections. The pipeline's internal budget is for the context portion only; the PromptComposer's budget covers the entire prompt.

---

## 6. Stage 5: Format (U-Shaped Placement)

### 6.1 Ordering Rule

The formatted output arranges entries by relevance with U-shaped placement:

```
Position 1-3:    Highest-scoring entries     → Beginning (highest attention)
Position 4..N-3: Medium-scoring entries      → Middle (lowest attention)
Position N-2..N: Second-highest entries       → End (second-highest attention)
```

### 6.2 Entry Format

Each entry is formatted with metadata for the consuming agent:

```
[Type: Insight] [Age: 3d] [Weight: 0.82] [Confirmations: 7]
{Content text}

[Type: Heuristic] [Age: 14d] [Weight: 0.91] [Confirmations: 23]
{Content text}
```

This metadata allows the agent to assess provenance at a glance without needing to read the full knowledge store.

### 6.3 Integration with Cache Alignment

The U-shaped context block is placed as a single section within the PromptComposer's assembly. Its placement within the overall prompt is determined by its CacheLayer (typically Session or Task) and Placement hint (typically Middle, since the Start and End positions are reserved for role identity and constraints).

---

## 7. Performance

The pipeline executes pre-task and produces a ready-to-inject context pack in under 5ms total:

| Stage | Latency |
|-------|---------|
| Query (HDC search) | <2ms (sub-50ns per comparison, no GPU) |
| Score | <0.5ms |
| Deduplicate | <0.5ms |
| Budget | <0.5ms |
| Format | <0.5ms |
| **Total** | **<5ms** |

The HDC fingerprint search is the dominant cost, and it operates on pre-computed binary vectors using Hamming distance (XOR + popcount), which is O(1) per comparison on modern CPUs with POPCNT instructions.

---

## 8. The Full Scoring Formula (Canonical)

From `agent-chain/15-dynamic-context-assembly.md`, the canonical composite scoring formula used in Stage 2:

```
score = (hdc_similarity × 0.4)
      + (weight_decay × 0.3)
      + (pf_utility × 0.2)
      + (freshness × 0.1)

Where:
  hdc_similarity: Hamming distance normalized to [0,1]
  weight_decay:   current entry weight (bucketed computation)
  pf_utility:     Predictive Foraging utility score — 0 if not calibrated
  freshness:      recency bonus, linear decay over last 7 days
```

The weights (0.4, 0.3, 0.2, 0.1) prioritize semantic relevance while giving meaningful influence to proven utility. The `pf_utility` component (see [09-predictive-foraging-mvt.md](09-predictive-foraging-mvt.md)) ensures that entries which actually improved task outcomes in verified predictions are ranked higher than entries that were merely popular.

When Predictive Foraging is not yet calibrated (new agent, new domain), pf_utility defaults to 0 and the remaining three signals absorb the weight.

---

## 9. Academic Foundations

**Retrieval-Augmented Generation (RAG)** [Lewis et al. 2020]. The foundational RAG paper: combining pre-trained seq2seq with dense vector retrieval. The 5-stage pipeline is an advanced RAG implementation with scoring, deduplication, and attention-aware formatting.

**Modular RAG** [Gao et al. 2023]. The evolution from Naive RAG to composable modules. Each stage of the pipeline is a replaceable module.

**Reciprocal Rank Fusion** [Cormack et al. 2009]. The RRF formula for combining ranked lists from multiple search methods. Used in Stage 1 for hybrid HDC + keyword search.

**Liu et al. (2023), "Lost in the Middle"** [TACL 2024, arXiv:2307.03172]. The U-shaped attention finding that motivates Stage 5 formatting.

**Sufficient Context** [Joren et al., ICLR 2025]. Adding insufficient context makes models 6× worse. Motivates Stage 4's "never truncate entries" policy.

**RAGAS** [Shahul Es et al., EACL 2024]. Three evaluation dimensions: Faithfulness, Answer Relevance, Context Relevance. The pipeline optimizes for Context Relevance (Stage 3 deduplication) and Answer Relevance (Stage 2 scoring).

**Predictive Foraging** [Charnov 1976, Pirolli & Card 1999]. Marginal Value Theorem applied to information foraging. See [09-predictive-foraging-mvt.md](09-predictive-foraging-mvt.md).

---

## 10. Current Status and Gaps

| Stage | Status | Implementation |
|-------|--------|---------------|
| Stage 1: Query | **Implemented** | ContextAssembler.gather_* methods |
| Stage 2: Score | **Implemented** (static) | score_chunk function |
| Stage 2: Score | **Not yet** (active inference) | E2 in 12a plan |
| Stage 3: Deduplicate | **Partial** (compression) | compress() method |
| Stage 3: Deduplicate | **Not yet** (HDC-based) | D16 in 12a plan |
| Stage 4: Budget | **Implemented** | compress() token budget loop |
| Stage 5: Format | **Partial** (Placement enum) | PromptComposer U-shape |
| Stage 5: Format | **Not yet** (metadata annotations) | Entry format with provenance |

---

## Cross-References

- [00-composer-trait.md](00-composer-trait.md) — Composer trait that consumes pipeline output
- [01-prompt-composer.md](01-prompt-composer.md) — PromptComposer assembly algorithm
- [06-lost-in-the-middle-u-shape.md](06-lost-in-the-middle-u-shape.md) — Stage 5 formatting rationale
- [07-active-inference-context-selection.md](07-active-inference-context-selection.md) — Stage 2 target scoring
- [09-predictive-foraging-mvt.md](09-predictive-foraging-mvt.md) — pf_utility in scoring formula
- `crates/roko-compose/src/context_assembler.rs` — Stage 1-3 implementation
- `crates/roko-compose/src/prompt.rs` — Stage 4-5 implementation
