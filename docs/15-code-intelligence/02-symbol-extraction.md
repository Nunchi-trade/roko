# Symbol Extraction

> Extracting structured symbol definitions from source code — the atomic data that feeds dependency graphs, fingerprints, and search.


> **Implementation**: Built

**Topic**: [Code Intelligence](./INDEX.md)
**Prerequisites**: [01-tree-sitter-parsing.md](./01-tree-sitter-parsing.md)
**Key sources**: `crates/roko-index/src/symbol.rs`, `crates/roko-core/src/language.rs`, `bardo-backup/tmp/mori-refactor/10-code-intelligence.md`, `bardo-backup/tmp/death/tools/02-code-index.md`

---

## Abstract

Symbol extraction is the process of turning raw source text into structured data: named definitions with their kinds, visibility, locations, and relationships. Every downstream capability in `roko-index` depends on the quality and completeness of symbol extraction. The dependency graph is only as good as its nodes. HDC fingerprints encode symbol properties. Search returns symbols. Context assembly prioritizes symbols.

The `roko-index` crate defines two key types for symbol identity: `SymbolId` (unique identifier) and `SymbolRef` (usage site reference). These work alongside the `Symbol` and `SymbolKind` types from `roko-core` to form a complete system for identifying, locating, and looking up code definitions across a multi-file workspace.

This document covers the symbol type system, the extraction process, the identification scheme, and the planned enrichments that tree-sitter will enable.

---

## The Symbol Type System

### Core types from `roko-core`

The foundational types live in `roko_core::language` and are re-exported by `roko-index`:

```rust
/// A symbol (definition) extracted from source code.
pub struct Symbol {
    /// The symbol's name (e.g., "process_input", "Config", "SymbolKind").
    pub name: String,
    /// What kind of definition this is.
    pub kind: SymbolKind,
    /// Visibility level.
    pub visibility: Visibility,
    /// 1-based line number where the symbol is defined.
    pub line: usize,
}
```

The `SymbolKind` enum classifies symbols into eight categories:

```rust
#[non_exhaustive]
pub enum SymbolKind {
    Function,   // fn, async fn, const fn, unsafe fn (Rust); function (TS); func (Go)
    Struct,     // struct (Rust); class (TS, mapped); type struct (Go)
    Enum,       // enum (Rust, TS); not directly in Go
    Trait,      // trait (Rust); interface (TS, mapped); type interface (Go)
    Const,      // const (Rust, TS, Go); var (Go)
    Type,       // type alias (Rust, TS); type (Go, non-struct/interface)
    Module,     // mod (Rust); namespace (TS); package (Go)
    Impl,       // impl (Rust only)
}
```

The `#[non_exhaustive]` attribute is intentional — new symbol kinds can be added without breaking downstream code. This matters as language support expands: Python classes, Java annotations, C++ templates, and other constructs may need new kinds.

Visibility is tracked at three levels:

```rust
pub enum Visibility {
    Public,     // pub (Rust); export (TS); capitalized name (Go)
    Private,    // no modifier (Rust, Go lowercase); no export (TS)
    Crate,      // pub(crate) (Rust); internal (Go package-level)
}
```

### Cross-language mapping conventions

The type system must accommodate different language paradigms:

| Language construct | Mapped SymbolKind | Rationale |
|---|---|---|
| Rust `fn` | `Function` | Direct |
| Rust `struct` | `Struct` | Direct |
| Rust `trait` | `Trait` | Direct |
| Rust `impl` | `Impl` | Unique to Rust; captures implementation blocks |
| TypeScript `class` | `Struct` | Classes are the TS equivalent of structs (data + methods) |
| TypeScript `interface` | `Trait` | Interfaces define contracts like traits |
| TypeScript `function` | `Function` | Direct |
| Go `func` | `Function` | Direct; methods also map here |
| Go `type X struct` | `Struct` | Direct |
| Go `type X interface` | `Trait` | Interfaces map to traits |
| Go uppercase name | `Visibility::Public` | Go's capitalization convention |

This mapping means the graph and fingerprint layers can treat symbols uniformly regardless of source language. A Rust `trait` and a Go `interface` both produce `SymbolKind::Trait` nodes in the dependency graph, enabling cross-language structural comparison via HDC fingerprints.

---

## Symbol Identification: SymbolId

### The unique key

`SymbolId` provides a unique identifier for a symbol within an index. It combines three components:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SymbolId {
    /// Path of the file that defines the symbol.
    pub file_path: String,
    /// Symbol name.
    pub symbol_name: String,
    /// Symbol kind.
    pub kind: SymbolKind,
}
```

The triple `(file_path, symbol_name, kind)` uniquely identifies a symbol because:

1. **Two symbols with the same name but different kinds** are distinct — a `struct Config` and a `fn Config` (constructor pattern) in the same file are different symbols.
2. **Two symbols with the same name and kind in different files** are distinct — `fn process` in `handler.rs` and `fn process` in `worker.rs` are different symbols.
3. **Two symbols with identical (file, name, kind)** are the same definition — this handles the case where a file is re-indexed and the same symbol is re-extracted.

### Construction

`SymbolId` can be constructed directly or derived from a `Symbol`:

```rust
impl SymbolId {
    /// Build from components.
    pub fn new(
        file_path: impl Into<String>,
        symbol_name: impl Into<String>,
        kind: SymbolKind,
    ) -> Self { ... }

    /// Derive from a Symbol and its file path.
    pub fn from_symbol(symbol: &Symbol, file_path: &str) -> Self {
        Self {
            file_path: file_path.to_string(),
            symbol_name: symbol.name.clone(),
            kind: symbol.kind.clone(),
        }
    }
}
```

### Display format

The `Display` implementation produces a human-readable form: `file_path::symbol_name(Kind)`. For example: `handler.rs::process(Function)`. This format is used in debugging, graph visualization, and error messages.

### Hash-based identity

`SymbolId` derives `Hash`, making it suitable as a key in `HashMap` and `HashSet`. This is critical for the `SymbolGraph` implementation, which uses `HashSet<SymbolId>` for nodes and `HashMap<SymbolId, Vec<...>>` for adjacency lists.

---

## Symbol References: SymbolRef

### Usage site tracking

While `SymbolId` identifies where a symbol is *defined*, `SymbolRef` identifies where a symbol is *used*:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SymbolRef {
    /// File containing the reference.
    pub file: String,
    /// 1-based line number.
    pub line: usize,
    /// 0-based column offset.
    pub column: usize,
}
```

`SymbolRef` tracks three dimensions of location:
- **File** — which file contains the usage
- **Line** — which line (1-based, matching editor conventions)
- **Column** — which column (0-based, matching LSP conventions)

### Current limitations

The heuristic parsers do not currently produce `SymbolRef` instances because they don't track usage sites — only definitions. Tree-sitter integration will enable reference tracking by traversing `identifier` and `call_expression` nodes in the AST.

When reference tracking is implemented, it enables:
- **Find all references** — Given a `SymbolId`, find all `SymbolRef` instances that refer to it
- **Impact analysis** — Given a proposed change to a symbol, enumerate every usage site affected
- **Dead code detection** — Symbols with zero references (outside their definition file) are candidates for removal

---

## Symbol Lookup

### The find_symbol function

The `find_symbol()` utility provides name-based lookup across parsed files:

```rust
pub fn find_symbol<'a>(files: &'a [SourceFile], name: &str) -> Vec<&'a Symbol> {
    files
        .iter()
        .flat_map(|f| f.symbols.iter())
        .filter(|s| s.name == name)
        .collect()
}
```

This function returns all symbols matching the given name across all files. It is intentionally simple — a linear scan — because:

1. **Symbol counts are manageable** — Even a ~322K-line workspace produces ~5,000–10,000 top-level symbols. Linear scan over 10K items is sub-millisecond.

2. **Name collisions are informative** — When multiple symbols share a name, the caller (typically the graph builder) can distinguish them by kind and file path.

3. **The function is a building block** — More sophisticated lookup (by kind, by file, by visibility, by regex) can be composed from this primitive and the existing data structures.

### Planned enhancements

For persistent storage (see [08-index-db-scaling.md](./08-index-db-scaling.md)), symbol lookup will be backed by SQLite with FTS5:

```sql
-- Planned schema
CREATE TABLE symbols (
    id INTEGER PRIMARY KEY,
    file_path TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    visibility TEXT NOT NULL,
    line INTEGER NOT NULL,
    column INTEGER DEFAULT 0,
    content_hash BLOB NOT NULL  -- BLAKE3 hash for incremental updates
);

CREATE INDEX idx_symbols_name ON symbols(name);
CREATE VIRTUAL TABLE symbols_fts USING fts5(name, file_path, content='symbols');
```

This enables:
- **Prefix search** — Find all symbols starting with "process_"
- **Fuzzy search** — FTS5 tokenization handles camelCase and snake_case splitting
- **Indexed lookup by kind** — Efficiently find all traits, all functions, etc.
- **Content-addressed updates** — Only re-index symbols whose file's BLAKE3 hash changed

---

## The Extraction Pipeline

### End-to-end flow

```
  Source file (path + content)
        │
        ▼
  LanguageProvider.extract_symbols()
        │
        ▼
  Vec<Symbol>  ──→  SourceFile { symbols, imports, ... }
        │                              │
        ▼                              ▼
  SymbolId::from_symbol()     build_graph() uses symbols as nodes
        │                              │
        ▼                              ▼
  Graph nodes (HashSet<SymbolId>)     Import edges (by name matching)
        │
        ▼
  fingerprint_symbol() ──→ HdcFingerprint (10,240 bits)
```

Each step enriches the raw data:

1. **Extraction** — `LanguageProvider` turns text into `Vec<Symbol>` and `Vec<Import>`.
2. **Packaging** — `parse_source()` bundles symbols and imports into a `SourceFile`.
3. **Identification** — `SymbolId::from_symbol()` creates unique keys for graph nodes.
4. **Graphing** — `build_graph()` registers symbols as nodes and creates edges from imports.
5. **Fingerprinting** — `fingerprint_symbol()` encodes each symbol into a 10,240-bit HDC vector.

### What each language provider extracts

| Construct | Rust | TypeScript | Go |
|---|---|---|---|
| Functions | `fn`, `async fn`, `unsafe fn`, `const fn` | `function`, `export function`, `async function` | `func`, `func (r T) method` |
| Structs | `struct Name` | `class Name` | `type Name struct` |
| Enums | `enum Name` | `enum Name` | — |
| Traits/Interfaces | `trait Name` | `interface Name` | `type Name interface` |
| Constants | `const NAME` | `const NAME`, `export const` | `const Name`, `var Name` |
| Type aliases | `type Name` | `type Name =` | `type Name` (non-struct/interface) |
| Modules | `mod name` | — | — |
| Impl blocks | `impl Name`, `impl Trait for Type` | — | — |
| Use imports | `use path::Item` | `import { X } from "path"` | `import "path"` |
| Require imports | — | `require("path")` | — |
| Module imports | `mod name;` | — | — |
| Type-only imports | — | `import type { X }` | — |

---

## Planned Enrichments

### Rich symbol metadata (tree-sitter enabled)

With tree-sitter, symbols can carry much more information:

```rust
// Planned: Enhanced Symbol type
pub struct RichSymbol {
    pub base: Symbol,           // Current fields
    pub byte_range: Range<usize>,  // Exact byte range in source
    pub column: usize,          // Column offset
    pub end_line: usize,        // End line (for multi-line definitions)
    pub signature: Option<String>,  // Function signature text
    pub doc_comment: Option<String>, // Associated doc comment
    pub parent: Option<SymbolId>,    // Containing symbol (e.g., impl block)
    pub generic_params: Vec<String>, // Generic type parameters
    pub annotations: Vec<String>,    // Attributes / decorators
}
```

### Scope-aware nesting

Currently, all symbols are extracted as a flat list. Tree-sitter enables hierarchical extraction:

```rust
// Planned: Symbol tree
pub struct SymbolTree {
    pub symbol: Symbol,
    pub children: Vec<SymbolTree>,  // Nested definitions
}

// Example: impl block containing methods
// SymbolTree {
//     symbol: Symbol { name: "Config", kind: Impl },
//     children: [
//         SymbolTree { symbol: Symbol { name: "new", kind: Function }, children: [] },
//         SymbolTree { symbol: Symbol { name: "validate", kind: Function }, children: [] },
//     ]
// }
```

This nesting information is critical for:
- **Impl-level context** — When working on a method, include the entire impl block's context
- **Module hierarchy** — Understand which symbols are children of which modules
- **Scope resolution** — Distinguish between two `new()` functions in different impl blocks

### Content hashing for incremental updates

Each symbol's surrounding content can be hashed with BLAKE3 to enable fine-grained change detection:

```rust
// Planned: Content-addressed symbol
pub struct IndexedSymbol {
    pub id: SymbolId,
    pub symbol: Symbol,
    pub content_hash: [u8; 32],  // BLAKE3 of the symbol's source text
    pub fingerprint: HdcFingerprint,
    pub last_indexed: u64,  // Unix timestamp
}
```

When re-indexing, only symbols whose `content_hash` changed need to be re-fingerprinted and re-inserted into the graph. This enables true incremental indexing at the symbol level, not just the file level.

---

## Academic Foundations

- **Principles of Program Analysis**: Nielson, Nielson, and Hankin (1999). The theoretical basis for extracting structural information from programs — what constitutes a "definition," how scoping works, what "uses" mean.
- **Program slicing**: Weiser (1981), "Program Slicing." *ICSE*. The concept of identifying the minimal set of program elements relevant to a computation. Symbol extraction is the first step: you can't slice what you can't identify.
- **Language Server Protocol**: Microsoft (2016). LSP defines a standard for symbol information exchange between editors and language servers. Roko's `Symbol` type mirrors LSP's `SymbolInformation` and `DocumentSymbol` types, ensuring conceptual compatibility.
- **code2vec**: Alon, Zilberstein, Levy, and Brody (2019), "code2vec: Learning Distributed Representations of Code." *POPL*. Demonstrated that AST path-based features capture meaningful code properties. The planned tree-sitter integration will enable similar path-based features.

---

## Current Status and Gaps

### Built

- `Symbol`, `SymbolKind`, `Visibility` types in `roko-core` — stable, well-tested
- `SymbolId` with hash-based identity — used as graph node keys
- `SymbolRef` for usage site tracking — defined but not yet populated by parsers
- `find_symbol()` name-based lookup — functional linear scan
- Symbol extraction in all three language providers — handles common constructs
- Serialization support (`Serialize`, `Deserialize`) on `SymbolId` and `SymbolRef`

### Missing

- Rich symbol metadata (signatures, doc comments, byte ranges, column offsets)
- Scope-aware nesting (flat list only, no parent-child relationships)
- Content hashing per symbol for incremental updates
- `SymbolRef` population from usage site analysis
- SQLite-backed persistent symbol storage with FTS5
- Cross-language symbol resolution (e.g., TypeScript importing a Rust WASM module)
- Regex or glob-based symbol search

---

## Cross-References

- See [01-tree-sitter-parsing.md](./01-tree-sitter-parsing.md) for how symbols are extracted from source code
- See [03-dependency-graph.md](./03-dependency-graph.md) for how symbols become graph nodes
- See [05-hdc-fingerprints.md](./05-hdc-fingerprints.md) for how symbols are encoded into fingerprint vectors
- See [08-index-db-scaling.md](./08-index-db-scaling.md) for persistent symbol storage design
- See topic [00-architecture](../00-architecture/INDEX.md) for the `roko-core` type definitions
