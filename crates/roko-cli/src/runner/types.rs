//! Core types for the runner v2 event-driven plan executor.
//!
//! These types form the protocol between the agent stream parser, the event
//! loop, and the TUI bridge.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

use roko_core::config::schema::RokoConfig;

// ─── Agent Events ───────────────────────────────────────────────────────

/// Events emitted by the agent stream parser.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// The runtime has launched an agent process.
    Started {
        agent_id: String,
        provider: String,
        model: String,
        pid: Option<u32>,
    },
    /// Claude CLI sent a `system` init message.
    SystemInit { session_id: String, model: String },
    /// A chunk of assistant text output.
    MessageDelta { text: String },
    /// The agent is invoking a tool.
    ToolCall { id: String, name: String },
    /// Result of a tool invocation.
    ToolOutput { id: String, output: String },
    /// Token usage update from a turn.
    TokenUsage {
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
    },
    /// An entire turn has completed.
    TurnCompleted {
        session_id: Option<String>,
        total_cost_usd: Option<f64>,
        num_turns: Option<u32>,
        is_error: bool,
    },
    /// An error from the agent process.
    Error { message: String },
    /// The agent process has exited.
    Exited { exit_code: Option<i32> },
}

impl AgentEvent {
    /// Stable normalized event type for logs/projections.
    pub const fn event_type(&self) -> &'static str {
        match self {
            Self::Started { .. } => "agent.started",
            Self::SystemInit { .. } => "agent.system_init",
            Self::MessageDelta { .. } => "agent.message_delta",
            Self::ToolCall { .. } => "agent.tool_call",
            Self::ToolOutput { .. } => "agent.tool_output",
            Self::TokenUsage { .. } => "agent.token_usage",
            Self::TurnCompleted { .. } => "agent.turn_completed",
            Self::Error { .. } => "agent.error",
            Self::Exited { .. } => "agent.exited",
        }
    }
}

// ─── Verify Completion ────────────────────────────────────────────────────

/// Which runtime effect produced a gate-like completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateCompletionKind {
    /// Gate ladder rung after implementation/autofix.
    Gate,
    /// Plan-level task verify commands after all gates pass.
    PlanVerify,
}

/// Coarse runner-level failure kind for retry policy and prompt shaping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerFailureKind {
    Transient,
    Permanent,
    Resource,
    Structural,
    Unknown,
}

impl RunnerFailureKind {
    pub const fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::Transient | Self::Permanent | Self::Structural | Self::Unknown
        )
    }

    pub const fn retry_cooldown_secs(self) -> u64 {
        match self {
            Self::Transient => 2,
            Self::Permanent => 0,
            Self::Resource => 0,
            Self::Structural => 5,
            Self::Unknown => 1,
        }
    }

    pub fn from_output(output: &str) -> Self {
        let lower = output.to_ascii_lowercase();
        if lower.contains("out of memory")
            || lower.contains("oom")
            || lower.contains("no space left")
            || lower.contains("disk full")
            || lower.contains("cannot allocate memory")
            || lower.contains("too many open files")
        {
            return Self::Resource;
        }
        if lower.contains("timed out")
            || lower.contains("timeout")
            || lower.contains("connection reset")
            || lower.contains("connection refused")
            || lower.contains("intermittent")
            || lower.contains("flaky")
        {
            return Self::Transient;
        }
        if lower.contains("verify script")
            || lower.contains("acceptance contract")
            || lower.contains("architectural conflict")
            || lower.contains("unsafe stub")
        {
            return Self::Structural;
        }
        if output.trim().is_empty() {
            Self::Unknown
        } else {
            Self::Permanent
        }
    }
}

/// Result of a gate run, sent back through the gate channel.
#[derive(Debug, Clone)]
pub struct GateCompletion {
    pub kind: GateCompletionKind,
    pub plan_id: String,
    pub task_id: String,
    pub rung: u32,
    pub passed: bool,
    pub failure_kind: Option<RunnerFailureKind>,
    pub verdicts: Vec<GateVerdictSummary>,
    pub output: String,
    pub duration_ms: u64,
}

/// Minimal gate verdict for reporting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateVerdictSummary {
    #[serde(rename = "gate", alias = "gate_name")]
    pub gate_name: String,
    pub passed: bool,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<RunnerFailureKind>,
}

// ─── Runtime Lifecycle Events ───────────────────────────────────────────

/// Stable reference to one task attempt within a run.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskAttemptRef {
    pub plan_id: String,
    pub task_id: String,
    pub attempt: u32,
}

impl TaskAttemptRef {
    pub fn new(plan_id: impl Into<String>, task_id: impl Into<String>, attempt: u32) -> Self {
        Self {
            plan_id: plan_id.into(),
            task_id: task_id.into(),
            attempt: attempt.max(1),
        }
    }

    pub fn key(&self) -> String {
        format!("{}:{}:{}", self.plan_id, self.task_id, self.attempt)
    }
}

/// Overall lifecycle status of a runner invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerRunStatus {
    Initialized,
    Running,
    Completed,
    Cancelled,
    Failed,
}

/// Terminal run outcome persisted in `run.completed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Succeeded,
    Failed,
    Cancelled,
}

/// Terminal plan outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanOutcome {
    Succeeded,
    Failed,
    Skipped,
}

/// Non-terminal and terminal plan lifecycle status held in the state projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanLifecycleStatus {
    Started,
    Succeeded,
    Failed,
    Skipped,
}

impl From<PlanOutcome> for PlanLifecycleStatus {
    fn from(value: PlanOutcome) -> Self {
        match value {
            PlanOutcome::Succeeded => Self::Succeeded,
            PlanOutcome::Failed => Self::Failed,
            PlanOutcome::Skipped => Self::Skipped,
        }
    }
}

/// Task attempt status in the runner projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskAttemptStatus {
    Started,
    DispatchingAgent,
    AgentRunning,
    AgentCompleted,
    Gating,
    Retrying,
    Passed,
    Failed,
    Exhausted,
    Cancelled,
}

/// Terminal task attempt outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskAttemptOutcome {
    Passed,
    Failed,
    Exhausted,
    Cancelled,
}

/// Result of an agent dispatch lifecycle step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDispatchOutcome {
    Spawned,
    SpawnFailed,
    Completed,
    Failed,
    Exited,
}

/// Retry policy decision after a gate-like failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryAction {
    RetryAfterBackoff,
    Exhausted,
    NotRetryable,
}

/// Snapshot/resume decision made before the event loop starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeOutcome {
    Fresh,
    Resumed,
    IgnoredStale,
    ReadFailed,
    Corrupt,
}

/// Durable resume marker for recovery/debugging projections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeMarker {
    pub outcome: ResumeOutcome,
    pub snapshot_path: String,
    pub snapshot_plan_ids: Vec<String>,
    pub current_plan_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Compact per-plan summary embedded in `run.completed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRunSummary {
    pub plan_id: String,
    pub completed: bool,
    pub tasks_total: usize,
    pub tasks_completed: usize,
    pub tasks_failed: usize,
}

/// Per-attempt projection maintained by [`RunState`](super::state::RunState).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAttemptLifecycle {
    #[serde(flatten)]
    pub attempt: TaskAttemptRef,
    pub status: TaskAttemptStatus,
    pub started_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<RunnerFailureKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_action: Option<RetryAction>,
}

/// Materialized lifecycle projection updated from typed runner events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerLifecycleProjection {
    pub run_id: String,
    pub status: RunnerRunStatus,
    pub total_tasks: usize,
    #[serde(default)]
    pub resumed: bool,
    #[serde(default)]
    pub plans: std::collections::HashMap<String, PlanLifecycleStatus>,
    #[serde(default)]
    pub task_attempts: std::collections::HashMap<String, TaskAttemptLifecycle>,
    #[serde(default)]
    pub last_resume_marker: Option<ResumeMarker>,
    #[serde(default)]
    pub events_seen: u64,
}

impl RunnerLifecycleProjection {
    pub fn new(total_tasks: usize) -> Self {
        Self {
            run_id: new_run_id(),
            status: RunnerRunStatus::Initialized,
            total_tasks,
            resumed: false,
            plans: std::collections::HashMap::new(),
            task_attempts: std::collections::HashMap::new(),
            last_resume_marker: None,
            events_seen: 0,
        }
    }
}

/// Normalized runtime event emitted by the runner event loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RunnerEvent {
    #[serde(rename = "resume.marker")]
    ResumeMarker {
        timestamp: String,
        timestamp_ms: u64,
        run_id: String,
        marker: ResumeMarker,
    },
    #[serde(rename = "run.started")]
    RunStarted {
        timestamp: String,
        timestamp_ms: u64,
        run_id: String,
        plan_ids: Vec<String>,
        total_tasks: usize,
        resumed: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resume_session: Option<String>,
    },
    #[serde(rename = "run.completed")]
    RunCompleted {
        timestamp: String,
        timestamp_ms: u64,
        run_id: String,
        outcome: RunOutcome,
        total_tasks: usize,
        tasks_completed: usize,
        tasks_failed: usize,
        total_agent_calls: usize,
        total_cost_usd: f64,
        duration_ms: u64,
        plans: Vec<PlanRunSummary>,
    },
    #[serde(rename = "plan.started")]
    PlanStarted {
        timestamp: String,
        timestamp_ms: u64,
        run_id: String,
        plan_id: String,
    },
    #[serde(rename = "plan.completed")]
    PlanCompleted {
        timestamp: String,
        timestamp_ms: u64,
        run_id: String,
        plan_id: String,
        outcome: PlanOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    #[serde(rename = "task.attempt.started")]
    TaskAttemptStarted {
        timestamp: String,
        timestamp_ms: u64,
        run_id: String,
        #[serde(flatten)]
        attempt: TaskAttemptRef,
        title: String,
        status: TaskAttemptStatus,
    },
    #[serde(rename = "task.attempt.completed")]
    TaskAttemptCompleted {
        timestamp: String,
        timestamp_ms: u64,
        run_id: String,
        #[serde(flatten)]
        attempt: TaskAttemptRef,
        outcome: TaskAttemptOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure_kind: Option<RunnerFailureKind>,
        duration_ms: u64,
    },
    #[serde(rename = "agent.dispatch.started")]
    AgentDispatchStarted {
        timestamp: String,
        timestamp_ms: u64,
        run_id: String,
        #[serde(flatten)]
        attempt: TaskAttemptRef,
        agent_id: String,
        role: String,
        requested_model: String,
    },
    #[serde(rename = "agent.dispatch.completed")]
    AgentDispatchCompleted {
        timestamp: String,
        timestamp_ms: u64,
        run_id: String,
        #[serde(flatten)]
        attempt: TaskAttemptRef,
        agent_id: String,
        outcome: AgentDispatchOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pid: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    #[serde(rename = "agent.completed")]
    AgentCompleted {
        timestamp: String,
        timestamp_ms: u64,
        run_id: String,
        #[serde(flatten)]
        attempt: TaskAttemptRef,
        agent_id: String,
        outcome: AgentDispatchOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total_cost_usd: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turns: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    #[serde(rename = "gate.dispatch.started")]
    GateDispatchStarted {
        timestamp: String,
        timestamp_ms: u64,
        run_id: String,
        #[serde(flatten)]
        attempt: TaskAttemptRef,
        kind: GateCompletionKind,
        rung: u32,
    },
    #[serde(rename = "gate.completed")]
    GateCompleted {
        timestamp: String,
        timestamp_ms: u64,
        run_id: String,
        #[serde(flatten)]
        attempt: TaskAttemptRef,
        kind: GateCompletionKind,
        rung: u32,
        passed: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure_kind: Option<RunnerFailureKind>,
        duration_ms: u64,
        output: String,
        verdicts: Vec<GateVerdictSummary>,
    },
    #[serde(rename = "retry.decision")]
    RetryDecision {
        timestamp: String,
        timestamp_ms: u64,
        run_id: String,
        #[serde(flatten)]
        attempt: TaskAttemptRef,
        action: RetryAction,
        failure_kind: RunnerFailureKind,
        current_attempt: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_attempt: Option<u32>,
        cooldown_ms: u64,
        reason: String,
    },
}

impl RunnerEvent {
    pub fn resume_marker(run_id: &str, marker: ResumeMarker) -> Self {
        let stamp = EventStamp::now();
        Self::ResumeMarker {
            timestamp: stamp.timestamp,
            timestamp_ms: stamp.timestamp_ms,
            run_id: run_id.to_string(),
            marker,
        }
    }

    pub fn run_started(
        run_id: &str,
        plan_ids: Vec<String>,
        total_tasks: usize,
        resumed: bool,
        resume_session: Option<String>,
    ) -> Self {
        let stamp = EventStamp::now();
        Self::RunStarted {
            timestamp: stamp.timestamp,
            timestamp_ms: stamp.timestamp_ms,
            run_id: run_id.to_string(),
            plan_ids,
            total_tasks,
            resumed,
            resume_session,
        }
    }

    pub fn run_completed(
        run_id: &str,
        outcome: RunOutcome,
        totals: RunTotals,
        plans: Vec<PlanRunSummary>,
    ) -> Self {
        let stamp = EventStamp::now();
        Self::RunCompleted {
            timestamp: stamp.timestamp,
            timestamp_ms: stamp.timestamp_ms,
            run_id: run_id.to_string(),
            outcome,
            total_tasks: totals.total_tasks,
            tasks_completed: totals.tasks_completed,
            tasks_failed: totals.tasks_failed,
            total_agent_calls: totals.total_agent_calls,
            total_cost_usd: totals.total_cost_usd,
            duration_ms: totals.duration_ms,
            plans,
        }
    }

    pub fn plan_started(run_id: &str, plan_id: &str) -> Self {
        let stamp = EventStamp::now();
        Self::PlanStarted {
            timestamp: stamp.timestamp,
            timestamp_ms: stamp.timestamp_ms,
            run_id: run_id.to_string(),
            plan_id: plan_id.to_string(),
        }
    }

    pub fn plan_completed(
        run_id: &str,
        plan_id: &str,
        outcome: PlanOutcome,
        reason: Option<String>,
    ) -> Self {
        let stamp = EventStamp::now();
        Self::PlanCompleted {
            timestamp: stamp.timestamp,
            timestamp_ms: stamp.timestamp_ms,
            run_id: run_id.to_string(),
            plan_id: plan_id.to_string(),
            outcome,
            reason,
        }
    }

    pub fn task_attempt_started(run_id: &str, attempt: TaskAttemptRef, title: &str) -> Self {
        let stamp = EventStamp::now();
        Self::TaskAttemptStarted {
            timestamp: stamp.timestamp,
            timestamp_ms: stamp.timestamp_ms,
            run_id: run_id.to_string(),
            attempt,
            title: title.to_string(),
            status: TaskAttemptStatus::Started,
        }
    }

    pub fn task_attempt_completed(
        run_id: &str,
        attempt: TaskAttemptRef,
        outcome: TaskAttemptOutcome,
        failure_kind: Option<RunnerFailureKind>,
        duration_ms: u64,
    ) -> Self {
        let stamp = EventStamp::now();
        Self::TaskAttemptCompleted {
            timestamp: stamp.timestamp,
            timestamp_ms: stamp.timestamp_ms,
            run_id: run_id.to_string(),
            attempt,
            outcome,
            failure_kind,
            duration_ms,
        }
    }

    pub fn agent_dispatch_started(
        run_id: &str,
        attempt: TaskAttemptRef,
        agent_id: &str,
        role: &str,
        requested_model: &str,
    ) -> Self {
        let stamp = EventStamp::now();
        Self::AgentDispatchStarted {
            timestamp: stamp.timestamp,
            timestamp_ms: stamp.timestamp_ms,
            run_id: run_id.to_string(),
            attempt,
            agent_id: agent_id.to_string(),
            role: role.to_string(),
            requested_model: requested_model.to_string(),
        }
    }

    pub fn agent_dispatch_completed(
        run_id: &str,
        attempt: TaskAttemptRef,
        agent_id: &str,
        outcome: AgentDispatchOutcome,
        model: Option<String>,
        pid: Option<u32>,
        message: Option<String>,
    ) -> Self {
        let stamp = EventStamp::now();
        Self::AgentDispatchCompleted {
            timestamp: stamp.timestamp,
            timestamp_ms: stamp.timestamp_ms,
            run_id: run_id.to_string(),
            attempt,
            agent_id: agent_id.to_string(),
            outcome,
            provider: None,
            model,
            pid,
            message,
        }
    }

    pub fn agent_completed(
        run_id: &str,
        attempt: TaskAttemptRef,
        agent_id: &str,
        outcome: AgentDispatchOutcome,
        completion: AgentCompletionSummary,
    ) -> Self {
        let stamp = EventStamp::now();
        Self::AgentCompleted {
            timestamp: stamp.timestamp,
            timestamp_ms: stamp.timestamp_ms,
            run_id: run_id.to_string(),
            attempt,
            agent_id: agent_id.to_string(),
            outcome,
            session_id: completion.session_id,
            total_cost_usd: completion.total_cost_usd,
            turns: completion.turns,
            exit_code: completion.exit_code,
            message: completion.message,
        }
    }

    pub fn gate_dispatch_started(
        run_id: &str,
        attempt: TaskAttemptRef,
        kind: GateCompletionKind,
        rung: u32,
    ) -> Self {
        let stamp = EventStamp::now();
        Self::GateDispatchStarted {
            timestamp: stamp.timestamp,
            timestamp_ms: stamp.timestamp_ms,
            run_id: run_id.to_string(),
            attempt,
            kind,
            rung,
        }
    }

    pub fn gate_completed(
        run_id: &str,
        attempt: TaskAttemptRef,
        completion: &GateCompletion,
    ) -> Self {
        let stamp = EventStamp::now();
        Self::GateCompleted {
            timestamp: stamp.timestamp,
            timestamp_ms: stamp.timestamp_ms,
            run_id: run_id.to_string(),
            attempt,
            kind: completion.kind,
            rung: completion.rung,
            passed: completion.passed,
            failure_kind: completion.failure_kind,
            duration_ms: completion.duration_ms,
            output: completion.output.clone(),
            verdicts: completion.verdicts.clone(),
        }
    }

    pub fn retry_decision(
        run_id: &str,
        attempt: TaskAttemptRef,
        action: RetryAction,
        failure_kind: RunnerFailureKind,
        next_attempt: Option<u32>,
        cooldown_ms: u64,
        reason: String,
    ) -> Self {
        let stamp = EventStamp::now();
        let current_attempt = attempt.attempt;
        Self::RetryDecision {
            timestamp: stamp.timestamp,
            timestamp_ms: stamp.timestamp_ms,
            run_id: run_id.to_string(),
            attempt,
            action,
            failure_kind,
            current_attempt,
            next_attempt,
            cooldown_ms,
            reason,
        }
    }

    pub const fn event_type(&self) -> &'static str {
        match self {
            Self::ResumeMarker { .. } => "resume.marker",
            Self::RunStarted { .. } => "run.started",
            Self::RunCompleted { .. } => "run.completed",
            Self::PlanStarted { .. } => "plan.started",
            Self::PlanCompleted { .. } => "plan.completed",
            Self::TaskAttemptStarted { .. } => "task.attempt.started",
            Self::TaskAttemptCompleted { .. } => "task.attempt.completed",
            Self::AgentDispatchStarted { .. } => "agent.dispatch.started",
            Self::AgentDispatchCompleted { .. } => "agent.dispatch.completed",
            Self::AgentCompleted { .. } => "agent.completed",
            Self::GateDispatchStarted { .. } => "gate.dispatch.started",
            Self::GateCompleted { .. } => "gate.completed",
            Self::RetryDecision { .. } => "retry.decision",
        }
    }

    pub const fn timestamp_ms(&self) -> u64 {
        match self {
            Self::ResumeMarker { timestamp_ms, .. }
            | Self::RunStarted { timestamp_ms, .. }
            | Self::RunCompleted { timestamp_ms, .. }
            | Self::PlanStarted { timestamp_ms, .. }
            | Self::PlanCompleted { timestamp_ms, .. }
            | Self::TaskAttemptStarted { timestamp_ms, .. }
            | Self::TaskAttemptCompleted { timestamp_ms, .. }
            | Self::AgentDispatchStarted { timestamp_ms, .. }
            | Self::AgentDispatchCompleted { timestamp_ms, .. }
            | Self::AgentCompleted { timestamp_ms, .. }
            | Self::GateDispatchStarted { timestamp_ms, .. }
            | Self::GateCompleted { timestamp_ms, .. }
            | Self::RetryDecision { timestamp_ms, .. } => *timestamp_ms,
        }
    }

    pub fn plan_id(&self) -> Option<&str> {
        match self {
            Self::PlanStarted { plan_id, .. } | Self::PlanCompleted { plan_id, .. } => {
                Some(plan_id)
            }
            Self::TaskAttemptStarted { attempt, .. }
            | Self::TaskAttemptCompleted { attempt, .. }
            | Self::AgentDispatchStarted { attempt, .. }
            | Self::AgentDispatchCompleted { attempt, .. }
            | Self::AgentCompleted { attempt, .. }
            | Self::GateDispatchStarted { attempt, .. }
            | Self::GateCompleted { attempt, .. }
            | Self::RetryDecision { attempt, .. } => Some(&attempt.plan_id),
            Self::ResumeMarker { .. } | Self::RunStarted { .. } | Self::RunCompleted { .. } => None,
        }
    }

    pub fn task_id(&self) -> Option<&str> {
        match self {
            Self::TaskAttemptStarted { attempt, .. }
            | Self::TaskAttemptCompleted { attempt, .. }
            | Self::AgentDispatchStarted { attempt, .. }
            | Self::AgentDispatchCompleted { attempt, .. }
            | Self::AgentCompleted { attempt, .. }
            | Self::GateDispatchStarted { attempt, .. }
            | Self::GateCompleted { attempt, .. }
            | Self::RetryDecision { attempt, .. } => Some(&attempt.task_id),
            _ => None,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::ResumeMarker { marker, .. } => marker
                .message
                .clone()
                .unwrap_or_else(|| format!("resume outcome: {:?}", marker.outcome)),
            Self::RunStarted { total_tasks, .. } => format!("run started with {total_tasks} tasks"),
            Self::RunCompleted { outcome, .. } => format!("run completed: {outcome:?}"),
            Self::PlanStarted { plan_id, .. } => format!("plan started: {plan_id}"),
            Self::PlanCompleted {
                outcome, reason, ..
            } => reason
                .clone()
                .unwrap_or_else(|| format!("plan completed: {outcome:?}")),
            Self::TaskAttemptStarted { attempt, .. } => {
                format!("task attempt {} started", attempt.attempt)
            }
            Self::TaskAttemptCompleted { outcome, .. } => {
                format!("task attempt completed: {outcome:?}")
            }
            Self::AgentDispatchStarted {
                agent_id,
                requested_model,
                ..
            } => format!("agent dispatch started: {agent_id} ({requested_model})"),
            Self::AgentDispatchCompleted {
                outcome, message, ..
            } => message
                .clone()
                .unwrap_or_else(|| format!("agent dispatch completed: {outcome:?}")),
            Self::AgentCompleted {
                outcome, message, ..
            } => message
                .clone()
                .unwrap_or_else(|| format!("agent completed: {outcome:?}")),
            Self::GateDispatchStarted { kind, rung, .. } => {
                format!("gate dispatch started: {kind:?} rung {rung}")
            }
            Self::GateCompleted {
                passed,
                failure_kind,
                ..
            } => format!("gate completed: passed={passed} failure_kind={failure_kind:?}"),
            Self::RetryDecision {
                action,
                failure_kind,
                reason,
                ..
            } => format!("retry decision: {action:?} after {failure_kind:?}: {reason}"),
        }
    }
}

/// Aggregate counters used to emit `run.completed`.
#[derive(Debug, Clone, Copy)]
pub struct RunTotals {
    pub total_tasks: usize,
    pub tasks_completed: usize,
    pub tasks_failed: usize,
    pub total_agent_calls: usize,
    pub total_cost_usd: f64,
    pub duration_ms: u64,
}

/// Agent completion payload used by the runtime event constructors.
#[derive(Debug, Clone, Default)]
pub struct AgentCompletionSummary {
    pub session_id: Option<String>,
    pub total_cost_usd: Option<f64>,
    pub turns: Option<u32>,
    pub exit_code: Option<i32>,
    pub message: Option<String>,
}

struct EventStamp {
    timestamp: String,
    timestamp_ms: u64,
}

impl EventStamp {
    fn now() -> Self {
        let now = chrono::Utc::now();
        Self {
            timestamp: now.to_rfc3339(),
            timestamp_ms: now.timestamp_millis().max(0) as u64,
        }
    }
}

fn new_run_id() -> String {
    let now = chrono::Utc::now();
    format!("run-{}", now.timestamp_millis().max(0))
}

// ─── Run Config ─────────────────────────────────────────────────────────

/// Configuration for a runner v2 execution.
#[derive(Debug, Clone)]
pub struct RunConfig {
    /// Working directory for the plan execution.
    pub workdir: PathBuf,
    /// Directory containing plan(s).
    pub plan_dir: PathBuf,
    /// Default model to use when task has no model_hint.
    pub model: String,
    /// Per-task timeout in seconds.
    pub timeout_secs: u64,
    /// Maximum auto-fix retries per task.
    pub max_retries: u32,
    /// Whether to require approval before each task.
    pub approval: bool,
    /// Whether to dangerously skip permissions in the agent.
    pub dangerously_skip_permissions: bool,
    /// Optional MCP config path.
    pub mcp_config: Option<PathBuf>,
    /// Optional session ID to resume from.
    pub resume_session: Option<String>,
    /// Maximum gate rung to run (0=compile, 1=clippy, 2=test, ...).
    pub max_gate_rung: u32,
    /// Claude CLI binary path.
    pub claude_program: PathBuf,
    /// Maximum USD spend per plan (0 = unlimited). From `[budget]`.
    pub max_plan_usd: f64,
    /// Maximum USD spend per single agent turn (0 = unlimited). From `[budget]`.
    pub max_turn_usd: f64,
    /// Whether clippy gate is enabled. From `[gates]` / gate config.
    pub clippy_enabled: bool,
    /// Whether to skip the test gate. From `[gates]` / gate config.
    pub skip_tests: bool,
    /// Effective project config used for provider/model dispatch resolution.
    pub roko_config: Option<Arc<RokoConfig>>,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            workdir: PathBuf::from("."),
            plan_dir: PathBuf::from("plans"),
            model: "claude-sonnet-4-6".to_string(),
            timeout_secs: 600,
            max_retries: 5,
            approval: false,
            dangerously_skip_permissions: true,
            mcp_config: None,
            resume_session: None,
            max_gate_rung: 2,
            claude_program: PathBuf::from("claude"),
            max_plan_usd: 25.0,
            max_turn_usd: 3.0,
            clippy_enabled: true,
            skip_tests: false,
            roko_config: None,
        }
    }
}

// ─── Claude Stream JSON Protocol ────────────────────────────────────────

/// Top-level stream event from `claude --output-format stream-json`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClaudeStreamEvent {
    System(ClaudeSystemEvent),
    Assistant(ClaudeAssistantEvent),
    Tool(ClaudeToolEvent),
    Result(ClaudeResultEvent),
}

/// The `system` init event.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeSystemEvent {
    #[serde(default)]
    pub subtype: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub tools: Vec<serde_json::Value>,
    // mcp_servers, cwd, etc. — we ignore them
}

/// An assistant message event.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeAssistantEvent {
    #[serde(default)]
    pub subtype: String,
    pub message: ClaudeMessage,
}

/// The message body inside an assistant event.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeMessage {
    #[serde(default)]
    pub content: Vec<ClaudeContentBlock>,
    #[serde(default)]
    pub usage: Option<ClaudeUsage>,
}

/// Content block — either text or tool_use.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClaudeContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
}

/// A tool result event.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeToolEvent {
    #[serde(default)]
    pub subtype: String,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub content: String,
}

/// The final result event.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeResultEvent {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub total_cost_usd: Option<f64>,
    #[serde(default)]
    pub num_turns: Option<u32>,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub duration_ms: Option<f64>,
    #[serde(default)]
    pub duration_api_ms: Option<f64>,
    /// Final cumulative usage for the session.
    #[serde(default)]
    pub usage: Option<ClaudeUsage>,
}

/// Token usage from a message.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_event_type_is_normalized() {
        let event = AgentEvent::Started {
            agent_id: "plan/task".to_string(),
            provider: "codex-cli".to_string(),
            model: "gpt-5".to_string(),
            pid: Some(42),
        };

        assert_eq!(event.event_type(), "agent.started");
    }

    #[test]
    fn failure_kind_classifies_retry_policy() {
        let resource = RunnerFailureKind::from_output("fatal: out of memory");
        assert_eq!(resource, RunnerFailureKind::Resource);
        assert!(!resource.is_retryable());

        let transient = RunnerFailureKind::from_output("test timed out intermittently");
        assert_eq!(transient, RunnerFailureKind::Transient);
        assert!(transient.is_retryable());
        assert_eq!(transient.retry_cooldown_secs(), 2);

        let permanent = RunnerFailureKind::from_output("error[E0308]: mismatched types");
        assert_eq!(permanent, RunnerFailureKind::Permanent);
        assert!(permanent.is_retryable());
    }
}
