## Batch P2B: TaskScheduler (Pure DAG)

### Write Scope
- **CREATE**: `crates/roko-runtime/src/task_scheduler.rs`
- **MODIFY**: `crates/roko-runtime/src/lib.rs` (add `pub mod task_scheduler;` and re-export)

### Dependencies
- None (this is a standalone pure-logic module)

### DO NOT
- Modify any other files
- Add Cargo.toml dependencies
- Put side-effects (I/O, spawning, etc.) in the scheduler — it's pure logic
- Create a new crate

### Task

Create `TaskScheduler` — a pure DAG scheduler that determines which tasks are ready to run
based on dependency resolution. No execution logic — just "given these tasks and their
dependencies, which are ready?"

This is used by the WorkflowEngine (P2D) for multi-task plan execution.

#### File: `crates/roko-runtime/src/task_scheduler.rs`

```rust
//! TaskScheduler — pure DAG dependency resolver.
//!
//! Given a set of tasks with dependencies, determines which tasks are ready
//! to run. No execution logic — just scheduling decisions.
//!
//! Used by WorkflowEngine for multi-task plan execution.

use std::collections::{HashMap, HashSet, VecDeque};

/// A task in the DAG.
#[derive(Debug, Clone)]
pub struct SchedulableTask {
    /// Unique task identifier
    pub id: String,
    /// Task IDs this task depends on (must complete first)
    pub depends_on: Vec<String>,
    /// Files this task will modify (for exclusion checking)
    pub files: Vec<String>,
}

/// Current status of a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    /// Waiting for dependencies
    Blocked,
    /// Dependencies satisfied, ready to run
    Ready,
    /// Currently executing
    Running,
    /// Completed successfully
    Completed,
    /// Failed
    Failed { error: String },
    /// Skipped (dependency failed)
    Skipped,
}

/// Pure DAG scheduler. No side-effects.
#[derive(Debug)]
pub struct TaskScheduler {
    tasks: HashMap<String, SchedulableTask>,
    status: HashMap<String, TaskStatus>,
    /// Maximum number of tasks that can run in parallel
    max_parallel: usize,
}

impl TaskScheduler {
    /// Create a new scheduler with the given tasks and parallelism limit.
    pub fn new(tasks: Vec<SchedulableTask>, max_parallel: usize) -> Self {
        let mut status = HashMap::new();
        let task_map: HashMap<String, SchedulableTask> = tasks
            .into_iter()
            .map(|t| {
                status.insert(t.id.clone(), TaskStatus::Blocked);
                (t.id.clone(), t)
            })
            .collect();

        let mut scheduler = Self {
            tasks: task_map,
            status,
            max_parallel,
        };
        scheduler.update_ready();
        scheduler
    }

    /// Get all tasks that are ready to run right now.
    pub fn ready_tasks(&self) -> Vec<&str> {
        self.status
            .iter()
            .filter(|(_, s)| **s == TaskStatus::Ready)
            .map(|(id, _)| id.as_str())
            .collect()
    }

    /// Get tasks that can be started now, respecting max_parallel and file exclusion.
    pub fn next_batch(&self) -> Vec<&str> {
        let running_count = self.status.values().filter(|s| **s == TaskStatus::Running).count();
        let available_slots = self.max_parallel.saturating_sub(running_count);

        if available_slots == 0 {
            return Vec::new();
        }

        // Collect files currently being modified by running tasks
        let running_files: HashSet<&str> = self
            .status
            .iter()
            .filter(|(_, s)| **s == TaskStatus::Running)
            .flat_map(|(id, _)| {
                self.tasks
                    .get(id)
                    .map(|t| t.files.iter().map(|f| f.as_str()).collect::<Vec<_>>())
                    .unwrap_or_default()
            })
            .collect();

        let mut batch = Vec::new();
        let mut batch_files: HashSet<&str> = HashSet::new();

        for (id, status) in &self.status {
            if *status != TaskStatus::Ready {
                continue;
            }
            if batch.len() >= available_slots {
                break;
            }

            // Check file exclusion
            let task = match self.tasks.get(id) {
                Some(t) => t,
                None => continue,
            };

            let has_conflict = task.files.iter().any(|f| {
                running_files.contains(f.as_str()) || batch_files.contains(f.as_str())
            });

            if !has_conflict {
                for f in &task.files {
                    batch_files.insert(f.as_str());
                }
                batch.push(id.as_str());
            }
        }

        batch
    }

    /// Mark a task as running.
    pub fn mark_running(&mut self, task_id: &str) {
        if let Some(status) = self.status.get_mut(task_id) {
            *status = TaskStatus::Running;
        }
    }

    /// Mark a task as completed. Updates downstream dependencies.
    pub fn mark_completed(&mut self, task_id: &str) {
        if let Some(status) = self.status.get_mut(task_id) {
            *status = TaskStatus::Completed;
        }
        self.update_ready();
    }

    /// Mark a task as failed. Skips downstream dependents.
    pub fn mark_failed(&mut self, task_id: &str, error: String) {
        if let Some(status) = self.status.get_mut(task_id) {
            *status = TaskStatus::Failed { error };
        }
        self.skip_dependents(task_id);
        self.update_ready();
    }

    /// Check if all tasks are in a terminal state.
    pub fn is_done(&self) -> bool {
        self.status.values().all(|s| matches!(s, TaskStatus::Completed | TaskStatus::Failed { .. } | TaskStatus::Skipped))
    }

    /// Get the status of a specific task.
    pub fn task_status(&self, task_id: &str) -> Option<&TaskStatus> {
        self.status.get(task_id)
    }

    /// Get a summary: (completed, failed, skipped, running, blocked, ready)
    pub fn summary(&self) -> (usize, usize, usize, usize, usize, usize) {
        let mut completed = 0;
        let mut failed = 0;
        let mut skipped = 0;
        let mut running = 0;
        let mut blocked = 0;
        let mut ready = 0;

        for status in self.status.values() {
            match status {
                TaskStatus::Completed => completed += 1,
                TaskStatus::Failed { .. } => failed += 1,
                TaskStatus::Skipped => skipped += 1,
                TaskStatus::Running => running += 1,
                TaskStatus::Blocked => blocked += 1,
                TaskStatus::Ready => ready += 1,
            }
        }

        (completed, failed, skipped, running, blocked, ready)
    }

    // ── internal ──

    fn update_ready(&mut self) {
        let blocked_ids: Vec<String> = self
            .status
            .iter()
            .filter(|(_, s)| **s == TaskStatus::Blocked)
            .map(|(id, _)| id.clone())
            .collect();

        for id in blocked_ids {
            if let Some(task) = self.tasks.get(&id) {
                let all_deps_done = task.depends_on.iter().all(|dep| {
                    matches!(self.status.get(dep), Some(TaskStatus::Completed))
                });
                if all_deps_done {
                    self.status.insert(id, TaskStatus::Ready);
                }
            }
        }
    }

    fn skip_dependents(&mut self, failed_id: &str) {
        let mut to_skip: VecDeque<String> = VecDeque::new();

        // Find all tasks that depend (directly or transitively) on the failed task
        for (id, task) in &self.tasks {
            if task.depends_on.iter().any(|d| d == failed_id) {
                to_skip.push_back(id.clone());
            }
        }

        while let Some(skip_id) = to_skip.pop_front() {
            if let Some(status) = self.status.get_mut(&skip_id) {
                if !matches!(status, TaskStatus::Completed | TaskStatus::Failed { .. } | TaskStatus::Skipped) {
                    *status = TaskStatus::Skipped;
                    // Also skip transitive dependents
                    for (id, task) in &self.tasks {
                        if task.depends_on.iter().any(|d| d == &skip_id) {
                            to_skip.push_back(id.clone());
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tasks() -> Vec<SchedulableTask> {
        vec![
            SchedulableTask { id: "T1".into(), depends_on: vec![], files: vec!["a.rs".into()] },
            SchedulableTask { id: "T2".into(), depends_on: vec![], files: vec!["b.rs".into()] },
            SchedulableTask { id: "T3".into(), depends_on: vec!["T1".into(), "T2".into()], files: vec!["c.rs".into()] },
            SchedulableTask { id: "T4".into(), depends_on: vec!["T3".into()], files: vec!["d.rs".into()] },
        ]
    }

    #[test]
    fn initial_ready_tasks() {
        let sched = TaskScheduler::new(make_tasks(), 4);
        let mut ready = sched.ready_tasks();
        ready.sort();
        assert_eq!(ready, vec!["T1", "T2"]);
    }

    #[test]
    fn completing_unblocks_dependents() {
        let mut sched = TaskScheduler::new(make_tasks(), 4);
        sched.mark_running("T1");
        sched.mark_completed("T1");
        sched.mark_running("T2");
        sched.mark_completed("T2");

        let ready = sched.ready_tasks();
        assert_eq!(ready, vec!["T3"]);
    }

    #[test]
    fn failure_skips_dependents() {
        let mut sched = TaskScheduler::new(make_tasks(), 4);
        sched.mark_running("T1");
        sched.mark_failed("T1", "compile error".into());

        assert!(matches!(sched.task_status("T3"), Some(TaskStatus::Skipped)));
        assert!(matches!(sched.task_status("T4"), Some(TaskStatus::Skipped)));
    }

    #[test]
    fn file_exclusion() {
        let tasks = vec![
            SchedulableTask { id: "A".into(), depends_on: vec![], files: vec!["shared.rs".into()] },
            SchedulableTask { id: "B".into(), depends_on: vec![], files: vec!["shared.rs".into()] },
            SchedulableTask { id: "C".into(), depends_on: vec![], files: vec!["other.rs".into()] },
        ];
        let sched = TaskScheduler::new(tasks, 4);
        let batch = sched.next_batch();
        // A and B conflict on shared.rs, so only one of them + C should be in the batch
        assert!(batch.len() <= 3);
        assert!(!(batch.contains(&"A") && batch.contains(&"B")));
    }

    #[test]
    fn respects_max_parallel() {
        let tasks = vec![
            SchedulableTask { id: "A".into(), depends_on: vec![], files: vec![] },
            SchedulableTask { id: "B".into(), depends_on: vec![], files: vec![] },
            SchedulableTask { id: "C".into(), depends_on: vec![], files: vec![] },
        ];
        let sched = TaskScheduler::new(tasks, 2);
        let batch = sched.next_batch();
        assert!(batch.len() <= 2);
    }

    #[test]
    fn is_done_when_all_terminal() {
        let mut sched = TaskScheduler::new(make_tasks(), 4);
        sched.mark_running("T1");
        sched.mark_failed("T1", "err".into());
        sched.mark_running("T2");
        sched.mark_completed("T2");
        // T3, T4 should be skipped due to T1 failure
        assert!(sched.is_done());
    }

    #[test]
    fn summary_counts() {
        let mut sched = TaskScheduler::new(make_tasks(), 4);
        let (c, f, s, r, b, rdy) = sched.summary();
        assert_eq!(c, 0);
        assert_eq!(rdy, 2); // T1, T2
        assert_eq!(b, 2);   // T3, T4
    }
}
```

#### Modification: `crates/roko-runtime/src/lib.rs`

Add:
```rust
pub mod task_scheduler;
pub use task_scheduler::{TaskScheduler, SchedulableTask, TaskStatus};
```

### Done Criteria
```bash
grep -q 'pub struct TaskScheduler' crates/roko-runtime/src/task_scheduler.rs
grep -q 'pub fn next_batch' crates/roko-runtime/src/task_scheduler.rs
grep -q 'pub mod task_scheduler' crates/roko-runtime/src/lib.rs
cargo check -p roko-runtime
cargo test -p roko-runtime --lib -- task_scheduler
```
