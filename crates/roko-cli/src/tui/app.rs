//! Interactive TUI application shell.
//!
//! Supports two modes:
//! - **Sync** (`App::run`) — 250ms polling, used when no StateHub is available.
//! - **Async** (`App::run_async`) — 60fps render with `tokio::select!` over
//!   keyboard events and `watch::Receiver<DashboardSnapshot>` from the StateHub.

use std::collections::HashMap;
use std::io;
use std::io::Stdout;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use serde_json::Value;

use super::dashboard::{DashboardData, DashboardScaffold, Theme};
use super::event::{Event, EventHandler};
use super::layout::RootLayout;
use super::mori_theme::MoriTheme;
use super::pages::{PageId, PageRegistry};
use super::tabs::Tab;
use super::tui_state::{DetailSubTab, FocusZone};
use super::widgets;

/// Interactive dashboard shell backed by the existing snapshot renderer.
pub struct App {
    workdir: PathBuf,
    /// Currently selected dashboard page (legacy system).
    pub current_page: PageId,
    /// Active tab in the new Tab system.
    pub active_tab: Tab,
    /// Shared dashboard data model, refreshed on tick.
    pub data: DashboardData,
    /// Live snapshot from StateHub (if connected).
    pub live_snapshot: Option<roko_core::dashboard_snapshot::DashboardSnapshot>,
    /// Static page scaffold used by the current renderer.
    scaffold: DashboardScaffold,
    /// Whether the event loop should keep running.
    pub running: bool,
    /// Timestamp of the last data refresh.
    pub last_refresh: Instant,
    /// Per-page scroll position.
    pub scroll_offset: HashMap<PageId, u16>,
    /// Per-tab scroll offset for new views.
    pub tab_scroll: HashMap<Tab, u16>,
    /// Selected plan index (for dashboard/plans views).
    pub plan_selection: usize,
    /// Selected signal row on the Signals page.
    pub signal_selection: usize,
    /// Selected gate-failure row on the Gate Results page.
    pub gate_failure_selection: usize,
    /// Active overlay, if any.
    overlay: Option<OverlayState>,
    /// Whether to use the new ROSEDUST rendering pipeline.
    pub use_new_renderer: bool,
    /// Persistent TUI state — survives across frames, drives all Mori widgets.
    pub tui_state: super::tui_state::TuiState,
    /// System info collector (reused across ticks to avoid re-allocation).
    sys: sysinfo::System,
    /// Last time sysinfo was collected.
    last_sysinfo: Instant,
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

/// Run the interactive dashboard event loop (legacy polling path).
pub async fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| render_page(f, app))?;
        if crossterm::event::poll(Duration::from_millis(250))? {
            match crossterm::event::read()? {
                crossterm::event::Event::Key(key) => handle_key(app, key),
                crossterm::event::Event::Resize(_, _) => {} // ratatui handles this
                _ => {}
            }
        }
        if app.last_refresh.elapsed() > Duration::from_secs(1) {
            app.data.refresh().await?;
            app.last_refresh = Instant::now();
        }
        if !app.running {
            break;
        }
    }

    Ok(())
}

/// Run the interactive dashboard with 60fps async rendering.
///
/// Uses `tokio::select!` over:
/// - 16ms render interval (60fps)
/// - async keyboard EventStream
/// - optional `watch::Receiver<DashboardSnapshot>` from StateHub
///
/// If `snapshot_rx` is `None`, falls back to one-time disk snapshot loading
/// (graceful standalone mode).
pub async fn run_async(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    mut snapshot_rx: Option<
        tokio::sync::watch::Receiver<roko_core::dashboard_snapshot::DashboardSnapshot>,
    >,
) -> Result<()> {
    use crossterm::event::EventStream;

    let mut render_interval = tokio::time::interval(Duration::from_millis(16));
    render_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut reader = EventStream::new();

    // Initial draw.
    terminal
        .draw(|f| render_page(f, app))
        .context("initial async TUI draw")?;

    loop {
        tokio::select! {
            _ = render_interval.tick() => {
                // In standalone mode (no StateHub), periodically refresh from disk.
                if snapshot_rx.is_none() {
                    app.refresh_snapshot_if_needed();
                }
                // Advance atmosphere animation every frame.
                app.tui_state.tick();
                // Collect sysinfo every 500ms.
                if app.last_sysinfo.elapsed() >= Duration::from_millis(500) {
                    app.sys.refresh_cpu_usage();
                    app.sys.refresh_memory();
                    app.tui_state.update_sys_metrics(
                        app.sys.global_cpu_usage(),
                        app.sys.used_memory(),
                        app.sys.total_memory(),
                    );
                    app.last_sysinfo = Instant::now();
                }
                terminal
                    .draw(|f| render_page(f, app))
                    .context("async TUI render")?;
            }
            maybe_event = reader.next() => {
                match maybe_event {
                    Some(Ok(crossterm::event::Event::Key(key)))
                        if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                    {
                        handle_key(app, key);
                    }
                    Some(Ok(crossterm::event::Event::Resize(_, _))) => {
                        // ratatui handles resize on next draw.
                    }
                    Some(Err(e)) => {
                        tracing::warn!("crossterm event error: {e}");
                    }
                    None => break, // Stream closed.
                    _ => {}
                }
            }
            result = async {
                if let Some(rx) = snapshot_rx.as_mut() {
                    rx.changed().await
                } else {
                    // No state hub — sleep forever (won't trigger).
                    std::future::pending::<std::result::Result<(), tokio::sync::watch::error::RecvError>>().await
                }
            } => {
                if result.is_ok() {
                    // Snapshot changed — update the app's live snapshot.
                    if let Some(rx) = snapshot_rx.as_ref() {
                        app.live_snapshot = Some(rx.borrow().clone());
                    }
                }
            }
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
        let live_snapshot = Some(data.to_core_snapshot());

        let mut tui_state = super::tui_state::TuiState::from_dashboard_data(&data);
        if let Some(snap) = &live_snapshot {
            tui_state.update_from_snapshot(snap);
        }
        // Seed git info
        Self::populate_git_info(&workdir, &mut tui_state);

        // Initial sysinfo collection
        let mut sys = sysinfo::System::new();
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        tui_state.update_sys_metrics(
            sys.global_cpu_usage(),
            sys.used_memory(),
            sys.total_memory(),
        );

        Self {
            workdir,
            current_page: scaffold.active_page(),
            active_tab: Tab::Dashboard,
            data,
            live_snapshot,
            scaffold,
            running: true,
            last_refresh: Instant::now(),
            scroll_offset: HashMap::new(),
            tab_scroll: HashMap::new(),
            plan_selection: 0,
            signal_selection: 0,
            gate_failure_selection: 0,
            overlay: None,
            use_new_renderer: true,
            tui_state,
            sys,
            last_sysinfo: Instant::now(),
        }
    }

    fn populate_git_info(workdir: &Path, state: &mut super::tui_state::TuiState) {
        if let Ok(out) = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(workdir)
            .output()
        {
            if out.status.success() {
                state.git_branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
            }
        }
        if let Ok(out) = std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(workdir)
            .output()
        {
            if out.status.success() {
                state.git_commit_short = String::from_utf8_lossy(&out.stdout).trim().to_string();
            }
        }
        if let Ok(out) = std::process::Command::new("git")
            .args(["log", "-1", "--format=%cr"])
            .current_dir(workdir)
            .output()
        {
            if out.status.success() {
                state.git_age = String::from_utf8_lossy(&out.stdout).trim().to_string();
            }
        }
    }

    /// Return the active page.
    #[must_use]
    pub const fn current_page(&self) -> PageId {
        self.current_page
    }

    /// Return the active page.
    #[must_use]
    pub const fn active_page(&self) -> PageId {
        self.current_page
    }

    /// Run the terminal UI with 60fps async rendering via StateHub.
    ///
    /// If `state_hub` is `Some`, the TUI reads from the watch channel for
    /// real-time updates. If `None`, it loads a one-time disk snapshot.
    pub async fn run_with_state_hub(
        mut self,
        state_hub: Option<roko_core::SharedStateHub>,
    ) -> Result<()> {
        let previous_hook: Arc<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static> =
            Arc::from(std::panic::take_hook());
        let panic_hook = Arc::clone(&previous_hook);
        let _restore_hook = PanicHookRestoreGuard(previous_hook);

        std::panic::set_hook(Box::new(move |panic_info| {
            let _ = Self::cleanup_terminal();
            panic_hook(panic_info);
        }));

        let mut terminal = Self::enter_terminal()?;
        let snapshot_rx = state_hub.map(|hub| hub.snapshot());
        let result = run_async(&mut terminal, &mut self, snapshot_rx).await;
        let cleanup = Self::leave_terminal();

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(err), Ok(())) => Err(err),
            (Ok(()), Err(err)) => Err(err),
            (Err(err), Err(_cleanup_err)) => Err(err),
        }
    }

    /// Run the terminal UI until the user quits (sync polling fallback).
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
        let mut events = EventHandler::new(Duration::from_millis(250));
        terminal
            .draw(|frame| self.draw(frame))
            .context("initial TUI draw")?;

        while self.running {
            match events.next().context("poll TUI event")? {
                Event::Key(key) => self.handle_key(key),
                Event::Resize(_, _) => {}
                Event::Tick => {
                    self.refresh_snapshot_if_needed();
                    // Collect sysinfo every 500ms.
                    if self.last_sysinfo.elapsed() >= Duration::from_millis(500) {
                        self.sys.refresh_cpu_usage();
                        self.sys.refresh_memory();
                        self.tui_state.update_sys_metrics(
                            self.sys.global_cpu_usage(),
                            self.sys.used_memory(),
                            self.sys.total_memory(),
                        );
                        self.last_sysinfo = Instant::now();
                    }
                }
            }

            // Advance atmosphere animation every frame.
            self.tui_state.tick();

            terminal
                .draw(|frame| self.draw(frame))
                .context("TUI redraw")?;
        }

        Ok(())
    }

    fn draw(&self, frame: &mut Frame<'_>) {
        if self.use_new_renderer {
            self.draw_new(frame);
        } else {
            self.draw_legacy(frame);
        }
    }

    /// New ROSEDUST rendering pipeline: Mori-accurate header + dashboard + status.
    fn draw_new(&self, frame: &mut Frame<'_>) {
        let root = RootLayout::compute(frame.area());

        // Clear frame with VOID background.
        let bg = ratatui::widgets::Block::default()
            .style(ratatui::style::Style::default().bg(MoriTheme::VOID));
        frame.render_widget(bg, frame.area());

        // Mori header bar on ALL tabs.
        super::widgets::header_bar::render_header_bar(frame, root.header, &self.tui_state);

        // Content: dispatch to the active tab's view.
        match self.active_tab {
            Tab::Dashboard => {
                super::views::mori_dashboard::render(frame, root.content, &self.tui_state);
            }
            Tab::Plans => {
                super::views::plans::render_plans_view(frame, root.content, &self.tui_state);
            }
            Tab::Agents => {
                super::views::agents::render_agents_view(frame, root.content, &self.tui_state);
            }
            Tab::Logs => {
                super::views::logs::render_logs_view(frame, root.content, &self.tui_state);
            }
            Tab::Signals => {
                super::views::signals::render_signals_view(frame, root.content, &self.tui_state);
            }
            Tab::Config => {
                super::views::config::render_config_view(frame, root.content, &self.tui_state);
            }
        }

        // Mori status bar on ALL tabs.
        super::widgets::status_bar::render_status_bar(frame, root.status, &self.tui_state);

        // Overlay (help / detail).
        if let Some(overlay) = &self.overlay {
            self.render_overlay(frame, overlay);
        }
    }

    /// Legacy rendering pipeline (old widgets::render_dashboard).
    fn draw_legacy(&self, frame: &mut Frame<'_>) {
        let pages = self.pages();
        widgets::render_dashboard(
            frame,
            &self.scaffold,
            &self.data,
            &pages,
            self.current_page,
            self.scroll_for(self.current_page),
            self.signal_selection,
            self.gate_failure_selection,
            &Theme::from_env(),
        );
        if let Some(overlay) = &self.overlay {
            self.render_overlay(frame, overlay);
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if self.handle_overlay_key(key) {
            return;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.running = false,
            KeyCode::Char('r') => self.refresh_snapshot(),
            KeyCode::Char('?') => self.toggle_help_overlay(),
            KeyCode::Enter => self.toggle_detail_overlay(),

            // Sub-tab switching (only on Dashboard tab)
            KeyCode::Char(ch @ ('a' | 'o' | 'd' | 'e' | 'g'))
                if self.active_tab == Tab::Dashboard =>
            {
                if let Some(sub) = DetailSubTab::from_key(ch) {
                    self.tui_state.detail_sub_tab = sub;
                }
            }

            // Tab/BackTab: on Dashboard, cycle focus panels; otherwise cycle pages
            KeyCode::Tab if self.active_tab == Tab::Dashboard => self.cycle_focus_forward(),
            KeyCode::BackTab if self.active_tab == Tab::Dashboard => self.cycle_focus_backward(),
            KeyCode::Tab | KeyCode::BackTab => self.select_next_page_by_key(key.code),

            // F1-F6: switch tabs
            KeyCode::F(n @ 1..=6) => self.select_tab_by_fkey(n),
            KeyCode::Char('1') => self.select_page_by_slot(0),
            KeyCode::Char('2') => self.select_page_by_slot(1),
            KeyCode::Char('3') => self.select_page_by_slot(2),
            KeyCode::Char('4') => self.select_page_by_slot(3),
            KeyCode::Char('5') => self.select_page_by_slot(4),
            KeyCode::Char('6') => self.select_page_by_slot(5),
            KeyCode::Left | KeyCode::Char('h') => self.select_previous_page(),
            KeyCode::Right | KeyCode::Char('l') => self.select_next_page(),
            KeyCode::Up | KeyCode::Char('k') => self.adjust_vertical(-1),
            KeyCode::Down | KeyCode::Char('j') => self.adjust_vertical(1),
            KeyCode::PageUp => self.adjust_scroll(-8),
            KeyCode::PageDown => self.adjust_scroll(8),
            KeyCode::Home => self.set_scroll(0),
            _ => {}
        }
    }

    fn cycle_focus_forward(&mut self) {
        self.tui_state.focus = match self.tui_state.focus {
            FocusZone::PlanTree => FocusZone::TaskProgress,
            FocusZone::TaskProgress => FocusZone::AgentOutput,
            FocusZone::AgentOutput => FocusZone::PlanTree,
            FocusZone::PhaseCompact => FocusZone::TaskProgress,
            FocusZone::CommandOutput => FocusZone::PlanTree,
        };
    }

    fn cycle_focus_backward(&mut self) {
        self.tui_state.focus = match self.tui_state.focus {
            FocusZone::PlanTree => FocusZone::AgentOutput,
            FocusZone::TaskProgress => FocusZone::PlanTree,
            FocusZone::AgentOutput => FocusZone::TaskProgress,
            FocusZone::PhaseCompact => FocusZone::PlanTree,
            FocusZone::CommandOutput => FocusZone::AgentOutput,
        };
    }

    fn select_tab_by_fkey(&mut self, n: u8) {
        if let Some(tab) = Tab::from_index((n - 1) as usize) {
            self.active_tab = tab;
            // Sync tui_state::Tab for header/status bar F-key highlighting
            if let Some(tui_tab) = super::tui_state::Tab::from_index((n - 1) as usize) {
                self.tui_state.active_tab = tui_tab;
            }
        }
    }

    fn handle_overlay_key(&mut self, key: KeyEvent) -> bool {
        let Some(overlay) = self.overlay.clone() else {
            return false;
        };

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.running = false;
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
            _ => true,
        }
    }

    fn select_next_page(&mut self) {
        // Also cycle tabs in new renderer.
        let all = Tab::all();
        if let Some(idx) = all.iter().position(|t| *t == self.active_tab) {
            let next_idx = (idx + 1) % all.len();
            self.active_tab = all[next_idx];
            if let Some(tui_tab) = super::tui_state::Tab::from_index(next_idx) {
                self.tui_state.active_tab = tui_tab;
            }
        }
        let pages = self.pages();
        self.current_page = pages.next(self.current_page);
        let _ = self.scaffold.set_active_page(self.current_page);
    }

    fn select_previous_page(&mut self) {
        // Also cycle tabs in new renderer.
        let all = Tab::all();
        if let Some(idx) = all.iter().position(|t| *t == self.active_tab) {
            let prev_idx = (idx + all.len() - 1) % all.len();
            self.active_tab = all[prev_idx];
            if let Some(tui_tab) = super::tui_state::Tab::from_index(prev_idx) {
                self.tui_state.active_tab = tui_tab;
            }
        }
        let pages = self.pages();
        self.current_page = pages.previous(self.current_page);
        let _ = self.scaffold.set_active_page(self.current_page);
    }

    fn select_next_page_by_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Tab => self.select_next_page(),
            KeyCode::BackTab => self.select_previous_page(),
            _ => {}
        }
    }

    fn select_page_by_slot(&mut self, slot: usize) {
        let pages = self.pages().ids();
        if let Some(page) = pages.get(slot).copied() {
            self.current_page = page;
            let _ = self.scaffold.set_active_page(self.current_page);
        }
    }

    fn refresh_snapshot(&mut self) {
        self.data = DashboardData::load_best_effort(&self.workdir);
        self.live_snapshot = Some(self.data.to_core_snapshot());
        self.scaffold = DashboardScaffold::new_in(&self.workdir);
        self.last_refresh = Instant::now();
        self.clamp_signal_selection();
        self.clamp_gate_failure_selection();
        if self.pages().scaffold(self.current_page).is_none() {
            self.current_page = self.scaffold.active_page();
        }
        // Sync tui_state from refreshed data (preserves UI state like focus, atmosphere)
        self.tui_state.sync_from_data(&self.data);
        if let Some(snap) = &self.live_snapshot {
            self.tui_state.update_from_snapshot(snap);
        }
        self.tui_state.selected_plan = self.plan_selection;
    }

    fn refresh_snapshot_if_needed(&mut self) {
        if self.last_refresh.elapsed() >= Duration::from_millis(250) {
            self.refresh_snapshot();
        }
    }

    fn pages(&self) -> PageRegistry {
        PageRegistry::from_dashboard(&self.scaffold)
    }

    fn scroll_for(&self, page: PageId) -> u16 {
        self.scroll_offset.get(&page).copied().unwrap_or(0)
    }

    fn set_scroll(&mut self, value: u16) {
        self.scroll_offset.insert(self.current_page, value);
    }

    fn adjust_scroll(&mut self, delta: i16) {
        let current = self.scroll_for(self.current_page) as i32;
        let next = (current + delta as i32).max(0).min(u16::MAX as i32) as u16;
        self.scroll_offset.insert(self.current_page, next);
    }

    fn adjust_vertical(&mut self, delta: i16) {
        // On Dashboard tab, j/k adjusts selected plan in the plan tree
        if self.active_tab == Tab::Dashboard && self.tui_state.focus == FocusZone::PlanTree {
            let len = self.tui_state.plans.len();
            if len == 0 {
                self.plan_selection = 0;
                self.tui_state.selected_plan = 0;
                return;
            }
            let current = self.plan_selection as i32;
            let next = (current + delta as i32).max(0).min((len - 1) as i32) as usize;
            self.plan_selection = next;
            self.tui_state.selected_plan = next;
            return;
        }

        if self.current_page == PageId::Signals {
            let len = self.data.recent_signals.len();
            if len == 0 {
                self.signal_selection = 0;
                return;
            }

            let current = self.signal_selection as i32;
            let next = (current + delta as i32).max(0).min((len - 1) as i32) as usize;
            self.signal_selection = next;
            return;
        }

        if self.current_page == PageId::GateResults {
            let len = self.data.gate_results_page.failure_rows.len();
            if len == 0 {
                self.gate_failure_selection = 0;
                return;
            }

            let current = self.gate_failure_selection as i32;
            let next = (current + delta as i32).max(0).min((len - 1) as i32) as usize;
            self.gate_failure_selection = next;
        } else {
            self.adjust_scroll(delta);
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

    fn toggle_help_overlay(&mut self) {
        self.overlay = match self.overlay {
            Some(OverlayState::Help) => None,
            Some(OverlayState::Detail(_)) => Some(OverlayState::Help),
            None => Some(OverlayState::Help),
        };
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
        let area = centered_rect(86, 84, frame.area());

        match overlay {
            OverlayState::Help => {
                super::modals::help::render_help_modal(frame, area);
            }
            OverlayState::Detail(detail) => {
                frame.render_widget(Clear, area);
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(detail.title.as_str())
                    .border_style(ratatui::style::Style::default().fg(MoriTheme::WARNING));
                let inner = block.inner(area);
                frame.render_widget(block, area);

                let body = Paragraph::new(detail.body.as_str())
                    .style(
                        ratatui::style::Style::default()
                            .fg(MoriTheme::TEXT)
                            .add_modifier(Modifier::BOLD),
                    )
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

fn render_page(frame: &mut Frame<'_>, app: &App) {
    app.draw(frame);
}

fn handle_key(app: &mut App, key: KeyEvent) {
    app.handle_key(key);
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

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
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
}
