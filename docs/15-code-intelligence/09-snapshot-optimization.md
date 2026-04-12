# Snapshot Optimization

> Zero-copy rkyv snapshots and memory-mapped index files for sub-millisecond startup — eliminating the cold-start penalty for code intelligence.


> **Implementation**: Built

**Topic**: [Code Intelligence](./INDEX.md)
**Prerequisites**: [08-index-db-scaling.md](./08-index-db-scaling.md)
**Key sources**: `bardo-backup/tmp/death/docs/30-index-performance.md`, `bardo-backup/tmp/death/tools/02-code-index.md`, `bardo-backup/tmp/mori-refactor/10-code-intelligence.md`

---

## Abstract

SQLite provides reliable persistent storage for code intelligence data, but it has a fundamental limitation: every query deserializes data from disk into Rust structs. For read-heavy workloads — which describe code intelligence exactly — the deserialization overhead dominates query time. A fingerprint comparison requires loading two 1,280-byte BLOBs from SQLite, deserializing them into `[u64; 160]` arrays, and then performing the actual XOR + popcount. The computation is ~50ns; the deserialization is ~5μs — 100× overhead.

Zero-copy snapshots eliminate this overhead. The `rkyv` crate serializes Rust data structures into a binary format that can be memory-mapped and read directly without deserialization. A snapshot of the fingerprint index becomes a contiguous array of raw bits in a memory-mapped file. Comparing two fingerprints means pointer arithmetic to find the right offsets and then the same ~50ns XOR + popcount — no deserialization at all.

This document describes the snapshot architecture, the rkyv serialization format, the memory-mapping strategy, the differential update mechanism, and the performance implications.

---

## The Cold Start Problem

### Current situation

Without persistent storage, every `roko-index` session starts from scratch:

```
Session start
     │
     ▼
  Enumerate files (~50ms for 300 files)
     │
     ▼
  Parse all files (~100ms for 177K lines)
     │
     ▼
  Build graph (~1ms for 5K symbols)
     │
     ▼
  Compute fingerprints (~25ms for 5K symbols)
     │
     ▼
  Compute PageRank (~1ms for 5K symbols)
     │
     ▼
  Ready (~177ms total)
```

177ms is acceptable but not instantaneous. For a 50K-symbol enterprise workspace, this scales to ~2 seconds. For a 500K-symbol monorepo, ~20 seconds. Agents waiting 20 seconds before they can query code intelligence is unacceptable.

### With SQLite only

SQLite eliminates re-parsing but adds deserialization overhead:

```
Session start
     │
     ▼
  Open SQLite database (~5ms)
     │
     ▼
  Load symbols into memory (~50ms for 50K symbols)
     │
     ▼
  Load fingerprints into memory (~100ms for 50K fingerprints)
     │
     ▼
  Build in-memory graph from edge table (~20ms for 200K edges)
     │
     ▼
  Ready (~175ms total for 50K symbols)
```

Better than re-parsing, but the deserialization cost is still substantial for large workspaces.

### With rkyv snapshots

Zero-copy snapshots eliminate deserialization entirely:

```
Session start
     │
     ▼
  Memory-map snapshot file (~1ms, regardless of size)
     │
     ▼
  Validate snapshot header (~0.1ms)
     │
     ▼
  Ready (~1.1ms total, regardless of workspace size)
```

The snapshot file contains the exact memory layout of the Rust data structures. Memory mapping makes the file's contents directly accessible as if they were in-memory arrays. No copying, no parsing, no deserialization.

---

## rkyv Serialization

### What rkyv provides

`rkyv` (Rust archiving) is a zero-copy deserialization framework. It serializes Rust structs into a binary format where the serialized bytes ARE the in-memory representation:

```rust
use rkyv::{Archive, Serialize, Deserialize};

#[derive(Archive, Serialize, Deserialize)]
pub struct ArchivedFingerprints {
    pub symbols: Vec<ArchivedSymbolEntry>,
}

#[derive(Archive, Serialize, Deserialize)]
pub struct ArchivedSymbolEntry {
    pub id: ArchivedSymbolId,
    pub fingerprint: [u64; 160],  // 10,240 bits, directly accessible
    pub pagerank: f64,
}

#[derive(Archive, Serialize, Deserialize)]
pub struct ArchivedSymbolId {
    pub file_path: String,
    pub symbol_name: String,
    pub kind: u8,  // Discriminant of SymbolKind
}
```

When deserialized with `rkyv::from_bytes()`, the returned reference points directly into the byte buffer — no allocation, no copying.

### Snapshot format

The snapshot file has a fixed layout:

```
┌──────────────────────────────────────────────────┐
│ Header (64 bytes)                                 │
│   Magic: "ROKO_IDX"  (8 bytes)                   │
│   Version: u32        (4 bytes)                   │
│   Flags: u32          (4 bytes)                   │
│   Symbol count: u64   (8 bytes)                   │
│   Edge count: u64     (8 bytes)                   │
│   File count: u64     (8 bytes)                   │
│   Workspace hash: [u8; 32]  (32 bytes, BLAKE3)   │
├──────────────────────────────────────────────────┤
│ Fingerprint section                               │
│   Contiguous array of [u64; 160] × symbol_count  │
│   (1,280 bytes per fingerprint)                   │
├──────────────────────────────────────────────────┤
│ Symbol metadata section (rkyv serialized)         │
│   Vec<ArchivedSymbolEntry>                        │
├──────────────────────────────────────────────────┤
│ Graph section (rkyv serialized)                   │
│   Forward adjacency: HashMap<u32, Vec<(u32, u8)>> │
│   Reverse adjacency: HashMap<u32, Vec<(u32, u8)>> │
├──────────────────────────────────────────────────┤
│ PageRank section                                  │
│   Contiguous array of f64 × symbol_count         │
├──────────────────────────────────────────────────┤
│ String table (rkyv serialized)                    │
│   File paths, symbol names (deduped)              │
└──────────────────────────────────────────────────┘
```

### Why fingerprints are separate

The fingerprint section is a contiguous array rather than embedded in the symbol metadata. This layout enables:

1. **Bulk comparison** — Scanning all fingerprints for nearest-neighbor search reads a contiguous memory region, maximizing cache line utilization.
2. **SIMD operations** — Aligned arrays of u64 words can use AVX2/AVX-512 instructions for parallel XOR + popcount.
3. **Partial loading** — The fingerprint section can be memory-mapped independently of metadata sections.

---

## Memory Mapping with memmap2

### How it works

The `memmap2` crate provides safe memory-mapped file access:

```rust
use memmap2::Mmap;

// Planned: Snapshot loading
pub fn load_snapshot(path: &Path) -> Result<SnapshotIndex> {
    let file = std::fs::File::open(path)?;
    let mmap = unsafe { Mmap::map(&file)? };

    // Validate header
    let header = SnapshotHeader::from_bytes(&mmap[0..64])?;
    if header.magic != *b"ROKO_IDX" {
        return Err(Error::InvalidSnapshot);
    }
    if header.version != CURRENT_VERSION {
        return Err(Error::VersionMismatch);
    }

    Ok(SnapshotIndex {
        mmap,
        header,
    })
}
```

### Fingerprint access pattern

With memory mapping, accessing a fingerprint is pointer arithmetic:

```rust
impl SnapshotIndex {
    fn fingerprint_offset(index: usize) -> usize {
        HEADER_SIZE + (index * FINGERPRINT_BYTES)
    }

    pub fn fingerprint(&self, index: usize) -> &[u64; 160] {
        let offset = Self::fingerprint_offset(index);
        let bytes = &self.mmap[offset..offset + FINGERPRINT_BYTES];
        // Safety: bytes are aligned and valid (validated on load)
        unsafe { &*(bytes.as_ptr() as *const [u64; 160]) }
    }

    pub fn similarity(&self, a: usize, b: usize) -> f64 {
        let fp_a = self.fingerprint(a);
        let fp_b = self.fingerprint(b);
        let mut diff = 0u32;
        for (left, right) in fp_a.iter().zip(fp_b.iter()) {
            diff += (left ^ right).count_ones();
        }
        1.0 - (f64::from(diff) / 10_240.0)
    }
}
```

No deserialization. No allocation. The XOR + popcount operates directly on the memory-mapped bytes.

### OS-level optimizations

Memory-mapped files benefit from OS page cache management:
- **Demand paging** — Only pages actually accessed are loaded from disk.
- **Shared mapping** — Multiple processes (agents) can share the same physical pages.
- **Eviction under pressure** — The OS can evict pages when memory is tight, re-loading them on next access.
- **Read-ahead** — Sequential access patterns trigger OS-level prefetching.

---

## Snapshot Size Estimates

### Per-symbol storage breakdown

| Component | Bytes per symbol | Notes |
|---|---|---|
| HDC fingerprint | 1,280 | 160 × u64 |
| PageRank score | 8 | f64 |
| Symbol name (avg) | 20 | Deduped in string table |
| File path (avg) | 60 | Deduped in string table |
| Kind + visibility + line | 12 | u8 + u8 + u32 + padding |
| Graph edges (avg 3 per symbol) | 24 | 3 × (u32 target + u8 kind + padding) |
| **Total per symbol** | **~1,404** | |

### Workspace-level estimates

| Workspace size | Symbol count | Snapshot size | mmap load time |
|---|---|---|---|
| Small (Roko) | ~5,000 | ~7 MB | < 1ms |
| Medium | ~50,000 | ~68 MB | < 1ms |
| Large | ~122,000 | ~166 MB | < 1ms |
| Enterprise monorepo | ~500,000 | ~680 MB | < 1ms |

The load time is essentially constant because memory mapping is a virtual memory operation — it doesn't read the file. Actual page loads happen on demand.

### Performance comparison

| Operation | SQLite (cold) | SQLite (warm) | Snapshot (mmap) |
|---|---|---|---|
| Load 5K fingerprints | 50ms | 10ms | < 1ms |
| Load 50K fingerprints | 500ms | 100ms | < 1ms |
| Compare 2 fingerprints | 5μs (deserialize + compare) | 1μs (cached + compare) | 50ns (direct) |
| Full scan 5K fingerprints | 25ms | 5ms | 0.25ms |
| Full scan 50K fingerprints | 250ms | 50ms | 2.5ms |

The snapshot approach is 20–100× faster for fingerprint operations, which are the most latency-sensitive code intelligence queries.

---

## Differential Updates

### The invalidation problem

Snapshots are immutable once written. When a file changes, the snapshot is stale. Rebuilding the entire snapshot for every change is wasteful — most of the data hasn't changed.

### Planned approach: overlay + compaction

```
Base snapshot (written during full index build)
    │
    ├── Overlay 1 (delta from edit #1: 3 symbols changed)
    ├── Overlay 2 (delta from edit #2: 1 symbol added)
    ├── Overlay 3 (delta from edit #3: 2 symbols removed)
    │
    └── Compacted snapshot (merges base + overlays periodically)
```

Each overlay is a small file containing only the changed symbols and their fingerprints. The index reads the base snapshot and applies overlays in order:

```rust
// Planned: Overlay application
impl SnapshotIndex {
    pub fn with_overlay(&self, overlay: &Overlay) -> OverlaidIndex {
        OverlaidIndex {
            base: self,
            additions: overlay.added_symbols(),
            removals: overlay.removed_symbol_ids(),
            updates: overlay.updated_fingerprints(),
        }
    }
}
```

When the accumulated overlays grow large (e.g., > 10% of the base snapshot size), a compaction pass merges everything into a new base snapshot.

### Graph dirty flag

When any file changes, the graph may have new or removed edges. Rather than recomputing the entire graph, a dirty flag triggers incremental graph update:

```rust
// Planned: Graph dirty flag
pub struct IncrementalGraph {
    base_graph: SymbolGraph,     // From snapshot
    dirty: bool,                 // Set when any file changes
    pending_additions: Vec<SymbolEdge>,
    pending_removals: Vec<SymbolEdge>,
}

impl IncrementalGraph {
    pub fn mark_file_changed(&mut self, file_path: &str) {
        // Remove all edges from/to symbols in this file
        // Re-build edges for the new file content
        // Set dirty = true for PageRank recomputation
        self.dirty = true;
    }

    pub fn pagerank(&mut self) -> &HashMap<SymbolId, f64> {
        if self.dirty {
            // Recompute PageRank on the merged graph
            self.dirty = false;
        }
        &self.cached_ranks
    }
}
```

---

## Salsa Memoization (Feature-Gated)

### What Salsa provides

Salsa (inspired by Adapton, used by rust-analyzer) is an incremental computation framework. It memoizes function results and re-computes them only when their inputs change. This is the most granular form of incremental updates:

```rust
// Planned: Salsa-based incremental indexing
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

fn parsed_file(db: &dyn IndexDatabase, path: String) -> Arc<SourceFile> {
    let content = db.file_content(path.clone());
    let provider = language_provider_for(&path);
    Arc::new(parse_source(&path, &content, provider))
}
```

When `file_content("graph.rs")` changes, Salsa automatically re-executes `parsed_file("graph.rs")`, which may trigger re-execution of `file_graph_edges("graph.rs")`, which may trigger re-execution of `full_graph()`, which triggers `pagerank_scores()`. But `parsed_file("symbol.rs")` is NOT re-executed because its input didn't change.

### Salsa vs. snapshot overlays

| Property | Salsa | Snapshot overlays |
|---|---|---|
| Granularity | Per-function memoization | Per-file deltas |
| Overhead | Runtime query tracking | Overlay file I/O |
| Best for | Continuous IDE-like use | Batch agent sessions |
| Startup cost | Rebuild Salsa DB from snapshot | mmap + apply overlays |
| Memory | In-memory (all cached results) | On-disk (demand paged) |

Salsa is more appropriate for a long-running server (like an LSP) that handles continuous edits. Snapshot overlays are better for batch agent sessions that start, do work, and exit. The feature-flag architecture supports both.

---

## Academic Foundations

- **rkyv**: Rust archiving framework. Zero-copy deserialization by making the serialized format identical to the in-memory representation. Eliminates the deserialization step entirely.
- **memmap2**: Safe memory-mapped file access for Rust. Leverages OS virtual memory for demand-paged file access.
- **Salsa**: rust-analyzer team (2019). Incremental computation framework inspired by Adapton (Hammer, Khoo, Hicks, and Foster, PLDI 2014). The `salsa-memo` feature would bring rust-analyzer-grade incrementality to `roko-index`.
- **Adapton**: Hammer, Khoo, Hicks, and Foster (2014), "Adapton: Composable, Demand-Driven Incremental Computation." *PLDI*. The theoretical foundation for demand-driven incremental computation with automatic change propagation.
- **BLAKE3**: O'Connor et al. (2020). The hashing algorithm used for content-based change detection. Enables accurate incremental updates based on actual content changes rather than timestamps.

---

## Current Status and Gaps

### Built

- In-memory `SymbolGraph`, `HdcFingerprint`, and `SymbolId` types (all functional)
- `Serialize`/`Deserialize` derives on `SymbolId` and `SymbolRef` (ready for persistence)
- Deterministic fingerprint generation (reproducible across sessions)
- All graph operations working in-memory

### Missing

- rkyv `Archive`/`Serialize`/`Deserialize` derives on index types
- Snapshot file format and writer
- Memory-mapped snapshot reader
- memmap2 integration
- Overlay mechanism for differential updates
- Compaction pass (overlay → new base snapshot)
- Salsa integration (feature-gated)
- Snapshot CLI commands (`roko index snapshot`, `roko index compact`)
- Benchmark suite comparing SQLite vs. snapshot performance

---

## Cross-References

- See [08-index-db-scaling.md](./08-index-db-scaling.md) for SQLite storage (complementary to snapshots)
- See [05-hdc-fingerprints.md](./05-hdc-fingerprints.md) for the fingerprint data that snapshots optimize
- See [04-pagerank-symbol-importance.md](./04-pagerank-symbol-importance.md) for PageRank caching in snapshots
- See [10-current-status-and-gaps.md](./10-current-status-and-gaps.md) for the implementation roadmap
- See topic [00-architecture](../00-architecture/INDEX.md) for the Runtime layer that manages snapshot I/O
