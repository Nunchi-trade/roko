//! Symbol identifiers, references, and lookup utilities.
//!
//! Re-exports the core [`Symbol`] and [`SymbolKind`] types from `roko_core` and
//! adds indexing-specific wrappers: [`SymbolId`] (unique key) and [`SymbolRef`]
//! (usage site).

use serde::{Deserialize, Serialize};

// Re-export the core types so consumers can use `roko_index::symbol::Symbol` etc.
pub use roko_core::language::{Symbol, SymbolKind, Visibility};

use crate::parser::SourceFile;

// ─── SymbolId ───────────────────────────────────────────────────────────

/// Unique identifier for a symbol within an index.
///
/// Composed of the file path, symbol name, and kind. Two symbols with
/// identical (path, name, kind) are considered the same definition.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SymbolId {
    /// Path of the file that defines the symbol.
    pub file_path: String,
    /// Symbol name.
    pub symbol_name: String,
    /// Symbol kind.
    pub kind: SymbolKind,
}

impl SymbolId {
    /// Build a `SymbolId` from its components.
    pub fn new(file_path: impl Into<String>, symbol_name: impl Into<String>, kind: SymbolKind) -> Self {
        Self {
            file_path: file_path.into(),
            symbol_name: symbol_name.into(),
            kind,
        }
    }

    /// Derive a `SymbolId` from a [`Symbol`] and the file that contains it.
    pub fn from_symbol(symbol: &Symbol, file_path: &str) -> Self {
        Self {
            file_path: file_path.to_string(),
            symbol_name: symbol.name.clone(),
            kind: symbol.kind.clone(),
        }
    }
}

impl std::fmt::Display for SymbolId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}::{}({:?})", self.file_path, self.symbol_name, self.kind)
    }
}

// ─── SymbolRef ──────────────────────────────────────────────────────────

/// A reference to a symbol at a specific location in source code.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SymbolRef {
    /// File containing the reference.
    pub file: String,
    /// 1-based line number.
    pub line: usize,
    /// 0-based column offset.
    pub column: usize,
}

impl SymbolRef {
    /// Create a new symbol reference.
    pub fn new(file: impl Into<String>, line: usize, column: usize) -> Self {
        Self {
            file: file.into(),
            line,
            column,
        }
    }
}

// ─── Lookup ─────────────────────────────────────────────────────────────

/// Find all symbols whose name matches `name` across a set of parsed files.
pub fn find_symbol<'a>(files: &'a [SourceFile], name: &str) -> Vec<&'a Symbol> {
    files
        .iter()
        .flat_map(|f| f.symbols.iter())
        .filter(|s| s.name == name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::SourceFile;
    use roko_core::language::{Import, ImportKind};

    fn sample_files() -> Vec<SourceFile> {
        vec![
            SourceFile {
                path: "a.rs".into(),
                language: "rust".into(),
                content: String::new(),
                symbols: vec![
                    Symbol { name: "foo".into(), kind: SymbolKind::Function, visibility: Visibility::Public, line: 1 },
                    Symbol { name: "Bar".into(), kind: SymbolKind::Struct, visibility: Visibility::Public, line: 5 },
                ],
                imports: vec![],
            },
            SourceFile {
                path: "b.rs".into(),
                language: "rust".into(),
                content: String::new(),
                symbols: vec![
                    Symbol { name: "foo".into(), kind: SymbolKind::Function, visibility: Visibility::Private, line: 1 },
                ],
                imports: vec![Import { path: "a::Bar".into(), alias: None, kind: ImportKind::Use }],
            },
        ]
    }

    #[test]
    fn find_symbol_by_name() {
        let files = sample_files();
        let results = find_symbol(&files, "foo");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn find_symbol_no_match() {
        let files = sample_files();
        let results = find_symbol(&files, "nonexistent");
        assert!(results.is_empty());
    }

    #[test]
    fn find_symbol_unique() {
        let files = sample_files();
        let results = find_symbol(&files, "Bar");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, SymbolKind::Struct);
    }

    #[test]
    fn symbol_id_display() {
        let id = SymbolId::new("lib.rs", "main", SymbolKind::Function);
        let display = format!("{id}");
        assert!(display.contains("lib.rs"));
        assert!(display.contains("main"));
    }

    #[test]
    fn symbol_id_equality() {
        let a = SymbolId::new("x.rs", "Foo", SymbolKind::Struct);
        let b = SymbolId::new("x.rs", "Foo", SymbolKind::Struct);
        assert_eq!(a, b);
    }

    #[test]
    fn symbol_id_different_kind() {
        let a = SymbolId::new("x.rs", "Foo", SymbolKind::Struct);
        let b = SymbolId::new("x.rs", "Foo", SymbolKind::Function);
        assert_ne!(a, b);
    }

    #[test]
    fn symbol_ref_creation() {
        let r = SymbolRef::new("main.rs", 10, 4);
        assert_eq!(r.file, "main.rs");
        assert_eq!(r.line, 10);
        assert_eq!(r.column, 4);
    }

    #[test]
    fn symbol_id_from_symbol() {
        let sym = Symbol {
            name: "process".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            line: 42,
        };
        let id = SymbolId::from_symbol(&sym, "handler.rs");
        assert_eq!(id.file_path, "handler.rs");
        assert_eq!(id.symbol_name, "process");
        assert_eq!(id.kind, SymbolKind::Function);
    }
}
