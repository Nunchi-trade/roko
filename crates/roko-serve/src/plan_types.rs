//! Local plan data types for the HTTP routes.
//!
//! These are pure data structs mirroring `roko_cli::plan::{Plan, PlanTask}`
//! so that `roko-serve` can work with plans without depending on `roko-cli`.

/// A full plan document.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Stable plan identifier.
    pub id: String,
    /// Human-readable plan title.
    pub title: String,
    /// Longer description of the plan goal and scope.
    pub description: String,
    /// Ordered task list belonging to the plan.
    pub tasks: Vec<PlanTask>,
}

impl Plan {
    /// Construct an empty plan with the provided metadata.
    #[must_use]
    pub const fn new(id: String, title: String, description: String) -> Self {
        Self {
            id,
            title,
            description,
            tasks: Vec::new(),
        }
    }

    /// Append one task to the plan's ordered task list.
    pub fn add_task(&mut self, task: PlanTask) {
        self.tasks.push(task);
    }

    /// Validate the plan for common issues.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if self.id.trim().is_empty() {
            errors.push("plan id must not be empty".into());
        }
        if self.title.trim().is_empty() {
            errors.push("plan title must not be empty".into());
        }
        for task in &self.tasks {
            if task.id.trim().is_empty() {
                errors.push(format!(
                    "task id must not be empty (description: {})",
                    task.description
                ));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// A plan task entry.
#[derive(Debug, Clone)]
pub struct PlanTask {
    /// Stable task identifier within the plan.
    pub id: String,
    /// Human-readable task description.
    pub description: String,
    /// IDs of tasks that must complete before this task can start.
    pub depends_on: Vec<String>,
    /// Files or paths expected to be touched by the task.
    pub files: Vec<String>,
    /// Whether the task has already been completed.
    pub completed: bool,
}
