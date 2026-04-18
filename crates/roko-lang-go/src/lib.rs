//! Go implementations of [`roko_core::BuildSystem`] and
//! [`roko_core::LanguageProvider`].
//!
//! - [`GoBuildSystem`]: produces `go build ./...`, `go test ./...`,
//!   `go vet ./...`, `gofmt -l .` commands as [`BuildCommand`] descriptors.
//! - [`GoLanguageProvider`]: parses single and grouped `import` statements and
//!   extracts `func`, `type ... struct`, `type ... interface`, `const`, and
//!   `var` symbols from Go source text, including grouped `const`/`var` blocks.

#![allow(clippy::module_name_repetitions)]
#![allow(clippy::unnecessary_literal_bound)]

use roko_core::build::{BuildCommand, BuildSystem};
use roko_core::language::{Import, ImportKind, LanguageProvider, Symbol, SymbolKind, Visibility};
use std::path::Path;

// ─── GoBuildSystem ──────────────────────────────────────────────────────

/// Build system implementation for Go.
pub struct GoBuildSystem;

impl BuildSystem for GoBuildSystem {
    fn name(&self) -> &str {
        "go"
    }

    fn compile_cmd(&self, target_dir: &Path) -> BuildCommand {
        BuildCommand::new("go")
            .args(["build", "./..."])
            .working_dir(target_dir)
    }

    fn test_cmd(&self, target_dir: &Path, filter: Option<&str>) -> BuildCommand {
        let mut cmd = BuildCommand::new("go")
            .args(["test", "./..."])
            .working_dir(target_dir);
        if let Some(f) = filter {
            cmd = cmd.arg("-run").arg(f);
        }
        cmd
    }

    fn lint_cmd(&self, target_dir: &Path) -> BuildCommand {
        BuildCommand::new("go")
            .args(["vet", "./..."])
            .working_dir(target_dir)
    }

    fn format_cmd(&self, target_dir: &Path, check_only: bool) -> BuildCommand {
        if check_only {
            BuildCommand::new("gofmt")
                .args(["-l", "."])
                .working_dir(target_dir)
        } else {
            BuildCommand::new("gofmt")
                .args(["-w", "."])
                .working_dir(target_dir)
        }
    }

    fn detect_from_files(&self, file_names: &[&str]) -> bool {
        file_names.contains(&"go.mod")
    }
}

// ─── GoLanguageProvider ─────────────────────────────────────────────────

/// Language provider for Go source files.
///
/// Uses line-by-line heuristic parsing to extract imports and symbol
/// definitions. Handles single imports (`import "pkg"`), grouped imports
/// (`import ( ... )`), top-level declarations (`func`, `type struct`,
/// `type interface`, `const`, `var`), and grouped `const`/`var` blocks.
pub struct GoLanguageProvider;

impl LanguageProvider for GoLanguageProvider {
    fn language_name(&self) -> &str {
        "go"
    }

    fn file_extensions(&self) -> &[&str] {
        &["go"]
    }

    fn parse_imports(&self, source: &str) -> Vec<Import> {
        let mut imports = Vec::new();
        let mut in_import_block = false;

        for line in source.lines() {
            let trimmed = line.trim();

            if in_import_block {
                if trimmed == ")" {
                    in_import_block = false;
                    continue;
                }
                if let Some(imp) = parse_go_import_line(trimmed) {
                    imports.push(imp);
                }
                continue;
            }

            // `import (` — start of grouped import block.
            if trimmed == "import (" {
                in_import_block = true;
                continue;
            }

            // Single-line import: `import "fmt"` or `import alias "pkg"`.
            if let Some(rest) = trimmed.strip_prefix("import ") {
                if let Some(imp) = parse_go_import_line(rest.trim()) {
                    imports.push(imp);
                }
            }
        }

        imports
    }

    fn extract_symbols(&self, source: &str) -> Vec<Symbol> {
        let mut symbols = Vec::new();
        let mut decl_group = None;

        for (line_idx, line) in source.lines().enumerate() {
            let line_num = line_idx + 1;

            if let Some(keyword) = decl_group {
                let trimmed = line.trim();
                if trimmed == ")" {
                    decl_group = None;
                    continue;
                }
                if let Some(sym) = extract_go_group_member(trimmed, line_num, keyword) {
                    symbols.push(sym);
                }
                continue;
            }

            let trimmed = line.trim();
            if is_go_decl_group_start(trimmed, "const") {
                decl_group = Some("const");
                continue;
            }
            if is_go_decl_group_start(trimmed, "var") {
                decl_group = Some("var");
                continue;
            }

            if let Some(sym) = extract_go_symbol(line, line_num) {
                symbols.push(sym);
            }
        }
        symbols
    }
}

// ─── Import parsing helpers ─────────────────────────────────────────────

/// Parse a single Go import line (inside or outside a group).
///
/// Handles:
/// - `"fmt"` (plain)
/// - `alias "pkg/path"` (aliased)
/// - `. "pkg"` (dot import)
/// - `_ "pkg"` (side-effect import)
fn parse_go_import_line(s: &str) -> Option<Import> {
    let s = s.trim();

    // Skip empty lines and comments inside import blocks.
    if s.is_empty() || s.starts_with("//") {
        return None;
    }

    // Try to find a quoted path.
    let quote_start = s.find('"')?;
    let after_quote = &s[quote_start + 1..];
    let quote_end = after_quote.find('"')?;
    let path = &after_quote[..quote_end];

    // Everything before the first quote is the optional alias.
    let before = s[..quote_start].trim();
    let alias = if before.is_empty() || before == "_" || before == "." {
        if before == "." || before == "_" {
            Some(before.to_string())
        } else {
            None
        }
    } else {
        Some(before.to_string())
    };

    Some(Import {
        path: path.to_string(),
        alias,
        kind: ImportKind::Use,
    })
}

// ─── Symbol extraction helpers ──────────────────────────────────────────

/// Try to extract a symbol definition from a single Go line.
fn extract_go_symbol(line: &str, line_num: usize) -> Option<Symbol> {
    let trimmed = line.trim();

    // Skip comments, blank lines, and indented lines (method bodies, etc.).
    if trimmed.starts_with("//")
        || trimmed.is_empty()
        || (line.starts_with('\t') || line.starts_with("    ")) && !trimmed.starts_with("func ")
    {
        // Indented `func` declarations can be methods — skip truly indented code.
        // But we do accept top-level lines that start at column 0.
    }

    // Only process lines that start at column 0 (top-level declarations).
    if !line.is_empty() && (line.starts_with(' ') || line.starts_with('\t')) {
        return None;
    }

    if trimmed.starts_with("//") || trimmed.is_empty() {
        return None;
    }

    // Determine visibility: Go uses capitalization.
    // We check after extracting the name.

    if let Some(sym) = try_extract_go_func(trimmed, line_num) {
        return Some(sym);
    }
    if let Some(sym) = try_extract_go_type(trimmed, line_num) {
        return Some(sym);
    }
    if let Some(sym) = try_extract_go_const_or_var(trimmed, line_num, "const") {
        return Some(sym);
    }
    if let Some(sym) = try_extract_go_const_or_var(trimmed, line_num, "var") {
        return Some(sym);
    }

    None
}

/// Determine Go visibility from a name. In Go, exported names start with an
/// uppercase letter.
fn go_visibility(name: &str) -> Visibility {
    name.chars().next().map_or(Visibility::Private, |c| {
        if c.is_uppercase() {
            Visibility::Public
        } else {
            Visibility::Private
        }
    })
}

/// Extract a Go function or method.
///
/// Handles:
/// - `func name(` — package-level function
/// - `func (r *Receiver) name(` — method
fn try_extract_go_func(trimmed: &str, line_num: usize) -> Option<Symbol> {
    let rest = trimmed.strip_prefix("func ")?;

    // Method with receiver: `(r *Receiver) Name(`
    let rest = if rest.starts_with('(') {
        // Skip past the receiver.
        let close_paren = rest.find(')')?;
        rest[close_paren + 1..].trim_start()
    } else {
        rest
    };

    let name = extract_go_identifier(rest);
    if name.is_empty() {
        return None;
    }

    Some(Symbol {
        visibility: go_visibility(&name),
        name,
        kind: SymbolKind::Function,
        line: line_num,
    })
}

/// Extract a Go type declaration.
///
/// Handles:
/// - `type Name struct {`
/// - `type Name interface {`
/// - `type Name = ...` (alias)
/// - `type Name OtherType` (named type)
fn try_extract_go_type(trimmed: &str, line_num: usize) -> Option<Symbol> {
    let rest = trimmed.strip_prefix("type ")?;
    let name = extract_go_identifier(rest);
    if name.is_empty() {
        return None;
    }

    let after_name = rest[name.len()..].trim_start();

    let kind = if after_name.starts_with("struct") {
        SymbolKind::Struct
    } else if after_name.starts_with("interface") {
        SymbolKind::Trait
    } else {
        SymbolKind::Type
    };

    Some(Symbol {
        visibility: go_visibility(&name),
        name,
        kind,
        line: line_num,
    })
}

/// Extract a `const` or `var` declaration.
///
/// Handles:
/// - `const Name = value`
/// - `const Name Type = value`
/// - `const (` — we treat this as a group opener and extract the keyword itself
/// - `var Name Type`
fn try_extract_go_const_or_var(trimmed: &str, line_num: usize, keyword: &str) -> Option<Symbol> {
    let prefix = format!("{keyword} ");
    let rest = trimmed.strip_prefix(&prefix)?;

    // Skip grouped block opener: `const (` / `var (`
    if rest.starts_with('(') {
        return None;
    }

    let name = extract_go_identifier(rest);
    if name.is_empty() {
        return None;
    }

    Some(Symbol {
        visibility: go_visibility(&name),
        name,
        kind: SymbolKind::Const,
        line: line_num,
    })
}

fn is_go_decl_group_start(trimmed: &str, keyword: &str) -> bool {
    let prefix = format!("{keyword} ");
    trimmed
        .strip_prefix(&prefix)
        .is_some_and(|rest| rest.trim_start().starts_with('('))
}

fn extract_go_group_member(trimmed: &str, line_num: usize, _keyword: &str) -> Option<Symbol> {
    if trimmed.is_empty() || trimmed.starts_with("//") {
        return None;
    }

    let name = extract_go_identifier(trimmed);
    if name.is_empty() {
        return None;
    }

    Some(Symbol {
        visibility: go_visibility(&name),
        name,
        kind: SymbolKind::Const,
        line: line_num,
    })
}

/// Extract a Go identifier from the start of a string.
fn extract_go_identifier(s: &str) -> String {
    s.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── GoBuildSystem tests ─────────────────────────────────────────────

    #[test]
    fn go_name_and_detect() {
        let bs = GoBuildSystem;
        assert_eq!(bs.name(), "go");
        assert!(bs.detect_from_files(&["go.mod", "main.go"]));
        assert!(!bs.detect_from_files(&["Cargo.toml", "src"]));
    }

    #[test]
    fn go_compile_cmd() {
        let bs = GoBuildSystem;
        let cmd = bs.compile_cmd(Path::new("/proj"));
        assert_eq!(cmd.program, "go");
        assert_eq!(cmd.args, vec!["build", "./..."]);
        assert_eq!(cmd.working_dir, Some(std::path::PathBuf::from("/proj")));
    }

    #[test]
    fn go_test_cmd_no_filter() {
        let bs = GoBuildSystem;
        let cmd = bs.test_cmd(Path::new("/proj"), None);
        assert_eq!(cmd.program, "go");
        assert_eq!(cmd.args, vec!["test", "./..."]);
    }

    #[test]
    fn go_test_cmd_with_filter() {
        let bs = GoBuildSystem;
        let cmd = bs.test_cmd(Path::new("/proj"), Some("TestFoo"));
        assert!(cmd.args.contains(&"-run".to_string()));
        assert!(cmd.args.contains(&"TestFoo".to_string()));
    }

    #[test]
    fn go_lint_cmd() {
        let bs = GoBuildSystem;
        let cmd = bs.lint_cmd(Path::new("/proj"));
        assert_eq!(cmd.program, "go");
        assert!(cmd.args.contains(&"vet".to_string()));
        assert!(cmd.args.contains(&"./...".to_string()));
    }

    #[test]
    fn go_format_check_and_write() {
        let bs = GoBuildSystem;
        let check = bs.format_cmd(Path::new("/proj"), true);
        assert_eq!(check.program, "gofmt");
        assert!(check.args.contains(&"-l".to_string()));

        let write = bs.format_cmd(Path::new("/proj"), false);
        assert_eq!(write.program, "gofmt");
        assert!(write.args.contains(&"-w".to_string()));
        assert!(!write.args.contains(&"-l".to_string()));
    }

    // ── GoLanguageProvider — metadata ───────────────────────────────────

    #[test]
    fn go_lang_metadata() {
        let lang = GoLanguageProvider;
        assert_eq!(lang.language_name(), "go");
        assert_eq!(lang.file_extensions(), &["go"]);
    }

    // ── Import parsing ──────────────────────────────────────────────────

    #[test]
    fn parse_single_import() {
        let lang = GoLanguageProvider;
        let imports = lang.parse_imports("import \"fmt\"\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].path, "fmt");
        assert!(imports[0].alias.is_none());
        assert_eq!(imports[0].kind, ImportKind::Use);
    }

    #[test]
    fn parse_aliased_import() {
        let lang = GoLanguageProvider;
        let imports = lang.parse_imports("import f \"fmt\"\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].path, "fmt");
        assert_eq!(imports[0].alias.as_deref(), Some("f"));
    }

    #[test]
    fn parse_grouped_imports() {
        let lang = GoLanguageProvider;
        let src = "\
import (
\t\"fmt\"
\t\"os\"
\tlog \"github.com/sirupsen/logrus\"
)
";
        let imports = lang.parse_imports(src);
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].path, "fmt");
        assert_eq!(imports[1].path, "os");
        assert_eq!(imports[2].path, "github.com/sirupsen/logrus");
        assert_eq!(imports[2].alias.as_deref(), Some("log"));
    }

    #[test]
    fn parse_dot_and_blank_imports() {
        let lang = GoLanguageProvider;
        let src = "\
import (
\t. \"testing\"
\t_ \"net/http/pprof\"
)
";
        let imports = lang.parse_imports(src);
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].path, "testing");
        assert_eq!(imports[0].alias.as_deref(), Some("."));
        assert_eq!(imports[1].path, "net/http/pprof");
        assert_eq!(imports[1].alias.as_deref(), Some("_"));
    }

    #[test]
    fn parse_no_imports() {
        let lang = GoLanguageProvider;
        let imports = lang.parse_imports("package main\n\nfunc main() {}\n");
        assert!(imports.is_empty());
    }

    // ── Symbol extraction ───────────────────────────────────────────────

    #[test]
    fn extract_exported_func() {
        let lang = GoLanguageProvider;
        let syms = lang.extract_symbols("func Hello() {\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Hello");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[0].visibility, Visibility::Public);
        assert_eq!(syms[0].line, 1);
    }

    #[test]
    fn extract_unexported_func() {
        let lang = GoLanguageProvider;
        let syms = lang.extract_symbols("func helper() error {\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "helper");
        assert_eq!(syms[0].visibility, Visibility::Private);
    }

    #[test]
    fn extract_method_with_receiver() {
        let lang = GoLanguageProvider;
        let syms = lang.extract_symbols("func (s *Server) Start() error {\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Start");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[0].visibility, Visibility::Public);
    }

    #[test]
    fn extract_type_struct() {
        let lang = GoLanguageProvider;
        let syms = lang.extract_symbols("type Config struct {\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Config");
        assert_eq!(syms[0].kind, SymbolKind::Struct);
        assert_eq!(syms[0].visibility, Visibility::Public);
    }

    #[test]
    fn extract_type_interface() {
        let lang = GoLanguageProvider;
        let syms =
            lang.extract_symbols("type Reader interface {\n\tRead(p []byte) (int, error)\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Reader");
        assert_eq!(syms[0].kind, SymbolKind::Trait);
    }

    #[test]
    fn extract_const_and_var() {
        let lang = GoLanguageProvider;
        let src = "const MaxRetries = 3\nvar DefaultTimeout = 30\n";
        let syms = lang.extract_symbols(src);
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].name, "MaxRetries");
        assert_eq!(syms[0].kind, SymbolKind::Const);
        assert_eq!(syms[1].name, "DefaultTimeout");
        assert_eq!(syms[1].kind, SymbolKind::Const); // var also maps to Const
    }

    #[test]
    fn extract_grouped_const_and_var_blocks() {
        let lang = GoLanguageProvider;
        let src = "\
const (
\tMaxRetries = 3
\tdefaultTimeout = 30
)

var (
\tDebug = false
)
";
        let syms = lang.extract_symbols(src);
        assert_eq!(syms.len(), 3);
        assert_eq!(syms[0].name, "MaxRetries");
        assert_eq!(syms[0].visibility, Visibility::Public);
        assert_eq!(syms[1].name, "defaultTimeout");
        assert_eq!(syms[1].visibility, Visibility::Private);
        assert_eq!(syms[2].name, "Debug");
        assert_eq!(syms[2].kind, SymbolKind::Const);
    }

    #[test]
    fn extract_unexported_type() {
        let lang = GoLanguageProvider;
        let syms = lang.extract_symbols("type config struct {\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "config");
        assert_eq!(syms[0].visibility, Visibility::Private);
    }

    #[test]
    fn extract_multiple_symbols() {
        let lang = GoLanguageProvider;
        let src = "\
package main

func main() {}

type Server struct {
\taddr string
}

type Handler interface {
\tHandle()
}

const Version = \"1.0\"

var Debug = false
";
        let syms = lang.extract_symbols(src);
        let kinds: Vec<&SymbolKind> = syms.iter().map(|s| &s.kind).collect();
        assert!(kinds.contains(&&SymbolKind::Function));
        assert!(kinds.contains(&&SymbolKind::Struct));
        assert!(kinds.contains(&&SymbolKind::Trait));
        assert!(kinds.contains(&&SymbolKind::Const));
    }

    #[test]
    fn skips_comments_and_indented() {
        let lang = GoLanguageProvider;
        let src = "\
// func notAFunc() {}
func Real() {}
\tfunc indented() {}
";
        let syms = lang.extract_symbols(src);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Real");
    }

    #[test]
    fn line_numbers_correct() {
        let lang = GoLanguageProvider;
        let src = "\n\nfunc Third() {}\n";
        let syms = lang.extract_symbols(src);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].line, 3);
    }

    // ── Helper unit tests ───────────────────────────────────────────────

    #[test]
    fn go_identifier_extraction() {
        assert_eq!(extract_go_identifier("Hello("), "Hello");
        assert_eq!(extract_go_identifier("foo_bar "), "foo_bar");
        assert_eq!(extract_go_identifier(""), "");
    }

    #[test]
    fn go_visibility_check() {
        assert_eq!(go_visibility("Exported"), Visibility::Public);
        assert_eq!(go_visibility("private"), Visibility::Private);
        assert_eq!(go_visibility(""), Visibility::Private);
    }
}
