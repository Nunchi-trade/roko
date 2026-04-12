# Index.db Storage and Scaling

> SQLite-backed persistent index with FTS5, BLAKE3 content hashing, and tiered storage — scaling code intelligence from single-crate projects to enterprise monorepos.


> **Implementation**: Built

**Topic**: [Code Intelligence](./INDEX.md)
**Prerequisites**: [02-symbol-extraction.md](./02-symbol-extraction.md), [03-dependency-graph.md](./03-dependency-graph.md)
**Key sources**: `bardo-backup/tmp/death/tools/02-code-index.md`, `bardo-backup/tmp/death/docs/30-index-performance.md`, `bardo-backup/tmp/mori-refactor/10-code-intelligence.md`

---

## Abstract

The current `roko-index` implementation is entirely in-memory: symbols, graphs, and fingerprints are built on each invocation and discarded when the process exits. This is acceptable for development and testing but inadequate for production use. A workspace with 100K+ symbols takes seconds to re-index from scratch — time that comes directly out of the agent's response latency.

Persistent storage solves this by maintaining the index across invocations. The planned design uses SQLite as the storage engine, providing ACID transactions, full-text search via FTS5, and single-file deployment (no external database server). BLAKE3 content hashing enables true incremental updates: only files whose content actually changed are re-indexed, regardless of modification timestamps.

This document describes the storage schema, the incremental update strategy, the scaling characteristics, and the feature-flag architecture that keeps SQLite optional.

---

## Storage Design Philosophy

### Why SQLite

SQLite is the right choice for a developer-side code intelligence database:

1. **Zero administration** — No server process, no configuration, no network. The index is a single file in the `.roko/` directory.
2. **ACID guarantees** — Concurrent reads with serialized writes. No corruption from crashes during re-indexing.
3. **FTS5** — Built-in full-text search with tokenizers that handle camelCase and snake_case.
4. **Performance** — Single-file databases up to 140 TB. Read throughput exceeds what code intelligence needs by orders of magnitude.
5. **Embeddable** — Links directly into the Rust binary via `rusqlite`. No dynamic dependencies.
6. **Mature** — The most deployed database engine in the world. Battle-tested across billions of installations.

### File location

The index database lives at:

```
.roko/index.db
```

This places it alongside other Roko state files (`signals.jsonl`, `episodes.jsonl`, `state/executor.json`) in the project's `.roko/` directory. The file is excluded from version control via `.gitignore`.

---

## Database Schema

### Core tables

```sql
-- Files tracked by the index
CREATE TABLE files (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    language TEXT NOT NULL,
    content_hash BLOB NOT NULL,    -- BLAKE3 hash of file content
    size_bytes INTEGER NOT NULL,
    last_indexed INTEGER NOT NULL,  -- Unix timestamp (ms)
    symbol_count INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_files_path ON files(path);
CREATE INDEX idx_files_hash ON files(content_hash);

-- Symbol definitions
CREATE TABLE symbols (
    id INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,             -- 'function', 'struct', 'trait', etc.
    visibility TEXT NOT NULL,       -- 'public', 'private', 'crate'
    line INTEGER NOT NULL,
    column INTEGER NOT NULL DEFAULT 0,
    end_line INTEGER,
    signature TEXT,                 -- Function signature or type definition
    doc_comment TEXT,               -- Associated doc comment
    content_hash BLOB,             -- BLAKE3 of the symbol's source text
    UNIQUE(file_id, name, kind)    -- Same (file, name, kind) = same symbol
);

CREATE INDEX idx_symbols_name ON symbols(name);
CREATE INDEX idx_symbols_kind ON symbols(kind);
CREATE INDEX idx_symbols_file ON symbols(file_id);

-- Full-text search over symbol names and doc comments
CREATE VIRTUAL TABLE symbols_fts USING fts5(
    name,
    doc_comment,
    content='symbols',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

-- Dependency edges
CREATE TABLE edges (
    id INTEGER PRIMARY KEY,
    from_symbol_id INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
    to_symbol_id INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,             -- 'imports', 'calls', 'implements', 'contains'
    weight REAL NOT NULL DEFAULT 1.0,
    UNIQUE(from_symbol_id, to_symbol_id, kind)
);

CREATE INDEX idx_edges_from ON edges(from_symbol_id);
CREATE INDEX idx_edges_to ON edges(to_symbol_id);
CREATE INDEX idx_edges_kind ON edges(kind);

-- HDC fingerprints
CREATE TABLE fingerprints (
    symbol_id INTEGER PRIMARY KEY REFERENCES symbols(id) ON DELETE CASCADE,
    fingerprint BLOB NOT NULL       -- 1,280 bytes (160 × u64, little-endian)
);

-- File-level fingerprints
CREATE TABLE file_fingerprints (
    file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    fingerprint BLOB NOT NULL
);

-- Import statements (for graph construction)
CREATE TABLE imports (
    id INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    alias TEXT,
    kind TEXT NOT NULL              -- 'use', 'require', 'import', 'type_only'
);

CREATE INDEX idx_imports_file ON imports(file_id);

-- PageRank scores (cached, recomputed on graph changes)
CREATE TABLE pagerank (
    symbol_id INTEGER PRIMARY KEY REFERENCES symbols(id) ON DELETE CASCADE,
    score REAL NOT NULL,
    computed_at INTEGER NOT NULL    -- Unix timestamp
);
```

### Metadata table

```sql
-- Index metadata for schema versioning and stats
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Initial metadata
INSERT INTO meta VALUES ('schema_version', '1');
INSERT INTO meta VALUES ('created_at', strftime('%s', 'now'));
INSERT INTO meta VALUES ('roko_version', '0.1.0');
```

### Embedding table (optional, feature-gated)

```sql
-- Dense embeddings (optional, behind 'embedding' feature flag)
CREATE TABLE embeddings (
    symbol_id INTEGER PRIMARY KEY REFERENCES symbols(id) ON DELETE CASCADE,
    model TEXT NOT NULL,             -- e.g., 'bge-small-en-v1.5'
    embedding BLOB NOT NULL,         -- 384 × f32 = 1,536 bytes
    computed_at INTEGER NOT NULL
);
```

---

## Incremental Update Strategy

### Content-based change detection

The incremental update algorithm uses BLAKE3 content hashing to detect actual changes:

```
1. Enumerate workspace files by extension
2. For each file:
   a. Compute BLAKE3(content)
   b. Look up file in `files` table by path
   c. If hash matches → skip (no change)
   d. If hash differs → re-parse, update symbols, edges, fingerprints
   e. If new file → insert everything fresh
3. Delete removed files (present in DB but not on disk)
4. Recompute PageRank if graph changed
5. Update `meta.last_indexed`
```

This is strictly more accurate than timestamp-based detection:
- Files touched by `git checkout` get new timestamps but identical content → skipped
- Files modified by IDEs that write-then-rename may have stale timestamps → caught
- Files with identical content across branches → skipped on branch switch

### Batch operations

For efficiency, the update runs within a single SQLite transaction:

```rust
// Planned: Incremental update
pub fn update_index(
    db: &Connection,
    workspace_root: &Path,
    providers: &[Box<dyn LanguageProvider>],
) -> Result<UpdateStats> {
    let tx = db.transaction()?;

    let mut stats = UpdateStats::default();

    // Phase 1: Detect changed files
    let disk_files = enumerate_source_files(workspace_root, providers)?;
    let db_files = get_indexed_files(&tx)?;

    for file_info in &disk_files {
        let content = std::fs::read_to_string(&file_info.path)?;
        let hash = blake3::hash(content.as_bytes());

        match db_files.get(&file_info.path) {
            Some(db_entry) if db_entry.content_hash == hash.as_bytes() => {
                stats.unchanged += 1;
                continue; // No change
            }
            Some(db_entry) => {
                // Changed: re-index
                delete_file_data(&tx, db_entry.id)?;
                insert_file_data(&tx, file_info, &content, &hash)?;
                stats.updated += 1;
            }
            None => {
                // New file: insert
                insert_file_data(&tx, file_info, &content, &hash)?;
                stats.added += 1;
            }
        }
    }

    // Phase 2: Remove deleted files
    let disk_paths: HashSet<_> = disk_files.iter().map(|f| &f.path).collect();
    for (path, entry) in &db_files {
        if !disk_paths.contains(path) {
            delete_file_data(&tx, entry.id)?;
            stats.deleted += 1;
        }
    }

    // Phase 3: Rebuild graph edges if any files changed
    if stats.added + stats.updated + stats.deleted > 0 {
        rebuild_edges(&tx)?;
        recompute_pagerank(&tx)?;
    }

    tx.commit()?;
    Ok(stats)
}
```

### Expected performance

| Operation | 100 files | 1K files | 10K files | 100K files |
|---|---|---|---|---|
| Full index build | 50ms | 500ms | 5s | 50s |
| Incremental (1 file changed) | 5ms | 5ms | 5ms | 5ms |
| Incremental (10 files changed) | 50ms | 50ms | 50ms | 50ms |
| BLAKE3 hashing (all files) | 10ms | 100ms | 1s | 10s |
| PageRank recomputation | <1ms | 1ms | 10ms | 100ms |
| FTS5 query | <1ms | <1ms | 1ms | 5ms |

The critical insight: incremental update time is proportional to the number of *changed* files, not the total number of files. A typical development commit changes 1–5 files, so re-indexing takes < 25ms regardless of workspace size.

---

## Scaling Characteristics

### Storage requirements

| Metric | Per symbol | 5K symbols | 50K symbols | 500K symbols |
|---|---|---|---|---|
| Symbol record | ~200 bytes | 1 MB | 10 MB | 100 MB |
| HDC fingerprint | 1,280 bytes | 6.25 MB | 62.5 MB | 625 MB |
| Dense embedding | 1,536 bytes | 7.5 MB | 75 MB | 750 MB |
| Edges (avg 3 per symbol) | ~50 bytes | 750 KB | 7.5 MB | 75 MB |
| FTS5 index | ~100 bytes | 500 KB | 5 MB | 50 MB |
| **Total (without embeddings)** | | **~9 MB** | **~85 MB** | **~850 MB** |
| **Total (with embeddings)** | | **~16 MB** | **~160 MB** | **~1.6 GB** |

For the Roko workspace (~5K symbols), the index is ~9 MB without embeddings. This fits comfortably on any development machine.

### Query performance

SQLite's query performance is more than sufficient for code intelligence workloads:

| Query type | Expected latency | Notes |
|---|---|---|
| Symbol by name (indexed) | < 0.1ms | B-tree lookup |
| Symbol by kind (indexed) | < 1ms | Index scan |
| FTS5 search | < 5ms | Full-text search with ranking |
| Forward/reverse edge lookup | < 0.1ms | Index on from/to |
| Fingerprint comparison (brute force, 5K) | ~0.25ms | All fingerprints in ~6MB |
| Fingerprint comparison (brute force, 50K) | ~2.5ms | May need HNSW for larger |

### Concurrent access

SQLite supports concurrent readers with one writer at a time (WAL mode). This matches the code intelligence access pattern:
- **Multiple agents** can query the index concurrently (reads)
- **One re-indexer** updates the index when files change (writes)
- Reads are not blocked by writes in WAL mode

---

## Feature-Flag Architecture

### Optional dependencies

The storage layer uses Cargo feature flags to keep heavy dependencies optional:

```toml
# Planned: Cargo.toml feature flags
[features]
default = []
sqlite = ["rusqlite"]                    # SQLite persistent storage
embedding = ["fastembed", "sqlite"]      # Dense embeddings (requires SQLite)
snapshot = ["rkyv", "memmap2"]           # Zero-copy snapshots
salsa-memo = ["salsa"]                   # Incremental computation caching
```

Without any features enabled, `roko-index` works entirely in-memory using the current four modules. Each feature adds a capability:

| Feature | Dependency | What it enables |
|---|---|---|
| `sqlite` | `rusqlite` | Persistent index in `.roko/index.db` |
| `embedding` | `fastembed` | Dense embeddings for semantic search |
| `snapshot` | `rkyv`, `memmap2` | Zero-copy index snapshots for fast startup |
| `salsa-memo` | `salsa` | Incremental computation (Salsa framework) |

### Graceful degradation

When features are disabled, the system degrades gracefully:

```rust
// Planned: Feature-gated storage
pub fn load_index(workspace: &Path) -> Box<dyn CodeIndex> {
    #[cfg(feature = "sqlite")]
    if let Ok(db) = open_sqlite_index(workspace) {
        return Box::new(SqliteIndex::new(db));
    }

    #[cfg(feature = "snapshot")]
    if let Ok(snap) = load_snapshot(workspace) {
        return Box::new(SnapshotIndex::new(snap));
    }

    // Fallback: in-memory index
    Box::new(InMemoryIndex::new())
}
```

---

## FTS5 Search Design

### Tokenization for code

Code identifiers follow different conventions than natural language. The FTS5 tokenizer must handle:

| Convention | Example | Tokens |
|---|---|---|
| snake_case | `process_input` | `process`, `input` |
| camelCase | `processInput` | `process`, `input` |
| PascalCase | `ProcessInput` | `process`, `input` |
| SCREAMING_SNAKE | `MAX_BUFFER_SIZE` | `max`, `buffer`, `size` |
| Acronyms | `HTTPClient` | `http`, `client` |

The `unicode61` tokenizer with custom separators handles snake_case and path separators. For camelCase splitting, a custom tokenizer or pre-processing step splits identifiers before insertion:

```rust
// Planned: CamelCase splitter for FTS5
fn split_identifier(name: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in name.chars() {
        if ch == '_' || ch == '-' || ch == '.' || ch == '/' || ch == ':' {
            if !current.is_empty() {
                tokens.push(current.drain(..).collect());
            }
        } else if ch.is_uppercase() && !current.is_empty()
            && current.chars().last().map_or(false, |c| c.is_lowercase())
        {
            tokens.push(current.drain(..).collect());
            current.push(ch.to_lowercase().next().unwrap_or(ch));
        } else {
            current.push(ch.to_lowercase().next().unwrap_or(ch));
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}
```

### Search examples

```sql
-- Find symbols named "process" or starting with "process"
SELECT s.* FROM symbols s
JOIN symbols_fts ON symbols_fts.rowid = s.id
WHERE symbols_fts MATCH 'process*'
ORDER BY rank;

-- Find symbols related to "graph" with "build" in the name
SELECT s.* FROM symbols s
JOIN symbols_fts ON symbols_fts.rowid = s.id
WHERE symbols_fts MATCH 'graph build'
ORDER BY rank
LIMIT 10;
```

---

## Migration and Versioning

### Schema migrations

The `meta` table tracks the schema version. On startup, the index checks the version and runs migrations if needed:

```rust
// Planned: Schema migration
fn migrate(db: &Connection) -> Result<()> {
    let version: i64 = db.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [], |row| row.get(0),
    )?;

    if version < 2 {
        db.execute_batch(include_str!("migrations/002_add_embeddings.sql"))?;
    }
    if version < 3 {
        db.execute_batch(include_str!("migrations/003_add_edge_weight.sql"))?;
    }

    db.execute(
        "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
        [CURRENT_SCHEMA_VERSION],
    )?;

    Ok(())
}
```

### Index rebuild triggers

The index is rebuilt from scratch when:
- Schema version changes incompatibly
- The workspace root changes
- The user explicitly requests it (`roko index rebuild`)
- The index file is corrupted or missing

---

## Academic Foundations

- **SQLite**: Hipp (2000). The storage engine. Chosen for its zero-configuration deployment, ACID guarantees, and FTS5 full-text search.
- **BLAKE3**: O'Connor et al. (2020). The content hashing algorithm for change detection. 10× faster than SHA-256, tree-hashable for parallel computation.
- **FTS5**: SQLite Extension. Full-text search with BM25 ranking, custom tokenizers, and incremental indexing. Used for keyword search over symbol names and documentation.
- **Salsa**: rust-analyzer team (2019). Incremental computation framework inspired by Adapton. The planned `salsa-memo` feature would memoize parse and graph results for fine-grained re-computation. Powers rust-analyzer's incremental analysis.

---

## Current Status and Gaps

### Built

- In-memory symbol, graph, and fingerprint storage (functional)
- `SymbolId` with `Serialize`/`Deserialize` support (ready for persistence)
- `SymbolRef` with serialization support
- Graph construction from in-memory data

### Missing

- SQLite database creation and schema
- `rusqlite` integration (no dependency yet)
- BLAKE3 content hashing for change detection
- Incremental update logic
- FTS5 search interface
- Feature-flag architecture (`sqlite`, `embedding`, `snapshot`, `salsa-memo`)
- Schema migration system
- Index CLI commands (`roko index build`, `roko index stats`, `roko index rebuild`)
- Concurrent access management (WAL mode configuration)

---

## Cross-References

- See [09-snapshot-optimization.md](./09-snapshot-optimization.md) for rkyv zero-copy snapshots as a complement to SQLite
- See [05-hdc-fingerprints.md](./05-hdc-fingerprints.md) for fingerprint storage requirements
- See [06-context-assembly-from-code.md](./06-context-assembly-from-code.md) for search queries that hit the database
- See [07-mcp-context-server.md](./07-mcp-context-server.md) for the server that queries the index
- See topic [00-architecture](../00-architecture/INDEX.md) for the Substrate trait that persistence implements
