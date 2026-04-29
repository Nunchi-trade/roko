use std::fmt::{self, Write as _};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const MAX_PROMPT_SECTION_CHARS: usize = 32_000;
const MAX_WORKSPACE_MEMBERS_RENDERED: usize = 50;
const MAX_KEY_FILES_RENDERED: usize = 10;
const MAX_SYMBOLS_RENDERED: usize = 15;
const MAX_RELATED_ITEMS_RENDERED: usize = 5;
const MAX_DO_NOT_CREATE_RENDERED: usize = 50;
const MAX_SYMBOL_TEXT_CHARS: usize = 120;
const TRUNCATION_MARKER: &str = "[truncated]";

/// Bounded repository context for grounding PRD/plan generation prompts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoContextPack {
    /// Workspace root directory (absolute path).
    pub root: PathBuf,
    /// Detected project kind.
    pub project_kind: ProjectKind,
    /// Workspace member names (crate names for Rust, package names for TS/Go).
    pub workspace_members: Vec<String>,
    /// Key files relevant to the feature being generated (max 20).
    pub key_files: Vec<PathBuf>,
    /// Symbol matches from grep-like search (max 30).
    pub matching_symbols: Vec<SymbolHit>,
    /// Related PRD paths found in `.roko/prd/` (max 5).
    pub related_prds: Vec<PathBuf>,
    /// Related plan paths found in `.roko/plans/` (max 5).
    pub related_plans: Vec<PathBuf>,
    /// Names that already exist and must not be re-created (workspace members + known crates).
    pub do_not_create: Vec<String>,
    /// True when feature keywords match workspace members AND real source exists.
    pub context_root_verified: bool,
}

/// A single symbol match from repository search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolHit {
    /// File path relative to workspace root.
    pub file: PathBuf,
    /// Line number (1-indexed).
    pub line: u32,
    /// Matched line text (trimmed, max 200 chars).
    pub text: String,
}

/// Detected project kind based on manifest files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectKind {
    Rust,
    TypeScript,
    Go,
    Python,
    Mixed,
    Unknown,
}

impl fmt::Display for ProjectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::Go => "go",
            Self::Python => "python",
            Self::Mixed => "mixed",
            Self::Unknown => "unknown",
        })
    }
}

impl RepoContextPack {
    /// Render as a markdown section suitable for prompt injection.
    ///
    /// Output is bounded to `<= 32000` characters, truncating gracefully.
    #[must_use]
    pub fn to_prompt_section(&self) -> String {
        let mut out = String::with_capacity(4_096);

        let _ = writeln!(out, "## Repository Context");
        let _ = writeln!(
            out,
            "**Project**: {} workspace at {}",
            self.project_kind,
            self.root.display()
        );

        render_inline_string_section(
            &mut out,
            "Workspace members",
            &self.workspace_members,
            MAX_WORKSPACE_MEMBERS_RENDERED,
        );
        render_path_section(&mut out, "Key files", &self.key_files, MAX_KEY_FILES_RENDERED);
        render_symbol_section(
            &mut out,
            "Symbol matches",
            &self.matching_symbols,
            MAX_SYMBOLS_RENDERED,
        );
        render_path_section(
            &mut out,
            "Related PRDs",
            &self.related_prds,
            MAX_RELATED_ITEMS_RENDERED,
        );
        render_path_section(
            &mut out,
            "Related plans",
            &self.related_plans,
            MAX_RELATED_ITEMS_RENDERED,
        );
        render_inline_string_section(
            &mut out,
            "Do NOT create these",
            &self.do_not_create,
            MAX_DO_NOT_CREATE_RENDERED,
        );

        let _ = writeln!(
            out,
            "**Context verification**: {}",
            if self.context_root_verified {
                "VERIFIED"
            } else {
                "UNVERIFIED WARNING"
            }
        );

        hard_truncate_section(out, MAX_PROMPT_SECTION_CHARS)
    }
}

fn render_inline_string_section(
    out: &mut String,
    title: &str,
    values: &[String],
    max_visible: usize,
) {
    let values: Vec<&str> = values
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect();
    let visible = values.len().min(max_visible);
    let _ = writeln!(out, "**{title}** {}:", count_label(values.len(), visible));

    if values.is_empty() {
        let _ = writeln!(out, "- none");
        return;
    }

    let mut rendered = values
        .iter()
        .take(visible)
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    if values.len() > visible {
        if !rendered.is_empty() {
            rendered.push_str(", ");
        }
        let _ = write!(rendered, "... and {} more", values.len() - visible);
    }

    let _ = writeln!(out, "{rendered}");
}

fn render_path_section(out: &mut String, title: &str, values: &[PathBuf], max_visible: usize) {
    let visible = values.len().min(max_visible);
    let _ = writeln!(out, "**{title}** {}:", count_label(values.len(), visible));

    if values.is_empty() {
        let _ = writeln!(out, "- none");
        return;
    }

    for path in values.iter().take(visible) {
        let _ = writeln!(out, "- {}", path.display());
    }

    if values.len() > visible {
        let _ = writeln!(out, "- ... and {} more", values.len() - visible);
    }
}

fn render_symbol_section(
    out: &mut String,
    title: &str,
    values: &[SymbolHit],
    max_visible: usize,
) {
    let visible = values.len().min(max_visible);
    let _ = writeln!(out, "**{title}** {}:", count_label(values.len(), visible));

    if values.is_empty() {
        let _ = writeln!(out, "- none");
        return;
    }

    for symbol in values.iter().take(visible) {
        let text = truncate_symbol_text(symbol.text.trim(), MAX_SYMBOL_TEXT_CHARS);
        let _ = writeln!(out, "- {}:{}: {}", symbol.file.display(), symbol.line, text);
    }

    if values.len() > visible {
        let _ = writeln!(out, "- ... and {} more", values.len() - visible);
    }
}

fn truncate_symbol_text(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let total = text.chars().count();
    if total <= max_chars {
        return text.to_string();
    }

    if max_chars <= TRUNCATION_MARKER.chars().count() {
        return TRUNCATION_MARKER.chars().take(max_chars).collect();
    }

    let keep = max_chars - TRUNCATION_MARKER.chars().count();
    let mut out: String = text.chars().take(keep).collect();
    out.push_str(TRUNCATION_MARKER);
    out
}

fn count_label(total: usize, visible: usize) -> String {
    if total == visible {
        format!("({total})")
    } else {
        format!("({total}, showing {visible})")
    }
}

fn hard_truncate_section(text: String, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let total = text.chars().count();
    if total <= max_chars {
        return text;
    }

    if TRUNCATION_MARKER.chars().count() >= max_chars {
        return TRUNCATION_MARKER.chars().take(max_chars).collect();
    }

    let keep = max_chars - TRUNCATION_MARKER.chars().count();
    let mut cut = char_boundary_index(&text, keep);
    if let Some(last_newline) = text[..cut].rfind('\n') {
        cut = last_newline + 1;
    }

    let mut out = text[..cut].to_string();
    out.push_str(TRUNCATION_MARKER);
    out
}

fn char_boundary_index(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(idx, _)| idx)
}
