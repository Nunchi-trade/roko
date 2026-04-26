//! Plan discovery and loading — loads `tasks.toml` without scanning `.md`
//! files or applying enrichment.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tracing::info;

use crate::task_parser::TasksFile;

/// A loaded plan ready for execution.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Plan identifier (directory name).
    pub id: String,
    /// Directory containing this plan's `tasks.toml`.
    pub dir: PathBuf,
    /// Parsed task definitions.
    pub tasks: TasksFile,
}

/// Load a single plan from a directory that must contain `tasks.toml`.
pub fn load_plan(dir: &Path) -> Result<Plan> {
    let tasks_path = dir.join("tasks.toml");
    if !tasks_path.exists() {
        bail!(
            "No tasks.toml found in {}",
            dir.display()
        );
    }

    let id = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed".to_string());

    let tasks = TasksFile::parse(&tasks_path)
        .with_context(|| format!("failed to parse {}", tasks_path.display()))?;

    info!(plan_id = %id, task_count = tasks.tasks.len(), "loaded plan");
    Ok(Plan { id, dir: dir.to_path_buf(), tasks })
}

/// Load plan(s) from a directory.
///
/// - If `dir/tasks.toml` exists, returns a single plan rooted at `dir`.
/// - Otherwise, scans immediate subdirectories for `tasks.toml` files.
/// - Never scans `.md` files or modifies anything on disk.
pub fn load_plans(dir: &Path) -> Result<Vec<Plan>> {
    // Case 1: dir itself is a plan
    if dir.join("tasks.toml").exists() {
        return Ok(vec![load_plan(dir)?]);
    }

    // Case 2: scan subdirs
    let mut plans = Vec::new();
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("cannot read directory {}", dir.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && path.join("tasks.toml").exists() {
            match load_plan(&path) {
                Ok(plan) => plans.push(plan),
                Err(e) => {
                    tracing::warn!(dir = %path.display(), err = %e, "skipping plan with parse error");
                }
            }
        }
    }

    if plans.is_empty() {
        bail!("No plans found in {}", dir.display());
    }

    // Sort by name for deterministic ordering.
    plans.sort_by(|a, b| a.id.cmp(&b.id));

    info!(count = plans.len(), "discovered plans");
    Ok(plans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_tasks_toml(dir: &Path, content: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("tasks.toml"), content).unwrap();
    }

    const MINIMAL_TASKS: &str = r#"
[meta]
plan = "test-plan"

[[task]]
id = "T1"
title = "Do something"
"#;

    #[test]
    fn load_single_plan() {
        let tmp = tempfile::tempdir().unwrap();
        let plan_dir = tmp.path().join("my-plan");
        write_tasks_toml(&plan_dir, MINIMAL_TASKS);

        let plan = load_plan(&plan_dir).unwrap();
        assert_eq!(plan.id, "my-plan");
        assert_eq!(plan.tasks.tasks.len(), 1);
        assert_eq!(plan.tasks.meta.plan, "test-plan");
    }

    #[test]
    fn load_plan_missing_tasks_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let result = load_plan(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn load_plans_direct() {
        let tmp = tempfile::tempdir().unwrap();
        write_tasks_toml(tmp.path(), MINIMAL_TASKS);

        let plans = load_plans(tmp.path()).unwrap();
        assert_eq!(plans.len(), 1);
    }

    #[test]
    fn load_plans_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        write_tasks_toml(&tmp.path().join("plan-a"), MINIMAL_TASKS);
        write_tasks_toml(
            &tmp.path().join("plan-b"),
            &MINIMAL_TASKS.replace("test-plan", "plan-b"),
        );

        let plans = load_plans(tmp.path()).unwrap();
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].id, "plan-a");
        assert_eq!(plans[1].id, "plan-b");
    }

    #[test]
    fn load_plans_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = load_plans(tmp.path());
        assert!(result.is_err());
    }
}
