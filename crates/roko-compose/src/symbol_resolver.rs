//! Resolve symbol names to their Rust signatures by searching the workspace.
//!
//! Given symbol names like `"SystemPromptBuilder::new"` or `"TaskDef"`, this
//! module finds their definitions in the codebase and extracts just the
//! signature (struct fields, function signature, trait methods, etc.) — not the
//! full implementation. This gives agents the API surface without implementation
//! noise.
//!
//! # Strategy
//!
//! 1. **Grep-based** (always available): walks `*.rs` files under the workdir,
//!    grepping for `pub (fn|struct|enum|trait|type|const) {name}` patterns.
//! 2. **roko-index** (future): when the index is available, use pre-parsed ASTs
//!    for faster, more accurate resolution. Not wired yet.

use std::path::{Path, PathBuf};

/// A resolved symbol with its signature and source location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSymbol {
    /// The original symbol name that was searched for.
    pub symbol: String,
    /// The file where the symbol was found (relative to workdir).
    pub file: String,
    /// The line number of the definition.
    pub line: usize,
    /// The extracted signature (struct def, fn signature, etc.).
    pub signature: String,
    /// What kind of symbol this is.
    pub kind: SymbolKind,
}

/// The kind of Rust symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    /// A `struct` definition.
    Struct,
    /// An `enum` definition.
    Enum,
    /// A `trait` definition.
    Trait,
    /// A `fn` definition.
    Function,
    /// A `type` alias.
    Type,
    /// A `const` definition.
    Const,
    /// An `impl` block.
    Impl,
    /// Couldn't determine the symbol kind.
    Unknown,
}

/// Resolves symbol names to their definitions in a workspace.
pub struct SymbolResolver {
    workdir: PathBuf,
}

impl SymbolResolver {
    /// Create a new resolver rooted at `workdir`.
    #[must_use]
    pub const fn new(workdir: PathBuf) -> Self {
        Self { workdir }
    }

    /// Resolve a list of symbol names to their signatures.
    ///
    /// Symbols can be:
    /// - Simple names: `"TaskDef"`, `"AgentRole"`
    /// - Qualified: `"SystemPromptBuilder::new"`, `"TaskDef::build_prompt"`
    /// - Partial paths: `"task_parser::TaskDef"`
    ///
    /// Returns resolved symbols for each name found. Missing symbols are
    /// silently skipped.
    pub fn resolve_symbols(&self, names: &[String]) -> Vec<ResolvedSymbol> {
        let mut resolved = Vec::new();

        // Collect all .rs files under crates/ (skip target/)
        let rs_files = collect_rs_files(&self.workdir);

        for name in names {
            if let Some(sym) = self.resolve_one(name, &rs_files) {
                resolved.push(sym);
            }
        }

        resolved
    }

    /// Resolve a single symbol name.
    fn resolve_one(&self, name: &str, rs_files: &[PathBuf]) -> Option<ResolvedSymbol> {
        // Parse qualified names: "Type::method" → search for method in Type's impl
        let (type_name, method_name) = name.rfind("::").map_or((name, None), |pos| {
            let prefix = &name[..pos];
            let suffix = &name[pos + 2..];
            // If prefix contains :: (e.g., "module::Type::method"), take last two parts
            prefix.rfind("::").map_or(
                (prefix, Some(suffix)),
                |pos2| (&prefix[pos2 + 2..], Some(suffix)),
            )
        });

        for file_path in rs_files {
            let Ok(content) = std::fs::read_to_string(file_path) else { continue };

            let relative = file_path
                .strip_prefix(&self.workdir)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();

            // Skip target directory
            if relative.starts_with("target/") {
                continue;
            }

            if let Some(method) = method_name {
                // Looking for Type::method — find the impl block then the method
                if let Some(sym) = find_method_in_impl(&content, type_name, method, &relative, name)
                {
                    return Some(sym);
                }
            } else {
                // Looking for a type/function definition
                if let Some(sym) = find_definition(&content, type_name, &relative) {
                    return Some(sym);
                }
            }
        }

        None
    }
}

/// Collect all `.rs` files under workdir/crates/, skipping target/.
fn collect_rs_files(workdir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let crates_dir = workdir.join("crates");
    if crates_dir.is_dir() {
        collect_rs_recursive(&crates_dir, &mut files);
    }
    // Also check src/ at root level
    let src_dir = workdir.join("src");
    if src_dir.is_dir() {
        collect_rs_recursive(&src_dir, &mut files);
    }
    files
}

fn collect_rs_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip target, hidden dirs, build artifacts
        if name_str.starts_with('.') || name_str == "target" {
            continue;
        }

        if path.is_dir() {
            collect_rs_recursive(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

/// Find a top-level definition (struct, enum, trait, fn, type, const) in file content.
fn find_definition(content: &str, name: &str, file: &str) -> Option<ResolvedSymbol> {
    let lines: Vec<&str> = content.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // Match patterns like:
        // pub struct Name { ... }
        // pub enum Name { ... }
        // pub trait Name { ... }
        // pub fn name(...) -> ... { ... }
        // pub type Name = ...;
        // pub const NAME: ...;
        let (kind, found) = if matches_definition(trimmed, "struct", name) {
            (SymbolKind::Struct, true)
        } else if matches_definition(trimmed, "enum", name) {
            (SymbolKind::Enum, true)
        } else if matches_definition(trimmed, "trait", name) {
            (SymbolKind::Trait, true)
        } else if matches_fn_definition(trimmed, name) {
            (SymbolKind::Function, true)
        } else if matches_definition(trimmed, "type", name) {
            (SymbolKind::Type, true)
        } else if matches_definition(trimmed, "const", name) {
            (SymbolKind::Const, true)
        } else {
            (SymbolKind::Unknown, false)
        };

        if found {
            let signature = extract_signature(&lines, i, kind);
            return Some(ResolvedSymbol {
                symbol: name.to_string(),
                file: file.to_string(),
                line: i + 1,
                signature,
                kind,
            });
        }
    }

    None
}

/// Find a method within an impl block for a given type.
fn find_method_in_impl(
    content: &str,
    type_name: &str,
    method_name: &str,
    file: &str,
    full_symbol: &str,
) -> Option<ResolvedSymbol> {
    let lines: Vec<&str> = content.lines().collect();
    let mut in_impl = false;
    let mut brace_depth: i32 = 0;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // Check for impl blocks: `impl TypeName {` or `impl TypeName for TraitName {`
        if !in_impl
            && (trimmed.starts_with("impl ")
                || trimmed.starts_with("impl<"))
            && trimmed.contains(type_name)
            && (trimmed.contains('{') || lines.get(i + 1).is_some_and(|l| l.trim().starts_with('{')))
        {
            in_impl = true;
            brace_depth = 0;
            // Count braces on this line
            for ch in trimmed.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => brace_depth -= 1,
                    _ => {}
                }
            }
            continue;
        }

        if in_impl {
            // Track brace depth
            for ch in trimmed.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => brace_depth -= 1,
                    _ => {}
                }
            }

            // End of impl block
            if brace_depth <= 0 {
                in_impl = false;
                continue;
            }

            // Look for the method
            if matches_fn_definition(trimmed, method_name) {
                let signature = extract_fn_signature(&lines, i);
                return Some(ResolvedSymbol {
                    symbol: full_symbol.to_string(),
                    file: file.to_string(),
                    line: i + 1,
                    signature,
                    kind: SymbolKind::Function,
                });
            }
        }
    }

    None
}

/// Check if a line matches `pub [keyword] [name]`.
fn matches_definition(line: &str, keyword: &str, name: &str) -> bool {
    // Patterns: "pub struct Name", "pub(crate) struct Name", "struct Name"
    let patterns = [
        format!("pub {keyword} {name}"),
        format!("pub(crate) {keyword} {name}"),
        format!("pub(super) {keyword} {name}"),
        format!("{keyword} {name}"),
    ];

    for pat in &patterns {
        if line.starts_with(pat.as_str()) {
            // Make sure the name is followed by a word boundary
            let rest = &line[pat.len()..];
            if rest.is_empty()
                || rest.starts_with(|c: char| !c.is_alphanumeric() && c != '_')
            {
                return true;
            }
        }
    }

    false
}

/// Check if a line matches a function definition with the given name.
fn matches_fn_definition(line: &str, name: &str) -> bool {
    let patterns = [
        format!("pub fn {name}"),
        format!("pub(crate) fn {name}"),
        format!("pub(super) fn {name}"),
        format!("fn {name}"),
        format!("pub async fn {name}"),
        format!("pub(crate) async fn {name}"),
        format!("async fn {name}"),
        format!("pub const fn {name}"),
        format!("pub(crate) const fn {name}"),
        format!("const fn {name}"),
        format!("pub unsafe fn {name}"),
        format!("unsafe fn {name}"),
    ];

    for pat in &patterns {
        if line.starts_with(pat.as_str()) || line.contains(pat.as_str()) {
            let after = line.find(pat.as_str())
                .map_or("", |pos| &line[pos + pat.len()..]);
            if after.is_empty()
                || after.starts_with(|c: char| !c.is_alphanumeric() && c != '_')
            {
                return true;
            }
        }
    }

    false
}

/// Extract the full signature of a definition starting at line `start`.
///
/// For structs/enums: includes fields up to closing brace (max 30 lines).
/// For functions: includes up to the opening brace or semicolon.
fn extract_signature(lines: &[&str], start: usize, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Trait => {
            extract_block_signature(lines, start, 30)
        }
        SymbolKind::Function => extract_fn_signature(lines, start),
        SymbolKind::Type | SymbolKind::Const => {
            // Single line or up to semicolon
            let mut sig = String::new();
            for &line in lines.iter().skip(start).take(5) {
                sig.push_str(line);
                sig.push('\n');
                if line.contains(';') {
                    break;
                }
            }
            sig
        }
        SymbolKind::Impl | SymbolKind::Unknown => {
            lines.get(start).unwrap_or(&"").to_string()
        }
    }
}

/// Extract a block signature (struct/enum/trait) up to the closing brace.
/// Caps at `max_lines` to avoid dumping huge types.
fn extract_block_signature(lines: &[&str], start: usize, max_lines: usize) -> String {
    let mut sig = String::new();
    let mut brace_depth: i32 = 0;
    let mut found_open = false;

    for (i, &line) in lines.iter().skip(start).take(max_lines).enumerate() {
        sig.push_str(line);
        sig.push('\n');

        for ch in line.chars() {
            match ch {
                '{' => {
                    brace_depth += 1;
                    found_open = true;
                }
                '}' => brace_depth -= 1,
                _ => {}
            }
        }

        // Stop when we've closed the opening brace
        if found_open && brace_depth <= 0 {
            break;
        }

        // If it's a unit struct (no brace), stop at semicolon
        if !found_open && line.contains(';') {
            break;
        }

        // For tuple structs like `struct Foo(Bar);`
        if i == 0 && line.contains(')') && line.contains(';') {
            break;
        }
    }

    sig
}

/// Extract a function signature up to the opening brace (not the body).
fn extract_fn_signature(lines: &[&str], start: usize) -> String {
    let mut sig = String::new();

    for &line in lines.iter().skip(start).take(10) {
        // Check if this line has the opening brace
        if let Some(brace_pos) = line.find('{') {
            // Include up to (but not including) the brace, unless on same line as fn
            let before_brace = line[..brace_pos].trim_end();
            if !before_brace.is_empty() {
                sig.push_str(before_brace);
            }
            sig.push_str(" { ... }");
            sig.push('\n');
            break;
        }

        sig.push_str(line);
        sig.push('\n');

        // Arrow functions or trait method declarations end with semicolon
        if line.trim_end().ends_with(';') {
            break;
        }
    }

    sig
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_struct_definition() {
        assert!(matches_definition("pub struct TaskDef {", "struct", "TaskDef"));
        assert!(matches_definition(
            "pub(crate) struct TaskDef {",
            "struct",
            "TaskDef"
        ));
        assert!(matches_definition("struct TaskDef {", "struct", "TaskDef"));
        // Should not match partial names
        assert!(!matches_definition(
            "pub struct TaskDefExtended {",
            "struct",
            "TaskDef"
        ));
    }

    #[test]
    fn matches_enum_definition() {
        assert!(matches_definition(
            "pub enum AgentRole {",
            "enum",
            "AgentRole"
        ));
    }

    #[test]
    fn matches_fn_definitions() {
        assert!(matches_fn_definition(
            "pub fn build_prompt(&self, plan_id: &str, workdir: &Path) -> String {",
            "build_prompt"
        ));
        assert!(matches_fn_definition(
            "    pub async fn run(&self, input: &Signal, ctx: &Context) -> AgentResult {",
            "run"
        ));
        assert!(matches_fn_definition(
            "    pub const fn new(config: EnrichmentConfig, client: C) -> Self {",
            "new"
        ));
        // Should not match partial names
        assert!(!matches_fn_definition(
            "pub fn build_prompt_extended(&self) -> String {",
            "build_prompt"
        ));
    }

    #[test]
    fn extract_fn_signature_stops_at_brace() {
        let lines = vec![
            "    pub fn build_prompt(&self, plan_id: &str) -> String {",
            "        let mut prompt = String::new();",
            "        prompt",
            "    }",
        ];
        let sig = extract_fn_signature(&lines, 0);
        assert!(sig.contains("build_prompt"));
        assert!(sig.contains("{ ... }"));
        assert!(!sig.contains("let mut prompt"));
    }

    #[test]
    fn extract_fn_signature_multiline() {
        let lines = vec![
            "    pub fn complex_fn(",
            "        &self,",
            "        plan_id: &str,",
            "        workdir: &Path,",
            "    ) -> String {",
            "        todo!()",
            "    }",
        ];
        let sig = extract_fn_signature(&lines, 0);
        assert!(sig.contains("complex_fn"));
        assert!(sig.contains("plan_id: &str"));
        assert!(sig.contains("workdir: &Path"));
        assert!(!sig.contains("todo!"));
    }

    #[test]
    fn extract_struct_signature() {
        let lines = vec![
            "pub struct TaskDef {",
            "    pub id: String,",
            "    pub title: String,",
            "    pub tier: String,",
            "}",
        ];
        let sig = extract_block_signature(&lines, 0, 30);
        assert!(sig.contains("pub struct TaskDef"));
        assert!(sig.contains("pub id: String"));
        assert!(sig.contains("pub title: String"));
        assert!(sig.contains("}"));
    }

    #[test]
    fn extract_unit_struct_signature() {
        let lines = vec!["pub struct Marker;", ""];
        let sig = extract_block_signature(&lines, 0, 30);
        assert!(sig.contains("pub struct Marker;"));
    }

    #[test]
    fn find_definition_in_content() {
        let content = "use std::path::Path;\n\n\
/// A task definition.\n\
pub struct TaskDef {\n\
    pub id: String,\n\
    pub title: String,\n\
}\n\n\
pub fn helper() {}\n";

        let sym = find_definition(content, "TaskDef", "src/lib.rs").unwrap();
        assert_eq!(sym.symbol, "TaskDef");
        assert_eq!(sym.kind, SymbolKind::Struct);
        assert!(sym.signature.contains("pub id: String"));
    }

    #[test]
    fn find_definition_function() {
        let content = "pub fn effective_model(&self, fallback: &str) -> String {\n\
    fallback.to_string()\n\
}\n";

        let sym = find_definition(content, "effective_model", "src/lib.rs").unwrap();
        assert_eq!(sym.kind, SymbolKind::Function);
        assert!(sym.signature.contains("effective_model"));
        assert!(sym.signature.contains("{ ... }"));
    }

    #[test]
    fn find_method_in_impl_block() {
        let content = "\
pub struct Foo;\n\
\n\
impl Foo {\n\
    pub fn bar(&self) -> i32 {\n\
        42\n\
    }\n\
\n\
    pub fn baz(&self, x: &str) -> String {\n\
        x.to_string()\n\
    }\n\
}\n";

        let sym = find_method_in_impl(content, "Foo", "baz", "src/lib.rs", "Foo::baz").unwrap();
        assert_eq!(sym.symbol, "Foo::baz");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert!(sym.signature.contains("baz"));
        assert!(sym.signature.contains("x: &str"));
    }

    #[test]
    fn find_method_not_in_wrong_impl() {
        let content = "\
impl Bar {\n\
    pub fn baz(&self) -> i32 { 0 }\n\
}\n\
\n\
impl Foo {\n\
    pub fn qux(&self) -> i32 { 1 }\n\
}\n";

        // Looking for Foo::baz should fail (baz is in Bar's impl)
        let result = find_method_in_impl(content, "Foo", "baz", "src/lib.rs", "Foo::baz");
        assert!(result.is_none());

        // Looking for Foo::qux should succeed
        let result = find_method_in_impl(content, "Foo", "qux", "src/lib.rs", "Foo::qux");
        assert!(result.is_some());
    }
}
