# MCP Server and Persistence

> Depth for the MCP server (Connect Cell), SQLite persistence (Store Cell), and snapshot
> optimization (Store Cell variant). Covers the 10 MCP tools, the SQLite schema, BLAKE3
> incremental updates, FTS5 search, rkyv zero-copy snapshots, and Salsa memoization.

---

## MCP Server as a Connect Cell

The MCP context server is a **Connect Cell** -- implementing the Connect protocol to expose
the code intelligence Pipeline via Model Context Protocol. See
[02-CELL.md](../../unified/02-CELL.md) for the Connect protocol and
[11-CONNECTIVITY.md](../../unified/11-CONNECTIVITY.md) for exoskeleton protocols.

MCP is one of the four exoskeleton protocols (MCP, A2A, ERC-8004, x402). The code intelligence
server uses the same MCP infrastructure as `roko-mcp-code`, `roko-mcp-github`, and other MCP
integrations. JSON-RPC over stdio transport -- the server starts as a child process of the
orchestrator and communicates via stdin/stdout.

### The 10 MCP Tools

Each tool wraps a capability from the code intelligence Pipeline:

| Tool | Maps to Cell | What it does |
|---|---|---|
| `search_code` | Search Cell (Route) | Multi-strategy code search with RRF |
| `get_symbol_context` | Graph Cell (Store) | Full context: definition + deps + callers + PageRank |
| `get_file_ast` | Parse Cell (Connect) | Symbol-level file structure (table of contents) |
| `find_similar_patterns` | Fingerprint Cell (Store) | HDC similarity search |
| `get_index_stats` | All Cells | File/symbol/edge counts, language breakdown, top PageRank |
| `find_references` | Graph Cell (Store) | All usage sites of a symbol |
| `find_implementations` | Graph Cell (Store) | All types implementing a trait/interface |
| `get_callers` | Graph Cell (Store) | Call graph traversal (direct or transitive) |
| `workspace_map` | All Cells | High-level workspace structure (Aider repo-map concept) |
| `get_context` | Assemble Cell (Compose) | Auto-assemble optimal context for a task |

### Tool Input/Output Schemas

**search_code** (primary entry point):
```json
{
    "query": "build dependency graph from source files",
    "strategy": "hybrid",          // keyword|structural|hdc|embedding|hybrid
    "max_results": 10,
    "file_pattern": "crates/roko-index/**",
    "kind_filter": "function"      // optional: filter by symbol kind
}
```

Returns ranked list of symbols with file paths, line numbers, scores, and code snippets.

**get_context** (meta-tool -- auto-assembly):
```json
{
    "task": "Add error handling to the build_graph function",
    "token_budget": 40000,
    "include_tests": false
}
```

Returns a fully assembled context block: ranked code slices, dependency information, and
relevant symbols -- ready to insert into the agent's prompt.

### Configuration

```toml
[agent.mcp_config.servers.code-intelligence]
command = "roko"
args = ["mcp", "code-intelligence"]
env = {}
```

The server registers with the existing MCP passthrough mechanism in `roko.toml`.

### Server Architecture

```rust
pub struct CodeIntelligenceServer {
    index: Arc<CodeIndex>,
    graph: Arc<SymbolGraph>,
    config: ServerConfig,
}

impl CodeIntelligenceServer {
    pub async fn handle_tool_call(
        &self,
        tool_name: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        match tool_name {
            "search_code"          => self.search_code(params).await,
            "get_symbol_context"   => self.get_symbol_context(params).await,
            "get_file_ast"         => self.get_file_ast(params).await,
            "find_similar_patterns"=> self.find_similar(params).await,
            "get_index_stats"      => self.get_stats(params).await,
            "find_references"      => self.find_refs(params).await,
            "find_implementations" => self.find_impls(params).await,
            "get_callers"          => self.get_callers(params).await,
            "workspace_map"        => self.workspace_map(params).await,
            "get_context"          => self.get_context(params).await,
            _ => Err(Error::UnknownTool(tool_name.into())),
        }
    }
}
```

### Security

Input validation on every tool call:
- **File paths**: must be within workspace directory (no path traversal)
- **Query strings**: sanitized for FTS5 SQL injection
- **Token budgets**: capped at maximum to prevent memory exhaustion
- **Result limits**: capped to prevent response explosion
- **Rate limiting**: per-agent sliding window (configurable, default 100/min)

### Index Lifecycle

```
Startup:
    Snapshot exists? -> Load rkyv snapshot -> Ready (~1ms)
    No snapshot? -> SQLite exists? -> Open + incremental update -> Ready (~50ms)
    Nothing? -> Full index build -> Persist -> Ready (~500ms for 5K symbols)

Runtime:
    File change detected (notify watcher, 500ms debounce)
        -> Re-parse changed files (BLAKE3 detects actual changes)
        -> Update graph incrementally
        -> Re-fingerprint changed symbols
        -> Persist updated state

Query:
    Tool call -> Read from shared index (RwLock) -> Compute results -> JSON response
```

Multiple agents can query concurrently via `Arc<CodeIndex>` with internal read/write lock.

---

## SQLite Persistence as a Store Cell

SQLite persistence is a **Store Cell** implementation. See
[02-CELL.md](../../unified/02-CELL.md) for the Store protocol definition.

### Why SQLite

- **Zero administration**: single file at `.roko/index.db`, no server process
- **ACID guarantees**: concurrent reads with serialized writes
- **FTS5**: built-in full-text search with BM25 ranking
- **Embeddable**: links directly via `rusqlite`, no dynamic dependencies
- **WAL mode**: concurrent reads during writes, no blocking

### Schema

```sql
CREATE TABLE files (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    language TEXT NOT NULL,
    content_hash BLOB NOT NULL,      -- BLAKE3(file content), 32 bytes
    size_bytes INTEGER NOT NULL,
    last_indexed INTEGER NOT NULL,
    symbol_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE symbols (
    id INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    visibility TEXT NOT NULL,
    line INTEGER NOT NULL,
    column INTEGER NOT NULL DEFAULT 0,
    end_line INTEGER,
    signature TEXT,
    doc_comment TEXT,
    content_hash BLOB,
    UNIQUE(file_id, name, kind)
);

CREATE VIRTUAL TABLE symbols_fts USING fts5(
    name, doc_comment,
    content='symbols', content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TABLE edges (
    id INTEGER PRIMARY KEY,
    from_symbol_id INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
    to_symbol_id INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    weight REAL NOT NULL DEFAULT 1.0,
    UNIQUE(from_symbol_id, to_symbol_id, kind)
);

CREATE TABLE fingerprints (
    symbol_id INTEGER PRIMARY KEY REFERENCES symbols(id) ON DELETE CASCADE,
    fingerprint BLOB NOT NULL        -- 1,280 bytes (160 x u64)
);

CREATE TABLE file_fingerprints (
    file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    fingerprint BLOB NOT NULL
);

CREATE TABLE imports (
    id INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    alias TEXT,
    kind TEXT NOT NULL
);

CREATE TABLE pagerank (
    symbol_id INTEGER PRIMARY KEY REFERENCES symbols(id) ON DELETE CASCADE,
    score REAL NOT NULL,
    computed_at INTEGER NOT NULL
);

CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Feature-gated:
CREATE TABLE embeddings (             -- behind 'embedding' feature flag
    symbol_id INTEGER PRIMARY KEY REFERENCES symbols(id) ON DELETE CASCADE,
    model TEXT NOT NULL,
    embedding BLOB NOT NULL,          -- 384 x f32 = 1,536 bytes
    computed_at INTEGER NOT NULL
);
```

### BLAKE3 Incremental Updates

Content-based change detection using BLAKE3 hashing:

```
For each file on disk:
    hash = BLAKE3(content)
    if hash matches DB record -> skip (unchanged)
    if hash differs -> re-parse, update symbols/edges/fingerprints
    if new file -> insert everything
Delete files present in DB but not on disk.
Rebuild edges and recompute PageRank if graph changed.
```

BLAKE3 is strictly better than timestamp-based detection:

| Scenario | Timestamp | BLAKE3 |
|---|---|---|
| `git checkout` (new timestamps, same content) | Re-indexes (wrong) | Skips (correct) |
| IDE write-then-rename (stale timestamp) | Skips (wrong) | Re-indexes (correct) |
| Same content across branches | Re-indexes (wrong) | Skips (correct) |

**Incremental update time is proportional to changed files, not total files.** A typical
commit changes 1--5 files -> re-indexing takes < 25ms regardless of workspace size.

### Storage Requirements

| Workspace | Symbols | Without embeddings | With embeddings |
|---|---|---|---|
| Small (Roko) | ~5K | ~9 MB | ~16 MB |
| Medium | ~50K | ~85 MB | ~160 MB |
| Large | ~122K | ~166 MB | ~310 MB |
| Enterprise monorepo | ~500K | ~850 MB | ~1.6 GB |

### Feature-Flag Architecture

```toml
[features]
default = []
sqlite = ["dep:rusqlite", "dep:r2d2", "dep:r2d2_sqlite"]
embedding = ["dep:fastembed", "sqlite"]     # Requires sqlite
snapshot = ["dep:rkyv", "dep:memmap2"]
salsa-memo = ["dep:salsa"]
```

Without any features, `roko-index` works in-memory. Each feature adds a capability with
graceful degradation:

```rust
pub fn load_index(workspace: &Path) -> Box<dyn CodeIndex> {
    #[cfg(feature = "sqlite")]
    if let Ok(db) = open_sqlite_index(workspace) {
        return Box::new(SqliteIndex::new(db));
    }
    #[cfg(feature = "snapshot")]
    if let Ok(snap) = load_snapshot(workspace) {
        return Box::new(SnapshotIndex::new(snap));
    }
    Box::new(InMemoryIndex::new())  // Fallback
}
```

---

## Snapshot Optimization as a Store Cell Variant

Zero-copy snapshots via `rkyv` + `memmap2` provide a 100x speedup on the hot path
(fingerprint comparisons). This is a specialized Store Cell variant optimized for
read-heavy workloads.

### The Cold Start Problem

| Approach | Load time (50K symbols) | Fingerprint compare |
|---|---|---|
| No persistence (rebuild from scratch) | ~2,000ms | ~50ns |
| SQLite (cold) | ~175ms | ~5us (deserialize + compare) |
| SQLite (warm, cached) | ~50ms | ~1us (cached + compare) |
| **rkyv snapshot (mmap)** | **< 1ms** | **~50ns (zero-copy)** |

The snapshot file contains the exact memory layout of Rust data structures. Memory mapping
makes contents directly accessible -- no deserialization, no allocation, no copying.

### Snapshot Format

```
+--------------------------------------------+
| Header (64 bytes)                          |
|   Magic: "ROKO_IDX" (8 bytes)             |
|   Version: u32                             |
|   Symbol count: u64                        |
|   Edge count: u64                          |
|   File count: u64                          |
|   Workspace hash: [u8; 32] (BLAKE3)       |
+--------------------------------------------+
| Fingerprint section                        |
|   Contiguous [u64; 160] x symbol_count     |
|   (1,280 bytes per fingerprint)            |
+--------------------------------------------+
| Symbol metadata (rkyv serialized)          |
+--------------------------------------------+
| Graph adjacency (rkyv serialized)          |
+--------------------------------------------+
| PageRank scores (contiguous f64 array)     |
+--------------------------------------------+
| String table (rkyv, deduped)               |
+--------------------------------------------+
```

Fingerprints are a separate contiguous array (not embedded in metadata) for:
- Bulk comparison with cache line utilization
- SIMD operations (AVX2/AVX-512 for parallel XOR + popcount)
- Partial loading (fingerprints can be mmapped independently)

### Differential Updates

Snapshots are immutable. When files change, overlays capture deltas:

```
Base snapshot (from full build)
    +-- Overlay 1 (3 symbols changed)
    +-- Overlay 2 (1 symbol added)
    +-- Overlay 3 (2 symbols removed)
    +-- Compacted snapshot (merges when overlays > 10% of base)
```

The index reads the base snapshot and applies overlays in order. Compaction merges
everything into a new base snapshot periodically.

### Salsa Memoization (Feature-Gated)

Salsa (rust-analyzer team, inspired by Adapton) provides fine-grained incremental computation:

```rust
#[salsa::query_group(IndexDatabaseStorage)]
trait IndexDatabase {
    #[salsa::input]
    fn file_content(&self, path: String) -> Arc<String>;
    fn parsed_file(&self, path: String) -> Arc<SourceFile>;
    fn symbol_fingerprint(&self, id: SymbolId) -> HdcFingerprint;
    fn file_graph_edges(&self, path: String) -> Vec<SymbolEdge>;
    fn full_graph(&self) -> Arc<SymbolGraph>;
    fn pagerank_scores(&self) -> Arc<HashMap<SymbolId, f64>>;
}
```

When `file_content("graph.rs")` changes, Salsa re-executes `parsed_file("graph.rs")` but
NOT `parsed_file("symbol.rs")` (unchanged input). This cascades through the dependency
chain: only affected computations re-execute.

| Property | Salsa | Snapshot overlays |
|---|---|---|
| Granularity | Per-function memoization | Per-file deltas |
| Best for | Long-running server (LSP-like) | Batch agent sessions |
| Memory | All cached results in-memory | On-disk, demand-paged |

---

## 4-Phase Implementation Roadmap

### Phase 1: Foundation (unblocks agent use) -- 8 days

| Task | File | Days |
|---|---|---|
| Define `CodeIndex` trait | `crates/roko-index/src/lib.rs` | 2 |
| Implement `InMemoryIndex` | `crates/roko-index/src/lib.rs` | 1 |
| Add keyword search | `crates/roko-index/src/lib.rs` | 1 |
| Wire into roko-compose | `crates/roko-compose/src/system_prompt_builder.rs` | 2 |
| Add `roko index` CLI | `crates/roko-cli/src/lib.rs` | 1 |
| Integration test | `crates/roko-cli/tests/` | 1 |

**Deliverable**: `roko index build && roko run "what calls build_graph?"` works end-to-end.

### Phase 2: Persistence -- 10 days

| Task | File | Days |
|---|---|---|
| Add `rusqlite` (feature-gated) | `crates/roko-index/Cargo.toml` | 1 |
| Implement schema + migrations | `crates/roko-index/src/sqlite.rs` | 2 |
| Implement `SqliteIndex` | `crates/roko-index/src/sqlite.rs` | 3 |
| Add BLAKE3 incremental updates | `crates/roko-index/src/sqlite.rs` | 2 |
| Add FTS5 keyword search | `crates/roko-index/src/sqlite.rs` | 1 |
| Benchmark suite | `crates/roko-index/benches/` | 1 |

**Deliverable**: `.roko/index.db` persists; re-indexing after commit < 50ms.

### Phase 3: MCP Server -- 15 days

| Task | File | Days |
|---|---|---|
| MCP stdio server skeleton | `crates/roko-mcp-code/src/lib.rs` | 2 |
| `search_code` tool | `crates/roko-mcp-code/src/lib.rs` | 2 |
| `get_symbol_context` tool | `crates/roko-mcp-code/src/lib.rs` | 1 |
| `get_callers` + `find_references` | `crates/roko-mcp-code/src/lib.rs` | 2 |
| `workspace_map` tool | `crates/roko-mcp-code/src/lib.rs` | 2 |
| `get_context` (auto-assembly) | `crates/roko-mcp-code/src/lib.rs` | 3 |
| Configure in roko.toml | `crates/roko-cli/src/lib.rs` | 1 |
| Integration tests | `crates/roko-mcp-code/tests/` | 2 |

**Deliverable**: Agents call `search_code("build dependency graph")` and get ranked results.

### Phase 4: Accuracy (tree-sitter) -- 14 days

| Task | File | Days |
|---|---|---|
| Add tree-sitter dependency | `crates/roko-lang-rust/Cargo.toml` | 1 |
| `TreeSitterProvider` for Rust | `crates/roko-lang-rust/src/lib.rs` | 3 |
| Extract `Calls` edges | `crates/roko-lang-rust/src/lib.rs` | 2 |
| Extract `Implements` edges | `crates/roko-lang-rust/src/lib.rs` | 2 |
| Extract `Contains` edges | `crates/roko-lang-rust/src/lib.rs` | 1 |
| Update TypeScript provider | `crates/roko-lang-typescript/src/lib.rs` | 2 |
| Update Go provider | `crates/roko-lang-go/src/lib.rs` | 2 |
| Accuracy benchmark | `crates/roko-index/benches/` | 1 |

**Deliverable**: Graph has 4 edge kinds; call graph enables precise impact analysis.

---

## What This Enables

1. **Agent-accessible code intelligence via MCP** -- 10 tools covering search, navigation,
   impact analysis, and context assembly. Agents call tools instead of learning the internal
   API.
2. **Persistent, incremental index** -- index survives restarts, updates in <50ms per commit.
   No cold-start penalty for agents.
3. **Zero-copy fingerprint comparison** -- rkyv snapshots eliminate deserialization overhead,
   enabling ~50ns similarity comparisons even on large indices.
4. **Concurrent multi-agent access** -- WAL mode SQLite and `Arc<RwLock>` enable multiple
   agents to query the same index simultaneously.
5. **Feature-gated progressive enhancement** -- start with in-memory (zero dependencies),
   add SQLite, add snapshots, add embeddings, add Salsa -- each feature flag is independent.

## Feedback Loops

- **MCP tool usage tracking**: the server logs which tools agents call and which produce
  results that lead to gate passes. Underused tools are candidates for deprecation or
  improvement. Heavily-used tools are candidates for optimization.
- **Index freshness monitoring**: if agents query symbols that no longer exist (stale index),
  the freshness metric degrades. The Trigger Cell increases its file-watching frequency
  or reduces debounce timeout.
- **Query latency calibration**: if MCP tool calls consistently exceed the 100ms target,
  the system escalates: in-memory caching, pre-computation, or HNSW index activation.

## Open Questions

1. Should the MCP server be a separate binary or embedded in `roko-cli`? Separate is cleaner
   for process isolation; embedded avoids startup overhead.
2. Should SQLite or snapshots be the primary persistence? SQLite provides query flexibility;
   snapshots provide read performance. The feature-flag architecture supports both, but the
   default matters for UX.
3. Should the `embedding` feature pull a 100MB model on first use, or require explicit
   download? Implicit download simplifies UX but surprises users with network activity.
4. Is Salsa worth the complexity for agent sessions that typically last minutes, not hours?
   The memoization overhead may exceed the savings for short sessions.

## Implementation Tasks

| Task | File paths | Priority |
|---|---|---|
| Implement MCP server skeleton | `crates/roko-mcp-code/src/lib.rs`, `src/main.rs` | Tier 1 |
| Implement all 10 MCP tool handlers | `crates/roko-mcp-code/src/lib.rs` | Tier 1 |
| Add input validation + security | `crates/roko-mcp-code/src/lib.rs` | Tier 1 |
| Add rate limiting | `crates/roko-mcp-code/src/lib.rs` | Tier 1 |
| Implement SQLite schema | `crates/roko-index/src/sqlite.rs` | Tier 1 |
| Implement BLAKE3 incremental updates | `crates/roko-index/src/sqlite.rs` | Tier 1 |
| Implement FTS5 search with code tokenizer | `crates/roko-index/src/sqlite.rs` | Tier 1 |
| Add WAL mode configuration | `crates/roko-index/src/sqlite.rs` | Tier 1 |
| Add schema migration system | `crates/roko-index/src/sqlite.rs` | Tier 1 |
| Add rkyv snapshot writer/reader | `crates/roko-index/src/` (new file) | Tier 2 |
| Add memmap2 fingerprint access | `crates/roko-index/src/` (new file) | Tier 2 |
| Add overlay mechanism for differential updates | `crates/roko-index/src/` (new file) | Tier 2 |
| Add Salsa integration (feature-gated) | `crates/roko-index/Cargo.toml`, new file | Tier 3 |
| Add `roko index build/stats/rebuild` CLI | `crates/roko-cli/src/lib.rs` | Tier 0 |
| Add file watcher for background re-indexing | `crates/roko-mcp-code/src/lib.rs` | Tier 1 |
| Configure MCP server in roko.toml | `crates/roko-cli/src/lib.rs` | Tier 1 |
| Integration tests (agent uses MCP tools) | `crates/roko-mcp-code/tests/` | Tier 1 |
