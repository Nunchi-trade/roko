//! Per-plan task DAG state.
//!
//! Owns per-plan execution state that the event loop previously kept inline:
//!
//! - **Running task ids per plan** so parallel execution cannot double-dispatch
//!   the same task within a single plan and so multi-plan execution can scale
//!   beyond a single global agent handle.
//! - **Skipped/blocked downstream tasks** when a prerequisite has been
//!   exhausted or has otherwise terminally failed.
//! - **Ready task resolution** that walks `TaskDef::depends_on` /
//!   `depends_on_plan` instead of relying on sentinel task names like
//!   `"next"`, `"fix"`, or `"regen-verify"`.
//! - **Plan-level deadlines and retry backoff** that are visible to the
//!   active runtime so a plan that has exceeded its wall-clock budget can
//!   stop dispatching new tasks instead of looping through retries
//!   indefinitely.
//!
//! This module is intentionally focused on DAG bookkeeping. It does not
//! execute tasks, run gates, or perform any I/O. Those concerns continue
//! to live in `event_loop.rs`.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use roko_core::defaults::{
    DEFAULT_PLAN_RETRY_BACKOFF_SHIFT_CAP, DEFAULT_PLAN_RETRY_BASE_SECS,
    DEFAULT_PLAN_RETRY_MAX_SECS, DEFAULT_PLAN_TIMEOUT_SECS,
};

use crate::task_parser::TaskDef;

// ─── Public ID aliases ─────────────────────────────────────────────────

/// Plan identifier (matches the directory name that owns `tasks.toml`).
pub type PlanId = String;

/// Task identifier (matches `TaskDef::id`).
pub type TaskId = String;

/// Whether a task's declared plan-file status is terminal.
#[must_use]
pub(crate) fn task_status_is_terminal(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "done" | "complete" | "completed" | "skipped"
    )
}

// ─── Skipped reason ─────────────────────────────────────────────────────

/// Why a downstream task was marked as skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkippedReason {
    /// One of the task's prerequisites terminally failed.
    PrerequisiteFailed { prerequisite: TaskId },
    /// The plan ran past its wall-clock deadline.
    PlanTimedOut,
}

// ─── Per-plan state ─────────────────────────────────────────────────────

/// Bookkeeping for a single plan's task DAG.
#[derive(Debug, Clone, Default)]
pub struct PlanDag {
    /// Tasks the runner has dispatched but not yet finalised. Used to prevent
    /// double-dispatch when the same plan tick produces overlapping
    /// `SpawnAgent` actions.
    pub running: HashSet<TaskId>,
    /// Tasks that completed successfully.
    pub completed: HashSet<TaskId>,
    /// Tasks that terminally failed (retries exhausted).
    pub failed: HashSet<TaskId>,
    /// Downstream tasks that were skipped because a prerequisite failed
    /// or the plan timed out.
    pub skipped: HashMap<TaskId, SkippedReason>,
    /// Wall-clock deadline for the plan (set on first dispatch). When the
    /// plan exceeds this, the runner stops dispatching further tasks for it.
    pub deadline: Option<Instant>,
    /// Earliest time at which the plan may be retried after a transient
    /// failure (exponential backoff visible to the active runtime).
    pub retry_not_before: Option<Instant>,
}

impl PlanDag {
    /// Whether this task is currently running (already dispatched but not
    /// yet finalised).
    #[must_use]
    pub fn is_running(&self, task_id: &str) -> bool {
        self.running.contains(task_id)
    }

    /// Whether this task is in any terminal state (completed, failed,
    /// or skipped).
    #[must_use]
    pub fn is_terminal(&self, task_id: &str) -> bool {
        self.completed.contains(task_id)
            || self.failed.contains(task_id)
            || self.skipped.contains_key(task_id)
    }
}

// ─── Plan-level config ──────────────────────────────────────────────────

/// Configuration for the DAG controller. Backoff defaults match
/// `02-PLAN-EXECUTION.md`.
#[derive(Debug, Clone, Copy)]
pub struct DagConfig {
    /// Wall-clock timeout for a plan (default: 1 hour).
    pub plan_timeout: Duration,
    /// Base retry delay (default: 1s).
    pub retry_base: Duration,
    /// Maximum retry delay cap (default: 30s).
    pub retry_max: Duration,
}

impl Default for DagConfig {
    fn default() -> Self {
        Self {
            plan_timeout: Duration::from_secs(DEFAULT_PLAN_TIMEOUT_SECS),
            retry_base: Duration::from_secs(DEFAULT_PLAN_RETRY_BASE_SECS),
            retry_max: Duration::from_secs(DEFAULT_PLAN_RETRY_MAX_SECS),
        }
    }
}

impl DagConfig {
    /// Compute exponential backoff delay (1s, 2s, 4s, ... capped at retry_max).
    #[must_use]
    pub fn backoff_for_attempt(&self, attempt: u32) -> Duration {
        // Attempt 0 is the first retry → 1s. Attempt 1 → 2s. Attempt 2 → 4s.
        let shift = attempt.min(DEFAULT_PLAN_RETRY_BACKOFF_SHIFT_CAP);
        let factor = 1u64.checked_shl(shift).unwrap_or(u64::MAX);
        let raw = self
            .retry_base
            .saturating_mul(factor.min(u32::MAX as u64) as u32);
        std::cmp::min(raw, self.retry_max)
    }
}

// ─── TaskDag ────────────────────────────────────────────────────────────

/// Owner of every plan's DAG bookkeeping for the run.
///
/// The event loop only needs to know about `TaskDag`. Per-plan state lives
/// inside `PlanDag` instances keyed by plan id.
#[derive(Debug, Clone, Default)]
pub struct TaskDag {
    plans: HashMap<PlanId, PlanDag>,
    config: DagConfig,
}

impl TaskDag {
    /// Construct an empty `TaskDag` with the supplied configuration.
    #[must_use]
    pub fn new(config: DagConfig) -> Self {
        Self {
            plans: HashMap::new(),
            config,
        }
    }

    /// Read-only access to a plan's DAG state.
    #[must_use]
    pub fn plan(&self, plan_id: &str) -> Option<&PlanDag> {
        self.plans.get(plan_id)
    }

    /// Mutable access to a plan's DAG state, creating an entry on demand.
    pub fn plan_mut(&mut self, plan_id: &str) -> &mut PlanDag {
        self.plans.entry(plan_id.to_string()).or_default()
    }

    /// Resolve the next ready task within a plan.
    ///
    /// A task is ready when:
    /// - it is not already running, completed, failed, or skipped,
    /// - its `depends_on` are all in `completed_in_plan`,
    /// - its `depends_on_plan` are all in `completed_plans`.
    ///
    /// Tasks are ordered by `TaskDef::id` to keep dispatch deterministic.
    #[must_use]
    pub fn next_ready_task<'a>(
        &self,
        plan_id: &str,
        tasks: &'a [&'a TaskDef],
        completed_in_plan: &[String],
        completed_plans: &[String],
    ) -> Option<&'a TaskDef> {
        let plan = self.plans.get(plan_id);
        let mut ordered: Vec<&'a TaskDef> = tasks.iter().copied().collect();
        ordered.sort_by(|a, b| a.id.cmp(&b.id));

        ordered.into_iter().find(|task| {
            if let Some(state) = plan {
                if state.is_running(&task.id) || state.is_terminal(&task.id) {
                    return false;
                }
            }
            if completed_in_plan.contains(&task.id) {
                return false;
            }
            if task_status_is_terminal(&task.status) {
                return false;
            }
            task.is_ready_with_plan_deps(completed_in_plan, completed_plans)
        })
    }

    /// All tasks in `tasks` that are ready right now, in deterministic order.
    #[must_use]
    pub fn ready_tasks<'a>(
        &self,
        plan_id: &str,
        tasks: &'a [&'a TaskDef],
        completed_in_plan: &[String],
        completed_plans: &[String],
    ) -> Vec<&'a TaskDef> {
        let plan = self.plans.get(plan_id);
        let mut ordered: Vec<&'a TaskDef> = tasks.iter().copied().collect();
        ordered.sort_by(|a, b| a.id.cmp(&b.id));

        ordered
            .into_iter()
            .filter(|task| {
                if let Some(state) = plan {
                    if state.is_running(&task.id) || state.is_terminal(&task.id) {
                        return false;
                    }
                }
                if completed_in_plan.contains(&task.id) {
                    return false;
                }
                if task_status_is_terminal(&task.status) {
                    return false;
                }
                task.is_ready_with_plan_deps(completed_in_plan, completed_plans)
            })
            .collect()
    }

    /// Mark a task as running. Returns `false` if the task was already
    /// marked running for this plan, in which case the dispatcher must
    /// skip the duplicate spawn.
    pub fn mark_running(&mut self, plan_id: &str, task_id: &str) -> bool {
        let plan_timeout = self.config.plan_timeout;
        let plan = self.plan_mut(plan_id);
        if plan.deadline.is_none() {
            plan.deadline = Some(Instant::now() + plan_timeout);
        }
        plan.running.insert(task_id.to_string())
    }

    /// Stop tracking a task as running. Used when a task transitions to
    /// any terminal state.
    pub fn clear_running(&mut self, plan_id: &str, task_id: &str) {
        if let Some(plan) = self.plans.get_mut(plan_id) {
            plan.running.remove(task_id);
        }
    }

    /// Record successful completion of a task.
    pub fn mark_complete(&mut self, plan_id: &str, task_id: &str) {
        let plan = self.plan_mut(plan_id);
        plan.running.remove(task_id);
        plan.completed.insert(task_id.to_string());
        plan.retry_not_before = None;
    }

    /// Record a terminal failure for a task and propagate skipped state to
    /// every downstream task (transitively) within this plan.
    ///
    /// Returns the list of task ids that were newly marked as skipped.
    pub fn mark_failed_blocking_downstream(
        &mut self,
        plan_id: &str,
        failed_task: &str,
        all_tasks: &[&TaskDef],
    ) -> Vec<TaskId> {
        let plan = self.plan_mut(plan_id);
        plan.running.remove(failed_task);
        plan.failed.insert(failed_task.to_string());

        // Build dependency graph: dep → set of tasks that depend on it.
        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
        for task in all_tasks {
            for dep in &task.depends_on {
                dependents.entry(dep.as_str()).or_default().push(&task.id);
            }
        }

        let mut newly_skipped: Vec<String> = Vec::new();
        let mut frontier: Vec<String> = vec![failed_task.to_string()];
        while let Some(current) = frontier.pop() {
            let Some(downstream) = dependents.get(current.as_str()) else {
                continue;
            };
            for child in downstream {
                let child_id = (*child).to_string();
                if plan.is_terminal(&child_id) {
                    continue;
                }
                plan.skipped.insert(
                    child_id.clone(),
                    SkippedReason::PrerequisiteFailed {
                        prerequisite: current.clone(),
                    },
                );
                plan.running.remove(&child_id);
                newly_skipped.push(child_id.clone());
                frontier.push(child_id);
            }
        }
        newly_skipped
    }

    /// Whether the plan has exceeded its wall-clock deadline.
    #[must_use]
    pub fn is_plan_timed_out(&self, plan_id: &str) -> bool {
        self.plans
            .get(plan_id)
            .and_then(|plan| plan.deadline)
            .is_some_and(|deadline| Instant::now() > deadline)
    }

    /// Mark every non-terminal task in this plan as skipped because the
    /// plan timed out. Returns the ids that were newly skipped.
    pub fn mark_plan_timed_out(&mut self, plan_id: &str, all_tasks: &[&TaskDef]) -> Vec<TaskId> {
        let plan = self.plan_mut(plan_id);
        let mut newly_skipped = Vec::new();
        for task in all_tasks {
            if plan.is_terminal(&task.id) {
                continue;
            }
            plan.skipped
                .insert(task.id.clone(), SkippedReason::PlanTimedOut);
            plan.running.remove(&task.id);
            newly_skipped.push(task.id.clone());
        }
        newly_skipped
    }

    /// Record an exponential-backoff retry deadline for the plan.
    pub fn schedule_retry(&mut self, plan_id: &str, attempt: u32) {
        let delay = self.config.backoff_for_attempt(attempt);
        let plan = self.plan_mut(plan_id);
        plan.retry_not_before = Some(Instant::now() + delay);
    }

    /// Whether the plan is still cooling down before its next retry.
    #[must_use]
    pub fn retry_remaining(&self, plan_id: &str) -> Option<Duration> {
        let plan = self.plans.get(plan_id)?;
        let deadline = plan.retry_not_before?;
        deadline.checked_duration_since(Instant::now())
    }

    /// Convenience: compute the configured backoff for an attempt.
    #[must_use]
    pub fn backoff_for_attempt(&self, attempt: u32) -> Duration {
        self.config.backoff_for_attempt(attempt)
    }

    /// Configured plan timeout.
    #[must_use]
    pub fn plan_timeout(&self) -> Duration {
        self.config.plan_timeout
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_parser::TaskDef;

    fn task(id: &str, deps: &[&str]) -> TaskDef {
        TaskDef {
            id: id.to_string(),
            title: id.to_string(),
            description: None,
            role: None,
            status: "ready".to_string(),
            tier: "focused".to_string(),
            frequency: None,
            model_hint: None,
            replan_strategy: None,
            max_loc: None,
            files: vec![],
            allowed_tools: None,
            denied_tools: None,
            mcp_servers: None,
            depends_on: deps.iter().map(|s| (*s).to_string()).collect(),
            depends_on_plan: vec![],
            split_into: None,
            context: None,
            verify: vec![],
            timeout_secs: 60,
            max_retries: 1,
            acceptance: vec![],
            acceptance_contract: None,
            domain: None,
            sequence: 0,
        }
    }

    #[test]
    fn ready_resolution_walks_dependency_branches() {
        let dag = TaskDag::default();
        let a = task("A", &[]);
        let b = task("B", &["A"]);
        let c = task("C", &["A"]);
        let d = task("D", &["B", "C"]);
        let tasks: Vec<&TaskDef> = vec![&a, &b, &c, &d];

        // Initially, only A is ready.
        let ready = dag.ready_tasks("p1", &tasks, &[], &[]);
        assert_eq!(
            ready.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["A"]
        );

        // After A completes, B and C become ready.
        let ready = dag.ready_tasks("p1", &tasks, &["A".into()], &[]);
        let ids: Vec<&str> = ready.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["B", "C"]);

        // After A and B, only C is ready (D still blocked on C).
        let ready = dag.ready_tasks("p1", &tasks, &["A".into(), "B".into()], &[]);
        assert_eq!(
            ready.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["C"]
        );

        // After A, B, C all done, D is ready.
        let ready = dag.ready_tasks("p1", &tasks, &["A".into(), "B".into(), "C".into()], &[]);
        assert_eq!(
            ready.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["D"]
        );
    }

    #[test]
    fn double_dispatch_guard_blocks_repeat_mark_running() {
        let mut dag = TaskDag::default();
        assert!(dag.mark_running("p1", "A"));
        // Second mark_running for the same task on the same plan returns false.
        assert!(!dag.mark_running("p1", "A"));
        // Different task on same plan succeeds.
        assert!(dag.mark_running("p1", "B"));
        // Same task id on a different plan succeeds.
        assert!(dag.mark_running("p2", "A"));
    }

    #[test]
    fn next_ready_task_skips_running_and_terminal() {
        let mut dag = TaskDag::default();
        let a = task("A", &[]);
        let b = task("B", &[]);
        let tasks: Vec<&TaskDef> = vec![&a, &b];

        assert!(dag.mark_running("p1", "A"));
        let next = dag.next_ready_task("p1", &tasks, &[], &[]);
        assert_eq!(next.map(|t| t.id.as_str()), Some("B"));

        // Mark B as running — nothing else is ready in this plan.
        assert!(dag.mark_running("p1", "B"));
        let next = dag.next_ready_task("p1", &tasks, &[], &[]);
        assert!(next.is_none());
    }

    #[test]
    fn ready_resolution_skips_terminal_task_statuses() {
        let dag = TaskDag::default();
        let mut done = task("A", &[]);
        done.status = "done".to_string();
        let mut complete = task("B", &[]);
        complete.status = "complete".to_string();
        let ready = task("C", &[]);
        let tasks: Vec<&TaskDef> = vec![&done, &complete, &ready];

        let ready_tasks = dag.ready_tasks("p1", &tasks, &[], &[]);

        assert_eq!(
            ready_tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec!["C"]
        );
    }

    #[test]
    fn skipped_propagation_on_prerequisite_failure() {
        let mut dag = TaskDag::default();
        let a = task("A", &[]);
        let b = task("B", &["A"]);
        let c = task("C", &["B"]);
        let d = task("D", &[]);
        let tasks: Vec<&TaskDef> = vec![&a, &b, &c, &d];

        let skipped = dag.mark_failed_blocking_downstream("p1", "A", &tasks);
        // B (depends on A) and C (transitively via B) are skipped. D is unaffected.
        let mut sorted = skipped.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["B".to_string(), "C".to_string()]);

        let plan = dag.plan("p1").expect("plan recorded");
        assert!(plan.failed.contains("A"));
        assert!(plan.skipped.contains_key("B"));
        assert!(plan.skipped.contains_key("C"));
        assert!(!plan.skipped.contains_key("D"));

        // Re-running mark_failed should not double-skip already-terminal tasks.
        let again = dag.mark_failed_blocking_downstream("p1", "A", &tasks);
        assert!(again.is_empty());
    }

    #[test]
    fn skipped_reason_records_failed_prerequisite() {
        let mut dag = TaskDag::default();
        let a = task("A", &[]);
        let b = task("B", &["A"]);
        let tasks: Vec<&TaskDef> = vec![&a, &b];

        dag.mark_failed_blocking_downstream("p1", "A", &tasks);
        let plan = dag.plan("p1").unwrap();
        match plan.skipped.get("B") {
            Some(SkippedReason::PrerequisiteFailed { prerequisite }) => {
                assert_eq!(prerequisite, "A");
            }
            other => panic!("expected PrerequisiteFailed, got {other:?}"),
        }
    }

    #[test]
    fn plan_timeout_propagates_skipped_state() {
        let cfg = DagConfig {
            plan_timeout: Duration::from_millis(0),
            ..DagConfig::default()
        };
        let mut dag = TaskDag::new(cfg);
        let a = task("A", &[]);
        let b = task("B", &["A"]);
        let tasks: Vec<&TaskDef> = vec![&a, &b];

        // Marking running primes the deadline. With zero timeout it is
        // immediately exceeded.
        dag.mark_running("p1", "A");
        std::thread::sleep(Duration::from_millis(2));
        assert!(dag.is_plan_timed_out("p1"));

        let skipped = dag.mark_plan_timed_out("p1", &tasks);
        let mut sorted = skipped.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["A".to_string(), "B".to_string()]);
        assert!(matches!(
            dag.plan("p1").unwrap().skipped.get("A"),
            Some(SkippedReason::PlanTimedOut)
        ));
    }

    #[test]
    fn backoff_grows_exponentially_then_caps() {
        let cfg = DagConfig {
            retry_base: Duration::from_secs(DEFAULT_PLAN_RETRY_BASE_SECS),
            retry_max: Duration::from_secs(DEFAULT_PLAN_RETRY_MAX_SECS),
            ..DagConfig::default()
        };
        let dag = TaskDag::new(cfg);
        assert_eq!(
            dag.backoff_for_attempt(0),
            Duration::from_secs(DEFAULT_PLAN_RETRY_BASE_SECS)
        );
        assert_eq!(
            dag.backoff_for_attempt(1),
            Duration::from_secs(DEFAULT_PLAN_RETRY_BASE_SECS * 2)
        );
        assert_eq!(
            dag.backoff_for_attempt(2),
            Duration::from_secs(DEFAULT_PLAN_RETRY_BASE_SECS * 4)
        );
        assert_eq!(
            dag.backoff_for_attempt(3),
            Duration::from_secs(DEFAULT_PLAN_RETRY_BASE_SECS * 8)
        );
        assert_eq!(
            dag.backoff_for_attempt(5),
            Duration::from_secs(DEFAULT_PLAN_RETRY_MAX_SECS)
        );
        assert_eq!(
            dag.backoff_for_attempt(99),
            Duration::from_secs(DEFAULT_PLAN_RETRY_MAX_SECS)
        );
    }

    #[test]
    fn schedule_retry_records_visible_cooldown() {
        let cfg = DagConfig {
            retry_base: Duration::from_secs(DEFAULT_PLAN_RETRY_BASE_SECS * 2),
            retry_max: Duration::from_secs(DEFAULT_PLAN_RETRY_MAX_SECS),
            ..DagConfig::default()
        };
        let mut dag = TaskDag::new(cfg);
        dag.schedule_retry("p1", 0);
        let remaining = dag.retry_remaining("p1").expect("retry deadline set");
        // Should be approximately one doubled base-delay window.
        assert!(remaining > Duration::from_millis(500));
        assert!(remaining <= Duration::from_secs(DEFAULT_PLAN_RETRY_BASE_SECS * 2));

        // After completing, retry_not_before should be cleared.
        dag.mark_complete("p1", "A");
        assert!(dag.retry_remaining("p1").is_none());
    }
}
