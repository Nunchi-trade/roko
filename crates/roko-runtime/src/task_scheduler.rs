//! TaskScheduler -- pure DAG dependency resolver.
//!
//! Given a set of tasks with dependencies, determines which tasks are ready
//! to run. No execution logic -- just scheduling decisions.
//!
//! Used by WorkflowEngine for multi-task plan execution.

use std::collections::{HashMap, HashSet, VecDeque};

/// A task in the DAG.
#[derive(Debug, Clone)]
pub struct SchedulableTask {
    /// Unique task identifier.
    pub id: String,
    /// Task IDs this task depends on (must complete first).
    pub depends_on: Vec<String>,
    /// Files this task will modify (for exclusion checking).
    pub files: Vec<String>,
}

/// Current status of a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    /// Waiting for dependencies.
    Blocked,
    /// Dependencies satisfied, ready to run.
    Ready,
    /// Currently executing.
    Running,
    /// Completed successfully.
    Completed,
    /// Failed.
    Failed {
        /// Failure details reported by the caller.
        error: String,
    },
    /// Skipped because a dependency failed.
    Skipped,
}

/// Pure DAG scheduler. No side-effects.
#[derive(Debug)]
pub struct TaskScheduler {
    tasks: HashMap<String, SchedulableTask>,
    status: HashMap<String, TaskStatus>,
    /// Maximum number of tasks that can run in parallel.
    max_parallel: usize,
}

impl TaskScheduler {
    /// Create a new scheduler with the given tasks and parallelism limit.
    pub fn new(tasks: Vec<SchedulableTask>, max_parallel: usize) -> Self {
        let mut status = HashMap::new();
        let task_map: HashMap<String, SchedulableTask> = tasks
            .into_iter()
            .map(|task| {
                status.insert(task.id.clone(), TaskStatus::Blocked);
                (task.id.clone(), task)
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
            .filter(|(_, status)| **status == TaskStatus::Ready)
            .map(|(id, _)| id.as_str())
            .collect()
    }

    /// Get tasks that can be started now, respecting max_parallel and file exclusion.
    pub fn next_batch(&self) -> Vec<&str> {
        let running_count = self
            .status
            .values()
            .filter(|status| **status == TaskStatus::Running)
            .count();
        let available_slots = self.max_parallel.saturating_sub(running_count);

        if available_slots == 0 {
            return Vec::new();
        }

        let running_files: HashSet<&str> = self
            .status
            .iter()
            .filter(|(_, status)| **status == TaskStatus::Running)
            .flat_map(|(id, _)| {
                self.tasks
                    .get(id)
                    .map(|task| task.files.iter().map(String::as_str).collect::<Vec<_>>())
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

            let Some(task) = self.tasks.get(id) else {
                continue;
            };

            let has_conflict = task.files.iter().any(|file| {
                running_files.contains(file.as_str()) || batch_files.contains(file.as_str())
            });

            if has_conflict {
                continue;
            }

            for file in &task.files {
                batch_files.insert(file.as_str());
            }
            batch.push(id.as_str());
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
        self.status.values().all(|status| {
            matches!(
                status,
                TaskStatus::Completed | TaskStatus::Failed { .. } | TaskStatus::Skipped
            )
        })
    }

    /// Get the status of a specific task.
    pub fn task_status(&self, task_id: &str) -> Option<&TaskStatus> {
        self.status.get(task_id)
    }

    /// Get a summary: (completed, failed, skipped, running, blocked, ready).
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

    fn update_ready(&mut self) {
        let blocked_ids: Vec<String> = self
            .status
            .iter()
            .filter(|(_, status)| **status == TaskStatus::Blocked)
            .map(|(id, _)| id.clone())
            .collect();

        for id in blocked_ids {
            if let Some(task) = self.tasks.get(&id) {
                let all_deps_done = task
                    .depends_on
                    .iter()
                    .all(|dep| matches!(self.status.get(dep), Some(TaskStatus::Completed)));
                if all_deps_done {
                    self.status.insert(id, TaskStatus::Ready);
                }
            }
        }
    }

    fn skip_dependents(&mut self, failed_id: &str) {
        let mut to_skip = VecDeque::new();

        for (id, task) in &self.tasks {
            if task.depends_on.iter().any(|dep| dep == failed_id) {
                to_skip.push_back(id.clone());
            }
        }

        while let Some(skip_id) = to_skip.pop_front() {
            if let Some(status) = self.status.get_mut(&skip_id) {
                if matches!(
                    status,
                    TaskStatus::Completed | TaskStatus::Failed { .. } | TaskStatus::Skipped
                ) {
                    continue;
                }

                *status = TaskStatus::Skipped;

                for (id, task) in &self.tasks {
                    if task.depends_on.iter().any(|dep| dep == &skip_id) {
                        to_skip.push_back(id.clone());
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
            SchedulableTask {
                id: "T1".into(),
                depends_on: vec![],
                files: vec!["a.rs".into()],
            },
            SchedulableTask {
                id: "T2".into(),
                depends_on: vec![],
                files: vec!["b.rs".into()],
            },
            SchedulableTask {
                id: "T3".into(),
                depends_on: vec!["T1".into(), "T2".into()],
                files: vec!["c.rs".into()],
            },
            SchedulableTask {
                id: "T4".into(),
                depends_on: vec!["T3".into()],
                files: vec!["d.rs".into()],
            },
        ]
    }

    #[test]
    fn initial_ready_tasks() {
        let sched = TaskScheduler::new(make_tasks(), 4);
        let mut ready = sched.ready_tasks();
        ready.sort_unstable();
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
            SchedulableTask {
                id: "A".into(),
                depends_on: vec![],
                files: vec!["shared.rs".into()],
            },
            SchedulableTask {
                id: "B".into(),
                depends_on: vec![],
                files: vec!["shared.rs".into()],
            },
            SchedulableTask {
                id: "C".into(),
                depends_on: vec![],
                files: vec!["other.rs".into()],
            },
        ];
        let sched = TaskScheduler::new(tasks, 4);
        let batch = sched.next_batch();
        assert!(batch.len() <= 3);
        assert!(!(batch.contains(&"A") && batch.contains(&"B")));
    }

    #[test]
    fn respects_max_parallel() {
        let tasks = vec![
            SchedulableTask {
                id: "A".into(),
                depends_on: vec![],
                files: vec![],
            },
            SchedulableTask {
                id: "B".into(),
                depends_on: vec![],
                files: vec![],
            },
            SchedulableTask {
                id: "C".into(),
                depends_on: vec![],
                files: vec![],
            },
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

        assert!(sched.is_done());
    }

    #[test]
    fn summary_counts() {
        let sched = TaskScheduler::new(make_tasks(), 4);
        let (completed, _failed, _skipped, _running, blocked, ready) = sched.summary();

        assert_eq!(completed, 0);
        assert_eq!(ready, 2);
        assert_eq!(blocked, 2);
    }
}
