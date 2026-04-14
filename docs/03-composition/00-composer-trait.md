# 00 — The Composer Trait

> Layer 2 Scaffold — Synapse Architecture
> Status: **Implemented** — `roko-compose` crate
> Canonical source: `refactoring-prd/01-synapse-architecture.md`


> **Implementation**: Shipping

---

## Abstract

The Composer trait is one of the six composable verb traits in the Synapse Architecture. It defines the contract for assembling scored, budgeted context into a single coherent prompt engram. Unlike the other five traits (Substrate, Scorer, Gate, Router, Policy), the Composer explicitly receives a `Scorer` reference at call time, making scoring an input to composition rather than a separate upstream phase. This design ensures that composition is always scoring-aware: the composer can re-score, re-rank, and re-prioritize engrams during assembly, not just consume a pre-ranked list.

This document specifies the Composer trait signature, the Budget struct that constrains it, the rationale for the scorer-in-signature design, and how composition fits into the universal cognitive loop.

---

## 1. Trait Signature

The Composer trait is defined in `roko-core` as one of the six Synapse verb traits:

```rust
// crates/roko-core/src/agent.rs

pub trait Composer: Send + Sync {
    fn compose(
        &self,
        engrams: &[Engram],
        budget: &Budget,
        scorer: &dyn Scorer,
        ctx: &Context,
    ) -> Result<Engram>;
}
```

**Parameters:**

| Parameter | Type | Purpose |
|-----------|------|---------|
| `engrams` | `&[Engram]` | Candidate context units to assemble |
| `budget` | `&Budget` | Hard constraints on output size |
| `scorer` | `&dyn Scorer` | Scoring function for ranking candidates |
| `ctx` | `&Context` | Ambient context (agent state, task metadata) |

**Returns:** A single `Engram` — the assembled prompt, ready for LLM consumption.

The trait is `Send + Sync`, allowing composers to be shared across threads in parallel plan execution. It is synchronous — composition is a CPU-bound operation that should never perform I/O. Composers do not read files, do not query databases, and do not call LLMs. They receive pre-gathered candidates and assemble them under budget constraints.

---

## 2. The Engram: Content-Addressed Unit of Cognition

Every input and output of the Composer is an `Engram` — the fundamental data type of the Synapse Architecture. An Engram is a content-addressed, scored, decaying, lineage-tracked unit of cognition:

```rust
// crates/roko-core/src/agent.rs (canonical PRD spec)

pub struct Engram {
    pub id: EngramId,           // Content-addressed hash (Blake3)
    pub body: Body,             // Payload (text, structured data, binary)
    pub score: Score,           // 7-axis quality assessment
    pub lineage: Lineage,       // DAG of parent engrams
    pub created_at: Timestamp,
    pub ttl: Option<Duration>,  // Time-to-live for decay
    pub tags: Vec<Tag>,         // Semantic labels
}
```

The 7-axis Score captures multiple quality dimensions:

```rust
pub struct Score {
    pub confidence: f64,    // [0,1] — how certain is this information?
    pub novelty: f64,       // [0,1] — how new/surprising is this?
    pub utility: f64,       // [0,1] — how useful for the current task?
    pub reputation: f64,    // [0,1] — trust in the source
    pub salience: f64,      // [0,1] — how attention-worthy?
    pub coherence: f64,     // [0,1] — internal consistency
    pub relevance: f64,     // [0,1] — match to current query
}
```

The Composer receives a slice of scored Engrams and produces a single output Engram whose body contains the assembled prompt. The output Engram's lineage field records which input Engrams were included, providing full provenance for every prompt.

---

## 3. The Budget Struct

The Budget struct constrains composition output:

```rust
// crates/roko-core/src/agent.rs

pub struct Budget {
    pub max_tokens: usize,
    pub max_signals: usize,
    pub max_bytes: usize,
}
```

| Field | Purpose | Typical values |
|-------|---------|---------------|
| `max_tokens` | Hard cap on estimated token count of output | 4,000 — 24,000 |
| `max_signals` | Maximum number of engrams to include | 10 — 50 |
| `max_bytes` | Byte-level cap (for binary payloads) | 100KB — 1MB |

The three constraints work as a conjunction: all must be satisfied. The tightest constraint wins. For text prompts, `max_tokens` is typically the binding constraint. Token estimation uses the heuristic of approximately 4 bytes per token (established by empirical measurement across Anthropic and OpenAI tokenizers for English text and source code).

### Budget Derivation

Budgets are derived from the context tier and model context window:

| Context Tier | Token Budget | Use Case |
|-------------|-------------|----------|
| **Surgical** | ~4,000 | Haiku, Ollama, Gemma — mechanical tasks |
| **Focused** | ~12,000 | Sonnet — focused/integrative tasks |
| **Full** | ~24,000 | Opus — architectural tasks |

The context tier is determined by `ContextTier::from_task_and_model()`, which maps the task complexity band and model backend to the appropriate tier. Local models (Ollama, Gemma, Llama, DeepSeek, Phi, StarCoder) always receive Surgical tier regardless of task complexity, because they cannot reliably handle large contexts or tools.

---

## 4. Why the Composer Takes a Scorer

The Composer trait's most distinctive design choice is accepting `&dyn Scorer` as a parameter rather than consuming pre-scored engrams. This is deliberate and has three motivations:

### 4.1 Re-scoring During Assembly

Static pre-scoring assumes that relevance is context-independent. It is not. An engram's value depends on what else is in the prompt. If two engrams contain overlapping information, including both wastes budget. If one engram provides definitions that another references, ordering matters. The Composer can re-score engrams during assembly to account for these interactions — marginal value decreases as similar content is already included.

### 4.2 Scorer as Strategy

Different scoring strategies produce different compositions from the same candidates. A priority-based scorer produces deterministic, predictable prompts. An active-inference scorer (see [07-active-inference-context-selection.md](07-active-inference-context-selection.md)) produces adaptive prompts that explore when uncertain and exploit when confident. By accepting the scorer as a parameter, the Composer is decoupled from any specific scoring strategy. The caller chooses the strategy; the Composer applies it.

### 4.3 Testability

Accepting a scorer as a parameter makes composition fully testable. Unit tests can inject mock scorers that return predetermined values, verifying that the Composer correctly implements priority dropping, budget fitting, and U-shape placement without needing real scoring infrastructure.

---

## 5. The Current Implementation: PromptComposer

The primary Composer implementation in `roko-compose` is `PromptComposer`, which implements the Composer trait:

```rust
// crates/roko-compose/src/prompt.rs

impl Composer for PromptComposer {
    fn compose(
        &self,
        signals: &[Signal],
        budget: &Budget,
        scorer: &dyn Scorer,
        ctx: &Context,
    ) -> Result<Signal> {
        // 1. Decode signals into PromptSections
        // 2. Score each section
        // 3. Partition into Critical and Optional
        // 4. Sort by cache_layer ASC, priority DESC
        // 5. Greedy include under budget (Critical sections never dropped)
        // 6. Order by Placement (Start/Middle/End) for U-shape
        // 7. Concatenate with section headers
        // 8. Return assembled prompt as a Signal
    }
}
```

The implementation is detailed in [01-prompt-composer.md](01-prompt-composer.md).

Note: The current codebase uses `Signal` where the Synapse Architecture PRD specifies `Engram`. The rename from Signal to Engram is tracked as Tier 0D in the implementation priorities. The trait semantics are identical — only the type name changes.

---

## 6. Composition in the Universal Cognitive Loop

The Composer operates at a specific point in the universal cognitive loop:

```
PERCEIVE (Substrate.query)
    → REMEMBER (Scorer.score)
        → ATTEND (Router.select)
            → **COMPOSE** (Composer.compose)  ← here
                → ACT (Agent.execute)
                    → VERIFY (Gate.verify)
                        → ADAPT (Policy.decide)
                            → META-COGNIZE (Daimon.assess)
```

The Composer receives the output of the Router (which has selected which engrams to include) and the Scorer (which has ranked them). It assembles these into the final prompt that the Agent will execute against.

In the current wiring (`roko-cli/src/orchestrate.rs`), composition happens via `RoleSystemPromptSpec::compose_with_budget()`, which builds the 7-layer system prompt, applies role-specific budgets, and outputs the assembled prompt string. The PromptComposer is invoked within this pipeline to handle the final budget-fitting and ordering.

---

## 7. Design Constraints

The Composer operates under several constraints derived from the Synapse Architecture:

1. **Synchronous only.** Composition must not perform I/O. All candidates are pre-gathered.
2. **Deterministic.** The same inputs must produce the same output. This is critical for prompt cache alignment — if composition is non-deterministic, prefix caching fails.
3. **Budget-respecting.** The output must satisfy all Budget constraints. No exceptions.
4. **Critical sections survive.** Sections marked as Critical priority are never dropped, only truncated. This ensures that safety instructions, role identity, and task description always appear.
5. **Lineage-preserving.** The output Engram's lineage must record which inputs were included, enabling provenance tracking and credit assignment.
6. **Placement-aware.** The Composer must respect Placement hints (Start/Middle/End) to implement U-shape attention optimization (Liu et al. 2023 [arXiv:2307.03172]).

---

## 8. Relationship to Other Traits

| Trait | Relationship to Composer |
|-------|-------------------------|
| **Substrate** | Provides raw engrams from storage/sensors |
| **Scorer** | Ranks engrams; passed as parameter to Composer |
| **Gate** | Validates composition output (does the prompt meet quality thresholds?) |
| **Router** | Selects which engrams to include; upstream of Composer |
| **Policy** | Decides when to recompose (e.g., after gate failure, trigger re-composition with different scorer) |

The Composer is the convergence point: it receives output from Substrate (candidates), Scorer (rankings), and Router (selection), and produces the input for the Agent (assembled prompt). It is the most downstream trait before execution.

---

## 9. Academic Foundations

The Composer trait's design draws on several bodies of work:

**Compound AI Systems** [Zaharia et al., BAIR 2024]. The Composer embodies the compound AI principle: state-of-the-art results come from composing multiple components, not from single model calls. The 6-trait architecture is a compound system where each trait is a composable module.

**CoALA: Cognitive Architectures for Language Agents** [Sumers et al. 2023]. CoALA provides the theoretical framework: cognitive agents have a universal structure (perception, memory, reasoning, action, reflection) with modular memory components. The Composer maps to CoALA's "working memory assembly" phase — constructing the agent's active context from long-term and episodic memory.

**DSPy: Programmatic Prompt Optimization** [Khattab et al. 2023]. DSPy reframed prompting as programming: define modules with typed signatures, compose them into pipelines, and let a compiler optimize prompts automatically against a metric. The Composer trait's typed signature (`engrams × budget × scorer × ctx → engram`) is DSPy-compatible: it defines a composable module that can be optimized against downstream task success.

**Modular RAG** [Gao et al. 2023]. The evolution from Naive RAG (retrieve-then-read) through Advanced RAG (query rewriting, re-ranking) to Modular RAG (composable retrieval/generation/augmentation modules). The Composer is the "augmentation" module in Modular RAG — it determines how retrieved content is assembled and presented to the generator.

---

## 10. Current Status

| Aspect | Status |
|--------|--------|
| Trait definition in `roko-core` | **Implemented** |
| `PromptComposer` implementation | **Implemented** (18 tests) |
| `SectionScorer` implementation | **Implemented** (6 tests) |
| Budget types | **Implemented** |
| ContextTier derivation | **Implemented** |
| Active inference scoring | **Scaffold** (see E2 in 12a-cognitive-layer.md) |
| U-shape placement | **Implemented** (Placement enum: Start/Middle/End) |
| Lineage tracking in output | **Not yet wired** |
| Signal → Engram rename | **Pending** (Tier 0D) |

---

## Cross-References

- [01-prompt-composer.md](01-prompt-composer.md) — PromptComposer implementation details
- [05-token-budget-management.md](05-token-budget-management.md) — Budget derivation and tier-specific allocation
- [06-lost-in-the-middle-u-shape.md](06-lost-in-the-middle-u-shape.md) — U-shape attention optimization
- [07-active-inference-context-selection.md](07-active-inference-context-selection.md) — EFE-based scoring
- [08-5-stage-assembly-pipeline.md](08-5-stage-assembly-pipeline.md) — Full assembly pipeline
- `refactoring-prd/01-synapse-architecture.md` — Synapse Architecture specification
- `refactoring-prd/02-five-layers.md` — Layer 2 Scaffold definition
- `crates/roko-compose/src/prompt.rs` — Implementation source
- `crates/roko-core/src/agent.rs` — Trait definitions
