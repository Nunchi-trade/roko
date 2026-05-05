//! `SymbolGate` — Rung 2 of the 6-rung verification ladder (§10.8).
//!
//! Parses Rust source files in the worktree and verifies that every symbol
//! the plan's [`SymbolManifest`] requires exists with the correct
//! [`SymbolKind`], [`Visibility`], and module path. Zero LLM calls, zero
//! subprocess spawns — just file walking + lightweight regex-based
//! extraction.
//!
//! This catches the most basic agent failure — "I was told to create
//! `pub struct RateLimiter` and did not" — at effectively zero cost, before
//! any test runs.
//!
//! # Manifest wiring
//!
//! The gate reads its [`SymbolManifest`] from the signal body (JSON). This
//! mirrors the existing [`TestGate`](crate::TestGate) /
//! [`CompileGate`](crate::CompileGate) pattern. A future `ArtifactStore`
//! abstraction can replace the body lookup without changing the verdict
//! contract.
//!
//! # Mismatch taxonomy
//!
//! Failure verdicts carry a machine-parseable `error_digest`, one line per
//! issue:
//!
//! ```text
//! 4 symbol expectations unmet:
//!   MISSING: struct RateLimiter at golem_core::rate_limit
//!   WRONG_VIS: fn check_rate at golem_core::rate_limit (found: private, expected: pub)
//!   WRONG_KIND: Limiter at golem_core::rate_limit (found: struct, expected: trait)
//!   WRONG_PATH: struct Clock at golem_core::time (found at: golem_core::clock)
//!   AMBIGUOUS: fn foo at golem_core::util (2 matches)
//! ```

use async_trait::async_trait;
use roko_core::{Context, Signal, Verdict, Verify};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Symbol kinds the gate knows how to verify.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    /// A Rust `struct` definition.
    Struct,
    /// A Rust `enum` definition.
    Enum,
    /// A Rust `trait` definition.
    Trait,
    /// A Rust `fn` item.
    Function,
    /// A Rust `type` alias.
    TypeAlias,
    /// A Rust `const` item.
    Const,
    /// A Rust `static` item.
    Static,
    /// A Rust `mod` declaration.
    Module,
}

impl SymbolKind {
    /// Human-readable name of this symbol kind.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Function => "fn",
            Self::TypeAlias => "type",
            Self::Const => "const",
            Self::Static => "static",
            Self::Module => "mod",
        }
    }

    /// Parse a keyword (e.g. `"struct"`) into a `SymbolKind`.
    #[must_use]
    pub fn from_keyword(keyword: &str) -> Option<Self> {
        match keyword {
            "struct" => Some(Self::Struct),
            "enum" => Some(Self::Enum),
            "trait" => Some(Self::Trait),
            "fn" => Some(Self::Function),
            "type" => Some(Self::TypeAlias),
            "const" => Some(Self::Const),
            "static" => Some(Self::Static),
            "mod" => Some(Self::Module),
            _ => None,
        }
    }
}

/// Visibility modifier on a symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// `pub`
    Pub,
    /// `pub(crate)` (or `pub(super)` / `pub(in …)` — all treated as crate-visible)
    PubCrate,
    /// No modifier.
    Private,
}

impl Visibility {
    /// Human-readable name of this visibility level.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Pub => "pub",
            Self::PubCrate => "pub(crate)",
            Self::Private => "private",
        }
    }
}

/// A single symbol the plan's manifest promises to create (or keep).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolExpectation {
    /// Bare symbol name (e.g. `"RateLimiter"`).
    pub name: String,
    /// Expected kind (struct / trait / fn / …).
    pub kind: SymbolKind,
    /// Expected visibility.
    pub visibility: Visibility,
    /// Logical module path (Rust: `"golem_core::rate_limit"`). Matching
    /// tolerates minor whitespace differences around the `::` separator.
    pub module_path: String,
    /// Optional signature substring that must appear in the found symbol's
    /// signature line. Best-effort; trait-bound-level matching is Rung 3.
    pub signature: Option<String>,
}

/// The plan's manifest of required symbols. Immutable once produced.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolManifest {
    /// Plan identifier (for logging / traceability).
    pub plan: String,
    /// Expected symbols.
    pub expectations: Vec<SymbolExpectation>,
}

impl SymbolManifest {
    /// Construct an empty manifest for the given plan id.
    #[must_use]
    pub fn new(plan: impl Into<String>) -> Self {
        Self {
            plan: plan.into(),
            expectations: Vec::new(),
        }
    }

    /// Append one expectation.
    #[must_use]
    pub fn with_expectation(mut self, exp: SymbolExpectation) -> Self {
        self.expectations.push(exp);
        self
    }
}

/// A discovered symbol in a source file.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscoveredSymbol {
    name: String,
    kind: SymbolKind,
    visibility: Visibility,
    module_path: String,
    signature_line: String,
}

/// Rung 2 gate: walks the worktree, parses Rust source, verifies that each
/// expectation in the manifest matches a real symbol.
pub struct SymbolGate {
    source_roots: Vec<PathBuf>,
    name: String,
}

impl SymbolGate {
    /// Construct a symbol gate that scans `source_roots` (typically the
    /// project's source directories, e.g. `["src", "crates"]`).
    #[must_use]
    pub fn new(source_roots: Vec<PathBuf>) -> Self {
        Self {
            source_roots,
            name: "symbol".into(),
        }
    }

    /// Override the gate's display name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

impl roko_core::Cell for SymbolGate {
    fn cell_id(&self) -> &str {
        "symbol-gate"
    }
    fn cell_name(&self) -> &str {
        "SymbolGate"
    }
    fn protocols(&self) -> &[&str] {
        &["Verify"]
    }
}

#[async_trait]
impl Verify for SymbolGate {
    async fn verify(&self, signal: &Signal, _ctx: &Context) -> Verdict {
        let started = Instant::now();
        let manifest: SymbolManifest = match signal.body.as_json() {
            Ok(m) => m,
            Err(e) => {
                let elapsed = elapsed_ms(started);
                return Verdict::fail(
                    &self.name,
                    format!("signal body is not a SymbolManifest: {e}"),
                )
                .with_duration(elapsed);
            }
        };

        if manifest.expectations.is_empty() {
            let elapsed = elapsed_ms(started);
            return Verdict::pass(&self.name)
                .with_detail("no symbol expectations")
                .with_duration(elapsed);
        }

        // Build an index of every discovered symbol, keyed by (name,
        // module_path). We collect vectors so we can detect ambiguity.
        let mut index: HashMap<(String, String), Vec<DiscoveredSymbol>> = HashMap::new();
        // Also keep a name-only lookup for "found elsewhere" (WRONG_PATH)
        // reporting.
        let mut by_name: HashMap<String, Vec<DiscoveredSymbol>> = HashMap::new();

        for root in &self.source_roots {
            if !root.exists() {
                // Partial worktree / shallow clone — skip missing root.
                continue;
            }
            let files = collect_rust_files(root);
            for file in files {
                let Some(module_path) = rust_module_path(&file, root) else {
                    continue;
                };
                let Ok(source) = std::fs::read_to_string(&file) else {
                    continue;
                };
                for sym in extract_symbols(&source, &module_path) {
                    by_name
                        .entry(sym.name.clone())
                        .or_default()
                        .push(sym.clone());
                    index
                        .entry((sym.name.clone(), sym.module_path.clone()))
                        .or_default()
                        .push(sym);
                }
            }
        }

        let mut mismatches: Vec<String> = Vec::new();
        for exp in &manifest.expectations {
            classify_expectation(exp, &index, &by_name, &mut mismatches);
        }

        let elapsed = elapsed_ms(started);
        if mismatches.is_empty() {
            let detail = format!("{} symbol expectation(s) met", manifest.expectations.len());
            Verdict::pass(&self.name)
                .with_detail(detail)
                .with_duration(elapsed)
        } else {
            let count = mismatches.len();
            let digest = format!(
                "{count} symbol expectation(s) unmet:\n  {}",
                mismatches.join("\n  ")
            );
            Verdict::fail(&self.name, format!("{count} symbol expectation(s) unmet"))
                .with_error_digest(digest.clone())
                .with_detail(digest)
                .with_duration(elapsed)
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

// ---------------------------------------------------------------------------
// Implementation helpers
// ---------------------------------------------------------------------------

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Normalize a module path: collapse whitespace around `::` separators.
pub(crate) fn normalize_path(path: &str) -> String {
    path.split("::")
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("::")
}

/// Decide how an expectation lines up against the discovered symbol index.
fn classify_expectation(
    exp: &SymbolExpectation,
    index: &HashMap<(String, String), Vec<DiscoveredSymbol>>,
    by_name: &HashMap<String, Vec<DiscoveredSymbol>>,
    mismatches: &mut Vec<String>,
) {
    let wanted_path = normalize_path(&exp.module_path);

    // When module_path is empty, treat as "find anywhere by name" — skip the
    // path-qualified lookup and go directly to by_name. This supports inferred
    // symbols where we don't know the module path.
    if wanted_path.is_empty() {
        classify_expectation_by_name(exp, by_name, mismatches);
        return;
    }

    let key = (exp.name.clone(), wanted_path.clone());

    if let Some(symbols) = index.get(&key) {
        if symbols.len() > 1 {
            mismatches.push(format!(
                "AMBIGUOUS: {} {} at {} ({} matches)",
                exp.kind.as_str(),
                exp.name,
                wanted_path,
                symbols.len()
            ));
            return;
        }
        let Some(found) = symbols.first() else {
            // Unreachable: we just checked the vec is non-empty. Defensive
            // fallback avoids unwrap_used/expect_used lint bites.
            mismatches.push(format!(
                "MISSING: {} {} at {}",
                exp.kind.as_str(),
                exp.name,
                wanted_path
            ));
            return;
        };
        if found.kind != exp.kind {
            mismatches.push(format!(
                "WRONG_KIND: {} at {} (found: {}, expected: {})",
                exp.name,
                wanted_path,
                found.kind.as_str(),
                exp.kind.as_str()
            ));
            return;
        }
        if found.visibility != exp.visibility {
            mismatches.push(format!(
                "WRONG_VIS: {} {} at {} (found: {}, expected: {})",
                exp.kind.as_str(),
                exp.name,
                wanted_path,
                found.visibility.as_str(),
                exp.visibility.as_str()
            ));
            return;
        }
        if let Some(sig) = exp.signature.as_deref() {
            if !found.signature_line.contains(sig) {
                mismatches.push(format!(
                    "WRONG_SIG: {} {} at {} (missing substring: {})",
                    exp.kind.as_str(),
                    exp.name,
                    wanted_path,
                    sig
                ));
            }
        }
        return;
    }

    // Not at the expected path — is it somewhere else?
    if let Some(candidates) = by_name.get(&exp.name) {
        if let Some(elsewhere) = candidates.first() {
            mismatches.push(format!(
                "WRONG_PATH: {} {} at {} (found at: {})",
                exp.kind.as_str(),
                exp.name,
                wanted_path,
                elsewhere.module_path
            ));
            return;
        }
    }

    mismatches.push(format!(
        "MISSING: {} {} at {}",
        exp.kind.as_str(),
        exp.name,
        wanted_path
    ));
}

/// Classify an expectation using only the name index (no path constraint).
/// Used when `module_path` is empty, meaning "find this symbol anywhere".
fn classify_expectation_by_name(
    exp: &SymbolExpectation,
    by_name: &HashMap<String, Vec<DiscoveredSymbol>>,
    mismatches: &mut Vec<String>,
) {
    let Some(candidates) = by_name.get(&exp.name) else {
        mismatches.push(format!("MISSING: {} {}", exp.kind.as_str(), exp.name));
        return;
    };
    if candidates.is_empty() {
        mismatches.push(format!("MISSING: {} {}", exp.kind.as_str(), exp.name));
        return;
    }
    // Find a candidate that matches kind and visibility.
    let best_match = candidates
        .iter()
        .find(|sym| sym.kind == exp.kind && sym.visibility == exp.visibility);
    // Fall back to any match by kind alone.
    let kind_match = best_match.or_else(|| candidates.iter().find(|sym| sym.kind == exp.kind));
    // Fall back to any match by name alone (symbol exists but with different kind).
    let found = kind_match.or_else(|| candidates.first());
    let Some(found) = found else {
        mismatches.push(format!("MISSING: {} {}", exp.kind.as_str(), exp.name));
        return;
    };
    // Check kind mismatch (only report if no kind match exists).
    if kind_match.is_none() {
        mismatches.push(format!(
            "WRONG_KIND: {} (found: {}, expected: {})",
            exp.name,
            found.kind.as_str(),
            exp.kind.as_str()
        ));
        return;
    }
    // Check visibility mismatch (only if kind matched).
    if best_match.is_none() && found.visibility != exp.visibility {
        mismatches.push(format!(
            "WRONG_VIS: {} {} (found: {}, expected: {})",
            exp.kind.as_str(),
            exp.name,
            found.visibility.as_str(),
            exp.visibility.as_str()
        ));
        return;
    }
    // Check signature if specified.
    if let Some(sig) = exp.signature.as_deref() {
        if let Some(matched) = best_match {
            if !matched.signature_line.contains(sig) {
                mismatches.push(format!(
                    "WRONG_SIG: {} {} (missing substring: {})",
                    exp.kind.as_str(),
                    exp.name,
                    sig
                ));
            }
        }
    }
}

/// Walk a directory collecting all `.rs` files.
fn collect_rust_files(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // Skip obvious build artifacts.
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name == "target" || name == ".git" || name == "node_modules" {
                    continue;
                }
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out
}

/// Map a Rust source path under `root` to a logical module path.
///
/// E.g. `root=src/`, `path=src/foo/bar.rs` → `Some("foo::bar")`.
/// `src/foo/mod.rs` and `src/foo.rs` both map to `"foo"`.
/// `src/lib.rs` and `src/main.rs` map to `""` (crate root).
pub(crate) fn rust_module_path(path: &Path, root: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let mut segments: Vec<String> = Vec::new();
    for component in rel.components() {
        let s = component.as_os_str().to_str()?;
        segments.push(s.to_string());
    }
    let file = segments.pop()?;
    let stem = Path::new(&file).file_stem().and_then(|s| s.to_str())?;
    if stem != "mod" && stem != "lib" && stem != "main" {
        segments.push(stem.to_string());
    }
    Some(segments.join("::"))
}

/// Lightweight single-pass scanner that extracts top-level Rust item
/// declarations from a source string.
///
/// Not a parser — it's an 80% regex-ish matcher. It handles:
///   - `pub`, `pub(crate)`, `pub(super)`, `pub(in path)`, and bare items
///   - `struct`, `enum`, `trait`, `fn`, `type`, `const`, `static`, `mod`
///   - single-line `//` comments
///
/// It deliberately does not try to descend into `mod foo { ... }` blocks —
/// Rust style guides nearly always use file modules, so inline modules are
/// rare. If the scanner sees one, symbols inside are tagged at the parent
/// module's path, which errs on the side of "found" rather than "missing".
fn extract_symbols(source: &str, module_path: &str) -> Vec<DiscoveredSymbol> {
    let mut out: Vec<DiscoveredSymbol> = Vec::new();
    for raw in source.lines() {
        let line = raw.trim_start();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        // Strip leading visibility keyword, if any.
        let (visibility, rest) = parse_visibility(line);
        // Allow modifiers between visibility and kind: async, const, unsafe, extern "C".
        let rest = skip_modifiers(rest);
        let Some((kind_kw, tail)) = rest.split_once(char::is_whitespace) else {
            continue;
        };
        let Some(kind) = SymbolKind::from_keyword(kind_kw) else {
            continue;
        };
        // Pull the identifier: first contiguous run of ident chars.
        let Some(name) = first_identifier(tail) else {
            continue;
        };
        out.push(DiscoveredSymbol {
            name: name.to_string(),
            kind,
            visibility,
            module_path: module_path.to_string(),
            signature_line: line.trim_end().to_string(),
        });
    }
    out
}

/// Parse the leading Rust visibility modifier from a source line.
pub(crate) fn parse_visibility(line: &str) -> (Visibility, &str) {
    if let Some(rest) = line.strip_prefix("pub(crate)") {
        return (Visibility::PubCrate, rest.trim_start());
    }
    if let Some(rest) = line.strip_prefix("pub(super)") {
        return (Visibility::PubCrate, rest.trim_start());
    }
    if let Some(rest) = line.strip_prefix("pub(in ") {
        // pub(in path::to::mod) — skip to close paren.
        if let Some(close) = rest.find(')') {
            let after = rest.get(close + 1..).unwrap_or("").trim_start();
            return (Visibility::PubCrate, after);
        }
        return (Visibility::PubCrate, rest);
    }
    if let Some(rest) = line.strip_prefix("pub ") {
        return (Visibility::Pub, rest.trim_start());
    }
    if line == "pub" {
        return (Visibility::Pub, "");
    }
    (Visibility::Private, line)
}

/// Skip modifier keywords (`async`, `unsafe`, `extern`, and the fn-level
/// `const`) so the caller sees the item kind keyword next.
///
/// Note: a bare `const NAME: …` item is itself a `Const` item — we do NOT
/// strip `const` in that case. We tell the two apart by peeking the token
/// after `const `: if it's `fn`, we strip; otherwise we leave it.
pub(crate) fn skip_modifiers(rest: &str) -> &str {
    let mut cur = rest;
    loop {
        let trimmed = cur.trim_start();
        if let Some(after) = trimmed.strip_prefix("async ") {
            cur = after;
            continue;
        }
        if let Some(after) = trimmed.strip_prefix("unsafe ") {
            cur = after;
            continue;
        }
        if let Some(after) = trimmed.strip_prefix("extern \"C\" ") {
            cur = after;
            continue;
        }
        if let Some(after) = trimmed.strip_prefix("extern ") {
            cur = after;
            continue;
        }
        if let Some(after) = trimmed.strip_prefix("const ") {
            // `const fn` → strip the const and let caller see `fn`.
            // `const FOO: …` → leave alone; caller will classify as Const.
            if after.trim_start().starts_with("fn ") {
                cur = after;
                continue;
            }
            return trimmed;
        }
        return trimmed;
    }
}

/// Extract the first Rust identifier from the start of a string.
pub(crate) fn first_identifier(s: &str) -> Option<&str> {
    let s = s.trim_start();
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let first = bytes[0];
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return None;
    }
    let mut end = 1;
    while end < bytes.len() {
        let b = bytes[end];
        if b.is_ascii_alphanumeric() || b == b'_' {
            end += 1;
        } else {
            break;
        }
    }
    Some(&s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind};
    use tempfile::TempDir;

    fn manifest_signal(manifest: &SymbolManifest) -> Signal {
        let body = Body::from_json(manifest).expect("manifest serializes");
        Signal::builder(Kind::Task).body(body).build()
    }

    fn write_file(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, body).expect("write source file");
    }

    fn exp(name: &str, kind: SymbolKind, vis: Visibility, path: &str) -> SymbolExpectation {
        SymbolExpectation {
            name: name.into(),
            kind,
            visibility: vis,
            module_path: path.into(),
            signature: None,
        }
    }

    #[tokio::test]
    async fn empty_manifest_passes() {
        let tmp = TempDir::new().expect("tempdir");
        let gate = SymbolGate::new(vec![tmp.path().to_path_buf()]);
        let manifest = SymbolManifest::new("plan-1");
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(v.passed);
        assert_eq!(v.detail.as_deref(), Some("no symbol expectations"));
        assert_eq!(v.gate, "symbol");
    }

    #[tokio::test]
    async fn all_expectations_met() {
        let tmp = TempDir::new().expect("tempdir");
        write_file(
            tmp.path(),
            "rate_limit.rs",
            "pub struct RateLimiter {}\npub fn check_rate() {}\n",
        );
        let gate = SymbolGate::new(vec![tmp.path().to_path_buf()]);
        let manifest = SymbolManifest::new("plan-2")
            .with_expectation(exp(
                "RateLimiter",
                SymbolKind::Struct,
                Visibility::Pub,
                "rate_limit",
            ))
            .with_expectation(exp(
                "check_rate",
                SymbolKind::Function,
                Visibility::Pub,
                "rate_limit",
            ));
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(v.passed, "expected pass, got: {v:?}");
        assert!(v.detail.as_deref().unwrap_or("").contains("2 symbol"));
    }

    #[tokio::test]
    async fn missing_symbol_reported() {
        let tmp = TempDir::new().expect("tempdir");
        write_file(tmp.path(), "lib.rs", "pub struct Other {}\n");
        let gate = SymbolGate::new(vec![tmp.path().to_path_buf()]);
        let manifest = SymbolManifest::new("plan-3").with_expectation(exp(
            "RateLimiter",
            SymbolKind::Struct,
            Visibility::Pub,
            "",
        ));
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(!v.passed);
        let digest = v.error_digest.expect("digest present");
        assert!(digest.contains("MISSING:"), "digest: {digest}");
        assert!(digest.contains("RateLimiter"));
    }

    #[tokio::test]
    async fn wrong_visibility_reported() {
        let tmp = TempDir::new().expect("tempdir");
        write_file(tmp.path(), "m.rs", "struct Secret {}\n");
        let gate = SymbolGate::new(vec![tmp.path().to_path_buf()]);
        let manifest = SymbolManifest::new("plan-4").with_expectation(exp(
            "Secret",
            SymbolKind::Struct,
            Visibility::Pub,
            "m",
        ));
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(!v.passed);
        let digest = v.error_digest.expect("digest");
        assert!(digest.contains("WRONG_VIS:"), "digest: {digest}");
        assert!(digest.contains("private"));
        assert!(digest.contains("pub"));
    }

    #[tokio::test]
    async fn wrong_kind_reported() {
        let tmp = TempDir::new().expect("tempdir");
        write_file(tmp.path(), "m.rs", "pub struct Limiter {}\n");
        let gate = SymbolGate::new(vec![tmp.path().to_path_buf()]);
        let manifest = SymbolManifest::new("plan-5").with_expectation(exp(
            "Limiter",
            SymbolKind::Trait,
            Visibility::Pub,
            "m",
        ));
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(!v.passed);
        let digest = v.error_digest.expect("digest");
        assert!(digest.contains("WRONG_KIND:"), "digest: {digest}");
        assert!(digest.contains("struct"));
        assert!(digest.contains("trait"));
    }

    #[tokio::test]
    async fn wrong_path_reported() {
        let tmp = TempDir::new().expect("tempdir");
        write_file(tmp.path(), "clock.rs", "pub struct Clock {}\n");
        let gate = SymbolGate::new(vec![tmp.path().to_path_buf()]);
        let manifest = SymbolManifest::new("plan-6").with_expectation(exp(
            "Clock",
            SymbolKind::Struct,
            Visibility::Pub,
            "time",
        ));
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(!v.passed);
        let digest = v.error_digest.expect("digest");
        assert!(digest.contains("WRONG_PATH:"), "digest: {digest}");
        assert!(digest.contains("clock"));
    }

    #[tokio::test]
    async fn ambiguous_match_reported() {
        let tmp = TempDir::new().expect("tempdir");
        // Same symbol name, same module_path via mod.rs aliasing.
        write_file(tmp.path(), "foo/mod.rs", "pub fn dup() {}\n");
        write_file(tmp.path(), "foo.rs", "pub fn dup() {}\n");
        let gate = SymbolGate::new(vec![tmp.path().to_path_buf()]);
        let manifest = SymbolManifest::new("plan-7").with_expectation(exp(
            "dup",
            SymbolKind::Function,
            Visibility::Pub,
            "foo",
        ));
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(!v.passed);
        let digest = v.error_digest.expect("digest");
        assert!(digest.contains("AMBIGUOUS:"), "digest: {digest}");
    }

    #[tokio::test]
    async fn unparseable_file_is_skipped() {
        let tmp = TempDir::new().expect("tempdir");
        // This won't parse as Rust, but the scanner is line-based and will
        // simply fail to match any items — treated as "no symbols".
        write_file(
            tmp.path(),
            "broken.rs",
            "this is <<not>> valid rust ??? { {\n",
        );
        write_file(tmp.path(), "good.rs", "pub struct Good {}\n");
        let gate = SymbolGate::new(vec![tmp.path().to_path_buf()]);
        let manifest = SymbolManifest::new("plan-8").with_expectation(exp(
            "Good",
            SymbolKind::Struct,
            Visibility::Pub,
            "good",
        ));
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(v.passed, "expected pass, got: {v:?}");
    }

    #[tokio::test]
    async fn missing_source_root_is_skipped() {
        let tmp = TempDir::new().expect("tempdir");
        let fake = tmp.path().join("does_not_exist");
        let gate = SymbolGate::new(vec![fake, tmp.path().to_path_buf()]);
        write_file(tmp.path(), "a.rs", "pub struct A {}\n");
        let manifest = SymbolManifest::new("plan-9").with_expectation(exp(
            "A",
            SymbolKind::Struct,
            Visibility::Pub,
            "a",
        ));
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(v.passed, "expected pass, got: {v:?}");
    }

    #[tokio::test]
    async fn collects_all_mismatches_not_just_first() {
        let tmp = TempDir::new().expect("tempdir");
        write_file(tmp.path(), "m.rs", "struct One {}\n");
        let gate = SymbolGate::new(vec![tmp.path().to_path_buf()]);
        let manifest = SymbolManifest::new("plan-10")
            .with_expectation(exp("One", SymbolKind::Struct, Visibility::Pub, "m"))
            .with_expectation(exp("Two", SymbolKind::Enum, Visibility::Pub, "m"))
            .with_expectation(exp("Three", SymbolKind::Trait, Visibility::Pub, "m"));
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(!v.passed);
        let digest = v.error_digest.expect("digest");
        // 3 unmet: 1 wrong-vis, 2 missing.
        assert!(digest.contains("3 symbol expectation(s) unmet"));
        assert!(digest.contains("WRONG_VIS:"));
        assert_eq!(digest.matches("MISSING:").count(), 2);
    }

    #[tokio::test]
    async fn signature_substring_check() {
        let tmp = TempDir::new().expect("tempdir");
        write_file(
            tmp.path(),
            "m.rs",
            "pub fn limit(rate: u32) -> Result<(), Error> {}\n",
        );
        let gate = SymbolGate::new(vec![tmp.path().to_path_buf()]);
        let mut manifest = SymbolManifest::new("plan-11").with_expectation(SymbolExpectation {
            name: "limit".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Pub,
            module_path: "m".into(),
            signature: Some("Result<(), Error>".into()),
        });
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(v.passed, "good signature should pass, got: {v:?}");

        manifest.expectations[0].signature = Some("NotInSource".into());
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(!v.passed);
        let digest = v.error_digest.expect("digest");
        assert!(digest.contains("WRONG_SIG:"), "digest: {digest}");
    }

    #[tokio::test]
    async fn multi_root_search() {
        let tmp1 = TempDir::new().expect("tempdir1");
        let tmp2 = TempDir::new().expect("tempdir2");
        write_file(tmp1.path(), "a.rs", "pub struct A {}\n");
        write_file(tmp2.path(), "b.rs", "pub trait B {}\n");
        let gate = SymbolGate::new(vec![tmp1.path().to_path_buf(), tmp2.path().to_path_buf()]);
        let manifest = SymbolManifest::new("plan-12")
            .with_expectation(exp("A", SymbolKind::Struct, Visibility::Pub, "a"))
            .with_expectation(exp("B", SymbolKind::Trait, Visibility::Pub, "b"));
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(v.passed, "expected pass, got: {v:?}");
    }

    #[tokio::test]
    async fn nested_module_path_resolves() {
        let tmp = TempDir::new().expect("tempdir");
        write_file(tmp.path(), "core/rate_limit.rs", "pub struct Limiter {}\n");
        let gate = SymbolGate::new(vec![tmp.path().to_path_buf()]);
        let manifest = SymbolManifest::new("plan-13").with_expectation(exp(
            "Limiter",
            SymbolKind::Struct,
            Visibility::Pub,
            "core::rate_limit",
        ));
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(v.passed, "expected pass, got: {v:?}");
    }

    #[tokio::test]
    async fn lib_rs_maps_to_crate_root() {
        let tmp = TempDir::new().expect("tempdir");
        write_file(tmp.path(), "lib.rs", "pub trait RootTrait {}\n");
        let gate = SymbolGate::new(vec![tmp.path().to_path_buf()]);
        let manifest = SymbolManifest::new("plan-14").with_expectation(exp(
            "RootTrait",
            SymbolKind::Trait,
            Visibility::Pub,
            "",
        ));
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(v.passed, "expected pass, got: {v:?}");
    }

    #[tokio::test]
    async fn bad_body_returns_fail_verdict() {
        let gate = SymbolGate::new(vec![]);
        // Body::Text is not valid JSON deserialization for SymbolManifest.
        let sig = Signal::builder(Kind::Task).body(Body::text("nope")).build();
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(!v.passed);
        assert!(v.reason.contains("not a SymbolManifest"));
    }

    #[tokio::test]
    async fn custom_name_appears_in_verdict() {
        let gate = SymbolGate::new(vec![]).with_name("my_symbol_gate");
        let manifest = SymbolManifest::new("plan-15");
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert_eq!(v.gate, "my_symbol_gate");
    }

    // ----- parser unit tests -----

    #[test]
    fn parse_visibility_recognizes_pub_crate() {
        let (v, rest) = parse_visibility("pub(crate) fn foo() {}");
        assert_eq!(v, Visibility::PubCrate);
        assert!(rest.starts_with("fn "));
    }

    #[test]
    fn parse_visibility_recognizes_pub_in_path() {
        let (v, rest) = parse_visibility("pub(in crate::foo) fn bar() {}");
        assert_eq!(v, Visibility::PubCrate);
        assert!(rest.starts_with("fn "));
    }

    #[test]
    fn parse_visibility_recognizes_plain_pub() {
        let (v, rest) = parse_visibility("pub struct X;");
        assert_eq!(v, Visibility::Pub);
        assert!(rest.starts_with("struct "));
    }

    #[test]
    fn parse_visibility_defaults_private() {
        let (v, rest) = parse_visibility("fn private() {}");
        assert_eq!(v, Visibility::Private);
        assert!(rest.starts_with("fn "));
    }

    #[test]
    fn first_identifier_extracts_name() {
        assert_eq!(first_identifier(" Foo<T> "), Some("Foo"));
        assert_eq!(first_identifier("_hidden()"), Some("_hidden"));
        assert_eq!(first_identifier("123bad"), None);
    }

    #[test]
    fn normalize_path_trims_spacing() {
        assert_eq!(normalize_path("foo :: bar ::baz"), "foo::bar::baz");
        assert_eq!(normalize_path("foo::bar"), "foo::bar");
    }

    #[test]
    fn rust_module_path_mod_rs_collapses() {
        let p = rust_module_path(Path::new("/root/foo/mod.rs"), Path::new("/root"));
        assert_eq!(p.as_deref(), Some("foo"));
    }

    #[test]
    fn rust_module_path_lib_rs_is_empty() {
        let p = rust_module_path(Path::new("/root/lib.rs"), Path::new("/root"));
        assert_eq!(p.as_deref(), Some(""));
    }

    #[test]
    fn extract_symbols_parses_simple_items() {
        let src = "pub struct A {}\nfn b() {}\npub(crate) trait T {}\n";
        let syms = extract_symbols(src, "m");
        assert_eq!(syms.len(), 3);
        assert_eq!(syms[0].name, "A");
        assert_eq!(syms[0].kind, SymbolKind::Struct);
        assert_eq!(syms[0].visibility, Visibility::Pub);
        assert_eq!(syms[1].name, "b");
        assert_eq!(syms[1].visibility, Visibility::Private);
        assert_eq!(syms[2].name, "T");
        assert_eq!(syms[2].visibility, Visibility::PubCrate);
    }

    #[test]
    fn extract_symbols_handles_async_unsafe_fn() {
        let src = "pub async fn connect() {}\npub unsafe fn raw() {}\n";
        let syms = extract_symbols(src, "m");
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].name, "connect");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[1].name, "raw");
        assert_eq!(syms[1].kind, SymbolKind::Function);
    }

    #[test]
    fn extract_symbols_ignores_comments_and_blanks() {
        let src = "// pub struct Ghost {}\n\n   // just whitespace\npub fn real() {}\n";
        let syms = extract_symbols(src, "m");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "real");
    }

    #[test]
    fn symbol_kind_roundtrip() {
        for kw in [
            "struct", "enum", "trait", "fn", "type", "const", "static", "mod",
        ] {
            let k = SymbolKind::from_keyword(kw).expect("known");
            assert_eq!(k.as_str(), kw);
        }
        assert!(SymbolKind::from_keyword("impl").is_none());
    }

    // ── Empty module_path (find-anywhere) tests ─────────────────────────

    #[tokio::test]
    async fn empty_module_path_finds_symbol_anywhere() {
        let tmp = TempDir::new().expect("tempdir");
        write_file(
            tmp.path(),
            "deep/nested/module.rs",
            "pub struct Target {}\n",
        );
        let gate = SymbolGate::new(vec![tmp.path().to_path_buf()]);
        // Empty module_path means "find it anywhere in the source tree"
        let manifest = SymbolManifest::new("plan-wildcard").with_expectation(exp(
            "Target",
            SymbolKind::Struct,
            Visibility::Pub,
            "", // wildcard: find anywhere
        ));
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(v.passed, "expected pass (find-anywhere), got: {v:?}");
    }

    #[tokio::test]
    async fn empty_module_path_reports_missing() {
        let tmp = TempDir::new().expect("tempdir");
        write_file(tmp.path(), "lib.rs", "pub struct Other {}\n");
        let gate = SymbolGate::new(vec![tmp.path().to_path_buf()]);
        let manifest = SymbolManifest::new("plan-wildcard-miss").with_expectation(exp(
            "NonExistent",
            SymbolKind::Struct,
            Visibility::Pub,
            "",
        ));
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(!v.passed);
        let digest = v.error_digest.expect("digest");
        assert!(digest.contains("MISSING:"), "digest: {digest}");
        assert!(digest.contains("NonExistent"));
    }

    #[tokio::test]
    async fn empty_module_path_reports_wrong_kind() {
        let tmp = TempDir::new().expect("tempdir");
        write_file(tmp.path(), "foo.rs", "pub struct Foo {}\n");
        let gate = SymbolGate::new(vec![tmp.path().to_path_buf()]);
        // Expect a trait named Foo, but it's actually a struct
        let manifest = SymbolManifest::new("plan-kind-mismatch").with_expectation(exp(
            "Foo",
            SymbolKind::Trait,
            Visibility::Pub,
            "",
        ));
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(!v.passed);
        let digest = v.error_digest.expect("digest");
        assert!(digest.contains("WRONG_KIND:"), "digest: {digest}");
    }

    #[tokio::test]
    async fn empty_module_path_reports_wrong_visibility() {
        let tmp = TempDir::new().expect("tempdir");
        write_file(tmp.path(), "bar.rs", "struct Bar {}\n"); // private
        let gate = SymbolGate::new(vec![tmp.path().to_path_buf()]);
        // Expect pub, got private
        let manifest = SymbolManifest::new("plan-vis-mismatch").with_expectation(exp(
            "Bar",
            SymbolKind::Struct,
            Visibility::Pub,
            "",
        ));
        let v = gate
            .verify(&manifest_signal(&manifest), &Context::at(0))
            .await;
        assert!(!v.passed);
        let digest = v.error_digest.expect("digest");
        assert!(digest.contains("WRONG_VIS:"), "digest: {digest}");
    }
}
