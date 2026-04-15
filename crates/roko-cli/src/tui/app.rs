//! Interactive TUI application shell.
//!
//! Integrates the Mori-style tab system (F1-F7), modal dialogs, TuiState,
//! TuiAction dispatch, PostFX pipeline, and atmosphere animations.

use std::collections::HashMap;
use std::io;
use std::io::Stdout;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, KeyEvent, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, size,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::{Frame, Terminal};

use sysinfo::System;

use super::dashboard::{DashboardData, DashboardScaffold, Theme};
use super::effects_config::EffectsConfig;
use super::event::{Event, EventHandler};
use super::input::{self, ConfirmAction, FocusZone, InputMode, TuiAction};
use super::modals::{
    self, Milestone, ModalState, QueueTask, TaskPickerRow, WaveInfo, WavePlanEntry,
};
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
    /// PostFX configuration.
    fx_config: EffectsConfig,
    /// Active modal overlay.
    active_modal: Option<ModalState>,
    /// Toast notifications.
    notifications: Vec<super::modals::Notification>,
    /// Keyboard scroll acceleration state for held-key scrolling.
    scroll_accel: super::scroll::ScrollAccel,

    // -- Legacy scaffold state (kept for text-mode compatibility) --
    /// Currently selected dashboard page (legacy path).
    pub current_page: PageId,
    /// Shared dashboard data model, refreshed on tick.
    pub data: DashboardData,
    /// Static page scaffold used by the legacy renderer.
    scaffold: DashboardScaffold,
    /// Last seen dashboard data generation used to avoid redundant scaffold rebuilds.
    last_data_gen: u64,

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
    // -- Background I/O channels --
    /// Background system metrics receiver (CPU/MEM collected off main thread).
    sys_rx: Option<std::sync::mpsc::Receiver<super::state::SysMetrics>>,
    /// Background data refresh receiver (file reads off main thread).
    data_rx: Option<std::sync::mpsc::Receiver<DashboardData>>,
    /// Background git data receiver (git commands off main thread).
    git_rx: Option<std::sync::mpsc::Receiver<GitBgData>>,
    /// Live dashboard snapshot receiver from `StateHub` when connected.
    pub snapshot_rx: Option<tokio::sync::watch::Receiver<roko_core::DashboardSnapshot>>,
    /// Last error entry surfaced from the live snapshot stream.
    last_snapshot_error_marker: Option<(String, u64)>,
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
    } else if plan.status != super::state::PlanPhase::Pending {
        plan.status.label().to_string()
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

struct PanicHookRestoreGuard(Arc<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static>);

impl Drop for PanicHookRestoreGuard {
    fn drop(&mut self) {
        let hook = Arc::clone(&self.0);
        std::panic::set_hook(Box::new(move |panic_info| hook(panic_info)));
    }
}

fn tui_log_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("tui.log")
}

fn tui_log_dispatch(workdir: &Path) -> Result<tracing::Dispatch> {
    let roko_dir = workdir.join(".roko");
    std::fs::create_dir_all(&roko_dir)
        .with_context(|| format!("create TUI log directory {}", roko_dir.display()))?;

    let log_path = tui_log_path(workdir);
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open TUI log file {}", log_path.display()))?;

    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Mutex::new(log_file))
        .finish();

    Ok(tracing::Dispatch::new(subscriber))
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
        app.drain_snapshot_channel();
        if app.snapshot_rx.is_none() && app.last_refresh.elapsed() > Duration::from_secs(1) {
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
        let last_data_gen = data.generation;
        let mut tui_state = TuiState::new();
        tui_state.update_from_snapshot(&data);
        tui_state.run_started = Some(Instant::now());

        let mut app = Self {
            workdir,
            tui_state,
            fx_config: EffectsConfig::default(),
            active_modal: None,
            notifications: Vec::new(),
            scroll_accel: super::scroll::ScrollAccel::new(),
            current_page: scaffold.active_page(),
            data,
            scaffold,
            last_data_gen,
            running: true,
            last_refresh: Instant::now(),
            scroll_offset: HashMap::new(),
            signal_selection: 0,
            gate_failure_selection: 0,
            sys_rx: None,
            data_rx: None,
            git_rx: None,
            snapshot_rx: None,
            last_snapshot_error_marker: None,
            frame_counter: 0,
            last_input: Instant::now(),
            terminal_size,
        };
        app.fx_config = EffectsConfig::load_from_root(&app.workdir);
        app
    }

    /// Build a new app connected to a shared `StateHub`.
    #[must_use]
    pub fn new_connected(root: impl AsRef<Path>, state_hub: &roko_core::SharedStateHub) -> Self {
        Self::new_connected_with_page(root, None, state_hub)
    }

    /// Build a new connected app with an optional initial page selection.
    #[must_use]
    pub fn new_connected_with_page(
        root: impl AsRef<Path>,
        initial_page: Option<PageId>,
        state_hub: &roko_core::SharedStateHub,
    ) -> Self {
        let mut app = Self::new_with_page(root, initial_page);
        let snapshot_rx = state_hub.snapshot();
        if snapshot_has_content(&snapshot_rx.borrow()) {
            let snapshot = snapshot_rx.borrow();
            apply_dashboard_snapshot(
                &mut app.tui_state,
                &mut app.notifications,
                &mut app.last_snapshot_error_marker,
                &snapshot,
            );
        }
        app.snapshot_rx = Some(snapshot_rx);
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
        let log_path = tui_log_path(&self.workdir);
        let log_dispatch =
            tui_log_dispatch(&self.workdir).context("initialize TUI file logging")?;
        let _log_guard = tracing::dispatcher::set_default(&log_dispatch);
        tracing::info!(path = %log_path.display(), "TUI file logging enabled");

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
        let log_dispatch = tracing::dispatcher::get_default(|dispatch| dispatch.clone());

        // ---------------------------------------------------------------
        // Spawn background sys metrics collector thread
        // ---------------------------------------------------------------
        let (sys_tx, sys_rx) = std::sync::mpsc::channel::<super::state::SysMetrics>();
        let sys_log_dispatch = log_dispatch.clone();
        std::thread::Builder::new()
            .name("tui-sys-metrics".into())
            .spawn(move || {
                let _log_guard = tracing::dispatcher::set_default(&sys_log_dispatch);
                collect_sys_metrics_bg(sys_tx);
            })
            .inspect_err(|err| {
                tracing::warn!(
                    error = %err,
                    thread = "tui-sys-metrics",
                    "failed to spawn background thread"
                );
            })
            .ok(); // graceful: TUI works without background thread
        self.sys_rx = Some(sys_rx);

        // ---------------------------------------------------------------
        // Spawn background data refresh thread
        // ---------------------------------------------------------------
        let (data_tx, data_rx) = std::sync::mpsc::channel::<DashboardData>();
        let data_workdir = self.workdir.clone();
        let data_log_dispatch = log_dispatch.clone();
        std::thread::Builder::new()
            .name("tui-data-refresh".into())
            .spawn(move || {
                let _log_guard = tracing::dispatcher::set_default(&data_log_dispatch);
                loop {
                    let data = DashboardData::load_best_effort(&data_workdir);
                    if data_tx.send(data).is_err() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            })
            .inspect_err(|err| {
                tracing::warn!(
                    error = %err,
                    thread = "tui-data-refresh",
                    "failed to spawn background thread"
                );
            })
            .ok();
        self.data_rx = Some(data_rx);

        // ---------------------------------------------------------------
        // Spawn background git data collector thread
        // ---------------------------------------------------------------
        let (git_tx, git_rx) = std::sync::mpsc::channel::<GitBgData>();
        let git_log_dispatch = log_dispatch;
        std::thread::Builder::new()
            .name("tui-git-refresh".into())
            .spawn(move || {
                let _log_guard = tracing::dispatcher::set_default(&git_log_dispatch);
                loop {
                    let view_data = super::views::git_view::collect_git_data();
                    let branch = view_data.current_branch.clone();
                    let commit_short = view_data
                        .commits
                        .first()
                        .map(|c| c.hash_short.clone())
                        .unwrap_or_default();
                    let age = view_data
                        .commits
                        .first()
                        .map(|commit| commit.age.clone())
                        .unwrap_or_default();
                    let summary_lines =
                        super::views::dashboard_view::collect_git_summary(&view_data, &age);
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
            .inspect_err(|err| {
                tracing::warn!(
                    error = %err,
                    thread = "tui-git-refresh",
                    "failed to spawn background thread"
                );
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
            self.drain_snapshot_channel();
            match events.next().context("poll TUI event")? {
                Event::Key(key) => {
                    self.last_input = Instant::now();
                    self.handle_key(key);
                    self.drain_snapshot_channel();
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
                    self.tui_state.atmosphere.tick();
                    self.drain_snapshot_channel();
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

        let view_state = self.current_view_state();
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
        ) && content_area.height > 0
        {
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
            let input = Paragraph::new(Line::from(vec![
                Span::styled(label, theme.accent_bold()),
                Span::styled(buf, theme.text()),
            ]));
            frame.render_widget(input, input_area);

            let cursor_x = input_area
                .x
                .saturating_add((label.chars().count() + buf.chars().count()) as u16)
                .min(input_area.right().saturating_sub(1));
            frame.set_cursor_position((cursor_x, input_area.y));
        }

        // Dim overlay before modals
        if self.active_modal.is_some() {
            let buf = frame.buffer_mut();
            super::postfx::dim_overlay(content_area, buf, 0.45);
        }

        // Modal rendering
        modals::render_modals(
            frame,
            full_area,
            self.active_modal.as_ref(),
            &self.tui_state,
            &self.data,
            &self.notifications,
            &theme,
            self.fx_config.screen_postfx,
        );

        // PostFX pipeline
        if self.fx_config.screen_postfx {
            let buf = frame.buffer_mut();
            super::postfx_pipeline::apply_pipeline(
                self.tui_state.active_tab as usize,
                content_area,
                buf,
                self.tui_state.atmosphere.elapsed,
                self.tui_state.atmosphere.frame_count,
                &self.fx_config,
            );
        }
    }

    // -----------------------------------------------------------------------
    // Key handling
    // -----------------------------------------------------------------------

    fn handle_key(&mut self, key: KeyEvent) {
        // Route through the full TuiAction dispatch
        let action = input::handle_key(
            key,
            self.tui_state.input_mode,
            self.tui_state.active_tab,
            self.tui_state.focus,
            &input::ModalVisibility::from_active_modal(self.active_modal.as_ref()),
        );

        self.dispatch_action(action);
    }

    fn dispatch_action(&mut self, action: TuiAction) {
        match action {
            TuiAction::Quit => {
                if self.has_modal() {
                    self.dismiss_all_modals();
                } else {
                    self.tui_state.input_mode = InputMode::Confirm;
                    self.active_modal = Some(ModalState::Quit);
                }
            }
            TuiAction::QuitConfirmed => {
                self.running = false;
            }
            TuiAction::SwitchTab(tab) => {
                self.tui_state.active_tab = tab;
                self.tui_state.focus = match tab {
                    Tab::Dashboard | Tab::Plans => FocusZone::PlanTree,
                    Tab::Agents => FocusZone::AgentOutput,
                    Tab::Git | Tab::Logs | Tab::Config | Tab::Inspect => FocusZone::RightPanel,
                };
                // Sync legacy page
                if let Some(page_id) = tab_to_page(tab) {
                    self.current_page = page_id;
                    let _ = self.scaffold.set_active_page(page_id);
                }
            }
            TuiAction::FocusNext => {
                self.tui_state.focus = self.tui_state.focus.next(self.tui_state.active_tab);
            }
            TuiAction::FocusPrev => {
                self.tui_state.focus = self.tui_state.focus.prev(self.tui_state.active_tab);
            }
            TuiAction::SelectPlanUp => {
                if matches!(self.tui_state.active_tab, Tab::Agents)
                    && !matches!(
                        self.tui_state.focus,
                        FocusZone::AgentOutput | FocusZone::RightPanel
                    )
                {
                    self.tui_state.selected_agent = self.tui_state.selected_agent.saturating_sub(1);
                } else if self.tui_state.selected_plan_idx > 0 {
                    self.tui_state.selected_plan_idx -= 1;
                }
            }
            TuiAction::SelectPlanDown => {
                if matches!(self.tui_state.active_tab, Tab::Agents)
                    && !matches!(
                        self.tui_state.focus,
                        FocusZone::AgentOutput | FocusZone::RightPanel
                    )
                {
                    let max = self.tui_state.agents.len().saturating_sub(1);
                    if self.tui_state.selected_agent < max {
                        self.tui_state.selected_agent += 1;
                    }
                } else {
                    let max = self.tui_state.plans.len().saturating_sub(1);
                    if self.tui_state.selected_plan_idx < max {
                        self.tui_state.selected_plan_idx += 1;
                    }
                }
            }
            TuiAction::TaskPickerUp => {
                if let Some(ModalState::TaskPicker { selected_index, .. }) =
                    self.active_modal.as_mut()
                {
                    *selected_index = selected_index.saturating_sub(1);
                }
            }
            TuiAction::TaskPickerDown => {
                if let Some(ModalState::TaskPicker {
                    selected_index,
                    tasks,
                    ..
                }) = self.active_modal.as_mut()
                {
                    let max = tasks.len().saturating_sub(1);
                    *selected_index = selected_index.saturating_add(1).min(max);
                }
            }
            TuiAction::ScrollFocusedUp => {
                let delta = i32::from(self.scroll_accel.push(-1));
                self.scroll_focused(delta);
            }
            TuiAction::ScrollFocusedDown => {
                let delta = i32::from(self.scroll_accel.push(1));
                self.scroll_focused(delta);
            }
            TuiAction::ScrollPageUp => self.scroll_focused(-PAGE_SCROLL_LINES),
            TuiAction::ScrollPageDown => self.scroll_focused(PAGE_SCROLL_LINES),
            TuiAction::ScrollFocusedHome => match (self.tui_state.active_tab, self.tui_state.focus)
            {
                (Tab::Dashboard | Tab::Agents, FocusZone::AgentOutput) => {
                    self.tui_state.agent_scroll = Some(usize::MAX);
                }
                (Tab::Logs, _) => {
                    self.tui_state.log_auto_tail = false;
                    self.tui_state.log_scroll = usize::MAX;
                }
                _ => self.set_focused_scroll(0),
            },
            TuiAction::ScrollFocusedEnd => {
                match (self.tui_state.active_tab, self.tui_state.focus) {
                    (Tab::Dashboard | Tab::Agents, FocusZone::AgentOutput) => {
                        self.tui_state.agent_scroll = None;
                    }
                    (Tab::Logs, _) => {
                        self.tui_state.log_auto_tail = true;
                        self.tui_state.log_scroll = 0;
                    }
                    _ => self.set_focused_scroll(usize::MAX),
                }
            }
            TuiAction::ScrollLogUp => {
                if self.tui_state.log_auto_tail {
                    self.tui_state.log_auto_tail = false;
                    self.tui_state.log_scroll = 1;
                } else {
                    self.tui_state.log_scroll = self.tui_state.log_scroll.saturating_add(1);
                }
            }
            TuiAction::ScrollLogDown => {
                if !self.tui_state.log_auto_tail {
                    if self.tui_state.log_scroll > 0 {
                        self.tui_state.log_scroll = self.tui_state.log_scroll.saturating_sub(1);
                    }
                    if self.tui_state.log_scroll == 0 {
                        self.tui_state.log_auto_tail = true;
                    }
                }
            }
            TuiAction::ScrollLogEnd => {
                self.tui_state.log_auto_tail = true;
                self.tui_state.log_scroll = 0;
            }
            TuiAction::ToggleLogFilter(level) => {
                self.tui_state.toggle_log_filter_level(level);
            }
            TuiAction::ShowAllLogFilters => {
                self.tui_state.show_all_log_filter_levels();
            }
            TuiAction::ScrollAgentUp => {
                let current = self.current_agent_scroll_offset();
                let delta = self.scroll_accel.push(-1);
                self.tui_state.agent_scroll = Some(Self::apply_signed_scroll(current, delta));
            }
            TuiAction::ScrollAgentDown => {
                let current = self.current_agent_scroll_offset();
                let delta = self.scroll_accel.push(1);
                self.tui_state.agent_scroll = Some(Self::apply_signed_scroll(current, delta));
            }
            TuiAction::ScrollAgentEnd => {
                self.tui_state.agent_scroll = None; // Resume auto-tail
            }
            TuiAction::ScrollDiffUp => {
                let delta = self.scroll_accel.push(-1);
                self.tui_state.diff_scroll =
                    Self::apply_signed_scroll(self.tui_state.diff_scroll, delta);
            }
            TuiAction::ScrollDiffDown => {
                let delta = self.scroll_accel.push(1);
                self.tui_state.diff_scroll =
                    Self::apply_signed_scroll(self.tui_state.diff_scroll, delta);
            }
            TuiAction::ScrollDetailUp => {
                if matches!(self.active_modal, Some(ModalState::PlanDetail { .. })) {
                    self.tui_state.plan_detail_scroll =
                        self.tui_state.plan_detail_scroll.saturating_sub(1);
                } else if let Some(ModalState::TaskDetail { scroll_offset, .. }) =
                    self.active_modal.as_mut()
                {
                    *scroll_offset = scroll_offset.saturating_sub(1);
                } else {
                    self.tui_state.plan_detail_scroll =
                        self.tui_state.plan_detail_scroll.saturating_sub(1);
                }
            }
            TuiAction::ScrollDetailDown => {
                if matches!(self.active_modal, Some(ModalState::PlanDetail { .. })) {
                    self.tui_state.plan_detail_scroll =
                        self.tui_state.plan_detail_scroll.saturating_add(1);
                } else if let Some(ModalState::TaskDetail { scroll_offset, .. }) =
                    self.active_modal.as_mut()
                {
                    *scroll_offset = scroll_offset.saturating_add(1);
                } else {
                    self.tui_state.plan_detail_scroll =
                        self.tui_state.plan_detail_scroll.saturating_add(1);
                }
            }
            TuiAction::ShowHelp => {
                self.active_modal = if matches!(self.active_modal, Some(ModalState::Help)) {
                    None
                } else {
                    Some(ModalState::Help)
                };
            }
            TuiAction::ToggleScreenPostFx => {
                self.fx_config.screen_postfx = !self.fx_config.screen_postfx;
                let state = if self.fx_config.screen_postfx {
                    "enabled"
                } else {
                    "disabled"
                };
                self.notifications
                    .push(super::modals::Notification::info(&format!(
                        "Screen postfx {state}"
                    )));
            }
            TuiAction::ShowPlanDetail => {
                self.active_modal = self
                    .tui_state
                    .plans
                    .get(self.tui_state.selected_plan_idx)
                    .map(|plan| plan.id.clone())
                    .and_then(|plan_id| {
                        if matches!(
                            self.active_modal.as_ref(),
                            Some(ModalState::PlanDetail {
                                plan_id: active_plan_id
                            }) if active_plan_id == &plan_id
                        ) {
                            None
                        } else {
                            Some(ModalState::PlanDetail { plan_id })
                        }
                    });
            }
            TuiAction::ClosePlanDetail => {
                if matches!(self.active_modal, Some(ModalState::PlanDetail { .. })) {
                    self.active_modal = None;
                }
            }
            TuiAction::ShowTaskDetail => {
                let task_count = self.tui_state.current_task_checklist.len();
                if task_count > 0 {
                    let task_idx = self.tui_state.task_scroll.min(task_count.saturating_sub(1));
                    self.active_modal = Some(ModalState::TaskDetail {
                        task_idx,
                        scroll_offset: 0,
                    });
                }
            }
            TuiAction::CloseTaskDetail => {
                if matches!(self.active_modal, Some(ModalState::TaskDetail { .. })) {
                    self.active_modal = None;
                }
            }
            TuiAction::ShowWaveOverview => {
                if matches!(self.active_modal, Some(ModalState::WaveOverview { .. })) {
                    self.active_modal = None;
                } else {
                    self.active_modal = Some(ModalState::WaveOverview {
                        waves: execution_waves_for_modal(&self.tui_state),
                        scroll_offset: 0,
                    });
                }
            }
            TuiAction::ShowQueueOverview => {
                if matches!(self.active_modal, Some(ModalState::QueueOverview { .. })) {
                    self.active_modal = None;
                } else {
                    let milestones = queue_overview_milestones(&self.tui_state);
                    self.active_modal = Some(ModalState::QueueOverview {
                        selected_index: self
                            .tui_state
                            .current_wave()
                            .min(milestones.len().saturating_sub(1)),
                        scroll_offset: self.tui_state.current_wave() as u16,
                        milestones,
                    });
                }
            }
            TuiAction::OpenTaskPicker => {
                let tasks = task_picker_rows(&self.tui_state);
                let selected_index = self
                    .tui_state
                    .task_scroll
                    .min(tasks.len().saturating_sub(1));
                self.active_modal = Some(ModalState::TaskPicker {
                    tasks,
                    selected_index,
                    scroll_offset: selected_index as u16,
                });
            }
            TuiAction::CloseTaskPicker => {
                if matches!(self.active_modal, Some(ModalState::TaskPicker { .. })) {
                    self.active_modal = None;
                }
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
            TuiAction::TogglePause => {
                self.tui_state.is_paused = !self.tui_state.is_paused;
            }
            TuiAction::SwitchAgentTab(idx) => {
                if idx == usize::MAX {
                    let agent_count = 7;
                    self.tui_state.selected_agent_tab =
                        (self.tui_state.selected_agent_tab + 1) % agent_count;
                } else {
                    let max_idx = self.tui_state.agents.len().saturating_sub(1).max(6);
                    self.tui_state.selected_agent_tab = idx.min(max_idx);
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
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&signal_path)
                        .inspect_err(|err| {
                            tracing::warn!(
                                error = %err,
                                path = %signal_path.display(),
                                "failed to open signal file for inject"
                            );
                        })
                        .ok()
                        .and_then(|mut f| {
                            use std::io::Write;
                            writeln!(f, "{}", entry)
                                .inspect_err(|err| {
                                    tracing::warn!(
                                        error = %err,
                                        path = %signal_path.display(),
                                        "failed to append inject signal"
                                    );
                                })
                                .ok()
                        });
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
                self.tui_state.filter = self.tui_state.filter_text.clone();
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
                self.open_confirm_modal(self.resolve_confirm_action(action));
            }
            TuiAction::ConfirmYes => {
                self.tui_state.input_mode = InputMode::Normal;
                if matches!(self.active_modal, Some(ModalState::Quit)) {
                    self.dismiss_all_modals();
                    self.dispatch_action(TuiAction::QuitConfirmed);
                    return;
                }
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
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&signal_path)
                        .inspect_err(|err| {
                            tracing::warn!(
                                error = %err,
                                path = %signal_path.display(),
                                "failed to open signal file for confirm"
                            );
                        })
                        .ok()
                        .and_then(|mut f| {
                            use std::io::Write;
                            writeln!(f, "{}", entry)
                                .inspect_err(|err| {
                                    tracing::warn!(
                                        error = %err,
                                        path = %signal_path.display(),
                                        "failed to append confirm signal"
                                    );
                                })
                                .ok()
                        });
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
                self.dismiss_all_modals();
            }
            TuiAction::DismissNotification => {
                if !self.notifications.is_empty() {
                    self.notifications.remove(0);
                }
            }
            TuiAction::ToggleAgentPaneGroup => {
                self.tui_state.agent_pane_group = (self.tui_state.agent_pane_group + 1) % 2;
            }
            TuiAction::DrillIn => match self.tui_state.active_tab {
                Tab::Dashboard | Tab::Plans => {
                    if let Some(plan) = self
                        .tui_state
                        .plans
                        .get_mut(self.tui_state.selected_plan_idx)
                    {
                        plan.expanded = true;
                    }
                }
                Tab::Git => {
                    let max = self.git_branch_count().saturating_sub(1);
                    self.tui_state.git_branch_cursor =
                        (self.tui_state.git_branch_cursor + 1).min(max);
                }
                Tab::Inspect => {}
                Tab::Agents | Tab::Logs | Tab::Config => {}
            },
            TuiAction::DrillOut => match self.tui_state.active_tab {
                Tab::Dashboard | Tab::Plans => {
                    if let Some(plan) = self
                        .tui_state
                        .plans
                        .get_mut(self.tui_state.selected_plan_idx)
                    {
                        plan.expanded = false;
                    }
                }
                Tab::Git => {
                    self.tui_state.git_branch_cursor =
                        self.tui_state.git_branch_cursor.saturating_sub(1);
                }
                Tab::Inspect => {}
                Tab::Agents | Tab::Logs | Tab::Config => {}
            },
            TuiAction::WaveNext => {
                let max = self.tui_state.execution_waves.len().max(1);
                self.tui_state.selected_wave_idx = (self.tui_state.selected_wave_idx + 1) % max;
            }
            TuiAction::WavePrev => {
                let max = self.tui_state.execution_waves.len().max(1);
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
                            self.save_config_changes();
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
                self.save_config_changes();
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

    fn open_confirm_modal(&mut self, action: ConfirmAction) {
        self.tui_state.input_mode = InputMode::Confirm;
        self.tui_state.pending_confirm = Some(action.clone());
        let modal_action = modals::ConfirmAction::Custom {
            message: action.to_string(),
        };
        self.active_modal = Some(ModalState::Confirm {
            action: modal_action,
        });
    }

    fn resolve_confirm_action(&self, action: ConfirmAction) -> ConfirmAction {
        match action {
            ConfirmAction::DiagnosePlan(plan_id) if plan_id.is_empty() => {
                ConfirmAction::DiagnosePlan(self.selected_plan_id().unwrap_or_default())
            }
            ConfirmAction::MergePlan { plan_id, branch }
                if plan_id.is_empty() || branch.is_empty() =>
            {
                ConfirmAction::MergePlan {
                    plan_id: if plan_id.is_empty() {
                        self.selected_plan_id().unwrap_or_default()
                    } else {
                        plan_id
                    },
                    branch: if branch.is_empty() {
                        self.current_git_branch()
                    } else {
                        branch
                    },
                }
            }
            ConfirmAction::MergeAllDone { branches } if branches.is_empty() => {
                ConfirmAction::MergeAllDone {
                    branches: self.completed_plan_branches(),
                }
            }
            other => other,
        }
    }

    fn selected_plan_id(&self) -> Option<String> {
        self.tui_state
            .plans
            .get(self.tui_state.selected_plan_idx)
            .map(|plan| plan.id.clone())
    }

    fn current_git_branch(&self) -> String {
        if !self.tui_state.git_branch.is_empty() {
            return self.tui_state.git_branch.clone();
        }

        self.tui_state
            .git_view_data
            .as_ref()
            .map(|git| git.current_branch.clone())
            .filter(|branch| !branch.is_empty())
            .unwrap_or_default()
    }

    fn completed_plan_branches(&self) -> Vec<String> {
        self.tui_state
            .plans
            .iter()
            .filter(|plan| !plan.active && !plan.status.is_failed())
            .map(|plan| plan.id.clone())
            .collect()
    }

    fn scroll_focused(&mut self, delta: i32) {
        match (self.tui_state.active_tab, self.tui_state.focus) {
            (Tab::Logs, _) => {
                if self.tui_state.log_auto_tail {
                    self.tui_state.log_auto_tail = false;
                    self.tui_state.log_scroll = delta.unsigned_abs() as usize;
                } else {
                    let current = self.tui_state.log_scroll as i32;
                    let next = (current + delta).max(0) as usize;
                    self.tui_state.log_scroll = next;
                    if next == 0 {
                        self.tui_state.log_auto_tail = true;
                    }
                }
            }
            (Tab::Agents, FocusZone::PlanTree) => {
                let max = self.tui_state.agents.len().saturating_sub(1);
                let next = (self.tui_state.selected_agent as i32 + delta).clamp(0, max as i32);
                self.tui_state.selected_agent = next as usize;
            }
            (Tab::Agents, FocusZone::AgentOutput) => {
                let current = self.tui_state.agent_scroll.unwrap_or(0);
                if delta < 0 {
                    self.tui_state.agent_scroll =
                        Some(current.saturating_add(delta.unsigned_abs() as usize));
                } else if current == 0 {
                    self.tui_state.agent_scroll = None;
                } else {
                    self.tui_state.agent_scroll = Some(current.saturating_sub(delta as usize));
                }
            }
            (_, FocusZone::PlanTree) => {
                let current = self.tui_state.plan_scroll_offset as i32;
                self.tui_state.plan_scroll_offset = (current + delta).max(0) as usize;
            }
            (_, FocusZone::TaskProgress) => {
                let current = self.tui_state.task_scroll as i32;
                self.tui_state.task_scroll = (current + delta).max(0) as usize;
            }
            (_, FocusZone::AgentOutput) => {
                let current = self.current_agent_scroll_offset() as i32;
                self.tui_state.agent_scroll = Some((current + delta).max(0) as usize);
            }
            (_, FocusZone::CommandOutput) => {
                let current = self.tui_state.command_output_scroll as i32;
                self.tui_state.command_output_scroll = (current + delta).max(0) as usize;
            }
            (_, FocusZone::RightPanel) => {
                let current = self.tui_state.diff_scroll as i32;
                self.tui_state.diff_scroll = (current + delta).max(0) as usize;
            }
        }
    }

    fn set_focused_scroll(&mut self, offset: usize) {
        match (self.tui_state.active_tab, self.tui_state.focus) {
            (Tab::Agents, FocusZone::PlanTree) => {
                let max = self.tui_state.agents.len().saturating_sub(1);
                self.tui_state.selected_agent = if offset == usize::MAX {
                    max
                } else {
                    offset.min(max)
                };
            }
            (Tab::Agents, FocusZone::AgentOutput) => {
                self.tui_state.agent_scroll = Some(offset);
            }
            (_, FocusZone::PlanTree) => {
                self.tui_state.plan_scroll_offset = offset;
            }
            (_, FocusZone::TaskProgress) => {
                self.tui_state.task_scroll = offset;
            }
            (_, FocusZone::AgentOutput) => {
                self.tui_state.agent_scroll = Some(offset);
            }
            (_, FocusZone::CommandOutput) => {
                self.tui_state.command_output_scroll = offset;
            }
            (_, FocusZone::RightPanel) => {
                self.tui_state.diff_scroll = offset;
            }
        }
    }


    fn apply_signed_scroll(current: usize, delta: i16) -> usize {
        if delta < 0 {
            current.saturating_sub(delta.saturating_abs() as usize)
        } else {
            current.saturating_add(delta as usize)
        }
    }

    fn current_agent_scroll_offset(&self) -> usize {
        self.tui_state.agent_scroll.unwrap_or_else(|| {
            self.current_agent_output_line_count()
                .saturating_sub(self.current_agent_output_viewport_height())
                .min(u16::MAX as usize)
        })
    }

    fn current_agent_output_line_count(&self) -> usize {
        match self.tui_state.active_tab {
            Tab::Agents => views::agents_view::collect_agent_output_lines(
                &self.data,
                &self.tui_state,
                self.current_view_state().selected,
            )
            .len(),
            Tab::Dashboard if self.tui_state.plan_detail_tab == 1 => {
                let collected: Vec<String> = self
                    .data
                    .current_plan_execution
                    .as_ref()
                    .map(|exec| exec.agent_output_tail.clone())
                    .unwrap_or_default();

                if !collected.is_empty() {
                    return collected.len();
                }

                if let Some(agent) = self.tui_state.agents.get(
                    self.tui_state
                        .selected_agent
                        .min(self.tui_state.agents.len().saturating_sub(1)),
                ) {
                    if !agent.output_lines.is_empty() {
                        return agent.output_lines.len();
                    }
                }

                self.data
                    .task_outputs
                    .values()
                    .max_by_key(|lines| lines.len())
                    .map_or(0, Vec::len)
            }
            _ => 0,
        }
    }

    fn current_agent_output_viewport_height(&self) -> usize {
        let Ok((width, height)) = crossterm::terminal::size() else {
            return 0;
        };

        let full_area = Rect::new(0, 0, width, height);
        let content_area = super::layout::responsive_outer_margin(full_area);
        let has_waves = !self.tui_state.execution_waves.is_empty();
        let wave_row_height = if has_waves { 1 } else { 0 };
        let main_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(wave_row_height),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(content_area);
        let content_area = main_layout[2];

        match self.tui_state.active_tab {
            Tab::Agents => {
                let panels =
                    Layout::horizontal([Constraint::Percentage(32), Constraint::Percentage(68)])
                        .split(content_area);
                let sections =
                    Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(panels[1]);
                sections[1].height.saturating_sub(2) as usize
            }
            Tab::Dashboard if self.tui_state.plan_detail_tab == 1 => {
                let outer = Layout::vertical([Constraint::Min(0), Constraint::Length(6)])
                    .split(content_area);
                let main =
                    Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)])
                        .split(outer[0]);
                let sections =
                    Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(main[1]);
                sections[1].height.saturating_sub(2) as usize
            }
            _ => 0,
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

    fn expire_notifications(&mut self) {
        self.notifications
            .retain(|notification| notification.created.elapsed() < Duration::from_secs(5));
    }

    fn has_modal(&self) -> bool {
        self.active_modal.is_some()
    }

    fn dismiss_all_modals(&mut self) {
        self.active_modal = None;
        self.tui_state.pending_confirm = None;
        if self.tui_state.input_mode == InputMode::Confirm {
            self.tui_state.input_mode = InputMode::Normal;
        }
    }

    // -----------------------------------------------------------------------
    // Legacy compatibility
    // -----------------------------------------------------------------------


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
        self.last_data_gen = self.data.generation;
        self.tui_state.update_from_snapshot(&self.data);
        self.fx_config = EffectsConfig::load_from_root(&self.workdir);
        self.last_refresh = Instant::now();
        self.clamp_signal_selection();
        self.clamp_gate_failure_selection();
        if self.pages().scaffold(self.current_page).is_none() {
            self.current_page = self.scaffold.active_page();
        }
    }

    fn save_config_changes(&mut self) {
        if self.tui_state.config_pending.is_empty() {
            self.notifications.push(super::modals::Notification::info(
                "No pending changes to save",
            ));
            return;
        }

        match super::config_meta::save_pending_edits(&self.workdir, &self.tui_state.config_pending)
        {
            Ok(()) => {
                self.tui_state.config_pending.clear();
                self.data = DashboardData::load_best_effort(&self.workdir);
                self.tui_state.update_from_snapshot(&self.data);
                self.fx_config = EffectsConfig::load_from_root(&self.workdir);
                self.notifications
                    .push(super::modals::Notification::info("Config saved and reloaded"));
            }
            Err(error) => {
                self.notifications
                    .push(super::modals::Notification::error(&format!(
                        "Save failed: {error}"
                    )));
            }
        }
    }

    fn current_view_state(&self) -> ViewState {
        match self.tui_state.active_tab {
            Tab::Dashboard => ViewState {
                scroll: self.tui_state.agent_scroll.unwrap_or(0) as u16,
                selected: self.tui_state.selected_plan_idx,
                sub_tab: self.tui_state.plan_detail_tab,
                secondary_selected: 0,
                auto_tail: self.tui_state.agent_scroll.is_none(),
            },
            Tab::Plans => ViewState {
                scroll: self.tui_state.plan_scroll_offset as u16,
                selected: self.tui_state.selected_plan_idx,
                sub_tab: self.tui_state.plan_detail_tab,
                secondary_selected: 0,
                auto_tail: false,
            },
            Tab::Agents => ViewState {
                scroll: self.tui_state.agent_scroll.unwrap_or(0) as u16,
                selected: self.tui_state.selected_agent,
                sub_tab: self.tui_state.selected_agent_tab,
                secondary_selected: 0,
                auto_tail: self.tui_state.agent_scroll.is_none(),
            },
            Tab::Git => ViewState {
                scroll: self.tui_state.plan_scroll_offset as u16,
                selected: self.tui_state.git_branch_cursor,
                sub_tab: 0,
                secondary_selected: 0,
                auto_tail: false,
            },
            Tab::Logs => ViewState {
                scroll: self.tui_state.log_scroll.min(u16::MAX as usize) as u16,
                selected: 0,
                sub_tab: 0,
                secondary_selected: 0,
                auto_tail: self.tui_state.log_auto_tail,
            },
            Tab::Config => ViewState {
                scroll: self.tui_state.config_scroll_offset.min(u16::MAX as usize) as u16,
                selected: self.tui_state.config_cursor,
                sub_tab: 0,
                secondary_selected: 0,
                auto_tail: false,
            },
            Tab::Inspect => ViewState {
                scroll: 0,
                selected: 0,
                sub_tab: 0,
                secondary_selected: 0,
                auto_tail: false,
            },
        }
    }

    fn git_branch_count(&self) -> usize {
        self.tui_state
            .git_view_data
            .as_ref()
            .map_or(self.tui_state.git_branch_tree.len(), |data| {
                data.branches.len()
            })
    }


    // `update_sys_metrics` removed -- see `collect_sys_metrics_bg()` standalone
    // function below, called from the background thread.

    /// Drain all background channels (sys metrics, data refresh, git) without
    /// blocking.  Called on every tick and after every keypress so the UI
    /// reflects the latest data produced by background threads.
    fn drain_background_channels(&mut self) {
        const MAX_MESSAGES_PER_DRAIN: usize = 20;

        self.drain_snapshot_channel();

        // -- sys metrics (merge, don't replace — keep history) --
        if let Some(rx) = &self.sys_rx {
            let mut count = 0;
            while let Ok(snap) = rx.try_recv() {
                // CPU
                let cpu_pct = self.tui_state.update_cpu_pct(snap.cpu_pct);
                let sys = &mut self.tui_state.sys;
                sys.cpu_history.push(cpu_pct);
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

                count += 1;
                if count >= MAX_MESSAGES_PER_DRAIN {
                    break;
                }
            }
        }

        // -- dashboard data --
        if let Some(rx) = &self.data_rx {
            let mut got_data = false;
            let mut rebuild_scaffold = false;
            let mut count = 0;
            while let Ok(new_data) = rx.try_recv() {
                if new_data.generation != self.last_data_gen {
                    rebuild_scaffold = true;
                }
                self.last_data_gen = new_data.generation;
                self.data = new_data;
                got_data = true;

                count += 1;
                if count >= MAX_MESSAGES_PER_DRAIN {
                    break;
                }
            }
            if got_data {
                if self.snapshot_rx.is_none() {
                    self.tui_state.update_from_snapshot(&self.data);
                }
                if rebuild_scaffold {
                    self.scaffold = DashboardScaffold::new_in(&self.workdir);
                }
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
            let mut count = 0;
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

                count += 1;
                if count >= MAX_MESSAGES_PER_DRAIN {
                    break;
                }
            }
        }
    }

    fn drain_snapshot_channel(&mut self) {
        let (snapshot_rx, tui_state, notifications, last_marker) = (
            &mut self.snapshot_rx,
            &mut self.tui_state,
            &mut self.notifications,
            &mut self.last_snapshot_error_marker,
        );

        let Some(rx) = snapshot_rx.as_mut() else {
            return;
        };

        if !rx.has_changed().unwrap_or(false) {
            return;
        }

        let snapshot = rx.borrow_and_update();
        apply_dashboard_snapshot(tui_state, notifications, last_marker, &snapshot);
    }

    fn pages(&self) -> PageRegistry {
        PageRegistry::from_dashboard(&self.scaffold)
    }

    #[allow(dead_code)]
    fn scroll_for(&self, page: PageId) -> u16 {
        self.scroll_offset.get(&page).copied().unwrap_or(0)
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

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

fn apply_dashboard_snapshot(
    tui_state: &mut TuiState,
    notifications: &mut Vec<super::modals::Notification>,
    last_snapshot_error_marker: &mut Option<(String, u64)>,
    snapshot: &roko_core::DashboardSnapshot,
) {
    tui_state.update_from_dashboard_snapshot(snapshot);

    if !snapshot.errors.is_empty() {
        let start_idx = last_snapshot_error_marker
            .as_ref()
            .and_then(|marker| {
                snapshot
                    .errors
                    .iter()
                    .position(|error| error.message == marker.0 && error.ts_millis == marker.1)
                    .map(|idx| idx + 1)
            })
            .unwrap_or(0);

        for error in snapshot.errors.iter().skip(start_idx) {
            notifications.push(super::modals::Notification::error(error.message.clone()));
        }

        if let Some(last_error) = snapshot.errors.last() {
            *last_snapshot_error_marker = Some((last_error.message.clone(), last_error.ts_millis));
        }
    }
}

fn snapshot_has_content(snapshot: &roko_core::DashboardSnapshot) -> bool {
    !snapshot.plans.is_empty()
        || !snapshot.tasks.is_empty()
        || !snapshot.agents.is_empty()
        || !snapshot.gates.is_empty()
        || !snapshot.errors.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::config::RokoConfig;
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
    fn app_new_connected_installs_snapshot_receiver() {
        let dir = tempdir().unwrap();
        let hub = roko_core::shared_state_hub();
        let app = App::new_connected(dir.path(), &hub);
        assert!(app.snapshot_rx.is_some());
    }

    #[test]
    fn dashboard_snapshot_updates_preserve_navigation_state() {
        let dir = tempdir().unwrap();
        let mut app = App::new(dir.path());
        app.tui_state.plans = vec![
            super::super::state::PlanEntry {
                id: "plan-a".to_string(),
                expanded: true,
                ..Default::default()
            },
            super::super::state::PlanEntry {
                id: "plan-b".to_string(),
                ..Default::default()
            },
        ];
        app.tui_state.agents = vec![
            super::super::state::AgentRow {
                id: "agent-a".to_string(),
                ..Default::default()
            },
            super::super::state::AgentRow {
                id: "agent-b".to_string(),
                ..Default::default()
            },
        ];
        app.tui_state.selected_plan_idx = 0;
        app.tui_state.current_plan_idx = 1;
        app.tui_state.selected_agent = 1;
        app.tui_state.active_tab = Tab::Agents;
        app.tui_state.plan_scroll_offset = 17;
        app.tui_state.agent_scroll = Some(9);

        let snapshot = roko_core::DashboardSnapshot {
            plans: [
                (
                    "plan-b".to_string(),
                    roko_core::dashboard_snapshot::PlanState {
                        plan_id: "plan-b".to_string(),
                        phase: "done".to_string(),
                        tasks_total: 2,
                        tasks_done: 2,
                        tasks_failed: 0,
                        active: false,
                    },
                ),
                (
                    "plan-c".to_string(),
                    roko_core::dashboard_snapshot::PlanState {
                        plan_id: "plan-c".to_string(),
                        phase: "active".to_string(),
                        tasks_total: 1,
                        tasks_done: 0,
                        tasks_failed: 0,
                        active: true,
                    },
                ),
            ]
            .into_iter()
            .collect(),
            agents: [
                (
                    "agent-b".to_string(),
                    roko_core::dashboard_snapshot::AgentState {
                        agent_id: "agent-b".to_string(),
                        role: "reviewer".to_string(),
                        active: true,
                        output_bytes: 0,
                    },
                ),
                (
                    "agent-c".to_string(),
                    roko_core::dashboard_snapshot::AgentState {
                        agent_id: "agent-c".to_string(),
                        role: "planner".to_string(),
                        active: false,
                        output_bytes: 0,
                    },
                ),
            ]
            .into_iter()
            .collect(),
            gates: vec![roko_core::dashboard_snapshot::GateVerdict {
                plan_id: "plan-b".to_string(),
                task_id: "task-1".to_string(),
                gate: "compile".to_string(),
                passed: true,
                ts_millis: 42,
            }],
            errors: vec![roko_core::dashboard_snapshot::ErrorEntry {
                message: "boom".to_string(),
                ts_millis: 7,
            }],
            ..Default::default()
        };

        apply_dashboard_snapshot(
            &mut app.tui_state,
            &mut app.notifications,
            &mut app.last_snapshot_error_marker,
            &snapshot,
        );

        assert_eq!(app.tui_state.active_tab, Tab::Agents);
        assert_eq!(app.tui_state.plan_scroll_offset, 17);
        assert_eq!(app.tui_state.agent_scroll, Some(9));
        assert_eq!(app.tui_state.plans[0].id, "plan-b");
        assert_eq!(app.tui_state.plans[0].status, super::super::state::PlanPhase::Done);
        assert_eq!(app.tui_state.plans[1].id, "plan-c");
        assert!(!app.tui_state.plans[0].expanded);
        assert_eq!(app.tui_state.selected_plan_idx, 0);
        assert_eq!(app.tui_state.current_plan_idx, 0);
        assert_eq!(app.tui_state.agents[0].id, "agent-b");
        assert!(app.tui_state.agents[0].active);
        assert_eq!(app.tui_state.selected_agent, 0);
        assert_eq!(app.tui_state.gate_results.len(), 1);
        assert_eq!(app.tui_state.gate_results[0].output, "task task-1");
        assert!(
            app.notifications
                .iter()
                .any(|notification| notification.message == "boom")
        );
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
        assert!(!matches!(app.active_modal, Some(ModalState::Help)));

        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(matches!(app.active_modal, Some(ModalState::Help)));

        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(!matches!(app.active_modal, Some(ModalState::Help)));
    }

    #[test]
    fn quit_opens_confirmation_modal_instead_of_exiting() {
        let dir = tempdir().unwrap();
        let mut app = App::new(dir.path());
        assert!(app.running);
        assert!(app.active_modal.is_none());

        app.dispatch_action(TuiAction::Quit);

        assert!(app.running);
        assert!(matches!(app.active_modal, Some(ModalState::Quit)));
        assert_eq!(app.tui_state.input_mode, InputMode::Confirm);
    }

    #[test]
    fn confirming_quit_exits() {
        let dir = tempdir().unwrap();
        let mut app = App::new(dir.path());
        app.active_modal = Some(ModalState::Quit);
        app.tui_state.input_mode = InputMode::Confirm;

        app.dispatch_action(TuiAction::ConfirmYes);

        assert!(!app.running);
        assert!(app.active_modal.is_none());
        assert_eq!(app.tui_state.input_mode, InputMode::Normal);
        assert!(app.tui_state.pending_confirm.is_none());
    }

    #[test]
    fn config_save_reloads_config_immediately() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("roko.toml"),
            RokoConfig::default().to_toml().unwrap(),
        )
        .unwrap();

        let mut app = App::new(dir.path());
        app.tui_state.config_pending.insert(
            "agent.default_model".to_string(),
            "claude-opus-4-6".to_string(),
        );

        app.dispatch_action(TuiAction::ConfigSave);

        let mut reloaded = roko_core::config::load_config(dir.path()).unwrap();
        reloaded.apply_process_env();

        assert!(app.tui_state.config_pending.is_empty());
        assert_eq!(reloaded.agent.default_model, "claude-opus-4-6");
        assert!(
            app.notifications
                .iter()
                .any(|notification| notification.message == "Config saved and reloaded")
        );
    }

    #[test]
    fn config_save_reloads_screen_postfx_immediately() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("roko.toml"),
            RokoConfig::default().to_toml().unwrap(),
        )
        .unwrap();

        let mut app = App::new(dir.path());
        app.tui_state
            .config_pending
            .insert("tui.effects.screen_postfx".to_string(), "true".to_string());

        app.dispatch_action(TuiAction::ConfigSave);

        assert!(app.fx_config.screen_postfx);
        let saved = std::fs::read_to_string(dir.path().join("roko.toml")).unwrap();
        assert!(saved.contains("[tui.effects]"));
        assert!(saved.contains("screen_postfx = true"));
    }

    #[test]
    fn ctrl_e_toggles_screen_postfx() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let dir = tempdir().unwrap();
        let mut app = App::new(dir.path());
        assert!(!app.fx_config.screen_postfx);

        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
        assert!(app.fx_config.screen_postfx);

        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
        assert!(!app.fx_config.screen_postfx);
    }

    #[test]
    fn drill_actions_on_git_use_git_cursor_not_plan_expansion() {
        let dir = tempdir().unwrap();
        let mut app = App::new(dir.path());
        app.tui_state.active_tab = Tab::Git;
        app.tui_state.plans = vec![super::super::state::PlanEntry::default()];
        app.tui_state.git_view_data = Some(super::views::git_view::GitViewData {
            branches: vec![
                super::views::git_view::GitBranchNode {
                    name: "main".to_string(),
                    is_current: true,
                    tracking: None,
                    ahead: 0,
                    behind: 0,
                    depth: 0,
                },
                super::views::git_view::GitBranchNode {
                    name: "feature/test".to_string(),
                    is_current: false,
                    tracking: None,
                    ahead: 0,
                    behind: 0,
                    depth: 1,
                },
            ],
            ..Default::default()
        });

        app.dispatch_action(TuiAction::DrillIn);
        assert_eq!(app.tui_state.git_branch_cursor, 1);
        assert!(!app.tui_state.plans[0].expanded);
        assert_eq!(app.current_view_state().selected, 1);

        app.dispatch_action(TuiAction::DrillOut);
        assert_eq!(app.tui_state.git_branch_cursor, 0);
        assert!(!app.tui_state.plans[0].expanded);
    }

    #[test]
    fn request_confirm_resolves_plan_and_git_context() {
        let dir = tempdir().unwrap();
        let mut app = App::new(dir.path());
        app.tui_state.plans = vec![super::super::state::PlanEntry {
            id: "plan-7".to_string(),
            phase: "done".to_string(),
            status: super::super::state::PlanPhase::Done,
            active: false,
            ..Default::default()
        }];
        app.tui_state.git_branch = "feature/plan-7".to_string();

        app.dispatch_action(TuiAction::RequestConfirm(ConfirmAction::DiagnosePlan(
            String::new(),
        )));
        assert_eq!(app.tui_state.input_mode, InputMode::Confirm);
        assert_eq!(
            app.tui_state.pending_confirm,
            Some(ConfirmAction::DiagnosePlan("plan-7".to_string()))
        );
        assert!(matches!(
            app.active_modal,
            Some(ModalState::Confirm {
                action: modals::ConfirmAction::Custom { .. }
            })
        ));

        app.dispatch_action(TuiAction::RequestConfirm(ConfirmAction::MergePlan {
            plan_id: String::new(),
            branch: String::new(),
        }));
        assert_eq!(
            app.tui_state.pending_confirm,
            Some(ConfirmAction::MergePlan {
                plan_id: "plan-7".to_string(),
                branch: "feature/plan-7".to_string(),
            })
        );

        app.dispatch_action(TuiAction::RequestConfirm(ConfirmAction::MergeAllDone {
            branches: Vec::new(),
        }));
        assert_eq!(
            app.tui_state.pending_confirm,
            Some(ConfirmAction::MergeAllDone {
                branches: vec!["plan-7".to_string()],
            })
        );
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

    #[test]
    fn current_view_state_is_tab_specific() {
        let dir = tempdir().unwrap();
        let mut app = App::new(dir.path());
        app.tui_state.selected_plan_idx = 2;
        app.tui_state.selected_agent = 3;
        app.tui_state.selected_agent_tab = 4;
        app.tui_state.plan_scroll_offset = 8;
        app.tui_state.agent_scroll = Some(5);
        app.tui_state.log_scroll = 7;
        app.tui_state.log_auto_tail = false;

        app.tui_state.active_tab = Tab::Dashboard;
        let view = app.current_view_state();
        assert_eq!(view.scroll, 5);
        assert_eq!(view.selected, 2);
        assert_eq!(view.sub_tab, 0);
        assert!(!view.auto_tail);

        app.tui_state.active_tab = Tab::Plans;
        let view = app.current_view_state();
        assert_eq!(view.scroll, 8);
        assert_eq!(view.selected, 2);
        assert_eq!(view.sub_tab, 0);
        assert!(!view.auto_tail);

        app.tui_state.active_tab = Tab::Agents;
        let view = app.current_view_state();
        assert_eq!(view.scroll, 5);
        assert_eq!(view.selected, 3);
        assert_eq!(view.sub_tab, 4);
        assert!(!view.auto_tail);

        app.tui_state.active_tab = Tab::Logs;
        let view = app.current_view_state();
        assert_eq!(view.scroll, 7);
        assert_eq!(view.selected, 0);
        assert!(!view.auto_tail);
    }

    #[test]
    fn agents_tab_selection_moves_agent_roster() {
        let dir = tempdir().unwrap();
        let mut app = App::new(dir.path());
        app.tui_state.active_tab = Tab::Agents;
        app.tui_state.selected_agent = 1;
        app.tui_state.focus = FocusZone::PlanTree;
        app.tui_state.agents = vec![
            super::super::state::AgentRow::default(),
            super::super::state::AgentRow::default(),
            super::super::state::AgentRow::default(),
        ];

        app.dispatch_action(TuiAction::ScrollFocusedUp);
        assert_eq!(app.tui_state.selected_agent, 0);

        app.dispatch_action(TuiAction::ScrollFocusedDown);
        assert_eq!(app.tui_state.selected_agent, 1);

        app.dispatch_action(TuiAction::ScrollFocusedEnd);
        assert_eq!(app.tui_state.selected_agent, 2);
    }

    #[test]
    fn logs_tail_toggle_and_filter_input_work() {
        let dir = tempdir().unwrap();
        let mut app = App::new(dir.path());
        app.tui_state.active_tab = Tab::Logs;
        app.tui_state.log_auto_tail = true;
        app.tui_state.log_scroll = 0;

        // Scrolling up pins the view (disables auto-tail).
        app.dispatch_action(TuiAction::ScrollLogUp);
        assert_eq!(app.tui_state.log_scroll, 1);

        app.dispatch_action(TuiAction::ScrollLogDown);
        assert!(app.tui_state.log_auto_tail);
        assert_eq!(app.tui_state.log_scroll, 0);
    }
}
