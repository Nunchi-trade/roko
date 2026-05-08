//! `roko plan` subcommand helpers for discovered plan directories.
//!
//! The canonical layout is `plans/<plan-id>/plan.md` with a sibling
//! `tasks.toml`. Legacy flat markdown files under `plans/` or `.roko/plans/`
//! are still discoverable, but new plans should use the directory layout.

use std::fs;
use std::path::{Path, PathBuf};

use roko_orchestrator::{DiscoveryError, PlanInfo, discover_plans};

/// Resolve the plans directory, preferring the top-level layout and falling
/// back to the legacy `.roko` location.
#[must_use]
pub fn plans_dir(workdir: &Path) -> PathBuf {
    let top = workdir.join("plans");
    if top.is_dir() {
        return top;
    }

    workdir.join(".roko").join("plans")
}

/// Return the stable orchestration id for a discovered plan.
#[must_use]
pub fn stable_plan_id(plan_info: &PlanInfo) -> &str {
    plan_info
        .frontmatter
        .as_ref()
        .and_then(|fm| fm.plan.as_deref())
        .filter(|plan_id| !plan_id.trim().is_empty())
        .unwrap_or(&plan_info.base)
}

/// Return the sibling `tasks.toml` path for a directory-style plan.
#[must_use]
pub fn tasks_path(plan_info: &PlanInfo) -> Option<PathBuf> {
    match plan_info.path.file_name().and_then(|name| name.to_str()) {
        Some("plan.md") => plan_info.path.parent().map(|dir| dir.join("tasks.toml")),
        Some("tasks.toml") => Some(plan_info.path.clone()),
        _ => None,
    }
}

/// A plan summary (used in list output).
#[derive(Debug, Clone)]
pub struct PlanSummary {
    /// Plan identifier (derived from the filename).
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Number of tasks in the plan.
    pub task_count: usize,
    /// Number of completed tasks observed for the plan.
    pub tasks_done: usize,
    /// Number of failed tasks observed for the plan.
    pub tasks_failed: usize,
    /// Whether the plan has been completed.
    pub completed: bool,
    /// Plan lifecycle status from `[meta].status`, when present.
    pub status: String,
    /// Replacement queue or plan declared by `[meta].superseded_by`.
    pub superseded_by: Option<String>,
    /// Whether the plan's `tasks.toml` is missing modern fields.
    pub old_format: bool,
    /// Last error message from executor state, if any.
    pub last_error: Option<String>,
}

impl PlanSummary {
    #[must_use]
    pub fn status_label(&self) -> String {
        match self.status.as_str() {
            "superseded" => "superseded".to_string(),
            "archived" => "archived".to_string(),
            _ if self.completed => "done".to_string(),
            _ if self.tasks_done > 0 || self.tasks_failed > 0 => "in-progress".to_string(),
            _ => "pending".to_string(),
        }
    }
}

impl std::fmt::Display for PlanSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let status_label = if matches!(self.status.as_str(), "superseded" | "archived") {
            let replacement = self
                .superseded_by
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(|value| format!(" by {value}"))
                .unwrap_or_default();
            format!("{}{replacement}", self.status)
        } else if self.completed {
            format!("done {}/{}", self.tasks_done, self.task_count)
        } else if self.tasks_done > 0 || self.tasks_failed > 0 {
            let mut parts = Vec::new();
            if self.tasks_done > 0 {
                parts.push(format!("{} done", self.tasks_done));
            }
            if self.tasks_failed > 0 {
                parts.push(format!("{} failed", self.tasks_failed));
            }
            let in_progress = self
                .task_count
                .saturating_sub(self.tasks_done)
                .saturating_sub(self.tasks_failed);
            if in_progress > 0 {
                parts.push(format!("{in_progress} remaining"));
            }
            format!(
                "{}/{} ({})",
                self.tasks_done,
                self.task_count,
                parts.join(", ")
            )
        } else if self.task_count > 0 {
            format!("pending 0/{}", self.task_count)
        } else {
            "pending".to_string()
        };
        let icon = if self.old_format { "⚠ " } else { "" };
        let error_hint = self
            .last_error
            .as_deref()
            .map(|e| format!("  err: {e}"))
            .unwrap_or_default();
        write!(
            f,
            "{icon}{:<16} {:<40} tasks={:<4} [{status_label}]{error_hint}",
            self.id, self.title, self.task_count,
        )
    }
}

/// A plan task entry.
#[derive(Debug, Clone)]
pub struct PlanTask {
    /// Task identifier within the plan.
    pub id: String,
    /// Description of what this task does.
    pub description: String,
    /// IDs of tasks this one depends on.
    pub depends_on: Vec<String>,
    /// Files this task is expected to modify.
    pub files: Vec<String>,
    /// Whether this task has been completed.
    pub completed: bool,
}

/// A full plan document.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Plan identifier.
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Detailed description of the plan's goal.
    pub description: String,
    /// Ordered list of tasks.
    pub tasks: Vec<PlanTask>,
}

impl Plan {
    /// Create a new empty plan.
    #[must_use]
    pub const fn new(id: String, title: String, description: String) -> Self {
        Self {
            id,
            title,
            description,
            tasks: Vec::new(),
        }
    }

    /// Add a task to the plan.
    pub fn add_task(&mut self, task: PlanTask) {
        self.tasks.push(task);
    }

    /// Summarize this plan.
    #[must_use]
    pub fn summary(&self) -> PlanSummary {
        let completed = !self.tasks.is_empty() && self.tasks.iter().all(|t| t.completed);
        PlanSummary {
            id: self.id.clone(),
            title: self.title.clone(),
            task_count: self.tasks.len(),
            tasks_done: self.tasks.iter().filter(|task| task.completed).count(),
            tasks_failed: 0,
            completed,
            status: if completed { "done" } else { "ready" }.into(),
            superseded_by: None,
            old_format: false,
            last_error: None,
        }
    }

    /// Validate the plan for internal consistency.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if self.id.is_empty() {
            errors.push("plan id must not be empty".into());
        }
        if self.title.is_empty() {
            errors.push("plan title must not be empty".into());
        }

        let task_ids: Vec<&str> = self.tasks.iter().map(|t| t.id.as_str()).collect();
        for task in &self.tasks {
            for dep in &task.depends_on {
                if !task_ids.contains(&dep.as_str()) {
                    errors.push(format!(
                        "task '{}' depends on unknown task '{}'",
                        task.id, dep
                    ));
                }
            }
            // Check for self-dependency.
            if task.depends_on.contains(&task.id) {
                errors.push(format!("task '{}' depends on itself", task.id));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// List plan files in the plans directory.
pub fn list_plan_files(workdir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let dir = plans_dir(workdir);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut plans = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let plan_md = path.join("plan.md");
            if plan_md.is_file() {
                plans.push(plan_md);
                continue;
            }
            let tasks_toml = path.join("tasks.toml");
            if tasks_toml.is_file() {
                plans.push(tasks_toml);
            }
            continue;
        }
        if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        {
            plans.push(path);
        }
    }
    plans.sort();
    Ok(plans)
}

/// Build a display summary from a discovered plan entry.
#[must_use]
pub fn summarize_plan_info(plan_info: &PlanInfo) -> PlanSummary {
    let title = if plan_info
        .path
        .file_name()
        .is_some_and(|name| name == "tasks.toml")
    {
        stable_plan_id(plan_info).to_string()
    } else {
        fs::read_to_string(&plan_info.path)
            .ok()
            .and_then(|content| extract_markdown_title(&content))
            .unwrap_or_else(|| stable_plan_id(plan_info).to_string())
    };
    let tasks_path = tasks_path(plan_info);
    let mut meta_status = String::new();
    let mut superseded_by = None;
    let (task_count, tasks_done, tasks_failed, old_format) = match tasks_path.as_deref() {
        Some(path) if path.is_file() => {
            let old_format = matches!(
                crate::task_parser::TasksFile::validate_modern_fields(path),
                Ok(issues) if !issues.is_empty()
            );
            match crate::task_parser::TasksFile::parse(path) {
                Ok(tasks_file) => {
                    meta_status = tasks_file.meta.status.clone();
                    superseded_by = tasks_file.meta.superseded_by.clone();
                    let total = tasks_file.tasks.len();
                    let done = tasks_file
                        .tasks
                        .iter()
                        .filter(|t| {
                            matches!(
                                t.status.as_str(),
                                "done" | "completed" | "passed" | "skipped"
                            )
                        })
                        .count();
                    let failed = tasks_file
                        .tasks
                        .iter()
                        .filter(|t| {
                            matches!(t.status.as_str(), "failed" | "error" | "gate_rejected")
                        })
                        .count();
                    (total, done, failed, old_format)
                }
                Err(_) => (0, 0, 0, old_format),
            }
        }
        _ => (0, 0, 0, false),
    };

    let completed = task_count > 0 && tasks_done == task_count;
    let status = if meta_status.trim().is_empty() {
        if completed { "done" } else { "ready" }.to_string()
    } else {
        meta_status
    };
    PlanSummary {
        id: stable_plan_id(plan_info).to_string(),
        title,
        task_count,
        tasks_done,
        tasks_failed,
        completed,
        status,
        superseded_by,
        old_format,
        last_error: None,
    }
}

/// Discover plans from the canonical plans directory and summarize them.
///
/// # Errors
///
/// Returns any discovery error reported by [`discover_plans`].
pub fn summarize_discovered_plans(workdir: &Path) -> Result<Vec<PlanSummary>, DiscoveryError> {
    let plans = discover_plans(&plans_dir(workdir))?;
    Ok(plans.iter().map(summarize_plan_info).collect())
}

/// Find a discovered plan by its stable id or base directory name.
///
/// # Errors
///
/// Returns any discovery error reported by [`discover_plans`].
pub fn discover_plan_by_id(
    workdir: &Path,
    plan_id: &str,
) -> Result<Option<PlanInfo>, DiscoveryError> {
    let plans = discover_plans(&plans_dir(workdir))?;
    Ok(plans
        .into_iter()
        .find(|plan_info| stable_plan_id(plan_info) == plan_id || plan_info.base == plan_id))
}

fn extract_markdown_title(contents: &str) -> Option<String> {
    let body = strip_frontmatter(contents);
    body.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix('#').map(str::trim))
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
}

fn strip_frontmatter(contents: &str) -> &str {
    let stripped = contents.strip_prefix('\u{FEFF}').unwrap_or(contents);
    let trimmed = stripped.trim_start();
    if !trimmed.starts_with("---") {
        return stripped;
    }

    let after_open = &trimmed[3..];
    let close_pos_lf = after_open.find("\n---");
    let close_pos_crlf = after_open.find("\r\n---");
    let Some(close_pos) = (match (close_pos_lf, close_pos_crlf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }) else {
        return stripped;
    };

    let after_close = &after_open[close_pos..];
    if let Some(rest) = after_close.strip_prefix("\r\n---") {
        rest.trim_start_matches(['\r', '\n'])
    } else if let Some(rest) = after_close.strip_prefix("\n---") {
        rest.trim_start_matches(['\r', '\n'])
    } else {
        stripped
    }
}

/// Format a list of plan summaries for display.
#[must_use]
pub fn format_plan_list(plans: &[PlanSummary]) -> String {
    if plans.is_empty() {
        return "no plans found".to_string();
    }
    let mut lines = vec![format!(
        "{:<16} {:<40} {:<10} {}",
        "ID", "TITLE", "TASKS", "STATUS"
    )];
    for p in plans {
        lines.push(p.to_string());
    }
    lines.join("\n")
}

/// Format a list of plan summaries as JSON.
#[must_use]
pub fn format_plan_list_json(plans: &[PlanSummary]) -> String {
    let entries: Vec<String> = plans
        .iter()
        .map(|p| {
            format!(
                r#"{{"id":"{}","title":"{}","task_count":{},"tasks_done":{},"tasks_failed":{},"completed":{},"status":"{}","superseded_by":"{}","old_format":{}}}"#,
                p.id,
                p.title.replace('"', "\\\""),
                p.task_count,
                p.tasks_done,
                p.tasks_failed,
                p.completed,
                p.status.replace('"', "\\\""),
                p.superseded_by
                    .as_deref()
                    .unwrap_or("")
                    .replace('"', "\\\""),
                p.old_format,
            )
        })
        .collect();
    format!("[{}]", entries.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_plan() -> Plan {
        let mut plan = Plan::new("p1".into(), "Test Plan".into(), "A test plan".into());
        plan.add_task(PlanTask {
            id: "t1".into(),
            description: "First task".into(),
            depends_on: vec![],
            files: vec!["src/main.rs".into()],
            completed: false,
        });
        plan.add_task(PlanTask {
            id: "t2".into(),
            description: "Second task".into(),
            depends_on: vec!["t1".into()],
            files: vec!["src/lib.rs".into()],
            completed: false,
        });
        plan
    }

    #[test]
    fn plan_summary_reflects_tasks() {
        let plan = sample_plan();
        let summary = plan.summary();
        assert_eq!(summary.id, "p1");
        assert_eq!(summary.task_count, 2);
        assert!(!summary.completed);
    }

    #[test]
    fn plan_summary_completed_when_all_tasks_done() {
        let mut plan = sample_plan();
        for task in &mut plan.tasks {
            task.completed = true;
        }
        assert!(plan.summary().completed);
    }

    #[test]
    fn plan_summary_not_completed_with_empty_tasks() {
        let plan = Plan::new("p".into(), "Empty".into(), "".into());
        assert!(!plan.summary().completed);
    }

    #[test]
    fn validate_ok_for_valid_plan() {
        let plan = sample_plan();
        assert!(plan.validate().is_ok());
    }

    #[test]
    fn validate_catches_unknown_dependency() {
        let mut plan = sample_plan();
        plan.tasks[0].depends_on.push("nonexistent".into());
        let errors = plan.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("nonexistent")));
    }

    #[test]
    fn validate_catches_self_dependency() {
        let mut plan = sample_plan();
        plan.tasks[0].depends_on.push("t1".into());
        let errors = plan.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("depends on itself")));
    }

    #[test]
    fn validate_catches_empty_id() {
        let plan = Plan::new(String::new(), "title".into(), "desc".into());
        let errors = plan.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("id must not be empty")));
    }

    #[test]
    fn tasks_path_accepts_task_only_plan_info() {
        let info = PlanInfo {
            base: "P08-example".into(),
            num: "P08".into(),
            path: PathBuf::from("/repo/plans/P08-example/tasks.toml"),
            frontmatter: None,
        };

        assert_eq!(
            tasks_path(&info),
            Some(PathBuf::from("/repo/plans/P08-example/tasks.toml"))
        );
    }

    #[test]
    fn format_plan_list_empty() {
        assert_eq!(format_plan_list(&[]), "no plans found");
    }

    #[test]
    fn format_plan_list_shows_entries() {
        let summaries = vec![PlanSummary {
            id: "p1".into(),
            title: "Test".into(),
            task_count: 3,
            tasks_done: 0,
            tasks_failed: 0,
            completed: false,
            status: "ready".into(),
            superseded_by: None,
            old_format: false,
            last_error: None,
        }];
        let text = format_plan_list(&summaries);
        assert!(text.contains("p1"));
        assert!(text.contains("Test"));
        assert!(text.contains("pending 0/3"));
    }

    #[test]
    fn format_plan_list_json_structure() {
        let summaries = vec![
            PlanSummary {
                id: "a".into(),
                title: "Alpha".into(),
                task_count: 1,
                tasks_done: 0,
                tasks_failed: 0,
                completed: true,
                status: "done".into(),
                superseded_by: None,
                old_format: false,
                last_error: None,
            },
            PlanSummary {
                id: "b".into(),
                title: "Beta".into(),
                task_count: 2,
                tasks_done: 0,
                tasks_failed: 0,
                completed: false,
                status: "ready".into(),
                superseded_by: None,
                old_format: true,
                last_error: None,
            },
        ];
        let json = format_plan_list_json(&summaries);
        assert!(json.starts_with('['));
        assert!(json.ends_with(']'));
        assert!(json.contains(r#""id":"a""#));
        assert!(json.contains(r#""completed":false"#));
        assert!(json.contains(r#""old_format":true"#));
    }

    #[test]
    fn uses_legacy_location_when_top_level_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".roko").join("plans");
        std::fs::create_dir_all(&dir).unwrap();

        assert_eq!(plans_dir(tmp.path()), dir);
    }

    #[test]
    fn falls_back_to_legacy_location() {
        let dir = plans_dir(Path::new("/project"));
        assert_eq!(dir, PathBuf::from("/project/.roko/plans"));
    }

    #[test]
    fn prefers_top_level_location_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let top_level = tmp.path().join("plans");
        std::fs::create_dir_all(&top_level).unwrap();

        assert_eq!(plans_dir(tmp.path()), top_level);
    }

    #[test]
    fn list_plan_files_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let plans = list_plan_files(tmp.path()).unwrap();
        assert!(plans.is_empty());
    }

    #[test]
    fn list_plan_files_finds_directory_and_legacy_markdown_plans() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".roko").join("plans");
        std::fs::create_dir_all(dir.join("plan1")).unwrap();
        std::fs::create_dir_all(dir.join("plan3")).unwrap();
        std::fs::write(dir.join("plan1").join("plan.md"), "# Plan One\n").unwrap();
        std::fs::write(dir.join("plan2.md"), "# Plan Two\n").unwrap();
        std::fs::write(
            dir.join("plan3").join("tasks.toml"),
            "[meta]\nplan = \"plan3\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();

        let plans = list_plan_files(tmp.path()).unwrap();
        assert_eq!(plans.len(), 3);
        assert!(plans.iter().any(|p| p.ends_with("plan1/plan.md")));
        assert!(plans.iter().any(|p| p.ends_with("plan2.md")));
        assert!(plans.iter().any(|p| p.ends_with("plan3/tasks.toml")));
    }

    #[test]
    fn list_plan_files_reads_top_level_dir_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("plans");
        std::fs::create_dir_all(dir.join("plan1")).unwrap();
        std::fs::write(dir.join("plan1").join("plan.md"), "# Plan One\n").unwrap();

        let plans = list_plan_files(tmp.path()).unwrap();
        assert_eq!(plans.len(), 1);
        assert!(plans[0].ends_with("plan1/plan.md"));
    }

    #[test]
    fn plan_summary_display_done() {
        let summary = PlanSummary {
            id: "p1".into(),
            title: "My Plan".into(),
            task_count: 5,
            tasks_done: 5,
            tasks_failed: 0,
            completed: true,
            status: "done".into(),
            superseded_by: None,
            old_format: true,
            last_error: None,
        };
        let text = summary.to_string();
        assert!(text.contains("p1"));
        assert!(text.contains("My Plan"));
        assert!(text.contains("done 5/5"));
        assert!(text.contains("⚠"));
    }

    #[test]
    fn plan_summary_display_partial_progress() {
        let summary = PlanSummary {
            id: "p2".into(),
            title: "Partial Plan".into(),
            task_count: 5,
            tasks_done: 2,
            tasks_failed: 1,
            completed: false,
            status: "ready".into(),
            superseded_by: None,
            old_format: false,
            last_error: None,
        };
        let text = summary.to_string();
        assert!(text.contains("2/5"));
        assert!(text.contains("2 done"));
        assert!(text.contains("1 failed"));
        assert!(text.contains("2 remaining"));
    }

    #[test]
    fn plan_summary_display_pending() {
        let summary = PlanSummary {
            id: "p3".into(),
            title: "Fresh Plan".into(),
            task_count: 3,
            tasks_done: 0,
            tasks_failed: 0,
            completed: false,
            status: "ready".into(),
            superseded_by: None,
            old_format: false,
            last_error: None,
        };
        let text = summary.to_string();
        assert!(text.contains("pending 0/3"));
    }

    #[test]
    fn plan_summary_display_superseded() {
        let summary = PlanSummary {
            id: "old".into(),
            title: "Old Plan".into(),
            task_count: 3,
            tasks_done: 0,
            tasks_failed: 0,
            completed: false,
            status: "superseded".into(),
            superseded_by: Some("new-plan".into()),
            old_format: false,
            last_error: None,
        };

        assert_eq!(summary.status_label(), "superseded");
        assert!(summary.to_string().contains("superseded by new-plan"));
    }

    #[test]
    fn strip_frontmatter_leaves_markdown_body() {
        let body = strip_frontmatter("---\nplan: alpha\n---\n# Hello\n");
        assert_eq!(body, "# Hello\n");
    }

    #[test]
    fn summarize_plan_info_uses_heading_and_tasks_count() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("plans").join("01-alpha");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plan.md"),
            "---\nplan: alpha\n---\n# Alpha Title\n\nBody\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("tasks.toml"),
            "[meta]\nplan = \"alpha\"\nmax_parallel = 1\n\n[[task]]\nid = \"t1\"\ntitle = \"Task\"\nverify = []\n",
        )
        .unwrap();

        let plan_info = discover_plan_by_id(tmp.path(), "alpha")
            .unwrap()
            .expect("plan");
        let summary = summarize_plan_info(&plan_info);

        assert_eq!(summary.id, "alpha");
        assert_eq!(summary.title, "Alpha Title");
        assert_eq!(summary.task_count, 1);
    }
}
