//! Interactive TUI application shell.

use std::collections::HashMap;
use std::io;
use std::io::Stdout;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent};
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

use super::dashboard::{DashboardData, DashboardScaffold, Theme};
use super::event::{Event, EventHandler};
use super::pages::{PageId, PageRegistry};
use super::widgets;

/// Interactive dashboard shell backed by the existing snapshot renderer.
#[derive(Debug)]
pub struct App {
    workdir: PathBuf,
    /// Currently selected dashboard page.
    pub current_page: PageId,
    /// Shared dashboard data model, refreshed on tick.
    pub data: DashboardData,
    /// Static page scaffold used by the current renderer.
    scaffold: DashboardScaffold,
    /// Whether the event loop should keep running.
    pub running: bool,
    /// Timestamp of the last data refresh.
    pub last_refresh: Instant,
    /// Per-page scroll position.
    pub scroll_offset: HashMap<PageId, u16>,
    /// Selected signal row on the Signals page.
    pub signal_selection: usize,
    /// Selected gate-failure row on the Gate Results page.
    pub gate_failure_selection: usize,
    /// Active overlay, if any.
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

/// Run the interactive dashboard event loop.
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
        Self {
            workdir,
            current_page: scaffold.active_page(),
            data,
            scaffold,
            running: true,
            last_refresh: Instant::now(),
            scroll_offset: HashMap::new(),
            signal_selection: 0,
            gate_failure_selection: 0,
            overlay: None,
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
        let mut events = EventHandler::new(Duration::from_millis(250));
        terminal
            .draw(|frame| self.draw(frame))
            .context("initial TUI draw")?;

        while self.running {
            match events.next().context("poll TUI event")? {
                Event::Key(key) => self.handle_key(key),
                Event::Resize(_, _) => {}
                Event::Tick => self.refresh_snapshot_if_needed(),
            }

            terminal
                .draw(|frame| self.draw(frame))
                .context("TUI redraw")?;
        }

        Ok(())
    }

    fn draw(&self, frame: &mut Frame<'_>) {
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
            KeyCode::Tab | KeyCode::BackTab => self.select_next_page_by_key(key.code),
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
        let pages = self.pages();
        self.current_page = pages.next(self.current_page);
        let _ = self.scaffold.set_active_page(self.current_page);
    }

    fn select_previous_page(&mut self) {
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
        self.scaffold = DashboardScaffold::new_in(&self.workdir);
        self.last_refresh = Instant::now();
        self.clamp_signal_selection();
        self.clamp_gate_failure_selection();
        if self.pages().scaffold(self.current_page).is_none() {
            self.current_page = self.scaffold.active_page();
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
        let theme = Theme::from_env();
        let area = centered_rect(86, 84, frame.area());
        frame.render_widget(Clear, area);

        match overlay {
            OverlayState::Help => {
                let lines = help_lines();
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title("help")
                    .border_style(theme.accent());
                let inner = block.inner(area);
                frame.render_widget(block, area);
                let paragraph = Paragraph::new(lines)
                    .alignment(Alignment::Left)
                    .style(theme.text())
                    .wrap(Wrap { trim: false });
                frame.render_widget(paragraph, inner);
            }
            OverlayState::Detail(detail) => {
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(detail.title.as_str())
                    .border_style(theme.warning());
                let inner = block.inner(area);
                frame.render_widget(block, area);

                let body = Paragraph::new(detail.body.as_str())
                    .style(
                        theme
                            .text()
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
    let pages = app.pages();
    widgets::render_dashboard(
        frame,
        &app.scaffold,
        &app.data,
        &pages,
        app.current_page,
        app.scroll_for(app.current_page),
        app.signal_selection,
        app.gate_failure_selection,
        &Theme::from_env(),
    );
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

fn help_lines() -> Vec<Line<'static>> {
    let theme = Theme::from_env();
    vec![
        Line::from(Span::styled(
            "dashboard keybindings",
            theme.accent_bold(),
        )),
        Line::from(""),
        Line::from("1-6      jump to dashboard pages 1 through 6"),
        Line::from("Tab      next page"),
        Line::from("Shift+Tab previous page"),
        Line::from("q / Esc  quit"),
        Line::from("Up/Down  scroll current page or selected list"),
        Line::from("j / k    alternate scroll keys"),
        Line::from("Enter    expand selected signal or gate failure"),
        Line::from("r        refresh data from .roko/"),
        Line::from("?        toggle this help overlay"),
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
}
