# Search and Context Assembly

> Depth for the Search Cell (Route protocol) and Assemble Cell (Compose protocol). Covers
> the 5-strategy search system, Reciprocal Rank Fusion, the 6-step assembly pipeline, VCG
> auction integration, and empirical token savings.

---

## Multi-Strategy Search as a Route Cell

The Search Cell implements the **Route protocol** -- selecting among candidates based on
query intent. See [02-CELL.md](../../unified/02-CELL.md) for the Route protocol definition.

The Route Cell does not execute searches directly. It selects which search strategy Cells to
invoke, dispatches the query, and combines results. Each strategy is itself a Cell:

### Strategy 1: KeywordSearch Cell

Full-text search over symbol names and documentation using SQLite FTS5 with BM25 ranking.

```rust
pub struct KeywordQuery {
    pub text: String,
    pub scope: SearchScope,     // Files, symbols, or both
    pub case_sensitive: bool,
    pub whole_word: bool,
}
```

FTS5 tokenization must handle code identifier conventions:

| Convention | Example | Tokens |
|---|---|---|
| snake_case | `process_input` | `process`, `input` |
| camelCase | `processInput` | `process`, `input` |
| PascalCase | `ProcessInput` | `process`, `input` |
| SCREAMING_SNAKE | `MAX_BUFFER_SIZE` | `max`, `buffer`, `size` |

A pre-processing step splits identifiers before FTS5 insertion. This requires a custom
tokenizer or the `unicode61` tokenizer with underscore as separator plus a camelCase
splitting pass.

**Best for**: specific name queries ("find the `build_graph` function").

### Strategy 2: StructuralSearch Cell

Query symbols by their structural properties -- kind, visibility, file pattern, PageRank
threshold:

```rust
pub struct StructuralQuery {
    pub kind: Option<SymbolKind>,
    pub visibility: Option<Visibility>,
    pub file_pattern: Option<String>,    // Glob
    pub has_callers: Option<bool>,
    pub min_pagerank: Option<f64>,
}
```

**Best for**: typed exploration ("find all public traits in roko-core", "find unused functions").

### Strategy 3: HdcSimilaritySearch Cell

Find symbols structurally similar to a query fingerprint via Hamming distance:

```rust
pub struct HdcQuery {
    pub anchor: HdcFingerprint,
    pub min_similarity: f64,    // e.g., 0.6
    pub max_results: usize,
}
```

The query fingerprint can be computed from an existing symbol, a synthetic description, or a
code snippet. See [03-hdc-fingerprints-and-similarity.md](03-hdc-fingerprints-and-similarity.md).

**Best for**: "find similar" queries, clone detection, pattern matching.

### Strategy 4: EmbeddingSimilaritySearch Cell

Dense vector cosine similarity for semantic queries (feature-gated behind `embedding`):

```rust
pub struct EmbeddingQuery {
    pub text: String,
    pub embedding: Vec<f32>,    // 384-dim for BGE-small
    pub max_results: usize,
    pub min_similarity: f32,
}
```

**Best for**: intent-based queries ("find error handling code" matches functions dealing with
errors even if the name doesn't contain "error").

### Strategy 5: HybridSearch Cell (RRF)

Reciprocal Rank Fusion (Cormack, Clarke, and Butt 2009) combines ranked lists from multiple
strategies:

```
RRF_score(symbol) = SUM_strategy  1 / (k + rank_strategy(symbol))
```

Where k = 60 (standard constant). The formula rewards symbols that appear in multiple result
lists and penalizes symbols that appear in only one.

```rust
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
    // ... same for structural, hdc, embedding
    let mut combined: Vec<_> = rrf_scores.into_iter().collect();
    combined.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    combined
}
```

**Best for**: open-ended exploration, broad task descriptions. The default strategy.

### Route Cell Strategy Selection

The Route Cell decides which strategies to invoke based on query intent analysis:

| Query pattern | Strategies invoked |
|---|---|
| Specific name (`build_graph`) | Keyword only (fast path) |
| "Find similar to X" | HDC + Embedding |
| Typed exploration ("all public traits") | Structural only |
| Broad task description | All five via RRF (hybrid) |

This is EFE-style gating: the Route Cell estimates the expected information gain from each
strategy and invokes only those whose expected gain exceeds their cost. Keyword search is
nearly free; embedding search costs ~10ms per query.

---

## The 6-Step Context Assembly Pipeline

The Assemble Cell implements the **Compose protocol** -- assembling context under a token
budget. See [02-CELL.md](../../unified/02-CELL.md) for the Compose protocol definition.

```
Task description
    |
    v
Step 1: PARSE QUERY     -- extract search terms, intent, focal symbols
    |
    v
Step 2: MULTI-STRATEGY  -- run applicable strategies (parallel)
    |
    v
Step 3: RANK (RRF)      -- combine results into single ranked list
    |
    v
Step 4: EXPAND GRAPH    -- add graph neighbors of top results (1-2 hops)
    |
    v
Step 5: SLICE            -- extract relevant code fragments (not whole files)
    |
    v
Step 6: BUDGET           -- fit into token budget, prioritizing by rank
    |
    v
Context block (ready for SystemPromptBuilder)
```

### Step 1: Parse Query

Analyze the task description to extract search parameters:
- **Explicit symbol mentions**: "modify the `build_graph` function" -> keyword search
- **Kind hints**: "add a new trait" -> structural search for existing traits
- **Similarity intent**: "like the existing Gate implementation" -> HDC similarity
- **Scope hints**: "in the roko-index crate" -> file pattern filter

### Step 2: Multi-Strategy Search

Applicable strategies run in parallel. Not all strategies apply to every query.

### Step 3: Rank via RRF

Results from all strategies are combined. Symbols appearing across multiple strategies rise
to the top.

### Step 4: Graph Expansion

Top-ranked symbols are expanded using the dependency graph:
- **Forward expansion (depth 1)**: include what focal symbols depend on
- **Reverse expansion (depth 1)**: include what depends on focal symbols
- **Contextual expansion**: include symbols in the same file

This step fills in structural context: search may find a function without its type definitions,
or a struct without its constructor. Graph expansion adds the neighborhood.

### Step 5: Slice

Extract minimal code fragments rather than whole files:

```rust
pub struct CodeSlice {
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    pub symbols_included: Vec<SymbolId>,
    pub token_estimate: usize,
}
```

Program slicing (Weiser 1981): include only the code relevant to the current computation.
For a function: signature + body + doc comments. For a struct: definition + key methods.
Token estimation at ~4 characters per token (heuristic for code).

### Step 6: Budget-Aware Composition

This is the same **VCG auction** mechanism used for general prompt assembly (see
[02-CELL.md](../../unified/02-CELL.md) -- Compose protocol). Code context sections bid for
attention alongside knowledge Signals, task description, system prompt, and conversation
history.

```rust
// Code slices wrapped as Signals, scored by PageRank x RRF rank
pub trait Composer {
    fn compose(
        &self,
        candidates: &[Signal],  // Code slices as Signals
        scorer: &dyn Scorer,    // PageRank x RRF composite score
        budget: TokenBudget,    // Total minus system prompt, instructions, history
    ) -> ComposedContext;       // Ordered list of slices that fit
}
```

Budget allocation:
```
token_budget(symbol) = total_budget * (pagerank(symbol) / SUM(pagerank(included)))
```

A symbol with twice the PageRank of another gets twice the token budget -- more of its
surrounding code, documentation, and context is included.

### Context Overlay System

Different agents may need different views. Per-agent overlays customize the index:

```rust
pub struct ContextOverlay {
    pub pinned_files: Vec<String>,         // Always include
    pub excluded_patterns: Vec<String>,    // Never include
    pub importance_overrides: HashMap<SymbolId, f64>,
    pub max_expansion_depth: usize,
}
```

### Privacy and Redaction

A pre-Compose **Verify Cell** checks for sensitive content before inclusion in the prompt:

```rust
pub struct PrivacyConfig {
    pub redact_patterns: Vec<String>,   // e.g., API keys, passwords
    pub ignore_files: Vec<String>,      // e.g., ".env", "secrets.toml"
    pub blocked_symbols: Vec<String>,   // e.g., internal auth functions
}
```

Redaction happens after search/ranking but before prompt composition. Sensitive data is never
sent to the LLM, even if it appears in the index.

---

## Integration with SystemPromptBuilder

The existing `SystemPromptBuilder` in `roko-compose` assembles prompts from multiple layers.
Code intelligence feeds the **context layer**:

```markdown
## Relevant Code Context

### Core types (PageRank > 0.01)
// crates/roko-index/src/graph.rs:46
pub struct SymbolGraph {
    nodes: HashSet<SymbolId>,
    forward: HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>>,
    reverse: HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>>,
}

### Focal function
// crates/roko-index/src/graph.rs:118
pub fn build_graph(files: &[SourceFile]) -> SymbolGraph { ... }

### Callers (7 found)
- orchestrate.rs:142: let graph = build_graph(&parsed_files);
- search.rs:89: let graph = build_graph(&workspace_files);
```

This structured context is far more useful to the LLM than raw file dumps.

---

## Empirical Token Savings

### Without intelligence
Agent tasked with "add error handling to `process_input`":
1. grep-like search -> 20--50 candidate files
2. Include full files -> ~50K tokens
3. LLM must identify the right function and dependencies
4. Risk missing callers, trait implementations, type definitions

### With intelligence
Same agent:
1. Keyword search -> 1 result (5ms)
2. Graph expansion -> 3 deps, 7 callers (1ms)
3. Code slicing -> 11 focused slices, ~5K tokens
4. Context includes exactly the function, dependencies, and callers

**Result: 10x fewer tokens, no missed dependencies.**

### Impact analysis scenario
"What breaks if I change the `Verdict` type?"

Without: `grep -rn "Verdict"` -> 47 files, ~150K tokens. No structural understanding.

With: `reverse_neighbors(Verdict)` -> 12 direct dependents. Transitive(depth=2) -> 23 total.
Code slices -> ~2K tokens. Structured impact report by relationship type.

**Result: 75x fewer tokens, structured understanding.**

---

## What This Enables

1. **Optimal context for every task** -- the Assemble Cell produces the best possible
   context block within any token budget, by combining search, ranking, graph expansion,
   slicing, and VCG auction.
2. **Multi-strategy search covers all query types** -- keyword for exact names, structural
   for typed exploration, HDC for similarity, embedding for semantics, hybrid for broad tasks.
3. **Privacy-safe code intelligence** -- sensitive patterns are redacted before reaching the
   LLM, via a Verify Cell in the assembly pipeline.
4. **Per-agent customization** -- context overlays let different agents see different views
   of the same codebase.

## Feedback Loops

- **Strategy effectiveness tracking**: the Route Cell tracks which strategies led to
  successful task completion (gate pass). Over time, Thompson sampling selects the most
  effective strategy mix per query type.
- **Context quality calibration**: the Assemble Cell publishes context quality predictions
  (expected gate pass rate for this context). After the gate runs, the prediction error
  feeds back to adjust the VCG section effect weights -- which code sections correlate with
  gate success.
- **Token budget learning**: if agents consistently use less than their allocated budget for
  code context (because the context is too broad), the budget allocation shrinks. If they
  consistently fail with insufficient context, it grows. Beta-Binomial tracker.

## Open Questions

1. Should the 6-step pipeline be a fixed sequence, or should it be adaptive (e.g., skip
   graph expansion if the search results are already complete)?
2. Should code slicing be line-based (current design) or AST-based (requires tree-sitter)?
   AST-based slicing would produce cleaner fragments but adds a hard tree-sitter dependency.
3. How should the VCG auction weigh code context against knowledge context? A coding task
   needs mostly code; a research task needs mostly knowledge. Should the weight be task-type
   driven or learned?

## Implementation Tasks

| Task | File paths | Priority |
|---|---|---|
| Implement unified search API | `crates/roko-index/src/lib.rs` | Tier 0 |
| Implement keyword search (in-memory) | `crates/roko-index/src/lib.rs` | Tier 0 |
| Wire search results into SystemPromptBuilder | `crates/roko-compose/src/system_prompt_builder.rs` | Tier 0 |
| Implement RRF hybrid search | `crates/roko-index/src/` (new file) | Tier 1 |
| Implement code slicing (line-based) | `crates/roko-index/src/` (new file) | Tier 1 |
| Implement graph expansion step | `crates/roko-index/src/graph.rs` | Tier 1 |
| Implement token budget allocation | `crates/roko-index/src/` (new file) | Tier 1 |
| Implement context overlay system | `crates/roko-index/src/` (new file) | Tier 2 |
| Implement privacy/redaction Verify Cell | `crates/roko-index/src/` (new file) | Tier 2 |
| Add embedding search (feature-gated) | `crates/roko-index/Cargo.toml`, new file | Tier 2 |
| Add structural search (SQL-backed) | `crates/roko-index/src/sqlite.rs` | Tier 1 |
