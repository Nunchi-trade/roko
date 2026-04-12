//! Input mode state machine and key dispatch for the interactive TUI.
//!
//! Provides the full `TuiAction` enum (matching Mori's action vocabulary),
//! `InputMode` state machine, `FocusZone` for panel focus, and the
//! `handle_key` dispatch function with modal intercept priority.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::tabs::Tab;

// ---------------------------------------------------------------------------
// InputMode
// ---------------------------------------------------------------------------

/// Modal input state. Determines how keystrokes are interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InputMode {
    /// Standard navigation: arrow keys, tab switching, selection.
    #[default]
    Normal,
    /// Free-text injection: typing a message to send to an agent.
    Inject,
    /// Filter mode: typing a filter string for logs/signals.
    Filter,
    /// Confirmation dialog: yes/no prompt for destructive actions.
    Confirm,
}

// ---------------------------------------------------------------------------
// FocusZone
// ---------------------------------------------------------------------------

/// Which panel currently has keyboard focus in split-pane views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FocusZone {
    /// Plan tree / plan list (left panel).
    #[default]
    PlanTree,
    /// Task progress list.
    TaskProgress,
    /// Agent output / log pane.
    AgentOutput,
    /// Command output / bottom pane.
    CommandOutput,
    /// Right detail panel.
    RightPanel,
}

impl FocusZone {
    /// Cycle to the next focus zone.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::PlanTree => Self::TaskProgress,
            Self::TaskProgress => Self::AgentOutput,
            Self::AgentOutput => Self::CommandOutput,
            Self::CommandOutput => Self::RightPanel,
            Self::RightPanel => Self::PlanTree,
        }
    }

    /// Cycle to the previous focus zone.
    #[must_use]
    pub const fn prev(self) -> Self {
        match self {
            Self::PlanTree => Self::RightPanel,
            Self::TaskProgress => Self::PlanTree,
            Self::AgentOutput => Self::TaskProgress,
            Self::CommandOutput => Self::AgentOutput,
            Self::RightPanel => Self::CommandOutput,
        }
    }
}

// ---------------------------------------------------------------------------
// ConfirmAction
// ---------------------------------------------------------------------------

/// Destructive or significant actions that require user confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    RestartAllPlans,
    RestartPhase,
    ResetSelectedPlan(String),
    ForceAdvance(String),
    ReverifyPlan(String),
    DiagnosePlan(String),
    RepairPlanPreserve(String),
    RepairPlanClean(String),
    SoftRetryPlan(String),
    GitReconcile,
    IngestTask {
        plan_num: usize,
        task_id: String,
    },
    MergeBatchToMain {
        plan_id: String,
        branch: String,
    },
    MergePlan {
        plan_id: String,
        branch: String,
    },
    MergeAllDone {
        branches: Vec<String>,
    },
}

impl std::fmt::Display for ConfirmAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RestartAllPlans => write!(f, "Restart all plans?"),
            Self::RestartPhase => write!(f, "Restart current phase?"),
            Self::ResetSelectedPlan(id) => write!(f, "Reset plan {id}?"),
            Self::ForceAdvance(id) => write!(f, "Force-advance plan {id}?"),
            Self::ReverifyPlan(id) => write!(f, "Re-verify plan {id}?"),
            Self::DiagnosePlan(id) => write!(f, "Diagnose plan {id}?"),
            Self::RepairPlanPreserve(id) => write!(f, "Repair plan {id} (preserve)?"),
            Self::RepairPlanClean(id) => write!(f, "Repair plan {id} (clean)?"),
            Self::SoftRetryPlan(id) => write!(f, "Soft-retry plan {id}?"),
            Self::GitReconcile => write!(f, "Reconcile git state?"),
            Self::IngestTask { plan_num, task_id } => {
                write!(f, "Ingest task {task_id} into plan {plan_num}?")
            }
            Self::MergeBatchToMain { plan_id, branch } => {
                write!(f, "Merge {branch} (plan {plan_id}) to main?")
            }
            Self::MergePlan { plan_id, branch } => {
                write!(f, "Merge plan {plan_id} branch {branch}?")
            }
            Self::MergeAllDone { branches } => {
                write!(f, "Merge {} completed branches to main?", branches.len())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// TuiAction
// ---------------------------------------------------------------------------

/// Every possible user action the TUI can dispatch.
///
/// Matches Mori's action vocabulary so that key bindings, mouse events, and
/// programmatic triggers all flow through a single enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiAction {
    // -- lifecycle --
    Quit,

    // -- tab navigation --
    SwitchTab(Tab),

    // -- plan list navigation --
    SelectPlanUp,
    SelectPlanDown,

    // -- log scrolling --
    ScrollLogUp,
    ScrollLogDown,

    // -- agent tab --
    SwitchAgentTab(usize),

    // -- approval --
    ApproveCommand,
    ApproveAll,
    RejectCommand,

    // -- inject mode --
    StartInject,
    SubmitInject,
    CancelInject,
    InputChar(char),
    InputBackspace,

    // -- help --
    ShowHelp,

    // -- focus --
    FocusNext,
    FocusPrev,
    ScrollFocusedUp,
    ScrollFocusedDown,

    // -- expand / collapse --
    ExpandCollapse,

    // -- plan detail modal --
    ShowPlanDetail,
    ClosePlanDetail,
    ScrollDetailUp,
    ScrollDetailDown,

    // -- agent scrolling --
    ScrollAgentUp,
    ScrollAgentDown,
    ScrollAgentEnd,

    // -- diff scrolling --
    ScrollDiffUp,
    ScrollDiffDown,

    // -- plan operations --
    RestartPhase,
    RestartPlan,

    // -- detail tabs --
    SwitchDetailTab(usize),

    // -- agent pane --
    ToggleAgentPaneGroup,

    // -- notifications --
    DismissNotification,

    // -- config navigation --
    ConfigUp,
    ConfigDown,
    ConfigLeft,
    ConfigRight,
    ConfigSelect,

    // -- force / reset --
    ForceAdvance,
    ResetPlanState,
    ReverifyPlan,

    // -- confirm dialog --
    RequestConfirm(ConfirmAction),
    ConfirmYes,
    ConfirmNo,

    // -- pause --
    TogglePause,

    // -- wave / queue overviews --
    ShowWaveOverview,
    ShowQueueOverview,

    // -- filter mode --
    StartFilter,
    AcceptFilter,
    CancelFilter,

    // -- tree collapse --
    CollapseExpand,

    // -- task detail modal --
    ShowTaskDetail,
    CloseTaskDetail,

    // -- task picker modal --
    OpenTaskPicker,
    CloseTaskPicker,

    // -- drill navigation --
    DrillIn,
    DrillOut,

    // -- wave navigation --
    WaveNext,
    WavePrev,

    // -- mouse events --
    MouseClick { x: u16, y: u16 },
    MouseScrollUp { x: u16, y: u16 },
    MouseScrollDown { x: u16, y: u16 },

    // -- refresh --
    Refresh,

    // -- no-op --
    None,
}

// ---------------------------------------------------------------------------
// Key dispatch
// ---------------------------------------------------------------------------

/// Top-level key dispatch with modal intercept priority.
///
/// Priority order (highest first):
/// 1. Task picker modal
/// 2. Task detail modal
/// 3. Queue overview modal
/// 4. Confirm dialog
/// 5. Inject / filter text input
/// 6. Normal per-tab navigation
pub fn handle_key(key: KeyEvent, mode: InputMode, active_tab: Tab, focus: FocusZone, modals: &ModalVisibility) -> TuiAction {
    // Modal intercepts (highest priority first)
    if modals.show_task_picker {
        return handle_task_picker_key(key);
    }
    if modals.show_task_detail {
        return handle_task_detail_key(key);
    }
    if modals.show_queue_overview {
        return handle_queue_overview_key(key);
    }

    // Confirm dialog
    if mode == InputMode::Confirm {
        return handle_confirm_key(key);
    }

    // Text input modes
    if mode == InputMode::Inject {
        return handle_inject_key(key);
    }
    if mode == InputMode::Filter {
        return handle_filter_key(key);
    }

    // Global keys that work in any tab
    if let Some(action) = handle_global_key(key) {
        return action;
    }

    // Per-tab dispatch
    match active_tab {
        Tab::Dashboard => handle_dashboard_key(key, focus),
        Tab::Plans => handle_plans_key(key, focus),
        Tab::Agents => handle_agents_key(key, focus),
        Tab::Git => handle_git_key(key, focus),
        Tab::Logs => handle_logs_key(key, focus),
        Tab::Config => handle_config_key(key),
        Tab::Inspect => handle_inspect_key(key, focus),
    }
}

/// Subset of TuiState modal flags needed by key dispatch.
#[derive(Debug, Clone, Copy, Default)]
pub struct ModalVisibility {
    pub show_task_picker: bool,
    pub show_task_detail: bool,
    pub show_queue_overview: bool,
    pub show_wave_overview: bool,
    pub show_plan_detail: bool,
    pub show_help: bool,
}

// ---------------------------------------------------------------------------
// Modal key handlers
// ---------------------------------------------------------------------------

fn handle_task_picker_key(key: KeyEvent) -> TuiAction {
    match key.code {
        KeyCode::Esc => TuiAction::CloseTaskPicker,
        KeyCode::Enter => TuiAction::ShowTaskDetail,
        KeyCode::Up | KeyCode::Char('k') => TuiAction::SelectPlanUp,
        KeyCode::Down | KeyCode::Char('j') => TuiAction::SelectPlanDown,
        _ => TuiAction::None,
    }
}

fn handle_task_detail_key(key: KeyEvent) -> TuiAction {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => TuiAction::CloseTaskDetail,
        KeyCode::Up | KeyCode::Char('k') => TuiAction::ScrollDetailUp,
        KeyCode::Down | KeyCode::Char('j') => TuiAction::ScrollDetailDown,
        KeyCode::Tab => TuiAction::SwitchDetailTab(0), // next detail sub-tab
        _ => TuiAction::None,
    }
}

fn handle_queue_overview_key(key: KeyEvent) -> TuiAction {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => TuiAction::ShowQueueOverview, // toggle off
        KeyCode::Up | KeyCode::Char('k') => TuiAction::ScrollFocusedUp,
        KeyCode::Down | KeyCode::Char('j') => TuiAction::ScrollFocusedDown,
        _ => TuiAction::None,
    }
}

fn handle_confirm_key(key: KeyEvent) -> TuiAction {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => TuiAction::ConfirmYes,
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => TuiAction::ConfirmNo,
        _ => TuiAction::None,
    }
}

fn handle_inject_key(key: KeyEvent) -> TuiAction {
    match key.code {
        KeyCode::Enter => TuiAction::SubmitInject,
        KeyCode::Esc => TuiAction::CancelInject,
        KeyCode::Backspace => TuiAction::InputBackspace,
        KeyCode::Char(c) => TuiAction::InputChar(c),
        _ => TuiAction::None,
    }
}

fn handle_filter_key(key: KeyEvent) -> TuiAction {
    match key.code {
        KeyCode::Enter => TuiAction::AcceptFilter,
        KeyCode::Esc => TuiAction::CancelFilter,
        KeyCode::Backspace => TuiAction::InputBackspace,
        KeyCode::Char(c) => TuiAction::InputChar(c),
        _ => TuiAction::None,
    }
}

// ---------------------------------------------------------------------------
// Global keys
// ---------------------------------------------------------------------------

fn handle_global_key(key: KeyEvent) -> Option<TuiAction> {
    // Ctrl-C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(TuiAction::Quit);
    }

    // F-keys switch tabs
    if let Some(tab) = Tab::from_key(key.code) {
        return Some(TuiAction::SwitchTab(tab));
    }

    match key.code {
        KeyCode::Char('q') => Some(TuiAction::Quit),
        KeyCode::Char('?') => Some(TuiAction::ShowHelp),
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(TuiAction::Refresh)
        }
        KeyCode::Tab => Some(TuiAction::FocusNext),
        KeyCode::BackTab => Some(TuiAction::FocusPrev),
        _ => Option::None,
    }
}

// ---------------------------------------------------------------------------
// Per-tab key handlers
// ---------------------------------------------------------------------------

fn handle_dashboard_key(key: KeyEvent, _focus: FocusZone) -> TuiAction {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => TuiAction::ScrollFocusedUp,
        KeyCode::Down | KeyCode::Char('j') => TuiAction::ScrollFocusedDown,
        KeyCode::Enter => TuiAction::ShowPlanDetail,
        KeyCode::Char('w') => TuiAction::ShowWaveOverview,
        KeyCode::Char('p') => TuiAction::TogglePause,
        KeyCode::Char('n') => TuiAction::DismissNotification,
        _ => TuiAction::None,
    }
}

fn handle_plans_key(key: KeyEvent, focus: FocusZone) -> TuiAction {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => match focus {
            FocusZone::PlanTree => TuiAction::SelectPlanUp,
            _ => TuiAction::ScrollFocusedUp,
        },
        KeyCode::Down | KeyCode::Char('j') => match focus {
            FocusZone::PlanTree => TuiAction::SelectPlanDown,
            _ => TuiAction::ScrollFocusedDown,
        },
        KeyCode::Enter => TuiAction::ShowPlanDetail,
        KeyCode::Esc => TuiAction::ClosePlanDetail,
        KeyCode::Char('e') => TuiAction::ExpandCollapse,
        KeyCode::Char('w') => TuiAction::ShowWaveOverview,
        KeyCode::Char('o') => TuiAction::ShowQueueOverview,
        KeyCode::Char('t') => TuiAction::OpenTaskPicker,
        KeyCode::Char('[') => TuiAction::WavePrev,
        KeyCode::Char(']') => TuiAction::WaveNext,
        KeyCode::Left | KeyCode::Char('h') => TuiAction::DrillOut,
        KeyCode::Right | KeyCode::Char('l') => TuiAction::DrillIn,
        KeyCode::Char('R') => TuiAction::RestartPlan,
        KeyCode::Char('F') => TuiAction::ForceAdvance,
        KeyCode::Char('V') => TuiAction::ReverifyPlan,
        _ => TuiAction::None,
    }
}

fn handle_agents_key(key: KeyEvent, focus: FocusZone) -> TuiAction {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => match focus {
            FocusZone::AgentOutput => TuiAction::ScrollAgentUp,
            FocusZone::RightPanel => TuiAction::ScrollDiffUp,
            _ => TuiAction::ScrollFocusedUp,
        },
        KeyCode::Down | KeyCode::Char('j') => match focus {
            FocusZone::AgentOutput => TuiAction::ScrollAgentDown,
            FocusZone::RightPanel => TuiAction::ScrollDiffDown,
            _ => TuiAction::ScrollFocusedDown,
        },
        KeyCode::End | KeyCode::Char('G') => TuiAction::ScrollAgentEnd,
        KeyCode::Char('1') => TuiAction::SwitchAgentTab(0),
        KeyCode::Char('2') => TuiAction::SwitchAgentTab(1),
        KeyCode::Char('3') => TuiAction::SwitchAgentTab(2),
        KeyCode::Char('4') => TuiAction::SwitchAgentTab(3),
        KeyCode::Char('a') => TuiAction::ApproveCommand,
        KeyCode::Char('A') => TuiAction::ApproveAll,
        KeyCode::Char('x') => TuiAction::RejectCommand,
        KeyCode::Char('i') => TuiAction::StartInject,
        KeyCode::Char('g') => TuiAction::ToggleAgentPaneGroup,
        _ => TuiAction::None,
    }
}

fn handle_git_key(key: KeyEvent, _focus: FocusZone) -> TuiAction {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => TuiAction::ScrollFocusedUp,
        KeyCode::Down | KeyCode::Char('j') => TuiAction::ScrollFocusedDown,
        KeyCode::Left | KeyCode::Char('h') => TuiAction::DrillOut,
        KeyCode::Right | KeyCode::Char('l') => TuiAction::DrillIn,
        KeyCode::Enter => TuiAction::ExpandCollapse,
        _ => TuiAction::None,
    }
}

fn handle_logs_key(key: KeyEvent, _focus: FocusZone) -> TuiAction {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => TuiAction::ScrollLogUp,
        KeyCode::Down | KeyCode::Char('j') => TuiAction::ScrollLogDown,
        KeyCode::Char('/') => TuiAction::StartFilter,
        KeyCode::End | KeyCode::Char('G') => TuiAction::ScrollAgentEnd,
        _ => TuiAction::None,
    }
}

fn handle_config_key(key: KeyEvent) -> TuiAction {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => TuiAction::ConfigUp,
        KeyCode::Down | KeyCode::Char('j') => TuiAction::ConfigDown,
        KeyCode::Left | KeyCode::Char('h') => TuiAction::ConfigLeft,
        KeyCode::Right | KeyCode::Char('l') => TuiAction::ConfigRight,
        KeyCode::Enter => TuiAction::ConfigSelect,
        _ => TuiAction::None,
    }
}

fn handle_inspect_key(key: KeyEvent, _focus: FocusZone) -> TuiAction {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => TuiAction::ScrollFocusedUp,
        KeyCode::Down | KeyCode::Char('j') => TuiAction::ScrollFocusedDown,
        KeyCode::Left | KeyCode::Char('h') => TuiAction::DrillOut,
        KeyCode::Right | KeyCode::Char('l') => TuiAction::DrillIn,
        KeyCode::Enter => TuiAction::ExpandCollapse,
        _ => TuiAction::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    fn key_with_mod(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    fn modals() -> ModalVisibility {
        ModalVisibility::default()
    }

    #[test]
    fn ctrl_c_always_quits() {
        let action = handle_key(
            key_with_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
            InputMode::Normal,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &modals(),
        );
        assert_eq!(action, TuiAction::Quit);
    }

    #[test]
    fn f_keys_switch_tabs() {
        let action = handle_key(
            key(KeyCode::F(3)),
            InputMode::Normal,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &modals(),
        );
        assert_eq!(action, TuiAction::SwitchTab(Tab::Agents));
    }

    #[test]
    fn confirm_mode_intercepts() {
        let action = handle_key(
            key(KeyCode::Char('y')),
            InputMode::Confirm,
            Tab::Plans,
            FocusZone::PlanTree,
            &modals(),
        );
        assert_eq!(action, TuiAction::ConfirmYes);
    }

    #[test]
    fn inject_mode_captures_chars() {
        let action = handle_key(
            key(KeyCode::Char('x')),
            InputMode::Inject,
            Tab::Agents,
            FocusZone::AgentOutput,
            &modals(),
        );
        assert_eq!(action, TuiAction::InputChar('x'));
    }

    #[test]
    fn filter_mode_enter_accepts() {
        let action = handle_key(
            key(KeyCode::Enter),
            InputMode::Filter,
            Tab::Logs,
            FocusZone::PlanTree,
            &modals(),
        );
        assert_eq!(action, TuiAction::AcceptFilter);
    }

    #[test]
    fn task_picker_modal_intercepts() {
        let mut m = modals();
        m.show_task_picker = true;
        let action = handle_key(
            key(KeyCode::Esc),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::PlanTree,
            &m,
        );
        assert_eq!(action, TuiAction::CloseTaskPicker);
    }

    #[test]
    fn plans_tab_focus_routing() {
        let action = handle_key(
            key(KeyCode::Up),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::PlanTree,
            &modals(),
        );
        assert_eq!(action, TuiAction::SelectPlanUp);

        let action = handle_key(
            key(KeyCode::Up),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::AgentOutput,
            &modals(),
        );
        assert_eq!(action, TuiAction::ScrollFocusedUp);
    }

    #[test]
    fn agents_tab_diff_scroll() {
        let action = handle_key(
            key(KeyCode::Down),
            InputMode::Normal,
            Tab::Agents,
            FocusZone::RightPanel,
            &modals(),
        );
        assert_eq!(action, TuiAction::ScrollDiffDown);
    }
}
