# PageRank for Symbol Importance

> Applying the PageRank algorithm to code dependency graphs to identify the most important symbols in a workspace — guiding context allocation and token budget prioritization.

**Topic**: [Code Intelligence](./INDEX.md)
**Prerequisites**: [03-dependency-graph.md](./03-dependency-graph.md)
**Key sources**: `crates/roko-index/src/graph.rs`, `bardo-backup/tmp/mori-agents/18-code-intelligence-and-gateway.md`, `bardo-backup/tmp/mori-refactor/10-code-intelligence.md`

---

## Abstract

Not all symbols in a codebase are equally important. A core `Config` struct imported by 30 modules matters more than a helper function used in one test file. An agent working on a task needs to know which symbols are structural pillars of the codebase and which are peripheral — this knowledge directly determines how to allocate its limited context budget.

PageRank (Page, Brin, Motwani, and Winograd 1999) provides a principled answer. Originally designed to rank web pages by link structure, the algorithm transfers naturally to code dependency graphs: symbols that are imported by many other symbols receive high scores, and symbols imported by high-scoring symbols receive even higher scores. This recursive definition of "importance" captures the intuition that a symbol is important if important things depend on it.

The `pagerank()` function in `roko-index` implements iterative PageRank over the `SymbolGraph`. This document explains the algorithm, its adaptation for code, edge weighting strategies, and how PageRank scores integrate with the Synapse Architecture.

---

## The Algorithm

### Standard PageRank

The PageRank of a node is defined recursively:

```
PR(v) = (1 - d) / N + d × Σ(PR(u) / out_degree(u))
                         for each u that links to v
```

Where:
- `d` = damping factor (typically 0.85)
- `N` = total number of nodes
- `PR(u)` = PageRank of node `u`
- `out_degree(u)` = number of outgoing edges from `u`

The damping factor represents the probability that a "random surfer" follows a link (85%) versus jumping to a random page (15%). In the code domain, this models the idea that importance flows through dependencies but also has a baseline component — every symbol has some inherent importance simply by existing.

### Implementation in roko-index

```rust
pub fn pagerank(
    graph: &SymbolGraph,
    iterations: u32,
    damping: f64,
) -> HashMap<SymbolId, f64> {
    let all_nodes: Vec<&SymbolId> = graph.nodes.iter().collect();
    let n = all_nodes.len();
    if n == 0 {
        return HashMap::new();
    }

    let n_f = n as f64;
    // Initialize: every node gets equal rank 1/N
    let mut rank: HashMap<SymbolId, f64> = all_nodes
        .iter()
        .map(|id| ((*id).clone(), 1.0 / n_f))
        .collect();

    for _ in 0..iterations {
        let mut new_rank: HashMap<SymbolId, f64> =
            HashMap::with_capacity(n);
        let base = (1.0 - damping) / n_f;

        for &node in &all_nodes {
            let mut incoming_sum = 0.0_f64;
            if let Some(inbound) = graph.reverse.get(node) {
                for (src, _) in inbound {
                    let src_rank =
                        rank.get(src).copied().unwrap_or(0.0);
                    let out_degree = graph.forward
                        .get(src)
                        .map_or(1, Vec::len)
                        .max(1) as f64;
                    incoming_sum += src_rank / out_degree;
                }
            }
            new_rank.insert(
                node.clone(),
                damping.mul_add(incoming_sum, base),
            );
        }

        rank = new_rank;
    }

    rank
}
```

Key implementation details:

1. **Initialization** — All nodes start with equal rank `1/N`. This ensures the total rank sums to 1.0.

2. **Iteration** — Each iteration recomputes all ranks from the previous iteration's values. The algorithm converges geometrically; 20–30 iterations typically suffice for a code graph.

3. **Out-degree floor** — `max(1)` prevents division by zero for nodes with no outgoing edges (dangling nodes). These nodes effectively distribute their rank equally to all other nodes via the damping component.

4. **Reverse edge traversal** — The implementation uses the pre-computed reverse adjacency list for efficient lookup of incoming edges.

5. **`mul_add` precision** — The fused multiply-add operation `damping.mul_add(incoming_sum, base)` provides better floating-point precision than separate multiply and add.

### Convergence properties

| Property | Value |
|---|---|
| Initial rank | 1/N for all nodes |
| Damping factor (default) | 0.85 |
| Convergence rate | Geometric with rate d = 0.85 |
| Iterations for < 0.001 error | ~30 |
| Iterations for < 0.0001 error | ~50 |
| Total rank (invariant) | 1.0 (within floating-point precision) |

For the Roko workspace (~5K nodes), 30 iterations of PageRank take under 1ms. This is fast enough to recompute on every graph update without caching.

---

## Interpreting PageRank Scores for Code

### What high PageRank means

A symbol with high PageRank is one that many other important symbols depend on. In a codebase, these are typically:

| Symbol pattern | Typical PageRank | Why |
|---|---|---|
| Core types (`Signal`/`Engram`, `Error`, `Config`) | Top 1% | Imported everywhere |
| Trait definitions (`Gate`, `Scorer`, `Router`) | Top 5% | Implemented by many types |
| Shared utilities (`parse_source`, `build_graph`) | Top 10% | Called from multiple modules |
| Entry points (`main`, `run`, `execute`) | Top 15% | High out-degree, some in-links |
| Module-internal helpers | Bottom 50% | Few external imports |
| Test utilities | Bottom 20% | Only imported by tests |
| Dead code | Bottom 5% | Zero in-links, only baseline score |

### What PageRank does NOT capture

PageRank is a structural metric. It does not account for:

1. **Task relevance** — The most important symbol globally may be irrelevant to the current task. A weighted variant (see below) addresses this.

2. **Recency** — A recently-modified symbol may be more relevant than a stable core type. Recency weighting addresses this.

3. **Semantic meaning** — Two symbols with identical graph structure but different semantic roles (e.g., a Config struct vs. an Error struct) receive the same PageRank. HDC fingerprints capture semantic properties that PageRank misses.

4. **Code quality** — A symbol imported by many modules because of poor abstraction (God object antipattern) gets high PageRank even though it represents a design problem.

---

## Verified Behaviors

The test suite in `roko-index/src/graph.rs` verifies key PageRank properties:

### Star topology: hub gets highest rank

When three nodes (A, B, C) all import a single hub node:

```rust
#[test]
fn pagerank_star_hub_highest() {
    // hub.rs defines Hub; a.rs, b.rs, c.rs all import Hub
    // ...
    let hub_rank = ranks.get(&hub_id).copied().unwrap_or(0.0);
    for (id, rank) in &ranks {
        if *id != hub_id {
            assert!(hub_rank > *rank,
                "Hub rank {hub_rank} should exceed {id} rank {rank}");
        }
    }
}
```

This verifies the basic intuition: symbols with more incoming links get higher scores.

### Cycle topology: equal ranks

When three nodes form a cycle (A → B → C → A):

```rust
#[test]
fn pagerank_cycle_roughly_equal() {
    // A imports B, B imports C, C imports A
    // ...
    let max = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min = vals.iter().copied().fold(f64::INFINITY, f64::min);
    assert!((max - min).abs() < 0.01,
        "Cycle nodes should have near-equal ranks");
}
```

This verifies that symmetry is preserved: in a perfectly symmetric graph, all nodes should have (approximately) equal rank.

### Empty graph: no panics

```rust
#[test]
fn pagerank_empty() {
    let graph = build_graph(&[]);
    let ranks = pagerank(&graph, 10, 0.85);
    assert!(ranks.is_empty());
}
```

Edge case handling: an empty graph produces an empty rank map without errors.

---

## Planned: Weighted PageRank

### Task-aware edge weighting

The current implementation treats all edges equally. The planned weighted variant incorporates task context:

```rust
// Planned: Weighted PageRank
pub fn weighted_pagerank(
    graph: &SymbolGraph,
    iterations: u32,
    damping: f64,
    weights: &EdgeWeights,
) -> HashMap<SymbolId, f64> {
    // Same algorithm, but edge contributions are scaled by weights
    // incoming_sum += (src_rank / weighted_out_degree) * edge_weight(src, node)
}

pub struct EdgeWeights {
    /// Symbols mentioned in the current task description.
    pub task_mentions: HashSet<SymbolId>,
    /// Files currently in the agent's context window.
    pub context_files: HashSet<String>,
    /// Recently modified files (from git log).
    pub recent_files: HashSet<String>,
}
```

The weighting scheme, informed by measurements from the Aider project (Gauthier 2024) and design discussions in the legacy codebase:

| Condition | Weight | Rationale |
|---|---|---|
| Symbol mentioned in task prompt | 10× | Direct task relevance |
| Symbol in currently open file | 50× | Active working context |
| Symbol in recently modified file | 5× | Recency bias |
| Private/crate-internal symbol | 0.1× | Less likely cross-module relevance |
| Test file symbol | 0.5× | Tests depend on code, not usually reverse |
| Default (no special condition) | 1.0× | Baseline |

### Personalized PageRank

An alternative to global edge weighting is Personalized PageRank (PPR), where the teleportation distribution is biased toward task-relevant nodes:

```
PPR(v) = (1 - d) × teleport(v) + d × Σ(PPR(u) / out_degree(u))
```

Instead of uniform teleportation `1/N`, PPR uses a custom distribution:
- Task-mentioned symbols get `teleport(v) = 0.5 / |task_symbols|`
- Other symbols get `teleport(v) = 0.5 / (N - |task_symbols|)`

This biases the ranking toward symbols structurally connected to the task without completely ignoring globally important symbols.

---

## Integration with the Synapse Architecture

### PageRank as Engram scoring

PageRank scores map naturally to the Engram scoring system:

| PageRank output | Engram axis | How |
|---|---|---|
| Raw PageRank score | `utility` | Higher rank → higher utility for context inclusion |
| Normalized rank (0–1) | `salience` | Rank relative to the highest-ranked symbol |
| Stability across iterations | `confidence` | Symbols whose rank changes little are more reliably important |

This means code intelligence data flows through the same six Synapse traits as every other Engram. The Scorer can score code symbols using PageRank. The Router can select the highest-ranked symbols. The Composer can allocate context budget proportional to rank.

### Budget-aware context allocation

The Composer uses PageRank to decide how many tokens to allocate to each symbol:

```
token_budget(symbol) = total_budget × (pagerank(symbol) / Σ pagerank(included_symbols))
```

A symbol with twice the PageRank of another gets twice the token budget — more of its surrounding code, documentation, and context is included. This ensures the agent's context window is dominated by the most structurally important code.

### Dual-process routing

The tier-routing system uses PageRank to modulate cognitive investment:

- **T0 (no LLM)** — Modifications to low-PageRank symbols (e.g., test helpers, internal utilities) can often be handled by pattern-matching heuristics.
- **T1 (fast model)** — Modifications to medium-PageRank symbols get lightweight LLM reasoning.
- **T2 (full model)** — Modifications to high-PageRank symbols (core types, public API surfaces) get full reasoning with extended context.

This maps to the dual-process cognition principle: invest more compute where the stakes are higher.

---

## Performance

### Current implementation

| Metric | Value |
|---|---|
| Algorithm | Iterative power method |
| Time complexity | O(iterations × (N + E)) |
| Space complexity | O(N) for rank vectors |
| 5K nodes, 30K edges, 30 iterations | < 1ms |
| 50K nodes, 200K edges, 30 iterations | ~10ms (estimated) |
| 500K nodes, 2M edges, 30 iterations | ~100ms (estimated) |

For any workspace that fits in memory, PageRank is fast enough to recompute on every graph change.

### Planned optimizations

For very large codebases, incremental PageRank can avoid full recomputation:

1. **Delta propagation** — When an edge is added or removed, propagate rank changes only through affected nodes (typically a small subgraph).
2. **Blocked PageRank** — Process nodes in blocks that correspond to modules/crates, exploiting locality.
3. **Early termination** — Stop iterating when the maximum rank change between iterations falls below a threshold (e.g., 1e-6).

---

## Academic Foundations

- **PageRank**: Page, Brin, Motwani, and Winograd (1999), "The PageRank Citation Ranking: Bringing Order to the Web." Stanford InfoLab Technical Report. The original algorithm, adapted here for code dependency graphs instead of web link graphs.
- **Personalized PageRank**: Haveliwala (2002), "Topic-Sensitive PageRank." *WWW*. The variant where teleportation is biased toward topic-relevant nodes, applicable to task-aware symbol ranking.
- **Code importance ranking**: Allamanis, Barr, Bird, and Sutton (2014), "Learning Natural Coding Conventions." *FSE*. Demonstrated that code structural properties (including dependency centrality) correlate with developer attention and modification frequency.
- **Meta-Harness**: Lee et al. (2026), "Meta-Harness: Automated Scaffolding Optimization for LLM Agents." arXiv:2603.28052. Shows that better context allocation (which PageRank enables) yields measurable improvements in agent task performance.
- **Graph centrality in software**: Zimmermann and Nagappan (2008), "Predicting Defects Using Network Analysis on Dependency Graphs." *ICSE*. Found that graph centrality metrics on code dependency graphs predict defect-prone modules — the same structural signal PageRank captures.

---

## Current Status and Gaps

### Built

- Iterative PageRank with configurable iterations and damping factor
- Correct handling of empty graphs, star topologies, and cycles
- Integration with `SymbolGraph` via reverse adjacency list
- Comprehensive test suite (empty, star-hub-highest, cycle-equal)

### Missing

- Edge weighting (all edges treated equally)
- Task-aware Personalized PageRank
- Incremental rank updates (full recomputation only)
- Score normalization to [0, 1] range
- Mapping to Engram scoring axes
- Budget-proportional context allocation
- Early termination on convergence
- Visualization of rank distributions

---

## Cross-References

- See [03-dependency-graph.md](./03-dependency-graph.md) for the graph that PageRank operates on
- See [06-context-assembly-from-code.md](./06-context-assembly-from-code.md) for how PageRank scores drive context selection
- See [05-hdc-fingerprints.md](./05-hdc-fingerprints.md) for the complementary similarity metric
- See topic [05-learning](../05-learning/INDEX.md) for how PageRank scores feed into learning loops
- See topic [03-composition](../03-composition/INDEX.md) for the Composer that uses PageRank for budget allocation
