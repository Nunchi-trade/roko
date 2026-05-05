//! Core types for the runner v2 event-driven plan executor.
//!
//! These types form the protocol between the agent stream parser, the event
//! loop, and the TUI bridge.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use roko_core::config::TimeoutConfig;
use roko_core::config::schema::RokoConfig;
use roko_core::defaults::{DEFAULT_MAX_AUTO_FIX_ITERATIONS, DEFAULT_PLAN_TIMEOUT_SECS};
use roko_fs::RokoLayout;

// ─── Agent Events ───────────────────────────────────────────────────────

// Events emitted by provider runtime adapters. Re-exporting the canonical
// runtime event keeps runner code away from provider-specific stream schemas.
pub use roko_agent::AgentRuntimeEvent as AgentEvent;

// ─── Verify Completion ────────────────────────────────────────────────────

/// Which runtime effect produced a gate-like completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateCompletionKind {
    /// Gate ladder rung after implementation/autofix.
    Gate,
    /// Plan-level task verify commands after all gates pass.
    PlanVerify,
    /// Plan merge/finalization plus post-merge regression gate.
    Merge,
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
        matches!(self, Self::Transient | Self::Structural | Self::Unknown)
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

// ─── Stderr Classification ──────────────────────────────────────────────

/// Severity classification for agent stderr lines, applied before persistence
/// so that downstream consumers (TUI, HTTP, query index) can distinguish
/// provider warnings, real errors, and informational/infra noise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StderrSeverity {
    /// Provider/CLI warning that does not abort the task.
    Warning,
    /// Hard error (panic, failure, abort).
    Error,
    /// Informational / infra-level chatter (banners, INFO lines).
    Infra,
}

impl StderrSeverity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Infra => "infra",
        }
    }

    /// Rule-based classification of a stderr line.
    ///
    /// Order matters: explicit "info"/"INFO" markers take precedence over
    /// substrings that happen to live inside an info banner; explicit warning
    /// markers beat error words inside an info banner; otherwise default to
    /// `Error` so we never silently downgrade a hard failure.
    pub fn from_message(message: &str) -> Self {
        let trimmed = message.trim();
        if trimmed.is_empty() {
            return Self::Infra;
        }
        let lower = trimmed.to_ascii_lowercase();

        // Explicit info markers — provider banners, "INFO ..." log lines.
        if message.contains("INFO")
            || lower.starts_with("info ")
            || lower.starts_with("info:")
            || lower.starts_with("[info]")
            || lower.contains(" info ")
            || lower.contains("debug")
            || lower.contains("trace")
        {
            // Promote to Error/Warning if the line *also* clearly indicates one.
            if lower.contains("error") || lower.contains("panic") || lower.contains("failed") {
                return Self::Error;
            }
            if lower.contains("warn") {
                return Self::Warning;
            }
            return Self::Infra;
        }

        if lower.contains("warn") {
            return Self::Warning;
        }
        if lower.contains("error") || lower.contains("failed") || lower.contains("panic") {
            return Self::Error;
        }

        // Default: an unannotated stderr line is treated as an error so we
        // never silently downgrade a real failure.
        Self::Error
    }
}

// ─── Normalized Event Category ──────────────────────────────────────────

/// Provider-agnostic event category emitted by the projection layer.
///
/// Every runtime event flowing through [`projection::Projection`](super::projection::Projection)
/// is mapped to exactly one `EventCategory` so that TUI / HTTP-SSE / non-TUI
/// CLI subscribers can filter, route, and index events without parsing the
/// nested provider-specific payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventCategory {
    /// Run-level lifecycle (`run.started`, `run.completed`, `resume.marker`).
    Run,
    /// Plan-level lifecycle (`plan.started`, `plan.completed`).
    Plan,
    /// Task attempt lifecycle (`task.attempt.started`, `task.attempt.completed`).
    Task,
    /// Resume markers and recovery hints.
    Resume,
    /// Agent process lifecycle (`agent.started`, `agent.dispatch.completed`,
    /// `agent.completed`, `agent.exited`, `agent.error`, `agent.system_init`).
    AgentLifecycle,
    /// Agent textual output (assistant deltas, structured messages).
    AgentMessage,
    /// Agent tool call start/finish.
    AgentTool,
    /// Per-turn token usage.
    Token,
    /// Authoritative cost updates.
    Cost,
    /// Gate dispatch and verdict events.
    Gate,
    /// Prompt assembly diagnostics.
    Prompt,
    /// Merge backend application and conflict evidence.
    Merge,
    /// Retry policy decisions.
    Retry,
    /// Dream / consolidation events.
    Dream,
    /// Fallback for unknown / coerced events.
    Other,
}

impl EventCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Plan => "plan",
            Self::Task => "task",
            Self::Resume => "resume",
            Self::AgentLifecycle => "agent_lifecycle",
            Self::AgentMessage => "agent_message",
            Self::AgentTool => "agent_tool",
            Self::Token => "token",
            Self::Cost => "cost",
            Self::Gate => "gate",
            Self::Prompt => "prompt",
            Self::Merge => "merge",
            Self::Retry => "retry",
            Self::Dream => "dream",
            Self::Other => "other",
        }
    }

    /// Map a `RunnerEvent` to its category. Stable mapping used by the
    /// projection facade and the on-disk event index.
    pub fn from_runner_event(event: &RunnerEvent) -> Self {
        match event {
            RunnerEvent::ResumeMarker { .. } => Self::Resume,
            RunnerEvent::RunStarted { .. } | RunnerEvent::RunCompleted { .. } => Self::Run,
            RunnerEvent::PlanStarted { .. } | RunnerEvent::PlanCompleted { .. } => Self::Plan,
            RunnerEvent::TaskAttemptStarted { .. } | RunnerEvent::TaskAttemptCompleted { .. } => {
                Self::Task
            }
            RunnerEvent::AgentDispatchStarted { .. }
            | RunnerEvent::AgentDispatchCompleted { .. }
            | RunnerEvent::AgentCompleted { .. } => Self::AgentLifecycle,
            RunnerEvent::GateDispatchStarted { .. } | RunnerEvent::GateCompleted { .. } => {
                Self::Gate
            }
            RunnerEvent::PromptAssembled { .. } => Self::Prompt,
            RunnerEvent::MergeBackendCompleted { .. } => Self::Merge,
            RunnerEvent::RetryDecision { .. } => Self::Retry,
        }
    }

    /// Map an `AgentEvent` to its category.
    pub fn from_agent_event(event: &AgentEvent) -> Self {
        match event {
            AgentEvent::Started { .. }
            | AgentEvent::SystemInit { .. }
            | AgentEvent::Error { .. }
            | AgentEvent::Exited { .. } => Self::AgentLifecycle,
            AgentEvent::MessageDelta { .. } => Self::AgentMessage,
            AgentEvent::ToolCall { .. } | AgentEvent::ToolOutput { .. } => Self::AgentTool,
            AgentEvent::TokenUsage { .. } => Self::Token,
            AgentEvent::TurnCompleted { .. } => Self::Cost,
        }
    }

    /// Map a free-form event-type string to its category. Returns
    /// `(category, coerced)` where `coerced = true` indicates the input did
    /// not match any known prefix and was bucketed into `Other`.
    pub fn from_event_type(event_type: &str) -> (Self, bool) {
        let cat = if let Some(rest) = event_type.strip_prefix("agent.") {
            match rest {
                "message_delta" => Self::AgentMessage,
                "tool_call" | "tool_output" => Self::AgentTool,
                "token_usage" => Self::Token,
                "turn_completed" => Self::Cost,
                _ => Self::AgentLifecycle,
            }
        } else if event_type.starts_with("plan.") {
            Self::Plan
        } else if event_type.starts_with("task.") {
            Self::Task
        } else if event_type.starts_with("gate.") {
            Self::Gate
        } else if event_type.starts_with("prompt.") {
            Self::Prompt
        } else if event_type.starts_with("merge.") {
            Self::Merge
        } else if event_type.starts_with("retry.") {
            Self::Retry
        } else if event_type.starts_with("run.") {
            Self::Run
        } else if event_type.starts_with("resume.") {
            Self::Resume
        } else if event_type.starts_with("dream.") {
            Self::Dream
        } else {
            return (Self::Other, true);
        };
        (cat, false)
    }
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
        #[serde(default, skip_serializing_if = "String::is_empty")]
        model: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        provider: String,
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
    #[serde(rename = "prompt.assembled")]
    PromptAssembled {
        timestamp: String,
        timestamp_ms: u64,
        run_id: String,
        #[serde(flatten)]
        attempt: TaskAttemptRef,
        role: String,
        requested_model: String,
        system_prompt_chars: usize,
        user_prompt_chars: usize,
        estimated_tokens: u32,
        included_sections: Vec<String>,
        dropped_sections: Vec<String>,
        knowledge_ids: Vec<String>,
        playbook_ids: Vec<String>,
    },
    #[serde(rename = "merge.backend.completed")]
    MergeBackendCompleted {
        timestamp: String,
        timestamp_ms: u64,
        run_id: String,
        #[serde(flatten)]
        attempt: TaskAttemptRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch_name: Option<String>,
        passed: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure_kind: Option<RunnerFailureKind>,
        duration_ms: u64,
        output: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        conflict_paths: Vec<String>,
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
        model: impl Into<String>,
        provider: impl Into<String>,
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
            model: model.into(),
            provider: provider.into(),
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

    pub fn prompt_assembled(
        run_id: &str,
        attempt: TaskAttemptRef,
        role: &str,
        requested_model: &str,
        system_prompt_chars: usize,
        user_prompt_chars: usize,
        diagnostics: PromptAssemblyDiagnostics,
    ) -> Self {
        let stamp = EventStamp::now();
        Self::PromptAssembled {
            timestamp: stamp.timestamp,
            timestamp_ms: stamp.timestamp_ms,
            run_id: run_id.to_string(),
            attempt,
            role: role.to_string(),
            requested_model: requested_model.to_string(),
            system_prompt_chars,
            user_prompt_chars,
            estimated_tokens: diagnostics.estimated_tokens,
            included_sections: diagnostics.included_sections,
            dropped_sections: diagnostics.dropped_sections,
            knowledge_ids: diagnostics.knowledge_ids,
            playbook_ids: diagnostics.playbook_ids,
        }
    }

    pub fn merge_backend_completed(
        run_id: &str,
        attempt: TaskAttemptRef,
        completion: &GateCompletion,
        branch_name: Option<String>,
        conflict_paths: Vec<String>,
    ) -> Self {
        let stamp = EventStamp::now();
        Self::MergeBackendCompleted {
            timestamp: stamp.timestamp,
            timestamp_ms: stamp.timestamp_ms,
            run_id: run_id.to_string(),
            attempt,
            branch_name,
            passed: completion.passed,
            failure_kind: completion.failure_kind,
            duration_ms: completion.duration_ms,
            output: completion.output.clone(),
            conflict_paths,
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
            Self::PromptAssembled { .. } => "prompt.assembled",
            Self::MergeBackendCompleted { .. } => "merge.backend.completed",
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
            | Self::PromptAssembled { timestamp_ms, .. }
            | Self::MergeBackendCompleted { timestamp_ms, .. }
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
            | Self::PromptAssembled { attempt, .. }
            | Self::MergeBackendCompleted { attempt, .. }
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
            | Self::PromptAssembled { attempt, .. }
            | Self::MergeBackendCompleted { attempt, .. }
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
            Self::PromptAssembled {
                estimated_tokens,
                included_sections,
                dropped_sections,
                ..
            } => format!(
                "prompt assembled: estimated_tokens={estimated_tokens} included={} dropped={}",
                included_sections.len(),
                dropped_sections.len()
            ),
            Self::MergeBackendCompleted {
                passed,
                failure_kind,
                conflict_paths,
                ..
            } => format!(
                "merge backend completed: passed={passed} failure_kind={failure_kind:?} conflicts={}",
                conflict_paths.len()
            ),
            Self::RetryDecision {
                action,
                failure_kind,
                reason,
                ..
            } => format!("retry decision: {action:?} after {failure_kind:?}: {reason}"),
        }
    }
}

/// Stable prompt assembly diagnostics persisted by `prompt.assembled`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptAssemblyDiagnostics {
    pub included_sections: Vec<String>,
    pub dropped_sections: Vec<String>,
    pub estimated_tokens: u32,
    pub knowledge_ids: Vec<String>,
    pub playbook_ids: Vec<String>,
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
#[derive(Clone)]
pub struct RunConfig {
    /// Typed layout for `.roko/` paths under [`Self::workdir`].
    pub layout: RokoLayout,
    /// Working directory for the plan execution.
    pub workdir: PathBuf,
    /// Directory containing plan(s).
    pub plan_dir: PathBuf,
    /// Default model to use when task has no model_hint.
    pub model: String,
    /// Hard CLI model override. Beats task model hints when present.
    pub cli_model_override: Option<String>,
    /// Per-task timeout in seconds.
    pub timeout_secs: u64,
    /// Wall-clock timeout for the entire plan execution.
    pub plan_timeout_secs: u64,
    /// Maximum auto-fix retries per task.
    pub max_retries: u32,
    /// Maximum number of tasks that may execute concurrently within a plan.
    pub max_concurrent_tasks: usize,
    /// Maximum number of gate rungs that may run concurrently across all tasks.
    pub gate_concurrency: usize,
    /// Whether to require approval before each task.
    pub approval: bool,
    /// Whether to dangerously skip permissions in the agent.
    pub dangerously_skip_permissions: bool,
    /// When true, resume skips task-drift validation and reuses the snapshot state as-is.
    pub force_resume: bool,
    /// Optional MCP config path.
    pub mcp_config: Option<PathBuf>,
    /// Optional session ID to resume from.
    pub resume_session: Option<String>,
    /// Maximum gate rung to run (0=compile, 1=clippy, 2=test, ...).
    pub max_gate_rung: u32,
    /// Default CLI binary path for legacy CLI-provider fallback.
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
    /// Extension chain for lifecycle hooks (init, pre/post inference, gate, error, shutdown).
    pub extension_chain: Option<Arc<tokio::sync::Mutex<roko_core::extension::ExtensionChain>>>,
    /// Learned model selection router (persists across runs).
    pub cascade_router: Option<Arc<roko_learn::cascade_router::CascadeRouter>>,
    /// Optional Daimon affect/somatic state used by runner v2 dispatch hooks.
    /// `None` preserves smoke-test/default behavior without affect modulation.
    pub daimon_state: Option<Arc<Mutex<roko_daimon::DaimonState>>>,
    /// MCP connector tracking registry.
    pub connector_registry: Option<Arc<std::sync::Mutex<roko_core::ConnectorRegistry>>>,
    /// Agent feed tracking registry.
    pub feed_registry: Option<Arc<std::sync::Mutex<roko_core::FeedRegistry>>>,
    /// Single feedback facade — receives every runner event and fans it
    /// out to the registered learning / knowledge / conductor / dream
    /// sinks. `None` means feedback is suppressed (tests, smoke runs).
    pub feedback_facade: Option<Arc<crate::runtime_feedback::FeedbackFacade>>,
    /// Projection facade — receives every runner / agent event and
    /// re-emits it as a normalized `ProjectionEvent` for TUI / HTTP / CLI
    /// subscribers. `None` means events are not mirrored.
    pub projection: Option<Arc<super::projection::Projection>>,
    /// Optional non-blocking HTTP sink for forwarding canonical RuntimeEvents
    /// to a running `roko serve` process.
    pub http_event_sink: Option<roko_runtime::HttpEventSink>,
    /// When true, print real-time agent and task lifecycle events to
    /// stderr instead of showing a spinner. Enabled in non-quiet,
    /// non-json, non-approval CLI mode.
    pub stream_to_stderr: bool,
    /// When true, run `cargo check --workspace` before the main event loop
    /// to warm the incremental cache. Makes subsequent compile gates fast.
    /// Default: true.
    pub warm_cache: bool,
}

impl RunConfig {
    /// Load the default Daimon state for a workdir.
    #[must_use]
    pub fn daimon_state_for_workdir(
        workdir: impl AsRef<Path>,
    ) -> Arc<Mutex<roko_daimon::DaimonState>> {
        Arc::new(Mutex::new(roko_daimon::DaimonState::load_or_new(
            crate::config_helpers::daimon_state_path(workdir.as_ref()),
        )))
    }

    /// Load Daimon state and apply the configured strategy-space definition.
    #[must_use]
    pub fn daimon_state_with_strategy(
        workdir: impl AsRef<Path>,
        strategy_space: roko_daimon::StrategySpaceDefinition,
    ) -> Arc<Mutex<roko_daimon::DaimonState>> {
        let mut state = roko_daimon::DaimonState::load_or_new(
            crate::config_helpers::daimon_state_path(workdir.as_ref()),
        );
        state.configure_strategy_space(strategy_space);
        Arc::new(Mutex::new(state))
    }

    /// Build a runner-v2 config from the effective project config.
    #[must_use]
    pub fn from_roko_config(workdir: PathBuf, plan_dir: PathBuf, roko_config: RokoConfig) -> Self {
        let model = if roko_config.agent.default_model.trim().is_empty() {
            "claude-sonnet-4-6".to_string()
        } else {
            roko_config.agent.default_model.clone()
        };

        let layout = RokoLayout::for_project(&workdir);
        let router_path = layout.cascade_router_path();
        let mut model_slugs = roko_config
            .effective_models()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        if model_slugs.is_empty() {
            model_slugs.push(model.clone());
        }
        model_slugs.sort();
        model_slugs.dedup();

        let cascade_router = Arc::new(roko_learn::cascade_router::CascadeRouter::load_or_new(
            &router_path,
            model_slugs,
        ));
        let mut ext_chain = roko_core::extension::ExtensionChain::new();
        let ext_names = &roko_config.agent.extensions;
        let ext_count =
            super::extension_loader::load_extensions(&workdir, ext_names, &mut ext_chain);
        if ext_count > 0 {
            tracing::info!(count = ext_count, "loaded plugin extensions into chain");
        }
        let extension_chain = Arc::new(tokio::sync::Mutex::new(ext_chain));
        let connector_registry =
            Arc::new(std::sync::Mutex::new(roko_core::ConnectorRegistry::new()));
        let feed_registry = Arc::new(std::sync::Mutex::new(roko_core::FeedRegistry::new()));
        let max_concurrent_tasks = roko_config.runner.max_concurrent_tasks.unwrap_or(4).max(1);
        let timeout_secs = roko_config.timeouts.agent_dispatch().as_secs().max(1);
        let plan_timeout_secs = roko_config.timeouts.plan_total().as_secs().max(1);
        let daimon_state = Self::daimon_state_for_workdir(&workdir);

        Self {
            layout,
            workdir,
            plan_dir,
            model,
            cli_model_override: None,
            timeout_secs,
            plan_timeout_secs,
            max_retries: 2,
            max_concurrent_tasks,
            gate_concurrency: max_concurrent_tasks,
            approval: false,
            dangerously_skip_permissions: roko_config.runner.dangerously_skip_permissions,
            force_resume: false,
            mcp_config: None,
            resume_session: None,
            max_gate_rung: if roko_config.gates.skip_tests {
                u32::from(roko_config.gates.clippy_enabled)
            } else {
                2
            },
            claude_program: roko_config
                .agent
                .command
                .clone()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("claude")),
            max_plan_usd: f64::from(roko_config.budget.max_plan_usd),
            max_turn_usd: f64::from(roko_config.budget.max_turn_usd),
            clippy_enabled: roko_config.gates.clippy_enabled,
            skip_tests: roko_config.gates.skip_tests,
            roko_config: Some(Arc::new(roko_config)),
            extension_chain: Some(extension_chain),
            cascade_router: Some(cascade_router),
            daimon_state: Some(daimon_state),
            connector_registry: Some(connector_registry),
            feed_registry: Some(feed_registry),
            stream_to_stderr: false,
            warm_cache: true,
            // The runner constructs feedback / projection facades at run
            // start (`event_loop::run`) so they share their lifetime
            // with the run id. `None` here is the safe default for
            // callers that build a `RunConfig` directly without going
            // through the full runner setup (tests, integration shims).
            feedback_facade: None,
            projection: None,
            http_event_sink: None,
        }
    }
}

impl Default for RunConfig {
    fn default() -> Self {
        let timeouts = TimeoutConfig::default();
        Self {
            layout: RokoLayout::for_project("."),
            workdir: PathBuf::from("."),
            plan_dir: PathBuf::from("plans"),
            model: "claude-sonnet-4-6".to_string(),
            cli_model_override: None,
            timeout_secs: timeouts.agent_dispatch().as_secs(),
            plan_timeout_secs: timeouts.plan_total().as_secs(),
            max_retries: DEFAULT_MAX_AUTO_FIX_ITERATIONS,
            max_concurrent_tasks: 4,
            gate_concurrency: 4,
            approval: false,
            dangerously_skip_permissions: true,
            force_resume: false,
            mcp_config: None,
            resume_session: None,
            max_gate_rung: 2,
            claude_program: PathBuf::from("claude"),
            max_plan_usd: 25.0,
            max_turn_usd: 3.0,
            clippy_enabled: true,
            skip_tests: false,
            roko_config: None,
            extension_chain: None,
            cascade_router: None,
            daimon_state: None,
            connector_registry: None,
            feed_registry: None,
            feedback_facade: None,
            projection: None,
            http_event_sink: None,
            stream_to_stderr: false,
            warm_cache: true,
        }
    }
}

impl std::fmt::Debug for RunConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunConfig")
            .field("layout", &self.layout)
            .field("workdir", &self.workdir)
            .field("plan_dir", &self.plan_dir)
            .field("model", &self.model)
            .field(
                "cli_model_override",
                &self.cli_model_override.as_ref().map(|_| ".."),
            )
            .field("timeout_secs", &self.timeout_secs)
            .field("plan_timeout_secs", &self.plan_timeout_secs)
            .field("max_retries", &self.max_retries)
            .field("max_concurrent_tasks", &self.max_concurrent_tasks)
            .field("max_gate_rung", &self.max_gate_rung)
            .field("max_plan_usd", &self.max_plan_usd)
            .field("max_turn_usd", &self.max_turn_usd)
            .field("force_resume", &self.force_resume)
            .field(
                "extension_chain",
                &self.extension_chain.as_ref().map(|_| ".."),
            )
            .field(
                "cascade_router",
                &self.cascade_router.as_ref().map(|_| ".."),
            )
            .field("daimon_state", &self.daimon_state.as_ref().map(|_| ".."))
            .field(
                "connector_registry",
                &self.connector_registry.as_ref().map(|_| ".."),
            )
            .field("feed_registry", &self.feed_registry.as_ref().map(|_| ".."))
            .field(
                "http_event_sink",
                &self.http_event_sink.as_ref().map(|_| ".."),
            )
            .field("stream_to_stderr", &self.stream_to_stderr)
            .field("warm_cache", &self.warm_cache)
            .finish()
    }
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
    fn stderr_severity_classifies_warning() {
        assert_eq!(
            StderrSeverity::from_message("warning: deprecated flag --foo"),
            StderrSeverity::Warning
        );
    }

    #[test]
    fn stderr_severity_classifies_error() {
        assert_eq!(
            StderrSeverity::from_message("thread 'main' panicked at 'bad'"),
            StderrSeverity::Error
        );
        assert_eq!(
            StderrSeverity::from_message("error: bad happened"),
            StderrSeverity::Error
        );
        assert_eq!(
            StderrSeverity::from_message("plain stderr line"),
            StderrSeverity::Error,
            "unannotated stderr should default to Error"
        );
    }

    #[test]
    fn stderr_severity_classifies_infra() {
        assert_eq!(
            StderrSeverity::from_message("INFO ready to dispatch"),
            StderrSeverity::Infra
        );
        assert_eq!(
            StderrSeverity::from_message(""),
            StderrSeverity::Infra,
            "empty stderr is infra"
        );
        // Info banner that *also* contains an error word is promoted to Error.
        assert_eq!(
            StderrSeverity::from_message("INFO error during retry"),
            StderrSeverity::Error
        );
    }

    #[test]
    fn event_category_runner_mapping() {
        let event = RunnerEvent::plan_started("run-1", "plan-a");
        assert_eq!(
            EventCategory::from_runner_event(&event),
            EventCategory::Plan
        );

        let event = RunnerEvent::resume_marker(
            "run-1",
            ResumeMarker {
                outcome: ResumeOutcome::Fresh,
                snapshot_path: String::new(),
                snapshot_plan_ids: Vec::new(),
                current_plan_ids: Vec::new(),
                message: None,
            },
        );
        assert_eq!(
            EventCategory::from_runner_event(&event),
            EventCategory::Resume
        );
    }

    #[test]
    fn event_category_agent_mapping() {
        let token = AgentEvent::TokenUsage {
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        };
        assert_eq!(
            EventCategory::from_agent_event(&token),
            EventCategory::Token
        );

        let tool = AgentEvent::ToolCall {
            id: "1".into(),
            name: "Read".into(),
        };
        assert_eq!(
            EventCategory::from_agent_event(&tool),
            EventCategory::AgentTool
        );
    }

    #[test]
    fn event_category_string_coercion() {
        let (cat, coerced) = EventCategory::from_event_type("agent.message_delta");
        assert_eq!(cat, EventCategory::AgentMessage);
        assert!(!coerced);

        let (cat, coerced) = EventCategory::from_event_type("custom.weird.thing");
        assert_eq!(cat, EventCategory::Other);
        assert!(coerced);
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
        assert!(!permanent.is_retryable());
    }

    #[test]
    fn run_config_uses_timeout_config_from_roko_toml() {
        let roko_config = RokoConfig::from_toml(
            r#"
            [timeouts]
            agent_dispatch_secs = 30
            plan_total_secs = 77
            "#,
        )
        .expect("parse roko.toml");

        let config = RunConfig::from_roko_config(
            PathBuf::from("/tmp/work"),
            PathBuf::from("/tmp/plan"),
            roko_config,
        );

        assert_eq!(config.timeout_secs, 30);
        assert_eq!(config.plan_timeout_secs, 77);
    }
}
