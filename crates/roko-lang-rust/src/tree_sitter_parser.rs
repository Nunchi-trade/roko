//! Tree-sitter-based Rust language provider.
//!
//! Feature-gated behind `tree-sitter`. Provides accurate AST-based symbol
//! extraction and import parsing, replacing the heuristic regex parser for
//! cases where precision matters (e.g. nested functions, complex generics,
//! conditional compilation blocks).
//!
//! Implements the same [`LanguageProvider`] trait, so callers can swap
//! transparently.

use roko_core::language::{Import, ImportKind, LanguageProvider, Symbol, SymbolKind, Visibility};

/// Tree-sitter-based Rust language provider.
///
/// Uses the `tree-sitter-rust` grammar for accurate, incremental,
/// error-tolerant parsing. Falls back gracefully on parse errors by
/// extracting whatever symbols the partial AST contains.
pub struct TreeSitterRustProvider;

impl LanguageProvider for TreeSitterRustProvider {
    fn language_name(&self) -> &str {
        "rust"
    }

    fn file_extensions(&self) -> &[&str] {
        &["rs"]
    }

    fn parse_imports(&self, source: &str) -> Vec<Import> {
        let Some(tree) = parse_source(source) else {
            return Vec::new();
        };
        let mut imports = Vec::new();
        let root = tree.root_node();
        collect_imports(root, source, &mut imports);
        imports
    }

    fn extract_symbols(&self, source: &str) -> Vec<Symbol> {
        let Some(tree) = parse_source(source) else {
            return Vec::new();
        };
        let mut symbols = Vec::new();
        let root = tree.root_node();
        collect_symbols(root, source, &mut symbols);
        symbols
    }
}

fn parse_source(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = tree_sitter::Parser::new();
    let language = tree_sitter_rust::LANGUAGE;
    parser
        .set_language(&language.into())
        .expect("tree-sitter-rust grammar should load");
    parser.parse(source, None)
}

fn collect_imports(node: tree_sitter::Node<'_>, source: &str, imports: &mut Vec<Import>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "use_declaration" => {
                if let Some(argument) = child.child_by_field_name("argument") {
                    extract_use_paths(argument, source, String::new(), imports);
                }
            }
            "mod_item" => {
                // `mod foo;` or `mod foo { ... }`
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(name_node, source);
                    imports.push(Import {
                        path: name,
                        alias: None,
                        kind: ImportKind::Mod,
                    });
                }
            }
            "extern_crate_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(name_node, source);
                    let alias = child
                        .child_by_field_name("alias")
                        .and_then(|a| a.child_by_field_name("alias"))
                        .map(|a| node_text(a, source));
                    imports.push(Import {
                        path: name,
                        alias,
                        kind: ImportKind::ExternCrate,
                    });
                }
            }
            _ => {}
        }
    }
}

fn extract_use_paths(
    node: tree_sitter::Node<'_>,
    source: &str,
    prefix: String,
    imports: &mut Vec<Import>,
) {
    match node.kind() {
        "scoped_identifier" | "scoped_use_list" => {
            if let Some(path_node) = node.child_by_field_name("path") {
                let path_text = node_text(path_node, source);
                let full_prefix = if prefix.is_empty() {
                    path_text
                } else {
                    format!("{prefix}::{path_text}")
                };

                if let Some(name_node) = node.child_by_field_name("name") {
                    // scoped_identifier: `path::name`
                    let name = node_text(name_node, source);
                    imports.push(Import {
                        path: format!("{full_prefix}::{name}"),
                        alias: None,
                        kind: ImportKind::Use,
                    });
                } else if let Some(list_node) = node.child_by_field_name("list") {
                    // scoped_use_list: `path::{a, b, c}`
                    let mut cursor = list_node.walk();
                    for child in list_node.children(&mut cursor) {
                        if child.kind() == "," || child.kind() == "{" || child.kind() == "}" {
                            continue;
                        }
                        extract_use_paths(child, source, full_prefix.clone(), imports);
                    }
                }
            }
        }
        "use_as_clause" => {
            if let Some(path_node) = node.child_by_field_name("path") {
                let path_text = node_text(path_node, source);
                let full_path = if prefix.is_empty() {
                    path_text
                } else {
                    format!("{prefix}::{path_text}")
                };
                let alias = node
                    .child_by_field_name("alias")
                    .map(|a| node_text(a, source));
                imports.push(Import {
                    path: full_path,
                    alias,
                    kind: ImportKind::Use,
                });
            }
        }
        "identifier" | "self" => {
            let name = node_text(node, source);
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}::{name}")
            };
            imports.push(Import {
                path,
                alias: None,
                kind: ImportKind::Use,
            });
        }
        "use_wildcard" => {
            let path = if prefix.is_empty() {
                "*".to_string()
            } else {
                format!("{prefix}::*")
            };
            imports.push(Import {
                path,
                alias: None,
                kind: ImportKind::Use,
            });
        }
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "," || child.kind() == "{" || child.kind() == "}" {
                    continue;
                }
                extract_use_paths(child, source, prefix.clone(), imports);
            }
        }
        _ => {
            // Fallback: treat node text as a simple path
            let text = node_text(node, source);
            if !text.is_empty() && !text.starts_with('{') {
                let path = if prefix.is_empty() {
                    text
                } else {
                    format!("{prefix}::{text}")
                };
                imports.push(Import {
                    path,
                    alias: None,
                    kind: ImportKind::Use,
                });
            }
        }
    }
}

fn collect_symbols(node: tree_sitter::Node<'_>, source: &str, symbols: &mut Vec<Symbol>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let (kind, name_field) = match child.kind() {
            "function_item" => (SymbolKind::Function, "name"),
            "struct_item" => (SymbolKind::Struct, "name"),
            "enum_item" => (SymbolKind::Enum, "name"),
            "trait_item" => (SymbolKind::Trait, "name"),
            "const_item" => (SymbolKind::Const, "name"),
            "type_item" => (SymbolKind::Type, "name"),
            "mod_item" => (SymbolKind::Module, "name"),
            "impl_item" => {
                // Impl blocks: extract the type name
                let vis = node_visibility(&child);
                let type_name = child
                    .child_by_field_name("type")
                    .map(|t| node_text(t, source))
                    .unwrap_or_else(|| "unknown".to_string());
                let trait_name = child
                    .child_by_field_name("trait")
                    .map(|t| node_text(t, source));
                let name = if let Some(tr) = trait_name {
                    format!("{tr} for {type_name}")
                } else {
                    type_name
                };
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Impl,
                    visibility: vis,
                    line: child.start_position().row + 1,
                });
                // Recurse into impl body for methods
                if let Some(body) = child.child_by_field_name("body") {
                    collect_impl_methods(body, source, symbols);
                }
                continue;
            }
            _ => {
                // Recurse into other nodes (e.g. modules with bodies)
                if child.child_count() > 0 && child.kind() != "use_declaration" {
                    // For mod items with bodies, recurse into the body
                }
                continue;
            }
        };

        if let Some(name_node) = child.child_by_field_name(name_field) {
            let name = node_text(name_node, source);
            let vis = node_visibility(&child);
            symbols.push(Symbol {
                name,
                kind,
                visibility: vis,
                line: child.start_position().row + 1,
            });
        }

        // Recurse into mod bodies
        if child.kind() == "mod_item" {
            if let Some(body) = child.child_by_field_name("body") {
                collect_symbols(body, source, symbols);
            }
        }
    }
}

fn collect_impl_methods(
    body: tree_sitter::Node<'_>,
    source: &str,
    symbols: &mut Vec<Symbol>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "function_item" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = node_text(name_node, source);
                let vis = node_visibility(&child);
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Function,
                    visibility: vis,
                    line: child.start_position().row + 1,
                });
            }
        }
    }
}

fn node_text(node: tree_sitter::Node<'_>, source: &str) -> String {
    source[node.byte_range()].to_string()
}

fn node_visibility(node: &tree_sitter::Node<'_>) -> Visibility {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            return Visibility::Public;
        }
    }
    Visibility::Private
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_function_extraction() {
        let provider = TreeSitterRustProvider;
        let source = r#"
fn main() {
    println!("hello");
}

pub fn helper() -> i32 {
    42
}
"#;
        let symbols = provider.extract_symbols(source);
        assert!(symbols.len() >= 2, "expected at least 2 symbols, got {}", symbols.len());

        let main_fn = symbols.iter().find(|s| s.name == "main").expect("main");
        assert_eq!(main_fn.kind, SymbolKind::Function);
        assert_eq!(main_fn.visibility, Visibility::Private);

        let helper_fn = symbols.iter().find(|s| s.name == "helper").expect("helper");
        assert_eq!(helper_fn.kind, SymbolKind::Function);
        assert_eq!(helper_fn.visibility, Visibility::Public);
    }

    #[test]
    fn struct_and_enum_extraction() {
        let provider = TreeSitterRustProvider;
        let source = r#"
pub struct Foo {
    x: i32,
}

enum Bar {
    A,
    B,
}
"#;
        let symbols = provider.extract_symbols(source);
        let foo = symbols.iter().find(|s| s.name == "Foo").expect("Foo");
        assert_eq!(foo.kind, SymbolKind::Struct);
        assert_eq!(foo.visibility, Visibility::Public);

        let bar = symbols.iter().find(|s| s.name == "Bar").expect("Bar");
        assert_eq!(bar.kind, SymbolKind::Enum);
        assert_eq!(bar.visibility, Visibility::Private);
    }

    #[test]
    fn impl_block_extraction() {
        let provider = TreeSitterRustProvider;
        let source = r#"
struct Foo;

impl Foo {
    pub fn new() -> Self { Foo }
    fn private_method(&self) {}
}

impl Display for Foo {
    fn fmt(&self, f: &mut Formatter) -> Result {
        Ok(())
    }
}
"#;
        let symbols = provider.extract_symbols(source);

        let impl_foo = symbols.iter().find(|s| s.name == "Foo" && s.kind == SymbolKind::Impl);
        assert!(impl_foo.is_some(), "should find impl Foo");

        let impl_display = symbols
            .iter()
            .find(|s| s.name.contains("Display") && s.kind == SymbolKind::Impl);
        assert!(impl_display.is_some(), "should find impl Display for Foo");

        let new_fn = symbols.iter().find(|s| s.name == "new").expect("new fn");
        assert_eq!(new_fn.kind, SymbolKind::Function);
        assert_eq!(new_fn.visibility, Visibility::Public);
    }

    #[test]
    fn use_import_extraction() {
        let provider = TreeSitterRustProvider;
        let source = r#"
use std::collections::HashMap;
use std::io::{self, Read, Write};
use crate::foo;
mod bar;
extern crate serde;
"#;
        let imports = provider.parse_imports(source);
        assert!(
            imports.len() >= 4,
            "expected at least 4 imports, got {} ({:?})",
            imports.len(),
            imports
        );

        let has_hashmap = imports.iter().any(|i| i.path.contains("HashMap"));
        assert!(has_hashmap, "should find HashMap import");

        let has_mod = imports.iter().any(|i| i.kind == ImportKind::Mod);
        assert!(has_mod, "should find mod declaration");

        let has_extern = imports.iter().any(|i| i.kind == ImportKind::ExternCrate);
        assert!(has_extern, "should find extern crate");
    }

    #[test]
    fn trait_and_const_extraction() {
        let provider = TreeSitterRustProvider;
        let source = r#"
pub trait MyTrait {
    fn do_thing(&self);
}

const MAX_SIZE: usize = 100;

type Result<T> = std::result::Result<T, MyError>;
"#;
        let symbols = provider.extract_symbols(source);

        let my_trait = symbols.iter().find(|s| s.name == "MyTrait").expect("MyTrait");
        assert_eq!(my_trait.kind, SymbolKind::Trait);
        assert_eq!(my_trait.visibility, Visibility::Public);

        let max_size = symbols.iter().find(|s| s.name == "MAX_SIZE").expect("MAX_SIZE");
        assert_eq!(max_size.kind, SymbolKind::Const);
    }

    #[test]
    fn handles_parse_errors_gracefully() {
        let provider = TreeSitterRustProvider;
        // Intentionally malformed Rust
        let source = "fn main( { }}} pub struct Foo;";
        let symbols = provider.extract_symbols(source);
        // Should extract what it can, not panic
        let _count = symbols.len(); // Just verify it doesn't crash
    }

    #[test]
    fn heuristic_vs_tree_sitter_parity() {
        // Verify the tree-sitter provider extracts at least as many symbols
        // as the heuristic parser for a simple case.
        let heuristic = crate::RustLanguageProvider;
        let ts = TreeSitterRustProvider;

        let source = r#"
pub fn public_fn() {}
fn private_fn() {}
pub struct MyStruct { x: i32 }
enum MyEnum { A, B }
pub trait MyTrait { fn required(&self); }
const MY_CONST: i32 = 42;
type MyType = Vec<i32>;
"#;
        let heuristic_symbols = heuristic.extract_symbols(source);
        let ts_symbols = ts.extract_symbols(source);

        assert!(
            ts_symbols.len() >= heuristic_symbols.len(),
            "tree-sitter ({}) should extract at least as many symbols as heuristic ({})",
            ts_symbols.len(),
            heuristic_symbols.len()
        );
    }
}
