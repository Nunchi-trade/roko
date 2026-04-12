# Tree-Sitter Incremental Parsing

> Language-agnostic, incremental AST parsing via tree-sitter grammars — the planned upgrade from heuristic line-by-line parsers to full structural analysis.

**Topic**: [Code Intelligence](./INDEX.md)
**Prerequisites**: [00-vision.md](./00-vision.md)
**Key sources**: `bardo-backup/tmp/mori-refactor/10-code-intelligence.md`, `bardo-backup/tmp/death/tools/02-code-index.md`, `crates/roko-index/src/parser.rs`, `crates/roko-lang-rust/src/lib.rs`

---

## Abstract

Parsing source code is the foundation of everything code intelligence does. Without accurate parsing, symbol extraction misses definitions, dependency graphs contain phantom edges, and HDC fingerprints encode noise rather than structure. The quality of every downstream capability is bounded by the quality of the parser.

Roko's current language providers (`roko-lang-rust`, `roko-lang-typescript`, `roko-lang-go`) use line-by-line heuristic parsers that scan source text for patterns like `fn `, `struct `, `pub enum`, `function `, `class `, `type `, and `func `. These heuristics work surprisingly well for common cases — they correctly extract top-level definitions, handle visibility modifiers, and parse import statements across three languages in under 2500 lines of code combined.

However, heuristic parsers have fundamental limitations. They cannot handle nested definitions, multi-line signatures, macro-generated code, conditional compilation, or the myriad edge cases that make real-world source code messy. Tree-sitter (Brunsfeld 2018) solves these problems by providing full AST parsing with incremental update support, error recovery, and consistent performance characteristics across 100+ language grammars.

This document describes the current parsing architecture, its limitations, and the planned migration to tree-sitter.

---

## Current Architecture: The LanguageProvider Trait

### The trait contract

All parsing in `roko-index` goes through a single trait defined in `roko-core`:

```rust
pub trait LanguageProvider: Send + Sync {
    /// Human-readable language name (e.g., "rust", "typescript", "go").
    fn language_name(&self) -> &str;

    /// File extensions this provider handles (e.g., &["rs"], &["ts", "tsx"]).
    fn file_extensions(&self) -> &[&str];

    /// Extract import statements from source text.
    fn parse_imports(&self, source: &str) -> Vec<Import>;

    /// Extract symbol definitions from source text.
    fn extract_symbols(&self, source: &str) -> Vec<Symbol>;
}
```

This trait is language-agnostic at the `roko-index` level. The `parse_source()` function in `roko-index/src/parser.rs` delegates entirely to the provider:

```rust
pub fn parse_source(
    path: &str,
    content: &str,
    provider: &dyn LanguageProvider,
) -> SourceFile {
    let symbols = provider.extract_symbols(content);
    let imports = provider.parse_imports(content);

    SourceFile {
        path: path.to_string(),
        language: provider.language_name().to_string(),
        content: content.to_string(),
        symbols,
        imports,
    }
}
```

The `SourceFile` struct captures the parsing output:

```rust
pub struct SourceFile {
    pub path: String,       // File path
    pub language: String,   // Language name from provider
    pub content: String,    // Raw source text
    pub symbols: Vec<Symbol>,  // Extracted definitions
    pub imports: Vec<Import>,  // Extracted imports
}
```

### Symbol and Import types

From `roko-core::language`:

```rust
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub line: usize,        // 1-based line number
}

pub enum SymbolKind {
    Function, Struct, Enum, Trait, Const, Type, Module, Impl,
    // #[non_exhaustive] — additional kinds can be added
}

pub enum Visibility {
    Public, Private, Crate,
}

pub struct Import {
    pub path: String,       // e.g., "std::collections::HashMap"
    pub alias: Option<String>,
    pub kind: ImportKind,
}

pub enum ImportKind {
    Use, Require, Import, TypeOnly,
}
```

### The design advantage

This trait-based design means `roko-index` never needs to know which language it's working with. The graph builder, PageRank, HDC fingerprinter, and (future) search layer all operate on `SourceFile`, `Symbol`, and `Import` — language-neutral abstractions. Adding Python support means writing a `PythonLanguageProvider`; the rest of the stack works unchanged.

---

## Current Heuristic Parsers

### Rust: `roko-lang-rust` (819 lines)

The `RustLanguageProvider` implements line-by-line heuristic parsing for Rust:

**Import parsing** extracts three forms:
- `use path::to::Item;` (and `use path::to::{A, B};` with brace expansion)
- `mod name;` (module declarations)
- `extern crate name;`

**Symbol extraction** recognizes:
- Functions: `fn`, `async fn`, `unsafe fn`, `const fn`, `pub fn`, `pub(crate) fn`
- Structs: `struct Name`
- Enums: `enum Name`
- Traits: `trait Name`
- Impls: `impl Name` (including `impl Trait for Type`)
- Constants: `const NAME`
- Type aliases: `type Name`
- Modules: `mod name`

**Visibility handling** parses `pub`, `pub(crate)`, and `pub(super)` prefixes.

**Angle bracket skipping** handles generic type parameters in function signatures (e.g., `fn process<T: Display>(x: T)`) to correctly extract the function name.

**Known limitations**:
- Cannot parse nested function definitions (closures, inner functions)
- Cannot parse items generated by procedural macros
- Multi-line function signatures where `fn` and the name are on different lines
- `#[cfg]`-gated items are always included regardless of feature flags
- Attribute macros that transform item syntax

### TypeScript: `roko-lang-typescript` (917 lines)

The `TypeScriptLanguageProvider` handles TypeScript and JavaScript:

**Import parsing** extracts:
- ES module imports: `import { X } from "path"`, `import X from "path"`, `import * as X from "path"`
- Type-only imports: `import type { X } from "path"`
- CommonJS requires: `const X = require("path")`

**Symbol extraction** recognizes:
- Functions: `function name(`, `export function name(`, `async function name(`
- Classes: `class Name` (mapped to `SymbolKind::Struct`)
- Interfaces: `interface Name` (mapped to `SymbolKind::Trait`)
- Type aliases: `type Name =`
- Constants: `const NAME`, `export const NAME`
- Enums: `enum Name`
- Export defaults: `export default class/function`

**Build systems**: includes `NpmBuildSystem`, `PnpmBuildSystem`, and `YarnBuildSystem` implementations for the `BuildSystem` trait.

**Known limitations**:
- Cannot parse destructured exports
- Cannot distinguish between `const` value declarations and `const enum`
- Template literal expressions in import paths
- Dynamic `import()` calls are not captured
- JSX/TSX component definitions are not recognized as symbols

### Go: `roko-lang-go` (600 lines)

The `GoLanguageProvider` handles Go:

**Import parsing** extracts:
- Single imports: `import "path"`
- Grouped imports: `import (\n\t"path1"\n\t"path2"\n)`
- Aliased imports: `import alias "path"`

**Symbol extraction** recognizes:
- Functions: `func name(` — distinguished from methods by absence of receiver
- Methods: `func (receiver Type) name(` — extracted with method name only
- Structs: `type Name struct`
- Interfaces: `type Name interface`
- Constants: `const Name` (single and grouped)
- Variables: `var Name`

**Visibility convention**: Go uses capitalization for visibility — names starting with uppercase are public, lowercase are private. The provider correctly applies this convention.

**Known limitations**:
- Cannot parse function types (`type Handler func(...)`)
- Cannot distinguish between method sets and embedded interfaces
- Generated code (from `go generate`) may contain patterns the heuristic misses
- Build tags (`//go:build`) are not evaluated

---

## The Case for Tree-Sitter

### What tree-sitter provides

Tree-sitter (Brunsfeld 2018) is an incremental parsing framework that generates parsers from grammar specifications. It provides:

1. **Full AST parsing** — Every syntactic construct is represented in a concrete syntax tree. No heuristic gaps.

2. **Incremental updates** — When source text changes, tree-sitter re-parses only the affected regions. A single-character edit re-parses in microseconds, not milliseconds.

3. **Error recovery** — Tree-sitter produces a valid (partial) tree even when the source contains syntax errors. This is critical for coding agents that work with incomplete code.

4. **Consistent API** — The same query and traversal APIs work across all 100+ supported languages.

5. **Performance** — Tree-sitter parsers are generated as C code and run at roughly the speed of a hand-written lexer. Initial parse of a 10,000-line file takes ~10ms.

### What tree-sitter enables that heuristics cannot

| Capability | Heuristic | Tree-sitter |
|---|---|---|
| Nested function definitions | Missed | Captured at correct scope |
| Multi-line signatures | Fragile | Robust |
| Macro-generated items | Invisible | Visible (if expanded) |
| Scope-aware symbol lookup | Impossible | Natural via AST traversal |
| Call graph extraction | Impossible | Via function call node traversal |
| `impl Trait for Type` edges | Partial | Complete with trait resolution |
| Error recovery (incomplete code) | Crashes or misparses | Partial tree with ERROR nodes |
| Column-level source locations | Not tracked | Exact byte offsets |

The most critical capability tree-sitter enables is **call graph extraction**. The current `SymbolGraph` can only create `Imports` edges because heuristic parsers don't identify function call sites. Tree-sitter's AST contains `call_expression` nodes that identify exactly which function is being called and where — enabling `Calls` edges that make the dependency graph dramatically more useful.

### Integration plan

The migration to tree-sitter is designed to be backward-compatible with the existing `LanguageProvider` trait:

```rust
// Planned: TreeSitterProvider wraps a tree-sitter grammar
pub struct TreeSitterProvider {
    language: tree_sitter::Language,
    queries: LanguageQueries,  // Pre-compiled tree-sitter queries
}

impl LanguageProvider for TreeSitterProvider {
    fn language_name(&self) -> &str { ... }
    fn file_extensions(&self) -> &[&str] { ... }
    fn parse_imports(&self, source: &str) -> Vec<Import> {
        // Use tree-sitter query to find import nodes
        ...
    }
    fn extract_symbols(&self, source: &str) -> Vec<Symbol> {
        // Use tree-sitter query to find definition nodes
        ...
    }
}
```

Each language would have a `LanguageQueries` struct containing pre-compiled tree-sitter queries for:
- Import extraction (use statements, require calls, import declarations)
- Symbol extraction (function definitions, type definitions, etc.)
- Call site extraction (function call expressions, method calls)
- Scope relationships (which symbols are nested within others)

The existing `roko-lang-*` crates would be updated to use `TreeSitterProvider` internally while maintaining the same `LanguageProvider` trait interface. Consumer code in `roko-index` would require zero changes.

### Tree-sitter query examples

For Rust, a tree-sitter query to extract function definitions:

```scheme
;; Match function definitions
(function_item
  name: (identifier) @name
  parameters: (parameters) @params
  return_type: (_)? @return_type
) @definition.function

;; Match method definitions in impl blocks
(impl_item
  type: (_) @impl_type
  body: (declaration_list
    (function_item
      name: (identifier) @method_name
    ) @definition.method
  )
)
```

For TypeScript:

```scheme
;; Match function declarations
(function_declaration
  name: (identifier) @name
) @definition.function

;; Match class declarations
(class_declaration
  name: (type_identifier) @name
) @definition.class

;; Match interface declarations
(interface_declaration
  name: (type_identifier) @name
) @definition.interface
```

### Incremental parsing workflow

```
                    Initial Parse
                    ─────────────
  Source text ──→ tree_sitter::Parser ──→ Tree (full AST)
                                            │
                                            ▼
                                     Store tree + hash

                    Incremental Update
                    ──────────────────
  Git diff ──→ compute edit ranges ──→ tree.edit(InputEdit)
                                            │
                                            ▼
                                   parser.parse(text, Some(old_tree))
                                            │
                                            ▼
                                   New tree (partial re-parse)
                                            │
                                            ▼
                                   Extract changed symbols only
```

The key insight is that `tree_sitter::Parser::parse()` accepts an optional `old_tree` parameter. When provided, tree-sitter diffs the old and new trees internally and re-parses only the changed regions. For a typical single-function edit, this means re-parsing a few hundred bytes rather than the entire file.

---

## Performance Characteristics

### Parsing benchmarks (target)

| Operation | Heuristic (current) | Tree-sitter (target) | Notes |
|---|---|---|---|
| Initial parse (10K line file) | ~2ms | ~10ms | Tree-sitter is slower for initial but more accurate |
| Incremental re-parse (1 line change) | N/A (full re-parse) | ~50μs | Tree-sitter's killer feature |
| Full workspace parse (177K lines) | ~100ms | ~500ms | Initial; subsequent passes are incremental |
| Symbol extraction (per file) | ~0.5ms | ~1ms | Including query execution |
| Import extraction (per file) | ~0.3ms | ~0.5ms | Including query execution |

The initial parse is slower with tree-sitter, but the incremental advantage is overwhelming: re-parsing a single-line change is ~40× faster than the heuristic approach (which re-parses the entire file). For the common case of agent-driven modifications — small, focused changes — tree-sitter's incremental parsing means re-indexing is essentially free.

### Memory characteristics

| Metric | Estimate |
|---|---|
| Tree per 10K-line file | ~1–3 MB |
| All trees for Roko workspace | ~30–50 MB |
| Query cursors (reused) | ~1 KB each |
| Total memory overhead | ~50 MB for full workspace |

The memory cost is acceptable for a development tool. Trees can be dropped and re-parsed from disk if memory pressure requires it; the re-parse is fast enough to be transparent.

---

## Academic Foundations

- **Tree-sitter**: Brunsfeld (2018). Incremental parsing framework. The target backend for `roko-index` language parsing. Supports 100+ languages with consistent API, error recovery, and sub-millisecond incremental updates.
- **Principles of Program Analysis**: Nielson, Nielson, and Hankin (1999). Foundational text on static analysis. Provides the theoretical basis for extracting structural information from source code.
- **Code property graphs**: Yamaguchi, Golde, Arp, and Rieck (2014), "Modeling and Discovering Vulnerabilities with Code Property Graphs." *IEEE S&P*. Demonstrates the value of unifying AST, CFG, and PDG into a single queryable structure — the goal for tree-sitter-enhanced parsing.
- **Aider repository map**: Gauthier (2024). Uses tree-sitter to build repository maps that improve coding agent context quality. Direct inspiration for `roko-index`'s planned tree-sitter integration.
- **cAST**: Xiao et al. (2024). Code AST-based methods for understanding and generating code. Demonstrates effectiveness of AST-derived features for code understanding tasks.

---

## Current Status and Gaps

### Built

- `LanguageProvider` trait in `roko-core` — stable, well-defined contract
- `parse_source()` function in `roko-index` — delegates to providers correctly
- `SourceFile` struct — captures parsed output with symbols and imports
- `RustLanguageProvider` — heuristic parser, handles common Rust constructs
- `TypeScriptLanguageProvider` — heuristic parser, handles TS/JS constructs
- `GoLanguageProvider` — heuristic parser, handles Go constructs
- Build system abstractions (`CargoBuildSystem`, `NpmBuildSystem`, etc.)

### Missing

- Tree-sitter integration (no `tree-sitter` dependency in any crate)
- Tree-sitter query definitions for any language
- Incremental parsing support (no `old_tree` reuse)
- Call site extraction (no `Calls` edges in the graph)
- Scope-aware symbol nesting (flat symbol list only)
- Column-level source locations (line numbers only)
- Python, Java, C++, and other language providers

---

## Cross-References

- See [00-vision.md](./00-vision.md) for why code intelligence matters
- See [02-symbol-extraction.md](./02-symbol-extraction.md) for what parsers extract and how symbols are represented
- See [03-dependency-graph.md](./03-dependency-graph.md) for how parsed symbols become a graph
- See topic [00-architecture](../00-architecture/INDEX.md) for the Synapse Architecture and `LanguageProvider` trait origin
- See topic [18-tools](../18-tools/INDEX.md) for the tool system that language providers plug into
