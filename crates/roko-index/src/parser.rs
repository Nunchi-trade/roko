//! Source code parser — delegates to [`LanguageProvider`] for extraction.
//!
//! The parser is language-agnostic: it receives a [`LanguageProvider`] trait
//! object that knows how to extract imports and symbols from source text for
//! a particular language.

use roko_core::language::{Import, LanguageProvider, Symbol};

/// A parsed source file with extracted symbols and imports.
#[derive(Clone, Debug)]
pub struct SourceFile {
    /// File path (relative or absolute).
    pub path: String,
    /// Language name (as reported by the provider).
    pub language: String,
    /// Raw source text.
    pub content: String,
    /// Symbols extracted from the source.
    pub symbols: Vec<Symbol>,
    /// Imports extracted from the source.
    pub imports: Vec<Import>,
}

/// Parse source code using the given [`LanguageProvider`].
///
/// Extracts symbols and imports from `content` and packages them into a
/// [`SourceFile`].
pub fn parse_source(path: &str, content: &str, provider: &dyn LanguageProvider) -> SourceFile {
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

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::language::{ImportKind, SymbolKind, Visibility};

    /// Minimal test provider that recognises simple patterns.
    struct TestLang;

    impl LanguageProvider for TestLang {
        fn language_name(&self) -> &str {
            "test"
        }

        fn file_extensions(&self) -> &[&str] {
            &["tst"]
        }

        fn parse_imports(&self, source: &str) -> Vec<Import> {
            source
                .lines()
                .filter_map(|line| {
                    let rest = line.trim().strip_prefix("import ")?;
                    Some(Import {
                        path: rest.trim_end_matches(';').to_string(),
                        alias: None,
                        kind: ImportKind::Use,
                    })
                })
                .collect()
        }

        fn extract_symbols(&self, source: &str) -> Vec<Symbol> {
            source
                .lines()
                .enumerate()
                .filter_map(|(i, line)| {
                    let trimmed = line.trim();
                    if let Some(name) = trimmed.strip_prefix("fn ") {
                        let name = name.trim_end_matches("()").trim_end_matches(" {");
                        Some(Symbol {
                            name: name.to_string(),
                            kind: SymbolKind::Function,
                            visibility: Visibility::Private,
                            line: i + 1,
                        })
                    } else if let Some(name) = trimmed.strip_prefix("struct ") {
                        let name = name.trim_end_matches(" {");
                        Some(Symbol {
                            name: name.to_string(),
                            kind: SymbolKind::Struct,
                            visibility: Visibility::Public,
                            line: i + 1,
                        })
                    } else {
                        None
                    }
                })
                .collect()
        }
    }

    #[test]
    fn parse_source_extracts_symbols() {
        let src = "fn hello()\nfn world()\n";
        let file = parse_source("test.tst", src, &TestLang);
        assert_eq!(file.symbols.len(), 2);
        assert_eq!(file.symbols[0].name, "hello");
        assert_eq!(file.symbols[1].name, "world");
    }

    #[test]
    fn parse_source_extracts_imports() {
        let src = "import foo;\nimport bar;\nfn baz()\n";
        let file = parse_source("test.tst", src, &TestLang);
        assert_eq!(file.imports.len(), 2);
        assert_eq!(file.imports[0].path, "foo");
        assert_eq!(file.imports[1].path, "bar");
    }

    #[test]
    fn parse_source_metadata() {
        let file = parse_source("src/main.tst", "fn main()", &TestLang);
        assert_eq!(file.path, "src/main.tst");
        assert_eq!(file.language, "test");
        assert_eq!(file.content, "fn main()");
    }

    #[test]
    fn parse_source_empty() {
        let file = parse_source("empty.tst", "", &TestLang);
        assert!(file.symbols.is_empty());
        assert!(file.imports.is_empty());
    }

    #[test]
    fn parse_source_mixed_content() {
        let src = "import std;\nstruct Config {\nfn new()\n";
        let file = parse_source("mixed.tst", src, &TestLang);
        assert_eq!(file.imports.len(), 1);
        assert_eq!(file.symbols.len(), 2); // struct + fn
    }
}
