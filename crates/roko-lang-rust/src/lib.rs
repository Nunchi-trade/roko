//! Rust/Cargo implementations of [`roko_core::BuildSystem`] and
//! [`roko_core::LanguageProvider`].
//!
//! - [`CargoBuildSystem`]: produces `cargo check`, `cargo test`, `cargo clippy`,
//!   `cargo fmt` commands as [`BuildCommand`] descriptors.
//! - [`RustLanguageProvider`]: parses `use`/`mod`/`extern crate` imports,
//!   including simple brace-expanded `use` trees, and extracts
//!   `fn`/`struct`/`enum`/`trait`/`impl`/`const`/`type`/`mod` symbols from Rust
//!   source text.

#![allow(clippy::module_name_repetitions)]
// The trait signatures use `&str` returns; clippy suggests `&'static str` but the
// trait contract binds the lifetime to `&self`. Allow it here.
#![allow(clippy::unnecessary_literal_bound)]

use roko_core::build::{BuildCommand, BuildSystem};
use roko_core::language::{Import, ImportKind, LanguageProvider, Symbol, SymbolKind, Visibility};
use std::path::Path;

// ─── CargoBuildSystem ────────────────────────────────────────────────────

/// Build system implementation for Cargo (Rust).
pub struct CargoBuildSystem;

impl BuildSystem for CargoBuildSystem {
    fn name(&self) -> &str {
        "cargo"
    }

    fn compile_cmd(&self, target_dir: &Path) -> BuildCommand {
        BuildCommand::new("cargo")
            .args(["check", "--workspace", "--all-targets"])
            .env(
                "CARGO_TARGET_DIR",
                target_dir.to_string_lossy().into_owned(),
            )
    }

    fn test_cmd(&self, target_dir: &Path, filter: Option<&str>) -> BuildCommand {
        let mut cmd = BuildCommand::new("cargo")
            .args(["test", "--workspace"])
            .env(
                "CARGO_TARGET_DIR",
                target_dir.to_string_lossy().into_owned(),
            );
        if let Some(f) = filter {
            cmd = cmd.arg("--").arg(f);
        }
        cmd
    }

    fn lint_cmd(&self, target_dir: &Path) -> BuildCommand {
        BuildCommand::new("cargo")
            .args([
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ])
            .env(
                "CARGO_TARGET_DIR",
                target_dir.to_string_lossy().into_owned(),
            )
    }

    fn format_cmd(&self, _target_dir: &Path, check_only: bool) -> BuildCommand {
        let mut cmd = BuildCommand::new("cargo").arg("fmt").arg("--all");
        if check_only {
            cmd = cmd.arg("--check");
        }
        cmd
    }

    fn detect_from_files(&self, file_names: &[&str]) -> bool {
        file_names.contains(&"Cargo.toml")
    }
}

// ─── RustLanguageProvider ────────────────────────────────────────────────

/// Language provider for Rust source files.
///
/// Uses line-by-line heuristic parsing (not a full Rust parser) to extract
/// imports and symbol definitions. This includes simple brace-expanded `use`
/// trees and both declaration (`mod foo;`) and block (`mod foo { ... }`)
/// modules. The parser remains intentionally simple and fast, suitable for
/// IDE-like features and context assembly.
pub struct RustLanguageProvider;

impl LanguageProvider for RustLanguageProvider {
    fn language_name(&self) -> &str {
        "rust"
    }

    fn file_extensions(&self) -> &[&str] {
        &["rs"]
    }

    fn parse_imports(&self, source: &str) -> Vec<Import> {
        let mut imports = Vec::new();
        for line in source.lines() {
            let trimmed = line.trim();
            if let Some(parsed) = parse_use_line(trimmed) {
                imports.extend(parsed);
            } else if let Some(imp) = parse_mod_line(trimmed) {
                imports.push(imp);
            } else if let Some(imp) = parse_extern_crate_line(trimmed) {
                imports.push(imp);
            }
        }
        imports
    }

    fn extract_symbols(&self, source: &str) -> Vec<Symbol> {
        let mut symbols = Vec::new();
        for (line_idx, line) in source.lines().enumerate() {
            let line_num = line_idx + 1;
            if let Some(sym) = extract_symbol_from_line(line, line_num) {
                symbols.push(sym);
            }
        }
        symbols
    }
}

// ─── Import parsing helpers ──────────────────────────────────────────────

/// Parse a `use ...;` line.
fn parse_use_line(trimmed: &str) -> Option<Vec<Import>> {
    // Strip optional `pub` / `pub(crate)` prefix.
    let rest = strip_visibility_prefix(trimmed);
    let rest = rest.strip_prefix("use ")?;
    let rest = rest.strip_suffix(';')?.trim();

    if let Some((prefix, items)) = split_brace_use(rest) {
        let imports = items
            .split(',')
            .filter_map(|item| {
                let item = item.trim();
                if item.is_empty() {
                    return None;
                }

                let (item_path, alias) = split_use_alias(item);
                let path = if item_path == "self" {
                    prefix.to_string()
                } else {
                    format!("{prefix}::{item_path}")
                };

                Some(Import {
                    path,
                    alias,
                    kind: ImportKind::Use,
                })
            })
            .collect();
        return Some(imports);
    }

    let (path, alias) = split_use_alias(rest);

    Some(vec![Import {
        path,
        alias,
        kind: ImportKind::Use,
    }])
}

/// Parse a `mod foo;` line (declaration, not inline block).
fn parse_mod_line(trimmed: &str) -> Option<Import> {
    let rest = strip_visibility_prefix(trimmed);
    let rest = rest.strip_prefix("mod ")?;
    // Only match declarations (ending with `;`), not `mod foo { ... }`.
    let name = rest.strip_suffix(';')?.trim();
    // Skip if it looks like a block mod (contains `{`).
    if name.contains('{') {
        return None;
    }
    Some(Import {
        path: name.to_string(),
        alias: None,
        kind: ImportKind::Mod,
    })
}

/// Parse an `extern crate foo;` or `extern crate foo as bar;` line.
fn parse_extern_crate_line(trimmed: &str) -> Option<Import> {
    let rest = strip_visibility_prefix(trimmed);
    let rest = rest.strip_prefix("extern crate ")?;
    let rest = rest.strip_suffix(';')?.trim();

    let (path, alias) = rest.rfind(" as ").map_or_else(
        || (rest.to_string(), None),
        |as_pos| {
            (
                rest[..as_pos].trim().to_string(),
                Some(rest[as_pos + 4..].trim().to_string()),
            )
        },
    );

    Some(Import {
        path,
        alias,
        kind: ImportKind::ExternCrate,
    })
}

fn split_use_alias(rest: &str) -> (String, Option<String>) {
    rest.rfind(" as ").map_or_else(
        || (rest.to_string(), None),
        |as_pos| {
            let path = rest[..as_pos].trim();
            let alias = rest[as_pos + 4..].trim();
            (path.to_string(), Some(alias.to_string()))
        },
    )
}

fn split_brace_use(rest: &str) -> Option<(&str, &str)> {
    let open = rest.find("::{")?;
    let prefix = rest[..open].trim();
    let items = rest[open + 3..].strip_suffix('}')?.trim();
    if prefix.is_empty() || items.is_empty() {
        return None;
    }
    Some((prefix, items))
}

/// Strip a leading `pub`, `pub(crate)`, `pub(super)`, or `pub(in ...)` prefix.
fn strip_visibility_prefix(s: &str) -> &str {
    let rest = s.trim_start();
    if let Some(after_pub) = rest.strip_prefix("pub") {
        let after_pub = after_pub.trim_start();
        if let Some(after_paren) = after_pub.strip_prefix('(') {
            // Find the matching `)`.
            if let Some(close) = after_paren.find(')') {
                return after_paren[close + 1..].trim_start();
            }
        }
        // Plain `pub` — just return the rest.
        after_pub
    } else {
        rest
    }
}

// ─── Symbol extraction helpers ───────────────────────────────────────────

/// Try to extract a symbol definition from a single line.
fn extract_symbol_from_line(line: &str, line_num: usize) -> Option<Symbol> {
    let trimmed = line.trim();

    // Skip comments and attributes.
    if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.is_empty() {
        return None;
    }

    // Determine visibility.
    let (vis, rest) = parse_visibility(trimmed);

    // Try each symbol kind.
    if let Some(name) = try_extract_fn(rest) {
        return Some(Symbol {
            name,
            kind: SymbolKind::Function,
            visibility: vis,
            line: line_num,
        });
    }
    if let Some(name) = try_extract_keyword(rest, "struct") {
        return Some(Symbol {
            name,
            kind: SymbolKind::Struct,
            visibility: vis,
            line: line_num,
        });
    }
    if let Some(name) = try_extract_keyword(rest, "enum") {
        return Some(Symbol {
            name,
            kind: SymbolKind::Enum,
            visibility: vis,
            line: line_num,
        });
    }
    if let Some(name) = try_extract_keyword(rest, "trait") {
        return Some(Symbol {
            name,
            kind: SymbolKind::Trait,
            visibility: vis,
            line: line_num,
        });
    }
    if let Some(name) = try_extract_impl(rest) {
        return Some(Symbol {
            name,
            kind: SymbolKind::Impl,
            visibility: vis,
            line: line_num,
        });
    }
    if let Some(name) = try_extract_const(rest) {
        return Some(Symbol {
            name,
            kind: SymbolKind::Const,
            visibility: vis,
            line: line_num,
        });
    }
    if let Some(name) = try_extract_type_alias(rest) {
        return Some(Symbol {
            name,
            kind: SymbolKind::Type,
            visibility: vis,
            line: line_num,
        });
    }
    if let Some(name) = try_extract_mod_decl(rest) {
        return Some(Symbol {
            name,
            kind: SymbolKind::Module,
            visibility: vis,
            line: line_num,
        });
    }
    if let Some(name) = try_extract_mod_block(rest) {
        return Some(Symbol {
            name,
            kind: SymbolKind::Module,
            visibility: vis,
            line: line_num,
        });
    }

    None
}

/// Parse leading visibility and return (Visibility, remaining text).
fn parse_visibility(s: &str) -> (Visibility, &str) {
    if let Some(rest) = s.strip_prefix("pub") {
        let rest = rest.trim_start();
        if let Some(after_paren) = rest.strip_prefix('(') {
            if let Some(close) = after_paren.find(')') {
                return (Visibility::Public, after_paren[close + 1..].trim_start());
            }
        }
        (Visibility::Public, rest)
    } else {
        (Visibility::Private, s)
    }
}

/// Extract function name from `fn name(` or `fn name<`.
fn try_extract_fn(s: &str) -> Option<String> {
    // Handle `async fn`, `unsafe fn`, `const fn`, `async unsafe fn`, etc.
    let rest = strip_fn_qualifiers(s);
    let rest = rest.strip_prefix("fn ")?;
    Some(extract_identifier(rest))
}

/// Strip `async`, `unsafe`, `const`, `extern "C"` qualifiers before `fn`.
fn strip_fn_qualifiers(s: &str) -> &str {
    let mut rest = s;
    loop {
        let trimmed = rest.trim_start();
        if let Some(r) = trimmed.strip_prefix("async ") {
            rest = r;
        } else if let Some(r) = trimmed.strip_prefix("unsafe ") {
            rest = r;
        } else if let Some(r) = trimmed.strip_prefix("const ") {
            // Be careful: `const FOO` is a const item, not a fn qualifier.
            // Only strip if followed by `fn`.
            if r.trim_start().starts_with("fn ") {
                rest = r;
            } else {
                break;
            }
        } else if let Some(r) = trimmed.strip_prefix("extern ") {
            // Skip the ABI string if present (e.g. `extern "C"`).
            let r = r.trim_start();
            if let Some(r2) = r.strip_prefix('"') {
                if let Some(close) = r2.find('"') {
                    rest = r2[close + 1..].trim_start();
                } else {
                    break;
                }
            } else {
                rest = r;
            }
        } else {
            break;
        }
    }
    rest
}

/// Extract a keyword-defined type name: `struct Name`, `enum Name`, `trait Name`.
fn try_extract_keyword(s: &str, keyword: &str) -> Option<String> {
    let prefix = format!("{keyword} ");
    let rest = s.strip_prefix(&prefix)?;
    Some(extract_identifier(rest))
}

/// Extract impl target: `impl Foo` or `impl Trait for Foo`.
fn try_extract_impl(s: &str) -> Option<String> {
    let rest = s.strip_prefix("impl")?;
    // Must be followed by whitespace or `<`.
    let first_char = rest.chars().next()?;
    if first_char != ' ' && first_char != '<' {
        return None;
    }
    let rest = rest.trim_start();
    // Skip generic params if present.
    let rest = skip_angle_brackets(rest);
    let rest = rest.trim_start();

    // Check for `Trait for Type` pattern.
    // First, grab the identifier (which might be the trait or the type).
    let ident = extract_identifier(rest);
    let after_ident = rest[ident.len()..].trim_start();

    // Skip generics on the first identifier.
    let after_generics = skip_angle_brackets(after_ident).trim_start();

    if let Some(after_for) = after_generics.strip_prefix("for ") {
        // `impl Trait for Type` -> name is "Trait for Type"
        let type_name = extract_identifier(after_for.trim_start());
        Some(format!("{ident} for {type_name}"))
    } else {
        Some(ident)
    }
}

/// Extract const name: `const FOO: ...`.
fn try_extract_const(s: &str) -> Option<String> {
    let rest = s.strip_prefix("const ")?;
    let name = extract_identifier(rest);
    // Make sure it's followed by `:` (not `fn` which would be a const fn).
    let after = rest[name.len()..].trim_start();
    if after.starts_with(':') {
        Some(name)
    } else {
        None
    }
}

/// Extract type alias name: `type Foo = ...`.
fn try_extract_type_alias(s: &str) -> Option<String> {
    let rest = s.strip_prefix("type ")?;
    let name = extract_identifier(rest);
    // Must be followed by `=` or `<` (generic alias).
    let after = rest[name.len()..].trim_start();
    if after.starts_with('=') || after.starts_with('<') {
        Some(name)
    } else {
        None
    }
}

/// Extract mod name from inline `mod foo {`.
fn try_extract_mod_block(s: &str) -> Option<String> {
    let rest = s.strip_prefix("mod ")?;
    let name = extract_identifier(rest);
    let after = rest[name.len()..].trim_start();
    // Only match block mods (contain `{`), not `mod foo;` (handled by import parser).
    if after.starts_with('{') {
        Some(name)
    } else {
        None
    }
}

/// Extract mod name from declaration `mod foo;`.
fn try_extract_mod_decl(s: &str) -> Option<String> {
    let rest = s.strip_prefix("mod ")?;
    let name = rest.strip_suffix(';')?.trim();
    if name.contains('{') || name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

/// Extract a Rust identifier from the start of a string.
/// Stops at the first non-identifier character.
fn extract_identifier(s: &str) -> String {
    s.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

/// Skip balanced angle brackets at the start of a string.
/// Returns the remaining text after `<...>`, or the original if no `<`.
fn skip_angle_brackets(s: &str) -> &str {
    if !s.starts_with('<') {
        return s;
    }
    let mut depth = 0i32;
    for (i, ch) in s.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return &s[i + 1..];
                }
            }
            _ => {}
        }
    }
    // Unbalanced — return original.
    s
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── CargoBuildSystem tests ───────────────────────────────────────────

    #[test]
    fn cargo_name() {
        let bs = CargoBuildSystem;
        assert_eq!(bs.name(), "cargo");
    }

    #[test]
    fn cargo_compile_cmd() {
        let bs = CargoBuildSystem;
        let cmd = bs.compile_cmd(Path::new("/tmp/target"));
        assert_eq!(cmd.program, "cargo");
        assert!(cmd.args.contains(&"check".to_string()));
        assert!(cmd.args.contains(&"--workspace".to_string()));
        assert_eq!(
            cmd.env.get("CARGO_TARGET_DIR").map(String::as_str),
            Some("/tmp/target")
        );
    }

    #[test]
    fn cargo_test_cmd_no_filter() {
        let bs = CargoBuildSystem;
        let cmd = bs.test_cmd(Path::new("/tmp/target"), None);
        assert_eq!(cmd.program, "cargo");
        assert!(cmd.args.contains(&"test".to_string()));
        assert!(!cmd.args.contains(&"--".to_string()));
    }

    #[test]
    fn cargo_test_cmd_with_filter() {
        let bs = CargoBuildSystem;
        let cmd = bs.test_cmd(Path::new("/tmp/target"), Some("my_test"));
        assert!(cmd.args.contains(&"--".to_string()));
        assert!(cmd.args.contains(&"my_test".to_string()));
    }

    #[test]
    fn cargo_lint_cmd() {
        let bs = CargoBuildSystem;
        let cmd = bs.lint_cmd(Path::new("/tmp/target"));
        assert_eq!(cmd.program, "cargo");
        assert!(cmd.args.contains(&"clippy".to_string()));
        assert!(cmd.args.contains(&"-D".to_string()));
        assert!(cmd.args.contains(&"warnings".to_string()));
    }

    #[test]
    fn cargo_format_cmd_check() {
        let bs = CargoBuildSystem;
        let cmd = bs.format_cmd(Path::new("/tmp/target"), true);
        assert_eq!(cmd.program, "cargo");
        assert!(cmd.args.contains(&"fmt".to_string()));
        assert!(cmd.args.contains(&"--check".to_string()));
    }

    #[test]
    fn cargo_format_cmd_write() {
        let bs = CargoBuildSystem;
        let cmd = bs.format_cmd(Path::new("/tmp/target"), false);
        assert!(cmd.args.contains(&"fmt".to_string()));
        assert!(!cmd.args.contains(&"--check".to_string()));
    }

    #[test]
    fn cargo_detect_positive() {
        let bs = CargoBuildSystem;
        assert!(bs.detect_from_files(&["Cargo.toml", "src"]));
    }

    #[test]
    fn cargo_detect_negative() {
        let bs = CargoBuildSystem;
        assert!(!bs.detect_from_files(&["package.json", "src"]));
    }

    // ── RustLanguageProvider — import parsing ────────────────────────────

    #[test]
    fn rust_lang_metadata() {
        let lang = RustLanguageProvider;
        assert_eq!(lang.language_name(), "rust");
        assert_eq!(lang.file_extensions(), &["rs"]);
    }

    #[test]
    fn parse_simple_use() {
        let lang = RustLanguageProvider;
        let imports = lang.parse_imports("use std::collections::HashMap;\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].path, "std::collections::HashMap");
        assert!(imports[0].alias.is_none());
        assert_eq!(imports[0].kind, ImportKind::Use);
    }

    #[test]
    fn parse_use_with_alias() {
        let lang = RustLanguageProvider;
        let imports = lang.parse_imports("use std::io::Result as IoResult;\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].path, "std::io::Result");
        assert_eq!(imports[0].alias.as_deref(), Some("IoResult"));
    }

    #[test]
    fn parse_brace_expanded_use() {
        let lang = RustLanguageProvider;
        let imports = lang.parse_imports("use std::collections::{HashMap, HashSet as Set};\n");
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].path, "std::collections::HashMap");
        assert!(imports[0].alias.is_none());
        assert_eq!(imports[1].path, "std::collections::HashSet");
        assert_eq!(imports[1].alias.as_deref(), Some("Set"));
    }

    #[test]
    fn parse_pub_use() {
        let lang = RustLanguageProvider;
        let imports = lang.parse_imports("pub use crate::error::Error;\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].path, "crate::error::Error");
        assert_eq!(imports[0].kind, ImportKind::Use);
    }

    #[test]
    fn parse_pub_crate_use() {
        let lang = RustLanguageProvider;
        let imports = lang.parse_imports("pub(crate) use crate::inner::Foo;\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].path, "crate::inner::Foo");
    }

    #[test]
    fn parse_mod_declaration() {
        let lang = RustLanguageProvider;
        let imports = lang.parse_imports("mod utils;\npub mod config;\n");
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].path, "utils");
        assert_eq!(imports[0].kind, ImportKind::Mod);
        assert_eq!(imports[1].path, "config");
    }

    #[test]
    fn parse_extern_crate() {
        let lang = RustLanguageProvider;
        let imports = lang.parse_imports("extern crate serde;\nextern crate serde_json as json;\n");
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].path, "serde");
        assert_eq!(imports[0].kind, ImportKind::ExternCrate);
        assert_eq!(imports[1].path, "serde_json");
        assert_eq!(imports[1].alias.as_deref(), Some("json"));
    }

    #[test]
    fn parse_mixed_imports() {
        let lang = RustLanguageProvider;
        let src = "\
use std::io;
mod helpers;
extern crate alloc;
// not an import
fn main() {}
";
        let imports = lang.parse_imports(src);
        assert_eq!(imports.len(), 3);
    }

    // ── RustLanguageProvider — symbol extraction ─────────────────────────

    #[test]
    fn extract_function() {
        let lang = RustLanguageProvider;
        let syms = lang.extract_symbols("fn hello() {\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "hello");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[0].visibility, Visibility::Private);
        assert_eq!(syms[0].line, 1);
    }

    #[test]
    fn extract_pub_function() {
        let lang = RustLanguageProvider;
        let syms = lang.extract_symbols("pub fn greet(name: &str) -> String {\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "greet");
        assert_eq!(syms[0].visibility, Visibility::Public);
    }

    #[test]
    fn extract_async_fn() {
        let lang = RustLanguageProvider;
        let syms = lang.extract_symbols("pub async fn fetch() {\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "fetch");
        assert_eq!(syms[0].kind, SymbolKind::Function);
    }

    #[test]
    fn extract_struct() {
        let lang = RustLanguageProvider;
        let syms = lang.extract_symbols("pub struct Foo {\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Foo");
        assert_eq!(syms[0].kind, SymbolKind::Struct);
    }

    #[test]
    fn extract_enum() {
        let lang = RustLanguageProvider;
        let syms = lang.extract_symbols("enum Color {\n    Red,\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Color");
        assert_eq!(syms[0].kind, SymbolKind::Enum);
    }

    #[test]
    fn extract_trait() {
        let lang = RustLanguageProvider;
        let syms = lang.extract_symbols("pub trait Drawable {\n    fn draw(&self);\n}\n");
        // Should find the trait and the fn inside.
        let trait_sym = syms.iter().find(|s| s.kind == SymbolKind::Trait);
        assert!(trait_sym.is_some());
        assert_eq!(trait_sym.map(|s| s.name.as_str()), Some("Drawable"));
    }

    #[test]
    fn extract_impl_block() {
        let lang = RustLanguageProvider;
        let syms = lang.extract_symbols("impl Foo {\n    fn bar() {}\n}\n");
        let impl_sym = syms.iter().find(|s| s.kind == SymbolKind::Impl);
        assert!(impl_sym.is_some());
        assert_eq!(impl_sym.map(|s| s.name.as_str()), Some("Foo"));
    }

    #[test]
    fn extract_impl_trait_for_type() {
        let lang = RustLanguageProvider;
        let syms = lang.extract_symbols("impl Display for Foo {\n}\n");
        let impl_sym = syms.iter().find(|s| s.kind == SymbolKind::Impl);
        assert!(impl_sym.is_some());
        assert_eq!(impl_sym.map(|s| s.name.as_str()), Some("Display for Foo"));
    }

    #[test]
    fn extract_const() {
        let lang = RustLanguageProvider;
        let syms = lang.extract_symbols("const MAX_SIZE: usize = 1024;\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "MAX_SIZE");
        assert_eq!(syms[0].kind, SymbolKind::Const);
    }

    #[test]
    fn extract_pub_const() {
        let lang = RustLanguageProvider;
        let syms = lang.extract_symbols("pub const VERSION: &str = \"1.0\";\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "VERSION");
        assert_eq!(syms[0].visibility, Visibility::Public);
    }

    #[test]
    fn extract_type_alias() {
        let lang = RustLanguageProvider;
        let syms = lang.extract_symbols("type Result<T> = std::result::Result<T, Error>;\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Result");
        assert_eq!(syms[0].kind, SymbolKind::Type);
    }

    #[test]
    fn extract_mod_block() {
        let lang = RustLanguageProvider;
        let syms = lang.extract_symbols("mod tests {\n    #[test]\n    fn it_works() {}\n}\n");
        let mod_sym = syms.iter().find(|s| s.kind == SymbolKind::Module);
        assert!(mod_sym.is_some());
        assert_eq!(mod_sym.map(|s| s.name.as_str()), Some("tests"));
    }

    #[test]
    fn extract_mod_declaration_symbol() {
        let lang = RustLanguageProvider;
        let syms = lang.extract_symbols("pub mod config;\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "config");
        assert_eq!(syms[0].kind, SymbolKind::Module);
        assert_eq!(syms[0].visibility, Visibility::Public);
    }

    #[test]
    fn extract_multiple_symbols() {
        let lang = RustLanguageProvider;
        let src = "\
pub struct Config {
    name: String,
}

impl Config {
    pub fn new() -> Self {
        Self { name: String::new() }
    }
}

enum Status {
    Active,
    Inactive,
}

const DEFAULT_TIMEOUT: u64 = 30;
";
        let syms = lang.extract_symbols(src);
        let kinds: Vec<&SymbolKind> = syms.iter().map(|s| &s.kind).collect();
        assert!(kinds.contains(&&SymbolKind::Struct));
        assert!(kinds.contains(&&SymbolKind::Impl));
        assert!(kinds.contains(&&SymbolKind::Function));
        assert!(kinds.contains(&&SymbolKind::Enum));
        assert!(kinds.contains(&&SymbolKind::Const));
    }

    #[test]
    fn skips_comments_and_attributes() {
        let lang = RustLanguageProvider;
        let src = "\
// fn not_a_function() {}
/// Doc comment
#[derive(Debug)]
pub struct Real {}
";
        let syms = lang.extract_symbols(src);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Real");
    }

    #[test]
    fn extract_unsafe_fn() {
        let lang = RustLanguageProvider;
        let syms = lang.extract_symbols("unsafe fn danger() {}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "danger");
        assert_eq!(syms[0].kind, SymbolKind::Function);
    }

    #[test]
    fn line_numbers_are_correct() {
        let lang = RustLanguageProvider;
        let src = "\n\nfn third_line() {}\n";
        let syms = lang.extract_symbols(src);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].line, 3);
    }

    // ── Helper unit tests ────────────────────────────────────────────────

    #[test]
    fn extract_identifier_basic() {
        assert_eq!(extract_identifier("hello_world("), "hello_world");
        assert_eq!(extract_identifier("Foo<T>"), "Foo");
        assert_eq!(extract_identifier(""), "");
    }

    #[test]
    fn skip_angle_brackets_basic() {
        assert_eq!(skip_angle_brackets("<T, U> rest"), " rest");
        assert_eq!(skip_angle_brackets("no_angles"), "no_angles");
        assert_eq!(skip_angle_brackets("<A<B>> rest"), " rest");
    }

    #[test]
    fn strip_visibility_prefix_variants() {
        assert_eq!(strip_visibility_prefix("pub fn foo()"), "fn foo()");
        assert_eq!(strip_visibility_prefix("pub(crate) fn foo()"), "fn foo()");
        assert_eq!(strip_visibility_prefix("fn foo()"), "fn foo()");
    }
}
