//! Task tracking system for agent work coordination.
//!
//! Provides a task lifecycle (Open -> Assigned -> InProgress -> Completed/Failed/Cancelled)
//! with stake/reward economics, tag-based matching, and retry limits. The [`TaskStore`]
//! is wired into [`super::super::chain_rpc::ChainContext`] and exposed via HTTP
//! (`/api/tasks/*`) endpoints.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// Task identifier (auto-incrementing u64).
pub type TaskId = u64;

/// Task lifecycle states.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// Task is open and available for assignment.
    Open,
    /// Task is assigned to an agent.
    Assigned,
    /// Agent is actively working on the task.
    InProgress,
    /// Task completed successfully.
    Completed,
    /// Task failed.
    Failed,
    /// Task was cancelled.
    Cancelled,
}

impl fmt::Display for TaskState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open => write!(f, "open"),
            Self::Assigned => write!(f, "assigned"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// Task priority levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    /// Low priority — background work.
    Low,
    /// Medium priority — normal work items.
    Medium,
    /// High priority — important work.
    High,
    /// Critical priority — must be handled immediately.
    Critical,
}

/// A task that can be created, assigned to an agent, worked on, and completed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskEntry {
    /// Unique task identifier.
    pub id: TaskId,
    /// Short human-readable title.
    pub title: String,
    /// Detailed description of the work.
    pub description: String,
    /// Task kind: "research", "validate", "analyze", "monitor", "report", etc.
    pub kind: String,
    /// Priority level for scheduling.
    pub priority: TaskPriority,
    /// Current lifecycle state.
    pub state: TaskState,
    /// Agent ID that created the task.
    pub creator: String,
    /// Agent ID assigned to work on the task.
    pub assignee: Option<String>,
    /// Unix timestamp when the task was created.
    pub created_at: u64,
    /// Unix timestamp when the task was assigned.
    pub assigned_at: Option<u64>,
    /// Unix timestamp when work started.
    pub started_at: Option<u64>,
    /// Unix timestamp when the task reached a terminal state.
    pub completed_at: Option<u64>,
    /// Stake deposited for this task (wei).
    pub stake_wei: u128,
    /// Reward paid on completion (wei).
    pub reward_wei: u128,
    /// ID of the insight produced as a result.
    pub result_insight_id: Option<String>,
    /// Parent task when this task is an improvement follow-up.
    #[serde(default)]
    pub parent_task_id: Option<TaskId>,
    /// Task deliverables recorded on completion.
    #[serde(default)]
    pub artifacts: Vec<TaskArtifact>,
    /// Human-readable completion summary.
    #[serde(default)]
    pub summary: Option<String>,
    /// Runtime metadata captured on completion.
    #[serde(default)]
    pub completion_metadata: Option<CompletionMetadata>,
    /// Topic tags for matching.
    pub tags: Vec<String>,
    /// Number of times this task was attempted.
    pub attempts: u32,
    /// Maximum attempts before auto-cancel.
    pub max_attempts: u32,
}

/// A deliverable produced by a completed task.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskArtifact {
    /// Artifact category, e.g. `code`, `report`, or `data`.
    #[serde(default)]
    pub kind: String,
    /// Human-readable artifact label.
    #[serde(default, alias = "name")]
    pub label: String,
    /// Stable content hash.
    #[serde(default, alias = "hash")]
    pub content_hash: String,
    /// Optional storage location for the artifact.
    #[serde(default)]
    pub uri: Option<String>,
    /// Optional byte size.
    #[serde(default, alias = "size")]
    pub size_bytes: Option<u64>,
    /// Optional MIME-like content type.
    #[serde(default)]
    pub content_type: Option<String>,
}

/// Additional completion metadata for a task run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CompletionMetadata {
    /// End-to-end task duration in milliseconds.
    #[serde(default)]
    pub duration_ms: u64,
    /// Model used for the terminal completion path.
    #[serde(default, alias = "model")]
    pub model_used: Option<String>,
    /// Input tokens consumed.
    #[serde(default)]
    pub tokens_in: Option<u64>,
    /// Output tokens consumed.
    #[serde(default)]
    pub tokens_out: Option<u64>,
    /// Approximate completion cost in USD.
    #[serde(default)]
    pub cost_usd: Option<f64>,
    /// Optional completion method label.
    #[serde(default)]
    pub method: Option<String>,
    /// Optional chain snapshot block.
    #[serde(default)]
    pub snapshot_block: Option<u64>,
}

/// Task event for real-time streaming.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskEvent {
    /// A new task was created.
    Created {
        /// Task identifier.
        id: TaskId,
        /// Task title.
        title: String,
        /// Task kind.
        kind: String,
        /// Agent that created the task.
        creator: String,
    },
    /// A task was assigned to an agent.
    Assigned {
        /// Task identifier.
        id: TaskId,
        /// Agent assigned to the task.
        assignee: String,
    },
    /// An agent started working on a task.
    Started {
        /// Task identifier.
        id: TaskId,
        /// Agent working on the task.
        assignee: String,
    },
    /// A task was completed successfully.
    Completed {
        /// Task identifier.
        id: TaskId,
        /// Agent that completed the task.
        assignee: String,
        /// Optional insight produced as a result.
        result_insight_id: Option<String>,
    },
    /// A task failed.
    Failed {
        /// Task identifier.
        id: TaskId,
        /// Agent that was working on the task.
        assignee: String,
        /// Reason for failure.
        reason: String,
    },
    /// A task was cancelled.
    Cancelled {
        /// Task identifier.
        id: TaskId,
        /// Reason for cancellation.
        reason: String,
    },
}

/// Aggregate stats across all tasks.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskStats {
    /// Number of tasks in `Open` state.
    pub open: usize,
    /// Number of tasks in `Assigned` state.
    pub assigned: usize,
    /// Number of tasks in `InProgress` state.
    pub in_progress: usize,
    /// Number of tasks in `Completed` state.
    pub completed: usize,
    /// Number of tasks in `Failed` state.
    pub failed: usize,
    /// Number of tasks in `Cancelled` state.
    pub cancelled: usize,
    /// Sum of all task stakes (wei).
    pub total_stake_wei: u128,
    /// Sum of all task rewards paid (wei).
    pub total_reward_wei: u128,
}

/// Errors returned from [`TaskStore`] operations.
#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    /// No task with the given id.
    #[error("task not found: {0}")]
    NotFound(TaskId),
    /// Task is in wrong state for the requested transition.
    #[error("task {id}: expected state {expected}, got {current}")]
    InvalidState {
        /// Task identifier.
        id: TaskId,
        /// Actual current state.
        current: TaskState,
        /// State required for the operation.
        expected: TaskState,
    },
    /// Task is already assigned to an agent.
    #[error("task already assigned: {0}")]
    AlreadyAssigned(TaskId),
    /// Task has exceeded maximum attempts.
    #[error("task {0} exceeded max attempts")]
    MaxAttempts(TaskId),
    /// Improvement tasks must inherit a concrete assignee from the parent.
    #[error("task {0} has no assignee to reuse for improvement")]
    ImprovementTargetUnassigned(TaskId),
}

/// In-memory task store with auto-incrementing IDs and lifecycle management.
pub struct TaskStore {
    tasks: HashMap<TaskId, TaskEntry>,
    next_id: TaskId,
}

impl TaskStore {
    /// Creates a new empty task store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            next_id: 1,
        }
    }

    /// Create a new task. Returns the assigned task ID.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &mut self,
        title: String,
        description: String,
        kind: String,
        priority: TaskPriority,
        creator: String,
        tags: Vec<String>,
        stake_wei: u128,
        now: u64,
    ) -> TaskId {
        let id = self.next_id;
        self.next_id += 1;
        let entry = TaskEntry {
            id,
            title,
            description,
            kind,
            priority,
            state: TaskState::Open,
            creator,
            assignee: None,
            created_at: now,
            assigned_at: None,
            started_at: None,
            completed_at: None,
            stake_wei,
            reward_wei: 0,
            result_insight_id: None,
            parent_task_id: None,
            artifacts: Vec::new(),
            summary: None,
            completion_metadata: None,
            tags,
            attempts: 0,
            max_attempts: 3,
        };
        self.tasks.insert(id, entry);
        id
    }

    /// Create a follow-up improvement task for a completed parent task.
    pub fn create_improvement(
        &mut self,
        parent_id: TaskId,
        feedback: String,
        creator: String,
        now: u64,
    ) -> Result<TaskId, TaskError> {
        let parent = self
            .tasks
            .get(&parent_id)
            .cloned()
            .ok_or(TaskError::NotFound(parent_id))?;
        if parent.state != TaskState::Completed {
            return Err(TaskError::InvalidState {
                id: parent_id,
                current: parent.state,
                expected: TaskState::Completed,
            });
        }

        let Some(parent_assignee) = parent.assignee.clone() else {
            return Err(TaskError::ImprovementTargetUnassigned(parent_id));
        };

        let id = self.next_id;
        self.next_id += 1;
        let entry = TaskEntry {
            id,
            title: format!("Improve: {}", parent.title),
            description: feedback,
            kind: "improvement".to_string(),
            priority: parent.priority,
            state: TaskState::Assigned,
            creator,
            assignee: Some(parent_assignee),
            created_at: now,
            assigned_at: Some(now),
            started_at: None,
            completed_at: None,
            stake_wei: 0,
            reward_wei: 0,
            result_insight_id: None,
            parent_task_id: Some(parent_id),
            artifacts: Vec::new(),
            summary: None,
            completion_metadata: None,
            tags: parent.tags,
            attempts: 0,
            max_attempts: parent.max_attempts,
        };
        self.tasks.insert(id, entry);
        Ok(id)
    }

    /// Assign a task to an agent. Task must be in `Open` state.
    pub fn assign(&mut self, id: TaskId, assignee: String, now: u64) -> Result<(), TaskError> {
        let entry = self.tasks.get_mut(&id).ok_or(TaskError::NotFound(id))?;
        if entry.state != TaskState::Open {
            return Err(TaskError::InvalidState {
                id,
                current: entry.state,
                expected: TaskState::Open,
            });
        }
        if entry.assignee.is_some() {
            return Err(TaskError::AlreadyAssigned(id));
        }
        entry.state = TaskState::Assigned;
        entry.assignee = Some(assignee);
        entry.assigned_at = Some(now);
        Ok(())
    }

    /// Mark a task as in-progress. Task must be in `Assigned` state.
    pub fn start(&mut self, id: TaskId, now: u64) -> Result<(), TaskError> {
        let entry = self.tasks.get_mut(&id).ok_or(TaskError::NotFound(id))?;
        if entry.state != TaskState::Assigned {
            return Err(TaskError::InvalidState {
                id,
                current: entry.state,
                expected: TaskState::Assigned,
            });
        }
        entry.state = TaskState::InProgress;
        entry.started_at = Some(now);
        entry.attempts += 1;
        Ok(())
    }

    /// Complete a task, optionally linking a result insight. Returns the reward (wei).
    ///
    /// Task must be in `InProgress` state. The reward is set to equal the stake.
    pub fn complete(
        &mut self,
        id: TaskId,
        result_insight_id: Option<String>,
        artifacts: Vec<TaskArtifact>,
        summary: Option<String>,
        completion_metadata: Option<CompletionMetadata>,
        now: u64,
    ) -> Result<u128, TaskError> {
        let entry = self.tasks.get_mut(&id).ok_or(TaskError::NotFound(id))?;
        if entry.state != TaskState::InProgress {
            return Err(TaskError::InvalidState {
                id,
                current: entry.state,
                expected: TaskState::InProgress,
            });
        }
        entry.state = TaskState::Completed;
        entry.completed_at = Some(now);
        entry.result_insight_id = result_insight_id;
        entry.artifacts = artifacts;
        entry.summary = summary;
        entry.completion_metadata = completion_metadata;
        entry.reward_wei = entry.stake_wei;
        Ok(entry.reward_wei)
    }

    /// Fail a task. Task must be in `InProgress` state.
    ///
    /// If the task has not exceeded `max_attempts`, it is returned to `Open` state
    /// so it can be reassigned. Otherwise it stays in `Failed`.
    pub fn fail(&mut self, id: TaskId, _reason: String, now: u64) -> Result<(), TaskError> {
        let entry = self.tasks.get_mut(&id).ok_or(TaskError::NotFound(id))?;
        if entry.state != TaskState::InProgress {
            return Err(TaskError::InvalidState {
                id,
                current: entry.state,
                expected: TaskState::InProgress,
            });
        }
        if entry.attempts < entry.max_attempts {
            // Return to open for retry.
            entry.state = TaskState::Open;
            entry.assignee = None;
            entry.assigned_at = None;
            entry.started_at = None;
        } else {
            entry.state = TaskState::Failed;
            entry.completed_at = Some(now);
        }
        Ok(())
    }

    /// Cancel a task. Task must be in `Open`, `Assigned`, or `InProgress` state.
    pub fn cancel(&mut self, id: TaskId, _reason: String) -> Result<(), TaskError> {
        let entry = self.tasks.get_mut(&id).ok_or(TaskError::NotFound(id))?;
        match entry.state {
            TaskState::Open | TaskState::Assigned | TaskState::InProgress => {
                entry.state = TaskState::Cancelled;
                Ok(())
            }
            _ => Err(TaskError::InvalidState {
                id,
                current: entry.state,
                expected: TaskState::Open,
            }),
        }
    }

    /// List tasks with optional filters. Returns matching entries and total count.
    pub fn list(
        &self,
        state: Option<TaskState>,
        kind: Option<&str>,
        assignee: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> (Vec<&TaskEntry>, usize) {
        let mut filtered: Vec<&TaskEntry> = self
            .tasks
            .values()
            .filter(|t| state.is_none() || Some(t.state) == state)
            .filter(|t| kind.is_none() || kind == Some(t.kind.as_str()))
            .filter(|t| {
                assignee.is_none() || t.assignee.as_deref().is_some_and(|a| Some(a) == assignee)
            })
            .collect();
        // Sort by id ascending (creation order).
        filtered.sort_by_key(|t| t.id);
        let total = filtered.len();
        let items = filtered.into_iter().skip(offset).take(limit).collect();
        (items, total)
    }

    /// Get open tasks suitable for an agent based on tag overlap.
    pub fn available_for(&self, agent_tags: &[String]) -> Vec<&TaskEntry> {
        let mut matches: Vec<&TaskEntry> = self
            .tasks
            .values()
            .filter(|t| t.state == TaskState::Open)
            .filter(|t| {
                if agent_tags.is_empty() {
                    return true;
                }
                t.tags.iter().any(|tag| agent_tags.contains(tag))
            })
            .collect();
        // Higher priority first, then older tasks first.
        matches.sort_by(|a, b| {
            priority_ord(b.priority)
                .cmp(&priority_ord(a.priority))
                .then(a.id.cmp(&b.id))
        });
        matches
    }

    /// Get a single task by ID.
    pub fn get(&self, id: TaskId) -> Option<&TaskEntry> {
        self.tasks.get(&id)
    }

    /// Number of tasks in the store.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    /// Whether the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Aggregate stats: counts per state and totals.
    #[must_use]
    pub fn stats(&self) -> TaskStats {
        let mut s = TaskStats::default();
        for t in self.tasks.values() {
            match t.state {
                TaskState::Open => s.open += 1,
                TaskState::Assigned => s.assigned += 1,
                TaskState::InProgress => s.in_progress += 1,
                TaskState::Completed => s.completed += 1,
                TaskState::Failed => s.failed += 1,
                TaskState::Cancelled => s.cancelled += 1,
            }
            s.total_stake_wei += t.stake_wei;
            s.total_reward_wei += t.reward_wei;
        }
        s
    }
}

impl Default for TaskStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Serialisable snapshot of a [`TaskStore`] for disk persistence.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskStoreSnapshot {
    /// All tasks.
    pub tasks: Vec<TaskEntry>,
    /// Monotonic id counter.
    pub next_id: TaskId,
}

impl TaskStore {
    /// Captures a serialisable snapshot of the store.
    #[must_use]
    pub fn snapshot(&self) -> TaskStoreSnapshot {
        let mut tasks: Vec<TaskEntry> = self.tasks.values().cloned().collect();
        tasks.sort_by_key(|t| t.id);
        TaskStoreSnapshot {
            tasks,
            next_id: self.next_id,
        }
    }

    /// Restores a store from a snapshot.
    #[must_use]
    pub fn from_snapshot(snap: TaskStoreSnapshot) -> Self {
        let tasks = snap.tasks.into_iter().map(|t| (t.id, t)).collect();
        Self {
            tasks,
            next_id: snap.next_id,
        }
    }
}

/// Map priority to a numeric ordering value for sorting.
fn priority_ord(p: TaskPriority) -> u8 {
    match p {
        TaskPriority::Low => 0,
        TaskPriority::Medium => 1,
        TaskPriority::High => 2,
        TaskPriority::Critical => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_complete_lifecycle() {
        let mut store = TaskStore::new();
        let id = store.create(
            "research uniswap v4".into(),
            "deep dive on hooks".into(),
            "research".into(),
            TaskPriority::High,
            "agent-1".into(),
            vec!["defi".into(), "uniswap".into()],
            1_000_000,
            100,
        );
        assert_eq!(id, 1);
        assert_eq!(store.len(), 1);

        store.assign(id, "agent-2".into(), 110).unwrap();
        let entry = store.get(id).unwrap();
        assert_eq!(entry.state, TaskState::Assigned);
        assert_eq!(entry.assignee.as_deref(), Some("agent-2"));

        store.start(id, 120).unwrap();
        let entry = store.get(id).unwrap();
        assert_eq!(entry.state, TaskState::InProgress);
        assert_eq!(entry.attempts, 1);

        let reward = store
            .complete(
                id,
                Some("insight-abc".into()),
                vec![TaskArtifact {
                    kind: "report".into(),
                    label: "deliverable.md".into(),
                    content_hash: "sha256:abc".into(),
                    uri: Some("ipfs://artifact".into()),
                    size_bytes: Some(42),
                    content_type: Some("text/markdown".into()),
                }],
                Some("found the answer".into()),
                Some(CompletionMetadata {
                    duration_ms: 500,
                    model_used: Some("claude-sonnet-4-5".into()),
                    tokens_in: Some(100),
                    tokens_out: Some(50),
                    cost_usd: Some(0.12),
                    method: None,
                    snapshot_block: None,
                }),
                130,
            )
            .unwrap();
        assert_eq!(reward, 1_000_000);
        let entry = store.get(id).unwrap();
        assert_eq!(entry.state, TaskState::Completed);
        assert_eq!(entry.result_insight_id.as_deref(), Some("insight-abc"));
        assert_eq!(entry.artifacts.len(), 1);
        assert_eq!(entry.summary.as_deref(), Some("found the answer"));
        assert_eq!(
            entry
                .completion_metadata
                .as_ref()
                .and_then(|meta| meta.model_used.as_deref()),
            Some("claude-sonnet-4-5")
        );
    }

    #[test]
    fn fail_retries_then_fails() {
        let mut store = TaskStore::new();
        let id = store.create(
            "validate tx".into(),
            "check tx validity".into(),
            "validate".into(),
            TaskPriority::Medium,
            "agent-1".into(),
            vec![],
            500,
            100,
        );

        // Attempt 1: assign -> start -> fail (returns to Open)
        store.assign(id, "agent-2".into(), 110).unwrap();
        store.start(id, 120).unwrap();
        store.fail(id, "timeout".into(), 130).unwrap();
        assert_eq!(store.get(id).unwrap().state, TaskState::Open);
        assert_eq!(store.get(id).unwrap().attempts, 1);

        // Attempt 2
        store.assign(id, "agent-3".into(), 140).unwrap();
        store.start(id, 150).unwrap();
        store.fail(id, "timeout".into(), 160).unwrap();
        assert_eq!(store.get(id).unwrap().state, TaskState::Open);
        assert_eq!(store.get(id).unwrap().attempts, 2);

        // Attempt 3 (max): should transition to Failed
        store.assign(id, "agent-4".into(), 170).unwrap();
        store.start(id, 180).unwrap();
        store.fail(id, "crash".into(), 190).unwrap();
        assert_eq!(store.get(id).unwrap().state, TaskState::Failed);
    }

    #[test]
    fn cancel_open_task() {
        let mut store = TaskStore::new();
        let id = store.create(
            "report".into(),
            "generate report".into(),
            "report".into(),
            TaskPriority::Low,
            "agent-1".into(),
            vec![],
            0,
            100,
        );
        store.cancel(id, "no longer needed".into()).unwrap();
        assert_eq!(store.get(id).unwrap().state, TaskState::Cancelled);
    }

    #[test]
    fn create_improvement_inherits_parent_assignment() {
        let mut store = TaskStore::new();
        let parent_id = store.create(
            "ship report".into(),
            "deliver first draft".into(),
            "report".into(),
            TaskPriority::High,
            "user-1".into(),
            vec!["defi".into()],
            100,
            10,
        );
        store.assign(parent_id, "agent-7".into(), 11).unwrap();
        store.start(parent_id, 12).unwrap();
        store
            .complete(parent_id, None, Vec::new(), Some("draft".into()), None, 13)
            .unwrap();

        let child_id = store
            .create_improvement(
                parent_id,
                "tighten the conclusions".into(),
                "user-2".into(),
                20,
            )
            .unwrap();

        let child = store.get(child_id).unwrap();
        assert_eq!(child.parent_task_id, Some(parent_id));
        assert_eq!(child.kind, "improvement");
        assert_eq!(child.state, TaskState::Assigned);
        assert_eq!(child.assignee.as_deref(), Some("agent-7"));
        assert_eq!(child.tags, vec!["defi"]);
        assert_eq!(child.description, "tighten the conclusions");
    }

    #[test]
    fn create_improvement_requires_completed_parent() {
        let mut store = TaskStore::new();
        let parent_id = store.create(
            "ship report".into(),
            "deliver first draft".into(),
            "report".into(),
            TaskPriority::High,
            "user-1".into(),
            vec![],
            0,
            10,
        );

        let err = store
            .create_improvement(parent_id, "please revise".into(), "user-2".into(), 20)
            .unwrap_err();
        assert!(matches!(err, TaskError::InvalidState { .. }));
    }

    #[test]
    fn list_filters() {
        let mut store = TaskStore::new();
        store.create(
            "t1".into(),
            "".into(),
            "research".into(),
            TaskPriority::Low,
            "a".into(),
            vec![],
            0,
            100,
        );
        let id2 = store.create(
            "t2".into(),
            "".into(),
            "validate".into(),
            TaskPriority::High,
            "a".into(),
            vec![],
            0,
            101,
        );
        store.assign(id2, "b".into(), 102).unwrap();

        let (all, total) = store.list(None, None, None, 100, 0);
        assert_eq!(total, 2);
        assert_eq!(all.len(), 2);

        let (open, total) = store.list(Some(TaskState::Open), None, None, 100, 0);
        assert_eq!(total, 1);
        assert_eq!(open[0].kind, "research");

        let (by_kind, _) = store.list(None, Some("validate"), None, 100, 0);
        assert_eq!(by_kind.len(), 1);

        let (by_assignee, _) = store.list(None, None, Some("b"), 100, 0);
        assert_eq!(by_assignee.len(), 1);
    }

    #[test]
    fn available_for_tags() {
        let mut store = TaskStore::new();
        store.create(
            "t1".into(),
            "".into(),
            "research".into(),
            TaskPriority::Low,
            "a".into(),
            vec!["defi".into()],
            0,
            100,
        );
        store.create(
            "t2".into(),
            "".into(),
            "research".into(),
            TaskPriority::Critical,
            "a".into(),
            vec!["nft".into()],
            0,
            101,
        );

        let matches = store.available_for(&["defi".into()]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].title, "t1");

        // Empty tags match everything.
        let all = store.available_for(&[]);
        assert_eq!(all.len(), 2);
        // Critical should be first.
        assert_eq!(all[0].title, "t2");
    }

    #[test]
    fn stats_aggregation() {
        let mut store = TaskStore::new();
        let id1 = store.create(
            "t1".into(),
            "".into(),
            "r".into(),
            TaskPriority::Low,
            "a".into(),
            vec![],
            100,
            0,
        );
        store.create(
            "t2".into(),
            "".into(),
            "r".into(),
            TaskPriority::Low,
            "a".into(),
            vec![],
            200,
            0,
        );

        store.assign(id1, "b".into(), 1).unwrap();
        store.start(id1, 2).unwrap();
        store
            .complete(id1, None, Vec::new(), None, None, 3)
            .unwrap();

        let s = store.stats();
        assert_eq!(s.completed, 1);
        assert_eq!(s.open, 1);
        assert_eq!(s.total_stake_wei, 300);
        assert_eq!(s.total_reward_wei, 100);
    }

    #[test]
    fn invalid_transitions() {
        let mut store = TaskStore::new();
        let id = store.create(
            "t".into(),
            "".into(),
            "r".into(),
            TaskPriority::Low,
            "a".into(),
            vec![],
            0,
            0,
        );
        // Can't start before assigning.
        assert!(store.start(id, 1).is_err());
        // Can't complete before starting.
        assert!(store.complete(id, None, Vec::new(), None, None, 1).is_err());
        // Can't fail before starting.
        assert!(store.fail(id, "x".into(), 1).is_err());

        // Not found.
        assert!(store.assign(999, "b".into(), 0).is_err());
    }

    #[test]
    fn completion_artifacts_default_to_empty() {
        let mut store = TaskStore::new();
        let id = store.create(
            "t".into(),
            "".into(),
            "r".into(),
            TaskPriority::Low,
            "a".into(),
            vec![],
            0,
            0,
        );
        store.assign(id, "b".into(), 1).unwrap();
        store.start(id, 2).unwrap();
        store.complete(id, None, Vec::new(), None, None, 3).unwrap();

        let task = store.get(id).unwrap();
        assert!(task.artifacts.is_empty());
        assert!(task.summary.is_none());
        assert!(task.completion_metadata.is_none());
    }
}
