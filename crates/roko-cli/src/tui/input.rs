//! Input mode state machine and key dispatch for the interactive TUI.
//!
//! Provides the full `TuiAction` enum (matching Mori's action vocabulary),
//! `InputMode` state machine, `FocusZone` for panel focus, and the
//! `handle_key` dispatch function with modal intercept priority.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::modals::ModalState;
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
    /// Config text-edit mode: typing a value for a config field.
    ConfigEdit,
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
    pub const fn next(self, tab: Tab) -> Self {
        match tab {
            Tab::Dashboard => match self {
                Self::PlanTree => Self::TaskProgress,
                Self::TaskProgress => Self::AgentOutput,
                Self::AgentOutput => Self::CommandOutput,
                Self::CommandOutput => Self::RightPanel,
                Self::RightPanel => Self::PlanTree,
            },
            Tab::Plans => match self {
                Self::PlanTree | Self::TaskProgress | Self::AgentOutput | Self::CommandOutput => {
                    Self::RightPanel
                }
                Self::RightPanel => Self::PlanTree,
            },
            Tab::Agents => match self {
                Self::PlanTree | Self::TaskProgress | Self::CommandOutput | Self::RightPanel => {
                    Self::AgentOutput
                }
                Self::AgentOutput => Self::RightPanel,
            },
            Tab::Git | Tab::Logs | Tab::Config | Tab::Inspect => self,
        }
    }

    /// Cycle to the previous focus zone.
    #[must_use]
    pub const fn prev(self, tab: Tab) -> Self {
        match tab {
            Tab::Dashboard => match self {
                Self::PlanTree => Self::RightPanel,
                Self::TaskProgress => Self::PlanTree,
                Self::AgentOutput => Self::TaskProgress,
                Self::CommandOutput => Self::AgentOutput,
                Self::RightPanel => Self::CommandOutput,
            },
            Tab::Plans => match self {
                Self::PlanTree | Self::TaskProgress | Self::AgentOutput | Self::CommandOutput => {
                    Self::RightPanel
                }
                Self::RightPanel => Self::PlanTree,
            },
            Tab::Agents => match self {
                Self::PlanTree | Self::TaskProgress | Self::CommandOutput | Self::RightPanel => {
                    Self::AgentOutput
                }
                Self::AgentOutput => Self::RightPanel,
            },
            Tab::Git | Tab::Logs | Tab::Config | Tab::Inspect => self,
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
    IngestTask { plan_num: usize, task_id: String },
    MergeBatchToMain { plan_id: String, branch: String },
    MergePlan { plan_id: String, branch: String },
    MergeAllDone { branches: Vec<String> },
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

/// Logs tab filter levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogFilterLevel {
    Info,
    Warn,
    Error,
    Debug,
}

impl LogFilterLevel {
    #[must_use]
    pub const fn all() -> [Self; 4] {
        [Self::Info, Self::Warn, Self::Error, Self::Debug]
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Info => "INF",
            Self::Warn => "WRN",
            Self::Error => "ERR",
            Self::Debug => "DBG",
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
    QuitConfirmed,

    // -- tab navigation --
    SwitchTab(Tab),

    // -- plan list navigation --
    SelectPlanUp,
    SelectPlanDown,
    TaskPickerUp,
    TaskPickerDown,

    // -- log scrolling --
    ScrollLogUp,
    ScrollLogDown,
    ScrollLogEnd,
    ToggleLogFilter(LogFilterLevel),
    ShowAllLogFilters,

    // -- agent tab --
    SwitchAgentTab(usize),
    ToggleAgentTopology,

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
    /// Cycle the visual-effects preset.
    CycleEffectsPreset,
    ToggleScreenPostFx,

    // -- focus --
    FocusNext,
    FocusPrev,
    ScrollFocusedUp,
    ScrollFocusedDown,
    ScrollPageUp,
    ScrollPageDown,
    ScrollFocusedHome,
    ScrollFocusedEnd,

    // -- expand / collapse --
    ExpandCollapse,

    // -- plan detail modal --
    ShowPlanDetail,
    ClosePlanDetail,
    ScrollDetailUp,
    ScrollDetailDown,
    ModalScrollUp,
    ModalScrollDown,
    QueueOverviewUp,
    QueueOverviewDown,
    CloseModal,

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

    // -- config editor --
    ConfigUp,
    ConfigDown,
    ConfigToggle,
    ConfigCycleLeft,
    ConfigCycleRight,
    ConfigCommitEdit,
    ConfigCancelEdit,
    ConfigSave,

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
    MouseClick {
        x: u16,
        y: u16,
    },
    MouseScrollUp {
        x: u16,
        y: u16,
    },
    MouseScrollDown {
        x: u16,
        y: u16,
    },

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
/// 1. Help / approval / detail modals
/// 2. Confirm dialog
/// 3. Inject / filter text input
/// 4. Normal per-tab navigation
pub fn handle_key(
    key: KeyEvent,
    mode: InputMode,
    active_tab: Tab,
    focus: FocusZone,
    modals: &ModalVisibility,
) -> TuiAction {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return TuiAction::QuitConfirmed;
    }

    // Modal intercepts (highest priority first)
    if let Some(modal) = modals.active_modal {
        return match modal {
            ModalState::Help => handle_help_key(key),
            ModalState::Approval { .. } => handle_approval_key(key),
            ModalState::WaveOverview { .. } => handle_wave_overview_key(key),
            ModalState::PlanDetail { .. } => handle_plan_detail_key(key),
            ModalState::TaskPicker { .. } => handle_task_picker_key(key),
            ModalState::TaskDetail { .. } => handle_task_detail_key(key),
            ModalState::QueueOverview { .. } => handle_queue_overview_key(key),
            ModalState::AgentPool { .. } => handle_agent_pool_key(key),
            _ => TuiAction::None,
        };
    }

    // Confirm dialog
    if mode == InputMode::Confirm {
        return handle_confirm_key(key);
    }

    // Text input modes
    if mode == InputMode::ConfigEdit {
        return handle_config_edit_key(key);
    }
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

/// Active modal reference used by key dispatch.
#[derive(Debug, Clone, Copy, Default)]
pub struct ModalVisibility<'a> {
    pub active_modal: Option<&'a ModalState>,
}

impl<'a> ModalVisibility<'a> {
    #[must_use]
    pub fn from_active_modal(active_modal: Option<&'a ModalState>) -> Self {
        Self { active_modal }
    }
}

// ---------------------------------------------------------------------------
// Modal key handlers
// ---------------------------------------------------------------------------

fn handle_help_key(key: KeyEvent) -> TuiAction {
    match key.code {
        KeyCode::Esc | KeyCode::Char('?' | 'q') => TuiAction::ShowHelp,
        _ => TuiAction::None,
    }
}

fn handle_approval_key(key: KeyEvent) -> TuiAction {
    match key.code {
        KeyCode::Char('y' | 'Y') | KeyCode::Enter => TuiAction::ApproveCommand,
        KeyCode::Char('n' | 'N') | KeyCode::Esc => TuiAction::RejectCommand,
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            TuiAction::ApproveAll
        }
        KeyCode::Char('A') => TuiAction::ApproveAll,
        _ => TuiAction::None,
    }
}

fn handle_wave_overview_key(key: KeyEvent) -> TuiAction {
    match key.code {
        KeyCode::Esc | KeyCode::Char('w') => TuiAction::ShowWaveOverview,
        KeyCode::Up | KeyCode::Char('k') => TuiAction::ModalScrollUp,
        KeyCode::Down | KeyCode::Char('j') => TuiAction::ModalScrollDown,
        _ => TuiAction::None,
    }
}

fn handle_plan_detail_key(key: KeyEvent) -> TuiAction {
    match key.code {
        KeyCode::Esc => TuiAction::ClosePlanDetail,
        KeyCode::Up | KeyCode::Char('k') => TuiAction::ScrollDetailUp,
        KeyCode::Down | KeyCode::Char('j') => TuiAction::ScrollDetailDown,
        _ => TuiAction::None,
    }
}

fn handle_task_picker_key(key: KeyEvent) -> TuiAction {
    match key.code {
        KeyCode::Esc => TuiAction::CloseTaskPicker,
        KeyCode::Enter => TuiAction::ShowTaskDetail,
        KeyCode::Up | KeyCode::Char('k') => TuiAction::TaskPickerUp,
        KeyCode::Down | KeyCode::Char('j') => TuiAction::TaskPickerDown,
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
        KeyCode::Up | KeyCode::Char('k') => TuiAction::QueueOverviewUp,
        KeyCode::Down | KeyCode::Char('j') => TuiAction::QueueOverviewDown,
        _ => TuiAction::None,
    }
}

fn handle_agent_pool_key(key: KeyEvent) -> TuiAction {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => TuiAction::CloseModal,
        KeyCode::Up | KeyCode::Char('k') => TuiAction::ModalScrollUp,
        KeyCode::Down | KeyCode::Char('j') => TuiAction::ModalScrollDown,
        _ => TuiAction::None,
    }
}

fn handle_confirm_key(key: KeyEvent) -> TuiAction {
    match key.code {
        KeyCode::Char('y' | 'Y') | KeyCode::Enter => TuiAction::ConfirmYes,
        KeyCode::Char('n' | 'N') | KeyCode::Esc => TuiAction::ConfirmNo,
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
    // F-keys switch tabs
    if let Some(tab) = Tab::from_key(key.code) {
        return Some(TuiAction::SwitchTab(tab));
    }

    match key.code {
        KeyCode::Char('q') => Some(TuiAction::Quit),
        KeyCode::Char('?') => Some(TuiAction::ShowHelp),
        KeyCode::Char('n') => Some(TuiAction::DismissNotification),
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(TuiAction::Refresh)
        }
        // Ctrl-a: approve all pending
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(TuiAction::ApproveAll)
        }
        // Ctrl-t: toggle the agent-topology panel from anywhere.
        KeyCode::Char('t') if is_ctrl_t(key) => Some(TuiAction::ToggleAgentTopology),
        // Ctrl-x: force advance (confirm)
        KeyCode::Char('x') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(TuiAction::ForceAdvance)
        }
        // Ctrl-d: reset selected plan (confirm)
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(TuiAction::ResetPlanState)
        }
        // Ctrl-e: toggle full-screen post-processing
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(TuiAction::ToggleScreenPostFx)
        }
        KeyCode::Char('v') => Some(TuiAction::CycleEffectsPreset),
        // Ctrl-g: reconcile git state (confirm)
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(TuiAction::RequestConfirm(ConfirmAction::GitReconcile))
        }
        // F8 / u: queue overview
        KeyCode::F(8) | KeyCode::Char('u') => Some(TuiAction::ShowQueueOverview),
        KeyCode::Tab => Some(TuiAction::FocusNext),
        KeyCode::BackTab => Some(TuiAction::FocusPrev),
        _ => Option::None,
    }
}

/// Returns `true` when the key event matches the global `Ctrl+T` shortcut.
#[must_use]
pub(crate) fn is_ctrl_t(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('t'))
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
        && !key.modifiers.contains(KeyModifiers::SHIFT)
        && !key.modifiers.contains(KeyModifiers::SUPER)
}

// ---------------------------------------------------------------------------
// Per-tab key handlers
// ---------------------------------------------------------------------------

fn handle_dashboard_key(key: KeyEvent, focus: FocusZone) -> TuiAction {
    match key.code {
        // Navigation — focus-aware
        KeyCode::Up | KeyCode::Char('k') => match focus {
            FocusZone::PlanTree => TuiAction::SelectPlanUp,
            FocusZone::AgentOutput => TuiAction::ScrollAgentUp,
            _ => TuiAction::ScrollFocusedUp,
        },
        KeyCode::Down | KeyCode::Char('j') => match focus {
            FocusZone::PlanTree => TuiAction::SelectPlanDown,
            FocusZone::AgentOutput => TuiAction::ScrollAgentDown,
            _ => TuiAction::ScrollFocusedDown,
        },
        KeyCode::PageUp => TuiAction::ScrollPageUp,
        KeyCode::PageDown => TuiAction::ScrollPageDown,
        KeyCode::Home => TuiAction::ScrollFocusedHome,
        KeyCode::End => TuiAction::ScrollFocusedEnd,

        // Plan tree operations
        KeyCode::Enter => TuiAction::ShowPlanDetail,
        KeyCode::Esc => TuiAction::ClosePlanDetail,
        KeyCode::Left if key.modifiers.contains(KeyModifiers::SHIFT) => TuiAction::WavePrev,
        KeyCode::Right if key.modifiers.contains(KeyModifiers::SHIFT) => TuiAction::WaveNext,
        KeyCode::Left | KeyCode::Char('h') => TuiAction::DrillOut,
        KeyCode::Right | KeyCode::Char('l') => TuiAction::DrillIn,

        // Sub-tab switching (a/o/d/e/g/m/L/P)
        KeyCode::Char('a') => TuiAction::SwitchDetailTab(0), // Agents
        KeyCode::Char('o') => TuiAction::SwitchDetailTab(1), // Output
        KeyCode::Char('d') => TuiAction::SwitchDetailTab(2), // Diff
        KeyCode::Char('e') => TuiAction::SwitchDetailTab(3), // Errors
        KeyCode::Char('g') => TuiAction::SwitchDetailTab(4), // Git
        KeyCode::Char('m') => TuiAction::SwitchDetailTab(5), // MCP
        KeyCode::Char('L') => TuiAction::SwitchDetailTab(6), // Learning
        KeyCode::Char('P') => TuiAction::SwitchDetailTab(7), // Processes

        // Modal triggers
        KeyCode::Char('w') => TuiAction::ShowWaveOverview,
        KeyCode::Char('p') => TuiAction::TogglePause,
        KeyCode::Char('i') => TuiAction::StartInject,
        KeyCode::Char('y') => TuiAction::ApproveCommand,
        // Agent role tabs (backtick cycles, Alt+N selects)
        KeyCode::Char('`') => TuiAction::SwitchAgentTab(usize::MAX), // cycle

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
        KeyCode::Left if key.modifiers.contains(KeyModifiers::SHIFT) => TuiAction::WavePrev,
        KeyCode::Right if key.modifiers.contains(KeyModifiers::SHIFT) => TuiAction::WaveNext,
        KeyCode::Left | KeyCode::Char('h') => TuiAction::DrillOut,
        KeyCode::Right | KeyCode::Char('l') => TuiAction::DrillIn,
        KeyCode::PageUp => TuiAction::ScrollPageUp,
        KeyCode::PageDown => TuiAction::ScrollPageDown,
        KeyCode::Home => TuiAction::ScrollFocusedHome,
        KeyCode::End => TuiAction::ScrollFocusedEnd,

        // Filter mode
        KeyCode::Char('/') => TuiAction::StartFilter,

        // Plan operations (Mori parity)
        KeyCode::Char('d') => TuiAction::RequestConfirm(ConfirmAction::DiagnosePlan(String::new())),
        KeyCode::Char('m') => TuiAction::RequestConfirm(ConfirmAction::MergePlan {
            plan_id: String::new(),
            branch: String::new(),
        }),
        KeyCode::Char('M') => TuiAction::RequestConfirm(ConfirmAction::MergeAllDone {
            branches: Vec::new(),
        }),
        KeyCode::Char('s') => TuiAction::RestartPlan, // soft retry
        KeyCode::Char('z') => TuiAction::ReverifyPlan, // diagnose
        KeyCode::Char('S') => TuiAction::ResetPlanState, // repair preserve
        KeyCode::Char('R') => TuiAction::RestartPhase, // repair clean
        KeyCode::Char('c') => TuiAction::ReverifyPlan, // reverify
        KeyCode::Char('F') => TuiAction::ForceAdvance,
        KeyCode::Char('V') => TuiAction::ReverifyPlan,
        _ => TuiAction::None,
    }
}

fn handle_agents_key(key: KeyEvent, focus: FocusZone) -> TuiAction {
    match key.code {
        // Focus-aware navigation
        KeyCode::Up | KeyCode::Char('k') => match focus {
            FocusZone::AgentOutput => TuiAction::ScrollAgentUp,
            FocusZone::RightPanel => TuiAction::ScrollDiffUp,
            _ => TuiAction::SelectPlanUp,
        },
        KeyCode::Down | KeyCode::Char('j') => match focus {
            FocusZone::AgentOutput => TuiAction::ScrollAgentDown,
            FocusZone::RightPanel => TuiAction::ScrollDiffDown,
            _ => TuiAction::SelectPlanDown,
        },
        KeyCode::PageUp => TuiAction::ScrollPageUp,
        KeyCode::PageDown => TuiAction::ScrollPageDown,
        KeyCode::Home => TuiAction::ScrollFocusedHome,
        KeyCode::End => TuiAction::ScrollFocusedEnd,
        KeyCode::Char('G') => TuiAction::ScrollAgentEnd,

        // Agent role tab switching (1-4 direct, backtick cycles)
        KeyCode::Char('`') => TuiAction::SwitchAgentTab(usize::MAX), // cycle
        KeyCode::Char('1') => TuiAction::SwitchAgentTab(0),
        KeyCode::Char('2') => TuiAction::SwitchAgentTab(1),
        KeyCode::Char('3') => TuiAction::SwitchAgentTab(2),
        KeyCode::Char('4') => TuiAction::SwitchAgentTab(3),
        KeyCode::Char('5') => TuiAction::SwitchAgentTab(4),
        KeyCode::Char('6') => TuiAction::SwitchAgentTab(5),
        KeyCode::Char('7') => TuiAction::SwitchAgentTab(6),

        // Agent approval and interaction
        KeyCode::Char('a') => TuiAction::ApproveCommand,
        KeyCode::Char('A') => TuiAction::ApproveAll,
        KeyCode::Char('x') => TuiAction::RejectCommand,
        KeyCode::Char('i') => TuiAction::StartInject,
        KeyCode::Char('g') => TuiAction::ToggleAgentPaneGroup,
        KeyCode::Char('t') => TuiAction::ToggleAgentTopology,
        _ => TuiAction::None,
    }
}

fn handle_git_key(key: KeyEvent, _focus: FocusZone) -> TuiAction {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => TuiAction::ScrollFocusedUp,
        KeyCode::Down | KeyCode::Char('j') => TuiAction::ScrollFocusedDown,
        KeyCode::PageUp => TuiAction::ScrollPageUp,
        KeyCode::PageDown => TuiAction::ScrollPageDown,
        KeyCode::Home => TuiAction::ScrollFocusedHome,
        KeyCode::End => TuiAction::ScrollFocusedEnd,
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
        KeyCode::PageUp => TuiAction::ScrollPageUp,
        KeyCode::PageDown => TuiAction::ScrollPageDown,
        KeyCode::Home => TuiAction::ScrollFocusedHome,
        KeyCode::End => TuiAction::ScrollLogEnd,
        KeyCode::Char('G') => TuiAction::ScrollLogEnd,
        KeyCode::Char('1') => TuiAction::ToggleLogFilter(LogFilterLevel::Info),
        KeyCode::Char('2') => TuiAction::ToggleLogFilter(LogFilterLevel::Warn),
        KeyCode::Char('3') => TuiAction::ToggleLogFilter(LogFilterLevel::Error),
        KeyCode::Char('4') => TuiAction::ToggleLogFilter(LogFilterLevel::Debug),
        KeyCode::Char('a') => TuiAction::ShowAllLogFilters,
        KeyCode::Char('/') => TuiAction::StartFilter,
        _ => TuiAction::None,
    }
}

fn handle_config_key(key: KeyEvent) -> TuiAction {
    // Ctrl-S saves pending config edits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
        return TuiAction::ConfigSave;
    }
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => TuiAction::ConfigUp,
        KeyCode::Down | KeyCode::Char('j') => TuiAction::ConfigDown,
        KeyCode::Left | KeyCode::Char('h') => TuiAction::ConfigCycleLeft,
        KeyCode::Right | KeyCode::Char('l') => TuiAction::ConfigCycleRight,
        KeyCode::Enter | KeyCode::Char(' ') => TuiAction::ConfigToggle,
        _ => TuiAction::None,
    }
}

/// Key handler for config text-edit mode (typing a value).
fn handle_config_edit_key(key: KeyEvent) -> TuiAction {
    match key.code {
        KeyCode::Enter => TuiAction::ConfigCommitEdit,
        KeyCode::Esc => TuiAction::ConfigCancelEdit,
        KeyCode::Backspace => TuiAction::InputBackspace,
        KeyCode::Char(c) => TuiAction::InputChar(c),
        _ => TuiAction::None,
    }
}

fn handle_inspect_key(key: KeyEvent, _focus: FocusZone) -> TuiAction {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => TuiAction::ScrollFocusedUp,
        KeyCode::Down | KeyCode::Char('j') => TuiAction::ScrollFocusedDown,
        KeyCode::PageUp => TuiAction::ScrollPageUp,
        KeyCode::PageDown => TuiAction::ScrollPageDown,
        KeyCode::Home => TuiAction::ScrollFocusedHome,
        KeyCode::End => TuiAction::ScrollFocusedEnd,
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

    fn modals<'a>(active_modal: Option<&'a ModalState>) -> ModalVisibility<'a> {
        ModalVisibility::from_active_modal(active_modal)
    }

    #[test]
    fn ctrl_c_always_quits() {
        let action = handle_key(
            key_with_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
            InputMode::Normal,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(action, TuiAction::QuitConfirmed);
    }

    #[test]
    fn global_n_dismisses_notifications_on_any_tab() {
        let action = handle_key(
            key(KeyCode::Char('n')),
            InputMode::Normal,
            Tab::Logs,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(action, TuiAction::DismissNotification);
    }

    #[test]
    fn f_keys_switch_tabs() {
        let action = handle_key(
            key(KeyCode::F(3)),
            InputMode::Normal,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &modals(None),
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
            &modals(None),
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
            &modals(None),
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
            &modals(None),
        );
        assert_eq!(action, TuiAction::AcceptFilter);
    }

    #[test]
    fn logs_tab_number_keys_toggle_expected_level() {
        let action = handle_key(
            key(KeyCode::Char('3')),
            InputMode::Normal,
            Tab::Logs,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(action, TuiAction::ToggleLogFilter(LogFilterLevel::Error));
    }

    #[test]
    fn ctrl_e_toggles_screen_postfx() {
        let action = handle_key(
            key_with_mod(KeyCode::Char('e'), KeyModifiers::CONTROL),
            InputMode::Normal,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(action, TuiAction::ToggleScreenPostFx);
    }

    #[test]
    fn ctrl_t_toggles_agent_topology_globally() {
        let action = handle_key(
            key_with_mod(KeyCode::Char('t'), KeyModifiers::CONTROL),
            InputMode::Normal,
            Tab::Logs,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(action, TuiAction::ToggleAgentTopology);
    }

    #[test]
    fn v_cycles_effects_presets_outside_plans_tab() {
        let action = handle_key(
            key(KeyCode::Char('v')),
            InputMode::Normal,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(action, TuiAction::CycleEffectsPreset);
    }

    #[test]
    fn v_cycles_effects_presets_on_plans_tab() {
        let action = handle_key(
            key(KeyCode::Char('v')),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(action, TuiAction::CycleEffectsPreset);
    }

    #[test]
    fn logs_tab_a_restores_all_levels() {
        let action = handle_key(
            key(KeyCode::Char('a')),
            InputMode::Normal,
            Tab::Logs,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(action, TuiAction::ShowAllLogFilters);
    }

    #[test]
    fn task_picker_modal_intercepts() {
        let modal = ModalState::TaskPicker {
            tasks: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
        };
        let m = modals(Some(&modal));
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
    fn approval_modal_intercepts_yes_and_no() {
        let modal = ModalState::Approval {
            role: "implementer".to_string(),
            command: "cargo check".to_string(),
        };
        let m = modals(Some(&modal));

        let approve = handle_key(
            key(KeyCode::Char('y')),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::PlanTree,
            &m,
        );
        assert_eq!(approve, TuiAction::ApproveCommand);

        let reject = handle_key(
            key(KeyCode::Char('n')),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::PlanTree,
            &m,
        );
        assert_eq!(reject, TuiAction::RejectCommand);
    }

    #[test]
    fn modal_visibility_reads_active_modal() {
        let help = ModalState::Help;
        let vis = ModalVisibility::from_active_modal(Some(&help));
        assert!(matches!(vis.active_modal, Some(ModalState::Help)));

        let approval = ModalState::Approval {
            role: "implementer".to_string(),
            command: "cargo check".to_string(),
        };
        let vis = ModalVisibility::from_active_modal(Some(&approval));
        assert!(matches!(
            vis.active_modal,
            Some(ModalState::Approval { .. })
        ));

        let task_picker = ModalState::TaskPicker {
            tasks: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
        };
        let vis = ModalVisibility::from_active_modal(Some(&task_picker));
        assert!(matches!(
            vis.active_modal,
            Some(ModalState::TaskPicker { .. })
        ));

        let agent_pool = ModalState::AgentPool {
            agents: Vec::new(),
            scroll_offset: 0,
        };
        let vis = ModalVisibility::from_active_modal(Some(&agent_pool));
        assert!(matches!(
            vis.active_modal,
            Some(ModalState::AgentPool { .. })
        ));
    }

    #[test]
    fn modal_open_keys_do_not_fall_through_to_background_navigation() {
        let modal = ModalState::Help;
        let vis = modals(Some(&modal));

        let action = handle_key(
            key(KeyCode::Tab),
            InputMode::Normal,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &vis,
        );
        assert_eq!(action, TuiAction::None);

        let action = handle_key(
            key(KeyCode::Down),
            InputMode::Normal,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &vis,
        );
        assert_eq!(action, TuiAction::None);
    }

    #[test]
    fn modal_specific_navigation_stays_local() {
        let modal = ModalState::PlanDetail {
            plan_id: "plan-1".to_string(),
        };
        let vis = modals(Some(&modal));

        let action = handle_key(
            key(KeyCode::Char('k')),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::PlanTree,
            &vis,
        );
        assert_eq!(action, TuiAction::ScrollDetailUp);

        let action = handle_key(
            key(KeyCode::Char('j')),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::PlanTree,
            &vis,
        );
        assert_eq!(action, TuiAction::ScrollDetailDown);

        let action = handle_key(
            key(KeyCode::Tab),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::PlanTree,
            &vis,
        );
        assert_eq!(action, TuiAction::None);
    }

    #[test]
    fn wave_overview_modal_still_handles_local_scroll() {
        let modal = ModalState::WaveOverview {
            waves: Vec::new(),
            scroll_offset: 0,
        };
        let vis = modals(Some(&modal));

        let action = handle_key(
            key(KeyCode::Up),
            InputMode::Normal,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &vis,
        );
        assert_eq!(action, TuiAction::ModalScrollUp);

        let action = handle_key(
            key(KeyCode::Char('j')),
            InputMode::Normal,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &vis,
        );
        assert_eq!(action, TuiAction::ModalScrollDown);

        let action = handle_key(
            key(KeyCode::Tab),
            InputMode::Normal,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &vis,
        );
        assert_eq!(action, TuiAction::None);
    }

    #[test]
    fn queue_overview_modal_uses_local_navigation() {
        let modal = ModalState::QueueOverview {
            milestones: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
        };
        let vis = modals(Some(&modal));

        let action = handle_key(
            key(KeyCode::Up),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::PlanTree,
            &vis,
        );
        assert_eq!(action, TuiAction::QueueOverviewUp);

        let action = handle_key(
            key(KeyCode::Char('j')),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::PlanTree,
            &vis,
        );
        assert_eq!(action, TuiAction::QueueOverviewDown);
    }

    #[test]
    fn agent_pool_modal_blocks_navigation_keys() {
        let modal = ModalState::AgentPool {
            agents: Vec::new(),
            scroll_offset: 0,
        };
        let vis = modals(Some(&modal));

        let action = handle_key(
            key(KeyCode::Up),
            InputMode::Normal,
            Tab::Agents,
            FocusZone::AgentOutput,
            &vis,
        );
        assert_eq!(action, TuiAction::ModalScrollUp);

        let action = handle_key(
            key(KeyCode::Char('j')),
            InputMode::Normal,
            Tab::Agents,
            FocusZone::AgentOutput,
            &vis,
        );
        assert_eq!(action, TuiAction::ModalScrollDown);

        let action = handle_key(
            key(KeyCode::Tab),
            InputMode::Normal,
            Tab::Agents,
            FocusZone::AgentOutput,
            &vis,
        );
        assert_eq!(action, TuiAction::None);

        let action = handle_key(
            key(KeyCode::Esc),
            InputMode::Normal,
            Tab::Agents,
            FocusZone::AgentOutput,
            &vis,
        );
        assert_eq!(action, TuiAction::CloseModal);
    }

    #[test]
    fn ctrl_c_takes_precedence_over_open_modal() {
        let modal = ModalState::WaveOverview {
            waves: Vec::new(),
            scroll_offset: 0,
        };
        let vis = modals(Some(&modal));

        let action = handle_key(
            key_with_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
            InputMode::Normal,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &vis,
        );
        assert_eq!(action, TuiAction::QuitConfirmed);
    }

    #[test]
    fn ctrl_c_takes_precedence_over_modal_and_mode_intercepts() {
        let modal = ModalState::Approval {
            role: "implementer".to_string(),
            command: "cargo check".to_string(),
        };
        let vis = modals(Some(&modal));

        let action = handle_key(
            key_with_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
            InputMode::Confirm,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &vis,
        );
        assert_eq!(action, TuiAction::QuitConfirmed);
    }

    #[test]
    fn plans_tab_focus_routing() {
        let action = handle_key(
            key(KeyCode::Up),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(action, TuiAction::SelectPlanUp);

        let action = handle_key(
            key(KeyCode::Up),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::AgentOutput,
            &modals(None),
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
            &modals(None),
        );
        assert_eq!(action, TuiAction::ScrollDiffDown);
    }

    #[test]
    fn page_keys_use_page_scroll_actions() {
        let action = handle_key(
            key(KeyCode::PageUp),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::TaskProgress,
            &modals(None),
        );
        assert_eq!(action, TuiAction::ScrollPageUp);

        let action = handle_key(
            key(KeyCode::PageDown),
            InputMode::Normal,
            Tab::Dashboard,
            FocusZone::CommandOutput,
            &modals(None),
        );
        assert_eq!(action, TuiAction::ScrollPageDown);
    }

    #[test]
    fn page_keys_keep_page_scroll_actions_on_logs_tab() {
        let action = handle_key(
            key(KeyCode::PageUp),
            InputMode::Normal,
            Tab::Logs,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(action, TuiAction::ScrollPageUp);

        let action = handle_key(
            key(KeyCode::PageDown),
            InputMode::Normal,
            Tab::Logs,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(action, TuiAction::ScrollPageDown);
    }

    #[test]
    fn logs_tab_slash_starts_filter() {
        let action = handle_key(
            key(KeyCode::Char('/')),
            InputMode::Normal,
            Tab::Logs,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(action, TuiAction::StartFilter);
    }

    #[test]
    fn home_end_use_focused_jump_actions() {
        let action = handle_key(
            key(KeyCode::Home),
            InputMode::Normal,
            Tab::Agents,
            FocusZone::RightPanel,
            &modals(None),
        );
        assert_eq!(action, TuiAction::ScrollFocusedHome);

        let action = handle_key(
            key(KeyCode::End),
            InputMode::Normal,
            Tab::Logs,
            FocusZone::CommandOutput,
            &modals(None),
        );
        assert_eq!(action, TuiAction::ScrollFocusedEnd);

        let action = handle_key(
            key(KeyCode::End),
            InputMode::Normal,
            Tab::Logs,
            FocusZone::CommandOutput,
            &modals(None),
        );
        assert_eq!(action, TuiAction::ScrollLogEnd);

        let action = handle_key(
            key(KeyCode::Char('G')),
            InputMode::Normal,
            Tab::Logs,
            FocusZone::AgentOutput,
            &modals(None),
        );
        assert_eq!(action, TuiAction::ScrollLogEnd);
    }

    #[test]
    fn shift_arrow_keys_navigate_waves() {
        let action = handle_key(
            key_with_mod(KeyCode::Left, KeyModifiers::SHIFT),
            InputMode::Normal,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(action, TuiAction::WavePrev);

        let action = handle_key(
            key_with_mod(KeyCode::Right, KeyModifiers::SHIFT),
            InputMode::Normal,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(action, TuiAction::WaveNext);

        let action = handle_key(
            key_with_mod(KeyCode::Left, KeyModifiers::SHIFT),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(action, TuiAction::WavePrev);

        let action = handle_key(
            key_with_mod(KeyCode::Right, KeyModifiers::SHIFT),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(action, TuiAction::WaveNext);
    }

    fn plans_tab_confirm_shortcuts_route_to_request_confirm() {
        let action = handle_key(
            key(KeyCode::Char('d')),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(
            action,
            TuiAction::RequestConfirm(ConfirmAction::DiagnosePlan(String::new()))
        );

        let action = handle_key(
            key(KeyCode::Char('m')),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(
            action,
            TuiAction::RequestConfirm(ConfirmAction::MergePlan {
                plan_id: String::new(),
                branch: String::new(),
            })
        );

        let action = handle_key(
            key(KeyCode::Char('M')),
            InputMode::Normal,
            Tab::Plans,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(
            action,
            TuiAction::RequestConfirm(ConfirmAction::MergeAllDone {
                branches: Vec::new(),
            })
        );
    }

    #[test]
    fn ctrl_g_requests_git_reconcile_confirmation() {
        let action = handle_key(
            key_with_mod(KeyCode::Char('g'), KeyModifiers::CONTROL),
            InputMode::Normal,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &modals(None),
        );
        assert_eq!(
            action,
            TuiAction::RequestConfirm(ConfirmAction::GitReconcile)
        );
    }
}
