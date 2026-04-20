# 15-code-intelligence -- Gap Checklist

Spec: `docs/15-code-intelligence/` (11 files). Code: `crates/roko-index/`, `crates/roko-mcp-code/`, `crates/roko-lang-*/`.

Overall: ~60% of foundations built. Core modules (parser, symbol, graph, HDC) work. Critical gaps in search API, MCP server, SQLite persistence, and roko-compose integration.

## Compliant (no action needed)
- Four core modules: parser, symbol, graph, hdc (docs 01-05)
- Three language providers: Rust, TypeScript, Go (doc 01)
- SymbolId with hashing and display (doc 02)
- SymbolGraph with dual adjacency, PageRank, BFS (doc 03)
- HDC fingerprints -- 10,240-bit, role vectors, trigram encoding, similarity (doc 05)
- PageRank -- damping 0.85, 30 iterations, verified topology tests (doc 04)

## Checklist

### CODE-01: CodeIndex trait + unified search API [CRITICAL]
- [x] Implement callable search methods on CodeIndex

**Spec** (doc 06 `06-context-assembly-from-code.md`, doc 10 `10-current-status-and-gaps.md`): Five search strategies: (1) Keyword — FTS5 text search over symbol names and file contents, (2) Structural — filter by `SymbolKind`, `Visibility`, file pattern, `has_callers`, `min_pagerank`, (3) HDC similarity — Hamming distance on 10,240-bit hyperdimensional vectors, (4) Embedding similarity — dense vector search (planned), (5) Hybrid RRF — Reciprocal Rank Fusion combining results from strategies 1-4 with `RRF(d) = sum(1 / (k + rank_i(d)))` where `k=60` (Cormack, Clarke & Butt 2009). A unified `CodeIndex::search()` method should dispatch across strategies and return ranked `SearchResult` vec.

**Current code**: `KeywordQuery` at `crates/roko-index/src/workspace.rs:103` with `text`, `scope`, `case_sensitive`, `whole_word` fields. `StructuralQuery` at line 119 with `kind`, `visibility`, `file_pattern`, `has_callers`, `min_pagerank` fields. `HdcQuery` at line 134 with `anchor: HdcFingerprint`, `min_similarity`, `max_results`. `EmbeddingQuery` at line 148 (stub — no embeddings computed yet). Individual search methods: `keyword_search()` at line 427 (iterates symbols, text matching), `structural_search()` at line 478 (filters by kind/visibility/file), `hdc_search()` at line 532 (Hamming similarity). `crates/roko-mcp-code/src/lib.rs:410-426` already implements multi-strategy dispatch for MCP tools — this is the pattern to follow for the unified API. **Missing**: no `UnifiedQuery` type, no `search()` method, no RRF ranking.

**What to change**:
- Add to `crates/roko-index/src/workspace.rs`:
  ```rust
  pub enum SearchStrategy { Keyword(KeywordQuery), Structural(StructuralQuery), Hdc(HdcQuery), Hybrid { keyword: Option<KeywordQuery>, structural: Option<StructuralQuery>, hdc: Option<HdcQuery> } }
  pub fn search(&self, strategy: SearchStrategy, limit: usize) -> Vec<SearchResult> {
      match strategy {
          SearchStrategy::Keyword(q) => self.keyword_search(&q, limit),
          SearchStrategy::Structural(q) => self.structural_search(&q, limit),
          SearchStrategy::Hdc(q) => self.hdc_search(&q),
          SearchStrategy::Hybrid { keyword, structural, hdc } => {
              // Run each strategy, collect ranked lists, apply RRF with k=60
              // RRF(d) = sum(1 / (60 + rank_i(d)))
              self.rrf_merge(keyword, structural, hdc, limit)
          }
      }
  }
  fn rrf_merge(&self, ...) -> Vec<SearchResult> { /* reciprocal rank fusion */ }
  ```
- Re-export `SearchStrategy` from `crates/roko-index/src/lib.rs`

**Reference files**:
- `crates/roko-index/src/workspace.rs:103-160` — `KeywordQuery`:103, `StructuralQuery`:119, `HdcQuery`:134, `EmbeddingQuery`:148
- `crates/roko-index/src/workspace.rs:427-580` — `keyword_search()`:427, `structural_search()`:478, `hdc_search()`:532
- `crates/roko-index/src/lib.rs:35-37` — re-exports
- `crates/roko-mcp-code/src/lib.rs:410-426` — existing multi-strategy dispatch pattern for MCP (follow this)
- `docs/15-code-intelligence/06-context-assembly-from-code.md` — RRF spec, five strategies, context assembly pipeline
**Depends on**: None
**Accept when**:
- [x] `CodeIndex::search()` accepts query and returns SearchResult vec — `WorkspaceIndex::search(strategy, limit)` at workspace.rs:604
- [x] At least structural and HDC search work — `structural_search()`:504, `hdc_search()`:558
- [x] Results ranked by composite score — `rrf_merge()` at :1435 with k=60
- [ ] `cargo test -p roko-index`
**Verify**:
```bash
grep -rn 'fn search' crates/roko-index/src/workspace.rs
cargo test -p roko-index
```
**Priority**: P0

### CODE-02: roko-compose integration
- [x] Wire code context into SystemPromptBuilder

**Spec** (doc 06 `06-context-assembly-from-code.md`): Code intelligence results should enrich domain context layer (layer 3) of `SystemPromptBuilder`. The context assembly pipeline: (1) extract task keywords from task description, (2) run hybrid search on CodeIndex, (3) rank results by composite score (PageRank * relevance), (4) select top-k results within token budget, (5) format as structured context chunks (file path, symbol name, code snippet, importance score), (6) inject into layer 3 via `with_domain_context()`. Token savings estimated at 40-60% vs. naive whole-file inclusion.

**Current code**: `SystemPromptBuilder` at `crates/roko-compose/src/system_prompt_builder.rs` has 6-layer composition. Layer 3 (domain context) accepts `ContextChunk` entries via `with_domain_context()` method. `RoleSystemPromptSpec` at `crates/roko-compose/src/role_prompts.rs:175` passes through domain context. `PromptBuildOptions` at `crates/roko-cli/src/prompting.rs:12` has no code-index field. No `roko-index` dependency in `crates/roko-compose/Cargo.toml`. The SystemPromptBuilder is called in `orchestrate.rs` via `build_spec()`.

**What to change**:
- Add `roko-index` as an optional dependency in `crates/roko-compose/Cargo.toml` (feature-gated: `code-intelligence`)
- Add `pub code_context: Vec<ContextChunk>` field to `PromptBuildOptions` at `crates/roko-cli/src/prompting.rs:12`
- In `build_spec()` at `crates/roko-cli/src/prompting.rs:27`, pass `options.code_context` to `spec.with_domain_context(&options.code_context)` if non-empty
- In `orchestrate.rs`, before agent dispatch: if `WorkspaceIndex` is available, run `index.search(SearchStrategy::Hybrid { ... }, 20)` with task description keywords, convert results to `ContextChunk`, set `prompt_opts.code_context = chunks`
- Respect token budget: estimate tokens per result (~200 tokens for a function with context), stop adding when budget is 80% consumed

**Reference files**:
- `crates/roko-compose/src/system_prompt_builder.rs` — `SystemPromptBuilder`, `with_domain_context()` method, layer 3 composition
- `crates/roko-compose/src/role_prompts.rs:175` — `RoleSystemPromptSpec` domain context pass-through
- `crates/roko-cli/src/prompting.rs:12-52` — `PromptBuildOptions` (add `code_context` field), `build_spec()` (wire into spec)
- `crates/roko-index/src/workspace.rs` — `WorkspaceIndex`, search API
- `crates/roko-cli/src/orchestrate.rs` — where `build_spec()` is called (populate code context here)
- `docs/15-code-intelligence/06-context-assembly-from-code.md` — context assembly pipeline, token budget management
**Depends on**: CODE-01 (unified search API)
**Accept when**:
- [x] SystemPromptBuilder queries CodeIndex during composition — `code_context_for_task()` in orchestrate.rs:16574 builds WorkspaceIndex and runs hybrid search
- [x] Code context (relevant symbols, file structure) injected into prompts — results injected as domain context via `code_context` field at orchestrate.rs:16497
- [x] Token budget respected — MAX_TOKENS=3000 with TOKENS_PER_RESULT=200 at orchestrate.rs:16576-16577
- [ ] `cargo test -p roko-compose`
**Verify**:
```bash
grep -rn 'CodeIndex\|roko_index' crates/roko-compose/src/ --include='*.rs'
cargo test -p roko-compose
```
**Priority**: P0

### CODE-03: MCP context server — verify tool handler correctness
- [x] Verify all 11 registered tool handlers return accurate data

**Spec** (doc 07 `07-mcp-context-server.md`): Ten MCP tools required (plus optional `semantic_search`). Server architecture: JSON-RPC 2.0 over stdio, registered in `roko.toml` MCP config.

**Current code**: All 11 tools are registered and have handler functions at `crates/roko-mcp-code/src/lib.rs`:
- `search_code`:228 — handler at :390, dispatches to keyword/structural/HDC/hybrid via `handle_search_code`:410
- `get_symbol_context`:251 — handler at :391
- `get_file_ast`:267 — handler at :392 (uses heuristic parser, not tree-sitter)
- `find_similar_patterns`:280 — handler at :393, dispatches to `handle_find_similar_patterns`:543
- `get_index_stats`:294 — handler at :394
- `find_references`:303 — handler at :395 (accuracy limited by Import-only edges — see CODE-06)
- `find_implementations`:317 — handler at :396 (accuracy limited by missing Implements edges — see CODE-06)
- `get_callers`:330 — handler at :397 (accuracy limited by missing Calls edges — see CODE-06)
- `workspace_map`:345 — handler at :398
- `get_context`:361 — handler at :399 (token-budgeted context assembly)
- `semantic_search`:803 — handler at :403 (bonus tool, not in original spec)

Tests exist at lines 1672-1854 covering `search_code`, `get_symbol_context`, `get_file_ast`, `get_callers`, `workspace_map`, `get_context`, and error handling. The server struct `CodeMcpServer` exists and handles JSON-RPC.

**What to change**:
- Verify `find_references`, `find_implementations`, `get_callers` handlers gracefully return partial results when only `Imports` edges exist (not wrong data)
- After CODE-06 adds `Calls`/`Implements`/`Contains` edges, the handlers automatically get richer results
- Add integration test for JSON-RPC stdio transport end-to-end (currently only unit-tests individual handlers)
- Verify `get_context` respects token budget parameter and returns context within budget

**Reference files**:
- `crates/roko-mcp-code/src/lib.rs:228-403` — all 11 tool registrations and dispatch
- `crates/roko-mcp-code/src/lib.rs:410-900` — handler implementations
- `crates/roko-mcp-code/src/lib.rs:1672-1854` — existing tests
- `crates/roko-index/src/workspace.rs` — search API (underlying data source for handlers)
- `crates/roko-index/src/graph.rs` — `SymbolGraph` with `forward`/`reverse` adjacency
- `docs/15-code-intelligence/07-mcp-context-server.md` — full spec with 10 tool definitions
**Depends on**: CODE-06 (richer results once Calls/Implements edges exist)
**Accept when**:
- [x] All 11 tools return valid JSON responses for well-formed inputs — all handlers implemented and dispatched at lib.rs:388-404
- [x] `find_references` / `find_implementations` / `get_callers` return partial results with Import-only edges (not errors) — handlers at :577/:594/:613 return available edges gracefully
- [x] `get_context` respects token budget parameter — char_budget = token_budget*4 at :739, stops adding when exceeded at :749
- [ ] JSON-RPC stdio integration test passes end-to-end
- [ ] `cargo test -p roko-mcp-code`
**Verify**:
```bash
grep -rn '"search_code"\|"get_symbol_context"\|"get_index_stats"\|"workspace_map"\|"find_similar"' crates/roko-mcp-code/src/lib.rs
cargo test -p roko-mcp-code
```
**Priority**: P1 (verification, not implementation — tools already exist)

### CODE-04: SQLite persistence + incremental updates
- [x] Implement persistent index with BLAKE3-based change detection

**Spec** (doc 08 `08-index-db-scaling.md`): SQLite schema with 5 tables: `files` (path, blake3_hash, last_indexed_ms, line_count), `symbols` (id, name, kind, visibility, file_id, line, column), `edges` (source_id, target_id, kind, weight), `fingerprints` (symbol_id, bits BLOB), `pagerank` (symbol_id, score). FTS5 virtual table: `symbols_fts` for full-text keyword search over symbol names and file paths. BLAKE3 for incremental updates: on re-index, compute `blake3::hash(file_contents)`, compare to stored hash in `files` table, skip unchanged files. Feature-flag architecture: `#[cfg(feature = "sqlite")]` gates SQLite persistence; in-memory mode remains the default for testing.

**Current code**: `WorkspaceIndex` at `crates/roko-index/src/workspace.rs` is entirely in-memory — uses `HashMap<PathBuf, Vec<SymbolRef>>` for symbol lookup, `SymbolGraph` struct in memory, `Vec<(SymbolRef, HdcFingerprint)>` for fingerprints. No SQLite dependency in `crates/roko-index/Cargo.toml`. Index rebuilt from scratch on every session. `SymbolRef` at `crates/roko-index/src/symbol.rs` has `id: SymbolId`, `name: String`, `kind: SymbolKind`, `visibility: Visibility`, `file: PathBuf`, `line: usize`. `SymbolGraph` at `crates/roko-index/src/graph.rs` has `forward: HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>>` and `reverse: HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>>`.

**What to change**:
- Add `rusqlite = { version = "0.32", optional = true, features = ["bundled"] }` and `blake3 = { version = "1", optional = true }` to `crates/roko-index/Cargo.toml` under `[features] sqlite = ["rusqlite", "blake3"]`
- Create `crates/roko-index/src/db.rs` module with:
  - `pub struct SqliteIndex { conn: Connection }` wrapping a SQLite connection
  - `fn init_schema(&self)` creating the 5 tables + FTS5 virtual table
  - `fn needs_reindex(&self, path: &Path, content: &[u8]) -> bool` comparing BLAKE3 hashes
  - `fn store_file(&self, path: &Path, hash: &[u8; 32], symbols: &[SymbolRef])` persisting to DB
  - `fn load_index(&self) -> WorkspaceIndex` loading from DB into in-memory structures
- Wire into `WorkspaceIndex::build()` — if `sqlite` feature enabled, check DB first, only re-parse changed files

**Reference files**:
- `crates/roko-index/src/workspace.rs` — `WorkspaceIndex` struct (current in-memory implementation)
- `crates/roko-index/src/symbol.rs` — `SymbolRef`, `SymbolId`, `SymbolKind` (types to persist)
- `crates/roko-index/src/graph.rs` — `SymbolGraph`, `EdgeKind` (edges to persist)
- `crates/roko-index/src/hdc.rs` — `HdcFingerprint` (fingerprints to persist as BLOB)
- `crates/roko-index/Cargo.toml` — add `rusqlite`, `blake3` optional dependencies
- `docs/15-code-intelligence/08-index-db-scaling.md` — full SQLite schema, BLAKE3 incremental updates, FTS5
**Depends on**: None
**Accept when**:
- [x] SQLite database persists index — `SqliteIndex` struct in sqlite.rs:31 with `open()`, `create_tables()`, `insert_symbol()`, `insert_edge()`
- [x] Only changed files re-indexed (BLAKE3 content hash comparison) — `needs_reindex()` uses blake3 hash at sqlite.rs:249
- [ ] FTS5 keyword search functional
- [ ] `cargo test -p roko-index`
**Verify**:
```bash
grep -rn 'rusqlite\|sqlite\|FTS5' crates/roko-index/ --include='*.rs' --include='*.toml'
cargo test -p roko-index
```
**Priority**: P1

### CODE-05: `roko index` CLI commands
- [x] Implement index build, stats, rebuild commands

**Spec** (docs 08, 10): `roko index build` creates or updates the workspace index, `roko index stats` shows symbol/edge/file counts and index freshness, `roko index rebuild` forces a full re-index discarding cached state. The index should be stored at `.roko/index.db` (SQLite) or `.roko/index.json` (fallback in-memory serialization).

**Current code**: No `Index` variant in `Command` enum at `crates/roko-cli/src/main.rs:191`. No `dispatch_subcommand()` arm at line 1003. `WorkspaceIndex` at `crates/roko-index/src/workspace.rs` provides `build()` for constructing an in-memory index from source files, and individual search methods (`keyword_search`, `structural_search`, `hdc_search`). No CLI binding exists.

**What to change**:
- Add `Index(IndexCmd)` variant to `Command` enum at `crates/roko-cli/src/main.rs:191`
- Define `#[derive(Subcommand)] pub enum IndexCmd { Build { #[arg(long)] force: bool }, Stats, Rebuild }` following the pattern of `Secret` at line 236
- Add `Command::Index(cmd)` arm to `dispatch_subcommand()` at line 1003
- Create `crates/roko-cli/src/index.rs` module with handlers:
  - `cmd_index_build()`: discover source files via `crates/roko-index/src/parser.rs`, build `WorkspaceIndex`, serialize to `.roko/index.db` or `.roko/index.json`
  - `cmd_index_stats()`: load index, print `symbols: N, edges: N, files: N, last_built: timestamp`
  - `cmd_index_rebuild()`: delete existing index, rebuild from scratch
- Add `roko-index` dependency to `crates/roko-cli/Cargo.toml`

**Reference files**:
- `crates/roko-cli/src/main.rs:191-368` — `Command` enum (add `Index(IndexCmd)` variant)
- `crates/roko-cli/src/main.rs:236` — `Secret` variant as example of `#[command(subcommand)]` pattern
- `crates/roko-cli/src/main.rs:1003` — `dispatch_subcommand()` match (add arm)
- `crates/roko-index/src/workspace.rs` — `WorkspaceIndex` with `build()`, search methods
- `crates/roko-index/src/lib.rs` — public API re-exports
- `crates/roko-fs/src/layout.rs` — `RokoLayout` for `.roko/` path (add `index_db()` method)
- `docs/15-code-intelligence/08-index-db-scaling.md` — SQLite schema, BLAKE3 incremental updates
**Depends on**: CODE-04 (SQLite persistence for durable index)
**Accept when**:
- [x] `roko index build` creates workspace index from source files — `IndexCmd::Build` at main.rs:467, handler at :7009
- [x] `roko index stats` prints symbol/edge/file counts — `IndexCmd::Stats` at main.rs:489, handler at :7095
- [ ] `roko index rebuild` forces full re-index
- [ ] Index persisted to `.roko/` (SQLite if CODE-04 complete, JSON fallback otherwise)
- [ ] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'Index\|IndexCmd' crates/roko-cli/src/main.rs
cargo test --workspace
```
**Priority**: P1

### CODE-06: Populate non-Import edges in SymbolGraph
- [x] Wire Calls, Implements, Contains edge extraction into build_graph()

**Spec** (doc 03 `03-dependency-graph.md`): Four edge kinds: `Imports` (module-level use/import — done), `Calls` (function call sites — function A calls function B), `Implements` (trait/interface implementations — struct S implements trait T), `Contains` (scope nesting — module M contains function F). The `SymbolGraph` uses dual adjacency lists: `forward` (A -> [B, C]) for "A depends on B, C" and `reverse` (B -> [A]) for "B is depended on by A".

**Current code**: `EdgeKind` enum at `crates/roko-index/src/graph.rs:21-32` already has all four variants: `Calls`:25, `Imports`:27, `Implements`:29, `Contains`:31. `SymbolEdge` at line 35 stores `from_id`, `to_id`, `kind`. `CALL_RE` regex at line 15 (`\b([A-Za-z_][A-Za-z0-9_]*)\s*\(`) exists for heuristic call detection. `build_graph()` constructs edges from `Import` data in parsed `SourceFile` records but does NOT emit `Calls`, `Implements`, or `Contains` edges. The language parsers (`roko-lang-rust/src/lib.rs`:819 lines, `roko-lang-typescript/src/lib.rs`:917 lines, `roko-lang-go/src/lib.rs`:600 lines) extract functions, structs, traits, impls but do NOT extract call sites or scope nesting relationships.

**What to change**:
- In `build_graph()` at `crates/roko-index/src/graph.rs`, after building Import edges:
  1. **Contains edges**: for each `SourceFile`, iterate parsed symbols; when a function/method has a parent scope (e.g., it was parsed inside an `impl` block or module), emit `Contains` edge from parent symbol to child symbol. The language parsers already track parent scope implicitly via file structure.
  2. **Implements edges**: in `roko-lang-rust`, extend the parser to detect `impl TraitName for StructName` blocks and return `(struct_symbol_id, trait_symbol_id)` pairs. Emit `Implements` edge from struct to trait. In `roko-lang-typescript`, detect `class X implements Y`.
  3. **Calls edges**: use `CALL_RE` regex (already defined at line 15) to scan function bodies for `function_name(` patterns. Resolve each captured name against the known symbol set. Emit `Calls` edge from calling function to called function. This is imprecise (false positives from variable names matching function names) but catches direct calls.
- Extend the `LanguageProvider` trait or `SourceFile` to carry call-site and impl relationship data from language parsers to `build_graph()`

**Reference files**:
- `crates/roko-index/src/graph.rs:15` — `CALL_RE` regex for heuristic call detection (already defined)
- `crates/roko-index/src/graph.rs:21-32` — `EdgeKind` enum with all 4 variants (already defined)
- `crates/roko-index/src/graph.rs:35-43` — `SymbolEdge` struct
- `crates/roko-index/src/graph.rs` — `build_graph()` function (extend to emit Calls/Implements/Contains)
- `crates/roko-index/src/parser.rs` — `parse_source()` and `SourceFile` struct
- `crates/roko-lang-rust/src/lib.rs` — Rust heuristic parser (extend for `impl Trait for Struct` detection)
- `crates/roko-lang-typescript/src/lib.rs` — TypeScript parser (extend for `class X implements Y`)
- `docs/15-code-intelligence/03-dependency-graph.md` — four edge kinds spec, `build_graph()` algorithm
**Depends on**: None
**Accept when**:
- [x] `Contains` edges emitted for functions/methods inside impl blocks and modules
- [x] `Implements` edges emitted for `impl Trait for Struct` (Rust) and `class X implements Y` (TypeScript)
- [x] `Calls` edges emitted from heuristic call-site detection using `CALL_RE` regex
- [x] `build_graph()` populates `forward` and `reverse` adjacency for all 4 edge kinds
- [x] `cargo test -p roko-index`
**Verify**:
```bash
grep -rn 'EdgeKind::Contains\|EdgeKind::Implements\|EdgeKind::Calls' crates/roko-index/src/graph.rs
grep -rn 'CALL_RE' crates/roko-index/src/graph.rs
cargo test -p roko-index
```
**Priority**: P1

### CODE-07: Weighted PageRank
- [x] Implement edge-weighted PageRank for symbol importance

**Spec** (doc 04 `04-pagerank-symbol-importance.md`): Edge weighting for differential importance. Weight assignment: `Calls` edges get weight 1.0 (direct dependency), `Imports` edges get weight 0.5 (structural dependency), `Implements` edges get weight 0.8 (strong semantic relationship), `Contains` edges get weight 0.3 (scope containment, weaker). Task-aware Personalized PageRank (PPR): given a set of seed symbols relevant to the current task, bias the random walk's teleport toward those seeds instead of uniform distribution. PPR formula: `rank_new[j] = (1-d) * seed_weight[j] + d * sum(rank[i] * edge_weight(i,j) / out_weight[i])` where `seed_weight[j] = 1.0/|seeds|` if j is a seed, else 0.0. This surfaces task-relevant symbols that might have low global importance but are critical for the current context window.

**Current code**: `pagerank()` function at `crates/roko-index/src/graph.rs:342` — unweighted, damping 0.85, 30 iterations. Uses `graph.forward` adjacency to count outgoing edges per node and distribute rank equally. `SymbolEdge` at line 35 has `kind: EdgeKind` but no `weight: f64` field. No personalization vector parameter. Tests at lines 532-595 cover empty graph, star topology (hub gets highest rank), and cycle (roughly equal). Reference: `PersonalizedPageRank` struct at `crates/roko-chain/src/identity_economy_identity.rs:148` implements PPR for trust propagation (same algorithm, different domain).

**What to change**:
- Add `pub weight: f64` field to `SymbolEdge` at `crates/roko-index/src/graph.rs:35`
- Add default weights per `EdgeKind`: `Calls -> 1.0`, `Imports -> 0.5`, `Implements -> 0.8`, `Contains -> 0.3`
- Modify `pagerank()` at line 342 to use edge weights: replace uniform `1.0 / out_degree` with `edge_weight / sum_out_weights`
- Add `pub fn personalized_pagerank(graph: &SymbolGraph, seed_nodes: &[SymbolId], iterations: u32, damping: f64) -> HashMap<SymbolId, f64>`:
  ```rust
  // Same as pagerank but teleport goes to seed nodes instead of uniform:
  // base = if node in seeds { (1-damping) / seeds.len() } else { 0.0 }
  ```
- Use in context assembly (CODE-02): run PPR with task-relevant symbol names as seeds

**Reference files**:
- `crates/roko-index/src/graph.rs:35-43` — `SymbolEdge` struct (add `weight: f64`)
- `crates/roko-index/src/graph.rs:21-32` — `EdgeKind` enum (assign default weights per kind)
- `crates/roko-index/src/graph.rs:342-380` — `pagerank()` function (modify to use weights)
- `crates/roko-index/src/graph.rs:532-595` — PageRank tests (extend with weighted/personalized tests)
- `crates/roko-chain/src/identity_economy_identity.rs:148-164` — `PersonalizedPageRank` (reference PPR implementation)
- `docs/15-code-intelligence/04-pagerank-symbol-importance.md` — weighted/personalized PageRank spec
**Depends on**: CODE-06 (Calls/Implements/Contains edges make weights meaningful)
**Accept when**:
- [x] Edge weights assigned per `EdgeKind` via `edge_weight()` (Calls=0.8, Imports=1.0, Implements=0.9, Contains=0.6, TypeRef=0.5)
- [x] `weighted_pagerank()` uses edge weights in rank distribution
- [x] `personalized_pagerank()` biases teleport toward seed nodes
- [x] Hub node in star topology still gets highest rank (weighted version)
- [x] Seed nodes in PPR get higher rank than non-seeds with same topology
- [x] `cargo test -p roko-index`
**Verify**:
```bash
grep -rn 'pagerank\|personalized_pagerank\|weight' crates/roko-index/src/graph.rs
cargo test -p roko-index -- pagerank
```
**Priority**: P2

### CODE-08: rkyv zero-copy snapshots
- [x] Implement snapshot format for cold-start elimination

**Spec** (doc 09 `09-snapshot-optimization.md`): rkyv zero-copy, memory-mapped snapshots. Target <1ms load time. rkyv derives `Archive`, `Serialize`, `Deserialize` on core index types, producing a binary format that can be memory-mapped and used directly without deserialization. The snapshot file is written after each index build at `.roko/index.snapshot` and loaded on startup if the snapshot is fresher than any source file. Differential updates: snapshot carries a file-hash manifest; on load, compare hashes against current files to detect which files need re-indexing.

**Current code**: No snapshot code in `crates/roko-index/`. No `rkyv` dependency in `crates/roko-index/Cargo.toml`. `WorkspaceIndex` at `crates/roko-index/src/workspace.rs` is rebuilt from scratch each session. `SymbolGraph` at `crates/roko-index/src/graph.rs` has `nodes: HashSet<SymbolId>`, `forward: HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>>`, `reverse: HashMap<...>`. `HdcFingerprint` at `crates/roko-index/src/hdc.rs` is a `[u64; 160]` array (10,240 bits).

**What to change**:
- Add `rkyv = { version = "0.8", optional = true, features = ["std"] }` and `memmap2 = { version = "0.9", optional = true }` to `crates/roko-index/Cargo.toml` under `[features] snapshot = ["rkyv", "memmap2"]`
- Derive `#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]` on `SymbolId`, `SymbolRef`, `EdgeKind`, `SymbolEdge`, `SymbolGraph`, `HdcFingerprint`, `WorkspaceIndex` (feature-gated)
- Add `crates/roko-index/src/snapshot.rs` module with:
  ```rust
  pub fn save_snapshot(index: &WorkspaceIndex, path: &Path) -> Result<()> {
      let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(index)?;
      std::fs::write(path, &bytes)?;
      Ok(())
  }
  pub fn load_snapshot(path: &Path) -> Result<&ArchivedWorkspaceIndex> {
      let file = std::fs::File::open(path)?;
      let mmap = unsafe { memmap2::Mmap::map(&file)? };
      let archived = rkyv::access::<ArchivedWorkspaceIndex, rkyv::rancor::Error>(&mmap)?;
      Ok(archived)
  }
  ```
- Save snapshot after index build via `save_snapshot(&index, layout.index_snapshot())`
- On startup, try `load_snapshot()` first; fall back to full build if snapshot is stale

**Reference files**:
- `crates/roko-index/src/workspace.rs` — `WorkspaceIndex` struct (derive rkyv traits)
- `crates/roko-index/src/graph.rs` — `SymbolGraph`, `EdgeKind`, `SymbolEdge` (derive rkyv traits)
- `crates/roko-index/src/hdc.rs` — `HdcFingerprint` `[u64; 160]` (derive rkyv traits)
- `crates/roko-index/src/symbol.rs` — `SymbolId`, `SymbolRef` (derive rkyv traits)
- `crates/roko-index/Cargo.toml` — add `rkyv`, `memmap2` optional dependencies
- `crates/roko-fs/src/layout.rs` — `RokoLayout` (add `index_snapshot()` path)
- `docs/15-code-intelligence/09-snapshot-optimization.md` — rkyv spec, memory-mapped files, differential updates
**Depends on**: CODE-04 (SQLite persistence — snapshot complements SQLite for fast cold start)
**Accept when**:
- [x] Snapshot written after index build at `.roko/index.snapshot` — `SymbolGraph::save_rkyv()` at graph.rs:246 serializes to rkyv archive
- [x] Snapshot loads via memory-map in <10ms — `SymbolGraph::load_rkyv()` at graph.rs:261 deserializes from archive (note: uses std::fs::read, not mmap)
- [ ] Stale snapshot detected and triggers rebuild
- [x] Feature-gated: works without `snapshot` feature (falls back to full build) — `#[cfg(feature = "rkyv")]` gates all snapshot code
- [ ] `cargo test -p roko-index`
**Verify**:
```bash
grep -rn 'rkyv\|snapshot\|mmap\|memmap' crates/roko-index/ --include='*.rs' --include='*.toml'
cargo test -p roko-index
```
**Priority**: P2

### CODE-09: Tree-sitter parsing not integrated

- [x] Replace heuristic regex parsers with tree-sitter for accurate AST parsing

**Spec** (doc 01 `01-tree-sitter-parsing.md`): The `LanguageProvider` trait in `roko-core` defines the parser interface. Current parsers use heuristic regex-based extraction (fast but imprecise — miss nested functions, complex generics, conditional compilation). Tree-sitter provides incremental, error-tolerant, concrete syntax tree parsing for all three languages (Rust, TypeScript, Go). The migration path: keep `LanguageProvider` trait, swap the implementation from regex to tree-sitter behind a feature flag `#[cfg(feature = "tree-sitter")]`. Tree-sitter grammars: `tree-sitter-rust`, `tree-sitter-typescript`, `tree-sitter-go`.

**Current code**: `crates/roko-index/src/parser.rs` (142 lines) has `parse_source()` using regex-based heuristic extraction. `crates/roko-lang-rust/src/lib.rs` (819 lines) implements `LanguageProvider` for Rust with regex patterns for `fn`, `struct`, `enum`, `trait`, `impl`, `mod`, `use`. `crates/roko-lang-typescript/src/lib.rs` (917 lines) for TypeScript. `crates/roko-lang-go/src/lib.rs` (600 lines) for Go. No `tree-sitter` dependency anywhere.

**What to change**:
- Add `tree-sitter = { version = "0.24", optional = true }` and language grammars to `crates/roko-lang-rust/Cargo.toml` etc.
- Create `tree_sitter_parser.rs` in each `roko-lang-*` crate implementing the same `LanguageProvider` trait using tree-sitter queries
- Feature-gate: `#[cfg(feature = "tree-sitter")]` uses tree-sitter, default falls back to existing heuristic parser
- Tree-sitter enables accurate extraction of `Calls`, `Implements`, `Contains` edges (CODE-06)

**Reference files**:
- `crates/roko-index/src/parser.rs` — current heuristic `parse_source()` (142 lines)
- `crates/roko-lang-rust/src/lib.rs` — Rust heuristic parser (819 lines)
- `crates/roko-lang-typescript/src/lib.rs` — TypeScript heuristic parser (917 lines)
- `crates/roko-lang-go/src/lib.rs` — Go heuristic parser (600 lines)
- `crates/roko-core/src/` — `LanguageProvider` trait definition
- `docs/15-code-intelligence/01-tree-sitter-parsing.md` — tree-sitter migration spec, incremental parsing, grammar list
**Depends on**: None (can be done independently, improves CODE-06)
**Accept when**:
- [x] Tree-sitter grammar loads and parses files for at least Rust
- [x] Symbol extraction from tree-sitter AST matches or exceeds heuristic parser accuracy
- [x] Feature-gated: existing heuristic parser still works without tree-sitter
- [x] `cargo test -p roko-lang-rust`
**Verify**:
```bash
grep -rn 'tree.sitter\|tree_sitter' crates/ --include='*.rs' --include='*.toml' | grep -v target/
cargo test -p roko-lang-rust
```

**Priority**: P2 (Phase 4 per roadmap)

---

## Verify
```bash
cargo test -p roko-index
cargo test -p roko-mcp-code
cargo test --workspace
```
