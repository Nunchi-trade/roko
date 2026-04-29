# Code Intelligence as a Cell Pipeline

> Depth for the code intelligence subsystem. Covers the core insight: code intelligence is a
> Pipeline Graph of Cells that transforms source code into Signals for the Compose protocol.

---

## The Problem

Coding agents operate under a fundamental constraint: context windows are finite, but codebases
are not. A 200K-token window covers at most 16% of a modest Rust workspace (~322K lines,
~500K tokens). For enterprise codebases (500K--5M lines), coverage drops below 1%. The agent
is blind to 84--99% of the code it is modifying.

Blind agents make predictable, expensive mistakes:

1. **Duplicate implementations** -- writing code that already exists because the agent never
   saw it. This is the single most common failure mode, catalogued as mistake #1 in the
   project's operational history.
2. **Broken dependencies** -- modifying a function signature without knowing that 47 call
   sites depend on it. Impact analysis requires graph traversal, not text search.
3. **Misunderstood abstractions** -- reimplementing a capability because the trait hierarchy
   is invisible. Understanding that `Scorer` is a composable protocol trait across layers
   requires structural comprehension.
4. **Token waste on irrelevant context** -- including entire files when three functions
   suffice. Empirical data (Gauthier 2024, Lee et al. 2026) shows that intelligent context
   selection reduces token consumption by 5--75x depending on scenario.

---

## The Solution: A Pipeline Graph

Code intelligence is a **Pipeline Graph** -- a linear chain of Cells where each stage
transforms its input and passes the result forward. See
[03-GRAPH.md](../../unified/03-GRAPH.md) for the Pipeline pattern definition.

Each stage in the Pipeline implements one of the 9 protocols defined in
[02-CELL.md](../../unified/02-CELL.md). The output of the final stage feeds the
Compose protocol (prompt assembly), making code structure available alongside knowledge,
task descriptions, and system prompts in the agent's context window.

```
Source files
    |
    v
[Parse Cell]       -- Connect protocol: reads files, emits Symbol Signals
    |
    v
[Graph Cell]       -- Store protocol: builds dependency graph from Symbols
    |
    v
[Score Cell]       -- Score protocol: computes PageRank importance scores
    |
    v
[Fingerprint Cell] -- Store protocol: generates HDC fingerprints for similarity
    |
    v
[Search Cell]      -- Route protocol: multi-strategy search, ranked results
    |
    v
[Assemble Cell]    -- Compose protocol: budget-constrained context assembly
    |
    v
Context block -> SystemPromptBuilder -> Agent prompt
```

### TOML Graph Definition

```toml
[graph]
id = "code-intelligence-pipeline"
kind = "pipeline"
description = "Transforms source files into token-budgeted code context for agents"

[[graph.cells]]
id = "parse"
protocol = "connect"
impl = "roko_index::ParseCell"
input = ["source_files"]
output = ["symbol_signals"]

[[graph.cells]]
id = "graph"
protocol = "store"
impl = "roko_index::GraphCell"
input = ["symbol_signals"]
output = ["dependency_graph"]

[[graph.cells]]
id = "score"
protocol = "score"
impl = "roko_index::ScoreCell"
input = ["dependency_graph"]
output = ["ranked_symbols"]

[[graph.cells]]
id = "fingerprint"
protocol = "store"
impl = "roko_index::FingerprintCell"
input = ["symbol_signals"]
output = ["hdc_index"]

[[graph.cells]]
id = "search"
protocol = "route"
impl = "roko_index::SearchCell"
input = ["ranked_symbols", "hdc_index", "query"]
output = ["search_results"]

[[graph.cells]]
id = "assemble"
protocol = "compose"
impl = "roko_index::AssembleCell"
input = ["search_results", "dependency_graph", "token_budget"]
output = ["context_block"]
```

### Each Cell Explained

**Parse Cell** (Connect protocol). Reads source files from the filesystem and emits Symbol
Signals. Each Symbol is a content-addressed Signal (BLAKE3 hash of source text) with
`kind: CodeSymbol` and properties for name, symbol kind (Function, Struct, Trait, etc.),
visibility, and line location. The Parse Cell delegates to `LanguageProvider` implementations
-- pluggable trait objects that know how to extract symbols from specific languages (Rust,
TypeScript, Go). Currently these use line-by-line heuristic parsers (~2,336 lines across 3
providers). The planned upgrade to tree-sitter (Brunsfeld 2018) would enable incremental
re-parsing (~50us for a single-line change) and call-site extraction.

**Graph Cell** (Store protocol). Consumes Symbol Signals and builds a directed dependency
graph with dual adjacency lists (forward edges for "what does X depend on?" and reverse
edges for "what depends on X?"). Currently produces `Imports` edges by matching import path
last-segments against symbol names. Tree-sitter would enable `Calls`, `Implements`, and
`Contains` edges. The Graph Cell implements `Store::put` (add node), `Store::get` (retrieve
neighbors), and `Store::query` (transitive closure via BFS).

**Score Cell** (Score protocol). Rates each Symbol Signal along a "structural importance"
dimension using the PageRank algorithm (Page et al. 1999). Iterative power method with
damping factor d=0.85, converging in ~30 iterations. For 5K symbols, this takes <1ms.
The Score Cell implements `Score::rate()` with a single dimension mapping PageRank to the
Signal's `utility` axis. See [02-symbol-graph-and-importance.md](02-symbol-graph-and-importance.md)
for the algorithm detail.

**Fingerprint Cell** (Store protocol). Generates 10,240-bit HDC fingerprints for each Symbol,
enabling structural similarity search without neural embeddings. Uses the same HDC vector
algebra (bind, bundle, Hamming distance) as the knowledge store (neuro), pheromone detection,
and immune system pattern matching. The Fingerprint Cell implements `Store::query_similar()`
with Hamming distance thresholding. See
[03-hdc-fingerprints-and-similarity.md](03-hdc-fingerprints-and-similarity.md).

**Search Cell** (Route protocol). Selects and combines search strategies based on query
intent. Five strategies: keyword (FTS5 BM25), structural (kind/visibility/PageRank filters),
HDC similarity (Hamming distance), embedding similarity (dense vector cosine, feature-gated),
and hybrid (Reciprocal Rank Fusion combining all). The Route Cell decides which strategies to
invoke via EFE-style gating: keyword query triggers strategy 1, "similar to X" triggers
strategy 3, broad task triggers strategy 5. See
[04-search-and-context-assembly.md](04-search-and-context-assembly.md).

**Assemble Cell** (Compose protocol). Constructs the token-budgeted context block that feeds
into the system prompt builder. Implements the same VCG auction mechanism used for general
prompt assembly -- code context sections bid for attention alongside knowledge, task
description, system prompt, and conversation history. The 6-step pipeline: parse query,
multi-strategy search, RRF rank, graph expand (1--2 hops), slice (minimal code fragments),
budget fit. See [04-search-and-context-assembly.md](04-search-and-context-assembly.md).

---

## The Pipeline as a Feed Specialization

The code intelligence Pipeline is a **Feed** -- a Cell specialization that combines Connect,
Trigger, and Store protocols for continuous data streams. See
[09-FEEDS.md](../../unified/09-FEEDS.md) for the Feed specialization definition.

The Feed behavior: a Trigger Cell watches the filesystem (via `notify::RecommendedWatcher`,
already built in `tui/fs_watch.rs`). When files change, the Trigger fires the Pipeline,
which incrementally re-indexes only the changed files (detected via BLAKE3 content hashing)
and keeps the Store current. This means the code intelligence data is always warm -- agents
never wait for a cold-start index build.

```toml
[[graph.cells]]
id = "file_watcher"
protocol = "trigger"
impl = "roko_index::FileWatchTrigger"
config.debounce_ms = 500
config.recursive = true
fires = "code-intelligence-pipeline"
```

---

## Economic Justification: Token Savings Feed the Compose Budget

Token savings from code intelligence directly improve the Compose protocol budget. The Compose
protocol (see [02-CELL.md](../../unified/02-CELL.md) -- Compose) assembles context under a
fixed token budget using VCG auction. Every token NOT wasted on irrelevant code is a token
available for knowledge, task description, or conversation history.

Empirical measurements from the Aider project (Gauthier 2024) and Meta-Harness (Lee et al.
2026):

| Scenario | Without intelligence | With intelligence | Token savings |
|---|---|---|---|
| Code search (find relevant function) | ~50K tokens | ~5K tokens | **10x** |
| Impact analysis (who calls this?) | ~150K tokens | ~2K tokens | **75x** |
| Similar pattern finding | ~100K tokens | ~3K tokens | **33x** |
| Modification context | ~40K tokens | ~8K tokens | **5x** |

Stacked with the system-wide cost reduction mechanisms (caching 5x, routing 3x, gating 2x),
code intelligence adds another 5--75x reduction layer. The agent's context window is dominated
by relevant code rather than noise.

---

## Current State: Built but Not Wired

The `roko-index` crate contains 4 working modules (~1,151 lines, 30 tests):

| Module | Lines | Tests | Maps to Cell |
|---|---|---|---|
| `parser` | 142 | 5 | Parse Cell |
| `symbol` | 211 | 8 | (types for Parse Cell output) |
| `graph` | 443 | 8 | Graph Cell + Score Cell |
| `hdc` | 355 | 9 | Fingerprint Cell |

Three language providers implement `LanguageProvider`:

| Crate | Lines | Languages |
|---|---|---|
| `roko-lang-rust` | 819 | Rust |
| `roko-lang-typescript` | 917 | TypeScript, JavaScript |
| `roko-lang-go` | 600 | Go |

**What is NOT wired:**

- No `CodeIndex` trait unifying backends (in-memory, SQLite, snapshot)
- No search API (modules exist as libraries with no consumer)
- No MCP server (agents cannot access code intelligence via tools)
- No `roko-compose` integration (code context not in prompt assembly)
- No SQLite persistence (in-memory only, rebuilt every session)
- No CLI commands (`roko index build/stats/search`)
- No tree-sitter (heuristic parsers only, no `Calls` edges)

---

## What This Enables

1. **Structural code understanding for all agents** -- coding agents perceive code structure,
   not raw text. Every task benefits from ranked, dependency-aware context.
2. **Token-efficient prompt assembly** -- 5--75x fewer tokens means more budget for knowledge,
   task description, and reasoning.
3. **Impact analysis before modification** -- agents know what breaks before they change it,
   reducing regressions from blind edits.
4. **Duplicate detection** -- HDC fingerprints detect similar code across the workspace,
   preventing the #1 mistake (reimplementation).
5. **Cross-language structural comparison** -- uniform symbol types across Rust, TypeScript,
   and Go enable cross-language dependency tracking.

## Feedback Loops

- **Gate pass rate as signal quality metric**: if code context improves, agents produce
  higher-quality output, gate pass rates increase. The Score Cell observes gate outcomes
  (via Bus subscription to `gate.verdict` topic) and adjusts PageRank weights: symbols
  whose inclusion correlated with gate passes get higher task-relevance weights. This is
  predict-publish-correct applied to code importance scoring.
- **Search strategy effectiveness**: the Route Cell tracks which search strategies led to
  successful task completion. Over time, it learns that keyword search works for specific
  names while hybrid search works for exploration. Thompson sampling across strategies.
- **Demurrage on stale index entries**: Symbol Signals in the Store decay via demurrage.
  Symbols that are never retrieved (never relevant to any task) fade. Symbols that are
  frequently retrieved stay warm. Self-trimming code intelligence.

## Open Questions

1. Should the `CodeIndex` trait include graph operations, or should graph traversal be a
   separate trait? The former is simpler; the latter is more composable.
2. Should tree-sitter be a hard dependency or remain feature-gated? It adds a C dependency
   to the build, but it is the only path to call-graph extraction.
3. How should code intelligence interact with the knowledge store (neuro)? Code patterns
   could be promoted to durable knowledge Signals when they prove consistently useful.
4. Should the Pipeline be a Hot Graph (tick-driven, always resident) or a cold Graph
   (triggered on demand)? Hot saves latency; cold saves resources.

## Implementation Tasks

| Task | File paths | Priority |
|---|---|---|
| Define `CodeIndex` trait | `crates/roko-index/src/lib.rs` | Tier 0 (blocks everything) |
| Implement `InMemoryIndex` | `crates/roko-index/src/lib.rs` | Tier 0 |
| Add keyword search (name matching) | `crates/roko-index/src/lib.rs` | Tier 0 |
| Wire code context into `SystemPromptBuilder` | `crates/roko-compose/src/system_prompt_builder.rs` | Tier 0 |
| Add `roko index build/stats` CLI commands | `crates/roko-cli/src/lib.rs` | Tier 0 |
| Integration test: agent uses code index | `crates/roko-cli/tests/` | Tier 0 |
| Add SQLite persistence (feature-gated) | `crates/roko-index/src/sqlite.rs` | Tier 1 |
| Add BLAKE3 incremental updates | `crates/roko-index/src/sqlite.rs` | Tier 1 |
| Add FTS5 keyword search | `crates/roko-index/src/sqlite.rs` | Tier 1 |
| Add tree-sitter for Rust | `crates/roko-lang-rust/src/lib.rs` | Tier 1 |
| Add tree-sitter for TypeScript | `crates/roko-lang-typescript/src/lib.rs` | Tier 1 |
| Add tree-sitter for Go | `crates/roko-lang-go/src/lib.rs` | Tier 1 |
| Implement MCP server (10 tools) | `crates/roko-mcp-code/src/lib.rs` | Tier 1 |
| Add rkyv snapshots | `crates/roko-index/src/` (new file) | Tier 2 |
| Add file watcher Trigger Cell | `crates/roko-index/src/` (new file) | Tier 2 |
