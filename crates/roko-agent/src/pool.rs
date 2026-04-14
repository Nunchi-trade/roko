//! `AgentPool` — sequential task execution for a single agent role.
//!
//! Manages a queue of tasks that execute one at a time. If the primary agent
//! fails, the pool retries with a fallback agent (different model) before
//! marking the task as failed.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use roko_core::AgentRole;

use crate::agent::{Agent, AgentResult};

// ─── AgentInstanceId ─────────────────────────────────────────────────────

/// Unique identifier for an agent instance, encoding role + instance name.
///
/// The instance name typically encodes plan and task information,
/// e.g. `"plan42-task3"`, producing a display form like `"implementer-plan42-task3"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentInstanceId {
    /// The role this instance fulfils.
    pub role: AgentRole,
    /// Human-readable instance discriminator (e.g. `"plan42-task3"`).
    pub instance: String,
}

impl AgentInstanceId {
    /// Construct a new instance ID.
    #[must_use]
    pub fn new(role: AgentRole, instance: impl Into<String>) -> Self {
        Self {
            role,
            instance: instance.into(),
        }
    }

    /// Default instance for a role (singleton use).
    #[must_use]
    pub fn default_for(role: AgentRole) -> Self {
        Self {
            role,
            instance: "default".into(),
        }
    }

    /// The full string key: `"{role}-{instance}"`.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}-{}", self.role.label(), self.instance)
    }

    /// Whether this instance's key contains `needle` (for plan-based matching).
    #[must_use]
    pub fn matches(&self, needle: &str) -> bool {
        self.key().contains(needle)
    }
}

impl fmt::Display for AgentInstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.instance == "default" {
            write!(f, "{}", self.role.label())
        } else {
            write!(f, "{}-{}", self.role.label(), self.instance)
        }
    }
}

// ─── InstanceStatus ──────────────────────────────────────────────────────

/// Lifecycle state of an agent instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceStatus {
    /// Pre-spawned, waiting for work.
    Warm,
    /// Queued, waiting its turn.
    Pending,
    /// Currently executing.
    Active,
    /// Completed successfully.
    Done,
    /// Completed with failure.
    Failed,
    /// Cancelled before completion.
    Cancelled,
}

impl fmt::Display for InstanceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Warm => f.write_str("warm"),
            Self::Pending => f.write_str("pending"),
            Self::Active => f.write_str("active"),
            Self::Done => f.write_str("done"),
            Self::Failed => f.write_str("failed"),
            Self::Cancelled => f.write_str("cancelled"),
        }
    }
}

// ─── AgentTask ───────────────────────────────────────────────────────────

/// A unit of work submitted to the pool.
pub struct AgentTask {
    /// Unique identifier for this task's agent instance.
    pub id: AgentInstanceId,
    /// The input signal to run the agent against.
    pub input: roko_core::Engram,
    /// The context for the agent run.
    pub ctx: roko_core::Context,
}

impl AgentTask {
    /// Create a new task.
    #[must_use]
    pub const fn new(
        id: AgentInstanceId,
        input: roko_core::Engram,
        ctx: roko_core::Context,
    ) -> Self {
        Self { id, input, ctx }
    }
}

// ─── TaskOutcome ─────────────────────────────────────────────────────────

/// The result of a completed (or cancelled) task.
#[derive(Debug, Clone)]
pub struct TaskOutcome {
    /// Which instance produced this result.
    pub id: AgentInstanceId,
    /// The agent result (if completed).
    pub result: Option<AgentResult>,
    /// Final status.
    pub status: InstanceStatus,
    /// Whether the fallback model was used.
    pub used_fallback: bool,
}

// ─── AgentPool ───────────────────────────────────────────────────────────

/// Sequential agent pool: executes tasks one at a time for a single role.
///
/// If the primary agent fails a task, the pool automatically retries with
/// the fallback agent (if configured) before reporting failure.
pub struct AgentPool {
    /// The role this pool serves.
    role: AgentRole,
    /// Primary agent implementation.
    primary: Arc<dyn Agent>,
    /// Optional fallback agent (different model) for retry on failure.
    fallback: Option<Arc<dyn Agent>>,
    /// Pending tasks waiting to execute.
    pending: VecDeque<AgentTask>,
    /// Status of each known instance.
    statuses: Vec<(AgentInstanceId, InstanceStatus)>,
    /// Completed task outcomes ready to be polled.
    completed: VecDeque<TaskOutcome>,
    /// The currently active task (if any).
    active_task: Option<AgentInstanceId>,
}

impl AgentPool {
    /// Create a new pool for the given role with a primary agent.
    #[must_use]
    pub fn new(role: AgentRole, primary: Arc<dyn Agent>) -> Self {
        Self {
            role,
            primary,
            fallback: None,
            pending: VecDeque::new(),
            statuses: Vec::new(),
            completed: VecDeque::new(),
            active_task: None,
        }
    }

    /// Set the fallback agent for retry on primary failure.
    #[must_use]
    pub fn with_fallback(mut self, fallback: Arc<dyn Agent>) -> Self {
        self.fallback = Some(fallback);
        self
    }

    /// The role this pool serves.
    #[must_use]
    pub const fn role(&self) -> AgentRole {
        self.role
    }

    /// Submit a task for sequential execution.
    pub fn submit(&mut self, task: AgentTask) {
        self.set_status(task.id.clone(), InstanceStatus::Pending);
        self.pending.push_back(task);
    }

    /// Submit multiple tasks for sequential execution.
    ///
    /// Tasks are enqueued in iterator order.
    pub fn submit_all<I>(&mut self, tasks: I)
    where
        I: IntoIterator<Item = AgentTask>,
    {
        for task in tasks {
            self.submit(task);
        }
    }

    /// Poll for the next completed outcome (if any are ready).
    pub fn poll(&mut self) -> Option<TaskOutcome> {
        self.completed.pop_front()
    }

    /// Drain all completed outcomes currently buffered by the pool.
    #[must_use]
    pub fn drain_completed(&mut self) -> Vec<TaskOutcome> {
        self.completed.drain(..).collect()
    }

    /// Cancel a pending or active task by instance ID.
    ///
    /// Returns `true` if the task was found and cancelled.
    pub fn cancel(&mut self, id: &AgentInstanceId) -> bool {
        // Check pending queue first.
        if let Some(pos) = self.pending.iter().position(|t| &t.id == id) {
            self.pending.remove(pos);
            self.set_status(id.clone(), InstanceStatus::Cancelled);
            self.completed.push_back(TaskOutcome {
                id: id.clone(),
                result: None,
                status: InstanceStatus::Cancelled,
                used_fallback: false,
            });
            return true;
        }

        // Check active task.
        if self.active_task.as_ref() == Some(id) {
            self.active_task = None;
            self.set_status(id.clone(), InstanceStatus::Cancelled);
            self.completed.push_back(TaskOutcome {
                id: id.clone(),
                result: None,
                status: InstanceStatus::Cancelled,
                used_fallback: false,
            });
            return true;
        }

        false
    }

    /// Number of currently active tasks (0 or 1 for sequential pool).
    #[must_use]
    pub fn active_count(&self) -> usize {
        usize::from(self.active_task.is_some())
    }

    /// Number of pending tasks in the queue.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Current status of an instance, if tracked.
    #[must_use]
    pub fn status(&self, id: &AgentInstanceId) -> Option<InstanceStatus> {
        self.statuses
            .iter()
            .find(|(sid, _)| sid == id)
            .map(|(_, s)| *s)
    }

    /// Execute the next pending task. Returns the outcome.
    ///
    /// If no tasks are pending, returns `None`.
    pub async fn execute_next(&mut self) -> Option<TaskOutcome> {
        let task = self.pending.pop_front()?;
        let id = task.id.clone();
        self.active_task = Some(id.clone());
        self.set_status(id.clone(), InstanceStatus::Active);

        // Try primary agent.
        let result = self.primary.run(&task.input, &task.ctx).await;

        if result.success {
            self.active_task = None;
            self.set_status(id.clone(), InstanceStatus::Done);
            let outcome = TaskOutcome {
                id,
                result: Some(result),
                status: InstanceStatus::Done,
                used_fallback: false,
            };
            self.completed.push_back(outcome.clone());
            return Some(outcome);
        }

        // Primary failed — try fallback if available.
        if let Some(fallback) = &self.fallback {
            let fb_result = fallback.run(&task.input, &task.ctx).await;
            self.active_task = None;

            if fb_result.success {
                self.set_status(id.clone(), InstanceStatus::Done);
                let outcome = TaskOutcome {
                    id,
                    result: Some(fb_result),
                    status: InstanceStatus::Done,
                    used_fallback: true,
                };
                self.completed.push_back(outcome.clone());
                return Some(outcome);
            }

            // Both primary and fallback failed.
            self.set_status(id.clone(), InstanceStatus::Failed);
            let outcome = TaskOutcome {
                id,
                result: Some(fb_result),
                status: InstanceStatus::Failed,
                used_fallback: true,
            };
            self.completed.push_back(outcome.clone());
            Some(outcome)
        } else {
            // No fallback — primary failure is final.
            self.active_task = None;
            self.set_status(id.clone(), InstanceStatus::Failed);
            let outcome = TaskOutcome {
                id,
                result: Some(result),
                status: InstanceStatus::Failed,
                used_fallback: false,
            };
            self.completed.push_back(outcome.clone());
            Some(outcome)
        }
    }

    /// Execute all pending tasks sequentially. Returns all outcomes.
    pub async fn execute_all(&mut self) -> Vec<TaskOutcome> {
        let mut outcomes = Vec::new();
        while let Some(outcome) = self.execute_next().await {
            outcomes.push(outcome);
        }
        outcomes
    }

    /// Update or insert the status for an instance.
    fn set_status(&mut self, id: AgentInstanceId, status: InstanceStatus) {
        if let Some(entry) = self.statuses.iter_mut().find(|(sid, _)| *sid == id) {
            entry.1 = status;
        } else {
            self.statuses.push((id, status));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockAgent;
    use roko_core::{Body, Context, Engram, Kind};
    use std::sync::Arc;

    fn prompt(text: &str) -> Engram {
        Engram::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    fn ctx() -> Context {
        Context::at(0)
    }

    // ── AgentInstanceId tests ────────────────────────────────────────────

    #[test]
    fn pool_instance_id_display_default() {
        let id = AgentInstanceId::default_for(AgentRole::Implementer);
        assert_eq!(id.to_string(), "implementer");
    }

    #[test]
    fn pool_instance_id_display_named() {
        let id = AgentInstanceId::new(AgentRole::Implementer, "plan42-task3");
        assert_eq!(id.to_string(), "implementer-plan42-task3");
    }

    #[test]
    fn pool_instance_id_key() {
        let id = AgentInstanceId::new(AgentRole::Auditor, "plan7");
        assert_eq!(id.key(), "auditor-plan7");
    }

    #[test]
    fn pool_instance_id_matches_substring() {
        let id = AgentInstanceId::new(AgentRole::Implementer, "plan42-task3");
        assert!(id.matches("plan42"));
        assert!(id.matches("task3"));
        assert!(!id.matches("plan99"));
    }

    // ── InstanceStatus tests ─────────────────────────────────────────────

    #[test]
    fn pool_instance_status_display() {
        assert_eq!(InstanceStatus::Warm.to_string(), "warm");
        assert_eq!(InstanceStatus::Pending.to_string(), "pending");
        assert_eq!(InstanceStatus::Active.to_string(), "active");
        assert_eq!(InstanceStatus::Done.to_string(), "done");
        assert_eq!(InstanceStatus::Failed.to_string(), "failed");
        assert_eq!(InstanceStatus::Cancelled.to_string(), "cancelled");
    }

    // ── AgentPool basic tests ────────────────────────────────────────────

    #[tokio::test]
    async fn pool_submit_and_execute_success() {
        let agent = Arc::new(MockAgent::reply("ok"));
        let mut pool = AgentPool::new(AgentRole::Implementer, agent);
        let id = AgentInstanceId::new(AgentRole::Implementer, "t1");
        pool.submit(AgentTask::new(id.clone(), prompt("do stuff"), ctx()));

        assert_eq!(pool.pending_count(), 1);
        assert_eq!(pool.active_count(), 0);

        let outcome = pool.execute_next().await.unwrap();
        assert_eq!(outcome.status, InstanceStatus::Done);
        assert!(!outcome.used_fallback);
        assert!(outcome.result.unwrap().success);

        assert_eq!(pool.pending_count(), 0);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.status(&id), Some(InstanceStatus::Done));
    }

    #[tokio::test]
    async fn pool_fallback_on_primary_failure() {
        let primary = Arc::new(MockAgent::fail_with("primary died"));
        let fallback = Arc::new(MockAgent::reply("fallback saved it"));
        let mut pool = AgentPool::new(AgentRole::Implementer, primary).with_fallback(fallback);

        let id = AgentInstanceId::new(AgentRole::Implementer, "t2");
        pool.submit(AgentTask::new(id.clone(), prompt("fix"), ctx()));

        let outcome = pool.execute_next().await.unwrap();
        assert_eq!(outcome.status, InstanceStatus::Done);
        assert!(outcome.used_fallback);
        assert!(outcome.result.unwrap().success);
    }

    #[tokio::test]
    async fn pool_both_primary_and_fallback_fail() {
        let primary = Arc::new(MockAgent::fail_with("primary dead"));
        let fallback = Arc::new(MockAgent::fail_with("fallback dead too"));
        let mut pool = AgentPool::new(AgentRole::Implementer, primary).with_fallback(fallback);

        let id = AgentInstanceId::new(AgentRole::Implementer, "t3");
        pool.submit(AgentTask::new(id.clone(), prompt("doomed"), ctx()));

        let outcome = pool.execute_next().await.unwrap();
        assert_eq!(outcome.status, InstanceStatus::Failed);
        assert!(outcome.used_fallback);
        assert!(!outcome.result.unwrap().success);
    }

    #[tokio::test]
    async fn pool_no_fallback_primary_fails() {
        let primary = Arc::new(MockAgent::fail_with("boom"));
        let mut pool = AgentPool::new(AgentRole::Implementer, primary);

        let id = AgentInstanceId::new(AgentRole::Implementer, "t4");
        pool.submit(AgentTask::new(id.clone(), prompt("fail"), ctx()));

        let outcome = pool.execute_next().await.unwrap();
        assert_eq!(outcome.status, InstanceStatus::Failed);
        assert!(!outcome.used_fallback);
    }

    #[tokio::test]
    async fn pool_execute_all_sequential() {
        let agent = Arc::new(MockAgent::reply("ok"));
        let mut pool = AgentPool::new(AgentRole::Implementer, agent);

        for i in 0..3 {
            let id = AgentInstanceId::new(AgentRole::Implementer, format!("t{i}"));
            pool.submit(AgentTask::new(id, prompt(&format!("task {i}")), ctx()));
        }

        assert_eq!(pool.pending_count(), 3);
        let outcomes = pool.execute_all().await;
        assert_eq!(outcomes.len(), 3);
        assert!(outcomes.iter().all(|o| o.status == InstanceStatus::Done));
        assert_eq!(pool.pending_count(), 0);
    }

    #[tokio::test]
    async fn pool_submit_all_preserves_order() {
        let agent = Arc::new(MockAgent::reply("ok"));
        let mut pool = AgentPool::new(AgentRole::Implementer, agent);

        let tasks = (0..3).map(|i| {
            AgentTask::new(
                AgentInstanceId::new(AgentRole::Implementer, format!("bulk-{i}")),
                prompt(&format!("task {i}")),
                ctx(),
            )
        });
        pool.submit_all(tasks);

        let outcomes = pool.execute_all().await;
        let ids: Vec<String> = outcomes.into_iter().map(|o| o.id.instance).collect();
        assert_eq!(ids, vec!["bulk-0", "bulk-1", "bulk-2"]);
    }

    #[tokio::test]
    async fn pool_cancel_pending_task() {
        let agent = Arc::new(MockAgent::reply("ok"));
        let mut pool = AgentPool::new(AgentRole::Implementer, agent);

        let id1 = AgentInstanceId::new(AgentRole::Implementer, "keep");
        let id2 = AgentInstanceId::new(AgentRole::Implementer, "cancel-me");

        pool.submit(AgentTask::new(id1.clone(), prompt("a"), ctx()));
        pool.submit(AgentTask::new(id2.clone(), prompt("b"), ctx()));

        assert_eq!(pool.pending_count(), 2);
        assert!(pool.cancel(&id2));
        assert_eq!(pool.pending_count(), 1);

        // The cancelled task should appear as a completed outcome.
        let cancelled = pool.poll().unwrap();
        assert_eq!(cancelled.status, InstanceStatus::Cancelled);
        assert_eq!(cancelled.id, id2);
    }

    #[tokio::test]
    async fn pool_cancel_nonexistent_returns_false() {
        let agent = Arc::new(MockAgent::reply("ok"));
        let mut pool = AgentPool::new(AgentRole::Implementer, agent);
        let id = AgentInstanceId::new(AgentRole::Implementer, "ghost");
        assert!(!pool.cancel(&id));
    }

    #[tokio::test]
    async fn pool_poll_returns_none_when_empty() {
        let agent = Arc::new(MockAgent::reply("ok"));
        let mut pool = AgentPool::new(AgentRole::Implementer, agent);
        assert!(pool.poll().is_none());
    }

    #[tokio::test]
    async fn pool_drain_completed_returns_every_buffered_outcome() {
        let agent = Arc::new(MockAgent::reply("ok"));
        let mut pool = AgentPool::new(AgentRole::Implementer, agent);

        for i in 0..2 {
            pool.submit(AgentTask::new(
                AgentInstanceId::new(AgentRole::Implementer, format!("done-{i}")),
                prompt("x"),
                ctx(),
            ));
        }

        let _ = pool.execute_all().await;
        let drained = pool.drain_completed();
        assert_eq!(drained.len(), 2);
        assert!(pool.poll().is_none());
    }

    #[tokio::test]
    async fn pool_execute_next_returns_none_when_empty() {
        let agent = Arc::new(MockAgent::reply("ok"));
        let mut pool = AgentPool::new(AgentRole::Implementer, agent);
        assert!(pool.execute_next().await.is_none());
    }

    #[test]
    fn pool_role_accessor() {
        let agent = Arc::new(MockAgent::reply("x"));
        let pool = AgentPool::new(AgentRole::Auditor, agent);
        assert_eq!(pool.role(), AgentRole::Auditor);
    }
}
