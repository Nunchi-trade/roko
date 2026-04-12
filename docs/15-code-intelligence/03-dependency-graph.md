# Dependency Graph

> Directed symbol dependency graph with forward/reverse traversal — the structural backbone for impact analysis, context prioritization, and PageRank scoring.


> **Implementation**: Built

**Topic**: [Code Intelligence](./INDEX.md)
**Prerequisites**: [02-symbol-extraction.md](./02-symbol-extraction.md)
**Key sources**: `crates/roko-index/src/graph.rs`, `bardo-backup/tmp/mori-refactor/10-code-intelligence.md`, `bardo-backup/tmp/mori-agents/18-code-intelligence-and-gateway.md`

---

## Abstract

A codebase is not a collection of files — it is a graph of relationships. Functions call other functions. Types implement traits. Modules import definitions. These relationships determine how changes propagate, which symbols matter most, and what context an agent needs to understand a piece of code.

The `SymbolGraph` in `roko-index` captures these relationships as a directed graph where nodes are `SymbolId` instances and edges are typed dependency relationships. The graph supports forward traversal (what does this symbol depend on?), reverse traversal (what depends on this symbol?), transitive closure via BFS, and PageRank scoring for importance ranking.

This document covers the graph data structure, edge types, construction algorithm, traversal operations, and planned enhancements including call graph extraction and impact analysis.

---

## Graph Data Structure

### SymbolGraph

The core data structure uses adjacency lists for both forward and reverse edges:

```rust
#[derive(Clone, Debug)]
pub struct SymbolGraph {
    /// Set of all node ids.
    nodes: HashSet<SymbolId>,
    /// Forward edges: from -> list of (to, kind).
    forward: HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>>,
    /// Reverse edges: to -> list of (from, kind).
    reverse: HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>>,
}
```

The dual-adjacency design is intentional:

- **Forward edges** answer "what does symbol X depend on?" — critical for transitive dependency analysis and understanding what a symbol needs to function.
- **Reverse edges** answer "what depends on symbol X?" — critical for impact analysis, finding callers, and determining how important a symbol is.

Maintaining both directions in sync costs 2× memory for edges but enables O(1) lookup in either direction. For a workspace with ~10K symbols and ~30K edges, this is ~2MB — negligible.

### Edge types

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EdgeKind {
    /// One symbol calls another (function call, method call).
    Calls,
    /// One symbol imports another (use statement, require, import).
    Imports,
    /// One symbol implements a trait/interface.
    Implements,
    /// One symbol is contained within another (method in impl block).
    Contains,
}
```

Each edge kind represents a different structural relationship:

| Edge kind | Meaning | Example | Requires |
|---|---|---|---|
| `Imports` | A imports B | `use crate::config::Config;` | Import parsing (built) |
| `Calls` | A calls B | `config.validate()` | Call site analysis (tree-sitter) |
| `Implements` | A implements B | `impl Gate for CompileGate` | Impl resolution (tree-sitter) |
| `Contains` | A contains B | `impl Config { fn new() }` | Scope nesting (tree-sitter) |

Currently, only `Imports` edges are created by `build_graph()`. The other three edge kinds require tree-sitter's AST-level analysis to identify call sites, impl blocks, and nesting relationships.

### SymbolEdge

For serialization and external representation, edges are also available as standalone structs:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SymbolEdge {
    pub from_id: SymbolId,
    pub to_id: SymbolId,
    pub kind: EdgeKind,
}
```

---

## Graph Construction

### The build_graph algorithm

Graph construction is a three-phase process:

```rust
pub fn build_graph(files: &[SourceFile]) -> SymbolGraph {
    // Phase 1: Register all symbols as nodes
    // Phase 2: Build name -> SymbolId lookup
    // Phase 3: Create import edges
}
```

**Phase 1: Node registration**

Every symbol in every file becomes a node:

```rust
for file in files {
    for sym in &file.symbols {
        nodes.insert(SymbolId::from_symbol(sym, &file.path));
    }
}
```

This is a simple traversal: O(total_symbols) time and space.

**Phase 2: Name-to-ID lookup table**

A reverse index maps symbol names to their `SymbolId`s across all files:

```rust
let mut name_to_ids: HashMap<&str, Vec<SymbolId>> = HashMap::new();
for file in files {
    for sym in &file.symbols {
        name_to_ids
            .entry(&sym.name)
            .or_default()
            .push(SymbolId::from_symbol(sym, &file.path));
    }
}
```

Names can map to multiple IDs because the same name may appear in different files (e.g., `fn new()` in multiple modules). The `Vec<SymbolId>` value type handles this.

**Phase 3: Import edge creation**

For each file, the algorithm matches import paths against the name lookup table:

```rust
for file in files {
    let source_id = match file.symbols.first() {
        Some(sym) => SymbolId::from_symbol(sym, &file.path),
        None => continue,
    };

    for import in &file.imports {
        // Extract last segment: "std::collections::HashMap" -> "HashMap"
        let target_name = import.path
            .rsplit("::")
            .next()
            .or_else(|| import.path.rsplit('/').next())
            .or_else(|| import.path.rsplit('.').next())
            .unwrap_or(&import.path);

        if let Some(targets) = name_to_ids.get(target_name) {
            for target in targets {
                if target.file_path == file.path {
                    continue; // No self-file edges
                }
                forward.entry(source_id.clone()).or_default()
                    .push((target.clone(), EdgeKind::Imports));
                reverse.entry(target.clone()).or_default()
                    .push((source_id.clone(), EdgeKind::Imports));
            }
        }
    }
}
```

Key design decisions in Phase 3:

1. **Last-segment matching** — Import paths like `std::collections::HashMap` are matched by their last segment (`HashMap`). This works because symbol names within a workspace are typically unique at the name level. Ambiguity is handled by creating edges to all matching targets.

2. **Multi-separator support** — The algorithm tries `::` (Rust), then `/` (path-style), then `.` (TypeScript/Go) as separators. This enables cross-language import resolution without language-specific logic.

3. **Self-file exclusion** — If a file imports its own symbol (e.g., `use crate::Foo` within the file that defines `Foo`), no edge is created. Self-edges add noise without information.

4. **First-symbol source** — Import edges originate from the first symbol in the importing file. This is a simplification — ideally, edges would originate from the specific symbol that uses the import. Tree-sitter will enable more precise attribution.

### Complexity analysis

| Phase | Time | Space |
|---|---|---|
| Node registration | O(S) where S = total symbols | O(S) |
| Name lookup table | O(S) | O(S) |
| Edge creation | O(F × I × M) where F = files, I = avg imports, M = avg name matches | O(E) where E = total edges |
| Total | O(S + F × I × M) | O(S + E) |

For the Roko workspace (~5K symbols, ~3K imports, ~1.2 avg matches): O(5000 + 200 × 15 × 1.2) ≈ O(8,600). This completes in under 1ms.

---

## Graph Traversal Operations

### Forward neighbors

```rust
pub fn neighbors(&self, id: &SymbolId) -> Vec<&SymbolId> {
    self.forward
        .get(id)
        .map(|edges| edges.iter().map(|(target, _)| target).collect())
        .unwrap_or_default()
}
```

Returns all symbols that `id` depends on. Use cases:
- "What does this function need?" — Understanding dependencies before modification
- "What imports does this module pull in?" — Assessing coupling

### Reverse neighbors

```rust
pub fn reverse_neighbors(&self, id: &SymbolId) -> Vec<&SymbolId> {
    self.reverse
        .get(id)
        .map(|edges| edges.iter().map(|(source, _)| source).collect())
        .unwrap_or_default()
}
```

Returns all symbols that depend on `id`. Use cases:
- "Who calls this function?" — Impact analysis for API changes
- "What breaks if I change this struct?" — Safe refactoring

### Transitive closure (BFS)

```rust
pub fn transitive(&self, start: &SymbolId, max_depth: usize) -> Vec<(SymbolId, usize)> {
    // BFS from start, following forward edges, up to max_depth hops
    // Returns (reached_symbol, depth) pairs
}
```

The BFS traversal follows forward edges to find all transitive dependencies up to a configurable depth. It returns each reached symbol paired with its distance from the start node.

Use cases:
- **Depth 1** — Direct dependencies only. Fast, focused context.
- **Depth 2** — Dependencies of dependencies. Good for understanding a symbol's neighborhood.
- **Depth 3+** — Extended reach. Risk of context explosion; use with budget constraints.

The `max_depth` parameter prevents unbounded traversal in graphs with cycles or high connectivity.

---

## Practical Examples

### Example: Star topology (hub with spokes)

```
    A ──imports──→ Hub ←──imports── B
                    ↑
                    │
              C ──imports──┘
```

Three files (A, B, C) all import `Hub`. The graph has:
- 4 nodes: `Hub`, `A`, `B`, `C`
- 3 edges: all `Imports` pointing toward `Hub`
- `Hub.reverse_neighbors()` returns `[A, B, C]`
- `A.neighbors()` returns `[Hub]`
- `pagerank()` gives `Hub` the highest score (many in-links, few out-links)

This pattern is common in real codebases: core types like `Config`, `Error`, `Context` are imported by many modules.

### Example: Chain topology

```
    Top ──imports──→ Mid ──imports──→ Core
```

A linear dependency chain:
- `Top.transitive(depth=1)` returns `[(Mid, 1)]`
- `Top.transitive(depth=2)` returns `[(Mid, 1), (Core, 2)]`
- `Core.reverse_neighbors()` returns `[Mid]` (not `Top` — only direct)
- `pagerank()` gives `Core` the highest score (end of the chain, no outgoing edges)

### Example: Cycle

```
    A ──→ B ──→ C ──→ A
```

Three symbols in a circular dependency:
- `pagerank()` gives all three roughly equal scores (verified by test: diff < 0.01)
- `transitive()` with any start and sufficient depth reaches all three
- The BFS `visited` set prevents infinite loops

---

## Planned Enhancements

### Call graph edges

Tree-sitter enables extraction of function call sites, producing `Calls` edges:

```rust
// Planned: Extract call edges from tree-sitter AST
fn extract_call_edges(
    tree: &tree_sitter::Tree,
    source: &str,
    file_path: &str,
    name_lookup: &HashMap<&str, Vec<SymbolId>>,
) -> Vec<SymbolEdge> {
    // Query for call_expression nodes
    // Match called function name against known symbols
    // Create Calls edges
}
```

Call edges are dramatically more informative than import edges because they capture actual runtime dependencies, not just namespace declarations.

### Weighted edges

The legacy design (from `bardo-backup/tmp/mori-agents/18-code-intelligence-and-gateway.md`) describes edge weighting for more accurate PageRank:

| Condition | Weight multiplier | Rationale |
|---|---|---|
| Symbol mentioned in current task | 10× | Direct relevance |
| File currently in agent's context | 50× | Active working set |
| Private/internal symbol | 0.1× | Less likely to be relevant externally |
| Recently modified symbol | 5× | Recency bias for active development |
| Test file dependency | 0.5× | Tests depend on code, not vice versa |

Weighted PageRank would replace the uniform edge treatment in the current implementation, producing scores that better reflect task-relevant importance.

### Impact analysis

Combining reverse traversal with edge types enables impact analysis:

```rust
// Planned: Impact analysis API
pub fn impact_analysis(
    graph: &SymbolGraph,
    changed: &SymbolId,
    max_depth: usize,
) -> ImpactReport {
    // Reverse BFS from changed symbol
    // Classify impact by edge type:
    //   Calls edge → callers may break
    //   Imports edge → importers may need update
    //   Implements edge → trait contract may be violated
    //   Contains edge → parent scope affected
    // Return structured report with severity levels
}
```

This replaces the manual process of `grep -rn "function_name"` with structural understanding. The agent knows not just that 47 files mention the symbol, but that 12 directly call it, 8 import it, and 3 implement it as a trait method.

### Subgraph extraction

For context assembly, the Composer needs to extract relevant subgraphs:

```rust
// Planned: Extract subgraph around a set of focal symbols
pub fn extract_subgraph(
    graph: &SymbolGraph,
    focal: &[SymbolId],
    radius: usize,
) -> SymbolGraph {
    // BFS from each focal symbol (both forward and reverse)
    // Include all reached nodes within radius
    // Include all edges between included nodes
    // Return a new SymbolGraph containing only the subgraph
}
```

This enables the Composer to include "the neighborhood of symbols relevant to this task" rather than entire files.

---

## Academic Foundations

- **Code property graphs**: Yamaguchi, Golde, Arp, and Rieck (2014), "Modeling and Discovering Vulnerabilities with Code Property Graphs." *IEEE S&P*. The model for unifying AST, control flow, and data flow into a single queryable graph. `SymbolGraph` is a simplified version focused on dependency relationships.
- **PageRank**: Page, Brin, Motwani, and Winograd (1999), "The PageRank Citation Ranking: Bringing Order to the Web." Stanford InfoLab. The algorithm adapted for symbol importance scoring. See [04-pagerank-symbol-importance.md](./04-pagerank-symbol-importance.md).
- **Program dependence graph**: Ferrante, Ottenstein, and Warren (1987), "The Program Dependence Graph and Its Use in Optimization." *TOPLAS*. The theoretical foundation for directed dependency graphs in program analysis.
- **Call graph construction**: Grove and Chambers (2001), "A Framework for Call Graph Construction Algorithms." *TOPLAS*. Survey of call graph construction techniques, relevant to the planned `Calls` edge extraction.
- **AriGraph**: (2024). Graph-based approach to code understanding, demonstrating the effectiveness of explicit graph structures for navigating code relationships.

---

## Current Status and Gaps

### Built

- `SymbolGraph` with dual adjacency lists — functional and tested
- `EdgeKind` enum with four edge types — `Imports` is populated, others defined
- `build_graph()` with three-phase construction — creates `Imports` edges from parsed files
- Forward and reverse neighbor queries — O(1) lookup
- Transitive closure via BFS with depth limit — prevents unbounded traversal
- `pagerank()` — iterative computation with configurable damping (see [04-pagerank-symbol-importance.md](./04-pagerank-symbol-importance.md))
- Comprehensive tests: empty graph, single file, import edges, self-file exclusion, reverse neighbors, transitive deps, PageRank star/cycle

### Missing

- `Calls` edges — requires tree-sitter call site extraction
- `Implements` edges — requires tree-sitter impl/interface resolution
- `Contains` edges — requires tree-sitter scope nesting
- Edge weighting for task-aware PageRank
- Impact analysis API
- Subgraph extraction for context assembly
- Incremental graph updates (currently rebuilds from scratch)
- Graph persistence to disk (in-memory only)
- Graph visualization / DOT export

---

## Cross-References

- See [02-symbol-extraction.md](./02-symbol-extraction.md) for how symbols become graph nodes
- See [04-pagerank-symbol-importance.md](./04-pagerank-symbol-importance.md) for importance scoring over the graph
- See [06-context-assembly-from-code.md](./06-context-assembly-from-code.md) for how graph structure informs context selection
- See [08-index-db-scaling.md](./08-index-db-scaling.md) for persistent graph storage
- See topic [03-composition](../03-composition/INDEX.md) for the Composer that consumes graph data
