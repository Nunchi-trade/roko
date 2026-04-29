# Code Intelligence

> Structural understanding of source code for cognitive agents — parsing, symbol graphs, HDC fingerprints, search, and context assembly via the `roko-index` crate and `roko-lang-*` language providers.

**Part of**: [Roko PRD](../INDEX.md)
**Status**: Written
**Last generated**: 2026-04-12
**Prerequisites**: [Architecture](../00-architecture/INDEX.md), [Composition](../03-composition/INDEX.md)

---

## Abstract

Code intelligence is the subsystem that gives coding agents structural understanding of the codebases they work on. Rather than treating source files as opaque text, code intelligence extracts symbols, builds dependency graphs, computes importance scores, generates structural fingerprints, and assembles precisely-targeted context for LLM consumption.

The `roko-index` crate is the engine. It provides four core modules — `parser`, `symbol`, `graph`, and `hdc` — that together enable language-agnostic code analysis. Three language providers (`roko-lang-rust`, `roko-lang-typescript`, `roko-lang-go`) supply language-specific parsing via the `LanguageProvider` trait from `roko-core`. The design separates language knowledge from analysis logic, making both independently extensible.

Code intelligence spans Layer 2 (Scaffold) of the Roko architecture, sitting between the Framework layer (which provides LLM connections and tools) and the Harness layer (which verifies outputs). It serves the Synapse Loop's PERCEIVE and INTEGRATE steps by providing ranked, structured code data that the Composer assembles into token-budgeted context windows.

The current implementation is functional (4 modules, ~1,151 lines, 30 tests) but in-memory only. Planned enhancements include SQLite persistence, tree-sitter parsing, MCP context server, hybrid search, and zero-copy snapshots. These are documented in detail across the 11 sub-docs below.

---

## Contents

| # | Sub-doc | What it covers |
|---|---|---|
| 00 | [Vision](./00-vision.md) | Why code intelligence matters; the niche construction thesis; design principles; relationship to the Synapse Architecture |
| 01 | [Tree-Sitter Parsing](./01-tree-sitter-parsing.md) | The `LanguageProvider` trait; current heuristic parsers; planned tree-sitter migration; incremental parsing; per-language grammar |
| 02 | [Symbol Extraction](./02-symbol-extraction.md) | `Symbol`, `SymbolKind`, `SymbolId`, `SymbolRef`; extraction pipeline; cross-language mapping; planned enrichments |
| 03 | [Dependency Graph](./03-dependency-graph.md) | `SymbolGraph` data structure; `EdgeKind` taxonomy; `build_graph()` algorithm; forward/reverse traversal; BFS transitive closure |
| 04 | [PageRank Symbol Importance](./04-pagerank-symbol-importance.md) | PageRank algorithm adapted for code; convergence properties; weighted/personalized variants; budget-aware context allocation |
| 05 | [HDC Fingerprints](./05-hdc-fingerprints.md) | 10,240-bit hyperdimensional vectors; bind/bundle/hamming operations; symbol and file fingerprinting; performance characteristics |
| 06 | [Context Assembly from Code](./06-context-assembly-from-code.md) | Five search strategies; Reciprocal Rank Fusion; context assembly pipeline; token savings; integration with `roko-compose` |
| 07 | [MCP Context Server](./07-mcp-context-server.md) | Ten MCP tools for agent-facing code intelligence; server architecture; integration with `roko.toml` MCP config |
| 08 | [Index.db Scaling](./08-index-db-scaling.md) | SQLite schema; BLAKE3 incremental updates; FTS5 search; feature-flag architecture; scaling characteristics |
| 09 | [Snapshot Optimization](./09-snapshot-optimization.md) | rkyv zero-copy snapshots; memory-mapped files; differential updates; Salsa memoization; cold start elimination |
| 10 | [Current Status and Gaps](./10-current-status-and-gaps.md) | Detailed inventory; tiered gap analysis; 4-phase implementation roadmap; quality metrics; risk assessment |

---

## Prerequisites

Before reading this topic, we recommend:

- [Topic 00: Architecture](../00-architecture/INDEX.md) — for the Synapse Architecture (Engrams, 6 traits, cognitive loop), the Five Layers, and the `LanguageProvider` trait origin
- [Topic 03: Composition](../03-composition/INDEX.md) — for context engineering and prompt assembly, which consumes code intelligence output
- [Topic 02: Agents](../02-agents/INDEX.md) — for agent types (especially coding agents) that use code intelligence

---

## Cross-References

This topic connects to:

- [Topic 00: Architecture](../00-architecture/INDEX.md) — Core types (`Symbol`, `SymbolKind`, `Visibility`, `Import`) are defined in `roko-core`
- [Topic 02: Agents](../02-agents/INDEX.md) — Coding agents are the primary consumers of code intelligence
- [Topic 03: Composition](../03-composition/INDEX.md) — `roko-compose` assembles code context into prompts via `SystemPromptBuilder`
- [Topic 04: Verification](../04-verification/INDEX.md) — Gates verify agent output; code intelligence could enable structural verification
- [Topic 05: Learning](../05-learning/INDEX.md) — Code intelligence data (PageRank, fingerprints) feeds learning loops
- [Topic 06: Neuro](../06-neuro/INDEX.md) — Neuro knowledge store uses HDC encoding similar to `roko-index`
- [Topic 18: Tools](../18-tools/INDEX.md) — MCP tools expose code intelligence to agents

---

## Key Academic Foundations

- Brunsfeld (2018) — Tree-sitter incremental parsing framework
- Kanerva (2009) — "Hyperdimensional Computing: An Introduction to Computing in Distributed Representation" *Cognitive Computation*
- Page, Brin, Motwani, and Winograd (1999) — "The PageRank Citation Ranking: Bringing Order to the Web" Stanford InfoLab
- Weiser (1981) — "Program Slicing" *ICSE*
- Yamaguchi, Golde, Arp, and Rieck (2014) — "Modeling and Discovering Vulnerabilities with Code Property Graphs" *IEEE S&P*
- Alon, Zilberstein, Levy, and Brody (2019) — "code2vec: Learning Distributed Representations of Code" *POPL*
- Feng, Guo, Tang et al. (2020) — "CodeBERT: A Pre-Trained Model for Programming and Natural Languages" *EMNLP*
- Odling-Smee, Laland, and Feldman (2003) — *Niche Construction: The Neglected Process in Evolution* Princeton University Press
- Nielson, Nielson, and Hankin (1999) — *Principles of Program Analysis*
- Lee et al. (2026) — "Meta-Harness: Automated Scaffolding Optimization for LLM Agents" arXiv:2603.28052
- Gauthier (2024) — Aider repository map and coding agent context optimization
- Li et al. (2023) — "StarCoder: May the Source Be with You!" arXiv:2305.06161
- Cormack, Clarke, and Butt (2009) — "Reciprocal Rank Fusion Outperforms Condorcet and Individual Rank Learning Methods" *SIGIR*

---

## Current Status and Implementation Gaps

### Built

The `roko-index` crate provides four working modules with 30 tests:

| Module | Lines | Tests | Key types / functions |
|---|---|---|---|
| `parser` | 142 | 5 | `SourceFile`, `parse_source()` |
| `symbol` | 211 | 8 | `SymbolId`, `SymbolRef`, `find_symbol()` |
| `graph` | 443 | 8 | `SymbolGraph`, `build_graph()`, `pagerank()` |
| `hdc` | 355 | 9 | `HdcFingerprint`, `fingerprint_symbol()`, `similarity()` |

Three language providers implement the `LanguageProvider` trait:
- `roko-lang-rust` (819 lines) — Rust heuristic parser + `CargoBuildSystem`
- `roko-lang-typescript` (917 lines) — TypeScript/JavaScript heuristic parser + 3 build systems
- `roko-lang-go` (600 lines) — Go heuristic parser + `GoBuildSystem`

### Major gaps

1. **No persistent storage** — In-memory only; rebuilt from scratch each session
2. **No search API** — Modules exist but have no consumer or query interface
3. **No MCP server** — Agents cannot access code intelligence via tools
4. **No roko-compose integration** — Code context not wired into prompt assembly
5. **Heuristic parsers only** — No tree-sitter for accurate AST parsing
6. **Import edges only** — No `Calls`, `Implements`, or `Contains` edges in graph

### Implementation roadmap

- **Phase 1**: CodeIndex trait + search API + compose integration + CLI commands
- **Phase 2**: SQLite persistence + BLAKE3 incremental updates + FTS5 search
- **Phase 3**: MCP context server with 10 tools
- **Phase 4**: Tree-sitter integration + call graph + scope nesting

See [10-current-status-and-gaps.md](./10-current-status-and-gaps.md) for the full roadmap with effort estimates.

---

## Generation Notes

- **Generated**: 2026-04-12
- **Model**: Claude Opus 4.6
- **Sub-docs produced**: 11
- **Total lines**: ~3,400
- **Primary sources consulted**:
  - `crates/roko-index/src/` (lib.rs, parser.rs, symbol.rs, graph.rs, hdc.rs)
  - `crates/roko-lang-rust/src/lib.rs`
  - `crates/roko-lang-typescript/src/lib.rs`
  - `crates/roko-lang-go/src/lib.rs`
  - `bardo-backup/tmp/mori-refactor/10-code-intelligence.md`
  - `bardo-backup/tmp/mori-agents/18-code-intelligence-and-gateway.md`
  - `bardo-backup/tmp/death/tools/02-code-index.md`
  - `bardo-backup/tmp/death/docs/30-index-performance.md`
  - `refactoring-prd/05-agent-types.md`
  - `refactoring-prd/00-overview.md`
  - `refactoring-prd/08-translation-guide.md`
- **Decisions requiring judgment**:
  - Organized HDC fingerprints as a dedicated sub-doc rather than folding into the vision doc, because the implementation is substantial (355 lines, 9 tests) and the mathematical foundations warrant standalone treatment
  - Separated PageRank from dependency graph because PageRank has its own academic lineage and planned weighted/personalized variants
  - Split context assembly and MCP server into separate docs because assembly is about algorithms while MCP is about interface design
  - Included detailed Rust code from the actual codebase rather than pseudocode, to accurately represent what is built
- **Open questions**:
  - Whether the `CodeIndex` trait should include graph operations or keep them separate
  - Whether tree-sitter should be a hard dependency or remain feature-gated
  - Whether dense embeddings (fastembed) are worth the ~100MB model size for initial deployment
  - Whether the MCP context server should be a separate binary or embedded in `roko-cli`
