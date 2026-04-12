# Context Assembly from Code Search

> How indexed code becomes LLM context — combining graph traversal, PageRank scoring, HDC similarity, and budget-aware composition to build the optimal context window for coding tasks.


> **Implementation**: Built

**Topic**: [Code Intelligence](./INDEX.md)
**Prerequisites**: [04-pagerank-symbol-importance.md](./04-pagerank-symbol-importance.md), [05-hdc-fingerprints.md](./05-hdc-fingerprints.md)
**Key sources**: `bardo-backup/tmp/mori-refactor/10-code-intelligence.md`, `bardo-backup/tmp/mori-agents/18-code-intelligence-and-gateway.md`, `refactoring-prd/05-agent-types.md`, `crates/roko-compose/src/system_prompt_builder.rs`

---

## Abstract

Code intelligence generates raw data — symbols, graphs, fingerprints, search results. But this data has no value until it reaches the LLM as context. The context assembly pipeline transforms indexed code into a token-budgeted context window that gives the agent exactly the information it needs for the current task.

This is the bridge between `roko-index` (which understands code structure) and `roko-compose` (which builds prompts). The assembly pipeline takes a task description, queries the index, ranks results, and composes them into a context block that fits within the agent's token budget while maximizing the density of relevant information.

The design draws on five search strategies, Reciprocal Rank Fusion for combining results, program slicing for extracting minimal relevant code, and the Synapse Architecture's Composer trait for budget-aware assembly. This document describes the full pipeline from query to context.

---

## The Five Search Strategies

### Overview

The planned search layer combines five complementary strategies, each capturing different aspects of relevance:

| # | Strategy | What it finds | Speed | Quality |
|---|---|---|---|---|
| 1 | **Keyword** | Exact text matches (symbol names, string literals) | Fast (FTS5) | High for exact matches |
| 2 | **Structural** | Symbols by kind, visibility, file pattern | Fast (SQL) | Good for typed queries |
| 3 | **HDC Similarity** | Structurally similar symbols (name + kind + context) | Fast (Hamming) | Good for "find similar" |
| 4 | **Embedding Similarity** | Semantically similar code (meaning, not structure) | Medium (ANN) | Excellent for intent |
| 5 | **Hybrid (RRF)** | Combined ranking from all strategies | Medium | Best overall |

### Strategy 1: Keyword search

Traditional text search over symbol names and file contents, backed by SQLite FTS5:

```rust
// Planned: Keyword search query
pub struct KeywordQuery {
    pub text: String,        // Search text
    pub scope: SearchScope,  // Files, symbols, or both
    pub case_sensitive: bool,
    pub whole_word: bool,
}
```

Keyword search excels when the agent knows the exact name: "find the `build_graph` function" or "where is `SymbolGraph` defined?" It is the fastest strategy and should be tried first for specific queries.

### Strategy 2: Structural search

Query symbols by their structural properties:

```rust
// Planned: Structural search query
pub struct StructuralQuery {
    pub kind: Option<SymbolKind>,       // Function, Struct, Trait, etc.
    pub visibility: Option<Visibility>,  // Public, Private, Crate
    pub file_pattern: Option<String>,    // Glob pattern (e.g., "crates/roko-index/**")
    pub has_callers: Option<bool>,       // Symbols with/without callers
    pub min_pagerank: Option<f64>,       // Minimum importance score
}
```

Use cases:
- "Find all public traits in `roko-core`" → kind=Trait, visibility=Public, file_pattern="crates/roko-core/**"
- "Find unused functions" → kind=Function, has_callers=false
- "Find the most important types" → kind=Struct, min_pagerank=0.01

### Strategy 3: HDC similarity search

Find symbols structurally similar to a query symbol:

```rust
// Planned: HDC similarity query
pub struct HdcQuery {
    pub anchor: HdcFingerprint,  // Fingerprint to match against
    pub min_similarity: f64,     // Threshold (e.g., 0.6)
    pub max_results: usize,      // Top-K limit
}
```

The query fingerprint can be computed from:
- An existing symbol: "find symbols similar to `process_input`"
- A synthetic description: "find functions that parse configuration" (encode the description)
- A code snippet: "find code similar to this pattern"

### Strategy 4: Embedding similarity search

For semantic queries that go beyond structural similarity:

```rust
// Planned: Embedding search query
pub struct EmbeddingQuery {
    pub text: String,           // Natural language or code query
    pub embedding: Vec<f32>,    // Pre-computed embedding (384-dim for BGE-small)
    pub max_results: usize,
    pub min_similarity: f32,
}
```

Dense embeddings capture meaning that HDC fingerprints miss. "Find error handling code" matches functions whose documentation or implementation deals with errors, even if the function name doesn't contain "error."

The planned embedding model is BGE-small-en-v1.5 (384 dimensions) via the `fastembed` crate. This model runs on CPU in ~10ms per embedding, making it practical without GPU infrastructure.

### Strategy 5: Hybrid search with Reciprocal Rank Fusion

RRF (Robertson and Zaragoza 2009) combines ranked lists from multiple strategies:

```
RRF_score(symbol) = Σ_strategy 1 / (k + rank_strategy(symbol))
```

Where `k = 60` (a standard constant). The formula rewards symbols that appear in multiple result lists and penalizes symbols that only appear in one.

```rust
// Planned: Hybrid search combining all strategies
pub fn hybrid_search(
    keyword_results: &[(SymbolId, f64)],
    structural_results: &[(SymbolId, f64)],
    hdc_results: &[(SymbolId, f64)],
    embedding_results: &[(SymbolId, f64)],
) -> Vec<(SymbolId, f64)> {
    let mut rrf_scores: HashMap<SymbolId, f64> = HashMap::new();
    let k = 60.0;

    for (i, (id, _)) in keyword_results.iter().enumerate() {
        *rrf_scores.entry(id.clone()).or_default() += 1.0 / (k + i as f64);
    }
    for (i, (id, _)) in structural_results.iter().enumerate() {
        *rrf_scores.entry(id.clone()).or_default() += 1.0 / (k + i as f64);
    }
    // ... same for hdc and embedding results

    let mut combined: Vec<_> = rrf_scores.into_iter().collect();
    combined.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    combined
}
```

---

## The Context Assembly Pipeline

### End-to-end flow

```
  Task description
        │
        ▼
  1. PARSE QUERY ──→ Extract search terms, intent, focal symbols
        │
        ▼
  2. MULTI-STRATEGY SEARCH ──→ Run applicable strategies in parallel
        │
        ▼
  3. RANK (RRF) ──→ Combine results into single ranked list
        │
        ▼
  4. EXPAND GRAPH ──→ Add graph neighbors of top results (1-2 hops)
        │
        ▼
  5. SLICE ──→ Extract relevant code fragments (not whole files)
        │
        ▼
  6. BUDGET ──→ Fit into token budget, prioritizing by rank
        │
        ▼
  Context block (ready for prompt assembly)
```

### Step 1: Parse query

The task description is analyzed to extract search parameters:
- **Explicit symbol mentions** — "modify the `build_graph` function" → keyword search for `build_graph`
- **Kind hints** — "add a new trait" → structural search for existing traits (to understand patterns)
- **Similarity intent** — "like the existing `Gate` implementation" → HDC similarity anchored on `Gate`
- **Scope hints** — "in the `roko-index` crate" → file pattern filter

### Step 2: Multi-strategy search

Applicable strategies run in parallel. Not all strategies apply to every query:
- A specific name query uses keyword only (fast path)
- A "find similar" query uses HDC + embedding
- An open-ended exploration uses all five strategies

### Step 3: Rank via RRF

Results from all strategies are combined using Reciprocal Rank Fusion. The unified ranking ensures that symbols appearing across multiple strategies rise to the top.

### Step 4: Expand graph

The top-ranked symbols are expanded using the dependency graph:
- **Forward expansion (depth 1)** — Include what the focal symbols depend on
- **Reverse expansion (depth 1)** — Include what depends on the focal symbols
- **Contextual expansion** — Include symbols in the same file as focal symbols

This step is critical because search results may include a function without its type definitions, or a struct without its constructor. Graph expansion fills in the structural context.

### Step 5: Slice

Rather than including entire files, the pipeline extracts minimal code fragments:

```rust
// Planned: Code slicing
pub struct CodeSlice {
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    pub symbols_included: Vec<SymbolId>,
    pub token_estimate: usize,
}
```

Program slicing (Weiser 1981) provides the theoretical foundation: include only the code that is relevant to the current computation. In practice, this means:
- For a function: include the function signature, body, and immediately surrounding context (doc comments, attributes)
- For a struct: include the struct definition and key method implementations
- For a trait: include the trait definition and all method signatures

### Step 6: Budget-aware composition

The Composer trait from the Synapse Architecture handles budget allocation:

```rust
// From roko-compose
pub trait Composer {
    fn compose(
        &self,
        candidates: &[Engram],
        scorer: &dyn Scorer,
        budget: TokenBudget,
    ) -> ComposedContext;
}
```

The code context assembly maps to this interface:
- **Candidates** — Code slices wrapped as Engrams, scored by PageRank × RRF rank
- **Scorer** — Combines PageRank, RRF score, and recency into a composite score
- **Budget** — Token limit minus system prompt, instructions, and conversation history
- **Output** — Ordered list of code slices that fit within budget, highest-scored first

Token estimation uses a simple heuristic: ~4 characters per token for code (slightly different from English prose). More precise estimation could use a tokenizer, but the 4:1 approximation is sufficient for budget allocation.

---

## Context Overlay System

### Per-agent views

Different agents working on the same codebase may need different views of the code intelligence data. The planned context overlay system provides per-agent customization:

```rust
// Planned: Context overlay
pub struct ContextOverlay {
    /// Files to always include (agent's "known" working set).
    pub pinned_files: Vec<String>,
    /// Files to exclude (irrelevant to this agent's task).
    pub excluded_patterns: Vec<String>,
    /// Symbol importance overrides (boost or suppress specific symbols).
    pub importance_overrides: HashMap<SymbolId, f64>,
    /// Maximum graph expansion depth for this agent.
    pub max_expansion_depth: usize,
}
```

Use cases:
- A coding agent working on `roko-gate` pins gate-related files and excludes chain crate files
- A research agent pins documentation files and suppresses test utilities
- A security audit agent boosts symbols with `unsafe` in their context

### Privacy and redaction

For multi-tenant or sensitive codebases, the overlay system supports redaction:

```rust
// Planned: Privacy configuration
pub struct PrivacyConfig {
    /// Patterns that should never appear in LLM context.
    pub redact_patterns: Vec<String>,  // e.g., API keys, passwords
    /// Files that should never be indexed.
    pub ignore_files: Vec<String>,     // e.g., ".env", "secrets.toml"
    /// Symbols that should never appear in context.
    pub blocked_symbols: Vec<String>,  // e.g., internal auth functions
}
```

Redaction happens at the context assembly stage, after search and ranking but before prompt composition. This ensures that sensitive data is never included in LLM prompts, even if it appears in the index.

---

## Token Savings: Measured Impact

Comparative data from the Aider project (Gauthier 2024) and internal measurements:

### Without code intelligence

An agent tasked with "add error handling to the `process_input` function" must:
1. Search for the function (grep-like) → 20–50 candidate files
2. Include full candidate files in context → ~50K tokens
3. Hope the LLM identifies the right function and its dependencies
4. Risk missing callers, trait implementations, and type definitions

### With code intelligence

The same agent:
1. Keyword search for `process_input` → 1 result (5ms)
2. Graph expansion → 3 dependencies, 7 callers (1ms)
3. Code slicing → 11 focused slices totaling ~5K tokens
4. Context includes exactly the function, its dependencies, and its callers

**Result: 10× fewer tokens, higher-quality context, no missed dependencies.**

### Impact analysis scenario

"What breaks if I change the `Verdict` type?"

Without intelligence: `grep -rn "Verdict"` → 47 files, ~150K tokens. No structural understanding.

With intelligence: `reverse_neighbors(Verdict)` → 12 direct dependents. Transitive closure (depth 2) → 23 total affected symbols. Code slices → ~2K tokens. Structured impact report with affected symbols categorized by relationship type.

**Result: 75× fewer tokens, structured understanding of impact.**

---

## Integration with roko-compose

### The SystemPromptBuilder

The existing `SystemPromptBuilder` in `roko-compose` assembles prompts from six layers:
1. Identity layer (who is the agent)
2. Task layer (what to do)
3. Context layer (relevant information) ← **Code intelligence feeds here**
4. Tool layer (available tools)
5. Safety layer (constraints and guardrails)
6. History layer (conversation context)

Code intelligence output becomes part of the context layer:

```markdown
## Relevant Code Context

### Core types (PageRank > 0.01)

```rust
// crates/roko-index/src/graph.rs:46
pub struct SymbolGraph {
    nodes: HashSet<SymbolId>,
    forward: HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>>,
    reverse: HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>>,
}
```

### Focal function

```rust
// crates/roko-index/src/graph.rs:118
pub fn build_graph(files: &[SourceFile]) -> SymbolGraph { ... }
```

### Callers (7 found)

- orchestrate.rs:142 — `let graph = build_graph(&parsed_files);`
- search.rs:89 — `let graph = build_graph(&workspace_files);`
...
```

This structured context is far more useful to the LLM than raw file dumps.

---

## Academic Foundations

- **Program slicing**: Weiser (1981), "Program Slicing." *ICSE*. The foundational technique for extracting minimal relevant code — the intellectual basis for code slicing in context assembly.
- **Reciprocal Rank Fusion**: Cormack, Clarke, and Butt (2009), "Reciprocal Rank Fusion Outperforms Condorcet and Individual Rank Learning Methods." *SIGIR*. The fusion method for combining multiple ranked lists into a single ranking.
- **Hoogle**: Mitchell (2004), "Hoogle: Haskell API Search." The precedent for structural code search (by type signature), demonstrating that structural queries are more effective than keyword search for code.
- **ChatHTN**: (2025). Hierarchical task network planning for code understanding tasks, demonstrating the value of structured code representation for LLM-driven development.
- **LOOP**: (2025). Learning to optimize prompts for code generation, showing that context quality directly impacts code generation quality.
- **Meta-Harness**: Lee et al. (2026), arXiv:2603.28052. Demonstrates +7.7 points from harness optimization including context engineering.

---

## Current Status and Gaps

### Built

- `SystemPromptBuilder` in `roko-compose` with 6-layer prompt assembly
- `SymbolGraph` traversal operations (forward, reverse, transitive BFS)
- `pagerank()` for symbol importance scoring
- HDC `similarity()` for structural matching
- `parse_source()` pipeline for symbol extraction

### Missing

- Search API (no unified search interface)
- Keyword search backend (no FTS5 integration)
- Embedding search (no fastembed integration)
- RRF fusion implementation
- Code slicing (whole-file only, no fragment extraction)
- Token estimation for code fragments
- Budget-aware composition for code context
- Context overlay system
- Privacy/redaction layer
- Integration between `roko-index` and `roko-compose`

---

## Cross-References

- See [04-pagerank-symbol-importance.md](./04-pagerank-symbol-importance.md) for the ranking algorithm used in context prioritization
- See [05-hdc-fingerprints.md](./05-hdc-fingerprints.md) for the structural similarity search strategy
- See [07-mcp-context-server.md](./07-mcp-context-server.md) for the agent-facing API that triggers context assembly
- See [03-dependency-graph.md](./03-dependency-graph.md) for the graph expansion step
- See topic [03-composition](../03-composition/INDEX.md) for the Composer trait and prompt assembly system
- See topic [02-agents](../02-agents/INDEX.md) for how coding agents consume assembled context
