//! `RunState` — mutable per-execution state tracking agent output, tokens,
//! and progress across all plans.

use std::collections::HashMap;
use std::time::Instant;

/// Mutable state for the current runner execution.
#[derive(Debug)]
pub struct RunState {
    // ─── Agent ──────────────────────────────────────────────────────
    /// Whether an agent process is currently alive.
    pub agent_active: bool,
    /// Model name reported by the agent's SystemInit.
    pub agent_model: String,
    /// Accumulated text output from the current agent run.
    pub agent_output: String,
    /// Session ID from the last SystemInit or TurnCompleted.
    pub session_id: Option<String>,
    /// PID of the current agent process.
    pub agent_pid: Option<u32>,

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
}

impl RunState {
    /// Create a new empty run state.
    pub fn new(total_tasks: usize) -> Self {
        Self {
            agent_active: false,
            agent_model: String::new(),
            agent_output: String::new(),
            session_id: None,
            agent_pid: None,
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
            snapshot_fail_streak: 0,
            snapshot_degraded: false,
            started_at: Instant::now(),
            task_started_at: Instant::now(),
        }
    }

    /// Reset per-task accumulators for a new task.
    pub fn reset_for_task(&mut self, plan_id: &str, task_id: &str) {
        self.agent_active = false;
        self.agent_model.clear();
        self.agent_output.clear();
        self.session_id = None;
        self.agent_pid = None;
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

    /// Total elapsed time since the run started.
    pub fn elapsed(&self) -> std::time::Duration {
        self.started_at.elapsed()
    }
}
