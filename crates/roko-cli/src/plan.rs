//! `roko plan` subcommand group — list, show, and create plans.
//!
//! Plans are declarative task graphs stored as TOML/JSON files under
//! `plans/` or the legacy `.roko/plans/`. Each plan describes a set of tasks
//! with dependencies,
//! assigned agent roles, and gate requirements.

use std::path::{Path, PathBuf};

pub use crate::workspace_paths::plans_dir;

/// A plan summary (used in list output).
#[derive(Debug, Clone)]
pub struct PlanSummary {
    /// Plan identifier (derived from the filename).
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Number of tasks in the plan.
    pub task_count: usize,
    /// Whether the plan has been completed.
    pub completed: bool,
    /// Whether the plan's `tasks.toml` is missing modern fields.
    pub old_format: bool,
    /// Last error message from executor state, if any.
    pub last_error: Option<String>,
}

impl std::fmt::Display for PlanSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let status = if self.completed { "done" } else { "pending" };
        let icon = if self.old_format { "⚠ " } else { "" };
        write!(
            f,
            "{icon}{:<16} {:<40} tasks={:<4} [{}]",
            self.id, self.title, self.task_count, status
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
            completed,
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
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|ext| ext == "toml" || ext == "json")
        {
            plans.push(path);
        }
    }
    plans.sort();
    Ok(plans)
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
                r#"{{"id":"{}","title":"{}","task_count":{},"completed":{},"old_format":{}}}"#,
                p.id,
                p.title.replace('"', "\\\""),
                p.task_count,
                p.completed,
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
    fn format_plan_list_empty() {
        assert_eq!(format_plan_list(&[]), "no plans found");
    }

    #[test]
    fn format_plan_list_shows_entries() {
        let summaries = vec![PlanSummary {
            id: "p1".into(),
            title: "Test".into(),
            task_count: 3,
            completed: false,
            old_format: false,
            last_error: None,
        }];
        let text = format_plan_list(&summaries);
        assert!(text.contains("p1"));
        assert!(text.contains("Test"));
        assert!(text.contains("pending"));
    }

    #[test]
    fn format_plan_list_json_structure() {
        let summaries = vec![
            PlanSummary {
                id: "a".into(),
                title: "Alpha".into(),
                task_count: 1,
                completed: true,
                old_format: false,
                last_error: None,
            },
            PlanSummary {
                id: "b".into(),
                title: "Beta".into(),
                task_count: 2,
                completed: false,
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
    fn plans_dir_uses_legacy_location_when_top_level_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".roko").join("plans");
        std::fs::create_dir_all(&dir).unwrap();

        assert_eq!(plans_dir(tmp.path()), dir);
    }

    #[test]
    fn plans_dir_falls_back_to_legacy_location() {
        let dir = plans_dir(Path::new("/project"));
        assert_eq!(dir, PathBuf::from("/project/.roko/plans"));
    }

    #[test]
    fn plans_dir_prefers_top_level_location_when_present() {
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
    fn list_plan_files_finds_toml_and_json_in_legacy_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".roko").join("plans");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("plan1.toml"), "").unwrap();
        std::fs::write(dir.join("plan2.json"), "").unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap(); // should be excluded

        let plans = list_plan_files(tmp.path()).unwrap();
        assert_eq!(plans.len(), 2);
        assert!(plans.iter().any(|p| p.ends_with("plan1.toml")));
        assert!(plans.iter().any(|p| p.ends_with("plan2.json")));
    }

    #[test]
    fn list_plan_files_reads_top_level_dir_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("plans");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("plan1.toml"), "").unwrap();

        let plans = list_plan_files(tmp.path()).unwrap();
        assert_eq!(plans.len(), 1);
        assert!(plans[0].ends_with("plan1.toml"));
    }

    #[test]
    fn plan_summary_display() {
        let summary = PlanSummary {
            id: "p1".into(),
            title: "My Plan".into(),
            task_count: 5,
            completed: true,
            old_format: true,
            last_error: None,
        };
        let text = summary.to_string();
        assert!(text.contains("p1"));
        assert!(text.contains("My Plan"));
        assert!(text.contains("done"));
        assert!(text.contains("⚠"));
    }
}
