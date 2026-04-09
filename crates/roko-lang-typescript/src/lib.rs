//! TypeScript/JavaScript implementations of [`roko_core::BuildSystem`] and
//! [`roko_core::LanguageProvider`].
//!
//! Build systems:
//! - [`NpmBuildSystem`]: produces `npm run build`, `npm test`, `npx eslint .`,
//!   `npx prettier --check .` commands.
//! - [`PnpmBuildSystem`]: same commands using `pnpm` instead of `npm`.
//! - [`YarnBuildSystem`]: same commands using `yarn` instead of `npm`.
//!
//! Language provider:
//! - [`TypeScriptLanguageProvider`]: parses ES module imports (`import ... from`,
//!   `import '...'`), `CommonJS` `require('...')` calls, and extracts `function`,
//!   `class`, `interface`, `type`, `const`, `enum`, and `export default` symbols.

#![allow(clippy::module_name_repetitions)]
#![allow(clippy::unnecessary_literal_bound)]

use roko_core::build::{BuildCommand, BuildSystem};
use roko_core::language::{Import, ImportKind, LanguageProvider, Symbol, SymbolKind, Visibility};
use std::path::Path;

// ─── NpmBuildSystem ─────────────────────────────────────────────────────

/// Build system implementation for npm (Node Package Manager).
pub struct NpmBuildSystem;

impl BuildSystem for NpmBuildSystem {
    fn name(&self) -> &str {
        "npm"
    }

    fn compile_cmd(&self, target_dir: &Path) -> BuildCommand {
        BuildCommand::new("npm")
            .args(["run", "build"])
            .working_dir(target_dir)
    }

    fn test_cmd(&self, target_dir: &Path, filter: Option<&str>) -> BuildCommand {
        let mut cmd = BuildCommand::new("npm").arg("test").working_dir(target_dir);
        if let Some(f) = filter {
            cmd = cmd.arg("--").arg(f);
        }
        cmd
    }

    fn lint_cmd(&self, target_dir: &Path) -> BuildCommand {
        BuildCommand::new("npx")
            .args(["eslint", "."])
            .working_dir(target_dir)
    }

    fn format_cmd(&self, target_dir: &Path, check_only: bool) -> BuildCommand {
        let mut cmd = BuildCommand::new("npx")
            .args(["prettier"])
            .working_dir(target_dir);
        if check_only {
            cmd = cmd.arg("--check").arg(".");
        } else {
            cmd = cmd.arg("--write").arg(".");
        }
        cmd
    }

    fn detect_from_files(&self, file_names: &[&str]) -> bool {
        file_names.contains(&"package.json")
            && !file_names.contains(&"pnpm-lock.yaml")
            && !file_names.contains(&"yarn.lock")
    }
}

// ─── PnpmBuildSystem ────────────────────────────────────────────────────

/// Build system implementation for pnpm.
pub struct PnpmBuildSystem;

impl BuildSystem for PnpmBuildSystem {
    fn name(&self) -> &str {
        "pnpm"
    }

    fn compile_cmd(&self, target_dir: &Path) -> BuildCommand {
        BuildCommand::new("pnpm")
            .args(["run", "build"])
            .working_dir(target_dir)
    }

    fn test_cmd(&self, target_dir: &Path, filter: Option<&str>) -> BuildCommand {
        let mut cmd = BuildCommand::new("pnpm")
            .arg("test")
            .working_dir(target_dir);
        if let Some(f) = filter {
            cmd = cmd.arg("--").arg(f);
        }
        cmd
    }

    fn lint_cmd(&self, target_dir: &Path) -> BuildCommand {
        BuildCommand::new("pnpm")
            .args(["exec", "eslint", "."])
            .working_dir(target_dir)
    }

    fn format_cmd(&self, target_dir: &Path, check_only: bool) -> BuildCommand {
        let mut cmd = BuildCommand::new("pnpm")
            .args(["exec", "prettier"])
            .working_dir(target_dir);
        if check_only {
            cmd = cmd.arg("--check").arg(".");
        } else {
            cmd = cmd.arg("--write").arg(".");
        }
        cmd
    }

    fn detect_from_files(&self, file_names: &[&str]) -> bool {
        file_names.contains(&"package.json") && file_names.contains(&"pnpm-lock.yaml")
    }
}

// ─── YarnBuildSystem ────────────────────────────────────────────────────

/// Build system implementation for Yarn.
pub struct YarnBuildSystem;

impl BuildSystem for YarnBuildSystem {
    fn name(&self) -> &str {
        "yarn"
    }

    fn compile_cmd(&self, target_dir: &Path) -> BuildCommand {
        BuildCommand::new("yarn")
            .args(["run", "build"])
            .working_dir(target_dir)
    }

    fn test_cmd(&self, target_dir: &Path, filter: Option<&str>) -> BuildCommand {
        let mut cmd = BuildCommand::new("yarn")
            .arg("test")
            .working_dir(target_dir);
        if let Some(f) = filter {
            cmd = cmd.arg("--").arg(f);
        }
        cmd
    }

    fn lint_cmd(&self, target_dir: &Path) -> BuildCommand {
        BuildCommand::new("yarn")
            .args(["run", "eslint", "."])
            .working_dir(target_dir)
    }

    fn format_cmd(&self, target_dir: &Path, check_only: bool) -> BuildCommand {
        let mut cmd = BuildCommand::new("yarn")
            .args(["run", "prettier"])
            .working_dir(target_dir);
        if check_only {
            cmd = cmd.arg("--check").arg(".");
        } else {
            cmd = cmd.arg("--write").arg(".");
        }
        cmd
    }

    fn detect_from_files(&self, file_names: &[&str]) -> bool {
        file_names.contains(&"package.json") && file_names.contains(&"yarn.lock")
    }
}

// ─── TypeScriptLanguageProvider ─────────────────────────────────────────

/// Language provider for `TypeScript` and `JavaScript` source files.
///
/// Uses line-by-line heuristic parsing to extract imports and symbol
/// definitions. Handles ES module `import` statements, `CommonJS` `require()`
/// calls, and top-level declarations (`function`, `class`, `interface`,
/// `type`, `const`, `enum`, `export default`).
pub struct TypeScriptLanguageProvider;

impl LanguageProvider for TypeScriptLanguageProvider {
    fn language_name(&self) -> &str {
        "typescript"
    }

    fn file_extensions(&self) -> &[&str] {
        &["ts", "tsx", "js", "jsx"]
    }

    fn parse_imports(&self, source: &str) -> Vec<Import> {
        let mut imports = Vec::new();
        for line in source.lines() {
            let trimmed = line.trim();
            if let Some(imp) = parse_es_import(trimmed) {
                imports.push(imp);
            } else if let Some(imp) = parse_require(trimmed) {
                imports.push(imp);
            }
        }
        imports
    }

    fn extract_symbols(&self, source: &str) -> Vec<Symbol> {
        let mut symbols = Vec::new();
        for (line_idx, line) in source.lines().enumerate() {
            let line_num = line_idx + 1;
            if let Some(sym) = extract_ts_symbol(line, line_num) {
                symbols.push(sym);
            }
        }
        symbols
    }
}

// ─── Import parsing helpers ─────────────────────────────────────────────

/// Extract the string inside matching quotes (single, double, or backtick).
fn extract_quoted_string(s: &str) -> Option<&str> {
    let s = s.trim();
    let quote = s.chars().next()?;
    if quote != '\'' && quote != '"' && quote != '`' {
        return None;
    }
    let inner = &s[1..];
    let end = inner.find(quote)?;
    Some(&inner[..end])
}

/// Parse an ES module import statement.
///
/// Handles:
/// - `import foo from 'module'`
/// - `import { foo } from 'module'`
/// - `import * as foo from 'module'`
/// - `import 'module'` (side-effect import)
/// - `import type { Foo } from 'module'`
fn parse_es_import(trimmed: &str) -> Option<Import> {
    let rest = trimmed.strip_prefix("import ")?;

    // Side-effect import: `import 'module';`
    let rest_no_semi = rest.trim_end_matches(';').trim();
    if rest_no_semi.starts_with('\'')
        || rest_no_semi.starts_with('"')
        || rest_no_semi.starts_with('`')
    {
        let path = extract_quoted_string(rest_no_semi)?;
        return Some(Import {
            path: path.to_string(),
            alias: None,
            kind: ImportKind::Use,
        });
    }

    // Find `from` keyword and extract the module path after it.
    let from_idx = find_from_keyword(rest)?;
    let after_from = rest[from_idx + 4..].trim().trim_end_matches(';').trim();
    let path = extract_quoted_string(after_from)?;

    // Extract alias: default import name or `* as name`.
    let before_from = rest[..from_idx].trim();
    let alias = extract_import_alias(before_from);

    Some(Import {
        path: path.to_string(),
        alias,
        kind: ImportKind::Use,
    })
}

/// Find the position of the `from` keyword that precedes the module specifier.
/// Must be preceded by whitespace or `}` and followed by whitespace.
fn find_from_keyword(s: &str) -> Option<usize> {
    let mut search_start = 0;
    loop {
        let pos = s[search_start..].find("from")?;
        let abs_pos = search_start + pos;

        // Check that `from` is preceded by start-of-string or whitespace/`}`
        let valid_before = abs_pos == 0 || {
            let prev = s.as_bytes()[abs_pos - 1];
            prev == b' ' || prev == b'\t' || prev == b'}'
        };

        // Check that `from` is followed by whitespace
        let valid_after = abs_pos + 4 < s.len() && {
            let next = s.as_bytes()[abs_pos + 4];
            next == b' ' || next == b'\t'
        };

        if valid_before && valid_after {
            return Some(abs_pos);
        }

        search_start = abs_pos + 1;
        if search_start >= s.len() {
            return None;
        }
    }
}

/// Extract the alias from the import clause (before `from`).
///
/// - `foo` -> Some("foo")
/// - `* as foo` -> Some("foo")
/// - `{ foo, bar }` -> None (named imports, no single alias)
/// - `type { Foo }` -> None
fn extract_import_alias(before_from: &str) -> Option<String> {
    let s = before_from.trim();

    // Skip `type` keyword for `import type`
    let s = s.strip_prefix("type ").map_or(s, |r| r.trim());

    // `* as name`
    if let Some(rest) = s.strip_prefix("* as ") {
        let name = rest.trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }

    // Named imports `{ ... }` — no single alias
    if s.starts_with('{') {
        return None;
    }

    // Default import: `foo` or `Foo` (possibly followed by `, { ... }`)
    let name = s.split([',', ' ']).next()?.trim();
    if !name.is_empty() && !name.starts_with('{') {
        Some(name.to_string())
    } else {
        None
    }
}

/// Parse a `CommonJS` `require('...')` call.
///
/// Handles:
/// - `const foo = require('module')`
/// - `const { foo } = require('module')`
/// - `require('module')` (bare)
fn parse_require(trimmed: &str) -> Option<Import> {
    // Find `require(` in the line
    let req_idx = trimmed.find("require(")?;
    let after_req = &trimmed[req_idx + 8..]; // skip "require("
    let close_paren = after_req.find(')')?;
    let inside = after_req[..close_paren].trim();
    let path = extract_quoted_string(inside)?;

    // Try to extract alias from `const/let/var name = require(...)`
    let before_req = trimmed[..req_idx].trim();
    let alias = extract_require_alias(before_req);

    Some(Import {
        path: path.to_string(),
        alias,
        kind: ImportKind::Use,
    })
}

/// Extract the variable name from the left side of a require assignment.
fn extract_require_alias(before: &str) -> Option<String> {
    // Strip trailing `=`
    let before = before.trim_end_matches('=').trim();

    // Strip leading `const`, `let`, `var`
    let rest = before
        .strip_prefix("const ")
        .or_else(|| before.strip_prefix("let "))
        .or_else(|| before.strip_prefix("var "))?
        .trim();

    // Skip destructuring `{ ... }`
    if rest.starts_with('{') {
        return None;
    }

    let name = rest.split_whitespace().next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

// ─── Symbol extraction helpers ──────────────────────────────────────────

/// Try to extract a symbol definition from a single TypeScript/JavaScript line.
fn extract_ts_symbol(line: &str, line_num: usize) -> Option<Symbol> {
    let trimmed = line.trim();

    // Skip comments.
    if trimmed.starts_with("//")
        || trimmed.starts_with('*')
        || trimmed.starts_with("/*")
        || trimmed.is_empty()
    {
        return None;
    }

    // Determine export/visibility and strip decorators.
    let (vis, rest) = parse_ts_visibility(trimmed);

    // Try each symbol kind in order.
    if let Some(name) = try_extract_ts_function(rest) {
        return Some(Symbol {
            name,
            kind: SymbolKind::Function,
            visibility: vis,
            line: line_num,
        });
    }
    if let Some(name) = try_extract_ts_class(rest) {
        return Some(Symbol {
            name,
            kind: SymbolKind::Struct,
            visibility: vis,
            line: line_num,
        });
    }
    if let Some(name) = try_extract_ts_interface(rest) {
        return Some(Symbol {
            name,
            kind: SymbolKind::Trait,
            visibility: vis,
            line: line_num,
        });
    }
    if let Some(name) = try_extract_ts_type_alias(rest) {
        return Some(Symbol {
            name,
            kind: SymbolKind::Type,
            visibility: vis,
            line: line_num,
        });
    }
    if let Some(name) = try_extract_ts_const(rest) {
        return Some(Symbol {
            name,
            kind: SymbolKind::Const,
            visibility: vis,
            line: line_num,
        });
    }
    if let Some(name) = try_extract_ts_enum(rest) {
        return Some(Symbol {
            name,
            kind: SymbolKind::Enum,
            visibility: vis,
            line: line_num,
        });
    }
    // `export default` as a symbol
    if let Some(name) = try_extract_export_default(trimmed) {
        return Some(Symbol {
            name,
            kind: SymbolKind::Module,
            visibility: Visibility::Public,
            line: line_num,
        });
    }

    None
}

/// Parse leading `export` / `export declare` / `declare` visibility prefix.
fn parse_ts_visibility(s: &str) -> (Visibility, &str) {
    let rest = s.trim_start();

    if let Some(after_export) = rest.strip_prefix("export ") {
        let after_export = after_export.trim_start();
        // `export declare`
        if let Some(after_declare) = after_export.strip_prefix("declare ") {
            return (Visibility::Public, after_declare.trim_start());
        }
        // `export default` is handled separately
        if after_export.starts_with("default ") {
            return (Visibility::Public, rest);
        }
        (Visibility::Public, after_export)
    } else if let Some(after_declare) = rest.strip_prefix("declare ") {
        (Visibility::Private, after_declare.trim_start())
    } else {
        (Visibility::Private, rest)
    }
}

/// Extract function name: `function foo(` or `async function foo(`.
fn try_extract_ts_function(s: &str) -> Option<String> {
    let rest = s.strip_prefix("async ").unwrap_or(s);
    let rest = rest.strip_prefix("function ")?;
    // Skip `*` for generators
    let rest = rest.strip_prefix('*').unwrap_or(rest).trim_start();
    Some(extract_ts_identifier(rest))
}

/// Extract class name: `class Foo` or `abstract class Foo`.
fn try_extract_ts_class(s: &str) -> Option<String> {
    let rest = s.strip_prefix("abstract ").unwrap_or(s);
    let rest = rest.strip_prefix("class ")?;
    Some(extract_ts_identifier(rest))
}

/// Extract interface name: `interface Foo`.
fn try_extract_ts_interface(s: &str) -> Option<String> {
    let rest = s.strip_prefix("interface ")?;
    Some(extract_ts_identifier(rest))
}

/// Extract type alias name: `type Foo = ...`.
fn try_extract_ts_type_alias(s: &str) -> Option<String> {
    let rest = s.strip_prefix("type ")?;
    let name = extract_ts_identifier(rest);
    if name.is_empty() {
        return None;
    }
    // Must be followed by `=` or `<` (generic)
    let after = rest[name.len()..].trim_start();
    if after.starts_with('=') || after.starts_with('<') {
        Some(name)
    } else {
        None
    }
}

/// Extract const name: `const FOO = ...` or `const FOO: Type = ...`.
fn try_extract_ts_const(s: &str) -> Option<String> {
    let rest = s.strip_prefix("const ")?;
    // Skip destructuring
    if rest.starts_with('{') || rest.starts_with('[') {
        return None;
    }
    let name = extract_ts_identifier(rest);
    if name.is_empty() {
        return None;
    }
    let after = rest[name.len()..].trim_start();
    if after.starts_with('=') || after.starts_with(':') {
        Some(name)
    } else {
        None
    }
}

/// Extract enum name: `enum Foo` or `const enum Foo`.
fn try_extract_ts_enum(s: &str) -> Option<String> {
    let rest = s.strip_prefix("const ").unwrap_or(s);
    let rest = rest.strip_prefix("enum ")?;
    Some(extract_ts_identifier(rest))
}

/// Detect `export default` expressions.
fn try_extract_export_default(s: &str) -> Option<String> {
    let rest = s.strip_prefix("export default ")?;
    let rest = rest.trim();

    // `export default class Foo` or `export default function foo` handled by
    // the main symbol extractor; here we handle `export default Foo;`
    if rest.starts_with("class ")
        || rest.starts_with("abstract class ")
        || rest.starts_with("function ")
        || rest.starts_with("async function ")
    {
        return None;
    }

    // Bare identifier export: `export default Foo;`
    let name = rest.trim_end_matches(';').trim();
    if name.is_empty() || name.contains(' ') {
        return None;
    }
    Some(name.to_string())
}

/// Extract a JavaScript/TypeScript identifier from the start of a string.
fn extract_ts_identifier(s: &str) -> String {
    s.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
        .collect()
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Build system tests ──────────────────────────────────────────────

    #[test]
    fn npm_name_and_detect() {
        let bs = NpmBuildSystem;
        assert_eq!(bs.name(), "npm");
        assert!(bs.detect_from_files(&["package.json", "src"]));
        // Should not detect when pnpm or yarn lock files are present.
        assert!(!bs.detect_from_files(&["package.json", "pnpm-lock.yaml"]));
        assert!(!bs.detect_from_files(&["package.json", "yarn.lock"]));
        assert!(!bs.detect_from_files(&["README.md"]));
    }

    #[test]
    fn npm_compile_cmd() {
        let bs = NpmBuildSystem;
        let cmd = bs.compile_cmd(Path::new("/proj"));
        assert_eq!(cmd.program, "npm");
        assert_eq!(cmd.args, vec!["run", "build"]);
        assert_eq!(cmd.working_dir, Some(std::path::PathBuf::from("/proj")));
    }

    #[test]
    fn npm_test_cmd_with_and_without_filter() {
        let bs = NpmBuildSystem;
        let cmd = bs.test_cmd(Path::new("/proj"), None);
        assert_eq!(cmd.program, "npm");
        assert_eq!(cmd.args, vec!["test"]);

        let cmd = bs.test_cmd(Path::new("/proj"), Some("my_test"));
        assert!(cmd.args.contains(&"--".to_string()));
        assert!(cmd.args.contains(&"my_test".to_string()));
    }

    #[test]
    fn npm_lint_and_format_cmds() {
        let bs = NpmBuildSystem;
        let lint = bs.lint_cmd(Path::new("/proj"));
        assert_eq!(lint.program, "npx");
        assert!(lint.args.contains(&"eslint".to_string()));

        let fmt_check = bs.format_cmd(Path::new("/proj"), true);
        assert_eq!(fmt_check.program, "npx");
        assert!(fmt_check.args.contains(&"--check".to_string()));

        let fmt_write = bs.format_cmd(Path::new("/proj"), false);
        assert!(fmt_write.args.contains(&"--write".to_string()));
        assert!(!fmt_write.args.contains(&"--check".to_string()));
    }

    #[test]
    fn pnpm_name_and_detect() {
        let bs = PnpmBuildSystem;
        assert_eq!(bs.name(), "pnpm");
        assert!(bs.detect_from_files(&["package.json", "pnpm-lock.yaml"]));
        assert!(!bs.detect_from_files(&["package.json"]));
    }

    #[test]
    fn pnpm_compile_and_test() {
        let bs = PnpmBuildSystem;
        let cmd = bs.compile_cmd(Path::new("/proj"));
        assert_eq!(cmd.program, "pnpm");
        assert_eq!(cmd.args, vec!["run", "build"]);

        let cmd = bs.test_cmd(Path::new("/proj"), Some("filter"));
        assert_eq!(cmd.program, "pnpm");
        assert!(cmd.args.contains(&"filter".to_string()));
    }

    #[test]
    fn yarn_name_and_detect() {
        let bs = YarnBuildSystem;
        assert_eq!(bs.name(), "yarn");
        assert!(bs.detect_from_files(&["package.json", "yarn.lock"]));
        assert!(!bs.detect_from_files(&["package.json"]));
    }

    #[test]
    fn yarn_compile_and_lint() {
        let bs = YarnBuildSystem;
        let cmd = bs.compile_cmd(Path::new("/proj"));
        assert_eq!(cmd.program, "yarn");
        assert_eq!(cmd.args, vec!["run", "build"]);

        let lint = bs.lint_cmd(Path::new("/proj"));
        assert_eq!(lint.program, "yarn");
        assert!(lint.args.contains(&"eslint".to_string()));
    }

    // ── Language provider — metadata ────────────────────────────────────

    #[test]
    fn ts_lang_metadata() {
        let lang = TypeScriptLanguageProvider;
        assert_eq!(lang.language_name(), "typescript");
        assert_eq!(lang.file_extensions(), &["ts", "tsx", "js", "jsx"]);
    }

    // ── Import parsing ──────────────────────────────────────────────────

    #[test]
    fn parse_es_import_default() {
        let lang = TypeScriptLanguageProvider;
        let imports = lang.parse_imports("import React from 'react';\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].path, "react");
        assert_eq!(imports[0].alias.as_deref(), Some("React"));
        assert_eq!(imports[0].kind, ImportKind::Use);
    }

    #[test]
    fn parse_es_import_named() {
        let lang = TypeScriptLanguageProvider;
        let imports = lang.parse_imports("import { useState, useEffect } from 'react';\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].path, "react");
        assert!(imports[0].alias.is_none()); // named imports have no single alias
    }

    #[test]
    fn parse_es_import_star() {
        let lang = TypeScriptLanguageProvider;
        let imports = lang.parse_imports("import * as path from 'path';\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].path, "path");
        assert_eq!(imports[0].alias.as_deref(), Some("path"));
    }

    #[test]
    fn parse_side_effect_import() {
        let lang = TypeScriptLanguageProvider;
        let imports = lang.parse_imports("import './styles.css';\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].path, "./styles.css");
        assert!(imports[0].alias.is_none());
    }

    #[test]
    fn parse_require_call() {
        let lang = TypeScriptLanguageProvider;
        let imports = lang.parse_imports("const express = require('express');\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].path, "express");
        assert_eq!(imports[0].alias.as_deref(), Some("express"));
    }

    #[test]
    fn parse_require_destructured() {
        let lang = TypeScriptLanguageProvider;
        let imports = lang.parse_imports("const { readFile } = require('fs');\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].path, "fs");
        assert!(imports[0].alias.is_none()); // destructured, no single alias
    }

    #[test]
    fn parse_import_type() {
        let lang = TypeScriptLanguageProvider;
        let imports = lang.parse_imports("import type { Config } from './config';\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].path, "./config");
    }

    #[test]
    fn parse_mixed_imports() {
        let lang = TypeScriptLanguageProvider;
        let src = "\
import React from 'react';
import { useState } from 'react';
import './global.css';
const fs = require('fs');
// not an import
function hello() {}
";
        let imports = lang.parse_imports(src);
        assert_eq!(imports.len(), 4);
    }

    // ── Symbol extraction ───────────────────────────────────────────────

    #[test]
    fn extract_function_declaration() {
        let lang = TypeScriptLanguageProvider;
        let syms = lang.extract_symbols("function hello() {\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "hello");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[0].visibility, Visibility::Private);
    }

    #[test]
    fn extract_exported_function() {
        let lang = TypeScriptLanguageProvider;
        let syms = lang.extract_symbols("export function greet(name: string): string {\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "greet");
        assert_eq!(syms[0].visibility, Visibility::Public);
    }

    #[test]
    fn extract_async_function() {
        let lang = TypeScriptLanguageProvider;
        let syms = lang.extract_symbols("export async function fetchData() {\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "fetchData");
        assert_eq!(syms[0].kind, SymbolKind::Function);
    }

    #[test]
    fn extract_class() {
        let lang = TypeScriptLanguageProvider;
        let syms = lang.extract_symbols("export class UserService {\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "UserService");
        assert_eq!(syms[0].kind, SymbolKind::Struct); // classes map to Struct
        assert_eq!(syms[0].visibility, Visibility::Public);
    }

    #[test]
    fn extract_interface() {
        let lang = TypeScriptLanguageProvider;
        let syms = lang.extract_symbols("export interface Config {\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Config");
        assert_eq!(syms[0].kind, SymbolKind::Trait); // interfaces map to Trait
    }

    #[test]
    fn extract_type_alias() {
        let lang = TypeScriptLanguageProvider;
        let syms = lang.extract_symbols("export type Result<T> = Success<T> | Error;\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Result");
        assert_eq!(syms[0].kind, SymbolKind::Type);
    }

    #[test]
    fn extract_const() {
        let lang = TypeScriptLanguageProvider;
        let syms = lang.extract_symbols("export const MAX_RETRIES = 3;\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "MAX_RETRIES");
        assert_eq!(syms[0].kind, SymbolKind::Const);
    }

    #[test]
    fn extract_enum() {
        let lang = TypeScriptLanguageProvider;
        let syms = lang.extract_symbols("export enum Direction {\n  Up,\n  Down,\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Direction");
        assert_eq!(syms[0].kind, SymbolKind::Enum);
    }

    #[test]
    fn extract_export_default() {
        let lang = TypeScriptLanguageProvider;
        let syms = lang.extract_symbols("export default App;\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "App");
        assert_eq!(syms[0].kind, SymbolKind::Module);
        assert_eq!(syms[0].visibility, Visibility::Public);
    }

    #[test]
    fn extract_multiple_symbols() {
        let lang = TypeScriptLanguageProvider;
        let src = "\
export interface User {
  name: string;
}

export class UserService {
  getUser(): User { return { name: '' }; }
}

export const VERSION = '1.0';

export function init() {}

enum Status {
  Active,
}
";
        let syms = lang.extract_symbols(src);
        let kinds: Vec<&SymbolKind> = syms.iter().map(|s| &s.kind).collect();
        assert!(kinds.contains(&&SymbolKind::Trait)); // interface
        assert!(kinds.contains(&&SymbolKind::Struct)); // class
        assert!(kinds.contains(&&SymbolKind::Const));
        assert!(kinds.contains(&&SymbolKind::Function));
        assert!(kinds.contains(&&SymbolKind::Enum));
    }

    #[test]
    fn skips_comments() {
        let lang = TypeScriptLanguageProvider;
        let src = "\
// function notAFunction() {}
/* block comment */
export function real() {}
";
        let syms = lang.extract_symbols(src);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "real");
    }

    #[test]
    fn line_numbers_correct() {
        let lang = TypeScriptLanguageProvider;
        let src = "\n\nfunction third() {}\n";
        let syms = lang.extract_symbols(src);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].line, 3);
    }

    // ── Helper unit tests ───────────────────────────────────────────────

    #[test]
    fn extract_ts_identifier_basic() {
        assert_eq!(extract_ts_identifier("hello_world("), "hello_world");
        assert_eq!(extract_ts_identifier("$foo.bar"), "$foo");
        assert_eq!(extract_ts_identifier(""), "");
    }

    #[test]
    fn extract_quoted_string_variants() {
        assert_eq!(extract_quoted_string("'hello'"), Some("hello"));
        assert_eq!(extract_quoted_string("\"world\""), Some("world"));
        assert_eq!(extract_quoted_string("`tick`"), Some("tick"));
        assert_eq!(extract_quoted_string("no quotes"), None);
    }
}
