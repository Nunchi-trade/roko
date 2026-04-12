//! Interactive TUI application shell.
//!
//! Integrates the Mori-style tab system (F1-F7), modal dialogs, TuiState,
//! TuiAction dispatch, PostFX pipeline, and atmosphere animations.

use std::collections::HashMap;
use std::io;
use std::io::Stdout;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use serde_json::Value;

use super::atmosphere::Atmosphere;
use super::dashboard::{DashboardData, DashboardScaffold, Theme};
use super::effects_config::EffectsConfig;
use super::event::{Event, EventHandler};
use super::input::{self, ConfirmAction, FocusZone, InputMode, TuiAction};
use super::modals::{self, ModalState};
use super::pages::{PageId, PageRegistry};
use super::state::TuiState;
use super::tabs::Tab;
use super::views::{self, ViewState};


/// Interactive dashboard shell backed by the existing snapshot renderer.
///
/// Supports two rendering paths:
/// - **Mori-style tabs** (F1-F7): full TuiState + views + modals + postfx
/// - **Legacy scaffold pages**: original PageId-based rendering
#[derive(Debug)]
pub struct App {
    workdir: PathBuf,

    // -- Mori-style state --
    /// Full TUI state (agents, plans, navigation, modals, scroll, etc.).
    pub tui_state: TuiState,
    /// Atmosphere for breathing/heartbeat animations.
    atmosphere: Atmosphere,
    /// PostFX configuration.
    fx_config: EffectsConfig,
    /// Active modal overlay.
    active_modal: Option<ModalState>,
    /// Toast notifications.
    notifications: Vec<super::modals::Notification>,

    // -- Legacy scaffold state (kept for text-mode compatibility) --
    /// Currently selected dashboard page (legacy path).
    pub current_page: PageId,
    /// Shared dashboard data model, refreshed on tick.
    pub data: DashboardData,
    /// Static page scaffold used by the legacy renderer.
    scaffold: DashboardScaffold,

    // -- Common --
    /// Whether the event loop should keep running.
    pub running: bool,
    /// Timestamp of the last data refresh.
    pub last_refresh: Instant,
    /// Per-page scroll position (legacy).
    pub scroll_offset: HashMap<PageId, u16>,
    /// Selected signal row on the Signals page (legacy).
    pub signal_selection: usize,
    /// Selected gate-failure row on the Gate Results page (legacy).
    pub gate_failure_selection: usize,
    /// Active overlay, if any (legacy help/detail).
    overlay: Option<OverlayState>,
}

type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;

#[derive(Debug, Clone)]
enum OverlayState {
    Help,
    Detail(DetailState),
}

struct PanicHookRestoreGuard(Arc<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static>);

impl Drop for PanicHookRestoreGuard {
    fn drop(&mut self) {
        let hook = Arc::clone(&self.0);
        std::panic::set_hook(Box::new(move |panic_info| hook(panic_info)));
    }
}

/// Run the interactive dashboard event loop (async variant).
pub async fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| app.draw(f))?;
        if crossterm::event::poll(Duration::from_millis(250))? {
            match crossterm::event::read()? {
                crossterm::event::Event::Key(key) => app.handle_key(key),
                crossterm::event::Event::Mouse(mouse) => app.handle_mouse(mouse),
                crossterm::event::Event::Resize(_, _) => {}
                _ => {}
            }
        }
        if app.last_refresh.elapsed() > Duration::from_secs(1) {
            app.data.refresh().await?;
            app.tui_state.update_from_snapshot(&app.data);
            app.last_refresh = Instant::now();
        }
        if !app.running {
            break;
        }
    }
    Ok(())
}

impl App {
    /// Build a new app from a workspace root.
    #[must_use]
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self::new_with_page(root, None)
    }

    /// Build a new app from a workspace root with an initial page selection.
    #[must_use]
    pub fn new_with_page(root: impl AsRef<Path>, initial_page: Option<PageId>) -> Self {
        let workdir = root.as_ref().to_path_buf();
        let mut scaffold = DashboardScaffold::new_in(&workdir);
        if let Some(page) = initial_page {
            let _ = scaffold.set_active_page(page);
        }
        let data = DashboardData::load_best_effort(&workdir);
        let mut tui_state = TuiState::new();
        tui_state.update_from_snapshot(&data);
        tui_state.run_started = Some(Instant::now());

        let mut app = Self {
            workdir,
            tui_state,
            atmosphere: Atmosphere::default(),
            fx_config: EffectsConfig::default(),
            active_modal: None,
            notifications: Vec::new(),
            current_page: scaffold.active_page(),
            data,
            scaffold,
            running: true,
            last_refresh: Instant::now(),
            scroll_offset: HashMap::new(),
            signal_selection: 0,
            gate_failure_selection: 0,
            overlay: None,
        };
        app.populate_git_info();
        app
    }

    /// Return the active page (legacy).
    #[must_use]
    pub const fn current_page(&self) -> PageId {
        self.current_page
    }

    /// Return the active page (legacy).
    #[must_use]
    pub const fn active_page(&self) -> PageId {
        self.current_page
    }

    /// Run the terminal UI until the user quits.
    pub fn run(mut self) -> Result<()> {
        let previous_hook: Arc<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static> =
            Arc::from(std::panic::take_hook());
        let panic_hook = Arc::clone(&previous_hook);
        let _restore_hook = PanicHookRestoreGuard(previous_hook);

        std::panic::set_hook(Box::new(move |panic_info| {
            let _ = Self::cleanup_terminal();
            panic_hook(panic_info);
        }));

        let mut terminal = Self::enter_terminal()?;
        let result = self.main_loop(&mut terminal);
        let cleanup = Self::leave_terminal();

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(err), Ok(())) => Err(err),
            (Ok(()), Err(err)) => Err(err),
            (Err(err), Err(_cleanup_err)) => Err(err),
        }
    }

    fn main_loop(&mut self, terminal: &mut TuiTerminal) -> Result<()> {
        let mut events = EventHandler::new(Duration::from_millis(16)); // ~60fps
        terminal
            .draw(|frame| self.draw(frame))
            .context("initial TUI draw")?;

        while self.running {
            match events.next().context("poll TUI event")? {
                Event::Key(key) => {
                    self.handle_key(key);
                    // Immediate redraw after keypress for responsiveness
                    terminal
                        .draw(|frame| self.draw(frame))
                        .context("TUI redraw after key")?;
                    continue;
                }
                Event::Resize(_, _) => {}
                Event::Tick => {
                    self.atmosphere.tick();
                    self.tui_state.atmosphere.tick();
                    self.refresh_snapshot_if_needed();
                    self.expire_notifications();
                }
            }

            terminal
                .draw(|frame| self.draw(frame))
                .context("TUI redraw")?;
        }

        Ok(())
    }

    fn draw(&self, frame: &mut Frame<'_>) {
        let theme = Theme::from_env();
        let full_area = frame.area();

        // Responsive outer margin on large terminals
        let content_area = super::layout::responsive_outer_margin(full_area);

        // Main layout: header + alert_banner + content + footer
        let main_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Tab bar header
                Constraint::Min(0),   // Content area
                Constraint::Length(1), // Status footer
            ])
            .split(content_area);

        // Header: tab bar
        self.render_tab_header(frame, main_layout[0], &theme);

        // Content: dispatch to active tab view
        let view_state = ViewState {
            scroll: self.tui_state.plan_scroll_offset as u16,
            selected: self.tui_state.selected_plan_idx,
            sub_tab: self.tui_state.plan_detail_tab,
            secondary_selected: 0,
            auto_tail: self.tui_state.agent_scroll.is_none(),
        };
        views::render_tab_content(
            frame,
            main_layout[1],
            self.tui_state.active_tab,
            &self.data,
            &self.tui_state,
            &view_state,
            &theme,
        );

        // Footer: status line
        self.render_status_footer(frame, main_layout[2], &theme);

        // Dim overlay before modals
        if self.active_modal.is_some() || self.tui_state.show_help {
            // Apply dim overlay on the buffer
            let buf = frame.buffer_mut();
            super::postfx::dim_overlay(content_area, buf, 0.45);
        }

        // Help overlay (legacy compatible)
        if self.tui_state.show_help {
            self.render_help_overlay(frame, full_area, &theme);
        }

        // Modal rendering
        modals::render_modals(
            frame,
            full_area,
            self.active_modal.as_ref(),
            &self.notifications,
            &theme,
        );

        // Legacy overlay (signal/gate detail)
        if let Some(overlay) = &self.overlay {
            self.render_overlay(frame, overlay);
        }

        // PostFX pipeline
        if self.fx_config.screen_postfx {
            let buf = frame.buffer_mut();
            super::postfx_pipeline::apply_pipeline(
                self.tui_state.active_tab as usize,
                content_area,
                buf,
                self.atmosphere.elapsed,
                self.atmosphere.frame_count,
                &self.fx_config,
            );
        }
    }

    // -----------------------------------------------------------------------
    // Key handling
    // -----------------------------------------------------------------------

    fn handle_key(&mut self, key: KeyEvent) {
        // Legacy overlay intercept
        if self.overlay.is_some() {
            if self.handle_overlay_key(key) {
                return;
            }
        }

        // Route through the full TuiAction dispatch
        let action = input::handle_key(
            key,
            self.tui_state.input_mode,
            self.tui_state.active_tab,
            self.tui_state.focus,
            &self.tui_state.modal_visibility(),
        );

        self.dispatch_action(action);
    }

    fn dispatch_action(&mut self, action: TuiAction) {
        match action {
            TuiAction::Quit => {
                // Bug fix: q with overlay closes overlay first
                if self.tui_state.has_modal() || self.overlay.is_some() {
                    self.tui_state.dismiss_all_modals();
                    self.active_modal = None;
                    self.overlay = None;
                } else {
                    self.running = false;
                }
            }
            TuiAction::SwitchTab(tab) => {
                self.tui_state.active_tab = tab;
                self.tui_state.focus = FocusZone::PlanTree;
                // Sync legacy page
                if let Some(page_id) = tab_to_page(tab) {
                    self.current_page = page_id;
                    let _ = self.scaffold.set_active_page(page_id);
                }
            }
            TuiAction::FocusNext => {
                self.tui_state.focus = self.tui_state.focus.next();
            }
            TuiAction::FocusPrev => {
                self.tui_state.focus = self.tui_state.focus.prev();
            }
            TuiAction::SelectPlanUp => {
                if self.tui_state.selected_plan_idx > 0 {
                    self.tui_state.selected_plan_idx -= 1;
                }
            }
            TuiAction::SelectPlanDown => {
                let max = self.tui_state.plans.len().saturating_sub(1);
                if self.tui_state.selected_plan_idx < max {
                    self.tui_state.selected_plan_idx += 1;
                }
            }
            TuiAction::ScrollFocusedUp => self.scroll_focused(-1),
            TuiAction::ScrollFocusedDown => self.scroll_focused(1),
            TuiAction::ScrollLogUp => {
                self.tui_state.log_scroll = self.tui_state.log_scroll.saturating_sub(1);
            }
            TuiAction::ScrollLogDown => {
                self.tui_state.log_scroll = self.tui_state.log_scroll.saturating_add(1);
            }
            TuiAction::ScrollAgentUp => {
                let current = self.tui_state.agent_scroll.unwrap_or(0);
                self.tui_state.agent_scroll = Some(current.saturating_sub(1));
            }
            TuiAction::ScrollAgentDown => {
                let current = self.tui_state.agent_scroll.unwrap_or(0);
                self.tui_state.agent_scroll = Some(current.saturating_add(1));
            }
            TuiAction::ScrollAgentEnd => {
                self.tui_state.agent_scroll = None; // Resume auto-tail
            }
            TuiAction::ScrollDiffUp => {
                self.tui_state.diff_scroll = self.tui_state.diff_scroll.saturating_sub(1);
            }
            TuiAction::ScrollDiffDown => {
                self.tui_state.diff_scroll = self.tui_state.diff_scroll.saturating_add(1);
            }
            TuiAction::ScrollDetailUp => {
                self.tui_state.plan_detail_scroll =
                    self.tui_state.plan_detail_scroll.saturating_sub(1);
            }
            TuiAction::ScrollDetailDown => {
                self.tui_state.plan_detail_scroll =
                    self.tui_state.plan_detail_scroll.saturating_add(1);
            }
            TuiAction::ShowHelp => {
                self.tui_state.show_help = !self.tui_state.show_help;
            }
            TuiAction::ShowPlanDetail => {
                self.tui_state.show_plan_detail = !self.tui_state.show_plan_detail;
                // Also toggle legacy detail overlay
                self.toggle_detail_overlay();
            }
            TuiAction::ClosePlanDetail => {
                self.tui_state.show_plan_detail = false;
            }
            TuiAction::ShowTaskDetail => {
                self.tui_state.show_task_detail = !self.tui_state.show_task_detail;
            }
            TuiAction::CloseTaskDetail => {
                self.tui_state.show_task_detail = false;
            }
            TuiAction::ShowWaveOverview => {
                self.tui_state.show_wave_overview = !self.tui_state.show_wave_overview;
                if self.tui_state.show_wave_overview {
                    self.active_modal = Some(ModalState::WaveOverview {
                        waves: Vec::new(),
                        scroll_offset: 0,
                    });
                } else {
                    self.active_modal = None;
                }
            }
            TuiAction::ShowQueueOverview => {
                self.tui_state.show_queue_overview = !self.tui_state.show_queue_overview;
                if self.tui_state.show_queue_overview {
                    self.active_modal = Some(ModalState::QueueOverview {
                        milestones: Vec::new(),
                        selected_index: 0,
                        scroll_offset: 0,
                    });
                } else {
                    self.active_modal = None;
                }
            }
            TuiAction::OpenTaskPicker => {
                self.tui_state.show_task_picker = true;
                self.active_modal = Some(ModalState::TaskPicker {
                    tasks: Vec::new(),
                    selected_index: 0,
                    scroll_offset: 0,
                });
            }
            TuiAction::CloseTaskPicker => {
                self.tui_state.show_task_picker = false;
                self.active_modal = None;
            }
            TuiAction::ExpandCollapse => {
                if let Some(plan) = self.tui_state.plans.get_mut(self.tui_state.selected_plan_idx) {
                    plan.expanded = !plan.expanded;
                }
            }
            TuiAction::CollapseExpand => {
                if let Some(plan) = self.tui_state.plans.get_mut(self.tui_state.selected_plan_idx) {
                    plan.expanded = !plan.expanded;
                }
            }
            TuiAction::TogglePause => {
                self.tui_state.pipeline_run_state =
                    if self.tui_state.pipeline_run_state == "paused" {
                        "running".to_string()
                    } else {
                        "paused".to_string()
                    };
            }
            TuiAction::SwitchAgentTab(idx) => {
                if idx == usize::MAX {
                    // Cycle: backtick
                    let agent_count = self.tui_state.agents.len().max(1);
                    self.tui_state.selected_agent_tab =
                        (self.tui_state.selected_agent_tab + 1) % agent_count;
                } else {
                    self.tui_state.selected_agent_tab = idx;
                }
            }
            TuiAction::SwitchDetailTab(idx) => {
                self.tui_state.plan_detail_tab = idx;
            }
            TuiAction::ApproveCommand => {
                self.tui_state.pending_approval = None;
            }
            TuiAction::ApproveAll => {
                self.tui_state.pending_approval = None;
            }
            TuiAction::RejectCommand => {
                self.tui_state.pending_approval = None;
            }
            TuiAction::StartInject => {
                self.tui_state.input_mode = InputMode::Inject;
                self.tui_state.message_input.clear();
            }
            TuiAction::SubmitInject => {
                self.tui_state.input_mode = InputMode::Normal;
                // TODO: send message_input to agent
                self.tui_state.message_input.clear();
            }
            TuiAction::CancelInject => {
                self.tui_state.input_mode = InputMode::Normal;
                self.tui_state.message_input.clear();
            }
            TuiAction::InputChar(c) => {
                if self.tui_state.input_mode == InputMode::Inject {
                    self.tui_state.message_input.push(c);
                } else if self.tui_state.input_mode == InputMode::Filter {
                    self.tui_state.filter_text.push(c);
                }
            }
            TuiAction::InputBackspace => {
                if self.tui_state.input_mode == InputMode::Inject {
                    self.tui_state.message_input.pop();
                } else if self.tui_state.input_mode == InputMode::Filter {
                    self.tui_state.filter_text.pop();
                }
            }
            TuiAction::StartFilter => {
                self.tui_state.input_mode = InputMode::Filter;
                self.tui_state.filter_text.clear();
            }
            TuiAction::AcceptFilter => {
                self.tui_state.input_mode = InputMode::Normal;
                self.tui_state.filter_active = !self.tui_state.filter_text.is_empty();
            }
            TuiAction::CancelFilter => {
                self.tui_state.input_mode = InputMode::Normal;
                self.tui_state.filter_text.clear();
                self.tui_state.filter_active = false;
            }
            TuiAction::RequestConfirm(action) => {
                self.tui_state.input_mode = InputMode::Confirm;
                self.tui_state.pending_confirm = Some(action.clone());
                // Convert input::ConfirmAction to modals::ConfirmAction for the modal renderer
                let modal_action = modals::ConfirmAction::Custom {
                    message: action.to_string(),
                };
                self.active_modal = Some(ModalState::Confirm { action: modal_action });
            }
            TuiAction::ConfirmYes => {
                self.tui_state.input_mode = InputMode::Normal;
                // TODO: execute the confirmed action
                self.tui_state.pending_confirm = None;
                self.active_modal = None;
            }
            TuiAction::ConfirmNo => {
                self.tui_state.input_mode = InputMode::Normal;
                self.tui_state.pending_confirm = None;
                self.active_modal = None;
            }
            TuiAction::DismissNotification => {
                if !self.notifications.is_empty() {
                    self.notifications.remove(0);
                }
            }
            TuiAction::ToggleAgentPaneGroup => {
                self.tui_state.agent_pane_group = (self.tui_state.agent_pane_group + 1) % 2;
            }
            TuiAction::DrillIn => {
                if let Some(plan) = self.tui_state.plans.get_mut(self.tui_state.selected_plan_idx) {
                    plan.expanded = true;
                }
            }
            TuiAction::DrillOut => {
                if let Some(plan) = self.tui_state.plans.get_mut(self.tui_state.selected_plan_idx) {
                    plan.expanded = false;
                }
            }
            TuiAction::WaveNext => {
                let max = self.tui_state.plans.len().max(1);
                self.tui_state.selected_wave_idx = (self.tui_state.selected_wave_idx + 1) % max;
            }
            TuiAction::WavePrev => {
                let max = self.tui_state.plans.len().max(1);
                self.tui_state.selected_wave_idx =
                    self.tui_state.selected_wave_idx.checked_sub(1).unwrap_or(max - 1);
            }
            TuiAction::RestartPhase => {
                self.tui_state.input_mode = InputMode::Confirm;
                self.tui_state.pending_confirm = Some(ConfirmAction::RestartPhase);
                let modal_action = modals::ConfirmAction::Custom {
                    message: "Restart current phase?".to_string(),
                };
                self.active_modal = Some(ModalState::Confirm { action: modal_action });
            }
            TuiAction::RestartPlan => {
                if let Some(plan) = self.tui_state.plans.get(self.tui_state.selected_plan_idx) {
                    let plan_id = plan.id.clone();
                    self.tui_state.input_mode = InputMode::Confirm;
                    self.tui_state.pending_confirm =
                        Some(ConfirmAction::ResetSelectedPlan(plan_id.clone()));
                    let modal_action = modals::ConfirmAction::Custom {
                        message: format!("Restart plan '{plan_id}'?"),
                    };
                    self.active_modal = Some(ModalState::Confirm { action: modal_action });
                }
            }
            TuiAction::ForceAdvance => {
                if let Some(plan) = self.tui_state.plans.get(self.tui_state.selected_plan_idx) {
                    let plan_id = plan.id.clone();
                    self.tui_state.input_mode = InputMode::Confirm;
                    self.tui_state.pending_confirm =
                        Some(ConfirmAction::ForceAdvance(plan_id.clone()));
                    let modal_action = modals::ConfirmAction::Custom {
                        message: format!("Force-advance plan '{plan_id}'?"),
                    };
                    self.active_modal = Some(ModalState::Confirm { action: modal_action });
                }
            }
            TuiAction::ResetPlanState => {
                if let Some(plan) = self.tui_state.plans.get(self.tui_state.selected_plan_idx) {
                    let plan_id = plan.id.clone();
                    self.tui_state.input_mode = InputMode::Confirm;
                    self.tui_state.pending_confirm =
                        Some(ConfirmAction::ResetSelectedPlan(plan_id.clone()));
                    let modal_action = modals::ConfirmAction::Custom {
                        message: format!("Reset state for plan '{plan_id}'?"),
                    };
                    self.active_modal = Some(ModalState::Confirm { action: modal_action });
                }
            }
            TuiAction::ReverifyPlan => {
                if let Some(plan) = self.tui_state.plans.get(self.tui_state.selected_plan_idx) {
                    let plan_id = plan.id.clone();
                    self.tui_state.input_mode = InputMode::Confirm;
                    self.tui_state.pending_confirm =
                        Some(ConfirmAction::ReverifyPlan(plan_id.clone()));
                    let modal_action = modals::ConfirmAction::Custom {
                        message: format!("Re-verify plan '{plan_id}'?"),
                    };
                    self.active_modal = Some(ModalState::Confirm { action: modal_action });
                }
            }
            TuiAction::ConfigUp => {
                self.tui_state.config_selected = self.tui_state.config_selected.saturating_sub(1);
            }
            TuiAction::ConfigDown => {
                self.tui_state.config_selected = self.tui_state.config_selected.saturating_add(1);
            }
            // TODO: wire config editing when config view supports inline edits
            TuiAction::ConfigLeft | TuiAction::ConfigRight | TuiAction::ConfigSelect => {}
            TuiAction::MouseClick { x, y } => {
                // Use hit_test to determine zone
                let zones = super::hit_test::HitZones::compute(
                    super::layout::responsive_outer_margin(Rect::new(
                        0,
                        0,
                        80, // approximate; real values come from terminal size
                        24,
                    )),
                    self.tui_state.active_tab as usize,
                    Tab::ALL.len(),
                );
                if let Some(zone) = zones.zone_at(x, y) {
                    // Convert hit_test::FocusZone to input::FocusZone
                    let mapped = match zone {
                        super::hit_test::FocusZone::PlanTree => FocusZone::PlanTree,
                        super::hit_test::FocusZone::TaskProgress => FocusZone::TaskProgress,
                        super::hit_test::FocusZone::AgentOutput => FocusZone::AgentOutput,
                        super::hit_test::FocusZone::CommandOutput => FocusZone::CommandOutput,
                        super::hit_test::FocusZone::RightContent => FocusZone::RightPanel,
                        super::hit_test::FocusZone::HeaderTab(_) => FocusZone::PlanTree,
                        super::hit_test::FocusZone::DetailTab(_) => FocusZone::RightPanel,
                    };
                    self.tui_state.focus = mapped;
                }
            }
            TuiAction::MouseScrollUp { .. } => self.scroll_focused(-3),
            TuiAction::MouseScrollDown { .. } => self.scroll_focused(3),
            TuiAction::Refresh => self.refresh_snapshot(),
            TuiAction::None => {}
        }
    }

    fn scroll_focused(&mut self, delta: i32) {
        match self.tui_state.focus {
            FocusZone::PlanTree => {
                let current = self.tui_state.plan_scroll_offset as i32;
                self.tui_state.plan_scroll_offset =
                    (current + delta).max(0) as usize;
            }
            FocusZone::TaskProgress => {
                let current = self.tui_state.task_scroll as i32;
                self.tui_state.task_scroll = (current + delta).max(0) as usize;
            }
            FocusZone::AgentOutput => {
                let current = self.tui_state.agent_scroll.unwrap_or(0) as i32;
                self.tui_state.agent_scroll = Some((current + delta).max(0) as usize);
            }
            FocusZone::CommandOutput => {
                let current = self.tui_state.command_output_scroll as i32;
                self.tui_state.command_output_scroll =
                    (current + delta).max(0) as usize;
            }
            FocusZone::RightPanel => {
                let current = self.tui_state.diff_scroll as i32;
                self.tui_state.diff_scroll = (current + delta).max(0) as usize;
            }
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        let action = match mouse.kind {
            MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                TuiAction::MouseClick {
                    x: mouse.column,
                    y: mouse.row,
                }
            }
            MouseEventKind::ScrollUp => TuiAction::MouseScrollUp {
                x: mouse.column,
                y: mouse.row,
            },
            MouseEventKind::ScrollDown => TuiAction::MouseScrollDown {
                x: mouse.column,
                y: mouse.row,
            },
            _ => TuiAction::None,
        };
        self.dispatch_action(action);
    }

    // -----------------------------------------------------------------------
    // Rendering helpers
    // -----------------------------------------------------------------------

    fn render_tab_header(&self, frame: &mut Frame<'_>, area: Rect, _theme: &Theme) {
        // Use the Mori-ported header_bar widget with full progress/ETA/tokens
        super::widgets::header_bar::render_header_bar(frame, area, &self.tui_state);
    }

    fn render_status_footer(&self, frame: &mut Frame<'_>, area: Rect, _theme: &Theme) {
        // Use the Mori-ported status_bar widget with context-sensitive hints
        super::widgets::status_bar::render_status_bar(frame, area, &self.tui_state);
    }

    fn render_help_overlay(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let popup = super::layout::centered_rect(86, 84, area);
        frame.render_widget(Clear, popup);

        let lines = help_lines();
        let block = Block::default()
            .borders(Borders::ALL)
            .title("help")
            .border_style(theme.accent());
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        let paragraph = Paragraph::new(lines)
            .alignment(Alignment::Left)
            .style(theme.text())
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, inner);
    }

    fn expire_notifications(&mut self) {
        self.notifications.retain(|n| !n.is_expired());
    }

    // -----------------------------------------------------------------------
    // Legacy compatibility
    // -----------------------------------------------------------------------

    fn handle_overlay_key(&mut self, key: KeyEvent) -> bool {
        let Some(overlay) = self.overlay.clone() else {
            return false;
        };

        match key.code {
            KeyCode::Esc => {
                // Bug fix: close overlay instead of quitting
                self.overlay = None;
                true
            }
            KeyCode::Char('q') => {
                // Bug fix: close overlay on first q, quit on second
                self.overlay = None;
                true
            }
            KeyCode::Char('r') => {
                self.refresh_snapshot();
                true
            }
            KeyCode::Char('?') => {
                self.overlay = match overlay {
                    OverlayState::Help => None,
                    OverlayState::Detail(_) => Some(OverlayState::Help),
                };
                true
            }
            KeyCode::Enter => {
                if matches!(overlay, OverlayState::Detail(_)) {
                    self.overlay = None;
                }
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if matches!(overlay, OverlayState::Detail(_)) {
                    self.adjust_overlay_scroll(-1);
                }
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if matches!(overlay, OverlayState::Detail(_)) {
                    self.adjust_overlay_scroll(1);
                }
                true
            }
            KeyCode::PageUp => {
                if matches!(overlay, OverlayState::Detail(_)) {
                    self.adjust_overlay_scroll(-8);
                }
                true
            }
            KeyCode::PageDown => {
                if matches!(overlay, OverlayState::Detail(_)) {
                    self.adjust_overlay_scroll(8);
                }
                true
            }
            KeyCode::Home => {
                if matches!(overlay, OverlayState::Detail(_)) {
                    self.set_overlay_scroll(0);
                }
                true
            }
            // F-keys and tab switching should work even with overlay open
            KeyCode::F(n) if (1..=7).contains(&n) => false,
            KeyCode::Char('1'..='7') => false,
            _ => true,
        }
    }

    #[allow(dead_code)]
    fn select_page_by_slot(&mut self, slot: usize) {
        let pages = self.pages().ids();
        if let Some(page) = pages.get(slot).copied() {
            self.current_page = page;
            let _ = self.scaffold.set_active_page(self.current_page);
        }
    }

    fn refresh_snapshot(&mut self) {
        self.data = DashboardData::load_best_effort(&self.workdir);
        self.scaffold = DashboardScaffold::new_in(&self.workdir);
        self.tui_state.update_from_snapshot(&self.data);
        self.populate_git_info();
        self.last_refresh = Instant::now();
        self.clamp_signal_selection();
        self.clamp_gate_failure_selection();
        if self.pages().scaffold(self.current_page).is_none() {
            self.current_page = self.scaffold.active_page();
        }
    }

    /// Populate TuiState git fields from actual git commands.
    fn populate_git_info(&mut self) {
        // Branch
        if self.tui_state.git_branch.is_empty() {
            if let Ok(out) = std::process::Command::new("git")
                .args(["rev-parse", "--abbrev-ref", "HEAD"])
                .current_dir(&self.workdir)
                .output()
            {
                if out.status.success() {
                    self.tui_state.git_branch =
                        String::from_utf8_lossy(&out.stdout).trim().to_string();
                }
            }
        }
        // Short commit hash
        if self.tui_state.git_commit_short.is_empty() {
            if let Ok(out) = std::process::Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .current_dir(&self.workdir)
                .output()
            {
                if out.status.success() {
                    self.tui_state.git_commit_short =
                        String::from_utf8_lossy(&out.stdout).trim().to_string();
                }
            }
        }
        // Commit age
        if self.tui_state.git_age.is_empty() {
            if let Ok(out) = std::process::Command::new("git")
                .args(["log", "-1", "--format=%cr"])
                .current_dir(&self.workdir)
                .output()
            {
                if out.status.success() {
                    self.tui_state.git_age =
                        String::from_utf8_lossy(&out.stdout).trim().to_string();
                }
            }
        }
    }

    fn refresh_snapshot_if_needed(&mut self) {
        if self.last_refresh.elapsed() >= Duration::from_millis(250) {
            self.refresh_snapshot();
        }
    }

    fn pages(&self) -> PageRegistry {
        PageRegistry::from_dashboard(&self.scaffold)
    }

    #[allow(dead_code)]
    fn scroll_for(&self, page: PageId) -> u16 {
        self.scroll_offset.get(&page).copied().unwrap_or(0)
    }

    fn adjust_overlay_scroll(&mut self, delta: i16) {
        if let Some(OverlayState::Detail(detail)) = &mut self.overlay {
            detail.adjust_scroll(delta);
        }
    }

    fn set_overlay_scroll(&mut self, value: u16) {
        if let Some(OverlayState::Detail(detail)) = &mut self.overlay {
            detail.set_scroll(value);
        }
    }

    fn toggle_detail_overlay(&mut self) {
        let next = match self.current_page {
            PageId::Signals => self.signal_detail_overlay().map(OverlayState::Detail),
            PageId::GateResults => self.gate_failure_detail_overlay().map(OverlayState::Detail),
            _ => None,
        };

        if let Some(next) = next {
            let next_title = match &next {
                OverlayState::Detail(detail) => detail.title.clone(),
                OverlayState::Help => String::from("help"),
            };
            let current = self.overlay.clone();
            self.overlay = match current {
                Some(OverlayState::Detail(current)) if current.title == next_title => None,
                _ => Some(next),
            };
        }
    }

    fn clamp_signal_selection(&mut self) {
        let len = self.data.recent_signals.len();
        if len == 0 {
            self.signal_selection = 0;
        } else if self.signal_selection >= len {
            self.signal_selection = len - 1;
        }
    }

    fn clamp_gate_failure_selection(&mut self) {
        let len = self.data.gate_results_page.failure_rows.len();
        if len == 0 {
            self.gate_failure_selection = 0;
        } else if self.gate_failure_selection >= len {
            self.gate_failure_selection = len - 1;
        }
    }

    fn signal_detail_overlay(&self) -> Option<DetailState> {
        let signal = self.data.recent_signals.get(self.signal_selection)?;
        let raw = load_signal_entry(&self.workdir, &signal.id)?;
        let mut body = String::new();
        let _ = std::fmt::Write::write_fmt(
            &mut body,
            format_args!(
                "signal: {}\nkind: {}\ncreated: {}\nplan/task: {}\nlineage: {}\nparent: {}\n\nraw payload:\n{}\n",
                signal.id,
                signal.kind,
                signal.created_at_ms,
                signal
                    .plan_id
                    .as_deref()
                    .or(signal.task_id.as_deref())
                    .unwrap_or("-"),
                if signal.lineage.is_empty() {
                    String::from("-")
                } else {
                    signal.lineage.join(" -> ")
                },
                signal.parent_hash.as_deref().unwrap_or("-"),
                pretty_json(&raw)
            ),
        );
        Some(DetailState::new(format!("signal {}", signal.id), body))
    }

    fn gate_failure_detail_overlay(&self) -> Option<DetailState> {
        let row = self
            .data
            .gate_results_page
            .failure_rows
            .get(self.gate_failure_selection)?;
        let raw = load_gate_failure_entry(&self.workdir, row)?;
        let mut body = String::new();
        let _ = std::fmt::Write::write_fmt(
            &mut body,
            format_args!(
                "gate: {}\ntask: {}\ncreated: {}\nexcerpt: {}\n\nraw payload:\n{}\n",
                row.gate_name,
                row.task_id,
                row.created_at_ms,
                row.error_excerpt,
                pretty_json(&raw)
            ),
        );
        Some(DetailState::new(
            format!("gate failure {}", row.gate_name),
            body,
        ))
    }

    fn render_overlay(&self, frame: &mut Frame<'_>, overlay: &OverlayState) {
        let theme = Theme::from_env();
        let area = super::layout::centered_rect(86, 84, frame.area());
        frame.render_widget(Clear, area);

        match overlay {
            OverlayState::Help => {
                // Handled by render_help_overlay
            }
            OverlayState::Detail(detail) => {
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(detail.title.as_str())
                    .border_style(theme.warning());
                let inner = block.inner(area);
                frame.render_widget(block, area);

                let body = Paragraph::new(detail.body.as_str())
                    .style(theme.text().add_modifier(Modifier::BOLD))
                    .wrap(Wrap { trim: false })
                    .scroll((detail.scroll, 0));
                frame.render_widget(body, inner);
            }
        }
    }

    fn enter_terminal() -> Result<TuiTerminal> {
        enable_raw_mode().context("enable raw mode")?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .context("enter alternate screen")?;
        Terminal::new(CrosstermBackend::new(stdout)).context("create terminal")
    }

    fn leave_terminal() -> Result<()> {
        Self::cleanup_terminal()
    }

    fn cleanup_terminal() -> Result<()> {
        disable_raw_mode().context("disable raw mode")?;
        execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)
            .context("leave alternate screen")?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map a Mori-style Tab to a legacy PageId (best effort).
fn tab_to_page(tab: Tab) -> Option<PageId> {
    match tab {
        Tab::Dashboard => Some(PageId::Health),
        Tab::Plans => Some(PageId::PlanView),
        Tab::Agents => Some(PageId::AgentStatus),
        Tab::Logs => Some(PageId::LogView),
        Tab::Config => Some(PageId::ConfigView),
        Tab::Git | Tab::Inspect => None,
    }
}

#[derive(Debug, Clone)]
struct DetailState {
    title: String,
    body: String,
    scroll: u16,
}

impl DetailState {
    fn new(title: String, body: String) -> Self {
        Self {
            title,
            body,
            scroll: 0,
        }
    }

    fn adjust_scroll(&mut self, delta: i16) {
        let current = self.scroll as i32;
        self.scroll = (current + delta as i32).max(0).min(u16::MAX as i32) as u16;
    }

    fn set_scroll(&mut self, value: u16) {
        self.scroll = value;
    }
}

fn help_lines() -> Vec<Line<'static>> {
    let theme = Theme::from_env();
    vec![
        Line::from(Span::styled(
            "roko dashboard keybindings",
            theme.accent_bold(),
        )),
        Line::from(""),
        Line::from(Span::styled("Navigation", theme.accent_bold())),
        Line::from("F1-F7      switch tabs (Dashboard/Plans/Agents/Git/Logs/Config/Inspect)"),
        Line::from("F8 / u     queue overview modal"),
        Line::from("Tab        cycle focus between panels"),
        Line::from("Shift+Tab  cycle focus backward"),
        Line::from("j/k ↑/↓    scroll focused panel"),
        Line::from("PgUp/PgDn  page scroll"),
        Line::from("Enter      expand/drill into selection"),
        Line::from("Esc        close overlay / drill out"),
        Line::from("q          close overlay or quit"),
        Line::from(""),
        Line::from(Span::styled(
            "Dashboard Sub-Tabs (F1)",
            theme.accent_bold(),
        )),
        Line::from("a          Agents panel"),
        Line::from("o          Output panel"),
        Line::from("d          Diff panel"),
        Line::from("e          Errors panel"),
        Line::from("g          Git panel"),
        Line::from("m          MCP / Context panel"),
        Line::from("P          Processes panel"),
        Line::from(""),
        Line::from(Span::styled("Modals & Modes", theme.accent_bold())),
        Line::from("?          toggle this help"),
        Line::from("w          wave overview"),
        Line::from("p          pause/resume pipeline"),
        Line::from("i          inject message to agent"),
        Line::from("/          filter mode (Plans/Logs)"),
        Line::from("Ctrl-t     task picker"),
        Line::from("Ctrl-a     approve all pending"),
        Line::from("Ctrl-x     force advance (confirm)"),
        Line::from("Ctrl-d     reset selected plan (confirm)"),
        Line::from(""),
        Line::from(Span::styled("Agent Controls (F3)", theme.accent_bold())),
        Line::from("y          approve pending command"),
        Line::from("A          approve all pending"),
        Line::from("x          reject pending command"),
        Line::from("`          cycle agent tabs"),
        Line::from("1-7        switch agent tab directly"),
        Line::from("G/End      resume auto-scroll"),
        Line::from(""),
        Line::from(Span::styled("Plans (F2)", theme.accent_bold())),
        Line::from("e          expand/collapse plan"),
        Line::from("[/]        wave prev/next"),
        Line::from("h/l ←/→    drill out/in"),
        Line::from("s          soft retry plan"),
        Line::from("R          restart phase"),
        Line::from("F          force advance"),
        Line::from("V / c      re-verify plan"),
        Line::from("S          repair (preserve completed)"),
        Line::from("t          task picker"),
        Line::from("o          queue overview"),
        Line::from(""),
        Line::from(Span::styled("General", theme.accent_bold())),
        Line::from("Ctrl-r     refresh data"),
        Line::from("Ctrl-C     quit immediately"),
    ]
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn load_signal_entry(workdir: &Path, signal_id: &str) -> Option<Value> {
    let path = workdir.join(".roko").join("signals.jsonl");
    super::dashboard::read_jsonl_values(&path)
        .into_iter()
        .rev()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(signal_id))
}

fn load_gate_failure_entry(
    workdir: &Path,
    row: &super::dashboard::GateFailureRow,
) -> Option<Value> {
    let path = workdir.join(".roko").join("signals.jsonl");
    super::dashboard::read_jsonl_values(&path)
        .into_iter()
        .rev()
        .find(|entry| {
            let kind = entry
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default();
            is_gate_result_kind(kind)
                && entry
                    .pointer("/tags/gate")
                    .and_then(Value::as_str)
                    .or_else(|| entry.pointer("/body/data/gate").and_then(Value::as_str))
                    .or_else(|| entry.pointer("/body/gate").and_then(Value::as_str))
                    == Some(row.gate_name.as_str())
                && entry
                    .pointer("/tags/task_id")
                    .and_then(Value::as_str)
                    .or_else(|| entry.pointer("/body/data/task_id").and_then(Value::as_str))
                    .or_else(|| entry.pointer("/body/task_id").and_then(Value::as_str))
                    == Some(row.task_id.as_str())
                && entry_timestamp_ms(entry) == Some(row.created_at_ms)
        })
}

fn entry_timestamp_ms(entry: &Value) -> Option<i64> {
    entry
        .get("created_at_ms")
        .and_then(Value::as_i64)
        .or_else(|| {
            entry
                .get("created_at_ms")
                .and_then(Value::as_u64)
                .map(|ts| ts as i64)
        })
}

fn is_gate_result_kind(kind: &str) -> bool {
    kind == "gate_verdict" || kind.starts_with("gate:") || kind.starts_with("gate_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn app_starts_on_requested_page() {
        let dir = tempdir().unwrap();
        let app = App::new_with_page(dir.path(), Some(PageId::PlanView));
        assert_eq!(app.current_page(), PageId::PlanView);
    }

    #[test]
    fn app_has_tui_state() {
        let dir = tempdir().unwrap();
        let app = App::new(dir.path());
        assert_eq!(app.tui_state.active_tab, Tab::Dashboard);
        assert_eq!(app.tui_state.input_mode, InputMode::Normal);
    }
}
