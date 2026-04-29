# Symbol Graph and Importance Scoring

> Depth for code intelligence graph and scoring subsystems. Covers the dependency graph as
> a Store of Signals with typed edges, the Score Cell implementing PageRank, parsing via
> LanguageProvider, and planned enhancements (tree-sitter, weighted/personalized PageRank).

---

## Symbols as Signals

Every symbol extracted from source code is a **Signal** -- the universal durable datum
defined in [01-SIGNAL.md](../../unified/01-SIGNAL.md). Each symbol is content-addressed
via BLAKE3 hash of its source text, typed via `Kind::CodeSymbol`, and carries the standard
5-axis score. The mapping from code to Signal:

| Symbol property | Signal field | Notes |
|---|---|---|
| BLAKE3(source text) | `hash` | Content address for dedup and change detection |
| `SymbolKind` | `kind` subtype | Function, Struct, Enum, Trait, Const, Type, Module, Impl |
| PageRank score | `utility` axis | Structural importance |
| HDC similarity to task | `salience` axis | Task relevance |
| Stability across re-indexes | `confidence` axis | Reliably important symbols score high |
| Demurrage-weighted age | `balance` | Recent retrievals restore balance |
| Parent Signal hash | `lineage` | File-level Signal is the parent |

Symbol identity is the triple `(file_path, symbol_name, kind)` -- captured in `SymbolId`.
Two symbols with the same name but different kinds are distinct (a `struct Config` and an
`fn Config` constructor). Two symbols with the same name and kind in different files are
distinct. This identity scheme is implemented and tested in `crates/roko-index/src/symbol.rs`.

### Cross-Language Mapping

The 8-variant `SymbolKind` enum normalizes constructs across languages:

| Rust | TypeScript | Go | SymbolKind |
|---|---|---|---|
| `fn` | `function` | `func` | `Function` |
| `struct` | `class` | `type X struct` | `Struct` |
| `enum` | `enum` | -- | `Enum` |
| `trait` | `interface` | `type X interface` | `Trait` |
| `const` | `const` | `const` / `var` | `Const` |
| `type` | `type` | `type` (non-struct) | `Type` |
| `mod` | -- | -- | `Module` |
| `impl` | -- | -- | `Impl` |

This uniform mapping means the Graph Cell and Score Cell treat symbols identically regardless
of source language. A Go `interface` and a Rust `trait` both produce `SymbolKind::Trait` nodes
in the graph, enabling cross-language structural comparison via HDC fingerprints.

---

## Parsing: The Connect Protocol

The Parse Cell implements the Connect protocol -- external system I/O with lifecycle management.
See [02-CELL.md](../../unified/02-CELL.md) for the Connect protocol definition.

### The LanguageProvider Trait

All parsing flows through a single trait in `roko-core`:

```rust
pub trait LanguageProvider: Send + Sync {
    fn language_name(&self) -> &str;
    fn file_extensions(&self) -> &[&str];
    fn parse_imports(&self, source: &str) -> Vec<Import>;
    fn extract_symbols(&self, source: &str) -> Vec<Symbol>;
}
```

The Parse Cell delegates to the appropriate provider based on file extension. The `roko-index`
crate itself contains zero language-specific logic -- all language knowledge lives in providers.
Adding Python support means implementing `PythonLanguageProvider`; every downstream Cell works
unchanged.

### Current: Heuristic Parsers

Three providers use line-by-line heuristic parsing (~2,336 lines combined):

- **Rust** (`roko-lang-rust`, 819 lines): scans for `fn`, `struct`, `enum`, `trait`, `impl`,
  `const`, `type`, `mod`. Handles `pub`, `pub(crate)`, `pub(super)` visibility. Parses
  `use` imports with brace expansion. Skips angle brackets in generic signatures.
- **TypeScript** (`roko-lang-typescript`, 917 lines): scans for `function`, `class`,
  `interface`, `enum`, `const`, `type`. Maps `class` to `Struct`, `interface` to `Trait`.
  Parses ES module imports, type-only imports, and CommonJS `require`.
- **Go** (`roko-lang-go`, 600 lines): scans for `func`, `type X struct`, `type X interface`,
  `const`, `var`. Uses capitalization convention for visibility. Parses single and grouped imports.

**Known limitations of heuristic parsers:**
- Cannot parse nested definitions (closures, inner functions)
- Multi-line signatures where keyword and name are on different lines
- Macro-generated items are invisible
- No call-site extraction (cannot produce `Calls` edges)
- No scope nesting (flat symbol list only)
- No column-level source locations

### Planned: Tree-Sitter

Tree-sitter (Brunsfeld 2018) provides full AST parsing with incremental updates, error recovery,
and consistent API across 300+ languages. The upgrade path preserves backward compatibility:

```rust
pub struct TreeSitterProvider {
    language: tree_sitter::Language,
    queries: LanguageQueries,  // Pre-compiled tree-sitter queries
}

impl LanguageProvider for TreeSitterProvider {
    fn extract_symbols(&self, source: &str) -> Vec<Symbol> {
        // Use tree-sitter query to find definition nodes
    }
    fn parse_imports(&self, source: &str) -> Vec<Import> {
        // Use tree-sitter query to find import nodes
    }
}
```

**Key capabilities tree-sitter enables that heuristics cannot:**

| Capability | Heuristic | Tree-sitter |
|---|---|---|
| Nested definitions | Missed | Captured at correct scope |
| Multi-line signatures | Fragile | Robust |
| Call site extraction | Impossible | Via `call_expression` node traversal |
| Scope-aware lookup | Impossible | Natural via AST depth |
| Error recovery (broken code) | Crashes/misparses | Partial tree with ERROR nodes |
| Incremental re-parse (1-line edit) | Full re-parse (~2ms) | ~50us (reuse unchanged subtrees) |

Tree-sitter's incremental parsing is the critical advantage: `parser.parse(text, Some(old_tree))`
reuses all unchanged subtrees via a `ReusableNode` tracker. Only regions overlapping the edit
are re-parsed. For typical agent-driven modifications (small, focused changes), re-indexing
is essentially free.

---

## The Dependency Graph: A Store of Typed Edges

The `SymbolGraph` is a **Store Cell** -- it implements the Store protocol for persisting and
retrieving Symbol Signals with typed edges. See [02-CELL.md](../../unified/02-CELL.md) for
the Store protocol definition.

### Data Structure

```rust
pub struct SymbolGraph {
    nodes: HashSet<SymbolId>,
    forward: HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>>,  // X depends on Y
    reverse: HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>>,  // Y is depended on by X
}
```

Dual adjacency lists enable O(1) lookup in either direction. For ~10K symbols and ~30K edges,
memory cost is ~2MB.

### Edge Types

```rust
#[non_exhaustive]
pub enum EdgeKind {
    Imports,     // A imports B (use/require/import)     -- Built
    Calls,       // A calls B (function/method call)     -- Needs tree-sitter
    Implements,  // A implements B (trait/interface)      -- Needs tree-sitter
    Contains,    // A contains B (method in impl block)  -- Needs tree-sitter
}
```

Edges are Signal lineage links -- but lateral, not parent-child. Import edges express
"this symbol references that symbol." Call edges express "this function invokes that function."
These are the same lineage mechanism used for Signal provenance in
[01-SIGNAL.md](../../unified/01-SIGNAL.md), applied to code structure.

### 3-Phase Construction Algorithm

```
Phase 1: Node registration        O(S)
    for each file, for each symbol:
        nodes.insert(SymbolId(file, name, kind))

Phase 2: Name-to-ID lookup        O(S)
    name_to_ids: HashMap<&str, Vec<SymbolId>>
    (handles name collisions across files)

Phase 3: Import edge creation     O(F x I x M)
    for each file, for each import:
        extract last segment of import path
        match against name_to_ids
        skip self-file edges
        create forward + reverse edge pair

Total: O(S + F x I x M)
    S = total symbols (~5K for Roko)
    F = files (~300)
    I = avg imports per file (~15)
    M = avg name matches (~1.2)
    => O(5000 + 300 x 15 x 1.2) = O(10,400)  < 1ms
```

### Store Protocol Operations

| Store operation | Graph implementation |
|---|---|
| `put(signal)` | `nodes.insert(id)` |
| `get(id)` | `forward.get(id)` / `reverse.get(id)` |
| `query(filter)` | Transitive closure via BFS with depth limit |
| `query_similar(hdc)` | Not applicable (HDC similarity is in Fingerprint Cell) |
| `prune(criteria)` | Remove symbols below demurrage threshold |

---

## PageRank: The Score Cell

PageRank computes structural importance for every symbol in the graph. It is a **Score Cell**
implementing the Score protocol -- rating each Symbol Signal along the "utility" dimension.
See [02-CELL.md](../../unified/02-CELL.md) for the Score protocol definition.

### The Algorithm

```
PR(v) = (1 - d) / N + d * SUM(PR(u) / out_degree(u))
                         for each u that links to v
```

Where d = 0.85 (damping factor), N = total nodes. The damping factor models the probability
that importance flows through dependencies (85%) versus a baseline floor (15%).

### Implementation

The built implementation in `crates/roko-index/src/graph.rs`:

```rust
pub fn pagerank(
    graph: &SymbolGraph,
    iterations: u32,
    damping: f64,
) -> HashMap<SymbolId, f64> {
    // Initialize: every node gets 1/N
    // Iterate: new_rank[v] = (1-d)/N + d * SUM(rank[u] / out_degree(u))
    // Uses mul_add for floating-point precision
    // Floors out_degree at 1 (handles dangling nodes)
}
```

Properties verified by tests:
- Star topology: hub node gets highest rank (3 spokes all import hub)
- Cycle topology: all nodes get approximately equal rank (diff < 0.01)
- Empty graph: returns empty map without panic

### Convergence

| Property | Value |
|---|---|
| Convergence rate | Geometric with rate d = 0.85 |
| Iterations for < 0.001 error | ~30 |
| Time for 5K nodes, 30 iterations | < 1ms |
| Time for 50K nodes, 30 iterations | ~10ms (estimated) |

### What PageRank Captures (and Does Not)

High PageRank means many important symbols depend on this symbol. In code:

| Pattern | Typical rank | Why |
|---|---|---|
| Core types (Signal, Error, Config) | Top 1% | Imported everywhere |
| Trait definitions (Gate, Scorer) | Top 5% | Implemented by many types |
| Entry points (main, run) | Top 15% | High out-degree |
| Module-internal helpers | Bottom 50% | Few external imports |
| Dead code | Bottom 5% | Zero in-links |

PageRank does NOT capture task relevance, recency, or semantic meaning. These are addressed
by the planned enhancements below.

---

## Planned: Weighted and Personalized PageRank

### Weighted PageRank as a Loop

Weighted PageRank is a **Loop** -- a Graph with a feedback edge from output back to input.
See [03-GRAPH.md](../../unified/03-GRAPH.md) for the Loop pattern definition.

The Loop: task context adjusts edge weights -> PageRank re-scores -> agent uses context ->
gate evaluates output -> gate pass/fail feeds back to weight adjustment.

```
                  +---> Score Cell (weighted PageRank) ---+
                  |                                       |
Task context --+--+                                       v
               |                                    Search Cell
               |                                       |
               +--- React Cell <-- gate.verdict <------+
                    (adjust weights)
```

Edge weight scheme from the project's design history:

| Condition | Weight | Rationale |
|---|---|---|
| Symbol mentioned in task prompt | 10x | Direct task relevance |
| Symbol in currently open file | 50x | Active working context |
| Recently modified symbol | 5x | Recency bias |
| Private/crate-internal symbol | 0.1x | Less cross-module relevance |
| Test file symbol | 0.5x | Tests depend on code, not vice versa |

### Personalized PageRank (PPR)

PPR replaces uniform teleportation with a biased distribution. Instead of jumping to any
random node with probability (1-d)/N, the random surfer teleports to task-relevant seed
nodes with higher probability:

```
PPR(v) = (1 - d) * teleport(v) + d * SUM(PPR(u) / out_degree(u))
```

This biases the entire ranking toward symbols structurally close to the current task.

For large graphs, the **push algorithm** (Andersen, Chung, and Lang 2006) computes approximate
local PPR with running time proportional to the output set, not the graph size:

```
push_ppr(graph, seed, alpha=0.15, epsilon=1e-4):
    residual = {seed: 1.0}
    estimate = {}
    while any residual[v] > epsilon * out_degree(v):
        pop v with highest residual
        estimate[v] += alpha * residual[v]
        push = (1 - alpha) * residual[v] / out_degree(v)
        for neighbor u of v:
            residual[u] += push
        residual[v] = 0
    return estimate
```

This is ideal for the MCP `get_context` tool: given a few task-relevant seed symbols, compute
local PPR to find structurally relevant neighbors without scanning the entire graph.

### Topic-Sensitive Multi-Concern Ranking

Pre-compute 5 PPR vectors, one per development concern:

| Concern | Seed heuristic |
|---|---|
| Architecture | Symbols with high import in-degree |
| Testing | Symbols in test files |
| API surface | Public symbols |
| Recent activity | Recently modified files |
| Current task | Task-mentioned symbols |

At query time, interpolate: `score = SUM(w_k * PPR_k(v))`. This gives topic-sensitive ranking
without per-query PPR computation.

### Dual-Process Routing Integration

PageRank integrates with the Route protocol's EFE gating (see
[05-AGENT.md](../../unified/05-AGENT.md)):

| Tier | PageRank range | Cognitive investment |
|---|---|---|
| T0 (heuristic, no LLM) | Low rank symbols | Pattern-matching heuristics |
| T1 (fast model) | Medium rank | Lightweight LLM reasoning |
| T2 (full model) | High rank (core types, public API) | Full reasoning + extended context |

---

## What This Enables

1. **Dependency-aware context** -- agents understand not just what a symbol IS, but what it
   depends on and what depends on it. Impact analysis is graph traversal, not grep.
2. **Importance-ranked token allocation** -- the Compose protocol allocates more tokens to
   structurally important symbols. A core `Signal` struct gets full context; a test helper
   gets a one-line signature.
3. **Cross-language structural analysis** -- uniform symbol types enable comparing a Rust
   trait and a Go interface in the same graph.
4. **Incremental, zero-latency re-indexing** -- tree-sitter's incremental parsing means
   re-indexing after an edit takes microseconds, not milliseconds.

## Feedback Loops

- **PageRank weight learning**: weighted PageRank is a Loop. Gate outcomes update edge weights.
  Symbols whose inclusion correlated with gate passes get boosted. This is predict-publish-correct
  applied to structural importance.
- **Parse accuracy tracking**: the Parse Cell publishes parse health Signals (error count, missing
  count from tree-sitter ERROR/MISSING nodes). A high error rate on a file triggers fallback
  to heuristic parsing. Over time, parse health per-language is a calibrated metric.
- **Graph completeness learning**: when agents report missing context ("I needed X but it
  wasn't in my context"), the feedback propagates as a negative Signal on the Search Cell's
  strategy selection. The Route Cell adjusts to include more graph expansion.

## Open Questions

1. Should `Calls` edges be weighted by call frequency (from runtime profiling data) or treated
   as uniform? Weighted calls would capture "hot paths" but require runtime instrumentation.
2. Should PDG-style edges (control dependence, data flow) be added to `EdgeKind`? They enable
   precise program slicing but require expensive analysis (CFG construction + dominance).
3. How should the graph handle monorepo-scale workspaces (500K+ symbols)? Local PPR scales
   well, but full PageRank at 500K nodes takes ~100ms.

## Implementation Tasks

| Task | File paths | Priority |
|---|---|---|
| Add `Calls` edge extraction via tree-sitter | `crates/roko-lang-rust/src/lib.rs` | Tier 1 |
| Add `Implements` edge extraction | `crates/roko-lang-rust/src/lib.rs` | Tier 1 |
| Add `Contains` edge extraction | `crates/roko-lang-rust/src/lib.rs` | Tier 1 |
| Implement weighted PageRank | `crates/roko-index/src/graph.rs` | Tier 2 |
| Implement Personalized PageRank | `crates/roko-index/src/graph.rs` | Tier 2 |
| Implement push algorithm for local PPR | `crates/roko-index/src/graph.rs` | Tier 2 |
| Add early termination to PageRank | `crates/roko-index/src/graph.rs` | Tier 2 |
| Add tree-sitter dependency (feature-gated) | `crates/roko-lang-rust/Cargo.toml` | Tier 1 |
| Add `TreeSitterProvider` for Rust | `crates/roko-lang-rust/src/lib.rs` | Tier 1 |
| Add `backward_slice()` graph operation | `crates/roko-index/src/graph.rs` | Tier 2 |
| Add `impact_analysis()` API | `crates/roko-index/src/graph.rs` | Tier 2 |
| Add `extract_subgraph()` for context | `crates/roko-index/src/graph.rs` | Tier 2 |
