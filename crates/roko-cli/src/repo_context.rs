use anyhow::Result;
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
    /// Original feature keywords used to ground this repository context.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
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

/// Verify that feature keywords are grounded in the actual workspace.
///
/// Returns `true` when keywords match workspace members and at least one real
/// source file exists for the match.
#[must_use]
pub fn verify_context_root(
    root: &Path,
    keywords: &[&str],
    workspace_members: &[String],
    key_files: &[PathBuf],
) -> bool {
    let normalized_keywords: Vec<String> = keywords
        .iter()
        .map(|keyword| keyword.trim().to_lowercase())
        .filter(|keyword| !keyword.is_empty())
        .collect();

    if normalized_keywords.is_empty() || workspace_members.is_empty() {
        return false;
    }

    let keyword_matches_member = normalized_keywords.iter().any(|keyword| {
        workspace_members.iter().any(|member| {
            let member = member.trim().to_lowercase();
            !member.is_empty() && (member.contains(keyword) || keyword.contains(&member))
        })
    });

    if !keyword_matches_member {
        return false;
    }

    key_files.iter().any(|file| {
        let full_path = root.join(file);
        full_path.exists() && full_path.is_file() && is_verification_source_file(&full_path)
    })
}

fn context_verification_warning(keywords: &[&str], workspace_members: &[String]) -> String {
    let keywords: Vec<&str> = keywords
        .iter()
        .map(|keyword| keyword.trim())
        .filter(|keyword| !keyword.is_empty())
        .collect();
    let kw_list = keywords.join(", ");
    let members_preview: Vec<&str> = workspace_members
        .iter()
        .take(10)
        .map(String::as_str)
        .map(str::trim)
        .filter(|member| !member.is_empty())
        .collect();
    let members_list = members_preview.join(", ");
    let suffix = if workspace_members.len() > 10 {
        format!(" (and {} more)", workspace_members.len() - 10)
    } else {
        String::new()
    };

    format!(
        "> **WARNING: Repository context not verified.**\n\
         > The feature keywords [{}] do not match any workspace members [{}{}].\n\
         > Generated artifacts may reference nonexistent code. Review carefully before accepting.\n",
        kw_list, members_list, suffix
    )
}

fn is_verification_source_file(path: &Path) -> bool {
    const VERIFICATION_SOURCE_EXTENSIONS: &[&str] = &["rs", "ts", "tsx", "js", "jsx", "go", "py"];
    has_allowed_extension(path, VERIFICATION_SOURCE_EXTENSIONS)
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

        if !self.context_root_verified {
            let keyword_refs: Vec<&str> = self.keywords.iter().map(String::as_str).collect();
            let _ = write!(
                out,
                "{}",
                context_verification_warning(&keyword_refs, &self.workspace_members)
            );
        }

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

/// Build a complete RepoContextPack for the given workspace and feature keywords.
///
/// Time budget: 10 seconds. On timeout, returns partial results with warnings.
/// Missing files or directories produce incomplete packs, never errors.
pub async fn build_repo_context(
    workdir: &Path,
    feature_keywords: &[&str],
) -> Result<RepoContextPack> {
    let start = std::time::Instant::now();
    let time_budget = std::time::Duration::from_secs(10);
    let project_kind = ProjectKind::detect(workdir);
    let workdir = workdir.to_path_buf();
    let feature_keywords_owned = feature_keywords
        .iter()
        .map(|keyword| (*keyword).to_owned())
        .collect::<Vec<_>>();

    let mut workspace_members = Vec::new();
    let mut do_not_create = Vec::new();
    let mut key_files = Vec::new();
    let mut matching_symbols = Vec::new();
    let mut related_prds = Vec::new();
    let mut related_plans = Vec::new();
    let mut timed_out = false;

    if start.elapsed() < time_budget {
        let workdir = workdir.clone();
        match run_blocking_step_with_budget(start, time_budget, "workspace members", move || {
            collect_workspace_members(&workdir, &project_kind)
        })
        .await
        {
            Some((members, do_not_create_members)) => {
                workspace_members = members;
                do_not_create = do_not_create_members;
            }
            None => timed_out = true,
        }
    } else {
        timed_out = true;
    }

    if !timed_out && start.elapsed() < time_budget {
        let workdir = workdir.clone();
        let keywords = feature_keywords_owned.clone();
        match run_blocking_step_with_budget(start, time_budget, "key files", move || {
            let keyword_refs: Vec<&str> = keywords.iter().map(String::as_str).collect();
            find_key_files(&workdir, &keyword_refs, 20)
        })
        .await
        {
            Some(results) => key_files = results,
            None => timed_out = true,
        }
    }

    if !timed_out && start.elapsed() < time_budget {
        let workdir = workdir.clone();
        let keywords = feature_keywords_owned.clone();
        match run_blocking_step_with_budget(start, time_budget, "symbol matches", move || {
            let keyword_refs: Vec<&str> = keywords.iter().map(String::as_str).collect();
            find_symbol_matches(&workdir, &keyword_refs, 30)
        })
        .await
        {
            Some(results) => matching_symbols = results,
            None => timed_out = true,
        }
    }

    if !timed_out && start.elapsed() < time_budget {
        let workdir = workdir.clone();
        let keywords = feature_keywords_owned.clone();
        match run_blocking_step_with_budget(start, time_budget, "related PRDs", move || {
            let keyword_refs: Vec<&str> = keywords.iter().map(String::as_str).collect();
            find_related_prds(&workdir, &keyword_refs, 5)
        })
        .await
        {
            Some(results) => related_prds = results,
            None => timed_out = true,
        }
    }

    if !timed_out && start.elapsed() < time_budget {
        let workdir = workdir.clone();
        let keywords = feature_keywords_owned.clone();
        match run_blocking_step_with_budget(start, time_budget, "related plans", move || {
            let keyword_refs: Vec<&str> = keywords.iter().map(String::as_str).collect();
            find_related_plans(&workdir, &keyword_refs, 5)
        })
        .await
        {
            Some(results) => related_plans = results,
            None => timed_out = true,
        }
    }

    let feature_keyword_refs: Vec<&str> =
        feature_keywords_owned.iter().map(String::as_str).collect();
    let context_root_verified =
        verify_context_root(&workdir, &feature_keyword_refs, &workspace_members, &key_files);

    extend_do_not_create(&mut do_not_create, &workdir);

    if !timed_out && start.elapsed() >= time_budget {
        eprintln!(
            "warning: repo context build exceeded the 10-second budget; returning partial results"
        );
    }

    Ok(RepoContextPack {
        root: workdir,
        project_kind,
        workspace_members,
        key_files,
        matching_symbols,
        related_prds,
        related_plans,
        do_not_create,
        keywords: feature_keywords_owned,
        context_root_verified,
    })
}

/// Extend `do_not_create` with crate directory names that may not yet be in the workspace manifest.
fn extend_do_not_create(do_not_create: &mut Vec<String>, root: &Path) {
    for dir_name in ["crates", "packages", "libs"] {
        let dir = root.join(dir_name);
        if !dir.is_dir() {
            continue;
        }

        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };

            let name = name.to_owned();
            if !do_not_create.contains(&name) {
                do_not_create.push(name);
            }
        }
    }

    do_not_create.sort_unstable();
    do_not_create.dedup();
}

async fn run_blocking_step_with_budget<T, F>(
    start: std::time::Instant,
    time_budget: std::time::Duration,
    task_name: &'static str,
    task: F,
) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    if start.elapsed() >= time_budget {
        eprintln!(
            "warning: repo context build timed out before collecting {task_name}; returning partial results"
        );
        return None;
    }

    let handle = tokio::task::spawn_blocking(task);
    loop {
        if handle.is_finished() {
            return match handle.await {
                Ok(value) => Some(value),
                Err(err) => {
                    eprintln!("warning: repo context {task_name} task failed: {err}");
                    None
                }
            };
        }

        if start.elapsed() >= time_budget {
            handle.abort();
            eprintln!(
                "warning: repo context build timed out while collecting {task_name}; returning partial results"
            );
            return None;
        }

        tokio::task::yield_now().await;
    }
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

    #[test]
    fn verify_context_root_returns_true_for_matching_member_with_source() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_file(temp.path().join("crates/roko-compose/src/lib.rs"), "pub fn build() {}\n");

        let keywords = ["compose"];
        let workspace_members = vec![String::from("roko-compose")];
        let key_files = vec![PathBuf::from("crates/roko-compose/src/lib.rs")];

        assert!(verify_context_root(
            temp.path(),
            &keywords,
            &workspace_members,
            &key_files
        ));
    }

    #[test]
    fn verify_context_root_returns_false_for_empty_keywords() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_file(temp.path().join("crates/roko-compose/src/lib.rs"), "pub fn build() {}\n");

        let workspace_members = vec![String::from("roko-compose")];
        let key_files = vec![PathBuf::from("crates/roko-compose/src/lib.rs")];

        assert!(!verify_context_root(
            temp.path(),
            &[],
            &workspace_members,
            &key_files
        ));
    }

    #[test]
    fn verify_context_root_returns_false_for_empty_workspace_members() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_file(temp.path().join("crates/roko-compose/src/lib.rs"), "pub fn build() {}\n");

        let keywords = ["compose"];
        let key_files = vec![PathBuf::from("crates/roko-compose/src/lib.rs")];

        assert!(!verify_context_root(
            temp.path(),
            &keywords,
            &[],
            &key_files
        ));
    }

    #[test]
    fn verify_context_root_returns_false_when_keywords_do_not_match_member() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_file(temp.path().join("crates/roko-compose/src/lib.rs"), "pub fn build() {}\n");

        let keywords = ["agent"];
        let workspace_members = vec![String::from("roko-compose")];
        let key_files = vec![PathBuf::from("crates/roko-compose/src/lib.rs")];

        assert!(!verify_context_root(
            temp.path(),
            &keywords,
            &workspace_members,
            &key_files
        ));
    }

    #[test]
    fn verify_context_root_returns_false_when_source_file_is_missing() {
        let temp = tempfile::tempdir().expect("temp dir");

        let keywords = ["compose"];
        let workspace_members = vec![String::from("roko-compose")];
        let key_files = vec![PathBuf::from("crates/roko-compose/src/lib.rs")];

        assert!(!verify_context_root(
            temp.path(),
            &keywords,
            &workspace_members,
            &key_files
        ));
    }

    #[test]
    fn repo_context_prompt_section_warns_when_unverified() {
        let pack = RepoContextPack {
            root: PathBuf::from("/tmp/nowhere"),
            project_kind: ProjectKind::Rust,
            workspace_members: (0..12).map(|idx| format!("member-{idx}")).collect(),
            key_files: Vec::new(),
            matching_symbols: Vec::new(),
            related_prds: Vec::new(),
            related_plans: Vec::new(),
            do_not_create: Vec::new(),
            keywords: vec![String::from("roko-compose"), String::from("prompt assembly")],
            context_root_verified: false,
        };

        let section = pack.to_prompt_section();
        assert!(section.starts_with("> **WARNING: Repository context not verified.**"));
        assert!(section.contains("feature keywords [roko-compose, prompt assembly]"));
        assert!(section.contains("member-0, member-1"));
        assert!(section.contains("(and 2 more)"));
        assert!(section.contains("## Repository Context"));
    }

    #[test]
    fn find_related_prds_returns_empty_when_dir_missing() {
        let tmp = tempfile::tempdir().expect("temp dir");

        let results = find_related_prds(tmp.path(), &["usage"], 5);
        assert!(results.is_empty());
    }

    #[test]
    fn find_related_prds_matches_by_filename_and_caps_results() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let drafts = tmp.path().join(".roko").join("prd").join("drafts");
        fs::create_dir_all(&drafts).expect("create drafts dir");
        fs::write(drafts.join("usage-tracking.md"), "# Usage Tracking PRD\n").expect("write prd");
        fs::write(drafts.join("usage-metering.md"), "# Usage Metering PRD\n").expect("write prd");
        fs::write(drafts.join("unrelated.md"), "# Something Else\n").expect("write prd");

        let results = find_related_prds(tmp.path(), &["usage"], 1);
        assert_eq!(results.len(), 1);
        assert!(results[0].to_string_lossy().contains("usage-"));
        assert!(results.iter().all(|path| !path.is_absolute()));
    }

    #[test]
    fn find_related_prds_matches_by_content_preview() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let drafts = tmp.path().join(".roko").join("prd").join("drafts");
        fs::create_dir_all(&drafts).expect("create drafts dir");
        fs::write(
            drafts.join("neutral.md"),
            "# Scope\nThis PRD covers usage tracking for the workspace.\n",
        )
        .expect("write prd");

        let results = find_related_prds(tmp.path(), &["usage"], 5);
        assert!(!results.is_empty());
        assert!(results
            .iter()
            .any(|path| path.to_string_lossy().contains("neutral.md")));
        assert!(results.iter().all(|path| !path.is_absolute()));
    }

    #[test]
    fn find_related_prds_ignores_keywords_past_preview_limit() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let drafts = tmp.path().join(".roko").join("prd").join("drafts");
        fs::create_dir_all(&drafts).expect("create drafts dir");
        let contents = format!("{}usage\n", "a".repeat(500));
        fs::write(drafts.join("post-limit.md"), contents).expect("write prd");

        let results = find_related_prds(tmp.path(), &["usage"], 5);
        assert!(results.is_empty());
    }

    #[test]
    fn find_related_plans_returns_empty_when_dirs_missing() {
        let tmp = tempfile::tempdir().expect("temp dir");

        let results = find_related_plans(tmp.path(), &["router"], 5);
        assert!(results.is_empty());
    }

    #[test]
    fn find_related_plans_matches_by_directory_name() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let plans = tmp.path().join("plans");
        fs::create_dir_all(plans.join("cascade-router")).expect("create plan dir");
        fs::create_dir_all(plans.join("unrelated-feature")).expect("create plan dir");

        let results = find_related_plans(tmp.path(), &["router"], 1);
        assert_eq!(results.len(), 1);
        assert!(results[0].to_string_lossy().contains("cascade-router"));
        assert!(results.iter().all(|path| !path.is_absolute()));
    }

    #[test]
    fn find_related_plans_matches_by_tasks_preview_in_roko_plans() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let plans = tmp.path().join(".roko").join("plans");
        fs::create_dir_all(plans.join("unrelated-feature")).expect("create plan dir");
        fs::write(
            plans.join("unrelated-feature").join("tasks.toml"),
            "# Plan tasks\nrouter integration\n",
        )
        .expect("write tasks.toml");

        let results = find_related_plans(tmp.path(), &["router"], 5);
        assert!(!results.is_empty());
        assert!(results
            .iter()
            .any(|path| path.to_string_lossy().contains("unrelated-feature")));
        assert!(results.iter().all(|path| !path.is_absolute()));
    }

    #[tokio::test]
    async fn build_repo_context_returns_partial_pack_when_files_are_missing() {
        let tmp = tempfile::tempdir().expect("temp dir");

        let pack = build_repo_context(tmp.path(), &["compose"])
            .await
            .expect("build repo context");

        assert_eq!(pack.root, tmp.path());
        assert_eq!(pack.project_kind, ProjectKind::Unknown);
        assert_eq!(pack.keywords, vec![String::from("compose")]);
        assert!(pack.workspace_members.is_empty());
        assert!(pack.key_files.is_empty());
        assert!(pack.matching_symbols.is_empty());
        assert!(pack.related_prds.is_empty());
        assert!(pack.related_plans.is_empty());
        assert!(pack.do_not_create.is_empty());
        assert!(!pack.context_root_verified);
    }

    #[tokio::test]
    async fn build_repo_context_extends_do_not_create_and_verifies_root() {
        let tmp = tempfile::tempdir().expect("temp dir");
        write_file(
            tmp.path().join("Cargo.toml"),
            r#"[workspace]
members = ["crates/roko-compose"]
"#,
        );
        write_file(
            tmp.path().join("crates/roko-compose/Cargo.toml"),
            r#"[package]
name = "roko-compose"
version = "0.1.0"
"#,
        );
        write_file(
            tmp.path().join("crates/roko-compose/src/lib.rs"),
            "pub fn build_prompt() {}\n",
        );
        fs::create_dir_all(tmp.path().join("packages/extra")).expect("create extra dir");
        fs::create_dir_all(tmp.path().join("libs/sidecar")).expect("create sidecar dir");

        let pack = build_repo_context(tmp.path(), &["compose"])
            .await
            .expect("build repo context");

        assert_eq!(pack.project_kind, ProjectKind::Rust);
        assert_eq!(pack.keywords, vec![String::from("compose")]);
        assert_eq!(
            pack.workspace_members,
            vec![String::from("roko-compose")]
        );
        assert!(pack
            .key_files
            .iter()
            .any(|path| path.to_string_lossy().ends_with("crates/roko-compose/src/lib.rs")));
        assert!(pack.context_root_verified);
        assert_eq!(
            pack.do_not_create,
            vec![
                String::from("extra"),
                String::from("roko-compose"),
                String::from("sidecar"),
            ]
        );
    }
}

/// Directories to exclude from file and symbol search.
pub const EXCLUDED_DIRS: &[&str] = &["target", "node_modules", ".git", "tmp", "vendor", ".roko"];

/// Source file extensions to include in symbol search.
const SOURCE_EXTENSIONS: &[&str] = &["rs", "ts", "tsx", "js", "jsx", "go", "py", "toml", "json"];

/// Find key files whose names or contents match any of the feature keywords.
///
/// Relevance scoring is higher for stronger filename matches and lowest for
/// content-only matches.
#[must_use]
pub fn find_key_files(root: &Path, keywords: &[&str], max_files: usize) -> Vec<PathBuf> {
    if max_files == 0 {
        return Vec::new();
    }

    let keywords = normalize_keywords(keywords);
    if keywords.is_empty() {
        return Vec::new();
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut scored: Vec<(u8, PathBuf)> = Vec::new();

    walk_source_dir(root, root, &deadline, &mut |rel_path: &Path| {
        if std::time::Instant::now() >= deadline {
            return;
        }

        let abs_path = root.join(rel_path);
        let mut score = score_path_keywords(rel_path, &keywords);
        if score > 0 {
            if is_binary_path(&abs_path) {
                return;
            }
        } else if has_allowed_extension(rel_path, SOURCE_EXTENSIONS) {
            if is_binary_path(&abs_path) {
                return;
            }
            if file_contains_any_keyword(&abs_path, &keywords, &deadline) {
                score = 1;
            }
        }

        if score > 0 {
            scored.push((score, rel_path.to_path_buf()));
        }
    });

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.truncate(max_files);
    scored.into_iter().map(|(_, path)| path).collect()
}

/// Find symbol matches (lines containing keywords) in source files.
///
/// Definition-like lines are prioritized ahead of other matching lines. The
/// returned symbol text is truncated to 200 characters.
#[must_use]
pub fn find_symbol_matches(root: &Path, keywords: &[&str], max_hits: usize) -> Vec<SymbolHit> {
    if max_hits == 0 {
        return Vec::new();
    }

    let keywords = normalize_keywords(keywords);
    if keywords.is_empty() {
        return Vec::new();
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut definition_hits: Vec<SymbolHit> = Vec::new();
    let mut other_hits: Vec<SymbolHit> = Vec::new();

    walk_source_dir(root, root, &deadline, &mut |rel_path: &Path| {
        if std::time::Instant::now() >= deadline {
            return;
        }

        if !has_allowed_extension(rel_path, SOURCE_EXTENSIONS) {
            return;
        }

        let abs_path = root.join(rel_path);
        if is_binary_path(&abs_path) {
            return;
        }

        let Ok(file) = std::fs::File::open(&abs_path) else {
            return;
        };
        let mut reader = std::io::BufReader::new(file);
        let mut line = String::new();
        let mut line_idx = 0usize;

        loop {
            if std::time::Instant::now() >= deadline {
                return;
            }

            line.clear();
            let Ok(bytes_read) = std::io::BufRead::read_line(&mut reader, &mut line) else {
                return;
            };
            if bytes_read == 0 {
                break;
            }

            line_idx += 1;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
                continue;
            }

            let lowered = trimmed.to_lowercase();
            if !keywords.iter().any(|keyword| lowered.contains(keyword)) {
                continue;
            }

            let hit = SymbolHit {
                file: rel_path.to_path_buf(),
                line: line_idx as u32,
                text: trimmed.chars().take(200).collect(),
            };

            if is_definition_line(trimmed) {
                definition_hits.push(hit);
            } else {
                other_hits.push(hit);
            }

            if definition_hits.len() + other_hits.len() >= max_hits.saturating_mul(3) {
                break;
            }
        }
    });

    let mut result = definition_hits;
    result.extend(other_hits);
    result.truncate(max_hits);
    result
}

/// Recursive directory walker that skips `EXCLUDED_DIRS` and respects a deadline.
fn walk_source_dir<F>(root: &Path, dir: &Path, deadline: &std::time::Instant, cb: &mut F)
where
    F: FnMut(&Path),
{
    if std::time::Instant::now() >= *deadline {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    let mut sorted_entries: Vec<_> = entries.filter_map(Result::ok).collect();
    sorted_entries.sort_by_key(|entry| entry.path());

    for entry in sorted_entries {
        if std::time::Instant::now() >= *deadline {
            return;
        }

        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        let abs = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if file_type.is_dir() {
            if EXCLUDED_DIRS
                .iter()
                .any(|excluded| excluded.eq_ignore_ascii_case(name_str.as_ref()))
            {
                continue;
            }
            walk_source_dir(root, &abs, deadline, cb);
        } else if file_type.is_file() && let Ok(rel) = abs.strip_prefix(root) {
            cb(rel);
        }
    }
}

/// Returns true if the first 512 bytes of the file contain a null byte.
fn is_binary_path(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };

    let mut buffer = [0_u8; 512];
    let Ok(bytes_read) = std::io::Read::read(&mut file, &mut buffer) else {
        return false;
    };

    buffer[..bytes_read].contains(&0)
}

/// Find related PRDs by scanning `.roko/prd/drafts/` for keyword matches.
///
/// Matches by:
/// - Filename stem contains any keyword (score 2)
/// - First 500 bytes of file content contains any keyword (score 1)
///
/// Returns up to `max_results` paths, relative to `root`. Returns an empty
/// vec (no error) when `.roko/prd/drafts/` does not exist.
#[must_use]
pub fn find_related_prds(root: &Path, keywords: &[&str], max_results: usize) -> Vec<PathBuf> {
    if max_results == 0 {
        return Vec::new();
    }

    let keywords = normalize_keywords(keywords);
    if keywords.is_empty() {
        return Vec::new();
    }

    let prd_dir = root.join(".roko").join("prd").join("drafts");
    if !prd_dir.exists() {
        return Vec::new();
    }

    let Ok(entries) = std::fs::read_dir(&prd_dir) else {
        return Vec::new();
    };

    let mut scored: Vec<(u8, PathBuf)> = Vec::new();

    for entry in entries.flatten() {
        let abs = entry.path();
        if !abs.is_file() {
            continue;
        }

        let Some(extension) = abs.extension().and_then(|ext| ext.to_str()) else {
            continue;
        };
        if !extension.eq_ignore_ascii_case("md") {
            continue;
        }

        let rel = match abs.strip_prefix(root) {
            Ok(rel) => rel.to_path_buf(),
            Err(_) => continue,
        };

        let stem = abs
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("")
            .to_lowercase();

        let mut score: u8 = if keywords.iter().any(|keyword| stem.contains(keyword)) {
            2
        } else {
            0
        };

        if score == 0 {
            let Ok(mut file) = std::fs::File::open(&abs) else {
                continue;
            };

            let mut buffer = [0_u8; 500];
            let Ok(bytes_read) = std::io::Read::read(&mut file, &mut buffer) else {
                continue;
            };

            let preview = String::from_utf8_lossy(&buffer[..bytes_read]).to_lowercase();
            if keywords.iter().any(|keyword| preview.contains(keyword)) {
                score = 1;
            }
        }

        if score > 0 {
            scored.push((score, rel));
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.truncate(max_results);
    scored.into_iter().map(|(_, path)| path).collect()
}

/// Find related plans by scanning plan directories for keyword matches.
///
/// Searches both `.roko/plans/` and `plans/` (whichever exists). For each
/// subdirectory, matches by:
/// - Directory name contains any keyword (score 2)
/// - First 500 bytes of `tasks.toml` in that directory contains any keyword (score 1)
///
/// Returns up to `max_results` paths (relative to `root`). Returns empty
/// vec when neither plan directory exists.
#[must_use]
pub fn find_related_plans(root: &Path, keywords: &[&str], max_results: usize) -> Vec<PathBuf> {
    if max_results == 0 {
        return Vec::new();
    }

    let keywords = normalize_keywords(keywords);
    if keywords.is_empty() {
        return Vec::new();
    }

    let plan_dirs = [root.join(".roko").join("plans"), root.join("plans")];
    let mut scored: Vec<(u8, PathBuf)> = Vec::new();

    for plan_dir in &plan_dirs {
        if !plan_dir.exists() {
            continue;
        }

        let Ok(entries) = std::fs::read_dir(plan_dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let abs = entry.path();
            if !abs.is_dir() {
                continue;
            }

            let rel = match abs.strip_prefix(root) {
                Ok(rel) => rel.to_path_buf(),
                Err(_) => continue,
            };

            let dir_name = entry.file_name().to_string_lossy().to_lowercase();
            let mut score: u8 = if keywords.iter().any(|keyword| dir_name.contains(keyword)) {
                2
            } else {
                0
            };

            if score == 0 {
                let tasks_toml = abs.join("tasks.toml");
                let Ok(mut file) = std::fs::File::open(&tasks_toml) else {
                    continue;
                };

                let mut buffer = [0_u8; 500];
                let Ok(bytes_read) = std::io::Read::read(&mut file, &mut buffer) else {
                    continue;
                };

                let preview = String::from_utf8_lossy(&buffer[..bytes_read]).to_lowercase();
                if keywords.iter().any(|keyword| preview.contains(keyword)) {
                    score = 1;
                }
            }

            if score > 0 {
                scored.push((score, rel));
            }
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.truncate(max_results);
    scored.into_iter().map(|(_, path)| path).collect()
}

fn normalize_keywords(keywords: &[&str]) -> Vec<String> {
    let mut normalized: Vec<String> = keywords
        .iter()
        .map(|keyword| keyword.trim().to_lowercase())
        .filter(|keyword| !keyword.is_empty())
        .collect();
    normalized.sort_unstable();
    normalized.dedup();
    normalized
}

fn score_path_keywords(rel_path: &Path, keywords: &[String]) -> u8 {
    let stem = rel_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("")
        .to_lowercase();
    let rel_lower = rel_path.to_string_lossy().to_lowercase();

    let mut score = 0_u8;
    for keyword in keywords {
        let keyword = keyword.as_str();
        if stem == keyword {
            score = score.max(4);
        } else if stem.contains(keyword) {
            score = score.max(3);
        } else if rel_lower.contains(keyword) {
            score = score.max(2);
        }
    }

    score
}

fn has_allowed_extension(path: &Path, extensions: &[&str]) -> bool {
    let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };

    extensions
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(extension))
}

fn file_contains_any_keyword(path: &Path, keywords: &[String], deadline: &std::time::Instant) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };

    let mut reader = std::io::BufReader::new(file);
    let mut line = String::new();

    loop {
        if std::time::Instant::now() >= *deadline {
            return false;
        }

        line.clear();
        let Ok(bytes_read) = std::io::BufRead::read_line(&mut reader, &mut line) else {
            return false;
        };
        if bytes_read == 0 {
            return false;
        }

        let lowered = line.to_lowercase();
        if keywords.iter().any(|keyword| lowered.contains(keyword)) {
            return true;
        }
    }
}

fn is_definition_line(trimmed: &str) -> bool {
    trimmed.contains("fn ")
        || trimmed.contains("struct ")
        || trimmed.contains("enum ")
        || trimmed.contains("trait ")
        || trimmed.contains("type ")
        || trimmed.contains("impl ")
        || trimmed.starts_with("pub ")
}
