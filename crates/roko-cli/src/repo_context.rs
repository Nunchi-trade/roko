use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};

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

impl ProjectKind {
    /// Detect project kind from manifest files at the given root.
    ///
    /// Returns `Mixed` when multiple kinds are detected and `Unknown` when none are found.
    #[must_use]
    pub fn detect(root: &Path) -> Self {
        let has_cargo = root.join("Cargo.toml").exists();
        let has_package_json = root.join("package.json").exists();
        let has_go = root.join("go.work").exists() || root.join("go.mod").exists();
        let has_python = root.join("pyproject.toml").exists() || root.join("setup.py").exists();

        let count = [has_cargo, has_package_json, has_go, has_python]
            .into_iter()
            .filter(|detected| *detected)
            .count();

        match count {
            0 => Self::Unknown,
            1 if has_cargo => Self::Rust,
            1 if has_package_json => Self::TypeScript,
            1 if has_go => Self::Go,
            1 if has_python => Self::Python,
            _ => Self::Mixed,
        }
    }
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
        render_path_section(
            &mut out,
            "Key files",
            &self.key_files,
            MAX_KEY_FILES_RENDERED,
        );
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

/// Collect workspace members from manifest files.
///
/// Returns `(members, do_not_create)` where `do_not_create` is a copy of the members.
#[must_use]
pub fn collect_workspace_members(root: &Path, kind: &ProjectKind) -> (Vec<String>, Vec<String>) {
    let members = match kind {
        ProjectKind::Rust => collect_rust_members(root),
        ProjectKind::TypeScript => collect_ts_members(root),
        ProjectKind::Go => collect_go_members(root),
        ProjectKind::Python => Vec::new(),
        ProjectKind::Mixed => {
            let mut all = collect_rust_members(root);
            all.extend(collect_ts_members(root));
            all.extend(collect_go_members(root));
            all.sort_unstable();
            all.dedup();
            all
        }
        ProjectKind::Unknown => Vec::new(),
    };

    let do_not_create = members.clone();
    (members, do_not_create)
}

fn collect_rust_members(root: &Path) -> Vec<String> {
    let cargo_toml_path = root.join("Cargo.toml");
    let content = match std::fs::read_to_string(&cargo_toml_path) {
        Ok(content) => content,
        Err(err) => {
            warn_manifest_error("Cargo.toml", &cargo_toml_path, err);
            return Vec::new();
        }
    };

    let parsed: toml::Value = match toml::from_str(&content) {
        Ok(value) => value,
        Err(err) => {
            warn_manifest_error("Cargo.toml", &cargo_toml_path, err);
            return Vec::new();
        }
    };

    let Some(members_array) = parsed
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(|members| members.as_array())
    else {
        return Vec::new();
    };

    let mut names = Vec::new();
    for member_value in members_array {
        let Some(pattern) = member_value.as_str() else {
            continue;
        };

        if let Some(prefix) = pattern.strip_suffix("/*") {
            let dir = root.join(prefix);
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(err) => {
                    warn_manifest_error("workspace directory", &dir, err);
                    return Vec::new();
                }
            };

            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(err) => {
                        warn_manifest_error("workspace directory entry", &dir, err);
                        return Vec::new();
                    }
                };

                let entry_path = entry.path();
                if !entry_path.is_dir() {
                    continue;
                }

                let name = read_cargo_package_name(&entry_path).unwrap_or_else(|| {
                    entry_path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default()
                });
                if !name.is_empty() {
                    names.push(name);
                }
            }
        } else {
            let dir = root.join(pattern);
            if dir.is_dir() {
                let name = read_cargo_package_name(&dir).unwrap_or_else(|| {
                    dir.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default()
                });
                if !name.is_empty() {
                    names.push(name);
                }
            }
        }
    }

    names.sort_unstable();
    names.dedup();
    names
}

fn read_cargo_package_name(crate_dir: &Path) -> Option<String> {
    let cargo_toml = crate_dir.join("Cargo.toml");
    let content = std::fs::read_to_string(&cargo_toml).ok()?;
    let parsed: toml::Value = toml::from_str(&content).ok()?;
    parsed
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(|name| name.as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn collect_ts_members(root: &Path) -> Vec<String> {
    let pkg_path = root.join("package.json");
    let content = match std::fs::read_to_string(&pkg_path) {
        Ok(content) => content,
        Err(err) => {
            warn_manifest_error("package.json", &pkg_path, err);
            return Vec::new();
        }
    };

    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(err) => {
            warn_manifest_error("package.json", &pkg_path, err);
            return Vec::new();
        }
    };

    let patterns: Vec<String> = if let Some(array) = parsed
        .get("workspaces")
        .and_then(|workspaces| workspaces.as_array())
    {
        array
            .iter()
            .filter_map(|value| value.as_str())
            .map(str::to_owned)
            .collect()
    } else if let Some(array) = parsed
        .get("workspaces")
        .and_then(|workspaces| workspaces.get("packages"))
        .and_then(|packages| packages.as_array())
    {
        array
            .iter()
            .filter_map(|value| value.as_str())
            .map(str::to_owned)
            .collect()
    } else {
        return Vec::new();
    };

    let mut names = Vec::new();
    for pattern in &patterns {
        if let Some(prefix) = pattern.strip_suffix("/*") {
            let dir = root.join(prefix);
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(err) => {
                    warn_manifest_error("workspace directory", &dir, err);
                    return Vec::new();
                }
            };

            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(err) => {
                        warn_manifest_error("workspace directory entry", &dir, err);
                        return Vec::new();
                    }
                };

                let entry_path = entry.path();
                if !entry_path.is_dir() {
                    continue;
                }

                let name = read_npm_package_name(&entry_path).unwrap_or_else(|| {
                    entry_path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default()
                });
                if !name.is_empty() {
                    names.push(name);
                }
            }
        } else {
            let dir = root.join(pattern);
            if dir.is_dir() {
                let name = read_npm_package_name(&dir).unwrap_or_else(|| {
                    dir.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default()
                });
                if !name.is_empty() {
                    names.push(name);
                }
            }
        }
    }

    names.sort_unstable();
    names.dedup();
    names
}

fn read_npm_package_name(pkg_dir: &Path) -> Option<String> {
    let pkg_json = pkg_dir.join("package.json");
    let content = std::fs::read_to_string(&pkg_json).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    parsed
        .get("name")
        .and_then(|name| name.as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn collect_go_members(root: &Path) -> Vec<String> {
    let go_work_path = root.join("go.work");
    if let Ok(content) = std::fs::read_to_string(&go_work_path) {
        return parse_go_work_members(&content);
    }

    let go_mod_path = root.join("go.mod");
    let content = match std::fs::read_to_string(&go_mod_path) {
        Ok(content) => content,
        Err(err) => {
            warn_manifest_error("go.mod", &go_mod_path, err);
            return Vec::new();
        }
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(module) = trimmed.strip_prefix("module ") {
            let name = module.trim().to_owned();
            if !name.is_empty() {
                return vec![name];
            }
        }
    }

    Vec::new()
}

fn parse_go_work_members(content: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_use_block = false;

    for line in content.lines() {
        let trimmed = line.trim();
        let trimmed = trimmed
            .split_once("//")
            .map_or(trimmed, |(code, _)| code)
            .trim();

        if trimmed.starts_with("use (") {
            in_use_block = true;
            continue;
        }

        if in_use_block && trimmed == ")" {
            in_use_block = false;
            continue;
        }

        if in_use_block {
            let path = trimmed.trim_start_matches("./");
            push_go_member_name(&mut names, path);
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("use ") {
            let path = rest.trim().trim_start_matches("./");
            if !path.starts_with('(') {
                push_go_member_name(&mut names, path);
            }
        }
    }

    names.sort_unstable();
    names.dedup();
    names
}

fn push_go_member_name(names: &mut Vec<String>, path: &str) {
    if path.is_empty() || path.starts_with("//") {
        return;
    }

    match path.split('/').next_back() {
        Some(name) if !name.is_empty() => names.push(name.to_owned()),
        _ => {}
    }
}

fn warn_manifest_error(path_kind: &str, path: &Path, err: impl fmt::Display) {
    eprintln!(
        "warning: failed to read {path_kind} at {}: {err}",
        path.display()
    );
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

fn render_symbol_section(out: &mut String, title: &str, values: &[SymbolHit], max_visible: usize) {
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

#[cfg(test)]
mod repo_context_tests {
    use super::*;

    use std::fs;
    use std::path::{Path, PathBuf};

    fn write_file(path: impl AsRef<Path>, contents: &str) {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directories");
        }
        fs::write(path, contents).expect("write file");
    }

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repo root")
            .to_path_buf()
    }

    #[test]
    fn repo_context_detects_project_kinds() {
        let temp = tempfile::tempdir().expect("temp dir");

        assert_eq!(ProjectKind::detect(temp.path()), ProjectKind::Unknown);

        write_file(
            temp.path().join("Cargo.toml"),
            "[workspace]\nmembers = []\n",
        );
        assert_eq!(ProjectKind::detect(temp.path()), ProjectKind::Rust);

        write_file(temp.path().join("package.json"), "{ \"workspaces\": [] }");
        assert_eq!(ProjectKind::detect(temp.path()), ProjectKind::Mixed);

        fs::remove_file(temp.path().join("Cargo.toml")).expect("remove Cargo.toml");
        assert_eq!(ProjectKind::detect(temp.path()), ProjectKind::TypeScript);

        write_file(temp.path().join("go.mod"), "module example.com/demo\n");
        assert_eq!(ProjectKind::detect(temp.path()), ProjectKind::Mixed);

        fs::remove_file(temp.path().join("package.json")).expect("remove package.json");
        assert_eq!(ProjectKind::detect(temp.path()), ProjectKind::Go);

        fs::remove_file(temp.path().join("go.mod")).expect("remove go.mod");
        write_file(
            temp.path().join("pyproject.toml"),
            "[project]\nname = \"demo\"\n",
        );
        assert_eq!(ProjectKind::detect(temp.path()), ProjectKind::Python);
    }

    #[test]
    fn repo_context_reads_cargo_package_name() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_file(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"demo-crate\"\nversion = \"0.1.0\"\n",
        );

        assert_eq!(
            read_cargo_package_name(temp.path()),
            Some(String::from("demo-crate"))
        );
    }

    #[test]
    fn repo_context_collects_rust_workspace_members_from_roko_repo() {
        let root = repo_root();
        let (members, do_not_create) = collect_workspace_members(&root, &ProjectKind::Rust);

        assert_eq!(members, do_not_create);
        assert!(
            members.len() >= 18,
            "expected at least 18 workspace members"
        );
        assert!(members.iter().any(|member| member == "roko-cli"));
    }

    #[test]
    fn repo_context_collects_npm_workspace_members() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_file(
            temp.path().join("package.json"),
            r#"{
  "workspaces": ["packages/*"]
}"#,
        );
        write_file(
            temp.path().join("packages/app-one/package.json"),
            r#"{ "name": "app-one" }"#,
        );
        write_file(
            temp.path().join("packages/app-two/package.json"),
            r#"{ "name": "app-two" }"#,
        );

        let (members, do_not_create) =
            collect_workspace_members(temp.path(), &ProjectKind::TypeScript);
        assert_eq!(
            members,
            vec![String::from("app-one"), String::from("app-two")]
        );
        assert_eq!(members, do_not_create);
    }

    #[test]
    fn repo_context_collects_yarn_workspace_members() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_file(
            temp.path().join("package.json"),
            r#"{
  "workspaces": {
    "packages": ["packages/*"]
  }
}"#,
        );
        write_file(
            temp.path().join("packages/app-three/package.json"),
            r#"{ "name": "app-three" }"#,
        );

        let (members, do_not_create) =
            collect_workspace_members(temp.path(), &ProjectKind::TypeScript);
        assert_eq!(members, vec![String::from("app-three")]);
        assert_eq!(members, do_not_create);
    }

    #[test]
    fn repo_context_collects_go_workspace_members() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_file(
            temp.path().join("go.work"),
            r#"go 1.21

use (
    ./module-a
    ./module-b
)

use ./module-c
"#,
        );
        fs::create_dir_all(temp.path().join("module-a")).expect("create module-a");
        fs::create_dir_all(temp.path().join("module-b")).expect("create module-b");
        fs::create_dir_all(temp.path().join("module-c")).expect("create module-c");

        let (members, do_not_create) = collect_workspace_members(temp.path(), &ProjectKind::Go);
        assert_eq!(
            members,
            vec![
                String::from("module-a"),
                String::from("module-b"),
                String::from("module-c"),
            ]
        );
        assert_eq!(members, do_not_create);
    }

    #[test]
    fn repo_context_returns_empty_on_parse_error() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_file(temp.path().join("Cargo.toml"), "[workspace]\nmembers = [\n");

        let (members, do_not_create) = collect_workspace_members(temp.path(), &ProjectKind::Rust);
        assert!(members.is_empty());
        assert!(do_not_create.is_empty());
    }
}
