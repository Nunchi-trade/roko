//! `RunState` — mutable per-execution state tracking agent output, tokens,
//! and progress across all plans.

use std::collections::{HashMap, HashSet};
use std::time::Duration;
use std::time::Instant;

use super::types::{
    AgentDispatchOutcome, PlanLifecycleStatus, RetryAction, RunnerEvent, RunnerFailureKind,
    RunnerLifecycleProjection, RunnerRunStatus, TaskAttemptLifecycle, TaskAttemptOutcome,
    TaskAttemptRef, TaskAttemptStatus,
};

/// Mutable state for the current runner execution.
#[derive(Debug)]
pub struct RunState {
    // ─── Runtime Lifecycle ──────────────────────────────────────────
    /// Typed runtime lifecycle projection updated from runner events.
    pub lifecycle: RunnerLifecycleProjection,

    // ─── Agent ──────────────────────────────────────────────────────
    /// Whether an agent process is currently alive.
    pub agent_active: bool,
    /// Model name reported by the agent's SystemInit.
    pub agent_model: String,
    /// Provider/backend reported by the dispatch layer.
    pub agent_provider: String,
    /// Accumulated text output from the current agent run.
    pub agent_output: String,
    /// Session ID from the last SystemInit or TurnCompleted.
    pub session_id: Option<String>,
    /// PID of the current agent process.
    pub agent_pid: Option<u32>,
    /// Whether the current agent emitted a structured turn-completed event.
    pub agent_turn_completed: bool,

    // ─── Tokens / Cost ──────────────────────────────────────────────
    /// Input tokens consumed this task.
    pub tokens_in: u64,
    /// Output tokens consumed this task.
    pub tokens_out: u64,
    /// Cache read tokens this task (subset of input).
    pub cache_read_tokens: u64,
    /// Cache write tokens this task.
    pub cache_write_tokens: u64,
    /// Estimated cost in USD this task.
    pub cost_usd: f64,
    /// Number of agent spawn attempts for the current task (retries).
    pub task_agent_calls: u32,

    // ─── Current task ───────────────────────────────────────────────
    /// Plan currently being executed.
    pub plan_id: String,
    /// Task currently being executed.
    pub current_task: String,

    // ─── Progress ───────────────────────────────────────────────────
    /// Number of tasks completed across all plans.
    pub tasks_completed: usize,
    /// Number of tasks that failed.
    pub tasks_failed: usize,
    /// Total tasks across all plans.
    pub tasks_total: usize,
    /// Current gate output (last gate run).
    pub gate_output: String,
    /// Iteration count for the current task (retries).
    pub iteration: u32,

    // ─── Totals ─────────────────────────────────────────────────────
    /// Total input tokens across the entire run.
    pub total_tokens_in: u64,
    /// Total output tokens across the entire run.
    pub total_tokens_out: u64,
    /// Total cost in USD across the entire run.
    pub total_cost_usd: f64,
    /// Total agent spawns across the entire run.
    pub total_agent_calls: usize,
    /// Cost accumulated per plan_id (for per-plan budget enforcement).
    pub plan_costs: HashMap<String, f64>,
    /// Current retry backoff deadline per plan.
    pub retry_backoff_until: HashMap<String, Instant>,
    /// Last structured failure kind per plan.
    pub last_failure_kind: HashMap<String, RunnerFailureKind>,
    /// Gate/verify effect keys currently running in background tasks.
    pub active_gate_effects: HashSet<String>,

    // ─── Task DAG ───────────────────────────────────────────────────
    /// Completed task IDs per plan (for DAG dependency resolution).
    pub completed_tasks: HashMap<String, Vec<String>>,

    // ─── Health ──────────────────────────────────────────────────────
    /// Consecutive snapshot-save failures. After 3, `snapshot_degraded` is set.
    pub snapshot_fail_streak: u32,
    /// Set after 3 consecutive snapshot failures — crash recovery data may be stale.
    pub snapshot_degraded: bool,

    // ─── Timing ─────────────────────────────────────────────────────
    /// When the run started.
    pub started_at: Instant,
    /// When the current task started (reset per task).
    pub task_started_at: Instant,

    // ─── Replan Context ──────────────────────────────────────────────
    /// Accumulated failure context per plan/task for retry prompt enrichment.
    pub replan_contexts: HashMap<String, String>,
}

impl RunState {
    /// Create a new empty run state.
    pub fn new(total_tasks: usize) -> Self {
        Self {
            lifecycle: RunnerLifecycleProjection::new(total_tasks),
            agent_active: false,
            agent_model: String::new(),
            agent_provider: String::new(),
            agent_output: String::new(),
            session_id: None,
            agent_pid: None,
            agent_turn_completed: false,
            tokens_in: 0,
            tokens_out: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: 0.0,
            task_agent_calls: 0,
            plan_id: String::new(),
            current_task: String::new(),
            tasks_completed: 0,
            tasks_failed: 0,
            tasks_total: total_tasks,
            gate_output: String::new(),
            iteration: 0,
            total_tokens_in: 0,
            total_tokens_out: 0,
            total_cost_usd: 0.0,
            total_agent_calls: 0,
            plan_costs: HashMap::new(),
            retry_backoff_until: HashMap::new(),
            last_failure_kind: HashMap::new(),
            active_gate_effects: HashSet::new(),
            completed_tasks: HashMap::new(),
            snapshot_fail_streak: 0,
            snapshot_degraded: false,
            started_at: Instant::now(),
            task_started_at: Instant::now(),
            replan_contexts: HashMap::new(),
        }
    }

    /// Stable ID for this runner invocation.
    pub fn run_id(&self) -> &str {
        &self.lifecycle.run_id
    }

    /// Current task attempt reference, using at least attempt 1.
    pub fn current_attempt_ref(&self) -> TaskAttemptRef {
        TaskAttemptRef::new(
            self.plan_id.clone(),
            self.current_task.clone(),
            self.iteration.max(1),
        )
    }

    /// Apply a normalized runner event to the in-memory lifecycle projection.
    pub fn apply_runner_event(&mut self, event: &RunnerEvent) {
        self.lifecycle.events_seen = self.lifecycle.events_seen.saturating_add(1);

        match event {
            RunnerEvent::ResumeMarker { marker, .. } => {
                self.lifecycle.resumed =
                    matches!(marker.outcome, super::types::ResumeOutcome::Resumed);
                self.lifecycle.last_resume_marker = Some(marker.clone());
            }
            RunnerEvent::RunStarted {
                total_tasks,
                resumed,
                ..
            } => {
                self.lifecycle.status = RunnerRunStatus::Running;
                self.lifecycle.total_tasks = *total_tasks;
                self.lifecycle.resumed = *resumed;
            }
            RunnerEvent::RunCompleted { outcome, .. } => {
                self.lifecycle.status = match outcome {
                    super::types::RunOutcome::Succeeded => RunnerRunStatus::Completed,
                    super::types::RunOutcome::Failed => RunnerRunStatus::Failed,
                    super::types::RunOutcome::Cancelled => RunnerRunStatus::Cancelled,
                };
            }
            RunnerEvent::PlanStarted { plan_id, .. } => {
                self.lifecycle
                    .plans
                    .insert(plan_id.clone(), PlanLifecycleStatus::Started);
            }
            RunnerEvent::PlanCompleted {
                plan_id, outcome, ..
            } => {
                self.lifecycle
                    .plans
                    .insert(plan_id.clone(), (*outcome).into());
            }
            RunnerEvent::TaskAttemptStarted {
                attempt,
                status,
                timestamp_ms,
                ..
            } => {
                self.upsert_attempt(attempt, *status, *timestamp_ms, None, None);
            }
            RunnerEvent::AgentDispatchStarted {
                attempt,
                agent_id,
                timestamp_ms,
                ..
            } => {
                self.upsert_attempt(
                    attempt,
                    TaskAttemptStatus::DispatchingAgent,
                    *timestamp_ms,
                    Some(agent_id.clone()),
                    None,
                );
            }
            RunnerEvent::AgentDispatchCompleted {
                attempt,
                outcome,
                agent_id,
                timestamp_ms,
                ..
            } => {
                let status = match outcome {
                    AgentDispatchOutcome::Spawned => TaskAttemptStatus::AgentRunning,
                    AgentDispatchOutcome::SpawnFailed | AgentDispatchOutcome::Failed => {
                        TaskAttemptStatus::Failed
                    }
                    AgentDispatchOutcome::Completed | AgentDispatchOutcome::Exited => {
                        TaskAttemptStatus::AgentCompleted
                    }
                };
                self.upsert_attempt(attempt, status, *timestamp_ms, Some(agent_id.clone()), None);
            }
            RunnerEvent::AgentCompleted {
                attempt,
                outcome,
                timestamp_ms,
                ..
            } => {
                let status = match outcome {
                    AgentDispatchOutcome::Completed | AgentDispatchOutcome::Exited => {
                        TaskAttemptStatus::AgentCompleted
                    }
                    AgentDispatchOutcome::Failed | AgentDispatchOutcome::SpawnFailed => {
                        TaskAttemptStatus::Failed
                    }
                    AgentDispatchOutcome::Spawned => TaskAttemptStatus::AgentRunning,
                };
                self.upsert_attempt(attempt, status, *timestamp_ms, None, None);
            }
            RunnerEvent::GateDispatchStarted {
                attempt,
                timestamp_ms,
                ..
            } => {
                self.upsert_attempt(
                    attempt,
                    TaskAttemptStatus::Gating,
                    *timestamp_ms,
                    None,
                    None,
                );
            }
            RunnerEvent::GateCompleted {
                attempt,
                passed,
                failure_kind,
                timestamp_ms,
                ..
            } => {
                let status = if *passed {
                    TaskAttemptStatus::Passed
                } else {
                    TaskAttemptStatus::Failed
                };
                self.upsert_attempt(attempt, status, *timestamp_ms, None, *failure_kind);
            }
            RunnerEvent::RetryDecision {
                attempt,
                action,
                failure_kind,
                timestamp_ms,
                ..
            } => {
                let status = match action {
                    RetryAction::RetryAfterBackoff => TaskAttemptStatus::Retrying,
                    RetryAction::Exhausted => TaskAttemptStatus::Exhausted,
                    RetryAction::NotRetryable => TaskAttemptStatus::Failed,
                };
                let key = attempt.key();
                self.upsert_attempt(attempt, status, *timestamp_ms, None, Some(*failure_kind));
                if let Some(attempt_state) = self.lifecycle.task_attempts.get_mut(&key) {
                    attempt_state.retry_action = Some(*action);
                }
            }
            RunnerEvent::TaskAttemptCompleted {
                attempt,
                outcome,
                failure_kind,
                timestamp_ms,
                ..
            } => {
                let status = match outcome {
                    TaskAttemptOutcome::Passed => TaskAttemptStatus::Passed,
                    TaskAttemptOutcome::Failed => TaskAttemptStatus::Failed,
                    TaskAttemptOutcome::Exhausted => TaskAttemptStatus::Exhausted,
                    TaskAttemptOutcome::Cancelled => TaskAttemptStatus::Cancelled,
                };
                let key = attempt.key();
                self.upsert_attempt(attempt, status, *timestamp_ms, None, *failure_kind);
                if let Some(attempt_state) = self.lifecycle.task_attempts.get_mut(&key) {
                    attempt_state.completed_at_ms = Some(*timestamp_ms);
                }
            }
        }
    }

    fn upsert_attempt(
        &mut self,
        attempt: &TaskAttemptRef,
        status: TaskAttemptStatus,
        timestamp_ms: u64,
        agent_id: Option<String>,
        failure_kind: Option<RunnerFailureKind>,
    ) {
        let key = attempt.key();
        let entry =
            self.lifecycle
                .task_attempts
                .entry(key)
                .or_insert_with(|| TaskAttemptLifecycle {
                    attempt: attempt.clone(),
                    status,
                    started_at_ms: timestamp_ms,
                    completed_at_ms: None,
                    agent_id: None,
                    failure_kind: None,
                    retry_action: None,
                });

        entry.status = status;
        if let Some(agent_id) = agent_id {
            entry.agent_id = Some(agent_id);
        }
        if let Some(failure_kind) = failure_kind {
            entry.failure_kind = Some(failure_kind);
        }
    }

    /// Reset per-task accumulators for a new task.
    pub fn reset_for_task(&mut self, plan_id: &str, task_id: &str) {
        self.agent_active = false;
        self.agent_model.clear();
        self.agent_provider.clear();
        self.agent_output.clear();
        self.session_id = None;
        self.agent_pid = None;
        self.agent_turn_completed = false;
        self.tokens_in = 0;
        self.tokens_out = 0;
        self.cache_read_tokens = 0;
        self.cache_write_tokens = 0;
        self.cost_usd = 0.0;
        self.task_agent_calls = 0;
        self.plan_id = plan_id.to_string();
        self.current_task = task_id.to_string();
        self.gate_output.clear();
        self.iteration = 0;
        self.task_started_at = Instant::now();
    }

    /// Record a completed task, rolling per-task stats into totals.
    pub fn task_completed(&mut self) {
        self.tasks_completed += 1;
        self.roll_into_totals();
    }

    /// Record a failed task, rolling per-task stats into totals.
    pub fn task_failed(&mut self) {
        self.tasks_failed += 1;
        self.roll_into_totals();
    }

    fn roll_into_totals(&mut self) {
        self.total_tokens_in += self.tokens_in;
        self.total_tokens_out += self.tokens_out;
        self.total_cost_usd += self.cost_usd;
        // Track per-plan cost for budget enforcement.
        if !self.plan_id.is_empty() {
            *self.plan_costs.entry(self.plan_id.clone()).or_default() += self.cost_usd;
        }
    }

    /// Cost accumulated for a specific plan.
    pub fn plan_cost(&self, plan_id: &str) -> f64 {
        self.plan_costs.get(plan_id).copied().unwrap_or(0.0)
    }

    /// Whether a plan is still cooling down before retry dispatch.
    pub fn retry_cooldown_remaining(&self, plan_id: &str) -> Option<Duration> {
        let deadline = self.retry_backoff_until.get(plan_id)?;
        deadline.checked_duration_since(Instant::now())
    }

    /// Record retry cooldown and classification after a failed gate.
    pub fn set_retry_backoff(
        &mut self,
        plan_id: &str,
        failure_kind: RunnerFailureKind,
        attempt: u32,
    ) {
        self.last_failure_kind
            .insert(plan_id.to_string(), failure_kind);
        let base = failure_kind.retry_cooldown_secs();
        if base == 0 {
            self.retry_backoff_until.remove(plan_id);
            return;
        }
        let multiplier = 1u64.checked_shl(attempt.min(5)).unwrap_or(32);
        let delay = Duration::from_secs(base.saturating_mul(multiplier).min(45));
        self.retry_backoff_until
            .insert(plan_id.to_string(), Instant::now() + delay);
    }

    /// Clear retry backoff state for a plan after successful forward progress.
    pub fn clear_retry_backoff(&mut self, plan_id: &str) {
        self.retry_backoff_until.remove(plan_id);
        self.last_failure_kind.remove(plan_id);
    }

    /// Returns true when this gate effect was newly marked active.
    pub fn mark_gate_active(&mut self, key: impl Into<String>) -> bool {
        self.active_gate_effects.insert(key.into())
    }

    /// Clear a finished gate effect key.
    pub fn clear_gate_active(&mut self, key: &str) {
        self.active_gate_effects.remove(key);
    }

    /// Wall time for the current task (since last `reset_for_task`).
    pub fn task_elapsed_ms(&self) -> u64 {
        self.task_started_at.elapsed().as_millis() as u64
    }

    /// Record a successful snapshot save — resets the failure streak.
    pub fn snapshot_succeeded(&mut self) {
        self.snapshot_fail_streak = 0;
    }

    /// Record a failed snapshot save. After 3 consecutive failures, sets degraded flag.
    pub fn snapshot_failed(&mut self) {
        self.snapshot_fail_streak += 1;
        if self.snapshot_fail_streak >= 3 && !self.snapshot_degraded {
            self.snapshot_degraded = true;
            tracing::warn!(
                streak = self.snapshot_fail_streak,
                "snapshot persistence degraded — crash recovery data may be stale"
            );
        }
    }

    /// Mark a task as completed for DAG dependency tracking.
    pub fn mark_task_completed(&mut self, plan_id: &str, task_id: &str) {
        self.completed_tasks
            .entry(plan_id.to_string())
            .or_default()
            .push(task_id.to_string());
    }

    /// Get completed task IDs for a plan.
    pub fn plan_completed_tasks(&self, plan_id: &str) -> &[String] {
        self.completed_tasks
            .get(plan_id)
            .map(|v| v.as_slice())
            .unwrap_or_default()
    }

    /// Total elapsed time since the run started.
    pub fn elapsed(&self) -> std::time::Duration {
        self.started_at.elapsed()
    }

    /// Store failure context for a task to enrich the next retry prompt.
    pub fn set_replan_context(&mut self, plan_id: &str, task_id: &str, context: String) {
        self.replan_contexts
            .insert(format!("{plan_id}/{task_id}"), context);
    }

    /// Take (and remove) stored replan context for a task.
    pub fn take_replan_context(&mut self, plan_id: &str, task_id: &str) -> Option<String> {
        self.replan_contexts.remove(&format!("{plan_id}/{task_id}"))
    }
}
