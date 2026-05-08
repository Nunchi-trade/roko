//! Plan discovery — scan a plans directory, parse YAML frontmatter,
//! return ranked [`PlanInfo`] entries (§1.1–§1.5).
//!
//! # Layout detection
//!
//! Two directory layouts are supported (the new one wins on conflict):
//!
//! - **New layout**: `plans/<num>-<slug>/plan.md`
//! - **Task-only layout**: `plans/<num>-<slug>/tasks.toml`
//! - **Legacy layout**: `plans/<num>-<slug>.md`
//!
//! The prefix (`<num>`) may be numeric (`01`) or numeric + alpha
//! (`08a`). Both are preserved in [`PlanInfo::num`] and sorted
//! lexicographically so `08` comes before `08a`.
//!
//! # Frontmatter contract
//!
//! Frontmatter lives between two `---` fences at the very top of
//! `plan.md`. All fields are optional — a plan without frontmatter
//! still discovers successfully with `frontmatter = None`. Malformed
//! YAML **fails loud** with [`DiscoveryError::BadFrontmatter`] rather
//! than silently dropping the plan.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Parsed YAML frontmatter from a plan file. All fields are optional so
/// that plans without frontmatter still round-trip through discovery.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanFrontmatter {
    /// Plan identifier, e.g. `"09-chain-layer"`.
    #[serde(default)]
    pub plan: Option<String>,
    /// Plan bases this plan depends on (must complete first).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Plan bases that can run in parallel with this one.
    #[serde(default)]
    pub parallel_with: Vec<String>,
    /// Crate directories this plan touches.
    #[serde(default)]
    pub crates_touched: Vec<String>,
    /// Estimated number of agent tasks.
    #[serde(default)]
    pub estimated_tasks: Option<usize>,
    /// Estimated maximum parallel agent sessions.
    #[serde(default)]
    pub estimated_parallel_width: Option<usize>,
    /// Total estimated wall-clock minutes for this plan.
    #[serde(default)]
    pub estimated_minutes: Option<u32>,
    /// Whether to run a refactor pass after this plan completes.
    #[serde(default)]
    pub refactor_after: bool,
    /// Whether this plan's tasks are safe to run in parallel with other
    /// plans. Defaults to `true` (matches Mori).
    #[serde(default = "default_true")]
    pub parallel_safe: bool,
    /// Priority for ranking (higher runs first). Tie-breaker is `num`.
    #[serde(default)]
    pub priority: Option<u32>,
    /// Free-form tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Milestone label.
    #[serde(default)]
    pub milestone: Option<String>,
}

const fn default_true() -> bool {
    true
}

/// One plan found on disk with its parsed frontmatter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanInfo {
    /// Full base name, e.g. `"01-workspace-scaffold"` or `"08a-whatever"`.
    pub base: String,
    /// Numeric / alphanumeric prefix, e.g. `"01"` or `"08a"`.
    pub num: String,
    /// Full path to `plan.md`, a task-only `tasks.toml`, or a legacy `.md` file.
    pub path: PathBuf,
    /// Parsed frontmatter. `None` when the file has no `---` fences.
    pub frontmatter: Option<PlanFrontmatter>,
}

impl PlanInfo {
    /// Short display name (the base).
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.base
    }
}

/// Errors returned by [`discover_plans`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DiscoveryError {
    /// The supplied plans directory does not exist.
    #[error("plans directory does not exist: {0}")]
    DirMissing(PathBuf),

    /// A plan file could not be read.
    #[error("failed to read plan {path}: {source}")]
    ReadFailed {
        /// The path that failed to read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// A plan had a frontmatter block but YAML parsing failed.
    #[error("invalid frontmatter in {path}: {reason}")]
    BadFrontmatter {
        /// The offending file.
        path: PathBuf,
        /// The parser's explanation.
        reason: String,
    },

    /// Validation on a parsed frontmatter failed.
    #[error("plan validation failed for {path}: {source}")]
    Invalid {
        /// The offending file.
        path: PathBuf,
        /// The validation error.
        #[source]
        source: ValidationError,
    },
}

/// Errors returned by [`validate_frontmatter`].
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValidationError {
    /// `plan` id was missing.
    #[error("frontmatter is missing required field `plan`")]
    MissingPlanId,
    /// `estimated_minutes` was set to 0 (or a negative deserialized value).
    #[error("`estimated_minutes` must be > 0 when present")]
    InvalidMinutes,
    /// `estimated_parallel_width` was set to 0 when present.
    #[error("`estimated_parallel_width` must be > 0 when present")]
    InvalidParallelWidth,
}

/// Scan `plans_dir` for plan files, parse each one, validate, and
/// return the discovered entries ordered by [`rank_plans`] rules.
///
/// The returned vector is empty when the directory has no plan files.
///
/// # Errors
///
/// - [`DiscoveryError::DirMissing`] if `plans_dir` does not exist.
/// - [`DiscoveryError::ReadFailed`] on I/O errors while reading a plan.
/// - [`DiscoveryError::BadFrontmatter`] on malformed YAML.
/// - [`DiscoveryError::Invalid`] if a parsed frontmatter fails validation.
pub fn discover_plans(plans_dir: &Path) -> Result<Vec<PlanInfo>, DiscoveryError> {
    if !plans_dir.exists() {
        return Err(DiscoveryError::DirMissing(plans_dir.to_path_buf()));
    }
    // Collect candidates first so we can enforce "new-layout wins" before
    // any I/O past metadata — directory entries are loaded before flat
    // `.md` entries so the dedup in the second pass is deterministic.
    let mut dir_candidates: Vec<(String, PathBuf)> = Vec::new();
    let mut file_candidates: Vec<(String, PathBuf)> = Vec::new();
    let read = fs::read_dir(plans_dir).map_err(|source| DiscoveryError::ReadFailed {
        path: plans_dir.to_path_buf(),
        source,
    })?;
    for entry in read {
        let entry = entry.map_err(|source| DiscoveryError::ReadFailed {
            path: plans_dir.to_path_buf(),
            source,
        })?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !starts_with_plan_prefix(&name) {
            continue;
        }
        // Skip well-known non-plan files that happen to start with an
        // alphanumeric character (e.g. INDEX.md, README.md, CONTEXT.md).
        let name_lower = name.to_ascii_lowercase();
        if matches!(
            name_lower.as_str(),
            "index" | "index.md" | "readme" | "readme.md" | "context" | "context.md"
        ) {
            continue;
        }
        let kind = entry
            .file_type()
            .map_err(|source| DiscoveryError::ReadFailed {
                path: entry.path(),
                source,
            })?;
        if kind.is_dir() {
            let plan_md = entry.path().join("plan.md");
            let tasks_toml = entry.path().join("tasks.toml");
            if plan_md.is_file() {
                dir_candidates.push((name, plan_md));
            } else if tasks_toml.is_file() {
                dir_candidates.push((name, tasks_toml));
            }
        } else if kind.is_file()
            && has_md_extension(&name)
            && !name.eq_ignore_ascii_case("CONTEXT.md")
        {
            let base = strip_md_extension(&name).to_string();
            file_candidates.push((base, entry.path()));
        }
    }
    let mut plans = Vec::new();
    for (base, path) in dir_candidates.into_iter().chain(file_candidates) {
        // New-layout dir wins over legacy flat file with the same base.
        if plans.iter().any(|p: &PlanInfo| p.base == base) {
            continue;
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => return Err(DiscoveryError::ReadFailed { path, source }),
        };
        let frontmatter =
            try_parse_frontmatter(&content).map_err(|reason| DiscoveryError::BadFrontmatter {
                path: path.clone(),
                reason,
            })?;
        if let Some(fm) = frontmatter.as_ref() {
            validate_frontmatter(fm).map_err(|source| DiscoveryError::Invalid {
                path: path.clone(),
                source,
            })?;
        }
        let num = plan_num(&base).to_string();
        plans.push(PlanInfo {
            base,
            num,
            path,
            frontmatter,
        });
    }
    rank_plans(&mut plans);
    Ok(plans)
}

/// Parse YAML frontmatter from a plan's text contents.
///
/// Returns `None` if the file has no `---` fence at the top, or
/// [`Some`] with the parsed frontmatter. The input is BOM-tolerant.
#[must_use]
pub fn parse_frontmatter(contents: &str) -> Option<PlanFrontmatter> {
    try_parse_frontmatter(contents).ok().flatten()
}

/// Internal: parse frontmatter returning a typed error on malformed YAML.
fn try_parse_frontmatter(contents: &str) -> Result<Option<PlanFrontmatter>, String> {
    let stripped = contents.strip_prefix('\u{FEFF}').unwrap_or(contents);
    let trimmed = stripped.trim_start();
    if !trimmed.starts_with("---") {
        return Ok(None);
    }
    // Everything after the opening `---`.
    let after_open = &trimmed[3..];
    // Tolerate CRLF line endings in addition to LF.
    let close_pos_lf = after_open.find("\n---");
    let close_pos_crlf = after_open.find("\r\n---");
    let close_pos = match (close_pos_lf, close_pos_crlf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    let Some(close_pos) = close_pos else {
        // Opening fence without close — treat as "no frontmatter".
        return Ok(None);
    };
    let block = &after_open[..close_pos];
    let fm: PlanFrontmatter = serde_yaml_ng::from_str(block).map_err(|e| e.to_string())?;
    Ok(Some(fm))
}

/// Sort plans by priority (descending) then by `num` (ascending).
pub fn rank_plans(plans: &mut [PlanInfo]) {
    plans.sort_by(|a, b| {
        let pri_a = a.frontmatter.as_ref().and_then(|f| f.priority).unwrap_or(0);
        let pri_b = b.frontmatter.as_ref().and_then(|f| f.priority).unwrap_or(0);
        pri_b
            .cmp(&pri_a)
            .then_with(|| a.num.cmp(&b.num))
            .then_with(|| a.base.cmp(&b.base))
    });
}

/// Check a parsed frontmatter for the minimum-required fields.
///
/// This is intentionally lax — only load-bearing invariants trigger
/// errors. Missing optional fields are fine.
///
/// # Errors
///
/// Returns a [`ValidationError`] if any invariant is broken.
pub fn validate_frontmatter(fm: &PlanFrontmatter) -> Result<(), ValidationError> {
    if fm.plan.as_deref().map(str::trim).is_some_and(str::is_empty) {
        return Err(ValidationError::MissingPlanId);
    }
    if let Some(minutes) = fm.estimated_minutes {
        if minutes == 0 {
            return Err(ValidationError::InvalidMinutes);
        }
    }
    if let Some(width) = fm.estimated_parallel_width {
        if width == 0 {
            return Err(ValidationError::InvalidParallelWidth);
        }
    }
    Ok(())
}

/// Does `name` start with an ASCII alphanumeric character (plan prefix)?
fn starts_with_plan_prefix(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
}

/// Extract the num prefix from a base name.
///
/// `"01-workspace-scaffold"` → `"01"`, `"08a-foo"` → `"08a"`.
fn plan_num(base: &str) -> &str {
    base.split('-').next().unwrap_or(base)
}

/// Case-insensitive check for a `.md` suffix.
fn has_md_extension(name: &str) -> bool {
    name.len() >= 3 && name.as_bytes()[name.len() - 3..].eq_ignore_ascii_case(b".md")
}

/// Strip a trailing `.md` / `.MD` / `.Md` suffix.
fn strip_md_extension(name: &str) -> &str {
    if has_md_extension(name) {
        &name[..name.len() - 3]
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_plan(root: &Path, base: &str, body: &str) {
        let dir = root.join(base);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("plan.md"), body).unwrap();
    }

    fn write_legacy_plan(root: &Path, base: &str, body: &str) {
        fs::write(root.join(format!("{base}.md")), body).unwrap();
    }

    fn write_tasks_only_plan(root: &Path, base: &str, body: &str) {
        let dir = root.join(base);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("tasks.toml"), body).unwrap();
    }

    #[test]
    fn missing_directory_errors_cleanly() {
        let err = discover_plans(Path::new("/definitely/not/real/plans")).unwrap_err();
        assert!(matches!(err, DiscoveryError::DirMissing(_)));
    }

    #[test]
    fn empty_directory_returns_empty_vec() {
        let dir = TempDir::new().unwrap();
        let plans = discover_plans(dir.path()).unwrap();
        assert!(plans.is_empty());
    }

    #[test]
    fn discovers_new_layout_plan() {
        let dir = TempDir::new().unwrap();
        write_plan(
            dir.path(),
            "01-workspace",
            "---\nplan: 01-workspace\n---\nBody",
        );
        let plans = discover_plans(dir.path()).unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].base, "01-workspace");
        assert_eq!(plans[0].num, "01");
        assert!(plans[0].path.ends_with("01-workspace/plan.md"));
        assert_eq!(
            plans[0].frontmatter.as_ref().unwrap().plan.as_deref(),
            Some("01-workspace")
        );
    }

    #[test]
    fn discovers_legacy_flat_file() {
        let dir = TempDir::new().unwrap();
        write_legacy_plan(dir.path(), "02-core", "---\nplan: 02-core\n---\n");
        let plans = discover_plans(dir.path()).unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].base, "02-core");
    }

    #[test]
    fn discovers_task_only_directory_plan() {
        let dir = TempDir::new().unwrap();
        write_tasks_only_plan(
            dir.path(),
            "02b-task-only",
            "[meta]\nplan = \"02b-task-only\"\nstatus = \"ready\"\n\n[[task]]\nid = \"T1\"\n",
        );
        let plans = discover_plans(dir.path()).unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].base, "02b-task-only");
        assert_eq!(plans[0].num, "02b");
        assert!(plans[0].path.ends_with("02b-task-only/tasks.toml"));
        assert!(plans[0].frontmatter.is_none());
    }

    #[test]
    fn new_layout_wins_over_legacy_flat_on_same_base() {
        let dir = TempDir::new().unwrap();
        write_plan(dir.path(), "03-foo", "---\nplan: 03-foo\n---\n");
        write_legacy_plan(dir.path(), "03-foo", "stale");
        let plans = discover_plans(dir.path()).unwrap();
        assert_eq!(plans.len(), 1);
        // The retained entry should point at the new-layout path.
        assert!(plans[0].path.ends_with("03-foo/plan.md"));
    }

    #[test]
    fn plan_without_frontmatter_is_discovered_with_none() {
        let dir = TempDir::new().unwrap();
        write_plan(dir.path(), "04-no-fm", "no frontmatter here\n");
        let plans = discover_plans(dir.path()).unwrap();
        assert_eq!(plans.len(), 1);
        assert!(plans[0].frontmatter.is_none());
    }

    #[test]
    fn bad_yaml_fails_loud() {
        let dir = TempDir::new().unwrap();
        write_plan(dir.path(), "05-broken", "---\nplan: [oops: unclosed\n---\n");
        let err = discover_plans(dir.path()).unwrap_err();
        assert!(matches!(err, DiscoveryError::BadFrontmatter { .. }));
    }

    #[test]
    fn alpha_suffix_prefix_is_preserved() {
        let dir = TempDir::new().unwrap();
        write_plan(dir.path(), "08a-variant", "");
        let plans = discover_plans(dir.path()).unwrap();
        assert_eq!(plans[0].num, "08a");
    }

    #[test]
    fn alpha_suffix_sorts_after_numeric() {
        let dir = TempDir::new().unwrap();
        write_plan(dir.path(), "08a-variant", "");
        write_plan(dir.path(), "08-base", "");
        write_plan(dir.path(), "09-next", "");
        let plans = discover_plans(dir.path()).unwrap();
        let nums: Vec<&str> = plans.iter().map(|p| p.num.as_str()).collect();
        assert_eq!(nums, vec!["08", "08a", "09"]);
    }

    #[test]
    fn bom_prefix_is_stripped_before_parse() {
        let dir = TempDir::new().unwrap();
        let body = "\u{FEFF}---\nplan: bom-plan\n---\n";
        write_plan(dir.path(), "10-bom", body);
        let plans = discover_plans(dir.path()).unwrap();
        assert_eq!(
            plans[0].frontmatter.as_ref().unwrap().plan.as_deref(),
            Some("bom-plan")
        );
    }

    #[test]
    fn priority_ordering_breaks_ties_by_num() {
        let dir = TempDir::new().unwrap();
        write_plan(dir.path(), "11-lo", "---\nplan: lo\npriority: 1\n---\n");
        write_plan(dir.path(), "12-hi", "---\nplan: hi\npriority: 10\n---\n");
        write_plan(dir.path(), "13-hi2", "---\nplan: hi2\npriority: 10\n---\n");
        let plans = discover_plans(dir.path()).unwrap();
        let bases: Vec<&str> = plans.iter().map(|p| p.base.as_str()).collect();
        // 10,10,1 → within the two 10s, 12 comes before 13.
        assert_eq!(bases, vec!["12-hi", "13-hi2", "11-lo"]);
    }

    #[test]
    fn directory_without_plan_md_is_skipped() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("20-empty")).unwrap();
        fs::write(dir.path().join("20-empty/README.md"), "not a plan").unwrap();
        let plans = discover_plans(dir.path()).unwrap();
        assert!(plans.is_empty());
    }

    #[test]
    fn context_md_is_skipped() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("CONTEXT.md"), "not a plan").unwrap();
        let plans = discover_plans(dir.path()).unwrap();
        assert!(plans.is_empty());
    }

    #[test]
    fn index_md_and_readme_md_are_skipped() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("INDEX.md"), "not a plan").unwrap();
        fs::write(dir.path().join("README.md"), "not a plan").unwrap();
        // Also test case-insensitive variants.
        fs::write(dir.path().join("index.md"), "not a plan").unwrap();
        write_plan(dir.path(), "01-real-plan", "---\nplan: real\n---\n");
        let plans = discover_plans(dir.path()).unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].base, "01-real-plan");
    }

    #[test]
    fn parse_frontmatter_parses_array_fields() {
        let body = "---\nplan: foo\ndepends_on: [01-a, 02-b]\ntags: [rust, orchestrator]\n---\n";
        let fm = parse_frontmatter(body).expect("frontmatter");
        assert_eq!(fm.depends_on, vec!["01-a", "02-b"]);
        assert_eq!(fm.tags, vec!["rust", "orchestrator"]);
    }

    #[test]
    fn parse_frontmatter_returns_none_when_no_fences() {
        assert!(parse_frontmatter("plain body\n").is_none());
    }

    #[test]
    fn parse_frontmatter_handles_crlf_line_endings() {
        let body = "---\r\nplan: winrt\r\n---\r\nbody\r\n";
        let fm = parse_frontmatter(body).expect("frontmatter");
        assert_eq!(fm.plan.as_deref(), Some("winrt"));
    }

    #[test]
    fn parallel_safe_defaults_to_true() {
        let body = "---\nplan: foo\n---\n";
        let fm = parse_frontmatter(body).expect("frontmatter");
        assert!(fm.parallel_safe);
    }

    #[test]
    fn parallel_safe_can_be_disabled() {
        let body = "---\nplan: foo\nparallel_safe: false\n---\n";
        let fm = parse_frontmatter(body).expect("frontmatter");
        assert!(!fm.parallel_safe);
    }

    #[test]
    fn validate_rejects_zero_minutes() {
        let fm = PlanFrontmatter {
            plan: Some("x".into()),
            estimated_minutes: Some(0),
            ..Default::default()
        };
        assert_eq!(
            validate_frontmatter(&fm).unwrap_err(),
            ValidationError::InvalidMinutes
        );
    }

    #[test]
    fn validate_rejects_zero_parallel_width() {
        let fm = PlanFrontmatter {
            plan: Some("x".into()),
            estimated_parallel_width: Some(0),
            ..Default::default()
        };
        assert_eq!(
            validate_frontmatter(&fm).unwrap_err(),
            ValidationError::InvalidParallelWidth
        );
    }

    #[test]
    fn validate_rejects_empty_plan_id() {
        let fm = PlanFrontmatter {
            plan: Some("   ".into()),
            ..Default::default()
        };
        assert_eq!(
            validate_frontmatter(&fm).unwrap_err(),
            ValidationError::MissingPlanId
        );
    }

    #[test]
    fn validate_accepts_minimal_valid() {
        let fm = PlanFrontmatter {
            plan: Some("ok".into()),
            estimated_minutes: Some(30),
            estimated_parallel_width: Some(4),
            ..Default::default()
        };
        validate_frontmatter(&fm).unwrap();
    }

    #[test]
    fn rank_plans_sorts_deterministically() {
        let mut plans = vec![
            PlanInfo {
                base: "02-b".into(),
                num: "02".into(),
                path: PathBuf::new(),
                frontmatter: None,
            },
            PlanInfo {
                base: "01-a".into(),
                num: "01".into(),
                path: PathBuf::new(),
                frontmatter: Some(PlanFrontmatter {
                    priority: Some(5),
                    ..Default::default()
                }),
            },
        ];
        rank_plans(&mut plans);
        assert_eq!(plans[0].num, "01"); // higher priority wins
        assert_eq!(plans[1].num, "02");
    }

    #[test]
    fn discovers_multiple_plans_in_order() {
        let dir = TempDir::new().unwrap();
        write_plan(dir.path(), "30-alpha", "");
        write_plan(dir.path(), "31-beta", "");
        write_plan(dir.path(), "32-gamma", "");
        let plans = discover_plans(dir.path()).unwrap();
        let nums: Vec<&str> = plans.iter().map(|p| p.num.as_str()).collect();
        assert_eq!(nums, vec!["30", "31", "32"]);
    }
}
