//! Full TUI state container matching Mori's `RunState`.
//!
//! `TuiState` holds every piece of mutable state the interactive dashboard
//! needs: navigation, scroll positions, modal visibility, agent/plan data,
//! cost tracking, git state, and more.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use super::atmosphere::Atmosphere;
use super::dashboard::DashboardData;
use super::input::{ConfirmAction, FocusZone, InputMode, ModalVisibility};
use super::tabs::Tab;

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

/// Agent state tracked per active agent (legacy HashMap-based tracking).
#[derive(Debug, Clone, Default)]
pub struct AgentState {
    /// Agent identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Current status label (e.g. "running", "waiting", "done").
    pub status: String,
    /// Accumulated output lines.
    pub output_lines: Vec<String>,
    /// Latest diff content, if any.
    pub diff_content: String,
    /// Cumulative input tokens.
    pub input_tokens: u64,
    /// Cumulative output tokens.
    pub output_tokens: u64,
    /// Cached rendered output (invalidated on new output).
    pub render_cache: Option<String>,
    /// Plan this agent is working on, if known.
    pub plan_id: Option<String>,
    /// Task this agent is working on, if known.
    pub task_id: Option<String>,
}

/// Agent row for the Vec-based agent roster used by widgets.
///
/// Widgets index into `TuiState::agents` by position, and read fields
/// like `.active`, `.role`, `.model`, `.current_plan`, `.current_task`,
/// `.context_limit`, `.last_output_line` etc.
#[derive(Debug, Clone, Default)]
pub struct AgentRow {
    /// Agent identifier.
    pub id: String,
    /// Whether the agent is currently active / running.
    pub active: bool,
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
    /// Last line of agent output (for the output pane).
    pub last_output_line: String,
}

/// State for parallel agent display.
#[derive(Debug, Clone, Default)]
pub struct ParallelAgentState {
    pub agent_id: String,
    pub plan_id: String,
    pub task_id: String,
    pub status: String,
    pub progress_pct: f64,
}

/// A plan entry in the plan list.
///
/// Extended with fields required by the plan_tree, header_bar, status_bar,
/// and wave_progress widgets.
#[derive(Debug, Clone, Default)]
pub struct PlanEntry {
    pub id: String,
    pub name: String,
    pub status: String,
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
    // -- legacy aliases (kept for backward compatibility) --
    /// Legacy: total tasks (alias for tasks_total).
    pub task_total: usize,
    /// Legacy: done tasks (alias for tasks_done).
    pub task_done: usize,
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
    pub status: String,
    pub agent_id: Option<String>,
}

/// A log entry for the log viewer.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp_ms: i64,
    pub level: String,
    pub source: String,
    pub message: String,
}

/// Git branch tree node.
#[derive(Debug, Clone, Default)]
pub struct GitBranchNode {
    pub name: String,
    pub is_current: bool,
    pub ahead: usize,
    pub behind: usize,
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

/// Token burn history entry for cost tracking.
#[derive(Debug, Clone, Default)]
pub struct TokenBurnEntry {
    pub timestamp_ms: i64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub role: String,
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
    pub status: PhaseStatus,
    /// Elapsed seconds in this phase.
    pub elapsed_secs: f64,
    /// Completion percentage (0.0 .. 100.0).
    pub pct: f64,
}

/// Status of a phase pipeline step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PhaseStatus {
    #[default]
    Pending,
    Active,
    Done,
    Failed,
}

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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TaskRowStatus {
    #[default]
    Pending,
    Active,
    Done,
    Failed,
    Blocked,
}

/// A row in the task checklist widget.
#[derive(Debug, Clone, Default)]
pub struct TaskRow {
    /// Task identifier.
    pub id: String,
    /// Human-readable task title.
    pub title: String,
    /// Task status.
    pub status: TaskRowStatus,
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

    // -- agents (Vec-based roster for widgets) --
    /// Ordered agent roster for widgets (agent_pool, agent_output, header_bar).
    pub agents: Vec<AgentRow>,
    /// Legacy per-agent state keyed by agent ID.
    pub agents_by_id: HashMap<String, AgentState>,
    /// Parallel agents currently executing.
    pub parallel_agents: Vec<ParallelAgentState>,

    // -- navigation --
    /// Active top-level tab.
    pub active_tab: Tab,
    /// Selected plan index for the plan tree widget.
    pub selected_plan: usize,
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
    /// Agent output scroll (usize alias, 0 = auto-tail).
    pub output_scroll: usize,
    /// Diff panel scroll offset.
    pub diff_scroll: usize,
    /// Task list scroll offset.
    pub task_scroll: usize,
    /// Command output panel scroll offset.
    pub command_output_scroll: usize,
    /// Plan detail overlay scroll offset.
    pub plan_detail_scroll: usize,
    /// Plan summary scroll offset.
    pub plan_summary_scroll: usize,
    /// Plan list scroll offset (for long plan lists).
    pub plan_scroll_offset: usize,
    /// Plan tree scroll offset.
    pub plan_scroll: usize,
    /// Log viewer scroll offset.
    pub log_scroll: usize,
    /// Task detail overlay scroll offset.
    pub task_detail_scroll: usize,

    // -- modal visibility --
    /// Whether the plan detail overlay is open.
    pub show_plan_detail: bool,
    /// Whether the help overlay is open.
    pub show_help: bool,
    /// Whether the wave overview overlay is open.
    pub show_wave_overview: bool,
    /// Whether the agent pool modal is open.
    pub show_agent_pool_modal: bool,
    /// Whether the queue overview overlay is open.
    pub show_queue_overview: bool,
    /// Whether the task detail overlay is open.
    pub show_task_detail: bool,
    /// Whether the task picker modal is open.
    pub show_task_picker: bool,

    // -- approval / confirm --
    /// Pending agent command approval, if any.
    pub pending_approval: Option<PendingApproval>,
    /// Pending confirmation dialog action, if any.
    pub pending_confirm: Option<ConfirmAction>,

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
    pub git_view_data: Option<super::views::git_view::GitViewData>,

    /// Log messages for the log viewer.
    pub log_messages: Vec<LogEntry>,

    // -- plan detail --
    /// Content for the plan detail overlay.
    pub plan_detail_content: String,
    /// Active sub-tab in the plan detail overlay.
    pub plan_detail_tab: usize,
    /// Content for the plan summary view.
    pub plan_summary_content: String,

    // -- pipeline --
    /// Pipeline run state label.
    pub pipeline_run_state: String,
    /// Whether parallel execution is active.
    pub parallel_run: bool,

    // -- cost / tokens --
    /// Cumulative cost in USD across all agents.
    pub cumulative_cost_usd: f64,
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
    /// Current token burn rate (tokens per minute) for token_sparkline.
    pub token_rate: f64,
    /// Cumulative cost in USD for header_bar display.
    pub cost_dollars: f64,

    // -- token history --
    /// Token burn history per role (role -> entries) — legacy.
    pub token_burn_history: HashMap<String, Vec<TokenBurnEntry>>,
    /// Per-role token time-series for sparkline rendering (role -> sample ring).
    pub token_history: HashMap<String, VecDeque<u64>>,

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
    /// Viewport scroll offset for the config view.
    pub config_scroll_offset: usize,
    /// Unsaved edits: config key -> new value string.
    pub config_pending: HashMap<String, String>,
    /// Whether text-input mode is active for a config field.
    pub config_editing: bool,
    /// Text input buffer for the field being edited.
    pub config_edit_buffer: String,
    /// Which config key is currently being text-edited.
    pub config_edit_key: Option<String>,

    // -- agent pane --
    /// Active agent pane display group (cycles through available groups).
    pub agent_pane_group: usize,
}

impl Default for TuiState {
    fn default() -> Self {
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

            agents: Vec::new(),
            agents_by_id: HashMap::new(),
            parallel_agents: Vec::new(),

            active_tab: Tab::default(),
            selected_plan: 0,
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
            output_scroll: 0,
            diff_scroll: 0,
            task_scroll: 0,
            command_output_scroll: 0,
            plan_detail_scroll: 0,
            plan_summary_scroll: 0,
            plan_scroll_offset: 0,
            plan_scroll: 0,
            log_scroll: 0,
            task_detail_scroll: 0,

            show_plan_detail: false,
            show_help: false,
            show_wave_overview: false,
            show_agent_pool_modal: false,
            show_queue_overview: false,
            show_task_detail: false,
            show_task_picker: false,

            pending_approval: None,
            pending_confirm: None,

            git_branch: String::new(),
            git_commit_short: String::new(),
            git_age: String::new(),
            git_branch_tree: Vec::new(),
            git_commit_graph: Vec::new(),
            git_worktree_list: Vec::new(),
            git_branch_cursor: 0,
            git_summary_lines: Vec::new(),
            git_view_data: None,

            log_messages: Vec::new(),

            plan_detail_content: String::new(),
            plan_detail_tab: 0,
            plan_summary_content: String::new(),

            pipeline_run_state: String::from("idle"),
            parallel_run: false,

            cumulative_cost_usd: 0.0,
            cost_per_plan: HashMap::new(),
            cost_per_task: HashMap::new(),
            cumulative_input_tokens: 0,
            cumulative_output_tokens: 0,
            token_total: 0,
            token_rate: 0.0,
            cost_dollars: 0.0,

            token_burn_history: HashMap::new(),
            token_history: HashMap::new(),

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

    /// Return the modal visibility flags needed by key dispatch.
    #[must_use]
    pub const fn modal_visibility(&self) -> ModalVisibility {
        ModalVisibility {
            show_task_picker: self.show_task_picker,
            show_task_detail: self.show_task_detail,
            show_queue_overview: self.show_queue_overview,
            show_wave_overview: self.show_wave_overview,
            show_plan_detail: self.show_plan_detail,
            show_help: self.show_help,
        }
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

    // -- snapshot bridging ---------------------------------------------------

    /// Populate state from a `DashboardData` snapshot.
    ///
    /// This bridges the existing snapshot-based data model into the full
    /// TuiState. Fields not covered by `DashboardData` are left unchanged.
    pub fn update_from_snapshot(&mut self, data: &DashboardData) {
        let executor_summary = data.executor_summary();
        self.orchestrator_state = executor_summary.orchestrator_state;
        self.current_iteration = executor_summary.current_iteration;
        self.current_phase = executor_summary.current_phase;

        // Plans
        self.plans = data
            .plans
            .iter()
            .map(|p| {
                let completed = p.completed;
                let phase = if completed {
                    "done".to_string()
                } else {
                    "pending".to_string()
                };
                let tasks_done = if completed { p.task_count } else { 0 };
                PlanEntry {
                    id: p.id.clone(),
                    name: p.title.clone(),
                    status: phase.clone(),
                    active: !completed,
                    phase,
                    tasks_total: p.task_count,
                    tasks_done,
                    tasks_failed: 0,
                    elapsed_secs: 0.0,
                    wave: None,
                    task_total: p.task_count,
                    task_done: tasks_done,
                    expanded: false,
                    tasks: Vec::new(),
                }
            })
            .collect();

        // Agents — populate both Vec and HashMap
        self.agents.clear();
        self.agents_by_id.clear();
        for agent in &data.agents {
            let is_active = agent.status == "active" || agent.status == "running";
            self.agents.push(AgentRow {
                id: agent.id.clone(),
                active: is_active,
                role: agent.label.clone(),
                model: String::new(),
                input_tokens: 0,
                output_tokens: 0,
                context_limit: 200_000, // sensible default
                current_plan: agent.plan_id.clone().unwrap_or_default(),
                current_task: String::new(),
                last_output_line: String::new(),
            });
            self.agents_by_id.insert(
                agent.id.clone(),
                AgentState {
                    id: agent.id.clone(),
                    name: agent.label.clone(),
                    status: agent.status.clone(),
                    plan_id: agent.plan_id.clone(),
                    ..AgentState::default()
                },
            );
        }

        // Populate agent output from episodes (Task 2)
        for episode in data.episodes() {
            // Populate the HashMap-based agent state
            if let Some(agent) = self.agents_by_id.get_mut(&episode.agent_id) {
                let output_text = extract_episode_output(episode);
                if !output_text.is_empty() {
                    agent.output_lines = output_text.lines().map(String::from).collect();
                }
            }
            // Populate the Vec-based agent roster last_output_line
            if let Some(row) = self.agents.iter_mut().find(|a| a.id == episode.agent_id) {
                let output_text = extract_episode_output(episode);
                if let Some(last_line) = output_text.lines().last() {
                    row.last_output_line = last_line.to_string();
                }
                // Also populate model and task from episode
                if !episode.model.is_empty() {
                    row.model = episode.model.clone();
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
                if row.last_output_line.is_empty() {
                    if let Some(last) = lines.last() {
                        row.last_output_line = last.clone();
                    }
                }
            }
        }

        self.parallel_agents = self
            .agents
            .iter()
            .filter(|a| a.active)
            .map(|a| ParallelAgentState {
                agent_id: a.id.clone(),
                plan_id: a.current_plan.clone(),
                task_id: a.current_task.clone(),
                status: if a.active {
                    "running".to_string()
                } else {
                    "idle".to_string()
                },
                progress_pct: 0.0,
            })
            .collect();

        self.cumulative_cost_usd=data.efficiency.total_cost_usd;
        self.cumulative_input_tokens=data.efficiency.total_input_tokens;
        self.cumulative_output_tokens=data.efficiency.total_output_tokens;
        self.cost_dollars=self.cumulative_cost_usd;
        self.token_total=self.cumulative_input_tokens+self.cumulative_output_tokens;
        sum_costs(data,&mut self.cost_per_plan,&mut self.cost_per_task);

                                                                          self.phase_pipeline = build_phase_pipeline(&data.active_tasks);

        // Populate phase elapsed times from episodes (Task 7)
        populate_phase_elapsed(&mut self.phase_pipeline, data.episodes());

        // Build current_task_checklist from active_tasks + task-trackers (Task 3)
        self.current_task_checklist = build_task_checklist_from_execution(data);

        // Build execution_waves — group plans by wave if available, else wave 0
        self.execution_waves = build_execution_waves(&self.plans);

        // Sync filter alias
        self.filter = self.filter_text.clone();

        // Clamp selections
        if !self.plans.is_empty() {
            if self.selected_plan_idx >= self.plans.len() {
                self.selected_plan_idx = self.plans.len() - 1;
            }
            if self.selected_plan >= self.plans.len() {
                self.selected_plan = self.plans.len() - 1;
            }
            if self.current_plan_idx >= self.plans.len() {
                self.current_plan_idx = self.plans.len() - 1;
            }
        } else {
            self.selected_plan_idx = 0;
            self.selected_plan = 0;
            self.current_plan_idx = 0;
        }

        if !self.agents.is_empty() && self.selected_agent >= self.agents.len() {
            self.selected_agent = self.agents.len() - 1;
        }
    }

    /// Close all modal overlays and return to normal mode.
    pub fn dismiss_all_modals(&mut self) {
        self.show_plan_detail = false;
        self.show_help = false;
        self.show_wave_overview = false;
        self.show_agent_pool_modal = false;
        self.show_queue_overview = false;
        self.show_task_detail = false;
        self.show_task_picker = false;
        self.pending_confirm = None;
        if self.input_mode == InputMode::Confirm {
            self.input_mode = InputMode::Normal;
        }
    }

    /// Whether any modal overlay is currently visible.
    #[must_use]
    pub const fn has_modal(&self) -> bool {
        self.show_plan_detail
            || self.show_help
            || self.show_wave_overview
            || self.show_agent_pool_modal
            || self.show_queue_overview
            || self.show_task_detail
            || self.show_task_picker
    }

    /// Whether the state is in a text-input mode (inject or filter).
    #[must_use]
    pub const fn is_text_input(&self) -> bool {
        matches!(self.input_mode, InputMode::Inject | InputMode::Filter)
    }

    /// Reset all scroll positions to zero.
    pub fn reset_scrolls(&mut self) {
        self.agent_scroll = None;
        self.output_scroll = 0;
        self.diff_scroll = 0;
        self.task_scroll = 0;
        self.command_output_scroll = 0;
        self.plan_detail_scroll = 0;
        self.plan_summary_scroll = 0;
        self.plan_scroll_offset = 0;
        self.plan_scroll = 0;
        self.log_scroll = 0;
        self.task_detail_scroll = 0;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build the canonical 9-phase pipeline, inferring status from active tasks.
fn build_phase_pipeline(active_tasks: &[super::dashboard::TaskSummary]) -> Vec<PhaseStep> {
    // Determine which phase is currently active based on task statuses
    let active_statuses: Vec<&str> = active_tasks
        .iter()
        .filter(|t| t.status == "running" || t.status == "active" || t.status == "executing")
        .map(|t| t.status.as_str())
        .collect();

    let has_active = !active_statuses.is_empty();
    let all_done = active_tasks
        .iter()
        .all(|t| t.status == "done" || t.status == "completed" || t.status == "passed");

    CANONICAL_PHASES
        .iter()
        .enumerate()
        .map(|(i, &name)| {
            let status = if all_done && !active_tasks.is_empty() {
                PhaseStatus::Done
            } else if has_active {
                // Simple heuristic: phases before the midpoint are done,
                // one phase is active, rest are pending.
                // In a real implementation this would map task statuses to phases.
                let midpoint = CANONICAL_PHASES.len() / 3;
                if i < midpoint {
                    PhaseStatus::Done
                } else if i == midpoint {
                    PhaseStatus::Active
                } else {
                    PhaseStatus::Pending
                }
            } else {
                PhaseStatus::Pending
            };
            PhaseStep {
                name: name.to_string(),
                status,
                elapsed_secs: 0.0,
                pct: match status {
                    PhaseStatus::Done => 100.0,
                    PhaseStatus::Active => 50.0,
                    _ => 0.0,
                },
            }
        })
        .collect()
}

/// Build execution waves from plan entries. Groups by `wave` field if set,
/// otherwise places all plans in wave 0.
fn build_execution_waves(plans: &[PlanEntry]) -> Vec<Wave> {
    if plans.is_empty() {
        return Vec::new();
    }

    let has_waves = plans.iter().any(|p| p.wave.is_some());
    if !has_waves {
        // All plans in a single wave
        let done = plans
            .iter()
            .filter(|p| !p.active && p.phase != "failed")
            .count();
        return vec![Wave {
            index: 0,
            plans: plans.iter().map(|p| p.id.clone()).collect(),
            done,
            total: plans.len(),
            expanded: true,
        }];
    }

    // Group by wave index
    let mut wave_map: std::collections::BTreeMap<usize, Vec<String>> =
        std::collections::BTreeMap::new();
    for plan in plans {
        let wi = plan.wave.unwrap_or(0);
        wave_map.entry(wi).or_default().push(plan.id.clone());
    }

    wave_map
        .into_iter()
        .map(|(idx, plan_ids)| {
            let done = plan_ids
                .iter()
                .filter(|pid| {
                    plans
                        .iter()
                        .any(|p| &p.id == *pid && !p.active && p.phase != "failed")
                })
                .count();
            Wave {
                index: idx,
                plans: plan_ids.clone(),
                done,
                total: plan_ids.len(),
                expanded: true,
            }
        })
        .collect()
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

fn sum_costs(data:&DashboardData,plans:&mut HashMap<String,f64>,tasks:&mut HashMap<String,f64>){
    plans.clear();
    tasks.clear();
    for e in &data.efficiency_events {
        if !e.plan_id.is_empty(){*plans.entry(e.plan_id.clone()).or_default()+=e.cost_usd;}
        if !e.task_id.is_empty(){*tasks.entry(e.task_id.clone()).or_default()+=e.cost_usd;}
    }
}

fn build_task_checklist_from_execution(data:&DashboardData)->Vec<TaskRow>{
    if let Some(exec)=&data.current_plan_execution {
                           return exec
            .tasks
            .iter()
            .map(|t| {
                let status = match t.phase.to_ascii_lowercase().as_str() {
                    "done" => TaskRowStatus::Done,
                    "failed" => TaskRowStatus::Failed,
                    "implementing"
                    | "gating"
                    | "verifying"
                    | "reviewing"
                    | "doc revision"
                    | "auto fixing"
                    | "regenerating verify" => TaskRowStatus::Active,
                    "queued" => TaskRowStatus::Pending,
                    _ if t.is_current => TaskRowStatus::Active,
                    _ => TaskRowStatus::Pending,
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
            let status = match t.status.as_str() {
                "done" | "completed" | "passed" => TaskRowStatus::Done,
                "running" | "active" | "executing" => TaskRowStatus::Active,
                "failed" | "error" => TaskRowStatus::Failed,
                "blocked" => TaskRowStatus::Blocked,
                _ => TaskRowStatus::Pending,
            };
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

    use super::*;
    use tempfile::tempdir;

    #[test]
    fn default_state_is_idle_dashboard() {
        let state = TuiState::default();
        assert_eq!(state.active_tab, Tab::Dashboard);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert_eq!(state.focus, FocusZone::PlanTree);
        assert_eq!(state.orchestrator_state, "idle");
        assert!(!state.has_modal());
        assert!(!state.is_text_input());
    }

    #[test]
    fn modal_visibility_reflects_state() {
        let mut state = TuiState::default();
        assert!(!state.has_modal());

        state.show_task_picker = true;
        assert!(state.has_modal());

        let vis = state.modal_visibility();
        assert!(vis.show_task_picker);
        assert!(!vis.show_task_detail);
    }

    #[test]
    fn dismiss_all_modals_clears_everything() {
        let mut state = TuiState::default();
        state.show_plan_detail = true;
        state.show_help = true;
        state.show_task_picker = true;
        state.pending_confirm = Some(ConfirmAction::RestartAllPlans);
        state.input_mode = InputMode::Confirm;

        state.dismiss_all_modals();

        assert!(!state.has_modal());
        assert!(state.pending_confirm.is_none());
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn reset_scrolls_zeroes_all() {
        let mut state = TuiState::default();
        state.agent_scroll = Some(50);
        state.diff_scroll = 10;
        state.log_scroll = 100;

        state.reset_scrolls();

        assert_eq!(state.agent_scroll, None);
        assert_eq!(state.diff_scroll, 0);
        assert_eq!(state.log_scroll, 0);
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
    fn new_fields_have_defaults() {
        let state = TuiState::default();
        assert!(state.phase_pipeline.is_empty());
        assert!(state.execution_waves.is_empty());
        assert!(state.current_task_checklist.is_empty());
        assert_eq!(state.sys.cpu_pct, 0.0);
        assert!(state.token_history.is_empty());
        assert_eq!(state.token_total, 0);
        assert_eq!(state.token_rate, 0.0);
        assert_eq!(state.cost_dollars, 0.0);
        assert!(state.git_commit_short.is_empty());
        assert!(state.git_age.is_empty());
        assert!(state.run_started.is_none());
        assert!(state.filter.is_empty());
        assert_eq!(state.selected_plan, 0);
        assert_eq!(state.selected_agent, 0);
        assert_eq!(state.output_scroll, 0);
        assert_eq!(state.plan_scroll, 0);
    }
}
