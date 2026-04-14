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
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, size,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use serde_json::Value;
use sysinfo::System;

use super::atmosphere::Atmosphere;
use super::dashboard::{DashboardData, DashboardScaffold, Theme};
use super::effects_config::EffectsConfig;
use super::event::{Event, EventHandler};
use super::input::{self, ConfirmAction, FocusZone, InputMode, TuiAction};
use super::modals::{self, Milestone, ModalState, QueueTask, TaskPickerRow, WaveInfo, WavePlanEntry};
use super::pages::{PageId, PageRegistry};
use super::state::{PlanEntry, TaskRowStatus, TuiState};
use super::tabs::Tab;
use super::views::{self, ViewState};

const PAGE_SCROLL_LINES: i32 = 20;

/// Interactive dashboard shell backed by the existing snapshot renderer.
///
/// Supports two rendering paths:
/// - **Mori-style tabs** (F1-F7): full TuiState + views + modals + postfx
/// - **Legacy scaffold pages**: original PageId-based rendering
///
/// All expensive I/O (system metrics, file reads, git commands) runs on
/// background threads.  The render path does zero I/O -- it only reads
/// `&self.tui_state` and `&self.data` and writes to the frame buffer.
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

    // -- Background I/O channels --
    /// Background system metrics receiver (CPU/MEM collected off main thread).
    sys_rx: Option<std::sync::mpsc::Receiver<super::state::SysMetrics>>,
    /// Background data refresh receiver (file reads off main thread).
    data_rx: Option<std::sync::mpsc::Receiver<DashboardData>>,
    /// Background git data receiver (git commands off main thread).
    git_rx: Option<std::sync::mpsc::Receiver<GitBgData>>,
    /// Frame counter for adaptive frame rate.
    frame_counter: u64,
    /// Last user input time for adaptive frame rate.
    last_input: Instant,
    /// Last known terminal size used for hit-testing.
    terminal_size: (u16, u16),
}

/// Bundle of git data collected by the background git thread.
struct GitBgData {
    /// Full git view data for the F4 Git tab.
    view_data: super::views::git_view::GitViewData,
    /// Summary lines for the dashboard sub-tab.
    summary_lines: Vec<String>,
    /// Git branch name.
    branch: String,
    /// Short commit hash.
    commit_short: String,
    /// Commit age string (e.g. "3 hours ago").
    age: String,
}

fn plan_status_label(plan: &PlanEntry) -> String {
    if !plan.phase.is_empty() {
        plan.phase.clone()
    } else if !plan.status.is_empty() {
        plan.status.clone()
    } else if plan.active {
        "active".to_string()
    } else {
        "pending".to_string()
    }
}

fn task_status_label(status: TaskRowStatus) -> &'static str {
    match status {
        TaskRowStatus::Pending => "pending",
        TaskRowStatus::Active => "active",
        TaskRowStatus::Done => "done",
        TaskRowStatus::Failed => "failed",
        TaskRowStatus::Blocked => "blocked",
    }
}

fn execution_waves_for_modal(state: &TuiState) -> Vec<WaveInfo> {
    let plans_by_id: HashMap<&str, &PlanEntry> = state
        .plans
        .iter()
        .map(|plan| (plan.id.as_str(), plan))
        .collect();

    state
        .execution_waves
        .iter()
        .map(|wave| WaveInfo {
            wave_index: wave.index,
            plans: wave
                .plans
                .iter()
                .map(|plan_id| {
                    if let Some(plan) = plans_by_id.get(plan_id.as_str()) {
                        WavePlanEntry {
                            plan_id: plan.id.clone(),
                            status: plan_status_label(plan),
                            duration_secs: Some(plan.elapsed_secs.max(0.0) as u64),
                        }
                    } else {
                        WavePlanEntry {
                            plan_id: plan_id.clone(),
                            status: "queued".to_string(),
                            duration_secs: None,
                        }
                    }
                })
                .collect(),
            total_duration_secs: Some(
                wave.plans
                    .iter()
                    .filter_map(|plan_id| plans_by_id.get(plan_id.as_str()))
                    .map(|plan| plan.elapsed_secs.max(0.0) as u64)
                    .sum(),
            ),
            eta_secs: None,
        })
        .collect()
}

fn queue_overview_milestones(state: &TuiState) -> Vec<Milestone> {
    let plans_by_id: HashMap<&str, &PlanEntry> = state
        .plans
        .iter()
        .map(|plan| (plan.id.as_str(), plan))
        .collect();

    state
        .execution_waves
        .iter()
        .map(|wave| Milestone {
            name: format!("Wave {}", wave.index),
            tasks: wave
                .plans
                .iter()
                .map(|plan_id| {
                    if let Some(plan) = plans_by_id.get(plan_id.as_str()) {
                        QueueTask {
                            id: plan.id.clone(),
                            title: if plan.name.is_empty() {
                                plan.id.clone()
                            } else {
                                plan.name.clone()
                            },
                            status: plan_status_label(plan),
                        }
                    } else {
                        QueueTask {
                            id: plan_id.clone(),
                            title: plan_id.clone(),
                            status: "queued".to_string(),
                        }
                    }
                })
                .collect(),
            completed: wave.done,
            total: wave.total,
        })
        .collect()
}

fn task_picker_rows(state: &TuiState) -> Vec<TaskPickerRow> {
    let plan_num = state.current_plan_idx.saturating_add(1) as u32;

    state
        .current_task_checklist
        .iter()
        .map(|task| TaskPickerRow {
            plan_num,
            task_id: task.id.clone(),
            title: task.title.clone(),
            status: task_status_label(task.status).to_string(),
        })
        .collect()
}

fn collect_git_age(workdir: &Path) -> String {
    std::process::Command::new("git")
        .args(["log", "-1", "--format=%cr"])
        .current_dir(workdir)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default()
}

fn convert_git_branch_tree(
    branches: &[super::views::git_view::GitBranchNode],
) -> Vec<super::state::GitBranchNode> {
    branches
        .iter()
        .map(|branch| super::state::GitBranchNode {
            name: branch.name.clone(),
            is_current: branch.is_current,
            ahead: branch.ahead as usize,
            behind: branch.behind as usize,
            children: Vec::new(),
        })
        .collect()
}

fn convert_git_commit_graph(
    commits: &[super::views::git_view::CommitEntry],
) -> Vec<super::state::GitCommitEntry> {
    commits
        .iter()
        .map(|commit| super::state::GitCommitEntry {
            hash: commit.hash_short.clone(),
            short_hash: commit.hash_short.clone(),
            message: commit.subject.clone(),
            author: commit.author.clone(),
            timestamp_ms: 0,
            branch: None,
        })
        .collect()
}

fn convert_git_worktree_list(worktrees: &[super::views::git_view::WorktreeEntry]) -> Vec<String> {
    worktrees
        .iter()
        .map(|worktree| worktree.path.clone())
        .collect()
}

// Manual Debug impl because mpsc::Receiver does not implement Debug.
impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("workdir", &self.workdir)
            .field("running", &self.running)
            .field("current_page", &self.current_page)
            .field("frame_counter", &self.frame_counter)
            .finish_non_exhaustive()
    }
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
                crossterm::event::Event::Resize(width, height) => {
                    app.terminal_size = (width, height);
                }
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
        let terminal_size = size().unwrap_or((80, 24));
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
            sys_rx: None,
            data_rx: None,
            git_rx: None,
            frame_counter: 0,
            last_input: Instant::now(),
            terminal_size,
        };
        // Populate git info synchronously on first load (fast enough for startup).
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

        // ---------------------------------------------------------------
        // Spawn background sys metrics collector thread
        // ---------------------------------------------------------------
        let (sys_tx, sys_rx) = std::sync::mpsc::channel::<super::state::SysMetrics>();
        std::thread::Builder::new()
            .name("tui-sys-metrics".into())
            .spawn(move || {
                collect_sys_metrics_bg(sys_tx);
            })
            .ok(); // graceful: TUI works without background thread
        self.sys_rx = Some(sys_rx);

        // ---------------------------------------------------------------
        // Spawn background data refresh thread
        // ---------------------------------------------------------------
        let (data_tx, data_rx) = std::sync::mpsc::channel::<DashboardData>();
        let data_workdir = self.workdir.clone();
        std::thread::Builder::new()
            .name("tui-data-refresh".into())
            .spawn(move || {
                loop {
                    let data = DashboardData::load_best_effort(&data_workdir);
                    if data_tx.send(data).is_err() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            })
            .ok();
        self.data_rx = Some(data_rx);

        // ---------------------------------------------------------------
        // Spawn background git data collector thread
        // ---------------------------------------------------------------
        let (git_tx, git_rx) = std::sync::mpsc::channel::<GitBgData>();
        let git_workdir = self.workdir.clone();
        std::thread::Builder::new()
            .name("tui-git-refresh".into())
            .spawn(move || {
                loop {
                    let view_data = super::views::git_view::collect_git_data();
                    let summary_lines = super::views::dashboard_view::collect_git_summary();
                    let branch = view_data.current_branch.clone();
                    let commit_short = view_data
                        .commits
                        .first()
                        .map(|c| c.hash_short.clone())
                        .unwrap_or_default();
                    let age = collect_git_age(&git_workdir);
                    let bg = GitBgData {
                        view_data,
                        summary_lines,
                        branch,
                        commit_short,
                        age,
                    };
                    if git_tx.send(bg).is_err() {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(3));
                }
            })
            .ok();
        self.git_rx = Some(git_rx);

        // ---------------------------------------------------------------
        // Initial draw
        // ---------------------------------------------------------------
        terminal
            .draw(|frame| self.draw(frame))
            .context("initial TUI draw")?;

        // ---------------------------------------------------------------
        // Event loop
        // ---------------------------------------------------------------
        while self.running {
            match events.next().context("poll TUI event")? {
                Event::Key(key) => {
                    self.last_input = Instant::now();
                    self.handle_key(key);
                    // Drain background channels before immediate redraw
                    self.drain_background_channels();
                    terminal
                        .draw(|frame| self.draw(frame))
                        .context("TUI redraw after key")?;
                    continue;
                }
                Event::Mouse(mouse) => {
                    self.last_input = Instant::now();
                    self.handle_mouse(mouse);
                }
                Event::Resize(width, height) => {
                    self.terminal_size = (width, height);
                }
                Event::Tick => {
                    self.atmosphere.tick();
                    self.tui_state.atmosphere.tick();
                    self.drain_background_channels();
                    self.expire_notifications();
                }
            }

            // Adaptive frame rate: skip frames when idle with no agents
            let user_idle = self.last_input.elapsed() > Duration::from_secs(3);
            let has_agents = self.tui_state.active_agent_count() > 0;
            let should_draw = if user_idle && !has_agents {
                self.frame_counter % 3 == 0 // ~20fps when idle
            } else {
                true // ~60fps when active
            };

            if should_draw {
                terminal
                    .draw(|frame| self.draw(frame))
                    .context("TUI redraw")?;
            }
            self.frame_counter = self.frame_counter.wrapping_add(1);
        }

        Ok(())
    }

    fn draw(&self, frame: &mut Frame<'_>) {
        let theme = Theme::from_env();
        let full_area = frame.area();

        // Responsive outer margin on large terminals
        let content_area = super::layout::responsive_outer_margin(full_area);

        // Main layout: header (1 line) + wave row (1 line) + content + footer (1 line)
        let has_waves = !self.tui_state.execution_waves.is_empty();
        let wave_row_height = if has_waves { 1 } else { 0 };
        let main_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),               // Mori-style header bar
                Constraint::Length(wave_row_height), // Wave indicator row (hidden when idle)
                Constraint::Min(0),                  // Content area
                Constraint::Length(1),               // Status footer
            ])
            .split(content_area);

        // Header: Mori header bar
        self.render_tab_header(frame, main_layout[0], &theme);

        // Wave indicator row (only when waves exist)
        if has_waves {
            super::widgets::wave_progress::render_wave_progress(
                frame,
                main_layout[1],
                &self.tui_state,
            );
        }

        // Content: dispatch to active tab view
        // Layout always has 4 slots: [0]=header [1]=wave [2]=content [3]=footer
        // The wave slot is 0-height when idle, but indices don't change.
        let content_idx = 2;
        let footer_idx = 3;

        let view_state = ViewState {
            scroll: self.tui_state.plan_scroll_offset as u16,
            selected: self.tui_state.selected_plan_idx,
            sub_tab: self.tui_state.plan_detail_tab,
            secondary_selected: 0,
            auto_tail: self.tui_state.agent_scroll.is_none(),
        };
        views::render_tab_content(
            frame,
            main_layout[content_idx],
            self.tui_state.active_tab,
            &self.data,
            &self.tui_state,
            &view_state,
            &theme,
        );

        // Footer: status line
        self.render_status_footer(frame, main_layout[footer_idx], &theme);

        if matches!(
            self.tui_state.input_mode,
            InputMode::Inject | InputMode::Filter
        ) {
            let input_area = Rect::new(
                content_area.x,
                content_area.bottom().saturating_sub(1),
                content_area.width,
                1,
            );
            frame.render_widget(Clear, input_area);

            let (label, buf) = match self.tui_state.input_mode {
                InputMode::Inject => ("inject> ", self.tui_state.message_input.as_str()),
                InputMode::Filter => ("filter> ", self.tui_state.filter_text.as_str()),
                _ => unreachable!(),
            };
            let input_line = Line::from(vec![
                Span::styled(label, theme.accent_bold()),
                Span::styled(buf, theme.text()),
                Span::styled(" ", theme.selection()),
            ]);
            frame.render_widget(Paragraph::new(input_line), input_area);
        }

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
            &self.data,
            &self.tui_state,
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
            TuiAction::ScrollPageUp => self.scroll_focused(-PAGE_SCROLL_LINES),
            TuiAction::ScrollPageDown => self.scroll_focused(PAGE_SCROLL_LINES),
            TuiAction::ScrollFocusedHome => self.set_focused_scroll(0),
            TuiAction::ScrollFocusedEnd => self.set_focused_scroll(usize::MAX),
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
                if let Some(ModalState::PlanDetail { scroll_offset, .. }) =
                    self.active_modal.as_mut()
                {
                    *scroll_offset = scroll_offset.saturating_sub(1);
                }
                self.tui_state.plan_detail_scroll =
                    self.tui_state.plan_detail_scroll.saturating_sub(1);
            }
            TuiAction::ScrollDetailDown => {
                if let Some(ModalState::PlanDetail { scroll_offset, .. }) =
                    self.active_modal.as_mut()
                {
                    *scroll_offset = scroll_offset.saturating_add(1);
                }
                self.tui_state.plan_detail_scroll =
                    self.tui_state.plan_detail_scroll.saturating_add(1);
            }
            TuiAction::ShowHelp => {
                self.tui_state.show_help = !self.tui_state.show_help;
            }
            TuiAction::ShowPlanDetail => {
                if self.tui_state.plans.is_empty() {
                    self.tui_state.show_plan_detail = false;
                    self.active_modal = None;
                } else {
                    self.tui_state.show_plan_detail = true;
                    self.tui_state.plan_detail_scroll = 0;
                    self.active_modal = Some(ModalState::PlanDetail {
                        plan_idx: self.tui_state.selected_plan_idx,
                        scroll_offset: 0,
                    });
                }
            }
            TuiAction::ClosePlanDetail => {
                self.tui_state.show_plan_detail = false;
                if matches!(self.active_modal, Some(ModalState::PlanDetail { .. })) {
                    self.active_modal = None;
                }
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
                        waves: execution_waves_for_modal(&self.tui_state),
                        scroll_offset: 0,
                    });
                } else {
                    self.active_modal = None;
                }
            }
            TuiAction::ShowQueueOverview => {
                self.tui_state.show_queue_overview = !self.tui_state.show_queue_overview;
                if self.tui_state.show_queue_overview {
                    let milestones = queue_overview_milestones(&self.tui_state);
                    self.active_modal = Some(ModalState::QueueOverview {
                        selected_index: self
                            .tui_state
                            .current_wave()
                            .min(milestones.len().saturating_sub(1)),
                        scroll_offset: self.tui_state.current_wave() as u16,
                        milestones,
                    });
                } else {
                    self.active_modal = None;
                }
            }
            TuiAction::OpenTaskPicker => {
                self.tui_state.show_task_picker = true;
                let tasks = task_picker_rows(&self.tui_state);
                let selected_index = self.tui_state.task_scroll.min(tasks.len().saturating_sub(1));
                self.active_modal = Some(ModalState::TaskPicker {
                    tasks,
                    selected_index,
                    scroll_offset: selected_index as u16,
                });
            }
            TuiAction::CloseTaskPicker => {
                self.tui_state.show_task_picker = false;
                self.active_modal = None;
            }
            TuiAction::ExpandCollapse => {
                if let Some(plan) = self
                    .tui_state
                    .plans
                    .get_mut(self.tui_state.selected_plan_idx)
                {
                    plan.expanded = !plan.expanded;
                }
            }
            TuiAction::CollapseExpand => {
                if let Some(plan) = self
                    .tui_state
                    .plans
                    .get_mut(self.tui_state.selected_plan_idx)
                {
                    plan.expanded = !plan.expanded;
                }
            }
            TuiAction::TogglePause => {
                self.tui_state.pipeline_run_state = if self.tui_state.pipeline_run_state == "paused"
                {
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
                let msg = self.tui_state.message_input.clone();
                self.tui_state.input_mode = InputMode::Normal;
                self.tui_state.message_input.clear();
                if !msg.is_empty() {
                    // Write inject signal to .roko/signals.jsonl for the orchestrator
                    let signal_path = self.workdir.join(".roko").join("signals.jsonl");
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    let entry = serde_json::json!({
                        "id": format!("inject-{ts}"),
                        "kind": "roko.inject.directive",
                        "created_at_ms": ts,
                        "payload": { "message": msg },
                    });
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&signal_path)
                    {
                        use std::io::Write;
                        let _ = writeln!(f, "{}", entry);
                    }
                    self.notifications
                        .push(super::modals::Notification::info(format!(
                            "Injected: {}",
                            truncate_str(&msg, 40)
                        )));
                }
            }
            TuiAction::CancelInject => {
                self.tui_state.input_mode = InputMode::Normal;
                self.tui_state.message_input.clear();
            }
            TuiAction::InputChar(c) => {
                if self.tui_state.input_mode == InputMode::ConfigEdit {
                    self.tui_state.config_edit_buffer.push(c);
                } else if self.tui_state.input_mode == InputMode::Inject {
                    self.tui_state.message_input.push(c);
                } else if self.tui_state.input_mode == InputMode::Filter {
                    self.tui_state.filter_text.push(c);
                }
            }
            TuiAction::InputBackspace => {
                if self.tui_state.input_mode == InputMode::ConfigEdit {
                    self.tui_state.config_edit_buffer.pop();
                } else if self.tui_state.input_mode == InputMode::Inject {
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
                self.tui_state.filter = self.tui_state.filter_text.clone();
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
                self.active_modal = Some(ModalState::Confirm {
                    action: modal_action,
                });
            }
            TuiAction::ConfirmYes => {
                self.tui_state.input_mode = InputMode::Normal;
                // Execute the confirmed action by writing a signal
                if let Some(action) = &self.tui_state.pending_confirm {
                    let action_str = action.to_string();
                    let signal_path = self.workdir.join(".roko").join("signals.jsonl");
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    let entry = serde_json::json!({
                        "id": format!("confirm-{ts}"),
                        "kind": "roko.tui.confirm",
                        "created_at_ms": ts,
                        "payload": { "action": action_str },
                    });
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&signal_path)
                    {
                        use std::io::Write;
                        let _ = writeln!(f, "{}", entry);
                    }
                    self.notifications
                        .push(super::modals::Notification::info(format!(
                            "Confirmed: {}",
                            truncate_str(&action_str, 40)
                        )));
                }
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
                if let Some(plan) = self
                    .tui_state
                    .plans
                    .get_mut(self.tui_state.selected_plan_idx)
                {
                    plan.expanded = true;
                }
            }
            TuiAction::DrillOut => {
                if let Some(plan) = self
                    .tui_state
                    .plans
                    .get_mut(self.tui_state.selected_plan_idx)
                {
                    plan.expanded = false;
                }
            }
            TuiAction::WaveNext => {
                let max = self.tui_state.plans.len().max(1);
                self.tui_state.selected_wave_idx = (self.tui_state.selected_wave_idx + 1) % max;
            }
            TuiAction::WavePrev => {
                let max = self.tui_state.plans.len().max(1);
                self.tui_state.selected_wave_idx = self
                    .tui_state
                    .selected_wave_idx
                    .checked_sub(1)
                    .unwrap_or(max - 1);
            }
            TuiAction::RestartPhase => {
                self.tui_state.input_mode = InputMode::Confirm;
                self.tui_state.pending_confirm = Some(ConfirmAction::RestartPhase);
                let modal_action = modals::ConfirmAction::Custom {
                    message: "Restart current phase?".to_string(),
                };
                self.active_modal = Some(ModalState::Confirm {
                    action: modal_action,
                });
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
                    self.active_modal = Some(ModalState::Confirm {
                        action: modal_action,
                    });
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
                    self.active_modal = Some(ModalState::Confirm {
                        action: modal_action,
                    });
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
                    self.active_modal = Some(ModalState::Confirm {
                        action: modal_action,
                    });
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
                    self.active_modal = Some(ModalState::Confirm {
                        action: modal_action,
                    });
                }
            }
            TuiAction::ConfigUp => {
                self.tui_state.config_cursor = self.tui_state.config_cursor.saturating_sub(1);
                // Skip headers when navigating up
                let items = super::config_meta::build_flat_items(
                    &self.workdir,
                    &self.tui_state.config_pending,
                );
                while self.tui_state.config_cursor > 0 {
                    if let Some(super::config_meta::ConfigItem::Header(_)) =
                        items.get(self.tui_state.config_cursor)
                    {
                        self.tui_state.config_cursor =
                            self.tui_state.config_cursor.saturating_sub(1);
                    } else {
                        break;
                    }
                }
            }
            TuiAction::ConfigDown => {
                let items = super::config_meta::build_flat_items(
                    &self.workdir,
                    &self.tui_state.config_pending,
                );
                let max_idx = items.len().saturating_sub(1);
                self.tui_state.config_cursor = (self.tui_state.config_cursor + 1).min(max_idx);
                // Skip headers when navigating down
                while self.tui_state.config_cursor < max_idx {
                    if let Some(super::config_meta::ConfigItem::Header(_)) =
                        items.get(self.tui_state.config_cursor)
                    {
                        self.tui_state.config_cursor += 1;
                    } else {
                        break;
                    }
                }
            }
            TuiAction::ConfigToggle => {
                let items = super::config_meta::build_flat_items(
                    &self.workdir,
                    &self.tui_state.config_pending,
                );
                if let Some(item) = items.get(self.tui_state.config_cursor) {
                    match item {
                        super::config_meta::ConfigItem::Field {
                            meta,
                            value,
                            source,
                        } => {
                            match &meta.kind {
                                super::config_meta::ConfigFieldKind::Bool => {
                                    let new_val = if value == "true" { "false" } else { "true" };
                                    self.tui_state
                                        .config_pending
                                        .insert(meta.key.to_string(), new_val.to_string());
                                }
                                super::config_meta::ConfigFieldKind::ReadOnly => {}
                                super::config_meta::ConfigFieldKind::Enum(_)
                                | super::config_meta::ConfigFieldKind::Int { .. } => {
                                    // For enums/presets, Enter cycles right
                                    if *source != super::config_meta::ConfigSource::Env {
                                        if let Some(new_val) = cycle_field_value(meta, value, true)
                                        {
                                            self.tui_state
                                                .config_pending
                                                .insert(meta.key.to_string(), new_val);
                                        }
                                    }
                                }
                                _ => {
                                    // Start text edit for free-form fields
                                    if *source != super::config_meta::ConfigSource::Env {
                                        self.tui_state.config_editing = true;
                                        self.tui_state.config_edit_buffer = value.clone();
                                        self.tui_state.config_edit_key = Some(meta.key.to_string());
                                        self.tui_state.input_mode = InputMode::ConfigEdit;
                                    }
                                }
                            }
                        }
                        super::config_meta::ConfigItem::SaveButton => {
                            // Inline save logic
                            if self.tui_state.config_pending.is_empty() {
                                self.notifications.push(super::modals::Notification::info(
                                    "No pending changes to save",
                                ));
                            } else {
                                match super::config_meta::save_pending_edits(
                                    &self.workdir,
                                    &self.tui_state.config_pending,
                                ) {
                                    Ok(()) => {
                                        let count = self.tui_state.config_pending.len();
                                        self.tui_state.config_pending.clear();
                                        self.notifications.push(super::modals::Notification::info(
                                            &format!("Config saved ({count} changes written to roko.toml)"),
                                        ));
                                    }
                                    Err(e) => {
                                        self.notifications.push(
                                            super::modals::Notification::error(&format!(
                                                "Save failed: {e}"
                                            )),
                                        );
                                    }
                                }
                            }
                        }
                        super::config_meta::ConfigItem::Header(_) => {}
                    }
                }
            }
            TuiAction::ConfigCycleLeft | TuiAction::ConfigCycleRight => {
                let items = super::config_meta::build_flat_items(
                    &self.workdir,
                    &self.tui_state.config_pending,
                );
                if let Some(super::config_meta::ConfigItem::Field {
                    meta,
                    value,
                    source,
                }) = items.get(self.tui_state.config_cursor)
                {
                    if *source == super::config_meta::ConfigSource::Env {
                        // Env-overridden: not editable
                    } else {
                        let direction = matches!(action, TuiAction::ConfigCycleRight);
                        if let Some(new_val) = cycle_field_value(meta, value, direction) {
                            self.tui_state
                                .config_pending
                                .insert(meta.key.to_string(), new_val);
                        }
                    }
                }
            }
            TuiAction::ConfigStartEdit => {
                let items = super::config_meta::build_flat_items(
                    &self.workdir,
                    &self.tui_state.config_pending,
                );
                if let Some(super::config_meta::ConfigItem::Field {
                    meta,
                    value,
                    source,
                }) = items.get(self.tui_state.config_cursor)
                {
                    if *source != super::config_meta::ConfigSource::Env
                        && !matches!(meta.kind, super::config_meta::ConfigFieldKind::ReadOnly)
                    {
                        self.tui_state.config_editing = true;
                        self.tui_state.config_edit_buffer = value.clone();
                        self.tui_state.config_edit_key = Some(meta.key.to_string());
                        self.tui_state.input_mode = InputMode::ConfigEdit;
                    }
                }
            }
            TuiAction::ConfigCommitEdit => {
                if self.tui_state.config_editing {
                    if let Some(key) = self.tui_state.config_edit_key.take() {
                        let val = self.tui_state.config_edit_buffer.clone();
                        self.tui_state.config_pending.insert(key, val);
                    }
                    self.tui_state.config_editing = false;
                    self.tui_state.config_edit_buffer.clear();
                    self.tui_state.input_mode = InputMode::Normal;
                }
            }
            TuiAction::ConfigCancelEdit => {
                self.tui_state.config_editing = false;
                self.tui_state.config_edit_buffer.clear();
                self.tui_state.config_edit_key = None;
                self.tui_state.input_mode = InputMode::Normal;
            }
            TuiAction::ConfigSave => {
                if self.tui_state.config_pending.is_empty() {
                    self.notifications.push(super::modals::Notification::info(
                        "No pending changes to save",
                    ));
                } else {
                    match super::config_meta::save_pending_edits(
                        &self.workdir,
                        &self.tui_state.config_pending,
                    ) {
                        Ok(()) => {
                            let count = self.tui_state.config_pending.len();
                            self.tui_state.config_pending.clear();
                            self.notifications
                                .push(super::modals::Notification::info(&format!(
                                    "Config saved ({count} changes written to roko.toml)"
                                )));
                        }
                        Err(e) => {
                            self.notifications
                                .push(super::modals::Notification::error(&format!(
                                    "Save failed: {e}"
                                )));
                        }
                    }
                }
            }
            TuiAction::MouseClick { x, y } => {
                // Use hit_test to determine zone
                let zones = super::hit_test::HitZones::compute(
                    super::layout::responsive_outer_margin(Rect::new(
                        0,
                        0,
                        self.terminal_size.0,
                        self.terminal_size.1,
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
                self.tui_state.plan_scroll_offset = (current + delta).max(0) as usize;
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
                self.tui_state.command_output_scroll = (current + delta).max(0) as usize;
            }
            FocusZone::RightPanel => {
                let current = self.tui_state.diff_scroll as i32;
                self.tui_state.diff_scroll = (current + delta).max(0) as usize;
            }
        }
    }

    fn set_focused_scroll(&mut self, offset: usize) {
        match self.tui_state.focus {
            FocusZone::PlanTree => {
                self.tui_state.plan_scroll_offset = offset;
            }
            FocusZone::TaskProgress => {
                self.tui_state.task_scroll = offset;
            }
            FocusZone::AgentOutput => {
                self.tui_state.agent_scroll = Some(offset);
            }
            FocusZone::CommandOutput => {
                self.tui_state.command_output_scroll = offset;
            }
            FocusZone::RightPanel => {
                self.tui_state.diff_scroll = offset;
            }
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        let action = match mouse.kind {
            MouseEventKind::Down(crossterm::event::MouseButton::Left) => TuiAction::MouseClick {
                x: mouse.column,
                y: mouse.row,
            },
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

    /// Manual refresh triggered by Ctrl-R.  Loads data synchronously as a
    /// one-shot fallback (the normal path uses background threads).
    fn refresh_snapshot(&mut self) {
        self.data = DashboardData::load_best_effort(&self.workdir);
        self.scaffold = DashboardScaffold::new_in(&self.workdir);
        self.tui_state.update_from_snapshot(&self.data);
        // Git info is refreshed by the background git thread; only
        // populate synchronously on first load (when fields are empty).
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

    // `update_sys_metrics` removed -- see `collect_sys_metrics_bg()` standalone
    // function below, called from the background thread.

    /// Drain all background channels (sys metrics, data refresh, git) without
    /// blocking.  Called on every tick and after every keypress so the UI
    /// reflects the latest data produced by background threads.
    fn drain_background_channels(&mut self) {
        // -- sys metrics (merge, don't replace — keep history) --
        if let Some(rx) = &self.sys_rx {
            while let Ok(snap) = rx.try_recv() {
                let sys = &mut self.tui_state.sys;

                // CPU
                sys.cpu_pct = snap.cpu_pct;
                sys.cpu_history.push(snap.cpu_pct);
                if sys.cpu_history.len() > 60 {
                    sys.cpu_history.remove(0);
                }

                // Memory
                sys.mem_used_bytes = snap.mem_used_bytes;
                sys.mem_total_bytes = snap.mem_total_bytes;
                let mem_frac = if snap.mem_total_bytes > 0 {
                    snap.mem_used_bytes as f32 / snap.mem_total_bytes as f32
                } else {
                    0.0
                };
                sys.mem_history.push(mem_frac);
                if sys.mem_history.len() > 60 {
                    sys.mem_history.remove(0);
                }

                // Network: compute rate from delta of cumulative totals
                if sys.prev_net_in > 0 && snap.net_down_bytes_sec > sys.prev_net_in {
                    sys.net_down_bytes_sec = snap.net_down_bytes_sec - sys.prev_net_in;
                }
                sys.prev_net_in = snap.net_down_bytes_sec;
                sys.net_out_bytes_total = snap.net_out_bytes_total;

                // Disk: compute rate from delta of cumulative totals
                if sys.prev_disk_read > 0 && snap.disk_read_bytes_sec > sys.prev_disk_read {
                    sys.disk_read_bytes_sec = snap.disk_read_bytes_sec - sys.prev_disk_read;
                }
                sys.prev_disk_read = snap.disk_read_bytes_sec;
                sys.disk_write_bytes_total = snap.disk_write_bytes_total;
            }
        }

        // -- dashboard data --
        if let Some(rx) = &self.data_rx {
            let mut got_data = false;
            while let Ok(new_data) = rx.try_recv() {
                self.data = new_data;
                got_data = true;
            }
            if got_data {
                self.tui_state.update_from_snapshot(&self.data);
                self.scaffold = DashboardScaffold::new_in(&self.workdir);
                self.last_refresh = Instant::now();
                self.clamp_signal_selection();
                self.clamp_gate_failure_selection();
                if self.pages().scaffold(self.current_page).is_none() {
                    self.current_page = self.scaffold.active_page();
                }
            }
        }

        // -- git data --
        if let Some(rx) = &self.git_rx {
            while let Ok(bg) = rx.try_recv() {
                self.tui_state.git_branch_tree = convert_git_branch_tree(&bg.view_data.branches);
                self.tui_state.git_commit_graph = convert_git_commit_graph(&bg.view_data.commits);
                self.tui_state.git_worktree_list =
                    convert_git_worktree_list(&bg.view_data.worktrees);
                self.tui_state.git_view_data = Some(bg.view_data);
                self.tui_state.git_summary_lines = bg.summary_lines;
                if !bg.branch.is_empty() {
                    self.tui_state.git_branch = bg.branch;
                }
                if !bg.commit_short.is_empty() {
                    self.tui_state.git_commit_short = bg.commit_short;
                }
                if !bg.age.is_empty() {
                    self.tui_state.git_age = bg.age;
                }
            }
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
// Config field value cycling
// ---------------------------------------------------------------------------

/// Cycle an enum/preset field value left (false) or right (true).
fn cycle_field_value(
    meta: &super::config_meta::ConfigFieldMeta,
    current: &str,
    forward: bool,
) -> Option<String> {
    match &meta.kind {
        super::config_meta::ConfigFieldKind::Enum(opts) => {
            let idx = opts.iter().position(|&o| o == current).unwrap_or(0);
            let new_idx = if forward {
                (idx + 1) % opts.len()
            } else {
                (idx + opts.len() - 1) % opts.len()
            };
            Some(opts[new_idx].to_string())
        }
        super::config_meta::ConfigFieldKind::Int { presets, .. } if !presets.is_empty() => {
            let cur: i64 = current.parse().unwrap_or(0);
            let idx = presets.iter().position(|&p| p == cur).unwrap_or(0);
            let new_idx = if forward {
                (idx + 1) % presets.len()
            } else {
                (idx + presets.len() - 1) % presets.len()
            };
            Some(presets[new_idx].to_string())
        }
        super::config_meta::ConfigFieldKind::Bool => {
            Some(if current == "true" { "false" } else { "true" }.to_string())
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Background sys metrics collection (runs on a dedicated thread)
// ---------------------------------------------------------------------------

/// Collect system metrics on a background thread using `sysinfo`.
fn collect_sys_metrics_bg(tx: std::sync::mpsc::Sender<super::state::SysMetrics>) {
    let mut sys = System::new();

    loop {
        sys.refresh_cpu_usage();
        sys.refresh_memory();

        let metrics = super::state::SysMetrics {
            cpu_pct: sys.global_cpu_usage(),
            mem_used_bytes: sys.used_memory(),
            mem_total_bytes: sys.total_memory(),
            ..Default::default()
        };

        if tx.send(metrics).is_err() {
            break;
        }

        std::thread::sleep(Duration::from_secs(3));
    }
}

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
        Line::from(Span::styled("Dashboard Sub-Tabs (F1)", theme.accent_bold())),
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
        Line::from("Home/End   jump to top/bottom"),
        Line::from("G          resume auto-scroll"),
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

/// Extract a numeric value from a vm_stat line like "Pages active:    123456."
#[cfg(target_os = "macos")]
fn extract_vm_stat_value(line: &str, key: &str) -> Option<u64> {
    if !line.contains(key) {
        return None;
    }
    line.split(':')
        .nth(1)?
        .trim()
        .trim_end_matches('.')
        .parse::<u64>()
        .ok()
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
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

    #[test]
    fn full_frame_render_no_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let dir = tempdir().unwrap();
        let app = App::new(dir.path());
        let backend = TestBackend::new(160, 50);
        let mut terminal = Terminal::new(backend).unwrap();
        // The real test: does a full frame render without panicking?
        terminal.draw(|frame| app.draw(frame)).unwrap();
    }

    #[test]
    fn all_tabs_render_without_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let dir = tempdir().unwrap();
        let mut app = App::new(dir.path());

        for tab in Tab::ALL {
            app.tui_state.active_tab = tab;
            let backend = TestBackend::new(160, 50);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| app.draw(frame))
                .unwrap_or_else(|e| panic!("Tab {:?} failed to render: {e}", tab));
        }
    }

    #[test]
    fn keybinding_o_switches_to_output_subtab() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let dir = tempdir().unwrap();
        let mut app = App::new(dir.path());
        assert_eq!(app.tui_state.plan_detail_tab, 0); // starts on Agents

        app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
        assert_eq!(app.tui_state.plan_detail_tab, 1); // switched to Output

        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(app.tui_state.plan_detail_tab, 2); // switched to Diff

        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        assert_eq!(app.tui_state.plan_detail_tab, 3); // switched to Errors

        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        assert_eq!(app.tui_state.plan_detail_tab, 4); // switched to Git

        app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
        assert_eq!(app.tui_state.plan_detail_tab, 5); // switched to MCP

        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert_eq!(app.tui_state.plan_detail_tab, 0); // back to Agents
    }

    #[test]
    fn keybinding_f_keys_switch_tabs() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let dir = tempdir().unwrap();
        let mut app = App::new(dir.path());
        assert_eq!(app.tui_state.active_tab, Tab::Dashboard);

        app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
        assert_eq!(app.tui_state.active_tab, Tab::Plans);

        app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE));
        assert_eq!(app.tui_state.active_tab, Tab::Agents);

        app.handle_key(KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE));
        assert_eq!(app.tui_state.active_tab, Tab::Git);

        app.handle_key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
        assert_eq!(app.tui_state.active_tab, Tab::Logs);

        app.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE));
        assert_eq!(app.tui_state.active_tab, Tab::Config);

        app.handle_key(KeyEvent::new(KeyCode::F(7), KeyModifiers::NONE));
        assert_eq!(app.tui_state.active_tab, Tab::Inspect);

        app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        assert_eq!(app.tui_state.active_tab, Tab::Dashboard);
    }

    #[test]
    fn keybinding_help_toggle() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let dir = tempdir().unwrap();
        let mut app = App::new(dir.path());
        assert!(!app.tui_state.show_help);

        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(app.tui_state.show_help);

        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(!app.tui_state.show_help);
    }

    #[test]
    fn page_scroll_moves_focused_panel_by_twenty_lines() {
        let dir = tempdir().unwrap();
        let mut app = App::new(dir.path());
        app.tui_state.focus = FocusZone::PlanTree;
        app.tui_state.plan_scroll_offset = 40;

        app.dispatch_action(TuiAction::ScrollPageUp);
        assert_eq!(app.tui_state.plan_scroll_offset, 20);

        app.dispatch_action(TuiAction::ScrollPageDown);
        assert_eq!(app.tui_state.plan_scroll_offset, 40);
    }

    #[test]
    fn focused_home_end_jump_to_bounds() {
        let dir = tempdir().unwrap();
        let mut app = App::new(dir.path());
        app.tui_state.focus = FocusZone::RightPanel;
        app.tui_state.diff_scroll = 12;

        app.dispatch_action(TuiAction::ScrollFocusedHome);
        assert_eq!(app.tui_state.diff_scroll, 0);

        app.dispatch_action(TuiAction::ScrollFocusedEnd);
        assert_eq!(app.tui_state.diff_scroll, usize::MAX);
    }
}
