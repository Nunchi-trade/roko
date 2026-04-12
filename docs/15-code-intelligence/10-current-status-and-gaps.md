# Current Status and Implementation Gaps

> A comprehensive assessment of what exists, what's missing, and the implementation roadmap for completing code intelligence in Roko.


> **Implementation**: Built

**Topic**: [Code Intelligence](./INDEX.md)
**Prerequisites**: All previous sub-docs
**Key sources**: `crates/roko-index/`, `crates/roko-lang-rust/`, `crates/roko-lang-typescript/`, `crates/roko-lang-go/`, `bardo-backup/tmp/mori-refactor/10-code-intelligence.md`, `bardo-backup/tmp/mori-agents/18-code-intelligence-and-gateway.md`

---

## Abstract

Code intelligence in Roko has a solid foundation: four working modules in `roko-index`, three language providers, and a clean trait-based architecture that separates language-specific parsing from language-agnostic analysis. However, the gap between what is built and what is needed for production-quality code intelligence is substantial. The index has no persistent storage, no search API, no MCP server, and no integration with the prompt assembly system.

This document provides a detailed inventory of what exists, what is missing, and a prioritized implementation roadmap organized into four tiers: critical (blocks self-hosting), important (significantly improves agent quality), enhancement (nice-to-have), and research (exploratory).

---

## What Exists Today

### roko-index (4 modules, ~1,151 lines, 30 tests)

| Module | File | Lines | Tests | Status | Core capability |
|---|---|---|---|---|---|
| `parser` | `src/parser.rs` | 142 | 5 | Built | Language-agnostic parse delegation |
| `symbol` | `src/symbol.rs` | 211 | 8 | Built | `SymbolId`, `SymbolRef`, `find_symbol()` |
| `graph` | `src/graph.rs` | 443 | 8 | Built | `SymbolGraph`, `build_graph()`, `pagerank()` |
| `hdc` | `src/hdc.rs` | 355 | 9 | Built | `HdcFingerprint`, `fingerprint_symbol()`, `similarity()` |

**Architecture strengths**:
- Clean module boundaries with well-defined public APIs
- Deterministic computations (no randomness in fingerprints or parsing)
- Comprehensive test coverage for core properties
- Performance-conscious implementation (sub-millisecond operations)
- `#[non_exhaustive]` on enums for forward compatibility

**Architecture limitations**:
- No trait abstraction for the index itself (no `CodeIndex` trait)
- No error types (functions return structs directly, no `Result`)
- In-memory only (no persistence across sessions)
- Graph builder only creates `Imports` edges

### Language providers (3 crates, ~2,336 lines)

| Crate | Lines | Languages | Parsing approach | Build systems |
|---|---|---|---|---|
| `roko-lang-rust` | 819 | Rust | Line-by-line heuristic | `CargoBuildSystem` |
| `roko-lang-typescript` | 917 | TypeScript, JavaScript | Line-by-line heuristic | `NpmBuildSystem`, `PnpmBuildSystem`, `YarnBuildSystem` |
| `roko-lang-go` | 600 | Go | Line-by-line heuristic | `GoBuildSystem` |

**Provider strengths**:
- All implement `LanguageProvider` trait from `roko-core`
- Handle common constructs correctly (functions, structs, enums, traits, imports)
- Visibility parsing across all three languages
- Build system abstractions for compile/test/lint/format

**Provider limitations**:
- Heuristic parsing misses nested definitions, multi-line signatures, macros
- No tree-sitter integration (the planned upgrade)
- No call site extraction (cannot create `Calls` edges)
- No scope nesting (flat symbol lists)
- No column-level source locations
- No test coverage reported in the crate-level test counts

### Integration points

| Integration | Status | Notes |
|---|---|---|
| `LanguageProvider` trait in `roko-core` | Stable | The contract between `roko-index` and language crates |
| `Symbol`, `SymbolKind`, `Visibility` in `roko-core` | Stable | Core types re-exported by `roko-index` |
| `Import`, `ImportKind` in `roko-core` | Stable | Import representation |
| MCP passthrough in `roko.toml` | Built | `agent.mcp_config` supports custom MCP servers |
| `SystemPromptBuilder` in `roko-compose` | Built | 6-layer prompt assembly, could consume code context |

---

## What Is Missing

### Tier 0: Critical (blocks self-hosting)

These are required for code intelligence to be usable by Roko's coding agents:

| Gap | Description | Effort | Depends on |
|---|---|---|---|
| **CodeIndex trait** | Unified trait abstracting over in-memory, SQLite, snapshot backends | Small | None |
| **Search API** | At minimum, keyword search over symbols by name | Medium | CodeIndex trait |
| **roko-compose integration** | Wire code search results into the context layer of `SystemPromptBuilder` | Medium | Search API |
| **CLI command** | `roko index build` / `roko index stats` — basic index management | Small | CodeIndex trait |

Without these, agents have no way to query code structure. The existing modules are libraries with no consumer.

### Tier 1: Important (significantly improves agent quality)

These make code intelligence meaningfully better than grep:

| Gap | Description | Effort | Depends on |
|---|---|---|---|
| **SQLite persistence** | Store symbols, edges, fingerprints in `.roko/index.db` | Large | CodeIndex trait |
| **BLAKE3 incremental updates** | Only re-index files whose content hash changed | Medium | SQLite |
| **MCP context server** | Expose 10 tools (`search_code`, `get_callers`, etc.) via MCP | Large | Search API, SQLite |
| **FTS5 keyword search** | Full-text search over symbol names with camelCase/snake_case tokenization | Medium | SQLite |
| **HDC similarity search** | Top-K nearest-neighbor via brute-force scan | Small | CodeIndex trait |
| **Graph expansion for context** | Include dependencies/callers of focal symbols in context | Medium | Search API |
| **Tree-sitter for Rust** | Replace heuristic parser with tree-sitter for accurate AST parsing | Medium | tree-sitter dependency |
| **Call graph edges** | Extract `Calls` edges from tree-sitter AST | Medium | Tree-sitter |

### Tier 2: Enhancement (nice-to-have)

These improve performance, coverage, or developer experience:

| Gap | Description | Effort | Depends on |
|---|---|---|---|
| **rkyv snapshots** | Zero-copy index loading via memory-mapped snapshots | Medium | SQLite or in-memory index |
| **Tree-sitter for TypeScript** | Accurate TS/JS parsing | Medium | tree-sitter |
| **Tree-sitter for Go** | Accurate Go parsing | Medium | tree-sitter |
| **Weighted PageRank** | Task-aware edge weights for personalized ranking | Small | PageRank (built) |
| **Code slicing** | Extract minimal code fragments instead of whole files | Medium | Tree-sitter |
| **Impact analysis API** | Structured impact reports from graph traversal | Small | Graph (built) |
| **Dense embeddings** | fastembed BGE-small integration for semantic search | Medium | CodeIndex trait |
| **RRF hybrid search** | Combine keyword, structural, HDC, and embedding results | Medium | Multiple search backends |
| **Subgraph extraction** | Extract relevant graph neighborhoods for context | Small | Graph (built) |
| **Context overlays** | Per-agent customization of index views | Medium | CodeIndex trait |

### Tier 3: Research (exploratory)

These are advanced capabilities from the legacy design documents:

| Gap | Description | Effort | Depends on |
|---|---|---|---|
| **HNSW index for HDC** | Approximate nearest-neighbor for large fingerprint sets | Medium | HDC search |
| **Salsa memoization** | Fine-grained incremental computation | Large | Tree-sitter |
| **Knowledge graph layer** | Connect code symbols to domain concepts | Large | Neuro integration |
| **Neuro-symbolic verification** | Combine code analysis with LLM reasoning for verification | Large | Gate integration |
| **Cross-domain insight resonance** | Detect structural analogies across different code domains | Medium | HDC fingerprints |
| **Privacy/redaction** | Prevent sensitive code from appearing in LLM context | Medium | MCP server |
| **Python/Java/C++ providers** | Additional language support | Medium per language | LanguageProvider trait |
| **Graph visualization** | DOT export for dependency graph visualization | Small | Graph (built) |

---

## Implementation Roadmap

### Phase 1: Foundation (unblocks agent use)

**Goal**: Agents can query code structure via CLI and basic API.

```
1.1  Define CodeIndex trait                    [2 days]
1.2  Implement InMemoryIndex                   [1 day]
1.3  Add keyword search (name matching)        [1 day]
1.4  Wire into roko-compose context layer      [2 days]
1.5  Add `roko index` CLI commands             [1 day]
1.6  Integration test: agent uses code index   [1 day]
```

**Deliverable**: `roko index build && roko run "what calls build_graph?"` works end-to-end.

### Phase 2: Persistence (survives restarts)

**Goal**: Index persists across sessions with incremental updates.

```
2.1  Add rusqlite dependency (feature-gated)   [1 day]
2.2  Implement SQLite schema + migrations      [2 days]
2.3  Implement SqliteIndex (CodeIndex trait)    [3 days]
2.4  Add BLAKE3 incremental update logic       [2 days]
2.5  Add FTS5 keyword search                   [1 day]
2.6  Benchmark: index build + query latency    [1 day]
```

**Deliverable**: `.roko/index.db` persists; re-indexing after a commit takes < 50ms.

### Phase 3: Agent-facing API (MCP server)

**Goal**: Agents access code intelligence via MCP tools.

```
3.1  Implement MCP stdio server skeleton       [2 days]
3.2  Implement search_code tool                [2 days]
3.3  Implement get_symbol_context tool         [1 day]
3.4  Implement get_callers + find_references   [2 days]
3.5  Implement workspace_map tool              [2 days]
3.6  Implement get_context (auto-assembly)     [3 days]
3.7  Configure in roko.toml MCP section        [1 day]
3.8  Integration test: agent uses MCP tools    [2 days]
```

**Deliverable**: Agents call `search_code("build dependency graph")` and get ranked results.

### Phase 4: Accuracy (tree-sitter)

**Goal**: Accurate AST parsing with call graph extraction.

```
4.1  Add tree-sitter dependency                [1 day]
4.2  Implement TreeSitterProvider for Rust     [3 days]
4.3  Extract Calls edges from call sites       [2 days]
4.4  Extract Implements edges from impl blocks [2 days]
4.5  Extract Contains edges from nesting       [1 day]
4.6  Update TypeScript provider                [2 days]
4.7  Update Go provider                        [2 days]
4.8  Benchmark: accuracy comparison            [1 day]
```

**Deliverable**: Graph has 4 edge kinds; call graph enables precise impact analysis.

---

## Quality Metrics

### How to measure code intelligence effectiveness

| Metric | Definition | Target |
|---|---|---|
| **Index coverage** | % of workspace symbols in the index | > 95% |
| **Parse accuracy** | % of symbols correctly extracted vs. manual audit | > 90% (heuristic), > 99% (tree-sitter) |
| **Search precision@10** | % of top-10 search results that are relevant | > 70% |
| **Context relevance** | % of context tokens that the agent actually uses | > 50% |
| **Token savings** | Reduction in tokens compared to whole-file context | > 5× |
| **Index build time** | Time to build index from scratch | < 1s (5K symbols) |
| **Incremental update time** | Time to update after a typical commit | < 50ms |
| **Query latency** | Time from tool call to response | < 100ms |

### Comparison with Aider

Aider (Gauthier 2024) provides a reference point for code intelligence in coding agents:

| Capability | Aider | Roko (current) | Roko (target) |
|---|---|---|---|
| Repository map | Tree-sitter-based | Not connected | Tree-sitter + PageRank + HDC |
| Context selection | Heuristic (file-level) | None | Graph + HDC + embedding + RRF |
| Incremental indexing | Git-based | None | BLAKE3 content hashing |
| Token optimization | Map-based reduction | None | Budget-aware Composer |
| Language support | 10+ languages | 3 (Rust, TS, Go) | 3 + extensible |
| Similarity search | None | HDC (built, not exposed) | HDC + embeddings |
| Impact analysis | None | Graph traversal (built, not exposed) | Weighted graph + callers |

Roko's advantage over Aider is the combination of multiple search strategies (not just tree-sitter maps), PageRank-based importance scoring, and HDC fingerprints for structural similarity — capabilities that Aider does not offer.

---

## Risk Assessment

### Technical risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Tree-sitter C dependency complicates builds | Medium | Medium | Feature-gate; fall back to heuristic parsers |
| SQLite write contention with multiple agents | Low | Medium | WAL mode; single writer design |
| rkyv version incompatibility (snapshot format breaks) | Medium | Low | Schema versioning; rebuild on version mismatch |
| fastembed model size (~100MB) | Low | Low | Feature-gate; HDC-only mode as default |
| FTS5 tokenization misses code patterns | Medium | Low | Custom tokenizer with camelCase splitting |

### Architecture risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| `CodeIndex` trait too narrow (limits future backends) | Medium | Medium | Design with 3+ backends in mind before stabilizing |
| MCP server becomes bottleneck (single process) | Low | Medium | Async handler with connection pooling |
| Heuristic parsers become unmaintainable | High | Low | Tree-sitter replaces them; heuristics are fallback only |

---

## Academic Foundations

- **Aider**: Gauthier (2024). The primary reference implementation for coding agent code intelligence. Tree-sitter repository maps, context selection, and token optimization.
- **Meta-Harness**: Lee et al. (2026), arXiv:2603.28052. Demonstrates that harness optimization (which includes context engineering) provides measurable, significant improvements to agent task performance.
- **cAST**: Xiao et al. (2024). Code AST-based approaches to code understanding, demonstrating tree-sitter's effectiveness for code intelligence tasks.
- **AriGraph**: (2024). Graph-based code navigation and understanding, validating the dependency graph approach.
- **ChatHTN**: (2025). Hierarchical task network planning for code understanding, relevant to the `get_context` auto-assembly tool.
- **LOOP**: (2025). Learning to optimize prompts for code generation, showing direct correlation between context quality and generation quality.

---

## Cross-References

- See [00-vision.md](./00-vision.md) for the motivation and design principles
- See [01-tree-sitter-parsing.md](./01-tree-sitter-parsing.md) for the planned tree-sitter migration (Phase 4)
- See [07-mcp-context-server.md](./07-mcp-context-server.md) for the agent-facing API (Phase 3)
- See [08-index-db-scaling.md](./08-index-db-scaling.md) for SQLite persistence (Phase 2)
- See [09-snapshot-optimization.md](./09-snapshot-optimization.md) for zero-copy snapshots (Tier 2)
- See topic [01-orchestration](../01-orchestration/INDEX.md) for how the orchestrator invokes code intelligence
- See topic [05-learning](../05-learning/INDEX.md) for how code intelligence data feeds learning loops
