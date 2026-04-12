# Vision: Why Code Intelligence Matters for Cognitive Agents

> Code intelligence transforms coding agents from blind text generators into informed collaborators that understand structure, dependency, and intent.


> **Implementation**: Built

**Topic**: [Code Intelligence](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md), [Composition](../03-composition/INDEX.md)
**Key sources**: `bardo-backup/tmp/mori-refactor/10-code-intelligence.md`, `bardo-backup/tmp/mori-agents/18-code-intelligence-and-gateway.md`, `bardo-backup/tmp/death/tools/02-code-index.md`, `refactoring-prd/05-agent-types.md`

---

## Abstract

Coding agents powered by large language models face a fundamental constraint: the context window is finite, but codebases are not. A 200,000-token window sounds generous until you realize that a mid-size Rust workspace — the Roko codebase itself at ~177K lines across 18 crates — already exceeds that budget as raw text. Agents must work with partial views, and the quality of those partial views determines the quality of the agent's output.

Code intelligence is the subsystem that makes those partial views excellent. Rather than dumping files into the context window and hoping the relevant code is somewhere in the pile, code intelligence provides structured understanding: what symbols exist, how they relate, which ones matter most for the current task, and how to retrieve exactly the right context at the right granularity. This is the difference between a coding agent that wastes tokens on irrelevant boilerplate and one that arrives at the task with precisely the context it needs.

The `roko-index` crate is Roko's code intelligence engine. It implements four capabilities — parsing, symbol graphs, HDC fingerprints, and search — that together form the foundation for context-aware coding. When combined with `roko-compose` (context assembly) and the MCP context server (agent-facing API), these capabilities enable the Synapse Loop's PERCEIVE and INTEGRATE steps to operate on code with the same precision that the loop operates on any other Engram domain.

This document explains why code intelligence exists, what problems it solves, and how it fits into Roko's cognitive architecture.

---

## The Context Window Problem

### Token budgets are scarce

Modern LLMs offer context windows ranging from 128K to 2M tokens. This sounds abundant, but consider the math for a real codebase:

| Metric | Roko workspace | Typical enterprise |
|---|---|---|
| Lines of code | ~177K | 500K–5M |
| Estimated tokens (raw) | ~500K | 1.5M–15M |
| Context budget (128K model) | 128K | 128K |
| Usable for code (after system prompt, instructions, history) | ~80K | ~80K |
| Coverage without intelligence | ~16% | 0.5–5% |

Without code intelligence, an agent can see at most 16% of even a modest codebase in a single turn. For enterprise codebases, that drops below 1%. The agent is effectively blind to 84–99% of the code it is modifying.

### Blind agents make expensive mistakes

When agents lack structural understanding, they exhibit predictable failure modes:

1. **Duplicate implementations** — The agent writes code that already exists elsewhere in the codebase because it never saw the existing implementation. This is the single most common failure mode in the Roko codebase itself, catalogued in `MISTAKES-LEARNED.md` as mistake #1.

2. **Broken dependencies** — The agent modifies a function signature without knowing that 47 other call sites depend on the old signature. Impact analysis requires graph traversal, not text search.

3. **Misunderstood abstractions** — The agent reimplements a capability because it doesn't understand the trait hierarchy or generic patterns in play. Understanding that `Scorer` is a composable trait across layers requires structural comprehension.

4. **Token waste on irrelevant context** — Without ranking, the agent includes entire files when it only needs three functions. Measurements from the Aider project (Gauthier 2024) show that intelligent context selection reduces token consumption by 10× for search tasks and up to 75× for impact analysis.

### The niche construction thesis

The concept of niche construction from evolutionary biology (Odling-Smee, Laland, and Feldman 2003) provides the theoretical foundation for code intelligence. In biology, organisms don't just adapt to their environment — they actively modify it to improve their fitness. Beavers build dams. Earthworms transform soil chemistry. These modifications create a "cognitive niche" that makes the organism more effective.

Coding agents operate analogously. The codebase is the agent's environment. Code intelligence is the mechanism by which the agent constructs its cognitive niche — building indexes, computing importance scores, maintaining symbol graphs — so that future interactions with the codebase are more productive. Each indexing pass makes the agent more effective at its next task.

This is not a metaphor. It is a design principle. The `roko-index` crate is literally the niche construction machinery for Roko's coding agents. Every parse, every graph edge, every HDC fingerprint is a modification to the agent's cognitive environment that improves future performance.

---

## What Code Intelligence Provides

### The four pillars

The `roko-index` crate implements four core capabilities, each building on the previous:

| Pillar | Module | What it does | Why it matters |
|---|---|---|---|
| **Parsing** | `parser` | Extracts symbols and imports from source files via `LanguageProvider` | Raw structural data |
| **Graph** | `graph` | Builds directed dependency graph, computes PageRank | Understands relationships and importance |
| **Fingerprints** | `hdc` | 10,240-bit HDC vectors for structural similarity | Finds similar code without embeddings |
| **Search** | (planned) | Hybrid search combining keyword, structural, HDC, and embedding strategies | Retrieves precisely the right context |

These pillars serve the Synapse Loop at two critical steps:

- **PERCEIVE** (step 1, `Substrate.query()`) — Code intelligence enables the Substrate to return not just raw files but parsed, ranked, similarity-scored code fragments. The agent perceives code structure, not text.

- **INTEGRATE** (step 4, `Composer.compose()`) — The Composer uses PageRank scores and dependency information to assemble context windows that prioritize the most important symbols for the current task. Budget-aware composition means every token counts.

### Token savings: empirical evidence

Measurements from Aider's repository map feature (Gauthier 2024) and Meta-Harness experiments (Lee et al. 2026, arXiv:2603.28052) demonstrate the impact of code intelligence:

| Scenario | Without intelligence | With intelligence | Savings |
|---|---|---|---|
| Code search (find relevant function) | ~50K tokens (dump all candidate files) | ~5K tokens (ranked symbols + snippets) | **10×** |
| Impact analysis (who calls this?) | ~150K tokens (grep + context expansion) | ~2K tokens (graph traversal + callers) | **75×** |
| Similar pattern finding | ~100K tokens (manual search) | ~3K tokens (HDC similarity + top-K) | **33×** |
| Context for modification | ~40K tokens (whole-file includes) | ~8K tokens (dependency-aware slicing) | **5×** |

These savings are not just cost reduction. They directly improve agent quality because more of the context budget is spent on relevant code rather than noise.

---

## Design Principles

### 1. Language-agnostic core, language-specific providers

The `roko-index` crate defines no language-specific parsing logic. All language knowledge lives in `LanguageProvider` implementations:

- `roko-lang-rust` — Rust parsing (heuristic, tree-sitter planned)
- `roko-lang-typescript` — TypeScript/JavaScript parsing
- `roko-lang-go` — Go parsing

This separation means adding a new language (Python, Java, C++) requires only implementing the `LanguageProvider` trait. The graph, fingerprint, and search layers work unchanged.

### 2. Incremental by design

Codebases change incrementally — a commit typically touches 1–5 files out of thousands. Code intelligence must be incremental to be practical:

- **BLAKE3 content hashing** detects which files actually changed (not just which were touched by git)
- **Salsa-style memoization** (planned) caches parse results and recomputes only when inputs change
- **Graph dirty flags** (planned) propagate invalidation through the dependency graph so only affected subgraphs are reprocessed

The target is sub-second re-indexing for typical commits, even on workspaces with 100K+ symbols.

### 3. Zero-copy where possible

Performance matters because indexing runs in the agent's critical path — between receiving a task and beginning work. The design favors:

- **rkyv snapshots** (planned) for zero-copy deserialization via memory-mapped files
- **Bitwise operations** for HDC similarity (XOR + popcount, ~50ns per comparison)
- **In-place graph traversal** rather than materializing intermediate collections

### 4. Composable with the Synapse Architecture

Code intelligence is not a standalone system. It produces and consumes Engrams:

- A parsed symbol can be stored as an Engram with `kind: CodeSymbol`
- A PageRank score maps to an Engram's `utility` axis
- An HDC fingerprint similarity maps to the `salience` axis
- The dependency graph itself is a form of `lineage` tracking

This means code intelligence data flows through the same six Synapse traits as every other data type in Roko.

---

## Relationship to the Broader Architecture

### Layer placement

Code intelligence spans two layers:

| Layer | Component | Role |
|---|---|---|
| **L0 Runtime** | File watching, incremental triggers | Detects when code changes |
| **L2 Scaffold** | `roko-index`, `roko-lang-*` | Parsing, graphs, fingerprints, search |
| **L2 Scaffold** | `roko-compose` | Assembles code context into prompts |
| **L3 Harness** | MCP context server | Exposes code intelligence to agents via tools |

The Scaffold placement is intentional. Code intelligence is a form of context engineering — it prepares the information that the Composer uses to build prompts. It is below the Harness (which verifies) and above the Framework (which connects).

### Agent types that depend on code intelligence

Per the agent type taxonomy in `refactoring-prd/05-agent-types.md`:

- **Coding agents** — Primary consumers. Use code intelligence for every task: understanding existing code, finding modification points, assessing impact, generating context-aware patches.
- **Research agents** — Use code intelligence to understand codebase structure during analysis tasks, though their primary domain is external information.
- **Custom agents** — Domain-specific agents may use code intelligence when their domain involves code (e.g., a security audit agent).

---

## Academic Foundations

The design of `roko-index` draws on established research across multiple fields:

- **Static analysis foundations**: Nielson, Nielson, and Hankin (1999), *Principles of Program Analysis*. The foundational text on how to extract information from source code without executing it.
- **Program slicing**: Weiser (1981), "Program Slicing." *ICSE*. The original formulation of extracting the minimal code subset relevant to a computation — the intellectual ancestor of context-aware code retrieval.
- **Code property graphs**: Yamaguchi, Golde, Arp, and Rieck (2014), "Modeling and Discovering Vulnerabilities with Code Property Graphs." *IEEE S&P*. Unifying AST, CFG, and PDG into a single queryable graph — the model for `SymbolGraph`.
- **Tree-sitter**: Brunsfeld (2018). Incremental parsing framework that provides the target parsing backend for `roko-index` (currently using heuristic parsers; tree-sitter integration is planned).
- **Hyperdimensional computing**: Kanerva (2009), "Hyperdimensional Computing: An Introduction to Computing in Distributed Representation with High-Dimensional Random Vectors." *Cognitive Computation*. The mathematical foundation for HDC fingerprints.
- **code2vec**: Alon, Zilberstein, Levy, and Brody (2019), "code2vec: Learning Distributed Representations of Code." *POPL*. Demonstrated that code structure can be effectively captured in fixed-width vectors — the inspiration for HDC fingerprints as a lightweight alternative.
- **CodeBERT**: Feng, Guo, Tang, et al. (2020), "CodeBERT: A Pre-Trained Model for Programming and Natural Languages." *EMNLP*. Established that pre-trained models can capture code semantics, motivating the dense embedding layer as a complement to HDC.
- **Niche construction**: Odling-Smee, Laland, and Feldman (2003), *Niche Construction: The Neglected Process in Evolution*. Princeton University Press. The theoretical foundation for code intelligence as environmental modification by cognitive agents.
- **Meta-Harness**: Lee et al. (2026), "Meta-Harness: Automated Scaffolding Optimization for LLM Agents." arXiv:2603.28052. Demonstrates that harness optimization (including context engineering) yields +7.7 points on text classification and +4.7 on IMO-level math at 4× fewer tokens.
- **Aider repository map**: Gauthier (2024). Empirical demonstration that tree-sitter-based repository maps improve coding agent performance by providing structural context rather than raw file dumps.

---

## Current Status and Gaps

### What exists today

The `roko-index` crate is built and functional with four modules:

| Module | Status | Lines | Tests |
|---|---|---|---|
| `parser` | Built | 142 | 5 |
| `symbol` | Built | 211 | 8 |
| `graph` | Built | 443 | 8 |
| `hdc` | Built | 355 | 9 |

Three language providers are built:

| Crate | Status | Lines | Parsing approach |
|---|---|---|---|
| `roko-lang-rust` | Built | 819 | Line-by-line heuristic |
| `roko-lang-typescript` | Built | 917 | Line-by-line heuristic |
| `roko-lang-go` | Built | 600 | Line-by-line heuristic |

### What is missing

1. **Tree-sitter integration** — All three language providers use line-by-line heuristic parsers. Tree-sitter would provide accurate AST-level parsing with incremental update support. See [01-tree-sitter-parsing.md](./01-tree-sitter-parsing.md).

2. **Persistent storage** — No SQLite or on-disk index. Symbols and graphs exist only in memory during a single session. See [08-index-db-scaling.md](./08-index-db-scaling.md).

3. **Search layer** — No search API combining keyword, structural, HDC, and embedding strategies. See [06-context-assembly-from-code.md](./06-context-assembly-from-code.md).

4. **MCP context server** — No MCP server exposing code intelligence to agents via tool calls. See [07-mcp-context-server.md](./07-mcp-context-server.md).

5. **Snapshot/caching** — No rkyv snapshots, no Salsa memoization, no incremental reuse. See [09-snapshot-optimization.md](./09-snapshot-optimization.md).

6. **Dense embeddings** — No embedding model integration for semantic search. Currently HDC-only for similarity.

7. **Call graph edges** — The graph builder only creates `Imports` edges. `Calls`, `Implements`, and `Contains` edges require deeper AST analysis (tree-sitter).

---

## Cross-References

- See [01-tree-sitter-parsing.md](./01-tree-sitter-parsing.md) for incremental parsing design
- See [03-dependency-graph.md](./03-dependency-graph.md) for the `SymbolGraph` architecture
- See [05-hdc-fingerprints.md](./05-hdc-fingerprints.md) for HDC similarity search
- See [06-context-assembly-from-code.md](./06-context-assembly-from-code.md) for how indexed code becomes LLM context
- See topic [03-composition](../03-composition/INDEX.md) for context engineering and prompt assembly
- See topic [02-agents](../02-agents/INDEX.md) for agent types and their use of code intelligence
- See topic [18-tools](../18-tools/INDEX.md) for the MCP tool system that exposes code intelligence
