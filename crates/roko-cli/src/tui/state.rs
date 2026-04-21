//! Full TUI state container matching Mori's `RunState`.
//!
//! `TuiState` holds every piece of mutable state the interactive dashboard
//! needs: navigation, scroll positions, modal visibility, agent/plan data,
//! cost tracking, git state, and more.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use chrono::{DateTime, Utc};
use ratatui::text::Line;
use roko_core::OperatingFrequency;

use super::atmosphere::Atmosphere;
use super::dashboard::{
    AgentSummary, AlertSummary, CascadeRouterState, DashboardData, EfficiencySummary,
    ExperimentSummary, GateResultSummary, GateResultsPageData, PlanExecutionSnapshot,
    PlanTaskListSnapshot, SignalSummary, TaskSummary, Theme,
};
use super::input::{ConfirmAction, FocusZone, InputMode, LogFilterLevel};
use super::modals::ModalState;
use super::segment::{CachedRender, output_byte_len, render_cached_output};
use super::tabs::Tab;
use crate::plan::{PlanSummary, plans_dir};
use crate::task_parser::{TaskDef, TasksFile};

// ---------------------------------------------------------------------------
// Supporting types
// ---------------------------------------------------------------------------

/// Pending command approval from an agent.
#[derive(Debug, Clone)]
pub struct PendingApproval {
    /// Agent that requested approval.
    pub agent_id: String,
    /// Human-readable description of what the agent wants to do.
    pub description: String,
    /// The raw command or tool call.
    pub command: String,
}

/// Canonical status for an agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AgentStatus {
    Active,
    #[default]
    Idle,
    Done,
    Failed,
}

impl AgentStatus {
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }

    #[must_use]
    pub const fn is_done(self) -> bool {
        matches!(self, Self::Done)
    }

    #[must_use]
    pub const fn is_failed(self) -> bool {
        matches!(self, Self::Failed)
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Idle => "idle",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

impl From<&str> for AgentStatus {
    fn from(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "running" | "active" | "executing" => Self::Active,
            "done" | "completed" | "passed" => Self::Done,
            "failed" | "error" => Self::Failed,
            _ => Self::Idle,
        }
    }
}

impl fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Canonical status for a task.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TaskStatus {
    #[default]
    Pending,
    Active,
    Done,
    Failed,
    Blocked,
}

impl TaskStatus {
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }

    #[must_use]
    pub const fn is_done(self) -> bool {
        matches!(self, Self::Done)
    }

    #[must_use]
    pub const fn is_failed(self) -> bool {
        matches!(self, Self::Failed)
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
        }
    }
}

impl From<&str> for TaskStatus {
    fn from(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "done" | "completed" | "complete" | "passed" | "skipped" => Self::Done,
            "running"
            | "active"
            | "executing"
            | "in_progress"
            | "implementing"
            | "gating"
            | "verifying"
            | "reviewing"
            | "review"
            | "doc revision"
            | "doc-revision"
            | "doc_revision"
            | "auto fixing"
            | "auto-fixing"
            | "auto_fixing"
            | "regenerating verify"
            | "regenerating-verify"
            | "regenerating_verify"
            | "preflight"
            | "strategist"
            | "implementer"
            | "compile-gate"
            | "compile_gate"
            | "test-gate"
            | "test_gate"
            | "critic-review"
            | "critic_review"
            | "verdict"
            | "committing"
            | "merge"
            | "merging"
            | "commit" => Self::Active,
            "failed" | "error" | "gate_rejected" | "gate-rejected" => Self::Failed,
            "blocked" => Self::Blocked,
            _ => Self::Pending,
        }
    }
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Canonical phase state for a plan or pipeline phase.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlanPhase {
    #[default]
    Pending,
    Active,
    Done,
    Failed,
}

impl PlanPhase {
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }

    #[must_use]
    pub const fn is_done(self) -> bool {
        matches!(self, Self::Done)
    }

    #[must_use]
    pub const fn is_failed(self) -> bool {
        matches!(self, Self::Failed)
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

impl From<&str> for PlanPhase {
    fn from(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "done" | "completed" | "complete" | "passed" | "skipped" => Self::Done,
            "failed" | "error" => Self::Failed,
            "pending" | "queued" | "" => Self::Pending,
            "running" | "active" | "executing" | "preflight" | "strategist" | "implementer"
            | "compile-gate" | "compile_gate" | "test-gate" | "test_gate" | "reviewing"
            | "critic-review" | "critic_review" | "verdict" | "committing" | "implementing"
            | "gating" | "verifying" | "review" | "merge" | "merging" | "commit" => Self::Active,
            _ => Self::Pending,
        }
    }
}

impl fmt::Display for PlanPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Fetch status for the agent-topology panel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AgentTopologyStatus {
    /// No fetch has been attempted yet in this session.
    #[default]
    Idle,
    /// A one-shot fetch is in flight.
    Loading,
    /// Topology data is available for rendering.
    Ready,
    /// The connected `roko serve` does not expose the endpoint.
    Unavailable,
    /// The fetch failed for some other reason.
    Error(String),
}

/// Agent row for the Vec-based agent roster used by widgets.
///
/// Widgets index into `TuiState::agents` by position, and read fields
/// like `.active`, `.role`, `.model`, `.current_plan`, `.current_task`,
/// `.context_limit`, `.output_lines`, `.last_output_line` etc.
#[derive(Debug, Clone, Default)]
pub struct AgentRow {
    /// Agent identifier.
    pub id: String,
    /// Whether the agent is currently active / running.
    pub active: bool,
    /// Canonical agent status.
    pub status: AgentStatus,
    /// Role label (e.g. "implementer", "strategist", "auditor").
    pub role: String,
    /// Model slug (e.g. "claude-sonnet-4-20250514").
    pub model: String,
    /// Cumulative input tokens.
    pub input_tokens: u64,
    /// Cumulative output tokens.
    pub output_tokens: u64,
    /// Context window limit in tokens.
    pub context_limit: u64,
    /// Plan this agent is working on.
    pub current_plan: String,
    /// Task this agent is working on.
    pub current_task: String,
    /// Accumulated output lines for the output pane.
    pub output_lines: Vec<String>,
    /// Last line of agent output (for the output pane).
    pub last_output_line: String,
}

const MAX_AGENT_STREAM_CHUNKS: usize = 200;

/// Live websocket-backed tail for one agent.
#[derive(Debug, Clone, Default)]
pub struct AgentStream {
    /// Recent streamed chunks for the Agents tab detail panel.
    pub chunks: VecDeque<String>,
    /// Whether the backing websocket is currently connected.
    pub connected: bool,
    /// Whether the latest observed stream has completed.
    pub completed: bool,
    /// When the most recent chunk arrived.
    pub last_chunk_at: Option<Instant>,
}

/// Per-agent routing and context metrics shown in the TUI.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouteMetrics {
    /// Routed or observed model slug (for example `claude-sonnet-4-5`).
    pub model: String,
    /// Compact routing tier label such as `fast`, `balanced`, or `deep`.
    pub tier: String,
    /// Total tokens consumed for the latest observed turn.
    pub context_used: u64,
    /// Model context window in tokens.
    pub context_limit: u64,
    /// Prompt-focus score in the range `0.0..=1.0`.
    pub focus_score: f64,
}

/// Live system metrics for a supervised agent process.
#[derive(Debug, Clone, Default)]
pub struct ProcessMetrics {
    /// OS process identifier.
    pub pid: u32,
    /// Human-readable process role or label.
    pub role: String,
    /// Current CPU usage percentage.
    pub cpu_pct: f32,
    /// Resident memory in bytes.
    pub mem_bytes: u64,
    /// Compact state label such as `running`, `sleeping`, or `stopped`.
    pub state: String,
    /// Process uptime in seconds.
    pub uptime_secs: f64,
    /// Rolling CPU samples used for inline sparklines.
    pub cpu_history: VecDeque<f32>,
    /// Rolling memory samples used for inline sparklines.
    pub mem_history: VecDeque<u64>,
}

/// Resolve a model slug to its known context window in tokens.
#[must_use]
pub fn model_context_limit(model: &str) -> u64 {
    let model = model.trim().to_ascii_lowercase();
    if model.is_empty() {
        return 200_000;
    }

    if model.contains("gemini") && model.contains("pro") {
        1_000_000
    } else if model.contains("gpt-4o") {
        128_000
    } else if model.contains("claude") {
        200_000
    } else {
        200_000
    }
}

#[must_use]
fn route_tier_label_for_frequency(frequency: OperatingFrequency) -> &'static str {
    match frequency {
        OperatingFrequency::Gamma => "fast",
        OperatingFrequency::Theta => "balanced",
        OperatingFrequency::Delta => "deep",
    }
}

#[must_use]
fn route_tier_label_for_model(model: &str) -> &'static str {
    let lower = model.trim().to_ascii_lowercase();
    if lower.is_empty() {
        "balanced"
    } else if lower.contains("haiku")
        || lower.contains("flash-lite")
        || lower.contains("flash lite")
        || lower.contains("mini")
        || lower.contains("nano")
    {
        "fast"
    } else if lower.contains("opus")
        || lower.contains("pro-preview")
        || lower.contains("pro preview")
        || lower.contains("o1")
        || lower.contains("o3")
        || lower.contains("r1")
    {
        "deep"
    } else {
        "balanced"
    }
}

#[must_use]
fn event_model_slug(event: &roko_learn::efficiency::AgentEfficiencyEvent) -> String {
    if event.model.trim().is_empty() {
        event.model_used.trim().to_string()
    } else {
        event.model.trim().to_string()
    }
}

#[must_use]
fn prompt_focus_score(event: &roko_learn::efficiency::AgentEfficiencyEvent) -> f64 {
    if event.prompt_sections.is_empty() {
        return if event.total_prompt_tokens > 0 {
            1.0
        } else {
            0.0
        };
    }

    let mut max_weighted = 0.0;
    let mut retained_weighted = 0.0;
    for section in &event.prompt_sections {
        let priority_weight = 1.0 / (1.0 + f64::from(section.priority));
        let weighted_tokens = section.tokens as f64 * priority_weight;
        max_weighted += weighted_tokens;

        let retention = if section.was_dropped {
            0.0
        } else if section.was_truncated {
            0.5
        } else {
            1.0
        };
        retained_weighted += weighted_tokens * retention;
    }

    if max_weighted > 0.0 {
        (retained_weighted / max_weighted).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[must_use]
fn route_focus_score(
    event: &roko_learn::efficiency::AgentEfficiencyEvent,
    data: &DashboardData,
    model: &str,
) -> f64 {
    if let Some(stats) = data.cascade_router.confidence_stats.get(model) {
        if stats.trials > 0 {
            return (stats.successes as f64 / stats.trials as f64).clamp(0.0, 1.0);
        }
    }
    prompt_focus_score(event)
}

#[must_use]
fn route_metrics_from_event(
    event: &roko_learn::efficiency::AgentEfficiencyEvent,
    data: &DashboardData,
) -> RouteMetrics {
    let model = event_model_slug(event);
    let context_limit = model_context_limit(&model);
    let focus_score = route_focus_score(event, data, &model);
    RouteMetrics {
        tier: if model.is_empty() {
            route_tier_label_for_frequency(event.frequency).to_string()
        } else {
            route_tier_label_for_model(&model).to_string()
        },
        model,
        context_used: event.total_tokens(),
        context_limit,
        focus_score,
    }
}

#[must_use]
fn fallback_route_metrics_for_agent(agent: &AgentRow) -> RouteMetrics {
    let context_limit = agent
        .context_limit
        .max(model_context_limit(agent.model.as_str()));
    RouteMetrics {
        model: agent.model.clone(),
        tier: route_tier_label_for_model(&agent.model).to_string(),
        context_used: agent.input_tokens.saturating_add(agent.output_tokens),
        context_limit,
        focus_score: 0.0,
    }
}

/// A plan entry in the plan list.
///
/// Extended with fields required by the plan_tree, header_bar, status_bar,
/// and wave_progress widgets.
#[derive(Debug, Clone, Default)]
pub struct PlanEntry {
    pub id: String,
    pub name: String,
    pub status: PlanPhase,
    /// Whether the plan is currently executing.
    pub active: bool,
    /// Current phase label (e.g. "implementing", "done", "failed").
    pub phase: String,
    /// Total task count.
    pub tasks_total: usize,
    /// Completed task count.
    pub tasks_done: usize,
    /// Failed task count.
    pub tasks_failed: usize,
    /// Elapsed wall-clock seconds.
    pub elapsed_secs: f64,
    /// Wave index this plan belongs to, if any.
    pub wave: Option<usize>,
    /// Whether the plan tree node is expanded.
    pub expanded: bool,
    /// Nested task entries (for plan detail view).
    pub tasks: Vec<TaskEntry>,
}

/// A task within a plan entry.
#[derive(Debug, Clone, Default)]
pub struct TaskEntry {
    pub id: String,
    pub name: String,
    pub status: TaskStatus,
    pub agent_id: Option<String>,
}

/// Git branch tree node.
#[derive(Debug, Clone, Default)]
pub struct GitBranchNode {
    /// Branch name.
    pub name: String,
    /// Whether this branch is currently checked out.
    pub is_current: bool,
    /// Upstream tracking branch, if configured.
    pub tracking: Option<String>,
    /// Number of commits ahead of the upstream branch.
    pub ahead: usize,
    /// Number of commits behind the upstream branch.
    pub behind: usize,
    /// Display indentation depth derived from the branch path.
    pub depth: u16,
    /// Nested child branches when rendered hierarchically.
    pub children: Vec<GitBranchNode>,
}

/// Git commit graph entry.
#[derive(Debug, Clone, Default)]
pub struct GitCommitEntry {
    pub hash: String,
    pub short_hash: String,
    pub message: String,
    pub author: String,
    pub timestamp_ms: i64,
    pub branch: Option<String>,
}

/// Severity level for a unified TUI log entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogEntryLevel {
    /// Debug-level event.
    Debug,
    /// Informational event.
    Info,
    /// Warning event.
    Warn,
    /// Error event.
    Error,
}

impl LogEntryLevel {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Debug => "DBG",
            Self::Info => "INF",
            Self::Warn => "WRN",
            Self::Error => "ERR",
        }
    }

    #[must_use]
    pub const fn filter_level(self) -> LogFilterLevel {
        match self {
            Self::Info => LogFilterLevel::Info,
            Self::Warn => LogFilterLevel::Warn,
            Self::Error => LogFilterLevel::Error,
            Self::Debug => LogFilterLevel::Debug,
        }
    }
}

/// A parsed, display-ready log row for the Logs tab.
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// Human-readable timestamp for the rendered row.
    pub timestamp: String,
    /// Severity level used for filtering and styling.
    pub level: LogEntryLevel,
    /// Compact source label such as `signal:gate`.
    pub source: String,
    /// Main message body shown in the Logs tab.
    pub message: String,
}

impl LogEntry {
    /// Construct a display-ready log entry.
    #[must_use]
    pub fn new(timestamp: String, level: LogEntryLevel, source: String, message: String) -> Self {
        Self {
            timestamp,
            level,
            source,
            message,
        }
    }
}

// ---------------------------------------------------------------------------
// Phase pipeline types (for phase_compact widget)
// ---------------------------------------------------------------------------

/// A single step in the phase pipeline.
#[derive(Debug, Clone, Default)]
pub struct PhaseStep {
    /// Phase name (e.g. "preflight", "implementer", "compile-gate").
    pub name: String,
    /// Current status of this phase.
    pub status: PlanPhase,
    /// Elapsed seconds in this phase.
    pub elapsed_secs: f64,
    /// Completion percentage (0.0 .. 100.0).
    pub pct: f64,
}

/// Status of a phase pipeline step.
pub type PhaseStatus = PlanPhase;

// ---------------------------------------------------------------------------
// Execution waves (for plan_tree, wave_progress, header_bar widgets)
// ---------------------------------------------------------------------------

/// An execution wave grouping plans for parallel execution.
#[derive(Debug, Clone, Default)]
pub struct Wave {
    /// Zero-based wave index.
    pub index: usize,
    /// Plan IDs in this wave.
    pub plans: Vec<String>,
    /// Number of completed plans in this wave.
    pub done: usize,
    /// Total plans in this wave.
    pub total: usize,
    /// Whether the wave tree node is expanded.
    pub expanded: bool,
}

// ---------------------------------------------------------------------------
// Task checklist (for task_progress widget)
// ---------------------------------------------------------------------------

/// Status of a task row in the checklist widget.
pub type TaskRowStatus = TaskStatus;

/// A row in the task checklist widget.
#[derive(Debug, Clone, Default)]
pub struct TaskRow {
    /// Task identifier.
    pub id: String,
    /// Human-readable task title.
    pub title: String,
    /// Task status.
    pub status: TaskStatus,
    /// Elapsed seconds for this task.
    pub elapsed_secs: f64,
}

// ---------------------------------------------------------------------------
// System metrics (for sys_metrics, header_bar widgets)
// ---------------------------------------------------------------------------

/// A single gate result for the command_output widget.
#[derive(Debug, Clone, Default)]
pub struct GateResultEntry {
    /// Gate name (e.g. "compile", "clippy", "test").
    pub gate: String,
    /// Plan ID this gate ran against.
    pub plan_id: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Gate output text (stdout + stderr).
    pub output: String,
}

impl From<&GateResultSummary> for GateResultEntry {
    fn from(value: &GateResultSummary) -> Self {
        Self {
            gate: value.gate_name.clone(),
            plan_id: value.plan_id.clone(),
            passed: value.passed,
            output: value.summary.clone(),
        }
    }
}

/// System resource metrics snapshot.
#[derive(Debug, Clone, Default)]
pub struct SysMetrics {
    /// CPU usage percentage (0.0 .. 100.0).
    pub cpu_pct: f32,
    /// Recent CPU usage history for sparkline.
    pub cpu_history: Vec<f32>,
    /// Memory currently used in bytes.
    pub mem_used_bytes: u64,
    /// Total system memory in bytes.
    pub mem_total_bytes: u64,
    /// Recent memory usage history (fractional, 0.0..1.0) for sparkline.
    pub mem_history: Vec<f32>,
    /// Network bytes received (cumulative total from OS).
    pub net_down_bytes_sec: u64,
    /// Network bytes sent (cumulative total from OS).
    pub net_out_bytes_total: u64,
    /// Disk bytes read (cumulative total from OS).
    pub disk_read_bytes_sec: u64,
    /// Disk bytes written (cumulative total from OS).
    pub disk_write_bytes_total: u64,
    /// Previous network in total (for rate calculation).
    pub prev_net_in: u64,
    /// Previous disk read total (for rate calculation).
    pub prev_disk_read: u64,
}

#[derive(Debug, Clone)]
struct SmoothedValue {
    current: f64,
    alpha: f64,
}

impl SmoothedValue {
    fn new(alpha: f64) -> Self {
        Self {
            current: 0.0,
            alpha,
        }
    }

    fn update(&mut self, sample: f64) -> f64 {
        self.current = self.alpha * sample + (1.0 - self.alpha) * self.current;
        self.current
    }
}

// ---------------------------------------------------------------------------
// TuiState
// ---------------------------------------------------------------------------

/// Complete TUI state, matching Mori's `RunState` field set.
#[derive(Debug, Clone)]
pub struct TuiState {
    // -- core orchestrator state --
    /// Serialized orchestrator state label (e.g. "running", "paused").
    pub orchestrator_state: String,
    /// Plan entries with nested tasks.
    pub plans: Vec<PlanEntry>,
    /// Index of the currently selected plan in the plan list.
    pub current_plan_idx: usize,
    /// Current iteration number.
    pub current_iteration: usize,
    /// Current phase label.
    pub current_phase: String,

    // -- phase pipeline --
    /// Ordered phase steps for the phase_compact widget.
    pub phase_pipeline: Vec<PhaseStep>,

    // -- execution waves --
    /// Execution waves grouping plans for parallel execution.
    pub execution_waves: Vec<Wave>,

    // -- task checklist --
    /// Task rows for the task_progress widget.
    pub current_task_checklist: Vec<TaskRow>,

    // -- gate results --
    /// Gate pipeline results for the command_output widget.
    pub gate_results: Vec<GateResultEntry>,
    /// Recent conductor diagnoses from the live dashboard snapshot.
    pub diagnoses: Vec<roko_core::dashboard_snapshot::DiagnosisSummary>,
    /// Concluded prompt experiment winners for the Learning tab.
    pub experiment_winners: Vec<roko_core::ExperimentWinnerSummary>,
    /// Rolling per-gate pass/fail trends from the live verdict reader.
    pub gate_trends: HashMap<String, roko_core::TrendBuckets>,
    /// Recent failing verdicts surfaced beside the trend grid.
    pub gate_recent_failures: Vec<roko_core::FailureEntry>,

    // -- agents (Vec-based roster for widgets) --
    /// Ordered agent roster for widgets (agent_pool, agent_output, header_bar).
    pub agents: Vec<AgentRow>,
    /// Latest fetched agent-topology payload.
    pub agent_topology: roko_core::AgentTopology,
    /// Fetch status for the agent-topology panel.
    pub agent_topology_status: AgentTopologyStatus,
    /// Per-agent route and context metrics keyed by agent identifier.
    pub route_metrics: HashMap<String, RouteMetrics>,
    /// Cached styled agent output keyed by agent identifier.
    pub agent_output_cache: RefCell<HashMap<String, CachedRender>>,
    /// Live websocket tails keyed by agent identifier.
    pub agent_streams: HashMap<String, AgentStream>,
    // -- navigation --
    /// Active top-level tab.
    pub active_tab: Tab,
    /// Selected plan index (legacy, may differ from current_plan_idx during browsing).
    pub selected_plan_idx: usize,
    /// Selected agent index in the agent roster.
    pub selected_agent: usize,
    /// Selected agent sub-tab index.
    pub selected_agent_tab: usize,
    /// Which panel has keyboard focus.
    pub focus: FocusZone,

    // -- animation --
    /// Atmosphere animation state for breathing/heartbeat/spinners.
    pub atmosphere: Atmosphere,

    // -- input --
    /// Current input mode (normal, inject, filter, confirm).
    pub input_mode: InputMode,
    /// Text buffer for inject mode.
    pub message_input: String,
    /// Text buffer for filter mode.
    pub filter_text: String,
    /// Whether filter is actively applied.
    pub filter_active: bool,
    /// Filter alias (mirrors filter_text for widget compatibility).
    pub filter: String,

    // -- scroll positions --
    /// Agent output scroll. `None` means auto-tail (follow latest output).
    pub agent_scroll: Option<usize>,
    /// Diff panel scroll offset.
    pub diff_scroll: usize,
    /// Task list scroll offset.
    pub task_scroll: usize,
    /// Command output panel scroll offset.
    pub command_output_scroll: usize,
    /// Plan detail overlay scroll offset.
    pub plan_detail_scroll: usize,
    /// Plan list scroll offset (for long plan lists).
    pub plan_scroll_offset: usize,
    /// Log viewer scroll offset.
    pub log_scroll: usize,
    /// Whether the agent-topology overlay is visible.
    pub agent_topology_visible: bool,
    /// Agent-topology overlay scroll offset.
    pub agent_topology_scroll_offset: usize,
    /// Whether the log viewer is following the tail.
    pub log_auto_tail: bool,
    /// Active log levels shown in the Logs tab.
    pub log_filter_levels: HashSet<LogFilterLevel>,

    // -- approval / confirm --
    /// Pending agent command approval, if any.
    pub pending_approval: Option<PendingApproval>,
    /// Pending confirmation dialog action, if any.
    pub pending_confirm: Option<ConfirmAction>,
    /// Active modal overlay, if any.
    pub active_modal: Option<ModalState>,

    // -- git --
    /// Current git branch name.
    pub git_branch: String,
    /// Short commit hash for the status bar.
    pub git_commit_short: String,
    /// Human-readable commit age for the status bar (e.g. "2m ago").
    pub git_age: String,
    /// Git branch tree for the Git tab.
    pub git_branch_tree: Vec<GitBranchNode>,
    /// Git commit graph entries.
    pub git_commit_graph: Vec<GitCommitEntry>,
    /// Git worktree list entries.
    pub git_worktree_list: Vec<String>,
    /// Cursor position in the git branch tree.
    pub git_branch_cursor: usize,
    /// Cached git summary lines for the dashboard sub-tab (populated by background thread).
    pub git_summary_lines: Vec<String>,
    /// Cached full git view data for F4 Git tab (populated by background thread).
    pub(crate) git_view_data: Option<super::views::git_view::GitViewData>,

    // -- plan detail --
    /// Active sub-tab in the plan detail overlay.
    pub plan_detail_tab: usize,

    // -- pipeline --
    /// Whether pipeline execution is currently paused.
    pub is_paused: bool,

    // -- cost / tokens --
    /// Cost per plan (plan_id -> USD).
    pub cost_per_plan: HashMap<String, f64>,
    /// Cost per task (task_id -> USD).
    pub cost_per_task: HashMap<String, f64>,
    /// Cumulative input tokens across all agents.
    pub cumulative_input_tokens: u64,
    /// Cumulative output tokens across all agents.
    pub cumulative_output_tokens: u64,
    /// Total token count (input + output) for header_bar / token_sparkline.
    pub token_total: u64,
    /// Rolling per-agent cumulative token totals, bounded to recent samples.
    pub token_history: HashMap<String, VecDeque<u64>>,
    /// Current token burn rate (tokens per minute) for token_sparkline.
    pub token_rate: f64,
    /// Current cost burn rate (USD per minute).
    pub cost_rate: f64,
    /// Cumulative cost in USD for header_bar display.
    pub cost_dollars: f64,
    /// Per-process metrics for the dashboard Procs sub-tab.
    pub process_metrics: Vec<ProcessMetrics>,

    // -- system metrics --
    /// System resource metrics snapshot.
    pub sys: SysMetrics,

    // -- timing --
    /// When the current run started, for elapsed time calculation.
    pub run_started: Option<Instant>,

    // -- wave navigation --
    /// Selected wave index for wave prev/next navigation.
    pub selected_wave_idx: usize,

    // -- config editor --
    /// Cursor index into the flat config item list.
    pub config_cursor: usize,
    /// Unsaved edits: config key -> new value string.
    pub config_pending: HashMap<String, String>,
    /// Config panel scroll offset.
    pub config_scroll_offset: usize,
    /// Whether text-input mode is active for a config field.
    pub config_editing: bool,
    /// Text input buffer for the field being edited.
    pub config_edit_buffer: String,
    /// Which config key is currently being text-edited.
    pub config_edit_key: Option<String>,

    // -- agent pane --
    /// Active agent pane display group (cycles through available groups).
    pub agent_pane_group: usize,

    // -- push-path state (from DashboardSnapshot) --
    /// Orchestrator event log entries.
    pub event_log: Vec<roko_core::DashboardEventLogEntry>,
    /// Cascade router state as opaque JSON.
    pub cascade_router_json: String,
    /// Adaptive gate thresholds as opaque JSON.
    pub gate_thresholds_json: String,

    // -- view data (migrated from DashboardData) --
    /// Workspace root path for config file loading.
    pub workdir: PathBuf,
    /// Efficiency summary stats.
    pub efficiency_summary: EfficiencySummary,
    /// Raw efficiency events for per-agent metrics and aggregation.
    pub efficiency_events: Vec<roko_learn::efficiency::AgentEfficiencyEvent>,
    /// Efficiency trend buckets for charts.
    pub efficiency_trend: Vec<roko_learn::aggregate::EfficiencyBucket>,
    /// C-factor trend buckets for charts.
    pub cfactor_trend_buckets: Vec<roko_learn::aggregate::CFactorBucket>,
    /// Cascade router state for model routing display.
    pub cascade_router: CascadeRouterState,
    /// Recent signals for the logs tab.
    pub recent_signals: Vec<SignalSummary>,
    /// Current plan execution snapshot for the plan detail view.
    pub current_plan_execution: Option<PlanExecutionSnapshot>,
    /// Conductor alerts for the inspect tab.
    pub conductor_alerts: Vec<AlertSummary>,
    /// C-factor snapshot for the inspect tab.
    pub cfactor: Option<roko_learn::cfactor::CFactor>,
    /// Gate results page data (gate_rows, failure_rows, threshold_rows).
    pub gate_results_page: GateResultsPageData,
    /// Experiment summaries for the config tab.
    pub experiments: Vec<ExperimentSummary>,
    /// Per-task output tails.
    pub task_output_tails: HashMap<String, Vec<String>>,
    /// Git diff content.
    pub git_diff: String,
    /// Plan summaries (legacy format from DashboardData).
    pub plan_summaries: Vec<PlanSummary>,
    /// Agent summaries (legacy format from DashboardData).
    pub agent_summaries: Vec<AgentSummary>,
    /// Active task summaries (legacy format from DashboardData).
    pub active_task_summaries: Vec<TaskSummary>,
    /// Gate result summaries (legacy format from DashboardData).
    pub gate_result_summaries: Vec<GateResultSummary>,
    /// Cached episodes for the logs tab.
    pub episodes_cache: Vec<roko_learn::episode_logger::Episode>,

    cpu_pct_smoothed: SmoothedValue,
    token_rate_smoothed: SmoothedValue,
    cost_rate_smoothed: SmoothedValue,
    last_rate_sample_at: Option<Instant>,
    last_token_total_sample: u64,
    last_cost_dollars_sample: f64,
}

impl Default for TuiState {
    fn default() -> Self {
        const METRIC_EMA_ALPHA: f64 = 0.25;

        Self {
            orchestrator_state: String::from("idle"),
            plans: Vec::new(),
            current_plan_idx: 0,
            current_iteration: 0,
            current_phase: String::new(),

            phase_pipeline: Vec::new(),
            execution_waves: Vec::new(),
            current_task_checklist: Vec::new(),
            gate_results: Vec::new(),
            diagnoses: Vec::new(),
            experiment_winners: Vec::new(),
            gate_trends: HashMap::new(),
            gate_recent_failures: Vec::new(),

            agents: Vec::new(),
            agent_topology: roko_core::AgentTopology::default(),
            agent_topology_status: AgentTopologyStatus::Idle,
            route_metrics: HashMap::new(),
            agent_output_cache: RefCell::new(HashMap::new()),
            agent_streams: HashMap::new(),
            active_tab: Tab::default(),
            selected_plan_idx: 0,
            selected_agent: 0,
            selected_agent_tab: 0,
            focus: FocusZone::default(),

            atmosphere: Atmosphere::default(),

            input_mode: InputMode::default(),
            message_input: String::new(),
            filter_text: String::new(),
            filter_active: false,
            filter: String::new(),

            agent_scroll: None,
            diff_scroll: 0,
            task_scroll: 0,
            command_output_scroll: 0,
            plan_detail_scroll: 0,
            plan_scroll_offset: 0,
            log_scroll: 0,
            agent_topology_visible: false,
            agent_topology_scroll_offset: 0,
            log_auto_tail: true,
            log_filter_levels: LogFilterLevel::all().into_iter().collect(),

            pending_approval: None,
            pending_confirm: None,
            active_modal: None,

            git_branch: String::new(),
            git_commit_short: String::new(),
            git_age: String::new(),
            git_branch_tree: Vec::new(),
            git_commit_graph: Vec::new(),
            git_worktree_list: Vec::new(),
            git_branch_cursor: 0,
            git_summary_lines: Vec::new(),
            git_view_data: None,

            plan_detail_tab: 0,

            is_paused: false,

            cost_per_plan: HashMap::new(),
            cost_per_task: HashMap::new(),
            cumulative_input_tokens: 0,
            cumulative_output_tokens: 0,
            token_total: 0,
            token_history: HashMap::new(),
            token_rate: 0.0,
            cost_rate: 0.0,
            cost_dollars: 0.0,
            process_metrics: Vec::new(),

            sys: SysMetrics::default(),

            run_started: None,

            selected_wave_idx: 0,

            config_cursor: 0,
            config_scroll_offset: 0,
            config_pending: HashMap::new(),
            config_editing: false,
            config_edit_buffer: String::new(),
            config_edit_key: None,

            agent_pane_group: 0,

            event_log: Vec::new(),
            cascade_router_json: String::new(),
            gate_thresholds_json: String::new(),

            workdir: PathBuf::new(),
            efficiency_summary: EfficiencySummary::default(),
            efficiency_events: Vec::new(),
            efficiency_trend: Vec::new(),
            cfactor_trend_buckets: Vec::new(),
            cascade_router: CascadeRouterState::default(),
            recent_signals: Vec::new(),
            current_plan_execution: None,
            conductor_alerts: Vec::new(),
            cfactor: None,
            gate_results_page: GateResultsPageData::default(),
            experiments: Vec::new(),
            task_output_tails: HashMap::new(),
            git_diff: String::new(),
            plan_summaries: Vec::new(),
            agent_summaries: Vec::new(),
            active_task_summaries: Vec::new(),
            gate_result_summaries: Vec::new(),
            episodes_cache: Vec::new(),

            cpu_pct_smoothed: SmoothedValue::new(METRIC_EMA_ALPHA),
            token_rate_smoothed: SmoothedValue::new(METRIC_EMA_ALPHA),
            cost_rate_smoothed: SmoothedValue::new(METRIC_EMA_ALPHA),
            last_rate_sample_at: None,
            last_token_total_sample: 0,
            last_cost_dollars_sample: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Canonical phase names for the phase pipeline
// ---------------------------------------------------------------------------

/// The canonical phase names used by the orchestrator pipeline.
const CANONICAL_PHASES: &[&str] = &[
    "preflight",
    "strategist",
    "implementer",
    "compile-gate",
    "test-gate",
    "reviewing",
    "critic-review",
    "verdict",
    "committing",
];

impl TuiState {
    /// Create a new default state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a `TuiState` from a `DashboardData` snapshot.
    ///
    /// This is the primary constructor used by widget tests and the TUI app
    /// to bootstrap a full state from the snapshot data model.
    #[must_use]
    pub fn from_dashboard_data(data: &DashboardData) -> Self {
        let mut state = Self::default();
        state.update_from_snapshot(data);
        state
    }

    // -- aggregate queries (used by header_bar, status_bar, etc.) -----------

    /// Return (done, total) task counts summed across all plans.
    #[must_use]
    pub fn task_counts(&self) -> (usize, usize) {
        let total: usize = self.plans.iter().map(|p| p.tasks_total).sum();
        let done: usize = self.plans.iter().map(|p| p.tasks_done).sum();
        (done, total)
    }

    /// Elapsed seconds since `run_started`, or 0.0 if not set.
    #[must_use]
    pub fn elapsed_secs(&self) -> f64 {
        self.run_started
            .map(|s| s.elapsed().as_secs_f64())
            .unwrap_or(0.0)
    }

    /// Number of execution waves.
    #[must_use]
    pub fn wave_count(&self) -> usize {
        self.execution_waves.len()
    }

    /// Index of the currently selected wave.
    #[must_use]
    pub fn current_wave(&self) -> usize {
        self.selected_wave_idx
    }

    /// Count of agents with status "active" or "running".
    #[must_use]
    pub fn active_agent_count(&self) -> usize {
        self.agents.iter().filter(|a| a.active).count()
    }

    /// Return the current filter text.
    #[must_use]
    pub fn filter_ref(&self) -> &str {
        &self.filter
    }

    /// Return the display label for the active text input mode.
    #[must_use]
    pub const fn input_mode_label(&self) -> &'static str {
        match self.input_mode {
            InputMode::Normal => "",
            InputMode::Inject => "INJECT",
            InputMode::Filter => "FILTER",
            InputMode::Confirm | InputMode::ConfigEdit => "",
        }
    }

    pub fn update_cpu_pct(&mut self, sample: f32) -> f32 {
        let smoothed = self.cpu_pct_smoothed.update(sample as f64) as f32;
        self.sys.cpu_pct = smoothed;
        smoothed
    }

    // -- snapshot bridging ---------------------------------------------------

    /// Populate state from a `DashboardData` snapshot.
    ///
    /// This bridges the existing snapshot-based data model into the full
    /// TuiState. Fields not covered by `DashboardData` are left unchanged.
    pub fn update_from_snapshot(&mut self, data: &DashboardData) {
        let executor_summary = data.executor_summary();
        if executor_summary.orchestrator_state.is_empty() {
            if self.orchestrator_state.is_empty() {
                self.orchestrator_state = String::from("idle");
            }
        } else {
            self.orchestrator_state = executor_summary.orchestrator_state;
        }
        self.current_iteration = executor_summary.current_iteration;
        self.current_phase = executor_summary.current_phase;
        let latest_events = latest_agent_events(&data.efficiency_events);
        let latest_route_metrics = latest_route_metrics(&data.efficiency_events, data);
        self.experiment_winners = data.experiment_winners.clone();
        self.gate_trends.clear();
        self.gate_recent_failures.clear();

        let mut tasks_by_plan: HashMap<String, Vec<TaskEntry>> = HashMap::new();
        for task in &data.active_tasks {
            tasks_by_plan
                .entry(task.plan_id.clone())
                .or_default()
                .push(TaskEntry {
                    id: task.task_id.clone(),
                    name: task.latest_gate.as_ref().map_or_else(
                        || task.task_id.clone(),
                        |gate| format!("{} ({gate})", task.task_id),
                    ),
                    status: TaskStatus::from(task.status.as_str()),
                    agent_id: task.assigned_agents.first().cloned(),
                });
        }

        if let Some(exec) = &data.current_plan_execution {
            let entry = tasks_by_plan.entry(exec.plan_id.clone()).or_default();
            if entry.is_empty() {
                entry.extend(exec.tasks.iter().map(|task| TaskEntry {
                    id: task.task_id.clone(),
                    name: if task.title.is_empty() {
                        task.task_id.clone()
                    } else {
                        task.title.clone()
                    },
                    status: TaskStatus::from(task.phase.as_str()),
                    agent_id: None,
                }));
            }
        }

        // Plans
        let expanded_by_plan: HashMap<String, bool> = self
            .plans
            .iter()
            .map(|plan| (plan.id.clone(), plan.expanded))
            .collect();
        let plan_waves = derive_plan_waves(data.root(), &data.plans);
        let plan_snapshots = data.plan_task_snapshots();
        self.plans = data
            .plans
            .iter()
            .map(|p| {
                let completed = p.completed;
                let snapshot = plan_snapshots.get(&p.id);
                let phase = snapshot.map(|plan| plan.phase.clone()).unwrap_or_else(|| {
                    if completed {
                        String::from("done")
                    } else {
                        String::from("pending")
                    }
                });
                let status = PlanPhase::from(phase.as_str());
                let tasks_total = snapshot
                    .map(|plan| plan.tasks.len())
                    .filter(|count| *count > 0)
                    .unwrap_or(p.task_count);
                let (tasks_done, derived_failed) = plan_task_counts(p, snapshot, tasks_total);
                let tasks_failed = snapshot
                    .map(|plan| {
                        usize::try_from(plan.failed_count)
                            .unwrap_or(tasks_total)
                            .min(tasks_total)
                    })
                    .unwrap_or(derived_failed);
                let elapsed_secs = snapshot
                    .map(|plan| {
                        if plan.elapsed_ms > 0 {
                            plan.elapsed_ms as f64 / 1000.0
                        } else {
                            plan.elapsed_secs
                        }
                    })
                    .unwrap_or(0.0);
                PlanEntry {
                    id: p.id.clone(),
                    name: p.title.clone(),
                    status,
                    active: snapshot.map(|plan| plan.active).unwrap_or(!completed),
                    phase,
                    tasks_total,
                    tasks_done,
                    tasks_failed,
                    elapsed_secs,
                    wave: snapshot
                        .map(|plan| usize::try_from(plan.wave).unwrap_or_default())
                        .or_else(|| plan_waves.get(&p.id).copied()),
                    expanded: expanded_by_plan.get(&p.id).copied().unwrap_or(false),
                    tasks: snapshot
                        .map(|plan| {
                            plan.tasks
                                .iter()
                                .map(|task| TaskEntry {
                                    id: task.id.clone(),
                                    name: task.title.clone(),
                                    status: TaskStatus::from(task.status.as_str()),
                                    agent_id: task.agent_id.clone(),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                }
            })
            .collect();

        // Agents
        self.agents.clear();
        let mut agent_ids = data
            .agents
            .iter()
            .map(|agent| agent.id.clone())
            .collect::<Vec<_>>();
        if agent_ids.is_empty() {
            agent_ids.extend(latest_events.keys().cloned());
            agent_ids.sort();
            agent_ids.dedup();
        }
        for agent_id in agent_ids {
            let summary = data.agents.iter().find(|agent| agent.id == agent_id);
            let latest = latest_events.get(&agent_id);
            let label = summary
                .map(|agent| agent.label.clone())
                .filter(|label| !label.is_empty())
                .or_else(|| latest.map(|event| event.role.clone()))
                .unwrap_or_else(|| agent_id.clone());
            let status = summary
                .map(|agent| agent.status.clone())
                .or_else(|| latest.map(|event| event.status.clone()))
                .unwrap_or_else(|| "idle".to_string());
            let current_plan = summary
                .and_then(|agent| agent.plan_id.clone())
                .or_else(|| latest.and_then(|event| event.plan_id.clone()))
                .unwrap_or_default();
            let current_task = latest
                .map(|event| event.task_id.clone())
                .unwrap_or_default();
            let is_active = plan_is_active(&status);
            self.agents.push(AgentRow {
                id: agent_id.clone(),
                active: is_active,
                status: AgentStatus::from(status.as_str()),
                role: label.clone(),
                model: latest.map(|event| event.model.clone()).unwrap_or_default(),
                input_tokens: latest.map_or(0, |event| event.input_tokens),
                output_tokens: latest.map_or(0, |event| event.output_tokens),
                context_limit: model_context_limit(latest.map(|e| e.model.as_str()).unwrap_or("")),
                current_plan: current_plan.clone(),
                current_task: current_task.clone(),
                output_lines: Vec::new(),
                last_output_line: String::new(),
            });
        }

        // Populate agent output from episodes (Task 2)
        for episode in data.episodes() {
            if let Some(row) = self.agents.iter_mut().find(|a| a.id == episode.agent_id) {
                let output_text = extract_episode_output(episode);
                if !output_text.is_empty() {
                    row.output_lines = output_text.lines().map(String::from).collect();
                }
                if let Some(last_line) = output_text.lines().last() {
                    row.last_output_line = last_line.to_string();
                }
                // Also populate model and task from episode
                if !episode.model.is_empty() {
                    row.model = episode.model.clone();
                    row.context_limit = model_context_limit(&episode.model);
                }
                if !episode.task_id.is_empty() {
                    row.current_task = episode.task_id.clone();
                }
                row.input_tokens = row.input_tokens.max(episode.usage.input_tokens);
                row.output_tokens = row.output_tokens.max(episode.usage.output_tokens);
            }
        }

        // Supplement agent output from task-outputs files (Task 2 continued)
        for (task_id, lines) in data.task_outputs() {
            // Find agent working on this task and add output if empty
            if let Some(row) = self.agents.iter_mut().find(|a| a.current_task == *task_id) {
                if row.output_lines.is_empty() && !lines.is_empty() {
                    row.output_lines = lines.clone();
                }
                if row.last_output_line.is_empty() {
                    if let Some(last) = lines.last() {
                        row.last_output_line = last.clone();
                    }
                }
            }
        }
        self.route_metrics = self
            .agents
            .iter()
            .map(|agent| {
                let metrics = latest_route_metrics
                    .get(&agent.id)
                    .cloned()
                    .map(|mut metrics| {
                        if metrics.model.is_empty() && !agent.model.is_empty() {
                            metrics.model = agent.model.clone();
                        }
                        if metrics.context_limit == 0 {
                            metrics.context_limit = agent.context_limit.max(1);
                        }
                        metrics
                    })
                    .unwrap_or_else(|| fallback_route_metrics_for_agent(agent));
                (agent.id.clone(), metrics)
            })
            .collect();
        self.prune_agent_output_cache();
        self.prune_agent_streams();

        self.cost_dollars = data.efficiency.total_cost_usd;
        self.cumulative_input_tokens = data.efficiency.total_input_tokens;
        self.cumulative_output_tokens = data.efficiency.total_output_tokens;
        self.token_total = self.cumulative_input_tokens + self.cumulative_output_tokens;
        self.update_efficiency_rates();
        self.gate_results = data
            .gate_results
            .iter()
            .map(GateResultEntry::from)
            .collect();
        sum_costs(data, &mut self.cost_per_plan, &mut self.cost_per_task);

        self.phase_pipeline = build_phase_pipeline(&data.active_tasks);

        // Populate phase elapsed times from episodes (Task 7)
        populate_phase_elapsed(&mut self.phase_pipeline, data.episodes());

        // Build current_task_checklist from active_tasks + task-trackers (Task 3)
        self.current_task_checklist = build_task_checklist_from_execution(data);

        self.execution_waves = rebuild_execution_waves(&self.plans, &self.execution_waves);

        // Sync filter alias
        self.filter = self.filter_text.clone();

        // Clamp selections
        if !self.plans.is_empty() {
            if self.selected_plan_idx >= self.plans.len() {
                self.selected_plan_idx = self.plans.len() - 1;
            }
            if self.current_plan_idx >= self.plans.len() {
                self.current_plan_idx = self.plans.len() - 1;
            }
        } else {
            self.selected_plan_idx = 0;
            self.current_plan_idx = 0;
        }

        if !self.agents.is_empty() && self.selected_agent >= self.agents.len() {
            self.selected_agent = self.agents.len() - 1;
        }
        self.selected_agent_tab = self.selected_agent_tab.min(6);
        self.selected_wave_idx =
            clamp_selected_wave_idx(self.selected_wave_idx, self.execution_waves.len());

        let token_samples = build_token_samples(data);
        self.token_history = token_samples
            .iter()
            .map(|(agent_id, samples)| {
                (
                    agent_id.clone(),
                    samples.iter().map(|(_, total)| *total).collect(),
                )
            })
            .collect();
        self.token_rate = self
            .agents
            .get(self.selected_agent)
            .and_then(|agent| token_samples.get(&agent.id))
            .map_or_else(
                || compute_token_rate(&data.efficiency_events),
                compute_windowed_token_rate,
            );

        // -- view data (migrated from DashboardData) --
        self.workdir = data.root().to_path_buf();
        self.efficiency_summary = data.efficiency.clone();
        self.efficiency_events = data.efficiency_events.clone();
        self.efficiency_trend = data.efficiency_trend.clone();
        self.cfactor_trend_buckets = data.cfactor_trend.clone();
        self.cascade_router = data.cascade_router.clone();
        self.recent_signals = data.recent_signals.clone();
        self.current_plan_execution = data.current_plan_execution.clone();
        self.conductor_alerts = data.conductor_alerts.clone();
        self.cfactor = data.cfactor.clone();
        self.gate_results_page = data.gate_results_page.clone();
        self.experiments = data.experiments.clone();
        self.task_output_tails = data.task_outputs().clone();
        self.git_diff = data.git_diff.clone();
        self.plan_summaries = data.plans.clone();
        self.agent_summaries = data.agents.clone();
        self.active_task_summaries = data.active_tasks.clone();
        self.gate_result_summaries = data.gate_results.clone();
        self.episodes_cache = data.episodes().to_vec();
    }

    /// Populate state from a connected-mode `DashboardSnapshot`.
    ///
    /// This mirrors the live state published by `StateHub` without touching
    /// navigation or scroll state.
    pub fn update_from_dashboard_snapshot(&mut self, snap: &roko_core::DashboardSnapshot) {
        let prev_selected_plan_id = self
            .plans
            .get(self.selected_plan_idx)
            .map(|plan| plan.id.clone());
        let prev_current_plan_id = self
            .plans
            .get(self.current_plan_idx)
            .map(|plan| plan.id.clone());
        let prev_selected_agent_id = self
            .agents
            .get(self.selected_agent)
            .map(|agent| agent.id.clone());
        let prev_plan_order: HashMap<String, usize> = self
            .plans
            .iter()
            .enumerate()
            .map(|(idx, plan)| (plan.id.clone(), idx))
            .collect();
        let prev_agent_order: HashMap<String, usize> = self
            .agents
            .iter()
            .enumerate()
            .map(|(idx, agent)| (agent.id.clone(), idx))
            .collect();
        let prev_plan_expanded: HashMap<String, bool> = self
            .plans
            .iter()
            .map(|plan| (plan.id.clone(), plan.expanded))
            .collect();
        let prev_plan_elapsed: HashMap<String, f64> = self
            .plans
            .iter()
            .map(|plan| (plan.id.clone(), plan.elapsed_secs))
            .collect();
        let prev_plan_wave: HashMap<String, Option<usize>> = self
            .plans
            .iter()
            .map(|plan| (plan.id.clone(), plan.wave))
            .collect();
        let prev_task_elapsed: HashMap<String, f64> = self
            .current_task_checklist
            .iter()
            .map(|task| (task.id.clone(), task.elapsed_secs))
            .collect();
        let prev_agent_rows: HashMap<String, AgentRow> = self
            .agents
            .iter()
            .cloned()
            .map(|agent| (agent.id.clone(), agent))
            .collect();
        let prev_route_metrics = self.route_metrics.clone();
        let mut snapshot_tasks: Vec<&roko_core::dashboard_snapshot::TaskState> =
            snap.tasks.values().collect();
        snapshot_tasks.sort_by(|lhs, rhs| {
            lhs.plan_id
                .cmp(&rhs.plan_id)
                .then_with(|| lhs.task_id.cmp(&rhs.task_id))
        });

        let mut tasks_by_plan: HashMap<String, Vec<TaskEntry>> = HashMap::new();
        self.current_task_checklist = snapshot_tasks
            .iter()
            .map(|task| {
                let status = snapshot_task_status(task);
                tasks_by_plan
                    .entry(task.plan_id.clone())
                    .or_default()
                    .push(TaskEntry {
                        id: task.task_id.clone(),
                        name: task.task_id.clone(),
                        status,
                        agent_id: None,
                    });
                TaskRow {
                    id: task.task_id.clone(),
                    title: task.task_id.clone(),
                    status,
                    elapsed_secs: prev_task_elapsed.get(&task.task_id).copied().unwrap_or(0.0),
                }
            })
            .collect();

        let mut plan_ids: Vec<String> = snap.plans.keys().cloned().collect();
        plan_ids.sort_by(|lhs, rhs| {
            prev_plan_order
                .get(lhs)
                .copied()
                .unwrap_or(usize::MAX)
                .cmp(&prev_plan_order.get(rhs).copied().unwrap_or(usize::MAX))
                .then_with(|| lhs.cmp(rhs))
        });

        self.plans = plan_ids
            .iter()
            .map(|plan_id| {
                let plan = &snap.plans[plan_id];
                let tasks = tasks_by_plan.remove(plan_id).unwrap_or_default();
                let tasks_total = plan.tasks_total.max(tasks.len());
                PlanEntry {
                    id: plan.plan_id.clone(),
                    name: plan.plan_id.clone(),
                    status: snapshot_plan_status(plan),
                    active: plan.active,
                    phase: snapshot_plan_phase(plan),
                    tasks_total,
                    tasks_done: plan.tasks_done.min(tasks_total),
                    tasks_failed: plan.tasks_failed.min(tasks_total),
                    elapsed_secs: prev_plan_elapsed.get(plan_id).copied().unwrap_or(0.0),
                    wave: prev_plan_wave.get(plan_id).copied().flatten(),
                    expanded: prev_plan_expanded.get(plan_id).copied().unwrap_or(false),
                    tasks,
                }
            })
            .collect();

        let mut orphaned_plan_ids: Vec<String> = tasks_by_plan.keys().cloned().collect();
        orphaned_plan_ids.sort();
        for plan_id in orphaned_plan_ids {
            let tasks = tasks_by_plan.remove(&plan_id).unwrap_or_default();
            let tasks_total = tasks.len();
            let tasks_done = tasks.iter().filter(|task| task.status.is_done()).count();
            let tasks_failed = tasks.iter().filter(|task| task.status.is_failed()).count();
            let active = tasks.iter().any(|task| task.status.is_active());
            self.plans.push(PlanEntry {
                id: plan_id.clone(),
                name: plan_id.clone(),
                status: if active {
                    PlanPhase::Active
                } else if tasks_failed > 0 {
                    PlanPhase::Failed
                } else if tasks_total > 0 && tasks_done == tasks_total {
                    PlanPhase::Done
                } else {
                    PlanPhase::Pending
                },
                active,
                phase: if active {
                    String::from("active")
                } else if tasks_failed > 0 {
                    String::from("failed")
                } else if tasks_total > 0 && tasks_done == tasks_total {
                    String::from("completed")
                } else {
                    String::from("pending")
                },
                tasks_total,
                tasks_done,
                tasks_failed,
                elapsed_secs: prev_plan_elapsed.get(&plan_id).copied().unwrap_or(0.0),
                wave: prev_plan_wave.get(&plan_id).copied().flatten(),
                expanded: prev_plan_expanded.get(&plan_id).copied().unwrap_or(false),
                tasks,
            });
        }

        let mut agent_ids: Vec<String> = snap.agents.keys().cloned().collect();
        agent_ids.sort_by(|lhs, rhs| {
            prev_agent_order
                .get(lhs)
                .copied()
                .unwrap_or(usize::MAX)
                .cmp(&prev_agent_order.get(rhs).copied().unwrap_or(usize::MAX))
                .then_with(|| lhs.cmp(rhs))
        });

        self.agents = agent_ids
            .iter()
            .map(|agent_id| {
                let agent = &snap.agents[agent_id];
                let prev_row = prev_agent_rows.get(agent_id);
                let model = prev_row.map(|row| row.model.clone()).unwrap_or_default();
                let context_limit = prev_row
                    .map(|row| row.context_limit)
                    .filter(|limit| *limit > 0)
                    .unwrap_or_else(|| model_context_limit(&model));

                // Prefer snapshot values for model/tokens/cost/task/plan, fall
                // back to previously cached values.
                let snap_model = if agent.model.is_empty() {
                    model
                } else {
                    agent.model.clone()
                };
                let snap_input_tokens = if agent.input_tokens > 0 {
                    agent.input_tokens
                } else {
                    prev_row.map(|row| row.input_tokens).unwrap_or(0)
                };
                let snap_output_tokens = if agent.output_tokens > 0 {
                    agent.output_tokens
                } else {
                    prev_row
                        .map(|row| row.output_tokens)
                        .unwrap_or(0)
                        .max(agent.output_bytes as u64)
                };
                let snap_current_plan = if agent.current_plan.is_empty() {
                    prev_row
                        .map(|row| row.current_plan.clone())
                        .unwrap_or_default()
                } else {
                    agent.current_plan.clone()
                };
                let snap_current_task = if agent.current_task.is_empty() {
                    prev_row
                        .map(|row| row.current_task.clone())
                        .unwrap_or_default()
                } else {
                    agent.current_task.clone()
                };

                // Merge task output lines from the snapshot ring buffer.
                let mut output_lines = prev_row
                    .map(|row| row.output_lines.clone())
                    .unwrap_or_default();
                if let Some(task_lines) = snap
                    .task_outputs
                    .get(&snap_current_task)
                    .filter(|lines| !lines.is_empty())
                {
                    output_lines.extend(task_lines.iter().cloned());
                }
                let last_output_line = output_lines
                    .last()
                    .cloned()
                    .or_else(|| prev_row.map(|row| row.last_output_line.clone()))
                    .unwrap_or_default();

                AgentRow {
                    id: agent.agent_id.clone(),
                    active: agent.active,
                    status: if agent.active {
                        AgentStatus::Active
                    } else {
                        AgentStatus::Idle
                    },
                    role: agent.role.clone(),
                    model: snap_model,
                    input_tokens: snap_input_tokens,
                    output_tokens: snap_output_tokens,
                    context_limit,
                    current_plan: snap_current_plan,
                    current_task: snap_current_task,
                    output_lines,
                    last_output_line,
                }
            })
            .collect();
        self.route_metrics = self
            .agents
            .iter()
            .map(|agent| {
                let metrics = prev_route_metrics
                    .get(&agent.id)
                    .cloned()
                    .map(|mut metrics| {
                        if metrics.model.is_empty() && !agent.model.is_empty() {
                            metrics.model = agent.model.clone();
                        }
                        metrics.context_used =
                            agent.input_tokens.saturating_add(agent.output_tokens);
                        if metrics.context_limit == 0 {
                            metrics.context_limit = agent.context_limit.max(1);
                        }
                        if metrics.tier.is_empty() {
                            metrics.tier = route_tier_label_for_model(&metrics.model).to_string();
                        }
                        metrics
                    })
                    .unwrap_or_else(|| fallback_route_metrics_for_agent(agent));
                (agent.id.clone(), metrics)
            })
            .collect();
        self.prune_agent_output_cache();
        self.prune_agent_streams();

        self.gate_results = snap
            .gates
            .iter()
            .map(|gate_result| GateResultEntry {
                gate: gate_result.gate.clone(),
                plan_id: gate_result.plan_id.clone(),
                passed: gate_result.passed,
                output: if gate_result.task_id.is_empty() {
                    String::new()
                } else {
                    format!("task {}", gate_result.task_id)
                },
            })
            .collect();
        self.diagnoses = snap.diagnoses.iter().cloned().collect();
        self.experiment_winners = snap.experiment_winners.clone();
        self.gate_trends = snap.gate_trends.clone();
        self.gate_recent_failures = snap.gate_recent_failures.clone();
        if !snap.agent_topology.is_empty() {
            self.agent_topology = snap.agent_topology.clone();
            self.agent_topology_status = AgentTopologyStatus::Ready;
        } else if !matches!(self.agent_topology_status, AgentTopologyStatus::Ready) {
            self.agent_topology = snap.agent_topology.clone();
        }

        // --- Event log from snapshot ---
        self.event_log = snap.event_log.iter().cloned().collect();

        // --- Learning data (opaque JSON for now) ---
        self.cascade_router_json = snap.cascade_router_json.clone();
        self.gate_thresholds_json = snap.gate_thresholds_json.clone();

        // --- Token/cost aggregation across agents ---
        self.cumulative_input_tokens = self.agents.iter().map(|a| a.input_tokens).sum();
        self.cumulative_output_tokens = self.agents.iter().map(|a| a.output_tokens).sum();
        self.token_total = self.cumulative_input_tokens + self.cumulative_output_tokens;
        if snap.stats.cost_usd_total > 0.0 {
            self.cost_dollars = snap.stats.cost_usd_total;
        }

        self.phase_pipeline = build_phase_pipeline_from_dashboard_snapshot(snap);
        self.execution_waves = rebuild_execution_waves(&self.plans, &self.execution_waves);

        self.orchestrator_state = if snap.stats.plans_active > 0 {
            String::from("running")
        } else if snap.stats.plans_failed > 0 {
            String::from("failed")
        } else if self.orchestrator_state.is_empty() {
            String::from("idle")
        } else {
            self.orchestrator_state.clone()
        };
        self.current_phase = self
            .plans
            .iter()
            .find(|plan| plan.active)
            .or_else(|| self.plans.first())
            .map(|plan| plan.phase.clone())
            .unwrap_or_default();
        self.filter = self.filter_text.clone();

        restore_selected_plan_idx(
            &self.plans,
            &mut self.selected_plan_idx,
            prev_selected_plan_id,
        );
        restore_selected_plan_idx(
            &self.plans,
            &mut self.current_plan_idx,
            prev_current_plan_id,
        );
        restore_selected_agent_idx(
            &self.agents,
            &mut self.selected_agent,
            prev_selected_agent_id,
        );
        self.selected_agent_tab = self.selected_agent_tab.min(6);
        self.selected_wave_idx =
            clamp_selected_wave_idx(self.selected_wave_idx, self.execution_waves.len());

        // Task outputs from push path
        for (task_id, lines) in &snap.task_outputs {
            self.task_output_tails
                .insert(task_id.clone(), lines.iter().cloned().collect());
        }
    }

    /// Return cached, styled agent output lines for the selected agent pane.
    #[must_use]
    pub fn render_agent_output_lines(
        &self,
        cache_key: &str,
        raw_output: &[String],
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        if raw_output.is_empty() {
            if !cache_key.is_empty() {
                self.agent_output_cache.borrow_mut().remove(cache_key);
            }
            return Vec::new();
        }

        let cache_key = if cache_key.is_empty() {
            "__agent-output__"
        } else {
            cache_key
        };
        let output_len = output_byte_len(raw_output);
        let mut cache = self.agent_output_cache.borrow_mut();
        let cached = cache
            .entry(cache_key.to_string())
            .or_insert_with(CachedRender::default);

        if cached.last_len != output_len {
            *cached = render_cached_output(raw_output, theme);
        }

        cached.styled_lines.clone()
    }

    /// Append one streamed chunk for the given agent, trimming to the last 200 entries.
    pub fn push_agent_chunk(&mut self, agent_id: &str, chunk: String) {
        let stream = self.agent_streams.entry(agent_id.to_string()).or_default();
        while stream.chunks.len() >= MAX_AGENT_STREAM_CHUNKS {
            stream.chunks.pop_front();
        }
        stream.chunks.push_back(chunk);
        stream.connected = true;
        stream.completed = false;
        stream.last_chunk_at = Some(Instant::now());
    }

    /// Mark the agent's live stream as connected.
    pub fn mark_agent_stream_connected(&mut self, agent_id: &str) {
        let stream = self.agent_streams.entry(agent_id.to_string()).or_default();
        stream.connected = true;
    }

    /// Mark the agent's live stream as disconnected.
    pub fn mark_agent_stream_disconnected(&mut self, agent_id: &str) {
        let stream = self.agent_streams.entry(agent_id.to_string()).or_default();
        stream.connected = false;
    }

    /// Mark the agent's live stream as complete.
    pub fn mark_agent_stream_done(&mut self, agent_id: &str) {
        let stream = self.agent_streams.entry(agent_id.to_string()).or_default();
        stream.connected = false;
        stream.completed = true;
    }

    fn update_efficiency_rates(&mut self) {
        let now = Instant::now();
        let token_total = self.token_total;
        let cost_dollars = self.cost_dollars;

        if token_total < self.last_token_total_sample
            || cost_dollars < self.last_cost_dollars_sample
        {
            const METRIC_EMA_ALPHA: f64 = 0.25;

            self.token_rate = 0.0;
            self.cost_rate = 0.0;
            self.token_rate_smoothed = SmoothedValue::new(METRIC_EMA_ALPHA);
            self.cost_rate_smoothed = SmoothedValue::new(METRIC_EMA_ALPHA);
        } else if let Some(last_sample_at) = self.last_rate_sample_at {
            let elapsed_secs = now.duration_since(last_sample_at).as_secs_f64();
            if elapsed_secs > 0.0 {
                let token_delta = token_total.saturating_sub(self.last_token_total_sample) as f64;
                let cost_delta = (cost_dollars - self.last_cost_dollars_sample).max(0.0);

                self.token_rate = self
                    .token_rate_smoothed
                    .update(token_delta * 60.0 / elapsed_secs);
                self.cost_rate = self
                    .cost_rate_smoothed
                    .update(cost_delta * 60.0 / elapsed_secs);
            }
        }

        self.last_rate_sample_at = Some(now);
        self.last_token_total_sample = token_total;
        self.last_cost_dollars_sample = cost_dollars;
    }

    /// Whether the state is in a text-input mode (inject or filter).
    #[must_use]
    pub const fn is_text_input(&self) -> bool {
        matches!(self.input_mode, InputMode::Inject | InputMode::Filter)
    }

    /// Reset all scroll positions to zero.
    pub fn reset_scrolls(&mut self) {
        self.agent_scroll = None;
        self.diff_scroll = 0;
        self.task_scroll = 0;
        self.command_output_scroll = 0;
        self.plan_detail_scroll = 0;
        self.plan_scroll_offset = 0;
        self.log_scroll = 0;
        self.agent_topology_scroll_offset = 0;
        self.log_auto_tail = true;
    }

    /// Clamp the plan tree scroll offset to the current rendered maximum.
    pub fn clamp_plan_scroll(&mut self, max: usize) {
        self.plan_scroll_offset = self.plan_scroll_offset.min(max);
    }

    /// Clamp the pinned agent-output scroll offset to the current rendered maximum.
    pub fn clamp_agent_scroll(&mut self, max: usize) {
        if let Some(scroll) = self.agent_scroll.as_mut() {
            *scroll = (*scroll).min(max);
        }
    }

    /// Clamp the log scroll offset to the current rendered maximum.
    pub fn clamp_log_scroll(&mut self, max: usize) {
        if self.log_auto_tail {
            self.log_scroll = 0;
        } else {
            self.log_scroll = self.log_scroll.min(max);
        }
    }

    /// Toggle visibility for the agent-topology overlay.
    pub fn toggle_agent_topology(&mut self) {
        self.agent_topology_visible = !self.agent_topology_visible;
    }

    /// Close the agent-topology overlay.
    pub fn close_agent_topology(&mut self) {
        self.agent_topology_visible = false;
    }

    /// Clamp the agent-topology scroll offset to the current rendered maximum.
    pub fn clamp_agent_topology_scroll(&mut self, max: usize) {
        self.agent_topology_scroll_offset = self.agent_topology_scroll_offset.min(max);
    }

    /// Mark the agent-topology panel as loading.
    pub fn set_agent_topology_loading(&mut self) {
        self.agent_topology_status = AgentTopologyStatus::Loading;
    }

    /// Store the latest fetched agent-topology payload.
    pub fn set_agent_topology(&mut self, topology: roko_core::AgentTopology) {
        self.agent_topology = topology;
        self.agent_topology_status = AgentTopologyStatus::Ready;
        self.agent_topology_scroll_offset = 0;
    }

    /// Mark the agent-topology endpoint as unavailable.
    pub fn set_agent_topology_unavailable(&mut self) {
        self.agent_topology = roko_core::AgentTopology::default();
        self.agent_topology_status = AgentTopologyStatus::Unavailable;
        self.agent_topology_scroll_offset = 0;
    }

    /// Record a topology fetch error message.
    pub fn set_agent_topology_error(&mut self, message: impl Into<String>) {
        self.agent_topology = roko_core::AgentTopology::default();
        self.agent_topology_status = AgentTopologyStatus::Error(message.into());
        self.agent_topology_scroll_offset = 0;
    }

    /// Clamp the right-panel scroll offset to the current rendered maximum.
    pub fn clamp_diff_scroll(&mut self, max: usize) {
        self.diff_scroll = self.diff_scroll.min(max);
    }

    /// Clamp the task list scroll offset to the current rendered maximum.
    pub fn clamp_task_scroll(&mut self, max: usize) {
        self.task_scroll = self.task_scroll.min(max);
    }

    /// Clamp the command-output scroll offset to the current rendered maximum.
    pub fn clamp_command_output_scroll(&mut self, max: usize) {
        self.command_output_scroll = self.command_output_scroll.min(max);
    }

    /// Toggle visibility for a single log level in the Logs tab.
    pub fn toggle_log_filter_level(&mut self, level: LogFilterLevel) {
        if !self.log_filter_levels.insert(level) {
            self.log_filter_levels.remove(&level);
        }
    }

    /// Restore the Logs tab to show all available levels.
    pub fn show_all_log_filter_levels(&mut self) {
        self.log_filter_levels = LogFilterLevel::all().into_iter().collect();
    }

    /// Whether a log level is currently visible in the Logs tab.
    #[must_use]
    pub fn log_level_visible(&self, level: LogFilterLevel) -> bool {
        self.log_filter_levels.contains(&level)
    }

    fn prune_agent_output_cache(&self) {
        let valid_ids = self
            .agents
            .iter()
            .map(|agent| agent.id.as_str())
            .collect::<HashSet<_>>();
        self.agent_output_cache
            .borrow_mut()
            .retain(|key, _| key == "__agent-output__" || valid_ids.contains(key.as_str()));
    }

    fn prune_agent_streams(&mut self) {
        let valid_ids = self
            .agents
            .iter()
            .map(|agent| agent.id.as_str())
            .collect::<HashSet<_>>();
        self.agent_streams
            .retain(|agent_id, _| valid_ids.contains(agent_id.as_str()));
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn plan_task_counts(
    summary: &PlanSummary,
    snapshot: Option<&PlanTaskListSnapshot>,
    tasks_total: usize,
) -> (usize, usize) {
    if let Some(snapshot) = snapshot {
        let tasks_done = if snapshot.tasks.is_empty() {
            snapshot.tasks_done
        } else {
            snapshot
                .tasks
                .iter()
                .filter(|task| TaskStatus::from(task.status.as_str()).is_done())
                .count()
        };
        let tasks_failed = if snapshot.tasks.is_empty() {
            snapshot.tasks_failed
        } else {
            snapshot
                .tasks
                .iter()
                .filter(|task| TaskStatus::from(task.status.as_str()).is_failed())
                .count()
        };
        return (tasks_done.min(tasks_total), tasks_failed.min(tasks_total));
    }

    (
        summary.tasks_done.min(tasks_total),
        summary.tasks_failed.min(tasks_total),
    )
}

fn snapshot_plan_phase(plan: &roko_core::dashboard_snapshot::PlanState) -> String {
    if plan.phase.is_empty() {
        if plan.active {
            String::from("active")
        } else if plan.tasks_failed > 0 {
            String::from("failed")
        } else if plan.tasks_done >= plan.tasks_total && plan.tasks_total > 0 {
            String::from("completed")
        } else {
            String::from("pending")
        }
    } else {
        plan.phase.clone()
    }
}

fn snapshot_plan_status(plan: &roko_core::dashboard_snapshot::PlanState) -> PlanPhase {
    if plan.active {
        PlanPhase::Active
    } else if plan.tasks_failed > 0 {
        PlanPhase::Failed
    } else {
        match snapshot_plan_phase(plan).as_str() {
            "completed" | "done" => PlanPhase::Done,
            "failed" | "error" => PlanPhase::Failed,
            phase => PlanPhase::from(phase),
        }
    }
}

fn snapshot_task_status(task: &roko_core::dashboard_snapshot::TaskState) -> TaskStatus {
    match task.outcome.as_deref() {
        Some(outcome)
            if outcome.contains("fail")
                || outcome.contains("error")
                || outcome.contains("Fail")
                || outcome.contains("Error") =>
        {
            TaskStatus::Failed
        }
        Some(_) => TaskStatus::Done,
        None => TaskStatus::from(task.phase.as_str()),
    }
}

fn canonical_phase_index_for_snapshot_task(
    task: &roko_core::dashboard_snapshot::TaskState,
) -> Option<usize> {
    let phase = task.phase.trim().to_ascii_lowercase();
    if phase.is_empty() {
        return None;
    }

    if matches!(phase.as_str(), "completed" | "done") {
        return Some(CANONICAL_PHASES.len().saturating_sub(1));
    }

    CANONICAL_PHASES.iter().position(|candidate| {
        *candidate == phase
            || (*candidate == "compile-gate" && phase.contains("compile"))
            || (*candidate == "test-gate" && (phase.contains("test") || phase.contains("verif")))
            || (*candidate == "critic-review" && phase.contains("critic"))
    })
}

fn build_phase_pipeline_from_dashboard_snapshot(
    snap: &roko_core::dashboard_snapshot::DashboardSnapshot,
) -> Vec<PhaseStep> {
    #[derive(Clone, Copy, Default)]
    struct PhaseTaskCounts {
        total: usize,
        done: usize,
        active: usize,
        failed: usize,
    }

    let mut counts = vec![PhaseTaskCounts::default(); CANONICAL_PHASES.len()];

    for task in snap.tasks.values() {
        let Some(current_idx) = canonical_phase_index_for_snapshot_task(task) else {
            continue;
        };

        let failed = snapshot_task_status(task).is_failed();
        let done = snapshot_task_status(task).is_done();

        for (phase_idx, phase_counts) in counts.iter_mut().enumerate() {
            phase_counts.total += 1;

            if phase_idx < current_idx {
                phase_counts.done += 1;
            } else if phase_idx == current_idx {
                if failed {
                    phase_counts.failed += 1;
                } else if done {
                    phase_counts.done += 1;
                } else {
                    phase_counts.active += 1;
                }
            }
        }
    }

    CANONICAL_PHASES
        .iter()
        .enumerate()
        .map(|(idx, phase)| {
            let counts = counts[idx];
            let status = if counts.failed > 0 {
                PlanPhase::Failed
            } else if counts.active > 0 {
                PlanPhase::Active
            } else if counts.total > 0 && counts.done == counts.total {
                PlanPhase::Done
            } else {
                PlanPhase::Pending
            };
            let pct = if counts.total == 0 {
                0.0
            } else {
                (counts.done as f64 / counts.total as f64) * 100.0
            };

            PhaseStep {
                name: (*phase).to_string(),
                status,
                elapsed_secs: 0.0,
                pct,
            }
        })
        .collect()
}

fn restore_selected_plan_idx(
    plans: &[PlanEntry],
    selected: &mut usize,
    previous_id: Option<String>,
) {
    match previous_id {
        Some(previous_id) => {
            if let Some(idx) = plans.iter().position(|plan| plan.id == previous_id) {
                *selected = idx;
            } else if plans.is_empty() {
                *selected = 0;
            } else {
                *selected = (*selected).min(plans.len() - 1);
            }
        }
        None if plans.is_empty() => *selected = 0,
        None if *selected >= plans.len() => *selected = plans.len() - 1,
        None => {}
    }
}

fn restore_selected_agent_idx(
    agents: &[AgentRow],
    selected: &mut usize,
    previous_id: Option<String>,
) {
    match previous_id {
        Some(previous_id) => {
            if let Some(idx) = agents.iter().position(|agent| agent.id == previous_id) {
                *selected = idx;
            } else if agents.is_empty() {
                *selected = 0;
            } else {
                *selected = (*selected).min(agents.len() - 1);
            }
        }
        None if agents.is_empty() => *selected = 0,
        None if *selected >= agents.len() => *selected = agents.len() - 1,
        None => {}
    }
}

/// Build the canonical 9-phase pipeline, inferring status from active tasks.
fn build_phase_pipeline(active_tasks: &[super::dashboard::TaskSummary]) -> Vec<PhaseStep> {
    #[derive(Clone, Copy, Default)]
    struct PhaseTaskCounts {
        total: usize,
        done: usize,
        active: usize,
        failed: usize,
    }

    let mut counts = vec![PhaseTaskCounts::default(); CANONICAL_PHASES.len()];

    for task in active_tasks {
        let Some(current_idx) = canonical_phase_index_for_task(task) else {
            continue;
        };

        let failed = task_status_is_failed(&task.status);
        let done = task_status_is_done(&task.status);
        let active = !failed && !done;

        for (phase_idx, phase_counts) in counts.iter_mut().enumerate() {
            phase_counts.total += 1;

            if phase_idx < current_idx {
                phase_counts.done += 1;
                continue;
            }

            if phase_idx == current_idx {
                if failed {
                    phase_counts.failed += 1;
                } else if done {
                    phase_counts.done += 1;
                } else if active {
                    phase_counts.active += 1;
                }
            }
        }
    }

    CANONICAL_PHASES
        .iter()
        .enumerate()
        .map(|(idx, &name)| {
            let phase_counts = counts[idx];
            let pct = if phase_counts.total > 0 {
                (phase_counts.done as f64 / phase_counts.total as f64) * 100.0
            } else {
                0.0
            };
            let status = if phase_counts.failed > 0 {
                PlanPhase::Failed
            } else if phase_counts.total > 0 && phase_counts.done == phase_counts.total {
                PlanPhase::Done
            } else if phase_counts.active > 0 {
                PlanPhase::Active
            } else {
                PlanPhase::Pending
            };

            PhaseStep {
                name: name.to_string(),
                status,
                elapsed_secs: 0.0,
                pct,
            }
        })
        .collect()
}

fn canonical_phase_index_for_task(task: &super::dashboard::TaskSummary) -> Option<usize> {
    let phase_name = canonical_phase_name_for_task(task)?;
    CANONICAL_PHASES.iter().position(|&name| name == phase_name)
}

fn canonical_phase_name_for_task(task: &super::dashboard::TaskSummary) -> Option<&'static str> {
    let status = task.status.to_ascii_lowercase();

    match status.as_str() {
        "preflight" => Some("preflight"),
        "strategist" => Some("strategist"),
        "implementer" => Some("implementer"),
        "compile-gate" | "compile_gate" => Some("compile-gate"),
        "test-gate" | "test_gate" => Some("test-gate"),
        "reviewing" => Some("reviewing"),
        "critic-review" | "critic_review" => Some("critic-review"),
        "verdict" => Some("verdict"),
        "committing" => Some("committing"),
        "queued" => Some("preflight"),
        "enriching" => Some("strategist"),
        "implementing" | "auto-fixing" | "auto_fixing" => Some("implementer"),
        "gating" => Some(classify_gate_phase(task)),
        "verifying" | "regenerating-verify" | "regenerating_verify" => Some("test-gate"),
        "review" => Some("reviewing"),
        "doc-revision" | "doc_revision" => Some("critic-review"),
        "done" | "passed" => Some("verdict"),
        "merging" | "commit" | "complete" | "completed" => Some("committing"),
        "failed" | "error" => Some(classify_failed_phase(task)),
        "running" | "active" | "executing" | "in_progress" => Some(classify_phase_from_hints(task)),
        _ => Some(classify_phase_from_hints(task)),
    }
}

fn classify_gate_phase(task: &super::dashboard::TaskSummary) -> &'static str {
    let latest_gate = task
        .latest_gate
        .as_deref()
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let task_id = task.task_id.to_ascii_lowercase();

    if latest_gate.contains("test")
        || latest_gate.contains("verify")
        || task_id.contains("test")
        || task_id.contains("verify")
    {
        "test-gate"
    } else {
        "compile-gate"
    }
}

fn classify_failed_phase(task: &super::dashboard::TaskSummary) -> &'static str {
    let hint = classify_phase_from_hints(task);
    if hint == "preflight" || hint == "strategist" {
        hint
    } else if task.latest_gate.as_deref().is_some_and(|gate| {
        let gate = gate.to_ascii_lowercase();
        gate.contains("test") || gate.contains("verify")
    }) {
        "test-gate"
    } else if task.latest_gate.is_some() {
        "compile-gate"
    } else {
        hint
    }
}

fn classify_phase_from_hints(task: &super::dashboard::TaskSummary) -> &'static str {
    let task_id = task.task_id.to_ascii_lowercase();
    let assigned_agents = task
        .assigned_agents
        .iter()
        .map(|agent| agent.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    let latest_gate = task
        .latest_gate
        .as_deref()
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    if task_id.contains("preflight") {
        "preflight"
    } else if task_id.contains("strateg") || assigned_agents.contains("strateg") {
        "strategist"
    } else if task_id.contains("critic") || assigned_agents.contains("critic") {
        "critic-review"
    } else if task_id.contains("review") {
        "reviewing"
    } else if task_id.contains("merge")
        || task_id.contains("commit")
        || latest_gate.contains("merge")
    {
        "committing"
    } else if task_id.contains("verdict") {
        "verdict"
    } else if latest_gate.contains("test")
        || latest_gate.contains("verify")
        || task_id.contains("test")
        || task_id.contains("verify")
    {
        "test-gate"
    } else if latest_gate.contains("compile")
        || latest_gate.contains("clippy")
        || task_id.contains("compile")
        || task_id.contains("clippy")
        || task_id.contains("build")
    {
        "compile-gate"
    } else {
        "implementer"
    }
}

fn task_status_is_failed(status: &str) -> bool {
    TaskStatus::from(status).is_failed()
}

fn task_status_is_done(status: &str) -> bool {
    TaskStatus::from(status).is_done()
}

/// Build execution waves from plan entries.
///
/// Groups by `wave` field if set, otherwise places all plans in wave 0.
fn build_execution_waves(plans: &[PlanEntry]) -> Vec<Wave> {
    if plans.is_empty() {
        return Vec::new();
    }

    let has_waves = plans.iter().any(|p| p.wave.is_some());
    if !has_waves {
        let done = plans.iter().filter(|plan| plan_is_complete(plan)).count();
        return vec![Wave {
            index: 0,
            plans: plans.iter().map(|plan| plan.id.clone()).collect(),
            done,
            total: plans.len(),
            expanded: true,
        }];
    }

    let mut wave_map: std::collections::BTreeMap<usize, Vec<&PlanEntry>> =
        std::collections::BTreeMap::new();
    for plan in plans {
        let wave_index = plan.wave.unwrap_or(0);
        wave_map.entry(wave_index).or_default().push(plan);
    }

    wave_map
        .into_iter()
        .map(|(idx, wave_plans)| {
            let done = wave_plans
                .iter()
                .filter(|plan| plan_is_complete(plan))
                .count();
            Wave {
                index: idx,
                plans: wave_plans.iter().map(|plan| plan.id.clone()).collect(),
                done,
                total: wave_plans.len(),
                expanded: true,
            }
        })
        .collect()
}

fn rebuild_execution_waves(plans: &[PlanEntry], previous: &[Wave]) -> Vec<Wave> {
    let prev_wave_expanded: std::collections::HashMap<usize, bool> = previous
        .iter()
        .map(|wave| (wave.index, wave.expanded))
        .collect();

    let mut waves = build_execution_waves(plans);
    for wave in &mut waves {
        if let Some(expanded) = prev_wave_expanded.get(&wave.index).copied() {
            wave.expanded = expanded;
        }
    }

    waves
}

fn clamp_selected_wave_idx(selected_wave_idx: usize, wave_count: usize) -> usize {
    if wave_count == 0 {
        0
    } else {
        selected_wave_idx.min(wave_count - 1)
    }
}

fn plan_is_complete(plan: &PlanEntry) -> bool {
    !plan.active && !plan.status.is_failed()
}

fn derive_plan_waves(root: &Path, plans: &[PlanSummary]) -> HashMap<String, usize> {
    if plans.is_empty() {
        return HashMap::new();
    }

    let known_plan_ids: HashSet<String> = plans.iter().map(|plan| plan.id.clone()).collect();
    let mut deps_by_plan: HashMap<String, Vec<String>> = HashMap::new();
    let mut saw_dependency = false;

    for plan in plans {
        let tasks_path = plans_dir(root).join(&plan.id).join("tasks.toml");
        let mut deps: HashSet<String> = HashSet::new();

        if let Ok(tasks_file) = TasksFile::parse(&tasks_path) {
            for task in &tasks_file.tasks {
                deps.extend(task_plan_dependencies(task, &plan.id, &known_plan_ids));
            }
        }

        let mut deps = deps.into_iter().collect::<Vec<_>>();
        deps.sort();
        saw_dependency |= !deps.is_empty();
        deps_by_plan.insert(plan.id.clone(), deps);
    }

    if !saw_dependency {
        return HashMap::new();
    }

    let mut plan_waves = HashMap::new();
    for plan in plans {
        let mut visiting = HashSet::new();
        let wave = resolve_plan_wave(&plan.id, &deps_by_plan, &mut plan_waves, &mut visiting);
        plan_waves.insert(plan.id.clone(), wave);
    }
    plan_waves
}

fn task_plan_dependencies(
    task: &TaskDef,
    current_plan_id: &str,
    known_plan_ids: &HashSet<String>,
) -> Vec<String> {
    let mut deps = HashSet::new();

    for dep in &task.depends_on_plan {
        let plan_id = dep.trim();
        if !plan_id.is_empty() && plan_id != current_plan_id && known_plan_ids.contains(plan_id) {
            deps.insert(plan_id.to_string());
        }
    }

    for dep in &task.depends_on {
        let Some((plan_id, _task_id)) = dep.split_once(':') else {
            continue;
        };
        let plan_id = plan_id.trim();
        if !plan_id.is_empty() && plan_id != current_plan_id && known_plan_ids.contains(plan_id) {
            deps.insert(plan_id.to_string());
        }
    }

    let mut deps = deps.into_iter().collect::<Vec<_>>();
    deps.sort();
    deps
}

fn resolve_plan_wave(
    plan_id: &str,
    deps_by_plan: &HashMap<String, Vec<String>>,
    cache: &mut HashMap<String, usize>,
    visiting: &mut HashSet<String>,
) -> usize {
    if let Some(&wave) = cache.get(plan_id) {
        return wave;
    }

    if !visiting.insert(plan_id.to_string()) {
        return 0;
    }

    let wave = deps_by_plan
        .get(plan_id)
        .map(|deps| {
            deps.iter()
                .map(|dep| resolve_plan_wave(dep, deps_by_plan, cache, visiting) + 1)
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);

    visiting.remove(plan_id);
    cache.insert(plan_id.to_string(), wave);
    wave
}

/// Extract output text from an episode's extra fields.
fn extract_episode_output(episode: &roko_learn::episode_logger::Episode) -> String {
    for key in [
        "stderr",
        "agent_stderr",
        "output",
        "stdout",
        "agent_output",
        "output_tail",
        "detail",
        "text",
    ] {
        if let Some(serde_json::Value::String(text)) = episode.extra.get(key) {
            if !text.trim().is_empty() {
                return text.clone();
            }
        }
    }
    episode.failure_reason.as_deref().unwrap_or("").to_string()
}

fn sum_costs(
    data: &DashboardData,
    plans: &mut HashMap<String, f64>,
    tasks: &mut HashMap<String, f64>,
) {
    plans.clear();
    tasks.clear();
    for e in &data.efficiency_events {
        if !e.plan_id.is_empty() {
            *plans.entry(e.plan_id.clone()).or_default() += e.cost_usd;
        }
        if !e.task_id.is_empty() {
            *tasks.entry(e.task_id.clone()).or_default() += e.cost_usd;
        }
    }
}

#[derive(Debug, Clone)]
struct LatestAgentEvent {
    role: String,
    status: String,
    model: String,
    plan_id: Option<String>,
    task_id: String,
    input_tokens: u64,
    output_tokens: u64,
    timestamp: Option<DateTime<Utc>>,
}

fn latest_agent_events(
    events: &[roko_learn::efficiency::AgentEfficiencyEvent],
) -> HashMap<String, LatestAgentEvent> {
    let mut latest = HashMap::new();

    for event in events {
        let timestamp = parse_efficiency_timestamp(&event.timestamp);
        let candidate = LatestAgentEvent {
            role: event.role.clone(),
            status: if event.gate_passed {
                "done".to_string()
            } else {
                "active".to_string()
            },
            model: event.model.clone(),
            plan_id: Some(event.plan_id.clone()),
            task_id: event.task_id.clone(),
            input_tokens: event.input_tokens,
            output_tokens: event.output_tokens,
            timestamp,
        };

        let should_replace = latest
            .get(&event.agent_id)
            .map(
                |existing: &LatestAgentEvent| match (existing.timestamp, candidate.timestamp) {
                    (Some(lhs), Some(rhs)) => rhs >= lhs,
                    (None, Some(_)) => true,
                    _ => false,
                },
            )
            .unwrap_or(true);
        if should_replace {
            latest.insert(event.agent_id.clone(), candidate);
        }
    }

    latest
}

fn latest_route_metrics(
    events: &[roko_learn::efficiency::AgentEfficiencyEvent],
    data: &DashboardData,
) -> HashMap<String, RouteMetrics> {
    let mut latest: HashMap<String, (Option<DateTime<Utc>>, RouteMetrics)> = HashMap::new();

    for event in events {
        let timestamp = parse_efficiency_timestamp(&event.timestamp);
        let should_replace = latest
            .get(&event.agent_id)
            .map(|(existing, _)| match (*existing, timestamp) {
                (Some(lhs), Some(rhs)) => rhs >= lhs,
                (None, Some(_)) => true,
                (None, None) => true,
                _ => false,
            })
            .unwrap_or(true);
        if should_replace {
            latest.insert(
                event.agent_id.clone(),
                (timestamp, route_metrics_from_event(event, data)),
            );
        }
    }

    latest
        .into_iter()
        .map(|(agent_id, (_, metrics))| (agent_id, metrics))
        .collect()
}

fn plan_is_active(status: &str) -> bool {
    matches!(
        status.to_ascii_lowercase().as_str(),
        "active"
            | "running"
            | "executing"
            | "in_progress"
            | "implementing"
            | "gating"
            | "verifying"
            | "reviewing"
            | "strategist"
            | "implementer"
            | "preflight"
    )
}

fn compute_token_rate(events: &[roko_learn::efficiency::AgentEfficiencyEvent]) -> f64 {
    let mut first_seen: Option<DateTime<Utc>> = None;
    let mut last_seen: Option<DateTime<Utc>> = None;
    let mut total_tokens = 0_u64;

    for event in events {
        let Some(timestamp) = parse_efficiency_timestamp(&event.timestamp) else {
            continue;
        };
        first_seen = Some(match first_seen {
            Some(current) => current.min(timestamp),
            None => timestamp,
        });
        last_seen = Some(match last_seen {
            Some(current) => current.max(timestamp),
            None => timestamp,
        });
        total_tokens = total_tokens.saturating_add(event.total_tokens());
    }

    let Some(first_seen) = first_seen else {
        return 0.0;
    };
    let Some(last_seen) = last_seen else {
        return 0.0;
    };

    let elapsed_seconds = last_seen.signed_duration_since(first_seen).num_seconds();
    if elapsed_seconds <= 0 {
        return 0.0;
    }

    total_tokens as f64 / (elapsed_seconds as f64 / 60.0)
}

fn build_token_samples(data: &DashboardData) -> HashMap<String, VecDeque<(DateTime<Utc>, u64)>> {
    const MAX_TOKEN_HISTORY_SAMPLES: usize = 120;

    let mut per_agent: HashMap<String, Vec<(DateTime<Utc>, u64)>> = HashMap::new();

    if data.efficiency_events.is_empty() {
        for episode in data.episodes() {
            let total_tokens = episode
                .usage
                .input_tokens
                .saturating_add(episode.usage.output_tokens);
            per_agent
                .entry(episode.agent_id.clone())
                .or_default()
                .push((episode.timestamp, total_tokens));
        }
    } else {
        for event in &data.efficiency_events {
            let Some(timestamp) = parse_efficiency_timestamp(&event.timestamp) else {
                continue;
            };
            per_agent
                .entry(event.agent_id.clone())
                .or_default()
                .push((timestamp, event.total_tokens()));
        }
    }

    let mut histories = HashMap::new();
    for (agent_id, mut samples) in per_agent {
        samples.sort_by(|lhs, rhs| lhs.0.cmp(&rhs.0));

        let mut cumulative_total = 0u64;
        let mut history = VecDeque::new();
        for (timestamp, total_tokens) in samples {
            cumulative_total = cumulative_total.saturating_add(total_tokens);
            history.push_back((timestamp, cumulative_total));
            if history.len() > MAX_TOKEN_HISTORY_SAMPLES {
                history.pop_front();
            }
        }

        histories.insert(agent_id, history);
    }

    histories
}

fn compute_windowed_token_rate(samples: &VecDeque<(DateTime<Utc>, u64)>) -> f64 {
    const TOKEN_RATE_WINDOW_SAMPLES: usize = 60;

    if samples.len() < 2 {
        return 0.0;
    }

    let start_idx = samples.len().saturating_sub(TOKEN_RATE_WINDOW_SAMPLES);
    let Some((start_time, start_total)) = samples.get(start_idx) else {
        return 0.0;
    };
    let Some((end_time, end_total)) = samples.back() else {
        return 0.0;
    };

    let elapsed_secs = end_time
        .signed_duration_since(*start_time)
        .num_milliseconds() as f64
        / 1_000.0;
    if elapsed_secs <= 0.0 {
        return 0.0;
    }

    end_total.saturating_sub(*start_total) as f64 * 60.0 / elapsed_secs
}

fn parse_efficiency_timestamp(timestamp: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

fn build_task_checklist_from_execution(data: &DashboardData) -> Vec<TaskRow> {
    if let Some(exec) = &data.current_plan_execution {
        return exec
            .tasks
            .iter()
            .map(|t| {
                let status = if t.is_current {
                    TaskStatus::Active
                } else {
                    TaskStatus::from(t.phase.as_str())
                };
                let elapsed_secs = parse_duration_to_secs(&t.duration);
                TaskRow {
                    id: t.task_id.clone(),
                    title: t.title.clone(),
                    status,
                    elapsed_secs,
                }
            })
            .collect();
    }

    // Fallback: build from active_tasks
    data.active_tasks
        .iter()
        .map(|t| {
            let status = TaskStatus::from(t.status.as_str());
            TaskRow {
                id: t.task_id.clone(),
                title: t.task_id.clone(),
                status,
                elapsed_secs: 0.0,
            }
        })
        .collect()
}

/// Parse a duration string like "5s", "2m 30s", "120ms" into seconds.
fn parse_duration_to_secs(duration: &str) -> f64 {
    if duration == "--" || duration.is_empty() {
        return 0.0;
    }
    if let Some(ms) = duration.strip_suffix("ms") {
        return ms.parse::<f64>().unwrap_or(0.0) / 1000.0;
    }
    let mut total = 0.0;
    for part in duration.split_whitespace() {
        if let Some(m) = part.strip_suffix('m') {
            total += m.parse::<f64>().unwrap_or(0.0) * 60.0;
        } else if let Some(s) = part.strip_suffix('s') {
            total += s.parse::<f64>().unwrap_or(0.0);
        }
    }
    total
}

/// Populate phase_pipeline elapsed times from episode timestamps (Task 7).
fn populate_phase_elapsed(
    pipeline: &mut [PhaseStep],
    episodes: &[roko_learn::episode_logger::Episode],
) {
    if episodes.is_empty() || pipeline.is_empty() {
        return;
    }

    // Compute per-phase elapsed from episodes that have a matching trigger_kind
    // or kind. Map episode kinds to canonical phase names.
    let mut phase_durations: std::collections::HashMap<String, f64> =
        std::collections::HashMap::new();

    for episode in episodes {
        let phase_name = episode_to_phase_name(episode);
        if !phase_name.is_empty() {
            *phase_durations.entry(phase_name).or_default() += episode.duration_secs;
        }
    }

    for step in pipeline.iter_mut() {
        if let Some(&elapsed) = phase_durations.get(&step.name) {
            step.elapsed_secs = elapsed;
        }
    }
}

/// Map an episode's kind/trigger to a canonical phase name.
fn episode_to_phase_name(episode: &roko_learn::episode_logger::Episode) -> String {
    let kind = episode.kind.to_ascii_lowercase();
    let trigger = episode.trigger_kind.to_ascii_lowercase();
    let template = episode.agent_template.to_ascii_lowercase();

    // Direct kind matches
    match kind.as_str() {
        "preflight" => return "preflight".to_string(),
        "compile" | "compile-gate" | "compile_gate" => return "compile-gate".to_string(),
        "test" | "test-gate" | "test_gate" => return "test-gate".to_string(),
        "review" | "reviewing" | "critic-review" | "critic_review" => {
            return "reviewing".to_string();
        }
        "verdict" => return "verdict".to_string(),
        "commit" | "committing" => return "committing".to_string(),
        _ => {}
    }

    // Template-based inference
    if template.contains("strategist") {
        return "strategist".to_string();
    }
    if template.contains("implementer") || template.contains("implement") {
        return "implementer".to_string();
    }
    if template.contains("reviewer") || template.contains("critic") {
        return "critic-review".to_string();
    }

    // Trigger-based inference
    if trigger.contains("gate") {
        if trigger.contains("compile") {
            return "compile-gate".to_string();
        }
        if trigger.contains("test") {
            return "test-gate".to_string();
        }
    }

    // Agent turn episodes map to implementer by default
    if kind == "agent_turn" {
        return "implementer".to_string();
    }

    String::new()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use roko_learn::episode_logger::Episode;

    use super::*;
    use crate::tui::dashboard::TaskSummary;
    use crate::tui::input::LogFilterLevel;
    use roko_learn::efficiency::AgentEfficiencyEvent;
    use tempfile::tempdir;

    fn efficiency_event(
        role: &str,
        input_tokens: u64,
        output_tokens: u64,
        timestamp: &str,
    ) -> AgentEfficiencyEvent {
        AgentEfficiencyEvent {
            role: role.to_string(),
            input_tokens,
            output_tokens,
            timestamp: timestamp.to_string(),
            ..AgentEfficiencyEvent::default()
        }
    }

    #[test]
    fn default_state_is_idle_dashboard() {
        let state = TuiState::default();
        assert_eq!(state.active_tab, Tab::Dashboard);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert_eq!(state.focus, FocusZone::PlanTree);
        assert_eq!(state.orchestrator_state, "idle");
        assert!(!state.is_text_input());
    }

    #[test]
    fn reset_scrolls_zeroes_all() {
        let mut state = TuiState::default();
        state.agent_scroll = Some(50);
        state.diff_scroll = 10;
        state.log_scroll = 100;
        state.agent_topology_scroll_offset = 8;

        state.reset_scrolls();

        assert_eq!(state.agent_scroll, None);
        assert_eq!(state.diff_scroll, 0);
        assert_eq!(state.log_scroll, 0);
        assert_eq!(state.agent_topology_scroll_offset, 0);
    }

    #[test]
    fn agent_topology_toggle_and_clamp_work() {
        let mut state = TuiState::default();

        assert!(!state.agent_topology_visible);

        state.toggle_agent_topology();
        assert!(state.agent_topology_visible);

        state.agent_topology_scroll_offset = 42;
        state.clamp_agent_topology_scroll(12);
        assert_eq!(state.agent_topology_scroll_offset, 12);

        state.close_agent_topology();
        assert!(!state.agent_topology_visible);
    }

    #[test]
    fn task_counts_sums_across_plans() {
        let mut state = TuiState::default();
        state.plans = vec![
            PlanEntry {
                tasks_total: 5,
                tasks_done: 3,
                ..PlanEntry::default()
            },
            PlanEntry {
                tasks_total: 10,
                tasks_done: 7,
                ..PlanEntry::default()
            },
        ];
        assert_eq!(state.task_counts(), (10, 15));
    }

    #[test]
    fn log_filter_defaults_to_all_levels() {
        let state = TuiState::default();
        for level in LogFilterLevel::all() {
            assert!(state.log_level_visible(level));
        }
    }

    #[test]
    fn log_filter_toggle_and_reset_work() {
        let mut state = TuiState::default();
        state.toggle_log_filter_level(LogFilterLevel::Warn);
        assert!(!state.log_level_visible(LogFilterLevel::Warn));

        state.show_all_log_filter_levels();
        assert!(state.log_level_visible(LogFilterLevel::Warn));
    }

    #[test]
    fn elapsed_secs_zero_when_not_started() {
        let state = TuiState::default();
        assert_eq!(state.elapsed_secs(), 0.0);
    }

    #[test]
    fn wave_count_and_current_wave() {
        let mut state = TuiState::default();
        assert_eq!(state.wave_count(), 0);
        assert_eq!(state.current_wave(), 0);

        state.execution_waves = vec![
            Wave {
                index: 0,
                total: 2,
                ..Wave::default()
            },
            Wave {
                index: 1,
                total: 1,
                ..Wave::default()
            },
        ];
        state.selected_wave_idx = 1;
        assert_eq!(state.wave_count(), 2);
        assert_eq!(state.current_wave(), 1);
    }

    #[test]
    fn derive_plan_waves_uses_cross_plan_dependencies() {
        let tmpdir = tempdir().expect("tempdir");
        let plans_root = tmpdir.path().join("plans");
        fs::create_dir_all(plans_root.join("plan-a")).expect("create plan-a");
        fs::create_dir_all(plans_root.join("plan-b")).expect("create plan-b");
        fs::create_dir_all(plans_root.join("plan-c")).expect("create plan-c");

        fs::write(
            plans_root.join("plan-a").join("tasks.toml"),
            r#"
[meta]
plan = "Plan A"
total = 1

[[task]]
id = "T1"
title = "start"
depends_on = []
"#,
        )
        .expect("write plan-a");

        fs::write(
            plans_root.join("plan-b").join("tasks.toml"),
            r#"
[meta]
plan = "Plan B"
total = 1

[[task]]
id = "T1"
title = "after a"
depends_on = []
depends_on_plan = ["plan-a"]
"#,
        )
        .expect("write plan-b");

        fs::write(
            plans_root.join("plan-c").join("tasks.toml"),
            r#"
[meta]
plan = "Plan C"
total = 1

[[task]]
id = "T1"
title = "after b"
depends_on = ["plan-b:T1"]
"#,
        )
        .expect("write plan-c");

        let plans = vec![
            PlanSummary {
                id: "plan-a".into(),
                title: "Plan A".into(),
                task_count: 1,
                tasks_done: 0,
                tasks_failed: 0,
                completed: false,
                old_format: false,
                last_error: None,
            },
            PlanSummary {
                id: "plan-b".into(),
                title: "Plan B".into(),
                task_count: 1,
                tasks_done: 0,
                tasks_failed: 0,
                completed: false,
                old_format: false,
                last_error: None,
            },
            PlanSummary {
                id: "plan-c".into(),
                title: "Plan C".into(),
                task_count: 1,
                tasks_done: 0,
                tasks_failed: 0,
                completed: false,
                old_format: false,
                last_error: None,
            },
        ];

        let plan_waves = derive_plan_waves(tmpdir.path(), &plans);
        assert_eq!(plan_waves.get("plan-a"), Some(&0));
        assert_eq!(plan_waves.get("plan-b"), Some(&1));
        assert_eq!(plan_waves.get("plan-c"), Some(&2));
    }

    #[test]
    fn rebuild_execution_waves_preserves_expanded_state_by_index() {
        let plans = vec![
            PlanEntry {
                id: "plan-a".into(),
                wave: Some(1),
                ..PlanEntry::default()
            },
            PlanEntry {
                id: "plan-b".into(),
                wave: Some(2),
                ..PlanEntry::default()
            },
        ];
        let previous = vec![
            Wave {
                index: 1,
                expanded: false,
                ..Wave::default()
            },
            Wave {
                index: 2,
                expanded: true,
                ..Wave::default()
            },
        ];

        let waves = rebuild_execution_waves(&plans, &previous);

        assert_eq!(waves.len(), 2);
        assert_eq!(waves[0].index, 1);
        assert!(!waves[0].expanded);
        assert_eq!(waves[1].index, 2);
        assert!(waves[1].expanded);
    }

    #[test]
    fn active_agent_count_filters_correctly() {
        let mut state = TuiState::default();
        state.agents = vec![
            AgentRow {
                active: true,
                ..AgentRow::default()
            },
            AgentRow {
                active: false,
                ..AgentRow::default()
            },
            AgentRow {
                active: true,
                ..AgentRow::default()
            },
        ];
        assert_eq!(state.active_agent_count(), 2);
    }

    #[test]
    fn from_dashboard_data_creates_valid_state() {
        let data = DashboardData::default();
        let state = TuiState::from_dashboard_data(&data);
        assert_eq!(state.orchestrator_state, "idle");
        assert!(state.plans.is_empty());
        assert!(state.agents.is_empty());
        assert_eq!(state.token_total, 0);
        assert_eq!(state.cost_dollars, 0.0);
    }

    #[test]
    fn phase_pipeline_defaults_to_canonical_phases() {
        let data = DashboardData::default();
        let state = TuiState::from_dashboard_data(&data);
        assert_eq!(state.phase_pipeline.len(), 9);
        assert_eq!(state.phase_pipeline[0].name, "preflight");
        assert_eq!(state.phase_pipeline[8].name, "committing");
    }

    #[test]
    fn phase_pipeline_uses_task_progression_instead_of_position_heuristics() {
        let tasks = vec![
            TaskSummary {
                plan_id: "plan-a".to_string(),
                task_id: "task-impl".to_string(),
                status: "implementing".to_string(),
                iteration: 1,
                assigned_agents: vec!["implementer-1".to_string()],
                latest_gate: None,
            },
            TaskSummary {
                plan_id: "plan-b".to_string(),
                task_id: "task-verify".to_string(),
                status: "verifying".to_string(),
                iteration: 1,
                assigned_agents: vec!["implementer-2".to_string()],
                latest_gate: Some("compile".to_string()),
            },
        ];

        let pipeline = build_phase_pipeline(&tasks);

        assert_eq!(pipeline[0].status, PhaseStatus::Done);
        assert_eq!(pipeline[0].pct, 100.0);
        assert_eq!(pipeline[1].status, PhaseStatus::Done);
        assert_eq!(pipeline[1].pct, 100.0);
        assert_eq!(pipeline[2].status, PhaseStatus::Active);
        assert_eq!(pipeline[2].pct, 50.0);
        assert_eq!(pipeline[3].status, PhaseStatus::Pending);
        assert_eq!(pipeline[3].pct, 50.0);
        assert_eq!(pipeline[4].status, PhaseStatus::Active);
        assert_eq!(pipeline[4].pct, 0.0);
        assert_eq!(pipeline[5].status, PhaseStatus::Pending);
        assert_eq!(pipeline[5].pct, 0.0);
    }

    #[test]
    fn phase_pipeline_marks_failed_gate_from_task_data() {
        let tasks = vec![TaskSummary {
            plan_id: "plan-a".to_string(),
            task_id: "task-test".to_string(),
            status: "failed".to_string(),
            iteration: 2,
            assigned_agents: vec!["implementer-1".to_string()],
            latest_gate: Some("test".to_string()),
        }];

        let pipeline = build_phase_pipeline(&tasks);

        assert_eq!(pipeline[0].status, PhaseStatus::Done);
        assert_eq!(pipeline[1].status, PhaseStatus::Done);
        assert_eq!(pipeline[2].status, PhaseStatus::Done);
        assert_eq!(pipeline[3].status, PhaseStatus::Done);
        assert_eq!(pipeline[4].status, PhaseStatus::Failed);
        assert_eq!(pipeline[4].pct, 0.0);
    }

    #[test]
    fn from_dashboard_data_populates_orchestrator_fields_from_executor_state() {
        let tmpdir = tempdir().expect("tempdir");
        let state_dir = tmpdir.path().join(".roko/state");
        fs::create_dir_all(&state_dir).expect("state dir");

        let executor_state = serde_json::json!({
            "plan_states": {
                "plan-a": {
                    "current_phase": { "kind": "gating" },
                    "iteration": 3,
                    "started_at_ms": 10,
                    "paused": false
                },
                "plan-b": {
                    "current_phase": { "kind": "implementing" },
                    "iteration": 2,
                    "started_at_ms": 20,
                    "paused": false
                }
            }
        });
        fs::write(
            state_dir.join("executor.json"),
            serde_json::to_vec(&executor_state).expect("executor json"),
        )
        .expect("write executor state");

        let data = DashboardData::load_best_effort(tmpdir.path());
        let state = TuiState::from_dashboard_data(&data);

        assert_eq!(state.orchestrator_state, "running");
        assert_eq!(state.current_iteration, 2);
        assert_eq!(state.current_phase, "implementing");
    }

    #[test]
    fn from_dashboard_data_populates_plan_tasks_from_tracker_and_episodes() {
        let tmpdir = tempdir().expect("tempdir");
        let root = tmpdir.path();
        let state_dir = root.join(".roko/state");
        let plan_dir = root.join(".roko/plans/plan-a");
        let memory_dir = root.join(".roko/memory");

        fs::create_dir_all(&state_dir).expect("state dir");
        fs::create_dir_all(&plan_dir).expect("plan dir");
        fs::create_dir_all(&memory_dir).expect("memory dir");

        let executor_state = serde_json::json!({
            "plan_states": {
                "plan-a": {
                    "current_phase": { "kind": "implementing" },
                    "task_id": "task-2",
                    "assigned_agents": ["agent-a"]
                }
            }
        });
        fs::write(
            state_dir.join("executor.json"),
            serde_json::to_vec(&executor_state).expect("executor json"),
        )
        .expect("write executor state");

        let tracker_state = serde_json::json!([
            {
                "plan_id": "plan-a",
                "completed": ["task-1"],
                "failed": ["task-3"],
                "current_group_index": 1
            }
        ]);
        fs::write(
            state_dir.join("task-trackers.json"),
            serde_json::to_vec(&tracker_state).expect("tracker json"),
        )
        .expect("write tracker state");

        fs::write(
            plan_dir.join("tasks.toml"),
            r#"
[meta]
plan = "Plan A"
iteration = 1
total = 3
done = 1
status = "running"

[[task]]
id = "task-1"
title = "Bootstrap"
status = "done"
model = "claude-haiku-4-5"
elapsed_ms = 1000
tier = "focused"

[[task]]
id = "task-2"
title = "Wire dashboard"
status = "implementing"
model = "claude-sonnet-4-6"
elapsed_ms = 2500
started_at_ms = 111
wave = 2
tier = "focused"

[[task]]
id = "task-3"
title = "Handle failures"
status = "gate_rejected"
model = "claude-sonnet-4-6"
elapsed_ms = 3500
ended_at_ms = 222
tier = "focused"
"#,
        )
        .expect("tasks.toml");

        let mut task_one = Episode::new("agent-a", "task-1");
        task_one.input_signal_hash = "plan-a".to_string();
        task_one
            .extra
            .insert("plan_id".to_string(), serde_json::json!("plan-a"));
        task_one
            .extra
            .insert("task_id".to_string(), serde_json::json!("task-1"));
        task_one.usage.wall_ms = 1_500;

        let mut task_two = Episode::new("agent-a", "task-2");
        task_two.input_signal_hash = "plan-a".to_string();
        task_two
            .extra
            .insert("plan_id".to_string(), serde_json::json!("plan-a"));
        task_two
            .extra
            .insert("task_id".to_string(), serde_json::json!("task-2"));
        task_two.usage.wall_ms = 2_500;

        let episodes = format!(
            "{}\n{}\n",
            serde_json::to_string(&task_one).expect("task one episode"),
            serde_json::to_string(&task_two).expect("task two episode")
        );
        fs::write(memory_dir.join("episodes.jsonl"), episodes).expect("write episodes");

        let data = DashboardData::load_best_effort(root);
        let state = TuiState::from_dashboard_data(&data);
        let plan = state
            .plans
            .iter()
            .find(|plan| plan.id == "plan-a")
            .expect("plan-a");

        assert_eq!(plan.status, PlanPhase::Active);
        assert_eq!(plan.phase, "implementing");
        assert!(plan.active);
        assert_eq!(plan.tasks_total, 3);
        assert_eq!(plan.tasks_done, 1);
        assert_eq!(plan.tasks_failed, 1);
        assert!((plan.elapsed_secs - 7.0).abs() < f64::EPSILON);
        assert_eq!(plan.wave, Some(2));
        assert_eq!(plan.tasks.len(), 3);
        assert_eq!(plan.tasks[0].id, "task-1");
        assert_eq!(plan.tasks[0].status, TaskStatus::Done);
        assert_eq!(plan.tasks[1].id, "task-2");
        assert_eq!(plan.tasks[1].status, TaskStatus::Active);
        assert_eq!(plan.tasks[1].agent_id.as_deref(), Some("agent-a"));
        assert_eq!(plan.tasks[2].id, "task-3");
        assert_eq!(plan.tasks[2].status, TaskStatus::Failed);
    }

    #[test]
    fn plan_task_counts_uses_summary_progress_without_snapshot() {
        let summary = crate::plan::PlanSummary {
            id: "plan-a".into(),
            title: "Plan A".into(),
            task_count: 5,
            tasks_done: 2,
            tasks_failed: 1,
            completed: false,
            old_format: false,
            last_error: None,
        };

        assert_eq!(plan_task_counts(&summary, None, 5), (2, 1));
    }

    #[test]
    fn plan_task_counts_prefers_snapshot_task_statuses() {
        let summary = crate::plan::PlanSummary {
            id: "plan-a".into(),
            title: "Plan A".into(),
            task_count: 3,
            tasks_done: 0,
            tasks_failed: 0,
            completed: false,
            old_format: false,
            last_error: None,
        };
        let snapshot = PlanTaskListSnapshot {
            tasks_done: 0,
            tasks_failed: 0,
            tasks: vec![
                crate::tui::dashboard::PlanTaskSnapshot {
                    id: "task-1".into(),
                    title: "Done".into(),
                    status: "done".into(),
                    agent_id: None,
                    ..crate::tui::dashboard::PlanTaskSnapshot::default()
                },
                crate::tui::dashboard::PlanTaskSnapshot {
                    id: "task-2".into(),
                    title: "Active".into(),
                    status: "implementing".into(),
                    agent_id: None,
                    ..crate::tui::dashboard::PlanTaskSnapshot::default()
                },
                crate::tui::dashboard::PlanTaskSnapshot {
                    id: "task-3".into(),
                    title: "Failed".into(),
                    status: "failed".into(),
                    agent_id: None,
                    ..crate::tui::dashboard::PlanTaskSnapshot::default()
                },
            ],
            ..PlanTaskListSnapshot::default()
        };

        assert_eq!(plan_task_counts(&summary, Some(&snapshot), 3), (1, 1));
    }

    #[test]
    fn new_fields_have_defaults() {
        let state = TuiState::default();
        assert!(state.phase_pipeline.is_empty());
        assert!(state.execution_waves.is_empty());
        assert!(state.current_task_checklist.is_empty());
        assert_eq!(state.sys.cpu_pct, 0.0);
        assert_eq!(state.token_total, 0);
        assert!(state.token_history.is_empty());
        assert_eq!(state.token_rate, 0.0);
        assert_eq!(state.cost_rate, 0.0);
        assert_eq!(state.cost_dollars, 0.0);
        assert!(state.git_commit_short.is_empty());
        assert!(state.git_age.is_empty());
        assert!(state.run_started.is_none());
        assert!(state.filter.is_empty());
        assert!(state.log_auto_tail);
        assert_eq!(state.selected_agent, 0);
        assert_eq!(state.agent_scroll, None);
        assert_eq!(state.plan_scroll_offset, 0);
    }

    #[test]
    fn update_from_dashboard_snapshot_maps_connected_state_and_preserves_navigation() {
        use roko_core::dashboard_snapshot::{
            AgentState as SnapshotAgentState, DashboardSnapshot, ErrorEntry, GateVerdict, PlanState,
        };

        let mut state = TuiState::default();
        state.active_tab = Tab::Git;
        state.selected_plan_idx = 1;
        state.current_plan_idx = 0;
        state.selected_agent = 1;
        state.selected_agent_tab = 4;
        state.focus = FocusZone::AgentOutput;
        state.agent_scroll = Some(12);
        state.diff_scroll = 7;
        state.task_scroll = 9;
        state.command_output_scroll = 11;
        state.plan_detail_scroll = 13;
        state.plan_scroll_offset = 15;
        state.log_scroll = 17;
        state.log_auto_tail = false;

        state.plans = vec![
            PlanEntry {
                id: "plan-b".into(),
                expanded: false,
                ..PlanEntry::default()
            },
            PlanEntry {
                id: "plan-a".into(),
                expanded: true,
                ..PlanEntry::default()
            },
        ];
        state.agents = vec![
            AgentRow {
                id: "agent-b".into(),
                active: false,
                ..AgentRow::default()
            },
            AgentRow {
                id: "agent-a".into(),
                active: true,
                ..AgentRow::default()
            },
        ];

        let snap = DashboardSnapshot {
            plans: [
                (
                    "plan-a".to_string(),
                    PlanState {
                        plan_id: "plan-a".into(),
                        phase: "started".into(),
                        tasks_total: 4,
                        tasks_done: 1,
                        tasks_failed: 0,
                        active: true,
                    },
                ),
                (
                    "plan-b".to_string(),
                    PlanState {
                        plan_id: "plan-b".into(),
                        phase: "failed".into(),
                        tasks_total: 2,
                        tasks_done: 1,
                        tasks_failed: 1,
                        active: false,
                    },
                ),
            ]
            .into_iter()
            .collect(),
            tasks: Default::default(),
            agents: [
                (
                    "agent-a".to_string(),
                    SnapshotAgentState {
                        agent_id: "agent-a".into(),
                        role: "implementer".into(),
                        active: true,
                        output_bytes: 128,
                        model: String::new(),
                        input_tokens: 0,
                        output_tokens: 0,
                        cost_usd: 0.0,
                        current_task: String::new(),
                        current_plan: String::new(),
                    },
                ),
                (
                    "agent-b".to_string(),
                    SnapshotAgentState {
                        agent_id: "agent-b".into(),
                        role: "reviewer".into(),
                        active: false,
                        output_bytes: 0,
                        model: String::new(),
                        input_tokens: 0,
                        output_tokens: 0,
                        cost_usd: 0.0,
                        current_task: String::new(),
                        current_plan: String::new(),
                    },
                ),
            ]
            .into_iter()
            .collect(),
            gates: vec![
                GateVerdict {
                    plan_id: "plan-a".into(),
                    task_id: "task-1".into(),
                    gate: "compile".into(),
                    passed: true,
                    ts_millis: 1_000,
                },
                GateVerdict {
                    plan_id: "plan-b".into(),
                    task_id: "task-2".into(),
                    gate: "test".into(),
                    passed: false,
                    ts_millis: 2_000,
                },
            ],
            diagnoses: Default::default(),
            experiment_winners: Vec::new(),
            agent_topology: roko_core::AgentTopology::default(),
            efficiency_trend: Vec::new(),
            cfactor_trend: Vec::new(),
            gate_trends: HashMap::new(),
            gate_recent_failures: Vec::new(),
            episodes: Default::default(),
            errors: vec![
                ErrorEntry {
                    message: "compile failed".into(),
                    ts_millis: 3_000,
                },
                ErrorEntry {
                    message: "timeout".into(),
                    ts_millis: 4_000,
                },
            ],
            event_log: Default::default(),
            task_outputs: Default::default(),
            cascade_router_json: String::new(),
            gate_thresholds_json: String::new(),
            stats: Default::default(),
        };

        state.update_from_dashboard_snapshot(&snap);

        assert_eq!(state.plans.len(), 2);
        let plan_a = state.plans.iter().find(|plan| plan.id == "plan-a").unwrap();
        let plan_b = state.plans.iter().find(|plan| plan.id == "plan-b").unwrap();
        assert_eq!(plan_a.status, PlanPhase::Active);
        assert!(plan_a.active);
        assert!(plan_a.expanded);
        assert_eq!(plan_b.status, PlanPhase::Failed);
        assert_eq!(plan_b.tasks_failed, 1);

        assert_eq!(state.agents.len(), 2);
        let agent_a = state
            .agents
            .iter()
            .find(|agent| agent.id == "agent-a")
            .unwrap();
        let agent_b = state
            .agents
            .iter()
            .find(|agent| agent.id == "agent-b")
            .unwrap();
        assert!(agent_a.active);
        assert_eq!(agent_a.role, "implementer");
        assert!(!agent_b.active);

        assert_eq!(state.gate_results.len(), 2);
        assert_eq!(state.gate_results[0].gate, "compile");
        assert_eq!(state.gate_results[1].plan_id, "plan-b");
        assert!(!state.gate_results[1].passed);

        assert_eq!(state.active_tab, Tab::Git);
        assert_eq!(state.plans[state.selected_plan_idx].id, "plan-a");
        assert_eq!(state.plans[state.current_plan_idx].id, "plan-b");
        assert_eq!(state.agents[state.selected_agent].id, "agent-a");
        assert_eq!(state.selected_agent_tab, 4);
        assert_eq!(state.focus, FocusZone::AgentOutput);
        assert_eq!(state.agent_scroll, Some(12));
        assert_eq!(state.diff_scroll, 7);
        assert_eq!(state.task_scroll, 9);
        assert_eq!(state.command_output_scroll, 11);
        assert_eq!(state.plan_detail_scroll, 13);
        assert_eq!(state.plan_scroll_offset, 15);
        assert_eq!(state.log_scroll, 17);
        assert!(!state.log_auto_tail);
    }

    #[test]
    fn update_from_dashboard_snapshot_keeps_expanded_state_when_matching_plan_remains() {
        use roko_core::dashboard_snapshot::{DashboardSnapshot, PlanState};

        let mut state = TuiState::default();
        state.plans = vec![
            PlanEntry {
                id: "plan-a".into(),
                expanded: false,
                ..PlanEntry::default()
            },
            PlanEntry {
                id: "plan-b".into(),
                expanded: true,
                ..PlanEntry::default()
            },
        ];

        let snap = DashboardSnapshot {
            plans: [(
                "plan-b".to_string(),
                PlanState {
                    plan_id: "plan-b".into(),
                    phase: "completed".into(),
                    tasks_total: 1,
                    tasks_done: 1,
                    tasks_failed: 0,
                    active: false,
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        state.update_from_dashboard_snapshot(&snap);

        assert_eq!(state.plans.len(), 1);
        assert_eq!(state.plans[0].id, "plan-b");
        assert!(state.plans[0].expanded);
    }

    #[test]
    fn update_from_snapshot_populates_token_rate() {
        let mut data = DashboardData::default();
        data.efficiency_events = vec![
            efficiency_event("impl", 100, 50, "2026-04-14T12:00:00Z"),
            efficiency_event("review", 20, 10, "2026-04-14T12:05:00Z"),
            efficiency_event("impl", 40, 10, "2026-04-14T12:10:00Z"),
        ];

        let mut state = TuiState::default();
        state.update_from_snapshot(&data);

        assert!((state.token_rate - 8.0).abs() < f64::EPSILON);
    }

    #[test]
    fn update_from_snapshot_populates_token_history_for_selected_agent() {
        let mut data = DashboardData::default();
        data.agents = vec![
            crate::tui::dashboard::AgentSummary {
                id: "agent-a".into(),
                label: "agent-a".into(),
                plan_id: None,
                status: "active".into(),
            },
            crate::tui::dashboard::AgentSummary {
                id: "agent-b".into(),
                label: "agent-b".into(),
                plan_id: None,
                status: "active".into(),
            },
        ];
        data.efficiency_events = vec![
            AgentEfficiencyEvent {
                agent_id: "agent-a".into(),
                role: "implementer".into(),
                input_tokens: 100,
                output_tokens: 20,
                timestamp: "2026-04-14T12:00:00Z".into(),
                ..AgentEfficiencyEvent::default()
            },
            AgentEfficiencyEvent {
                agent_id: "agent-b".into(),
                role: "reviewer".into(),
                input_tokens: 30,
                output_tokens: 10,
                timestamp: "2026-04-14T12:01:00Z".into(),
                ..AgentEfficiencyEvent::default()
            },
            AgentEfficiencyEvent {
                agent_id: "agent-a".into(),
                role: "implementer".into(),
                input_tokens: 50,
                output_tokens: 10,
                timestamp: "2026-04-14T12:02:00Z".into(),
                ..AgentEfficiencyEvent::default()
            },
            AgentEfficiencyEvent {
                agent_id: "agent-b".into(),
                role: "reviewer".into(),
                input_tokens: 50,
                output_tokens: 10,
                timestamp: "2026-04-14T12:04:00Z".into(),
                ..AgentEfficiencyEvent::default()
            },
        ];

        let mut state = TuiState::default();
        state.selected_agent = 1;
        state.update_from_snapshot(&data);

        assert_eq!(
            state.token_history.get("agent-a").cloned(),
            Some(VecDeque::from(vec![120, 180]))
        );
        assert_eq!(
            state.token_history.get("agent-b").cloned(),
            Some(VecDeque::from(vec![40, 100]))
        );
        assert!((state.token_rate - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn update_from_snapshot_populates_route_metrics() {
        let mut data = DashboardData::default();
        data.efficiency_events = vec![AgentEfficiencyEvent {
            agent_id: "agent-a".to_string(),
            role: "implementer".to_string(),
            model: "claude-haiku-4-5".to_string(),
            input_tokens: 12_000,
            output_tokens: 3_000,
            prompt_sections: vec![roko_learn::efficiency::PromptSectionMeta {
                name: "workspace_map".to_string(),
                tokens: 800,
                priority: 0,
                was_truncated: false,
                was_dropped: false,
            }],
            frequency: OperatingFrequency::Gamma,
            timestamp: "2026-04-14T12:00:00Z".to_string(),
            ..AgentEfficiencyEvent::default()
        }];
        data.cascade_router.confidence_stats.insert(
            "claude-haiku-4-5".to_string(),
            crate::tui::dashboard::CascadeRouterModelStats {
                trials: 10,
                successes: 8,
            },
        );

        let mut state = TuiState::default();
        state.update_from_snapshot(&data);

        let metrics = state.route_metrics.get("agent-a").expect("route metrics");
        assert_eq!(metrics.model, "claude-haiku-4-5");
        assert_eq!(metrics.tier, "fast");
        assert_eq!(metrics.context_used, 15_000);
        assert_eq!(metrics.context_limit, 200_000);
        assert!((metrics.focus_score - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn update_from_dashboard_snapshot_maps_streaming_fields() {
        let mut snap = roko_core::DashboardSnapshot::default();
        snap.plans.insert(
            "plan-a".into(),
            roko_core::dashboard_snapshot::PlanState {
                plan_id: "plan-a".into(),
                phase: "implementer".into(),
                tasks_total: 2,
                tasks_done: 1,
                tasks_failed: 0,
                active: true,
            },
        );
        snap.tasks.insert(
            "plan-a/task-1".into(),
            roko_core::dashboard_snapshot::TaskState {
                task_id: "task-1".into(),
                plan_id: "plan-a".into(),
                phase: "implementer".into(),
                outcome: None,
            },
        );
        snap.tasks.insert(
            "plan-a/task-2".into(),
            roko_core::dashboard_snapshot::TaskState {
                task_id: "task-2".into(),
                plan_id: "plan-a".into(),
                phase: "completed".into(),
                outcome: Some("success".into()),
            },
        );
        snap.agents.insert(
            "agent-1".into(),
            roko_core::dashboard_snapshot::AgentState {
                agent_id: "agent-1".into(),
                role: "implementer".into(),
                active: true,
                output_bytes: 42,
                model: String::new(),
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: 0.0,
                current_task: String::new(),
                current_plan: String::new(),
            },
        );
        snap.gates.push(roko_core::dashboard_snapshot::GateVerdict {
            plan_id: "plan-a".into(),
            task_id: "task-2".into(),
            gate: "compile".into(),
            passed: true,
            ts_millis: 1,
        });
        snap.diagnoses
            .push_back(roko_core::dashboard_snapshot::DiagnosisSummary {
                id: "plan:plan-a:watcher:circuit-breaker:pattern:loop-detected".into(),
                ts: chrono::Utc::now(),
                severity: roko_core::dashboard_snapshot::DiagnosisSeverity::Warn,
                subject: "Circuit Breaker: Loop Detected".into(),
                detail: "repeated identical output".into(),
                suggested_action: Some("Restart Agent".into()),
                intervention_taken: Some("Paused plan".into()),
            });
        snap.experiment_winners
            .push(roko_core::ExperimentWinnerSummary {
                experiment_id: "exp-01".into(),
                parameter: "constraints".into(),
                winner: "claude-opus-4-6".into(),
                winner_variant_id: "opus".into(),
                win_rate: 0.71,
                sample_size: 142,
                ci_lower: 0.63,
                ci_upper: 0.78,
                confidence: 0.97,
            });
        snap.gate_trends.insert(
            "compile".into(),
            roko_core::TrendBuckets::new(3_600, 24, chrono::Utc::now()),
        );
        snap.gate_recent_failures.push(roko_core::FailureEntry {
            ts: chrono::Utc::now(),
            plan_id: "plan-a".into(),
            task_id: "task-2".into(),
            gate: "compile".into(),
            summary: "compile failed".into(),
            artifacts: None,
        });
        snap.errors.push(roko_core::dashboard_snapshot::ErrorEntry {
            message: "compile failed once".into(),
            ts_millis: 2,
        });
        snap.stats.plans_active = 1;
        snap.stats.tasks_active = 1;
        snap.stats.gates_passed = 1;
        snap.stats.errors_total = 1;

        let mut state = TuiState::default();
        state.update_from_dashboard_snapshot(&snap);

        assert_eq!(state.orchestrator_state, "running");
        assert_eq!(state.current_phase, "implementer");
        assert_eq!(state.plans.len(), 1);
        assert_eq!(state.plans[0].id, "plan-a");
        assert_eq!(state.plans[0].tasks_total, 2);
        assert_eq!(state.plans[0].tasks_done, 1);
        assert_eq!(state.plans[0].phase, "implementer");
        assert_eq!(state.plans[0].tasks.len(), 2);
        assert_eq!(state.current_task_checklist.len(), 2);
        assert_eq!(state.current_task_checklist[0].id, "task-1");
        assert_eq!(state.current_task_checklist[0].status, TaskStatus::Active);
        assert_eq!(state.current_task_checklist[1].status, TaskStatus::Done);
        assert_eq!(state.agents.len(), 1);
        assert_eq!(state.agents[0].id, "agent-1");
        assert_eq!(state.agents[0].role, "implementer");
        assert_eq!(state.agents[0].output_tokens, 42);
        assert_eq!(state.gate_results.len(), 1);
        assert_eq!(state.gate_results[0].gate, "compile");
        assert_eq!(state.gate_results[0].output, "task task-2");
        assert_eq!(state.diagnoses.len(), 1);
        assert_eq!(state.diagnoses[0].subject, "Circuit Breaker: Loop Detected");
        assert_eq!(state.experiment_winners.len(), 1);
        assert_eq!(state.experiment_winners[0].experiment_id, "exp-01");
        assert!(state.gate_trends.contains_key("compile"));
        assert_eq!(state.gate_recent_failures.len(), 1);
        assert_eq!(state.gate_recent_failures[0].task_id, "task-2");
        assert_eq!(state.execution_waves.len(), 1);
        assert_eq!(state.execution_waves[0].plans, vec![String::from("plan-a")]);
        assert_eq!(state.phase_pipeline.len(), 9);
        assert_eq!(state.phase_pipeline[2].status, PhaseStatus::Active);
    }

    #[test]
    fn update_from_dashboard_snapshot_preserves_navigation_state_by_id() {
        let mut state = TuiState::default();
        state.active_tab = Tab::Agents;
        state.focus = FocusZone::RightPanel;
        state.selected_plan_idx = 1;
        state.current_plan_idx = 1;
        state.selected_agent = 1;
        state.selected_agent_tab = 4;
        state.agent_scroll = Some(9);
        state.plan_scroll_offset = 12;
        state.log_scroll = 7;
        state.plans = vec![
            PlanEntry {
                id: "plan-a".into(),
                expanded: false,
                ..PlanEntry::default()
            },
            PlanEntry {
                id: "plan-b".into(),
                expanded: true,
                ..PlanEntry::default()
            },
        ];
        state.agents = vec![
            AgentRow {
                id: "agent-1".into(),
                ..AgentRow::default()
            },
            AgentRow {
                id: "agent-2".into(),
                ..AgentRow::default()
            },
        ];
        state.current_task_checklist = vec![TaskRow {
            id: "task-2".into(),
            title: "task-2".into(),
            status: TaskStatus::Active,
            elapsed_secs: 15.0,
        }];

        let mut snap = roko_core::DashboardSnapshot::default();
        snap.plans.insert(
            "plan-b".into(),
            roko_core::dashboard_snapshot::PlanState {
                plan_id: "plan-b".into(),
                phase: "implementer".into(),
                tasks_total: 1,
                tasks_done: 0,
                tasks_failed: 0,
                active: true,
            },
        );
        snap.plans.insert(
            "plan-a".into(),
            roko_core::dashboard_snapshot::PlanState {
                plan_id: "plan-a".into(),
                phase: "pending".into(),
                tasks_total: 0,
                tasks_done: 0,
                tasks_failed: 0,
                active: false,
            },
        );
        snap.tasks.insert(
            "plan-b/task-2".into(),
            roko_core::dashboard_snapshot::TaskState {
                task_id: "task-2".into(),
                plan_id: "plan-b".into(),
                phase: "implementer".into(),
                outcome: None,
            },
        );
        snap.agents.insert(
            "agent-2".into(),
            roko_core::dashboard_snapshot::AgentState {
                agent_id: "agent-2".into(),
                role: "reviewer".into(),
                active: true,
                output_bytes: 3,
                model: String::new(),
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: 0.0,
                current_task: String::new(),
                current_plan: String::new(),
            },
        );
        snap.agents.insert(
            "agent-1".into(),
            roko_core::dashboard_snapshot::AgentState {
                agent_id: "agent-1".into(),
                role: "implementer".into(),
                active: false,
                output_bytes: 0,
                model: String::new(),
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: 0.0,
                current_task: String::new(),
                current_plan: String::new(),
            },
        );
        snap.stats.plans_active = 1;

        state.update_from_dashboard_snapshot(&snap);

        assert_eq!(state.active_tab, Tab::Agents);
        assert_eq!(state.focus, FocusZone::RightPanel);
        assert_eq!(state.agent_scroll, Some(9));
        assert_eq!(state.plan_scroll_offset, 12);
        assert_eq!(state.log_scroll, 7);
        assert_eq!(state.selected_plan_idx, 1);
        assert_eq!(state.current_plan_idx, 1);
        assert_eq!(state.plans[state.selected_plan_idx].id, "plan-b");
        assert_eq!(state.selected_agent, 1);
        assert_eq!(state.agents[state.selected_agent].id, "agent-2");
        assert_eq!(state.selected_agent_tab, 4);
        assert!(state.plans[1].expanded);
        assert_eq!(state.current_task_checklist[0].elapsed_secs, 15.0);
    }

    #[test]
    fn smoothed_value_applies_ema() {
        let mut value = SmoothedValue::new(0.25);
        assert_eq!(value.update(100.0), 25.0);
        assert_eq!(value.update(100.0), 43.75);
    }
}
