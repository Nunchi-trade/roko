//! Full TUI state container matching Mori's `RunState`.
//!
//! `TuiState` holds every piece of mutable state the interactive dashboard
//! needs: navigation, scroll positions, modal visibility, agent/plan data,
//! cost tracking, git state, and more.

use std::collections::HashMap;

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

/// Agent state tracked per active agent.
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
#[derive(Debug, Clone, Default)]
pub struct PlanEntry {
    pub id: String,
    pub name: String,
    pub status: String,
    pub task_total: usize,
    pub task_done: usize,
    pub expanded: bool,
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

/// A notification message shown in the TUI.
#[derive(Debug, Clone)]
pub struct Notification {
    pub message: String,
    pub level: NotificationLevel,
    pub timestamp_ms: i64,
}

/// Notification severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationLevel {
    Info,
    Warning,
    Error,
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

    // -- agents --
    /// Per-agent state keyed by agent ID.
    pub agents: HashMap<String, AgentState>,
    /// Parallel agents currently executing.
    pub parallel_agents: Vec<ParallelAgentState>,

    // -- navigation --
    /// Active top-level tab.
    pub active_tab: Tab,
    /// Selected plan index (may differ from current_plan_idx during browsing).
    pub selected_plan_idx: usize,
    /// Selected agent sub-tab index.
    pub selected_agent_tab: usize,
    /// Which panel has keyboard focus.
    pub focus: FocusZone,

    // -- input --
    /// Current input mode (normal, inject, filter, confirm).
    pub input_mode: InputMode,
    /// Text buffer for inject mode.
    pub message_input: String,
    /// Text buffer for filter mode.
    pub filter_text: String,
    /// Whether filter is actively applied.
    pub filter_active: bool,

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
    /// Plan summary scroll offset.
    pub plan_summary_scroll: usize,
    /// Plan list scroll offset (for long plan lists).
    pub plan_scroll_offset: usize,
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
    /// Git branch tree for the Git tab.
    pub git_branch_tree: Vec<GitBranchNode>,
    /// Git commit graph entries.
    pub git_commit_graph: Vec<GitCommitEntry>,
    /// Git worktree list entries.
    pub git_worktree_list: Vec<String>,
    /// Cursor position in the git branch tree.
    pub git_branch_cursor: usize,

    // -- notifications --
    /// Active notification messages.
    pub notifications: Vec<Notification>,
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

    // -- token history --
    /// Token burn history per role (role -> entries).
    pub token_burn_history: HashMap<String, Vec<TokenBurnEntry>>,

    // -- wave navigation --
    /// Selected wave index for wave prev/next navigation.
    pub selected_wave_idx: usize,

    // -- config navigation --
    /// Selected row in the config view.
    pub config_selected: usize,
    /// Scroll offset in the config view.
    pub config_scroll: usize,

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

            agents: HashMap::new(),
            parallel_agents: Vec::new(),

            active_tab: Tab::default(),
            selected_plan_idx: 0,
            selected_agent_tab: 0,
            focus: FocusZone::default(),

            input_mode: InputMode::default(),
            message_input: String::new(),
            filter_text: String::new(),
            filter_active: false,

            agent_scroll: None,
            diff_scroll: 0,
            task_scroll: 0,
            command_output_scroll: 0,
            plan_detail_scroll: 0,
            plan_summary_scroll: 0,
            plan_scroll_offset: 0,
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
            git_branch_tree: Vec::new(),
            git_commit_graph: Vec::new(),
            git_worktree_list: Vec::new(),
            git_branch_cursor: 0,

            notifications: Vec::new(),
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

            token_burn_history: HashMap::new(),

            selected_wave_idx: 0,

            config_selected: 0,
            config_scroll: 0,

            agent_pane_group: 0,
        }
    }
}

impl TuiState {
    /// Create a new default state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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

    /// Populate state from a `DashboardData` snapshot.
    ///
    /// This bridges the existing snapshot-based data model into the full
    /// TuiState. Fields not covered by `DashboardData` are left unchanged.
    pub fn update_from_snapshot(&mut self, data: &DashboardData) {
        // Plans
        self.plans = data
            .plans
            .iter()
            .map(|p| {
                let status = if p.completed {
                    "done".to_string()
                } else {
                    "pending".to_string()
                };
                PlanEntry {
                    id: p.id.clone(),
                    name: p.title.clone(),
                    status,
                    task_total: p.task_count,
                    task_done: if p.completed { p.task_count } else { 0 },
                    expanded: false,
                    tasks: Vec::new(),
                }
            })
            .collect();

        // Agents
        self.agents.clear();
        for agent in &data.agents {
            self.agents.insert(
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

        // Cost from efficiency summary
        self.cumulative_cost_usd = data.efficiency.total_cost_usd;
        self.cumulative_input_tokens = data.efficiency.total_input_tokens;
        self.cumulative_output_tokens = data.efficiency.total_output_tokens;

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
        self.diff_scroll = 0;
        self.task_scroll = 0;
        self.command_output_scroll = 0;
        self.plan_detail_scroll = 0;
        self.plan_summary_scroll = 0;
        self.plan_scroll_offset = 0;
        self.log_scroll = 0;
        self.task_detail_scroll = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
