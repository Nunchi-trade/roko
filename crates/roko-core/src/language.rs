//! Language provider abstraction — source-level analysis without I/O.
//!
//! [`LanguageProvider`] defines how to parse imports and extract symbols from
//! source code for a given programming language. Implementations operate on
//! `&str` source text and never touch the filesystem.

use serde::{Deserialize, Serialize};

// ─── Import ──────────────────────────────────────────────────────────────

/// The kind of import statement.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ImportKind {
    /// Rust `use` / `TypeScript` `import` / Python `import` / Go `import`.
    Use,
    /// Rust `mod` declaration.
    Mod,
    /// Rust `extern crate`.
    ExternCrate,
}

/// A single import extracted from source code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    /// The import path (e.g. `"std::collections::HashMap"`).
    pub path: String,
    /// Optional alias (`as` rename).
    pub alias: Option<String>,
    /// What kind of import this is.
    pub kind: ImportKind,
}

// ─── Symbol ──────────────────────────────────────────────────────────────

/// The kind of symbol defined in source.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(
    feature = "rkyv",
    derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)
)]
#[non_exhaustive]
pub enum SymbolKind {
    /// A function or method.
    Function,
    /// A struct.
    Struct,
    /// An enum.
    Enum,
    /// A trait (Rust) / interface (Go, TS).
    Trait,
    /// A constant (`const`).
    Const,
    /// A type alias.
    Type,
    /// A module declaration.
    Module,
    /// An impl block.
    Impl,
}

/// Visibility of a symbol.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(
    feature = "rkyv",
    derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)
)]
pub enum Visibility {
    /// Publicly visible (`pub`).
    Public,
    /// Private / crate-private.
    Private,
}

/// A symbol extracted from source code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    /// Symbol name (e.g. `"HashMap"`, `"main"`).
    pub name: String,
    /// What kind of symbol this is.
    pub kind: SymbolKind,
    /// Whether the symbol is public or private.
    pub visibility: Visibility,
    /// 1-based line number where the symbol is defined.
    pub line: usize,
}

// ─── LanguageProvider trait ──────────────────────────────────────────────

/// Provides source-level analysis for a specific language.
///
/// Implementations parse source text to extract imports and symbols. They must
/// be pure functions of the input text — no filesystem, no network.
pub trait LanguageProvider: Send + Sync {
    /// Human-readable language name (e.g. `"rust"`, `"typescript"`).
    fn language_name(&self) -> &str;

    /// File extensions this provider handles (e.g. `["rs"]`).
    fn file_extensions(&self) -> &[&str];

    /// Parse import statements from source text.
    fn parse_imports(&self, source: &str) -> Vec<Import>;

    /// Extract top-level symbol definitions from source text.
    fn extract_symbols(&self, source: &str) -> Vec<Symbol>;
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal provider for testing the trait contract.
    struct FakeLang;

    impl LanguageProvider for FakeLang {
        fn language_name(&self) -> &str {
            "fake"
        }

        fn file_extensions(&self) -> &[&str] {
            &["fk", "fake"]
        }

        fn parse_imports(&self, source: &str) -> Vec<Import> {
            // Recognises lines like "import foo" or "import foo as bar"
            source
                .lines()
                .filter_map(|line| {
                    let trimmed = line.trim();
                    let rest = trimmed.strip_prefix("import ")?;
                    let parts: Vec<&str> = rest.splitn(3, ' ').collect();
                    match parts.as_slice() {
                        [path] => Some(Import {
                            path: (*path).to_string(),
                            alias: None,
                            kind: ImportKind::Use,
                        }),
                        [path, "as", alias] => Some(Import {
                            path: (*path).to_string(),
                            alias: Some((*alias).to_string()),
                            kind: ImportKind::Use,
                        }),
                        _ => None,
                    }
                })
                .collect()
        }

        fn extract_symbols(&self, source: &str) -> Vec<Symbol> {
            source
                .lines()
                .enumerate()
                .filter_map(|(i, line)| {
                    let trimmed = line.trim();
                    let name = trimmed.strip_prefix("fn ")?.trim_end_matches("()");
                    Some(Symbol {
                        name: name.to_string(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        line: i + 1,
                    })
                })
                .collect()
        }
    }

    #[test]
    fn fake_lang_metadata() {
        let lang = FakeLang;
        assert_eq!(lang.language_name(), "fake");
        assert_eq!(lang.file_extensions(), &["fk", "fake"]);
    }

    #[test]
    fn fake_lang_parse_imports() {
        let lang = FakeLang;
        let src = "import foo\nimport bar as baz\nsome other line\n";
        let imports = lang.parse_imports(src);
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].path, "foo");
        assert!(imports[0].alias.is_none());
        assert_eq!(imports[1].path, "bar");
        assert_eq!(imports[1].alias.as_deref(), Some("baz"));
    }

    #[test]
    fn fake_lang_extract_symbols() {
        let lang = FakeLang;
        let src = "fn hello()\nfn world()\n";
        let syms = lang.extract_symbols(src);
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].name, "hello");
        assert_eq!(syms[0].line, 1);
        assert_eq!(syms[1].name, "world");
        assert_eq!(syms[1].line, 2);
    }

    #[test]
    fn import_kind_variants_distinct() {
        assert_ne!(ImportKind::Use, ImportKind::Mod);
        assert_ne!(ImportKind::Mod, ImportKind::ExternCrate);
    }

    #[test]
    fn symbol_visibility_variants_distinct() {
        assert_ne!(Visibility::Public, Visibility::Private);
    }
}
