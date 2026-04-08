//! Project-specific conventions extractor.
//!
//! Analyzes source files (passed as strings — no `std::fs`) to detect coding
//! conventions used in a project. The resulting [`ProjectConventions`] feeds
//! into layer 2 of the [`SystemPromptBuilder`](super::system_prompt_builder).
//!
//! Detection is heuristic and conservative: we look for strong signals (e.g.
//! `use thiserror` in Cargo.toml, `snake_case` identifiers in Rust source)
//! rather than trying to be exhaustive. False-negatives are preferable to
//! false-positives — an absent convention just means the agent gets a slightly
//! less tailored prompt, while a wrong convention actively misleads.

use serde::{Deserialize, Serialize};

/// Naming convention style detected in the project.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamingStyle {
    /// `snake_case` — typical Rust, Python.
    SnakeCase,
    /// `camelCase` — typical `JavaScript`, `TypeScript`.
    CamelCase,
    /// `PascalCase` — typical C#, Go exported identifiers.
    PascalCase,
    /// Mixed or unknown.
    Mixed,
}

/// Error handling pattern detected in the project.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorPattern {
    /// Uses `thiserror` derive macros for error types.
    Thiserror,
    /// Uses `anyhow::Result` / `anyhow::Error` for ad-hoc errors.
    Anyhow,
    /// Uses both `thiserror` (for library errors) and `anyhow` (in binaries).
    ThiserrorAndAnyhow,
    /// Custom error types without external crates.
    Custom,
    /// Could not determine.
    Unknown,
}

/// Module organization pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModulePattern {
    /// Flat modules: `src/foo.rs`, `src/bar.rs`.
    Flat,
    /// Nested modules: `src/foo/mod.rs`, `src/foo/sub.rs`.
    Nested,
    /// Mix of flat and nested.
    Mixed,
    /// Could not determine.
    Unknown,
}

/// Detected project conventions.
///
/// All fields are best-effort detections — callers should treat them as hints
/// rather than hard constraints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectConventions {
    /// Primary naming convention for identifiers.
    pub naming: NamingStyle,
    /// Error handling pattern.
    pub error_handling: ErrorPattern,
    /// Module organization style.
    pub module_organization: ModulePattern,
    /// Whether `#[must_use]` is commonly applied.
    pub uses_must_use: bool,
    /// Whether doc comments (`///`) are consistently present on public items.
    pub has_doc_comments: bool,
    /// Additional free-form conventions extracted from AGENTS.md or similar.
    pub extra: Vec<String>,
}

impl Default for ProjectConventions {
    fn default() -> Self {
        Self {
            naming: NamingStyle::Mixed,
            error_handling: ErrorPattern::Unknown,
            module_organization: ModulePattern::Unknown,
            uses_must_use: false,
            has_doc_comments: false,
            extra: Vec::new(),
        }
    }
}

impl ProjectConventions {
    /// Format conventions as a human-readable summary suitable for injection
    /// into a system prompt.
    #[must_use]
    pub fn to_prompt_fragment(&self) -> String {
        let mut lines = Vec::new();

        lines.push(format!(
            "- Naming convention: {}",
            match self.naming {
                NamingStyle::SnakeCase => "snake_case (Rust-style)",
                NamingStyle::CamelCase => "camelCase (JS-style)",
                NamingStyle::PascalCase => "PascalCase",
                NamingStyle::Mixed => "mixed",
            }
        ));

        lines.push(format!(
            "- Error handling: {}",
            match self.error_handling {
                ErrorPattern::Thiserror => "thiserror derive macros",
                ErrorPattern::Anyhow => "anyhow for ad-hoc errors",
                ErrorPattern::ThiserrorAndAnyhow =>
                    "thiserror for library errors, anyhow in binaries",
                ErrorPattern::Custom => "custom error types",
                ErrorPattern::Unknown => "not determined",
            }
        ));

        lines.push(format!(
            "- Module organization: {}",
            match self.module_organization {
                ModulePattern::Flat => "flat (one file per module)",
                ModulePattern::Nested => "nested (mod.rs directories)",
                ModulePattern::Mixed => "mixed flat and nested",
                ModulePattern::Unknown => "not determined",
            }
        ));

        if self.uses_must_use {
            lines.push("- #[must_use] applied to public functions".to_string());
        }
        if self.has_doc_comments {
            lines.push("- Doc comments (///) on public items".to_string());
        }
        for extra in &self.extra {
            lines.push(format!("- {extra}"));
        }

        lines.join("\n")
    }
}

/// Detect project conventions from source artifacts.
///
/// # Arguments
///
/// * `cargo_toml` — Contents of the project's root `Cargo.toml` (or workspace
///   `Cargo.toml`). Pass empty string if unavailable.
/// * `source_samples` — A handful of representative `.rs` source files. The
///   more files provided, the more confident the detection — but even one
///   file is enough for useful signal.
/// * `file_listing` — A list of file paths (relative to repo root). Used to
///   detect module organization patterns.
#[must_use]
pub fn detect_conventions(
    cargo_toml: &str,
    source_samples: &[&str],
    file_listing: &[&str],
) -> ProjectConventions {
    ProjectConventions {
        naming: detect_naming(source_samples),
        error_handling: detect_error_pattern(cargo_toml, source_samples),
        module_organization: detect_module_pattern(file_listing),
        uses_must_use: detect_must_use(source_samples),
        has_doc_comments: detect_doc_comments(source_samples),
        extra: Vec::new(),
    }
}

/// Detect naming convention from source samples.
fn detect_naming(sources: &[&str]) -> NamingStyle {
    let mut snake_count = 0usize;
    let mut camel_count = 0usize;

    for src in sources {
        // Count `fn foo_bar` patterns (snake_case).
        snake_count += src.matches("fn ").count();
        // Look for camelCase: lowercase letter followed by uppercase in identifiers.
        for word in src.split_whitespace() {
            let has_lower_upper = word
                .as_bytes()
                .windows(2)
                .any(|w| w[0].is_ascii_lowercase() && w[1].is_ascii_uppercase());
            if has_lower_upper && word.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
                camel_count += 1;
            }
        }
    }

    if snake_count == 0 && camel_count == 0 {
        return NamingStyle::Mixed;
    }

    // If the ratio of snake_case signals to camelCase signals is > 3:1, call it snake_case.
    if snake_count > camel_count * 3 {
        NamingStyle::SnakeCase
    } else if camel_count > snake_count * 3 {
        NamingStyle::CamelCase
    } else {
        NamingStyle::Mixed
    }
}

/// Detect error handling pattern from Cargo.toml and source.
fn detect_error_pattern(cargo_toml: &str, sources: &[&str]) -> ErrorPattern {
    let has_thiserror =
        cargo_toml.contains("thiserror") || sources.iter().any(|s| s.contains("thiserror"));
    let has_anyhow = cargo_toml.contains("anyhow") || sources.iter().any(|s| s.contains("anyhow"));

    match (has_thiserror, has_anyhow) {
        (true, true) => ErrorPattern::ThiserrorAndAnyhow,
        (true, false) => ErrorPattern::Thiserror,
        (false, true) => ErrorPattern::Anyhow,
        (false, false) => {
            // Check for custom error types.
            let has_custom = sources
                .iter()
                .any(|s| s.contains("impl std::error::Error") || s.contains("impl Error for"));
            if has_custom {
                ErrorPattern::Custom
            } else {
                ErrorPattern::Unknown
            }
        }
    }
}

/// Detect module organization pattern from file listing.
fn detect_module_pattern(files: &[&str]) -> ModulePattern {
    if files.is_empty() {
        return ModulePattern::Unknown;
    }

    let has_mod_rs = files.iter().any(|f| f.ends_with("/mod.rs"));
    let has_flat = files.iter().any(|f| {
        std::path::Path::new(f)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
            && !f.ends_with("/mod.rs")
            && !f.ends_with("/lib.rs")
            && !f.ends_with("/main.rs")
    });

    match (has_mod_rs, has_flat) {
        (true, true) => ModulePattern::Mixed,
        (true, false) => ModulePattern::Nested,
        (false, true) => ModulePattern::Flat,
        (false, false) => ModulePattern::Unknown,
    }
}

/// Detect `#[must_use]` usage.
fn detect_must_use(sources: &[&str]) -> bool {
    sources.iter().any(|s| s.contains("#[must_use]"))
}

/// Detect doc comment presence.
fn detect_doc_comments(sources: &[&str]) -> bool {
    let total_pub_items: usize = sources.iter().map(|s| s.matches("pub fn ").count()).sum();
    if total_pub_items == 0 {
        return false;
    }
    let doc_comment_count: usize = sources.iter().map(|s| s.matches("/// ").count()).sum();
    // If at least half of pub functions have doc comments, consider it a convention.
    doc_comment_count >= total_pub_items / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST_SOURCE: &str = r#"
use thiserror::Error;

/// A config value.
#[must_use]
pub fn get_config() -> Config {
    Config::default()
}

/// Parse the input.
pub fn parse_input(data: &str) -> Result<Parsed> {
    // ...
    Ok(Parsed)
}

fn helper_fn() -> bool {
    true
}
"#;

    const CARGO_TOML: &str = r#"
[dependencies]
thiserror = "1"
serde = { version = "1", features = ["derive"] }
"#;

    #[test]
    fn detect_snake_case_naming() {
        let conventions = detect_conventions(CARGO_TOML, &[RUST_SOURCE], &[]);
        assert_eq!(conventions.naming, NamingStyle::SnakeCase);
    }

    #[test]
    fn detect_thiserror_pattern() {
        let conventions = detect_conventions(CARGO_TOML, &[RUST_SOURCE], &[]);
        assert_eq!(conventions.error_handling, ErrorPattern::Thiserror);
    }

    #[test]
    fn detect_module_organization() {
        let files = vec![
            "src/lib.rs",
            "src/config.rs",
            "src/parser/mod.rs",
            "src/parser/lexer.rs",
        ];
        let conventions = detect_conventions("", &[], &files);
        assert_eq!(conventions.module_organization, ModulePattern::Mixed);
    }

    #[test]
    fn detect_must_use_and_docs() {
        let conventions = detect_conventions("", &[RUST_SOURCE], &[]);
        assert!(conventions.uses_must_use);
        assert!(conventions.has_doc_comments);
    }

    #[test]
    fn to_prompt_fragment_is_readable() {
        let conventions = detect_conventions(CARGO_TOML, &[RUST_SOURCE], &[]);
        let fragment = conventions.to_prompt_fragment();
        assert!(fragment.contains("snake_case"));
        assert!(fragment.contains("thiserror"));
        assert!(fragment.contains("#[must_use]"));
    }

    #[test]
    fn empty_input_yields_defaults() {
        let conventions = detect_conventions("", &[], &[]);
        assert_eq!(conventions.naming, NamingStyle::Mixed);
        assert_eq!(conventions.error_handling, ErrorPattern::Unknown);
        assert_eq!(conventions.module_organization, ModulePattern::Unknown);
        assert!(!conventions.uses_must_use);
        assert!(!conventions.has_doc_comments);
    }

    #[test]
    fn detect_anyhow_pattern() {
        let cargo = "[dependencies]\nanyhow = \"1\"\n";
        let conventions = detect_conventions(cargo, &[], &[]);
        assert_eq!(conventions.error_handling, ErrorPattern::Anyhow);
    }

    #[test]
    fn detect_camel_case_in_js_style() {
        let js_source = "const myVariable = getValue(); const anotherThing = computeStuff();";
        let conventions = detect_conventions("", &[js_source], &[]);
        assert_eq!(conventions.naming, NamingStyle::CamelCase);
    }

    #[test]
    fn extra_conventions_in_fragment() {
        let mut conventions = ProjectConventions::default();
        conventions
            .extra
            .push("Always use Result, never panic".to_string());
        let fragment = conventions.to_prompt_fragment();
        assert!(fragment.contains("Always use Result, never panic"));
    }
}
